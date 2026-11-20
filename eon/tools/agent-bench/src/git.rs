//! A thin layer over the `git` executable: argument vectors, never a shell. Every call drops
//! all `GIT_*` variables and reads neither system nor global configuration, so neither the
//! caller's environment nor their settings change what a workspace looks like.

use crate::{Result, fail};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn command(dir: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command
}

/// Runs `command`, feeding `input` on stdin, and returns stdout or an error with stderr.
pub fn output(mut command: Command, input: Option<&[u8]>) -> Result<Vec<u8>> {
    let description = format!("{command:?}");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|error| crate::Error(format!("cannot run git: {error}")))?;
    let output = std::thread::scope(|scope| {
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            scope.spawn(move || stdin.write_all(input));
        }
        child.wait_with_output()
    })?;
    if !output.status.success() {
        fail!(
            "{description} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// Runs git in `dir` with `args` and returns stdout.
pub fn run(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = command(Some(dir));
    command.args(args);
    output(command, None)
}

/// Like [`run`], with standard input.
pub fn run_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut command = command(Some(dir));
    command.args(args);
    output(command, Some(input))
}

/// Runs git and returns stdout as one trimmed line.
pub fn line(dir: &Path, args: &[&str]) -> Result<String> {
    let out = run(dir, args)?;
    match String::from_utf8(out) {
        Ok(text) => Ok(text.trim_end_matches('\n').to_owned()),
        Err(_) => fail!("git {} printed non-UTF-8 output", args.join(" ")),
    }
}

/// Resolves `rev` in `dir` to a full commit id, refusing anything that looks like an option.
pub fn commit(dir: &Path, rev: &str) -> Result<String> {
    if rev.starts_with('-') {
        fail!("{rev:?} is not a revision");
    }
    line(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
}

/// One `ls-tree -z` entry.
pub struct Entry {
    pub mode: String,
    pub kind: String,
    pub oid: String,
    pub path: String,
}

/// Parses `ls-tree -z` output.
pub fn entries(listing: &[u8]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in listing.split(|b| *b == 0).filter(|i| !i.is_empty()) {
        let Ok(item) = std::str::from_utf8(item) else {
            fail!("a path in the tree is not UTF-8");
        };
        let Some((meta, path)) = item.split_once('\t') else {
            fail!("unexpected ls-tree line {item:?}");
        };
        let mut fields = meta.split(' ');
        let (Some(mode), Some(kind), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            fail!("unexpected ls-tree line {item:?}");
        };
        out.push(Entry {
            mode: mode.into(),
            kind: kind.into(),
            oid: oid.into(),
            path: path.into(),
        });
    }
    Ok(out)
}
