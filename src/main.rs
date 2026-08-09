use bittorrent_rs::*;
use clap::Parser;

fn main() {
    let args = cli_parser::Args::parse();

    println!("Entered  name: {}", args.name);
}
