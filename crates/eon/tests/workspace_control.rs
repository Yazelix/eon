use std::{
    fs,
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixStream},
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

    let mut abandoned = UnixStream::connect(&control).unwrap();
    abandoned
        .write_all(b"abandoned-1\thuman\tinspect\n")
        .unwrap();
    drop(abandoned);

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
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = supervisor.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Eon supervisor did not exit");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    assert!(!control.exists());
    fs::remove_dir_all(root).unwrap();
}
