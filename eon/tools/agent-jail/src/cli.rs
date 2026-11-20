//! The `agent-jail` command line. The same binary is init, the hooks, the
//! probe and every wrapper; a wrapper is recognised by the path it was
//! executed from, never by argv[0].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::enter::{Host, Options};
use crate::pins::Pins;
use crate::proxy::{Limits, Proxy};
use crate::spec::Mode;
use crate::task_env::{TaskEnv, Template};

const USAGE: &str = "usage:
  agent-jail enter <profile> --task-env <dir> --run <dir> --wall-cap <minutes> --model <m> --effort <e> --prompt <text>
  agent-jail command <profile> --task-env <dir> --run <dir> --wall-cap <minutes> --template build|test [--base-rw]
  agent-jail probe <profile> --task-env <dir> --run <dir> [--wall-cap <minutes>]
  agent-jail probe-model <profile> --task-env <dir> --root <dir> --probe --model <m> --effort <e> [--only <id,id>]
  agent-jail task-env store --flake <dir> --dev-shell <attr> --out <dir> [--exclude <pname>]... [--pass <pattern>]...
                            --test-command <json> --test-build <json>
  agent-jail task-env cargo --out <dir> --lockfile <path>... --tree <name>=<dir>...
  agent-jail task-env clean <dir>
  agent-jail scan-tree --needles <file> [--credentials claude,codex] <path>...
  agent-jail describe <profile>
inside the jail: init | jail-hook <harness> | dcg-hook <harness> | control-hook <kind> <harness> | where
                 probe-harness <profile> | probe-inner <profile> | probe-peer <kind> <arg>";

/// Exit status of `scan-tree` on a usage error (1 means a hit).
const USAGE_EXIT: i32 = 2;

pub fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    if let Ok(exe) = std::fs::read_link("/proc/self/exe") {
        let exe = exe.display().to_string();
        // agent-jail runs from /bench/jail as init and the hooks; from any
        // other jail path (the tool names, the shells, a harness release's
        // executables) it is a wrapper.
        let wrapper = (exe.starts_with("/bench/")
            && exe != format!("{}/agent-jail", crate::JAIL_DIR))
            || crate::SHELL_PATHS.contains(&exe.as_str());
        if wrapper {
            let argv0 = args.first().cloned().unwrap_or_else(|| exe.clone().into());
            let code = crate::wrapper::run(&exe, argv0, args.into_iter().skip(1).collect());
            return ExitCode::from(u8::try_from(code).unwrap_or(1));
        }
    }
    let args: Vec<String> = match args
        .into_iter()
        .skip(1)
        .map(OsString::into_string)
        .collect()
    {
        Ok(args) => args,
        Err(_) => return fail("arguments must be UTF-8"),
    };
    let code = match run(&args) {
        Ok(code) => code,
        Err(err) => return fail(&err),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn fail(message: &str) -> ExitCode {
    eprintln!("agent-jail: {message}");
    ExitCode::from(1)
}

/// Parsed arguments: positionals in order, and each `--name`'s values.
#[derive(Debug, Default)]
struct Args {
    positional: Vec<String>,
    flags: BTreeMap<String, Vec<String>>,
}

impl Args {
    /// Parses `args`: `valued` flags take one value each and may repeat
    /// only if in `repeated`; `bare` flags take none. Any other `--` word
    /// is an error.
    fn parse(
        args: &[String],
        valued: &[&str],
        repeated: &[&str],
        bare: &[&str],
    ) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let Some(name) = arg.strip_prefix("--") else {
                parsed.positional.push(arg.clone());
                continue;
            };
            let value = if bare.contains(&name) {
                String::new()
            } else if valued.contains(&name) || repeated.contains(&name) {
                iter.next()
                    .ok_or(format!("--{name} needs a value"))?
                    .clone()
            } else {
                return Err(format!("unknown flag --{name}\n{USAGE}"));
            };
            let values = parsed.flags.entry(name.to_owned()).or_default();
            if !values.is_empty() && !repeated.contains(&name) {
                return Err(format!("--{name} given twice"));
            }
            values.push(value);
        }
        Ok(parsed)
    }

    fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.flags
            .get(name)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.get(name).ok_or(format!("--{name} is required"))
    }

    fn all(&self, name: &str) -> &[String] {
        self.flags.get(name).map(Vec::as_slice).unwrap_or_default()
    }

    /// The single positional argument, named `what` in the error.
    fn one(&self, what: &str) -> Result<&str, String> {
        match self.positional.as_slice() {
            [one] => Ok(one),
            _ => Err(format!("expected exactly one {what}\n{USAGE}")),
        }
    }
}

fn now() -> Result<i64, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    i64::try_from(now).map_err(|err| err.to_string())
}

fn self_exe() -> Result<PathBuf, String> {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|err| format!("cannot find agent-jail itself: {err}"))
}

fn host(profile: &str, args: &Args) -> Result<Host, String> {
    let loaded = crate::profile::load(profile)?;
    let task = TaskEnv::load(Path::new(args.required("task-env")?))?;
    let pins = Pins::load(
        &[
            "bwrap",
            "dcg",
            "bash",
            "nix",
            "ca-bundle",
            loaded.profile.binary.as_str(),
        ],
        task.store_dir(),
    )?;
    Ok(Host {
        pins,
        self_exe: self_exe()?,
        auth: crate::enter::default_auth(&loaded.profile.harness)?,
        resolver: Proxy::system_resolver(),
        limits: Limits::default(),
        now: now()?,
        task,
        disk_floor: crate::enter::DiskFloor::new(crate::task_build::DISK_FLOOR),
    })
}

fn options(
    profile: &str,
    args: &Args,
    mode: Mode,
    default_cap: Option<&str>,
) -> Result<Options, String> {
    let cap = args
        .get("wall-cap")
        .or(default_cap)
        .ok_or("--wall-cap is required")?;
    let slots: BTreeMap<String, String> = ["prompt", "model", "effort"]
        .iter()
        .filter_map(|name| {
            args.get(name)
                .map(|value| ((*name).to_owned(), value.to_owned()))
        })
        .collect();
    Ok(Options {
        profile: profile.to_owned(),
        run: PathBuf::from(args.required("run")?),
        wall_cap: cap
            .parse()
            .map_err(|_| "--wall-cap must be a number of minutes")?,
        probe: args.has("probe"),
        mode,
        slots,
        // Control variants come only from probe-model, never from the CLI.
        control: crate::control::Control::None,
        template: args.get("template").map(Template::parse).transpose()?,
        base_rw: args.has("base-rw"),
    })
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string(value).map_err(|err| err.to_string())?
    );
    Ok(())
}

/// Reads one `--test-*` flag: a JSON array of strings.
fn argv_flag(args: &Args, name: &str) -> Result<Vec<String>, String> {
    serde_json::from_str(args.required(name)?)
        .map_err(|err| format!("--{name} is not a JSON array of strings: {err}"))
}

fn task_env(rest: &[String]) -> Result<i32, String> {
    let (which, rest) = rest.split_first().ok_or(USAGE)?;
    match which.as_str() {
        "store" => {
            let args = Args::parse(
                rest,
                &["flake", "dev-shell", "out", "test-command", "test-build"],
                &["exclude", "pass"],
                &[],
            )?;
            if !args.positional.is_empty() {
                return Err(USAGE.into());
            }
            let options = crate::task_build::StoreOptions {
                flake: PathBuf::from(args.required("flake")?),
                dev_shell: args.required("dev-shell")?.to_owned(),
                out: PathBuf::from(args.required("out")?),
                exclude: args.all("exclude").to_vec(),
                pass: args.all("pass").to_vec(),
                test: crate::task_env::Test {
                    command: argv_flag(&args, "test-command")?,
                    build: argv_flag(&args, "test-build")?,
                },
            };
            let printed = crate::task_build::store(
                &options,
                &self_exe()?,
                crate::task_build::DISK_FLOOR,
                &mut crate::task_build::system,
            )?;
            print_json(&printed)?;
        }
        "cargo" => {
            let args = Args::parse(rest, &["out"], &["lockfile", "tree"], &[])?;
            if !args.positional.is_empty()
                || args.all("lockfile").is_empty()
                || args.all("tree").is_empty()
            {
                return Err(USAGE.into());
            }
            let trees = args
                .all("tree")
                .iter()
                .map(|tree| {
                    let (name, dir) = tree
                        .split_once('=')
                        .filter(|(name, dir)| !name.is_empty() && !dir.is_empty())
                        .ok_or(format!("--tree {tree} is not NAME=DIR"))?;
                    Ok((name.to_owned(), PathBuf::from(dir)))
                })
                .collect::<Result<_, String>>()?;
            let options = crate::task_build::CargoOptions {
                out: PathBuf::from(args.required("out")?),
                lockfiles: args.all("lockfile").to_vec(),
                trees,
            };
            let pins = Pins::load(&["ca-bundle"], Path::new(crate::task_env::STORE))?;
            let printed = crate::task_build::cargo(
                &options,
                &pins.host("ca-bundle")?,
                &mut crate::task_build::system,
            )?;
            print_json(&printed)?;
        }
        "clean" => {
            let args = Args::parse(rest, &[], &[], &[])?;
            crate::task_env::clean(Path::new(args.one("task env directory")?))?;
        }
        _ => return Err(USAGE.into()),
    }
    Ok(0)
}

/// `scan-tree`: exit 1 on any hit, 0 on none, [`USAGE_EXIT`] when it
/// cannot search.
fn scan_tree(rest: &[String]) -> i32 {
    let searched = || -> Result<Vec<crate::scan::TreeHit>, String> {
        let args = Args::parse(rest, &["needles", "credentials"], &[], &[])?;
        if args.positional.is_empty() {
            return Err(format!("scan-tree needs at least one path\n{USAGE}"));
        }
        let needles_file = args.required("needles")?;
        let text = std::fs::read_to_string(needles_file)
            .map_err(|err| format!("{needles_file}: {err}"))?;
        let needles = crate::scan::needles(&text)?;
        let (mut secrets, mut account_ids) = (Vec::new(), Vec::new());
        for harness in args
            .get("credentials")
            .into_iter()
            .flat_map(|list| list.split(','))
        {
            let auth = crate::enter::default_auth(harness)?;
            let text = std::fs::read_to_string(&auth)
                .map_err(|_| format!("{harness}: cannot read the credential file"))?;
            let (found, accounts) = crate::credential::secrets(harness, &text)?;
            secrets.extend(found);
            account_ids.extend(accounts);
        }
        let mut paths = Vec::new();
        for path in &args.positional {
            let metadata =
                std::fs::symlink_metadata(path).map_err(|err| format!("{path}: {err}"))?;
            if metadata.file_type().is_symlink() {
                return Err(format!("{path} is a symbolic link; name what it points to"));
            }
            if !metadata.is_dir() && !metadata.is_file() {
                return Err(format!("{path} is not a directory or a regular file"));
            }
            paths.push(PathBuf::from(path));
        }
        crate::scan::scan_tree(needles, &secrets, &account_ids, &paths)
    };
    match searched() {
        Ok(hits) => {
            for hit in &hits {
                if print_json(hit).is_err() {
                    return USAGE_EXIT;
                }
            }
            i32::from(!hits.is_empty())
        }
        Err(err) => {
            eprintln!("agent-jail: scan-tree: {err}");
            USAGE_EXIT
        }
    }
}

fn hook_command(command: &str, rest: &[String]) -> i32 {
    let blocked = |message: &str| {
        eprintln!("agent-jail: {command}: {message}");
        crate::hook::DENY_EXIT
    };
    match (command, rest) {
        ("jail-hook", [harness]) => crate::hook::run_jail_hook(harness),
        ("control-hook", [kind, harness]) => crate::control::run_control_hook(kind, harness),
        ("dcg-hook", [_harness]) => match crate::spec::Spec::read() {
            Ok(spec) => crate::hook::run_dcg_hook(&spec.dcg),
            Err(err) => blocked(&err),
        },
        _ => blocked("unexpected arguments"),
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    let command = args.first().map(String::as_str).ok_or(USAGE)?;
    let rest = &args[1..];
    let one = |what: &str| {
        rest.first()
            .cloned()
            .ok_or(format!("{command} needs {what}\n{USAGE}"))
    };
    match command {
        "init" => Ok(crate::guard::init()),
        // Hooks block on any error of their own: exit 2 is the only status
        // both harnesses treat as a block (Codex fails open on others).
        "jail-hook" | "control-hook" | "dcg-hook" => Ok(hook_command(command, rest)),
        "where" => Ok(crate::probe::where_command()),
        "probe-harness" => Ok(crate::probe::probe_harness(&one("a profile")?)),
        "probe-inner" => Ok(crate::probe::probe_inner(&one("a profile")?)),
        "probe-peer" => {
            let arg = rest
                .get(1)
                .ok_or("probe-peer needs a kind and an argument")?;
            Ok(crate::probe::probe_peer(&one("a kind")?, arg))
        }
        "describe" => {
            let args = Args::parse(rest, &[], &[], &[])?;
            let record = crate::describe::describe(args.one("profile")?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&record).map_err(|err| err.to_string())?
            );
            Ok(0)
        }
        "task-env" => task_env(rest),
        "scan-tree" => Ok(scan_tree(rest)),
        "enter" | "command" => {
            let (mode, valued, bare): (Mode, &[&str], &[&str]) = if command == "enter" {
                (
                    Mode::Harness,
                    &["task-env", "run", "wall-cap", "model", "effort", "prompt"],
                    &[],
                )
            } else {
                (
                    Mode::Command,
                    &["task-env", "run", "wall-cap", "template"],
                    &["base-rw"],
                )
            };
            let args = Args::parse(rest, valued, &[], bare)?;
            let profile = args.one("profile")?;
            if mode == Mode::Command && !args.has("template") {
                return Err("--template is required".into());
            }
            let outcome = crate::enter::enter(
                &options(profile, &args, mode, None)?,
                &host(profile, &args)?,
            )?;
            print_json(&outcome)?;
            Ok(crate::enter::exit_status(&outcome))
        }
        "probe-model" => {
            let args = Args::parse(
                rest,
                &["task-env", "root", "model", "effort", "only"],
                &[],
                &["probe"],
            )?;
            let profile = args.one("profile")?;
            if !args.has("probe") {
                return Err("probe-model runs only with --probe".into());
            }
            let root = PathBuf::from(args.required("root")?);
            std::fs::create_dir_all(&root).map_err(|err| format!("{}: {err}", root.display()))?;
            let slots: BTreeMap<String, String> = ["model", "effort"]
                .iter()
                .map(|name| Ok(((*name).to_owned(), args.required(name)?.to_owned())))
                .collect::<Result<_, String>>()?;
            let only: Vec<String> = args
                .get("only")
                .map(|list| list.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            let report = crate::model::run(profile, &root, &slots, &only, &host(profile, &args)?)?;
            let pass = report["pass"] == true;
            println!(
                "probe-model {profile}: {}",
                if pass { "pass" } else { "FAIL" }
            );
            Ok(i32::from(!pass))
        }
        "probe" => {
            let args = Args::parse(rest, &["task-env", "run", "wall-cap"], &[], &[])?;
            let profile = args.one("profile")?;
            let report = crate::probe::run(
                &options(profile, &args, Mode::StandIn, Some("10"))?,
                &host(profile, &args)?,
            )?;
            for check in report["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|check| check["pass"] != true)
            {
                println!("FAIL {check}");
            }
            let pass = report["pass"] == true;
            println!("probe {profile}: {}", if pass { "pass" } else { "FAIL" });
            Ok(i32::from(!pass))
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn flags_repeat_only_where_allowed() {
        let args = Args::parse(
            &words("--out o --exclude br --exclude bv --base-rw extra"),
            &["out"],
            &["exclude"],
            &["base-rw"],
        )
        .unwrap();
        assert_eq!(args.all("exclude"), ["br", "bv"]);
        assert_eq!(args.get("out"), Some("o"));
        assert!(args.has("base-rw"));
        assert_eq!(args.one("x").unwrap(), "extra");
        assert!(Args::parse(&words("--out a --out b"), &["out"], &[], &[]).is_err());
        assert!(Args::parse(&words("--probe"), &["out"], &[], &[]).is_err());
        assert!(Args::parse(&words("--out"), &["out"], &[], &[]).is_err());
    }

    #[test]
    fn scan_tree_exits_two_when_it_cannot_search() {
        assert_eq!(scan_tree(&words("/tmp")), USAGE_EXIT);
        assert_eq!(
            scan_tree(&words("--needles /nonexistent/needles /tmp")),
            USAGE_EXIT
        );
        let dir = crate::test_dir("cli-scan-tree");
        std::fs::write(dir.join("needles"), "canary AGENT-JAIL-CLI-CANARY\n").unwrap();
        std::fs::create_dir(dir.join("tree")).unwrap();
        let needles = dir.join("needles").display().to_string();
        let tree = dir.join("tree").display().to_string();
        assert_eq!(
            scan_tree(&["--needles".into(), needles.clone()]),
            USAGE_EXIT
        );
        assert_eq!(
            scan_tree(&["--needles".into(), needles.clone(), tree.clone()]),
            0
        );
        std::fs::write(dir.join("tree/hit"), "x AGENT-JAIL-CLI-CANARY y").unwrap();
        assert_eq!(
            scan_tree(&["--needles".into(), needles.clone(), tree.clone()]),
            1
        );
        std::os::unix::fs::symlink(&tree, dir.join("link")).unwrap();
        let link = dir.join("link").display().to_string();
        assert_eq!(
            scan_tree(&["--needles".into(), needles.clone(), link]),
            USAGE_EXIT
        );
        // A FIFO or socket target is not something to search.
        crate::mknod(&dir.join("fifo"), rustix::fs::FileType::Fifo, 0).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(dir.join("socket")).unwrap();
        for special in ["fifo", "socket"] {
            let path = dir.join(special).display().to_string();
            assert_eq!(
                scan_tree(&["--needles".into(), needles.clone(), path]),
                USAGE_EXIT,
                "{special}"
            );
        }
        // A needles file with nothing in it looks for nothing: refused.
        std::fs::remove_file(dir.join("tree/hit")).unwrap();
        for empty in ["", "\n\n"] {
            std::fs::write(dir.join("needles"), empty).unwrap();
            assert_eq!(
                scan_tree(&["--needles".into(), needles.clone(), tree.clone()]),
                USAGE_EXIT,
                "{empty:?}"
            );
        }
    }
}
