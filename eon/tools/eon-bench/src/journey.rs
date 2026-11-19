//! Installed Eon journeys on the shared private display.
mod first_launch;

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

use crate::{
    child, environment, isolation,
    json::Value,
    launcher::{self, BusPolicy, Headless, Renderer, WindowMode},
    picture::Picture,
    suite,
};

type Result<T = ()> = std::result::Result<T, String>;

const HELP: &str = "\
eon-bench journey --eon PATH --case first-launch --out DIR

Exercise the installed Nix Eon candidate in private 1280x800 headless Sway.
PATH is the candidate's bin/eon. DIR must not exist. Linux only.
Writes result.json, capture-*.png and private launch logs.
Exit 0 on a passing case and Stop, 1 on a run failure, 2 on invalid arguments.";

const PARENT_PID_NS: &str = "EON_BENCH_JOURNEY_PARENT_PID_NS";
const PRODUCT_ROOT: &str = "EON_BENCH_JOURNEY_PRODUCT_ROOT";
const RUN_LIMIT: Duration = Duration::from_secs(180);
const CALL_LIMIT: Duration = Duration::from_secs(10);
const CAPTURE_LIMIT: Duration = Duration::from_secs(10);

pub struct Options {
    eon: PathBuf,
    out: PathBuf,
    case: String,
}

pub fn options(args: &[String]) -> Result<Options> {
    if matches!(args, [help] if matches!(help.as_str(), "--help" | "-h")) {
        println!("{HELP}");
        return Err(String::new());
    }
    let mut values = BTreeMap::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--eon" | "--out" | "--case") {
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
    if case != "first-launch" {
        return Err(format!("unknown journey case {case}\n{HELP}"));
    }
    Ok(Options {
        case: case.into(),
        eon: values.get("--eon").ok_or("--eon is required")?.into(),
        out: values.get("--out").ok_or("--out is required")?.into(),
    })
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
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
    let mut eon = options.eon.clone();
    let mut root = None;
    let started = Instant::now();
    let result = (|| {
        eon = launcher::candidate(&options.eon)?;
        root = Some(product_root(&options.eon, &eon)?);
        let exe = std::env::current_exe().map_err(error)?;
        let shell = std::env::var_os("EON_BENCH_SH")
            .ok_or("missing EON_BENCH_SH (use the Nix bench package)")?;
        let shell = launcher::candidate(Path::new(&shell))?;
        let log = fs::File::create(out.join("sandbox.log")).map_err(error)?;
        let mut command = isolation::journey(&out, &exe, &shell)?;
        command
            .env_clear()
            .envs(
                ["PATH", "EON_BENCH_LAVAPIPE", "EON_BENCH_MAGICK"]
                    .into_iter()
                    .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .stdout(log.try_clone().map_err(error)?)
            .stderr(log)
            .args(["--setenv", PARENT_PID_NS])
            .arg(fs::read_link("/proc/self/ns/pid").map_err(error)?)
            .args(["--setenv", PRODUCT_ROOT])
            .arg(root.as_ref().expect("resolved product root"))
            .arg("--")
            .arg(exe)
            .args(["journey-inner", "--case", &options.case, "--eon"])
            .arg(&eon)
            .arg("--out")
            .arg(isolation::JOURNEY_EVIDENCE);
        isolation::run_until(&mut command, Some(started + RUN_LIMIT))
            .map_err(|reason| format!("{reason}; see sandbox.log"))
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
            eon: eon.display().to_string().into(),
            product_root: root.map(|root| root.display().to_string()).into(),
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

    fn write(self, out: &Path) -> Result {
        let value = object([
            ("schema", 1u64.into()),
            ("suite", "journey".into()),
            ("case", self.case.into()),
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
    let root =
        PathBuf::from(std::env::var_os(PRODUCT_ROOT).ok_or("missing candidate product root")?);
    let mut receipt = Receipt::new(options, &options.eon, Some(&root), Instant::now());
    let mut run = || {
        let mut launch = Headless::new(&options.out, &options.eon, WindowMode::Fullscreen)?;
        receipt.source_receipt = source_receipt(&root)?;
        let expected_build = receipt
            .source_receipt
            .get("build")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if receipt.bench.get("error").is_some() {
            return Err(format!("could not identify bench: {:?}", receipt.bench));
        }
        Renderer("lavapipe".into()).apply(&mut launch)?;
        launch.start_compositor(BusPolicy::Private)?;
        let mut journey = Journey {
            launch,
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
            .and_then(|()| receipt.record("seed-home", "setup", || first_launch::prepare(&journey)))
            .and_then(|()| {
                first_launch::STEPS
                    .iter()
                    .try_for_each(|step| step.run(&mut journey, &mut receipt))
            });
        let stopped = receipt.record("stop", "cleanup", || journey.stop(&baseline));
        let cleaned = journey.launch.processes.cleanup();
        if !cleaned {
            let _ = receipt.record("cleanup", "cleanup", || {
                Err("private process cleanup failed".into())
            });
        }
        result.and(stopped)
    };
    let result = run();
    if let Err(reason) = &result
        && !receipt.failed
    {
        let _ = receipt.record("setup", "setup", || Err(reason.clone()));
    }
    let failed = receipt.failed;
    receipt.write(&options.out)?;
    if failed && result.is_ok() {
        return Err("private cleanup failed; see result.json".into());
    }
    result
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

impl Journey {
    fn output(&self, program: &Path, args: &[&str], deadline: Instant) -> Result<Output> {
        self.launch
            .output_until(deadline.min(Instant::now() + CALL_LIMIT), program, args)
    }

    fn call(&self, program: &Path, args: &[&str], deadline: Instant) -> Result<Vec<u8>> {
        let output = self.output(program, args, deadline)?;
        if !output.status.success() {
            return Err(format!(
                "{} {args:?}: {}",
                program.display(),
                describe(&output)
            ));
        }
        Ok(output.stdout)
    }

    fn call_json(&self, args: &[&str], deadline: Instant) -> Result<Value> {
        let output = self.call(&self.launch.eon, args, deadline)?;
        Value::parse(std::str::from_utf8(&output).map_err(error)?)
            .map_err(|error| format!("eon {args:?}: {error}"))
    }

    fn send(&self, target: &str, text: Option<&str>) -> Result<Value> {
        let mut args = vec!["pane", "send", target, "--json"];
        if let Some(text) = text {
            args.extend(["--", text]);
        } else {
            args.push("--enter");
        }
        let reply = self.call_json(&args, Instant::now() + CALL_LIMIT)?;
        if reply.get("enter").and_then(Value::as_bool) != Some(text.is_none())
            || reply.get("bytes").and_then(Value::as_u64) != Some(text.map_or(0, str::len) as u64)
        {
            return Err(format!(
                "eon pane send {target} returned unexpected evidence: {reply:?}"
            ));
        }
        Ok(reply)
    }

    fn wait(&mut self, limit: Duration, predicate: Predicate) -> Result<Value> {
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
            "predicate did not match within {}s; last observation: {}",
            limit.as_secs(),
            last.to_pretty().trim()
        ))
    }

    fn capture(&self, name: &str, check: ImageCheck) -> Result<Value> {
        let path = self.launch.out.join(format!("capture-{name}.png"));
        let path_arg = path_text(&path)?;
        let magick = PathBuf::from(
            std::env::var_os("EON_BENCH_MAGICK")
                .ok_or("missing EON_BENCH_MAGICK (use the Nix bench package)")?,
        );
        let deadline = Instant::now() + CAPTURE_LIMIT;
        let frame = || {
            self.call(Path::new("grim"), &["-t", "png", path_arg], deadline)?;
            let pixels = self.call(
                &magick,
                &[path_arg, "-strip", "-alpha", "off", "-depth", "8", "ppm:-"],
                deadline,
            )?;
            Picture::parse(&pixels)
        };
        let mut last = Value::Null;
        while Instant::now() < deadline {
            let picture = match frame() {
                Err(_) if last != Value::Null && Instant::now() >= deadline => break,
                result => result?,
            };
            if (picture.width, picture.height) != (1280, 800) {
                return Err(format!(
                    "capture is {}x{}, expected 1280x800",
                    picture.width, picture.height
                ));
            }
            let (passed, evidence) = check(&picture)?;
            if passed {
                return Ok(object([
                    ("file", format!("capture-{name}.png").into()),
                    ("width", 1280u64.into()),
                    ("height", 800u64.into()),
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
        let current = launcher::namespace_processes()?;
        let survivors: Vec<_> = current
            .iter()
            .filter(|(pid, start)| baseline.get(pid) != Some(*start))
            .map(|(pid, start)| format!("{pid}@{start}"))
            .collect();
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
        ] {
            let args = bad
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert!(options(&args).is_err(), "{bad}");
        }
    }
}
