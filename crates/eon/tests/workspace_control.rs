use eon_workspace_protocol::{
    Action, HEADER_BYTES, Request, Response, VERSION, declared_message_len, decode_response,
    encode_request,
};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "{} was not created", path.display());
}

fn wait_for_successful_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "Eon supervisor did not exit");
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
fn relative_configuration_root_is_resolved_once() {
    let root = temporary_directory();
    let config = root.join("config");
    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
        .arg("config-path")
        .current_dir(&root)
        .env("EON_CONFIG_HOME", "config")
        .output()
        .unwrap();

    assert!(output.status.success());
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
fn bare_eon_attaches_through_workspace_when_initial_session_is_gone() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let venus = root.join("venus");
    let log = root.join("venus.log");
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let _control = UnixListener::bind(runtime.join("eon.sock")).unwrap();
    executable(
        &venus,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$EON_TEST_LOG\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_LOG", &log)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(log).unwrap(),
        format!(
            "{}\n{}\n",
            runtime.join("orbit.sock").display(),
            runtime.join("eon.sock").display()
        )
    );
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
    let control = runtime.join("eon.sock");
    wait_for(&control);

    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create", "--json"])
            .status
            .success()
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
        let endpoint = runtime.join(endpoint);
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
    let control = runtime.join("eon.sock");
    wait_for(&control);
    wait_for(&venus_pid);
    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create", "--json"])
            .status
            .success()
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
