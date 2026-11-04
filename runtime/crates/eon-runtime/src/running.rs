use super::{
    diagnostics::DiagnosticLog,
    generation::{
        RUNNING_PROBE_TIMEOUT, SupervisorInventory, generation_directory, live_supervisors,
        private_directory_exists,
    },
    supervisor::{effective_uid, supervisor_lock_path},
    windows::valid_window_id,
};
use eon_workspace_protocol::v32::{
    MAX_RUNNING_WINDOWS, RunningWindows, Snapshot, WindowEntry, WindowState,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};

const REFRESH: Duration = Duration::from_secs(1);
const MAX_WINDOWS: usize = 256;

pub(super) fn window_roots(
    base: &Path,
    log: Option<&DiagnosticLog>,
) -> Result<(Vec<PathBuf>, bool), String> {
    let mut roots = vec![base.to_path_buf()];
    if !private_directory_exists(base, "runtime directory")? {
        return Ok((roots, false));
    }
    let parent = base.join("w");
    if !private_directory_exists(&parent, "window directory")? {
        return Ok((roots, false));
    }
    let entries = fs::read_dir(&parent)
        .map_err(|error| format!("cannot list {}: {error}", parent.display()))?;
    let mut limited = false;
    for (index, entry) in entries.enumerate() {
        if index == MAX_WINDOWS {
            limited = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(log) = log {
                    log.note_once(&format!("skipping unreadable Eon window entry: {error}"));
                }
                continue;
            }
        };
        if entry.file_name().to_str().is_some_and(valid_window_id) {
            match private_directory_exists(&entry.path(), "window directory") {
                Ok(true) => roots.push(entry.path()),
                Ok(false) => {}
                Err(error) => {
                    if let Some(log) = log {
                        log.note_once(&format!("skipping unsafe Eon window entry: {error}"));
                    }
                }
            }
        }
    }
    roots[1..].sort();
    Ok((roots, limited))
}

/// An inspection never creates, protects or removes a lifecycle lock.
pub(super) fn supervisor_held(path: &Path) -> Result<bool, String> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != effective_uid() {
        return Err(format!(
            "{} must be an owned regular lifecycle lock",
            path.display()
        ));
    }
    match file.try_lock() {
        Ok(()) => {
            file.unlock().map_err(|error| error.to_string())?;
            Ok(false)
        }
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(error)) => {
            Err(format!("cannot inspect {}: {error}", path.display()))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    directories: Vec<(PathBuf, Option<SystemTime>)>,
    locks: Vec<(PathBuf, Option<bool>)>,
    limited: bool,
}

impl Fingerprint {
    pub(super) fn read(base: &Path, peers: &[(PathBuf, String)]) -> Result<Self, String> {
        let (roots, limited) = window_roots(base, None)?;
        let mut paths = vec![base.join("w")];
        paths.extend(roots.into_iter().map(|root| root.join("generations")));
        let mut locks = Vec::with_capacity(peers.len());
        for (root, generation) in peers {
            paths.push(generation_directory(root, generation));
            let lock = supervisor_lock_path(root, generation);
            locks.push((lock.clone(), supervisor_held(&lock).ok()));
        }
        let directories = paths
            .into_iter()
            .map(|path| {
                let modified = match fs::symlink_metadata(&path) {
                    Ok(metadata) => Some(metadata.modified().map_err(|error| error.to_string())?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => None,
                };
                Ok((path, modified))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            directories,
            locks,
            limited,
        })
    }
}

#[derive(Default)]
struct Cache {
    inventory: Option<SupervisorInventory>,
    fingerprint: Option<Fingerprint>,
}

fn complete_scan(
    cache: &Mutex<Cache>,
    log: &DiagnosticLog,
    result: Result<(SupervisorInventory, Fingerprint), String>,
) {
    match result {
        Ok((inventory, fingerprint)) => {
            *cache.lock().unwrap_or_else(|p| p.into_inner()) = Cache {
                inventory: Some(inventory),
                fingerprint: Some(fingerprint),
            }
        }
        Err(error) => {
            cache.lock().unwrap_or_else(|p| p.into_inner()).fingerprint = None;
            log.note_once(&format!("cannot refresh running Eon windows: {error}"));
        }
    }
}

/// How text for a person names a running window: its EON-C23 name, else its
/// window ID, else its generation.
pub(super) fn window_label(entry: &WindowEntry) -> String {
    entry
        .name
        .clone()
        .or_else(|| entry.window.clone())
        .unwrap_or_else(|| entry.generation.clone())
}

pub(super) fn running_notice(others: &[WindowEntry], omitted: u16) -> Option<String> {
    let count = others.len() + usize::from(omitted);
    if count == 0 {
        return None;
    }
    let mut labels = others
        .iter()
        .take(3)
        .map(|entry| match (&entry.name, entry.state) {
            (Some(name), _) => {
                let mut chars = name.chars();
                let mut label: String = chars.by_ref().take(24).collect();
                if chars.next().is_some() {
                    label.push('…');
                }
                label
            }
            (None, WindowState::Incompatible) => "another Eon build".into(),
            (None, WindowState::Unreachable) => "not answering".into(),
            (None, _) => window_label(entry),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = count - others.len().min(3);
    if more != 0 {
        if !labels.is_empty() {
            labels.push(' ');
        }
        labels.push_str(&format!("and {more} more"));
    }
    let windows = if count == 1 {
        "window is"
    } else {
        "windows are"
    };
    Some(format!(
        "{count} other Eon {windows} still running: {labels}\nSee them: eon workspace · Quit one: attach to it, then Commands > Quit this window"
    ))
}

pub(super) struct Running {
    base: PathBuf,
    runtime: PathBuf,
    next: Instant,
    last_tick: Instant,
    published: Option<(String, u16)>,
    sender: mpsc::SyncSender<()>,
    cache: Arc<Mutex<Cache>>,
    log: Arc<DiagnosticLog>,
}

impl Running {
    pub(super) fn start(
        base: PathBuf,
        root: &Path,
        generation: &str,
        log: Arc<DiagnosticLog>,
    ) -> Result<Self, String> {
        let cache = Arc::new(Mutex::new(Cache {
            inventory: None,
            fingerprint: Fingerprint::read(&base, &[]).ok(),
        }));
        let (sender, receiver) = mpsc::sync_channel(1);
        let result = Arc::clone(&cache);
        let scan_base = base.clone();
        let scan_root = root.to_path_buf();
        let scan_generation = generation.to_owned();
        let scan_log = Arc::clone(&log);
        thread::Builder::new()
            .name("eon-running-windows".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    complete_scan(
                        &result,
                        &scan_log,
                        live_supervisors(&scan_base, &scan_root, &scan_generation, &scan_log),
                    );
                }
            })
            .map_err(|error| format!("cannot start running Eon window worker: {error}"))?;
        let running = Self {
            base,
            runtime: generation_directory(root, generation),
            next: Instant::now() + REFRESH,
            last_tick: Instant::now(),
            published: None,
            sender,
            cache,
            log,
        };
        let _ = running.sender.try_send(());
        Ok(running)
    }

    pub(super) fn tick(&mut self, now: Instant, metadata: impl FnOnce() -> (String, u16)) {
        let resumed = now.saturating_duration_since(self.last_tick) >= RUNNING_PROBE_TIMEOUT;
        self.last_tick = now;
        if now < self.next {
            return;
        }
        self.next = now + REFRESH;
        let metadata = metadata();
        // Publish both metadata changes and readiness after a control operation blocked probes.
        if resumed || self.published.as_ref() != Some(&metadata) {
            match fs::File::open(&self.runtime)
                .and_then(|file| file.set_modified(SystemTime::now()))
            {
                Ok(()) => self.published = Some(metadata),
                Err(error) => self
                    .log
                    .note_once(&format!("cannot publish Eon window metadata: {error}")),
            }
        }
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        match Fingerprint::read(
            &self.base,
            cache
                .inventory
                .as_ref()
                .map_or(&[][..], |inventory| inventory.observed.as_slice()),
        ) {
            Ok(fingerprint) => {
                if fingerprint.limited {
                    self.log.note_once(
                        "running Eon window discovery is limited to 256 window directories",
                    );
                }
                if resumed || cache.fingerprint.as_ref() != Some(&fingerprint) {
                    let _ = self.sender.try_send(());
                    cache.fingerprint = Some(fingerprint);
                }
            }
            Err(error) => {
                cache.fingerprint = None;
                self.log
                    .note_once(&format!("cannot fingerprint running Eon windows: {error}"));
            }
        }
    }

    pub(super) fn apply(&self, snapshot: &mut Snapshot, this: WindowEntry) {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        snapshot.running = Some(RunningWindows {
            this,
            others: cache.inventory.as_ref().map(|inventory| {
                inventory
                    .entries
                    .iter()
                    .take(MAX_RUNNING_WINDOWS)
                    .cloned()
                    .collect()
            }),
            omitted: cache.inventory.as_ref().map_or(0, |entries| {
                entries
                    .entries
                    .len()
                    .saturating_sub(MAX_RUNNING_WINDOWS)
                    .min(u16::MAX as usize) as u16
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{lock_supervisor_lifecycle, temporary_directory};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    #[test]
    fn fifo_lifecycle_lock_does_not_block_inspection() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let base = temporary_directory();
        let fifo = base.join("lock");
        let path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = mpsc::channel();
        let probe_path = fifo.clone();
        let probe = thread::spawn(move || sender.send(supervisor_held(&probe_path)).unwrap());
        let answer = receiver.recv_timeout(Duration::from_secs(1));
        if answer.is_err() {
            // Release an old blocking reader before asserting, so the test always joins it.
            let _writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        }
        probe.join().unwrap();
        fs::remove_dir_all(base).unwrap();
        assert!(answer.expect("FIFO blocked lifecycle inspection").is_err());
    }

    #[test]
    fn window_directory_scan_is_bounded_and_reports_truncation() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        for index in 0..=MAX_WINDOWS {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(base.join("w").join(format!("{index:08x}")))
                .unwrap();
        }
        let (roots, limited) = window_roots(&base, None).unwrap();
        assert_eq!(roots.len(), 257);
        assert!(limited);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn running_notice_counts_labels_and_bounds() {
        let entry = WindowEntry {
            window: Some("0a1b2c3d".into()),
            generation: "g1".into(),
            name: Some("refs".into()),
            started_at: None,
            live_terminals: None,
            state: WindowState::Live,
        };
        let hint =
            "See them: eon workspace · Quit one: attach to it, then Commands > Quit this window";
        assert_eq!(running_notice(&[], 0), None);
        assert_eq!(
            running_notice(std::slice::from_ref(&entry), 0).unwrap(),
            format!("1 other Eon window is still running: refs\n{hint}")
        );
        let incompatible = WindowEntry {
            name: None,
            state: WindowState::Incompatible,
            ..entry.clone()
        };
        let unreachable = WindowEntry {
            state: WindowState::Unreachable,
            ..incompatible.clone()
        };
        assert_eq!(
            running_notice(&[entry.clone(), incompatible, unreachable], 0).unwrap(),
            format!(
                "3 other Eon windows are still running: refs, another Eon build, not answering\n{hint}"
            )
        );
        let long = WindowEntry {
            name: Some("é".repeat(25)),
            ..entry.clone()
        };
        assert_eq!(
            running_notice(&[long, entry.clone(), entry.clone(), entry.clone()], 2).unwrap(),
            format!(
                "6 other Eon windows are still running: {}…, refs, refs and 3 more\n{hint}",
                "é".repeat(24)
            )
        );
        let fallback = WindowEntry {
            name: None,
            ..entry
        };
        assert_eq!(
            running_notice(&[fallback], 0).unwrap(),
            format!("1 other Eon window is still running: 0a1b2c3d\n{hint}")
        );
        assert_eq!(
            running_notice(&[], 2).unwrap(),
            format!("2 other Eon windows are still running: and 2 more\n{hint}")
        );
    }

    #[test]
    fn window_label_prefers_name_then_window_then_generation() {
        let mut entry = WindowEntry {
            window: Some("0a1b2c3d".into()),
            generation: "g1".into(),
            name: Some("refs".into()),
            started_at: None,
            live_terminals: None,
            state: WindowState::Unreachable,
        };
        assert_eq!(window_label(&entry), "refs");
        entry.name = None;
        assert_eq!(window_label(&entry), "0a1b2c3d");
        entry.window = None;
        assert_eq!(window_label(&entry), "g1");
    }

    #[test]
    fn cached_summary_bounds_peers_and_unchanged_ticks_send_nothing() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let log = DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
        let this = WindowEntry {
            window: None,
            generation: "g1".into(),
            name: Some("test".into()),
            started_at: Some(1),
            live_terminals: Some(1),
            state: WindowState::Live,
        };
        let entries: Vec<_> = (0..40)
            .map(|index| WindowEntry {
                window: Some(format!("{index:08x}")),
                ..this.clone()
            })
            .collect();
        let observed: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    base.join("w").join(entry.window.as_ref().unwrap()),
                    entry.generation.clone(),
                )
            })
            .collect();
        let fingerprint = Fingerprint::read(&base, &observed).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let now = Instant::now();
        let mut running = Running {
            base: base.clone(),
            runtime: base.clone(),
            next: now,
            last_tick: now,
            published: Some(("test".into(), 1)),
            sender,
            cache: Arc::new(Mutex::new(Cache {
                inventory: Some(SupervisorInventory { entries, observed }),
                fingerprint: Some(fingerprint),
            })),
            log,
        };
        let mut snapshot = crate::workspace::WorkspaceSet::with_recovered_sessions(
            base.clone(),
            "/".into(),
            None,
            vec![(
                1,
                crate::workspace::Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            crate::fixtures::inputs().defaults.popups,
        )
        .unwrap()
        .snapshot();
        running.apply(&mut snapshot, this);
        let summary = snapshot.running.unwrap();
        assert_eq!(summary.others.unwrap().len(), MAX_RUNNING_WINDOWS);
        assert_eq!(summary.omitted, 8);
        fs::File::open(&base)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
        let modified = fs::metadata(&base).unwrap().modified().unwrap();
        let mut calls = 0;
        for tick in 0..100 {
            running.tick(now + Duration::from_millis(tick * 25), || {
                calls += 1;
                ("test".into(), 1)
            });
        }
        assert_eq!(calls, 3, "metadata work escaped the one-second gate");
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(fs::metadata(&base).unwrap().modified().unwrap(), modified);
        complete_scan(
            &running.cache,
            &running.log,
            Err("temporary scan failure".into()),
        );
        assert_eq!(
            running
                .cache
                .lock()
                .unwrap()
                .inventory
                .as_ref()
                .unwrap()
                .entries
                .len(),
            40
        );
        running.tick(now + Duration::from_secs(3), || ("test".into(), 1));
        assert!(
            receiver.try_recv().is_ok(),
            "failed scan was not retried at the next gate"
        );
        running.tick(now + Duration::from_secs(5), || ("test".into(), 1));
        assert!(
            receiver.try_recv().is_ok(),
            "resuming the control loop did not refresh stale peer health"
        );
        assert_ne!(fs::metadata(&base).unwrap().modified().unwrap(), modified);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn fingerprint_changes_on_window_start_end_terminal_churn_and_released_lock_only() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let before = Fingerprint::read(&base, &[]).unwrap();
        let root = base.join("w/1234abcd");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join("generations"))
            .unwrap();
        let started = Fingerprint::read(&base, &[]).unwrap();
        assert_ne!(before, started);
        let generation = "g1-11111111111111111111111111111111";
        let directory = generation_directory(&root, generation);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let lock = lock_supervisor_lifecycle(&supervisor_lock_path(&root, generation)).unwrap();
        let peers = [(root.clone(), generation.to_owned())];
        let held = Fingerprint::read(&base, &peers).unwrap();
        assert_eq!(held, Fingerprint::read(&base, &peers).unwrap());
        fs::write(directory.join("session-1.sock"), b"one").unwrap();
        fs::File::open(&directory)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
        let churn = Fingerprint::read(&base, &peers).unwrap();
        assert_ne!(held, churn);
        fs::write(directory.join("session-1.sock"), b"two").unwrap();
        assert_eq!(churn, Fingerprint::read(&base, &peers).unwrap());
        fs::remove_file(directory.join("session-1.sock")).unwrap();
        fs::File::open(&directory)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(2))
            .unwrap();
        let ended_terminal = Fingerprint::read(&base, &peers).unwrap();
        assert_ne!(churn, ended_terminal);
        lock.unlock().unwrap();
        let released = Fingerprint::read(&base, &peers).unwrap();
        assert_ne!(ended_terminal, released);
        assert_eq!(released, Fingerprint::read(&base, &peers).unwrap());
        fs::remove_dir(&directory).unwrap();
        assert_ne!(released, Fingerprint::read(&base, &peers).unwrap());
        fs::remove_dir_all(base).unwrap();
    }
}
