use anyhow::Result;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::torrent::Torrent;

pub fn start() -> Result<()> {
    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let client = Client::build()?;

    println!("Client ID: {:?}", client.id);
    println!("{:#?}", torrent);

    Ok(())
}
