use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    /// Full path to .torrent file in your PC
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
