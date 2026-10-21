//! Whole-machine CPU use for the Eon Bar (VEN-C28), from the aggregate `/proc/stat` line.

/// Cumulative whole-machine CPU time since boot, in kernel ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    busy: u64,
    total: u64,
}

impl CpuTimes {
    /// Reads the aggregate line of `/proc/stat`; other platforms have no reading.
    #[must_use]
    pub fn read() -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        Self::parse(&std::fs::read_to_string("/proc/stat").ok()?)
    }

    /// Parses the first `/proc/stat` line. Idle is idle plus iowait; total is the first eight
    /// fields, leaving out guest and guest_nice, which the kernel already counts in user and nice.
    pub(crate) fn parse(stat: &str) -> Option<Self> {
        let mut fields = stat.lines().next()?.split_ascii_whitespace();
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
        Some(Self {
            busy: total - ticks[3] - ticks[4],
            total,
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
        Some(Self {
            // Padded to the width of `CPU 100%`, so the header never shifts as the value changes.
            label: format!("{:>8}", format!("CPU {percent}%")),
            description: format!("CPU use {percent}%, all cores, over the last 2 seconds"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(line: &str) -> CpuTimes {
        CpuTimes::parse(line).unwrap()
    }

    #[test]
    fn cpu_use_counts_iowait_idle_and_guest_once() {
        // user nice system idle iowait irq softirq steal guest guest_nice
        let earlier = times("cpu  100 0 0 800 100 0 0 0 50 50\ncpu0 1 2 3 4 5 6 7 8\n");
        // 30 busy (10 of it steal) and 70 idle (20 of it iowait); guest moves but is not counted.
        let later = times("cpu  115 0 5 850 120 0 0 10 90 90\n");
        let shown = CpuUse::between(earlier, later).unwrap();
        assert_eq!(shown.label, " CPU 30%");
        assert_eq!(
            shown.description,
            "CPU use 30%, all cores, over the last 2 seconds"
        );
        // Whole percentages round to nearest, from 0 to 100.
        let at = |busy, idle| times(&format!("cpu {busy} 0 0 {idle} 0 0 0 0"));
        for (busy, idle, label) in [
            (0, 3, "  CPU 0%"),
            (1, 199, "  CPU 1%"),
            (2, 1, " CPU 67%"),
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
            "intr 1 2 3 4 5 6 7 8",
            "cpu0 1 2 3 4 5 6 7 8",
            "cpu 1 2 3 4 5 6 7",
            "cpu 1 2 3 4 x 6 7 8",
            "cpu 1 2 3 -4 5 6 7 8",
            "cpu 18446744073709551615 1 0 0 0 0 0 0",
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
}
