//! Steps: each runs to its own deadline, writes its own log and ends with
//! one status; the summary renders them.

use serde_json::json;
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// How long a killed child gets between TERM and KILL.
const GRACE: Duration = Duration::from_secs(10);
pub const POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Timeout,
    /// A step it requires did not pass, the run's time limit had passed, or
    /// the step cannot run on this machine.
    Skipped,
    /// A dry run records the step without running it.
    Planned,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Timeout => "timeout",
            Status::Skipped => "skipped",
            Status::Planned => "planned",
        }
    }
}

pub enum Failure {
    /// The deadline passed.
    Timeout,
    /// A command failed; its own output names the cause better than this.
    Command(String),
    /// A check of the tool's own failed; this message is the cause.
    Check(String),
    /// The step cannot run on this machine, so a person must check it.
    Unavailable(String),
}

pub type Outcome = Result<(), Failure>;

pub fn check(condition: bool, message: impl FnOnce() -> String) -> Outcome {
    if condition {
        Ok(())
    } else {
        Err(Failure::Check(message()))
    }
}

pub struct Log {
    file: File,
}

impl Log {
    pub fn new(file: File) -> Self {
        Log { file }
    }

    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.file, "{text}");
    }

    pub fn stdio(&self) -> io::Result<Stdio> {
        Ok(Stdio::from(self.file.try_clone()?))
    }
}

/// Runs `command` with no stdin until it exits or `deadline` passes, with its
/// output in `log`. With `capture`, stdout is returned and also logged.
/// `group` starts it in its own process group, which a timeout kills whole.
pub fn execute(
    log: &mut Log,
    command: &mut Command,
    deadline: Instant,
    capture: bool,
    group: bool,
) -> Result<(ExitStatus, Vec<u8>), Failure> {
    execute_filtered(log, command, deadline, capture, group, Ok)
}

/// Filters captured stdout before either returning or logging it.
pub fn execute_filtered(
    log: &mut Log,
    command: &mut Command,
    deadline: Instant,
    capture: bool,
    group: bool,
    filter: impl FnOnce(Vec<u8>) -> Result<Vec<u8>, Failure>,
) -> Result<(ExitStatus, Vec<u8>), Failure> {
    log.line(&format!("$ {command:?}"));
    let started = (|| {
        command.stdin(Stdio::null()).stderr(log.stdio()?);
        command.stdout(if capture {
            Stdio::piped()
        } else {
            log.stdio()?
        });
        if group {
            std::os::unix::process::CommandExt::process_group(command, 0);
        }
        command.spawn()
    })();
    let mut child = started.map_err(|error| Failure::Command(format!("cannot start: {error}")))?;
    let reader = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        })
    });
    let Some(status) = wait(&mut child, deadline) else {
        log.line("mac-proof: deadline passed; stopping it");
        stop(&mut child, group);
        return Err(Failure::Timeout);
    };
    let mut stdout = Vec::new();
    if let Some(reader) = reader {
        // A descendant that outlives the command can hold stdout open.
        while !reader.is_finished() {
            if Instant::now() >= deadline {
                log.line("mac-proof: deadline passed with stdout still open; stopping its group");
                if group {
                    let _ = signal("KILL", child.id());
                }
                return Err(Failure::Timeout);
            }
            thread::sleep(POLL);
        }
        stdout = filter(reader.join().unwrap_or_default())?;
        log.line(&String::from_utf8_lossy(&stdout));
    }
    Ok((status, stdout))
}

/// Like `execute`, but a nonzero exit fails the step.
pub fn succeed(
    log: &mut Log,
    command: &mut Command,
    deadline: Instant,
    capture: bool,
) -> Result<Vec<u8>, Failure> {
    let program = command.get_program().to_string_lossy().into_owned();
    let (status, stdout) = execute(log, command, deadline, capture, true)?;
    if status.success() {
        Ok(stdout)
    } else {
        Err(Failure::Command(format!("{program} exited with {status}")))
    }
}

fn wait(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            _ => return None,
        }
    }
}

/// Sends the child's process group TERM, then KILL once the child exits or
/// the grace period ends, so no member outlives it; without `group`, kills
/// the child alone.
pub fn stop(child: &mut Child, group: bool) {
    if group {
        let _ = signal("TERM", child.id());
        wait(child, Instant::now() + GRACE);
        let _ = signal("KILL", child.id());
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Signals process group `group` through the shell's `kill`, whose `--` and
/// negative pids Linux and macOS read alike.
fn signal(name: &str, group: u32) -> io::Result<ExitStatus> {
    Command::new("/bin/sh")
        .args(["-c", r#"kill -s "$0" -- "-$1""#, name, &group.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

struct Record {
    name: String,
    status: Status,
    duration: Duration,
    log: String,
    first_failure: Option<String>,
    /// Skipped because it, or a step it requires, could not run here.
    for_yu: bool,
}

/// Runs steps in order into `evidence/steps/NN-name.log` and keeps their records.
pub struct Runner {
    evidence: PathBuf,
    dry_run: bool,
    /// No step runs past this.
    end: Instant,
    records: Vec<Record>,
    /// Written at the head of the summary.
    pub identity: Vec<(&'static str, String)>,
}

impl Runner {
    pub fn new(evidence: &Path, dry_run: bool, limit: Duration) -> io::Result<Self> {
        fs::create_dir_all(evidence.join("steps"))?;
        Ok(Runner {
            evidence: evidence.to_owned(),
            dry_run,
            end: Instant::now() + limit,
            records: Vec::new(),
            identity: Vec::new(),
        })
    }

    fn passed(&self, name: &str) -> bool {
        self.records
            .iter()
            .any(|record| record.name == name && record.status == Status::Pass)
    }

    pub fn status(&self, name: &str) -> Option<&str> {
        self.records
            .iter()
            .find(|record| record.name == name)
            .map(|record| record.status.as_str())
    }

    fn for_yu(&self, name: &str) -> bool {
        self.records
            .iter()
            .any(|record| record.name == name && record.for_yu)
    }

    /// Runs `body` unless a step in `requires` did not pass or the run's
    /// time limit has passed, then rewrites the summary. A dry run runs only
    /// bodies marked `always`.
    pub fn step<C>(
        &mut self,
        context: &mut C,
        name: &str,
        timeout: Duration,
        requires: &[&str],
        always: bool,
        body: impl FnOnce(&mut C, &mut Log, Instant) -> Outcome,
    ) -> io::Result<()> {
        let log = format!(
            "steps/{:02}-{}.log",
            self.records.len() + 1,
            name.replace(':', "-")
        );
        let mut file = Log {
            file: File::create(self.evidence.join(&log))?,
        };
        let started = Instant::now();
        let missing: Vec<&str> = requires
            .iter()
            .copied()
            .filter(|required| !self.passed(required))
            .collect();
        let mut for_yu = false;
        let (status, message) = if self.dry_run && !always {
            file.line("mac-proof: dry run; not run");
            (Status::Planned, None)
        } else if !missing.is_empty() || started >= self.end {
            let message = if missing.is_empty() {
                "the run's time limit passed".to_owned()
            } else if missing.iter().all(|required| self.for_yu(required)) {
                for_yu = true;
                format!("needs {}, which could not run here", missing.join(", "))
            } else {
                format!("needs {}, which did not pass", missing.join(", "))
            };
            file.line(&format!("mac-proof: skipped: {message}"));
            (Status::Skipped, Some(message))
        } else {
            file.line(&format!(
                "mac-proof: step {name}, timeout {}",
                format_duration(timeout)
            ));
            match body(context, &mut file, self.end.min(started + timeout)) {
                Ok(()) => (Status::Pass, None),
                Err(Failure::Timeout) => (
                    Status::Timeout,
                    Some(format!(
                        "timed out after {}",
                        format_duration(started.elapsed())
                    )),
                ),
                Err(Failure::Command(message)) => {
                    file.line(&format!("mac-proof: {message}"));
                    // Build and test output need not be UTF-8.
                    let text = fs::read(self.evidence.join(&log)).unwrap_or_default();
                    let text = String::from_utf8_lossy(&text);
                    (Status::Fail, Some(first_failure(&text).unwrap_or(message)))
                }
                Err(Failure::Check(message)) => {
                    file.line(&format!("mac-proof: {message}"));
                    (Status::Fail, Some(message))
                }
                Err(Failure::Unavailable(message)) => {
                    file.line(&format!("mac-proof: skipped: {message}"));
                    for_yu = true;
                    (Status::Skipped, Some(message))
                }
            }
        };
        let record = Record {
            name: name.to_owned(),
            status,
            duration: started.elapsed(),
            log,
            first_failure: message,
            for_yu,
        };
        progress(&format!(
            "{:<32} {:<8} {}",
            record.name,
            status.as_str(),
            format_duration(record.duration)
        ));
        self.records.push(record);
        self.write(false)?;
        Ok(())
    }

    pub fn left_for_yu(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.status == Status::Skipped && record.for_yu)
            .count()
    }

    /// Writes summary.json and summary.txt and returns whether every step
    /// passed, was planned, or was skipped for Yu. Until the last step has
    /// run (`complete`), their result is `incomplete`, so a run that dies
    /// midway reads as such.
    pub fn write(&self, complete: bool) -> io::Result<bool> {
        let identity = &self.identity;
        let passed = self.records.iter().all(|record| {
            matches!(record.status, Status::Pass | Status::Planned)
                || (record.status == Status::Skipped && record.for_yu)
        });
        let result = match (complete, passed) {
            (false, _) => "incomplete",
            (true, true) => "pass",
            (true, false) => "fail",
        };
        fs::write(
            self.evidence.join("summary.json"),
            summary_json(&self.records, identity, self.dry_run, result) + "\n",
        )?;
        fs::write(
            self.evidence.join("summary.txt"),
            summary_text(&self.records, identity, self.dry_run, result),
        )?;
        Ok(passed)
    }
}

/// A line of progress for whoever watches; a closed stdout must not stop the run.
pub fn progress(text: &str) {
    let _ = writeln!(io::stdout(), "{text}");
}

/// The first log line that reads as a failure, from Cargo, rustc, Nix or a test.
fn first_failure(log: &str) -> Option<String> {
    log.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("$ "))
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            line.contains("FAILED")
                || lower.contains("error:")
                || lower.contains("error[")
                || line.contains("panicked at")
        })
        .map(|line| line.chars().take(200).collect())
}

pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m{s:02}s"),
        (h, m, _) => format!("{h}h{m:02}m"),
    }
}

/// Reads `30s`, `90m` or `12h`.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.len().checked_sub(1)?;
    let (number, unit) = text.split_at_checked(split)?;
    let number: u64 = number.parse().ok().filter(|number| *number > 0)?;
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return None,
    };
    Some(Duration::from_secs(number.checked_mul(scale)?))
}

fn summary_json(
    records: &[Record],
    identity: &[(&str, String)],
    dry_run: bool,
    result: &str,
) -> String {
    let steps: Vec<_> = records
        .iter()
        .map(|record| {
            json!({
                "name": record.name,
                "status": record.status.as_str(),
                "seconds": record.duration.as_secs(),
                "log": record.log,
                "first_failure": record.first_failure,
                "for_yu": record.for_yu,
            })
        })
        .collect();
    let mut summary = json!({
        "schema": 1,
        "result": result,
        "dry_run": dry_run,
        "steps": steps,
    });
    for (key, value) in identity {
        summary[*key] = json!(value);
    }
    serde_json::to_string_pretty(&summary).expect("a JSON value serializes")
}

fn summary_text(
    records: &[Record],
    identity: &[(&str, String)],
    dry_run: bool,
    result: &str,
) -> String {
    let failed = records
        .iter()
        .filter(|record| !matches!(record.status, Status::Pass | Status::Planned))
        .count();
    let remaining = if result == "pass" && failed > 0 {
        format!("{failed} left for Yu")
    } else {
        format!("{failed} not passed")
    };
    let mut text = format!(
        "mac-proof {}{}: {} steps, {remaining}\n",
        if dry_run { "dry run " } else { "" },
        result,
        records.len()
    );
    for (key, value) in identity {
        text += &format!("{key}: {value}\n");
    }
    text += &format!(
        "\n{:<32} {:<8} {:>7}  first failing line\n",
        "step", "status", "time"
    );
    for record in records {
        text += format!(
            "{:<32} {:<8} {:>7}  {}",
            record.name,
            record.status.as_str(),
            format_duration(record.duration),
            record.first_failure.as_deref().unwrap_or("")
        )
        .trim_end();
        text += "\n";
    }
    let for_yu: Vec<&Record> = records.iter().filter(|record| record.for_yu).collect();
    if !for_yu.is_empty() {
        text += "\nfor Yu, since they could not run here:\n";
        for record in for_yu {
            text += &format!(
                "  {:<30} {}\n",
                record.name,
                record.first_failure.as_deref().unwrap_or("")
            );
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("mac-proof-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn steps_after_a_failure_are_skipped_and_the_summary_names_its_line() {
        let evidence = scratch("runner");
        let mut runner = Runner::new(&evidence, false, Duration::from_secs(3600)).unwrap();
        runner.identity.push(("tree", "abc".to_owned()));
        let minute = Duration::from_secs(60);
        let mut ran = Vec::new();
        runner
            .step(
                &mut ran,
                "build",
                minute,
                &[],
                false,
                |ran, log, deadline| {
                    ran.push("build");
                    succeed(
                        log,
                        Command::new("sh")
                            // Output need not be UTF-8.
                            .args([
                                "-c",
                                r"printf 'compiling \377\n'; echo 'error: no space' >&2; exit 3",
                            ]),
                        deadline,
                        false,
                    )
                    .map(drop)
                },
            )
            .unwrap();
        runner
            .step(
                &mut ran,
                "test:a",
                minute,
                &["build"],
                false,
                |ran, _, _| {
                    ran.push("test:a");
                    Ok(())
                },
            )
            .unwrap();
        runner
            .step(&mut ran, "leftovers", minute, &[], false, |ran, _, _| {
                ran.push("leftovers");
                check(false, || "1 stale directory".to_owned())
            })
            .unwrap();
        assert_eq!(ran, ["build", "leftovers"]);
        let read = |name: &str| -> serde_json::Value {
            serde_json::from_slice(&fs::read(evidence.join(name)).unwrap()).unwrap()
        };
        assert_eq!(read("summary.json")["result"], "incomplete");
        assert!(!runner.write(true).unwrap());

        let text = fs::read_to_string(evidence.join("summary.txt")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "mac-proof fail: 3 steps, 3 not passed");
        assert_eq!(lines[1], "tree: abc");
        assert!(lines[4].starts_with("build ") && lines[4].ends_with("  error: no space"));
        assert!(
            lines[5].contains(" skipped ") && lines[5].ends_with("needs build, which did not pass")
        );
        assert!(lines[6].ends_with("1 stale directory"));

        let summary = read("summary.json");
        assert_eq!(summary["result"], "fail");
        assert_eq!(summary["tree"], "abc");
        assert_eq!(summary["steps"][1]["status"], "skipped");
        assert_eq!(summary["steps"][2]["log"], "steps/03-leftovers.log");
        let log = fs::read(evidence.join("steps/01-build.log")).unwrap();
        let log = String::from_utf8_lossy(&log);
        assert!(
            log.contains("compiling \u{fffd}\n") && log.contains("sh exited with exit status: 3")
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn a_step_that_cannot_run_here_is_listed_for_yu_with_the_steps_that_need_it() {
        let evidence = scratch("for-yu");
        let mut runner = Runner::new(&evidence, false, Duration::from_secs(3600)).unwrap();
        let minute = Duration::from_secs(60);
        runner
            .step(&mut (), "launch", minute, &[], false, |_, _, _| Ok(()))
            .unwrap();
        runner
            .step(&mut (), "picker", minute, &["launch"], false, |_, _, _| {
                Err(Failure::Unavailable(
                    "needs Accessibility: not granted".to_owned(),
                ))
            })
            .unwrap();
        runner
            .step(&mut (), "broken", minute, &[], false, |_, _, _| {
                check(false, || "no window".to_owned())
            })
            .unwrap();
        for (name, requires) in [
            ("output", &["picker"][..]),
            ("focus", &["picker", "broken"]),
        ] {
            runner
                .step(
                    &mut (),
                    name,
                    minute,
                    requires,
                    false,
                    |_, _, _| unreachable!(),
                )
                .unwrap();
        }
        assert!(!runner.write(true).unwrap());
        let text = fs::read_to_string(evidence.join("summary.txt")).unwrap();
        let (_, for_yu) = text
            .split_once("\nfor Yu, since they could not run here:\n")
            .unwrap();
        let listed: Vec<(&str, &str)> = for_yu
            .lines()
            .filter_map(|line| line.trim().split_once(' '))
            .map(|(name, reason)| (name, reason.trim()))
            .collect();
        assert_eq!(
            listed,
            [
                ("picker", "needs Accessibility: not granted"),
                ("output", "needs picker, which could not run here"),
            ]
        );
        let summary: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["result"], "fail");
        let marks: Vec<_> = summary["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| (step["status"].as_str().unwrap(), step["for_yu"] == true))
            .collect();
        assert_eq!(
            marks,
            [
                ("pass", false),
                ("skipped", true),
                ("fail", false),
                ("skipped", true),
                ("skipped", false)
            ]
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn a_run_passes_with_only_for_yu_skips_but_not_other_dependency_skips() {
        let evidence = scratch("for-yu-verdict");
        let mut runner = Runner::new(&evidence, false, Duration::from_secs(3600)).unwrap();
        let minute = Duration::from_secs(60);
        runner
            .step(&mut (), "launch", minute, &[], false, |_, _, _| Ok(()))
            .unwrap();
        runner
            .step(&mut (), "picker", minute, &["launch"], false, |_, _, _| {
                Err(Failure::Unavailable(
                    "needs Accessibility: not granted".to_owned(),
                ))
            })
            .unwrap();
        runner
            .step(
                &mut (),
                "output",
                minute,
                &["picker"],
                false,
                |_, _, _| unreachable!(),
            )
            .unwrap();
        assert!(runner.write(true).unwrap());
        assert_eq!(runner.left_for_yu(), 2);
        let summary: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["result"], "pass");
        for index in [1, 2] {
            assert_eq!(summary["steps"][index]["status"], "skipped");
            assert_eq!(summary["steps"][index]["for_yu"], true);
        }
        let text = fs::read_to_string(evidence.join("summary.txt")).unwrap();
        assert!(text.starts_with("mac-proof pass: 3 steps, 2 left for Yu\n"));
        let (_, for_yu) = text
            .split_once("\nfor Yu, since they could not run here:\n")
            .unwrap();
        assert!(for_yu.contains("picker") && for_yu.contains("output"));

        runner
            .step(
                &mut (),
                "focus",
                minute,
                &["package"],
                false,
                |_, _, _| unreachable!(),
            )
            .unwrap();
        assert!(!runner.write(true).unwrap());
        let summary: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["result"], "fail");
        assert_eq!(summary["steps"][3]["status"], "skipped");
        assert_eq!(summary["steps"][3]["for_yu"], false);
        assert_eq!(
            summary["steps"][3]["first_failure"],
            "needs package, which did not pass"
        );
        let text = fs::read_to_string(evidence.join("summary.txt")).unwrap();
        assert!(text.starts_with("mac-proof fail: 4 steps, 3 not passed\n"));
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn a_dry_run_runs_only_steps_marked_always_and_no_step_starts_past_the_limit() {
        let evidence = scratch("dry");
        let mut runner = Runner::new(&evidence, true, Duration::from_secs(3600)).unwrap();
        let minute = Duration::from_secs(60);
        runner
            .step(&mut (), "source", minute, &[], true, |_, _, _| Ok(()))
            .unwrap();
        runner
            .step(
                &mut (),
                "package",
                minute,
                &["source"],
                false,
                |_, _, _| unreachable!(),
            )
            .unwrap();
        assert!(runner.write(true).unwrap());
        let statuses: Vec<_> = runner.records.iter().map(|record| record.status).collect();
        assert_eq!(statuses, [Status::Pass, Status::Planned]);
        let text = fs::read_to_string(evidence.join("summary.txt")).unwrap();
        assert!(text.starts_with("mac-proof dry run pass: 2 steps, 0 not passed\n"));
        fs::remove_dir_all(&evidence).unwrap();

        let mut runner = Runner::new(&evidence, false, Duration::ZERO).unwrap();
        runner
            .step(
                &mut (),
                "host",
                minute,
                &[],
                false,
                |_, _, _| unreachable!(),
            )
            .unwrap();
        assert!(!runner.write(true).unwrap());
        assert_eq!(runner.records[0].status, Status::Skipped);
        assert_eq!(
            runner.records[0].first_failure.as_deref(),
            Some("the run's time limit passed")
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn a_timeout_leaves_no_member_of_the_process_group() {
        let evidence = scratch("timeout");
        fs::create_dir_all(&evidence).unwrap();
        let pid_file = evidence.join("pid");
        let mut log = Log {
            file: File::create(evidence.join("log")).unwrap(),
        };
        // The shell dies of TERM but its worker ignores it; the shell exits
        // at once while its worker holds the captured stdout open; both
        // ignore TERM, so only KILL after the grace period ends them.
        for script in [
            "(trap '' TERM; exec sleep 600) & echo $! > \"$0\"; wait",
            "sleep 600 & echo $! > \"$0\"",
            "trap '' TERM; sleep 600 & echo $! > \"$0\"; wait",
        ] {
            let started = Instant::now();
            let result = execute(
                &mut log,
                Command::new("sh").args(["-c", script]).arg(&pid_file),
                started + Duration::from_secs(1),
                true,
                true,
            );
            assert!(matches!(result, Err(Failure::Timeout)), "{script}");
            assert!(started.elapsed() < GRACE * 2, "{script}");
            let pid = fs::read_to_string(&pid_file).unwrap();
            let gone = (0..50).any(|_| {
                let alive = Command::new("kill")
                    .args(["-0", pid.trim()])
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                thread::sleep(POLL);
                !alive
            });
            assert!(gone, "{script}: the worker {pid} outlived the timeout");
        }
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn failure_lines_durations_and_their_text() {
        let log = "$ \"cargo\" \"test\" error: in argv\n\
                   test result: ok. 3 passed; 0 failed\n\
                   test pty::resize ... FAILED\n\
                   error: test failed, to rerun pass `--lib`\n";
        assert_eq!(
            first_failure(log).as_deref(),
            Some("test pty::resize ... FAILED")
        );
        assert_eq!(first_failure("warning: unused\nall good\n"), None);

        assert_eq!(parse_duration("90m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("12h"), Some(Duration::from_secs(43200)));
        for bad in ["", "m", "0s", "5", "5d", "-5s", "1.5h"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
        assert_eq!(format_duration(Duration::from_secs(9)), "9s");
        assert_eq!(format_duration(Duration::from_secs(185)), "3m05s");
        assert_eq!(format_duration(Duration::from_secs(3720)), "1h02m");
    }
}
