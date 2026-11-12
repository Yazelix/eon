//! Private listener paths for the packaged Helix in Editor popups (EON-C10).

use super::{
    diagnostics::DiagnosticLog,
    managed_environment::{self, ManagedPrograms, Tool},
    platform,
    sessions::{self, RunningSession},
    supervisor::{effective_uid, prepare_runtime, validate_private_directory},
};
use eon_workspace_protocol::v42::Failure;
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct EditCommand {
    pub(crate) argv: Vec<OsString>,
    pub(crate) popup: bool,
}

const FRAME_LIMIT: usize = 65_536;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);

fn failure(code: &str, detail: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        detail: detail.into(),
    }
}

/// S3's marker survives recovery and the Ready-before-bind interval.
pub(super) fn has_listener(endpoint: &Path) -> Result<bool, Failure> {
    let path = socket_path(endpoint).with_extension("owner");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Ok(metadata)
            if metadata.is_file()
                && metadata.uid() == effective_uid()
                && metadata.mode() & 0o077 == 0 =>
        {
            validate_private_directory(path.parent().unwrap())
                .map_err(|detail| failure("editor-unavailable", detail))?;
            Ok(true)
        }
        _ => Err(failure(
            "editor-unavailable",
            "Editor listener marker is not an owned private file",
        )),
    }
}

pub(super) fn open(endpoint: &Path, file: &Path, id: &str) -> Result<(), Failure> {
    let path = file.to_str().ok_or_else(|| {
        failure(
            "editor-rejected",
            "the running Editor accepts only UTF-8 file names",
        )
    })?;
    let mut request = serde_json::to_vec(&serde_json::json!({
        "version": 1, "id": id, "action": "open", "paths": [path], "timeout_ms": 750
    }))
    .expect("JSON scalar values serialize");
    request.push(b'\n');
    if request.len() > FRAME_LIMIT {
        return Err(failure("editor-rejected", "Editor request exceeds 64 KiB"));
    }
    let socket = socket_path(endpoint);
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut waiting_for_bind = false;
    let mut stream = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failure(
                if waiting_for_bind {
                    "editor-unavailable"
                } else {
                    "editor-unresponsive"
                },
                "Editor connect exceeded 100 ms; no request sent",
            ));
        }
        match sessions::unix_connect_with_timeout(&socket, remaining) {
            Ok(stream) => break stream,
            Err(error) if error.kind() == io::ErrorKind::NotFound && has_listener(endpoint)? => {
                waiting_for_bind = true;
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(error) => {
                let detail = format!("Editor connection failed: {error}; no request sent");
                let mut failure = io_failure(error);
                failure.detail = detail;
                return Err(failure);
            }
        }
    };
    let deadline = Instant::now() + ANSWER_TIMEOUT;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                failure(
                    "editor-unresponsive",
                    "Editor answer exceeded 1 second; open outcome is unknown",
                )
            })
    };
    let mut bytes = request.as_slice();
    while !bytes.is_empty() {
        sessions::bound_io(&stream, stream.set_write_timeout(Some(remaining()?)))
            .map_err(io_failure)?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(failure(
                    "editor-unavailable",
                    "Editor closed before receiving the request; open outcome is unknown",
                ));
            }
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io_failure(error)),
        }
    }
    let mut reply = Vec::new();
    loop {
        sessions::bound_io(&stream, stream.set_read_timeout(Some(remaining()?)))
            .map_err(io_failure)?;
        let mut chunk = [0; 4096];
        let count = match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(failure(
                    "editor-unavailable",
                    "Editor closed before a complete reply; open outcome is unknown",
                ));
            }
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_failure(error)),
        };
        let end = chunk[..count].iter().position(|byte| *byte == b'\n');
        reply.extend_from_slice(&chunk[..end.map_or(count, |end| end + 1)]);
        if reply.len() > FRAME_LIMIT || (reply.len() == FRAME_LIMIT && end.is_none()) {
            return Err(failure(
                "editor-unavailable",
                "Editor reply exceeds 64 KiB; open outcome is unknown",
            ));
        }
        if end.is_some() {
            break;
        }
    }
    parse_reply(&reply, id)
}

fn io_failure(error: io::Error) -> Failure {
    let code = if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        "editor-unresponsive"
    } else {
        "editor-unavailable"
    };
    failure(
        code,
        format!("Editor exchange failed: {error}; a sent request's open outcome is unknown"),
    )
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    version: u8,
    id: String,
    status: String,
    error: Option<EditorError>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EditorError {
    class: String,
    message: String,
    path_index: Option<usize>,
}

fn parse_reply(bytes: &[u8], id: &str) -> Result<(), Failure> {
    let invalid = || {
        failure(
            "editor-unavailable",
            "invalid Editor reply; open outcome is unknown",
        )
    };
    let answer: Answer = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if answer.version != 1 || answer.id != id {
        return Err(invalid());
    }
    match (answer.status.as_str(), answer.error) {
        ("ok", None) => Ok(()),
        ("error", Some(error)) => {
            if error.path_index.is_some_and(|index| index != 0) {
                return Err(invalid());
            }
            let code = match error.class.as_str() {
                "timeout" => "editor-unresponsive",
                "rejected" | "invalid" | "unsupported" => "editor-rejected",
                "unavailable" => "editor-unavailable",
                _ => return Err(invalid()),
            };
            let detail: String = error
                .message
                .chars()
                .filter(|character| !character.is_control())
                .take(512)
                .collect();
            if detail.is_empty() {
                return Err(invalid());
            }
            Err(failure(
                code,
                if error.class == "timeout" {
                    format!("{detail}; queued open was abandoned")
                } else {
                    detail
                },
            ))
        }
        _ => Err(invalid()),
    }
}

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
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::{fs::symlink, net::UnixListener};

    #[test]
    fn editor_replies_distinguish_rejection_timeout_and_uncertain_delivery() {
        for (class, code) in [
            ("timeout", "editor-unresponsive"),
            ("rejected", "editor-rejected"),
            ("invalid", "editor-rejected"),
            ("unsupported", "editor-rejected"),
            ("unavailable", "editor-unavailable"),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "version":1,"id":"r1","status":"error",
                "error":{"class":class,"message":"bad\u{1b}[2J\npath","path_index":0}
            }))
            .unwrap();
            let error = parse_reply(&bytes, "r1").unwrap_err();
            assert_eq!(error.code, code);
            assert!(!error.detail.chars().any(char::is_control));
        }
        for reply in [
            r#"{"version":2,"id":"r1","status":"ok"}"#,
            r#"{"version":1,"id":"other","status":"ok"}"#,
            r#"{"version":1,"id":"r1","status":"unknown"}"#,
            r#"{"version":1,"id":"r1","status":"error"}"#,
            r#"{"version":1,"id":"r1","status":"ok","extra":true}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"unknown","message":"bad"}}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"rejected","message":""}}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"rejected","message":"\u001b\n"}}"#,
            "invalid JSON",
        ] {
            assert_eq!(
                parse_reply(reply.as_bytes(), "r1").unwrap_err().code,
                "editor-unavailable",
                "{reply}"
            );
        }
    }

    #[test]
    fn editor_exchange_bounds_frames_and_lost_replies_without_retry() {
        for (reply, code) in [
            (Vec::new(), "editor-unavailable"),
            (b"{\"version\":1".to_vec(), "editor-unavailable"),
            (vec![b'x'; FRAME_LIMIT + 1], "editor-unavailable"),
        ] {
            let root = super::super::supervisor::temporary_directory();
            let endpoint = root.join("terminal-1.sock");
            let socket = socket_path(&endpoint);
            fs::create_dir(socket.parent().unwrap()).unwrap();
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let _ = stream.write_all(&reply);
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "a failed exchange was retried");
            });
            assert_eq!(
                open(&endpoint, Path::new("/a"), "r1").unwrap_err().code,
                code
            );
            server.join().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
        use std::os::unix::ffi::OsStrExt;
        let invalid = Path::new(std::ffi::OsStr::from_bytes(b"/bad\xff"));
        assert_eq!(
            open(Path::new("/no/socket"), invalid, "r1")
                .unwrap_err()
                .code,
            "editor-rejected"
        );
        let oversized = "/".to_owned() + &"a".repeat(FRAME_LIMIT);
        assert_eq!(
            open(Path::new("/no/socket"), Path::new(&oversized), "r1")
                .unwrap_err()
                .code,
            "editor-rejected"
        );
    }

    #[test]
    fn editor_answer_deadline_does_not_start_another_exchange() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        fs::create_dir(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let (release, held) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            held.recv_timeout(Duration::from_secs(5)).unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        });
        let started = Instant::now();
        assert_eq!(
            open(&endpoint, Path::new("/a"), "r1").unwrap_err().code,
            "editor-unresponsive"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        release.send(()).unwrap();
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_connect_handles_initial_binding_refusal_and_a_full_queue() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        prepare_runtime(socket.parent().unwrap()).unwrap();
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(socket.with_extension("owner"))
            .unwrap();
        assert!(has_listener(&endpoint).unwrap());
        let bound = socket.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(5));
            let listener = UnixListener::bind(&bound).unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            stream
                .write_all(b"{\"version\":1,\"id\":\"r1\",\"status\":\"ok\"}\n")
                .unwrap();
        });
        started.recv().unwrap();
        open(&endpoint, Path::new("/a"), "r1").unwrap();
        server.join().unwrap();
        let refused = open(&endpoint, Path::new("/a"), "r2").unwrap_err();
        assert_eq!(refused.code, "editor-unavailable");
        assert!(refused.detail.contains("no request sent"), "{refused:?}");
        fs::remove_file(&socket).unwrap();
        let listener =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        listener
            .bind(&socket2::SockAddr::unix(&socket).unwrap())
            .unwrap();
        listener.listen(1).unwrap();
        let mut queued = Vec::new();
        for _ in 0..10 {
            match sessions::unix_connect_with_timeout(&socket, Duration::from_millis(10)) {
                Ok(stream) => queued.push(stream),
                Err(_) => break,
            }
        }
        assert!(
            !queued.is_empty() && queued.len() < 10,
            "queue did not fill"
        );
        let full = open(&endpoint, Path::new("/a"), "r3").unwrap_err();
        assert_eq!(full.code, "editor-unavailable", "{full:?}");
        drop(queued);
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_a_file_checks_the_reply_and_escapes_the_path() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        fs::create_dir(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["paths"], serde_json::json!(["/a\n\"b.rs:2:3"]));
            assert_eq!(request["timeout_ms"], 750);
            stream
                .write_all(b"{\"version\":1,\"id\":\"edit-1\",\"status\":\"ok\"}\n")
                .unwrap();
        });
        open(&endpoint, Path::new("/a\n\"b.rs:2:3"), "edit-1").unwrap();
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

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
