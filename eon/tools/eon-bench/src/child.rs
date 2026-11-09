//! `eon-bench child DIR --rows R --cols C [--setup FILE] [--stdout FILE] --
//! PROGRAM ARGS...`: the process a terminal under test runs, which holds its
//! workload until the harness is ready and keeps the terminal alive after it.
//!
//! It waits until its terminal reports the grid, has the completion writer
//! send the setup FILE when given, so the terminal has handled it, writes
//! `DIR/ready` with its PID, and waits for `DIR/go`; then it runs PROGRAM (stdout to FILE when
//! given) as its own child, writes PROGRAM's exit code to `DIR/done`, and
//! waits for `DIR/release` before exiting with that code. The harness thus
//! starts the workload only once attachment has settled, and reads the
//! terminal's processes after the workload ends but before the terminal
//! does. Files appear whole: each is written aside and renamed.

use std::{
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const USAGE: &str = "usage: eon-bench child DIR --rows R --cols C [--setup FILE] [--release-wait SECONDS] \
                         [--stdout FILE] [--grid-report FILE] -- PROGRAM [ARGS...]";

/// How long the child waits for the grid, the go and the release.
const GRID_WAIT: Duration = Duration::from_secs(30);
const GO_WAIT: Duration = Duration::from_secs(600);
const RELEASE_WAIT: Duration = Duration::from_secs(60);

pub struct Options {
    dir: PathBuf,
    rows: u16,
    cols: u16,
    setup: Option<PathBuf>,
    stdout: Option<PathBuf>,
    grid_report: Option<PathBuf>,
    release_wait: Duration,
    program: Vec<String>,
}

pub fn options(arguments: &[String]) -> Result<Options, String> {
    let split = arguments
        .iter()
        .position(|argument| argument == "--")
        .ok_or(USAGE)?;
    let (flags, program) = (&arguments[..split], &arguments[split + 1..]);
    let [dir, rest @ ..] = flags else {
        return Err(USAGE.into());
    };
    let (mut rows, mut cols, mut setup, mut stdout) = (None, None, None, None);
    let mut release_wait = None;
    let mut grid_report = None;
    for pair in rest.chunks(2) {
        let [flag, value] = pair else {
            return Err(USAGE.into());
        };
        let size = || {
            value
                .parse::<u16>()
                .ok()
                .filter(|size| *size > 0)
                .ok_or(format!("{flag} takes a size, not {value:?}"))
        };
        let taken = match flag.as_str() {
            "--rows" => rows.replace(size()?).is_some(),
            "--cols" => cols.replace(size()?).is_some(),
            "--setup" => setup.replace(PathBuf::from(value)).is_some(),
            "--stdout" => stdout.replace(PathBuf::from(value)).is_some(),
            "--grid-report" => grid_report.replace(PathBuf::from(value)).is_some(),
            "--release-wait" => release_wait
                .replace(Duration::from_secs(crate::suite::number(flag, value)?))
                .is_some(),
            _ => return Err(USAGE.into()),
        };
        if taken {
            return Err(format!("{flag} is repeated\n{USAGE}"));
        }
    }
    if program.is_empty() {
        return Err(USAGE.into());
    }
    Ok(Options {
        dir: PathBuf::from(dir),
        rows: rows.ok_or(USAGE)?,
        cols: cols.ok_or(USAGE)?,
        setup,
        stdout,
        grid_report,
        release_wait: release_wait.unwrap_or(RELEASE_WAIT),
        program: program.to_vec(),
    })
}

pub fn run(options: &Options) -> Result<u8, String> {
    let start = Instant::now();
    loop {
        let actual = grid();
        if let Some(path) = &options.grid_report {
            publish(path, &format!("{actual:?}"))?;
        }
        if actual == Some((options.rows, options.cols)) {
            break;
        }
        if start.elapsed() >= GRID_WAIT {
            return Err("no matching terminal grid within 30s".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some(setup) = &options.setup {
        let exe = std::env::current_exe().map_err(|error| format!("eon-bench's path: {error}"))?;
        let status = Command::new(exe)
            .arg("write")
            .arg(setup)
            .arg("--report")
            .arg(options.dir.join("setup.json"))
            .status()
            .map_err(|error| format!("the setup's writer: {error}"))?;
        if !status.success() {
            return Err(format!("the setup's writer exited {status}"));
        }
    }
    publish(&options.dir.join("ready"), &std::process::id().to_string())?;
    let go = options.dir.join("go");
    let start = Instant::now();
    let mut previous = grid();
    while !go.exists() {
        let actual = grid();
        if let Some(path) = &options.grid_report
            && actual != previous
        {
            publish(path, &format!("{actual:?}"))?;
        }
        previous = actual;
        if start.elapsed() >= GO_WAIT {
            return Err("no go within 600s".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some(path) = &options.grid_report {
        let actual = grid();
        publish(&path.with_extension("go"), &format!("{actual:?}"))?;
        if actual != Some((options.rows, options.cols)) {
            publish(&options.dir.join("done"), "1")?;
            return Err(format!("the terminal's grid changed before go: {actual:?}"));
        }
    }
    let mut command = Command::new(&options.program[0]);
    command.args(&options.program[1..]);
    if let Some(path) = &options.stdout {
        let file =
            std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
        command.stdout(Stdio::from(file));
    }
    let status = command
        .status()
        .map_err(|error| format!("{}: {error}", options.program[0]))?;
    if let Some(path) = &options.grid_report {
        let actual = grid();
        publish(&path.with_extension("after"), &format!("{actual:?}"))?;
        if actual != Some((options.rows, options.cols)) {
            publish(&options.dir.join("done"), "1")?;
            return Err(format!(
                "the terminal's grid changed during the workload: {actual:?}"
            ));
        }
    }
    // A signal counts as a failure.
    let code = status
        .code()
        .map_or(1, |code| u8::try_from(code).unwrap_or(1));
    publish(&options.dir.join("done"), &code.to_string())?;
    let release = options.dir.join("release");
    // Past the wait the harness is gone; exit anyway.
    let _ = wait_until(options.release_wait, "the release", || release.exists());
    Ok(code)
}

/// The rows and columns stdin's terminal reports.
fn grid() -> Option<(u16, u16)> {
    // SAFETY: winsize is plain data that TIOCGWINSZ fills.
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    // SAFETY: stdin is open for the process's life and size is a valid winsize.
    let result = unsafe {
        libc::ioctl(
            std::io::stdin().as_raw_fd(),
            libc::TIOCGWINSZ,
            &raw mut size,
        )
    };
    (result == 0).then_some((size.ws_row, size.ws_col))
}

fn wait_until(limit: Duration, what: &str, done: impl Fn() -> bool) -> Result<(), String> {
    let start = Instant::now();
    while !done() {
        if start.elapsed() >= limit {
            return Err(format!("no {what} within {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Writes `text` to `path` whole: aside first, then renamed into place.
pub fn publish(path: &Path, text: &str) -> Result<(), String> {
    let aside = path.with_extension("partial");
    std::fs::write(&aside, text)
        .and_then(|()| std::fs::rename(&aside, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}
