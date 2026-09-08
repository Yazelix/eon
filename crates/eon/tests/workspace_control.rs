use eon_workspace_protocol::v4::{
    Action, Direction, HEADER_BYTES, MAX_PANES, Pane, Request, Response, Snapshot, Tab, VERSION,
    declared_message_len, decode_request, decode_response, encode_request, encode_response,
};
use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage, EndpointIdentity, Failure,
    FailureCode, LiveIdentity, ObjectIdentity, ProcessOutcome, Record as ManagementRecord,
    ServerMessage as ManagementServerMessage, TerminationReason, Tombstone,
};
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

struct TestProcess {
    child: Child,
    stop: PathBuf,
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = fs::write(&self.stop, "");
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn temporary_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ec-{}-{}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn quick_picker_executable(path: &Path) {
    executable(
        path,
        "#!/bin/sh\n[ -z \"$FZF_DEFAULT_OPTS$FZF_DEFAULT_OPTS_FILE\" ] || exit 99\nselection=$(cat)\n[ \"$selection\" != cancel ] || exit 130\nprintf '\\0%s\\0' \"$selection\"\n",
    );
}

fn artifact_path(endpoint: &Path, suffix: &str) -> PathBuf {
    let mut path = endpoint.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

fn object_identity(path: &Path) -> ObjectIdentity {
    let metadata = fs::symlink_metadata(path).unwrap();
    ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn write_management_record(path: &Path, record: &ManagementRecord) {
    let bytes = management::encode_record(record).unwrap();
    let temporary = artifact_path(path, &format!(".tmp.{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .unwrap();
    file.write_all(&bytes).unwrap();
    drop(file);
    fs::rename(temporary, path).unwrap();
}

fn publish_management_record(path: &Path, record: &ManagementRecord) {
    let mut claim = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .unwrap();
    let expected = object_identity(path);
    let metadata = claim.metadata().unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.len(), 0);
    claim.try_lock().unwrap();
    assert_eq!(object_identity(path), expected);
    claim.write_all(b"1").unwrap();
    if std::env::var_os("EON_TEST_REPLACE_READY_RECORD").is_some() {
        fs::remove_file(path).unwrap();
        let mut replacement = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        replacement.write_all(b"invalid").unwrap();
    } else {
        write_management_record(path, record);
    }
}

fn managed_orbit_executable(path: &Path) {
    let helper = std::env::current_exe().unwrap();
    executable(
        path,
        &format!(
            "#!/bin/sh\nEON_TEST_MANAGED_ORBIT=1 exec '{}' --exact managed_orbit_helper --nocapture -- \"$@\"\n",
            helper.display()
        ),
    );
}

fn process_start_identity() -> u64 {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap()
}

fn endpoint_identity(path: &Path) -> EndpointIdentity {
    EndpointIdentity {
        path: path.as_os_str().as_bytes().to_vec(),
        object: object_identity(path),
    }
}

fn write_management(stream: &mut UnixStream, message: ManagementServerMessage) {
    let bytes = management::encode_server_message(&message).unwrap();
    stream.set_nonblocking(false).unwrap();
    stream.write_all(&bytes).unwrap();
    stream.set_nonblocking(true).unwrap();
}

fn finish_managed_orbit(
    record_path: &Path,
    presentation: &Path,
    management_path: &Path,
    identity: &LiveIdentity,
    reason: TerminationReason,
    outcome: ProcessOutcome,
    client: Option<&mut UnixStream>,
) {
    let tombstone = Tombstone {
        identity: identity.clone(),
        reason,
        outcome,
    };
    write_management_record(record_path, &ManagementRecord::Tombstone(tombstone.clone()));
    fs::remove_file(presentation).unwrap();
    fs::remove_file(management_path).unwrap();
    if let Some(client) = client {
        write_management(client, ManagementServerMessage::Stopped(tombstone));
    }
}

#[test]
fn managed_orbit_helper() {
    if std::env::var_os("EON_TEST_MANAGED_ORBIT").is_none() {
        return;
    }
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let serve = arguments
        .iter()
        .position(|argument| argument == OsStr::new("serve"))
        .unwrap();
    let presentation = PathBuf::from(&arguments[serve + 1]);
    let management_argument = arguments
        .iter()
        .position(|argument| argument == OsStr::new("--management-v1"))
        .unwrap();
    let session_id = arguments[management_argument + 1]
        .to_str()
        .unwrap()
        .to_string();
    let run_id = arguments[management_argument + 2]
        .to_str()
        .unwrap()
        .to_string();
    let component_generation =
        std::env::var("EON_TEST_ORBIT_COMPONENT_GENERATION").unwrap_or_else(|_| {
            arguments[management_argument + 3]
                .to_str()
                .unwrap()
                .to_string()
        });
    if let Some(delay) = std::env::var_os("EON_TEST_ORBIT_DELAY_MS") {
        thread::sleep(Duration::from_millis(
            delay.to_str().unwrap().parse().unwrap(),
        ));
    }
    if session_id == "session-2"
        && let Some(delay) = std::env::var_os("EON_TEST_DELAY_SESSION_2_MS")
    {
        thread::sleep(Duration::from_millis(
            delay.to_str().unwrap().parse().unwrap(),
        ));
    }
    let session_id = std::env::var("EON_TEST_ORBIT_SESSION_ID").unwrap_or(session_id);

    let management_path = artifact_path(&presentation, ".management");
    let record_path = artifact_path(&presentation, ".record");
    let presentation_listener = UnixListener::bind(&presentation).unwrap();
    let management_listener = UnixListener::bind(&management_path).unwrap();
    fs::set_permissions(&presentation, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&management_path, fs::Permissions::from_mode(0o600)).unwrap();
    management_listener.set_nonblocking(true).unwrap();
    let identity = LiveIdentity {
        session_id,
        run_id,
        component_generation,
        record_generation: management::RECORD_GENERATION,
        management_generation: management::VERSION,
        process_id: std::process::id(),
        process_start: process_start_identity(),
        uid: unsafe { libc::geteuid() },
        presentation: endpoint_identity(&presentation),
        management: endpoint_identity(&management_path),
    };
    publish_management_record(&record_path, &ManagementRecord::Live(identity.clone()));
    if let Some(log) = std::env::var_os("EON_TEST_ORBIT_LOG") {
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(log, "{}", std::process::id()).unwrap();
    }
    if let Some(log) = std::env::var_os("EON_TEST_ORBIT_CWD_LOG") {
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(
            log,
            "{} {}",
            identity.session_id,
            std::env::current_dir().unwrap().display()
        )
        .unwrap();
    }
    if std::env::var_os("EON_TEST_REMOVE_ORBIT_CWD").is_some() {
        fs::remove_dir(std::env::current_dir().unwrap()).unwrap();
    }

    let command = arguments
        .iter()
        .rposition(|argument| argument == OsStr::new("--"))
        .map(|separator| &arguments[separator + 1..])
        .unwrap_or(&[]);
    let mut child = std::env::var_os("EON_TEST_RUN_CHILD").map(|_| {
        Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    });
    let mut client: Option<(UnixStream, Vec<u8>, bool)> = None;
    loop {
        match management_listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(true).unwrap();
                if client.as_ref().is_some_and(|client| client.2) {
                    write_management(&mut stream, ManagementServerMessage::Busy);
                } else {
                    client = Some((stream, Vec::new(), false));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("cannot accept management client: {error}"),
        }

        let mut stop = false;
        if let Some((stream, input, leased)) = &mut client {
            let mut bytes = [0; management::MAX_MESSAGE_BYTES];
            let disconnected = match stream.read(&mut bytes) {
                Ok(0) => true,
                Ok(count) => {
                    input.extend_from_slice(&bytes[..count]);
                    if let Some(length) = management::client_message_len(input).unwrap()
                        && input.len() >= length
                    {
                        let request = management::decode_client_message(&input[..length]).unwrap();
                        input.drain(..length);
                        match request {
                            ManagementClientMessage::Acquire { expected, record }
                                if !*leased
                                    && expected == identity
                                    && record == object_identity(&record_path) =>
                            {
                                *leased = true;
                                write_management(
                                    stream,
                                    ManagementServerMessage::Lease(identity.clone()),
                                );
                            }
                            ManagementClientMessage::Status if *leased => write_management(
                                stream,
                                ManagementServerMessage::Status(identity.clone()),
                            ),
                            ManagementClientMessage::Stop if *leased => stop = true,
                            _ => write_management(
                                stream,
                                ManagementServerMessage::Failure(Failure {
                                    code: FailureCode::InvalidRequest,
                                    detail: "invalid test management request".into(),
                                }),
                            ),
                        }
                    }
                    false
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => false,
                Err(_) => true,
            };
            if disconnected {
                let lease_released = *leased;
                client = None;
                if lease_released && let Some(path) = std::env::var_os("EON_TEST_LEASE_RELEASED") {
                    fs::write(path, "released").unwrap();
                }
            }
        }
        if stop {
            if std::env::var("EON_TEST_NATURAL_EXIT_ON_STOP")
                .ok()
                .as_deref()
                == Some(&identity.session_id)
            {
                if let Some(child) = &mut child {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                finish_managed_orbit(
                    &record_path,
                    &presentation,
                    &management_path,
                    &identity,
                    TerminationReason::NaturalExit,
                    ProcessOutcome::ExitCode(0),
                    None,
                );
                drop(presentation_listener);
                return;
            }
            if let Some(path) = std::env::var_os("EON_TEST_FAIL_STOP_ONCE")
                && fs::read_to_string(&path).ok().as_deref() == Some(&identity.session_id)
            {
                fs::remove_file(path).unwrap();
                write_management(
                    &mut client.as_mut().unwrap().0,
                    ManagementServerMessage::Failure(Failure {
                        code: FailureCode::InvalidRequest,
                        detail: "injected stop failure".into(),
                    }),
                );
                continue;
            }
            if let Some(delay) = std::env::var_os("EON_TEST_STOP_DELAY")
                .and_then(|path| fs::read_to_string(path).ok())
            {
                thread::sleep(Duration::from_millis(delay.parse().unwrap()));
            }
            if let Some(child) = &mut child {
                let _ = child.kill();
                let _ = child.wait();
            }
            finish_managed_orbit(
                &record_path,
                &presentation,
                &management_path,
                &identity,
                TerminationReason::ExplicitStop,
                ProcessOutcome::Signal(libc::SIGHUP),
                client.as_mut().map(|client| &mut client.0),
            );
            drop(presentation_listener);
            return;
        }

        let child_status = child.as_mut().and_then(|child| child.try_wait().unwrap());
        let initial_exit = presentation.file_name() == Some(OsStr::new("orbit.sock"))
            && std::env::var_os("EON_TEST_EXIT_INITIAL")
                .is_some_and(|path| Path::new(&path).exists());
        let requested_exit =
            std::env::var_os("EON_TEST_STOP").is_some_and(|path| Path::new(&path).exists());
        if child_status.is_some() || initial_exit || requested_exit {
            let outcome = child_status.map_or_else(
                || {
                    ProcessOutcome::ExitCode(
                        std::env::var("EON_TEST_ORBIT_EXIT_CODE")
                            .ok()
                            .and_then(|code| code.parse().ok())
                            .unwrap_or(0),
                    )
                },
                |status| {
                    status.code().map_or_else(
                        || ProcessOutcome::Signal(status.signal().unwrap()),
                        ProcessOutcome::ExitCode,
                    )
                },
            );
            finish_managed_orbit(
                &record_path,
                &presentation,
                &management_path,
                &identity,
                TerminationReason::NaturalExit,
                outcome,
                None,
            );
            drop(presentation_listener);
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn live_identity(endpoint: &Path) -> LiveIdentity {
    let record = artifact_path(endpoint, ".record");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(&record)
            && let Ok(ManagementRecord::Live(identity)) = management::decode_record(&bytes)
        {
            return identity;
        }
        assert!(
            Instant::now() < deadline,
            "{} was not live",
            record.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for(path: &Path) {
    let ready =
        || fs::metadata(path).is_ok_and(|metadata| !metadata.is_file() || metadata.len() != 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready(), "{} was not created or populated", path.display());
}

fn wait_for_connection(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} did not become connectable",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_process_removal(process_id: u32, failure: &str) {
    let process = Path::new("/proc").join(process_id.to_string());
    let deadline = Instant::now() + Duration::from_secs(5);
    while process.exists() {
        assert!(Instant::now() < deadline, "{failure}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn generation_runtime(root: &Path) -> PathBuf {
    let parent = root.join("generations");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(entries) = fs::read_dir(&parent) {
            let directories = entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            if directories.len() == 1 {
                return directories.into_iter().next().unwrap();
            }
        }
        assert!(
            Instant::now() < deadline,
            "one generation directory was not created in {}",
            parent.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn overfill_generation_directory(root: &Path) {
    for index in 0..256 {
        fs::create_dir(root.join("generations").join(format!("g1-{index:032x}"))).unwrap();
    }
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "Eon process did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_successful_exit(child: &mut Child) {
    let status = wait_for_exit(child);
    assert!(status.success(), "process exited with {status}");
}

fn invoke(binary: &Path, runtime: &Path, config: &Path, arguments: &[&str]) -> Output {
    eon_command(binary)
        .args(arguments)
        .env("EON_RUNTIME_DIR", runtime)
        .env("EON_CONFIG_HOME", config)
        .output()
        .unwrap()
}

fn workspace_action(socket: &Path, id: &str, action: Action) -> Response {
    let request = encode_request(&Request {
        id: id.into(),
        action,
    })
    .unwrap();
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(&request).unwrap();
    let mut response = vec![0; HEADER_BYTES];
    stream
        .read_exact(&mut response)
        .unwrap_or_else(|error| panic!("{id} response header failed: {error}"));
    let length = declared_message_len(&response).unwrap();
    response.resize(length, 0);
    stream.read_exact(&mut response[HEADER_BYTES..]).unwrap();
    decode_response(&response).unwrap()
}

fn wait_for_picker_close(socket: &Path, directory: &Path) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sequence = 0;
    loop {
        let response = workspace_action(
            socket,
            &format!("picker-inspect-{sequence}"),
            Action::Inspect,
        );
        if let Response::Snapshot(snapshot) = response
            && snapshot.directory_picker.is_none()
            && snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == snapshot.active_tab)
                .is_some_and(|tab| tab.directory == directory.as_os_str().as_bytes())
        {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "directory picker did not close");
        sequence += 1;
        thread::sleep(Duration::from_millis(10));
    }
}

fn picker_endpoint(socket: &Path) -> PathBuf {
    let id = format!(
        "picker-endpoint-{}",
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    );
    let Response::Snapshot(snapshot) = workspace_action(socket, &id, Action::Inspect) else {
        panic!("cannot inspect directory picker");
    };
    Path::new(OsStr::from_bytes(
        &snapshot.directory_picker.unwrap().endpoint,
    ))
    .to_path_buf()
}

fn eon_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_remove("EON_SESSION_BIN");
    command
}

#[test]
fn directory_picker_browse_preserves_raw_directory_and_cancellation() {
    let root = temporary_directory();
    let socket = root.join("eon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    executable(&root.join("zoxide"), "#!/bin/sh\nprintf '/known\\n'\n");
    executable(
        &root.join("fzf"),
        "#!/bin/sh\ncat >/dev/null\nprintf 'esc\\0'\nexit 1\n",
    );
    executable(
        &root.join("yazi"),
        "#!/bin/sh\nprintf '/untracked-\\377\\n'\n",
    );
    let mut path = vec![root.clone()];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_eon"));
    command
        .args([
            OsStr::new("__directory-picker"),
            socket.as_os_str(),
            OsStr::new("t1"),
        ])
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("EON_YAZI", root.join("yazi"))
        .env("EON_DIRECTORY_PICKER_CONFIG", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        if let Ok((stream, _)) = listener.accept() {
            break stream;
        }
        assert!(
            Instant::now() < deadline,
            "picker did not submit its directory"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let mut request = vec![0; HEADER_BYTES];
    stream.read_exact(&mut request).unwrap();
    request.resize(declared_message_len(&request).unwrap(), 0);
    stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
    let request = decode_request(&request).unwrap();
    assert_eq!(
        request.action,
        Action::SetTabDirectory {
            tab: "t1".into(),
            directory: b"/untracked-\xff\n".to_vec(),
        }
    );
    let response = encode_response(&Response::Snapshot(Snapshot {
        active_tab: "t1".into(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/untracked-\xff\n".to_vec(),
            selected_pane: Some("p1".into()),
            panes: vec![Pane {
                id: "p1".into(),
                session: "session-1".into(),
                endpoint: b"/test.sock".to_vec(),
                live: true,
            }],
        }],
        directory_picker: None,
    }))
    .unwrap();
    stream.write_all(&response).unwrap();
    wait_for_successful_exit(&mut child);
    for program in ["fzf", "yazi"] {
        executable(&root.join(program), "#!/bin/sh\nexit 130\n");
        let mut child = command.spawn().unwrap();
        wait_for_successful_exit(&mut child);
        assert!(
            listener.accept().is_err(),
            "cancellation sent a directory action"
        );
        executable(
            &root.join("fzf"),
            "#!/bin/sh\ncat >/dev/null\nprintf 'esc\\0'\nexit 1\n",
        );
    }
    let history_pid = root.join("history-pid");
    let browser_started = root.join("browser-started");
    command
        .env("EON_TEST_HISTORY_PID", &history_pid)
        .env("EON_TEST_BROWSER_STARTED", &browser_started);
    executable(
        &root.join("zoxide"),
        "#!/bin/sh\necho $$ > \"$EON_TEST_HISTORY_PID\"\nexec sleep 30\n",
    );
    executable(
        &root.join("yazi"),
        "#!/bin/sh\ntouch \"$EON_TEST_BROWSER_STARTED\"\nexit 130\n",
    );
    for (mode, finish) in [
        ("cancel", "exit 130"),
        ("browse", "printf 'esc\\0'\nexit 1"),
        ("accept", "printf '\\0/known\\0'"),
        ("failure", "exit 2"),
    ] {
        let _ = fs::remove_file(&history_pid);
        let _ = fs::remove_file(&browser_started);
        executable(
            &root.join("fzf"),
            &format!(
                "#!/bin/sh\nwhile [ ! -s \"$EON_TEST_HISTORY_PID\" ]; do sleep 0.01; done\n{finish}\n"
            ),
        );
        let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
        // Picker errors remain visible for two seconds before the process exits.
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut submitted = false;
        let finished = loop {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = vec![0; HEADER_BYTES];
                stream.read_exact(&mut request).unwrap();
                request.resize(declared_message_len(&request).unwrap(), 0);
                stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
                assert_eq!(
                    decode_request(&request).unwrap().action,
                    Action::SetTabDirectory {
                        tab: "t1".into(),
                        directory: b"/known".to_vec(),
                    }
                );
                assert!(!submitted, "selection was submitted twice");
                submitted = true;
                stream.write_all(&response).unwrap();
            }
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status.success());
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        let pid: i32 = fs::read_to_string(&history_pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: the PID belongs to this test's history child.
        let history_reaped = unsafe { libc::kill(pid, 0) } == -1;
        if !history_reaped {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        if finished.is_none() {
            let _ = child.kill();
        }
        wait_for_exit(&mut child);
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        assert!(
            finished == Some(mode != "failure") && history_reaped,
            "{mode}: result {finished:?}, history reaped {history_reaped}: {error}"
        );
        assert_eq!(submitted, mode == "accept");
        assert_eq!(browser_started.exists(), mode == "browse");
        assert!(listener.accept().is_err());
        if mode == "failure" {
            assert!(
                error.contains("packaged directory picker exited"),
                "{error}"
            );
        }
    }
    for (program, script, message) in [
        (
            "yazi",
            "#!/bin/sh\nexit 2\n",
            "packaged folder browser exited",
        ),
        (
            "fzf",
            "#!/bin/sh\ncat >/dev/null\nprintf 'unexpected\\0'\n",
            "directory picker returned an invalid selection",
        ),
        (
            "zoxide",
            "#!/bin/sh\nexit 2\n",
            "packaged directory history exited",
        ),
    ] {
        executable(&root.join("zoxide"), "#!/bin/sh\nprintf '/known\\n'\n");
        executable(
            &root.join("fzf"),
            "#!/bin/sh\ncat >/dev/null\nprintf 'esc\\0'\nexit 1\n",
        );
        executable(
            &root.join("yazi"),
            "#!/bin/sh\nprintf '/untracked-\\377\n'\n",
        );
        executable(&root.join(program), script);
        let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
        assert!(!wait_for_exit(&mut child).success());
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        assert!(error.contains(message), "{error}");
        assert!(
            listener.accept().is_err(),
            "failure sent a directory action"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_picker_retargets_cancels_and_stops_with_venus() {
    let root = temporary_directory();
    let runtime = root.join("runtime-padding");
    let config = root.join("config");
    let initial = root.join("initial");
    let selected = root.join("selected");
    let stop = root.join("stop");
    let stop_delay = root.join("stop-delay");
    let fail_stop_once = root.join("fail-stop-once");
    let release = root.join("picker-release");
    let selection = root.join("picker-selection");
    let orbit_log = root.join("orbit-cwd.log");
    let venus_pid = root.join("venus.pid");
    let venus_args = root.join("venus.args");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let session_bin = root.join("session-bin");
    fs::create_dir(&initial).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\nprintf '%s' \"$*\" > \"$EON_TEST_VENUS_ARGS\"\ncat >/dev/null\n",
    );
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_PICKER_RELEASE\" ]; do sleep 0.01; done\ncat \"$EON_TEST_PICKER_SELECTION\"\n",
    );
    executable(
        &session_bin.join("eon-nu"),
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-directory-picker")).unwrap();
    quick_picker_executable(&session_bin.join("fzf"));
    fs::write(&selection, selected.as_os_str().as_bytes()).unwrap();

    let child = eon_command(&binary)
        .arg("run")
        .current_dir(&initial)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_RUN_CHILD", "1")
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_FAIL_STOP_ONCE", &fail_stop_once)
        .env("EON_TEST_NATURAL_EXIT_ON_STOP", "session-1")
        .env("EON_TEST_PICKER_RELEASE", &release)
        .env("EON_TEST_PICKER_SELECTION", &selection)
        .env("FZF_DEFAULT_OPTS", "--preview=cat {}")
        .env("FZF_DEFAULT_OPTS_FILE", root.join("ambient-fzf-opts"))
        .env("EON_TEST_ORBIT_CWD_LOG", &orbit_log)
        .env("EON_TEST_VENUS_PID", &venus_pid)
        .env("EON_TEST_VENUS_ARGS", &venus_args)
        .env("EON_TEST_STOP_DELAY", &stop_delay)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_connection(&control);
    wait_for(&venus_pid);
    wait_for(&venus_args);
    let args = fs::read_to_string(&venus_args).unwrap();
    assert!(args.contains(&format!("--workspace {}", control.display())));
    assert!(!args.contains("orbit.sock"));

    let opened = workspace_action(&control, "picker-open", Action::Inspect);
    let Response::Snapshot(opened) = opened else {
        panic!("picker open did not return a snapshot");
    };
    let picker = opened.directory_picker.unwrap();
    assert_eq!(picker.tab, "t1");
    let initial_picker = Path::new(OsStr::from_bytes(&picker.endpoint)).to_path_buf();
    assert_eq!(initial_picker.parent(), Some(generation.as_path()));
    assert!(opened.tabs[0].panes.is_empty());
    assert!(opened.tabs[0].selected_pane.is_none());
    assert!(!generation.join("orbit.sock").exists());
    assert!(matches!(
        workspace_action(&control, "picker-duplicate", Action::PickTabDirectory),
        Response::Failure(failure) if failure.code == "picker-active"
    ));
    assert!(matches!(
        workspace_action(&control, "picker-singleton", Action::Focus(Direction::Left)),
        Response::Failure(failure) if failure.code == "unavailable"
    ));

    fs::write(&release, "").unwrap();
    let retargeted = wait_for_picker_close(&control, &selected);
    assert_eq!(retargeted.tabs[0].panes.len(), 1);
    assert_eq!(retargeted.tabs[0].selected_pane.as_deref(), Some("p1"));
    assert!(!initial_picker.exists());
    assert!(!artifact_path(&initial_picker, ".record").exists());

    let created = workspace_action(&control, "pane-after-picker", Action::CreatePane);
    assert!(matches!(
        created,
        Response::Snapshot(snapshot) if snapshot.tabs[0].panes.len() == 2
    ));
    wait_for(&orbit_log);
    let launches = fs::read_to_string(&orbit_log).unwrap();
    assert!(launches.contains(&format!("session-2 {}", selected.display())));

    fs::remove_file(&release).unwrap();
    fs::write(&selection, "cancel").unwrap();
    assert!(matches!(
        workspace_action(&control, "picker-cancel", Action::PickTabDirectory),
        Response::Snapshot(snapshot) if snapshot.directory_picker.is_some()
    ));
    fs::write(&release, "").unwrap();
    wait_for_picker_close(&control, &selected);

    fs::remove_file(&release).unwrap();
    assert!(matches!(
        workspace_action(&control, "new-tab-cancel", Action::CreateTab),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t2"
                && snapshot.tabs[1].panes.is_empty()
                && snapshot.tabs[1].selected_pane.is_none()
    ));
    let pending_picker = picker_endpoint(&control);
    assert_ne!(pending_picker, initial_picker);
    assert!(matches!(
        workspace_action(
            &control,
            "close-pending-tab",
            Action::CloseTab { tab: "t2".into() },
        ),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1" && snapshot.tabs.len() == 1
    ));
    assert!(!pending_picker.exists());

    fs::write(&selection, selected.as_os_str().as_bytes()).unwrap();
    let traversed_picker = match workspace_action(&control, "new-tab-accept", Action::CreateTab) {
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t3" && snapshot.tabs[1].panes.is_empty() =>
        {
            snapshot.directory_picker.unwrap()
        }
        _ => panic!("new tab did not open its picker"),
    };
    assert!(matches!(
        workspace_action(&control, "picker-traverse-left", Action::Focus(Direction::Left)),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1"
                && snapshot.directory_picker.as_ref() == Some(&traversed_picker)
                && snapshot.tabs[0].selected_pane.as_deref() == Some("p2")
    ));
    assert!(matches!(
        workspace_action(&control, "picker-traverse-right", Action::Focus(Direction::Right)),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t3"
                && snapshot.directory_picker.as_ref() == Some(&traversed_picker)
                && snapshot.tabs[1].panes.is_empty()
                && snapshot.tabs[1].selected_pane.is_none()
    ));
    fs::write(&release, "").unwrap();
    let accepted = wait_for_picker_close(&control, &selected);
    assert_eq!(accepted.active_tab, "t3");
    assert_eq!(accepted.tabs[1].selected_pane.as_deref(), Some("p3"));

    let invoke_ok = |arguments: &[&str]| {
        let output = invoke(&binary, &runtime, &config, arguments);
        assert!(output.status.success());
        output
    };
    let moved_tab = invoke_ok(&["tab", "move", "left", "--json"]);
    assert!(stdout(&moved_tab).contains("\"active_tab\":\"t3\",\"tabs\":[{\"id\":\"t3\""));
    invoke_ok(&["tab", "move", "right", "--json"]);
    invoke_ok(&["focus", "t1", "--json"]);
    let moved_pane = invoke_ok(&["pane", "move", "up", "--json"]);
    assert!(stdout(&moved_pane).contains("\"panes\":[{\"id\":\"p2\""));
    invoke_ok(&["pane", "move", "down", "--json"]);

    fs::write(&selection, root.join("missing").as_os_str().as_bytes()).unwrap();
    assert!(matches!(
        workspace_action(&control, "picker-invalid", Action::PickTabDirectory),
        Response::Snapshot(snapshot) if snapshot.directory_picker.is_some()
    ));
    thread::sleep(Duration::from_millis(100));
    assert!(matches!(
        workspace_action(&control, "picker-invalid-visible", Action::Inspect),
        Response::Snapshot(snapshot)
            if snapshot.directory_picker.is_some()
                && snapshot.tabs[1].directory == selected.as_os_str().as_bytes()
    ));
    wait_for_picker_close(&control, &selected);

    fs::remove_file(&release).unwrap();
    fs::write(&selection, initial.as_os_str().as_bytes()).unwrap();
    assert!(matches!(
        workspace_action(&control, "picker-client-loss", Action::PickTabDirectory),
        Response::Snapshot(snapshot) if snapshot.directory_picker.is_some()
    ));
    let lost_client_picker = picker_endpoint(&control);
    let pid = fs::read_to_string(&venus_pid)
        .unwrap()
        .parse::<i32>()
        .unwrap();
    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
    wait_for_picker_close(&control, &selected);
    assert!(!lost_client_picker.exists());

    fs::write(&fail_stop_once, "session-2").unwrap();
    assert!(matches!(
        workspace_action(
            &control,
            "close-t1-partial",
            Action::CloseTab { tab: "t1".into() },
        ),
        Response::Failure(failure)
            if failure.code == "tab-close-failed" && failure.detail.contains("injected stop failure")
    ));
    assert!(matches!(
        workspace_action(&control, "close-t1-partial", Action::CloseTab { tab: "t1".into() }),
        Response::Failure(failure) if failure.code == "duplicate-request"
    ));
    assert!(matches!(
        workspace_action(&control, "close-partial-inspect", Action::Inspect),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1"
                && snapshot.tabs[0].panes.len() == 1
                && snapshot.tabs[0].panes[0].session == "session-2"
                && snapshot.tabs[1].panes[0].session == "session-3"
    ));
    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["tab", "close", "t1", "--json"],
    );
    assert!(closed.status.success(), "{}", stdout(&closed));
    assert!(stdout(&closed).contains("\"active_tab\":\"t3\""));
    assert!(matches!(
        workspace_action(
            &control,
            "close-final-tab",
            Action::CloseTab { tab: "t3".into() },
        ),
        Response::Failure(failure) if failure.code == "unavailable"
    ));

    assert!(matches!(
        workspace_action(&control, "picker-before-stop", Action::PickTabDirectory),
        Response::Snapshot(snapshot) if snapshot.directory_picker.is_some()
    ));
    fs::write(&stop_delay, "3500").unwrap();
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    assert!(stdout(&stopped).contains("\"sessions\":[\"session-3\"]"));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_directory_picker_stop_is_retried_during_supervisor_cleanup() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let fail_stop_once = root.join("fail-stop-once");
    let venus_pid = root.join("venus.pid");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let session_bin = root.join("session-bin");
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\ncat >/dev/null\n",
    );
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    executable(
        &session_bin.join("eon-nu"),
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-directory-picker")).unwrap();
    quick_picker_executable(&session_bin.join("fzf"));

    let child = eon_command(&binary)
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_RUN_CHILD", "1")
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_FAIL_STOP_ONCE", &fail_stop_once)
        .env("EON_TEST_VENUS_PID", &venus_pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_connection(&control);
    wait_for(&venus_pid);
    assert!(matches!(
        workspace_action(&control, "picker", Action::Inspect),
        Response::Snapshot(snapshot)
            if snapshot.directory_picker.is_some() && snapshot.tabs[0].panes.is_empty()
    ));
    let picker = live_identity(&picker_endpoint(&control));
    fs::write(&fail_stop_once, "directory-picker").unwrap();
    let venus = fs::read_to_string(&venus_pid).unwrap().parse().unwrap();
    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(unsafe { libc::kill(venus, libc::SIGTERM) }, 0);

    let status = wait_for_exit(&mut supervisor.child);
    assert!(!status.success());
    let picker_survived_cleanup = Path::new("/proc")
        .join(picker.process_id.to_string())
        .exists();
    fs::write(&stop, "").unwrap();
    wait_for_process_removal(picker.process_id, "test Session did not exit");
    fs::remove_dir_all(root).unwrap();
    assert!(!picker_survived_cleanup);
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

#[test]
fn closed_stdout_pipe_is_not_a_panic() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
        .arg("versions")
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn relative_configuration_root_is_resolved_once() {
    let root = temporary_directory();
    let config = root.join("config");
    let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
        .arg("config-path")
        .current_dir(&root)
        .env("EON_CONFIG_HOME", "config")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout(&output), format!("{}\n", config.display()));
    assert!(config.is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_supervisor_is_a_structured_workspace_failure() {
    let root = temporary_directory();
    let output = invoke(
        Path::new(env!("CARGO_BIN_EXE_eon")),
        &root.join("runtime"),
        &root.join("config"),
        &["workspace", "--json"],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).contains("\"code\":\"missing-supervisor\""));
    assert!(output.stderr.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_focus_is_rejected_before_supervisor_connection() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));

    let missing = invoke(binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(missing.status.code(), Some(2));
    let control = generation_runtime(&runtime).join("eon.sock");
    let too_long = "x".repeat(129);
    let assert_invalid = |id| {
        let output = invoke(binary, &runtime, &config, &["focus", id, "--json"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(stdout(&output).contains("\"code\":\"malformed-action\""));
        assert!(!stdout(&output).contains("run `eon` first"));
        assert!(output.stderr.is_empty());
    };
    for id in ["", too_long.as_str()] {
        assert_invalid(id);
    }

    let listener = UnixListener::bind(&control).unwrap();
    fs::set_permissions(&control, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    for id in ["", too_long.as_str()] {
        assert_invalid(id);
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejected_native_startup_creates_no_session_in_either_product() {
    let root = temporary_directory();
    let config = root.join("config");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "[terminal]\nfont_size = 20\n").unwrap();
    managed_orbit_executable(&orbit);
    let eon = Path::new(env!("CARGO_BIN_EXE_eon"));
    let eonterm = root.join("eonterm");
    symlink(eon, &eonterm).unwrap();
    for response in ["exit 1", "printf wrong-v1", "exec cat >/dev/null"] {
        executable(&venus, &format!("#!/bin/sh\n{response}\n"));
        for (binary, args) in [
            (eon, vec!["run", "--", "/bin/false"]),
            (eonterm.as_path(), vec!["--", "/bin/false"]),
        ] {
            let started = Instant::now();
            let output = eon_command(binary)
                .args(args)
                .env("EON_RUNTIME_DIR", root.join("runtime"))
                .env("EON_CONFIG_HOME", &config)
                .env("EON_ORBIT", &orbit)
                .env("EON_VENUS", &venus)
                .env("EON_TEST_ORBIT_LOG", &orbit_log)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                started.elapsed()
                    < Duration::from_secs(if response.starts_with("exec") { 7 } else { 4 }),
                "startup rejection did not finish within its bound"
            );
            assert!(
                !orbit_log.exists(),
                "native rejection started an Orbit Session"
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("cannot admit Eon Desktop"));
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_terminal_configuration_precedes_supervisor_generation_and_children() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let orbit_log = root.join("orbit.log");
    let venus_log = root.join("venus.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    fs::create_dir(&config).unwrap();
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf started > \"$EON_TEST_VENUS_LOG\"\n",
    );

    for (source, field) in [
        (
            "[terminal]\nbackground_opacity = 1.01\n",
            "background_opacity",
        ),
        ("[terminal]\nbackground_blur = \"yes\"\n", "background_blur"),
        ("[terminal]\nfont_size = nan\n", "font_size"),
        ("[terminal]\nfont_size = 96.1\n", "font_size"),
        ("[terminal]\nline_height = 0.9\n", "line_height"),
        ("[terminal]\nfont_family = ' Font'\n", "font_family"),
        ("[terminal]\nfont_fallbacks = ['']\n", "font_fallbacks"),
        ("[terminal]\ncolumns = 0\n", "columns"),
        ("[terminal]\nrows = 65536\n", "rows"),
        ("[terminal]\ncolumns = 1000\nrows = 1000\n", "columns"),
    ] {
        fs::write(config.join("config.toml"), source).unwrap();
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_VENUS_LOG", &venus_log)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains(field));
        assert!(!runtime.join("generations").exists());
        assert!(!orbit_log.exists() && !venus_log.exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bare_eon_attaches_only_to_the_live_current_generation() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let log = root.join("venus.log");
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$EON_TEST_LOG\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_LOG", &log)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    wait_for(&log);

    let output = eon_command(&binary)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_LOG", &log)
        .output()
        .unwrap();

    assert!(output.status.success());
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    overfill_generation_directory(&runtime);
    let exact = eon_command(&binary)
        .args(["attach", generation_id])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_LOG", &log)
        .output()
        .unwrap();
    assert!(
        exact.status.success(),
        "stdout={} stderr={}",
        stdout(&exact),
        String::from_utf8_lossy(&exact.stderr)
    );
    assert_eq!(
        fs::read_to_string(log).unwrap(),
        format!(
            "--no-decorations\n--application-id\neon\n--background-opacity\n0.8\n--background-blur\n--workspace\n{}\n",
            control.display(),
        )
    );
    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn eonterm_reopens_without_workspace_or_a_second_session() {
    let root = temporary_directory();
    let runtime = root.join("eonterm");
    let config = root.join("config");
    let child_exit = root.join("child-exit");
    let child_pid = root.join("child.pid");
    let orbit_log = root.join("orbit.log");
    let venus_log = root.join("venus.log");
    let presentation_log = root.join("presentation.log");
    let supervisor_log = root.join("supervisor.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let command = root.join("command");
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\ncase \"$*\" in *EonMissingProofFont*) exit 1;; esac\nif test ! -e \"$EON_TEST_VENUS_LOG\"; then test ! -e \"$EON_TEST_ORBIT_LOG\" || exit 97; fi\nprintf ready-v1\nprintf '%s|%s|%s\\n' \"$$\" \"$EON_VENUS_PRESENTATION_CONTROL\" \"$*\" >> \"$EON_TEST_VENUS_LOG\"\ndd bs=8 count=1 status=none >> \"$EON_TEST_PRESENTATION_LOG\"\ncat >/dev/null\n",
    );
    executable(
        &command,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_CHILD_PID\"\nwhile test ! -e \"$EON_TEST_CHILD_EXIT\"; do sleep 0.01; done\n",
    );
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_opacity = 0.88\nbackground_blur = false\nfont_size = 20\ncolumns = 100\nrows = 30\n",
    )
    .unwrap();

    let eon = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let binary = root.join("bin/eonterm");
    fs::create_dir(root.join("bin")).unwrap();
    symlink(&eon, &binary).unwrap();
    let eonterm = |configuration: &Path| {
        let mut process = eon_command(&binary);
        process
            .env_remove("EON_RUNTIME_DIR")
            .env("XDG_RUNTIME_DIR", &root)
            .env("EON_CONFIG_HOME", configuration)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_CHILD_EXIT", &child_exit)
            .env("EON_TEST_CHILD_PID", &child_pid)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_RUN_CHILD", "1")
            .env("EON_TEST_VENUS_LOG", &venus_log)
            .env("EON_TEST_PRESENTATION_LOG", &presentation_log);
        process
    };
    let invalid = eonterm(&config)
        .args(["--application-id", "bad/id", "--"])
        .arg(&command)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid Eon application identity"));
    assert!(!runtime.join("generations").exists());

    let child = eonterm(&config)
        .args(["--no-decorations", "--application-id", "eonova", "--"])
        .arg(&command)
        .stdout(Stdio::null())
        .stderr(fs::File::create(&supervisor_log).unwrap())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: child_exit.clone(),
    };
    let generation = generation_runtime(&runtime);
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    wait_for(&generation.join("eon.sock"));
    wait_for(&venus_log);
    wait_for(&child_pid);
    let initial_venus: u32 = fs::read_to_string(&venus_log)
        .unwrap()
        .split('|')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let listed = eon_command(&binary)
        .args(["generations", "--json"])
        .env_remove("EON_RUNTIME_DIR")
        .env("XDG_RUNTIME_DIR", &root)
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(stdout(&listed).contains(generation_id));

    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_opacity = 0.0\nfont_size = 24\nline_height = 1.5\nrows = 24\n",
    )
    .unwrap();
    let mut repeated = eonterm(&command)
        .args(["--", "/bin/false"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_successful_exit(&mut repeated);
    wait_for(&presentation_log);
    assert_eq!(fs::read_to_string(&presentation_log).unwrap(), "present\n");
    assert_eq!(fs::read_to_string(&venus_log).unwrap().lines().count(), 1);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);

    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(
        unsafe { libc::kill(initial_venus as i32, libc::SIGTERM) },
        0
    );
    wait_for_process_removal(initial_venus, "terminal surface did not exit");
    wait_for(&supervisor_log);
    assert!(
        fs::read_to_string(&supervisor_log)
            .unwrap()
            .contains(&format!(
                "Run `eonterm attach {generation_id}` to reconnect"
            ))
    );

    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_blur = \"yes\"\n",
    )
    .unwrap();
    let rejected = eonterm(&config)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    let rejected_error = String::from_utf8_lossy(&rejected.stderr);
    assert!(rejected_error.contains("cannot present Eon Desktop"));
    assert!(rejected_error.contains("background_blur"));
    assert!(supervisor.child.try_wait().unwrap().is_none());
    fs::write(
        config.join("config.toml"),
        "[terminal]\nfont_family = 'EonMissingProofFont'\n",
    )
    .unwrap();
    let rejected = eonterm(&config)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("cannot admit Eon Desktop"));
    let orbit_pid: i32 = fs::read_to_string(&orbit_log)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let child_pid: i32 = fs::read_to_string(&child_pid).unwrap().parse().unwrap();
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    assert_eq!(unsafe { libc::kill(orbit_pid, 0) }, 0);
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    assert_eq!(unsafe { libc::kill(child_pid, 0) }, 0);
    assert_eq!(fs::read_to_string(&venus_log).unwrap().lines().count(), 1);

    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_opacity = 0.0\nfont_size = 24\nline_height = 1.5\nrows = 24\n",
    )
    .unwrap();
    let reopened = eonterm(&config)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert!(reopened.status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&venus_log).unwrap().lines().count() != 2 {
        assert!(Instant::now() < deadline, "EonTerm did not reopen");
        thread::sleep(Duration::from_millis(10));
    }
    let refocused = eonterm(&config)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert!(refocused.status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&presentation_log)
        .unwrap()
        .lines()
        .count()
        != 2
    {
        assert!(
            Instant::now() < deadline,
            "reopened EonTerm was not presented"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    assert_eq!(unsafe { libc::kill(orbit_pid, 0) }, 0);
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    assert_eq!(unsafe { libc::kill(child_pid, 0) }, 0);
    let venus_log = fs::read_to_string(&venus_log).unwrap();
    let venus_pids = venus_log
        .lines()
        .map(|line| line.split('|').next().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        venus_log
            .matches(
                "|stdin-ready-v1|--no-decorations --application-id eonova --background-opacity "
            )
            .count(),
        2
    );
    assert!(venus_log.contains("--background-opacity 0.88"));
    assert!(venus_log.contains("--background-opacity 0 --background-blur"));
    assert_eq!(venus_log.matches("--background-blur").count(), 1);
    assert!(venus_log.contains("--font-size 20 --columns 100 --rows 30"));
    assert!(venus_log.contains("--font-size 24 --line-height 1.5 --rows 24"));
    assert_eq!(
        venus_log
            .matches(&generation.join("orbit.sock").display().to_string())
            .count(),
        2
    );

    let workspace = invoke(&eon, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(workspace.status.code(), Some(2));
    assert!(stdout(&workspace).contains("\"code\":\"workspace-unavailable\""));
    let wrong_mode = invoke(&binary, &runtime, &config, &[]);
    assert!(!wrong_mode.status.success());
    assert!(String::from_utf8_lossy(&wrong_mode.stderr).contains("usage: eonterm"));
    let removed = invoke(
        &eon,
        &root.join("removed-runtime"),
        &config,
        &["terminal", "--", "/bin/false"],
    );
    assert!(!removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stderr).contains("usage: eon "));
    let unknown = eon_command(&binary)
        .args(["stop", "g1-00000000000000000000000000000000", "--json"])
        .env_remove("EON_RUNTIME_DIR")
        .env("XDG_RUNTIME_DIR", &root)
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stdout(&unknown).contains("\"code\":\"unknown-generation\""));

    fs::write(&child_exit, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    for pid in venus_pids {
        assert!(!Path::new("/proc").join(pid).exists());
    }
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn eonterm_replaces_exact_residue_after_its_supervisor_and_session_die() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let first_stop = root.join("first-stop");
    let second_stop = root.join("second-stop");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");

    let eon = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let binary = root.join("eonterm");
    symlink(&eon, &binary).unwrap();
    let launch = |stop: &Path| {
        eon_command(&binary)
            .args(["--application-id", "eonova", "--", "/bin/false"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };

    let mut first = launch(&first_stop);
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    let orbit_socket = generation.join("orbit.sock");
    wait_for_connection(&control);
    wait_for(&orbit_log);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    let orbit_pid = live_identity(&orbit_socket).process_id;
    first.kill().unwrap();
    first.wait().unwrap();
    // SAFETY: the test owns this exact process identity.
    assert_eq!(unsafe { libc::kill(orbit_pid as i32, libc::SIGKILL) }, 0);
    wait_for_process_removal(orbit_pid, "dead Session was not reaped");
    assert!(artifact_path(&orbit_socket, ".record").exists());
    fs::remove_file(&orbit_log).unwrap();

    let mut second = launch(&second_stop);
    wait_for_connection(&control);
    wait_for(&orbit_log);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(&eon, &runtime, &config, &["stop", generation_id, "--json"]);
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut second);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_second_cli_receives_committed_workspace_and_controls_three_sessions() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_DELAY_SESSION_2_MS", "3000")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);

    let pane = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(
        pane.status.success(),
        "stdout={} stderr={}",
        stdout(&pane),
        String::from_utf8_lossy(&pane.stderr)
    );
    assert_eq!(
        fs::metadata(&control).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(stdout(&pane).matches("\"session\":").count(), 2);
    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create", "--json"])
            .status
            .success()
    );
    for arguments in [
        &["focus", "p1", "--json"][..],
        &["focus", "down", "--json"][..],
        &["focus", "up", "--json"][..],
    ] {
        assert!(
            invoke(&binary, &runtime, &config, arguments)
                .status
                .success()
        );
    }

    let wrapped = invoke(&binary, &runtime, &config, &["focus", "up", "--json"]);
    assert!(wrapped.status.success());
    assert!(stdout(&wrapped).contains("\"selected_pane\":\"p3\""));

    let before = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert!(before.status.success());
    let rejected = invoke(&binary, &runtime, &config, &["focus", "left", "--json"]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(stdout(&rejected).contains("\"code\":\"unavailable\""));
    let after = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(stdout(&before), stdout(&after));
    let human = invoke(&binary, &runtime, &config, &["workspace"]);
    assert!(human.status.success());
    assert!(stdout(&human).contains("active t1\n"));
    assert_eq!(stdout(&human).matches("  pane ").count(), 3);

    let mut incompatible = encode_request(&Request {
        id: "incompatible-1".into(),
        action: Action::Inspect,
    })
    .unwrap();
    incompatible[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
    let mut client = UnixStream::connect(&control).unwrap();
    client.write_all(&incompatible).unwrap();
    let mut response = vec![0; HEADER_BYTES];
    client.read_exact(&mut response).unwrap();
    let length = declared_message_len(&response).unwrap();
    response.resize(length, 0);
    client.read_exact(&mut response[HEADER_BYTES..]).unwrap();
    assert!(matches!(
        decode_response(&response).unwrap(),
        Response::Failure(failure) if failure.code == "unsupported-version"
    ));

    let snapshot = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert!(snapshot.status.success());
    let snapshot = stdout(&snapshot);
    assert!(snapshot.contains("\"active_tab\":\"t1\""));
    assert!(snapshot.contains("\"id\":\"p3\""));
    assert_eq!(snapshot.matches("\"session\":").count(), 3);

    for endpoint in ["orbit.sock", "session-2.sock", "session-3.sock"] {
        let endpoint = generation.join(endpoint);
        wait_for(&endpoint);
        let pid = live_identity(&endpoint).process_id as i32;
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    }

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn launch_directory_can_disappear_after_the_initial_session_starts() {
    let root = temporary_directory();
    let launch = root.join("launch");
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    fs::create_dir(&launch).unwrap();
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .arg("run")
        .current_dir(&launch)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_REMOVE_ORBIT_CWD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let control = generation_runtime(&runtime).join("eon.sock");
    wait_for_connection(&control);

    let snapshot = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert!(snapshot.status.success());
    assert!(stdout(&snapshot).contains(&format!(
        "\"directory\":[{}]",
        launch
            .as_os_str()
            .as_bytes()
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )));

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_directory_retargets_future_sessions_without_crossing_tabs() {
    let root = temporary_directory();
    let first = root.join("first");
    let second = root.join("second");
    let disappearing = root.join("disappearing");
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit_log = root.join("orbit-cwd.log");
    let child_log = root.join("child-cwd.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let reporter = root.join("report-cwd");
    let picker_release = root.join("picker-release");
    let session_bin = root.join("session-bin");
    for directory in [&first, &second, &disappearing, &config] {
        fs::create_dir(directory).unwrap();
    }
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    executable(
        &reporter,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" >> \"$EON_TEST_CHILD_CWD_LOG\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_PICKER_RELEASE\"; do sleep 0.01; done\nprintf '%s\\n' \"$EON_TEST_PICKER_SELECTION\"\n",
    );
    fs::write(
        config.join("config.toml"),
        format!("[shell]\ncommand = [\"{}\"]\n", reporter.to_string_lossy()),
    )
    .unwrap();

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-directory-picker")).unwrap();
    quick_picker_executable(&session_bin.join("fzf"));
    let child = eon_command(&binary)
        .arg("run")
        .current_dir(&first)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_PICKER_RELEASE", &picker_release)
        .env("EON_TEST_PICKER_SELECTION", &first)
        .env("EON_TEST_ORBIT_CWD_LOG", &orbit_log)
        .env("EON_TEST_CHILD_CWD_LOG", &child_log)
        .env("EON_TEST_RUN_CHILD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_connection(&control);
    fs::write(&picker_release, "").unwrap();
    wait_for_picker_close(&control, &first);

    assert!(
        invoke(&binary, &runtime, &config, &["tab", "create"])
            .status
            .success()
    );
    wait_for_picker_close(&control, &first);
    assert!(
        invoke(
            &binary,
            &runtime,
            &config,
            &["tab", "directory", "t1", "--", second.to_str().unwrap()],
        )
        .status
        .success()
    );
    assert!(
        invoke(&binary, &runtime, &config, &["focus", "t1"])
            .status
            .success()
    );
    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create"])
            .status
            .success()
    );
    assert!(
        invoke(&binary, &runtime, &config, &["focus", "t2"])
            .status
            .success()
    );
    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create"])
            .status
            .success()
    );
    assert!(
        invoke(
            &binary,
            &runtime,
            &config,
            &[
                "tab",
                "directory",
                "t1",
                "--",
                disappearing.to_str().unwrap(),
            ],
        )
        .status
        .success()
    );
    fs::remove_dir(&disappearing).unwrap();
    assert!(
        invoke(&binary, &runtime, &config, &["focus", "t1"])
            .status
            .success()
    );
    let before = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    let failed = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert_eq!(failed.status.code(), Some(2));
    assert!(stdout(&failed).contains("\"code\":\"session-start\""));
    let after = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(stdout(&before), stdout(&after));
    assert!(!generation.join("session-5.sock").exists());
    assert!(!artifact_path(&generation.join("session-5.sock"), ".record").exists());

    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&orbit_log)
        .unwrap_or_default()
        .lines()
        .count()
        < 4
        || fs::read_to_string(&child_log)
            .unwrap_or_default()
            .lines()
            .count()
            < 4
    {
        assert!(
            Instant::now() < deadline,
            "directory reports did not arrive"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let orbit_directories = fs::read_to_string(&orbit_log).unwrap();
    for expected in [
        format!("session-1 {}", first.display()),
        format!("session-2 {}", first.display()),
        format!("session-3 {}", second.display()),
        format!("session-4 {}", first.display()),
    ] {
        assert!(orbit_directories.lines().any(|line| line == expected));
    }
    let child_directories = fs::read_to_string(&child_log).unwrap();
    assert_eq!(
        child_directories
            .lines()
            .filter(|line| *line == first.to_string_lossy())
            .count(),
        3
    );
    assert_eq!(
        child_directories
            .lines()
            .filter(|line| *line == second.to_string_lossy())
            .count(),
        1
    );

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn session_exit_prunes_the_workspace_and_the_last_exit_closes_eon() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let exit_initial = root.join("exit-initial");
    let venus_pid = root.join("venus.pid");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\ncat >/dev/null\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_EXIT_INITIAL", &exit_initial)
        .env("EON_TEST_VENUS_PID", &venus_pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    wait_for(&venus_pid);
    let pane = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(
        pane.status.success(),
        "stdout={} stderr={}",
        stdout(&pane),
        String::from_utf8_lossy(&pane.stderr)
    );

    fs::write(&exit_initial, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
        if snapshot.status.success()
            && !stdout(&snapshot).contains("\"id\":\"p1\"")
            && stdout(&snapshot).contains("\"id\":\"p2\"")
        {
            break;
        }
        assert!(Instant::now() < deadline, "initial pane was not removed");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(supervisor.child.try_wait().unwrap().is_none());
    let pid: i32 = fs::read_to_string(&venus_pid).unwrap().parse().unwrap();
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn launch_overlapping_last_session_exit_starts_a_fresh_session() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let child_exit = root.join("child-exit");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let command = root.join("command");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\nsleep 2\n");
    executable(
        &command,
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_CHILD_EXIT\"; do sleep 0.01; done\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .args(["run", "--"])
        .arg(&command)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_CHILD_EXIT", &child_exit)
        .env("EON_TEST_ORBIT_LOG", &orbit_log)
        .env("EON_TEST_RUN_CHILD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut first = TestProcess {
        child,
        stop: child_exit.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    let record = artifact_path(&generation.join("orbit.sock"), ".record");
    wait_for_connection(&control);
    assert!(matches!(
        workspace_action(&control, "startup-ready", Action::Inspect),
        Response::Snapshot(_)
    ));
    wait_for(&record);
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let lifecycle_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(runtime.join(format!("supervisor-{generation_id}.lock")))
        .unwrap();
    assert!(matches!(
        lifecycle_lock.try_lock(),
        Err(fs::TryLockError::WouldBlock)
    ));
    drop(lifecycle_lock);
    let first_orbit = live_identity(&generation.join("orbit.sock")).process_id;

    // Freeze the owner so the replacement launch necessarily overlaps teardown.
    assert_eq!(
        unsafe { libc::kill(first.child.id() as i32, libc::SIGSTOP) },
        0
    );
    fs::write(&child_exit, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read(&record).ok().is_some_and(|bytes| {
            matches!(
                management::decode_record(&bytes),
                Ok(ManagementRecord::Tombstone(_))
            )
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "initial Session did not exit");
        thread::sleep(Duration::from_millis(10));
    }

    let second = eon_command(&binary)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_ORBIT_LOG", &orbit_log)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut second = TestProcess {
        child: second,
        stop: root.join("unused-stop"),
    };
    thread::sleep(Duration::from_secs(5));
    assert_eq!(
        unsafe { libc::kill(first.child.id() as i32, libc::SIGCONT) },
        0
    );
    wait_for_successful_exit(&mut first.child);

    let deadline = Instant::now() + Duration::from_secs(5);
    let second_orbit = loop {
        if let Ok(bytes) = fs::read(&record)
            && let Ok(ManagementRecord::Live(identity)) = management::decode_record(&bytes)
            && identity.process_id != first_orbit
        {
            break identity.process_id;
        }
        assert!(
            second.child.try_wait().unwrap().is_none(),
            "replacement launch exited instead of becoming the new supervisor"
        );
        assert!(Instant::now() < deadline, "fresh Session did not start");
        thread::sleep(Duration::from_millis(10));
    };
    assert_ne!(second_orbit, first_orbit);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 2);
    wait_for_connection(&control);

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut second.child);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacement_reconciles_exact_tombstone_endpoints_without_deleting_replacements() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let replacement = || {
        let mut command = eon_command(&binary);
        command
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    };
    let mut first = TestProcess {
        child: replacement().spawn().unwrap(),
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    let presentation = generation.join("orbit.sock");
    let management = artifact_path(&presentation, ".management");
    let record = artifact_path(&presentation, ".record");
    wait_for_connection(&control);
    wait_for(&orbit_log);
    let identity = live_identity(&presentation);

    first.child.kill().unwrap();
    assert!(!first.child.wait().unwrap().success());
    // SAFETY: the test owns this exact managed-Orbit process identity.
    assert_eq!(
        unsafe { libc::kill(identity.process_id as i32, libc::SIGKILL) },
        0
    );
    wait_for_process_removal(identity.process_id, "dead Session was not reaped");
    let tombstone = |identity: &LiveIdentity| {
        ManagementRecord::Tombstone(Tombstone {
            identity: identity.clone(),
            reason: TerminationReason::NaturalExit,
            outcome: ProcessOutcome::Signal(libc::SIGKILL),
        })
    };
    write_management_record(&record, &tombstone(&identity));
    let retained_record = object_identity(&record);
    fs::remove_file(&control).unwrap();

    let refused = replacement().spawn().unwrap();
    fs::remove_file(&presentation).unwrap();
    let mut presentation_listener = UnixListener::bind(&presentation).unwrap();
    if object_identity(&presentation) == identity.presentation.object {
        fs::remove_file(&presentation).unwrap();
        let collision_guard = presentation_listener;
        presentation_listener = UnixListener::bind(&presentation).unwrap();
        drop(collision_guard);
    }
    fs::set_permissions(&presentation, fs::Permissions::from_mode(0o600)).unwrap();
    let replacement_object = object_identity(&presentation);
    assert_ne!(replacement_object, identity.presentation.object);
    fs::remove_file(&management).unwrap();
    let refused = refused.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(refused.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("replaced during cleanup"), "{stderr}");
    assert_eq!(object_identity(&presentation), replacement_object);
    assert_eq!(object_identity(&record), retained_record);
    assert!(!control.exists());
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);

    let management_listener = UnixListener::bind(&management).unwrap();
    fs::set_permissions(&management, fs::Permissions::from_mode(0o600)).unwrap();
    let mut reconciled = identity;
    reconciled.presentation = endpoint_identity(&presentation);
    reconciled.management = endpoint_identity(&management);
    write_management_record(&record, &tombstone(&reconciled));
    let tombstone_record = object_identity(&record);

    let waiting = replacement().spawn().unwrap();
    fs::remove_file(&presentation).unwrap();
    drop(presentation_listener);
    let waiting = waiting.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&waiting.stderr);
    assert_eq!(waiting.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("ended Sessions cleanup exceeded five seconds"),
        "{stderr}"
    );
    assert_eq!(object_identity(&record), tombstone_record);
    assert!(!control.exists());
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    fs::remove_file(&management).unwrap();
    drop(management_listener);

    let mut recovered = TestProcess {
        child: replacement().spawn().unwrap(),
        stop: stop.clone(),
    };
    wait_for_connection(&control);
    assert_ne!(live_identity(&presentation).run_id, reconciled.run_id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&orbit_log).unwrap().lines().count() != 2 {
        assert!(Instant::now() < deadline, "fresh Session count changed");
        thread::sleep(Duration::from_millis(10));
    }
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut recovered.child);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacement_with_only_a_stale_initial_picker_falls_back_without_reopening_it() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let lease_released = root.join("lease-released");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let session_bin = root.join("session-bin");
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(
        &session_bin.join("eon-nu"),
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-directory-picker")).unwrap();
    quick_picker_executable(&session_bin.join("fzf"));
    let launch = || {
        eon_command(&binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_SESSION_BIN", &session_bin)
            .env("EON_TEST_RUN_CHILD", "1")
            .env("EON_TEST_STOP", &stop)
            .env("EON_TEST_LEASE_RELEASED", &lease_released)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };

    let mut first = launch();
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_connection(&control);
    let pending = workspace_action(&control, "pending", Action::Inspect);
    assert!(matches!(
        pending,
        Response::Snapshot(snapshot)
            if snapshot.directory_picker.is_some() && snapshot.tabs[0].panes.is_empty()
    ));
    let picker = live_identity(&picker_endpoint(&control));
    first.kill().unwrap();
    assert!(!first.wait().unwrap().success());
    wait_for(&lease_released);

    let child = launch();
    let mut replacement = TestProcess {
        child,
        stop: stop.clone(),
    };
    wait_for_connection(&control);
    let recovered = workspace_action(&control, "recovered", Action::Inspect);
    assert!(matches!(
        recovered,
        Response::Snapshot(snapshot)
            if snapshot.directory_picker.is_none()
                && snapshot.tabs[0].selected_pane.as_deref() == Some("p1")
                && snapshot.tabs[0].panes.len() == 1
    ));
    assert!(
        !Path::new("/proc")
            .join(picker.process_id.to_string())
            .exists()
    );

    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut replacement.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacement_eon_adopts_exact_runs_and_projects_numeric_workspace() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let fallback_stop = root.join("fallback-stop");
    let orbit_log = root.join("orbit.log");
    let venus_log = root.join("venus.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$$\" \"$EON_VENUS_PRESENTATION_CONTROL\" \"$*\" >> \"$EON_TEST_VENUS_LOG\"\ncat >/dev/null\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let launch = || {
        eon_command(&binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &fallback_stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_VENUS_LOG", &venus_log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = launch();
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    wait_for(&venus_log);
    let pane = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(pane.status.success(), "{}", stdout(&pane));
    let initial = [
        live_identity(&generation.join("orbit.sock")),
        live_identity(&generation.join("session-2.sock")),
    ];
    let first_venus_log = fs::read_to_string(&venus_log).unwrap();
    let first_venus: u32 = first_venus_log.split('|').next().unwrap().parse().unwrap();
    assert!(first_venus_log.contains("|--no-decorations "));

    first.kill().unwrap();
    assert!(!first.wait().unwrap().success());
    wait_for_process_removal(first_venus, "owner-loss Venus did not exit");
    for identity in &initial {
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        assert_eq!(unsafe { libc::kill(identity.process_id as i32, 0) }, 0);
    }
    let first_record = artifact_path(&generation.join("orbit.sock"), ".record");
    let assert_recovery_failure = |program: &Path, arguments: &[&str], expected: &str| {
        let refused = eon_command(program)
            .args(arguments)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &fallback_stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_VENUS_LOG", &venus_log)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(refused.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
        assert!(UnixStream::connect(&control).is_err());
    };
    let decoys = (0..MAX_PANES)
        .map(|number| generation.join(format!("limit-{number}.record")))
        .collect::<Vec<_>>();
    for decoy in &decoys {
        fs::write(decoy, []).unwrap();
    }
    assert_recovery_failure(&binary, &["run"], "recovery limit");
    for decoy in decoys {
        fs::remove_file(decoy).unwrap();
    }

    let wrong_record = generation.join("wrong.record");
    fs::rename(&first_record, &wrong_record).unwrap();
    assert_recovery_failure(&binary, &["run"], "wrong presentation identity");
    fs::rename(&wrong_record, &first_record).unwrap();

    let eonterm = root.join("eonterm");
    symlink(&binary, &eonterm).unwrap();
    assert_recovery_failure(
        &eonterm,
        &["--", "/bin/false"],
        "EonTerm cannot recover more than one live Session",
    );

    fs::set_permissions(&first_record, fs::Permissions::from_mode(0o640)).unwrap();
    assert_recovery_failure(&binary, &["run"], "mode-0600");
    fs::set_permissions(&first_record, fs::Permissions::from_mode(0o600)).unwrap();

    let mut wrong_process = initial[0].clone();
    wrong_process.process_start += 1;
    write_management_record(&first_record, &ManagementRecord::Live(wrong_process));
    assert_recovery_failure(&binary, &["run"], "process start");
    write_management_record(&first_record, &ManagementRecord::Live(initial[0].clone()));

    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 2);
    for identity in &initial {
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        assert_eq!(unsafe { libc::kill(identity.process_id as i32, 0) }, 0);
    }

    let mut replacement = launch();
    wait_for_connection(&control);
    let recovered = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert!(recovered.status.success(), "{}", stdout(&recovered));
    let recovered = stdout(&recovered);
    assert!(recovered.contains("\"active_tab\":\"t1\""));
    assert!(recovered.contains("\"selected_pane\":\"p1\""));
    assert_eq!(recovered.matches("\"session\":").count(), 2);
    assert!(recovered.contains("\"id\":\"p1\""));
    assert!(recovered.contains("\"id\":\"p2\""));
    assert!(recovered.contains("\"directory_picker\":null"));
    assert_eq!(
        [
            live_identity(&generation.join("orbit.sock")),
            live_identity(&generation.join("session-2.sock")),
        ],
        initial
    );
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 2);

    let created = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(created.status.success(), "{}", stdout(&created));
    assert!(stdout(&created).contains("\"id\":\"p3\""));
    assert!(stdout(&created).contains("\"session\":\"session-3\""));
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 3);

    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    assert!(stdout(&stopped).contains("\"sessions\":[\"session-1\",\"session-2\",\"session-3\"]"));
    wait_for_successful_exit(&mut replacement);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_launch_failure_stops_the_ready_session_through_management() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    managed_orbit_executable(&orbit);

    let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", root.join("missing-venus"))
        .env("EON_TEST_ORBIT_LOG", &orbit_log)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot launch Eon Desktop"));
    let orbit_pid: i32 = fs::read_to_string(&orbit_log)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!Path::new("/proc").join(orbit_pid.to_string()).exists());
    assert_eq!(
        fs::read_dir(runtime.join("generations")).unwrap().count(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejected_ready_identity_stops_the_spawned_session_through_management() {
    for (variable, value, expected_error) in [
        (
            "EON_TEST_ORBIT_COMPONENT_GENERATION",
            "wrong-generation",
            "Sessions record reports component generation wrong-generation",
        ),
        (
            "EON_TEST_ORBIT_SESSION_ID",
            "session-2",
            "uses unexpected presentation endpoint",
        ),
    ] {
        let root = temporary_directory();
        let runtime = root.join("runtime");
        let config = root.join("config");
        let orbit_log = root.join("orbit.log");
        let orbit = root.join("orbit");
        managed_orbit_executable(&orbit);

        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", root.join("missing-venus"))
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env(variable, value)
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains(expected_error), "{stderr}");
        let orbit_pid: i32 = fs::read_to_string(&orbit_log)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!Path::new("/proc").join(orbit_pid.to_string()).exists());
        let generation = fs::read_dir(runtime.join("generations"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(generation.len(), 1);
        assert!(
            fs::read_dir(&generation[0]).unwrap().next().is_none(),
            "rejected Ready left residue in {}",
            generation[0].display()
        );

        let retry = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", root.join("missing-venus"))
            .output()
            .unwrap();
        let retry_stderr = String::from_utf8_lossy(&retry.stderr);
        assert_eq!(retry.status.code(), Some(1), "{retry_stderr}");
        assert!(
            retry_stderr.contains("cannot launch Eon Desktop"),
            "{retry_stderr}"
        );
        assert_eq!(
            fs::read_dir(runtime.join("generations")).unwrap().count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn marked_ready_claim_never_falls_back_to_child_stop() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let orbit_log = root.join("orbit.log");
    let fallback_stop = root.join("fallback-stop");
    let orbit = root.join("orbit");
    managed_orbit_executable(&orbit);

    let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", root.join("missing-venus"))
        .env("EON_TEST_ORBIT_LOG", &orbit_log)
        .env("EON_TEST_REPLACE_READY_RECORD", "1")
        .env("EON_TEST_STOP", &fallback_stop)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("invalid Sessions record"), "{stderr}");
    let orbit_pid: i32 = fs::read_to_string(&orbit_log)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let process = Path::new("/proc").join(orbit_pid.to_string());
    assert!(process.exists(), "marked Ready helper was killed");

    fs::write(&fallback_stop, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while process.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!process.exists(), "marked Ready helper did not exit");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn lost_stop_response_still_finishes_the_supervisor() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let fallback_stop = root.join("fallback-stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let mut supervisor = TestProcess {
        child: eon_command(&binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &fallback_stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: fallback_stop,
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_connection(&control);
    let orbit_pid = live_identity(&generation.join("orbit.sock")).process_id;
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let request = encode_request(&Request {
        id: "lost-stop-response".into(),
        action: Action::Stop {
            generation: generation_id.into(),
        },
    })
    .unwrap();
    let mut stream = UnixStream::connect(control).unwrap();
    stream.write_all(&request).unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    drop(stream);

    wait_for_successful_exit(&mut supervisor.child);
    assert!(!generation.exists());
    assert!(!Path::new("/proc").join(orbit_pid.to_string()).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stop_confirmation_refuses_a_replacement_supervisor() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let first_stop = root.join("first-stop");
    let second_stop = root.join("second-stop");
    let prompt = root.join("prompt");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let launch = |stop: &Path| {
        eon_command(&binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };

    let mut first = TestProcess {
        child: launch(&first_stop),
        stop: first_stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    let generation_id = generation.file_name().unwrap().to_str().unwrap();

    let mut confirmation = eon_command(&binary)
        .args(["stop", generation_id])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(fs::File::create(&prompt).unwrap()))
        .spawn()
        .unwrap();
    wait_for(&prompt);

    let stopped_first = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped_first.status.success(), "{}", stdout(&stopped_first));
    wait_for_successful_exit(&mut first.child);

    let mut second = TestProcess {
        child: launch(&second_stop),
        stop: second_stop.clone(),
    };
    wait_for(&control);
    let orbit_socket = generation.join("orbit.sock");
    wait_for(&orbit_socket);

    confirmation
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"y\n")
        .unwrap();
    let attempted = confirmation.wait_with_output().unwrap();
    let prompt = fs::read_to_string(&prompt).unwrap();
    let replacement_live = second.child.try_wait().unwrap().is_none();
    let replacement_pid = live_identity(&orbit_socket).process_id as i32;
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    let replacement_child_live = unsafe { libc::kill(replacement_pid, 0) } == 0;
    let workspace_live = invoke(&binary, &runtime, &config, &["workspace", "--json"])
        .status
        .success();
    let stopped_second = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    wait_for_successful_exit(&mut second.child);
    fs::remove_dir_all(root).unwrap();

    assert_eq!(
        attempted.status.code(),
        Some(2),
        "stdout={} prompt={prompt}",
        stdout(&attempted)
    );
    assert!(prompt.contains("supervisor") && prompt.contains("changed"));
    assert!(replacement_live);
    assert!(replacement_child_live);
    assert!(workspace_live);
    assert!(
        stopped_second.status.success(),
        "{}",
        stdout(&stopped_second)
    );
}

#[test]
fn concurrent_launches_converge_and_generation_stop_is_owner_routed() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let fallback_stop = root.join("fallback-stop");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let launch = || {
        eon_command(&binary)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &fallback_stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_ORBIT_DELAY_MS", "250")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = TestProcess {
        child: launch(),
        stop: fallback_stop.clone(),
    };
    let mut second = TestProcess {
        child: launch(),
        stop: fallback_stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    wait_for(&generation.join("eon.sock"));

    let deadline = Instant::now() + Duration::from_secs(5);
    let first_is_supervisor = loop {
        let first_status = first.child.try_wait().unwrap();
        let second_status = second.child.try_wait().unwrap();
        match (first_status, second_status) {
            (None, Some(status)) => {
                assert!(status.success());
                break true;
            }
            (Some(status), None) => {
                assert!(status.success());
                break false;
            }
            (Some(_), Some(_)) => panic!("both concurrent Eon launches exited"),
            (None, None) => {
                assert!(Instant::now() < deadline, "neither Eon launch attached");
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);

    let listed = invoke(&binary, &runtime, &config, &["generations", "--json"]);
    assert!(listed.status.success());
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    assert!(stdout(&listed).contains(&format!("\"id\":\"{generation_id}\"")));
    assert!(stdout(&listed).contains("\"kind\":\"current\",\"state\":\"live\""));
    assert!(stdout(&listed).contains("\"sessions\":[\"session-1\"]"));
    assert!(stdout(&listed).contains("\"stop\":{\"available\":true"));

    overfill_generation_directory(&runtime);
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(
        stopped.status.success(),
        "stdout={} stderr={}",
        stdout(&stopped),
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(stdout(&stopped).contains(&format!(
        "\"generation\":\"{generation_id}\",\"sessions\":[\"session-1\"]"
    )));
    if first_is_supervisor {
        wait_for_successful_exit(&mut first.child);
    } else {
        wait_for_successful_exit(&mut second.child);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_workspace_is_visible_and_attachable_but_not_stoppable() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let venus = root.join("venus");
    let venus_log = root.join("venus.log");
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(runtime.join("orbit.sock"), "legacy-orbit").unwrap();
    let listener = UnixListener::bind(runtime.join("eon.sock")).unwrap();
    fs::set_permissions(runtime.join("eon.sock"), fs::Permissions::from_mode(0o600)).unwrap();
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$EON_TEST_LOG\"\n",
    );
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0; HEADER_BYTES];
            stream.read_exact(&mut request).unwrap();
            let length = declared_message_len(&request).unwrap();
            request.resize(length, 0);
            stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
            assert!(matches!(
                decode_request(&request).unwrap().action,
                Action::Inspect
            ));
            let response = encode_response(&Response::Snapshot(Snapshot {
                active_tab: "t1".into(),
                tabs: vec![Tab {
                    id: "t1".into(),
                    directory: b"/legacy".to_vec(),
                    selected_pane: Some("pane-1".into()),
                    panes: vec![Pane {
                        id: "pane-1".into(),
                        session: "session-1".into(),
                        endpoint: b"/legacy/orbit.sock".to_vec(),
                        live: true,
                    }],
                }],
                directory_picker: None,
            }))
            .unwrap();
            stream.write_all(&response).unwrap();
        }
    });

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let listed = invoke(&binary, &runtime, &config, &["generations", "--json"]);
    assert!(listed.status.success());
    assert!(stdout(&listed).contains("\"id\":\"legacy\",\"kind\":\"legacy\",\"state\":\"live\""));
    assert!(stdout(&listed).contains("legacy supervisor has no authoritative stop action"));

    let attached = eon_command(&binary)
        .args(["attach", "legacy"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_LOG", &venus_log)
        .output()
        .unwrap();
    assert!(attached.status.success());
    assert_eq!(
        fs::read_to_string(&venus_log).unwrap(),
        format!(
            "--application-id\neon\n--background-opacity\n0.8\n--background-blur\n--workspace\n{}\n",
            runtime.join("eon.sock").display()
        )
    );

    let stopped = invoke(&binary, &runtime, &config, &["stop", "legacy", "--json"]);
    assert_eq!(stopped.status.code(), Some(2));
    assert!(stdout(&stopped).contains("\"code\":\"stop-unavailable\""));
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
