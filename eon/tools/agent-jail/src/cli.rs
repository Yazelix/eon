//! The `agent-jail` command line. The same binary is init, the hooks, the
//! probe and every wrapper; a wrapper is recognised by the path it was
//! executed from, never by argv[0].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::enter::{Host, Options};
use crate::pins::Pins;
use crate::proxy::{Limits, Proxy};
use crate::spec::Mode;

const USAGE: &str = "usage:
  agent-jail enter <profile> --run <dir> --wall-cap <minutes> --probe --model <m> --effort <e> --prompt <text>
  agent-jail probe <profile> --run <dir> --probe [--wall-cap <minutes>]
  agent-jail probe-model <profile> --root <dir> --probe --model <m> --effort <e> [--only <id,id>]
  agent-jail describe <profile> [--probe-only]
inside the jail: init | jail-hook <harness> | dcg-hook <harness> | control-hook <kind> <harness> | where
                 probe-harness <profile> | probe-inner <profile> | probe-peer <kind> <arg>";

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

/// Flags as `--name value` pairs; bare flags map to an empty value.
fn flags(args: &[String], bare: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let name = arg
            .strip_prefix("--")
            .ok_or(format!("unexpected argument {arg}"))?;
        if map.contains_key(name) {
            return Err(format!("--{name} given twice"));
        }
        let value = if bare.contains(&name) {
            String::new()
        } else {
            iter.next()
                .ok_or(format!("--{name} needs a value"))?
                .clone()
        };
        map.insert(name.to_owned(), value);
    }
    Ok(map)
}

fn host(profile: &str) -> Result<Host, String> {
    let loaded = crate::profile::load(profile)?;
    let pins = Pins::load(&["bwrap", "dcg", "ca-bundle", loaded.profile.binary.as_str()])?;
    let self_exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|err| format!("cannot find agent-jail itself: {err}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    Ok(Host {
        pins,
        self_exe,
        auth: crate::enter::default_auth(&loaded.profile.harness)?,
        resolver: Proxy::system_resolver(),
        limits: Limits::default(),
        now: i64::try_from(now).map_err(|err| err.to_string())?,
    })
}

fn options(
    profile: &str,
    flags: &BTreeMap<String, String>,
    mode: Mode,
    default_cap: Option<&str>,
) -> Result<Options, String> {
    let run = flags.get("run").ok_or("--run is required")?;
    let cap = flags
        .get("wall-cap")
        .map(String::as_str)
        .or(default_cap)
        .ok_or("--wall-cap is required")?;
    let slots: BTreeMap<String, String> = ["prompt", "model", "effort"]
        .iter()
        .filter_map(|name| {
            flags
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect();
    Ok(Options {
        profile: profile.to_owned(),
        run: PathBuf::from(run),
        wall_cap: cap
            .parse()
            .map_err(|_| "--wall-cap must be a number of minutes")?,
        probe: flags.contains_key("probe"),
        mode,
        slots,
        // Control variants come only from probe-model, never from the CLI.
        control: crate::control::Control::None,
    })
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
            let flags = flags(&rest[1.min(rest.len())..], &["probe-only"])?;
            let record =
                crate::describe::describe(&one("a profile")?, flags.contains_key("probe-only"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&record).map_err(|err| err.to_string())?
            );
            Ok(0)
        }
        "enter" => {
            let profile = one("a profile")?;
            let flags = flags(&rest[1..], &["probe"])?;
            let outcome = crate::enter::enter(
                &options(&profile, &flags, Mode::Harness, None)?,
                &host(&profile)?,
            )?;
            println!(
                "{}",
                serde_json::to_string(&outcome).map_err(|err| err.to_string())?
            );
            Ok(crate::enter::exit_status(&outcome))
        }
        "probe-model" => {
            let profile = one("a profile")?;
            let flags = flags(&rest[1..], &["probe"])?;
            if !flags.contains_key("probe") {
                return Err("probe-model runs only with --probe".into());
            }
            let root = PathBuf::from(flags.get("root").ok_or("--root is required")?);
            std::fs::create_dir_all(&root).map_err(|err| format!("{}: {err}", root.display()))?;
            let slots: BTreeMap<String, String> = ["model", "effort"]
                .iter()
                .map(|name| {
                    Ok((
                        (*name).to_owned(),
                        flags
                            .get(*name)
                            .ok_or(format!("--{name} is required"))?
                            .clone(),
                    ))
                })
                .collect::<Result<_, String>>()?;
            let only: Vec<String> = flags
                .get("only")
                .map(|list| list.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            let report = crate::model::run(&profile, &root, &slots, &only, &host(&profile)?)?;
            println!(
                "probe-model {profile}: {}",
                if report["pass"] == true {
                    "pass"
                } else {
                    "FAIL"
                }
            );
            Ok(if report["pass"] == true { 0 } else { 1 })
        }
        "probe" => {
            let profile = one("a profile")?;
            let flags = flags(&rest[1..], &["probe"])?;
            let report = crate::probe::run(
                &options(&profile, &flags, Mode::StandIn, Some("10"))?,
                &host(&profile)?,
            )?;
            let failed: Vec<&serde_json::Value> = report["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|check| check["pass"] != true)
                .collect();
            for check in &failed {
                println!("FAIL {check}");
            }
            println!(
                "probe {}: {}",
                profile,
                if report["pass"] == true {
                    "pass"
                } else {
                    "FAIL"
                }
            );
            Ok(if report["pass"] == true { 0 } else { 1 })
        }
        _ => Err(USAGE.into()),
    }
}
