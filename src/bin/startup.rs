use anyhow::Result;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::torrent::Torrent;

pub async fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let client = Client::build()?;

    let tracker_response = client.send_tracker_req(&torrent).await?;

    println!("Client ID: {:?}", client.id);
    println!("Tracker response: {:?}", tracker_response);

    Ok(())
}
