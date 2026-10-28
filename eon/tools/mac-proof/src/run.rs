//! `run`: the native steps on the Mac.

use crate::{
    scenario::{self, Scenario},
    step::{Failure, Log, Outcome, Runner, check, execute, succeed},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
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
}

pub struct ScenarioOptions {
    pub package: PathBuf,
    pub evidence: PathBuf,
    pub limit: Duration,
    pub dry_run: bool,
}

#[derive(Default)]
struct Context {
    source: PathBuf,
    evidence: PathBuf,
    expect_tree: Option<String>,
    tree: String,
    head: String,
    package: PathBuf,
    checks: Vec<String>,
    packages: Vec<String>,
    private_dirs: BTreeSet<String>,
    processes: BTreeSet<(String, String)>,
}

impl Context {
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.source);
        command
    }

    fn nix(&self, arguments: &[&str]) -> Command {
        let mut command = self.command("nix");
        command
            .args(["--extra-experimental-features", "nix-command flakes"])
            .arg("--accept-flake-config")
            .args(arguments);
        command
    }

    fn git(&self, arguments: &[&str]) -> Command {
        let mut command = self.command("git");
        command.args(arguments);
        command
    }
}

/// Runs every step and returns whether all passed.
pub fn run(options: Options) -> Result<bool, String> {
    if !options.dry_run && !NATIVE {
        return Err("run is macOS only (Apple Silicon); use --dry-run elsewhere".to_owned());
    }
    if options.evidence.exists() {
        return Err(format!(
            "{} exists; give a fresh --out",
            options.evidence.display()
        ));
    }
    let source = fs::canonicalize(".").map_err(|error| format!("current directory: {error}"))?;
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
    let mut context = Context {
        source,
        evidence: options.evidence,
        expect_tree: options.expect_tree,
        ..Context::default()
    };
    steps(&mut runner, &mut context).map_err(|error| format!("evidence: {error}"))?;
    runner
        .write(true)
        .map_err(|error| format!("evidence: {error}"))
}

/// Runs the scenario alone, on a package built before, and returns whether
/// every step passed.
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
    let mut scenario = Scenario::new(package, options.evidence.clone());
    scenario::steps(&mut runner, &mut scenario, &[])
        .map_err(|error| format!("evidence: {error}"))?;
    runner
        .write(true)
        .map_err(|error| format!("evidence: {error}"))
}

fn steps(runner: &mut Runner, context: &mut Context) -> std::io::Result<()> {
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
    runner.step(
        context,
        "check-list",
        10 * MINUTE,
        &["source"],
        false,
        check_list,
    )?;
    for name in context.checks.clone() {
        runner.step(
            context,
            &format!("check:{name}"),
            HOUR,
            &["check-list"],
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
    let mut gui = Scenario::new(context.package.clone(), context.evidence.clone());
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
        &["uname", "-a"][..],
        &["sw_vers"],
        &["sysctl", "hw.model", "hw.ncpu", "hw.memsize"],
        &[
            "system_profiler",
            "-json",
            "SPHardwareDataType",
            "SPDisplaysDataType",
        ],
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
    // Venus's build tests run /bin/ps, which a sandboxed Darwin builder
    // hides. `nix config show` needs Nix 2.20 or later.
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

/// The flake's Darwin checks, but `default`, the package already built.
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
    context.checks = names.into_iter().filter(|name| name != "default").collect();
    check(!context.checks.is_empty(), || {
        "the flake has no Darwin checks".to_owned()
    })
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
