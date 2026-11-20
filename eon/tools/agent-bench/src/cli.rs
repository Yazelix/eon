//! The `agent-bench` command line. Exit status: 0 done, 1 refused (an invalid record, a
//! refused declaration, a failed check), 2 usage error.

use crate::pooling::PoolKind;
use crate::record::Kind;
use crate::store::Store;
use crate::{Error, Result, diff, workspace};
use std::io::Write;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage:
  agent-bench record --store DIR FILE
  agent-bench config diff --store DIR A B
  agent-bench pool declare --store DIR --kind config|task --pair OLD NEW [--pair OLD NEW]... --reason TEXT --signed-by NAME
  agent-bench pool show --store DIR CONFIG TASK    (prints {\"rows\": [...], \"excluded\": [...]})
  agent-bench workspace create --source REPO --base COMMIT --out DIR [--drop NAME]...
  agent-bench workspace check DIR --object-set SHA256

Hashes may be given as unique hex prefixes. Run `agent-bench help` for this text.";

/// Why the command stopped.
pub enum Failure {
    Usage(String),
    Refused(String),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Refused(error.0)
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Refused(error.to_string())
    }
}

/// Runs the command line `args` (without the program name), writing results to `out`.
pub fn run(args: &[String], out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["help" | "--help" | "-h"] => {
            writeln!(out, "{USAGE}")?;
            Ok(())
        }
        ["record", rest @ ..] => record(&parse(rest, &["--store"], &[], 1)?, out),
        ["config", "diff", rest @ ..] => config_diff(&parse(rest, &["--store"], &[], 2)?, out),
        ["pool", "declare", rest @ ..] => pool_declare(
            &parse(
                rest,
                &["--store", "--kind", "--reason", "--signed-by"],
                &["--pair"],
                0,
            )?,
            out,
        ),
        ["pool", "show", rest @ ..] => pool_show(&parse(rest, &["--store"], &[], 2)?, out),
        ["workspace", "create", rest @ ..] => workspace_create(
            &parse(rest, &["--source", "--base", "--out"], &["--drop"], 0)?,
            out,
        ),
        ["workspace", "check", rest @ ..] => {
            workspace_check(&parse(rest, &["--object-set"], &[], 1)?, out)
        }
        _ => Err(Failure::Usage("unknown command".into())),
    }
}

/// Parsed arguments: single-valued options, repeatable options, positionals.
struct Args {
    single: Vec<(String, String)>,
    /// `--pair` takes two values; `--drop` one.
    repeated: Vec<(String, Vec<String>)>,
    positional: Vec<String>,
}

impl Args {
    fn get(&self, name: &str) -> &str {
        self.single
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .expect("required options are checked in parse")
    }

    fn all(&self, name: &str) -> Vec<&Vec<String>> {
        self.repeated
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }

    fn store(&self) -> Store {
        Store::new(PathBuf::from(self.get("--store")))
    }
}

/// Every option in `single` is required exactly once; `repeated` options may repeat.
fn parse(
    words: &[&str],
    single: &[&str],
    repeated: &[&str],
    positionals: usize,
) -> std::result::Result<Args, Failure> {
    let usage = |text: String| Err(Failure::Usage(text));
    let mut args = Args {
        single: Vec::new(),
        repeated: Vec::new(),
        positional: Vec::new(),
    };
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let arity = if word == "--pair" { 2 } else { 1 };
        if single.contains(&word) || repeated.contains(&word) {
            let values = &words[index + 1..];
            if values.len() < arity || values[..arity].iter().any(|v| v.starts_with("--")) {
                return usage(format!("{word} needs {arity} value(s)"));
            }
            let values: Vec<String> = values[..arity].iter().map(|v| (*v).to_owned()).collect();
            if single.contains(&word) {
                if args.single.iter().any(|(n, _)| n == word) {
                    return usage(format!("{word} given twice"));
                }
                args.single.push((word.to_owned(), values[0].clone()));
            } else {
                args.repeated.push((word.to_owned(), values));
            }
            index += 1 + arity;
        } else if word.starts_with("--") {
            return usage(format!("unknown option {word}"));
        } else {
            args.positional.push(word.to_owned());
            index += 1;
        }
    }
    for name in single {
        if !args.single.iter().any(|(n, _)| n == name) {
            return usage(format!("missing {name}"));
        }
    }
    if args.positional.len() != positionals {
        return usage(format!(
            "expected {positionals} argument(s), got {}",
            args.positional.len()
        ));
    }
    Ok(args)
}

fn record(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let path = &args.positional[0];
    let input = std::fs::read(path).map_err(|error| Error(format!("{path}: {error}")))?;
    let stored = args
        .store()
        .put(&input)
        .map_err(|error| Error(format!("{path}: {}", error.0)))?;
    writeln!(out, "{} {}", stored.kind().name(), stored.hash)?;
    Ok(())
}

fn config_diff(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let store = args.store();
    let kinds = [Kind::Config, Kind::Judge];
    let load = |prefix: &str| -> Result<serde_json::Value> {
        let (kind, hash) = store.resolve(&kinds, prefix)?;
        Ok(store.get(kind, &hash)?.record.to_value())
    };
    let old = load(&args.positional[0])?;
    let new = load(&args.positional[1])?;
    for change in diff::diff(&old, &new) {
        writeln!(out, "{change}")?;
    }
    Ok(())
}

fn pool_declare(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let (kind, record_kind) = match args.get("--kind") {
        "config" => (PoolKind::Config, Kind::Config),
        "task" => (PoolKind::Task, Kind::Task),
        other => {
            return Err(Failure::Usage(format!(
                "--kind is config or task, not {other}"
            )));
        }
    };
    let store = args.store();
    let mut pairs = Vec::new();
    for pair in args.all("--pair") {
        let old = store.resolve(&[record_kind], &pair[0])?.1;
        let new = store.resolve(&[record_kind], &pair[1])?.1;
        pairs.push((old, new));
    }
    let (hash, declaration) =
        store.declare(kind, &pairs, args.get("--reason"), args.get("--signed-by"))?;
    writeln!(out, "pooling {hash}")?;
    for pair in &declaration.pairs {
        writeln!(
            out,
            "{} -> {}: {}",
            crate::short(pair.old.as_str()),
            crate::short(pair.new.as_str()),
            pair.changed.join(", ")
        )?;
    }
    Ok(())
}

fn pool_show(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let store = args.store();
    let config = store.resolve(&[Kind::Config], &args.positional[0])?.1;
    let task = store.resolve(&[Kind::Task], &args.positional[1])?.1;
    let pooled = store.pooled(&config, &task)?;
    writeln!(
        out,
        "{}",
        serde_json::to_string(&pooled).map_err(Error::from)?
    )?;
    Ok(())
}

fn workspace_create(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let drops: Vec<String> = args
        .all("--drop")
        .into_iter()
        .map(|v| v[0].clone())
        .collect();
    let snapshot = workspace::create(
        &PathBuf::from(args.get("--source")),
        args.get("--base"),
        &PathBuf::from(args.get("--out")),
        &drops,
    )?;
    let json = serde_json::json!({
        "commit": snapshot.commit,
        "branch": snapshot.branch,
        "object_set_sha256": snapshot.object_set_sha256,
    });
    writeln!(out, "{json}")?;
    Ok(())
}

fn workspace_check(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let report = workspace::check(
        &PathBuf::from(&args.positional[0]),
        args.get("--object-set"),
    )?;
    for violation in &report.violations {
        writeln!(out, "{violation}")?;
    }
    if report.git_skipped {
        writeln!(out, "git checks skipped: fix the violations above first")?;
    }
    if report.passed() {
        writeln!(out, "ok")?;
        Ok(())
    } else {
        Err(Failure::Refused(format!(
            "{} violation(s)",
            report.violations.len()
        )))
    }
}
