use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, anyhow};
use bittorrent_rs::*;
use clap::Parser;

fn main() -> ExitCode {
    let args = args_parser::Args::parse();

    let Ok(bytes) = open_file(args.path) else {
        eprintln!("Error reading file");
        return ExitCode::FAILURE;
    };

    let mut parser = bencode::BencodeParser::new(bytes);
    let info_hash = parser.info_hash().unwrap();
    let _torrent_meta = parser.parse().and_then(|value| value.torrent_meta());

    println!("{:?}", hex::encode(info_hash));

    ExitCode::SUCCESS
}

fn open_file<I: AsRef<Path>>(path: I) -> Result<Vec<u8>> {
    let path = path.as_ref();
    if !path.is_file() {
        let e = anyhow!("Файл не знайдено!");
        return Err(e);
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut file = File::open(path)?;
    file.read_to_end(&mut bytes)?;

    Ok(bytes)
}
