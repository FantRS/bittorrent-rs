use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::{Announce, Client, Event};
use bittorrent_rs::download::download;
use bittorrent_rs::peer::PeerConnection;
use bittorrent_rs::storage::Storage;
use bittorrent_rs::torrent::Torrent;

const READY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let info = &torrent.meta.info;
    let client = Client::build()?;

    let started = Announce {
        left: info.length,
        event: Some(Event::Started),
        ..Announce::default()
    };
    let tracker_response = client.send_tracker_req(&torrent, started).await?;

    println!("Client ID: {:?}", String::from_utf8_lossy(&client.id));
    println!("Tracker response: {tracker_response:?}");

    let mut peers: JoinSet<_> = tracker_response
        .peers
        .iter()
        .map(|&addr| await_ready(addr, torrent.info_hash, client.id, info.piece_count()))
        .collect();

    let mut conn = loop {
        match peers.join_next().await {
            Some(Ok(Ok(conn))) => break conn,
            Some(Ok(Err(e))) => eprintln!("{e:#}"),
            Some(Err(e)) => eprintln!("Peer task failed: {e}"),
            None => bail!("No peer is ready to share pieces"),
        }
    };
    peers.abort_all();

    println!(
        "Downloading {} from {} ({})",
        info.name,
        conn.addr,
        String::from_utf8_lossy(&conn.peer_id)
    );

    let progress = ProgressBar::new(info.length).with_style(ProgressStyle::with_template(
        "{bar:40} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
    )?);
    let mut storage = Storage::create(&args.output, info).await?;
    download(&mut conn, info, &mut storage, |len| progress.inc(len)).await?;
    progress.finish();

    let completed = Announce {
        downloaded: info.length,
        event: Some(Event::Completed),
        ..Announce::default()
    };
    if let Err(e) = client.send_tracker_req(&torrent, completed).await {
        eprintln!("Failed to announce completion: {e:#}");
    }

    Ok(())
}

async fn await_ready(
    addr: SocketAddrV4,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    piece_count: usize,
) -> Result<PeerConnection> {
    let mut conn = PeerConnection::connect(addr, info_hash, peer_id, piece_count).await?;
    conn.interested().await?;

    timeout(READY_TIMEOUT, async {
        while conn.is_choked() || !conn.has_any_piece() {
            conn.recv().await?;
        }
        anyhow::Ok(())
    })
    .await
    .with_context(|| format!("{addr}: peer not ready in time"))??;

    Ok(conn)
}
