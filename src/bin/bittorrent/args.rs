use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    /// Path to a .torrent file or a magnet link
    #[arg(short, long)]
    pub path: String,

    #[arg(
        short,
        long,
        default_value = ".",
        help = "Directory to save downloaded files into"
    )]
    pub output: PathBuf,
}

impl Args {
    #[must_use]
    pub fn get() -> Self {
        Self::parse()
    }
}
