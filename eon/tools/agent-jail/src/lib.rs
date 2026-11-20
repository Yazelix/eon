//! agent-jail: the bench's no-network jail for agent harnesses.
//!
//! The outer layer runs only the harness, with the network unshared; its one
//! route out is init's loopback bridge to the host's allowlist proxy, and
//! both hops check who is connecting. Every command the harness starts runs
//! in the inner layer, through wrappers, without network, credentials,
//! sockets or harness policy. See `bead/00-plan.md` of eon-2r02l.2. The
//! jail's store, Cargo home and target cache come from a task environment
//! (`task_env`, eon-2r02l.3).
//!
//! The jail is Linux-only (bwrap, user and network namespaces, `/proc`,
//! `SO_PEERCRED`); elsewhere the crate is empty so the workspace still builds.
#![cfg(target_os = "linux")]

pub mod allowlist;
pub mod bridge;
pub mod cli;
pub mod control;
pub mod credential;
pub mod describe;
pub mod elf;
pub mod enter;
pub mod fds;
pub mod guard;
pub mod hook;
pub mod manifest;
pub mod model;
pub mod pins;
pub mod probe;
pub mod profile;
pub mod proxy;
pub mod run_dir;
pub mod scan;
pub mod seal;
pub mod spec;
pub mod task_build;
pub mod task_env;
pub mod wrapper;

use sha2::{Digest, Sha256};

/// Errors are messages: they name what failed and never carry a token.
pub type Result<T = ()> = std::result::Result<T, String>;

/// Where the jail keeps its own files, read-only in both layers.
pub const JAIL_DIR: &str = "/bench/jail";
/// The wrapper directory on the outer layer's `PATH`.
pub const BIN_DIR: &str = "/bench/bin";
/// The proxy socket directory, read-only in the outer layer, masked inside.
pub const SOCK_DIR: &str = "/bench/sock";
/// Init's private state, masked in the inner layer.
pub const PRIVATE_DIR: &str = "/bench/jail-private";
/// The jail home.
pub const HOME: &str = "/home/bench";
/// The task workspace.
/// The inner layer's view of the workspace: a bind init makes before it
/// seals the harness's view (see `seal`), so commands keep ordinary links.
pub const INNER_WORKSPACE: &str = "/bench/jail-private/workspace";
pub const WORKSPACE: &str = "/bench/workspace";
/// The bridge address the harness's proxy variables name.
pub const BRIDGE: &str = "127.0.0.1:3128";
/// The shell paths that are always wrappers.
pub const SHELL_PATHS: &[&str] = &["/bin/sh", "/bin/bash", "/usr/bin/bash", "/usr/bin/sh"];
/// Exit status for a guard refusal.
pub const GUARD_EXIT: i32 = 3;

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|err| format!("{}: {err}", path.display()))?;
        if count == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

/// Free bytes for an unprivileged writer on `path`'s filesystem.
pub fn free_bytes(path: &std::path::Path) -> Result<u64> {
    let stat = rustix::fs::statvfs(path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Runs `command` with `input` on its standard input, killing it after
/// `timeout`; returns its exit code, standard output and standard error.
pub fn capture(
    command: &mut std::process::Command,
    input: &[u8],
    timeout: std::time::Duration,
) -> Result<(i32, String, String)> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let error_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let text = reader.join().unwrap_or_default();
    let errors = error_reader.join().unwrap_or_default();
    Ok((crate::wrapper::exit_code(status), text, errors))
}

/// A fresh directory for one test, under the target directory's tmp.
#[cfg(test)]
pub(crate) fn test_dir(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let base = std::env::var_os("AGENT_JAIL_TEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "agent-jail-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Makes a special file for a test: a FIFO, or with `rdev` a character
/// device (0:0 is an overlay whiteout, which Linux lets anyone make).
#[cfg(test)]
pub(crate) fn mknod(path: &std::path::Path, kind: rustix::fs::FileType, rdev: u64) -> Result {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        kind,
        rustix::fs::Mode::from_raw_mode(0o600),
        rdev,
    )
    .map_err(|err| format!("{}: {err}", path.display()))
}
