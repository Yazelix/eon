use eon_workspace_protocol::{
    Action, HEADER_BYTES, Pane, Request, Response, Snapshot, Tab, VERSION, declared_message_len,
    decode_request, decode_response, encode_request, encode_response,
};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
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
        "eon-control-test-{}-{}",
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

fn wait_for(path: &Path) {
    let ready =
        || fs::metadata(path).is_ok_and(|metadata| !metadata.is_file() || metadata.len() != 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready(), "{} was not created or populated", path.display());
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

fn wait_for_successful_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "process exited with {status}");
            return;
        }
        assert!(Instant::now() < deadline, "Eon process did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

fn invoke(binary: &Path, runtime: &Path, config: &Path, arguments: &[&str]) -> Output {
    Command::new(binary)
        .args(arguments)
        .env("EON_RUNTIME_DIR", runtime)
        .env("EON_CONFIG_HOME", config)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

#[test]
fn closed_stdout_pipe_is_not_a_panic() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
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
fn bare_eon_attaches_only_to_the_live_current_generation() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let log = root.join("venus.log");
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$2\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$EON_TEST_LOG\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = Command::new(&binary)
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

    let output = Command::new(&binary)
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
    let exact = Command::new(&binary)
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
            "{}\n{}\n",
            generation.join("orbit.sock").display(),
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
    let orbit_log = root.join("orbit.log");
    let venus_log = root.join("venus.log");
    let presentation_log = root.join("presentation.log");
    let supervisor_log = root.join("supervisor.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let command = root.join("command");
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$EON_TEST_ORBIT_LOG\"\nendpoint=$2\nprintf '%s' \"$$\" > \"$endpoint\"\nwhile [ \"$1\" != -- ]; do shift; done\nshift\n\"$@\"\nstatus=$?\nrm -f \"$endpoint\"\nexit $status\n",
    );
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s|%s|%s|%s|%s\\n' \"$$\" \"$#\" \"$1\" \"${2-}\" \"$EON_VENUS_PRESENTATION_CONTROL\" >> \"$EON_TEST_VENUS_LOG\"\ndd bs=8 count=1 status=none >> \"$EON_TEST_PRESENTATION_LOG\"\nwhile :; do sleep 0.01; done\n",
    );
    executable(
        &command,
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_CHILD_EXIT\"; do sleep 0.01; done\n",
    );

    let eon = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let binary = root.join("bin/eonterm");
    fs::create_dir(root.join("bin")).unwrap();
    symlink(&eon, &binary).unwrap();
    let eonterm = |configuration: &Path| {
        let mut process = Command::new(&binary);
        process
            .env_remove("EON_RUNTIME_DIR")
            .env("XDG_RUNTIME_DIR", &root)
            .env("EON_CONFIG_HOME", configuration)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_CHILD_EXIT", &child_exit)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
            .env("EON_TEST_VENUS_LOG", &venus_log)
            .env("EON_TEST_PRESENTATION_LOG", &presentation_log);
        process
    };
    let child = eonterm(&config)
        .args(["--no-decorations", "--"])
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
    let initial_venus = fs::read_to_string(&venus_log)
        .unwrap()
        .split('|')
        .next()
        .unwrap()
        .to_string();
    let listed = Command::new(&binary)
        .args(["generations", "--json"])
        .env_remove("EON_RUNTIME_DIR")
        .env("XDG_RUNTIME_DIR", &root)
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(stdout(&listed).contains(generation_id));

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

    assert!(
        Command::new("kill")
            .arg(&initial_venus)
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Path::new("/proc").join(&initial_venus).exists() {
        assert!(Instant::now() < deadline, "terminal surface did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    wait_for(&supervisor_log);
    assert!(
        fs::read_to_string(&supervisor_log)
            .unwrap()
            .contains(&format!(
                "Run `eonterm attach {generation_id}` to reconnect"
            ))
    );

    let reopened = eonterm(&command)
        .args(["attach", generation_id])
        .output()
        .unwrap();
    assert!(reopened.status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&venus_log).unwrap().lines().count() != 2 {
        assert!(Instant::now() < deadline, "EonTerm did not reopen");
        thread::sleep(Duration::from_millis(10));
    }
    let refocused = eonterm(&command)
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
    let venus_log = fs::read_to_string(&venus_log).unwrap();
    let venus_pids = venus_log
        .lines()
        .map(|line| line.split('|').next().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(venus_log.matches("|2|--no-decorations|").count(), 2);
    assert_eq!(venus_log.matches("|stdin\n").count(), 2);
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
    let unknown = Command::new(&binary)
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
fn second_cli_controls_three_live_sessions_without_owning_them() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$2\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = Command::new(&binary)
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
    assert!(
        invoke(&binary, &runtime, &config, &["tab", "create", "--json"])
            .status
            .success()
    );
    for arguments in [
        &["focus", "pane-1", "--json"][..],
        &["focus", "down", "--json"][..],
        &["focus", "right", "--json"][..],
        &["focus", "left", "--json"][..],
        &["focus", "up", "--json"][..],
    ] {
        assert!(
            invoke(&binary, &runtime, &config, arguments)
                .status
                .success()
        );
    }

    let before = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert!(before.status.success());
    let rejected = invoke(&binary, &runtime, &config, &["focus", "up", "--json"]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(stdout(&rejected).contains("\"code\":\"unavailable\""));
    let after = invoke(&binary, &runtime, &config, &["workspace", "--json"]);
    assert_eq!(stdout(&before), stdout(&after));
    let human = invoke(&binary, &runtime, &config, &["workspace"]);
    assert!(human.status.success());
    assert!(stdout(&human).contains("active tab-1\n"));
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
    assert!(snapshot.contains("\"active_tab\":\"tab-1\""));
    assert!(snapshot.contains("\"id\":\"tab-2\""));
    assert!(snapshot.contains("\"id\":\"pane-3\""));
    assert_eq!(snapshot.matches("\"session\":").count(), 3);

    for endpoint in ["orbit.sock", "session-2.sock", "session-3.sock"] {
        let endpoint = generation.join(endpoint);
        wait_for(&endpoint);
        let pid: i32 = fs::read_to_string(endpoint).unwrap().parse().unwrap();
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    }

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
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
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$2\"\ncase \"$2\" in\n  */orbit.sock) while test ! -e \"$EON_TEST_EXIT_INITIAL\"; do sleep 0.01; done ;;\n  *) while test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done ;;\nesac\n",
    );
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\nwhile :; do sleep 0.01; done\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = Command::new(&binary)
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
            && !stdout(&snapshot).contains("pane-1")
            && stdout(&snapshot).contains("pane-2")
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
fn stop_confirmation_refuses_a_replacement_supervisor() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let first_stop = root.join("first-stop");
    let second_stop = root.join("second-stop");
    let prompt = root.join("prompt");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$2\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let launch = |stop: &Path| {
        Command::new(&binary)
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

    let mut confirmation = Command::new(&binary)
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
    let replacement_child_live = fs::read_to_string(&orbit_socket)
        .ok()
        .and_then(|pid| pid.parse::<i32>().ok())
        // SAFETY: signal 0 performs existence/permission checking without sending a signal.
        .is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0);
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
    executable(
        &orbit,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" >> \"$EON_TEST_ORBIT_LOG\"\nsleep 0.25\nprintf '%s' \"$$\" > \"$2\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(&venus, "#!/bin/sh\nexit 0\n");

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let launch = || {
        Command::new(&binary)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &fallback_stop)
            .env("EON_TEST_ORBIT_LOG", &orbit_log)
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
                active_tab: "tab-1".into(),
                tabs: vec![Tab {
                    id: "tab-1".into(),
                    selected_pane: "pane-1".into(),
                    panes: vec![Pane {
                        id: "pane-1".into(),
                        session: "session-1".into(),
                        endpoint: b"/legacy/orbit.sock".to_vec(),
                        live: true,
                    }],
                }],
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

    let attached = Command::new(&binary)
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
            "{}\n{}\n",
            runtime.join("orbit.sock").display(),
            runtime.join("eon.sock").display()
        )
    );

    let stopped = invoke(&binary, &runtime, &config, &["stop", "legacy", "--json"]);
    assert_eq!(stopped.status.code(), Some(2));
    assert!(stdout(&stopped).contains("\"code\":\"stop-unavailable\""));
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
