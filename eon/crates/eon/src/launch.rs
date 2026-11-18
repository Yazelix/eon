//! Installed composition association; Runtime receives only validated opaque launch facts.

use eon_runtime::{LAUNCH_FACTS_LIMIT, LAUNCH_FACTS_VARIABLE, LaunchFacts, LaunchInputs};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

/// Caller must still be single-threaded, before any other library can start workers.
#[allow(unsafe_code)]
pub(crate) unsafe fn clear_capture() {
    use std::os::unix::ffi::OsStrExt;
    for (name, _) in std::env::vars_os() {
        if name
            .as_bytes()
            .starts_with(eon_runtime::LAUNCH_CAPTURE_PREFIX.as_bytes())
        {
            // SAFETY: process-wide removal has no safe API; the caller guarantees
            // single-threaded startup before any library can start workers.
            unsafe { std::env::remove_var(name) };
        }
    }
}

pub(crate) fn capture() -> Result<LaunchInputs, String> {
    let arguments = std::env::args_os().collect();
    let environment: Vec<_> = std::env::vars_os().collect();
    if !environment
        .iter()
        .any(|(name, _)| name == LAUNCH_FACTS_VARIABLE)
    {
        return Err("no installed launch capture; target is ineligible".into());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    capture_at(&executable, arguments, environment)
}

fn capture_at(
    executable: &Path,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
) -> Result<LaunchInputs, String> {
    let executable = fs::canonicalize(executable).map_err(|error| error.to_string())?;
    if executable.file_name() != Some(std::ffi::OsStr::new(".eon-wrapped")) {
        return Err("launch capture does not belong to an installed Eon executable".into());
    }
    let bin = executable
        .parent()
        .ok_or("missing installed bin directory")?;
    let wrapper = bin.join("eon");
    let metadata = fs::symlink_metadata(&wrapper).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("installed launch wrapper is not a regular executable".into());
    }
    let locator = bin
        .parent()
        .ok_or("missing installed root")?
        .join("share/eon/launch-environment");
    if environment
        .iter()
        .find(|(name, _)| name == LAUNCH_FACTS_VARIABLE)
        .map(|(_, value)| value.as_os_str())
        != Some(locator.as_os_str())
    {
        return Err("launch facts locator contradicts installed executable".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&locator)
        .map_err(|error| format!("cannot open installed launch facts: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("installed launch facts are not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((LAUNCH_FACTS_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let facts = LaunchFacts::decode(&bytes, &wrapper, &executable)?;
    LaunchInputs::capture(&facts, arguments, environment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_capture_refuses_a_foreign_locator_and_symlinked_facts() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};

        // Force an ancestor alias on every platform, as macOS has for temporary paths.
        let parent = tempfile::tempdir().unwrap();
        let alias = parent.path().join("alias");
        symlink(parent.path(), &alias).unwrap();
        let root = tempfile::tempdir_in(&alias).unwrap();
        let directory = root.path().canonicalize().unwrap();
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::create_dir_all(directory.join("share/eon")).unwrap();
        let executable = directory.join("bin/.eon-wrapped");
        fs::write(&executable, "raw").unwrap();
        let wrapper = directory.join("bin/eon");
        fs::write(&wrapper, "wrapper").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let locator = directory.join("share/eon/launch-environment");
        let arguments = || vec!["eon".into()];
        let env = |path: &Path| vec![(LAUNCH_FACTS_VARIABLE.into(), path.into())];
        assert!(capture_at(&executable, arguments(), env(Path::new("/foreign"))).is_err());
        assert!(capture_at(&executable, arguments(), env(&locator)).is_err());
        let mut table = b"EON-LAUNCH-1\0".to_vec();
        for path in [&wrapper, &executable] {
            table.extend_from_slice(path.as_os_str().as_bytes());
            table.push(0);
        }
        fs::write(directory.join("table"), &table).unwrap();
        symlink(directory.join("table"), &locator).unwrap();
        assert!(capture_at(&executable, arguments(), env(&locator)).is_err());
        fs::remove_file(&locator).unwrap();
        fs::write(&locator, table).unwrap();
        capture_at(&executable, arguments(), env(&locator)).unwrap();
    }
}
