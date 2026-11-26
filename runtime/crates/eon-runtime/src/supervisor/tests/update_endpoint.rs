//! `update.sock` in a real full-Eon window: `supervise` in its own process,
//! with one recovered real Orbit terminal, in an isolated root.

use super::*;
use crate::{control, update};
use std::process::{Child, Stdio};

/// A window under test and the Orbit host it must keep alive.
struct Window {
    root: PathBuf,
    runtime: PathBuf,
    generation: String,
    child: Option<Child>,
    orbit: i32,
}

impl Window {
    /// Starts a window whose root `prepare` may seed first, with `fault`
    /// passed to the child as `EON_UPDATE_TEST_FAULT`.
    fn start(prepare: impl FnOnce(&Path), fault: Option<&str>) -> Self {
        let generation = current_generation(&crate::fixtures::inputs()).unwrap();
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = root.join("a").join(&generation);
        prepare_runtime(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        crate::session_registry::Registry::in_test_directory(root.join("state/eon/sessions"));
        script(&root.join("venus"), "#!/bin/sh\nexec sleep 60\n");
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = std::env::var_os("ORBIT_BIN")
            .expect("ORBIT_BIN names the Orbit to run")
            .into();
        let log = DiagnosticLog::open_in(Ok(root.join("orbit-logs")), &generation, "eon");
        let live = super::super::start_orbit(
            &inputs,
            &root,
            &runtime.join("orbit.sock"),
            "terminal-1",
            &generation,
            &root,
            &["sh".into(), "-c".into(), "exec sleep 60".into()],
            None,
            &log,
            Instant::now() + Duration::from_secs(5),
            &mut sessions::FailedStarts::default(),
            &mut sessions::sleep_until,
        )
        .unwrap();
        let orbit = orbit_pid(&live);
        sessions::tests::release_lease(live);
        prepare(&runtime);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "supervisor::tests::c38_supervisor_startup_process",
                "--nocapture",
            ])
            .env("EON_C38_STARTUP_ROOT", &root)
            .env("HOME", &root)
            .env("EON_C38_GENERATION", &generation)
            .env("XDG_STATE_HOME", root.join("state"))
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(fault) = fault {
            command.env("EON_UPDATE_TEST_FAULT", fault);
        }
        let mut window = Self {
            root,
            runtime,
            generation,
            child: Some(command.spawn().unwrap()),
            orbit,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while !window.control_answers() {
            let exited = window.child.as_mut().unwrap().try_wait().unwrap().is_some();
            if exited || Instant::now() >= deadline {
                window.fail("the window never answered on eon.sock");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        window
    }

    /// An ordinary EONW request still gets its answer.
    fn control_answers(&self) -> bool {
        matches!(
            control::send_action(
                &self.runtime.join("eon.sock"),
                Action::Workspace(WorkspaceAction::Inspect)
            ),
            Ok(control::ControlResponse::Workspace(
                super::super::Response::Snapshot(_)
            ))
        )
    }

    /// What a refused request must leave unchanged: the workspace snapshot,
    /// the identity record's bytes and the lifetime directory's names.
    fn state(
        &self,
    ) -> (
        eon_workspace_protocol::v51::Snapshot,
        Option<Vec<u8>>,
        Vec<std::ffi::OsString>,
    ) {
        let Ok(control::ControlResponse::Workspace(super::super::Response::Snapshot(snapshot))) =
            control::send_action(
                &self.runtime.join("eon.sock"),
                Action::Workspace(WorkspaceAction::Inspect),
            )
        else {
            panic!("no snapshot");
        };
        let mut names = fs::read_dir(&self.runtime)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        (
            snapshot,
            fs::read(self.runtime.join("identity")).ok(),
            names,
        )
    }

    /// Sends `bytes` to eon.sock and returns its whole reply.
    fn send_control_raw(&self, bytes: &[u8]) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut stream = UnixStream::connect(self.runtime.join("eon.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let _ = stream.write_all(bytes);
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply);
        reply
    }

    fn orbit_alive(&self) -> bool {
        rustix::process::test_kill_process(Pid::from_raw(self.orbit).unwrap()).is_ok()
    }

    fn update_socket(&self) -> PathBuf {
        self.runtime.join(update::SOCKET)
    }

    /// Kills everything this window started and panics with its output.
    fn fail(&mut self, why: &str) -> ! {
        let mut child = self.child.take().unwrap();
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        self.reap_orbit();
        panic!(
            "{why}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn reap_orbit(&self) {
        let pid = Pid::from_raw(self.orbit).unwrap();
        let _ = kill_process(pid, Signal::KILL);
        let _ = rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty());
    }

    /// Stops the window as a person would, then requires a clean exit.
    fn stop(self) -> PathBuf {
        self.stop_with_stderr().0
    }

    /// `stop`, also returning what the window wrote on stderr.
    fn stop_with_stderr(mut self) -> (PathBuf, String) {
        let stopped = control::send_stop_on(
            UnixStream::connect(self.runtime.join("eon.sock")).unwrap(),
            &self.generation,
        );
        if !matches!(
            stopped,
            Ok(control::StopAnswer::Reply(
                eon_workspace_protocol::stop::Reply::Stopped { .. }
            ))
        ) {
            self.fail("Stop was not answered");
        }
        let output = self.child.take().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.reap_orbit();
        (
            self.root.clone(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Sends `bytes` to update.sock, reads to EOF, and returns the reply.
    fn send_raw(&self, bytes: &[u8]) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut stream = UnixStream::connect(self.update_socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let _ = stream.write_all(bytes);
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply);
        reply
    }
}

#[test]
fn the_window_serves_update_sock_beside_eon_sock_and_removes_both() {
    let window = Window::start(|_| {}, None);
    let socket = window.update_socket();
    let metadata = fs::symlink_metadata(&socket).unwrap();
    assert!(std::os::unix::fs::FileTypeExt::is_socket(
        &metadata.file_type()
    ));
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert!(window.orbit_alive());
    let runtime = window.runtime.clone();
    let root = window.stop();
    assert!(!runtime.join("eon.sock").exists());
    assert!(!runtime.join(update::SOCKET).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failed_update_bind_never_stops_the_window() {
    // A foreign object at the path, and an injected bind failure: the window
    // starts, serves eon.sock, keeps its terminal and stops cleanly.
    for case in ["object", "fault"] {
        let window = Window::start(
            |runtime| {
                if case == "object" {
                    fs::write(runtime.join(update::SOCKET), b"not a socket").unwrap();
                }
            },
            (case == "fault").then_some("bind"),
        );
        match case {
            "object" => assert!(window.update_socket().is_file()),
            _ => assert!(!window.update_socket().exists()),
        }
        assert!(window.control_answers(), "{case}");
        assert!(window.orbit_alive(), "{case}");
        let root = window.stop();
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn a_real_window_answers_inspect_and_keeps_serving_eonw() {
    use eon_workspace_protocol::update::{self as eonu, Code, Direction, Refused};
    let window = Window::start(|_| {}, None);
    let inspected = update::inspect(&window.runtime, Instant::now() + Duration::from_secs(10))
        .unwrap_or_else(|refusal| panic!("{refusal}"));
    assert_eq!(inspected.lifetime, window.generation);
    assert_eq!(inspected.pid, window.pid());
    assert_eq!(inspected.eonw, eon_workspace_protocol::v51::VERSION);
    assert_eq!(inspected.eonr, crate::checkpoint::VERSION);
    assert!(
        inspected
            .hosts
            .iter()
            .any(|host| host.terminal == "terminal-1"),
        "{:?}",
        inspected.hosts
    );
    let before = window.state();
    // A request this slice does not serve, and EONW bytes on the wrong socket.
    for (kind, code) in [(2, Code::Malformed), (3, Code::Unsupported)] {
        let mut request = b"EONU\x01\x00\x00\x00\x10\x00\x00\x00".to_vec();
        request[6] = kind;
        request.extend_from_slice(&[4; 16]);
        let reply = Refused::decode(&window.send_raw(&request), Direction::Reply).unwrap();
        assert_eq!((reply.code, reply.request), (code, [4; 16]));
    }
    let reply = window.send_raw(b"EONW\x2f\x00\x01\x00\x00\x00\x00\x00");
    assert_eq!(
        Refused::decode(&reply, Direction::Reply).unwrap().code,
        Code::Malformed
    );
    assert_eq!(&reply[..eonu::PREFIX_BYTES], b"EONU\x01\x00");
    let reply = window.send_raw(b"EONU\x01\x00\x01\x07\x10\x00\x00\x00");
    assert_eq!(
        Refused::decode(&reply, Direction::Reply).unwrap().code,
        Code::Malformed
    );
    // An EONU frame on eon.sock gets exactly what any foreign bytes get there.
    let inspect = eonu::Inspect { request: [8; 16] }.encode().unwrap();
    let mut foreign = inspect.clone();
    foreign[..4].copy_from_slice(b"XXXX");
    let answered = window.send_control_raw(&inspect);
    assert!(!answered.is_empty());
    assert_eq!(answered, window.send_control_raw(&foreign));
    assert_eq!(window.state(), before, "refusals change nothing");
    // An unreadable identity record refuses; it is not "no launcher".
    let record = window.runtime.join("identity");
    let kept = fs::read(&record).ok();
    fs::write(&record, b"not an identity record").unwrap();
    let refused = update::inspect(&window.runtime, Instant::now() + Duration::from_secs(10));
    match kept {
        Some(bytes) => fs::write(&record, bytes).unwrap(),
        None => fs::remove_file(&record).unwrap(),
    }
    assert!(
        matches!(
            refused,
            Err(update::Refusal::Refused {
                code: Code::Internal,
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(window.state(), before);
    assert!(window.control_answers());
    assert!(window.orbit_alive());
    let root = window.stop();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failing_update_accept_never_stops_the_window() {
    let window = Window::start(|_| {}, Some("accept"));
    for _ in 0..3 {
        let result = update::inspect(&window.runtime, Instant::now() + Duration::from_millis(300));
        assert!(
            matches!(result, Err(update::Refusal::Unavailable(_))),
            "{result:?}"
        );
    }
    assert!(window.control_answers());
    assert!(window.orbit_alive());
    let (root, stderr) = window.stop_with_stderr();
    assert_eq!(
        stderr.matches("cannot accept an Eon Update client").count(),
        1,
        "logged once: {stderr}"
    );
    fs::remove_dir_all(root).unwrap();
}
