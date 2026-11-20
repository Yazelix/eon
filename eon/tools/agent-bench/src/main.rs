use agent_bench::cli::{self, Failure};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    match cli::run(&args, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Refused(message)) => {
            eprintln!("agent-bench: {message}");
            ExitCode::from(1)
        }
        Err(Failure::Usage(message)) => {
            eprintln!("agent-bench: {message}\n{}", cli::USAGE);
            ExitCode::from(2)
        }
    }
}
