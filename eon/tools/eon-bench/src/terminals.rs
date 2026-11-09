//! Render-enabled terminal throughput on the shared private compositor.
use crate::{
    child, corpus, environment, isolation,
    json::Value,
    kitten,
    launcher::{BusPolicy, Headless, Renderer, WindowMode},
    memory,
    result::{Cell, Grid, RunResult, Sample, utc_timestamp},
    sha256::Sha256,
    stats::Summary,
    suite, writer,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime},
};

type Result<T = ()> = std::result::Result<T, String>;
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
const SUBJECTS: [&str; 7] = [
    "eon",
    "foot",
    "alacritty",
    "kitty",
    "ghostty",
    "wezterm",
    "herdr-app",
];
const USAGE: &str =
    "usage: eon-bench terminals --eon PATH --source-tree HASH --out NEW_DIR [OPTIONS]";
const COLS: u64 = 80;
const ROWS: u64 = 24;
const MARKER: &str = "EON BENCH COMPLETE";

pub struct Options {
    settings: suite::Settings,
    eon: PathBuf,
    renderer: Renderer,
    subjects: Vec<String>,
    workloads: Vec<String>,
    repetitions: u64,
    deadline: Duration,
    source_tree: String,
}
fn help() -> String {
    format!(
        "{USAGE}\n\nCompare native-VT status-reply completion or one-window process memory.\n\
Memory seconds are rollup collection time; byte counters/window-memory.md hold memory.\n\
No pixel-latency claim.\n\
  --eon PATH       out-link-built Nix Eon candidate from this bench's tree (required)\n\
  --source-tree HASH  exact git write-tree used for candidate and bench builds (required)\n\
  --renderer MODE  lavapipe (default) or /dev/dri/renderD<N>\n\
  --subject NAME   eon, foot, alacritty, kitty, ghostty, wezterm, herdr-app; repeat\n\
                   opt-in: foot-tmux, foot-zellij (one pane inside foot)\n\
  --only WORKLOAD  cat64 or kitten-<ascii|unicode|csi|images|long_escape_codes>\n\
                   repeat; default cat64 and all five kitten workloads\n\
                   opt-in: memory-empty, memory-filled (eon/foot/ghostty only)\n\
  --kitten-repetitions N  default 100 (1 with --quick)\n\
  --deadline SECONDS      per sample, including startup/writes (default 180)\n\n\
Private Sway/HOME/runtime/D-Bus and PID/network namespace; never the desktop.\n\
80x24 PTY grid checked inside each terminal; DejaVu Sans Mono at 16 px,\n\
Scrollback: Eon/Ghostty 16 MiB; line-based references 10,000 lines.\n\
Nix bench supplies every reference and the font.\n\
Each subject/workload is interleaved with rotating order. Gaps and failures\n\
stay in result.json and logs; failed required subjects make the command fail.\n\
Tern/Termua gaps are historical availability observations dated 2026-11-08;\n\
refresh them before a future comparison.\n\
Zed is out of scope for this comparison.\n\n{}",
        suite::settings_help()
    )
}
pub fn options(args: &[String]) -> Result<Options> {
    let (settings, flags) = suite::parse(
        args,
        USAGE,
        help,
        &[
            "--eon",
            "--renderer",
            "--subject",
            "--only",
            "--kitten-repetitions",
            "--deadline",
            "--source-tree",
        ],
    )?;
    let (mut eon, mut renderer, mut repetitions, mut deadline) = (None, None, None, None);
    let mut source_tree = None;
    let (mut subjects, mut workloads) = (Vec::new(), Vec::new());
    for (flag, value) in flags {
        let repeated = match flag.as_str() {
            "--eon" => eon.replace(PathBuf::from(value)).is_some(),
            "--renderer" => renderer.replace(Renderer(value)).is_some(),
            "--kitten-repetitions" => repetitions.replace(suite::number(&flag, &value)?).is_some(),
            "--deadline" => deadline.replace(suite::number(&flag, &value)?).is_some(),
            "--source-tree" => {
                if value.len() != 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("--source-tree needs a 40-digit git tree hash".into());
                }
                source_tree.replace(value.to_lowercase()).is_some()
            }
            "--subject" => {
                add(
                    &mut subjects,
                    value,
                    &[&SUBJECTS[..], &["foot-tmux", "foot-zellij"]].concat(),
                )?;
                false
            }
            "--only" => {
                let mut names = vec![
                    "cat64".to_owned(),
                    "memory-empty".into(),
                    "memory-filled".into(),
                ];
                names.extend(kitten::BENCHMARKS.iter().map(|n| format!("kitten-{n}")));
                add(
                    &mut workloads,
                    value,
                    &names.iter().map(String::as_str).collect::<Vec<_>>(),
                )?;
                false
            }
            _ => unreachable!(),
        };
        if repeated {
            return Err(format!("{flag} is repeated"));
        }
    }
    let renderer = renderer.unwrap_or(Renderer("lavapipe".into()));
    renderer.render_node()?;
    let repetitions = repetitions.unwrap_or(if settings.quick {
        1
    } else {
        kitten::REPETITIONS
    });
    let deadline = deadline.unwrap_or(180);
    if repetitions == 0 || deadline == 0 {
        return Err("repetitions and deadline must be positive".into());
    }
    if subjects.is_empty() {
        subjects = SUBJECTS.iter().map(|s| s.to_string()).collect();
    }
    if workloads.is_empty() {
        workloads.push("cat64".into());
        workloads.extend(kitten::BENCHMARKS.iter().map(|s| format!("kitten-{s}")));
    }
    if workloads.iter().any(|w| w.starts_with("memory-"))
        && subjects
            .iter()
            .any(|s| !["eon", "foot", "ghostty"].contains(&s.as_str()))
    {
        return Err("window memory subjects are eon, foot and ghostty".into());
    }
    Ok(Options {
        settings,
        eon: eon.ok_or("--eon is required")?,
        renderer,
        subjects,
        workloads,
        repetitions,
        deadline: Duration::from_secs(deadline),
        source_tree: source_tree
            .ok_or("--source-tree is required; record git write-tree used for the package build")?,
    })
}
fn add(out: &mut Vec<String>, value: String, allowed: &[&str]) -> Result {
    if !allowed.contains(&value.as_str()) {
        return Err(format!("unknown selection {value:?}"));
    }
    if out.contains(&value) {
        return Err(format!("repeated selection {value:?}"));
    }
    out.push(value);
    Ok(())
}

pub fn run(args: &[String], options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("terminals requires Linux/Wayland".into());
    }
    let eon = crate::launcher::candidate(&options.eon)?;
    if options.subjects.iter().any(|s| s == "eon") {
        check_candidate_fontconfig(&eon, &tool("product_fontconfig")?)?;
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.settings.out)
        .map_err(error)?;
    let out = fs::canonicalize(&options.settings.out).map_err(error)?;
    let invocation = std::iter::once(std::env::args().next().ok_or("missing benchmark argv0")?)
        .chain(std::iter::once("terminals".into()))
        .chain(args.iter().cloned())
        .map(Value::from)
        .collect();
    let mut args = args.to_vec();
    for (key, path) in [("--out", out.clone()), ("--eon", eon)] {
        let index = args.iter().position(|s| s == key).ok_or(key)?;
        args[index + 1] = path.to_str().ok_or("non-UTF8 path")?.into();
    }
    let exe = std::env::current_exe().map_err(error)?;
    let mut command = isolation::command(&out, &[&exe]);
    command.env("EON_BENCH_INVOCATION", Value::Array(invocation).to_pretty());
    command.env("EON_BENCH_EON_PASSED", &options.eon);
    isolation::display(&mut command, options.renderer.render_node()?);
    // The product wrapper sets FONTCONFIG_FILE; control its contents only in this namespace.
    command
        .arg("--ro-bind")
        .arg(tool("fontconfig")?)
        .arg(tool("product_fontconfig")?);
    command.arg("--").arg(exe).arg("terminals-inner").args(args);
    let outcome = isolation::run(&mut command);
    suite::interrupt::resend();
    outcome
}

fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn read(path: &Path) -> Result<Value> {
    Value::parse(&fs::read_to_string(path).map_err(error)?)
}
fn hash(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(&fs::read(path).map_err(error)?);
    Ok(hash.hex())
}
fn tool(name: &str) -> Result<PathBuf> {
    let key = format!("EON_BENCH_{}", name.to_uppercase().replace('-', "_"));
    let path = PathBuf::from(
        std::env::var(&key).map_err(|_| format!("missing {key}; use the Nix bench package"))?,
    );
    crate::launcher::candidate(&path)
}

fn wrapper_path(text: &str, key: &str) -> Result<PathBuf> {
    let prefix = format!("export {key}=");
    let mut values = text.lines().filter_map(|line| line.strip_prefix(&prefix));
    let value = values
        .next()
        .ok_or(format!("candidate wrapper has no {key} export"))?;
    if values.next().is_some() {
        return Err(format!("candidate wrapper has ambiguous {key} exports"));
    }
    let value = value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| value.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .ok_or(format!("candidate wrapper has an unsupported {key} export"))?;
    let path = PathBuf::from(value);
    if !path.starts_with("/nix/store") {
        return Err(format!("candidate {key} is not a Nix store path"));
    }
    Ok(path)
}

fn check_candidate_fontconfig(eon: &Path, expected: &Path) -> Result {
    let venus = wrapper_path(&fs::read_to_string(eon).map_err(error)?, "EON_VENUS")?;
    let actual = wrapper_path(
        &fs::read_to_string(venus).map_err(error)?,
        "FONTCONFIG_FILE",
    )?;
    if fs::canonicalize(&actual).map_err(error)? != fs::canonicalize(expected).map_err(error)? {
        return Err(format!(
            "candidate Venus wrapper FONTCONFIG_FILE={} differs from EON_BENCH_PRODUCT_FONTCONFIG={}; build candidate and bench from the same tree",
            actual.display(),
            expected.display()
        ));
    }
    Ok(())
}

struct CoreStop {
    command: Command,
    dir: PathBuf,
    attempted: bool,
}
impl CoreStop {
    fn new(launch: &Headless, program: &Path, dir: &Path) -> Self {
        let mut command = launch.command("timeout");
        command.arg("2s").arg(program).args(["server", "stop"]);
        Self {
            command,
            dir: dir.into(),
            attempted: false,
        }
    }
    fn stop(&mut self) -> Result {
        self.attempted = true;
        let result = (|| {
            let output = self.command.output().map_err(error)?;
            fs::write(self.dir.join("core-stop.txt"), &output.stdout).map_err(error)?;
            fs::write(self.dir.join("core-stop.stderr.log"), &output.stderr).map_err(error)?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!("Herdr core stop: {}", output.status))
            }
        })();
        if let Err(message) = &result {
            let _ = fs::write(self.dir.join("core-stop-failure.txt"), message);
        }
        result
    }
}
impl Drop for CoreStop {
    fn drop(&mut self) {
        if !self.attempted {
            let _ = self.stop();
        }
    }
}

fn subject_order(round: u64, count: usize) -> impl Iterator<Item = usize> {
    (0..count).map(move |offset| (round as usize + offset) % count)
}

fn retain_sample(cell: &mut Cell, measured: bool, sample: Sample) {
    if measured {
        cell.samples.push(sample);
    }
}

fn start_once(issued: &mut bool, ready: bool) -> bool {
    if ready && !*issued {
        *issued = true;
        true
    } else {
        false
    }
}

fn adapter_evidence(name: &str, gpu: Option<&str>, log: &str) -> &'static str {
    if name == "foot" || name.starts_with("foot-") {
        return "cpu";
    }
    if let Some(gpu) = gpu {
        return if gpu.starts_with("software=false ") {
            "positive-hardware"
        } else {
            "positive-software"
        };
    }
    let log = log.to_lowercase();
    if log.contains("llvmpipe") || log.contains("softpipe") {
        return "positive-software";
    }
    if log.lines().any(|line| {
        ["renderer", "adapter", "opengl"]
            .iter()
            .any(|key| line.contains(key))
            && ["intel", "radeon", "nvidia", "nouveau"]
                .iter()
                .any(|name| line.contains(name))
    }) {
        "positive-hardware"
    } else {
        "no-software-renderer-seen"
    }
}

fn incomplete_cells(cells: &mut [Cell], expected: u64, observations: &[Value]) -> Vec<Value> {
    cells
        .iter_mut()
        .filter_map(|cell| {
            if cell.samples.len() as u64 == expected {
                return None;
            }
            let observed = cell.samples.len();
            cell.samples.clear();
            Some(object([
                ("subject", cell.subject.as_str().into()),
                ("workload", cell.workload.as_str().into()),
                ("status", "incomplete".into()),
                ("expected_samples", expected.into()),
                ("observed_samples", (observed as u64).into()),
                (
                    "observations",
                    Value::Array(
                        observations
                            .iter()
                            .filter(|o| {
                                o.get("subject").and_then(Value::as_str) == Some(&cell.subject)
                                    && o.get("workload").and_then(Value::as_str)
                                        == Some(&cell.workload)
                            })
                            .cloned()
                            .collect(),
                    ),
                ),
            ]))
        })
        .collect()
}

fn comparison_status(has_failures: bool, complete: bool) -> &'static str {
    if has_failures {
        "fail"
    } else if !complete {
        "incomplete"
    } else {
        "pass"
    }
}

fn summary(cells: &[Cell]) -> String {
    let mut table = String::from("subject\tworkload\tn\tp50_seconds\tp95_seconds\tp50_MB/s\n");
    for cell in cells.iter().filter(|c| !c.workload.starts_with("memory-")) {
        let times = cell.samples.iter().map(|s| s.seconds).collect::<Vec<_>>();
        if let Some(s) = Summary::of(&times) {
            table.push_str(&format!(
                "{}\t{}\t{}\t{:.6}\t{:.6}\t{:.3}\n",
                cell.subject,
                cell.workload,
                times.len(),
                s.median,
                s.p95,
                cell.input_bytes as f64 / 1e6 / s.median
            ));
        }
    }
    table
}

fn sample_counters(detail: &Value, bytes: u64) -> Result<Vec<(String, u64)>> {
    let mut counters = vec![("input_bytes".into(), bytes)];
    if let Some(memory) = detail.get("memory").filter(|m| **m != Value::Null) {
        for prefix in ["", "venus_"] {
            for metric in ["pss_bytes", "rss_bytes", "pss_anon_bytes"] {
                let key = format!("{prefix}{metric}");
                if let Some(value) = memory.get(&key) {
                    counters.push((key, value.as_u64().ok_or("invalid memory byte counter")?));
                } else if prefix.is_empty() {
                    return Err("missing window memory metric".into());
                }
            }
        }
    }
    Ok(counters)
}

fn multiplexer_summary(cells: &[Cell]) -> String {
    let mut table =
        "subject\tworkload\ttime_ratio_to_foot\toverhead_percent\tcompletion_rate_ratio_to_foot\n"
            .to_owned();
    for cell in cells.iter().filter(|c| !c.workload.starts_with("memory-")) {
        let median = |c: &Cell| {
            Summary::of(&c.samples.iter().map(|s| s.seconds).collect::<Vec<_>>()).map(|s| s.median)
        };
        if let Some(base) = cells
            .iter()
            .find(|c| c.subject == "foot" && c.workload == cell.workload)
            && let (Some(base), Some(subject)) = (median(base), median(cell))
        {
            let ratio = subject / base;
            table.push_str(&format!(
                "{}\t{}\t{ratio:.6}\t{:.3}\t{:.6}\n",
                cell.subject,
                cell.workload,
                100.0 * (ratio - 1.0),
                1.0 / ratio
            ));
        }
    }
    table
}

fn memory_summary(cells: &[Cell]) -> Result<String> {
    let mut table = "MiB per fresh window, median (nearest-rank p05–p95); held workload excluded. Venus is attribution, not the full Eon total.\n\n| Subject | Workload | n | PSS | RSS | Pss_Anon |\n| --- | --- | ---: | ---: | ---: | ---: |\n".to_owned();
    for cell in cells
        .iter()
        .filter(|c| c.workload.starts_with("memory-") && !c.samples.is_empty())
    {
        for prefix in ["", "venus_"] {
            if prefix == "venus_" && cell.subject != "eon" {
                continue;
            }
            table.push_str(&format!(
                "| {}{} | {} | {} |",
                cell.subject,
                if prefix.is_empty() { "" } else { "/Venus only" },
                cell.workload,
                cell.samples.len()
            ));
            for metric in ["pss_bytes", "rss_bytes", "pss_anon_bytes"] {
                let key = format!("{prefix}{metric}");
                let mut values = cell
                    .samples
                    .iter()
                    .map(|s| {
                        s.counters
                            .iter()
                            .find(|(k, _)| k == &key)
                            .map(|(_, v)| *v as f64 / 1048576.0)
                            .ok_or_else(|| "missing memory counter".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?;
                values.sort_by(f64::total_cmp);
                let s = Summary::of(&values).ok_or("missing memory samples")?;
                let p05 = values[(values.len() * 5).div_ceil(100) - 1];
                table.push_str(&format!(" {:.3} ({p05:.3}–{:.3}) |", s.median, s.p95));
            }
            table.push('\n');
        }
    }
    Ok(table)
}

pub fn run_inner(options: &Options) -> Result {
    let settings = &options.settings;
    if let Some(cpus) = &settings.cpus {
        environment::pin(cpus)?;
    }
    let started_at = utc_timestamp(SystemTime::now());
    let load_start = environment::load()?;
    let build = environment::build()?;
    let font = tool("font")?;
    let kitten_path = tool("kitten")?;
    let corpus_dir = settings.out.join("corpus");
    if options.workloads.iter().any(|s| s == "cat64") {
        corpus::write_files(&corpus_dir, &[corpus::find("cat64").unwrap()])?;
    }
    for workload in options
        .workloads
        .iter()
        .filter(|w| w.starts_with("memory-"))
    {
        let payload = if workload == "memory-filled" {
            memory::payload()
        } else {
            b"\x1b[0m\x1b[2J\x1b[H".to_vec()
        };
        fs::write(settings.out.join(format!("{workload}.vt")), payload).map_err(error)?;
    }
    let mut tools = vec![
        (
            "font-file".into(),
            format!("{} sha256:{}", font.display(), hash(&font)?),
        ),
        ("font-px".into(), "16".into()),
        (
            "scrollback".into(),
            "Eon/Ghostty: 16 MiB; foot/Alacritty/kitty/WezTerm: 10000 lines; Herdr: binary default"
                .into(),
        ),
        (
            "kitten".into(),
            format!("{} sha256:{}", kitten_path.display(), hash(&kitten_path)?),
        ),
        ("kitten-repetitions".into(), options.repetitions.to_string()),
    ];
    let mut subjects = Vec::new();
    let mut gaps = vec![object([("subject", "tern".into()), ("observed_at", "2026-11-08".into()), ("refresh", "Historical availability observation; refresh before a future comparison".into()), ("reason", "Closed beta/waitlist at https://stencil.so/tern; no downloadable Linux build observed".into())]), object([("subject", "termua".into()), ("observed_at", "2026-11-08".into()), ("refresh", "Historical availability observation; refresh before a future comparison".into()), ("reason", "Not in this pinned nixpkgs; upstream v0.1.5 binary packaging required additional investigation; optional comparison deferred".into())])];
    let mut failures = Vec::new();
    let mut unavailable = std::collections::BTreeSet::new();
    for name in &options.subjects {
        let path = if name == "eon" {
            Ok(options.eon.clone())
        } else {
            tool(if name.starts_with("foot-") {
                "foot"
            } else {
                name
            })
        };
        match path {
            Ok(path) => {
                let identity = format!("{} sha256:{}", path.display(), hash(&path)?);
                tools.push((name.clone(), identity.clone()));
                subjects.push((name.clone(), path, identity));
                if let Some(mux) = name.strip_prefix("foot-") {
                    let mux = tool(mux)?;
                    tools.push((
                        name.clone() + "-mux",
                        format!("{} sha256:{}", mux.display(), hash(&mux)?),
                    ));
                }
            }
            Err(e) => {
                unavailable.insert(name.clone());
                let gap = object([
                    ("subject", name.as_str().into()),
                    ("reason", e.as_str().into()),
                ]);
                if name == "herdr-app" {
                    gaps.push(gap);
                } else {
                    failures.push(gap);
                }
            }
        }
    }
    let mut cells = Vec::new();
    for workload in &options.workloads {
        for (name, _, identity) in &subjects {
            let corpus = corpus::find(workload);
            cells.push(Cell {
                workload: workload.clone(),
                corpus_sha256: if workload.starts_with("memory-") {
                    hash(&settings.out.join(format!("{workload}.vt")))?
                } else {
                    corpus.map_or(String::new(), |c| c.sha256.into())
                },
                input_bytes: corpus.map_or(0, |c| c.length),
                layer: if workload.starts_with("memory-") {
                    "one-window-process-memory"
                } else {
                    "render-enabled-completion"
                }
                .into(),
                subject: name.clone(),
                subject_version: identity.clone(),
                samples: Vec::new(),
            });
        }
    }
    let mut observations = Vec::new();
    if options.subjects.iter().any(|s| s == "eon") {
        let out = settings.out.join("adapter");
        fs::create_dir(&out).map_err(error)?;
        suite::wait_for_load("terminals adapter preflight", settings)?;
        if let Err(e) = crate::headless::run_inner(&crate::headless::Options {
            eon: options.eon.clone(),
            out,
            renderer: Renderer(options.renderer.0.clone()),
        }) {
            unavailable.insert("eon".into());
            failures.push(object([
                ("subject", "eon".into()),
                ("evidence", "adapter/result.json".into()),
                ("reason", e.into()),
            ]));
        }
    }
    for round in 0..settings.warmups + settings.samples {
        let (kind, number, count) = suite::round_label(settings, round);
        for workload in &options.workloads {
            for index in subject_order(round, subjects.len()) {
                let (name, program, _) = &subjects[index];
                if unavailable.contains(name) {
                    continue;
                }
                suite::interrupt::check()?;
                let load = environment::load()?;
                let id = format!("{round:02}-{workload}-{name}");
                let dir = settings.out.join(&id);
                fs::create_dir(&dir).map_err(error)?;
                eprintln!("terminals: {kind} {number}/{count} {workload} {name} load={load:.2}");
                let outcome = sample(options, &dir, name, program, workload, &font, &kitten_path);
                if dir.join("core-stop-failure.txt").exists() {
                    return Err(format!(
                        "environment failure: Herdr core cleanup failed; see {}",
                        dir.display()
                    ));
                }
                let end = environment::load()?;
                let (mut detail, failure) = match outcome {
                    Ok((seconds, bytes, detail)) => {
                        let cell = cells
                            .iter_mut()
                            .find(|c| c.subject == *name && c.workload == *workload)
                            .unwrap();
                        retain_sample(
                            cell,
                            kind == "sample",
                            Sample {
                                seconds,
                                load: detail
                                    .get("load_start")
                                    .and_then(Value::as_f64)
                                    .ok_or("missing sample load")?,
                                counters: sample_counters(&detail, bytes)?,
                            },
                        );
                        (detail, None)
                    }
                    Err(SampleFailure::Environment(e)) => {
                        return Err(format!("environment failure: {e}"));
                    }
                    Err(SampleFailure::Subject(e)) => {
                        unavailable.insert(name.clone());
                        let gap = object([
                            ("subject", name.as_str().into()),
                            ("workload", workload.as_str().into()),
                            ("evidence", id.as_str().into()),
                            ("reason", e.as_str().into()),
                        ]);
                        if name == "herdr-app" {
                            gaps.push(gap);
                        } else {
                            failures.push(gap);
                        }
                        (Value::Object(Vec::new()), Some(e))
                    }
                };
                if let Value::Object(fields) = &mut detail {
                    fields.extend([
                        ("run".into(), id.into()),
                        ("subject".into(), name.as_str().into()),
                        ("workload".into(), workload.as_str().into()),
                        ("kind".into(), kind.into()),
                        ("startup_load".into(), load.into()),
                        ("cleanup_load".into(), end.into()),
                        ("error".into(), failure.into()),
                    ]);
                }
                fs::write(dir.join("sample.json"), detail.to_pretty()).map_err(error)?;
                observations.push(detail);
            }
        }
    }
    let incomplete = incomplete_cells(&mut cells, settings.samples, &observations);
    let complete = incomplete.is_empty() && unavailable.is_empty();
    for cell in &mut cells {
        if cell.workload.starts_with("kitten-") {
            let bytes = cell
                .samples
                .iter()
                .map(|s| s.counters[0].1 as f64)
                .collect::<Vec<_>>();
            cell.input_bytes = Summary::of(&bytes).map_or(0, |s| s.median.round() as u64);
        }
    }
    let result = RunResult {
        suite: "terminals".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: settings.quick,
        corpus_version: corpus::CORPUS_VERSION.into(),
        warmups: settings.warmups,
        samples: settings.samples,
        started_at,
        finished_at: utc_timestamp(SystemTime::now()),
        environment: suite::environment(
            settings,
            environment::kernel(),
            load_start,
            Grid {
                cols: COLS,
                rows: ROWS,
            },
            tools,
            build,
        )?,
        cells,
    };
    let mut json = result.to_json();
    if let Value::Object(fields) = &mut json {
        fields.extend([
            (
                "requested_renderer".into(),
                options.renderer.0.as_str().into(),
            ),
            ("font_family".into(), "DejaVu Sans Mono".into()),
            ("fontconfig".into(), tool("fontconfig")?.display().to_string().into()),
            ("product_fontconfig_mount".into(), tool("product_fontconfig")?.display().to_string().into()),
            ("measurement".into(), "memory-* cells store rollup collection duration in seconds and memory in byte counters. Other workloads store native-VT status-reply completion with rendering enabled; outer foot redraw work is mostly outside the mux timer. Image-display support is not validated, so images is protocol-consumption timing, not an image-rendering rate".into()),
            ("gaps".into(), Value::Array(gaps)),
            ("failures".into(), Value::Array(failures.clone())),
            ("incomplete_cells".into(), Value::Array(incomplete)),
            ("source_tree".into(), options.source_tree.as_str().into()),
            ("source_tree_origin".into(), "Declared --source-tree: git write-tree used for candidate and bench package builds".into()),
            ("invocation".into(), std::env::var("EON_BENCH_INVOCATION").ok().map(|s| Value::parse(&s)).transpose()?.unwrap_or(Value::Null)),
            ("eon_passed".into(), std::env::var("EON_BENCH_EON_PASSED").unwrap_or_else(|_| options.eon.display().to_string()).into()),
            ("eon_canonical".into(), options.eon.display().to_string().into()),
            ("observations".into(), Value::Array(observations)),
            (
                "adapter_preflight".into(),
                if settings.out.join("adapter/result.json").exists() {
                    read(&settings.out.join("adapter/result.json"))?
                } else {
                    Value::Null
                },
            ),
            (
                "result".into(),
                comparison_status(!failures.is_empty(), complete).into(),
            ),
        ]);
    }
    fs::write(settings.out.join("result.json"), json.to_pretty()).map_err(error)?;
    let table = summary(&result.cells);
    fs::write(settings.out.join("summary.tsv"), &table).map_err(error)?;
    fs::write(
        settings.out.join("multiplexer.tsv"),
        multiplexer_summary(&result.cells),
    )
    .map_err(error)?;
    fs::write(
        settings.out.join("window-memory.md"),
        memory_summary(&result.cells)?,
    )
    .map_err(error)?;
    println!("{table}");
    if failures.is_empty() && complete {
        Ok(())
    } else {
        Err("terminal comparison failed or is incomplete; see result.json and subject logs".into())
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or("sample deadline elapsed".into())
}
fn call(launch: &Headless, deadline: Instant, program: &Path, args: &[&str]) -> Result<String> {
    let output = launch
        .command("timeout")
        .arg(format!("{}s", remaining(deadline)?.as_secs_f64()))
        .arg(program)
        .args(args)
        .output()
        .map_err(error)?;
    if !output.status.success() {
        return Err(format!(
            "{} {args:?}: {}: {} {}",
            program.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    String::from_utf8(output.stdout).map_err(error)
}
fn sway(launch: &Headless, deadline: Instant, args: &[&str]) -> Result<Value> {
    Value::parse(&call(launch, deadline, Path::new("swaymsg"), args)?)
}
fn window(tree: &Value) -> Option<(u64, i64, i64)> {
    if tree.get("app_id").and_then(Value::as_str).is_some() {
        let rect = tree.get("rect")?;
        return Some((
            tree.get("id")?.as_u64()?,
            rect.get("width")?.as_u64()? as i64,
            rect.get("height")?.as_u64()? as i64,
        ));
    }
    for key in ["nodes", "floating_nodes"] {
        for node in tree.get(key).and_then(Value::as_array).unwrap_or(&[]) {
            if let Some(found) = window(node) {
                return Some(found);
            }
        }
    }
    None
}
fn grid(text: &str) -> Result<(i64, i64)> {
    let text = text
        .trim()
        .strip_prefix("Some((")
        .and_then(|s| s.strip_suffix("))"))
        .ok_or("no PTY grid report")?;
    let (rows, cols) = text.split_once(',').ok_or("invalid PTY grid")?;
    let rows = rows.trim().parse::<i64>().map_err(error)?;
    let cols = cols.trim().parse::<i64>().map_err(error)?;
    if rows <= 0 || cols <= 0 {
        return Err("empty PTY grid".into());
    }
    Ok((rows, cols))
}

enum SampleFailure {
    Subject(String),
    Environment(String),
}
impl From<String> for SampleFailure {
    fn from(message: String) -> Self {
        Self::Subject(message)
    }
}
impl From<&str> for SampleFailure {
    fn from(message: &str) -> Self {
        Self::Subject(message.into())
    }
}

fn sample(
    options: &Options,
    dir: &Path,
    name: &str,
    program: &Path,
    workload: &str,
    font: &Path,
    kitten_path: &Path,
) -> std::result::Result<(f64, u64, Value), SampleFailure> {
    let deadline = Instant::now() + options.deadline;
    let mut launch = Headless::new(dir, &options.eon, WindowMode::Floating)?;
    let outcome = sample_run(
        options,
        &mut launch,
        name,
        program,
        workload,
        (font, kitten_path),
        deadline,
    );
    // Mux servers can daemonize; remember this sample's marked processes even on errors.
    for entry in fs::read_dir("/proc").map_err(error)? {
        if let Some(pid) = entry
            .ok()
            .and_then(|e| e.file_name().to_str()?.parse().ok())
        {
            launch.processes.remember(pid);
        }
    }
    if !launch.processes.cleanup() {
        return Err(SampleFailure::Environment(
            "private processes still live".into(),
        ));
    }
    outcome
}

fn sample_run(
    options: &Options,
    launch: &mut Headless,
    name: &str,
    program: &Path,
    workload: &str,
    paths: (&Path, &Path),
    deadline: Instant,
) -> std::result::Result<(f64, u64, Value), SampleFailure> {
    let dir = launch.out.clone();
    let dir = dir.as_path();
    let (font, kitten_path) = paths;
    options.renderer.apply(launch)?;
    if options.renderer.0 != "lavapipe" {
        let directory = PathBuf::from(&launch.env["VK_DRIVER_FILES"]);
        let mut manifests = fs::read_dir(directory)
            .map_err(error)?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(error)?
            .into_iter()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|s| s == "json")
                    && !p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .contains("lvp")
            })
            .collect::<Vec<_>>();
        manifests.sort();
        if manifests.is_empty() {
            return Err("no pinned hardware Vulkan ICD".into());
        }
        launch.env.insert(
            "VK_DRIVER_FILES".into(),
            std::env::join_paths(manifests)
                .map_err(error)?
                .to_str()
                .ok_or("non-UTF8 Vulkan manifests")?
                .into(),
        );
    }
    // GL subjects and Venus's Vulkan use the same pinned Mesa, including software mode.
    for (source, target) in [
        ("EON_BENCH_EGL", "__EGL_VENDOR_LIBRARY_FILENAMES"),
        ("EON_BENCH_DRI", "LIBGL_DRIVERS_PATH"),
        ("EON_BENCH_GBM", "GBM_BACKENDS_PATH"),
    ] {
        launch
            .env
            .insert(target.into(), std::env::var(source).map_err(error)?);
    }
    if options.renderer.0 == "lavapipe" {
        launch
            .env
            .insert("LIBGL_ALWAYS_SOFTWARE".into(), "1".into());
        launch
            .env
            .insert("GALLIUM_DRIVER".into(), "llvmpipe".into());
    }
    for (k, v) in [
        ("XDG_SESSION_TYPE", "wayland"),
        ("GDK_BACKEND", "wayland"),
        ("WINIT_UNIX_BACKEND", "wayland"),
        ("RUST_LOG", "info"),
        ("WEZTERM_LOG", "info"),
    ] {
        launch.env.insert(k.into(), v.into());
    }
    launch.env.insert(
        "XDG_CONFIG_HOME".into(),
        dir.join("config").display().to_string(),
    );
    let fonts = dir.join("fonts.conf");
    let font_config = tool("fontconfig")?;
    fs::write(&fonts, format!("<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"fonts.dtd\"><fontconfig><include>{}</include><cachedir>{}/font-cache</cachedir><match target=\"pattern\"><edit name=\"dpi\" mode=\"assign\"><double>96</double></edit></match></fontconfig>", font_config.display(), dir.display())).map_err(error)?;
    launch
        .env
        .insert("FONTCONFIG_FILE".into(), fonts.display().to_string());
    fs::write(dir.join("sway.conf"), "output * mode 1280x800\noutput * scale 1\noutput * bg #000000 solid_color\nseat seat0 fallback true\nxwayland disable\ndefault_border none\nfocus_follows_mouse no\nfor_window [app_id=\".*\"] floating enable\n").map_err(error)?;
    let exe = std::env::current_exe().map_err(error)?;
    fs::write(dir.join("setup.vt"), b"\x1b[0m\x1b[2J\x1b[HEON BENCH READY").map_err(error)?;
    let mut child_args = vec![
        exe.display().to_string(),
        "child".into(),
        dir.display().to_string(),
        "--rows".into(),
        ROWS.to_string(),
        "--cols".into(),
        COLS.to_string(),
        "--grid-report".into(),
        dir.join("grid").display().to_string(),
        "--".into(),
        exe.display().to_string(),
        "terminal-workload".into(),
        dir.display().to_string(),
        workload.into(),
        options.repetitions.to_string(),
        kitten_path.display().to_string(),
        options
            .settings
            .out
            .join("corpus/cat64.vt")
            .display()
            .to_string(),
    ];
    if name == "eon" {
        let index = child_args.iter().position(|s| s == "--").unwrap();
        child_args.insert(index, "--setup".into());
        child_args.insert(index + 1, dir.join("setup.vt").display().to_string());
    }
    let config = dir.join("terminal.conf");
    let mut args: Vec<String> = match if name.starts_with("foot-") {
        "foot"
    } else {
        name
    } {
        "eon" => {
            fs::write(dir.join("config/config.toml"), "[anima]\nenabled = false\n[terminal]\nfont_family = \"DejaVu Sans Mono\"\nfont_size = 16\ncolumns = 80\nrows = 24\nbackground_opacity = 1.0\n").map_err(error)?;
            [vec!["run".into(), "--".into()], child_args.clone()].concat()
        }
        "foot" => {
            fs::write(&config, "[main]\nfont=DejaVu Sans Mono:pixelsize=16\ndpi-aware=no\ninitial-window-size-chars=80x24\npad=0x0\n[scrollback]\nlines=10000\n[cursor]\nblink=no\n").map_err(error)?;
            [
                vec!["--config".into(), config.display().to_string()],
                child_args.clone(),
            ]
            .concat()
        }
        "alacritty" => {
            fs::write(&config, "[font]\nsize = 12.0\n[font.normal]\nfamily = \"DejaVu Sans Mono\"\n[window.dimensions]\ncolumns = 80\nlines = 24\n[scrolling]\nhistory = 10000\n[cursor.style]\nblinking = \"Never\"\n").map_err(error)?;
            [
                vec![
                    "-v".into(),
                    "--config-file".into(),
                    config.display().to_string(),
                    "-e".into(),
                ],
                child_args.clone(),
            ]
            .concat()
        }
        "kitty" => {
            fs::write(&config, "font_family DejaVu Sans Mono\nfont_size 12.0\nscrollback_lines 10000\nremember_window_size no\ninitial_window_width 80c\ninitial_window_height 24c\nwindow_padding_width 0\ncursor_blink_interval 0\nshell_integration disabled\nlinux_display_server wayland\n").map_err(error)?;
            [
                vec!["--config".into(), config.display().to_string()],
                child_args.clone(),
            ]
            .concat()
        }
        "ghostty" => {
            fs::write(&config, "font-family = DejaVu Sans Mono\nfont-size = 12\nwindow-width = 80\nwindow-height = 24\nwindow-padding-x = 0\nwindow-padding-y = 0\nscrollback-limit = 16777216\nshell-integration = none\ncursor-style-blink = false\ngtk-single-instance = false\nconfirm-close-surface = false\n").map_err(error)?;
            [
                vec![
                    "--config-default-files=false".into(),
                    format!("--config-file={}", config.display()),
                    "-e".into(),
                ],
                child_args.clone(),
            ]
            .concat()
        }
        "wezterm" => {
            fs::write(&config, "local w = require 'wezterm'\nreturn { font = w.font('DejaVu Sans Mono'), font_size = 12.0, dpi = 96.0, initial_cols = 80, initial_rows = 24, scrollback_lines = 10000, enable_wayland = true, enable_tab_bar = false, check_for_updates = false, cursor_blink_rate = 0, window_padding = {left=0,right=0,top=0,bottom=0}, front_end = 'OpenGL' }\n").map_err(error)?;
            [
                vec![
                    "--config-file".into(),
                    config.display().to_string(),
                    "start".into(),
                    "--always-new-process".into(),
                    "--".into(),
                ],
                child_args.clone(),
            ]
            .concat()
        }
        "herdr-app" => {
            launch.env.insert(
                "HC_SESSION".into(),
                format!(
                    "eb-{:x}",
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map_err(error)?
                        .as_nanos()
                ),
            );
            launch.env.insert("HERDR_APP_NO_SOUND".into(), "1".into());
            let home = dir.join("config/herdr-app");
            fs::create_dir(&home).map_err(error)?;
            fs::write(
                home.join("shared.json"),
                object([("frame.welcomed", "benchmark".into())]).to_pretty(),
            )
            .map_err(error)?;
            let short = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]).join("herdr");
            std::os::unix::fs::symlink(&home, &short).map_err(error)?;
            launch
                .env
                .insert("HERDR_APP_HOME".into(), short.display().to_string());
            fs::write(
                home.join("settings.json"),
                object([
                    ("font", "DejaVu Sans Mono".into()),
                    ("font_size", 16.0.into()),
                    ("zoom", 1.0.into()),
                    ("telemetry", false.into()),
                    (
                        "core",
                        program
                            .parent()
                            .ok_or("missing Herdr core directory")?
                            .join("herdr-core")
                            .display()
                            .to_string()
                            .into(),
                    ),
                ])
                .to_pretty(),
            )
            .map_err(error)?;
            Vec::new()
        }
        _ => unreachable!(),
    };
    let mux_version = if let Some(mux) = name.strip_prefix("foot-") {
        let mux_path = tool(mux)?;
        let version = call(
            launch,
            deadline,
            &mux_path,
            &[if mux == "tmux" { "-V" } else { "--version" }],
        )?;
        let mux_args = if mux == "tmux" {
            let conf = dir.join("tmux.conf");
            fs::write(
                &conf,
                "set -g status off\nset -g history-limit 10000\nset -g default-size 80x24\n",
            )
            .map_err(error)?;
            [
                vec![
                    mux_path.display().to_string(),
                    "-f".into(),
                    conf.display().to_string(),
                    "-S".into(),
                    Path::new(&launch.env["XDG_RUNTIME_DIR"])
                        .join("tmux.sock")
                        .display()
                        .to_string(),
                    "new-session".into(),
                    "-s".into(),
                    "bench".into(),
                    "--".into(),
                ],
                child_args.clone(),
            ]
            .concat()
        } else {
            let conf = dir.join("zellij.kdl");
            fs::write(&conf, "pane_frames false\nscroll_buffer_size 10000\nsession_serialization false\nshow_release_notes false\nshow_startup_tips false\nweb_server false\n").map_err(error)?;
            let sockets = Path::new(&launch.env["XDG_RUNTIME_DIR"]).join("zellij");
            fs::create_dir(&sockets).map_err(error)?;
            launch
                .env
                .insert("ZELLIJ_SOCKET_DIR".into(), sockets.display().to_string());
            // Start the workload only inside foot, so its grid wait excludes compositor setup.
            vec![
                mux_path.display().to_string(),
                "--config".into(),
                conf.display().to_string(),
                "--layout-string".into(),
                memory::zellij_layout(&child_args),
                "attach".into(),
                "--create".into(),
                "bench".into(),
            ]
        };
        args.truncate(2); // Keep foot's config arguments; replace only its child argv.
        args.extend(mux_args);
        Some(version)
    } else {
        None
    };
    let mut core_stop = if name == "herdr-app" {
        Some(CoreStop::new(
            launch,
            &program
                .parent()
                .ok_or("missing Herdr core directory")?
                .join("hc"),
            dir,
        ))
    } else {
        None
    };
    let mut version = call(
        launch,
        deadline,
        program,
        &[if name == "eon" {
            "versions"
        } else {
            "--version"
        }],
    )?;
    if name == "herdr-app" {
        let running = version
            .trim()
            .strip_prefix("herdr-app ")
            .ok_or("unknown Herdr version format")?;
        fs::write(
            dir.join("config/herdr-app/update-notes.json"),
            object([
                ("last_run", running.into()),
                ("presented", Value::Array(Vec::new())),
            ])
            .to_pretty(),
        )
        .map_err(error)?;
        version.push_str(&call(
            launch,
            deadline,
            &program
                .parent()
                .ok_or("missing Herdr core directory")?
                .join("herdr-core"),
            &["--version"],
        )?);
    }
    fs::write(dir.join("version.txt"), &version).map_err(error)?;
    if let Some(mux) = mux_version {
        version.push_str(&format!("\n{mux}"));
        fs::write(dir.join("version.txt"), &version).map_err(error)?;
    }
    launch.start_compositor_until(BusPolicy::Private, deadline)?;
    let outputs = sway(launch, deadline, &["-t", "get_outputs", "-r"])?;
    let active = outputs
        .as_array()
        .ok_or("missing compositor outputs")?
        .iter()
        .filter(|o| o.get("active").and_then(Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    if active.len() != 1
        || active[0].get("current_mode").is_none_or(|m| {
            m.get("width").and_then(Value::as_u64) != Some(1280)
                || m.get("height").and_then(Value::as_u64) != Some(800)
        })
        || active[0].get("scale").and_then(Value::as_f64) != Some(1.0)
    {
        return Err("private compositor is not one 1280x800 output at scale 1".into());
    }
    let matched_font = call(
        launch,
        deadline,
        &tool("fc_match")?,
        &["-f", "%{file}", "DejaVu Sans Mono:style=Book"],
    )?;
    if fs::canonicalize(matched_font.trim()).map_err(error)?
        != fs::canonicalize(font).map_err(error)?
    {
        return Err(format!(
            "font resolved to {matched_font:?}, expected {}",
            font.display()
        )
        .into());
    }
    if name == "eon" {
        launch.start_eon_with(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    } else {
        launch.spawn(
            program.to_str().ok_or("non-UTF8 tool path")?,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            "terminal.log",
        )?;
    }
    let app_index = launch.processes.children.len() - 1;
    let gpu = if name == "eon" {
        Some(
            read(&options.settings.out.join("adapter/result.json"))?
                .get("venus_gpu")
                .and_then(Value::as_str)
                .ok_or("missing preflight GPU identity")?
                .to_owned(),
        )
    } else {
        None
    };
    let mut last_resize = Instant::now() - Duration::from_secs(1);
    let mut herdr_request: Option<usize> = None;
    let mut herdr_issued = false;
    let mut herdr_ready = false;
    let mut dismissed = false;
    let mut settled = None;
    let (window_id, window_width, window_height) = loop {
        suite::interrupt::check()?;
        if Instant::now() >= deadline {
            return Err("terminal never reported an 80x24 grid before deadline".into());
        }
        if launch
            .processes
            .children
            .get_mut(app_index)
            .ok_or("missing terminal process")?
            .try_wait()
            .map_err(error)?
            .is_some()
        {
            return Err(
                "terminal exited before the workload was ready; see terminal.log/eon.log".into(),
            );
        }
        if name == "herdr-app"
            && !herdr_issued
            && start_once(
                &mut herdr_issued,
                window(&sway(launch, deadline, &["-t", "get_tree", "-r"])?).is_some()
                    && call(launch, deadline, program, &["status"]).is_ok(),
            )
        {
            let arguments = object([
                ("name", "bench".into()),
                ("folder", dir.display().to_string().into()),
                (
                    "argv",
                    Value::Array(child_args.iter().map(|s| s.as_str().into()).collect()),
                ),
            ])
            .to_pretty();
            // Create once; the reply may wait for the grid calibration below.
            launch.spawn_separate(
                program.to_str().ok_or("non-UTF8 Herdr path")?,
                &["do", "space.new", &arguments],
                "space.json",
                "space.stderr.log",
            )?;
            herdr_request = Some(launch.processes.children.len() - 1);
        }
        if let Some(index) = herdr_request
            && let Some(status) = launch.processes.children[index].try_wait().map_err(error)?
        {
            let reply = fs::read_to_string(dir.join("space.json")).map_err(error)?;
            if !status.success() {
                return Err(format!("Herdr space.new: {status}: {reply}").into());
            }
            if Value::parse(&reply)?
                .get("terminal")
                .and_then(Value::as_str)
                != Some("t1")
            {
                return Err("Herdr startup did not return its first terminal t1".into());
            }
            herdr_ready = true;
            herdr_request = None;
        }
        if name == "eon" && !dismissed {
            match call(
                launch,
                deadline,
                &launch.eon,
                &[
                    "tab",
                    "directory",
                    "t1",
                    "--json",
                    "--",
                    dir.to_str().ok_or("non-UTF8 output")?,
                ],
            ) {
                Ok(reply) => {
                    fs::write(dir.join("startup.json"), reply).map_err(error)?;
                    dismissed = true;
                }
                Err(e) => fs::write(dir.join("startup-probe.log"), e).map_err(error)?,
            }
        }
        if last_resize.elapsed() > Duration::from_millis(150) {
            let native = window(&sway(launch, deadline, &["-t", "get_tree", "-r"])?);
            let actual = fs::read_to_string(dir.join("grid"))
                .ok()
                .and_then(|s| grid(&s).ok());
            if dir.join("ready").exists()
                && native.is_some()
                && actual == Some((ROWS as i64, COLS as i64))
                && (name != "eon" || gpu.is_some())
                && (name != "herdr-app" || herdr_ready)
            {
                let since = settled.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(1) {
                    break native.ok_or("native window disappeared")?;
                }
            } else {
                settled = None;
            }
            if let Ok(actual) = fs::read_to_string(dir.join("grid"))
                && let Ok((rows, cols)) = grid(&actual)
                && (rows, cols) != (ROWS as i64, COLS as i64)
                && let Some((id, width, height)) = native
            {
                // DejaVu at 16px: use the observed grid for correction; the child is the oracle.
                let width = (width + (COLS as i64 - cols) * 10).clamp(100, 1260);
                let height = (height + (ROWS as i64 - rows) * 19).clamp(100, 780);
                sway(
                    launch,
                    deadline,
                    &[&format!(
                        "[con_id={id}] resize set width {width} px height {height} px, move position center"
                    )],
                )?;
            }
            last_resize = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(20).min(remaining(deadline)?));
    };
    if name == "eon" {
        let gpu_value = gpu.as_deref().ok_or("Venus provided no GPU identity")?;
        if gpu_value.starts_with("software=true ") != (options.renderer.0 == "lavapipe") {
            return Err(format!("renderer mismatch: {gpu_value}").into());
        }
    }
    suite::wait_for_load_until("terminals", &options.settings, deadline)
        .map_err(SampleFailure::Environment)?;
    child::publish(&dir.join("go"), "")?;
    while !dir.join("done").exists() {
        suite::interrupt::check()?;
        if Instant::now() >= deadline {
            return Err("workload overran sample deadline (includes blocked writes)".into());
        }
        std::thread::sleep(Duration::from_millis(10).min(remaining(deadline)?));
    }
    if fs::read_to_string(dir.join("done")).map_err(error)?.trim() != "0" {
        return Err("workload failed; see terminal.log/eon.log".into());
    }
    let report = read(&dir.join("report.json"))?;
    let (mut seconds, mut bytes) = measurement(&report)?;
    let load = read(&dir.join("load.json"))?;
    if name == "eon" {
        let text = call(launch, deadline, &launch.eon, &["pane", "read", "p1"])?;
        fs::write(dir.join("visible.txt"), &text).map_err(error)?;
        if workload == "memory-filled" {
            memory::verify_screen(&text)?;
        } else if workload == "memory-empty" {
            if !text.trim().is_empty() {
                return Err("empty window contains unexpected text".into());
            }
        } else if !text.contains(MARKER) {
            return Err("Eon visible text does not contain the final marker".into());
        }
    }
    let memory = if workload.starts_with("memory-") {
        std::thread::sleep(Duration::from_secs(1).min(remaining(deadline)?));
        remaining(deadline)?;
        let held = fs::read_to_string(dir.join("ready"))
            .map_err(error)?
            .trim()
            .parse()
            .map_err(error)?;
        let memory = memory::window::measure(
            launch.processes.children[app_index].id(),
            held,
            name == "eon",
            dir,
        )?;
        seconds = memory
            .get("collection_seconds")
            .and_then(Value::as_f64)
            .ok_or("missing memory collection time")?;
        bytes = 0;
        memory
    } else {
        Value::Null
    };
    // Retain an image of the final marker without putting screencopy in the timed region.
    std::thread::sleep(Duration::from_millis(200).min(remaining(deadline)?));
    call(
        launch,
        deadline,
        Path::new("grim"),
        &[dir.join("final.png").to_str().ok_or("non-UTF8 output")?],
    )?;
    let logs = fs::read_to_string(dir.join(if name == "eon" {
        "eon.log"
    } else {
        "terminal.log"
    }))
    .unwrap_or_default();
    let adapter = logs
        .lines()
        .filter(|s| {
            let l = s.to_lowercase();
            ["renderer", "opengl", "vulkan", "adapter", "llvmpipe", "gpu"]
                .iter()
                .any(|p| l.contains(p))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if options.renderer.0 != "lavapipe"
        && name != "foot"
        && (adapter.to_lowercase().contains("llvmpipe")
            || adapter.to_lowercase().contains("softpipe"))
    {
        return Err("hardware run fell back to software; see terminal.log".into());
    }
    let detail = object([
        ("seconds", seconds.into()),
        ("input_bytes", bytes.into()),
        ("memory", memory),
        (
            "load_start",
            load.get("start").cloned().ok_or("missing start load")?,
        ),
        (
            "load_end",
            load.get("end").cloned().ok_or("missing end load")?,
        ),
        ("version", version.trim().into()),
        ("outputs", outputs),
        (
            "native_window",
            object([
                ("id", window_id.into()),
                ("width", (window_width as u64).into()),
                ("height", (window_height as u64).into()),
            ]),
        ),
        (
            "grid",
            fs::read_to_string(dir.join("grid"))
                .map_err(error)?
                .trim()
                .into(),
        ),
        (
            "adapter_evidence",
            adapter_evidence(name, gpu.as_deref(), &adapter).into(),
        ),
        ("venus_gpu", gpu.into()),
        (
            "venus_gpu_evidence",
            if name == "eon" {
                "adapter/result.json (separate untimed preflight; same Nix candidate and renderer)"
            } else {
                "not an Eon subject"
            }
            .into(),
        ),
        ("adapter_log", adapter.into()),
        (
            "compositor_renderer",
            launch.env["WLR_RENDERER"].as_str().into(),
        ),
        (
            "vulkan_icd",
            launch.env.get("VK_DRIVER_FILES").cloned().into(),
        ),
        ("font_file", font.display().to_string().into()),
        (
            "herdr_session",
            launch.env.get("HC_SESSION").cloned().into(),
        ),
        ("matched_font", matched_font.into()),
        (
            "oracle",
            if workload.starts_with("memory-") {
                "DSR after memory payload; history preserved; final.png retained"
            } else {
                "DSR after final marker; final.png retained (not a pixel-latency measurement)"
            }
            .into(),
        ),
    ]);
    child::publish(&dir.join("release"), "")?;
    if name == "eon" {
        launch.stop_eon()?;
    } else if let Some(stop) = &mut core_stop {
        stop.stop()?;
    }
    Ok((seconds, bytes, detail))
}

fn measurement(report: &Value) -> Result<(f64, u64)> {
    let seconds = report
        .get("seconds")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .ok_or("invalid completion time")?;
    let bytes = report
        .get("bytes")
        .and_then(Value::as_u64)
        .filter(|b| *b > 0)
        .ok_or("invalid byte count")?;
    Ok((seconds, bytes))
}

/// Runs inside a reference terminal through the shared grid/go/release child.
pub fn workload(args: &[String]) -> Result {
    let [dir, name, repetitions, kitten_path, cat64] = args else {
        return Err("terminal-workload needs DIR NAME REPETITIONS KITTEN CAT64".into());
    };
    let dir = Path::new(dir);
    writer::run(&dir.join("setup.vt"), &dir.join("setup-final.json"))?;
    let start_load = environment::load()?;
    if name.starts_with("memory-") {
        let payload = dir
            .parent()
            .ok_or("missing suite directory")?
            .join(format!("{name}.vt"));
        writer::run(&payload, &dir.join("report.json"))?;
    } else if name == "cat64" {
        writer::run(Path::new(cat64), &dir.join("report.json"))?;
    } else {
        let name = name
            .strip_prefix("kitten-")
            .filter(|n| kitten::BENCHMARKS.contains(n))
            .ok_or("unknown kitten workload")?;
        let output = fs::File::create(dir.join("kitten.txt")).map_err(error)?;
        let status = std::process::Command::new(kitten_path)
            .args([
                "__benchmark__",
                "--render",
                "--repetitions",
                repetitions,
                name,
            ])
            .stdout(Stdio::from(output))
            .status()
            .map_err(error)?;
        if !status.success() {
            return Err(format!("kitten: {status}"));
        }
        let measured = kitten::parse(&fs::read_to_string(dir.join("kitten.txt")).map_err(error)?)?;
        fs::write(
            dir.join("report.json"),
            object([
                ("seconds", measured.seconds.into()),
                ("bytes", measured.bytes.into()),
            ])
            .to_pretty(),
        )
        .map_err(error)?;
    }
    fs::write(
        dir.join("load.json"),
        object([
            ("start", start_load.into()),
            ("end", environment::load()?.into()),
        ])
        .to_pretty(),
    )
    .map_err(error)?;
    if name.starts_with("memory-") {
        return Ok(());
    }
    let marker = dir.join("marker.vt");
    fs::write(
        &marker,
        format!("\x1b[?2026l\x1b[?1049l\x1b[0m\x1b[2J\x1b[H{MARKER}"),
    )
    .map_err(error)?;
    writer::run(&marker, &dir.join("oracle.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_tables_use_memory_bytes_and_complete_same_cohort_foot_times() {
        let mut foot = cell();
        foot.subject = "foot".into();
        foot.samples = vec![sample_value(1.0), sample_value(3.0)];
        let mut mux = foot.clone();
        mux.subject = "foot-tmux".into();
        mux.samples = vec![sample_value(2.0), sample_value(6.0)];
        let table = multiplexer_summary(&[foot.clone(), mux.clone()]);
        assert!(table.contains("foot-tmux\tcat64\t2.000000\t100.000\t0.500000"));
        mux.samples.pop();
        let mut incomplete = vec![foot, mux];
        incomplete_cells(&mut incomplete, 2, &[]);
        assert!(!multiplexer_summary(&incomplete).contains("foot-tmux\t"));
        let mut memory = cell();
        memory.subject = "foot".into();
        memory.workload = "memory-empty".into();
        memory.samples = vec![Sample {
            seconds: 0.001,
            load: 1.0,
            counters: ["pss_bytes", "rss_bytes", "pss_anon_bytes"]
                .map(|k| (k.into(), 2 * 1048576))
                .to_vec(),
        }];
        assert!(
            memory_summary(&[memory.clone()])
                .unwrap()
                .contains("2.000 (2.000–2.000)")
        );
        assert!(!summary(&[memory]).contains("memory-empty"));
    }
    #[test]
    fn mux_and_window_memory_use_the_terminal_rig_selections() {
        let args = "--eon /candidate --out /evidence --source-tree 0000000000000000000000000000000000000000 --subject foot-tmux --subject foot-zellij --only cat64"
            .split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let selected = options(&args).unwrap();
        assert_eq!(selected.subjects, ["foot-tmux", "foot-zellij"]);
        assert_eq!(selected.workloads, ["cat64"]);
        let args = "--eon /candidate --out /evidence --source-tree 0000000000000000000000000000000000000000 --subject eon --subject foot --subject ghostty --only memory-empty --only memory-filled"
            .split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            options(&args).unwrap().workloads,
            ["memory-empty", "memory-filled"]
        );
        let mut bad = args.clone();
        bad.extend(["--subject".into(), "foot-tmux".into()]);
        assert!(options(&bad).is_err());
    }
    #[test]
    fn selections_and_measurements_reject_invalid_or_empty_samples() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let base = "--eon /candidate --out /evidence --source-tree 0000000000000000000000000000000000000000";
        let selected = options(&args(&format!(
            "{base} --quick --only cat64 --subject foot"
        )))
        .unwrap();
        assert_eq!(selected.repetitions, 1);
        assert_eq!(selected.subjects, ["foot"]);
        for bad in [
            "--subject zed",
            "--only kitten-wrong",
            "--deadline 0",
            "--kitten-repetitions 0",
            "--subject foot --subject foot",
        ] {
            assert!(options(&args(&format!("{base} {bad}"))).is_err());
        }
        assert!(options(&args("--eon /candidate --out /evidence")).is_err());
        assert!(
            options(&args("--eon /candidate --out /evidence --source-tree bad"))
                .err()
                .unwrap()
                .contains("40-digit")
        );
        assert_eq!(grid("Some((24, 80))").unwrap(), (24, 80));
        for bad in ["None", "Some((0, 80))", "24 80"] {
            assert!(grid(bad).is_err());
        }
        assert_eq!(
            measurement(&Value::parse("{\"seconds\":0.5,\"bytes\":128}").unwrap()).unwrap(),
            (0.5, 128)
        );
        for bad in [
            "{\"seconds\":0,\"bytes\":128}",
            "{\"seconds\":1,\"bytes\":0}",
            "{}",
        ] {
            assert!(measurement(&Value::parse(bad).unwrap()).is_err());
        }
    }

    fn cell() -> Cell {
        Cell {
            workload: "cat64".into(),
            corpus_sha256: "fixture".into(),
            input_bytes: 128,
            layer: "render-enabled-completion".into(),
            subject: "herdr-app".into(),
            subject_version: "fixture".into(),
            samples: Vec::new(),
        }
    }
    fn sample_value(seconds: f64) -> Sample {
        Sample {
            seconds,
            load: 1.0,
            counters: vec![("input_bytes".into(), 128)],
        }
    }

    #[test]
    fn warmups_do_not_enter_published_samples() {
        let mut cell = cell();
        retain_sample(&mut cell, false, sample_value(99.0));
        retain_sample(&mut cell, true, sample_value(1.0));
        assert_eq!(cell.samples.len(), 1);
        assert_eq!(cell.samples[0].seconds, 1.0);
        assert_eq!(
            cell.to_json()
                .get("stats")
                .unwrap()
                .get("seconds")
                .unwrap()
                .get("median")
                .unwrap()
                .as_f64(),
            Some(1.0)
        );
    }

    #[test]
    fn a_failure_after_one_sample_publishes_no_partial_cell_statistics() {
        let mut cells = vec![cell()];
        retain_sample(&mut cells[0], true, sample_value(1.0));
        let observations = vec![
            object([
                ("subject", "herdr-app".into()),
                ("workload", "cat64".into()),
                ("seconds", 1.0.into()),
                ("error", Value::Null),
            ]),
            object([
                ("subject", "herdr-app".into()),
                ("workload", "cat64".into()),
                ("error", "fixture failure".into()),
            ]),
        ];
        let incomplete = incomplete_cells(&mut cells, 2, &observations);
        assert_eq!(incomplete.len(), 1);
        assert_eq!(
            incomplete[0].get("status").and_then(Value::as_str),
            Some("incomplete")
        );
        assert_eq!(
            incomplete[0]
                .get("observed_samples")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            incomplete[0]
                .get("observations")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(cells[0].samples.is_empty());
        assert!(matches!(cells[0].to_json().get("stats"), Some(Value::Null)));
        assert!(!summary(&cells).contains("herdr-app"));
        assert_eq!(
            comparison_status(false, incomplete.is_empty()),
            "incomplete"
        );
        assert_eq!(comparison_status(true, false), "fail");
    }

    #[test]
    fn every_round_rotates_the_first_subject() {
        assert_eq!(subject_order(0, 3).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(subject_order(1, 3).collect::<Vec<_>>(), [1, 2, 0]);
        assert_eq!(subject_order(2, 3).collect::<Vec<_>>(), [2, 0, 1]);
        assert_eq!(subject_order(3, 3).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn herdr_space_creation_is_issued_once_even_while_the_reply_is_pending() {
        let mut issued = false;
        assert!(!start_once(&mut issued, false));
        assert!(start_once(&mut issued, true));
        assert!(!start_once(&mut issued, true));
        assert!(!start_once(&mut issued, false));
        assert!(!start_once(&mut issued, true));
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "terminal-rule-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn core_stop_guard_runs_on_error_and_keeps_stderr_out_of_json() {
        let dir = tempdir();
        let result: Result = {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                r#"printf '{"stopped":true}'; printf 'fixture warning\n' >&2"#,
            ]);
            let _stop = CoreStop {
                command,
                dir: dir.clone(),
                attempted: false,
            };
            Err("fixture startup error".into())
        };
        assert!(result.is_err());
        assert_eq!(
            read(&dir.join("core-stop.txt"))
                .unwrap()
                .get("stopped")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            fs::read_to_string(dir.join("core-stop.stderr.log")).unwrap(),
            "fixture warning\n"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_short_load_deadline_remains_an_environment_failure() {
        let settings = suite::Settings {
            out: PathBuf::new(),
            samples: 1,
            warmups: 0,
            max_load: -1.0,
            load_wait: Duration::from_secs(600),
            cpus: None,
            quick: true,
        };
        let start = Instant::now();
        let result =
            suite::wait_for_load_until("fixture", &settings, start + Duration::from_millis(30))
                .map_err(SampleFailure::Environment);
        assert!(matches!(result, Err(SampleFailure::Environment(_))));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wrapper_exports_and_adapter_evidence_are_explicit() {
        assert_eq!(
            wrapper_path(
                "export FONTCONFIG_FILE='/nix/store/fixture-fonts'",
                "FONTCONFIG_FILE"
            )
            .unwrap(),
            Path::new("/nix/store/fixture-fonts")
        );
        assert!(
            wrapper_path(
                "export FONTCONFIG_FILE='/home/user/fonts'",
                "FONTCONFIG_FILE"
            )
            .is_err()
        );
        assert!(wrapper_path("", "FONTCONFIG_FILE").is_err());
        assert!(
            wrapper_path(
                "export FONTCONFIG_FILE='/nix/store/a'\nexport FONTCONFIG_FILE='/nix/store/b'",
                "FONTCONFIG_FILE"
            )
            .is_err()
        );
        assert_eq!(
            adapter_evidence("kitty", None, ""),
            "no-software-renderer-seen"
        );
        assert_eq!(
            adapter_evidence("alacritty", None, "GL renderer: Intel(R) Graphics"),
            "positive-hardware"
        );
        assert_eq!(
            adapter_evidence("kitty", None, "GL renderer: llvmpipe"),
            "positive-software"
        );
        assert_eq!(adapter_evidence("foot", None, ""), "cpu");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn external_probes_use_the_remaining_sample_budget() {
        let dir = tempdir();
        // eon-test owns this empty /run; production display isolation creates the parent.
        // SAFETY: geteuid has no side effects or arguments.
        fs::create_dir_all(format!("/run/user/{}", unsafe { libc::geteuid() })).unwrap();
        let candidate = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("true"))
            .find(|p| p.starts_with("/nix/store") && p.is_file())
            .unwrap();
        let launch = Headless::new(&dir, &candidate, WindowMode::Floating).unwrap();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(60);
        std::thread::sleep(Duration::from_millis(30));
        assert!(call(&launch, deadline, Path::new("sleep"), &["10"]).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(remaining(deadline).is_err());
        drop(launch);
        fs::remove_dir_all(dir).unwrap();
    }
}
