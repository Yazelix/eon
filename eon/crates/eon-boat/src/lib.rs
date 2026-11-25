//! Opt-in build execution; the local slot scheduler remains the admission owner.
#![cfg(target_os = "linux")]
mod attach;
mod config;
pub(crate) mod image;
mod lease;
pub(crate) mod local;
mod memory;
pub(crate) mod tree;

pub use config::{BoatConfig, Shape};
use image::{Image, Refusal};
use lease::{Admission, Lease};
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
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Boat's noun in private-state error tokens (`unsafe-boat-state-directory`, ...).
pub(crate) const NOUN: &str = "boat";
const CONTROL: u64 = 30;
const CREATE_UNSUBMITTED: &str = "boat-create-not-submitted";
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

impl Record {
    fn has_live_vm(&self) -> bool {
        matches!(self.phase.as_str(), "preparing" | "running" | "stopping")
    }
}

struct Client {
    deadline: Instant,
    cleanup: bool,
    reserve_seconds: f64,
    parallel_leases: u8,
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
            parallel_leases: 1,
            machine,
        }
    }

    fn canceled(&self) -> bool {
        !self.cleanup && Signals::caught() != 0
    }

    fn can_launch_create(&self, retry: bool, now: Instant, signal: i32) -> bool {
        let needed = Duration::from_secs(if retry { CONTROL } else { 0 });
        (self.cleanup || signal == 0) && self.deadline > now + needed
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
        let creating = args.first() == Some(&"new");
        if !creating && Instant::now() >= self.deadline {
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
        let mut command = self.command(json || attach, args);
        let mut child = if creating {
            spawn_create(
                || command.spawn(),
                |retry| self.can_launch_create(retry, Instant::now(), Signals::caught()),
            )?
        } else {
            command.spawn().map_err(|_| "boat-unreachable")?
        };
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

fn spawn_create<T>(
    mut spawn: impl FnMut() -> io::Result<T>,
    permitted: impl Fn(bool) -> bool,
) -> Result<T, &'static str> {
    // A spawn error means Boat never ran. Once it runs, its result cannot authorize a replay.
    for retry in [false, true] {
        if !permitted(retry) {
            break;
        }
        if let Ok(child) = spawn() {
            return Ok(child);
        }
    }
    Err(CREATE_UNSUBMITTED)
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
    let creating = operation == "new";
    let unsupported = |reason| {
        if creating {
            reason
        } else {
            "unsupported-provider-output"
        }
    };
    let values = serde_json::Deserializer::from_slice(bytes)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unsupported("boat-create-invalid-json"))?;
    let error_frame = values.iter().any(|value| {
        value["ok"] == false || value["success"] == false || value["event"] == "error"
    });
    if code != 0 || error_frame {
        return Err(if !creating {
            "boat-command-failed".into()
        } else if error_frame {
            "boat-create-error-frame".into()
        } else {
            format!("boat-create-exit-{code}")
        });
    }
    let value = if matches!(operation, "new" | "info") {
        values
            .into_iter()
            .rev()
            .find(|value| sandbox(value)["id"].as_str().is_some_and(safe_id))
    } else {
        values.into_iter().last()
    };
    value.ok_or_else(|| unsupported("boat-create-missing-id").into())
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
    if list["sandboxes"].as_array().is_some_and(|vms| {
        vms.iter().all(|vm| {
            vm["id"]
                .as_str()
                .is_some_and(|other| safe_id(other) && other != id)
        })
    }) {
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

fn check_owned_id(root: &Path, slot: u8, id: &str) -> Result<(), String> {
    for other in lease::SLOTS.filter(|other| *other != slot) {
        if lease::record(root, other)?.is_some_and(|record| record.vm_id.as_deref() == Some(id)) {
            return Err("remote-vm-id-conflict".into());
        }
    }
    Ok(())
}

fn created_vm(list: &Value, run_id: &str) -> Result<Option<String>, String> {
    if list.get("pageInfo").is_some() && list["pageInfo"]["hasMore"] != false {
        return Err("unsupported-provider-output".into());
    }
    let rows = list["sandboxes"]
        .as_array()
        .ok_or("unsupported-provider-output")?;
    let expected = format!("eon-build-{run_id}");
    let mut matches = rows.iter().filter(|row| row["name"] == expected);
    let Some(row) = matches.next() else {
        return Ok(None);
    };
    let id = row["id"]
        .as_str()
        .filter(|id| safe_id(id))
        .ok_or("remote-create-unknown")?;
    if matches.next().is_some() || rows.iter().filter(|row| row["id"] == id).nth(1).is_some() {
        return Err("remote-create-unknown".into());
    }
    Ok(Some(id.to_owned()))
}

fn recover(client: &Client, lease: &Lease) -> Result<(), String> {
    if let Some(mut old) = lease::record(&lease.root, lease.slot)?
        && old.phase != "stopped"
    {
        recover_record(client, lease, &mut old, None)?;
    }
    Ok(())
}

fn recover_record(
    client: &Client,
    lease: &Lease,
    old: &mut Record,
    evidence: Option<&attach::Evidence>,
) -> Result<(), String> {
    let id = match &old.vm_id {
        Some(id) if safe_id(id) => id.clone(),
        Some(_) => return Err("remote-create-unknown".into()),
        None => {
            let find = || created_vm(&client.json(&["list", "--all"])?, &old.run_id);
            let mut found = find()?;
            if found.is_none()
                && let Some(log) = evidence
            {
                log.control("create inventory empty, rechecking");
                found = find()?;
            }
            match found {
                Some(id) => id,
                None if old
                    .expires_at
                    .is_some_and(|end| rota::slots::now_ns() / rota::slots::SECOND > end) =>
                {
                    old.phase = "stopped".into();
                    old.reason = Some(match old.reason.take() {
                        Some(reason) => format!("{reason}; expired-create-absent"),
                        None => "expired-create-absent".into(),
                    });
                    return lease.save(old);
                }
                None => return Err("remote-create-unknown".into()),
            }
        }
    };
    check_owned_id(&lease.root, lease.slot, &id)?;
    let published = if old.vm_id.is_none() {
        old.vm_id = Some(id.clone());
        lease.save(old)
    } else {
        Ok(())
    };
    // Current-run recovery still retires a proved orphan under admission if publication fails.
    if evidence.is_none() {
        published.clone()?;
    }
    let target = (id.as_str(), old.started);
    let (usage, retired) = retire(client.machine, !client.cleanup, target, evidence);
    if usage.is_some() {
        old.usage = usage;
    }
    if let Err(cleanup) = &retired {
        if evidence.is_none() {
            return retired;
        }
        old.phase = "cleanup-unknown".into();
        old.reason = Some(match old.reason.take() {
            Some(reason) => format!("{reason}; cleanup {cleanup}"),
            None => cleanup.clone(),
        });
    } else {
        old.phase = "stopped".into();
        old.reason
            .get_or_insert_with(|| "recovered-after-wrapper-loss".into());
    }
    // `duration_seconds` stays the run's own: reconciliation time is not run time.
    let saved = lease.save(old);
    published?;
    saved?;
    retired
}

/// While starts are serialized, recover only the journals whose locks we can take.
fn reconcile(client: &Client, own: &Lease) -> Result<Vec<Record>, String> {
    let mut siblings = Vec::new();
    for slot in lease::SLOTS {
        if slot == own.slot {
            recover(client, own)?;
            continue;
        }
        if lease::record(&own.root, slot)?.is_none_or(|record| record.phase == "stopped") {
            continue;
        }
        if let Some(abandoned) = Lease::claim(&own.root, slot)? {
            recover(client, &abandoned)?;
        } else if let Some(record) = lease::record(&own.root, slot)?
            && record.phase != "stopped"
        {
            let id = record
                .vm_id
                .as_deref()
                .filter(|id| safe_id(id))
                .ok_or("remote-sibling-unverified")?;
            if !record.has_live_vm()
                || record.expires_at.is_none()
                || record.machine != Machine::Large
            {
                return Err("remote-sibling-unverified".into());
            }
            check_owned_id(&own.root, slot, id)?;
            siblings.push(record);
        }
    }
    Ok(siblings)
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
            "REQUEST: Boat compute credit/plan needs owner attention ({reason}); using queued local admission, no billing action"
        );
    }
    let signal = Signals::caught();
    Ok(if signal == 0 {
        Outcome::Local(reason)
    } else {
        Outcome::Complete((128 + signal) as u8, None)
    })
}

fn run(
    admission: Admission,
    config: &BoatConfig,
    argv: &[String],
    shape: Shape,
    memory: (Limit, &'static Image),
    identity: &mut Option<tree::Identity>,
    patience: u64,
) -> Result<Outcome, String> {
    let Admission { lease, gate } = admission;
    let mut gate = Some(gate);
    let (limit, image) = memory;
    let machine = image.machine;
    let _signals = Signals::install();
    let wall_started = Instant::now();
    let started = rota::slots::now_ns() / rota::slots::SECOND;
    let client = Client {
        deadline: Instant::now() + Duration::from_secs(config.timeout_seconds - CLEANUP),
        cleanup: false,
        reserve_seconds: config.reserve_hours * 3600.0 * machine.weight(),
        parallel_leases: config.max_parallel_leases,
        machine,
    };
    if qualification_blocked(&lease.root, machine) {
        return fallback("boat-capacity qualification-stale".into());
    }
    let siblings = match reconcile(&client, &lease) {
        Ok(siblings) => siblings,
        Err(reason) => return fallback(reason),
    };
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
        start_limits(&client, config, &siblings)?;
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
        expires_at: None,
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
    let name = format!("eon-build-{run_id}");
    let ttl = config.timeout_seconds.to_string();
    let allowance_before = match reconcile(&client, &lease)
        .and_then(|siblings| start_limits(&client, config, &siblings))
    {
        Ok(limits) => limits,
        Err(reason) => return fallback(reason),
    };
    if qualification_blocked(&lease.root, machine) {
        return fallback("boat-capacity qualification-stale".into());
    }
    // Include the bounded create call before the provider's VM lifetime.
    record.expires_at =
        Some(rota::slots::now_ns() / rota::slots::SECOND + config.timeout_seconds + CONTROL);
    lease.save(&record)?;
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
    let id = created.map_err(|reason| (reason, false)).and_then(|value| {
        let id = sandbox(&value)["id"]
            .as_str()
            .filter(|id| safe_id(id))
            .ok_or_else(|| ("boat-create-missing-id".to_owned(), false))?;
        check_owned_id(&lease.root, lease.slot, id).map_err(|reason| (reason, true))?;
        Ok(id.to_owned())
    });
    let id = match id {
        Ok(id) => id,
        Err((reason, identity_conflict)) => {
            evidence.control(format!("create error={reason}"));
            record.reason = Some(reason.clone());
            if reason == CREATE_UNSUBMITTED {
                record.phase = "stopped".into();
                lease.save(&record)?;
                return fallback(reason);
            }
            let saved = lease.save(&record);
            let recovered = recover_record(
                &Client::cleanup(machine, CLEANUP, false),
                &lease,
                &mut record,
                Some(&evidence),
            );
            let detail = recovered.as_ref().err().map_or("ok", String::as_str);
            evidence.control(format!("create recovery {detail}"));
            saved?;
            return match recovered {
                Err(reason) if identity_conflict || record.vm_id.is_some() => Err(reason),
                Err(reason) => fallback(reason),
                Ok(()) if identity_conflict => {
                    fallback("remote-create-id-conflict-reconciled".into())
                }
                Ok(()) => fallback("remote-create-failed-reconciled".into()),
            };
        }
    };
    record.vm_id = Some(id.clone());
    record.phase = "preparing".into();
    let saved = lease.save(&record);
    // Keep admission closed through retirement if the VM identity could not be published.
    if saved.is_ok() {
        drop(gate.take());
    }
    let mut released = false;
    let execution = (|| {
        saved?;
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
    if let Ok(after) = Client::cleanup(machine, CONTROL, false).json(&["limits", "--all"]) {
        let before = &allowance_before;
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

/// Classifies the literal command. The original argv still runs as given.
pub(crate) fn shape(config: &BoatConfig, argv: &[String]) -> Option<Shape> {
    let limit = config.timeout_seconds.saturating_sub(CLEANUP + PREPARATION);
    let command = gate_command(argv, limit)?;
    let shape = if nix_gate(command) {
        Shape::NixGate
    } else {
        dev_command(command)?
    };
    config.allow_shapes.contains(&shape).then_some(shape)
}

fn gate_command(mut argv: &[String], limit: u64) -> Option<&[String]> {
    let mut seen_timeout = false;
    let mut seen_env = false;
    loop {
        let (program, args) = argv.split_first()?;
        argv = match program.as_str() {
            "timeout" if !seen_timeout => {
                seen_timeout = true;
                timeout_command(args, limit)?
            }
            "env" if !seen_env => {
                seen_env = true;
                let args = match args {
                    [separator, rest @ ..] if separator == "--" => rest,
                    _ => args,
                };
                // These exact values are safe to forward to the guest in argv.
                let command = args
                    .iter()
                    .position(|arg| !matches!(arg.as_str(), "LANG=C" | "LC_ALL=C"))?;
                &args[command..]
            }
            "timeout" | "env" => return None,
            _ => return Some(argv),
        };
    }
}

fn timeout_command(mut argv: &[String], limit: u64) -> Option<&[String]> {
    let mut kill_after = None;
    let mut seen_signal = false;
    while let Some((option, rest)) = argv.split_first().filter(|(word, _)| word.starts_with('-')) {
        argv = rest;
        let (flag, inline) = option
            .split_once('=')
            .map_or((option.as_str(), None), |(flag, value)| (flag, Some(value)));
        let mut operand = || {
            inline.or_else(|| {
                let (value, rest) = argv.split_first()?;
                argv = rest;
                Some(value.as_str())
            })
        };
        match (flag, inline) {
            ("--", None) => break,
            ("--foreground" | "--preserve-status", None) => (),
            ("-k", None) | ("--kill-after", _) if kill_after.is_none() => {
                kill_after = Some(seconds(operand()?)?);
            }
            ("-s", None) | ("--signal", _) if !seen_signal => {
                let signal = operand()?;
                if !matches!(signal, "1" | "2" | "3" | "9" | "15")
                    && !matches!(
                        signal.strip_prefix("SIG").unwrap_or(signal),
                        "HUP" | "INT" | "QUIT" | "KILL" | "TERM"
                    )
                {
                    return None;
                }
                seen_signal = true;
            }
            _ => return None,
        }
    }
    let (duration, command) = argv.split_first()?;
    // The envelope and any kill grace period share Boat's post-preparation budget.
    (seconds(duration)?.checked_add(kill_after.unwrap_or(0))? <= limit).then_some(command)
}

fn seconds(duration: &str) -> Option<u64> {
    let digits = duration.strip_suffix(['s', 'm', 'h']).unwrap_or(duration);
    if digits.len() > 6 || digits.starts_with('0') {
        return None;
    }
    rota::run::parse_timeout(duration)
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
            "-p" if cargo => {
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

fn keep_going_off(limits: &Value) -> bool {
    limits["keepGoing"]["enabled"].as_bool() == Some(false)
}

fn nonnegative_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}

fn pack_fields_supported(value: &Value, root: bool) -> bool {
    match value {
        Value::Object(fields) => fields.iter().all(|(key, value)| {
            if root {
                match key.as_str() {
                    "packBalanceSeconds" => return true,
                    "packBalanceHours" | "packBalanceDollars" => {
                        return nonnegative_number(value).is_some();
                    }
                    _ => {}
                }
            }
            // The current limits schema exposes no pack expiry. Unknown terms refuse the paid route.
            let key = key.to_ascii_lowercase();
            !key.contains("pack") && !key.contains("expir") && pack_fields_supported(value, false)
        }),
        Value::Array(values) => values
            .iter()
            .all(|value| pack_fields_supported(value, false)),
        _ => true,
    }
}

fn prepaid_remaining(limits: &Value) -> Result<f64, String> {
    let missing = || "unsupported-provider-output".to_owned();
    let number = |v: &Value| nonnegative_number(v).ok_or_else(missing);
    let plan_seconds = number(&limits["planIncluded"]["vmHoursRemaining"])? * 3600.0;
    if !plan_seconds.is_finite() {
        return Err(missing());
    }
    let plan_seconds = plan_seconds.min(number(&limits["subscriptionRemainingSeconds"])?);
    if billing_policy(limits)? {
        let trial = number(&limits["trialComputeCapSeconds"])?
            - number(&limits["trialComputeUsedSeconds"])?;
        return Ok(plan_seconds.min(trial));
    }
    let pack_seconds = limits["packBalanceSeconds"]
        .as_u64()
        .filter(|&seconds| seconds <= (1u64 << f64::MANTISSA_DIGITS))
        .ok_or_else(missing)?;
    if !pack_fields_supported(limits, true) {
        return Err(missing());
    }
    Ok(plan_seconds + pack_seconds as f64)
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
        parallel_leases: 1,
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

fn start_limits(
    client: &Client,
    config: &BoatConfig,
    siblings: &[Record],
) -> Result<Value, String> {
    let limits = client.json(&["limits", "--all"])?;
    let now = rota::slots::now_ns() / rota::slots::SECOND;
    budget(&limits, config, client.machine, now, siblings)?;
    let active = limits["activeSandboxes"]
        .as_u64()
        .ok_or("unsupported-provider-output")?;
    if active != 0 || !siblings.is_empty() {
        let inventory = client.json(&["list", "--all"])?;
        let rows = inventory["sandboxes"]
            .as_array()
            .ok_or("unsupported-provider-output")?;
        let mut seen = std::collections::BTreeSet::new();
        for row in rows.iter().filter(|row| {
            !matches!(
                row["state"].as_str(),
                Some("stopped" | "archived" | "deleted")
            )
        }) {
            let id = row["id"]
                .as_str()
                .filter(|id| safe_id(id))
                .ok_or("unsupported-provider-output")?;
            if !seen.insert(id)
                || !siblings.iter().any(|record| {
                    record.vm_id.as_deref() == Some(id)
                        && row["name"] == format!("eon-build-{}", record.run_id)
                        && row["type"] == record.machine.as_str()
                })
            {
                return Err("boat-unowned-active-vm".into());
            }
        }
        if seen.len() as u64 != active {
            return Err("boat-active-inventory-mismatch".into());
        }
    }
    Ok(limits)
}

fn budget(
    limits: &Value,
    config: &BoatConfig,
    machine: Machine,
    now: u64,
    siblings: &[Record],
) -> Result<(), String> {
    if limits["canStart"].as_bool() != Some(true)
        || !keep_going_off(limits)
        || limits["nextStartDollars"].as_f64() != Some(0.0)
        || !type_available(limits, machine)
    {
        return Err("boat-unavailable-or-paid-start".into());
    }
    if limits["activeSandboxes"]
        .as_u64()
        .is_none_or(|active| active > siblings.len() as u64)
    {
        return Err("boat-unowned-active-vm".into());
    }
    for window in ["minute", "hour", "day"] {
        if limits["starts"][window]["remaining"]
            .as_u64()
            .is_none_or(|n| n == 0)
        {
            return Err("start-limit".into());
        }
    }
    let mut required = machine.weight()
        * (config.reserve_hours * 3600.0 + config.timeout_seconds as f64 + TEARDOWN as f64);
    for sibling in siblings {
        let expires = sibling.expires_at.ok_or("remote-sibling-unverified")?;
        required +=
            sibling.machine.weight() * (expires.saturating_sub(now) as f64 + TEARDOWN as f64);
    }
    if prepaid_remaining(limits)? < required {
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
        let Ok(mut config) = BoatConfig::from_table(&request.config.boat) else {
            return Remote::Failed {
                reason: "boat-config-invalid".into(),
                memory: None,
            };
        };
        if let Some(path) = request.config_file {
            let current = fs::read_to_string(path)
                .ok()
                .and_then(|text| BoatConfig::from_text(&text).ok());
            let Some(current) = current else {
                static WARNED_CONFIG: AtomicBool = AtomicBool::new(false);
                if !WARNED_CONFIG.swap(true, Ordering::Relaxed) {
                    let _ = writeln!(
                        io::stderr(),
                        "eon agent slot: waiting for readable and valid [slots.boat] configuration"
                    );
                }
                return Remote::Wait;
            };
            config.max_parallel_leases = current.max_parallel_leases;
        }
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
        let admission = match Admission::claim(request.root, config.max_parallel_leases) {
            Ok(Some(admission)) => admission,
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
            admission,
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
        lease::listing(root, lock_owner)
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
        let error = b"{\"sandbox\":{\"id\":\"bx_test\"}}\n{\"event\":\"error\"}";
        for (code, bytes, operation, reason) in [
            (0, error.as_slice(), "new", "boat-create-error-frame"),
            (1, error.as_slice(), "new", "boat-create-error-frame"),
            (7, b"{}", "new", "boat-create-exit-7"),
            (0, b"{", "new", "boat-create-invalid-json"),
            (0, b"{}", "new", "boat-create-missing-id"),
            (1, error.as_slice(), "list", "boat-command-failed"),
        ] {
            assert_eq!(decode(code, bytes, operation).unwrap_err(), reason);
        }
    }

    #[test]
    fn create_launch_replays_only_a_failed_spawn_and_honors_preflight() {
        let client = Client::cleanup(Machine::Large, CONTROL + 1, true);
        let failed = Err(io::ErrorKind::WouldBlock);
        let gone = Err(io::ErrorKind::NotFound);
        let refused = Err(CREATE_UNSUBMITTED);
        for (remaining, signal, launches, expected, expected_calls) in [
            (1, 0, [Ok(42), failed], Ok(42), 1),
            (CONTROL + 1, 0, [failed, Ok(42)], Ok(42), 2),
            (CONTROL + 1, 0, [failed, gone], refused, 2),
            (CONTROL - 1, 0, [failed, Ok(42)], refused, 1),
            (1, libc::SIGTERM, [Ok(42), failed], refused, 0),
            (0, 0, [Ok(42), failed], refused, 0),
        ] {
            let now = client.deadline - Duration::from_secs(remaining);
            let mut attempts = launches.into_iter();
            let result = spawn_create(
                || attempts.next().unwrap().map_err(io::Error::from),
                |retry| client.can_launch_create(retry, now, signal),
            );
            assert_eq!(2 - attempts.len(), expected_calls);
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn create_inventory_requires_a_complete_unambiguous_identity() {
        let matching = json!({"id": "bx_vm", "name": "eon-build-test"});
        let unrelated = json!({"id": "unsupported"});
        let scan = |rows: &[Value]| created_vm(&json!({"sandboxes": rows}), "test");
        assert_eq!(scan(&[unrelated.clone(), unrelated.clone()]), Ok(None));
        let found = scan(&[matching.clone(), unrelated]);
        assert_eq!(found, Ok(Some("bx_vm".into())));
        assert!(scan(&[matching.clone(), json!({"id":"bx_vm"})]).is_err());
        assert!(scan(&[matching, json!({"id":"bx_other","name":"eon-build-test"})]).is_err());
        assert!(created_vm(&json!({"sandboxes":[],"pageInfo":{"hasMore":true}}), "test").is_err());
    }

    pub(super) fn trial_limits() -> Value {
        json!({
            "canStart": true, "activeSandboxes": 0, "keepGoing": {"enabled": false}, "nextStartDollars": 0,
            "machineTypes": [{"type": "default", "available": true}],
            "starts": {"minute": {"remaining": 3}, "hour": {"remaining": 20}, "day": {"remaining": 74}},
            "planIncluded": {"vmHoursRemaining": 555.0}, "subscriptionRemainingSeconds": 1998000,
            "accessTier": "trial", "subscriptionStatus": "trialing", "sandboxPlanKey": "trial",
            "trialComputeCapSeconds": 90000, "trialComputeUsedSeconds": 2000, "subscriptionTrialEndsAt": "2099-01-01T00:00:00.000Z"
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
            &words(&["nix", "develop", "-c", "eon-test"])
        ));
        for argv in [
            vec!["nix", "develop", "-c", "cargo", "test"],
            vec!["nix", "develop", "-c", "eon-test", "-p", "rota-cli"],
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
                "timeout",
                "30s",
                "nix",
                "develop",
                "-c",
                "eon-test",
                "--no-capture"
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
            "0", "0m", "012m", "1.5h", "2d", "inf", "infinity", "9999999h", "m", "1M", "1m\u{e9}",
            "\u{e9}",
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
    fn boat_gate_prefixes_preserve_shape() {
        let config = BoatConfig {
            allow_shapes: vec![Shape::EonTest],
            timeout_seconds: 7200,
            ..BoatConfig::default()
        };
        let gate = ["nix", "develop", "-c", "eon-test", "--no-capture"];
        for prefix in [
            "timeout 120",
            "timeout -k 5 -s KILL -- 5335s",
            "timeout --kill-after=5s --signal=SIGTERM 1m",
            "timeout --kill-after 5s --signal 15 --foreground --preserve-status 1m env -- LC_ALL=C",
            "env -- LANG=C LC_ALL=C timeout 1m",
            "env LANG=C",
            "env --",
            "env",
        ] {
            let argv: Vec<_> = prefix.split_ascii_whitespace().chain(gate).collect();
            assert_eq!(
                shape(&config, &words(&argv)),
                Some(Shape::EonTest),
                "{argv:?}"
            );
        }
        let disabled = BoatConfig::default();
        assert!(!allowed(
            &disabled,
            &words(&["env", "nix", "develop", "-c", "eon-test"])
        ));
    }

    #[test]
    fn boat_gate_prefixes_fail_closed() {
        let config = BoatConfig {
            allow_shapes: vec![Shape::NixGate, Shape::EonTest],
            timeout_seconds: 7200,
            ..BoatConfig::default()
        };
        for prefix in [
            "/usr/bin/timeout 1m",
            "timeout --verbose 1m",
            "timeout --foregroun 1m",
            "timeout --foreground=yes 1m",
            "timeout -k 1m",
            "timeout -k 0 1m",
            "timeout -k 5 -k 1 1m",
            "timeout --kill-after=5 5336",
            "timeout --signal=STOP 1m",
            "timeout --signal= 1m",
            "timeout -s TERM -s KILL 1m",
            "timeout 1m --",
            "timeout 1m env timeout 1m",
            "/usr/bin/env",
            "env env",
            "env LANG=C --",
            "env -- -",
            "env -C /work",
            "env --chdir=/work",
            "env -S LC_ALL=C",
            "env --split-string=LC_ALL=C",
            "env -u HOME",
            "env --unset=HOME",
            "env -i",
            "env --ignore-environment",
            "env --debug",
            "env LITERAL=value",
            "env LC_ALL=/private/value",
            "env CARGO_TARGET_DIR=/private/target",
            "env -- PATH=/private/bin",
        ] {
            let argv: Vec<_> = prefix
                .split_ascii_whitespace()
                .chain(["nix", "build", ".#default"])
                .collect();
            assert!(!allowed(&config, &words(&argv)), "{argv:?}");
        }
        for command in [
            &["bash", "-c", "nix build .#default"][..],
            &["sh", "-c", "nix build .#default"],
            &["bash", "-e", "gate.sh"],
            &["nix build .#default"],
            &["nix", "develop", "-c", "eon-test", "-p", "rota-cli"],
            &["nix", "develop", "-c", "cargo", "fmt"],
            &["nix", "develop", "-c", "eon-bench", "gate"],
            &[],
        ] {
            let argv: Vec<_> = ["env", "LC_ALL=C", "timeout", "1m"]
                .into_iter()
                .chain(command.iter().copied())
                .collect();
            assert!(!allowed(&config, &words(&argv)), "{argv:?}");
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
        let check = |limits: &Value| budget(limits, &config, Machine::Default, 1, &[]);
        let mut good = trial_limits();
        good["packBalanceSeconds"] = Value::Null;
        good["packExpiresAt"] = Value::Null;
        assert!(check(&good).is_ok());
        let mut bad = good.clone();
        bad["trialComputeUsedSeconds"] = json!(89999);
        bad["packBalanceSeconds"] = json!(1_000_000);
        let exhausted = Err("budget-reserve-or-trial-cap".into());
        assert_eq!(check(&bad), exhausted);
        bad["trialComputeUsedSeconds"] = json!(0);
        bad["planIncluded"]["vmHoursRemaining"] = json!(1);
        assert_eq!(check(&bad), exhausted);
        for (pointer, value) in [
            ("/activeSandboxes", Value::Null),
            ("/activeSandboxes", json!(1)),
            ("/starts/minute/remaining", json!(0)),
            ("/subscriptionRemainingSeconds", Value::Null),
            ("/trialComputeUsedSeconds", Value::Null),
            ("/accessTier", Value::Null),
        ] {
            let mut bad = good.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(check(&bad).is_err(), "{pointer}");
        }
        let mut boundary = good;
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 240);
        assert!(check(&boundary).is_ok());
        boundary["trialComputeUsedSeconds"] = json!(90000 - 7200 - 3600 - 239);
        assert_eq!(check(&boundary), exhausted);
    }

    pub(super) fn paid_limits() -> Value {
        // Observed Boat 1.0.105 limits --all shape with synthetic values.
        serde_json::from_str(include_str!("../tests/fixtures/limits-1.0.105.json")).unwrap()
    }

    #[test]
    fn prepaid_pack_admission_preserves_weighted_budget_and_start_policy() {
        let mut paid = paid_limits();
        assert_eq!(prepaid_remaining(&paid), Ok(23599.0));
        let config = BoatConfig::default();
        let check =
            |paid: &Value, siblings: &[Record]| budget(paid, &config, Machine::Large, 1, siblings);
        let pack_needed = 2 * (7200 + 3600 + TEARDOWN) - 3599;
        let insufficient = || Err("budget-reserve-or-trial-cap".into());
        let siblings = [serde_json::from_value(json!({
            "schema": 1, "run_id": "sibling", "pid": 1, "phase": "running", "where": "boat",
            "tree": "test", "base": "test", "started": 0, "expires_at": 61,
            "credentials_absent": true, "machine": "large"
        }))
        .unwrap()];
        for (siblings, needed) in [
            (&[][..], pack_needed),
            (&siblings[..], pack_needed + 2 * (60 + TEARDOWN)),
        ] {
            paid["activeSandboxes"] = json!(siblings.len());
            paid["packBalanceSeconds"] = json!(needed - 1);
            assert_eq!(check(&paid, siblings), insufficient());
            paid["packBalanceSeconds"] = json!(needed);
            assert_eq!(check(&paid, siblings), Ok(()));
        }
        paid["planIncluded"]["vmHoursRemaining"] = json!(0);
        assert_eq!(check(&paid, &siblings), insufficient());
        paid["packBalanceSeconds"] = json!(0);
        paid["planIncluded"]["vmHoursRemaining"] = json!(10);
        paid["subscriptionRemainingSeconds"] = json!(36000);
        assert_eq!(check(&paid, &siblings), Ok(()));

        let mut paid = paid_limits();
        paid["packBalanceHours"] = json!(1000);
        let paid_start = "boat-unavailable-or-paid-start";
        let exhausted = "budget-reserve-or-trial-cap";
        let plan = "unverified-or-nonstandard-plan";
        let past = json!("1970-01-01T00:00:01Z");
        for (pointer, value, reason) in [
            ("/packBalanceSeconds", json!(0), exhausted),
            ("/keepGoing/enabled", json!(true), paid_start),
            ("/keepGoing/enabled", Value::Null, paid_start),
            ("/nextStartDollars", json!(0.03), paid_start),
            ("/starts/day/remaining", json!(0), "start-limit"),
            ("/planIncluded/periodEnd", past, "billing-period-boundary"),
            ("/activeSandboxPlanKey", json!("box_100"), plan),
        ] {
            let mut limits = paid.clone();
            *limits.pointer_mut(pointer).unwrap() = value;
            assert_eq!(check(&limits, &[]), Err(reason.into()), "{pointer}");
        }
    }

    #[test]
    fn prepaid_pack_schema_fails_closed() {
        let mut paid = paid_limits();
        paid["packBalanceSeconds"] = json!(1u64 << 53);
        paid["subscriptionRemainingSeconds"] = json!(36000);
        for key in ["packBalanceHours", "packBalanceDollars"] {
            paid.as_object_mut().unwrap().remove(key);
        }
        assert!(prepaid_remaining(&paid).is_ok());
        let unsupported = Err("unsupported-provider-output".into());
        let refuse = |v: &Value| assert_eq!(prepaid_remaining(v), unsupported);
        let mutate = |key: &str, value| {
            let mut bad = paid.clone();
            bad[key] = value;
            refuse(&bad);
        };
        let mut bad = paid.clone();
        bad.as_object_mut().unwrap().remove("packBalanceSeconds");
        refuse(&bad);
        bad = paid.clone();
        bad["planIncluded"]["vmHoursRemaining"] = json!(1e306);
        refuse(&bad);
        let seconds = "packBalanceSeconds";
        for key in ["packBalanceHours", "packBalanceDollars", seconds] {
            for value in [json!(-1), Value::Null, json!("1")] {
                mutate(key, value);
            }
            mutate("other", json!([{(key): 1}]));
        }
        for (key, value) in [
            (seconds, json!(1e306)),
            (seconds, json!((1u64 << 53) + 1)),
            ("packStatus", json!({})),
            ("packExpiresAt", json!("1970-01-01T00:00:00Z")),
            ("packExpiresAt", json!("2099-01-01T00:00:00Z")),
            ("packExpiresAt", Value::Null),
            ("packExpiresAt", json!("unknown")),
            ("other", json!([{"expiresAt": null}])),
        ] {
            mutate(key, value);
        }
        assert!(decode(0, br#"{"packBalanceDollars":1e999}"#, "limits").is_err());
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
            config_file: None,
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
                config_file: None,
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
