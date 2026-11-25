mod picker;

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout_at};
use tracing::{debug, info, warn};

use crate::cpu;
use crate::peer::PeerConnection;
use crate::session::Session;
use crate::torrent::PieceIndex;
use crate::transfer::swarm::Swarm;
use picker::Picker;

const IDLE_TIMEOUT: Duration = Duration::from_mins(2);
const PIECE_BUFFER: usize = 16;
const EXCHANGE_BUFFER: usize = 64;
const MAX_PEERS: usize = 50;

pub async fn download(
    session: Arc<Session>,
    mut peers: mpsc::Receiver<Vec<SocketAddr>>,
    mut on_piece: impl FnMut(u64),
) -> Result<()> {
    let missing = session.missing();
    let mut remaining = missing.count_ones();
    let picker = Arc::new(Picker::new(missing));
    let (sender, mut pieces) = mpsc::channel::<(PieceIndex, Bytes)>(PIECE_BUFFER);
    let (exchange, mut exchanged) = mpsc::channel(EXCHANGE_BUFFER);
    let mut swarm = Swarm::new(MAX_PEERS);

    while remaining > 0 {
        swarm.spawn_queued(|addr| {
            run_peer(
                addr,
                Arc::clone(&session),
                Arc::clone(&picker),
                sender.clone(),
                exchange.clone(),
            )
        });

        if !swarm.has_tasks()
            && pieces.is_empty()
            && exchanged.is_empty()
            && peers.is_closed()
            && peers.is_empty()
        {
            bail!("No peers left with {remaining} pieces missing");
        }

        tokio::select! {
            Some((index, piece)) = pieces.recv() => {
                let len = u64::try_from(piece.len())?;
                session.store_piece(index, piece).await?;
                remaining -= 1;
                on_piece(len);
            }
            Some(addrs) = peers.recv() => swarm.enqueue(addrs),
            Some(addrs) = exchanged.recv() => swarm.enqueue(addrs),
            Some(result) = swarm.join_next() => {
                if let Err(e) = result {
                    debug!("{e:#}");
                }
                if swarm.is_idle() {
                    warn!("All peers disconnected, waiting for trackers");
                }
            }
        }
    }

    Ok(())
}

async fn run_peer(
    addr: SocketAddr,
    session: Arc<Session>,
    picker: Arc<Picker>,
    pieces: mpsc::Sender<(PieceIndex, Bytes)>,
    exchange: mpsc::Sender<Vec<SocketAddr>>,
) -> Result<()> {
    let info = &session.torrent.meta.info;
    let info_hash = session.torrent.info_hash;
    let mut conn =
        PeerConnection::connect(addr, info_hash, session.peer_id, Some(info.piece_count())).await?;
    info!("{addr}: connected ({})", conn.peer_id());

    conn.enable_extensions(exchange).await?;
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

        let index = reservation.index();
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

async fn verify(session: Arc<Session>, index: PieceIndex, piece: Bytes) -> Result<bool> {
    cpu::spawn(move || session.torrent.meta.info.verify(index, &piece)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::Handshake;
    use crate::peer::testing::spawn_seeder;
    use crate::storage::{ResumeData, Storage};
    use crate::torrent::{Torrent, TorrentInfo, TorrentMeta};
    use crate::transfer::seed;
    use std::path::Path;
    use std::time::SystemTime;
    use tokio::net::TcpListener;

    const INFO_HASH: [u8; 20] = [7; 20];

    fn queue(addrs: impl IntoIterator<Item = SocketAddr>) -> mpsc::Receiver<Vec<SocketAddr>> {
        let (sender, receiver) = mpsc::channel(1);
        sender.try_send(addrs.into_iter().collect()).unwrap();
        receiver
    }

    fn data() -> Bytes {
        (0..=u8::MAX).cycle().take(100_000).collect()
    }

    async fn session(dir: &Path, peer_id: [u8; 20]) -> Arc<Session> {
        let info = TorrentInfo::from_data("file", 40_000, &data());
        let storage = Storage::create(dir, &info).await.unwrap();
        let torrent = Torrent {
            meta: TorrentMeta {
                trackers: Vec::new(),
                info,
            },
            info_hash: INFO_HASH.into(),
        };

        Arc::new(Session::new(torrent, peer_id.into(), storage))
    }

    #[tokio::test]
    async fn downloads_from_many_peers() {
        let reply = || Some(Handshake::new(INFO_HASH.into(), [1; 20].into()));
        let corrupt = Bytes::from(vec![0; 100_000]);
        let peers = [
            spawn_seeder(reply(), &[0b1110_0000], corrupt, 40_000).await,
            spawn_seeder(reply(), &[0b1100_0000], data(), 40_000).await,
            spawn_seeder(reply(), &[0b0010_0000], data(), 40_000).await,
        ];

        let dir = tempfile::tempdir().unwrap();
        let leecher = session(dir.path(), [2; 20]).await;
        let mut received = 0;
        download(Arc::clone(&leecher), queue(peers), |len| received += len)
            .await
            .unwrap();

        assert_eq!(received, 100_000);
        assert_eq!(leecher.announce(0, None).left, 0);
        assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), data());
    }

    async fn seeded(dir: &Path) -> Arc<Session> {
        let seeder = session(dir, [1; 20]).await;
        for (index, chunk) in (0u32..).zip(data().chunks(40_000)) {
            seeder
                .store_piece(index.into(), Bytes::copy_from_slice(chunk))
                .await
                .unwrap();
        }
        seeder
    }

    #[tokio::test]
    async fn downloads_from_own_seeder() {
        let seeder_dir = tempfile::tempdir().unwrap();
        let seeder = seeded(seeder_dir.path()).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let leecher_dir = tempfile::tempdir().unwrap();
        let leecher = session(leecher_dir.path(), [2; 20]).await;

        let (discovered, _) = mpsc::channel(1);
        tokio::select! {
            result = seed(Arc::clone(&seeder), &listener, discovered) => result.unwrap(),
            result = download(Arc::clone(&leecher), queue([addr]), |_| {}) => result.unwrap(),
        }

        assert_eq!(
            std::fs::read(leecher_dir.path().join("file")).unwrap(),
            data()
        );
        assert_eq!(seeder.announce(0, None).uploaded, 100_000);

        let resumed = session(leecher_dir.path(), [3; 20]).await;
        assert_eq!(resumed.resume(None).await.unwrap(), 3);
        assert!(resumed.is_complete());
        assert_eq!(resumed.announce(0, None).left, 0);
        download(resumed, queue([]), |_| {}).await.unwrap();

        let empty = session(tempfile::tempdir().unwrap().path(), [4; 20]).await;
        assert_eq!(empty.resume(None).await.unwrap(), 0);
        assert!(download(empty, queue([]), |_| {}).await.is_err());
    }

    #[tokio::test]
    async fn fastresume_cases() {
        let dir = tempfile::tempdir().unwrap();
        let saved = seeded(dir.path()).await.resume_data().await.unwrap();
        let path = dir.path().join("file.fastresume");
        saved.save(&path).await.unwrap();
        assert_eq!(ResumeData::load(&path).await.unwrap(), saved);

        let partial = ResumeData {
            have: vec![0b1000_0000],
            ..saved
        };
        let fast = session(dir.path(), [2; 20]).await;
        assert_eq!(fast.resume(Some(partial.clone())).await.unwrap(), 1);
        assert!(!fast.is_complete());

        let foreign = ResumeData {
            info_hash: [0; 20].into(),
            ..partial.clone()
        };
        let rehashed = session(dir.path(), [3; 20]).await;
        assert_eq!(rehashed.resume(Some(foreign)).await.unwrap(), 3);

        std::fs::File::options()
            .write(true)
            .open(dir.path().join("file"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        let stale = session(dir.path(), [4; 20]).await;
        assert_eq!(stale.resume(Some(partial)).await.unwrap(), 3);
    }
}
