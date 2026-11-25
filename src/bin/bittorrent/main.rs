mod args;
mod startup;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = startup::start().await {
        eprintln!("Execution error: {e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
