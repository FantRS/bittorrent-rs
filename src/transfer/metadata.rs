use anyhow::{Context, Result, bail, ensure};
use bitvec::prelude::BitVec;
use futures_util::future::{join, join_all};
use sha1::{Digest, Sha1};
use std::convert::Infallible;
use std::future;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::lookup_host;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::dht::Dht;
use crate::peer::{METADATA_PIECE_LEN, PeerConnection};
use crate::torrent::{Magnet, Torrent};
use crate::tracker::{Announce, announce_periodically};
use crate::transfer::swarm::Swarm;

const MAX_PEERS: usize = 10;
const MAX_METADATA_SIZE: usize = 1 << 24;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const PEER_BUFFER: usize = 16;
const UNKNOWN_LEFT: u64 = 1 << 14;
const REQUEST_WINDOW: usize = 8;

pub async fn resolve(
    magnet: &Magnet,
    peer_id: [u8; 20],
    port: u16,
    dht: Option<&Dht>,
) -> Result<Torrent> {
    ensure!(
        !magnet.trackers.is_empty() || !magnet.peers.is_empty() || dht.is_some(),
        "Magnet link has no trackers or peers, and DHT is unavailable"
    );
    info!(
        "Fetching metadata for {}",
        magnet.name.as_deref().unwrap_or("magnet link")
    );

    let request = |event| Announce {
        info_hash: magnet.info_hash,
        peer_id,
        port,
        uploaded: 0,
        downloaded: 0,
        left: UNKNOWN_LEFT,
        event,
    };
    let (sender, peers) = mpsc::channel(PEER_BUFFER);
    sender.try_send(lookup(&magnet.peers).await)?;
    let announcing = async move {
        let trackers = magnet
            .trackers
            .iter()
            .map(|tracker| announce_periodically(tracker, request, &sender));
        let dht = async {
            if let Some(dht) = dht {
                dht.announce_periodically(magnet.info_hash, port, &sender)
                    .await;
            }
        };
        join(join_all(trackers), dht).await;
        drop(sender);
        future::pending::<Infallible>().await
    };

    let metadata = tokio::select! {
        metadata = fetch(magnet.info_hash, peer_id, peers) => metadata?,
        never = announcing => match never {},
    };
    info!("Fetched {} bytes of metadata", metadata.len());

    Torrent::from_metadata(magnet, metadata)
}

async fn fetch(
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    mut peers: mpsc::Receiver<Vec<SocketAddr>>,
) -> Result<Vec<u8>> {
    let (exchange, mut exchanged) = mpsc::channel(PEER_BUFFER);
    let mut swarm = Swarm::new(MAX_PEERS);

    loop {
        swarm.spawn_queued(|addr| {
            let fetching = fetch_from(addr, info_hash, peer_id, exchange.clone());
            async move {
                timeout(FETCH_TIMEOUT, fetching)
                    .await
                    .with_context(|| format!("{addr}: metadata fetch timed out"))?
                    .with_context(|| format!("{addr}: metadata fetch failed"))
            }
        });

        if !swarm.has_tasks() && exchanged.is_empty() && peers.is_closed() && peers.is_empty() {
            bail!("No peers left to fetch metadata from");
        }

        tokio::select! {
            Some(addrs) = peers.recv() => swarm.enqueue(addrs),
            Some(addrs) = exchanged.recv() => swarm.enqueue(addrs),
            Some(result) = swarm.join_next() => match result {
                Ok(metadata) => return Ok(metadata),
                Err(e) => {
                    debug!("{e:#}");
                    if swarm.is_idle() {
                        warn!("All peers failed to provide metadata, waiting for trackers");
                    }
                }
            },
        }
    }
}

async fn lookup(peers: &[String]) -> Vec<SocketAddr> {
    let resolved = join_all(peers.iter().map(|peer| async move {
        lookup_host(peer.as_str())
            .await
            .inspect_err(|e| debug!("{peer}: {e}"))
            .ok()
    }))
    .await;

    resolved.into_iter().flatten().flatten().collect()
}

async fn fetch_from(
    addr: SocketAddr,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    exchange: mpsc::Sender<Vec<SocketAddr>>,
) -> Result<Vec<u8>> {
    let mut conn = PeerConnection::connect(addr, info_hash, peer_id, None).await?;
    ensure!(
        conn.supports_extensions(),
        "Peer does not support extensions"
    );
    conn.enable_extensions(exchange).await?;

    let size = conn.metadata_size().await?;
    ensure!(
        (1..=MAX_METADATA_SIZE).contains(&size),
        "Invalid metadata size: {size}"
    );

    let count = u32::try_from(size.div_ceil(METADATA_PIECE_LEN))?;
    let mut requests = 0..count;
    for piece in requests.by_ref().take(REQUEST_WINDOW) {
        conn.request_metadata(piece).await?;
    }

    let mut metadata = vec![0; size];
    let mut missing: BitVec = BitVec::repeat(true, usize::try_from(count)?);
    while missing.any() {
        let (piece, block) = conn.recv_metadata().await?;
        let index = usize::try_from(piece)?;
        *missing
            .get_mut(index)
            .with_context(|| format!("Invalid metadata piece: {piece}"))? = false;
        let start = index
            .checked_mul(METADATA_PIECE_LEN)
            .context("Metadata piece offset overflow")?;
        let expected = size.saturating_sub(start).min(METADATA_PIECE_LEN);
        ensure!(
            block.len() == expected,
            "Invalid metadata piece {piece}: {} bytes",
            block.len()
        );

        metadata
            .get_mut(start..start + expected)
            .context("Metadata piece out of bounds")?
            .copy_from_slice(&block);

        if let Some(piece) = requests.next() {
            conn.request_metadata(piece).await?;
        }
    }

    ensure!(
        Sha1::digest(&metadata).as_slice() == info_hash,
        "Metadata hash mismatch"
    );

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::extension::MetadataMessage;
    use crate::peer::{Handshake, Message, MessageCodec};
    use bytes::Bytes;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_util::codec::Framed;

    fn metadata() -> Bytes {
        let name = "a".repeat(300_000);
        format!(
            "d6:lengthi3e4:name{}:{name}12:piece lengthi4e6:pieces20:aaaaaaaaaaaaaaaaaaaae",
            name.len()
        )
        .into()
    }

    async fn spawn_peer(info_hash: [u8; 20], metadata: Bytes) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            Handshake::read_from(&mut stream).await.unwrap();
            Handshake::new(info_hash, [1; 20])
                .write_to(&mut stream)
                .await
                .unwrap();

            let mut frames = Framed::new(stream, MessageCodec);
            let handshake = format!(
                "d1:md11:ut_metadatai3ee13:metadata_sizei{}ee",
                metadata.len()
            );
            frames
                .send(Message::Extended {
                    id: 0,
                    payload: handshake.into(),
                })
                .await
                .unwrap();

            while let Some(Ok(message)) = frames.next().await {
                if let Message::Extended { id: 3, payload } = message
                    && let Ok(MetadataMessage::Request(piece)) = MetadataMessage::decode(&payload)
                {
                    let start = usize::try_from(piece).unwrap() * METADATA_PIECE_LEN;
                    let end = (start + METADATA_PIECE_LEN).min(metadata.len());
                    let reply = MetadataMessage::Data {
                        piece,
                        block: metadata.slice(start..end),
                    };
                    frames
                        .send(Message::Extended {
                            id: 2,
                            payload: reply.encode(),
                        })
                        .await
                        .unwrap();
                }
            }
        });

        addr
    }

    fn queue(addrs: Vec<SocketAddr>) -> mpsc::Receiver<Vec<SocketAddr>> {
        let (sender, receiver) = mpsc::channel(1);
        sender.try_send(addrs).unwrap();
        receiver
    }

    #[tokio::test]
    async fn fetch_cases() {
        let metadata = metadata();
        let info_hash: [u8; 20] = Sha1::digest(&metadata).into();
        let forged = metadata.iter().rev().copied().collect::<Bytes>();

        let peers = vec![
            spawn_peer(info_hash, forged.clone()).await,
            spawn_peer(info_hash, metadata.clone()).await,
        ];
        let fetched = fetch(info_hash, [2; 20], queue(peers)).await.unwrap();
        assert_eq!(fetched, metadata);

        let magnet = Magnet {
            info_hash,
            name: None,
            trackers: vec!["udp://tracker.example:1337".into()],
            peers: Vec::new(),
        };
        let torrent = Torrent::from_metadata(&magnet, fetched).unwrap();
        assert_eq!(torrent.info_hash, info_hash);
        assert_eq!(torrent.meta.info.name.len(), 300_000);
        assert_eq!(torrent.meta.trackers, magnet.trackers);

        let forged_only = vec![spawn_peer(info_hash, forged).await];
        assert!(fetch(info_hash, [2; 20], queue(forged_only)).await.is_err());
    }

    #[tokio::test]
    async fn resolve_cases() {
        let metadata = metadata();
        let info_hash: [u8; 20] = Sha1::digest(&metadata).into();
        let port = spawn_peer(info_hash, metadata).await.port();
        let magnet = |peers: Vec<String>| Magnet {
            info_hash,
            name: Some("dir".into()),
            trackers: Vec::new(),
            peers,
        };

        let torrent = resolve(&magnet(vec![format!("localhost:{port}")]), [2; 20], 0, None)
            .await
            .unwrap();
        assert_eq!(torrent.info_hash, info_hash);
        assert!(torrent.meta.trackers.is_empty());

        assert!(
            resolve(&magnet(Vec::new()), [2; 20], 0, None)
                .await
                .is_err()
        );

        let forged = spawn_peer(info_hash, Bytes::from_static(b"de")).await;
        let unreachable = vec![forged.to_string(), "unresolvable.invalid:1".into()];
        assert!(
            resolve(&magnet(unreachable), [2; 20], 0, None)
                .await
                .is_err()
        );
    }
}
