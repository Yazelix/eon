//! Bounded waiting for admission and the admission exit statuses (EON-C40 clause 7).
//!
//! `launch` and `run` make one admission attempt, or with `--wait DURATION` repeat it until it
//! admits, fails with anything but a hold, or the deadline passes. Each attempt is the whole
//! admission: it re-reads the configuration, takes the admission lock and the ledger, and reads
//! fresh quota. It holds the lock only while it runs, so a waiter never blocks other admissions
//! while it sleeps. Waiters are not ordered.

use crate::launch::Failure;
use std::io::Write;
use std::time::{Duration, Instant};

/// A hold: waiting can change it (quota, a spent observation, cap, access checks, project cap,
/// a busy admission lock, the wait deadline). sysexits' EX_TEMPFAIL.
pub const HOLD: u8 = 75;
/// A refusal waiting cannot change (no enabled backend serves the role, the options or the
/// directory; an unknown project or `--only` backend). sysexits' EX_UNAVAILABLE.
pub const REFUSED: u8 = 69;
/// `wait_retry_seconds` when the configuration leaves it out.
pub const DEFAULT_RETRY_SECONDS: u64 = 60;
/// The longest `--wait`: one day.
pub const MAX_WAIT_SECONDS: u64 = 24 * 3600;

/// `--wait DURATION`: how long admission may keep retrying, and the text the person gave.
#[derive(Debug, PartialEq, Eq)]
pub struct Wait {
    pub limit: Duration,
    pub text: String,
}

/// Parses `5400`, `5400s`, `90m` or `2h` into seconds between 1 and `max`.
pub fn parse_duration(text: &str, max: u64) -> Option<u64> {
    let (digits, unit) = match text.strip_suffix(['s', 'm', 'h']) {
        Some(digits) => (digits, &text[digits.len()..]),
        None => (text, "s"),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        _ => 3600,
    };
    digits
        .parse::<u64>()
        .ok()?
        .checked_mul(scale)
        .filter(|s| (1..=max).contains(s))
}

/// The `--wait` value, or the usage refusal (exit 2).
pub fn parse(text: &str) -> Result<Wait, Failure> {
    parse_duration(text, MAX_WAIT_SECONDS)
        .map(|seconds| Wait {
            limit: Duration::from_secs(seconds),
            text: text.to_owned(),
        })
        .ok_or_else(|| {
            (
                2,
                "bad --wait (use e.g. 300, 300s, 30m or 2h; at most 24h)".into(),
            )
        })
}

/// What one attempt is told, and the stderr lines it reports. The lines are printed after the
/// attempt returns, so they follow the release of its admission lock, and only when they differ
/// from the last attempt's.
pub struct Attempt {
    first: bool,
    lines: Vec<String>,
    reasons: Vec<serde_json::Value>,
    retry: Duration,
}

impl Attempt {
    /// The first attempt of this admission: the only one that may run a quota `refresh`.
    pub fn first(&self) -> bool {
        self.first
    }

    /// Adds one stderr line (without its newline).
    pub fn line(&mut self, text: String) {
        self.lines.push(text);
    }

    pub(crate) fn reason(&mut self, backend: &str, detail: &str) {
        self.reasons
            .push(serde_json::json!({ "backend": backend, "detail": detail }));
    }

    /// How long to sleep before the next attempt, from this attempt's configuration.
    pub fn retry_every(&mut self, retry: Duration) {
        self.retry = retry;
    }
}

fn stderr_line(text: &str) {
    // A waiter may outlive its caller's stderr; never panic here.
    let _ = writeln!(std::io::stderr(), "{text}");
}

/// Runs `attempt` until it succeeds, fails with anything but [`HOLD`], or the wait ends. Without
/// `wait` it runs once. Between attempts it sleeps up to the attempt's retry interval, never past
/// the deadline; the deadline gates the start of an attempt, a sleep cut to the deadline is
/// followed by one last attempt, and a hold at or after the deadline is final.
pub fn admit<T>(
    wait: Option<&Wait>,
    program: &str,
    json: bool,
    mut attempt: impl FnMut(&mut Attempt) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let mut reasons = Vec::new();
    retry_with(
        wait,
        program,
        Instant::now,
        std::thread::sleep,
        stderr_line,
        |current| {
            let result = attempt(current);
            if json {
                reasons = std::mem::take(&mut current.reasons);
            }
            result
        },
    )
    .inspect_err(|(code, detail)| {
        let code = match (json, *code) {
            (true, HOLD) => "agent-unavailable",
            (true, REFUSED) => "agent-unsupported",
            _ => return,
        };
        crate::cli::write_json(
            &serde_json::json!({ "error": { "code": code, "detail": detail, "reasons": reasons } }),
        );
    })
}

fn retry_with<T>(
    wait: Option<&Wait>,
    program: &str,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
    mut print: impl FnMut(&str),
    mut attempt: impl FnMut(&mut Attempt) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let deadline = wait.map(|wait| (now() + wait.limit, wait));
    let mut shown: Option<Vec<String>> = None;
    let mut first = true;
    loop {
        let mut current = Attempt {
            first,
            lines: Vec::new(),
            reasons: Vec::new(),
            retry: Duration::from_secs(DEFAULT_RETRY_SECONDS),
        };
        // The attempt's lock and records are dropped when it returns, before anything below.
        let result = attempt(&mut current);
        if shown.as_ref() != Some(&current.lines) {
            current.lines.iter().for_each(|line| print(line));
            shown = Some(current.lines);
        }
        let (code, message) = match result {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        let Some((deadline, wait)) = deadline.as_ref().filter(|_| code == HOLD) else {
            return Err((code, message));
        };
        let final_hold = || {
            Err((
                HOLD,
                format!("{message}; still held after waiting {}", wait.text),
            ))
        };
        let left = deadline.saturating_duration_since(now());
        if left.is_zero() {
            return final_hold();
        }
        if first {
            print(&format!(
                "{program}: held: {message}; retrying for up to {}",
                wait.text
            ));
        }
        // A sleep cut to the deadline runs over a little; the deadline attempt still follows, and
        // its hold is final. Only an uncut sleep that ran past the deadline skips it.
        let cut = current.retry >= left;
        sleep(current.retry.min(left));
        if !cut && now() > *deadline {
            return final_hold();
        }
        first = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn wait(seconds: u64) -> Wait {
        Wait {
            limit: Duration::from_secs(seconds),
            text: format!("{seconds}s"),
        }
    }

    /// A fake clock: `sleep` advances it by what it is asked plus `oversleep`, as a real sleep
    /// may run over, and an attempt may advance it too. Returns the result, the attempt start
    /// offsets and the sleeps, in whole seconds.
    fn drive(
        wait_for: Option<&Wait>,
        retry: u64,
        oversleep: Duration,
        mut outcome: impl FnMut(usize, &Cell<Instant>) -> Result<(), Failure>,
    ) -> (Result<(), Failure>, Vec<u64>, Vec<u64>, Vec<String>) {
        let start = Instant::now();
        let clock = Cell::new(start);
        let starts = RefCell::new(Vec::new());
        let sleeps = RefCell::new(Vec::new());
        let printed = RefCell::new(Vec::new());
        let result = retry_with(
            wait_for,
            "rota",
            || clock.get(),
            |d| {
                sleeps.borrow_mut().push(d.as_secs());
                clock.set(clock.get() + d + oversleep);
            },
            |line| printed.borrow_mut().push(line.to_owned()),
            |attempt| {
                attempt.retry_every(Duration::from_secs(retry));
                let n = starts.borrow().len();
                starts.borrow_mut().push((clock.get() - start).as_secs());
                attempt.line(format!("attempt lines {}", n.min(1)));
                outcome(n, &clock)
            },
        );
        (
            result,
            starts.into_inner(),
            sleeps.into_inner(),
            printed.into_inner(),
        )
    }

    fn held(_: usize, _: &Cell<Instant>) -> Result<(), Failure> {
        Err((HOLD, "no backend available, refusing".into()))
    }

    #[test]
    fn without_wait_one_attempt_and_no_sleep() {
        let (result, starts, sleeps, printed) = drive(None, 60, Duration::ZERO, held);
        assert_eq!(result, Err((HOLD, "no backend available, refusing".into())));
        assert_eq!((starts, sleeps), (vec![0], vec![]));
        assert_eq!(printed, ["attempt lines 0"]);
    }

    #[test]
    fn a_non_hold_ends_the_wait_at_once() {
        for code in [REFUSED, 1, 2] {
            let (result, starts, sleeps, printed) =
                drive(Some(&wait(3600)), 60, Duration::ZERO, |_, _| {
                    Err((code, "why".into()))
                });
            assert_eq!(result, Err((code, "why".into())));
            assert_eq!((starts, sleeps), (vec![0], vec![]));
            assert!(!printed.iter().any(|l| l.contains("held:")), "{printed:?}");
        }
    }

    #[test]
    fn a_waiter_admits_on_a_later_attempt() {
        let (result, starts, sleeps, printed) =
            drive(Some(&wait(600)), 60, Duration::ZERO, |n, _| {
                if n < 2 {
                    Err((HOLD, "no backend available, refusing".into()))
                } else {
                    Ok(())
                }
            });
        assert_eq!(result, Ok(()));
        assert_eq!((starts, sleeps), (vec![0, 60, 120], vec![60, 60]));
        // Unchanged lines print once; the held line once, after the first hold.
        assert_eq!(
            printed,
            [
                "attempt lines 0",
                "rota: held: no backend available, refusing; retrying for up to 600s",
                "attempt lines 1",
            ]
        );
    }

    #[test]
    fn the_last_attempt_starts_at_the_deadline() {
        let (result, starts, sleeps, _) = drive(Some(&wait(150)), 60, Duration::ZERO, held);
        assert_eq!((starts, sleeps), (vec![0, 60, 120, 150], vec![60, 60, 30]));
        assert_eq!(
            result,
            Err((
                HOLD,
                "no backend available, refusing; still held after waiting 150s".into()
            ))
        );
    }

    #[test]
    fn a_wait_shorter_than_the_interval_makes_two_attempts() {
        let (result, starts, sleeps, _) = drive(Some(&wait(2)), 60, Duration::ZERO, held);
        assert_eq!((starts, sleeps), (vec![0, 2], vec![2]));
        assert_eq!(result.unwrap_err().0, HOLD);
    }

    #[test]
    fn a_sleep_cut_to_the_deadline_that_runs_over_still_makes_the_deadline_attempt() {
        // A real sleep always runs a little over; the attempt it was cut for still happens.
        let over = Duration::from_millis(1);
        let (result, starts, sleeps, _) = drive(Some(&wait(2)), 60, over, held);
        assert_eq!((starts, sleeps), (vec![0, 2], vec![2]));
        assert_eq!(
            result.unwrap_err().1,
            "no backend available, refusing; still held after waiting 2s"
        );
        // Each uncut sleep runs over too, so the cut one is 29.998 s.
        let (_, starts, sleeps, _) = drive(Some(&wait(150)), 60, over, held);
        assert_eq!((starts, sleeps), (vec![0, 60, 120, 150], vec![60, 60, 29]));
    }

    #[test]
    fn an_oversleep_past_the_deadline_makes_no_further_attempt() {
        let (result, starts, sleeps, _) =
            drive(Some(&wait(100)), 60, Duration::from_secs(50), held);
        assert_eq!((starts, sleeps), (vec![0], vec![60]));
        assert_eq!(
            result.unwrap_err().1,
            "no backend available, refusing; still held after waiting 100s"
        );
    }

    #[test]
    fn an_attempt_that_crosses_the_deadline_is_final() {
        let (result, starts, sleeps, _) =
            drive(Some(&wait(100)), 60, Duration::ZERO, |n, clock| {
                if n == 1 {
                    clock.set(clock.get() + Duration::from_secs(45));
                }
                held(n, clock)
            });
        assert_eq!((starts, sleeps), (vec![0, 60], vec![60]));
        assert_eq!(result.unwrap_err().0, HOLD);
    }

    #[test]
    fn durations_parse_with_their_bounds() {
        assert_eq!(parse_duration("5400", 7200), Some(5400));
        assert_eq!(parse_duration("90m", 7200), Some(5400));
        assert_eq!(parse_duration("2h", 7200), Some(7200));
        for bad in ["", "0", "3h", "-1", "1d", "1.5h", "h", "+5"] {
            assert_eq!(parse_duration(bad, 7200), None, "{bad}");
        }
        assert_eq!(parse("24h").unwrap().limit, Duration::from_secs(86_400));
        assert_eq!(parse("25h").unwrap_err().0, 2);
    }
}
