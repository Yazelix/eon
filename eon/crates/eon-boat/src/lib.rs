//! Opt-in build execution; the local slot scheduler remains the admission owner.
#![cfg(target_os = "linux")]
mod attach;
mod config;
pub(crate) mod image;
pub(crate) mod local;
mod memory;
pub(crate) mod tree;

pub use config::{BoatConfig, Shape};
use fs2::FileExt;
use image::{Image, Refusal};
use memory::{Cause, Machine, Observation, Report};
use rota::launch::atomic_write;
use rota::slots::private;
use rota::slots::{
    Fallback, Kind, Limit, Listing, Local, Placement, Remote, Request, Route, Signals, Stage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Boat's noun in private-state error tokens (`unsafe-boat-state-directory`, ...).
pub(crate) const NOUN: &str = "boat";
const CONTROL: u64 = 30;
/// Sizes the run deadline (`timeout_seconds - CLEANUP`) and the `timeout` envelope.
const CLEANUP: u64 = 60;
/// The stop step of retirement: the stop request, then a wait of `STOP_WAIT` for it.
const STOP_STEP: u64 = 60;
const STOP_WAIT: u64 = 45;
/// The longest teardown after the deadline: kill, log copy and result copy (one
/// CONTROL each), then stop, usage and delete. The budget guards reserve it; billed time
/// stays bounded by the VM's TTL (`timeout_seconds`), when Boat auto-stops it.
const TEARDOWN: u64 = 3 * CONTROL + STOP_STEP + CONTROL + 2 * CONTROL;
/// Boat's deadline starts before recovery, upload, Nix bootstrap and warmup; a `timeout`
/// envelope must leave this much of it for them (a cold warmup measured about 23 minutes).
const PREPARATION: u64 = 1800;
#[derive(Deserialize, Serialize)]
pub(crate) struct Record {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory: Option<Report>,
    #[serde(default)]
    machine: Machine,
    /// The pinned bootstrap image the run started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    usage: Option<Value>,
    environment_version: Option<u64>,
    reason: Option<String>,
    credentials_absent: bool,
    #[serde(default)]
    kill_sent: bool,
    #[serde(default)]
    trial_seconds_used: Option<u64>,
    #[serde(default)]
    trial_usage_reason: Option<String>,
    #[serde(default)]
    queue_wait_avoided_seconds: Option<u64>,
    #[serde(default)]
    queue_wait_reason: Option<String>,
}

pub(crate) struct Lease {
    _lock: File,
    root: PathBuf,
}

impl Lease {
    pub fn claim(root: &Path) -> Result<Option<Self>, String> {
        let directory = root.join("boat");
        private::directory(&directory, NOUN)?;
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
        if !meta.is_file()
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.mode() & 0o077 != 0
        {
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

pub(crate) fn listing(root: &Path) -> Result<Option<Record>, String> {
    let record: Option<Record> = private::read(&root.join("boat/current.json"), NOUN)?;
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

/// Observe the existing lease inode only: listing must never claim or create it.
pub(crate) fn lease_metadata(root: &Path) -> Result<Option<fs::Metadata>, String> {
    let directory = root.join("boat");
    match fs::symlink_metadata(&directory) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("boat-lease-directory-unreadable".into()),
        Ok(_) => (),
    }
    for path in [root.parent().unwrap_or(root), root, &directory] {
        private::check(path, NOUN)?;
    }
    match fs::symlink_metadata(directory.join("lease.lock")) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Ok(meta)
            if meta.is_file()
                && meta.uid() == rustix::process::geteuid().as_raw()
                && meta.mode() & 0o077 == 0 =>
        {
            Ok(Some(meta))
        }
        _ => Err("unsafe-or-unreadable-boat-lease".into()),
    }
}

struct Client {
    deadline: Instant,
    cleanup: bool,
    reserve_seconds: f64,
    machine: Machine,
}

/// Where a provider call's output goes.
enum Output<'a> {
    /// Keep up to 1 MiB of stdout for the caller; stderr is drained.
    Saved,
    /// Stream both pipes to the live log and rota's stdout/stderr, under `Guard`.
    Log(&'a File, &'a attach::Guard),
    /// `exec --attach` frames: data to the stream, the rest and the CLI's own stderr to
    /// the evidence, under the run's shared `Guard`.
    Attach(
        &'a mut attach::Stream,
        &'a attach::Evidence,
        &'a attach::Guard,
        &'a mut attach::Attempt,
    ),
}

impl Client {
    /// A client for one cleanup step with its own budget. A cancelable step honours a
    /// signal (startup reconciliation); otherwise cleanup runs to its budget.
    fn cleanup(machine: Machine, seconds: u64, cancelable: bool) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(seconds),
            cleanup: !cancelable,
            reserve_seconds: 0.0,
            machine,
        }
    }

    fn canceled(&self) -> bool {
        !self.cleanup && Signals::caught() != 0
    }

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
        output: Output<'_>,
    ) -> Result<(u8, Vec<u8>), String> {
        if Instant::now() >= self.deadline {
            return Err("boat-client-timeout".into());
        }
        let logs = match &output {
            Output::Log(file, _) => Some([
                file.try_clone().map_err(|_| "boat-live-log")?,
                file.try_clone().map_err(|_| "boat-live-log")?,
            ]),
            _ => None,
        };
        let guard = match &output {
            Output::Saved => None,
            Output::Log(_, guard) | Output::Attach(_, _, guard, _) => Some(*guard),
        };
        let attach = matches!(output, Output::Attach(..));
        let mut child = self
            .command(json || attach, args)
            .spawn()
            .map_err(|_| "boat-unreachable")?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        std::thread::scope(|scope| {
            let (stdout, stderr) = match (output, logs) {
                (Output::Attach(stream, evidence, guard, attempt), _) => (
                    scope.spawn(move || {
                        attach::frames(stdout, stream, evidence, guard, attempt);
                        Vec::new()
                    }),
                    scope.spawn(move || {
                        attach::cli_stderr(stderr, evidence, &mut io::stderr());
                        Vec::new()
                    }),
                ),
                (_, Some([out_log, err_log])) => (
                    scope.spawn(move || raw(stdout, Some((out_log, false)), guard)),
                    scope.spawn(move || raw(stderr, Some((err_log, true)), guard)),
                ),
                _ => (
                    scope.spawn(move || raw(stdout, None, None)),
                    scope.spawn(move || {
                        raw(stderr, None, None);
                        Vec::new()
                    }),
                ),
            };
            let deadline = self
                .deadline
                .min(Instant::now() + Duration::from_secs(CONTROL));
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
                // A streamed call runs under its guard; a control call is short and bounded.
                let stop = match guard {
                    Some(guard) => guard.check(self).err(),
                    None if self.canceled() => Some("interrupted".into()),
                    None if Instant::now() >= deadline => Some("boat-client-timeout".into()),
                    None => None,
                };
                if let Some(reason) = stop {
                    break Err(reason);
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            if result.is_err() {
                // This group belongs to our unreaped direct child, not a process-name match.
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_raw(child.id() as i32).unwrap(),
                    rustix::process::Signal::KILL,
                );
                let _ = child.wait();
            }
            let saved = stdout.join().map_err(|_| "boat-client-output")?;
            let _ = stderr.join();
            Ok((result?, saved))
        })
    }

    fn json(&self, args: &[&str]) -> Result<Value, String> {
        let (code, bytes) = self.invoke(true, args, Output::Saved)?;
        decode(code, &bytes, args[0])
    }

    fn success(&self, args: &[&str]) -> Result<(), String> {
        status_ok(self.invoke(false, args, Output::Saved)?.0)
    }

    /// A streamed call to the live log, with a fresh guard of its own.
    fn logged(&self, args: &[&str], live: &File, patience: u64) -> Result<(), String> {
        let guard = attach::Guard::new(patience);
        status_ok(self.invoke(false, args, Output::Log(live, &guard))?.0)
    }
}

fn status_ok(code: u8) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err("boat-command-failed".into())
    }
}

/// Copies one pipe: to a log and rota's matching stream (any output is progress for the
/// guard), or, without a log, keeps up to 1 MiB of it.
fn raw(mut input: impl Read, log: Option<(File, bool)>, guard: Option<&attach::Guard>) -> Vec<u8> {
    let mut saved = Vec::new();
    let mut buffer = [0u8; 16384];
    let mut log = log;
    while let Ok(n) = input.read(&mut buffer) {
        if n == 0 {
            break;
        }
        if let Some(guard) = guard {
            guard.output();
        }
        match &mut log {
            Some((log, stderr)) => {
                let _ = log.write_all(&buffer[..n]);
                if *stderr {
                    let _ = io::stderr().write_all(&buffer[..n]);
                } else {
                    let _ = io::stdout().write_all(&buffer[..n]);
                }
            }
            None if saved.len() < 1024 * 1024 => {
                saved.extend_from_slice(&buffer[..n.min(1024 * 1024 - saved.len())]);
            }
            None => (),
        }
    }
    saved
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
    let value = if matches!(operation, "new" | "info") {
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

/// The pinned image's named row, read from the provider now.
fn pinned_row(client: &Client, image: &Image) -> Result<(), String> {
    let list = client
        .json(&["snapshots"])
        .map_err(|_| Refusal::Unverified("snapshot-unreadable"))?;
    image::verify(&list, image).map_err(Into::into)
}

/// Asks the VM to stop and waits, bounded, for it to report stopped, so its final usage
/// is complete. Deletion follows either way, so a VM that never stops is still retired.
fn stop_vm(client: &Client, id: &str) -> Result<(), String> {
    let started = Instant::now();
    let requested = client.json(&["stop", id]);
    let wait_until = (started + Duration::from_secs(STOP_WAIT)).min(client.deadline);
    while Instant::now() < wait_until && !client.canceled() {
        if let Ok(info) = client.json(&["info", id])
            && matches!(
                sandbox(&info)["state"].as_str(),
                Some("stopped" | "archived")
            )
        {
            return Ok(());
        }
        let next = Instant::now() + Duration::from_secs(1);
        while Instant::now() < next && !client.canceled() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err(requested.err().unwrap_or_else(|| "stop-unconfirmed".into()))
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

/// Stop for final usage, then delete once: no VM outlives its run, so none can be
/// resumed. Each step has its own budget, so a slow stop cannot starve the delete.
fn retire(
    machine: Machine,
    cancelable: bool,
    (id, started): (&str, u64),
    log: Option<&attach::Evidence>,
) -> (Option<Value>, Result<(), String>) {
    let step = |name: &str, seconds: u64, run: &dyn Fn(&Client) -> Result<(), String>| {
        let began = Instant::now();
        let result = run(&Client::cleanup(machine, seconds, cancelable));
        if let Some(log) = log {
            log.control(format!(
                "{name} {} elapsed={}",
                result
                    .as_ref()
                    .err()
                    .map_or("ok".into(), |e| format!("error={e}")),
                began.elapsed().as_secs()
            ));
        }
        result
    };
    let _ = step("stop", STOP_STEP, &|client| stop_vm(client, id));
    let usage = std::cell::RefCell::new(None);
    let _ = step("usage", CONTROL, &|client| {
        *usage.borrow_mut() = self::usage(client, id, started);
        usage
            .borrow()
            .as_ref()
            .map(|_| ())
            .ok_or_else(|| "usage-unavailable".into())
    });
    let deleted = step("delete", 2 * CONTROL, &|client| delete_vm(client, id));
    (usage.into_inner(), deleted)
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
                .is_some_and(|expires| rota::slots::now_ns() / rota::slots::SECOND > expires)
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
    // Startup reconciliation runs under the run client and stays cancelable.
    let (usage, retired) = retire(client.machine, !client.cleanup, (id, old.started), None);
    if usage.is_some() {
        old.usage = usage;
    }
    retired?;
    old.phase = "stopped".into();
    old.reason
        .get_or_insert_with(|| "recovered-after-wrapper-loss".into());
    // `duration_seconds` stays the run's own: reconciliation time is not run time.
    lease.save(&old)
}

pub(crate) enum Outcome {
    Complete(u8, Option<Report>),
    Local(String),
    Uncertain(String, Option<Report>),
}

fn fallback(reason: String) -> Result<Outcome, String> {
    if matches!(
        reason.as_str(),
        "budget-reserve-or-trial-cap"
            | "unverified-or-nonstandard-plan"
            | "billing-period-boundary"
    ) {
        let _ = writeln!(
            io::stderr(),
            "REQUEST: Boat included credit/plan needs owner attention ({reason}); using queued local admission, no billing action"
        );
    }
    let signal = Signals::caught();
    Ok(if signal == 0 {
        Outcome::Local(reason)
    } else {
        Outcome::Complete((128 + signal) as u8, None)
    })
}

pub(crate) fn run(
    lease: Lease,
    config: &BoatConfig,
    argv: &[String],
    shape: Shape,
    memory: (Limit, &'static Image),
    identity: &mut Option<tree::Identity>,
    patience: u64,
) -> Result<Outcome, String> {
    let (limit, image) = memory;
    let machine = image.machine;
    let _signals = Signals::install();
    let wall_started = Instant::now();
    let started = rota::slots::now_ns() / rota::slots::SECOND;
    let client = Client {
        deadline: Instant::now() + Duration::from_secs(config.timeout_seconds - CLEANUP),
        cleanup: false,
        reserve_seconds: config.reserve_hours * 3600.0 * machine.weight(),
        machine,
    };
    if qualification_blocked(&lease.root, machine) {
        return fallback("boat-capacity qualification-stale".into());
    }
    if let Err(reason) = recover(&client, &lease) {
        return fallback(reason);
    }
    let source = match tree::Source::capture(&std::env::current_dir().map_err(|_| "source-cwd")?) {
        Ok(source) => source,
        Err(reason) => return fallback(reason),
    };
    *identity = Some(source.identity());
    // Everything before the receipt exists refuses with nothing to reconcile.
    let preflight = || {
        if config.environment != image.environment {
            return Err(Refusal::Mismatch("environment").into());
        }
        let env_version = environment(&client.json(&["env", "info", &config.environment])?)?;
        if env_version != image.environment_version {
            return Err(Refusal::Mismatch("environment").into());
        }
        budget(
            &client.json(&["limits", "--all"])?,
            config,
            machine,
            rota::slots::now_ns() / rota::slots::SECOND,
        )?;
        pinned_row(&client, image)?;
        Ok::<_, String>(env_version)
    };
    let env_version = match preflight() {
        Ok(version) => version,
        Err(reason) => return fallback(reason),
    };
    let run_id = rota::run::random().map_err(|_| "run-id")?;
    private::directory(&lease.root.join("runs"), NOUN)?;
    let directory = lease.root.join("runs").join(&run_id);
    private::directory(&directory, NOUN)?;
    let live = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("live.log"))
        .map_err(|_| "boat-live-log")?;
    let evidence = attach::Evidence::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("boat.log"))
            .ok(),
    );
    let mut record = Record {
        schema: 1,
        run_id: run_id.clone(),
        pid: std::process::id(),
        phase: "creating".into(),
        location: "boat".into(),
        vm_id: None,
        command_pid: None,
        tree: source.tree.clone(),
        base: source.base.clone(),
        started,
        // Include both bounded admission/create requests before the provider TTL.
        expires_at: Some(
            rota::slots::now_ns() / rota::slots::SECOND + config.timeout_seconds + 2 * CONTROL,
        ),
        duration_seconds: None,
        exit_code: None,
        memory: None,
        machine,
        image: Some(image.name.into()),
        usage: None,
        environment_version: Some(env_version),
        reason: None,
        credentials_absent: false,
        kill_sent: false,
        trial_seconds_used: None,
        trial_usage_reason: Some("unavailable".into()),
        queue_wait_avoided_seconds: None,
        queue_wait_reason: Some("deferred-to-child26".into()),
    };
    lease.save(&record)?;
    let name = format!("eon-build-{run_id}");
    let ttl = config.timeout_seconds.to_string();
    let allowance_before = client.json(&["limits", "--all"]);
    if let Err(reason) = allowance_before
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|limits| {
            budget(
                limits,
                config,
                machine,
                rota::slots::now_ns() / rota::slots::SECOND,
            )
        })
    {
        record.phase = "stopped".into();
        record.reason = Some(reason.clone());
        lease.save(&record)?;
        return fallback(reason);
    }
    // Every run starts fresh from the pinned image; captures stay off because the VM is
    // deleted after its run.
    let created = client.json(&[
        "new",
        "--name",
        &name,
        "--environment",
        &config.environment,
        "--no-env",
        "--type",
        machine.as_str(),
        "--ttl",
        &ttl,
        "--from",
        image.name,
        "--no-snapshots",
    ]);
    let value = match created {
        Ok(value) => value,
        Err(reason) => {
            record.reason = Some(reason);
            let saved = lease.save(&record);
            let recovered = recover(&Client::cleanup(machine, CLEANUP, false), &lease);
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
        let recovered = recover(&Client::cleanup(machine, CLEANUP, false), &lease);
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
        // Nothing but these reads and the marker check runs before the image is verified.
        let info = client
            .json(&["info", &id])
            .map_err(|_| Refusal::Unverified("vm-state"))?;
        let info = sandbox(&info);
        if !matches!(info["state"].as_str(), Some("ready" | "idle" | "running")) {
            return Err(Refusal::Unverified("vm-state").into());
        }
        // A field Boat leaves out or mistypes is unreadable; only a readable other value
        // is a different image.
        let (Some(kind), Some(environment), Some(version)) = (
            info["type"].as_str(),
            info["environment"].as_str(),
            info["environmentVersion"].as_u64(),
        ) else {
            return Err(Refusal::Unverified("vm-identity").into());
        };
        if (kind, environment, version)
            != (
                machine.as_str(),
                image.environment,
                image.environment_version,
            )
        {
            return Err(Refusal::Mismatch("vm-identity").into());
        }
        if info["holdsCreatorLogins"].as_bool() != Some(false) {
            return Err("vm-environment-unsafe".into());
        }
        // Boat reports no origin snapshot for a clone: the row before and after the start
        // and the guest's root-owned recipe marker are the identity evidence.
        pinned_row(&client, image)?;
        image::marker_status(
            client
                .invoke(
                    false,
                    &[
                        "exec",
                        &id,
                        "--",
                        "env",
                        "-i",
                        "PATH=/usr/bin:/bin",
                        "LC_ALL=C",
                        "bash",
                        "--noprofile",
                        "--norc",
                        "-c",
                        image::MARKER,
                        "eon-image",
                        image.recipe_sha256,
                    ],
                    Output::Saved,
                )
                .map(|(code, _)| code),
        )?;
        client
            .logged(
                &[
                    "exec",
                    &id,
                    "--",
                    "bash",
                    "--noprofile",
                    "--norc",
                    "-c",
                    include_str!("probe.sh"),
                ],
                &live,
                patience,
            )
            .map_err(|_| "vm-credential-probe-failed")?;
        record.credentials_absent = true;
        lease.save(&record)?;
        client.success(&["exec", &id, "--", "bash", "-c", "set -eu; rm -rf /home/user/eon /home/user/eon-run; mkdir -p /home/user/eon /home/user/eon-run"])?;
        client.logged(
            &[
                "push",
                &id,
                source.directory.to_str().ok_or("source-path")?,
                "--to",
                "/home/user/eon",
            ],
            &live,
            patience,
        )?;
        let jobs = config.jobs.to_string();
        let gate_warm = if shape == Shape::NixGate { "1" } else { "0" };
        let cap = limit.bytes().to_string();
        let high = limit
            .high()
            .map(|bytes| bytes.to_string())
            .unwrap_or_default();
        let margin = machine.margin().to_string();
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
            concat!(include_str!("scope.sh"), "\n", include_str!("run.sh")),
            "eon-build",
            &source.tree,
            &source.cwd,
            &jobs,
            &run_id,
            gate_warm,
            include_str!("dependencies.nix"),
            &cap,
            &high,
            &margin,
            include_str!("read-kernel.py"),
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
        client.success(&["exec", &id, "--", "touch", "/home/user/eon-run/go"])?;
        let process = record.command_pid.unwrap().to_string();
        let mut stream = attach::Stream::new(
            attach::Sink::new(Box::new(live.try_clone().map_err(|_| "boat-live-log")?)),
            attach::Sink::new(Box::new(io::stdout())),
            attach::Sink::new(Box::new(io::stderr())),
        );
        attach::follow(
            &client,
            &attach::Guard::new(patience),
            (&id, &process),
            &mut stream,
            &evidence,
            &directory.join("result.json"),
        )
    })();
    // Run time ends with execution; teardown and any later reconciliation are not part of it.
    let ran = wall_started.elapsed().as_secs();
    if let Err(reason) = &execution {
        evidence.control(format!("give-up {reason}"));
    }
    // Each step below has its own budget, so a slow kill cannot cut a copy short.
    let step = |name: &str, args: &[&str]| {
        let result = Client::cleanup(machine, CONTROL, false)
            .invoke(false, args, Output::Saved)
            .and_then(|(code, _)| status_ok(code));
        evidence.control(format!(
            "{name} {}",
            result
                .as_ref()
                .err()
                .map_or("ok".into(), |e| format!("error={e}"))
        ));
        result
    };
    if execution.is_err()
        && let Some(process) = record.command_pid
    {
        record.kill_sent = true;
        if lease.save(&record).is_ok() {
            let _ = step("kill", &["exec", &id, "--kill", &process.to_string()]);
        }
    }
    let output_path = directory.join("output.log");
    // A VM refused before its workload started is never read from, only retired.
    let log_result = (|| {
        if !released {
            return Err("remote-log-not-started".to_owned());
        }
        step(
            "copy output.log",
            &[
                "scp",
                &format!("{id}:/home/user/eon-run/output.log"),
                output_path.to_str().ok_or("log-path")?,
            ],
        )?;
        fs::set_permissions(&output_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "boat-log-permissions".to_owned())
    })();
    let execution = execution.and_then(|()| {
        let result_path = directory.join("result.json");
        step(
            "copy result.json",
            &[
                "scp",
                &format!("{id}:/home/user/eon-run/result.json"),
                result_path.to_str().ok_or("result-path")?,
            ],
        )?;
        let result: Value =
            serde_json::from_slice(&fs::read(&result_path).map_err(|_| "remote-result-read")?)
                .map_err(|_| "remote-result-format")?;
        if result["tree"].as_str() != Some(&source.tree)
            || result["run_id"].as_str() != Some(&run_id)
        {
            return Err("remote-result-identity".into());
        }
        let complete_log = || {
            if result["log_complete"] != true {
                return Err("remote-log-incomplete".into());
            }
            log_result.as_ref().map(|_| ()).map_err(Clone::clone)
        };
        let code = u8::try_from(result["exit_code"].as_u64().ok_or("remote-result-status")?)
            .map_err(|_| "remote-result-status".to_owned())?;
        let prepared = result["preparation_started"].as_bool().ok_or("remote-memory-unverified")?;
        if !prepared && code == 96 && result["gate_ran"] == false && result["source_verified"] == false {
            complete_log()?;
            let reason = if let Some(headroom) = result["headroom_failed"].as_object() {
                let amount = |key: &str| headroom.get(key).and_then(Value::as_u64);
                let total = amount("total").ok_or("remote-memory-unverified")?;
                let available = amount("available").ok_or("remote-memory-unverified")?;
                let ceiling = amount("ceiling").ok_or("remote-memory-unverified")?;
                if total == 0 || available > total || ceiling >= limit.bytes()
                    || ceiling > available.saturating_sub(machine.margin())
                {
                    return Err("remote-memory-unverified".into());
                }
                "remote-memory-headroom"
            } else {
                "remote-memory-unavailable"
            };
            let hold = serde_json::json!({"run_id":run_id, "limit":limit, "machine":machine,
                "environment_version":env_version, "reason":reason, "evidence":result["headroom_failed"]});
            atomic_write(&qualification_path(&lease.root, machine), &serde_json::to_vec(&hold).map_err(|_| "boat-qualification-hold")?)
                .map_err(|_| "boat-qualification-hold")?;
            return Err(reason.into());
        }
        let observation: Observation = serde_json::from_value(result["memory"].clone())
            .map_err(|_| "remote-memory-unverified")?;
        let report = observation.report(limit, machine, result["gate_ran"] == false)?;
        let cause = report.cause;
        record.memory = Some(report);
        match cause {
            Cause::JobLimit if prepared => {
                record.exit_code = Some(137);
                complete_log()?;
                let _ = writeln!(io::stderr(), "{}", limit.oom_message());
                return Ok(137);
            }
            Cause::VmWide => {
                let _ = writeln!(io::stderr(), "Boat VM out of memory (not the job's cap)");
                return Err("vm-wide-oom".into());
            }
            Cause::Unknown => return Err("oom-cause-unknown".into()),
            Cause::JobLimit => return Err("remote-memory-unverified".into()),
            Cause::None => (),
        }
        if !prepared || result["gate_ran"] != true || result["source_verified"] != true {
            return Err("remote-preparation-failed".into());
        }
        if result["build_identity_absent"] != true {
            return Err("remote-build-identity-unproven".into());
        }
        record.exit_code = Some(code);
        complete_log()?;
        Ok(code)
    });
    record.exit_code = execution.as_ref().ok().copied().or(record.exit_code);
    record.reason = execution.as_ref().err().cloned().or_else(|| {
        record
            .memory
            .as_ref()
            .filter(|report| report.cause == Cause::JobLimit)
            .map(|_| "job-limit-oom".into())
    });
    record.phase = "stopping".into();
    record.duration_seconds = Some(ran);
    let record_saved = lease.save(&record);
    let (usage, retired) = retire(machine, false, (&id, started), Some(&evidence));
    record.usage = usage;
    if let (Ok(before), Ok(after)) = (
        allowance_before.as_ref(),
        Client::cleanup(machine, CONTROL, false).json(&["limits", "--all"]),
    ) {
        if billing_policy(before) == Ok(true) && billing_policy(&after) == Ok(true) {
            record.trial_seconds_used = before["trialComputeUsedSeconds"]
                .as_u64()
                .zip(after["trialComputeUsedSeconds"].as_u64())
                .and_then(|(a, b)| b.checked_sub(a));
            record.trial_usage_reason = Some(
                if record.trial_seconds_used.is_some() {
                    "account-counter-delta"
                } else {
                    "counter-reset-or-unavailable"
                }
                .into(),
            );
        } else if billing_policy(before) == Ok(false) && billing_policy(&after) == Ok(false) {
            record.trial_seconds_used = Some(0);
            record.trial_usage_reason = Some("standard-plan-run".into());
        }
    }
    match &retired {
        Ok(()) => record.phase = "stopped".into(),
        Err(cleanup) => {
            // Unknown cleanup keeps the run's own reason and says why cleanup is unknown.
            record.phase = "cleanup-unknown".into();
            record.reason = Some(match record.reason.take() {
                Some(run) => format!("{run}; cleanup {cleanup}"),
                None => cleanup.clone(),
            });
        }
    }
    let saved = lease.save(&record);
    record_saved?;
    saved?;
    // Unknown cleanup exits 125 before a signal status or any local fallback (EON-C43).
    if retired.is_err() {
        let reason = record.reason.clone().unwrap_or_default();
        return match record.memory {
            Some(memory) => Ok(Outcome::Uncertain(reason, Some(memory))),
            None => Err(reason),
        };
    }
    let signal = Signals::caught();
    if signal != 0 {
        return Ok(Outcome::Complete((128 + signal) as u8, None));
    }
    if let Err(reason) = &execution
        && (!released
            || matches!(
                reason.as_str(),
                "remote-memory-headroom" | "remote-memory-unavailable"
            ))
    {
        return fallback(reason.clone());
    }
    match execution {
        Ok(code) => Ok(Outcome::Complete(code, record.memory)),
        Err(reason) if record.memory.is_some() => Ok(Outcome::Uncertain(reason, record.memory)),
        Err(reason) => Err(reason),
    }
}

/// The Boat shape `argv` is permitted as, or `None` when it must run locally. One
/// `timeout DURATION` envelope may wrap a shape; the original argv still runs as given.
pub(crate) fn shape(config: &BoatConfig, argv: &[String]) -> Option<Shape> {
    let command = match argv {
        [program, duration, command @ ..] if program == "timeout" => {
            // Boat's deadline also covers preparation, so the envelope must fit inside it.
            let limit = config.timeout_seconds.saturating_sub(CLEANUP + PREPARATION);
            (!command.is_empty() && seconds(duration)? <= limit).then_some(command)?
        }
        _ => argv,
    };
    let shape = if nix_gate(command) {
        Shape::NixGate
    } else {
        dev_command(command)?
    };
    config.allow_shapes.contains(&shape).then_some(shape)
}

/// A `timeout` duration: a positive integer, without leading zeros, of seconds, minutes or
/// hours. Options, fractions, other units and `infinity` are not durations here.
fn seconds(duration: &str) -> Option<u64> {
    let (unit, digits) = duration.as_bytes().split_last()?;
    let scale = match unit {
        b's' => 1,
        b'm' => 60,
        b'h' => 3600,
        _ => return None,
    };
    if !(1..=6).contains(&digits.len())
        || digits[0] == b'0'
        || !digits.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    std::str::from_utf8(digits)
        .ok()?
        .parse::<u64>()
        .ok()?
        .checked_mul(scale)
}

fn dev_command(argv: &[String]) -> Option<Shape> {
    let [nix, develop, command, program, tail @ ..] = argv else {
        return None;
    };
    if [nix.as_str(), develop, command] != ["nix", "develop", "-c"] {
        return None;
    }
    let (shape, rest) = match (program.as_str(), tail) {
        ("cargo", [verb, rest @ ..]) if verb == "build" => (Shape::CargoBuild, rest),
        ("cargo", [verb, rest @ ..]) if verb == "clippy" => {
            // Only the landing lint level may follow `--`.
            match rest.iter().position(|word| word == "--") {
                Some(at) if rest[at + 1..] == ["-D", "warnings"] => {
                    (Shape::CargoClippy, &rest[..at])
                }
                Some(_) => return None,
                None => (Shape::CargoClippy, rest),
            }
        }
        ("eon-test", rest) if rest.first().is_none_or(|arg| arg != "list") => {
            (Shape::EonTest, rest)
        }
        _ => return None,
    };
    let cargo = matches!(shape, Shape::CargoBuild | Shape::CargoClippy);
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--workspace" | "--locked" | "--offline" if cargo => (),
            "--release" | "--all-targets" if cargo => (),
            "--no-capture" | "--no-fail-fast" | "--doc" if shape == Shape::EonTest => (),
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
            "--test-threads" if shape == Shape::EonTest => {
                if !matches!(words.next()?.as_str(), "1" | "2" | "3" | "4") {
                    return None;
                }
            }
            filter
                if shape == Shape::EonTest
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
    let bytes = text.as_bytes();
    if !bytes[..19].iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        10 => *b == b'T',
        13 | 16 => *b == b':',
        _ => b.is_ascii_digit(),
    }) {
        return None;
    }
    let suffix = &text[19..];
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
    let field = |range: std::ops::Range<usize>| text[range].parse::<u64>().ok();
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let months = [
        31,
        28 + u64::from(leap),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1970
        || !(1..=12).contains(&month)
        || day == 0
        || day > months[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let leaps = |year: u64| year / 4 - year / 100 + year / 400;
    let days = (year - 1970) * 365 + leaps(year - 1) - leaps(1969)
        + months[..(month - 1) as usize].iter().sum::<u64>()
        + day
        - 1;
    Some(((days * 24 + hour) * 60 + minute) * 60 + second)
}

fn billing_policy(limits: &Value) -> Result<bool, String> {
    match (
        limits["accessTier"].as_str(),
        limits["subscriptionStatus"].as_str(),
        limits["sandboxPlanKey"].as_str(),
    ) {
        (Some("trial"), Some("trialing"), Some("trial")) => Ok(true),
        (Some("paid" | "standard"), Some("active"), Some("box_20"))
            if limits["activeSandboxPlanKey"] == "box_20" =>
        {
            Ok(false)
        }
        _ => Err("unverified-or-nonstandard-plan".into()),
    }
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
    let trial = billing_policy(limits)?;
    if trial {
        let trial = number(&limits["trialComputeCapSeconds"])?
            - number(&limits["trialComputeUsedSeconds"])?;
        seconds = seconds.min(trial);
    }
    Ok(seconds)
}

fn type_available(limits: &Value, machine: Machine) -> bool {
    limits["machineTypes"].as_array().is_some_and(|types| {
        let mut matches = types.iter().filter(|t| t["type"] == machine.as_str());
        matches.next().is_some_and(|t| t["available"] == true) && matches.next().is_none()
    })
}

pub(crate) fn available(machine: Machine) -> bool {
    let client = Client {
        deadline: Instant::now() + Duration::from_secs(CONTROL),
        cleanup: false,
        reserve_seconds: 0.0,
        machine,
    };
    client
        .json(&["limits", "--all"])
        .is_ok_and(|limits| type_available(&limits, machine))
}

fn qualification_path(root: &Path, machine: Machine) -> PathBuf {
    root.join("boat")
        .join(format!("{}_qualification_hold.json", machine.as_str()))
}

pub(crate) fn qualification_blocked(root: &Path, machine: Machine) -> bool {
    !fs::symlink_metadata(qualification_path(root, machine))
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
}

fn budget(limits: &Value, config: &BoatConfig, machine: Machine, now: u64) -> Result<(), String> {
    if limits["canStart"].as_bool() != Some(true)
        || limits["keepGoing"]["enabled"].as_bool() != Some(false)
        || limits["nextStartDollars"].as_f64() != Some(0.0)
        || limits["activeSandboxes"].as_u64() != Some(0)
        || !type_available(limits, machine)
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
    let required = machine.weight()
        * (config.reserve_hours * 3600.0 + config.timeout_seconds as f64 + TEARDOWN as f64);
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

/// Eon's Boat route: permitted build shapes run on a Boat VM, everything else stays local.
pub struct Boat {
    /// Seconds of output silence on the first, unbroken attach, and of unreadable
    /// process status after the stream was lost, before the run is killed.
    patience: u64,
}

impl Boat {
    /// Eon's route.
    pub const EON: Boat = Boat { patience: 600 };

    /// A route with another patience, for test hosts that cannot wait ten minutes.
    pub const fn with_patience(seconds: u64) -> Self {
        Self { patience: seconds }
    }
}

/// Boat runs only when its switch file exists (in the legacy queue directory when set).
fn enabled(request: &Request<'_>) -> bool {
    request
        .config
        .legacy_dir
        .as_deref()
        .unwrap_or(request.root)
        .join("boat-on")
        .try_exists()
        .unwrap_or(false)
}

fn local(reason: impl Into<String>, kind: Kind) -> Placement {
    Placement::Local(Some(Fallback {
        reason: reason.into(),
        kind,
    }))
}

/// How the queue treats a reason a Boat run gave back: the policy reasons are expected
/// and exit 1 on failure; anything else is Boat failing to run the command (exit 125).
fn returned_kind(reason: &str) -> Kind {
    match reason {
        "policy-local" => Kind::Declined,
        "boat-not-enabled" => Kind::Off,
        "timing-local" => Kind::Kept,
        _ if reason.starts_with("boat-capacity ") => Kind::Kept,
        _ => Kind::Returned,
    }
}

fn identity(source: Option<&Value>) -> Result<Option<tree::Identity>, String> {
    source
        .map(tree::Identity::deserialize)
        .transpose()
        .map_err(|_| "boat-source-format".into())
}

impl Route for Boat {
    fn name(&self) -> &'static str {
        "boat"
    }

    fn check_config(&self, text: &str) -> Result<Vec<String>, String> {
        let config = BoatConfig::from_text(text)?;
        Ok(config.tombstone().into_iter().map(Into::into).collect())
    }

    fn place(&self, request: &Request<'_>, stage: Stage) -> Placement {
        // Unreadable here only for a caller that skipped `check_config`: fail closed, and
        // let `run_remote` refuse the command with the reason.
        let Ok(config) = BoatConfig::from_table(&request.config.boat) else {
            return Placement::Remote;
        };
        let config = &config;
        if config.allow_shapes.is_empty() {
            return Placement::Local(None);
        }
        if stage == Stage::Queued {
            if shape(config, request.argv).is_none() {
                return local("policy-local", Kind::Declined);
            }
            if request.quiet_exempt {
                return local("timing-local", Kind::Kept);
            }
        }
        if !enabled(request) {
            return local("boat-not-enabled", Kind::Off);
        }
        let limit = request.limit;
        let capacity = |reason: &str| {
            local(
                format!(
                    "boat-capacity class={} cap={}G {reason}",
                    limit.class.as_str(),
                    limit.cap_gib
                ),
                Kind::Kept,
            )
        };
        // Asking Boat whether a machine is free costs a provider call: only when the
        // command queues or its memory limit (and so its machine) changed.
        let check_machine = match stage {
            Stage::Queued => true,
            Stage::Waiting { limit_changed } => limit_changed,
        };
        match image::select(limit, config) {
            Err(reason) => capacity(reason),
            Ok(image) if qualification_blocked(request.root, image.machine) => {
                capacity("qualification-stale")
            }
            Ok(image) if check_machine && !available(image.machine) => {
                capacity("machine-unavailable")
            }
            Ok(_) => Placement::Remote,
        }
    }

    fn run_remote(
        &self,
        request: &Request<'_>,
        granted: &mut dyn FnMut(),
        source: &mut Option<Value>,
    ) -> Remote {
        let Ok(config) = BoatConfig::from_table(&request.config.boat) else {
            return Remote::Failed {
                reason: "boat-config-invalid".into(),
                memory: None,
            };
        };
        let config = &config;
        let mut prepared = match identity(source.as_ref()) {
            Ok(prepared) => prepared,
            Err(reason) => {
                return Remote::Failed {
                    reason,
                    memory: None,
                };
            }
        };
        let Some(shape) = shape(config, request.argv) else {
            return Remote::Wait;
        };
        let Ok(image) = image::select(request.limit, config) else {
            return Remote::Wait;
        };
        if qualification_blocked(request.root, image.machine) || !enabled(request) {
            return Remote::Wait;
        }
        let lease = match Lease::claim(request.root) {
            Ok(Some(lease)) => lease,
            Ok(None) => return Remote::Wait,
            Err(reason) => {
                return Remote::Failed {
                    reason,
                    memory: None,
                };
            }
        };
        granted();
        let memory = |report: Option<Report>| report.and_then(|r| serde_json::to_value(r).ok());
        match run(
            lease,
            config,
            request.argv,
            shape,
            (request.limit, image),
            &mut prepared,
            self.patience,
        ) {
            Ok(Outcome::Complete(code, report)) => Remote::Exit {
                code,
                memory: memory(report),
            },
            Ok(Outcome::Uncertain(reason, report)) => Remote::Failed {
                reason,
                memory: memory(report),
            },
            Err(reason) => Remote::Failed {
                reason,
                memory: None,
            },
            Ok(Outcome::Local(reason)) => {
                match prepared.map(serde_json::to_value).transpose() {
                    Ok(value) => *source = value,
                    Err(_) => {
                        return Remote::Failed {
                            reason: "boat-source-format".into(),
                            memory: None,
                        };
                    }
                }
                Remote::Local(Fallback {
                    kind: returned_kind(&reason),
                    reason,
                })
            }
        }
    }

    fn run_local(&self, run: Local<'_>) -> io::Result<std::process::ExitStatus> {
        // Decode before `local::run` may return early for a terminal: a bad source never runs.
        let prepared = identity(run.source).map_err(io::Error::other)?;
        match run.fallback {
            Some(fallback) if fallback.kind.routed() => local::run(
                run.root,
                run.command,
                &fallback.reason,
                prepared.as_ref().map(tree::Identity::tree),
                run.program,
            ),
            _ => local::status(run.command),
        }
    }

    fn verify(&self, source: &Value) -> Result<(), String> {
        identity(Some(source))?
            .ok_or_else(|| "boat-source-format".to_owned())?
            .verify()
    }

    fn listing(
        &self,
        root: &Path,
        lock_owner: &dyn Fn(&fs::Metadata) -> Option<Option<u32>>,
    ) -> Result<Listing, String> {
        let (record, mut error) = match listing(root) {
            Ok(record) => (record, None),
            Err(reason) => (None, Some(reason)),
        };
        let (lease_held, lease_pid) = match lease_metadata(root) {
            Ok(metadata) => {
                let owner = metadata.as_ref().and_then(lock_owner);
                (Some(owner.is_some()), owner.flatten())
            }
            Err(reason) => {
                error.get_or_insert(reason);
                (None, None)
            }
        };
        let mut out = Listing::default();
        if lease_held == Some(true) {
            out.lines
                .push("boat lease held (including preflight)".into());
        }
        if let Some(reason) = &error {
            out.lines.push(format!("boat record unavailable: {reason}"));
        }
        let format = |_| "boat-record-format".to_owned();
        if let Some(active) = record.as_ref().filter(|record| record.phase != "stopped") {
            out.lines.push(format!(
                "boat held pid={} vm={} phase={} run={}",
                active.pid,
                active.vm_id.as_deref().unwrap_or("unknown"),
                active.phase,
                active.run_id
            ));
            out.holders
                .push(serde_json::to_value(active).map_err(format)?);
            out.busy.push(active.pid);
        }
        out.busy.extend(lease_pid);
        out.fields
            .insert("boat_lease_held".into(), lease_held.into());
        if let Some(pid) = lease_pid {
            out.fields.insert("boat_lease_pid".into(), pid.into());
        }
        if let Some(record) = record {
            out.fields
                .insert("boat".into(), serde_json::to_value(record).map_err(format)?);
        }
        if let Some(reason) = error {
            out.fields.insert("boat_error".into(), reason.into());
        }
        Ok(out)
    }
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

    fn allowed(config: &BoatConfig, argv: &[String]) -> bool {
        shape(config, argv).is_some()
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
            "subscriptionStatus": "trialing", "sandboxPlanKey": "trial",
            "trialComputeUsedSeconds": 2000,
            "subscriptionTrialEndsAt": "2099-01-01T00:00:00.000Z"
        })
    }

    #[test]
    fn policy_checks_the_entire_command_before_any_provider_call() {
        let config = BoatConfig {
            allow_shapes: vec![Shape::NixGate, Shape::CargoBuild, Shape::EonTest],
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
            &words(&["nix", "develop", "-c", "eon-test", "-p", "rota-cli"])
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
    fn landing_clippy_and_timeout_envelopes_qualify_only_within_boat_budget() {
        let config = |timeout_seconds| BoatConfig {
            allow_shapes: vec![
                Shape::NixGate,
                Shape::CargoBuild,
                Shape::CargoClippy,
                Shape::EonTest,
            ],
            timeout_seconds,
            ..BoatConfig::default()
        };
        let remote =
            |timeout_seconds, argv: &[&str]| allowed(&config(timeout_seconds), &words(argv));
        let gate = |duration| {
            vec![
                "timeout",
                duration,
                "nix",
                "build",
                "-L",
                "--keep-going",
                ".#default",
                ".#bench",
                ".#no-helix",
                "--max-jobs",
                "2",
                "--cores",
                "6",
                "--no-link",
            ]
        };
        let clippy = [
            "nix",
            "develop",
            "-c",
            "cargo",
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ];
        assert!(remote(7200, &clippy));
        assert!(remote(
            7200,
            &["nix", "develop", "-c", "cargo", "clippy", "-p", "rota-cli"]
        ));
        assert!(!allowed(
            &BoatConfig {
                allow_shapes: vec![Shape::CargoBuild],
                ..BoatConfig::default()
            },
            &words(&clippy)
        ));
        for duration in ["85m", "89m", "5340s", "1h"] {
            let argv = words(&gate(duration));
            assert_eq!(
                shape(&config(7200), &argv),
                Some(Shape::NixGate),
                "{duration}"
            );
        }
        assert!(remote(
            7200,
            &[
                "timeout", "30s", "nix", "develop", "-c", "eon-test", "-p", "rota-cli"
            ]
        ));
        assert!(remote(3600, &gate("29m")));
        for (limit, duration) in [
            (7200, "90m"),
            (7200, "5341s"),
            (7200, "120m"),
            (3600, "30m"),
            (1860, "1s"),
        ] {
            assert!(!remote(limit, &gate(duration)), "{limit} {duration}");
        }
        for duration in [
            "0", "0m", "012m", "1.5h", "2d", "120", "inf", "infinity", "9999999h", "m", "1M",
            "1m\u{e9}", "\u{e9}",
        ] {
            assert!(!remote(7200, &gate(duration)), "{duration:?}");
        }
        let landing_out_link = [
            "nix",
            "build",
            "-L",
            "--keep-going",
            ".#default",
            "--max-jobs",
            "2",
            "--cores",
            "6",
            "--out-link",
            "/host/lead/land-x-built",
        ];
        let mut wrapped_out_link = vec!["timeout", "85m"];
        wrapped_out_link.extend(landing_out_link);
        for argv in [
            &landing_out_link[..],
            &wrapped_out_link,
            &["timeout", "--", "1m", "nix", "build", ".#default"],
            &["timeout", "-s", "KILL", "1m", "nix", "build", ".#default"],
            &["timeout", "-k", "5", "1m", "nix", "build", ".#default"],
            &["timeout", "--foreground", "1m", "nix", "build", ".#default"],
            &[
                "timeout",
                "--preserve-status",
                "1m",
                "nix",
                "build",
                ".#default",
            ],
            &[
                "timeout",
                "1m",
                "timeout",
                "1m",
                "nix",
                "build",
                ".#default",
            ],
            &["timeout", "1m"],
            &["timeout", "1m", "bash", "-c", "nix build .#default"],
            &[
                "timeout",
                "1m",
                "env",
                "CARGO_TARGET_DIR=/x",
                "nix",
                "develop",
                "-c",
                "cargo",
                "build",
            ],
            &["env", "X=1", "nix", "build", ".#default"],
            &["nix", "develop", "-c", "cargo", "clippy", "--fix"],
            &[
                "nix",
                "develop",
                "-c",
                "cargo",
                "clippy",
                "--manifest-path",
                "/x/Cargo.toml",
            ],
            &[
                "nix", "develop", "-c", "cargo", "clippy", "--", "-A", "warnings",
            ],
            &[
                "nix",
                "develop",
                "-c",
                "cargo",
                "clippy",
                "--",
                "-D",
                "warnings",
                "--cap-lints",
                "allow",
            ],
            &[
                "nix",
                "develop",
                "-c",
                "cargo",
                "clippy",
                "--",
                "-D",
                "warnings",
                "-D",
                "clippy::all",
            ],
            &["nix", "develop", "-c", "cargo", "clippy", "--"],
        ] {
            assert!(!remote(7200, argv), "{argv:?}");
        }
    }

    #[test]
    fn utc_timestamps_preserve_calendar_and_fraction_rules() {
        for (text, seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("1970-01-01T00:00:00.123Z", 0),
            ("2000-02-29T00:00:00Z", 951_782_400),
            ("2100-03-01T00:00:00Z", 4_107_542_400),
            ("9999-12-31T23:59:59Z", 253_402_300_799),
        ] {
            assert_eq!(timestamp(&json!(text)), Some(seconds), "{text}");
        }
        for text in [
            "1900-02-29T00:00:00Z",
            "2100-02-29T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-01-01T00:00:60Z",
            "1969-12-31T23:59:59Z",
            "2026-1-01T00:00:00Z",
            "2026-01-01T00:00:00.Z",
            " 2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00+00:00",
            "2026-01-01T00:00:00.1234567890Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-00-01T00:00:00Z",
            "2026-01-00T00:00:00Z",
            "2026-01-01T00:00:00.１２Z",
        ] {
            assert_eq!(timestamp(&json!(text)), None, "{text}");
        }
    }

    #[test]
    fn trial_cap_start_prices_and_missing_safety_fields_refuse_remote() {
        let config = BoatConfig::default();
        let good = limits();
        assert!(budget(&good, &config, Machine::Default, 1).is_ok());
        let mut bad = good.clone();
        bad["trialComputeUsedSeconds"] = json!(89999);
        assert!(budget(&bad, &config, Machine::Default, 1).is_err());
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
            assert!(
                budget(&bad, &config, Machine::Default, 1).is_err(),
                "{pointer}"
            );
        }
        assert!(timestamp(&json!("2026-02-30T00:00:00.000Z")).is_none());
        let mut boundary = good;
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 240);
        assert!(budget(&boundary, &config, Machine::Default, 1).is_ok());
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 239);
        assert!(budget(&boundary, &config, Machine::Default, 1).is_err());
    }

    #[test]
    fn only_verified_standard_plan_can_spend_included_paid_credit() {
        let mut paid = limits();
        paid["accessTier"] = json!("paid");
        paid["subscriptionStatus"] = json!("active");
        paid["sandboxPlanKey"] = json!("box_20");
        paid["activeSandboxPlanKey"] = json!("box_20");
        assert!(remaining(&paid).is_ok());
        paid["sandboxPlanKey"] = json!("box_100");
        assert!(remaining(&paid).is_err());
        let mut trial = limits();
        trial["trialComputeUsedSeconds"] = json!(90000);
        trial["wallet"] = json!({"balanceDollars": 10000});
        assert!(budget(&trial, &BoatConfig::default(), Machine::Default, 1).is_err());
    }

    #[test]
    fn eon_waits_ten_minutes_of_silence_or_unreadable_status() {
        assert_eq!(Boat::EON.patience, 600);
    }

    #[test]
    fn a_malformed_source_fails_verify() {
        for source in [json!({"x": 1}), json!("tree"), Value::Null] {
            let error = Boat::EON.verify(&source).unwrap_err();
            assert!(error.contains("boat-source-format"), "{source}: {error}");
        }
    }

    #[test]
    fn a_malformed_source_fails_a_remote_run_before_the_grant() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = rota::config::SlotConfig::default();
        config
            .boat
            .insert("allow_shapes".into(), vec!["nix-gate"].into());
        let argv = words(&["nix", "build", ".#default"]);
        let request = Request {
            config: &config,
            root: directory.path(),
            argv: &argv,
            quiet_exempt: false,
            limit: Limit {
                class: rota::slots::Class::Nix,
                cap_gib: 8,
            },
        };
        let mut granted = false;
        let mut source = Some(json!({"x": 1}));
        let outcome = Boat::EON.run_remote(&request, &mut || granted = true, &mut source);
        assert!(
            matches!(&outcome, Remote::Failed { reason, .. } if reason == "boat-source-format"),
            "{outcome:?}"
        );
        assert!(!granted);
    }

    #[test]
    fn an_unreadable_boat_table_fails_closed_before_the_grant() {
        let directory = tempfile::tempdir().unwrap();
        let argv = words(&["nix", "build", ".#default"]);
        for (key, value) in [
            ("jobs", toml::Value::from(9)),
            ("allow_shapes", toml::Value::from("nix-gate")),
            ("unknown", toml::Value::from(1)),
        ] {
            let mut config = rota::config::SlotConfig::default();
            config
                .boat
                .insert("allow_shapes".into(), vec!["nix-gate"].into());
            config.boat.insert(key.into(), value);
            let request = Request {
                config: &config,
                root: directory.path(),
                argv: &argv,
                quiet_exempt: false,
                limit: Limit {
                    class: rota::slots::Class::Nix,
                    cap_gib: 8,
                },
            };
            for stage in [
                Stage::Queued,
                Stage::Waiting {
                    limit_changed: true,
                },
            ] {
                assert!(
                    matches!(Boat::EON.place(&request, stage), Placement::Remote),
                    "{key}"
                );
            }
            let mut granted = false;
            let outcome = Boat::EON.run_remote(&request, &mut || granted = true, &mut None);
            assert!(
                matches!(&outcome, Remote::Failed { reason, memory: None } if reason == "boat-config-invalid"),
                "{key}: {outcome:?}"
            );
            assert!(!granted, "{key}");
        }
    }

    #[test]
    fn a_malformed_source_never_runs_locally() {
        // The decode comes before any local path, routed or plain, so a bad source never
        // reaches the command whether or not a terminal is attached.
        let directory = tempfile::tempdir().unwrap();
        let sentinel = directory.path().join("ran");
        let routed = Fallback {
            reason: "boat-capacity full".into(),
            kind: Kind::Kept,
        };
        let declined = Fallback {
            reason: "policy-local".into(),
            kind: Kind::Declined,
        };
        for (source, fallback) in [json!({"x": 1}), Value::Null]
            .into_iter()
            .flat_map(|source| [Some(&routed), Some(&declined), None].map(|f| (source.clone(), f)))
        {
            let mut command = Command::new("touch");
            command.arg(&sentinel);
            let error = Boat::EON
                .run_local(Local {
                    root: directory.path(),
                    command: &mut command,
                    fallback,
                    source: Some(&source),
                    program: "rota",
                })
                .unwrap_err();
            assert!(error.to_string().contains("boat-source-format"), "{error}");
            assert!(!sentinel.exists(), "{fallback:?}");
        }
    }
}
