//! `eon update --check/--install`, Eon's install-update helper (EON-C41). It finds the
//! one installation this `eon` came from, reads its owner and source from the
//! package manager, asks the official repository where that source is now,
//! and reports whether an update exists and whether Eon may install it. It
//! upgrades only a consenting profile entry, preserving running work, and runs
//! only when invoked: normal launch never reaches it.

use std::{
    env,
    ffi::{CString, OsStr, OsString},
    fs,
    io::{IsTerminal, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::OpenOptionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const REPOSITORY: &str = "https://github.com/Yazelix/eon";
const API: &str = "https://api.github.com/repos/Yazelix/eon";
/// The composition's source receipt, beside the package's `bin`.
const RECEIPT: &str = "share/eon-source-receipt.json";
/// The words that select a package mode; bare `eon update` is live adoption's.
const PACKAGE_WORDS: [&str; 5] = ["--check", "--install", "--profile", "--entry", "--prefix"];
/// How long `nix profile list` may take: a fixed safety budget for a local read.
const NIX_LIMIT: Duration = Duration::from_secs(30);
/// curl's own limit is 30 s; this bounds a curl that ignores it.
const CURL_LIMIT: Duration = Duration::from_secs(45);
/// Fixed cancellation safety budget before killing the helper-owned process group.
const TERMINATION_GRACE: Duration = Duration::from_secs(5);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Whether this command line is a package mode of `eon update`, which the
/// product answers before Runtime sees it. Help stays with Runtime.
pub(crate) fn wanted(arguments: &[OsString]) -> bool {
    let Some((program, words)) = arguments.split_first() else {
        return false;
    };
    let Some((command, options)) = words.split_first() else {
        return false;
    };
    Path::new(program).file_name() == Some(OsStr::new("eon"))
        && command == "update"
        && options
            .iter()
            .any(|word| PACKAGE_WORDS.iter().any(|package| word == package))
        && !options.iter().any(|word| word == "-h" || word == "--help")
}

/// Runs `eon update` from `arguments` (the whole command line), prints the
/// result and returns the exit status.
pub(crate) fn run(arguments: &[OsString], invoked: Build, defaults: &eon_runtime::Defaults) -> u8 {
    let mut request = parse(&arguments[2..]);
    let mut config_error = None;
    if matches!(request.selection, Ok(None)) {
        match eon_runtime::update_target(defaults) {
            Ok(target) => {
                request.selection = Ok(target.map(|(profile, entry)| Selector { profile, entry }))
            }
            Err(message) => config_error = Some(message),
        }
    }
    let places = Places::of_this_process(&arguments[0]);
    let system = System {
        version: env!("CARGO_PKG_VERSION"),
        curl: env::var_os("EON_CURL")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
    };
    let report = match &request.selection {
        Ok(_) if config_error.is_some() => {
            let mut report = Report::invalid(request.operation, config_error.unwrap());
            report.error.as_mut().unwrap().code = "invalid-configuration";
            report
        }
        Ok(selector) => match Signals::new() {
            Ok(_signals) => {
                let mut report = if request.operation == Some("install") {
                    install(&request, invoked, &places, &system)
                } else {
                    check(
                        selector.as_ref(),
                        invoked,
                        &places,
                        &system,
                        SystemTime::now(),
                    )
                };
                if INTERRUPTED.swap(false, Ordering::Relaxed) {
                    let stage = report.error.as_ref().map_or("check", |error| error.stage);
                    report.fail(
                        "interrupted",
                        stage,
                        "Update interrupted; helper-owned children stopped".into(),
                    );
                }
                report
            }
            Err(error) => {
                let mut report = Report::invalid(request.operation, error.to_string());
                report.fail("selection-failed", "resolve", error.to_string());
                report
            }
        },
        Err(message) => Report::invalid(request.operation, message.clone()),
    };
    if request.json {
        println!("{}", report.json());
        // The object has no place for why discovery could not decide.
        if report.capability.reason == Some("install-ambiguous") {
            eprintln!(
                "eon update: {}",
                report.owner.as_deref().unwrap_or_default()
            );
        }
    } else {
        print!("{}", report.human());
    }
    if let Some(error) = &report.error {
        eprintln!("eon update: {}", error.message);
    }
    report.status()
}

struct Request {
    json: bool,
    yes: bool,
    operation: Option<&'static str>,
    selection: Result<Option<Selector>, String>,
}

/// An explicit profile entry, `--profile ABS --entry NAME`.
#[derive(Clone, Debug, PartialEq)]
struct Selector {
    profile: PathBuf,
    entry: String,
}

fn parse(options: &[OsString]) -> Request {
    let mut json = false;
    let mut modes = Vec::new();
    let mut yes = false;
    let (mut profile, mut entry, mut prefix) = (None, None, None);
    let mut problem = None;
    let mut words = options.iter();
    while let Some(word) = words.next() {
        let value = match word.to_str() {
            Some("--json") if !json => {
                json = true;
                continue;
            }
            Some("--yes") if !yes => {
                yes = true;
                continue;
            }
            Some(mode @ ("--check" | "--install")) => {
                let mode = if mode == "--check" {
                    "check"
                } else {
                    "install"
                };
                if modes.contains(&mode) {
                    problem.get_or_insert_with(|| format!("--{mode} given twice"));
                } else {
                    modes.push(mode);
                }
                continue;
            }
            Some("--profile") if profile.is_none() => &mut profile,
            Some("--entry") if entry.is_none() => &mut entry,
            Some("--prefix") if prefix.is_none() => &mut prefix,
            _ => {
                let word = word.to_string_lossy();
                problem.get_or_insert_with(|| {
                    let known = ["--json", "--yes"]
                        .iter()
                        .chain(&PACKAGE_WORDS)
                        .any(|known| *known == word);
                    if known && options.iter().filter(|other| **other == *word).count() > 1 {
                        format!("{word} given twice")
                    } else {
                        format!("unexpected argument {word}")
                    }
                });
                continue;
            }
        };
        match words.next() {
            Some(argument) => *value = Some(argument.clone()),
            None => {
                problem.get_or_insert_with(|| format!("{} needs a value", word.to_string_lossy()));
            }
        }
    }
    let operation = match modes.as_slice() {
        [mode] => Some(*mode),
        _ => None,
    };
    let selection = selection(&modes, yes, profile, entry, prefix, problem);
    Request {
        json,
        yes,
        operation,
        selection,
    }
}

fn selection(
    modes: &[&str],
    yes: bool,
    profile: Option<OsString>,
    entry: Option<OsString>,
    prefix: Option<OsString>,
    problem: Option<String>,
) -> Result<Option<Selector>, String> {
    if let Some(problem) = problem {
        return Err(problem);
    }
    match modes {
        [] => return Err("--profile, --entry and --prefix need --check or --install".into()),
        ["check" | "install"] => {}
        _ => return Err("--check and --install exclude each other".into()),
    }
    if yes && modes == ["check"] {
        return Err("--yes applies only to --install".into());
    }
    if prefix.is_some() {
        return Err(if profile.is_some() || entry.is_some() {
            "--prefix excludes --profile and --entry".into()
        } else {
            "eon update --prefix is not available in this build".into()
        });
    }
    match (profile, entry) {
        (None, None) => Ok(None),
        (Some(profile), Some(entry)) => {
            let profile = PathBuf::from(profile);
            if !profile.is_absolute() {
                return Err("--profile needs an absolute path".into());
            }
            match entry.into_string() {
                Ok(entry) if !entry.is_empty() => Ok(Some(Selector { profile, entry })),
                _ => Err("--entry needs a profile entry name".into()),
            }
        }
        _ => Err("--profile and --entry go together".into()),
    }
}

/// One build's identity; unknown fields are null.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Build {
    pub(crate) version: Option<String>,
    pub(crate) build: Option<String>,
    pub(crate) revision: Option<String>,
    pub(crate) platform: Option<String>,
}

/// Where discovery looks, and nowhere else.
struct Places {
    /// The directory holding this process's executable, symlinks resolved.
    invoked: Option<PathBuf>,
    /// The path this `eon` was started as, unresolved: its symlink chain
    /// names the installation that launched it. A store output alone does
    /// not, since `nix run` and a profile can run the same one.
    launcher: Option<PathBuf>,
    /// HOME is unset or relative, so the default profile is unknown.
    homeless: bool,
    /// The user's default profile, as Nix names it with and without XDG
    /// directories; the same profile under both names counts once.
    user_profiles: Vec<PathBuf>,
    /// Profiles a declarative manager owns, with that manager's name.
    declarative: Vec<(PathBuf, &'static str)>,
}

impl Places {
    fn of_this_process(program: &OsStr) -> Self {
        let absolute = |name| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let home = absolute("HOME");
        let state = absolute("XDG_STATE_HOME")
            .or_else(|| home.as_ref().map(|home| home.join(".local/state")));
        let mut declarative = Vec::new();
        if let Some(user) =
            env::var_os("USER").filter(|user| !user.is_empty() && !user.as_bytes().contains(&b'/'))
        {
            declarative.push((
                Path::new("/etc/profiles/per-user").join(user),
                "your NixOS, nix-darwin or Home Manager configuration (per-user packages)",
            ));
        }
        declarative.push((
            "/run/current-system/sw".into(),
            "your NixOS or nix-darwin configuration (system packages)",
        ));
        // A shell runs a command it found on PATH as that directory's path,
        // and the wrapper keeps it as the program name.
        let program = Path::new(program);
        let launcher = if program.is_absolute() {
            Some(program.to_path_buf())
        } else if program.components().count() > 1 {
            env::current_dir()
                .ok()
                .map(|directory| directory.join(program))
        } else {
            None
        };
        Self {
            invoked: env::current_exe()
                .and_then(fs::canonicalize)
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf)),
            launcher,
            homeless: home.is_none(),
            user_profiles: home
                .map(|home| home.join(".nix-profile"))
                .into_iter()
                .chain(state.map(|state| state.join("nix/profile")))
                .collect(),
            declarative,
        }
    }
}

/// A backend failure: its availability reason and message.
type Trouble = (&'static str, String);

/// The package manager and the network, which tests replace.
trait Host {
    /// `nix profile list --json` for `profile`.
    fn profile_list(&self, profile: &Path) -> Result<Vec<u8>, Trouble>;
    /// One HTTPS GET: the status and body, or why no response arrived.
    fn get(&self, url: &str, accept: &str) -> Result<(u16, Vec<u8>), Trouble>;
}

struct System {
    version: &'static str,
    /// The composition's pinned curl, `EON_CURL`.
    curl: Option<PathBuf>,
}

impl Host for System {
    fn profile_list(&self, profile: &Path) -> Result<Vec<u8>, Trouble> {
        // The person's own Nix owns the profile, so the check reads it with
        // that Nix rather than a pinned one.
        let mut command = Command::new("nix");
        command
            .args(["profile", "list", "--json", "--profile"])
            .arg(profile);
        let output = bounded(command, NIX_LIMIT).map_err(|error| {
            error.trouble(
                ["nix-unavailable", "nix-failed", "nix-failed"],
                "nix profile list",
            )
        })?;
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        let diagnostics = diagnostics.trim();
        if output.status.success() {
            if !diagnostics.is_empty() {
                eprintln!("{diagnostics}");
            }
            Ok(output.stdout)
        } else {
            Err((
                "nix-failed",
                format!("nix profile list failed: {diagnostics}"),
            ))
        }
    }

    fn get(&self, url: &str, accept: &str) -> Result<(u16, Vec<u8>), Trouble> {
        let Some(curl) = &self.curl else {
            return Err((
                "curl-unavailable",
                "this eon has no pinned curl (EON_CURL); run the eon of an installed package"
                    .into(),
            ));
        };
        // The installer's discipline: no curlrc, HTTPS only, no redirects,
        // bounded time and size (a fixed safety budget).
        let mut command = Command::new(curl);
        command.args([
            "-q",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--max-filesize",
            "16777216",
            "--header",
            &format!("Accept: {accept}"),
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
            "--user-agent",
            &format!("eon/{}", self.version),
            "--write-out",
            "\n%{http_code}",
            url,
        ]);
        let output = bounded(command, CURL_LIMIT).map_err(|error| {
            error.trouble(["curl-unavailable", "offline", "curl-failed"], "curl")
        })?;
        let diagnostics = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        match output.status.code() {
            Some(0) => {}
            // Resolve, connect, timeout, TLS and connection failures: the network.
            Some(5 | 6 | 7 | 28 | 35 | 52 | 55 | 56) => {
                return Err(("offline", format!("cannot reach GitHub: {diagnostics}")));
            }
            _ => return Err(("curl-failed", format!("curl failed: {diagnostics}"))),
        }
        let split = output
            .stdout
            .iter()
            .rposition(|byte| *byte == b'\n')
            .unwrap_or(0);
        let status = std::str::from_utf8(&output.stdout[split..])
            .ok()
            .and_then(|code| code.trim().parse().ok())
            .ok_or(("curl-failed", "curl reported no HTTP status".to_owned()))?;
        Ok((status, output.stdout[..split].to_vec()))
    }
}

enum Bounded {
    Spawn(std::io::Error),
    Wait(std::io::Error),
    TimedOut(Duration),
    Interrupted,
}

impl Bounded {
    /// The failure's reason: `missing` when the program cannot start,
    /// `timed_out` when it overran, `failed` when it could not be waited for.
    fn trouble(self, [missing, timed_out, failed]: [&'static str; 3], program: &str) -> Trouble {
        match self {
            Bounded::Interrupted => ("interrupted", format!("{program} interrupted")),
            Bounded::Spawn(error) => (missing, format!("cannot run {program}: {error}")),
            Bounded::Wait(error) => (failed, format!("cannot wait for {program}: {error}")),
            Bounded::TimedOut(limit) => (
                timed_out,
                format!("{program} did not finish within {} s", limit.as_secs()),
            ),
        }
    }
}

/// Runs `command` without input, collecting its output, and kills it once
/// `limit` passes.
fn bounded(mut command: Command, limit: Duration) -> Result<Output, Bounded> {
    let mut child = owned_child(&mut command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Bounded::Spawn)?;
    let collect = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = collect(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = collect(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let status = wait_child(&mut child, Some(limit))?;
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn owned_child(command: &mut Command) -> &mut Command {
    command.process_group(0);
    #[cfg(target_os = "linux")]
    unsafe {
        let parent = libc::getpid();
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Cover parent death between fork and installing the death signal.
            if libc::getppid() != parent {
                libc::raise(libc::SIGKILL);
            }
            Ok(())
        });
    }
    command
}

fn wait_child(child: &mut Child, limit: Option<Duration>) -> Result<ExitStatus, Bounded> {
    let started = Instant::now();
    loop {
        let result = child.try_wait();
        let failure = if INTERRUPTED.load(Ordering::Relaxed) {
            Some(Bounded::Interrupted)
        } else if limit.is_some_and(|limit| started.elapsed() >= limit) {
            Some(Bounded::TimedOut(limit.unwrap()))
        } else if let Err(error) = result.as_ref() {
            Some(Bounded::Wait(std::io::Error::other(error.to_string())))
        } else {
            None
        };
        if let Some(failure) = failure {
            // The child is a new process-group leader; never signal the shared Nix daemon.
            let group = -(child.id() as i32);
            unsafe {
                libc::kill(group, libc::SIGTERM);
            }
            let deadline = Instant::now() + TERMINATION_GRACE;
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            unsafe {
                libc::kill(group, libc::SIGKILL);
            }
            let _ = child.wait();
            return Err(failure);
        }
        if let Ok(Some(status)) = result {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

struct Signals(Vec<(libc::c_int, libc::sigaction)>);

impl Signals {
    fn new() -> std::io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Relaxed);
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut previous = action;
            action.sa_sigaction = interrupt as *const () as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            if unsafe { libc::sigaction(signal, &action, &mut previous) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            guard.0.push((signal, previous));
        }
        Ok(guard)
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, previous) in &self.0 {
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}

/// A `nix profile list --json` element.
#[derive(Clone, Debug, PartialEq)]
struct Element {
    record: Value,
    name: String,
    attr_path: Option<String>,
    original_url: Option<String>,
    url: Option<String>,
    store_paths: Vec<PathBuf>,
}

fn elements(listing: &[u8]) -> Result<Vec<Element>, Trouble> {
    let invalid = |message: String| ("nix-output-invalid", message);
    let listing: Value = serde_json::from_slice(listing)
        .map_err(|error| invalid(format!("nix profile list printed no JSON: {error}")))?;
    if listing["version"] != 3 {
        return Err(invalid(format!(
            "unsupported nix profile list format {}",
            listing["version"]
        )));
    }
    let Some(elements) = listing["elements"].as_object() else {
        return Err(invalid("nix profile list printed no elements".into()));
    };
    let text = |value: &Value| value.as_str().map(str::to_owned);
    Ok(elements
        .iter()
        .map(|(name, element)| Element {
            record: element.clone(),
            name: name.clone(),
            attr_path: text(&element["attrPath"]),
            original_url: text(&element["originalUrl"]),
            url: text(&element["url"]),
            store_paths: element["storePaths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect(),
        })
        .collect())
}

/// Who owns the installation a check addresses.
enum Owner {
    Entry {
        profile: PathBuf,
        element: Element,
        others: Vec<Element>,
    },
    Declarative(&'static str),
    Ambiguous(String),
    Unmanaged,
    /// The explicit target does not exist.
    Missing(String),
    /// The package manager could not be read.
    Unreadable(Trouble),
}

const HOME_MANAGER: &str = "Home Manager (its home-manager-path profile entry)";

fn owner(selector: Option<&Selector>, places: &Places, host: &dyn Host) -> Owner {
    if let Some(Selector { profile, entry }) = selector {
        if let Some((_, manager)) = places.declarative.iter().find(|(managed, _)| {
            profile == managed
                || profile_location(profile)
                    .ok()
                    .zip(profile_location(managed).ok())
                    .is_some_and(|(profile, managed)| profile == managed)
        }) {
            return Owner::Declarative(manager);
        }
        let is_profile = fs::canonicalize(profile)
            .is_ok_and(|generation| generation.join("manifest.json").is_file());
        if !is_profile {
            return Owner::Missing(format!("{} is not a Nix profile", profile.display()));
        }
        return match host
            .profile_list(profile)
            .and_then(|listing| elements(&listing))
        {
            Err(trouble) => Owner::Unreadable(trouble),
            Ok(mut elements) => match elements.iter().position(|element| &element.name == entry) {
                Some(index) => Owner::Entry {
                    profile: profile.clone(),
                    element: elements.remove(index),
                    others: elements,
                },
                None => Owner::Missing(format!("{} has no entry {entry}", profile.display())),
            },
        };
    }
    let Some(invoked) = &places.invoked else {
        return Owner::Ambiguous("cannot resolve this eon's own executable".into());
    };
    let Some(launcher) = places
        .launcher
        .as_ref()
        .filter(|launcher| resolves_to(launcher, invoked))
    else {
        return Owner::Ambiguous("cannot tell which path started this eon".into());
    };
    let mut managers: Vec<&str> = places
        .declarative
        .iter()
        .filter(|(profile, _)| through(launcher, profile))
        .map(|(_, manager)| *manager)
        .collect();
    let mut entries = Vec::new();
    let mut listed = None;
    let mut seen = Vec::new();
    for profile in &places.user_profiles {
        let Ok(canonical) = fs::canonicalize(profile) else {
            continue;
        };
        if seen.contains(&canonical) || !through(launcher, profile) {
            continue;
        }
        seen.push(canonical);
        let elements = match host
            .profile_list(profile)
            .and_then(|listing| elements(&listing))
        {
            Ok(elements) => elements,
            Err(trouble) => return Owner::Unreadable(trouble),
        };
        listed.get_or_insert_with(|| profile.clone());
        for element in &elements {
            let installs = element
                .store_paths
                .iter()
                .any(|path| resolves_to(&path.join("bin/eon"), invoked));
            if installs && element.name == "home-manager-path" {
                managers.push(HOME_MANAGER);
            } else if installs {
                entries.push((
                    profile.clone(),
                    element.clone(),
                    elements
                        .iter()
                        .filter(|other| other.name != element.name)
                        .cloned()
                        .collect(),
                ));
            }
        }
    }
    match (entries.len(), managers.as_slice()) {
        (1, []) => {
            let (profile, element, others) = entries.remove(0);
            Owner::Entry {
                profile,
                element,
                others,
            }
        }
        (0, [manager]) => Owner::Declarative(manager),
        (0, []) => {
            if let Some(profile) = listed {
                Owner::Ambiguous(format!(
                    "{} started this eon, but none of its entries installs it",
                    profile.display()
                ))
            } else if launcher.ends_with("libexec/eon/bin/eon") {
                Owner::Ambiguous(
                    "this eon is its Eon window's own build, which no installation launches \
                     directly"
                        .into(),
                )
            } else if places.homeless {
                Owner::Ambiguous("HOME is not set, so the default Nix profile is unknown".into())
            } else {
                Owner::Unmanaged
            }
        }
        _ => Owner::Ambiguous(format!(
            "several installations own this eon: {}",
            entries
                .iter()
                .map(|(profile, element, _)| format!(
                    "{} entry {}",
                    profile.display(),
                    element.name
                ))
                .chain(managers.iter().map(|manager| manager.to_string()))
                .collect::<Vec<_>>()
                .join("; ")
        )),
    }
}

/// Whether `launcher` resolves to an executable in `directory`.
fn resolves_to(launcher: &Path, directory: &Path) -> bool {
    fs::canonicalize(launcher).is_ok_and(|path| path.parent() == Some(directory))
}

/// Whether `launcher` runs through `profile`: one of its leading directories
/// is that profile's selected generation.
fn through(launcher: &Path, profile: &Path) -> bool {
    let Ok(generation) = fs::canonicalize(profile) else {
        return false;
    };
    // A launcher can be a symlink to the profile's, as `~/.local/bin/eon`
    // to `~/.nix-profile/bin/eon`, so each hop of its chain counts.
    let mut hop = launcher.to_path_buf();
    for _ in 0..32 {
        let runs_through = hop
            .ancestors()
            .skip(1)
            .any(|directory| fs::canonicalize(directory).is_ok_and(|path| path == generation));
        if runs_through {
            return true;
        }
        let (Ok(target), Some(directory)) = (fs::read_link(&hop), hop.parent()) else {
            return false;
        };
        hop = directory.join(target);
    }
    false
}

/// What the profile entry records about its source.
#[derive(Clone, Debug, PartialEq)]
struct Source {
    repository: Option<String>,
    policy: &'static str,
    original_ref: Option<String>,
    locked_ref: Option<String>,
    attribute: Option<String>,
    revision: Option<String>,
}

fn official(owner: &str, repo: &str) -> bool {
    owner.eq_ignore_ascii_case("Yazelix") && repo.eq_ignore_ascii_case("eon")
}

fn source(element: &Element) -> Source {
    let original = element.original_url.as_deref().unwrap_or_default();
    let (repository, policy) = match original.strip_prefix("github:") {
        Some(reference) => {
            // A query (`?ref=`, `?rev=`) is another spelling of the source, which this check
            // does not trust.
            let (path, query) = reference.split_once('?').unwrap_or((reference, ""));
            let parts: Vec<&str> = path.split('/').collect();
            let (owner, repo, rest) = match parts.as_slice() {
                [owner, repo, rest @ ..] => (*owner, *repo, rest),
                _ => ("", "", &[][..]),
            };
            let official = official(owner, repo);
            let policy = match rest {
                _ if !official || !query.is_empty() => "unknown",
                ["main"] => "main",
                ["edge"] => "edge",
                [tag] if is_release_tag(tag) => "tag",
                [commit] if is_revision(commit) => "commit",
                _ => "unknown",
            };
            let repository = if official {
                Some(REPOSITORY.to_owned())
            } else {
                (!owner.is_empty()).then(|| format!("https://github.com/{owner}/{repo}"))
            };
            (repository, policy)
        }
        None if original.starts_with("path:") || original.starts_with("git+file:") => {
            (None, "local")
        }
        None => (None, "unknown"),
    };
    Source {
        repository,
        policy,
        original_ref: element.original_url.clone(),
        locked_ref: element.url.clone(),
        attribute: element.attr_path.clone(),
        revision: element.url.as_deref().and_then(locked_revision),
    }
}

/// The commit an official locked GitHub reference names,
/// `github:Yazelix/eon/REVISION?…`.
fn locked_revision(url: &str) -> Option<String> {
    let path = url.strip_prefix("github:")?.split('?').next()?;
    let mut parts = path.split('/');
    let (owner, repo, revision) = (parts.next()?, parts.next()?, parts.next()?);
    (official(owner, repo) && parts.next().is_none() && is_revision(revision))
        .then(|| revision.to_owned())
}

fn is_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_release_tag(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|version| {
        let parts: Vec<_> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

/// The composition's source receipt (EON-C41 Provenance), schema 1.
#[derive(Debug, PartialEq)]
struct Receipt {
    revision: Option<String>,
    build: String,
    platform: String,
    attribute: Option<String>,
}

fn receipt(package: &Path) -> Option<Receipt> {
    let mut bytes = Vec::new();
    fs::File::open(package.join(RECEIPT))
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    let optional = |name: &str| match object.get(name)? {
        Value::Null => Some(None),
        Value::String(text) => Some(Some(text.clone())),
        _ => None,
    };
    let receipt = Receipt {
        revision: optional("revision")?,
        build: optional("build")??,
        platform: optional("platform")??,
        attribute: optional("attribute")?,
    };
    (object.len() == 6
        && object.get("schema_version") == Some(&Value::from(1))
        && object.get("repository").and_then(Value::as_str) == Some(REPOSITORY)
        && receipt.revision.as_deref().is_none_or(is_revision)
        && is_build(&receipt.build))
    .then_some(receipt)
}

fn is_build(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Debug, PartialEq)]
struct Availability {
    state: &'static str,
    candidate: Option<String>,
    reason: Option<&'static str>,
}

#[derive(Debug, PartialEq)]
struct Capability {
    install: bool,
    reason: Option<&'static str>,
    next_step: Option<String>,
}

#[derive(Debug, PartialEq)]
struct Failure {
    code: &'static str,
    stage: &'static str,
    message: String,
    next_step: Option<String>,
}

#[derive(Debug, PartialEq)]
struct Report {
    operation: Option<&'static str>,
    outcome: &'static str,
    install_kind: &'static str,
    /// The owner in words, for human output.
    owner: Option<String>,
    target: Option<Selector>,
    launcher: Option<PathBuf>,
    source: Option<Source>,
    /// Whether the installed build is tied to its locked source revision.
    proved: bool,
    invoked: Option<Build>,
    installed: Option<Build>,
    checked_at: Option<String>,
    availability: Availability,
    capability: Capability,
    selection: Option<Selection>,
    snapshot: Option<Snapshot>,
    error: Option<Failure>,
}

impl Report {
    fn invalid(operation: Option<&'static str>, message: String) -> Self {
        Self {
            operation,
            outcome: "failed",
            install_kind: "unknown",
            owner: None,
            target: None,
            launcher: None,
            source: None,
            proved: false,
            invoked: None,
            installed: None,
            checked_at: None,
            availability: unknown(None),
            capability: undecided(None),
            selection: (operation == Some("install")).then_some(Selection {
                state: "unknown",
                selected: None,
                source: None,
            }),
            snapshot: None,
            error: Some(Failure {
                code: "invalid-arguments",
                stage: "resolve",
                message,
                next_step: Some(
                    "eon update --check [--json] [--profile /ABS/PROFILE --entry ENTRY]".into(),
                ),
            }),
        }
    }

    fn status(&self) -> u8 {
        if self.outcome == "refused" {
            return 4;
        }
        match self.error.as_ref().map(|error| error.code) {
            None => 0,
            Some("invalid-arguments") => 2,
            Some("unknown-target") => 3,
            Some("interrupted") => 130,
            Some(_) => 1,
        }
    }

    fn json(&self) -> Value {
        let build = |build: &Option<Build>| {
            build.as_ref().map(|build| {
                json!({
                    "version": build.version,
                    "build": build.build,
                    "revision": build.revision,
                    "platform": build.platform,
                })
            })
        };
        json!({
            "schema_version": 1,
            "operation": self.operation,
            "outcome": self.outcome,
            "install_kind": self.install_kind,
            "target": self.target.as_ref().map(|target| json!({
                "profile": json_path(&target.profile),
                "entry": target.entry,
            })),
            "launcher": self.launcher.as_deref().map(json_path),
            "source": self.source.as_ref().map(|source| json!({
                "repository": source.repository,
                "policy": source.policy,
                "original_ref": source.original_ref,
                "locked_ref": source.locked_ref,
                "attribute": source.attribute,
                "revision": source.revision,
                "archive_url": null,
                "archive_sha256": null,
            })),
            "invoked": build(&self.invoked),
            "installed": build(&self.installed),
            "running": null,
            "checked_at": self.checked_at,
            "availability": {
                "state": self.availability.state,
                "candidate": self.availability.candidate.as_ref().map(|revision| json!({
                    "version": null,
                    "build": null,
                    "revision": revision,
                    "archive_url": null,
                })),
                "reason": self.availability.reason,
            },
            "capability": {
                "install": self.capability.install,
                "reason": self.capability.reason,
                "next_step": self.capability.next_step,
            },
            "selection": self.selection.as_ref().map(|selection| json!({
                "state": selection.state,
                "selected": build(&selection.selected),
                "source": selection.source.as_ref().map(Source::json),
            })),
            "error": self.error.as_ref().map(|error| json!({
                "code": error.code,
                "stage": error.stage,
                "message": error.message,
                "next_step": error.next_step,
            })),
        })
    }

    fn human(&self) -> String {
        if self.operation.is_none()
            || self
                .error
                .as_ref()
                .is_some_and(|error| error.stage == "resolve" && self.owner.is_none())
        {
            return String::new();
        }
        let build = |build: &Build| {
            let mut words = vec![
                build
                    .version
                    .clone()
                    .map_or("eon".into(), |version| format!("eon {version}")),
            ];
            words.extend(build.build.clone());
            words.extend(
                build
                    .revision
                    .as_ref()
                    .map(|revision| format!("from {revision}")),
            );
            if words.len() == 1 {
                words.push("(unknown build)".into());
            }
            words.join(" ")
        };
        let mut lines = vec![format!(
            "Installation: {}",
            self.owner.as_deref().unwrap_or("unknown")
        )];
        if let Some(launcher) = &self.launcher {
            lines.push(format!("Launcher: {}", launcher.display()));
        }
        if let Some(source) = &self.source {
            lines.push(format!(
                "Source: {} ({}), locked at {}",
                source.policy,
                source
                    .original_ref
                    .as_deref()
                    .unwrap_or("no flake reference"),
                source.revision.as_deref().unwrap_or("an unknown revision")
            ));
            lines.push(if self.proved {
                "Provenance: proved, the installed build's receipt names that revision".into()
            } else {
                "Provenance: unknown, no source receipt ties the installed build to that revision"
                    .into()
            });
        }
        lines.extend(
            self.invoked
                .as_ref()
                .map(|invoked| format!("This eon: {}", build(invoked))),
        );
        lines.extend(
            self.installed
                .as_ref()
                .map(|installed| format!("Installed: {}", build(installed))),
        );
        let candidate = self
            .availability
            .candidate
            .as_deref()
            .map_or(String::new(), |revision| {
                format!(", the source is at {revision}")
            });
        lines.push(match (self.availability.state, self.availability.reason) {
            ("current", _) => {
                "Update: none, the installed build is the source's current revision".into()
            }
            ("available", _) => format!("Update: available{candidate}"),
            ("different", reason) => format!(
                "Update: the source moved to another revision ({}){candidate}",
                reason.unwrap_or("no ordering")
            ),
            (_, reason) => format!(
                "Update: unknown ({}){candidate}",
                reason.unwrap_or("not checked")
            ),
        });
        let next = self
            .capability
            .next_step
            .as_deref()
            .map_or(String::new(), |step| format!("; {step}"));
        lines.push(match self.capability.reason {
            None if self.capability.install => format!("Install: Eon may update this entry{next}"),
            None => format!("Install: undecided{next}"),
            Some(reason) => format!("Install: refused, {reason}{next}"),
        });
        if self.operation == Some("install") {
            lines.push(format!("Result: {}", self.outcome));
            if let Some(selection) = &self.selection {
                lines.push(format!(
                    "Selection: {}{}",
                    selection.state,
                    selection
                        .selected
                        .as_ref()
                        .map(|selected| format!(", {}", build(selected)))
                        .unwrap_or_default()
                ));
                if let Some(source) = &selection.source {
                    lines.push(format!(
                        "Selected source: {}",
                        source.locked_ref.as_deref().unwrap_or("unknown")
                    ));
                }
            }
            lines.push("Running windows keep their build and terminals. Use the reported launcher for fresh launches.".into());
        }
        lines.join("\n") + "\n"
    }
}

impl Report {
    fn fail(&mut self, code: &'static str, stage: &'static str, message: String) {
        self.outcome = "failed";
        self.error = Some(Failure {
            code,
            stage,
            message,
            next_step: self.capability.next_step.clone(),
        });
    }

    fn refuse(&mut self, reason: &'static str, stage: &'static str, message: String) {
        self.capability.install = false;
        self.capability.reason = Some(reason);
        self.fail(reason, stage, message);
        self.outcome = "refused";
    }
}

#[derive(Debug, PartialEq)]
struct Selection {
    state: &'static str,
    selected: Option<Build>,
    source: Option<Source>,
}

impl Source {
    fn json(&self) -> Value {
        json!({"repository": self.repository, "policy": self.policy, "original_ref": self.original_ref,
            "locked_ref": self.locked_ref, "attribute": self.attribute, "revision": self.revision,
            "archive_url": null, "archive_sha256": null})
    }
}

fn unknown(reason: Option<&'static str>) -> Availability {
    Availability {
        state: "unknown",
        candidate: None,
        reason,
    }
}

/// Neither eligible nor refused, as when the check could not finish.
fn undecided(next_step: Option<String>) -> Capability {
    Capability {
        install: false,
        reason: None,
        next_step,
    }
}

fn json_path(path: &Path) -> Value {
    match path.to_str() {
        Some(path) => path.into(),
        None => path
            .as_os_str()
            .as_bytes()
            .iter()
            .map(|byte| Value::from(*byte))
            .collect(),
    }
}

/// Quotes `word` for a POSIX shell when it needs it.
fn shell_word(word: &str) -> String {
    if !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+:@=".contains(&byte))
    {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn check(
    selector: Option<&Selector>,
    invoked: Build,
    places: &Places,
    host: &dyn Host,
    now: SystemTime,
) -> Report {
    check_remote(inspect(selector, invoked, places, host, now), host)
}

fn check_remote(mut report: Report, host: &dyn Host) -> Report {
    let Some(source) = report.source.clone() else {
        return report;
    };
    let proved = report.proved;
    let follows = matches!(source.policy, "main" | "edge")
        && source.attribute
            == report
                .invoked
                .as_ref()
                .and_then(|build| build.platform.as_ref())
                .map(|platform| format!("packages.{platform}.default"));
    report.availability = if !follows {
        unknown(Some(match source.policy {
            "tag" | "commit" => "pinned",
            "local" => "local-source",
            _ => "source-unverified",
        }))
    } else if let Some(installed) = source.revision.clone() {
        match remote(host, source.policy, &installed, proved) {
            Ok(availability) => availability,
            Err((reason, message)) => {
                report.outcome = "failed";
                report.error = Some(Failure {
                    code: "remote-unavailable",
                    stage: "check",
                    message,
                    next_step: Some("Check again later; nothing changed.".into()),
                });
                // Eligibility waits for an answer.
                if report.capability.install {
                    report.capability = undecided(Some("Check again once GitHub answers.".into()));
                }
                unknown(Some(reason))
            }
        }
    } else {
        unknown(Some("locked-revision-unknown"))
    };
    report
}

fn inspect(
    selector: Option<&Selector>,
    invoked: Build,
    places: &Places,
    host: &dyn Host,
    now: SystemTime,
) -> Report {
    let mut report = Report {
        operation: Some("check"),
        outcome: "checked",
        install_kind: "unknown",
        owner: None,
        target: selector.cloned(),
        launcher: None,
        source: None,
        proved: false,
        invoked: Some(invoked.clone()),
        installed: None,
        checked_at: Some(rfc3339(now)),
        availability: unknown(None),
        capability: undecided(None),
        selection: None,
        snapshot: None,
        error: None,
    };
    let refuse = |report: &mut Report, reason, next_step: String| {
        report.availability = unknown(Some(reason));
        report.capability = Capability {
            install: false,
            reason: Some(reason),
            next_step: Some(next_step),
        };
    };
    let name_it = "Name the installation with --profile /ABS/PROFILE --entry ENTRY";
    let (profile, element, others) = match owner(selector, places, host) {
        Owner::Missing(message) => {
            report.outcome = "failed";
            report.error = Some(Failure {
                code: "unknown-target",
                stage: "resolve",
                message,
                next_step: Some("List entries with nix profile list --profile /ABS/PROFILE".into()),
            });
            return report;
        }
        Owner::Unreadable((reason, message)) => {
            report.outcome = "failed";
            report.availability = unknown(Some(reason));
            report.error = Some(Failure {
                code: "selection-failed",
                stage: "resolve",
                message,
                next_step: Some(
                    "Check that nix profile list --json --profile PROFILE works".into(),
                ),
            });
            return report;
        }
        Owner::Ambiguous(why) => {
            report.owner = Some(format!("ambiguous: {why}"));
            refuse(&mut report, "install-ambiguous", format!("{name_it}."));
            return report;
        }
        Owner::Unmanaged => {
            report.install_kind = "unmanaged";
            report.owner = Some("none Eon can identify".into());
            refuse(
                &mut report,
                "install-unmanaged",
                format!(
                    "This eon is a local build, a nix run or nix shell, or a manual extraction: \
                     update it the way you installed it. {name_it} to check a profile install."
                ),
            );
            return report;
        }
        Owner::Declarative(manager) => {
            report.install_kind = "externally-managed";
            report.owner = Some(format!("managed by {manager}"));
            refuse(&mut report, "externally-managed", externally_managed());
            return report;
        }
        Owner::Entry {
            profile,
            element,
            others,
        } => (profile, element, others),
    };
    report.target = Some(Selector {
        profile: profile.clone(),
        entry: element.name.clone(),
    });
    report.snapshot = profile_location(&profile).ok().and_then(|location| {
        let generation = fs::canonicalize(&profile).ok()?;
        Some(Snapshot {
            link: fs::read_link(&location).ok()?,
            location,
            generation,
            element: element.clone(),
            others,
        })
    });
    let package = element
        .store_paths
        .iter()
        .find(|path| path.join("bin/eon").exists())
        .cloned();
    report.launcher = package
        .as_ref()
        .and_then(|package| fs::canonicalize(package.join("bin/eon")).ok())
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
        .filter(|directory| resolves_to(&profile.join("bin/eon"), directory))
        .map(|_| profile.join("bin/eon"));
    let invoked_here = package.as_ref().is_some_and(|package| {
        places
            .invoked
            .as_deref()
            .is_some_and(|directory| resolves_to(&package.join("bin/eon"), directory))
    });
    if element.original_url.is_none() {
        // A store-path entry has no source to follow.
        if element.name == "home-manager-path" {
            report.install_kind = "externally-managed";
            report.owner = Some(format!("managed by {HOME_MANAGER}"));
            refuse(&mut report, "externally-managed", externally_managed());
        } else {
            report.install_kind = "unmanaged";
            report.owner = Some(format!(
                "store-path entry {} in {}",
                element.name,
                profile.display()
            ));
            refuse(
                &mut report,
                "install-unmanaged",
                "This entry names a store path, not a source: install a source with nix profile \
                 add."
                    .into(),
            );
        }
        return report;
    }
    report.install_kind = "nix-profile";
    report.owner = Some(format!(
        "Nix profile entry {} in {}",
        element.name,
        profile.display()
    ));
    let source = source(&element);
    let platform = invoked.platform.clone();
    let attribute = platform
        .as_ref()
        .map(|platform| format!("packages.{platform}.default"));
    let receipt = package.as_deref().and_then(receipt);
    report.proved = receipt.as_ref().is_some_and(|receipt| {
        receipt.revision.is_some()
            && receipt.revision == source.revision
            && receipt.attribute == source.attribute
            && Some(&receipt.platform) == platform.as_ref()
            && (!invoked_here || invoked.build.as_ref() == Some(&receipt.build))
    });
    let proved = report.proved;
    report.installed = Some(Build {
        version: invoked_here.then(|| invoked.version.clone()).flatten(),
        build: receipt
            .as_ref()
            .filter(|_| proved)
            .map(|receipt| receipt.build.clone()),
        revision: source.revision.clone().filter(|_| proved),
        platform: proved.then_some(platform).flatten(),
    });
    if invoked_here && proved {
        report.invoked.as_mut().unwrap().revision = source.revision.clone();
    }
    let profile_word = shell_word(&profile.to_string_lossy());
    let entry_word = shell_word(&element.name);
    let upgrade = format!("nix profile upgrade --profile {profile_word} --refresh {entry_word}");
    let follows = matches!(source.policy, "main" | "edge") && source.attribute == attribute;
    report.capability = if matches!(source.policy, "tag" | "commit") {
        Capability {
            install: false,
            reason: Some("pinned-install"),
            next_step: Some(format!(
                "This entry stays pinned to {}. To follow the main branch, which tracks \
                 releases, replace it: nix profile remove --profile {profile_word} {entry_word}, \
                 then nix profile add --profile {profile_word} --refresh github:Yazelix/eon/main",
                source.original_ref.as_deref().unwrap_or_default()
            )),
        }
    } else if !follows {
        Capability {
            install: false,
            reason: Some("source-unverified"),
            next_step: Some(format!(
                "Eon updates only entries of github:Yazelix/eon/main or edge with their default \
                 package; update this one yourself with {upgrade}"
            )),
        }
    } else if !proved {
        Capability {
            install: false,
            reason: Some("provenance-invalid"),
            next_step: Some(format!(
                "This build carries no source receipt for its locked revision (builds before \
                 eon update --check have none); update it once with {upgrade}"
            )),
        }
    } else if element.record["active"] != true || report.launcher.is_none() {
        Capability {
            install: false,
            reason: Some("install-ambiguous"),
            next_step: Some(
                "Choose an active entry that supplies this profile's selected eon launcher.".into(),
            ),
        }
    } else if !writable(&profile) {
        Capability {
            install: false,
            reason: Some("target-unwritable"),
            next_step: Some(format!(
                "You cannot change {}; update it as the user who owns it",
                profile.display()
            )),
        }
    } else {
        Capability {
            install: true,
            reason: None,
            next_step: Some(format!(
                "run eon update --install --profile {profile_word} --entry {entry_word}"
            )),
        }
    };
    report.source = Some(source);
    report
}

fn externally_managed() -> String {
    "Update the Eon input of that configuration and activate it, for a flake: \
     nix flake update eon --flake /ABS/CONFIG, then its switch command"
        .into()
}

fn install(request: &Request, invoked: Build, places: &Places, system: &System) -> Report {
    let mut report = install_inner(request, invoked.clone(), places, system);
    let interrupted = INTERRUPTED.swap(false, Ordering::Relaxed);
    if report.selection.is_none() || interrupted {
        if let (Some(before), Some(target)) = (report.snapshot.clone(), report.target.clone()) {
            observe(&mut report, &before, &target, &invoked, places, system);
        } else {
            report.selection = Some(Selection {
                state: "unknown",
                selected: None,
                source: None,
            });
        }
    }
    if interrupted {
        let stage = report.error.as_ref().map_or("confirm", |error| error.stage);
        report.fail(
            "interrupted",
            stage,
            "Install interrupted; reporting the actual selection".into(),
        );
    }
    report
}

fn install_inner(request: &Request, invoked: Build, places: &Places, system: &System) -> Report {
    let explicit = request.selection.as_ref().unwrap();
    let mut report = inspect(
        explicit.as_ref(),
        invoked.clone(),
        places,
        system,
        SystemTime::now(),
    );
    report.operation = Some("install");
    let before = report.snapshot.clone();
    if report.error.is_some() {
        return report;
    }
    let interactive = !request.json && std::io::stdin().is_terminal();
    if explicit.is_none() && (!interactive || request.yes) {
        report.refuse(
            "install-ambiguous",
            "confirm",
            "--yes and noninteractive Install require an explicit or configured profile and entry"
                .into(),
        );
        return report;
    }
    if let Some(reason) = report.capability.reason {
        let message = report
            .capability
            .next_step
            .clone()
            .unwrap_or_else(|| reason.into());
        report.refuse(reason, "resolve", message);
        return report;
    }
    let Some(before) = before else {
        report.refuse(
            "install-ambiguous",
            "resolve",
            "cannot resolve the mutable profile selector".into(),
        );
        return report;
    };
    let target = report.target.clone().unwrap();
    if !exact_entry(&target.entry) {
        report.refuse(
            "install-ambiguous",
            "resolve",
            "Install cannot use an integer or absolute path as an exact Nix entry name".into(),
        );
        return report;
    }
    let mut lock_path = before.location.as_os_str().to_os_string();
    lock_path.push(".eon-update.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(PathBuf::from(lock_path));
    let _lock = match lock {
        Ok(lock) => {
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    report.refuse(
                        "operation-busy",
                        "resolve",
                        "another Eon update owns this profile".into(),
                    );
                } else {
                    report.fail(
                        "selection-failed",
                        "resolve",
                        format!("cannot lock profile: {error}"),
                    );
                }
                return report;
            }
            lock
        }
        Err(error) => {
            report.fail(
                "selection-failed",
                "resolve",
                format!("cannot open profile update lock: {error}"),
            );
            return report;
        }
    };
    report = check_remote(report, system);
    if report.error.is_some() {
        return report;
    }
    eprintln!(
        "eon update: install {} entry {:?} from {} ({})",
        target.profile.display(),
        target.entry,
        report
            .source
            .as_ref()
            .and_then(|source| source.original_ref.as_deref())
            .unwrap_or("unknown"),
        report
            .source
            .as_ref()
            .and_then(|source| source.attribute.as_deref())
            .unwrap_or("unknown")
    );
    if !request.yes {
        if !interactive {
            report.fail(
                "not-confirmed",
                "confirm",
                "Install requires --yes outside an interactive terminal".into(),
            );
            return report;
        }
        let terminal = match confirmation_terminal() {
            Ok(terminal) => terminal,
            Err(error) => {
                report.fail("not-confirmed", "confirm", format!("Install requires a controlling terminal: {error}; use --yes with an explicit or configured target"));
                return report;
            }
        };
        eprint!("Upgrade this entry for fresh launches? [y/N] ");
        let _ = std::io::stderr().flush();
        match confirmation(terminal.as_raw_fd()) {
            Err(error) => {
                report.fail(
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        "interrupted"
                    } else {
                        "selection-failed"
                    },
                    "confirm",
                    error.to_string(),
                );
                return report;
            }
            Ok(false) => {
                report.outcome = "cancelled";
                observe(&mut report, &before, &target, &invoked, places, system);
                return report;
            }
            Ok(true) => {}
        }
    }
    let mut ran_nix = false;
    let result = (|| {
        if snapshot(&target, system).as_ref() != Ok(&before) {
            return Err((
                "target-changed",
                "select",
                "profile identity, source, entry or selection changed since it was displayed"
                    .into(),
            ));
        }
        if INTERRUPTED.load(Ordering::Relaxed) {
            return Err(("interrupted", "confirm", "Install interrupted".into()));
        }
        if report.availability.state == "current" {
            return Ok(false);
        }
        let mut command = Command::new("nix");
        command.arg("--version");
        let version = bounded(command, NIX_LIMIT).map_err(|error| {
            let (code, message) = error.trouble(["selection-failed"; 3], "nix --version");
            (code, "resolve", message)
        })?;
        if !version.status.success() || !exact_names(&version.stdout) {
            return Err((
                "selection-failed",
                "resolve",
                "Install requires upstream or Determinate Nix 2.21 or later for exact entry names; Lix and other implementations are unsupported".into(),
            ));
        }
        // Version probing can take time: revalidate again immediately before mutation.
        if snapshot(&target, system).as_ref() != Ok(&before) {
            return Err((
                "target-changed",
                "select",
                "profile changed before Nix upgrade".into(),
            ));
        }
        eprintln!("eon update: fetching/building and selecting with Nix; Ctrl+C cancels");
        let mut command = Command::new("nix");
        let mut child = owned_child(&mut command)
            .args(["profile", "upgrade", "--profile"])
            .arg(&before.location)
            .args(["--refresh", "--"])
            .arg(&target.entry)
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::io::stderr()))
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| ("selection-failed", "fetch-build", error.to_string()))?;
        ran_nix = true;
        let status = wait_child(&mut child, None).map_err(|error| {
            let (code, message) = error.trouble(["selection-failed"; 3], "nix profile upgrade");
            (code, "fetch-build", message)
        })?;
        if !status.success() {
            return Err((
                "selection-failed",
                "fetch-build",
                format!("nix profile upgrade exited {status}"),
            ));
        }
        Ok(true)
    })();
    let interrupted = INTERRUPTED.swap(false, Ordering::Relaxed);
    let after = observe(&mut report, &before, &target, &invoked, places, system);
    match result {
        Err(("target-changed", stage, message)) => report.refuse("target-changed", stage, message),
        _ if ran_nix
            && after
                .as_ref()
                .is_some_and(|after| after.others != before.others) =>
        {
            report.fail(
                "selection-failed",
                "inspect-result",
                "another profile entry changed during Install; reporting the observed selection"
                    .into(),
            );
        }
        _ if ran_nix
            && after.as_ref().is_some_and(|after| {
                after.location != before.location
                    || after.element.original_url != before.element.original_url
                    || after.element.attr_path != before.element.attr_path
            }) =>
        {
            report.fail("source-changed", "inspect-result", "target identity or source changed during Nix upgrade; reporting the observed selection".into());
        }
        Err((code, stage, message)) => report.fail(code, stage, message),
        Ok(_)
            if report.selection.as_ref().is_none_or(|selection| {
                selection.state == "unknown"
                    || selection
                        .selected
                        .as_ref()
                        .and_then(|build| build.build.as_ref())
                        .is_none()
            }) || report.launcher.is_none() =>
        {
            report.fail(
                "selection-failed",
                "inspect-result",
                "cannot verify the actual selected build after Install".into(),
            );
        }
        Ok(_) => {
            report.outcome = if report.selection.as_ref().unwrap().state == "changed" {
                "installed"
            } else {
                "unchanged"
            }
        }
    }
    if interrupted {
        let stage = report
            .error
            .as_ref()
            .map_or("fetch-build", |error| error.stage);
        report.fail(
            "interrupted",
            stage,
            "Install interrupted; selection is the observed state, cancellation does not roll back"
                .into(),
        );
    }
    report
}

fn exact_names(version: &[u8]) -> bool {
    let text = String::from_utf8_lossy(version);
    if !text.starts_with("nix (Nix) ") && !text.starts_with("nix (Determinate Nix ") {
        return false;
    }
    let Some(version) = text.split_whitespace().last() else {
        return false;
    };
    let mut parts = version.split('.');
    let parsed = parts
        .next()
        .and_then(|major| major.parse::<u64>().ok())
        .zip(parts.next().and_then(|minor| minor.parse::<u64>().ok()));
    if parts
        .next()
        .and_then(|patch| patch.parse::<u64>().ok())
        .is_none()
        || parts.next().is_some()
    {
        return false;
    }
    parsed.is_some_and(|version| version >= (2, 21))
}

fn exact_entry(entry: &str) -> bool {
    let digits = entry.strip_prefix(['+', '-']).unwrap_or(entry);
    !Path::new(entry).is_absolute()
        && (digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn confirmation_terminal() -> std::io::Result<fs::File> {
    let terminal = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOCTTY)
        .open("/dev/tty")?;
    let session = unsafe { libc::getsid(0) };
    if session == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // At most one controlling terminal per session (and one session per terminal)
    // makes these SID checks bind stdin to the terminal that /dev/tty opened.
    for input in [libc::STDIN_FILENO, terminal.as_raw_fd()] {
        let controlling = unsafe { libc::tcgetsid(input) };
        if controlling == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if controlling != session {
            return Err(std::io::Error::other(
                "stdin is not this session's controlling terminal",
            ));
        }
    }
    Ok(terminal)
}

fn confirmation(input: libc::c_int) -> std::io::Result<bool> {
    // The independently opened terminal is nonblocking: a signal can flush a ready line.
    let mut answer = Vec::new();
    loop {
        if INTERRUPTED.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let mut input = libc::pollfd {
            fd: input,
            events: libc::POLLIN,
            revents: 0,
        };
        match unsafe { libc::poll(&mut input, 1, 100) } {
            0 => continue,
            -1 => {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            _ => {}
        }
        let mut byte = 0u8;
        let count = unsafe { libc::read(input.fd, (&mut byte as *mut u8).cast(), 1) };
        if count == 0 || (count == 1 && byte == b'\n') {
            break;
        }
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            return Err(error);
        }
        // Bound terminal input; a long answer is a decline.
        if answer.len() < 64 {
            answer.push(byte);
        } else {
            answer[0] = 0;
        }
    }
    Ok(matches!(answer.trim_ascii(), b"y" | b"Y" | b"yes" | b"YES"))
}

fn observe(
    report: &mut Report,
    before: &Snapshot,
    target: &Selector,
    invoked: &Build,
    places: &Places,
    host: &dyn Host,
) -> Option<Snapshot> {
    let after = inspect(
        Some(target),
        invoked.clone(),
        places,
        host,
        SystemTime::now(),
    );
    let snapshot = after.snapshot.as_ref();
    let state = match snapshot {
        Some(after) if after.element.record == before.element.record => "unchanged",
        Some(_) => "changed",
        None => "unknown",
    };
    report.launcher = after.launcher;
    report.selection = Some(Selection {
        state,
        selected: after.installed,
        source: after.source,
    });
    after.snapshot
}

/// Asks GitHub where `branch` is now and, when it moved, which way. Without
/// proved provenance a remote revision is only ever different, never newer.
fn remote(
    host: &dyn Host,
    branch: &str,
    installed: &str,
    proved: bool,
) -> Result<Availability, Trouble> {
    let revision = match request(
        host,
        &format!("{API}/commits/{branch}"),
        "application/vnd.github.sha",
    )? {
        Some(body) => {
            let body = String::from_utf8_lossy(&body).trim().to_owned();
            if !is_revision(&body) {
                return Err((
                    "metadata-invalid",
                    "GitHub returned no commit for the branch".into(),
                ));
            }
            body
        }
        None => return Err(("ref-missing", format!("GitHub has no branch {branch}"))),
    };
    let candidate = Some(revision.clone());
    if !proved {
        let state = if revision == installed {
            "unknown"
        } else {
            "different"
        };
        return Ok(Availability {
            state,
            candidate,
            reason: Some("provenance-unknown"),
        });
    }
    if revision == installed {
        return Ok(Availability {
            state: "current",
            candidate,
            reason: None,
        });
    }
    let comparison = request(
        host,
        &format!("{API}/compare/{installed}...{revision}?per_page=1"),
        "application/vnd.github+json",
    )?;
    let status = match comparison {
        Some(body) => serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|body| body["status"].as_str().map(str::to_owned))
            .ok_or((
                "metadata-invalid",
                "GitHub returned no comparison".to_owned(),
            ))?,
        // GitHub no longer has the installed revision: no ordering evidence.
        None => "unrelated".into(),
    };
    let (state, reason) = match status.as_str() {
        "ahead" => ("available", None),
        "identical" => ("current", None),
        "behind" => ("different", Some("behind")),
        "diverged" => ("different", Some("diverged")),
        _ => ("different", Some("unordered")),
    };
    Ok(Availability {
        state,
        candidate,
        reason,
    })
}

/// One API request: its body on success, None when GitHub has no such
/// object, or why the check failed.
fn request(host: &dyn Host, url: &str, accept: &str) -> Result<Option<Vec<u8>>, Trouble> {
    match host.get(url, accept)? {
        (200, body) => Ok(Some(body)),
        (404 | 422, _) => Ok(None),
        (403 | 429, _) => Err((
            "rate-limited",
            "GitHub's rate limit refused the check".into(),
        )),
        (status, _) => Err(("metadata-invalid", format!("GitHub answered HTTP {status}"))),
    }
}

/// Whether this user can write the directory that holds `profile`'s
/// generations, where Nix records an upgrade.
fn writable(profile: &Path) -> bool {
    profile_location(profile)
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .and_then(|directory| CString::new(directory.as_os_str().as_bytes()).ok())
        .is_some_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
}

/// Resolve aliases up to the mutable profile link, not its selected generation.
fn profile_location(profile: &Path) -> std::io::Result<PathBuf> {
    let mut link = profile.to_path_buf();
    for _ in 0..32 {
        let directory = link
            .parent()
            .ok_or_else(|| std::io::Error::other("profile has no parent"))?;
        let name = link
            .file_name()
            .ok_or_else(|| std::io::Error::other("profile has no name"))?;
        let location = fs::canonicalize(directory)?.join(name);
        let target = fs::read_link(&location)?;
        let target = location.parent().unwrap().join(target);
        if target
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| {
                name.strip_suffix("-link")
                    .and_then(|name| name.rsplit_once('-'))
                    .is_some_and(|(_, number)| {
                        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
        {
            return Ok(location);
        }
        link = target;
    }
    Err(std::io::Error::other(
        "profile symlink chain exceeds 32 links",
    ))
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    location: PathBuf,
    link: PathBuf,
    generation: PathBuf,
    element: Element,
    others: Vec<Element>,
}

fn snapshot(target: &Selector, host: &dyn Host) -> Result<Snapshot, Trouble> {
    let unreadable = |error| {
        (
            "selection-failed",
            format!("cannot snapshot profile: {error}"),
        )
    };
    let location = profile_location(&target.profile).map_err(unreadable)?;
    let generation = fs::canonicalize(&target.profile).map_err(unreadable)?;
    let link = fs::read_link(&location).map_err(unreadable)?;
    let mut others = elements(&host.profile_list(&target.profile)?)?;
    let index = others
        .iter()
        .position(|element| element.name == target.entry)
        .ok_or(("selection-failed", "profile entry disappeared".into()))?;
    let element = others.remove(index);
    if fs::canonicalize(&target.profile).map_err(unreadable)? != generation
        || profile_location(&target.profile).map_err(unreadable)? != location
        || fs::read_link(&location).map_err(unreadable)? != link
    {
        return Err(("target-changed", "profile changed while reading it".into()));
    }
    Ok(Snapshot {
        location,
        link,
        generation,
        element,
        others,
    })
}

/// `time` as an RFC 3339 UTC timestamp with seconds.
fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Howard Hinnant's civil-from-days, for days since 1970-01-01.
    let days = days as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    static SIGNAL_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use std::{
        cell::RefCell,
        collections::HashMap,
        os::unix::fs::symlink,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";
    const BUILD: &str = "g1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_BUILD: &str = "g1-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ATTRIBUTE: &str = "packages.x86_64-linux.default";

    /// A fake store, profiles and launchers under one temporary directory.
    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "eon-update-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("store")).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        /// A product package, as the composition builds it: its `bin` and
        /// `libexec` link to the wrapped build's, and it may carry a receipt.
        fn package(&self, name: &str, receipt: Option<String>) -> PathBuf {
            let build = self.0.join("store").join(format!("{name}-build"));
            fs::create_dir_all(build.join("bin")).unwrap();
            fs::write(build.join("bin/eon"), "").unwrap();
            fs::create_dir_all(build.join("libexec/eon/bin")).unwrap();
            symlink("../../../bin/eon", build.join("libexec/eon/bin/eon")).unwrap();
            let product = self.0.join("store").join(name);
            fs::create_dir_all(product.join("share")).unwrap();
            symlink(build.join("bin"), product.join("bin")).unwrap();
            symlink(build.join("libexec"), product.join("libexec")).unwrap();
            if let Some(receipt) = receipt {
                fs::write(product.join(RECEIPT), receipt).unwrap();
            }
            product
        }

        /// A Nix profile at `path` whose one generation selects `product`'s `eon`.
        fn profile(&self, path: &Path, product: &Path) -> PathBuf {
            let name = path.file_name().unwrap().to_str().unwrap();
            let environment = self.0.join("store").join(format!("env-{name}"));
            fs::create_dir_all(environment.join("bin")).unwrap();
            fs::write(environment.join("manifest.json"), "{}").unwrap();
            symlink(product.join("bin/eon"), environment.join("bin/eon")).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let generation = path.with_file_name(format!("{name}-1-link"));
            symlink(&environment, &generation).unwrap();
            symlink(generation.file_name().unwrap(), path).unwrap();
            path.to_path_buf()
        }

        /// Discovery for an `eon` started as `launcher`.
        fn places(&self, launcher: &Path) -> Places {
            Places {
                invoked: fs::canonicalize(launcher)
                    .ok()
                    .and_then(|path| path.parent().map(Path::to_path_buf)),
                launcher: Some(launcher.to_path_buf()),
                homeless: false,
                user_profiles: vec![
                    self.0.join("home/.nix-profile"),
                    self.0.join("state/nix/profile"),
                ],
                declarative: vec![
                    (self.0.join("per-user"), "per-user packages"),
                    (self.0.join("system"), "system packages"),
                ],
            }
        }

        fn home_profile(&self, product: &Path) -> PathBuf {
            let profile = self.profile(&self.0.join("profiles/profile"), product);
            fs::create_dir_all(self.0.join("home")).unwrap();
            symlink(&profile, self.0.join("home/.nix-profile")).unwrap();
            self.0.join("home/.nix-profile")
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn receipt(revision: Option<&str>, build: &str) -> Option<String> {
        Some(
            json!({
                "schema_version": 1,
                "repository": REPOSITORY,
                "revision": revision,
                "build": build,
                "platform": "x86_64-linux",
                "attribute": ATTRIBUTE,
            })
            .to_string(),
        )
    }

    /// A profile element: its name, original and locked references, attribute and package.
    type Listed<'a> = (
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        &'a Path,
    );

    /// `nix profile list --json` with one element per listed entry.
    fn listing(elements: &[Listed]) -> Result<Vec<u8>, Trouble> {
        let elements: serde_json::Map<_, _> = elements
            .iter()
            .map(|(name, original, locked, attribute, package)| {
                let mut element = json!({"active": true, "priority": 5, "storePaths": [package]});
                if let Some(original) = original {
                    element["originalUrl"] = (*original).into();
                    element["url"] = (*locked).into();
                    element["attrPath"] = (*attribute).into();
                    element["outputs"] = Value::Null;
                }
                ((*name).to_owned(), element)
            })
            .collect();
        Ok(json!({"version": 3, "elements": elements})
            .to_string()
            .into_bytes())
    }

    type Response = Result<(u16, Vec<u8>), Trouble>;

    #[derive(Default)]
    struct Fake {
        listings: HashMap<PathBuf, Result<Vec<u8>, Trouble>>,
        /// Responses by URL prefix after the API root.
        responses: Vec<(&'static str, Response)>,
        requests: RefCell<Vec<String>>,
    }

    impl Host for Fake {
        fn profile_list(&self, profile: &Path) -> Result<Vec<u8>, Trouble> {
            self.listings
                .get(profile)
                .cloned()
                .unwrap_or_else(|| panic!("unexpected listing of {}", profile.display()))
        }

        fn get(&self, url: &str, _accept: &str) -> Response {
            self.requests.borrow_mut().push(url.into());
            self.responses
                .iter()
                .find(|(prefix, _)| url.starts_with(&format!("{API}/{prefix}")))
                .map_or_else(
                    || panic!("unexpected request {url}"),
                    |(_, response)| response.clone(),
                )
        }
    }

    fn invoked() -> Build {
        Build {
            version: Some("0.1.21".into()),
            build: Some(BUILD.into()),
            revision: None,
            platform: Some("x86_64-linux".into()),
        }
    }

    fn at(time: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(time)
    }

    fn edge_listing(product: &Path, original: &str) -> Result<Vec<u8>, Trouble> {
        let locked = format!("github:Yazelix/eon/{OLD}?narHash=sha256-x");
        listing(&[(
            "eon",
            Some(original),
            Some(&locked),
            Some(ATTRIBUTE),
            product,
        )])
    }

    /// A default-profile `edge` install of the invoked build, started through
    /// the profile's launcher.
    fn edge_install(tree: &Tree, receipt_text: Option<String>) -> (Places, Fake) {
        let product = tree.package("eon", receipt_text);
        let profile = tree.home_profile(&product);
        let fake = Fake {
            listings: HashMap::from([(
                profile.clone(),
                edge_listing(&product, "github:Yazelix/eon/edge"),
            )]),
            ..Fake::default()
        };
        (tree.places(&profile.join("bin/eon")), fake)
    }

    fn words(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_package_words_after_update_reach_the_product() {
        for (line, wanted_here) in [
            (&["eon", "update", "--check"][..], true),
            (
                &["/nix/store/x-eon/bin/eon", "update", "--profile", "/p"],
                true,
            ),
            (&["eon", "update", "--install", "--yes"], true),
            (&["eon", "update"], false),
            (&["eon", "update", "--json"], false),
            (&["eon", "update", "--check", "--help"], false),
            (&["eonbase", "update", "--check"], false),
            (&["eon", "versions", "--check"], false),
        ] {
            assert_eq!(wanted(&words(line)), wanted_here, "{line:?}");
        }
    }

    #[test]
    fn arguments_outside_the_contract_are_invalid() {
        let accepted = parse(&words(&[
            "--json",
            "--check",
            "--profile",
            "/p",
            "--entry",
            "my eon",
        ]));
        assert!(accepted.json);
        assert_eq!(accepted.operation, Some("check"));
        assert_eq!(
            accepted.selection,
            Ok(Some(Selector {
                profile: "/p".into(),
                entry: "my eon".into()
            }))
        );
        assert_eq!(parse(&words(&["--check"])).selection, Ok(None));
        for line in [
            &["--profile", "/p", "--entry", "e"][..],
            &["--check", "--install"],
            &["--check", "--yes"],
            &["--check", "--prefix", "/x"],
            &[
                "--check",
                "--prefix",
                "/x",
                "--profile",
                "/p",
                "--entry",
                "e",
            ],
            &["--check", "--profile", "p", "--entry", "e"],
            &["--check", "--profile", "/p"],
            &["--check", "--entry", "e"],
            &["--check", "--entry", ""],
            &["--check", "--profile"],
            &["--check", "--check"],
            &["--check", "--json", "--json"],
            &["--check", "--window", "w1"],
        ] {
            let request = parse(&words(line));
            assert!(request.selection.is_err(), "{line:?}");
            let report = Report::invalid(request.operation, request.selection.unwrap_err());
            assert_eq!(report.status(), 2, "{line:?}");
            assert_eq!(report.json()["error"]["code"], "invalid-arguments");
        }
        assert_eq!(parse(&words(&["--install"])).operation, Some("install"));
        assert_eq!(parse(&words(&["--check", "--install"])).operation, None);
        assert_eq!(
            parse(&words(&["--check", "--check"])).selection,
            Err("--check given twice".into())
        );
    }

    #[test]
    fn inactive_entries_cannot_claim_to_update_the_profiles_launcher() {
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        let profile = &places.user_profiles[0];
        let mut list: Value =
            serde_json::from_slice(fake.listings[profile].as_ref().unwrap()).unwrap();
        list["elements"]["eon"]["active"] = false.into();
        fake.listings
            .insert(profile.clone(), Ok(list.to_string().into_bytes()));
        let report = inspect(None, invoked(), &places, &fake, at(0));
        assert_eq!(report.capability.reason, Some("install-ambiguous"));
        assert_eq!(
            Report::invalid(Some("install"), "bad argument".into()).json()["selection"]["state"],
            "unknown"
        );
    }

    #[test]
    fn exact_name_support_uses_nix_version_not_vendor_version() {
        assert!(exact_names(b"nix (Determinate Nix 3.21.2) 2.34.7\n"));
        assert!(exact_names(b"nix (Nix) 2.21.0"));
        assert!(!exact_names(b"nix (Nix) 2.20.9"));
        assert!(!exact_names(b"nix (Nix) 2.21.0pre"));
        assert!(!exact_names(b"nix (Lix, like Nix) 2.93.0"));
        assert!(!exact_names(b"nix (Determinate Nix 3.21.2) 2.19.9"));
        assert!(!exact_names(b"unrecognized"));
        for entry in [
            "1",
            "01",
            "+1",
            "-1",
            "/nix/store/00000000000000000000000000000000-eon",
        ] {
            assert!(!exact_entry(entry), "{entry}");
        }
        assert!(exact_entry("eon+x"));
        assert!(exact_entry("EON"));
        assert!(profile_location(Path::new("/tmp/..")).is_err());
    }

    #[test]
    fn confirmation_interrupt_before_read_does_not_wait_for_input() {
        use std::os::unix::net::UnixStream;
        let _signals = SIGNAL_TEST.lock().unwrap();
        let (input, mut output) = UnixStream::pair().unwrap();
        input.set_nonblocking(true).unwrap();
        let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            output.write_all(b"\n").unwrap();
        });
        INTERRUPTED.store(true, Ordering::Relaxed);
        let start = Instant::now();
        let result = confirmation(input.as_raw_fd());
        let elapsed = start.elapsed();
        INTERRUPTED.store(false, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(elapsed < Duration::from_millis(200), "{elapsed:?}");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(
            unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) },
            flags
        );
    }

    #[test]
    fn confirmation_read_after_ready_input_is_drained_cannot_block() {
        use std::os::unix::net::UnixStream;
        let _signals = SIGNAL_TEST.lock().unwrap();
        let (mut input, mut output) = UnixStream::pair().unwrap();
        input.set_nonblocking(true).unwrap();
        let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
        output.write_all(b"y\n").unwrap();
        let mut polled = libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut polled, 1, 0) }, 1);
        // Reproduce a terminal flushing its line after poll reported readiness.
        input.read_exact(&mut [0; 2]).unwrap();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            output.write_all(b"n\n").unwrap();
        });
        let mut byte = 0u8;
        let start = Instant::now();
        let count = unsafe { libc::read(input.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) };
        let error = std::io::Error::last_os_error();
        let elapsed = start.elapsed();
        writer.join().unwrap();
        assert!(elapsed < Duration::from_millis(200), "{elapsed:?}");
        assert_eq!(count, -1);
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(
            unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) },
            flags
        );
        assert!(!confirmation(input.as_raw_fd()).unwrap());
        assert_eq!(
            unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) },
            flags
        );
        assert!(!confirmation(input.as_raw_fd()).unwrap());
        assert_eq!(
            unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) },
            flags
        );
    }

    #[test]
    fn install_accepts_exact_targets_and_snapshots_selection_and_source() {
        let request = parse(&words(&[
            "--install",
            "--yes",
            "--profile",
            "/p",
            "--entry",
            "eon+x",
        ]));
        assert_eq!(request.operation, Some("install"));
        assert!(request.selection.is_ok());
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        let selector = Selector {
            profile: places.user_profiles[0].clone(),
            entry: "eon".into(),
        };
        let before = snapshot(&selector, &fake).unwrap();
        assert_eq!(snapshot(&selector, &fake).unwrap(), before);
        let mut managed = tree.places(&selector.profile.join("bin/eon"));
        managed.declarative = vec![(selector.profile.clone(), "declarative fixture")];
        assert!(matches!(
            owner(Some(&selector), &managed, &fake),
            Owner::Declarative(_)
        ));
        symlink(
            &before.generation,
            before.location.with_file_name("profile-2-link"),
        )
        .unwrap();
        fs::remove_file(&before.location).unwrap();
        symlink("profile-2-link", &before.location).unwrap();
        assert_ne!(snapshot(&selector, &fake).unwrap(), before);
        fs::remove_file(&before.location).unwrap();
        symlink(&before.link, &before.location).unwrap();
        let mut changed: Value =
            serde_json::from_slice(fake.listings[&selector.profile].as_ref().unwrap()).unwrap();
        changed["elements"]["eon"]["originalUrl"] = "github:Yazelix/eon/main".into();
        fake.listings.insert(
            selector.profile.clone(),
            Ok(changed.to_string().into_bytes()),
        );
        assert_ne!(snapshot(&selector, &fake).unwrap(), before);
    }

    #[test]
    fn an_edge_entry_behind_its_branch_has_an_update_it_may_install() {
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![
            ("commits/edge", Ok((200, format!("{NEW}\n").into_bytes()))),
            (
                "compare/",
                Ok((200, br#"{"status":"ahead","ahead_by":3}"#.to_vec())),
            ),
        ];
        let report = check(None, invoked(), &places, &fake, at(951_782_400));
        assert_eq!(report.status(), 0);
        let json = report.json();
        let profile = tree.0.join("home/.nix-profile");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["operation"], "check");
        assert_eq!(json["outcome"], "checked");
        assert_eq!(json["install_kind"], "nix-profile");
        assert_eq!(json["target"], json!({"profile": profile, "entry": "eon"}));
        assert_eq!(json["launcher"], json!(profile.join("bin/eon")));
        assert_eq!(json["source"]["policy"], "edge");
        assert_eq!(json["source"]["repository"], REPOSITORY);
        assert_eq!(json["source"]["revision"], OLD);
        assert_eq!(json["installed"]["build"], BUILD);
        assert_eq!(json["installed"]["revision"], OLD);
        assert_eq!(json["installed"]["version"], "0.1.21");
        assert_eq!(json["invoked"]["revision"], OLD);
        assert_eq!(json["running"], Value::Null);
        assert_eq!(json["selection"], Value::Null);
        assert_eq!(json["checked_at"], "2000-02-29T00:00:00Z");
        assert_eq!(
            json["availability"],
            json!({"state": "available", "candidate": {"version": null, "build": null, "revision": NEW, "archive_url": null}, "reason": null})
        );
        assert_eq!(json["capability"]["install"], true);
        assert_eq!(
            json["capability"]["next_step"],
            format!(
                "run eon update --install --profile {} --entry eon",
                profile.display()
            )
        );
        assert_eq!(json["error"], Value::Null);
        assert_eq!(
            *fake.requests.borrow(),
            [
                format!("{API}/commits/edge"),
                format!("{API}/compare/{OLD}...{NEW}?per_page=1")
            ]
        );
        let human = report.human();
        assert!(human.contains("Provenance: proved"), "{human}");
        assert!(human.contains("Eon may update this entry"), "{human}");
    }

    #[test]
    fn a_branch_moves_only_forward_with_ordering_evidence() {
        for (body, state, reason) in [
            (&br#"{"status":"behind"}"#[..], "different", Some("behind")),
            (br#"{"status":"diverged"}"#, "different", Some("diverged")),
            (br#"{"status":"identical"}"#, "current", None),
        ] {
            let tree = Tree::new();
            let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
            fake.responses = vec![
                ("commits/edge", Ok((200, NEW.into()))),
                ("compare/", Ok((200, body.to_vec()))),
            ];
            let report = check(None, invoked(), &places, &fake, at(0));
            assert_eq!(
                (report.availability.state, report.availability.reason),
                (state, reason)
            );
        }
        // GitHub no longer knows the installed revision.
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![
            ("commits/edge", Ok((200, NEW.into()))),
            ("compare/", Ok((404, Vec::new()))),
        ];
        assert_eq!(
            check(None, invoked(), &places, &fake, at(0))
                .availability
                .state,
            "different"
        );
    }

    #[test]
    fn an_entry_at_its_branch_revision_is_current_with_one_request() {
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![("commits/edge", Ok((200, OLD.into())))];
        let report = check(None, invoked(), &places, &fake, at(0));
        assert_eq!(report.availability.state, "current");
        assert_eq!(fake.requests.borrow().len(), 1);
    }

    #[test]
    fn remote_failures_never_read_as_current_or_eligible() {
        for (response, reason) in [
            (Err(("offline", "Could not resolve host".into())), "offline"),
            (
                Err(("curl-unavailable", "no curl".into())),
                "curl-unavailable",
            ),
            (Err(("curl-failed", "curl: (2)".into())), "curl-failed"),
            (Ok((403, Vec::new())), "rate-limited"),
            (Ok((429, Vec::new())), "rate-limited"),
            (Ok((500, Vec::new())), "metadata-invalid"),
            (Ok((200, b"<html>".to_vec())), "metadata-invalid"),
            (Ok((404, Vec::new())), "ref-missing"),
        ] {
            let tree = Tree::new();
            let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
            fake.responses = vec![("commits/edge", response)];
            let report = check(None, invoked(), &places, &fake, at(0));
            assert_eq!(report.status(), 1, "{reason}");
            let json = report.json();
            assert_eq!(json["outcome"], "failed");
            assert_eq!(json["error"]["code"], "remote-unavailable");
            assert_eq!(json["error"]["stage"], "check");
            assert_eq!(
                json["availability"],
                json!({"state": "unknown", "candidate": null, "reason": reason})
            );
            assert_eq!(json["capability"]["install"], false, "{reason}");
            assert_eq!(json["capability"]["reason"], Value::Null, "{reason}");
            assert!(!report.human().contains("Eon may update"), "{reason}");
        }
        // A failed comparison after a found revision fails the check too.
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![
            ("commits/edge", Ok((200, NEW.into()))),
            ("compare/", Ok((429, Vec::new()))),
        ];
        let report = check(None, invoked(), &places, &fake, at(0));
        assert_eq!(
            (report.outcome, report.availability.state),
            ("failed", "unknown")
        );
    }

    #[test]
    fn without_a_matching_receipt_nothing_is_current_or_installable() {
        let swapped = json!({
            "schema_version": 1,
            "extra": REPOSITORY,
            "revision": OLD,
            "build": BUILD,
            "platform": "x86_64-linux",
            "attribute": ATTRIBUTE,
        });
        let mismatched = [
            None,
            Some("{".into()),
            Some("[]".into()),
            Some(swapped.to_string()),
            receipt(None, BUILD),
            receipt(Some(NEW), BUILD),
            // The invoked build is this package's, so its receipt must name it.
            receipt(Some(OLD), OTHER_BUILD),
            Some(
                receipt(Some(OLD), BUILD)
                    .unwrap()
                    .replace(REPOSITORY, "https://github.com/fork/eon"),
            ),
            Some(
                receipt(Some(OLD), BUILD)
                    .unwrap()
                    .replace("\"schema_version\":1", "\"schema_version\":2"),
            ),
        ];
        for receipt_text in mismatched {
            for (remote, state) in [(OLD, "unknown"), (NEW, "different")] {
                let tree = Tree::new();
                let (places, mut fake) = edge_install(&tree, receipt_text.clone());
                fake.responses = vec![("commits/edge", Ok((200, remote.into())))];
                let report = check(None, invoked(), &places, &fake, at(0));
                assert!(!report.proved, "{receipt_text:?}");
                assert_eq!(report.availability.state, state, "{receipt_text:?}");
                assert_eq!(report.availability.reason, Some("provenance-unknown"));
                assert_eq!(report.capability.reason, Some("provenance-invalid"));
                assert_eq!(report.installed.as_ref().unwrap().build, None);
                assert_eq!(fake.requests.borrow().len(), 1);
                assert!(report.human().contains("Provenance: unknown"));
            }
        }
    }

    #[test]
    fn a_renamed_entry_in_a_custom_profile_is_named_explicitly_with_its_own_build() {
        let tree = Tree::new();
        let running = tree.package("eon-running", receipt(Some(OLD), BUILD));
        let installed = tree.package("eon-installed", receipt(Some(OLD), OTHER_BUILD));
        let profile = tree.profile(&tree.0.join("custom/my profile"), &installed);
        let locked = format!("github:Yazelix/eon/{OLD}");
        let fake = Fake {
            listings: HashMap::from([(
                profile.clone(),
                listing(&[(
                    "work-eon",
                    Some("github:yazelix/EON/main"),
                    Some(&locked),
                    Some(ATTRIBUTE),
                    &installed,
                )]),
            )]),
            responses: vec![("commits/main", Ok((200, OLD.into())))],
            ..Fake::default()
        };
        let selector = Selector {
            profile: profile.clone(),
            entry: "work-eon".into(),
        };
        let places = tree.places(&running.join("bin/eon"));
        let report = check(Some(&selector), invoked(), &places, &fake, at(0));
        let json = report.json();
        assert_eq!(json["install_kind"], "nix-profile");
        assert_eq!(json["source"]["policy"], "main");
        assert_eq!(json["availability"]["state"], "current");
        // Same version, different builds: the installed one is not this eon.
        assert_eq!(json["invoked"]["build"], BUILD);
        assert_eq!(json["invoked"]["revision"], Value::Null);
        assert_eq!(json["installed"]["build"], OTHER_BUILD);
        assert_eq!(json["installed"]["version"], Value::Null);
        assert_eq!(
            json["capability"]["next_step"],
            format!(
                "run eon update --install --profile '{}' --entry work-eon",
                profile.display()
            )
        );
    }

    #[test]
    fn a_pinned_entry_stays_pinned_and_checks_nothing_remote() {
        for (original, policy) in [
            ("github:Yazelix/eon/v0.1.21", "tag"),
            (&*format!("github:Yazelix/eon/{OLD}"), "commit"),
        ] {
            let tree = Tree::new();
            let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
            let product = tree.0.join("store/eon");
            fake.listings = HashMap::from([(
                tree.0.join("home/.nix-profile"),
                edge_listing(&product, original),
            )]);
            let report = check(None, invoked(), &places, &fake, at(0));
            assert_eq!(report.status(), 0);
            assert_eq!(report.source.as_ref().unwrap().policy, policy);
            assert_eq!(report.capability.reason, Some("pinned-install"));
            assert!(
                report
                    .capability
                    .next_step
                    .as_deref()
                    .unwrap()
                    .contains("github:Yazelix/eon/main")
            );
            assert_eq!(report.availability, unknown(Some("pinned")));
            assert!(fake.requests.borrow().is_empty());
        }
    }

    #[test]
    fn sources_other_than_the_official_branches_are_unverified() {
        for (original, attribute, policy, availability) in [
            (
                "github:someone/eon/edge",
                ATTRIBUTE,
                "unknown",
                "source-unverified",
            ),
            (
                "github:Yazelix/eon/feature",
                ATTRIBUTE,
                "unknown",
                "source-unverified",
            ),
            (
                "github:Yazelix/eon?ref=edge",
                ATTRIBUTE,
                "unknown",
                "source-unverified",
            ),
            (
                "github:Yazelix/eon",
                ATTRIBUTE,
                "unknown",
                "source-unverified",
            ),
            (
                "github:Yazelix/eon/edge",
                "packages.x86_64-linux.eonbase",
                "edge",
                "source-unverified",
            ),
            ("path:/home/me/eon", ATTRIBUTE, "local", "local-source"),
            (
                "git+https://github.com/Yazelix/eon?ref=edge",
                ATTRIBUTE,
                "unknown",
                "source-unverified",
            ),
        ] {
            let tree = Tree::new();
            let product = tree.package("eon", receipt(Some(OLD), BUILD));
            let profile = tree.profile(&tree.0.join("p"), &product);
            let locked = format!("github:Yazelix/eon/{OLD}");
            let fake = Fake {
                listings: HashMap::from([(
                    profile.clone(),
                    listing(&[(
                        "eon",
                        Some(original),
                        Some(&locked),
                        Some(attribute),
                        &product,
                    )]),
                )]),
                ..Fake::default()
            };
            let selector = Selector {
                profile,
                entry: "eon".into(),
            };
            let places = tree.places(&product.join("bin/eon"));
            let report = check(Some(&selector), invoked(), &places, &fake, at(0));
            assert_eq!(report.source.as_ref().unwrap().policy, policy, "{original}");
            assert_eq!(
                report.capability.reason,
                Some("source-unverified"),
                "{original}"
            );
            assert_eq!(
                report.availability,
                unknown(Some(availability)),
                "{original}"
            );
            assert!(fake.requests.borrow().is_empty());
        }
    }

    #[test]
    fn a_declarative_launch_is_externally_managed() {
        let tree = Tree::new();
        let product = tree.package("eon", receipt(Some(OLD), BUILD));
        let per_user = tree.profile(&tree.0.join("per-user"), &product);
        let report = check(
            None,
            invoked(),
            &tree.places(&per_user.join("bin/eon")),
            &Fake::default(),
            at(0),
        );
        assert_eq!(report.status(), 0);
        assert_eq!(report.install_kind, "externally-managed");
        assert_eq!(report.capability.reason, Some("externally-managed"));
        assert!(
            report
                .owner
                .as_deref()
                .unwrap()
                .contains("per-user packages")
        );

        // Standalone Home Manager owns the user profile's eon through its own entry.
        let tree = Tree::new();
        let product = tree.package("eon", None);
        let profile = tree.home_profile(&product);
        let fake = Fake {
            listings: HashMap::from([(
                profile.clone(),
                listing(&[("home-manager-path", None, None, None, &product)]),
            )]),
            ..Fake::default()
        };
        let places = tree.places(&profile.join("bin/eon"));
        let report = check(None, invoked(), &places, &fake, at(0));
        assert_eq!(report.install_kind, "externally-managed");
        assert!(report.owner.as_deref().unwrap().contains("Home Manager"));
    }

    #[test]
    fn a_launch_outside_any_installation_is_unmanaged_even_with_the_same_output() {
        // `nix run` starts the very output the default profile selects.
        let tree = Tree::new();
        let (_, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![("commits/edge", Ok((200, OLD.into())))];
        let places = tree.places(&tree.0.join("store/eon/bin/eon"));
        let report = check(None, invoked(), &places, &fake, at(0));
        assert_eq!(report.status(), 0);
        assert_eq!(report.install_kind, "unmanaged");
        assert_eq!(report.capability.reason, Some("install-unmanaged"));
        assert_eq!(report.target, None);
        assert!(fake.requests.borrow().is_empty());

        // A store-path entry has no source to follow.
        let tree = Tree::new();
        let product = tree.package("eon", None);
        let profile = tree.profile(&tree.0.join("p"), &product);
        let fake = Fake {
            listings: HashMap::from([(
                profile.clone(),
                listing(&[("eon", None, None, None, &product)]),
            )]),
            ..Fake::default()
        };
        let selector = Selector {
            profile,
            entry: "eon".into(),
        };
        let places = tree.places(&product.join("bin/eon"));
        let report = check(Some(&selector), invoked(), &places, &fake, at(0));
        assert_eq!(report.install_kind, "unmanaged");
    }

    #[test]
    fn conflicting_owners_a_windows_own_build_and_no_home_are_ambiguous() {
        let ambiguous = |places: &Places, fake: &Fake, why: &str| {
            let report = check(None, invoked(), places, fake, at(0));
            assert_eq!(report.status(), 0, "{why}");
            assert_eq!(report.install_kind, "unknown", "{why}");
            assert_eq!(report.capability.reason, Some("install-ambiguous"), "{why}");
            let owner = report.owner.unwrap();
            assert!(owner.contains(why), "{owner}");
            assert!(fake.requests.borrow().is_empty());
        };
        // Two entries of one profile install this eon.
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        let product = tree.0.join("store/eon");
        let profile = tree.0.join("home/.nix-profile");
        let locked = format!("github:Yazelix/eon/{OLD}");
        fake.listings = HashMap::from([(
            profile.clone(),
            listing(&[
                (
                    "eon",
                    Some("github:Yazelix/eon/edge"),
                    Some(&locked),
                    Some(ATTRIBUTE),
                    &product,
                ),
                (
                    "eon-1",
                    Some("github:Yazelix/eon/main"),
                    Some(&locked),
                    Some(ATTRIBUTE),
                    &product,
                ),
            ]),
        )]);
        ambiguous(&places, &fake, "several installations");
        // The profile started it, yet lists no entry that installs it.
        fake.listings = HashMap::from([(profile, listing(&[]))]);
        ambiguous(&places, &fake, "none of its entries");
        // An Eon terminal's `eon` is its window's build, whatever the profile now selects.
        let window = tree.0.join("store/eon/libexec/eon/bin/eon");
        ambiguous(
            &tree.places(&window),
            &Fake::default(),
            "Eon window's own build",
        );
        // No HOME, no default profile.
        let mut places = tree.places(&tree.0.join("store/eon/bin/eon"));
        places.homeless = true;
        places.user_profiles.clear();
        ambiguous(&places, &Fake::default(), "HOME is not set");
    }

    #[test]
    fn a_missing_or_unreadable_explicit_target_fails() {
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fs::create_dir(tree.0.join("not-a-profile")).unwrap();
        symlink(tree.0.join("gone"), tree.0.join("dangling")).unwrap();
        for profile in ["home/.nix-profile", "absent", "not-a-profile", "dangling"] {
            let entry = if profile == "home/.nix-profile" {
                "nope"
            } else {
                "eon"
            };
            let selector = Selector {
                profile: tree.0.join(profile),
                entry: entry.into(),
            };
            let report = check(Some(&selector), invoked(), &places, &fake, at(0));
            assert_eq!(report.status(), 3, "{profile}");
            assert_eq!(report.json()["error"]["code"], "unknown-target");
            assert_eq!(report.json()["error"]["stage"], "resolve");
        }
        // A Nix that cannot list the profile is its own failure, not ambiguity.
        let profile = tree.0.join("home/.nix-profile");
        for (trouble, reason) in [
            (
                ("nix-unavailable", "cannot run nix".to_owned()),
                "nix-unavailable",
            ),
            (
                ("nix-failed", "nix did not finish".to_owned()),
                "nix-failed",
            ),
        ] {
            fake.listings = HashMap::from([(profile.clone(), Err(trouble))]);
            let selector = Selector {
                profile: profile.clone(),
                entry: "eon".into(),
            };
            for report in [
                check(Some(&selector), invoked(), &places, &fake, at(0)),
                check(None, invoked(), &places, &fake, at(0)),
            ] {
                assert_eq!(report.status(), 1);
                assert_eq!(report.json()["error"]["code"], "selection-failed");
                assert_eq!(report.availability.reason, Some(reason));
            }
        }
        fake.listings = HashMap::from([(profile.clone(), Ok(b"{\"version\":4}".to_vec()))]);
        let report = check(None, invoked(), &places, &fake, at(0));
        assert_eq!(report.availability.reason, Some("nix-output-invalid"));
    }

    #[test]
    fn an_unwritable_profile_refuses() {
        let tree = Tree::new();
        let (places, mut fake) = edge_install(&tree, receipt(Some(OLD), BUILD));
        fake.responses = vec![("commits/edge", Ok((200, OLD.into())))];
        let generations = tree.0.join("profiles");
        let mode = |mode| {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&generations, fs::Permissions::from_mode(mode)).unwrap();
        };
        mode(0o555);
        let report = check(None, invoked(), &places, &fake, at(0));
        mode(0o755);
        assert_eq!(report.capability.reason, Some("target-unwritable"));
        assert_eq!(report.availability.state, "current");
    }

    #[test]
    fn locked_references_name_only_an_official_revision() {
        assert_eq!(
            locked_revision(&format!("github:Yazelix/eon/{OLD}?narHash=sha256-x")),
            Some(OLD.into())
        );
        assert_eq!(locked_revision(&format!("github:someone/eon/{OLD}")), None);
        assert_eq!(
            locked_revision(&format!("git+https://example.org/eon?ref=edge&rev={OLD}")),
            None
        );
        assert_eq!(locked_revision("github:Yazelix/eon/edge"), None);
        assert_eq!(locked_revision("path:/home/me/eon?lastModified=1"), None);
    }

    #[test]
    fn a_backend_that_hangs_is_stopped() {
        let _signals = SIGNAL_TEST.lock().unwrap();
        let mut command = Command::new("sleep");
        command.arg("10");
        let started = Instant::now();
        assert!(matches!(
            bounded(command, Duration::from_millis(200)),
            Err(Bounded::TimedOut(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(
            bounded(Command::new("/nonexistent/nix"), NIX_LIMIT),
            Err(Bounded::Spawn(_))
        ));
        let output = bounded(Command::new("true"), NIX_LIMIT).ok().unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn times_and_paths_have_their_contract_forms() {
        assert_eq!(rfc3339(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(at(1_700_000_000)), "2023-11-14T22:13:20Z");
        assert_eq!(
            json_path(Path::new(OsStr::from_bytes(b"/tmp/\xff"))),
            json!([47, 116, 109, 112, 47, 255])
        );
        assert_eq!(shell_word("/home/me/.nix-profile"), "/home/me/.nix-profile");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
    }
}
