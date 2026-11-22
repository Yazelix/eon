//! Venus frame work and process CPU on the shared isolated compositor.
#[cfg(target_os = "linux")]
use crate::{
    child, corpus,
    launcher::{BusPolicy, Headless, WindowMode},
    result::{Grid, RunResult, utc_timestamp},
};
#[cfg(any(target_os = "linux", test))]
use crate::{environment, stats::Summary};
use crate::{isolation, json::Value, launcher::Renderer, suite, terminals};
#[cfg(target_os = "linux")]
use std::time::{Instant, SystemTime};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(any(target_os = "linux", test))]
mod diagnostics;
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
pub use native::workload;
type Result<T = ()> = std::result::Result<T, String>;
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
#[cfg(target_os = "linux")]
fn read(path: &Path) -> Result<Value> {
    Value::parse(&fs::read_to_string(path).map_err(error)?)
}
fn hash(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(error)?;
    let mut hash = crate::sha256::Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(error)?;
        if n == 0 {
            return Ok(hash.hex());
        }
        hash.update(&bytes[..n]);
    }
}
const USAGE: &str = "eon-bench frames --eon LABEL=PATH --revision LABEL=COMMIT --candidate-tree LABEL=TREE --source-tree BENCH_TREE --out NEW_DIR [OPTIONS]";
const WORKLOADS: [&str; 10] = [
    "cat64",
    "kitten-ascii",
    "kitten-unicode",
    "kitten-csi",
    "kitten-images",
    "kitten-long_escape_codes",
    "scroll-wheel",
    "scroll-pixel",
    "idle-blink",
    "idle-steady",
];
#[derive(Clone)]
struct Candidate {
    label: String,
    path: PathBuf,
    #[cfg(target_os = "linux")]
    revision: String,
    #[cfg(target_os = "linux")]
    tree: String,
}
pub struct Options {
    settings: suite::Settings,
    candidates: Vec<Candidate>,
    #[cfg(target_os = "linux")]
    source_tree: String,
    renderer: Renderer,
    #[cfg(any(target_os = "linux", test))]
    cells: Vec<Cell>,
    #[cfg(target_os = "linux")]
    scroll_seconds: u64,
    #[cfg(target_os = "linux")]
    idle_seconds: u64,
    #[cfg(target_os = "linux")]
    repetitions: u64,
    #[cfg(target_os = "linux")]
    deadline: Duration,
    #[cfg(target_os = "linux")]
    mode: String,
}
#[cfg(target_os = "linux")]
#[derive(Clone)]
struct Cell {
    workload: String,
    columns: u64,
    width: u64,
    height: u64,
}
// Other targets still validate the matrix size during argument parsing.
#[cfg(not(target_os = "linux"))]
struct Cell;

#[cfg(target_os = "linux")]
impl Cell {
    fn new(workload: String, columns: u64, width: u64, height: u64) -> Self {
        Self {
            workload,
            columns,
            width,
            height,
        }
    }

    fn id(&self) -> String {
        format!(
            "{}-{}c-{}x{}",
            self.workload, self.columns, self.width, self.height
        )
    }
    fn scroll(&self) -> bool {
        self.workload.starts_with("scroll-")
    }
    fn idle(&self) -> bool {
        self.workload.starts_with("idle-")
    }
    fn throughput(&self) -> bool {
        !self.scroll() && !self.idle()
    }
}
#[cfg(not(target_os = "linux"))]
impl Cell {
    fn new(_: String, _: u64, _: u64, _: u64) -> Self {
        Self
    }
}
fn help() -> String {
    format!(
        "{USAGE}\n\n\
Linux/Wayland only. Private Sway/runtime/HOME; never the desktop or profile.\n\
  --eon LABEL=PATH          repeat up to two labelled Nix candidate executables\n\
  --revision LABEL=COMMIT   candidate git revision, separately from the bench\n\
  --candidate-tree LABEL=TREE  exact candidate source tree (required per label)\n\
  --source-tree TREE        exact tree used for this benchmark build\n\
  --renderer MODE          lavapipe (default) or /dev/dri/renderD<N>\n\
  --only WORKLOAD           repeat; default cat64, all five kitten workloads,\n\
                            scroll-wheel, scroll-pixel, idle-blink, idle-steady\n\
  --columns N              1 or 4; repeat (default both; scroll/idle only)\n\
  --size WxH               1280x720 or 2560x1440; repeat (default both)\n\
  --scroll-seconds N        1..30 (default 5); 30 Hz, one step or 4 px/event\n\
  --idle-seconds N          1..120 (default 30; 2 with --quick)\n\
  --kitten-repetitions N    1..100 (default 10; 1 with --quick)\n\
  --deadline SECONDS        10..600 per sample (default 180)\n\
  --mode MODE              measure (default), check (untimed correctness),\n\
                            control (logger off; CPU/writer completion only)\n\n\
Candidates rotate inside every cell/round. Timings require a booked quiet slot. Frames refuses high load immediately, never waiting inside a heavy slot; --load-wait is inapplicable.\n\
Frame host work = scene+render+submit elapsed time; rebuild/prepare are included\n\
in render. Acquisition is separate. These are not GPU/scanout latency. CPU is\n\
Venus utime+stime, 100% = one core. p99 is null below 100 frames; n is recorded.\n\
Scroll includes a separately reported 2-second post-stop kinetic tail.\n\
Each sample needs complete logger v2 start/frame/end records with zero loss.\n\
500000 frames/sample and pooled cell; <=1000 runs, <128 MiB numeric working set.\n\
Check mode uses one sample, protocol observation and no published timing stats.\n\
Control mode supplies no frame statistics and is not a frame baseline.\n\n{}",
        suite::settings_help()
    )
}
fn labelled(value: String) -> Result<(String, String)> {
    let (label, value) = value.split_once('=').ok_or("expected LABEL=VALUE")?;
    if label.is_empty()
        || label.len() > 32
        || !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || value.is_empty()
    {
        return Err(
            "label must be 1..32 ASCII letters/digits, '-' or '_' with a nonempty value".into(),
        );
    }
    Ok((label.into(), value.into()))
}
fn identity(value: String) -> Result<String> {
    if value.len() != 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("identity needs a full 40-digit git hash".into());
    }
    Ok(value.to_lowercase())
}
pub fn options(args: &[String]) -> Result<Options> {
    let (mut settings, flags) = suite::parse(
        args,
        USAGE,
        help,
        &[
            "--eon",
            "--revision",
            "--candidate-tree",
            "--source-tree",
            "--renderer",
            "--only",
            "--columns",
            "--size",
            "--scroll-seconds",
            "--idle-seconds",
            "--kitten-repetitions",
            "--deadline",
            "--mode",
        ],
    )?;
    let (mut paths, mut revisions, mut trees) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let (mut singles, mut workloads, mut columns, mut sizes) =
        (BTreeMap::new(), Vec::new(), Vec::new(), Vec::new());
    for (flag, value) in flags {
        match flag.as_str() {
            "--eon" | "--revision" | "--candidate-tree" => {
                let (label, value) = labelled(value)?;
                let map = match flag.as_str() {
                    "--eon" => &mut paths,
                    "--revision" => &mut revisions,
                    _ => &mut trees,
                };
                let value = if flag == "--eon" {
                    value
                } else {
                    identity(value)?
                };
                if map.insert(label, value).is_some() {
                    return Err(format!("repeated {flag} label"));
                }
            }
            "--only" => {
                if !WORKLOADS.contains(&value.as_str()) || workloads.contains(&value) {
                    return Err("unknown or repeated workload".into());
                }
                workloads.push(value);
            }
            "--columns" => {
                let n = suite::number(&flag, &value)?;
                if ![1, 4].contains(&n) || columns.contains(&n) {
                    return Err("columns must select 1 or 4, without repetition".into());
                }
                columns.push(n);
            }
            "--size" => {
                let n = match value.as_str() {
                    "1280x720" => (1280, 720),
                    "2560x1440" => (2560, 1440),
                    _ => return Err("size must be 1280x720 or 2560x1440".into()),
                };
                if sizes.contains(&n) {
                    return Err("repeated size".into());
                }
                sizes.push(n);
            }
            _ => {
                if singles.insert(flag.clone(), value).is_some() {
                    return Err(format!("repeated {flag}"));
                }
            }
        }
    }
    if paths.is_empty()
        || paths.len() > 2
        || paths.keys().ne(revisions.keys())
        || paths.keys().ne(trees.keys())
    {
        return Err("supply one or two candidates, each with matching --revision and --candidate-tree labels".into());
    }
    let source_tree = identity(
        singles
            .remove("--source-tree")
            .ok_or("--source-tree is required")?,
    )?;
    let renderer = Renderer(
        singles
            .remove("--renderer")
            .unwrap_or_else(|| "lavapipe".into()),
    );
    renderer.render_node()?;
    let mode = singles.remove("--mode").unwrap_or_else(|| "measure".into());
    if !["measure", "check", "control"].contains(&mode.as_str()) {
        return Err("mode must be measure, check or control".into());
    }
    if mode == "check" {
        settings.samples = 1;
        settings.warmups = 0;
    }
    let mut number = |key: &str, default, min, max| -> Result<u64> {
        let n = singles
            .remove(key)
            .map(|v| suite::number(key, &v))
            .transpose()?
            .unwrap_or(default);
        if n < min || n > max {
            return Err(format!("{key} must be {min}..{max}"));
        }
        Ok(n)
    };
    let scroll_seconds = number(
        "--scroll-seconds",
        if settings.quick { 1 } else { 5 },
        1,
        30,
    )?;
    let idle_seconds = number(
        "--idle-seconds",
        if settings.quick { 2 } else { 30 },
        1,
        120,
    )?;
    let repetitions = number(
        "--kitten-repetitions",
        if settings.quick { 1 } else { 10 },
        1,
        100,
    )?;
    let deadline = Duration::from_secs(number("--deadline", 180, 10, 600)?);
    if workloads.is_empty() {
        workloads = WORKLOADS.iter().map(|s| (*s).into()).collect();
    }
    if columns.is_empty() {
        columns = vec![1, 4];
    }
    if sizes.is_empty() {
        sizes = vec![(1280, 720), (2560, 1440)];
    }
    let mut cells = Vec::new();
    for workload in workloads {
        if workload == "cat64" || workload.starts_with("kitten-") {
            cells.push(Cell::new(workload, 1, 1280, 800));
        } else {
            for &columns in &columns {
                for &(width, height) in &sizes {
                    cells.push(Cell::new(workload.clone(), columns, width, height));
                }
            }
        }
    }
    let runs = settings
        .samples
        .checked_add(settings.warmups)
        .and_then(|n| n.checked_mul(cells.len() as u64 * paths.len() as u64))
        .ok_or("run count overflow")?;
    if runs > 1000 || settings.warmups > 10 {
        return Err("frames run budget is <=1000 runs and <=10 warmups".into());
    }
    let candidates = paths
        .into_iter()
        .map(|(label, path)| Candidate {
            path: path.into(),
            #[cfg(target_os = "linux")]
            revision: revisions[&label].clone(),
            #[cfg(target_os = "linux")]
            tree: trees[&label].clone(),
            label,
        })
        .collect();
    #[cfg(not(target_os = "linux"))]
    let _ = (
        source_tree,
        scroll_seconds,
        idle_seconds,
        repetitions,
        deadline,
    );
    Ok(Options {
        settings,
        candidates,
        #[cfg(target_os = "linux")]
        source_tree,
        renderer,
        #[cfg(any(target_os = "linux", test))]
        cells,
        #[cfg(target_os = "linux")]
        scroll_seconds,
        #[cfg(target_os = "linux")]
        idle_seconds,
        #[cfg(target_os = "linux")]
        repetitions,
        #[cfg(target_os = "linux")]
        deadline,
        #[cfg(target_os = "linux")]
        mode,
    })
}

pub fn run(args: &[String], options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("frames requires Linux/Wayland".into());
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.settings.out)
        .map_err(error)?;
    let out = fs::canonicalize(&options.settings.out).map_err(error)?;
    let outcome = (|| -> Result {
        let mut arguments = args.to_vec();
        let mut mounts = Vec::new();
        for candidate in &options.candidates {
            let path = crate::launcher::candidate(&candidate.path)?;
            mounts.push(terminals::candidate_fontconfig(&path)?);
            let passed = format!("{}={}", candidate.label, candidate.path.display());
            let index = arguments
                .iter()
                .position(|a| a == &passed)
                .ok_or("missing candidate argument")?;
            arguments[index] = format!("{}={}", candidate.label, path.display());
        }
        let index = arguments
            .iter()
            .position(|a| a == "--out")
            .ok_or("missing --out")?;
        arguments[index + 1] = out.to_str().ok_or("non-UTF8 output path")?.into();
        let exe = std::env::current_exe().map_err(error)?;
        let mut command = isolation::command(&out, &[&exe]);
        isolation::display(&mut command, options.renderer.render_node()?);
        for mount in mounts {
            command
                .arg("--ro-bind")
                .arg(terminals::tool("frames_fontconfig")?)
                .arg(mount);
        }
        command
            .arg("--")
            .arg(exe)
            .arg("frames-inner")
            .args(arguments);
        isolation::run(&mut command)
    })();
    if let Err(e) = &outcome
        && !out.join("result.json").exists()
    {
        fs::write(
            out.join("result.json"),
            object([
                ("schema", 1u64.into()),
                ("tool", "eon-bench-frames".into()),
                ("suite", "frames".into()),
                ("result", "fail".into()),
                ("error", e.as_str().into()),
                (
                    "scope",
                    "candidate/isolation preflight; no measurements".into(),
                ),
                ("cells", Value::Array(Vec::new())),
            ])
            .to_pretty(),
        )
        .map_err(error)?;
        fs::write(
            out.join("SHA256SUMS"),
            format!("{}  result.json\n", hash(&out.join("result.json"))?),
        )
        .map_err(error)?;
    }
    suite::interrupt::resend();
    outcome
}

pub fn run_inner(options: &Options) -> Result {
    #[cfg(target_os = "linux")]
    {
        native::run(options)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = options;
        Err("frames requires Linux/Wayland".into())
    }
}
#[cfg(not(target_os = "linux"))]
pub fn workload(_: &[String]) -> Result {
    Err("frames workload requires Linux".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_labels_need_separate_exact_identities_and_bound_the_matrix() {
        let hash = "0".repeat(40);
        let mut args = format!("--out /x --eon edge=/nix/store/c/bin/eon --revision edge={hash} --candidate-tree edge={hash} --source-tree {hash}").split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(options(&args).unwrap().cells.len(), 22);
        args.extend(["--eon".into(), "parent=/old".into()]);
        assert!(options(&args).is_err());
        args.extend([
            "--revision".into(),
            format!("parent={hash}"),
            "--candidate-tree".into(),
            format!("parent={hash}"),
        ]);
        assert_eq!(options(&args).unwrap().candidates.len(), 2);
        args.extend(["--samples".into(), "1000".into()]);
        assert!(options(&args).is_err());
    }
}
