use eon_workspace_protocol::v27::{
    ClaudeQuota, ClaudeQuotaState, CodexQuota, CodexQuotaState, MAX_QUOTA_SCOPE_BYTES,
    MAX_SCOPED_QUOTA_WINDOWS, QuotaFailure, QuotaFailureReason as Failure, QuotaWindow, Snapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: usize = 64 * 1024;
/// A reading older than this is rejected when read and shown `old` once retained.
const STALE_AFTER: u64 = 3600;
const EXPIRE_AFTER: u64 = 24 * 3600;
const STORE_VERSION: u32 = 1;
/// CodexBar names Claude's model-scoped weekly windows `claude-weekly-scoped-<model>`.
const CLAUDE_SCOPED: &str = "claude-weekly-scoped-";

fn timestamp(value: &Value) -> Result<u64, ()> {
    let value = value.as_str().ok_or(())?;
    let date = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| ())?;
    u64::try_from(date.unix_timestamp()).map_err(|_| ())
}

/// Whether `windows` is a set Runtime can hold for `provider`: at most two unscoped windows and,
/// for Claude only, four scoped ones, no two of one duration and scope, each of a supported
/// duration (EON-C27).
fn valid_windows(provider: &str, windows: &[QuotaWindow]) -> bool {
    let scoped = windows.iter().filter(|w| w.scope.is_some()).count();
    (1..=2 + MAX_SCOPED_QUOTA_WINDOWS).contains(&windows.len())
        && windows.len() - scoped <= 2
        && scoped
            <= if provider == "claude" {
                MAX_SCOPED_QUOTA_WINDOWS
            } else {
                0
            }
        && windows.iter().enumerate().all(|(index, w)| {
            w.duration_minutes > 0
                && w.remaining_percent <= 100
                && (provider != "claude" || matches!(w.duration_minutes, 300 | 10080))
                && w.scope.as_deref().is_none_or(|scope| {
                    !scope.is_empty()
                        && scope.len() <= MAX_QUOTA_SCOPE_BYTES
                        && !scope.chars().any(char::is_control)
                })
                && !windows[..index]
                    .iter()
                    .any(|o| o.duration_minutes == w.duration_minutes && o.scope == w.scope)
        })
}

/// A supplied window's duration, used percentage and optional reset.
type Supplied = (u32, f64, Option<u64>);

fn supplied(value: &Value, provider: &str) -> Result<Supplied, Failure> {
    let used = value["usedPercent"]
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .ok_or(Failure::InvalidWindow)?;
    let duration_minutes = value["windowMinutes"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or(Failure::InvalidWindow)?;
    if provider == "claude" && !matches!(duration_minutes, 300 | 10080) {
        return Err(Failure::InvalidWindow);
    }
    let resets_at = (!value["resetsAt"].is_null())
        .then(|| timestamp(&value["resetsAt"]))
        .transpose()
        .map_err(|()| Failure::InvalidWindow)?;
    Ok((duration_minutes, used, resets_at))
}

/// CodexBar titles a scoped window `<model> only`; its scope is the model, cut to fit EONW.
fn scope_of(title: &str) -> &str {
    let scope = title.strip_suffix(" only").unwrap_or(title).trim();
    let end = (0..=scope.len().min(MAX_QUOTA_SCOPE_BYTES))
        .rev()
        .find(|end| scope.is_char_boundary(*end))
        .unwrap_or(0);
    scope[..end].trim_end()
}

fn normalize(bytes: &[u8], provider: &str, now: u64) -> Result<Option<CodexQuota>, Failure> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| Failure::Malformed)?;
    let [result] = value.as_array().ok_or(Failure::Malformed)?.as_slice() else {
        return Err(Failure::Malformed);
    };
    let source = if provider == "codex" {
        "codex-cli"
    } else {
        "oauth"
    };
    if result["provider"] != provider || result["source"] != source || !result["error"].is_null() {
        return Err(Failure::ProviderError);
    }
    let usage = &result["usage"];
    if usage.is_null() {
        return Ok(None);
    }
    if !usage["dataConfidence"].is_null() && usage["dataConfidence"] != "authoritative" {
        return Err(Failure::Estimated);
    }
    let observed_at = timestamp(&usage["updatedAt"]).map_err(|()| Failure::ObservationTime)?;
    if observed_at == 0 || observed_at > now || now - observed_at > STALE_AFTER {
        return Err(Failure::ObservationTime);
    }
    // The selected providers have at most two windows; silently dropping another is incorrect.
    if !usage["tertiary"].is_null() {
        return Err(Failure::InvalidWindow);
    }
    let mut unscoped: Vec<Supplied> = Vec::new();
    for value in [&usage["primary"], &usage["secondary"]] {
        if value.is_null() {
            continue;
        }
        let next = supplied(value, provider)?;
        // CodexBar repeats Claude's weekly window as primary when no five-hour window is
        // reported (eon-8jdmw); an exact repeat is one window, a differing one is invalid.
        match unscoped.iter().find(|first| first.0 == next.0) {
            Some(first) if *first == next => {}
            Some(_) => return Err(Failure::InvalidWindow),
            None => unscoped.push(next),
        }
    }
    let current = |(_, _, resets_at): &Supplied| resets_at.is_none_or(|reset| reset > now);
    let window = |scope, (duration_minutes, used, resets_at): Supplied| QuotaWindow {
        duration_minutes,
        remaining_percent: (100.0 - used).floor() as u8,
        resets_at,
        scope,
    };
    let mut windows: Vec<_> = unscoped
        .into_iter()
        .filter(current)
        .map(|supplied| window(None, supplied))
        .collect();
    // Claude's model-scoped weekly windows follow its unscoped ones (eon-wqkjc); other extra
    // windows are not quota Eon shows. A scoped window never fails the reading: one with an
    // unusable scope or value, a scope whose repeats differ, and any past the fourth are left out.
    if provider == "claude"
        && let Some(extras) = usage["extraRateWindows"].as_array()
    {
        let mut scoped: Vec<(&str, Option<Supplied>)> = Vec::new();
        for extra in extras {
            if !extra["id"]
                .as_str()
                .is_some_and(|id| id.starts_with(CLAUDE_SCOPED))
                || extra["usageKnown"] == false
            {
                continue;
            }
            let Some(scope) = extra["title"]
                .as_str()
                .map(scope_of)
                .filter(|scope| !scope.is_empty() && !scope.chars().any(char::is_control))
            else {
                continue;
            };
            let next = supplied(&extra["window"], provider).ok();
            match scoped.iter_mut().find(|(first, _)| *first == scope) {
                Some((_, first)) if *first != next => *first = None,
                Some(_) => {}
                None => scoped.push((scope, next)),
            }
        }
        windows.extend(
            scoped
                .into_iter()
                .filter_map(|(scope, supplied)| Some((scope, supplied?)))
                .filter(|(_, supplied)| current(supplied))
                .take(MAX_SCOPED_QUOTA_WINDOWS)
                .map(|(scope, supplied)| window(Some(scope.to_owned()), supplied)),
        );
    }
    if windows.is_empty() {
        return Ok(None);
    }
    if !valid_windows(provider, &windows) {
        return Err(Failure::InvalidWindow);
    }
    Ok(Some(CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at,
        windows,
    }))
}

/// Staleness depends only on the age of the last good reading (EON-C27).
fn present_at(mut quota: CodexQuota, now: u64) -> Option<CodexQuota> {
    let age = now.checked_sub(quota.observed_at)?;
    if age > EXPIRE_AFTER {
        return None;
    }
    if age > STALE_AFTER {
        quota.state = CodexQuotaState::Stale;
    }
    quota
        .windows
        .retain(|w| w.resets_at.is_none_or(|reset| reset > now));
    (!quota.windows.is_empty()).then_some(quota)
}

/// A provider's last good reading, kept for the supervisors that start after this one
/// (EON-C27).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    observed_at: u64,
    windows: Vec<StoredWindow>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredWindow {
    duration_minutes: u32,
    remaining_percent: u8,
    resets_at: Option<u64>,
    // Omitted when unscoped, so a reading without scoped windows stays readable by older Eons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

/// The stored reading, unless it is unreadable, malformed, of another version or from the future.
fn load(path: &Path, provider: &str, now: u64) -> Option<CodexQuota> {
    let mut bytes = Vec::new();
    // A nonblocking open keeps a FIFO in its place from stalling the worker.
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let stored: Stored = serde_json::from_slice(&bytes).ok()?;
    let windows = stored
        .windows
        .into_iter()
        .map(|w| QuotaWindow {
            duration_minutes: w.duration_minutes,
            remaining_percent: w.remaining_percent,
            resets_at: w.resets_at,
            scope: w.scope,
        })
        .collect::<Vec<_>>();
    (stored.version == STORE_VERSION
        && (1..=now).contains(&stored.observed_at)
        && valid_windows(provider, &windows))
    .then_some(CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at: stored.observed_at,
        windows,
    })
}

/// Replaces the stored reading whole, or removes it after a read that found no quota.
fn save(path: &Path, quota: Option<&CodexQuota>) {
    let Some(quota) = quota else {
        let _ = fs::remove_file(path);
        return;
    };
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let stored = Stored {
        version: STORE_VERSION,
        observed_at: quota.observed_at,
        windows: quota
            .windows
            .iter()
            .map(|w| StoredWindow {
                duration_minutes: w.duration_minutes,
                remaining_percent: w.remaining_percent,
                resets_at: w.resets_at,
                scope: w.scope.clone(),
            })
            .collect(),
    };
    // Each supervisor writes its own file and renames it over the reading, so a reader never
    // sees a partial one.
    let temporary = directory.join(format!(".{}.{}", name.display(), process::id()));
    let _ = fs::remove_file(&temporary);
    let saved = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .and_then(|()| {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
        })
        .and_then(|mut file| file.write_all(&serde_json::to_vec(&stored)?))
        .and_then(|()| fs::rename(&temporary, path));
    if saved.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

fn epoch_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

struct Provider {
    quota: Arc<Mutex<Option<CodexQuota>>>,
    /// The last read's failure, until a read succeeds.
    failure: Arc<Mutex<Option<QuotaFailure>>>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Provider {
    fn start(
        program: PathBuf,
        provider: &'static str,
        interval: Duration,
        timeout: Duration,
        last_read: Arc<Mutex<Option<Instant>>>,
        store: Option<PathBuf>,
    ) -> Self {
        let quota = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&quota);
        let failure = Arc::new(Mutex::new(None));
        let failed = Arc::clone(&failure);
        let (stop, stopped) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(format!("eon-{provider}-quota"))
            .spawn(move || {
                // The stored reading stands in until this worker's own read succeeds.
                if let Some(stored) = store
                    .as_deref()
                    .zip(epoch_seconds())
                    .and_then(|(path, now)| load(path, provider, now))
                    && let Ok(mut current) = shared.lock()
                {
                    *current = Some(stored);
                }
                // Reads are spaced from the end of the last one, also across a restarted worker.
                let since = last_read.lock().ok().and_then(|last| *last);
                if let Some(wait) = since.and_then(|since| interval.checked_sub(since.elapsed()))
                    && !matches!(
                        stopped.recv_timeout(wait),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    )
                {
                    return;
                }
                loop {
                    let fetched = fetch(&program, provider, timeout, &stopped);
                    if let Ok(mut last) = last_read.lock() {
                        *last = Some(Instant::now());
                    }
                    let reading = match fetched {
                        Ok(None) => break,
                        Ok(Some(bytes)) => epoch_seconds()
                            .ok_or(Failure::ObservationTime)
                            .and_then(|now| normalize(&bytes, provider, now)),
                        Err(reason) => Err(reason),
                    };
                    // A failed read keeps the last good reading; its age alone marks it old.
                    // Without a clock to date it, a failure leaves the previous one in place.
                    let outcome = match reading {
                        Ok(reading) => {
                            if let Some(path) = &store {
                                save(path, reading.as_ref());
                            }
                            if let Ok(mut current) = shared.lock() {
                                *current = reading;
                            }
                            Some(None)
                        }
                        Err(reason) => epoch_seconds()
                            .filter(|at| *at > 0)
                            .map(|at| Some(QuotaFailure { at, reason })),
                    };
                    if let Some(outcome) = outcome
                        && let Ok(mut last) = failed.lock()
                    {
                        *last = outcome;
                    }
                    if !matches!(
                        stopped.recv_timeout(interval),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            })
            .ok();
        Self {
            quota,
            failure,
            stop,
            worker,
        }
    }

    fn snapshot(&self) -> Option<CodexQuota> {
        present_at(self.quota.lock().ok()?.clone()?, epoch_seconds()?)
    }

    fn failure(&self) -> Option<QuotaFailure> {
        *self.failure.lock().ok()?
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One provider's worker while its chip is shown, and the end of its last read, which outlives
/// the worker.
struct Slot {
    provider: &'static str,
    widget: &'static str,
    interval: Duration,
    last_read: Arc<Mutex<Option<Instant>>>,
    worker: Option<Provider>,
}

impl Slot {
    fn snapshot(&self) -> Option<CodexQuota> {
        self.worker.as_ref()?.snapshot()
    }

    fn failure(&self) -> Option<QuotaFailure> {
        self.worker.as_ref()?.failure()
    }
}

pub(crate) struct Quotas {
    program: PathBuf,
    /// Where each provider's last good reading is kept, if anywhere.
    store: Option<PathBuf>,
    codex: Slot,
    claude: Slot,
}

impl Quotas {
    pub(crate) fn start(program: &Path, widgets: &[&str]) -> Self {
        let store = super::diagnostics::state_directory().map(|state| state.join("quota"));
        Self::start_every(program, Duration::from_secs(1), store, widgets)
    }

    /// Reads Codex every 60 units and Claude every 300, whatever the outcome (EON-C27).
    fn start_every(
        program: &Path,
        unit: Duration,
        store: Option<PathBuf>,
        widgets: &[&str],
    ) -> Self {
        let slot = |provider, widget, units| Slot {
            provider,
            widget,
            interval: unit * units,
            last_read: Arc::default(),
            worker: None,
        };
        let mut quotas = Self {
            program: program.to_path_buf(),
            store,
            codex: slot("codex", "codex_quota", 60),
            claude: slot("claude", "claude_quota", 300),
        };
        quotas.show(widgets);
        quotas
    }

    /// Polls exactly the providers whose chips `[bar] widgets` lists (EON-C27).
    pub(crate) fn show(&mut self, widgets: &[&str]) {
        for slot in [&mut self.codex, &mut self.claude] {
            if !widgets.contains(&slot.widget) {
                slot.worker = None;
            } else if slot.worker.is_none() {
                slot.worker = Some(Provider::start(
                    self.program.clone(),
                    slot.provider,
                    slot.interval,
                    Duration::from_secs(10),
                    Arc::clone(&slot.last_read),
                    self.store
                        .as_ref()
                        .map(|store| store.join(format!("{}.json", slot.provider))),
                ));
            }
        }
    }

    pub(crate) fn apply(&self, snapshot: &mut Snapshot) {
        snapshot.codex_quota = self.codex.snapshot();
        snapshot.codex_quota_failure = self.codex.failure();
        snapshot.claude_quota_failure = self.claude.failure();
        snapshot.claude_quota = self.claude.snapshot().map(|quota| ClaudeQuota {
            state: if quota.state == CodexQuotaState::Stale {
                ClaudeQuotaState::Stale
            } else {
                ClaudeQuotaState::Fresh
            },
            observed_at: quota.observed_at,
            windows: quota.windows,
        });
    }
}

fn fetch(
    program: &Path,
    provider: &str,
    timeout: Duration,
    stop: &mpsc::Receiver<()>,
) -> Result<Option<Vec<u8>>, Failure> {
    let mut child = Command::new(program)
        .env_remove(super::sessions::TERMINAL_VARIABLE)
        .env_remove(super::sessions::TAB_VARIABLE)
        .args([
            "usage",
            "--provider",
            provider,
            "--source",
            if provider == "codex" { "cli" } else { "oauth" },
            "--format",
            "json",
            "--json-only",
            "--web-timeout",
            "8",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| Failure::Start)?;
    let mut output = child.stdout.take().expect("piped quota output");
    let result = (|| {
        let fd = output.as_raw_fd();
        // SAFETY: fcntl changes only the flags of this live pipe.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(Failure::Start);
        }
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut eof = false;
        loop {
            if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Deadline);
            }
            let mut chunk = [0; 8192];
            match output.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(count) => {
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.len() > MAX_BYTES {
                        return Err(Failure::OutputLimit);
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(Failure::Start),
            }
            if let Some(status) = child.try_wait().map_err(|_| Failure::Start)?
                && eof
            {
                return status
                    .success()
                    .then_some(Some(bytes))
                    .ok_or(Failure::ExitStatus);
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    // SAFETY: only the process group created above is targeted, including provider helpers.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1791936000;
    fn payload(provider: &str) -> Value {
        json!([{"provider":provider, "source":if provider == "codex" {"codex-cli"} else {"oauth"},
            "usage":{"updatedAt":"2026-10-14T00:00:00Z",
                "primary":{"usedPercent":23.5,"windowMinutes":300},
                "secondary":{"usedPercent":2,"windowMinutes":10080,"resetsAt":"2026-10-19T20:59:59Z"}},
            "identity":{"accountEmail":"discard@example.invalid"}}])
    }

    /// A one-window reading observed now, as a fake backend prints it.
    fn fresh(provider: &str) -> (Value, u64) {
        let mut fixture = payload(provider);
        let now = epoch_seconds().unwrap();
        let date = time::OffsetDateTime::from_unix_timestamp(now as i64).unwrap();
        fixture[0]["usage"]["updatedAt"] = json!(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            date.year(),
            date.month() as u8,
            date.day(),
            date.hour(),
            date.minute(),
            date.second()
        ));
        fixture[0]["usage"]["secondary"] = Value::Null;
        (fixture, now)
    }

    /// Writes an executable shell script; callers use a fresh path for each one (eon-1fxc). A
    /// child shell writes it: had this process opened it for writing, a child forked meanwhile by
    /// a parallel test would inherit that descriptor, and exec would fail with ETXTBSY.
    fn script(path: &Path, body: &str) {
        use std::{env, os::unix::fs::PermissionsExt};
        let sh = env::split_paths(&env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let status = Command::new(&sh)
            .args(["-c", "printf '%s' \"$1\" > \"$0\""])
            .arg(path)
            .arg(format!("#!{}\n{body}\n", sh.display()))
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn actual_periods_optional_resets_and_missing_quota_are_preserved() {
        for provider in ["codex", "claude"] {
            let bytes = serde_json::to_vec(&payload(provider)).unwrap();
            let quota = normalize(&bytes, provider, NOW).unwrap().unwrap();
            assert_eq!(quota.observed_at, NOW);
            assert_eq!(
                quota.windows[0],
                QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 76,
                    resets_at: None,
                    scope: None,
                }
            );
            assert_eq!(quota.windows[1].remaining_percent, 98);
            let mut weekly = payload(provider);
            weekly[0]["usage"]["primary"] = Value::Null;
            let quota = normalize(&serde_json::to_vec(&weekly).unwrap(), provider, NOW)
                .unwrap()
                .unwrap();
            assert_eq!(quota.windows.len(), 1);
            assert_eq!(quota.windows[0].duration_minutes, 10080);
            // CodexBar 0.74.1 repeats the weekly window as primary when Claude reports no
            // five-hour one (eon-8jdmw).
            let mut repeated = payload(provider);
            repeated[0]["usage"]["primary"] = repeated[0]["usage"]["secondary"].clone();
            repeated[0]["usage"]["extraRateWindows"] = json!([{"id":"claude-weekly-scoped-fable",
                "title":"Fable only","window":{"usedPercent":40,"windowMinutes":10080,
                    "resetsAt":"2026-10-19T20:59:59Z"}}]);
            let repeated = normalize(&serde_json::to_vec(&repeated).unwrap(), provider, NOW)
                .unwrap()
                .unwrap();
            // Claude's model-scoped window follows as its own window (eon-wqkjc); Codex reads
            // no extra windows.
            let mut expected = quota.windows.clone();
            if provider == "claude" {
                expected.push(QuotaWindow {
                    remaining_percent: 60,
                    scope: Some("Fable".into()),
                    ..quota.windows[0].clone()
                });
            }
            assert_eq!(repeated.windows, expected);
            // Other extra windows, and scoped ones without known usage, are not quota.
            let mut extras = payload(provider);
            extras[0]["usage"]["extraRateWindows"] = json!([
                {"id":"claude-routines","title":"Daily Routines",
                    "window":{"usedPercent":10,"windowMinutes":10080}},
                {"id":"claude-weekly-scoped-fable","title":"Fable only","usageKnown":false,
                    "window":{"usedPercent":0,"windowMinutes":10080}}]);
            let extras = normalize(&serde_json::to_vec(&extras).unwrap(), provider, NOW)
                .unwrap()
                .unwrap();
            assert_eq!(extras.windows.len(), 2);
            weekly[0]["usage"]["secondary"] = Value::Null;
            assert!(
                normalize(&serde_json::to_vec(&weekly).unwrap(), provider, NOW)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn invalid_source_values_and_timestamps_never_become_fresh_usage() {
        for (field, bad) in [
            ("usedPercent", json!(101)),
            ("usedPercent", json!(-1)),
            ("usedPercent", json!("10")),
            ("windowMinutes", json!(0)),
            ("windowMinutes", json!(300.5)),
            ("resetsAt", json!("garbage")),
        ] {
            let mut input = payload("codex");
            input[0]["usage"]["primary"][field] = bad;
            assert!(normalize(&serde_json::to_vec(&input).unwrap(), "codex", NOW).is_err());
        }
        for bad in [
            "2026-02-30T00:00:00Z",
            "2026-10-15T00:00:00Z",
            "1970-01-01T00:00:00Z",
        ] {
            let mut input = payload("claude");
            input[0]["usage"]["updatedAt"] = json!(bad);
            assert_eq!(
                normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW),
                Err(Failure::ObservationTime)
            );
        }
        for (bytes, reason) in [
            (b"not json".as_slice(), Failure::Malformed),
            (b"[]", Failure::Malformed),
            (b"[{},{}]", Failure::Malformed),
            (
                b"[{\"provider\":\"claude\",\"error\":{}}]",
                Failure::ProviderError,
            ),
        ] {
            assert_eq!(normalize(bytes, "codex", NOW), Err(reason));
        }
        // A scoped window never fails the reading: one with an unusable scope or value, a scope
        // whose repeats differ, and any past the fourth are left out; a long scope is cut.
        let scoped = |title: &str, minutes: u32| {
            json!({"id":format!("claude-weekly-scoped-{title}"),"title":title,
                "window":{"usedPercent":40,"windowMinutes":minutes}})
        };
        let mut invalid = scoped("Bad", 10080);
        invalid["window"]["usedPercent"] = json!(101);
        let mut repeat = scoped("Fable only", 10080);
        repeat["id"] = json!("claude-weekly-scoped-fable-2");
        let mut differing = repeat.clone();
        differing["window"]["usedPercent"] = json!(41);
        let multibyte = format!("x{}", "é".repeat(16));
        let mut expired = scoped("Old", 10080);
        expired["window"]["resetsAt"] = json!("2026-10-01T00:00:00Z");
        for (extras, scopes) in [
            (json!({}), vec![]),
            (json!([{"title":"Fable only"}]), vec![]),
            (
                json!([
                    scoped(" only", 10080),
                    scoped("a\u{7}", 10080),
                    scoped("Fable", 60),
                    invalid
                ]),
                vec![],
            ),
            (
                json!([scoped(&"x".repeat(33), 10080)]),
                vec!["x".repeat(32)],
            ),
            (
                json!([scoped(&multibyte, 10080)]),
                vec![format!("x{}", "é".repeat(15))],
            ),
            (
                json!(["a", "b", "c", "d", "e"].map(|t| scoped(t, 10080))),
                vec!["a".into(), "b".into(), "c".into(), "d".into()],
            ),
            // An expired window is not one of the four.
            (
                json!([
                    expired,
                    scoped("a", 10080),
                    scoped("b", 10080),
                    scoped("c", 10080),
                    scoped("d", 10080)
                ]),
                vec!["a".into(), "b".into(), "c".into(), "d".into()],
            ),
            (
                json!([scoped("Fable", 10080), repeat]),
                vec!["Fable".into()],
            ),
            (
                json!([
                    scoped("Fable", 10080),
                    scoped("Muse", 10080),
                    differing,
                    scoped("Fable", 10080)
                ]),
                vec!["Muse".into()],
            ),
        ] {
            let mut input = payload("claude");
            input[0]["usage"]["extraRateWindows"] = extras;
            let quota = normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW)
                .unwrap()
                .unwrap();
            assert!(quota.windows[..2].iter().all(|w| w.scope.is_none()));
            let read: Vec<String> = quota.windows[2..]
                .iter()
                .map(|w| w.scope.clone().unwrap())
                .collect();
            assert_eq!(read, scopes);
        }
        for (field, other) in [
            ("usedPercent", json!(3)),
            ("resetsAt", json!("2026-10-19T21:59:59Z")),
            ("resetsAt", Value::Null),
        ] {
            let mut input = payload("claude");
            input[0]["usage"]["primary"] = input[0]["usage"]["secondary"].clone();
            input[0]["usage"]["primary"][field] = other;
            assert!(normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW).is_err());
        }
        let mut input = payload("claude");
        input[0]["source"] = json!("web");
        assert!(normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW).is_err());
        input = payload("claude");
        input[0]["usage"]["dataConfidence"] = json!("estimated");
        assert_eq!(
            normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW),
            Err(Failure::Estimated)
        );
    }
    #[test]
    fn retained_observations_age_and_expire_without_invented_resets() {
        let quota = normalize(
            &serde_json::to_vec(&payload("claude")).unwrap(),
            "claude",
            NOW,
        )
        .unwrap()
        .unwrap();
        let state = |at| present_at(quota.clone(), at).map(|quota| quota.state);
        assert_eq!(state(NOW + 3600), Some(CodexQuotaState::Fresh));
        assert_eq!(state(NOW + 3601), Some(CodexQuotaState::Stale));
        assert_eq!(state(NOW + 86400), Some(CodexQuotaState::Stale));
        assert_eq!(state(NOW + 86401), None);
        assert!(present_at(quota.clone(), NOW - 1).is_none());
        let mut expires = quota;
        expires.windows[0].resets_at = Some(NOW + 10);
        assert_eq!(present_at(expires, NOW + 10).unwrap().windows.len(), 1);
    }

    #[test]
    fn workers_isolate_failure_and_bound_output_timeout_and_shutdown() {
        let root = super::super::supervisor::temporary_directory();
        let program = root.join("backend");
        let (fixture, now) = fresh("codex");
        let quota = normalize(&serde_json::to_vec(&fixture).unwrap(), "codex", now)
            .unwrap()
            .unwrap();
        script(
            &program,
            &format!(
                "if [ \"$3\" = claude ]; then sleep 30; else printf '%s' '{}'; fi",
                serde_json::to_string(&fixture).unwrap()
            ),
        );
        let (_stop, stopped) = mpsc::channel();
        let started = Instant::now();
        assert_eq!(
            fetch(&program, "claude", Duration::from_millis(50), &stopped),
            Err(Failure::Deadline)
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        let quotas = Quotas::start_every(
            &program,
            Duration::from_secs(1),
            None,
            &["codex_quota", "claude_quota"],
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while quotas.codex.snapshot().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            quotas.codex.snapshot().unwrap().windows[0].remaining_percent,
            76
        );
        assert!(quotas.claude.snapshot().is_none());
        let started = Instant::now();
        drop(quotas);
        assert!(started.elapsed() < Duration::from_secs(1));
        let provider = Provider::start(
            root.join("missing"),
            "codex",
            Duration::from_millis(20),
            Duration::from_secs(1),
            Arc::default(),
            None,
        );
        *provider.quota.lock().unwrap() = Some(quota.clone());
        // Several 20 ms intervals of failed reads leave a recent reading fresh, and say why.
        thread::sleep(Duration::from_millis(200));
        let retained = provider.snapshot().unwrap();
        assert_eq!(retained.state, CodexQuotaState::Fresh);
        assert_eq!(retained.windows[0].remaining_percent, 76);
        let failure = provider.failure().unwrap();
        assert_eq!(failure.reason, Failure::Start);
        assert!(failure.at.abs_diff(epoch_seconds().unwrap()) < 5);
        drop(provider);
        let program = root.join("flood");
        script(&program, "while :; do printf '%01024d' 0; done");
        assert_eq!(
            fetch(&program, "codex", Duration::from_secs(1), &stopped),
            Err(Failure::OutputLimit)
        );
        let program = root.join("exits");
        script(&program, "exit 3");
        assert_eq!(
            fetch(&program, "codex", Duration::from_secs(1), &stopped),
            Err(Failure::ExitStatus)
        );
        // A rejected reading records why and when, keeps the retained one, and a later good read
        // clears it; each provider's failure reaches its own snapshot field (eon-d0kyj).
        let mut rejected = fixture.clone();
        rejected[0]["usage"]["dataConfidence"] = json!("estimated");
        let program = root.join("rejects");
        script(
            &program,
            &format!(
                "[ \"$3\" = claude ] && exit 3
                count=$(cat '{count}' 2>/dev/null || echo 0); echo $((count + 1)) > '{count}'
                case $count in 0) printf '%s' '{good}';; 1) printf '%s' '{rejected}';;
                *) until [ -e '{go}' ]; do sleep 0.01; done; printf '%s' '{good}';; esac",
                count = root.join("count").display(),
                go = root.join("go").display(),
                good = serde_json::to_string(&fixture).unwrap(),
                rejected = serde_json::to_string(&rejected).unwrap(),
            ),
        );
        let quotas = Quotas::start_every(
            &program,
            Duration::from_millis(1),
            None,
            &["codex_quota", "claude_quota"],
        );
        let mut snapshot = Snapshot {
            workspace: String::new(),
            active_workspace: String::new(),
            workspaces: vec![],
            active_tab: String::new(),
            geometry: eon_workspace_protocol::v27::PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: vec![],
            tabs: vec![],
            codex_quota: None,
            claude_quota: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            sidebar: eon_workspace_protocol::v27::Sidebar::Off,
            attention_queue: vec![],
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while (snapshot.codex_quota_failure.is_none() || snapshot.claude_quota_failure.is_none())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
            quotas.apply(&mut snapshot);
        }
        let failure = snapshot.codex_quota_failure.unwrap();
        assert_eq!(failure.reason, Failure::Estimated);
        assert!(failure.at.abs_diff(epoch_seconds().unwrap()) < 5);
        assert_eq!(
            snapshot.codex_quota.as_ref().unwrap().windows,
            quota.windows
        );
        assert_eq!(
            snapshot.claude_quota_failure.unwrap().reason,
            Failure::ExitStatus
        );
        assert!(snapshot.claude_quota.is_none());
        fs::write(root.join("go"), "").unwrap();
        quotas.apply(&mut snapshot);
        while snapshot.codex_quota_failure.is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            quotas.apply(&mut snapshot);
        }
        assert!(snapshot.codex_quota_failure.is_none());
        assert!(snapshot.codex_quota.is_some());
        drop(quotas);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_waits_300_units_after_a_failed_read_while_codex_reads_every_60() {
        let root = super::super::supervisor::temporary_directory();
        let program = root.join("backend");
        let log = root.join("reads");
        fs::create_dir(&log).unwrap();
        // Each read leaves a file whose kernel mtime marks its start; every Claude read fails, as
        // a rate-limited OAuth read does.
        script(
            &program,
            &format!(": > '{}'/\"$3.$$\"\n[ \"$3\" = codex ]", log.display()),
        );
        let gaps = |provider: &str| {
            let mut starts = fs::read_dir(&log)
                .unwrap()
                .map(|entry| entry.unwrap())
                .filter(|entry| {
                    let name = entry.file_name();
                    name.to_string_lossy().split('.').next() == Some(provider)
                })
                .map(|entry| entry.metadata().unwrap().modified().unwrap())
                .collect::<Vec<_>>();
            starts.sort();
            starts
                .windows(2)
                .map(|pair| pair[1].duration_since(pair[0]).unwrap())
                .collect::<Vec<_>>()
        };
        let unit = Duration::from_millis(20);
        let mut quotas =
            Quotas::start_every(&program, unit, None, &["codex_quota", "claude_quota"]);
        let deadline = Instant::now() + unit * 1000;
        while gaps("claude").is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // Hidden briefly and shown again, Claude still waits 300 units from its last read.
        quotas.show(&["codex_quota"]);
        thread::sleep(unit * 100);
        quotas.show(&["codex_quota", "claude_quota"]);
        let deadline = Instant::now() + unit * 1000;
        while gaps("claude").len() < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // With every chip hidden, no provider is read, though Codex was due every 60 units.
        quotas.show(&[]);
        let reads = fs::read_dir(&log).unwrap().count();
        thread::sleep(unit * 100);
        assert_eq!(fs::read_dir(&log).unwrap().count(), reads);
        drop(quotas);
        let claude = gaps("claude");
        assert!(
            claude.len() == 2 && claude.iter().all(|gap| *gap >= unit * 300),
            "Claude read gaps {claude:?}"
        );
        // The upper bound leaves 60 units of slack for process startup under load.
        let codex = gaps("codex");
        assert!(
            codex.len() >= 3
                && codex
                    .iter()
                    .all(|gap| *gap >= unit * 60 && *gap < unit * 120),
            "Codex read gaps {codex:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stored_readings_outlive_the_supervisor_and_bad_ones_are_ignored() {
        let root = super::super::supervisor::temporary_directory();
        let store = root.join("quota");
        let widgets = ["codex_quota", "claude_quota"];
        let shown = |quotas: &Quotas| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while quotas.codex.snapshot().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            quotas.codex.snapshot()
        };
        let (fixture, now) = fresh("codex");
        let reads = root.join("reads");
        script(
            &reads,
            &format!(
                "[ \"$3\" = codex ] && printf '%s' '{}'",
                serde_json::to_string(&fixture).unwrap()
            ),
        );
        let quotas = Quotas::start_every(
            &reads,
            Duration::from_secs(1),
            Some(store.clone()),
            &widgets,
        );
        assert_eq!(shown(&quotas).unwrap().windows[0].remaining_percent, 76);
        drop(quotas);
        assert!(!store.join("claude.json").exists());
        // A later supervisor whose every read fails, as a rate-limited one's does, shows it.
        let quotas = Quotas::start_every(
            &root.join("missing"),
            Duration::from_secs(1),
            Some(store.clone()),
            &widgets,
        );
        let retained = shown(&quotas).unwrap();
        assert_eq!(retained.state, CodexQuotaState::Fresh);
        assert_eq!(retained.windows[0].remaining_percent, 76);
        assert!(quotas.claude.snapshot().is_none());
        drop(quotas);
        // It ages as a retained reading does.
        let path = store.join("codex.json");
        let stored = load(&path, "codex", now).unwrap();
        assert_eq!(
            present_at(stored.clone(), now + STALE_AFTER + 1)
                .unwrap()
                .state,
            CodexQuotaState::Stale
        );
        assert!(present_at(stored.clone(), now + EXPIRE_AFTER + 1).is_none());
        let window = json!({"duration_minutes":300,"remaining_percent":76,"resets_at":null});
        for (provider, bad) in [
            (
                "codex",
                json!({"version":1,"observed_at":now + 60,"windows":[window]}),
            ),
            (
                "codex",
                json!({"version":2,"observed_at":now,"windows":[window]}),
            ),
            (
                "codex",
                json!({"version":1,"observed_at":0,"windows":[window]}),
            ),
            ("codex", json!({"version":1,"observed_at":now,"windows":[]})),
            (
                "codex",
                json!({"version":1,"observed_at":now,"windows":[window, window]}),
            ),
            (
                "codex",
                json!({"version":1,"observed_at":now,"windows":[window],"account":"x"}),
            ),
            (
                "codex",
                json!({"version":1,"observed_at":now,
                "windows":[{"duration_minutes":300,"remaining_percent":101,"resets_at":null}]}),
            ),
            (
                "claude",
                json!({"version":1,"observed_at":now,
                "windows":[{"duration_minutes":60,"remaining_percent":76,"resets_at":null}]}),
            ),
        ] {
            fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
            assert!(load(&path, provider, now).is_none(), "{bad}");
        }
        fs::write(&path, b"not json").unwrap();
        assert!(load(&path, "codex", now).is_none());
        // A valid reading padded past the size bound is not read as its first part.
        let mut long =
            serde_json::to_vec(&json!({"version":1,"observed_at":now,"windows":[window]})).unwrap();
        long.resize(MAX_BYTES, b' ');
        long.push(b'x');
        fs::write(&path, long).unwrap();
        assert!(load(&path, "codex", now).is_none());
        // A successful read that finds no quota clears the stored reading too.
        save(&path, Some(&stored));
        let mut empty = fixture;
        empty[0]["usage"] = Value::Null;
        let clears = root.join("clears");
        script(
            &clears,
            &format!("printf '%s' '{}'", serde_json::to_string(&empty).unwrap()),
        );
        let quotas = Quotas::start_every(
            &clears,
            Duration::from_secs(1),
            Some(store.clone()),
            &["codex_quota"],
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while (path.exists() || quotas.codex.snapshot().is_some()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!path.exists());
        assert!(quotas.codex.snapshot().is_none());
        drop(quotas);
        // A scoped window keeps its scope; a reading without one stores no scope key.
        let path = store.join("claude.json");
        let mut scoped = stored.clone();
        scoped.windows[0].duration_minutes = 10080;
        save(&path, Some(&scoped));
        assert!(!fs::read_to_string(&path).unwrap().contains("scope"));
        scoped.windows.push(QuotaWindow {
            scope: Some("Fable".into()),
            ..scoped.windows[0].clone()
        });
        save(&path, Some(&scoped));
        assert_eq!(load(&path, "claude", now).unwrap().windows, scoped.windows);
        fs::remove_dir_all(root).unwrap();
    }
}
