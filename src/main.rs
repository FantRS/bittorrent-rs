use std::process::ExitCode;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::torrent::Torrent;

fn main() -> ExitCode {
    let args = Args::get();

    let Ok(torrent) = Torrent::load(args.path) else {
        eprintln!("Error reading file");
        return ExitCode::FAILURE;
    };

    let client = Client::new();

    println!("Client ID: {:?}", client.id);
    println!("{:#?}", torrent);

    ExitCode::SUCCESS
}
