use super::{Policy, SECOND, Ticket, now_ns, proc_start};
use crate::config::SlotConfig;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MINUTE: u64 = 60 * SECOND;

pub(super) fn policy(config: &SlotConfig) -> Policy<'_> {
    let dir = config.legacy_dir.as_deref();
    let read = |name| dir.and_then(|dir| fs::read_to_string(dir.join(name)).ok());
    let number = |name| read(name).and_then(|text| text.trim().parse::<u64>().ok());
    Policy {
        config,
        extra_until: number("extra-slot-until").unwrap_or_default(),
        one_until: number("one-slot-until").unwrap_or_default(),
        quiet_until: number("quiet-until").unwrap_or_default(),
        stalled: dir.is_some_and(|dir| dir.join("slot2-stalled").exists()),
        yield_roots: read("yield-roots").map_or_else(Vec::new, |text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .collect()
        }),
        yield_ns: read("yield-max-minutes")
            .map_or(30, |text| {
                let text = text.trim();
                if text.is_empty() {
                    30
                } else {
                    text.parse::<u64>().unwrap_or_default()
                }
            })
            .saturating_mul(MINUTE),
    }
}

fn boot_time() -> io::Result<(u64, u64)> {
    let mut uptime = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // /proc/PID/stat expresses start time in clock ticks since boot.
    let now = now_ns();
    let result = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut uptime) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 {
        return Err(io::Error::last_os_error());
    }
    let uptime_ns = (uptime.tv_sec as u64)
        .saturating_mul(SECOND)
        .saturating_add(uptime.tv_nsec as u64);
    Ok((now.saturating_sub(uptime_ns), ticks as u64))
}

pub(super) fn tickets(dir: Option<&Path>) -> Vec<Ticket> {
    let Some(entries) = dir.and_then(|dir| fs::read_dir(dir.join("queue")).ok()) else {
        return Vec::new();
    };
    let Ok((boot, ticks)) = boot_time() else {
        return Vec::new();
    };
    let mut tickets = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let mut fields = name.split('.');
        let (Some(Ok(queued_ns)), Some(Ok(pid)), Some(class @ ("run" | "nix")), None) = (
            fields.next().map(str::parse::<u64>),
            fields.next().map(str::parse::<u32>),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        let Ok(Some(start)) = proc_start(pid) else {
            continue;
        };
        let start_ns = (u128::from(start) * u128::from(SECOND) / u128::from(ticks))
            .try_into()
            .unwrap_or(u64::MAX);
        if boot.saturating_add(start_ns) > queued_ns {
            continue;
        }
        tickets.push(Ticket {
            queued_ns,
            pid,
            class: class.into(),
            cwd: fs::read_link(format!("/proc/{pid}/cwd"))
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            argv: Vec::new(),
            front: false,
            quiet_exempt: false,
            legacy: true,
            malformed: false,
        });
    }
    tickets
}
