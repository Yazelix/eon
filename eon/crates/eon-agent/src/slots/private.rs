//! Owner-only slot state: directories and small records no other account can swap or read.
//! `noun` names the state's owner in error tokens (`unsafe-{noun}-state-directory`, ...).
use crate::launch::ensure_parent;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Creates `path` if absent, then checks it is an owner-only directory of this user.
pub fn directory(path: &Path, noun: &str) -> Result<(), String> {
    ensure_parent(&path.join("entry")).map_err(|_| format!("{noun}-state-directory"))?;
    check(path, noun)
}

/// Checks without creating: `path` must be a directory owned by this user with no group or
/// other permission bits.
pub fn check(path: &Path, noun: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{noun}-state-directory"))?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(format!("unsafe-{noun}-state-directory"));
    }
    Ok(())
}

/// Reads a small owner-only JSON record; `None` when it does not exist.
pub fn read<T: serde::de::DeserializeOwned>(path: &Path, noun: &str) -> Result<Option<T>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Ok(meta)
            if meta.is_file()
                && meta.uid() == rustix::process::geteuid().as_raw()
                && meta.mode() & 0o077 == 0
                && meta.len() <= 65536 => {}
        _ => return Err(format!("unsafe-or-unreadable-{noun}-record")),
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| format!("{noun}-record-read"))?)
        .map(Some)
        .map_err(|_| format!("{noun}-record-format"))
}
