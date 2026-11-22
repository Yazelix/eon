//! The bench store: a directory of small text files.
//!
//! - `configs/`, `judges/`, `tasks/`, `pooling/`: one file per record or declaration, named
//!   `<hash>.json` and holding exactly the canonical bytes that were hashed (no trailing
//!   newline), so `sha256sum` of a file equals its name.
//! - `results.jsonl`, `grades.jsonl`: append-only rows ([`crate::rows`]).
//! - `.lock`: every append and declaration holds an exclusive lock on it, so two bench
//!   sessions cannot race a check against a write.

use crate::record::{self, Hashed, Kind, Record};
use crate::{Result, fail, sha256_hex};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validates, hashes and stores a record; storing an identical record again is a no-op.
    /// Historical `eon-bench-config/1` records are read-only and refused here.
    pub fn put(&self, input: &[u8]) -> Result<Hashed> {
        let hashed = record::hash(input)?;
        if let Record::Config(ref config) = hashed.record
            && config.is_v1()
        {
            fail!(
                "{} is read-only; new configuration records must use {}",
                record::CONFIG_SCHEMA_V1,
                record::CONFIG_SCHEMA
            );
        }
        self.write_canonical(hashed.kind().dir(), &hashed.hash, &hashed.canonical)?;
        Ok(hashed)
    }

    /// Writes `canonical` as `<dir>/<hash>.json`, atomically.
    pub(crate) fn write_canonical(&self, dir: &str, hash: &str, canonical: &str) -> Result<()> {
        let dir = self.root.join(dir);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{hash}.json"));
        if path.exists() {
            if fs::read(&path)? != canonical.as_bytes() {
                fail!("{} exists with other content", path.display());
            }
            return Ok(());
        }
        crate::write_atomic(&path, canonical.as_bytes())
    }

    /// Reads `<dir>/<hash>.json` and checks that its bytes hash to its name.
    pub(crate) fn read_canonical(&self, dir: &str, hash: &str) -> Result<Vec<u8>> {
        let path = self.root.join(dir).join(format!("{hash}.json"));
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fail!(
                    "no {} record {hash} in {}",
                    dir.trim_end_matches('s'),
                    self.root.display()
                )
            }
            Err(error) => fail!("{}: {error}", path.display()),
        };
        if sha256_hex(&bytes) != hash {
            fail!("{}: content does not hash to its name", path.display());
        }
        Ok(bytes)
    }

    /// Loads a stored record of `kind`, re-validating it.
    pub fn get(&self, kind: Kind, hash: &str) -> Result<Hashed> {
        let hashed = record::hash(&self.read_canonical(kind.dir(), hash)?)?;
        // The bytes hash to the name, so an equal recomputed hash means they are canonical.
        if hashed.kind() != kind || hashed.hash != hash {
            fail!("{}/{hash}.json is not a {} record", kind.dir(), kind.name());
        }
        Ok(hashed)
    }

    pub fn config(&self, hash: &str) -> Result<record::Config> {
        match self.get(Kind::Config, hash)?.record {
            Record::Config(config) => Ok(config),
            Record::Task(_) => unreachable!("kind checked"),
        }
    }

    pub fn judge(&self, hash: &str) -> Result<record::Config> {
        match self.get(Kind::Judge, hash)?.record {
            Record::Config(config) => Ok(config),
            Record::Task(_) => unreachable!("kind checked"),
        }
    }

    pub fn task(&self, hash: &str) -> Result<record::Task> {
        match self.get(Kind::Task, hash)?.record {
            Record::Task(task) => Ok(task),
            Record::Config(_) => unreachable!("kind checked"),
        }
    }

    /// Resolves a full hash or a unique hex prefix among the stored records of `kinds`.
    pub fn resolve(&self, kinds: &[Kind], prefix: &str) -> Result<(Kind, String)> {
        if prefix.is_empty() || !crate::is_lower_hex(prefix) {
            fail!("{prefix:?} is not a lowercase hex hash or prefix");
        }
        let mut found = Vec::new();
        for &kind in kinds {
            for hash in self.hashes(kind.dir())? {
                if hash.starts_with(prefix) {
                    found.push((kind, hash));
                }
            }
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => fail!(
                "no {} record matches {prefix}",
                kinds
                    .iter()
                    .map(|k| k.name())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
            _ => fail!("{prefix} is ambiguous: {} records match", found.len()),
        }
    }

    /// The hashes stored in `dir`, sorted.
    pub(crate) fn hashes(&self, dir: &str) -> Result<Vec<String>> {
        let dir = self.root.join(dir);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => fail!("{}: {error}", dir.display()),
        };
        let mut hashes = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if let Some(hash) = name.strip_suffix(".json")
                && !hash.starts_with('.')
            {
                hashes.push(hash.to_owned());
            }
        }
        hashes.sort();
        Ok(hashes)
    }

    /// Holds the store's exclusive lock until the returned file is dropped.
    pub(crate) fn lock(&self) -> Result<File> {
        fs::create_dir_all(&self.root)?;
        crate::lock(&self.root.join(".lock"))
    }

    /// Reads a JSONL file's lines (missing file: none).
    pub(crate) fn lines(&self, name: &str) -> Result<Vec<String>> {
        match fs::read_to_string(self.root.join(name)) {
            Ok(text) => Ok(text.lines().map(str::to_owned).collect()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => fail!("{name}: {error}"),
        }
    }

    /// Appends one line with a single write. Callers hold [`Store::lock`].
    pub(crate) fn append_line(&self, name: &str, line: &str) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(name))?;
        file.write_all(format!("{line}\n").as_bytes())?;
        file.sync_data()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::{CONFIG, CONFIG_V1, JUDGE, JUDGE_V1, TASK};

    #[test]
    fn records_are_stored_as_their_canonical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        for (input, kind) in [
            (CONFIG, Kind::Config),
            (JUDGE, Kind::Judge),
            (TASK, Kind::Task),
        ] {
            let stored = store.put(input.as_bytes()).unwrap();
            assert_eq!(stored.kind(), kind);
            let path = dir
                .path()
                .join(kind.dir())
                .join(format!("{}.json", stored.hash));
            assert_eq!(fs::read_to_string(&path).unwrap(), stored.canonical);
            assert_eq!(store.get(kind, &stored.hash).unwrap(), stored);
            // Storing again is a no-op.
            assert_eq!(store.put(input.as_bytes()).unwrap(), stored);
        }
        // A tampered file no longer matches its name.
        let config = store.put(CONFIG.as_bytes()).unwrap();
        let path = dir
            .path()
            .join("configs")
            .join(format!("{}.json", config.hash));
        fs::write(&path, config.canonical.replace("high", "low")).unwrap();
        assert!(store.get(Kind::Config, &config.hash).is_err());
    }

    #[test]
    fn prefixes_resolve_when_unique() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let config = store.put(CONFIG.as_bytes()).unwrap();
        let judge = store.put(JUDGE.as_bytes()).unwrap();
        let kinds = [Kind::Config, Kind::Judge];
        assert_eq!(
            store.resolve(&kinds, &config.hash[..12]).unwrap(),
            (Kind::Config, config.hash.clone())
        );
        assert_eq!(
            store.resolve(&kinds, &judge.hash).unwrap(),
            (Kind::Judge, judge.hash.clone())
        );
        assert!(store.resolve(&[Kind::Task], &config.hash[..12]).is_err());
        assert!(store.resolve(&kinds, "XYZ").is_err());
        // Plant a second config sharing the first one's leading digit: the digit alone is
        // ambiguous.
        let twin = format!("{}{}", &config.hash[..1], "0".repeat(63));
        fs::write(
            dir.path().join("configs").join(format!("{twin}.json")),
            "{}",
        )
        .unwrap();
        assert!(store.resolve(&kinds, &config.hash[..1]).is_err());
    }

    #[test]
    fn v1_configs_are_readable_but_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        for input in [CONFIG_V1, JUDGE_V1] {
            let error = store.put(input.as_bytes()).unwrap_err().0;
            assert!(error.contains("eon-bench-config/1"), "{error}");
            assert!(error.contains("eon-bench-config/2"), "{error}");
        }
        // Historical `/1` files already on disk stay readable for comparison and `config diff`.
        let config_v1 = record::hash(CONFIG_V1.as_bytes()).unwrap();
        let judge_v1 = record::hash(JUDGE_V1.as_bytes()).unwrap();
        store
            .write_canonical("configs", &config_v1.hash, &config_v1.canonical)
            .unwrap();
        store
            .write_canonical("judges", &judge_v1.hash, &judge_v1.canonical)
            .unwrap();
        assert_eq!(store.get(Kind::Config, &config_v1.hash).unwrap(), config_v1);
        assert_eq!(store.get(Kind::Judge, &judge_v1.hash).unwrap(), judge_v1);
        assert!(store.config(&config_v1.hash).unwrap().is_v1());
        assert!(store.judge(&judge_v1.hash).unwrap().is_v1());
        let kinds = [Kind::Config, Kind::Judge];
        assert_eq!(
            store.resolve(&kinds, &config_v1.hash[..12]).unwrap(),
            (Kind::Config, config_v1.hash)
        );
        assert_eq!(
            store.resolve(&kinds, &judge_v1.hash[..12]).unwrap(),
            (Kind::Judge, judge_v1.hash)
        );
    }
}
