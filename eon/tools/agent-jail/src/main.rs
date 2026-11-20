//! agent-jail: the bench's no-network jail for agent harnesses (Linux only).

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    agent_jail::cli::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("agent-jail: the bench jail runs only on Linux");
    std::process::ExitCode::FAILURE
}
