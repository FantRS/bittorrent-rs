use bittorrent_rs::*;
use clap::Parser;
use anyhow::{Result, anyhow};

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = args_parser::Args::parse();

    let bytes = b"i43e";
    // println!("{:?}", String::from_utf8(bytes.to_vec()));
    
    // match fss(&args) {
    //     Ok(bytes) => {
    //         bencode_parser::parse(&bytes, 0);
    //         ExitCode::SUCCESS
    //     },
    //     Err(e) => {
    //         eprintln!("{}", e);
    //         ExitCode::FAILURE
    //     }
    // }

    ExitCode::SUCCESS
}

fn fss(args: &args_parser::Args) -> Result<Vec<u8>> {
    let path = Path::new(&args.path);
    if !path.is_file() {
        let e = anyhow!("Файл не знайдено!");
        return Err(e);
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut file = File::open(path).unwrap();
    file.read_to_end(&mut bytes).unwrap();

    println!("Прочитано байтів: {}", bytes.len());
    println!("Byte: {:?}", bytes.first());

    Ok(bytes)
}