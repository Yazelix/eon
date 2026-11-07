//! The `floor` suite: the corpora at the layers below Eon. `--layer pty` drains
//! each corpus through a bare PTY, the kernel floor.

use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    corpus::{self, Corpus},
    environment,
    json::Value,
    pty,
    result::{Cell, Environment, Grid, RunResult, Sample, utc_timestamp},
    stats::Summary,
};

pub const USAGE: &str = "\
usage: eon-bench floor --layer pty --out DIR [--only ID]... [--samples N]
                       [--warmups N] [--max-load LOAD] [--cpus LIST] [--quick]
       eon-bench floor --help";

/// How often, and for how long, a sample waits for the load to drop.
const LOAD_POLL: Duration = Duration::from_secs(5);
const LOAD_WAIT: Duration = Duration::from_secs(600);

pub fn help() -> String {
    format!(
        "\
eon-bench floor: the corpora at the layers below Eon

  --layer pty      drain each corpus through a bare PTY (the kernel floor)
  --out DIR        writes DIR/corpus/ (as `eon-bench corpus`) and
                   DIR/result.json; created if missing
  --only ID        run only this corpus; repeat for several (default: all)
  --samples N      timed samples per corpus (default 10; 3 with --quick)
  --warmups N      untimed rounds first (default 2; 1 with --quick)
  --max-load LOAD  wait before a sample while the 1-minute load is above LOAD,
                   up to {} minutes, then fail (default: online CPUs / 4,
                   at least 1)
  --cpus LIST      pin the harness and its writers to these CPUs, such as 2,3
                   or 4-7 (Linux)
  --quick          a seconds-long smoke run

Each sample runs `eon-bench write` on a fresh {}x{} PTY: it writes the corpus,
asks for a status report (ESC[5n) and stops its clock at the reply, which the
reader sends once it has drained everything before the query. The bytes
drained must be the corpus plus one CR per LF (ONLCR) plus the query, or the
run fails. Rounds run every corpus once, in order, so corpora interleave.
Throughput is MB (10^6 bytes) of corpus per second. Exits 0 on success, 1 when
a sample fails, and 2 for a usage error.",
        LOAD_WAIT.as_secs() / 60,
        pty::COLS,
        pty::ROWS,
    )
}

pub struct Options {
    out: PathBuf,
    only: Vec<&'static Corpus>,
    samples: u64,
    warmups: u64,
    max_load: f64,
    cpus: Option<Vec<u64>>,
    quick: bool,
}

/// `--help` prints the help and ends with an empty error.
pub fn options(arguments: &[String]) -> Result<Options, String> {
    let (mut out, mut layer, mut samples, mut warmups, mut max_load, mut cpus) =
        (None, None, None, None, None, None);
    let mut only: Vec<&Corpus> = Vec::new();
    let mut quick = false;
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
        let known = [
            "--out",
            "--layer",
            "--samples",
            "--warmups",
            "--max-load",
            "--cpus",
            "--only",
        ];
        if !known.contains(&flag.as_str()) {
            return Err(format!("unknown argument {flag}\n{USAGE}"));
        }
        let Some(value) = arguments.next() else {
            return Err(format!("{flag} needs a value\n{USAGE}"));
        };
        let number = |value: &str| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{flag} takes a whole number, not {value:?}"))
        };
        let slot_taken = match flag.as_str() {
            "--out" => out.replace(PathBuf::from(value)).is_some(),
            "--layer" => layer.replace(value.as_str()).is_some(),
            "--samples" => samples.replace(number(value)?).is_some(),
            "--warmups" => warmups.replace(number(value)?).is_some(),
            "--max-load" => max_load
                .replace(
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|load| *load > 0.0 && load.is_finite())
                        .ok_or(format!("--max-load takes a positive number, not {value:?}"))?,
                )
                .is_some(),
            "--cpus" => cpus.replace(environment::parse_cpus(value)?).is_some(),
            "--only" => {
                let corpus = corpus::find(value).ok_or(format!(
                    "no corpus {value}; `eon-bench corpus --help` lists them"
                ))?;
                if !only.iter().any(|known| known.id == corpus.id) {
                    only.push(corpus);
                }
                false
            }
            _ => unreachable!("checked against the known flags"),
        };
        if slot_taken {
            return Err(format!("{flag} is repeated\n{USAGE}"));
        }
    }
    match layer {
        Some("pty") => {}
        Some(other) => return Err(format!("no layer {other}; the layer is pty")),
        None => return Err(format!("--layer is required\n{USAGE}")),
    }
    let samples = samples.unwrap_or(if quick { 3 } else { 10 });
    if samples == 0 {
        return Err("--samples must be at least 1".into());
    }
    if only.is_empty() {
        only = corpus::CORPORA.iter().collect();
    }
    Ok(Options {
        out: out.ok_or(format!("--out is required\n{USAGE}"))?,
        only,
        samples,
        warmups: warmups.unwrap_or(if quick { 1 } else { 2 }),
        max_load: max_load.unwrap_or((environment::cpus_online() as f64 / 4.0).max(1.0)),
        cpus,
        quick,
    })
}

pub fn run(options: &Options) -> Result<(), String> {
    if let Some(cpus) = &options.cpus {
        environment::pin(cpus)?;
    }
    let corpus_dir = options.out.join("corpus");
    corpus::write_files(&corpus_dir, &options.only)?;
    let writer = std::env::current_exe().map_err(|error| format!("eon-bench's path: {error}"))?;
    let report = options.out.join("writer-report.json");
    let kernel = environment::kernel();
    let build = environment::build()?;
    let started_at = utc_timestamp(SystemTime::now());
    let load_start = environment::load()?;
    let mut cells = Vec::new();
    for corpus in &options.only {
        let path = corpus_dir.join(corpus.file_name());
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let cell = Cell {
            workload: corpus.id.to_owned(),
            corpus_sha256: corpus.sha256.to_owned(),
            input_bytes: corpus.length,
            layer: "pty".into(),
            subject: "kernel-pty".into(),
            subject_version: kernel.clone(),
            samples: Vec::new(),
        };
        cells.push((cell, path, pty::expected_bytes(&bytes)));
    }
    let rounds = options.warmups + options.samples;
    for round in 0..rounds {
        let warmup = round < options.warmups;
        let (kind, number, of) = if warmup {
            ("warmup", round + 1, options.warmups)
        } else {
            ("sample", round - options.warmups + 1, options.samples)
        };
        eprintln!("eon-bench floor: {kind} {number} of {of}");
        for (cell, path, expected) in &mut cells {
            let load = wait_for_load(options.max_load)?;
            let drained = pty::drain(&writer, path, &report, *expected)
                .map_err(|error| format!("{}: {error}", cell.workload))?;
            if !warmup {
                cell.samples.push(Sample {
                    seconds: drained.seconds,
                    load,
                    counters: vec![
                        ("pty_bytes".into(), drained.bytes),
                        ("pty_reads".into(), drained.reads),
                    ],
                });
            }
        }
    }
    let _ = std::fs::remove_file(&report);
    let cpus_allowed = environment::cpus_allowed();
    let result = RunResult {
        suite: "floor".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: options.quick,
        corpus_version: corpus::CORPUS_VERSION.into(),
        warmups: options.warmups,
        samples: options.samples,
        started_at,
        finished_at: utc_timestamp(SystemTime::now()),
        environment: Environment {
            os: environment::os(),
            cpu: environment::cpu_model(),
            cpus_online: environment::cpus_online(),
            pinned_cpus: options.cpus.clone(),
            governor: cpus_allowed.as_deref().and_then(environment::governor),
            cpus_allowed,
            kernel,
            load_start,
            load_end: environment::load()?,
            max_load: options.max_load,
            grid: Grid {
                cols: pty::COLS.into(),
                rows: pty::ROWS.into(),
            },
            tools: vec![("eon-bench".into(), env!("CARGO_PKG_VERSION").into())],
            build,
        },
        cells: cells.into_iter().map(|(cell, _, _)| cell).collect(),
    };
    let path = options.out.join("result.json");
    let failure = |error: String| format!("{}: {error}", path.display());
    std::fs::write(&path, result.to_json().to_pretty())
        .map_err(|error| failure(error.to_string()))?;
    // The table comes from the file, so it shows what the file holds.
    let text = std::fs::read_to_string(&path).map_err(|error| failure(error.to_string()))?;
    let written = RunResult::from_json(&Value::parse(&text).map_err(failure)?).map_err(failure)?;
    println!("{}", table(&written));
    println!("result: {}", path.display());
    Ok(())
}

/// The 1-minute load once it is at most `max`, waiting for it to drop.
fn wait_for_load(max: f64) -> Result<f64, String> {
    let start = Instant::now();
    let mut said = false;
    loop {
        let load = environment::load()?;
        if load <= max {
            return Ok(load);
        }
        if start.elapsed() >= LOAD_WAIT {
            return Err(format!(
                "the 1-minute load stayed above {max} for {} minutes (now {load:.2})",
                LOAD_WAIT.as_secs() / 60
            ));
        }
        if !said {
            eprintln!("eon-bench floor: load {load:.2} is above {max}; waiting");
            said = true;
        }
        std::thread::sleep(LOAD_POLL);
    }
}

/// Throughput per corpus: median, the p95 time's throughput (the slow tail),
/// and the range.
fn table(result: &RunResult) -> String {
    let mut table = format!(
        "floor --layer pty, MB/s over {} samples after {} warmups{}\n{:<14}{:>10}{:>10}{:>22}",
        result.samples,
        result.warmups,
        if result.samples < 20 {
            "; below 20 samples p95 is the slowest"
        } else {
            ""
        },
        "corpus",
        "median",
        "p95",
        "min-max"
    );
    for cell in &result.cells {
        let seconds: Vec<f64> = cell.samples.iter().map(|sample| sample.seconds).collect();
        let Some(time) = Summary::of(&seconds) else {
            continue;
        };
        let rate = |seconds: f64| cell.input_bytes as f64 / 1e6 / seconds;
        table.push_str(&format!(
            "\n{:<14}{:>10.1}{:>10.1}{:>22}",
            cell.workload,
            rate(time.median),
            rate(time.p95),
            format!("{:.1}-{:.1}", rate(time.max), rate(time.min))
        ));
    }
    table
}
