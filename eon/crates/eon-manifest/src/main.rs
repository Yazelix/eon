use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let Some(path) = arguments.next() else {
        eprintln!("usage: {} <manifest.json>", program.to_string_lossy());
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: {} <manifest.json>", program.to_string_lossy());
        return ExitCode::from(2);
    }

    let input = match fs::read_to_string(&path) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("{}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };

    match eon_manifest::parse_and_validate(&input) {
        Ok(()) => {
            println!("valid: {}", path.to_string_lossy());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}: {error}", path.to_string_lossy());
            ExitCode::FAILURE
        }
    }
}
