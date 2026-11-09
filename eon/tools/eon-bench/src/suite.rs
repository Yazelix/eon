//! What every suite shares: the run settings and their flags, the load gate,
//! the environment block, and writing `result.json`.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    environment,
    json::Value,
    result::{Build, Environment, Grid, RunResult},
};

/// How often, and by default for how long, a sample waits for the load to
/// drop; the wait checks for an interrupt more often.
const LOAD_POLL: Duration = Duration::from_secs(5);
const INTERRUPT_POLL: Duration = Duration::from_millis(100);
const LOAD_WAIT_MINUTES: u64 = 10;

/// The help lines for the flags every suite takes.
pub fn settings_help() -> String {
    format!(
        "\
  --samples N      timed samples per cell (default 10; 3 with --quick)
  --warmups N      untimed rounds first (default 2; 1 with --quick)
  --max-load LOAD  wait before a sample while the 1-minute load is above LOAD
                   (default: online CPUs / 4, at least 1)
  --load-wait MINUTES
                   how long one such wait may last before the run fails
                   (default {LOAD_WAIT_MINUTES})
  --cpus LIST      pin the harness and what it starts to these CPUs, such as
                   2,3 or 4-7 (Linux)
  --quick          a seconds-long smoke run"
    )
}

pub struct Settings {
    pub out: PathBuf,
    pub samples: u64,
    pub warmups: u64,
    pub max_load: f64,
    pub load_wait: Duration,
    pub cpus: Option<Vec<u64>>,
    pub quick: bool,
}

/// Parses `--out` and the shared settings, and returns the suite's own flags,
/// those in `own` (each taking a value), in order. `--help` prints `help` and
/// ends with an empty error.
pub fn parse(
    arguments: &[String],
    usage: &str,
    help: impl Fn() -> String,
    own: &[&str],
) -> Result<(Settings, Vec<(String, String)>), String> {
    let (mut out, mut samples, mut warmups, mut max_load, mut cpus) =
        (None, None, None, None, None);
    let mut load_wait = None;
    let mut quick = false;
    let mut rest = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        if matches!(flag.as_str(), "--help" | "-h") {
            println!("{}", help());
            return Err(String::new());
        }
        if flag == "--quick" {
            quick = true;
            continue;
        }
        let shared = [
            "--out",
            "--samples",
            "--warmups",
            "--max-load",
            "--load-wait",
            "--cpus",
        ];
        if !shared.contains(&flag.as_str()) && !own.contains(&flag.as_str()) {
            return Err(format!("unknown argument {flag}\n{usage}"));
        }
        let Some(value) = arguments.next() else {
            return Err(format!("{flag} needs a value\n{usage}"));
        };
        let slot_taken = match flag.as_str() {
            "--out" => out.replace(PathBuf::from(value)).is_some(),
            "--samples" => samples.replace(number(flag, value)?).is_some(),
            "--warmups" => warmups.replace(number(flag, value)?).is_some(),
            "--max-load" => max_load
                .replace(
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|load| *load > 0.0 && load.is_finite())
                        .ok_or(format!("--max-load takes a positive number, not {value:?}"))?,
                )
                .is_some(),
            "--load-wait" => load_wait.replace(number(flag, value)?).is_some(),
            "--cpus" => cpus.replace(environment::parse_cpus(value)?).is_some(),
            _ => {
                rest.push((flag.clone(), value.clone()));
                false
            }
        };
        if slot_taken {
            return Err(format!("{flag} is repeated\n{usage}"));
        }
    }
    let samples = samples.unwrap_or(if quick { 3 } else { 10 });
    if samples == 0 {
        return Err("--samples must be at least 1".into());
    }
    let settings = Settings {
        out: out.ok_or(format!("--out is required\n{usage}"))?,
        samples,
        warmups: warmups.unwrap_or(if quick { 1 } else { 2 }),
        max_load: max_load.unwrap_or((environment::cpus_online() as f64 / 4.0).max(1.0)),
        load_wait: Duration::from_secs(60 * load_wait.unwrap_or(LOAD_WAIT_MINUTES)),
        cpus,
        quick,
    };
    Ok((settings, rest))
}

pub fn number(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} takes a whole number, not {value:?}"))
}

/// The 1-minute load once it is at most the settings' maximum, waiting up to
/// their `load_wait` for it to drop.
pub fn wait_for_load(suite: &str, settings: &Settings) -> Result<f64, String> {
    wait_for_load_until(suite, settings, Instant::now() + settings.load_wait)
}

pub(crate) fn wait_for_load_until(
    suite: &str,
    settings: &Settings,
    deadline: Instant,
) -> Result<f64, String> {
    let max = settings.max_load;
    let limit = settings
        .load_wait
        .min(deadline.saturating_duration_since(Instant::now()));
    let start = Instant::now();
    let mut said = false;
    loop {
        let load = environment::load()?;
        if load <= max {
            return Ok(load);
        }
        if start.elapsed() >= limit {
            return Err(format!(
                "the 1-minute load stayed above {max} for {} minutes (now {load:.2})",
                limit.as_secs() / 60
            ));
        }
        if !said {
            eprintln!("eon-bench {suite}: load {load:.2} is above {max}; waiting");
            said = true;
        }
        let poll = Instant::now();
        while poll.elapsed() < LOAD_POLL && start.elapsed() < limit {
            interrupt::check()?;
            std::thread::sleep(INTERRUPT_POLL.min(limit.saturating_sub(start.elapsed())));
        }
    }
}

/// The run's environment, read at its end.
pub fn environment(
    settings: &Settings,
    kernel: String,
    load_start: f64,
    grid: Grid,
    tools: Vec<(String, String)>,
    build: Build,
) -> Result<Environment, String> {
    let cpus_allowed = environment::cpus_allowed();
    Ok(Environment {
        os: environment::os(),
        cpu: environment::cpu_model(),
        cpus_online: environment::cpus_online(),
        pinned_cpus: settings.cpus.clone(),
        governor: cpus_allowed.as_deref().and_then(environment::governor),
        cpus_allowed,
        kernel,
        load_start,
        load_end: environment::load()?,
        max_load: settings.max_load,
        grid,
        tools,
        build,
    })
}

/// Writes `OUT/result.json` and returns what the file holds, so a table made
/// from it shows what was recorded.
pub fn write(out: &Path, result: &RunResult) -> Result<RunResult, String> {
    let path = out.join("result.json");
    let failure = |error: String| format!("{}: {error}", path.display());
    std::fs::write(&path, result.to_json().to_pretty())
        .map_err(|error| failure(error.to_string()))?;
    let text = std::fs::read_to_string(&path).map_err(|error| failure(error.to_string()))?;
    RunResult::from_json(&Value::parse(&text).map_err(failure)?).map_err(failure)
}

/// Which round this is, for progress lines: (`warmup` or `sample`, its
/// number, the count of its kind).
pub fn round_label(settings: &Settings, round: u64) -> (&'static str, u64, u64) {
    if round < settings.warmups {
        ("warmup", round + 1, settings.warmups)
    } else {
        ("sample", round - settings.warmups + 1, settings.samples)
    }
}

/// SIGINT, SIGTERM and SIGHUP while a suite that installs this runs: the first
/// only records the signal, which the suite's waits check, so the suite
/// cleans up and fails; a second takes the default action.
pub mod interrupt {
    use std::sync::atomic::{AtomicI32, Ordering};

    static SIGNAL: AtomicI32 = AtomicI32::new(0);

    extern "C" fn record(signal: libc::c_int) {
        SIGNAL.store(signal, Ordering::SeqCst);
    }

    pub fn install() -> Result<(), String> {
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: the handler only stores to an atomic, and the action is
            // fully initialized before sigaction reads it.
            let result = unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = record as extern "C" fn(libc::c_int) as libc::sighandler_t;
                action.sa_flags = libc::SA_RESETHAND | libc::SA_RESTART;
                libc::sigemptyset(&raw mut action.sa_mask);
                libc::sigaction(signal, &raw const action, std::ptr::null_mut())
            };
            if result != 0 {
                return Err(format!("sigaction: {}", std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }

    /// Fails once a signal has arrived.
    pub fn check() -> Result<(), String> {
        match SIGNAL.load(Ordering::SeqCst) {
            0 => Ok(()),
            signal => Err(format!("interrupted by signal {signal}")),
        }
    }

    /// Ends the process by the signal that interrupted it, if one did, so
    /// its parent sees that signal.
    pub fn resend() {
        let signal = SIGNAL.load(Ordering::SeqCst);
        if signal != 0 {
            // SAFETY: restoring the default action and raising a signal have
            // no preconditions.
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
                libc::raise(signal);
            }
        }
    }
}
