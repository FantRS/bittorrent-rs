use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use bittorrent_rs::*;
use clap::Parser;
use anyhow::{Result, anyhow};

fn main() -> ExitCode {
    let args = args_parser::Args::parse();

    match fs_operate(&args) {
        Ok(bytes) => {
            let mut parser = bencode::BencodeParser::new(bytes);
            let parse_result = parser.parse().unwrap();

            println!("{:?}", parse_result);

            ExitCode::SUCCESS
        },
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

fn fs_operate(args: &args_parser::Args) -> Result<Vec<u8>> {
    let path = Path::new(&args.path);
    if !path.is_file() {
        let e = anyhow!("Файл не знайдено!");
        return Err(e);
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut file = File::open(path).unwrap();
    file.read_to_end(&mut bytes).unwrap();

    Ok(bytes)
}