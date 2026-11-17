use clap::Parser;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    /// Full path to .torrent file in your PC
    #[arg(short, long)]
    pub path: String,
}

impl Args {
    #[must_use]
    pub fn get() -> Self {
        Self::parse()
    }
}
