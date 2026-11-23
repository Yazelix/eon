//! Installed Eon journeys on the shared private display.
mod first_launch;
mod tutor;
mod update_direct;

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

use crate::{
    bundle::{self, Bundle, HostSet},
    child, environment, isolation,
    json::{Value, object},
    launcher::{self, BusPolicy, Headless, OutputSize, Renderer, WindowMode},
    picture::Picture,
    suite,
};

type Result<T = ()> = std::result::Result<T, String>;

const HELP: &str = "\
eon-bench journey (--eon PATH | --bundle ARCHIVE) --case first-launch --out DIR
eon-bench journey --eon PATH --case tutor --out DIR [--output-size WIDTHxHEIGHT]
eon-bench journey --bundle ARCHIVE --next NEXT --case update-direct --out DIR

Exercise an installed Eon candidate in private headless Sway (default 1280x800).
PATH is a Nix candidate's bin/eon. ARCHIVE is the Nix-free release archive
(eon-VERSION-x86_64-linux.tar.zst from .#manual-bundle): it runs extracted, in
a jail with no /nix, on the host's glibc and Mesa lavapipe (a merged-/usr host).
update-direct installs ARCHIVE with its own installer, then updates it to NEXT,
a newer build of the same source, through eon update and a fake release
network (no window). DIR must not exist. Linux only.
Writes result.json, capture-*.png and private launch logs.
Exit 0 on a passing case and Stop, 1 on a run failure, 2 on invalid arguments.";

const PARENT_PID_NS: &str = "EON_BENCH_JOURNEY_PARENT_PID_NS";
const PRODUCT_ROOT: &str = "EON_BENCH_JOURNEY_PRODUCT_ROOT";
const HOST_SET: &str = "EON_BENCH_JOURNEY_HOST_SET";
const NEXT_SOURCE: &str = "EON_BENCH_JOURNEY_NEXT_SOURCE";
const ARCHIVE: &str = "/bundle.tar.zst";
const NEXT: &str = "/next.tar.zst";
/// Lines of `eon.log` a failed wait carries.
const LOG_TAIL: usize = 20;
/// The extraction, under the evidence directory.
const BUNDLE_ROOT: &str = "bundle";
const FIRST_LAUNCH: &str = "first-launch";
const TUTOR: &str = "tutor";
const UPDATE_DIRECT: &str = "update-direct";
const CALL_LIMIT: Duration = Duration::from_secs(10);
const CAPTURE_LIMIT: Duration = Duration::from_secs(10);
const EXTRACT_LIMIT: Duration = Duration::from_secs(60);

enum Input {
    Nix(PathBuf),
    Bundle(PathBuf),
}

impl Input {
    fn path(&self) -> &Path {
        match self {
            Self::Nix(path) | Self::Bundle(path) => path,
        }
    }

    fn mode(&self) -> &'static str {
        match self {
            Self::Nix(_) => "nix",
            Self::Bundle(_) => "bundle",
        }
    }
}

pub struct Options {
    input: Input,
    /// update-direct's newer archive.
    next: Option<PathBuf>,
    out: PathBuf,
    case: String,
    output_size: OutputSize,
}

impl Options {
    /// The whole journey's limit, setup and cleanup included.
    fn run_limit(&self) -> Duration {
        Duration::from_secs(if self.case == UPDATE_DIRECT { 900 } else { 180 })
    }
}

pub fn options(args: &[String]) -> Result<Options> {
    if matches!(args, [help] if matches!(help.as_str(), "--help" | "-h")) {
        println!("{HELP}");
        return Err(String::new());
    }
    let mut values = BTreeMap::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--eon" | "--bundle" | "--next" | "--out" | "--case" | "--output-size"
        ) {
            return Err(format!("unknown argument {flag}\n{HELP}"));
        }
        let value = args
            .next()
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or(format!("{flag} needs a value"))?;
        if values.insert(flag.as_str(), value.as_str()).is_some() {
            return Err(format!("{flag} is repeated"));
        }
    }
    let case = *values.get("--case").ok_or("--case is required")?;
    if !matches!(case, FIRST_LAUNCH | UPDATE_DIRECT | TUTOR) {
        return Err(format!("unknown journey case {case}\n{HELP}"));
    }
    let input = match (values.get("--eon"), values.get("--bundle")) {
        (Some(eon), None) => Input::Nix(eon.into()),
        (None, Some(archive)) => Input::Bundle(archive.into()),
        _ => return Err(format!("give exactly one of --eon and --bundle\n{HELP}")),
    };
    let next = values.get("--next").map(PathBuf::from);
    match (case, &input, &next) {
        (UPDATE_DIRECT, Input::Bundle(_), Some(_))
        | (FIRST_LAUNCH, _, None)
        | (TUTOR, Input::Nix(_), None) => {}
        (TUTOR, Input::Bundle(_), _) => return Err("tutor needs --eon".into()),
        (UPDATE_DIRECT, ..) => {
            return Err(format!("update-direct needs --bundle and --next\n{HELP}"));
        }
        _ => return Err(format!("--next belongs to update-direct\n{HELP}")),
    }
    let output_size = match values.get("--output-size") {
        Some(size) if case == TUTOR => OutputSize::parse(size)?,
        Some(_) => return Err("--output-size belongs to tutor".into()),
        None => OutputSize::DEFAULT,
    };
    Ok(Options {
        case: case.into(),
        input,
        next,
        out: values.get("--out").ok_or("--out is required")?.into(),
        output_size,
    })
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn text(path: &Path) -> Value {
    path.display().to_string().into()
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn product_root(requested: &Path, executable: &Path) -> Result<PathBuf> {
    // Resolve the product link without following its bin symlink into the composition.
    let path = if requested.ends_with("bin/eon") {
        requested
    } else {
        executable
    };
    let root = path
        .parent()
        .and_then(Path::parent)
        .ok_or("candidate has no product root")?;
    let root = fs::canonicalize(if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    })
    .map_err(error)?;
    if !root.starts_with("/nix/store") {
        return Err("candidate product root must resolve to the Nix store".into());
    }
    Ok(root)
}

pub fn run(options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("journey requires Linux/Wayland".into());
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.out)
        .map_err(error)?;
    let out = fs::canonicalize(&options.out).map_err(error)?;
    let mut eon = options.input.path().to_path_buf();
    let mut root = None;
    let started = Instant::now();
    let result = (|| {
        let exe = std::env::current_exe().map_err(error)?;
        let shell = std::env::var_os("EON_BENCH_SH")
            .ok_or("missing EON_BENCH_SH (use the Nix bench package)")?;
        let shell = launcher::candidate(Path::new(&shell))?;
        let (binds, setenv, inner) = match &options.input {
            Input::Nix(requested) => {
                eon = launcher::candidate(requested)?;
                let product = product_root(requested, &eon)?;
                root = Some(product.clone());
                (
                    Vec::new(),
                    (PRODUCT_ROOT, product.into_os_string()),
                    ["--eon".into(), eon.clone().into_os_string()],
                )
            }
            Input::Bundle(archive) => {
                eon = input_archive("--bundle", archive)?;
                let tools = if options.case == UPDATE_DIRECT {
                    update_direct::HOST_TOOLS
                } else {
                    &[]
                };
                let host = HostSet::discover(tools)?;
                let mut binds = host.outer_binds();
                binds.push((eon.clone(), ARCHIVE.into()));
                (
                    binds,
                    (HOST_SET, host.to_json().to_pretty().into()),
                    ["--bundle".into(), ARCHIVE.into()],
                )
            }
        };
        let mut binds = binds;
        let mut inner = Vec::from(inner);
        let mut next_source = None;
        if let Some(next) = &options.next {
            let next = input_archive("--next", next)?;
            binds.push((next.clone(), NEXT.into()));
            inner.extend(["--next".into(), NEXT.into()]);
            next_source = Some(next);
        }
        let log = fs::File::create(out.join("sandbox.log")).map_err(error)?;
        let mut command = isolation::journey(&out, &exe, &shell, &binds)?;
        command
            .env_clear()
            .envs(
                [
                    "PATH",
                    "EON_BENCH_LAVAPIPE",
                    "EON_BENCH_MAGICK",
                    "EON_BENCH_BWRAP",
                    "EON_BENCH_TAR",
                    "EON_BENCH_ZSTD",
                    "EON_BENCH_FAKE_CURL",
                ]
                .into_iter()
                .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .stdout(log.try_clone().map_err(error)?)
            .stderr(log)
            .args(["--setenv", PARENT_PID_NS])
            .arg(fs::read_link("/proc/self/ns/pid").map_err(error)?)
            .args(["--setenv", setenv.0])
            .arg(setenv.1);
        if let Some(next) = next_source {
            command
                .args(["--setenv", NEXT_SOURCE])
                .arg(next.into_os_string());
        }
        command
            .arg("--")
            .arg(exe)
            .args(["journey-inner", "--case", &options.case])
            .args(inner)
            .arg("--out")
            .arg(isolation::JOURNEY_EVIDENCE);
        if options.case == TUTOR {
            command.args(["--output-size", &options.output_size.to_string()]);
        }
        let ran = isolation::run_until(&mut command, Some(started + options.run_limit()))
            .map_err(|reason| format!("{reason}; see sandbox.log"));
        // The sandbox's PID namespace is gone, so nothing runs from an
        // extraction the inner run kept or never reached removing, such as
        // after the run limit killed it.
        reclaim(&out, ran)
    })();
    if let Err(reason) = &result
        && !out.join("result.json").exists()
    {
        let mut receipt = Receipt::new(options, &eon, root.as_deref(), started);
        let _ = receipt.record("sandbox", "setup", || Err(reason.clone()));
        receipt.write(&out)?;
    }
    suite::interrupt::resend();
    result
}

fn require_isolation() -> Result {
    let parent = std::env::var_os(PARENT_PID_NS).ok_or("journey-inner requires private bwrap")?;
    if fs::read_link("/proc/self/ns/pid")
        .map_err(error)?
        .as_os_str()
        == parent
        || fs::read_to_string("/proc/1/comm").map_err(error)?.trim() != "bwrap"
        || fs::read_dir(environment::user_runtime_parent())
            .map_err(error)?
            .next()
            .is_some()
    {
        return Err("journey-inner requires a new bwrap PID namespace and runtime parent".into());
    }
    Ok(())
}

struct Receipt<'a> {
    case: &'a str,
    mode: &'static str,
    eon: Value,
    product_root: Value,
    source_receipt: Value,
    bench: Value,
    started: Instant,
    steps: Vec<Value>,
    failed: bool,
}

impl<'a> Receipt<'a> {
    fn new(options: &'a Options, eon: &Path, root: Option<&Path>, started: Instant) -> Self {
        let bench = match environment::build() {
            Ok(build) => build.to_json(),
            Err(error) => object([("error", error.into())]),
        };
        Self {
            case: &options.case,
            mode: options.input.mode(),
            eon: text(eon),
            product_root: root.map(text).unwrap_or(Value::Null),
            source_receipt: Value::Null,
            bench,
            started,
            steps: Vec::new(),
            failed: false,
        }
    }

    fn record(&mut self, name: &str, kind: &str, action: impl FnOnce() -> Result<Value>) -> Result {
        let started = Instant::now();
        let result = action();
        self.failed |= result.is_err();
        let (status, data) = match &result {
            Ok(value) => ("pass", ("evidence", value.clone())),
            Err(error) => ("fail", ("error", error.as_str().into())),
        };
        self.steps.push(object([
            ("name", name.into()),
            ("kind", kind.into()),
            ("result", status.into()),
            ("seconds", started.elapsed().as_secs_f64().into()),
            data,
        ]));
        result.map(drop)
    }

    /// Records a step whose action also yields a value for later steps.
    fn produce<T>(
        &mut self,
        name: &str,
        kind: &str,
        action: impl FnOnce() -> Result<(T, Value)>,
    ) -> Result<T> {
        let mut produced = None;
        self.record(name, kind, || {
            let (value, evidence) = action()?;
            produced = Some(value);
            Ok(evidence)
        })?;
        Ok(produced.expect("a passing step produced its value"))
    }

    /// Removes `root`, a large tree its archive reproduces, once process
    /// cleanup succeeded and no process runs from or maps it; otherwise the
    /// row fails and keeps it for the outer run's `reclaim`.
    fn remove_root(&mut self, name: &str, root: &Path, cleaned: bool) -> Result {
        self.record(name, "cleanup", || {
            let occupants = bundle::occupied(root)?;
            if !cleaned || !occupants.is_empty() {
                return Err(format!(
                    "kept {} for the outer run: process cleanup {}, occupants {occupants:?}",
                    root.display(),
                    if cleaned { "succeeded" } else { "failed" }
                ));
            }
            bundle::remove(root)
        })
    }

    /// Writes the receipt; a run whose steps passed still fails when a
    /// cleanup row did.
    fn finish(self, out: &Path, result: Result) -> Result {
        let failed = self.failed;
        self.write(out)?;
        if failed && result.is_ok() {
            return Err("private cleanup failed; see result.json".into());
        }
        result
    }

    fn write(self, out: &Path) -> Result {
        let value = object([
            ("schema", 1u64.into()),
            ("suite", "journey".into()),
            ("case", self.case.into()),
            ("mode", self.mode.into()),
            ("eon", self.eon),
            ("product_root", self.product_root),
            ("source_receipt", self.source_receipt),
            ("bench", self.bench),
            ("result", if self.failed { "fail" } else { "pass" }.into()),
            ("seconds", self.started.elapsed().as_secs_f64().into()),
            ("steps", Value::Array(self.steps)),
        ]);
        child::publish(&out.join("result.json"), &value.to_pretty())
    }
}

/// The last `lines` lines of a log, for a failed step's evidence.
fn log_tail(path: &Path, lines: usize) -> String {
    match fs::read_to_string(path) {
        Ok(text) => {
            let all: Vec<_> = text.lines().collect();
            let tail = all[all.len().saturating_sub(lines)..].join("\n");
            format!("{} ends:\n{tail}", path.display())
        }
        Err(error) => format!("{}: {error}", path.display()),
    }
}

/// Removes the extractions, prefix and scratch archive the inner run left
/// behind, once its sandbox is gone, and adds any removal failure to the run's result.
fn reclaim(out: &Path, ran: Result) -> Result {
    [
        BUNDLE_ROOT,
        update_direct::NEXT_ROOT,
        update_direct::PREFIX_ROOT,
        update_direct::TAMPERED,
    ]
    .iter()
    .map(|name| out.join(name))
    .filter(|leftover| fs::symlink_metadata(leftover).is_ok())
    .fold(ran, |ran, leftover| {
        match (ran, bundle::remove(&leftover)) {
            (ran, Ok(_)) => ran,
            (Ok(()), Err(removal)) => Err(removal),
            (Err(reason), Err(removal)) => Err(format!("{reason}; {removal}")),
        }
    })
}

/// A journey input archive, resolved for its read-only bind.
fn input_archive(flag: &str, archive: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(archive)
        .map_err(|reason| format!("{flag} {}: {reason}", archive.display()))?;
    if !path.is_file() {
        return Err(format!("{flag} {} is not a file", path.display()));
    }
    Ok(path)
}

fn source_receipt(root: &Path) -> Result<Value> {
    match fs::read_to_string(root.join("share/eon-source-receipt.json")) {
        Ok(receipt) => {
            let receipt = Value::parse(&receipt)?;
            if receipt.get("build").and_then(Value::as_str).is_none() {
                return Err("product source receipt has no build".into());
            }
            Ok(receipt)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error.to_string()),
    }
}

struct Journey {
    launch: Headless,
    /// The installed product root: the Nix package, or the bundle's package.
    product_root: PathBuf,
    /// The extracted bundle tree, whose processes the case audits.
    tree: Option<PathBuf>,
    generation: Option<String>,
    tab: Option<String>,
    popup: Option<String>,
}

type Predicate = fn(&mut Journey, &Value, Instant) -> Result<Probe>;
type ImageCheck = fn(&Picture) -> Result<(bool, Value)>;

struct Step(&'static str, Action);
enum Action {
    Cli(Cli),
    Send(fn(&Journey) -> Option<&str>, Option<&'static str>),
    Wait(Duration, Predicate),
    Capture(ImageCheck),
}

enum Cli {
    Call(&'static [&'static str]),
    Launch(&'static [&'static str]),
}

impl Cli {
    fn run(&self, journey: &mut Journey) -> Result<Value> {
        let (args, stdout) = match self {
            Self::Call(args) => {
                let output =
                    journey.call(&journey.launch.eon, args, Instant::now() + CALL_LIMIT)?;
                let text = String::from_utf8(output).map_err(error)?;
                (*args, Some(text.trim().to_owned()))
            }
            Self::Launch(args) => {
                journey.launch.start_eon_with(args)?;
                (*args, None)
            }
        };
        Ok(object([
            (
                "args",
                Value::Array(args.iter().map(|&arg| arg.into()).collect()),
            ),
            ("stdout", stdout.into()),
        ]))
    }
}

enum Probe {
    Ready(Value),
    Pending(Value),
}

impl Step {
    fn run(&self, journey: &mut Journey, receipt: &mut Receipt<'_>) -> Result {
        let Step(name, action) = self;
        let kind = match action {
            Action::Cli(_) => "cli",
            Action::Send(..) => "send",
            Action::Wait(_, _) => "wait",
            Action::Capture(_) => "capture",
        };
        receipt.record(name, kind, || match action {
            Action::Cli(command) => command.run(journey),
            Action::Send(target, text) => journey.send(
                target(journey).ok_or("send target has not been observed")?,
                *text,
                text.is_none(),
            ),
            Action::Wait(limit, predicate) => journey.wait(*limit, *predicate),
            Action::Capture(check) => journey.capture(name, *check),
        })
    }
}

fn verify_build(versions: &str, expected: Option<&str>) -> Result {
    let identity = versions.lines().next().unwrap_or_default();
    let mut fields = identity.split_whitespace();
    let build = if fields.next() == Some("eon") {
        fields.nth(1)
    } else {
        None
    };
    if build.is_none() || expected.is_some_and(|expected| build != Some(expected)) {
        return Err(format!(
            "eon versions disagrees with candidate build {expected:?}: {identity}"
        ));
    }
    Ok(())
}

pub fn run_inner(options: &Options) -> Result {
    require_isolation()?;
    if options.case == UPDATE_DIRECT {
        return update_direct::run(options);
    }
    let mut receipt = Receipt::new(options, options.input.path(), None, Instant::now());
    let mut extraction = None;
    let mut extracted = None;
    let mut cleaned = true;
    let result = (|| {
        if receipt.bench.get("error").is_some() {
            return Err(format!("could not identify bench: {:?}", receipt.bench));
        }
        let (launch, product_root) = match &options.input {
            Input::Nix(eon) => {
                let root = PathBuf::from(
                    std::env::var_os(PRODUCT_ROOT).ok_or("missing candidate product root")?,
                );
                receipt.product_root = text(&root);
                let mut launch = Headless::with_size(
                    &options.out,
                    eon,
                    WindowMode::Fullscreen,
                    options.output_size,
                )?;
                receipt.source_receipt = source_receipt(&root)?;
                Renderer("lavapipe".into()).apply(&mut launch)?;
                launch.start_compositor(BusPolicy::Private)?;
                (launch, root)
            }
            Input::Bundle(archive) => {
                let host = std::env::var(HOST_SET).map_err(|_| "missing bundle host set")?;
                let host = HostSet::from_json(&Value::parse(&host)?)?;
                // Owned from creation: removed at the end however setup ends.
                let root = extraction.insert(options.out.join(BUNDLE_ROOT));
                let opened = receipt.produce("bundle", "setup", || {
                    let tool = |key: &str| {
                        std::env::var_os(key)
                            .ok_or(format!("missing {key} (use the Nix bench package)"))
                    };
                    let opened = Bundle::extract(
                        archive,
                        root,
                        (&tool("EON_BENCH_TAR")?, &tool("EON_BENCH_ZSTD")?),
                        Instant::now() + EXTRACT_LIMIT,
                    )?;
                    let evidence = opened.to_json();
                    Ok((opened, evidence))
                })?;
                let opened = extracted.insert(opened);
                receipt.eon = text(&opened.entry);
                receipt.product_root = text(&opened.product);
                receipt.source_receipt = opened.receipt.clone();
                let launch = receipt.produce("host", "setup", || {
                    let mut launch = Headless::bundle(
                        &options.out,
                        &opened.entry,
                        WindowMode::Fullscreen,
                        host.clone(),
                    )?;
                    launch.start_compositor(BusPolicy::Private)?;
                    let ipc = launch.env["SWAYSOCK"].clone();
                    let wayland = Path::new(&launch.env["XDG_RUNTIME_DIR"])
                        .join(&launch.env["WAYLAND_DISPLAY"]);
                    let probe = launch.candidate_output_until(
                        Instant::now() + CALL_LIMIT,
                        Path::new("/bin/sh"),
                        &[
                            "-c",
                            bundle::PROBE,
                            "sh",
                            bundle::LOADER,
                            bundle::ICD,
                            &ipc,
                            path_text(&wayland)?,
                        ],
                    )?;
                    let evidence = bundle::host_probe(
                        &String::from_utf8_lossy(&probe.stdout),
                        &opened.glibc_floor,
                        &bundle::Expect {
                            tools: &[],
                            display: true,
                        },
                    )
                    .map_err(|reason| format!("{reason}; probe: {}", describe(&probe)))?;
                    Ok((
                        launch,
                        object([("jail", evidence), ("host_set", host.to_json())]),
                    ))
                })?;
                (launch, opened.product.clone())
            }
        };
        let expected_build = receipt
            .source_receipt
            .get("build")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut journey = Journey {
            launch,
            product_root,
            tree: extracted
                .as_ref()
                .map(|opened: &Bundle| opened.tree.clone()),
            generation: None,
            tab: None,
            popup: None,
        };
        let baseline = launcher::namespace_processes()?;
        let result = receipt
            .record("versions", "cli", || {
                let versions = Cli::Call(&["versions"]).run(&mut journey)?;
                let text = versions
                    .get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                verify_build(text, expected_build.as_deref())?;
                Ok(versions)
            })
            .and_then(|()| {
                if options.case == TUTOR {
                    return tutor::run(&mut journey, &mut receipt);
                }
                receipt.record("seed-home", "setup", || first_launch::prepare(&journey))?;
                first_launch::STEPS.iter().try_for_each(|step| {
                    step.run(&mut journey, &mut receipt)?;
                    let audit = first_launch::BUNDLE_AUDITS
                        .iter()
                        .find(|(after, _)| *after == step.0);
                    match (&journey.tree, audit) {
                        (Some(tree), Some((after, required))) => {
                            receipt.record(&format!("origins-{after}"), "check", || {
                                journey.launch.audit_origins(tree, required)
                            })
                        }
                        _ => Ok(()),
                    }
                })
            })
            .and_then(|()| match &journey.tree {
                Some(_) => receipt.record("doctor", "cli", || doctor(&journey)),
                None => Ok(()),
            });
        let stopped = receipt.record("stop", "cleanup", || journey.stop(&baseline));
        cleaned = journey.launch.processes.cleanup();
        if !cleaned {
            let _ = receipt.record("cleanup", "cleanup", || {
                Err("private process cleanup failed".into())
            });
        }
        result.and(stopped)
    })();
    if let Err(reason) = &result
        && !receipt.failed
    {
        let _ = receipt.record("setup", "setup", || Err(reason.clone()));
    }
    // The extraction is large and the archive reproduces it; keep it only
    // while a process may still run from it. Setup errors drop the launch
    // early, so its cleanup result is unknown here: check for such processes
    // directly. The outer run removes a kept extraction once the journey
    // sandbox is gone.
    if let Some(root) = &extraction {
        let _ = receipt.remove_root("remove-bundle", root, cleaned);
    }
    receipt.finish(&options.out, result)
}

/// `eon doctor --json` on the healthy bundle (eon-6zoxr): no FAIL by check
/// ID, and the packaged editor passes.
fn doctor(journey: &Journey) -> Result<Value> {
    let output = journey.output(
        &journey.launch.eon,
        &["doctor", "--json"],
        Instant::now() + CALL_LIMIT,
    )?;
    let report = std::str::from_utf8(&output.stdout)
        .map_err(error)
        .and_then(Value::parse)
        .map_err(|reason| format!("eon doctor --json: {reason}; {}", describe(&output)))?;
    judge_doctor(report, output.status.code())
}

fn judge_doctor(report: Value, exit: Option<i32>) -> Result<Value> {
    let checks = report
        .get("checks")
        .and_then(Value::as_array)
        .ok_or("eon doctor --json has no checks list")?;
    fn field<'a>(check: &'a Value, key: &str) -> Option<&'a str> {
        check.get(key).and_then(Value::as_str)
    }
    let failed: Vec<_> = checks
        .iter()
        .filter(|check| field(check, "status") == Some("FAIL"))
        .filter_map(|check| field(check, "id"))
        .collect();
    let editor = checks
        .iter()
        .find(|check| field(check, "id") == Some("editor"))
        .and_then(|check| field(check, "status"));
    if !failed.is_empty() || exit != Some(0) || editor != Some("PASS") {
        return Err(format!(
            "eon doctor failed {failed:?} with editor {editor:?} (exit {exit:?}): {}",
            report.to_pretty().trim()
        ));
    }
    Ok(object([("exit", 0u64.into()), ("report", report)]))
}

fn describe(output: &Output) -> String {
    format!(
        "{}: {} {}",
        output.status,
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    )
}

fn startup_pending(generation: Option<&str>, reply: &Value) -> bool {
    generation.is_none()
        && matches!(
            reply
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("missing-supervisor" | "supervisor-unavailable" | "registry-busy")
        )
}

/// The live candidate processes, as `PID@START`, that `baseline` (taken
/// before the case's first candidate call) does not hold.
fn survivors(baseline: &BTreeMap<u32, String>) -> Result<Vec<String>> {
    Ok(launcher::namespace_processes()?
        .into_iter()
        .filter(|(pid, start)| baseline.get(pid) != Some(start))
        .map(|(pid, start)| format!("{pid}@{start}"))
        .collect())
}

fn succeeded(program: &Path, args: &[&str], output: Output) -> Result<Vec<u8>> {
    if !output.status.success() {
        return Err(format!(
            "{} {args:?}: {}",
            program.display(),
            describe(&output)
        ));
    }
    Ok(output.stdout)
}

impl Journey {
    /// Runs a candidate program through the candidate's route.
    fn output(&self, program: &Path, args: &[&str], deadline: Instant) -> Result<Output> {
        self.launch
            .candidate_output_until(deadline.min(Instant::now() + CALL_LIMIT), program, args)
    }

    fn call(&self, program: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>> {
        succeeded(program, args, self.output(program, args, deadline)?)
    }

    /// Runs a bench tool, never through the candidate's route.
    fn tool(&self, program: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>> {
        let output =
            self.launch
                .output_until(deadline.min(Instant::now() + CALL_LIMIT), program, args)?;
        succeeded(program, args, output)
    }

    fn call_json(&self, args: &[&str], deadline: Instant) -> Result<Value> {
        let output = self.call(&self.launch.eon, args, deadline)?;
        Value::parse(std::str::from_utf8(&output).map_err(error)?)
            .map_err(|error| format!("eon {args:?}: {error}"))
    }

    fn send(&self, target: &str, text: Option<&str>, enter: bool) -> Result<Value> {
        let mut args = vec!["pane", "send", target, "--json"];
        if enter {
            args.push("--enter");
        }
        if let Some(text) = text {
            args.extend(["--", text]);
        }
        let reply = self.call_json(&args, Instant::now() + CALL_LIMIT)?;
        if reply.get("enter").and_then(Value::as_bool) != Some(enter)
            || reply.get("bytes").and_then(Value::as_u64) != Some(text.map_or(0, str::len) as u64)
        {
            return Err(format!(
                "eon pane send {target} returned unexpected evidence: {reply:?}"
            ));
        }
        Ok(reply)
    }

    fn wait(
        &mut self,
        limit: Duration,
        mut predicate: impl FnMut(&mut Self, &Value, Instant) -> Result<Probe>,
    ) -> Result<Value> {
        let deadline = Instant::now() + limit;
        let mut last = Value::Null;
        while Instant::now() < deadline {
            let output = match self.output(&self.launch.eon, &["session", "--json"], deadline) {
                Err(_) if Instant::now() >= deadline => break,
                result => result?,
            };
            if output.status.code() == Some(124) && Instant::now() >= deadline {
                break;
            }
            let json = Value::parse(std::str::from_utf8(&output.stdout).map_err(error)?);
            if !output.status.success() {
                if json
                    .as_ref()
                    .is_ok_and(|reply| startup_pending(self.generation.as_deref(), reply))
                {
                    last = json?;
                    continue;
                }
                return Err(format!("eon session --json: {}", describe(&output)));
            }
            let snapshot = json.map_err(|error| format!("eon session --json: {error}"))?;
            if snapshot.get("tabs").and_then(Value::as_array).is_none() {
                return Err(format!(
                    "eon session --json returned no tabs array: {snapshot:?}"
                ));
            }
            if let Some(generation) = snapshot
                .get("running")
                .and_then(|running| running.get("this"))
                .and_then(|window| window.get("generation"))
                .and_then(Value::as_str)
            {
                self.generation.get_or_insert_with(|| generation.to_owned());
            }
            match predicate(self, &snapshot, deadline) {
                Ok(Probe::Ready(observation)) => return Ok(observation),
                Ok(Probe::Pending(observation)) => last = observation,
                Err(_) if Instant::now() >= deadline => break,
                Err(reason) => return Err(reason),
            }
        }
        Err(format!(
            "predicate did not match within {}s; last observation: {}; {}",
            limit.as_secs(),
            last.to_pretty().trim(),
            log_tail(&self.launch.out.join("eon.log"), LOG_TAIL)
        ))
    }

    fn capture(&self, name: &str, check: ImageCheck) -> Result<Value> {
        let path = self.launch.out.join(format!("capture-{name}.png"));
        let path_arg = path_text(&path)?;
        let raster = path.with_extension("ppm");
        let raster_arg = path_text(&raster)?;
        let magick = PathBuf::from(
            std::env::var_os("EON_BENCH_MAGICK")
                .ok_or("missing EON_BENCH_MAGICK (use the Nix bench package)")?,
        );
        let deadline = Instant::now() + CAPTURE_LIMIT;
        let frame = || {
            self.tool(Path::new("grim"), &["-t", "png", path_arg], deadline)?;
            // Large outputs exceed the command runner's stdout limit as raw RGB.
            self.tool(
                &magick,
                &[
                    path_arg, "-strip", "-alpha", "off", "-depth", "8", raster_arg,
                ],
                deadline,
            )?;
            let picture = Picture::read(&raster);
            fs::remove_file(&raster).map_err(error)?;
            picture
        };
        let mut last = Value::Null;
        while Instant::now() < deadline {
            let picture = match frame() {
                Err(_) if last != Value::Null && Instant::now() >= deadline => break,
                result => result?,
            };
            let size = self.launch.output_size;
            if (picture.width as u64, picture.height as u64) != (size.width, size.height) {
                return Err(format!(
                    "capture is {}x{}, expected {size}",
                    picture.width, picture.height
                ));
            }
            let (passed, evidence) = check(&picture)?;
            if passed {
                return Ok(object([
                    ("file", format!("capture-{name}.png").into()),
                    ("width", size.width.into()),
                    ("height", size.height.into()),
                    ("pixels", evidence),
                ]));
            }
            last = evidence;
        }
        Err(format!(
            "{} did not show the expected pixels: {}",
            path.display(),
            last.to_pretty().trim()
        ))
    }

    fn stop(&mut self, baseline: &BTreeMap<u32, String>) -> Result<Value> {
        let reply = self.launch.stop_eon_report()?;
        let reply = reply.as_deref().map(Value::parse).transpose()?;
        if let Some(generation) = &self.generation {
            let stopped = reply
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|windows| {
                    windows
                        .iter()
                        .filter_map(|window| window.get("results")?.as_array())
                        .flatten()
                        .any(|result| {
                            result
                                .get("stopped")
                                .and_then(|stopped| stopped.get("generation"))
                                .and_then(Value::as_str)
                                == Some(generation)
                        })
                });
            if !stopped {
                return Err(format!(
                    "Stop did not acknowledge generation {generation}: {reply:?}"
                ));
            }
        }
        let survivors = survivors(baseline)?;
        if !survivors.is_empty() {
            return Err(format!(
                "live candidate processes after Stop: {}; Stop reply: {reply:?}",
                survivors.join(", ")
            ));
        }
        Ok(object([
            ("reply", reply.unwrap_or(Value::Null)),
            ("live_candidate_processes", 0u64.into()),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_match_the_candidate_receipt_before_case_steps() {
        assert!(verify_build("eon 1.0 g1-tested\neonw 47\n", Some("g1-tested")).is_ok());
        assert!(verify_build("eon 1.0 g1-other", Some("g1-tested")).is_err());
        assert!(verify_build("missing version identity", None).is_err());
    }

    #[test]
    fn doctor_needs_no_failure_and_a_passing_editor() {
        let report = |editor: &str, gpu: &str| {
            Value::parse(&format!(
                r#"{{"checks":[{{"id":"install","status":"PASS"}},{{"id":"editor","status":"{editor}"}},{{"id":"gpu","status":"{gpu}","fix":"later"}}]}}"#
            ))
            .unwrap()
        };
        assert!(judge_doctor(report("PASS", "WARN"), Some(0)).is_ok());
        assert!(judge_doctor(report("WARN", "WARN"), Some(0)).is_err());
        let failed = judge_doctor(report("PASS", "FAIL"), Some(1)).unwrap_err();
        assert!(failed.contains(r#"["gpu"]"#), "{failed}");
        assert!(judge_doctor(report("PASS", "PASS"), Some(1)).is_err());
        assert!(judge_doctor(Value::parse("{}").unwrap(), Some(0)).is_err());
    }

    #[test]
    fn first_inspection_waits_through_initial_registry_admission() {
        let busy = Value::parse(r#"{"error":{"code":"registry-busy"}}"#).unwrap();
        let invalid = Value::parse(r#"{"error":{"code":"malformed-action"}}"#).unwrap();
        assert!(startup_pending(None, &busy));
        assert!(!startup_pending(Some("observed-generation"), &busy));
        assert!(!startup_pending(None, &invalid));
    }

    #[test]
    fn options_are_explicit_and_unique() {
        for bad in [
            "--eon eon --out out",
            "--eon eon --out out --case other",
            "--eon eon --out out --case first-launch --out again",
            "--eon --out out --case first-launch",
            "--out out --case first-launch",
            "--eon eon --bundle a.tar.zst --out out --case first-launch",
            "--bundle a.tar.zst --bundle b.tar.zst --out out --case first-launch",
            "--bundle a.tar.zst --out out --case first-launch --extra x",
        ] {
            let args = bad
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert!(options(&args).is_err(), "{bad}");
        }
        let modes = ["--eon eon", "--bundle a.tar.zst"].map(|input| {
            let args: Vec<_> = format!("{input} --out out --case first-launch")
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            options(&args).unwrap().input.mode()
        });
        assert_eq!(modes, ["nix", "bundle"]);
    }

    #[test]
    fn tutor_parses_one_size_and_preserves_the_shared_default() {
        let parse = |line: &str| {
            options(
                &line
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        };
        let tutor = "--eon eon --out out --case tutor";
        assert_eq!(parse(tutor).unwrap().output_size, OutputSize::DEFAULT);
        let large = parse(&format!("{tutor} --output-size 1728x1117"))
            .unwrap()
            .output_size;
        assert_eq!(large.to_string(), "1728x1117");
        for suffix in [
            "",
            "0x800",
            "1280x",
            "1280X800",
            "1280x4097",
            "+1280x800",
            "1728x1117 --output-size 1280x800",
        ] {
            assert!(
                parse(&format!("{tutor} --output-size {suffix}")).is_err(),
                "{suffix}"
            );
        }
        assert!(parse("--eon eon --out out --case first-launch --output-size 1728x1117").is_err());
        assert!(
            parse("--bundle a --next b --out out --case update-direct --output-size 1728x1117")
                .is_err()
        );
        assert!(parse("--bundle a --out out --case tutor").is_err());
    }

    #[test]
    fn the_outer_run_reclaims_a_kept_read_only_extraction() {
        use std::os::unix::fs::PermissionsExt;
        let out = std::env::temp_dir().join(format!("bench-reclaim-{}", std::process::id()));
        let kept = out.join(BUNDLE_ROOT).join("eon-1/nix");
        fs::create_dir_all(&kept).unwrap();
        for dir in [&kept, &out.join(BUNDLE_ROOT).join("eon-1")] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        }
        assert_eq!(
            reclaim(&out, Err("run limit".into())),
            Err("run limit".into())
        );
        assert!(!out.join(BUNDLE_ROOT).exists());
        assert_eq!(reclaim(&out, Ok(())), Ok(()));
        fs::remove_dir(&out).unwrap();
    }

    #[test]
    fn failed_waits_carry_the_tail_of_eon_log() {
        let dir = std::env::temp_dir().join(format!("bench-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("eon.log");
        let text: String = (1..=30).map(|line| format!("l{line:02}\n")).collect();
        fs::write(&log, text).unwrap();
        let tail = log_tail(&log, LOG_TAIL);
        assert!(tail.contains("l11") && tail.contains("l30"), "{tail}");
        assert!(!tail.contains("l10"), "{tail}");
        fs::remove_file(&log).unwrap();
        assert!(log_tail(&log, LOG_TAIL).contains("eon.log"));
        fs::remove_dir(&dir).unwrap();
    }
}
