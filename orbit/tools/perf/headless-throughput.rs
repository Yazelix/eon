//! PTY-throughput comparison of Orbit servers without a desktop.
//!
//! Runs vtebench inside each server with the arguments of Eon's
//! orbit-venus-baseline.sh, detached and with an attached client that reads
//! and discards every message, as Venus would without rendering, and counts
//! the bytes, the frames and the last frame's revision from each message's
//! start. A detached run attaches only to set the grid and then detaches; it
//! starts the benchmark once the server has closed that connection. Once a
//! second, from the CPUs outside CPUS, it counts the threads running or
//! waiting on I/O outside the harness and its servers and inside them, and
//! prints them after the table with the 1-min load at the start and the end.
//! Every server takes Orbit's command line
//! `serve SOCKET --ansi-palette-v1 PALETTE -- COMMAND`. The harness pins
//! itself and its servers with taskset.
//!
//! Usage, from the orbit directory:
//!   mkdir -p target
//!   rustc --edition 2024 -O tools/perf/headless-throughput.rs -o target/headless-throughput
//!   target/headless-throughput VTEBENCH_DIR CPUS ROWS COLS REPETITIONS NAME=SERVER...

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};

const USAGE: &str =
    "usage: headless-throughput VTEBENCH_DIR CPUS ROWS COLS REPETITIONS NAME=SERVER...";
const PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";
const CHILD: &str = r#"
until [[ $(stty size) == "$1" ]]; do sleep 0.05; done
touch "$2"
until [[ -e $3 ]]; do sleep 0.05; done
stty size >"$4"
cd "$5"
exec target/release/vtebench --silent --warmup 1 --min-bytes 1048576 --max-samples 5 --max-secs 10 --dat "$6"
"#;
const FRAME: u8 = 132;

fn message(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = b"ORBS".to_vec();
    out.extend(14u16.to_le_bytes());
    out.extend([kind, 0]);
    out.extend((payload.len() as u32).to_le_bytes());
    out.extend(payload);
    out
}

fn resize(rows: u32, cols: u32) -> Vec<u8> {
    let (cell_width, cell_height) = (10, 20);
    let mut payload = vec![];
    payload.extend(
        u16::try_from(cols)
            .expect("COLS must fit u16")
            .to_le_bytes(),
    );
    payload.extend(
        u16::try_from(rows)
            .expect("ROWS must fit u16")
            .to_le_bytes(),
    );
    for value in [
        cols * cell_width,
        rows * cell_height,
        cell_width,
        cell_height,
        0,
        0,
        0,
        0,
    ] {
        payload.extend(value.to_le_bytes());
    }
    message(6, &payload)
}

/// A server process, killed when dropped, with its output read as it comes.
struct Server {
    child: Child,
    output: Option<(thread::JoinHandle<String>, thread::JoinHandle<String>)>,
}

impl Server {
    fn start(command: &mut Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("server: {e}"));
        let out = child.stdout.take().expect("piped");
        let err = child.stderr.take().expect("piped");
        let read = |mut pipe: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut text = vec![];
                let _ = pipe.read_to_end(&mut text);
                String::from_utf8_lossy(&text).into_owned()
            })
        };
        let output = Some((read(Box::new(out)), read(Box::new(err))));
        Self { child, output }
    }

    /// The exit status, if the server has exited.
    fn poll(&mut self) -> Option<i32> {
        let status = self.child.try_wait().expect("server status")?;
        Some(status.code().unwrap_or(-1))
    }

    fn output(&mut self) -> String {
        let (out, err) = self.output.take().expect("output read once");
        format!(
            "stdout {:?}, stderr {:?}",
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default()
        )
    }

    /// Waits until done holds, failing if the server exits or time runs out.
    fn wait_for(&mut self, done: impl Fn() -> bool, seconds: u64, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while !done() {
            if let Some(code) = self.poll() {
                panic!("server exited {code}: {}", self.output());
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.poll().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The connections the server holds on SOCKET_PATH, from /proc/net/unix,
/// where each accepted socket carries its listener's path.
fn connections(socket_path: &Path) -> usize {
    let table = fs::read_to_string("/proc/net/unix").expect("read /proc/net/unix");
    let path = socket_path.to_str().expect("UTF-8 socket path");
    table
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .filter(|f| f.len() == 8 && f[5] == "03" && f[7] == path)
        .count()
}

#[derive(Default)]
struct Received {
    bytes: u64,
    frames: u64,
    revision: Option<u64>,
    complete: bool,
}

/// How much of a message's start to read: its ORBS header, and for a frame
/// the ORBF magic, version and revision after it.
fn wanted(start: &[u8]) -> usize {
    if start.len() >= 12 && start[6] == FRAME {
        12 + 6 + 8
    } else {
        12
    }
}

fn payload_len(start: &[u8]) -> usize {
    u32::from_le_bytes(start[8..12].try_into().expect("four bytes")) as usize
}

/// Reads to the end of the stream; counts its bytes and frames and keeps the
/// last frame's revision, reading only each message's start.
fn discard(mut peer: UnixStream, received: Arc<Mutex<Received>>) {
    let mut start = vec![];
    let mut skip = 0; // the rest of the current message
    let mut chunk = vec![0; 1 << 20];
    loop {
        let n = match peer.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return,
        };
        let mut r = received.lock().expect("received");
        r.bytes += n as u64;
        let mut offset = 0;
        while offset < n {
            if skip > 0 {
                let taken = skip.min(n - offset);
                skip -= taken;
                offset += taken;
                continue;
            }
            let taken = (wanted(&start) - start.len()).min(n - offset);
            start.extend_from_slice(&chunk[offset..offset + taken]);
            offset += taken;
            if start.len() == 12 {
                assert!(
                    &start[..6] == b"ORBS\x0e\0" && start[7] == 0,
                    "invalid ORBS v14 header"
                );
                assert!(payload_len(&start) >= wanted(&start) - 12, "short frame");
            }
            if start.len() < wanted(&start) {
                continue;
            }
            if start[6] == FRAME {
                assert!(&start[12..18] == b"ORBF\x03\0", "invalid ORBF v3 header");
                r.frames += 1;
                r.revision = Some(u64::from_le_bytes(
                    start[18..26].try_into().expect("eight bytes"),
                ));
            }
            skip = 12 + payload_len(&start) - start.len();
            start.clear();
        }
    }
    received.lock().expect("received").complete = start.is_empty() && skip == 0;
}

/// Waits up to seconds for a thread to finish.
fn join_within(handle: &thread::JoinHandle<()>, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

struct Run {
    names: Vec<String>,
    samples: Vec<Vec<String>>,
    wall: f64,
    received: Received,
}

fn run_once(
    server_bin: &str,
    vtebench: &Path,
    cpus: &str,
    rows: u32,
    cols: u32,
    attached: bool,
    root: &Path,
) -> Run {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root)
        .expect("create run directory");
    let socket_path = root.join("orbit.sock");
    let (ready, go, grid, dat) = (
        root.join("ready"),
        root.join("go"),
        root.join("grid"),
        root.join("results.dat"),
    );
    let paths = [&ready, &go, &grid].map(|p| p.to_str().expect("UTF-8 path").to_string());
    let mut server = Server::start(Command::new("taskset").args([
        "-c",
        cpus,
        server_bin,
        "serve",
        socket_path.to_str().expect("UTF-8 path"),
        "--ansi-palette-v1",
        PALETTE,
        "--",
        "/bin/bash",
        "-c",
        CHILD,
        "bash",
        &format!("{rows} {cols}"),
        &paths[0],
        &paths[1],
        &paths[2],
        vtebench.to_str().expect("UTF-8 path"),
        dat.to_str().expect("UTF-8 path"),
    ]));
    server.wait_for(
        || socket_path.exists(),
        10,
        &socket_path.display().to_string(),
    );
    let mut client = UnixStream::connect(&socket_path).expect("connect");
    let mut hello = message(1, &[]);
    hello.extend(resize(rows, cols));
    client.write_all(&hello).expect("send hello and resize");
    let received = Arc::new(Mutex::new(Received::default()));
    let reader = {
        let (peer, received) = (client.try_clone().expect("clone client"), received.clone());
        thread::spawn(move || discard(peer, received))
    };
    server.wait_for(|| ready.exists(), 10, &ready.display().to_string());
    if !attached {
        assert_eq!(
            connections(&socket_path),
            1,
            "the server does not hold exactly one client connection"
        );
        // Dropping alone would not close the connection: the reader's clone
        // keeps it open. Shutting it down ends the reader's read too.
        client
            .shutdown(Shutdown::Both)
            .expect("shut the client down");
        join_within(&reader, 5);
        drop(client);
        server.wait_for(
            || connections(&socket_path) == 0,
            5,
            "the server to close the client",
        );
    }
    let start = Instant::now();
    fs::write(&go, "").expect("touch go");
    let deadline = start + Duration::from_secs(900);
    let code = loop {
        if let Some(code) = server.poll() {
            break code;
        }
        assert!(Instant::now() < deadline, "the benchmark timed out");
        thread::sleep(Duration::from_millis(20));
    };
    let wall = start.elapsed().as_secs_f64();
    if attached {
        join_within(&reader, 10);
        assert!(
            received.lock().expect("received").complete,
            "client did not finish reading a complete ORBS stream"
        );
    }
    assert_eq!(code, 0, "server exited {code}: {}", server.output());
    let grid = fs::read_to_string(&grid).expect("read grid");
    assert_eq!(
        grid.split_whitespace().collect::<Vec<_>>(),
        [rows.to_string(), cols.to_string()],
        "{grid}"
    );
    let dat = fs::read_to_string(&dat).expect("read results");
    let mut lines = dat
        .lines()
        .map(|l| l.split_whitespace().map(String::from).collect::<Vec<_>>());
    let names = lines.next().unwrap_or_default();
    let received = std::mem::take(&mut *received.lock().expect("received"));
    Run {
        names,
        samples: lines.collect(),
        wall,
        received,
    }
}

/// The threads in state R or D outside ROOT's process tree, and inside it,
/// leaving out the calling process, which is always running while it reads.
fn runnable(root: u32) -> (u64, u64) {
    let me = std::process::id();
    let (mut parents, mut counts) = (HashMap::new(), HashMap::new());
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me {
            continue;
        }
        // Bytes: a name cut to 15 bytes need not be valid UTF-8.
        let Ok(stat) = fs::read(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(close) = stat.iter().rposition(|&b| b == b')') else {
            continue;
        };
        let ppid = stat
            .get(close + 2..)
            .and_then(|rest| rest.split(|&b| b == b' ').nth(1))
            .and_then(|f| std::str::from_utf8(f).ok()?.parse::<u32>().ok());
        let Some(ppid) = ppid else { continue };
        parents.insert(pid, ppid);
        let count = (|| {
            let mut count = 0;
            for task in fs::read_dir(format!("/proc/{pid}/task")).ok()? {
                let stat = fs::read(task.ok()?.path().join("stat")).ok()?;
                let close = stat.iter().rposition(|&b| b == b')')?;
                count += u64::from(matches!(stat.get(close + 2)?, b'R' | b'D'));
            }
            Some(count)
        })();
        // The process exited while it was read.
        if let Some(count) = count {
            counts.insert(pid, count);
        }
    }
    let mut tree = HashSet::from([root]);
    loop {
        let size = tree.len();
        tree.extend(
            parents
                .iter()
                .filter(|(_, p)| tree.contains(p))
                .map(|(pid, _)| *pid)
                .collect::<Vec<_>>(),
        );
        if tree.len() == size {
            break;
        }
    }
    let inside: u64 = tree.iter().filter_map(|pid| counts.get(pid)).sum();
    (counts.values().sum::<u64>() - inside, inside)
}

/// Counts runnable threads once a second until stdin closes, then prints
/// one `OUTSIDE INSIDE` line per sample.
fn sample(root: u32) {
    let (stop, stopped) = mpsc::channel::<()>();
    thread::spawn(move || {
        let _ = std::io::stdin().read_to_end(&mut vec![]);
        let _ = stop.send(());
    });
    let mut samples = vec![];
    while stopped.recv_timeout(Duration::from_secs(1)).is_err() {
        samples.push(runnable(root));
    }
    // The harness may have ended first; then there is no one to tell.
    let mut out = std::io::stdout().lock();
    for (outside, inside) in samples {
        if writeln!(out, "{outside} {inside}").is_err() {
            return;
        }
    }
}

/// The CPUs of a list such as `6-11` or `0,2-3`.
fn cpu_set(list: &str) -> BTreeSet<u32> {
    list.trim()
        .split(',')
        .filter(|p| !p.is_empty())
        .flat_map(|part| {
            let (low, high) = part.split_once('-').unwrap_or((part, part));
            let parse = |n: &str| {
                n.trim()
                    .parse::<u32>()
                    .unwrap_or_else(|_| panic!("bad CPU list {list}"))
            };
            parse(low)..=parse(high)
        })
        .collect()
}

fn cpu_list(set: &BTreeSet<u32>) -> String {
    set.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

fn percentile(values: &[f64], share: usize) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[(share * ordered.len()).div_ceil(100).max(1) - 1]
}

/// A number as Python's `{:g}` writes it.
fn general(x: f64) -> String {
    let scientific = format!("{x:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("exponent");
    let exponent: i32 = exponent.parse().expect("exponent");
    let trim = |s: &str| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    };
    if !(-4..6).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{}e{sign}{:02}", trim(mantissa), exponent.abs());
    }
    trim(&format!("{x:.*}", (5 - exponent) as usize))
}

fn load() -> String {
    let text = fs::read_to_string("/proc/loadavg").expect("read /proc/loadavg");
    text.split_whitespace().next().unwrap_or("?").to_string()
}

/// Removes the runs' directory however the harness ends.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 3 && args[1] == "--sample" {
        return sample(args[2].parse().expect("root PID"));
    }
    let [_, vtebench, cpus, rows, cols, repetitions, servers @ ..] = &args[..] else {
        panic!("{USAGE}");
    };
    assert!(!servers.is_empty(), "{USAGE}");
    let start_load = load();
    let benchmark_cpus = cpu_set(cpus);
    let status = fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let allowed = status
        .lines()
        .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
        .map(cpu_set)
        .expect("allowed CPUs");
    let others: BTreeSet<u32> = allowed.difference(&benchmark_cpus).copied().collect();
    let sampler_cpus = if others.is_empty() {
        &benchmark_cpus
    } else {
        &others
    };
    // Started before the harness pins itself, on the CPUs outside the benchmark.
    let me = std::process::id();
    let mut sampler = Command::new("taskset")
        .args(["-c", &cpu_list(sampler_cpus)])
        .arg(env::current_exe().expect("own path"))
        .args(["--sample", &me.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the sampler");
    let pinned = Command::new("taskset")
        .args([
            "-a",
            "-p",
            "-c",
            &cpu_list(&benchmark_cpus),
            &me.to_string(),
        ])
        .stdout(Stdio::null())
        .status()
        .expect("taskset");
    assert!(pinned.success(), "could not pin the harness to {cpus}");
    let (rows, cols, repetitions): (u32, u32, usize) = (
        rows.parse().expect("ROWS"),
        cols.parse().expect("COLS"),
        repetitions.parse().expect("REPETITIONS"),
    );
    let vtebench = fs::canonicalize(vtebench).expect("VTEBENCH_DIR");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // Short, since each run's socket path must fit a Unix socket address.
    let directory = env::temp_dir().join(format!("ht{me}-{}", stamp % 1_000_000));
    fs::create_dir(&directory).expect("create the runs' directory");
    let directory = TempDir(directory);
    let specs: Vec<(&str, &str)> = servers
        .iter()
        .map(|s| s.split_once('=').unwrap_or_else(|| panic!("{USAGE}")))
        .collect();
    let mut results: BTreeMap<(String, String, String), Vec<f64>> = BTreeMap::new();
    for repetition in 0..repetitions {
        for (name, server_bin) in &specs {
            for condition in ["detached", "attached"] {
                let root = directory.0.join(format!("{name}-{condition}-{repetition}"));
                let run = run_once(
                    server_bin,
                    &vtebench,
                    cpus,
                    rows,
                    cols,
                    condition == "attached",
                    &root,
                );
                let revision = run
                    .received
                    .revision
                    .map_or("None".into(), |r| r.to_string());
                eprintln!(
                    "{name} {condition} {}: {:.1} s, client read {} bytes, {} frames, last revision {revision}",
                    repetition + 1,
                    run.wall,
                    run.received.bytes,
                    run.received.frames,
                );
                for (index, benchmark) in run.names.iter().enumerate() {
                    let values = run
                        .samples
                        .iter()
                        .filter_map(|row| row.get(index).filter(|v| *v != "_"))
                        .map(|v| v.parse::<f64>().expect("sample"));
                    results
                        .entry((benchmark.clone(), name.to_string(), condition.to_string()))
                        .or_default()
                        .extend(values);
                }
            }
        }
    }
    let benchmarks: BTreeSet<&String> = results.keys().map(|(b, _, _)| b).collect();
    let columns: Vec<(&str, &str)> = specs
        .iter()
        .flat_map(|(name, _)| [(*name, "detached"), (*name, "attached")])
        .collect();
    let heads: Vec<String> = columns
        .iter()
        .map(|(n, c)| format!("{n} {c} p50/p95 ms (n)"))
        .collect();
    println!("benchmark\t{}", heads.join("\t"));
    for benchmark in benchmarks {
        let cells: Vec<String> = columns
            .iter()
            .map(|(name, condition)| {
                match results.get(&(benchmark.clone(), name.to_string(), condition.to_string())) {
                    Some(values) if !values.is_empty() => format!(
                        "{}/{} ({})",
                        general(percentile(values, 50)),
                        general(percentile(values, 95)),
                        values.len()
                    ),
                    _ => "-".into(),
                }
            })
            .collect();
        println!("{benchmark}\t{}", cells.join("\t"));
    }
    drop(sampler.stdin.take());
    let counts: Vec<(f64, f64)> = BufReader::new(sampler.stdout.take().expect("piped"))
        .lines()
        .map(|line| {
            let line = line.expect("sampler output");
            let (outside, inside) = line.split_once(' ').expect("sample line");
            (
                outside.parse().expect("count"),
                inside.parse().expect("count"),
            )
        })
        .collect();
    let _ = sampler.wait();
    assert!(!counts.is_empty(), "the sampler took no samples");
    let (outside, inside): (Vec<f64>, Vec<f64>) = counts.into_iter().unzip();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    println!(
        "runnable threads, sampled once a second (n={}): outside the benchmark mean {:.2} p50 {} p95 {} \
         max {}; benchmark mean {:.2}. 1-min load {start_load} at the start, {} at the end",
        outside.len(),
        mean(&outside),
        general(percentile(&outside, 50)),
        general(percentile(&outside, 95)),
        general(outside.iter().copied().fold(f64::MIN, f64::max)),
        mean(&inside),
        load(),
    );
}
