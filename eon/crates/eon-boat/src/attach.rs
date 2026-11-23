//! Following a detached Boat command until it ends. A lost `exec --attach` stream is
//! not a lost result: liveness comes only from Boat's own process status and the run
//! deadline, never from what the stream delivered (eon-lnfc7).
use super::{CONTROL, Client, Output, TEARDOWN, billing_policy, remaining};
use rota::slots::Signals;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The longest frame line read into memory; a longer line is dropped up to its newline.
pub(super) const LINE_CAP: usize = 8 * 1024 * 1024;
/// One evidence entry, after escaping.
const ENTRY_CAP: usize = 4096;
/// Frame and stderr entries stop here; control entries continue to `HARD_CAP`.
const FRAME_CAP: u64 = 1024 * 1024;
const HARD_CAP: u64 = 2 * 1024 * 1024;
/// An attempt held this long resets the re-attach backoff.
const HELD: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const STEP: Duration = Duration::from_millis(100);

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn budget_safe(limits: &Value, client: &Client) -> bool {
    if billing_policy(limits).is_err() || limits["keepGoing"]["enabled"] != false {
        return false;
    }
    limits["activeSandboxes"].as_u64().is_some_and(|active| {
        // Cover the configured concurrency at this run's grant and any higher
        // count already reported by the provider.
        let parallel = active.max(u64::from(client.parallel_leases));
        let buffer = parallel as f64 * client.machine.weight() * (TEARDOWN + CONTROL) as f64;
        remaining(limits).is_ok_and(|seconds| seconds > client.reserve_seconds + buffer)
    })
}

/// The kill reasons of a run's attach phase, checked from every poll, status read and
/// wait, with clocks shared across all of them.
pub(super) struct Guard {
    patience: u64,
    next_budget: Mutex<Instant>,
    /// Watch output silence: only while the first stream holds.
    silence: AtomicBool,
    last_output: AtomicU64,
    /// The first reason that fired; every later check returns it again.
    fired: Mutex<Option<String>>,
}

impl Guard {
    pub(super) fn new(patience: u64) -> Self {
        Self {
            patience,
            next_budget: Mutex::new(Instant::now() + Duration::from_secs(CONTROL)),
            silence: AtomicBool::new(true),
            last_output: AtomicU64::new(now_seconds()),
            fired: Mutex::new(None),
        }
    }

    /// Output arrived; it counts only while silence is watched.
    pub(super) fn output(&self) {
        self.last_output.store(now_seconds(), Ordering::Relaxed);
    }

    /// The first stream was lost: from now on silence decides nothing.
    fn lose(&self) {
        self.silence.store(false, Ordering::Relaxed);
    }

    /// Ends the attach phase with `reason` unless another reason fired first, and returns
    /// the reason that stands. A running attach sees it at its next poll and is killed.
    fn give_up(&self, reason: String) -> String {
        self.fired.lock().unwrap().get_or_insert(reason).clone()
    }

    /// Signal, run deadline, budget, then silence (first stream only), in that
    /// precedence. An unreadable status is never a timer here: only a fresh failed status
    /// read decides it (`follow`).
    pub(super) fn check(&self, client: &Client) -> Result<(), String> {
        let mut fired = self.fired.lock().unwrap();
        if let Some(reason) = fired.as_ref() {
            return Err(reason.clone());
        }
        let reason = self.reason(client);
        if let Some(reason) = &reason {
            *fired = Some(reason.clone());
        }
        reason.map_or(Ok(()), Err)
    }

    fn reason(&self, client: &Client) -> Option<String> {
        if !client.cleanup && Signals::caught() != 0 {
            return Some("interrupted".into());
        }
        if Instant::now() >= client.deadline {
            return Some("remote-deadline".into());
        }
        let mut next = self.next_budget.lock().unwrap();
        if !client.cleanup && Instant::now() >= *next {
            let limits = client.json(&["limits", "--all"]);
            let safe = limits
                .as_ref()
                .is_ok_and(|limits| budget_safe(limits, client));
            if !safe {
                return Some("remote-budget-observation-or-reserve".into());
            }
            *next = Instant::now() + Duration::from_secs(CONTROL);
        }
        drop(next);
        let now = now_seconds();
        if self.silence.load(Ordering::Relaxed)
            && now.saturating_sub(self.last_output.load(Ordering::Relaxed)) >= self.patience
        {
            return Some("remote-no-progress".into());
        }
        None
    }

    /// Waits `span` in short steps, ending early with the guard's reason.
    fn wait(&self, client: &Client, span: Duration) -> Result<(), String> {
        let end = Instant::now() + span;
        while Instant::now() < end {
            self.check(client)?;
            std::thread::sleep(STEP.min(end.saturating_duration_since(Instant::now())));
        }
        self.check(client)
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for a time since the epoch.
fn utc(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// One entry's text: printable ASCII as is, every other byte as `\xNN` (so an entry is
/// one line and cannot drive a terminal), capped at `ENTRY_CAP`.
fn escape(text: &[u8]) -> String {
    let mut out = String::new();
    for (at, &byte) in text.iter().enumerate() {
        if out.len() >= ENTRY_CAP {
            out.push_str(&format!("...[truncated {} bytes]", text.len() - at));
            break;
        }
        match byte {
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    /// Attach starts and exits, status, result probes, cleanup steps.
    Control,
    /// Provider frames and the attach CLI's own stderr.
    Frame,
}

struct Ledger {
    file: Option<File>,
    written: u64,
    full: bool,
}

/// `runs/<id>/boat.log`: bounded, timestamped, one escaped line per event. Writing is
/// best effort; a failure never reaches the run.
pub(super) struct Evidence(Mutex<Ledger>);

impl Evidence {
    pub(super) fn new(file: Option<File>) -> Self {
        Self(Mutex::new(Ledger {
            file,
            written: 0,
            full: false,
        }))
    }

    fn entry(&self, class: Class, text: &[u8]) {
        let line = format!("{} {}\n", utc(SystemTime::now()), escape(text));
        let mut ledger = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let ledger = &mut *ledger;
        let Some(file) = ledger.file.as_mut() else {
            return;
        };
        let size = line.len() as u64;
        if class == Class::Frame && ledger.written + size > FRAME_CAP {
            if !ledger.full {
                ledger.full = true;
                let notice = format!(
                    "{} boat.log full; further frame/stderr entries dropped\n",
                    utc(SystemTime::now())
                );
                ledger.written += notice.len() as u64;
                let _ = file.write_all(notice.as_bytes());
            }
            return;
        }
        if ledger.written + size > HARD_CAP {
            return;
        }
        ledger.written += size;
        let _ = file.write_all(line.as_bytes());
    }

    pub(super) fn control(&self, text: impl AsRef<[u8]>) {
        self.entry(Class::Control, text.as_ref());
    }

    fn frame(&self, text: impl AsRef<[u8]>) {
        self.entry(Class::Frame, text.as_ref());
    }
}

/// A line of at most `cap` bytes, or how many bytes an over-long line had.
pub(super) enum Line {
    Read,
    Dropped(u64),
    End,
}

/// Reads one `\n`-terminated line into `line` without ever holding more than `cap`
/// bytes of it; a longer line is consumed to its newline and reported as dropped.
pub(super) fn read_line(
    input: &mut impl BufRead,
    line: &mut Vec<u8>,
    cap: usize,
) -> io::Result<Line> {
    line.clear();
    let mut dropped: Option<u64> = None;
    loop {
        let buffer = match input.fill_buf() {
            Ok(buffer) => buffer,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buffer.is_empty() {
            return Ok(match dropped {
                Some(n) => Line::Dropped(n),
                None if line.is_empty() => Line::End,
                None => Line::Read,
            });
        }
        let (chunk, done) = match buffer.iter().position(|&b| b == b'\n') {
            Some(at) => (&buffer[..=at], true),
            None => (buffer, false),
        };
        let used = chunk.len();
        match &mut dropped {
            Some(n) => *n += used as u64,
            None if line.len() + used > cap => {
                dropped = Some((line.len() + used) as u64);
                line.clear();
            }
            None => line.extend_from_slice(chunk),
        }
        input.consume(used);
        if done {
            return Ok(dropped.map_or(Line::Read, Line::Dropped));
        }
    }
}

/// A sink that remembers whether its output ends a line, so a marker starts on its own.
pub(super) struct Sink {
    write: Box<dyn Write + Send>,
    at_line_start: bool,
}

impl Sink {
    pub(super) fn new(write: Box<dyn Write + Send>) -> Self {
        Self {
            write,
            at_line_start: true,
        }
    }

    fn put(&mut self, bytes: &[u8]) {
        if let Some(&last) = bytes.last() {
            let _ = self.write.write_all(bytes);
            let _ = self.write.flush();
            self.at_line_start = last == b'\n';
        }
    }

    fn line(&mut self, text: &str) {
        if !self.at_line_start {
            self.put(b"\n");
        }
        self.put(text.as_bytes());
    }
}

/// Where a run's remote stdout/stderr go: the live log and rota's own stdout/stderr.
/// After a re-attach the replay is deduplicated by byte count only (best effort); each
/// re-attach is marked once in every sink before that attempt's first byte there.
pub(super) struct Stream {
    live: Sink,
    console: [Sink; 2],
    delivered: [u64; 2],
    seen: [u64; 2],
    /// The marker still owed to the live log and to rota's stderr. Stdout carries only
    /// command bytes, so the marker never goes there.
    owed: [Option<String>; 2],
}

impl Stream {
    pub(super) fn new(live: Sink, stdout: Sink, stderr: Sink) -> Self {
        Self {
            live,
            console: [stdout, stderr],
            delivered: [0; 2],
            seen: [0; 2],
            owed: [None, None],
        }
    }

    /// Starts a re-attach: the replay is counted from 0 again.
    fn reattach(&mut self) -> String {
        self.seen = [0; 2];
        let marker = format!(
            "[eon agent: Boat stream re-attached at stdout={} stderr={}; output near here may repeat or be missing]\n",
            self.delivered[0], self.delivered[1]
        );
        self.owed = [Some(marker.clone()), Some(marker)];
        format!(
            "reattach stdout={} stderr={}",
            self.delivered[0], self.delivered[1]
        )
    }

    /// One data frame of `stream` (0 stdout, 1 stderr); writes only bytes past what was
    /// already delivered. Works on bytes, never on `str` slices.
    pub(super) fn data(&mut self, stream: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // Both markers come before the attempt's first byte on either stream.
        if let Some(marker) = self.owed[0].take() {
            self.live.line(&marker);
        }
        if let Some(marker) = self.owed[1].take() {
            self.console[1].line(&marker);
        }
        let start = self.seen[stream];
        self.seen[stream] += bytes.len() as u64;
        let skip = self.delivered[stream]
            .saturating_sub(start)
            .min(bytes.len() as u64);
        let new = &bytes[skip as usize..];
        if !new.is_empty() {
            self.live.put(new);
            self.console[stream].put(new);
            self.delivered[stream] = self.seen[stream];
        }
    }
}

/// What one attach attempt saw.
#[derive(Default)]
pub(super) struct Attempt {
    pub exit_frame: bool,
    pub error_frame: bool,
}

/// Reads the attach CLI's JSON frames: data frames go to the stream, every other frame
/// and every non-JSON line to the evidence.
pub(super) fn frames(
    input: impl io::Read,
    stream: &mut Stream,
    evidence: &Evidence,
    guard: &Guard,
    attempt: &mut Attempt,
) {
    let mut input = io::BufReader::new(input);
    let mut line = Vec::new();
    loop {
        match read_line(&mut input, &mut line, LINE_CAP) {
            Err(_) | Ok(Line::End) => return,
            Ok(Line::Dropped(n)) => {
                evidence.frame(format!("frame dropped bytes={n}"));
                continue;
            }
            Ok(Line::Read) => (),
        }
        let frame = serde_json::from_slice::<Value>(&line).ok();
        let kind = frame.as_ref().and_then(|f| f["type"].as_str());
        let data = frame.as_ref().and_then(|f| f["data"].as_str());
        if let (Some(kind @ ("stdout" | "stderr")), Some(data)) = (kind, data) {
            if !data.is_empty() {
                guard.output();
            }
            stream.data(usize::from(kind == "stderr"), data.as_bytes());
            continue;
        }
        if let Some(frame) = &frame {
            attempt.exit_frame |= frame["type"] == "exit";
            attempt.error_frame |= frame["event"] == "error"
                || frame["type"] == "error"
                || frame["ok"] == false
                || frame["success"] == false;
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        evidence.frame([b"frame ".as_slice(), text].concat());
    }
}

/// The attach CLI's own stderr: diagnostics, to rota's stderr and the evidence (not the
/// live log, and not progress).
pub(super) fn cli_stderr(input: impl io::Read, evidence: &Evidence, terminal: &mut impl Write) {
    let mut input = io::BufReader::new(input);
    let mut line = Vec::new();
    loop {
        match read_line(&mut input, &mut line, LINE_CAP) {
            Ok(Line::Read) => {
                let _ = terminal.write_all(terminal_safe(&line).as_bytes());
                evidence.frame([b"stderr ".as_slice(), &line].concat());
            }
            Ok(Line::Dropped(n)) => {
                evidence.frame(format!("stderr line over the cap, {n} bytes dropped"))
            }
            Ok(Line::End) | Err(_) => break,
        }
    }
}

/// Provider text for a terminal: every control character except newline and tab (C0,
/// DEL, C1, so escape sequences) as `\xNN`, invalid UTF-8 replaced.
fn terminal_safe(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .chars()
        .map(|c| match c {
            '\n' | '\t' => c.to_string(),
            c if c.is_control() => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

enum Status {
    Ended,
    Running,
    Unreadable,
}

fn status(
    client: &Client,
    guard: &Guard,
    evidence: &Evidence,
    id: &str,
    process: &str,
) -> Result<Status, String> {
    let read = client.invoke(true, &["exec", id, "--status", process], Output::Saved);
    // A signal or the deadline inside the read is the run's reason, not an unreadable status.
    guard.check(client)?;
    let value = match read {
        Ok((code, bytes)) => {
            evidence.control([b"status ".as_slice(), &bytes].concat());
            super::decode(code, &bytes, "exec").ok()
        }
        Err(reason) => {
            evidence.control(format!("status error={reason}"));
            None
        }
    };
    Ok(match value.as_ref().and_then(|v| v["running"].as_bool()) {
        Some(true) => Status::Running,
        Some(false) => Status::Ended,
        None => Status::Unreadable,
    })
}

/// Whether the command's result is on the VM (run.sh moves it into place last).
fn result_present(client: &Client, evidence: &Evidence, id: &str, path: &Path) -> bool {
    let present = path.to_str().is_some_and(|local| {
        matches!(
            client.invoke(
                false,
                &[
                    "scp",
                    &format!("{id}:/home/user/eon-run/result.json"),
                    local
                ],
                Output::Saved,
            ),
            Ok((0, _))
        )
    });
    if !present {
        let _ = fs::remove_file(path);
    }
    evidence.control(if present {
        "result present"
    } else {
        "result absent"
    });
    present
}

/// Follows `process` until it ends (`Ok`) or the guard gives a reason to kill it.
pub(super) fn follow(
    client: &Client,
    guard: &Guard,
    (id, process): (&str, &str),
    stream: &mut Stream,
    evidence: &Evidence,
    result: &Path,
) -> Result<(), String> {
    // While a failed status read is unanswered, status is read again this often, also
    // during an attach, so only an unbroken run of failed reads gives up.
    let refresh = Duration::from_secs((guard.patience / 10).clamp(1, 30));
    let patience = Duration::from_secs(guard.patience);
    let unreadable: std::cell::Cell<Option<Instant>> = std::cell::Cell::new(None);
    let mut exit = "none".to_owned();
    let mut backoff = Duration::from_secs(1);
    let mut number = 0u32;
    // One fresh status read: Ok(true) when the command has ended, and the give-up once
    // reads have failed without a break for the patience.
    let read = |exit: &str| -> Result<bool, String> {
        match status(client, guard, evidence, id, process)? {
            Status::Ended => return Ok(true),
            Status::Running => unreadable.set(None),
            Status::Unreadable => {
                if result_present(client, evidence, id, result) {
                    return Ok(true);
                }
                let since = unreadable.get().unwrap_or_else(Instant::now);
                unreadable.set(Some(since));
                if since.elapsed() >= patience {
                    let reason = format!("remote-attach-lost exit={exit} status=unreadable");
                    return Err(guard.give_up(reason));
                }
            }
        }
        Ok(false)
    };
    loop {
        guard.check(client)?;
        number += 1;
        if number > 1 {
            evidence.control(stream.reattach());
        }
        evidence.control(format!(
            "attach start attempt={number} delivered_stdout={} delivered_stderr={}",
            stream.delivered[0], stream.delivered[1]
        ));
        let started = Instant::now();
        let mut attempt = Attempt::default();
        let attached = std::thread::scope(|scope| {
            let attaching = scope.spawn(|| {
                client.invoke(
                    true,
                    &["exec", id, "--attach", process],
                    Output::Attach(stream, evidence, guard, &mut attempt),
                )
            });
            let mut last = Instant::now();
            while !attaching.is_finished() {
                std::thread::sleep(STEP);
                if unreadable.get().is_some() && last.elapsed() >= refresh {
                    // An ended command's attach ends by itself; a give-up kills it.
                    if read(&exit).is_err() {
                        break;
                    }
                    last = Instant::now();
                }
            }
            attaching.join().unwrap()
        });
        exit = match &attached {
            Ok((code, _)) => {
                evidence.control(format!("attach exit={code}"));
                code.to_string()
            }
            Err(reason) => {
                evidence.control(format!("attach error={reason}"));
                "none".into()
            }
        };
        if matches!(attached, Ok((0, _))) && attempt.exit_frame && !attempt.error_frame {
            return Ok(());
        }
        // A guard reason ended the attempt (or arrives now): it wins over the loss.
        guard.check(client)?;
        guard.lose();
        if read(&exit)? {
            return Ok(());
        }
        if started.elapsed() >= HELD {
            backoff = Duration::from_secs(1);
        }
        guard.wait(client, backoff)?;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn budget_buffer_covers_concurrent_teardown_and_the_next_poll() {
        let mut client = Client::cleanup(crate::Machine::Large, 120, false);
        client.reserve_seconds = 14400.0;
        let mut limits = serde_json::json!({
            "accessTier": "trial",
            "subscriptionStatus": "trialing",
            "sandboxPlanKey": "trial",
            "keepGoing": {"enabled": false},
            "planIncluded": {"vmHoursRemaining": 20},
            "subscriptionRemainingSeconds": 72000,
            "trialComputeCapSeconds": 90000,
            "trialComputeUsedSeconds": 74800,
            "activeSandboxes": 1
        });
        assert!(budget_safe(&limits, &client));
        limits["activeSandboxes"] = 2.into();
        assert!(!budget_safe(&limits, &client));
        limits["activeSandboxes"] = 1.into();
        client.parallel_leases = 3;
        assert!(!budget_safe(&limits, &client));
        limits["trialComputeUsedSeconds"] = 70000.into();
        assert!(budget_safe(&limits, &client));
        limits["activeSandboxes"] = Value::Null;
        assert!(!budget_safe(&limits, &client));
    }

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn stream() -> (Stream, [Shared; 3]) {
        let sinks = [Shared::default(), Shared::default(), Shared::default()];
        let sink = |s: &Shared| Sink::new(Box::new(s.clone()));
        (
            Stream::new(sink(&sinks[0]), sink(&sinks[1]), sink(&sinks[2])),
            sinks,
        )
    }

    #[test]
    fn an_identical_rechunked_replay_writes_only_new_bytes_per_stream() {
        let (mut s, [live, out, err]) = stream();
        s.data(0, b"ab");
        s.data(1, b"E1");
        s.data(0, b"cd");
        s.reattach();
        s.data(0, b"abc");
        s.data(1, b"E");
        s.data(0, b"de");
        s.data(1, b"12");
        let marker = "[eon agent: Boat stream re-attached at stdout=4 stderr=2; output near here may repeat or be missing]\n";
        assert_eq!(out.text(), "abcde");
        assert_eq!(err.text(), format!("E1\n{marker}2"));
        assert_eq!(live.text(), format!("abE1cd\n{marker}e2"));
    }

    #[test]
    fn every_reattach_marks_the_live_log_and_stderr_once_and_never_stdout() {
        let (mut s, [live, out, err]) = stream();
        s.data(0, b"line\n");
        for _ in 0..2 {
            s.reattach();
            s.data(0, b"line\n");
            s.data(0, b"more\n");
        }
        s.reattach();
        s.data(1, b"oops\n");
        let live = live.text();
        assert_eq!(
            live.matches("[eon agent: Boat stream re-attached").count(),
            3
        );
        assert!(live.starts_with("line\n[eon agent: Boat stream re-attached at stdout=5 "));
        assert!(live.ends_with("stderr=0; output near here may repeat or be missing]\noops\n"));
        // Stdout holds only command bytes; stderr is marked for every re-attach,
        // even those that brought only stdout.
        assert_eq!(out.text(), "line\nmore\n");
        let err = err.text();
        assert_eq!(
            err.matches("[eon agent: Boat stream re-attached").count(),
            3
        );
        assert!(err.starts_with("[eon agent: Boat stream re-attached at stdout=5 stderr=0"));
        assert!(err.ends_with("]\noops\n"));
    }

    #[test]
    fn a_character_split_at_the_count_boundary_never_panics_and_keeps_bytes() {
        let text = "é─x".as_bytes();
        let (mut s, [live, out, _]) = stream();
        s.data(0, &text[..1]);
        s.data(0, &text[1..3]);
        s.reattach();
        s.data(0, &text[..2]);
        s.data(0, &text[2..]);
        assert_eq!(*out.0.lock().unwrap(), text);
        let marker = b"\n[eon agent: Boat stream re-attached at stdout=3 stderr=0; output near here may repeat or be missing]\n";
        let expected = [&text[..3], marker.as_slice(), &text[3..]].concat();
        assert_eq!(*live.0.lock().unwrap(), expected);
    }

    #[test]
    fn a_line_over_the_cap_is_dropped_without_holding_it() {
        let mut data = vec![b'a'; 9 * 1024 * 1024];
        data.extend_from_slice(b"\n{\"type\":\"exit\"}\ntail");
        let mut input = io::BufReader::new(io::Cursor::new(data));
        let mut line = Vec::new();
        match read_line(&mut input, &mut line, LINE_CAP).unwrap() {
            Line::Dropped(n) => assert_eq!(n, 9 * 1024 * 1024 + 1),
            _ => panic!("not dropped"),
        }
        assert!(line.len() <= LINE_CAP);
        assert!(matches!(
            read_line(&mut input, &mut line, LINE_CAP).unwrap(),
            Line::Read
        ));
        assert_eq!(line, b"{\"type\":\"exit\"}\n");
        assert!(matches!(
            read_line(&mut input, &mut line, LINE_CAP).unwrap(),
            Line::Read
        ));
        assert_eq!(line, b"tail");
        assert!(matches!(
            read_line(&mut input, &mut line, LINE_CAP).unwrap(),
            Line::End
        ));
        // A line just under the cap is kept.
        let mut input = io::BufReader::new(io::Cursor::new(b"abc\n".to_vec()));
        assert!(matches!(
            read_line(&mut input, &mut line, 4).unwrap(),
            Line::Read
        ));
        assert_eq!(line, b"abc\n");
    }

    fn evidence() -> (Evidence, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        (Evidence::new(Some(file.reopen().unwrap())), file)
    }

    fn lines(file: &tempfile::NamedTempFile) -> Vec<String> {
        fs::read_to_string(file.path())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn concurrent_entries_stay_whole_and_timestamped() {
        let (log, file) = evidence();
        std::thread::scope(|scope| {
            for writer in 0..4 {
                let log = &log;
                scope.spawn(move || {
                    for i in 0..5000 {
                        log.control(format!("w{writer} {i}"));
                    }
                });
            }
        });
        let lines = lines(&file);
        assert_eq!(lines.len(), 20_000);
        for line in &lines {
            let (time, text) = line.split_once(' ').unwrap();
            assert_eq!(time.len(), 24, "{line}");
            assert!(time.ends_with('Z') && time.as_bytes()[10] == b'T', "{line}");
            let (writer, i) = text.split_once(' ').unwrap();
            assert!(
                writer.starts_with('w') && i.parse::<u32>().is_ok(),
                "{line}"
            );
        }
    }

    #[test]
    fn frames_stop_at_their_cap_and_control_entries_at_the_hard_cap() {
        let (log, file) = evidence();
        let frame = "f".repeat(1000);
        for _ in 0..2000 {
            log.frame(&frame);
        }
        let size = fs::metadata(file.path()).unwrap().len();
        assert!(size <= FRAME_CAP + 200, "{size}");
        let control = "c".repeat(1000);
        for _ in 0..2000 {
            log.control(&control);
        }
        let lines = lines(&file);
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.ends_with("boat.log full; further frame/stderr entries dropped"))
                .count(),
            1
        );
        assert!(lines.iter().any(|l| l.ends_with(&control)));
        assert!(fs::metadata(file.path()).unwrap().len() <= HARD_CAP);
    }

    #[test]
    fn a_failing_evidence_file_is_ignored() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let log = Evidence::new(Some(File::open(file.path()).unwrap()));
        log.control("nothing lands");
        log.frame("nor this");
        assert_eq!(fs::metadata(file.path()).unwrap().len(), 0);
        Evidence::new(None).control("no file at all");
    }

    #[test]
    fn entries_are_escaped_to_one_line_and_capped() {
        assert_eq!(escape(b"a\r\x1b[2Jb\nc\\"), "a\\x0d\\x1b[2Jb\\x0ac\\\\");
        assert_eq!(escape("é".as_bytes()), "\\xc3\\xa9");
        let long = escape(&[b'x'; 5000]);
        assert!(long.starts_with(&"x".repeat(ENTRY_CAP)));
        assert!(long.ends_with("...[truncated 904 bytes]"), "{long}");
    }

    #[test]
    fn cli_stderr_reaches_the_terminal_without_control_sequences() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let evidence = Evidence::new(Some(file.reopen().unwrap()));
        let mut terminal = Vec::new();
        let input =
            "boat: first\n\tsecond \u{1b}[2J \u{9b}31m caf\u{e9}\nno newline\x7f".as_bytes();
        cli_stderr(input, &evidence, &mut terminal);
        assert_eq!(
            String::from_utf8(terminal).unwrap(),
            "boat: first\n\tsecond \\x1b[2J \\x9b31m caf\u{e9}\nno newline\\x7f"
        );
        let log = std::fs::read_to_string(file.path()).unwrap();
        assert!(log.contains(r"stderr \x09second \x1b[2J"), "{log}");
        assert!(log.contains("stderr no newline"), "{log}");
    }

    #[test]
    fn utc_formats_known_instants() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let t = UNIX_EPOCH + Duration::from_millis(1_795_651_200_123);
        assert_eq!(utc(t), "2026-11-26T00:00:00.123Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(utc(leap), "2000-02-29T00:00:00.000Z");
    }
}
