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
    pub fn put(&self, input: &[u8]) -> Result<Hashed> {
        let hashed = record::hash(input)?;
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
        let temporary = dir.join(format!(".{hash}.{}.tmp", std::process::id()));
        fs::write(&temporary, canonical)?;
        fs::rename(&temporary, &path)?;
        Ok(())
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
        if prefix.is_empty()
            || !prefix
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
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
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        file.lock()?;
        Ok(file)
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
    use crate::record::tests::{CONFIG, JUDGE, TASK};

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
}
