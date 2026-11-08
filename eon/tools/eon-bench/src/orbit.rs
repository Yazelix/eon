//! The `orbit` suite: Orbit's throughput to completion, detached and with an
//! attached client that decodes and discards every message, with counters
//! that say where the time goes.
//!
//! Each sample starts a fresh `yazelix-orbit serve` whose child is
//! `eon-bench child` (see `child.rs`). The harness attaches with the
//! `orbit-protocol` codec, sets the grid, and, detached, closes its client
//! and waits until Orbit has dropped it; then it starts the workload. The
//! completion writer's time (first write to the status reply) is the
//! sample's time; a kitten workload's time is kitten's own. Orbit dies with
//! the harness (a parent-death signal on Linux), and SIGINT, SIGTERM or
//! SIGHUP ends the run through the same teardown as a failed sample. Counters come
//! from the wire (frames, bytes, revisions) and, on Linux, from `/proc`
//! (CPU time and voluntary context switches of Orbit's host and its Bend
//! worker, taken while the child holds the session open after the workload).
//! Orbit itself is not changed or instrumented.

use std::{
    io::{BufReader, Read, Write},
    net::Shutdown,
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime},
};

use orbit_protocol::session::{
    ClientMessage, HEADER_BYTES, ServerMessage, SurfaceSize, decode_server_message,
    encode_client_message, server_message_len,
};

use crate::{
    child::publish,
    corpus,
    environment::{self, monotonic_ns},
    json::Value,
    kitten,
    result::{Cell, Grid, RunResult, Sample, utc_timestamp},
    sha256::Sha256,
    stats::Summary,
    suite::{self, Settings},
    vtebench,
};

pub const USAGE: &str = "\
usage: eon-bench orbit --out DIR [--orbit PATH] [--mode detached|attached]...
                       [--grid ROWSxCOLS] [--only ID]... [--vtebench DIR]
                       [--vtebench-bytes N] [--kitten PATH]
                       [--kitten-repetitions N]
                       [--deadline SECONDS] [--samples N] [--warmups N]
                       [--max-load LOAD] [--load-wait MINUTES] [--cpus LIST]
                       [--quick]
       eon-bench orbit --help";

pub fn help() -> String {
    format!(
        "\
eon-bench orbit: Orbit's throughput to completion, detached and attached

  --out DIR        writes DIR/corpus/, DIR/vtebench-<ROWS>x<COLS>/ and
                   DIR/result.json; created if missing
  --orbit PATH     the yazelix-orbit to run (default: $EON_BENCH_ORBIT, which
                   `nix run .#bench` sets to the flake's release Orbit)
  --mode MODE      detached or attached; repeat for both (default: both)
  --grid RxC       the terminal's rows and columns (default 24x80; the
                   `cursor` corpus addresses 24x80)
  --only ID        run only this workload; repeat for several (default: all):
                   the corpora ({}), vtebench-<name> for vtebench's
                   benchmarks and kitten-<name> for kitten's ({})
  --vtebench DIR   vtebench's benchmarks/ directory (default:
                   $EON_BENCH_VTEBENCH); its scripts run with
                   $EON_BENCH_VTEBENCH_PATH, when set, in front of PATH
  --vtebench-bytes N
                   repeat each vtebench benchmark to at least N bytes, as
                   vtebench's --min-bytes (default {})
  --kitten PATH    kitty's kitten (default: $EON_BENCH_KITTEN)
  --kitten-repetitions N
                   kitten's --repetitions (default {}, the pinned kitten's; 20
                   with --quick)
  --deadline SECONDS
                   kill Orbit, and with it the workload, when a sample has not
                   finished in this time (default 300; 60 with --quick)
{}

Each sample runs a fresh Orbit at the grid with `eon-bench child` as its
child. The harness attaches (Hello, Resize) with the orbit-protocol codec and
decodes and discards every message from then on. Once the child has the grid
and has sent any setup, and the client has read nothing for 100 ms (at most
2 s, or the sample fails), detached closes the client and waits until Orbit
has dropped it, and attached starts counting. Then the child starts the
workload: the completion writer for a corpus or a vtebench file, or kitten.
Attached, the counts end when the client has again read nothing for 100 ms,
at most 2 s after the workload. A vtebench
workload is a vtebench sample at the grid: a reset and the setup, which the
child sends untimed before the go, and the benchmark repeated to
--vtebench-bytes, which is timed.

Time is the completion writer's, from its first write to Orbit's reply to its
status query, or kitten's own. Throughput is MB (10^6 bytes) per second.
Counters per sample: attach_us (handshake, outside the time); attached,
setup_frames and setup_last_revision (read before the go, not counted after),
frames, frame_bytes, other_bytes, first_revision, last_revision,
revision_gaps (frames whose revision skipped one: Orbit replaced a queued
frame), revision_repeats, frames_after_reply, last_frame_after_reply_us,
reply_after_last_frame_us and client_cpu_us; on Linux, host_cpu_us,
worker_cpu_us, host_context_switches and worker_context_switches (CPU time
and voluntary context switches of Orbit's host and its Bend worker from the
start of the workload until it ended and the client went quiet). Rounds run
every workload in every mode, so they interleave.

Orbit and the commands the harness runs to prepare (kitten --version, the
vtebench scripts, the environment probes) get SIGKILL if the harness dies
(Linux). Once samples start, SIGINT,
SIGTERM or SIGHUP stops the run within about a second: the current sample's
Orbit is killed, its directory removed, no result.json is kept, and the
harness then ends by that signal (before then, the signal's default action
ends it). Exits 0 on success, 1 when a sample
fails, and 2 for a usage error.",
        corpus::CORPORA.map(|corpus| corpus.id).join(", "),
        kitten::BENCHMARKS.join(", "),
        vtebench::MIN_BYTES,
        kitten::REPETITIONS,
        suite::settings_help(),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Detached,
    Attached,
}

impl Mode {
    fn layer(self) -> &'static str {
        match self {
            Self::Detached => "orbit-detached",
            Self::Attached => "orbit-attached",
        }
    }
}

pub struct Options {
    settings: Settings,
    orbit: PathBuf,
    modes: Vec<Mode>,
    rows: u16,
    cols: u16,
    corpora: Vec<&'static corpus::Corpus>,
    vtebench: Vec<&'static str>,
    kitten: Vec<&'static str>,
    vtebench_dir: Option<PathBuf>,
    vtebench_bytes: u64,
    kitten_path: Option<PathBuf>,
    kitten_repetitions: u64,
    deadline: Duration,
}

/// `--help` prints the help and ends with an empty error.
pub fn options(arguments: &[String]) -> Result<Options, String> {
    let own = [
        "--orbit",
        "--mode",
        "--grid",
        "--only",
        "--vtebench",
        "--vtebench-bytes",
        "--kitten",
        "--kitten-repetitions",
        "--deadline",
    ];
    let (settings, own) = suite::parse(arguments, USAGE, help, &own)?;
    let (mut orbit, mut grid, mut vtebench_dir, mut kitten_path) = (None, None, None, None);
    let (mut repetitions, mut deadline, mut vtebench_bytes) = (None, None, None);
    let mut modes = Vec::new();
    let (mut corpora, mut vtebench, mut kitten) = (Vec::new(), Vec::new(), Vec::new());
    for (flag, value) in &own {
        let taken = match flag.as_str() {
            "--orbit" => orbit.replace(PathBuf::from(value)).is_some(),
            "--grid" => grid.replace(parse_grid(value)?).is_some(),
            "--vtebench" => vtebench_dir.replace(PathBuf::from(value)).is_some(),
            "--kitten" => kitten_path.replace(PathBuf::from(value)).is_some(),
            "--vtebench-bytes" => vtebench_bytes
                .replace(suite::number(flag, value)?.max(1))
                .is_some(),
            "--kitten-repetitions" => repetitions
                .replace(suite::number(flag, value)?.max(1))
                .is_some(),
            "--deadline" => deadline
                .replace(suite::number(flag, value)?.max(1))
                .is_some(),
            "--mode" => {
                let mode = match value.as_str() {
                    "detached" => Mode::Detached,
                    "attached" => Mode::Attached,
                    _ => return Err(format!("--mode is detached or attached, not {value:?}")),
                };
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
                false
            }
            _ => {
                if let Some(name) = value.strip_prefix(vtebench::PREFIX) {
                    add(&mut vtebench, &vtebench::BENCHMARKS, name, value)?;
                } else if let Some(name) = value.strip_prefix(kitten::PREFIX) {
                    add(&mut kitten, &kitten::BENCHMARKS, name, value)?;
                } else {
                    corpus::add(&mut corpora, value)?;
                }
                false
            }
        };
        if taken {
            return Err(format!("{flag} is repeated\n{USAGE}"));
        }
    }
    if corpora.is_empty() && vtebench.is_empty() && kitten.is_empty() {
        corpora = corpus::CORPORA.iter().collect();
        vtebench = vtebench::BENCHMARKS.to_vec();
        kitten = kitten::BENCHMARKS.to_vec();
    }
    if modes.is_empty() {
        modes = vec![Mode::Detached, Mode::Attached];
    }
    let from_env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let orbit = orbit
        .or_else(|| from_env("EON_BENCH_ORBIT"))
        .ok_or("no Orbit: pass --orbit PATH or set EON_BENCH_ORBIT")?;
    let vtebench_dir = vtebench_dir.or_else(|| from_env("EON_BENCH_VTEBENCH"));
    if !vtebench.is_empty() && vtebench_dir.is_none() {
        return Err(
            "no vtebench: pass --vtebench DIR, set EON_BENCH_VTEBENCH, or --only \
             other workloads"
                .into(),
        );
    }
    let kitten_path = kitten_path.or_else(|| from_env("EON_BENCH_KITTEN"));
    if !kitten.is_empty() && kitten_path.is_none() {
        return Err(
            "no kitten: pass --kitten PATH, set EON_BENCH_KITTEN, or --only other workloads".into(),
        );
    }
    let (rows, cols) = grid.unwrap_or((24, 80));
    let quick = settings.quick;
    Ok(Options {
        settings,
        orbit,
        modes,
        rows,
        cols,
        corpora,
        vtebench,
        kitten,
        vtebench_dir,
        vtebench_bytes: vtebench_bytes.unwrap_or(vtebench::MIN_BYTES),
        kitten_path,
        kitten_repetitions: repetitions.unwrap_or(if quick { 20 } else { kitten::REPETITIONS }),
        deadline: Duration::from_secs(deadline.unwrap_or(if quick { 60 } else { 300 })),
    })
}

fn add(
    chosen: &mut Vec<&'static str>,
    known: &[&'static str],
    name: &str,
    id: &str,
) -> Result<(), String> {
    let name = known.iter().find(|known| **known == name).ok_or(format!(
        "no workload {id}; `eon-bench orbit --help` lists them"
    ))?;
    if !chosen.contains(name) {
        chosen.push(name);
    }
    Ok(())
}

fn parse_grid(value: &str) -> Result<(u16, u16), String> {
    value
        .split_once('x')
        .and_then(|(rows, cols)| Some((rows.parse().ok()?, cols.parse().ok()?)))
        .filter(|&(rows, cols): &(u16, u16)| rows > 0 && cols > 0)
        .ok_or(format!(
            "--grid takes ROWSxCOLS, such as 24x80, not {value:?}"
        ))
}

/// What a sample runs inside Orbit.
enum Workload {
    /// The completion writer with this file, after the setup file if any.
    Writer(PathBuf, Option<PathBuf>),
    /// One kitten benchmark.
    Kitten(&'static str),
}

pub fn run(options: &Options) -> Result<(), String> {
    let outcome = run_suite(options);
    // A signal makes the run incomplete however it ended: no result stays,
    // and the harness ends by that signal.
    if let Err(interrupted) = suite::interrupt::check() {
        match &outcome {
            Ok(()) => {
                let _ = std::fs::remove_file(options.settings.out.join("result.json"));
                eprintln!("eon-bench: {interrupted}; result.json removed");
            }
            Err(error) => eprintln!("eon-bench: {error}"),
        }
        suite::interrupt::resend();
        return Err(interrupted);
    }
    outcome
}

fn run_suite(options: &Options) -> Result<(), String> {
    let settings = &options.settings;
    if let Some(cpus) = &settings.cpus {
        environment::pin(cpus)?;
    }
    let started_at = utc_timestamp(SystemTime::now());
    let load_start = environment::load()?;
    let kernel = environment::kernel();
    let build = environment::build()?;
    let orbit_sha256 = file_sha256(&options.orbit)?;
    let mut tools = vec![
        ("eon-bench".to_owned(), env!("CARGO_PKG_VERSION").to_owned()),
        (
            "yazelix-orbit".to_owned(),
            format!("{} sha256:{orbit_sha256}", options.orbit.display()),
        ),
    ];
    let mut workloads: Vec<(Cell, Workload)> = Vec::new();
    let cell = |workload: String, sha256: String, input_bytes: u64| Cell {
        workload,
        corpus_sha256: sha256,
        input_bytes,
        layer: String::new(),
        subject: "yazelix-orbit".into(),
        subject_version: orbit_sha256.clone(),
        samples: Vec::new(),
    };
    let corpus_dir = settings.out.join("corpus");
    corpus::write_files(&corpus_dir, &options.corpora)?;
    for corpus in &options.corpora {
        workloads.push((
            cell(corpus.id.into(), corpus.sha256.into(), corpus.length),
            Workload::Writer(corpus_dir.join(corpus.file_name()), None),
        ));
    }
    if let Some(dir) = options
        .vtebench_dir
        .as_ref()
        .filter(|_| !options.vtebench.is_empty())
    {
        let out = settings
            .out
            .join(format!("vtebench-{}x{}", options.rows, options.cols));
        // The bench package names the scripts' tools; the rest of PATH
        // stays behind them.
        let path = std::env::var_os("EON_BENCH_VTEBENCH_PATH").map(|mut path| {
            if let Some(rest) = std::env::var_os("PATH") {
                path.push(":");
                path.push(rest);
            }
            path
        });
        let generated = vtebench::generate(
            dir,
            &options.vtebench,
            &out,
            (options.rows, options.cols),
            options.vtebench_bytes,
            path.as_ref(),
        )?;
        for workload in generated {
            workloads.push((
                cell(workload.id, workload.sha256, workload.length),
                Workload::Writer(workload.path, Some(workload.setup)),
            ));
        }
        tools.push(("vtebench".into(), dir.display().to_string()));
        tools.push(("vtebench-bytes".into(), options.vtebench_bytes.to_string()));
    }
    if let Some(path) = options
        .kitten_path
        .as_ref()
        .filter(|_| !options.kitten.is_empty())
    {
        for name in &options.kitten {
            // Bytes are known only from kitten's report; set from the samples.
            workloads.push((
                cell(format!("{}{name}", kitten::PREFIX), String::new(), 0),
                Workload::Kitten(name),
            ));
        }
        tools.push(("kitten".into(), kitten_version(path)?));
        tools.push((
            "kitten-repetitions".into(),
            options.kitten_repetitions.to_string(),
        ));
    }
    let mut cells: Vec<(Cell, &Workload, Mode)> = Vec::new();
    for (cell, workload) in &workloads {
        for &mode in &options.modes {
            let mut cell = cell.clone();
            cell.layer = mode.layer().into();
            cells.push((cell, workload, mode));
        }
    }
    let exe = std::env::current_exe().map_err(|error| format!("eon-bench's path: {error}"))?;
    // Until the first sample starts nothing needs cleaning up, so a signal
    // takes its default action during the preparation above.
    suite::interrupt::install()?;
    let mut sessions = 0;
    for round in 0..settings.warmups + settings.samples {
        let (kind, number, of) = suite::round_label(settings, round);
        eprintln!("eon-bench orbit: {kind} {number} of {of}");
        for (cell, workload, mode) in &mut cells {
            suite::interrupt::check()?;
            let load = suite::wait_for_load("orbit", settings)?;
            sessions += 1;
            let spec = Spec {
                exe: &exe,
                options,
                workload,
                mode: *mode,
                dir: run_dir(sessions)?,
            };
            let outcome = sample(&spec)
                .map_err(|error| format!("{} {}: {error}", cell.workload, cell.layer))?;
            if kind == "sample" {
                let mut counters = outcome.counters;
                if let Some(bytes) = outcome.kitten_bytes {
                    counters.push(("kitten_bytes".into(), bytes));
                }
                cell.samples.push(Sample {
                    seconds: outcome.seconds,
                    load,
                    counters,
                });
            }
        }
    }
    let mut cells: Vec<Cell> = cells.into_iter().map(|(cell, _, _)| cell).collect();
    for cell in &mut cells {
        if cell.workload.starts_with(kitten::PREFIX) {
            // kitten's data sizes are fixed per benchmark but for `csi`,
            // which varies by a few bytes per MiB; the median stands for it.
            let bytes: Vec<f64> = cell
                .samples
                .iter()
                .flat_map(|sample| &sample.counters)
                .filter(|(name, _)| name == "kitten_bytes")
                .map(|&(_, bytes)| bytes as f64)
                .collect();
            cell.input_bytes = Summary::of(&bytes).map_or(0, |bytes| bytes.median.round() as u64);
        }
    }
    let grid = Grid {
        cols: options.cols.into(),
        rows: options.rows.into(),
    };
    let result = RunResult {
        suite: "orbit".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: settings.quick,
        corpus_version: corpus::CORPUS_VERSION.into(),
        warmups: settings.warmups,
        samples: settings.samples,
        started_at,
        finished_at: utc_timestamp(SystemTime::now()),
        environment: suite::environment(settings, kernel, load_start, grid, tools, build)?,
        cells,
    };
    suite::interrupt::check()?;
    let written = suite::write(&settings.out, &result)?;
    println!("{}", table(&written));
    println!("result: {}", settings.out.join("result.json").display());
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher.hex())
}

fn kitten_version(path: &Path) -> Result<String, String> {
    let output = environment::die_with_parent(Command::new(path).arg("--version"))
        .output()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!(
        "{} {}",
        path.display(),
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

/// A private directory for one session, short enough for its socket path.
fn run_dir(session: u64) -> Result<TempDir, String> {
    let path = std::env::temp_dir().join(format!("eb{}-{session}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(TempDir(path))
}

/// Removes the session's directory however the sample ends.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Spec<'a> {
    exe: &'a Path,
    options: &'a Options,
    workload: &'a Workload,
    mode: Mode,
    dir: TempDir,
}

struct Outcome {
    seconds: f64,
    kitten_bytes: Option<u64>,
    counters: Vec<(String, u64)>,
}

/// How long the harness waits for Orbit's socket, the handshake, the child's
/// grid, a detach, the client to go quiet, and Orbit to exit.
const SETUP_WAIT: Duration = Duration::from_secs(10);
/// How long one handshake read blocks before it checks for an interrupt.
const READ_POLL: Duration = Duration::from_millis(100);
const QUIET: Duration = Duration::from_millis(100);
const QUIET_WAIT: Duration = Duration::from_secs(2);

/// Orbit, killed by its exact PID when dropped while running; the hangup
/// then ends its child and the workload.
struct Server {
    child: Child,
    log: PathBuf,
}

impl Server {
    fn alive(&mut self) -> Result<(), String> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(format!("Orbit exited early, {status}{}", self.log_tail())),
            Err(error) => Err(format!("Orbit: {error}")),
        }
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let log = log.trim();
        if log.is_empty() {
            String::new()
        } else {
            let start = log.len() - log.len().min(2000);
            format!("; its output: {}", &log[log.ceil_char_boundary(start)..])
        }
    }

    /// Waits until `done`, failing when Orbit exits or `limit` passes.
    fn wait_for(
        &mut self,
        limit: Duration,
        what: &str,
        mut done: impl FnMut() -> bool,
    ) -> Result<(), String> {
        let start = Instant::now();
        while !done() {
            self.alive()?;
            suite::interrupt::check()?;
            if start.elapsed() >= limit {
                return Err(format!("no {what} within {} s", limit.as_secs_f64()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn sample(spec: &Spec) -> Result<Outcome, String> {
    let options = spec.options;
    let dir = &spec.dir.0;
    let socket = dir.join("s");
    let report = dir.join("report.json");
    let kitten_out = dir.join("kitten.txt");
    let log = dir.join("orbit.log");
    let mut command = Command::new(&options.orbit);
    command
        .arg("serve")
        .arg(&socket)
        .arg("--")
        .arg(spec.exe)
        .arg("child")
        .arg(dir)
        .args(["--rows", &options.rows.to_string()])
        .args(["--cols", &options.cols.to_string()]);
    match spec.workload {
        Workload::Writer(file, setup) => {
            if let Some(setup) = setup {
                command.arg("--setup").arg(setup);
            }
            command
                .arg("--")
                .arg(spec.exe)
                .arg("write")
                .arg(file)
                .arg("--report")
                .arg(&report);
        }
        Workload::Kitten(name) => {
            let kitten = options.kitten_path.as_ref().ok_or("no kitten")?;
            command
                .arg("--stdout")
                .arg(&kitten_out)
                .arg("--")
                .arg(kitten)
                .arg("__benchmark__")
                .arg("--repetitions")
                .arg(options.kitten_repetitions.to_string())
                .arg(name);
        }
    }
    let log_file =
        std::fs::File::create(&log).map_err(|error| format!("{}: {error}", log.display()))?;
    let log_copy = log_file
        .try_clone()
        .map_err(|error| format!("{}: {error}", log.display()))?;
    environment::die_with_parent(&mut command);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_copy))
        .spawn()
        .map_err(|error| format!("{}: {error}", options.orbit.display()))?;
    let mut server = Server { child, log };
    server.wait_for(SETUP_WAIT, "Orbit socket", || socket.exists())?;
    let attach_start = monotonic_ns();
    let stream = UnixStream::connect(&socket).map_err(|error| format!("connect: {error}"))?;
    let mut client = Client::new(stream)?;
    client.attach(options.rows, options.cols)?;
    let attach_us = (monotonic_ns() - attach_start) / 1000;
    // The client reads from here on in both modes, so the frames of a
    // setup never back up behind it, and both modes start from the same
    // state: the setup handled and its frames read.
    let reader = client.discard_in_background()?;
    let ready = dir.join("ready");
    server.wait_for(SETUP_WAIT, "grid at the child", || ready.exists())?;
    let child_pid: u32 = read_number(&ready)?;
    if !reader.wait_quiet() {
        return Err(format!(
            "frames still arrived {} s after the setup",
            QUIET_WAIT.as_secs()
        ));
    }
    let reader = match spec.mode {
        Mode::Attached => {
            reader.restart();
            Some(reader)
        }
        Mode::Detached if cfg!(target_os = "linux") => {
            // A path no line matches would read as dropped at once.
            if procfs::connections(&socket).unwrap_or(0) == 0 {
                return Err("the client's connection is not in /proc/net/unix".into());
            }
            reader.close()?;
            server.wait_for(SETUP_WAIT, "detach", || {
                procfs::connections(&socket) == Some(0)
            })?;
            None
        }
        Mode::Detached => {
            reader.close()?;
            // No portable way to see Orbit drop the client: wait past its
            // 100 ms wake.
            std::thread::sleep(Duration::from_millis(300));
            None
        }
    };
    let host_pid = server.child.id();
    let processes = procfs::Processes::find(host_pid, child_pid);
    let before = processes.as_ref().map(procfs::Processes::read);
    let client_before = reader.as_ref().and_then(|reader| reader.cpu_ns());
    publish(&dir.join("go"), "")?;
    let done = dir.join("done");
    server.wait_for(options.deadline, "end of the workload", || done.exists())?;
    if let Some(reader) = &reader
        && !reader.wait_quiet()
    {
        return Err(format!(
            "frames still arrived {} s after the workload ended",
            QUIET_WAIT.as_secs()
        ));
    }
    let after = processes.as_ref().map(procfs::Processes::read);
    let client_after = reader.as_ref().and_then(|reader| reader.cpu_ns());
    let code: u64 = read_number(&done)?;
    let (seconds, kitten_bytes, reply_ns) = match spec.workload {
        Workload::Writer(file, _) => {
            if code != 0 {
                return Err(format!("the writer exited {code}{}", server.log_tail()));
            }
            let (seconds, bytes, replied_ns) = writer_report(&report)?;
            let length = std::fs::metadata(file)
                .map_err(|error| format!("{}: {error}", file.display()))?
                .len();
            if bytes != length {
                return Err(format!("the writer sent {bytes} bytes of {length}"));
            }
            (seconds, None, Some(replied_ns))
        }
        Workload::Kitten(_) => {
            let output = std::fs::read_to_string(&kitten_out).unwrap_or_default();
            if code != 0 {
                return Err(format!(
                    "kitten exited {code}: {output}{}",
                    server.log_tail()
                ));
            }
            let reported = kitten::parse(&output)?;
            (reported.seconds, Some(reported.bytes), None)
        }
    };
    publish(&dir.join("release"), "")?;
    let start = Instant::now();
    let status = loop {
        match server.child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < SETUP_WAIT => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(None) => return Err("Orbit did not exit after its child".into()),
            Err(error) => return Err(format!("Orbit: {error}")),
        }
    };
    if !status.success() {
        return Err(format!("Orbit exited {status}{}", server.log_tail()));
    }
    let mut counters = vec![("attach_us".to_owned(), attach_us)];
    if let Some(reader) = reader {
        let received = reader.finish()?;
        counters.extend(received.counters(reply_ns));
        if let (Some(before), Some(after)) = (client_before, client_after) {
            counters.push(("client_cpu_us".into(), after.saturating_sub(before) / 1000));
        }
    }
    if let (Some(before), Some(after)) = (before, after) {
        counters.extend(after.since(&before));
    }
    Ok(Outcome {
        seconds,
        kitten_bytes,
        counters,
    })
}

fn read_number<T: std::str::FromStr>(path: &Path) -> Result<T, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    text.trim()
        .parse()
        .map_err(|_| format!("{}: not a number: {text:?}", path.display()))
}

/// The writer's seconds, bytes and monotonic reply time.
fn writer_report(path: &Path) -> Result<(f64, u64, u64), String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let report = Value::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let seconds = report.get("seconds").and_then(Value::as_f64);
    let bytes = report.get("bytes").and_then(Value::as_u64);
    let replied = report.get("replied_ns").and_then(Value::as_u64);
    match (seconds, bytes, replied) {
        (Some(seconds), Some(bytes), Some(replied)) if seconds > 0.0 => {
            Ok((seconds, bytes, replied))
        }
        _ => Err(format!("{}: incomplete report {text:?}", path.display())),
    }
}

/// The harness's client: reads whole ORBS messages with the shared codec.
struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn new(stream: UnixStream) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(READ_POLL))
            .map_err(|error| format!("socket: {error}"))?;
        let reader = BufReader::with_capacity(
            1 << 20,
            stream
                .try_clone()
                .map_err(|error| format!("socket: {error}"))?,
        );
        Ok(Self { stream, reader })
    }

    fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        let bytes = encode_client_message(message).map_err(|error| error.to_string())?;
        self.stream
            .write_all(&bytes)
            .map_err(|error| format!("socket: {error}"))
    }

    /// Hello, then a resize to the grid; done at the first frame of the grid,
    /// within `SETUP_WAIT`.
    fn attach(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let start = Instant::now();
        let mut waiting = || {
            suite::interrupt::check()?;
            if start.elapsed() >= SETUP_WAIT {
                return Err(format!("no handshake within {} s", SETUP_WAIT.as_secs()));
            }
            Ok(())
        };
        self.send(&ClientMessage::Hello)?;
        loop {
            match next_message(&mut self.reader, &mut waiting)?
                .ok_or("Orbit closed before Attached")?
            {
                (ServerMessage::Attached, _) => break,
                (ServerMessage::Busy, _) => return Err("Orbit answered Busy".into()),
                (ServerMessage::Failure(failure), _) => {
                    return Err(format!("Orbit refused Hello: {failure:?}"));
                }
                _ => {}
            }
        }
        let (cell_width, cell_height) = (10, 20);
        self.send(&ClientMessage::Resize(SurfaceSize {
            cols,
            rows,
            screen_width: u32::from(cols) * cell_width,
            screen_height: u32::from(rows) * cell_height,
            cell_width,
            cell_height,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        }))?;
        loop {
            match next_message(&mut self.reader, &mut waiting)?
                .ok_or("Orbit closed before the grid")?
            {
                (ServerMessage::Frame(frame), _)
                    if (frame.dimensions.rows, frame.dimensions.cols) == (rows, cols) =>
                {
                    return Ok(());
                }
                (ServerMessage::Failure(failure), _) => {
                    return Err(format!("Orbit refused the resize: {failure:?}"));
                }
                _ => {}
            }
        }
    }

    fn discard_in_background(self) -> Result<Reader, String> {
        let Self { stream, mut reader } = self;
        stream
            .set_read_timeout(None)
            .map_err(|error| format!("socket: {error}"))?;
        let state = Arc::new(Mutex::new(Received {
            last_message_ns: monotonic_ns(),
            ..Received::default()
        }));
        let shared = Arc::clone(&state);
        let handle = std::thread::spawn(move || {
            shared.lock().expect("received").thread = procfs::thread_id();
            loop {
                let message = next_message(&mut reader, &mut || Ok(()));
                let mut received = shared.lock().expect("received");
                match message {
                    Ok(Some((message, bytes))) => received.count(&message, bytes),
                    Ok(None) => return,
                    Err(error) => {
                        received.error = Some(error);
                        return;
                    }
                }
            }
        });
        Ok(Reader {
            _stream: stream,
            state,
            handle,
        })
    }
}

/// The next whole message and its length; `None` at the end of the stream
/// between messages. A read that times out calls `waiting`, which ends the
/// wait with its error or lets the read go on.
fn next_message(
    reader: &mut impl Read,
    waiting: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Option<(ServerMessage, u64)>, String> {
    let mut bytes = vec![0; HEADER_BYTES];
    if !fill(reader, &mut bytes, waiting)? {
        return Ok(None);
    }
    let length = server_message_len(&bytes)
        .map_err(|error| error.to_string())?
        .ok_or("a whole header has no length")?;
    bytes.resize(length, 0);
    if !fill(reader, &mut bytes[HEADER_BYTES..], waiting)? {
        return Err("the stream ended inside a message".into());
    }
    let message = decode_server_message(&bytes).map_err(|error| error.to_string())?;
    Ok(Some((message, length as u64)))
}

/// Fills `bytes`; false when the stream ends before the first byte, an
/// error when it ends after it.
fn fill(
    reader: &mut impl Read,
    bytes: &mut [u8],
    waiting: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let mut filled = 0;
    while filled < bytes.len() {
        match reader.read(&mut bytes[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err("the stream ended inside a message".into()),
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                waiting()?;
            }
            Err(error) => return Err(format!("socket: {error}")),
        }
    }
    Ok(true)
}

/// What the client received after the handshake, or since the go.
#[derive(Default)]
struct Received {
    thread: Option<u32>,
    /// Frames between the handshake and the go, and the last one's revision.
    setup_frames: u64,
    setup_last_revision: Option<u64>,
    frames: u64,
    frame_bytes: u64,
    other_bytes: u64,
    first_revision: Option<u64>,
    last_revision: Option<u64>,
    revision_gaps: u64,
    revision_repeats: u64,
    /// Monotonic arrival of each frame.
    frame_ns: Vec<u64>,
    last_message_ns: u64,
    exited: Option<i32>,
    error: Option<String>,
}

impl Received {
    fn count(&mut self, message: &ServerMessage, bytes: u64) {
        let now = monotonic_ns();
        self.last_message_ns = now;
        match message {
            ServerMessage::Frame(frame) => self.frame(frame.revision, bytes, now),
            ServerMessage::Exited { code } => {
                self.exited = Some(*code);
                self.other_bytes += bytes;
            }
            _ => self.other_bytes += bytes,
        }
    }

    fn frame(&mut self, revision: u64, bytes: u64, at_ns: u64) {
        self.frames += 1;
        self.frame_bytes += bytes;
        self.frame_ns.push(at_ns);
        match self.last_revision {
            Some(last) if revision == last => self.revision_repeats += 1,
            Some(last) if revision > last + 1 => self.revision_gaps += 1,
            _ => {}
        }
        self.first_revision.get_or_insert(revision);
        self.last_revision = Some(revision);
    }

    /// Counts from here on, keeping the last revision so a skip across the
    /// go still counts as a gap.
    fn restart(&mut self) {
        *self = Self {
            thread: self.thread,
            setup_frames: self.frames,
            setup_last_revision: self.last_revision,
            last_revision: self.last_revision,
            last_message_ns: self.last_message_ns,
            ..Self::default()
        };
    }

    /// The wire counters; with the writer's monotonic reply time, how the
    /// last frame and the reply fall relative to each other.
    fn counters(&self, reply_ns: Option<u64>) -> Vec<(String, u64)> {
        let mut counters = vec![
            ("setup_frames".to_owned(), self.setup_frames),
            ("frames".to_owned(), self.frames),
            ("frame_bytes".to_owned(), self.frame_bytes),
            ("other_bytes".to_owned(), self.other_bytes),
            ("revision_gaps".to_owned(), self.revision_gaps),
            ("revision_repeats".to_owned(), self.revision_repeats),
        ];
        if let Some(revision) = self.setup_last_revision {
            counters.push(("setup_last_revision".into(), revision));
        }
        if let (Some(first), Some(last)) = (self.first_revision, self.last_revision) {
            counters.push(("first_revision".into(), first));
            counters.push(("last_revision".into(), last));
        }
        if let (Some(reply), Some(&last)) = (reply_ns, self.frame_ns.last()) {
            counters.push((
                "frames_after_reply".into(),
                self.frame_ns.iter().filter(|&&at| at > reply).count() as u64,
            ));
            counters.push((
                "last_frame_after_reply_us".into(),
                last.saturating_sub(reply) / 1000,
            ));
            counters.push((
                "reply_after_last_frame_us".into(),
                reply.saturating_sub(last) / 1000,
            ));
        }
        counters
    }
}

/// The client's reader thread.
struct Reader {
    /// Holds the connection open until the reader is done.
    _stream: UnixStream,
    state: Arc<Mutex<Received>>,
    handle: JoinHandle<()>,
}

impl Reader {
    fn cpu_ns(&self) -> Option<u64> {
        procfs::thread_cpu_ns(self.state.lock().expect("received").thread?)
    }

    /// Waits until no message has arrived for `QUIET`, so frames still
    /// queued when the setup or the workload ends are read; false when
    /// messages still arrive after `QUIET_WAIT`.
    fn wait_quiet(&self) -> bool {
        let start = Instant::now();
        while start.elapsed() < QUIET_WAIT {
            let last = self.state.lock().expect("received").last_message_ns;
            if monotonic_ns().saturating_sub(last) >= QUIET.as_nanos() as u64 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Closes the connection and ends the reader, discarding its counts.
    fn close(self) -> Result<(), String> {
        let _ = self._stream.shutdown(Shutdown::Both);
        let start = Instant::now();
        while !self.handle.is_finished() {
            if start.elapsed() >= SETUP_WAIT {
                return Err("the client's reader did not end after it closed".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn restart(&self) {
        self.state.lock().expect("received").restart();
    }

    /// The counts once Orbit has closed the stream, which must end whole and
    /// after `Exited` with code 0.
    fn finish(self) -> Result<Received, String> {
        let start = Instant::now();
        while !self.handle.is_finished() {
            if start.elapsed() >= SETUP_WAIT {
                return Err("the client's stream did not end after Orbit exited".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.handle
            .join()
            .map_err(|_| "the client's reader panicked")?;
        let received = std::mem::take(&mut *self.state.lock().expect("received"));
        if let Some(error) = received.error {
            return Err(format!("client: {error}"));
        }
        if received.exited != Some(0) {
            return Err(format!(
                "the client read Exited {:?}, not Exited 0",
                received.exited
            ));
        }
        Ok(received)
    }
}

/// Per-workload throughput in both modes, and the attached run's frame
/// counters: frames and frame bytes per MiB of input, the host's and worker's
/// CPU time per frame, and the host's voluntary context switches per MiB.
fn table(result: &RunResult) -> String {
    let mut table = format!(
        "orbit at {}x{}, medians over {} samples after {} warmups\n\
         {:<32}{:>9}{:>9}{:>9}{:>11}{:>9}{:>10}{:>10}{:>9}",
        result.environment.grid.rows,
        result.environment.grid.cols,
        result.samples,
        result.warmups,
        "workload",
        "det MB/s",
        "att MB/s",
        "+att ms",
        "frames/MiB",
        "KB/frame",
        "host us/f",
        "work us/f",
        "csw/MiB",
    );
    let mut workloads: Vec<&str> = Vec::new();
    for cell in &result.cells {
        if !workloads.contains(&cell.workload.as_str()) {
            workloads.push(&cell.workload);
        }
    }
    for workload in workloads {
        let find = |layer: &str| {
            result
                .cells
                .iter()
                .find(|cell| cell.workload == workload && cell.layer == layer)
        };
        let seconds = |cell: Option<&Cell>| {
            let seconds: Vec<f64> = cell?.samples.iter().map(|sample| sample.seconds).collect();
            Some((Summary::of(&seconds)?.median, cell?.input_bytes))
        };
        let counter = |cell: Option<&Cell>, name: &str| {
            let values: Vec<f64> = cell?
                .samples
                .iter()
                .flat_map(|sample| &sample.counters)
                .filter(|(counter, _)| counter == name)
                .map(|&(_, value)| value as f64)
                .collect();
            Some(Summary::of(&values)?.median)
        };
        let (detached, attached) = (find("orbit-detached"), find("orbit-attached"));
        let rate = |time: Option<(f64, u64)>| {
            time.map_or("-".into(), |(seconds, bytes)| {
                format!("{:.1}", bytes as f64 / 1e6 / seconds)
            })
        };
        let number = |value: Option<f64>, digits: usize| {
            value.map_or("-".into(), |value| format!("{value:.digits$}"))
        };
        let mib =
            |cell: Option<&Cell>| cell.map(|cell| cell.input_bytes as f64 / f64::from(1 << 20));
        let per = |value: Option<f64>, by: Option<f64>| match (value, by) {
            (Some(value), Some(by)) if by > 0.0 => Some(value / by),
            _ => None,
        };
        let frames = counter(attached, "frames");
        let extra = match (seconds(detached), seconds(attached)) {
            (Some((detached, _)), Some((attached, _))) => Some((attached - detached) * 1e3),
            _ => None,
        };
        table.push_str(&format!(
            "\n{:<32}{:>9}{:>9}{:>9}{:>11}{:>9}{:>10}{:>10}{:>9}",
            workload,
            rate(seconds(detached)),
            rate(seconds(attached)),
            number(extra, 0),
            number(per(frames, mib(attached)), 1),
            number(
                per(counter(attached, "frame_bytes"), frames).map(|b| b / 1e3),
                1
            ),
            number(per(counter(attached, "host_cpu_us"), frames), 0),
            number(per(counter(attached, "worker_cpu_us"), frames), 0),
            number(
                per(counter(attached, "host_context_switches"), mib(attached)),
                1
            ),
        ));
    }
    table.push_str(
        "\n+att ms: attached minus detached median time. Per frame and per MiB of \
         input: medians of the attached samples; csw: the host's voluntary \
         context switches.",
    );
    table
}

/// Orbit's processes and this harness's threads, read from `/proc` (Linux).
mod procfs {
    use std::path::Path;

    /// Orbit's host and its Bend worker.
    pub struct Processes {
        host: u32,
        worker: u32,
    }

    pub struct Usage {
        host_cpu_ns: u64,
        worker_cpu_ns: u64,
        host_context_switches: u64,
        worker_context_switches: u64,
    }

    impl Usage {
        pub fn since(&self, before: &Usage) -> Vec<(String, u64)> {
            vec![
                (
                    "host_cpu_us".into(),
                    self.host_cpu_ns.saturating_sub(before.host_cpu_ns) / 1000,
                ),
                (
                    "worker_cpu_us".into(),
                    self.worker_cpu_ns.saturating_sub(before.worker_cpu_ns) / 1000,
                ),
                (
                    "host_context_switches".into(),
                    self.host_context_switches
                        .saturating_sub(before.host_context_switches),
                ),
                (
                    "worker_context_switches".into(),
                    self.worker_context_switches
                        .saturating_sub(before.worker_context_switches),
                ),
            ]
        }
    }

    impl Processes {
        /// The host and its one other child besides the terminal's child;
        /// `None` off Linux or when that child is not found.
        pub fn find(host: u32, terminal_child: u32) -> Option<Self> {
            let worker = std::fs::read_dir("/proc")
                .ok()?
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                .find(|&pid| pid != terminal_child && parent(pid) == Some(host))?;
            Some(Self { host, worker })
        }

        pub fn read(&self) -> Usage {
            let (host_cpu_ns, host_context_switches) = usage(self.host);
            let (worker_cpu_ns, worker_context_switches) = usage(self.worker);
            Usage {
                host_cpu_ns,
                worker_cpu_ns,
                host_context_switches,
                worker_context_switches,
            }
        }
    }

    fn parent(pid: u32) -> Option<u32> {
        let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
        let close = stat.iter().rposition(|&byte| byte == b')')?;
        let fields = std::str::from_utf8(stat.get(close + 2..)?).ok()?;
        fields.split(' ').nth(1)?.parse().ok()
    }

    /// The process's CPU time in nanoseconds and its voluntary context
    /// switches, summed over its threads; zero for a process that is gone.
    fn usage(pid: u32) -> (u64, u64) {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
            return (0, 0);
        };
        let (mut cpu, mut switches) = (0, 0);
        for task in tasks.flatten() {
            cpu += task_cpu_ns(&task.path()).unwrap_or(0);
            switches += std::fs::read_to_string(task.path().join("status"))
                .ok()
                .and_then(|status| {
                    status
                        .lines()
                        .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
                        .and_then(|count| count.trim().parse::<u64>().ok())
                })
                .unwrap_or(0);
        }
        (cpu, switches)
    }

    /// A task's time on CPU, the first field of its schedstat.
    fn task_cpu_ns(task: &Path) -> Option<u64> {
        std::fs::read_to_string(task.join("schedstat"))
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    pub fn thread_cpu_ns(thread: u32) -> Option<u64> {
        task_cpu_ns(Path::new(&format!("/proc/self/task/{thread}")))
    }

    #[cfg(target_os = "linux")]
    pub fn thread_id() -> Option<u32> {
        // SAFETY: gettid has no preconditions.
        u32::try_from(unsafe { libc::gettid() }).ok()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn thread_id() -> Option<u32> {
        None
    }

    /// The connections Orbit holds on `socket`: each accepted socket in
    /// `/proc/net/unix` carries its listener's path, and the listener alone
    /// is unconnected (state 01).
    pub fn connections(socket: &Path) -> Option<usize> {
        let table = std::fs::read_to_string("/proc/net/unix").ok()?;
        let path = socket.to_str()?;
        Some(
            table
                .lines()
                .skip(1)
                .map(|line| line.split_whitespace().collect::<Vec<_>>())
                .filter(|fields| fields.len() == 8 && fields[5] == "03" && fields[7] == path)
                .count(),
        )
    }
}

#[cfg(test)]
mod tests {
    use orbit_protocol::session::ServerMessage;

    use super::Received;

    /// Revisions that skip count as gaps and repeats as repeats; the frame
    /// times place the last frame against the reply.
    #[test]
    fn frame_counters_follow_revisions_and_the_reply() {
        let mut received = Received::default();
        for (revision, at_ns) in [
            (4, 10_000),
            (5, 20_000),
            (5, 30_000),
            (8, 40_000),
            (9, 50_000),
        ] {
            received.frame(revision, 100, at_ns);
        }
        received.count(&ServerMessage::Exited { code: 0 }, 16);
        assert_eq!(received.exited, Some(0));
        let counters = received.counters(Some(35_000));
        let get = |name: &str| {
            counters
                .iter()
                .find(|(counter, _)| counter == name)
                .map(|&(_, value)| value)
        };
        assert_eq!(get("frames"), Some(5));
        assert_eq!(get("frame_bytes"), Some(500));
        assert_eq!(get("other_bytes"), Some(16));
        assert_eq!(get("revision_gaps"), Some(1));
        assert_eq!(get("revision_repeats"), Some(1));
        assert_eq!(get("first_revision"), Some(4));
        assert_eq!(get("last_revision"), Some(9));
        assert_eq!(get("frames_after_reply"), Some(2));
        assert_eq!(get("last_frame_after_reply_us"), Some(15));
        assert_eq!(get("reply_after_last_frame_us"), Some(0));
    }
}
