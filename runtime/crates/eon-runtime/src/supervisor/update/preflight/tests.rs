use super::*;
use crate::{
    attention::Attention,
    control::{ControlListener, socket_identity},
    scrollback::Snapshots,
    sessions,
    supervisor::{lock_supervisor_lifecycle, temporary_directory, try_lock_supervisor_lifecycle},
    workspace::{IdAllocator, Session, WorkspaceSet},
};
use eonu::Inspected;
use std::{
    fs::{self, Permissions},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
};

const LIFE: &str = "g1-00000000000000000000000000000001";
const ACTIVE: &str = "g1-00000000000000000000000000000002";
const OTHER: &str = "g1-00000000000000000000000000000003";

fn target_inputs() -> Inputs {
    let mut inputs = crate::fixtures::inputs();
    inputs.assembly = b"preflight-target-fixture";
    inputs
}

fn wrapper_facts(wrapper: &Path) -> Result<crate::LaunchFacts, String> {
    let executable = wrapper.with_file_name(".eon-wrapped");
    let bytes = fs::read(wrapper.with_extension("facts")).map_err(|error| error.to_string())?;
    crate::LaunchFacts::decode(&bytes, wrapper, &executable)
}

fn executable(path: &Path, script: &str) {
    sessions::tests::script(path, script);
    fs::set_permissions(path, Permissions::from_mode(0o555)).unwrap();
}

fn make_wrapper(store: &Path, name: &str, script: &str) -> PathBuf {
    let bin = store.join(name).join("bin");
    fs::create_dir_all(&bin).unwrap();
    let wrapper = bin.join("eon");
    executable(&wrapper, script);
    let executable = bin.join(".eon-wrapped");
    fs::write(
        wrapper.with_extension("facts"),
        [
            b"EON-LAUNCH-1\0".as_slice(),
            wrapper.as_os_str().as_bytes(),
            b"\0",
            executable.as_os_str().as_bytes(),
            b"\0",
        ]
        .concat(),
    )
    .unwrap();
    wrapper
}

#[test]
fn target_process() {
    let Some(lifetime) = std::env::var_os("EON_TEST_PREFLIGHT_LIFETIME") else {
        return;
    };
    let root = nonempty_environment_path(WINDOW_RUNTIME).unwrap();
    let mut reply = fs::OpenOptions::new()
        .write(true)
        .open("/dev/fd/3")
        .unwrap();
    reply
        .write_all(
            &target_response(
                &target_inputs(),
                &lifetime,
                Some(&root),
                &mut target_stdin().unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
}

struct Window {
    root: PathBuf,
    runtime: PathBuf,
    state: SupervisorState,
    update: PreflightState,
    inputs: Inputs,
    target: PathBuf,
    blocked: PathBuf,
    listener: UpdateListener,
    control: ControlListener,
    _lock: crate::supervisor::LifecycleLock,
}

impl Window {
    fn new() -> Self {
        let root = temporary_directory();
        let runtime = generation_directory(&root, LIFE);
        fs::create_dir_all(&runtime).unwrap();
        fs::set_permissions(&runtime, Permissions::from_mode(0o700)).unwrap();
        let session = sessions::tests::leased_orbit(&runtime, "exec sleep 120");
        let mut inputs = crate::fixtures::inputs();
        let settings = WindowSettings::read(
            root.clone(),
            LaunchMode::Workspace,
            &inputs,
            &LaunchOptions::default(),
            None,
        )
        .unwrap();
        let state_home = root.join("state");
        let registry = Registry::in_test_directory(state_home.join("eon/sessions"));
        let registration = registry.create(Some("preflight fixture")).unwrap();
        let set = WorkspaceSet::registered_recovered(
            Some(registry),
            Some(registration),
            IdAllocator::new(runtime.clone(), 0).unwrap(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                },
            )],
            settings.popup_catalog(&inputs),
        )
        .unwrap();
        let mut state = crate::supervisor::tests::admission_state(&root, Some(set), vec![session]);
        state.component_generation = LIFE.into();
        state.created_build = ACTIVE.into();
        state.active_build = ACTIVE.into();
        state.attention = Some(Attention::new(true));
        state.scrollback = Snapshots::new(&runtime);

        Identity::initial(LIFE, ACTIVE, None)
            .publish(&runtime)
            .unwrap();
        let lock = lock_supervisor_lifecycle(&runtime.join("lifecycle.lock")).unwrap();
        let control = ControlListener::bind(&runtime.join("eon.sock")).unwrap();
        let listener = UpdateListener::bind(&runtime, &state.log).unwrap();
        let store = root.join("store");
        fs::create_dir(&store).unwrap();
        let store = fs::canonicalize(store).unwrap();
        let target = make_wrapper(
            &store,
            "target",
            r#"#!/bin/sh
[ "$1" = "--update-validate-target" ] || exit 1
ulimit -f 0
export EON_TEST_PREFLIGHT_LIFETIME="$2"
exec 3>&1
exec 1>&2
exec "$EON_TEST_EXE" --exact supervisor::update::preflight::tests::target_process --nocapture
"#,
        );
        let blocked = make_wrapper(&store, "blocked", "#!/bin/sh\nexec sleep 60\n");
        let mut environment = vec![
            ("EON_CONFIG_HOME".into(), root.clone().into_os_string()),
            ("XDG_STATE_HOME".into(), state_home.into_os_string()),
            (
                "EON_TEST_EXE".into(),
                std::env::current_exe().unwrap().into_os_string(),
            ),
            (
                crate::LAUNCH_FACTS_VARIABLE.into(),
                store
                    .join("target/share/eon/launch-environment")
                    .into_os_string(),
            ),
        ];
        if let Some(path) = std::env::var_os("PATH") {
            environment.push(("PATH".into(), path));
        }
        inputs.launch = crate::LaunchInputs::capture(
            &wrapper_facts(&target).unwrap(),
            vec!["eon".into()],
            environment,
        );
        assert!(inputs.launch.is_ok());
        inputs.target_launch_facts = wrapper_facts;
        inputs.update_store = store;
        Self {
            root,
            runtime,
            state,
            update: PreflightState::default(),
            inputs,
            target,
            blocked,
            listener,
            control,
            _lock: lock,
        }
    }

    fn request(&self, id: u8) -> Preflight {
        Preflight {
            request: [id; 16],
            lifetime: LIFE.into(),
            expected_active_build: ACTIVE.into(),
            target_wrapper: self.target.as_os_str().as_bytes().to_vec(),
            target_build: current_generation(&target_inputs()).unwrap(),
            caller: None,
        }
    }

    fn turn(&mut self) {
        let log = Arc::clone(&self.state.log);
        self.update.serve(
            &self.listener,
            &mut self.state,
            &self.inputs,
            &self.root,
            &self.runtime,
            &log,
        );
    }

    fn send(&mut self, bytes: &[u8]) -> UnixStream {
        let mut stream = UnixStream::connect(self.listener.path()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(12)))
            .unwrap();
        stream.write_all(bytes).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        self.turn();
        stream
    }

    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(12);
        while self.update.pending.is_some() {
            assert!(Instant::now() < deadline);
            self.turn();
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn refused(&mut self, request: &Preflight, code: Code) {
        let mut response = Vec::new();
        let mut stream = self.send(&request.encode().unwrap());
        if self
            .update
            .pending
            .as_ref()
            .is_some_and(|job| job.request == request.request)
        {
            self.finish();
        }
        stream.read_to_end(&mut response).unwrap();
        let refusal = Refused::decode(&response, Direction::Reply).unwrap();
        assert_eq!(
            (refusal.request, refusal.code),
            (request.request, code),
            "{refusal:?}"
        );
    }

    fn inspect(&mut self, id: u8) -> Inspected {
        let bytes = eonu::Inspect { request: [id; 16] }.encode().unwrap();
        let mut reply = Vec::new();
        self.send(&bytes).read_to_end(&mut reply).unwrap();
        Inspected::decode(&reply).unwrap()
    }

    fn protected(
        &self,
    ) -> (
        eon_workspace_protocol::v47::Snapshot,
        Vec<u8>,
        Vec<(OsString, u64, u64)>,
    ) {
        use std::os::unix::fs::MetadataExt as _;
        let mut files = fs::read_dir(&self.runtime)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let metadata = fs::symlink_metadata(&path).unwrap();
                (
                    path.file_name().unwrap().to_os_string(),
                    metadata.dev(),
                    metadata.ino(),
                )
            })
            .collect::<Vec<_>>();
        files.sort_unstable();
        (
            self.state.workspaces.as_ref().unwrap().snapshot(),
            fs::read(self.runtime.join("identity")).unwrap(),
            files,
        )
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        drop(self.update.pending.take());
        let _ = sessions::stop_managed_sessions(
            &mut self.state.sessions,
            eon_workspace_protocol::SESSION_CLEANUP_TIMEOUT,
        );
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn protected_tree(root: &Path) -> std::collections::BTreeMap<PathBuf, (u64, u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt as _;
    let mut entries = std::collections::BTreeMap::new();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            entries.insert(
                path.clone(),
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                ),
            );
            // Supervisor and Orbit diagnostics are owned by the parent and host.
            if metadata.is_dir()
                && path != root.join("logs")
                && path.file_name() != Some(OsStr::new("launch-logs"))
            {
                directories.push(path);
            }
        }
    }
    entries
}

#[test]
fn admission_and_lost_reply_leave_a_live_window_untouched() {
    let mut window = Window::new();
    let protected = window.protected();
    let (actual_caller, origin) = {
        let host = window.state.sessions[0].inventory();
        (
            sessions::terminal_identity(&host.terminal, &host.run),
            host.origin_build.unwrap(),
        )
    };
    let mut request = window.request(1);
    request.lifetime = OTHER.into();
    request.expected_active_build = OTHER.into();
    window.refused(&request, Code::Lifetime);
    request.lifetime = LIFE.into();
    window.refused(&request, Code::StaleBuild);

    let mut request = window.request(2);
    request.target_wrapper = b"/bin/sh".to_vec();
    request.caller = Some("absent".into());
    window.refused(&request, Code::UntrustedTarget);
    let outside_store = fs::canonicalize(&window.root).unwrap().join("relocated");
    let relocated = make_wrapper(&outside_store, "bundle", "#!/bin/sh\nexit 1\n");
    let writable = make_wrapper(
        &window.inputs.update_store,
        "writable",
        "#!/bin/sh\nexit 1\n",
    );
    fs::set_permissions(&writable, Permissions::from_mode(0o755)).unwrap();
    let linked = make_wrapper(
        &window.inputs.update_store,
        "aliased",
        "#!/bin/sh\nexit 1\n",
    );
    let linked_target = linked.with_extension("real");
    fs::rename(&linked, &linked_target).unwrap();
    std::os::unix::fs::symlink(&linked_target, &linked).unwrap();
    for wrapper in [&relocated, &writable, &linked] {
        assert!(wrapper_facts(wrapper).is_ok());
        request.target_wrapper = wrapper.as_os_str().as_bytes().to_vec();
        window.refused(&request, Code::UntrustedTarget);
    }
    let invalid_facts = make_wrapper(
        &window.inputs.update_store,
        "invalid",
        "#!/bin/sh\nexit 1\n",
    );
    fs::write(invalid_facts.with_extension("facts"), b"invalid").unwrap();
    request.target_wrapper = invalid_facts.as_os_str().as_bytes().to_vec();
    window.refused(&request, Code::UntrustedTarget);

    request.target_wrapper = window.target.as_os_str().as_bytes().to_vec();
    window.refused(&request, Code::UnknownCaller);
    request.caller = Some(actual_caller.clone());
    request.target_build = origin.clone();
    window.refused(&request, Code::StaleCommand);

    let (recovered, _lease, _) = sessions::tests::tombstoned_session(
        &window.runtime,
        1,
        orbit_protocol::management::TerminationReason::NaturalExit,
    );
    let recovered_inventory = recovered.inventory();
    let recovered_caller =
        sessions::terminal_identity(&recovered_inventory.terminal, &recovered_inventory.run);
    let fresh = std::mem::replace(&mut window.state.sessions[0], recovered);
    request.caller = Some(recovered_caller);
    window.refused(&request, Code::UnknownOrigin);
    window.state.sessions[0] = fresh;
    fs::remove_file(window.runtime.join("terminal-1.record")).unwrap();

    let mut request = window.request(3);
    request.target_wrapper = window.blocked.as_os_str().as_bytes().to_vec();
    let client = window.send(&request.encode().unwrap());
    assert!(window.update.pending.is_some());
    let mut busy = window.request(4);
    busy.target_wrapper = b"/bin/sh".to_vec();
    window.refused(&busy, Code::Busy);
    drop(client);
    drop(window.update.pending.take());

    let mut timeout = window.request(12);
    timeout.target_wrapper = window.blocked.as_os_str().as_bytes().to_vec();
    window.update.child_timeout = Some(Duration::from_millis(500));
    window.refused(&timeout, Code::Internal);
    window.update.child_timeout = None;
    assert!(window.update.pending.is_none());
    let timed_out = window.inspect(13).last.unwrap();
    assert_eq!(
        (timed_out.request, timed_out.refusal),
        (timeout.request, Some(Code::Internal))
    );
    assert!(timed_out.detail.contains("timed out"), "{timed_out:?}");

    let request = window.request(8);
    window.state.stopping = Some(crate::supervisor::StopReason::StopRequired);
    window.refused(&request, Code::Unsettled);
    window.state.stopping = None;

    let wrong = make_wrapper(
        &window.inputs.update_store,
        "wrong-build",
        "#!/bin/sh\ncat >/dev/null\nexec cat \"$0.verdict\"\n",
    );
    fs::write(
        wrong.with_extension("verdict"),
        TargetVerdict {
            target_build: OTHER.into(),
            eonw: 48,
            update_schema: 1,
        }
        .encode()
        .unwrap(),
    )
    .unwrap();
    let mut request = window.request(9);
    request.target_wrapper = wrong.as_os_str().as_bytes().to_vec();
    window.refused(&request, Code::TargetRefused);

    let malformed = make_wrapper(
        &window.inputs.update_store,
        "malformed",
        "#!/bin/sh\nprintf 'not EONU'\n",
    );
    let mut request = window.request(10);
    request.target_wrapper = malformed.as_os_str().as_bytes().to_vec();
    window.refused(&request, Code::Internal);
    assert!(window.update.pending.is_none());
    let failed = window.inspect(11).last.unwrap();
    assert_eq!(
        (failed.request, failed.refusal),
        (request.request, Some(Code::Internal))
    );

    let future = make_wrapper(
        &window.inputs.update_store,
        "future-update",
        "#!/bin/sh\nprintf 'EONU\\002\\000'\n",
    );
    let mut request = window.request(14);
    request.target_wrapper = future.as_os_str().as_bytes().to_vec();
    window.refused(&request, Code::Version);
    assert!(
        window
            .inspect(15)
            .last
            .unwrap()
            .detail
            .contains("move the window by hand once")
    );

    let lock_path = window.runtime.join("lifecycle.lock");
    assert!(try_lock_supervisor_lifecycle(&lock_path).unwrap().is_none());
    let sockets = ["eon.sock", update::SOCKET, "orbit.sock"]
        .map(|name| socket_identity(&window.runtime.join(name)).unwrap());
    let registry_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(window.root.join("state/eon/sessions/registry.lock"))
        .unwrap();
    registry_lock.try_lock().unwrap();
    let tree_before = protected_tree(&window.root);
    let request = window.request(5);
    drop(window.send(&request.encode().unwrap()));
    window.finish();
    let last = window.inspect(6).last.unwrap();
    assert_eq!(last.request, request.request);
    assert_eq!(last.refusal, None, "{last:?}");
    assert_eq!(
        last.detail,
        format!("target EONW {}", eon_workspace_protocol::v47::VERSION)
    );
    assert_eq!(
        sockets,
        ["eon.sock", update::SOCKET, "orbit.sock"]
            .map(|name| socket_identity(&window.runtime.join(name)).unwrap())
    );
    assert!(matches!(
        window
            .control
            .accept(|_, _| -> crate::control::Dispatched {
                panic!("the validation child connected to workspace control")
            })
            .unwrap(),
        crate::control::Accepted::None
    ));
    assert_eq!(window.protected(), protected);
    assert_eq!(protected_tree(&window.root), tree_before);
    // Opening this lock reapplies its permissions and changes its ctime.
    assert!(try_lock_supervisor_lifecycle(&lock_path).unwrap().is_none());
    let log = Arc::clone(&window.state.log);
    assert!(sessions::read_visible(&mut window.state.sessions[0], &log).is_ok());

    let lifetime = window.runtime.clone();
    let mut request = window.request(7);
    request.caller = Some(actual_caller);
    let client = thread::spawn(move || {
        update::preflight(
            &lifetime,
            &request,
            Instant::now() + Duration::from_secs(12),
        )
    });
    while !client.is_finished() {
        window.turn();
        thread::sleep(Duration::from_millis(5));
    }
    let reply = client.join().unwrap().unwrap();
    assert_eq!(reply.request, [7; 16]);
    assert_eq!(reply.caller_origin, Some(origin));
}

#[test]
fn retained_hosts_require_each_exact_orbit_version() {
    let window = Window::new();
    let expected = window.state.update_hosts().remove(0);
    let mut check = bare_check(0);
    check.namespace = window.root.as_os_str().as_bytes().to_vec();
    let inputs = target_inputs();
    check.target_build = current_generation(&inputs).unwrap();
    for (field, host) in [
        (
            "management",
            eonu::Host {
                management: expected.management + 1,
                ..expected.clone()
            },
        ),
        (
            "record generation",
            eonu::Host {
                record_generation: expected.record_generation + 1,
                ..expected.clone()
            },
        ),
        (
            "ORBS",
            eonu::Host {
                orbs: None,
                ..expected.clone()
            },
        ),
        (
            "ORBF",
            eonu::Host {
                orbf: Some(orbit_protocol::VERSION + 1),
                ..expected
            },
        ),
    ] {
        check.hosts = vec![host];
        let refused = validate_check(&inputs, LIFE, &window.root, &check).unwrap_err();
        assert_eq!(refused.code, Code::HostProtocol, "{refused:?}");
        assert!(refused.detail.contains(field), "{refused:?}");
        assert!(refused.detail.contains(update::MANUAL_MOVE), "{refused:?}");
    }
}

fn bare_check(size: usize) -> TargetCheck {
    TargetCheck {
        lifetime: LIFE.into(),
        namespace: b"/test".to_vec(),
        created_build: ACTIVE.into(),
        active_build: ACTIVE.into(),
        identity_range: 0,
        target_build: OTHER.into(),
        hosts: Vec::new(),
        checkpoint: vec![0xa0; size],
    }
}

#[test]
fn pipe_failures_and_early_refusal_reap_the_exact_child() {
    let root = temporary_directory();
    let log = DiagnosticLog::open_in(Ok(root.join("logs")), LIFE, "eon");
    let response = root.join("response");
    let (cancelled, _cancel) = io::pipe().unwrap();
    let mut sequence = 0;
    let mut run = |body: String, size| {
        sequence += 1;
        let pid = root.join(format!("pid-{sequence}"));
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &body, "--"]).arg(&pid).arg(&response);
        let result = exchange(
            command,
            &bare_check(size),
            &cancelled,
            &log,
            Duration::from_secs(2),
        );
        let child: i32 = fs::read_to_string(&pid).unwrap().parse().unwrap();
        assert!(matches!(
            rustix::process::waitpid(
                rustix::process::Pid::from_raw(child),
                rustix::process::WaitOptions::NOHANG
            ),
            Err(rustix::io::Errno::CHILD)
        ));
        result
    };
    let mark = r#"printf '%s' "$$" > "$1"; "#;
    for (body, size, expected) in [
        ("exec sleep 60", 8 << 20, io::ErrorKind::TimedOut),
        (
            "cat >/dev/null; exec sleep 60",
            1024,
            io::ErrorKind::TimedOut,
        ),
        ("printf 'not EONU'; exit", 1024, io::ErrorKind::Other),
        ("exit", 1024, io::ErrorKind::Other),
    ] {
        assert_eq!(
            run(format!("{mark}{body}"), size).unwrap_err().kind(),
            expected
        );
    }
    let refusal = refusal(
        [0; 16],
        Code::TargetRefused,
        "configuration is invalid".into(),
    );
    fs::write(&response, refusal.encode().unwrap()).unwrap();
    assert_eq!(
        run(format!("{mark}exec cat \"$2\""), 8 << 20).unwrap(),
        Err(refusal)
    );
    let verdict = TargetVerdict {
        target_build: OTHER.into(),
        eonw: 48,
        update_schema: 1,
    };
    fs::write(&response, verdict.encode().unwrap()).unwrap();
    assert_eq!(
        run(format!("{mark}cat >/dev/null; exec cat \"$2\""), 1024).unwrap(),
        Ok(verdict)
    );
    drop(log);
    fs::remove_dir_all(root).unwrap();
}
