//! What a result records about the machine it ran on, and CPU pinning.

use std::{ffi::CStr, path::Path, process::Command};

use crate::{
    result::{Build, Source},
    sha256::Sha256,
};

/// Has `command`'s process get SIGKILL when this process dies, however it
/// dies, and refuse to start if this process died before it could ask
/// (Linux; elsewhere a no-op). A spawn from a thread that ends first kills
/// it too, so callers spawn from the main thread.
pub fn die_with_parent(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id();
        // SAFETY: prctl and getppid are async-signal-safe, and the errors
        // need no allocation.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if u32::try_from(libc::getppid()) != Ok(parent) {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    command
}

/// Nanoseconds on the system's monotonic clock, which every process reads
/// alike, so times taken in different processes compare.
pub fn monotonic_ns() -> u64 {
    // SAFETY: timespec is plain data that clock_gettime fills.
    let mut time = unsafe { std::mem::zeroed::<libc::timespec>() };
    // SAFETY: time is a valid timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut time) };
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

/// The 1-minute load average.
pub fn load() -> Result<f64, String> {
    let mut loads = [0.0; 1];
    // SAFETY: getloadavg writes at most one value into the array.
    if unsafe { libc::getloadavg(loads.as_mut_ptr(), 1) } != 1 {
        return Err("the load average is unavailable".into());
    }
    Ok(loads[0])
}

pub fn cpus_online() -> u64 {
    // SAFETY: sysconf only reads a configuration value.
    u64::try_from(unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }).unwrap_or(1)
}

/// `sysname release machine`, from uname.
pub fn kernel() -> String {
    // SAFETY: utsname is plain data that uname fills on success.
    let mut name = unsafe { std::mem::zeroed::<libc::utsname>() };
    // SAFETY: name is a valid utsname.
    if unsafe { libc::uname(&raw mut name) } == -1 {
        return std::env::consts::OS.to_owned();
    }
    let text = |field: &[libc::c_char]| {
        // SAFETY: uname writes NUL-terminated strings within each field.
        unsafe { CStr::from_ptr(field.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    format!(
        "{} {} {}",
        text(&name.sysname),
        text(&name.release),
        text(&name.machine)
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
    /// Pins this process, and the processes it starts, to `cpus`.
    pub fn pin(cpus: &[u64]) -> Result<(), String> {
        // SAFETY: cpu_set_t is a plain bitmask; zero is the empty set.
        let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        for &cpu in cpus {
            let cpu = usize::try_from(cpu)
                .ok()
                .filter(|&cpu| cpu < libc::CPU_SETSIZE as usize)
                .ok_or(format!("CPU {cpu} is out of range"))?;
            // SAFETY: cpu is below CPU_SETSIZE, inside the set.
            unsafe { libc::CPU_SET(cpu, &mut set) };
        }
        // SAFETY: set is a valid cpu_set_t of the size passed.
        if unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &raw const set) } == -1
        {
            return Err(format!(
                "pinning to CPUs {cpus:?}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// The CPUs this process may run on.
    pub fn cpus_allowed() -> Option<Vec<u64>> {
        // SAFETY: cpu_set_t is a plain bitmask that sched_getaffinity fills.
        let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        // SAFETY: set is a valid cpu_set_t of the size passed.
        if unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &raw mut set) } == -1 {
            return None;
        }
        Some(
            (0..libc::CPU_SETSIZE as usize)
                // SAFETY: cpu is below CPU_SETSIZE, inside the set.
                .filter(|&cpu| unsafe { libc::CPU_ISSET(cpu, &set) })
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
    use super::parse_cpus;

    #[test]
    fn cpu_lists_take_numbers_and_ranges() {
        assert_eq!(parse_cpus("3,2").unwrap(), [2, 3]);
        assert_eq!(parse_cpus("4-6,9,5").unwrap(), [4, 5, 6, 9]);
        for bad in ["", "a", "3-1", "1,", "-2"] {
            assert!(parse_cpus(bad).is_err(), "accepted {bad:?}");
        }
    }
}
