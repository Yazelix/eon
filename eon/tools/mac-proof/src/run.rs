//! `run`: the native steps on the Mac.

use crate::{
    retina,
    scenario::{self, Scenario},
    step::{Failure, Log, Outcome, Runner, check, execute, progress, succeed},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::{fs::DirBuilderExt, net::UnixListener},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);
const SYSTEM: &str = "aarch64-darwin";
const NATIVE: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));
/// Workspace packages that compile no tests on macOS: each is gated to Linux, and
/// its verified empty test result is not-applicable. Listing a package removes it
/// from the macOS proof. The tests in `macos_tests` keep this list equal to the
/// gated packages without macOS tests, so a new one fails on Linux until listed.
const LINUX_ONLY_TEST_PACKAGES: &[&str] = &["agent-jail", "eon-boat", "rota-cli"];
/// Names of the processes an Eon run or its tests start.
const EON_PROCESSES: &[&str] = &[
    "eon",
    "eonbase",
    ".eon-wrapped",
    ".eonbase-wrapped",
    "yazelix-orbit",
    "yazelix-orbit-worker",
    "yazelix-venus",
    ".yazelix-venus-wrapped",
    "venus-rune-candidate",
    ".venus-rune-candidate-wrapped",
];

pub struct Options {
    pub evidence: PathBuf,
    pub expect_tree: Option<String>,
    pub limit: Duration,
    pub dry_run: bool,
    /// Whether this run is on a person's own Mac.
    pub personal_mac: bool,
}

pub struct ScenarioOptions {
    pub package: PathBuf,
    pub evidence: PathBuf,
    pub limit: Duration,
    pub dry_run: bool,
}

pub struct Report {
    pub passed: bool,
    pub left_for_yu: usize,
    pub tree: String,
    pub package: Result<PathBuf, String>,
}

#[derive(Default)]
struct Context {
    source: PathBuf,
    evidence: PathBuf,
    expect_tree: Option<String>,
    tree: String,
    head: String,
    package: PathBuf,
    checks: BTreeMap<String, Option<String>>,
    ready_checks: BTreeSet<String>,
    packages: Vec<String>,
    private_dirs: BTreeSet<String>,
    processes: BTreeSet<(String, String)>,
    nix_program: Option<String>,
    nix_store_program: Option<String>,
    cargo_home: Option<PathBuf>,
}

struct CargoHome(PathBuf);

impl CargoHome {
    fn new() -> Result<Self, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mac-proof-cargo-{}-{stamp}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for CargoHome {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            progress(&format!(
                "mac-proof: remove cargo home {}: {error}",
                self.0.display()
            ));
        }
    }
}

impl Context {
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.source);
        if let Some(home) = &self.cargo_home {
            command.env("CARGO_HOME", home);
        }
        command
    }

    fn nix(&self, arguments: &[&str]) -> Command {
        let mut command = self.command(self.nix_program.as_deref().unwrap_or("nix"));
        command
            .args(["--extra-experimental-features", "nix-command flakes"])
            .arg("--accept-flake-config")
            .args(arguments);
        command
    }

    fn nix_store(&self, arguments: &[&str]) -> Command {
        let mut command = self.command(self.nix_store_program.as_deref().unwrap_or("nix-store"));
        command.args(arguments);
        command
    }

    fn git(&self, arguments: &[&str]) -> Command {
        let mut command = self.command("git");
        command.args(arguments);
        command
    }
}

/// Runs every step and reports the run's verdict and how many steps were left for Yu.
pub fn run(options: Options, source: &Path) -> Result<Report, String> {
    if !options.dry_run && !NATIVE {
        return Err("run is macOS only (Apple Silicon); use --dry-run elsewhere".to_owned());
    }
    if options.evidence.exists() {
        return Err(format!(
            "{} exists; give a fresh --out",
            options.evidence.display()
        ));
    }
    let source = fs::canonicalize(source).map_err(|error| format!("source: {error}"))?;
    // Evidence inside the source would change the tree that it records.
    let parent = match options.evidence.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent =
        fs::canonicalize(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    if parent.starts_with(&source) {
        return Err("give an --out outside the source".to_owned());
    }
    let mut runner = Runner::new(&options.evidence, options.dry_run, options.limit)
        .map_err(|error| format!("{}: {error}", options.evidence.display()))?;
    runner.identity.push((
        "system",
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
    ));
    let cargo_home = if options.personal_mac && !options.dry_run {
        Some(CargoHome::new()?)
    } else {
        None
    };
    let mut context = Context {
        source,
        evidence: options.evidence,
        expect_tree: options.expect_tree,
        cargo_home: cargo_home.as_ref().map(|home| home.0.clone()),
        ..Context::default()
    };
    steps(&mut runner, &mut context, options.personal_mac)
        .map_err(|error| format!("evidence: {error}"))?;
    let passed = runner
        .write(true)
        .map_err(|error| format!("evidence: {error}"))?;
    Ok(Report {
        passed,
        left_for_yu: runner.left_for_yu(),
        tree: context.tree,
        package: match runner.status("package") {
            Some("pass") => Ok(context.package),
            status => Err(format!("package step {}", status.unwrap_or("not run"))),
        },
    })
}

/// Uses the source step's owner to name a copied archive before choosing its bundle path.
pub fn source_tree(
    source_path: &Path,
    log: &mut Log,
    deadline: Instant,
) -> Result<String, Failure> {
    let mut context = Context {
        source: source_path.to_owned(),
        ..Context::default()
    };
    source(&mut context, log, deadline)?;
    Ok(context.tree)
}

/// Runs the scenario alone, on a package built before, and returns whether
/// it passed, allowing steps skipped for Yu.
pub fn scenario(options: ScenarioOptions) -> Result<bool, String> {
    if !options.dry_run && !NATIVE {
        return Err("scenario is macOS only (Apple Silicon); use --dry-run elsewhere".to_owned());
    }
    if options.evidence.exists() {
        return Err(format!(
            "{} exists; give a fresh --out",
            options.evidence.display()
        ));
    }
    // Its commands run from the scenario's own directory.
    let package = match options.dry_run {
        true => options.package,
        false => fs::canonicalize(&options.package)
            .map_err(|error| format!("--package {}: {error}", options.package.display()))?,
    };
    let mut runner = Runner::new(&options.evidence, options.dry_run, options.limit)
        .map_err(|error| format!("{}: {error}", options.evidence.display()))?;
    if !options.dry_run {
        runner
            .identity
            .push(("package", package.display().to_string()));
    }
    let mut scenario = Scenario::new(
        package,
        options.evidence.clone(),
        scenario::Environment::Private,
        false,
    );
    scenario::steps(&mut runner, &mut scenario, &[])
        .map_err(|error| format!("evidence: {error}"))?;
    runner
        .write(true)
        .map_err(|error| format!("evidence: {error}"))
}

fn steps(runner: &mut Runner, context: &mut Context, personal_mac: bool) -> std::io::Result<()> {
    runner.step(context, "host", 5 * MINUTE, &[], false, host)?;
    runner.step(context, "source", 10 * MINUTE, &[], true, source)?;
    runner.identity.push(("tree", context.tree.clone()));
    // An archive's commit is made on the Mac and names nothing; a checkout's is its HEAD.
    if !context.head.is_empty() {
        runner.identity.push(("head", context.head.clone()));
    }
    runner.write(false)?;
    runner.step(context, "package", 3 * HOUR, &["source"], false, package)?;
    runner.step(
        context,
        "terminfo",
        5 * MINUTE,
        &["package"],
        false,
        terminfo,
    )?;
    checks(runner, context)?;
    runner.step(
        context,
        "workspace",
        30 * MINUTE,
        &["source"],
        false,
        workspace,
    )?;
    test_packages(runner, context)?;
    // Dropped before leftovers, so that it has stopped whatever it started.
    let mut gui = Scenario::new(
        context.package.clone(),
        context.evidence.clone(),
        scenario::Environment::Private,
        personal_mac,
    );
    scenario::steps(runner, &mut gui, &["package"])?;
    let (gui_avail, keys_avail) = gui.gui_and_keys();
    drop(gui);
    appkit_events(runner, context, &gui_avail, &keys_avail)?;
    runner.step(
        context,
        "leftovers",
        5 * MINUTE,
        &["host"],
        false,
        leftovers,
    )?;
    Ok(())
}

fn wrapper_export(path: &Path, var: &str) -> Option<String> {
    let prefix = format!("export {var}=");
    fs::read_to_string(path).ok()?.lines().find_map(|line| {
        let raw = line.trim().strip_prefix(&prefix)?;
        let val = if let Some(s) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
            s.replace("'\\''", "'")
        } else {
            raw.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(raw)
                .to_owned()
        };
        (!val.is_empty()).then_some(val)
    })
}

fn resolve_appkit_env(
    ctx: &Context,
    log: &mut Log,
    deadline: Instant,
) -> Result<(PathBuf, Option<String>), Failure> {
    let eon_bin = ctx.package.join("bin/eon");
    let orbit_bin = match wrapper_export(&eon_bin, "EON_ORBIT").map(PathBuf::from) {
        Some(path) if path.is_file() => path,
        _ => {
            let pkg = ctx.package.to_string_lossy();
            let mut cmd = ctx.nix(&["path-info", "--recursive", &pkg]);
            let closure = text(succeed(log, &mut cmd, deadline, true)?);
            let bins: Vec<PathBuf> = closure
                .lines()
                .map(|p| Path::new(p).join("bin/yazelix-orbit"))
                .filter(|p| p.is_file())
                .collect();
            match bins.as_slice() {
                [bin] => bin.clone(),
                _ => {
                    return Err(Failure::Check(format!(
                        "want one bin/yazelix-orbit in Eon's closure, found {bins:?}"
                    )));
                }
            }
        }
    };
    let venus_bin = wrapper_export(&eon_bin, "EON_VENUS");
    let font_dirs = venus_bin
        .as_deref()
        .and_then(|v| wrapper_export(Path::new(v), "EON_VENUS_FONT_DIRS"));
    if font_dirs.is_none() {
        log.line(&format!(
            "mac-proof: no EON_VENUS_FONT_DIRS via {eon_bin:?} {venus_bin:?}"
        ));
    }
    Ok((orbit_bin, font_dirs))
}

fn failure_msg(e: Failure) -> String {
    match e {
        Failure::Timeout => "timed out".into(),
        Failure::Command(m)
        | Failure::Check(m)
        | Failure::Unavailable(m)
        | Failure::NotApplicable(m) => m,
    }
}

fn is_appkit_process(name: &str) -> bool {
    is_eon_process(name)
        || matches!(name, "venus-rune-candidate" | "mac-proof")
        || name.starts_with("mac_proof-")
}

/// Unix listener held open for one step; dropping it (or SIGKILL) signals EOF to detached peers.
struct RunLife {
    path: PathBuf,
    _listener: UnixListener,
}
impl RunLife {
    fn bind(path: PathBuf) -> Result<Self, String> {
        let _ = fs::remove_file(&path);
        let _listener =
            UnixListener::bind(&path).map_err(|e| format!("life {}: {e}", path.display()))?;
        Ok(Self { path, _listener })
    }
}
impl Drop for RunLife {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Signals detached payloads via `finish` + `RunLife` EOF (never signaling
/// `ps`-discovered PIDs), verifies zero Eon-owned survivors, and removes `/tmp/ev-*`.
fn cleanup_appkit_root(run_root: &Path, life: RunLife, log: &mut Log) -> Result<(), String> {
    let mut errs = Vec::new();
    for p in fs::read_dir(run_root).into_iter().flatten().flatten() {
        if p.path().is_dir() {
            let _ = fs::write(p.path().join("finish"), "");
        }
    }
    drop(life);
    let marker = format!("{}/", run_root.display());
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let lim = Instant::now() + Duration::from_secs(10);
        match scenario::listing(log, lim).map_err(failure_msg) {
            Ok(all) => {
                let left: Vec<_> = all
                    .into_iter()
                    .filter(|p| is_appkit_process(p.name()) && p.command.contains(&marker))
                    .collect();
                if left.is_empty() {
                    break;
                }
                if Instant::now() >= end {
                    errs.push(format!("survived: {left:?}"));
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                errs.push(format!("verify: {e}"));
                break;
            }
        }
    }
    if run_root
        .to_str()
        .is_some_and(|s| s.starts_with("/tmp/ev-") && !s[8..].contains('/'))
        && run_root.exists()
        && let Err(e) = fs::remove_dir_all(run_root)
    {
        errs.push(format!("remove {}: {e}", run_root.display()));
    }
    if !errs.is_empty() {
        let msg = errs.join("; ");
        log.line(&format!("mac-proof: appkit cleanup failed: {msg}"));
        return Err(msg);
    }
    Ok(())
}

fn run_appkit_step(
    ctx: &mut Context,
    log: &mut Log,
    deadline: Instant,
    (idx, group, fixtures, host, rune): (usize, &str, &str, &str, bool),
) -> Outcome {
    let fail = |m: &'static str| move |e: std::io::Error| Failure::Check(format!("{m}: {e}"));
    let (orbit_bin, font_dirs) = resolve_appkit_env(ctx, log, deadline)?;
    let out = ctx.evidence.join(format!("appkit-events/{group}-{host}"));
    let exe = std::env::current_exe().map_err(fail("exe"))?;
    let (exe, orbit) = (exe.to_string_lossy(), orbit_bin.to_string_lossy());
    let mut args: Vec<&str> = "develop --command cargo test --locked -p yazelix-venus"
        .split(' ')
        .collect();
    if rune {
        args.extend(["--features", "candidate-rune"]);
    }
    args.extend(["--test", "appkit_event_driver", "--", "--host", host]);
    args.extend(["--orbit", &orbit, "--mac-proof", &exe, "--only", fixtures]);
    args.extend(["--deadline", "180"]);
    let run_root = PathBuf::from(format!("/tmp/ev-{}-{idx}", std::process::id()));
    let _ = fs::remove_dir_all(&run_root);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&run_root)
        .map_err(fail("mkdir run_root"))?;
    let life = RunLife::bind(run_root.join("life.sock")).map_err(|e| {
        let _ = fs::remove_dir_all(&run_root);
        Failure::Check(e)
    })?;
    let mut cmd = ctx.nix(&args);
    cmd.env("EON_APPKIT_PROOF_OUT", &out)
        .env("EON_APPKIT_RUN_ROOT", &run_root);
    if let Some(dirs) = font_dirs {
        cmd.env("EON_VENUS_FONT_DIRS", dirs);
    }
    let run_res = succeed(log, &mut cmd, deadline, false);
    let clean_res = cleanup_appkit_root(&run_root, life, log);
    match (run_res, clean_res) {
        (Ok(_), Ok(())) => {}
        (Err(e), Ok(())) => return Err(e),
        (Ok(_), Err(c)) => return Err(Failure::Check(format!("appkit cleanup: {c}"))),
        (Err(e), Err(c)) => {
            return Err(Failure::Check(format!("{} (cleanup: {c})", failure_msg(e))));
        }
    }
    let bytes = fs::read(out.join("result.json")).map_err(fail("result.json"))?;
    let r: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| Failure::Check(format!("parse result.json: {e}")))?;
    check(r["result"] == "pass", || {
        format!("appkit-events {group}:{host} result {}", r["result"])
    })
}

fn appkit_events(
    runner: &mut Runner,
    context: &mut Context,
    gui_avail: &Result<(), String>,
    keys_avail: &Result<(), String>,
) -> std::io::Result<()> {
    let groups = [
        (
            "ime",
            "static-echo,moving-caret-30,moving-caret-60,configure-storm",
            false,
        ),
        ("cgevent", "held-repeat,modifier-storm", true),
    ];
    let mut idx = 0usize;
    for (group, fixtures, needs_keys) in groups {
        for (host, rune) in [("venus", false), ("rune", true)] {
            let (step_idx, name) = (idx, format!("appkit-events-{group}:{host}"));
            idx += 1;
            runner.step(
                context,
                &name,
                25 * MINUTE,
                &["package", "session"],
                false,
                |ctx, log, dl| {
                    if let Err(r) = gui_avail {
                        return Err(Failure::Unavailable(format!("needs GUI login: {r}")));
                    }
                    if needs_keys && let Err(r) = keys_avail {
                        return Err(Failure::Unavailable(format!("needs Accessibility: {r}")));
                    }
                    run_appkit_step(ctx, log, dl, (step_idx, group, fixtures, host, rune))
                },
            )?;
        }
    }
    Ok(())
}

fn test_packages(runner: &mut Runner, context: &mut Context) -> std::io::Result<()> {
    for name in context.packages.clone() {
        runner.step(
            context,
            &format!("test:{name}"),
            HOUR,
            &["workspace"],
            false,
            |context, log, deadline| {
                let (status, _) = execute(
                    log,
                    &mut context.nix(&["develop", "--command", "eon-test", "--package", &name]),
                    deadline,
                    false,
                    true,
                )?;
                let listed = LINUX_ONLY_TEST_PACKAGES.contains(&name.as_str());
                if listed {
                    check(!status.success(), || {
                        format!("{name} has macOS tests now; remove it from the Linux-only list")
                    })?;
                }
                if status.success() {
                    return Ok(());
                }
                let no_tests = status.code() == Some(4)
                    && log
                        .text()
                        .map_err(|error| Failure::Check(format!("reading test log: {error}")))?
                        .lines()
                        .any(|line| line.trim() == "error: no tests to run");
                match (listed, no_tests) {
                    (true, true) => Err(Failure::NotApplicable(format!(
                        "{name} is listed Linux-only and compiles no tests on macOS, \
                         as mac-proof's own tests check"
                    ))),
                    (false, true) => Err(Failure::Command(format!(
                        "eon-test --package {name} exited with {status}: it ran no tests; \
                         a Linux-only package is gated to Linux and listed in \
                         mac-proof's LINUX_ONLY_TEST_PACKAGES"
                    ))),
                    (_, false) => Err(Failure::Command(format!(
                        "eon-test --package {name} exited with {status}"
                    ))),
                }
            },
        )?;
    }
    Ok(())
}

fn checks(runner: &mut Runner, context: &mut Context) -> std::io::Result<()> {
    runner.step(
        context,
        "check-list",
        10 * MINUTE,
        &["source"],
        false,
        check_list,
    )?;
    runner.step(
        context,
        "check-inputs",
        2 * HOUR,
        &["check-list"],
        false,
        check_inputs,
    )?;
    for name in context.checks.keys().cloned().collect::<Vec<_>>() {
        let requires = if runner.status("check-inputs") == Some("pass")
            || context.ready_checks.contains(&name)
            || context.checks[&name].is_none()
        {
            "check-list"
        } else {
            "check-inputs"
        };
        runner.step(
            context,
            &format!("check:{name}"),
            HOUR,
            &[requires],
            false,
            |context, log, deadline| {
                let attribute = format!(".#checks.{SYSTEM}.{name}");
                succeed(
                    log,
                    &mut context.nix(&["build", "--no-link", "--print-build-logs", &attribute]),
                    deadline,
                    false,
                )
                .map(drop)
            },
        )?;
    }
    Ok(())
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

/// Records the machine. Fails below macOS 14, which CodexBarCLI needs.
fn host(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let version = text(succeed(
        log,
        Command::new("sw_vers").arg("-productVersion"),
        deadline,
        true,
    )?);
    for arguments in [
        &["uname", "-srm"][..],
        &["sw_vers"],
        &["sysctl", "hw.model", "hw.ncpu", "hw.memsize"],
        &["nix", "--version"],
    ] {
        execute(
            log,
            Command::new(arguments[0]).args(&arguments[1..]),
            deadline,
            false,
            true,
        )?;
    }
    retina::system_profiler(log, &["SPHardwareDataType", "SPDisplaysDataType"], deadline)?;
    // Only unsandboxed Darwin builds are proved so far (eon-zuou).
    // `nix config show` needs Nix 2.20 or later.
    let (sandbox, _) = execute(
        log,
        &mut context.nix(&["config", "show", "sandbox"]),
        deadline,
        false,
        true,
    )?;
    // Directories Eon's Darwin tests take as TMPDIR; a failed build leaves its own.
    context.private_dirs = private_dirs();
    log.line(&format!(
        "mac-proof: /tmp/e<c> directories before the run: {:?}",
        context.private_dirs
    ));
    context.processes = eon_processes(log, deadline)?;
    log.line(&format!(
        "mac-proof: Eon processes before the run: {:?}",
        context.processes
    ));
    // Venus's build tests also see the Mac's fonts, which could stand in for Eon's.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for directory in [PathBuf::from("/Library/Fonts"), home.join("Library/Fonts")] {
        let matches: Vec<String> = fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                ["dejavu", "nerd", "symbols", "accelerando"]
                    .iter()
                    .any(|font| lower.contains(font))
            })
            .collect();
        log.line(&format!(
            "mac-proof: fonts in {} like Eon's: {matches:?}",
            directory.display()
        ));
    }
    let major: u32 = version.split('.').next().unwrap_or("").parse().unwrap_or(0);
    check(major >= 14, || {
        format!("macOS {version}; Eon needs macOS 14 or later")
    })?;
    check(sandbox.success(), || {
        "nix config show sandbox failed; the harness needs Nix 2.20 or later".to_owned()
    })
}

/// Makes the source a Git checkout, as eon-test and Nix need, and checks
/// that it holds the tree that was sent.
fn source(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    if context.source.join(".git").exists() {
        log.line("mac-proof: the source is a Git checkout; recording it as it is");
        let status = succeed(
            log,
            &mut context.git(&["status", "--porcelain"]),
            deadline,
            true,
        )?;
        check(status.is_empty(), || {
            "the checkout has uncommitted changes, which Nix would build but the record would not name".to_owned()
        })?;
        context.head = text(succeed(
            log,
            &mut context.git(&["rev-parse", "HEAD"]),
            deadline,
            true,
        )?);
    } else {
        // A fresh archive: commit everything in it, ignored paths included,
        // so the commit's tree is the archive's.
        let tree = context.expect_tree.clone().unwrap_or_default();
        for arguments in [
            &["init", "--quiet", "--initial-branch=candidate"][..],
            &["add", "--all", "--force"],
            &[
                "-c",
                "user.name=mac-proof",
                "-c",
                "user.email=mac-proof@localhost",
                "commit",
                "--quiet",
                "--no-verify",
                "--no-gpg-sign",
                "--message",
                &format!("mac-proof candidate {tree}"),
            ],
        ] {
            succeed(log, &mut context.git(arguments), deadline, false)?;
        }
    }
    context.tree = text(succeed(
        log,
        &mut context.git(&["rev-parse", "HEAD^{tree}"]),
        deadline,
        true,
    )?);
    match &context.expect_tree {
        Some(expected) => check(&context.tree == expected, || {
            format!(
                "the source holds tree {}, not the sent {expected}",
                context.tree
            )
        }),
        None => Ok(()),
    }
}

fn package(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let attribute = format!(".#packages.{SYSTEM}.default");
    let mut build = context.nix(&[
        "build",
        "--print-build-logs",
        "--print-out-paths",
        "--out-link",
        "target/mac-proof/eon",
        &attribute,
    ]);
    context.package = PathBuf::from(text(succeed(log, &mut build, deadline, true)?));
    check(context.package.is_absolute(), || {
        format!("nix build printed no store path: {:?}", context.package)
    })
}

/// ORB-C10 with macOS's own ncurses: `eon` resolves only from the package's
/// terminfo, and `clear` emits xterm-256color's bytes. A cleared environment
/// and an empty HOME keep TERMINFO, TERMINFO_DIRS and ~/.terminfo from
/// supplying an entry the package lacks.
fn terminfo(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let closure = text(succeed(
        log,
        &mut context.nix(&[
            "path-info",
            "--recursive",
            &context.package.to_string_lossy(),
        ]),
        deadline,
        true,
    )?);
    let directories: Vec<PathBuf> = closure
        .lines()
        .map(|path| Path::new(path).join("share/terminfo"))
        .filter(|directory| directory.join("e/eon").is_file())
        .collect();
    let [directory] = &directories[..] else {
        return Err(Failure::Check(format!(
            "want one store path with share/terminfo/e/eon in Eon's closure, found {directories:?}"
        )));
    };
    let home = context.source.join("target/mac-proof/terminfo-home");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home)
        .map_err(|error| Failure::Check(format!("{}: {error}", home.display())))?;
    let ncurses = |entry: Option<&Path>, term: Option<&str>, program: &str, arguments: &[&str]| {
        let mut command = Command::new(program);
        command
            .args(arguments)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home);
        if let Some(entry) = entry {
            command.env("TERMINFO_DIRS", entry);
        }
        if let Some(term) = term {
            command.env("TERM", term);
        }
        command
    };
    execute(
        log,
        &mut ncurses(None, None, "/usr/bin/infocmp", &["-V"]),
        deadline,
        false,
        true,
    )?;
    let (without, _) = execute(
        log,
        &mut ncurses(None, None, "/usr/bin/infocmp", &["eon"]),
        deadline,
        false,
        true,
    )?;
    check(!without.success(), || {
        "infocmp found eon without the package's terminfo".to_owned()
    })?;
    let entry = text(succeed(
        log,
        &mut ncurses(Some(directory), None, "/usr/bin/infocmp", &["eon"]),
        deadline,
        true,
    )?);
    check(
        entry.lines().any(|line| line == "eon|Eon terminal,"),
        || "infocmp printed no `eon|Eon terminal,` line".to_owned(),
    )?;
    let clear = succeed(
        log,
        &mut ncurses(Some(directory), Some("eon"), "/usr/bin/tput", &["clear"]),
        deadline,
        true,
    )?;
    let control = succeed(
        log,
        &mut ncurses(None, Some("xterm-256color"), "/usr/bin/tput", &["clear"]),
        deadline,
        true,
    )?;
    log.line(&format!(
        "mac-proof: TERM=eon clear {clear:02x?}, xterm-256color {control:02x?}"
    ));
    let _ = fs::remove_dir_all(&home);
    check(!clear.is_empty() && clear == control, || {
        format!("TERM=eon clear is {clear:02x?}, not xterm-256color's {control:02x?}")
    })
}

/// Names the flake's Darwin checks without evaluating them, excluding `default`.
fn check_list(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let attribute = format!(".#checks.{SYSTEM}");
    let names = succeed(
        log,
        &mut context.nix(&[
            "eval",
            "--json",
            &attribute,
            "--apply",
            "builtins.attrNames",
        ]),
        deadline,
        true,
    )?;
    let names: Vec<String> = serde_json::from_slice(&names)
        .map_err(|error| Failure::Check(format!("nix eval printed no list of names: {error}")))?;
    context.checks = names
        .into_iter()
        .filter(|name| name != "default")
        .map(|name| (name, None))
        .collect();
    check(!context.checks.is_empty(), || {
        "the flake has no Darwin checks".to_owned()
    })
}

/// Evaluates derivations separately from discovery, then builds their inputs once.
fn check_inputs(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    context.ready_checks.clear();
    let attribute = format!(".#checks.{SYSTEM}");
    let drvs = succeed(
        log,
        &mut context.nix(&[
            "eval",
            "--json",
            &attribute,
            "--apply",
            "checks: builtins.mapAttrs (_: check: let result = builtins.tryEval check.drvPath; in if result.success then result.value else null) (builtins.removeAttrs checks [\"default\"])",
        ]),
        deadline,
        true,
    );
    let drvs: BTreeMap<String, Option<String>> = match drvs {
        Ok(drvs) => serde_json::from_slice(&drvs).map_err(|error| {
            Failure::Check(format!("nix eval printed no check derivations: {error}"))
        })?,
        Err(Failure::Timeout) => return Err(Failure::Timeout),
        Err(_) => {
            log.line(
                "mac-proof: combined derivation evaluation failed; evaluating checks individually",
            );
            let mut drvs = BTreeMap::new();
            for name in context.checks.keys() {
                let attribute = format!(".#checks.{SYSTEM}.{name}.drvPath");
                match execute(
                    log,
                    &mut context.nix(&["eval", "--raw", &attribute]),
                    deadline,
                    true,
                    true,
                ) {
                    Ok((status, drv)) if status.success() => {
                        drvs.insert(name.clone(), Some(text(drv)));
                    }
                    Err(Failure::Timeout) => return Err(Failure::Timeout),
                    _ => {}
                }
            }
            drvs
        }
    };
    for drv in drvs.values().flatten() {
        check(Path::new(drv).is_absolute(), || {
            format!("nix eval printed a non-absolute derivation path: {drv}")
        })?;
    }
    for (name, drv) in drvs {
        if let Some(path) = context.checks.get_mut(&name) {
            *path = drv;
        }
    }
    let mut inputs = BTreeMap::new();
    let mut all = BTreeSet::new();
    for (name, drv) in &context.checks {
        let Some(drv) = drv else { continue };
        let references = store_paths(succeed(
            log,
            &mut context.nix_store(&["--query", "--references", drv]),
            deadline,
            true,
        )?)?;
        let references: BTreeSet<_> = references
            .into_iter()
            .filter(|path| path.ends_with(".drv"))
            .collect();
        all.extend(references.iter().cloned());
        inputs.insert(name.clone(), references);
    }
    let built = if all.is_empty() {
        Ok(())
    } else {
        let mut command =
            context.nix(&["build", "--keep-going", "--no-link", "--print-build-logs"]);
        command.args(all.iter().map(|drv| format!("{drv}^*")));
        succeed(log, &mut command, deadline, false).map(drop)
    };
    if matches!(&built, Err(Failure::Timeout)) {
        return built;
    }
    let mut outputs = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for drv in &all {
        let drv_outputs = store_paths(succeed(
            log,
            &mut context.nix_store(&["--query", "--outputs", drv]),
            deadline,
            true,
        )?)?;
        paths.extend(drv_outputs.iter().cloned());
        outputs.insert(drv, drv_outputs);
    }
    let invalid = if paths.is_empty() {
        BTreeSet::new()
    } else {
        let mut command = context.nix_store(&["--check-validity", "--print-invalid"]);
        command.args(&paths);
        store_paths(succeed(log, &mut command, deadline, true)?)?
    };
    let failed: BTreeSet<_> = outputs
        .into_iter()
        .filter(|(_, paths)| !paths.is_disjoint(&invalid))
        .map(|(drv, _)| drv.clone())
        .collect();
    // Publish only a complete result; a failed query leaves every check blocked.
    context.ready_checks = inputs
        .into_iter()
        .filter(|(_, inputs)| inputs.is_disjoint(&failed))
        .map(|(name, _)| name)
        .collect();
    built
}

fn store_paths(bytes: Vec<u8>) -> Result<BTreeSet<String>, Failure> {
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            check(Path::new(line).is_absolute(), || {
                format!("nix-store printed a non-absolute path: {line}")
            })?;
            Ok(line.to_owned())
        })
        .collect()
}

/// The workspace's packages, which eon-test then tests one at a time.
fn workspace(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let metadata = succeed(
        log,
        &mut context.nix(&[
            "develop",
            "--command",
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ]),
        deadline,
        true,
    )?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata)
        .map_err(|error| Failure::Check(format!("cargo metadata printed no JSON: {error}")))?;
    context.packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| Failure::Check("cargo metadata printed no packages array".to_owned()))?
        .iter()
        .map(|package| {
            package["name"].as_str().map(str::to_owned).ok_or_else(|| {
                Failure::Check(format!(
                    "cargo metadata listed a package without a name: {package}"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    check(!context.packages.is_empty(), || {
        "cargo metadata listed no packages".to_owned()
    })?;
    for name in LINUX_ONLY_TEST_PACKAGES {
        check(
            context.packages.iter().any(|package| package == name),
            || {
                format!(
                    "Linux-only test package {name} is absent from cargo metadata; remove it from the list"
                )
            },
        )?;
    }
    Ok(())
}

/// Fails when the run left an Eon process or a /tmp/e<c> directory behind.
fn leftovers(context: &mut Context, log: &mut Log, deadline: Instant) -> Outcome {
    let profiles = Path::new("/Library/ColorSync/Profiles/Displays");
    match fs::read_dir(profiles) {
        Ok(entries) => {
            let mut found = false;
            for entry in entries {
                match entry {
                    Ok(entry)
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .contains(crate::mac::VIRTUAL_DISPLAY_NAME) =>
                    {
                        found = true;
                        log.line(&format!(
                            "mac-proof: {} is expected, known residue of retina's virtual display",
                            entry.path().display()
                        ));
                    }
                    Ok(_) => (),
                    Err(error) => log.line(&format!(
                        "mac-proof: reading {}: {error}",
                        profiles.display()
                    )),
                }
            }
            if !found {
                log.line("mac-proof: no ColorSync profile for retina's virtual display");
            }
        }
        Err(error) => log.line(&format!(
            "mac-proof: reading {}: {error}",
            profiles.display()
        )),
    }
    let directories: Vec<_> = private_dirs()
        .difference(&context.private_dirs)
        .cloned()
        .collect();
    let processes: Vec<_> = eon_processes(log, deadline)?
        .difference(&context.processes)
        .cloned()
        .collect();
    check(directories.is_empty() && processes.is_empty(), || {
        format!("left behind: directories {directories:?}, processes {processes:?}")
    })
}

fn private_dirs() -> BTreeSet<String> {
    fs::read_dir("/tmp")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            let bytes = name.as_bytes();
            name.starts_with("ev-")
                || (bytes.len() == 2
                    && bytes[0] == b'e'
                    && (bytes[1].is_ascii_digit() || bytes[1].is_ascii_lowercase()))
        })
        .collect()
}

fn is_eon_process(name: &str) -> bool {
    EON_PROCESSES.contains(&name) || name.starts_with("appkit_event_driver")
}

/// This user's Eon processes, as (pid, command).
fn eon_processes(log: &mut Log, deadline: Instant) -> Result<BTreeSet<(String, String)>, Failure> {
    Ok(scenario::listing(log, deadline)?
        .into_iter()
        .filter(|process| is_eon_process(process.name()))
        .map(|process| (process.pid.to_string(), process.command))
        .collect())
}

#[cfg(test)]
mod macos_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn personal_commands_use_a_private_cargo_home_that_is_removed_on_drop() {
        let context = Context::default();
        assert!(
            !context
                .command("git")
                .get_envs()
                .any(|(key, _)| key == "CARGO_HOME")
        );
        let path;
        {
            let cargo_home = CargoHome::new().unwrap();
            path = cargo_home.0.clone();
            assert!(path.starts_with(std::env::temp_dir()));
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let context = Context {
                cargo_home: Some(path.clone()),
                ..Context::default()
            };
            for command in [
                context.git(&["status"]),
                context.nix(&["develop"]),
                context.nix_store(&["--query"]),
            ] {
                assert!(
                    command
                        .get_envs()
                        .any(|(key, value)| key == "CARGO_HOME" && value == Some(path.as_os_str()))
                );
            }
            fs::write(path.join("download"), "cargo cache").unwrap();
        }
        assert!(!path.exists());
    }

    fn stub(evidence: &Path) -> Context {
        let program = evidence.join("nix-stub");
        fs::write(
            &program,
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$(dirname "$0")/calls"
if [ "$1" = --extra-experimental-features ]; then shift 3; fi
case "$1 $2" in
  'develop --command')
    case "$3" in
      cargo)
        if [ "$4" = test ]; then
          mkdir -p "$EON_APPKIT_PROOF_OUT"
          printf '%s\n' "${EON_VENUS_FONT_DIRS:-}" > "$(dirname "$0")/font-dirs"
          cat "$(dirname "$0")/appkit-result.json" > "$EON_APPKIT_PROOF_OUT/result.json"
          exit "$(cat "$(dirname "$0")/appkit.exit")"
        fi
        cat "$(dirname "$0")/metadata.json" ;;
      eon-test)
        cat "$(dirname "$0")/$5.output" >&2
        exit "$(cat "$(dirname "$0")/$5.exit")" ;;
      *) exit 2 ;;
    esac ;;
  'eval --json')
    case "$5" in
      builtins.attrNames) echo '["a","b","c","d","default"]' ;;
      *)
        if [ -f "$(dirname "$0")/fail-eval" ]; then
          echo 'error: missing attribute' >&2; exit 1
        fi
        echo '{"a":"/nix/store/a.drv","b":"/nix/store/b.drv","c":"/nix/store/c.drv","d":null}' ;;
    esac ;;
  'eval --raw')
    case "$3" in
      '.#checks.aarch64-darwin.a.drvPath') echo /nix/store/a.drv ;;
      '.#checks.aarch64-darwin.b.drvPath') echo /nix/store/b.drv ;;
      '.#checks.aarch64-darwin.c.drvPath') echo /nix/store/c.drv ;;
      *) echo 'error: check d does not evaluate' >&2; exit 1 ;;
    esac ;;
  '--query --references')
    case "$3" in
      /nix/store/a.drv|/nix/store/b.drv)
        printf ' /nix/store/shared.drv \n/nix/store/shared.drv\n/nix/store/source\n\n' ;;
      /nix/store/c.drv)
        if [ -f "$(dirname "$0")/fail-eval" ]; then
          echo /nix/store/shared.drv
        else
          echo /nix/store/good.drv
        fi ;;
      *) exit 2 ;;
    esac ;;
  'build --keep-going') echo 'error: shared input failed' >&2; exit 1 ;;
  '--query --outputs')
    case "$3" in
      /nix/store/shared.drv) printf '/nix/store/shared-out\n/nix/store/shared-dev\n' ;;
      /nix/store/good.drv) echo /nix/store/good-out ;;
      *) exit 2 ;;
    esac ;;
  '--check-validity --print-invalid') echo /nix/store/shared-dev ;;
  'build --no-link')
    case "$4" in
      '.#checks.aarch64-darwin.c') exit 0 ;;
      '.#checks.aarch64-darwin.d') echo 'error: check d does not evaluate' >&2; exit 1 ;;
      *) echo 'error: rebuilt a known failure' >&2; exit 1 ;;
    esac ;;
  *) echo 'error: unexpected invocation' >&2; exit 2 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Context {
            source: evidence.to_owned(),
            nix_program: Some(program.to_str().unwrap().to_owned()),
            nix_store_program: Some(program.to_str().unwrap().to_owned()),
            ..Context::default()
        }
    }

    fn evidence_logs(evidence: &Path) -> String {
        let mut paths = vec![evidence.join("calls")];
        let mut logs = String::new();
        match fs::read_dir(evidence.join("steps")).and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()
        }) {
            Ok(steps) => paths.extend(steps),
            Err(error) => logs.push_str(&format!("reading step logs: {error}\n")),
        }
        paths.sort();
        for path in paths {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| format!("could not read: {error}"));
            logs.push_str(&format!("\n{}:\n{text}\n", path.display()));
        }
        logs
    }

    #[test]
    fn linux_only_test_exclusion_is_checked_and_does_not_hide_missing_tests() {
        let root =
            std::env::temp_dir().join(format!("mac-proof-test-packages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut context = stub(&root);
        let no_tests = "error: no tests to run\n";
        let other = "error: different failure\n";
        let listed = ["agent-jail", "eon-boat", "rota-cli"];
        assert_eq!(LINUX_ONLY_TEST_PACKAGES, listed);
        // (metadata, listed exit and output, unlisted exit and output,
        //  listed status, unlisted status, unlisted failure contains the hint)
        let all = serde_json::json!({"packages": [
            {"name": "agent-jail"}, {"name": "eon-boat"}, {"name": "rota-cli"}, {"name": "plain"}
        ]});
        let cases = [
            (
                all.clone(),
                4,
                no_tests,
                0,
                "",
                "not-applicable",
                "pass",
                false,
            ),
            (all.clone(), 0, "", 0, "", "fail", "pass", false),
            (all.clone(), 4, other, 0, "", "fail", "pass", false),
            (all.clone(), 101, no_tests, 0, "", "fail", "pass", false),
            (
                all.clone(),
                4,
                no_tests,
                4,
                no_tests,
                "not-applicable",
                "fail",
                true,
            ),
            (
                all.clone(),
                4,
                no_tests,
                4,
                other,
                "not-applicable",
                "fail",
                false,
            ),
            (
                all.clone(),
                4,
                no_tests,
                101,
                no_tests,
                "not-applicable",
                "fail",
                false,
            ),
            (
                serde_json::json!({"packages": [
                    {"name": "agent-jail"}, {"name": "eon-boat"}, {"name": "plain"}
                ]}),
                4,
                no_tests,
                0,
                "",
                "absent",
                "skipped",
                false,
            ),
            (
                serde_json::json!({"packages": [
                    {"name": "agent-jail"}, {"name": "eon-boat"}, {"name": "rota-cli"},
                    {"version": "1"}
                ]}),
                4,
                no_tests,
                0,
                "",
                "absent",
                "skipped",
                false,
            ),
        ];
        let mut runs = 0;
        for (
            index,
            (metadata, listed_exit, listed_output, exit, output, listed_status, status, hint),
        ) in cases.into_iter().enumerate()
        {
            let evidence = root.join(index.to_string());
            let mut runner = Runner::new(&evidence, false, MINUTE).unwrap();
            context.packages.clear();
            fs::write(root.join("metadata.json"), metadata.to_string()).unwrap();
            for name in listed {
                fs::write(root.join(format!("{name}.exit")), listed_exit.to_string()).unwrap();
                fs::write(root.join(format!("{name}.output")), listed_output).unwrap();
            }
            fs::write(root.join("plain.exit"), exit.to_string()).unwrap();
            fs::write(root.join("plain.output"), output).unwrap();
            runner
                .step(&mut context, "source", MINUTE, &[], false, |_, _, _| Ok(()))
                .unwrap();
            runner
                .step(
                    &mut context,
                    "workspace",
                    MINUTE,
                    &["source"],
                    false,
                    workspace,
                )
                .unwrap();
            test_packages(&mut runner, &mut context).unwrap();
            let passed = runner.write(true).unwrap();
            let summary: serde_json::Value =
                serde_json::from_slice(&fs::read(evidence.join("summary.json")).unwrap()).unwrap();
            let steps = summary["steps"].as_array().unwrap();
            let step = |name: &str| steps.iter().find(|step| step["name"] == name);
            let logs = evidence_logs(&evidence);
            assert_eq!(passed, index == 0, "case {index}: {logs}");
            if listed_status == "absent" {
                let workspace = step("workspace").unwrap();
                assert_eq!(workspace["status"], "fail", "{logs}");
                assert!(
                    workspace["first_failure"]
                        .as_str()
                        .unwrap()
                        .contains(if index == 7 {
                            "rota-cli is absent"
                        } else {
                            "without a name"
                        }),
                    "case {index}: {logs}"
                );
                // No package is tested after a failed workspace step.
                for step in steps {
                    if step["name"].as_str().unwrap().starts_with("test:") {
                        assert_eq!(step["status"], status, "case {index}: {logs}");
                    }
                }
                continue;
            }
            runs += 1;
            assert_eq!(step("workspace").unwrap()["status"], "pass", "{logs}");
            for name in listed {
                let step = step(&format!("test:{name}")).unwrap();
                assert_eq!(step["status"], listed_status, "case {index}: {logs}");
                assert_eq!(step["for_yu"], false);
                if listed_status == "not-applicable" {
                    assert!(
                        step["first_failure"]
                            .as_str()
                            .unwrap()
                            .contains("listed Linux-only")
                    );
                }
            }
            let plain = step("test:plain").unwrap();
            assert_eq!(plain["status"], status, "case {index}: {logs}");
            assert_eq!(
                logs.contains("listed in mac-proof's LINUX_ONLY_TEST_PACKAGES"),
                hint,
                "case {index}: {logs}"
            );
        }
        let calls = fs::read_to_string(root.join("calls")).unwrap();
        for name in ["agent-jail", "eon-boat", "rota-cli", "plain"] {
            assert_eq!(
                calls
                    .lines()
                    .filter(|line| line.ends_with(&format!("eon-test --package {name}")))
                    .count(),
                runs,
                "{name}"
            );
        }
        assert!(!calls.contains("--no-tests"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_shared_inputs_skip_only_the_checks_that_need_them() {
        let evidence =
            std::env::temp_dir().join(format!("mac-proof-check-inputs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&evidence);
        let mut runner = Runner::new(&evidence, false, MINUTE).unwrap();
        let mut context = stub(&evidence);
        runner
            .step(&mut context, "source", MINUTE, &[], false, |_, _, _| Ok(()))
            .unwrap();
        checks(&mut runner, &mut context).unwrap();
        assert!(!runner.write(true).unwrap());
        let summary: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence.join("summary.json")).unwrap()).unwrap();
        let steps = summary["steps"].as_array().unwrap();
        let step = |name: &str| steps.iter().find(|step| step["name"] == name).unwrap();
        for name in ["check:a", "check:b"] {
            assert_eq!(
                step(name)["status"],
                "skipped",
                "{}",
                evidence_logs(&evidence)
            );
            assert_eq!(
                step(name)["first_failure"],
                "needs check-inputs, which did not pass"
            );
        }
        assert_eq!(
            step("check:c")["status"],
            "pass",
            "{}",
            evidence_logs(&evidence)
        );
        assert_eq!(
            step("check:d")["status"],
            "fail",
            "{}",
            evidence_logs(&evidence)
        );
        assert_eq!(
            step("check:d")["first_failure"],
            "error: check d does not evaluate"
        );
        assert_eq!(
            step("check-inputs")["status"],
            "fail",
            "{}",
            evidence_logs(&evidence)
        );
        assert_eq!(
            step("check-inputs")["first_failure"],
            "error: shared input failed"
        );
        assert_eq!(summary["result"], "fail");
        let calls = fs::read_to_string(evidence.join("calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("build --keep-going"))
                .collect::<Vec<_>>(),
            [
                "--extra-experimental-features nix-command flakes --accept-flake-config build --keep-going --no-link --print-build-logs /nix/store/good.drv^* /nix/store/shared.drv^*"
            ]
        );
        let check_builds: Vec<_> = calls
            .lines()
            .filter(|line| line.contains("build --no-link"))
            .collect();
        assert_eq!(check_builds.len(), 2);
        assert!(check_builds[0].ends_with(".#checks.aarch64-darwin.c"));
        assert!(check_builds[1].ends_with(".#checks.aarch64-darwin.d"));
        assert!(
            calls
                .lines()
                .filter(|line| line.starts_with("--query") || line.starts_with("--check-validity"))
                .all(|line| !line.contains("/nix/store/d.drv"))
        );
        assert!(!calls.contains("/nix/store/default.drv"));
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn failed_combined_evaluation_keeps_the_shared_build() {
        let evidence =
            std::env::temp_dir().join(format!("mac-proof-check-eval-{}", std::process::id()));
        let _ = fs::remove_dir_all(&evidence);
        let mut runner = Runner::new(&evidence, false, MINUTE).unwrap();
        let mut context = stub(&evidence);
        fs::write(evidence.join("fail-eval"), "").unwrap();
        runner
            .step(&mut context, "source", MINUTE, &[], false, |_, _, _| Ok(()))
            .unwrap();
        checks(&mut runner, &mut context).unwrap();
        assert!(!runner.write(true).unwrap());
        assert_eq!(
            runner.status("check-list"),
            Some("pass"),
            "{}",
            evidence_logs(&evidence)
        );
        assert_eq!(
            runner.status("check-inputs"),
            Some("fail"),
            "{}",
            evidence_logs(&evidence)
        );
        for name in ["a", "b", "c"] {
            assert_eq!(
                runner.status(&format!("check:{name}")),
                Some("skipped"),
                "{}",
                evidence_logs(&evidence)
            );
        }
        assert_eq!(
            runner.status("check:d"),
            Some("fail"),
            "{}",
            evidence_logs(&evidence)
        );
        let calls = fs::read_to_string(evidence.join("calls")).unwrap();
        let shared_builds: Vec<_> = calls
            .lines()
            .filter(|line| line.contains("build --keep-going"))
            .collect();
        assert_eq!(shared_builds.len(), 1);
        assert!(shared_builds[0].ends_with(" --print-build-logs /nix/store/shared.drv^*"));
        let check_builds: Vec<_> = calls
            .lines()
            .filter(|line| line.contains("build --no-link"))
            .collect();
        assert_eq!(check_builds.len(), 1);
        assert!(check_builds[0].ends_with(".#checks.aarch64-darwin.d"));
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("eval --raw"))
                .count(),
            4
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    #[test]
    fn appkit_events_steps_verify_receipts_and_availability() {
        use std::{
            os::unix::{net::UnixStream, process::CommandExt},
            process::Stdio,
        };
        struct Reap(std::process::Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        assert!(is_eon_process("appkit_event_driver-0123456789abcdef"));
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("mac-proof-appkit-{pid}"));
        let _ = fs::remove_dir_all(&root);
        let pkg = root.join("pkg");
        fs::create_dir_all(pkg.join("bin")).unwrap();
        let (orbit, venus) = (pkg.join("bin/yazelix-orbit"), pkg.join("bin/yazelix-venus"));
        fs::write(&orbit, "#!/bin/sh\n").unwrap();
        let font = "export EON_VENUS_FONT_DIRS='/nix/store/a-fonts:/nix/store/b-fonts'\n";
        fs::write(&venus, font).unwrap();
        let (o, v) = (orbit.display(), venus.display());
        let eon = format!("export EON_ORBIT='{o}'\nexport EON_VENUS='{v}'\n");
        fs::write(pkg.join("bin/eon"), eon).unwrap();
        let mut ctx = stub(&root);
        ctx.package = pkg;
        let nk = Err("no keys".into());
        let cases = [
            (r#"{"result":"pass"}"#, Ok(()), "pass", "pass"),
            (r#"{"result":"fail"}"#, nk, "fail", "skipped"),
        ];
        for (idx, (result, keys, ime, cg)) in cases.into_iter().enumerate() {
            let ev = root.join(idx.to_string());
            ctx.evidence = ev.clone();
            fs::write(root.join("appkit-result.json"), result).unwrap();
            fs::write(root.join("appkit.exit"), "0").unwrap();
            let mut r = Runner::new(&ev, false, MINUTE).unwrap();
            for dep in ["package", "session"] {
                r.step(&mut ctx, dep, MINUTE, &[], false, |_, _, _| Ok(()))
                    .unwrap();
            }
            appkit_events(&mut r, &mut ctx, &Ok(()), &keys).unwrap();
            assert_eq!(r.write(true).unwrap(), idx == 0);
            let raw = fs::read(ev.join("summary.json")).unwrap();
            let s: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            let steps = s["steps"].as_array().unwrap();
            let st = |n| steps.iter().find(|x| x["name"] == n).unwrap();
            assert_eq!(st("appkit-events-ime:venus")["status"], ime);
            assert_eq!(st("appkit-events-ime:rune")["status"], ime);
            assert_eq!(st("appkit-events-cgevent:venus")["status"], cg);
            assert_eq!(st("appkit-events-cgevent:rune")["for_yu"], idx == 1);
        }
        let calls = fs::read_to_string(root.join("calls")).unwrap();
        let needle = "develop --command cargo test --locked -p yazelix-venus --features candidate-rune --test appkit_event_driver";
        assert!(calls.contains(needle));
        let dirs = fs::read_to_string(root.join("font-dirs")).unwrap();
        assert_eq!(dirs.trim(), "/nix/store/a-fonts:/nix/store/b-fonts");
        let (orphan_root, sibling_root) = (
            PathBuf::from(format!("/tmp/ev-test-cleanup-{pid}")),
            PathBuf::from(format!("/tmp/ev-test-cleanup-{pid}0")),
        );
        for d in [&orphan_root, &sibling_root] {
            let _ = fs::remove_dir_all(d);
            fs::create_dir_all(d.join("0")).unwrap();
        }
        let (life_sock, ready) = (orphan_root.join("life.sock"), root.join("life.ready"));
        let life = RunLife::bind(life_sock.clone()).unwrap();
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "run::tests::life_peer", "--ignored"])
            .envs([("EON_LIFE_S", &life_sock), ("EON_LIFE_R", &ready)])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut peer = Reap(cmd.spawn().unwrap());
        let end = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(peer.0.try_wait().unwrap().is_none() && Instant::now() < end);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(peer.0.try_wait().unwrap().is_none());
        drop(life);
        let end = Instant::now() + Duration::from_secs(5);
        while peer.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < end, "detached peer survived RunLife drop");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(peer.0.wait().unwrap().success() && orphan_root.join("0").exists());
        assert!(UnixStream::connect(&life_sock).is_err());
        let mut log = Log::new(fs::File::create(root.join("cleanup.log")).unwrap());
        cleanup_appkit_root(&orphan_root, RunLife::bind(life_sock).unwrap(), &mut log).unwrap();
        assert!(!orphan_root.exists() && sibling_root.exists());
        let _ = fs::remove_dir_all(&sibling_root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore]
    fn life_peer() {
        use std::io::{ErrorKind, Read};
        let (Ok(s), Ok(r)) = (std::env::var("EON_LIFE_S"), std::env::var("EON_LIFE_R")) else {
            return;
        };
        let mut c = std::os::unix::net::UnixStream::connect(s).unwrap();
        let mut b = [0u8; 1];
        c.set_nonblocking(true).unwrap();
        assert_eq!(c.read(&mut b).unwrap_err().kind(), ErrorKind::WouldBlock);
        c.set_nonblocking(false).unwrap();
        fs::write(r, "").unwrap();
        let res = c.read(&mut b);
        assert!(matches!(res, Ok(0)) || res.is_err_and(|e| e.kind() == ErrorKind::ConnectionReset));
    }
}
