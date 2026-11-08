//! Whole-machine CPU and memory use and one filesystem's use for the Eon Bar
//! (`docs/PRESENTATION.md`): on Linux from `/proc/stat`'s aggregate and per-CPU lines and
//! `/proc/meminfo`, on macOS from the Mach host statistics, and on both from `statvfs`.

use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The Symbols Nerd Font icons that lead the CPU, RAM and disk labels: `nf-oct-cpu`,
/// `nf-fa-memory` and `nf-fa-hard_drive`. Hover and accessibility name each reading in words.
pub(crate) const CPU_ICON: char = '\u{f4bc}';
pub(crate) const RAM_ICON: char = '\u{efc5}';
pub(crate) const DISK_ICON: char = '\u{f0a0}';

/// How loaded a CPU, RAM or disk chip shows the machine; only load beyond normal colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Load {
    Normal,
    Notice,
    High,
    Critical,
}

impl Load {
    /// Inclusive whole-percent levels; equal levels select the stronger grade.
    fn of(percent: u64, notice: u8, warning: u8, critical: u8) -> Self {
        if percent >= u64::from(critical) {
            Self::Critical
        } else if percent >= u64::from(warning) {
            Self::High
        } else if percent >= u64::from(notice) {
            Self::Notice
        } else {
            Self::Normal
        }
    }
}

/// When the CPU, RAM and disk chips colour, and the CPU hover's averaging window, as Eon's `[bar]`
/// configuration resolves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadLevels {
    pub ram_notice_percent: u8,
    pub ram_warning_percent: u8,
    pub ram_critical_percent: u8,
    pub cpu_notice_percent: u8,
    pub cpu_warning_percent: u8,
    pub cpu_critical_percent: u8,
    pub cpu_average: Duration,
    pub cpu_temperature_warning_celsius: u8,
    pub cpu_temperature_critical_celsius: u8,
    pub disk_notice_percent: u8,
    pub disk_warning_percent: u8,
    pub disk_critical_percent: u8,
}

impl Default for LoadLevels {
    fn default() -> Self {
        Self {
            ram_notice_percent: 60,
            ram_warning_percent: 75,
            ram_critical_percent: 90,
            cpu_notice_percent: 60,
            cpu_warning_percent: 75,
            cpu_critical_percent: 90,
            cpu_average: Duration::from_secs(30),
            cpu_temperature_warning_celsius: 85,
            cpu_temperature_critical_celsius: 95,
            disk_notice_percent: 60,
            disk_warning_percent: 75,
            disk_critical_percent: 90,
        }
    }
}

/// `part` of `whole` in whole percent, rounded to nearest; `whole` must be positive.
fn percent(part: u64, whole: u64) -> u64 {
    ((u128::from(part) * 200 + u128::from(whole)) / (u128::from(whole) * 2)) as u64
}

/// Cumulative whole-machine CPU time since boot, in kernel ticks, and the logical CPUs online.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    total: u64,
    cpus: usize,
    per_cpu: Option<Vec<(usize, [u64; 8])>>,
    name: Option<&'static str>,
    temperature: Option<u64>,
    load_average: Option<String>,
}

impl CpuTimes {
    /// Reads `/proc/stat` on Linux and the host CPU load on macOS; other platforms have no
    /// reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        static NAME: OnceLock<Option<String>> = OnceLock::new();
        static TEMPERATURE_INPUT: Mutex<Option<TemperatureInput>> = Mutex::new(None);
        let mut times = if cfg!(target_os = "linux") {
            Self::parse(&std::fs::read_to_string("/proc/stat").ok()?)?
        } else {
            let (ticks, cpus) = host::cpu_load()?;
            let mut times = Self::from_host_ticks(ticks, cpus);
            times.per_cpu = host::per_cpu_load().map(|per_cpu| {
                per_cpu
                    .into_iter()
                    .enumerate()
                    .map(|(id, [user, system, idle, nice])| {
                        (
                            id,
                            [
                                u64::from(user),
                                u64::from(nice),
                                u64::from(system),
                                u64::from(idle),
                                0,
                                0,
                                0,
                                0,
                            ],
                        )
                    })
                    .collect()
            });
            times
        };
        times.name = NAME
            .get_or_init(|| {
                if cfg!(target_os = "linux") {
                    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
                    info.lines()
                        .filter_map(|line| line.split_once(':'))
                        .find_map(|(key, value)| {
                            (key.trim() == "model name")
                                .then(|| clean_cpu_name(value))
                                .flatten()
                        })
                } else {
                    host::cpu_name().and_then(|name| clean_cpu_name(&name))
                }
            })
            .as_deref();
        times.temperature = cfg!(target_os = "linux")
            .then(|| {
                TEMPERATURE_INPUT.lock().ok().and_then(|mut input| {
                    cpu_temperature(Path::new("/sys/class/hwmon"), &mut input)
                })
            })
            .flatten();
        times.load_average = if cfg!(target_os = "linux") {
            std::fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|load| parse_load(&load))
        } else {
            host::load_average().and_then(format_load)
        };
        Some(times)
    }

    /// Takes Mach's whole-machine `HOST_CPU_LOAD_INFO` ticks, in its order user, system, idle,
    /// nice, and the logical CPUs online. Busy is user, system and nice. The counters are 32-bit
    /// and wrap, which makes the next interval go backwards and show nothing.
    fn from_host_ticks([user, system, idle, nice]: [u32; 4], cpus: usize) -> Self {
        let busy = u64::from(user) + u64::from(system) + u64::from(nice);
        Self {
            busy,
            total: busy + u64::from(idle),
            cpus,
            per_cpu: None,
            name: None,
            temperature: None,
            load_average: None,
        }
    }

    /// Parses the aggregate first line and counts the `cpuN` lines right after it, which list the
    /// logical CPUs online; a reading without them is malformed. Idle is idle plus iowait; total
    /// is the first eight fields, leaving out guest and guest_nice, which the kernel already
    /// counts in user and nice.
    pub(crate) fn parse(stat: &str) -> Option<Self> {
        let mut lines = stat.lines();
        let mut fields = lines.next()?.split_ascii_whitespace();
        if fields.next()? != "cpu" {
            return None;
        }
        let (busy, total) = cpu_totals(&linux_ticks(fields)?)?;
        let lines: Vec<_> = lines
            .take_while(|line| {
                line.strip_prefix("cpu")
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            })
            .collect();
        let cpus = lines.len();
        if cpus == 0 {
            return None;
        }
        let per_cpu = lines
            .into_iter()
            .map(|line| {
                let mut fields = line.split_ascii_whitespace();
                let id = fields.next()?.strip_prefix("cpu")?.parse().ok()?;
                let ticks = linux_ticks(fields)?;
                cpu_totals(&ticks)?;
                Some((id, ticks))
            })
            .collect::<Option<Vec<_>>>()
            .filter(|cpus| cpus.windows(2).all(|pair| pair[0].0 < pair[1].0));
        Some(Self {
            busy,
            total,
            cpus,
            per_cpu,
            name: None,
            temperature: None,
            load_average: None,
        })
    }

    fn busiest(&self, later: &Self) -> Option<(usize, u64)> {
        let (before, after) = (self.per_cpu.as_ref()?, later.per_cpu.as_ref()?);
        if before.len() != after.len() {
            return None;
        }
        let mut busiest = None;
        for (&(id, earlier), &(next_id, later)) in before.iter().zip(after) {
            if id != next_id {
                return None;
            }
            let mut deltas = [0; 8];
            // A field can go backwards even while its busy/total sums advance (e.g. iowait).
            for (delta, (before, after)) in deltas.iter_mut().zip(earlier.iter().zip(later)) {
                *delta = after.checked_sub(*before)?;
            }
            let (busy, total) = cpu_totals(&deltas)?;
            if total == 0 || busy > total {
                return None;
            }
            if busiest.is_none_or(|(_, highest_busy, highest_total)| {
                u128::from(busy) * u128::from(highest_total)
                    > u128::from(highest_busy) * u128::from(total)
            }) {
                busiest = Some((id, busy, total));
            }
        }
        busiest.map(|(id, busy, total)| (id, percent(busy, total)))
    }
}

fn linux_ticks(mut fields: std::str::SplitAsciiWhitespace<'_>) -> Option<[u64; 8]> {
    let mut ticks = [0_u64; 8];
    for tick in &mut ticks {
        *tick = fields.next()?.parse().ok()?;
    }
    Some(ticks)
}

fn cpu_totals(ticks: &[u64; 8]) -> Option<(u64, u64)> {
    let total = ticks
        .iter()
        .try_fold(0_u64, |sum, tick| sum.checked_add(*tick))?;
    Some((total - ticks[3] - ticks[4], total))
}

fn clean_cpu_name(name: &str) -> Option<String> {
    let name = name
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace("(C)", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!name.is_empty()).then_some(name)
}

fn parse_load(load: &str) -> Option<String> {
    format_load(load.split_ascii_whitespace().next()?.parse().ok()?)
}

fn format_load(load: f64) -> Option<String> {
    (load.is_finite() && load >= 0.0).then(|| format!("{load:.1}"))
}

struct TemperatureInput {
    path: PathBuf,
    driver: &'static str,
    label: &'static str,
}

impl TemperatureInput {
    fn read(&self) -> Option<u64> {
        let name = std::fs::read_to_string(self.path.parent()?.join("name")).ok()?;
        let stem = self.path.file_name()?.to_str()?.strip_suffix("_input")?;
        let label =
            std::fs::read_to_string(self.path.with_file_name(format!("{stem}_label"))).ok()?;
        if name.trim() != self.driver || label.trim() != self.label {
            return None;
        }
        read_temperature(&self.path)
    }
}

/// Only CPU package sensors: ACPI/board zones do not measure this CPU.
fn cpu_temperature(root: &Path, cached: &mut Option<TemperatureInput>) -> Option<u64> {
    if let Some(temperature) = cached.as_ref().and_then(TemperatureInput::read) {
        return Some(temperature);
    }
    *cached = None;
    let mut devices = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    devices.sort();
    for device in devices {
        let Ok(name) = std::fs::read_to_string(device.join("name")) else {
            continue;
        };
        let (driver, labels): (&str, &[&str]) = match name.trim() {
            "coretemp" => ("coretemp", &["Package id 0"]),
            "k10temp" => ("k10temp", &["Tctl", "Tdie"]),
            _ => continue,
        };
        let Ok(entries) = std::fs::read_dir(&device) else {
            continue;
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();
        for wanted in labels {
            for path in &paths {
                let Some(stem) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix("_label"))
                else {
                    continue;
                };
                if !stem
                    .strip_prefix("temp")
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                {
                    continue;
                }
                if std::fs::read_to_string(path)
                    .ok()
                    .is_none_or(|label| label.trim() != *wanted)
                {
                    continue;
                }
                let input = device.join(format!("{stem}_input"));
                if let Some(temperature) = read_temperature(&input) {
                    *cached = Some(TemperatureInput {
                        path: input,
                        driver,
                        label: wanted,
                    });
                    return Some(temperature);
                }
            }
        }
    }
    None
}

fn read_temperature(path: &Path) -> Option<u64> {
    let millidegrees = std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    (millidegrees <= 150_000).then(|| (millidegrees + 500) / 1000)
}

/// CPU use between two readings, as shown in the workspace header.
#[derive(Debug, PartialEq, Eq)]
pub struct CpuUse {
    pub(crate) percent: u64,
    pub(crate) label: String,
    /// The hover list: one fact per line, without the use the label shows.
    pub(crate) tooltip: String,
    /// The accessible value: every fact in one sentence.
    pub(crate) description: String,
    pub(crate) load: Load,
}

impl CpuUse {
    /// Whole-percent use between two readings. Counters that did not advance or went backwards
    /// give no value rather than a false 0% or 100%.
    #[must_use]
    pub(crate) fn between(earlier: &CpuTimes, later: &CpuTimes) -> Option<Self> {
        let total = later.total.checked_sub(earlier.total)?;
        let busy = later.busy.checked_sub(earlier.busy)?;
        if total == 0 || busy > total {
            return None;
        }
        let percent = percent(busy, total);
        let cpus = later.cpus;
        let plural = if cpus == 1 { "" } else { "s" };
        Some(Self {
            percent,
            label: format!("{CPU_ICON} {percent}%"),
            tooltip: format!(
                "CPU{}\n{cpus} logical CPU{plural}{}",
                later
                    .name
                    .map_or(String::new(), |name| format!(" · {name}")),
                later
                    .temperature
                    .map_or(String::new(), |temperature| format!(" · {temperature} °C"))
            ),
            description: format!(
                "CPU use {percent}% across {cpus} logical CPU{plural}, over the last 2 seconds{}{}",
                later.name.map_or(String::new(), |name| format!("; {name}")),
                later
                    .temperature
                    .map_or(String::new(), |temperature| format!("; {temperature} °C"))
            ),
            load: Load::Normal,
        })
    }
}

/// The CPU readings since the last break, back to the newest one at least the averaging window
/// old, so hover can give the average over the whole window beside the shown use. Each reading's
/// counters advance from the one before it, or the readings start over.
#[derive(Debug, Default)]
pub struct CpuReadings {
    readings: VecDeque<(Instant, CpuTimes)>,
}

impl CpuReadings {
    /// Records a reading taken at `now`, or a failed one, and returns the use since the previous
    /// reading, judged against the configured use levels and with the average over
    /// `levels.cpu_average` once the readings span it. A failed reading, one taken `gap` or more
    /// after the previous, or one that gives no use starts over, so every value compares readings
    /// about one interval apart.
    pub fn push(
        &mut self,
        now: Instant,
        times: Option<CpuTimes>,
        gap: Duration,
        levels: LoadLevels,
    ) -> Option<CpuUse> {
        let shown = self
            .readings
            .back()
            .filter(|(taken, _)| now.duration_since(*taken) < gap)
            .zip(times.as_ref())
            .and_then(|((_, earlier), later)| CpuUse::between(earlier, later));
        if shown.is_none() {
            self.readings.clear();
        }
        let times = times?;
        let busiest = self
            .readings
            .back()
            .and_then(|(_, earlier)| earlier.busiest(&times));
        if let Some((_, earlier)) = self.readings.back_mut() {
            earlier.per_cpu = None;
        }
        let temperature = times.temperature;
        let load_average = times.load_average.clone();
        self.readings.push_back((now, times));
        let window = levels.cpu_average;
        while self
            .readings
            .get(1)
            .is_some_and(|(taken, _)| now.duration_since(*taken) >= window)
        {
            self.readings.pop_front();
        }
        let mut shown = shown?;
        shown.load = Load::of(
            shown.percent,
            levels.cpu_notice_percent,
            levels.cpu_warning_percent,
            levels.cpu_critical_percent,
        );
        let (words, line) = match shown.load {
            Load::Normal => ("", ""),
            Load::Notice => (": elevated use", "\nElevated use"),
            Load::High => (": under high load", "\nUnder high load"),
            Load::Critical => (": under heavy load", "\nUnder heavy load"),
        };
        shown.description.push_str(words);
        if let Some(average) = self.average(now, window) {
            let seconds = window.as_secs();
            shown.description.push_str(&format!(
                "; {average}% on average over the last {seconds} seconds"
            ));
            shown.tooltip.push_str(&format!(
                "\n{average}% on average over the last {seconds} s"
            ));
        }
        if let Some((cpu, percent)) = busiest {
            shown
                .tooltip
                .push_str(&format!("\nCPU {cpu} at {percent}%"));
            shown
                .description
                .push_str(&format!("; CPU {cpu} at {percent}%"));
        }
        if let Some(load) = load_average {
            shown.tooltip.push_str(&format!("\nLoad {load} over 1 min"));
            shown
                .description
                .push_str(&format!("; Load {load} over 1 min"));
        }
        shown.tooltip.push_str(line);
        if let Some(temperature) = temperature {
            let thermal = Load::of(
                temperature,
                levels.cpu_temperature_warning_celsius,
                levels.cpu_temperature_warning_celsius,
                levels.cpu_temperature_critical_celsius,
            );
            if thermal != Load::Normal {
                shown.load = shown.load.max(thermal);
                shown.tooltip.push_str("\nRunning hot");
                shown.description.push_str("; running hot");
            }
        }
        Some(shown)
    }

    /// Whole-percent use over exactly the last `window`, once the readings span it. Only the share
    /// of the oldest interval inside the window counts, taking use as even within it.
    fn average(&self, now: Instant, window: Duration) -> Option<u64> {
        let ((first, earliest), (next, after)) = (self.readings.front()?, self.readings.get(1)?);
        let latest = &self.readings.back()?.1;
        // Pruning keeps `next` younger than the window, so the share is in [0, 1).
        let outside = (now.duration_since(*first).checked_sub(window)?).as_secs_f64()
            / next.duration_since(*first).as_secs_f64();
        let at_start =
            |from: u64, to: u64| from as f64 + (to.saturating_sub(from)) as f64 * outside;
        let busy = latest.busy as f64 - at_start(earliest.busy, after.busy);
        let total = latest.total as f64 - at_start(earliest.total, after.total);
        (total > 0.0).then(|| (busy / total * 100.0).round().clamp(0.0, 100.0) as u64)
    }
}

/// Memory use from one reading, as shown in the workspace header.
#[derive(Debug, PartialEq, Eq)]
pub struct RamUse {
    pub(crate) label: String,
    /// The hover list: one fact per line, without the percentage the label shows.
    pub(crate) tooltip: String,
    /// The accessible value: every fact in one sentence.
    pub(crate) description: String,
    pub(crate) load: Load,
    pub(crate) percent: u64,
}

impl RamUse {
    /// Reads `/proc/meminfo` on Linux and the host memory statistics on macOS; other platforms
    /// have no reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        if cfg!(target_os = "linux") {
            return Self::parse(&std::fs::read_to_string("/proc/meminfo").ok()?);
        }
        let (pages, page_size, memsize) = host::memory()?;
        Self::from_host_pages(pages, page_size, memsize)
    }

    /// Whole-percent use as btop computes it: MemTotal minus MemAvailable, over MemTotal. A
    /// reading without both fields, or with MemAvailable above MemTotal, gives no value rather
    /// than a guess or a false 0%.
    pub(crate) fn parse(meminfo: &str) -> Option<Self> {
        let field = |name: &str| -> Option<u64> {
            let line = meminfo
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))?;
            match line.split_ascii_whitespace().collect::<Vec<_>>()[..] {
                [kib, "kB"] => kib.parse().ok(),
                _ => None,
            }
        };
        let total = field("MemTotal")?;
        let used = total.checked_sub(field("MemAvailable")?)?;
        Self::of(used, total, f64::from(1 << 20))
    }

    /// Whole-percent use as Activity Monitor's Memory Used computes it from Mach's
    /// `HOST_VM_INFO64` page counts, internal, purgeable, wired and compressed: app memory
    /// (internal pages less purgeable ones), wired and compressed pages, over the physical memory
    /// in bytes. Purgeable pages above internal ones, or use above the physical memory, give no
    /// value; so does a zero page size, which would read as 0%.
    fn from_host_pages(
        [internal, purgeable, wired, compressed]: [u64; 4],
        page_size: u64,
        memsize: u64,
    ) -> Option<Self> {
        if page_size == 0 {
            return None;
        }
        let used = internal
            .checked_sub(purgeable)?
            .checked_add(wired)?
            .checked_add(compressed)?
            .checked_mul(page_size)?;
        Self::of(used, memsize, f64::from(1 << 30))
    }

    /// `used` of `total`, in units of which `per_gib` make a GiB; no value for an empty total or
    /// use above it.
    fn of(used: u64, total: u64, per_gib: f64) -> Option<Self> {
        if total == 0 || used > total {
            return None;
        }
        let percent = percent(used, total);
        let (used, total) = (used as f64 / per_gib, total as f64 / per_gib);
        Some(Self {
            label: format!("{RAM_ICON} {percent}%"),
            tooltip: format!("RAM\n{used:.1} GiB used of {total:.1} GiB"),
            description: format!("RAM use {percent}%, {used:.1} GiB of {total:.1} GiB"),
            load: Load::Normal,
            percent,
        })
    }

    /// This reading judged against the configured levels, with the pressure named in words.
    #[must_use]
    pub fn rated(mut self, levels: LoadLevels) -> Self {
        self.load = Load::of(
            self.percent,
            levels.ram_notice_percent,
            levels.ram_warning_percent,
            levels.ram_critical_percent,
        );
        let (words, line) = match self.load {
            Load::Normal => ("", ""),
            Load::Notice => (": elevated use", "\nElevated use"),
            Load::High => (": under pressure", "\nUnder pressure"),
            Load::Critical => (": under heavy pressure", "\nUnder heavy pressure"),
        };
        self.description.push_str(words);
        self.tooltip.push_str(line);
        self
    }
}

/// The use of the filesystem holding one path, from one reading, as shown in the Eon Bar.
#[derive(Debug, PartialEq, Eq)]
pub struct DiskUse {
    pub(crate) label: String,
    /// The hover list: one fact per line, without the use the label shows.
    pub(crate) tooltip: String,
    /// The accessible value: every fact in one sentence.
    pub(crate) description: String,
    pub(crate) load: Load,
    percent: u64,
}

impl DiskUse {
    /// Reads the filesystem holding `path` with `statvfs` on Linux and macOS. A failed call, or
    /// another platform, gives no value.
    #[must_use]
    pub fn read(path: &Path) -> Option<Self> {
        let (blocks, free, available, fragment) = statvfs(path)?;
        // Hover names the mount point, or the measured path when it cannot be found.
        let mount = mount_point(path);
        Self::of(
            blocks,
            free,
            available,
            fragment,
            &mount.as_deref().unwrap_or(path).display().to_string(),
        )
    }

    /// Whole-percent use as GNU df computes it from `statvfs` counts in `fragment`-byte units: used
    /// blocks (all less free ones) over used and unprivileged-available ones, so space reserved
    /// for root counts as neither, rounded up. Free space is the available blocks. An empty
    /// filesystem size, free blocks above all blocks, nothing used or available, or a size past
    /// `u64` bytes gives no value rather than a false 0%.
    pub(crate) fn of(
        blocks: u64,
        free: u64,
        available: u64,
        fragment: u64,
        mount: &str,
    ) -> Option<Self> {
        let size = blocks.checked_mul(fragment)?;
        let used = blocks.checked_sub(free)?.checked_mul(fragment)?;
        let available = available.checked_mul(fragment)?;
        let counted = used.checked_add(available)?;
        if size == 0 || counted == 0 {
            return None;
        }
        let percent = (u128::from(used) * 100).div_ceil(u128::from(counted)) as u64;
        let gib = |bytes: u64| bytes as f64 / f64::from(1 << 30);
        let (size, available) = (gib(size), gib(available));
        Some(Self {
            label: format!("{DISK_ICON} {percent}%"),
            tooltip: format!("Disk \u{b7} {mount}\n{available:.1} GiB free of {size:.1} GiB"),
            description: format!(
                "Disk use {percent}% of {size:.1} GiB on {mount}, {available:.1} GiB free"
            ),
            load: Load::Normal,
            percent,
        })
    }

    /// This reading judged against the configured levels, with how full it is named in words.
    #[must_use]
    pub fn rated(mut self, levels: LoadLevels) -> Self {
        self.load = Load::of(
            self.percent,
            levels.disk_notice_percent,
            levels.disk_warning_percent,
            levels.disk_critical_percent,
        );
        let (words, line) = match self.load {
            Load::Normal => ("", ""),
            Load::Notice => (": elevated use", "\nElevated use"),
            Load::High => (": filling up", "\nFilling up"),
            Load::Critical => (": nearly full", "\nNearly full"),
        };
        self.description.push_str(words);
        self.tooltip.push_str(line);
        self
    }
}

/// The total, free and unprivileged-available blocks of the filesystem holding `path`, and its
/// fragment size.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn statvfs(path: &Path) -> Option<(u64, u64, u64, u64)> {
    let stats = rustix::fs::statvfs(path).ok()?;
    Some((
        stats.f_blocks,
        stats.f_bfree,
        stats.f_bavail,
        stats.f_frsize,
    ))
}

/// Other platforms have no filesystem reading.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn statvfs(_: &Path) -> Option<(u64, u64, u64, u64)> {
    None
}

/// The mount point of the filesystem holding `path`, without reading mount tables: the topmost
/// ancestor of its canonical form on the same device. None when that cannot be read.
fn mount_point(path: &Path) -> Option<std::path::PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    let device = std::fs::metadata(&path).ok()?.dev();
    let mut mount = path.as_path();
    while let Some(parent) = mount.parent() {
        if std::fs::metadata(parent).ok()?.dev() != device {
            break;
        }
        mount = parent;
    }
    Some(mount.to_path_buf())
}

/// The macOS host calls, the only unsafe code in Venus (`docs/CRATES.md`); `eon-b4vh` tracks
/// removing it.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod host {
    use std::mem::{offset_of, size_of};
    use std::sync::OnceLock;

    /// One send right to the host port for the life of the process, so repeated readings do not
    /// add references to it. libc points Mach calls at the mach2 crate, which Venus does not use.
    #[allow(deprecated)]
    fn port() -> libc::host_t {
        static PORT: OnceLock<libc::host_t> = OnceLock::new();
        // SAFETY: `mach_host_self` takes no arguments and returns a port name.
        *PORT.get_or_init(|| unsafe { libc::mach_host_self() })
    }

    /// Whole-machine ticks per CPU state and the logical CPUs online.
    pub(super) fn cpu_load() -> Option<([u32; 4], usize)> {
        let mut info = libc::host_cpu_load_info { cpu_ticks: [0; 4] };
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        // SAFETY: `info` is a writable `host_cpu_load_info` and `count` holds its length in
        // `integer_t` units, which bounds what the call writes.
        let status = unsafe {
            libc::host_statistics(
                port(),
                libc::HOST_CPU_LOAD_INFO,
                (&raw mut info).cast(),
                &raw mut count,
            )
        };
        if status != libc::KERN_SUCCESS || count != libc::HOST_CPU_LOAD_INFO_COUNT {
            return None;
        }
        let cpus = std::thread::available_parallelism().ok()?.get();
        Some((info.cpu_ticks, cpus))
    }

    pub(super) fn cpu_name() -> Option<String> {
        let mut name = [0_u8; 256];
        let mut len = name.len();
        // SAFETY: name is NUL-terminated, output has len writable bytes, and no value is set.
        let status = unsafe {
            libc::sysctlbyname(
                c"machdep.cpu.brand_string".as_ptr(),
                name.as_mut_ptr().cast(),
                &raw mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || len == 0 || len > name.len() {
            return None;
        }
        let bytes = &name[..len];
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned)
    }

    pub(super) fn load_average() -> Option<f64> {
        let mut load = 0.0;
        // SAFETY: load is writable for the one requested double.
        (unsafe { libc::getloadavg(&raw mut load, 1) } == 1).then_some(load)
    }

    #[allow(deprecated)]
    pub(super) fn per_cpu_load() -> Option<Vec<[u32; 4]>> {
        let mut cpus = 0;
        let mut info = std::ptr::null_mut();
        let mut count = 0;
        // SAFETY: all output pointers are writable; Mach allocates the returned count integers.
        let status = unsafe {
            libc::host_processor_info(
                port(),
                libc::PROCESSOR_CPU_LOAD_INFO,
                &raw mut cpus,
                &raw mut info,
                &raw mut count,
            )
        };
        if status != libc::KERN_SUCCESS {
            return None;
        }
        // The supported Darwin target is 64-bit; count and cpus are 32-bit.
        let bytes = count as usize * size_of::<libc::integer_t>();
        let valid = !info.is_null() && cpus > 0 && cpus as usize * 4 == count as usize;
        let result = if valid {
            // SAFETY: the successful call returned count integers, checked as four per CPU.
            let ticks = unsafe { std::slice::from_raw_parts(info, count as usize) };
            Some(
                ticks
                    .chunks_exact(4)
                    .map(|ticks| {
                        [
                            ticks[0] as u32,
                            ticks[1] as u32,
                            ticks[2] as u32,
                            ticks[3] as u32,
                        ]
                    })
                    .collect(),
            )
        } else {
            None
        };
        if !info.is_null() {
            // SAFETY: return this Mach allocation, in bytes, to the task that received it.
            let released = unsafe {
                libc::vm_deallocate(
                    libc::mach_task_self(),
                    info as libc::vm_address_t,
                    bytes as libc::vm_size_t,
                )
            };
            if released != libc::KERN_SUCCESS {
                return None;
            }
        }
        result
    }

    /// Internal, purgeable, wired and compressed page counts, the page size in bytes and the
    /// physical memory in bytes.
    pub(super) fn memory() -> Option<([u64; 4], u64, u64)> {
        // SAFETY: `vm_statistics64` is plain integers, for which all zeroes is a valid value.
        let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        // Revision 1, the first with the internal and compressor counts. libc's struct can be
        // newer than the running kernel's, and a count above the kernel's makes it log an error.
        let rev1 = (offset_of!(libc::vm_statistics64, swapped_count) / size_of::<libc::integer_t>())
            as libc::mach_msg_type_number_t;
        let mut count = rev1;
        // SAFETY: `stats` is a writable `vm_statistics64` and `count` holds its length in
        // `integer_t` units, which bounds what the call writes.
        let status = unsafe {
            libc::host_statistics64(
                port(),
                libc::HOST_VM_INFO64,
                (&raw mut stats).cast(),
                &raw mut count,
            )
        };
        if status != libc::KERN_SUCCESS || count != rev1 {
            return None;
        }
        let pages = [
            stats.internal_page_count,
            stats.purgeable_count,
            stats.wire_count,
            stats.compressor_page_count,
        ]
        .map(u64::from);
        // SAFETY: `_SC_PAGESIZE` is a valid `sysconf` name.
        let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
        let mut memsize = 0_u64;
        let mut len = size_of::<u64>();
        // SAFETY: the name is NUL-terminated, `memsize` is writable for `len` bytes, and no new
        // value is set.
        let status = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut memsize).cast(),
                &raw mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && len == size_of::<u64>()).then_some((pages, page_size, memsize))
    }
}

/// Other platforms have no host reading.
#[cfg(not(target_os = "macos"))]
mod host {
    pub(super) fn cpu_load() -> Option<([u32; 4], usize)> {
        None
    }

    pub(super) fn cpu_name() -> Option<String> {
        None
    }
    pub(super) fn load_average() -> Option<f64> {
        None
    }
    pub(super) fn per_cpu_load() -> Option<Vec<[u32; 4]>> {
        None
    }

    pub(super) fn memory() -> Option<([u64; 4], u64, u64)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_detail_parsers_reject_bad_readings() {
        assert_eq!(
            clean_cpu_name("  13th Gen Intel(R) Core(TM)  i5 (C) "),
            Some("13th Gen Intel Core i5".into())
        );
        assert_eq!(clean_cpu_name(" (R) (TM) (C) "), None);
        assert_eq!(parse_load("1.24 2.5 3.6 1/500 123"), Some("1.2".into()));
        for invalid in ["", "NaN", "inf", "-1.2", "no load"] {
            assert_eq!(parse_load(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn busiest_cpu_uses_matching_online_identities_and_valid_deltas() {
        let before = CpuTimes::parse(
            "cpu 100 0 0 100 0 0 0 0\ncpu0 50 0 0 50 0 0 0 0\ncpu3 50 0 0 50 0 0 0 0",
        )
        .unwrap();
        let after = CpuTimes::parse(
            "cpu 210 0 0 190 0 0 0 0\ncpu0 63 0 0 137 0 0 0 0\ncpu3 147 0 0 53 0 0 0 0",
        )
        .unwrap();
        assert_eq!(before.busiest(&after), Some((3, 97)));
        for stat in [
            "cpu 210 0 0 190 0 0 0 0\ncpu3 147 0 0 53 0 0 0 0",
            "cpu 210 0 0 190 0 0 0 0\ncpu1 63 0 0 137 0 0 0 0\ncpu3 147 0 0 53 0 0 0 0",
            "cpu 210 0 0 190 0 0 0 0\ncpu0 49 0 0 137 0 0 0 0\ncpu3 147 0 0 53 0 0 0 0",
            "cpu 210 0 0 190 0 0 0 0\ncpu0 broken\ncpu3 147 0 0 53 0 0 0 0",
        ] {
            assert_eq!(
                before.busiest(&CpuTimes::parse(stat).unwrap()),
                None,
                "{stat}"
            );
        }
        assert_eq!(before.busiest(&before), None);
        let first =
            CpuTimes::parse("cpu 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0\ncpu1 0 0 0 0 0 0 0 0")
                .unwrap();
        let rounded_tie = CpuTimes::parse(
            "cpu 1003 0 0 997 0 0 0 0\ncpu0 501 0 0 499 0 0 0 0\ncpu1 502 0 0 498 0 0 0 0",
        )
        .unwrap();
        assert_eq!(first.busiest(&rounded_tie), Some((1, 50)));
    }

    #[test]
    fn a_per_cpu_field_rollback_omits_busiest_cpu() {
        let before = CpuTimes::parse(
            "cpu 200 0 0 200 200 0 0 0\ncpu0 100 0 0 100 100 0 0 0\ncpu1 100 0 0 100 100 0 0 0",
        )
        .unwrap();
        let after = CpuTimes::parse(
            "cpu 310 0 0 300 150 0 0 0\ncpu0 150 0 0 160 50 0 0 0\ncpu1 160 0 0 140 100 0 0 0",
        )
        .unwrap();
        // CPU 0's decreasing iowait would falsely beat CPU 1's real 60% use.
        assert_eq!(before.busiest(&after), None);
        let now = Instant::now();
        let mut readings = CpuReadings::default();
        let gap = Duration::from_secs(3);
        let levels = LoadLevels::default();
        readings.push(now, Some(before), gap, levels);
        let shown = readings
            .push(now + Duration::from_secs(2), Some(after), gap, levels)
            .unwrap();
        assert_eq!(shown.tooltip, "CPU\n2 logical CPUs\nElevated use");
        assert!(!shown.description.contains("CPU 0 at"));
        assert!(!shown.description.contains("CPU 1 at"));
    }

    #[test]
    fn averaged_readings_keep_per_cpu_ticks_only_for_the_latest_interval() {
        let now = Instant::now();
        let mut readings = CpuReadings::default();
        for step in 0..4 {
            let times = CpuTimes::parse(&format!(
                "cpu {step} 0 0 {step} 0 0 0 0\ncpu0 {step} 0 0 {step} 0 0 0 0"
            ))
            .unwrap();
            let shown = readings.push(
                now + Duration::from_secs(step * 2),
                Some(times),
                Duration::from_secs(3),
                LoadLevels::default(),
            );
            if step > 0 {
                assert!(shown.unwrap().tooltip.ends_with("CPU 0 at 50%"));
            }
        }
        assert_eq!(readings.readings.len(), 4);
        assert!(readings.readings.back().unwrap().1.per_cpu.is_some());
        assert!(
            readings
                .readings
                .iter()
                .take(3)
                .all(|(_, times)| times.per_cpu.is_none())
        );
    }

    #[test]
    fn cpu_details_follow_title_detail_layout_and_thermal_levels() {
        let now = Instant::now();
        let gap = Duration::from_secs(3);
        let levels = LoadLevels {
            cpu_average: Duration::from_secs(2),
            ..LoadLevels::default()
        };
        let first = CpuTimes::parse("cpu 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0").unwrap();
        let mut later = CpuTimes::parse("cpu 5 0 0 95 0 0 0 0\ncpu0 5 0 0 95 0 0 0 0").unwrap();
        later.name = Some("Example CPU");
        later.load_average = Some("1.2".into());
        for (temperature, load, hot) in [
            (59, Load::Normal, false),
            (84, Load::Normal, false),
            (85, Load::High, true),
            (94, Load::High, true),
            (95, Load::Critical, true),
        ] {
            later.temperature = Some(temperature);
            let mut readings = CpuReadings::default();
            readings.push(now, Some(first.clone()), gap, levels);
            let shown = readings
                .push(
                    now + Duration::from_secs(2),
                    Some(later.clone()),
                    gap,
                    levels,
                )
                .unwrap();
            assert_eq!(shown.load, load);
            assert_eq!(
                shown.tooltip,
                format!(
                    "CPU · Example CPU\n1 logical CPU · {temperature} °C\n5% on average over the last 2 s\nCPU 0 at 5%\nLoad 1.2 over 1 min{}",
                    if hot { "\nRunning hot" } else { "" }
                )
            );
            assert_eq!(
                shown.description,
                format!(
                    "CPU use 5% across 1 logical CPU, over the last 2 seconds; Example CPU; {temperature} °C; 5% on average over the last 2 seconds; CPU 0 at 5%; Load 1.2 over 1 min{}",
                    if hot { "; running hot" } else { "" }
                )
            );
        }
    }

    #[test]
    fn cpu_keeps_the_stronger_use_or_temperature_grade() {
        let now = Instant::now();
        let gap = Duration::from_secs(3);
        let first = CpuTimes::parse("cpu 0 0 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0").unwrap();
        for (percent, temperature, load) in [
            (90, 85, Load::Critical),
            (60, 85, Load::High),
            (75, 95, Load::Critical),
            (60, 84, Load::Notice),
        ] {
            let mut later = CpuTimes::parse(&format!(
                "cpu {percent} 0 0 {} 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0",
                100 - percent
            ))
            .unwrap();
            later.temperature = Some(temperature);
            let mut readings = CpuReadings::default();
            readings.push(now, Some(first.clone()), gap, LoadLevels::default());
            let shown = readings
                .push(
                    now + Duration::from_secs(2),
                    Some(later),
                    gap,
                    LoadLevels::default(),
                )
                .unwrap();
            assert_eq!(shown.load, load, "{percent}% {temperature} °C");
            assert_eq!(shown.description.contains("running hot"), temperature >= 85);
        }
    }

    #[test]
    fn hwmon_selects_only_cpu_package_sensors() {
        let root = std::env::temp_dir().join(format!("venus-hwmon-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let sensor = |device: &str, name: &str, readings: &[(&str, &str, &str)]| {
            let dir = root.join(device);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("name"), name).unwrap();
            for (id, label, value) in readings {
                std::fs::write(dir.join(format!("temp{id}_label")), label).unwrap();
                std::fs::write(dir.join(format!("temp{id}_input")), value).unwrap();
            }
        };
        let mut cached = None;
        sensor("hwmon0", "acpitz\n", &[("1", "Package id 0", "98000")]);
        assert_eq!(cpu_temperature(&root, &mut cached), None);
        sensor(
            "hwmon1",
            "coretemp\n",
            &[("1", "Core 0", "80000"), ("2", "Package id 0", "59499")],
        );
        assert_eq!(cpu_temperature(&root, &mut cached), Some(59));
        assert_eq!(
            cached.as_ref().map(|input| &input.path),
            Some(&root.join("hwmon1/temp2_input"))
        );
        std::fs::write(root.join("hwmon1/temp2_input"), "60500").unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), Some(61));
        // Linux can recycle hwmon IDs without an intervening failed input read.
        std::fs::write(root.join("hwmon1/name"), "acpitz\n").unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), None);
        assert!(cached.is_none());
        std::fs::write(root.join("hwmon1/name"), "coretemp\n").unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), Some(61));
        std::fs::write(root.join("hwmon1/temp2_label"), "Board\n").unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), None);
        assert!(cached.is_none());
        std::fs::remove_dir_all(root.join("hwmon1")).unwrap();
        sensor(
            "hwmon2",
            "k10temp\n",
            &[("1", "Tdie", "60500"), ("2", "Tctl", "70500")],
        );
        assert_eq!(cpu_temperature(&root, &mut cached), Some(71));
        assert_eq!(
            cached.as_ref().map(|input| &input.path),
            Some(&root.join("hwmon2/temp2_input"))
        );
        std::fs::remove_file(root.join("hwmon2/temp2_input")).unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), Some(61));
        assert_eq!(
            cached.as_ref().map(|input| &input.path),
            Some(&root.join("hwmon2/temp1_input"))
        );
        std::fs::write(root.join("hwmon2/temp1_input"), "broken").unwrap();
        assert_eq!(cpu_temperature(&root, &mut cached), None);
        assert!(cached.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A reading whose aggregate line is `line`, from a machine with `cpus` logical CPUs.
    fn times_on(cpus: usize, line: &str) -> CpuTimes {
        let per_cpu: String = (0..cpus)
            .map(|cpu| format!("\ncpu{cpu} 0 0 0 0 0 0 0 0"))
            .collect();
        CpuTimes::parse(&format!("{line}{per_cpu}\nintr 1 2")).unwrap()
    }

    fn times(line: &str) -> CpuTimes {
        times_on(1, line)
    }

    #[test]
    fn cpu_use_counts_iowait_idle_and_guest_once() {
        // user nice system idle iowait irq softirq steal guest guest_nice
        let earlier_line = "cpu  100 0 0 800 100 0 0 0 50 50";
        // 30 busy (10 of it steal) and 70 idle (20 of it iowait); guest moves but is not counted.
        let later_line = "cpu  115 0 5 850 120 0 0 10 90 90";
        let shown =
            CpuUse::between(&times_on(16, earlier_line), &times_on(16, later_line)).unwrap();
        assert_eq!(shown.label, "\u{f4bc} 30%");
        assert_eq!(
            shown.description,
            "CPU use 30% across 16 logical CPUs, over the last 2 seconds"
        );
        assert_eq!(shown.tooltip, "CPU\n16 logical CPUs");
        let shown = CpuUse::between(&times(earlier_line), &times(later_line)).unwrap();
        assert_eq!(
            shown.description,
            "CPU use 30% across 1 logical CPU, over the last 2 seconds"
        );
        assert_eq!(shown.tooltip, "CPU\n1 logical CPU");
        // Whole percentages round to nearest, from 0 to 100.
        let at = |busy, idle| times(&format!("cpu {busy} 0 0 {idle} 0 0 0 0"));
        for (busy, idle, label) in [
            (0, 3, "\u{f4bc} 0%"),
            (1, 199, "\u{f4bc} 1%"),
            (2, 1, "\u{f4bc} 67%"),
            (5, 0, "\u{f4bc} 100%"),
        ] {
            assert_eq!(
                CpuUse::between(&at(0, 0), &at(busy, idle)).unwrap().label,
                label
            );
        }
    }

    #[test]
    fn cpu_use_rejects_bad_readings_instead_of_showing_zero() {
        for stat in [
            "",
            "intr 1 2 3 4 5 6 7 8\ncpu0 0 0 0 0 0 0 0 0",
            "cpu0 1 2 3 4 5 6 7 8\ncpu1 0 0 0 0 0 0 0 0",
            "cpu 1 2 3 4 5 6 7\ncpu0 0 0 0 0 0 0 0 0",
            "cpu 1 2 3 4 x 6 7 8\ncpu0 0 0 0 0 0 0 0 0",
            "cpu 1 2 3 -4 5 6 7 8\ncpu0 0 0 0 0 0 0 0 0",
            "cpu 18446744073709551615 1 0 0 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0",
            // A valid aggregate line without the per-CPU lines right after it.
            "cpu 1 2 3 4 5 6 7 8",
            "cpu 1 2 3 4 5 6 7 8\nintr 1 2\ncpu0 0 0 0 0 0 0 0 0",
            "cpu 1 2 3 4 5 6 7 8\ncpux 0 0 0 0 0 0 0 0",
        ] {
            assert_eq!(CpuTimes::parse(stat), None, "{stat:?}");
        }
        let base = times("cpu 100 0 0 100 0 0 0 0");
        for later in [
            "cpu 100 0 0 100 0 0 0 0",
            "cpu 90 0 0 120 0 0 0 0",
            "cpu 120 0 0 90 0 0 0 0",
        ] {
            assert_eq!(CpuUse::between(&base, &times(later)), None, "{later:?}");
        }
    }

    #[test]
    fn host_cpu_use_counts_user_system_and_nice_as_busy() {
        // user system idle nice
        let earlier = CpuTimes::from_host_ticks([100, 50, 800, 10], 10);
        let later = CpuTimes::from_host_ticks([115, 60, 870, 15], 10);
        let shown = CpuUse::between(&earlier, &later).unwrap();
        assert_eq!(shown.label, "\u{f4bc} 30%");
        assert_eq!(
            shown.description,
            "CPU use 30% across 10 logical CPUs, over the last 2 seconds"
        );
        // A counter that wrapped shows nothing for that interval.
        let wrapped = CpuTimes::from_host_ticks([u32::MAX, 0, 800, 0], 10);
        let after_wrap = CpuTimes::from_host_ticks([5, 0, 801, 0], 10);
        assert_eq!(CpuUse::between(&wrapped, &after_wrap), None);
    }

    #[test]
    fn host_ram_use_is_app_wired_and_compressed_over_physical_memory() {
        // internal purgeable wired compressed, in 16 KiB pages of 16 GiB: 6 GiB app memory,
        // 2 GiB wired and 1 GiB compressed.
        let gib = 1 << 16;
        let pages = [7 * gib, gib, 2 * gib, gib];
        let shown = RamUse::from_host_pages(pages, 16_384, 16 << 30).unwrap();
        assert_eq!(shown.label, "\u{efc5} 56%");
        assert_eq!(shown.description, "RAM use 56%, 9.0 GiB of 16.0 GiB");
        assert_eq!(shown.tooltip, "RAM\n9.0 GiB used of 16.0 GiB");
        for (pages, page_size, memsize) in [
            ([0, 0, 0, 0], 16_384, 0),
            ([1, 2, 0, 0], 16_384, 1 << 30),
            ([gib, 0, 0, 1], 16_384, 1 << 30),
            ([gib, 0, 0, 0], 0, 1 << 30),
            ([u64::MAX, 0, 0, 0], 16_384, u64::MAX),
        ] {
            assert_eq!(
                RamUse::from_host_pages(pages, page_size, memsize),
                None,
                "{pages:?} {page_size} {memsize}"
            );
        }
    }

    #[test]
    fn ram_use_is_total_minus_available_over_total() {
        let shown = RamUse::parse(
            "MemTotal:       32800000 kB\nMemFree:         1000000 kB\n\
             MemAvailable:   18677760 kB\nCached:         9000000 kB\n",
        )
        .unwrap();
        assert_eq!(shown.label, "\u{efc5} 43%");
        assert_eq!(shown.description, "RAM use 43%, 13.5 GiB of 31.3 GiB");
        assert_eq!(shown.tooltip, "RAM\n13.5 GiB used of 31.3 GiB");
        // Whole percentages round to nearest, from 0 to 100.
        for (total, available, label) in [
            (3, 3, "\u{efc5} 0%"),
            (200, 199, "\u{efc5} 1%"),
            (3, 1, "\u{efc5} 67%"),
            (3, 0, "\u{efc5} 100%"),
        ] {
            let meminfo = format!("MemTotal: {total} kB\nMemAvailable: {available} kB\n");
            assert_eq!(RamUse::parse(&meminfo).unwrap().label, label, "{meminfo:?}");
        }
    }

    #[test]
    fn disk_use_is_used_over_used_and_available_rounded_up_as_gnu_df_does() {
        // 4 KiB fragments of a 100 GiB filesystem: 80 GiB used, 15 GiB free to users and 5 GiB
        // reserved for root, so use is 80 of 95 GiB, 84.2% rounded up.
        let gib = 1 << 18;
        let shown = DiskUse::of(100 * gib, 20 * gib, 15 * gib, 4096, "/home").unwrap();
        assert_eq!(shown.label, "\u{f0a0} 85%");
        assert_eq!(
            shown.description,
            "Disk use 85% of 100.0 GiB on /home, 15.0 GiB free"
        );
        assert_eq!(
            shown.tooltip,
            "Disk \u{b7} /home\n15.0 GiB free of 100.0 GiB"
        );
        // GiB keep one decimal, like RAM's total.
        let shown = DiskUse::of(9 * gib + gib / 2, 3 * gib, 3 * gib, 4096, "/").unwrap();
        assert_eq!(
            shown.description,
            "Disk use 69% of 9.5 GiB on /, 3.0 GiB free"
        );
        assert_eq!(shown.tooltip, "Disk \u{b7} /\n3.0 GiB free of 9.5 GiB");
        // Whole percentages round up, from 0 to 100.
        for (blocks, free, available, label) in [
            (3, 3, 3, "\u{f0a0} 0%"),
            (1000, 999, 999, "\u{f0a0} 1%"),
            (200, 199, 199, "\u{f0a0} 1%"),
            (3, 1, 1, "\u{f0a0} 67%"),
            (3, 0, 0, "\u{f0a0} 100%"),
            // A full filesystem with root's reserve left still reads 100%.
            (10, 1, 0, "\u{f0a0} 100%"),
        ] {
            assert_eq!(
                DiskUse::of(blocks, free, available, 1, "/").unwrap().label,
                label,
                "{blocks} {free} {available}"
            );
        }
    }

    #[test]
    fn disk_use_rejects_bad_readings_instead_of_showing_zero() {
        for (blocks, free, available, fragment) in [
            // No size.
            (0, 0, 0, 4096),
            (100, 50, 50, 0),
            // Free blocks above all blocks.
            (100, 101, 50, 4096),
            // Nothing used or available.
            (100, 100, 0, 4096),
            // Sizes past u64 bytes.
            (u64::MAX, 0, 0, 2),
            (u64::MAX / 2, u64::MAX / 4, u64::MAX / 2, 2),
        ] {
            assert_eq!(
                DiskUse::of(blocks, free, available, fragment, "/"),
                None,
                "{blocks} {free} {available} {fragment}"
            );
        }
    }

    #[test]
    fn disk_use_reads_the_filesystem_holding_a_path() {
        let shown = DiskUse::read(Path::new("/")).unwrap();
        assert!(shown.label.starts_with("\u{f0a0} "), "{}", shown.label);
        assert!(
            shown.description.contains(" on /, "),
            "{}",
            shown.description
        );
        assert_eq!(mount_point(Path::new("/")), Some("/".into()));
        // A path below the root mount names the topmost ancestor on its device.
        let directory = std::env::temp_dir();
        let mount = mount_point(&directory).unwrap();
        let device = |path: &Path| std::fs::metadata(path).unwrap().dev();
        assert_eq!(device(&mount), device(&directory));
        assert!(
            mount
                .parent()
                .is_none_or(|parent| device(parent) != device(&mount))
        );
        let missing = directory.join("eon-disk-reading-missing");
        assert_eq!(DiskUse::read(&missing), None);
        assert_eq!(mount_point(&missing), None);
    }

    #[test]
    fn ram_use_rejects_bad_readings_instead_of_showing_zero() {
        for meminfo in [
            "",
            "MemTotal: 100 kB\nMemFree: 90 kB\n",
            "MemAvailable: 10 kB\n",
            "MemTotal: 0 kB\nMemAvailable: 0 kB\n",
            "MemTotal: 100 kB\nMemAvailable: 101 kB\n",
            "MemTotal: 100\nMemAvailable: 10 kB\n",
            "MemTotal: 100 kB\nMemAvailable: x kB\n",
            "MemTotal: -100 kB\nMemAvailable: 10 kB\n",
            "MemTotalX: 100 kB\nMemAvailable: 10 kB\n",
        ] {
            assert_eq!(RamUse::parse(meminfo), None, "{meminfo:?}");
        }
    }
}
