use eon_workspace_protocol::v50::{
    ClaudeQuota, ClaudeQuotaState, CodexQuota, CodexQuotaState, GoogleQuota, GoogleQuotaState,
    MAX_GOOGLE_QUOTA_WINDOWS, MAX_QUOTA_SCOPE_BYTES, MAX_SCOPED_QUOTA_WINDOWS, QuotaFailure,
    QuotaFailureReason as Failure, QuotaWindow, Snapshot,
};

mod google;
pub use google::{AgyCheck, AgyVersion, agy_version_check};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        process::CommandExt,
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
/// Codex is read every 60 units, Claude and Google every 300; a unit is a second (EON-C27).
const UNIT: Duration = Duration::from_secs(1);
const CODEX_UNITS: u32 = 60;
const SHARED_UNITS: u32 = 300;
/// Codex keeps no failure record, so its stored reading stands in for a window only while it is
/// at most two reads old (EON-C40).
const STORED_CODEX_MAX_AGE: u64 = 2 * CODEX_UNITS as u64;
/// CodexBar names Claude's model-scoped weekly windows `claude-weekly-scoped-<model>`.
const CLAUDE_SCOPED: &str = "claude-weekly-scoped-";

fn timestamp(value: &Value) -> Result<u64, ()> {
    let value = value.as_str().ok_or(())?;
    let date = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| ())?;
    u64::try_from(date.unix_timestamp()).map_err(|_| ())
}

/// Validates each provider's own scope/count/duration bounds, including retained data (EON-C27).
fn valid_windows(provider: &str, windows: &[QuotaWindow]) -> bool {
    if provider == "google" {
        return (1..=MAX_GOOGLE_QUOTA_WINDOWS).contains(&windows.len())
            && windows.iter().enumerate().all(|(index, w)| {
                matches!(w.duration_minutes, 300 | 10080)
                    && w.remaining_percent <= 100
                    && w.scope.as_deref().is_some_and(|scope| {
                        !scope.is_empty()
                            && scope.len() <= MAX_QUOTA_SCOPE_BYTES
                            && !scope.chars().any(char::is_control)
                    })
                    && !windows[..index].iter().any(|other| {
                        other.duration_minutes == w.duration_minutes && other.scope == w.scope
                    })
            });
    }
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
        && valid_windows(provider, &windows)
        && (provider != "google"
            || windows
                .iter()
                .all(|w| w.resets_at.is_none_or(|reset| reset > stored.observed_at))))
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
    let temporary = directory.join(format!(
        ".{}.{}",
        name.display(),
        super::supervisor::request_id()
    ));
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

/// The CLI and shared schedule's failure reason (EON-C27).
pub(crate) fn failure_reason(reason: Failure) -> &'static str {
    match reason {
        Failure::Start => "start",
        Failure::Deadline => "deadline",
        Failure::OutputLimit => "output_limit",
        Failure::ExitStatus => "exit_status",
        Failure::Malformed => "malformed",
        Failure::ProviderError => "provider_error",
        Failure::Estimated => "estimated",
        Failure::ObservationTime => "observation_time",
        Failure::InvalidWindow => "invalid_window",
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFailure {
    at: u64,
    reason: String,
}

fn save_failure<S: serde::Serializer>(
    failure: &Option<QuotaFailure>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    failure
        .map(|f| StoredFailure {
            at: f.at,
            reason: failure_reason(f.reason).to_owned(),
        })
        .serialize(serializer)
}

fn load_failure<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<QuotaFailure>, D::Error> {
    Option::<StoredFailure>::deserialize(deserializer)?
        .map(|f| {
            Ok(QuotaFailure {
                at: f.at,
                reason: match f.reason.as_str() {
                    "start" => Failure::Start,
                    "deadline" => Failure::Deadline,
                    "output_limit" => Failure::OutputLimit,
                    "exit_status" => Failure::ExitStatus,
                    "malformed" => Failure::Malformed,
                    "provider_error" => Failure::ProviderError,
                    "estimated" => Failure::Estimated,
                    "observation_time" => Failure::ObservationTime,
                    "invalid_window" => Failure::InvalidWindow,
                    _ => return Err(serde::de::Error::custom("unknown quota failure")),
                },
            })
        })
        .transpose()
}

/// A shared provider read schedule; Codex keeps its schedule in memory (EON-C27).
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Schedule {
    version: u32,
    next_at: u64,
    failures: u32,
    #[serde(serialize_with = "save_failure", deserialize_with = "load_failure")]
    failure: Option<QuotaFailure>,
    #[serde(skip)]
    write_failed: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            next_at: 0,
            failures: 0,
            failure: None,
            write_failed: false,
        }
    }
}

fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// How a read finished: the failure it reports or clears (None leaves the last one), whether
/// it succeeded, and whether it backs off (a provider error on a shared schedule).
#[derive(Clone, Copy)]
struct Finished {
    outcome: Option<Option<QuotaFailure>>,
    success: bool,
    back_off: bool,
}

/// Records a finished read in the schedule at `now` (ms): its failure, the provider-error count,
/// which only a success resets, and when the next read may start.
fn record_read(schedule: &mut Schedule, now: u64, read: Finished, interval: Duration, jitter: f64) {
    if let Some(outcome) = read.outcome {
        schedule.failure = outcome;
    }
    if read.success {
        schedule.failures = 0;
    } else if read.back_off {
        schedule.failures = schedule.failures.saturating_add(1);
    }
    let delay = if read.back_off {
        provider_error_wait(interval, schedule.failures, jitter)
    } else {
        interval
    };
    schedule.next_at = now.saturating_add(milliseconds(delay));
}

/// The wait after `failures` consecutive provider errors on a shared schedule (EON-C27): the
/// first since a success retries after one interval plus up to a fifth more by `jitter`, in
/// `[0, 1)` (five to six minutes for Claude); later ones wait 6, 12, then 24 intervals
/// (30 minutes, 1 hour, then 2 hours).
fn provider_error_wait(interval: Duration, failures: u32, jitter: f64) -> Duration {
    match failures {
        0 | 1 => interval + interval.mul_f64(jitter.clamp(0.0, 1.0) / 5.0),
        2 => interval * 6,
        3 => interval * 12,
        _ => interval * 24,
    }
}

/// A fraction in `[0, 1)` that keeps a retry out of step with other pollers.
fn jitter() -> f64 {
    use std::hash::BuildHasher;
    let random = std::collections::hash_map::RandomState::new().hash_one(Instant::now());
    (random >> 11) as f64 / (1u64 << 53) as f64
}

/// The longest backoff; a schedule further ahead is ignored (EON-C27).
fn longest_wait(interval: Duration) -> Duration {
    provider_error_wait(interval, u32::MAX, 0.0)
}

fn open_schedule(path: &Path, create: bool) -> io::Result<fs::File> {
    if create {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(
            path.parent()
                .ok_or_else(|| io::Error::other("no quota directory"))?,
        )?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a quota schedule file"));
    }
    // Peers hold the lock only to rewrite this file; one that stays stopped holding it for a
    // second leaves this worker on its in-memory schedule rather than blocked (EON-C27).
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn load_schedule(file: &mut fs::File, now: u64, maximum: Duration) -> Option<Schedule> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let schedule: Schedule = serde_json::from_slice(&bytes).ok()?;
    (schedule.version == STORE_VERSION
        && schedule.failure.is_none_or(|failure| failure.at > 0)
        && schedule.next_at.saturating_sub(now) <= milliseconds(maximum))
    .then_some(schedule)
}

fn write_schedule(file: &mut fs::File, schedule: &mut Schedule) -> io::Result<()> {
    let written = (|| -> io::Result<()> {
        let bytes = serde_json::to_vec(&*schedule)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&bytes)
    })();
    // A failed rewrite can leave a partial file; keep this slot's in-memory backoff (EON-C27).
    schedule.write_failed = written.is_err();
    written
}

/// Reads and updates under one short lock, never held during collection (EON-C27).
fn update_schedule(
    memory: &Mutex<Schedule>,
    path: Option<&Path>,
    maximum: Duration,
    clock: &Clock,
    create: bool,
    // None suppresses all writes, including retries of an old failed write.
    update: impl FnOnce(&mut Schedule, u64, bool) -> Option<bool>,
) -> (Schedule, bool) {
    let mut memory = memory.lock().expect("quota schedule");
    let mut file = path.and_then(|path| open_schedule(path, create).ok());
    let now = clock.now_ms();
    // Without a usable file the in-memory schedule stands (EON-C27).
    let loaded = file
        .as_mut()
        .and_then(|file| load_schedule(file, now, maximum));
    let shared_usable = loaded.is_some();
    if let Some(schedule) = loaded {
        *memory = schedule;
    }
    // A clock set back never stops reads, with or without a file.
    if memory.next_at.saturating_sub(now) > milliseconds(maximum) {
        memory.next_at = now;
    }
    if update(&mut memory, now, shared_usable).is_some_and(|changed| changed || memory.write_failed)
        && let Some(locked) = &mut file
        && write_schedule(locked, &mut memory).is_err()
    {
        file = None;
    }
    (memory.clone(), file.is_some())
}

/// Where a provider worker reads the schedule's time and waits for its next read: the system
/// clock, or in tests a manual one that moves only when the test advances it.
#[derive(Clone)]
enum Clock {
    System,
    #[cfg(test)]
    Manual(Arc<tests::ManualClock>),
}

#[derive(Clone, Copy)]
enum Resume {
    System(Instant),
    #[cfg(test)]
    Manual(u64),
}

impl Clock {
    fn resume(&self) -> Resume {
        match self {
            Self::System => Resume::System(Instant::now()),
            #[cfg(test)]
            Self::Manual(clock) => Resume::Manual(clock.now_ms()),
        }
    }

    fn elapsed(&self, resume: Resume) -> Duration {
        match resume {
            Resume::System(start) => start.elapsed(),
            #[cfg(test)]
            Resume::Manual(start) => Duration::from_millis(self.now_ms().saturating_sub(start)),
        }
    }

    /// Milliseconds since the epoch.
    fn now_ms(&self) -> u64 {
        match self {
            Self::System => milliseconds(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default(),
            ),
            #[cfg(test)]
            Self::Manual(clock) => clock.now_ms(),
        }
    }

    /// Enrols a worker before its thread starts; the worker waits through the returned
    /// `Parking`, and dropping it withdraws the worker.
    fn register(&self) -> Parking {
        match self {
            Self::System => Parking::System,
            #[cfg(test)]
            Self::Manual(clock) => Parking::Manual(Arc::clone(clock), clock.register()),
        }
    }

    /// Wakes the workers waiting on this clock to see a stop just sent.
    fn wake(&self) {
        #[cfg(test)]
        if let Self::Manual(clock) = self {
            clock.wake();
        }
    }
}

/// A worker's place on its clock.
enum Parking {
    System,
    #[cfg(test)]
    Manual(Arc<tests::ManualClock>, u64),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Park {
    #[default]
    Running,
    Requested,
    Parked,
}

fn stopped(stop: &mpsc::Receiver<()>) -> bool {
    stop.try_recv() != Err(mpsc::TryRecvError::Empty)
}

impl Parking {
    /// Returns on a scheduled wake or park request, and returns true on stop.
    fn wait(&self, stop: &mpsc::Receiver<()>, park: &Mutex<Park>, wait: Duration) -> bool {
        match self {
            Self::System => {
                let deadline = Instant::now() + wait;
                loop {
                    if stopped(stop) {
                        return true;
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() || *park.lock().expect("quota park") != Park::Running {
                        return false;
                    }
                    thread::park_timeout(left);
                }
            }
            #[cfg(test)]
            Self::Manual(clock, id) => clock.wait(*id, stop, park, wait),
        }
    }
}

#[cfg(test)]
impl Drop for Parking {
    fn drop(&mut self) {
        if let Self::Manual(clock, id) = self {
            clock.withdraw(*id);
        }
    }
}

struct Provider {
    quota: Arc<Mutex<Option<CodexQuota>>>,
    /// The last read's failure, until a read succeeds.
    failure: Arc<Mutex<Option<QuotaFailure>>>,
    stop: mpsc::Sender<()>,
    clock: Clock,
    park: Arc<Mutex<Park>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Provider {
    #[expect(
        clippy::too_many_arguments,
        reason = "a slot's settings plus the program, store and clock each worker needs"
    )]
    fn start(
        program: PathBuf,
        provider: &'static str,
        interval: Duration,
        timeout: Duration,
        schedule_memory: Arc<Mutex<Schedule>>,
        store: Option<PathBuf>,
        shared_schedule: bool,
        clock: Clock,
        resume: Option<Resume>,
    ) -> Self {
        let quota = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&quota);
        let failure = Arc::new(Mutex::new(None));
        let failed = Arc::clone(&failure);
        let (stop, stopping) = mpsc::channel();
        let parking = clock.register();
        let worker_clock = clock.clone();
        let phase = Arc::new(Mutex::new(Park::Running));
        let park = Arc::clone(&phase);
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
                let schedule_path = store
                    .as_deref()
                    .filter(|_| shared_schedule)
                    .and_then(Path::parent)
                    .map(|dir| dir.join(format!("{provider}.schedule")));
                let maximum = longest_wait(interval);
                loop {
                    if stopped(&stopping) {
                        break;
                    }
                    {
                        let mut phase = park.lock().expect("quota park");
                        if *phase != Park::Running {
                            *phase = Park::Parked;
                            drop(phase);
                            thread::park();
                            continue;
                        }
                    }
                    let floor = resume.map_or(Duration::ZERO, |resume| {
                        interval.saturating_sub(worker_clock.elapsed(resume))
                    });
                    let mut wait = Duration::ZERO;
                    let mut reserved = false;
                    let (schedule, schedule_usable) = update_schedule(
                        &schedule_memory,
                        schedule_path.as_deref(),
                        maximum,
                        &worker_clock,
                        floor.is_zero(),
                        |schedule, now, loaded| {
                            // Hold the same lock as the park request until
                            // either this cycle is reserved or park wins.
                            let phase = park.lock().expect("quota park");
                            if *phase != Park::Running {
                                return None;
                            }
                            wait = Duration::from_millis(schedule.next_at.saturating_sub(now));
                            if !loaded && !floor.is_zero() {
                                wait = wait.max(floor);
                                if shared_schedule {
                                    wait = wait.min(interval / 30);
                                }
                                return None;
                            }
                            if !wait.is_zero() {
                                return Some(false);
                            }
                            schedule.next_at = now.saturating_add(milliseconds(interval));
                            reserved = true;
                            Some(true)
                        },
                    );
                    if !reserved {
                        if wait.is_zero() {
                            continue;
                        }
                        if schedule_usable {
                            if let Ok(mut last) = failed.lock() {
                                *last = schedule.failure;
                            }
                            if let Some(path) = &store
                                && let Some(now) = epoch_seconds()
                                && let Ok(mut current) = shared.lock()
                            {
                                if let Some(stored) = load(path, provider, now) {
                                    if current
                                        .as_ref()
                                        .is_none_or(|q| stored.observed_at > q.observed_at)
                                    {
                                        *current = Some(stored);
                                    }
                                } else if schedule.failure.is_none()
                                    && fs::metadata(path)
                                        .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
                                {
                                    *current = None;
                                }
                            }
                            wait = wait.min(interval / 30);
                        }
                        if parking.wait(&stopping, &park, wait) {
                            break;
                        }
                        continue;
                    }
                    let fetched = fetch(&program, provider, timeout, &stopping);
                    let reading = match fetched {
                        // A stopped read is spaced from its end, as a finished one is.
                        Ok(None) => {
                            update_schedule(
                                &schedule_memory,
                                schedule_path.as_deref(),
                                maximum,
                                &worker_clock,
                                true,
                                |schedule, now, _| {
                                    schedule.next_at = now.saturating_add(milliseconds(interval));
                                    Some(true)
                                },
                            );
                            break;
                        }
                        Ok(Some(bytes)) => epoch_seconds()
                            .ok_or(Failure::ObservationTime)
                            .and_then(|now| {
                                if provider == "google" {
                                    google::normalize(&bytes, now).map(|quota| {
                                        quota.map(|quota| CodexQuota {
                                            state: CodexQuotaState::Fresh,
                                            observed_at: quota.observed_at,
                                            windows: quota.windows,
                                        })
                                    })
                                } else {
                                    normalize(&bytes, provider, now)
                                }
                            }),
                        Err(reason) => Err(reason),
                    };
                    let success = reading.is_ok();
                    let back_off =
                        shared_schedule && matches!(reading, Err(Failure::ProviderError));
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
                    update_schedule(
                        &schedule_memory,
                        schedule_path.as_deref(),
                        maximum,
                        &worker_clock,
                        true,
                        |schedule, now, _| {
                            record_read(
                                schedule,
                                now,
                                Finished {
                                    outcome,
                                    success,
                                    back_off,
                                },
                                interval,
                                jitter(),
                            );
                            Some(true)
                        },
                    );
                }
            })
            .ok();
        Self {
            quota,
            failure,
            stop,
            clock,
            park: phase,
            worker,
        }
    }

    fn snapshot(&self) -> Option<CodexQuota> {
        present_at(self.quota.lock().ok()?.clone()?, epoch_seconds()?)
    }

    fn failure(&self) -> Option<QuotaFailure> {
        *self.failure.lock().ok()?
    }

    fn wake(&self) {
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        self.clock.wake();
    }

    fn set_park(&self, parked: bool) {
        let mut phase = self.park.lock().expect("quota park");
        match (*phase, parked) {
            (Park::Running, true) => *phase = Park::Requested,
            (_, false) => *phase = Park::Running,
            _ => return,
        }
        drop(phase);
        self.wake();
    }

    fn parked(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
            || *self.park.lock().expect("quota park") == Park::Parked
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        self.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One provider's worker while its chip is shown, and its retained schedule (EON-C27).
struct Slot {
    provider: &'static str,
    widgets: &'static [&'static str],
    interval: Duration,
    shared_schedule: bool,
    schedule_memory: Arc<Mutex<Schedule>>,
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
    google_program: PathBuf,
    /// Where each provider's last good reading is kept, if anywhere.
    store: Option<PathBuf>,
    /// The clock every provider worker schedules its reads by.
    clock: Clock,
    resume: Option<Resume>,
    codex: Slot,
    claude: Slot,
    google: Slot,
}

impl Quotas {
    pub(crate) fn start(program: &Path, widgets: &[String]) -> Self {
        Self::start_every(program, UNIT, store_directory(), widgets)
    }

    pub(crate) fn resume(program: &Path, widgets: &[String]) -> Self {
        Self::start_with_clock(
            program,
            Path::new("agy"),
            UNIT,
            store_directory(),
            widgets,
            Clock::System,
            true,
        )
    }

    /// Reads Codex every 60 units and Claude/Google every 300 (EON-C27).
    fn start_every(
        program: &Path,
        unit: Duration,
        store: Option<PathBuf>,
        widgets: &[String],
    ) -> Self {
        Self::start_with_google(program, Path::new("agy"), unit, store, widgets)
    }

    fn start_with_google(
        program: &Path,
        google_program: &Path,
        unit: Duration,
        store: Option<PathBuf>,
        widgets: &[String],
    ) -> Self {
        Self::start_with_clock(
            program,
            google_program,
            unit,
            store,
            widgets,
            Clock::System,
            false,
        )
    }

    fn start_with_clock(
        program: &Path,
        google_program: &Path,
        unit: Duration,
        store: Option<PathBuf>,
        widgets: &[String],
        clock: Clock,
        resumed: bool,
    ) -> Self {
        let slot = |provider, widgets, units, shared_schedule| Slot {
            provider,
            widgets,
            interval: unit * units,
            shared_schedule,
            schedule_memory: Arc::default(),
            worker: None,
        };
        let mut quotas = Self {
            program: program.to_path_buf(),
            google_program: google_program.to_path_buf(),
            store,
            resume: resumed.then(|| clock.resume()),
            clock,
            codex: slot(
                "codex",
                &["codex_quota_5h", "codex_quota_7d"],
                CODEX_UNITS,
                false,
            ),
            claude: slot(
                "claude",
                &["claude_quota_5h", "claude_quota_7d", "claude_quota_scoped"],
                SHARED_UNITS,
                true,
            ),
            google: slot("google", &[], SHARED_UNITS, true),
        };
        quotas.show(widgets);
        quotas
    }

    fn workers(&self) -> impl Iterator<Item = &Provider> {
        [&self.codex, &self.claude, &self.google]
            .into_iter()
            .filter_map(|slot| slot.worker.as_ref())
    }

    pub(crate) fn set_park(&self, parked: bool) {
        for worker in self.workers() {
            worker.set_park(parked);
        }
    }

    pub(crate) fn parked(&self) -> bool {
        self.workers().all(Provider::parked)
    }

    /// The providers polled now.
    #[cfg(test)]
    pub(crate) fn polled(&self) -> Vec<&'static str> {
        [&self.codex, &self.claude, &self.google]
            .into_iter()
            .filter(|slot| slot.worker.is_some())
            .map(|slot| slot.provider)
            .collect()
    }

    /// Polls exactly the providers whose chips `[bar] widgets` lists (EON-C27).
    pub(crate) fn show(&mut self, widgets: &[String]) {
        for slot in [&mut self.codex, &mut self.claude, &mut self.google] {
            if !widgets.iter().any(|widget| {
                slot.widgets.contains(&widget.as_str())
                    || (slot.provider == "google"
                        && super::managed_environment::google_quota_widget(widget))
            }) {
                slot.worker = None;
            } else if slot.worker.is_none() {
                slot.worker = Some(Provider::start(
                    if slot.provider == "google" {
                        self.google_program.clone()
                    } else {
                        self.program.clone()
                    },
                    slot.provider,
                    slot.interval,
                    Duration::from_secs(10),
                    Arc::clone(&slot.schedule_memory),
                    self.store
                        .as_ref()
                        .map(|store| store.join(format!("{}.json", slot.provider))),
                    slot.shared_schedule,
                    self.clock.clone(),
                    self.resume,
                ));
            }
        }
    }

    pub(crate) fn apply(&self, snapshot: &mut Snapshot) {
        snapshot.google_quota = self.google.snapshot().map(|quota| GoogleQuota {
            state: if quota.state == CodexQuotaState::Stale {
                GoogleQuotaState::Stale
            } else {
                GoogleQuotaState::Fresh
            },
            observed_at: quota.observed_at,
            windows: quota.windows,
        });
        snapshot.google_quota_failure = self.google.failure();
        snapshot.codex_quota = self.codex.snapshot();
        snapshot.codex_quota_failure = self.codex.failure();
        snapshot.claude_quota_failure = self.claude.failure();
        snapshot.claude_quota = self.claude.snapshot().map(claude_quota);
    }
}

/// Where each provider's last good reading is kept, shared by every window (EON-C27).
pub(crate) fn store_directory() -> Option<PathBuf> {
    super::diagnostics::state_directory().map(|state| state.join("quota"))
}

/// A Claude reading as windows present it.
pub(crate) fn claude_quota(quota: CodexQuota) -> ClaudeQuota {
    ClaudeQuota {
        state: if quota.state == CodexQuotaState::Stale {
            ClaudeQuotaState::Stale
        } else {
            ClaudeQuotaState::Fresh
        },
        observed_at: quota.observed_at,
        windows: quota.windows,
    }
}

/// One provider's facts from the store in `store`, for an `eon agent` caller whose window does
/// not supply them (EON-C40): the stored reading as a window presents it and the failure a
/// window would report beside it. Claude's failure comes from its shared schedule; Codex keeps
/// none, so its reading is used only while at most two reads old. Nothing is created, written,
/// locked exclusively or removed, and no provider is called; any doubt refuses.
pub(crate) fn stored_facts(
    store: &Path,
    provider: &str,
    deadline: Instant,
) -> Result<(CodexQuota, Option<QuotaFailure>), String> {
    let now = epoch_seconds().ok_or("the system clock is before 1970")?;
    let path = store.join(format!("{provider}.json"));
    let stored = load(&path, provider, now).ok_or_else(|| {
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                format!("no stored reading at {}", path.display())
            }
            _ => format!(
                "the stored reading at {} is unreadable, malformed, of another version or from the future",
                path.display()
            ),
        }
    })?;
    let age = now - stored.observed_at;
    let quota = present_at(stored, now)
        .ok_or_else(|| format!("the stored reading at {} has expired", path.display()))?;
    if provider == "claude" {
        return Ok((
            quota,
            schedule_failure(&store.join("claude.schedule"), deadline)?,
        ));
    }
    if age > STORED_CODEX_MAX_AGE {
        return Err(format!(
            "the stored reading is {age} s old; Codex records no failure, so only one at most {STORED_CODEX_MAX_AGE} s old is used"
        ));
    }
    Ok((quota, None))
}

/// The failure a shared schedule records, read under a shared lock without creating or
/// rewriting the file; a file that cannot be locked, read or validated leaves it unknown.
fn schedule_failure(path: &Path, deadline: Instant) -> Result<Option<QuotaFailure>, String> {
    let unknown = |why: String| format!("failure state unknown: {why} ({})", path.display());
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| unknown(error.to_string()))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(unknown("not a schedule file".into()));
    }
    // A peer holds the lock only to rewrite the file (EON-C27): retry briefly, never past the
    // reader's deadline.
    let pause = Duration::from_millis(20);
    let mut retries = 3;
    loop {
        match file.try_lock_shared() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock)
                if retries > 0 && Instant::now() + pause < deadline =>
            {
                retries -= 1;
                thread::sleep(pause);
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(unknown("a window is rewriting it".into()));
            }
            Err(fs::TryLockError::Error(error)) => return Err(unknown(error.to_string())),
        }
    }
    load_schedule(
        &mut file,
        Clock::System.now_ms(),
        longest_wait(UNIT * SHARED_UNITS),
    )
    .map(|schedule| schedule.failure)
    .ok_or_else(|| unknown("unreadable or invalid".into()))
}

fn quota_command(program: &Path, provider: &str) -> Command {
    let mut command = Command::new(program);
    super::managed_environment::remove_agent_session(&mut command)
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
        .process_group(0);
    command
}

fn fetch(
    program: &Path,
    provider: &str,
    timeout: Duration,
    stop: &mpsc::Receiver<()>,
) -> Result<Option<Vec<u8>>, Failure> {
    if provider == "google" {
        return google::fetch(program, timeout, stop);
    }
    let mut remaining = MAX_BYTES;
    run_command(quota_command(program, provider), Instant::now() + timeout, &mut remaining, stop)?
        .map(|(bytes, status)| {
            if status.success() {
                return Ok(bytes);
            }
            let reported_error = serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| {
                matches!(v.as_array().map(Vec::as_slice), Some([result]) if !result["error"].is_null())
            });
            Err(if reported_error { Failure::ProviderError } else { Failure::ExitStatus })
        })
        .transpose()
}

type CommandResult = Result<Option<(Vec<u8>, process::ExitStatus)>, Failure>;

fn consume_output(remaining: &mut usize, count: usize) -> Result<(), Failure> {
    *remaining = remaining.checked_sub(count).ok_or(Failure::OutputLimit)?;
    Ok(())
}

/// Drains bounded output and always cleans up the command's process group.
fn run_command(
    mut command: Command,
    deadline: Instant,
    remaining: &mut usize,
    stop: &mpsc::Receiver<()>,
) -> CommandResult {
    use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
    if stopped(stop) {
        return Ok(None);
    }
    if Instant::now() >= deadline {
        return Err(Failure::Deadline);
    }
    crate::platform::require_waitable_children().map_err(|_| Failure::Start)?;
    let mut child = command.spawn().map_err(|_| Failure::Start)?;
    let pid = Pid::from_child(&child);
    let check = || loop {
        let flags = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
        match waitid(WaitId::Pid(pid), flags) {
            Err(rustix::io::Errno::INTR) => {}
            result => break result.map_err(|_| Failure::Start),
        }
    };
    let mut output = child.stdout.take().expect("piped quota output");
    let result = (|| {
        let flags = rustix::fs::fcntl_getfl(&output).map_err(|_| Failure::Start)?;
        rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| Failure::Start)?;
        let mut bytes = Vec::new();
        let mut eof = false;
        loop {
            if stopped(stop) {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Deadline);
            }
            let mut chunk = [0; 8192];
            match output.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(count) => {
                    consume_output(remaining, count)?;
                    bytes.extend_from_slice(&chunk[..count]);
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(Failure::Start),
            }
            if eof && check()?.is_some() {
                return Ok(Some(bytes));
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    // Observe without reaping so no other process can take this group ID
    // before cleanup. ECHILD (or any failed identity check) forbids a signal.
    if check().is_ok() {
        let _ = kill_process_group(pid, Signal::KILL);
    }
    let status = child.wait().map_err(|_| Failure::Start)?;
    result.map(|bytes| bytes.map(|bytes| (bytes, status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{collections::HashMap, sync::Condvar};

    const NOW: u64 = 1791936000;
    #[test]
    fn google_only_widgets_start_no_existing_provider_workers() {
        let widgets = [
            "google_quota:Gemini Models:5h".to_owned(),
            "google_quota:A,B:C:7d".to_owned(),
        ];
        let quotas = Quotas::start_with_google(
            Path::new("/absent/codexbar"),
            Path::new("/absent/agy"),
            Duration::from_secs(1),
            None,
            &widgets,
        );
        assert!(quotas.codex.worker.is_none());
        assert!(quotas.claude.worker.is_none());
    }

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
    pub(super) fn script(path: &Path, body: &str) {
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
    fn quota_reads_carry_no_terminal_or_agent_session() {
        for provider in ["codex", "claude"] {
            let command = quota_command(Path::new("/managed/codexbar"), provider);
            for name in ["EON_TERMINAL", "EON_TAB", "CLAUDE_CODE_MESSAGING_TOKEN"] {
                assert!(
                    command
                        .get_envs()
                        .any(|(key, value)| key == name && value.is_none()),
                    "{provider} {name}"
                );
            }
        }
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
            &["codex_quota_5h", "claude_quota_5h"].map(String::from),
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
            false,
            Clock::System,
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
        let program = root.join("refused");
        script(
            &program,
            "printf '%s' '[{\"provider\":\"claude\",\"source\":\"oauth\",\"error\":{\"code\":3,\"message\":\"Claude OAuth usage endpoint is rate limited by Anthropic right now.\",\"kind\":\"provider\"}}]'; exit 3",
        );
        assert_eq!(
            fetch(&program, "claude", Duration::from_secs(1), &stopped),
            Err(Failure::ProviderError)
        );
        for (index, (output, reason)) in [
            ("not json", Failure::ExitStatus),
            ("[null]", Failure::ExitStatus),
            ("[{\"error\":null}]", Failure::ExitStatus),
            ("[{\"error\":{}},{}]", Failure::ExitStatus),
            ("[{\"error\":false}]", Failure::ProviderError),
        ]
        .into_iter()
        .enumerate()
        {
            let program = root.join(format!("exit-shape-{index}"));
            script(&program, &format!("printf '%s' '{output}'; exit 3"));
            assert_eq!(
                fetch(&program, "claude", Duration::from_secs(1), &stopped),
                Err(reason)
            );
        }
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
            &["codex_quota_5h", "claude_quota_5h"].map(String::from),
        );
        let mut snapshot = Snapshot {
            focus: None,
            input_lock: crate::fixtures::input_lock(),
            input_locked: false,
            released_shortcuts: Default::default(),
            generation: "g1-00000000000000000000000000000001".into(),
            created_build: "g1-00000000000000000000000000000001".into(),
            active_build: "g1-00000000000000000000000000000001".into(),
            workspace: String::new(),
            active_workspace: String::new(),
            workspaces: vec![],
            active_tab: String::new(),
            geometry: eon_workspace_protocol::v50::PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: vec![],
            tabs: vec![],
            codex_quota: None,
            claude_quota: None,
            google_quota: None,
            google_quota_failure: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v50::Sidebar::Off,
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

    /// Virtual time for the provider workers of one `Quotas`: it moves only when a test calls
    /// `advance_to`, and `settle` returns once every worker has evaluated its schedule at the
    /// current time and parked, so no read can start until the next advance.
    pub(super) struct ManualClock {
        state: Mutex<ManualState>,
        changed: Condvar,
    }

    struct ManualState {
        now: u64,
        next_id: u64,
        /// Each live worker, and the deadline it is parked until (None while it runs).
        workers: HashMap<u64, Option<u64>>,
    }

    impl ManualClock {
        fn starting_at(now: u64) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(ManualState {
                    now,
                    next_id: 0,
                    workers: HashMap::new(),
                }),
                changed: Condvar::new(),
            })
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, ManualState> {
            self.state.lock().expect("manual clock")
        }

        pub(super) fn now_ms(&self) -> u64 {
            self.lock().now
        }

        /// A new worker counts as running from before its thread starts.
        pub(super) fn register(&self) -> u64 {
            let mut state = self.lock();
            let id = state.next_id;
            state.next_id += 1;
            state.workers.insert(id, None);
            id
        }

        pub(super) fn withdraw(&self, id: u64) {
            self.lock().workers.remove(&id);
            self.changed.notify_all();
        }

        pub(super) fn wake(&self) {
            let _state = self.lock();
            self.changed.notify_all();
        }

        /// Parks worker `id` until the clock reaches `wait` from now (false) or a stop arrives
        /// (true). The stop channel is checked under the lock `wake` takes, so none is lost.
        pub(super) fn wait(
            &self,
            id: u64,
            stop: &mpsc::Receiver<()>,
            park: &Mutex<Park>,
            wait: Duration,
        ) -> bool {
            let mut state = self.lock();
            let deadline = state.now + milliseconds(wait).max(1);
            state.workers.insert(id, Some(deadline));
            self.changed.notify_all();
            loop {
                let stop = stopped(stop);
                if stop
                    || state.now >= deadline
                    || *park.lock().expect("quota park") != Park::Running
                {
                    state.workers.insert(id, None);
                    return stop;
                }
                state = self.changed.wait(state).expect("manual clock");
            }
        }

        fn advance_to(&self, now: u64) {
            let mut state = self.lock();
            assert!(now >= state.now, "the manual clock only moves forward");
            state.now = now;
            self.changed.notify_all();
        }

        /// Waits until every live worker is parked past the current time.
        fn settle(&self) {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut state = self.lock();
            loop {
                let now = state.now;
                if state
                    .workers
                    .values()
                    .all(|parked| parked.is_some_and(|until| until > now))
                {
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "workers never settled at {now}: {:?}",
                    state.workers
                );
                state = self
                    .changed
                    .wait_timeout(state, left)
                    .expect("manual clock")
                    .0;
            }
        }
    }

    /// A backend at `root`/backend that logs each read's provider to `root`/calls, then runs
    /// `body`.
    fn logging_backend(root: &Path, body: &str) {
        let calls = root.join("calls");
        script(
            &root.join("backend"),
            &format!("echo \"$3\" >> '{}'\n{body}", calls.display()),
        );
    }

    /// Workers on a manual clock at the production unit, reading through `logging_backend`, and
    /// a count of (Claude, Codex) reads since this start.
    fn virtual_quotas(
        root: &Path,
        store: Option<PathBuf>,
        widgets: &[String],
    ) -> (
        Arc<ManualClock>,
        Quotas,
        impl Fn() -> (usize, usize) + use<>,
    ) {
        let calls = root.join("calls");
        let _ = fs::remove_file(&calls);
        let clock = ManualClock::starting_at(T0);
        let quotas = Quotas::start_with_clock(
            &root.join("backend"),
            Path::new("/absent/agy"),
            UNIT,
            store,
            widgets,
            Clock::Manual(Arc::clone(&clock)),
            false,
        );
        let reads = move || {
            let calls = fs::read_to_string(&calls).unwrap_or_default();
            let count = |provider| calls.lines().filter(|line| *line == provider).count();
            (count("claude"), count("codex"))
        };
        (clock, quotas, reads)
    }

    /// The manual clock's start, in milliseconds.
    const T0: u64 = NOW * 1000;

    #[test]
    fn update_resume_floor_uses_local_time_and_leaves_unusable_schedules_untouched() {
        let root = super::super::supervisor::temporary_directory();
        let store = root.join("quota");
        fs::create_dir(&store).unwrap();
        let claude = store.join("claude.schedule");
        let google = store.join("google.schedule");
        fs::write(&claude, b"partial").unwrap();
        logging_backend(&root, "echo '{}'");
        script(
            &root.join("agy"),
            &format!(
                "echo google >> '{}'; echo '{{}}'",
                root.join("calls").display()
            ),
        );
        let clock = ManualClock::starting_at(T0);
        let mut quotas = Quotas::start_with_clock(
            &root.join("backend"),
            &root.join("agy"),
            UNIT,
            Some(store),
            &[
                "claude_quota_5h".into(),
                "google_quota:Gemini Models:5h".into(),
            ],
            Clock::Manual(Arc::clone(&clock)),
            true,
        );
        let calls = || fs::read_to_string(root.join("calls")).unwrap_or_default();
        let advance = |seconds| {
            clock.advance_to(T0 + seconds * 1_000);
            clock.settle();
        };
        advance(20u64);
        // A widget enabled after import shares the original import anchor.
        quotas.show(&[
            "codex_quota_5h".into(),
            "claude_quota_5h".into(),
            "google_quota:Gemini Models:5h".into(),
        ]);
        advance(59);
        assert_eq!(calls(), "");
        assert_eq!(fs::read(&claude).unwrap(), b"partial");
        assert!(!google.exists());
        advance(60);
        assert_eq!(calls(), "codex\n");
        fs::write(
            &claude,
            serde_json::to_vec(&Schedule {
                next_at: T0 + 70_000,
                ..Schedule::default()
            })
            .unwrap(),
        )
        .unwrap();
        advance(70);
        assert_eq!(calls(), "codex\nclaude\n");
        advance(299);
        assert!(!google.exists());
        assert!(!calls().contains("google"));
        advance(300);
        assert_eq!(calls().lines().filter(|line| *line == "google").count(), 1);
        drop(quotas);
        fs::remove_dir_all(root).unwrap();
    }

    /// Advances to each `(seconds after T0, Claude reads, Codex reads)` and checks the counts
    /// once every worker has settled there.
    fn expect_reads(
        clock: &ManualClock,
        reads: &impl Fn() -> (usize, usize),
        steps: &[(u64, usize, usize)],
    ) {
        for &(at, claude, codex) in steps {
            clock.advance_to(T0 + at * 1000);
            clock.settle();
            assert_eq!(
                reads(),
                (claude, codex),
                "(Claude, Codex) reads at T0+{at} s"
            );
        }
    }

    #[test]
    fn claude_retries_once_soon_then_backs_off_without_a_usable_schedule() {
        let root = super::super::supervisor::temporary_directory();
        let blocked = root.join("not-a-directory");
        fs::write(&blocked, "").unwrap();
        let widgets = ["codex_quota_5h", "claude_quota_5h"].map(String::from);
        // Every read of either provider is a provider error, as a rate-limited read is.
        logging_backend(&root, "printf '%s' '[{\"error\":{\"code\":3}}]'; exit 3");
        for store in [None, Some(blocked.clone())] {
            let (clock, mut quotas, reads) = virtual_quotas(&root, store.clone(), &widgets);
            clock.settle();
            assert_eq!(reads(), (1, 1), "{store:?}");
            assert_eq!(
                quotas.claude.failure().unwrap().reason,
                Failure::ProviderError
            );
            // Hidden and shown again, both keep their in-memory spacing.
            quotas.show(&[]);
            clock.settle();
            quotas.show(&widgets);
            clock.settle();
            assert_eq!(reads(), (1, 1));
            // Claude retries once within one 300 s interval plus a fifth of jitter, then waits
            // 6 and 12 intervals; Codex reads every 60 s through the same provider errors.
            expect_reads(
                &clock,
                &reads,
                &[
                    (59, 1, 1),
                    (60, 1, 2),
                    (120, 1, 3),
                    (299, 1, 4),
                    (360, 2, 5),
                    (2159, 2, 6),
                    (2160, 3, 6),
                    (5759, 3, 7),
                    (5760, 4, 7),
                ],
            );
            // With every chip hidden, nothing is read, though Codex is due.
            quotas.show(&[]);
            expect_reads(&clock, &reads, &[(9360, 4, 7)]);
            // Shown again, Codex is due; Claude still waits 24 intervals from its last read.
            quotas.show(&widgets);
            clock.settle();
            assert_eq!(reads(), (4, 8));
            drop(quotas);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_failures_other_than_provider_errors_keep_the_300_unit_interval() {
        let root = super::super::supervisor::temporary_directory();
        let (codex, _) = fresh("codex");
        // Claude's read fails without a provider error; Codex's succeeds.
        logging_backend(
            &root,
            &format!("[ \"$3\" = codex ] && printf '%s' '{codex}'"),
        );
        let (clock, mut quotas, reads) = virtual_quotas(
            &root,
            None,
            &["codex_quota_5h", "claude_quota_5h"].map(String::from),
        );
        clock.settle();
        assert_eq!(reads(), (1, 1));
        assert_eq!(quotas.claude.failure().unwrap().reason, Failure::ExitStatus);
        assert!(quotas.codex.failure().is_none());
        expect_reads(&clock, &reads, &[(299, 1, 2), (300, 2, 2)]);
        // Switched from the 5h to the 7d chip while hidden, Claude still waits 300 s from its
        // last read.
        quotas.show(&["codex_quota_5h"].map(String::from));
        expect_reads(&clock, &reads, &[(359, 2, 3)]);
        quotas.show(&["codex_quota_5h", "claude_quota_7d"].map(String::from));
        expect_reads(&clock, &reads, &[(359, 2, 3), (599, 2, 4), (600, 3, 4)]);
        drop(quotas);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_is_read_once_across_supervisors_and_backs_off_after_provider_errors() {
        let root = super::super::supervisor::temporary_directory();
        let program = root.join("backend");
        let mode = root.join("mode");
        let calls = root.join("calls");
        let store = root.join("quota");
        fs::write(&mode, "ok").unwrap();
        let (fixture, _) = fresh("claude");
        script(
            &program,
            &format!(
                "[ \"$3\" = claude ] || exit 1\nmode=$(cat '{}')\n: > '{}'/\"$mode.$$\"\nprintf '%s %s\\n' \"$mode\" \"$$\" >> '{}'\nif [ \"$mode\" = error ]; then\nprintf '%s' '[{{\"provider\":\"claude\",\"source\":\"oauth\",\"error\":{{\"code\":3,\"message\":\"Claude OAuth usage endpoint is rate limited by Anthropic right now.\",\"kind\":\"provider\"}}}}]'\nexit 3\nfi\nprintf '%s' '{}'",
                mode.display(),
                root.display(),
                calls.display(),
                fixture
            ),
        );
        let unit = Duration::from_millis(2);
        let quotas = (0..3)
            .map(|_| {
                Quotas::start_every(
                    &program,
                    unit,
                    Some(store.clone()),
                    &["claude_quota_5h"].map(String::from),
                )
            })
            .collect::<Vec<_>>();
        let starts = || {
            fs::read_to_string(&calls)
                .unwrap_or_default()
                .lines()
                .map(|line| {
                    let (mode, pid) = line.split_once(' ').unwrap();
                    (
                        mode.to_owned(),
                        fs::metadata(root.join(format!("{mode}.{pid}")))
                            .unwrap()
                            .modified()
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let wait = |condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !condition() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(condition(), "calls: {:?}", starts());
        };
        wait(&|| !starts().is_empty());
        thread::sleep(unit * 750);
        assert!((2..=3).contains(&starts().len()), "calls: {:?}", starts());
        wait(&|| quotas.iter().all(|q| q.claude.snapshot().is_some()));
        fs::write(&mode, "error").unwrap();
        wait(&|| {
            quotas.iter().all(|q| {
                q.claude
                    .failure()
                    .is_some_and(|f| f.reason == Failure::ProviderError)
            })
        });
        // `record_read` pins each delay (claude_schedule_preserves_reasons_bounds_and_backoff);
        // here the supervisors share them: one soon retry, then a backoff.
        wait(&|| starts().iter().filter(|(mode, _)| mode == "error").count() >= 2);
        fs::write(&mode, "ok").unwrap();
        wait(&|| {
            let calls = starts();
            calls.last().is_some_and(|(mode, _)| mode == "ok")
                && quotas.iter().all(|q| q.claude.failure().is_none())
        });
        let calls = starts();
        let errors = calls
            .iter()
            .enumerate()
            .filter(|(_, (mode, _))| mode == "error")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 2, "{calls:?}");
        let retry = calls[errors[1]]
            .1
            .duration_since(calls[errors[0]].1)
            .unwrap();
        // One interval plus jitter (at most 360 units), with room for a loaded machine; the
        // backoff is 1800.
        assert!(
            retry >= unit * 300 && retry < unit * 1200,
            "first retry: {retry:?}"
        );
        let recovered = errors[1] + 1;
        assert!(
            calls[recovered]
                .1
                .duration_since(calls[errors[1]].1)
                .unwrap()
                >= unit * 1800
        );
        wait(&|| starts().len() > recovered + 1);
        let calls = starts();
        assert!(
            calls
                .windows(2)
                .all(|pair| pair[1].1.duration_since(pair[0].1).unwrap() >= unit * 300),
            "read starts: {calls:?}"
        );
        let gap = calls[recovered + 1]
            .1
            .duration_since(calls[recovered].1)
            .unwrap();
        assert!(
            gap >= unit * 300 && gap < unit * 1800,
            "recovered gap: {gap:?}"
        );
        assert!(quotas.iter().all(|q| q.claude.snapshot().is_some()));
        drop(quotas);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_claude_schedules_allow_an_immediate_read() {
        let root = super::super::supervisor::temporary_directory();
        let program = root.join("backend");
        let store = root.join("quota");
        fs::create_dir(&store).unwrap();
        let (fixture, _) = fresh("claude");
        script(&program, &format!("printf '%s' '{fixture}'"));
        for invalid in [
            "not json".to_owned(),
            json!({"version":2,"next_at":epoch_seconds().unwrap() * 1000 + 300_000,
                "failures":0,"failure":null})
            .to_string(),
            json!({"version":1,"next_at":u64::MAX,"failures":0,"failure":null}).to_string(),
        ] {
            fs::write(store.join("claude.schedule"), &invalid).unwrap();
            let _ = fs::remove_file(store.join("claude.json"));
            let quotas = Quotas::start_every(
                &program,
                Duration::from_secs(1),
                Some(store.clone()),
                &["claude_quota_5h"].map(String::from),
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while quotas.claude.snapshot().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(quotas.claude.snapshot().is_some(), "{invalid}");
            drop(quotas);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_schedule_preserves_reasons_bounds_and_backoff() {
        // The first provider error since a success retries after one interval plus a jitter of
        // up to a fifth; later ones back off to 6, 12 and 24 intervals.
        let interval = Duration::from_secs(300);
        assert_eq!(provider_error_wait(interval, 1, 0.0), interval);
        assert_eq!(
            provider_error_wait(interval, 1, 0.5),
            interval + interval / 10
        );
        assert!(provider_error_wait(interval, 1, 0.999) < interval * 6 / 5);
        assert_eq!(
            [2, 3, 4, u32::MAX].map(|failures| provider_error_wait(interval, failures, 0.5)),
            [6, 12, 24, 24].map(|n| interval * n)
        );
        assert_eq!(longest_wait(interval), interval * 24);
        // A finished read schedules the next from the same rule: the first provider error after
        // a success retries after one interval plus jitter, the next backs off, a failure that is
        // not a provider error keeps the count, and only a success resets it.
        let mut schedule = Schedule::default();
        let error = |reason| Finished {
            outcome: Some(Some(QuotaFailure { at: NOW, reason })),
            success: false,
            back_off: reason == Failure::ProviderError,
        };
        let mut next = |read: Finished, now: u64| {
            record_read(&mut schedule, now, read, interval, 0.5);
            (
                schedule.failures,
                Duration::from_millis(schedule.next_at - now),
            )
        };
        let provider = error(Failure::ProviderError);
        assert_eq!(next(provider, 1_000), (1, interval + interval / 10));
        assert_eq!(next(error(Failure::Deadline), 2_000), (1, interval));
        assert_eq!(next(provider, 3_000), (2, interval * 6));
        assert_eq!(next(provider, 4_000), (3, interval * 12));
        let ok = Finished {
            outcome: Some(None),
            success: true,
            back_off: false,
        };
        assert_eq!(next(ok, 5_000), (0, interval));
        assert_eq!(next(provider, 6_000), (1, interval + interval / 10));
        assert_eq!(
            schedule.failure.map(|f| f.reason),
            Some(Failure::ProviderError)
        );
        let draws = (0..16).map(|_| jitter()).collect::<Vec<_>>();
        assert!(draws.iter().all(|j| (0.0..1.0).contains(j)), "{draws:?}");
        assert!(draws.iter().any(|j| *j != draws[0]), "{draws:?}");
        // A clock set back never stops reads, also without a schedule file.
        let ahead = Mutex::new(Schedule {
            next_at: u64::MAX,
            ..Schedule::default()
        });
        let (bounded, _) = update_schedule(
            &ahead,
            None,
            Duration::from_secs(1),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(bounded.next_at < u64::MAX);
        let root = super::super::supervisor::temporary_directory();
        let path = root.join("schedule");
        let mut file = open_schedule(&path, true).unwrap();
        for reason in [
            Failure::Start,
            Failure::Deadline,
            Failure::OutputLimit,
            Failure::ExitStatus,
            Failure::Malformed,
            Failure::ProviderError,
            Failure::Estimated,
            Failure::ObservationTime,
            Failure::InvalidWindow,
        ] {
            let schedule = Schedule {
                failure: Some(QuotaFailure { at: NOW, reason }),
                next_at: 7200,
                ..Schedule::default()
            };
            file.set_len(0).unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&serde_json::to_vec(&schedule).unwrap())
                .unwrap();
            assert_eq!(
                load_schedule(&mut file, 0, Duration::from_millis(7200))
                    .unwrap()
                    .failure,
                schedule.failure
            );
            assert!(load_schedule(&mut file, 0, Duration::from_millis(7199)).is_none());
        }
        for invalid in [
            json!({"version":1,"next_at":0,"failures":0,"failure":null,"other":0}).to_string(),
            json!({"version":1,"next_at":0,"failures":0,"failure":{"at":0,"reason":"provider_error"}}).to_string(),
            json!({"version":1,"next_at":0,"failures":0,"failure":{"at":NOW,"reason":"unknown"}}).to_string(),
            json!({"version":1,"next_at":0,"failures":0,"failure":{"at":NOW,"reason":"start","message":"discard"}}).to_string(),
            " ".repeat(4097),
        ] {
            file.set_len(0).unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(invalid.as_bytes()).unwrap();
            assert!(load_schedule(&mut file, 0, Duration::from_secs(7200)).is_none());
        }
        drop(file);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_schedule_rewrites_keep_the_in_memory_backoff() {
        let root = super::super::supervisor::temporary_directory();
        let path = root.join("claude.schedule");
        fs::write(&path, "partial").unwrap();
        let mut read_only = fs::File::open(&path).unwrap();
        let failure = QuotaFailure {
            at: NOW,
            reason: Failure::ProviderError,
        };
        let next_at = epoch_seconds().unwrap() * 1000 + 3_600_000;
        let mut schedule = Schedule {
            next_at,
            failures: 2,
            failure: Some(failure),
            ..Schedule::default()
        };
        assert!(write_schedule(&mut read_only, &mut schedule).is_err());
        let memory = Mutex::new(schedule);
        let (retained, shared) = update_schedule(
            &memory,
            Some(&path.join("unavailable")),
            Duration::from_secs(7200),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(!shared);
        assert_eq!(retained.next_at, next_at);
        assert_eq!(retained.failures, 2);
        assert_eq!(retained.failure, Some(failure));
        assert!(
            !serde_json::to_string(&retained)
                .unwrap()
                .contains("write_failed")
        );
        let (repaired, shared) = update_schedule(
            &memory,
            Some(&path),
            Duration::from_secs(7200),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(shared);
        assert_eq!(repaired.next_at, retained.next_at);
        assert_eq!(repaired.failure, retained.failure);
        assert!(!repaired.write_failed);
        assert_eq!(
            serde_json::from_slice::<Schedule>(&fs::read(&path).unwrap())
                .unwrap()
                .failures,
            2
        );
        assert!(write_schedule(&mut read_only, &mut memory.lock().unwrap()).is_err());
        let peer = Schedule {
            next_at: 100,
            ..Schedule::default()
        };
        fs::write(&path, serde_json::to_vec(&peer).unwrap()).unwrap();
        let (adopted, shared) = update_schedule(
            &memory,
            Some(&path),
            Duration::from_secs(7200),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(shared);
        assert_eq!(adopted.next_at, peer.next_at);
        assert!(adopted.failure.is_none());
        assert_eq!(adopted.failures, 0);
        assert!(!adopted.write_failed);
        // A corrupt file leaves the in-memory schedule in place rather than resetting it.
        memory.lock().unwrap().failures = 3;
        fs::write(&path, "partial").unwrap();
        let (kept, shared) = update_schedule(
            &memory,
            Some(&path),
            Duration::from_secs(7200),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(shared);
        assert_eq!(kept.failures, 3);
        // A peer that keeps the lock leaves it on the in-memory schedule, not the file's.
        fs::write(&path, serde_json::to_vec(&peer).unwrap()).unwrap();
        let held = open_schedule(&path, true).unwrap();
        let (locked_out, shared) = update_schedule(
            &memory,
            Some(&path),
            Duration::from_secs(7200),
            &Clock::System,
            true,
            |_, _, _| Some(false),
        );
        assert!(!shared);
        assert_eq!(locked_out.failures, 3);
        drop(held);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_peers_adopt_newer_readings_and_clear_only_successful_missing_quota() {
        let root = super::super::supervisor::temporary_directory();
        let store = root.join("quota");
        let path = store.join("claude.json");
        let schedule_path = store.join("claude.schedule");
        let memory = Mutex::new(Schedule::default());
        let maximum = Duration::from_secs(36);
        let failure = QuotaFailure {
            at: epoch_seconds().unwrap(),
            reason: Failure::ProviderError,
        };
        update_schedule(
            &memory,
            Some(&schedule_path),
            maximum,
            &Clock::System,
            true,
            |schedule, now, _| {
                schedule.next_at = now + 20_000;
                schedule.failure = Some(failure);
                Some(true)
            },
        );
        let (fixture, now) = fresh("claude");
        let mut reading = normalize(&serde_json::to_vec(&fixture).unwrap(), "claude", now)
            .unwrap()
            .unwrap();
        reading.observed_at -= 10;
        save(&path, Some(&reading));
        let quotas = (0..3)
            .map(|_| {
                Quotas::start_every(
                    &root.join("never-read"),
                    Duration::from_millis(5),
                    Some(store.clone()),
                    &["claude_quota_5h"].map(String::from),
                )
            })
            .collect::<Vec<_>>();
        let wait = |condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !condition() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(condition());
        };
        wait(&|| {
            quotas
                .iter()
                .all(|q| q.claude.snapshot().is_some() && q.claude.failure() == Some(failure))
        });
        fs::write(&path, "not json").unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(quotas.iter().all(|q| q.claude.snapshot().is_some()));
        reading.observed_at -= 10;
        save(&path, Some(&reading));
        thread::sleep(Duration::from_millis(200));
        assert!(
            quotas
                .iter()
                .all(|q| q.claude.snapshot().unwrap().observed_at == now - 10)
        );
        reading.observed_at = now;
        save(&path, Some(&reading));
        wait(&|| {
            quotas
                .iter()
                .all(|q| q.claude.snapshot().unwrap().observed_at == now)
        });
        save(&path, None);
        thread::sleep(Duration::from_millis(200));
        assert!(quotas.iter().all(|q| q.claude.snapshot().is_some()));
        update_schedule(
            &memory,
            Some(&schedule_path),
            maximum,
            &Clock::System,
            true,
            |schedule, _, _| {
                schedule.failure = None;
                Some(true)
            },
        );
        wait(&|| {
            quotas
                .iter()
                .all(|q| q.claude.snapshot().is_none() && q.claude.failure().is_none())
        });
        drop(quotas);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stored_readings_outlive_the_supervisor_and_bad_ones_are_ignored() {
        let root = super::super::supervisor::temporary_directory();
        let store = root.join("quota");
        let widgets = ["codex_quota_5h", "claude_quota_5h"].map(String::from);
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
            &["codex_quota_5h"].map(String::from),
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

    /// Every entry in `directory` with its length and modification time.
    fn listing(directory: &Path) -> Vec<(std::ffi::OsString, u64, SystemTime)> {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                (
                    entry.file_name(),
                    metadata.len(),
                    metadata.modified().unwrap(),
                )
            })
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn stored_facts_stand_in_for_a_window_only_when_certain() {
        let root = super::super::supervisor::temporary_directory();
        let store = root.join("quota");
        let soon = || Instant::now() + Duration::from_secs(2);
        // Without a store nothing is read and nothing is created.
        assert_eq!(
            stored_facts(&store, "codex", soon()).unwrap_err(),
            format!(
                "no stored reading at {}",
                store.join("codex.json").display()
            )
        );
        assert!(stored_facts(&store, "claude", soon()).is_err());
        assert!(!store.exists());

        // A window's poller leaves both readings and Claude's schedule; they stand in as the
        // window presents them, and reading them changes nothing.
        let (codex, now) = fresh("codex");
        let (claude, _) = fresh("claude");
        let reads = root.join("reads");
        script(
            &reads,
            &format!(
                "if [ \"$3\" = codex ]; then printf '%s' '{}'; else printf '%s' '{}'; fi",
                serde_json::to_string(&codex).unwrap(),
                serde_json::to_string(&claude).unwrap()
            ),
        );
        let quotas = Quotas::start_every(
            &reads,
            UNIT,
            Some(store.clone()),
            &["codex_quota_5h", "claude_quota_5h"].map(String::from),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(quotas.codex.snapshot().is_some()
            && quotas.claude.snapshot().is_some()
            && stored_facts(&store, "codex", soon()).is_ok()
            && stored_facts(&store, "claude", soon()).is_ok())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let window = (quotas.codex.snapshot(), quotas.claude.snapshot());
        drop(quotas);
        let before = listing(&store);
        let (codex, failure) = stored_facts(&store, "codex", soon()).unwrap();
        assert_eq!((Some(codex), failure), (window.0, None));
        let (claude, failure) = stored_facts(&store, "claude", soon()).unwrap();
        assert_eq!((Some(claude), failure), (window.1, None));
        assert_eq!(listing(&store), before);

        // Claude's recorded failure comes with its reading, as a window reports it.
        let schedule = store.join("claude.schedule");
        let failed = QuotaFailure {
            at: now,
            reason: Failure::ProviderError,
        };
        let recorded = Schedule {
            next_at: now * 1000,
            failures: 1,
            failure: Some(failed),
            ..Schedule::default()
        };
        fs::write(&schedule, serde_json::to_vec(&recorded).unwrap()).unwrap();
        assert_eq!(
            stored_facts(&store, "claude", soon()).unwrap().1,
            Some(failed)
        );
        // A schedule a window is rewriting, or one that cannot be read, leaves the failure
        // unknown: the reading is refused, inside the deadline.
        let unknown = |why: &str| {
            Err(format!(
                "failure state unknown: {why} ({})",
                schedule.display()
            ))
        };
        let writer = fs::File::open(&schedule).unwrap();
        writer.lock().unwrap();
        let started = Instant::now();
        assert_eq!(
            stored_facts(&store, "claude", soon()),
            unknown("a window is rewriting it")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(writer);
        fs::write(&schedule, "{").unwrap();
        assert_eq!(
            stored_facts(&store, "claude", soon()),
            unknown("unreadable or invalid")
        );
        fs::remove_file(&schedule).unwrap();
        assert!(
            stored_facts(&store, "claude", soon())
                .unwrap_err()
                .starts_with("failure state unknown: ")
        );
        assert!(!schedule.exists());

        // Codex records no failure: its reading stands in only while at most two reads old.
        let path = store.join("codex.json");
        let mut reading = load(&path, "codex", now).unwrap();
        reading.observed_at = epoch_seconds().unwrap() - 60;
        save(&path, Some(&reading));
        assert!(stored_facts(&store, "codex", soon()).is_ok());
        reading.observed_at = epoch_seconds().unwrap() - 300;
        save(&path, Some(&reading));
        assert!(
            stored_facts(&store, "codex", soon())
                .unwrap_err()
                .ends_with(
                    " s old; Codex records no failure, so only one at most 120 s old is used"
                )
        );
        // An expired reading, and one a window would not load, are refused.
        reading.observed_at = epoch_seconds().unwrap() - EXPIRE_AFTER - 1;
        save(&path, Some(&reading));
        assert_eq!(
            stored_facts(&store, "codex", soon()).unwrap_err(),
            format!("the stored reading at {} has expired", path.display())
        );
        let invalid = format!(
            "the stored reading at {} is unreadable, malformed, of another version or from the future",
            path.display()
        );
        let window = json!({"duration_minutes":300,"remaining_percent":76,"resets_at":null});
        let now = epoch_seconds().unwrap();
        for bad in [
            json!({"version":2,"observed_at":now,"windows":[window]}),
            json!({"version":1,"observed_at":now + 3600,"windows":[window]}),
            json!({"version":1,"observed_at":now,"windows":[window],"account":"x"}),
        ] {
            fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
            assert_eq!(stored_facts(&store, "codex", soon()), Err(invalid.clone()));
        }
        fs::write(&path, "not json").unwrap();
        assert_eq!(stored_facts(&store, "codex", soon()), Err(invalid));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "quota/google_collection_tests.rs"]
mod google_collection_tests;
