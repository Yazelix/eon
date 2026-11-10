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
/// `expected_bytes`, plus only duplicated ONLCR CRs on Apple, with content checked.
pub fn drain(
    writer: &Path,
    file: &Path,
    report: &Path,
    expected_bytes: u64,
) -> Result<Drained, String> {
    let mut output = Vec::new();
    if cfg!(target_vendor = "apple") {
        output.reserve(expected_bytes as usize + CHUNK);
    }
    let (mut master, slave) = open(ROWS, COLS).map_err(|error| format!("openpty: {error}"))?;
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
    let read = read_until_closed(&mut master, &mut output);
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
    let extra_crs = if cfg!(target_vendor = "apple") {
        let corpus = std::fs::read(file).map_err(|error| format!("{}: {error}", file.display()))?;
        duplicated_crs(&corpus, &output)?
    } else {
        0
    };
    if bytes != expected_bytes + extra_crs {
        return Err(format!(
            "the PTY delivered {bytes} bytes; expected {expected_bytes} plus {extra_crs} duplicated CRs"
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
fn read_until_closed(master: &mut File, output: &mut Vec<u8>) -> Result<(u64, u64), String> {
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
        if cfg!(target_vendor = "apple") {
            output.extend_from_slice(&buffer[..count]);
        }
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

/// XNU can enqueue ONLCR's CR, fill the queue, then enqueue it again when retrying LF.
fn duplicated_crs(corpus: &[u8], output: &[u8]) -> Result<u64, String> {
    let (mut at, mut extra) = (0, 0);
    let mismatch = |at: usize, expected: &str| {
        let actual = output
            .get(at)
            .map_or_else(|| "EOF".into(), |byte| format!("0x{byte:02x}"));
        format!("the PTY output at byte {at}: expected {expected}, received {actual}")
    };
    for byte in corpus.iter().chain(STATUS_QUERY) {
        let translated = if *byte == b'\n' {
            b"\r\n".as_slice()
        } else {
            std::slice::from_ref(byte)
        };
        for &expected in translated {
            if expected == b'\n' {
                while output.get(at) == Some(&b'\r') {
                    at += 1;
                    extra += 1;
                }
            }
            if output.get(at) != Some(&expected) {
                return Err(mismatch(at, &format!("0x{expected:02x}")));
            }
            at += 1;
        }
    }
    if at != output.len() {
        return Err(mismatch(at, "EOF"));
    }
    Ok(extra)
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

/// A PTY of `rows` x `cols` with both descriptors closed on exec and a
/// nonblocking master; the kernel's default line discipline, so output gets
/// ONLCR.
pub fn open(rows: u16, cols: u16) -> std::io::Result<(File, File)> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
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
/// each LF gains a CR before it. Apple may additionally duplicate that CR.
pub fn expected_bytes(corpus: &[u8]) -> u64 {
    let newlines = corpus.iter().filter(|&&byte| byte == b'\n').count();
    (corpus.len() + newlines + STATUS_QUERY.len()) as u64
}

#[cfg(test)]
mod tests {
    use super::{QueryAtEnd, duplicated_crs, expected_bytes};
    use crate::writer::STATUS_QUERY;

    #[test]
    fn apple_output_allows_only_extra_crs_before_lf() {
        let valid: &[(&[u8], &[u8], u64)] = &[
            (b"", b"", 0),
            (b"a\r\nb\r", b"a\r\r\nb\r", 0),
            (b"a\r\nb\r", b"a\r\r\r\nb\r", 1),
            (b"\n", b"\r\r\r\n", 2),
            (b"\r\n", b"\r\r\r\r\n", 2),
            (b"\r\r\n", b"\r\r\r\r\n", 1),
            (b"\n\n", b"\r\r\n\r\r\r\n", 3),
        ];
        for &(corpus, data, extra) in valid {
            let output = [data, STATUS_QUERY].concat();
            assert_eq!(duplicated_crs(corpus, &output), Ok(extra));
            assert_eq!(output.len() as u64, expected_bytes(corpus) + extra);
            // The old oracle, retained on Linux, rejects any duplicate count.
            assert_eq!(output.len() as u64 == expected_bytes(corpus), extra == 0);
        }
        for data in [
            b"ab\r\n".as_slice(),
            b"ab\r\r",
            b"\rab\r\r\n",
            b"a\rb\r\r\n",
            b"ac\r\r\n",
            b"a\r\r\r\n", // One duplicate cannot hide the dropped 'b'.
        ] {
            let output = [data, STATUS_QUERY].concat();
            assert!(duplicated_crs(b"ab\r\n", &output).is_err(), "{output:?}");
        }
        for output in [
            b"ab\r\r\n\x1b[5".as_slice(),
            b"ab\r\r\n\x1b[6n",
            b"ab\r\r\n\x1b[5n\r",
        ] {
            assert!(duplicated_crs(b"ab\r\n", output).is_err(), "{output:?}");
        }
        assert_eq!(
            duplicated_crs(b"ab\r\n", b"ac\r\r\n\x1b[5n"),
            Err("the PTY output at byte 1: expected 0x62, received 0x63".into())
        );
    }

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
