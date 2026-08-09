use bittorrent_rs::*;
use clap::Parser;

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = args_parser::Args::parse();

    let path = Path::new(&args.path);
    if !path.is_file() {
        eprintln!("Файл не знайдено!");
        return ExitCode::FAILURE;
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut file = File::open(path).unwrap();
    file.read_to_end(&mut bytes).unwrap();

    println!("Прочитано байтів: {}", bytes.len());
    println!("Вміст: {}", String::from_utf8_lossy(&bytes));

    ExitCode::SUCCESS
}
