//! Scrollback's private snapshots (EON-C10). Each press writes the shown
//! terminal's ORB-C17 history to a mode-0600 file in a fresh mode-0700
//! directory under `snapshots/` in the window's private runtime directory,
//! and the popup's editor opens that file. The directory goes when the
//! popup's terminal ends.
//!
//! A directory is named `<pid>-<start>-<n>` for the supervisor that wrote it,
//! and records in `.terminal` the terminal its popup starts as, before that
//! terminal's editor starts. A starting supervisor removes the directories
//! of supervisors no longer running, except one whose recorded terminal it
//! adopts in recovery: it takes that one over and removes it when the
//! terminal ends. A removal that fails is retried on later turns.

use super::{
    diagnostics::DiagnosticLog,
    platform,
    supervisor::{prepare_runtime, validate_private_directory},
};
use orbit_protocol::management::{HistoryText, Screen};
use std::{
    fs, io,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// The first line of a snapshot of a terminal on the alternate screen.
const ALTERNATE_NOTICE: &str = "This pane is in a full-screen app; Eon shows only its screen. Quit the app to see the history.";

/// The file naming the terminal whose popup owns a snapshot directory.
const OWNER: &str = ".terminal";

/// An Orbit terminal ID is `session-<n>`, well under this.
const MAX_OWNER_BYTES: u64 = 64;

/// One written snapshot whose popup has not started yet.
pub(crate) struct Snapshot {
    directory: PathBuf,
    pub(crate) file: PathBuf,
}

impl Snapshot {
    /// Records `terminal` as the popup's, before that terminal starts, so a
    /// recovery that adopts its editor keeps the snapshot.
    pub(crate) fn mark(&self, terminal: &str) -> Result<(), String> {
        let path = self.directory.join(OWNER);
        create_private(&path)?
            .write_all(terminal.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

pub(crate) struct Snapshots {
    root: PathBuf,
    /// `<pid>-<start>` of this supervisor, or why it is unknown.
    prefix: Result<String, String>,
    next: u64,
    /// Each Scrollback popup's terminal, its snapshot directory, and the
    /// endpoint of a failed start that may still run it.
    owned: Vec<(String, PathBuf, Option<PathBuf>)>,
    /// Snapshot directories whose removal failed, retried on later turns.
    stale: Vec<PathBuf>,
}

impl Snapshots {
    /// The store under `runtime`, which nothing creates before a snapshot.
    pub(crate) fn new(runtime: &Path) -> Self {
        let pid = std::process::id();
        let prefix = match platform::process(pid) {
            Ok(Some(process)) => Ok(format!("{pid}-{}", process.start)),
            Ok(None) => Err("this supervisor's process identity is unavailable".into()),
            Err(error) => Err(error),
        };
        Self {
            root: runtime.join("snapshots"),
            prefix,
            next: 1,
            owned: Vec::new(),
            stale: Vec::new(),
        }
    }

    /// Removes, as a supervisor starts, every snapshot directory whose
    /// supervisor is no longer running, except those whose recorded terminal
    /// is in `adopted`, which this supervisor takes over.
    pub(crate) fn sweep<'a>(
        &mut self,
        adopted: impl IntoIterator<Item = &'a str>,
        log: &DiagnosticLog,
    ) {
        let adopted = adopted.into_iter().collect::<Vec<_>>();
        if let Err(error) = self.sweep_stale(&adopted) {
            log.record(&format!("cannot sweep stale scrollback snapshots: {error}"));
        }
    }

    fn sweep_stale(&mut self, adopted: &[&str]) -> Result<(), String> {
        let entries = match fs::read_dir(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            entries => {
                validate_private_directory(&self.root)?;
                entries.map_err(|error| format!("cannot list {}: {error}", self.root.display()))?
            }
        };
        let mut errors = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if writer_running(&entry.file_name().to_string_lossy()) {
                continue;
            }
            match owner(&path) {
                Some(terminal) if adopted.contains(&terminal.as_str()) => {
                    self.owned.push((terminal, path, None));
                }
                _ => {
                    if let Err(error) = remove(&path) {
                        errors.push(error);
                        self.stale.push(path);
                    }
                }
            }
        }
        self.remove_empty_root();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Writes `history` of the pane or popup `target` to a new snapshot,
    /// after a notice line when the bound cut it or it holds only the
    /// alternate screen. Leaves nothing behind when it fails.
    pub(crate) fn write(
        &mut self,
        target: &str,
        history: &HistoryText,
    ) -> Result<Snapshot, String> {
        let prefix = self.prefix.as_ref().map_err(Clone::clone)?;
        prepare_runtime(&self.root)?;
        let directory = self.root.join(format!("{prefix}-{}", self.next));
        self.next += 1;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|error| {
                format!(
                    "cannot create snapshot directory {}: {error}",
                    directory.display()
                )
            })?;
        let snapshot = Snapshot {
            file: directory.join(format!("{target}-scrollback.txt")),
            directory,
        };
        let written = validate_private_directory(&snapshot.directory).and_then(|()| {
            let mut file = create_private(&snapshot.file)?;
            file.write_all(contents(history).as_bytes())
                .map_err(|error| format!("cannot write {}: {error}", snapshot.file.display()))
        });
        match written {
            Ok(()) => Ok(snapshot),
            Err(error) => Err(match self.discard(snapshot) {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; {cleanup}"),
            }),
        }
    }

    /// Hands `snapshot` to the popup whose terminal is `terminal`; `uncertain`
    /// is the endpoint of a failed start whose terminal may still run.
    pub(crate) fn own(&mut self, snapshot: Snapshot, terminal: String, uncertain: Option<PathBuf>) {
        self.owned.push((terminal, snapshot.directory, uncertain));
    }

    /// Removes a snapshot whose popup did not start, or keeps it for a retry.
    pub(crate) fn discard(&mut self, snapshot: Snapshot) -> Result<(), String> {
        remove(&snapshot.directory).inspect_err(|_| self.stale.push(snapshot.directory))
    }

    /// Removes the snapshot of each popup whose terminal, with any uncertain
    /// start's endpoint, `live` no longer names, retries earlier removals
    /// that failed, and removes the empty store with the last snapshot.
    pub(crate) fn retain(
        &mut self,
        live: impl Fn(&str, Option<&Path>) -> bool,
        log: &DiagnosticLog,
    ) {
        let before = self.owned.len() + self.stale.len();
        // Earlier failures were logged when they first failed.
        self.stale.retain(|directory| remove(directory).is_err());
        let stale = &mut self.stale;
        self.owned.retain(|(terminal, directory, uncertain)| {
            if live(terminal, uncertain.as_deref()) {
                return true;
            }
            if let Err(error) = remove(directory) {
                log.record(&error);
                stale.push(directory.clone());
            }
            false
        });
        if self.owned.len() + self.stale.len() < before {
            self.remove_empty_root();
        }
    }

    /// Removes the store once it holds nothing, as a supervisor leaves its
    /// runtime directory.
    pub(crate) fn remove_empty_root(&self) {
        if self.owned.is_empty() && self.stale.is_empty() {
            // Another supervisor's snapshot keeps it.
            let _ = fs::remove_dir(&self.root);
        }
    }
}

/// The snapshot file's text: an optional notice line, then the text as
/// `eon pane history` prints it.
fn contents(history: &HistoryText) -> String {
    let mut notices = Vec::new();
    if let Some(left_out) = super::cli::history_left_out(history.omitted_lines, history.cut_bytes) {
        notices.push(format!(
            "Eon left out {left_out} to keep this snapshot within its {} MiB bound.",
            orbit_protocol::management::MAX_HISTORY_TEXT_BYTES / (1024 * 1024)
        ));
    }
    if history.screen == Screen::Alternate {
        notices.push(ALTERNATE_NOTICE.into());
    }
    let mut contents = String::new();
    if !notices.is_empty() {
        contents.push_str(&notices.join(" "));
        contents.push('\n');
    }
    contents.push_str(&history.text);
    contents.push('\n');
    contents
}

fn create_private(path: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

/// Whether the supervisor a snapshot directory's `<pid>-<start>-<n>` name
/// records is still running. An unreadable identity counts as running, so
/// the sweep keeps what it cannot judge.
fn writer_running(name: &str) -> bool {
    let mut parts = name.splitn(3, '-');
    let (Some(Ok(pid)), Some(Ok(start)), Some(_)) = (
        parts.next().map(str::parse::<u32>),
        parts.next().map(str::parse::<u64>),
        parts.next(),
    ) else {
        return false;
    };
    match platform::process(pid) {
        Ok(Some(process)) => process.start == start && !process.ended,
        Ok(None) => false,
        Err(_) => true,
    }
}

/// The terminal a snapshot directory's record names, if it has one.
fn owner(directory: &Path) -> Option<String> {
    let mut text = String::new();
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(directory.join(OWNER))
        .ok()?
        .take(MAX_OWNER_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// Removes a snapshot directory, or a link planted in its place, without
/// following links.
fn remove(directory: &Path) -> Result<(), String> {
    let removed = match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(directory),
        Ok(_) => fs::remove_file(directory),
        Err(error) => Err(error),
    };
    match removed {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(format!(
            "cannot remove scrollback snapshot {}: {error}",
            directory.display()
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{ALTERNATE_NOTICE, OWNER, Snapshots, contents};
    use crate::{diagnostics::DiagnosticLog, supervisor::temporary_directory};
    use orbit_protocol::management::{HistoryText, Screen};
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        path::Path,
    };

    pub(crate) fn history(screen: Screen, omitted_lines: u32, cut_bytes: u32) -> HistoryText {
        HistoryText {
            revision: 1,
            screen,
            rows: 24,
            columns: 80,
            title: String::new(),
            directory: Vec::new(),
            omitted_lines,
            cut_bytes,
            lossy: true,
            text: "first\nlast".into(),
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    /// The text follows as `eon pane history` prints it, after one notice
    /// line only when the bound cut it or it shows the alternate screen.
    #[test]
    fn a_notice_line_names_the_bound_cut_and_the_alternate_screen() {
        assert_eq!(contents(&history(Screen::Primary, 0, 0)), "first\nlast\n");
        assert_eq!(
            contents(&history(Screen::Primary, 3, 0)),
            "Eon left out the 3 earliest lines to keep this snapshot within its 8 MiB bound.\nfirst\nlast\n"
        );
        assert_eq!(
            contents(&history(Screen::Primary, 1, 7)),
            "Eon left out the earliest line and the first 7 bytes of the newest line to keep this snapshot within its 8 MiB bound.\nfirst\nlast\n"
        );
        assert_eq!(
            contents(&history(Screen::Alternate, 0, 0)),
            format!("{ALTERNATE_NOTICE}\nfirst\nlast\n")
        );
    }

    /// A snapshot is a 0600 file in a fresh 0700 directory of a 0700 store,
    /// and a link planted at the store or at the next directory refuses the
    /// write, leaving the link's target untouched.
    #[test]
    fn snapshots_are_private_and_a_planted_link_refuses() {
        let runtime = temporary_directory();
        let elsewhere = runtime.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let mut store = Snapshots::new(&runtime);
        let snapshot = store.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
        assert_eq!(
            snapshot.file.parent().unwrap().parent().unwrap(),
            runtime.join("snapshots")
        );
        assert_eq!(snapshot.file.file_name().unwrap(), "p1-scrollback.txt");
        assert_eq!(fs::read_to_string(&snapshot.file).unwrap(), "first\nlast\n");
        assert_eq!(
            [
                &runtime.join("snapshots"),
                &snapshot.directory,
                &snapshot.file
            ]
            .map(|path| mode(path)),
            [0o700, 0o700, 0o600]
        );
        store.discard(snapshot).unwrap();

        let next = runtime.join("snapshots").join(format!(
            "{}-{}",
            store.prefix.clone().unwrap(),
            store.next
        ));
        symlink(&elsewhere, &next).unwrap();
        assert!(store.write("p1", &history(Screen::Primary, 0, 0)).is_err());
        assert!(
            fs::symlink_metadata(&next)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);

        fs::remove_dir_all(runtime.join("snapshots")).unwrap();
        symlink(&elsewhere, runtime.join("snapshots")).unwrap();
        assert!(store.write("p1", &history(Screen::Primary, 0, 0)).is_err());
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        fs::remove_dir_all(runtime).unwrap();
    }

    /// A starting supervisor removes the snapshots of supervisors no longer
    /// running, except one whose recorded terminal it adopts; it keeps a
    /// running supervisor's. An owned snapshot goes when its terminal ends,
    /// and the store with the last one.
    #[test]
    fn the_start_sweep_keeps_adopted_and_running_snapshots_and_ends_remove_them() {
        let runtime = temporary_directory();
        let log = DiagnosticLog::open_in(Ok(runtime.join("logs")), "g1-test", "workspace");
        let mut writer = Snapshots::new(&runtime);
        let mut write = |terminal: Option<&str>| {
            let snapshot = writer.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
            let directory = snapshot.directory.clone();
            if let Some(terminal) = terminal {
                snapshot.mark(terminal).unwrap();
                writer.own(snapshot, terminal.into(), None);
            }
            directory
        };
        let [adopted, ended, unowned, running] =
            [Some("session-3"), Some("session-4"), None, None].map(&mut write);
        let snapshots = runtime.join("snapshots");
        // A pid no process has stands for a supervisor that stopped running.
        let dead = |directory: &Path| {
            let name = directory.file_name().unwrap().to_str().unwrap();
            let (_, rest) = name.split_once('-').unwrap();
            let moved = snapshots.join(format!("{}-{rest}", u32::MAX));
            fs::rename(directory, &moved).unwrap();
            moved
        };
        let [adopted, ended, unowned] = [&adopted, &ended, &unowned].map(|path| dead(path));
        let junk = snapshots.join("junk");
        fs::create_dir(&junk).unwrap();
        assert_eq!(
            fs::read_to_string(adopted.join(OWNER)).unwrap(),
            "session-3"
        );

        let mut store = Snapshots::new(&runtime);
        store.sweep(["session-1", "session-3"], &log);
        assert!(adopted.exists() && running.exists());
        assert!(!ended.exists() && !unowned.exists() && !junk.exists());

        store.retain(|terminal, _| terminal == "session-3", &log);
        assert!(adopted.exists());
        store.retain(|_, _| false, &log);
        assert!(!adopted.exists());
        // The running supervisor's snapshot keeps the store.
        assert!(running.exists());
        fs::remove_dir_all(&running).unwrap();
        let mut last = Snapshots::new(&runtime);
        let snapshot = last.write("u2", &history(Screen::Primary, 0, 0)).unwrap();
        snapshot.mark("session-9").unwrap();
        last.own(snapshot, "session-9".into(), None);
        last.retain(|_, _| false, &log);
        assert!(!snapshots.exists());
        fs::remove_dir_all(runtime).unwrap();
    }

    /// A removal that fails, of an ended popup's snapshot or of one whose
    /// popup did not start, is retried on later turns until it succeeds.
    #[test]
    fn a_failed_removal_is_retried_on_later_turns() {
        let runtime = temporary_directory();
        let log = DiagnosticLog::open_in(Ok(runtime.join("logs")), "g1-test", "workspace");
        let mut store = Snapshots::new(&runtime);
        let ended = store.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
        let discarded = store.write("p2", &history(Screen::Primary, 0, 0)).unwrap();
        let directories = [ended.directory.clone(), discarded.directory.clone()];
        // A directory its owner cannot write keeps its file.
        for directory in &directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
        }
        store.own(ended, "session-2".into(), None);
        store.retain(|_, _| false, &log);
        assert!(store.discard(discarded).is_err());
        store.retain(|_, _| false, &log);
        assert!(directories.iter().all(|directory| directory.exists()));
        for directory in &directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        store.retain(|_, _| false, &log);
        assert!(directories.iter().all(|directory| !directory.exists()));
        assert!(!runtime.join("snapshots").exists());
        fs::remove_dir_all(runtime).unwrap();
    }
}
