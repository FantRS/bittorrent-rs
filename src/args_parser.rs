use clap::Parser;

#[derive(Debug, Parser)]
#[command(version = "0.1", about = "")]
pub struct Args {
    #[arg(short)]
    pub path: String
}
