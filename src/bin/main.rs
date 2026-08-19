use std::process::ExitCode;

mod startup;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = startup::start().await {
        eprintln!("Execution error: {}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
