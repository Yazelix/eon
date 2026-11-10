//! The `eon-bench` command line.

use std::{path::PathBuf, process::ExitCode};

use eon_bench::{child, corpus, floor, gate, headless, memory, orbit, response, terminals, writer};

const USAGE: &str = "\
usage: eon-bench corpus --out DIR [--only ID]...
       eon-bench floor --layer pty --out DIR [OPTIONS]
       eon-bench orbit --out DIR [OPTIONS]
       eon-bench gate --baseline FILE --eon PATH --source-tree HASH [OPTIONS]
       eon-bench memory --out DIR [OPTIONS]
       eon-bench headless --eon PATH --out DIR [OPTIONS]
       eon-bench terminals --eon PATH --source-tree HASH --out DIR [OPTIONS]
       eon-bench response --eon PATH --out DIR [OPTIONS]
       eon-bench write FILE --report REPORT
       eon-bench COMMAND --help";

const WRITE_HELP: &str = "\
eon-bench write FILE --report REPORT: the completion writer

Run inside a terminal: turns off line editing and echo on stdin and discards
input already queued, writes FILE to stdout, asks for a status report (ESC[5n)
and waits up to 60 s for the terminal's reply (ESC[0n), then restores stdin.
REPORT gets {\"bytes\": N, \"seconds\": S, \"started_ns\": T0, \"replied_ns\": T1}:
the time from the first write to the reply, and the monotonic clock at both.
Exits 0 on a reply, 1 on a failure, an ESC[3n reply or no reply, and 2 for a
usage error. It sets no deadline on its write: a suite that runs it inside a
terminal kills that terminal when a sample overruns, which ends the writer.";

enum Command {
    Corpus(PathBuf, Vec<&'static corpus::Corpus>),
    Floor(floor::Options),
    Orbit(orbit::Options),
    Gate(gate::Options),
    GateWriter(PathBuf),
    Memory(memory::Options, bool),
    Child(child::Options),
    Write(PathBuf, PathBuf),
    Response(response::Options),
    Fixture(Vec<String>),
    Headless(headless::Options, bool),
    Terminals(terminals::Options, bool),
    TerminalWorkload(Vec<String>),
}

fn corpus_help() -> String {
    let mut help = format!(
        "\
eon-bench corpus: write the benchmark corpora (set version {})

  --out DIR   the directory for the files; created if missing
  --only ID   write only this corpus; repeat for several (default: all)

Writes DIR/<id>.vt for each corpus and DIR/SHA256SUMS, which `sha256sum -c`
checks. Every corpus is generated from a fixed seed and must match its recorded
length and SHA-256; a mismatch fails the run, and an earlier SHA256SUMS is
removed first. `cursor` addresses an 80x24 grid with absolute positions, so it
draws as intended only on that grid. Exits 0 when every corpus was written and
matched, 1 when one failed, and 2 for a usage error.

Corpora:",
        corpus::CORPUS_VERSION
    );
    for corpus in &corpus::CORPORA {
        help.push_str(&format!(
            "\n  {:<13}{:>9} bytes  {}\n  {:13}sha256 {}",
            corpus.id, corpus.length, corpus.summary, "", corpus.sha256
        ));
    }
    help
}

fn main() -> ExitCode {
    if std::env::args_os().next().is_some_and(|arg| {
        std::path::Path::new(&arg)
            .file_name()
            .is_some_and(|n| n == "memory-workload")
    }) {
        return match memory::workspace_workload() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("memory workload: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if std::env::args().next().is_some_and(|arg| {
        std::path::Path::new(&arg)
            .file_name()
            .is_some_and(|name| name == "memory-presentation")
    }) {
        return match memory::presentation() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("memory presentation: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let rest = arguments.get(1..).unwrap_or_default();
    let command = match arguments.first().map(String::as_str) {
        Some("corpus") => corpus_options(rest).map(|(out, only)| Command::Corpus(out, only)),
        Some("floor") => floor::options(rest).map(Command::Floor),
        Some("orbit") => orbit::options(rest).map(Command::Orbit),
        Some("gate") => gate::options(rest).map(Command::Gate),
        Some("gate-writer") if rest.len() == 1 => Ok(Command::GateWriter(rest[0].clone().into())),
        Some("memory" | "memory-inner") => memory::options(rest)
            .map(|options| Command::Memory(options, arguments[0] == "memory-inner")),
        Some("child") => child::options(rest).map(Command::Child),
        Some("headless" | "headless-inner") => headless::options(rest)
            .map(|options| Command::Headless(options, arguments[0] == "headless-inner")),
        Some("response") => response::options(rest).map(Command::Response),
        Some("terminals" | "terminals-inner") => terminals::options(rest)
            .map(|options| Command::Terminals(options, arguments[0] == "terminals-inner")),
        Some("terminal-workload") => Ok(Command::TerminalWorkload(rest.to_vec())),
        Some("response-fixture") if rest.len() == 4 => Ok(Command::Fixture(rest.to_vec())),
        Some("write") => write_options(rest).map(|(file, report)| Command::Write(file, report)),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(USAGE.to_owned()),
    };
    let command = match command {
        Ok(command) => command,
        Err(message) if message.is_empty() => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("eon-bench: {message}");
            return ExitCode::from(2);
        }
    };
    let outcome = match command {
        Command::Corpus(out, only) => corpus::write_files(&out, &only),
        Command::Floor(options) => floor::run(&options),
        Command::Orbit(options) => orbit::run(&options),
        Command::Gate(options) => gate::run(&options),
        Command::GateWriter(dir) => orbit::invariants::writer(&dir),
        Command::Memory(options, false) => memory::run(rest, &options),
        Command::Memory(options, true) => memory::run_inner(&options),
        Command::Child(options) => match child::run(&options) {
            Ok(code) => return ExitCode::from(code),
            Err(error) => Err(error),
        },
        Command::Write(file, report) => writer::run(&file, &report),
        Command::Response(options) => response::run(&options),
        Command::Headless(options, false) => headless::run(rest, &options),
        Command::Headless(options, true) => headless::run_inner(&options),
        Command::Fixture(arguments) => response::fixture(&arguments),
        Command::Terminals(options, false) => terminals::run(rest, &options),
        Command::Terminals(options, true) => terminals::run_inner(&options),
        Command::TerminalWorkload(arguments) => terminals::workload(&arguments),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eon-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `--help` prints the help and ends with an empty error.
fn write_options(arguments: &[String]) -> Result<(PathBuf, PathBuf), String> {
    match arguments {
        [help] if matches!(help.as_str(), "--help" | "-h") => {
            println!("{WRITE_HELP}");
            Err(String::new())
        }
        [file, flag, report] if flag == "--report" => {
            Ok((PathBuf::from(file), PathBuf::from(report)))
        }
        _ => Err(format!("write takes FILE --report REPORT\n{USAGE}")),
    }
}

/// `--help` prints the help and ends with an empty error.
fn corpus_options(arguments: &[String]) -> Result<(PathBuf, Vec<&'static corpus::Corpus>), String> {
    let mut out = None;
    let mut only: Vec<&corpus::Corpus> = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                println!("{}", corpus_help());
                return Err(String::new());
            }
            "--out" | "--only" => {
                let value = arguments
                    .next()
                    .ok_or(format!("{flag} needs a value\n{USAGE}"))?;
                if flag == "--out" {
                    if out.replace(PathBuf::from(value)).is_some() {
                        return Err(format!("--out is repeated\n{USAGE}"));
                    }
                } else {
                    corpus::add(&mut only, value)?;
                }
            }
            _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
        }
    }
    let out = out.ok_or(format!("--out is required\n{USAGE}"))?;
    if only.is_empty() {
        only = corpus::CORPORA.iter().collect();
    }
    Ok((out, only))
}
