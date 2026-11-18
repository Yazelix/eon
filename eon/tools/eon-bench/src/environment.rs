//! What a result records about the machine it ran on, and CPU pinning.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    result::{Build, Source},
    sha256::Sha256,
    sys::die_with_parent,
};

/// Nanoseconds on the system's monotonic clock, which every process reads
/// alike, so times taken in different processes compare.
pub fn monotonic_ns() -> u64 {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

/// `/run/user/<effective uid>`, the parent of private runtime roots.
pub(crate) fn user_runtime_parent() -> PathBuf {
    PathBuf::from(format!("/run/user/{}", rustix::process::geteuid().as_raw()))
}

/// The 1-minute load average.
pub fn load() -> Result<f64, String> {
    #[cfg(target_os = "linux")]
    let load = Some(load_from_fixed(rustix::system::sysinfo().loads[0]));
    #[cfg(not(target_os = "linux"))]
    let load = crate::sys::load_average();
    load.ok_or_else(|| "the load average is unavailable".into())
}

/// The kernel's fixed-point load (16 fraction bits), converted as glibc's
/// getloadavg converts sysinfo's. `/proc/loadavg` rounds to two decimals,
/// which would admit a load just above a `--max-load` ceiling.
#[cfg(target_os = "linux")]
fn load_from_fixed(raw: u64) -> f64 {
    raw as f64 / f64::from(1 << 16)
}

/// The CPUs online on the machine, not only those this process may use
/// (`cpus_allowed`), counted as glibc's `sysconf(_SC_NPROCESSORS_ONLN)`
/// counts them on Linux; elsewhere std's count, which on Apple is that same
/// sysconf (1 when unknown).
pub fn cpus_online() -> u64 {
    #[cfg(target_os = "linux")]
    let count = online_count(
        std::fs::read_to_string("/sys/devices/system/cpu/online")
            .ok()
            .as_deref(),
        std::fs::read_to_string("/proc/stat").ok().as_deref(),
        cpus_allowed().map(|cpus| cpus.len() as u64),
    );
    #[cfg(not(target_os = "linux"))]
    let count = std::thread::available_parallelism().map_or(1, |count| count.get() as u64);
    count
}

/// glibc's `get_nprocs` order: the sysfs online list, else the `cpuN` lines
/// at the top of `/proc/stat`, else the affinity mask, else 2.
#[cfg(target_os = "linux")]
fn online_count(online: Option<&str>, stat: Option<&str>, allowed: Option<u64>) -> u64 {
    let listed = online
        .and_then(|list| parse_cpus(list.trim()).ok())
        .map(|cpus| cpus.len() as u64);
    let counted = stat.map(|text| {
        text.lines()
            .take_while(|line| line.starts_with("cpu"))
            .filter(|line| line.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
            .count() as u64
    });
    [listed, counted, allowed]
        .into_iter()
        .flatten()
        .find(|&count| count > 0)
        .unwrap_or(2)
}

/// `sysname release machine`, from uname.
pub fn kernel() -> String {
    let name = rustix::system::uname();
    format!(
        "{} {} {}",
        name.sysname().to_string_lossy(),
        name.release().to_string_lossy(),
        name.machine().to_string_lossy()
    )
}

/// The operating system and its version: `macOS <ProductVersion>` from
/// sw_vers, or os-release's `PRETTY_NAME` elsewhere.
pub fn os() -> Option<String> {
    if cfg!(target_os = "macos") {
        return command_output("sw_vers", &["-productVersion"])
            .map(|version| format!("macOS {version}"));
    }
    std::fs::read_to_string("/etc/os-release")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|name| name.trim_matches('"').to_owned())
}

/// A command's trimmed standard output, when it succeeds and prints any.
fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = die_with_parent(Command::new(program).args(arguments))
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

pub fn cpu_model() -> Option<String> {
    if cfg!(target_os = "macos") {
        return command_output("sysctl", &["-n", "machdep.cpu.brand_string"]);
    }
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()?
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "model name").then(|| value.trim().to_owned())
        })
}

/// Parses a CPU list such as `2,3` or `4-7,9`.
pub fn parse_cpus(list: &str) -> Result<Vec<u64>, String> {
    let mut cpus = Vec::new();
    for part in list.split(',') {
        let number = |text: &str| {
            text.trim()
                .parse::<u64>()
                .map_err(|_| format!("bad CPU list {list:?}"))
        };
        let (low, high) = match part.split_once('-') {
            Some((low, high)) => (number(low)?, number(high)?),
            None => (number(part)?, number(part)?),
        };
        if low > high {
            return Err(format!("bad CPU list {list:?}"));
        }
        cpus.extend(low..=high);
    }
    cpus.sort_unstable();
    cpus.dedup();
    Ok(cpus)
}

#[cfg(target_os = "linux")]
pub use linux::{cpus_allowed, pin};

#[cfg(target_os = "linux")]
mod linux {
    use rustix::thread::{CpuSet, sched_getaffinity, sched_setaffinity};

    /// Pins this process, and the processes it starts, to `cpus`.
    pub fn pin(cpus: &[u64]) -> Result<(), String> {
        let mut set = CpuSet::new();
        for &cpu in cpus {
            let cpu = usize::try_from(cpu)
                .ok()
                .filter(|&cpu| cpu < CpuSet::MAX_CPU)
                .ok_or(format!("CPU {cpu} is out of range"))?;
            set.set(cpu);
        }
        sched_setaffinity(None, &set)
            .map_err(|error| format!("pinning to CPUs {cpus:?}: {}", std::io::Error::from(error)))
    }

    /// The CPUs this process may run on.
    pub fn cpus_allowed() -> Option<Vec<u64>> {
        let set = sched_getaffinity(None).ok()?;
        Some(
            (0..CpuSet::MAX_CPU)
                .filter(|&cpu| set.is_set(cpu))
                .map(|cpu| cpu as u64)
                .collect(),
        )
    }
}

#[cfg(not(target_os = "linux"))]
pub fn pin(_cpus: &[u64]) -> Result<(), String> {
    Err("--cpus pins only on Linux".into())
}

#[cfg(not(target_os = "linux"))]
pub fn cpus_allowed() -> Option<Vec<u64>> {
    None
}

/// The distinct frequency governors of `cpus`, comma-separated (Linux).
pub fn governor(cpus: &[u64]) -> Option<String> {
    let mut governors: Vec<String> = cpus
        .iter()
        .filter_map(|cpu| {
            std::fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .ok()
        })
        .map(|governor| governor.trim().to_owned())
        .collect();
    governors.sort();
    governors.dedup();
    (!governors.is_empty()).then(|| governors.join(","))
}

/// The running eon-bench binary, its build profile and the Eon source it came
/// from: the commit the Nix build recorded in `EON_BENCH_REVISION` (`<commit>`
/// or `<commit>-dirty`), else the Git checkout the binary was built in, read
/// now, with untracked files ignored.
pub fn build() -> Result<Build, String> {
    let path = std::env::current_exe().map_err(|error| format!("eon-bench's path: {error}"))?;
    let binary = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&binary);
    let source = match option_env!("EON_BENCH_REVISION").filter(|revision| !revision.is_empty()) {
        Some(revision) => Some(Source {
            commit: revision.trim_end_matches("-dirty").to_owned(),
            dirty: revision.ends_with("-dirty"),
            from: "build".into(),
        }),
        None => checkout_source(),
    };
    Ok(Build {
        path: path.display().to_string(),
        sha256: hasher.hex(),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .into(),
        source,
    })
}

fn checkout_source() -> Option<Source> {
    // The workspace root, three levels above this crate's manifest.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let git = |arguments: &[&str]| {
        let output = die_with_parent(Command::new("git").arg("-C").arg(&root).args(arguments))
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    Some(Source {
        commit: git(&["rev-parse", "HEAD"])?,
        dirty: !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty(),
        from: "checkout-at-run".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists_take_numbers_and_ranges() {
        assert_eq!(parse_cpus("3,2").unwrap(), [2, 3]);
        assert_eq!(parse_cpus("4-6,9,5").unwrap(), [4, 5, 6, 9]);
        for bad in ["", "a", "3-1", "1,", "-2"] {
            assert!(parse_cpus(bad).is_err(), "accepted {bad:?}");
        }
    }

    /// /proc/loadavg would print 1.00 here and pass a --max-load 1 ceiling.
    #[cfg(target_os = "linux")]
    #[test]
    fn load_keeps_the_kernel_fixed_point_above_a_ceiling() {
        assert_eq!(load_from_fixed(65_792), 1.003_906_25);
        assert!(load_from_fixed(65_537) > 1.0);
        assert_eq!(load_from_fixed(1 << 16), 1.0);
        assert!(load().unwrap() >= 0.0);
    }

    /// Without sysfs, glibc counts /proc/stat; a fallback of 1 would lower the
    /// default load ceiling and fail a gate baseline's fixed conditions.
    #[cfg(target_os = "linux")]
    #[test]
    fn online_cpus_fall_back_as_glibc_does() {
        let stat = "cpu  1 2 3\ncpu0 1 2\ncpu1 1 2\ncpu2 1\ncpu3 1\nintr 5\ncpu9 1\n";
        assert_eq!(online_count(Some("0-7\n"), Some(stat), Some(3)), 8);
        assert_eq!(online_count(None, Some(stat), Some(3)), 4);
        assert_eq!(online_count(Some("garbage"), Some(stat), Some(3)), 4);
        assert_eq!(online_count(None, None, Some(3)), 3);
        assert_eq!(online_count(None, Some("intr 5\n"), None), 2);
    }

    /// A wrong CpuSet bit or size pins to, or reports, other CPUs.
    #[cfg(target_os = "linux")]
    #[test]
    fn pin_and_cpus_allowed_round_trip() {
        let allowed = cpus_allowed().unwrap();
        let last = *allowed.last().unwrap();
        pin(&[last]).unwrap();
        assert_eq!(cpus_allowed().unwrap(), [last]);
        pin(&allowed).unwrap();
        assert_eq!(cpus_allowed().unwrap(), allowed);
        assert!(cpus_online() >= allowed.len() as u64);
        assert!(pin(&[CPU_OUT_OF_RANGE]).is_err());
    }

    #[cfg(target_os = "linux")]
    const CPU_OUT_OF_RANGE: u64 = rustix::thread::CpuSet::MAX_CPU as u64;
}
