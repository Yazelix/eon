//! Bench v2 pre-start run gates and before/after pool usage readings (`eon-2r02l.16`).
//!
//! [`RunGate`] enforces all eight pre-start controls in one owner before any trial run begins:
//!
//! 1. **Machine limits:** 1-minute load average and available memory against `LanesFile.machine`
//!    (live Linux `/proc/loadavg` and `/proc/meminfo` reads under the Bench v2 A3 exception,
//!    `eon-78qnj`; all parsers, state transitions, lock/caps readers, and tests remain portable).
//! 2. **Bench-wide builder cap:** at most `max_concurrent_builders` (default `2`) builder runs
//!    active across the whole host, enforced via shared `flock` slot files under
//!    `GateConfig.state_dir`; reviewer runs ignore this cap and run up to their lane cap.
//! 3. **Lane run cap:** external live runs read-only from each lane's `lock_file` or `caps_file`
//!    plus in-process reservations and launches, keeping aggregate account counters for admission
//!    while requiring pool-attributable counts for usage snapshots.
//! 4. **Pool floors:** runs the lane's `usage_read.command` (and `usage_read.in_slot_command` or a
//!    caller-supplied held-slot probe during confirmation) with bounded output and process-group
//!    deadline cleanup, parses `agy`, `codex`, `key_value`, `claude`, or `json` output, and holds
//!    when any meter is below floor (failing closed on command, timeout, parse, or missing-meter
//!    errors).
//! 5. **Account stagger:** enforces `lane.stagger_s` per account (`lane.account`, falling back to
//!    the lane name) across confirmed starts, pending reservations, and independent processes
//!    sharing `GateConfig.state_dir`.
//! 6. **Rate-limit back-off:** persists lane cooldowns, single-retry ownership (with crash-safe
//!    liveness locks), and lead holds under `GateConfig.state_dir`, rechecking them at both
//!    reservation and confirmation, and writing an atomic `eon-bench-notice/1` JSON notice when a
//!    post-cooldown retry hits a rate limit again.
//! 7. **Hold, never spill, and slot-confirmed wall caps:** held requests stay queued on their
//!    original lane and FIFO position (returned on confirmation hold or kept in place by
//!    [`RunGate::poll_queue_acquire`]); [`GateReservation`] carries no [`WallCap`], and the
//!    wall-cap clock starts only in [`RunGate::confirm_slot`] / [`RunGate::confirm_slot_with_probe`]
//!    after external slot acquisition.
//! 8. **Usage readings:** captures before and after [`PoolSnapshot`] values (with pool-attributable
//!    `other_live_runs` excluding self) as a [`PoolReading`] ready for `rows::RunRow`.

use crate::record::{Text, Utc};
use crate::rows::{Load, PoolReading, PoolSnapshot};
use crate::trial::{
    Lane, LanesFile, LiveRunsCounter, LiveRunsCounterKind, MachineLimits, TrialRole, UsageParser,
    validate_lanes,
};
use crate::{Error, Result, check_non_negative_finite, fail, jcs, sha256_hex, time, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const NOTICE_SCHEMA: &str = "eon-bench-notice/1";
pub const GATE_STATE_SCHEMA: &str = "eon-bench-gate-state/1";
pub const DEFAULT_MAX_CONCURRENT_BUILDERS: u64 = 2;
pub const DEFAULT_RATE_LIMIT_HOLD_S: u64 = 600;
pub const DEFAULT_RATE_LIMIT_RETRIES: u64 = 1;
pub const DEFAULT_USAGE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_USAGE_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_CAPS_FILE_BYTES: u64 = 64 * 1024;
const BUILDER_SLOTS_SUBDIR: &str = "builder-slots";
const COORD_LOCK_FILE: &str = ".coord.lock";
const GATE_STATE_FILE: &str = "gate-state.json";
const GATE_STATE_LOCK_FILE: &str = ".gate-state.lock";
const LEASES_SUBDIR: &str = "leases";
const REAP_TIMEOUT: Duration = Duration::from_secs(1);
const NOTICE_ACTION_REQUIRED: &str =
    "lane held after rate-limit retry failed; lead must inspect pool and clear hold";

static NEXT_GATE_ID: AtomicU64 = AtomicU64::new(1);

/// Policy bounds and local state/notice directories for [`RunGate`].
///
/// No filesystem path has a hardcoded default so callers explicitly place shared bench state and
/// lead notices without embedding machine-specific paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateConfig {
    /// Directory where bench-wide shared gate state (builder-slot locks) is stored.
    pub state_dir: PathBuf,
    /// Directory where lead notice JSON files (`eon-bench-notice/1`) are written.
    pub notice_dir: PathBuf,
    /// Bench-wide cap on concurrent builder runs (default: 2; reviewers ignore this cap).
    pub max_concurrent_builders: u64,
    /// Cooldown duration after a rate-limit error (default: 600s = 10 min).
    pub rate_limit_hold_s: u64,
    /// Post-cooldown retries before latching the lane held for the lead (default: 1).
    pub rate_limit_max_retries: u64,
    /// Deadline for both command execution and stdout/stderr pipe draining (default: 30s).
    pub usage_command_timeout: Duration,
}

impl GateConfig {
    /// Creates a configuration with the Bench v2 defaults and caller-supplied state/notice paths.
    pub fn new(state_dir: impl Into<PathBuf>, notice_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            notice_dir: notice_dir.into(),
            max_concurrent_builders: DEFAULT_MAX_CONCURRENT_BUILDERS,
            rate_limit_hold_s: DEFAULT_RATE_LIMIT_HOLD_S,
            rate_limit_max_retries: DEFAULT_RATE_LIMIT_RETRIES,
            usage_command_timeout: DEFAULT_USAGE_COMMAND_TIMEOUT,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.state_dir.as_os_str().is_empty() {
            fail!("gate config state_dir must not be empty");
        }
        if self.notice_dir.as_os_str().is_empty() {
            fail!("gate config notice_dir must not be empty");
        }
        if self.max_concurrent_builders == 0 {
            fail!("gate config max_concurrent_builders must be at least 1");
        }
        if self.max_concurrent_builders > jcs::MAX_EXACT {
            fail!("gate config max_concurrent_builders exceeds exact integer range (2^53 - 1)");
        }
        if self.rate_limit_hold_s > jcs::MAX_EXACT {
            fail!("gate config rate_limit_hold_s exceeds exact integer range (2^53 - 1)");
        }
        if self.rate_limit_max_retries > jcs::MAX_EXACT {
            fail!("gate config rate_limit_max_retries exceeds exact integer range (2^53 - 1)");
        }
        if self.usage_command_timeout.is_zero() {
            fail!("gate config usage_command_timeout must be greater than zero");
        }
        Ok(())
    }
}

/// One queued trial run requesting admission on a fixed lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub id: String,
    pub role: TrialRole,
    pub lane: String,
    pub wall_cap_s: u64,
}

/// Wall-clock deadline initialized strictly after external lane-slot confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallCap {
    pub started_at_s: u64,
    pub duration_s: u64,
    pub deadline_unix_s: u64,
}

impl WallCap {
    pub const fn start(now_s: u64, duration_s: u64) -> Self {
        Self {
            started_at_s: now_s,
            duration_s,
            deadline_unix_s: now_s.saturating_add(duration_s),
        }
    }

    pub const fn remaining_s(self, now_s: u64) -> u64 {
        self.deadline_unix_s.saturating_sub(now_s)
    }

    pub const fn is_expired(self, now_s: u64) -> bool {
        now_s >= self.deadline_unix_s
    }
}

/// Pre-confirmation reservation returned by [`RunGate::try_reserve`] / [`RunGate::poll_queue`].
///
/// Carries no [`WallCap`] and is intentionally non-`Clone` with private identity so it can only
/// be consumed once by [`RunGate::confirm_slot`] or [`RunGate::cancel`] on the owning gate.
#[derive(Debug, PartialEq)]
pub struct GateReservation {
    gate_id: u64,
    grant_id: u64,
    pub request: RunRequest,
    pub pool: String,
    pub account: String,
    pub checked_at_s: u64,
    checked_load: Load,
    checked_meters: BTreeMap<Text, f64>,
}

/// Confirmed active-run lease returned by [`RunGate::confirm_slot`].
///
/// Intentionally non-`Clone` with private identity so double-release is rejected at compile time
/// and unknown or foreign grants are rejected at runtime before mutating state.
#[derive(Debug, PartialEq)]
pub struct GateGrant {
    gate_id: u64,
    grant_id: u64,
    pub request: RunRequest,
    pub pool: String,
    pub account: String,
    pub granted_at_s: u64,
    pub wall_cap: WallCap,
    pub machine_load: Load,
    pub before_snapshot: PoolSnapshot,
}

/// Why a run request is held in its queue rather than admitted.
#[derive(Clone, Debug, PartialEq)]
pub enum HoldReason {
    MachineLoad {
        load_1m: f64,
        max_load_1m: f64,
    },
    MachineMemory {
        available_mib: u64,
        min_available_mib: u64,
    },
    BenchBuilderCap {
        active: u64,
        cap: u64,
    },
    LaneRunCap {
        lane: String,
        live: u64,
        cap: u64,
    },
    PoolBelowFloor {
        pool: String,
        meter: String,
        value: f64,
        floor: f64,
    },
    Stagger {
        account: String,
        wait_s: u64,
        ready_at_s: u64,
    },
    RateLimitCooldown {
        lane: String,
        wait_s: u64,
        ready_at_s: u64,
    },
    RateLimitRetryInFlight {
        lane: String,
    },
    RateLimitHeldForLead {
        lane: String,
        notice_path: PathBuf,
    },
}

impl HoldReason {
    /// Known minimum wait in seconds before retrying can succeed, if time-bounded.
    pub const fn retry_after_s(&self) -> Option<u64> {
        match self {
            Self::Stagger { wait_s, .. } | Self::RateLimitCooldown { wait_s, .. } => Some(*wait_s),
            _ => None,
        }
    }
}

impl fmt::Display for HoldReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MachineLoad {
                load_1m,
                max_load_1m,
            } => write!(
                f,
                "machine 1m load {load_1m:.2} exceeds max {max_load_1m:.2}"
            ),
            Self::MachineMemory {
                available_mib,
                min_available_mib,
            } => write!(
                f,
                "machine available memory {available_mib} MiB below minimum {min_available_mib} MiB"
            ),
            Self::BenchBuilderCap { active, cap } => {
                write!(f, "bench builder cap reached ({active}/{cap})")
            }
            Self::LaneRunCap { lane, live, cap } => {
                write!(f, "lane {lane:?} run cap reached ({live}/{cap})")
            }
            Self::PoolBelowFloor {
                pool,
                meter,
                value,
                floor,
            } => write!(
                f,
                "pool {pool:?} meter {meter:?} is {value} (below floor {floor})"
            ),
            Self::Stagger {
                account,
                wait_s,
                ready_at_s,
            } => write!(
                f,
                "account {account:?} stagger wait {wait_s}s (ready at {ready_at_s})"
            ),
            Self::RateLimitCooldown {
                lane,
                wait_s,
                ready_at_s,
            } => write!(
                f,
                "lane {lane:?} rate-limit cooldown {wait_s}s remaining (ready at {ready_at_s})"
            ),
            Self::RateLimitRetryInFlight { lane } => {
                write!(f, "lane {lane:?} rate-limit retry already in flight")
            }
            Self::RateLimitHeldForLead { lane, notice_path } => write!(
                f,
                "lane {lane:?} held for lead after rate-limit retry failed (notice: {})",
                notice_path.display()
            ),
        }
    }
}

/// Outcome of pre-slot gate checks ([`RunGate::try_reserve`]). Non-`Clone`.
#[derive(Debug, PartialEq)]
pub enum ReserveDecision {
    Reserved(GateReservation),
    Hold(HoldReason),
}

/// Outcome of finalizing a reservation after lane-slot acquisition ([`RunGate::confirm_slot`] /
/// [`RunGate::confirm_slot_with_probe`]). Non-`Clone`.
///
/// On [`Self::Hold`], returns the original [`RunRequest`] so callers can requeue it with its lane
/// and queue position intact (see [`requeue_request`]).
#[derive(Debug, PartialEq)]
pub enum GateDecision {
    Granted(GateGrant),
    Hold {
        request: RunRequest,
        reason: HoldReason,
    },
}

/// Outcome of polling a FIFO run queue ([`RunGate::poll_queue`]). Non-`Clone`.
///
/// `queue_index` records the reserved request's original FIFO position so callers can pass it to
/// [`requeue_request`] if external acquisition or confirmation holds.
#[derive(Debug, PartialEq)]
pub enum PollOutcome {
    Reserved {
        reservation: GateReservation,
        queue_index: usize,
        held: Vec<(String, HoldReason)>,
    },
    AllHeld {
        held: Vec<(String, HoldReason)>,
    },
}

/// Reinserts a held or cancelled [`RunRequest`] back into `queue` at `queue_index` (clamped to
/// `queue.len()`), preserving its original lane and FIFO position.
pub fn requeue_request(queue: &mut VecDeque<RunRequest>, queue_index: usize, request: RunRequest) {
    let pos = queue_index.min(queue.len());
    queue.insert(pos, request);
}

/// Result returned by a caller's external lane-slot acquirer in [`RunGate::acquire_slot`].
#[derive(Debug, PartialEq, Eq)]
pub enum SlotAcquisition<S> {
    Acquired(S),
    Busy,
}

/// Outcome of [`RunGate::acquire_slot`] / [`RunGate::acquire_slot_with_probe`]. Non-`Clone`.
#[derive(Debug, PartialEq)]
pub enum AcquireOutcome<S> {
    Granted {
        grant: GateGrant,
        slot: S,
    },
    Hold {
        request: RunRequest,
        reason: HoldReason,
    },
}

/// Outcome of [`RunGate::poll_queue_acquire`]. Non-`Clone`.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum QueueAcquireOutcome<S> {
    Granted {
        grant: GateGrant,
        slot: S,
        held: Vec<(String, HoldReason)>,
    },
    AllHeld {
        held: Vec<(String, HoldReason)>,
    },
}

/// How a confirmed run ended, supplied to [`RunGate::finish`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunCompletion {
    Completed,
    RateLimited { detail: String },
}

/// Rate-limit state transition triggered by [`RunGate::finish`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateLimitAction {
    Cooldown {
        lane: String,
        until_unix_s: u64,
        retries_remaining: u64,
    },
    HeldForLead {
        lane: String,
        notice_path: PathBuf,
    },
    Cleared {
        lane: String,
    },
}

/// Result of [`RunGate::finish`], containing the `PoolReading` ready for `rows::RunRow`.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishOutcome {
    pub pool_reading: PoolReading,
    pub after_reading_error: Option<Error>,
    pub rate_limit: Option<RateLimitAction>,
}

/// JSON notice written atomically when a lane's post-cooldown rate-limit retry fails.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitNotice {
    pub schema: String,
    pub kind: String,
    pub lane: String,
    pub pool: String,
    pub first_hit_at: String,
    pub second_hit_at: String,
    pub hold_seconds: u64,
    pub retries_attempted: u64,
    pub detail: String,
    pub action_required: String,
}

/// Scope of an external live-runs reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterScope {
    /// Reading counts only the queried lane (for example `{"lanes": {"lane-a": 1}}`).
    Lane,
    /// Reading is an aggregate shared by every lane pointing to the same counter path (for example
    /// a `lock_file`, plain integer `caps_file`, or top-level `{"live_runs": 2}`). Pool-attributable
    /// for usage snapshots only when no lane in another pool shares that counter path.
    SharedPath,
}

/// One external live-runs sample paired with its counter scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRunsSample {
    pub count: u64,
    pub scope: CounterScope,
}

/// Injectable Unix clock (seconds since epoch).
pub trait Clock {
    fn now_unix_s(&self) -> u64;
}

/// Production wall clock backed by [`SystemTime::now`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_s(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }
}

/// Machine load and available memory sample (KiB, matching Linux `/proc/meminfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct MachineSample {
    pub load: Load,
    pub available_kib: u64,
}

/// Injectable host load and memory probe.
pub trait MachineProbe {
    fn sample(&self) -> Result<MachineSample>;
}

/// Production machine probe: reads `/proc/loadavg` and `/proc/meminfo` on Linux.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemMachineProbe;

impl MachineProbe for SystemMachineProbe {
    fn sample(&self) -> Result<MachineSample> {
        #[cfg(target_os = "linux")]
        {
            let load_text = fs::read_to_string("/proc/loadavg")
                .map_err(|err| Error(format!("machine: /proc/loadavg: {err}")))?;
            let mem_text = fs::read_to_string("/proc/meminfo")
                .map_err(|err| Error(format!("machine: /proc/meminfo: {err}")))?;
            let load = parse_proc_loadavg(&load_text)?;
            let available_kib = parse_proc_meminfo_available_kib(&mem_text)?;
            Ok(MachineSample {
                load,
                available_kib,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            fail!("machine load and memory checks are Linux-only (Bench v2 A3 / eon-78qnj)")
        }
    }
}

/// Injectable read-only external live-runs counter reader.
pub trait LiveRunsReader {
    fn read_live_runs(&self, lane_name: &str, counter: &LiveRunsCounter) -> Result<LiveRunsSample>;
}

/// Production read-only counter reader (`File::open` + `try_lock_shared` / bounded `caps_file`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FsLiveRunsReader;

impl LiveRunsReader for FsLiveRunsReader {
    fn read_live_runs(&self, lane_name: &str, counter: &LiveRunsCounter) -> Result<LiveRunsSample> {
        let path = Path::new(counter.path.as_str());
        match counter.kind {
            LiveRunsCounterKind::LockFile => read_lock_counter(lane_name, path),
            LiveRunsCounterKind::CapsFile => read_caps_counter(lane_name, path),
        }
    }
}

/// Captured output of a usage-read command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Injectable runner for `lane.usage_read.command`.
pub trait UsageCommandRunner {
    fn run(&self, command: &[String], timeout: Duration) -> Result<CommandOutput>;
}

/// Production runner executing `command[0]` directly with bounded pipes and deadline enforcement.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemUsageCommandRunner;

impl UsageCommandRunner for SystemUsageCommandRunner {
    fn run(&self, command: &[String], timeout: Duration) -> Result<CommandOutput> {
        run_usage_command(command, timeout)
    }
}

// ---------------------------------------------------------------------------
// Portable parsers
// ---------------------------------------------------------------------------

/// Parses Linux `/proc/loadavg` text into 1-, 5-, and 15-minute load averages.
pub fn parse_proc_loadavg(text: &str) -> Result<Load> {
    let mut parts = text.split_whitespace();
    let one = parse_load_field(parts.next(), "1m")?;
    let five = parse_load_field(parts.next(), "5m")?;
    let fifteen = parse_load_field(parts.next(), "15m")?;
    Ok(Load { one, five, fifteen })
}

fn parse_load_field(token: Option<&str>, label: &str) -> Result<f64> {
    let Some(raw) = token else {
        fail!("machine: /proc/loadavg missing {label} load average");
    };
    let value: f64 = raw.parse().map_err(|_| {
        Error(format!(
            "machine: /proc/loadavg invalid {label} value {raw:?}"
        ))
    })?;
    check_non_negative_finite(value, &format!("machine: /proc/loadavg {label}"))?;
    Ok(normalize_zero(value))
}

/// Parses Linux `/proc/meminfo` text and returns `MemAvailable` in KiB.
pub fn parse_proc_meminfo_available_kib(text: &str) -> Result<u64> {
    let mut found = None;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("MemAvailable:") else {
            continue;
        };
        if found.is_some() {
            fail!("machine: /proc/meminfo contains duplicate MemAvailable lines");
        }
        let mut parts = rest.split_whitespace();
        let Some(value_tok) = parts.next() else {
            fail!("machine: /proc/meminfo MemAvailable missing value");
        };
        if !is_ascii_uint(value_tok) {
            fail!("machine: /proc/meminfo MemAvailable invalid value {value_tok:?}");
        }
        let kib: u64 = value_tok.parse().map_err(|_| {
            Error(format!(
                "machine: /proc/meminfo MemAvailable overflows u64: {value_tok:?}"
            ))
        })?;
        if let Some(unit) = parts.next()
            && unit != "kB"
        {
            fail!("machine: /proc/meminfo MemAvailable unexpected unit {unit:?}");
        }
        if let Some(extra) = parts.next() {
            fail!("machine: /proc/meminfo MemAvailable trailing token {extra:?}");
        }
        found = Some(kib);
    }
    found.ok_or_else(|| Error("machine: /proc/meminfo missing MemAvailable".into()))
}

/// Parses a `caps_file` payload into a [`LiveRunsSample`] with explicit [`CounterScope`].
///
/// Supported forms:
/// - JSON per-lane map: `{"lanes": {"<lane>": N | {"live_runs": N} | {"running": N}}}`
///   -> [`CounterScope::Lane`] (absent lane inside `"lanes"` yields `0`).
/// - JSON aggregate object: `{"live_runs": N}` and/or `{"running": N}` -> [`CounterScope::SharedPath`].
/// - Plain non-negative integer text (`"2\n"`) -> [`CounterScope::SharedPath`].
pub fn parse_caps_file(text: &str, lane_name: &str) -> Result<LiveRunsSample> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fail!("lanes.{lane_name}: caps file is empty");
    }
    if trimmed.starts_with('{') {
        jcs::reject_duplicate_names(trimmed.as_bytes())
            .map_err(|err| Error(format!("lanes.{lane_name}: caps file JSON: {}", err.0)))?;
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|err| Error(format!("lanes.{lane_name}: caps file JSON: {err}")))?;
        let Some(map) = value.as_object() else {
            fail!("lanes.{lane_name}: caps file JSON must be an object");
        };
        if let Some(lanes_val) = map.get("lanes") {
            if map.contains_key("live_runs") || map.contains_key("running") {
                fail!(
                    "lanes.{lane_name}: caps file cannot combine top-level `lanes` with `live_runs` or `running`"
                );
            }
            let Some(lanes_map) = lanes_val.as_object() else {
                fail!("lanes.{lane_name}: caps file `lanes` must be a JSON object");
            };
            let count = match lanes_map.get(lane_name) {
                None => 0,
                Some(entry) => parse_caps_lane_entry(entry, lane_name)?,
            };
            return Ok(LiveRunsSample {
                count,
                scope: CounterScope::Lane,
            });
        }
        let count = parse_live_count_fields(map, &format!("lanes.{lane_name}: caps file"))?;
        return Ok(LiveRunsSample {
            count,
            scope: CounterScope::SharedPath,
        });
    }
    if !is_ascii_uint(trimmed) {
        fail!("lanes.{lane_name}: caps file must be a non-negative integer or JSON object");
    }
    let count: u64 = trimmed.parse().map_err(|_| {
        Error(format!(
            "lanes.{lane_name}: caps file integer overflows u64"
        ))
    })?;
    Ok(LiveRunsSample {
        count,
        scope: CounterScope::SharedPath,
    })
}

fn parse_caps_lane_entry(value: &Value, lane_name: &str) -> Result<u64> {
    match value {
        Value::Number(num) => num.as_u64().ok_or_else(|| {
            Error(format!(
                "lanes.{lane_name}: caps file lanes.{lane_name} must be a non-negative integer"
            ))
        }),
        Value::Object(map) => parse_live_count_fields(
            map,
            &format!("lanes.{lane_name}: caps file lanes.{lane_name}"),
        ),
        _ => fail!("lanes.{lane_name}: caps file lanes.{lane_name} must be an integer or object"),
    }
}

fn parse_live_count_fields(map: &Map<String, Value>, label: &str) -> Result<u64> {
    let live_runs =
        match map.get("live_runs") {
            None => None,
            Some(val) => Some(val.as_u64().ok_or_else(|| {
                Error(format!("{label}.live_runs must be a non-negative integer"))
            })?),
        };
    let running = match map.get("running") {
        None => None,
        Some(val) => Some(
            val.as_u64()
                .ok_or_else(|| Error(format!("{label}.running must be a non-negative integer")))?,
        ),
    };
    match (live_runs, running) {
        (Some(a), Some(b)) if a != b => {
            fail!("{label}: conflicting live_runs ({a}) and running ({b})")
        }
        (Some(n), _) | (None, Some(n)) => Ok(n),
        (None, None) => fail!("{label}: expected `live_runs` or `running` integer field"),
    }
}

/// Parses stdout from `lane.usage_read.command` into meter readings (`Text -> f64`).
pub fn parse_usage(
    parser: UsageParser,
    group: Option<&str>,
    text: &str,
) -> Result<BTreeMap<Text, f64>> {
    if group.is_some() && parser != UsageParser::Agy {
        fail!("usage_read.group is only valid with the `agy` parser");
    }
    match parser {
        UsageParser::Agy => parse_usage_agy(group, text),
        UsageParser::Codex => parse_usage_codex(text),
        UsageParser::KeyValue => parse_usage_key_value(text),
        UsageParser::Claude => parse_usage_claude(text),
        UsageParser::Json => parse_usage_json(text),
    }
}

fn parse_usage_agy(group: Option<&str>, text: &str) -> Result<BTreeMap<Text, f64>> {
    if let Some(target) = group
        && target.trim().is_empty()
    {
        fail!("agy usage group filter must not be empty");
    }
    let mut meters = BTreeMap::new();
    let mut tsv_groups = BTreeSet::new();
    let mut matched_group_rows = 0usize;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if raw_line.contains('\t') {
            let cols: Vec<&str> = raw_line.split('\t').map(str::trim).collect();
            match cols.len() {
                3 | 4 => {
                    let row_group = cols[0];
                    if row_group.is_empty() {
                        fail!("agy usage TSV row has empty group column");
                    }
                    tsv_groups.insert(row_group.to_ascii_lowercase());
                    if group.is_none() && tsv_groups.len() > 1 {
                        fail!(
                            "agy usage output contains multiple groups ({}); set usage_read.group",
                            tsv_groups.into_iter().collect::<Vec<_>>().join(", ")
                        );
                    }
                    if let Some(target) = group
                        && !row_group.eq_ignore_ascii_case(target)
                    {
                        continue;
                    }
                    matched_group_rows += 1;
                    record_agy_window(&mut meters, cols[1], cols[2])?;
                }
                2 => {
                    if group.is_some() {
                        continue;
                    }
                    record_agy_window(&mut meters, cols[0], cols[1])?;
                }
                count => {
                    fail!("agy usage TSV row has unexpected column count {count}: {line:?}");
                }
            }
            continue;
        }

        if let Some((lhs, rhs)) = split_label_value(line)
            && classify_window_label(lhs).is_some()
        {
            if group.is_some() {
                continue;
            }
            record_agy_window(&mut meters, lhs, strip_trailing_paren(rhs))?;
        }
    }

    if let Some(target) = group
        && matched_group_rows == 0
    {
        fail!("agy usage output has no TSV rows matching group {target:?}");
    }
    if meters.is_empty() {
        fail!("agy usage output contained no recognized meters");
    }
    Ok(meters)
}

fn record_agy_window(meters: &mut BTreeMap<Text, f64>, window: &str, value: &str) -> Result<()> {
    let Some(meter) = classify_window_label(window) else {
        fail!("agy usage TSV unrecognized window {window:?}");
    };
    let pct = parse_percent_or_disabled(value, "agy usage")?;
    insert_unique_meter(meters, meter, pct, "agy usage")
}

fn parse_usage_codex(text: &str) -> Result<BTreeMap<Text, f64>> {
    let mut meters = BTreeMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((lhs, rhs)) = split_label_value(line) {
            if is_codex_credits_label(lhs) {
                let amount = parse_codex_amount(rhs)?;
                insert_unique_meter(&mut meters, "credits", amount, "codex usage")?;
                continue;
            }
            if let Some(meter) = classify_window_label(lhs) {
                let pct = parse_strict_percent(strip_trailing_paren(rhs), "codex usage")?;
                insert_unique_meter(&mut meters, meter, pct, "codex usage")?;
                continue;
            }
            continue;
        }
        if let Some(amount) = parse_bare_codex_credits(line)? {
            insert_unique_meter(&mut meters, "credits", amount, "codex usage")?;
        }
    }
    if meters.is_empty() {
        fail!("codex usage output contained no recognized meters");
    }
    Ok(meters)
}

fn parse_usage_key_value(text: &str) -> Result<BTreeMap<Text, f64>> {
    let mut meters = BTreeMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for token in line.split_whitespace() {
            let Some((raw_key, raw_val)) = token.split_once('=') else {
                fail!("key_value usage token {token:?} is missing `=`");
            };
            if raw_key.is_empty()
                || !raw_key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
            {
                fail!("key_value usage invalid meter name {raw_key:?}");
            }
            let value = if let Some(num) = raw_val.strip_suffix('%') {
                parse_percent_number(num, "key_value usage")?
            } else {
                parse_non_negative_decimal(raw_val, "key_value usage")?
            };
            insert_unique_meter(&mut meters, raw_key, value, "key_value usage")?;
        }
    }
    if meters.is_empty() {
        fail!("key_value usage output contained no meters");
    }
    Ok(meters)
}

fn parse_usage_claude(text: &str) -> Result<BTreeMap<Text, f64>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fail!("claude usage output is empty");
    }
    let mut meters = BTreeMap::new();
    if trimmed.starts_with('{') {
        jcs::reject_duplicate_names(trimmed.as_bytes())
            .map_err(|err| Error(format!("claude usage JSON: {}", err.0)))?;
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|err| Error(format!("claude usage JSON: {err}")))?;
        let Some(obj) = value.as_object() else {
            fail!("claude usage JSON must be an object");
        };
        for (field, meter) in [("five_hour", "five_hour_pct"), ("seven_day", "week_pct")] {
            let Some(window) = obj.get(field) else {
                continue;
            };
            if window.is_null() {
                continue;
            }
            let Some(window_obj) = window.as_object() else {
                fail!("claude usage JSON `{field}` must be an object or null");
            };
            let Some(util) = window_obj.get("utilization") else {
                fail!("claude usage JSON `{field}` is missing `utilization`");
            };
            if util.is_null() {
                continue;
            }
            let Some(used) = util.as_f64() else {
                fail!("claude usage JSON `{field}.utilization` must be a number");
            };
            let remaining = used_to_remaining_pct(used, &format!("claude usage {field}"))?;
            insert_unique_meter(&mut meters, meter, remaining, "claude usage")?;
        }
        if meters.is_empty() {
            fail!("claude usage JSON contained no five_hour or seven_day utilization");
        }
        return Ok(meters);
    }

    let lines: Vec<&str> = trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() != 1 {
        fail!("claude usage text expected a single line");
    }
    let tokens: Vec<&str> = lines[0].split_whitespace().collect();
    if tokens.len() < 2 {
        fail!("claude usage text expected at least two tokens (<5h_used> <7d_used>)");
    }
    for (token, meter) in [(tokens[0], "five_hour_pct"), (tokens[1], "week_pct")] {
        if token.eq_ignore_ascii_case("none") {
            continue;
        }
        let used = parse_strict_percent(token, "claude usage")?;
        let remaining = used_to_remaining_pct(used, "claude usage")?;
        insert_unique_meter(&mut meters, meter, remaining, "claude usage")?;
    }
    if meters.is_empty() {
        fail!("claude usage text contained no numeric window values");
    }
    Ok(meters)
}

fn parse_usage_json(text: &str) -> Result<BTreeMap<Text, f64>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fail!("json usage output is empty");
    }
    jcs::reject_duplicate_names(trimmed.as_bytes())
        .map_err(|err| Error(format!("json usage: {}", err.0)))?;
    let value: Value =
        serde_json::from_str(trimmed).map_err(|err| Error(format!("json usage: {err}")))?;
    let Some(map) = value.as_object() else {
        fail!("json usage output must be a JSON object");
    };
    let mut meters = BTreeMap::new();
    for (key, val) in map {
        if key == "observed_at" {
            if val.as_u64().is_none() {
                fail!("json usage `observed_at` must be a non-negative integer");
            }
            continue;
        }
        let Some(num) = val.as_f64() else {
            fail!("json usage meter {key:?} must be a number");
        };
        check_non_negative_finite(num, &format!("json usage meter {key:?}"))?;
        insert_unique_meter(&mut meters, key, normalize_zero(num), "json usage")?;
    }
    if meters.is_empty() {
        fail!("json usage output contained no meters");
    }
    Ok(meters)
}

fn insert_unique_meter(
    meters: &mut BTreeMap<Text, f64>,
    key: &str,
    value: f64,
    context: &str,
) -> Result<()> {
    let meter = Text::new(key).map_err(|err| Error(format!("{context}: {}", err.0)))?;
    if meters.insert(meter, normalize_zero(value)).is_some() {
        fail!("{context}: duplicate meter {key:?}");
    }
    Ok(())
}

fn normalize_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_space = true;
    for ch in raw.chars() {
        if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn classify_window_label(raw: &str) -> Option<&'static str> {
    let norm = normalize_label(raw);
    if norm
        .split(' ')
        .any(|tok| matches!(tok, "used" | "utilization"))
    {
        return None;
    }
    match norm.as_str() {
        "five hour"
        | "5 hour"
        | "5h"
        | "5hr"
        | "five hour limit"
        | "5 hour limit"
        | "5h limit"
        | "five hour remaining"
        | "5 hour remaining"
        | "5h remaining"
        | "five hour limit remaining"
        | "5 hour limit remaining"
        | "5h limit remaining"
        | "five hour pct"
        | "5 hour pct"
        | "5h pct" => Some("five_hour_pct"),
        "weekly"
        | "week"
        | "7 day"
        | "7d"
        | "weekly limit"
        | "week limit"
        | "7 day limit"
        | "7d limit"
        | "weekly remaining"
        | "week remaining"
        | "7 day remaining"
        | "7d remaining"
        | "weekly limit remaining"
        | "week limit remaining"
        | "7 day limit remaining"
        | "7d limit remaining"
        | "week pct"
        | "weekly pct"
        | "7d pct" => Some("week_pct"),
        other
            if other.ends_with(" five hour limit")
                || other.ends_with(" five hour limit remaining")
                || other.ends_with(" 5 hour limit")
                || other.ends_with(" 5 hour limit remaining") =>
        {
            Some("five_hour_pct")
        }
        other
            if other.ends_with(" weekly limit")
                || other.ends_with(" weekly limit remaining")
                || other.ends_with(" week limit")
                || other.ends_with(" week limit remaining")
                || other.ends_with(" 7 day limit")
                || other.ends_with(" 7 day limit remaining") =>
        {
            Some("week_pct")
        }
        _ => None,
    }
}

fn is_codex_credits_label(raw: &str) -> bool {
    matches!(
        normalize_label(raw).as_str(),
        "balance"
            | "credit balance"
            | "credits"
            | "credit"
            | "credits remaining"
            | "remaining credits"
            | "available credits"
            | "codex credits"
            | "codex balance"
    )
}

fn is_codex_unit(raw: &str) -> bool {
    let cleaned = raw.trim_end_matches('.');
    cleaned.eq_ignore_ascii_case("credit")
        || cleaned.eq_ignore_ascii_case("credits")
        || cleaned.eq_ignore_ascii_case("cr")
        || cleaned.eq_ignore_ascii_case("usd")
}

fn parse_codex_amount(raw: &str) -> Result<f64> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > 2 {
        fail!("codex usage invalid balance value {raw:?}");
    }
    let num_tok = tokens[0].strip_prefix('$').unwrap_or(tokens[0]);
    let value = parse_non_negative_decimal(num_tok, "codex usage balance")?;
    if let Some(unit) = tokens.get(1)
        && !is_codex_unit(unit)
    {
        fail!("codex usage unexpected balance unit {unit:?}");
    }
    Ok(value)
}

fn parse_bare_codex_credits(line: &str) -> Result<Option<f64>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() != 2 || !is_codex_unit(tokens[1]) {
        return Ok(None);
    }
    let num_tok = tokens[0].strip_prefix('$').unwrap_or(tokens[0]);
    Ok(Some(parse_non_negative_decimal(
        num_tok,
        "codex usage balance",
    )?))
}

fn split_label_value(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':');
    let eq = line.find('=');
    let idx = match (colon, eq) {
        (Some(c), Some(e)) => c.min(e),
        (Some(c), None) => c,
        (None, Some(e)) => e,
        (None, None) => return None,
    };
    Some((line[..idx].trim(), line[idx + 1..].trim()))
}

fn strip_trailing_paren(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.ends_with(')')
        && let Some(open) = trimmed.rfind('(')
    {
        return trimmed[..open].trim();
    }
    trimmed
}

fn parse_percent_or_disabled(raw: &str, context: &str) -> Result<f64> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("disabled") {
        return Ok(0.0);
    }
    parse_strict_percent(trimmed, context)
}

fn parse_strict_percent(raw: &str, context: &str) -> Result<f64> {
    let trimmed = raw.trim();
    let num = trimmed.strip_suffix('%').unwrap_or(trimmed).trim_end();
    parse_percent_number(num, context)
}

fn parse_percent_number(raw: &str, context: &str) -> Result<f64> {
    let value = parse_non_negative_decimal(raw, context)?;
    if value > 100.0 {
        fail!("{context}: percentage {value} exceeds 100");
    }
    Ok(value)
}

fn used_to_remaining_pct(used: f64, context: &str) -> Result<f64> {
    check_non_negative_finite(used, context)?;
    if used > 100.0 {
        fail!("{context}: utilization {used} exceeds 100");
    }
    Ok(normalize_zero(100.0 - used))
}

fn parse_non_negative_decimal(raw: &str, context: &str) -> Result<f64> {
    if raw.is_empty() || !raw.as_bytes()[0].is_ascii_digit() {
        fail!("{context}: invalid non-negative number {raw:?}");
    }
    let value: f64 = raw
        .parse()
        .map_err(|_| Error(format!("{context}: invalid number {raw:?}")))?;
    check_non_negative_finite(value, context)?;
    Ok(normalize_zero(value))
}

fn is_ascii_uint(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn normalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

// ---------------------------------------------------------------------------
// Read-only filesystem live-runs readers & shared builder-slot coordinator
// ---------------------------------------------------------------------------

fn check_parent_dir_exists(lane_name: &str, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let meta = fs::metadata(parent).map_err(|err| {
            Error(format!(
                "lanes.{lane_name}: counter parent directory {}: {err}",
                parent.display()
            ))
        })?;
        if !meta.is_dir() {
            fail!(
                "lanes.{lane_name}: counter parent {} is not a directory",
                parent.display()
            );
        }
    }
    Ok(())
}

fn is_file_exclusively_locked(lane_name: &str, path: &Path) -> Result<bool> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => fail!("lanes.{lane_name}: lock file {}: {err}", path.display()),
    };
    // This read-only probe briefly holds a shared `flock` until `file` drops to detect an
    // exclusive holder without creating or modifying the lock file.
    match file.try_lock_shared() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(err)) => {
            fail!("lanes.{lane_name}: lock probe {}: {err}", path.display())
        }
    }
}

fn read_lock_counter(lane_name: &str, path: &Path) -> Result<LiveRunsSample> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            check_parent_dir_exists(lane_name, path)?;
            return Ok(LiveRunsSample {
                count: 0,
                scope: CounterScope::SharedPath,
            });
        }
        Err(err) => fail!("lanes.{lane_name}: lock counter {}: {err}", path.display()),
    };

    if meta.is_file() {
        let locked = is_file_exclusively_locked(lane_name, path)?;
        return Ok(LiveRunsSample {
            count: u64::from(locked),
            scope: CounterScope::SharedPath,
        });
    }

    if meta.is_dir() {
        let entries = fs::read_dir(path).map_err(|err| {
            Error(format!(
                "lanes.{lane_name}: lock directory {}: {err}",
                path.display()
            ))
        })?;
        let mut live = 0u64;
        for entry in entries {
            let entry = entry.map_err(|err| {
                Error(format!(
                    "lanes.{lane_name}: lock directory {}: {err}",
                    path.display()
                ))
            })?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let entry_path = entry.path();
            let entry_meta = match fs::metadata(&entry_path) {
                Ok(meta) => meta,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => fail!(
                    "lanes.{lane_name}: lock file {}: {err}",
                    entry_path.display()
                ),
            };
            if entry_meta.is_file() && is_file_exclusively_locked(lane_name, &entry_path)? {
                live = live.saturating_add(1);
            }
        }
        return Ok(LiveRunsSample {
            count: live,
            scope: CounterScope::SharedPath,
        });
    }

    fail!(
        "lanes.{lane_name}: lock counter {} is neither a regular file nor a directory",
        path.display()
    );
}

fn read_caps_counter(lane_name: &str, path: &Path) -> Result<LiveRunsSample> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            check_parent_dir_exists(lane_name, path)?;
            return Ok(LiveRunsSample {
                count: 0,
                scope: CounterScope::SharedPath,
            });
        }
        Err(err) => fail!("lanes.{lane_name}: caps file {}: {err}", path.display()),
    };
    let meta = file.metadata().map_err(|err| {
        Error(format!(
            "lanes.{lane_name}: caps file {}: {err}",
            path.display()
        ))
    })?;
    if !meta.is_file() {
        fail!(
            "lanes.{lane_name}: caps file {} is not a regular file",
            path.display()
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_CAPS_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| {
            Error(format!(
                "lanes.{lane_name}: caps file {}: {err}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > MAX_CAPS_FILE_BYTES {
        fail!(
            "lanes.{lane_name}: caps file {} exceeds {MAX_CAPS_FILE_BYTES} bytes",
            path.display()
        );
    }
    let text = String::from_utf8(bytes).map_err(|err| {
        Error(format!(
            "lanes.{lane_name}: caps file {} is not valid UTF-8: {err}",
            path.display()
        ))
    })?;
    parse_caps_file(&text, lane_name)
}

/// Cross-process bench-wide builder slot coordinator backed by `flock` files under
/// `<state_dir>/builder-slots/`. Slot lock files are never unlinked so `flock` never races on
/// replaced inodes, and kernel FD teardown automatically releases slots if a process crashes.
struct BuilderSlotCoordinator {
    slots_dir: PathBuf,
}

enum BuilderSlotOutcome {
    Acquired(File),
    AtCap { active: u64 },
}

impl BuilderSlotCoordinator {
    fn new(state_dir: &Path) -> Self {
        Self {
            slots_dir: state_dir.join(BUILDER_SLOTS_SUBDIR),
        }
    }

    fn lock_coord(&self) -> Result<File> {
        fs::create_dir_all(&self.slots_dir).map_err(|err| slot_err(&self.slots_dir, err))?;
        let coord_path = self.slots_dir.join(COORD_LOCK_FILE);
        let file = open_lock_file(&coord_path).map_err(|err| slot_err(&coord_path, err))?;
        file.lock().map_err(|err| slot_err(&coord_path, err))?;
        Ok(file)
    }

    fn scan_locked_slots(&self) -> Result<HashSet<u64>> {
        let mut locked = HashSet::new();
        let entries =
            fs::read_dir(&self.slots_dir).map_err(|err| slot_err(&self.slots_dir, err))?;
        for entry in entries {
            let entry = entry.map_err(|err| slot_err(&self.slots_dir, err))?;
            let Some(index) = parse_slot_filename(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let path = entry.path();
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(slot_err(&path, err)),
            };
            match file.try_lock_shared() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => {
                    locked.insert(index);
                }
                Err(TryLockError::Error(err)) => return Err(slot_err(&path, err)),
            }
        }
        Ok(locked)
    }

    fn probe_active(&self) -> Result<u64> {
        let _coord = self.lock_coord()?;
        Ok(self.scan_locked_slots()?.len() as u64)
    }

    fn try_acquire(&self, cap: u64) -> Result<BuilderSlotOutcome> {
        let _coord = self.lock_coord()?;
        let mut locked = self.scan_locked_slots()?;
        if (locked.len() as u64) >= cap {
            return Ok(BuilderSlotOutcome::AtCap {
                active: locked.len() as u64,
            });
        }
        let mut candidate = 0u64;
        loop {
            while locked.contains(&candidate) {
                candidate = candidate.saturating_add(1);
            }
            let slot_path = self.slots_dir.join(format!("slot-{candidate}.lock"));
            let file = open_lock_file(&slot_path).map_err(|err| slot_err(&slot_path, err))?;
            match file.try_lock() {
                Ok(()) => return Ok(BuilderSlotOutcome::Acquired(file)),
                Err(TryLockError::WouldBlock) => {
                    locked.insert(candidate);
                    if (locked.len() as u64) >= cap {
                        return Ok(BuilderSlotOutcome::AtCap {
                            active: locked.len() as u64,
                        });
                    }
                    candidate = candidate.saturating_add(1);
                }
                Err(TryLockError::Error(err)) => return Err(slot_err(&slot_path, err)),
            }
        }
    }
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
}

fn slot_err(path: &Path, err: io::Error) -> Error {
    Error(format!("builder slots {}: {err}", path.display()))
}

fn parse_slot_filename(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("slot-")?.strip_suffix(".lock")?;
    if !is_ascii_uint(digits) {
        return None;
    }
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// Deadline-bound usage command runner (non-blocking pipes + process-group kill)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

impl StreamKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

fn set_pipe_nonblocking(
    fd: &impl std::os::fd::AsFd,
    program: &str,
    kind: StreamKind,
) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(|err| {
        Error(format!(
            "usage_read command {program:?} {} fcntl(F_GETFL) failed: {err}",
            kind.label()
        ))
    })?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(|err| {
        Error(format!(
            "usage_read command {program:?} {} fcntl(F_SETFL) failed: {err}",
            kind.label()
        ))
    })
}

/// Sends SIGKILL to the unreaped child's process group (killing wrapper subprocesses as well as
/// the direct child) and performs a bounded reap so even a D-state process cannot hang the gate.
fn kill_group_and_reap(child: &mut Child) -> Option<ExitStatus> {
    let _ = rustix::process::kill_process_group(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::KILL,
    );
    let _ = child.kill();
    let until = Instant::now() + REAP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= until {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Drains all immediately available bytes from a non-blocking pipe into `dest`.
/// Returns `Ok(true)` if any bytes or EOF were observed on this poll.
fn drain_nonblocking_pipe<R: Read>(
    reader: &mut R,
    dest: &mut Vec<u8>,
    eof: &mut bool,
    program: &str,
    kind: StreamKind,
) -> Result<bool> {
    if *eof {
        return Ok(false);
    }
    let mut made_progress = false;
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                *eof = true;
                return Ok(true);
            }
            Ok(n) => {
                made_progress = true;
                if dest.len().saturating_add(n) > MAX_USAGE_OUTPUT_BYTES {
                    fail!(
                        "usage_read command {program:?} {} exceeded {MAX_USAGE_OUTPUT_BYTES} bytes",
                        kind.label()
                    );
                }
                dest.extend_from_slice(&buf[..n]);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(made_progress),
            Err(err) => {
                fail!(
                    "usage_read command {program:?} {} read failed: {err}",
                    kind.label()
                );
            }
        }
    }
}

fn run_usage_command(command: &[String], timeout: Duration) -> Result<CommandOutput> {
    use std::os::unix::process::CommandExt;

    let Some((program, args)) = command.split_first() else {
        fail!("usage_read command must not be empty");
    };
    if timeout.is_zero() {
        fail!("usage_read timeout must be greater than zero");
    }

    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            Error(format!(
                "usage_read command {program:?} failed to spawn: {err}"
            ))
        })?;

    let mut stdout_pipe = match child.stdout.take() {
        Some(pipe) => pipe,
        None => {
            kill_group_and_reap(&mut child);
            fail!("usage_read command {program:?} missing stdout pipe");
        }
    };
    let mut stderr_pipe = match child.stderr.take() {
        Some(pipe) => pipe,
        None => {
            kill_group_and_reap(&mut child);
            fail!("usage_read command {program:?} missing stderr pipe");
        }
    };
    if let Err(err) = set_pipe_nonblocking(&stdout_pipe, program, StreamKind::Stdout)
        .and_then(|()| set_pipe_nonblocking(&stderr_pipe, program, StreamKind::Stderr))
    {
        kill_group_and_reap(&mut child);
        return Err(err);
    }

    let deadline = Instant::now() + timeout;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;

    loop {
        let stdout_progress = match drain_nonblocking_pipe(
            &mut stdout_pipe,
            &mut stdout_bytes,
            &mut stdout_eof,
            program,
            StreamKind::Stdout,
        ) {
            Ok(progress) => progress,
            Err(err) => {
                kill_group_and_reap(&mut child);
                return Err(err);
            }
        };
        let stderr_progress = match drain_nonblocking_pipe(
            &mut stderr_pipe,
            &mut stderr_bytes,
            &mut stderr_eof,
            program,
            StreamKind::Stderr,
        ) {
            Ok(progress) => progress,
            Err(err) => {
                kill_group_and_reap(&mut child);
                return Err(err);
            }
        };

        let both_eof = stdout_eof && stderr_eof;
        // Keep the child unreaped while either pipe is still open so its PID/PGID cannot be
        // recycled before process-group cleanup runs on timeout or error.
        if both_eof {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = String::from_utf8(stdout_bytes).map_err(|err| {
                        Error(format!(
                            "usage_read command {program:?} produced non-UTF-8 stdout: {err}"
                        ))
                    })?;
                    let stderr = String::from_utf8(stderr_bytes).map_err(|err| {
                        Error(format!(
                            "usage_read command {program:?} produced non-UTF-8 stderr: {err}"
                        ))
                    })?;
                    return Ok(CommandOutput {
                        success: status.success(),
                        status_code: status.code(),
                        stdout,
                        stderr,
                    });
                }
                Ok(None) => {}
                Err(err) => {
                    kill_group_and_reap(&mut child);
                    fail!("usage_read command {program:?} wait failed: {err}");
                }
            }
        }

        let now = Instant::now();
        if now >= deadline {
            let reaped = kill_group_and_reap(&mut child);
            if !both_eof && reaped.is_some_and(|status| status.code().is_some()) {
                fail!(
                    "usage_read command {program:?} output pipes did not close within {}ms timeout",
                    timeout.as_millis()
                );
            }
            fail!(
                "usage_read command {program:?} timed out after {}ms",
                timeout.as_millis()
            );
        }

        if !stdout_progress && !stderr_progress {
            let remaining = deadline.saturating_duration_since(now);
            thread::sleep(remaining.min(Duration::from_millis(5)));
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-process shared gate state (stagger & rate limits + crash-safe leases)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountStart {
    started_at_s: u64,
    stagger_until_s: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingStagger {
    lease_id: u64,
    lock_slot: u64,
    account: String,
    stagger_s: u64,
    checked_at_s: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CooldownSnapshot {
    until_unix_s: u64,
    retries_done: u64,
    detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LaneRateLimitState {
    CoolingDown {
        first_hit_at_s: u64,
        until_unix_s: u64,
        retries_done: u64,
        detail: String,
    },
    RetryInFlight {
        lease_id: u64,
        lock_slot: u64,
        first_hit_at_s: u64,
        retries_done: u64,
        prior_cooldown: CooldownSnapshot,
    },
    HeldForLead {
        notice_path: PathBuf,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedGateState {
    schema: String,
    next_lease_id: u64,
    account_starts: BTreeMap<String, AccountStart>,
    pending_staggers: Vec<PendingStagger>,
    rate_limits: BTreeMap<String, LaneRateLimitState>,
}

impl Default for SharedGateState {
    fn default() -> Self {
        Self {
            schema: GATE_STATE_SCHEMA.to_owned(),
            next_lease_id: 1,
            account_starts: BTreeMap::new(),
            pending_staggers: Vec::new(),
            rate_limits: BTreeMap::new(),
        }
    }
}

impl SharedGateState {
    fn allocate_lease_id(&mut self) -> Result<u64> {
        let id = self.next_lease_id;
        self.next_lease_id = self
            .next_lease_id
            .checked_add(1)
            .ok_or_else(|| Error("gate lease id counter overflowed u64".into()))?;
        Ok(id)
    }

    fn check_stagger(
        &self,
        account: &str,
        stagger_s: u64,
        now_s: u64,
        exclude_lease_id: Option<u64>,
    ) -> Option<HoldReason> {
        let mut required_until = self.account_starts.get(account).map(|start| {
            start
                .stagger_until_s
                .max(start.started_at_s.saturating_add(stagger_s))
        });

        for pending in &self.pending_staggers {
            if exclude_lease_id == Some(pending.lease_id) || pending.account != account {
                continue;
            }
            let pending_stagger = pending.stagger_s.max(stagger_s);
            if pending_stagger > 0 {
                let pending_until = now_s
                    .max(pending.checked_at_s)
                    .saturating_add(pending_stagger);
                required_until = Some(match required_until {
                    Some(existing) => existing.max(pending_until),
                    None => pending_until,
                });
            }
        }

        if let Some(ready_at_s) = required_until
            && now_s < ready_at_s
        {
            return Some(HoldReason::Stagger {
                account: account.to_owned(),
                wait_s: ready_at_s - now_s,
                ready_at_s,
            });
        }
        None
    }

    fn check_rate_limit_for_reserve(&self, lane_name: &str, now_s: u64) -> Option<HoldReason> {
        match self.rate_limits.get(lane_name)? {
            LaneRateLimitState::HeldForLead { notice_path } => {
                Some(HoldReason::RateLimitHeldForLead {
                    lane: lane_name.to_owned(),
                    notice_path: notice_path.clone(),
                })
            }
            LaneRateLimitState::RetryInFlight { .. } => Some(HoldReason::RateLimitRetryInFlight {
                lane: lane_name.to_owned(),
            }),
            LaneRateLimitState::CoolingDown { until_unix_s, .. } if now_s < *until_unix_s => {
                Some(HoldReason::RateLimitCooldown {
                    lane: lane_name.to_owned(),
                    wait_s: until_unix_s - now_s,
                    ready_at_s: *until_unix_s,
                })
            }
            LaneRateLimitState::CoolingDown { .. } => None,
        }
    }

    fn check_rate_limit_for_confirm(
        &self,
        lane_name: &str,
        lease_id: u64,
        now_s: u64,
    ) -> Option<HoldReason> {
        match self.rate_limits.get(lane_name)? {
            LaneRateLimitState::HeldForLead { notice_path } => {
                Some(HoldReason::RateLimitHeldForLead {
                    lane: lane_name.to_owned(),
                    notice_path: notice_path.clone(),
                })
            }
            LaneRateLimitState::CoolingDown { until_unix_s, .. } if now_s < *until_unix_s => {
                Some(HoldReason::RateLimitCooldown {
                    lane: lane_name.to_owned(),
                    wait_s: until_unix_s - now_s,
                    ready_at_s: *until_unix_s,
                })
            }
            LaneRateLimitState::CoolingDown { .. } => None,
            LaneRateLimitState::RetryInFlight {
                lease_id: owner_id,
                prior_cooldown,
                ..
            } => {
                if *owner_id != lease_id {
                    return Some(HoldReason::RateLimitRetryInFlight {
                        lane: lane_name.to_owned(),
                    });
                }
                if now_s < prior_cooldown.until_unix_s {
                    return Some(HoldReason::RateLimitCooldown {
                        lane: lane_name.to_owned(),
                        wait_s: prior_cooldown.until_unix_s - now_s,
                        ready_at_s: prior_cooldown.until_unix_s,
                    });
                }
                None
            }
        }
    }

    fn record_account_start(&mut self, account: &str, started_at_s: u64, stagger_s: u64) {
        let stagger_until_s = started_at_s.saturating_add(stagger_s);
        let entry = self
            .account_starts
            .entry(account.to_owned())
            .or_insert(AccountStart {
                started_at_s,
                stagger_until_s,
            });
        entry.started_at_s = entry.started_at_s.max(started_at_s);
        entry.stagger_until_s = entry.stagger_until_s.max(stagger_until_s);
    }

    fn promote_cooldown_to_retry(&mut self, lane_name: &str, lease_id: u64, lock_slot: u64) {
        if let Some(LaneRateLimitState::CoolingDown {
            first_hit_at_s,
            until_unix_s,
            retries_done,
            detail,
        }) = self.rate_limits.remove(lane_name)
        {
            self.rate_limits.insert(
                lane_name.to_owned(),
                LaneRateLimitState::RetryInFlight {
                    lease_id,
                    lock_slot,
                    first_hit_at_s,
                    retries_done: retries_done.saturating_add(1),
                    prior_cooldown: CooldownSnapshot {
                        until_unix_s,
                        retries_done,
                        detail,
                    },
                },
            );
        }
    }
}

struct LeaseLocker<'a> {
    leases_dir: &'a Path,
}

impl LeaseLocker<'_> {
    fn acquire(&self) -> Result<(u64, File)> {
        fs::create_dir_all(self.leases_dir).map_err(|err| state_err(self.leases_dir, err))?;
        let mut slot = 0u64;
        loop {
            let path = self.leases_dir.join(format!("lease-{slot}.lock"));
            let file = open_lock_file(&path).map_err(|err| state_err(&path, err))?;
            match file.try_lock() {
                Ok(()) => return Ok((slot, file)),
                Err(TryLockError::WouldBlock) => {
                    slot = slot
                        .checked_add(1)
                        .ok_or_else(|| Error("gate lease lock slot overflowed u64".into()))?;
                }
                Err(TryLockError::Error(err)) => return Err(state_err(&path, err)),
            }
        }
    }
}

/// Persists cross-process account stagger and lane rate-limit state under `<state_dir>`,
/// serialized by `.gate-state.lock` and backed by per-lease `flock` liveness files under
/// `<state_dir>/leases/` so crashed holders are pruned automatically.
struct SharedGateStateStore {
    state_dir: PathBuf,
    lock_path: PathBuf,
    state_path: PathBuf,
    leases_dir: PathBuf,
}

impl SharedGateStateStore {
    fn new(state_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            lock_path: state_dir.join(GATE_STATE_LOCK_FILE),
            state_path: state_dir.join(GATE_STATE_FILE),
            leases_dir: state_dir.join(LEASES_SUBDIR),
        }
    }

    fn with_state_mut<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut SharedGateState, &LeaseLocker<'_>) -> Result<(T, bool)>,
    {
        fs::create_dir_all(&self.state_dir).map_err(|err| state_err(&self.state_dir, err))?;
        let lock_file =
            open_lock_file(&self.lock_path).map_err(|err| state_err(&self.lock_path, err))?;
        lock_file
            .lock()
            .map_err(|err| state_err(&self.lock_path, err))?;

        let mut state = self.load_locked()?;
        let pruned = self.prune_dead_leases_locked(&mut state)?;
        let leases = LeaseLocker {
            leases_dir: &self.leases_dir,
        };
        let (result, mutated) = f(&mut state, &leases)?;
        if pruned || mutated {
            self.save_locked(&state)?;
        }
        Ok(result)
    }

    fn load_locked(&self) -> Result<SharedGateState> {
        let bytes = match fs::read(&self.state_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(SharedGateState::default());
            }
            Err(err) => return Err(state_err(&self.state_path, err)),
        };
        jcs::reject_duplicate_names(&bytes).map_err(|err| {
            Error(format!(
                "gate state {}: {}",
                self.state_path.display(),
                err.0
            ))
        })?;
        let state: SharedGateState = serde_json::from_slice(&bytes)
            .map_err(|err| Error(format!("gate state {}: {err}", self.state_path.display())))?;
        if state.schema != GATE_STATE_SCHEMA {
            fail!(
                "gate state {}: unexpected schema {:?} (expected {GATE_STATE_SCHEMA:?})",
                self.state_path.display(),
                state.schema
            );
        }
        if state.next_lease_id == 0 {
            fail!(
                "gate state {}: next_lease_id must be at least 1",
                self.state_path.display()
            );
        }
        Ok(state)
    }

    fn save_locked(&self, state: &SharedGateState) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(state)?;
        bytes.push(b'\n');
        write_atomic(&self.state_path, &bytes).map_err(|err| {
            Error(format!(
                "gate state {}: {}",
                self.state_path.display(),
                err.0
            ))
        })
    }

    fn is_lease_lock_alive(&self, lock_slot: u64) -> Result<bool> {
        let path = self.leases_dir.join(format!("lease-{lock_slot}.lock"));
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(state_err(&path, err)),
        };
        match file.try_lock_shared() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(err)) => Err(state_err(&path, err)),
        }
    }

    fn prune_dead_leases_locked(&self, state: &mut SharedGateState) -> Result<bool> {
        let mut changed = false;

        let mut kept_pending = Vec::with_capacity(state.pending_staggers.len());
        for pending in state.pending_staggers.drain(..) {
            if self.is_lease_lock_alive(pending.lock_slot)? {
                kept_pending.push(pending);
            } else {
                changed = true;
            }
        }
        state.pending_staggers = kept_pending;

        let dead_retry_lanes: Vec<String> = {
            let mut lanes = Vec::new();
            for (lane, rl) in &state.rate_limits {
                if let LaneRateLimitState::RetryInFlight { lock_slot, .. } = rl
                    && !self.is_lease_lock_alive(*lock_slot)?
                {
                    lanes.push(lane.clone());
                }
            }
            lanes
        };
        for lane in dead_retry_lanes {
            if let Some(LaneRateLimitState::RetryInFlight {
                first_hit_at_s,
                prior_cooldown,
                ..
            }) = state.rate_limits.remove(&lane)
            {
                state.rate_limits.insert(
                    lane,
                    LaneRateLimitState::CoolingDown {
                        first_hit_at_s,
                        until_unix_s: prior_cooldown.until_unix_s,
                        retries_done: prior_cooldown.retries_done,
                        detail: prior_cooldown.detail,
                    },
                );
                changed = true;
            }
        }

        Ok(changed)
    }
}

fn state_err(path: &Path, err: io::Error) -> Error {
    Error(format!("gate state {}: {err}", path.display()))
}

// ---------------------------------------------------------------------------
// RunGate state machine
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunPhase {
    Pending,
    Launched,
}

struct ActiveRun {
    lane: String,
    lock_slot: u64,
    phase: RunPhase,
    _builder_slot: Option<File>,
    _lease_lock: File,
}

struct PendingNoticeWrite {
    path: PathBuf,
    lane: String,
    pool: String,
    first_hit_at_s: u64,
    second_hit_at_s: u64,
    retries_attempted: u64,
    detail: String,
}

/// Pre-start admission gate and pool-usage tracker for Bench v2 trial runners.
pub struct RunGate<
    C = SystemClock,
    M = SystemMachineProbe,
    L = FsLiveRunsReader,
    U = SystemUsageCommandRunner,
> {
    gate_id: u64,
    lanes: LanesFile,
    machine_limits: MachineLimits,
    config: GateConfig,
    builder_slots: BuilderSlotCoordinator,
    shared_state: SharedGateStateStore,
    clock: C,
    machine: M,
    live_runs: L,
    usage_runner: U,
    active_runs: BTreeMap<u64, ActiveRun>,
}

impl RunGate<SystemClock, SystemMachineProbe, FsLiveRunsReader, SystemUsageCommandRunner> {
    /// Constructs a production gate backed by the host clock, Linux `/proc` probe, read-only
    /// filesystem counter reader, and direct usage command runner.
    pub fn new(lanes: LanesFile, config: GateConfig) -> Result<Self> {
        Self::with_deps(
            lanes,
            config,
            SystemClock,
            SystemMachineProbe,
            FsLiveRunsReader,
            SystemUsageCommandRunner,
        )
    }
}

impl<C, M, L, U> RunGate<C, M, L, U>
where
    C: Clock,
    M: MachineProbe,
    L: LiveRunsReader,
    U: UsageCommandRunner,
{
    /// Constructs a gate with injectable clock, machine probe, live-runs reader, and command
    /// runner for deterministic unit testing.
    pub fn with_deps(
        lanes: LanesFile,
        config: GateConfig,
        clock: C,
        machine: M,
        live_runs: L,
        usage_runner: U,
    ) -> Result<Self> {
        config.validate()?;
        validate_lanes(&lanes)?;
        let Some(machine_limits) = lanes.machine.clone() else {
            fail!(
                "lanes file is missing required `machine` limits (`max_load_1m`, `min_available_mem_mib`)"
            );
        };
        for (name, lane) in &lanes.lanes {
            if lane.live_runs_counter.kind == LiveRunsCounterKind::LockFile
                && lane.usage_read.in_slot_command.is_none()
            {
                fail!(
                    "lanes.{name}: `lock_file` lane requires `usage_read.in_slot_command` so slot confirmation can refresh usage without reacquiring the held lock"
                );
            }
        }
        let builder_slots = BuilderSlotCoordinator::new(&config.state_dir);
        let shared_state = SharedGateStateStore::new(&config.state_dir);
        Ok(Self {
            gate_id: NEXT_GATE_ID.fetch_add(1, Ordering::Relaxed),
            lanes,
            machine_limits,
            config,
            builder_slots,
            shared_state,
            clock,
            machine,
            live_runs,
            usage_runner,
            active_runs: BTreeMap::new(),
        })
    }

    /// Number of currently reserved or launched runs tracked by this gate instance.
    pub fn active_run_count(&self) -> usize {
        self.active_runs.len()
    }

    /// Runs all pre-start checks for `req` and, if admitted, returns a [`GateReservation`].
    ///
    /// Does not start `req`'s wall-cap clock; callers acquire the external lane slot from the lane
    /// owner and then call [`Self::confirm_slot`] or [`Self::confirm_slot_with_probe`] (or
    /// [`Self::cancel`] if slot acquisition fails).
    pub fn try_reserve(&mut self, req: RunRequest) -> Result<ReserveDecision> {
        let mut machine_cache = None;
        self.try_reserve_inner(req, &mut machine_cache)
    }

    /// Polls `queue` in FIFO order without ever rewriting a request's lane (hold, never spill).
    ///
    /// Removes and returns the first request whose lane passes all pre-slot checks along with its
    /// original `queue_index`, preserving queue order for every held request. If external slot
    /// acquisition or confirmation later holds, callers receive the original [`RunRequest`] back
    /// from [`GateDecision::Hold`] or [`Self::cancel`] and can restore its exact lane and FIFO
    /// position with [`requeue_request`] (or use [`Self::poll_queue_acquire`] /
    /// [`Self::poll_queue_acquire_with_probe`] to keep the item in `queue` until confirmed).
    pub fn poll_queue(&mut self, queue: &mut VecDeque<RunRequest>) -> Result<PollOutcome> {
        let mut held = Vec::new();
        let mut machine_cache = None;
        for index in 0..queue.len() {
            let req = queue[index].clone();
            match self.try_reserve_inner(req.clone(), &mut machine_cache)? {
                ReserveDecision::Reserved(reservation) => {
                    queue.remove(index);
                    return Ok(PollOutcome::Reserved {
                        reservation,
                        queue_index: index,
                        held,
                    });
                }
                ReserveDecision::Hold(reason) => {
                    held.push((req.id, reason));
                }
            }
        }
        Ok(PollOutcome::AllHeld { held })
    }

    /// Polls `queue` in FIFO order, acquires the external lane slot, and confirms the grant before
    /// removing the request from `queue`.
    ///
    /// Any request that holds during reservation, external acquisition, or confirmation stays in
    /// `queue` at its exact original position and lane without allowing a later request on the
    /// same lane to leapfrog it.
    pub fn poll_queue_acquire<S, F>(
        &mut self,
        queue: &mut VecDeque<RunRequest>,
        acquire_external: F,
    ) -> Result<QueueAcquireOutcome<S>>
    where
        F: FnMut(&GateReservation) -> Result<SlotAcquisition<S>>,
    {
        self.poll_queue_acquire_inner(
            queue,
            acquire_external,
            None::<fn(&S, &Lane, Duration) -> Result<CommandOutput>>,
        )
    }

    /// Like [`Self::poll_queue_acquire`], using a caller-supplied held-slot usage probe if time
    /// elapses between reservation and confirmation.
    pub fn poll_queue_acquire_with_probe<S, F, P>(
        &mut self,
        queue: &mut VecDeque<RunRequest>,
        acquire_external: F,
        probe_in_slot: P,
    ) -> Result<QueueAcquireOutcome<S>>
    where
        F: FnMut(&GateReservation) -> Result<SlotAcquisition<S>>,
        P: FnMut(&S, &Lane, Duration) -> Result<CommandOutput>,
    {
        self.poll_queue_acquire_inner(queue, acquire_external, Some(probe_in_slot))
    }

    fn poll_queue_acquire_inner<S, F, P>(
        &mut self,
        queue: &mut VecDeque<RunRequest>,
        mut acquire_external: F,
        mut probe_in_slot: Option<P>,
    ) -> Result<QueueAcquireOutcome<S>>
    where
        F: FnMut(&GateReservation) -> Result<SlotAcquisition<S>>,
        P: FnMut(&S, &Lane, Duration) -> Result<CommandOutput>,
    {
        let mut held = Vec::new();
        let mut machine_cache = None;
        let mut blocked_lanes: BTreeMap<String, HoldReason> = BTreeMap::new();

        for index in 0..queue.len() {
            let req = queue[index].clone();
            if let Some(reason) = blocked_lanes.get(&req.lane) {
                held.push((req.id, reason.clone()));
                continue;
            }
            let reservation = match self.try_reserve_inner(req.clone(), &mut machine_cache)? {
                ReserveDecision::Reserved(reservation) => reservation,
                ReserveDecision::Hold(reason) => {
                    held.push((req.id, reason));
                    continue;
                }
            };
            let lane_name = reservation.request.lane.clone();
            let lane_cap = self.lane(&lane_name)?.run_cap;
            let acquired = acquire_external(&reservation);
            // External acquisition may block while waiting for a slot, so drop any cached machine
            // sample before inspecting subsequent queue entries.
            machine_cache = None;
            match acquired {
                Ok(SlotAcquisition::Acquired(slot)) => {
                    let decision = match probe_in_slot.as_mut() {
                        Some(probe) => self
                            .confirm_slot_with_probe(reservation, |lane, timeout| {
                                probe(&slot, lane, timeout)
                            })?,
                        None => self.confirm_slot(reservation)?,
                    };
                    match decision {
                        GateDecision::Granted(grant) => {
                            queue.remove(index);
                            return Ok(QueueAcquireOutcome::Granted { grant, slot, held });
                        }
                        GateDecision::Hold { request: _, reason } => {
                            drop(slot);
                            if !matches!(reason, HoldReason::BenchBuilderCap { .. }) {
                                blocked_lanes.insert(lane_name, reason.clone());
                            }
                            held.push((req.id, reason));
                        }
                    }
                }
                Ok(SlotAcquisition::Busy) => {
                    let _ = self.cancel(reservation)?;
                    let reason = HoldReason::LaneRunCap {
                        lane: lane_name.clone(),
                        live: lane_cap,
                        cap: lane_cap,
                    };
                    blocked_lanes.insert(lane_name, reason.clone());
                    held.push((req.id, reason));
                }
                Err(err) => {
                    let _ = self.cancel(reservation);
                    return Err(err);
                }
            }
        }
        Ok(QueueAcquireOutcome::AllHeld { held })
    }

    /// Finalizes a [`GateReservation`] AFTER the lane owner has acquired the external lane slot.
    ///
    /// Starts the [`WallCap`] clock at the confirmation timestamp (never during queue or lock
    /// waiting), rechecks lane rate-limit state, retry ownership, stagger, and run cap, and if
    /// time advanced between reservation and confirmation refreshes machine and pool readings using
    /// `usage_read.in_slot_command` (required at gate construction for `lock_file` lanes so
    /// re-running `usage_read.command` cannot deadlock on the held lock; callers may still override
    /// it via [`Self::confirm_slot_with_probe`]). On hold, rolls back cleanly and returns the
    /// original [`RunRequest`] inside [`GateDecision::Hold`].
    pub fn confirm_slot(&mut self, reservation: GateReservation) -> Result<GateDecision> {
        self.confirm_slot_with_optional_probe(
            reservation,
            None::<fn(&Lane, Duration) -> Result<CommandOutput>>,
        )
    }

    /// Like [`Self::confirm_slot`], invoking `probe_in_slot` when a usage refresh is needed while
    /// holding the external lane slot so serialized lanes can probe usage without reacquiring their
    /// account lock.
    pub fn confirm_slot_with_probe<P>(
        &mut self,
        reservation: GateReservation,
        probe_in_slot: P,
    ) -> Result<GateDecision>
    where
        P: FnOnce(&Lane, Duration) -> Result<CommandOutput>,
    {
        self.confirm_slot_with_optional_probe(reservation, Some(probe_in_slot))
    }

    fn confirm_slot_with_optional_probe<P>(
        &mut self,
        reservation: GateReservation,
        probe_in_slot: Option<P>,
    ) -> Result<GateDecision>
    where
        P: FnOnce(&Lane, Duration) -> Result<CommandOutput>,
    {
        self.verify_pending_reservation(&reservation)?;
        let grant_id = reservation.grant_id;
        match self.confirm_slot_inner(reservation, probe_in_slot) {
            Ok(GateDecision::Granted(grant)) => Ok(GateDecision::Granted(grant)),
            Ok(GateDecision::Hold { request, reason }) => {
                self.rollback_pending(grant_id)?;
                Ok(GateDecision::Hold { request, reason })
            }
            Err(err) => {
                let _ = self.rollback_pending(grant_id);
                Err(err)
            }
        }
    }

    /// Cancels an unconfirmed [`GateReservation`], releasing its bench-wide builder slot and
    /// restoring any pending stagger or rate-limit cooldown/retry allowance, and returns the
    /// original [`RunRequest`].
    ///
    /// Rejects unknown, foreign, or already-consumed reservations before mutating any state.
    pub fn cancel(&mut self, reservation: GateReservation) -> Result<RunRequest> {
        self.verify_pending_reservation(&reservation)?;
        self.rollback_pending(reservation.grant_id)?;
        Ok(reservation.request)
    }

    /// Convenience helper combining [`Self::try_reserve`], caller-supplied external lane-slot
    /// acquisition, and [`Self::confirm_slot`] (automatically cancelling on `Busy` or error and
    /// returning the original [`RunRequest`] on `Hold`).
    pub fn acquire_slot<S, F>(
        &mut self,
        req: RunRequest,
        acquire_external: F,
    ) -> Result<AcquireOutcome<S>>
    where
        F: FnOnce(&GateReservation) -> Result<SlotAcquisition<S>>,
    {
        self.acquire_slot_inner(
            req,
            acquire_external,
            None::<fn(&S, &Lane, Duration) -> Result<CommandOutput>>,
        )
    }

    /// Like [`Self::acquire_slot`], supplying `probe_in_slot` with `&slot` so confirmation can
    /// refresh usage using the already-held external slot without reacquiring its lock.
    pub fn acquire_slot_with_probe<S, F, P>(
        &mut self,
        req: RunRequest,
        acquire_external: F,
        probe_in_slot: P,
    ) -> Result<AcquireOutcome<S>>
    where
        F: FnOnce(&GateReservation) -> Result<SlotAcquisition<S>>,
        P: FnOnce(&S, &Lane, Duration) -> Result<CommandOutput>,
    {
        self.acquire_slot_inner(req, acquire_external, Some(probe_in_slot))
    }

    fn acquire_slot_inner<S, F, P>(
        &mut self,
        req: RunRequest,
        acquire_external: F,
        probe_in_slot: Option<P>,
    ) -> Result<AcquireOutcome<S>>
    where
        F: FnOnce(&GateReservation) -> Result<SlotAcquisition<S>>,
        P: FnOnce(&S, &Lane, Duration) -> Result<CommandOutput>,
    {
        let reservation = match self.try_reserve(req.clone())? {
            ReserveDecision::Hold(reason) => {
                return Ok(AcquireOutcome::Hold {
                    request: req,
                    reason,
                });
            }
            ReserveDecision::Reserved(reservation) => reservation,
        };
        let lane_name = reservation.request.lane.clone();
        let lane_cap = self.lane(&lane_name)?.run_cap;
        match acquire_external(&reservation) {
            Ok(SlotAcquisition::Acquired(slot)) => {
                let decision = match probe_in_slot {
                    Some(probe) => self.confirm_slot_with_probe(reservation, |lane, timeout| {
                        probe(&slot, lane, timeout)
                    })?,
                    None => self.confirm_slot(reservation)?,
                };
                match decision {
                    GateDecision::Granted(grant) => Ok(AcquireOutcome::Granted { grant, slot }),
                    GateDecision::Hold { request, reason } => {
                        drop(slot);
                        Ok(AcquireOutcome::Hold { request, reason })
                    }
                }
            }
            Ok(SlotAcquisition::Busy) => {
                let request = self.cancel(reservation)?;
                Ok(AcquireOutcome::Hold {
                    request,
                    reason: HoldReason::LaneRunCap {
                        lane: lane_name,
                        live: lane_cap,
                        cap: lane_cap,
                    },
                })
            }
            Err(err) => {
                let _ = self.cancel(reservation);
                Err(err)
            }
        }
    }

    /// Completes a confirmed [`GateGrant`] AFTER the lane owner has released the external slot.
    ///
    /// Releases the in-process lease and shared builder slot first (so a failed after-reading
    /// never leaks capacity), persists rate-limit transitions in shared gate state while keeping
    /// the lease liveness lock held, writes any atomic lead notice, and captures the post-run
    /// [`PoolSnapshot`] for [`PoolReading`]. Rejects unknown, foreign, or already-consumed grants
    /// before mutating any state.
    pub fn finish(&mut self, grant: GateGrant, completion: RunCompletion) -> Result<FinishOutcome> {
        self.verify_launched_grant(&grant)?;
        let now_s = self.clock.now_unix_s();
        let Some(active) = self.active_runs.remove(&grant.grant_id) else {
            fail!(
                "grant for {:?} is unknown or already consumed",
                grant.request.id
            );
        };
        // Release the builder slot immediately so no error can leak builder capacity, while keeping
        // `_lease_lock` alive until the shared rate-limit transition commits.
        drop(active._builder_slot);

        let lane_name = grant.request.lane.clone();
        let pool_name = grant.pool.clone();

        let transition = self.apply_completion_rate_limit(
            grant.grant_id,
            &lane_name,
            &pool_name,
            now_s,
            completion,
        );
        drop(active._lease_lock);
        let (rate_limit, pending_notice) = transition?;

        let notice_error = pending_notice.and_then(|notice| {
            write_rate_limit_notice(
                &self.config.notice_dir,
                &notice.path,
                NoticePayload {
                    lane: &notice.lane,
                    pool: &notice.pool,
                    first_hit_at_s: notice.first_hit_at_s,
                    second_hit_at_s: notice.second_hit_at_s,
                    hold_seconds: self.config.rate_limit_hold_s,
                    retries_attempted: notice.retries_attempted,
                    detail: notice.detail,
                },
            )
            .err()
        });

        let after_s = now_s.max(grant.granted_at_s);
        let (after_snapshot, after_reading_error) =
            match self.capture_after_snapshot(&lane_name, &pool_name, after_s) {
                Ok(snapshot) => (Some(snapshot), None),
                Err(err) => (None, Some(err)),
            };

        if let Some(err) = notice_error {
            return Err(err);
        }

        let pool = Text::new(&pool_name)?;
        Ok(FinishOutcome {
            pool_reading: PoolReading {
                pool,
                before: Some(grant.before_snapshot),
                after: after_snapshot,
            },
            after_reading_error,
            rate_limit,
        })
    }

    /// Clears a `HeldForLead` rate-limit latch in shared gate state after the lead has inspected
    /// the pool. Returns `Ok(true)` if the lane was held for lead, or `Ok(false)` otherwise.
    pub fn clear_rate_limit_hold(&mut self, lane_name: &str) -> Result<bool> {
        let _ = self.lane(lane_name)?;
        self.shared_state.with_state_mut(|state, _| {
            if matches!(
                state.rate_limits.get(lane_name),
                Some(LaneRateLimitState::HeldForLead { .. })
            ) {
                state.rate_limits.remove(lane_name);
                return Ok((true, true));
            }
            Ok((false, false))
        })
    }

    fn try_reserve_inner(
        &mut self,
        req: RunRequest,
        machine_cache: &mut Option<MachineSample>,
    ) -> Result<ReserveDecision> {
        // 1. Validate request against configured lanes.
        let lane = self.lane(&req.lane)?.clone();
        if req.wall_cap_s == 0 {
            fail!("run request {:?} wall_cap_s must be at least 1", req.id);
        }
        let now_s = self.clock.now_unix_s();
        let pool = lane.pool.as_str().to_owned();
        let account = lane.account_key(&req.lane).to_owned();

        // 2. Shared lane rate-limit state and per-account stagger pre-check.
        let pre_hold = self.shared_state.with_state_mut(|state, _| {
            if let Some(hold) = state.check_rate_limit_for_reserve(&req.lane, now_s) {
                return Ok((Some(hold), false));
            }
            Ok((
                state.check_stagger(&account, lane.stagger_s, now_s, None),
                false,
            ))
        })?;
        if let Some(hold) = pre_hold {
            return Ok(ReserveDecision::Hold(hold));
        }

        // 3. Bench-wide builder cap pre-check (cheap local flock scan before external usage read).
        if req.role == TrialRole::Builder {
            let active = self.builder_slots.probe_active()?;
            if active >= self.config.max_concurrent_builders {
                return Ok(ReserveDecision::Hold(HoldReason::BenchBuilderCap {
                    active,
                    cap: self.config.max_concurrent_builders,
                }));
            }
        }

        // 4. Machine load and memory limits.
        let machine_sample = match machine_cache {
            Some(cached) => cached.clone(),
            None => {
                let fresh = self.machine.sample()?;
                *machine_cache = Some(fresh.clone());
                fresh
            }
        };
        if let Some(hold) = self.check_machine(&machine_sample) {
            return Ok(ReserveDecision::Hold(hold));
        }

        // 5. Lane run cap (external read-only aggregate counter + in-process leases).
        let live_sample = self
            .live_runs
            .read_live_runs(&req.lane, &lane.live_runs_counter)?;
        let lane_live = self.lane_live_for_reserve(&req.lane, &lane, live_sample);
        if lane_live >= lane.run_cap {
            return Ok(ReserveDecision::Hold(HoldReason::LaneRunCap {
                lane: req.lane,
                live: lane_live,
                cap: lane.run_cap,
            }));
        }

        // Require a pool-attributable counter for usage snapshots before admitting.
        if lane.pool_live_runs_counter.is_none() {
            self.ensure_pool_attributable(
                &pool,
                &req.lane,
                &lane.live_runs_counter,
                live_sample.scope,
            )?;
        }

        // 6. Pool usage read and floor check.
        let meters = self.read_and_parse_usage(&req.lane, &lane, &lane.usage_read.command)?;
        if let Some(hold) = check_floors(&req.lane, &pool, &lane.floors, &meters)? {
            return Ok(ReserveDecision::Hold(hold));
        }

        // 7. Acquire bench-wide builder slot atomically before committing shared gate state.
        let builder_slot = if req.role == TrialRole::Builder {
            match self
                .builder_slots
                .try_acquire(self.config.max_concurrent_builders)?
            {
                BuilderSlotOutcome::Acquired(file) => Some(file),
                BuilderSlotOutcome::AtCap { active } => {
                    return Ok(ReserveDecision::Hold(HoldReason::BenchBuilderCap {
                        active,
                        cap: self.config.max_concurrent_builders,
                    }));
                }
            }
        } else {
            None
        };

        // 8. Atomically recheck shared rate-limit/stagger state and commit pending lease.
        let commit_now_s = self.clock.now_unix_s().max(now_s);
        let commit = self.shared_state.with_state_mut(|state, leases| {
            if let Some(hold) = state.check_rate_limit_for_reserve(&req.lane, commit_now_s) {
                return Ok((Err(hold), false));
            }
            if let Some(hold) = state.check_stagger(&account, lane.stagger_s, commit_now_s, None) {
                return Ok((Err(hold), false));
            }
            let (lock_slot, lease_lock) = leases.acquire()?;
            let lease_id = state.allocate_lease_id()?;
            if matches!(
                state.rate_limits.get(&req.lane),
                Some(LaneRateLimitState::CoolingDown { until_unix_s, .. })
                    if commit_now_s >= *until_unix_s
            ) {
                state.promote_cooldown_to_retry(&req.lane, lease_id, lock_slot);
            }
            state.pending_staggers.push(PendingStagger {
                lease_id,
                lock_slot,
                account: account.clone(),
                stagger_s: lane.stagger_s,
                checked_at_s: commit_now_s,
            });
            Ok((Ok((lease_id, lock_slot, lease_lock)), true))
        })?;

        let (lease_id, lock_slot, lease_lock) = match commit {
            Ok(tuple) => tuple,
            Err(hold) => {
                drop(builder_slot);
                return Ok(ReserveDecision::Hold(hold));
            }
        };

        self.active_runs.insert(
            lease_id,
            ActiveRun {
                lane: req.lane.clone(),
                lock_slot,
                phase: RunPhase::Pending,
                _builder_slot: builder_slot,
                _lease_lock: lease_lock,
            },
        );

        Ok(ReserveDecision::Reserved(GateReservation {
            gate_id: self.gate_id,
            grant_id: lease_id,
            request: req,
            pool,
            account,
            checked_at_s: commit_now_s,
            checked_load: machine_sample.load,
            checked_meters: meters,
        }))
    }

    fn confirm_slot_inner<P>(
        &mut self,
        reservation: GateReservation,
        probe_in_slot: Option<P>,
    ) -> Result<GateDecision>
    where
        P: FnOnce(&Lane, Duration) -> Result<CommandOutput>,
    {
        let lane = self.lane(&reservation.request.lane)?.clone();
        let raw_now_s = self.clock.now_unix_s();
        let granted_at_s = raw_now_s.max(reservation.checked_at_s);
        let lock_slot = self
            .active_runs
            .get(&reservation.grant_id)
            .map(|entry| entry.lock_slot)
            .ok_or_else(|| {
                Error(format!(
                    "reservation for {:?} is unknown or already consumed",
                    reservation.request.id
                ))
            })?;

        // 1. Pre-probe check of shared rate-limit state (cooldown, retry ownership, lead hold) and
        //    account stagger before invoking any external probe.
        let pre_hold = self.shared_state.with_state_mut(|state, _| {
            if let Some(hold) = state.check_rate_limit_for_confirm(
                &reservation.request.lane,
                reservation.grant_id,
                granted_at_s,
            ) {
                return Ok((Some(hold), false));
            }
            Ok((
                state.check_stagger(
                    &reservation.account,
                    lane.stagger_s,
                    granted_at_s,
                    Some(reservation.grant_id),
                ),
                false,
            ))
        })?;
        if let Some(hold) = pre_hold {
            return Ok(GateDecision::Hold {
                request: reservation.request,
                reason: hold,
            });
        }

        // 2. Re-read the lane's external counter (now including this slot) and verify no contention
        //    oversubscribed the lane between checking and slot confirmation.
        let live_sample = self
            .live_runs
            .read_live_runs(&reservation.request.lane, &lane.live_runs_counter)?;
        let lane_live_with_self = self.lane_live_for_confirm(
            &reservation.request.lane,
            &lane,
            live_sample,
            reservation.grant_id,
        );
        if lane_live_with_self > lane.run_cap {
            let lane_name = reservation.request.lane.clone();
            return Ok(GateDecision::Hold {
                request: reservation.request,
                reason: HoldReason::LaneRunCap {
                    lane: lane_name,
                    live: lane_live_with_self.saturating_sub(1),
                    cap: lane.run_cap,
                },
            });
        }

        // 3. If time elapsed between checking and slot confirmation, refresh machine and pool
        //    readings using a probe that can operate inside the held slot without reacquiring it.
        let (machine_load, meters) = if granted_at_s > reservation.checked_at_s {
            let sample = self.machine.sample()?;
            if let Some(hold) = self.check_machine(&sample) {
                return Ok(GateDecision::Hold {
                    request: reservation.request,
                    reason: hold,
                });
            }
            let fresh_meters = match probe_in_slot {
                Some(probe) => {
                    let output =
                        probe(&lane, self.config.usage_command_timeout).map_err(|err| {
                            Error(format!("lanes.{}: {}", reservation.request.lane, err.0))
                        })?;
                    parse_usage_command_output(&reservation.request.lane, &lane, &output)?
                }
                None => self.read_and_parse_usage_in_slot(&reservation.request.lane, &lane)?,
            };
            if let Some(hold) = check_floors(
                &reservation.request.lane,
                &reservation.pool,
                &lane.floors,
                &fresh_meters,
            )? {
                return Ok(GateDecision::Hold {
                    request: reservation.request,
                    reason: hold,
                });
            }
            (sample.load, fresh_meters)
        } else {
            (reservation.checked_load, reservation.checked_meters)
        };

        // 4. Refresh the grant timestamp after any external refresh probe, then compute
        //    pool-attributable other_live_runs for before_snapshot before committing state.
        let granted_at_s = self.clock.now_unix_s().max(granted_at_s);
        let at = unix_s_to_utc(granted_at_s)?;
        let other_live_runs = self.count_other_live_runs(
            &reservation.pool,
            &reservation.request.lane,
            Some(live_sample),
            true,
        )?;

        // 5. Final atomic commit in shared gate state: recheck rate-limit & stagger (catching
        //    concurrent 429s or starts during step 3), promote elapsed cooldown to RetryInFlight,
        //    remove pending stagger, and record confirmed account start.
        let commit_hold = self.shared_state.with_state_mut(|state, _| {
            if let Some(hold) = state.check_rate_limit_for_confirm(
                &reservation.request.lane,
                reservation.grant_id,
                granted_at_s,
            ) {
                return Ok((Some(hold), false));
            }
            if let Some(hold) = state.check_stagger(
                &reservation.account,
                lane.stagger_s,
                granted_at_s,
                Some(reservation.grant_id),
            ) {
                return Ok((Some(hold), false));
            }
            if matches!(
                state.rate_limits.get(&reservation.request.lane),
                Some(LaneRateLimitState::CoolingDown { until_unix_s, .. })
                    if granted_at_s >= *until_unix_s
            ) {
                state.promote_cooldown_to_retry(
                    &reservation.request.lane,
                    reservation.grant_id,
                    lock_slot,
                );
            }
            state
                .pending_staggers
                .retain(|pending| pending.lease_id != reservation.grant_id);
            state.record_account_start(&reservation.account, granted_at_s, lane.stagger_s);
            Ok((None, true))
        })?;
        if let Some(hold) = commit_hold {
            return Ok(GateDecision::Hold {
                request: reservation.request,
                reason: hold,
            });
        }

        if let Some(entry) = self.active_runs.get_mut(&reservation.grant_id) {
            entry.phase = RunPhase::Launched;
        }

        let wall_cap = WallCap::start(granted_at_s, reservation.request.wall_cap_s);
        let before_snapshot = PoolSnapshot {
            at,
            meters,
            other_live_runs,
        };

        Ok(GateDecision::Granted(GateGrant {
            gate_id: self.gate_id,
            grant_id: reservation.grant_id,
            request: reservation.request,
            pool: reservation.pool,
            account: reservation.account,
            granted_at_s,
            wall_cap,
            machine_load,
            before_snapshot,
        }))
    }

    fn verify_pending_reservation(&self, reservation: &GateReservation) -> Result<()> {
        self.verify_lease(
            "reservation",
            reservation.gate_id,
            reservation.grant_id,
            &reservation.request.id,
            RunPhase::Pending,
        )
    }

    fn verify_launched_grant(&self, grant: &GateGrant) -> Result<()> {
        self.verify_lease(
            "grant",
            grant.gate_id,
            grant.grant_id,
            &grant.request.id,
            RunPhase::Launched,
        )
    }

    fn verify_lease(
        &self,
        kind: &str,
        gate_id: u64,
        grant_id: u64,
        request_id: &str,
        expected_phase: RunPhase,
    ) -> Result<()> {
        if gate_id != self.gate_id {
            fail!("{kind} for {request_id:?} belongs to a different RunGate instance");
        }
        match self.active_runs.get(&grant_id) {
            Some(entry) if entry.phase == expected_phase => Ok(()),
            _ => fail!("{kind} for {request_id:?} is unknown or already consumed"),
        }
    }

    fn rollback_pending(&mut self, grant_id: u64) -> Result<()> {
        let Some(removed) = self.active_runs.remove(&grant_id) else {
            return Ok(());
        };
        drop(removed._builder_slot);
        // Keep `_lease_lock` alive across the shared-state rollback so another process cannot
        // prune this lease concurrently while we restore any RetryInFlight cooldown.
        let result = self.shared_state.with_state_mut(|state, _| {
            let mut mutated = false;
            let before_len = state.pending_staggers.len();
            state
                .pending_staggers
                .retain(|pending| pending.lease_id != grant_id);
            if state.pending_staggers.len() != before_len {
                mutated = true;
            }
            if let Some(LaneRateLimitState::RetryInFlight {
                lease_id: retry_id, ..
            }) = state.rate_limits.get(&removed.lane)
                && *retry_id == grant_id
                && let Some(LaneRateLimitState::RetryInFlight {
                    first_hit_at_s,
                    prior_cooldown,
                    ..
                }) = state.rate_limits.remove(&removed.lane)
            {
                state.rate_limits.insert(
                    removed.lane.clone(),
                    LaneRateLimitState::CoolingDown {
                        first_hit_at_s,
                        until_unix_s: prior_cooldown.until_unix_s,
                        retries_done: prior_cooldown.retries_done,
                        detail: prior_cooldown.detail,
                    },
                );
                mutated = true;
            }
            Ok(((), mutated))
        });
        drop(removed._lease_lock);
        result
    }

    fn lane(&self, lane_name: &str) -> Result<&Lane> {
        let key = Text::new(lane_name).map_err(|_| Error(format!("unknown lane {lane_name:?}")))?;
        self.lanes
            .lanes
            .get(&key)
            .ok_or_else(|| Error(format!("unknown lane {lane_name:?}")))
    }

    fn check_machine(&self, sample: &MachineSample) -> Option<HoldReason> {
        if sample.load.one > self.machine_limits.max_load_1m {
            return Some(HoldReason::MachineLoad {
                load_1m: sample.load.one,
                max_load_1m: self.machine_limits.max_load_1m,
            });
        }
        let min_kib = self.machine_limits.min_available_mem_mib * 1024;
        if sample.available_kib < min_kib {
            return Some(HoldReason::MachineMemory {
                available_mib: sample.available_kib / 1024,
                min_available_mib: self.machine_limits.min_available_mem_mib,
            });
        }
        None
    }

    fn lane_live_for_reserve(&self, lane_name: &str, lane: &Lane, sample: LiveRunsSample) -> u64 {
        let (launched, pending) = self.in_process_counts(lane_name, lane, sample.scope, None);
        sample.count.max(launched).saturating_add(pending)
    }

    fn lane_live_for_confirm(
        &self,
        lane_name: &str,
        lane: &Lane,
        sample: LiveRunsSample,
        confirming_grant_id: u64,
    ) -> u64 {
        let (other_launched, other_pending) =
            self.in_process_counts(lane_name, lane, sample.scope, Some(confirming_grant_id));
        let launched_with_self = other_launched.saturating_add(1);
        sample
            .count
            .max(launched_with_self)
            .saturating_add(other_pending)
    }

    fn in_process_counts(
        &self,
        lane_name: &str,
        lane: &Lane,
        scope: CounterScope,
        exclude_grant_id: Option<u64>,
    ) -> (u64, u64) {
        let target_key =
            (scope == CounterScope::SharedPath).then(|| counter_key(&lane.live_runs_counter));
        let mut launched = 0u64;
        let mut pending = 0u64;
        for (&id, run) in &self.active_runs {
            if exclude_grant_id == Some(id) {
                continue;
            }
            let matches = match &target_key {
                None => run.lane == lane_name,
                Some(key) => self
                    .lane(&run.lane)
                    .ok()
                    .is_some_and(|other| counter_key(&other.live_runs_counter) == *key),
            };
            if matches {
                match run.phase {
                    RunPhase::Launched => launched = launched.saturating_add(1),
                    RunPhase::Pending => pending = pending.saturating_add(1),
                }
            }
        }
        (launched, pending)
    }

    fn ensure_pool_attributable(
        &self,
        pool: &str,
        lane_name: &str,
        counter: &LiveRunsCounter,
        scope: CounterScope,
    ) -> Result<()> {
        match scope {
            CounterScope::Lane => Ok(()),
            CounterScope::SharedPath => {
                let key = counter_key(counter);
                let mut other_pools = BTreeSet::new();
                for other_lane in self.lanes.lanes.values() {
                    if other_lane.pool.as_str() == pool {
                        continue;
                    }
                    let shares_live = counter_key(&other_lane.live_runs_counter) == key;
                    let shares_pool = other_lane
                        .pool_live_runs_counter
                        .as_ref()
                        .is_some_and(|other_counter| counter_key(other_counter) == key);
                    if shares_live || shares_pool {
                        other_pools.insert(other_lane.pool.as_str());
                    }
                }
                if !other_pools.is_empty() {
                    let pools = other_pools.into_iter().collect::<Vec<_>>().join(", ");
                    fail!(
                        "lanes.{lane_name}: shared counter {:?} also covers other pool(s) ({pools}) and cannot be attributed to pool {pool:?} for usage snapshots; configure per-lane counts or `pool_live_runs_counter`",
                        counter.path.as_str()
                    );
                }
                Ok(())
            }
        }
    }

    fn count_other_live_runs(
        &self,
        pool: &str,
        target_lane_name: &str,
        cached_target: Option<LiveRunsSample>,
        self_launched: bool,
    ) -> Result<u64> {
        // Order the target lane first so its cached SharedPath sample seeds shared_groups before
        // any sibling lane pointing to the same counter path is read.
        let mut pool_lanes: Vec<(&str, &Lane)> = self
            .lanes
            .lanes
            .iter()
            .filter(|(_, lane)| lane.pool.as_str() == pool)
            .map(|(name, lane)| (name.as_str(), lane))
            .collect();
        if let Some(pos) = pool_lanes
            .iter()
            .position(|(name, _)| *name == target_lane_name)
        {
            pool_lanes.swap(0, pos);
        }

        let mut shared_groups: BTreeMap<(LiveRunsCounterKind, PathBuf), (u64, u64)> =
            BTreeMap::new();
        let mut lane_scoped_total = 0u64;

        for (lane_name, lane) in pool_lanes {
            let mut launched_on_lane = self
                .active_runs
                .values()
                .filter(|run| run.phase == RunPhase::Launched && run.lane == lane_name)
                .count() as u64;
            if self_launched && lane_name == target_lane_name {
                launched_on_lane = launched_on_lane.saturating_add(1);
            }

            let snapshot_counter = lane
                .pool_live_runs_counter
                .as_ref()
                .unwrap_or(&lane.live_runs_counter);
            let key = counter_key(snapshot_counter);
            if let Some((_, launched_sum)) = shared_groups.get_mut(&key) {
                *launched_sum = launched_sum.saturating_add(launched_on_lane);
                continue;
            }

            let can_use_cached = lane_name == target_lane_name
                && key == counter_key(&lane.live_runs_counter)
                && cached_target.is_some();
            let sample = if can_use_cached {
                cached_target.expect("checked is_some")
            } else {
                self.live_runs.read_live_runs(lane_name, snapshot_counter)?
            };

            self.ensure_pool_attributable(pool, lane_name, snapshot_counter, sample.scope)?;

            match sample.scope {
                CounterScope::SharedPath => {
                    shared_groups.insert(key, (sample.count, launched_on_lane));
                }
                CounterScope::Lane => {
                    lane_scoped_total =
                        lane_scoped_total.saturating_add(sample.count.max(launched_on_lane));
                }
            }
        }

        let shared_total: u64 = shared_groups
            .into_values()
            .map(|(external, launched_sum)| external.max(launched_sum))
            .fold(0u64, u64::saturating_add);
        let total = lane_scoped_total.saturating_add(shared_total);
        Ok(if self_launched {
            total.saturating_sub(1)
        } else {
            total
        })
    }

    fn read_and_parse_usage(
        &self,
        lane_name: &str,
        lane: &Lane,
        command: &[Text],
    ) -> Result<BTreeMap<Text, f64>> {
        let cmd: Vec<String> = command.iter().map(|arg| arg.as_str().to_owned()).collect();
        let output = self
            .usage_runner
            .run(&cmd, self.config.usage_command_timeout)
            .map_err(|err| Error(format!("lanes.{lane_name}: {}", err.0)))?;
        parse_usage_command_output(lane_name, lane, &output)
    }

    fn read_and_parse_usage_in_slot(
        &self,
        lane_name: &str,
        lane: &Lane,
    ) -> Result<BTreeMap<Text, f64>> {
        if let Some(in_slot) = &lane.usage_read.in_slot_command {
            return self.read_and_parse_usage(lane_name, lane, in_slot);
        }
        if lane.live_runs_counter.kind == LiveRunsCounterKind::LockFile {
            fail!(
                "lanes.{lane_name}: `lock_file` lane is missing required `usage_read.in_slot_command`"
            );
        }
        self.read_and_parse_usage(lane_name, lane, &lane.usage_read.command)
    }

    fn capture_after_snapshot(
        &self,
        lane_name: &str,
        pool: &str,
        after_s: u64,
    ) -> Result<PoolSnapshot> {
        let lane = self.lane(lane_name)?;
        let meters = self.read_and_parse_usage(lane_name, lane, &lane.usage_read.command)?;
        let other_live_runs = self.count_other_live_runs(pool, lane_name, None, false)?;
        let at = unix_s_to_utc(after_s)?;
        Ok(PoolSnapshot {
            at,
            meters,
            other_live_runs,
        })
    }

    fn apply_completion_rate_limit(
        &mut self,
        lease_id: u64,
        lane_name: &str,
        pool_name: &str,
        now_s: u64,
        completion: RunCompletion,
    ) -> Result<(Option<RateLimitAction>, Option<PendingNoticeWrite>)> {
        let hold_s = self.config.rate_limit_hold_s;
        let max_retries = self.config.rate_limit_max_retries;
        let notice_dir = self.config.notice_dir.clone();

        self.shared_state
            .with_state_mut(|state, _| match completion {
                RunCompletion::Completed => {
                    if let Some(LaneRateLimitState::RetryInFlight {
                        lease_id: retry_id, ..
                    }) = state.rate_limits.get(lane_name)
                        && *retry_id == lease_id
                    {
                        state.rate_limits.remove(lane_name);
                        return Ok((
                            (
                                Some(RateLimitAction::Cleared {
                                    lane: lane_name.to_owned(),
                                }),
                                None,
                            ),
                            true,
                        ));
                    }
                    Ok(((None, None), false))
                }
                RunCompletion::RateLimited { detail } => {
                    let next_until = now_s.saturating_add(hold_s);
                    match state.rate_limits.remove(lane_name) {
                        None => {
                            if max_retries == 0 {
                                let res = latch_held_in_state(
                                    state,
                                    &notice_dir,
                                    lane_name,
                                    pool_name,
                                    now_s,
                                    now_s,
                                    0,
                                    detail,
                                );
                                return Ok((res, true));
                            }
                            let action = enter_cooldown_in_state(
                                state,
                                lane_name,
                                now_s,
                                next_until,
                                0,
                                max_retries,
                                detail,
                            );
                            Ok(((Some(action), None), true))
                        }
                        Some(LaneRateLimitState::CoolingDown {
                            first_hit_at_s,
                            until_unix_s,
                            retries_done,
                            ..
                        }) => {
                            let action = enter_cooldown_in_state(
                                state,
                                lane_name,
                                first_hit_at_s,
                                until_unix_s.max(next_until),
                                retries_done,
                                max_retries,
                                detail,
                            );
                            Ok(((Some(action), None), true))
                        }
                        Some(LaneRateLimitState::RetryInFlight {
                            lease_id: retry_id,
                            lock_slot,
                            first_hit_at_s,
                            retries_done,
                            mut prior_cooldown,
                        }) => {
                            if retry_id != lease_id {
                                prior_cooldown.until_unix_s =
                                    prior_cooldown.until_unix_s.max(next_until);
                                prior_cooldown.detail = detail;
                                state.rate_limits.insert(
                                    lane_name.to_owned(),
                                    LaneRateLimitState::RetryInFlight {
                                        lease_id: retry_id,
                                        lock_slot,
                                        first_hit_at_s,
                                        retries_done,
                                        prior_cooldown,
                                    },
                                );
                                return Ok(((None, None), true));
                            }
                            if retries_done < max_retries {
                                let action = enter_cooldown_in_state(
                                    state,
                                    lane_name,
                                    first_hit_at_s,
                                    next_until,
                                    retries_done,
                                    max_retries,
                                    detail,
                                );
                                return Ok(((Some(action), None), true));
                            }
                            let res = latch_held_in_state(
                                state,
                                &notice_dir,
                                lane_name,
                                pool_name,
                                first_hit_at_s,
                                now_s,
                                retries_done,
                                detail,
                            );
                            Ok((res, true))
                        }
                        Some(LaneRateLimitState::HeldForLead { notice_path }) => {
                            let ret_path = notice_path.clone();
                            state.rate_limits.insert(
                                lane_name.to_owned(),
                                LaneRateLimitState::HeldForLead { notice_path },
                            );
                            Ok((
                                (
                                    Some(RateLimitAction::HeldForLead {
                                        lane: lane_name.to_owned(),
                                        notice_path: ret_path,
                                    }),
                                    None,
                                ),
                                false,
                            ))
                        }
                    }
                }
            })
    }
}

fn parse_usage_command_output(
    lane_name: &str,
    lane: &Lane,
    output: &CommandOutput,
) -> Result<BTreeMap<Text, f64>> {
    if !output.success {
        let code = output
            .status_code
            .map_or_else(|| "signal".to_owned(), |c| c.to_string());
        fail!(
            "lanes.{lane_name}: usage_read command exited with status {code}: {}",
            output.stderr.trim()
        );
    }
    parse_usage(
        lane.usage_read.parser,
        lane.usage_read.group.as_ref().map(Text::as_str),
        &output.stdout,
    )
    .map_err(|err| Error(format!("lanes.{lane_name}: {}", err.0)))
}

fn enter_cooldown_in_state(
    state: &mut SharedGateState,
    lane_name: &str,
    first_hit_at_s: u64,
    until_unix_s: u64,
    retries_done: u64,
    max_retries: u64,
    detail: String,
) -> RateLimitAction {
    state.rate_limits.insert(
        lane_name.to_owned(),
        LaneRateLimitState::CoolingDown {
            first_hit_at_s,
            until_unix_s,
            retries_done,
            detail,
        },
    );
    RateLimitAction::Cooldown {
        lane: lane_name.to_owned(),
        until_unix_s,
        retries_remaining: max_retries.saturating_sub(retries_done),
    }
}

#[allow(clippy::too_many_arguments)]
fn latch_held_in_state(
    state: &mut SharedGateState,
    notice_dir: &Path,
    lane_name: &str,
    pool_name: &str,
    first_hit_at_s: u64,
    second_hit_at_s: u64,
    retries_attempted: u64,
    detail: String,
) -> (Option<RateLimitAction>, Option<PendingNoticeWrite>) {
    let notice_path = notice_file_path(notice_dir, lane_name);
    // Latch HeldForLead in shared state before writing the notice file so every gate stays
    // fail-closed even if notice I/O fails.
    state.rate_limits.insert(
        lane_name.to_owned(),
        LaneRateLimitState::HeldForLead {
            notice_path: notice_path.clone(),
        },
    );
    (
        Some(RateLimitAction::HeldForLead {
            lane: lane_name.to_owned(),
            notice_path: notice_path.clone(),
        }),
        Some(PendingNoticeWrite {
            path: notice_path,
            lane: lane_name.to_owned(),
            pool: pool_name.to_owned(),
            first_hit_at_s,
            second_hit_at_s,
            retries_attempted,
            detail,
        }),
    )
}

fn check_floors(
    lane_name: &str,
    pool: &str,
    floors: &BTreeMap<Text, f64>,
    meters: &BTreeMap<Text, f64>,
) -> Result<Option<HoldReason>> {
    for (meter, &floor) in floors {
        let Some(&value) = meters.get(meter) else {
            fail!(
                "lanes.{lane_name}: usage_read floor meter {:?} missing from reading",
                meter.as_str()
            );
        };
        if value < floor {
            return Ok(Some(HoldReason::PoolBelowFloor {
                pool: pool.to_owned(),
                meter: meter.as_str().to_owned(),
                value,
                floor,
            }));
        }
    }
    Ok(None)
}

fn counter_key(counter: &LiveRunsCounter) -> (LiveRunsCounterKind, PathBuf) {
    let normalized: PathBuf = Path::new(counter.path.as_str()).components().collect();
    (counter.kind, normalized)
}

fn unix_s_to_utc(seconds: u64) -> Result<Utc> {
    let signed = i64::try_from(seconds)
        .map_err(|_| Error(format!("unix timestamp {seconds} exceeds i64 range")))?;
    Utc::new(time::format_utc(signed))
}

fn notice_file_path(notice_dir: &Path, lane_name: &str) -> PathBuf {
    let safe_lane: String = lane_name
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = sha256_hex(lane_name.as_bytes());
    notice_dir.join(format!("rate-limit-{safe_lane}-{}.json", &digest[..8]))
}

struct NoticePayload<'a> {
    lane: &'a str,
    pool: &'a str,
    first_hit_at_s: u64,
    second_hit_at_s: u64,
    hold_seconds: u64,
    retries_attempted: u64,
    detail: String,
}

fn write_rate_limit_notice(
    notice_dir: &Path,
    path: &Path,
    payload: NoticePayload<'_>,
) -> Result<()> {
    fs::create_dir_all(notice_dir)
        .map_err(|err| Error(format!("notice_dir {}: {err}", notice_dir.display())))?;
    let notice = RateLimitNotice {
        schema: NOTICE_SCHEMA.to_owned(),
        kind: "rate_limit_hold".to_owned(),
        lane: payload.lane.to_owned(),
        pool: payload.pool.to_owned(),
        first_hit_at: unix_s_to_utc(payload.first_hit_at_s)?.as_str().to_owned(),
        second_hit_at: unix_s_to_utc(payload.second_hit_at_s)?.as_str().to_owned(),
        hold_seconds: payload.hold_seconds,
        retries_attempted: payload.retries_attempted,
        detail: payload.detail,
        action_required: NOTICE_ACTION_REQUIRED.to_owned(),
    };
    let mut bytes = serde_json::to_vec_pretty(&notice)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::tests::{fixture, run};
    use crate::trial::{LANES_SCHEMA, parse_lanes};
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FakeClock(Arc<AtomicU64>);

    impl FakeClock {
        fn new(start: u64) -> Self {
            Self(Arc::new(AtomicU64::new(start)))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::Relaxed);
        }

        fn advance(&self, delta: u64) {
            self.0.fetch_add(delta, Ordering::Relaxed);
        }
    }

    impl Clock for FakeClock {
        fn now_unix_s(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[derive(Clone)]
    struct FakeMachine(Rc<RefCell<Result<MachineSample>>>);

    impl FakeMachine {
        fn healthy() -> Self {
            Self::ok(2.0, 16_384 * 1024)
        }

        fn ok(load_1m: f64, available_kib: u64) -> Self {
            Self(Rc::new(RefCell::new(Ok(MachineSample {
                load: Load {
                    one: load_1m,
                    five: load_1m,
                    fifteen: load_1m,
                },
                available_kib,
            }))))
        }

        fn set(&self, sample: Result<MachineSample>) {
            *self.0.borrow_mut() = sample;
        }
    }

    impl MachineProbe for FakeMachine {
        fn sample(&self) -> Result<MachineSample> {
            self.0.borrow().clone()
        }
    }

    #[derive(Clone, Default)]
    struct FakeLiveRuns(Rc<RefCell<BTreeMap<String, LiveRunsSample>>>);

    impl FakeLiveRuns {
        fn set(&self, lane: &str, count: u64, scope: CounterScope) {
            self.0
                .borrow_mut()
                .insert(lane.to_owned(), LiveRunsSample { count, scope });
        }

        fn bump(&self, lane: &str, delta: i64) {
            let mut map = self.0.borrow_mut();
            let entry = map.entry(lane.to_owned()).or_insert(LiveRunsSample {
                count: 0,
                scope: CounterScope::Lane,
            });
            entry.count = if delta >= 0 {
                entry.count.saturating_add(delta as u64)
            } else {
                entry.count.saturating_sub(delta.unsigned_abs())
            };
        }
    }

    impl LiveRunsReader for FakeLiveRuns {
        fn read_live_runs(
            &self,
            lane_name: &str,
            _counter: &LiveRunsCounter,
        ) -> Result<LiveRunsSample> {
            Ok(self
                .0
                .borrow()
                .get(lane_name)
                .copied()
                .unwrap_or(LiveRunsSample {
                    count: 0,
                    scope: CounterScope::Lane,
                }))
        }
    }

    #[derive(Clone, Default)]
    struct FakeUsage {
        responses: Rc<RefCell<BTreeMap<String, Result<CommandOutput>>>>,
        calls: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl FakeUsage {
        fn set_stdout(&self, program: &str, stdout: &str) {
            self.responses.borrow_mut().insert(
                program.to_owned(),
                Ok(CommandOutput {
                    success: true,
                    status_code: Some(0),
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                }),
            );
        }

        fn set_result(&self, program: &str, result: Result<CommandOutput>) {
            self.responses
                .borrow_mut()
                .insert(program.to_owned(), result);
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl UsageCommandRunner for FakeUsage {
        fn run(&self, command: &[String], _timeout: Duration) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(command.to_vec());
            let key = command.first().map(String::as_str).unwrap_or("");
            self.responses
                .borrow()
                .get(key)
                .cloned()
                .unwrap_or_else(|| {
                    Ok(CommandOutput {
                        success: true,
                        status_code: Some(0),
                        stdout: "five_hour_pct=80%\nweek_pct=75%\n".to_owned(),
                        stderr: String::new(),
                    })
                })
        }
    }

    fn sample_lanes() -> LanesFile {
        let value = json!({
            "schema": LANES_SCHEMA,
            "machine": {
                "max_load_1m": 16.0,
                "min_available_mem_mib": 8192
            },
            "lanes": {
                "lane-a": {
                    "pool": "gemini-pro",
                    "account": "acct-1",
                    "launch_template": ["harness-a"],
                    "usage_read": {
                        "command": ["usage-a"],
                        "parser": "key_value"
                    },
                    "floors": {
                        "five_hour_pct": 20.0,
                        "week_pct": 15.0
                    },
                    "run_cap": 2,
                    "live_runs_counter": {
                        "kind": "caps_file",
                        "path": "/fake/caps/lane-a.json"
                    },
                    "stagger_s": 60,
                    "heavy_queue_command": ["heavy"],
                    "context_handoff_tokens": 400000,
                    "extra_env": [],
                    "approved_containment": ["clone", "jail"]
                },
                "lane-b": {
                    "pool": "claude-pro",
                    "launch_template": ["harness-b"],
                    "usage_read": {
                        "command": ["usage-b"],
                        "parser": "key_value"
                    },
                    "floors": {
                        "week_pct": 10.0
                    },
                    "run_cap": 2,
                    "live_runs_counter": {
                        "kind": "caps_file",
                        "path": "/fake/caps/lane-b.json"
                    },
                    "stagger_s": 60,
                    "heavy_queue_command": ["heavy"],
                    "context_handoff_tokens": null,
                    "extra_env": [],
                    "approved_containment": ["clone"]
                },
                "lane-a2": {
                    "pool": "gemini-pro",
                    "account": "acct-1",
                    "launch_template": ["harness-a2"],
                    "usage_read": {
                        "command": ["usage-a2"],
                        "parser": "key_value"
                    },
                    "floors": {
                        "five_hour_pct": 20.0,
                        "week_pct": 15.0
                    },
                    "run_cap": 2,
                    "live_runs_counter": {
                        "kind": "caps_file",
                        "path": "/fake/caps/lane-a2.json"
                    },
                    "stagger_s": 60,
                    "heavy_queue_command": ["heavy"],
                    "context_handoff_tokens": 400000,
                    "extra_env": [],
                    "approved_containment": ["clone"]
                },
                "lane-c": {
                    "pool": "codex-pool",
                    "launch_template": ["harness-c"],
                    "usage_read": {
                        "command": ["usage-c"],
                        "parser": "codex"
                    },
                    "floors": {
                        "credits": 10.0
                    },
                    "run_cap": 3,
                    "live_runs_counter": {
                        "kind": "caps_file",
                        "path": "/fake/caps/lane-c.json"
                    },
                    "stagger_s": 0,
                    "heavy_queue_command": ["heavy"],
                    "context_handoff_tokens": null,
                    "extra_env": [],
                    "approved_containment": ["clone"]
                }
            }
        });
        parse_lanes(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn req(id: &str, role: TrialRole, lane: &str) -> RunRequest {
        RunRequest {
            id: id.to_owned(),
            role,
            lane: lane.to_owned(),
            wall_cap_s: 1800,
        }
    }

    fn test_config(dir: &TempDir) -> GateConfig {
        GateConfig::new(dir.path().join("state"), dir.path().join("notices"))
    }

    fn reserve_and_confirm(
        gate: &mut RunGate<FakeClock, FakeMachine, FakeLiveRuns, FakeUsage>,
        live: &FakeLiveRuns,
        request: RunRequest,
    ) -> GateGrant {
        let lane = request.lane.clone();
        let res = match gate.try_reserve(request).unwrap() {
            ReserveDecision::Reserved(r) => r,
            ReserveDecision::Hold(h) => panic!("expected Reserved, got Hold({h})"),
        };
        live.bump(&lane, 1);
        match gate.confirm_slot(res).unwrap() {
            GateDecision::Granted(g) => g,
            GateDecision::Hold { reason, .. } => panic!("expected Granted, got Hold({reason})"),
        }
    }

    // -----------------------------------------------------------------------
    // Pure parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn proc_loadavg_and_meminfo_parsers() {
        let load = parse_proc_loadavg("1.25 0.98 0.50 2/412 12345\n").unwrap();
        assert_eq!(
            load,
            Load {
                one: 1.25,
                five: 0.98,
                fifteen: 0.50
            }
        );
        assert_eq!(
            parse_proc_loadavg("0.00 -0.0 0.0").unwrap(),
            Load {
                one: 0.0,
                five: 0.0,
                fifteen: 0.0
            }
        );
        for bad in [
            "",
            "1.0 2.0",
            "-1.0 1.0 1.0",
            "NaN 1.0 1.0",
            "inf 1.0 1.0",
            "a b c",
        ] {
            assert!(parse_proc_loadavg(bad).is_err(), "{bad:?}");
        }

        let meminfo = "\
MemTotal:       65712344 kB
MemFree:         4123456 kB
MemAvailable:    8388608 kB
Buffers:          512000 kB
";
        assert_eq!(
            parse_proc_meminfo_available_kib(meminfo).unwrap(),
            8_388_608
        );
        assert_eq!(
            parse_proc_meminfo_available_kib("MemAvailable: 1024\n").unwrap(),
            1024
        );
        for bad in [
            "MemTotal: 1024 kB\n",
            "MemAvailable:\n",
            "MemAvailable: -5 kB\n",
            "MemAvailable: +5 kB\n",
            "MemAvailable: 1024 MB\n",
            "MemAvailable: 1024 kB extra\n",
            "MemAvailable: 1024 kB\nMemAvailable: 2048 kB\n",
        ] {
            assert!(parse_proc_meminfo_available_kib(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn caps_file_parser_covers_scopes_and_rejects_malformed() {
        assert_eq!(
            parse_caps_file("3\n", "lane-a").unwrap(),
            LiveRunsSample {
                count: 3,
                scope: CounterScope::SharedPath
            }
        );
        assert_eq!(
            parse_caps_file(r#"{"live_runs": 2}"#, "lane-a").unwrap(),
            LiveRunsSample {
                count: 2,
                scope: CounterScope::SharedPath
            }
        );
        assert_eq!(
            parse_caps_file(r#"{"running": 2, "live_runs": 2}"#, "lane-a").unwrap(),
            LiveRunsSample {
                count: 2,
                scope: CounterScope::SharedPath
            }
        );
        let per_lane =
            r#"{"lanes": {"lane-a": 1, "lane-b": {"live_runs": 2}, "lane-c": {"running": 3}}}"#;
        assert_eq!(
            parse_caps_file(per_lane, "lane-a").unwrap(),
            LiveRunsSample {
                count: 1,
                scope: CounterScope::Lane
            }
        );
        assert_eq!(
            parse_caps_file(per_lane, "lane-b").unwrap(),
            LiveRunsSample {
                count: 2,
                scope: CounterScope::Lane
            }
        );
        assert_eq!(
            parse_caps_file(per_lane, "lane-c").unwrap(),
            LiveRunsSample {
                count: 3,
                scope: CounterScope::Lane
            }
        );
        assert_eq!(
            parse_caps_file(per_lane, "lane-missing").unwrap(),
            LiveRunsSample {
                count: 0,
                scope: CounterScope::Lane
            }
        );

        for bad in [
            "",
            "   ",
            "-1",
            "+1",
            "1.5",
            "abc",
            r#"{}"#,
            r#"{"live_runs": 1, "running": 2}"#,
            r#"{"live_runs": 1, "live_runs": 1}"#,
            r#"{"lanes": {"lane-a": 1}, "live_runs": 1}"#,
            r#"{"lanes": []}"#,
            r#"{"lanes": {"lane-a": -1}}"#,
            r#"{"lanes": {"lane-a": 1.5}}"#,
            r#"{"lanes": {"lane-a": {"live_runs": 1, "running": 2}}}"#,
            r#"{"lanes": {"lane-a": {}}}"#,
        ] {
            assert!(parse_caps_file(bad, "lane-a").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn usage_parsers_cover_all_five_formats_and_refusals() {
        // 1. agy: multi-group TSV with explicit group, single-group TSV, colon lines, disabled.
        let agy_tsv = "\
gemini-pro\tfive-hour\t85%\t2026-12-01T15:00:00Z
gemini-pro\tweekly\t72.5%\t2026-12-07T00:00:00Z
claude-gpt\t5h\tdisabled\t2026-12-01T15:00:00Z
claude-gpt\tweek\t40%
";
        let m = parse_usage(UsageParser::Agy, Some("gemini-pro"), agy_tsv).unwrap();
        assert_eq!(m[&Text::new("five_hour_pct").unwrap()], 85.0);
        assert_eq!(m[&Text::new("week_pct").unwrap()], 72.5);
        let m2 = parse_usage(UsageParser::Agy, Some("CLAUDE-GPT"), agy_tsv).unwrap();
        assert_eq!(m2[&Text::new("five_hour_pct").unwrap()], 0.0);
        assert_eq!(m2[&Text::new("week_pct").unwrap()], 40.0);
        // Multi-group without group filter refused.
        assert!(parse_usage(UsageParser::Agy, None, agy_tsv).is_err());
        // Unknown group refused.
        assert!(parse_usage(UsageParser::Agy, Some("missing"), agy_tsv).is_err());
        // Single-group TSV without group succeeds.
        let single_tsv = "gemini\t5-hour\t90%\ngemini\tweekly\t55%\n";
        let m3 = parse_usage(UsageParser::Agy, None, single_tsv).unwrap();
        assert_eq!(m3[&Text::new("five_hour_pct").unwrap()], 90.0);
        assert_eq!(m3[&Text::new("week_pct").unwrap()], 55.0);
        // Colon format succeeds.
        let colon = "Five hour limit: 85% (resets in 2h)\nWeekly limit: 72.5%\n";
        let m4 = parse_usage(UsageParser::Agy, None, colon).unwrap();
        assert_eq!(m4[&Text::new("five_hour_pct").unwrap()], 85.0);
        assert_eq!(m4[&Text::new("week_pct").unwrap()], 72.5);
        // Refusals: duplicate window, >100%, <0%, n/a, empty.
        for bad in [
            "Five hour limit: 80%\n5h: 70%\n",
            "Five hour limit: 101%\n",
            "Five hour limit: -1%\n",
            "Five hour limit: n/a\n",
            "Error: unauthorized\n",
        ] {
            assert!(parse_usage(UsageParser::Agy, None, bad).is_err(), "{bad:?}");
        }

        // 2. codex: balance forms + optional windows + strict error refusal.
        for (input, expected) in [
            ("Balance: 42.50 credits\n", 42.5),
            ("Credit balance: $12.34\n", 12.34),
            ("Credits remaining: 85\n", 85.0),
            ("credits=99.5\n", 99.5),
            ("12 credits\n", 12.0),
        ] {
            let m = parse_usage(UsageParser::Codex, None, input).unwrap();
            assert_eq!(m[&Text::new("credits").unwrap()], expected, "{input:?}");
        }
        let codex_full = "Balance: 42.5 credits\nFive hour limit: 90%\nWeekly limit: 60%\n";
        let mf = parse_usage(UsageParser::Codex, None, codex_full).unwrap();
        assert_eq!(mf[&Text::new("credits").unwrap()], 42.5);
        assert_eq!(mf[&Text::new("five_hour_pct").unwrap()], 90.0);
        assert_eq!(mf[&Text::new("week_pct").unwrap()], 60.0);
        for bad in [
            "Error 429: balance unavailable\n",
            "Balance: unavailable\n",
            "Balance: -5 credits\n",
            "Balance: 10 credits\nCredits: 20\n",
            "Balance: 10 bananas\n",
        ] {
            assert!(
                parse_usage(UsageParser::Codex, None, bad).is_err(),
                "{bad:?}"
            );
        }

        // 3. key_value: multi-line and single-line, comments, % suffix, refusals.
        let kv = "# pool reading\nfive_hour_pct=82.5% week_pct=60\ncredits=14.25\n";
        let mkv = parse_usage(UsageParser::KeyValue, None, kv).unwrap();
        assert_eq!(mkv[&Text::new("five_hour_pct").unwrap()], 82.5);
        assert_eq!(mkv[&Text::new("week_pct").unwrap()], 60.0);
        assert_eq!(mkv[&Text::new("credits").unwrap()], 14.25);
        for bad in [
            "",
            "# only comment\n",
            "a=1 a=2",
            "stray_token a=1",
            "a=-1",
            "a=NaN",
            "a=105%",
            "bad$key=1",
        ] {
            assert!(
                parse_usage(UsageParser::KeyValue, None, bad).is_err(),
                "{bad:?}"
            );
        }

        // 4. claude: JSON utilization -> remaining and text form with None.
        let claude_json = r#"{"five_hour":{"utilization":23.5},"seven_day":{"utilization":12.0}}"#;
        let mc = parse_usage(UsageParser::Claude, None, claude_json).unwrap();
        assert_eq!(mc[&Text::new("five_hour_pct").unwrap()], 76.5);
        assert_eq!(mc[&Text::new("week_pct").unwrap()], 88.0);
        let mc_text = parse_usage(UsageParser::Claude, None, "25.0 None extra").unwrap();
        assert_eq!(mc_text[&Text::new("five_hour_pct").unwrap()], 75.0);
        assert!(!mc_text.contains_key(&Text::new("week_pct").unwrap()));
        for bad in [
            r#"{"five_hour":{"utilization":120.0}}"#,
            r#"{"five_hour":{"utilization":-1.0}}"#,
            "None None",
            "Error: unauthorized",
        ] {
            assert!(
                parse_usage(UsageParser::Claude, None, bad).is_err(),
                "{bad:?}"
            );
        }

        // 5. json: valid object with optional observed_at, duplicate/empty/negative refused.
        let mj = parse_usage(
            UsageParser::Json,
            None,
            r#"{"observed_at": 1700000000, "week_pct": 55.5, "credits": 0}"#,
        )
        .unwrap();
        assert_eq!(mj.len(), 2);
        assert_eq!(mj[&Text::new("week_pct").unwrap()], 55.5);
        assert_eq!(mj[&Text::new("credits").unwrap()], 0.0);
        for bad in [
            "{}",
            r#"{"observed_at": 1700000000}"#,
            r#"{"week_pct": 1, "week_pct": 2}"#,
            r#"{"week_pct": -1}"#,
            r#"{"observed_at": "bad", "week_pct": 10}"#,
        ] {
            assert!(
                parse_usage(UsageParser::Json, None, bad).is_err(),
                "{bad:?}"
            );
        }

        // Group filter rejected on non-agy parsers.
        assert!(parse_usage(UsageParser::KeyValue, Some("g"), "a=1").is_err());
    }

    // -----------------------------------------------------------------------
    // Real filesystem counter & deadline-bound command runner tests
    // -----------------------------------------------------------------------

    #[test]
    fn fs_live_runs_reader_lock_file_and_caps_file() {
        let dir = TempDir::new().unwrap();
        let reader = FsLiveRunsReader;

        // Missing lock file in existing parent -> 0 without creating it.
        let missing_lock = dir.path().join("absent.lock");
        let counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(missing_lock.to_string_lossy().as_ref()).unwrap(),
        };
        assert_eq!(
            reader.read_live_runs("lane-a", &counter).unwrap(),
            LiveRunsSample {
                count: 0,
                scope: CounterScope::SharedPath
            }
        );
        assert!(!missing_lock.exists());

        // Missing parent directory -> Err.
        let bad_parent = dir.path().join("no-such-dir/a.lock");
        let bad_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(bad_parent.to_string_lossy().as_ref()).unwrap(),
        };
        assert!(reader.read_live_runs("lane-a", &bad_counter).is_err());

        // Regular lock file: unlocked -> 0, exclusively locked -> 1.
        let lock_path = dir.path().join("slot.lock");
        let holder = open_lock_file(&lock_path).unwrap();
        let lock_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(lock_path.to_string_lossy().as_ref()).unwrap(),
        };
        assert_eq!(
            reader
                .read_live_runs("lane-a", &lock_counter)
                .unwrap()
                .count,
            0
        );
        holder.lock().unwrap();
        assert_eq!(
            reader
                .read_live_runs("lane-a", &lock_counter)
                .unwrap()
                .count,
            1
        );
        drop(holder);
        assert_eq!(
            reader
                .read_live_runs("lane-a", &lock_counter)
                .unwrap()
                .count,
            0
        );

        // Lock directory with 2 locked + 1 unlocked + 1 hidden file -> 2.
        let lock_dir = dir.path().join("locks");
        fs::create_dir(&lock_dir).unwrap();
        let f1 = open_lock_file(&lock_dir.join("s1.lock")).unwrap();
        let f2 = open_lock_file(&lock_dir.join("s2.lock")).unwrap();
        let _f3 = open_lock_file(&lock_dir.join("s3.lock")).unwrap();
        let hidden = open_lock_file(&lock_dir.join(".coord.lock")).unwrap();
        f1.lock().unwrap();
        f2.lock().unwrap();
        hidden.lock().unwrap();
        let dir_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(lock_dir.to_string_lossy().as_ref()).unwrap(),
        };
        assert_eq!(
            reader.read_live_runs("lane-a", &dir_counter).unwrap().count,
            2
        );
        drop((f1, f2, hidden));

        // CapsFile: missing in existing dir -> 0, missing parent -> Err, real JSON & int files.
        let caps_path = dir.path().join("caps.json");
        let caps_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::CapsFile,
            path: Text::new(caps_path.to_string_lossy().as_ref()).unwrap(),
        };
        assert_eq!(
            reader
                .read_live_runs("lane-a", &caps_counter)
                .unwrap()
                .count,
            0
        );
        assert!(!caps_path.exists());
        fs::write(&caps_path, r#"{"lanes":{"lane-a":2,"lane-b":1}}"#).unwrap();
        assert_eq!(
            reader.read_live_runs("lane-a", &caps_counter).unwrap(),
            LiveRunsSample {
                count: 2,
                scope: CounterScope::Lane
            }
        );
        fs::write(&caps_path, "not-valid").unwrap();
        assert!(reader.read_live_runs("lane-a", &caps_counter).is_err());

        // Real flock contention through acquire_slot + FsLiveRunsReader.
        let lane_lock = dir.path().join("lane-a.lock");
        let lane_lock_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(lane_lock.to_string_lossy().as_ref()).unwrap(),
        };
        let mut lanes = sample_lanes();
        {
            let lane_a = lanes.lanes.get_mut(&Text::new("lane-a").unwrap()).unwrap();
            lane_a.run_cap = 1;
            lane_a.live_runs_counter = lane_lock_counter.clone();
            lane_a.usage_read.in_slot_command = Some(vec![Text::new("usage-a-inner").unwrap()]);
        }
        {
            let lane_a2 = lanes.lanes.get_mut(&Text::new("lane-a2").unwrap()).unwrap();
            lane_a2.live_runs_counter = lane_lock_counter;
            lane_a2.usage_read.in_slot_command = Some(vec![Text::new("usage-a2-inner").unwrap()]);
        }

        let clock = FakeClock::new(500);
        let mut gate = RunGate::with_deps(
            lanes,
            test_config(&dir),
            clock.clone(),
            FakeMachine::healthy(),
            FsLiveRunsReader,
            FakeUsage::default(),
        )
        .unwrap();

        // Competing fleet process grabs the real flock after try_reserve checked it.
        let mut fleet_holder: Option<File> = None;
        let contended = gate
            .acquire_slot(req("flock-1", TrialRole::Builder, "lane-a"), |_| {
                let f = open_lock_file(&lane_lock).unwrap();
                f.try_lock().unwrap();
                fleet_holder = Some(f);
                let attempt = open_lock_file(&lane_lock).unwrap();
                match attempt.try_lock() {
                    Ok(()) => Ok(SlotAcquisition::Acquired(attempt)),
                    Err(TryLockError::WouldBlock) => Ok(SlotAcquisition::Busy),
                    Err(TryLockError::Error(e)) => Err(Error(e.to_string())),
                }
            })
            .unwrap();
        assert!(matches!(
            contended,
            AcquireOutcome::Hold {
                reason: HoldReason::LaneRunCap {
                    live: 1,
                    cap: 1,
                    ..
                },
                ..
            }
        ));
        assert_eq!(gate.active_run_count(), 0);

        // Once the competing fleet lock drops, acquire_slot succeeds and wall cap starts at t=700.
        drop(fleet_holder);
        clock.set(700);
        let (grant, slot_file) = match gate
            .acquire_slot(req("flock-1", TrialRole::Builder, "lane-a"), |_| {
                let f = open_lock_file(&lane_lock).unwrap();
                f.try_lock().unwrap();
                Ok(SlotAcquisition::Acquired(f))
            })
            .unwrap()
        {
            AcquireOutcome::Granted { grant, slot } => (grant, slot),
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant.wall_cap.started_at_s, 700);
        assert_eq!(grant.before_snapshot.other_live_runs, 0);
        drop(slot_file);
        let out = gate.finish(grant, RunCompletion::Completed).unwrap();
        assert_eq!(out.pool_reading.after.as_ref().unwrap().other_live_runs, 0);
    }

    #[test]
    fn system_usage_runner_handles_success_timeout_overflow_and_inherited_pipes() {
        let runner = SystemUsageCommandRunner;

        // 1. Missing binary fails cleanly.
        assert!(
            runner
                .run(
                    &["/nonexistent/agent-bench-missing-usage-cmd".to_owned()],
                    Duration::from_secs(1)
                )
                .is_err()
        );

        // 2. Normal command captures stdout.
        let ok = runner
            .run(
                &[
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf 'five_hour_pct=85\\n'".to_owned(),
                ],
                Duration::from_secs(5),
            )
            .unwrap();
        assert!(ok.success);
        assert_eq!(ok.stdout, "five_hour_pct=85\n");

        // 3. Inherited pipe regression: direct child exits immediately while a background
        //    descendant keeps stdout/stderr open. Bounded process-group cleanup must kill the
        //    descendant and return within the deadline WITHOUT any manual test cleanup.
        let dir = TempDir::new().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let start = Instant::now();
        let err = runner
            .run(
                &[
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "sleep 30 & pid=$!; printf '%s\\n' \"$pid\" > \"$1\"; printf 'five_hour_pct=85\\n'; exit 0".to_owned(),
                    "_".to_owned(),
                    pid_file.to_string_lossy().into_owned(),
                ],
                Duration::from_secs(5),
            )
            .unwrap_err()
            .0;
        let elapsed = start.elapsed();
        assert!(
            err.contains("output pipes did not close"),
            "unexpected error: {err}"
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "pipe draining blocked past deadline: {elapsed:?}"
        );
        let descendant_pid: i32 = fs::read_to_string(&pid_file)
            .expect("descendant pid file must be written before timeout")
            .trim()
            .parse()
            .unwrap();
        let descendant = rustix::process::Pid::from_raw(descendant_pid).unwrap();
        let mut gone = false;
        for _ in 0..200 {
            if rustix::process::test_kill_process(descendant) == Err(rustix::io::Errno::SRCH) {
                gone = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            gone,
            "descendant {descendant_pid} survived process-group timeout cleanup"
        );

        // 4. Live wrapper spawning a subprocess: timeout kills both wrapper and descendant.
        let wrapper_pid_file = dir.path().join("wrapper-child.pid");
        let start = Instant::now();
        let err = runner
            .run(
                &[
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "sleep 30 & pid=$!; printf '%s\\n' \"$pid\" > \"$1\"; wait".to_owned(),
                    "_".to_owned(),
                    wrapper_pid_file.to_string_lossy().into_owned(),
                ],
                Duration::from_secs(5),
            )
            .unwrap_err()
            .0;
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(20));
        let wrapper_child_pid: i32 = fs::read_to_string(&wrapper_pid_file)
            .expect("wrapper child pid file must be written before timeout")
            .trim()
            .parse()
            .unwrap();
        let wrapper_child = rustix::process::Pid::from_raw(wrapper_child_pid).unwrap();
        let mut wrapper_child_gone = false;
        for _ in 0..200 {
            if rustix::process::test_kill_process(wrapper_child) == Err(rustix::io::Errno::SRCH) {
                wrapper_child_gone = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            wrapper_child_gone,
            "wrapper descendant {wrapper_child_pid} survived timeout cleanup"
        );

        // 5. Output exceeding MAX_USAGE_OUTPUT_BYTES is refused without buffering unbounded memory.
        let err = runner
            .run(
                &[
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "dd if=/dev/zero bs=1024 count=300 2>/dev/null | tr '\\0' 'a'".to_owned(),
                ],
                Duration::from_secs(5),
            )
            .unwrap_err()
            .0;
        assert!(err.contains("exceeded"), "{err}");
    }

    // -----------------------------------------------------------------------
    // Gate rules & state-machine tests
    // -----------------------------------------------------------------------

    #[test]
    fn refuses_lanes_without_machine_limits() {
        let dir = TempDir::new().unwrap();
        let mut lanes = sample_lanes();
        lanes.machine = None;
        let err = RunGate::with_deps(
            lanes,
            test_config(&dir),
            FakeClock::new(0),
            FakeMachine::healthy(),
            FakeLiveRuns::default(),
            FakeUsage::default(),
        )
        .err()
        .expect("missing machine limits must fail")
        .0;
        assert!(err.contains("machine"), "{err}");
    }

    #[test]
    fn refuses_lock_file_lane_without_in_slot_command_and_accepts_canonical_fixture() {
        let dir = TempDir::new().unwrap();
        let fixture_lanes = parse_lanes(include_str!("fixtures/lanes.json").as_bytes()).unwrap();
        // Canonical fixture configures `usage_read.in_slot_command` on its `lock_file` lane and
        // constructs a RunGate cleanly.
        assert!(
            RunGate::with_deps(
                fixture_lanes.clone(),
                GateConfig::new(dir.path().join("state-ok"), dir.path().join("notices-ok")),
                FakeClock::new(0),
                FakeMachine::healthy(),
                FakeLiveRuns::default(),
                FakeUsage::default(),
            )
            .is_ok()
        );

        // Dropping `usage_read.in_slot_command` from a `lock_file` lane is rejected at construction.
        let mut missing_in_slot = fixture_lanes;
        missing_in_slot
            .lanes
            .get_mut(&Text::new("lane-a").unwrap())
            .unwrap()
            .usage_read
            .in_slot_command = None;
        let err = RunGate::with_deps(
            missing_in_slot,
            GateConfig::new(dir.path().join("state-bad"), dir.path().join("notices-bad")),
            FakeClock::new(0),
            FakeMachine::healthy(),
            FakeLiveRuns::default(),
            FakeUsage::default(),
        )
        .err()
        .expect("lock_file lane without in_slot_command must fail at construction")
        .0;
        assert!(
            err.contains("lock_file") && err.contains("usage_read.in_slot_command"),
            "{err}"
        );
    }

    #[test]
    fn machine_gate_holds_before_usage_command_and_checks_boundaries() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(100);
        let machine = FakeMachine::ok(16.01, 8192 * 1024);
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock,
            machine.clone(),
            live,
            usage.clone(),
        )
        .unwrap();

        // Over load limit holds and never calls usage command.
        assert!(matches!(
            gate.try_reserve(req("r1", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::MachineLoad { .. })
        ));
        assert_eq!(usage.call_count(), 0);

        // Exact memory boundary minus 1 KiB holds (8192 MiB - 1 KiB).
        machine.set(Ok(MachineSample {
            load: Load {
                one: 16.0,
                five: 16.0,
                fifteen: 16.0,
            },
            available_kib: 8192 * 1024 - 1,
        }));
        assert_eq!(
            gate.try_reserve(req("r1", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::MachineMemory {
                available_mib: 8191,
                min_available_mib: 8192,
            })
        );
        assert_eq!(usage.call_count(), 0);

        // Exact load == max_load_1m and exact memory == min_available_mem_mib * 1024 passes.
        machine.set(Ok(MachineSample {
            load: Load {
                one: 16.0,
                five: 10.0,
                fifteen: 5.0,
            },
            available_kib: 8192 * 1024,
        }));
        let res = match gate
            .try_reserve(req("r1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        assert_eq!(usage.call_count(), 1);
        gate.cancel(res).unwrap();
    }

    #[test]
    fn bench_builder_cap_enforced_across_independent_gate_callers_while_reviewers_pass() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        usage.set_stdout("usage-c", "Balance: 50 credits\n");

        // Two independent RunGate instances sharing the same state_dir (simulating two trial
        // processes on one host).
        let mut gate_1 = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine.clone(),
            live.clone(),
            usage.clone(),
        )
        .unwrap();
        let mut gate_2 = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage,
        )
        .unwrap();

        // Gate 1 takes builder slot 1 on lane-a; Gate 2 takes builder slot 2 on lane-b.
        let b1 = reserve_and_confirm(&mut gate_1, &live, req("b1", TrialRole::Builder, "lane-a"));
        let b2 = reserve_and_confirm(&mut gate_2, &live, req("b2", TrialRole::Builder, "lane-b"));

        // Neither Gate 1 nor Gate 2 can admit a 3rd builder on lane-c (combined bench cap = 2).
        assert_eq!(
            gate_1
                .try_reserve(req("b3", TrialRole::Builder, "lane-c"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::BenchBuilderCap { active: 2, cap: 2 })
        );
        assert_eq!(
            gate_2
                .try_reserve(req("b4", TrialRole::Builder, "lane-c"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::BenchBuilderCap { active: 2, cap: 2 })
        );

        // Reviewers ignore the bench-wide builder cap and run up to lane-c's cap (3).
        let rev1 = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("rev1", TrialRole::Reviewer, "lane-c"),
        );
        let rev2 = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("rev2", TrialRole::Reviewer, "lane-c"),
        );
        let rev3 = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("rev3", TrialRole::Reviewer, "lane-c"),
        );
        assert!(matches!(
            gate_2
                .try_reserve(req("rev4", TrialRole::Reviewer, "lane-c"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::LaneRunCap {
                live: 3,
                cap: 3,
                ..
            })
        ));

        // Finishing b2 in Gate 2 unblocks a new builder in Gate 1 on lane-b (once stagger clears),
        // while lane-c remains full with its 3 reviewers.
        live.bump("lane-b", -1);
        gate_2.finish(b2, RunCompletion::Completed).unwrap();
        clock.advance(60);
        let b3 = reserve_and_confirm(&mut gate_1, &live, req("b3", TrialRole::Builder, "lane-b"));

        // A third independent gate with a stricter cap (cap=1) sees active=2 across the shared
        // state_dir even when slot indices are sparse.
        let mut strict_cfg = test_config(&dir);
        strict_cfg.max_concurrent_builders = 1;
        let mut gate_3 = RunGate::with_deps(
            sample_lanes(),
            strict_cfg,
            clock.clone(),
            FakeMachine::healthy(),
            live.clone(),
            FakeUsage::default(),
        )
        .unwrap();
        assert_eq!(
            gate_3
                .try_reserve(req("strict-b", TrialRole::Builder, "lane-a2"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::BenchBuilderCap { active: 2, cap: 1 })
        );

        // Clean up gate_2 reviewers, then drop gate_1 (simulating process exit) and confirm its
        // builder slots are automatically released by OS FD teardown.
        for (lane, g) in [("lane-c", rev1), ("lane-c", rev2), ("lane-c", rev3)] {
            live.bump(lane, -1);
            gate_2.finish(g, RunCompletion::Completed).unwrap();
        }
        drop((b1, b3, gate_1));
        live.set("lane-a", 0, CounterScope::Lane);
        live.set("lane-b", 0, CounterScope::Lane);
        let after_drop = reserve_and_confirm(
            &mut gate_3,
            &live,
            req("after-drop", TrialRole::Builder, "lane-a"),
        );
        live.bump("lane-a", -1);
        gate_3.finish(after_drop, RunCompletion::Completed).unwrap();
    }

    #[test]
    fn contention_between_check_and_acquisition_delays_wall_cap_and_handles_busy_or_oversubscribe()
    {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(100);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        let mut lanes = sample_lanes();
        lanes
            .lanes
            .get_mut(&Text::new("lane-a").unwrap())
            .unwrap()
            .run_cap = 1;

        let mut gate = RunGate::with_deps(
            lanes,
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        // Case 1: Competing fleet run grabs the lane slot between check and non-blocking
        // acquisition -> `acquire_slot` cancels reservation, returns Hold, never starts wall cap.
        let outcome: AcquireOutcome<()> = gate
            .acquire_slot(req("r1", TrialRole::Builder, "lane-a"), |_res| {
                live.set("lane-a", 1, CounterScope::Lane);
                Ok(SlotAcquisition::Busy)
            })
            .unwrap();
        assert_eq!(
            outcome,
            AcquireOutcome::Hold {
                request: req("r1", TrialRole::Builder, "lane-a"),
                reason: HoldReason::LaneRunCap {
                    lane: "lane-a".to_owned(),
                    live: 1,
                    cap: 1,
                },
            }
        );
        assert_eq!(gate.active_run_count(), 0);

        // Case 2: Competing fleet run grabs the lane lock AND blocks the lane owner from t=100
        // until t=400. Wall cap must start at t=400 (slot confirmation), not t=100, and
        // before_snapshot must reflect the refreshed pool reading at t=400.
        live.set("lane-a", 0, CounterScope::Lane);
        let res = match gate
            .try_reserve(req("r1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        assert_eq!(res.checked_at_s, 100);
        // Simulate waiting 300s behind a competing fleet run that also consumed 10% quota.
        clock.set(400);
        usage.set_stdout("usage-a", "five_hour_pct=70%\nweek_pct=65%\n");
        live.set("lane-a", 1, CounterScope::Lane); // lane owner acquired our slot at t=400
        let grant = match gate.confirm_slot(res).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant.granted_at_s, 400);
        assert_eq!(
            grant.wall_cap,
            WallCap {
                started_at_s: 400,
                duration_s: 1800,
                deadline_unix_s: 2200,
            }
        );
        assert_eq!(grant.wall_cap.remaining_s(400), 1800);
        assert!(!grant.wall_cap.is_expired(2199));
        assert!(grant.wall_cap.is_expired(2200));
        assert_eq!(grant.before_snapshot.at.as_str(), "1970-01-01T00:06:40Z");
        assert_eq!(
            grant.before_snapshot.meters[&Text::new("five_hour_pct").unwrap()],
            70.0
        );
        assert_eq!(grant.before_snapshot.other_live_runs, 0);

        live.set("lane-a", 0, CounterScope::Lane);
        gate.finish(grant, RunCompletion::Completed).unwrap();

        // Case 3: Contention oversubscribes the lane between try_reserve and confirm_slot
        // (external live becomes 2 on a cap=1 lane) -> confirm_slot rolls back and holds.
        clock.advance(60);
        let res2 = match gate
            .try_reserve(req("r2", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        live.set("lane-a", 2, CounterScope::Lane);
        assert_eq!(
            gate.confirm_slot(res2).unwrap(),
            GateDecision::Hold {
                request: req("r2", TrialRole::Builder, "lane-a"),
                reason: HoldReason::LaneRunCap {
                    lane: "lane-a".to_owned(),
                    live: 1,
                    cap: 1,
                },
            }
        );
        assert_eq!(gate.active_run_count(), 0);
    }

    #[test]
    fn pool_floors_stagger_and_account_sharing() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(0);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        // Under floor holds; equal to floor passes; missing meter returns Err; failing cmd returns Err.
        usage.set_stdout("usage-a", "five_hour_pct=19.9%\nweek_pct=50%\n");
        assert_eq!(
            gate.try_reserve(req("r1", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::PoolBelowFloor {
                pool: "gemini-pro".to_owned(),
                meter: "five_hour_pct".to_owned(),
                value: 19.9,
                floor: 20.0,
            })
        );
        usage.set_stdout("usage-a", "week_pct=50%\n");
        assert!(
            gate.try_reserve(req("r1", TrialRole::Builder, "lane-a"))
                .unwrap_err()
                .0
                .contains("missing from reading")
        );
        usage.set_result(
            "usage-a",
            Ok(CommandOutput {
                success: false,
                status_code: Some(1),
                stdout: String::new(),
                stderr: "auth failed".to_owned(),
            }),
        );
        assert!(
            gate.try_reserve(req("r1", TrialRole::Builder, "lane-a"))
                .is_err()
        );

        // Floor exact match passes at t = 0 (proves absence-based stagger, no 0 sentinel bug).
        usage.set_stdout("usage-a", "five_hour_pct=20.0%\nweek_pct=15.0%\n");
        let g1 = reserve_and_confirm(&mut gate, &live, req("r1", TrialRole::Builder, "lane-a"));

        // At t = 59, lane-a2 sharing account `acct-1` holds with wait_s = 1; held check does not
        // reset stagger.
        clock.set(59);
        assert_eq!(
            gate.try_reserve(req("r2", TrialRole::Builder, "lane-a2"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::Stagger {
                account: "acct-1".to_owned(),
                wait_s: 1,
                ready_at_s: 60,
            })
        );
        clock.set(60);
        let g2 = reserve_and_confirm(&mut gate, &live, req("r2", TrialRole::Builder, "lane-a2"));

        live.bump("lane-a", -1);
        gate.finish(g1, RunCompletion::Completed).unwrap();
        live.bump("lane-a2", -1);
        gate.finish(g2, RunCompletion::Completed).unwrap();
    }

    #[test]
    fn rate_limit_cooldown_cancel_restore_retry_and_notice_lifecycle() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage,
        )
        .unwrap();

        // Start two runs on lane-a (staggered by 60s) to also test pre-cooldown sibling behavior.
        let g1 = reserve_and_confirm(&mut gate, &live, req("r1", TrialRole::Builder, "lane-a"));
        clock.set(1_060);
        let g2 = reserve_and_confirm(&mut gate, &live, req("r2", TrialRole::Builder, "lane-a"));

        // r1 reports rate limit at t = 1_100 -> 600s cooldown until t = 1_700.
        clock.set(1_100);
        live.bump("lane-a", -1);
        let out1 = gate
            .finish(
                g1,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 first".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(
            out1.rate_limit,
            Some(RateLimitAction::Cooldown {
                lane: "lane-a".to_owned(),
                until_unix_s: 1_700,
                retries_remaining: 1,
            })
        );

        // Pre-cooldown sibling r2 also reports rate limit at t = 1_110 -> extends cooldown to
        // 1_710 without falsely escalating to HeldForLead.
        clock.set(1_110);
        live.bump("lane-a", -1);
        let out2 = gate
            .finish(
                g2,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 sibling".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(
            out2.rate_limit,
            Some(RateLimitAction::Cooldown {
                lane: "lane-a".to_owned(),
                until_unix_s: 1_710,
                retries_remaining: 1,
            })
        );

        // During cooldown (t = 1_709), lane holds and writes no notice yet.
        clock.set(1_709);
        let hold = gate
            .try_reserve(req("r3", TrialRole::Builder, "lane-a"))
            .unwrap();
        assert_eq!(
            hold,
            ReserveDecision::Hold(HoldReason::RateLimitCooldown {
                lane: "lane-a".to_owned(),
                wait_s: 1,
                ready_at_s: 1_710,
            })
        );
        assert!(!dir.path().join("notices").exists());

        // At t = 1_710, single retry is reserved. While pending, another request holds with
        // RateLimitRetryInFlight.
        clock.set(1_710);
        let retry_res = match gate
            .try_reserve(req("retry-cancelled", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved retry, got {other:?}"),
        };
        assert_eq!(
            gate.try_reserve(req("other", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitRetryInFlight {
                lane: "lane-a".to_owned(),
            })
        );

        // Cancelling the unlaunched retry must restore the cooldown/retry allowance instead of
        // stranding the lane in RateLimitRetryInFlight.
        gate.cancel(retry_res).unwrap();

        // Contention during confirm_slot on a retry must also roll back to CoolingDown.
        let retry_contended = match gate
            .try_reserve(req("retry-contended", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved retry after cancel, got {other:?}"),
        };
        live.set("lane-a", 3, CounterScope::Lane); // exceeds run_cap=2 -> Hold & rollback
        assert!(matches!(
            gate.confirm_slot(retry_contended).unwrap(),
            GateDecision::Hold {
                reason: HoldReason::LaneRunCap { .. },
                ..
            }
        ));
        live.set("lane-a", 0, CounterScope::Lane);

        // Next request at t = 1_710 is admitted as the post-cooldown retry!
        let retry_grant = reserve_and_confirm(
            &mut gate,
            &live,
            req("retry-actual", TrialRole::Builder, "lane-a"),
        );
        // Concurrent request while retry is launched still holds.
        clock.set(1_770);
        assert_eq!(
            gate.try_reserve(req("r4", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitRetryInFlight {
                lane: "lane-a".to_owned(),
            })
        );

        // Retry hits rate limit again at t = 1_775 -> latched HeldForLead + atomic notice written.
        clock.set(1_775);
        live.bump("lane-a", -1);
        let retry_out = gate
            .finish(
                retry_grant,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 Too Many Requests".to_owned(),
                },
            )
            .unwrap();
        let notice_path = match retry_out.rate_limit {
            Some(RateLimitAction::HeldForLead { lane, notice_path }) => {
                assert_eq!(lane, "lane-a");
                notice_path
            }
            other => panic!("expected HeldForLead, got {other:?}"),
        };
        let notice: RateLimitNotice =
            serde_json::from_slice(&fs::read(&notice_path).unwrap()).unwrap();
        assert_eq!(notice.schema, NOTICE_SCHEMA);
        assert_eq!(notice.kind, "rate_limit_hold");
        assert_eq!(notice.lane, "lane-a");
        assert_eq!(notice.pool, "gemini-pro");
        assert_eq!(notice.first_hit_at, "1970-01-01T00:18:20Z");
        assert_eq!(notice.second_hit_at, "1970-01-01T00:29:35Z");
        assert_eq!(notice.hold_seconds, 600);
        assert_eq!(notice.retries_attempted, 1);
        assert_eq!(notice.detail, "HTTP 429 Too Many Requests");

        // Still held far in the future until lead clears hold.
        clock.set(10_000);
        assert_eq!(
            gate.try_reserve(req("r5", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitHeldForLead {
                lane: "lane-a".to_owned(),
                notice_path,
            })
        );
        assert!(gate.clear_rate_limit_hold("lane-a").unwrap());
        assert!(!gate.clear_rate_limit_hold("lane-a").unwrap());

        // Successful retry clears cooldown without writing a notice.
        let g3 = reserve_and_confirm(&mut gate, &live, req("r6", TrialRole::Builder, "lane-a"));
        live.bump("lane-a", -1);
        gate.finish(
            g3,
            RunCompletion::RateLimited {
                detail: "HTTP 429".to_owned(),
            },
        )
        .unwrap();
        clock.advance(600);
        let g4 = reserve_and_confirm(
            &mut gate,
            &live,
            req("r7-retry", TrialRole::Builder, "lane-a"),
        );
        live.bump("lane-a", -1);
        let cleared = gate.finish(g4, RunCompletion::Completed).unwrap();
        assert_eq!(
            cleared.rate_limit,
            Some(RateLimitAction::Cleared {
                lane: "lane-a".to_owned(),
            })
        );
    }

    #[test]
    fn hold_never_spill_queue_and_wall_cap_start() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(100);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        usage.set_stdout("usage-a", "five_hour_pct=5%\nweek_pct=50%\n");
        usage.set_stdout("usage-b", "week_pct=50%\n");

        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        // Queue has [run_a (lane-a under floor), run_b (lane-b healthy)].
        // poll_queue dispatches run_b and keeps run_a on lane-a without spilling.
        let mut queue = VecDeque::from([
            req("run-a", TrialRole::Builder, "lane-a"),
            req("run-b", TrialRole::Builder, "lane-b"),
        ]);
        let res_b = match gate.poll_queue(&mut queue).unwrap() {
            PollOutcome::Reserved {
                reservation,
                queue_index,
                held,
            } => {
                assert_eq!(queue_index, 1);
                assert_eq!(held.len(), 1);
                assert_eq!(held[0].0, "run-a");
                assert!(matches!(held[0].1, HoldReason::PoolBelowFloor { .. }));
                reservation
            }
            other => panic!("expected Reserved run-b, got {other:?}"),
        };
        assert_eq!(res_b.request.id, "run-b");
        assert_eq!(res_b.request.lane, "lane-b");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, "run-a");
        assert_eq!(queue[0].lane, "lane-a");
        let cancelled = gate.cancel(res_b).unwrap();
        assert_eq!(cancelled.id, "run-b");
        assert_eq!(cancelled.lane, "lane-b");

        // With only run-a left while lane-b is idle, poll_queue holds and never spills.
        assert!(matches!(
            gate.poll_queue(&mut queue).unwrap(),
            PollOutcome::AllHeld { .. }
        ));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].lane, "lane-a");

        // Lane-a recovers at t = 2_500; wall cap starts at slot confirmation (2_500), not t = 100.
        clock.set(2_500);
        usage.set_stdout("usage-a", "five_hour_pct=60%\nweek_pct=50%\n");
        let res_a = match gate.poll_queue(&mut queue).unwrap() {
            PollOutcome::Reserved {
                reservation,
                queue_index,
                held,
            } => {
                assert_eq!(queue_index, 0);
                assert!(held.is_empty());
                reservation
            }
            other => panic!("expected Reserved run-a, got {other:?}"),
        };
        assert!(queue.is_empty());
        live.bump("lane-a", 1);
        let grant_a = match gate.confirm_slot(res_a).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(
            grant_a.wall_cap,
            WallCap {
                started_at_s: 2_500,
                duration_s: 1_800,
                deadline_unix_s: 4_300,
            }
        );
        live.bump("lane-a", -1);
        gate.finish(grant_a, RunCompletion::Completed).unwrap();
    }

    #[test]
    fn counter_scope_deduplicates_shared_aggregate_caps_and_preserves_per_lane_json_counts() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let usage = FakeUsage::default();

        // Both lane-a and lane-a2 share pool `gemini-pro` and point to the exact same caps_file.
        let shared_caps = dir.path().join("shared-caps.json");
        let mut lanes = sample_lanes();
        for name in ["lane-a", "lane-a2"] {
            let lane = lanes.lanes.get_mut(&Text::new(name).unwrap()).unwrap();
            lane.run_cap = 5;
            lane.live_runs_counter = LiveRunsCounter {
                kind: LiveRunsCounterKind::CapsFile,
                path: Text::new(shared_caps.to_string_lossy().as_ref()).unwrap(),
            };
        }

        let mut gate = RunGate::with_deps(
            lanes,
            test_config(&dir),
            clock.clone(),
            machine,
            FsLiveRunsReader,
            usage,
        )
        .unwrap();

        // Case 1: Shared aggregate caps file `{"live_runs": 2}` -> counted ONCE (2, not 4!).
        fs::write(&shared_caps, r#"{"live_runs": 2}"#).unwrap();
        let res = match gate
            .try_reserve(req("agg-1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        // External slot acquired: aggregate count bumps 2 -> 3 (2 others + self).
        fs::write(&shared_caps, r#"{"live_runs": 3}"#).unwrap();
        let grant = match gate.confirm_slot(res).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant.before_snapshot.other_live_runs, 2);

        // External slot released: aggregate count returns to 2.
        fs::write(&shared_caps, r#"{"live_runs": 2}"#).unwrap();
        clock.advance(10);
        let out = gate.finish(grant, RunCompletion::Completed).unwrap();
        assert_eq!(out.pool_reading.after.as_ref().unwrap().other_live_runs, 2);

        // Case 2: Same file path switched to lane-specific JSON `{"lanes": {"lane-a": 1, "lane-a2": 2}}`
        // -> both lanes' counts are preserved and summed (1 + 2 = 3)!
        clock.advance(60);
        fs::write(&shared_caps, r#"{"lanes": {"lane-a": 1, "lane-a2": 2}}"#).unwrap();
        let res2 = match gate
            .try_reserve(req("per-lane-1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        // External slot acquired on lane-a: lane-a becomes 2 (1 other + self), lane-a2 stays 2.
        fs::write(&shared_caps, r#"{"lanes": {"lane-a": 2, "lane-a2": 2}}"#).unwrap();
        let grant2 = match gate.confirm_slot(res2).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant2.before_snapshot.other_live_runs, 3);

        // External slot released: lane-a back to 1, lane-a2 stays 2 -> after other_live_runs == 3.
        fs::write(&shared_caps, r#"{"lanes": {"lane-a": 1, "lane-a2": 2}}"#).unwrap();
        clock.advance(10);
        let out2 = gate.finish(grant2, RunCompletion::Completed).unwrap();
        assert_eq!(out2.pool_reading.after.as_ref().unwrap().other_live_runs, 3);
    }

    #[test]
    fn foreign_and_already_consumed_leases_are_rejected_without_mutating_state() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(100);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();

        let mut gate_a = RunGate::with_deps(
            sample_lanes(),
            GateConfig::new(dir.path().join("state-a"), dir.path().join("notices-a")),
            clock.clone(),
            machine.clone(),
            live.clone(),
            usage.clone(),
        )
        .unwrap();
        let mut gate_b = RunGate::with_deps(
            sample_lanes(),
            GateConfig::new(dir.path().join("state-b"), dir.path().join("notices-b")),
            clock,
            machine,
            live.clone(),
            usage,
        )
        .unwrap();

        let res_a = match gate_a
            .try_reserve(req("a1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        let grant_b =
            reserve_and_confirm(&mut gate_b, &live, req("b1", TrialRole::Builder, "lane-a"));

        // Passing gate_a's reservation into gate_b (which also has grant_id == 1) is refused and
        // leaves gate_b's active run intact.
        assert!(
            gate_b
                .confirm_slot(res_a)
                .unwrap_err()
                .0
                .contains("different RunGate")
        );
        assert_eq!(gate_b.active_run_count(), 1);
        let res_a2 = match gate_a
            .try_reserve(req("a2", TrialRole::Builder, "lane-b"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        assert!(
            gate_b
                .cancel(res_a2)
                .unwrap_err()
                .0
                .contains("different RunGate")
        );
        assert_eq!(gate_b.active_run_count(), 1);

        // Passing gate_b's grant into gate_a is refused without mutating gate_a's rate limits or
        // active runs.
        assert!(
            gate_a
                .finish(
                    grant_b,
                    RunCompletion::RateLimited {
                        detail: "forged".to_owned(),
                    }
                )
                .unwrap_err()
                .0
                .contains("different RunGate")
        );
        assert_eq!(gate_a.active_run_count(), 2);
        assert!(
            gate_a
                .shared_state
                .with_state_mut(|s, _| Ok((s.rate_limits.is_empty(), false)))
                .unwrap()
        );

        // Forged grant_id matching gate_a's gate_id is also rejected before state mutation.
        let forged = GateGrant {
            gate_id: gate_a.gate_id,
            grant_id: 999,
            request: req("forged", TrialRole::Builder, "lane-a"),
            pool: "gemini-pro".to_owned(),
            account: "acct-1".to_owned(),
            granted_at_s: 100,
            wall_cap: WallCap::start(100, 1800),
            machine_load: Load {
                one: 1.0,
                five: 1.0,
                fifteen: 1.0,
            },
            before_snapshot: PoolSnapshot {
                at: unix_s_to_utc(100).unwrap(),
                meters: BTreeMap::new(),
                other_live_runs: 0,
            },
        };
        assert!(
            gate_a
                .finish(
                    forged,
                    RunCompletion::RateLimited {
                        detail: "forged".to_owned(),
                    }
                )
                .unwrap_err()
                .0
                .contains("unknown or already consumed")
        );
        assert_eq!(gate_a.active_run_count(), 2);
        assert!(
            gate_a
                .shared_state
                .with_state_mut(|s, _| Ok((s.rate_limits.is_empty(), false)))
                .unwrap()
        );

        // Sanitized notice paths prevent traversal and collisions for punctuation-bearing names.
        let n1 = notice_file_path(dir.path(), "../evil/lane");
        let n2 = notice_file_path(dir.path(), "___evil_lane");
        assert_eq!(n1.parent(), Some(dir.path()));
        assert_ne!(n1, n2);
    }

    #[test]
    fn usage_readings_round_trip_through_run_row_store_and_failed_after_probe_still_releases_slot()
    {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_796_119_200); // 2026-12-01T10:00:00Z
        let machine = FakeMachine::ok(2.5, 16_384 * 1024);
        let live = FakeLiveRuns::default();
        live.set("lane-a2", 1, CounterScope::Lane);
        let usage = FakeUsage::default();
        usage.set_stdout("usage-a", "five_hour_pct=90%\nweek_pct=80%\n");

        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        let grant = reserve_and_confirm(&mut gate, &live, req("r1", TrialRole::Builder, "lane-a"));
        assert_eq!(grant.before_snapshot.other_live_runs, 1);

        // During the run, another run starts on lane-a2 and quota drops.
        live.set("lane-a2", 2, CounterScope::Lane);
        usage.set_stdout("usage-a", "five_hour_pct=82%\nweek_pct=78.5%\n");
        clock.advance(1_200);
        live.bump("lane-a", -1);
        let load = grant.machine_load.clone();
        let started = unix_s_to_utc(grant.granted_at_s).unwrap();
        let out = gate.finish(grant, RunCompletion::Completed).unwrap();
        assert!(out.after_reading_error.is_none());
        assert_eq!(out.pool_reading.after.as_ref().unwrap().other_live_runs, 2);

        // Assemble a RunRow using the gate's PoolReading and Load and validate via Store.
        let fx = fixture();
        let mut row = run(&fx.config, &fx.task, 1, "run-gate-1");
        row.started_at = started;
        row.finished_at = unix_s_to_utc(clock.now_unix_s()).unwrap();
        row.load = load;
        row.pool_readings = vec![out.pool_reading.clone()];
        fx.store.append_run(row).unwrap();
        let stored = fx.store.live_runs().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].pool_readings, vec![out.pool_reading]);

        // Failed post-run usage probe still releases the active slot and returns before-only data.
        usage.set_stdout("usage-a", "five_hour_pct=80%\nweek_pct=75%\n");
        clock.advance(60);
        let grant2 = reserve_and_confirm(&mut gate, &live, req("r2", TrialRole::Builder, "lane-a"));
        usage.set_result("usage-a", Err(Error("usage probe timeout".into())));
        live.bump("lane-a", -1);
        let out2 = gate.finish(grant2, RunCompletion::Completed).unwrap();
        assert_eq!(gate.active_run_count(), 0);
        assert!(out2.pool_reading.before.is_some());
        assert!(out2.pool_reading.after.is_none());
        assert!(
            out2.after_reading_error
                .as_ref()
                .unwrap()
                .0
                .contains("usage probe timeout")
        );
    }

    #[test]
    fn agy_parser_accepts_fleet_fixtures_and_emitted_remaining_labels() {
        const FLEET_EXHAUSTED: &str =
            include_str!("../../../crates/eon-agent/tests/fixtures/agy-exhausted.tsv");
        const FLEET_HEALTHY: &str =
            include_str!("../../../crates/eon-agent/tests/fixtures/agy.tsv");

        // Exact labels emitted by the fleet's `agy /usage` command (`Weekly Limit Remaining`,
        // `Five Hour Limit Remaining`, trailing tab on `disabled`, and model-prefixed weekly).
        let gemini = parse_usage(UsageParser::Agy, Some("Gemini Models"), FLEET_EXHAUSTED).unwrap();
        assert_eq!(gemini[&Text::new("week_pct").unwrap()], 80.0);
        assert_eq!(gemini[&Text::new("five_hour_pct").unwrap()], 55.0);

        let claude = parse_usage(
            UsageParser::Agy,
            Some("Claude and GPT models"),
            FLEET_EXHAUSTED,
        )
        .unwrap();
        assert_eq!(claude[&Text::new("week_pct").unwrap()], 0.0);
        assert_eq!(claude[&Text::new("five_hour_pct").unwrap()], 0.0);

        let example =
            parse_usage(UsageParser::Agy, Some("Example Model"), FLEET_EXHAUSTED).unwrap();
        assert_eq!(example[&Text::new("week_pct").unwrap()], 90.0);
        assert!(parse_usage(UsageParser::Agy, None, FLEET_EXHAUSTED).is_err());

        let healthy = parse_usage(
            UsageParser::Agy,
            Some("Claude and GPT models"),
            FLEET_HEALTHY,
        )
        .unwrap();
        assert_eq!(healthy[&Text::new("five_hour_pct").unwrap()], 67.5);
        assert_eq!(healthy[&Text::new("week_pct").unwrap()], 48.0);

        // Utilization / used labels must never be mistaken for remaining percentages.
        for bad in [
            "Five Hour Limit Used: 50%\n",
            "Weekly Utilization: 20%\n",
            "Gemini Models\tWeekly Limit Remaining\t\n",
        ] {
            assert!(parse_usage(UsageParser::Agy, None, bad).is_err(), "{bad:?}");
        }

        // End-to-end RunGate admission against the real fleet fixture: healthy Gemini Models is
        // admitted while exhausted Claude and GPT models holds below floor.
        let dir = TempDir::new().unwrap();
        let mut lanes = sample_lanes();
        {
            let lane_a = lanes.lanes.get_mut(&Text::new("lane-a").unwrap()).unwrap();
            lane_a.usage_read.parser = UsageParser::Agy;
            lane_a.usage_read.group = Some(Text::new("Gemini Models").unwrap());
        }
        {
            let lane_b = lanes.lanes.get_mut(&Text::new("lane-b").unwrap()).unwrap();
            lane_b.usage_read.parser = UsageParser::Agy;
            lane_b.usage_read.group = Some(Text::new("Claude and GPT models").unwrap());
        }
        let usage = FakeUsage::default();
        usage.set_stdout("usage-a", FLEET_EXHAUSTED);
        usage.set_stdout("usage-b", FLEET_EXHAUSTED);
        let mut gate = RunGate::with_deps(
            lanes,
            test_config(&dir),
            FakeClock::new(1_000),
            FakeMachine::healthy(),
            FakeLiveRuns::default(),
            usage,
        )
        .unwrap();

        let res_a = match gate
            .try_reserve(req("agy-gemini", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved for healthy Gemini Models, got {other:?}"),
        };
        gate.cancel(res_a).unwrap();

        assert!(matches!(
            gate.try_reserve(req("agy-claude", TrialRole::Builder, "lane-b"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::PoolBelowFloor {
                meter,
                value,
                ..
            }) if meter == "week_pct" && value == 0.0
        ));
    }

    #[test]
    fn confirm_slot_refreshes_through_held_slot_probe_without_reacquiring_lock_file() {
        /// Simulates the fleet's `agy-run-as.sh` wrapper: `usage-wrapper` attempts to acquire the
        /// account's exclusive `flock` (and fails loudly if called while this run already holds
        /// it), whereas `usage-inner` runs inside the already-held slot without touching the lock.
        #[derive(Clone)]
        struct FlockAwareUsage {
            lock_path: PathBuf,
            wrapper_stdout: Rc<RefCell<String>>,
            inner_stdout: Rc<RefCell<String>>,
            calls: Rc<RefCell<Vec<Vec<String>>>>,
        }

        impl UsageCommandRunner for FlockAwareUsage {
            fn run(&self, command: &[String], _timeout: Duration) -> Result<CommandOutput> {
                self.calls.borrow_mut().push(command.to_vec());
                let prog = command.first().map(String::as_str).unwrap_or("");
                if prog == "usage-wrapper" {
                    let probe = open_lock_file(&self.lock_path)?;
                    match probe.try_lock() {
                        Ok(()) => {
                            return Ok(CommandOutput {
                                success: true,
                                status_code: Some(0),
                                stdout: self.wrapper_stdout.borrow().clone(),
                                stderr: String::new(),
                            });
                        }
                        Err(TryLockError::WouldBlock) => {
                            fail!("self-deadlock: usage-wrapper tried to reacquire held flock");
                        }
                        Err(TryLockError::Error(e)) => fail!("flock error: {e}"),
                    }
                }
                if prog == "usage-inner" {
                    return Ok(CommandOutput {
                        success: true,
                        status_code: Some(0),
                        stdout: self.inner_stdout.borrow().clone(),
                        stderr: String::new(),
                    });
                }
                fail!("unexpected usage command {prog:?}")
            }
        }

        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("acct.lock");
        let lock_counter = LiveRunsCounter {
            kind: LiveRunsCounterKind::LockFile,
            path: Text::new(lock_path.to_string_lossy().as_ref()).unwrap(),
        };

        let make_lanes = |with_in_slot: bool| {
            let mut lanes = sample_lanes();
            lanes.lanes.remove(&Text::new("lane-a2").unwrap());
            let lane_a = lanes.lanes.get_mut(&Text::new("lane-a").unwrap()).unwrap();
            lane_a.run_cap = 1;
            lane_a.live_runs_counter = lock_counter.clone();
            lane_a.usage_read.command = vec![Text::new("usage-wrapper").unwrap()];
            lane_a.usage_read.in_slot_command =
                with_in_slot.then(|| vec![Text::new("usage-inner").unwrap()]);
            lanes
        };

        let usage = FlockAwareUsage {
            lock_path: lock_path.clone(),
            wrapper_stdout: Rc::new(RefCell::new("five_hour_pct=80%\nweek_pct=75%\n".into())),
            inner_stdout: Rc::new(RefCell::new("five_hour_pct=72%\nweek_pct=70%\n".into())),
            calls: Rc::new(RefCell::new(Vec::new())),
        };

        // Case A: LockFile lane WITHOUT `usage_read.in_slot_command` is rejected at construction
        // so `confirm_slot`, `acquire_slot`, and `poll_queue_acquire` can never fail mid-flight
        // across a wall-clock second boundary.
        let clock = FakeClock::new(100);
        let err = RunGate::with_deps(
            make_lanes(false),
            test_config(&dir),
            clock.clone(),
            FakeMachine::healthy(),
            FsLiveRunsReader,
            usage.clone(),
        )
        .err()
        .expect("lock_file lane without in_slot_command must be rejected at construction")
        .0;
        assert!(
            err.contains("lock_file") && err.contains("usage_read.in_slot_command"),
            "{err}"
        );
        assert!(usage.calls.borrow().is_empty());

        // Case B: LockFile lane WITH `usage_read.in_slot_command` crossing a second boundary uses
        // `usage-inner` while the flock is held and refreshes `before_snapshot`.
        usage.calls.borrow_mut().clear();
        let mut gate_with_in_slot = RunGate::with_deps(
            make_lanes(true),
            GateConfig::new(dir.path().join("state-b"), dir.path().join("notices-b")),
            clock.clone(),
            FakeMachine::healthy(),
            FsLiveRunsReader,
            usage.clone(),
        )
        .unwrap();
        clock.set(200);
        let (grant, slot_file) = match gate_with_in_slot
            .acquire_slot(req("r-in-slot", TrialRole::Builder, "lane-a"), |_| {
                let f = open_lock_file(&lock_path).unwrap();
                f.try_lock().unwrap();
                clock.set(201); // crossed a second boundary before confirm_slot
                Ok(SlotAcquisition::Acquired(f))
            })
            .unwrap()
        {
            AcquireOutcome::Granted { grant, slot } => (grant, slot),
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant.granted_at_s, 201);
        assert_eq!(
            grant.before_snapshot.meters[&Text::new("five_hour_pct").unwrap()],
            72.0
        );
        assert_eq!(
            *usage.calls.borrow(),
            vec![
                vec!["usage-wrapper".to_owned()],
                vec!["usage-inner".to_owned()],
            ]
        );
        drop(slot_file);
        gate_with_in_slot
            .finish(grant, RunCompletion::Completed)
            .unwrap();

        // Case C: `acquire_slot_with_probe` overrides `in_slot_command` with the caller-supplied
        // held-slot probe without invoking `usage-wrapper` or `usage-inner`.
        usage.calls.borrow_mut().clear();
        let mut gate_custom_probe = RunGate::with_deps(
            make_lanes(true),
            GateConfig::new(dir.path().join("state-c"), dir.path().join("notices-c")),
            clock.clone(),
            FakeMachine::healthy(),
            FsLiveRunsReader,
            usage.clone(),
        )
        .unwrap();
        clock.set(300);
        let mut probed_with_held_slot = false;
        let (grant_c, slot_c) = match gate_custom_probe
            .acquire_slot_with_probe(
                req("r-custom-probe", TrialRole::Builder, "lane-a"),
                |_| {
                    let f = open_lock_file(&lock_path).unwrap();
                    f.try_lock().unwrap();
                    clock.set(301);
                    Ok(SlotAcquisition::Acquired(f))
                },
                |held_slot, _lane, _timeout| {
                    // Verify caller actually passed the held flock file without reacquiring it.
                    assert!(held_slot.metadata().is_ok());
                    probed_with_held_slot = true;
                    Ok(CommandOutput {
                        success: true,
                        status_code: Some(0),
                        stdout: "five_hour_pct=66%\nweek_pct=64%\n".to_owned(),
                        stderr: String::new(),
                    })
                },
            )
            .unwrap()
        {
            AcquireOutcome::Granted { grant, slot } => (grant, slot),
            other => panic!("expected Granted, got {other:?}"),
        };
        assert!(probed_with_held_slot);
        assert_eq!(grant_c.granted_at_s, 301);
        assert_eq!(
            grant_c.before_snapshot.meters[&Text::new("five_hour_pct").unwrap()],
            66.0
        );
        assert_eq!(
            *usage.calls.borrow(),
            vec![vec!["usage-wrapper".to_owned()]]
        );
        drop(slot_c);
        gate_custom_probe
            .finish(grant_c, RunCompletion::Completed)
            .unwrap();

        // Case D: `poll_queue_acquire` on a `lock_file` lane crossing a second boundary refreshes
        // via `usage-inner` cleanly without aborting the queue poll.
        usage.calls.borrow_mut().clear();
        clock.set(400);
        let mut queue = VecDeque::from([req("r-queue-in-slot", TrialRole::Builder, "lane-a")]);
        let (grant_d, slot_d) = match gate_with_in_slot
            .poll_queue_acquire(&mut queue, |_| {
                let f = open_lock_file(&lock_path).unwrap();
                f.try_lock().unwrap();
                clock.set(401);
                Ok(SlotAcquisition::Acquired(f))
            })
            .unwrap()
        {
            QueueAcquireOutcome::Granted { grant, slot, .. } => (grant, slot),
            other => panic!("expected Granted, got {other:?}"),
        };
        assert!(queue.is_empty());
        assert_eq!(grant_d.granted_at_s, 401);
        assert_eq!(
            *usage.calls.borrow(),
            vec![
                vec!["usage-wrapper".to_owned()],
                vec!["usage-inner".to_owned()],
            ]
        );
        drop(slot_d);
        gate_with_in_slot
            .finish(grant_d, RunCompletion::Completed)
            .unwrap();
    }

    #[test]
    fn independent_gates_share_stagger_rate_limits_lead_holds_and_recover_from_crashed_leases() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();

        let mut gate_1 = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine.clone(),
            live.clone(),
            usage.clone(),
        )
        .unwrap();
        let mut gate_2 = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine.clone(),
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        // 1. Cross-gate pending stagger: gate_1 reserves on acct-1 at t=1000; gate_2 holds on
        //    lane-a2 (same account) even before gate_1 confirms.
        let res1 = match gate_1
            .try_reserve(req("g1-r1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        assert_eq!(
            gate_2
                .try_reserve(req("g2-r1", TrialRole::Builder, "lane-a2"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::Stagger {
                account: "acct-1".to_owned(),
                wait_s: 60,
                ready_at_s: 1_060,
            })
        );

        // Gate 1 confirms at t=1005 -> gate_2 sees confirmed stagger until t=1065.
        clock.set(1_005);
        live.bump("lane-a", 1);
        let grant1 = match gate_1.confirm_slot(res1).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        clock.set(1_064);
        assert_eq!(
            gate_2
                .try_reserve(req("g2-r1", TrialRole::Builder, "lane-a2"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::Stagger {
                account: "acct-1".to_owned(),
                wait_s: 1,
                ready_at_s: 1_065,
            })
        );
        clock.set(1_065);
        let grant2 = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("g2-r1", TrialRole::Builder, "lane-a2"),
        );
        live.bump("lane-a2", -1);
        gate_2.finish(grant2, RunCompletion::Completed).unwrap();

        // 2. Cross-gate rate-limit cooldown, single-retry ownership, and crash recovery.
        clock.set(1_100);
        live.bump("lane-a", -1);
        gate_1
            .finish(
                grant1,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 from gate_1".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(
            gate_2
                .try_reserve(req("g2-r2", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitCooldown {
                lane: "lane-a".to_owned(),
                wait_s: 600,
                ready_at_s: 1_700,
            })
        );

        // At t=1700, gate_1 reserves the single post-cooldown retry; gate_2 holds on RetryInFlight.
        clock.set(1_700);
        let crashed_retry = match gate_1
            .try_reserve(req("g1-retry-crash", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved retry, got {other:?}"),
        };
        assert_eq!(
            gate_2
                .try_reserve(req("g2-r2", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitRetryInFlight {
                lane: "lane-a".to_owned(),
            })
        );

        // Simulate gate_1 crashing while holding the retry lease (drops FD without cancel/finish):
        // gate_2 automatically prunes the dead lease lock and takes over the post-cooldown retry.
        drop((crashed_retry, gate_1));
        let retry_grant = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("g2-retry", TrialRole::Builder, "lane-a"),
        );

        // 3. Retry fails again -> HeldForLead persisted across a fresh restarted gate_3.
        clock.set(1_705);
        live.bump("lane-a", -1);
        let retry_out = gate_2
            .finish(
                retry_grant,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 retry failed".to_owned(),
                },
            )
            .unwrap();
        let notice_path = match retry_out.rate_limit {
            Some(RateLimitAction::HeldForLead { notice_path, .. }) => notice_path,
            other => panic!("expected HeldForLead, got {other:?}"),
        };

        // Restarted gate_3 on the same state_dir MUST still hold lane-a for the lead.
        clock.set(10_000);
        let mut gate_3 = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock.clone(),
            machine,
            live.clone(),
            usage,
        )
        .unwrap();
        assert_eq!(
            gate_3
                .try_reserve(req("g3-after-restart", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitHeldForLead {
                lane: "lane-a".to_owned(),
                notice_path,
            })
        );

        // Lead clears the hold on gate_3 -> unblocks gate_2 as well.
        assert!(gate_3.clear_rate_limit_hold("lane-a").unwrap());
        let resumed = reserve_and_confirm(
            &mut gate_2,
            &live,
            req("g2-resumed", TrialRole::Builder, "lane-a"),
        );
        live.bump("lane-a", -1);
        gate_2.finish(resumed, RunCompletion::Completed).unwrap();
    }

    #[test]
    fn confirm_slot_rechecks_rate_limit_cooldown_lead_hold_and_retry_ownership() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        let mut lanes = sample_lanes();
        lanes
            .lanes
            .get_mut(&Text::new("lane-a").unwrap())
            .unwrap()
            .stagger_s = 0;

        let mut gate = RunGate::with_deps(
            lanes.clone(),
            test_config(&dir),
            clock.clone(),
            machine.clone(),
            live.clone(),
            usage.clone(),
        )
        .unwrap();

        // Case 1: Cap-2 lane has Run 1 active; Run 2 reserves; Run 1 finishes with 429 before
        // Run 2 confirms -> confirm_slot MUST hold on RateLimitCooldown instead of granting.
        let g1 = reserve_and_confirm(&mut gate, &live, req("r1", TrialRole::Builder, "lane-a"));
        let res2 = match gate
            .try_reserve(req("r2", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved r2, got {other:?}"),
        };
        clock.set(1_100);
        live.bump("lane-a", -1);
        gate.finish(
            g1,
            RunCompletion::RateLimited {
                detail: "HTTP 429".to_owned(),
            },
        )
        .unwrap();

        clock.set(1_101);
        live.bump("lane-a", 1);
        let decision = gate.confirm_slot(res2).unwrap();
        assert_eq!(
            decision,
            GateDecision::Hold {
                request: req("r2", TrialRole::Builder, "lane-a"),
                reason: HoldReason::RateLimitCooldown {
                    lane: "lane-a".to_owned(),
                    wait_s: 599,
                    ready_at_s: 1_700,
                },
            }
        );
        live.bump("lane-a", -1);
        assert_eq!(gate.active_run_count(), 0);

        // Case 2: Sibling 429 extends cooldown while a post-cooldown retry reservation is pending
        // -> confirm_slot holds on the extended cooldown and restores CoolingDown.
        let dir2 = TempDir::new().unwrap();
        let clock2 = FakeClock::new(2_000);
        let live2 = FakeLiveRuns::default();
        let mut gate2 = RunGate::with_deps(
            lanes.clone(),
            test_config(&dir2),
            clock2.clone(),
            machine.clone(),
            live2.clone(),
            usage.clone(),
        )
        .unwrap();
        let s1 = reserve_and_confirm(&mut gate2, &live2, req("s1", TrialRole::Builder, "lane-a"));
        let s2 = reserve_and_confirm(&mut gate2, &live2, req("s2", TrialRole::Builder, "lane-a"));
        clock2.set(2_010);
        live2.bump("lane-a", -1);
        gate2
            .finish(
                s1,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 first".to_owned(),
                },
            )
            .unwrap();
        clock2.set(2_610);
        let retry_res = match gate2
            .try_reserve(req("retry-1", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved retry, got {other:?}"),
        };
        // Pre-cooldown sibling s2 hits 429 at t=2615 before retry_res confirms -> extends until 3215.
        clock2.set(2_615);
        live2.bump("lane-a", -1);
        gate2
            .finish(
                s2,
                RunCompletion::RateLimited {
                    detail: "HTTP 429 sibling".to_owned(),
                },
            )
            .unwrap();
        clock2.set(2_616);
        live2.bump("lane-a", 1);
        assert_eq!(
            gate2.confirm_slot(retry_res).unwrap(),
            GateDecision::Hold {
                request: req("retry-1", TrialRole::Builder, "lane-a"),
                reason: HoldReason::RateLimitCooldown {
                    lane: "lane-a".to_owned(),
                    wait_s: 599,
                    ready_at_s: 3_215,
                },
            }
        );
        live2.bump("lane-a", -1);
        // Lane is cleanly restored to CoolingDown until 3215 (not stranded in RetryInFlight).
        clock2.set(3_214);
        assert_eq!(
            gate2
                .try_reserve(req("after-ext", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::RateLimitCooldown {
                lane: "lane-a".to_owned(),
                wait_s: 1,
                ready_at_s: 3_215,
            })
        );

        // Case 3: Foreign RetryInFlight ownership at confirmation holds with RateLimitRetryInFlight,
        // and HeldForLead triggered before confirmation holds with RateLimitHeldForLead.
        let dir3 = TempDir::new().unwrap();
        let clock3 = FakeClock::new(3_000);
        let live3 = FakeLiveRuns::default();
        let mut gate_a = RunGate::with_deps(
            lanes.clone(),
            test_config(&dir3),
            clock3.clone(),
            machine.clone(),
            live3.clone(),
            usage.clone(),
        )
        .unwrap();
        let mut gate_b = RunGate::with_deps(
            lanes,
            test_config(&dir3),
            clock3.clone(),
            machine,
            live3.clone(),
            usage,
        )
        .unwrap();
        let a1 = reserve_and_confirm(&mut gate_a, &live3, req("a1", TrialRole::Builder, "lane-a"));
        let res_b = match gate_b
            .try_reserve(req("b-stale", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved b-stale, got {other:?}"),
        };
        // a1 triggers cooldown until 3600; at 3600 gate_a reserves the single retry slot.
        clock3.set(3_000);
        live3.bump("lane-a", -1);
        gate_a
            .finish(
                a1,
                RunCompletion::RateLimited {
                    detail: "HTTP 429".to_owned(),
                },
            )
            .unwrap();
        clock3.set(3_600);
        let a_retry = reserve_and_confirm(
            &mut gate_a,
            &live3,
            req("a-retry", TrialRole::Builder, "lane-a"),
        );
        // gate_b's stale pre-cooldown reservation must hold because gate_a owns RetryInFlight.
        assert_eq!(
            gate_b.confirm_slot(res_b).unwrap(),
            GateDecision::Hold {
                request: req("b-stale", TrialRole::Builder, "lane-a"),
                reason: HoldReason::RateLimitRetryInFlight {
                    lane: "lane-a".to_owned(),
                },
            }
        );
        live3.bump("lane-a", -1);
        gate_a.finish(a_retry, RunCompletion::Completed).unwrap();
    }

    #[test]
    fn shared_account_counter_enforces_admission_and_requires_pool_attributable_snapshots() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let usage = FakeUsage::default();
        let acct_caps = dir.path().join("google1-acct.json");
        let pool_caps = dir.path().join("google1-pools.json");

        // Two lanes on the same account covering DIFFERENT quota pools (gemini-pro vs claude-pro)
        // sharing the same account aggregate counter (`acct_caps`).
        let mut lanes = sample_lanes();
        lanes.lanes.remove(&Text::new("lane-a2").unwrap());
        for name in ["lane-a", "lane-b"] {
            let lane = lanes.lanes.get_mut(&Text::new(name).unwrap()).unwrap();
            lane.account = Some(Text::new("google1").unwrap());
            lane.stagger_s = 0;
            lane.run_cap = 2;
            lane.live_runs_counter = LiveRunsCounter {
                kind: LiveRunsCounterKind::CapsFile,
                path: Text::new(acct_caps.to_string_lossy().as_ref()).unwrap(),
            };
        }

        // Sub-case A: Without a pool-attributable counter, admission STILL enforces the account
        // aggregate cap, while admitting for a usage snapshot fails closed rather than assigning
        // a claude-pro run to gemini-pro.
        let mut gate_unattributable = RunGate::with_deps(
            lanes.clone(),
            GateConfig::new(dir.path().join("state-a"), dir.path().join("notices-a")),
            clock.clone(),
            machine.clone(),
            FsLiveRunsReader,
            usage.clone(),
        )
        .unwrap();
        fs::write(&acct_caps, r#"{"live_runs": 2}"#).unwrap();
        assert_eq!(
            gate_unattributable
                .try_reserve(req("cap-hit", TrialRole::Builder, "lane-a"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::LaneRunCap {
                lane: "lane-a".to_owned(),
                live: 2,
                cap: 2,
            })
        );
        // With 1 claude-pro run active (aggregate = 1 < 2), trying to admit lane-a must fail
        // because `acct_caps` is shared across pools and not pool-attributable.
        fs::write(&acct_caps, r#"{"live_runs": 1}"#).unwrap();
        let err = gate_unattributable
            .try_reserve(req("cross-pool", TrialRole::Builder, "lane-a"))
            .unwrap_err()
            .0;
        assert!(
            err.contains("other pool(s)") && err.contains("claude-pro"),
            "{err}"
        );

        // Sub-case B: With `pool_live_runs_counter` providing per-lane counts while keeping the
        // aggregate `live_runs_counter` for admission, a concurrent claude-pro run counts toward
        // the account cap of 2 AND leaves gemini-pro's before/after snapshots quiet (0).
        for name in ["lane-a", "lane-b"] {
            let lane = lanes.lanes.get_mut(&Text::new(name).unwrap()).unwrap();
            lane.pool_live_runs_counter = Some(LiveRunsCounter {
                kind: LiveRunsCounterKind::CapsFile,
                path: Text::new(pool_caps.to_string_lossy().as_ref()).unwrap(),
            });
        }
        let mut gate_split = RunGate::with_deps(
            lanes,
            GateConfig::new(dir.path().join("state-b"), dir.path().join("notices-b")),
            clock.clone(),
            machine,
            FsLiveRunsReader,
            usage,
        )
        .unwrap();
        // External claude-pro run active: account aggregate = 1, lane-a = 0, lane-b = 1.
        fs::write(&acct_caps, r#"{"live_runs": 1}"#).unwrap();
        fs::write(&pool_caps, r#"{"lanes": {"lane-a": 0, "lane-b": 1}}"#).unwrap();
        let res = match gate_split
            .try_reserve(req("gemini-quiet", TrialRole::Builder, "lane-a"))
            .unwrap()
        {
            ReserveDecision::Reserved(r) => r,
            other => panic!("expected Reserved, got {other:?}"),
        };
        // lane-a acquires slot: account aggregate = 2 (at cap), lane-a = 1 (self), lane-b = 1.
        fs::write(&acct_caps, r#"{"live_runs": 2}"#).unwrap();
        fs::write(&pool_caps, r#"{"lanes": {"lane-a": 1, "lane-b": 1}}"#).unwrap();
        let grant = match gate_split.confirm_slot(res).unwrap() {
            GateDecision::Granted(g) => g,
            other => panic!("expected Granted, got {other:?}"),
        };
        assert_eq!(grant.before_snapshot.other_live_runs, 0);

        // Admission still enforces account aggregate cap (2/2) for another run on either lane.
        assert_eq!(
            gate_split
                .try_reserve(req("third-run", TrialRole::Reviewer, "lane-b"))
                .unwrap(),
            ReserveDecision::Hold(HoldReason::LaneRunCap {
                lane: "lane-b".to_owned(),
                live: 2,
                cap: 2,
            })
        );

        // lane-a finishes while claude-pro run is still active -> after snapshot is still quiet (0).
        fs::write(&acct_caps, r#"{"live_runs": 1}"#).unwrap();
        fs::write(&pool_caps, r#"{"lanes": {"lane-a": 0, "lane-b": 1}}"#).unwrap();
        clock.advance(30);
        let out = gate_split.finish(grant, RunCompletion::Completed).unwrap();
        assert_eq!(out.pool_reading.after.as_ref().unwrap().other_live_runs, 0);
    }

    #[test]
    fn poll_queue_and_poll_queue_acquire_preserve_lane_and_position_on_confirmation_hold() {
        let dir = TempDir::new().unwrap();
        let clock = FakeClock::new(1_000);
        let machine = FakeMachine::healthy();
        let live = FakeLiveRuns::default();
        let usage = FakeUsage::default();
        usage.set_stdout("usage-a", "five_hour_pct=5%\nweek_pct=50%\n");
        usage.set_stdout("usage-b", "week_pct=50%\n");
        usage.set_stdout("usage-c", "Balance: 50 credits\n");

        let mut gate = RunGate::with_deps(
            sample_lanes(),
            test_config(&dir),
            clock,
            machine,
            live.clone(),
            usage,
        )
        .unwrap();

        // Sub-case A: `poll_queue` returns `queue_index`, `confirm_slot` returns the original
        // `RunRequest` on hold, and `requeue_request` restores its exact lane and FIFO position.
        let mut queue = VecDeque::from([
            req("run-a", TrialRole::Builder, "lane-a"),
            req("run-b1", TrialRole::Builder, "lane-b"),
            req("run-b2", TrialRole::Builder, "lane-b"),
        ]);
        let (res_b1, queue_index) = match gate.poll_queue(&mut queue).unwrap() {
            PollOutcome::Reserved {
                reservation,
                queue_index,
                held,
            } => {
                assert_eq!(held.len(), 1);
                assert_eq!(held[0].0, "run-a");
                (reservation, queue_index)
            }
            other => panic!("expected Reserved run-b1, got {other:?}"),
        };
        assert_eq!(queue_index, 1);
        live.set("lane-b", 3, CounterScope::Lane); // oversubscribed at confirm time
        let returned = match gate.confirm_slot(res_b1).unwrap() {
            GateDecision::Hold { request, reason } => {
                assert!(matches!(reason, HoldReason::LaneRunCap { .. }));
                request
            }
            other => panic!("expected Hold with original request, got {other:?}"),
        };
        assert_eq!(returned, req("run-b1", TrialRole::Builder, "lane-b"));
        requeue_request(&mut queue, queue_index, returned);
        let ids: Vec<(&str, &str)> = queue
            .iter()
            .map(|r| (r.id.as_str(), r.lane.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("run-a", "lane-a"),
                ("run-b1", "lane-b"),
                ("run-b2", "lane-b"),
            ]
        );

        // Sub-case B: `poll_queue_acquire` never removes a request until confirmation grants it,
        // prevents a later same-lane request (`run-b2`) from leapfrogging `run-b1`, and still
        // dispatches a healthy request on another lane (`run-c`).
        live.set("lane-b", 0, CounterScope::Lane);
        queue.push_back(req("run-c", TrialRole::Builder, "lane-c"));
        let (grant_c, (), held) = match gate
            .poll_queue_acquire(&mut queue, |res| {
                if res.request.lane == "lane-b" {
                    // Simulate contention arriving right before confirmation on lane-b.
                    live.set("lane-b", 3, CounterScope::Lane);
                    Ok(SlotAcquisition::Acquired(()))
                } else {
                    live.bump(&res.request.lane, 1);
                    Ok(SlotAcquisition::Acquired(()))
                }
            })
            .unwrap()
        {
            QueueAcquireOutcome::Granted { grant, slot, held } => (grant, slot, held),
            other => panic!("expected Granted run-c, got {other:?}"),
        };
        assert_eq!(grant_c.request.id, "run-c");
        assert_eq!(grant_c.request.lane, "lane-c");
        // Both run-b1 and run-b2 were held (run-b2 did not leapfrog run-b1), and queue order for
        // all three held requests is preserved verbatim.
        assert_eq!(held.len(), 3);
        assert_eq!(held[0].0, "run-a");
        assert_eq!(held[1].0, "run-b1");
        assert_eq!(held[2].0, "run-b2");
        let remaining: Vec<(&str, &str)> = queue
            .iter()
            .map(|r| (r.id.as_str(), r.lane.as_str()))
            .collect();
        assert_eq!(
            remaining,
            vec![
                ("run-a", "lane-a"),
                ("run-b1", "lane-b"),
                ("run-b2", "lane-b"),
            ]
        );
        live.bump("lane-c", -1);
        gate.finish(grant_c, RunCompletion::Completed).unwrap();
    }
}
