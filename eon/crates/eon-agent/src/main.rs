#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    rota::cli::main(std::env::args_os().skip(1), Ok(None), &|_| {
        Err("native quota requires eon agent".into())
    })
}

#[cfg(not(target_os = "linux"))]
fn main() {}
