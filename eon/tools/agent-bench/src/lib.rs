//! `agent-bench`: identity, storage and workspaces for the Eon bench (eon-2r02l).
//!
//! The bench compares fleet configurations (model, harness, effort, prompts, rules, tools) on
//! frozen tasks taken from landed work. This crate holds the parts every later bench piece keys
//! on, and nothing in it is specific to one repository: a task's repository commands come from
//! its `task.json`.
//!
//! - [`record`]: the configuration record (`eon-bench-config/2`, with `eon-bench-config/1` kept
//!   readable for comparison; judges included) and the task record (`eon-bench-task/1`). A record
//!   that parses is complete; its hash is SHA-256 over its RFC 8785 canonical JSON ([`jcs`]),
//!   shown as 12 hex digits.
//! - [`trial`]: trial (`eon-bench-trial/1`) and local lanes (`eon-bench-lanes/1`) JSON files and
//!   planned-run expansion (`agent-bench trial check`).
//! - [`stream`]: incremental NDJSON stream parsers for Claude Code, Codex CLI, and `agy` into
//!   one run-event model, plus cross-resume usage accounting.
//! - [`store`]: a directory holding each record as its canonical bytes under its own hash
//!   (`configs/`, `judges/`, `tasks/`), the append-only `results.jsonl` and `grades.jsonl`, and
//!   pooling declarations (`pooling/`). See [`rows`] and [`pooling`].
//! - [`diff`]: changed leaf paths between two records (`agent-bench config diff`).
//! - [`workspace`]: a synthetic workspace (one commit in a fresh repository, no history, refs or
//!   tracker data) and the check a workspace passes before an agent starts in it.
//! - [`tree`]: a content hash of a directory tree (`agent-bench tree-hash`).
//! - [`task_env`]: the task environment's half of the launched-workspace check, and the warm
//!   build that fills its target cache (`agent-bench task-env warm`).
//! - [`fingerprints`]: the needles the fingerprint scan looks for in every mounted host path.
//!
//! Row schemas (`eon-bench-run/1`, `eon-bench-grade/1`) are not hashed. They may gain fields
//! until the runner writes the first real row; after that, a field change bumps the schema.

pub mod cli;
pub mod diff;
pub mod fingerprints;
mod git;
pub mod jcs;
pub mod pooling;
pub mod record;
pub mod rows;
pub mod store;
pub mod stream;
pub mod task_env;
mod time;
pub mod tree;
pub mod trial;
pub mod workspace;

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::path::Path;

/// An error with its message; the CLI prints it and exits 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Returns early with an [`Error`] built like `format!`.
macro_rules! fail {
    ($($arg:tt)*) => {
        return Err($crate::Error(format!($($arg)*)))
    };
}
pub(crate) use fail;

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether every byte in `text` is a lowercase ASCII hex digit (`0-9`, `a-f`).
pub(crate) fn is_lower_hex(text: &str) -> bool {
    text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Refuses non-finite or negative floating-point values.
pub(crate) fn check_non_negative_finite(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        fail!("{label} must be finite and non-negative");
    }
    Ok(())
}

/// Refuses non-finite or non-positive floating-point values.
pub(crate) fn check_positive_finite(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        fail!("{label} must be finite and above 0");
    }
    Ok(())
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(bytes))
}

/// Writes `bytes` to `path` through a temporary file in the same directory and a rename, so
/// a reader sees the whole file or none.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

/// Holds an exclusive `flock` on `path` (created if absent) until the file is dropped.
pub(crate) fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|error| Error(format!("{}: {error}", path.display())))?;
    file.lock()?;
    Ok(file)
}

/// The 12-digit form shown in tables.
pub fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}
