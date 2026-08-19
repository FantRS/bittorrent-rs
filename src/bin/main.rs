use std::process::ExitCode;

mod startup;

fn main() -> ExitCode {
    if let Err(e) = startup::start() {
        eprintln!("Execution error: {}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
