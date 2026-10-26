//! Whole-machine CPU and memory use for the Eon Bar (VEN-C28), from `/proc/stat`'s aggregate
//! and per-CPU lines and `/proc/meminfo`.

/// Cumulative whole-machine CPU time since boot, in kernel ticks, and the logical CPUs online.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    total: u64,
    cpus: usize,
}

impl CpuTimes {
    /// Reads `/proc/stat`; other platforms have no reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        Self::parse(&std::fs::read_to_string("/proc/stat").ok()?)
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

/// Memory use from one `/proc/meminfo` reading, as shown in the workspace header.
#[derive(Debug, PartialEq, Eq)]
pub struct RamUse {
    pub(crate) label: String,
    pub(crate) description: String,
}

impl RamUse {
    /// Reads `/proc/meminfo`; other platforms have no reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        Self::parse(&std::fs::read_to_string("/proc/meminfo").ok()?)
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
        if total == 0 {
            return None;
        }
        let percent = (u128::from(used) * 200 + u128::from(total)) / (u128::from(total) * 2);
        Some(Self {
            label: format!("RAM {percent}%"),
            description: format!(
                "RAM use {percent}% of {:.1} GiB",
                total as f64 / f64::from(1 << 20)
            ),
        })
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
