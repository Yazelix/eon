//! Opt-in build execution; the local slot scheduler remains the admission owner.
pub(super) mod local;
mod tree;

use crate::config::BoatConfig;
use crate::launch::{atomic_write, ensure_parent};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const CONTROL: u64 = 30;
const CLEANUP: u64 = 60;
static SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn cancel(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
}

struct Signals(Vec<(i32, libc::sighandler_t)>);

impl Signals {
    fn install() -> Self {
        SIGNAL.store(0, Ordering::Relaxed);
        Self(
            [libc::SIGINT, libc::SIGTERM, libc::SIGHUP]
                .into_iter()
                .map(|signal| {
                    // The handler only writes an atomic integer.
                    (signal, unsafe {
                        libc::signal(signal, cancel as *const () as libc::sighandler_t)
                    })
                })
                .collect(),
        )
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, previous) in &self.0 {
            unsafe {
                libc::signal(*signal, *previous);
            }
        }
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    ensure_parent(&path.join("entry")).map_err(|_| "boat-state-directory")?;
    let metadata = fs::symlink_metadata(path).map_err(|_| "boat-state-directory")?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("unsafe-boat-state-directory".into());
    }
    Ok(())
}

fn read_private<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Ok(meta)
            if meta.is_file()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.len() <= 65536 => {}
        _ => return Err("unsafe-or-unreadable-boat-record".into()),
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "boat-record-read")?)
        .map(Some)
        .map_err(|_| "boat-record-format".into())
}

#[derive(Deserialize, Serialize)]
pub(super) struct Record {
    schema: u8,
    pub run_id: String,
    pub pid: u32,
    pub phase: String,
    #[serde(rename = "where")]
    location: String,
    pub vm_id: Option<String>,
    command_pid: Option<u64>,
    tree: String,
    base: String,
    started: u64,
    expires_at: Option<u64>,
    duration_seconds: Option<u64>,
    exit_code: Option<u8>,
    usage: Option<Value>,
    environment_version: Option<u64>,
    reason: Option<String>,
    credentials_absent: bool,
}

pub(super) struct Lease {
    _lock: File,
    root: PathBuf,
}

impl Lease {
    pub fn claim(root: &Path) -> Result<Option<Self>, String> {
        let directory = root.join("boat");
        private_directory(&directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .mode(0o600)
            .open(directory.join("lease.lock"))
            .map_err(|_| "boat-lease-open")?;
        let meta = lock.metadata().map_err(|_| "boat-lease-metadata")?;
        if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err("unsafe-boat-lease".into());
        }
        match lock.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self {
                _lock: lock,
                root: root.to_owned(),
            })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(_) => Err("boat-lease-lock".into()),
        }
    }

    fn save(&self, record: &Record) -> Result<(), String> {
        let bytes = serde_json::to_vec(record).map_err(|_| "boat-record-format")?;
        atomic_write(&self.root.join("boat/current.json"), &bytes)
            .map_err(|_| "boat-record-write")?;
        atomic_write(
            &self
                .root
                .join("runs")
                .join(&record.run_id)
                .join("receipt.json"),
            &bytes,
        )
        .map_err(|_| "boat-receipt-write".to_owned())
    }
}

pub(super) fn listing(root: &Path) -> Result<Option<Record>, String> {
    let record: Option<Record> = read_private(&root.join("boat/current.json"))?;
    if record.as_ref().is_some_and(|record| {
        record.schema != 1
            || record.location != "boat"
            || record.run_id.len() != 32
            || !record
                .run_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        return Err("incompatible-boat-record".into());
    }
    Ok(record)
}

#[derive(Deserialize, Serialize)]
struct Warm {
    schema: u8,
    vm_id: String,
    lock_key: String,
    snapshot: Option<Saved>,
}

#[derive(Deserialize, Serialize)]
struct Saved {
    name: String,
    lock_key: String,
    source_vm: String,
    snapshot_id: String,
}

fn warm_cache(root: &Path) -> Option<Warm> {
    read_private::<Warm>(&root.join("boat/warm.json"))
        .ok()
        .flatten()
        .filter(|cache| cache.schema == 1)
}

fn snapshot_ready<'a>(list: &'a Value, saved: &Saved) -> Option<&'a Value> {
    list["named"].as_array()?.iter().find(|snapshot| {
        snapshot["name"] == saved.name
            && snapshot["status"] == "ready"
            && snapshot["sourceSandboxId"] == saved.source_vm
            && (saved.snapshot_id.is_empty() || snapshot["snapshotId"] == saved.snapshot_id)
            && snapshot["snapshotId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
    })
}

fn publish_snapshot(client: &Client, config: &BoatConfig, warm: &mut Warm) -> Result<(), String> {
    let limits = client.json(&["limits", "--all"])?;
    if limits["accessTier"] == "trial" {
        return Ok(());
    }
    if limits["subscriptionStatus"] != "active" {
        return Err("snapshot-plan-not-paid".into());
    }
    if limits["keepGoing"]["enabled"] != false {
        return Err("snapshot-budget-unsafe".into());
    }
    let list = client.json(&["snapshots"])?;
    if let Some(saved) = warm.snapshot.as_mut() {
        if saved.name == config.snapshot && saved.lock_key == warm.lock_key {
            if let Some(ready) = snapshot_ready(&list, saved) {
                saved.snapshot_id = ready["snapshotId"]
                    .as_str()
                    .ok_or("snapshot-identity-unknown")?
                    .into();
                return Ok(());
            }
        }
    }
    let named = list["named"]
        .as_array()
        .ok_or("snapshot-allowance-unknown")?;
    if let Some(existing) = named
        .iter()
        .find(|snapshot| snapshot["name"] == config.snapshot)
    {
        if existing["sourceSandboxId"] != warm.vm_id
            && !warm.snapshot.as_ref().is_some_and(|saved| {
                existing["sourceSandboxId"] == saved.source_vm && saved.name == config.snapshot
            })
        {
            return Err("snapshot-owner-unknown".into());
        }
    } else {
        let allowance = &list["namedAllowance"];
        let used = allowance["used"]
            .as_u64()
            .ok_or("snapshot-allowance-unknown")?;
        let included = allowance["included"]
            .as_u64()
            .ok_or("snapshot-allowance-unknown")?;
        if used >= included || allowance["extra"].as_u64() != Some(0) {
            return Err("snapshot-included-allowance-exhausted".into());
        }
    }
    client.json(&["snapshot", &warm.vm_id, &config.snapshot])?;
    // An accepted save can still be pinning; keep its source identity for later readiness.
    warm.snapshot = Some(Saved {
        name: config.snapshot.clone(),
        lock_key: warm.lock_key.clone(),
        source_vm: warm.vm_id.clone(),
        snapshot_id: String::new(),
    });
    let list = client.json(&["snapshots"])?;
    let saved = list["named"]
        .as_array()
        .and_then(|snapshots| {
            snapshots.iter().find(|snapshot| {
                snapshot["name"] == config.snapshot
                    && snapshot["status"] == "ready"
                    && snapshot["sourceSandboxId"] == warm.vm_id
            })
        })
        .ok_or("snapshot-not-ready")?;
    warm.snapshot = Some(Saved {
        name: config.snapshot.clone(),
        lock_key: warm.lock_key.clone(),
        source_vm: warm.vm_id.clone(),
        snapshot_id: saved["snapshotId"]
            .as_str()
            .ok_or("snapshot-identity-unknown")?
            .into(),
    });
    Ok(())
}

struct Client {
    deadline: Instant,
    cleanup: bool,
    reserve_seconds: f64,
}

impl Client {
    fn command(&self, json: bool, args: &[&str]) -> Command {
        let mut cmd = Command::new("boat");
        cmd.env_remove("SSH_AUTH_SOCK");
        if args.first() == Some(&"push") {
            // Normalize tar metadata instead of exporting the host account's ownership names.
            cmd.env("TAR_OPTIONS", "--owner=0 --group=0 --numeric-owner");
        }
        cmd.args(["--no-update", "--org", "personal"]);
        if json {
            cmd.arg("--json");
        }
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        cmd
    }

    fn invoke(
        &self,
        json: bool,
        args: &[&str],
        stream: Option<&File>,
    ) -> Result<(u8, Vec<u8>), String> {
        if Instant::now() >= self.deadline {
            return Err("boat-client-timeout".into());
        }
        let logs = [
            stream.map(File::try_clone).transpose(),
            stream.map(File::try_clone).transpose(),
        ];
        let [stdout_log, stderr_log] = logs;
        let logs = [
            stdout_log.map_err(|_| "boat-live-log")?,
            stderr_log.map_err(|_| "boat-live-log")?,
        ];
        let frames =
            stream.is_some() && args.first() == Some(&"exec") && args.contains(&"--attach");
        let mut child = self
            .command(json || frames, args)
            .spawn()
            .map_err(|_| "boat-unreachable")?;
        let last_output = Arc::new(AtomicU64::new(super::now_ns() / super::SECOND));
        let mut readers = Vec::new();
        for ((mut input, stderr), log) in [
            (
                Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
                false,
            ),
            (
                Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
                true,
            ),
        ]
        .into_iter()
        .zip(logs)
        {
            let last_output = last_output.clone();
            readers.push(std::thread::spawn(move || {
                let mut saved = Vec::new();
                let mut log = log;
                if frames && !stderr {
                    let mut input = io::BufReader::new(input);
                    let mut line = Vec::new();
                    while input.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                        if let Ok(frame) = serde_json::from_slice::<Value>(&line) {
                            if let Some(data) = frame["data"].as_str() {
                                if !data.is_empty()
                                    && matches!(frame["type"].as_str(), Some("stdout" | "stderr"))
                                {
                                    last_output
                                        .store(super::now_ns() / super::SECOND, Ordering::Relaxed);
                                }
                                if let Some(log) = &mut log {
                                    let _ = log.write_all(data.as_bytes());
                                }
                                if frame["type"] == "stderr" {
                                    let _ = io::stderr().write_all(data.as_bytes());
                                } else if frame["type"] == "stdout" {
                                    let _ = io::stdout().write_all(data.as_bytes());
                                }
                            }
                        }
                        line.clear();
                    }
                    return saved;
                }
                let mut buffer = [0u8; 16384];
                while let Ok(n) = input.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    last_output.store(super::now_ns() / super::SECOND, Ordering::Relaxed);
                    if let Some(log) = &mut log {
                        let _ = log.write_all(&buffer[..n]);
                        if stderr {
                            let _ = io::stderr().write_all(&buffer[..n]);
                        } else {
                            let _ = io::stdout().write_all(&buffer[..n]);
                        }
                    } else if !stderr && saved.len() < 1024 * 1024 {
                        saved.extend_from_slice(&buffer[..n.min(1024 * 1024 - saved.len())]);
                    }
                }
                saved
            }));
        }
        let deadline = if stream.is_some() {
            self.deadline
        } else {
            self.deadline
                .min(Instant::now() + Duration::from_secs(CONTROL))
        };
        let mut next_budget = Instant::now() + Duration::from_secs(CONTROL);
        let result: Result<u8, String> = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break Ok(status
                        .code()
                        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
                        as u8);
                }
                Ok(None) => (),
                Err(_) => break Err("boat-client-wait".into()),
            }
            if stream.is_some() && !self.cleanup && Instant::now() >= next_budget {
                let limits = self.json(&["limits", "--all"]);
                let safe = limits.as_ref().is_ok_and(|limits| {
                    limits["keepGoing"]["enabled"] == false
                        && remaining(limits)
                            .is_ok_and(|seconds| seconds > self.reserve_seconds + CLEANUP as f64)
                });
                if !safe {
                    break Err("remote-budget-observation-or-reserve".into());
                }
                next_budget = Instant::now() + Duration::from_secs(CONTROL);
            }
            let canceled = !self.cleanup && SIGNAL.load(Ordering::Relaxed) != 0;
            let stalled = stream.is_some()
                && (super::now_ns() / super::SECOND)
                    .saturating_sub(last_output.load(Ordering::Relaxed))
                    >= 600;
            if canceled || stalled || Instant::now() >= deadline {
                break Err(if canceled {
                    "interrupted"
                } else if stalled {
                    "remote-no-progress"
                } else {
                    "boat-client-timeout"
                }
                .into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if result.is_err() {
            // This group belongs to our unreaped direct child, not a process-name match.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
        let stdout = readers.remove(0).join().map_err(|_| "boat-client-output")?;
        for reader in readers {
            let _ = reader.join();
        }
        Ok((result?, stdout))
    }

    fn json(&self, args: &[&str]) -> Result<Value, String> {
        let (code, bytes) = self.invoke(true, args, None)?;
        decode(code, &bytes, args[0])
    }

    fn success(&self, args: &[&str], log: Option<&File>) -> Result<(), String> {
        if self.invoke(false, args, log)?.0 == 0 {
            Ok(())
        } else {
            Err("boat-command-failed".into())
        }
    }
}

fn decode(code: u8, bytes: &[u8], operation: &str) -> Result<Value, String> {
    let values = serde_json::Deserializer::from_slice(bytes)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "unsupported-provider-output")?;
    if code != 0
        || values.iter().any(|value| {
            value["ok"] == false || value["success"] == false || value["event"] == "error"
        })
    {
        return Err("boat-command-failed".into());
    }
    let value = if matches!(operation, "new" | "resume" | "info") {
        values
            .into_iter()
            .rev()
            .find(|value| sandbox(value)["id"].as_str().is_some_and(safe_id))
    } else {
        values.into_iter().last()
    };
    value.ok_or_else(|| "unsupported-provider-output".into())
}

fn sandbox(value: &Value) -> &Value {
    value.get("sandbox").unwrap_or(value)
}

fn environment(value: &Value) -> Result<u64, String> {
    let value = value.get("environment").unwrap_or(value);
    for key in [
        "passGithub",
        "passSecrets",
        "passSandboxCredentials",
        "passAgentsCredentials",
        "proxyAiCredentials",
    ] {
        if value[key].as_bool() != Some(false) {
            return Err("environment-unsafe".into());
        }
    }
    if value["safeForThirdParties"].as_bool() != Some(true)
        || value["variables"].as_u64() != Some(0)
        || !value["secretFiles"].as_array().is_some_and(Vec::is_empty)
        || !value["selectedRepositories"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("environment-unsafe".into());
    }
    value["latestVersionNumber"]
        .as_u64()
        .ok_or_else(|| "unsupported-provider-output".into())
}

fn stop_vm(client: &Client, id: &str) -> Result<(), String> {
    let _ = client.json(&["stop", id]);
    let stop_deadline = (Instant::now() + Duration::from_secs(45)).min(
        client
            .deadline
            .checked_sub(Duration::from_secs(15))
            .unwrap_or_else(Instant::now),
    );
    loop {
        if let Ok(info) = client.json(&["info", id]) {
            if matches!(
                sandbox(&info)["state"].as_str(),
                Some("stopped" | "archived")
            ) {
                return Ok(());
            }
        }
        if Instant::now() >= stop_deadline {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    delete_vm(client, id)
}

fn safe_id(id: &str) -> bool {
    id.starts_with("bx_")
        && id.len() <= 80
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn delete_vm(client: &Client, id: &str) -> Result<(), String> {
    let _ = client.json(&["delete", "--yes", id]);
    let list = client.json(&["list", "--all"])?;
    if list["sandboxes"]
        .as_array()
        .is_some_and(|vms| vms.iter().all(|vm| vm["id"].as_str() != Some(id)))
    {
        Ok(())
    } else {
        Err("remote-cleanup-unknown".into())
    }
}

fn usage(client: &Client, id: &str, started: u64) -> Option<Value> {
    client.json(&["usage", id, "--since", &started.to_string()]).ok().map(|usage| {
        serde_json::json!({"seconds": usage["seconds"], "dollars": usage["dollars"], "running": usage["running"]})
    })
}

fn recover(client: &Client, lease: &Lease) -> Result<(), String> {
    let Some(mut old) = listing(&lease.root)? else {
        return Ok(());
    };
    if old.phase == "stopped" {
        return Ok(());
    }
    if old.vm_id.is_none() {
        let list = client.json(&["list", "--all"])?;
        let vms = list["sandboxes"]
            .as_array()
            .ok_or("unsupported-provider-output")?;
        let matched: Vec<_> = vms
            .iter()
            .filter(|vm| vm["name"].as_str() == Some(&format!("eon-build-{}", old.run_id)))
            .collect();
        if matched.is_empty()
            && old
                .expires_at
                .is_some_and(|expires| super::now_ns() / super::SECOND > expires)
        {
            old.phase = "stopped".into();
            old.reason = Some("expired-create-absent".into());
            return lease.save(&old);
        }
        if matched.len() != 1 {
            return Err("remote-create-unknown".into());
        }
        old.vm_id = matched[0]["id"]
            .as_str()
            .filter(|id| safe_id(id))
            .map(str::to_owned);
        lease.save(&old)?;
    }
    let id = old
        .vm_id
        .as_deref()
        .filter(|id| safe_id(id))
        .ok_or("remote-create-unknown")?;
    stop_vm(client, id)?;
    old.usage = usage(client, id, old.started);
    // Stopping can suspend detached work; an uncertain VM must never be resumed.
    delete_vm(client, id)?;
    old.phase = "stopped".into();
    old.reason
        .get_or_insert_with(|| "recovered-after-wrapper-loss".into());
    old.duration_seconds = Some((super::now_ns() / super::SECOND).saturating_sub(old.started));
    lease.save(&old)
}

pub(super) enum Outcome {
    Complete(u8),
    Local(String),
}

fn fallback(reason: String) -> Result<Outcome, String> {
    let signal = SIGNAL.load(Ordering::Relaxed);
    Ok(if signal == 0 {
        Outcome::Local(reason)
    } else {
        Outcome::Complete((128 + signal) as u8)
    })
}

pub(super) fn run(lease: Lease, config: &BoatConfig, argv: &[String]) -> Result<Outcome, String> {
    let _signals = Signals::install();
    let started = super::now_ns() / super::SECOND;
    let client = Client {
        deadline: Instant::now() + Duration::from_secs(config.timeout_seconds - CLEANUP),
        cleanup: false,
        reserve_seconds: config.reserve_hours * 3600.0,
    };
    if let Err(reason) = recover(&client, &lease) {
        return fallback(reason);
    }
    let source = match tree::Source::capture(&std::env::current_dir().map_err(|_| "source-cwd")?) {
        Ok(source) => source,
        Err(reason) => return fallback(reason),
    };
    let lock_key = match source.lock_key() {
        Ok(key) => key,
        Err(reason) => return fallback(reason),
    };
    let preflight = || {
        let env_version = environment(&client.json(&["env", "info", &config.environment])?)?;
        budget(
            &client.json(&["limits", "--all"])?,
            config,
            super::now_ns() / super::SECOND,
        )?;
        Ok::<_, String>(env_version)
    };
    let env_version = match preflight() {
        Ok(version) => version,
        Err(reason) => return fallback(reason),
    };
    let mut warm = warm_cache(&lease.root);
    let from_snapshot = warm
        .as_ref()
        .and_then(|warm| warm.snapshot.as_ref())
        .filter(|saved| saved.name == config.snapshot && saved.lock_key == lock_key)
        .filter(|saved| {
            client
                .json(&["snapshots"])
                .is_ok_and(|list| snapshot_ready(&list, saved).is_some())
        })
        .map(|saved| saved.name.clone());
    let previous = listing(&lease.root)?;
    let pool = if from_snapshot.is_none() {
        warm.as_ref()
            .map(|warm| warm.vm_id.clone())
            .or_else(|| {
                previous
                    .as_ref()
                    .filter(|record| {
                        record.phase == "stopped"
                            && record.credentials_absent
                            && (record.exit_code.is_some()
                                || record.reason.as_deref() == Some("remote-preparation-failed"))
                    })
                    .and_then(|record| record.vm_id.clone())
            })
            .filter(|id| safe_id(id))
            .filter(|id| {
                client.json(&["info", id]).is_ok_and(|info| {
                    let info = sandbox(&info);
                    matches!(info["state"].as_str(), Some("stopped" | "archived"))
                        && info["holdsCreatorLogins"] == false
                        && info["environment"] == config.environment
                })
            })
    } else {
        None
    };
    let run_id = crate::run::random().map_err(|_| "run-id")?;
    let directory = lease.root.join("runs").join(&run_id);
    private_directory(&directory)?;
    let live = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("live.log"))
        .map_err(|_| "boat-live-log")?;
    let mut record = Record {
        schema: 1,
        run_id: run_id.clone(),
        pid: std::process::id(),
        phase: "creating".into(),
        location: "boat".into(),
        vm_id: pool.clone(),
        command_pid: None,
        tree: source.tree.clone(),
        base: source.base.clone(),
        started,
        // Include both bounded admission/create requests before the provider TTL.
        expires_at: Some(super::now_ns() / super::SECOND + config.timeout_seconds + 2 * CONTROL),
        duration_seconds: None,
        exit_code: None,
        usage: None,
        environment_version: Some(env_version),
        reason: None,
        credentials_absent: false,
    };
    lease.save(&record)?;
    let name = format!("eon-build-{run_id}");
    let ttl = config.timeout_seconds.to_string();
    if let Err(reason) = client
        .json(&["limits", "--all"])
        .and_then(|limits| budget(&limits, config, super::now_ns() / super::SECOND))
    {
        record.phase = "stopped".into();
        record.reason = Some(reason.clone());
        lease.save(&record)?;
        return fallback(reason);
    }
    let created = if let Some(id) = pool.as_deref() {
        client.json(&[
            "resume",
            id,
            "--environment",
            &config.environment,
            "--no-env",
            "--type",
            "default",
            "--ttl",
            &ttl,
        ])
    } else {
        let mut args = vec![
            "new",
            "--name",
            &name,
            "--environment",
            &config.environment,
            "--no-env",
            "--type",
            "default",
            "--ttl",
            &ttl,
        ];
        if let Some(snapshot) = &from_snapshot {
            args.extend(["--from", snapshot]);
        }
        client.json(&args)
    };
    let value = match created {
        Ok(value) => value,
        Err(reason) => {
            record.reason = Some(reason);
            let saved = lease.save(&record);
            let cleanup = Client {
                deadline: Instant::now() + Duration::from_secs(CLEANUP),
                cleanup: true,
                reserve_seconds: 0.0,
            };
            let recovered = recover(&cleanup, &lease);
            saved?;
            return fallback(
                recovered
                    .err()
                    .unwrap_or_else(|| "remote-create-failed-reconciled".into()),
            );
        }
    };
    let id = sandbox(&value)["id"]
        .as_str()
        .filter(|id| safe_id(id))
        .map(str::to_owned);
    let Some(id) = id else {
        let cleanup = Client {
            deadline: Instant::now() + Duration::from_secs(CLEANUP),
            cleanup: true,
            reserve_seconds: 0.0,
        };
        let recovered = recover(&cleanup, &lease);
        return fallback(
            recovered
                .err()
                .unwrap_or_else(|| "remote-create-invalid-response-reconciled".into()),
        );
    };
    record.vm_id = Some(id.clone());
    record.phase = "preparing".into();
    let mut released = false;
    let execution = (|| {
        lease.save(&record)?;
        let info = client.json(&["info", &id])?;
        let info = sandbox(&info);
        if info["holdsCreatorLogins"].as_bool() != Some(false)
            || info["environment"].as_str() != Some(&config.environment)
            || info["environmentVersion"].as_u64() != Some(env_version)
        {
            return Err("vm-environment-unsafe".into());
        }
        client
            .success(
                &[
                    "exec",
                    &id,
                    "--",
                    "bash",
                    "--noprofile",
                    "--norc",
                    "-c",
                    include_str!("boat/probe.sh"),
                ],
                Some(&live),
            )
            .map_err(|_| "vm-credential-probe-failed")?;
        record.credentials_absent = true;
        lease.save(&record)?;
        client.success(&["exec", &id, "--", "bash", "-c", "set -eu; rm -rf /home/user/eon /home/user/eon-run; mkdir -p /home/user/eon /home/user/eon-run"], None)?;
        client.success(
            &[
                "push",
                &id,
                source.directory.to_str().ok_or("source-path")?,
                "--to",
                "/home/user/eon",
            ],
            Some(&live),
        )?;
        let jobs = config.jobs.to_string();
        let gate_warm = if nix_gate(argv) { "1" } else { "0" };
        let mut args = vec![
            "exec",
            &id,
            "--detach",
            "--",
            "env",
            "-i",
            "HOME=/home/user",
            "USER=user",
            "LOGNAME=user",
            "PATH=/usr/local/bin:/usr/bin:/bin",
            "bash",
            "--noprofile",
            "--norc",
            "-c",
            include_str!("boat/run.sh"),
            "eon-build",
            &source.tree,
            &source.cwd,
            &jobs,
            &run_id,
            gate_warm,
            include_str!("boat/dependencies.nix"),
        ];
        args.extend(argv.iter().map(String::as_str));
        let command = client.json(&args)?;
        record.command_pid = Some(
            command["processId"]
                .as_u64()
                .ok_or("remote-command-unknown")?,
        );
        record.phase = "running".into();
        lease.save(&record)?;
        released = true;
        client.success(&["exec", &id, "--", "touch", "/home/user/eon-run/go"], None)?;
        let process = record.command_pid.unwrap().to_string();
        let attach = client.success(&["exec", &id, "--attach", &process], Some(&live));
        if attach.is_err() {
            let _ = client.json(&["exec", &id, "--kill", &process]);
            return Err("remote-result-unknown".into());
        }
        Ok::<_, String>(())
    })();
    let cleanup = Client {
        deadline: Instant::now() + Duration::from_secs(CLEANUP),
        cleanup: true,
        reserve_seconds: 0.0,
    };
    if execution.is_err() {
        if let Some(process) = record.command_pid {
            let _ = cleanup.json(&["exec", &id, "--kill", &process.to_string()]);
        }
    }
    let output_path = directory.join("output.log");
    let log_result = (|| {
        cleanup.success(
            &[
                "scp",
                &format!("{id}:/home/user/eon-run/output.log"),
                output_path.to_str().ok_or("log-path")?,
            ],
            None,
        )?;
        fs::set_permissions(&output_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "boat-log-permissions".to_owned())
    })();
    let execution = execution.and_then(|()| {
        log_result?;
        let result_path = directory.join("result.json");
        cleanup.success(
            &[
                "scp",
                &format!("{id}:/home/user/eon-run/result.json"),
                result_path.to_str().ok_or("result-path")?,
            ],
            None,
        )?;
        let result: Value =
            serde_json::from_slice(&fs::read(&result_path).map_err(|_| "remote-result-read")?)
                .map_err(|_| "remote-result-format")?;
        if result["tree"].as_str() != Some(&source.tree)
            || result["run_id"].as_str() != Some(&run_id)
        {
            return Err("remote-result-identity".into());
        }
        if result["gate_ran"] != true || result["source_verified"] != true {
            return Err("remote-preparation-failed".into());
        }
        if result["build_identity_absent"] != true {
            return Err("remote-build-identity-unproven".into());
        }
        if result["log_complete"] != true {
            return Err("remote-log-incomplete".into());
        }
        u8::try_from(result["exit_code"].as_u64().ok_or("remote-result-status")?)
            .map_err(|_| "remote-result-status".into())
    });
    record.exit_code = execution.as_ref().ok().copied();
    record.reason = execution.as_ref().err().cloned();
    record.phase = "stopping".into();
    let mut cacheable = matches!(execution, Ok(0)) && record.credentials_absent;
    if cacheable {
        let prepare = client.json(&["limits", "--all"]);
        if prepare
            .as_ref()
            .is_ok_and(|limits| limits["accessTier"] != "trial")
        {
            // Complete proof is already local; named images retain tools/caches, not old source/logs.
            if client
                .success(
                    &[
                        "exec",
                        &id,
                        "--",
                        "bash",
                        "--noprofile",
                        "--norc",
                        "-c",
                        include_str!("boat/probe.sh"),
                    ],
                    None,
                )
                .and_then(|()| {
                    client.success(
                        &[
                            "exec",
                            &id,
                            "--",
                            "bash",
                            "-c",
                            "set -eu; rm -rf /home/user/eon /home/user/eon-run",
                        ],
                        None,
                    )
                })
                .is_err()
            {
                cacheable = false;
                record.reason = Some("warm-preparation-failed".into());
            }
        }
    }
    let record_saved = lease.save(&record);
    let cleanup = Client {
        deadline: Instant::now() + Duration::from_secs(CLEANUP),
        cleanup: true,
        reserve_seconds: 0.0,
    };
    let stopped = stop_vm(&cleanup, &id);
    record.usage = usage(&cleanup, &id, started);
    let stopped = stopped.and_then(|()| {
        if execution.is_err() {
            delete_vm(&cleanup, &id)
        } else {
            Ok(())
        }
    });
    if stopped.is_ok() {
        record.phase = "stopped".into();
    } else {
        record.phase = "cleanup-unknown".into();
    }
    record.duration_seconds = Some((super::now_ns() / super::SECOND).saturating_sub(started));
    let saved = lease.save(&record);
    record_saved?;
    saved?;
    stopped?;
    if cacheable {
        let mut next = Warm {
            schema: 1,
            vm_id: id.clone(),
            lock_key,
            snapshot: warm.take().and_then(|warm| warm.snapshot),
        };
        if let Err(reason) = if nix_gate(argv) {
            publish_snapshot(&cleanup, config, &mut next)
        } else {
            Ok(())
        } {
            let _ = writeln!(
                io::stderr(),
                "eon agent slot: warm snapshot unavailable ({reason}); retaining stopped pool"
            );
        }
        atomic_write(
            &lease.root.join("boat/warm.json"),
            &serde_json::to_vec(&next).map_err(|_| "warm-cache-format")?,
        )
        .map_err(|_| "warm-cache-write")?;
    }
    let signal = SIGNAL.load(Ordering::Relaxed);
    if signal != 0 {
        return Ok(Outcome::Complete((128 + signal) as u8));
    }
    if !released {
        if let Err(reason) = &execution {
            return fallback(reason.clone());
        }
    }
    Ok(Outcome::Complete(execution?))
}

pub(super) fn allowed(config: &BoatConfig, argv: &[String]) -> bool {
    let shape = if nix_gate(argv) {
        "nix-gate"
    } else if let Some(shape) = dev_command(argv) {
        shape
    } else {
        return false;
    };
    config.allow_shapes.iter().any(|entry| entry == shape)
}

fn dev_command(argv: &[String]) -> Option<&'static str> {
    let [nix, develop, command, program, tail @ ..] = argv else {
        return None;
    };
    if [nix.as_str(), develop, command] != ["nix", "develop", "-c"] {
        return None;
    }
    let (shape, rest) = match (program.as_str(), tail) {
        ("cargo", [verb, rest @ ..]) if verb == "build" => ("cargo-build", rest),
        ("eon-test", rest) if rest.first().is_none_or(|arg| arg != "list") => ("eon-test", rest),
        _ => return None,
    };
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--workspace" | "--locked" | "--offline" if shape == "cargo-build" => (),
            "--release" | "--all-targets" if shape == "cargo-build" => (),
            "--no-capture" | "--no-fail-fast" | "--doc" if shape == "eon-test" => (),
            "-p" => {
                let package = words.next()?;
                if package.is_empty()
                    || !package
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                {
                    return None;
                }
            }
            "--test-threads" if shape == "eon-test" => {
                if !matches!(words.next()?.as_str(), "1" | "2" | "3" | "4") {
                    return None;
                }
            }
            filter
                if shape == "eon-test"
                    && !filter.starts_with('-')
                    && !filter.is_empty()
                    && filter
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
                    && !["live", "visual", "bench", "frame"]
                        .iter()
                        .any(|s| filter.contains(s)) => {}
            _ => return None,
        }
    }
    Some(shape)
}

fn nix_gate(argv: &[String]) -> bool {
    let [program, verb, rest @ ..] = argv else {
        return false;
    };
    if program != "nix" || verb != "build" || rest.is_empty() {
        return false;
    }
    let mut targets = 0;
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            ".#checks.x86_64-linux.eon-tests" | ".#default" | ".#no-helix" | ".#bench" => {
                targets += 1
            }
            "-L" | "--print-build-logs" | "--keep-going" | "--no-link" => (),
            "--max-jobs" => {
                if !words
                    .next()
                    .is_some_and(|n| matches!(n.as_str(), "1" | "2"))
                {
                    return false;
                }
            }
            "--cores" => {
                if !words
                    .next()
                    .is_some_and(|n| matches!(n.as_str(), "1" | "2" | "3" | "4" | "5" | "6"))
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    targets > 0
}

fn timestamp(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.len() < 20 || text.len() > 30 || !text.ends_with('Z') {
        return None;
    }
    let input = std::ffi::CString::new(text).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // Only UTC provider timestamps are accepted; no locale/time-zone configuration.
    let tail = unsafe { libc::strptime(input.as_ptr(), c"%Y-%m-%dT%H:%M:%S".as_ptr(), &mut tm) };
    if tail.is_null() {
        return None;
    }
    let suffix = unsafe { std::ffi::CStr::from_ptr(tail) }.to_str().ok()?;
    if suffix != "Z"
        && !(suffix.starts_with('.')
            && suffix.ends_with('Z')
            && suffix.len() > 2
            && suffix[1..suffix.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let epoch = u64::try_from(unsafe { libc::timegm(&mut tm) }).ok()?;
    let canonical = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    );
    text.starts_with(&canonical).then_some(epoch)
}

fn remaining(limits: &Value) -> Result<f64, String> {
    let missing = || "unsupported-provider-output".to_owned();
    let number = |v: &Value| {
        v.as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(missing)
    };
    let mut seconds = (number(&limits["planIncluded"]["vmHoursRemaining"])? * 3600.0)
        .min(number(&limits["subscriptionRemainingSeconds"])?);
    let trial = match limits["accessTier"].as_str() {
        Some("trial") => true,
        Some(_) if limits["subscriptionStatus"] == "active" => false,
        _ => return Err("unsupported-provider-output".into()),
    };
    if trial {
        let trial = number(&limits["trialComputeCapSeconds"])?
            - number(&limits["trialComputeUsedSeconds"])?;
        seconds = seconds.min(trial);
    }
    Ok(seconds)
}

fn budget(limits: &Value, config: &BoatConfig, now: u64) -> Result<(), String> {
    if limits["canStart"].as_bool() != Some(true)
        || limits["keepGoing"]["enabled"].as_bool() != Some(false)
        || limits["nextStartDollars"].as_f64() != Some(0.0)
        || limits["activeSandboxes"].as_u64() != Some(0)
        || !limits["machineTypes"].as_array().is_some_and(|types| {
            types
                .iter()
                .any(|t| t["type"] == "default" && t["available"] == true)
        })
    {
        return Err("boat-unavailable-or-paid-start".into());
    }
    for window in ["minute", "hour", "day"] {
        if limits["starts"][window]["remaining"]
            .as_u64()
            .is_none_or(|n| n == 0)
        {
            return Err("start-limit".into());
        }
    }
    let required = config.reserve_hours * 3600.0 + config.timeout_seconds as f64 + 60.0;
    if remaining(limits)? < required {
        return Err("budget-reserve-or-trial-cap".into());
    }
    let end = if limits["accessTier"] == "trial" {
        &limits["subscriptionTrialEndsAt"]
    } else {
        &limits["planIncluded"]["periodEnd"]
    };
    if timestamp(end).is_none_or(|end| end.saturating_sub(now) < config.timeout_seconds + 60) {
        return Err("billing-period-boundary".into());
    }
    Ok(())
}

#[cfg(test)]
mod script_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn creation_jsonl_selects_identity_and_never_ignores_an_error_frame() {
        let bytes = b"{\"event\":\"creating\"}\n{\"sandbox\":{\"id\":\"bx_test\"}}\n{\"event\":\"ready\"}\n";
        assert_eq!(sandbox(&decode(0, bytes, "new").unwrap())["id"], "bx_test");
        assert!(
            decode(
                0,
                b"{\"sandbox\":{\"id\":\"bx_test\"}}\n{\"event\":\"error\"}",
                "new"
            )
            .is_err()
        );
    }

    fn limits() -> Value {
        json!({
            "canStart": true, "activeSandboxes": 0,
            "keepGoing": {"enabled": false}, "nextStartDollars": 0,
            "machineTypes": [{"type": "default", "available": true}],
            "starts": {"minute": {"remaining": 3}, "hour": {"remaining": 20}, "day": {"remaining": 74}},
            "planIncluded": {"vmHoursRemaining": 555.0},
            "subscriptionRemainingSeconds": 1998000,
            "accessTier": "trial", "trialComputeCapSeconds": 90000,
            "trialComputeUsedSeconds": 2000,
            "subscriptionTrialEndsAt": "2099-01-01T00:00:00.000Z"
        })
    }

    #[test]
    fn policy_checks_the_entire_command_before_any_provider_call() {
        let config = BoatConfig {
            allow_shapes: vec!["nix-gate".into(), "cargo-build".into(), "eon-test".into()],
            ..BoatConfig::default()
        };
        assert!(allowed(
            &config,
            &words(&[
                "nix",
                "build",
                "-L",
                "--keep-going",
                ".#checks.x86_64-linux.eon-tests",
                ".#default",
                ".#no-helix",
                ".#bench"
            ])
        ));
        for argv in [
            vec!["nix", "profile", "install", ".#default"],
            vec!["bash", "-c", "nix build .#default; eon-bench gate"],
            vec!["nix", "build", ".#default", "--impure"],
            vec!["nix", "build", ".#default", "--out-link", "/private/path"],
            vec!["nix", "build", ".#default", "github:someone/other"],
            vec!["nix", "develop", "-c", "eon-bench", "gate"],
            vec!["nix", "run", ".#bench", "--", "frames"],
            vec!["nix", "build", ".#checks.x86_64-linux.live-visual"],
        ] {
            assert!(!allowed(&config, &words(&argv)), "{argv:?}");
        }
        assert!(allowed(
            &config,
            &words(&[
                "nix",
                "develop",
                "-c",
                "cargo",
                "build",
                "--workspace",
                "--locked"
            ])
        ));
        assert!(allowed(
            &config,
            &words(&["nix", "develop", "-c", "eon-test", "-p", "eon-agent"])
        ));
        for argv in [
            vec!["nix", "develop", "-c", "cargo", "test"],
            vec![
                "nix",
                "develop",
                "-c",
                "eon-test",
                "--",
                "--include-ignored",
            ],
            vec!["nix", "develop", "-c", "eon-test", "live_visual"],
            vec![
                "nix",
                "develop",
                "-c",
                "cargo",
                "build",
                "--manifest-path",
                "/private/Cargo.toml",
            ],
        ] {
            assert!(!allowed(&config, &words(&argv)), "{argv:?}");
        }
        assert!(!allowed(
            &BoatConfig::default(),
            &words(&["nix", "build", ".#default"])
        ));
    }

    #[test]
    fn trial_cap_start_prices_and_missing_safety_fields_refuse_remote() {
        let config = BoatConfig::default();
        let good = limits();
        assert!(budget(&good, &config, 1).is_ok());
        let mut bad = good.clone();
        bad["trialComputeUsedSeconds"] = json!(89999);
        assert!(budget(&bad, &config, 1).is_err());
        for (pointer, value) in [
            ("/keepGoing/enabled", json!(true)),
            ("/nextStartDollars", json!(0.03)),
            ("/starts/minute/remaining", json!(0)),
            ("/subscriptionRemainingSeconds", Value::Null),
            ("/trialComputeUsedSeconds", Value::Null),
            ("/accessTier", Value::Null),
        ] {
            let mut bad = good.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(budget(&bad, &config, 1).is_err(), "{pointer}");
        }
        assert!(timestamp(&json!("2026-02-30T00:00:00.000Z")).is_none());
        let mut boundary = good;
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 60);
        assert!(budget(&boundary, &config, 1).is_ok());
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 59);
        assert!(budget(&boundary, &config, 1).is_err());
    }
}
