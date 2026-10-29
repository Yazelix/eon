//! `run`: the native steps on the Mac.

use crate::{
    retina,
    scenario::{self, Scenario},
    step::{Failure, Log, Outcome, Runner, check, execute, progress, succeed},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);
const SYSTEM: &str = "aarch64-darwin";
const NATIVE: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));
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
    for name in context.packages.clone() {
        runner.step(
            context,
            &format!("test:{name}"),
            HOUR,
            &["workspace"],
            false,
            |context, log, deadline| {
                succeed(
                    log,
                    &mut context.nix(&["develop", "--command", "eon-test", "--package", &name]),
                    deadline,
                    false,
                )
                .map(drop)
            },
        )?;
    }
    // Dropped before leftovers, so that it has stopped whatever it started.
    let mut gui = Scenario::new(
        context.package.clone(),
        context.evidence.clone(),
        scenario::Environment::Private,
        personal_mac,
    );
    scenario::steps(runner, &mut gui, &["package"])?;
    drop(gui);
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
        .into_iter()
        .flatten()
        .filter_map(|package| package["name"].as_str().map(str::to_owned))
        .collect();
    check(!context.packages.is_empty(), || {
        "cargo metadata listed no packages".to_owned()
    })
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
            bytes.len() == 2
                && bytes[0] == b'e'
                && (bytes[1].is_ascii_digit() || bytes[1].is_ascii_lowercase())
        })
        .collect()
}

/// This user's Eon processes, as (pid, command).
fn eon_processes(log: &mut Log, deadline: Instant) -> Result<BTreeSet<(String, String)>, Failure> {
    Ok(scenario::listing(log, deadline)?
        .into_iter()
        .filter(|process| EON_PROCESSES.contains(&process.name()))
        .map(|process| (process.pid.to_string(), process.command))
        .collect())
}

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
        let calls = fs::read_to_string(evidence.join("calls")).unwrap();
        for name in ["check:a", "check:b"] {
            assert_eq!(step(name)["status"], "skipped", "{calls}");
            assert_eq!(
                step(name)["first_failure"],
                "needs check-inputs, which did not pass"
            );
        }
        assert_eq!(step("check:c")["status"], "pass");
        assert_eq!(step("check:d")["status"], "fail");
        assert_eq!(
            step("check:d")["first_failure"],
            "error: check d does not evaluate"
        );
        assert_eq!(step("check-inputs")["status"], "fail");
        assert_eq!(
            step("check-inputs")["first_failure"],
            "error: shared input failed"
        );
        assert_eq!(summary["result"], "fail");
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
        assert_eq!(runner.status("check-list"), Some("pass"));
        assert_eq!(runner.status("check-inputs"), Some("fail"));
        for name in ["a", "b", "c"] {
            assert_eq!(runner.status(&format!("check:{name}")), Some("skipped"));
        }
        assert_eq!(runner.status("check:d"), Some("fail"));
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
}
