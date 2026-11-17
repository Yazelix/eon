use crate::admissions::Ledger;
use crate::config::{Backend, Cap, Config, Reader};
use crate::decision::Reading;
use crate::quota;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub fn command(backend: &Backend, argv: &[String]) -> Command {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    for key in &backend.unset_env {
        cmd.env_remove(key);
    }
    cmd.envs(&backend.env);
    cmd
}

/// How long rota waits for a process it killed to exit before giving up on it, so a process
/// stuck in uninterruptible I/O cannot hold rota up.
pub const REAP: Duration = Duration::from_secs(1);

/// Whether `child`, already sent SIGKILL, exits within `limit`.
pub fn reap_within(child: &mut Child, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// SIGKILLs the process group a check runs in.
fn kill_group(child: &mut Child) {
    // The unreaped child's PID identifies only the process group we created.
    let _ = rustix::process::kill_process_group(
        rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
        rustix::process::Signal::KILL,
    );
    let _ = child.kill();
}

pub fn probe(
    backend: &Backend,
    argv: &[String],
    seconds: u64,
    cwd: &Path,
) -> Result<String, String> {
    probe_command(command(backend, argv), seconds, cwd)
}

/// A configured check with the same bounded output and cleanup as backend probes.
pub fn probe_command(cmd: Command, seconds: u64, cwd: &Path) -> Result<String, String> {
    probe_killing(cmd, seconds, cwd, kill_group)
}

/// `probe`, ending the check with `kill` when it must stop early.
fn probe_killing(
    mut command: Command,
    seconds: u64,
    cwd: &Path,
    kill: fn(&mut Child),
) -> Result<String, String> {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or("check timeout too large")?;
    let mut child = command
        .process_group(0)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "could not execute check")?;
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let stop = |child: &mut Child, problem: &str| -> String {
        kill(child);
        match reap_within(child, REAP) {
            true => problem.to_string(),
            false => format!("{problem}; still running after SIGKILL"),
        }
    };
    let mut bytes = None;
    loop {
        if bytes.is_none() {
            match receiver.try_recv() {
                Ok(Ok(output)) if output.len() <= 1_048_576 => bytes = Some(output),
                Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(stop(&mut child, "check output unreadable or exceeds 1 MiB"));
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        // Keep the PID unreaped while a descendant may still hold stdout open.
        if let Some(output) = &mut bytes {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    return String::from_utf8(std::mem::take(output))
                        .map_err(|_| "check returned non-UTF-8 output".into());
                }
                Ok(Some(status)) => return Err(format!("check failed ({status})")),
                Ok(None) => (),
                Err(_) => {
                    return Err(stop(&mut child, "cannot wait for check"));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(stop(&mut child, "check timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn canonical_equal(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    a == b
        || fs::canonicalize(a)
            .ok()
            .zip(fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

fn executable_matches(
    cap: &Cap,
    argv: &[String],
    actual: Option<&Path>,
    resolved: Option<&Path>,
) -> bool {
    argv.first().is_some_and(|a| {
        Path::new(a)
            .file_name()
            .is_some_and(|n| n == cap.executable.as_str())
    }) || actual.is_some_and(|p| {
        p.file_name().is_some_and(|n| n == cap.executable.as_str())
            || resolved == Some(p)
            || cap
                .runtime_executables
                .iter()
                .any(|runtime| fs::canonicalize(runtime).ok().as_deref() == Some(p))
    })
}

fn matches_filters(cap: &Cap, argv: &[String], env: &BTreeMap<String, String>, cwd: &Path) -> bool {
    cap.script.as_ref().is_none_or(|script| {
        argv.iter()
            .skip(1)
            .any(|arg| canonical_equal(cwd.join(arg), script))
    }) && !argv.iter().any(|arg| cap.exclude_args.contains(arg))
        && cap.env.iter().all(|(key, value)| {
            env.get(key)
                .is_some_and(|v| v == value || canonical_equal(cwd.join(v), value))
        })
}

fn split_null(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

pub fn running(cap: &Cap) -> Result<usize, String> {
    running_in(Path::new("/proc"), rustix::process::geteuid().as_raw(), cap)
}

fn owned_by(dir: &Path, uid: u32) -> bool {
    fs::metadata(dir).is_ok_and(|meta| meta.uid() == uid)
}

/// The cap is per user: only `uid`'s processes count. A process that exits or
/// changes owner mid-scan is skipped. A live process of `uid` that matches by
/// command line but cannot be inspected fails the scan, as does an unlistable
/// `proc`.
fn running_in(proc: &Path, uid: u32, cap: &Cap) -> Result<usize, String> {
    let mut count = 0;
    let resolved = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .find_map(|dir| fs::canonicalize(dir.join(&cap.executable)).ok());
    for entry in fs::read_dir(proc).map_err(|_| "cannot scan /proc for concurrency cap")? {
        let entry = entry.map_err(|_| "cannot read process entry")?;
        if !entry
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            continue;
        }
        if !owned_by(&entry.path(), uid) {
            continue;
        }
        let Ok(cmdline) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let argv = split_null(&cmdline);
        let actual = fs::read_link(entry.path().join("exe")).ok();
        if !executable_matches(cap, &argv, actual.as_deref(), resolved.as_deref()) {
            continue;
        }
        let gone = |e: &std::io::Error| {
            e.kind() == std::io::ErrorKind::NotFound || !owned_by(&entry.path(), uid)
        };
        let env = if cap.env.is_empty() {
            BTreeMap::new()
        } else {
            match fs::read(entry.path().join("environ")) {
                Ok(bytes) => split_null(&bytes)
                    .into_iter()
                    .filter_map(|s| s.split_once('=').map(|(k, v)| (k.into(), v.into())))
                    .collect(),
                Err(e) if gone(&e) => continue,
                Err(_) => return Err("cannot inspect matching process environment".into()),
            }
        };
        let cwd = if cap.script.is_some() || !cap.env.is_empty() {
            match fs::read_link(entry.path().join("cwd")) {
                Ok(cwd) => cwd,
                Err(e) if gone(&e) => continue,
                Err(_) => return Err("cannot inspect matching process working directory".into()),
            }
        } else {
            PathBuf::new()
        };
        if matches_filters(cap, &argv, &env, &cwd) {
            count += 1;
        }
    }
    Ok(count)
}

/// The host's quota facts, read by the native parsers `eon_codex` and `eon_claude`.
///
/// The reader is called with a deadline and returns one JSON object as text, or an error
/// message (which makes the reading fail). A host without facts returns an error; backends that
/// use a native parser then fail their reading. Only these keys are read; others are ignored:
///
/// ```text
/// {
///   "codex_quota":  SNAPSHOT,   // read by parser "eon_codex"
///   "claude_quota": SNAPSHOT,   // read by parser "eon_claude"
///   "codex_quota_failure":  FAILURE,   // optional; present while the last poll failed
///   "claude_quota_failure": FAILURE,
///   "quota_source": {"codex": "store", "claude": "window ID"},  // optional; where each
///                               // provider's facts came from, reported as Source::via
///   "quota_unreadable": {"codex": "why"}   // optional; a provider no source has: its
///                               // reading fails with this reason
/// }
/// FAILURE = { "at": 123, "reason": "deadline" }   // only `reason` is read
/// SNAPSHOT = {
///   "state": "fresh",           // any other value ("stale", ...) fails the reading
///   "observed_at": 1720000001000,  // Unix milliseconds when the provider data was fetched;
///                               // reported as Source::observed_at_ms; an unsigned
///                               // integer; required by backends that set limits
///   "windows": [                // nonempty; in the provider's order
///     { "duration_minutes": 300,      // unsigned integer
///       "remaining_percent": 25 }     // number, 0 to 100
///   ]
/// }
/// ```
///
/// A missing snapshot, a missing or empty `windows`, or a window without both fields fails the
/// reading. Window `i` is named `DURATIONm/i` (for example `300m/0`) in limits and output, so
/// windows sharing a duration stay distinct; a limit key `DURATIONm` covers all of them. A
/// present FAILURE holds a backend that sets limits, whatever its snapshot says, since the facts
/// beside it are retained from an earlier poll. Eon supplies this object in the projection of
/// `eon session --json`, from the caller's window or, for a provider that window does not
/// supply, from the quota store Eon's windows share (EON-C40).
pub type SessionReader<'a> = dyn Fn(Instant) -> Result<String, String> + 'a;

/// How much of a backend's quota a check reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaMode {
    /// Lead launches: the quota reader never runs.
    Skip,
    /// Status and dry runs: read, never refresh.
    Read,
    /// Real admissions: read, and run a configured refresh (below the cap) after a failed read
    /// or when the observation was already spent or is too old.
    Admit,
}

/// Why a reading cannot admit: an access problem (which also holds leads) or a quota problem.
enum Hold {
    Access(String),
    Quota(String),
}

/// What freshness says about a limited backend's reading.
enum Stale {
    /// Spent on its account, or older than its bound: a refresh may bring a later observation.
    Retry(String),
    Hold(String),
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Whether a limited backend's reading may admit: it must carry a time that is not in the
/// future, no older than the backend's `max_age_seconds` and not yet spent on its account, with
/// no provider failure beside it. Each hold names the account.
fn freshness(quota: &quota::Quota, backend: &Backend, ledger: &Ledger) -> Result<(), Stale> {
    let account = backend.account();
    if let Some(reason) = &quota.failure {
        return Err(Stale::Hold(format!(
            "quota unreadable on account {account}: provider failure ({reason})"
        )));
    }
    let at = quota.observed_at_ms.ok_or_else(|| {
        Stale::Hold(format!(
            "quota reading on account {account} has no observed_at"
        ))
    })?;
    let now = now_ms();
    if at > now {
        return Err(Stale::Hold(format!(
            "quota reading on account {account} observed in the future (observed_at {at})"
        )));
    }
    let age = now.saturating_sub(at);
    if let Some(max) = backend.max_age_seconds()
        && age > max.saturating_mul(1000)
    {
        return Err(Stale::Retry(format!(
            "quota reading on account {account} is {} s old (max_age_seconds {max})",
            age / 1000
        )));
    }
    let spent = ledger
        .spent(account)
        .map_err(|e| Stale::Hold(format!("quota hold on account {account}: {e}")))?;
    match spent {
        Some(spent) if spent >= at => Err(Stale::Retry(format!(
            "reading observed at {at} already admitted a launch on account {account}; \
             hold for a fresh reading"
        ))),
        _ => Ok(()),
    }
}

/// Reads one backend: access checks, then its quota as `mode` allows. A backend that sets limits
/// also needs a fresh observation (see [`freshness`]); `ledger` says which ones are spent.
pub fn read(
    backend: &Backend,
    cwd: &Path,
    mode: QuotaMode,
    ledger: &Ledger,
    session: &SessionReader<'_>,
) -> Reading {
    let mut reading = Reading::default();
    let access = (|| -> Result<(), String> {
        if !backend.enabled {
            return Err("disabled".into());
        }
        if backend
            .excluded_roots
            .iter()
            .any(|root| cwd.starts_with(fs::canonicalize(root).unwrap_or_else(|_| root.clone())))
        {
            return Err("folder belongs to an excluded account".into());
        }
        for path in &backend.required_files {
            if !path.is_file() {
                return Err("required credentials/file missing".into());
            }
        }
        probe(backend, &backend.guard, 15, cwd).map_err(|_| "command guard missing or failed")?;
        if let Some(check) = &backend.account_check {
            probe(backend, check, 20, cwd).map_err(|_| "account check failed")?;
        }
        Ok(())
    })();
    if let Err(problem) = access {
        reading.problem = Some(problem);
        return reading;
    }
    match (&backend.quota, mode) {
        (_, QuotaMode::Skip) => reading.detail = "quota not read for a lead launch".into(),
        (None, _) => reading.detail = "quota unknown".into(),
        (Some(reader), _) => {
            match read_quota(backend, reader, cwd, mode, ledger, session, &mut reading) {
                Ok(()) => (),
                Err(Hold::Access(problem)) => reading.problem = Some(problem),
                Err(Hold::Quota(problem)) => reading.quota_problem = Some(problem),
            }
        }
    }
    reading
}

fn read_quota(
    backend: &Backend,
    reader: &Reader,
    cwd: &Path,
    mode: QuotaMode,
    ledger: &Ledger,
    session: &SessionReader<'_>,
    reading: &mut Reading,
) -> Result<(), Hold> {
    // A failed read keeps the source a native host named, so status and launch lines say where
    // the unusable facts came from.
    let get = || -> Result<quota::Quota, (String, Option<quota::Origin>)> {
        let text = if reader.parser.native() {
            Instant::now()
                .checked_add(Duration::from_secs(reader.timeout_seconds()))
                .ok_or_else(|| "check timeout too large".to_owned())
                .and_then(|deadline| {
                    let result = session(deadline);
                    if Instant::now() >= deadline {
                        Err("check timed out".into())
                    } else {
                        result
                    }
                })
        } else {
            reader
                .command
                .as_ref()
                .ok_or_else(|| "reader command missing".into())
                .and_then(|command| probe(backend, command, reader.timeout_seconds(), cwd))
        };
        let text = text.map_err(|e| (e, None))?;
        quota::parse(reader.parser, reader.agy_group.as_deref(), &text)
            .map_err(|e| (e, quota::origin(reader.parser, &text)))
    };
    let account = backend.account();
    let unreadable = |reading: &mut Reading, (e, origin): (String, Option<quota::Origin>)| {
        reading.source = origin.map(|origin| crate::decision::Source {
            account: account.into(),
            observed_at_ms: origin.observed_at_ms,
            via: origin.via,
        });
        Hold::Quota(format!("quota unreadable on account {account}: {e}"))
    };
    let refresh = reader.refresh.as_ref().filter(|_| mode == QuotaMode::Admit);
    let mut refreshed = false;
    let mut run_refresh = |reading: &mut Reading| -> Result<(), Hold> {
        if let Some(refresh) = refresh
            && !refreshed
        {
            refreshed = true;
            recheck_cap(backend, reading).map_err(Hold::Access)?;
            let _ = probe(backend, refresh, reader.refresh_timeout_seconds, cwd);
        }
        Ok(())
    };
    let mut quota = get();
    if quota.is_err() && refresh.is_some() {
        run_refresh(reading)?;
        quota = get();
    }
    let mut quota = quota.map_err(|e| unreadable(reading, e))?;
    let mut fresh = backend
        .limited()
        .then(|| freshness(&quota, backend, ledger));
    // A spent or too old observation: a real admission reads once more, after a refresh where
    // configured.
    if mode == QuotaMode::Admit && matches!(fresh, Some(Err(Stale::Retry(_)))) {
        run_refresh(reading)?;
        quota = get().map_err(|e| unreadable(reading, e))?;
        fresh = Some(freshness(&quota, backend, ledger));
    }
    reading.source = Some(crate::decision::Source {
        account: account.into(),
        observed_at_ms: quota.observed_at_ms,
        via: quota.via,
    });
    reading.detail = quota
        .remaining
        .iter()
        .map(|(k, v)| format!("{k}_left={v}%"))
        .collect::<Vec<_>>()
        .join(" ");
    reading.resets = quota.resets;
    reading.remaining = Some(quota.remaining);
    match fresh {
        Some(Err(Stale::Retry(problem) | Stale::Hold(problem))) => Err(Hold::Quota(problem)),
        _ => Ok(()),
    }
}

pub fn recheck_cap(backend: &Backend, reading: &mut Reading) -> Result<(), String> {
    if let Some(cap) = &backend.cap {
        reading.running = running(cap)?;
        if reading.running >= cap.max {
            return Err(format!("busy ({} running)", reading.running));
        }
    }
    Ok(())
}

/// [`read`], then the backend's cap, as an admission sees it.
pub fn check(
    backend: &Backend,
    cwd: &Path,
    mode: QuotaMode,
    ledger: &Ledger,
    session: &SessionReader<'_>,
) -> Reading {
    let mut reading = read(backend, cwd, mode, ledger, session);
    if backend.enabled
        && let Err(problem) = recheck_cap(backend, &mut reading)
        && reading.problem.is_none()
    {
        reading.problem = Some(problem);
    }
    reading
}

/// A reading for a backend that was not read.
pub fn not_checked() -> Reading {
    Reading {
        problem: Some("not checked".into()),
        ..Reading::default()
    }
}

/// Checks every backend in `indices`; the others are not checked.
pub fn collect(
    config: &Config,
    indices: &[usize],
    cwd: &Path,
    mode: QuotaMode,
    ledger: &Ledger,
    session: &SessionReader<'_>,
) -> Vec<Reading> {
    config
        .backends
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if indices.contains(&i) {
                check(b, cwd, mode, ledger, session)
            } else {
                not_checked()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(extra: &str) -> Backend {
        toml::from_str(&format!(
            "name = 'b'\nroles = ['implement']\nguard = ['true']\ncommands = {{}}\n{extra}"
        ))
        .unwrap()
    }

    /// An empty ledger: no observation is spent.
    fn empty() -> Ledger {
        Ledger::load(Path::new("/nonexistent/rota-test/anchor"))
    }

    #[test]
    fn readings_report_the_configured_account_and_the_reader_reported_time() {
        let no_host = |_: Instant| Err("no host".to_string());
        let command = "[quota]\ncommand = ['printf', '{\"week\":45}']\nparser = 'remaining_json'\n";
        // Without an `account` key, a backend is its own account; command readers report no time.
        let ledger = empty();
        let reading = read(
            &backend(command),
            Path::new("."),
            QuotaMode::Read,
            &ledger,
            &no_host,
        );
        let source = reading.source.expect("a successful reading has a source");
        assert_eq!(source.account, "b");
        assert_eq!(source.observed_at_ms, None);
        // Backends that declare one account share it, whatever their reader.
        let shared = format!("account = 'acct'\n{command}");
        assert_eq!(
            read(
                &backend(&shared),
                Path::new("."),
                QuotaMode::Read,
                &ledger,
                &no_host
            )
            .source
            .unwrap()
            .account,
            "acct"
        );
        // A failed reading has no source.
        let failed = "[quota]\ncommand = ['printf', 'not-json']\nparser = 'remaining_json'\n";
        assert_eq!(
            read(
                &backend(failed),
                Path::new("."),
                QuotaMode::Read,
                &ledger,
                &no_host
            )
            .source,
            None
        );
        // Host facts report their snapshot's own `observed_at`, in milliseconds, as Eon
        // Runtime writes it (`workspace.rs` session JSON test: 1720000001000).
        let native = "account = 'claude-main'\n[quota]\nparser = 'eon_claude'\n";
        for at in [1_720_000_001_000_u64, 1_720_000_002_000] {
            let facts = format!(
                "{{\"claude_quota\":{{\"state\":\"fresh\",\"observed_at\":{at},\
                 \"windows\":[{{\"duration_minutes\":300,\"remaining_percent\":60}}]}}}}"
            );
            let host = |_: Instant| Ok(facts.clone());
            for _ in 0..2 {
                let source = read(
                    &backend(native),
                    Path::new("."),
                    QuotaMode::Read,
                    &ledger,
                    &host,
                )
                .source
                .unwrap();
                assert_eq!(
                    source,
                    crate::decision::Source {
                        account: "claude-main".into(),
                        observed_at_ms: Some(at),
                        via: None,
                    }
                );
            }
        }
    }

    #[test]
    fn native_readings_carry_the_hosts_source_and_reason_per_provider() {
        let at = now_ms() - 60_000;
        let snapshot = format!(
            r#"{{"state":"fresh","observed_at":{at},"windows":[{{"duration_minutes":300,"remaining_percent":60}}]}}"#
        );
        let facts = format!(
            r#"{{"codex_quota":{snapshot},"claude_quota":null,
            "quota_source":{{"codex":"store"}},
            "quota_unreadable":{{"claude":"window: gone; store: no stored reading"}}}}"#
        );
        let host = |_: Instant| Ok(facts.clone());
        let read_with = |parser: &str| {
            read(
                &backend(&format!(
                    "account = 'acct'\n[quota]\nparser = '{parser}'\n\
                     [limits.implement.min_remaining]\n'*' = 20\n"
                )),
                Path::new("."),
                QuotaMode::Read,
                &empty(),
                &host,
            )
        };
        let codex = read_with("eon_codex");
        assert_eq!((codex.problem, codex.quota_problem), (None, None));
        assert_eq!(codex.source.unwrap().via.as_deref(), Some("store"));
        // A provider no source has fails with the host's reason, not a missing snapshot.
        let claude = read_with("eon_claude");
        assert_eq!(
            claude.quota_problem.as_deref(),
            Some("quota unreadable on account acct: window: gone; store: no stored reading")
        );
        assert_eq!(claude.source, None);
        // Facts without the keys read as before, and a reading names no source.
        let plain = format!(r#"{{"codex_quota":{snapshot}}}"#);
        let reading = read(
            &backend("[quota]\nparser = 'eon_codex'\n"),
            Path::new("."),
            QuotaMode::Read,
            &empty(),
            &|_| Ok(plain.clone()),
        );
        let source = reading.source.unwrap();
        assert_eq!(source.via, None);
        assert!(serde_json::to_value(&source).unwrap().get("via").is_none());
    }

    #[test]
    fn native_quota_matches_session_reader_admission_table() {
        use crate::{config::Role, decision};
        use serde_json::json;
        let at = now_ms() - 60_000;

        for (parser, key) in [("eon_codex", "codex_quota"), ("eon_claude", "claude_quota")] {
            let backend: Backend = toml::from_str(&format!(
                "name = 'native'\naccount = 'native'\nroles = ['implement', 'review']\n\
                 guard = ['true']\ncommands = {{}}\n[quota]\nparser = '{parser}'\n\
                 [limits.implement]\nstrictly_above_min = true\n\
                 [limits.implement.min_remaining]\n'*' = 20\n"
            ))
            .unwrap();
            let reader = backend.quota.as_ref().unwrap();
            for (name, snapshot, failure, implement, review) in [
                (
                    "fresh",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":300,"remaining_percent":21}]}),
                    false,
                    None,
                    None,
                ),
                (
                    "threshold",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":300,"remaining_percent":20}]}),
                    false,
                    Some("quota limit on account native: 300m/0 left 20% (min_remaining > 20%)"),
                    None,
                ),
                (
                    "empty quota",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":300,"remaining_percent":0}]}),
                    false,
                    Some("quota limit on account native: 300m/0 left 0% (min_remaining > 20%)"),
                    None,
                ),
                (
                    // Facts retained from an earlier poll never admit while the last one failed.
                    "retained after failure",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":300,"remaining_percent":21}]}),
                    true,
                    Some("quota unreadable on account native: provider failure (provider_error)"),
                    Some("quota unreadable on account native: provider failure (provider_error)"),
                ),
                (
                    "untimed",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota reading on account native has no observed_at"),
                    Some("quota reading on account native has no observed_at"),
                ),
                (
                    "scoped",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":10080,"remaining_percent":80},{"duration_minutes":10080,"remaining_percent":19,"scope":"Fable"}]}),
                    false,
                    Some("quota limit on account native: 10080m/1 left 19% (min_remaining > 20%)"),
                    None,
                ),
                (
                    "old",
                    json!({"state":"stale","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                ),
                (
                    "missing",
                    json!(null),
                    false,
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                ),
                (
                    // The failure's reason is kept when there are no facts to hold.
                    "failed without retained facts",
                    json!(null),
                    true,
                    Some(
                        "quota unreadable on account native: provider failure (provider_error); quota snapshot is not fresh",
                    ),
                    Some(
                        "quota unreadable on account native: provider failure (provider_error); quota snapshot is not fresh",
                    ),
                ),
                (
                    "blocked",
                    json!({"state":"blocked","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                ),
                (
                    "unknown",
                    json!({"state":"unknown","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                    Some("quota unreadable on account native: quota snapshot is not fresh"),
                ),
                (
                    "empty windows",
                    json!({"state":"fresh","observed_at":at,"windows":[]}),
                    false,
                    Some("quota unreadable on account native: quota windows unavailable"),
                    Some("quota unreadable on account native: quota windows unavailable"),
                ),
                (
                    "invalid percent",
                    json!({"state":"fresh","observed_at":at,"windows":[{"duration_minutes":300,"remaining_percent":101}]}),
                    false,
                    Some("quota unreadable on account native: invalid percentage"),
                    Some("quota unreadable on account native: invalid percentage"),
                ),
            ] {
                let mut facts = json!({key: snapshot.clone(), "quota_source": {key.trim_end_matches("_quota"): "window w"}});
                if failure {
                    facts[format!("{key}_failure")] =
                        json!({"at":123000,"reason":"provider_error"});
                }
                let text = facts.to_string();
                // The old reader ran a command and parsed its stdout.
                let old = probe(
                    &backend,
                    &["printf".into(), "%s".into(), text.clone()],
                    5,
                    Path::new("."),
                )
                .and_then(|text| quota::parse(reader.parser, None, &text));
                let old = match old {
                    Ok(quota) => Reading {
                        remaining: Some(quota.remaining),
                        resets: quota.resets,
                        ..Reading::default()
                    },
                    Err(error) => Reading {
                        problem: Some(format!("quota unreadable on account native: {error}")),
                        ..Reading::default()
                    },
                };
                let new = read(&backend, Path::new("."), QuotaMode::Read, &empty(), &|_| {
                    Ok(text.clone())
                });
                assert_eq!(new.remaining, old.remaining, "{parser}/{name}");
                assert_eq!(new.resets, old.resets, "{parser}/{name}");
                // Usable or not, the reading names its source, with the snapshot's time if any.
                assert_eq!(
                    new.source,
                    Some(crate::decision::Source {
                        account: "native".into(),
                        observed_at_ms: snapshot.get("observed_at").and_then(|at| at.as_u64()),
                        via: Some("window w".into()),
                    }),
                    "{parser}/{name}"
                );
                for (role, expected) in [(Role::Implement, implement), (Role::Review, review)] {
                    let result = decision::availability(&backend, &new, role);
                    // The old reader ignored provider failures and observation times.
                    if !failure && name != "untimed" {
                        assert_eq!(
                            result,
                            decision::availability(&backend, &old, role),
                            "{parser}/{name}/{role:?}"
                        );
                    }
                    assert_eq!(
                        result.err().as_deref(),
                        expected,
                        "{parser}/{name}/{role:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_retry_reads_facts_again_and_status_never_refreshes() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("refresh");
        let backend: Backend = toml::from_str(&format!(
            "name = 'native'\nroles = []\nguard = ['true']\ncommands = {{}}\n\
             [quota]\nparser = 'eon_codex'\nrefresh = ['touch', '{}']\n",
            marker.display()
        ))
        .unwrap();
        let calls = std::cell::Cell::new(0);
        let source = |_| {
            calls.set(calls.get() + 1);
            if marker.exists() {
                Ok(r#"{"codex_quota":{"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":80}]}}"#.into())
            } else {
                Err("supervisor unavailable".into())
            }
        };
        let status = read(&backend, root.path(), QuotaMode::Read, &empty(), &source);
        assert_eq!(
            status.quota_problem.as_deref(),
            Some("quota unreadable on account native: supervisor unavailable")
        );
        assert_eq!(calls.get(), 1);
        assert!(!marker.exists());
        let launch = read(&backend, root.path(), QuotaMode::Admit, &empty(), &source);
        assert_eq!((launch.problem, launch.quota_problem), (None, None));
        assert_eq!(launch.remaining.unwrap()["300m/0"], 80.0);
        assert_eq!(calls.get(), 3);
    }
    #[test]
    fn a_provider_failure_beside_fresh_unspent_facts_holds_only_a_limited_backend() {
        let at = now_ms() - 60_000;
        let facts = format!(
            r#"{{"claude_quota":{{"state":"fresh","observed_at":{at},
            "windows":[{{"duration_minutes":10080,"remaining_percent":80}}]}},
            "claude_quota_failure":{{"at":3,"reason":"deadline"}}}}"#
        );
        let host = |_: Instant| Ok(facts.clone());
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("anchor");
        let mut ledger = Ledger::load(&state);
        ledger.record("acct", at - 1).unwrap();
        let limited = backend(
            "account = 'acct'\n[quota]\nparser = 'eon_claude'\n\
             [limits.implement.min_remaining]\n'10080m' = 3\n",
        );
        let reading = read(&limited, Path::new("."), QuotaMode::Admit, &ledger, &host);
        assert_eq!(
            reading.quota_problem.as_deref(),
            Some("quota unreadable on account acct: provider failure (deadline)")
        );
        assert_eq!(reading.problem, None);
        // Backends without limits keep today's behaviour: the retained facts admit.
        let unlimited = backend("[quota]\nparser = 'eon_claude'\n");
        let reading = read(&unlimited, Path::new("."), QuotaMode::Admit, &ledger, &host);
        assert_eq!((reading.problem, reading.quota_problem), (None, None));
        assert_eq!(reading.remaining.unwrap()["10080m/0"], 80.0);
    }

    #[test]
    fn a_spent_observation_reads_again_after_refresh_only_when_admitting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let reading = root.join("reading");
        let refreshed = root.join("refreshed");
        let (first, later) = (now_ms() - 60_000, now_ms() - 30_000);
        fs::write(&reading, format!(r#"{{"week":80,"observed_at":{first}}}"#)).unwrap();
        let limited = backend(&format!(
            "account = 'acct'\n[quota]\ncommand = ['cat', '{reading}']\nparser = 'remaining_json'\n\
             refresh = ['sh', '-c', 'touch \"$0\"; printf \"{{\\\"week\\\":70,\\\"observed_at\\\":{later}}}\" > \"$1\"', '{refreshed}', '{reading}']\n\
             [limits.implement.min_remaining]\nweek = 10\n",
            reading = reading.display(),
            refreshed = refreshed.display(),
        ));
        let mut ledger = Ledger::load(&root.join("state/anchor"));
        ledger.record("acct", first).unwrap();
        let repeat = |at: u64| {
            format!(
                "reading observed at {at} already admitted a launch on account acct; \
                 hold for a fresh reading"
            )
        };
        // Status and dry runs show the hold, with the reading, and never refresh.
        let shown = read(
            &limited,
            root,
            QuotaMode::Read,
            &ledger,
            &|_| unreachable!(),
        );
        assert_eq!(shown.quota_problem, Some(repeat(first)));
        assert_eq!(shown.remaining.unwrap()["week"], 80.0);
        assert_eq!(
            shown.source,
            Some(crate::decision::Source {
                account: "acct".into(),
                observed_at_ms: Some(first),
                via: None,
            })
        );
        assert!(!refreshed.exists());
        // A real admission refreshes once, reads again, and admits the later observation.
        let admitted = read(
            &limited,
            root,
            QuotaMode::Admit,
            &ledger,
            &|_| unreachable!(),
        );
        assert!(refreshed.exists());
        assert_eq!(admitted.quota_problem, None);
        assert_eq!(admitted.source.unwrap().observed_at_ms, Some(later));
        assert_eq!(admitted.remaining.unwrap()["week"], 70.0);
        // A refresh that brings nothing later still holds, naming the account.
        ledger.record("acct", later).unwrap();
        let held = read(
            &limited,
            root,
            QuotaMode::Admit,
            &ledger,
            &|_| unreachable!(),
        );
        assert_eq!(held.quota_problem, Some(repeat(later)));
    }

    #[test]
    fn a_reading_older_than_its_bound_holds_until_a_refresh_brings_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The reader stamps its time when it runs, 1805 s back, so the age it reports is exact.
        let old = "['sh', '-c', 'printf \"{\\\"week\\\":80,\\\"observed_at\\\":%s}\" \
                   $(( $(date +%s%3N) - 1805000 ))']";
        let limited = |extra: &str| {
            backend(&format!(
                "account = 'acct'\n[quota]\ncommand = {old}\nparser = 'remaining_json'\n{extra}\n\
                 [limits.implement.min_remaining]\nweek = 10\n"
            ))
        };
        let held = read(
            &limited(""),
            root,
            QuotaMode::Admit,
            &empty(),
            &|_| unreachable!(),
        );
        assert_eq!(
            held.quota_problem.as_deref(),
            Some("quota reading on account acct is 1805 s old (max_age_seconds 600)")
        );
        // A wider bound admits it, and a backend without limits has no bound.
        let wide = read(
            &limited("max_age_seconds = 2000"),
            root,
            QuotaMode::Admit,
            &empty(),
            &|_| unreachable!(),
        );
        assert_eq!((wide.problem, wide.quota_problem), (None, None));
        let unlimited = backend(&format!(
            "[quota]\ncommand = {old}\nparser = 'remaining_json'\n"
        ));
        let reading = read(
            &unlimited,
            root,
            QuotaMode::Admit,
            &empty(),
            &|_| unreachable!(),
        );
        assert_eq!((reading.problem, reading.quota_problem), (None, None));
        // A real admission refreshes an old reading once and admits the newer one; status never
        // refreshes.
        let reading = root.join("reading");
        fs::write(&reading, "{\"week\":80,\"observed_at\":1}").unwrap();
        let refreshing = backend(&format!(
            "account = 'acct'\n[quota]\ncommand = ['cat', '{reading}']\nparser = 'remaining_json'\n\
             refresh = ['sh', '-c', 'printf \"{{\\\"week\\\":80,\\\"observed_at\\\":%s}}\" \
             $(date +%s%3N) > \"$0\"', '{reading}']\n\
             [limits.implement.min_remaining]\nweek = 10\n",
            reading = reading.display(),
        ));
        let shown = read(
            &refreshing,
            root,
            QuotaMode::Read,
            &empty(),
            &|_| unreachable!(),
        );
        assert!(
            shown
                .quota_problem
                .as_deref()
                .is_some_and(|p| p.ends_with(" s old (max_age_seconds 600)")),
            "{shown:?}"
        );
        let admitted = read(
            &refreshing,
            root,
            QuotaMode::Admit,
            &empty(),
            &|_| unreachable!(),
        );
        assert_eq!((admitted.problem, admitted.quota_problem), (None, None));
    }

    #[test]
    fn a_lead_check_never_runs_the_reader() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("reader-ran");
        let limited = backend(&format!(
            "account = 'acct'\n[quota]\ncommand = ['touch', '{}']\nparser = 'remaining_json'\n\
             refresh = ['touch', '{}']\n",
            marker.display(),
            marker.display(),
        ));
        let native = backend("account = 'acct'\n[quota]\nparser = 'eon_claude'\n");
        for backend in [&limited, &native] {
            let reading = read(backend, dir.path(), QuotaMode::Skip, &empty(), &|_| {
                panic!("a lead launch read quota")
            });
            assert_eq!((reading.problem, reading.quota_problem), (None, None));
            assert_eq!(reading.remaining, None);
            assert_eq!(reading.source, None);
        }
        assert!(!marker.exists());
    }

    #[test]
    fn a_check_that_outlives_sigkill_is_given_up_on() {
        // A kill that does nothing stands in for a check stuck in uninterruptible I/O.
        let text = "name = 'b'\nroles = []\nguard = ['true']\ncommands = {}";
        let backend: Backend = toml::from_str(text).unwrap();
        let argv = ["sleep".to_string(), "5".to_string()];
        let start = Instant::now();
        let result = probe_killing(command(&backend, &argv), 0, Path::new("."), |_| {});
        let elapsed = start.elapsed();
        assert_eq!(
            result.unwrap_err(),
            "check timed out; still running after SIGKILL"
        );
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        let result = probe(&backend, &argv, 0, Path::new("."));
        assert_eq!(result.unwrap_err(), "check timed out");
    }
    #[test]
    fn process_caps_include_all_roles_and_exclude_usage() {
        let mut cap = Cap {
            max: 1,
            executable: "fake-agent".into(),
            runtime_executables: vec![],
            script: None,
            env: BTreeMap::new(),
            exclude_args: vec!["/usage".into()],
        };
        let args = vec!["/bin/fake-agent".into(), "review anything".into()];
        assert!(matches_filters(
            &cap,
            &args,
            &BTreeMap::new(),
            Path::new(".")
        ));
        assert!(!matches_filters(
            &cap,
            &["fake-agent".into(), "/usage".into()],
            &BTreeMap::new(),
            Path::new(".")
        ));
        cap.env
            .insert("AGENT_HOME".into(), "/fictional/account".into());
        assert!(!matches_filters(
            &cap,
            &args,
            &BTreeMap::new(),
            Path::new(".")
        ));
        assert!(matches_filters(&cap, &args, &cap.env, Path::new(".")));
    }
    #[test]
    fn canonical_account_directory_aliases_match() {
        assert!(canonical_equal("src/../src", "src"));
        assert!(!canonical_equal("absent-one", "absent-two"));
    }
    #[test]
    fn disabled_backend_never_runs_checks() {
        let config = Config::parse(include_str!("../config.example.toml"), "rota").unwrap();
        assert_eq!(
            read(
                &config.backends[3],
                Path::new("."),
                QuotaMode::Read,
                &empty(),
                &|_| panic!("disabled backend read quota")
            )
            .problem
            .as_deref(),
            Some("disabled")
        );
    }
    #[test]
    fn timeout_includes_descendant_output_drain() {
        let config = Config::parse(include_str!("../config.example.toml"), "rota").unwrap();
        let start = Instant::now();
        let result = probe(
            &config.backends[0],
            &["sh".into(), "-c".into(), "sleep 8 & printf '25 25'".into()],
            1,
            Path::new("."),
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    fn scan_cap() -> Cap {
        Cap {
            max: 1,
            executable: "fake-agent".into(),
            runtime_executables: vec![],
            script: None,
            env: BTreeMap::from([("AGENT_HOME".into(), "/fictional/account".into())]),
            exclude_args: vec![],
        }
    }
    fn fake_process(proc: &Path, pid: &str, argv0: &str) -> PathBuf {
        let dir = proc.join(pid);
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("cmdline"), format!("{argv0}\0work\0")).unwrap();
        fs::write(dir.join("environ"), "AGENT_HOME=/fictional/account\0").unwrap();
        std::os::unix::fs::symlink(proc, dir.join("cwd")).unwrap();
        dir
    }
    #[test]
    fn cap_scan_counts_current_users_matching_processes() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path();
        let uid = fs::metadata(proc).unwrap().uid();
        fake_process(proc, "100", "/bin/fake-agent");
        fake_process(proc, "101", "fake-agent");
        fake_process(proc, "102", "/bin/other-agent");
        fs::create_dir(proc.join("self")).unwrap();
        assert_eq!(running_in(proc, uid, &scan_cap()), Ok(2));
    }
    #[test]
    fn cap_scan_skips_other_users_processes() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path();
        let uid = fs::metadata(proc).unwrap().uid();
        fake_process(proc, "100", "/bin/fake-agent");
        assert_eq!(running_in(proc, uid.wrapping_add(1), &scan_cap()), Ok(0));
    }
    #[test]
    fn cap_scan_skips_vanished_processes() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path();
        let uid = fs::metadata(proc).unwrap().uid();
        fake_process(proc, "100", "/bin/fake-agent");
        std::os::unix::fs::symlink(proc.join("exited"), proc.join("200")).unwrap();
        let no_cwd = fake_process(proc, "201", "/bin/fake-agent");
        fs::remove_file(no_cwd.join("cwd")).unwrap();
        let no_environ = fake_process(proc, "202", "/bin/fake-agent");
        fs::remove_file(no_environ.join("environ")).unwrap();
        let unrelated = fake_process(proc, "203", "/bin/other-agent");
        fs::remove_file(unrelated.join("environ")).unwrap();
        fs::create_dir(unrelated.join("environ")).unwrap();
        assert_eq!(running_in(proc, uid, &scan_cap()), Ok(1));
    }
    #[test]
    fn cap_scan_fails_on_a_live_unreadable_matching_process() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path();
        let uid = fs::metadata(proc).unwrap().uid();
        let agent = fake_process(proc, "100", "/bin/fake-agent");
        fs::remove_file(agent.join("environ")).unwrap();
        fs::create_dir(agent.join("environ")).unwrap();
        assert_eq!(
            running_in(proc, uid, &scan_cap()),
            Err("cannot inspect matching process environment".into())
        );
        fs::remove_dir(agent.join("environ")).unwrap();
        fs::write(agent.join("environ"), "AGENT_HOME=/fictional/account\0").unwrap();
        fs::remove_file(agent.join("cwd")).unwrap();
        fs::write(agent.join("cwd"), "").unwrap();
        assert_eq!(
            running_in(proc, uid, &scan_cap()),
            Err("cannot inspect matching process working directory".into())
        );
    }
}
