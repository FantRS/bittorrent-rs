use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, sleep_until};
use tracing::{debug, info, warn};

use crate::peer::{Message, PeerConnection, Piece};
use crate::session::Session;

const UPLOAD_SLOTS: usize = 4;
const SLOT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

pub async fn seed(
    session: Arc<Session>,
    listener: &TcpListener,
    discovered: mpsc::Sender<Vec<SocketAddr>>,
) -> Result<()> {
    let slots = Arc::new(Semaphore::new(UPLOAD_SLOTS));
    let mut peers = JoinSet::new();

    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let peer = serve(stream, Arc::clone(&session), Arc::clone(&slots), discovered.clone());
                    peers.spawn(peer);
                }
                Err(e) => {
                    warn!("Failed to accept connection: {e}");
                    sleep(ACCEPT_BACKOFF).await;
                }
            },
            Some(joined) = peers.join_next() => {
                if let Err(e) = joined.map_err(anyhow::Error::from).and_then(|result| result) {
                    debug!("{e:#}");
                }
            }
        }
    }
}

async fn serve(
    stream: TcpStream,
    session: Arc<Session>,
    slots: Arc<Semaphore>,
    discovered: mpsc::Sender<Vec<SocketAddr>>,
) -> Result<()> {
    let info_hash = session.torrent.info_hash;
    let piece_count = session.torrent.meta.info.piece_count();
    let mut conn = PeerConnection::accept(stream, info_hash, session.peer_id, piece_count).await?;
    info!("{}: incoming peer ({})", conn.addr, conn.peer_id());

    let mut completed = session.subscribe();
    let mut slot: Option<OwnedSemaphorePermit> = None;
    let mut acquire = Box::pin(Arc::clone(&slots).acquire_owned());
    let mut idle_deadline = Instant::now();
    conn.send(Message::Bitfield(session.bitfield())).await?;
    conn.enable_extensions(discovered).await?;

    loop {
        tokio::select! {
            message = conn.recv() => match message? {
                Message::NotInterested => {
                    acquire.set(Arc::clone(&slots).acquire_owned());
                    if slot.take().is_some() {
                        conn.send(Message::Choke).await?;
                    }
                }
                Message::Request(request) if slot.is_some() => {
                    let block = session.read_block(request).await?;
                    conn.send(Message::Piece(Piece {
                        index: request.index,
                        begin: request.begin,
                        block,
                    }))
                    .await?;
                    idle_deadline = Instant::now() + SLOT_IDLE_TIMEOUT;
                }
                _ => {}
            },
            permit = &mut acquire, if slot.is_none() && conn.is_peer_interested() => {
                acquire.set(Arc::clone(&slots).acquire_owned());
                slot = Some(permit?);
                idle_deadline = Instant::now() + SLOT_IDLE_TIMEOUT;
                conn.send(Message::Unchoke).await?;
            }
            () = sleep_until(idle_deadline), if slot.is_some() => {
                slot = None;
                conn.send(Message::Choke).await?;
            }
            Ok(index) = completed.recv() => conn.send(Message::Have(index)).await?,
        }
    }
}
