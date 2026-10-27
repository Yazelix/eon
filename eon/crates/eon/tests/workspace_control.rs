use eon_workspace_protocol::v20::{
    Action as ProtocolAction, Availability, Column, Direction, Failure as ProtocolFailure,
    HEADER_BYTES, InvokeIntent, LifecycleResponse, MAX_SESSIONS, Pane, Popup, PopupGeometry,
    PopupTarget, Request, Response, Runtime, Snapshot, Stopped, Tab, VERSION,
    WorkspaceAction as Action, declared_message_len, decode_request, decode_response,
    encode_lifecycle_response, encode_request, encode_response,
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
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static NEXT_TEST: AtomicU64 = AtomicU64::new(0);
static CODEX_STUB: OnceLock<PathBuf> = OnceLock::new();

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

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_some() {
            return;
        }
        // SAFETY: this still-unreaped child was spawned as its own group leader.
        let result = unsafe { libc::kill(-i32::try_from(self.0.id()).unwrap(), libc::SIGKILL) };
        assert!(result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH));
        wait_for_exit(&mut self.0);
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

fn executable(path: &Path, source: impl AsRef<[u8]>) {
    // A concurrent test's forked child keeps this process's descriptors until it execs, and
    // exec fails with ETXTBSY while any writer is open, so a child process writes the script.
    let mut writer = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 700 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_ref())
        .unwrap();
    assert!(writer.wait().unwrap().success());
}

fn configure_mock_desktop(config: &Path) {
    fs::create_dir_all(config).unwrap();
    // These lifecycle mocks do not implement native font admission.
    fs::write(
        config.join("config.toml"),
        "[terminal]\nfont_fallbacks = []\n\n[anima]\nenabled = false\n",
    )
    .unwrap();
}

fn quick_picker_executable(path: &Path) {
    executable(
        path,
        r#"#!/bin/sh
[ -z "$FZF_DEFAULT_OPTS$FZF_DEFAULT_OPTS_FILE" ] || exit 99
selection=$(cat)
[ "$selection" != cancel ] || exit 130
printf '\0%s\0' "$selection"
"#,
    );
}

#[test]
fn config_defaults_ignore_user_configuration_and_have_documented_values() {
    let root = temporary_directory();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let mut printed = Vec::new();
    for variable in ["EON_CONFIG_HOME", "XDG_CONFIG_HOME"] {
        let config = root.join(variable);
        fs::create_dir(&config).unwrap();
        let file = if variable == "XDG_CONFIG_HOME" {
            fs::create_dir(config.join("eon")).unwrap();
            config.join("eon/config.toml")
        } else {
            config.join("config.toml")
        };
        fs::write(&file, "broken = [").unwrap();
        for json in [false, true] {
            let mut command = eon_command(binary);
            command.args(["config", "defaults"]);
            if json {
                command.arg("--json");
            }
            let output = command
                .env_remove("EON_CONFIG_HOME")
                .env(variable, &config)
                .env("EON_RUNTIME_DIR", root.join("absent-runtime"))
                .env("EON_ORBIT", root.join("absent-orbit"))
                .env("EON_VENUS", root.join("absent-venus"))
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
            printed.push(stdout(&output).to_owned());
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "broken = [");
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    }
    assert_eq!(printed[0], printed[2]);
    assert_eq!(printed[1], printed[3]);
    assert!(!root.join("absent-runtime").exists());
    let absent_config = root.join("absent-config");
    let output = eon_command(binary)
        .args(["config", "defaults"])
        .env("EON_CONFIG_HOME", &absent_config)
        .env("EON_RUNTIME_DIR", root.join("absent-runtime"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(stdout(&output), printed[0]);
    assert!(!absent_config.exists());
    assert!(!root.join("absent-runtime").exists());
    let document = include_str!("../../../../docs/CONFIGURATION.md");
    let documented = document
        .split("<!-- defaults:start -->\n```toml\n")
        .nth(1)
        .unwrap()
        .split("```\n<!-- defaults:end -->")
        .next()
        .unwrap();
    assert_eq!(documented, printed[0]);
    for arguments in [
        vec!["config"],
        vec!["config", "path"],
        vec!["config", "--json", "defaults"],
        vec!["config", "defaults", "extra"],
        vec!["config", "defaults", "--json", "--json"],
    ] {
        let output = eon_command(binary).args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = eon_command(binary)
        .args(["config", "defaults"])
        .stdout(writer)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_is_readable_and_does_not_start_or_prepare_a_workspace() {
    let root = temporary_directory();
    let config = root.join("config");
    let absent_config = root.join("absent-config");
    let runtime = root.join("runtime");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "invalid configuration = [").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    for (flag, config_home) in [("-h", &config), ("--help", &absent_config)] {
        let output = eon_command(binary)
            .arg(flag)
            .env("EON_CONFIG_HOME", config_home)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env("EON_ORBIT", root.join("missing-orbit"))
            .env("EON_VENUS", root.join("missing-venus"))
            .env("EON_ANIMA", root.join("missing-anima"))
            .env_remove("NO_COLOR")
            .env("TERM", "xterm-256color")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{flag}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let help = stdout(&output);
        assert!(help.contains("terminals") && !help.contains("Sessions"));
        assert!(!help.contains('\u{1b}'), "piped help must be plain text");
        assert!(help.lines().count() > 20);
        assert!(help.lines().all(|line| line.chars().count() <= 80));
        let mut columns = help
            .lines()
            .filter(|line| line.starts_with("  "))
            .map(|line| line.len() - line.rsplit_once("  ").unwrap().1.len());
        let column = columns.next().expect("help includes command descriptions");
        assert!(columns.all(|next| next == column));
        for command in [
            "window attach ID",
            "tab directory TAB",
            "config-path",
            "eon run --",
        ] {
            assert!(help.contains(command), "missing {command}");
        }
        assert!(!runtime.exists());
        assert!(!absent_config.exists());
        assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    }
    for args in [vec!["unknown"], vec!["-h", "extra"], vec!["tab", "close"]] {
        let output = eon_command(binary)
            .args(&args)
            .env("EON_CONFIG_HOME", &absent_config)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("eon --help"));
        assert!(error.len() < 200);
        assert!(!runtime.exists());
        assert!(!root.join("absent-config").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_configuration_keys_warn_once_on_the_managed_cli() {
    let root = temporary_directory();
    for (directory, source, key) in [
        (
            "plain",
            "[terminal]\nfuture_border_width = 2\n",
            "terminal.future_border_width",
        ),
        (
            "line\nbreak",
            "[terminal]\n\"line\\nbreak\" = true\n",
            "terminal.line\\nbreak",
        ),
    ] {
        let home = root.join(directory);
        let config = home.join("eon");
        fs::create_dir_all(&config).unwrap();
        let path = config.join("config.toml");
        fs::write(&path, source).unwrap();
        // The supervisor marks only the default shell it starts, after logging the
        // warning; there stderr is the terminal's scrollback.
        // - No variables: a launch outside Eon must warn.
        // - EON_TERMINAL alone, inherited by explicit commands, popups and a tmux
        //   started inside Eon: fails if those launches lose the warning.
        // - The marker: fails if each new terminal prints the warning again, or if
        //   the shell keeps the marker for what it starts (the -c script checks).
        for (variables, warns) in [
            (&[][..], true),
            (&[("EON_TERMINAL", "fixture-terminal")][..], true),
            (
                &[
                    ("EON_TERMINAL", "fixture-terminal"),
                    ("EON_CONFIG_WARNED", "1"),
                ][..],
                false,
            ),
        ] {
            let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
            command
                .arg0("eon-bash")
                .env("XDG_CONFIG_HOME", &home)
                .env_remove("EON_TERMINAL")
                .env_remove("EON_CONFIG_WARNED")
                .envs(variables.iter().copied());
            let output = command
                .args([
                    "--noprofile",
                    "--norc",
                    "-c",
                    "[ -z \"${EON_CONFIG_WARNED+set}\" ] && printf 'config-loaded\\n'",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(stdout(&output), "config-loaded\n");
            let expected = if warns {
                format!(
                    "eon: ignoring unknown setting {key} in {}\n",
                    path.to_string_lossy().replace('\n', "\\n")
                )
            } else {
                String::new()
            };
            assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn anima_command_preserves_child_arguments_status_and_avoids_workspace() {
    let root = temporary_directory();
    let anima = root.join("anima");
    let log = root.join("args");
    let runtime = root.join("runtime");
    executable(
        &anima,
        "#!/bin/sh\nprintf '%s\\0' \"$YAZELIX_SCREEN_COMMAND_NAME\" \"$@\" > \"$EON_TEST_ANIMA_ARGS\"\nexit 23\n",
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    for arguments in [
        vec![],
        vec!["aquarium"],
        vec!["random", "--duration-seconds", "5"],
        vec!["--help"],
    ] {
        let output = eon_command(binary)
            .arg("anima")
            .args(&arguments)
            .env("EON_ANIMA", &anima)
            .env("EON_TEST_ANIMA_ARGS", &log)
            .env("EON_RUNTIME_DIR", &runtime)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(23));
        let expected = std::iter::once("eon anima")
            .chain(arguments.iter().copied())
            .flat_map(|argument| {
                argument
                    .as_bytes()
                    .iter()
                    .copied()
                    .chain(std::iter::once(0))
            })
            .collect::<Vec<_>>();
        assert_eq!(fs::read(&log).unwrap(), expected);
        assert!(!runtime.exists());
    }
    let unavailable = eon_command(binary)
        .arg("anima")
        .env("EON_ANIMA", root.join("missing"))
        .env("EON_RUNTIME_DIR", &runtime)
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unavailable.stderr).contains("cannot launch Anima"));
    assert!(!runtime.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn anima_waits_for_native_readiness_then_hands_off_picker_and_popup_stops_on_dismissal() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let session_bin = root.join("session-bin");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let anima = root.join("anima");
    let venus_started = root.join("venus-started");
    let venus_ready = root.join("venus-ready");
    let startup_started = root.join("startup-started");
    let startup_release = root.join("startup-release");
    let picker_started = root.join("picker-started");
    let picker_release = root.join("picker-release");
    let popup_pid = root.join("popup-pid");
    let popup_release = root.join("popup-release");
    let stop = root.join("stop");
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf started > \"$EON_TEST_VENUS_STARTED\"\nwhile [ ! -e \"$EON_TEST_VENUS_READY\" ]; do sleep 0.01; done\nprintf ready-v1\ncat >/dev/null\n",
    );
    executable(
        &anima,
        "#!/bin/sh\nif [ \"$2\" = --duration-seconds ]; then\n  printf '%s|%s' \"$1\" \"$3\" > \"$EON_TEST_STARTUP_STARTED\"\n  while [ ! -e \"$EON_TEST_STARTUP_RELEASE\" ]; do sleep 0.01; done\nelse\n  printf '%s' \"$$\" > \"$EON_TEST_POPUP_PID\"\n  while [ ! -e \"$EON_TEST_POPUP_RELEASE\" ]; do sleep 0.01; done\nfi\n",
    );
    symlink(&anima, session_bin.join("anima")).unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-directory-picker")).unwrap();
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nprintf started > \"$EON_TEST_PICKER_STARTED\"\nprintf '/known\\n'\n",
    );
    executable(
        &session_bin.join("fzf"),
        "#!/bin/sh\ncat >/dev/null\nwhile [ ! -e \"$EON_TEST_PICKER_RELEASE\" ]; do sleep 0.01; done\nexit 130\n",
    );
    executable(
        &session_bin.join("eon-nu"),
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let child = eon_command(&binary)
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_ANIMA", &anima)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_RUN_CHILD", "1")
        .env("EON_TEST_VENUS_STARTED", &venus_started)
        .env("EON_TEST_VENUS_READY", &venus_ready)
        .env("EON_TEST_STARTUP_STARTED", &startup_started)
        .env("EON_TEST_STARTUP_RELEASE", &startup_release)
        .env("EON_TEST_PICKER_STARTED", &picker_started)
        .env("EON_TEST_PICKER_RELEASE", &picker_release)
        .env("EON_TEST_POPUP_PID", &popup_pid)
        .env("EON_TEST_POPUP_RELEASE", &popup_release)
        .env("EON_TEST_STOP", &stop)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    wait_for(&venus_started);
    // Admission outlasts session start, so a slow native startup is not killed.
    thread::sleep(Duration::from_secs(6));
    assert!(!startup_started.exists() && !picker_started.exists());
    fs::write(&venus_ready, "ready").unwrap();
    wait_for(&startup_started);
    assert_eq!(fs::read_to_string(&startup_started).unwrap(), "random|3");
    assert!(!picker_started.exists());
    fs::write(&startup_release, "done").unwrap();
    wait_for(&picker_started);
    let control = generation_runtime(&runtime).join("eon.sock");
    let Response::Snapshot(pending) = workspace_action(&control, "anima-pending", Action::Inspect)
    else {
        panic!("initial picker snapshot missing");
    };
    assert!(project_popup(&pending).is_some());
    assert!(pending.tabs[0].columns.is_empty());

    fs::write(&picker_release, "cancel").unwrap();
    wait_for(&generation_runtime(&runtime).join("orbit.sock"));
    let Response::Snapshot(opened) = protocol_action(
        &control,
        "anima-popup-open",
        ProtocolAction::InvokePopup {
            tab: "t1".into(),
            entry: "anima".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
    ) else {
        panic!("Anima popup did not open");
    };
    wait_for(&popup_pid);
    let popup = opened.tabs[0]
        .popups
        .iter()
        .find(|popup| popup.entry == "anima")
        .unwrap();
    let selected_pane = opened.tabs[0].selected_pane.clone();
    let Response::Snapshot(dismissed) = protocol_action(
        &control,
        "anima-popup-dismiss",
        ProtocolAction::DismissPopup(PopupTarget {
            tab: "t1".into(),
            instance: popup.id.clone(),
        }),
    ) else {
        panic!("Anima popup did not dismiss");
    };
    assert!(dismissed.tabs[0].popups.is_empty());
    assert_eq!(dismissed.tabs[0].selected_pane, selected_pane);
    wait_for_process_removal(
        fs::read_to_string(&popup_pid).unwrap().parse().unwrap(),
        "dismissed Anima kept running",
    );
    fs::write(&stop, "stop").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_or_stuck_startup_anima_reports_and_continues_to_picker() {
    let root = temporary_directory();
    let anima = root.join("anima");
    let pid = root.join("anima-pid");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    executable(&bin.join("zoxide"), "#!/bin/sh\nprintf '/known\\n'\n");
    executable(&bin.join("fzf"), "#!/bin/sh\ncat >/dev/null\nexit 130\n");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    for (program, expected) in [
        (root.join("missing"), "cannot launch pinned executable"),
        (anima.clone(), "timed out"),
    ] {
        if program == anima {
            executable(
                &anima,
                "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_ANIMA_PID\"\nexec sleep 30\n",
            );
        }
        let started = Instant::now();
        let output = eon_command(binary)
            .args([
                "__startup-anima",
                "random",
                "1",
                "/tmp/eon-test.sock",
                "t1",
                "u1",
            ])
            .env("EON_ANIMA", &program)
            .env("EON_TEST_ANIMA_PID", &pid)
            .env(
                "PATH",
                std::env::join_paths(
                    std::iter::once(bin.clone())
                        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
                )
                .unwrap(),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    wait_for_process_removal(
        fs::read_to_string(&pid).unwrap().parse().unwrap(),
        "timed-out Anima kept running",
    );
    fs::remove_dir_all(root).unwrap();
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
    if matches!(record, ManagementRecord::Live(identity) if identity.session_id.starts_with("directory-picker"))
        && let Some(delay) = std::env::var_os("EON_TEST_PICKER_READY_DELAY_MS")
    {
        thread::sleep(Duration::from_millis(
            delay.to_str().unwrap().parse().unwrap(),
        ));
    }
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
        format!(
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
        .position(|argument| argument == OsStr::new("--management-v2"))
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
        ManagedChild(
            Command::new(&command[0])
                .args(&command[1..])
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
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
                drop(child.take());
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
            drop(child.take());
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

        let child_status = child.as_mut().and_then(|child| child.0.try_wait().unwrap());
        let initial_exit = presentation.file_name() == Some(OsStr::new("orbit.sock"))
            && std::env::var_os("EON_TEST_EXIT_INITIAL")
                .is_some_and(|path| Path::new(&path).exists());
        let requested_exit =
            std::env::var_os("EON_TEST_STOP").is_some_and(|path| Path::new(&path).exists());
        if child_status.is_some() || initial_exit || requested_exit {
            drop(child.take());
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
    protocol_action(socket, id, ProtocolAction::Workspace(action))
}

fn protocol_action(socket: &Path, id: &str, action: ProtocolAction) -> Response {
    protocol_action_from(socket, id, None, action)
}

fn protocol_action_from(
    socket: &Path,
    id: &str,
    caller: Option<&str>,
    action: ProtocolAction,
) -> Response {
    let request = encode_request(&Request {
        id: id.into(),
        caller: caller.map(str::to_owned),
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

fn project_popup(snapshot: &Snapshot) -> Option<(&Tab, &Popup)> {
    snapshot.tabs.iter().find_map(|tab| {
        tab.popups
            .iter()
            .find(|popup| popup.entry == "project")
            .map(|popup| (tab, popup))
    })
}

fn invoke_project(socket: &Path, id: &str) -> Response {
    let Response::Snapshot(snapshot) =
        workspace_action(socket, &format!("{id}-preflight"), Action::Inspect)
    else {
        panic!("cannot inspect Project popup precondition");
    };
    let tab = snapshot.active_tab.clone();
    let expected_instance = snapshot
        .tabs
        .iter()
        .find(|candidate| candidate.id == tab)
        .and_then(|tab| tab.popups.iter().find(|popup| popup.entry == "project"))
        .map(|popup| popup.id.clone());
    protocol_action(
        socket,
        id,
        ProtocolAction::InvokePopup {
            tab,
            entry: "project".into(),
            expected_instance,
            intent: InvokeIntent::Toggle,
        },
    )
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
            && project_popup(&snapshot).is_none()
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
        &project_popup(&snapshot).unwrap().1.endpoint,
    ))
    .to_path_buf()
}

fn eon_command(binary: &Path) -> Command {
    let codex_stub = CODEX_STUB.get_or_init(|| {
        let directory = std::env::temp_dir().join(format!("eon-codex-stub-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        executable(&directory.join("codex"), "#!/bin/sh\nexit 1\n");
        directory
    });
    let path = std::iter::once(codex_stub.clone())
        .chain(
            std::env::var_os("PATH")
                .as_deref()
                .map(std::env::split_paths)
                .into_iter()
                .flatten(),
        )
        .collect::<Vec<_>>();
    let mut command = Command::new(binary);
    command
        .env("HOME", std::env::temp_dir())
        .env(
            "XDG_STATE_HOME",
            std::env::temp_dir().join("eon-test-state"),
        )
        .env_remove("EON_SESSION_BIN")
        .env("PATH", std::env::join_paths(path).unwrap());
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
        "#!/bin/sh\ncat >/dev/null\nprintf 'tab\\0'\nexit 1\n",
    );
    let mut path = vec![root.clone()];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_eon"));
    command
        .args([
            OsStr::new("__directory-picker"),
            socket.as_os_str(),
            OsStr::new("t1"),
            OsStr::new("u1"),
        ])
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("EON_YAZI", root.join("yazi"))
        .env("EON_DIRECTORY_PICKER_CONFIG", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let returned = root.join("browser-returned");
    command.env("EON_TEST_BROWSER_RETURNED", &returned);
    executable(
        &root.join("yazi"),
        r#"#!/bin/sh
[ "$1" = --cwd-file ] || exit 2
[ "$2" = /dev/stdout ] || exit 2
[ "$3" = --chooser-file ] || exit 2
[ "$5" = -- ] || exit 2

if [ ! -e "$EON_TEST_BROWSER_RETURNED" ]; then
  touch "$EON_TEST_BROWSER_RETURNED"
  printf '/browsed-\377\n'
  exit 10
fi

[ "$6." = "$(printf '/browsed-\377\n.')" ] || exit 2
printf '/parent-\377\n'
printf '/selected-\377\n' > "$4"
"#,
    );
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
        ProtocolAction::CommitDirectory {
            target: PopupTarget {
                tab: "t1".into(),
                instance: "u1".into(),
            },
            directory: b"/selected-\xff\n".to_vec(),
        }
    );
    let response = encode_response(&Response::Snapshot(Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v20::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
        }],
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 0.0,
            vertical_margin: 0.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/untracked-\xff\n".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "session-1".into(),
                    endpoint: b"/test.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v20::Sidebar::Open,
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
            "#!/bin/sh\ncat >/dev/null\nprintf 'tab\\0'\nexit 1\n",
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
        ("browse", "printf 'tab\\0'\nexit 1"),
        ("accept", "printf '\\0/known\\0'"),
        ("failure", "exit 2"),
    ] {
        let _ = fs::remove_file(&history_pid);
        let _ = fs::remove_file(&browser_started);
        executable(
            &root.join("fzf"),
            format!(
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
                    ProtocolAction::CommitDirectory {
                        target: PopupTarget {
                            tab: "t1".into(),
                            instance: "u1".into(),
                        },
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
            "#!/bin/sh\ncat >/dev/null\nprintf 'tab\\0'\nexit 1\n",
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
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let initial = root.join("initial");
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let selected = root.join("selected");
    let stop = root.join("stop");
    let stop_delay = root.join("stop-delay");
    let fail_stop_once = root.join("fail-stop-once");
    let release = root.join("picker-release");
    let selection = root.join("picker-selection");
    let orbit_log = root.join("orbit-cwd.log");
    let venus_pid = root.join("venus.pid");
    let picker_child_pid = root.join("picker-child.pid");
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
        r#"#!/bin/sh
printf '%s' "$$" > "$EON_TEST_VENUS_PID"
printf '%s' "$*" > "$EON_TEST_VENUS_ARGS"
cat >/dev/null
"#,
    );
    executable(
        &session_bin.join("zoxide"),
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_PICKER_CHILD_PID\"\nwhile [ ! -e \"$EON_TEST_PICKER_RELEASE\" ]; do sleep 0.01; done\ncat \"$EON_TEST_PICKER_SELECTION\"\n",
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
        .env("HOME", &home)
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
        .env("EON_TEST_PICKER_CHILD_PID", &picker_child_pid)
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
    let (picker_tab, picker) = project_popup(&opened).unwrap();
    assert_eq!(picker_tab.id, "t1");
    let initial_picker = Path::new(OsStr::from_bytes(&picker.endpoint)).to_path_buf();
    assert_eq!(initial_picker.parent(), Some(generation.as_path()));
    assert!(opened.tabs[0].columns.is_empty());
    assert!(opened.tabs[0].selected_pane.is_none());
    assert!(!generation.join("orbit.sock").exists());
    assert_eq!(
        protocol_action(
            &control,
            "picker-focus",
            ProtocolAction::InvokePopup {
                tab: "t1".into(),
                entry: "project".into(),
                expected_instance: Some(picker.id.clone()),
                intent: InvokeIntent::Focus,
            },
        ),
        Response::Snapshot(opened.clone())
    );
    assert_eq!(
        workspace_action(&control, "picker-singleton", Action::Focus(Direction::Left)),
        Response::Snapshot(opened)
    );

    let two_pending = match workspace_action(&control, "second-pending", Action::CreateTab) {
        Response::Snapshot(snapshot) => snapshot,
        response => panic!("second pending tab failed: {response:?}"),
    };
    assert_eq!(two_pending.active_tab, "t2");
    assert_eq!(two_pending.tabs.len(), 2);
    assert_eq!(
        two_pending.tabs[0].directory,
        initial.as_os_str().as_bytes()
    );
    assert_eq!(two_pending.tabs[1].directory, home.as_os_str().as_bytes());
    assert!(
        fs::read_to_string(&orbit_log)
            .unwrap()
            .contains(&format!("directory-picker-2 {}", home.display()))
    );
    assert!(two_pending.tabs.iter().all(|tab| tab.pending));
    let second_picker = two_pending.tabs[1].popups[0].endpoint.clone();
    assert_ne!(two_pending.tabs[0].popups[0].endpoint, second_picker);
    assert!(matches!(
        workspace_action(&control, "return-to-first-pending", Action::FocusId("t1".into())),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1" && snapshot.tabs[0].popups[0] == two_pending.tabs[0].popups[0]
    ));
    assert!(matches!(
        workspace_action(&control, "return-to-second-pending", Action::FocusId("t2".into())),
        Response::Snapshot(snapshot) if snapshot.active_tab == "t2"
    ));
    assert!(matches!(
        workspace_action(&control, "close-second-pending", Action::CloseTab { tab: "t2".into() }),
        Response::Snapshot(snapshot) if snapshot.active_tab == "t1" && snapshot.tabs.len() == 1
    ));
    assert!(!Path::new(OsStr::from_bytes(&second_picker)).exists());

    fs::write(&release, "").unwrap();
    let retargeted = wait_for_picker_close(&control, &selected);
    assert_eq!(retargeted.tabs[0].panes().count(), 1);
    assert_eq!(retargeted.tabs[0].selected_pane.as_deref(), Some("p1"));
    assert!(!initial_picker.exists());
    assert!(!artifact_path(&initial_picker, ".record").exists());

    let created = workspace_action(
        &control,
        "pane-after-picker",
        Action::CreatePane { focus: true },
    );
    assert!(
        matches!(&created, Response::Snapshot(snapshot) if snapshot.tabs[0].panes().count() == 2),
        "pane creation after picker failed: {created:?}"
    );
    wait_for(&orbit_log);
    let launches = fs::read_to_string(&orbit_log).unwrap();
    assert!(launches.contains(&format!("session-2 {}", selected.display())));

    fs::remove_file(&release).unwrap();
    fs::write(&selection, "cancel").unwrap();
    assert!(matches!(
        invoke_project(&control, "picker-cancel"),
        Response::Snapshot(snapshot) if project_popup(&snapshot).is_some()
    ));
    fs::write(&release, "").unwrap();
    wait_for_picker_close(&control, &selected);

    fs::remove_file(&release).unwrap();
    assert!(matches!(
        workspace_action(&control, "new-tab-cancel", Action::CreateTab),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t3"
                && snapshot.tabs[1].columns.is_empty()
                && snapshot.tabs[1].selected_pane.is_none()
    ));
    let pending_picker = picker_endpoint(&control);
    assert_ne!(pending_picker, initial_picker);
    assert!(matches!(
        workspace_action(
            &control,
            "close-pending-tab",
            Action::CloseTab { tab: "t3".into() },
        ),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1" && snapshot.tabs.len() == 1
    ));
    assert!(!pending_picker.exists());

    fs::write(&selection, selected.as_os_str().as_bytes()).unwrap();
    let traversed_picker = match workspace_action(&control, "new-tab-accept", Action::CreateTab) {
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t4" && snapshot.tabs[1].columns.is_empty() =>
        {
            project_popup(&snapshot).unwrap().1.clone()
        }
        _ => panic!("new tab did not open its picker"),
    };
    assert!(matches!(
        workspace_action(&control, "picker-traverse-left", Action::Focus(Direction::Left)),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t1"
                && project_popup(&snapshot).is_some_and(|(_, popup)| popup == &traversed_picker)
                && snapshot.tabs[0].selected_pane.as_deref() == Some("p2")
    ));
    assert!(matches!(
        workspace_action(&control, "picker-traverse-right", Action::Focus(Direction::Right)),
        Response::Snapshot(snapshot)
            if snapshot.active_tab == "t4"
                && project_popup(&snapshot).is_some_and(|(_, popup)| popup == &traversed_picker)
                && snapshot.tabs[1].columns.is_empty()
                && snapshot.tabs[1].selected_pane.is_none()
    ));
    fs::write(&release, "").unwrap();
    let accepted = wait_for_picker_close(&control, &selected);
    assert_eq!(accepted.active_tab, "t4");
    assert_eq!(accepted.tabs[1].selected_pane.as_deref(), Some("p3"));

    let invoke_ok = |arguments: &[&str]| {
        let output = invoke(&binary, &runtime, &config, arguments);
        assert!(output.status.success());
        output
    };
    let moved_tab = invoke_ok(&["tab", "move", "left", "--json"]);
    assert!(stdout(&moved_tab).contains("\"active_tab\":\"t4\""));
    assert!(stdout(&moved_tab).contains("\"tabs\":[{\"id\":\"t4\""));
    invoke_ok(&["tab", "move", "right", "--json"]);
    invoke_ok(&["focus", "t1", "--json"]);
    let moved_pane = invoke_ok(&["pane", "move", "up", "--json"]);
    assert!(stdout(&moved_pane).contains("\"panes\":[{\"id\":\"p2\""));
    invoke_ok(&["pane", "move", "down", "--json"]);

    fs::write(&selection, root.join("missing").as_os_str().as_bytes()).unwrap();
    assert!(matches!(
        invoke_project(&control, "picker-invalid"),
        Response::Snapshot(snapshot) if project_popup(&snapshot).is_some()
    ));
    thread::sleep(Duration::from_millis(100));
    assert!(matches!(
        workspace_action(&control, "picker-invalid-visible", Action::Inspect),
        Response::Snapshot(snapshot)
            if project_popup(&snapshot).is_some()
                && snapshot.tabs[1].directory == selected.as_os_str().as_bytes()
    ));
    wait_for_picker_close(&control, &selected);

    fs::remove_file(&release).unwrap();
    fs::remove_file(&picker_child_pid).unwrap();
    fs::write(&selection, initial.as_os_str().as_bytes()).unwrap();
    assert!(matches!(
        invoke_project(&control, "picker-client-loss"),
        Response::Snapshot(snapshot) if project_popup(&snapshot).is_some()
    ));
    let lost_client_picker = picker_endpoint(&control);
    wait_for(&picker_child_pid);
    let picker_child = fs::read_to_string(&picker_child_pid)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let pid = fs::read_to_string(&venus_pid)
        .unwrap()
        .parse::<i32>()
        .unwrap();
    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
    wait_for_picker_close(&control, &selected);
    assert!(!lost_client_picker.exists());
    let process = Path::new("/proc").join(picker_child.to_string());
    let deadline = Instant::now() + Duration::from_secs(5);
    while process.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if process.exists() {
        // Release the leaked probe before reporting the regression.
        fs::write(&release, "").unwrap();
        wait_for_process_removal(picker_child, "released picker child did not exit");
        panic!("mock terminal Stop left its picker descendant running");
    }

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
                && snapshot.tabs[0].panes().count() == 1
                && snapshot.tabs[0].panes().next().unwrap().session == "session-2"
                && snapshot.tabs[1].panes().next().unwrap().session == "session-3"
    ));
    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["tab", "close", "t1", "--json"],
    );
    assert!(closed.status.success(), "{}", stdout(&closed));
    assert!(stdout(&closed).contains("\"active_tab\":\"t4\""));

    fs::write(&selection, "cancel").unwrap();
    let before_cancel = match workspace_action(&control, "home-tab", Action::CreateTab) {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("home tab failed: {other:?}"),
    };
    assert_eq!(before_cancel.active_tab, "t5");
    assert_eq!(before_cancel.tabs[1].directory, home.as_os_str().as_bytes());
    fs::write(&release, "").unwrap();
    let cancelled = wait_for_picker_close(&control, &home);
    assert_eq!(cancelled.active_tab, "t5");
    assert_eq!(cancelled.tabs.len(), 2);
    assert_eq!(
        stable_tabs(&cancelled.tabs)[0],
        stable_tabs(&before_cancel.tabs)[0]
    );
    assert!(!cancelled.tabs[1].pending);
    assert_eq!(cancelled.tabs[1].panes().count(), 1);
    assert!(
        fs::read_to_string(&orbit_log)
            .unwrap()
            .contains(&format!("session-4 {}", home.display()))
    );
    assert!(
        matches!(workspace_action(&control, "close-home-tab", Action::CloseTab { tab: "t5".into() }), Response::Snapshot(snapshot) if snapshot.tabs.len() == 1)
    );
    let supervisor_log = fs::read_dir(std::env::temp_dir().join("eon-test-state/eon/logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.to_string_lossy()
                .ends_with(&format!("-{}.log", supervisor.child.id()))
        })
        .unwrap();
    let supervisor_log = fs::read_to_string(supervisor_log).unwrap();
    assert!(supervisor_log.contains("tab t1 close failed after"));
    assert!(supervisor_log.contains("injected stop failure"));
    assert!(supervisor_log.contains("closed tab t5 in"));

    fs::remove_file(&release).unwrap();
    assert!(
        matches!(workspace_action(&control, "vanishing-home", Action::CreateTab), Response::Snapshot(snapshot) if snapshot.active_tab == "t6")
    );
    fs::remove_dir(&home).unwrap();
    fs::write(&release, "").unwrap();
    let failed = wait_for_picker_close(&control, &selected);
    assert_eq!(
        stable_tabs(&failed.tabs)[0],
        stable_tabs(&before_cancel.tabs)[0]
    );
    assert_eq!(failed.tabs.len(), 1);
    assert_eq!(failed.active_tab, "t4");
    assert!(supervisor.child.try_wait().unwrap().is_none());
    fs::create_dir(&home).unwrap();
    fs::remove_file(&release).unwrap();

    assert!(matches!(
        invoke_project(&control, "picker-before-stop"),
        Response::Snapshot(snapshot) if project_popup(&snapshot).is_some()
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
fn failed_directory_picker_stop_on_detach_keeps_the_supervisor_and_picker() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
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
            if project_popup(&snapshot).is_some() && snapshot.tabs[0].columns.is_empty()
    ));
    let picker = live_identity(&picker_endpoint(&control));
    fs::write(&fail_stop_once, "directory-picker-1").unwrap();
    let venus = fs::read_to_string(&venus_pid).unwrap().parse().unwrap();
    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(unsafe { libc::kill(venus, libc::SIGTERM) }, 0);

    // The picker's stop fails on detachment: it stays in its pending tab and
    // the supervisor keeps running (EON-C18).
    let deadline = Instant::now() + Duration::from_secs(5);
    while fail_stop_once.exists() {
        assert!(Instant::now() < deadline, "the picker stop was never sent");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        workspace_action(&control, "kept", Action::Inspect),
        Response::Snapshot(snapshot)
            if project_popup(&snapshot).is_some() && snapshot.tabs[0].pending
    ));
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(
        Path::new("/proc")
            .join(picker.process_id.to_string())
            .exists()
    );

    // Stopping the generation still stops the kept picker.
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["stop", generation_id, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut supervisor.child);
    wait_for_process_removal(picker.process_id, "the kept picker did not stop");
    assert!(!generation.exists());
    fs::write(&stop, "").unwrap();
    fs::remove_dir_all(root).unwrap();
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

// Lifecycle assertions exclude labels, which change independently as metadata arrives.
fn stable_tabs(tabs: &[Tab]) -> Vec<Tab> {
    let mut tabs = tabs.to_vec();
    for tab in &mut tabs {
        for pane in tab.columns.iter_mut().flat_map(|column| &mut column.panes) {
            pane.label.clear();
        }
    }
    tabs
}

fn stable_workspace_json(output: &Output) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(stdout(output)).unwrap();
    for tab in value["tabs"].as_array_mut().unwrap() {
        for pane in tab["panes"].as_array_mut().unwrap() {
            pane.as_object_mut().unwrap().remove("label");
        }
    }
    value
}

#[test]
fn stop_accounts_for_late_ready_picker_and_retains_partial_cleanup_owner() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let fail = root.join("fail");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let venus_pid = root.join("venus.pid");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    executable(&bin.join("eon-directory-picker"), "#!/bin/sh\ncat\n");
    executable(&bin.join("eon-nu"), "#!/bin/sh\ncat\n");
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\nexec cat >/dev/null\n",
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let mut supervisor = TestProcess {
        child: eon_command(binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_VENUS_PID", &venus_pid)
            .env("EON_SESSION_BIN", &bin)
            .env("EON_TEST_STOP", &stop)
            .env("EON_TEST_PICKER_READY_DELAY_MS", "5100")
            .env("EON_TEST_FAIL_STOP_ONCE", &fail)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop,
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !generation.join("orbit.sock").exists() {
        assert!(
            Instant::now() < deadline,
            "late Ready fallback did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let picker = live_identity(&generation.join("k1.sock"));
    let first = live_identity(&generation.join("orbit.sock"));
    assert!(
        invoke(binary, &runtime, &config, &["pane", "create", "--json"])
            .status
            .success()
    );
    let second = live_identity(&generation.join("session-2.sock"));
    fs::write(&fail, "session-2").unwrap();
    let id = generation.file_name().unwrap().to_str().unwrap();
    let failed = invoke(binary, &runtime, &config, &["stop", id, "--json"]);
    assert_eq!(failed.status.code(), Some(1), "{}", stdout(&failed));
    assert!(stdout(&failed).contains("injected stop failure"));
    assert!(
        supervisor.child.try_wait().unwrap().is_none(),
        "partial Stop discarded its owner"
    );
    wait_for_process_removal(picker.process_id, "late Ready picker survived Stop");
    wait_for_process_removal(first.process_id, "completed terminal survived Stop");
    let venus_pid: u32 = fs::read_to_string(venus_pid).unwrap().parse().unwrap();
    // SAFETY: this is the unreaped presentation child recorded by this test's supervisor.
    assert_eq!(unsafe { libc::kill(venus_pid as i32, libc::SIGTERM) }, 0);
    wait_for_process_removal(venus_pid, "partial Stop retained an ended Venus child");
    let inventory = invoke(binary, &runtime, &config, &["generations", "--json"]);
    assert!(inventory.status.success(), "{}", stdout(&inventory));
    assert!(
        stdout(&inventory).contains("\"sessions\":[\"session-2\"]"),
        "{}",
        stdout(&inventory)
    );
    assert!(matches!(
        workspace_action(&control, "blocked", Action::CreatePane { focus: true }),
        Response::Failure(_)
    ));
    let retried = invoke(binary, &runtime, &config, &["stop", id, "--json"]);
    assert!(retried.status.success(), "{}", stdout(&retried));
    wait_for_successful_exit(&mut supervisor.child);
    wait_for_process_removal(second.process_id, "retried terminal survived Stop");
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stop_result_covers_queued_operation_and_managed_cleanup() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let delay = root.join("delay");
    fs::write(&delay, "3500").unwrap();
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let mut supervisor = TestProcess {
        child: eon_command(binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .env("EON_TEST_DELAY_SESSION_2_MS", "3500")
            .env("EON_TEST_STOP_DELAY", &delay)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop,
    };
    let generation = generation_runtime(&runtime);
    wait_for_connection(&generation.join("eon.sock"));
    let id = generation.file_name().unwrap().to_str().unwrap();
    let mut stopping = eon_command(binary)
        .args(["stop", id])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut prompt = Vec::new();
    while !prompt.ends_with(b"[y/N] ") {
        let mut byte = [0];
        stopping
            .stderr
            .as_mut()
            .unwrap()
            .read_exact(&mut byte)
            .unwrap();
        prompt.push(byte[0]);
    }
    let mut creating = eon_command(binary)
        .args(["pane", "create", "--json"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !generation.join("session-2.sock.record").exists() {
        assert!(Instant::now() < deadline, "queued pane did not start");
        thread::sleep(Duration::from_millis(10));
    }
    stopping.stdin.take().unwrap().write_all(b"y\n").unwrap();
    assert!(creating.wait().unwrap().success());
    let result = stopping.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(stdout(&result).contains("session-1, session-2"));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
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
fn failure_kinds_exit_with_distinct_statuses_with_and_without_json() {
    let root = temporary_directory();
    let run = |arguments: &[&str]| {
        invoke(
            Path::new(env!("CARGO_BIN_EXE_eon")),
            &root.join("runtime"),
            &root.join("config"),
            arguments,
        )
    };
    let unknown_generation = "g1-00000000000000000000000000000000";
    for (arguments, status, code) in [
        (&["tab", "close"][..], 2, None),
        (&["stop", unknown_generation], 3, Some("unknown-generation")),
        (&["workspace"], 4, Some("missing-supervisor")),
    ] {
        let human = run(arguments);
        assert_eq!(human.status.code(), Some(status), "{arguments:?}");
        assert!(human.stdout.is_empty(), "{arguments:?}");
        let json = run(&[arguments, &["--json"]].concat());
        assert_eq!(json.status.code(), Some(status), "{arguments:?}");
        let human_error = String::from_utf8_lossy(&human.stderr);
        match code {
            Some(code) => {
                assert!(human_error.starts_with(&format!("error {code}: ")));
                assert!(stdout(&json).contains(&format!("\"code\":\"{code}\"")));
                assert!(json.stderr.is_empty());
            }
            None => {
                assert!(human_error.contains("usage: eon "));
                assert_eq!(json.stderr, human.stderr);
            }
        }
    }
    // No live owner for an opaque caller: all workspace commands refuse without fallback.
    for (arguments, status, code) in [
        (&["pane", "create", "--json"][..], 3, "unknown-caller"),
        (&["workspace", "--json"], 3, "unknown-caller"),
    ] {
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(arguments)
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"))
            .env("EON_TERMINAL", "session-1.other-build")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{arguments:?}");
        let output = stdout(&output);
        assert!(output.contains(&format!("\"code\":\"{code}\"")), "{output}");
        assert!(!output.contains("run `eon` first"), "{output}");
    }
    // A control call creates no runtime or generation directory for a build
    // that isn't running, as after an upgrade.
    assert!(!root.join("runtime").exists());
    let generations = root.join("runtime").join("generations");
    fs::create_dir_all(&generations).unwrap();
    for directory in [generations.parent().unwrap(), &generations] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert_eq!(run(&["workspace", "--json"]).status.code(), Some(4));
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 0);
    // An existing directory that is not private still refuses the call.
    fs::set_permissions(&generations, fs::Permissions::from_mode(0o755)).unwrap();
    let unsafe_directory = run(&["workspace", "--json"]);
    assert_eq!(unsafe_directory.status.code(), Some(4));
    assert!(stdout(&unsafe_directory).contains("owned private directory"));
    fs::set_permissions(&generations, fs::Permissions::from_mode(0o700)).unwrap();
    // An unknown window is missing whether or not the windows directory exists.
    let windows = root.join("runtime").join("w");
    for windows_exist in [false, true] {
        if windows_exist {
            fs::create_dir_all(&windows).unwrap();
            for directory in [windows.parent().unwrap(), &windows] {
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        for (arguments, status) in [
            (&["window", "attach", "0000abcd"][..], 3),
            (&["window", "stop", "0000abcd", "--json"], 3),
            (&["window", "attach", "bad/id"], 2),
        ] {
            let output = run(arguments);
            assert_eq!(
                output.status.code(),
                Some(status),
                "{windows_exist} {arguments:?}"
            );
            assert!(output.stdout.is_empty(), "{windows_exist} {arguments:?}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_focus_is_rejected_before_supervisor_connection() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));

    let missing = invoke(binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(missing.status.code(), Some(4));
    let too_long = "x".repeat(129);
    let assert_invalid = |id| {
        let output = invoke(binary, &runtime, &config, &["focus", id, "--json"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(stdout(&output).contains("\"code\":\"malformed-action\""));
        assert!(!stdout(&output).contains("run `eon` first"));
        assert!(output.stderr.is_empty());
    };
    // The action is validated before the runtime directory is known missing.
    for id in ["", too_long.as_str()] {
        assert_invalid(id);
    }

    let listed = invoke(binary, &runtime, &config, &["generations", "--json"]);
    let listed: serde_json::Value = serde_json::from_str(stdout(&listed)).unwrap();
    let current = listed["generations"][0]["id"].as_str().unwrap();
    let generation = runtime.join("generations").join(current);
    fs::create_dir_all(&generation).unwrap();
    for directory in [&runtime, &runtime.join("generations"), &generation] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let control = generation.join("eon.sock");
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
    let venus_log = root.join("venus.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[terminal]\ncursor_trail_color = 'preset:volt'\ncursor_trail_duration = 2.5\n",
    )
    .unwrap();
    managed_orbit_executable(&orbit);
    let eon = Path::new(env!("CARGO_BIN_EXE_eon"));
    let eonbase = root.join("eonbase");
    symlink(eon, &eonbase).unwrap();
    for response in ["exit 1", "printf wrong-v1", "exec cat >/dev/null"] {
        executable(
            &venus,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$EON_TEST_VENUS_LOG\"\n{response}\n"),
        );
        // A silent Venus uses the whole admission deadline, so one product covers it.
        let silent = response.starts_with("exec");
        for (binary, args) in [
            (eon, vec!["run", "--", "/bin/false"]),
            (eonbase.as_path(), vec!["--", "/bin/false"]),
        ]
        .into_iter()
        .take(if silent { 1 } else { 2 })
        {
            let started = Instant::now();
            let output = eon_command(binary)
                .args(args)
                .env("EON_RUNTIME_DIR", root.join("runtime"))
                .env("EON_CONFIG_HOME", &config)
                .env("EON_ORBIT", &orbit)
                .env("EON_VENUS", &venus)
                .env("EON_TEST_ORBIT_LOG", &orbit_log)
                .env("EON_TEST_VENUS_LOG", &venus_log)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                started.elapsed() < Duration::from_secs(if silent { 32 } else { 4 }),
                "startup rejection did not finish within its bound"
            );
            assert!(
                !orbit_log.exists(),
                "native rejection started an Orbit Session"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("cannot admit Eon Desktop presentation"),
                "{stderr}"
            );
            assert!(!silent || stderr.contains("Venus was not ready within 30s"));
            assert!(
                fs::read_to_string(&venus_log)
                    .unwrap()
                    .contains("--cursor-trail-duration-v1\n2.5\n")
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_presentation_configuration_precedes_supervisor_generation_and_children() {
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
        ("[bar]\nshow_version = false\n", "bar.widgets"),
        ("[bar]\nwidgets = ['gpu']\n", "unknown widget"),
        ("[bar]\nwidgets = ['cpu', 'cpu']\n", "repeats"),
        (
            "[terminal]\nbackground_opacity = 1.01\n",
            "background_opacity",
        ),
        ("[terminal]\nbackground_blur = \"yes\"\n", "background_blur"),
        (
            "[terminal]\ncursor_trail_duration = nan\n",
            "cursor_trail_duration",
        ),
        (
            "[terminal]\ncursor_trail_duration = inf\n",
            "cursor_trail_duration",
        ),
        (
            "[terminal]\ncursor_trail_duration = 0.24\n",
            "cursor_trail_duration",
        ),
        (
            "[terminal]\ncursor_trail_duration = 4.01\n",
            "cursor_trail_duration",
        ),
        (
            "[terminal]\ncursor_trail_duration = '2'\n",
            "cursor_trail_duration",
        ),
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
    configure_mock_desktop(&config);
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
            "--no-decorations\n--application-id\neon\n--background-opacity\n0.8\n--padding-color\nextend\n--background-blur\n--cursor-trail-duration-v1\n1.5\n--eon-version\n{}\n--bar-items\ncodex_quota,claude_quota,cpu,ram,clock,version\n--pane-frames\ntrue\n--warning-close-last-tab\ntrue\n--workspace\n{}\n",
            env!("CARGO_PKG_VERSION"),
            control.display(),
        )
    );
    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn eonbase_reopens_without_workspace_or_a_second_session() {
    let root = temporary_directory();
    let runtime = root.join("eonbase");
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
        r#"#!/bin/sh
case "$*" in
  *EonMissingProofFont*) exit 1 ;;
esac
if test ! -e "$EON_TEST_VENUS_LOG"; then
  test ! -e "$EON_TEST_ORBIT_LOG" || exit 97
  sleep 6
fi
printf ready-v1
printf '%s|%s|%s\n' "$$" "$EON_VENUS_PRESENTATION_CONTROL" "$*" >> "$EON_TEST_VENUS_LOG"
dd bs=8 count=1 status=none >> "$EON_TEST_PRESENTATION_LOG"
cat >/dev/null
"#,
    );
    executable(
        &command,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_CHILD_PID\"\nwhile test ! -e \"$EON_TEST_CHILD_EXIT\"; do sleep 0.01; done\n",
    );
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_opacity = 0.88\nbackground_blur = false\ncursor_trail_duration = 2.5\nfont_size = 20\ncolumns = 100\nrows = 30\n",
    )
    .unwrap();

    let eon = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let binary = root.join("bin/eonbase");
    fs::create_dir(root.join("bin")).unwrap();
    symlink(&eon, &binary).unwrap();
    let eonbase = |configuration: &Path| {
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
    let invalid = eonbase(&config)
        .args(["--application-id", "bad/id", "--"])
        .arg(&command)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid Eon application identity"));
    assert!(!runtime.join("generations").exists());

    let child = eonbase(&config)
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
    // The first Venus admits after session start's five seconds; its terminal still starts.
    thread::sleep(Duration::from_secs(6));
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
        "[terminal]\nbackground_opacity = 0.0\ncursor_trail_duration = 3.0\nfont_size = 24\nline_height = 1.5\nrows = 24\n",
    )
    .unwrap();
    let mut repeated = eonbase(&command)
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
                "Run `eonbase attach {generation_id}` to reconnect"
            ))
    );

    fs::write(
        config.join("config.toml"),
        "[terminal]\nbackground_blur = \"yes\"\n",
    )
    .unwrap();
    let rejected = eonbase(&config)
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
    let rejected = eonbase(&config)
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
        "[terminal]\nbackground_opacity = 0.0\ncursor_trail_duration = 3.0\nfont_size = 24\nline_height = 1.5\nrows = 24\nfuture_border_width = 2\n",
    )
    .unwrap();
    let reopened = eonbase(&config)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert!(reopened.status.success());
    assert!(supervisor.child.try_wait().unwrap().is_none());
    let diagnostic = fs::read_dir(std::env::temp_dir().join("eon-test-state/eon/logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.to_string_lossy()
                .ends_with(&format!("-{}.log", supervisor.child.id()))
        })
        .unwrap();
    let warning = format!(
        "eon: ignoring unknown setting terminal.future_border_width in {}",
        config.join("config.toml").display(),
    );
    let stderr = fs::read_to_string(&supervisor_log).unwrap();
    let warnings = stderr.lines().filter(|line| *line == warning).count();
    assert!(warnings > 0);
    let diagnostic = fs::read_to_string(diagnostic).unwrap();
    assert_eq!(
        diagnostic
            .lines()
            .filter(|line| line.ends_with(&warning))
            .count(),
        warnings,
        "every supervisor warning must reach its diagnostic log",
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&venus_log).unwrap().lines().count() != 2 {
        assert!(Instant::now() < deadline, "eonbase did not reopen");
        thread::sleep(Duration::from_millis(10));
    }
    let refocused = eonbase(&config)
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
            "reopened eonbase was not presented"
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
    assert!(venus_log.contains("--cursor-trail-duration-v1 2.5"));
    assert!(venus_log.contains("--cursor-trail-duration-v1 3"));
    assert!(venus_log.contains("--background-opacity 0 --padding-color extend --background-blur"));
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
    assert_eq!(workspace.status.code(), Some(1));
    assert!(stdout(&workspace).contains("\"code\":\"workspace-unavailable\""));
    let wrong_mode = invoke(&binary, &runtime, &config, &[]);
    assert!(!wrong_mode.status.success());
    assert!(String::from_utf8_lossy(&wrong_mode.stderr).contains("usage: eonbase"));
    let removed = invoke(
        &eon,
        &root.join("removed-runtime"),
        &config,
        &["terminal", "--", "/bin/false"],
    );
    assert_eq!(removed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed.stderr).contains("usage: eon "));
    let unknown = eon_command(&binary)
        .args(["stop", "g1-00000000000000000000000000000000", "--json"])
        .env_remove("EON_RUNTIME_DIR")
        .env("XDG_RUNTIME_DIR", &root)
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(3));
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
fn eonbase_replaces_exact_residue_after_its_supervisor_and_session_die() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let first_stop = root.join("first-stop");
    let second_stop = root.join("second-stop");
    let orbit_log = root.join("orbit.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");

    let eon = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let binary = root.join("eonbase");
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
fn pane_rename_cli_sets_clears_and_reports_the_runtime_label() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let control = generation_runtime(&runtime).join("eon.sock");
    wait_for(&control);
    let read = |arguments: &[&str]| {
        let output = invoke(&binary, &runtime, &config, arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let before = read(&["workspace", "--json"]);
    let named = read(&["pane", "rename", "p1", "  Review pane labels  ", "--json"]);
    let pane = &named["tabs"][0]["panes"][0];
    assert_eq!(pane["pinned_name"], "Review pane labels");
    assert!(
        pane["label"]
            .as_str()
            .unwrap()
            .starts_with("p1  Review pane labels")
    );
    assert_eq!(before["active_tab"], named["active_tab"]);
    for key in ["selected_pane", "selected_popup"] {
        assert_eq!(before["tabs"][0][key], named["tabs"][0][key]);
    }
    assert_eq!(
        read(&["workspace", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        "Review pane labels"
    );
    assert_eq!(
        read(&["pane", "rename", "p1", "--clear", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        serde_json::Value::Null
    );
    read(&["pane", "rename", "p1", "Again", "--json"]);
    assert_eq!(
        read(&["pane", "rename", "p1", "  ", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        serde_json::Value::Null
    );
    let literal = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "rename", "p1", "--", "--clear"],
    );
    assert!(literal.status.success());
    assert_eq!(
        read(&["workspace", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        "--clear"
    );
    let missing = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "rename", "p999", "x", "--json"],
    );
    assert_eq!(missing.status.code(), Some(3));
    assert!(stdout(&missing).contains("unknown-pane"));
    let malformed = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "rename", "p1", "bad\u{202e}name", "--json"],
    );
    assert_eq!(malformed.status.code(), Some(2));
    assert_eq!(
        read(&["workspace", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        "--clear"
    );
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    let commands = root.join("eon-commands");
    symlink(&binary, &commands).unwrap();
    let environment = fs::read(format!(
        "/proc/{}/environ",
        live_identity(&generation_runtime(&runtime).join("orbit.sock")).process_id
    ))
    .unwrap();
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .unwrap();
    for (menu, expected) in [
        ("Rename pane", Some("From Commands")),
        ("Clear pane name", None),
    ] {
        executable(
            &tools.join("fzf"),
            format!(
                r#"#!/bin/sh
case "$*" in
  *--bind=enter:print-query*) printf 'From Commands\0';;
  *) while IFS="$(printf '\t')" read -r index rest; do
       case "$rest" in '{menu}'*) printf '%s\tchosen\n' "$index";; esac
     done;;
esac
"#
            ),
        );
        let mut command = eon_command(&commands);
        let original = command
            .get_envs()
            .find_map(|(key, value)| (key == "PATH").then_some(value).flatten())
            .unwrap()
            .to_owned();
        let path = std::iter::once(tools.clone()).chain(std::env::split_paths(&original));
        let output = command
            .env("PATH", std::env::join_paths(path).unwrap())
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_TAB", "t1")
            .env("EON_TERMINAL", OsStr::from_bytes(caller))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            read(&["workspace", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
            expected.map_or(serde_json::Value::Null, |value| value.into())
        );
    }
    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_requests_project_callers_and_close_only_the_target_workspace() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let fail = root.join("fail-stop-once");
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
        .env("EON_TEST_FAIL_STOP_ONCE", &fail)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess { child, stop };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    let caller = |endpoint: &Path| {
        let environment = fs::read(format!(
            "/proc/{}/environ",
            live_identity(endpoint).process_id
        ))
        .unwrap();
        environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
            .map(|token| String::from_utf8(token.to_vec()).unwrap())
            .unwrap()
    };
    let first = caller(&generation.join("orbit.sock"));
    let snapshot = |response| match response {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("expected a workspace: {other:?}"),
    };
    let original = snapshot(workspace_action(
        &control,
        "second-pane",
        Action::CreatePane { focus: true },
    ));
    let second = caller(&generation.join("session-2.sock"));
    let created = snapshot(protocol_action_from(
        &control,
        "new",
        Some(&first),
        ProtocolAction::Workspace(Action::NewWorkspace {
            name: Some("other".into()),
            focus: true,
        }),
    ));
    assert_eq!(
        (
            created.workspace.as_str(),
            created.active_workspace.as_str()
        ),
        ("w1", "w2")
    );
    assert_eq!(stable_tabs(&created.tabs), stable_tabs(&original.tabs));
    assert_eq!(created.workspaces.len(), 2);
    let shown = snapshot(workspace_action(&control, "shown", Action::Inspect));
    assert_eq!(shown.workspace, "w2");
    let foreign_tab = protocol_action_from(
        &control,
        "foreign-tab",
        Some(&first),
        ProtocolAction::Workspace(Action::SetTabDirectory {
            tab: shown.active_tab.clone(),
            directory: root.as_os_str().as_bytes().to_vec(),
        }),
    );
    assert!(matches!(foreign_tab, Response::Failure(error) if error.code == "unknown-tab"));
    let renamed = snapshot(protocol_action_from(
        &control,
        "rename-caller",
        Some(&first),
        ProtocolAction::Workspace(Action::RenameWorkspace {
            workspace: None,
            name: "original".into(),
        }),
    ));
    assert_eq!(renamed.workspaces[0].name.as_deref(), Some("original"));
    let switched = snapshot(workspace_action(
        &control,
        "switch",
        Action::SwitchWorkspace {
            workspace: "original".into(),
        },
    ));
    assert_eq!(switched.workspace, "w1");
    assert_eq!(
        stable_tabs(
            &snapshot(workspace_action(
                &control,
                "same",
                Action::SwitchWorkspace {
                    workspace: "w1".into()
                }
            ))
            .tabs
        ),
        stable_tabs(&switched.tabs)
    );
    snapshot(workspace_action(
        &control,
        "back",
        Action::SwitchWorkspace {
            workspace: "w2".into(),
        },
    ));
    let inspected = snapshot(protocol_action_from(
        &control,
        "inspect-caller",
        Some(&first),
        ProtocolAction::Workspace(Action::Inspect),
    ));
    assert_eq!(inspected.workspace, "w1");
    assert_eq!(stable_tabs(&inspected.tabs), stable_tabs(&original.tabs));

    // The first confirmed stop is pruned; the failing terminal and workspace remain.
    fs::write(&fail, "session-2").unwrap();
    assert!(
        matches!(workspace_action(&control, "close-fails", Action::CloseWorkspace { workspace: "w1".into() }), Response::Failure(error) if error.code == "workspace-close-failed" && error.detail.contains("injected stop failure"))
    );
    let remaining = snapshot(protocol_action_from(
        &control,
        "remaining",
        Some(&second),
        ProtocolAction::Workspace(Action::Inspect),
    ));
    assert_eq!(remaining.workspace, "w1");
    assert_eq!(
        remaining.tabs[0]
            .panes()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["p2"]
    );
    assert_eq!(remaining.active_workspace, "w2");
    assert!(
        matches!(workspace_action(&control, "close-fails", Action::CloseWorkspace { workspace: "w1".into() }), Response::Failure(error) if error.code == "duplicate-request")
    );
    let closed = snapshot(protocol_action_from(
        &control,
        "retry-close",
        Some(&second),
        ProtocolAction::Workspace(Action::CloseWorkspace {
            workspace: "w1".into(),
        }),
    ));
    assert_eq!(
        closed.workspace, "w2",
        "a removed caller workspace falls back to the shown survivor"
    );
    assert_eq!(
        stable_tabs(&closed.tabs),
        stable_tabs(&shown.tabs),
        "closing a hidden workspace leaves shown terminals alone"
    );
    assert_eq!(closed.workspaces.len(), 1);
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(
        matches!(protocol_action_from(&control, "stale", Some(&second), ProtocolAction::Workspace(Action::Inspect)), Response::Failure(error) if error.code == "unknown-caller")
    );
    assert!(matches!(
        workspace_action(
            &control,
            "last",
            Action::CloseWorkspace {
                workspace: "w2".into()
            }
        ),
        Response::Ended(_)
    ));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_target_failures_are_explicit_and_bounded() {
    let root = temporary_directory();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let config = root.join("config");
    let runtime = root.join("runtime");
    let id = "g1-00000000000000000000000000000001";
    for (arguments, status) in [
        (vec!["workspace", "--generation", id, "--json"], 3),
        (vec!["workspace", "--window", "0000abcd", "--json"], 3),
        (vec!["workspace", "--generation", "bad"], 2),
        (vec!["workspace", "--window", "ABCDEFGH"], 2),
        (vec!["workspace", "--window"], 2),
        (vec!["versions", "--generation", id], 2),
    ] {
        assert_eq!(
            invoke(binary, &runtime, &config, &arguments).status.code(),
            Some(status),
            "{arguments:?}"
        );
    }
    assert!(!runtime.exists());
    assert!(!config.exists());

    // An older EONW cannot validate the opaque caller; never guess another owner.
    let listener = generation_listener(&runtime, id);
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            let mut header = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: "old supervisor".into(),
            }))
            .unwrap();
            header[4..6].copy_from_slice(&(VERSION - 1).to_le_bytes());
            let _ = stream.write_all(&header);
        }
    });
    for caller in [None, Some("session-1.unknown")] {
        let mut command = eon_command(binary);
        if let Some(caller) = caller {
            command.env("EON_TERMINAL", caller);
        }
        let output = command
            .args(["workspace", "--generation", id, "--json"])
            .env("EON_RUNTIME_DIR", &runtime)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{}", stdout(&output));
        assert!(stdout(&output).contains(id));
        assert!(stdout(&output).contains("EONW"));
        assert!(stdout(&output).contains("the terminal's own `eon`"));
    }
    server.join().unwrap();
    fs::remove_dir_all(&runtime).unwrap();

    fs::create_dir_all(runtime.join("generations")).unwrap();
    for path in [&runtime, &runtime.join("generations")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    overfill_generation_directory(&runtime);
    fs::create_dir(runtime.join("generations").join(format!("g1-{:032x}", 256))).unwrap();
    let output = eon_command(binary)
        .args(["workspace", "--json"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_TERMINAL", "session-1.unknown")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4), "{}", stdout(&output));
    assert!(stdout(&output).contains("256-entry inspection limit"));
    assert_eq!(
        fs::read_dir(runtime.join("generations")).unwrap().count(),
        257
    );
    assert!(!config.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_control_follows_the_exact_caller_across_generations() {
    let root = temporary_directory();
    // The package harness uses /dev/shm; leave room for a window's management sockets.
    let runtime = root.join("r");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    // A second immutable assembly exercises the real supervisor without a second build.
    let previous_binary = root.join("previous-eon");
    let mut bytes = fs::read(binary).unwrap();
    let digest = env!("EON_WORKSPACE_DIGEST").as_bytes();
    let offsets = bytes
        .windows(digest.len())
        .enumerate()
        .filter_map(|(offset, value)| (value == digest).then_some(offset))
        .collect::<Vec<_>>();
    assert!(!offsets.is_empty());
    for offset in offsets {
        bytes[offset] = if bytes[offset] == b'0' { b'1' } else { b'0' };
    }
    executable(&previous_binary, bytes);
    let launch = |binary: &Path, name: &str, namespace: &Path| {
        let stop = root.join(name);
        let child = eon_command(binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", namespace)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        TestProcess { child, stop }
    };
    let mut previous = launch(&previous_binary, "previous-stop", &runtime);
    let previous_runtime = generation_runtime(&runtime);
    let previous_control = previous_runtime.join("eon.sock");
    wait_for(&previous_control);
    let endpoint = previous_runtime.join("orbit.sock");
    wait_for(&endpoint);
    let environment = fs::read(format!(
        "/proc/{}/environ",
        live_identity(&endpoint).process_id
    ))
    .unwrap();
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .unwrap();
    let mut current = launch(binary, "current-stop", &runtime);
    let current_id = stdout(&invoke(binary, &runtime, &config, &["versions"]))
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();
    let current_control = runtime
        .join("generations")
        .join(&current_id)
        .join("eon.sock");
    wait_for(&current_control);
    assert_ne!(current_control, previous_control);
    let from = |arguments: &[&str]| {
        eon_command(binary)
            .args(arguments)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_TERMINAL", OsStr::from_bytes(caller))
            .output()
            .unwrap()
    };
    assert!(
        matches!(workspace_action(&previous_control, "hide-caller", Action::NewWorkspace { name: Some("shown".into()), focus: true }), Response::Snapshot(snapshot) if snapshot.workspace == "w2")
    );
    let inspected = from(&["workspace", "--json"]);
    assert!(inspected.status.success(), "{}", stdout(&inspected));
    let described: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    let created = from(&["pane", "create", "--no-focus", "--json"]);
    assert!(
        created.status.success(),
        "inspection={described}; mutation={}",
        stdout(&created)
    );
    assert_eq!(
        described["tabs"][0]["panes"][0]["endpoint"],
        endpoint.to_str().unwrap()
    );
    let previous_id = previous_runtime.file_name().unwrap().to_str().unwrap();
    assert_eq!(described["generation"], previous_id);
    assert_eq!(described["window"], serde_json::Value::Null);
    assert_eq!(described["workspace"], "w1");
    let older = generation_listener(&runtime, "g1-00000000000000000000000000000002");
    let older = thread::spawn(move || {
        let (mut stream, _) = older.accept().unwrap();
        read_request(&mut stream);
        let mut response = encode_response(&Response::Failure(ProtocolFailure {
            code: "unsupported-version".into(),
            detail: "older unrelated build".into(),
        }))
        .unwrap();
        response[4..6].copy_from_slice(&(VERSION - 1).to_le_bytes());
        let _ = stream.write_all(&response);
    });
    let still_owned = from(&["workspace", "--json"]);
    assert!(still_owned.status.success(), "{}", stdout(&still_owned));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&still_owned.stdout).unwrap()["generation"],
        previous_id
    );
    older.join().unwrap();
    for arguments in [
        vec!["workspace", "--generation", previous_id, "--json"],
        vec!["--generation", previous_id, "workspace", "--json"],
    ] {
        let outside = invoke(binary, &runtime, &config, &arguments);
        assert!(outside.status.success(), "{}", stdout(&outside));
        let result: serde_json::Value = serde_json::from_slice(&outside.stdout).unwrap();
        assert_eq!(result["generation"], previous_id);
        assert_eq!(result["workspace"], "w2");
        let inside = from(&arguments);
        assert!(inside.status.success(), "{}", stdout(&inside));
        let result: serde_json::Value = serde_json::from_slice(&inside.stdout).unwrap();
        assert_eq!(result["workspace"], "w1");
    }
    let wrong = from(&[
        "pane",
        "create",
        "--generation",
        &current_id,
        "--no-focus",
        "--json",
    ]);
    assert_eq!(wrong.status.code(), Some(3), "{}", stdout(&wrong));
    let human = from(&["workspace"]);
    assert!(human.status.success());
    assert!(stdout(&human).starts_with(&format!(
        "generation {previous_id}\nwindow default\nworkspace w1\n"
    )));
    for token in [
        String::from_utf8(caller.to_vec()).unwrap() + "0",
        "session-1".into(),
        "not a terminal".into(),
        "".into(),
    ] {
        for arguments in [
            vec!["workspace", "--json"],
            vec!["pane", "create", "--generation", &current_id, "--json"],
        ] {
            let stale = eon_command(binary)
                .args(arguments)
                .env("EON_RUNTIME_DIR", &runtime)
                .env("EON_TERMINAL", &token)
                .output()
                .unwrap();
            assert_eq!(stale.status.code(), Some(3), "{}", stdout(&stale));
        }
    }
    for (socket, count) in [(&previous_control, 2), (&current_control, 1)] {
        let Response::Snapshot(snapshot) = protocol_action_from(
            socket,
            "verify-routing",
            (socket == &previous_control).then(|| std::str::from_utf8(caller).unwrap()),
            ProtocolAction::Workspace(Action::Inspect),
        ) else {
            panic!("cannot inspect supervisor");
        };
        assert_eq!(snapshot.tabs[0].panes().count(), count);
        assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p1"));
    }
    let Response::Snapshot(shown) =
        workspace_action(&previous_control, "shown-still", Action::Inspect)
    else {
        panic!("cannot inspect shown workspace");
    };
    assert_eq!(shown.workspace, "w2");
    assert_eq!(shown.tabs[0].panes().count(), 1);
    // Caller discovery also works when this build's supervisor has ended.
    fs::write(&current.stop, "").unwrap();
    wait_for_successful_exit(&mut current.child);
    assert!(from(&["workspace", "--json"]).status.success());
    let window_id = "0000abcd";
    let window_root = runtime.join("w").join(window_id);
    fs::create_dir(runtime.join("w")).unwrap();
    fs::set_permissions(runtime.join("w"), fs::Permissions::from_mode(0o700)).unwrap();
    let mut window = launch(&previous_binary, "window-stop", &window_root);
    let window_runtime = generation_runtime(&window_root);
    wait_for(&window_runtime.join("eon.sock"));
    let outside = invoke(
        binary,
        &runtime,
        &config,
        &["workspace", "--window", window_id, "--json"],
    );
    assert!(outside.status.success(), "{}", stdout(&outside));
    let result: serde_json::Value = serde_json::from_slice(&outside.stdout).unwrap();
    assert_eq!(
        result["generation"], previous_id,
        "window-only selects its sole previous generation"
    );
    assert_eq!(result["window"], window_id);
    let constrained = from(&["workspace", "--window", window_id, "--json"]);
    assert_eq!(constrained.status.code(), Some(3));
    let other_id = "g1-00000000000000000000000000000001";
    let other = previous_supervisor(&window_root, other_id);
    let ambiguous = invoke(
        binary,
        &runtime,
        &config,
        &["workspace", "--window", window_id, "--json"],
    );
    assert_eq!(ambiguous.status.code(), Some(1), "{}", stdout(&ambiguous));
    assert!(stdout(&ambiguous).contains("--generation"));
    let exact = invoke(
        binary,
        &runtime,
        &config,
        &[
            "workspace",
            "--window",
            window_id,
            "--generation",
            previous_id,
            "--json",
        ],
    );
    assert!(exact.status.success(), "{}", stdout(&exact));
    assert!(
        invoke(binary, &window_root, &config, &["stop", other_id, "--json"])
            .status
            .success()
    );
    other.join().unwrap();
    fs::write(&window.stop, "").unwrap();
    wait_for_successful_exit(&mut window.child);
    for process in [&mut previous, &mut current] {
        fs::write(&process.stop, "").unwrap();
        wait_for_successful_exit(&mut process.child);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commands_from_a_terminal_target_its_column_and_stale_callers_change_nothing() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    // A stand-in Helix for `eon edit`, first on the supervisor's PATH.
    let editor = root.join("editor");
    fs::create_dir(&editor).unwrap();
    executable(&editor.join("hx"), "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let mut command = eon_command(&binary);
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "PATH").then_some(value))
        .flatten()
        .unwrap()
        .to_owned();
    let path = std::iter::once(editor).chain(std::env::split_paths(&path));
    let child = command
        .env("PATH", std::env::join_paths(path).unwrap())
        .arg("run")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
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

    // The first terminal names itself to commands run inside it.
    let first = generation.join("orbit.sock");
    wait_for(&first);
    let environment = fs::read(format!(
        "/proc/{}/environ",
        live_identity(&first).process_id
    ))
    .unwrap();
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .map(|value| String::from_utf8(value.to_vec()).unwrap())
        .expect("terminal environment names its terminal");
    let from = |caller: &str, arguments: &[&str]| {
        eon_command(&binary)
            .args(arguments)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_TERMINAL", caller)
            .output()
            .unwrap()
    };
    let inspect = |id: &str| match workspace_action(&control, id, Action::Inspect) {
        Response::Snapshot(mut snapshot) => {
            snapshot.tabs = stable_tabs(&snapshot.tabs);
            snapshot
        }
        response => panic!("cannot inspect workspace: {response:?}"),
    };

    // A second column takes the selection away from the first terminal's column.
    assert!(matches!(
        protocol_action(&control, "column", ProtocolAction::CreateColumn),
        Response::Snapshot(snapshot) if snapshot.tabs[0].selected_pane.as_deref() == Some("p2")
    ));
    let created = from(&caller, &["pane", "create", "--json"]);
    assert!(
        created.status.success(),
        "stdout={} stderr={}",
        stdout(&created),
        String::from_utf8_lossy(&created.stderr)
    );
    let snapshot = inspect("after-create");
    let ids = |column: usize| {
        snapshot.tabs[0].columns[column]
            .panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!((ids(0), ids(1)), (vec!["p1", "p3"], vec!["p2"]));
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p3"));

    // A caller that names no live terminal, or nothing at all, changes nothing.
    let stale = format!("{caller}0");
    for caller in [stale.as_str(), "not a terminal", ""] {
        let refused = from(caller, &["pane", "create", "--json"]);
        assert_eq!(refused.status.code(), Some(3), "{caller}");
        assert!(stdout(&refused).contains("\"code\":\"unknown-caller\""));
    }
    assert_eq!(inspect("after-refusals"), snapshot);

    let closed = from(&caller, &["pane", "close", "p3", "--json"]);
    assert!(closed.status.success(), "{}", stdout(&closed));
    assert!(!stdout(&closed).contains("\"id\":\"p3\""));

    // `eon edit` opens a relative file from the caller's directory in Helix,
    // in a new pane beside the calling terminal.
    let edited = eon_command(&binary)
        .args(["edit", "--json", "--", "notes.txt"])
        .current_dir(&root)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_TERMINAL", &caller)
        .output()
        .unwrap();
    assert!(edited.status.success(), "{}", stdout(&edited));
    let snapshot = inspect("after-edit");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p4"));
    let pane = &snapshot.tabs[0].columns[0].panes[1];
    assert_eq!(pane.id, "p4");
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = fs::read(format!(
        "/proc/{}/cmdline",
        live_identity(endpoint).process_id
    ))
    .unwrap();
    let file = root.join("notes.txt");
    assert!(
        orbit_arguments.ends_with(&[b"hx\0", file.as_os_str().as_bytes(), b"\0"].concat()),
        "{}",
        String::from_utf8_lossy(&orbit_arguments)
    );

    // `eon pane create -- COMMAND` runs its exact argv beside the caller, and
    // is refused outside an Eon terminal.
    let command = ["pane", "create", "--json", "--", "hx", "two words"];
    let uncalled = eon_command(&binary)
        .args(command)
        .env_remove("EON_TERMINAL")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert_eq!(uncalled.status.code(), Some(1), "{}", stdout(&uncalled));
    assert!(stdout(&uncalled).contains("caller-required"));
    assert_eq!(inspect("after-uncalled"), snapshot);
    let ran = eon_command(&binary)
        .args(command)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_TERMINAL", &caller)
        .output()
        .unwrap();
    assert!(ran.status.success(), "{}", stdout(&ran));
    assert!(stdout(&ran).contains(r#""selected_pane":"p5""#));
    let snapshot = inspect("after-command");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p5"));
    let pane = &snapshot.tabs[0].columns[0].panes[2];
    assert_eq!(pane.id, "p5");
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = fs::read(format!(
        "/proc/{}/cmdline",
        live_identity(endpoint).process_id
    ))
    .unwrap();
    assert!(
        orbit_arguments.ends_with(b"hx\0two words\0"),
        "{}",
        String::from_utf8_lossy(&orbit_arguments)
    );

    // With `--no-focus`, the pane joins the caller's column and the selection
    // stays.
    let unfocused = from(
        &caller,
        &["pane", "create", "--no-focus", "--json", "--", "hx"],
    );
    assert!(unfocused.status.success(), "{}", stdout(&unfocused));
    let snapshot = inspect("after-unfocused");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p5"));
    assert_eq!(snapshot.tabs[0].columns[0].panes[3].id, "p6");

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_second_cli_receives_committed_workspace_and_controls_three_sessions() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
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
    let unchanged = invoke(&binary, &runtime, &config, &["focus", "left", "--json"]);
    assert!(unchanged.status.success());
    assert_eq!(
        stable_workspace_json(&before),
        stable_workspace_json(&unchanged)
    );
    for (id, action) in [
        ("next-attention", ProtocolAction::NextAttention),
        ("previous-attention", ProtocolAction::PreviousAttention),
    ] {
        assert!(matches!(
            protocol_action(&control, id, action),
            Response::Failure(failure)
                if failure.code == "no-attention" && failure.detail == "No agent needs attention"
        ));
    }
    let after = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(
        stable_workspace_json(&before),
        stable_workspace_json(&after)
    );
    let human = invoke(&binary, &runtime, &config, &["workspace"]);
    assert!(human.status.success());
    assert!(stdout(&human).contains("active t1\n"));
    assert_eq!(stdout(&human).matches("  pane ").count(), 3);
    assert_eq!(stdout(&human).matches(" terminal=session-").count(), 3);

    let mut incompatible = encode_request(&Request {
        id: "incompatible-1".into(),
        caller: None,
        action: ProtocolAction::Workspace(Action::Inspect),
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

    // A sideways move splits the selected stacked pane into a new column.
    let moved = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "move", "right", "--json"],
    );
    assert!(
        moved.status.success(),
        "stdout={} stderr={}",
        stdout(&moved),
        String::from_utf8_lossy(&moved.stderr)
    );
    assert!(stdout(&moved).contains("{\"id\":\"p3\",\"column\":2,\"expanded\":true,"));

    // Closing a pane stops exactly its terminal and prunes it like an exit.
    let second = live_identity(&generation.join("session-2.sock")).process_id;
    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "close", "p2", "--json"],
    );
    assert!(
        closed.status.success(),
        "stdout={} stderr={}",
        stdout(&closed),
        String::from_utf8_lossy(&closed.stderr)
    );
    assert!(!stdout(&closed).contains("\"id\":\"p2\""));
    assert_eq!(stdout(&closed).matches("\"session\":").count(), 2);
    wait_for_process_removal(second, "a closed pane's terminal kept running");
    let close_third = || {
        protocol_action(
            &control,
            "close-p3",
            ProtocolAction::ClosePane { pane: "p3".into() },
        )
    };
    assert!(matches!(
        close_third(),
        Response::Snapshot(snapshot) if snapshot.tabs[0].panes().count() == 1
    ));
    assert!(matches!(
        close_third(),
        Response::Failure(failure) if failure.code == "duplicate-request"
    ));
    // The final tab's last pane ends the window; stdin is not a terminal, so it does not ask.
    let final_pane = invoke(&binary, &runtime, &config, &["pane", "close", "p1"]);
    assert!(final_pane.status.success(), "{}", stdout(&final_pane));
    assert!(stdout(&final_pane).starts_with("ended generation "));
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
    configure_mock_desktop(&config);
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
    assert!(stdout(&snapshot).contains(&format!("\"directory\":\"{}\"", launch.to_str().unwrap())));

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
        format!(
            "[terminal]\nfont_fallbacks = []\n\n[shell]\ncommand = [\"{}\"]\n\n[anima]\nenabled = false\n",
            reporter.to_string_lossy()
        ),
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
    assert_eq!(failed.status.code(), Some(1));
    assert!(stdout(&failed).contains("\"code\":\"session-start\""));
    let after = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(
        stable_workspace_json(&before),
        stable_workspace_json(&after)
    );
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
    for unclean in [false, true] {
        let root = temporary_directory();
        let runtime = root.join("runtime");
        let config = root.join("config");
        configure_mock_desktop(&config);
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

        let peer = live_identity(&generation.join("session-2.sock"));
        if unclean {
            let identity = live_identity(&generation.join("orbit.sock"));
            // SAFETY: this exact still-owned Orbit process belongs to the test.
            assert_eq!(
                unsafe { libc::kill(identity.process_id as i32, libc::SIGKILL) },
                0
            );
        } else {
            fs::write(&exit_initial, "").unwrap();
        }
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
        assert_eq!(live_identity(&generation.join("session-2.sock")), peer);
        assert!(!artifact_path(&generation.join("orbit.sock"), ".record").exists());
        let pid: i32 = fs::read_to_string(&venus_pid).unwrap().parse().unwrap();
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

        fs::write(&stop, "").unwrap();
        assert_eq!(
            wait_for_exit(&mut supervisor.child).code(),
            Some(i32::from(unclean))
        );
        assert!(!control.exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn closing_the_final_tab_ends_the_window_and_its_terminals() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
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
        .env("EON_TEST_VENUS_PID", &venus_pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess { child, stop };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    wait_for(&venus_pid);
    let pane = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(pane.status.success(), "{}", stdout(&pane));
    let terminals = ["orbit.sock", "session-2.sock"]
        .map(|endpoint| live_identity(&generation.join(endpoint)).process_id);

    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["tab", "close", "t1", "--json"],
    );
    assert!(closed.status.success(), "{}", stdout(&closed));
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        stdout(&closed),
        format!(
            "{{\"ended\":{{\"generation\":\"{generation_id}\"}},\"generation\":\"{generation_id}\",\"window\":null}}\n"
        )
    );
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    for terminal in terminals {
        wait_for_process_removal(terminal, "a final close left an Orbit terminal running");
    }
    let pid: i32 = fs::read_to_string(&venus_pid).unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    // SAFETY: signal 0 performs existence/permission checking without sending a signal.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "a final close left Venus running"
        );
        thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_workspace_final_close_shows_the_recent_one_and_attach_keeps_it() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    // A font size makes Present hand each Venus its startup workspace.
    fs::write(
        config.join("config.toml"),
        "[terminal]\nfont_fallbacks = []\nfont_size = 20\n\n[anima]\nenabled = false\n",
    )
    .unwrap();
    let stop = root.join("stop");
    let venus_pids = root.join("venus.pids");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    // Each Venus keeps its input. Told to detach, it exits 0, as Venus does when its window
    // closes.
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$EON_TEST_VENUS_PIDS\"\nexec 3<&0\n(cat <&3 > \"$EON_TEST_VENUS_INPUT.$$\"; touch \"$EON_TEST_VENUS_INPUT.$$.eof\") &\nprintf ready-v1\nwhile test ! -e \"$EON_TEST_VENUS_INPUT.$$.detach\" && test ! -e \"$EON_TEST_VENUS_INPUT.$$.eof\"; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let eon = |arguments: &[&str]| {
        let mut command = eon_command(&binary);
        command
            .args(arguments)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .env("EON_TEST_VENUS_PIDS", &venus_pids)
            .env("EON_TEST_VENUS_INPUT", root.join("venus-input"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    let mut supervisor = TestProcess {
        child: eon(&["run"]).spawn().unwrap(),
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    let venus_pid = |count: usize| -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pids = fs::read_to_string(&venus_pids).unwrap_or_default();
            if let Some(pid) = pids.lines().nth(count - 1) {
                return pid.parse().unwrap();
            }
            assert!(Instant::now() < deadline, "Venus {count} did not start");
            thread::sleep(Duration::from_millis(10));
        }
    };
    let snapshot = |response| match response {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("expected a workspace: {other:?}"),
    };
    let ids = |snapshot: &Snapshot| {
        snapshot
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>()
    };
    let first_venus = venus_pid(1);
    for id in ["w2", "w3"] {
        let created = snapshot(workspace_action(
            &control,
            &format!("new-{id}"),
            Action::NewWorkspace {
                name: None,
                focus: true,
            },
        ));
        assert_eq!(created.active_workspace, id);
    }
    // Shown w1, w2, w3, then w1 again: closing w1 shows w3, the most recent, rather than
    // its following sibling or the first remaining workspace, both w2.
    let first = snapshot(workspace_action(
        &control,
        "back",
        Action::SwitchWorkspace {
            workspace: "w1".into(),
        },
    ));
    assert_eq!(first.tabs.len(), 1);
    let closed = snapshot(workspace_action(
        &control,
        "close-w1",
        Action::CloseTab {
            tab: first.active_tab.clone(),
        },
    ));
    assert_eq!(
        (closed.workspace.as_str(), closed.active_workspace.as_str()),
        ("w3", "w3")
    );
    assert_eq!(ids(&closed), ["w2", "w3"]);
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(Path::new("/proc").join(first_venus.to_string()).exists());

    // Detaching keeps every workspace, and attach presents the one last shown.
    fs::write(root.join(format!("venus-input.{first_venus}.detach")), "").unwrap();
    wait_for_process_removal(first_venus, "Venus did not detach");
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    assert!(eon(&["attach", generation_id]).status().unwrap().success());
    let second_venus = venus_pid(2);
    let input = root.join(format!("venus-input.{second_venus}"));
    let deadline = Instant::now() + Duration::from_secs(5);
    let presented = loop {
        let bytes = fs::read(&input).unwrap_or_default();
        if bytes.len() >= HEADER_BYTES {
            let length = declared_message_len(&bytes[..HEADER_BYTES]).unwrap();
            if bytes.len() >= length {
                break snapshot(decode_response(&bytes[..length]).unwrap());
            }
        }
        assert!(Instant::now() < deadline, "attach sent Venus no workspace");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        (
            presented.workspace.as_str(),
            presented.active_workspace.as_str()
        ),
        ("w3", "w3")
    );
    assert_eq!(ids(&presented), ["w2", "w3"]);

    // Only the last workspace's final close ends the window.
    let emptied = snapshot(workspace_action(
        &control,
        "close-w3",
        Action::CloseTab {
            tab: presented.active_tab.clone(),
        },
    ));
    assert_eq!(ids(&emptied), ["w2"]);
    assert_eq!(emptied.active_workspace, "w2");
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(matches!(
        workspace_action(
            &control,
            "close-w2",
            Action::CloseTab {
                tab: emptied.active_tab.clone(),
            },
        ),
        Response::Ended(_)
    ));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    wait_for_process_removal(second_venus, "a final close left Venus running");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn launch_overlapping_last_session_exit_starts_a_fresh_session() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
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
    configure_mock_desktop(&config);
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
        stderr.contains("ended terminal cleanup exceeded five seconds"),
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
    configure_mock_desktop(&config);
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
            if project_popup(&snapshot).is_some() && snapshot.tabs[0].columns.is_empty()
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
            if project_popup(&snapshot).is_none()
                && snapshot.tabs[0].selected_pane.as_deref() == Some("p1")
                && snapshot.tabs[0].panes().count() == 1
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
    configure_mock_desktop(&config);
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
    let decoys = (0..MAX_SESSIONS)
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

    let eonbase = root.join("eonbase");
    symlink(&binary, &eonbase).unwrap();
    assert_recovery_failure(
        &eonbase,
        &["--", "/bin/false"],
        "eonbase cannot recover more than one live terminal",
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
    assert!(recovered.contains("\"popups\":[]"));
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
    configure_mock_desktop(&config);
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
            "terminal record reports component generation wrong-generation",
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
        configure_mock_desktop(&config);
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
    configure_mock_desktop(&config);
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
    assert!(stderr.contains("invalid terminal record"), "{stderr}");
    wait_for(&orbit_log);
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
    configure_mock_desktop(&config);
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
        caller: None,
        action: ProtocolAction::Workspace(Action::Stop {
            generation: generation_id.into(),
        }),
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
    configure_mock_desktop(&config);
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
        Some(1),
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
    configure_mock_desktop(&config);
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
fn new_windows_keep_independent_supervisors_and_stop_by_window_id() {
    let root = std::env::temp_dir().join(format!(
        "ew{:x}{:x}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let runtime = root.join("r");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    let session_bin = root.join("bin");
    fs::create_dir(&session_bin).unwrap();
    executable(&session_bin.join("eon"), "#!/bin/sh\nexit 1\n");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let command = || {
        let mut command = eon_command(&binary);
        command
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_SESSION_BIN", &session_bin)
            .env("EON_TEST_STOP", &stop);
        command
    };
    let mut default = TestProcess {
        child: command()
            .arg("run")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let default_control = generation_runtime(&runtime).join("eon.sock");
    wait_for_connection(&default_control);
    let open = || {
        let output = command().args(["window", "new"]).output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            stdout(&output),
            String::from_utf8_lossy(&output.stderr)
        );
        stdout(&output).trim().to_owned()
    };
    let first = open();
    let second = open();
    assert_ne!(first, second);
    let first_control = generation_runtime(&runtime.join("w").join(&first)).join("eon.sock");
    let second_control = generation_runtime(&runtime.join("w").join(&second)).join("eon.sock");
    assert_ne!(first_control, second_control);
    wait_for_connection(&first_control);
    wait_for_connection(&second_control);
    let endpoint = second_control.parent().unwrap().join("orbit.sock");
    let environment = fs::read(format!(
        "/proc/{}/environ",
        live_identity(&endpoint).process_id
    ))
    .unwrap();
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .unwrap();
    for inherited in [false, true] {
        let mut selected = command();
        selected.env("EON_TERMINAL", OsStr::from_bytes(caller));
        if inherited {
            selected.env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(&second));
        }
        let args = if inherited {
            vec!["workspace", "--json"]
        } else {
            vec!["workspace", "--window", &second, "--json"]
        };
        let output = selected.args(args).output().unwrap();
        assert!(output.status.success(), "{}", stdout(&output));
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["window"], second);
        assert_eq!(
            result["tabs"][0]["panes"][0]["endpoint"],
            endpoint.to_str().unwrap()
        );
    }
    let constrained = command()
        .env("EON_TERMINAL", OsStr::from_bytes(caller))
        .args(["pane", "create", "--window", &first, "--no-focus", "--json"])
        .output()
        .unwrap();
    assert_eq!(constrained.status.code(), Some(3));
    let outside = invoke(
        &binary,
        &runtime,
        &config,
        &["workspace", "--window", &second, "--json"],
    );
    assert!(outside.status.success(), "{}", stdout(&outside));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&outside.stdout).unwrap()["window"],
        second
    );
    // Each generation records its own build's `eon` for later builds to run.
    let first_generation = first_control.parent().unwrap().to_owned();
    assert_eq!(
        fs::read_link(first_generation.join("eon")).unwrap(),
        session_bin.join("eon")
    );
    let listed = invoke(&binary, &runtime, &config, &["windows", "--json"]);
    assert!(listed.status.success());
    assert!(stdout(&listed).contains(&format!("\"id\":\"{first}\"")));
    assert!(stdout(&listed).contains(&format!("\"id\":\"{second}\"")));
    let human = invoke(&binary, &runtime, &config, &["windows"]);
    assert!(human.status.success());
    assert!(stdout(&human).contains(&format!("window {first}\n  current ")));
    assert!(!stdout(&human).contains("\"generations\""));

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", &first, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    assert!(stdout(&stopped).contains("\"stopped\""));
    let deadline = Instant::now() + Duration::from_secs(5);
    while first_generation.exists() {
        assert!(
            Instant::now() < deadline,
            "a stopped generation kept its directory"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        invoke(&binary, &runtime, &config, &["workspace", "--json"])
            .status
            .success()
    );
    assert!(matches!(
        workspace_action(&second_control, "still-live", Action::Inspect),
        Response::Snapshot(_)
    ));
    symlink(&config, runtime.join("w").join("aaaaaaaa")).unwrap();
    let unsafe_listing = invoke(&binary, &runtime, &config, &["windows", "--json"]);
    assert!(!unsafe_listing.status.success());
    assert!(String::from_utf8_lossy(&unsafe_listing.stderr).contains("owned private directory"));
    fs::remove_file(runtime.join("w").join("aaaaaaaa")).unwrap();

    let third = open();
    let (caller, other) = if second < third {
        (&second, &third)
    } else {
        (&third, &second)
    };
    let mut batch = command()
        .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(caller))
        .args(["window", "stop", "all"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    batch.stdin.as_mut().unwrap().write_all(b"y\ny\n").unwrap();
    let batch = batch.wait_with_output().unwrap();
    assert!(batch.status.success(), "{}", stdout(&batch));
    assert_eq!(stdout(&batch).matches("stopped generation").count(), 2);
    let prompts = String::from_utf8_lossy(&batch.stderr);
    assert!(
        prompts.find(&format!("Stop Eon window {other}:")).unwrap()
            < prompts.find(&format!("Stop Eon window {caller}:")).unwrap(),
        "{prompts}"
    );

    let (fourth, fifth) = (open(), open());
    let caller = fourth.clone().min(fifth.clone());
    let batch = command()
        .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(&caller))
        .args(["window", "stop", "all", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(batch.status.success(), "{}", stdout(&batch));
    let results: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), 5, "{results:?}");
    assert_eq!(results[4]["window"], caller);
    for result in results {
        let window = result["window"].as_str().unwrap();
        let stopped = result["results"].as_array().unwrap();
        if window == fourth || window == fifth {
            assert!(stopped[0]["stopped"]["generation"].is_string(), "{result}");
        } else {
            assert!(stopped.is_empty(), "{result}");
        }
    }
    let current = generation_runtime(&runtime);
    let current_id = current.file_name().unwrap().to_str().unwrap();
    let stopped = invoke(&binary, &runtime, &config, &["stop", current_id, "--json"]);
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

fn generation_listener(runtime: &Path, id: &str) -> UnixListener {
    let parent = runtime.join("generations");
    let generation = parent.join(id);
    fs::create_dir_all(&generation).unwrap();
    for directory in [runtime, parent.as_path(), generation.as_path()] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket = generation.join("eon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    listener
}

fn read_request(stream: &mut UnixStream) -> Vec<u8> {
    let mut request = vec![0; HEADER_BYTES];
    stream.read_exact(&mut request).unwrap();
    let length = declared_message_len(&request).unwrap();
    request.resize(length, 0);
    stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
    request
}

/// Serves generation `id` of another build with the current EONW: it answers
/// inspection until it receives Stop for itself.
fn previous_supervisor(runtime: &Path, id: &str) -> thread::JoinHandle<()> {
    let listener = generation_listener(runtime, id);
    let server_id = id.to_string();
    thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let request = decode_request(&read_request(&mut stream)).unwrap();
            let (response, stopped) = match request.action {
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime) => {
                    (
                        LifecycleResponse::Runtime(Runtime {
                            generation: server_id.clone(),
                            eon_version: "0.1.0".into(),
                            workspace_protocol: VERSION,
                            component_report: "previous components".into(),
                            sessions: vec!["session-1".into()],
                            attach: Availability {
                                available: true,
                                reason: "previous presentation".into(),
                            },
                            stop: Availability {
                                available: true,
                                reason: "supervisor owns these Sessions".into(),
                            },
                        }),
                        false,
                    )
                }
                ProtocolAction::Workspace(Action::Stop { generation })
                    if generation == server_id =>
                {
                    (
                        LifecycleResponse::Stopped(Stopped {
                            generation,
                            sessions: vec!["session-1".into()],
                        }),
                        true,
                    )
                }
                action => panic!("unexpected request {action:?}"),
            };
            stream
                .write_all(&encode_lifecycle_response(&response).unwrap())
                .unwrap();
            if stopped {
                return;
            }
        }
    })
}

#[test]
fn generations_from_another_eonw_are_incompatible_and_not_stopped() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let id = format!("g1-{:032x}", 11);
    let listener = generation_listener(&runtime, &id);
    // An older build's supervisor rejects each request in its own framing.
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(
                decode_request(&read_request(&mut stream)).unwrap().action,
                ProtocolAction::Workspace(Action::InspectPresentation)
            );
            let mut failure = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: format!("unsupported EONW version {VERSION}"),
            }))
            .unwrap();
            failure[4..6].copy_from_slice(&11u16.to_le_bytes());
            stream.write_all(&failure).unwrap();
        }
    });

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let listed = invoke(&binary, &runtime, &config, &["generations", "--json"]);
    assert!(listed.status.success(), "{}", stdout(&listed));
    let listed: serde_json::Value = serde_json::from_str(stdout(&listed)).unwrap();
    let older = listed["generations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|generation| generation["id"] == id.as_str())
        .unwrap();
    assert_eq!(older["state"], "incompatible");
    assert_eq!(older["stop"]["available"], false);
    assert!(
        older["stop"]["reason"]
            .as_str()
            .unwrap()
            .contains("supervisor uses EONW 11"),
        "{older}"
    );

    let stopped = invoke(&binary, &runtime, &config, &["stop", &id, "--json"]);
    assert_eq!(stopped.status.code(), Some(1), "{}", stdout(&stopped));
    let stopped: serde_json::Value = serde_json::from_str(stdout(&stopped)).unwrap();
    assert_eq!(stopped["error"]["code"], "stop-unavailable", "{stopped}");
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn window_attach_runs_the_build_that_started_its_generation() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let windows = runtime.join("w");
    let ended = windows.join("0000dcba");
    fs::create_dir_all(ended.join("generations")).unwrap();
    for directory in [&runtime, &windows, &ended, &ended.join("generations")] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let window = windows.join("0000abcd");
    let id = format!("g1-{:032x}", 12);
    let listener = generation_listener(&window, &id);
    let done = root.join("done");
    // An older build's supervisor rejects each request in its own framing.
    let server_done = done.clone();
    let server = thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            if server_done.exists() {
                return;
            }
            read_request(&mut stream);
            let mut failure = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: format!("unsupported EONW version {VERSION}"),
            }))
            .unwrap();
            failure[4..6].copy_from_slice(&11u16.to_le_bytes());
            stream.write_all(&failure).unwrap();
        }
    });
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let listed = |window: &str| {
        let output = invoke(&binary, &runtime, &config, &["windows", "--json"]);
        assert!(output.status.success(), "{}", stdout(&output));
        let listed: serde_json::Value = serde_json::from_str(stdout(&output)).unwrap();
        listed["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == window)
            .unwrap()
            .clone()
    };

    // A window with no supervisor is listed as ended, never as the current generation.
    let ended_window = listed("0000dcba");
    assert_eq!(ended_window["ended"], true, "{ended_window}");
    assert_eq!(
        ended_window["inventory"]["generations"],
        serde_json::json!([])
    );
    let human = invoke(&binary, &runtime, &config, &["windows"]);
    assert!(
        stdout(&human).contains("window 0000dcba ended\n"),
        "{}",
        stdout(&human)
    );
    assert!(
        stdout(&human).contains(&format!("window 0000abcd\n  previous {id} incompatible")),
        "{}",
        stdout(&human)
    );
    let attach_ended = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", "0000dcba"],
    );
    assert_eq!(attach_ended.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&attach_ended.stderr).contains("has ended"));

    // Without a record of its build, attach names the generation and the recovery.
    let older = listed("0000abcd");
    assert_eq!(older["ended"], false, "{older}");
    let generations = older["inventory"]["generations"].as_array().unwrap();
    assert_eq!(generations.len(), 1, "{older}");
    assert_eq!(generations[0]["id"], id.as_str());
    assert_eq!(generations[0]["attach"]["available"], false);
    let unrecorded = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", "0000abcd"],
    );
    assert_eq!(unrecorded.status.code(), Some(1));
    let error = String::from_utf8_lossy(&unrecorded.stderr);
    assert!(error.contains(&id), "{error}");
    assert!(error.contains("EONW 11"), "{error}");
    assert!(
        error.contains("the Eon build that started it can attach or stop it"),
        "{error}"
    );

    // With the record, attach runs that build's `eon` in the window.
    let attached = root.join("attached");
    let older_eon = root.join("older-eon");
    executable(
        &older_eon,
        format!(
            "#!/bin/sh\nprintf '%s %s|%s' \"$1\" \"$2\" \"$EON_WINDOW_RUNTIME_DIR\" > '{}'\nexit 7\n",
            attached.display()
        ),
    );
    // A relative record is not trusted.
    let record = window.join("generations").join(&id).join("eon");
    symlink("older-eon", &record).unwrap();
    assert_eq!(
        listed("0000abcd")["inventory"]["generations"][0]["attach"]["available"],
        false
    );
    fs::remove_file(&record).unwrap();
    symlink(&older_eon, &record).unwrap();
    let recorded = &listed("0000abcd")["inventory"]["generations"][0];
    assert_eq!(recorded["attach"]["available"], true, "{recorded}");
    assert_eq!(recorded["executable"], older_eon.to_str().unwrap());
    let human = invoke(&binary, &runtime, &config, &["windows"]);
    assert!(
        stdout(&human).contains(&format!("  executable {}\n", older_eon.display())),
        "{}",
        stdout(&human)
    );
    let delegated = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", "0000abcd"],
    );
    assert_eq!(
        delegated.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&delegated.stderr)
    );
    assert_eq!(
        fs::read_to_string(&attached).unwrap(),
        format!("attach {id}|{}", window.display())
    );

    // A generation with the current EONW and another component graph is
    // presented by its own build too, but a delegated attach never delegates again.
    let previous = format!("g1-{:032x}", 13);
    let previous_server = previous_supervisor(&runtime, &previous);
    symlink(
        &older_eon,
        runtime.join("generations").join(&previous).join("eon"),
    )
    .unwrap();
    let delegated = invoke(&binary, &runtime, &config, &["attach", &previous]);
    assert_eq!(delegated.status.code(), Some(7));
    assert_eq!(
        fs::read_to_string(&attached).unwrap(),
        format!("attach {previous}|")
    );
    let again = eon_command(&binary)
        .args(["attach", &previous])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ATTACH_DELEGATED", &previous)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("different component graph"));
    let stopped = invoke(&binary, &runtime, &config, &["stop", &previous, "--json"]);
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    previous_server.join().unwrap();

    // A window running several generations names each one's `eon`.
    let shared = windows.join("0000beef");
    let (first, second) = (format!("g1-{:032x}", 14), format!("g1-{:032x}", 15));
    let servers = [
        previous_supervisor(&shared, &first),
        previous_supervisor(&shared, &second),
    ];
    symlink(
        &older_eon,
        shared.join("generations").join(&first).join("eon"),
    )
    .unwrap();
    let several = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", "0000beef"],
    );
    assert_eq!(several.status.code(), Some(1));
    let error = String::from_utf8_lossy(&several.stderr);
    assert!(
        error.contains(&format!("{first} (its eon: {})", older_eon.display())),
        "{error}"
    );
    assert!(error.contains(&second), "{error}");
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "0000beef", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    for server in servers {
        server.join().unwrap();
    }

    fs::write(&done, "").unwrap();
    drop(UnixStream::connect(window.join("generations").join(&id).join("eon.sock")).unwrap());
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn previous_and_all_stop_older_generations_when_current_is_unstarted() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let empty = invoke(&binary, &runtime, &config, &["stop", "all", "--json"]);
    assert!(empty.status.success(), "{}", stdout(&empty));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout(&empty)).unwrap(),
        serde_json::json!([])
    );
    let first_id = format!("g1-{:032x}", 2);
    let first = previous_supervisor(&runtime, &first_id);

    let previous = invoke(&binary, &runtime, &config, &["stop", "previous", "--json"]);
    assert!(previous.status.success(), "{}", stdout(&previous));
    let result: serde_json::Value = serde_json::from_str(stdout(&previous)).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["stopped"]["generation"], first_id);
    first.join().unwrap();

    let second_id = format!("g1-{:032x}", 3);
    let second = previous_supervisor(&runtime, &second_id);
    let all = invoke(&binary, &runtime, &config, &["stop", "all", "--json"]);
    assert!(all.status.success(), "{}", stdout(&all));
    let result: serde_json::Value = serde_json::from_str(stdout(&all)).unwrap();
    assert_eq!(result[0]["stopped"]["generation"], second_id);
    second.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn batch_stop_preserves_current_until_all_and_reports_unavailable_generations() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut current = TestProcess {
        child,
        stop: stop.clone(),
    };
    let current_runtime = generation_runtime(&runtime);
    wait_for_connection(&current_runtime.join("eon.sock"));
    let current_id = current_runtime.file_name().unwrap().to_str().unwrap();

    let first_id = format!("g1-{:032x}", 2);
    let second_id = format!("g1-{:032x}", 3);
    let first = previous_supervisor(&runtime, &first_id);
    let second = previous_supervisor(&runtime, &second_id);
    let unavailable = runtime.join("generations").join(format!("g1-{:032x}", 1));
    fs::create_dir(&unavailable).unwrap();
    fs::set_permissions(&unavailable, fs::Permissions::from_mode(0o755)).unwrap();

    let previous = invoke(&binary, &runtime, &config, &["stop", "previous", "--json"]);
    assert_eq!(previous.status.code(), Some(1), "{}", stdout(&previous));
    let results: serde_json::Value = serde_json::from_str(stdout(&previous)).unwrap();
    assert_eq!(results.as_array().unwrap().len(), 3);
    assert_eq!(results[0]["error"]["code"], "stop-unavailable");
    assert_eq!(results[1]["stopped"]["generation"], first_id);
    assert_eq!(results[2]["stopped"]["generation"], second_id);
    assert!(current.child.try_wait().unwrap().is_none());
    assert!(
        invoke(&binary, &runtime, &config, &["workspace", "--json"])
            .status
            .success()
    );
    first.join().unwrap();
    second.join().unwrap();
    fs::remove_dir_all(unavailable).unwrap();

    let third_id = format!("g1-{:032x}", 4);
    let third = previous_supervisor(&runtime, &third_id);
    let mut human = eon_command(&binary)
        .args(["stop", "all"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    human.stdin.as_mut().unwrap().write_all(b"y\nn\n").unwrap();
    let human = human.wait_with_output().unwrap();
    assert!(human.status.success(), "{}", stdout(&human));
    assert!(stdout(&human).contains(&format!("stopped generation {third_id}:")));
    assert!(stdout(&human).contains(&format!(
        "cancelled; no Stop sent for generation {current_id}"
    )));
    assert!(current.child.try_wait().unwrap().is_none());
    third.join().unwrap();

    let fourth_id = format!("g1-{:032x}", 5);
    let fourth = previous_supervisor(&runtime, &fourth_id);
    let all = invoke(&binary, &runtime, &config, &["stop", "all", "--json"]);
    assert!(all.status.success(), "{}", stdout(&all));
    let results: serde_json::Value = serde_json::from_str(stdout(&all)).unwrap();
    assert_eq!(results.as_array().unwrap().len(), 2);
    assert_eq!(results[0]["stopped"]["generation"], fourth_id);
    assert_eq!(results[1]["stopped"]["generation"], current_id);
    fourth.join().unwrap();
    wait_for_successful_exit(&mut current.child);
    assert!(!current_runtime.exists());
    fs::remove_dir_all(root).unwrap();
}
