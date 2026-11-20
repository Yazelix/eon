//! The `agent-bench` command line. Exit status: 0 done, 1 refused (an invalid record, a
//! refused declaration, a failed check), 2 usage error.

use crate::pooling::PoolKind;
use crate::record::Kind;
use crate::store::Store;
use crate::{Error, Result, diff, fingerprints, task_env, tree, workspace};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage:
  agent-bench record --store DIR FILE
  agent-bench config diff --store DIR A B
  agent-bench pool declare --store DIR --kind config|task --pair OLD NEW [--pair OLD NEW]... --reason TEXT --signed-by NAME
  agent-bench pool show --store DIR CONFIG TASK    (prints {\"rows\": [...], \"excluded\": [...]})
  agent-bench workspace create --source REPO --base COMMIT --out DIR [--drop NAME]...
  agent-bench workspace check DIR --object-set SHA256 [--task FILE --task-env DIR --private-layer DIR...]
      (--task, --task-env and at least one --private-layer go together: they add the task
      environment's checks against task.json)
  agent-bench tree-hash DIR    (prints the tree's content hash)
  agent-bench fingerprints --source REPO --base COMMIT --landed COMMIT --canary FILE --out FILE
      (writes the needles file; prints {\"needles\": N, \"identifiers\": I, \"excluded_crates\": [...]})
  agent-bench task-env warm --agent-jail BIN --profile NAME --task-env DIR --run DIR --object-set SHA256 --wall-cap MINUTES
      (runs the warm build through agent-jail, then writes and prints warm.json)

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
        ["record", rest @ ..] => record(&parse(rest, &["--store"], &[], &[], 1)?, out),
        ["config", "diff", rest @ ..] => config_diff(&parse(rest, &["--store"], &[], &[], 2)?, out),
        ["pool", "declare", rest @ ..] => pool_declare(
            &parse(
                rest,
                &["--store", "--kind", "--reason", "--signed-by"],
                &[],
                &["--pair"],
                0,
            )?,
            out,
        ),
        ["pool", "show", rest @ ..] => pool_show(&parse(rest, &["--store"], &[], &[], 2)?, out),
        ["workspace", "create", rest @ ..] => workspace_create(
            &parse(rest, &["--source", "--base", "--out"], &[], &["--drop"], 0)?,
            out,
        ),
        ["workspace", "check", rest @ ..] => workspace_check(
            &parse(
                rest,
                &["--object-set"],
                &["--task", "--task-env"],
                &["--private-layer"],
                1,
            )?,
            out,
        ),
        ["tree-hash", rest @ ..] => {
            let args = parse(rest, &[], &[], &[], 1)?;
            writeln!(out, "{}", tree::hash(Path::new(&args.positional[0]))?)?;
            Ok(())
        }
        ["fingerprints", rest @ ..] => write_fingerprints(
            &parse(
                rest,
                &["--source", "--base", "--landed", "--canary", "--out"],
                &[],
                &[],
                0,
            )?,
            out,
        ),
        ["task-env", "warm", rest @ ..] => task_env_warm(
            &parse(
                rest,
                &[
                    "--agent-jail",
                    "--profile",
                    "--task-env",
                    "--run",
                    "--object-set",
                    "--wall-cap",
                ],
                &[],
                &[],
                0,
            )?,
            out,
        ),
        _ => Err(Failure::Usage("unknown command".into())),
    }
}

/// Parsed arguments: single-valued options, repeatable options, positionals.
struct Args {
    single: Vec<(String, String)>,
    /// `--pair` takes two values; the others one.
    repeated: Vec<(String, Vec<String>)>,
    positional: Vec<String>,
}

impl Args {
    fn get(&self, name: &str) -> &str {
        self.optional(name)
            .expect("required options are checked in parse")
    }

    fn optional(&self, name: &str) -> Option<&str> {
        self.single
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
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

/// Every option in `required` is given exactly once, every one in `optional` at most once;
/// `repeated` options may repeat.
fn parse(
    words: &[&str],
    required: &[&str],
    optional: &[&str],
    repeated: &[&str],
    positionals: usize,
) -> std::result::Result<Args, Failure> {
    let single = [required, optional].concat();
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
    for name in required {
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
    let layers: Vec<PathBuf> = args
        .all("--private-layer")
        .into_iter()
        .map(|v| PathBuf::from(&v[0]))
        .collect();
    let task_env = match (args.optional("--task"), args.optional("--task-env")) {
        (Some(task), Some(dir)) if !layers.is_empty() => Some((task, dir)),
        (None, None) if layers.is_empty() => None,
        _ => {
            return Err(Failure::Usage(
                "--task, --task-env and at least one --private-layer go together".into(),
            ));
        }
    };
    let object_set = args.get("--object-set");
    let mut report = workspace::check(&PathBuf::from(&args.positional[0]), object_set)?;
    if let Some((task, dir)) = task_env {
        let input = std::fs::read(task).map_err(|error| Error(format!("{task}: {error}")))?;
        let task =
            crate::record::task(&input).map_err(|error| Error(format!("{task}: {}", error.0)))?;
        report
            .violations
            .extend(task_env::check(&task, Path::new(dir), object_set, &layers));
    }
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

fn write_fingerprints(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    let summary = fingerprints::write(
        Path::new(args.get("--source")),
        args.get("--base"),
        args.get("--landed"),
        Path::new(args.get("--canary")),
        Path::new(args.get("--out")),
    )?;
    writeln!(
        out,
        "{}",
        serde_json::to_string(&summary).map_err(Error::from)?
    )?;
    Ok(())
}

fn task_env_warm(args: &Args, out: &mut dyn Write) -> std::result::Result<(), Failure> {
    task_env::warm(
        task_env::WarmBuild {
            agent_jail: std::process::Command::new(args.get("--agent-jail")),
            profile: args.get("--profile"),
            task_env: Path::new(args.get("--task-env")),
            run: Path::new(args.get("--run")),
            object_set: args.get("--object-set"),
            wall_cap: args.get("--wall-cap"),
        },
        out,
    )?;
    Ok(())
}
