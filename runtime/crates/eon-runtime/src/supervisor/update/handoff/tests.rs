//! A/B subprocess exercise of the private EONU adoption route.

use super::*;
use crate::launch_environment::WAYLAND_SOCKET;
use crate::{
    attention::Attention,
    control::{StopAnswer, connect_control, send_stop_on, socket_identity},
    scrollback::Snapshots,
    session_registry::Registry,
    sessions::{self, FailedStarts},
    supervisor::{
        PresentationProcess, launch_current, lock_supervisor_lifecycle, prepare_runtime,
        temporary_directory,
    },
    workspace::{IdAllocator, Session, WorkspaceSet},
};
use eon_workspace_protocol::update::{self as eonu, Inspect, Inspected, Preflighted};
use std::{
    env,
    ffi::OsStr,
    io::Read,
    os::unix::{ffi::OsStringExt, fs::OpenOptionsExt, fs::PermissionsExt},
    path::PathBuf,
    process::{Child, Stdio},
    sync::Arc,
};

const LIFE: &str = "g1-00000000000000000000000000000001";
const ENTRY: &str = "supervisor::update::handoff::tests::update_process";
const GATE: &str = "EON_TEST_HANDOFF";
const START: Duration = Duration::from_secs(30);
const SENTINEL: &str = "EON_TEST_FRESH_SENTINEL";

fn bin(root: &Path, name: &str) -> PathBuf {
    root.join("store").join(name).join("bin")
}

fn facts(wrapper: &Path) -> Result<LaunchFacts, String> {
    let bytes = fs::read(wrapper.with_extension("facts")).map_err(|error| error.to_string())?;
    LaunchFacts::decode(&bytes, wrapper, &wrapper.with_file_name(".eon-wrapped"))
}

fn inputs(root: &Path, name: &str) -> Inputs {
    let mut inputs = crate::fixtures::inputs();
    inputs.assembly = if name == "a" { b"image A" } else { b"image B" };
    inputs.orbit = env::var_os("ORBIT_BIN")
        .map(PathBuf::from)
        .unwrap_or_default();
    inputs.managed.shell_bin = Some(bin(root, name));
    inputs.update_store = root.join("store");
    inputs.target_launch_facts = facts;
    inputs
}

fn wrapper(root: &Path, name: &str, image: &Path) {
    let bin = bin(root, name);
    fs::create_dir_all(&bin).unwrap();
    let wrapper = bin.join("eon");
    let locator = bin.parent().unwrap().join("share/eon/launch-environment");
    let mut frame = [
        b"EON-LAUNCH-1\0".as_slice(),
        wrapper.as_os_str().as_bytes(),
        b"\0",
        bin.join(".eon-wrapped").as_os_str().as_bytes(),
        b"\0prefix\0TERMINFO_DIRS\0",
        name.as_bytes(),
        b"\0",
    ]
    .concat();
    let old = if name == "a" {
        frame.extend(b"set\0EON_TEST_A_ONLY\0source-only\0");
        "export EON_LAUNCH_SAVED_EON_TEST_A_ONLY=- EON_TEST_A_ONLY=source-only"
    } else {
        ""
    };
    fs::write(wrapper.with_extension("facts"), frame).unwrap();
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"));
    let script = format!(
        r#"#!/bin/sh
export EON_TEST_HANDOFF={name}
export EON_LAUNCH_FACTS={locator}
export EON_LAUNCH_SAVED_TERMINFO_DIRS="+$TERMINFO_DIRS"
export TERMINFO_DIRS="{name}:$TERMINFO_DIRS"
{old}
case "$1" in
--update-resume)
  export EON_TEST_HANDOFF_LIFETIME="$2" EON_TEST_HANDOFF_FD="$3"
  ;;
--update-validate-target)
  export EON_TEST_PREFLIGHT_LIFETIME="$2"
  exec 3>&1
  exec 1>&2
  ;;
--check-descriptors)
  export EON_TEST_HANDOFF=check
  ;;
--check-fresh-descriptors)
  export EON_TEST_HANDOFF=fresh-check
  ;;
esac
exec {image} --exact {ENTRY} --nocapture
"#,
        locator = quote(&locator),
        image = quote(image),
    );
    sessions::tests::script(&wrapper, &script);
    fs::set_permissions(wrapper, fs::Permissions::from_mode(0o555)).unwrap();
}

fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + START;
    while !ready() {
        assert!(Instant::now() < deadline, "{what} did not complete");
        thread::sleep(Duration::from_millis(10));
    }
}

fn inherited_sentinel(root: &Path) -> Option<bool> {
    let number = env::var(SENTINEL).ok()?.parse().unwrap();
    let expected = object(&fstat(File::open(root.join("sentinel")).unwrap()).unwrap());
    Some(
        platform::inspect_inherited(number, |fd| Ok(object(&fstat(fd)?) == expected))
            .unwrap_or(false),
    )
}

#[test]
fn update_process() {
    let Ok(mode) = env::var(GATE) else { return };
    let root = PathBuf::from(env::var_os("EON_TEST_HANDOFF_ROOT").unwrap());
    if mode == "fresh-check" {
        use rustix::fs::{FileType, fstat};
        // The control socket moves to fd 3 so libtest output stays off it.
        let input = fstat(io::stdin()).unwrap();
        let output = platform::inspect_inherited(3, |fd| Ok(fstat(fd)?)).unwrap();
        let error = fstat(io::stderr()).unwrap();
        assert_eq!(FileType::from_raw_mode(input.st_mode), FileType::Socket);
        assert_eq!(object(&input), object(&output));
        assert_eq!(FileType::from_raw_mode(error.st_mode), FileType::Fifo);
        assert!(env::var_os("EON_NEW_WINDOW_ENV_5741594c414e445f534f434b4554").is_none());
        let proof = if let Ok(number) = env::var(WAYLAND_SOCKET) {
            let sent = number.parse().ok().is_some_and(|number| {
                platform::inspect_inherited(number, |fd| {
                    rustix::io::write(fd, b"venus").map_err(Into::into)
                })
                .is_ok_and(|written| written == b"venus".len())
            });
            if sent { "display" } else { "invalid-display" }
        } else if inherited_sentinel(&root) == Some(true) {
            "leaked"
        } else {
            "isolated"
        };
        fs::write(root.join("fresh-child"), proof).unwrap();
        return;
    }
    if mode == "check" {
        for line in fs::read_to_string(root.join("descriptor-objects"))
            .unwrap()
            .lines()
        {
            let parts: Vec<u64> = line
                .split_whitespace()
                .map(|word| word.parse().unwrap())
                .collect();
            let &[fd, dev, ino] = parts.as_slice() else {
                unreachable!()
            };
            let escaped = platform::inspect_inherited(fd as i32, |borrowed| {
                let observed = object(&fstat(borrowed)?);
                Ok(observed.device == dev && observed.inode == ino)
            })
            .unwrap_or(false);
            assert!(!escaped, "retained descriptor {fd} escaped to a new child");
        }
        return;
    }
    let mut inputs = inputs(&root, &mode);
    let wrapper = bin(&root, &mode).join("eon");
    inputs.launch = crate::LaunchInputs::capture(
        &facts(&wrapper).unwrap(),
        vec![wrapper.into_os_string()],
        env::vars_os(),
    );
    assert!(inputs.launch.is_ok(), "{:?}", inputs.launch);
    if let Some(lifetime) = env::var_os("EON_TEST_PREFLIGHT_LIFETIME") {
        let mut out = OpenOptions::new().write(true).open("/dev/fd/3").unwrap();
        preflight::reply_for_test(&inputs, &lifetime, &mut out);
        return;
    }
    if mode == "a" {
        if update::fault("fresh-launch-fd") || update::fault("fresh-launch-display") {
            fresh_source(&root, inputs);
        } else {
            source(&root, &inputs);
        }
    } else {
        replacement(&root, &inputs);
    }
}

fn fresh_source(root: &Path, mut inputs: Inputs) {
    if update::fault("fresh-launch-fd") {
        assert_eq!(inherited_sentinel(root), Some(true));
    } else {
        let launch = inputs.launch.as_ref().unwrap();
        assert!(
            launch
                .command(launch.facts())
                .get_envs()
                .all(|(key, _)| key != WAYLAND_SOCKET)
        );
    }
    let runtime = generation_directory(root, LIFE);
    prepare_runtime(&runtime).unwrap();
    crate::generation::record_launcher(&inputs, &runtime).unwrap();
    inputs.venus = root.join("venus");
    let result = launch_current(
        &inputs,
        LaunchMode::Workspace,
        &[
            "/bin/sh".into(),
            "-c".into(),
            r#"printf '%s\n' "${WAYLAND_SOCKET-unset}" > "$EON_TEST_HANDOFF_ROOT/orbit-display"; IFS= read -r answer"#.into(),
        ],
        false,
        false,
        "eon",
        &LaunchOptions {
            directory: Some(root.into()),
            no_anima: true,
        },
    );
    assert_eq!(result, Ok(0));
}

fn replacement(root: &Path, inputs: &Inputs) {
    assert_eq!(env::var("TERMINFO_DIRS").unwrap(), "b:base");
    assert_eq!(env::var_os("EON_TEST_A_ONLY"), None);
    assert_eq!(env::var_os(WINDOW_RUNTIME), None);
    assert_eq!(
        fs::canonicalize(env::current_exe().unwrap()).unwrap(),
        bin(root, "b").join(".eon-wrapped")
    );
    let lifetime = env::var_os("EON_TEST_HANDOFF_LIFETIME").unwrap();
    let fd = env::var_os("EON_TEST_HANDOFF_FD").unwrap();
    let number = fd.to_str().unwrap().parse().unwrap();
    let bytes = read_file(number, &generation_directory(root, LIFE)).unwrap();
    let envelope = Envelope::read(&bytes, number).unwrap();
    let mut descriptors = vec![
        envelope.lifecycle,
        envelope.namespace,
        envelope.control.fd,
        envelope.update.fd,
        envelope.reply,
    ];
    if let Some(venus) = envelope.venus {
        descriptors.extend([venus.control, venus.stderr]);
    }
    descriptors.extend(envelope.leases.iter().map(|lease| lease.fd));
    descriptors.push(Descriptor {
        number,
        object: platform::inspect_inherited(number, |fd| Ok(object(&fstat(fd)?))).unwrap(),
    });
    let mut actual = platform::inherited_numbers().unwrap();
    actual.sort_unstable();
    let mut expected = descriptors.iter().map(|fd| fd.number).collect::<Vec<_>>();
    expected.sort_unstable();
    assert_eq!(
        actual, expected,
        "the replacement inherited exactly the carry set above stdio"
    );
    write_descriptor_table(root, &descriptors).unwrap();
    fs::write(root.join("entered-b"), std::process::id().to_string()).unwrap();
    if root.join("pause").exists() {
        until("resume release", || root.join("release").exists());
    }
    if update::fault("malformed-import")
        || update::fault("venus-identity")
        || update::fault("reply-sent")
    {
        let mut reader = Reader {
            bytes: &bytes,
            numbers: HashSet::new(),
        };
        reader.bytes(18).unwrap();
        reader.frame(HEADER_BYTES + Kind::Carry.bound()).unwrap();
        let reply_progress = bytes.len() - reader.bytes.len() - 1;
        reader
            .frame(HEADER_BYTES + Kind::TargetCheck.bound())
            .unwrap();
        let position = if update::fault("malformed-import") {
            0
        } else if update::fault("reply-sent") {
            reply_progress
        } else {
            bytes.len() - reader.bytes.len() + 132 + 1 + 4
        };
        platform::inspect_inherited(number, |fd| {
            rustix::io::pwrite(fd, &[bytes[position] ^ 1][..], position as u64)?;
            Ok(())
        })
        .unwrap();
    }
    let result = resume_update(inputs, &[lifetime, fd]).map_err(|error| error.message);
    if let Err(error) = &result {
        eprintln!("fixture B resume failed: {error}");
    }
    fs::write(root.join("result"), format!("{result:?}")).unwrap();
}

fn source(root: &Path, inputs: &Inputs) {
    platform::isolate_fresh_descriptors().unwrap();
    assert_eq!(env::var("TERMINFO_DIRS").unwrap(), "a:base");
    assert_eq!(env::var("EON_TEST_A_ONLY").unwrap(), "source-only");
    let runtime = generation_directory(root, LIFE);
    prepare_runtime(&runtime).unwrap();
    let build = current_generation(inputs).unwrap();
    let namespace = NamespaceLock::acquire(root).unwrap();
    let lifecycle = lock_supervisor_lifecycle(&supervisor_lock_path(root, LIFE)).unwrap();
    let identity = Identity::initial(LIFE, &build, crate::generation::installed_launcher(inputs));
    identity.publish(&runtime).unwrap();
    identity.project_launcher(&runtime).unwrap();
    let log = DiagnosticLog::open(LIFE, "eon");
    let settings = WindowSettings::read(
        root.into(),
        LaunchMode::Workspace,
        inputs,
        &LaunchOptions::default(),
        None,
    )
    .unwrap();
    let rows: Vec<_> = [
        (1, "IFS= read -r answer"),
        (
            2,
            "IFS= read -r answer < \"$EON_TEST_HANDOFF_ROOT/exit-gate\"; exit 7",
        ),
    ]
    .into_iter()
    .map(|(number, action)| {
        let command = format!("echo \"$$\" > \"$EON_TEST_HANDOFF_ROOT/pty-{number}\"; {action}");
        sessions::start_orbit(
            inputs,
            root,
            &crate::workspace::terminal_endpoint(&runtime, number),
            &format!("terminal-{number}"),
            LIFE,
            root,
            &["/bin/sh".into(), "-c".into(), command.into()],
            None,
            &log,
            Instant::now() + START,
            &mut FailedStarts::default(),
            &mut sessions::sleep_until,
        )
        .unwrap()
    })
    .collect();
    let registry = Registry::new().unwrap();
    let registry_worker = crate::session_registry::Worker::start(registry.clone()).unwrap();
    let registration = registry.create(Some("handoff")).unwrap();
    let workspaces = WorkspaceSet::registered_recovered(
        Some(registry),
        Some(registration),
        IdAllocator::new(runtime.clone(), 0).unwrap(),
        root.into(),
        Some(root.into()),
        rows.iter()
            .map(|row| {
                (
                    row.number,
                    Session {
                        id: row.id.clone(),
                        endpoint: row.endpoint.clone(),
                    },
                )
            })
            .collect(),
        settings.popup_catalog(inputs),
    )
    .unwrap();
    let mut state = crate::supervisor::tests::admission_state(root, Some(workspaces), rows);
    state.registry_worker = Some(registry_worker);
    state.log = Arc::clone(&log);
    state.component_generation = LIFE.into();
    state.created_build = build.clone();
    state.active_build = build;
    state.scrollback = Snapshots::new(&runtime);
    state.running = Some(
        crate::running::Running::start(root.to_path_buf(), root, LIFE, Arc::clone(&log)).unwrap(),
    );
    if update::fault("held-zoxide") {
        crate::supervisor::record_visit(
            inputs,
            root,
            &log,
            root.as_os_str().as_bytes(),
            &mut state.visits,
        );
    }
    let quota_widget = if update::fault("held-codex") {
        Some("codex_quota_5h")
    } else if update::fault("held-agy-version") || update::fault("held-agy-usage") {
        Some("google_quota:Gemini Models:5h")
    } else {
        None
    };
    if let Some(widget) = quota_widget {
        state.quotas = Some(crate::quota::Quotas::start(
            &bin(root, "a").join("codexbar"),
            &[widget.to_owned()],
        ));
    }
    if root.join("venus").exists() {
        state.venus = Some(
            PresentationProcess::start(
                Command::new(root.join("venus")),
                false,
                None,
                START,
                &mut state.sessions,
                &log,
            )
            .unwrap(),
        );
        until("settled Venus naming", || {
            state.name_presentation(false, None);
            state
                .sessions
                .iter()
                .all(|row| row.checkpoint_extra().is_ok())
        });
    }
    // Keep the checkpoint's required attention state, with its first poll
    // after the fixture unless a test holds a lease's status request.
    let mut attention = Attention::new(false);
    if !update::fault("held-status") {
        attention.defer_poll_until(Instant::now() + START * 4);
    }
    state.attention = Some(attention);
    let control = ControlListener::bind(&runtime.join("eon.sock")).unwrap();
    state.start_pump =
        crate::start_pump::StartPump::new(control.try_clone_listener(), Arc::clone(&log));
    let listener = UpdateListener::bind(&runtime, &log).unwrap();
    drop(namespace);
    let _cleanup = GenerationCleanup {
        runtime: &runtime,
        log: &log,
    };
    let foreign = update::fault("foreign-fd").then(|| {
        let file = File::create(root.join("foreign")).unwrap();
        rustix::io::fcntl_setfd(&file, rustix::io::FdFlags::empty()).unwrap();
        file
    });
    let result = service(
        inputs,
        root,
        &lifecycle,
        &runtime,
        LIFE,
        LaunchMode::Workspace,
        &log,
        state,
        control,
        Some(listener),
        None,
        None,
    );
    if let Some(file) = foreign {
        assert_eq!(
            rustix::io::fcntl_getfd(file).unwrap(),
            rustix::io::FdFlags::empty()
        );
        fs::write(root.join("foreign-intact"), b"").unwrap();
    }
    fs::write(root.join("result"), format!("{result:?}")).unwrap();
}

fn write_descriptor_table(root: &Path, descriptors: &[Descriptor]) -> io::Result<()> {
    let table: String = descriptors
        .iter()
        .map(|fd| format!("{} {} {}\n", fd.number, fd.object.device, fd.object.inode))
        .collect();
    fs::write(root.join("descriptor-objects"), table)
}

pub(super) fn verify_rollback_inheritance(root: &Path, fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    let descriptors = fds
        .iter()
        .map(|fd| {
            Ok(Descriptor {
                number: fd.as_raw_fd(),
                object: object(&fstat(fd)?),
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    write_descriptor_table(root, &descriptors)?;
    verify_descriptors_in_child(root, "a")?;
    fs::write(root.join("verified-rollback-child"), b"ok")
}

pub(super) fn verify_child_inheritance(
    root: &Path,
    venus: Option<&InheritedPresentation>,
) -> io::Result<()> {
    if env::var_os(GATE).as_deref() != Some(OsStr::new("b")) {
        return Ok(());
    }
    verify_descriptors_in_child(root, "b")?;
    if let Some(venus) = venus {
        (&venus.control).write_all(b"present\n")?;
    }
    fs::write(root.join("verified-child"), b"ok")
}

fn verify_descriptors_in_child(root: &Path, image: &str) -> io::Result<()> {
    let status = Command::new(bin(root, image).join("eon"))
        .arg("--check-descriptors")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(invalid("a retained descriptor leaked to a new child"));
    }
    Ok(())
}

struct Fixture {
    root: PathBuf,
    child: Child,
    sentinel: Option<File>,
}

impl Fixture {
    fn start(fault: Option<&str>, pause: bool, attached: bool) -> Self {
        Self::start_with_display(fault, pause, attached, None)
    }

    fn start_with_display(
        fault: Option<&str>,
        pause: bool,
        attached: bool,
        display: Option<OwnedFd>,
    ) -> Self {
        let temporary = temporary_directory();
        let suffix = temporary
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("eon-test-")
            .unwrap();
        let short = temporary.with_file_name(format!("uh-{suffix}"));
        fs::rename(temporary, &short).unwrap();
        let root = fs::canonicalize(short).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(platform::socket_path_fits(
            &generation_directory(&root, LIFE).join("terminal-2.sock.management")
        ));
        let original = env::current_exe().unwrap();
        let image = bin(&root, "b").join(".eon-wrapped");
        fs::create_dir_all(image.parent().unwrap()).unwrap();
        fs::copy(&original, &image).unwrap();
        wrapper(&root, "a", &original);
        wrapper(&root, "b", &image);
        crate::fixtures::make_fifo(&root.join("exit-gate"));
        fs::write(root.join("config.toml"), "[bar]\nwidgets = []\n").unwrap();
        if pause {
            fs::write(root.join("pause"), b"").unwrap();
        }
        if attached {
            sessions::tests::script(
                &root.join("venus"),
                r#"#!/bin/sh
echo "$$" > "$EON_TEST_HANDOFF_ROOT/venus-pid"
if [ -n "${EON_TEST_FRESH_SENTINEL-}" ]; then
    "$EON_TEST_HANDOFF_ROOT/store/a/bin/eon" --check-fresh-descriptors 3>&1 >/dev/null || exit
fi
if [ "${EON_UPDATE_TEST_FAULT-}" = fresh-launch-display ]; then
    exec "$EON_TEST_HANDOFF_ROOT/store/a/bin/eon" --check-fresh-descriptors 3>&1 >/dev/null
fi
while IFS= read -r line; do
    if [ "$line" = present ]; then
        echo "old Venus after exec" >&2
    fi
done
"#,
            );
        }
        let output = File::create(root.join("transcript")).unwrap();
        let mut command = Command::new(bin(&root, "a").join("eon"));
        command
            .env("EON_TEST_HANDOFF_ROOT", &root)
            .env("EON_UPDATE_TEST_COMMIT", "1")
            .env("HOME", &root)
            .env("EON_RUNTIME_DIR", &root)
            .env("EON_CONFIG_HOME", &root)
            .env("XDG_STATE_HOME", root.join("state"))
            .env("TERMINFO_DIRS", "base")
            .env_remove("EON_TEST_A_ONLY")
            .env_remove(WINDOW_RUNTIME)
            .env_remove("EON_UPDATE_TEST_FAULT")
            .env_remove(SENTINEL)
            .env_remove(WAYLAND_SOCKET)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        if let Some(fault) = fault {
            command.env("EON_UPDATE_TEST_FAULT", fault);
            if let Some(hold) = fault.strip_prefix("held-") {
                crate::fixtures::make_fifo(&root.join("helper-gate"));
                let executable = match hold {
                    "zoxide" => "zoxide",
                    "codex" => "codexbar",
                    _ => "agy",
                };
                let helper_bin = bin(&root, "a");
                sessions::tests::script(
                    &helper_bin.join(executable),
                    &format!(
                        r#"#!/bin/sh
phase={hold}
case "$0" in
  */agy) if [ "$1" = --version ]; then phase=agy-version; else phase=agy-usage; fi ;;
esac
root="$EON_TEST_HANDOFF_ROOT"
echo "$phase" >> "$root/helper-calls"
if [ "$phase" = {hold} ]; then
    echo "$$" > "$root/helper-pid"
    IFS= read -r release < "$root/helper-gate"
fi
if [ "$phase" = agy-version ]; then echo 1.5.6; else echo '{{}}'; fi
"#
                    ),
                );
                command.env(
                    "PATH",
                    env::join_paths(
                        std::iter::once(helper_bin).chain(
                            env::var_os("PATH")
                                .as_ref()
                                .into_iter()
                                .flat_map(env::split_paths),
                        ),
                    )
                    .unwrap(),
                );
            }
        }
        let sentinel = (fault == Some("fresh-launch-fd")).then(|| {
            let file = OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(root.join("sentinel"))
                .unwrap();
            let fd = rustix::io::fcntl_dupfd_cloexec(&file, 131).unwrap();
            command.env(SENTINEL, fd.as_raw_fd().to_string());
            platform::inherit_for_child(&mut command, fd);
            file
        });
        if let Some(display) = display {
            command.env(WAYLAND_SOCKET, display.as_raw_fd().to_string());
            platform::inherit_for_child(&mut command, display);
        } else if fault == Some("fresh-launch-display") {
            command.env(WAYLAND_SOCKET, "-1");
        }
        let mut fixture = Self {
            root,
            child: command.spawn().unwrap(),
            sentinel,
        };
        until("source window", || {
            fixture.assert_running();
            fixture.path(update::SOCKET).exists()
        });
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        generation_directory(&self.root, LIFE).join(name)
    }

    fn transcript(&self) -> String {
        fs::read_to_string(self.root.join("transcript")).unwrap_or_default()
    }

    fn assert_running(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "{}",
            self.transcript()
        );
    }

    fn fresh_child(&mut self) -> String {
        let mut proof = String::new();
        until("initial presentation descriptor check", || {
            self.assert_running();
            proof = fs::read_to_string(self.root.join("fresh-child")).unwrap_or_default();
            !proof.is_empty()
        });
        proof
    }

    fn request(&self, id: u8) -> Preflight {
        Preflight {
            request: [id; 16],
            lifetime: LIFE.into(),
            expected_active_build: current_generation(&inputs(&self.root, "a")).unwrap(),
            target_wrapper: bin(&self.root, "b").join("eon").into_os_string().into_vec(),
            target_build: current_generation(&inputs(&self.root, "b")).unwrap(),
            caller: None,
        }
    }

    fn send(&self, bytes: &[u8]) -> UnixStream {
        let mut stream = UnixStream::connect(self.path(update::SOCKET)).unwrap_or_else(|error| {
            panic!(
                "cannot connect to Update fixture: {error}\n{}",
                self.transcript()
            )
        });
        stream.set_read_timeout(Some(START)).unwrap();
        stream.write_all(bytes).unwrap();
        stream
    }

    fn read(&self, mut stream: UnixStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .unwrap_or_else(|error| panic!("{error}: {}", self.transcript()));
        bytes
    }

    fn inspect(&self) -> Inspected {
        let request = Inspect { request: [99; 16] };
        Inspected::decode(&self.read(self.send(&request.encode().unwrap()))).unwrap()
    }

    fn preflight(&self, request: &Preflight) -> Preflighted {
        let deadline = Instant::now() + START;
        let request = request.encode().unwrap();
        loop {
            let bytes = self.read(self.send(&request));
            if let Ok(reply) = Preflighted::decode(&bytes) {
                return reply;
            }
            let refusal = Refused::decode(&bytes, eonu::Direction::Reply).unwrap();
            assert!(
                refusal.code == Code::Unsettled && Instant::now() < deadline,
                "preflight did not settle: {refusal:?}\n{}",
                self.transcript()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn commit(&self, request: &Preflight) -> UnixStream {
        self.send(&request.encode_commit().unwrap())
    }

    fn record(&self, terminal: usize) -> PathBuf {
        let endpoint =
            crate::workspace::terminal_endpoint(&generation_directory(&self.root, LIFE), terminal);
        endpoint.with_extension("sock.record")
    }

    fn live_host(&self, terminal: usize) -> LiveIdentity {
        let bytes = fs::read(self.record(terminal)).unwrap();
        let Record::Live(identity) = management::decode_record(&bytes).unwrap() else {
            panic!("terminal {terminal} was not live");
        };
        identity
    }

    fn pty(&self, terminal: usize) -> u32 {
        let path = self.root.join(format!("pty-{terminal}"));
        let mut result = None;
        until("PTY program start", || {
            result = fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse().ok());
            result.is_some()
        });
        result.unwrap()
    }

    fn stop(&mut self) {
        let answer = send_stop_on(connect_control(&self.path("eon.sock")).unwrap(), LIFE).unwrap();
        assert!(
            matches!(
                answer,
                StopAnswer::Reply(eon_workspace_protocol::stop::Reply::Stopped { .. })
            ),
            "{answer:?}"
        );
        self.wait();
    }

    fn wait(&mut self) {
        until("supervisor exit", || {
            self.child.try_wait().unwrap().is_some()
        });
        assert!(
            self.child.wait().unwrap().success(),
            "{}",
            self.transcript()
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _helper_gate = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(self.root.join("helper-gate"))
            .ok()
            .map(|mut gate| {
                let _ = gate.write_all(b"go\n");
                gate
            });
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        if let Ok(mut runs) = sessions::acquire_generation_sessions(
            self.path("eon.sock").parent().unwrap(),
            LaunchMode::Workspace,
            LIFE,
            &[],
            None,
            deadline,
        ) {
            let _ = sessions::stop_managed_sessions(
                &mut runs,
                deadline.saturating_duration_since(Instant::now()),
            );
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn update_isolates_descriptors_at_fresh_launch() {
    let mut fixture = Fixture::start(Some("fresh-launch-fd"), false, true);
    let child = fixture.fresh_child();
    let before = fixture.inspect();
    let request = fixture.request(11);
    fixture.preflight(&request);
    let reply = fixture.read(fixture.commit(&request));
    assert!(
        reply.is_empty(),
        "{:?}\n{}",
        Refused::decode(&reply, eonu::Direction::Reply),
        fixture.transcript()
    );
    let after = fixture.inspect();
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.active_build, request.target_build);
    let outcome = after.last.unwrap();
    assert_eq!(outcome.request, request.request);
    assert_eq!(outcome.operation, Operation::Commit);
    assert_eq!(outcome.refusal, None);
    assert!(fixture.root.join("verified-child").exists());
    assert_eq!(child, "isolated");
    let sentinel = fixture.sentinel.as_ref().unwrap();
    assert_eq!(
        rustix::io::fcntl_getfd(sentinel).unwrap(),
        rustix::io::FdFlags::CLOEXEC
    );
    assert!(
        rustix::fs::fcntl_getfl(sentinel)
            .unwrap()
            .contains(rustix::fs::OFlags::APPEND)
    );
    fixture.stop();
}

#[test]
#[cfg(target_os = "linux")]
fn fresh_launch_passes_the_display_only_to_initial_venus() {
    use rustix::net::{AddressFamily, SocketType, socket};
    let (mut compositor, display) = UnixStream::pair().unwrap();
    let (datagram, _peer) = std::os::unix::net::UnixDatagram::pair().unwrap();
    let high = |fd: OwnedFd| Some(rustix::io::fcntl_dupfd_cloexec(fd, 131).unwrap());
    // Only an inherited, connected Unix stream is a display; Venus gets nothing else.
    for (display, expected) in [
        (high(display.into()), "display"),
        (high(File::open("/dev/null").unwrap().into()), "isolated"),
        (
            high(socket(AddressFamily::UNIX, SocketType::STREAM, None).unwrap()),
            "isolated",
        ),
        (high(datagram.into()), "isolated"),
        (None, "isolated"),
    ] {
        let mut fixture =
            Fixture::start_with_display(Some("fresh-launch-display"), false, true, display);
        assert_eq!(fixture.fresh_child(), expected);
        if expected == "display" {
            compositor.set_read_timeout(Some(START)).unwrap();
            let mut received = [0; 5];
            compositor.read_exact(&mut received).unwrap();
            assert_eq!(&received, b"venus");
            assert_eq!(compositor.read(&mut received).unwrap(), 0);
            fixture.assert_running();
        }
        until("terminal display environment", || {
            fs::read_to_string(fixture.root.join("orbit-display"))
                .is_ok_and(|value| value == "unset\n")
        });
        fixture.stop();
    }
}

#[test]
fn update_keeps_exact_resources_and_reaps_the_exiting_host() {
    let mut fixture = Fixture::start(None, true, false);
    let a = fixture.inspect();
    let live = fixture.live_host(1);
    let ending = fixture.live_host(2);
    let program = fixture.pty(1);
    let pid = fixture.child.id();
    let pid_text = pid.to_string();
    let start = platform::process(pid).unwrap().unwrap().start;
    let sockets =
        ["eon.sock", update::SOCKET].map(|name| socket_identity(&fixture.path(name)).unwrap());
    let mut same = fixture.request(10);
    same.target_build.clone_from(&same.expected_active_build);
    same.target_wrapper = bin(&fixture.root, "a")
        .join("eon")
        .into_os_string()
        .into_vec();
    let refusal =
        Refused::decode(&fixture.read(fixture.commit(&same)), eonu::Direction::Reply).unwrap();
    assert_eq!(refusal.code, Code::TargetRefused);
    assert_eq!(fixture.inspect().active_build, a.active_build);

    let mut request = fixture.request(1);
    request.caller = Some(sessions::terminal_identity(&live.session_id, &live.run_id));
    assert_eq!(
        fixture.preflight(&request).caller_origin,
        Some(a.active_build.clone())
    );
    request.request = [2; 16];
    let reply = fixture.commit(&request);
    until("target exec", || {
        fs::read_to_string(fixture.root.join("entered-b"))
            .ok()
            .as_deref()
            == Some(pid_text.as_str())
    });
    assert!(
        crate::supervisor::try_lock_supervisor_lifecycle(&supervisor_lock_path(
            &fixture.root,
            LIFE
        ))
        .unwrap()
        .is_none()
    );
    assert!(NamespaceLock::try_acquire(&fixture.root).unwrap().is_none());
    let mut gate = None;
    until("terminal opened its test gate", || {
        gate = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(fixture.root.join("exit-gate"))
            .ok();
        gate.is_some()
    });
    let mut gate = gate.unwrap();
    gate.write_all(b"end\n").unwrap();
    until("terminal's final record", || {
        fs::read(fixture.record(2)).ok().is_some_and(|bytes| {
            matches!(management::decode_record(&bytes), Ok(Record::Tombstone(_)))
        })
    });
    fs::write(fixture.root.join("release"), b"").unwrap();
    let bytes = fixture.read(reply);
    let updated = Updated::decode(&bytes).unwrap_or_else(|_| {
        panic!(
            "{:?}\n{}",
            Refused::decode(&bytes, eonu::Direction::Reply),
            fixture.transcript()
        )
    });
    assert_eq!(updated.request, request.request);
    assert!(updated.changed);
    assert_eq!(updated.to_build, request.target_build);
    assert_eq!(updated.presentation, UpdatedPresentation::Detached);
    let b = fixture.inspect();
    assert_eq!(b.pid, a.pid);
    assert_eq!(b.created_build, a.created_build);
    assert_eq!(b.active_build, request.target_build);
    assert_eq!(b.last.unwrap().request, request.request);
    assert_eq!(platform::process(pid).unwrap().unwrap().start, start);
    assert_eq!(fixture.live_host(1), live);
    assert_eq!(
        sockets,
        ["eon.sock", update::SOCKET].map(|name| socket_identity(&fixture.path(name)).unwrap())
    );
    assert!(fixture.root.join("verified-child").exists());
    assert!(NamespaceLock::try_acquire(&fixture.root).unwrap().is_some());
    until("exact host reap", || {
        platform::process(ending.process_id).unwrap().is_none()
    });
    assert!(platform::process(program).unwrap().is_some());
    fixture.stop();
    until("remaining host and PTY program stop", || {
        platform::process(live.process_id).unwrap().is_none()
            && platform::process(program).unwrap().is_none()
    });
}

#[test]
fn update_lost_reply_preserves_the_old_venus_control_and_stderr() {
    let mut fixture = Fixture::start(None, true, true);
    let a = fixture.inspect();
    let request = fixture.request(3);
    fixture.preflight(&request);
    let reply = fixture.commit(&request);
    until("target exec", || fixture.root.join("entered-b").exists());
    drop(reply);
    fs::write(fixture.root.join("release"), b"").unwrap();
    until("old Venus stderr", || {
        let transcript = fixture.transcript();
        assert!(fixture.child.try_wait().unwrap().is_none(), "{transcript}");
        transcript.contains("old Venus after exec")
    });
    let b = fixture.inspect();
    assert_eq!(b.pid, a.pid);
    assert_eq!(b.presentation, eonu::Presentation::Attached);
    assert_eq!(b.active_build, request.target_build);
    let last = b.last.unwrap();
    assert_eq!(last.request, request.request);
    assert_eq!(last.operation, Operation::Commit);
    assert_eq!(last.refusal, None);
    assert!(
        last.detail
            .contains("target presentation admission is pending")
    );
    let verdict = Refused::decode(
        &fixture.read(fixture.commit(&request)),
        eonu::Direction::Reply,
    )
    .unwrap();
    assert_eq!(verdict.code, Code::StaleBuild);
    assert_eq!(fixture.inspect().last.unwrap().request, request.request);
    let venus: u32 = fs::read_to_string(fixture.root.join("venus-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    fixture.stop();
    assert!(platform::process(venus).unwrap().is_none());
}

#[test]
fn update_does_not_send_a_second_reply_or_report_attached_admission_early() {
    for (fault, attached) in [(Some("reply-sent"), false), (None, true)] {
        let mut fixture = Fixture::start(fault, false, attached);
        let a = fixture.inspect();
        let request = fixture.request(5);
        fixture.preflight(&request);
        assert!(fixture.read(fixture.commit(&request)).is_empty());
        let b = fixture.inspect();
        assert_eq!(b.pid, a.pid);
        assert_eq!(b.active_build, request.target_build);
        let last = b.last.unwrap();
        assert_eq!(last.request, request.request);
        assert_eq!(last.operation, Operation::Commit);
        assert_eq!(last.refusal, None);
        assert_eq!(
            last.detail
                .contains("target presentation admission is pending"),
            attached
        );
        fixture.stop();
    }
}

#[test]
fn update_refuses_before_or_after_exec_without_destroying_work() {
    for (fault, executed, attached) in [
        ("exec-return", false, false),
        ("host-version", false, false),
        ("malformed-import", true, false),
        ("restore", true, false),
        ("venus-identity", true, true),
    ] {
        let mut fixture = Fixture::start(Some(fault), false, attached);
        let a = fixture.inspect();
        let host = fixture.live_host(1);
        let program = fixture.pty(1);
        let program_start = platform::process(program).unwrap().unwrap().start;
        let identity = fs::read(fixture.path("identity")).unwrap();
        let sockets =
            ["eon.sock", update::SOCKET].map(|name| socket_identity(&fixture.path(name)).unwrap());
        let request = fixture.request(4);
        if fault != "host-version" {
            fixture.preflight(&request);
        }
        let response = fixture.read(fixture.commit(&request));
        if fault == "host-version" || fault == "exec-return" || fault == "restore" {
            let refusal = Refused::decode(&response, eonu::Direction::Reply)
                .unwrap_or_else(|error| panic!("{fault}: {error} {}", fixture.transcript()));
            let expected = if fault == "host-version" {
                Code::HostProtocol
            } else {
                Code::Internal
            };
            assert_eq!(refusal.code, expected, "{refusal:?}");
        } else {
            assert!(response.is_empty(), "{fault}");
        }
        assert_eq!(fixture.root.join("entered-b").exists(), executed, "{fault}");
        assert_eq!(fs::read(fixture.path("identity")).unwrap(), identity);
        assert_eq!(
            sockets,
            ["eon.sock", update::SOCKET].map(|name| socket_identity(&fixture.path(name)).unwrap())
        );
        assert_eq!(fixture.live_host(1), host);
        assert!(platform::process(program).unwrap().is_some(), "{fault}");
        if executed {
            fixture.wait();
            assert!(
                fs::read_to_string(fixture.root.join("result"))
                    .unwrap()
                    .contains("Err("),
                "{fault}"
            );
            assert_eq!(fixture.live_host(1), host);
            for (pid, start) in [
                (host.process_id, host.process_start),
                (program, program_start),
            ] {
                assert!(
                    platform::process(pid)
                        .unwrap()
                        .is_some_and(|p| p.start == start && !p.ended),
                    "{fault}: process {pid} did not survive the supervisor"
                );
            }
        } else {
            let still_a = fixture.inspect();
            assert_eq!(still_a.active_build, a.active_build);
            if fault == "exec-return" {
                assert!(NamespaceLock::try_acquire(&fixture.root).unwrap().is_some());
                assert!(fixture.root.join("verified-rollback-child").exists());
                let refusal = Refused::decode(&response, eonu::Direction::Reply).unwrap();
                let last = still_a.last.unwrap();
                assert_eq!(last.request, request.request);
                assert_eq!(last.operation, Operation::Commit);
                assert_eq!(last.refusal, Some(refusal.code));
                assert_eq!(last.detail, refusal.detail);
                assert!(last.detail.contains("cannot replace the supervisor"));
            }
            fixture.stop();
        }
    }
}

#[test]
fn update_refuses_a_foreign_inheritable_descriptor_without_changing_its_flags() {
    let mut fixture = Fixture::start(Some("foreign-fd"), false, false);
    let a = fixture.inspect();
    let request = fixture.request(9);
    let response = fixture.read(fixture.commit(&request));
    let refusal = Refused::decode(&response, eonu::Direction::Reply)
        .unwrap_or_else(|error| panic!("{error}: {}", fixture.transcript()));
    assert!(
        refusal.detail.contains("unexpected inheritable descriptor"),
        "{refusal:?}"
    );
    let after = fixture.inspect();
    assert_eq!(after.active_build, a.active_build);
    assert_eq!(after.last.unwrap().refusal, Some(refusal.code));
    fixture.stop();
    assert!(fixture.root.join("foreign-intact").exists());
}

#[test]
fn update_waits_for_owned_helpers_then_keeps_the_same_child_on_refusal() {
    for (held, calls) in [
        ("held-zoxide", "zoxide\n"),
        ("held-codex", "codex\n"),
        ("held-agy-version", "agy-version\nagy-usage\n"),
        ("held-agy-usage", "agy-version\nagy-usage\n"),
    ] {
        let mut fixture = Fixture::start(Some(held), false, false);
        let a = fixture.inspect();
        let mut helper = None;
        until("held helper identity", || {
            helper = fs::read_to_string(fixture.root.join("helper-pid"))
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
                .and_then(|pid| platform::process(pid).unwrap().map(|child| (pid, child)));
            helper.is_some()
        });
        let (pid, child) = helper.unwrap();
        assert_eq!(child.parent, a.pid);
        let started = Instant::now();
        let request = fixture.request(7);
        let bytes = fixture.read(fixture.commit(&request));
        let elapsed = started.elapsed();
        let refusal = Refused::decode(&bytes, eonu::Direction::Reply)
            .unwrap_or_else(|error| panic!("{held}: {error}: {}", fixture.transcript()));
        assert_eq!(refusal.code, Code::Unsettled, "{held}: {refusal:?}");
        assert!(
            refusal.detail.contains(if held == "held-zoxide" {
                "Zoxide"
            } else {
                "quota"
            }),
            "{refusal:?}"
        );
        assert!(
            (Duration::from_secs(1)..Duration::from_secs(8)).contains(&elapsed),
            "{held}: {elapsed:?}"
        );
        assert!(
            platform::process(pid)
                .unwrap()
                .is_some_and(|p| p.start == child.start && !p.ended)
        );
        let still_a = fixture.inspect();
        assert_eq!(still_a.active_build, a.active_build);
        assert_eq!(still_a.last.unwrap().refusal, Some(Code::Unsettled));
        until("helper waiting on its gate", || {
            OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fixture.root.join("helper-gate"))
                .and_then(|mut writer| writer.write_all(b"go\n"))
                .is_ok()
        });
        until("exact helper reaped", || {
            platform::process(pid)
                .unwrap()
                .is_none_or(|p| p.start != child.start)
        });
        let request = fixture.request(8);
        let commit = fixture.read(fixture.commit(&request));
        assert!(
            Updated::decode(&commit).is_ok(),
            "{held}: {:?}\n{}",
            Refused::decode(&commit, eonu::Direction::Reply),
            fixture.transcript()
        );
        assert_eq!(fixture.inspect().pid, a.pid);
        assert_eq!(
            fs::read_to_string(fixture.root.join("helper-calls")).unwrap(),
            calls
        );
        fixture.stop();
    }
}

#[test]
fn update_cannot_audit_while_an_observer_is_between_socket_and_cloexec() {
    let mut fixture = Fixture::start(Some("observer-cloexec"), false, true);
    let a = fixture.inspect();
    until("observer in socket creation", || {
        fixture.root.join("observer-in-connect").exists()
    });
    let request = fixture.request(11);
    let bytes = fixture.read(fixture.commit(&request));
    let refusal = Refused::decode(&bytes, eonu::Direction::Reply)
        .unwrap_or_else(|error| panic!("{error}: {}", fixture.transcript()));
    assert_eq!(refusal.code, Code::Unsettled, "{refusal:?}");
    assert!(refusal.detail.contains("pane observer"), "{refusal:?}");
    assert_eq!(fixture.inspect().active_build, a.active_build);
    fs::write(fixture.root.join("observer-release"), b"").unwrap();
    let request = fixture.request(12);
    assert!(fixture.read(fixture.commit(&request)).is_empty());
    let b = fixture.inspect();
    assert_eq!(b.pid, a.pid);
    assert_eq!(b.active_build, request.target_build);
    assert_eq!(b.presentation, eonu::Presentation::Attached);
    let last = b.last.unwrap();
    assert_eq!(last.request, request.request);
    assert_eq!(last.refusal, None);
    assert!(fixture.root.join("verified-child").exists());
    fixture.stop();
}

/// C02 and C18 against real Orbit leases and the window's own listener. A
/// status request that Orbit leaves unanswered past the 1,000 ms lease drain
/// refuses Commit before exec; a control client that connected while the old
/// supervisor held that barrier stays queued and the old supervisor serves it
/// afterwards. In the next Commit Orbit answers that same request inside the
/// drain, and the window adopts. Clients queued during that barrier pass to
/// the replacement, which serves a compatible request once and refuses one of
/// another EONW before dispatch. The Stop after it is the window's one Stop.
#[test]
fn update_drains_a_held_lease_and_leaves_queued_clients_to_their_owner() {
    use eon_workspace_protocol::v51::{
        Action, Request, Response, VERSION, WorkspaceAction, decode_response, encode_request,
    };
    use rustix::process::{Pid, Signal, kill_process};
    // Resumes the stopped Orbit however the test ends.
    struct Stopped(Pid);
    impl Drop for Stopped {
        fn drop(&mut self) {
            let _ = kill_process(self.0, Signal::CONT);
        }
    }
    let mut fixture = Fixture::start(Some("held-status"), false, false);
    let a = fixture.inspect();
    let host = fixture.live_host(1);
    let action = |id: &str, action| {
        encode_request(&Request {
            id: id.into(),
            caller: None,
            action: Action::Workspace(action),
        })
        .unwrap()
    };
    let rename = |id: &str| {
        action(
            id,
            WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: id.into(),
            },
        )
    };
    let send = |bytes: &[u8]| {
        let mut stream = UnixStream::connect(fixture.path("eon.sock")).unwrap();
        stream.set_read_timeout(Some(START)).unwrap();
        stream.write_all(bytes).unwrap();
        stream
    };
    let snapshot = |stream| match decode_response(&fixture.read(stream)).unwrap() {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("{other:?}\n{}", fixture.transcript()),
    };
    let names = |snapshot: &eon_workspace_protocol::v51::Snapshot| {
        snapshot
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.name.clone())
            .collect::<Vec<_>>()
    };
    let quiescing = fixture.root.join("quiescing");
    let in_barrier = || {
        until("Commit's barrier", || quiescing.exists());
        fs::remove_file(&quiescing).unwrap();
    };
    let orbit = Pid::from_raw(host.process_id as i32).unwrap();
    kill_process(orbit, Signal::STOP).unwrap();
    let stopped = Stopped(orbit);
    // The window polls every 500 ms, so its next status request waits.
    thread::sleep(Duration::from_secs(2));
    let started = Instant::now();
    let refused = fixture.commit(&fixture.request(13));
    in_barrier();
    let held = send(&rename("held"));
    let bytes = fixture.read(refused);
    let elapsed = started.elapsed();
    let refusal = Refused::decode(&bytes, eonu::Direction::Reply)
        .unwrap_or_else(|error| panic!("{error}: {}", fixture.transcript()));
    assert_eq!(refusal.code, Code::Unsettled, "{refusal:?}");
    assert!(
        refusal.detail.contains("a terminal request is in flight"),
        "{refusal:?}"
    );
    assert!(
        (Duration::from_secs(1)..Duration::from_secs(8)).contains(&elapsed),
        "{elapsed:?}"
    );
    let served = snapshot(held);
    assert_eq!(served.active_build, a.active_build);
    assert_eq!(names(&served), ["held"]);
    let still_a = fixture.inspect();
    assert_eq!(still_a.active_build, a.active_build);
    assert_eq!(still_a.last.unwrap().refusal, Some(Code::Unsettled));
    assert_eq!(fixture.live_host(1), host);

    // The same request is still in flight.
    let request = fixture.request(14);
    let pending = fixture.commit(&request);
    in_barrier();
    let mut other = rename("other");
    other[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
    let queued = [send(&rename("queued")), send(&other)];
    drop(stopped);
    let commit = fixture.read(pending);
    assert!(
        Updated::decode(&commit).is_ok(),
        "{:?}\n{}",
        Refused::decode(&commit, eonu::Direction::Reply),
        fixture.transcript()
    );
    let [compatible, incompatible] = queued;
    let renamed = snapshot(compatible);
    assert_eq!(renamed.active_build, request.target_build);
    assert_eq!(names(&renamed), ["queued"]);
    let Response::Failure(other) = decode_response(&fixture.read(incompatible)).unwrap() else {
        panic!("another EONW's request was served");
    };
    assert_eq!(other.code, "unsupported-version", "{other:?}");
    let after = snapshot(send(&action("after", WorkspaceAction::Inspect)));
    assert_eq!(names(&after), ["queued"]);
    let b = fixture.inspect();
    assert_eq!(b.pid, a.pid);
    assert_eq!(b.active_build, request.target_build);
    assert_eq!(fixture.live_host(1), host);
    fixture.stop();
}
