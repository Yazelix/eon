#![forbid(unsafe_code)]
#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    let session = |_: std::time::Instant| Err("native quota requires eon agent".into());
    rota::cli::main(
        std::env::args_os().skip(1),
        &rota::cli::Host::rota(&session),
    )
}

#[cfg(not(target_os = "linux"))]
fn main() {}
