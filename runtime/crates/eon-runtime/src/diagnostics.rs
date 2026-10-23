//! The supervisor's persistent diagnostic log. Desktop launchers may discard the
//! supervisor's stderr, which Eon Desktop inherits, so its exit and panic output
//! needs a home that outlives the window.

use super::{
    generation::valid_generation, managed_environment::nonempty_environment_path,
    supervisor::effective_uid,
};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process,
    sync::{Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

const RETAINED_LOGS: usize = 20;
const LOG_BYTES: u64 = 1 << 20;
/// Copied output stops this far below the bound, so a full log still records
/// how Eon Desktop and the supervisor ended.
const RECORD_RESERVE: u64 = 64 << 10;

pub(super) struct DiagnosticLog(Mutex<Option<Sink>>);

struct Sink {
    file: File,
    written: u64,
    truncated: bool,
}

impl DiagnosticLog {
    /// Opens this supervisor's log in `$XDG_STATE_HOME/eon/logs`; a log that cannot
    /// be created is reported on stderr and leaves the supervisor without one.
    pub(super) fn open(generation: &str, mode: &str) -> Arc<Self> {
        let directory = state_directory()
            .map(|state| state.join("logs"))
            .ok_or_else(|| "neither XDG_STATE_HOME nor HOME is an absolute path".to_string());
        Self::open_in(directory, generation, mode)
    }

    pub(super) fn open_in(
        directory: Result<PathBuf, String>,
        generation: &str,
        mode: &str,
    ) -> Arc<Self> {
        let sink = directory
            .and_then(|directory| create(&directory, generation))
            .inspect_err(|error| {
                let _ = writeln!(
                    io::stderr(),
                    "eon: warning: {error}; continuing without a diagnostic log"
                );
            })
            .ok();
        let log = Arc::new(Self(Mutex::new(sink)));
        log.record(&format!(
            "supervisor started: generation {generation}, {mode} mode, pid {}",
            process::id()
        ));
        log
    }

    /// Appends one timestamped line.
    pub(super) fn record(&self, message: &str) {
        self.append(format!("{} {message}\n", timestamp()).as_bytes(), true);
    }

    /// Records a supervisor diagnostic and prints it on stderr.
    pub(super) fn note(&self, message: &str) {
        self.record(message);
        let _ = writeln!(io::stderr(), "{message}");
    }

    /// Copies a child's stderr verbatim to the log and to the supervisor's
    /// stderr until every writer closes it.
    pub(super) fn relay(
        self: &Arc<Self>,
        mut stderr: impl Read + Send + 'static,
    ) -> io::Result<JoinHandle<()>> {
        let log = Arc::clone(self);
        thread::Builder::new()
            .name("eon-diagnostic-relay".into())
            .spawn(move || {
                let mut buffer = [0; 8192];
                loop {
                    match stderr.read(&mut buffer) {
                        Ok(0) => return,
                        Ok(count) => {
                            log.append(&buffer[..count], false);
                            let _ = io::stderr().write_all(&buffer[..count]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => return,
                    }
                }
            })
    }

    fn append(&self, bytes: &[u8], record: bool) {
        let mut sink = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sink) = sink.as_mut() else {
            return;
        };
        if !record && sink.truncated {
            return;
        }
        let limit = if record {
            LOG_BYTES
        } else {
            LOG_BYTES - RECORD_RESERVE
        };
        if sink.written + bytes.len() as u64 <= limit {
            if sink.file.write_all(bytes).is_ok() {
                sink.written += bytes.len() as u64;
            }
        } else if !record {
            sink.truncated = true;
            let marker = format!(
                "\n{} copied output reached the log bound; later output is dropped\n",
                timestamp()
            );
            if sink.written + marker.len() as u64 <= LOG_BYTES
                && sink.file.write_all(marker.as_bytes()).is_ok()
            {
                sink.written += marker.len() as u64;
            }
        }
    }
}

/// Eon's directory in `$XDG_STATE_HOME`, or in `~/.local/state` without it.
pub(super) fn state_directory() -> Option<PathBuf> {
    nonempty_environment_path("XDG_STATE_HOME")
        .filter(|path| path.is_absolute())
        .or_else(|| {
            nonempty_environment_path("HOME")
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })
        .map(|state| state.join("eon"))
}

fn create(directory: &Path, generation: &str) -> Result<Sink, String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("cannot inspect {}: {error}", directory.display()))?;
    if !metadata.file_type().is_dir() || metadata.uid() != effective_uid() {
        return Err(format!(
            "{} must be an owned directory",
            directory.display()
        ));
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("cannot protect {}: {error}", directory.display()))?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = directory.join(format!("{generation}-{seconds}-{}.log", process::id()));
    let file = fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    // The lock marks the log live, so no supervisor's pruning removes it.
    let _ = file.try_lock();
    prune(directory, RETAINED_LOGS);
    Ok(Sink {
        file,
        written: 0,
        truncated: false,
    })
}

/// Deletes all but the newest `keep` Eon logs, skipping logs a live supervisor holds.
fn prune(directory: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut logs = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name();
            let mut fields = name.to_str()?.strip_suffix(".log")?.rsplitn(3, '-');
            let pid = fields.next()?.parse::<u32>().ok()?;
            let seconds = fields.next()?.parse::<u64>().ok()?;
            fields
                .next()
                .filter(|generation| valid_generation(generation))?;
            Some(((seconds, pid), entry.path()))
        })
        .collect::<Vec<_>>();
    logs.sort_unstable_by_key(|(order, _)| std::cmp::Reverse(*order));
    for (_, path) in logs.into_iter().skip(keep) {
        let finished = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .is_ok_and(|file| file.try_lock().is_ok());
        if finished {
            let _ = fs::remove_file(path);
        }
    }
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as libc::time_t;
    // SAFETY: tm is plain data that localtime_r fills; it reports failure as null.
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
        return format!("@{seconds}");
    }
    let format = |pattern: &std::ffi::CStr| {
        let mut buffer = [0u8; 32];
        // SAFETY: strftime writes at most buffer.len() bytes and returns how many.
        let length = unsafe {
            libc::strftime(
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                pattern.as_ptr(),
                &tm,
            )
        };
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    };
    format!(
        "{}.{:03}{}",
        format(c"%Y-%m-%dT%H:%M:%S"),
        now.subsec_millis(),
        format(c"%z")
    )
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticLog, LOG_BYTES, create, prune};
    use std::{
        fs::{self, File},
        os::unix::fs::{PermissionsExt, symlink},
        path::Path,
    };

    const GENERATION: &str = "g1-0123456789abcdef0123456789abcdef";

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn pruning_keeps_the_newest_and_live_logs_only() {
        let root = crate::supervisor::temporary_directory();
        let logs = root.join("logs");
        drop(create(&logs, GENERATION).unwrap());
        let own = format!("-{}.log", std::process::id());
        // Fixtures carry another PID, so none can pass for the log create() made.
        let pid = std::process::id() + 1;
        let name = |seconds: u64| format!("{GENERATION}-{seconds}-{pid}.log");
        for seconds in 1..=4 {
            File::create(logs.join(name(seconds))).unwrap();
        }
        let live = File::open(logs.join(name(1))).unwrap();
        live.try_lock().unwrap();
        File::create(root.join("target")).unwrap();
        symlink(root.join("target"), logs.join(name(0))).unwrap();
        let foreign = [
            "notes.log".to_string(),
            format!("g1-abc-2-{pid}.log"),
            format!("grocery-1-{pid}.log"),
            format!("{GENERATION}-x-{pid}.log"),
        ];
        for name in &foreign {
            File::create(logs.join(name)).unwrap();
        }
        let left = || {
            let mut names = fs::read_dir(&logs)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect::<Vec<_>>();
            names.sort();
            names
        };

        prune(&logs, 2);

        let mut expected = [name(0), name(1), name(4)]
            .into_iter()
            .chain(foreign.iter().cloned())
            .collect::<Vec<_>>();
        expected.sort();
        let (created, others): (Vec<_>, Vec<_>) = left()
            .into_iter()
            .partition(|name| name.ends_with(&own) && name.starts_with(GENERATION));
        assert_eq!(created.len(), 1);
        assert_eq!(others, expected);
        assert_eq!(mode(&logs), 0o700);
        assert_eq!(mode(&logs.join(&created[0])), 0o600);

        prune(&logs, 0);

        assert!(!left().iter().any(|name| name.ends_with(&own)));
        assert!(logs.join(name(1)).exists());
        assert!(!logs.join(name(4)).exists());
        assert!(root.join("target").exists());
        drop(live);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copied_output_stops_below_the_bound_and_records_still_fit() {
        let root = crate::supervisor::temporary_directory();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), GENERATION, "workspace");
        let chunk = vec![b'x'; 8192];
        for _ in 0..(LOG_BYTES / 8192 + 2) {
            log.append(&chunk, false);
        }
        log.append(b"late copied output", false);
        log.record("Eon Desktop exited: signal: 6 (SIGABRT)");
        let path = fs::read_dir(root.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains(&format!("supervisor started: generation {GENERATION}"))
        );
        assert_eq!(
            text.matches("copied output reached the log bound").count(),
            1
        );
        assert!(!text.contains("late copied output"));
        assert!(
            text.trim_end()
                .ends_with("Eon Desktop exited: signal: 6 (SIGABRT)")
        );

        for _ in 0..1024 {
            log.record(&"y".repeat(1000));
        }
        assert!(fs::metadata(&path).unwrap().len() <= LOG_BYTES);
        fs::remove_dir_all(root).unwrap();
    }
}
