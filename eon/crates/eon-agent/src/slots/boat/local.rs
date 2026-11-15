//! Private evidence for noninteractive, policy-eligible local fallbacks.

use super::super::{SECOND, now_ns};
use crate::launch::atomic_write;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn copy_output(
    mut input: impl Read + AsRawFd,
    mut log: File,
    stderr: bool,
    done: &AtomicBool,
) -> io::Result<bool> {
    let fd = input.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut remaining = None;
    let mut complete = true;
    let mut buffer = [0u8; 16384];
    loop {
        if remaining.is_none() && done.load(Ordering::Acquire) {
            // Drain foreground bytes, without waiting on detached descendants' open pipes.
            let mut available: libc::c_int = 0;
            if unsafe { libc::ioctl(fd, libc::FIONREAD, &mut available) } < 0 {
                return Err(io::Error::last_os_error());
            }
            remaining = Some(available.max(0) as usize);
        }
        let limit = remaining.unwrap_or(buffer.len()).min(buffer.len());
        if limit == 0 {
            return Ok(complete);
        }
        match input.read(&mut buffer[..limit]) {
            Ok(0) => return Ok(complete),
            Ok(count) => {
                complete &= log.write_all(&buffer[..count]).is_ok();
                if stderr {
                    let _ = io::stderr().write_all(&buffer[..count]);
                } else {
                    let _ = io::stdout().write_all(&buffer[..count]);
                }
                if let Some(remaining) = &mut remaining {
                    *remaining -= count;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
}

pub(in crate::slots) fn run(
    root: &Path,
    command: &mut Command,
    reason: &str,
) -> io::Result<ExitStatus> {
    if io::stdin().is_terminal() || io::stdout().is_terminal() || io::stderr().is_terminal() {
        return command.status();
    }
    let run_id =
        crate::run::random().map_err(|_| io::Error::other("cannot create run identity"))?;
    let directory = root.join("runs").join(&run_id);
    super::private_directory(&directory).map_err(io::Error::other)?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("output.log"))?;
    let started = now_ns() / SECOND;
    let mut receipt = serde_json::json!({
        "schema": 1, "run_id": run_id, "pid": std::process::id(), "where": "local",
        "phase": "running", "fallback_reason": reason, "started": started,
        "tree": null, "exit_code": null, "output_complete": false
    });
    let receipt_path = directory.join("receipt.json");
    atomic_write(&receipt_path, &serde_json::to_vec(&receipt)?)?;
    let _ = writeln!(
        io::stderr(),
        "eon agent slot: local output {}",
        directory.join("output.log").display()
    );
    let mut complete = false;
    let status = (|| {
        let stderr_log = log.try_clone()?;
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let stdout = scope.spawn(|| copy_output(stdout, log, false, &done));
            let stderr = scope.spawn(|| copy_output(stderr, stderr_log, true, &done));
            let status = child.wait();
            if status.is_err() {
                let _ = child.kill();
                let _ = child.wait();
            }
            done.store(true, Ordering::Release);
            let stdout = stdout.join();
            let stderr = stderr.join();
            complete = matches!(stdout, Ok(Ok(true))) && matches!(stderr, Ok(Ok(true)));
            status
        })
    })();
    receipt["phase"] = "complete".into();
    receipt["duration_seconds"] = (now_ns() / SECOND).saturating_sub(started).into();
    receipt["output_complete"] = complete.into();
    if let Ok(status) = &status {
        receipt["exit_code"] = (status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or_default()))
        .into();
    }
    if let Err(error) = atomic_write(&receipt_path, &serde_json::to_vec(&receipt)?) {
        let _ = writeln!(
            io::stderr(),
            "eon agent slot: cannot finish local receipt: {error}"
        );
    }
    if !complete {
        let _ = writeln!(
            io::stderr(),
            "eon agent slot: local output recording incomplete"
        );
    }
    status
}
