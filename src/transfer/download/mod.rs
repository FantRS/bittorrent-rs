mod assembler;
mod picker;

use anyhow::{Context, Result, bail};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout_at};
use tracing::{debug, info, warn};

use crate::cpu;
use crate::peer::{Message, PeerConnection};
use crate::session::Session;
use crate::torrent::TorrentInfo;
use crate::transfer::swarm::Swarm;
use assembler::{Assembled, Assembler};
use picker::{Picker, Reservation};

const IDLE_TIMEOUT: Duration = Duration::from_mins(2);
const STALL_TIMEOUT: Duration = Duration::from_mins(1);
const PIECE_BUFFER: usize = 16;
const EXCHANGE_BUFFER: usize = 64;
const MAX_PEERS: usize = 50;

#[derive(Debug)]
struct Downloaded {
    addr: SocketAddr,
    piece: Assembled,
}

#[derive(Debug)]
enum Verdict {
    Stored(u64),
    Corrupt(SocketAddr),
}

pub async fn download(
    session: Arc<Session>,
    mut peers: mpsc::Receiver<Vec<SocketAddr>>,
    mut on_piece: impl FnMut(u64),
) -> Result<()> {
    let missing = session.missing();
    let mut remaining = missing.count_ones();
    let picker = Arc::new(Picker::new(missing));
    let (sender, mut downloaded) = mpsc::channel::<Downloaded>(PIECE_BUFFER);
    let (exchange, mut exchanged) = mpsc::channel(EXCHANGE_BUFFER);
    let mut swarm = Swarm::new(MAX_PEERS);
    let mut verifying = JoinSet::new();

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
            && verifying.is_empty()
            && downloaded.is_empty()
            && exchanged.is_empty()
            && peers.is_closed()
            && peers.is_empty()
        {
            bail!("No peers left with {remaining} pieces missing");
        }

        tokio::select! {
            Some(Downloaded { addr, piece }) = downloaded.recv() => {
                verifying.spawn(store(Arc::clone(&session), addr, piece));
            }
            Some(verdict) = verifying.join_next() => match verdict?? {
                Verdict::Stored(len) => {
                    remaining -= 1;
                    on_piece(len);
                }
                Verdict::Corrupt(addr) => {
                    warn!("{addr}: sent a corrupt piece, banning");
                    swarm.ban(addr);
                }
            },
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

async fn store(session: Arc<Session>, addr: SocketAddr, piece: Assembled) -> Result<Verdict> {
    let Assembled { reservation, data } = piece;
    let index = reservation.index();

    let checked = Arc::clone(&session);
    let block = data.clone();
    if !cpu::spawn(move || checked.torrent.meta.info.verify(index, &block)).await? {
        return Ok(Verdict::Corrupt(addr));
    }

    let len = u64::try_from(data.len())?;
    session.store_piece(index, data).await?;
    reservation.complete();

    Ok(Verdict::Stored(len))
}

async fn run_peer(
    addr: SocketAddr,
    session: Arc<Session>,
    picker: Arc<Picker>,
    downloaded: mpsc::Sender<Downloaded>,
    exchange: mpsc::Sender<Vec<SocketAddr>>,
) -> Result<()> {
    let info = &session.torrent.meta.info;
    let mut conn = PeerConnection::connect(
        addr,
        session.torrent.info_hash,
        session.peer_id,
        Some(info.piece_count()),
    )
    .await?;
    info!("{addr}: connected ({})", conn.peer_id());

    conn.enable_extensions(exchange).await?;
    conn.interested().await?;
    let mut assembler = Assembler::default();
    let mut released = picker.subscribe();
    let mut deadline = Instant::now() + IDLE_TIMEOUT;

    loop {
        if !conn.is_choked() {
            let mut batch = Vec::new();
            while assembler.pending() < conn.queue_depth()
                && let Some(request) = assembler.next_request(|| reserve(&picker, &conn, info))
            {
                batch.push(request);
            }
            if !batch.is_empty() {
                conn.request(&batch).await?;
                deadline = Instant::now() + STALL_TIMEOUT;
            }
        }

        if assembler.is_idle() {
            if picker.is_done() {
                return Ok(());
            }
            if !conn.is_choked() && picker.wants(|index| conn.has_piece(index)) {
                deadline = Instant::now() + IDLE_TIMEOUT;
            }
        }

        let was_choked = conn.is_choked();
        tokio::select! {
            message = timeout_at(deadline, conn.recv()) => {
                let message = message.with_context(|| format!("{addr}: no progress for too long"))??;
                if was_choked && !conn.is_choked() {
                    assembler.requeue();
                    deadline = Instant::now() + STALL_TIMEOUT;
                }
                if let Message::Piece(piece) = &message {
                    deadline = Instant::now() + STALL_TIMEOUT;
                    if let Some(piece) = assembler.receive(piece) {
                        downloaded.send(Downloaded { addr, piece }).await?;
                    }
                }
            }
            _ = released.changed(), if assembler.is_idle() => {}
        }
    }
}

fn reserve(
    picker: &Arc<Picker>,
    conn: &PeerConnection,
    info: &TorrentInfo,
) -> Option<(Reservation, u32)> {
    let reservation = picker.reserve(|index| conn.has_piece(index))?;
    let length = info.piece_len(reservation.index())?;

    Some((reservation, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::Handshake;
    use crate::peer::testing::spawn_seeder;
    use crate::storage::{ResumeData, Storage};
    use crate::torrent::{Torrent, TorrentInfo, TorrentMeta};
    use crate::transfer::seed;
    use bytes::Bytes;
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
