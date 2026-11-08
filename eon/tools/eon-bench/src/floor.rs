//! The `floor` suite: the corpora at the layers below Eon. `--layer pty` drains
//! each corpus through a bare PTY, the kernel floor.

use std::time::SystemTime;

use crate::{
    corpus::{self, Corpus},
    environment, pty,
    result::{Cell, Grid, RunResult, Sample, utc_timestamp},
    stats::Summary,
    suite::{self, Settings},
};

pub const USAGE: &str = "\
usage: eon-bench floor --layer pty --out DIR [--only ID]... [--samples N]
                       [--warmups N] [--max-load LOAD] [--load-wait MINUTES]
                       [--cpus LIST] [--quick]
       eon-bench floor --help";

pub fn help() -> String {
    format!(
        "\
eon-bench floor: the corpora at the layers below Eon

  --layer pty      drain each corpus through a bare PTY (the kernel floor)
  --out DIR        writes DIR/corpus/ (as `eon-bench corpus`) and
                   DIR/result.json; created if missing
  --only ID        run only this corpus; repeat for several (default: all)
{}

Each sample runs `eon-bench write` on a fresh {}x{} PTY: it writes the corpus,
asks for a status report (ESC[5n) and stops its clock at the reply, which the
reader sends once it has drained everything before the query. The bytes
drained must be the corpus plus one CR per LF (ONLCR) plus the query, or the
run fails. Rounds run every corpus once, in order, so corpora interleave.
Throughput is MB (10^6 bytes) of corpus per second. Exits 0 on success, 1 when
a sample fails, and 2 for a usage error.",
        suite::settings_help(),
        pty::COLS,
        pty::ROWS,
    )
}

pub struct Options {
    settings: Settings,
    only: Vec<&'static Corpus>,
}

/// `--help` prints the help and ends with an empty error.
pub fn options(arguments: &[String]) -> Result<Options, String> {
    let (settings, own) = suite::parse(arguments, USAGE, help, &["--layer", "--only"])?;
    let mut layer = None;
    let mut only = Vec::new();
    for (flag, value) in &own {
        if flag == "--layer" {
            if layer.replace(value.as_str()).is_some() {
                return Err(format!("--layer is repeated\n{USAGE}"));
            }
        } else {
            corpus::add(&mut only, value)?;
        }
    }
    match layer {
        Some("pty") => {}
        Some(other) => return Err(format!("no layer {other}; the layer is pty")),
        None => return Err(format!("--layer is required\n{USAGE}")),
    }
    if only.is_empty() {
        only = corpus::CORPORA.iter().collect();
    }
    Ok(Options { settings, only })
}

pub fn run(options: &Options) -> Result<(), String> {
    let settings = &options.settings;
    if let Some(cpus) = &settings.cpus {
        environment::pin(cpus)?;
    }
    let corpus_dir = settings.out.join("corpus");
    corpus::write_files(&corpus_dir, &options.only)?;
    let writer = std::env::current_exe().map_err(|error| format!("eon-bench's path: {error}"))?;
    let report = settings.out.join("writer-report.json");
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
    for round in 0..settings.warmups + settings.samples {
        let (kind, number, of) = suite::round_label(settings, round);
        eprintln!("eon-bench floor: {kind} {number} of {of}");
        for (cell, path, expected) in &mut cells {
            let load = suite::wait_for_load("floor", settings)?;
            let drained = pty::drain(&writer, path, &report, *expected)
                .map_err(|error| format!("{}: {error}", cell.workload))?;
            if kind == "sample" {
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
    let grid = Grid {
        cols: pty::COLS.into(),
        rows: pty::ROWS.into(),
    };
    let tools = vec![("eon-bench".into(), env!("CARGO_PKG_VERSION").into())];
    let result = RunResult {
        suite: "floor".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: settings.quick,
        corpus_version: corpus::CORPUS_VERSION.into(),
        warmups: settings.warmups,
        samples: settings.samples,
        started_at,
        finished_at: utc_timestamp(SystemTime::now()),
        environment: suite::environment(settings, kernel, load_start, grid, tools, build)?,
        cells: cells.into_iter().map(|(cell, _, _)| cell).collect(),
    };
    let written = suite::write(&settings.out, &result)?;
    println!("{}", table(&written));
    println!("result: {}", settings.out.join("result.json").display());
    Ok(())
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
