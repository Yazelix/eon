//! The bare-PTY layer: a reader drains a PTY's master as fast as it can while
//! the completion writer writes a corpus to the slave, the kernel floor under
//! every terminal.

use std::{
    fs::File,
    io::{ErrorKind, Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::Path,
    process::{Command, Stdio},
};

use crate::writer::{STATUS_OK, STATUS_QUERY};

/// The grid the PTY reports; the `cursor` corpus addresses it.
pub const COLS: u16 = 80;
pub const ROWS: u16 = 24;
/// Orbit's PTY read size.
const CHUNK: usize = 64 * 1024;
/// How long the master may stay silent before the sample fails.
const IDLE_TIMEOUT_MS: i32 = 60_000;

pub struct Drained {
    /// The writer's time from its first write to the status reply.
    pub seconds: f64,
    /// The bytes and reads the master delivered, the status query included.
    pub bytes: u64,
    pub reads: u64,
}

/// Runs `writer write FILE --report REPORT` on a fresh PTY's slave and drains
/// the master, answering the writer's status query. The bytes drained must be
/// `expected_bytes`.
pub fn drain(
    writer: &Path,
    file: &Path,
    report: &Path,
    expected_bytes: u64,
) -> Result<Drained, String> {
    let (mut master, slave) = open_pty().map_err(|error| format!("openpty: {error}"))?;
    let stdio = |slave: &File| {
        slave
            .try_clone()
            .map(Stdio::from)
            .map_err(|error| format!("PTY slave: {error}"))
    };
    let _ = std::fs::remove_file(report);
    let mut child = Command::new(writer)
        .arg("write")
        .arg(file)
        .arg("--report")
        .arg(report)
        .stdin(stdio(&slave)?)
        .stdout(stdio(&slave)?)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", writer.display()))?;
    // Only the writer holds the slave now, so the master ends when it exits.
    drop(slave);
    let read = read_until_closed(&mut master);
    if read.is_err() {
        // Exact PID; the writer is our own child.
        let _ = child.kill();
    }
    let status = child.wait().map_err(|error| format!("writer: {error}"))?;
    let mut errors = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut errors);
    }
    let errors = errors.trim();
    let (bytes, reads) = match read {
        Ok(read) => read,
        Err(error) if errors.is_empty() => return Err(error),
        Err(error) => return Err(format!("{error}; the writer said: {errors}")),
    };
    if !status.success() {
        return Err(format!("writer {status}: {errors}"));
    }
    if bytes != expected_bytes {
        return Err(format!(
            "the PTY delivered {bytes} bytes; expected {expected_bytes}"
        ));
    }
    let report = std::fs::read_to_string(report)
        .map_err(|error| format!("{}: {error}", report.display()))?;
    let seconds = crate::json::Value::parse(&report)
        .ok()
        .and_then(|report| report.get("seconds")?.as_f64())
        .filter(|seconds| *seconds > 0.0)
        .ok_or("the writer's report has no time")?;
    Ok(Drained {
        seconds,
        bytes,
        reads,
    })
}

/// Reads the master until every slave descriptor is closed, answering the
/// status query that ends the writer's output. The master is nonblocking and
/// polled only when it has nothing to read, so a busy run costs one syscall
/// per read.
fn read_until_closed(master: &mut File) -> Result<(u64, u64), String> {
    let mut buffer = vec![0; CHUNK];
    let (mut bytes, mut reads) = (0u64, 0u64);
    let mut end = QueryAtEnd::default();
    let mut answered = false;
    loop {
        let count = match master.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                wait_readable(master, bytes)?;
                continue;
            }
            // Linux reports a closed slave as EIO.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => return Err(format!("PTY master: {error}")),
        };
        bytes += count as u64;
        reads += 1;
        if end.push(&buffer[..count]) && !answered {
            master
                .write_all(STATUS_OK)
                .map_err(|error| format!("PTY master: {error}"))?;
            answered = true;
        }
    }
    if !answered {
        return Err(format!(
            "the writer ended after {bytes} bytes without a status query"
        ));
    }
    Ok((bytes, reads))
}

/// Waits until the master has data or its slave has closed, failing after
/// `IDLE_TIMEOUT_MS` of silence.
fn wait_readable(master: &File, bytes: u64) -> Result<(), String> {
    loop {
        let mut poll = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll reads one valid pollfd.
        match unsafe { libc::poll(&raw mut poll, 1, IDLE_TIMEOUT_MS) } {
            -1 if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted => {}
            -1 => return Err(format!("poll: {}", std::io::Error::last_os_error())),
            0 => {
                return Err(format!(
                    "the PTY was silent for {} s after {bytes} bytes",
                    IDLE_TIMEOUT_MS / 1000
                ));
            }
            _ => return Ok(()),
        }
    }
}

/// Whether a stream, fed in reads of any size, ends with the status query.
/// The query is the last thing the writer sends before it waits, and no
/// corpus holds it, so checking each read's end finds it without scanning the
/// stream; the last bytes carry over, so a query split across reads counts.
#[derive(Default)]
struct QueryAtEnd {
    tail: Vec<u8>,
}

impl QueryAtEnd {
    fn push(&mut self, read: &[u8]) -> bool {
        let keep = STATUS_QUERY.len();
        self.tail
            .extend_from_slice(&read[read.len().saturating_sub(keep)..]);
        self.tail.drain(..self.tail.len().saturating_sub(keep));
        self.tail == STATUS_QUERY
    }
}

/// A PTY of the benchmark grid with both descriptors closed on exec and a
/// nonblocking master; the kernel's default line discipline, so output gets
/// ONLCR.
fn open_pty() -> std::io::Result<(File, File)> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: ROWS,
        ws_col: COLS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty initializes both descriptors on success.
    if unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    } == -1
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a successful openpty transferred ownership of both descriptors.
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        // SAFETY: fd is a live descriptor owned above.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let fd = master.as_raw_fd();
    // SAFETY: fd is the live master owned above.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((master, slave))
}

/// The bytes a PTY with ONLCR delivers for `corpus` and the status query:
/// each LF gains a CR before it.
pub fn expected_bytes(corpus: &[u8]) -> u64 {
    let newlines = corpus.iter().filter(|&&byte| byte == b'\n').count();
    (corpus.len() + newlines + STATUS_QUERY.len()) as u64
}

#[cfg(test)]
mod tests {
    use super::QueryAtEnd;

    /// The query is found at the end of the stream however reads split it,
    /// including one byte at a time, and not when output follows it.
    #[test]
    fn query_is_found_at_the_end_across_every_split() {
        let stream = b"ab\r\n\x1b[1mc\x1b[5n";
        for first in 0..=stream.len() {
            for second in first..=stream.len() {
                let mut end = QueryAtEnd::default();
                let found: Vec<bool> =
                    [&stream[..first], &stream[first..second], &stream[second..]]
                        .iter()
                        .map(|read| end.push(read))
                        .collect();
                let ends = [first, second, stream.len()];
                assert_eq!(
                    found,
                    ends.map(|end| end == stream.len()),
                    "split at {first} and {second}"
                );
            }
        }
        let mut bytewise = QueryAtEnd::default();
        let found: Vec<bool> = stream.iter().map(|byte| bytewise.push(&[*byte])).collect();
        assert_eq!(found.iter().filter(|found| **found).count(), 1);
        assert!(found[stream.len() - 1]);
        let mut followed = QueryAtEnd::default();
        assert!(followed.push(b"x\x1b[5n"));
        assert!(!followed.push(b"y"));
    }
}
