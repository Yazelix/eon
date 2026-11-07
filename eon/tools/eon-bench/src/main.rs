//! Eon's benchmark harness (docs/DEVELOPMENT.md), a development tool outside
//! Eon's closure. Each suite writes its evidence under `--out`.

mod corpus;
mod sha256;

use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
usage: eon-bench corpus --out DIR [--only ID]...
       eon-bench corpus --help";

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
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let options = match arguments.first().map(String::as_str) {
        Some("corpus") => corpus_options(&arguments[1..]),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(USAGE.to_owned()),
    };
    let (out, only) = match options {
        Ok(options) => options,
        Err(message) if message.is_empty() => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("eon-bench: {message}");
            return ExitCode::from(2);
        }
    };
    match corpus::write_files(&out, &only) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eon-bench: {error}");
            ExitCode::FAILURE
        }
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
                    let corpus = corpus::find(value).ok_or(format!(
                        "no corpus {value}; `eon-bench corpus --help` lists them"
                    ))?;
                    if !only.iter().any(|known| known.id == corpus.id) {
                        only.push(corpus);
                    }
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
