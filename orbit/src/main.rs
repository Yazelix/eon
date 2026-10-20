use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

/// The worker in the host's package, `libexec/yazelix-orbit` beside the
/// host's `bin`, or else the one this build made, since a cargo build has no
/// package.
fn worker(host: &Path) -> PathBuf {
    host.parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("libexec/yazelix-orbit/yazelix-orbit-worker"))
        .filter(|packaged| packaged.exists())
        .unwrap_or_else(|| PathBuf::from(env!("ORBIT_BUILT_WORKER")))
}

fn main() -> ExitCode {
    let worker = match env::current_exe() {
        Ok(host) => worker(&host),
        Err(error) => {
            eprintln!("orbit: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args: Vec<_> = env::args_os().skip(1).collect();
    match yazelix_orbit::host::run(&worker, &args) {
        Ok(status) => ExitCode::from(status as u8),
        Err(error) => {
            eprintln!("orbit: {error}");
            ExitCode::FAILURE
        }
    }
}
