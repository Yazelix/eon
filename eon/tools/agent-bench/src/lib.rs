//! `agent-bench`: identity, storage and workspaces for the Eon bench (eon-2r02l).
//!
//! The bench compares fleet configurations (model, harness, effort, prompts, rules, tools) on
//! frozen tasks taken from landed work. This crate holds the parts every later bench piece keys
//! on, and nothing in it is specific to one repository: a task's repository commands come from
//! its `task.json`.
//!
//! - [`record`]: the configuration record (`eon-bench-config/1`, judges included) and the task
//!   record (`eon-bench-task/1`). A record that parses is complete; its hash is SHA-256 over
//!   its RFC 8785 canonical JSON ([`jcs`]), shown as 12 hex digits.
//! - [`store`]: a directory holding each record as its canonical bytes under its own hash
//!   (`configs/`, `judges/`, `tasks/`), the append-only `results.jsonl` and `grades.jsonl`, and
//!   pooling declarations (`pooling/`). See [`rows`] and [`pooling`].
//! - [`diff`]: changed leaf paths between two records (`agent-bench config diff`).
//! - [`workspace`]: a synthetic workspace (one commit in a fresh repository, no history, refs or
//!   tracker data) and the check a workspace passes before an agent starts in it.
//!
//! Row schemas (`eon-bench-run/1`, `eon-bench-grade/1`) are not hashed. They may gain fields
//! until the runner writes the first real row; after that, a field change bumps the schema.

pub mod cli;
pub mod diff;
mod git;
pub mod jcs;
pub mod pooling;
pub mod record;
pub mod rows;
pub mod store;
mod time;
pub mod workspace;

use std::fmt;

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

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The 12-digit form shown in tables.
pub fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}
