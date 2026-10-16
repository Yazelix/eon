use eon_workspace_protocol::v10::{
    ClaudeQuota, ClaudeQuotaState, CodexQuota, CodexQuotaState, QuotaWindow, Snapshot,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{self, Read},
    os::{
        fd::{AsFd, AsRawFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: usize = 64 * 1024;
/// A reading older than this is rejected when read and shown `old` once retained.
const STALE_AFTER: u64 = 3600;
const EXPIRE_AFTER: u64 = 24 * 3600;

fn timestamp(value: &Value) -> Result<u64, ()> {
    let value = value.as_str().ok_or(())?;
    let date = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| ())?;
    u64::try_from(date.unix_timestamp()).map_err(|_| ())
}

fn normalize(bytes: &[u8], provider: &str, now: u64) -> Result<Option<CodexQuota>, ()> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    let [result] = value.as_array().ok_or(())?.as_slice() else {
        return Err(());
    };
    let source = if provider == "codex" {
        "codex-cli"
    } else {
        "oauth"
    };
    if result["provider"] != provider || result["source"] != source || !result["error"].is_null() {
        return Err(());
    }
    let usage = &result["usage"];
    if usage.is_null() {
        return Ok(None);
    }
    if !usage["dataConfidence"].is_null() && usage["dataConfidence"] != "authoritative" {
        return Err(());
    }
    let observed_at = timestamp(&usage["updatedAt"])?;
    if observed_at == 0 || observed_at > now || now - observed_at > STALE_AFTER {
        return Err(());
    }
    // The selected providers have at most two windows; silently dropping another is incorrect.
    if !usage["tertiary"].is_null() {
        return Err(());
    }
    let mut windows = Vec::new();
    for name in ["primary", "secondary"] {
        let value = &usage[name];
        if value.is_null() {
            continue;
        }
        let used = value["usedPercent"]
            .as_f64()
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            .ok_or(())?;
        let duration_minutes = value["windowMinutes"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or(())?;
        if provider == "claude" && !matches!(duration_minutes, 300 | 10080) {
            return Err(());
        }
        if windows
            .iter()
            .any(|w: &QuotaWindow| w.duration_minutes == duration_minutes)
        {
            return Err(());
        }
        let resets_at = (!value["resetsAt"].is_null())
            .then(|| timestamp(&value["resetsAt"]))
            .transpose()?;
        if resets_at.is_some_and(|reset| reset <= now) {
            continue;
        }
        windows.push(QuotaWindow {
            duration_minutes,
            remaining_percent: (100.0 - used).floor() as u8,
            resets_at,
        });
    }
    Ok((!windows.is_empty()).then_some(CodexQuota {
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

fn epoch_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

struct Provider {
    quota: Arc<Mutex<Option<CodexQuota>>>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Provider {
    fn start(
        program: PathBuf,
        provider: &'static str,
        interval: Duration,
        timeout: Duration,
    ) -> Self {
        let quota = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&quota);
        let (stop, stopped) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(format!("eon-{provider}-quota"))
            .spawn(move || {
                loop {
                    let reading = match fetch(&program, provider, timeout, &stopped) {
                        Ok(None) => break,
                        Ok(Some(bytes)) => epoch_seconds()
                            .ok_or(())
                            .and_then(|now| normalize(&bytes, provider, now)),
                        Err(()) => Err(()),
                    };
                    // A failed read keeps the last good reading; its age alone marks it old.
                    if let Ok(reading) = reading
                        && let Ok(mut current) = shared.lock()
                    {
                        *current = reading;
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
            stop,
            worker,
        }
    }

    fn snapshot(&self) -> Option<CodexQuota> {
        present_at(self.quota.lock().ok()?.clone()?, epoch_seconds()?)
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

pub(crate) struct Quotas {
    codex: Provider,
    claude: Provider,
}

impl Quotas {
    pub(crate) fn start(program: &Path) -> Self {
        let start = |provider| {
            Provider::start(
                program.to_path_buf(),
                provider,
                Duration::from_secs(60),
                Duration::from_secs(10),
            )
        };
        Self {
            codex: start("codex"),
            claude: start("claude"),
        }
    }

    pub(crate) fn apply(&self, snapshot: &mut Snapshot) {
        snapshot.codex_quota = self.codex.snapshot();
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
) -> Result<Option<Vec<u8>>, ()> {
    let mut child = Command::new(program)
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
        .map_err(|_| ())?;
    let mut output = child.stdout.take().expect("piped quota output");
    let result = (|| {
        let fd = output.as_raw_fd();
        // SAFETY: fcntl changes only the flags of this live pipe.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(());
        }
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut eof = false;
        loop {
            if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(());
            }
            let mut chunk = [0; 8192];
            match output.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(count) => {
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.len() > MAX_BYTES {
                        return Err(());
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(()),
            }
            if let Some(status) = child.try_wait().map_err(|_| ())?
                && eof
            {
                return status.success().then_some(Some(bytes)).ok_or(());
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

pub(crate) fn legacy_statusline() -> Result<i32, String> {
    // Drain ordinary callbacks so old wrappers with pipefail can still run their own renderer.
    let mut input = File::from(
        io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|e| e.to_string())?,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut remaining = MAX_BYTES;
    while remaining > 0 && Instant::now() < deadline {
        let mut poll = libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll writes only this live, single-descriptor array.
        if unsafe { libc::poll(&mut poll, 1, 100) } <= 0 {
            continue;
        }
        let mut chunk = [0; 4096];
        match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => remaining -= count.min(remaining),
            Err(_) => break,
        }
    }
    Ok(0)
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
                    resets_at: None
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
            assert!(normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW).is_err());
        }
        for bytes in [
            b"not json".as_slice(),
            b"[]",
            b"[{},{}]",
            b"[{\"provider\":\"claude\",\"error\":{}}]",
        ] {
            assert!(normalize(bytes, "codex", NOW).is_err());
        }
        let mut input = payload("claude");
        input[0]["source"] = json!("web");
        assert!(normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW).is_err());
        input = payload("claude");
        input[0]["usage"]["dataConfidence"] = json!("estimated");
        assert!(normalize(&serde_json::to_vec(&input).unwrap(), "claude", NOW).is_err());
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
        use std::{env, fs, os::unix::fs::PermissionsExt};
        let root = super::super::supervisor::temporary_directory();
        let sh = env::split_paths(&env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let program = root.join("backend");
        let mut fixture = payload("codex");
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
        let quota = normalize(&serde_json::to_vec(&fixture).unwrap(), "codex", now)
            .unwrap()
            .unwrap();
        let encoded = serde_json::to_string(&fixture).unwrap();
        fs::write(
            &program,
            format!(
                "#!{}\nif [ \"$3\" = claude ]; then sleep 30; else printf '%s' '{}'; fi\n",
                sh.display(),
                encoded
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let (_stop, stopped) = mpsc::channel();
        let started = Instant::now();
        assert!(fetch(&program, "claude", Duration::from_millis(50), &stopped).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let quotas = Quotas::start(&program);
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
        );
        *provider.quota.lock().unwrap() = Some(quota);
        // Several 20 ms intervals of failed reads leave a recent reading fresh.
        thread::sleep(Duration::from_millis(200));
        let retained = provider.snapshot().unwrap();
        assert_eq!(retained.state, CodexQuotaState::Fresh);
        assert_eq!(retained.windows[0].remaining_percent, 76);
        drop(provider);
        fs::write(
            &program,
            format!("#!{}\nwhile :; do printf '%01024d' 0; done\n", sh.display()),
        )
        .unwrap();
        assert!(fetch(&program, "codex", Duration::from_secs(1), &stopped).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
