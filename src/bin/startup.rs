use anyhow::{Context, Result, bail, ensure};
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::peer::{Message, PeerConnection};
use bittorrent_rs::torrent::Torrent;

const READY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let client = Client::build()?;

    let tracker_response = client.send_tracker_req(&torrent).await?;

    println!("Client ID: {:?}", String::from_utf8_lossy(&client.id));
    println!("Tracker response: {tracker_response:?}");

    let info = &torrent.meta.info;
    let mut peers: JoinSet<_> = tracker_response
        .peers
        .iter()
        .map(|&addr| await_ready(addr, torrent.info_hash, client.id, info.piece_count()))
        .collect();

    let (mut conn, index) = loop {
        match peers.join_next().await {
            Some(Ok(Ok(ready))) => break ready,
            Some(Ok(Err(e))) => eprintln!("{e:#}"),
            Some(Err(e)) => eprintln!("Peer task failed: {e}"),
            None => bail!("No peer is ready to share pieces"),
        }
    };
    peers.abort_all();

    let length = info
        .piece_len(index)
        .with_context(|| format!("Invalid piece index: {index}"))?;
    let piece = conn.download_piece(index, length).await?;
    ensure!(
        info.verify(index, &piece),
        "Piece {index} failed hash check"
    );

    println!(
        "Downloaded piece {index} ({} bytes) from {} ({})",
        piece.len(),
        conn.addr,
        String::from_utf8_lossy(&conn.peer_id)
    );

    Ok(())
}

async fn await_ready(
    addr: SocketAddrV4,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    piece_count: usize,
) -> Result<(PeerConnection, u32)> {
    let mut conn = PeerConnection::connect(addr, info_hash, peer_id, piece_count).await?;
    conn.send(Message::Interested).await?;

    let index = timeout(READY_TIMEOUT, async {
        loop {
            if !conn.is_choked()
                && let Some(index) = conn.pieces().next()
            {
                return anyhow::Ok(index);
            }
            conn.recv().await?;
        }
    })
    .await
    .with_context(|| format!("{addr}: peer not ready in time"))??;

    Ok((conn, index))
}
