//! Whole-machine CPU and memory use for the Eon Bar (`docs/PRESENTATION.md`): on Linux from
//! `/proc/stat`'s aggregate and per-CPU lines and `/proc/meminfo`, on macOS from the Mach host
//! statistics.

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
    pub(crate) label: String,
    pub(crate) description: String,
}

impl CpuUse {
    /// Whole-percent use between two readings. Counters that did not advance or went backwards
    /// give no value rather than a false 0% or 100%.
    #[must_use]
    pub fn between(earlier: CpuTimes, later: CpuTimes) -> Option<Self> {
        let total = later.total.checked_sub(earlier.total)?;
        let busy = later.busy.checked_sub(earlier.busy)?;
        if total == 0 || busy > total {
            return None;
        }
        let percent = (u128::from(busy) * 200 + u128::from(total)) / (u128::from(total) * 2);
        let cpus = later.cpus;
        let plural = if cpus == 1 { "" } else { "s" };
        Some(Self {
            label: format!("CPU {percent}%"),
            description: format!(
                "CPU use {percent}% across {cpus} logical CPU{plural}, over the last 2 seconds"
            ),
        })
    }
}

/// Memory use from one reading, as shown in the workspace header.
#[derive(Debug, PartialEq, Eq)]
pub struct RamUse {
    pub(crate) label: String,
    pub(crate) description: String,
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
        let percent = (u128::from(used) * 200 + u128::from(total)) / (u128::from(total) * 2);
        Some(Self {
            label: format!("RAM {percent}%"),
            description: format!("RAM use {percent}% of {:.1} GiB", total as f64 / per_gib),
        })
    }
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
