//! The completion writer, `eon-bench write FILE --report REPORT`: run inside a
//! terminal, it writes FILE to the terminal, then asks for a status report
//! (DSR, `ESC[5n`) and waits for the terminal's reply (`ESC[0n`). A terminal
//! answers only after it has read everything before the query, so the time
//! from the first write to the reply covers all of FILE and cannot be won by
//! skipping work. Input queued before the run is discarded; the timer starts
//! after FILE is in memory and stops at the reply; REPORT gets
//! `{"bytes": N, "seconds": S}`. Without a reply within 60 s of the query it
//! fails, and the terminal's mode is restored either way. The report also
//! holds the monotonic clock at the first write and at the reply
//! (`started_ns`, `replied_ns`), which the `orbit` suite compares with frames.

use std::{
    fs::File,
    io::{ErrorKind, Read, Write},
    os::fd::{AsFd, BorrowedFd},
    path::Path,
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec},
    io::Errno,
    termios::{LocalModes, OptionalActions, QueueSelector, SpecialCodeIndex, Termios},
};

use crate::{environment::monotonic_ns, json::Value};

pub const STATUS_QUERY: &[u8] = b"\x1b[5n";
pub const STATUS_OK: &[u8] = b"\x1b[0n";

pub fn run(file: &Path, report: &Path) -> Result<(), String> {
    let failure = |what: &str, error: std::io::Error| format!("{what}: {error}");
    let bytes = std::fs::read(file).map_err(|error| failure(&file.display().to_string(), error))?;
    // Raw descriptors, not std's Stdout, which buffers by line.
    let mut output = File::from(
        std::io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|error| failure("stdout", error))?,
    );
    let input = File::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|error| failure("stdin", error))?,
    );
    let mode =
        RawInput::enter(&input).map_err(|error| failure("stdin is not a terminal", error))?;
    let started_ns = monotonic_ns();
    let start = Instant::now();
    output
        .write_all(&bytes)
        .and_then(|()| output.write_all(STATUS_QUERY))
        .map_err(|error| failure("stdout", error))?;
    // Read through a shared borrow: the mode guard holds one too.
    wait_for_reply(&mut &input, REPLY_TIMEOUT)?;
    let seconds = start.elapsed().as_secs_f64();
    let replied_ns = monotonic_ns();
    drop(mode);
    let result = Value::Object(vec![
        ("bytes".into(), (bytes.len() as u64).into()),
        ("seconds".into(), seconds.into()),
        ("started_ns".into(), started_ns.into()),
        ("replied_ns".into(), replied_ns.into()),
    ]);
    std::fs::write(report, result.to_pretty())
        .map_err(|error| failure(&report.display().to_string(), error))
}

/// How long the writer waits for the reply once the query is written.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Reads the terminal's input until the status reply. Anything before it,
/// such as a key pressed during the run, is skipped; a reply other than OK,
/// or none within `timeout`, even while other input keeps arriving, fails the
/// run.
fn wait_for_reply(input: &mut (impl Read + AsFd), timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    let mut buffer = [0; 256];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!(
                "no reply to the status query within {} s",
                timeout.as_secs_f64()
            ));
        }
        let mut poll = [PollFd::new(&*input, PollFlags::IN)];
        // The exact remainder, so a wait under a millisecond still waits.
        let left = Timespec::try_from(left).unwrap_or(Timespec {
            tv_sec: i64::MAX,
            tv_nsec: 0,
        });
        match rustix::event::poll(&mut poll, Some(&left)) {
            Err(Errno::INTR) => continue,
            Err(error) => return Err(format!("stdin: {}", std::io::Error::from(error))),
            // The deadline check above reports the timeout.
            Ok(0) => continue,
            Ok(_) => {}
        }
        let count = match input.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("stdin: {error}")),
        };
        if count == 0 {
            return Err("stdin closed before the terminal answered the status query".into());
        }
        seen.extend_from_slice(&buffer[..count]);
        if seen
            .windows(STATUS_OK.len())
            .any(|window| window == STATUS_OK)
        {
            return Ok(());
        }
        if seen.windows(4).any(|window| window == b"\x1b[3n") {
            return Err("the terminal reported a malfunction (ESC[3n)".into());
        }
        // Keep only what could start a reply split across reads.
        seen.drain(..seen.len().saturating_sub(STATUS_OK.len() - 1));
    }
}

/// The terminal's input without line editing or echo, so the reply arrives
/// unbuffered and is not echoed into the output being measured; output
/// processing, such as ONLCR, stays as it was. Dropping it restores the mode.
pub(crate) struct RawInput<'a> {
    fd: BorrowedFd<'a>,
    saved: Termios,
}

impl<'a> RawInput<'a> {
    pub(crate) fn enter(input: &'a File) -> std::io::Result<Self> {
        let fd = input.as_fd();
        let saved = rustix::termios::tcgetattr(fd)?;
        let mut raw = saved.clone();
        raw.local_modes &= !(LocalModes::ICANON | LocalModes::ECHO);
        raw.special_codes[SpecialCodeIndex::VMIN] = 1;
        raw.special_codes[SpecialCodeIndex::VTIME] = 0;
        rustix::termios::tcsetattr(fd, OptionalActions::Now, &raw)?;
        let mode = Self { fd, saved };
        // Input queued before the run, such as a stale reply, must not end it.
        rustix::termios::tcflush(fd, QueueSelector::IFlush)?;
        Ok(mode)
    }
}

impl Drop for RawInput<'_> {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(self.fd, OptionalActions::Now, &self.saved);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    use super::wait_for_reply;

    /// Input that never holds the reply cannot hold the writer past its
    /// deadline, however steadily it arrives; the reply ends the wait even when
    /// split across reads.
    #[test]
    fn reply_wait_ends_at_the_deadline_despite_other_input() {
        let (mut reader, mut writer) = std::io::pipe().unwrap();
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    writer.write_all(b"x").unwrap();
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            let start = Instant::now();
            let result = wait_for_reply(&mut reader, Duration::from_millis(300));
            stop.store(true, Ordering::Relaxed);
            let error = result.unwrap_err();
            assert!(error.contains("no reply"), "{error}");
            assert!(start.elapsed() < Duration::from_secs(5));
        });

        let (mut reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"key\x1b[").unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                writer.write_all(b"0n").unwrap();
            });
            wait_for_reply(&mut reader, Duration::from_secs(30)).unwrap();
        });
    }
}
