use anyhow::{Context, Result};
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::peer::{Message, PeerConnection};
use bittorrent_rs::torrent::Torrent;

const UNCHOKE_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let client = Client::build()?;

    let tracker_response = client.send_tracker_req(&torrent).await?;

    println!("Client ID: {:?}", String::from_utf8_lossy(&client.id));
    println!("Tracker response: {tracker_response:?}");

    let mut peers: JoinSet<_> = tracker_response
        .peers
        .iter()
        .map(|&addr| await_unchoke(addr, torrent.info_hash, client.id))
        .collect();

    while let Some(joined) = peers.join_next().await {
        match joined {
            Ok(Ok(conn)) => println!(
                "Unchoked by {} ({})",
                conn.addr,
                String::from_utf8_lossy(&conn.peer_id)
            ),
            Ok(Err(e)) => eprintln!("{e:#}"),
            Err(e) => eprintln!("Peer task failed: {e}"),
        }
    }

    Ok(())
}

async fn await_unchoke(
    addr: SocketAddrV4,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
) -> Result<PeerConnection> {
    let mut conn = PeerConnection::connect(addr, info_hash, peer_id).await?;
    conn.send(Message::Interested).await?;

    timeout(UNCHOKE_TIMEOUT, async {
        loop {
            match conn.recv().await? {
                Message::Unchoke => return anyhow::Ok(()),
                Message::Bitfield(bits) => {
                    let pieces: u32 = bits.iter().map(|byte| byte.count_ones()).sum();
                    println!("{addr}: has {pieces} pieces");
                }
                message => println!("{addr}: {message:?}"),
            }
        }
    })
    .await
    .with_context(|| format!("{addr}: unchoke timed out"))??;

    Ok(conn)
}
