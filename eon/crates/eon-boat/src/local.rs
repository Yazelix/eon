//! Private evidence for noninteractive, policy-eligible local fallbacks.

use rota::launch::atomic_write;
use rota::slots::{SECOND, Signals, now_ns};
use rustix::process::{Pid, Signal};
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn wait(child: &mut Child) -> io::Result<ExitStatus> {
    let mut forwarded = 0;
    let mut deadline = None;
    loop {
        if let Some(status) = child.try_wait()? {
            let signal = Signals::caught();
            return Ok(if signal == 0 {
                status
            } else {
                ExitStatus::from_raw(signal)
            });
        }
        let signal = Signals::caught();
        if signal != 0 && forwarded == 0 {
            forwarded = signal;
            deadline = Some(Instant::now() + Duration::from_secs(super::CLEANUP));
            let _ = rustix::process::kill_process_group(
                Pid::from_raw(child.id() as i32).unwrap(),
                Signal::from_named_raw(signal).unwrap(),
            );
        }
        if deadline.is_some_and(|until| Instant::now() >= until) {
            let _ = rustix::process::kill_process_group(
                Pid::from_raw(child.id() as i32).unwrap(),
                Signal::KILL,
            );
            deadline = None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Foreground {
    tty: File,
    previous: Option<Pid>,
}

fn previous_group(tty: &File) -> io::Result<Option<Pid>> {
    match rustix::termios::tcgetpgrp(tty) {
        Ok(group) => Ok(Some(group)),
        // rustix represents the kernel's successful group-zero reply as OPNOTSUPP.
        Err(rustix::io::Errno::OPNOTSUPP) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl Foreground {
    fn open() -> io::Result<Option<Self>> {
        if !(io::stdin().is_terminal() || io::stdout().is_terminal() || io::stderr().is_terminal())
        {
            return Ok(None);
        }
        let tty = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
            Ok(tty) => tty,
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::ENXIO | libc::ENOENT | libc::ENODEV)
                ) =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let previous = previous_group(&tty)?;
        Ok(Some(Self { tty, previous }))
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            let _ = rota::slots::foreground(&self.tty, previous);
        }
    }
}

pub(crate) fn status(command: &mut Command) -> io::Result<ExitStatus> {
    let _signals = Signals::install();
    let terminal = Foreground::open()?;
    command.process_group(0);
    if let Some(terminal) = &terminal {
        // Give the child its terminal before exec: it must not race into SIGTTIN/EIO.
        rota::slots::foreground_on_exec(command, &terminal.tty)?;
    }
    wait(&mut command.spawn()?)
}

fn copy_output(
    mut input: impl Read + AsFd,
    mut log: File,
    stderr: bool,
    done: &AtomicBool,
) -> io::Result<bool> {
    let flags = rustix::fs::fcntl_getfl(&input)?;
    rustix::fs::fcntl_setfl(&input, flags | rustix::fs::OFlags::NONBLOCK)?;
    let mut remaining = None;
    let mut complete = true;
    let mut buffer = [0u8; 16384];
    loop {
        if remaining.is_none() && done.load(Ordering::Acquire) {
            // Drain foreground bytes, without waiting on detached descendants' open pipes.
            let available = rustix::io::ioctl_fionread(&input)? as i32;
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

pub(crate) fn run(
    root: &Path,
    command: &mut Command,
    reason: &str,
    tree: Option<&str>,
    program: &str,
) -> io::Result<ExitStatus> {
    if io::stdin().is_terminal() || io::stdout().is_terminal() || io::stderr().is_terminal() {
        return status(command);
    }
    let _signals = Signals::install();
    command.process_group(0);
    let run_id = rota::run::random().map_err(|_| io::Error::other("cannot create run identity"))?;
    rota::slots::private::directory(&root.join("runs"), super::NOUN).map_err(io::Error::other)?;
    let directory = root.join("runs").join(&run_id);
    rota::slots::private::directory(&directory, super::NOUN).map_err(io::Error::other)?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("output.log"))?;
    let started = now_ns() / SECOND;
    let mut receipt = serde_json::json!({
        "schema": 1, "run_id": run_id, "pid": std::process::id(), "where": "local",
        "phase": "running", "fallback_reason": reason, "started": started,
        "tree": tree, "exit_code": null, "output_complete": false
    });
    let receipt_path = directory.join("receipt.json");
    atomic_write(&receipt_path, &serde_json::to_vec(&receipt)?)?;
    let _ = writeln!(
        io::stderr(),
        "{program} slot: local output {}",
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
            let status = wait(&mut child);
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
            "{program} slot: cannot finish local receipt: {error}"
        );
    }
    if !complete {
        let _ = writeln!(
            io::stderr(),
            "{program} slot: local output recording incomplete"
        );
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_pty_has_no_previous_group_and_nonterminals_keep_their_error() {
        let master = File::from(
            rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)
                .unwrap(),
        );
        assert_eq!(previous_group(&master).unwrap(), None);
        assert_eq!(
            previous_group(&tempfile::tempfile().unwrap())
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ENOTTY)
        );
    }
}
