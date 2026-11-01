//! Whole-machine CPU and memory use and one filesystem's use for the Eon Bar
//! (`docs/PRESENTATION.md`): on Linux from `/proc/stat`'s aggregate and per-CPU lines and
//! `/proc/meminfo`, on macOS from the Mach host statistics, and on both from `statvfs`.

use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

/// How loaded a CPU, RAM or disk chip shows the machine; only load beyond normal colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Load {
    Normal,
    High,
    Critical,
}

impl Load {
    /// A whole-percent use judged against two levels: high from `warning`, critical from
    /// `critical`, both inclusive.
    fn of(percent: u64, warning: u8, critical: u8) -> Self {
        if percent >= u64::from(critical) {
            Self::Critical
        } else if percent >= u64::from(warning) {
            Self::High
        } else {
            Self::Normal
        }
    }
}

/// When the CPU, RAM and disk chips colour, and the CPU hover's averaging window, as Eon's `[bar]`
/// configuration resolves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadLevels {
    pub ram_warning_percent: u8,
    pub ram_critical_percent: u8,
    pub cpu_warning_percent: u8,
    pub cpu_average: Duration,
    pub disk_warning_percent: u8,
    pub disk_critical_percent: u8,
}

impl Default for LoadLevels {
    fn default() -> Self {
        Self {
            ram_warning_percent: 80,
            ram_critical_percent: 90,
            cpu_warning_percent: 90,
            cpu_average: Duration::from_secs(30),
            disk_warning_percent: 85,
            disk_critical_percent: 95,
        }
    }
}

/// `part` of `whole` in whole percent, rounded to nearest; `whole` must be positive.
fn percent(part: u64, whole: u64) -> u64 {
    ((u128::from(part) * 200 + u128::from(whole)) / (u128::from(whole) * 2)) as u64
}

/// Cumulative whole-machine CPU time since boot, in kernel ticks, and the logical CPUs online.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    total: u64,
    cpus: usize,
}

impl CpuTimes {
    /// Reads `/proc/stat` on Linux and the host CPU load on macOS; other platforms have no
    /// reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        if cfg!(target_os = "linux") {
            return Self::parse(&std::fs::read_to_string("/proc/stat").ok()?);
        }
        let (ticks, cpus) = host::cpu_load()?;
        Some(Self::from_host_ticks(ticks, cpus))
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
        let mut ticks = [0_u64; 8];
        for tick in &mut ticks {
            *tick = fields.next()?.parse().ok()?;
        }
        let total = ticks
            .iter()
            .try_fold(0_u64, |sum, tick| sum.checked_add(*tick))?;
        let cpus = lines
            .take_while(|line| {
                line.strip_prefix("cpu")
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            })
            .count();
        if cpus == 0 {
            return None;
        }
        Some(Self {
            busy: total - ticks[3] - ticks[4],
            total,
            cpus,
        })
    }
}

/// CPU use between two readings, as shown in the workspace header.
#[derive(Debug, PartialEq, Eq)]
pub struct CpuUse {
    pub(crate) percent: u64,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) load: Load,
}

impl CpuUse {
    /// Whole-percent use between two readings. Counters that did not advance or went backwards
    /// give no value rather than a false 0% or 100%.
    #[must_use]
    pub(crate) fn between(earlier: CpuTimes, later: CpuTimes) -> Option<Self> {
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
            label: format!("CPU {percent}%"),
            description: format!(
                "CPU use {percent}% across {cpus} logical CPU{plural}, over the last 2 seconds"
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
    /// reading, high from `levels.cpu_warning_percent` and with the average over
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
            .zip(times)
            .and_then(|((_, earlier), later)| CpuUse::between(*earlier, later));
        if shown.is_none() {
            self.readings.clear();
        }
        let times = times?;
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
        if shown.percent >= u64::from(levels.cpu_warning_percent) {
            shown.load = Load::High;
            shown.description.push_str(": under high load");
        }
        if let Some(average) = self.average(now, window) {
            shown.description.push_str(&format!(
                "; {average}% on average over the last {} seconds",
                window.as_secs()
            ));
        }
        Some(shown)
    }

    /// Whole-percent use over exactly the last `window`, once the readings span it. Only the share
    /// of the oldest interval inside the window counts, taking use as even within it.
    fn average(&self, now: Instant, window: Duration) -> Option<u64> {
        let (&(first, earliest), &(next, after)) = (self.readings.front()?, self.readings.get(1)?);
        let latest = self.readings.back()?.1;
        // Pruning keeps `next` younger than the window, so the share is in [0, 1).
        let outside = (now.duration_since(first).checked_sub(window)?).as_secs_f64()
            / next.duration_since(first).as_secs_f64();
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
        Some(Self {
            label: format!("RAM {percent}%"),
            description: format!("RAM use {percent}% of {:.1} GiB", total as f64 / per_gib),
            load: Load::Normal,
            percent,
        })
    }

    /// This reading judged against the configured levels, with the pressure named in words.
    #[must_use]
    pub fn rated(mut self, levels: LoadLevels) -> Self {
        self.load = Load::of(
            self.percent,
            levels.ram_warning_percent,
            levels.ram_critical_percent,
        );
        self.description.push_str(match self.load {
            Load::Normal => "",
            Load::High => ": under pressure",
            Load::Critical => ": under heavy pressure",
        });
        self
    }
}

/// The use of the filesystem holding one path, from one reading, as shown in the Eon Bar.
#[derive(Debug, PartialEq, Eq)]
pub struct DiskUse {
    pub(crate) label: String,
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
        Some(Self {
            label: format!("DISK {percent}%"),
            description: format!(
                "Disk use {percent}% of {:.1} GiB on {mount}, {:.1} GiB free",
                gib(size),
                gib(available)
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
            levels.disk_warning_percent,
            levels.disk_critical_percent,
        );
        self.description.push_str(match self.load {
            Load::Normal => "",
            Load::High => ": filling up",
            Load::Critical => ": nearly full",
        });
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

    pub(super) fn memory() -> Option<([u64; 4], u64, u64)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let shown = CpuUse::between(times_on(16, earlier_line), times_on(16, later_line)).unwrap();
        assert_eq!(shown.label, "CPU 30%");
        assert_eq!(
            shown.description,
            "CPU use 30% across 16 logical CPUs, over the last 2 seconds"
        );
        let shown = CpuUse::between(times(earlier_line), times(later_line)).unwrap();
        assert_eq!(
            shown.description,
            "CPU use 30% across 1 logical CPU, over the last 2 seconds"
        );
        // Whole percentages round to nearest, from 0 to 100.
        let at = |busy, idle| times(&format!("cpu {busy} 0 0 {idle} 0 0 0 0"));
        for (busy, idle, label) in [
            (0, 3, "CPU 0%"),
            (1, 199, "CPU 1%"),
            (2, 1, "CPU 67%"),
            (5, 0, "CPU 100%"),
        ] {
            assert_eq!(
                CpuUse::between(at(0, 0), at(busy, idle)).unwrap().label,
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
            assert_eq!(CpuUse::between(base, times(later)), None, "{later:?}");
        }
    }

    #[test]
    fn host_cpu_use_counts_user_system_and_nice_as_busy() {
        // user system idle nice
        let earlier = CpuTimes::from_host_ticks([100, 50, 800, 10], 10);
        let later = CpuTimes::from_host_ticks([115, 60, 870, 15], 10);
        let shown = CpuUse::between(earlier, later).unwrap();
        assert_eq!(shown.label, "CPU 30%");
        assert_eq!(
            shown.description,
            "CPU use 30% across 10 logical CPUs, over the last 2 seconds"
        );
        // A counter that wrapped shows nothing for that interval.
        let wrapped = CpuTimes::from_host_ticks([u32::MAX, 0, 800, 0], 10);
        let after_wrap = CpuTimes::from_host_ticks([5, 0, 801, 0], 10);
        assert_eq!(CpuUse::between(wrapped, after_wrap), None);
    }

    #[test]
    fn host_ram_use_is_app_wired_and_compressed_over_physical_memory() {
        // internal purgeable wired compressed, in 16 KiB pages of 16 GiB: 6 GiB app memory,
        // 2 GiB wired and 1 GiB compressed.
        let gib = 1 << 16;
        let pages = [7 * gib, gib, 2 * gib, gib];
        let shown = RamUse::from_host_pages(pages, 16_384, 16 << 30).unwrap();
        assert_eq!(shown.label, "RAM 56%");
        assert_eq!(shown.description, "RAM use 56% of 16.0 GiB");
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
        assert_eq!(shown.label, "RAM 43%");
        assert_eq!(shown.description, "RAM use 43% of 31.3 GiB");
        // Whole percentages round to nearest, from 0 to 100.
        for (total, available, label) in [
            (3, 3, "RAM 0%"),
            (200, 199, "RAM 1%"),
            (3, 1, "RAM 67%"),
            (3, 0, "RAM 100%"),
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
        assert_eq!(shown.label, "DISK 85%");
        assert_eq!(
            shown.description,
            "Disk use 85% of 100.0 GiB on /home, 15.0 GiB free"
        );
        // GiB keep one decimal, like RAM's total.
        let shown = DiskUse::of(9 * gib + gib / 2, 3 * gib, 3 * gib, 4096, "/").unwrap();
        assert_eq!(
            shown.description,
            "Disk use 69% of 9.5 GiB on /, 3.0 GiB free"
        );
        // Whole percentages round up, from 0 to 100.
        for (blocks, free, available, label) in [
            (3, 3, 3, "DISK 0%"),
            (1000, 999, 999, "DISK 1%"),
            (200, 199, 199, "DISK 1%"),
            (3, 1, 1, "DISK 67%"),
            (3, 0, 0, "DISK 100%"),
            // A full filesystem with root's reserve left still reads 100%.
            (10, 1, 0, "DISK 100%"),
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
        assert!(shown.label.starts_with("DISK "), "{}", shown.label);
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
