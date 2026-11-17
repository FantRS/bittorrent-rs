use anyhow::Result;
use tokio::task::JoinSet;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::peer::PeerConnection;
use bittorrent_rs::torrent::Torrent;

pub async fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let client = Client::build()?;

    let tracker_response = client.send_tracker_req(&torrent).await?;

    println!("Client ID: {:?}", String::from_utf8_lossy(&client.id));
    println!("Tracker response: {tracker_response:?}");

    let mut handshakes: JoinSet<_> = tracker_response
        .peers
        .iter()
        .map(|&addr| PeerConnection::connect(addr, torrent.info_hash, client.id))
        .collect();

    while let Some(joined) = handshakes.join_next().await {
        match joined {
            Ok(Ok(conn)) => println!(
                "Connected to {} ({})",
                conn.addr,
                String::from_utf8_lossy(&conn.peer_id)
            ),
            Ok(Err(e)) => eprintln!("{e:#}"),
            Err(e) => eprintln!("Handshake task failed: {e}"),
        }
    }

    Ok(())
}
