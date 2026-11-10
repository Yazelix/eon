//! Bounded machine pressure and 1 Hz telemetry for `eon-bench response`
//! (docs/DEVELOPMENT.md). Each profile runs as separate
//! `eon-bench pressure-worker` children with their own deadlines, so a memory
//! hog never swaps the driver's heap and a stuck driver cannot leave pressure
//! running. The sampler reads host-wide procfs counters, the bench's own cgroup
//! and the counters of the processes the driver owns; never terminal contents
//! or another process's memory, so a busy foreign process cannot stall it.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::{CString, OsString},
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    environment::{self, die_with_parent, monotonic_ns},
    json::Value,
};

type Result<T = ()> = std::result::Result<T, String>;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
/// The bounds eon-stress-responsive-ap1j6.1 approved; no flag exceeds them.
const MAX_SECONDS: u64 = 120;
const MIN_SECONDS: u64 = 10;
const WARMUP: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(2);
const SEQUENTIAL_FILE: u64 = 512 * MIB;
const METADATA_FILE: usize = 4096;
const MAX_ALLOCATED: u64 = 2 * GIB;
const START_FREE: u64 = 15 * GIB;
const ABORT_FREE: u64 = 10 * GIB;
const MAX_MEMORY_MIB: u64 = 64 * 1024;
/// Room left for the driver, Eon and the kernel beside a memory hog, in its
/// cgroup and on the host above the guard.
const MEMORY_HEADROOM: u64 = 512 * MIB;
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);
const SAMPLER_LAG: Duration = Duration::from_secs(5);
const TMPFS_MAGIC: u64 = 0x0102_1994;
/// Warm cases in one response run; each gets at most one pressure window.
pub(crate) const WARM_CASES: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    None,
    CpuHalf,
    CpuAll,
    Disk,
    Memory,
}
impl Profile {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::None,
            "cpu-half" => Self::CpuHalf,
            "cpu-all" => Self::CpuAll,
            "disk" => Self::Disk,
            "memory" => Self::Memory,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CpuHalf => "cpu-half",
            Self::CpuAll => "cpu-all",
            Self::Disk => "disk",
            Self::Memory => "memory",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub profile: Profile,
    pub seconds: u64,
    pub workers: u64,
    pub memory_mib: u64,
    pub guard_available_mib: u64,
}
pub const FLAGS: [&str; 5] = [
    "--pressure",
    "--pressure-seconds",
    "--pressure-workers",
    "--pressure-memory-mib",
    "--guard-available-mib",
];
impl Settings {
    pub fn parse(values: &HashMap<&str, &str>) -> Result<Self> {
        let profile = values.get("--pressure").copied().unwrap_or("none");
        let profile = Profile::parse(profile)
            .ok_or("--pressure must be none, cpu-half, cpu-all, disk or memory")?;
        let number = |key: &str, min: u64, max: u64| -> Result<Option<u64>> {
            values
                .get(key)
                .map(|value| {
                    value
                        .parse::<u64>()
                        .ok()
                        .filter(|n| (min..=max).contains(n))
                        .ok_or(format!("{key} must be {min}..{max}"))
                })
                .transpose()
        };
        let only = |key: &str, allowed: bool| -> Result {
            if values.contains_key(key) && !allowed {
                return Err(format!(
                    "{key} does not apply to --pressure {}",
                    profile.name()
                ));
            }
            Ok(())
        };
        only("--pressure-seconds", profile != Profile::None)?;
        only("--pressure-workers", profile == Profile::Disk)?;
        only("--pressure-memory-mib", profile == Profile::Memory)?;
        let memory_mib = number("--pressure-memory-mib", 1, MAX_MEMORY_MIB)?;
        if profile == Profile::Memory && memory_mib.is_none() {
            return Err("--pressure memory needs --pressure-memory-mib".into());
        }
        let settings = Self {
            profile,
            seconds: number("--pressure-seconds", MIN_SECONDS, MAX_SECONDS)?.unwrap_or(60),
            workers: number("--pressure-workers", 1, 2)?.unwrap_or(1),
            memory_mib: memory_mib.unwrap_or(0),
            guard_available_mib: number("--guard-available-mib", 0, 1 << 20)?.unwrap_or(6144),
        };
        if settings.allocated() > MAX_ALLOCATED {
            return Err("disk pressure would allocate more than 2 GiB".into());
        }
        Ok(settings)
    }
    /// Bytes the disk workers may hold on the filesystem at once.
    fn allocated(&self) -> u64 {
        match self.profile {
            Profile::Disk => self.workers * (SEQUENTIAL_FILE + METADATA_FILE as u64),
            _ => 0,
        }
    }
    /// The longest a run's warm cases can spend under pressure, stop included.
    pub(crate) fn total(&self) -> Duration {
        match self.profile {
            Profile::None => Duration::ZERO,
            _ => (Duration::from_secs(self.seconds) + STOP_GRACE) * WARM_CASES,
        }
    }
    pub(crate) fn value(&self) -> Value {
        object([
            ("profile", self.profile.name().into()),
            (
                "seconds_per_case",
                (self.profile != Profile::None)
                    .then_some(self.seconds)
                    .into(),
            ),
            ("warmup_ms", (WARMUP.as_millis() as u64).into()),
            (
                "workers",
                (self.profile == Profile::Disk)
                    .then_some(self.workers)
                    .into(),
            ),
            (
                "memory_mib",
                (self.profile == Profile::Memory)
                    .then_some(self.memory_mib)
                    .into(),
            ),
            ("guard_available_mib", self.guard_available_mib.into()),
            ("abort_free_bytes", ABORT_FREE.into()),
            ("sequential_file_bytes", SEQUENTIAL_FILE.into()),
            ("metadata_file_bytes", (METADATA_FILE as u64).into()),
        ])
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// `name value` (vmstat) and `Name: value kB` (meminfo, memory.events) lines.
fn keyed(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()?.trim_end_matches(':') != key {
            return None;
        }
        fields.next()?.parse().ok()
    })
}

/// The `total=` microseconds of a PSI file's `some` and `full` lines.
fn psi(text: &str) -> (Option<u64>, Option<u64>) {
    let total = |kind: &str| {
        text.lines()
            .find(|line| line.starts_with(kind))?
            .split_whitespace()
            .find_map(|field| field.strip_prefix("total=")?.parse().ok())
    };
    (total("some "), total("full "))
}

/// A cgroup v2 limit file: a number, or `max` for none.
fn limit(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// The unified-hierarchy path in `/proc/self/cgroup`.
fn cgroup_path(text: &str) -> Option<PathBuf> {
    let path = text.lines().find_map(|line| line.strip_prefix("0::"))?;
    Some(Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

fn own_cgroup() -> Option<PathBuf> {
    cgroup_path(&fs::read_to_string("/proc/self/cgroup").ok()?)
}

fn statfs(path: &Path) -> Result<libc::statfs> {
    let name = CString::new(path.as_os_str().as_bytes()).map_err(failure)?;
    // SAFETY: statfs is plain data that the call fills.
    let mut stat = unsafe { std::mem::zeroed::<libc::statfs>() };
    // SAFETY: name is NUL-terminated and stat is a valid statfs.
    if unsafe { libc::statfs(name.as_ptr(), &raw mut stat) } != 0 {
        return Err(format!(
            "{}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(stat)
}

fn free_bytes(path: &Path) -> Option<u64> {
    let stat = statfs(path).ok()?;
    Some(stat.f_bavail as u64 * stat.f_bsize as u64)
}

/// Why a profile cannot start here, checked before every pressure window;
/// `available_kb` is the host's MemAvailable.
fn preflight(
    settings: &Settings,
    directory: &Path,
    cgroup: Option<&Path>,
    available_kb: Option<u64>,
) -> Result {
    match settings.profile {
        Profile::None | Profile::CpuHalf | Profile::CpuAll => Ok(()),
        Profile::Disk => {
            let stat = statfs(directory)?;
            if stat.f_type as u64 == TMPFS_MAGIC {
                return Err(format!(
                    "disk pressure needs a real filesystem; {} is tmpfs",
                    directory.display()
                ));
            }
            let free = stat.f_bavail as u64 * stat.f_bsize as u64;
            if free < START_FREE {
                return Err(format!(
                    "disk pressure needs 15 GiB free; {} has {} MiB",
                    directory.display(),
                    free / MIB
                ));
            }
            Ok(())
        }
        Profile::Memory => {
            let cgroup = cgroup.ok_or("memory pressure needs a cgroup v2 hierarchy")?;
            let read = |name: &str| fs::read_to_string(cgroup.join(name)).ok();
            let max = read("memory.max").as_deref().and_then(limit);
            let swap = read("memory.swap.max").as_deref().and_then(limit);
            let (Some(max), Some(_)) = (max, swap) else {
                return Err(format!(
                    "memory pressure needs memory.max and memory.swap.max on {}; run under \
                     systemd-run --user --scope -p MemoryMax=... -p MemorySwapMax=...",
                    cgroup.display()
                ));
            };
            let current = read("memory.current")
                .as_deref()
                .and_then(limit)
                .ok_or_else(|| format!("{} has no memory.current", cgroup.display()))?;
            let request = settings.memory_mib * MIB;
            if current + request + MEMORY_HEADROOM > max {
                return Err(format!(
                    "--pressure-memory-mib {} plus the {} MiB the cgroup already uses and \
                     512 MiB headroom exceeds memory.max {} MiB",
                    settings.memory_mib,
                    current / MIB,
                    max / MIB
                ));
            }
            let available =
                available_kb.ok_or("MemAvailable is missing from /proc/meminfo")? * 1024;
            if available < request + settings.guard_available_mib * MIB + MEMORY_HEADROOM {
                return Err(format!(
                    "--pressure-memory-mib {} would leave the host's {} MiB MemAvailable \
                     within 512 MiB of the {} MiB guard",
                    settings.memory_mib,
                    available / MIB,
                    settings.guard_available_mib
                ));
            }
            Ok(())
        }
    }
}

/// One process's counters, or several summed per command name with their `count`.
#[derive(Clone, Debug, Default, PartialEq)]
struct Process {
    count: u64,
    cpu_ticks: u64,
    major_faults: u64,
    rss_kb: u64,
    swap_kb: u64,
    write_bytes: u64,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    at: u64,
    /// Total stall microseconds: cpu some, io some, io full, memory some, memory full.
    psi: [Option<u64>; 5],
    /// pswpin, pswpout, pgmajfault.
    vm: [Option<u64>; 3],
    /// MemAvailable, Dirty, Writeback, SwapFree (kB).
    memory: [Option<u64>; 4],
    /// memory.current, memory.swap.current, oom + oom_kill events.
    cgroup: [Option<u64>; 3],
    free: Option<u64>,
    /// Owned processes by PID and start time, with their command names.
    processes: BTreeMap<(u32, String), (String, Process)>,
}
const COLUMNS: &str = "at_ns\tkind\tlabel\tcpu_some_us\tio_some_us\tio_full_us\tmemory_some_us\tmemory_full_us\tpswpin\tpswpout\tpgmajfault\tmem_available_kb\tdirty_kb\twriteback_kb\tswap_free_kb\tcgroup_current\tcgroup_swap\tcgroup_oom\tfree_bytes\tprocesses";
impl Snapshot {
    fn read(cgroup: Option<&Path>, directory: &Path, owned: &[(u32, String)]) -> Self {
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        let (cpu, _) = psi(&read("/proc/pressure/cpu"));
        let (io_some, io_full) = psi(&read("/proc/pressure/io"));
        let (memory_some, memory_full) = psi(&read("/proc/pressure/memory"));
        let vmstat = read("/proc/vmstat");
        let meminfo = read("/proc/meminfo");
        let group = |name: &str| cgroup.and_then(|c| fs::read_to_string(c.join(name)).ok());
        let events = group("memory.events");
        let processes = owned
            .iter()
            .filter_map(|(pid, identity)| {
                Some(((*pid, identity.clone()), process(*pid, identity)?))
            })
            .collect();
        Self {
            at: monotonic_ns(),
            psi: [cpu, io_some, io_full, memory_some, memory_full],
            vm: ["pswpin", "pswpout", "pgmajfault"].map(|key| keyed(&vmstat, key)),
            memory: ["MemAvailable", "Dirty", "Writeback", "SwapFree"]
                .map(|key| keyed(&meminfo, key)),
            cgroup: [
                group("memory.current").as_deref().and_then(limit),
                group("memory.swap.current").as_deref().and_then(limit),
                events.map(|text| {
                    keyed(&text, "oom").unwrap_or(0) + keyed(&text, "oom_kill").unwrap_or(0)
                }),
            ],
            free: free_bytes(directory),
            processes,
        }
    }
    fn row(&self, kind: &str, label: &str) -> String {
        let cell = |value: &Option<u64>| value.map(|n| n.to_string()).unwrap_or_default();
        let mut row = format!("{}\t{kind}\t{label}", self.at);
        for value in self
            .psi
            .iter()
            .chain(&self.vm)
            .chain(&self.memory)
            .chain(&self.cgroup)
            .chain([&self.free])
        {
            row.push('\t');
            row.push_str(&cell(value));
        }
        row.push('\t');
        row.push_str(
            &named(self.processes.values())
                .iter()
                .map(|(name, p)| {
                    format!(
                        "{name}:n={},cpu_ticks={},majflt={},rss_kb={},swap_kb={},write_bytes={}",
                        p.count, p.cpu_ticks, p.major_faults, p.rss_kb, p.swap_kb, p.write_bytes
                    )
                })
                .collect::<Vec<_>>()
                .join(";"),
        );
        row.push('\n');
        row
    }
}

/// Processes summed per command name, which keeps rows and records bounded.
fn named<'a>(
    processes: impl IntoIterator<Item = &'a (String, Process)>,
) -> BTreeMap<String, Process> {
    let mut named = BTreeMap::<String, Process>::new();
    for (name, process) in processes {
        let entry = named.entry(name.clone()).or_default();
        entry.count += process.count;
        entry.cpu_ticks += process.cpu_ticks;
        entry.major_faults += process.major_faults;
        entry.rss_kb += process.rss_kb;
        entry.swap_kb += process.swap_kb;
        entry.write_bytes += process.write_bytes;
    }
    named
}

/// One owned process's counters, by its command name; None once its PID
/// no longer names the process with this start time.
fn process(pid: u32, identity: &str) -> Option<(String, Process)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (head, rest) = stat.rsplit_once(") ")?;
    let name = head.split_once(" (")?.1.to_owned();
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    // Field 22 of proc(5), the start time the launcher records as identity.
    if fields.get(19) != Some(&identity) {
        return None;
    }
    let field = |index: usize| fields.get(index)?.parse::<u64>().ok();
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let io = fs::read_to_string(format!("/proc/{pid}/io")).unwrap_or_default();
    Some((
        name,
        Process {
            count: 1,
            // Fields 12, 14 and 15 of proc(5), counted after the command name.
            major_faults: field(9)?,
            cpu_ticks: field(11)? + field(12)?,
            rss_kb: keyed(&status, "VmRSS").unwrap_or(0),
            swap_kb: keyed(&status, "VmSwap").unwrap_or(0),
            write_bytes: keyed(&io, "write_bytes").unwrap_or(0),
        },
    ))
}

/// Why pressure must not run now, if it must not. A counter a guard needs and
/// cannot read stops pressure too, rather than leaving it unwatched.
fn guard(settings: &Settings, now: &Snapshot, oom_baseline: Option<u64>) -> Option<String> {
    let Some(available) = now.memory[0] else {
        return Some("MemAvailable is missing from /proc/meminfo".into());
    };
    if available * 1024 <= settings.guard_available_mib * MIB {
        return Some(format!(
            "host MemAvailable {} MiB is at or below the {} MiB guard",
            available / 1024,
            settings.guard_available_mib
        ));
    }
    if settings.profile == Profile::Disk {
        let Some(free) = now.free else {
            return Some("the pressure filesystem's free space is unreadable".into());
        };
        if free < ABORT_FREE {
            return Some(format!(
                "pressure filesystem has {} MiB free, below 10 GiB",
                free / MIB
            ));
        }
    }
    match (now.cgroup[2], oom_baseline) {
        (Some(now), Some(before)) if now > before => Some(format!(
            "{} new OOM events in the bench's cgroup",
            now - before
        )),
        (None, Some(_)) => Some("the bench cgroup's memory.events became unreadable".into()),
        (_, None) if settings.profile == Profile::Memory => {
            Some("memory pressure needs the bench cgroup's memory.events".into())
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
struct Peaks {
    dirty_kb: u64,
    writeback_kb: u64,
    min_available_kb: Option<u64>,
    /// Peak RSS and swap summed per command name.
    owned: BTreeMap<String, (u64, u64)>,
    /// Each owned process's last counters, kept after it exits.
    last: BTreeMap<(u32, String), (String, Process)>,
}
impl Peaks {
    fn add(&mut self, snapshot: &Snapshot) {
        self.dirty_kb = self.dirty_kb.max(snapshot.memory[1].unwrap_or(0));
        self.writeback_kb = self.writeback_kb.max(snapshot.memory[2].unwrap_or(0));
        if let Some(available) = snapshot.memory[0] {
            self.min_available_kb = Some(
                self.min_available_kb
                    .map_or(available, |m| m.min(available)),
            );
        }
        for (name, process) in named(snapshot.processes.values()) {
            let peak = self.owned.entry(name).or_default();
            peak.0 = peak.0.max(process.rss_kb);
            peak.1 = peak.1.max(process.swap_kb);
        }
        for (key, process) in &snapshot.processes {
            self.last.insert(key.clone(), process.clone());
        }
    }
}

/// One pressure worker process.
struct Worker {
    kind: &'static str,
    child: Child,
    report: PathBuf,
    /// Kept once reaped: a reaped worker's PID may name another process.
    status: Option<ExitStatus>,
}
impl Worker {
    fn poll(&mut self) -> Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait().map_err(failure)?;
        }
        Ok(self.status)
    }
}

/// The case being measured, with its OOM baseline and the peaks since its begin.
struct Active {
    case: String,
    oom: Option<u64>,
    /// Whether workers run, so guards apply.
    guarded: bool,
    peaks: Peaks,
}

/// The state the sampler thread shares with the driver.
struct Shared {
    stop: AtomicBool,
    file: Mutex<File>,
    active: Mutex<Option<Active>>,
    abort: Mutex<Option<String>>,
    owned: Mutex<Vec<(u32, String)>>,
    /// The open window's workers, so a tripped guard kills them at once.
    workers: Mutex<Vec<Worker>>,
}
impl Shared {
    fn write(&self, row: &str) -> Result {
        self.file
            .lock()
            .map_err(failure)?
            .write_all(row.as_bytes())
            .map_err(|error| format!("telemetry write failed: {error}"))
    }
    /// Records why pressure must stop and kills the running workers (SIGKILL, which
    /// std never sends to a reaped child); true the first time.
    fn trip(&self, reason: String) -> bool {
        let first = match self.abort.lock() {
            Ok(mut abort) if abort.is_none() => {
                *abort = Some(reason);
                true
            }
            _ => false,
        };
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.iter_mut() {
                let _ = worker.child.kill();
            }
        }
        first
    }
    fn aborted(&self) -> Result<Option<String>> {
        Ok(self.abort.lock().map_err(failure)?.clone())
    }
}

/// A running pressure window over one case's samples.
pub(crate) struct Window {
    case: String,
    guarded: bool,
    deadline: Instant,
    begin: Snapshot,
    directory: PathBuf,
    shared: Arc<Shared>,
}
impl Drop for Window {
    /// Runs after `end`, or alone when the window never got that far.
    fn drop(&mut self) {
        if let Ok(mut workers) = self.shared.workers.lock() {
            for worker in workers.iter_mut() {
                let _ = worker.child.kill();
                let _ = worker.child.wait();
            }
            workers.clear();
        }
        if let Ok(mut active) = self.shared.active.lock() {
            *active = None;
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Telemetry for the whole run plus one pressure window per warm case.
pub(crate) struct Pressure {
    pub(crate) settings: Settings,
    /// The worker command before its arguments: the bench and `pressure-worker`.
    worker: Vec<OsString>,
    directory: PathBuf,
    cgroup: Option<PathBuf>,
    warmup: Duration,
    shared: Arc<Shared>,
    sampler: Option<thread::JoinHandle<()>>,
    pub(crate) cases: Vec<Value>,
}
impl Pressure {
    /// Starts the 1 Hz sampler, writing `out/telemetry.tsv`.
    pub(crate) fn start(settings: &Settings, out: &Path, bench: &Path) -> Result<Self> {
        Self::open(settings, out, bench, own_cgroup())
    }

    /// `start` with the cgroup whose memory and OOM counters the guards read.
    fn open(
        settings: &Settings,
        out: &Path,
        bench: &Path,
        cgroup: Option<PathBuf>,
    ) -> Result<Self> {
        let directory = out.join("pressure");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(failure)?;
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        preflight(
            settings,
            &directory,
            cgroup.as_deref(),
            keyed(&meminfo, "MemAvailable"),
        )?;
        let mut file = File::create(out.join("telemetry.tsv")).map_err(failure)?;
        writeln!(file, "{COLUMNS}").map_err(failure)?;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            file: Mutex::new(file),
            active: Mutex::new(None),
            abort: Mutex::new(None),
            owned: Mutex::new(Vec::new()),
            workers: Mutex::new(Vec::new()),
        });
        let sampler = {
            let (shared, settings, cgroup, directory) = (
                shared.clone(),
                settings.clone(),
                cgroup.clone(),
                directory.clone(),
            );
            thread::Builder::new()
                .name("telemetry".into())
                .spawn(move || sample(&shared, &settings, cgroup.as_deref(), &directory))
                .map_err(failure)?
        };
        Ok(Self {
            settings: settings.clone(),
            worker: vec![bench.into(), "pressure-worker".into()],
            directory,
            cgroup,
            warmup: WARMUP,
            shared,
            sampler: Some(sampler),
            cases: Vec::new(),
        })
    }

    /// The processes to sample: the driver's owned PIDs with their start times.
    pub(crate) fn track(&self, known: &BTreeMap<u32, String>) {
        if let Ok(mut owned) = self.shared.owned.lock() {
            *owned = known.iter().map(|(&pid, id)| (pid, id.clone())).collect();
        }
    }

    fn snapshot(&self) -> Snapshot {
        let owned = self
            .shared
            .owned
            .lock()
            .map(|o| o.clone())
            .unwrap_or_default();
        Snapshot::read(self.cgroup.as_deref(), &self.directory, &owned)
    }

    /// The workers the profile runs, with each one's amount.
    fn workers(&self) -> Vec<(&'static str, u64)> {
        match self.settings.profile {
            Profile::None => Vec::new(),
            Profile::CpuHalf | Profile::CpuAll => {
                let cpus = environment::cpus_allowed()
                    .map(|cpus| cpus.len() as u64)
                    .unwrap_or_else(environment::cpus_online);
                let threads = if self.settings.profile == Profile::CpuAll {
                    cpus
                } else {
                    (cpus / 2).max(1)
                };
                vec![("cpu", threads)]
            }
            Profile::Disk => (0..self.settings.workers)
                .flat_map(|_| {
                    [
                        ("sequential", SEQUENTIAL_FILE),
                        ("metadata", METADATA_FILE as u64),
                    ]
                })
                .collect(),
            Profile::Memory => vec![("memory", self.settings.memory_mib)],
        }
    }

    /// Opens `case`'s window: workers start and warm up before its first sample,
    /// unless the host is already unsafe for them. Callers `check` before each
    /// sample and always `end` the window.
    pub(crate) fn begin(&mut self, case: &str) -> Result<Window> {
        if let Some(reason) = self.shared.aborted()? {
            return Err(format!("pressure was aborted earlier: {reason}"));
        }
        let directory = self.directory.join(case);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(failure)?;
        let workers = self.workers();
        let mut window = Window {
            case: case.to_owned(),
            guarded: !workers.is_empty(),
            // Set again when the workers start.
            deadline: Instant::now(),
            begin: self.snapshot(),
            directory,
            shared: self.shared.clone(),
        };
        self.shared.write(&window.begin.row("begin", case))?;
        if window.guarded {
            preflight(
                &self.settings,
                &self.directory,
                self.cgroup.as_deref(),
                window.begin.memory[0],
            )?;
            if let Some(reason) = guard(&self.settings, &window.begin, window.begin.cgroup[2]) {
                return Err(format!("{case} cannot start under pressure: {reason}"));
            }
        }
        let mut peaks = Peaks::default();
        peaks.add(&window.begin);
        *self.shared.active.lock().map_err(failure)? = Some(Active {
            case: case.to_owned(),
            oom: window.begin.cgroup[2],
            guarded: window.guarded,
            peaks,
        });
        window.deadline = Instant::now() + Duration::from_secs(self.settings.seconds);
        let deadline_ns = monotonic_ns() + self.settings.seconds * 1_000_000_000;
        for (index, (kind, amount)) in workers.into_iter().enumerate() {
            let report = window.directory.join(format!("{kind}-{index}.json"));
            let child = spawn(
                &self.worker,
                kind,
                deadline_ns,
                amount,
                &window.directory,
                &report,
            )?;
            self.shared.workers.lock().map_err(failure)?.push(Worker {
                kind,
                child,
                report,
                status: None,
            });
        }
        if window.guarded {
            let warm = Instant::now() + self.warmup;
            while Instant::now() < warm && self.shared.aborted()?.is_none() {
                thread::sleep(Duration::from_millis(50));
            }
        }
        self.check(&window)?;
        Ok(window)
    }

    /// Fails when pressure no longer holds: a guard tripped, a worker ended or the window closed.
    pub(crate) fn check(&self, window: &Window) -> Result {
        if let Some(reason) = self.shared.aborted()? {
            return Err(format!("pressure aborted during {}: {reason}", window.case));
        }
        if !window.guarded {
            return Ok(());
        }
        if Instant::now() >= window.deadline {
            return Err(format!(
                "the {} s pressure window ended before {} finished its samples",
                self.settings.seconds, window.case
            ));
        }
        for worker in self.shared.workers.lock().map_err(failure)?.iter_mut() {
            if let Some(status) = worker.poll()? {
                return Err(format!(
                    "{} pressure worker ended early: {status}",
                    worker.kind
                ));
            }
        }
        Ok(())
    }

    /// Closes the window: stops its workers and records what pressure the case
    /// attained. Fails when pressure did not hold through the last sample.
    pub(crate) fn end(&mut self, window: Window) -> Result {
        let mut problems = Vec::new();
        if window.guarded && Instant::now() >= window.deadline {
            problems.push(format!(
                "the {} s pressure window ended before {} finished its samples",
                self.settings.seconds, window.case
            ));
        }
        let mut workers = std::mem::take(&mut *self.shared.workers.lock().map_err(failure)?);
        let mut signalled = Vec::new();
        for worker in &mut workers {
            match worker.poll() {
                Ok(None) => {
                    // Unreaped, and only this thread reaps it now: the PID is still ours.
                    // SAFETY: kill has no memory arguments.
                    unsafe { libc::kill(worker.child.id() as i32, libc::SIGTERM) };
                    signalled.push(true);
                }
                Ok(Some(status)) => {
                    problems.push(format!(
                        "{} pressure worker ended before the window closed: {status}",
                        worker.kind
                    ));
                    signalled.push(false);
                }
                Err(error) => {
                    problems.push(error);
                    signalled.push(false);
                }
            }
        }
        let grace = Instant::now() + STOP_GRACE;
        let mut reports = Vec::new();
        for (worker, signalled) in workers.iter_mut().zip(signalled) {
            let status = loop {
                match worker.poll() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) if Instant::now() < grace => thread::sleep(Duration::from_millis(20)),
                    _ => break None,
                }
            };
            let report = fs::read_to_string(&worker.report)
                .ok()
                .and_then(|text| Value::parse(&text).ok());
            match status {
                None => {
                    let _ = worker.child.kill();
                    let _ = worker.child.wait();
                    problems.push(format!(
                        "{} pressure worker did not stop within 2 s",
                        worker.kind
                    ));
                }
                Some(status) if signalled && !(status.success() && report.is_some()) => {
                    problems.push(format!("{} pressure worker failed: {status}", worker.kind));
                }
                Some(_) => {}
            }
            reports.push(report.unwrap_or(Value::Null));
        }
        let active = self.shared.active.lock().map_err(failure)?.take();
        let end = self.snapshot();
        if let Err(error) = self.shared.write(&end.row("end", &window.case)) {
            problems.push(error);
        }
        let mut peaks = active.map(|active| active.peaks).unwrap_or_default();
        peaks.add(&end);
        let aborted = self.shared.aborted()?;
        if let Some(reason) = &aborted {
            problems.insert(
                0,
                format!("pressure aborted during {}: {reason}", window.case),
            );
        }
        self.cases.push(attainment(
            &self.settings,
            &window,
            &end,
            &peaks,
            reports,
            aborted,
        ));
        if let Err(error) = fs::remove_dir_all(&window.directory) {
            problems.push(format!("{}: {error}", window.directory.display()));
        }
        match problems.is_empty() {
            true => Ok(()),
            false => Err(problems.join("; ")),
        }
    }

    /// Stops the sampler; the file keeps every row written so far. Fails when
    /// pressure aborted or telemetry was lost at any point, after the last
    /// window's checks too.
    pub(crate) fn finish(&mut self) -> Result {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(sampler) = self.sampler.take() {
            sampler
                .join()
                .map_err(|_| "the telemetry thread panicked".to_owned())?;
        }
        let _ = fs::remove_dir(&self.directory);
        match self.shared.aborted()? {
            Some(reason) => Err(format!("pressure telemetry: {reason}")),
            None => Ok(()),
        }
    }
}
impl Drop for Pressure {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Starts one `pressure-worker` (`worker` is the bench and `pressure-worker`),
/// with the parent-death signal every worker gets: if the driver dies, the
/// kernel kills its workers.
pub fn spawn(
    worker: &[OsString],
    kind: &str,
    deadline_ns: u64,
    amount: u64,
    directory: &Path,
    report: &Path,
) -> Result<Child> {
    let mut command = Command::new(&worker[0]);
    command
        .args(&worker[1..])
        .args([kind, &deadline_ns.to_string(), &amount.to_string()])
        .arg(directory)
        .arg(report);
    die_with_parent(&mut command).spawn().map_err(failure)
}

/// Every second: one telemetry row, and while workers run, the guards. A tripped
/// guard or a lost row stops pressure at once, whatever the driver is doing.
fn sample(shared: &Shared, settings: &Settings, cgroup: Option<&Path>, directory: &Path) {
    let mut last = Instant::now();
    while !shared.stop.load(Ordering::SeqCst) {
        let owned = shared.owned.lock().map(|o| o.clone()).unwrap_or_default();
        let snapshot = Snapshot::read(cgroup, directory, &owned);
        let lag = last.elapsed();
        last = Instant::now();
        let mut reason = None;
        let label = match shared.active.lock().as_deref_mut() {
            Ok(Some(active)) => {
                active.peaks.add(&snapshot);
                if active.guarded {
                    reason = guard(settings, &snapshot, active.oom).or_else(|| {
                        (lag > SAMPLE_PERIOD + SAMPLER_LAG)
                            .then(|| format!("telemetry fell {} ms behind", lag.as_millis()))
                    });
                }
                active.case.clone()
            }
            _ => String::new(),
        };
        // Kill first: a write that blocks must not delay a decided abort.
        let tripped = reason.map(|reason| {
            let row = snapshot.row("abort", &reason);
            (shared.trip(reason), row)
        });
        let written = shared.write(&snapshot.row("sample", &label));
        if let Some((true, row)) = tripped {
            let _ = shared.write(&row);
        }
        if let Err(error) = written {
            let row = snapshot.row("abort", &error);
            if shared.trip(error) {
                let _ = shared.write(&row);
            }
        }
        let wake = last + SAMPLE_PERIOD;
        while !shared.stop.load(Ordering::SeqCst) && Instant::now() < wake {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn attainment(
    settings: &Settings,
    window: &Window,
    end: &Snapshot,
    peaks: &Peaks,
    workers: Vec<Value>,
    aborted: Option<String>,
) -> Value {
    let begin = &window.begin;
    let duration_ns = end.at.saturating_sub(begin.at);
    let delta = |a: Option<u64>, b: Option<u64>| -> Value {
        b.zip(a).and_then(|(b, a)| b.checked_sub(a)).into()
    };
    let psi_names = [
        "cpu_some",
        "io_some",
        "io_full",
        "memory_some",
        "memory_full",
    ];
    let stall = |i: usize| -> Value {
        delta(begin.psi[i], end.psi[i])
            .as_u64()
            .map(|us| us / 1000)
            .into()
    };
    let share = |i: usize| -> Value {
        match (delta(begin.psi[i], end.psi[i]).as_u64(), duration_ns) {
            (Some(us), ns) if ns > 0 => (us as f64 * 1000.0 / ns as f64).into(),
            _ => Value::Null,
        }
    };
    // Per process from its begin (or first sight) to its last sample, so one that
    // exits or shares its name with a newer one keeps its own delta.
    let deltas = peaks.last.iter().map(|(key, (name, last))| {
        let before = begin
            .processes
            .get(key)
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        let delta = Process {
            count: 1,
            cpu_ticks: last.cpu_ticks.saturating_sub(before.cpu_ticks),
            major_faults: last.major_faults.saturating_sub(before.major_faults),
            rss_kb: 0,
            swap_kb: 0,
            write_bytes: last.write_bytes.saturating_sub(before.write_bytes),
        };
        (name.clone(), delta)
    });
    let owned = named(&deltas.collect::<Vec<_>>())
        .into_iter()
        .map(|(name, delta)| {
            let (rss, swap) = peaks.owned.get(&name).copied().unwrap_or_default();
            (
                name,
                object([
                    ("processes", delta.count.into()),
                    ("cpu_ticks", delta.cpu_ticks.into()),
                    ("major_faults", delta.major_faults.into()),
                    ("peak_rss_kb", rss.into()),
                    ("peak_swap_kb", swap.into()),
                    ("write_bytes", delta.write_bytes.into()),
                ]),
            )
        })
        .collect();
    object([
        ("case", window.case.as_str().into()),
        ("profile", settings.profile.name().into()),
        ("begin_ns", begin.at.to_string().into()),
        ("end_ns", end.at.to_string().into()),
        ("duration_ms", (duration_ns / 1_000_000).into()),
        (
            "psi_stall_ms",
            Value::Object(
                psi_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| ((*name).to_owned(), stall(i)))
                    .collect(),
            ),
        ),
        (
            "psi_share",
            Value::Object(
                psi_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| ((*name).to_owned(), share(i)))
                    .collect(),
            ),
        ),
        ("swap_in_pages", delta(begin.vm[0], end.vm[0])),
        ("swap_out_pages", delta(begin.vm[1], end.vm[1])),
        ("major_faults", delta(begin.vm[2], end.vm[2])),
        ("peak_dirty_kb", peaks.dirty_kb.into()),
        ("peak_writeback_kb", peaks.writeback_kb.into()),
        ("min_available_kb", peaks.min_available_kb.into()),
        ("cgroup_oom_events", delta(begin.cgroup[2], end.cgroup[2])),
        ("owned", Value::Object(owned)),
        ("workers", Value::Array(workers)),
        ("aborted", aborted.into()),
    ])
}

static STOP: AtomicBool = AtomicBool::new(false);

/// `eon-bench pressure-worker KIND DEADLINE_NS AMOUNT DIR REPORT`: one bounded
/// load until SIGTERM or the monotonic deadline, then a JSON report of its work.
pub fn worker(arguments: &[String]) -> Result {
    let [kind, deadline, amount, directory, report] = arguments else {
        return Err("pressure-worker needs kind, deadline, amount, directory and report".into());
    };
    let deadline: u64 = deadline.parse().map_err(failure)?;
    let amount: u64 = amount.parse().map_err(failure)?;
    let directory = Path::new(directory);
    extern "C" fn stop(_: libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }
    // SAFETY: the handler only stores an atomic, which is async-signal-safe.
    unsafe { libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t) };
    let running = || !STOP.load(Ordering::SeqCst) && monotonic_ns() < deadline;
    let (work, unit) = match kind.as_str() {
        "cpu" => (cpu(amount, &running), "spin batches"),
        "sequential" => (sequential(directory, amount, &running)?, "bytes written"),
        "metadata" => (metadata(directory, amount, &running)?, "fsynced renames"),
        "memory" => (memory(amount, &running), "pages touched"),
        _ => return Err(format!("unknown pressure worker {kind}")),
    };
    // SAFETY: rusage is plain data that getrusage fills.
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    // SAFETY: usage is a valid rusage.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
    let cpu_ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    let value = object([
        ("kind", kind.as_str().into()),
        ("amount", amount.into()),
        ("work", work.into()),
        ("unit", unit.into()),
        (
            "cpu_ms",
            (cpu_ms(usage.ru_utime) + cpu_ms(usage.ru_stime)).into(),
        ),
        ("major_faults", (usage.ru_majflt as u64).into()),
        ("stopped_ns", monotonic_ns().to_string().into()),
    ]);
    fs::write(report, value.to_pretty()).map_err(failure)
}

fn cpu(threads: u64, running: &(impl Fn() -> bool + Sync)) -> u64 {
    thread::scope(|scope| {
        let spinners: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut batches = 0u64;
                    let mut x = 1u64;
                    while running() {
                        for _ in 0..100_000 {
                            x = std::hint::black_box(x.wrapping_mul(6_364_136_223_846_793_005) + 1);
                        }
                        batches += 1;
                    }
                    batches
                })
            })
            .collect();
        spinners.into_iter().map(|s| s.join().unwrap_or(0)).sum()
    })
}

/// Buffered writes that rewrite one file of at most `cap` bytes, leaving the
/// kernel's writeback to drain them. The file never has a name (O_TMPFILE), so
/// its blocks return when this process dies, however it dies.
fn sequential(directory: &Path, cap: u64, running: &impl Fn() -> bool) -> Result<u64> {
    let chunk: Vec<u8> = (0..MIB).map(|i| (i % 251) as u8).collect();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_TMPFILE)
        .open(directory)
        .map_err(|error| format!("{}: unnamed file: {error}", directory.display()))?;
    let (mut written, mut offset) = (0u64, 0u64);
    while running() {
        if offset + MIB > cap {
            file.set_len(0).map_err(failure)?;
            file.seek(SeekFrom::Start(0)).map_err(failure)?;
            offset = 0;
        }
        file.write_all(&chunk).map_err(failure)?;
        offset += MIB;
        written += MIB;
    }
    Ok(written)
}

/// Journal pressure: create, write, fsync, rename, fsync the directory, unlink.
/// A worker killed mid-loop can leave one 4 KiB file in its window's directory.
fn metadata(directory: &Path, size: u64, running: &impl Fn() -> bool) -> Result<u64> {
    let block = vec![0x5a; size as usize];
    let parent = File::open(directory).map_err(failure)?;
    let (temporary, done) = (
        directory.join(format!("metadata-{}.tmp", std::process::id())),
        directory.join(format!("metadata-{}", std::process::id())),
    );
    let mut operations = 0;
    let result = (|| {
        while running() {
            let mut file = File::create(&temporary).map_err(failure)?;
            file.write_all(&block).map_err(failure)?;
            file.sync_all().map_err(failure)?;
            fs::rename(&temporary, &done).map_err(failure)?;
            parent.sync_all().map_err(failure)?;
            fs::remove_file(&done).map_err(failure)?;
            operations += 1;
        }
        Ok(operations)
    })();
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(&done);
    result
}

/// Keeps `mib` of anonymous pages resident by rewriting one byte per page.
fn memory(mib: u64, running: &impl Fn() -> bool) -> u64 {
    let mut pages = vec![0u8; (mib * MIB) as usize];
    let mut touched = 0;
    'outer: loop {
        for chunk in pages.chunks_mut(4096 * 256) {
            if !running() {
                break 'outer;
            }
            for page in chunk.chunks_mut(4096) {
                page[0] = page[0].wrapping_add(1);
                touched += 1;
            }
        }
    }
    std::hint::black_box(&pages);
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&'static str, &'static str)]) -> Result<Settings> {
        Settings::parse(&pairs.iter().copied().collect())
    }

    #[test]
    fn settings_reject_bounds_beyond_the_approved_envelope() {
        // A cap that silently grows would let a matrix run exceed what was approved.
        assert_eq!(settings(&[]).unwrap().profile, Profile::None);
        assert!(settings(&[("--pressure", "cpu-all"), ("--pressure-seconds", "121")]).is_err());
        assert!(settings(&[("--pressure", "cpu-all"), ("--pressure-seconds", "9")]).is_err());
        assert!(settings(&[("--pressure", "disk"), ("--pressure-workers", "3")]).is_err());
        assert!(settings(&[("--pressure", "memory")]).is_err());
        assert!(settings(&[("--pressure", "cpu-half"), ("--pressure-workers", "2")]).is_err());
        assert!(settings(&[("--pressure-seconds", "30")]).is_err());
        assert!(settings(&[("--pressure", "fork-bomb")]).is_err());
        let disk = settings(&[("--pressure", "disk"), ("--pressure-workers", "2")]).unwrap();
        assert!(disk.allocated() <= MAX_ALLOCATED);
        assert_eq!(disk.total(), Duration::from_secs(62) * WARM_CASES);
    }

    #[test]
    fn procfs_and_cgroup_parsers_read_the_kernel_formats() {
        let pressure = "some avg10=1.00 avg60=0.50 avg300=0.10 total=123456\n\
                        full avg10=0.00 avg60=0.00 avg300=0.00 total=789\n";
        assert_eq!(psi(pressure), (Some(123456), Some(789)));
        assert_eq!(
            psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=5\n"),
            (Some(5), None)
        );
        assert_eq!(keyed("pswpin 10\npswpout 20\n", "pswpout"), Some(20));
        assert_eq!(
            keyed("MemAvailable:   1024 kB\nDirty: 7 kB\n", "Dirty"),
            Some(7)
        );
        assert_eq!(keyed("low 0\noom 2\noom_kill 1\n", "oom"), Some(2));
        assert_eq!(limit("max\n"), None);
        assert_eq!(limit("8589934592\n"), Some(8 << 30));
        assert_eq!(
            cgroup_path("0::/user.slice/run-1.scope\n"),
            Some(PathBuf::from("/sys/fs/cgroup/user.slice/run-1.scope"))
        );
    }

    #[test]
    fn guards_trip_on_low_memory_low_disk_new_oom_events_and_lost_counters() {
        let disk = settings(&[("--pressure", "disk")]).unwrap();
        let mut now = Snapshot {
            memory: [Some(8 << 20), None, None, None],
            free: Some(20 * GIB),
            cgroup: [None, None, Some(0)],
            ..Snapshot::default()
        };
        assert_eq!(guard(&disk, &now, Some(0)), None);
        now.memory[0] = Some(6 << 20);
        assert!(
            guard(&disk, &now, Some(0))
                .unwrap()
                .contains("MemAvailable")
        );
        now.memory[0] = Some(8 << 20);
        now.free = Some(9 * GIB);
        assert!(guard(&disk, &now, Some(0)).unwrap().contains("free"));
        now.free = Some(20 * GIB);
        now.cgroup[2] = Some(1);
        assert!(guard(&disk, &now, Some(0)).unwrap().contains("OOM"));
        // A counter a guard cannot read stops pressure instead of leaving it unwatched.
        now.cgroup[2] = None;
        assert!(
            guard(&disk, &now, Some(0))
                .unwrap()
                .contains("memory.events")
        );
        now.cgroup[2] = Some(0);
        now.free = None;
        assert!(guard(&disk, &now, Some(0)).unwrap().contains("free space"));
        now.free = Some(20 * GIB);
        now.memory[0] = None;
        assert!(
            guard(&disk, &now, Some(0))
                .unwrap()
                .contains("MemAvailable")
        );
        let memory =
            settings(&[("--pressure", "memory"), ("--pressure-memory-mib", "64")]).unwrap();
        now.memory[0] = Some(8 << 20);
        assert!(
            guard(&memory, &now, None)
                .unwrap()
                .contains("memory.events")
        );
    }

    #[test]
    fn memory_refuses_without_a_containing_cgroup_limit_or_headroom() {
        // A hog outside a bounded cgroup, or one that fills it or the host, could
        // push the shared machine into swap or the OOM killer.
        let memory =
            settings(&[("--pressure", "memory"), ("--pressure-memory-mib", "64")]).unwrap();
        let root = scratch("cgroup");
        let plenty = Some(64 << 20);
        assert!(preflight(&memory, &root, None, plenty).is_err());
        fs::write(root.join("memory.max"), "max\n").unwrap();
        fs::write(root.join("memory.swap.max"), "max\n").unwrap();
        fs::write(root.join("memory.current"), format!("{}\n", 128 * MIB)).unwrap();
        assert!(preflight(&memory, &root, Some(&root), plenty).is_err());
        fs::write(root.join("memory.max"), "33554432\n").unwrap();
        fs::write(root.join("memory.swap.max"), "0\n").unwrap();
        assert!(preflight(&memory, &root, Some(&root), plenty).is_err());
        fs::write(root.join("memory.max"), "1073741824\n").unwrap();
        assert_eq!(preflight(&memory, &root, Some(&root), plenty), Ok(()));
        // What the driver and Eon already hold counts against the limit.
        fs::write(root.join("memory.current"), format!("{}\n", 600 * MIB)).unwrap();
        assert!(
            preflight(&memory, &root, Some(&root), plenty)
                .unwrap_err()
                .contains("already uses")
        );
        fs::write(root.join("memory.current"), format!("{}\n", 128 * MIB)).unwrap();
        // The hog may not bring the host within the headroom of the guard.
        let host = |mib: u64| Some(mib * 1024);
        assert!(
            preflight(&memory, &root, Some(&root), host(6144 + 512 + 63))
                .unwrap_err()
                .contains("guard")
        );
        assert_eq!(
            preflight(&memory, &root, Some(&root), host(6144 + 512 + 64)),
            Ok(())
        );
        assert!(preflight(&memory, &root, Some(&root), None).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let out = std::env::temp_dir().join(format!("eon-bench-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out);
        fs::create_dir_all(&out).unwrap();
        out
    }

    /// A controller whose workers run `script` in sh with the worker's arguments
    /// ($1 kind, $4 window directory, $5 report), without the 5 s warm-up.
    fn controller(name: &str, script: &str, guard_mib: &'static str) -> (PathBuf, Pressure) {
        controller_in(name, script, guard_mib, own_cgroup())
    }

    fn controller_in(
        name: &str,
        script: &str,
        guard_mib: &'static str,
        cgroup: Option<PathBuf>,
    ) -> (PathBuf, Pressure) {
        let out = scratch(name);
        let cpu = settings(&[
            ("--pressure", "cpu-half"),
            ("--pressure-seconds", "10"),
            ("--guard-available-mib", guard_mib),
        ])
        .unwrap();
        let mut pressure = Pressure::open(&cpu, &out, Path::new("/nonexistent"), cgroup).unwrap();
        let sh = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("sh"))
            .find(|sh| sh.exists())
            .unwrap();
        pressure.worker = vec![sh.into(), "-c".into(), script.into(), "worker".into()];
        pressure.warmup = Duration::ZERO;
        (out, pressure)
    }

    fn running(pressure: &Pressure) -> usize {
        let mut workers = pressure.shared.workers.lock().unwrap();
        workers
            .iter_mut()
            .map(|worker| worker.poll().unwrap())
            .filter(Option::is_none)
            .count()
    }

    #[test]
    fn lost_telemetry_kills_the_workers_at_once_and_fails_the_run() {
        // The sampler stops pressure itself: a driver stuck in a long measurement
        // must not keep a hog running past a tripped guard.
        let (out, mut pressure) = controller("trip", "exec sleep 60", "0");
        let window = pressure.begin("typing").unwrap();
        assert_eq!(running(&pressure), 1);
        *pressure.shared.file.lock().unwrap() = fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let tripped = Instant::now();
        while running(&pressure) > 0 {
            assert!(
                tripped.elapsed() < Duration::from_secs(3),
                "worker outlived the trip"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let checked = pressure.check(&window).unwrap_err();
        assert!(checked.contains("telemetry write failed"), "{checked}");
        let ended = pressure.end(window).unwrap_err();
        assert!(
            ended.starts_with("pressure aborted during typing: telemetry write failed"),
            "{ended}"
        );
        let [case] = pressure.cases.as_slice() else {
            panic!("{:?}", pressure.cases);
        };
        assert!(case.get("aborted").and_then(Value::as_str).is_some());
        // The abort fails the run: no later case starts.
        assert!(
            pressure
                .begin("tab-key")
                .map(|_| ())
                .unwrap_err()
                .contains("aborted")
        );
        assert!(pressure.finish().is_err());
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn a_new_oom_event_kills_the_workers_at_once_and_fails_the_run() {
        // A resource guard tripping mid-window, through the sampler, not only guard(),
        // while every telemetry write blocks: the kill must not wait for the write.
        let cgroup = scratch("oom-cgroup");
        fs::write(cgroup.join("memory.events"), "oom 0\noom_kill 0\n").unwrap();
        let (out, mut pressure) = controller_in("oom", "exec sleep 60", "0", Some(cgroup.clone()));
        let window = pressure.begin("typing").unwrap();
        assert_eq!(running(&pressure), 1);
        // Right after a sample row lands, the next sample is about 1 s away: block
        // its writes (the file lock is the telemetry gate) and add the OOM event.
        let rows = || {
            fs::read_to_string(out.join("telemetry.tsv"))
                .unwrap()
                .lines()
                .count()
        };
        let written = rows();
        while rows() == written {
            thread::sleep(Duration::from_millis(5));
        }
        let gate = pressure.shared.file.lock().unwrap();
        fs::write(cgroup.join("memory.events"), "oom 0\noom_kill 1\n").unwrap();
        let tripped = Instant::now();
        while running(&pressure) > 0 {
            assert!(
                tripped.elapsed() < Duration::from_secs(3),
                "worker outlived the trip while telemetry was blocked"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let blocked = rows();
        drop(gate);
        let ended = pressure.end(window).unwrap_err();
        assert!(
            ended.starts_with("pressure aborted during typing: 1 new OOM events"),
            "{ended}"
        );
        let finished = pressure.finish().unwrap_err();
        assert!(finished.contains("OOM"), "{finished}");
        // The blocked rows were written once the gate opened, the abort among them.
        let telemetry = fs::read_to_string(out.join("telemetry.tsv")).unwrap();
        assert!(telemetry.lines().count() > blocked);
        assert!(
            telemetry
                .lines()
                .any(|row| row.split('\t').nth(1) == Some("abort"))
        );
        fs::remove_dir_all(&out).unwrap();
        fs::remove_dir_all(&cgroup).unwrap();
    }

    #[test]
    fn telemetry_lost_after_the_last_window_still_fails_the_run() {
        let out = scratch("late-loss");
        let none = settings(&[]).unwrap();
        let mut pressure = Pressure::start(&none, &out, Path::new("/nonexistent")).unwrap();
        let window = pressure.begin("typing").unwrap();
        pressure.end(window).unwrap();
        // The last window closed cleanly; the sampler then loses its file during shutdown.
        *pressure.shared.file.lock().unwrap() = fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let lost = Instant::now();
        while pressure.shared.aborted().unwrap().is_none() {
            assert!(
                lost.elapsed() < Duration::from_secs(3),
                "the sampler kept going"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let finished = pressure.finish().unwrap_err();
        assert!(finished.contains("telemetry write failed"), "{finished}");
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn unsafe_starting_conditions_start_no_worker() {
        let marker = scratch("unsafe-marker").join("started");
        let script = format!("touch '{}'; exec sleep 60", marker.display());
        // No host has the 1 TiB the guard demands, so MemAvailable is always below it.
        let (out, mut pressure) = controller("unsafe", &script, "1048576");
        let refused = pressure.begin("typing").map(|_| ()).unwrap_err();
        assert!(refused.contains("cannot start under pressure"), "{refused}");
        assert!(refused.contains("MemAvailable"), "{refused}");
        thread::sleep(Duration::from_millis(100));
        assert!(!marker.exists());
        assert!(pressure.shared.workers.lock().unwrap().is_empty());
        assert!(pressure.shared.active.lock().unwrap().is_none());
        pressure.finish().unwrap();
        fs::remove_dir_all(&out).unwrap();
        fs::remove_dir_all(marker.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_worker_that_ends_early_fails_its_case_and_is_never_signalled() {
        let (out, mut pressure) = controller(
            "early",
            r#"while [ ! -e "$4/stop" ]; do sleep 0.05; done; exit 0"#,
            "0",
        );
        let window = pressure.begin("typing").unwrap();
        fs::write(window.directory.join("stop"), "").unwrap();
        let stopped = Instant::now();
        let error = loop {
            if let Err(error) = pressure.check(&window) {
                break error;
            }
            assert!(stopped.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        };
        assert!(error.contains("ended early"), "{error}");
        // check reaped it; the kept status is what stops end signalling its PID.
        assert!(pressure.shared.workers.lock().unwrap()[0].status.is_some());
        let ended = pressure.end(window).unwrap_err();
        assert!(ended.contains("ended before the window closed"), "{ended}");
        assert!(
            !ended.contains("did not stop") && !ended.contains("failed"),
            "{ended}"
        );
        pressure.finish().unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn a_case_that_outlasts_its_window_fails_after_measuring() {
        // Workers stop at the window's deadline, so samples taken after it ran unpressured.
        let (out, mut pressure) = controller(
            "overrun",
            r#"trap 'echo "{}" > "$5"; exit 0' TERM; : > "$4/ready"; while :; do sleep 0.05; done"#,
            "0",
        );
        let mut window = pressure.begin("typing").unwrap();
        let started = Instant::now();
        while !window.directory.join("ready").exists() {
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(pressure.check(&window), Ok(()));
        window.deadline = Instant::now();
        let ended = pressure.end(window).unwrap_err();
        assert!(
            ended.contains("window ended before typing finished"),
            "{ended}"
        );
        // The worker itself stopped cleanly on SIGTERM with its report.
        assert!(!ended.contains("worker"), "{ended}");
        assert_eq!(pressure.cases.len(), 1);
        pressure.finish().unwrap();
        fs::remove_dir_all(&out).unwrap();
    }

    fn start_time(pid: u32) -> String {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        stat.rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .to_owned()
    }

    #[test]
    fn every_case_gets_marked_telemetry_and_an_attainment_record() {
        // Without the markers and record, a matrix result could not say what pressure a case saw.
        let out = scratch("telemetry");
        let none = settings(&[]).unwrap();
        let mut pressure = Pressure::start(&none, &out, Path::new("/nonexistent")).unwrap();
        // Only a PID whose start time still matches is attributed (no PID-reuse
        // sampling), and a process that exits mid-case keeps its counters.
        let mut short = Command::new("sleep").arg("0.2").spawn().unwrap();
        let me = std::process::id();
        pressure.track(&BTreeMap::from([
            (me, start_time(me)),
            (short.id(), start_time(short.id())),
            (1, "0".to_owned()),
        ]));
        let window = pressure.begin("typing").unwrap();
        assert_eq!(pressure.check(&window), Ok(()));
        short.wait().unwrap();
        pressure.end(window).unwrap();
        pressure.finish().unwrap();
        let telemetry = fs::read_to_string(out.join("telemetry.tsv")).unwrap();
        let mut rows = telemetry.lines();
        assert_eq!(rows.next(), Some(COLUMNS));
        let columns = COLUMNS.split('\t').count();
        let marker = |kind: &str| -> Vec<&str> {
            let row = telemetry
                .lines()
                .find(|row| row.split('\t').nth(1) == Some(kind))
                .unwrap();
            row.split('\t').collect()
        };
        for kind in ["begin", "end"] {
            let row = marker(kind);
            assert_eq!(row.len(), columns, "{row:?}");
            assert_eq!(row[2], "typing");
        }
        let [case] = pressure.cases.as_slice() else {
            panic!("{:?}", pressure.cases);
        };
        assert_eq!(case.get("case").and_then(Value::as_str), Some("typing"));
        assert_eq!(case.get("workers").and_then(Value::as_array), Some(&[][..]));
        assert!(
            case.get("psi_stall_ms")
                .and_then(|p| p.get("cpu_some"))
                .is_some()
        );
        assert_eq!(case.get("aborted"), Some(&Value::Null));
        let owned = case.get("owned").and_then(Value::as_object).unwrap();
        assert_eq!(owned.len(), 2, "{owned:?}");
        for (name, process) in owned {
            assert_eq!(
                process.get("processes").and_then(Value::as_u64),
                Some(1),
                "{name}"
            );
        }
        assert!(owned.iter().any(|(name, _)| name == "sleep"), "{owned:?}");
        // Peaks and minima include the begin snapshot, with or without pressure.
        let begin = marker("begin");
        let number = |text: &str| text.parse::<u64>().unwrap();
        let field = |key: &str| case.get(key).and_then(Value::as_u64).unwrap();
        assert!(field("peak_dirty_kb") >= number(begin[12]));
        assert!(field("peak_writeback_kb") >= number(begin[13]));
        assert!(field("min_available_kb") <= number(begin[11]));
        assert!(!out.join("pressure").exists());
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn workers_stop_at_their_deadline_and_leave_no_files() {
        // A worker that outlives its window or leaves files behind would leak pressure.
        let directory =
            std::env::temp_dir().join(format!("eon-bench-workers-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let until = monotonic_ns() + 200_000_000;
        let running = || monotonic_ns() < until;
        let started = Instant::now();
        assert!(cpu(1, &running) > 0);
        assert!(started.elapsed() < Duration::from_secs(2));
        let until = monotonic_ns() + 200_000_000;
        let running = || monotonic_ns() < until;
        assert!(sequential(&directory, 2 * MIB, &running).unwrap() >= MIB);
        let until = monotonic_ns() + 200_000_000;
        let running = || monotonic_ns() < until;
        assert!(metadata(&directory, 4096, &running).unwrap() > 0);
        let until = monotonic_ns() + 100_000_000;
        let running = || monotonic_ns() < until;
        assert!(memory(1, &running) >= 256);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(&directory).unwrap();
    }
}
