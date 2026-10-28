//! Turns an Apple Silicon Mac reachable over SSH into an evidence folder for
//! Eon's macOS proof: the package build, the Darwin checks and the test
//! suites, each with its own deadline, log and status.

mod mac;
mod picture;
mod remote;
mod retina;
mod run;
mod scenario;
mod step;

use std::{path::PathBuf, process::ExitCode, time::Duration};

const USAGE: &str = "\
usage: mac-proof remote (--host USER@HOST | --local DIR) --out DIR [--rev REV] [--timeout 12h] [--dry-run]
       mac-proof run --out DIR [--expect-tree TREE] [--timeout 12h] [--dry-run]
       mac-proof scenario (--package PATH | --dry-run) --out DIR [--timeout 1h]
       mac-proof (remote | run | scenario) --help";

const REMOTE_HELP: &str = "\
mac-proof remote: prove a tree on a Mac over SSH (run it on Linux, inside the Eon checkout)

  --host USER@HOST  the Mac, reached with the system ssh in batch mode: a key login and a
                    known host key, no prompts; it needs Nix (the tool enables flakes)
                    and no Xcode
  --local DIR       use this machine instead, with DIR standing in for the Mac's HOME
  --out DIR         a fresh local directory for the evidence
  --rev REV         prove this commit; without it, prove HEAD when the checkout is clean,
                    else the candidate tree with uncommitted and untracked files
  --timeout DUR     the run's time limit on the Mac, as 30s, 90m or 12h (default 12h);
                    remote waits 30 minutes more for the tool to build and finish
  --dry-run         run only the source step there and plan the rest

It sends `git archive` of the tree into ~/eon-proof/<tree>-<time>/src on the host, runs
`nix run path:.#mac-proof -- run` there with stdin closed, and copies the evidence back into
--out, beside remote.log. The run keeps going on the Mac if the connection drops; the last
lines printed then name the command that fetches its evidence again, and otherwise the
cleanup command to run on the host. Exits 0 when every step passed (or, in a dry run, was
planned), 1 when one did not, and 2 when the round trip itself failed or the run's outcome
is unknown.";

const RUN_HELP: &str = "\
mac-proof run: the native steps, on Apple Silicon macOS only, in the current directory: a
fresh `git archive` extraction, which run commits into a new Git repository, or a clean Git
checkout

  --out DIR           a fresh directory for the evidence, outside the source
  --expect-tree TREE  fail the source step unless the source holds this Git tree
  --timeout DUR       the run's time limit, as 30s, 90m or 12h (default 12h): a step ends
                      at it at the latest, and later steps are skipped
  --dry-run           run only the source step, on any system, and plan the other fixed steps

Steps, in order; each has a deadline, and a step whose prerequisite did not pass is skipped:
  host        macOS (14 or later), hardware, displays, Nix (2.20 or later) and its builder
              sandbox setting, /tmp/e<c> directories, Eon processes and fonts like Eon's
              already present
  source      the Git tree, checked against --expect-tree
  package     nix build .#packages.aarch64-darwin.default
  terminfo    macOS's own infocmp and tput find `eon` only through the package (ORB-C10)
  check-list  the flake's aarch64-darwin checks, then check:<name> builds each but default
  workspace   the Cargo workspace's packages, then test:<package> runs eon-test on each
  session ... stop
              the GUI scenario on the built package; `mac-proof scenario --help` lists
              its steps and what each needs
  leftovers   fails on Eon processes or /tmp/e<c> directories the run left behind

Evidence folder:
  summary.txt          one line per step: status, duration, first failing line
  summary.json         {schema, result, dry_run, system, tree, head?, steps: [{name,
                       status, seconds, log, first_failure, for_yu}]}; status is pass,
                       fail, timeout, skipped or planned; head only for a Git checkout;
                       for_yu marks a step skipped because it, or a step it requires,
                       could not run on this Mac, which summary.txt then lists for Yu
  steps/NN-<step>.log  each step's commands and their output, with : in the step as -
Both summaries are rewritten after every step, with result incomplete until the last step
has run; then it is pass or fail. Exits 0 when every step passed (or, in a
dry run, was planned), 1 when one did not, and 2 when the run could not start.";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("remote") => parse(&arguments[1..], REMOTE_HELP)
            .and_then(remote_options)
            .and_then(remote::remote),
        Some("run") => parse(&arguments[1..], RUN_HELP)
            .and_then(run_options)
            .and_then(run::run),
        Some("scenario") => parse(&arguments[1..], &scenario::help())
            .and_then(scenario_options)
            .and_then(run::scenario),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) if message.is_empty() => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mac-proof: {message}");
            ExitCode::from(2)
        }
    }
}

/// Flags as (name, value), with `true` for the valueless ones. `--help`
/// prints `help` and ends with an empty error.
fn parse(arguments: &[String], help: &str) -> Result<Vec<(String, String)>, String> {
    let mut flags = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                println!("{help}");
                return Err(String::new());
            }
            "--dry-run" => flags.push((flag.clone(), "true".to_owned())),
            "--host" | "--local" | "--out" | "--rev" | "--timeout" | "--expect-tree"
            | "--package" => {
                let value = arguments
                    .next()
                    .ok_or(format!("{flag} needs a value\n{USAGE}"))?;
                flags.push((flag.clone(), value.clone()));
            }
            _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
        }
    }
    Ok(flags)
}

fn take(flags: &mut Vec<(String, String)>, name: &str) -> Option<String> {
    let index = flags.iter().position(|(flag, _)| flag == name)?;
    Some(flags.remove(index).1)
}

fn finish(flags: Vec<(String, String)>) -> Result<(), String> {
    match flags.first() {
        None => Ok(()),
        Some((flag, _)) => Err(format!(
            "{flag} is not a flag of this command or is repeated\n{USAGE}"
        )),
    }
}

fn remote_options(mut flags: Vec<(String, String)>) -> Result<remote::Options, String> {
    let host = match (take(&mut flags, "--host"), take(&mut flags, "--local")) {
        (Some(host), None) if !host.starts_with('-') => remote::Host::Ssh(host),
        (None, Some(home)) => remote::Host::Local(
            std::fs::canonicalize(&home).map_err(|error| format!("--local {home}: {error}"))?,
        ),
        _ => return Err(format!("give one of --host and --local\n{USAGE}")),
    };
    let out = take(&mut flags, "--out").ok_or(format!("--out is required\n{USAGE}"))?;
    let timeout = timeout(&mut flags, 12 * 3600)?;
    let options = remote::Options {
        host,
        out: PathBuf::from(out),
        rev: take(&mut flags, "--rev"),
        timeout,
        dry_run: take(&mut flags, "--dry-run").is_some(),
    };
    finish(flags)?;
    Ok(options)
}

fn run_options(mut flags: Vec<(String, String)>) -> Result<run::Options, String> {
    let out = take(&mut flags, "--out").ok_or(format!("--out is required\n{USAGE}"))?;
    let options = run::Options {
        limit: timeout(&mut flags, 12 * 3600)?,
        evidence: PathBuf::from(out),
        expect_tree: take(&mut flags, "--expect-tree"),
        dry_run: take(&mut flags, "--dry-run").is_some(),
    };
    finish(flags)?;
    Ok(options)
}

fn scenario_options(mut flags: Vec<(String, String)>) -> Result<run::ScenarioOptions, String> {
    let out = take(&mut flags, "--out").ok_or(format!("--out is required\n{USAGE}"))?;
    let dry_run = take(&mut flags, "--dry-run").is_some();
    let package = match take(&mut flags, "--package") {
        Some(package) => PathBuf::from(package),
        None if dry_run => PathBuf::new(),
        None => return Err(format!("--package is required\n{USAGE}")),
    };
    let options = run::ScenarioOptions {
        package,
        evidence: PathBuf::from(out),
        limit: timeout(&mut flags, 3600)?,
        dry_run,
    };
    finish(flags)?;
    Ok(options)
}

fn timeout(flags: &mut Vec<(String, String)>, default: u64) -> Result<Duration, String> {
    match take(flags, "--timeout") {
        Some(text) => {
            step::parse_duration(&text).ok_or(format!("--timeout {text}: give 30s, 90m or 12h"))
        }
        None => Ok(Duration::from_secs(default)),
    }
}
