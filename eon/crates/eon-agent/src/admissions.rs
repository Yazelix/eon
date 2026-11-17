//! The admission ledger: for each account, the newest quota observation an admission spent.
//!
//! `admissions.json` beside `state_file`:
//!
//! ```text
//! {"version": 1, "accounts": {"ACCOUNT": {"observed_at": MS}}}
//! ```
//!
//! Admissions read and write it under the admission lock, so one observation admits at most one
//! launch or run per account. A file that cannot be read, does not parse, or has another version
//! holds every backend that sets limits; it is never guessed at or rewritten. Deleting it
//! recovers, at the cost of letting each account's last observation admit once more.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    accounts: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    observed_at: u64,
}

pub struct Ledger {
    path: PathBuf,
    /// The spent observation per account, or why the file is unusable.
    accounts: Result<BTreeMap<String, u64>, String>,
}

impl Ledger {
    /// The ledger beside `state_file`; a missing file is an empty ledger.
    pub fn load(state_file: &Path) -> Self {
        let path = crate::session::state_dir(state_file).join("admissions.json");
        let unreadable =
            |why: String| format!("admission ledger unreadable: {}: {why}", path.display());
        let accounts = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|e| unreadable(e.to_string()))
                .and_then(|json| match json.get("version").and_then(|v| v.as_u64()) {
                    Some(v) if v == u64::from(VERSION) => {
                        serde_json::from_value::<File>(json).map_err(|e| unreadable(e.to_string()))
                    }
                    Some(v) => Err(unreadable(format!("version {v}, expected {VERSION}"))),
                    None => Err(unreadable("no version".into())),
                })
                .map(|file| {
                    file.accounts
                        .into_iter()
                        .map(|(account, entry)| (account, entry.observed_at))
                        .collect()
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(unreadable(e.to_string())),
        };
        Ledger { path, accounts }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The observation an admission on `account` last spent, if any.
    pub fn spent(&self, account: &str) -> Result<Option<u64>, String> {
        self.accounts
            .as_ref()
            .map(|accounts| accounts.get(account).copied())
            .map_err(Clone::clone)
    }

    /// Records that an admission on `account` spent `observed_at`, replacing the file atomically.
    pub fn record(&mut self, account: &str, observed_at: u64) -> Result<(), String> {
        let accounts = self.accounts.as_mut().map_err(|e| e.clone())?;
        accounts.insert(account.into(), observed_at);
        let file = File {
            version: VERSION,
            accounts: accounts
                .iter()
                .map(|(account, at)| (account.clone(), Entry { observed_at: *at }))
                .collect(),
        };
        let mut bytes = serde_json::to_vec(&file).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        crate::launch::atomic_write(&self.path, &bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_bad_files_are_never_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state/anchor");
        let mut ledger = Ledger::load(&state);
        assert_eq!(ledger.spent("a"), Ok(None));
        ledger.record("a", 1000).unwrap();
        ledger.record("b", 7).unwrap();
        let ledger = Ledger::load(&state);
        assert_eq!(ledger.spent("a"), Ok(Some(1000)));
        assert_eq!(ledger.spent("b"), Ok(Some(7)));
        assert_eq!(
            std::fs::read_to_string(ledger.path()).unwrap(),
            "{\"version\":1,\"accounts\":{\"a\":{\"observed_at\":1000},\"b\":{\"observed_at\":7}}}\n"
        );
        for (text, why) in [
            ("{", "EOF"),
            ("{\"version\":2,\"accounts\":{}}", "version 2, expected 1"),
            ("{\"accounts\":{}}", "no version"),
            (
                "{\"version\":1,\"accounts\":{\"a\":{\"observed_at\":-1}}}",
                "invalid",
            ),
            (
                "{\"version\":1,\"accounts\":{},\"extra\":1}",
                "unknown field",
            ),
        ] {
            std::fs::write(ledger.path(), text).unwrap();
            let mut bad = Ledger::load(&state);
            let error = bad.spent("a").unwrap_err();
            assert!(
                error.starts_with(&format!(
                    "admission ledger unreadable: {}: ",
                    ledger.path().display()
                )) && error.contains(why),
                "{text}: {error}"
            );
            assert!(bad.record("a", 2000).is_err());
            assert_eq!(std::fs::read_to_string(ledger.path()).unwrap(), text);
        }
    }
}
