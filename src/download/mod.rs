mod picker;

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout_at};
use tracing::{debug, info};

use crate::peer::PeerConnection;
use crate::session::Session;
use picker::Picker;

const IDLE_TIMEOUT: Duration = Duration::from_mins(2);
const PIECE_BUFFER: usize = 16;

pub async fn download(
    session: Arc<Session>,
    peers: impl IntoIterator<Item = SocketAddr>,
    mut on_piece: impl FnMut(u64),
) -> Result<()> {
    let piece_count = session.torrent.meta.info.piece_count();
    let picker = Arc::new(Picker::new(piece_count));
    let (sender, mut pieces) = mpsc::channel(PIECE_BUFFER);

    let mut tasks: JoinSet<_> = peers
        .into_iter()
        .map(|addr| {
            run_peer(
                addr,
                Arc::clone(&session),
                Arc::clone(&picker),
                sender.clone(),
            )
        })
        .collect();
    drop(sender);

    let mut remaining = piece_count;
    while remaining > 0 {
        tokio::select! {
            Some((index, piece)) = pieces.recv() => {
                let len = u64::try_from(piece.len())?;
                session.store_piece(index, piece).await?;
                remaining -= 1;
                on_piece(len);
            }
            Some(joined) = tasks.join_next() => {
                if let Err(e) = joined.map_err(anyhow::Error::from).and_then(|result| result) {
                    debug!("{e:#}");
                }
            }
            else => bail!("All peers disconnected with {remaining} pieces left"),
        }
    }

    Ok(())
}

async fn run_peer(
    addr: SocketAddr,
    session: Arc<Session>,
    picker: Arc<Picker>,
    pieces: mpsc::Sender<(u32, Bytes)>,
) -> Result<()> {
    let info = &session.torrent.meta.info;
    let info_hash = session.torrent.info_hash;
    let mut conn =
        PeerConnection::connect(addr, info_hash, session.peer_id, info.piece_count()).await?;
    info!(
        "{addr}: connected ({})",
        String::from_utf8_lossy(&conn.peer_id)
    );

    conn.interested().await?;
    let mut released = picker.subscribe();
    let mut deadline = Instant::now() + IDLE_TIMEOUT;

    while !picker.is_done() {
        let reservation = if conn.is_choked() {
            None
        } else {
            picker.reserve(|index| conn.has_piece(index))
        };

        let Some(reservation) = reservation else {
            if !conn.is_choked() && picker.wants(|index| conn.has_piece(index)) {
                deadline = Instant::now() + IDLE_TIMEOUT;
            }
            tokio::select! {
                message = timeout_at(deadline, conn.recv()) => {
                    message.with_context(|| format!("{addr}: idle for too long"))??;
                }
                _ = released.changed() => {}
            }
            continue;
        };

        let index = u32::try_from(reservation.index())?;
        let length = info
            .piece_len(index)
            .with_context(|| format!("Invalid piece index: {index}"))?;
        let piece = conn.download_piece(index, length).await?;
        ensure!(
            verify(Arc::clone(&session), index, piece.clone()).await?,
            "{addr}: piece {index} failed hash check"
        );

        reservation.complete();
        pieces.send((index, piece)).await?;
        deadline = Instant::now() + IDLE_TIMEOUT;
    }

    Ok(())
}

async fn verify(session: Arc<Session>, index: u32, piece: Bytes) -> Result<bool> {
    let (sender, receiver) = oneshot::channel();
    rayon::spawn(move || {
        let _ = sender.send(session.torrent.meta.info.verify(index, &piece));
    });

    Ok(receiver.await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::Handshake;
    use crate::peer::testing::spawn_seeder;
    use crate::storage::Storage;
    use crate::torrent::{Torrent, TorrentInfo, TorrentMeta};
    use crate::upload::seed;
    use sha1::{Digest, Sha1};
    use std::path::Path;
    use tokio::net::TcpListener;

    const INFO_HASH: [u8; 20] = [7; 20];

    fn data() -> Bytes {
        (0..=u8::MAX).cycle().take(100_000).collect()
    }

    async fn session(dir: &Path, peer_id: [u8; 20]) -> Arc<Session> {
        let info = TorrentInfo {
            name: "file".into(),
            piece_length: 40_000,
            pieces: data()
                .chunks(40_000)
                .map(|chunk| Sha1::digest(chunk).into())
                .collect(),
            length: 100_000,
        };
        let storage = Storage::create(dir, &info).await.unwrap();
        let torrent = Torrent {
            meta: TorrentMeta {
                announce: String::new(),
                info,
            },
            info_hash: INFO_HASH,
        };

        Arc::new(Session::new(torrent, peer_id, storage))
    }

    #[tokio::test]
    async fn downloads_from_many_peers() {
        let reply = || Some(Handshake::new(INFO_HASH, [1; 20]));
        let corrupt = Bytes::from(vec![0; 100_000]);
        let peers = [
            spawn_seeder(reply(), &[0b1110_0000], corrupt, 40_000).await,
            spawn_seeder(reply(), &[0b1100_0000], data(), 40_000).await,
            spawn_seeder(reply(), &[0b0010_0000], data(), 40_000).await,
        ];

        let dir = tempfile::tempdir().unwrap();
        let leecher = session(dir.path(), [2; 20]).await;
        let mut received = 0;
        download(Arc::clone(&leecher), peers, |len| received += len)
            .await
            .unwrap();

        assert_eq!(received, 100_000);
        assert_eq!(leecher.stats(None).left, 0);
        assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), data());
    }

    #[tokio::test]
    async fn downloads_from_own_seeder() {
        let seeder_dir = tempfile::tempdir().unwrap();
        let seeder = session(seeder_dir.path(), [1; 20]).await;
        for (index, chunk) in (0..).zip(data().chunks(40_000)) {
            seeder
                .store_piece(index, Bytes::copy_from_slice(chunk))
                .await
                .unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let leecher_dir = tempfile::tempdir().unwrap();
        let leecher = session(leecher_dir.path(), [2; 20]).await;

        tokio::select! {
            result = seed(Arc::clone(&seeder), &listener) => result.unwrap(),
            result = download(Arc::clone(&leecher), [addr], |_| {}) => result.unwrap(),
        }

        assert_eq!(
            std::fs::read(leecher_dir.path().join("file")).unwrap(),
            data()
        );
        assert_eq!(seeder.stats(None).uploaded, 100_000);
    }
}
