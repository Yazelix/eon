mod cli;
mod control;
mod generation;
mod managed_environment;
mod sessions;
mod supervisor;
mod workspace;

use std::process::ExitCode;

fn main() -> ExitCode {
    let (product, result) = cli::run();
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{product}: {error}");
            ExitCode::FAILURE
        }
    }
}
