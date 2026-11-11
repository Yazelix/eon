//! Private listener paths for the packaged Helix in Editor popups (EON-C10).

use super::{
    diagnostics::DiagnosticLog,
    managed_environment::{self, ManagedPrograms, Tool},
    platform,
    sessions::{self, RunningSession},
    supervisor::{effective_uid, prepare_runtime, validate_private_directory},
};
use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn listen_position(
    argv: &[OsString],
    programs: &ManagedPrograms,
    directory: &Path,
) -> Result<Option<usize>, String> {
    let Some(helix) = &programs.helix else {
        return Ok(None);
    };
    let program = Path::new(&argv[0]);
    let Ok(metadata) = fs::metadata(program) else {
        return Ok(None);
    };
    let matches = |path: &Path| fs::metadata(path).is_ok_and(|other| same_file(&metadata, &other));
    let managed = managed_environment::tool(program.as_os_str()) == Some(Tool::Helix)
        && programs
            .shell_bin
            .as_ref()
            .is_some_and(|bin| matches(&directory.join(bin).join("hx")));
    if !matches(&directory.join(helix)) && !managed {
        return Ok(None);
    }

    // The pinned Helix parser consumes the next token for these options, even
    // when it spells "--" or "--listen".
    let mut arguments = argv.iter().enumerate().skip(1);
    while let Some((index, argument)) = arguments.next() {
        match argument.to_str() {
            Some("--") => return Ok(Some(index)),
            Some("--listen") => {
                return Err("Editor supplies the packaged Helix listener; remove --listen from [editor] command to open the popup".into());
            }
            Some(
                "-g" | "--grammar" | "-c" | "--config" | "--config-dir" | "--log" | "-w"
                | "--working-dir",
            ) => {
                arguments
                    .next()
                    .ok_or_else(|| format!("[editor] command's {argument:?} needs an argument"))?;
            }
            _ => {}
        }
    }
    Ok(Some(argv.len()))
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    (left.dev(), left.ino()) == (right.dev(), right.ino())
}

#[derive(Default)]
pub(super) struct Sockets {
    listeners: Vec<Listener>,
}

struct Listener {
    endpoint: PathBuf,
    retry_at: Option<Instant>,
}

impl Sockets {
    pub(super) fn recover(
        runtime: &Path,
        sessions: &[RunningSession],
        log: &DiagnosticLog,
    ) -> Self {
        let directory = runtime.join("editor");
        let mut sockets = Self::default();
        let recovered = (|| {
            match fs::symlink_metadata(&directory) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
            validate_private_directory(&directory)?;
            let entries = fs::read_dir(&directory).map_err(|error| error.to_string())?;
            for entry in entries {
                let name = entry.map_err(|error| error.to_string())?.file_name();
                let path = Path::new(&name);
                if matches!(
                    path.extension().and_then(|suffix| suffix.to_str()),
                    Some("sock" | "owner")
                ) && path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(sessions::session_number)
                    .is_some()
                {
                    sockets.track(runtime.join(path).with_extension("sock"));
                }
            }
            sockets.retain(sessions, Instant::now(), log);
            sockets.remove_empty_root(&directory);
            Ok(())
        })();
        if let Err(error) = recovered {
            log.record(&format!("cannot sweep Editor sockets: {error}"));
        }
        sockets
    }

    pub(super) fn prepare(
        &mut self,
        endpoint: &Path,
        sessions: &[RunningSession],
    ) -> Result<PathBuf, String> {
        let path = socket_path(endpoint);
        let directory = path.parent().expect("Editor socket has a parent");
        prepare_runtime(directory)?;
        let prepared = (|| {
            if terminal_may_run(endpoint, sessions) {
                return Err(format!(
                    "Editor socket {} belongs to a terminal that may still run",
                    path.display()
                ));
            }
            let canonical = fs::canonicalize(directory)
                .map_err(|error| format!("cannot resolve Editor socket directory: {error}"))?
                .join(
                    endpoint
                        .file_name()
                        .expect("terminal endpoint has a filename"),
                );
            if canonical.to_str().is_none() || !platform::socket_path_fits(&canonical) {
                return Err(format!(
                    "Editor socket path {} must fit a Unix socket and be UTF-8",
                    canonical.display()
                ));
            }
            remove_stale(&canonical)?;
            let marker = canonical.with_extension("owner");
            remove_marker(&marker)?;
            // Orbit can report Ready, and the supervisor can restart, before Helix binds.
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&marker)
                .map_err(|error| {
                    format!("cannot mark Editor listener {}: {error}", marker.display())
                })?;
            Ok(canonical)
        })();
        if prepared.is_ok() {
            self.track(endpoint.to_owned());
        } else {
            self.remove_empty_root(directory);
        }
        prepared
    }

    fn track(&mut self, endpoint: PathBuf) {
        if let Some(listener) = self
            .listeners
            .iter_mut()
            .find(|listener| listener.endpoint == endpoint)
        {
            listener.retry_at = None;
        } else {
            self.listeners.push(Listener {
                endpoint,
                retry_at: None,
            });
        }
    }

    pub(super) fn retain(
        &mut self,
        sessions: &[RunningSession],
        now: Instant,
        log: &DiagnosticLog,
    ) {
        let mut removed = None;
        self.listeners.retain_mut(|listener| {
            if listener.retry_at.is_some_and(|retry| now < retry)
                || terminal_may_run(&listener.endpoint, sessions)
            {
                return true;
            }
            let path = socket_path(&listener.endpoint);
            if let Err(error) =
                remove_stale(&path).and_then(|()| remove_marker(&path.with_extension("owner")))
            {
                if listener.retry_at.is_none() {
                    log.record(&error);
                }
                listener.retry_at = Some(now + Duration::from_secs(1));
                return true;
            }
            removed = Some(path);
            false
        });
        if let Some(path) = removed {
            self.remove_empty_root(path.parent().expect("Editor socket has a parent"));
        }
    }

    fn remove_empty_root(&self, directory: &Path) {
        if self.listeners.is_empty() {
            let _ = fs::remove_dir(directory);
        }
    }
}

fn terminal_may_run(endpoint: &Path, running: &[RunningSession]) -> bool {
    running
        .iter()
        .any(|session| session.endpoint == endpoint && !session.is_finished())
        || sessions::launch_record_remains(endpoint)
}

fn socket_path(endpoint: &Path) -> PathBuf {
    endpoint.with_file_name("editor").join(
        endpoint
            .file_name()
            .expect("terminal endpoint has a filename"),
    )
}

fn remove_marker(marker: &Path) -> Result<(), String> {
    let failure = |error| {
        format!(
            "cannot remove Editor listener marker {}: {error}",
            marker.display()
        )
    };
    match fs::symlink_metadata(marker) {
        Ok(metadata) if metadata.is_file() && metadata.uid() == effective_uid() => {}
        Ok(_) => {
            return Err(format!(
                "Editor listener marker {} is not an owned file",
                marker.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
    }
    validate_private_directory(marker.parent().expect("Editor marker has a parent"))?;
    match fs::remove_file(marker) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failure(error)),
        _ => Ok(()),
    }
}

fn remove_stale(path: &Path) -> Result<(), String> {
    let failure =
        |detail: &str| format!("cannot remove Editor socket {}: {detail}", path.display());
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(&error.to_string())),
    };
    validate_private_directory(path.parent().expect("Editor socket has a parent"))?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(failure("expected an owned socket"));
    }
    // A nonblocking attempt distinguishes a refused socket from one that may listen.
    match sessions::unix_connect_with_timeout(path, Duration::ZERO) {
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => return Err(failure("the socket may still be listening")),
    }
    match fs::symlink_metadata(path) {
        Ok(current) if same_file(&metadata, &current) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => return Err(failure("the socket changed before cleanup")),
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failure(&error.to_string())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::symlink, net::UnixListener};

    #[test]
    fn only_the_packaged_helix_gets_an_eon_listener_option() {
        let root = super::super::supervisor::temporary_directory();
        let mut programs = crate::fixtures::inputs().managed;
        let packaged = root.join("packaged-hx");
        let user_hx = root.join("hx");
        let session_bin = root.join("bin");
        fs::create_dir(&session_bin).unwrap();
        let launcher = session_bin.join("hx");
        let alias = root.join("eon-hx");
        fs::write(&packaged, "").unwrap();
        fs::write(&user_hx, "").unwrap();
        fs::write(&launcher, "").unwrap();
        symlink(&launcher, &alias).unwrap();
        programs.helix = Some(packaged.clone());
        programs.shell_bin = Some(session_bin);
        let argv = |path: &Path, arguments: &[&str]| {
            std::iter::once(path.as_os_str().to_owned())
                .chain(arguments.iter().map(OsString::from))
                .collect::<Vec<_>>()
        };
        for path in [&packaged, &launcher, &alias] {
            let command = argv(path, &["--log", "--", "--", "--listen"]);
            assert_eq!(
                listen_position(&command, &programs, &root).unwrap(),
                Some(3)
            );
            let command = argv(path, &["--log", "--listen"]);
            assert_eq!(
                listen_position(&command, &programs, &root).unwrap(),
                Some(3)
            );
            let command = argv(path, &["--listen", "/other.sock"]);
            assert!(
                listen_position(&command, &programs, &root)
                    .unwrap_err()
                    .contains("[editor] command")
            );
        }
        let command = argv(&user_hx, &["--listen", "/other.sock"]);
        assert_eq!(listen_position(&command, &programs, &root).unwrap(), None);
        programs.helix = None;
        assert_eq!(
            listen_position(&argv(&launcher, &[]), &programs, &root).unwrap(),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_preflight_leaves_a_live_socket_and_reuses_it_after_exit() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-2.sock");
        let path = socket_path(&endpoint);
        prepare_runtime(path.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        let mut sockets = Sockets::default();
        assert!(
            sockets
                .prepare(&endpoint, &[])
                .unwrap_err()
                .contains("may still be listening")
        );
        assert!(path.exists());
        drop(listener);
        let selected = sockets.prepare(&endpoint, &[]).unwrap();
        assert_eq!(
            selected,
            root.canonicalize().unwrap().join("editor/terminal-2.sock")
        );
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_cleanup_retries_quietly_after_a_listener_outlives_its_terminal() {
        let root = super::super::supervisor::temporary_directory();
        let logs = root.join("logs");
        let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
        let endpoint = root.join("terminal-2.sock");
        let mut sockets = Sockets::default();
        let path = sockets.prepare(&endpoint, &[]).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        let marker = path.with_extension("owner");
        let now = Instant::now();
        sockets.retain(&[], now, &log);
        sockets.retain(&[], now + Duration::from_secs(1), &log);
        assert!(marker.exists());
        let diagnostic = fs::read_dir(logs).unwrap().next().unwrap().unwrap().path();
        let text = fs::read_to_string(diagnostic).unwrap();
        assert_eq!(text.matches("the socket may still be listening").count(), 1);

        drop(listener);
        sockets.retain(&[], now + Duration::from_secs(1), &log);
        assert!(path.exists(), "cleanup waits until the retry is due");
        sockets.retain(&[], now + Duration::from_secs(2), &log);
        assert!(!path.parent().unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }
}
