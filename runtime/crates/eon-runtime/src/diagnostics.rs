//! The supervisor's persistent diagnostic log. Desktop launchers may discard the
//! supervisor's stderr, which Eon Desktop inherits, so its exit and panic output
//! needs a home that outlives the window.

use super::{
    generation::valid_generation, managed_environment::nonempty_environment_path,
    supervisor::effective_uid,
};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process,
    sync::{Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

const RETAINED_LOGS: usize = 20;
const RETAINED_ORBIT_LOGS: usize = 100;
const LOG_BYTES: u64 = 1 << 20;
/// Copied output stops this far below the bound, so a full log still records
/// how Eon Desktop and the supervisor ended.
const RECORD_RESERVE: u64 = 64 << 10;

pub(super) struct DiagnosticLog {
    sink: Mutex<Option<Sink>>,
    /// Where this supervisor's Orbit hosts keep their stderr: the `orbit`
    /// directory inside the log's own, which the log prune never enters.
    orbit_directory: Result<PathBuf, String>,
    /// The messages `note_once` has already noted.
    noted: Mutex<HashSet<String>>,
}

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
        let orbit_directory = directory
            .as_ref()
            .map(|directory| directory.join("orbit"))
            .map_err(Clone::clone);
        let sink = directory
            .and_then(|directory| create(&directory, generation))
            .inspect_err(|error| {
                let _ = writeln!(
                    io::stderr(),
                    "eon: warning: {error}; continuing without a diagnostic log"
                );
            })
            .ok();
        let log = Arc::new(Self {
            sink: Mutex::new(sink),
            orbit_directory,
            noted: Mutex::default(),
        });
        log.record(&format!(
            "supervisor started: generation {generation}, {mode} mode, pid {}",
            process::id()
        ));
        log
    }

    pub(super) fn orbit_directory(&self) -> Result<&Path, &str> {
        self.orbit_directory.as_deref().map_err(String::as_str)
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

    /// Notes `message` unless this log already noted it, so a diagnostic that every
    /// reread of the same input repeats appears once per supervisor.
    pub(super) fn note_once(&self, message: &str) {
        let first = self
            .noted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(message.to_owned());
        if first {
            self.note(message);
        }
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
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
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

fn prepare_directory(directory: &Path) -> Result<(), String> {
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
        .map_err(|error| format!("cannot protect {}: {error}", directory.display()))
}

/// The stderr log of one Orbit run in `directory`.
pub(super) fn orbit_log_path(
    directory: &Path,
    component_generation: &str,
    session_id: &str,
    run_id: &str,
) -> PathBuf {
    directory.join(format!("{component_generation}-{session_id}-{run_id}.log"))
}

/// The generation whose Orbit log names `caller`, an `EON_TERMINAL` value, as
/// `orbit_log_path` named it: the generation that ran that terminal, while
/// its log remains.
pub(super) fn orbit_run_generation(caller: &str) -> Option<String> {
    let (session, run) = caller.split_once('.')?;
    let suffix = format!("-{session}-{run}.log");
    let logs = state_directory()?.join("logs").join("orbit");
    fs::read_dir(logs).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().into_string().ok()?;
        let generation = name.strip_suffix(&suffix)?;
        valid_generation(generation).then(|| generation.to_owned())
    })
}

/// The suffix a failure line carries to name an Orbit log that exists.
pub(super) fn orbit_log_suffix(path: Option<&Path>) -> String {
    path.filter(|path| path.exists())
        .map(|path| format!("; Orbit log: {}", path.display()))
        .unwrap_or_default()
}

pub(super) fn open_orbit_log(
    directory: &Path,
    component_generation: &str,
    session_id: &str,
    run_id: &str,
) -> Result<(File, PathBuf), String> {
    prepare_directory(directory)?;
    let path = orbit_log_path(directory, component_generation, session_id, run_id);
    let file = fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    // Orbit inherits this open file description, so its lock outlives the spawn.
    file.try_lock()
        .map_err(|error| format!("cannot lock {}: {error}", path.display()))?;
    prune_orbit(directory, RETAINED_ORBIT_LOGS);
    Ok((file, path))
}

fn create(directory: &Path, generation: &str) -> Result<Sink, String> {
    prepare_directory(directory)?;
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

/// Deletes all but the newest `keep` ordered logs, skipping logs a live process holds.
fn prune_by<K: Ord>(
    directory: &Path,
    keep: usize,
    mut order: impl FnMut(&fs::DirEntry) -> Option<K>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut logs = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| Some((order(&entry)?, entry.path())))
        .collect::<Vec<_>>();
    logs.sort_unstable_by(|(left, _), (right, _)| right.cmp(left));
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

/// Deletes all but the newest `keep` Eon logs, skipping logs a live supervisor holds.
fn prune(directory: &Path, keep: usize) {
    prune_by(directory, keep, |entry| {
        let (_, seconds, pid) = log_fields(entry.file_name().to_str()?)?;
        Some((seconds, pid))
    });
}

/// The generation, start second and PID that an Eon log's name records.
fn log_fields(name: &str) -> Option<(&str, u64, u32)> {
    let mut fields = name.strip_suffix(".log")?.rsplitn(3, '-');
    let pid = fields.next()?.parse().ok()?;
    let seconds = fields.next()?.parse().ok()?;
    let generation = fields
        .next()
        .filter(|generation| valid_generation(generation))?;
    Some((generation, seconds, pid))
}

/// The newest of the Eon logs `names` that `generation` wrote, else the newest of all.
fn newest_log_name<'a>(names: &[&'a str], generation: Option<&str>) -> Option<&'a str> {
    let newest = |of: Option<&str>| {
        names
            .iter()
            .filter_map(|name| Some((log_fields(name)?, *name)))
            .filter(|((written, ..), _)| of.is_none_or(|of| of == *written))
            .max_by_key(|((_, seconds, pid), _)| (*seconds, *pid))
            .map(|(_, name)| name)
    };
    generation
        .and_then(|generation| newest(Some(generation)))
        .or_else(|| newest(None))
}

/// The last `lines` lines of `text`.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
}

/// The newest supervisor log of `generation`, else the newest of any, with its last
/// `lines` lines. It reads the log directory without creating or pruning anything.
pub(super) fn newest_log(
    generation: Option<&str>,
    lines: usize,
) -> Result<(PathBuf, String), String> {
    let directory = state_directory()
        .map(|state| state.join("logs"))
        .ok_or("neither XDG_STATE_HOME nor HOME is an absolute path")?;
    let entries = fs::read_dir(&directory)
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
    let names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let path = directory.join(
        newest_log_name(&names, generation)
            .ok_or_else(|| format!("no supervisor log in {}", directory.display()))?,
    );
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(length.saturating_sub(LOG_BYTES)))
        .and_then(|_| file.take(LOG_BYTES).read_to_end(&mut bytes))
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text = tail(&String::from_utf8_lossy(&bytes), lines);
    Ok((path, text))
}

fn prune_orbit(directory: &Path, keep: usize) {
    prune_by(directory, keep, |entry| {
        let name = entry.file_name();
        if !name.to_str()?.ends_with(".log") {
            return None;
        }
        entry.metadata().ok()?.modified().ok()
    });
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

/// Opt-in benchmark diagnostics; never waits for a reader or writes the pressured disk.
// The Venus and Orbit copies mirror this format at their own ownership boundaries.
#[doc(hidden)]
pub fn bench_trace(stage: &str, detail: impl std::fmt::Display) {
    if !bench_tracing() {
        return;
    }
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static SOCKET: OnceLock<Option<std::os::unix::net::UnixDatagram>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static LOST: AtomicU64 = AtomicU64::new(0);
    let socket = SOCKET.get_or_init(|| {
        let path = std::env::var_os("EON_BENCH_TRACE")?;
        let socket = std::os::unix::net::UnixDatagram::unbound().ok()?;
        socket.set_nonblocking(true).ok()?;
        socket.connect(path).ok()?;
        Some(socket)
    });
    let Some(socket) = socket else { return };
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is a valid timespec and CLOCK_MONOTONIC has no input state.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut time) } != 0 {
        return;
    }
    let nanos = time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64;
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let packet = format!(
        "runtime\t{}\t{sequence}\t{nanos}\t{}\t{stage}\t{detail}",
        std::process::id(),
        LOST.load(Ordering::Relaxed)
    );
    if packet.len() > 4096 || socket.send(packet.as_bytes()).is_err() {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}

/// The final event lets the benchmark reject incomplete diagnostics.
#[doc(hidden)]
pub struct BenchTrace;
impl BenchTrace {
    #[doc(hidden)]
    pub fn start() -> Self {
        bench_trace("start", "");
        Self
    }
}
impl Drop for BenchTrace {
    fn drop(&mut self) {
        bench_trace("end", "");
    }
}

#[doc(hidden)]
pub fn bench_tracing() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("EON_BENCH_TRACE").is_some())
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticLog, LOG_BYTES, create, newest_log_name, prune, tail};
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
        // Unlock, not only drop: a child forked meanwhile by a parallel test shares this open file
        // description until it execs, and the lock would last as long (eon-prune-test-flake-sj0c).
        create(&logs, GENERATION).unwrap().file.unlock().unwrap();
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
    fn report_reads_the_newest_log_of_the_generation_else_of_any() {
        const OTHER: &str = "g1-fedcba9876543210fedcba9876543210";
        let names = [
            format!("{GENERATION}-100-7.log"),
            format!("{GENERATION}-100-9.log"),
            format!("{GENERATION}-99-50.log"),
            format!("{OTHER}-200-1.log"),
            "notes.log".to_string(),
            format!("{GENERATION}-300-x.log"),
        ];
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let pick = |generation| newest_log_name(&names, generation);
        assert_eq!(
            pick(Some(GENERATION)),
            Some(format!("{GENERATION}-100-9.log").as_str())
        );
        // A generation without logs, and no generation, take the newest of any.
        let newest = format!("{OTHER}-200-1.log");
        assert_eq!(
            pick(Some("g1-00000000000000000000000000000000")),
            Some(newest.as_str())
        );
        assert_eq!(pick(None), Some(newest.as_str()));
        assert_eq!(newest_log_name(&["notes.log"], None), None);

        assert_eq!(tail("a\nb\nc\n", 2), "b\nc\n");
        assert_eq!(tail("a\nb", 5), "a\nb\n");
        assert_eq!(tail("", 5), "");
    }

    #[test]
    fn copied_output_stops_below_the_bound_and_records_still_fit() {
        let root = crate::supervisor::temporary_directory();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), GENERATION, "eon");
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
