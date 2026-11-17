use crate::config::{Backend, Cap, Config};
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
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
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
    running_in(Path::new("/proc"), unsafe { libc::geteuid() }, cap)
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
///   "claude_quota": SNAPSHOT    // read by parser "eon_claude"
/// }
/// SNAPSHOT = {
///   "state": "fresh",           // any other value ("stale", ...) fails the reading
///   "observed_at": 1720000001000,  // Unix milliseconds when the provider data was fetched;
///                               // optional; reported as Source::observed_at_ms
///   "windows": [                // nonempty; in the provider's order
///     { "duration_minutes": 300,      // unsigned integer
///       "remaining_percent": 25 }     // number, 0 to 100
///   ]
/// }
/// ```
///
/// A missing snapshot, a missing or empty `windows`, or a window without both fields fails the
/// reading. Window `i` is named `DURATIONm/i` (for example `300m/0`) in limits and output, so
/// windows sharing a duration stay distinct. Eon supplies this object from `eon session --json`.
pub type SessionReader<'a> = dyn Fn(Instant) -> Result<String, String> + 'a;

pub fn read(
    backend: &Backend,
    cwd: &Path,
    allow_refresh: bool,
    session: &SessionReader<'_>,
) -> Reading {
    let mut reading = Reading::default();
    let result = (|| -> Result<(), String> {
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
        if let Some(reader) = &backend.quota {
            let get = || -> Result<(crate::quota::Quota, Option<u64>), String> {
                let text = if reader.parser.native() {
                    let deadline = Instant::now()
                        .checked_add(Duration::from_secs(reader.timeout_seconds()))
                        .ok_or("check timeout too large")?;
                    let result = session(deadline);
                    if Instant::now() >= deadline {
                        Err("check timed out".into())
                    } else {
                        result
                    }
                } else {
                    reader
                        .command
                        .as_ref()
                        .ok_or_else(|| "reader command missing".into())
                        .and_then(|command| probe(backend, command, reader.timeout_seconds(), cwd))
                };
                let text = text?;
                let quota = quota::parse(reader.parser, reader.agy_group.as_deref(), &text)?;
                Ok((quota, quota::observed_at_ms(reader.parser, &text)))
            };
            let mut quota = get();
            if quota.is_err() && allow_refresh {
                if let Some(refresh) = &reader.refresh {
                    recheck_cap(backend, &mut reading)?;
                    let _ = probe(backend, refresh, reader.refresh_timeout_seconds, cwd);
                    quota = get();
                }
            }
            let (quota, observed_at_ms) = quota.map_err(|e| format!("quota unreadable: {e}"))?;
            reading.source = Some(crate::decision::Source {
                account: backend.account().into(),
                observed_at_ms,
            });
            reading.detail = quota
                .remaining
                .iter()
                .map(|(k, v)| format!("{k}_left={v}%"))
                .collect::<Vec<_>>()
                .join(" ");
            reading.resets = quota.resets;
            reading.remaining = Some(quota.remaining);
        } else {
            reading.detail = "quota unknown".into();
        }
        Ok(())
    })();
    if let Err(problem) = result {
        reading.problem = Some(problem);
    }
    reading
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

pub fn collect(
    config: &Config,
    indices: &[usize],
    cwd: &Path,
    allow_refresh: bool,
    session: &SessionReader<'_>,
) -> Vec<Reading> {
    let mut readings: Vec<_> = config
        .backends
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if indices.contains(&i) {
                read(b, cwd, allow_refresh, session)
            } else {
                Reading {
                    problem: Some("not checked".into()),
                    ..Reading::default()
                }
            }
        })
        .collect();
    for index in indices {
        if config.backends[*index].enabled {
            if let Err(problem) = recheck_cap(&config.backends[*index], &mut readings[*index]) {
                if readings[*index].problem.is_none() {
                    readings[*index].problem = Some(problem);
                }
            }
        }
    }
    readings
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

    #[test]
    fn readings_report_the_configured_account_and_the_reader_reported_time() {
        let no_host = |_: Instant| Err("no host".to_string());
        let command = "[quota]\ncommand = ['printf', '{\"week\":45}']\nparser = 'remaining_json'\n";
        // Without an `account` key, a backend is its own account; command readers report no time.
        let reading = read(&backend(command), Path::new("."), false, &no_host);
        let source = reading.source.expect("a successful reading has a source");
        assert_eq!(source.account, "b");
        assert_eq!(source.observed_at_ms, None);
        // Backends that declare one account share it, whatever their reader.
        let shared = format!("account = 'acct'\n{command}");
        assert_eq!(
            read(&backend(&shared), Path::new("."), false, &no_host)
                .source
                .unwrap()
                .account,
            "acct"
        );
        // A failed reading has no source.
        let failed = "[quota]\ncommand = ['printf', 'not-json']\nparser = 'remaining_json'\n";
        assert_eq!(
            read(&backend(failed), Path::new("."), false, &no_host).source,
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
                let source = read(&backend(native), Path::new("."), false, &host)
                    .source
                    .unwrap();
                assert_eq!(
                    source,
                    crate::decision::Source {
                        account: "claude-main".into(),
                        observed_at_ms: Some(at),
                    }
                );
            }
        }
    }

    #[test]
    fn native_quota_matches_session_reader_admission_table() {
        use crate::{config::Role, decision};
        use serde_json::json;

        for (parser, key) in [("eon_codex", "codex_quota"), ("eon_claude", "claude_quota")] {
            let backend: Backend = toml::from_str(&format!(
                "name = 'native'\nroles = ['implement', 'review']\nguard = ['true']\ncommands = {{}}\n\
                 allow_unknown_quota = true\n[quota]\nparser = '{parser}'\n\
                 [limits.implement]\nstrictly_above_min = true\n\
                 [limits.implement.min_remaining]\n'*' = 20\n"
            ))
            .unwrap();
            let reader = backend.quota.as_ref().unwrap();
            for (name, snapshot, failure, implement, review) in [
                (
                    "fresh",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":21}]}),
                    false,
                    None,
                    None,
                ),
                (
                    "threshold",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":20}]}),
                    false,
                    Some("quota limit: 300m/0_left=20%"),
                    None,
                ),
                (
                    "empty quota",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":0}]}),
                    false,
                    Some("quota limit: 300m/0_left=0%"),
                    None,
                ),
                (
                    "retained after failure",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":21}]}),
                    true,
                    None,
                    None,
                ),
                (
                    "scoped",
                    json!({"state":"fresh","windows":[{"duration_minutes":10080,"remaining_percent":80},{"duration_minutes":10080,"remaining_percent":19,"scope":"Fable"}]}),
                    false,
                    Some("quota limit: 10080m/1_left=19%"),
                    None,
                ),
                (
                    "old",
                    json!({"state":"stale","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable: quota snapshot is not fresh"),
                    Some("quota unreadable: quota snapshot is not fresh"),
                ),
                (
                    "missing",
                    json!(null),
                    false,
                    Some("quota unreadable: quota snapshot is not fresh"),
                    Some("quota unreadable: quota snapshot is not fresh"),
                ),
                (
                    "failed without retained facts",
                    json!(null),
                    true,
                    Some("quota unreadable: quota snapshot is not fresh"),
                    Some("quota unreadable: quota snapshot is not fresh"),
                ),
                (
                    "blocked",
                    json!({"state":"blocked","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable: quota snapshot is not fresh"),
                    Some("quota unreadable: quota snapshot is not fresh"),
                ),
                (
                    "unknown",
                    json!({"state":"unknown","windows":[{"duration_minutes":300,"remaining_percent":80}]}),
                    false,
                    Some("quota unreadable: quota snapshot is not fresh"),
                    Some("quota unreadable: quota snapshot is not fresh"),
                ),
                (
                    "empty windows",
                    json!({"state":"fresh","windows":[]}),
                    false,
                    Some("quota unreadable: quota windows unavailable"),
                    Some("quota unreadable: quota windows unavailable"),
                ),
                (
                    "invalid percent",
                    json!({"state":"fresh","windows":[{"duration_minutes":300,"remaining_percent":101}]}),
                    false,
                    Some("quota unreadable: invalid percentage"),
                    Some("quota unreadable: invalid percentage"),
                ),
            ] {
                let mut facts = json!({key: snapshot});
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
                        problem: Some(format!("quota unreadable: {error}")),
                        ..Reading::default()
                    },
                };
                let new = read(&backend, Path::new("."), false, &|_| Ok(text.clone()));
                assert_eq!(new.remaining, old.remaining, "{parser}/{name}");
                assert_eq!(new.resets, old.resets, "{parser}/{name}");
                for (role, expected) in [(Role::Implement, implement), (Role::Review, review)] {
                    let result = decision::availability(&backend, &new, role);
                    assert_eq!(
                        result,
                        decision::availability(&backend, &old, role),
                        "{parser}/{name}/{role:?}"
                    );
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
        let status = read(&backend, root.path(), false, &source);
        assert_eq!(
            status.problem.as_deref(),
            Some("quota unreadable: supervisor unavailable")
        );
        assert_eq!(calls.get(), 1);
        assert!(!marker.exists());
        let launch = read(&backend, root.path(), true, &source);
        assert_eq!(launch.problem, None);
        assert_eq!(launch.remaining.unwrap()["300m/0"], 80.0);
        assert_eq!(calls.get(), 3);
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
            read(&config.backends[3], Path::new("."), false, &|_| panic!(
                "disabled backend read quota"
            ))
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
