use super::{Policy, SECOND, Ticket, now_ns, proc_start};
use crate::config::SlotConfig;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MINUTE: u64 = 60 * SECOND;

pub(super) fn policy(config: &SlotConfig) -> io::Result<Policy<'_>> {
    let dir = config.legacy_dir.as_deref();
    let read = |name| dir.and_then(|dir| fs::read_to_string(dir.join(name)).ok());
    let number = |name| read(name).and_then(|text| text.trim().parse::<u64>().ok());
    let quiet_until = match dir.map(|dir| fs::read_to_string(dir.join("quiet-until"))) {
        None => 0,
        Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => 0,
        Some(result) => {
            let text = result?;
            let invalid =
                || io::Error::new(io::ErrorKind::InvalidData, "invalid quiet-until policy");
            if !text.trim().bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            text.trim().parse().map_err(|_| invalid())?
        }
    };
    Ok(Policy {
        config,
        extra_until: number("extra-slot-until").unwrap_or_default(),
        one_until: number("one-slot-until").unwrap_or_default(),
        quiet_until,
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
    })
}

fn boot_time() -> io::Result<(u64, u64)> {
    // /proc/PID/stat expresses start time in clock ticks since boot.
    let now = now_ns();
    let uptime = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime)?;
    let ticks = rustix::param::clock_ticks_per_second() as i64;
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
        let recorded_start = fs::read_to_string(entry.path())
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok());
        if recorded_start.is_some_and(|recorded| recorded != start)
            || (recorded_start.is_none() && boot.saturating_add(start_ns) > queued_ns)
        {
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
            remote: false,
            fallback_reason: None,
            legacy: true,
            malformed: false,
        });
    }
    tickets
}
