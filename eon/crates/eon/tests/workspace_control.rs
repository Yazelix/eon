use eon_workspace_protocol::stop;
use eon_workspace_protocol::v39::{
    Action as ProtocolAction, Availability, Change, Direction, End, Failure as ProtocolFailure,
    Filter, HEADER_BYTES, InvokeIntent, LifecycleResponse, MAX_SESSIONS, Popup, PopupTarget,
    Request, Response, Runtime, Screen, Sent, Snapshot, SubscriptionMessage, Tab, TabChange,
    TargetId, TerminalHistory, TerminalInput, TerminalTarget, TerminalText, VERSION,
    WorkspaceAction as Action, declared_message_len, decode_lifecycle_response, decode_request,
    decode_response, decode_subscription_message, encode_lifecycle_response, encode_request,
    encode_response,
};
use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage, EndpointIdentity, Failure,
    FailureCode, Foreground, HistoryText, LeaseInput, LiveIdentity, ObjectIdentity, ProcessOutcome,
    ProgramStatus, Record as ManagementRecord, Refusal, Screen as OrbitScreen,
    ServerMessage as ManagementServerMessage, TerminationReason, Tombstone, VisibleText,
};
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::fd::AsRawFd,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

mod process;

static NEXT_TEST: AtomicU64 = AtomicU64::new(0);
static CODEX_STUB: OnceLock<PathBuf> = OnceLock::new();

struct TestProcess {
    child: Child,
    stop: PathBuf,
}

impl TestProcess {
    fn wait_with_output(mut self) -> std::io::Result<Output> {
        drop(self.child.stdin.take());
        thread::scope(move |scope| {
            let stdout = self.child.stdout.take().map(|mut pipe| {
                scope.spawn(move || {
                    let mut bytes = Vec::new();
                    pipe.read_to_end(&mut bytes).map(|_| bytes)
                })
            });
            let mut stderr = Vec::new();
            if let Some(mut pipe) = self.child.stderr.take() {
                pipe.read_to_end(&mut stderr)?;
            }
            let status = self.child.wait()?;
            let stdout = stdout.map_or_else(|| Ok(Vec::new()), |reader| reader.join().unwrap())?;
            // A completed CLI must not stop its supervisor when the guard drops.
            self.stop.clear();
            Ok(Output {
                status,
                stdout,
                stderr,
            })
        })
    }
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
        match self.0.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => return unwound(self.0.id(), status),
            // Its PID may no longer be its own, so it is not signalled.
            Err(error) => {
                if !thread::panicking() {
                    panic!("{error}");
                }
                return;
            }
        }
        // SAFETY: this still-unreaped child was spawned as its own group leader.
        let result = unsafe { libc::kill(-i32::try_from(self.0.id()).unwrap(), libc::SIGKILL) };
        let error = std::io::Error::last_os_error();
        // A second panic while unwinding aborts the test binary, and then no other
        // test's guard stops its supervisor.
        if thread::panicking() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                match self.0.try_wait() {
                    Ok(Some(status)) => return unwound(self.0.id(), status),
                    Ok(None) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => return,
                }
            }
            return;
        }
        // macOS reports EPERM, not ESRCH, for a group with no member left to signal.
        assert!(
            result == 0
                || error.raw_os_error() == Some(libc::ESRCH)
                || cfg!(target_vendor = "apple") && error.raw_os_error() == Some(libc::EPERM),
            "{error}"
        );
        wait_for_exit(&mut self.0);
    }
}

/// How each child that a guard reaped while its test was failing ended, by PID.
static UNWOUND: Mutex<Vec<(u32, ExitStatus)>> = Mutex::new(Vec::new());

fn unwound(pid: u32, status: ExitStatus) {
    if thread::panicking() {
        UNWOUND
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((pid, status));
    }
}

/// A child on private terminals. A macOS session leader drains its controlling
/// terminal's output, such as echoed input, before it can exit, so the masters
/// close first (fields drop in order) and only then does the guard reap it.
struct TerminalChild {
    terminals: Vec<fs::File>,
    guard: ManagedChild,
}

impl TerminalChild {
    fn spawn(command: &mut Command, terminals: Vec<fs::File>) -> Self {
        let guard = ManagedChild(command.spawn().unwrap());
        Self { terminals, guard }
    }

    /// Waits until `deadline` for the child, reading its terminals meanwhile.
    fn exit(&mut self, deadline: Instant) -> Option<ExitStatus> {
        exit_reading_terminals(&mut self.guard.0, &mut self.terminals, deadline)
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
        vec!["config", "paths"],
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

/// Fails when `text`, which a person or agent reads, says "workspace" outside
/// the names Eon does not own: Eon calls them sessions.
fn assert_names_no_workspace(text: &str, context: &str) {
    const ALLOWED: &[&str] = &[
        // Codex's sandbox setting.
        "sandbox_workspace_write",
        // `eon versions` names the source of in-tree components.
        "report `workspace`, the Cargo workspace they are built in.",
    ];
    let text = ALLOWED
        .iter()
        .fold(text.to_owned(), |text, allowed| text.replace(allowed, ""));
    if let Some(line) = text
        .lines()
        .find(|line| line.to_lowercase().contains("workspace"))
    {
        panic!("{context} says workspace: {line:?}");
    }
}

#[test]
fn help_skill_and_usage_name_sessions_and_old_commands_are_invalid() {
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    for arguments in [&["--help"][..], &["session", "--help"]] {
        let output = eon_command(binary).args(arguments).output().unwrap();
        assert!(output.status.success(), "{arguments:?}");
        assert_names_no_workspace(stdout(&output), &format!("eon {arguments:?}"));
    }
    assert_names_no_workspace(include_str!("../../../skills/eon/SKILL.md"), "the skill");
    assert_names_no_workspace(include_str!("../../../../docs/USAGE.md"), "USAGE.md");
    // A removed command names its replacement and still fails.
    for (arguments, hint) in [
        (
            &["workspace"][..],
            Some("`eon workspace` is now `eon session`"),
        ),
        (
            &["workspace", "--json"],
            Some("`eon workspace` is now `eon session`"),
        ),
        (
            &["workspaces", "--json"],
            Some("`eon workspaces` is now `eon session ls`"),
        ),
        (
            &["workspace", "new", "--json"],
            Some("`eon workspace` is now `eon session`"),
        ),
        (&["session", "ls", "extra", "--json"], None),
        // `--json` follows the command word, as for every command.
        (&["--json", "session", "ls"], None),
    ] {
        let output = eon_command(binary).args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            hint.is_some_and(|hint| error.contains(hint)),
            hint.is_some(),
            "{error}"
        );
        assert!(error.contains("eon --help"), "{error}");
    }
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
        assert!(help.contains("terminals") && !help.to_lowercase().contains("workspace"));
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
            "quit [--all] [--json]",
            "window ls [--json]",
            "window stop ID|previous|all [--json]",
            "window attach [ID]",
            "tab directory TAB",
            "config path",
            "config dir",
            "-V, --version",
            "eon run --",
        ] {
            assert!(help.contains(command), "missing {command}");
        }
        assert!(!help.contains("--file"));
        assert!(!help.contains("generations"));
        // `attach`, `stop` and `--generation` are gone from eon (EON-C11).
        for removed in [
            "GENERATION",
            "--generation",
            "Attach & stop",
            "\n  attach",
            "\n  stop",
        ] {
            assert!(!help.contains(removed), "help still names {removed:?}");
        }
        for note in [
            "Window IDs, as eon window ls shows them: default, another build's or an\n\
             independent window's ID, or any unique prefix of 4+ characters.\n",
            "Window flag: --window ID, before any --.\n",
        ] {
            assert!(help.contains(note), "missing {note:?}");
        }
        // An entry's description may continue on the next line.
        let lines: Vec<&str> = help.lines().collect();
        for command in ["config path ", "config defaults "] {
            let entry = lines
                .iter()
                .position(|line| line.trim_start().starts_with(command))
                .unwrap_or_else(|| panic!("missing {command}"));
            let description = lines[entry].to_owned()
                + lines
                    .get(entry + 1)
                    .filter(|line| line.starts_with("    "))
                    .unwrap_or(&"");
            assert!(
                description.contains("config.toml"),
                "{command} must name config.toml"
            );
        }
        assert!(!runtime.exists());
        assert!(!absent_config.exists());
        assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    }
    // A family's or command's help is its rows of `eon --help`, continuations included.
    let full = stdout(&eon_command(binary).arg("--help").output().unwrap()).to_owned();
    for (args, present, absent) in [
        (
            vec!["pane", "--help"],
            &["pane create", "pane send PANE|POPUP", "[--stdin | -- TEXT]"][..],
            &["tab create", "edit [--json]"][..],
        ),
        (
            vec!["pane", "send", "--help"],
            &["pane send PANE|POPUP", "[--stdin | -- TEXT]"],
            &["pane create", "pane read"],
        ),
        (
            vec!["pane", "send", "-h"],
            &["pane send PANE|POPUP"],
            &["pane read"],
        ),
        (
            vec!["quit", "--help"],
            &["quit [--all] [--json]"],
            &["window new"],
        ),
        (
            vec!["window", "ls", "--help"],
            &["window ls [--json]", "compatible or not"],
            &["window new"],
        ),
    ] {
        let output = eon_command(binary)
            .args(&args)
            .env("EON_CONFIG_HOME", &absent_config)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env("EON_ORBIT", root.join("missing-orbit"))
            .env("EON_VENUS", root.join("missing-venus"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let help = stdout(&output);
        assert!(
            !help.contains('\u{1b}'),
            "{args:?}: piped help must be plain text"
        );
        for row in present {
            assert!(help.contains(row), "{args:?} is missing {row}");
        }
        for row in absent {
            assert!(!help.contains(row), "{args:?} must not list {row}");
        }
        for line in help.lines().filter(|line| line.starts_with("  ")) {
            assert!(
                full.lines().any(|full| full == line),
                "{line:?} is not in eon --help"
            );
        }
        assert!(!runtime.exists());
        assert!(!absent_config.exists());
    }
    for args in [
        vec!["unknown"],
        vec!["-h", "extra"],
        vec!["tab", "close"],
        vec!["unknown", "--help"],
        vec!["pane", "unknown", "--help"],
    ] {
        let output = eon_command(binary)
            .args(&args)
            .env("EON_CONFIG_HOME", &absent_config)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("eon --help"));
        assert!(error.len() < 200);
        assert!(!runtime.exists());
        assert!(!root.join("absent-config").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

/// Catches an alias or fallback for a removed command creeping back: the
/// window listings, `attach`, `stop` and the `--generation` flag (EON-C23,
/// EON-C11), with the delegation marker set too, and `config-path` and
/// `config-dir` (EON-C9).
#[test]
fn removed_commands_are_invalid() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let generation = format!("g1-{:032x}", 7);
    for args in [
        vec!["windows"],
        vec!["windows", "--json"],
        vec!["generations"],
        vec!["generations", "--json"],
        vec!["windows", "--help"],
        vec!["generations", "--help"],
        vec!["attach"],
        vec!["attach", &generation],
        vec!["attach", "--help"],
        vec!["stop", &generation],
        vec!["stop", &generation, "--json"],
        vec!["stop", "--json", &generation],
        vec!["stop", "previous"],
        vec!["stop", "all", "--json"],
        vec!["stop", "--help"],
        vec!["--generation", &generation, "session"],
        vec!["session", "--generation", &generation, "--json"],
        vec!["config-path"],
        vec!["config-dir"],
    ] {
        let output = eon_command(binary)
            .args(&args)
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid command or arguments"),
            "{args:?}"
        );
        assert!(!runtime.exists(), "{args:?}");
    }
    // Against a running window, none sends a Stop, even run as a delegated
    // attach of that generation: the window's supervisor still answers the
    // one Stop sent after them.
    let server = previous_supervisor(&runtime, &generation);
    let run = |args: &[&str], marker: Option<&str>| {
        let mut command = eon_command(binary);
        command
            .args(args)
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_ATTACH_DELEGATED")
            .stdin(Stdio::null());
        if let Some(marker) = marker {
            command.env(marker, &generation);
        }
        command.output().unwrap()
    };
    for marker in [None, Some("EON_ATTACH_DELEGATED")] {
        for args in [
            vec!["stop", generation.as_str(), "--json"],
            vec!["stop", generation.as_str()],
            vec!["stop", "all", "--json"],
            vec!["attach", generation.as_str()],
        ] {
            let output = run(&args, marker);
            assert_eq!(output.status.code(), Some(2), "{args:?} {marker:?}");
            assert!(output.stdout.is_empty(), "{args:?} {marker:?}");
        }
    }
    let stopped = run(&["window", "stop", &generation[3..], "--json"], None);
    assert!(stopped.status.success(), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"], generation,
        "{stopped}"
    );
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// Catches a delegated attach reaching another generation of its window, or
/// a `--window` prefix that two windows share reaching either: a marker that
/// names no generation of the window, like an ambiguous prefix, sends
/// nothing, and a window that runs two generations is stopped whole
/// (EON-C11, EON-C10).
#[test]
fn delegated_window_attach_reaches_only_the_marked_generation() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let windows = runtime.join("w");
    let window = windows.join("abcd3000");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let run = |args: &[&str], marker: Option<(&str, &str)>| {
        let mut command = eon_command(&binary);
        command
            .args(args)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .env_remove("EON_ATTACH_DELEGATED")
            .stdin(Stdio::null());
        if let Some((marker, value)) = marker {
            command.env(marker, value);
        }
        command.output().unwrap()
    };
    // The independent window runs another build's generation and this
    // build's own, the only one a marker may name.
    let current = stdout(&run(&["versions"], None))
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();
    let first = format!("g1-{:032x}", 3);
    let other = format!("g1-abcd1{:027x}", 1);
    let mut servers = vec![previous_supervisor(&runtime, &other)];
    fs::create_dir_all(&windows).unwrap();
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    servers.push(previous_supervisor(&window, &first));
    servers.push(previous_supervisor(&window, &current));
    let listed = || {
        let listed = run(&["window", "ls", "--json"], None);
        let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
        let mut generations: Vec<_> = listed["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|window| window["generation"].as_str().unwrap().to_owned())
            .collect();
        generations.sort();
        generations
    };
    let mut all = vec![other.clone(), first.clone(), current.clone()];
    all.sort();
    assert_eq!(listed(), all);

    // An ambiguous prefix sends nothing.
    let ambiguous = run(&["session", "--window", "abcd", "--json"], None);
    assert_eq!(ambiguous.status.code(), Some(3), "{ambiguous:?}");
    let failure: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "ambiguous-window-id", "{failure}");
    // A marker that is not this build's generation, or a window that does not
    // run it, is refused: each server would stop on its own Stop and fail on
    // a Present, so the listing below and the joins prove nothing was sent.
    let other_hex = &other[3..];
    for (args, marker, value) in [
        (
            ["window", "attach", "abcd3000"],
            "EON_ATTACH_DELEGATED",
            "not-a-generation",
        ),
        (["window", "attach", "abcd3000"], "EON_ATTACH_DELEGATED", ""),
        (
            ["window", "attach", "abcd3000"],
            "EON_ATTACH_DELEGATED",
            "g1-bad",
        ),
        (
            ["window", "attach", "abcd3000"],
            "EON_ATTACH_DELEGATED",
            first.as_str(),
        ),
        (
            ["window", "attach", other_hex],
            "EON_ATTACH_DELEGATED",
            other.as_str(),
        ),
        (
            ["window", "attach", other_hex],
            "EON_ATTACH_DELEGATED",
            current.as_str(),
        ),
    ] {
        let refused = run(&args, Some((marker, value)));
        assert_eq!(
            refused.status.code(),
            Some(2),
            "{args:?} {value}: {refused:?}"
        );
        assert!(refused.stdout.is_empty(), "{refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("nothing was sent"),
            "{refused:?}"
        );
    }
    assert_eq!(listed(), all);

    // The window's Stop covers every generation it runs.
    let stopped = run(&["window", "stop", "abcd3000", "--json"], None);
    assert!(stopped.status.success(), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    let mut generations: Vec<_> = stopped[0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["stopped"]["generation"].as_str().unwrap().to_owned())
        .collect();
    generations.sort();
    let mut both = vec![first.clone(), current.clone()];
    both.sort();
    assert_eq!(generations, both, "{stopped}");
    let stopped = run(&["window", "stop", other_hex, "--json"], None);
    assert!(stopped.status.success(), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"], other,
        "{stopped}"
    );
    for server in servers {
        server.join().unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

/// Catches `window stop default --json` opening its result array before it
/// inspects the window: an unsafe namespace must still print one JSON value,
/// and its endpoint must never be contacted.
#[test]
fn window_stop_default_reports_an_unsafe_namespace_as_json() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let current = stdout(&invoke(&binary, &runtime, &config, &["versions"]))
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();
    let listener = generation_listener(&runtime, &current);
    listener.set_nonblocking(true).unwrap();
    fs::set_permissions(
        runtime.join("generations"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    let output = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", stdout(&output)));
    let results = reply[0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{reply}");
    assert_eq!(results[0]["error"]["code"], "stop-failed", "{reply}");
    assert_eq!(reply[0]["window"], "default", "{reply}");
    assert_eq!(
        listener.accept().map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skill_prints_the_shipped_skill_and_help_points_agents_to_it() {
    let root = temporary_directory();
    let config = root.join("config");
    let absent_config = root.join("absent-config");
    let runtime = root.join("runtime");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "invalid configuration = [").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let eon_with = |config: &Path, args: &[&str]| {
        eon_command(binary)
            .args(args)
            .env("EON_CONFIG_HOME", config)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env("EON_ORBIT", root.join("missing-orbit"))
            .env("EON_VENUS", root.join("missing-venus"))
            .env("EON_ANIMA", root.join("missing-anima"))
            .output()
            .unwrap()
    };
    let eon = |args: &[&str]| eon_with(&config, args);
    for config in [&config, &absent_config] {
        let output = eon_with(config, &["--skill"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(
            stdout(&output),
            include_str!("../../../skills/eon/SKILL.md")
        );
    }
    let output = eon(&["--skill", "extra"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("eon --help") && error.len() < 200);
    let help = eon(&["--help"]);
    let help = stdout(&help);
    assert!(
        help.lines()
            .any(|line| line.trim_start().starts_with("--skill ")),
        "eon --help must list --skill"
    );
    assert!(
        help.lines().last().unwrap().contains("eon --skill"),
        "eon --help must end by pointing agents to eon --skill"
    );
    assert!(!runtime.exists());
    assert!(!absent_config.exists());
    assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

/// `-V` and `--version` print one line and prepare nothing (EON-C1).
#[test]
fn version_prints_one_line_without_preparing_anything() {
    let root = temporary_directory();
    let config = root.join("config");
    let runtime = root.join("runtime");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "invalid configuration = [").unwrap();
    let eon = |args: &[&str]| {
        eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(args)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_RUNTIME_DIR", &runtime)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env("EON_ORBIT", root.join("missing-orbit"))
            .env("EON_VENUS", root.join("missing-venus"))
            .output()
            .unwrap()
    };
    for flag in ["-V", "--version"] {
        let output = eon(&[flag]);
        assert!(output.status.success(), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}");
        assert_eq!(
            stdout(&output),
            format!("eon {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        let output = eon(&[flag, "extra"]);
        assert_eq!(output.status.code(), Some(2), "{flag} extra");
        assert!(output.stdout.is_empty(), "{flag} extra");
    }
    assert!(!runtime.exists());
    assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn report_prints_a_redacted_bug_report_without_preparing_anything() {
    let root = temporary_directory();
    let home = root.join("home");
    let config = home.join(".config/eon");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[terminal]\nfuture_border_width = 3\n",
    )
    .unwrap();
    let runtime = root.join("runtime");
    let state = home.join(".local/state");
    // Install inspection is local: a Nix it asks records the call, warns on stderr with the
    // home and a secret, and lists one profile entry that installs this eon.
    let package = root.join("package");
    fs::create_dir_all(package.join("bin")).unwrap();
    symlink(env!("CARGO_BIN_EXE_eon"), package.join("bin/eon")).unwrap();
    let stubs = root.join("stubs");
    fs::create_dir(&stubs).unwrap();
    executable(
        &stubs.join("nix"),
        format!(
            "#!/bin/sh\necho \"nix $*\" >> '{}'\n\
             echo \"warning: $HOME/.nix-profile is old; $SECRET_TOKEN\" >&2\n\
             printf '%s' '{{\"version\":3,\"elements\":{{\"eon\":{{\
             \"originalUrl\":\"github:Yazelix/eon\",\"storePaths\":[\"{}\"]}}}}}}'\n",
            root.join("nix-calls").display(),
            package.display()
        ),
    );
    let path = std::iter::once(stubs)
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()))
        .collect::<Vec<_>>();
    let token = "s3cr3t-value-123";
    let eon = |program: &Path, arguments: &[&str]| {
        Command::new(program)
            .args(arguments)
            .env_clear()
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", &home)
            .env("USER", "reporter")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("SECRET_TOKEN", token)
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env("XDG_SESSION_TYPE", "wayland")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let built = Path::new(env!("CARGO_BIN_EXE_eon"));
    let (distribution, compositor) = if cfg!(target_os = "macos") {
        let version = Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .unwrap();
        (
            format!(
                "macOS {}",
                String::from_utf8(version.stdout).unwrap().trim()
            ),
            "macOS".to_owned(),
        )
    } else {
        // A build sandbox has no os-release: the report then names why it cannot tell.
        let pretty = match ["/etc/os-release", "/usr/lib/os-release"]
            .into_iter()
            .find_map(|path| Some((path, fs::read_to_string(path).ok()?)))
        {
            None => {
                "unknown (neither /etc/os-release nor /usr/lib/os-release is readable)".to_owned()
            }
            Some((path, os_release)) => os_release
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim().trim_matches(['"', '\'']))
                .filter(|name| !name.is_empty())
                .map_or_else(
                    || format!("unknown ({path} has no PRETTY_NAME)"),
                    str::to_owned,
                ),
        };
        (pretty, "sway (wayland)".to_owned())
    };
    let check = |output: &Output, install: &str| -> String {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let report = stdout(output).to_owned();
        assert!(report.starts_with("<!-- eon report: read this before sharing it."));
        let mut from = 0;
        for heading in [
            "### Eon version\n```text\neon ".to_owned(),
            "### Window\nnone running\n".to_owned(),
            format!("### Install route\n{install}\n"),
            format!("### Distribution\n{distribution}\n"),
            format!("### Compositor\n{compositor}\n"),
            "### GPU and driver\nnot detected yet; please fill in\n".to_owned(),
            "### Configuration warnings\n```text\neon: ignoring unknown setting \
             terminal.future_border_width in ~/.config/eon/config.toml\n```\n"
                .to_owned(),
            "### Logs\n".to_owned(),
        ] {
            let at = report[from..]
                .find(&heading)
                .unwrap_or_else(|| panic!("missing {heading:?} in order:\n{report}"));
            from += at + heading.len();
        }
        let home = home.to_str().unwrap();
        assert!(!report.contains(home), "{report}");
        assert!(!report.contains(token), "{report}");
        assert!(!report.contains("reporter"), "{report}");
        report
    };

    // Without logs, the Logs section is unknown and nothing is created.
    let report = check(&eon(built, &["report"]), "Other (install_kind: unmanaged)");
    assert!(
        report.contains("### Logs\nunknown (cannot list ~/.local/state/eon/logs: "),
        "{report}"
    );
    assert!(!state.exists());
    assert!(!runtime.exists());
    assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    assert!(!root.join("nix-calls").exists());

    // The report reads the newest log of its window's generation, not a newer one of another.
    let generation = report
        .lines()
        .find(|line| line.starts_with("eon "))
        .and_then(|line| line.split(' ').nth(2))
        .unwrap()
        .to_owned();
    let logs = state.join("eon/logs");
    fs::create_dir_all(&logs).unwrap();
    let log = logs.join(format!("{generation}-1000-1.log"));
    let lines: String = (1..=60)
        .map(|line| {
            format!(
                "line {line} in {} with {token} on reporter-desktop\n",
                home.display()
            )
        })
        .collect();
    fs::write(&log, lines).unwrap();
    fs::write(
        logs.join("g1-ffffffffffffffffffffffffffffffff-2000-1.log"),
        "other generation\n",
    )
    .unwrap();
    let report = check(&eon(built, &["report"]), "Other (install_kind: unmanaged)");
    let tail = format!(
        "### Logs\n```text\n~/.local/state/eon/logs/{generation}-1000-1.log\n{}```\n",
        (21..=60)
            .map(|line| format!("line {line} in ~ with [redacted] on [user]-desktop\n"))
            .collect::<String>()
    );
    assert!(report.ends_with(&tail), "{report}");
    assert!(!report.contains("other generation"));

    // Started through the Nix profile that installs it, the report asks Nix and drops the
    // warning Nix prints beside the listing.
    let profile = home.join(".nix-profile");
    symlink(&package, &profile).unwrap();
    check(
        &eon(&profile.join("bin/eon"), &["report"]),
        "Nix profile (nix profile add) (install_kind: nix-profile)",
    );
    assert_eq!(
        fs::read_to_string(root.join("nix-calls")).unwrap(),
        format!("nix profile list --json --profile {}\n", profile.display())
    );

    // Extra arguments are invalid syntax; help stays with the help.
    let output = eon(built, &["report", "extra"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("eon --help"));
    let output = eon(built, &["report", "--help"]);
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("  report "));
    fs::remove_dir_all(root).unwrap();
}

/// Each entry under `root` with its type, mode, owner, size, times, link target and
/// contents: what a change to it would change.
fn tree_state(root: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        }
        entries.push(format!(
            "{} mode={:o} uid={} size={} mtime={}.{} ctime={}.{} link={:?} contents={:?}",
            path.display(),
            metadata.mode(),
            metadata.uid(),
            metadata.size(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            file_type
                .is_symlink()
                .then(|| fs::read_link(&path).unwrap()),
            file_type.is_file().then(|| fs::read(&path).unwrap()),
        ));
    }
    entries.sort();
    entries
}

#[test]
fn report_leaves_an_orphaned_generation_untouched() {
    let root = temporary_directory();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let runtime = root.join("runtime");
    let eon = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_eon"))
            .args(arguments)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &home)
            .env("EON_RUNTIME_DIR", &runtime)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let versions = eon(&["versions"]);
    assert!(versions.status.success(), "{versions:?}");
    let current = stdout(&versions)
        .lines()
        .find(|line| line.starts_with("eon "))
        .and_then(|line| line.split(' ').nth(2))
        .unwrap()
        .to_owned();
    assert!(!runtime.exists());

    // The current generation and an earlier one, each orphaned: its supervisor is gone, but
    // its directory, dead socket and launcher record stay, and no lifecycle lock exists.
    let private = |path: &Path| {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    };
    private(&runtime);
    private(&runtime.join("generations"));
    for generation in [current.as_str(), "g1-ffffffffffffffffffffffffffffffff"] {
        let directory = runtime.join("generations").join(generation);
        private(&directory);
        let socket = directory.join("eon.sock");
        drop(UnixListener::bind(&socket).unwrap());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        symlink("/bin/false", directory.join("eon")).unwrap();
        // A concurrent test's forked child shares the listener until it execs.
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(&socket).is_ok() {
            assert!(Instant::now() < deadline, "dead socket kept accepting");
            thread::sleep(Duration::from_millis(1));
        }
    }
    let before = tree_state(&runtime);

    let output = eon(&["report"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(
        stdout(&output).contains("### Window\nnone running\n"),
        "{output:?}"
    );
    assert_eq!(tree_state(&runtime), before);
    assert!(
        fs::read_dir(&runtime).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .as_bytes()
            .ends_with(b".lock"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn report_inside_eon_never_asks_a_supervisor_about_its_caller() {
    let root = temporary_directory();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    let runtime = root.join("runtime");
    let eon = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_eon"))
            .args(arguments)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &home)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_TERMINAL", "terminal-1.run-1")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let versions = eon(&["versions"]);
    assert!(versions.status.success(), "{versions:?}");
    let current = stdout(&versions)
        .lines()
        .find(|line| line.starts_with("eon "))
        .and_then(|line| line.split(' ').nth(2))
        .unwrap()
        .to_owned();

    // The window's supervisor answers what `eon window ls` asks and records every request:
    // validating a caller would first reconcile its terminals, which changes runtime state.
    let listener = generation_listener(&runtime, &current);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let server_id = current.clone();
    thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let request = decode_request(&read_request(&mut stream)).unwrap();
            recorded
                .lock()
                .unwrap()
                .push((request.action.clone(), request.caller.clone()));
            let response = match request.action {
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime) => {
                    encode_lifecycle_response(&LifecycleResponse::Runtime(previous_runtime(
                        &server_id,
                    )))
                }
                _ => encode_response(&Response::Failure(ProtocolFailure {
                    code: "unknown-caller".into(),
                    detail: "not this supervisor's terminal".into(),
                })),
            };
            stream.write_all(&response.unwrap()).unwrap();
        }
    });

    let output = eon(&["report"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report = stdout(&output);
    assert!(
        report.contains(&format!("\"generation\":\"{current}\"")),
        "{report}"
    );
    let requests = requests.lock().unwrap();
    assert!(!requests.is_empty());
    for (action, caller) in requests.iter() {
        assert!(
            matches!(
                action,
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime)
            ) && caller.is_none(),
            "{action:?} from {caller:?}"
        );
    }
    drop(requests);
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
fn anima_waits_for_native_readiness_then_hands_off_to_the_pair_and_popup_stops_on_dismissal() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let session_bin = root.join("session-bin");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let anima = root.join("anima");
    let yazi = root.join("yazi");
    let venus_started = root.join("venus-started");
    let venus_ready = root.join("venus-ready");
    let startup_started = root.join("startup-started");
    let startup_release = root.join("startup-release");
    let files_started = root.join("files-started");
    let popup_pid = root.join("popup-pid");
    let popup_release = root.join("popup-release");
    let stop = root.join("stop");
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(
        &venus,
        "#!/bin/sh\nprintf started > \"$EON_TEST_VENUS_STARTED\"\nwhile [ ! -e \"$EON_TEST_VENUS_READY\" ] && [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n[ ! -e \"$EON_TEST_STOP\" ] || exit 0\nprintf ready-v1\ncat >/dev/null\n",
    );
    executable(
        &anima,
        "#!/bin/sh\nif [ \"$2\" = --duration-seconds ]; then\n  printf '%s|%s' \"$1\" \"$3\" > \"$EON_TEST_STARTUP_STARTED\"\n  while [ ! -e \"$EON_TEST_STARTUP_RELEASE\" ]; do sleep 0.01; done\nelse\n  printf '%s' \"$$\" > \"$EON_TEST_POPUP_PID\"\n  while [ ! -e \"$EON_TEST_POPUP_RELEASE\" ]; do sleep 0.01; done\nfi\n",
    );
    symlink(&anima, session_bin.join("anima")).unwrap();
    // The pair's Yazi starts only once its terminal's Anima ends.
    executable(
        &yazi,
        "#!/bin/sh\nprintf '%s|%s' \"$EON_FILES_START\" \"$*\" > \"$EON_TEST_FILES_STARTED\"\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-files")).unwrap();
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
        .env("EON_YAZI", &yazi)
        .env("EON_FILES_CONFIG", &root)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_RUN_CHILD", "1")
        .env("EON_TEST_VENUS_STARTED", &venus_started)
        .env("EON_TEST_VENUS_READY", &venus_ready)
        .env("EON_TEST_STARTUP_STARTED", &startup_started)
        .env("EON_TEST_STARTUP_RELEASE", &startup_release)
        .env("EON_TEST_FILES_STARTED", &files_started)
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
    assert!(!startup_started.exists() && !files_started.exists());
    fs::write(&venus_ready, "ready").unwrap();
    wait_for(&startup_started);
    assert_eq!(fs::read_to_string(&startup_started).unwrap(), "random|3");
    assert!(!files_started.exists());
    fs::write(&startup_release, "done").unwrap();
    wait_for(&files_started);
    let started = fs::read_to_string(&files_started).unwrap();
    assert!(started.starts_with("quick|--client-id "), "{started}");
    let control = generation_runtime(&runtime).join("eon.sock");
    let Response::Snapshot(pending) = workspace_action(&control, "anima-pending", Action::Inspect)
    else {
        panic!("initial pair snapshot missing");
    };
    assert!(pending_pair(&pending).is_some());
    assert!(pending.tabs[0].columns.is_empty());

    assert!(matches!(
        protocol_action(
            &control,
            "pair-dismiss",
            ProtocolAction::DismissPopup(PopupTarget {
                tab: "t1".into(),
                instance: "u1".into(),
            }),
        ),
        Response::Snapshot(snapshot) if !snapshot.tabs[0].pending
    ));
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
    // The kept pair stays as the tab's Files.
    assert_eq!(
        dismissed.tabs[0]
            .popups
            .iter()
            .map(|popup| popup.entry.as_str())
            .collect::<Vec<_>>(),
        ["files"]
    );
    assert_eq!(dismissed.tabs[0].selected_pane, selected_pane);
    wait_for_process_removal(
        fs::read_to_string(&popup_pid).unwrap().parse().unwrap(),
        "dismissed Anima kept running",
    );
    fs::write(&stop, "stop").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

/// Launch flags (EON-C18): `--no-anima` keeps the pair but plays no startup
/// Anima, yet still validates `[anima]`; `--dir` starts t1's first pane, or
/// `run`'s COMMAND, at its directory with neither the pair nor Anima;
/// `window new` forwards both; and `eon --dir` refuses a running window.
#[test]
fn launch_flags_skip_the_startup_anima_and_the_pair() {
    // Independent windows need a short runtime root for their sockets.
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("el{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
    let session_bin = root.join("bin");
    let config = root.join("config");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let anima = root.join("anima");
    let yazi = root.join("yazi");
    let first_command = root.join("command");
    let launch = root.join("launch dir");
    let stop = root.join("stop");
    fs::create_dir(&session_bin).unwrap();
    fs::create_dir(&config).unwrap();
    fs::create_dir(&launch).unwrap();
    let launched = fs::canonicalize(&launch).unwrap();
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nprintf ready-v1\ncat >/dev/null\n");
    executable(
        &anima,
        "#!/bin/sh\nprintf anima > \"$EON_TEST_ANIMA_STARTED\"\n",
    );
    symlink(&anima, session_bin.join("anima")).unwrap();
    executable(
        &yazi,
        "#!/bin/sh\nprintf files > \"$EON_TEST_FILES_STARTED\"\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, session_bin.join("eon-files")).unwrap();
    executable(
        &session_bin.join("eon-nu"),
        "#!/bin/sh\npwd -P > \"$EON_TEST_SHELL_STARTED\"\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    executable(
        &first_command,
        "#!/bin/sh\nprintf '%s|%s' \"$(pwd -P)\" \"$*\" > \"$EON_TEST_COMMAND_STARTED\"\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    // Each invocation names its own observation files, which the window it
    // starts inherits.
    let observed = |name: &str, kind: &str| root.join(format!("{name}-{kind}"));
    let command = |runtime: &str, name: &str| {
        let mut command = eon_command(&binary);
        command
            .current_dir(&root)
            .env("EON_RUNTIME_DIR", root.join(runtime))
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_ANIMA", &anima)
            .env("EON_YAZI", &yazi)
            .env("EON_FILES_CONFIG", &root)
            .env("EON_SESSION_BIN", &session_bin)
            .env("EON_TEST_RUN_CHILD", "1")
            .env("EON_TEST_ANIMA_STARTED", observed(name, "anima"))
            .env("EON_TEST_FILES_STARTED", observed(name, "files"))
            .env("EON_TEST_SHELL_STARTED", observed(name, "shell"))
            .env("EON_TEST_COMMAND_STARTED", observed(name, "command"))
            .env("EON_TEST_STOP", &stop)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL");
        command
    };
    let spawn = |command: &mut Command| TestProcess {
        child: command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let inspect = |control: &Path, id: &str| {
        let Response::Snapshot(snapshot) = workspace_action(control, id, Action::Inspect) else {
            panic!("{id}: snapshot missing");
        };
        snapshot
    };
    let assert_started_in_launch = |snapshot: &Snapshot| {
        let [tab] = snapshot.tabs.as_slice() else {
            panic!("one tab");
        };
        assert!(!tab.pending && tab.popups.is_empty());
        assert_eq!(tab.directory, launch.as_os_str().as_bytes());
        assert_eq!(tab.panes().count(), 1);
    };

    // `--no-anima` still validates `[anima]` and starts nothing.
    fs::write(
        config.join("config.toml"),
        "[anima]\nduration_seconds = 31\n",
    )
    .unwrap();
    let invalid = command("r", "invalid")
        .args(["run", "--no-anima"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1), "{invalid:?}");
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("anima.duration_seconds"),
        "{invalid:?}"
    );
    assert!(!observed("invalid", "files").exists());
    fs::remove_file(config.join("config.toml")).unwrap();

    // `run --no-anima`: the pair without Anima.
    let mut default = spawn(command("r", "a").args(["run", "--no-anima"]));
    wait_for(&observed("a", "files"));
    assert!(!observed("a", "anima").exists());
    let control = generation_runtime(&root.join("r")).join("eon.sock");
    assert!(pending_pair(&inspect(&control, "no-anima")).is_some());

    // `eon --dir` refuses the running window and presents nothing.
    let refused = command("r", "refused")
        .arg("--dir")
        .arg(&launch)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("this Eon window is already running, so --dir has no fresh first tab"),
        "{refused:?}"
    );

    // `window new` forwards `--no-anima`, then `--dir`.
    let window = |name: &str, flags: &[&OsStr]| {
        let output = command("r", name)
            .args(["window", "new"])
            .args(flags)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let id = stdout(&output).trim().to_owned();
        let control = generation_runtime(&root.join("r").join("w").join(&id)).join("eon.sock");
        (id, control)
    };
    let (quick, quick_control) = window("b", &[OsStr::new("--no-anima")]);
    wait_for(&observed("b", "files"));
    assert!(!observed("b", "anima").exists());
    assert!(pending_pair(&inspect(&quick_control, "window-no-anima")).is_some());
    let (started, started_control) = window("c", &[OsStr::new("--dir"), launch.as_os_str()]);
    wait_for(&observed("c", "shell"));
    assert_eq!(
        Path::new(
            fs::read_to_string(observed("c", "shell"))
                .unwrap()
                .trim_end()
        ),
        launched
    );
    assert_started_in_launch(&inspect(&started_control, "window-dir"));
    assert!(!observed("c", "files").exists() && !observed("c", "anima").exists());

    // `run --dir D -- COMMAND` runs COMMAND at once in D.
    let mut commanded = spawn(
        command("s", "d")
            .args(["run", "--dir"])
            .arg(&launch)
            .arg("--")
            .arg(&first_command)
            .arg("first argument"),
    );
    wait_for(&observed("d", "command"));
    assert_eq!(
        fs::read_to_string(observed("d", "command")).unwrap(),
        format!("{}|first argument", launched.display())
    );
    let commanded_control = generation_runtime(&root.join("s")).join("eon.sock");
    assert_started_in_launch(&inspect(&commanded_control, "run-dir-command"));
    assert!(
        !observed("d", "files").exists()
            && !observed("d", "anima").exists()
            && !observed("d", "shell").exists()
    );

    for id in [&quick, &started] {
        let stopped = command("r", "stop")
            .args(["window", "stop", id, "--json"])
            .output()
            .unwrap();
        assert!(stopped.status.success(), "{stopped:?}");
    }
    fs::write(&stop, "stop").unwrap();
    wait_for_successful_exit(&mut default.child);
    wait_for_successful_exit(&mut commanded.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_or_stuck_startup_anima_reports_and_continues_to_quick_search() {
    let root = temporary_directory();
    let anima = root.join("anima");
    let pid = root.join("anima-pid");
    let yazi = root.join("yazi");
    let yazi_log = root.join("yazi.log");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    symlink(env!("CARGO_BIN_EXE_eon"), bin.join("eon-files")).unwrap();
    executable(
        &yazi,
        "#!/bin/sh\nprintf '%s|%s\\n' \"$EON_FILES_START\" \"$*\" >> \"$EON_TEST_YAZI_LOG\"\n",
    );
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
        let output = eon_command(&bin.join("eon-files"))
            .args([
                "--eon-anima",
                "random",
                "1",
                "--eon-quick",
                "--client-id",
                "7",
            ])
            .env("EON_ANIMA", &program)
            .env("EON_YAZI", &yazi)
            .env("EON_FILES_CONFIG", &root)
            .env("EON_CONFIG_HOME", root.join("config"))
            .env("EON_TEST_ANIMA_PID", &pid)
            .env("EON_TEST_YAZI_LOG", &yazi_log)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(stderr.contains("continuing to quick search"), "{stderr}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    assert_eq!(log_lines(&yazi_log), ["quick|--client-id 7"; 2]);
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

fn publish_management_record(path: &Path, record: &ManagementRecord, ready_delay: Option<u64>) {
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
    if let Some(delay) = ready_delay {
        thread::sleep(Duration::from_millis(delay));
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

fn endpoint_identity(path: &Path) -> EndpointIdentity {
    EndpointIdentity {
        path: path.as_os_str().as_bytes().to_vec(),
        object: object_identity(path),
    }
}

/// Whether the process `pid` waits, as in a poll: Some(true) when its main
/// thread sleeps on Linux, or no thread runs on macOS; None once it has ended.
/// Linux shows a page-in as `D`, so a sleep there is a real wait. macOS also
/// counts loader and page-in waits as not running, so there it is best effort.
#[cfg(target_os = "linux")]
fn process_waits(pid: u32) -> std::io::Result<Option<bool>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let state = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.trim_start().chars().next())
        .ok_or_else(|| std::io::Error::other(format!("unreadable /proc/{pid}/stat: {stat}")))?;
    Ok(match state {
        'Z' | 'X' => None,
        state => Some(state == 'S'),
    })
}

#[cfg(target_os = "macos")]
fn process_waits(pid: u32) -> std::io::Result<Option<bool>> {
    // SAFETY: proc_taskinfo holds only integers, so all zeros is a valid value.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into the local info.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if read == size {
        return Ok(Some(info.pti_numrunning == 0));
    }
    let error = std::io::Error::last_os_error();
    // An ended process, a zombie included, has no task to describe.
    if read <= 0 && error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(None);
    }
    Err(std::io::Error::other(format!(
        "proc_pidinfo({pid}) gave {read} of {size} bytes: {error}"
    )))
}

/// Writes `message` to a management client. A client that went away, as a
/// supervisor killed mid-request does, shows on the next read, which releases
/// its lease as a real Orbit does.
fn write_management(stream: &mut UnixStream, message: ManagementServerMessage) {
    let bytes = management::encode_server_message(&message).unwrap();
    stream.set_nonblocking(false).unwrap();
    let _ = stream.write_all(&bytes);
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

fn test_foreground() -> Option<Foreground> {
    let path = std::env::var_os("EON_TEST_ORBIT_FOREGROUND")?;
    match fs::read_to_string(path).ok()?.trim() {
        "child" => Some(Foreground::Child),
        "other" => Some(Foreground::Other),
        _ => None,
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
        .position(|argument| argument == OsStr::new("--management-v7"))
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
    if session_id == "terminal-2"
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
        process_start: process::start_identity(std::process::id()),
        uid: unsafe { libc::geteuid() },
        presentation: endpoint_identity(&presentation),
        management: endpoint_identity(&management_path),
    };
    let command = arguments
        .iter()
        .rposition(|argument| argument == OsStr::new("--"))
        .map(|separator| &arguments[separator + 1..])
        .unwrap_or(&[]);
    // A Scrollback editor, whose snapshot must already name its terminal.
    let snapshot = command
        .last()
        .filter(|argument| argument.as_bytes().ends_with(b"-scrollback.txt"))
        .map(Path::new);
    if let (Some(file), Some(log)) = (snapshot, std::env::var_os("EON_TEST_SNAPSHOT_LOG")) {
        let marker = fs::read_to_string(file.with_file_name(".terminal")).unwrap_or_default();
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(log, "{}|{marker}", identity.session_id).unwrap();
    }
    let ready_delay = if identity.session_id == "terminal-2" {
        std::env::var("EON_TEST_PAIR_READY_DELAY_MS").ok()
    } else if snapshot.is_some() {
        std::env::var_os("EON_TEST_EDITOR_READY_DELAY")
            .and_then(|path| fs::read_to_string(path).ok())
    } else {
        None
    };
    publish_management_record(
        &record_path,
        &ManagementRecord::Live(identity.clone()),
        ready_delay.map(|delay| delay.trim().parse().unwrap()),
    );
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
            // macOS reports a client that left before its accept.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::ConnectionAborted
                ) => {}
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
                        // A test scripts how Orbit refuses a read or an input
                        // by writing `busy`, `closed`, `unbracketed` (pastes
                        // only) or `busy-enter` (Enter only) to the file
                        // EON_TEST_ORBIT_REPLY names.
                        let reply = std::env::var_os("EON_TEST_ORBIT_REPLY")
                            .and_then(|path| fs::read_to_string(path).ok())
                            .unwrap_or_default();
                        if let ManagementClientMessage::Input { quiet_ms, .. } = &request
                            && let Some(log) = std::env::var_os("EON_TEST_QUIET_LOG")
                        {
                            fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(log)
                                .unwrap()
                                .write_all(format!("{quiet_ms}\n").as_bytes())
                                .unwrap();
                        }
                        let scripted = match (reply.trim(), &request) {
                            ("typing", ManagementClientMessage::Input { quiet_ms, .. })
                            | (
                                "typing-enter",
                                ManagementClientMessage::Input {
                                    input: LeaseInput::Enter,
                                    quiet_ms,
                                },
                            ) if *quiet_ms != 0 => {
                                Some(ManagementServerMessage::Refused(Refusal::Typing))
                            }
                            (
                                "busy",
                                ManagementClientMessage::ReadVisible
                                | ManagementClientMessage::ReadHistory
                                | ManagementClientMessage::Input { .. },
                            )
                            | (
                                "busy-enter",
                                ManagementClientMessage::Input {
                                    input: LeaseInput::Enter,
                                    ..
                                },
                            ) => Some(ManagementServerMessage::Busy),
                            (
                                "closed",
                                ManagementClientMessage::ReadVisible
                                | ManagementClientMessage::ReadHistory
                                | ManagementClientMessage::Input { .. },
                            ) => Some(ManagementServerMessage::Refused(Refusal::Closed)),
                            (
                                "unbracketed",
                                ManagementClientMessage::Input {
                                    input: LeaseInput::Paste(_),
                                    ..
                                },
                            ) => Some(ManagementServerMessage::Refused(
                                Refusal::UnbracketedNewline,
                            )),
                            _ => None,
                        };
                        let foreground = matches!(request, ManagementClientMessage::ProgramStatus)
                            .then(test_foreground)
                            .flatten();
                        match request {
                            _ if *leased && scripted.is_some() => {
                                write_management(stream, scripted.unwrap());
                            }
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
                            // A test scripts the terminal's foreground by
                            // writing `child` or `other` to the file
                            // EON_TEST_ORBIT_FOREGROUND names; otherwise
                            // Program status fails, so every terminal is busy.
                            ManagementClientMessage::ProgramStatus
                                if *leased && foreground.is_some() =>
                            {
                                write_management(
                                    stream,
                                    ManagementServerMessage::ProgramStatus(ProgramStatus {
                                        count: 0,
                                        background: 0,
                                        foreground: foreground.unwrap(),
                                        update: 0,
                                        terminal_title: String::new(),
                                        omitted: Vec::new(),
                                        records: Vec::new(),
                                    }),
                                )
                            }
                            ManagementClientMessage::Stop if *leased => stop = true,
                            ManagementClientMessage::AttachAllow {
                                process_id,
                                process_start,
                            } if *leased => {
                                if let Some(log) = std::env::var_os("EON_TEST_ATTACH_LOG") {
                                    // One append per line, so the fake Orbits
                                    // that share the log never interleave.
                                    let line = format!(
                                        "{} {process_id} {process_start}\n",
                                        identity.session_id
                                    );
                                    fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(log)
                                        .unwrap()
                                        .write_all(line.as_bytes())
                                        .unwrap();
                                }
                                write_management(stream, ManagementServerMessage::Allowed);
                            }
                            ManagementClientMessage::ReadVisible if *leased => write_management(
                                stream,
                                ManagementServerMessage::Visible(VisibleText {
                                    revision: 1,
                                    screen: OrbitScreen::Primary,
                                    rows: 24,
                                    columns: 80,
                                    title: identity.session_id.clone(),
                                    directory: b"/fake".to_vec(),
                                    omitted_rows: 0,
                                    lossy: true,
                                    text: format!("screen of {}", identity.session_id),
                                }),
                            ),
                            // `long-history` answers with more than EONW's
                            // 2 MiB, and `cut-history` with a newest line
                            // the bound cut.
                            ManagementClientMessage::ReadHistory if *leased => {
                                // `slow-history` answers after the supervisor's
                                // 2 s terminal deadline.
                                if reply.trim() == "slow-history" {
                                    thread::sleep(Duration::from_millis(2500));
                                }
                                // A send buffer as small as macOS's 8 KiB, so the
                                // supervisor reads `long-history` in as many pieces
                                // here as there.
                                if reply.trim() == "long-history" {
                                    let size: libc::c_int = 8192;
                                    // SAFETY: size is a live c_int of the length given.
                                    let set = unsafe {
                                        libc::setsockopt(
                                            stream.as_raw_fd(),
                                            libc::SOL_SOCKET,
                                            libc::SO_SNDBUF,
                                            (&raw const size).cast(),
                                            size_of::<libc::c_int>() as libc::socklen_t,
                                        )
                                    };
                                    assert_eq!(set, 0);
                                }
                                let (omitted_lines, cut_bytes, text) = match reply.trim() {
                                    "long-history" => (5, 0, "h\n".repeat(3 << 19) + "end"),
                                    "cut-history" => (1, 7, "line".into()),
                                    "empty-history" => (0, 0, String::new()),
                                    _ => (
                                        0,
                                        0,
                                        format!(
                                            "history of {0}\nscreen of {0}",
                                            identity.session_id
                                        ),
                                    ),
                                };
                                write_management(
                                    stream,
                                    ManagementServerMessage::History(HistoryText {
                                        revision: 2,
                                        screen: OrbitScreen::Alternate,
                                        rows: 24,
                                        columns: 80,
                                        title: identity.session_id.clone(),
                                        directory: b"/fake".to_vec(),
                                        omitted_lines,
                                        cut_bytes,
                                        lossy: true,
                                        text,
                                    }),
                                )
                            }
                            ManagementClientMessage::Input { input, .. } if *leased => {
                                if let Some(log) = std::env::var_os("EON_TEST_INPUT_LOG") {
                                    let line = format!("{} {input:?}\n", identity.session_id);
                                    fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(log)
                                        .unwrap()
                                        .write_all(line.as_bytes())
                                        .unwrap();
                                }
                                // When each input arrived, in milliseconds.
                                if let Some(log) = std::env::var_os("EON_TEST_INPUT_TIMES") {
                                    let since = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis();
                                    fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(log)
                                        .unwrap()
                                        .write_all(format!("{since} {input:?}\n").as_bytes())
                                        .unwrap();
                                }
                                write_management(
                                    stream,
                                    ManagementServerMessage::Input {
                                        bracketed: matches!(input, LeaseInput::Paste(_)),
                                    },
                                );
                            }
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
                // One append per line, so the fake Orbits that share the
                // log never interleave.
                if lease_released && let Some(path) = std::env::var_os("EON_TEST_LEASE_RELEASED") {
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .unwrap()
                        .write_all(format!("{}\n", identity.session_id).as_bytes())
                        .unwrap();
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

/// Asserts that within `timeout` the fake Orbit of each of `sessions` let the
/// running process `venus` attach, named by its process ID and start identity
/// (EON-C11).
fn assert_named(log: &Path, sessions: &[String], venus: u32, timeout: Duration) {
    let name = format!("{venus} {}", process::start_identity(venus));
    let deadline = Instant::now() + timeout;
    loop {
        let lines = fs::read_to_string(log).unwrap_or_default();
        let unnamed = sessions
            .iter()
            .filter(|session| {
                !lines
                    .lines()
                    .any(|line| line == format!("{session} {name}"))
            })
            .collect::<Vec<_>>();
        if unnamed.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{unnamed:?} not named to Venus {venus}:\n{lines}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The terminals of a snapshot's shown workspace.
fn snapshot_sessions(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .tabs
        .iter()
        .flat_map(|tab| {
            tab.panes()
                .map(|pane| pane.session.clone())
                .chain(tab.popups.iter().map(|popup| popup.session.clone()))
        })
        .collect()
}

fn wait_for(path: &Path) {
    if path.file_name().is_some_and(|name| name == "eon.sock") {
        return wait_for_ready(path);
    }
    let ready =
        || fs::metadata(path).is_ok_and(|metadata| !metadata.is_file() || metadata.len() != 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready(), "{} was not created or populated", path.display());
}

fn wait_for_ready(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ready = (|| {
            let mut stream = UnixStream::connect(path).ok()?;
            let remaining = deadline.checked_duration_since(Instant::now())?;
            stream.set_read_timeout(Some(remaining)).ok()?;
            stream.set_write_timeout(Some(remaining)).ok()?;
            stream
                .write_all(
                    &encode_request(&Request {
                        id: "await-runtime".into(),
                        caller: None,
                        action: ProtocolAction::Workspace(Action::InspectRuntime),
                    })
                    .unwrap(),
                )
                .ok()?;
            let mut bytes = vec![0; HEADER_BYTES];
            stream.read_exact(&mut bytes).ok()?;
            bytes.resize(declared_message_len(&bytes).ok()?, 0);
            stream.read_exact(&mut bytes[HEADER_BYTES..]).ok()?;
            match decode_lifecycle_response(&bytes).ok()? {
                LifecycleResponse::Runtime(runtime) => Some(runtime.attach.available),
                LifecycleResponse::Failure(_) => Some(false),
            }
        })()
        .unwrap_or(false);
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} did not admit a ready runtime",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_process_removal(process_id: u32, failure: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while process::exists(process_id) {
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

/// Waits until `deadline` for `child` while reading its terminals, as a terminal
/// does. On macOS a session leader drains its controlling terminal's output, such
/// as echoed input, before it can exit.
fn exit_reading_terminals(
    child: &mut Child,
    terminals: &mut [fs::File],
    deadline: Instant,
) -> Option<ExitStatus> {
    let mut polled = terminals
        .iter()
        .map(|terminal| libc::pollfd {
            fd: terminal.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect::<Vec<_>>();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        // SAFETY: the descriptors stay open for the call; poll skips negative ones.
        if unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as _, 10) } > 0 {
            for (terminal, polled) in terminals.iter_mut().zip(&mut polled) {
                if polled.revents == 0 {
                    continue;
                }
                // A terminal that ends, fails or reports anything but input is done.
                let done = polled.revents & libc::POLLIN == 0
                    || match terminal.read(&mut [0; 256]) {
                        Ok(count) => count == 0,
                        Err(error) => error.kind() != std::io::ErrorKind::Interrupted,
                    };
                if done {
                    polled.fd = -1;
                }
            }
        }
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

/// The first pending tab's Files pair in `snapshot` (EON-C18).
fn pending_pair(snapshot: &Snapshot) -> Option<(&Tab, &Popup)> {
    snapshot
        .tabs
        .iter()
        .filter(|tab| tab.pending)
        .find_map(|tab| {
            tab.popups
                .iter()
                .find(|popup| popup.entry == "files")
                .map(|popup| (tab, popup))
        })
}

/// Waits until no tab is pending and the active tab's directory is `directory`.
fn wait_for_commit(socket: &Path, directory: &Path) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sequence = 0;
    loop {
        let response = workspace_action(
            socket,
            &format!("commit-inspect-{sequence}"),
            Action::Inspect,
        );
        let last_response = match response {
            Response::Snapshot(snapshot) => {
                let active_directory = snapshot
                    .tabs
                    .iter()
                    .find(|tab| tab.id == snapshot.active_tab)
                    .map(|tab| tab.directory.as_slice());
                if pending_pair(&snapshot).is_none()
                    && active_directory == Some(directory.as_os_str().as_bytes())
                {
                    return snapshot;
                }
                format!(
                    "active tab directory {:?}, expected {:?}",
                    active_directory.map(String::from_utf8_lossy),
                    directory.to_string_lossy()
                )
            }
            response => format!("{response:?}"),
        };
        assert!(
            Instant::now() < deadline,
            "the pending tab did not commit: {last_response}"
        );
        sequence += 1;
        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for a snapshot that `ready` accepts.
fn wait_for_snapshot(socket: &Path, what: &str, ready: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    for sequence in 0.. {
        if let Response::Snapshot(snapshot) =
            workspace_action(socket, &format!("{what}-{sequence}"), Action::Inspect)
        {
            if ready(&snapshot) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "{what}: {snapshot:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    unreachable!()
}

fn log_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Waits until `path` holds at least `count` lines.
fn wait_for_lines(path: &Path, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let lines = log_lines(path);
        if lines.len() >= count {
            return lines;
        }
        assert!(
            Instant::now() < deadline,
            "{} holds {lines:?}, want {count} lines",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// How one pair's `eon-files` started: its tab, directory and arguments.
struct PairLaunch {
    tab: String,
    directory: PathBuf,
    arguments: Vec<String>,
}

impl PairLaunch {
    fn client_id(&self) -> u64 {
        let index = self
            .arguments
            .iter()
            .position(|argument| argument == "--client-id")
            .unwrap();
        self.arguments[index + 1].parse().unwrap()
    }
}

/// One full Eon window for the Files pair (EON-C18). Its fake Orbit runs each
/// terminal's command: `eon-files` is a stub that logs its terminal, tab,
/// directory and arguments and runs until `files-exit.SESSION` appears, `ya`
/// and `zoxide` log their arguments, panes log their terminal and directory,
/// and Venus stays until it is killed.
struct PairWindow {
    root: PathBuf,
    runtime: PathBuf,
    config: PathBuf,
    launch: PathBuf,
    home: PathBuf,
    xdg: PathBuf,
    stop: PathBuf,
    venus_pid: PathBuf,
    lease_released: PathBuf,
    fail_stop_once: PathBuf,
    files_log: PathBuf,
    ya_log: PathBuf,
    zoxide_log: PathBuf,
    panes_log: PathBuf,
    binary: PathBuf,
}

impl PairWindow {
    fn new() -> Self {
        let directory = temporary_directory();
        let root = fs::canonicalize(&directory).unwrap();
        let window = Self {
            // macOS resolves /tmp to /private/tmp, which would push the terminal sockets past
            // their 103 bytes; the runtime keeps the short spelling the suites' TMPDIR gives.
            runtime: directory.join("runtime"),
            config: root.join("config"),
            launch: root.join("launch"),
            home: root.join("home"),
            xdg: root.join("xdg"),
            stop: root.join("stop"),
            venus_pid: root.join("venus.pid"),
            lease_released: root.join("lease-released"),
            fail_stop_once: root.join("fail-stop-once"),
            files_log: root.join("files.log"),
            ya_log: root.join("ya.log"),
            zoxide_log: root.join("zoxide.log"),
            panes_log: root.join("panes.log"),
            binary: PathBuf::from(env!("CARGO_BIN_EXE_eon")),
            root,
        };
        configure_mock_desktop(&window.config);
        // No quota chips, so no quota read lands between two snapshots.
        fs::OpenOptions::new()
            .append(true)
            .open(window.config.join("config.toml"))
            .unwrap()
            .write_all(b"\n[bar]\nwidgets = []\n")
            .unwrap();
        let session_bin = window.root.join("session-bin");
        for directory in [&window.launch, &window.home, &window.xdg, &session_bin] {
            fs::create_dir(directory).unwrap();
        }
        managed_orbit_executable(&window.root.join("orbit"));
        executable(
            &window.root.join("venus"),
            "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\ncat >/dev/null\n",
        );
        executable(
            &session_bin.join("eon-files"),
            r#"#!/bin/sh
session=${EON_TERMINAL%%.*}
printf '%s|%s|%s|%s\n' "$session" "$EON_TAB" "$(pwd -P)" "$*" >> "$EON_TEST_FILES_LOG"
while [ ! -e "$EON_TEST_STOP" ] && [ ! -e "$EON_TEST_FILES_EXIT.$session" ]; do sleep 0.01; done
"#,
        );
        executable(
            &session_bin.join("ya"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$EON_TEST_YA_LOG\"\n",
        );
        executable(
            &session_bin.join("zoxide"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$EON_TEST_ZOXIDE_LOG\"\n",
        );
        executable(
            &session_bin.join("eon-nu"),
            r#"#!/bin/sh
printf '%s|%s\n' "${EON_TERMINAL%%.*}" "$(pwd -P)" >> "$EON_TEST_PANES_LOG"
while [ ! -e "$EON_TEST_STOP" ]; do sleep 0.01; done
"#,
        );
        window
    }

    /// Starts `eon run`, with `child` as `eon run -- CHILD` when given.
    fn spawn(&self, child: &[&str]) -> TestProcess {
        let process = self.start(self.command(child));
        wait_for_ready(&self.control());
        process
    }

    fn start(&self, mut command: Command) -> TestProcess {
        TestProcess {
            child: command.spawn().unwrap(),
            stop: self.stop.clone(),
        }
    }

    fn command(&self, child: &[&str]) -> Command {
        let mut command = eon_command(&self.binary);
        command.arg("run");
        if !child.is_empty() {
            command.arg("--").args(child);
        }
        command
            .current_dir(&self.launch)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_RUNTIME_DIR", &self.xdg)
            .env("EON_RUNTIME_DIR", &self.runtime)
            .env("EON_CONFIG_HOME", &self.config)
            .env("EON_ORBIT", self.root.join("orbit"))
            .env("EON_VENUS", self.root.join("venus"))
            .env("EON_SESSION_BIN", self.root.join("session-bin"))
            .env("EON_TEST_RUN_CHILD", "1")
            .env("EON_TEST_STOP", &self.stop)
            .env("EON_TEST_VENUS_PID", &self.venus_pid)
            .env("EON_TEST_LEASE_RELEASED", &self.lease_released)
            .env("EON_TEST_FAIL_STOP_ONCE", &self.fail_stop_once)
            .env("EON_TEST_FILES_LOG", &self.files_log)
            .env("EON_TEST_FILES_EXIT", self.root.join("files-exit"))
            .env("EON_TEST_YA_LOG", &self.ya_log)
            .env("EON_TEST_ZOXIDE_LOG", &self.zoxide_log)
            .env("EON_TEST_PANES_LOG", &self.panes_log)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn generation(&self) -> PathBuf {
        generation_runtime(&self.runtime)
    }

    fn control(&self) -> PathBuf {
        self.generation().join("eon.sock")
    }

    fn eon(&self, arguments: &[&str]) -> Output {
        invoke(&self.binary, &self.runtime, &self.config, arguments)
    }

    fn inspect(&self, id: &str) -> Snapshot {
        match workspace_action(&self.control(), id, Action::Inspect) {
            Response::Snapshot(snapshot) => snapshot,
            response => panic!("{id}: {response:?}"),
        }
    }

    fn invoke_popup(&self, id: &str, entry: &str, expected: Option<&str>) -> Response {
        protocol_action(
            &self.control(),
            id,
            ProtocolAction::InvokePopup {
                tab: self.inspect(&format!("{id}-tab")).active_tab,
                entry: entry.into(),
                expected_instance: expected.map(Into::into),
                intent: InvokeIntent::Toggle,
            },
        )
    }

    /// How the pair of terminal `session` started.
    fn pair_launch(&self, session: &str) -> PairLaunch {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(line) = log_lines(&self.files_log)
                .into_iter()
                .find(|line| line.starts_with(&format!("{session}|")))
            {
                let fields = line.split('|').collect::<Vec<_>>();
                return PairLaunch {
                    tab: fields[1].into(),
                    directory: fields[2].into(),
                    arguments: fields[3].split(' ').map(str::to_owned).collect(),
                };
            }
            assert!(Instant::now() < deadline, "pair {session} did not start");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn visits(&self) -> Vec<String> {
        log_lines(&self.zoxide_log)
    }

    /// Kills `supervisor` as a crash would and waits until the fake Orbit of
    /// every terminal its window showed has seen its lease end. Each fake
    /// Orbit notices on its own poll, and one still holding the ended lease
    /// refuses a replacement supervisor's lease as Busy.
    fn crash(&self, supervisor: &mut TestProcess) {
        let leased = snapshot_sessions(&self.inspect("leased"));
        let _ = fs::remove_file(&self.lease_released);
        supervisor.child.kill().unwrap();
        assert!(!supervisor.child.wait().unwrap().success());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let released = log_lines(&self.lease_released);
            if leased.iter().all(|session| released.contains(session)) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "leases of {leased:?} not all released: {released:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn stop_generation(&self, supervisor: &mut TestProcess) -> String {
        let stopped = self.eon(&["window", "stop", "default", "--json"]);
        assert!(stopped.status.success(), "{}", stdout(&stopped));
        wait_for_successful_exit(&mut supervisor.child);
        stdout(&stopped).to_owned()
    }
}

impl Drop for PairWindow {
    fn drop(&mut self) {
        let _ = fs::write(&self.stop, "");
        if !thread::panicking() {
            // Pair and pane stubs see the stop file before their directories go.
            thread::sleep(Duration::from_millis(50));
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn pending_tabs_host_the_files_pair_on_quick_search() {
    let window = PairWindow::new();
    let mut supervisor = window.spawn(&[]);
    let generation = window.generation();
    let control = window.control();
    let first = window.pair_launch("terminal-2");
    let snapshot = window.inspect("pending");
    let (tab, pair) = pending_pair(&snapshot).unwrap();
    assert_eq!(
        (tab.id.as_str(), pair.id.as_str(), pair.session.as_str()),
        ("t1", "u1", "terminal-2")
    );
    assert_eq!(tab.selected_popup.as_deref(), Some("u1"));
    assert_eq!(tab.popups.len(), 1);
    assert!(tab.columns.is_empty() && tab.selected_pane.is_none());
    assert_eq!(
        pair.endpoint,
        generation.join("terminal-2.sock").as_os_str().as_bytes()
    );
    // terminal-1 and orbit.sock wait for the window's first pane.
    assert!(!generation.join("orbit.sock").exists());
    assert!(artifact_path(&generation.join("terminal-2.sock"), ".pending").exists());
    assert_eq!(first.tab, "t1");
    assert_eq!(first.directory, window.launch);
    let id = first.client_id();
    assert!((1 << 62..1 << 63).contains(&id), "{id}");
    // fzf listens beside the pair's endpoint, in the window's own runtime.
    let socket = generation.join("terminal-2.quick.sock");
    assert_eq!(
        first.arguments,
        [
            "--eon-notice",
            control.to_str().unwrap(),
            "--eon-quick",
            "--eon-pair-socket",
            socket.to_str().unwrap(),
            "--client-id",
            &id.to_string(),
        ]
    );

    // Focusing the shown pair, or moving focus inside its pending tab, changes nothing.
    assert_eq!(
        protocol_action(
            &control,
            "pair-focus",
            ProtocolAction::InvokePopup {
                tab: "t1".into(),
                entry: "project".into(),
                expected_instance: Some("u1".into()),
                intent: InvokeIntent::Focus,
            },
        ),
        Response::Snapshot(snapshot.clone())
    );
    assert_eq!(
        workspace_action(&control, "pair-singleton", Action::Focus(Direction::Left)),
        Response::Snapshot(snapshot.clone())
    );
    // Tools other than the pair wait for the tab's directory.
    assert!(matches!(
        protocol_action(
            &control,
            "pending-agent",
            ProtocolAction::InvokePopup {
                tab: "t1".into(),
                entry: "agent".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        ),
        Response::Failure(failure) if failure.code == "choose-directory-first"
    ));

    // Later tabs and workspaces start their own pairs at home, without the notice.
    assert!(window.eon(&["tab", "create"]).status.success());
    let second = window.pair_launch("terminal-3");
    assert_eq!(
        (second.tab.as_str(), second.directory.as_path()),
        ("t2", window.home.as_path())
    );
    let second_id = second.client_id();
    assert_ne!(second_id, id);
    assert_eq!(
        second.arguments,
        [
            "--eon-quick",
            "--eon-pair-socket",
            generation.join("terminal-3.quick.sock").to_str().unwrap(),
            "--client-id",
            &second_id.to_string(),
        ]
    );
    let snapshot = window.inspect("two-pending");
    assert!(snapshot.tabs.iter().all(|tab| tab.pending));
    assert_ne!(
        snapshot.tabs[0].popups[0].endpoint,
        snapshot.tabs[1].popups[0].endpoint
    );
    assert!(window.eon(&["session", "new"]).status.success());
    let third = window.pair_launch("terminal-4");
    assert_eq!(third.tab, "t3");
    assert_eq!(third.arguments[0], "--eon-quick");

    // Pending pairs are popups, not durable terminals: lifecycle inspection
    // and Stop count none of them, and Stop still stops each (EON-C18).
    let pairs = ["terminal-2", "terminal-3", "terminal-4"]
        .map(|session| live_identity(&generation.join(format!("{session}.sock"))));
    let inventory = window.eon(&["window", "ls", "--json"]);
    assert!(inventory.status.success(), "{}", stdout(&inventory));
    let listed: serde_json::Value = serde_json::from_str(stdout(&inventory)).unwrap();
    assert_eq!(
        listed["windows"][0]["terminals"],
        serde_json::json!([]),
        "{listed}"
    );
    let stopped = window.stop_generation(&mut supervisor);
    assert!(stopped.contains("\"terminals\":[]"), "{stopped}");
    for pair in pairs {
        wait_for_process_removal(pair.process_id, "Stop left a pending pair running");
    }
}

#[test]
fn a_pick_commits_a_pending_tab_with_one_pane_and_keeps_its_pair() {
    let window = PairWindow::new();
    let [selected, left, right] = ["selected", "left", "right"].map(|name| {
        let directory = window.root.join(name);
        fs::create_dir(&directory).unwrap();
        directory
    });
    let child_log = window.root.join("child.log");
    let child = window.root.join("child");
    executable(
        &child,
        format!(
            "#!/bin/sh\nprintf '%s|%s\\n' \"${{EON_TERMINAL%%.*}}\" \"$(pwd -P)\" >> '{}'\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
            child_log.display()
        ),
    );
    let mut supervisor = window.spawn(&[child.to_str().unwrap()]);
    let generation = window.generation();
    window.pair_launch("terminal-2");

    let picked = window.eon(&["tab", "directory", "t1", "--", selected.to_str().unwrap()]);
    assert!(picked.status.success(), "{picked:?}");
    let snapshot = window.inspect("picked");
    let tab = &snapshot.tabs[0];
    assert!(!tab.pending);
    assert_eq!(tab.directory, selected.as_os_str().as_bytes());
    assert!(tab.selected_popup.is_none(), "the pick hides the pair");
    let panes = tab.panes().collect::<Vec<_>>();
    assert_eq!(panes.len(), 1);
    assert_eq!(
        (panes[0].id.as_str(), panes[0].session.as_str()),
        ("p1", "terminal-1")
    );
    assert_eq!(
        panes[0].endpoint,
        generation.join("orbit.sock").as_os_str().as_bytes()
    );
    assert_eq!(tab.popups.len(), 1);
    assert_eq!(
        (tab.popups[0].entry.as_str(), tab.popups[0].session.as_str()),
        ("files", "terminal-2")
    );
    assert!(
        generation.join("terminal-2.sock").exists(),
        "the pair stays as Files"
    );
    assert!(!artifact_path(&generation.join("terminal-2.sock"), ".pending").exists());
    // Committed, the pair counts as its window's Files popup terminal.
    let inventory = stdout(&window.eon(&["window", "ls", "--json"])).to_owned();
    let listed: serde_json::Value = serde_json::from_str(&inventory).unwrap();
    let terminals = listed["windows"][0]["terminals"].as_array().unwrap();
    for session in ["terminal-1", "terminal-2"] {
        assert!(terminals.contains(&session.into()), "{inventory}");
    }
    // t1's first pane runs `eon run -- CHILD`.
    assert_eq!(
        wait_for_lines(&child_log, 1),
        [format!("terminal-1|{}", selected.display())]
    );
    assert!(log_lines(&window.panes_log).is_empty());
    let first_visit = vec![format!("add -- {}", selected.display())];
    assert_eq!(wait_for_lines(&window.zoxide_log, 1), first_visit);

    // Racing picks on a pending tab start one pane; the later one retargets.
    // The pending tab has no pane to name until then.
    let pending = window.eon(&["tab", "create", "--json"]);
    assert!(pending.status.success(), "{pending:?}");
    let pending: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    assert_eq!(
        pending["created"],
        serde_json::json!({"tab": "t2", "pane": null}),
        "{pending}"
    );
    window.pair_launch("terminal-3");
    thread::scope(|scope| {
        let picks = [&left, &right].map(|directory| {
            let window = &window;
            scope.spawn(move || {
                window.eon(&["tab", "directory", "t2", "--", directory.to_str().unwrap()])
            })
        });
        for pick in picks {
            let output = pick.join().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    });
    let snapshot = window.inspect("raced");
    assert!(!snapshot.tabs[1].pending);
    assert_eq!(snapshot.tabs[1].panes().count(), 1);
    let panes = wait_for_lines(&window.panes_log, 1);
    assert_eq!(panes.len(), 1, "{panes:?}");
    assert!(
        [&left, &right]
            .iter()
            .any(|directory| panes[0] == format!("terminal-4|{}", directory.display())),
        "{panes:?}"
    );
    assert_eq!(wait_for_lines(&window.zoxide_log, 3).len(), 3);

    // A refused pick changes nothing and adds no visit.
    assert!(window.eon(&["tab", "create"]).status.success());
    window.pair_launch("terminal-5");
    let refused = window.eon(&[
        "tab",
        "directory",
        "t3",
        "--",
        window.root.join("missing").to_str().unwrap(),
    ]);
    assert!(!refused.status.success());
    let snapshot = window.inspect("refused");
    let (tab, pair) = pending_pair(&snapshot).unwrap();
    assert_eq!(
        (tab.id.as_str(), pair.session.as_str()),
        ("t3", "terminal-5")
    );
    assert_eq!(tab.selected_popup.as_deref(), Some(pair.id.as_str()));
    thread::sleep(Duration::from_millis(300));
    assert_eq!(window.visits().len(), 3);
    // Stop reports the committed tabs' terminals but not t3's pending pair.
    let stopped = window.stop_generation(&mut supervisor);
    for session in ["terminal-1", "terminal-2", "terminal-3", "terminal-4"] {
        assert!(stopped.contains(&format!("\"{session}\"")), "{stopped}");
    }
    assert!(!stopped.contains("\"terminal-5\""), "{stopped}");
}

#[test]
fn edit_from_files_runs_the_configured_editor_in_its_tab_and_hides_files() {
    let window = PairWindow::new();
    // A stand-in editor records its arguments and stays open, as an editor does.
    let record = window.root.join("editor.log");
    let editor = window.root.join("fake-editor");
    executable(
        &editor,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
            record.display()
        ),
    );
    let settings = window.config.join("config.toml");
    let base = fs::read_to_string(&settings).unwrap();
    let configure = |command: &str| {
        fs::write(
            &settings,
            format!("{base}\n[editor]\ncommand = {command}\n"),
        )
        .unwrap()
    };
    configure(&format!(r#"["{}", "--wait"]"#, editor.display()));
    let mut supervisor = window.spawn(&[]);
    window.pair_launch("terminal-2");
    let picked = window.eon(&[
        "tab",
        "directory",
        "t1",
        "--",
        window.launch.to_str().unwrap(),
    ]);
    assert!(picked.status.success(), "{picked:?}");

    // Alt+Shift+Y shows t1's Files; `eon edit` runs as its terminal, as Yazi's opener does.
    assert!(matches!(
        window.invoke_popup("y-show", "files", Some("u1")),
        Response::Snapshot(snapshot) if snapshot.tabs[0].selected_popup.as_deref() == Some("u1")
    ));
    let files = live_identity(&window.generation().join("terminal-2.sock")).process_id;
    let caller = process::environ(files)
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .map(|token| String::from_utf8(token.to_vec()).unwrap())
        .unwrap();
    let file = window.root.join("notes.txt");
    fs::write(&file, "").unwrap();
    let edit = || {
        eon_command(&window.binary)
            .args(["edit", "--json", "--", "notes.txt"])
            .current_dir(&window.root)
            .env("EON_RUNTIME_DIR", &window.runtime)
            .env("EON_CONFIG_HOME", &window.config)
            .env("EON_TERMINAL", &caller)
            .output()
            .unwrap()
    };
    let edited = edit();
    assert!(edited.status.success(), "{}", stdout(&edited));
    // The editor runs with its configured argument and the file's absolute path last.
    assert_eq!(
        wait_for_lines(&record, 2),
        ["--wait", file.to_str().unwrap()]
    );
    // Its pane joins Files' tab, selected and running the editor; Files hides but stays.
    let stable = |mut snapshot: Snapshot| {
        snapshot.tabs = stable_tabs(&snapshot.tabs);
        for terminal in snapshot
            .busy
            .iter_mut()
            .flat_map(|busy| &mut busy.terminals)
        {
            terminal.name.clear();
        }
        snapshot
    };
    let snapshot = stable(window.inspect("after-edit"));
    let tab = &snapshot.tabs[0];
    let pane = tab.panes().find(|pane| pane.id != "p1").unwrap();
    assert_eq!(tab.panes().count(), 2);
    assert_eq!(tab.selected_pane.as_deref(), Some(pane.id.as_str()));
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = process::cmdline(live_identity(endpoint).process_id);
    assert!(
        orbit_arguments.ends_with(&[b"--wait\0", file.as_os_str().as_bytes(), b"\0"].concat()),
        "{}",
        String::from_utf8_lossy(&orbit_arguments)
    );
    assert_eq!(
        tab.popups
            .iter()
            .map(|popup| popup.entry.as_str())
            .collect::<Vec<_>>(),
        ["files"]
    );
    assert!(tab.selected_popup.is_none(), "the edit hides Files");

    // The editor setting is reread on each edit: a missing one fails and changes nothing.
    configure(r#"["missing-editor"]"#);
    let missing = edit();
    assert_eq!(missing.status.code(), Some(1), "{}", stdout(&missing));
    assert!(stdout(&missing).contains("edit-command"));
    assert_eq!(stable(window.inspect("after-missing-editor")), snapshot);
    assert_eq!(log_lines(&record).len(), 2);
    window.stop_generation(&mut supervisor);
}

#[test]
fn closing_a_pending_pair_dismisses_it_and_starts_the_first_pane() {
    let window = PairWindow::new();
    let mut supervisor = window.spawn(&[]);
    let generation = window.generation();
    let control = window.control();
    window.pair_launch("terminal-2");

    // The pair's Esc runs `eon popup hide`.
    let hidden = window.eon(&["popup", "hide", "u1"]);
    assert!(hidden.status.success(), "{hidden:?}");
    let snapshot = window.inspect("dismissed");
    let tab = &snapshot.tabs[0];
    assert!(!tab.pending && tab.selected_popup.is_none());
    assert_eq!(tab.directory, window.launch.as_os_str().as_bytes());
    assert_eq!(
        tab.panes()
            .map(|pane| pane.session.as_str())
            .collect::<Vec<_>>(),
        ["terminal-1"]
    );
    assert_eq!(tab.popups[0].session, "terminal-2");
    assert!(generation.join("terminal-2.sock").exists());
    assert!(!artifact_path(&generation.join("terminal-2.sock"), ".pending").exists());
    assert_eq!(
        wait_for_lines(&window.panes_log, 1),
        [format!("terminal-1|{}", window.launch.display())]
    );

    // A pending pair that exits is dismissed too.
    assert!(window.eon(&["tab", "create"]).status.success());
    window.pair_launch("terminal-3");
    fs::write(window.root.join("files-exit.terminal-3"), "").unwrap();
    let exited = wait_for_snapshot(&control, "pair-exit", |snapshot| {
        snapshot.tabs.len() == 2 && !snapshot.tabs[1].pending
    });
    assert!(exited.tabs[1].popups.is_empty());
    assert_eq!(exited.tabs[1].panes().count(), 1);
    assert!(
        wait_for_lines(&window.panes_log, 2)
            .contains(&format!("terminal-4|{}", window.home.display()))
    );

    // Closing a pending tab stops its pair and starts no pane.
    assert!(window.eon(&["tab", "create"]).status.success());
    window.pair_launch("terminal-5");
    assert!(matches!(
        workspace_action(&control, "close-pending", Action::CloseTab { tab: "t3".into() }),
        Response::Snapshot(snapshot) if snapshot.tabs.len() == 2 && snapshot.active_tab == "t2"
    ));
    wait_for_process_removal_of(&generation.join("terminal-5.sock"));
    assert_eq!(log_lines(&window.panes_log).len(), 2);

    // Presentation detachment dismisses a pending pair without stopping it.
    assert!(window.eon(&["tab", "create"]).status.success());
    window.pair_launch("terminal-6");
    wait_for(&window.venus_pid);
    let venus: i32 = fs::read_to_string(&window.venus_pid)
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: the PID came from this test's live Venus child.
    assert_eq!(unsafe { libc::kill(venus, libc::SIGTERM) }, 0);
    let detached = wait_for_snapshot(&control, "detached", |snapshot| {
        snapshot.tabs.len() == 3 && !snapshot.tabs[2].pending
    });
    assert_eq!(detached.tabs[2].popups[0].session, "terminal-6");
    assert_eq!(detached.tabs[2].panes().count(), 1);
    assert!(generation.join("terminal-6.sock").exists());
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(window.visits().is_empty(), "closing adds no Zoxide history");

    // A tab whose pair cannot stop stays, and a later close finishes it.
    fs::write(&window.fail_stop_once, "terminal-2").unwrap();
    assert!(window.eon(&["tab", "switch", "t1"]).status.success());
    assert!(matches!(
        workspace_action(&control, "close-t1-partial", Action::CloseTab { tab: "t1".into() }),
        Response::Failure(failure)
            if failure.code == "tab-close-failed" && failure.detail.contains("injected stop failure")
    ));
    assert!(matches!(
        workspace_action(&control, "close-t1-partial", Action::CloseTab { tab: "t1".into() }),
        Response::Failure(failure) if failure.code == "duplicate-request"
    ));
    let closed = window.eon(&["tab", "close", "t1", "--json"]);
    assert!(closed.status.success(), "{}", stdout(&closed));

    // A dismissal whose directory vanished stops the pair and drops its tab.
    assert!(window.eon(&["tab", "create"]).status.success());
    window.pair_launch("terminal-8");
    fs::remove_dir(&window.home).unwrap();
    let hidden = window.eon(&["popup", "hide", "u5"]);
    assert!(!hidden.status.success(), "{hidden:?}");
    let failed = wait_for_snapshot(&control, "vanished", |snapshot| {
        snapshot.tabs.len() == 2 && pending_pair(snapshot).is_none()
    });
    assert!(failed.tabs.iter().all(|tab| tab.id != "t5"));
    wait_for_process_removal_of(&generation.join("terminal-8.sock"));
    assert!(supervisor.child.try_wait().unwrap().is_none());
    window.stop_generation(&mut supervisor);
}

/// Waits until the terminal endpoint at `path` is gone.
fn wait_for_process_removal_of(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() {
        assert!(Instant::now() < deadline, "{} remained", path.display());
        thread::sleep(Duration::from_millis(10));
    }
}

/// A stand-in for a shown quick search's fzf: it accepts on the pair socket
/// and answers each POST, keeping its body.
struct QuickSearch {
    path: PathBuf,
    posts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl QuickSearch {
    fn listen(path: &Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let posts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = {
            let (posts, done) = (posts.clone(), done.clone());
            thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    // macOS refuses the timeout on a connection whose peer
                    // already closed, as a probe's may have.
                    if stream
                        .set_read_timeout(Some(Duration::from_millis(500)))
                        .is_err()
                    {
                        continue;
                    }
                    let mut request = Vec::new();
                    let mut buffer = [0; 256];
                    // A probe connects and closes without a request.
                    while let Ok(count @ 1..) = stream.read(&mut buffer) {
                        request.extend_from_slice(&buffer[..count]);
                        if request.ends_with(b"become(exit 11)") {
                            let _ =
                                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                            posts
                                .lock()
                                .unwrap()
                                .push(String::from_utf8_lossy(&request).into_owned());
                            break;
                        }
                    }
                }
            })
        };
        Self {
            path: path.to_path_buf(),
            posts,
            done,
            server: Some(server),
        }
    }

    fn wait_for_posts(&self, count: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let posts = self.posts.lock().unwrap().clone();
            if posts.len() >= count {
                return posts;
            }
            assert!(Instant::now() < deadline, "posts: {posts:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for QuickSearch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn hotkeys_switch_the_pair_screens_or_hide_it() {
    let window = PairWindow::new();
    let mut supervisor = window.spawn(&[]);
    let launch = window.pair_launch("terminal-2");
    let id = launch.client_id();
    let socket = window.generation().join("terminal-2.quick.sock");
    let emit = format!("emit-to {id} plugin quick-search");
    assert!(
        window
            .eon(&[
                "tab",
                "directory",
                "t1",
                "--",
                window.launch.to_str().unwrap()
            ])
            .status
            .success()
    );
    let shown = |snapshot: &Response| match snapshot {
        Response::Snapshot(snapshot) => snapshot.tabs[0].selected_popup.as_deref() == Some("u1"),
        response => panic!("{response:?}"),
    };

    // Project names the tab's Files pair as its observed instance.
    assert!(matches!(
        window.invoke_popup("project-stale", "project", None),
        Response::Failure(failure) if failure.code == "stale-popup"
    ));
    // Alt+Z on the hidden pair shows it and asks it for quick search.
    assert!(shown(&window.invoke_popup("z-show", "project", Some("u1"))));
    assert_eq!(
        wait_for_lines(&window.ya_log, 1),
        std::slice::from_ref(&emit)
    );

    // While quick search is up, Alt+Z hides the pair and never emits again.
    let quick = QuickSearch::listen(&socket);
    assert!(!shown(&window.invoke_popup(
        "z-hide",
        "project",
        Some("u1")
    )));
    // Alt+Shift+Y shows the hidden pair on Files, leaving quick search.
    assert!(shown(&window.invoke_popup("y-show", "files", Some("u1"))));
    assert!(quick.wait_for_posts(1)[0].starts_with("POST / HTTP/1.1\r\n"));
    // On quick search, Alt+Shift+Y switches to Files and keeps the pair shown.
    assert!(shown(&window.invoke_popup("y-switch", "files", Some("u1"))));
    assert_eq!(quick.wait_for_posts(2).len(), 2);
    drop(quick);

    // On Files, Alt+Shift+Y hides the pair; on a hidden Files it shows it as is.
    assert!(!shown(&window.invoke_popup("y-hide", "files", Some("u1"))));
    assert!(shown(&window.invoke_popup(
        "y-show-files",
        "files",
        Some("u1")
    )));
    // On Files, Alt+Z switches to quick search and keeps the pair shown.
    assert!(shown(&window.invoke_popup(
        "z-switch",
        "project",
        Some("u1")
    )));
    assert_eq!(
        wait_for_lines(&window.ya_log, 2),
        [emit.clone(), emit.clone()]
    );
    thread::sleep(Duration::from_millis(300));
    assert_eq!(log_lines(&window.ya_log).len(), 2);

    // Both keys work on a pending tab; its own screen's key dismisses it. A
    // new tab starts on quick search, which counts as shown while its fzf is
    // still starting: Alt+Shift+Y waits for it and switches to Files.
    assert!(window.eon(&["tab", "create"]).status.success());
    let pending = window.pair_launch("terminal-3");
    let pending_socket = window.generation().join("terminal-3.quick.sock");
    let still_pending = |snapshot: &Response| match snapshot {
        Response::Snapshot(snapshot) => {
            snapshot.tabs[1].pending && snapshot.tabs[1].selected_popup.as_deref() == Some("u2")
        }
        response => panic!("{response:?}"),
    };
    assert!(still_pending(&window.invoke_popup(
        "pending-y",
        "files",
        Some("u2")
    )));
    thread::sleep(Duration::from_millis(300));
    let starting = QuickSearch::listen(&pending_socket);
    assert!(starting.wait_for_posts(1)[0].starts_with("POST / HTTP/1.1\r\n"));
    drop(starting);
    assert_eq!(log_lines(&window.ya_log).len(), 2);
    // On Files, Alt+Z asks for quick search; while it starts, Alt+Z keeps
    // the pair shown and asks for nothing more; once it is up, it dismisses.
    assert!(still_pending(&window.invoke_popup(
        "pending-z",
        "project",
        Some("u2")
    )));
    assert_eq!(
        wait_for_lines(&window.ya_log, 3)[2],
        format!("emit-to {} plugin quick-search", pending.client_id())
    );
    assert!(still_pending(&window.invoke_popup(
        "pending-z-starting",
        "project",
        Some("u2")
    )));
    let up = QuickSearch::listen(&pending_socket);
    assert!(matches!(
        window.invoke_popup("pending-z-dismiss", "project", Some("u2")),
        Response::Snapshot(snapshot)
            if !snapshot.tabs[1].pending && snapshot.tabs[1].panes().count() == 1
    ));
    drop(up);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(log_lines(&window.ya_log).len(), 3);
    window.stop_generation(&mut supervisor);
}

/// A quick search closed with Esc before its fzf answered a key stops
/// counting as starting once its pair is hidden: the next Alt+Z asks for
/// quick search again, and Alt+Shift+Y shows Files without leaving a quick
/// search that is not coming (EON-C18).
#[test]
fn hiding_a_starting_quick_search_lets_the_next_key_ask_again() {
    let window = PairWindow::new();
    let mut supervisor = window.spawn(&[]);
    let launch = window.pair_launch("terminal-2");
    let emit = format!("emit-to {} plugin quick-search", launch.client_id());
    let socket = window.generation().join("terminal-2.quick.sock");
    assert!(
        window
            .eon(&[
                "tab",
                "directory",
                "t1",
                "--",
                window.launch.to_str().unwrap()
            ])
            .status
            .success()
    );
    let shown = |snapshot: &Response| match snapshot {
        Response::Snapshot(snapshot) => snapshot.tabs[0].selected_popup.as_deref() == Some("u1"),
        response => panic!("{response:?}"),
    };

    // Alt+Z, Esc (the pair's `eon popup hide`), Alt+Z, all within 2 s.
    assert!(shown(&window.invoke_popup("z-show", "project", Some("u1"))));
    assert!(window.eon(&["popup", "hide", "u1"]).status.success());
    assert!(shown(&window.invoke_popup(
        "z-again",
        "project",
        Some("u1")
    )));
    assert_eq!(
        wait_for_lines(&window.ya_log, 2),
        [emit.clone(), emit.clone()]
    );
    // Esc, then Alt+Shift+Y: Files shows and no fzf is asked to leave.
    assert!(window.eon(&["popup", "hide", "u1"]).status.success());
    assert!(shown(&window.invoke_popup("y-show", "files", Some("u1"))));
    let quick = QuickSearch::listen(&socket);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(quick.posts.lock().unwrap().len(), 0);
    drop(quick);
    window.stop_generation(&mut supervisor);
}

/// The window's first pair plays the startup Anima before its Yazi starts,
/// so its quick search can listen more than 2 s after launch. Alt+Shift+Y
/// meanwhile keeps the pending tab and its pair shown, then switches the pair
/// to Files once quick search listens (EON-C18).
#[test]
fn alt_shift_y_during_the_startup_anima_waits_for_quick_search() {
    let (window, mut supervisor) = anima_window();
    let socket = window.generation().join("terminal-2.quick.sock");

    // The 3 s Anima is still playing.
    thread::sleep(Duration::from_millis(2300));
    assert!(still_pending(&window.invoke_popup(
        "anima-y",
        "files",
        Some("u1")
    )));
    thread::sleep(Duration::from_millis(1000));
    let quick = QuickSearch::listen(&socket);
    assert!(quick.wait_for_posts(1)[0].starts_with("POST / HTTP/1.1\r\n"));
    drop(quick);
    assert!(pending_pair(&window.inspect("after-anima")).is_some());
    window.stop_generation(&mut supervisor);
}

/// Alt+Z while the startup Anima plays shows the first tab's pair on the
/// quick search that is still starting: it neither dismisses the pending tab
/// nor asks for a second quick search. Once quick search is up, Alt+Z on it
/// dismisses the tab (EON-C18).
#[test]
fn alt_z_during_the_startup_anima_keeps_the_pending_tab_on_quick_search() {
    let (window, mut supervisor) = anima_window();
    let socket = window.generation().join("terminal-2.quick.sock");
    for (id, wait) in [("anima-z-early", 300), ("anima-z-late", 2000)] {
        thread::sleep(Duration::from_millis(wait));
        assert!(
            still_pending(&window.invoke_popup(id, "project", Some("u1"))),
            "{id}"
        );
    }
    let quick = QuickSearch::listen(&socket);
    assert!(matches!(
        window.invoke_popup("anima-z-dismiss", "project", Some("u1")),
        Response::Snapshot(snapshot)
            if !snapshot.tabs[0].pending && snapshot.tabs[0].panes().count() == 1
    ));
    drop(quick);
    assert!(log_lines(&window.ya_log).is_empty());
    window.stop_generation(&mut supervisor);
}

/// A window whose first pair plays a 3 s startup Anima, once that pair started.
fn anima_window() -> (PairWindow, TestProcess) {
    let window = PairWindow::new();
    fs::write(
        window.config.join("config.toml"),
        "[terminal]\nfont_fallbacks = []\n\n[anima]\nenabled = true\nduration_seconds = 3\n",
    )
    .unwrap();
    // Anima waits for a ready Venus.
    executable(
        &window.root.join("venus"),
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$EON_TEST_VENUS_PID\"\nprintf ready-v1\ncat >/dev/null\n",
    );
    let supervisor = window.spawn(&[]);
    let launch = window.pair_launch("terminal-2");
    assert!(
        launch.arguments.starts_with(&["--eon-anima".into()]),
        "{:?}",
        launch.arguments
    );
    (window, supervisor)
}

/// Whether t1 is still pending with its pair `u1` shown.
fn still_pending(snapshot: &Response) -> bool {
    match snapshot {
        Response::Snapshot(snapshot) => {
            snapshot.tabs[0].pending && snapshot.tabs[0].selected_popup.as_deref() == Some("u1")
        }
        response => panic!("{response:?}"),
    }
}

#[test]
fn recovery_stops_a_pending_pair_and_keeps_a_committed_files() {
    for committed in [false, true] {
        let window = PairWindow::new();
        let mut first = window.spawn(&[]);
        let generation = window.generation();
        // The supervisor holds the pair's lease once it answers.
        assert!(pending_pair(&window.inspect("started")).is_some());
        let pair = live_identity(&generation.join("terminal-2.sock"));
        if committed {
            assert!(
                window
                    .eon(&[
                        "tab",
                        "directory",
                        "t1",
                        "--",
                        window.home.to_str().unwrap()
                    ])
                    .status
                    .success()
            );
            live_identity(&generation.join("orbit.sock"));
        }
        window.crash(&mut first);

        let mut replacement = window.spawn(&[]);
        let recovered = window.inspect("recovered");
        assert!(pending_pair(&recovered).is_none());
        assert!(recovered.tabs[0].popups.is_empty());
        let sessions = recovered.tabs[0]
            .panes()
            .map(|pane| pane.session.as_str())
            .collect::<Vec<_>>();
        if committed {
            // A committed tab's Files recovers as today's Files does, as a pane.
            assert_eq!(sessions, ["terminal-1", "terminal-2"]);
            assert!(process::exists(pair.process_id));
        } else {
            // A still-pending pair is stopped and the launch directory's pane starts.
            assert_eq!(sessions, ["terminal-1"]);
            wait_for_process_removal(pair.process_id, "the pending pair was adopted");
            assert!(
                wait_for_lines(&window.panes_log, 1)
                    .contains(&format!("terminal-1|{}", window.launch.display()))
            );
        }
        assert!(!artifact_path(&generation.join("terminal-2.sock"), ".pending").exists());
        window.stop_generation(&mut replacement);
    }
}

#[test]
fn a_pending_pair_left_by_a_failed_rollback_is_stopped_by_recovery() {
    let window = PairWindow::new();
    // The pair publishes Ready after its start and rollback deadlines, so its
    // failed start cannot confirm that rollback stopped it.
    let mut command = window.command(&[]);
    command.env("EON_TEST_PAIR_READY_DELAY_MS", "5100");
    let mut first = window.start(command);
    let generation = window.generation();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !generation.join("orbit.sock").exists() {
        assert!(Instant::now() < deadline, "the fallback pane did not start");
        thread::sleep(Duration::from_millis(10));
    }
    wait_for_ready(&window.control());
    let pair = live_identity(&generation.join("terminal-2.sock"));
    let marker = artifact_path(&generation.join("terminal-2.sock"), ".pending");
    let marked = marker.exists();

    window.crash(&mut first);
    let mut replacement = window.spawn(&[]);
    let recovered = window.inspect("recovered");
    assert_eq!(recovered.tabs.len(), 1);
    assert!(recovered.tabs[0].popups.is_empty());
    assert_eq!(
        recovered.tabs[0]
            .panes()
            .map(|pane| pane.session.as_str())
            .collect::<Vec<_>>(),
        ["terminal-1"],
        "recovery adopted the failed pair as a pane"
    );
    wait_for_process_removal(pair.process_id, "recovery left the failed pair running");
    assert!(marked, "the failed start cleared the pair's marker");
    assert!(!marker.exists());
    window.stop_generation(&mut replacement);
}

#[test]
fn a_failing_test_closes_its_terminals_before_reaping_their_child() {
    let fail_once = || {
        let (leader, device) = pseudo_terminal();
        let mut command = Command::new("sleep");
        command
            .arg("30")
            .stdin(Stdio::from(device))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: only the child acquires this fresh private controlling terminal, and
        // a hangup ends it whatever this process ignores.
        unsafe {
            command.pre_exec(|| {
                if libc::signal(libc::SIGHUP, libc::SIG_DFL) == libc::SIG_ERR
                    || libc::setsid() == -1
                    || libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = TerminalChild::spawn(&mut command, vec![leader]);
        let pid = child.guard.0.id();
        // Unread echo, which macOS drains before the killed session leader can exit.
        child.terminals[0].write_all(b"unread\n").unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _child = child;
            panic!("failing test");
        }));
        let reaped = UNWOUND
            .lock()
            .unwrap()
            .iter()
            .find(|(reaped, _)| *reaped == pid)
            .map(|(_, status)| *status);
        let pid = i32::try_from(pid).unwrap();
        // SAFETY: WNOHANG only reports, and a child still waiting here keeps its PID.
        if unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == 0 {
            // SAFETY: the child is still unreaped, so its PID and group are its own.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
        }
        assert_eq!(
            unwound.unwrap_err().downcast_ref::<&str>(),
            Some(&"failing test")
        );
        reaped.expect("the guard did not reap the child")
    };
    // Linux hangs the session up as the last master closes, so a child the guard
    // kills only afterwards has already ended by that hangup. Another thread's
    // spawn can hold a copy of the master between its fork and exec, which delays
    // the hangup past the kill, so one hangup in a few tries is enough.
    let statuses = (0..20)
        .map(|_| fail_once())
        .take_while(|status| cfg!(target_os = "linux") && status.signal() != Some(libc::SIGHUP))
        .collect::<Vec<_>>();
    assert!(statuses.len() < 20, "{statuses:?}");
}

#[test]
fn update_check_reports_a_profile_entry_as_one_json_object_without_changing_it() {
    let root = temporary_directory();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let platform = if cfg!(target_os = "macos") {
        "aarch64-darwin"
    } else {
        "x86_64-linux"
    };
    let versions = Command::new(binary).arg("versions").output().unwrap();
    let build = stdout(&versions)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_owned();
    let (old, new) = ("1".repeat(40), "2".repeat(40));
    // A product package holding a copy of this eon, selected by a default profile.
    let package = root.join("store/eon");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::create_dir_all(package.join("share")).unwrap();
    executable(&package.join("bin/eon"), fs::read(binary).unwrap());
    let attribute = format!("packages.{platform}.default");
    fs::write(
        package.join("share/eon-source-receipt.json"),
        serde_json::json!({
            "schema_version": 1,
            "repository": "https://github.com/Yazelix/eon",
            "revision": old,
            "build": build,
            "platform": platform,
            "attribute": attribute,
        })
        .to_string(),
    )
    .unwrap();
    let environment = root.join("store/profile-env");
    fs::create_dir_all(environment.join("bin")).unwrap();
    fs::write(environment.join("manifest.json"), "{}").unwrap();
    symlink(package.join("bin/eon"), environment.join("bin/eon")).unwrap();
    fs::create_dir(root.join("profiles")).unwrap();
    symlink(&environment, root.join("profiles/profile-1-link")).unwrap();
    symlink("profile-1-link", root.join("profiles/profile")).unwrap();
    let profile = root.join("profiles/profile");
    // Eon hands the manager the profile link with its directory's aliases
    // resolved (update.rs `profile_location`); on macOS /tmp is one.
    let location = fs::canonicalize(root.join("profiles"))
        .unwrap()
        .join("profile");
    fs::create_dir(root.join("home")).unwrap();
    symlink(&profile, root.join("home/.nix-profile")).unwrap();
    let listing = serde_json::json!({"version": 3, "elements": {"eon": {
        "active": true, "priority": 5, "outputs": null, "attrPath": attribute,
        "originalUrl": "github:Yazelix/eon/edge",
        "url": format!("github:Yazelix/eon/{old}?narHash=sha256-x"),
        "storePaths": [package],
    }}});
    fs::write(root.join("listing.json"), listing.to_string()).unwrap();
    let stubs = root.join("stubs");
    fs::create_dir(&stubs).unwrap();
    let log = root.join("calls");
    executable(
        &stubs.join("nix"),
        format!(
            "#!/bin/sh\necho \"nix $*\" >> '{}'\ncat '{}'\n",
            log.display(),
            root.join("listing.json").display()
        ),
    );
    let curl = root.join("pinned-curl");
    executable(
        &curl,
        format!(
            "#!/bin/sh\nfor url; do :; done\necho \"curl $*\" >> '{}'\ncase \"$url\" in\n\
             */commits/edge) printf '{new}\\n200' ;;\n\
             */compare/{old}...{new}*) printf '{{\"status\":\"ahead\"}}\\n200' ;;\n\
             *) exit 7 ;;\nesac\n",
            log.display()
        ),
    );
    // PATH's curl is never asked.
    executable(&stubs.join("curl"), "#!/bin/sh\nexit 99\n");
    let path = std::iter::once(stubs.clone())
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()))
        .collect::<Vec<_>>();
    let check = |program: &Path, arguments: &[&OsStr]| {
        eon_command(program)
            .arg("update")
            .args(arguments)
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("USER", "eon-update-test-nobody")
            .env("EON_CURL", &curl)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let report = |output: &Output| -> serde_json::Value {
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
    };
    let explicit = [
        "--check".as_ref(),
        "--json".as_ref(),
        "--profile".as_ref(),
        profile.as_os_str(),
        "--entry".as_ref(),
        "eon".as_ref(),
    ];

    let output = check(binary, &explicit);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let explicit_report = report(&output);
    assert_eq!(explicit_report["install_kind"], "nix-profile");
    assert_eq!(explicit_report["source"]["policy"], "edge");
    assert_eq!(explicit_report["availability"]["state"], "available");
    assert_eq!(
        explicit_report["availability"]["candidate"]["revision"],
        new
    );
    assert_eq!(explicit_report["capability"]["install"], true);
    assert_eq!(explicit_report["selection"], serde_json::Value::Null);
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains(&format!(
        "nix profile list --json --profile {}",
        profile.display()
    )));
    assert!(calls.contains("curl -q --silent --show-error --proto =https --max-redirs 0"));
    let count = |calls: &str, program| {
        calls
            .lines()
            .filter(|line| line.starts_with(program))
            .count()
    };
    assert_eq!(
        (count(&calls, "nix "), count(&calls, "curl ")),
        (1, 2),
        "{calls}"
    );
    // Check changed no selector.
    assert_eq!(
        fs::read_link(&profile).unwrap(),
        Path::new("profile-1-link")
    );

    // Started through the default profile, the copy finds that entry itself.
    let output = check(
        &root.join("home/.nix-profile/bin/eon"),
        &["--check".as_ref(), "--json".as_ref()],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let discovered = report(&output);
    assert_eq!(discovered["install_kind"], "nix-profile");
    assert_eq!(
        discovered["target"],
        serde_json::json!({"profile": root.join("home/.nix-profile"), "entry": "eon"})
    );
    assert_eq!(discovered["installed"]["build"], build);
    assert_eq!(discovered["invoked"]["revision"], old);
    assert_eq!(discovered["availability"]["state"], "available");

    let output = eon_command(binary)
        .args(["update", "--check", "--json"])
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("EON_CONFIG_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(report(&output)["capability"]["reason"], "install-ambiguous");

    // A personal symlink to the profile's launcher still runs through the profile.
    fs::create_dir_all(root.join("home/.local/bin")).unwrap();
    symlink(
        root.join("home/.nix-profile/bin/eon"),
        root.join("home/.local/bin/eon"),
    )
    .unwrap();
    let output = check(
        &root.join("home/.local/bin/eon"),
        &["--check".as_ref(), "--json".as_ref()],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(report(&output)["target"], discovered["target"]);

    // The same output started directly, as `nix run` does, belongs to no installation.
    let output = check(
        &package.join("bin/eon"),
        &["--check".as_ref(), "--json".as_ref()],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(report(&output)["install_kind"], "unmanaged");

    // Install binds consent to the exact name and source and reads back the manager result.
    let selected_b = root.join("store/eon-b");
    fs::create_dir_all(selected_b.join("bin")).unwrap();
    fs::create_dir_all(selected_b.join("share")).unwrap();
    fs::copy(binary, selected_b.join("bin/eon")).unwrap();
    let mut receipt_b: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("share/eon-source-receipt.json")).unwrap())
            .unwrap();
    receipt_b["revision"] = new.clone().into();
    receipt_b["build"] = "g1-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    fs::write(
        selected_b.join("share/eon-source-receipt.json"),
        receipt_b.to_string(),
    )
    .unwrap();
    let environment_b = root.join("store/profile-env-b");
    fs::create_dir_all(environment_b.join("bin")).unwrap();
    fs::write(environment_b.join("manifest.json"), "{}").unwrap();
    symlink(selected_b.join("bin/eon"), environment_b.join("bin/eon")).unwrap();
    symlink(&environment_b, root.join("profiles/profile-2-link")).unwrap();
    let mut install_a = listing.clone();
    let mut entry = install_a["elements"]
        .as_object_mut()
        .unwrap()
        .remove("eon")
        .unwrap();
    install_a["elements"]["eon+x"] = entry.clone();
    install_a["elements"]["eonx"] =
        serde_json::json!({"storePaths": [], "active": true, "priority": 7});
    entry["url"] = format!("github:Yazelix/eon/{new}?narHash=sha256-b").into();
    entry["storePaths"] = serde_json::json!([selected_b]);
    let mut install_b = install_a.clone();
    install_b["elements"]["eon+x"] = entry;
    fs::write(root.join("before.json"), install_a.to_string()).unwrap();
    fs::write(root.join("after.json"), install_b.to_string()).unwrap();
    fs::write(root.join("listing.json"), install_a.to_string()).unwrap();
    let install = [
        "--install".as_ref(),
        "--json".as_ref(),
        "--yes".as_ref(),
        "--profile".as_ref(),
        profile.as_os_str(),
        "--entry".as_ref(),
        "eon+x".as_ref(),
    ];
    executable(
        &stubs.join("nix"),
        format!(
            r#"#!/bin/sh
case "$1 $2" in
  '--version ') if [ -f '{r}/nix-version' ]; then cat '{r}/nix-version'; else echo 'nix (Nix) 2.34.1'; fi; exit ;;
  'profile list') if [ "$(cat '{r}/mode')" = read-cancel ]; then echo $$ > '{r}/read-manager-pid'; exec sleep 30; fi; cat '{r}/listing.json'; exit ;;
  'profile upgrade')
    printf '%s\n' "$@" > '{r}/upgrade-argv'
    echo 'manager output goes to stderr'
    case "$(cat '{r}/mode')" in
      fail) echo 'build failed' >&2; exit 1 ;;
      cancel) trap '' TERM; sleep 30 & echo $! > '{r}/sleep-pid'; echo $$ > '{r}/manager-pid'; wait ;;
      hard-kill) echo $$ > '{r}/manager-pid'; exec sleep 30 ;;
      unreadable) rm '{r}/listing.json'; exit 1 ;;
    esac
    cp '{r}/after.json' '{r}/listing.json'
    ln -sfn profile-2-link '{p}'
    if [ "$(cat '{r}/mode')" = late-fail ]; then exit 1; fi ;;
  *) exit 99 ;;
esac
"#,
            r = root.display(),
            p = profile.display()
        ),
    );
    fs::write(root.join("mode"), "success").unwrap();
    let without_yes = install
        .iter()
        .copied()
        .filter(|word| *word != OsStr::new("--yes"))
        .collect::<Vec<_>>();
    let output = check(binary, &without_yes);
    assert_eq!(
        report(&output)["error"]["code"],
        "not-confirmed",
        "{output:?}"
    );
    let prompt = without_yes
        .iter()
        .copied()
        .filter(|word| *word != OsStr::new("--json"))
        .collect::<Vec<_>>();
    for ending in [
        "decline",
        "interrupt",
        "stopped",
        "kill",
        "no-controlling-terminal",
        "different-terminal",
        "unreadable-terminal",
    ] {
        let (leader, device) = pseudo_terminal();
        if ending == "unreadable-terminal" {
            assert_eq!(unsafe { libc::fchmod(device.as_raw_fd(), 0) }, 0);
        }
        let mut other = (ending == "different-terminal").then(pseudo_terminal);
        let controlling = other
            .as_ref()
            .map_or(libc::STDIN_FILENO, |(_, device)| device.as_raw_fd());
        if let Some((leader, _)) = other.as_mut() {
            leader.write_all(b"y\n").unwrap();
        }
        let flags = status_flags(device.as_raw_fd());
        assert_eq!(flags & libc::O_NONBLOCK, 0);
        let mut command = eon_command(binary);
        command
            .arg("update")
            .args(&prompt)
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_CURL", &curl)
            .stdin(Stdio::from(device.try_clone().unwrap()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Only this private PTY becomes the child's controlling terminal.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if ending != "no-controlling-terminal"
                    && libc::ioctl(controlling, libc::TIOCSCTTY as _, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let (other, _other_device) = other.unzip();
        let terminals = std::iter::once(leader).chain(other).collect();
        let mut child = TerminalChild::spawn(&mut command, terminals);
        let mut err = child.guard.0.stderr.take().unwrap();
        let mut diagnostics = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        if !matches!(ending, "no-controlling-terminal" | "different-terminal") {
            while !diagnostics.ends_with(b"[y/N] ") {
                assert!(Instant::now() < deadline, "no question: {diagnostics:?}");
                let mut polled = libc::pollfd {
                    fd: err.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut polled, 1, 100) } > 0 {
                    let mut bytes = [0; 256];
                    let count = err.read(&mut bytes).unwrap();
                    assert!(count > 0, "helper exited before question: {diagnostics:?}");
                    diagnostics.extend_from_slice(&bytes[..count]);
                }
            }
            if ending == "stopped" {
                unsafe {
                    libc::kill(child.guard.0.id() as i32, libc::SIGSTOP);
                }
            }
            for _ in 0..10 {
                assert_eq!(
                    status_flags(device.as_raw_fd()),
                    flags,
                    "{ending}: inherited stdin flags changed during prompt"
                );
                thread::sleep(Duration::from_millis(20));
            }
            match ending {
                "interrupt" => unsafe {
                    libc::kill(child.guard.0.id() as i32, libc::SIGINT);
                },
                "kill" => child.guard.0.kill().unwrap(),
                _ => {
                    if ending == "stopped" {
                        unsafe {
                            libc::kill(child.guard.0.id() as i32, libc::SIGCONT);
                        }
                    }
                    child.terminals[0].write_all(b"n\n").unwrap();
                }
            }
        }
        let status = child
            .exit(deadline)
            .unwrap_or_else(|| panic!("{ending}: helper did not finish"));
        assert_eq!(
            status_flags(device.as_raw_fd()),
            flags,
            "{ending}: inherited stdin flags changed after exit"
        );
        let mut bytes = Vec::new();
        child
            .guard
            .0
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        err.read_to_end(&mut diagnostics).unwrap();
        let output = Output {
            status,
            stdout: bytes,
            stderr: diagnostics,
        };
        match ending {
            "kill" => assert_eq!(status.signal(), Some(libc::SIGKILL)),
            "no-controlling-terminal" | "different-terminal" => {
                assert_eq!(status.code(), Some(1), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("controlling terminal"),
                    "{output:?}"
                );
                assert!(!output.stderr.windows(6).any(|word| word == b"[y/N] "));
            }
            _ => {
                assert_eq!(
                    status.code(),
                    Some(if ending == "interrupt" { 130 } else { 0 }),
                    "{output:?}"
                );
                assert!(
                    stdout(&output).contains("Selection: unchanged"),
                    "{output:?}"
                );
            }
        }
        assert!(!root.join("upgrade-argv").exists());
        assert_eq!(
            fs::read_to_string(root.join("listing.json")).unwrap(),
            install_a.to_string()
        );
    }
    // Commands must reach the product dispatcher and leave stdin in the same
    // controlling-terminal session for Install's own consent, then retain its result.
    let commands = root.join("eon-commands");
    symlink(binary, &commands).unwrap();
    symlink(binary, stubs.join("eon")).unwrap();
    let config = root.join("popup-config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        format!(
            "[updates]\nnix_profile = {:?}\nnix_entry = \"eon+x\"\n",
            profile.to_str().unwrap()
        ),
    )
    .unwrap();
    for (mode, answer, manager, interrupt, result, status) in [
        ("--check", None, "success", None, "checked", 0),
        ("--install", Some("n\n"), "success", None, "cancelled", 0),
        ("--install", Some(""), "success", Some(3), "failed", 130),
        ("--install", Some(""), "success", Some(28), "failed", 130),
        (
            "--install",
            Some("y\n"),
            "hard-kill",
            Some(3),
            "failed",
            130,
        ),
        (
            "--install",
            Some("y\n"),
            "hard-kill",
            Some(28),
            "failed",
            130,
        ),
        ("--install", Some("y\n"), "fail", None, "failed", 1),
        ("--install", Some("y\n"), "success", None, "installed", 0),
    ] {
        executable(
            &stubs.join("fzf"),
            format!(
                "#!/bin/sh\nwhile IFS=\"$(printf '\\t')\" read -r index rest; do\n\
                 case \"$rest\" in *'eon update {mode}') printf '%s\\tchosen\\n' \"$index\";; esac\ndone\n"
            ),
        );
        fs::write(root.join("mode"), manager).unwrap();
        let (leader, device) = pseudo_terminal();
        let flags = status_flags(device.as_raw_fd());
        let mut command = eon_command(&commands);
        command
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("EON_CONFIG_HOME", &config)
            .env("EON_CURL", &curl)
            .env("EON_TAB", "t1")
            .stdin(Stdio::from(device.try_clone().unwrap()))
            .stdout(Stdio::from(device.try_clone().unwrap()))
            .stderr(Stdio::from(device.try_clone().unwrap()));
        // SAFETY: only the child acquires this fresh private controlling terminal.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1
                    || libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = TerminalChild::spawn(&mut command, vec![leader]);
        let read_until = |leader: &mut fs::File, marker: &[u8]| {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut text = Vec::new();
            while !text.windows(marker.len()).any(|word| word == marker) {
                assert!(
                    Instant::now() < deadline,
                    "{mode}: no output marker: {text:?}"
                );
                let mut polled = libc::pollfd {
                    fd: leader.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut polled, 1, 100) } > 0 {
                    let mut bytes = [0; 4096];
                    let count = leader.read(&mut bytes).unwrap();
                    assert!(count > 0, "{mode}: popup ended: {text:?}");
                    text.extend_from_slice(&bytes[..count]);
                }
            }
            String::from_utf8(text).unwrap()
        };
        if let Some(answer) = answer {
            let prompt = read_until(&mut child.terminals[0], b"[y/N] ");
            assert!(prompt.contains(profile.to_str().unwrap()), "{prompt}");
            assert!(prompt.contains("eon+x"), "{prompt}");
            assert!(!root.join("upgrade-argv").exists());
            child.terminals[0].write_all(answer.as_bytes()).unwrap();
        }
        if let Some(interrupt) = interrupt {
            if manager == "hard-kill" {
                wait_for(&root.join("manager-pid"));
            }
            // Terminal ISIG sends Ctrl+C/Ctrl+\ to the whole foreground group.
            child.terminals[0].write_all(&[interrupt]).unwrap();
        }
        let output = read_until(&mut child.terminals[0], b"Press Enter to close");
        let expected = if mode == "--check" {
            "Update: available".into()
        } else {
            format!("Result: {result}")
        };
        assert!(output.contains(&expected), "{output}");
        if interrupt.is_some() {
            assert!(output.contains("Selection: unchanged"), "{output}");
            assert!(output.contains("interrupted"), "{output}");
        }
        assert!(
            child.guard.0.try_wait().unwrap().is_none(),
            "result was not retained"
        );
        assert_eq!(status_flags(device.as_raw_fd()), flags);
        child.terminals[0].write_all(b"\n").unwrap();
        let exit = Instant::now() + Duration::from_secs(5);
        let exited = child.exit(exit);
        assert_eq!(
            exited.expect("Eon process did not exit").code(),
            Some(status)
        );
        if answer == Some("y\n") {
            assert_eq!(
                fs::read_to_string(root.join("upgrade-argv")).unwrap(),
                format!(
                    "profile\nupgrade\n--profile\n{}\n--refresh\n--\neon+x\n",
                    location.display()
                )
            );
            fs::remove_file(root.join("upgrade-argv")).unwrap();
        } else {
            assert!(!root.join("upgrade-argv").exists());
        }
        if manager == "hard-kill" {
            let pid = fs::read_to_string(root.join("manager-pid")).unwrap();
            assert!(!process::exists(pid.trim().parse().unwrap()));
            fs::remove_file(root.join("manager-pid")).unwrap();
        }
        let selected: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("listing.json")).unwrap()).unwrap();
        assert_eq!(
            &selected,
            if result == "installed" {
                &install_b
            } else {
                &install_a
            }
        );
        fs::write(root.join("listing.json"), install_a.to_string()).unwrap();
        fs::remove_file(&profile).unwrap();
        symlink("profile-1-link", &profile).unwrap();
    }
    let output = check(
        &root.join("home/.nix-profile/bin/eon"),
        &["--install".as_ref(), "--yes".as_ref(), "--json".as_ref()],
    );
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(report(&output)["error"]["code"], "install-ambiguous");
    assert!(!root.join("upgrade-argv").exists());
    let mut pinned = install_a.clone();
    pinned["elements"]["eon+x"]["originalUrl"] = "github:Yazelix/eon/v0.1.21".into();
    fs::write(root.join("listing.json"), pinned.to_string()).unwrap();
    let human = install
        .iter()
        .copied()
        .filter(|word| *word != OsStr::new("--json"))
        .collect::<Vec<_>>();
    let output = check(binary, &human);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert!(stdout(&output).contains("Result: refused"), "{output:?}");
    assert!(
        stdout(&output).contains("Selection: unchanged"),
        "{output:?}"
    );
    fs::write(root.join("listing.json"), install_a.to_string()).unwrap();
    for version in ["nix (Nix) 2.20.9", "nix (Lix, like Nix) 2.93.0"] {
        fs::write(root.join("nix-version"), version).unwrap();
        let output = check(binary, &install);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(report(&output)["error"]["stage"], "resolve");
        assert_eq!(report(&output)["selection"]["state"], "unchanged");
        assert!(!root.join("upgrade-argv").exists());
    }
    fs::remove_file(root.join("nix-version")).unwrap();
    for name in ["1", "+1", "/nix/store/00000000000000000000000000000000-eon"] {
        let mut ambiguous = install_a.clone();
        let element = ambiguous["elements"]
            .as_object_mut()
            .unwrap()
            .remove("eon+x")
            .unwrap();
        ambiguous["elements"][name] = element;
        fs::write(root.join("listing.json"), ambiguous.to_string()).unwrap();
        let mut arguments = install.to_vec();
        *arguments.last_mut().unwrap() = OsStr::new(name);
        let output = check(binary, &arguments);
        assert_eq!(output.status.code(), Some(4), "{output:?}");
        assert_eq!(report(&output)["error"]["code"], "install-ambiguous");
        assert!(!root.join("upgrade-argv").exists());
    }
    fs::write(root.join("listing.json"), install_a.to_string()).unwrap();
    let output = check(binary, &install);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(report(&output)["outcome"], "installed");
    assert_eq!(report(&output)["installed"]["revision"], old);
    assert_eq!(report(&output)["selection"]["selected"]["revision"], new);
    assert_eq!(
        report(&output)["selection"]["source"]["original_ref"],
        "github:Yazelix/eon/edge"
    );
    assert_eq!(
        fs::read_to_string(root.join("upgrade-argv")).unwrap(),
        format!(
            "profile\nupgrade\n--profile\n{}\n--refresh\n--\neon+x\n",
            location.display()
        )
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("manager output goes to stderr"));
    let selected: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("listing.json")).unwrap()).unwrap();
    assert_eq!(selected["elements"]["eonx"], install_a["elements"]["eonx"]);
    let output = check(binary, &install);
    assert_eq!(report(&output)["outcome"], "unchanged", "{output:?}");
    assert_eq!(report(&output)["selection"]["state"], "unchanged");
    let reset = || {
        fs::write(root.join("listing.json"), install_a.to_string()).unwrap();
        fs::remove_file(&profile).unwrap();
        symlink("profile-1-link", &profile).unwrap();
    };
    for (kind, expected_code, expected_state, status) in [
        ("generation", None, "unchanged", 0),
        ("other", Some("selection-failed"), "changed", 1),
        ("only-other", Some("selection-failed"), "unchanged", 1),
        ("source", Some("source-changed"), "changed", 1),
        ("failed-source", Some("source-changed"), "changed", 1),
    ] {
        reset();
        fs::write(
            root.join("mode"),
            if kind == "failed-source" {
                "late-fail"
            } else {
                "success"
            },
        )
        .unwrap();
        fs::remove_file(root.join("profiles/profile-2-link")).unwrap();
        symlink(
            if kind == "generation" || kind == "only-other" {
                &environment
            } else {
                &environment_b
            },
            root.join("profiles/profile-2-link"),
        )
        .unwrap();
        let mut selected = if kind == "generation" || kind == "only-other" {
            install_a.clone()
        } else {
            install_b.clone()
        };
        if kind == "other" || kind == "only-other" {
            selected["elements"]["eonx"]["priority"] = 8.into();
        }
        if kind == "source" || kind == "failed-source" {
            selected["elements"]["eon+x"]["originalUrl"] = "github:Yazelix/eon/main".into();
        }
        fs::write(root.join("after.json"), selected.to_string()).unwrap();
        let output = check(binary, &install);
        assert_eq!(output.status.code(), Some(status), "{kind}: {output:?}");
        assert_eq!(
            report(&output)["selection"]["state"],
            expected_state,
            "{kind}: {output:?}"
        );
        if let Some(code) = expected_code {
            assert_eq!(report(&output)["outcome"], "failed");
            assert_eq!(report(&output)["error"]["code"], code, "{kind}: {output:?}");
            assert_eq!(report(&output)["error"]["stage"], "inspect-result");
            if code == "source-changed" {
                assert_eq!(report(&output)["selection"]["source"]["policy"], "main");
            }
        } else {
            assert_eq!(report(&output)["outcome"], "unchanged");
        }
    }
    fs::remove_file(root.join("profiles/profile-2-link")).unwrap();
    symlink(&environment_b, root.join("profiles/profile-2-link")).unwrap();
    fs::write(root.join("after.json"), install_b.to_string()).unwrap();
    reset();
    fs::write(root.join("mode"), "fail").unwrap();
    let output = check(binary, &install);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report(&output)["error"]["code"], "selection-failed");
    assert_eq!(report(&output)["selection"]["state"], "unchanged");
    fs::write(root.join("mode"), "unreadable").unwrap();
    let output = check(binary, &install);
    assert_eq!(report(&output)["error"]["code"], "selection-failed");
    assert_eq!(report(&output)["selection"]["state"], "unknown");
    reset();
    // A source edit during remote metadata lookup invalidates the displayed snapshot.
    let original_curl = fs::read_to_string(&curl).unwrap();
    let mut changed = install_a.clone();
    changed["elements"]["eon+x"]["originalUrl"] = "github:Yazelix/eon/main".into();
    fs::write(root.join("changed.json"), changed.to_string()).unwrap();
    fs::remove_file(root.join("upgrade-argv")).unwrap();
    executable(
        &curl,
        format!(
            r#"#!/bin/sh
for url; do :; done
case "$url" in
  */commits/edge) printf '{new}\n200' ;;
  *) cp '{r}/changed.json' '{r}/listing.json'; printf '{{"status":"ahead"}}\n200' ;;
esac
"#,
            r = root.display()
        ),
    );
    let output = check(binary, &install);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(report(&output)["error"]["code"], "target-changed");
    assert_eq!(report(&output)["selection"]["source"]["policy"], "main");
    assert!(!root.join("upgrade-argv").exists());
    executable(&curl, original_curl);
    reset();
    fs::write(root.join("mode"), "fail").unwrap();
    // Configured target works from a window's pinned binary, without routing to a window.
    let config = root.join("config");
    fs::create_dir(&config).unwrap();
    let configured = |text: &str, selector: &[&OsStr]| {
        fs::write(config.join("config.toml"), text).unwrap();
        eon_command(binary)
            .args(["update", "--install", "--yes", "--json"])
            .args(selector)
            .env("EON_CONFIG_HOME", &config)
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("EON_CURL", &curl)
            .env("EON_RUNTIME_DIR", root.join("must-not-exist"))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let output = configured("[updates]\nnix_entry = 'eon+x'\n", &[]);
    assert_eq!(report(&output)["error"]["code"], "invalid-configuration");
    let output = configured(
        &format!(
            "[updates]\nnix_profile = '{}'\nnix_entry = 'eon+x'\n",
            profile.display()
        ),
        &[],
    );
    assert_eq!(report(&output)["error"]["code"], "selection-failed");
    for text in [
        "[broken",
        "[updates]\nnix_profile = '/missing/profile'\nnix_entry = 'other'\n",
    ] {
        let output = configured(text, &install[3..]);
        assert_eq!(
            report(&output)["target"]["profile"],
            profile.to_str().unwrap()
        );
        assert_eq!(report(&output)["target"]["entry"], "eon+x");
        assert_eq!(
            report(&output)["error"]["code"],
            "selection-failed",
            "{output:?}"
        );
    }
    assert!(!root.join("must-not-exist").exists());
    let stopped = |files: &[&str]| {
        let pids = files
            .iter()
            .map(|name| {
                fs::read_to_string(root.join(name))
                    .unwrap()
                    .trim()
                    .parse::<i32>()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(3);
        while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
        let leaked = pids
            .into_iter()
            .filter(|pid| unsafe { libc::kill(*pid, 0) } == 0)
            .collect::<Vec<_>>();
        for pid in &leaked {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
        assert!(leaked.is_empty(), "helper left processes {leaked:?}");
    };
    // An interrupt reaches only the child this helper started, then reports selection.
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        let _ = fs::remove_file(root.join("manager-pid"));
        let _ = fs::remove_file(root.join("sleep-pid"));
        fs::write(root.join("mode"), "cancel").unwrap();
        let mut child = eon_command(binary)
            .arg("update")
            .args(install)
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_CURL", &curl)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("manager-pid").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(root.join("manager-pid").exists());
        let output_busy = check(binary, &install);
        assert_eq!(report(&output_busy)["error"]["code"], "operation-busy");
        unsafe {
            libc::kill(child.id() as i32, signal);
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            panic!("cancel did not finish within grace");
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(130), "{output:?}");
        assert_eq!(report(&output)["error"]["code"], "interrupted");
        assert_eq!(report(&output)["selection"]["state"], "unchanged");
        stopped(&["manager-pid", "sleep-pid"]);
    }
    #[cfg(target_os = "linux")]
    {
        fs::remove_file(root.join("manager-pid")).unwrap();
        fs::write(root.join("mode"), "hard-kill").unwrap();
        let mut child = eon_command(binary)
            .arg("update")
            .args(install)
            .env("PATH", std::env::join_paths(&path).unwrap())
            .env("HOME", root.join("home"))
            .env("EON_CURL", &curl)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("manager-pid").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(root.join("manager-pid").exists());
        child.kill().unwrap();
        child.wait().unwrap();
        stopped(&["manager-pid"]);
    }
    // Restore check fixture.
    fs::write(root.join("listing.json"), listing.to_string()).unwrap();

    // Offline is a failed check, never an up-to-date one.
    executable(
        &curl,
        "#!/bin/sh\necho 'Could not resolve host' >&2\nexit 6\n",
    );
    let output = check(binary, &explicit);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let offline = report(&output);
    assert_eq!(offline["error"]["code"], "remote-unavailable");
    assert_eq!(offline["availability"]["state"], "unknown");
    assert_eq!(offline["availability"]["reason"], "offline");
    assert_eq!(offline["capability"]["install"], false);
    // The backend's diagnostic reaches stderr in JSON mode too.
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not resolve host"));
    // Without its pinned curl the check says so.
    let output = eon_command(binary)
        .args(["update", "--check", "--json", "--profile"])
        .arg(&profile)
        .args(["--entry", "eon"])
        .env("PATH", std::env::join_paths(&path).unwrap())
        .env_remove("EON_CURL")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        report(&output)["availability"]["reason"],
        "curl-unavailable"
    );

    let output = check(
        binary,
        &["--check".as_ref(), "--install".as_ref(), "--json".as_ref()],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let invalid = report(&output);
    assert_eq!(invalid["operation"], serde_json::Value::Null);
    assert_eq!(invalid["error"]["code"], "invalid-arguments");
    // Check's process group must also end when its caller cancels the helper.
    fs::write(root.join("mode"), "read-cancel").unwrap();
    let mut child = eon_command(binary)
        .arg("update")
        .args(explicit)
        .env("PATH", std::env::join_paths(&path).unwrap())
        .env("HOME", root.join("home"))
        .env("EON_CURL", &curl)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("read-manager-pid").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(root.join("read-manager-pid").exists());
    let manager: i32 = fs::read_to_string(root.join("read-manager-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let leaked = unsafe { libc::kill(manager, 0) } == 0;
    if leaked {
        unsafe {
            libc::kill(manager, libc::SIGKILL);
        }
    }
    assert_eq!(output.status.code(), Some(130), "{output:?}");
    assert!(!leaked, "Check left its own manager child running");
    stopped(&["read-manager-pid"]);
    assert_eq!(report(&output)["error"]["code"], "interrupted");
    assert_eq!(report(&output)["selection"], serde_json::Value::Null);
    fs::write(root.join("mode"), "fail").unwrap();
    // Bare `eon update` is not a package mode.
    let output = check(binary, &["--json".as_ref()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
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
    // The full product: its composition supplies Helix. Nothing here runs it, and a
    // test of a build without Helix removes `EON_HX`.
    command
        .env("HOME", std::env::temp_dir())
        .env("XDG_STATE_HOME", test_state_directory())
        .env_remove("EON_SESSION_BIN")
        .env("EON_HX", "/eon-test-absent/hx")
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("PATH", std::env::join_paths(path).unwrap());
    command
}

/// The window `eon window ls --json` lists first, as this build's `eon` sees it from
/// `command`'s composition, with busy-terminal labels omitted.
fn listed_window(mut command: Command, runtime: &Path, config: &Path) -> serde_json::Value {
    let output = command
        .args(["window", "ls", "--json"])
        .env("EON_RUNTIME_DIR", runtime)
        .env("EON_CONFIG_HOME", config)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("window ls: {error}; {output:?}"));
    let mut window = listed["windows"][0].clone();
    assert!(window.is_object(), "window ls: {output:?}");
    for terminal in window["busy"]["terminals"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        terminal.as_object_mut().unwrap().remove("name");
    }
    window
}

/// A window of a build without Helix: its supervisor and every `eon` it runs have no
/// `EON_HX`. Stubs in `editor` are on the supervisor's PATH.
struct WindowWithoutHelix {
    root: PathBuf,
    runtime: PathBuf,
    config: PathBuf,
    editor: PathBuf,
    control: PathBuf,
    caller: String,
    supervisor: TestProcess,
}

impl WindowWithoutHelix {
    fn start(environment: &[(&str, &str)]) -> Self {
        let root = temporary_directory();
        let runtime = root.join("runtime");
        let config = root.join("config");
        configure_mock_desktop(&config);
        let stop = root.join("stop");
        let orbit = root.join("orbit");
        let venus = root.join("venus");
        managed_orbit_executable(&orbit);
        executable(&venus, "#!/bin/sh\nexit 0\n");
        let editor = root.join("editor");
        fs::create_dir(&editor).unwrap();

        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        let path = command
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value))
            .flatten()
            .unwrap()
            .to_owned();
        let path = std::iter::once(editor.clone()).chain(std::env::split_paths(&path));
        let child = command
            .env("PATH", std::env::join_paths(path).unwrap())
            .arg("run")
            .env_remove("EON_HX")
            .envs(environment.iter().copied())
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let supervisor = TestProcess { child, stop };
        let generation = generation_runtime(&runtime);
        let control = generation.join("eon.sock");
        wait_for(&control);
        let first = generation.join("orbit.sock");
        wait_for(&first);
        let environment = process::environ(live_identity(&first).process_id);
        let caller = environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
            .map(|value| String::from_utf8(value.to_vec()).unwrap())
            .expect("terminal environment names its terminal");
        Self {
            root,
            runtime,
            config,
            editor,
            control,
            caller,
            supervisor,
        }
    }

    /// An `eon` of the same build without Helix, run from the window's first terminal.
    fn command(&self) -> Command {
        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        command
            .env_remove("EON_HX")
            .current_dir(&self.root)
            .env("EON_RUNTIME_DIR", &self.runtime)
            .env("EON_CONFIG_HOME", &self.config)
            .env("EON_TERMINAL", &self.caller);
        command
    }

    fn edit(&self) -> Output {
        self.command()
            .args(["edit", "--json", "--", "notes.txt"])
            .output()
            .unwrap()
    }

    fn tabs(&self) -> Vec<Tab> {
        // Each request to a supervisor needs its own ID.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!("inspect-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        match workspace_action(&self.control, &id, Action::Inspect) {
            Response::Snapshot(snapshot) => stable_tabs(&snapshot.tabs),
            response => panic!("cannot inspect workspace: {response:?}"),
        }
    }

    /// Asserts that the last edit started `editor` on notes.txt in the selected pane.
    fn assert_editing(&self, edited: &Output, editor: &[u8]) {
        assert!(edited.status.success(), "edit: {edited:?}");
        let tabs = self.tabs();
        let tab = tabs
            .first()
            .unwrap_or_else(|| panic!("edit: {edited:?}; tabs: {tabs:?}"));
        let pane = tab
            .panes()
            .find(|pane| Some(pane.id.as_str()) == tab.selected_pane.as_deref())
            .unwrap_or_else(|| panic!("edit: {edited:?}; tabs: {tabs:?}"));
        let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
        let arguments = process::cmdline(live_identity(endpoint).process_id);
        let file = fs::canonicalize(&self.root).unwrap().join("notes.txt");
        assert!(
            arguments.ends_with(&[editor, file.as_os_str().as_bytes(), b"\0"].concat()),
            "edit: {edited:?}; tabs: {tabs:?}; arguments: {}",
            String::from_utf8_lossy(&arguments)
        );
    }

    /// Asserts that the last edit was refused with `edit-command`, said each of
    /// `detail`, and left the tabs as `before`.
    fn assert_refused(&self, refused: &Output, detail: &[&str], before: &[Tab]) {
        assert_eq!(refused.status.code(), Some(1), "edit: {refused:?}");
        for text in [&["\"code\":\"edit-command\""], detail].concat() {
            assert!(
                stdout(refused).contains(text),
                "missing {text:?}; edit: {refused:?}"
            );
        }
        assert_eq!(self.tabs(), before, "refused edit: {refused:?}");
    }

    fn configure_editor(&self, argv: &str) {
        let settings = fs::read_to_string(self.config.join("config.toml")).unwrap();
        fs::write(
            self.config.join("config.toml"),
            format!("{settings}\n[editor]\ncommand = {argv}\n"),
        )
        .unwrap();
    }

    fn stop(mut self) {
        fs::write(&self.supervisor.stop, "").unwrap();
        wait_for_successful_exit(&mut self.supervisor.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn a_build_without_helix_omits_it_from_versions_and_defaults_and_refuses_its_names() {
    let root = temporary_directory();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let run = |helix: bool, arguments: &[&str]| {
        let mut command = eon_command(binary);
        if !helix {
            command.env_remove("EON_HX");
        }
        let output = command
            .args(arguments)
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_RUNTIME_DIR", root.join("absent-runtime"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        stdout(&output).to_owned()
    };

    // `eon versions` leaves out exactly the Helix line: the build identity on its
    // first line and every other component stay.
    let full = run(true, &["versions"]);
    let helix: Vec<_> = full
        .lines()
        .filter(|line| line.starts_with("helix "))
        .collect();
    assert_eq!(helix.len(), 1, "{full}");
    let without = run(false, &["versions"]);
    assert_eq!(
        without.lines().collect::<Vec<_>>(),
        full.lines()
            .filter(|line| *line != helix[0])
            .collect::<Vec<_>>()
    );

    // `[editor] command` has no default.
    assert!(run(true, &["config", "defaults"]).contains("\n[editor]\ncommand = [\"hx\"] #"));
    assert!(run(false, &["config", "defaults"]).contains(
        "\n[editor]\n# command: unset. Direct argv that `eon edit` and Files run with the \
         file appended. Unset, each edit runs $VISUAL, then $EDITOR.\n"
    ));
    let json = |helix| -> serde_json::Value {
        serde_json::from_str(&run(helix, &["config", "defaults", "--json"])).unwrap()
    };
    assert_eq!(json(true)["editor"]["command"], serde_json::json!(["hx"]));
    assert_eq!(json(false)["editor"], serde_json::json!({}));

    // The Helix names refuse with one line, before anything is prepared. Running an
    // `hx` from PATH instead would find the same name again inside a terminal.
    let names = root.join("names");
    fs::create_dir(&names).unwrap();
    for name in ["hx", "eon-hx"] {
        symlink(binary, names.join(name)).unwrap();
        let refused = eon_command(&names.join(name))
            .env_remove("EON_HX")
            .arg("notes.txt")
            .env("EON_CONFIG_HOME", root.join("absent-config"))
            .env("EON_RUNTIME_DIR", root.join("absent-runtime"))
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(1), "{name}: {refused:?}");
        assert!(refused.stdout.is_empty(), "{name}: {refused:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            "eon: this Eon package includes no Helix, so it provides no hx or eon-hx\n",
            "{name}"
        );
    }
    assert!(!root.join("absent-config").exists());
    assert!(!root.join("absent-runtime").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn without_helix_an_unset_editor_is_visual_and_a_configured_one_overrides_it() {
    let window =
        WindowWithoutHelix::start(&[("VISUAL", "vis-editor --flag"), ("EDITOR", "other-editor")]);
    executable(&window.editor.join("other-editor"), "#!/bin/sh\nexit 0\n");

    // An unavailable $VISUAL refuses the edit; a valid $EDITOR does not stand in.
    let before = window.tabs();
    window.assert_refused(
        &window.edit(),
        &["vis-editor", "is unavailable on Eon's terminal PATH"],
        &before,
    );

    // $VISUAL comes first, split into its arguments.
    executable(&window.editor.join("vis-editor"), "#!/bin/sh\nexit 0\n");
    window.assert_editing(&window.edit(), b"vis-editor\0--flag\0");

    // The configured editor wins over both variables, from the next edit.
    executable(&window.editor.join("my-editor"), "#!/bin/sh\nexit 0\n");
    window.configure_editor("[\"my-editor\", \"--wait\"]");
    window.assert_editing(&window.edit(), b"my-editor\0--wait\0");

    // The full product's `eon` of this build sees the same build and component graph,
    // so it lists this window as its own and presents it.
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let build = |command: &mut Command| {
        let output = command.arg("versions").output().unwrap();
        assert!(output.status.success(), "versions: {output:?}");
        stdout(&output)
            .lines()
            .next()
            .unwrap_or_else(|| panic!("versions: {output:?}"))
            .to_owned()
    };
    assert_eq!(
        build(&mut eon_command(binary)),
        build(&mut window.command())
    );
    // A bare launch compares the supervisor's component graph with its own before
    // it presents the running window.
    let launched = eon_command(binary)
        .env("EON_RUNTIME_DIR", &window.runtime)
        .env("EON_CONFIG_HOME", &window.config)
        .env("EON_ORBIT", window.root.join("orbit"))
        .env("EON_VENUS", window.root.join("venus"))
        .output()
        .unwrap();
    assert!(launched.status.success(), "{launched:?}");
    let full = listed_window(eon_command(binary), &window.runtime, &window.config);
    let own = listed_window(window.command(), &window.runtime, &window.config);
    assert_eq!(full["attach"]["available"], true, "{full}");
    assert_eq!(full, own);
    assert!(
        full["component_report"]
            .as_str()
            .unwrap()
            .contains("\nhelix "),
        "{full}"
    );
    let attached = eon_command(binary)
        .args(["window", "attach", "default"])
        .env("EON_RUNTIME_DIR", &window.runtime)
        .env("EON_CONFIG_HOME", &window.config)
        .output()
        .unwrap();
    assert!(attached.status.success(), "{attached:?}");
    window.stop();
}

#[test]
fn without_helix_an_unset_editor_falls_to_editor_with_its_arguments() {
    let window = WindowWithoutHelix::start(&[("EDITOR", "my-editor --wait")]);
    executable(&window.editor.join("my-editor"), "#!/bin/sh\nexit 0\n");
    window.assert_editing(&window.edit(), b"my-editor\0--wait\0");
    window.stop();
}

#[test]
fn without_helix_and_without_an_editor_the_edit_names_the_setting() {
    let window = WindowWithoutHelix::start(&[]);
    let before = window.tabs();
    window.assert_refused(
        &window.edit(),
        &[
            "neither VISUAL nor EDITOR",
            "set [editor] command in config.toml",
        ],
        &before,
    );

    // The setting the message names applies to the next edit, without a restart.
    executable(&window.editor.join("my-editor"), "#!/bin/sh\nexit 0\n");
    window.configure_editor("[\"my-editor\"]");
    window.assert_editing(&window.edit(), b"my-editor\0");
    window.stop();
}

fn test_state_directory() -> PathBuf {
    std::env::temp_dir()
        .join("eon-test-state")
        .join(std::thread::current().name().unwrap_or("unnamed"))
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

// Lifecycle assertions exclude labels, and the busy names made of them, which
// change independently as metadata arrives.
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
    // Inspect-only caller metadata does not change the workspace state.
    value.as_object_mut().unwrap().remove("caller");
    for tab in value["tabs"].as_array_mut().unwrap() {
        for pane in tab["panes"].as_array_mut().unwrap() {
            pane.as_object_mut().unwrap().remove("label");
        }
    }
    for terminal in value["busy"]["terminals"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        terminal.as_object_mut().unwrap().remove("name");
    }
    value
}

#[test]
fn stop_accounts_for_late_ready_pair_and_retains_partial_cleanup_owner() {
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
    executable(&bin.join("eon-files"), "#!/bin/sh\ncat\n");
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
            .env("EON_TEST_PAIR_READY_DELAY_MS", "5100")
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
    let pair = live_identity(&generation.join("terminal-2.sock"));
    let first = live_identity(&generation.join("orbit.sock"));
    assert!(
        invoke(binary, &runtime, &config, &["pane", "create", "--json"])
            .status
            .success()
    );
    // The late pair keeps terminal-2, so the next terminal never reuses it.
    let second = live_identity(&generation.join("terminal-3.sock"));
    fs::write(&fail, "terminal-3").unwrap();
    let stop = ["window", "stop", "default", "--json"];
    let failed = invoke(binary, &runtime, &config, &stop);
    assert_eq!(failed.status.code(), Some(1), "{}", stdout(&failed));
    assert!(stdout(&failed).contains("injected stop failure"));
    assert!(
        supervisor.child.try_wait().unwrap().is_none(),
        "partial Stop discarded its owner"
    );
    wait_for_process_removal(pair.process_id, "late Ready pair survived Stop");
    wait_for_process_removal(first.process_id, "completed terminal survived Stop");
    let venus_pid: u32 = fs::read_to_string(venus_pid).unwrap().parse().unwrap();
    // SAFETY: this is the unreaped presentation child recorded by this test's supervisor.
    assert_eq!(unsafe { libc::kill(venus_pid as i32, libc::SIGTERM) }, 0);
    wait_for_process_removal(venus_pid, "partial Stop retained an ended Venus child");
    let inventory = invoke(binary, &runtime, &config, &["window", "ls", "--json"]);
    assert!(inventory.status.success(), "{}", stdout(&inventory));
    let listed: serde_json::Value = serde_json::from_str(stdout(&inventory)).unwrap();
    assert_eq!(
        listed["windows"][0]["terminals"],
        serde_json::json!(["terminal-3"]),
        "{listed}"
    );
    assert!(matches!(
        workspace_action(&control, "blocked", Action::CreatePane { focus: true }),
        Response::Failure(_)
    ));
    let retried = invoke(binary, &runtime, &config, &stop);
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
    wait_for_ready(&generation.join("eon.sock"));
    let mut stopping = TestProcess {
        child: eon_command(binary)
            .args(["window", "stop", "default"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        stop: supervisor.stop.clone(),
    };
    let mut prompt = Vec::new();
    while !prompt.ends_with(b"[y/N] ") {
        let mut byte = [0];
        stopping
            .child
            .stderr
            .as_mut()
            .unwrap()
            .read_exact(&mut byte)
            .unwrap();
        prompt.push(byte[0]);
    }
    let mut creating = TestProcess {
        child: eon_command(binary)
            .args(["pane", "create", "--json"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
        stop: supervisor.stop.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !generation.join("terminal-2.sock.record").exists() {
        assert!(Instant::now() < deadline, "queued pane did not start");
        thread::sleep(Duration::from_millis(10));
    }
    stopping
        .child
        .stdin
        .take()
        .unwrap()
        .write_all(b"y\n")
        .unwrap();
    assert!(creating.child.wait().unwrap().success());
    let result = stopping.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(stdout(&result).contains("terminal-1, terminal-2"));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn events_cli_streams_resumes_interrupts_and_exits_on_closed_stdout() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    wait_for_ready(&generation.join("eon.sock"));
    let stream_command = || {
        let mut command = eon_command(binary);
        command
            .args(["events", "--json", "--kind", "session", "--kind", "session"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config);
        command
    };
    let log = root.join("events");
    let mut reader = TestProcess {
        child: stream_command()
            .stdout(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let first: serde_json::Value = serde_json::from_str(&wait_for_lines(&log, 1)[0]).unwrap();
    assert_eq!(first["type"], "snapshot");
    let filtered_log = root.join("filtered");
    let mut filtered = TestProcess {
        child: eon_command(binary)
            .args(["events", "--json", "--tab", "t1", "--kind", "tab"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdout(fs::File::create(&filtered_log).unwrap())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    wait_for_lines(&filtered_log, 1);
    // Creating t2 is excluded; the admitted t1 change is an ordered delivery barrier.
    assert!(
        invoke(
            binary,
            &runtime,
            &config,
            &[
                "tab",
                "create",
                "--no-focus",
                "--json",
                "--dir",
                root.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert!(
        invoke(
            binary,
            &runtime,
            &config,
            &[
                "tab",
                "directory",
                "t1",
                "--json",
                "--",
                root.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    let filtered_lines = wait_for_lines(&filtered_log, 2);
    assert_eq!(filtered_lines.len(), 2, "excluded tab produced a line");
    let admitted: serde_json::Value = serde_json::from_str(&filtered_lines[1]).unwrap();
    assert_eq!(admitted["event"], "tab-directory");
    assert_eq!(admitted["tab"]["id"], "t1");
    let created = invoke(
        binary,
        &runtime,
        &config,
        &[
            "session",
            "new",
            "--no-focus",
            "--name",
            "event-proof",
            "--json",
        ],
    );
    assert!(created.status.success(), "{}", stdout(&created));
    let lines = wait_for_lines(&log, 2);
    let event: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(event["event"], "session-created");
    assert_eq!(event["session"]["name"], "event-proof");
    let cursor = format!("{}:{}", first["stream"].as_str().unwrap(), first["seq"]);
    let resumed_log = root.join("resumed");
    let mut resumed = TestProcess {
        child: stream_command()
            .args(["--after", &cursor])
            .stdout(fs::File::create(&resumed_log).unwrap())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let replay = wait_for_lines(&resumed_log, 2);
    let snapshot: serde_json::Value = serde_json::from_str(&replay[0]).unwrap();
    assert_eq!(snapshot["resumed"], true);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&replay[1]).unwrap(),
        event
    );
    assert_eq!(
        unsafe { libc::kill(resumed.child.id() as i32, libc::SIGINT) },
        0
    );
    assert_eq!(wait_for_exit(&mut resumed.child).code(), Some(130));
    let mut closed = TestProcess {
        child: stream_command().stdout(Stdio::piped()).spawn().unwrap(),
        stop: stop.clone(),
    };
    let mut pipe = closed.child.stdout.take().unwrap();
    pipe.read_exact(&mut [0]).unwrap();
    drop(pipe);
    wait_for_successful_exit(&mut closed.child);
    // A terminal that is not the stream's controlling terminal sends it no
    // SIGHUP when it closes; Linux reports the hangup to poll instead. The
    // guard lives to the end, as dropping it stops the supervisor.
    let hung_up = cfg!(target_os = "linux").then(|| {
        let (mut leader, device) = pseudo_terminal();
        let mut hung_up = TestProcess {
            child: stream_command()
                .stdout(Stdio::from(device))
                .spawn()
                .unwrap(),
            stop: stop.clone(),
        };
        leader.read_exact(&mut [0]).unwrap();
        drop(leader);
        wait_for_successful_exit(&mut hung_up.child);
        hung_up
    });
    let refused = stream_command()
        .env("EON_TERMINAL", "terminal-99999")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(3));
    let human_refused = eon_command(binary)
        .args(["events", "--target", "p9999"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert_eq!(human_refused.status.code(), Some(3));
    assert!(human_refused.stdout.is_empty());
    assert!(String::from_utf8_lossy(&human_refused.stderr).contains("p9999"));
    let (closed_peer, closed_errors) = UnixStream::pair().unwrap();
    drop(closed_peer);
    let stderr_failed = eon_command(binary)
        .arg("events")
        .env("EON_TERMINAL", "!")
        .stdout(Stdio::piped())
        .stderr(Stdio::from(std::os::fd::OwnedFd::from(closed_errors)))
        .output()
        .unwrap();
    assert_eq!(stderr_failed.status.code(), Some(3));
    assert!(stderr_failed.stdout.is_empty());
    let (closed_peer, closed_output) = UnixStream::pair().unwrap();
    drop(closed_peer);
    let closed_startup = TestProcess {
        child: stream_command()
            .env("EON_TERMINAL", "!")
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(closed_output)))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let cancelled = closed_startup.wait_with_output().unwrap();
    assert_eq!(cancelled.status.code(), Some(0));
    assert!(cancelled.stderr.is_empty());
    let invalid = stream_command().args(["--after", "bad"]).output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let stopped = invoke(
        binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut reader.child);
    assert!(
        fs::read_to_string(log)
            .unwrap()
            .contains("\"reason\":\"ended\"")
    );
    wait_for_successful_exit(&mut supervisor.child);
    // Prevent the guards from recreating stop markers after removing the root.
    wait_for_successful_exit(&mut filtered.child);
    drop((reader, resumed, closed, hung_up, filtered, supervisor));
    fs::remove_dir_all(root).unwrap();
}

/// Connects and subscribes to every event of the window behind `socket`.
fn subscription(socket: &Path, id: &str) -> UnixStream {
    let request = encode_request(&Request {
        id: id.into(),
        caller: None,
        action: ProtocolAction::Subscribe {
            filter: Filter::default(),
            after: None,
        },
    })
    .unwrap();
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(&request).unwrap();
    stream
}

/// Reads one whole subscription frame, or None at a clean end of stream.
fn read_subscription_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut frame = vec![0; HEADER_BYTES];
    let mut filled = 0;
    while filled < HEADER_BYTES {
        match stream.read(&mut frame[filled..]) {
            Ok(0) if filled == 0 => return None,
            Ok(0) => panic!("subscription ended inside a frame header"),
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => panic!("subscription read failed: {error}"),
        }
    }
    let length = declared_message_len(&frame).unwrap();
    frame.resize(length, 0);
    stream
        .read_exact(&mut frame[HEADER_BYTES..])
        .unwrap_or_else(|error| panic!("subscription ended inside a frame: {error}"));
    Some(frame)
}

#[test]
fn a_stalled_subscriber_never_delays_requests_another_subscriber_or_stop() {
    const ACTIONS: usize = 450;
    const BOUND: Duration = Duration::from_secs(5);
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");
    // Two directories named by nearly PATH_MAX bytes each (about 4000 on Linux, 1000 on
    // macOS), so every directory event is large and a reader that stops reading fills its
    // connection.
    let directories = ["a", "b"].map(|name| {
        let directory = root.join(name);
        fs::create_dir(&directory).unwrap();
        let mut bytes = directory.as_os_str().as_bytes().to_vec();
        while bytes.len() + 2 <= libc::PATH_MAX as usize - 96 {
            bytes.extend_from_slice(b"/.");
        }
        bytes
    });
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let mut supervisor = TestProcess {
        child: eon_command(binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop,
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_ready(&control);
    let id = generation.file_name().unwrap().to_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut dismissed = false;
    let tab = loop {
        if let Response::Snapshot(snapshot) = workspace_action(&control, "ready", Action::Inspect) {
            if snapshot
                .tabs
                .iter()
                .any(|tab| tab.id == snapshot.active_tab && !tab.pending)
            {
                break snapshot.active_tab;
            }
            // Dismissing t1's pending pair starts its first pane (EON-C18).
            if let Some((pending, pair)) = pending_pair(&snapshot)
                && !dismissed
            {
                let target = PopupTarget {
                    tab: pending.id.clone(),
                    instance: pair.id.clone(),
                };
                protocol_action(&control, "dismiss", ProtocolAction::DismissPopup(target));
                dismissed = true;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the first tab was not established"
        );
        thread::sleep(Duration::from_millis(10));
    };

    // A subscribes and never reads; it keeps its connection open throughout.
    let mut stalled = subscription(&control, "stalled");
    // B reads every frame as it arrives.
    let received = Arc::new(Mutex::new(Vec::<SubscriptionMessage>::new()));
    let received_bytes = Arc::new(AtomicU64::new(0));
    let reader = {
        let mut stream = subscription(&control, "reading");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let received = Arc::clone(&received);
        let received_bytes = Arc::clone(&received_bytes);
        thread::spawn(move || {
            while let Some(frame) = read_subscription_frame(&mut stream) {
                received_bytes.fetch_add(frame.len() as u64, Ordering::Relaxed);
                let message = decode_subscription_message(&frame).unwrap();
                received.lock().unwrap().push(message);
            }
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !received
        .lock()
        .unwrap()
        .iter()
        .any(|message| matches!(message, SubscriptionMessage::Subscribed(_)))
    {
        assert!(
            Instant::now() < deadline,
            "the reading subscriber was not served"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let mut slowest = Duration::ZERO;
    let mut timed = |name: String, action: Action| {
        let started = Instant::now();
        let response = workspace_action(&control, &name, action);
        let elapsed = started.elapsed();
        assert!(
            matches!(response, Response::Snapshot(_)),
            "{name} failed: {response:?}"
        );
        assert!(
            elapsed < BOUND,
            "{name} took {elapsed:?} beside a stalled subscriber"
        );
        slowest = slowest.max(elapsed);
    };
    let started = Instant::now();
    for index in 0..ACTIONS {
        timed(
            format!("directory-{index}"),
            Action::SetTabDirectory {
                tab: tab.clone(),
                directory: directories[index % 2].clone(),
            },
        );
        if index % 10 == 9 {
            timed(format!("inspect-{index}"), Action::Inspect);
        }
    }
    let driven = started.elapsed();

    // B receives every directory change in order while A reads nothing.
    let directory_events = |messages: &[SubscriptionMessage]| {
        messages
            .iter()
            .filter_map(|message| match message {
                SubscriptionMessage::Event(event) => match &event.change {
                    Change::Tab {
                        change: TabChange::Directory,
                        tab: changed,
                        ..
                    } if changed.id == tab => Some(changed.directory.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while directory_events(&received.lock().unwrap()).len() < ACTIONS {
        assert!(
            Instant::now() < deadline,
            "the reading subscriber received {} of {ACTIONS} directory events",
            directory_events(&received.lock().unwrap()).len()
        );
        thread::sleep(Duration::from_millis(10));
    }

    let stopping = Instant::now();
    let stopped = invoke(
        binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    let stop_took = stopping.elapsed();
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    assert!(
        stop_took < Duration::from_secs(5),
        "Stop took {stop_took:?} beside a stalled subscriber"
    );
    wait_for_successful_exit(&mut supervisor.child);

    // B's stream ends with ENDED for this generation, then end of stream.
    reader.join().unwrap();
    let messages = std::mem::take(&mut *received.lock().unwrap());
    let SubscriptionMessage::Subscribed(subscribed) = &messages[0] else {
        panic!("the stream did not open with SUBSCRIBED: {:?}", messages[0]);
    };
    assert_eq!(subscribed.generation, id);
    assert_eq!(
        messages.last(),
        Some(&SubscriptionMessage::End(End::Ended {
            generation: id.clone()
        }))
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, SubscriptionMessage::Gap(_))),
        "the reading subscriber fell behind"
    );
    let seqs = messages
        .iter()
        .filter_map(|message| match message {
            SubscriptionMessage::Event(event) => Some(event.seq),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "event seqs were not increasing"
    );
    assert!(seqs.first().is_some_and(|first| *first > subscribed.seq));
    let expected = (0..ACTIONS)
        .map(|index| directories[index % 2].clone())
        .collect::<Vec<_>>();
    assert_eq!(directory_events(&messages), expected);
    let read_by_b = received_bytes.load(Ordering::Relaxed);

    // A reaches end of stream, having been held to its connection's buffer.
    // macOS refuses the timeout once the supervisor has closed A's
    // connection; its reads then end without waiting.
    let _ = stalled.set_read_timeout(Some(Duration::from_secs(10)));
    let mut read_by_a = 0u64;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        match stalled.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read_by_a += read as u64,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => panic!("the stalled subscriber did not reach end of stream: {error}"),
        }
    }
    assert!(
        read_by_a < read_by_b / 2,
        "the stalled subscriber read {read_by_a} of {read_by_b} bytes"
    );
    eprintln!(
        "{ACTIONS} actions in {driven:?}, slowest round trip {slowest:?}, Stop {stop_took:?}, \
         reader {read_by_b} bytes, stalled {read_by_a} bytes"
    );
    assert!(!generation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn closed_stdout_pipe_is_not_a_panic() {
    let root = temporary_directory();
    for arguments in [
        &["config", "path"][..],
        &["config", "dir"],
        &["versions"],
        &["-V"],
    ] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(arguments)
            .env("EON_CONFIG_HOME", root.join("config"))
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{arguments:?}: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{arguments:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_configuration_root_is_resolved_once() {
    let root = temporary_directory();
    // Eon resolves the root from its working directory, which macOS reports
    // through /private for a root under /tmp.
    let config = fs::canonicalize(&root).unwrap().join("config");
    for (command, path) in [
        ("dir", config.clone()),
        ("path", config.join("config.toml")),
    ] {
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(["config", command])
            .current_dir(&root)
            .env("EON_CONFIG_HOME", "config")
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{command}: stdout={} stderr={}",
            stdout(&output),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(stdout(&output), format!("{}\n", path.display()));
        assert!(config.is_dir());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_path_prints_the_settings_file_and_config_dir_the_root() {
    let root = temporary_directory();
    let config = root.join("config");
    let run = |arguments: &[&str]| {
        eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(arguments)
            .env("EON_CONFIG_HOME", &config)
            .output()
            .unwrap()
    };

    let output = run(&["config", "path"]);
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        stdout(&output),
        format!("{}\n", config.join("config.toml").display())
    );
    assert!(config.is_dir());
    assert!(!config.join("config.toml").exists());

    let output = run(&["config", "dir"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(stdout(&output), format!("{}\n", config.display()));

    for arguments in [
        &["config", "path", "--file"][..],
        &["config", "path", "--json"],
        &["config", "dir", "--json"],
        &["config"],
    ] {
        assert_eq!(run(arguments).status.code(), Some(2), "{arguments:?}");
    }

    // A root under a regular file cannot be created.
    fs::write(root.join("file"), "").unwrap();
    for command in ["path", "dir"] {
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(["config", command])
            .env("EON_CONFIG_HOME", root.join("file/config"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot create"),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
    for (arguments, status, code) in [
        (&["tab", "close"][..], 2, None),
        (&["window", "stop", "ffff"], 3, Some("unknown-window")),
        (&["session"], 4, Some("missing-supervisor")),
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
        (&["session", "--json"], 3, "unknown-caller"),
    ] {
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(arguments)
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"))
            .env("EON_TERMINAL", "terminal-1.other-build")
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
    let missing = run(&["session", "--json"]);
    assert_eq!(missing.status.code(), Some(4));
    let missing: serde_json::Value = serde_json::from_str(stdout(&missing)).unwrap();
    assert_eq!(
        missing["error"]["detail"],
        "the default Eon window is not running; run `eon` to open it"
    );
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 0);
    // An existing directory that is not private still refuses the call.
    fs::set_permissions(&generations, fs::Permissions::from_mode(0o755)).unwrap();
    let unsafe_directory = run(&["session", "--json"]);
    assert_eq!(unsafe_directory.status.code(), Some(4));
    let detail: serde_json::Value = serde_json::from_str(stdout(&unsafe_directory)).unwrap();
    let detail = detail["error"]["detail"].as_str().unwrap();
    assert!(
        detail.starts_with("Eon window default: ") && detail.contains("owned private directory"),
        "{detail}"
    );
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
            (&["window", "stop", "0000abcd"], 3),
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
        let output = run(&["window", "stop", "0000abcd", "--json"]);
        assert_eq!(output.status.code(), Some(3), "{windows_exist}");
        let failure: serde_json::Value = serde_json::from_str(stdout(&output)).unwrap();
        assert_eq!(
            failure["error"]["code"], "unknown-window",
            "{windows_exist}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_focus_is_rejected_before_supervisor_connection() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));

    let missing = invoke(binary, &runtime, &config, &["session", "--json"]);
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

    let versions = invoke(binary, &runtime, &config, &["versions"]);
    let current = stdout(&versions).split_whitespace().nth(2).unwrap();
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
        (
            "[bar]\nshow_version = 'no'\n",
            "bar.show_version must be true or false",
        ),
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
        .env("EON_ICON", "/fixture/eon-icon.rgba")
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
    overfill_generation_directory(&runtime);
    let exact = eon_command(&binary)
        .args(["window", "attach", "default"])
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
            "--no-decorations\n--application-id\neon\n--background-opacity\n0.8\n--padding-color\nextend\n--background-blur\n--detect-local-hosts\ntrue\n--cursor-trail-duration-v1\n1.5\n--eon-version\n{}\n--eon-icon\n/fixture/eon-icon.rgba\n--ram-notice-percent\n60\n--ram-warning-percent\n75\n--ram-critical-percent\n90\n--cpu-notice-percent\n60\n--cpu-warning-percent\n75\n--cpu-critical-percent\n90\n--cpu-average-seconds\n30\n--cpu-temperature-warning-celsius\n85\n--cpu-temperature-critical-celsius\n95\n--disk-notice-percent\n60\n--disk-warning-percent\n75\n--disk-critical-percent\n90\n--disk-path\n{}\n--bar-items\ncodex_quota_5h,codex_quota_7d,claude_quota_5h,claude_quota_7d,claude_quota_scoped,cpu,ram,disk,clock,version\n--pane-frames\ntrue\n--warning-close-last-tab\ntrue\n--animate-working-badge\ntrue\n--number-keys\ntrue\n--workspace\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::temp_dir().display(),
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
    // This fixture deliberately takes six seconds to admit its native surface.
    let bound = Instant::now() + Duration::from_secs(5);
    while !generation.join("eon.sock").exists() {
        assert!(Instant::now() < bound, "eonbase did not bind its endpoint");
        thread::sleep(Duration::from_millis(10));
    }
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
    let mut repeated = TestProcess {
        child: eonbase(&command)
            .args(["--", "/bin/false"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: child_exit.clone(),
    };
    wait_for_successful_exit(&mut repeated.child);
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
    let diagnostic = fs::read_dir(test_state_directory().join("eon/logs"))
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
                "|stdin-ready-focus-v1|--no-decorations --application-id eonova --background-opacity "
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

    let workspace = invoke(&eon, &runtime, &config, &["session", "--json"]);
    assert_eq!(workspace.status.code(), Some(1));
    assert!(stdout(&workspace).contains("\"code\":\"session-unavailable\""));
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
        assert!(!process::exists(pid.parse().unwrap()));
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

    let mut first = TestProcess {
        child: launch(&first_stop),
        stop: first_stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    let orbit_socket = generation.join("orbit.sock");
    wait_for_ready(&control);
    wait_for(&orbit_log);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    let orbit_pid = live_identity(&orbit_socket).process_id;
    first.child.kill().unwrap();
    first.child.wait().unwrap();
    // SAFETY: the test owns this exact process identity.
    assert_eq!(unsafe { libc::kill(orbit_pid as i32, libc::SIGKILL) }, 0);
    wait_for_process_removal(orbit_pid, "dead Session was not reaped");
    assert!(artifact_path(&orbit_socket, ".record").exists());
    fs::remove_file(&orbit_log).unwrap();

    let mut second = TestProcess {
        child: launch(&second_stop),
        stop: second_stop.clone(),
    };
    wait_for_ready(&control);
    wait_for(&orbit_log);
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 1);
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    // eonbase keeps `stop GENERATION` (EON-C12).
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
    let before = read(&["session", "--json"]);
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
        read(&["session", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
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
        read(&["session", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
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
        read(&["session", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
        "--clear"
    );
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    let commands = root.join("eon-commands");
    symlink(&binary, &commands).unwrap();
    let environment = process::environ(
        live_identity(&generation_runtime(&runtime).join("orbit.sock")).process_id,
    );
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
            read(&["session", "--json"])["tabs"][0]["panes"][0]["pinned_name"],
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
        let environment = process::environ(live_identity(endpoint).process_id);
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
    let second = caller(&generation.join("terminal-2.sock"));
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
        ("s1", "s2")
    );
    assert_eq!(stable_tabs(&created.tabs), stable_tabs(&original.tabs));
    assert_eq!(created.workspaces.len(), 2);
    let shown = snapshot(workspace_action(&control, "shown", Action::Inspect));
    assert_eq!(shown.workspace, "s2");
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
            workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("original".into()),
        },
    ));
    assert_eq!(switched.workspace, "s1");
    assert_eq!(
        stable_tabs(
            &snapshot(workspace_action(
                &control,
                "same",
                Action::SwitchWorkspace {
                    workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s1".into())
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
            workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s2".into()),
        },
    ));
    let inspected = snapshot(protocol_action_from(
        &control,
        "inspect-caller",
        Some(&first),
        ProtocolAction::Workspace(Action::Inspect),
    ));
    assert_eq!(inspected.workspace, "s1");
    assert_eq!(stable_tabs(&inspected.tabs), stable_tabs(&original.tabs));

    // The first confirmed stop is pruned; the failing terminal and workspace remain.
    fs::write(&fail, "terminal-2").unwrap();
    assert!(
        matches!(workspace_action(&control, "close-fails", Action::CloseWorkspace { workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s1".into()) }), Response::Failure(error) if error.code == "session-close-failed" && error.detail.contains("injected stop failure"))
    );
    let remaining = snapshot(protocol_action_from(
        &control,
        "remaining",
        Some(&second),
        ProtocolAction::Workspace(Action::Inspect),
    ));
    assert_eq!(remaining.workspace, "s1");
    assert_eq!(
        remaining.tabs[0]
            .panes()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["p2"]
    );
    assert_eq!(remaining.active_workspace, "s2");
    assert!(
        matches!(workspace_action(&control, "close-fails", Action::CloseWorkspace { workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s1".into()) }), Response::Failure(error) if error.code == "duplicate-request")
    );
    let closed = snapshot(protocol_action_from(
        &control,
        "retry-close",
        Some(&second),
        ProtocolAction::Workspace(Action::CloseWorkspace {
            workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s1".into()),
        }),
    ));
    assert_eq!(
        closed.workspace, "s2",
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
                workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s2".into())
            }
        ),
        Response::Ended(_)
    ));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!control.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn c38_registry_queries_are_read_only_and_independent_of_windows() {
    let root = temporary_directory();
    let state = root.join("state");
    let runtime = root.join("absent-runtime");
    let config = root.join("absent-config");
    let run = |arguments: &[&str]| {
        eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(arguments)
            .env("XDG_STATE_HOME", &state)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_WINDOW_RUNTIME_DIR", root.join("stale-window"))
            .env("EON_TERMINAL", "invalid caller")
            .env("EON_ORBIT", root.join("missing-orbit"))
            .env("EON_VENUS", root.join("missing-venus"))
            .output()
            .unwrap()
    };
    let empty = run(&["session", "registry", "--json"]);
    assert_eq!(empty.status.code(), Some(0), "{empty:?}");
    assert_eq!(stdout(&empty), "[]\n");
    assert!(!state.exists());

    let first = "0123456789abcdef0123456789abcdef0000000000000001";
    let second = "0123456789abcdef0123456789abcdef0000000000000002";
    let missing = "0123456789abcdef0123456789abcdef0000000000000003";
    let registry = state.join("eon/sessions");
    fs::create_dir_all(&registry).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o700)).unwrap();
    for (name, value) in [
        ("registry.lock".to_owned(), String::new()),
        (
            "allocator.json".to_owned(),
            r#"{"format":1,"namespace":"0123456789abcdef0123456789abcdef","sequence":2}"#.into(),
        ),
        (
            format!("{second}.json"),
            serde_json::json!({"format":1,"id":second}).to_string(),
        ),
        (
            format!("{first}.json"),
            serde_json::json!({"format":1,"id":first,"name":"a\"b"}).to_string(),
        ),
    ] {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(registry.join(name))
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
    }
    let expected = serde_json::json!([
        {"persistent_id":first,"name":"a\"b"},
        {"persistent_id":second,"name":null},
    ]);
    let listed = run(&["session", "registry", "--json"]);
    assert_eq!(listed.status.code(), Some(0), "{listed:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&listed.stdout).unwrap(),
        expected
    );
    let read = run(&["session", "registry", first, "--json"]);
    assert_eq!(read.status.code(), Some(0), "{read:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&read.stdout).unwrap(),
        expected[0]
    );
    let human = run(&["session", "registry", first]);
    assert_eq!(human.status.code(), Some(0), "{human:?}");
    assert!(stdout(&human).contains(first) && stdout(&human).contains("a\\\"b"));
    for json in [false, true] {
        let mut args = vec!["session", "registry", missing];
        if json {
            args.push("--json");
        }
        let output = run(&args);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
    }
    for arguments in [
        vec!["session", "registry", "--window", "default"],
        vec!["--window", "default", "session", "registry"],
        vec!["session", "--window", "default", "registry"],
        vec!["session", "registry", "--generation", "g1"],
        vec!["session", "registry", "--json", "--json"],
        vec!["session", "registry", first, second],
        vec!["session", "registry", "0123"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
    }
    let corrupt = registry.join(format!("{second}.json"));
    fs::write(&corrupt, b"{").unwrap();
    for json in [false, true] {
        let mut args = vec!["session", "registry"];
        if json {
            args.push("--json");
        }
        let output = run(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            !stdout(&output).contains(first),
            "no partial list: {output:?}"
        );
    }
    assert_eq!(fs::read(corrupt).unwrap(), b"{");
    assert!(!runtime.exists() && !config.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn c38_cli_identity_targets_and_creation_results_keep_exact_membership() {
    let window = PairWindow::new();
    let state = window.root.join("state");
    let mut launch = window.command(&[]);
    launch.env("XDG_STATE_HOME", &state);
    let mut supervisor = window.start(launch);
    wait_for_ready(&window.control());
    let command = |arguments: &[&str]| {
        let mut command = eon_command(&window.binary);
        command
            .args(arguments)
            .env("HOME", &window.home)
            .env("XDG_STATE_HOME", &state)
            .env("EON_RUNTIME_DIR", &window.runtime)
            .env("EON_CONFIG_HOME", &window.config);
        command
    };
    let read = |arguments: &[&str]| {
        let output = command(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let initial = window.inspect("initial");
    let first = initial.workspaces[0].persistent_id.as_deref().unwrap();
    assert_eq!(read(&["session", "--json"])["persistent_id"], first);
    let a = read(&["session", "new", "--name", "same", "--no-focus", "--json"]);
    let b = read(&["session", "new", "--name", "same", "--no-focus", "--json"]);
    let second = a["created_persistent_id"].as_str().unwrap();
    let third = b["created_persistent_id"].as_str().unwrap();
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert_eq!(a["created"], "s2");
    assert_eq!(a["persistent_id"], first);
    let listing = read(&["session", "ls", "--json"]);
    for (handle, id) in [("s2", second), ("s3", third)] {
        assert!(
            listing["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == handle && item["persistent_id"] == id)
        );
    }
    let inspected = read(&["session", "--id", second, "--json"]);
    assert_eq!(inspected["session"], "s2");
    assert_eq!(inspected["persistent_id"], second);
    assert_eq!(window.inspect("after-id-inspect").active_workspace, "s1");
    assert_eq!(
        inspected["tabs"][0]["directory"],
        read(&["session", "--id", third, "--json"])["tabs"][0]["directory"]
    );
    let human = command(&["session", "--id", second]).output().unwrap();
    assert!(stdout(&human).contains(&format!("persistent_id {second}\n")));
    let caller = quit_terminal_caller(Path::new(OsStr::from_bytes(
        &initial.tabs[0].popups[0].endpoint,
    )));
    let from_caller = |arguments: &[&str]| {
        let output = command(arguments)
            .env("EON_TERMINAL", OsStr::from_bytes(&caller))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        output
    };
    let ordinary: serde_json::Value =
        serde_json::from_slice(&from_caller(&["session", "--json"]).stdout).unwrap();
    assert_eq!(
        ordinary["caller"],
        serde_json::json!({"terminal":"terminal-2","tab":"t1","pane":null,"popup":"u1"})
    );
    let targeted: serde_json::Value =
        serde_json::from_slice(&from_caller(&["session", "--id", second, "--json"]).stdout)
            .unwrap();
    assert_eq!(targeted["caller"], ordinary["caller"]);
    assert_eq!(targeted["session"], "s2");
    assert_eq!(targeted["persistent_id"], second);
    assert_eq!(targeted["tabs"], inspected["tabs"]);
    let human = from_caller(&["session", "--id", second]);
    assert!(stdout(&human).contains("caller terminal=terminal-2 tab=t1 pane=none popup=u1\n"));
    let after = window.inspect("after-caller-inspect");
    assert_eq!(after.active_workspace, "s1");
    assert_eq!(after.active_tab, initial.active_tab);
    assert_eq!(after.tabs, initial.tabs);
    let ambiguous = command(&["session", "switch", "same", "--json"])
        .output()
        .unwrap();
    assert_eq!(ambiguous.status.code(), Some(1), "{ambiguous:?}");
    read(&["session", "rename", "--id", second, "renamed", "--json"]);
    assert_eq!(
        read(&["session", "registry", second, "--json"]),
        serde_json::json!({"persistent_id":second,"name":"renamed"})
    );
    read(&["session", "rename", "--id", second, "", "--json"]);
    assert!(read(&["session", "registry", second, "--json"])["name"].is_null());
    let switched = read(&["session", "switch", "--id", second, "--json"]);
    assert_eq!(switched["session"], "s2");
    for arguments in [
        vec!["session", "--id", "0123", "--json"],
        vec!["session", "--id", second, "--id", third],
        vec!["session", "switch", "s1", "--id", second],
        vec!["session", "rename", "s1", "x", "--id", second],
        vec!["session", "close", "s1", "--id", second],
        vec!["session", "new", "--id", second],
        vec!["session", "ls", "--id", second],
    ] {
        let output = command(&arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
    }
    // A registration without membership never resolves through another member's name.
    read(&["session", "close", "--id", third, "--json"]);
    read(&["session", "rename", "--id", second, third, "--json"]);
    for arguments in [
        vec!["session", "--id", third, "--json"],
        vec!["session", "switch", "--id", third, "--json"],
        vec!["session", "rename", "--id", third, "wrong", "--json"],
        vec!["session", "close", "--id", third, "--json"],
    ] {
        let output = command(&arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{arguments:?}: {output:?}");
    }
    let vanished = at_terminal(
        &mut command(&["session", "close", "--id", second]),
        Some(b"y\n"),
        || {
            read(&["session", "close", "--id", second, "--json"]);
            read(&["session", "rename", "--id", first, "s2", "--json"]);
        },
    );
    assert_eq!(vanished.status.code(), Some(3), "{vanished:?}");
    let remaining = window.inspect("remaining");
    assert_eq!(remaining.workspaces.len(), 1);
    assert_eq!(
        remaining.workspaces[0].persistent_id.as_deref(),
        Some(first)
    );
    assert_eq!(
        read(&["session", "registry", second, "--json"])["persistent_id"],
        second
    );
    assert_eq!(
        read(&["session", "registry", third, "--json"])["persistent_id"],
        third
    );

    // Losing a reply after acceptance cannot create another identity on replay.
    let request = Request {
        id: "c38-lost-reply".into(),
        caller: None,
        action: ProtocolAction::Workspace(Action::NewWorkspace {
            name: None,
            focus: false,
        }),
    };
    let mut stream = UnixStream::connect(window.control()).unwrap();
    stream
        .write_all(&encode_request(&request).unwrap())
        .unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    drop(stream);
    let created = wait_for_snapshot(&window.control(), "lost-reply", |snapshot| {
        snapshot.workspaces.len() == 2
    });
    assert!(
        matches!(protocol_action_from(&window.control(), &request.id, None, request.action), Response::Failure(error) if error.code == "duplicate-request")
    );
    assert_eq!(window.inspect("replayed").workspaces, created.workspaces);
    let count = read(&["session", "registry", "--json"])
        .as_array()
        .unwrap()
        .len();
    assert_eq!(count, 4);
    window.stop_generation(&mut supervisor);
    let unavailable = command(&["session", "--id", first, "--json"])
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(4), "{unavailable:?}");
    assert_eq!(
        read(&["session", "registry", "--json"])
            .as_array()
            .unwrap()
            .len(),
        count
    );
}

#[test]
fn workspace_cli_lists_creates_switches_renames_and_closes() {
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
    let mut supervisor = TestProcess { child, stop };
    let generation = generation_runtime(&runtime);
    wait_for(&generation.join("eon.sock"));
    let read = |arguments: &[&str]| {
        let output = invoke(&binary, &runtime, &config, arguments);
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_names_no_workspace(stdout(&output), &format!("{arguments:?}"));
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    read(&["session", "--json"]);
    // `--json` may come anywhere after `session`.
    assert!(read(&["session", "--json", "ls"])["sessions"].is_array());
    // Each step's whole list: (id, name, shown, tabs).
    let list = || {
        let listed = read(&["session", "ls", "--json"]);
        let workspaces = listed["sessions"].as_array().unwrap();
        // Outside Eon, the caller's workspace is the shown one.
        let shown = workspaces
            .iter()
            .find(|workspace| workspace["shown"] == true);
        assert_eq!(shown.unwrap()["id"], listed["session"]);
        assert_eq!(shown.unwrap()["recency"], 0);
        // Plain shells report no attention, which lists as null.
        assert!(
            workspaces
                .iter()
                .all(|workspace| { workspace.get("attention") == Some(&serde_json::Value::Null) })
        );
        workspaces
            .iter()
            .map(|workspace| {
                (
                    workspace["id"].as_str().unwrap().to_owned(),
                    workspace["name"].as_str().map(str::to_owned),
                    workspace["shown"].as_bool().unwrap(),
                    workspace["tabs"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let entry = |id: &str, name: Option<&str>, shown: bool| {
        (id.to_owned(), name.map(str::to_owned), shown, 1)
    };
    assert_eq!(list(), [entry("s1", None, true)]);

    // A creation names its sN in `created`, whether or not it is shown.
    let created = read(&["session", "new", "--name", " alpha ", "--json"]);
    assert_eq!(created["created"], "s2");
    assert_eq!(
        list(),
        [entry("s1", None, false), entry("s2", Some("alpha"), true)]
    );
    let background = read(&["session", "new", "--no-focus", "--json"]);
    assert_eq!(
        (&background["session"], &background["created"]),
        (&"s2".into(), &"s3".into())
    );
    let three = [
        entry("s1", None, false),
        entry("s2", Some("alpha"), true),
        entry("s3", None, false),
    ];
    assert_eq!(list(), three);

    // A name that matches two workspaces changes nothing.
    read(&["session", "rename", "s3", "alpha", "--json"]);
    let ambiguous = invoke(
        &binary,
        &runtime,
        &config,
        &["session", "switch", "alpha", "--json"],
    );
    assert_eq!(ambiguous.status.code(), Some(1));
    assert_names_no_workspace(stdout(&ambiguous), "an ambiguous switch");
    let error: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert_eq!(error["error"]["code"], "ambiguous-session");
    assert!(
        error["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("s2, s3")
    );
    let unasked = invoke(&binary, &runtime, &config, &["session", "close", "alpha"]);
    assert_eq!(unasked.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&unasked.stderr).contains("Close session"));
    let renamed = [
        entry("s1", None, false),
        entry("s2", Some("alpha"), true),
        entry("s3", Some("alpha"), false),
    ];
    assert_eq!(list(), renamed);

    // Switch by ID, then by name; the caller's own workspace is renamed through EON_TERMINAL.
    assert_eq!(
        read(&["session", "switch", "s3", "--json"])["session"],
        "s3"
    );
    let environment = process::environ(live_identity(&generation.join("orbit.sock")).process_id);
    let first = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .map(|token| String::from_utf8(token.to_vec()).unwrap())
        .unwrap();
    let inside = eon_command(&binary)
        .args(["session", "rename", "main", "--json"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_TERMINAL", &first)
        .output()
        .unwrap();
    assert!(
        inside.status.success(),
        "{}",
        String::from_utf8_lossy(&inside.stderr)
    );
    assert_eq!(
        list(),
        [
            entry("s1", Some("main"), false),
            entry("s2", Some("alpha"), false),
            entry("s3", Some("alpha"), true),
        ]
    );
    read(&["session", "switch", "main", "--json"]);
    read(&["session", "rename", "s3", "", "--json"]);
    let shown_main = [
        entry("s1", Some("main"), true),
        entry("s2", Some("alpha"), false),
        entry("s3", None, false),
    ];
    assert_eq!(list(), shown_main);

    // Without --json, close asks about the one workspace WS names; anything but yes keeps it.
    let cancelled = invoke(&binary, &runtime, &config, &["session", "close", "alpha"]);
    assert_eq!(cancelled.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&cancelled.stderr)
            .contains("Close session s2 (alpha)? This stops every terminal in it.")
    );
    assert_eq!(stdout(&cancelled), "");
    assert!(String::from_utf8_lossy(&cancelled.stderr).contains(
        "error not-confirmed: session s2 stays open; no yes was read and stdin is not a terminal"
    ));
    assert_eq!(list(), shown_main);
    let unknown = invoke(&binary, &runtime, &config, &["session", "close", "s9"]);
    assert_eq!(unknown.status.code(), Some(3));
    assert!(!String::from_utf8_lossy(&unknown.stderr).contains("Close session"));
    assert_eq!(list(), shown_main);
    let mut confirmed = TestProcess {
        child: eon_command(&binary)
            .args(["session", "close", "alpha"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        stop: supervisor.stop.clone(),
    };
    confirmed
        .child
        .stdin
        .take()
        .unwrap()
        .write_all(b"yes\n")
        .unwrap();
    assert!(confirmed.wait_with_output().unwrap().status.success());
    assert_eq!(
        list(),
        [entry("s1", Some("main"), true), entry("s3", None, false)]
    );
    let human = invoke(&binary, &runtime, &config, &["session", "ls"]);
    assert!(stdout(&human).ends_with(
        "session s1\ns1 shown=true tabs=1 attention=none recency=0 name=\"main\"\n\
         s3 shown=false tabs=1 attention=none recency=1 name=none\n"
    ));
    assert!(supervisor.child.try_wait().unwrap().is_none());

    // The last workspace's close quits the window.
    read(&["session", "close", "s3", "--json"]);
    let last = invoke(&binary, &runtime, &config, &["session", "close", "s1"]);
    assert!(String::from_utf8_lossy(&last.stderr).contains(
        "Close session s1 (main)? This quits the window and stops every terminal in it."
    ));
    assert_eq!(list(), [entry("s1", Some("main"), true)]);
    assert!(read(&["session", "close", "main", "--json"])["ended"].is_object());
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_picker_switches_creates_closes_and_cancels() {
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
    let mut supervisor = TestProcess { child, stop };
    let generation = generation_runtime(&runtime);
    wait_for(&generation.join("eon.sock"));
    let read = |arguments: &[&str]| {
        let output = invoke(&binary, &runtime, &config, arguments);
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    // Each step's whole list: (id, name, shown).
    let list = || {
        read(&["session", "ls", "--json"])["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|workspace| {
                (
                    workspace["id"].as_str().unwrap().to_owned(),
                    workspace["name"].as_str().map(str::to_owned),
                    workspace["shown"].as_bool().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let entry =
        |id: &str, name: Option<&str>, shown: bool| (id.to_owned(), name.map(str::to_owned), shown);
    // Each workspace's first terminal: s1 is the initial one, then one per `workspace new`.
    let caller = |endpoint: &str| {
        let environment = process::environ(live_identity(&generation.join(endpoint)).process_id);
        environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
            .map(|token| String::from_utf8(token.to_vec()).unwrap())
            .unwrap()
    };
    read(&["session", "new", "--name", "alpha", "--json"]);
    read(&["session", "new", "--no-focus", "--json"]);
    read(&["session", "switch", "s1", "--json"]);
    let three = [
        entry("s1", None, true),
        entry("s2", Some("alpha"), false),
        entry("s3", None, false),
    ];
    assert_eq!(list(), three);
    let (first, second, third) = (
        caller("orbit.sock"),
        caller("terminal-2.sock"),
        caller("terminal-3.sock"),
    );

    // A scripted fzf records the rows it is given and answers in turn: a row, a query that
    // matches none, then Escape.
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    let count = root.join("fzf-count");
    fs::write(&count, "0").unwrap();
    let recorded = root.join("rows");
    executable(
        &tools.join("fzf"),
        format!(
            r#"#!/bin/sh
n=$(($(cat '{count}') + 1)); echo "$n" > '{count}'
cat > '{rows}'$n
case $n in
  1) printf 's3\n\ns3\ts3\t1 tab\n';;
  2) printf 'fresh\n';;
  *) exit 130;;
esac
"#,
            count = count.display(),
            rows = recorded.display(),
        ),
    );
    // Runs a popup with `input` on its stdin: a pipe, or with `terminal` a pseudoterminal whose
    // controller stays open until the popup exits.
    let popup_status = |name: &str, caller: &str, input: &[u8], terminal: bool| {
        let program = root.join(name);
        if !program.exists() {
            symlink(&binary, &program).unwrap();
        }
        let mut command = eon_command(&program);
        let original = command
            .get_envs()
            .find_map(|(key, value)| (key == "PATH").then_some(value).flatten())
            .unwrap()
            .to_owned();
        let path = std::iter::once(tools.clone()).chain(std::env::split_paths(&original));
        let mut controller = None;
        let stdin = if terminal {
            let (leader, device) = pseudo_terminal();
            controller.insert(leader).write_all(input).unwrap();
            Stdio::from(device)
        } else {
            Stdio::piped()
        };
        let mut child = TestProcess {
            child: command
                .env("PATH", std::env::join_paths(path).unwrap())
                .env("EON_RUNTIME_DIR", &runtime)
                .env("EON_CONFIG_HOME", &config)
                .env("EON_TAB", "t1")
                .env("EON_TERMINAL", caller)
                .stdin(stdin)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
            stop: supervisor.stop.clone(),
        };
        if let Some(mut pipe) = child.child.stdin.take() {
            pipe.write_all(input).unwrap();
        }
        let output = child.wait_with_output().unwrap();
        drop(controller);
        output
    };
    let popup = |name: &str, caller: &str, input: &[u8]| {
        let output = popup_status(name, caller, input, false);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let rows = |run: u32| fs::read_to_string(format!("{}{run}", recorded.display())).unwrap();

    // Every workspace, most recently shown first, the shown one last and marked; a row shows it.
    popup("eon-sessions", &first, b"");
    assert_eq!(
        rows(1),
        "s2\talpha\t1 tab\ns3\ts3\t1 tab\ns1\ts1\t1 tab · shown\n"
    );
    assert_eq!(
        list(),
        [
            entry("s1", None, false),
            entry("s2", Some("alpha"), false),
            entry("s3", None, true),
        ]
    );
    // A query that matches no row creates a workspace of that name and shows it.
    popup("eon-sessions", &third, b"");
    assert_eq!(
        rows(2),
        "s1\ts1\t1 tab\ns2\talpha\t1 tab\ns3\ts3\t1 tab · shown\n"
    );
    let four = [
        entry("s1", None, false),
        entry("s2", Some("alpha"), false),
        entry("s3", None, false),
        entry("s4", Some("fresh"), true),
    ];
    assert_eq!(list(), four);
    // Escape changes nothing.
    popup("eon-sessions", &third, b"");
    assert_eq!(list(), four);

    // Commands renames the caller's workspace, even a hidden one, closes the popup's own
    // workspace after a yes, and runs the same picker for Switch workspace.
    for (menu, terminal, input) in [
        ("Rename session", &first, &b""[..]),
        ("Close this session", &second, &b"yes\n"[..]),
        ("Switch session", &third, &b""[..]),
    ] {
        executable(
            &tools.join("fzf"),
            format!(
                r#"#!/bin/sh
case "$*" in
  *--print-query*) cat > /dev/null; printf '\n\ns1\tRenamed\t1 tab\n';;
  *--bind=enter:print-query*) printf 'Renamed\0';;
  *) while IFS="$(printf '\t')" read -r index rest; do
       case "$rest" in '{menu}'*) printf '%s\tchosen\n' "$index";; esac
     done;;
esac
"#
            ),
        );
        popup("eon-commands", terminal, input);
    }
    assert_eq!(
        list(),
        [
            entry("s1", Some("Renamed"), true),
            entry("s3", None, false),
            entry("s4", Some("fresh"), false),
        ]
    );

    // A row whose workspace closes while the picker is open switches nothing, even when another
    // workspace now carries that sN as its name.
    executable(
        &tools.join("fzf"),
        format!(
            r#"#!/bin/sh
cat > /dev/null
'{eon}' session close s3 --json > /dev/null
'{eon}' session rename s4 s3 --json > /dev/null
printf '\n\ns3\ts3\t1 tab\n'
"#,
            eon = binary.display(),
        ),
    );
    let output = popup_status("eon-sessions", &first, b"\n", false);
    assert_eq!(output.status.code(), Some(3));
    let stale = [
        entry("s1", Some("Renamed"), true),
        entry("s4", Some("s3"), false),
    ];
    assert_eq!(list(), stale);
    // Ctrl+X on that stale row closes nothing and asks nothing.
    executable(
        &tools.join("fzf"),
        "#!/bin/sh\ncat > /dev/null\nprintf '\\nctrl-x\\ns3\\ts3\\t1 tab\\n'\n",
    );
    let output = popup_status("eon-sessions", &first, b"yes\n\n", false);
    assert_eq!(output.status.code(), Some(3));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Close session"));
    assert_eq!(list(), stale);

    // Ctrl+X closes the highlighted row's workspace after a yes and lists the rest again. A "no"
    // through a pipe is the non-terminal refusal: not-confirmed, exit 1, then the wait for
    // Enter; a terminal's "no" lists again, as the pseudoterminal steps below show.
    read(&["session", "new", "--no-focus", "--json"]);
    read(&["session", "new", "--no-focus", "--json"]);
    read(&["session", "switch", "s6", "--json"]);
    read(&["session", "switch", "s5", "--json"]);
    read(&["session", "switch", "s1", "--json"]);
    fs::write(&count, "0").unwrap();
    executable(
        &tools.join("fzf"),
        format!(
            r#"#!/bin/sh
case "$*" in *--expect=ctrl-x*) ;; *) exit 2;; esac
n=$(($(cat '{count}') + 1)); echo "$n" > '{count}'
cat > '{rows}'$n
case $n in
  1) printf '\nctrl-x\ns4\ts3\t1 tab\n';;
  2) printf 's5\nctrl-x\ns5\ts5\t1 tab\n';;
  3|5) printf '\nctrl-x\ns1\tRenamed\t1 tab · shown\n';;
  6) printf '\nctrl-x\ns5\ts5\t1 tab · shown\n';;
  *) exit 130;;
esac
"#,
            count = count.display(),
            rows = recorded.display(),
        ),
    );
    let output = popup_status("eon-sessions", &first, b"yes\nno\n", false);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("error not-confirmed: session s5"));
    assert_eq!(
        rows(2),
        "s5\ts5\t1 tab\ns6\ts6\t1 tab\ns1\tRenamed\t1 tab · shown\n"
    );
    let kept = [
        entry("s1", Some("Renamed"), true),
        entry("s6", None, false),
        entry("s5", None, false),
    ];
    assert_eq!(list(), kept);
    let fzf_runs = || {
        fs::read_to_string(&count)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    };
    // A terminal's "no" to closing the picker's own workspace keeps it and lists again.
    let output = popup_status("eon-sessions", &first, b"no\n", true);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("session s1 stays open"));
    assert_eq!(fzf_runs(), 4);
    assert_eq!(list(), kept);
    // Ctrl+X on the picker's own workspace closes it, ends the picker and shows the most
    // recently shown remaining workspace.
    let last = caller("terminal-5.sock");
    popup("eon-sessions", &first, b"yes\n");
    assert_eq!(fzf_runs(), 5);
    assert_eq!(list(), [entry("s6", None, false), entry("s5", None, true)]);
    // Declining the last workspace's close keeps the window and lists again.
    read(&["session", "close", "s6", "--json"]);
    let output = popup_status("eon-sessions", &last, b"no\n", true);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("session s5 stays open"));
    assert_eq!(fzf_runs(), 7);
    assert_eq!(list(), [entry("s5", None, true)]);
    assert!(read(&["session", "close", "s5", "--json"])["ended"].is_object());
    wait_for_successful_exit(&mut supervisor.child);
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
        (vec!["session", "--window", &id[3..], "--json"], 3),
        (vec!["session", "--window", "0000abcd", "--json"], 3),
        (vec!["session", "--window", "0000", "--json"], 3),
        (vec!["session", "--generation", id, "--json"], 2),
        (vec!["--generation", id, "session"], 2),
        (vec!["session", "--window", "ABCDEFGH"], 2),
        (vec!["session", "--window", "abc"], 2),
        (vec!["session", "--window"], 2),
        (vec!["versions", "--window", "0000abcd"], 2),
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
    let done = root.join("done");
    let server = older_supervisor(&runtime, id, &done);
    for caller in [None, Some("terminal-1.unknown")] {
        let mut command = eon_command(binary);
        if let Some(caller) = caller {
            command.env("EON_TERMINAL", caller);
        }
        let output = command
            .args(["session", "--window", &id[3..9], "--json"])
            .env("EON_RUNTIME_DIR", &runtime)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{}", stdout(&output));
        let failure: serde_json::Value = serde_json::from_str(stdout(&output)).unwrap();
        let detail = failure["error"]["detail"].as_str().unwrap();
        assert!(
            detail.starts_with(&format!("Eon window {}: ", &id[3..])),
            "{detail}"
        );
        assert!(detail.contains("EONW"), "{detail}");
        assert!(detail.contains("the terminal's own `eon`"), "{detail}");
    }
    end_older_supervisors(
        &done,
        &[runtime.join("generations").join(id).join("eon.sock")],
        vec![server],
    );
    fs::remove_dir_all(&runtime).unwrap();

    fs::create_dir_all(runtime.join("generations")).unwrap();
    for path in [&runtime, &runtime.join("generations")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    overfill_generation_directory(&runtime);
    fs::create_dir(runtime.join("generations").join(format!("g1-{:032x}", 256))).unwrap();
    let output = eon_command(binary)
        .args(["session", "--json"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_TERMINAL", "terminal-1.unknown")
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
    #[cfg(target_os = "macos")]
    {
        // Patching invalidates the ad-hoc signature, so macOS kills the copy at first page-in.
        let output = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&previous_binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "codesign failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
    let environment = process::environ(live_identity(&endpoint).process_id);
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
        matches!(workspace_action(&previous_control, "hide-caller", Action::NewWorkspace { name: Some("shown".into()), focus: true }), Response::Snapshot(snapshot) if snapshot.workspace == "s2")
    );
    let inspected = from(&["session", "--json"]);
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
    assert_eq!(described["session"], "s1");
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
    let still_owned = from(&["session", "--json"]);
    assert!(still_owned.status.success(), "{}", stdout(&still_owned));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&still_owned.stdout).unwrap()["generation"],
        previous_id
    );
    older.join().unwrap();
    // Another build's default-namespace window answers to its generation's
    // hex digits and their prefixes.
    for arguments in [
        vec!["session", "--window", &previous_id[3..], "--json"],
        vec!["--window", &previous_id[3..9], "session", "--json"],
    ] {
        let outside = invoke(binary, &runtime, &config, &arguments);
        assert!(outside.status.success(), "{}", stdout(&outside));
        let result: serde_json::Value = serde_json::from_slice(&outside.stdout).unwrap();
        assert_eq!(result["generation"], previous_id);
        assert_eq!(result["session"], "s2");
        let inside = from(&arguments);
        assert!(inside.status.success(), "{}", stdout(&inside));
        let result: serde_json::Value = serde_json::from_slice(&inside.stdout).unwrap();
        assert_eq!(result["session"], "s1");
    }
    let wrong = from(&[
        "pane",
        "create",
        "--window",
        "default",
        "--no-focus",
        "--json",
    ]);
    assert_eq!(wrong.status.code(), Some(3), "{}", stdout(&wrong));
    let human = from(&["session"]);
    assert!(human.status.success());
    assert!(stdout(&human).starts_with(&format!(
        "generation {previous_id}\nwindow default\nsession s1\n"
    )));
    for token in [
        String::from_utf8(caller.to_vec()).unwrap() + "0",
        "terminal-1".into(),
        "not a terminal".into(),
        "".into(),
    ] {
        for arguments in [
            vec!["session", "--json"],
            vec!["pane", "create", "--window", "default", "--json"],
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
    assert_eq!(shown.workspace, "s2");
    assert_eq!(shown.tabs[0].panes().count(), 1);
    // Caller discovery also works when this build's supervisor has ended.
    fs::write(&current.stop, "").unwrap();
    wait_for_successful_exit(&mut current.child);
    assert!(from(&["session", "--json"]).status.success());
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
        &["session", "--window", window_id, "--json"],
    );
    assert!(outside.status.success(), "{}", stdout(&outside));
    let result: serde_json::Value = serde_json::from_slice(&outside.stdout).unwrap();
    assert_eq!(
        result["generation"], previous_id,
        "window-only selects its sole previous generation"
    );
    assert_eq!(result["window"], window_id);
    let constrained = from(&["session", "--window", window_id, "--json"]);
    assert_eq!(constrained.status.code(), Some(3));
    let other_id = "g1-00000000000000000000000000000001";
    let other = previous_supervisor(&window_root, other_id);
    let ambiguous = invoke(
        binary,
        &runtime,
        &config,
        &["session", "--window", window_id, "--json"],
    );
    assert_eq!(ambiguous.status.code(), Some(1), "{}", stdout(&ambiguous));
    assert!(stdout(&ambiguous).contains("\"code\":\"ambiguous-window\""));
    assert!(stdout(&ambiguous).contains("none is this build's"));
    assert!(!stdout(&ambiguous).contains("--generation"));
    assert!(!stdout(&ambiguous).contains(previous_id));
    // `--generation` no longer narrows a window to one of its generations.
    let removed = invoke(
        binary,
        &runtime,
        &config,
        &[
            "session",
            "--window",
            window_id,
            "--generation",
            previous_id,
            "--json",
        ],
    );
    assert_eq!(removed.status.code(), Some(2), "{}", stdout(&removed));
    // As the base namespace, the window root lists the other generation by
    // its hex digits.
    assert!(
        invoke(
            binary,
            &window_root,
            &config,
            &["window", "stop", &other_id[3..], "--json"]
        )
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
    let orbit_log = root.join("orbit-cwd.log");
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
        .env("EON_TEST_ORBIT_CWD_LOG", &orbit_log)
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
    let environment = process::environ(live_identity(&first).process_id);
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
    let expected_caller = serde_json::json!({
        "terminal": "terminal-1", "tab": "t1", "pane": "p1", "popup": null
    });
    for arguments in [
        vec!["session", "--json"],
        vec!["session", "--window", "default", "--json"],
    ] {
        let output = from(&caller, &arguments);
        assert!(output.status.success(), "{}", stdout(&output));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["caller"], expected_caller);
    }
    let human = from(&caller, &["session"]);
    assert!(human.status.success(), "{}", stdout(&human));
    assert!(
        stdout(&human)
            .contains("session s1\ncaller terminal=terminal-1 tab=t1 pane=p1 popup=none\n")
    );
    for json in [false, true] {
        let output = eon_command(&binary)
            .args(if json {
                vec!["session", "--json"]
            } else {
                vec!["session"]
            })
            .env_remove("EON_TERMINAL")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stdout(&output));
        if json {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value.get("caller"), Some(&serde_json::Value::Null));
        } else {
            assert!(!stdout(&output).contains("\ncaller "));
        }
    }
    let listed = from(&caller, &["session", "ls", "--json"]);
    assert!(listed.status.success(), "{}", stdout(&listed));
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(listed.get("caller").is_none());
    let inspect = |id: &str| match workspace_action(&control, id, Action::Inspect) {
        Response::Snapshot(mut snapshot) => {
            snapshot.tabs = stable_tabs(&snapshot.tabs);
            for terminal in snapshot
                .busy
                .iter_mut()
                .flat_map(|busy| &mut busy.terminals)
            {
                terminal.name.clear();
            }
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
    let created: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    assert!(created.get("caller").is_none());
    // Each pane or tab creation names what it started in `created`.
    let names = |output: &Output, tab: &str, pane: Option<&str>| {
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            reply["created"],
            serde_json::json!({"tab": tab, "pane": pane}),
            "{reply}"
        );
    };
    assert_eq!(
        created["created"],
        serde_json::json!({"tab": "t1", "pane": "p3"})
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

    let closed = from(&caller, &["pane", "close", "p3", "--yes", "--json"]);
    assert!(closed.status.success(), "{}", stdout(&closed));
    assert!(!stdout(&closed).contains("\"id\":\"p3\""));
    // A reply that creates nothing carries no `created`.
    let closed: serde_json::Value = serde_json::from_slice(&closed.stdout).unwrap();
    assert!(closed.get("created").is_none(), "{closed}");

    // `eon edit` opens a relative file from the caller's directory in Helix,
    // the default editor, in a new pane beside the calling terminal.
    let edited = eon_command(&binary)
        .args(["edit", "--json", "--", "notes.txt"])
        .current_dir(&root)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_TERMINAL", &caller)
        .output()
        .unwrap();
    assert!(edited.status.success(), "{}", stdout(&edited));
    names(&edited, "t1", Some("p4"));
    let snapshot = inspect("after-edit");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p4"));
    let pane = &snapshot.tabs[0].columns[0].panes[1];
    assert_eq!(pane.id, "p4");
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = process::cmdline(live_identity(endpoint).process_id);
    let file = fs::canonicalize(&root).unwrap().join("notes.txt");
    assert!(
        orbit_arguments.ends_with(&[b"hx\0", file.as_os_str().as_bytes(), b"\0"].concat()),
        "{}",
        String::from_utf8_lossy(&orbit_arguments)
    );

    // An `eon` of this build composed without Helix sees the same build and component
    // graph, so it lists this window as its own and presents it.
    let without_helix = || {
        let mut command = eon_command(&binary);
        command.env_remove("EON_HX");
        command
    };
    let build = |command: &mut Command| {
        let output = command.arg("versions").output().unwrap();
        stdout(&output).lines().next().unwrap().to_owned()
    };
    assert_eq!(
        build(&mut eon_command(&binary)),
        build(&mut without_helix())
    );
    // A bare launch compares the supervisor's component graph with its own before
    // it presents the running window.
    let launched = without_helix()
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .output()
        .unwrap();
    assert!(launched.status.success(), "{launched:?}");
    let other = listed_window(without_helix(), &runtime, &config);
    assert_eq!(other["attach"]["available"], true, "{other}");
    assert_eq!(
        other,
        listed_window(eon_command(&binary), &runtime, &config)
    );
    assert!(
        other["component_report"]
            .as_str()
            .unwrap()
            .contains("\nhelix "),
        "{other}"
    );
    let attached = without_helix()
        .args(["window", "attach", "default"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert!(attached.status.success(), "{attached:?}");

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
    names(&ran, "t1", Some("p5"));
    let snapshot = inspect("after-command");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p5"));
    let pane = &snapshot.tabs[0].columns[0].panes[2];
    assert_eq!(pane.id, "p5");
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = process::cmdline(live_identity(endpoint).process_id);
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
    names(&unfocused, "t1", Some("p6"));
    let snapshot = inspect("after-unfocused");
    assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p5"));
    assert_eq!(snapshot.tabs[0].columns[0].panes[3].id, "p6");

    // A command tab needs a caller, then starts at its own directory without
    // changing the existing tab's selection.
    let directory = root.join("project");
    fs::create_dir(&directory).unwrap();
    let command = [
        "tab",
        "create",
        "--no-focus",
        "--json",
        "--dir",
        directory.to_str().unwrap(),
        "--",
        "hx",
    ];
    let uncalled = eon_command(&binary)
        .args(command)
        .env_remove("EON_TERMINAL")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert_eq!(uncalled.status.code(), Some(1), "{}", stdout(&uncalled));
    assert!(stdout(&uncalled).contains("caller-required"));
    assert_eq!(inspect("after-uncalled-tab"), snapshot);
    let launched = from(&caller, &command);
    assert!(launched.status.success(), "{}", stdout(&launched));
    let reply: serde_json::Value = serde_json::from_slice(&launched.stdout).unwrap();
    let after = inspect("after-launched-tab");
    let tab = after.tabs.last().unwrap();
    assert_eq!(
        reply["tabs"].as_array().unwrap().last().unwrap()["id"],
        tab.id
    );
    assert_eq!(after.tabs.len(), snapshot.tabs.len() + 1);
    assert!(!tab.pending);
    assert_eq!(tab.directory, directory.as_os_str().as_bytes());
    assert_eq!(tab.panes().count(), 1);
    let pane = tab.panes().next().unwrap();
    assert_eq!(tab.selected_pane.as_deref(), Some(pane.id.as_str()));
    names(&launched, &tab.id, Some(&pane.id));
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    assert!(process::cmdline(live_identity(endpoint).process_id).ends_with(b"hx\0"));
    assert!(fs::read_to_string(&orbit_log).unwrap().contains(&format!(
        "{} {}\n",
        pane.session,
        fs::canonicalize(&directory).unwrap().display()
    )));
    assert_eq!(after.active_tab, snapshot.active_tab);
    assert_eq!(after.tabs[0], snapshot.tabs[0]);
    let invalid = from(
        &caller,
        &[
            "tab",
            "create",
            "--json",
            "--dir",
            "/nonexistent",
            "--",
            "hx",
        ],
    );
    assert_eq!(invalid.status.code(), Some(1), "{}", stdout(&invalid));
    assert!(stdout(&invalid).contains("invalid-directory"));
    assert_eq!(inspect("after-invalid-tab"), after);

    // The next `eon edit` runs a newly configured editor with the file last.
    let settings = fs::read_to_string(config.join("config.toml")).unwrap();
    fs::write(
        config.join("config.toml"),
        format!("{settings}\n[editor]\ncommand = [\"my-editor\", \"--wait\"]\n"),
    )
    .unwrap();
    executable(&root.join("editor/my-editor"), "#!/bin/sh\nexit 0\n");
    let file = fs::canonicalize(&root).unwrap().join("notes.txt");
    let edited = from(&caller, &["edit", "--json", "--", file.to_str().unwrap()]);
    assert!(edited.status.success(), "{}", stdout(&edited));
    let edited = inspect("after-configured-edit");
    let tab = &edited.tabs[0];
    let pane = tab
        .panes()
        .find(|pane| Some(pane.id.as_str()) == tab.selected_pane.as_deref())
        .unwrap();
    let endpoint = Path::new(OsStr::from_bytes(&pane.endpoint));
    let orbit_arguments = process::cmdline(live_identity(endpoint).process_id);
    assert!(
        orbit_arguments
            .ends_with(&[b"my-editor\0--wait\0", file.as_os_str().as_bytes(), b"\0"].concat()),
        "{}",
        String::from_utf8_lossy(&orbit_arguments)
    );

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
    assert_eq!(stdout(&pane).matches("\"expanded\":").count(), 2);
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

    let before = invoke(&binary, &runtime, &config, &["session", "--json"]);
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
    let after = invoke(&binary, &runtime, &config, &["session", "--json"]);
    assert_eq!(
        stable_workspace_json(&before),
        stable_workspace_json(&after)
    );
    let human = invoke(&binary, &runtime, &config, &["session"]);
    assert!(human.status.success());
    assert!(stdout(&human).contains("active t1\n"));
    assert_eq!(stdout(&human).matches("  pane ").count(), 3);
    // Each pane names its terminal, and so does each busy line.
    assert_eq!(stdout(&human).matches("\nbusy ").count(), 3);
    assert_eq!(stdout(&human).matches(" terminal=terminal-").count(), 6);

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

    let snapshot = invoke(&binary, &runtime, &config, &["session", "--json"]);
    assert!(snapshot.status.success());
    let snapshot = stdout(&snapshot);
    assert!(snapshot.contains("\"active_tab\":\"t1\""));
    assert!(snapshot.contains("\"id\":\"p3\""));
    assert_eq!(snapshot.matches("\"expanded\":").count(), 3);

    for endpoint in ["orbit.sock", "terminal-2.sock", "terminal-3.sock"] {
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
    let second = live_identity(&generation.join("terminal-2.sock")).process_id;
    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "close", "p2", "--yes", "--json"],
    );
    assert!(
        closed.status.success(),
        "stdout={} stderr={}",
        stdout(&closed),
        String::from_utf8_lossy(&closed.stderr)
    );
    assert!(!stdout(&closed).contains("\"id\":\"p2\""));
    assert_eq!(stdout(&closed).matches("\"expanded\":").count(), 2);
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
    // The final tab's last pane ends the window.
    let final_pane = invoke(
        &binary,
        &runtime,
        &config,
        &["pane", "close", "p1", "--yes"],
    );
    assert!(final_pane.status.success(), "{}", stdout(&final_pane));
    let closed = stdout(&final_pane);
    assert!(
        closed.starts_with("Eon window ") && closed.ends_with(" closed\n"),
        "{closed}"
    );
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
    let launch_directory = fs::canonicalize(&launch).unwrap();
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
    wait_for_ready(&control);

    let snapshot = invoke(&binary, &runtime, &config, &["session", "--json"]);
    assert!(snapshot.status.success());
    assert!(stdout(&snapshot).contains(&format!(
        "\"directory\":\"{}\"",
        launch_directory.to_str().unwrap()
    )));

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminals_drop_the_launching_agents_tool_settings() {
    // Fails when the supervisor's terminal launch stops removing what Claude Code's
    // and Codex's tool runners give the commands they run (eon-yxo2y, EON-C9).
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\ncat >/dev/null\n");
    // A stand-in systemd user manager, first on the supervisor's PATH, whose desktop
    // session has its own locale.
    let desktop = root.join("desktop");
    fs::create_dir(&desktop).unwrap();
    executable(
        &desktop.join("systemctl"),
        "#!/bin/sh\n[ \"$*\" = '--user --no-pager show-environment' ] || exit 1\necho LANG=pt_BR.UTF-8\n",
    );

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let mut command = eon_command(&binary);
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "PATH").then_some(value))
        .flatten()
        .unwrap()
        .to_owned();
    let path = std::iter::once(desktop).chain(std::env::split_paths(&path));
    let child = command
        .env("PATH", std::env::join_paths(path).unwrap())
        .arg("run")
        .env_remove("CURL_CA_BUNDLE")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .envs([
            ("CLAUDECODE", "1"),
            ("GIT_EDITOR", "true"),
            ("CODEX_CI", "1"),
            ("PAGER", "cat"),
            ("LANG", "C.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
            ("EDITOR", "hx"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    wait_for_ready(&generation.join("eon.sock"));
    assert!(
        invoke(&binary, &runtime, &config, &["session", "--json"])
            .status
            .success()
    );
    let first = generation.join("orbit.sock");

    let environment = process::environ(live_identity(&first).process_id);
    let value = |name: &str| {
        environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(format!("{name}=").as_bytes()))
            .map(|value| String::from_utf8(value.to_vec()).unwrap())
    };
    for name in ["CLAUDECODE", "GIT_EDITOR", "CODEX_CI", "PAGER", "LC_ALL"] {
        assert_eq!(value(name), None, "{name} reaches the terminal");
    }
    // Fails when Codex's forced locale survives a desktop session that has its own.
    assert_eq!(value("LANG").as_deref(), Some("pt_BR.UTF-8"));
    assert_eq!(value("EDITOR").as_deref(), Some("hx"));
    assert_eq!(
        value("CURL_CA_BUNDLE"),
        None,
        "update-check certificates reach the terminal"
    );

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
    let supervisor_log = root.join("supervisor.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    let reporter = root.join("report-cwd");
    let session_bin = root.join("session-bin");
    for directory in [&first, &second, &disappearing, &config] {
        fs::create_dir(directory).unwrap();
    }
    let first_directory = fs::canonicalize(&first).unwrap();
    let second_directory = fs::canonicalize(&second).unwrap();
    fs::create_dir(&session_bin).unwrap();
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    executable(
        &reporter,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" >> \"$EON_TEST_CHILD_CWD_LOG\"\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
    );
    executable(&session_bin.join("zoxide"), "#!/bin/sh\nexit 0\n");
    executable(
        &session_bin.join("eon-files"),
        "#!/bin/sh\nwhile test ! -e \"$EON_TEST_STOP\"; do sleep 0.01; done\n",
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
    struct SupervisorLog(PathBuf);
    impl Drop for SupervisorLog {
        fn drop(&mut self) {
            if thread::panicking()
                && let Ok(log) = fs::read_to_string(&self.0)
            {
                eprintln!("supervisor stderr ({}):\n{log}", self.0.display());
            }
        }
    }
    let _supervisor_log = SupervisorLog(supervisor_log.clone());
    let child = eon_command(&binary)
        .arg("run")
        .current_dir(&first)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_SESSION_BIN", &session_bin)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_ORBIT_CWD_LOG", &orbit_log)
        .env("EON_TEST_CHILD_CWD_LOG", &child_log)
        .env("EON_TEST_RUN_CHILD", "1")
        .stdout(Stdio::null())
        .stderr(fs::File::create(&supervisor_log).unwrap())
        .spawn()
        .unwrap();
    let mut supervisor = TestProcess {
        child,
        stop: stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for_ready(&control);
    // The exiting Venus stub dismisses t1's pair, so the tab keeps its launch directory.
    wait_for_commit(&control, &first_directory);

    assert!(
        invoke(&binary, &runtime, &config, &["tab", "create"])
            .status
            .success()
    );
    assert!(
        invoke(
            &binary,
            &runtime,
            &config,
            &["tab", "directory", "t2", "--", first.to_str().unwrap()],
        )
        .status
        .success()
    );
    wait_for_commit(&control, &first);
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
        invoke(&binary, &runtime, &config, &["tab", "switch", "t1"])
            .status
            .success()
    );
    // A dead tab, a tab given to `focus` and a pane given to `tab switch`
    // fail with their exit statuses and change nothing.
    let selection = || {
        let session = invoke(&binary, &runtime, &config, &["session", "--json"]);
        let session: serde_json::Value = serde_json::from_slice(&session.stdout).unwrap();
        let tabs = session["tabs"].as_array().unwrap().iter();
        let selected = tabs.map(|tab| (tab["id"].clone(), tab["selected_pane"].clone()));
        (session["active_tab"].clone(), selected.collect::<Vec<_>>())
    };
    let before = selection();
    assert_eq!(before.0, "t1");
    let dead = invoke(
        &binary,
        &runtime,
        &config,
        &["tab", "switch", "t9", "--json"],
    );
    assert_eq!(dead.status.code(), Some(3));
    assert!(
        stdout(&dead).contains("\"code\":\"unknown-tab\""),
        "{}",
        stdout(&dead)
    );
    let focused_tab = invoke(&binary, &runtime, &config, &["focus", "t2", "--json"]);
    assert_eq!(focused_tab.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&focused_tab.stderr).contains("`eon tab switch t2`"),
        "{}",
        String::from_utf8_lossy(&focused_tab.stderr)
    );
    let pane_as_tab = invoke(&binary, &runtime, &config, &["tab", "switch", "p1"]);
    assert_eq!(pane_as_tab.status.code(), Some(2));
    assert_eq!(selection(), before);
    assert!(
        invoke(&binary, &runtime, &config, &["pane", "create"])
            .status
            .success()
    );
    assert!(
        invoke(&binary, &runtime, &config, &["tab", "switch", "t2"])
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
        invoke(&binary, &runtime, &config, &["tab", "switch", "t1"])
            .status
            .success()
    );
    let before = invoke(&binary, &runtime, &config, &["session", "--json"]);
    let failed = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(stdout(&failed).contains("\"code\":\"terminal-start\""));
    let after = invoke(&binary, &runtime, &config, &["session", "--json"]);
    assert_eq!(
        stable_workspace_json(&before),
        stable_workspace_json(&after)
    );
    // Pairs took terminal-2 and terminal-3, panes terminal-1 and 4 to 6.
    assert!(!generation.join("terminal-7.sock").exists());
    assert!(!artifact_path(&generation.join("terminal-7.sock"), ".record").exists());

    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&orbit_log)
        .unwrap_or_default()
        .lines()
        .count()
        < 6
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
        format!("terminal-1 {}", first_directory.display()),
        format!("terminal-2 {}", first_directory.display()),
        format!("terminal-4 {}", first_directory.display()),
        format!("terminal-5 {}", second_directory.display()),
        format!("terminal-6 {}", first_directory.display()),
    ] {
        assert!(orbit_directories.lines().any(|line| line == expected));
    }
    let child_directories = fs::read_to_string(&child_log).unwrap();
    assert_eq!(
        child_directories
            .lines()
            .filter(|line| *line == first_directory.to_string_lossy())
            .count(),
        3
    );
    assert_eq!(
        child_directories
            .lines()
            .filter(|line| *line == second_directory.to_string_lossy())
            .count(),
        1
    );

    fs::write(&stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_directory_records_one_zoxide_visit_unless_disabled() {
    let root = temporary_directory();
    let launch = root.join("launch");
    let visited = root.join("visited");
    let unrecorded = root.join("unrecorded");
    let runtime = root.join("runtime");
    let config = root.join("config");
    let stop = root.join("stop");
    let zoxide_log = root.join("zoxide.log");
    let session_bin = root.join("session-bin");
    for directory in [&launch, &visited, &unrecorded, &config, &session_bin] {
        fs::create_dir(directory).unwrap();
    }
    let launch_directory = fs::canonicalize(&launch).unwrap();
    let visited = fs::canonicalize(&visited).unwrap();
    let unrecorded = fs::canonicalize(&unrecorded).unwrap();
    let missing = fs::canonicalize(&root).unwrap().join("missing");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    executable(
        &session_bin.join("zoxide"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            zoxide_log.display()
        ),
    );
    executable(&session_bin.join("eon-files"), "#!/bin/sh\ncat\n");
    let settings = "[terminal]\nfont_fallbacks = []\n\n[anima]\nenabled = false\n";
    fs::write(config.join("config.toml"), settings).unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let child = eon_command(&binary)
        .arg("run")
        .current_dir(&launch)
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_SESSION_BIN", &session_bin)
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
    wait_for_ready(&control);
    // The exiting Venus stub dismisses the pair, which records no visit.
    let mut snapshot = wait_for_commit(&control, &launch_directory);
    let deadline = Instant::now() + Duration::from_secs(5);
    while snapshot.tabs[0].panes().next().is_none() {
        assert!(Instant::now() < deadline, "t1 has no pane: {snapshot:?}");
        thread::sleep(Duration::from_millis(10));
        let Response::Snapshot(next) = workspace_action(&control, "t1-pane", Action::Inspect)
        else {
            panic!("cannot inspect t1");
        };
        snapshot = next;
    }
    let visits = || {
        fs::read_to_string(&zoxide_log)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("add"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let recorded = vec![format!("add -- {}", visited.display())];

    let retarget = |directory: &Path| {
        invoke(
            &binary,
            &runtime,
            &config,
            &["tab", "directory", "t1", "--", directory.to_str().unwrap()],
        )
    };
    let output = retarget(&visited);
    assert!(output.status.success(), "{output:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while visits().is_empty() {
        assert!(Instant::now() < deadline, "no zoxide visit was recorded");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(visits(), recorded);

    let output = retarget(&missing);
    assert!(!output.status.success(), "{output:?}");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(visits(), recorded);

    fs::write(
        config.join("config.toml"),
        format!("{settings}\n[shell]\nzoxide = false\n"),
    )
    .unwrap();
    let output = retarget(&unrecorded);
    assert!(output.status.success(), "{output:?}");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(visits(), recorded);

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

        let peer = live_identity(&generation.join("terminal-2.sock"));
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
            let snapshot = invoke(&binary, &runtime, &config, &["session", "--json"]);
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
        assert_eq!(live_identity(&generation.join("terminal-2.sock")), peer);
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
    let terminals = ["orbit.sock", "terminal-2.sock"]
        .map(|endpoint| live_identity(&generation.join(endpoint)).process_id);

    let closed = invoke(
        &binary,
        &runtime,
        &config,
        &["tab", "close", "t1", "--yes", "--json"],
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
    let attach_log = root.join("attach.log");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    // Each Venus keeps its input. Told to detach, it exits 0, as Venus does on Alt+Shift+D.
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
            .env("EON_TEST_ATTACH_LOG", &attach_log)
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
    // The Venus that Eon starts is the one each terminal admits: named at
    // launch, and each new terminal before the reply that shows it.
    assert_named(
        &attach_log,
        &["terminal-1".into()],
        first_venus,
        Duration::from_secs(5),
    );
    let mut later = Vec::new();
    for id in ["s2", "s3"] {
        let created = snapshot(workspace_action(
            &control,
            &format!("new-{id}"),
            Action::NewWorkspace {
                name: None,
                focus: true,
            },
        ));
        assert_eq!(created.active_workspace, id);
        let sessions = snapshot_sessions(&created);
        assert!(!sessions.is_empty());
        assert_named(&attach_log, &sessions, first_venus, Duration::ZERO);
        later.extend(sessions);
    }
    // Shown s1, s2, s3, then s1 again: closing s1 shows s3, the most recent, rather than
    // its following sibling or the first remaining workspace, both s2.
    let first = snapshot(workspace_action(
        &control,
        "back",
        Action::SwitchWorkspace {
            workspace: eon_workspace_protocol::v39::WorkspaceTarget::Selector("s1".into()),
        },
    ));
    assert_eq!(first.tabs.len(), 1);
    let closed = snapshot(workspace_action(
        &control,
        "close-s1",
        Action::CloseTab {
            tab: first.active_tab.clone(),
        },
    ));
    assert_eq!(
        (closed.workspace.as_str(), closed.active_workspace.as_str()),
        ("s3", "s3")
    );
    assert_eq!(ids(&closed), ["s2", "s3"]);
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(process::exists(first_venus));

    // Detaching keeps every workspace, and attach presents the one last shown.
    fs::write(root.join(format!("venus-input.{first_venus}.detach")), "").unwrap();
    wait_for_process_removal(first_venus, "Venus did not detach");
    assert!(
        eon(&["window", "attach", "default"])
            .status()
            .unwrap()
            .success()
    );
    let second_venus = venus_pid(2);
    // The presented Venus is named to every live terminal before attach
    // replies.
    assert_named(&attach_log, &later, second_venus, Duration::ZERO);
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
        ("s3", "s3")
    );
    assert_eq!(ids(&presented), ["s2", "s3"]);

    // Only the last workspace's final close ends the window.
    let emptied = snapshot(workspace_action(
        &control,
        "close-s3",
        Action::CloseTab {
            tab: presented.active_tab.clone(),
        },
    ));
    assert_eq!(ids(&emptied), ["s2"]);
    assert_eq!(emptied.active_workspace, "s2");
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(matches!(
        workspace_action(
            &control,
            "close-s2",
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
    wait_for_ready(&control);
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
        .env("EON_TEST_STOP", root.join("second-stop"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut second = TestProcess {
        child: second,
        stop: root.join("second-stop"),
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
    wait_for_ready(&control);

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
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
    wait_for_ready(&control);
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

    let refused = TestProcess {
        child: replacement().spawn().unwrap(),
        stop: stop.clone(),
    };
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

    let waiting = TestProcess {
        child: replacement().spawn().unwrap(),
        stop: stop.clone(),
    };
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
    wait_for_ready(&control);
    assert_ne!(live_identity(&presentation).run_id, reconciled.run_id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(&orbit_log).unwrap().lines().count() != 2 {
        assert!(Instant::now() < deadline, "fresh Session count changed");
        thread::sleep(Duration::from_millis(10));
    }
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut recovered.child);
    assert!(!generation.exists());
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
    let attach_log = root.join("attach.log");
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
            .env("EON_TEST_ATTACH_LOG", &attach_log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = TestProcess {
        child: launch(),
        stop: fallback_stop.clone(),
    };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    wait_for(&venus_log);
    let pane = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(pane.status.success(), "{}", stdout(&pane));
    let initial = [
        live_identity(&generation.join("orbit.sock")),
        live_identity(&generation.join("terminal-2.sock")),
    ];
    let first_venus_log = fs::read_to_string(&venus_log).unwrap();
    let first_venus: u32 = first_venus_log.split('|').next().unwrap().parse().unwrap();
    assert!(first_venus_log.contains("|--no-decorations "));
    assert_named(
        &attach_log,
        &["terminal-1".into(), "terminal-2".into()],
        first_venus,
        Duration::ZERO,
    );

    first.child.kill().unwrap();
    assert!(!first.child.wait().unwrap().success());
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

    let mut replacement = TestProcess {
        child: launch(),
        stop: fallback_stop.clone(),
    };
    wait_for_ready(&control);
    // The replacement names its own Venus to the terminals it recovered.
    let deadline = Instant::now() + Duration::from_secs(5);
    let replacement_venus: u32 = loop {
        if let Some(line) = fs::read_to_string(&venus_log).unwrap().lines().nth(1) {
            break line.split('|').next().unwrap().parse().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the replacement started no Venus"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_named(
        &attach_log,
        &["terminal-1".into(), "terminal-2".into()],
        replacement_venus,
        Duration::from_secs(5),
    );
    let recovered = invoke(&binary, &runtime, &config, &["session", "--json"]);
    assert!(recovered.status.success(), "{}", stdout(&recovered));
    let recovered = stdout(&recovered);
    assert!(recovered.contains("\"active_tab\":\"t1\""));
    assert!(recovered.contains("\"selected_pane\":\"p1\""));
    assert_eq!(recovered.matches("\"expanded\":").count(), 2);
    assert!(recovered.contains("\"id\":\"p1\""));
    assert!(recovered.contains("\"id\":\"p2\""));
    assert!(recovered.contains("\"popups\":[]"));
    assert_eq!(
        [
            live_identity(&generation.join("orbit.sock")),
            live_identity(&generation.join("terminal-2.sock")),
        ],
        initial
    );
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 2);

    let created = invoke(&binary, &runtime, &config, &["pane", "create", "--json"]);
    assert!(created.status.success(), "{}", stdout(&created));
    assert!(stdout(&created).contains("\"id\":\"p3\""));
    assert!(stdout(&created).contains("\"terminal\":\"terminal-3\""));
    assert_eq!(fs::read_to_string(&orbit_log).unwrap().lines().count(), 3);
    assert_named(
        &attach_log,
        &["terminal-3".into()],
        replacement_venus,
        Duration::ZERO,
    );

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    assert!(
        stdout(&stopped).contains("\"terminals\":[\"terminal-1\",\"terminal-2\",\"terminal-3\"]")
    );
    wait_for_successful_exit(&mut replacement.child);
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
    let orbit_pid: u32 = fs::read_to_string(&orbit_log)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!process::exists(orbit_pid));
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
            "terminal-2",
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
        let orbit_pid: u32 = fs::read_to_string(&orbit_log)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!process::exists(orbit_pid));
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
    let orbit_pid: u32 = fs::read_to_string(&orbit_log)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(process::exists(orbit_pid), "marked Ready helper was killed");

    fs::write(&fallback_stop, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while process::exists(orbit_pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !process::exists(orbit_pid),
        "marked Ready helper did not exit"
    );
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
    wait_for_ready(&control);
    let orbit_pid = live_identity(&generation.join("orbit.sock")).process_id;
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let request = stop::encode_request(generation_id).unwrap();
    let mut stream = UnixStream::connect(control).unwrap();
    stream.write_all(&request).unwrap();
    stream.shutdown(Shutdown::Both).unwrap();
    drop(stream);

    wait_for_successful_exit(&mut supervisor.child);
    assert!(!generation.exists());
    assert!(!process::exists(orbit_pid));
    fs::remove_dir_all(root).unwrap();
}

/// Catches a supervisor that stops on a Reserved Stop for another
/// generation: it answers `generation-mismatch` in the reserved framing and
/// keeps running until its own generation's Stop (EON-C11).
#[test]
fn a_reserved_stop_for_another_generation_stops_nothing() {
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
    wait_for_ready(&control);
    let orbit_pid = live_identity(&generation.join("orbit.sock")).process_id;
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let other = format!("g1-{:032x}", 9);
    assert_ne!(other, generation_id);
    let exchange = |target: &str| {
        let mut stream = UnixStream::connect(&control).unwrap();
        stream
            .write_all(&stop::encode_request(target).unwrap())
            .unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        stop::decode_reply(&reply).unwrap()
    };

    let refused = exchange(&other);
    assert!(
        matches!(&refused, stop::Reply::Failure { code, .. } if code == "generation-mismatch"),
        "{refused:?}"
    );
    thread::sleep(Duration::from_millis(200));
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(process::exists(orbit_pid));

    assert!(matches!(
        exchange(generation_id),
        stop::Reply::Stopped { generation, .. } if generation == generation_id
    ));
    wait_for_successful_exit(&mut supervisor.child);
    assert!(!process::exists(orbit_pid));
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

    let mut confirmation = TestProcess {
        child: eon_command(&binary)
            .args(["window", "stop", "default"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(fs::File::create(&prompt).unwrap()))
            .spawn()
            .unwrap(),
        stop: first_stop.clone(),
    };
    wait_for(&prompt);

    let stopped_first = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
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
        .child
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
    let workspace_live = invoke(&binary, &runtime, &config, &["session", "--json"])
        .status
        .success();
    let stopped_second = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
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

    let listed = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
    assert!(listed.status.success());
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    let listed: serde_json::Value = serde_json::from_str(stdout(&listed)).unwrap();
    let window = &listed["windows"][0];
    assert_eq!(window["generation"], generation_id, "{listed}");
    assert_eq!(window["current"], true, "{listed}");
    assert_eq!(window["state"], "live", "{listed}");
    assert_eq!(
        window["terminals"],
        serde_json::json!(["terminal-1"]),
        "{listed}"
    );
    assert_eq!(window["stop"]["available"], true, "{listed}");

    overfill_generation_directory(&runtime);
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(
        stopped.status.success(),
        "stdout={} stderr={}",
        stdout(&stopped),
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(stdout(&stopped).contains(&format!(
        "\"generation\":\"{generation_id}\",\"terminals\":[\"terminal-1\"]"
    )));
    if first_is_supervisor {
        wait_for_successful_exit(&mut first.child);
    } else {
        wait_for_successful_exit(&mut second.child);
    }
    fs::remove_dir_all(root).unwrap();
}

fn quit_command(root: &Path) -> Command {
    let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
    command
        .env("EON_RUNTIME_DIR", root.join("r"))
        .env("EON_CONFIG_HOME", root.join("config"))
        .env("EON_ORBIT", root.join("orbit"))
        .env("EON_VENUS", root.join("venus"))
        .env("EON_TEST_STOP", root.join("stop"))
        .env("EON_TEST_ORBIT_FOREGROUND", root.join("foreground"))
        .env_remove("EON_WINDOW_RUNTIME_DIR")
        .env_remove("EON_TERMINAL");
    command
}

/// Waits until the busy terminals of the window at `window`, else of the
/// default window, are exactly `panes`.
fn wait_for_busy_panes(root: &Path, window: Option<&Path>, panes: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut command = quit_command(root);
        if let Some(window) = window {
            command.env("EON_WINDOW_RUNTIME_DIR", window);
        }
        let output = command.args(["session", "--json"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let busy = &value["busy"];
        let shown = busy["terminals"].as_array().map(|terminals| {
            terminals
                .iter()
                .map(|terminal| terminal["pane"].as_str().unwrap_or("popup"))
                .collect::<Vec<_>>()
        });
        if shown.is_some_and(|shown| shown == panes) && busy["omitted"] == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "busy terminals stay {busy}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn quit_workspace() -> (PathBuf, TestProcess) {
    // Independent windows need short socket paths on macOS too.
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("eq{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
    configure_mock_desktop(&root.join("config"));
    managed_orbit_executable(&root.join("orbit"));
    executable(&root.join("venus"), "#!/bin/sh\nexit 0\n");
    let supervisor = TestProcess {
        child: quit_command(&root)
            .arg("run")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: root.join("stop"),
    };
    wait_for_ready(&generation_runtime(&root.join("r")).join("eon.sock"));
    (root, supervisor)
}

fn open_quit_window(root: &Path) -> String {
    let output = quit_command(root).args(["window", "new"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    stdout(&output).trim().to_owned()
}

fn quit_caller(namespace: &Path) -> Vec<u8> {
    quit_terminal_caller(&generation_runtime(namespace).join("orbit.sock"))
}

fn quit_terminal_caller(endpoint: &Path) -> Vec<u8> {
    let environment = process::environ(live_identity(endpoint).process_id);
    environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .unwrap()
        .to_vec()
}

fn quit_with_answers(command: &mut Command, answers: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(answers).unwrap();
    child.wait_with_output().unwrap()
}

fn wait_for_generation_removal(generation: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while generation.exists() {
        assert!(
            Instant::now() < deadline,
            "stopped generation remains: {generation:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn quit_from_a_terminal_stops_only_the_callers_window() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let first = open_quit_window(&root);
    let second = open_quit_window(&root);
    let first_root = runtime.join("w").join(&first);
    let second_root = runtime.join("w").join(&second);
    let first_generation = generation_runtime(&first_root);
    let second_generation = generation_runtime(&second_root);
    let caller = quit_caller(&second_root);
    for caller in [b"bad caller".as_slice(), b"terminal-9999-0000000000000000"] {
        let output = quit_command(&root)
            .env("EON_WINDOW_RUNTIME_DIR", &second_root)
            .env("EON_TERMINAL", OsStr::from_bytes(caller))
            .args(["quit", "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(stdout(&output).contains("unknown-caller"));
        assert!(first_generation.exists() && second_generation.exists());
    }
    let output = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &second_root)
        .env("EON_TERMINAL", OsStr::from_bytes(&caller))
        .args(["quit", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("\"stopped\""));
    wait_for_generation_removal(&second_generation);
    assert!(first_generation.exists());
    assert!(default.child.try_wait().unwrap().is_none());
    let output = quit_with_answers(
        quit_command(&root)
            .env("EON_WINDOW_RUNTIME_DIR", &first_root)
            .env("EON_TERMINAL", OsStr::from_bytes(&quit_caller(&first_root)))
            .arg("quit"),
        b"y\n",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // The window is named by its short ID, at least 4 of its hex digits.
    let shown = stdout(&output)
        .strip_prefix("stopped window ")
        .and_then(|rest| rest.split_once(": "))
        .map(|(shown, _)| shown.to_owned())
        .unwrap();
    assert!(shown.len() >= 4 && first.starts_with(&shown), "{output:?}");
    wait_for_generation_removal(&first_generation);
    assert!(default.child.try_wait().unwrap().is_none());
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_from_default_and_commands_popup_callers_stops_only_their_windows() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let default_generation = generation_runtime(&runtime);
    let commands = root.join("commands");
    executable(&commands, "#!/bin/sh\nexit 0\n");
    fs::write(
        root.join("config/config.toml"),
        format!(
            "[terminal]\nfont_fallbacks = []\n[anima]\nenabled = false\n[popups.commands]\ncommand = [\"{}\"]\n",
            commands.display()
        ),
    )
    .unwrap();
    let first = open_quit_window(&root);
    let second = open_quit_window(&root);
    let first_root = runtime.join("w").join(&first);
    let first_generation = generation_runtime(&first_root);
    let second_generation = generation_runtime(&runtime.join("w").join(&second));
    let stopped = quit_command(&root)
        .env("EON_TERMINAL", OsStr::from_bytes(&quit_caller(&runtime)))
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    assert!(stdout(&stopped).contains("\"stopped\""));
    wait_for_successful_exit(&mut default.child);
    wait_for_generation_removal(&default_generation);
    assert!(first_generation.exists() && second_generation.exists());
    let Response::Snapshot(opened) = protocol_action(
        &first_generation.join("eon.sock"),
        "quit-commands-popup-open",
        ProtocolAction::InvokePopup {
            tab: "t1".into(),
            entry: "commands".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
    ) else {
        panic!("Commands popup did not open");
    };
    let popup = opened.tabs[0]
        .popups
        .iter()
        .find(|popup| popup.entry == "commands")
        .unwrap();
    let caller = quit_terminal_caller(Path::new(OsStr::from_bytes(&popup.endpoint)));
    assert_ne!(caller, quit_caller(&first_root));
    let stopped = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &first_root)
        .env("EON_TERMINAL", OsStr::from_bytes(&caller))
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    assert!(stdout(&stopped).contains("\"stopped\""));
    wait_for_generation_removal(&first_generation);
    assert!(second_generation.exists());
    let stopped = quit_command(&root)
        .args(["window", "stop", &second, "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_generation_removal(&second_generation);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn popup_hide_hides_a_popup_and_refuses_unknown_ones() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let git = root.join("git");
    executable(
        &git,
        format!(
            "#!/bin/sh\nwhile test ! -e '{}'; do sleep 0.01; done\n",
            root.join("stop").display()
        ),
    );
    fs::write(
        root.join("config/config.toml"),
        format!(
            "[terminal]\nfont_fallbacks = []\n[anima]\nenabled = false\n[popups.git]\ncommand = [\"{}\"]\n",
            git.display()
        ),
    )
    .unwrap();
    let window = open_quit_window(&root);
    let window_root = runtime.join("w").join(&window);
    let generation = generation_runtime(&window_root);
    let control = generation.join("eon.sock");
    let Response::Snapshot(opened) = protocol_action(
        &control,
        "popup-hide-open",
        ProtocolAction::InvokePopup {
            tab: "t1".into(),
            entry: "git".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
    ) else {
        panic!("Git popup did not open");
    };
    let popup = opened.tabs[0]
        .popups
        .iter()
        .find(|popup| popup.entry == "git")
        .unwrap()
        .clone();
    assert_eq!(opened.tabs[0].selected_popup.as_ref(), Some(&popup.id));
    let host = live_identity(Path::new(OsStr::from_bytes(&popup.endpoint))).process_id;

    let hidden = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &window_root)
        .args(["popup", "hide", &popup.id, "--json"])
        .output()
        .unwrap();
    assert_eq!(hidden.status.code(), Some(0), "{hidden:?}");
    let Response::Snapshot(hidden) =
        workspace_action(&control, "popup-hide-inspect", Action::Inspect)
    else {
        panic!("cannot inspect the hidden popup");
    };
    let tab = hidden.tabs.iter().find(|tab| tab.id == "t1").unwrap();
    assert_eq!(tab.selected_popup, None);
    assert!(tab.popups.iter().any(|listed| listed.id == popup.id));
    assert!(process::exists(host), "the hidden popup's terminal stopped");
    // Only t1's shown popup changed.
    let mut expected = stable_tabs(&opened.tabs);
    expected[0].selected_popup = None;
    assert_eq!(hidden.active_tab, opened.active_tab);
    assert_eq!(stable_tabs(&hidden.tabs), expected);

    let usage = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &window_root)
        .args(["popup", "hide", "--json"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");

    let refused = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &window_root)
        .args(["popup", "hide", "u999", "--json"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    assert!(
        stdout(&refused).contains("\"code\":\"unknown-popup\""),
        "{refused:?}"
    );
    let Response::Snapshot(unchanged) =
        workspace_action(&control, "popup-hide-unknown-inspect", Action::Inspect)
    else {
        panic!("cannot inspect after the refused hide");
    };
    assert_eq!(unchanged.active_tab, hidden.active_tab);
    assert_eq!(stable_tabs(&unchanged.tabs), stable_tabs(&hidden.tabs));
    assert!(process::exists(host));

    let stopped = quit_command(&root)
        .args(["window", "stop", &window, "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_generation_removal(&generation);
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sidebar_commands_show_hide_and_toggle() {
    let (root, mut default) = quit_workspace();
    let sidebar = |window: Option<&Path>, verb: &str| {
        let mut command = quit_command(&root);
        if let Some(window) = window {
            command.env("EON_WINDOW_RUNTIME_DIR", window);
        }
        command.args(["sidebar", verb, "--json"]).output().unwrap()
    };
    // The sidebar starts open; show and hide repeat without flipping it back.
    for (verb, state) in [
        ("hide", "closed"),
        ("hide", "closed"),
        ("show", "open"),
        ("show", "open"),
        ("toggle", "closed"),
        ("toggle", "open"),
    ] {
        let output = sidebar(None, verb);
        assert_eq!(output.status.code(), Some(0), "{verb}: {output:?}");
        assert!(
            stdout(&output).contains(&format!("\"sidebar\":\"{state}\"")),
            "{verb} should leave the sidebar {state}: {output:?}"
        );
    }
    let mut hides = (0..2)
        .map(|_| {
            quit_command(&root)
                .args(["sidebar", "hide", "--json"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for hide in hides.drain(..) {
        let output = hide.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            stdout(&output).contains("\"sidebar\":\"closed\""),
            "{output:?}"
        );
    }
    let usage = quit_command(&root)
        .args(["sidebar", "open"])
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");

    // A window whose sidebar is off: hide has nothing to do, show and toggle cannot.
    fs::write(
        root.join("config/config.toml"),
        "[terminal]\nfont_fallbacks = []\n[anima]\nenabled = false\n[sidebar]\nenabled = false\n",
    )
    .unwrap();
    let window = open_quit_window(&root);
    let window_root = root.join("r").join("w").join(&window);
    let hidden = sidebar(Some(&window_root), "hide");
    assert_eq!(hidden.status.code(), Some(0), "{hidden:?}");
    assert!(
        stdout(&hidden).contains("\"sidebar\":\"off\""),
        "{hidden:?}"
    );
    for verb in ["show", "toggle"] {
        let refused = sidebar(Some(&window_root), verb);
        assert_eq!(refused.status.code(), Some(1), "{verb}: {refused:?}");
        assert!(
            stdout(&refused).contains("\"code\":\"unavailable\""),
            "{verb}: {refused:?}"
        );
    }

    let generation = generation_runtime(&window_root);
    let stopped = quit_command(&root)
        .args(["window", "stop", &window, "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_generation_removal(&generation);
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_outside_eon_stops_the_only_window_and_refuses_several() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let generation = generation_runtime(&runtime);
    let first = open_quit_window(&root);
    let first_generation = generation_runtime(&runtime.join("w").join(&first));
    let output = quit_command(&root)
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "ambiguous-window");
    for detail in [
        "window default name=",
        "state=live",
        "terminals=1",
        "; eon window stop default\n",
        "eon quit --all",
    ] {
        assert!(
            result["error"]["detail"].as_str().unwrap().contains(detail),
            "{output:?}"
        );
    }
    // Each row names the window as `eon window ls` shows it, and the command
    // that stops it; no generation appears.
    let row = result["error"]["detail"]
        .as_str()
        .unwrap()
        .lines()
        .find(|line| line.starts_with("window ") && !line.starts_with("window default "))
        .unwrap()
        .to_owned();
    let shown = row["window ".len()..].split(' ').next().unwrap();
    assert!(first.starts_with(shown) && shown.len() >= 4, "{row}");
    assert!(
        row.ends_with(&format!("; eon window stop {shown}")),
        "{row}"
    );
    assert!(
        !stdout(&output).contains(generation.file_name().unwrap().to_str().unwrap()),
        "{output:?}"
    );
    assert!(default.child.try_wait().unwrap().is_none());
    assert!(first_generation.exists());
    let human = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(&first))
        .arg("quit")
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1), "{human:?}");
    let detail = String::from_utf8_lossy(&human.stderr);
    assert!(detail.contains("error ambiguous-window:"), "{detail}");
    assert!(detail.contains("\nwindow "), "{detail}");
    assert!(!detail.contains(r"\n"), "{detail}");
    assert!(detail.contains("; eon window stop default\n"), "{detail}");
    assert!(
        !detail.contains(generation.file_name().unwrap().to_str().unwrap()),
        "{detail}"
    );
    let corrupt = runtime.join("generations").join("bad\n\u{1b}[31m");
    fs::create_dir(&corrupt).unwrap();
    let output = quit_command(&root).arg("quit").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let detail = String::from_utf8_lossy(&output.stderr);
    assert!(!detail.contains('\u{1b}'), "{detail:?}");
    assert!(
        detail.contains("state=corrupt terminals=unknown"),
        "{detail}"
    );
    // The listing keeps the damaged entry's raw name as its ID, escaped in text.
    let listed = quit_command(&root).args(["window", "ls"]).output().unwrap();
    assert!(listed.status.success(), "{listed:?}");
    assert!(!stdout(&listed).contains('\u{1b}'), "{listed:?}");
    assert!(
        stdout(&listed)
            .lines()
            .any(|line| line.starts_with(r"bad\n\u{1b}[31m ") && line.ends_with(" no (corrupt)")),
        "{}",
        stdout(&listed)
    );
    let listed = quit_command(&root)
        .args(["window", "ls", "--json"])
        .output()
        .unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(
        listed["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| window["id"] == "bad\n\u{1b}[31m"
                && window["short_id"] == "bad\n\u{1b}[31m"
                && window["state"] == "corrupt"),
        "{listed}"
    );
    fs::remove_dir(corrupt).unwrap();
    // Rows name short IDs, so it takes this many to pass 1,024 bytes.
    let others = (0..14).map(|_| open_quit_window(&root)).collect::<Vec<_>>();
    let output = quit_command(&root)
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let detail = result["error"]["detail"].as_str().unwrap();
    // Every window keeps its row past the EONW detail limit, by the ID
    // `eon window ls` shows.
    for id in &others {
        assert!(
            detail.lines().any(|line| {
                line.strip_prefix("window ")
                    .and_then(|rest| rest.split_once(' '))
                    .is_some_and(|(shown, _)| {
                        id.starts_with(shown)
                            && shown.len() >= 4
                            && line.ends_with(&format!("; eon window stop {shown}"))
                    })
            }),
            "{id}: {detail}"
        );
        assert!(generation_runtime(&runtime.join("w").join(id)).exists());
    }
    assert!(detail.contains("eon quit --all"), "{detail}");
    assert!(
        detail.len() > 1024,
        "this inventory exercises the EONW detail limit"
    );
    for id in &others {
        let generation = generation_runtime(&runtime.join("w").join(id));
        let output = quit_command(&root)
            .args(["window", "stop", id, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        wait_for_generation_removal(&generation);
    }
    assert!(
        quit_command(&root)
            .args(["window", "stop", &first, "--json"])
            .output()
            .unwrap()
            .status
            .success()
    );
    wait_for_generation_removal(&first_generation);
    let declined = quit_command(&root)
        .arg("quit")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    assert!(String::from_utf8_lossy(&declined.stderr).contains("not-confirmed"));
    assert!(default.child.try_wait().unwrap().is_none());
    let output = quit_with_answers(
        quit_command(&root)
            .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(&first))
            .arg("quit"),
        b"yes\n",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    wait_for_successful_exit(&mut default.child);
    let missing = quit_command(&root)
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(4), "{missing:?}");
    assert!(stdout(&missing).contains("missing-supervisor"));
    let lone = open_quit_window(&root);
    let lone_generation = generation_runtime(&runtime.join("w").join(&lone));
    let output = quit_command(&root)
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("\"stopped\""));
    wait_for_generation_removal(&lone_generation);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_reports_unknown_counts_and_preserves_unavailable_discovery() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let current = generation_runtime(&runtime);
    let id = format!("g1-{:032x}", 11);
    let listener = generation_listener(&runtime, &id);
    // An older supervisor rejects inspections in its own framing, as in the lifecycle harness.
    let server = thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(matches!(
                decode_request(&read_request(&mut stream)).unwrap().action,
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime)
            ));
            let mut failure = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: format!("unsupported EONW version {VERSION}"),
            }))
            .unwrap();
            failure[4..6].copy_from_slice(&11u16.to_le_bytes());
            stream.write_all(&failure).unwrap();
        }
    });
    let output = quit_command(&root)
        .args(["window", "ls", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let short = listed["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["generation"] == id)
        .unwrap()["short_id"]
        .as_str()
        .unwrap();
    for json in [false, true] {
        let mut command = quit_command(&root);
        command.arg("quit");
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let detail = if json {
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["error"]["code"], "ambiguous-window");
            result["error"]["detail"].as_str().unwrap().to_owned()
        } else {
            String::from_utf8(output.stderr).unwrap()
        };
        assert!(
            detail.contains(&format!(
                "window {short} name=- state=incompatible terminals=unknown"
            )),
            "{detail}"
        );
        assert!(detail.contains("state=live terminals=1"), "{detail}");
        assert!(current.exists());
        assert!(default.child.try_wait().unwrap().is_none());
    }
    let output = quit_command(&root)
        .env("EON_TERMINAL", "unrecognized-caller")
        .args(["quit", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "supervisor-unavailable");
    assert!(current.exists());
    assert!(default.child.try_wait().unwrap().is_none());
    server.join().unwrap();
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_all_stops_the_default_namespace_and_every_window_caller_last() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let first = open_quit_window(&root);
    let second = open_quit_window(&root);
    let first_root = runtime.join("w").join(&first);
    let generations = [
        generation_runtime(&runtime),
        generation_runtime(&first_root),
        generation_runtime(&runtime.join("w").join(&second)),
    ];
    let output = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &first_root)
        .env("EON_TERMINAL", OsStr::from_bytes(&quit_caller(&first_root)))
        .args(["quit", "--all", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["window"], "default");
    assert_eq!(results[1]["window"], second);
    assert_eq!(results[2]["window"], first);
    for result in results {
        assert!(result["results"][0]["stopped"].is_object(), "{result}");
    }
    for generation in generations {
        wait_for_generation_removal(&generation);
    }
    wait_for_successful_exit(&mut default.child);
    default.stop.clear();
    drop(default);

    let mut default = TestProcess {
        child: quit_command(&root)
            .arg("run")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: root.join("stop"),
    };
    wait_for_ready(&generation_runtime(&runtime).join("eon.sock"));
    let caller = open_quit_window(&root);
    let other = open_quit_window(&root);
    let caller_root = runtime.join("w").join(&caller);
    let caller_generation = generation_runtime(&caller_root);
    let other_generation = generation_runtime(&runtime.join("w").join(&other));
    // The caller's window asks only for a terminal besides the caller.
    let created = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &caller_root)
        .env(
            "EON_TERMINAL",
            OsStr::from_bytes(&quit_caller(&caller_root)),
        )
        .args(["pane", "create", "--no-focus", "--json"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let output = quit_with_answers(
        quit_command(&root)
            .env("EON_WINDOW_RUNTIME_DIR", &caller_root)
            .env(
                "EON_TERMINAL",
                OsStr::from_bytes(&quit_caller(&caller_root)),
            )
            .args(["quit", "--all"]),
        b"y\ny\nn\n",
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    // Results name each window as `eon window ls` shows it, in stop order:
    // the default namespace first, the caller's own window last.
    let stopped: Vec<&str> = stdout(&output)
        .lines()
        .map(|line| {
            line.strip_prefix("stopped window ")
                .and_then(|rest| rest.split_once(':'))
                .unwrap()
                .0
        })
        .collect();
    assert_eq!(stopped.len(), 2, "{output:?}");
    assert_eq!(stopped[0], "default");
    assert!(other.starts_with(stopped[1]) && stopped[1].len() >= 4);
    let prompts = String::from_utf8_lossy(&output.stderr);
    assert!(!prompts.contains("generation"), "{prompts}");
    assert!(
        prompts.contains("It stops 2 terminals; 1 still running: p2"),
        "{prompts}"
    );
    assert!(prompts.contains("not-confirmed"));
    wait_for_generation_removal(&other_generation);
    assert!(caller_generation.exists());
    wait_for_successful_exit(&mut default.child);
    let output = quit_command(&root)
        .args(["quit", "--all", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // Ended windows are not listed: only the caller's window still runs.
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["window"], caller.as_str());
    assert!(results[0]["results"][0]["stopped"].is_object());
    wait_for_generation_removal(&caller_generation);
    default.stop.clear();
    drop(default);
    let mut default = TestProcess {
        child: quit_command(&root)
            .arg("run")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: root.join("stop"),
    };
    let default_generation = generation_runtime(&runtime);
    wait_for_ready(&default_generation.join("eon.sock"));
    let independent = open_quit_window(&root);
    let independent_generation = generation_runtime(&runtime.join("w").join(&independent));
    let output = quit_command(&root)
        .env(
            "EON_WINDOW_RUNTIME_DIR",
            runtime.join("w").join(&independent),
        )
        .args(["quit", "--all", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = results.as_array().unwrap();
    assert_eq!(results.last().unwrap()["window"], "default");
    assert!(results.last().unwrap()["results"][0]["stopped"].is_object());
    let independent_result = results
        .iter()
        .find(|result| result["window"] == independent)
        .unwrap();
    assert!(independent_result["results"][0]["stopped"].is_object());
    wait_for_generation_removal(&independent_generation);
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fresh_window_pair_counts_other_running_windows_and_attach_shows_none() {
    // Independent windows need short socket paths on macOS too.
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("ep{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
    let runtime = root.join("r");
    let stop = root.join("stop");
    configure_mock_desktop(&root.join("config"));
    managed_orbit_executable(&root.join("orbit"));
    executable(&root.join("venus"), "#!/bin/sh\ncat >/dev/null\n");
    let bin = root.join("bin");
    let logs = root.join("logs");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&logs).unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    symlink(&binary, bin.join("eon-files")).unwrap();
    // Each pair's Yazi logs its arguments and its quick search's notice.
    executable(
        &root.join("yazi"),
        r#"#!/bin/sh
printf '%s\n' "$*" "$EON_FILES_NOTICE" > "$EON_TEST_YAZI_LOG.$$.tmp"
mv "$EON_TEST_YAZI_LOG.$$.tmp" "$EON_TEST_YAZI_LOG.$$"
while [ ! -e "$EON_TEST_STOP" ]; do sleep 0.01; done
"#,
    );
    let command = || {
        let mut command = quit_command(&root);
        command
            .current_dir(&root)
            .env("EON_SESSION_BIN", &bin)
            .env("EON_TEST_RUN_CHILD", "1")
            .env("EON_YAZI", root.join("yazi"))
            .env("EON_FILES_CONFIG", &root)
            .env("EON_TEST_YAZI_LOG", logs.join("yazi"));
        command
    };
    let mut first = TestProcess {
        child: command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let control = generation_runtime(&runtime).join("eon.sock");
    wait_for_ready(&control);
    let pair_logs = || {
        fs::read_dir(&logs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_none_or(|extension| extension != "tmp"))
            .map(|path| fs::read_to_string(path).unwrap())
            .collect::<Vec<_>>()
    };
    let wait_for_pairs = |count| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let logs = pair_logs();
            if logs.len() == count {
                break logs;
            }
            assert!(Instant::now() < deadline, "pair logs: {logs:?}");
            thread::sleep(Duration::from_millis(10));
        }
    };
    assert!(!wait_for_pairs(1)[0].contains("still running"));
    let name = root.file_name().unwrap().to_str().unwrap();
    let hint = "See them or quit one: Window menu (Eon icon, top left)\nCLI: eon window ls · eon window stop ID";
    let mut windows = Vec::new();
    for count in 1..=2 {
        let output = command().args(["window", "new"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        windows.push(stdout(&output).trim().to_owned());
        let logs = wait_for_pairs(count + 1);
        let text = if count == 1 {
            format!("1 other Eon window is still running: {name}\n{hint}")
        } else {
            format!("2 other Eon windows are still running: {name}, {name}\n{hint}")
        };
        assert!(logs.iter().any(|log| log.contains(&text)), "{logs:?}");
    }
    // A later tab's pair stays plain, in window A, which already lists D.
    let output = command()
        .args(["tab", "create", "--window", &windows[0]])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let logs = wait_for_pairs(4);
    assert_eq!(
        logs.iter()
            .filter(|log| log.contains("still running"))
            .count(),
        2,
        "{logs:?}"
    );
    for args in [
        vec!["window", "attach", &windows[0]],
        vec!["window", "attach", "default"],
    ] {
        let output = command().args(args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(pair_logs().len(), 4);
    }
    for window in &windows {
        let output = command()
            .args(["window", "stop", window, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let output = command()
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    wait_for_successful_exit(&mut first.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_without_a_busy_terminal_stops_without_asking() {
    let (root, mut default) = quit_workspace();
    let generation = generation_runtime(&root.join("r"));
    fs::write(root.join("foreground"), "child").unwrap();
    wait_for_busy_panes(&root, None, &[]);
    let output = quit_command(&root)
        .arg("quit")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(stdout(&output), "stopped window default: terminal-1\n");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[y/N]"));
    wait_for_successful_exit(&mut default.child);
    wait_for_generation_removal(&generation);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_asks_in_window_language_and_names_busy_terminals() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let generation = generation_runtime(&runtime);
    fs::write(root.join("foreground"), "other").unwrap();
    let caller = quit_caller(&runtime);
    let created = quit_command(&root)
        .env("EON_TERMINAL", OsStr::from_bytes(&caller))
        .args([
            "pane",
            "create",
            "--no-focus",
            "--json",
            "--",
            "sleep",
            "300",
        ])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    // The window's list names the caller too; only its own quit leaves it out.
    wait_for_busy_panes(&root, None, &["p1", "p2"]);
    let quit = || {
        let mut command = quit_command(&root);
        command
            .env("EON_TERMINAL", OsStr::from_bytes(&caller))
            .arg("quit");
        command
    };
    let declined = quit_with_answers(&mut quit(), b"n\n");
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    let prompt = String::from_utf8_lossy(&declined.stderr);
    assert!(
        prompt.starts_with("Quit this window? It stops 2 terminals; 1 still running: p2"),
        "{prompt}"
    );
    assert!(!prompt.contains("p1"), "{prompt}");
    assert!(
        prompt.contains("error not-confirmed: this window keeps running;"),
        "{prompt}"
    );
    assert!(generation.exists());
    assert!(default.child.try_wait().unwrap().is_none());
    let confirmed = quit_with_answers(&mut quit(), b"y\n");
    assert_eq!(confirmed.status.code(), Some(0), "{confirmed:?}");
    assert!(stdout(&confirmed).starts_with("stopped window default: "));
    wait_for_successful_exit(&mut default.child);
    wait_for_generation_removal(&generation);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_counts_a_silent_terminal_busy_after_3_s() {
    let (root, mut default) = quit_workspace();
    fs::write(root.join("foreground"), "child").unwrap();
    wait_for_busy_panes(&root, None, &[]);
    fs::write(root.join("foreground"), "silent").unwrap();
    thread::sleep(Duration::from_millis(3500));
    let asked = quit_command(&root)
        .arg("quit")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(asked.status.code(), Some(1), "{asked:?}");
    let prompt = String::from_utf8_lossy(&asked.stderr);
    assert!(prompt.starts_with("Quit window "), "{prompt}");
    assert!(
        prompt.contains("It stops 1 terminal; 1 still running: p1"),
        "{prompt}"
    );
    assert!(prompt.contains("error not-confirmed:"), "{prompt}");
    assert!(default.child.try_wait().unwrap().is_none());
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stop_keeps_its_always_ask_question() {
    let (root, mut default) = quit_workspace();
    fs::write(root.join("foreground"), "child").unwrap();
    wait_for_busy_panes(&root, None, &[]);
    let declined = quit_command(&root)
        .args(["window", "stop", "default"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    let prompt = String::from_utf8_lossy(&declined.stderr);
    assert!(
        prompt.starts_with("Stop window default and 1 live terminal [terminal-1]? [y/N] "),
        "{prompt}"
    );
    assert!(
        prompt.contains("error not-confirmed: no Stop sent for window default;"),
        "{prompt}"
    );
    assert!(default.child.try_wait().unwrap().is_none());
    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_all_asks_only_busy_windows() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    fs::write(root.join("foreground"), "child").unwrap();
    let window = open_quit_window(&root);
    let window_root = runtime.join("w").join(&window);
    let generations = [
        generation_runtime(&runtime),
        generation_runtime(&window_root),
    ];
    // A pane that runs its own command is busy beside an idle shell.
    let created = quit_command(&root)
        .env("EON_WINDOW_RUNTIME_DIR", &window_root)
        .env(
            "EON_TERMINAL",
            OsStr::from_bytes(&quit_caller(&window_root)),
        )
        .args([
            "pane",
            "create",
            "--no-focus",
            "--json",
            "--",
            "sleep",
            "300",
        ])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    wait_for_busy_panes(&root, None, &[]);
    wait_for_busy_panes(&root, Some(&window_root), &["p2"]);
    let output = quit_with_answers(quit_command(&root).args(["quit", "--all"]), b"y\n");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // The idle default window stops without a question; each result names
    // its window, `default` last outside Eon.
    let results = stdout(&output);
    let mut results = results.lines();
    let (shown, _) = results
        .next()
        .unwrap()
        .strip_prefix("stopped window ")
        .and_then(|rest| rest.split_once(':'))
        .unwrap();
    assert!(window.starts_with(shown) && shown.len() >= 4, "{shown}");
    assert!(
        results
            .next()
            .unwrap()
            .starts_with("stopped window default: ")
    );
    assert_eq!(results.next(), None);
    let prompts = String::from_utf8_lossy(&output.stderr);
    assert_eq!(prompts.matches("Quit window ").count(), 1, "{prompts}");
    assert!(
        prompts.contains("It stops 2 terminals; 1 still running: p2"),
        "{prompts}"
    );
    for generation in generations {
        wait_for_generation_removal(&generation);
    }
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn running_windows_name_each_other_and_refresh_on_start_and_end() {
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("er{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
    let runtime = root.join("r");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let command = || {
        let mut command = eon_command(&binary);
        command
            .current_dir(&root)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop);
        command
    };
    let mut first = TestProcess {
        child: command()
            .arg("run")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    let control = generation_runtime(&runtime).join("eon.sock");
    wait_for_ready(&control);
    let query = |window: Option<&str>| {
        let mut c = command();
        c.args(["session", "--json"]);
        if let Some(id) = window {
            c.args(["--window", id]);
        }
        let output = c.output().unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            stdout(&output),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["running"].clone()
    };
    let wait = |window: Option<&str>, expected: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let running = query(window);
            if running["others"]
                .as_array()
                .is_some_and(|others| others.len() == expected)
            {
                break running;
            }
            assert!(
                Instant::now() < deadline,
                "summary did not refresh: {running}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    };
    let single = wait(None, 0);
    assert!(single["this"]["window"].is_null());
    assert_eq!(single["this"]["live_terminals"], 1);
    assert_eq!(
        single["this"]["name"],
        root.file_name().unwrap().to_str().unwrap()
    );
    assert!(single["this"]["started_at"].as_u64().unwrap() > 0);
    // A burst of peer inspections must fit the same one-second deadline as discovery.
    let deadline = Instant::now() + Duration::from_secs(1);
    let clients: Vec<_> = (0..64)
        .map(|index| {
            let mut stream = UnixStream::connect(&control).unwrap();
            stream
                .write_all(
                    &encode_request(&Request {
                        id: format!("burst-{index}"),
                        caller: None,
                        action: ProtocolAction::Workspace(Action::InspectRuntime),
                    })
                    .unwrap(),
                )
                .unwrap();
            stream
        })
        .collect();
    for mut stream in clients {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .expect("queued inspections exceeded one second");
        // macOS refuses socket options once the supervisor has answered and
        // closed; the read then ends at once, without the timeout.
        if let Err(error) = stream.set_read_timeout(Some(remaining)) {
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{error}");
        }
        let response = read_request(&mut stream);
        assert!(matches!(
            eon_workspace_protocol::v39::decode_lifecycle_response(&response).unwrap(),
            LifecycleResponse::Runtime(_)
        ));
    }
    assert!(
        Instant::now() <= deadline,
        "queued inspections exceeded one second"
    );
    let output = command().args(["window", "new"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let second = stdout(&output).trim().to_owned();
    let second_control = generation_runtime(&runtime.join("w").join(&second)).join("eon.sock");
    wait_for_ready(&second_control);
    let a = wait(None, 1);
    let b = wait(Some(&second), 1);
    assert_eq!(a["others"][0]["window"], second);
    assert!(b["others"][0]["window"].is_null());
    assert_eq!(a["others"][0]["generation"], b["this"]["generation"]);
    assert_eq!(a["others"][0]["started_at"], b["this"]["started_at"]);
    assert_eq!(b["others"][0]["name"], a["this"]["name"]);
    assert!(matches!(
        workspace_action(
            &second_control,
            "rename",
            Action::RenameWorkspace {
                workspace: None,
                name: "second".into(),
            }
        ),
        Response::Snapshot(_)
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let running = query(None);
        if running["others"][0]["name"] == "second" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "renamed peer is stale: {running}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(
        protocol_action(
            &second_control,
            "terminal",
            ProtocolAction::LaunchTab {
                directory: root.as_os_str().as_bytes().to_vec(),
                argv: vec![],
                focus: false,
            }
        ),
        Response::Snapshot(_)
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let running = query(None);
        if running["others"][0]["live_terminals"] == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "peer terminal count is stale: {running}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let inventory = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
    let inventory: serde_json::Value = serde_json::from_slice(&inventory.stdout).unwrap();
    let listed = inventory["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["id"] == second.as_str())
        .unwrap();
    assert_eq!(listed["name"], "second", "{inventory}");
    assert_eq!(
        unsafe { libc::kill(first.child.id() as libc::pid_t, libc::SIGSTOP) },
        0
    );
    let output = command().args(["window", "new"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let third = stdout(&output).trim().to_owned();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let summary = query(Some(&second));
        if summary["others"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["window"].is_null() && entry["state"] == "unreachable")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "paused peer was not marked unreachable: {summary}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    // Let the initial candidate-list change and its in-flight probes settle.
    thread::sleep(Duration::from_millis(2200));
    assert_eq!(
        unsafe { libc::kill(first.child.id() as libc::pid_t, libc::SIGCONT) },
        0
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let summary = query(Some(&second));
        if summary["others"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["window"].is_null() && entry["state"] == "live")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "resumed peer stayed unreachable: {summary}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let three = wait(None, 2);
    let peers = three["others"].as_array().unwrap();
    assert!(peers[0]["window"].as_str().unwrap() < peers[1]["window"].as_str().unwrap());
    assert_eq!(three["omitted"], 0);
    wait(Some(&second), 2);
    wait(Some(&third), 2);
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", &third, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait(None, 1);
    first.child.kill().unwrap();
    first.child.wait().unwrap();
    wait(Some(&second), 0);
    thread::sleep(Duration::from_millis(1200));
    assert!(
        control.parent().unwrap().is_dir(),
        "crash unexpectedly removed the generation directory"
    );
    first.child = command()
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_ready(&control);
    wait(Some(&second), 1);
    assert!(
        query(None)["this"]["started_at"].as_u64().unwrap()
            > single["this"]["started_at"].as_u64().unwrap()
    );
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", &second, "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait(None, 0);
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut first.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_windows_keep_independent_supervisors_and_stop_by_window_id() {
    // A window's terminal sockets leave this root only about ten bytes within macOS's
    // 104-byte socket paths, so it takes the first free short name, not a pid and counter.
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("ew{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
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
    wait_for_ready(&default_control);
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
    wait_for_ready(&first_control);
    wait_for_ready(&second_control);
    let endpoint = second_control.parent().unwrap().join("orbit.sock");
    let environment = process::environ(live_identity(&endpoint).process_id);
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
            vec!["session", "--json"]
        } else {
            vec!["session", "--window", &second, "--json"]
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
        &["session", "--window", &second, "--json"],
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
    let listed = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let short = |id: &str| {
        listed["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed: {listed}"))["short_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let human = invoke(&binary, &runtime, &config, &["window", "ls"]);
    assert!(human.status.success());
    for id in [&first, &second] {
        let short = short(id);
        assert!(
            stdout(&human)
                .lines()
                .any(|line| line.starts_with(&format!("{short} ")) && line.ends_with(" yes")),
            "{}",
            stdout(&human)
        );
    }

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
        invoke(&binary, &runtime, &config, &["session", "--json"])
            .status
            .success()
    );
    assert!(matches!(
        workspace_action(&second_control, "still-live", Action::Inspect),
        Response::Snapshot(_)
    ));
    symlink(&config, runtime.join("w").join("aaaaaaaa")).unwrap();
    let unsafe_listing = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
    assert!(!unsafe_listing.status.success());
    assert!(String::from_utf8_lossy(&unsafe_listing.stderr).contains("owned private directory"));
    fs::remove_file(runtime.join("w").join("aaaaaaaa")).unwrap();

    let third = open();
    let declined = invoke(&binary, &runtime, &config, &["window", "stop", &second]);
    assert_eq!(declined.status.code(), Some(1));
    assert_eq!(stdout(&declined), "");
    assert!(String::from_utf8_lossy(&declined.stderr).contains("error not-confirmed:"));
    let (caller, other) = if second < third {
        (&second, &third)
    } else {
        (&third, &second)
    };
    // `window stop all` covers the default window too; every window asks.
    let declined = command()
        .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(caller))
        .args(["window", "stop", "all"])
        .output()
        .unwrap();
    assert_eq!(declined.status.code(), Some(1));
    assert_eq!(stdout(&declined), "");
    assert_eq!(
        String::from_utf8_lossy(&declined.stderr)
            .matches("error not-confirmed:")
            .count(),
        3
    );
    // Without `EON_TERMINAL` the CLI runs outside Eon, so `default` is the
    // caller's window and goes last; one answer per prompt.
    let mut batch = TestProcess {
        child: command()
            .env("EON_WINDOW_RUNTIME_DIR", runtime.join("w").join(caller))
            .args(["window", "stop", "all"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    batch
        .child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"y\ny\nn\n")
        .unwrap();
    let batch = batch.wait_with_output().unwrap();
    assert_eq!(batch.status.code(), Some(1), "{}", stdout(&batch));
    assert_eq!(stdout(&batch).matches("stopped window ").count(), 2);
    let prompts = String::from_utf8_lossy(&batch.stderr);
    let asked: Vec<&str> = prompts
        .split("Stop window ")
        .skip(1)
        .map(|rest| rest.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(asked.len(), 3, "{prompts}");
    assert!(
        caller.starts_with(asked[0]) && asked[0].len() >= 4,
        "{prompts}"
    );
    assert!(
        other.starts_with(asked[1]) && asked[1].len() >= 4,
        "{prompts}"
    );
    assert_eq!(asked[2], "default", "{prompts}");
    assert!(!prompts.contains("generation"), "{prompts}");
    assert!(prompts.contains("error not-confirmed: no Stop sent for window default"));

    // JSON lists the running windows only, ended ones left out, in stop
    // order with `default` last.
    let (fourth, fifth) = (open(), open());
    // Every window runs this build, so `previous` selects none of them.
    let previous = command()
        .args(["window", "stop", "previous", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(previous.status.success(), "{previous:?}");
    assert_eq!(stdout(&previous), "[]\n");
    let current = generation_runtime(&runtime);
    let batch = command()
        .args(["window", "stop", "all", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(batch.status.success(), "{}", stdout(&batch));
    let results: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
    let results = results.as_array().unwrap();
    let windows: Vec<&str> = results
        .iter()
        .map(|result| result["window"].as_str().unwrap())
        .collect();
    assert_eq!(
        windows,
        [
            fourth.as_str().min(fifth.as_str()),
            fourth.as_str().max(fifth.as_str()),
            "default"
        ]
    );
    for result in results {
        let stopped = result["results"].as_array().unwrap();
        assert_eq!(stopped.len(), 1, "{result}");
        assert!(stopped[0]["stopped"]["generation"].is_string(), "{result}");
    }
    assert_eq!(
        results[2]["results"][0]["stopped"]["generation"],
        current.file_name().unwrap().to_str().unwrap()
    );
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

/// A Unix millisecond time as UTC `%Y-%m-%d %H:%M`, through Howard Hinnant's
/// days-to-civil.
fn utc_minute(ms: u64) -> String {
    let seconds = ms / 1000;
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let minute = seconds % 86_400 / 60;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        minute / 60,
        minute % 60
    )
}

/// `eon window ls`'s rows after its header, each split into its cells.
fn window_rows(output: &Output) -> Vec<Vec<String>> {
    let text = stdout(output);
    assert!(
        text.lines().all(|line| !line.ends_with(' ')),
        "trailing space: {text:?}"
    );
    let rows: Vec<Vec<String>> = text
        .lines()
        .map(|line| {
            line.split("  ")
                .map(str::trim)
                .filter(|cell| !cell.is_empty())
                .map(String::from)
                .collect()
        })
        .collect();
    assert_eq!(
        rows[0],
        [
            "ID",
            "NAME",
            "VERSION",
            "STARTED",
            "TERMINALS",
            "COMPATIBLE"
        ],
        "{text}"
    );
    rows[1..].to_vec()
}

/// Catches the default window missing from `eon window ls`, its text and JSON
/// disagreeing, and one `window stop` form failing to stop the default window
/// or stopping more than its target.
#[test]
fn window_ls_lists_the_default_and_an_independent_window_and_one_stop_form_stops_either() {
    // Short root: an independent window's terminal sockets must fit macOS's socket paths.
    let root = (0_u32..)
        .map(|n| std::env::temp_dir().join(format!("ew{n:x}")))
        .find(|path| match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("{}: {error}", path.display()),
        })
        .unwrap();
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
            .env("EON_TEST_STOP", &stop)
            .env("TZ", "UTC");
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
    wait_for_ready(&default_control);
    let output = command().args(["window", "new"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let first = stdout(&output).trim().to_owned();
    let first_control = generation_runtime(&runtime.join("w").join(&first)).join("eon.sock");
    wait_for_ready(&first_control);
    let file_name = |path: &Path| path.file_name().unwrap().to_str().unwrap().to_owned();
    let default_generation = file_name(&generation_runtime(&runtime));
    let first_generation = file_name(first_control.parent().unwrap());
    let run = |args: &[&str]| command().args(args).output().unwrap();
    let listed = || {
        let output = run(&["window", "ls", "--json"]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["windows"]
            .as_array()
            .unwrap()
            .clone()
    };

    let windows = listed();
    assert_eq!(windows.len(), 2, "{windows:?}");
    let window = &windows[0];
    assert_eq!(window["id"], "default", "{window}");
    assert_eq!(window["short_id"], "default", "{window}");
    assert_eq!(window["current"], true, "{window}");
    assert_eq!(window["compatible"], true, "{window}");
    assert_eq!(window["state"], "live", "{window}");
    assert_eq!(
        window["generation"],
        default_generation.as_str(),
        "{window}"
    );
    assert!(window["executable"].is_null(), "{window}");
    assert!(window["name"].is_string(), "{window}");
    assert!(
        window["terminals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|terminal| terminal.as_str().unwrap().starts_with("terminal-")),
        "{window}"
    );
    let window = &windows[1];
    assert_eq!(window["id"], first.as_str(), "{window}");
    let short = window["short_id"].as_str().unwrap().to_owned();
    assert!(short.len() >= 4 && first.starts_with(&short), "{window}");
    assert_eq!(window["generation"], first_generation.as_str(), "{window}");
    assert_eq!(window["current"], true, "{window}");

    // The text table shows the JSON's windows, in its order.
    let human = run(&["window", "ls"]);
    assert!(human.status.success(), "{human:?}");
    assert!(human.stderr.is_empty(), "{human:?}");
    let rows = window_rows(&human);
    assert_eq!(rows.len(), 2, "{}", stdout(&human));
    for (row, window) in rows.iter().zip(&windows) {
        assert_eq!(
            row,
            &[
                window["short_id"].as_str().unwrap().to_owned(),
                window["name"].as_str().unwrap_or("-").to_owned(),
                window["eon_version"].as_str().unwrap().to_owned(),
                utc_minute(window["started_at"].as_u64().unwrap()),
                window["terminals"].as_array().unwrap().len().to_string(),
                "yes".to_owned(),
            ],
            "{}",
            stdout(&human)
        );
    }

    let attached = run(&["window", "attach"]);
    assert_eq!(attached.status.code(), Some(0), "{attached:?}");

    // One form stops the independent window, and only it.
    let stopped = run(&["window", "stop", &short, "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(stopped.as_array().unwrap().len(), 1, "{stopped}");
    assert_eq!(stopped[0]["window"], first.as_str(), "{stopped}");
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"],
        first_generation.as_str(),
        "{stopped}"
    );
    wait_for_generation_removal(first_control.parent().unwrap());
    assert!(matches!(
        workspace_action(&default_control, "still-live", Action::Inspect),
        Response::Snapshot(_)
    ));
    let windows = listed();
    assert_eq!(windows.len(), 1, "{windows:?}");
    assert_eq!(windows[0]["id"], "default");

    // The same form stops the default window.
    let stopped = run(&["window", "stop", "default", "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(stopped[0]["window"], "default", "{stopped}");
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"],
        default_generation.as_str(),
        "{stopped}"
    );
    wait_for_successful_exit(&mut default.child);

    assert_eq!(
        stdout(&run(&["window", "ls", "--json"])),
        "{\"windows\":[]}\n"
    );
    assert_eq!(stdout(&run(&["window", "ls"])), "no running Eon windows\n");
    let attached = run(&["window", "attach"]);
    assert_eq!(attached.status.code(), Some(4), "{attached:?}");
    assert!(
        String::from_utf8_lossy(&attached.stderr).contains("the default Eon window is not running"),
        "{attached:?}"
    );
    let stopped = run(&["window", "stop", "default", "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&stopped.stdout).unwrap(),
        serde_json::json!([{"window": "default", "results": []}])
    );
    fs::remove_dir_all(root).unwrap();
}

/// Catches a prefix that stops the wrong window, an ambiguous prefix that
/// stops anything, and bad syntax that reaches a supervisor.
#[test]
fn window_id_prefixes_select_one_window_and_refuse_ambiguous_unknown_or_short_ones() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let a = format!("g1-abcd1{:027x}", 1);
    let b = format!("g1-abcd2{:027x}", 2);
    let c = format!("g1-{:032x}", 3);
    let mut servers = vec![
        previous_supervisor(&runtime, &a),
        previous_supervisor(&runtime, &b),
    ];
    let windows = runtime.join("w");
    fs::create_dir(&windows).unwrap();
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    servers.push(previous_supervisor(&windows.join("abcd3000"), &c));
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let run = |args: &[&str]| {
        eon_command(&binary)
            .args(args)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .env("TZ", "UTC")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let listed = || {
        let output = run(&["window", "ls", "--json"]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["windows"]
            .as_array()
            .unwrap()
            .clone()
    };
    let field = |windows: &[serde_json::Value], key: &str| {
        windows
            .iter()
            .map(|window| window[key].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    let windows = listed();
    assert_eq!(field(&windows, "id"), [&a[3..], &b[3..], "abcd3000"]);
    assert_eq!(field(&windows, "short_id"), ["abcd1", "abcd2", "abcd3"]);
    for window in &windows {
        assert_eq!(window["current"], false, "{window}");
        assert_eq!(window["compatible"], false, "{window}");
    }
    let human = run(&["window", "ls"]);
    let rows = window_rows(&human);
    assert_eq!(rows.len(), 3, "{}", stdout(&human));
    for row in &rows {
        assert_eq!(row[3], "1970-01-01 00:00", "{row:?}");
        assert_eq!(row.last().unwrap(), "no (other build)", "{row:?}");
    }

    // An ambiguous prefix names each match and stops nothing.
    let ambiguous = run(&["window", "stop", "abcd", "--json"]);
    assert_eq!(ambiguous.status.code(), Some(3), "{ambiguous:?}");
    let failure: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "ambiguous-window-id", "{failure}");
    let detail = failure["error"]["detail"].as_str().unwrap();
    for short in ["abcd1", "abcd2", "abcd3"] {
        assert!(detail.contains(short), "{detail}");
    }
    assert_eq!(listed().len(), 3);
    let ambiguous = run(&["window", "stop", "abcd"]);
    assert_eq!(ambiguous.status.code(), Some(3), "{ambiguous:?}");
    assert!(ambiguous.stdout.is_empty(), "{ambiguous:?}");
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr).starts_with("error ambiguous-window-id:"),
        "{ambiguous:?}"
    );
    assert_eq!(run(&["window", "attach", "abcd"]).status.code(), Some(3));
    let unknown = run(&["window", "attach", "ffff"]);
    assert_eq!(unknown.status.code(), Some(3), "{unknown:?}");
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown-window"));
    let unknown = run(&["window", "stop", "ffff", "--json"]);
    assert_eq!(unknown.status.code(), Some(3), "{unknown:?}");
    let failure: serde_json::Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "unknown-window", "{failure}");
    for args in [
        &["window", "stop", "abc"][..],
        &["window", "stop", "ABCD"],
        &["window", "stop", "bad/id"],
        &["window", "attach", "abc"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
    assert_eq!(listed().len(), 3);

    // A unique prefix, a full ID and an independent window's prefix each stop one window.
    let stopped = run(&["window", "stop", "abcd2", "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(stopped[0]["window"], &b[3..], "{stopped}");
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"],
        b.as_str()
    );
    let windows = listed();
    assert_eq!(field(&windows, "id"), [&a[3..], "abcd3000"]);
    assert_eq!(field(&windows, "short_id"), ["abcd1", "abcd3"]);
    let stopped = run(&["window", "stop", &a[3..], "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(
        stopped[0]["results"][0]["stopped"]["generation"],
        a.as_str()
    );
    let stopped = run(&["window", "stop", "abcd3", "--json"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(stopped[0]["window"], "abcd3000", "{stopped}");
    assert_eq!(
        stopped[0]["results"],
        serde_json::json!([{"stopped": {"generation": c, "terminals": ["terminal-1"]}}])
    );
    assert_eq!(listed(), Vec::<serde_json::Value>::new());

    // A damaged default-namespace entry is listed by its raw name, and an ID
    // or prefix that reaches it stops nothing, never another window.
    for name in ["abcd", "abce0"] {
        fs::create_dir(runtime.join("generations").join(name)).unwrap();
    }
    let (d, e) = (format!("g1-{:032x}", 4), format!("g1-{:032x}", 5));
    servers.push(previous_supervisor(&runtime.join("w").join("abcd1234"), &d));
    servers.push(previous_supervisor(&runtime.join("w").join("abce0123"), &e));
    let windows = listed();
    assert_eq!(
        field(&windows, "id"),
        ["abcd", "abce0", "abcd1234", "abce0123"]
    );
    assert_eq!(
        field(&windows, "short_id"),
        ["abcd", "abce0", "abcd1", "abce01"]
    );
    assert_eq!(
        field(&windows, "state"),
        ["corrupt", "corrupt", "live", "live"]
    );
    for argument in ["abcd", "abce0"] {
        let refused = run(&["window", "stop", argument, "--json"]);
        assert_eq!(refused.status.code(), Some(3), "{refused:?}");
        let failure: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "unknown-window", "{failure}");
        assert!(
            failure["error"]["detail"]
                .as_str()
                .unwrap()
                .contains("damaged"),
            "{failure}"
        );
    }
    let refused = run(&["window", "stop", "abcd"]);
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    assert!(refused.stdout.is_empty(), "{refused:?}");
    assert_eq!(run(&["window", "attach", "abcd"]).status.code(), Some(3));
    let ambiguous = run(&["window", "stop", "abce", "--json"]);
    assert_eq!(ambiguous.status.code(), Some(3), "{ambiguous:?}");
    let failure: serde_json::Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert_eq!(failure["error"]["code"], "ambiguous-window-id", "{failure}");
    let detail = failure["error"]["detail"].as_str().unwrap();
    assert!(
        detail.contains("abce0 (") && detail.contains("abce01 ("),
        "{detail}"
    );
    // An exact ID that a damaged entry shares selects nothing either, even
    // when it also names an independent window or the default one.
    for name in ["abcd1234", "default"] {
        let damaged = runtime.join("generations").join(name);
        fs::create_dir(&damaged).unwrap();
        let refused = run(&["window", "stop", name, "--json"]);
        assert_eq!(refused.status.code(), Some(3), "{name}: {refused:?}");
        let failure: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
        assert_eq!(failure["error"]["code"], "unknown-window", "{failure}");
        assert!(
            failure["error"]["detail"]
                .as_str()
                .unwrap()
                .contains("damaged"),
            "{failure}"
        );
        assert_eq!(run(&["window", "attach", name]).status.code(), Some(3));
        fs::remove_dir(&damaged).unwrap();
    }
    // Both windows still answer: no Stop reached either supervisor.
    assert_eq!(
        field(&listed(), "state"),
        ["corrupt", "corrupt", "live", "live"]
    );
    for (id, generation) in [("abcd1234", &d), ("abce0123", &e)] {
        let stopped = run(&["window", "stop", id, "--json"]);
        assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
        let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
        assert_eq!(
            stopped[0]["results"][0]["stopped"]["generation"],
            generation.as_str(),
            "{stopped}"
        );
    }
    for name in ["abcd", "abce0"] {
        fs::remove_dir(runtime.join("generations").join(name)).unwrap();
    }
    assert_eq!(listed(), Vec::<serde_json::Value>::new());
    for server in servers {
        server.join().unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

/// Catches `window stop previous|all` missing a window in either namespace,
/// naming a window by its generation, or sending a Stop to a damaged entry
/// (or to anything at all when nothing confirms).
#[test]
fn window_stop_previous_and_all_reach_every_window_and_never_a_damaged_entry() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let a = format!("g1-abcd1{:027x}", 1);
    let c = format!("g1-{:032x}", 3);
    let mut servers = vec![previous_supervisor(&runtime, &a)];
    let windows = runtime.join("w");
    fs::create_dir(&windows).unwrap();
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    servers.push(previous_supervisor(&windows.join("abcd3000"), &c));
    let damaged = "bad\n\u{1b}[31m";
    // An endpoint inside the damaged entry counts any Stop that reaches it.
    let reached = untouchable_endpoint(&runtime, damaged);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let run = |args: &[&str]| {
        eon_command(&binary)
            .args(args)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let states = || {
        let output = run(&["window", "ls", "--json"]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|window| window["state"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    // Without a yes nothing stops; each window is asked by the ID
    // `eon window ls` shows, and the damaged entry is refused, escaped.
    for which in ["previous", "all"] {
        let declined = run(&["window", "stop", which]);
        assert_eq!(declined.status.code(), Some(1), "{declined:?}");
        assert!(declined.stdout.is_empty(), "{declined:?}");
        let prompts = String::from_utf8_lossy(&declined.stderr);
        for id in ["abcd1", "abcd3"] {
            assert!(
                prompts.contains(&format!(
                    "Stop window {id} and 1 live terminal [terminal-1]? [y/N] "
                )),
                "{prompts}"
            );
            assert!(
                prompts.contains(&format!(
                    "error not-confirmed: no Stop sent for window {id};"
                )),
                "{prompts}"
            );
        }
        assert!(
            prompts.contains(r"error stop-unavailable: bad\n\u{1b}[31m is a damaged"),
            "{prompts}"
        );
        assert!(!prompts.contains('\u{1b}'), "{prompts:?}");
        assert!(!prompts.contains("g1-"), "{prompts}");
        assert_eq!(states(), ["corrupt", "live", "live"]);
    }

    // `previous` reaches another build's window in the default namespace and
    // in an independent one, each by its full ID, in listing order (the
    // default namespace by directory name).
    let stopped = run(&["window", "stop", "previous", "--json"]);
    assert_eq!(stopped.status.code(), Some(1), "{stopped:?}");
    let stopped: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    let stopped = stopped.as_array().unwrap();
    let ids: Vec<&str> = stopped
        .iter()
        .map(|window| window["window"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [damaged, &a[3..], "abcd3000"]);
    assert_eq!(
        stopped[1]["results"][0]["stopped"]["generation"],
        a.as_str()
    );
    assert_eq!(
        stopped[2]["results"][0]["stopped"]["generation"],
        c.as_str()
    );
    let refused = &stopped[0]["results"];
    assert_eq!(refused.as_array().unwrap().len(), 1, "{refused}");
    assert_eq!(refused[0]["error"]["code"], "stop-unavailable", "{refused}");
    for server in servers {
        server.join().unwrap();
    }

    // `all` refuses the damaged entry alone and leaves it in place.
    let all = run(&["window", "stop", "all", "--json"]);
    assert_eq!(all.status.code(), Some(1), "{all:?}");
    let all: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1, "{all}");
    assert_eq!(all[0]["window"], damaged);
    assert_eq!(all[0]["results"][0]["error"]["code"], "stop-unavailable");
    assert!(runtime.join("generations").join(damaged).is_dir());
    let quit = run(&["quit", "--all", "--json"]);
    assert_eq!(quit.status.code(), Some(1), "{quit:?}");
    let quit: serde_json::Value = serde_json::from_slice(&quit.stdout).unwrap();
    assert_eq!(quit[0]["window"], damaged, "{quit}");
    assert_eq!(quit[0]["results"][0]["error"]["code"], "stop-unavailable");
    assert_eq!(reached.load(Ordering::SeqCst), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_damaged_entry_is_named_in_full_past_the_eonw_detail_limit() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let generations = runtime.join("generations");
    fs::create_dir_all(&generations).unwrap();
    for directory in [&runtime, &generations] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    // 255 bytes, each control byte escaping to 6 characters: past 1,024.
    let damaged = format!("{}end", "\u{1}".repeat(252));
    assert_eq!(damaged.len(), 255);
    fs::create_dir(generations.join(&damaged)).unwrap();
    let escaped = format!("{}end", r"\u{1}".repeat(252));
    let detail = format!("{escaped} is a damaged Eon generation directory, not a window");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    for which in ["previous", "all"] {
        for json in [false, true] {
            let mut command = eon_command(&binary);
            command
                .args(["window", "stop", which])
                .args(json.then_some("--json"))
                .env("EON_RUNTIME_DIR", &runtime)
                .env("EON_CONFIG_HOME", &config)
                .env_remove("EON_WINDOW_RUNTIME_DIR")
                .env_remove("EON_TERMINAL")
                .stdin(Stdio::null());
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            if json {
                let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(results[0]["window"], damaged.as_str());
                assert_eq!(results[0]["results"][0]["error"]["detail"], detail.as_str());
            } else {
                assert_eq!(
                    String::from_utf8_lossy(&output.stderr),
                    format!("error stop-unavailable: {detail}\n")
                );
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bulk_stops_report_each_uninspectable_namespace_and_never_reach_it() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    // The default namespace lists generations from a directory others can
    // read; an endpoint inside it counts any connection.
    let mut reached = vec![untouchable_endpoint(&runtime, &format!("g1-{:032x}", 7))];
    fs::set_permissions(
        runtime.join("generations"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let windows = runtime.join("w");
    fs::create_dir(&windows).unwrap();
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    // A window entry that links outside the runtime root, to a live endpoint.
    let outside = root.join("outside");
    reached.push(untouchable_endpoint(&outside, &format!("g1-{:032x}", 8)));
    symlink(&outside, windows.join("abcd4000")).unwrap();
    // A window whose generations directory others can read.
    reached.push(untouchable_endpoint(
        &windows.join("abcd5000"),
        &format!("g1-{:032x}", 9),
    ));
    fs::set_permissions(
        windows.join("abcd5000").join("generations"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let run = |args: &[&str]| {
        eon_command(&binary)
            .args(args)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    // Each command still stops the live window it can inspect.
    let rounds: [&[&str]; 3] = [
        &["window", "stop", "previous", "--json"],
        &["window", "stop", "all", "--json"],
        &["quit", "--all", "--json"],
    ];
    for (round, args) in rounds.into_iter().enumerate() {
        let live = format!("abcd30{round:02}");
        let generation = format!("g1-{:032x}", 30 + round);
        let server = previous_supervisor(&windows.join(&live), &generation);
        let output = run(args);
        for reached in &reached {
            assert_eq!(
                reached.load(Ordering::SeqCst),
                0,
                "{args:?} reached a refused endpoint"
            );
        }
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let ids: Vec<&str> = results
            .as_array()
            .unwrap()
            .iter()
            .map(|window| window["window"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["default", &live, "abcd4000", "abcd5000"], "{args:?}");
        for (index, id) in [(0, "default"), (2, "abcd4000"), (3, "abcd5000")] {
            let refused = results[index]["results"].as_array().unwrap();
            assert_eq!(refused.len(), 1, "{results}");
            assert_eq!(refused[0]["error"]["code"], "stop-failed", "{results}");
            assert!(
                refused[0]["error"]["detail"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("cannot inspect Eon window {id}: ")),
                "{results}"
            );
        }
        assert_eq!(
            results[1]["results"][0]["stopped"]["generation"],
            generation.as_str(),
            "{results}"
        );
        server.join().unwrap();
    }

    // People read the same failures, and the live window is still asked.
    let live = "abcd3009";
    // `abcd3` alone: the uninspectable windows share `abcd`.
    let _server = previous_supervisor(&windows.join(live), &format!("g1-{:032x}", 39));
    let declined = run(&["window", "stop", "all"]);
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    let prompts = String::from_utf8_lossy(&declined.stderr);
    for id in ["default", "abcd4000", "abcd5000"] {
        assert!(
            prompts.contains(&format!(
                "error stop-failed: cannot inspect Eon window {id}: "
            )),
            "{prompts}"
        );
    }
    assert!(
        prompts.contains("Stop window abcd3 and 1 live terminal [terminal-1]? [y/N] "),
        "{prompts}"
    );
    for reached in reached {
        assert_eq!(reached.load(Ordering::SeqCst), 0);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bulk_stops_end_the_callers_window_last_when_an_older_build_owns_it() {
    // A newer `eon` runs in a terminal of another build's default-namespace
    // window: that window, not this build's `default`, is the caller's.
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let socket = generation_runtime(&runtime).join("eon.sock");
    let current = generation_runtime(&runtime);
    let Response::Snapshot(snapshot) = workspace_action(&socket, "snapshot", Action::Inspect)
    else {
        panic!("no snapshot");
    };
    let windows = runtime.join("w");
    fs::create_dir(&windows).unwrap();
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    // A damaged sibling is reported and does not hide the caller's window.
    fs::create_dir(runtime.join("generations").join("bad")).unwrap();
    let rounds: [&[&str]; 3] = [
        &["window", "stop", "previous", "--json"],
        &["window", "stop", "all", "--json"],
        &["quit", "--all", "--json"],
    ];
    for (round, args) in rounds.into_iter().enumerate() {
        let stops = Arc::new(Mutex::new(Vec::new()));
        // Listed first: the caller's own window, then another one.
        let own = format!("g1-0000a{round}{:026x}", 1);
        let other = format!("g1-0000b{round}{:026x}", 2);
        let independent = format!("g1-{:032x}", 3 + round);
        let live = format!("abcd30{round:02}");
        let servers = [
            owning_supervisor(&runtime, &own, Some(("old-caller", &snapshot)), &stops),
            owning_supervisor(&runtime, &other, None, &stops),
            owning_supervisor(&windows.join(&live), &independent, None, &stops),
        ];
        // A terminal that no supervisor validates is refused: no guess.
        let unknown = quit_command(&root)
            .args(args)
            .env("EON_TERMINAL", "unknown-caller")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(unknown.status.code(), Some(3), "{args:?}: {unknown:?}");
        let refused: serde_json::Value = serde_json::from_slice(&unknown.stdout).unwrap();
        assert_eq!(refused["error"]["code"], "unknown-caller", "{refused}");
        assert!(stops.lock().unwrap().is_empty());
        let output = quit_command(&root)
            .args(args)
            .env("EON_TERMINAL", "old-caller")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let ids: Vec<&str> = results
            .as_array()
            .unwrap()
            .iter()
            .map(|window| window["window"].as_str().unwrap())
            .collect();
        // This build's `default` runs until `all` stops it, before the
        // caller's window; the damaged entry is refused in listing order.
        let mut expected = vec!["bad", &other[3..], live.as_str(), &own[3..]];
        if round == 1 {
            expected.insert(0, "default");
        }
        assert_eq!(ids, expected, "{args:?}");
        let bad = &results[usize::from(round == 1)]["results"][0]["error"]["code"];
        assert_eq!(bad, "stop-unavailable", "{results}");
        assert_eq!(
            *stops.lock().unwrap(),
            [other.clone(), independent, own.clone()],
            "{args:?}"
        );
        for server in servers {
            server.join().unwrap();
        }
        if round == 1 {
            wait_for_successful_exit(&mut default.child);
            wait_for_generation_removal(&current);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

/// A real workspace snapshot, for fakes that validate a caller.
fn inspected_snapshot() -> Snapshot {
    let (root, mut supervisor) = quit_workspace();
    let generation = generation_runtime(&root.join("r"));
    let Response::Snapshot(snapshot) =
        workspace_action(&generation.join("eon.sock"), "snapshot", Action::Inspect)
    else {
        panic!("no snapshot");
    };
    let output = quit_command(&root)
        .args(["quit", "--all", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    wait_for_successful_exit(&mut supervisor.child);
    wait_for_generation_removal(&generation);
    fs::remove_dir_all(root).unwrap();
    snapshot
}

#[test]
fn an_id_that_two_builds_windows_share_selects_neither() {
    // An older build's default-namespace window whose generation starts
    // 1a2b3c4d, as its notice prints, and an independent window 1a2b3c4d,
    // seen by a newer `eon` whose own generation differs.
    let root = temporary_directory();
    let runtime = root.join("r");
    let older = format!("g1-1a2b3c4d{:024x}", 1);
    let independent = format!("g1-{:032x}", 2);
    let stops = Arc::new(Mutex::new(Vec::new()));
    let windows = runtime.join("w");
    let servers = [
        owning_supervisor(&runtime, &older, None, &stops),
        owning_supervisor(&windows.join("1a2b3c4d"), &independent, None, &stops),
    ];
    fs::set_permissions(&windows, fs::Permissions::from_mode(0o700)).unwrap();
    let run = |args: &[&str]| {
        eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .args(args)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", root.join("config"))
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    for args in [
        &["window", "stop", "1a2b3c4d", "--json"][..],
        &["window", "stop", "1a2b3c4d"],
        &["window", "stop", "1a2b"],
        &["window", "attach", "1a2b3c4d"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains("ambiguous-window-id"), "{args:?}: {text}");
        assert!(text.contains("1a2b3c4d ("), "{args:?}: {text}");
        assert!(
            text.contains(&format!("{} (", &older[3..12])),
            "{args:?}: {text}"
        );
        assert!(stops.lock().unwrap().is_empty(), "{args:?}");
    }
    // Each still answers to an ID only it starts.
    let output = run(&["window", "stop", &older[3..], "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let output = run(&["window", "stop", "1a2b3c4d", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(*stops.lock().unwrap(), [older, independent]);
    for server in servers {
        server.join().unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn people_read_window_ids_as_the_listing_shows_them() {
    let snapshot = inspected_snapshot();
    let root = temporary_directory();
    let runtime = root.join("r");
    let windows = runtime.join("w");
    let stops = Arc::new(Mutex::new(Vec::new()));
    let first = format!("g1-{:032x}", 1);
    let servers = [
        owning_supervisor(
            &windows.join("abcd1000"),
            &first,
            Some(("c1", &snapshot)),
            &stops,
        ),
        owning_supervisor(
            &windows.join("abcd2000"),
            &format!("g1-{:032x}", 2),
            None,
            &stops,
        ),
    ];
    for directory in [&runtime, &windows] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let command = || {
        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        command
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", root.join("config"))
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .stdin(Stdio::null());
        command
    };
    let listed = command().args(["window", "ls"]).output().unwrap();
    assert!(stdout(&listed).contains("\nabcd1 "), "{listed:?}");
    assert!(stdout(&listed).contains("\nabcd2 "), "{listed:?}");
    // A whole typed ID is shown as the listing shows it.
    let declined = command()
        .args(["window", "stop", "abcd1000"])
        .output()
        .unwrap();
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    assert!(
        String::from_utf8_lossy(&declined.stderr)
            .starts_with("Stop window abcd1 and 1 live terminal [terminal-1]? [y/N] "),
        "{declined:?}"
    );
    // So is the window `eon quit` ends from one of its terminals.
    let quit = quit_with_answers(
        command()
            .arg("quit")
            .env("EON_WINDOW_RUNTIME_DIR", windows.join("abcd1000"))
            .env("EON_TERMINAL", "c1"),
        b"y\n",
    );
    assert_eq!(quit.status.code(), Some(0), "{quit:?}");
    assert_eq!(stdout(&quit), "stopped window abcd1: terminal-1\n");
    assert_eq!(*stops.lock().unwrap(), [first]);
    drop(servers);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_default_window_also_answers_to_its_generation_hex() {
    let (root, mut default) = quit_workspace();
    let generation = generation_runtime(&root.join("r"));
    let id = generation.file_name().unwrap().to_str().unwrap().to_owned();
    // The ID notices and logs print, which `default` names only in this build.
    let output = quit_command(&root)
        .args(["window", "stop", &id[3..11], "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(results[0]["window"], "default", "{results}");
    assert_eq!(
        results[0]["results"][0]["stopped"]["generation"],
        id.as_str()
    );
    wait_for_successful_exit(&mut default.child);
    wait_for_generation_removal(&generation);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_window_that_ends_before_its_stop_is_named_as_a_window() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let windows = runtime.join("w");
    fs::create_dir_all(&windows).unwrap();
    for directory in [&runtime, &windows] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let id = format!("g1-{:032x}", 5);
    let listener = generation_listener(&windows.join("abcd3000"), &id);
    let generation = windows.join("abcd3000").join("generations").join(&id);
    // It answers the listing's inspection and is gone before the Stop.
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        fs::remove_dir_all(&generation).unwrap();
        stream
            .write_all(
                &encode_lifecycle_response(&LifecycleResponse::Runtime(previous_runtime(&id)))
                    .unwrap(),
            )
            .unwrap();
    });
    let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
        .args(["window", "stop", "all", "--json"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", root.join("config"))
        .env_remove("EON_WINDOW_RUNTIME_DIR")
        .env_remove("EON_TERMINAL")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(results[0]["window"], "abcd3000", "{results}");
    assert_eq!(
        results[0]["results"][0]["error"],
        serde_json::json!({
            "code": "unknown-generation",
            "detail": "window abcd3000 was not found"
        })
    );
    fs::remove_dir_all(root).unwrap();
}

/// As `previous_supervisor`, and it validates exactly the terminal of
/// `owner`, if any (answering its Inspect with the snapshot), and appends its
/// ID to `stops` when it stops.
fn owning_supervisor(
    runtime: &Path,
    id: &str,
    owner: Option<(&str, &Snapshot)>,
    stops: &Arc<Mutex<Vec<String>>>,
) -> thread::JoinHandle<()> {
    let listener = generation_listener(runtime, id);
    let (server_id, owner, stops) = (
        id.to_owned(),
        owner.map(|(caller, snapshot)| (caller.to_owned(), snapshot.clone())),
        Arc::clone(stops),
    );
    thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let bytes = read_request(&mut stream);
            if stop::is_reserved(&bytes) {
                let generation = stop::decode_request(&bytes).unwrap();
                assert_eq!(generation, server_id, "a Stop of another generation");
                stops.lock().unwrap().push(generation.clone());
                let stopped = stop::encode_reply(&stop::Reply::Stopped {
                    generation,
                    terminals: vec!["terminal-1".into()],
                });
                stream.write_all(&stopped.unwrap()).unwrap();
                return;
            }
            let request = decode_request(&bytes).unwrap();
            let response = match request.action {
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime) => {
                    encode_lifecycle_response(&LifecycleResponse::Runtime(previous_runtime(
                        &server_id,
                    )))
                }
                ProtocolAction::Workspace(Action::Inspect)
                    if let Some((caller, snapshot)) = &owner
                        && request.caller.as_ref() == Some(caller) =>
                {
                    encode_response(&Response::Snapshot(snapshot.clone()))
                }
                ProtocolAction::Workspace(Action::Inspect) => {
                    encode_response(&Response::Failure(ProtocolFailure {
                        code: "unknown-caller".into(),
                        detail: "not this supervisor's terminal".into(),
                    }))
                }
                action => panic!("unexpected request {action:?}"),
            };
            stream.write_all(&response.unwrap()).unwrap();
        }
    })
}

/// A socket at `socket`, under private directories, that counts the
/// connections it accepts and answers none: an endpoint no command may reach.
fn untouchable_endpoint(runtime: &Path, id: &str) -> Arc<AtomicU64> {
    let listener = generation_listener(runtime, id);
    let reached = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&reached);
    thread::spawn(move || {
        for stream in listener.incoming() {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    reached
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

/// What a fake supervisor of generation `id` from another build reports.
fn previous_runtime(id: &str) -> Runtime {
    Runtime {
        generation: id.into(),
        name: None,
        started_at: 0,
        eon_version: "0.1.0".into(),
        workspace_protocol: VERSION,
        component_report: "previous components".into(),
        sessions: vec!["terminal-1".into()],
        attach: Availability {
            available: true,
            reason: "previous presentation".into(),
        },
        stop: Availability {
            available: true,
            reason: "supervisor owns these Sessions".into(),
        },
        busy: None,
    }
}

/// Serves generation `id` of another build with the current EONW: it answers
/// inspection until it receives the Reserved Stop for itself.
fn previous_supervisor(runtime: &Path, id: &str) -> thread::JoinHandle<()> {
    let listener = generation_listener(runtime, id);
    let server_id = id.to_string();
    thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let bytes = read_request(&mut stream);
            if stop::is_reserved(&bytes) {
                let generation = stop::decode_request(&bytes).unwrap();
                assert_eq!(generation, server_id, "a Stop of another generation");
                let stopped = stop::encode_reply(&stop::Reply::Stopped {
                    generation,
                    terminals: vec!["terminal-1".into()],
                });
                stream.write_all(&stopped.unwrap()).unwrap();
                return;
            }
            let request = decode_request(&bytes).unwrap();
            let response = match request.action {
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime) => {
                    LifecycleResponse::Runtime(previous_runtime(&server_id))
                }
                action => panic!("unexpected request {action:?}"),
            };
            stream
                .write_all(&encode_lifecycle_response(&response).unwrap())
                .unwrap();
        }
    })
}

#[test]
fn generations_that_predate_the_reserved_stop_say_how_to_quit_them() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let id = format!("g1-{:032x}", 11);
    let listener = generation_listener(&runtime, &id);
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(runtime.join(format!("supervisor-{id}.lock")))
        .unwrap();
    held.lock().unwrap();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probes = std::sync::Arc::new(AtomicU64::new(0));
    let server_probes = probes.clone();
    let server_done = done.clone();
    listener.set_nonblocking(true).unwrap();
    // An older build's supervisor rejects each request in its own framing,
    // the Reserved Stop included.
    let server = thread::spawn(move || {
        while !server_done.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(peer) => peer,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            // macOS hands out accepted streams nonblocking like their listener.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_request(&mut stream);
            assert!(
                stop::is_reserved(&request)
                    || matches!(
                        decode_request(&request).unwrap().action,
                        ProtocolAction::Workspace(
                            Action::InspectPresentation | Action::InspectRuntime
                        )
                    )
            );
            let mut failure = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: format!("unsupported EONW version {VERSION}"),
            }))
            .unwrap();
            failure[4..6].copy_from_slice(&11u16.to_le_bytes());
            stream.write_all(&failure).unwrap();
            server_probes.fetch_add(1, Ordering::SeqCst);
        }
    });

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let listed = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
    assert!(listed.status.success(), "{}", stdout(&listed));
    let listed: serde_json::Value = serde_json::from_str(stdout(&listed)).unwrap();
    let older = listed["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["generation"] == id.as_str())
        .unwrap();
    assert_eq!(older["state"], "incompatible");
    assert_eq!(older["workspace_protocol"], 11, "{older}");
    // Its version predates the Reserved Stop, so the listing says how to quit
    // it, and its Stop sends nothing.
    let predates = "supervisor uses EONW 11, which predates the reserved Stop; \
                    quit it from inside its window: Alt+Shift+Q, or `eon quit` in one of its terminals";
    assert_eq!(
        older["stop"],
        serde_json::json!({"available": false, "reason": predates}),
        "{older}"
    );

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", &id[3..], "--json"],
    );
    assert_eq!(stopped.status.code(), Some(1), "{}", stdout(&stopped));
    let stopped: serde_json::Value = serde_json::from_str(stdout(&stopped)).unwrap();
    assert_eq!(stopped[0]["window"], &id[3..], "{stopped}");
    let error = &stopped[0]["results"][0]["error"];
    assert_eq!(error["code"], "stop-unavailable", "{stopped}");
    assert_eq!(
        error["detail"],
        format!("window {}: {predates}", &id[3..]),
        "{stopped}"
    );
    configure_mock_desktop(&config);
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    let mut current = TestProcess {
        child: eon_command(&binary)
            .arg("run")
            .current_dir(&root)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop,
    };
    // The new generation is the only other directory besides the mock.
    let startup_deadline = Instant::now() + Duration::from_secs(5);
    let control = loop {
        assert!(
            Instant::now() < startup_deadline,
            "current generation did not start"
        );
        let path = fs::read_dir(runtime.join("generations"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("eon.sock"))
            .find(|path| path.parent().unwrap().file_name().unwrap() != id.as_str());
        if let Some(path) = path {
            break path;
        }
        assert!(current.child.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(10));
    };
    wait_for_ready(&control);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = invoke(&binary, &runtime, &config, &["session", "--json"]);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(peer) = value["running"]["others"]
            .as_array()
            .and_then(|peers| peers.first())
        {
            assert_eq!(peer["state"], "incompatible");
            assert_eq!(peer["generation"], id);
            for field in ["name", "started_at", "live_terminals"] {
                assert!(peer[field].is_null());
            }
            break;
        }
        assert!(
            Instant::now() < deadline,
            "missing incompatible peer: {value}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(1200));
    let idle_probes = probes.load(Ordering::SeqCst);
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        probes.load(Ordering::SeqCst),
        idle_probes,
        "idle discovery probed a peer"
    );
    done.store(true, Ordering::SeqCst);
    server.join().unwrap();
    held.unlock().unwrap();
    drop(current);
    fs::remove_dir_all(root).unwrap();
}

/// Serves an older build's supervisor at `runtime`'s generation `id`: it
/// rejects each request in its own framing until `done` exists.
fn older_supervisor(runtime: &Path, id: &str, done: &Path) -> thread::JoinHandle<()> {
    other_eonw_supervisor(runtime, id, done, 11)
}

/// Serves a supervisor of EONW `version` at `runtime`'s generation `id`, as
/// `older_supervisor` does, except that from this EONW on it answers the
/// Reserved Stop for itself and ends.
fn other_eonw_supervisor(
    runtime: &Path,
    id: &str,
    done: &Path,
    version: u16,
) -> thread::JoinHandle<()> {
    let listener = generation_listener(runtime, id);
    let (done, server_id) = (done.to_owned(), id.to_owned());
    thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            if done.exists() {
                return;
            }
            let request = read_request(&mut stream);
            // This EONW is the first with the Reserved Stop.
            if version >= VERSION && stop::is_reserved(&request) {
                let generation = stop::decode_request(&request).unwrap();
                assert_eq!(generation, server_id, "a Stop of another generation");
                let stopped = stop::encode_reply(&stop::Reply::Stopped {
                    generation,
                    terminals: vec!["terminal-1".into()],
                });
                stream.write_all(&stopped.unwrap()).unwrap();
                return;
            }
            let mut failure = encode_response(&Response::Failure(ProtocolFailure {
                code: "unsupported-version".into(),
                detail: format!("unsupported EONW version {VERSION}"),
            }))
            .unwrap();
            failure[4..6].copy_from_slice(&version.to_le_bytes());
            stream.write_all(&failure).unwrap();
        }
    })
}

fn end_older_supervisors(done: &Path, sockets: &[PathBuf], servers: Vec<thread::JoinHandle<()>>) {
    fs::write(done, "").unwrap();
    for socket in sockets {
        let _ = UnixStream::connect(socket);
    }
    for server in servers {
        server.join().unwrap();
    }
}

/// Catches a window of another EONW stopped any way but the Reserved Stop:
/// a newer build's windows stop through it in their own namespaces without
/// running that build's `eon`, which only attach runs, and a window whose
/// supervisor predates it is told how to quit (EON-C11, Reserved Stop).
#[test]
fn other_eonw_windows_stop_with_the_reserved_stop() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let window = runtime.join("w").join("0000beef");
    let done = root.join("done");
    let newer = format!("g1-{:032x}", 31);
    let windowed = format!("g1-{:032x}", 32);
    let older = format!("g1-{:032x}", 33);
    let hex = &newer[3..];
    let mut servers = vec![
        other_eonw_supervisor(&runtime, &newer, &done, VERSION + 1),
        older_supervisor(&runtime, &older, &done),
    ];
    fs::create_dir(runtime.join("w")).unwrap();
    fs::set_permissions(runtime.join("w"), fs::Permissions::from_mode(0o700)).unwrap();
    servers.push(other_eonw_supervisor(
        &window,
        &windowed,
        &done,
        VERSION + 1,
    ));
    let sockets = [
        runtime.join("generations").join(&newer).join("eon.sock"),
        runtime.join("generations").join(&older).join("eon.sock"),
        window.join("generations").join(&windowed).join("eon.sock"),
    ];
    let log = root.join("log");
    let newer_eon = root.join("newer-eon");
    executable(
        &newer_eon,
        format!(
            "#!/bin/sh\n\
             printf '%s|%s|%s\\n' \"$*\" \"${{EON_WINDOW_RUNTIME_DIR-unset}}\" \
             \"${{EON_ATTACH_DELEGATED-unset}}\" >> '{log}'\n\
             exit 7\n",
            log = log.display(),
        ),
    );
    for (namespace, id) in [(&runtime, &newer), (&window, &windowed)] {
        symlink(
            &newer_eon,
            namespace.join("generations").join(id).join("eon"),
        )
        .unwrap();
    }
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let eon = |arguments: &[&str], input: &[u8]| {
        let mut child = eon_command(&binary)
            .args(arguments)
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_TERMINAL")
            .env_remove("EON_ATTACH_DELEGATED")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    };
    let take_log = || {
        let lines = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        lines
    };

    // The listing names the EONW each supervisor reported, and offers a Stop
    // for every one.
    let listed = eon(&["window", "ls", "--json"], b"");
    assert!(listed.status.success(), "{listed:?}");
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let windows = listed["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 3, "{listed}");
    for window in windows {
        let version = if window["generation"] == older.as_str() {
            11
        } else {
            VERSION + 1
        };
        assert_eq!(window["state"], "incompatible", "{window}");
        assert_eq!(window["workspace_protocol"], version, "{window}");
        // EONW 11 predates the Reserved Stop; a newer EONW answers it.
        assert_eq!(window["stop"]["available"], version != 11, "{window}");
    }

    // Attach runs the recorded `eon` as `window attach ID`.
    let attached = eon(&["window", "attach", "0000beef"], b"");
    assert_eq!(attached.status.code(), Some(7), "{attached:?}");
    assert_eq!(
        take_log(),
        format!("window attach 0000beef|{}|{windowed}\n", window.display())
    );

    // A human Stop asks first, naming the window as `eon window ls` shows
    // it; a refusal sends nothing.
    let refused = eon(&["window", "stop", "0000beef"], b"n\n");
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        error.contains(
            "Stop window 0000b? It runs another Eon build, so its terminals are unknown. [y/N] "
        ),
        "{error}"
    );
    assert!(error.contains("error not-confirmed:"), "{error}");
    let human = eon(&["window", "stop", "0000beef"], b"y\n");
    assert_eq!(human.status.code(), Some(0), "{human:?}");
    assert_eq!(stdout(&human), "stopped window 0000b: terminal-1\n");

    // Quit stops the newer window with the Reserved Stop, and names how to
    // quit the one that predates it.
    let quit = eon(&["quit", "--all", "--json"], b"");
    assert_eq!(quit.status.code(), Some(1), "{quit:?}");
    let quit: serde_json::Value = serde_json::from_slice(&quit.stdout).unwrap();
    let result = |id: &str| {
        quit.as_array()
            .unwrap()
            .iter()
            .find(|window| window["window"] == id)
            .unwrap_or_else(|| panic!("no result for {id}: {quit}"))["results"][0]
            .clone()
    };
    assert_eq!(
        result(hex),
        serde_json::json!({"stopped": {"generation": newer, "terminals": ["terminal-1"]}})
    );
    assert_eq!(
        result(&older[3..]),
        serde_json::json!({"error": {
            "code": "stop-unavailable",
            "detail": format!(
                "window {}: supervisor uses EONW 11, which predates the reserved Stop; quit it from inside its window: Alt+Shift+Q, or `eon quit` in one of its terminals",
                &older[3..]
            ),
        }})
    );
    // No Stop ran another build's `eon`.
    assert_eq!(take_log(), "");

    end_older_supervisors(&done, &sockets, servers);
    fs::remove_dir_all(root).unwrap();
}

/// Catches the ambiguous-quit listing leaving out the Stop of a window that
/// another build runs: with or without its recorded `eon`, the Reserved Stop
/// reaches it, so each row names `eon window stop` (EON-C11).
#[test]
fn quit_listing_names_the_stop_of_every_window() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let config = root.join("config");
    let done = root.join("done");
    let recorded = format!("g1-{:032x}", 23);
    let unrecorded = format!("g1-{:032x}", 24);
    let servers = vec![
        other_eonw_supervisor(&runtime, &recorded, &done, VERSION + 1),
        other_eonw_supervisor(&runtime, &unrecorded, &done, VERSION + 1),
    ];
    let sockets =
        [&recorded, &unrecorded].map(|id| runtime.join("generations").join(id).join("eon.sock"));
    let newer_eon = root.join("newer-eon");
    executable(&newer_eon, "#!/bin/sh\nexit 1\n");
    symlink(
        &newer_eon,
        runtime.join("generations").join(&recorded).join("eon"),
    )
    .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let output = eon_command(&binary)
        .arg("quit")
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env_remove("EON_WINDOW_RUNTIME_DIR")
        .env_remove("EON_TERMINAL")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let detail = String::from_utf8_lossy(&output.stderr);
    assert!(detail.contains("error ambiguous-window:"), "{detail}");
    // The two IDs differ only in their last digit, so each shows in full.
    for id in [&recorded, &unrecorded] {
        let row = detail
            .lines()
            .find(|line| line.starts_with(&format!("window {} ", &id[3..])))
            .unwrap_or_else(|| panic!("no row for {id}: {detail}"));
        assert!(
            row.ends_with(&format!("; eon window stop {}", &id[3..])),
            "{detail}"
        );
    }
    assert!(!detail.contains("g1-"), "{detail}");
    assert!(!detail.contains("cannot stop"), "{detail}");

    end_older_supervisors(&done, &sockets, servers);
    fs::remove_dir_all(root).unwrap();
}

/// An `eon` run against a fake window of another EONW, as `caller` when set,
/// with `routed` as `EON_ROUTED` when set and `input` on stdin.
fn routed_eon(
    runtime: &Path,
    state: &Path,
    arguments: &[&str],
    caller: Option<&str>,
    routed: Option<&str>,
    input: &[u8],
) -> Output {
    let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
    command
        .args(arguments)
        .env("EON_RUNTIME_DIR", runtime)
        .env("EON_CONFIG_HOME", runtime.with_file_name("config"))
        .env("XDG_STATE_HOME", state)
        .env_remove("EON_WINDOW_RUNTIME_DIR")
        .env_remove("EON_TERMINAL")
        .env_remove("EON_ROUTED")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(caller) = caller {
        command.env("EON_TERMINAL", caller);
    }
    if let Some(routed) = routed {
        command.env("EON_ROUTED", routed);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    // A routing loop never ends on its own.
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("eon {arguments:?} still runs after 60 s");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

/// A fake recorded `eon` at `path` that appends its arguments, `EON_ROUTED`,
/// `EON_TERMINAL` and stdin to `log`, writes to stdout and stderr, and exits 7.
fn recording_eon(path: &Path, log: &Path) {
    executable(
        path,
        format!(
            "#!/bin/sh\n\
             {{ printf '%s|%s|%s|' \"$*\" \"${{EON_ROUTED-unset}}\" \"${{EON_TERMINAL-unset}}\"; \
             cat; echo; }} >> '{log}'\n\
             echo routed\n\
             echo routed-error >&2\n\
             exit 7\n",
            log = log.display(),
        ),
    );
}

/// Catches a session command that reports a window's other EONW instead of
/// running the build that started it, that changes the command on the way,
/// or that runs it when no build or a second hop would (EON-C11, Routing).
#[test]
fn session_commands_run_the_build_of_a_window_of_another_eonw() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let state = root.join("state");
    let done = root.join("done");
    let id = format!("g1-{:032x}", 21);
    let server = other_eonw_supervisor(&runtime, &id, &done, VERSION + 1);
    let log = root.join("log");
    let newer_eon = root.join("newer-eon");
    recording_eon(&newer_eon, &log);
    symlink(
        &newer_eon,
        runtime.join("generations").join(&id).join("eon"),
    )
    .unwrap();
    let eon = |arguments: &[&str], caller: Option<&str>, routed: Option<&str>, input: &[u8]| {
        routed_eon(&runtime, &state, arguments, caller, routed, input)
    };
    let take_log = || {
        let lines = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        lines
    };
    let caller = Some("terminal-3.r1");

    // Its build runs the same arguments, environment and stdin, and its output
    // and status are the command's.
    for (arguments, input) in [
        (
            &["pane", "send", "p2", "--stdin", "--json"][..],
            &b"hello"[..],
        ),
        (&["quit"][..], &b""[..]),
    ] {
        let output = eon(arguments, caller, None, input);
        assert_eq!(output.status.code(), Some(7), "{output:?}");
        assert_eq!(output.stdout, b"routed\n");
        assert_eq!(output.stderr, b"routed-error\n");
        assert_eq!(
            take_log(),
            format!(
                "{}|{id}|terminal-3.r1|{}\n",
                arguments.join(" "),
                String::from_utf8_lossy(input)
            )
        );
    }
    // So does an explicit target, from outside Eon.
    let window = &id[3..11];
    let shown = eon(&["session", "--window", window, "--json"], None, None, b"");
    assert_eq!(shown.status.code(), Some(7), "{shown:?}");
    assert_eq!(
        take_log(),
        format!("session --window {window} --json|{id}|unset|\n")
    );
    // A routed `eon` never routes: whatever the marker says, the refusal
    // names it, and nothing runs.
    let refused_as_routed = |output: &Output, marker: &str| {
        assert_eq!(output.status.code(), Some(4), "{output:?}");
        let failure: serde_json::Value = serde_json::from_str(stdout(output)).unwrap();
        assert_eq!(failure["error"]["code"], "supervisor-unavailable");
        let detail = failure["error"]["detail"].as_str().unwrap();
        assert!(detail.contains("EONW"), "{detail}");
        assert!(
            detail.contains(&format!("EON_ROUTED is \"{marker}\"")),
            "{detail}"
        );
    };
    for (arguments, marker) in [
        (&["session", "--json"][..], id.as_str()),
        (&["quit", "--json"][..], "forged"),
    ] {
        refused_as_routed(&eon(arguments, caller, Some(marker), b""), marker);
    }
    assert_eq!(take_log(), "");

    // Commands that are not session commands never route.
    let versions = eon(&["versions"], caller, None, b"");
    assert!(versions.status.success(), "{versions:?}");
    assert!(stdout(&versions).starts_with("eon "), "{versions:?}");
    let listed = eon(&["window", "ls", "--json"], caller, None, b"");
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(take_log(), "");

    // A record that leads back to an `eon` that would route again makes one
    // hop, then fails, within the bound.
    let record = runtime.join("generations").join(&id).join("eon");
    fs::remove_file(&record).unwrap();
    symlink(env!("CARGO_BIN_EXE_eon"), &record).unwrap();
    refused_as_routed(&eon(&["session", "--json"], caller, None, b""), &id);
    fs::remove_file(&record).unwrap();
    symlink(&newer_eon, &record).unwrap();

    // Without the build it recorded, the refusal stays and names it.
    fs::remove_file(&newer_eon).unwrap();
    let gone = eon(&["session", "--json"], caller, None, b"");
    assert_eq!(gone.status.code(), Some(4), "{gone:?}");
    let failure: serde_json::Value = serde_json::from_str(stdout(&gone)).unwrap();
    let detail = failure["error"]["detail"].as_str().unwrap();
    assert!(
        detail.contains(&format!("{}, no longer exists", newer_eon.display())),
        "{detail}"
    );
    assert!(detail.contains("the terminal's own `eon`"), "{detail}");

    // Stop is the Reserved Stop, never routed, even with its build present.
    recording_eon(&newer_eon, &log);
    let stopped = eon(&["window", "stop", window, "--json"], None, None, b"");
    assert!(stopped.status.success(), "{stopped:?}");
    assert_eq!(take_log(), "");
    end_older_supervisors(
        &done,
        &[runtime.join("generations").join(&id).join("eon.sock")],
        vec![server],
    );
    fs::remove_dir_all(root).unwrap();
}

/// Catches a stale `EON_TERMINAL` reported as another window's EONW, or routed
/// to a build whose window never ran it, instead of named (EON-C10).
#[test]
fn a_stale_eon_terminal_names_its_terminal_and_window() {
    let root = temporary_directory();
    let runtime = root.join("r");
    let state = root.join("state");
    let done = root.join("done");
    let id = format!("g1-{:032x}", 22);
    let ended = format!("g1-{:032x}", 23);
    let compatible = format!("g1-{:032x}", 24);
    let server = other_eonw_supervisor(&runtime, &id, &done, VERSION + 1);
    let stops = Arc::new(Mutex::new(Vec::new()));
    let compatible_server = owning_supervisor(&runtime, &compatible, None, &stops);
    let log = root.join("log");
    let newer_eon = root.join("newer-eon");
    recording_eon(&newer_eon, &log);
    symlink(
        &newer_eon,
        runtime.join("generations").join(&id).join("eon"),
    )
    .unwrap();
    let logs = state.join("eon").join("logs").join("orbit");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join(format!("{ended}-terminal-3-r1.log")), "").unwrap();
    fs::write(logs.join(format!("{id}-terminal-4-r2.log")), "").unwrap();
    fs::write(logs.join(format!("{compatible}-terminal-3-r5.log")), "").unwrap();
    let eon = |arguments: &[&str], caller: &str| {
        routed_eon(&runtime, &state, arguments, Some(caller), None, b"")
    };
    let stale = |output: &Output, place: &str| {
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let failure: serde_json::Value = serde_json::from_str(stdout(output)).unwrap();
        assert_eq!(failure["error"]["code"], "unknown-caller");
        let detail = failure["error"]["detail"].as_str().unwrap();
        assert!(
            detail.starts_with(&format!("EON_TERMINAL names terminal terminal-3{place}; ")),
            "{detail}"
        );
        assert!(
            detail.contains("Run `eon` from a live Eon terminal's own shell"),
            "{detail}"
        );
    };

    // Its log places the terminal in an ended window: no version error, no route.
    let ended_window = format!(" of Eon window {}, which has ended", &ended[3..]);
    stale(&eon(&["session", "--json"], "terminal-3.r1"), &ended_window);
    // The stops of every window say the same of the caller.
    let all = eon(&["quit", "--all", "--json"], "terminal-3.r1");
    assert_eq!(all.status.code(), Some(3), "{all:?}");
    assert!(stdout(&all).contains(&ended_window), "{}", stdout(&all));
    assert!(!log.exists());
    // Its log places it in a live window that no longer runs it.
    stale(
        &eon(&["session", "--json"], "terminal-3.r5"),
        &format!(
            " of Eon window {}, which no longer runs it",
            &compatible[3..]
        ),
    );
    // Its log places it in the window of another EONW: that build runs it.
    let routed = eon(&["session", "--json"], "terminal-4.r2");
    assert_eq!(routed.status.code(), Some(7), "{routed:?}");
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!("session --json|{id}|terminal-4.r2|\n")
    );

    end_older_supervisors(
        &done,
        &[runtime.join("generations").join(&id).join("eon.sock")],
        vec![server],
    );
    let stopped = routed_eon(
        &runtime,
        &state,
        &["window", "stop", &compatible[3..], "--json"],
        None,
        None,
        b"",
    );
    assert!(stopped.status.success(), "{stopped:?}");
    compatible_server.join().unwrap();
    assert_eq!(*stops.lock().unwrap(), [compatible]);
    // No window runs, and no log names it.
    stale(
        &eon(&["session", "--json"], "terminal-3.r9"),
        ", which no running Eon window here runs",
    );
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
    let find = |window: &str| {
        let output = invoke(&binary, &runtime, &config, &["window", "ls", "--json"]);
        assert!(output.status.success(), "{}", stdout(&output));
        let listed: serde_json::Value = serde_json::from_str(stdout(&output)).unwrap();
        listed["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == window)
            .cloned()
    };
    let listed = |window: &str| find(window).unwrap_or_else(|| panic!("{window} is not listed"));

    // A window with no supervisor is left out, never listed as the current generation.
    assert_eq!(find("0000dcba"), None);
    let older = listed("0000abcd");
    let short = older["short_id"].as_str().unwrap();
    let human = invoke(&binary, &runtime, &config, &["window", "ls"]);
    assert!(
        !stdout(&human).contains("0000dcba") && !stdout(&human).contains(&id),
        "{}",
        stdout(&human)
    );
    assert!(
        stdout(&human).lines().any(
            |line| line.starts_with(&format!("{short} ")) && line.ends_with(" no (other EONW)")
        ),
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

    // Without a record of its build, attach names the window and the recovery.
    assert_eq!(older["state"], "incompatible", "{older}");
    assert_eq!(older["generation"], id.as_str());
    assert_eq!(older["attach"]["available"], false);
    let unrecorded = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", "0000abcd"],
    );
    assert_eq!(unrecorded.status.code(), Some(1));
    let error = String::from_utf8_lossy(&unrecorded.stderr);
    assert!(
        error.starts_with(&format!("eon: Eon window {short} ")),
        "{error}"
    );
    assert!(!error.contains(&id), "{error}");
    assert!(error.contains("EONW 11"), "{error}");
    assert!(
        error.contains("the Eon build that started it can attach it"),
        "{error}"
    );

    // With the record, attach runs that build's `eon` in the window.
    let attached = root.join("attached");
    let older_eon = root.join("older-eon");
    executable(
        &older_eon,
        format!(
            "#!/bin/sh\nprintf '%s|%s|%s' \"$*\" \"$EON_WINDOW_RUNTIME_DIR\" \"$EON_ATTACH_DELEGATED\" > '{}'\nexit 7\n",
            attached.display()
        ),
    );
    // A relative record is not trusted.
    let record = window.join("generations").join(&id).join("eon");
    symlink("older-eon", &record).unwrap();
    assert_eq!(listed("0000abcd")["attach"]["available"], false);
    fs::remove_file(&record).unwrap();
    symlink(&older_eon, &record).unwrap();
    let recorded = &listed("0000abcd");
    assert_eq!(recorded["attach"]["available"], true, "{recorded}");
    assert_eq!(recorded["compatible"], false, "{recorded}");
    assert_eq!(recorded["executable"], older_eon.to_str().unwrap());
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
    // Every build is run as `window attach ID`, whatever its EONW.
    assert_eq!(
        fs::read_to_string(&attached).unwrap(),
        format!("window attach 0000abcd|{}|{id}", window.display())
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
    // `window attach` reaches another build's default-namespace window by its
    // ID, in the default namespace. Its first 4 digits also start 0000abcd's ID.
    // That build is of this EONW, so it takes `window attach default`.
    fs::remove_file(&attached).unwrap();
    let delegated = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "attach", &previous[3..]],
    );
    assert_eq!(
        delegated.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&delegated.stderr)
    );
    assert_eq!(
        fs::read_to_string(&attached).unwrap(),
        format!("window attach default||{previous}")
    );
    let again = eon_command(&binary)
        .args(["window", "attach", "default"])
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ATTACH_DELEGATED", &previous)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(2), "{again:?}");
    assert!(String::from_utf8_lossy(&again.stderr).contains("nothing was sent"));
    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", &previous[3..], "--json"],
    );
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
        error.contains(&format!("Eon 0.1.0 (its eon: {})", older_eon.display())),
        "{error}"
    );
    assert!(
        error.contains("Eon 0.1.0 (its eon is not recorded)"),
        "{error}"
    );
    assert!(
        !error.contains(&first) && !error.contains(&second),
        "{error}"
    );
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
fn stop_without_terminal_requires_yes() {
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
    let mut supervisor = TestProcess {
        child: eon_command(&binary)
            .arg("run")
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .env("EON_ORBIT", &orbit)
            .env("EON_VENUS", &venus)
            .env("EON_TEST_STOP", &stop)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        stop,
    };
    let generation = generation_runtime(&runtime);
    wait_for_ready(&generation.join("eon.sock"));

    let declined = invoke(&binary, &runtime, &config, &["window", "stop", "default"]);
    assert_eq!(declined.status.code(), Some(1), "{declined:?}");
    assert_eq!(stdout(&declined), "");
    assert!(String::from_utf8_lossy(&declined.stderr).contains(
        "error not-confirmed: no Stop sent for window default; no yes was read and stdin is not a terminal"
    ));
    assert!(supervisor.child.try_wait().unwrap().is_none());
    assert!(
        invoke(&binary, &runtime, &config, &["session", "--json"])
            .status
            .success()
    );

    let stopped = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "default", "--json"],
    );
    assert!(stopped.status.success(), "{}", stdout(&stopped));
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn previous_and_all_stop_older_generations_when_current_is_unstarted() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    let window = |which: &str| {
        let output = invoke(
            &binary,
            &runtime,
            &config,
            &["window", "stop", which, "--json"],
        );
        assert!(output.status.success(), "{}", stdout(&output));
        serde_json::from_str::<serde_json::Value>(stdout(&output)).unwrap()
    };
    assert_eq!(window("all"), serde_json::json!([]));
    let first_id = format!("g1-{:032x}", 2);
    let first = previous_supervisor(&runtime, &first_id);

    let result = window("previous");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["window"], &first_id[3..]);
    assert_eq!(result[0]["results"][0]["stopped"]["generation"], first_id);
    first.join().unwrap();

    let second_id = format!("g1-{:032x}", 3);
    let second = previous_supervisor(&runtime, &second_id);
    let result = window("all");
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["results"][0]["stopped"]["generation"], second_id);
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
    wait_for_ready(&current_runtime.join("eon.sock"));
    let current_id = current_runtime.file_name().unwrap().to_str().unwrap();
    // Each window's one Stop result, in stop order.
    let results = |output: &Output| {
        let windows: serde_json::Value = serde_json::from_str(stdout(output)).unwrap();
        windows
            .as_array()
            .unwrap()
            .iter()
            .map(|window| {
                assert_eq!(window["results"].as_array().unwrap().len(), 1, "{window}");
                window["results"][0].clone()
            })
            .collect::<Vec<_>>()
    };

    let first_id = format!("g1-{:032x}", 2);
    let second_id = format!("g1-{:032x}", 3);
    let first = previous_supervisor(&runtime, &first_id);
    let second = previous_supervisor(&runtime, &second_id);
    let unavailable = runtime.join("generations").join(format!("g1-{:032x}", 1));
    fs::create_dir(&unavailable).unwrap();
    fs::set_permissions(&unavailable, fs::Permissions::from_mode(0o755)).unwrap();

    let previous = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "previous", "--json"],
    );
    assert_eq!(previous.status.code(), Some(1), "{}", stdout(&previous));
    let stopped = results(&previous);
    assert_eq!(stopped.len(), 3);
    assert_eq!(stopped[0]["error"]["code"], "stop-unavailable");
    assert_eq!(stopped[1]["stopped"]["generation"], first_id);
    assert_eq!(stopped[2]["stopped"]["generation"], second_id);
    assert!(current.child.try_wait().unwrap().is_none());
    assert!(
        invoke(&binary, &runtime, &config, &["session", "--json"])
            .status
            .success()
    );
    first.join().unwrap();
    second.join().unwrap();
    fs::remove_dir_all(unavailable).unwrap();

    let third_id = format!("g1-{:032x}", 4);
    let third = previous_supervisor(&runtime, &third_id);
    let mut human = TestProcess {
        child: eon_command(&binary)
            .args(["window", "stop", "all"])
            .env("EON_RUNTIME_DIR", &runtime)
            .env("EON_CONFIG_HOME", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        stop: stop.clone(),
    };
    human
        .child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"y\nn\n")
        .unwrap();
    let human = human.wait_with_output().unwrap();
    assert_eq!(human.status.code(), Some(1), "{}", stdout(&human));
    // The older window is named by its short ID, at least 4 hex digits.
    let shown = stdout(&human)
        .strip_prefix("stopped window ")
        .and_then(|rest| rest.split_once(": "))
        .map(|(shown, _)| shown.to_owned())
        .unwrap_or_else(|| panic!("{}", stdout(&human)));
    assert!(
        shown.len() >= 4 && third_id[3..].starts_with(&shown),
        "{shown}"
    );
    assert!(!stdout(&human).contains("cancelled;"));
    assert!(String::from_utf8_lossy(&human.stderr).contains(
        "error not-confirmed: no Stop sent for window default; no yes was read and stdin is not a terminal"
    ));
    assert!(current.child.try_wait().unwrap().is_none());
    third.join().unwrap();

    let fourth_id = format!("g1-{:032x}", 5);
    let fourth = previous_supervisor(&runtime, &fourth_id);
    let all = invoke(
        &binary,
        &runtime,
        &config,
        &["window", "stop", "all", "--json"],
    );
    assert!(all.status.success(), "{}", stdout(&all));
    let stopped = results(&all);
    assert_eq!(stopped.len(), 2);
    assert_eq!(stopped[0]["stopped"]["generation"], fourth_id);
    assert_eq!(stopped[1]["stopped"]["generation"], current_id);
    fourth.join().unwrap();
    wait_for_successful_exit(&mut current.child);
    assert!(!current_runtime.exists());
    fs::remove_dir_all(root).unwrap();
}

/// A full-Eon supervisor over fake Orbits and its control socket. `agents` is
/// appended to the mock desktop's configuration, and stub Commands and
/// Workspaces popups are on its terminal PATH.
fn terminal_access_workspace(root: &Path, agents: &str) -> (TestProcess, PathBuf, PathBuf) {
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    fs::OpenOptions::new()
        .append(true)
        .open(config.join("config.toml"))
        .unwrap()
        .write_all(agents.as_bytes())
        .unwrap();
    let stop = root.join("stop");
    let orbit = root.join("orbit");
    let venus = root.join("venus");
    managed_orbit_executable(&orbit);
    executable(&venus, "#!/bin/sh\nexit 0\n");
    // The fake Orbits never run it.
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    for popup in ["eon-commands", "eon-sessions"] {
        executable(&bin.join(popup), "#!/bin/sh\nexit 0\n");
    }
    let mut command = eon_command(&PathBuf::from(env!("CARGO_BIN_EXE_eon")));
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
        .unwrap()
        .to_owned();
    let path = std::iter::once(bin).chain(std::env::split_paths(&path));
    let child = command
        .arg("run")
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("EON_RUNTIME_DIR", &runtime)
        .env("EON_CONFIG_HOME", &config)
        .env("EON_ORBIT", &orbit)
        .env("EON_VENUS", &venus)
        .env("EON_TEST_STOP", &stop)
        .env("EON_TEST_INPUT_LOG", root.join("input.log"))
        .env("EON_TEST_QUIET_LOG", root.join("quiet.log"))
        .env("EON_TEST_ORBIT_REPLY", root.join("orbit-reply"))
        .env("EON_TEST_INPUT_TIMES", root.join("input-times.log"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let supervisor = TestProcess { child, stop };
    let generation = generation_runtime(&runtime);
    let control = generation.join("eon.sock");
    wait_for(&control);
    (supervisor, generation, control)
}

#[test]
fn the_shown_workspace_pickers_key_presses_enter_in_it_instead_of_hiding_it() {
    let root = temporary_directory();
    let (mut supervisor, _generation, control) = terminal_access_workspace(&root, "");
    let invoke = |id: &str, entry: &str, expected: Option<&str>, intent| match protocol_action(
        &control,
        id,
        ProtocolAction::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: expected.map(Into::into),
            intent,
        },
    ) {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("{id}: {other:?}"),
    };
    let inputs = || fs::read_to_string(root.join("input.log")).unwrap_or_default();
    let opened = invoke("open", "sessions", None, InvokeIntent::Toggle);
    let picker = opened.tabs[0]
        .popups
        .iter()
        .find(|popup| popup.entry == "sessions")
        .unwrap()
        .clone();
    assert_eq!(opened.tabs[0].selected_popup.as_ref(), Some(&picker.id));
    // Focusing the shown picker presses nothing; its key keeps it shown and presses Enter in it,
    // which confirms the highlighted row (EON-C33).
    let focused = invoke("focus", "sessions", Some(&picker.id), InvokeIntent::Focus);
    assert_eq!(stable_tabs(&focused.tabs), stable_tabs(&opened.tabs));
    assert_eq!(inputs(), "");
    let confirmed = invoke(
        "confirm",
        "sessions",
        Some(&picker.id),
        InvokeIntent::Toggle,
    );
    assert_eq!(stable_tabs(&confirmed.tabs), stable_tabs(&opened.tabs));
    assert_eq!(inputs(), format!("{} Enter\n", picker.session));
    // After the tab's directory changes, the key relaunches the picker there, pressing nothing.
    let Response::Snapshot(_) = protocol_action(
        &control,
        "directory",
        ProtocolAction::Workspace(Action::SetTabDirectory {
            tab: "t1".into(),
            directory: root.as_os_str().as_bytes().to_vec(),
        }),
    ) else {
        panic!("the tab directory did not change");
    };
    let relaunched = invoke(
        "relaunch",
        "sessions",
        Some(&picker.id),
        InvokeIntent::Toggle,
    );
    let selected = relaunched.tabs[0].selected_popup.clone().unwrap();
    assert_ne!(selected, picker.id);
    assert_eq!(inputs(), format!("{} Enter\n", picker.session));
    assert_eq!(fs::read_to_string(root.join("quiet.log")).unwrap(), "0\n");
    // Another transient popup's key still hides it, pressing nothing.
    let commands = invoke("commands", "commands", None, InvokeIntent::Toggle);
    let shown = commands.tabs[0].selected_popup.clone().unwrap();
    let hidden = invoke("hide", "commands", Some(&shown), InvokeIntent::Toggle);
    assert_eq!(hidden.tabs[0].selected_popup, None);
    assert_eq!(inputs(), format!("{} Enter\n", picker.session));
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_reads_and_sends_reach_a_hidden_workspace_and_change_nothing() {
    let root = temporary_directory();
    let (mut supervisor, generation, control) = terminal_access_workspace(&root, "");
    let snapshot = |response| match response {
        Response::Snapshot(snapshot) => snapshot,
        other => panic!("expected a workspace: {other:?}"),
    };
    let failure = |response| match response {
        Response::Failure(failure) => failure.code,
        other => panic!("expected a failure: {other:?}"),
    };
    let hidden = snapshot(workspace_action(&control, "inspect", Action::Inspect));
    let tab = hidden.tabs[0].id.clone();
    let pane = hidden.tabs[0].columns[0].panes[0].clone();
    assert_eq!(pane.session, "terminal-1");
    snapshot(workspace_action(
        &control,
        "new",
        Action::NewWorkspace {
            name: None,
            focus: true,
        },
    ));
    let before = snapshot(workspace_action(&control, "before", Action::Inspect));
    assert_eq!(before.workspace, "s2");
    let target = TerminalTarget {
        workspace: "s1".into(),
        tab,
        id: TargetId::Pane(pane.id.clone()),
        terminal: pane.session.clone(),
    };
    let read = TerminalText {
        target: target.clone(),
        caller: None,
        revision: 1,
        screen: Screen::Primary,
        rows: 24,
        columns: 80,
        title: "terminal-1".into(),
        directory: b"/fake".to_vec(),
        omitted_rows: 0,
        text: "screen of terminal-1".into(),
    };
    let read_action = || ProtocolAction::ReadTerminal {
        target: pane.id.clone(),
    };
    assert_eq!(
        protocol_action(&control, "read", read_action()),
        Response::Text(read.clone())
    );
    // A calling terminal is projected as `eon session` names it.
    let environment = process::environ(live_identity(&generation.join("orbit.sock")).process_id);
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .map(|token| String::from_utf8(token.to_vec()).unwrap())
        .unwrap();
    assert_eq!(
        protocol_action_from(&control, "read-caller", Some(&caller), read_action()),
        Response::Text(TerminalText {
            caller: Some(target.clone()),
            ..read
        })
    );
    assert_eq!(
        protocol_action(
            &control,
            "history",
            ProtocolAction::ReadTerminalHistory {
                target: pane.id.clone(),
            },
        ),
        Response::History(TerminalHistory {
            target: target.clone(),
            caller: None,
            revision: 2,
            screen: Screen::Alternate,
            rows: 24,
            columns: 80,
            title: "terminal-1".into(),
            directory: b"/fake".to_vec(),
            omitted_lines: 0,
            cut_bytes: 0,
            text: "history of terminal-1\nscreen of terminal-1".into(),
        })
    );
    let send = |id: &str, input| {
        protocol_action(
            &control,
            id,
            ProtocolAction::SendTerminal {
                target: pane.id.clone(),
                force: false,
                input,
            },
        )
    };
    let sent = |bracketed| {
        Response::Sent(Sent {
            target: target.clone(),
            caller: None,
            bracketed,
            enter_delay_ms: 300,
        })
    };
    assert_eq!(
        send("paste", TerminalInput::Paste("echo hi".into())),
        sent(true)
    );
    assert_eq!(send("enter", TerminalInput::Enter), sent(false));
    assert_eq!(
        fs::read_to_string(root.join("input.log")).unwrap(),
        "terminal-1 Paste(\"echo hi\")\nterminal-1 Enter\n"
    );
    assert_eq!(
        failure(send("enter", TerminalInput::Enter)),
        "duplicate-request"
    );
    assert_eq!(
        failure(protocol_action(
            &control,
            "missing",
            ProtocolAction::ReadTerminal {
                target: "p99".into()
            },
        )),
        "unknown-target"
    );
    let after = snapshot(workspace_action(&control, "after", Action::Inspect));
    assert_eq!(
        (after.workspace.as_str(), after.active_tab.as_str()),
        (before.workspace.as_str(), before.active_tab.as_str())
    );
    assert_eq!(stable_tabs(&after.tabs), stable_tabs(&before.tabs));
    assert_eq!(after.workspaces, before.workspaces);
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

/// A window with `configuration` added whose configured editor logs the file
/// it opens and runs until the editor release file exists, with its first
/// tab committed.
fn scrollback_window(window: &PairWindow, configuration: &str) -> TestProcess {
    let editor = window.root.join("editor");
    executable(
        &editor,
        r#"#!/bin/sh
printf '%s\n' "$1" >> "$EON_TEST_EDITOR_LOG"
while [ ! -e "$EON_TEST_EDITOR_RELEASE" ] && [ ! -e "$EON_TEST_STOP" ]; do sleep 0.01; done
"#,
    );
    fs::OpenOptions::new()
        .append(true)
        .open(window.config.join("config.toml"))
        .unwrap()
        .write_all(
            format!(
                "\n[editor]\ncommand = [\"{}\"]\n{configuration}",
                editor.display()
            )
            .as_bytes(),
        )
        .unwrap();
    let supervisor = window.spawn_scrollback();
    window.pair_launch("terminal-2");
    let home = window.home.to_str().unwrap();
    assert!(
        window
            .eon(&["tab", "directory", "t1", "--", home])
            .status
            .success()
    );
    live_identity(&window.generation().join("orbit.sock"));
    supervisor
}

impl PairWindow {
    fn scrollback_command(&self) -> Command {
        let mut command = self.command(&[]);
        command
            .env("EON_TEST_ORBIT_REPLY", self.root.join("orbit-reply"))
            .env("EON_TEST_EDITOR_LOG", self.root.join("editor.log"))
            .env("EON_TEST_EDITOR_RELEASE", self.root.join("editor-release"))
            .env("EON_TEST_SNAPSHOT_LOG", self.root.join("snapshot.log"))
            .env(
                "EON_TEST_EDITOR_READY_DELAY",
                self.root.join("editor-ready-delay"),
            );
        command
    }

    fn spawn_scrollback(&self) -> TestProcess {
        let process = self.start(self.scrollback_command());
        wait_for_ready(&self.control());
        process
    }

    /// Presses the active tab's Scrollback key and returns the new popup and
    /// the snapshot its editor opened, which named the popup's terminal
    /// before its Orbit started.
    fn open_scrollback(&self, id: &str) -> (Popup, PathBuf) {
        let opened = log_lines(&self.root.join("editor.log")).len();
        let Response::Snapshot(snapshot) = self.invoke_popup(id, "scrollback", None) else {
            panic!("{id}: Scrollback did not open");
        };
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab)
            .unwrap();
        let popup = tab.popups.last().unwrap().clone();
        assert_eq!(
            (popup.entry.as_str(), tab.selected_popup.as_ref()),
            ("scrollback", Some(&popup.id))
        );
        let file =
            PathBuf::from(&wait_for_lines(&self.root.join("editor.log"), opened + 1)[opened]);
        assert!(
            log_lines(&self.root.join("snapshot.log")).contains(&format!("{0}|{0}", popup.session)),
            "{id}"
        );
        (popup, file)
    }

    fn snapshot_directories(&self) -> Vec<PathBuf> {
        fs::read_dir(self.generation().join("snapshots"))
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_default()
    }
}

fn wait_for_removal(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::symlink_metadata(path).is_ok() {
        assert!(Instant::now() < deadline, "{} remained", path.display());
        thread::sleep(Duration::from_millis(10));
    }
}

/// Scrollback writes the shown terminal's history to a private snapshot,
/// opens it in the configured editor over that terminal, and removes it when
/// the popup's terminal ends: by the editor's exit, a dismissal, or its tab's
/// close. A failed export or a missing editor starts nothing and leaves no
/// snapshot (EON-C10).
#[test]
fn scrollback_opens_a_private_snapshot_and_removes_it_when_its_popup_ends() {
    let window = PairWindow::new();
    let mut supervisor = scrollback_window(&window, "");
    let generation = window.generation();
    let snapshots = generation.join("snapshots");
    let release = window.root.join("editor-release");
    let mode = |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777;

    // The fake Orbit's history is on the alternate screen, so the snapshot
    // starts with that notice; then the text as `eon pane history` prints it.
    let (popup, file) = window.open_scrollback("open");
    let directory = file.parent().unwrap().to_owned();
    assert_eq!(directory.parent().unwrap(), snapshots);
    assert_eq!(file.file_name().unwrap(), "p1-scrollback.txt");
    assert_eq!(
        [&snapshots, &directory, &file].map(|path| mode(path)),
        [0o700, 0o700, 0o600]
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "This pane is in a full-screen app; Eon shows only its screen. Quit the app to see the history.\nhistory of terminal-1\nscreen of terminal-1\n"
    );
    let history = window.eon(&["pane", "history", "p1"]);
    assert_eq!(
        stdout(&history),
        "history of terminal-1\nscreen of terminal-1\n"
    );
    // The editor's exit ends the popup, shows the pane and removes the snapshot.
    fs::write(&release, "").unwrap();
    wait_for_removal(&directory);
    let tab = window.inspect("after-exit").tabs.remove(0);
    assert!(tab.popups.iter().all(|item| item.id != popup.id));
    assert_eq!(
        (tab.selected_popup, tab.selected_pane.as_deref()),
        (None, Some("p1"))
    );
    wait_for_removal(&snapshots);
    fs::remove_file(&release).unwrap();

    // A dismissal removes it too; the bound's notice comes first.
    fs::write(window.root.join("orbit-reply"), "cut-history").unwrap();
    let (popup, file) = window.open_scrollback("cut");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "Eon left out the earliest line and the first 7 bytes of the newest line to keep this snapshot within its 8 MiB bound. This pane is in a full-screen app; Eon shows only its screen. Quit the app to see the history.\nline\n"
    );
    fs::remove_file(window.root.join("orbit-reply")).unwrap();
    assert!(matches!(
        window.invoke_popup("dismiss", "scrollback", Some(&popup.id)),
        Response::Snapshot(snapshot) if snapshot.tabs[0].popups.iter().all(|item| item.id != popup.id)
    ));
    wait_for_removal(file.parent().unwrap());

    // Closing its tab removes it too.
    assert!(window.eon(&["tab", "create"]).status.success());
    let pair = window.inspect("tab-two").tabs[1].popups[0].session.clone();
    window.pair_launch(&pair);
    let home = window.home.to_str().unwrap();
    assert!(
        window
            .eon(&["tab", "directory", "t2", "--", home])
            .status
            .success()
    );
    let (_, file) = window.open_scrollback("second");
    assert!(file.exists());
    assert!(matches!(
        workspace_action(&window.control(), "close-t2", Action::CloseTab { tab: "t2".into() }),
        Response::Snapshot(snapshot) if snapshot.tabs.len() == 1
    ));
    wait_for_removal(file.parent().unwrap());

    // An export that fails or misses the terminal deadline, and a missing
    // editor, show the popup start-failure notice and leave no snapshot.
    let tab = window.inspect("before-failures").tabs.remove(0);
    // The timed-out read goes last: its late answer holds the lease's slot.
    for (id, reply, editor, code) in [
        ("busy", "busy", None, "popup-start"),
        (
            "missing",
            "",
            Some(window.root.join("missing")),
            "popup-command",
        ),
        ("slow", "slow-history", None, "popup-start"),
    ] {
        fs::write(window.root.join("orbit-reply"), reply).unwrap();
        if let Some(editor) = &editor {
            fs::write(
                window.config.join("config.toml"),
                format!(
                    "[terminal]\nfont_fallbacks = []\n\n[anima]\nenabled = false\n\n[editor]\ncommand = [\"{}\"]\n",
                    editor.display()
                ),
            )
            .unwrap();
        }
        assert!(
            matches!(
                window.invoke_popup(id, "scrollback", None),
                Response::Failure(failure) if failure.code == code
            ),
            "{id}"
        );
        assert_eq!(window.snapshot_directories(), Vec::<PathBuf>::new(), "{id}");
        assert_eq!(window.inspect(&format!("{id}-after")).tabs[0], tab, "{id}");
    }
    window.stop_generation(&mut supervisor);
    assert!(!snapshots.exists());
}

/// A recovering supervisor removes the snapshots of supervisors no longer
/// running but adopts its recovered Scrollback terminal's, and removes that
/// one when the terminal ends (EON-C10).
#[test]
fn recovery_adopts_a_live_scrollback_snapshot_and_sweeps_stale_ones() {
    let window = PairWindow::new();
    let mut first = scrollback_window(&window, "");
    let (popup, file) = window.open_scrollback("open");
    let stale = window
        .generation()
        .join("snapshots")
        .join(format!("{}-1-1", u32::MAX));
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("p9-scrollback.txt"), "old").unwrap();
    window.crash(&mut first);

    let mut replacement = window.spawn_scrollback();
    let recovered = window.inspect("recovered");
    assert!(
        recovered.tabs[0]
            .panes()
            .any(|pane| pane.session == popup.session)
    );
    assert!(!stale.exists());
    assert!(file.exists());
    fs::write(window.root.join("editor-release"), "").unwrap();
    wait_for_removal(file.parent().unwrap());
    window.stop_generation(&mut replacement);
}

/// A recovering supervisor whose Venus cannot start stops the terminals it
/// adopted and removes their snapshots with them (EON-C10).
#[test]
fn recovery_whose_venus_cannot_start_removes_adopted_snapshots() {
    let window = PairWindow::new();
    let mut first = scrollback_window(&window, "");
    let (_, file) = window.open_scrollback("open");
    let snapshots = window.generation().join("snapshots");
    window.crash(&mut first);

    let mut command = window.scrollback_command();
    command.env("EON_VENUS", window.root.join("missing-venus"));
    let mut replacement = window.start(command);
    assert!(!wait_for_exit(&mut replacement.child).success());
    assert!(!file.parent().unwrap().exists());
    assert!(!snapshots.exists());
}

/// With `[agents] terminal_access = "none"`, a calling terminal cannot
/// snapshot a terminal, but the window's key still opens Scrollback; stopping
/// the window with it live removes its snapshot and the store (EON-C10).
#[test]
fn scrollback_refuses_callers_without_terminal_access_and_ends_with_its_window() {
    let window = PairWindow::new();
    let mut supervisor = scrollback_window(&window, "\n[agents]\nterminal_access = \"none\"\n");
    let snapshots = window.generation().join("snapshots");
    let caller =
        process::environ(live_identity(&window.generation().join("orbit.sock")).process_id)
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
            .map(|token| String::from_utf8(token.to_vec()).unwrap())
            .unwrap();
    let tab = window.inspect("before").tabs.remove(0);
    assert!(matches!(
        protocol_action_from(
            &window.control(),
            "caller",
            Some(&caller),
            ProtocolAction::InvokePopup {
                tab: tab.id.clone(),
                entry: "scrollback".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        ),
        Response::Failure(failure) if failure.code == "terminal-access-off"
    ));
    assert_eq!(window.snapshot_directories(), Vec::<PathBuf>::new());
    assert_eq!(
        stable_tabs(&window.inspect("after-caller").tabs),
        stable_tabs(std::slice::from_ref(&tab))
    );

    // A shown Scrollback is refused to the terminal too, whatever the intent.
    let (popup, file) = window.open_scrollback("key");
    let shown = window.inspect("shown").tabs.remove(0);
    for (id, expected, intent) in [
        ("toggle", Some(popup.id.clone()), InvokeIntent::Toggle),
        ("focus", None, InvokeIntent::Focus),
    ] {
        assert!(
            matches!(
                protocol_action_from(
                    &window.control(),
                    id,
                    Some(&caller),
                    ProtocolAction::InvokePopup {
                        tab: shown.id.clone(),
                        entry: "scrollback".into(),
                        expected_instance: expected,
                        intent,
                    },
                ),
                Response::Failure(failure) if failure.code == "terminal-access-off"
            ),
            "{id}"
        );
        assert_eq!(
            stable_tabs(&window.inspect(&format!("{id}-after")).tabs),
            stable_tabs(std::slice::from_ref(&shown)),
            "{id}"
        );
    }
    window.stop_generation(&mut supervisor);
    assert!(!file.parent().unwrap().exists());
    assert!(!snapshots.exists());
}

/// `eon pane history TARGET --edit` opens TARGET's snapshot in a Scrollback
/// popup over its tab, as the window's key does for the shown terminal,
/// leaves the selection as it was and prints nothing but its JSON. It works
/// only in the shown workspace's active tab while that tab shows no
/// Scrollback; otherwise it changes nothing (EON-C35, EON-C10).
#[test]
fn pane_history_edit_opens_its_target_in_scrollback_in_the_active_tab_only() {
    let window = PairWindow::new();
    let mut supervisor = scrollback_window(&window, "");
    let editor_log = window.root.join("editor.log");
    let release = window.root.join("editor-release");
    assert!(
        window
            .eon(&["pane", "create", "--no-focus"])
            .status
            .success()
    );
    let before = window.inspect("before").tabs.remove(0);
    assert_eq!(before.selected_pane.as_deref(), Some("p1"));
    assert!(before.panes().any(|pane| pane.id == "p2"));

    // A pane the tab does not show: its snapshot, over the tab.
    let edit = window.eon(&["pane", "history", "p2", "--edit"]);
    assert_eq!(
        (edit.status.code(), stdout(&edit), edit.stderr.as_slice()),
        (Some(0), "", &b""[..])
    );
    let file = PathBuf::from(&wait_for_lines(&editor_log, 1)[0]);
    assert_eq!(file.file_name().unwrap(), "p2-scrollback.txt");
    let history = window.eon(&["pane", "history", "p2"]);
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!(
            "This pane is in a full-screen app; Eon shows only its screen. Quit the app to see the history.\n{}",
            stdout(&history)
        )
    );
    let shown = window.inspect("shown").tabs.remove(0);
    let popup = shown.popups.last().unwrap().clone();
    assert_eq!(
        (
            popup.entry.as_str(),
            shown.selected_popup.as_ref(),
            shown.selected_pane.as_deref()
        ),
        ("scrollback", Some(&popup.id), Some("p1"))
    );
    assert!(
        log_lines(&window.root.join("snapshot.log")).contains(&format!("{0}|{0}", popup.session))
    );

    // While Scrollback shows, another edit changes nothing.
    let again = window.eon(&["pane", "history", "p1", "--edit", "--json"]);
    assert_eq!(edit_error(&again), (Some(1), "unavailable".to_owned()));
    assert!(
        stdout(&again).contains("tab t1 already shows a Scrollback popup"),
        "{again:?}"
    );
    assert_eq!(
        stable_tabs(&window.inspect("again").tabs),
        stable_tabs(std::slice::from_ref(&shown))
    );
    assert_eq!(
        window.snapshot_directories(),
        [file.parent().unwrap().to_owned()]
    );
    fs::write(&release, "").unwrap();
    wait_for_removal(file.parent().unwrap());
    fs::remove_file(&release).unwrap();

    // JSON names the target and the Scrollback popup.
    let edit = window.eon(&["pane", "history", "p1", "--edit", "--json"]);
    assert_eq!(edit.status.code(), Some(0), "{edit:?}");
    let value: serde_json::Value = serde_json::from_str(stdout(&edit)).unwrap();
    let shown = window.inspect("json").tabs.remove(0);
    let popup = shown.popups.last().unwrap().clone();
    assert_eq!(
        value,
        serde_json::json!({
            "generation": value["generation"],
            "window": null,
            "session": "s1",
            "tab": "t1",
            "pane": "p1",
            "popup": null,
            "terminal": "terminal-1",
            "caller": null,
            "scrollback": {"popup": popup.id, "terminal": popup.session},
        })
    );
    let file = PathBuf::from(&wait_for_lines(&editor_log, 2)[1]);
    assert_eq!(file.file_name().unwrap(), "p1-scrollback.txt");
    assert!(matches!(
        window.invoke_popup("dismiss", "scrollback", Some(&popup.id)),
        Response::Snapshot(snapshot) if snapshot.tabs[0].popups.iter().all(|item| item.id != popup.id)
    ));
    wait_for_removal(file.parent().unwrap());

    // In a hidden tab it refuses with not-active and changes nothing.
    assert!(window.eon(&["tab", "create"]).status.success());
    let pair = window.inspect("tab-two").tabs[1].popups[0].session.clone();
    window.pair_launch(&pair);
    let home = window.home.to_str().unwrap();
    assert!(
        window
            .eon(&["tab", "directory", "t2", "--", home])
            .status
            .success()
    );
    let before = window.inspect("hidden");
    assert_eq!(before.active_tab, "t2");
    let human = window.eon(&["pane", "history", "p1", "--edit"]);
    assert_eq!((human.status.code(), stdout(&human)), (Some(1), ""));
    assert!(
        String::from_utf8_lossy(&human.stderr).contains("not-active"),
        "{human:?}"
    );
    let json = window.eon(&["pane", "history", "p2", "--edit", "--json"]);
    assert_eq!(edit_error(&json), (Some(1), "not-active".to_owned()));
    let after = window.inspect("hidden-after");
    assert_eq!(after.active_tab, before.active_tab);
    assert_eq!(stable_tabs(&after.tabs), stable_tabs(&before.tabs));

    // And in a hidden workspace.
    assert!(window.eon(&["session", "new"]).status.success());
    let pair = window.inspect("workspace-two").tabs[0].popups[0]
        .session
        .clone();
    window.pair_launch(&pair);
    let before = window.inspect("hidden-workspace");
    assert_eq!(before.workspace, "s2");
    let json = window.eon(&["pane", "history", "p1", "--edit", "--json"]);
    assert_eq!(edit_error(&json), (Some(1), "not-active".to_owned()));
    let after = window.inspect("hidden-workspace-after");
    assert_eq!(
        (after.workspace.as_str(), after.active_tab.as_str()),
        (before.workspace.as_str(), before.active_tab.as_str())
    );
    assert_eq!(stable_tabs(&after.tabs), stable_tabs(&before.tabs));
    assert_eq!(after.workspaces, before.workspaces);
    assert_eq!(window.snapshot_directories(), Vec::<PathBuf>::new());
    assert_eq!(log_lines(&editor_log).len(), 2);
    window.stop_generation(&mut supervisor);
}

/// `eon pane history TARGET --edit` refuses while the tab shows a transient
/// popup, in a tab still choosing its directory, when the export fails or
/// misses the terminal deadline, and when the editor is missing; each refusal
/// leaves no snapshot and changes no tab (EON-C35).
#[test]
fn pane_history_edit_refusals_leave_no_snapshot_and_change_nothing() {
    let window = PairWindow::new();
    let commands = window.root.join("commands");
    executable(
        &commands,
        "#!/bin/sh\nwhile [ ! -e \"$EON_TEST_STOP\" ]; do sleep 0.01; done\n",
    );
    let mut supervisor = scrollback_window(
        &window,
        &format!(
            "\n[popups.commands]\ncommand = [\"{}\"]\n",
            commands.display()
        ),
    );
    let refused = |id: &str, target: &str, code: &str| {
        let before = window.inspect(id);
        let output = window.eon(&["pane", "history", target, "--edit", "--json"]);
        assert_eq!(edit_error(&output), (Some(1), code.to_owned()), "{id}");
        assert_eq!(window.snapshot_directories(), Vec::<PathBuf>::new(), "{id}");
        let after = window.inspect(&format!("{id}-after"));
        assert_eq!(
            (after.active_tab, stable_tabs(&after.tabs)),
            (before.active_tab, stable_tabs(&before.tabs)),
            "{id}"
        );
    };

    // A transient popup the person has open stays open.
    let shown = match window.invoke_popup("commands", "commands", None) {
        Response::Snapshot(shown) => shown,
        other => panic!("Commands did not open: {other:?}"),
    };
    let commands = shown.tabs[0].selected_popup.clone().unwrap();
    refused("transient", "p1", "unavailable");
    assert!(matches!(
        window.invoke_popup("hide", "commands", Some(&commands)),
        Response::Snapshot(snapshot) if snapshot.tabs[0].selected_popup.is_none()
    ));

    fs::write(window.root.join("orbit-reply"), "busy").unwrap();
    refused("busy", "p1", "terminal-unavailable");
    fs::remove_file(window.root.join("orbit-reply")).unwrap();

    // A tab still choosing its directory.
    assert!(window.eon(&["tab", "create"]).status.success());
    let tab = window.inspect("tab-two").tabs.remove(1);
    window.pair_launch(&tab.popups[0].session);
    refused("pending", &tab.popups[0].id, "choose-directory-first");
    let home = window.home.to_str().unwrap();
    assert!(
        window
            .eon(&["tab", "directory", &tab.id, "--", home])
            .status
            .success()
    );
    let pane = window.inspect("committed").tabs[1]
        .panes()
        .next()
        .unwrap()
        .id
        .clone();

    fs::write(
        window.config.join("config.toml"),
        format!(
            "[terminal]\nfont_fallbacks = []\n\n[anima]\nenabled = false\n\n[editor]\ncommand = [\"{}\"]\n",
            window.root.join("missing").display()
        ),
    )
    .unwrap();
    refused("missing", &pane, "popup-command");

    // The timed-out read goes last: its late answer holds the lease's slot.
    fs::write(window.root.join("orbit-reply"), "slow-history").unwrap();
    refused("slow", &pane, "terminal-unavailable");
    assert!(!window.root.join("editor.log").exists());
    window.stop_generation(&mut supervisor);
}

/// With Scrollback disabled, `eon pane history TARGET --edit` exits 3 with
/// `unknown-popup` and changes nothing (EON-C35).
#[test]
fn pane_history_edit_with_scrollback_disabled_refuses_with_unknown_popup() {
    let window = PairWindow::new();
    let mut supervisor = scrollback_window(&window, "\n[popups.scrollback]\nenabled = false\n");
    let before = window.inspect("before");
    let human = window.eon(&["pane", "history", "p1", "--edit"]);
    assert_eq!((human.status.code(), stdout(&human)), (Some(3), ""));
    assert!(
        String::from_utf8_lossy(&human.stderr).starts_with("error unknown-popup: "),
        "{human:?}"
    );
    let json = window.eon(&["pane", "history", "p1", "--edit", "--json"]);
    assert_eq!(edit_error(&json), (Some(3), "unknown-popup".to_owned()));
    let after = window.inspect("after");
    assert_eq!(
        (after.active_tab, stable_tabs(&after.tabs)),
        (before.active_tab, stable_tabs(&before.tabs))
    );
    assert_eq!(window.snapshot_directories(), Vec::<PathBuf>::new());
    window.stop_generation(&mut supervisor);
}

/// A refused `eon pane history --edit --json`'s exit status and error code.
fn edit_error(output: &Output) -> (Option<i32>, String) {
    let value: serde_json::Value = serde_json::from_str(stdout(output)).unwrap();
    (
        output.status.code(),
        value["error"]["code"].as_str().unwrap().to_owned(),
    )
}

/// Fails a Scrollback press whose editor's Orbit publishes Ready after its
/// start and rollback deadlines, and returns the running editor's terminal
/// and its snapshot, which stays.
fn unconfirmed_scrollback(window: &PairWindow) -> (String, PathBuf) {
    let delay = window.root.join("editor-ready-delay");
    fs::write(&delay, "5100").unwrap();
    assert!(matches!(
        window.invoke_popup("late", "scrollback", None),
        Response::Failure(failure) if failure.code == "popup-start"
    ));
    fs::remove_file(&delay).unwrap();
    let file = PathBuf::from(&wait_for_lines(&window.root.join("editor.log"), 1)[0]);
    assert_eq!(
        window.snapshot_directories(),
        [file.parent().unwrap().to_owned()]
    );
    let (session, recorded) = log_lines(&window.root.join("snapshot.log"))[0]
        .split_once('|')
        .map(|(session, recorded)| (session.to_owned(), recorded.to_owned()))
        .unwrap();
    assert_eq!(recorded, session);
    // Later turns keep it while the editor may run.
    window.inspect("later");
    assert!(file.exists());
    (session, file)
}

/// A Scrollback start that fails without confirming its rollback may leave
/// its editor running, so its recorded snapshot stays: a stop that ends the
/// editor removes it and the store, and a recovery that adopts the editor
/// removes it when the editor ends (EON-C10).
#[test]
fn a_scrollback_start_whose_rollback_is_unconfirmed_keeps_its_snapshot() {
    let window = PairWindow::new();
    let mut supervisor = scrollback_window(&window, "");
    let snapshots = window.generation().join("snapshots");
    let (_, file) = unconfirmed_scrollback(&window);
    window.stop_generation(&mut supervisor);
    assert!(!file.parent().unwrap().exists());
    assert!(!snapshots.exists());
}

#[test]
fn a_recovery_adopts_the_snapshot_of_an_unconfirmed_scrollback_start() {
    let window = PairWindow::new();
    let mut first = scrollback_window(&window, "");
    let (session, file) = unconfirmed_scrollback(&window);

    window.crash(&mut first);
    let mut replacement = window.spawn_scrollback();
    assert!(
        window.inspect("recovered").tabs[0]
            .panes()
            .any(|pane| pane.session == session)
    );
    assert!(file.exists());
    fs::write(window.root.join("editor-release"), "").unwrap();
    wait_for_removal(file.parent().unwrap());
    window.stop_generation(&mut replacement);
}

#[test]
fn terminal_access_none_refuses_every_read_and_send() {
    let root = temporary_directory();
    let (mut supervisor, _, control) =
        terminal_access_workspace(&root, "\n[agents]\nterminal_access = \"none\"\n");
    for (id, action) in [
        (
            "read",
            ProtocolAction::ReadTerminal {
                target: "p1".into(),
            },
        ),
        (
            "history",
            ProtocolAction::ReadTerminalHistory {
                target: "p1".into(),
            },
        ),
        (
            "edit",
            ProtocolAction::EditTerminalHistory {
                target: "p1".into(),
            },
        ),
        (
            "send",
            ProtocolAction::SendTerminal {
                target: "p1".into(),
                force: false,
                input: TerminalInput::Paste("echo hi".into()),
            },
        ),
    ] {
        assert!(matches!(
            protocol_action(&control, id, action),
            Response::Failure(failure) if failure.code == "terminal-access-off"
        ));
    }
    assert!(!root.join("input.log").exists());
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pane_send_uses_the_windows_typing_interval() {
    for quiet in [0, 60000] {
        let root = temporary_directory();
        let (mut supervisor, _, _) = terminal_access_workspace(
            &root,
            &format!("\n[agents]\nsend_typing_quiet_ms = {quiet}\n"),
        );
        fs::write(root.join("orbit-reply"), "typing").unwrap();
        let output = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")))
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"))
            .args(["pane", "send", "p1", "--json", "--", "x"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if quiet == 0 { 0 } else { 5 }),
            "{output:?}"
        );
        assert_eq!(
            fs::read_to_string(root.join("quiet.log")).unwrap(),
            format!("{quiet}\n")
        );
        assert_eq!(root.join("input.log").exists(), quiet == 0);
        fs::write(&supervisor.stop, "").unwrap();
        wait_for_successful_exit(&mut supervisor.child);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn pane_read_prints_the_text_or_json_and_exits_by_failure() {
    let root = temporary_directory();
    let (mut supervisor, generation, _) = terminal_access_workspace(&root, "");
    let eon = |root: &Path| {
        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        command
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"));
        command
    };
    let run = |arguments: &[&str]| eon(&root).args(arguments).output().unwrap();
    let before = stable_workspace_json(&run(&["session", "--json"]));
    let human = run(&["pane", "read", "p1"]);
    assert_eq!(human.status.code(), Some(0));
    assert_eq!(stdout(&human), "screen of terminal-1\n");
    assert!(human.stderr.is_empty());
    let id = generation.file_name().unwrap().to_str().unwrap();
    let expected = |caller| {
        serde_json::json!({
            "generation": id,
            "window": null,
            "session": "s1",
            "tab": "t1",
            "pane": "p1",
            "popup": null,
            "terminal": "terminal-1",
            "caller": caller,
            "revision": 1,
            "screen": "primary",
            "rows": 24,
            "columns": 80,
            "title": "terminal-1",
            "directory": "/fake",
            "text": "screen of terminal-1",
            "omitted_rows": 0,
            "lossy": true,
        })
    };
    let json = run(&["pane", "read", "p1", "--json"]);
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout(&json)).unwrap(),
        expected(serde_json::Value::Null)
    );
    // From an Eon terminal, `caller` names it as `eon session` does.
    let environment = process::environ(live_identity(&generation.join("orbit.sock")).process_id);
    let caller = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"EON_TERMINAL="))
        .map(|token| String::from_utf8(token.to_vec()).unwrap())
        .unwrap();
    let from_caller = eon(&root)
        .args(["pane", "read", "p1", "--json"])
        .env("EON_TERMINAL", &caller)
        .output()
        .unwrap();
    assert_eq!(from_caller.status.code(), Some(0));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout(&from_caller)).unwrap(),
        expected(serde_json::json!({
            "terminal": "terminal-1",
            "tab": "t1",
            "pane": "p1",
            "popup": null,
        }))
    );
    for (arguments, status, code) in [
        (&["pane", "read", "p99"][..], 3, Some("unknown-target")),
        (&["pane", "read", "bad/id"], 2, Some("malformed-action")),
        (&["pane", "read"], 2, None),
        (&["pane", "read", "p1", "p2"], 2, None),
    ] {
        let human = run(arguments);
        assert_eq!(human.status.code(), Some(status), "{arguments:?}");
        assert!(human.stdout.is_empty(), "{arguments:?}");
        let json = run(&[arguments, &["--json"]].concat());
        assert_eq!(json.status.code(), Some(status), "{arguments:?}");
        if let Some(code) = code {
            assert!(String::from_utf8_lossy(&human.stderr).starts_with(&format!("error {code}: ")));
            assert!(
                stdout(&json).contains(&format!("\"code\":\"{code}\"")),
                "{arguments:?}"
            );
        }
    }
    // Orbit's refusals reach the CLI as exit 1 without terminal content.
    for (reply, code) in [
        ("closed", "terminal-ended"),
        ("busy", "terminal-unavailable"),
    ] {
        fs::write(root.join("orbit-reply"), reply).unwrap();
        let human = run(&["pane", "read", "p1"]);
        let json = run(&["pane", "read", "p1", "--json"]);
        assert_eq!(
            (human.status.code(), json.status.code()),
            (Some(1), Some(1)),
            "{reply}"
        );
        assert!(human.stdout.is_empty(), "{reply}");
        assert!(String::from_utf8_lossy(&human.stderr).starts_with(&format!("error {code}: ")));
        assert!(
            stdout(&json).contains(&format!("\"code\":\"{code}\"")),
            "{reply}"
        );
        assert!(!stdout(&json).contains("screen of"), "{reply}");
    }
    fs::remove_file(root.join("orbit-reply")).unwrap();
    assert_eq!(run(&["pane", "read", "p1"]).status.code(), Some(0));
    // Reading changed nothing a person sees.
    assert_eq!(stable_workspace_json(&run(&["session", "--json"])), before);
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    let missing = run(&["pane", "read", "p1", "--json"]);
    assert_eq!(missing.status.code(), Some(4));
    assert!(stdout(&missing).contains("\"code\":\"missing-supervisor\""));
    fs::remove_dir_all(&root).unwrap();

    let off = temporary_directory();
    let (mut supervisor, _, _) =
        terminal_access_workspace(&off, "\n[agents]\nterminal_access = \"none\"\n");
    let refused = eon(&off)
        .args(["pane", "read", "p1", "--json"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(stdout(&refused).contains("\"code\":\"terminal-access-off\""));
    assert!(!stdout(&refused).contains("screen of"));
    let refused = eon(&off).args(["pane", "read", "p1"]).output().unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(refused.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refused.stderr).starts_with("error terminal-access-off: "));
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(off).unwrap();
}

#[test]
fn pane_history_prints_the_text_or_json_and_names_what_the_bound_left_out() {
    let root = temporary_directory();
    let (mut supervisor, generation, control) = terminal_access_workspace(&root, "");
    let eon = |root: &Path| {
        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        command
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"));
        command
    };
    let run = |arguments: &[&str]| eon(&root).args(arguments).output().unwrap();
    let json = |output: &Output| serde_json::from_str::<serde_json::Value>(stdout(output)).unwrap();
    let before = stable_workspace_json(&run(&["session", "--json"]));
    let human = run(&["pane", "history", "p1"]);
    assert_eq!(human.status.code(), Some(0));
    assert_eq!(
        stdout(&human),
        "history of terminal-1\nscreen of terminal-1\n"
    );
    assert!(human.stderr.is_empty());
    let id = generation.file_name().unwrap().to_str().unwrap();
    let read = run(&["pane", "history", "p1", "--json"]);
    assert_eq!(read.status.code(), Some(0));
    assert!(read.stderr.is_empty());
    assert_eq!(
        json(&read),
        serde_json::json!({
            "generation": id,
            "window": null,
            "session": "s1",
            "tab": "t1",
            "pane": "p1",
            "popup": null,
            "terminal": "terminal-1",
            "caller": null,
            "revision": 2,
            "screen": "alternate",
            "rows": 24,
            "columns": 80,
            "title": "terminal-1",
            "directory": "/fake",
            "text": "history of terminal-1\nscreen of terminal-1",
            "omitted_lines": 0,
            "cut_bytes": 0,
            "lossy": true,
        })
    );
    // An answer over EONW's general 2 MiB reaches the CLI whole; the notice
    // goes to standard error, and only in human output.
    fs::write(root.join("orbit-reply"), "long-history").unwrap();
    let long = run(&["pane", "history", "p1"]);
    assert_eq!(long.status.code(), Some(0));
    assert!(long.stdout.len() > 2 * 1024 * 1024);
    assert_eq!(long.stdout, ("h\n".repeat(3 << 19) + "end\n").into_bytes());
    assert_eq!(
        String::from_utf8_lossy(&long.stderr),
        "eon: the history is over its 8 MiB bound; left out the 5 earliest lines\n"
    );
    let long = run(&["pane", "history", "p1", "--json"]);
    assert_eq!(long.status.code(), Some(0));
    assert!(long.stderr.is_empty());
    assert_eq!(
        (&json(&long)["omitted_lines"], &json(&long)["cut_bytes"]),
        (&serde_json::json!(5), &serde_json::json!(0))
    );
    fs::write(root.join("orbit-reply"), "cut-history").unwrap();
    let cut = run(&["pane", "history", "p1"]);
    assert_eq!((cut.status.code(), stdout(&cut)), (Some(0), "line\n"));
    assert_eq!(
        String::from_utf8_lossy(&cut.stderr),
        "eon: the history is over its 8 MiB bound; left out the earliest line and the first 7 \
         bytes of the newest line\n"
    );
    fs::write(root.join("orbit-reply"), "empty-history").unwrap();
    let empty = run(&["pane", "history", "p1"]);
    assert_eq!((empty.status.code(), stdout(&empty)), (Some(0), "\n"));
    assert!(empty.stderr.is_empty());
    for (arguments, status, code) in [
        (&["pane", "history", "p99"][..], 3, Some("unknown-target")),
        (&["pane", "history", "bad/id"], 2, Some("malformed-action")),
        (&["pane", "history"], 2, None),
        (&["pane", "history", "p1", "p2"], 2, None),
    ] {
        let human = run(arguments);
        assert_eq!(human.status.code(), Some(status), "{arguments:?}");
        assert!(human.stdout.is_empty(), "{arguments:?}");
        let json = run(&[arguments, &["--json"]].concat());
        assert_eq!(json.status.code(), Some(status), "{arguments:?}");
        if let Some(code) = code {
            assert!(String::from_utf8_lossy(&human.stderr).starts_with(&format!("error {code}: ")));
            assert!(
                stdout(&json).contains(&format!("\"code\":\"{code}\"")),
                "{arguments:?}"
            );
        }
    }
    // Orbit's refusals reach the CLI as exit 1 without terminal content.
    for (reply, code) in [
        ("closed", "terminal-ended"),
        ("busy", "terminal-unavailable"),
    ] {
        fs::write(root.join("orbit-reply"), reply).unwrap();
        let human = run(&["pane", "history", "p1"]);
        let json = run(&["pane", "history", "p1", "--json"]);
        assert_eq!(
            (human.status.code(), json.status.code()),
            (Some(1), Some(1)),
            "{reply}"
        );
        assert!(human.stdout.is_empty(), "{reply}");
        assert!(String::from_utf8_lossy(&human.stderr).starts_with(&format!("error {code}: ")));
        assert!(
            stdout(&json).contains(&format!("\"code\":\"{code}\"")),
            "{reply}"
        );
        assert!(!stdout(&json).contains("of session"), "{reply}");
    }
    fs::remove_file(root.join("orbit-reply")).unwrap();
    // Reading changed nothing a person sees.
    assert_eq!(stable_workspace_json(&run(&["session", "--json"])), before);
    // A popup's history, named by its uN.
    let Response::Snapshot(opened) = protocol_action(
        &control,
        "open",
        ProtocolAction::InvokePopup {
            tab: "t1".into(),
            entry: "commands".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
    ) else {
        panic!("the Commands popup did not open");
    };
    let popup = opened.tabs[0]
        .popups
        .iter()
        .find(|popup| popup.entry == "commands")
        .unwrap()
        .clone();
    let read = run(&["pane", "history", &popup.id, "--json"]);
    assert_eq!(read.status.code(), Some(0));
    let read = json(&read);
    assert_eq!(
        (
            &read["pane"],
            &read["popup"],
            &read["terminal"],
            &read["text"]
        ),
        (
            &serde_json::Value::Null,
            &serde_json::json!(popup.id),
            &serde_json::json!(popup.session),
            &serde_json::json!(format!("history of {0}\nscreen of {0}", popup.session)),
        )
    );
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(&root).unwrap();

    let off = temporary_directory();
    let (mut supervisor, _, _) =
        terminal_access_workspace(&off, "\n[agents]\nterminal_access = \"none\"\n");
    let refused = eon(&off)
        .args(["pane", "history", "p1", "--json"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(stdout(&refused).contains("\"code\":\"terminal-access-off\""));
    assert!(!stdout(&refused).contains("of session"));
    let refused = eon(&off).args(["pane", "history", "p1"]).output().unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(refused.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refused.stderr).starts_with("error terminal-access-off: "));
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(off).unwrap();
}

#[test]
fn pane_send_pastes_then_waits_for_enter_and_exits_by_failure() {
    let root = temporary_directory();
    let (mut supervisor, generation, _) =
        terminal_access_workspace(&root, "\n[agents]\nsend_enter_delay_ms = 400\n");
    let eon = |root: &Path| {
        let mut command = eon_command(Path::new(env!("CARGO_BIN_EXE_eon")));
        command
            .env("EON_RUNTIME_DIR", root.join("runtime"))
            .env("EON_CONFIG_HOME", root.join("config"));
        command
    };
    let run = |arguments: &[&str]| eon(&root).args(arguments).output().unwrap();
    let sent = || fs::read_to_string(root.join("input.log")).unwrap_or_default();
    let json = |output: &Output| serde_json::from_str::<serde_json::Value>(stdout(output)).unwrap();
    let before = stable_workspace_json(&run(&["session", "--json"]));
    let id = generation.file_name().unwrap().to_str().unwrap();
    let reply = |bytes: usize, bracketed: serde_json::Value, enter: bool| {
        serde_json::json!({
            "generation": id,
            "window": null,
            "session": "s1",
            "tab": "t1",
            "pane": "p1",
            "popup": null,
            "terminal": "terminal-1",
            "caller": null,
            "bytes": bytes,
            "bracketed": bracketed,
            "enter": enter,
        })
    };

    let paste = run(&["pane", "send", "p1", "--json", "--", "echo hi"]);
    assert_eq!(paste.status.code(), Some(0));
    assert_eq!(json(&paste), reply(7, true.into(), false));
    // --enter waits the window's delay between the paste and the Enter.
    let submit = run(&["pane", "send", "p1", "--enter", "--json", "--", "ls"]);
    assert_eq!(submit.status.code(), Some(0));
    assert_eq!(json(&submit), reply(2, true.into(), true));
    let times = fs::read_to_string(root.join("input-times.log")).unwrap();
    let times = times
        .lines()
        .map(|line| line.split_once(' ').unwrap())
        .collect::<Vec<_>>();
    let [.., (pasted, "Paste(\"ls\")"), (entered, "Enter")] = times.as_slice() else {
        panic!("{times:?}");
    };
    let waited = entered.parse::<u64>().unwrap() - pasted.parse::<u64>().unwrap();
    assert!(waited >= 400, "Enter came {waited} ms after the paste");
    let human = run(&["pane", "send", "p1", "--enter"]);
    assert_eq!(human.status.code(), Some(0));
    assert!(human.stdout.is_empty() && human.stderr.is_empty());
    // Empty TEXT counts as none: with --enter it sends Enter alone.
    let enter = run(&["pane", "send", "p1", "--enter", "--json", "--", ""]);
    assert_eq!(json(&enter), reply(0, serde_json::Value::Null, true));
    let mut piped = eon(&root)
        .args(["pane", "send", "p1", "--stdin", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    piped
        .stdin
        .take()
        .unwrap()
        .write_all(b"from stdin")
        .unwrap();
    let piped = piped.wait_with_output().unwrap();
    assert_eq!(json(&piped), reply(10, true.into(), false));
    assert_eq!(
        sent(),
        "terminal-1 Paste(\"echo hi\")\nterminal-1 Paste(\"ls\")\nterminal-1 Enter\n\
         terminal-1 Enter\nterminal-1 Enter\nterminal-1 Paste(\"from stdin\")\n"
    );
    // Without --stdin or TEXT, input waiting in a pipe, socket or file fails
    // rather than vanish, also when it arrives just after Eon starts. An input
    // with nothing in it yet, as agents' shells give commands, an ended one, or
    // one read to its end, still sends Enter alone.
    let log = sent();
    enum When {
        Now,
        InWait,
    }
    // Whether the last write in the wait proves nothing: Eon went unsampled for
    // 100 ms, which could hide its whole 100 ms wait, or the write landed 100 ms
    // or more after Eon was last seen running, before its wait began.
    let late = std::cell::Cell::new(false);
    let send_enter =
        |json: bool, stdin: Stdio, mut writer: Option<Box<dyn Write>>, when: Option<When>| {
            let spawned = Instant::now();
            let mut child = eon(&root)
                .args(["pane", "send", "p1", "--enter"])
                .args(json.then_some("--json"))
                .stdin(stdin)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            if let (Some(writer), Some(when)) = (&mut writer, when) {
                // Before it connects, Eon waits only in its stdin wait, so the
                // samples that first find it waiting find that wait. That holds
                // on Linux, so there a CLI that skips its wait fails this case;
                // on macOS, whose samples also see loader and page-in waits, it
                // is best effort, helped by asking for two in a row.
                let mut running = spawned;
                let mut gap = Duration::ZERO;
                if matches!(when, When::InWait) {
                    let mut sampled = spawned;
                    let mut waits = 0;
                    while waits < 2 {
                        assert!(
                            spawned.elapsed() < Duration::from_secs(10),
                            "Eon never waited"
                        );
                        // Taken before the sample, so Eon ran no earlier.
                        let at = Instant::now();
                        gap = gap.max(at - sampled);
                        sampled = at;
                        match process_waits(child.id()) {
                            Ok(Some(false)) => {
                                waits = 0;
                                running = at;
                            }
                            Ok(Some(true)) => waits += 1,
                            // Ended: the write below fails, which fails the case
                            // unless it came late.
                            Ok(None) => break,
                            Err(error) => panic!("cannot sample Eon: {error}"),
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                let wrote = writer.write_all(b"lost text");
                let after = running.elapsed();
                late.set(gap.max(after) >= Duration::from_millis(100));
                // Eon gone before the wait's bound did not wait.
                assert!(
                    wrote.is_ok() || late.get(),
                    "Eon ended within {after:?} of running, inside the stdin wait: {wrote:?}"
                );
            }
            wait_for_exit(&mut child);
            drop(writer);
            child.wait_with_output().unwrap()
        };
    let pipe = || {
        let (reader, writer) = std::io::pipe().unwrap();
        (
            Stdio::from(reader),
            Some(Box::new(writer) as Box<dyn Write>),
        )
    };
    let socket = || {
        let (reader, writer) = UnixStream::pair().unwrap();
        let reader = std::os::fd::OwnedFd::from(reader);
        (
            Stdio::from(reader),
            Some(Box::new(writer) as Box<dyn Write>),
        )
    };
    let file = root.join("piped-text");
    fs::write(&file, "lost text").unwrap();
    let mut late_enters = 0;
    for json in [false, true] {
        let (written, mut writer) = pipe();
        writer.as_mut().unwrap().write_all(b"lost text").unwrap();
        drop(writer);
        let refused = [
            send_enter(json, written, None, None),
            // Written once Eon waits, so only the wait sees these bytes. A
            // write a loaded machine delays past the bound proves nothing, so
            // that run, which may have sent Enter, goes again.
            {
                let mut runs = 0;
                loop {
                    runs += 1;
                    let (stdin, writer) = pipe();
                    let output = send_enter(json, stdin, writer, Some(When::InWait));
                    if !late.get() {
                        break output;
                    }
                    assert!(runs < 5, "the write landed past the wait {runs} times");
                    match output.status.code() {
                        Some(0) => late_enters += 1,
                        Some(2) => {}
                        code => panic!("a run whose write came late exited {code:?}"),
                    }
                }
            },
            {
                let (stdin, writer) = socket();
                send_enter(json, stdin, writer, Some(When::Now))
            },
            send_enter(json, fs::File::open(&file).unwrap().into(), None, None),
        ];
        for (case, output) in refused.iter().enumerate() {
            assert_eq!(output.status.code(), Some(2), "{case} {json}");
            let message = if json {
                assert!(output.stderr.is_empty());
                assert!(stdout(output).contains("\"code\":\"unread-stdin\""));
                stdout(output).to_owned()
            } else {
                assert!(output.stdout.is_empty());
                let message = String::from_utf8_lossy(&output.stderr).into_owned();
                assert!(message.starts_with("error unread-stdin: "), "{message}");
                message
            };
            assert!(
                message.contains("--stdin") && message.contains("</dev/null"),
                "{message}"
            );
        }
    }
    let log = log + &"terminal-1 Enter\n".repeat(late_enters);
    assert_eq!(sent(), log);
    let mut read = fs::File::open(&file).unwrap();
    read.read_to_end(&mut Vec::new()).unwrap();
    let accepted = [
        {
            let (stdin, writer) = pipe();
            send_enter(true, stdin, writer, None)
        },
        // Ended: its writer closes before Eon starts, so the pipe is at EOF.
        {
            let (stdin, writer) = pipe();
            drop(writer);
            send_enter(true, stdin, None, None)
        },
        {
            let (stdin, writer) = socket();
            send_enter(true, stdin, writer, None)
        },
        send_enter(true, read.into(), None, None),
    ];
    for (case, output) in accepted.iter().enumerate() {
        assert_eq!(output.status.code(), Some(0), "{case}");
        assert_eq!(json(output), reply(0, serde_json::Value::Null, true));
    }
    assert_eq!(sent(), log + &"terminal-1 Enter\n".repeat(4));

    // Refusals before the supervisor, and the supervisor's, send nothing.
    let log = sent();
    let too_large = "a".repeat(64 * 1024 + 1);
    let not_utf8 = OsStr::from_bytes(b"\xff");
    for (arguments, status, code) in [
        (
            vec![OsStr::new("--"), OsStr::new("\u{1b}[31mred")],
            2,
            Some("invalid-text"),
        ),
        (vec![OsStr::new("--"), not_utf8], 2, Some("invalid-text")),
        (
            vec![OsStr::new("--"), OsStr::new(&too_large)],
            2,
            Some("input-too-large"),
        ),
        (vec![], 2, None),
        (vec![OsStr::new("--"), OsStr::new("")], 2, None),
        (vec![OsStr::new("text")], 2, None),
        (
            vec![OsStr::new("--stdin"), OsStr::new("--"), OsStr::new("x")],
            2,
            None,
        ),
        // Empty standard input is no TEXT.
        (vec![OsStr::new("--stdin")], 2, None),
    ] {
        for flags in [&[][..], &[OsStr::new("--json")]] {
            let output = eon(&root)
                .args(["pane", "send", "p1"])
                .args(flags)
                .args(&arguments)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(status),
                "{arguments:?} {flags:?}"
            );
            match (code, flags.is_empty()) {
                (Some(code), true) => assert!(
                    String::from_utf8_lossy(&output.stderr).starts_with(&format!("error {code}: ")),
                    "{arguments:?}"
                ),
                (Some(code), false) => assert!(
                    stdout(&output).contains(&format!("\"code\":\"{code}\"")),
                    "{arguments:?}"
                ),
                (None, _) => assert!(output.stdout.is_empty(), "{arguments:?}"),
            }
        }
    }
    let missing = run(&["pane", "send", "p99", "--json", "--", "x"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(stdout(&missing).contains("\"code\":\"unknown-target\""));
    // Orbit's refusals exit 1, with nothing sent.
    for (script, text, code) in [
        ("unbracketed", "a\nb", "unbracketed-newline"),
        ("closed", "x", "terminal-ended"),
        ("busy", "x", "terminal-unavailable"),
    ] {
        fs::write(root.join("orbit-reply"), script).unwrap();
        let human = run(&["pane", "send", "p1", "--enter", "--", text]);
        assert_eq!(human.status.code(), Some(1), "{script}");
        assert!(String::from_utf8_lossy(&human.stderr).starts_with(&format!("error {code}: ")));
        let output = run(&["pane", "send", "p1", "--enter", "--json", "--", text]);
        assert_eq!(output.status.code(), Some(1), "{script}");
        assert!(
            stdout(&output).contains(&format!("\"code\":\"{code}\"")),
            "{script}"
        );
    }
    assert_eq!(sent(), log);
    fs::write(root.join("orbit-reply"), "typing").unwrap();
    for args in [vec!["--", "x"], vec!["--enter"]] {
        for flags in [vec![], vec!["--json"]] {
            let output = run(&[vec!["pane", "send", "p1"], flags.clone(), args.clone()].concat());
            assert_eq!(output.status.code(), Some(5), "{output:?}");
            if flags.is_empty() {
                assert!(output.stdout.is_empty());
                assert!(
                    String::from_utf8_lossy(&output.stderr).starts_with("error person-typing: ")
                );
            } else {
                assert_eq!(json(&output)["error"]["code"], "person-typing");
            }
        }
    }
    assert_eq!(sent(), log);
    assert!(
        fs::read_to_string(root.join("quiet.log"))
            .unwrap()
            .lines()
            .all(|line| line == "2000")
    );
    let forced = run(&[
        "pane", "send", "p1", "--force", "--enter", "--json", "--", "forced",
    ]);
    assert_eq!(forced.status.code(), Some(0), "{forced:?}");
    assert_eq!(json(&forced), reply(6, true.into(), true));
    assert!(
        fs::read_to_string(root.join("quiet.log"))
            .unwrap()
            .ends_with("0\n0\n")
    );
    let log = sent();
    // Do not retry text when the person starts typing during the Enter delay.
    fs::write(root.join("orbit-reply"), "typing-enter").unwrap();
    for flags in [vec![], vec!["--json"]] {
        let half = run(&[
            vec!["pane", "send", "p1", "--enter"],
            flags.clone(),
            vec!["--", "partial"],
        ]
        .concat());
        assert_eq!(half.status.code(), Some(1), "{half:?}");
        if flags.is_empty() {
            let error = String::from_utf8_lossy(&half.stderr);
            assert!(error.starts_with("error person-typing: "));
            assert!(!error.contains("--force"), "{error}");
            assert!(
                error.contains("--enter alone without resending the text"),
                "{error}"
            );
            assert!(error.ends_with("the text was pasted; Enter was not sent\n"));
        } else {
            let mut value = json(&half);
            let error = value.as_object_mut().unwrap().remove("error").unwrap();
            assert_eq!(error["code"], "person-typing");
            let detail = error["detail"].as_str().unwrap();
            assert!(!detail.contains("--force"), "{detail}");
            assert!(
                detail.contains("--enter alone without resending the text"),
                "{detail}"
            );
            assert_eq!(value, reply(7, true.into(), false));
        }
    }
    assert_eq!(
        sent(),
        format!("{log}{}", "terminal-1 Paste(\"partial\")\n".repeat(2))
    );
    let log = sent();
    // A paste that lands before a failed Enter stays, and the reply says so.
    fs::write(root.join("orbit-reply"), "busy-enter").unwrap();
    let half = run(&["pane", "send", "p1", "--enter", "--json", "--", "x"]);
    assert_eq!(half.status.code(), Some(1));
    let mut value = json(&half);
    let error = value.as_object_mut().unwrap().remove("error").unwrap();
    assert_eq!(error["code"], "terminal-unavailable");
    assert_eq!(value, reply(1, true.into(), false));
    let half = run(&["pane", "send", "p1", "--enter", "--", "y"]);
    assert_eq!(half.status.code(), Some(1));
    assert!(half.stdout.is_empty());
    let error = String::from_utf8_lossy(&half.stderr);
    assert!(error.starts_with("error terminal-unavailable: "), "{error}");
    assert!(
        error.ends_with("the text was pasted; Enter was not sent\n"),
        "{error}"
    );
    assert_eq!(
        sent(),
        format!("{log}terminal-1 Paste(\"x\")\nterminal-1 Paste(\"y\")\n")
    );
    fs::remove_file(root.join("orbit-reply")).unwrap();
    // A help flag after `--` is TEXT, not a request for help.
    let log = sent();
    let help_text = run(&["pane", "send", "p1", "--json", "--", "--help"]);
    assert_eq!(help_text.status.code(), Some(0));
    assert_eq!(json(&help_text), reply(6, true.into(), false));
    assert_eq!(sent(), format!("{log}terminal-1 Paste(\"--help\")\n"));
    // Sending changed nothing a person sees but the program.
    assert_eq!(stable_workspace_json(&run(&["session", "--json"])), before);
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    let gone = run(&["pane", "send", "p1", "--json", "--", "x"]);
    assert_eq!(gone.status.code(), Some(4));
    assert!(stdout(&gone).contains("\"code\":\"missing-supervisor\""));
    fs::remove_dir_all(&root).unwrap();

    let off = temporary_directory();
    let (mut supervisor, _, _) =
        terminal_access_workspace(&off, "\n[agents]\nterminal_access = \"none\"\n");
    let refused = eon(&off)
        .args([
            "pane", "send", "p1", "--force", "--enter", "--json", "--", "x",
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(stdout(&refused).contains("\"code\":\"terminal-access-off\""));
    assert!(!off.join("input.log").exists());
    fs::write(&supervisor.stop, "").unwrap();
    wait_for_successful_exit(&mut supervisor.child);
    fs::remove_dir_all(off).unwrap();
}

/// XNU's kernel-private FWASWRITTEN, "descriptor was written" (xnu
/// bsd/sys/fcntl.h), which F_GETFL reports once anything has written through
/// the open file description.
#[cfg(target_os = "macos")]
const FWASWRITTEN: libc::c_int = 0x0001_0000;

/// The status flags of `fd`, without what a kernel records on its own, so that
/// only flags someone set count when they are compared.
fn status_flags(fd: std::os::fd::RawFd) -> libc::c_int {
    // SAFETY: F_GETFL only reads the descriptor's flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    #[cfg(target_os = "macos")]
    let flags = flags & !FWASWRITTEN;
    flags
}

/// A pseudoterminal's controller and the device a child reads as stdin, both
/// closed on exec.
fn pseudo_terminal() -> (fs::File, std::os::fd::OwnedFd) {
    use std::os::fd::{FromRawFd, OwnedFd};
    let (mut leader, mut device) = (-1, -1);
    // SAFETY: openpty initializes both descriptors on success.
    let opened = unsafe {
        libc::openpty(
            &raw mut leader,
            &raw mut device,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0, "{}", std::io::Error::last_os_error());
    // SAFETY: a successful openpty transferred ownership of both descriptors.
    let (leader, device) = unsafe { (fs::File::from_raw_fd(leader), OwnedFd::from_raw_fd(device)) };
    for descriptor in [leader.as_raw_fd(), device.as_raw_fd()] {
        // SAFETY: both descriptors are open and owned here.
        assert_eq!(
            unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
    }
    (leader, device)
}

/// Runs `command` with stdin null and returns its output, killing it and
/// failing if it has not exited within 10 seconds.
fn bounded_output(command: &mut Command) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{command:?} did not finish");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

/// Runs `command` with a pseudoterminal as stdin, bounding every wait. With an
/// `answer`, it waits for the `[y/N] ` question, runs `meanwhile`, then
/// answers; without one the command must finish unasked. If `meanwhile`
/// fails, the command is killed. The controller stays open until the command
/// exits.
fn at_terminal(command: &mut Command, answer: Option<&[u8]>, meanwhile: impl FnOnce()) -> Output {
    let (mut leader, device) = pseudo_terminal();
    let mut child = command
        .stdin(Stdio::from(device))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        out.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut err = child.stderr.take().unwrap();
    let (send, received) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || {
        let mut chunk = [0; 256];
        while let Ok(count @ 1..) = err.read(&mut chunk) {
            if send.send(chunk[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stderr = Vec::new();
    if let Some(answer) = answer {
        while !stderr.ends_with(b"[y/N] ") {
            let Ok(bytes) =
                received.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            else {
                child.kill().unwrap();
                panic!("no question: {}", String::from_utf8_lossy(&stderr));
            };
            stderr.extend(bytes);
        }
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(meanwhile)) {
            let _ = child.kill();
            let _ = child.wait();
            std::panic::resume_unwind(panic);
        }
        leader.write_all(answer).unwrap();
    }
    let Some(status) =
        exit_reading_terminals(&mut child, std::slice::from_mut(&mut leader), deadline)
    else {
        child.kill().unwrap();
        panic!("did not finish: {}", String::from_utf8_lossy(&stderr));
    };
    drop(leader);
    reader.join().unwrap();
    stderr.extend(received.try_iter().flatten());
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr,
    }
}

/// Waits until the busy terminals are exactly `panes` and the shown
/// workspace's labels have settled, and returns its JSON.
fn wait_for_settled_workspace(root: &Path, panes: &[&str]) -> Output {
    wait_for_busy_panes(root, None, panes);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = quit_command(root)
            .args(["session", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let connecting =
            |name: &serde_json::Value| name.as_str().unwrap().ends_with("  connecting");
        let settled = !value["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|tab| tab["panes"].as_array().unwrap())
            .any(|pane| connecting(&pane["label"]))
            && !value["busy"]["terminals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| connecting(&item["name"]));
        if settled {
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "labels stayed connecting: {value}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Shows the Agent popup over the shown workspace's active `tab` and
/// returns its terminal's endpoint.
fn show_agent(control: &Path, id: &str, tab: &str) -> String {
    let response = protocol_action(
        control,
        id,
        ProtocolAction::InvokePopup {
            tab: tab.into(),
            entry: "agent".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
    );
    let Response::Snapshot(snapshot) = response else {
        panic!("{id}: Agent did not open: {response:?}");
    };
    let tab = snapshot.tabs.iter().find(|item| item.id == tab).unwrap();
    let popup = tab
        .popups
        .iter()
        .find(|popup| tab.selected_popup.as_ref() == Some(&popup.id))
        .unwrap();
    String::from_utf8(popup.endpoint.clone()).unwrap()
}

/// Whether the fake Orbit at `endpoint` still records its terminal as live.
fn terminal_is_live(endpoint: &str) -> bool {
    fs::read(artifact_path(Path::new(endpoint), ".record")).is_ok_and(|bytes| {
        matches!(
            management::decode_record(&bytes),
            Ok(ManagementRecord::Live(_))
        )
    })
}

/// The endpoint of every pane and popup terminal in a workspace's JSON.
fn terminal_endpoints(workspace: &serde_json::Value) -> Vec<String> {
    workspace["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|tab| {
            tab["panes"]
                .as_array()
                .unwrap()
                .iter()
                .chain(tab["popups"].as_array().unwrap())
        })
        .map(|terminal| terminal["endpoint"].as_str().unwrap().to_owned())
        .collect()
}

/// The live Orbit identity of the terminal at each of `endpoints`, read
/// before a close so its process can be checked afterwards.
fn live_terminals(endpoints: &[String]) -> Vec<(String, LiveIdentity)> {
    endpoints
        .iter()
        .map(|endpoint| (endpoint.clone(), live_identity(Path::new(endpoint))))
        .collect()
}

/// Asserts that of `terminals` exactly `ended` stopped: their processes
/// exit and their records stop being live, while every other terminal's
/// process (by start identity) and record stay live.
fn assert_only_ended(terminals: &[(String, LiveIdentity)], ended: &[String]) {
    for (endpoint, identity) in terminals {
        if ended.contains(endpoint) {
            wait_for_process_removal(identity.process_id, &format!("{endpoint} kept running"));
            assert!(
                !terminal_is_live(endpoint),
                "{endpoint} stayed recorded live"
            );
        }
    }
    for (endpoint, identity) in terminals {
        if !ended.contains(endpoint) {
            assert!(
                process::exists(identity.process_id)
                    && process::start_identity(identity.process_id) == identity.process_start,
                "{endpoint} stopped running"
            );
            assert!(
                terminal_is_live(endpoint),
                "{endpoint} stopped being recorded live"
            );
        }
    }
}

/// The workspace JSON without the caller, labels, the window's running and
/// busy terminals and quota reads, which change on their own as terminals run
/// or end elsewhere in the window.
fn presented(output: &Output) -> serde_json::Value {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let mut value = stable_workspace_json(output);
    let object = value.as_object_mut().unwrap();
    for field in [
        "running",
        "busy",
        "codex_quota",
        "claude_quota",
        "codex_quota_failure",
        "claude_quota_failure",
    ] {
        object.remove(field);
    }
    value
}

/// Asserts that `after` is `before` without the tab or pane `target`, with
/// the same shown workspace, active tab and every other tab unchanged.
fn assert_closed_only(before: &serde_json::Value, after: &serde_json::Value, target: &str) {
    for field in ["session", "active_workspace", "active_tab", "entries"] {
        assert_eq!(after[field], before[field], "{field}");
    }
    let tabs = |value: &serde_json::Value| value["tabs"].as_array().unwrap().clone();
    let mut expected = tabs(before);
    expected.retain(|tab| tab["id"] != target);
    let after_tabs = tabs(after);
    assert_eq!(after_tabs.len(), expected.len(), "{after}");
    for (after, mut expected) in after_tabs.into_iter().zip(expected) {
        let panes = expected["panes"].as_array_mut().unwrap();
        if panes.iter().any(|pane| pane["id"] == target) {
            panes.retain(|pane| pane["id"] != target);
            let ids = |tab: &serde_json::Value| {
                tab["panes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|pane| pane["id"].clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(ids(&after), ids(&expected));
            for field in ["id", "selected_pane", "popups", "selected_popup"] {
                assert_eq!(after[field], expected[field], "{field}");
            }
        } else {
            assert_eq!(after, expected);
        }
    }
}

#[test]
fn cli_closes_background_ids_and_preserves_the_presented_workspace() {
    let (root, mut default) = quit_workspace();
    let control = generation_runtime(&root.join("r")).join("eon.sock");
    let directory = root.to_str().unwrap().to_owned();
    let run = |caller: Option<&[u8]>, arguments: &[&str]| {
        let mut command = quit_command(&root);
        if let Some(caller) = caller {
            command.env("EON_TERMINAL", OsStr::from_bytes(caller));
        }
        command
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let parsed = |output: &Output| -> serde_json::Value {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let inspect = || run(None, &["session", "--json"]);

    // The caller is t1's Agent popup. The active t2 shows its own Agent
    // popup, and the background t3 and t4 each hold two panes.
    let caller = quit_terminal_caller(Path::new(&show_agent(&control, "t1-agent", "t1")));
    // Each creation names its tab and the pane it started in `created`,
    // focused or not, and wherever the calling terminal is.
    let names = |reply: &serde_json::Value, tab: &serde_json::Value, pane: &serde_json::Value| {
        assert_eq!(
            reply["created"],
            serde_json::json!({"tab": tab, "pane": pane}),
            "{reply}"
        );
    };
    let focused = parsed(&run(
        Some(&caller),
        &["tab", "create", "--dir", &directory, "--json"],
    ));
    let tab = focused["tabs"].as_array().unwrap().last().unwrap();
    assert_eq!(tab["id"], "t2");
    names(&focused, &tab["id"], &tab["panes"][0]["id"]);
    show_agent(&control, "t2-agent", "t2");
    let mut background = Vec::new();
    for _ in 0..2 {
        let created = parsed(&run(
            Some(&caller),
            &["tab", "create", "--no-focus", "--dir", &directory, "--json"],
        ));
        let tab = created["tabs"].as_array().unwrap().last().unwrap();
        names(&created, &tab["id"], &tab["panes"][0]["id"]);
        let first = quit_terminal_caller(Path::new(tab["panes"][0]["endpoint"].as_str().unwrap()));
        let pane = parsed(&run(
            Some(&first),
            &["pane", "create", "--no-focus", "--json"],
        ));
        let panes = pane["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == tab["id"])
            .unwrap()["panes"]
            .clone();
        assert_eq!(panes.as_array().unwrap().len(), 2);
        names(&pane, &tab["id"], &panes[1]["id"]);
        background.push(tab["id"].as_str().unwrap().to_owned());
    }
    let before = parsed(&inspect());
    assert!(before.get("created").is_none(), "{before}");
    assert_eq!(before["active_tab"], "t2");
    assert!(before["tabs"][0]["selected_popup"].is_string());
    assert!(before["tabs"][1]["selected_popup"].is_string());
    let tab = |value: &serde_json::Value, id: &str| {
        value["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["id"] == id)
            .unwrap()
            .clone()
    };
    let second_pane = |id: &str| tab(&before, id)["panes"][1].clone();

    // Each close names its target by ID: a pane from another tab's popup, a
    // pane without a caller, then each tab in the same two ways. Only the
    // target's terminals end.
    let (t3, t4) = (background[0].as_str(), background[1].as_str());
    let (p3b, p4b) = (second_pane(t3), second_pane(t4));
    let endpoint = |terminal: &serde_json::Value| terminal["endpoint"].as_str().unwrap().to_owned();
    for (caller, kind, target, ended, json) in [
        (
            Some(&caller[..]),
            "pane",
            p3b["id"].as_str().unwrap(),
            vec![endpoint(&p3b)],
            true,
        ),
        (
            None,
            "pane",
            p4b["id"].as_str().unwrap(),
            vec![endpoint(&p4b)],
            false,
        ),
        (
            Some(&caller[..]),
            "tab",
            t3,
            terminal_endpoints(&serde_json::json!({ "tabs": [tab(&before, t3)] })),
            true,
        ),
        (
            None,
            "tab",
            t4,
            terminal_endpoints(&serde_json::json!({ "tabs": [tab(&before, t4)] })),
            false,
        ),
    ] {
        let state = presented(&inspect());
        let endpoints = live_terminals(&terminal_endpoints(&parsed(&inspect())));
        let mut arguments = vec![kind, "close", target, "--yes"];
        if json {
            arguments.push("--json");
        }
        let closed = run(caller, &arguments);
        assert_eq!(closed.status.code(), Some(0), "{closed:?}");
        if json {
            assert_eq!(parsed(&closed)["active_tab"], "t2");
        }
        assert_closed_only(&state, &presented(&inspect()), target);
        assert_only_ended(&endpoints, &ended);
    }

    // A stale ID changes and stops nothing, as JSON or for a person.
    let state = presented(&inspect());
    let endpoints = live_terminals(&terminal_endpoints(&parsed(&inspect())));
    let gone = run(Some(&caller), &["tab", "close", t3, "--yes", "--json"]);
    assert_eq!(gone.status.code(), Some(3), "{gone:?}");
    assert_eq!(
        stdout(&gone),
        format!("{{\"error\":{{\"code\":\"unknown-tab\",\"detail\":\"tab {t3} is not live\"}}}}\n")
    );
    let pane = p3b["id"].as_str().unwrap();
    let gone = run(None, &["pane", "close", pane]);
    assert_eq!(gone.status.code(), Some(3), "{gone:?}");
    assert_eq!(
        String::from_utf8_lossy(&gone.stderr),
        format!("error unknown-pane: pane {pane} is not live\n")
    );
    assert_eq!(presented(&inspect()), state);
    assert_only_ended(&endpoints, &[]);

    // A replayed close is a duplicate and stops nothing more.
    let created = parsed(&run(
        Some(&caller),
        &["tab", "create", "--no-focus", "--dir", &directory, "--json"],
    ));
    let t5 = created["tabs"].as_array().unwrap().last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let close_t5 = || {
        workspace_action(
            &control,
            "replayed-close",
            Action::CloseTab { tab: t5.clone() },
        )
    };
    assert!(matches!(close_t5(), Response::Snapshot(_)));
    let endpoints = live_terminals(&terminal_endpoints(&parsed(&inspect())));
    assert!(matches!(
        close_t5(),
        Response::Failure(failure) if failure.code == "duplicate-request"
    ));
    assert_eq!(presented(&inspect()), state);
    assert_only_ended(&endpoints, &[]);

    let stopped = run(None, &["window", "stop", "default", "--json"]);
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_column_close_needs_yes_and_binds_consent_to_runtimes_answer() {
    let (root, mut default) = quit_workspace();
    fs::write(root.join("foreground"), "child").unwrap();
    let runtime = root.join("r");
    let caller = quit_caller(&runtime);
    let run = |args: &[&str]| {
        let mut command = quit_command(&root);
        command
            .env("EON_TERMINAL", OsStr::from_bytes(&caller))
            .args(args);
        command
    };
    let inspect = || run(&["session", "--json"]).output().unwrap();
    let parsed =
        |output: &Output| serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    let directory = root.to_str().unwrap();
    let malformed = run(&["column", "close", "", "--json"]).output().unwrap();
    assert_eq!(malformed.status.code(), Some(2), "{malformed:?}");
    assert_eq!(parsed(&malformed)["error"]["code"], "malformed-action");
    // An idle background one-pane Stack still needs --yes. Its refusal preserves focus.
    let background = run(&["tab", "create", "--no-focus", "--dir", directory, "--json"])
        .output()
        .unwrap();
    assert!(background.status.success(), "{background:?}");
    let background = parsed(&background);
    let tab = background["tabs"].as_array().unwrap().last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let pane = background["tabs"].as_array().unwrap().last().unwrap()["panes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let endpoint = background["tabs"].as_array().unwrap().last().unwrap()["panes"][0]["endpoint"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_for_busy_panes(&root, None, &[]);
    let before = presented(&inspect());
    let refused = quit_with_answers(&mut run(&["column", "close", &pane]), b"y\n");
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let text = String::from_utf8_lossy(&refused.stderr);
    assert!(text.contains("would stop 1 terminal:"), "{text}");
    assert!(text.contains("Nothing was closed."), "{text}");
    assert!(text.contains("error not-confirmed:"), "{text}");
    assert!(!text.contains("[y/N]"), "{text}");
    assert_eq!(presented(&inspect()), before);
    // Input queued at a real terminal is equally unable to authorize a close.
    let (mut leader, device) = pseudo_terminal();
    leader.write_all(b"y\n").unwrap();
    let refused = run(&["column", "close", &pane, "--json"])
        .stdin(Stdio::from(device))
        .output()
        .unwrap();
    drop(leader);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let value = parsed(&refused);
    assert_eq!(value["error"]["code"], "not-confirmed");
    assert_eq!(value["close"]["end"], "column");
    assert_eq!(value["close"]["pane"], pane);
    assert_eq!(value["close"]["terminals"][0]["busy"], false);
    assert!(!value["close"]["generation"].as_str().unwrap().is_empty());
    assert!(
        !value["close"]["terminals"][0]["run"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(refused.stderr.is_empty());
    assert_eq!(presented(&inspect()), before);
    // Work begins in the listed run before a later --yes invocation.
    fs::write(root.join("foreground"), "other").unwrap();
    wait_for_busy_panes(&root, None, &["p1", &pane]);
    let endpoints = live_terminals(&terminal_endpoints(&parsed(&inspect())));
    let closed = run(&["column", "close", &pane, "--yes", "--json"])
        .output()
        .unwrap();
    assert!(closed.status.success(), "{closed:?}");
    assert_only_ended(&endpoints, &[endpoint]);
    assert_closed_only(&before, &presented(&inspect()), &tab);
    // A command pane's running mark comes from Runtime; the same consent path closes it.
    let background = run(&[
        "tab",
        "create",
        "--no-focus",
        "--dir",
        directory,
        "--json",
        "--",
        "sleep",
        "300",
    ])
    .output()
    .unwrap();
    assert!(background.status.success(), "{background:?}");
    let background = parsed(&background);
    let pane = background["tabs"].as_array().unwrap().last().unwrap()["panes"][0]["id"]
        .as_str()
        .unwrap();
    let refused = run(&["column", "close", pane, "--json"]).output().unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert_eq!(parsed(&refused)["close"]["terminals"][0]["busy"], true);
    let closed = run(&["column", "close", pane, "--yes", "--json"])
        .output()
        .unwrap();
    assert!(closed.status.success(), "{closed:?}");
    let stopped = run(&["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_busy_background_close_confirms_refuses_and_cancels_without_focus() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let control = generation_runtime(&runtime).join("eon.sock");
    // Every default shell runs another program, so every terminal is busy.
    fs::write(root.join("foreground"), "other").unwrap();
    let caller = quit_caller(&runtime);
    let run_caller = |args: &[&str]| {
        let mut cmd = quit_command(&root);
        cmd.env("EON_TERMINAL", OsStr::from_bytes(&caller))
            .args(args);
        cmd
    };
    let inspect = || run_caller(&["session", "--json"]).output().unwrap();
    let not_confirmed = |kind: &str, target: &str| {
        format!(
            "{{\"error\":{{\"code\":\"not-confirmed\",\"detail\":\"close of {kind} {target} requires confirmation; rerun with --yes\"}}}}\n"
        )
    };
    let create_background = || {
        let created = run_caller(&[
            "tab",
            "create",
            "--no-focus",
            "--dir",
            root.to_str().unwrap(),
            "--json",
            "--",
            "sleep",
            "300",
        ])
        .output()
        .unwrap();
        assert!(created.status.success(), "{created:?}");
        let snap: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
        let tab = snap["tabs"].as_array().unwrap().last().unwrap();
        (
            tab["id"].as_str().unwrap().to_owned(),
            tab["panes"][0]["id"].as_str().unwrap().to_owned(),
        )
    };

    // The active t2 shows its Agent popup throughout.
    let created_t2 = run_caller(&["tab", "create", "--dir", root.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(created_t2.status.success(), "{created_t2:?}");
    show_agent(&control, "t2-agent", "t2");

    for kind in ["tab", "pane"] {
        let (tab_id, pane_id) = create_background();
        let target = if kind == "tab" { &tab_id } else { &pane_id };
        let before_output = wait_for_settled_workspace(&root, &["p1", "p2", "popup", &pane_id]);
        let before_val: serde_json::Value = serde_json::from_slice(&before_output.stdout).unwrap();
        let pane_label =
            before_val["tabs"].as_array().unwrap().last().unwrap()["panes"][0]["label"]
                .as_str()
                .unwrap()
                .to_owned();
        let before = presented(&before_output);
        let question = format!(
            "Close {kind} {target}? This stops 1 terminal; 1 still running: {pane_label}. [y/N] "
        );

        // JSON without --yes, even with piped "y\n", refuses with not-confirmed/1 without asking.
        let json_refused =
            quit_with_answers(&mut run_caller(&[kind, "close", target, "--json"]), b"y\n");
        assert_eq!(json_refused.status.code(), Some(1), "{json_refused:?}");
        assert_eq!(stdout(&json_refused), not_confirmed(kind, target));
        assert!(json_refused.stderr.is_empty(), "{json_refused:?}");
        assert_eq!(presented(&inspect()), before);

        // Human pipe "n\n" and empty input refuse with not-confirmed/1 without mutating.
        for answer in [b"n\n".as_slice(), b"".as_slice()] {
            let declined = quit_with_answers(&mut run_caller(&[kind, "close", target]), answer);
            assert_eq!(declined.status.code(), Some(1), "{declined:?}");
            assert!(declined.stdout.is_empty(), "{declined:?}");
            assert_eq!(
                String::from_utf8_lossy(&declined.stderr),
                format!(
                    "{question}error not-confirmed: {kind} {target} stays open; no yes was read and stdin is not a terminal\n"
                )
            );
            assert_eq!(presented(&inspect()), before);
        }

        // A "n" at a terminal cancels with exit 0 and the exact cancel note on stdout.
        let cancelled = at_terminal(
            &mut run_caller(&[kind, "close", target]),
            Some(b"n\n"),
            || (),
        );
        assert_eq!(cancelled.status.code(), Some(0), "{cancelled:?}");
        assert_eq!(
            stdout(&cancelled),
            format!("cancelled; {kind} {target} stays open\n")
        );
        assert_eq!(String::from_utf8_lossy(&cancelled.stderr), question);
        assert_eq!(presented(&inspect()), before);

        // A piped "y\n" confirms and closes only the target; t2 stays active with its popup.
        let confirmed = quit_with_answers(&mut run_caller(&[kind, "close", target]), b"y\n");
        assert_eq!(confirmed.status.code(), Some(0), "{confirmed:?}");
        assert_closed_only(&before, &presented(&inspect()), &tab_id);

        // --yes --json closes a busy background target without reading stdin.
        let (yes_tab, yes_pane) = create_background();
        let yes_target = if kind == "tab" { &yes_tab } else { &yes_pane };
        let before = presented(&wait_for_settled_workspace(
            &root,
            &["p1", "p2", "popup", &yes_pane],
        ));
        let closed_yes = run_caller(&[kind, "close", yes_target, "--yes", "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(closed_yes.status.code(), Some(0), "{closed_yes:?}");
        assert_closed_only(&before, &presented(&inspect()), &yes_tab);
    }

    // A target that ends while its question waits is not live when the yes
    // arrives (exit 3), and nothing else closes.
    let (vanish_tab, vanish_pane) = create_background();
    let before = presented(&wait_for_settled_workspace(
        &root,
        &["p1", "p2", "popup", &vanish_pane],
    ));
    let vanished = at_terminal(
        &mut run_caller(&["tab", "close", &vanish_tab]),
        Some(b"y\n"),
        || {
            let concurrent = bounded_output(&mut run_caller(&[
                "tab",
                "close",
                &vanish_tab,
                "--yes",
                "--json",
            ]));
            assert!(concurrent.status.success(), "{concurrent:?}");
        },
    );
    assert_eq!(vanished.status.code(), Some(3), "{vanished:?}");
    assert!(
        String::from_utf8_lossy(&vanished.stderr).ends_with(&format!(
            "error unknown-tab: tab {vanish_tab} is not live\n"
        )),
        "{vanished:?}"
    );
    assert_closed_only(&before, &presented(&inspect()), &vanish_tab);

    // A busy final close names its consequence, with the warning on or off,
    // at a terminal and through a pipe, and JSON refuses it without --yes.
    assert!(
        run_caller(&["tab", "close", "t2", "--yes", "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap()
            .status
            .success()
    );
    let sole_out = wait_for_settled_workspace(&root, &["p1"]);
    let sole_snap: serde_json::Value = serde_json::from_slice(&sole_out.stdout).unwrap();
    let sole = presented(&sole_out);
    let p1_label = sole_snap["tabs"][0]["panes"][0]["label"]
        .as_str()
        .unwrap()
        .to_owned();
    let busy_final = format!(
        "Close the last tab? This quits the window and stops 1 terminal [{p1_label}]; 1 still running: {p1_label}. [y/N] "
    );
    let configure = |warning: bool| {
        fs::write(
            root.join("config/config.toml"),
            format!(
                "[terminal]\nfont_fallbacks = []\n[anima]\nenabled = false\n[session]\nwarning_close_last_tab = {warning}\n"
            ),
        )
        .unwrap();
    };
    for warning in [true, false] {
        configure(warning);
        for (kind, target) in [("tab", "t1"), ("pane", "p1")] {
            let tty = at_terminal(
                &mut run_caller(&[kind, "close", target]),
                Some(b"n\n"),
                || (),
            );
            assert_eq!(tty.status.code(), Some(0), "{tty:?}");
            assert_eq!(stdout(&tty), "cancelled; the window stays open\n");
            assert_eq!(String::from_utf8_lossy(&tty.stderr), busy_final);
            let pipe = quit_with_answers(&mut run_caller(&[kind, "close", target]), b"n\n");
            assert_eq!(pipe.status.code(), Some(1), "{pipe:?}");
            assert_eq!(
                String::from_utf8_lossy(&pipe.stderr),
                format!(
                    "{busy_final}error not-confirmed: the window stays open; no yes was read and stdin is not a terminal\n"
                )
            );
            let json = run_caller(&[kind, "close", target, "--json"])
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert_eq!(json.status.code(), Some(1), "{json:?}");
            assert_eq!(stdout(&json), not_confirmed(kind, target));
            assert_eq!(presented(&inspect()), sole);
        }
    }

    // An idle final close asks only a person at a terminal with the warning
    // on and no --yes. Each case closes a fresh shown workspace, whose final
    // close removes it while t1's workspace remains.
    fs::write(root.join("foreground"), "child").unwrap();
    for (warning, mode) in [
        (true, "terminal"),
        (true, "terminal --yes"),
        (true, "pipe"),
        (true, "json"),
        (false, "terminal"),
        (false, "pipe"),
        (false, "json"),
    ] {
        configure(warning);
        let created = quit_command(&root)
            .args(["session", "new", "--json"])
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
        let shown: serde_json::Value =
            serde_json::from_slice(&wait_for_settled_workspace(&root, &[]).stdout).unwrap();
        let workspace = shown["session"].as_str().unwrap().to_owned();
        let tab = shown["tabs"][0]["id"].as_str().unwrap().to_owned();
        let label = shown["tabs"][0]["panes"][0]["label"].as_str().unwrap();
        let close = |extra: &[&str]| {
            let mut command = quit_command(&root);
            command.args(["tab", "close", &tab]).args(extra);
            command
        };
        let closed = match mode {
            "terminal" if warning => {
                let question = format!(
                    "Close the last tab? This closes session {workspace} and stops 1 terminal [{label}]. [y/N] "
                );
                let cancelled = at_terminal(&mut close(&[]), Some(b"n\n"), || ());
                assert_eq!(cancelled.status.code(), Some(0), "{cancelled:?}");
                assert_eq!(
                    stdout(&cancelled),
                    format!("cancelled; session {workspace} stays open\n")
                );
                assert_eq!(String::from_utf8_lossy(&cancelled.stderr), question);
                let confirmed = at_terminal(&mut close(&[]), Some(b"y\n"), || ());
                assert_eq!(String::from_utf8_lossy(&confirmed.stderr), question);
                confirmed
            }
            "terminal" => at_terminal(&mut close(&[]), None, || ()),
            "terminal --yes" => at_terminal(&mut close(&["--yes"]), None, || ()),
            "pipe" => close(&[]).stdin(Stdio::null()).output().unwrap(),
            _ => close(&["--json"]).stdin(Stdio::null()).output().unwrap(),
        };
        assert_eq!(
            closed.status.code(),
            Some(0),
            "{warning} {mode}: {closed:?}"
        );
        if mode != "terminal" || !warning {
            assert!(closed.stderr.is_empty(), "{warning} {mode}: {closed:?}");
        }
        let listed = quit_command(&root)
            .args(["session", "ls", "--json"])
            .output()
            .unwrap();
        assert!(
            !stdout(&listed).contains(&format!("\"id\":\"{workspace}\"")),
            "{listed:?}"
        );
    }

    // The idle final close of the last workspace ends the window unasked through a pipe.
    configure(true);
    wait_for_settled_workspace(&root, &[]);
    let idle_final_pipe = run_caller(&["tab", "close", "t1"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        idle_final_pipe.status.code(),
        Some(0),
        "{idle_final_pipe:?}"
    );
    assert!(idle_final_pipe.stderr.is_empty(), "{idle_final_pipe:?}");
    assert_eq!(stdout(&idle_final_pipe), "Eon window default closed\n");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_idle_background_close_ignores_other_busy_tabs() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let control = generation_runtime(&runtime).join("eon.sock");
    // Default shells are idle; a command terminal and Agent stay busy.
    fs::write(root.join("foreground"), "child").unwrap();
    let caller = quit_caller(&runtime);
    let run = |caller: &[u8], args: &[&str]| {
        quit_command(&root)
            .env("EON_TERMINAL", OsStr::from_bytes(caller))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let parsed = |output: &Output| -> serde_json::Value {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let directory = root.to_str().unwrap();
    let busy = parsed(&run(
        &caller,
        &[
            "tab",
            "create",
            "--no-focus",
            "--dir",
            directory,
            "--json",
            "--",
            "sleep",
            "300",
        ],
    ));
    let busy_pane = busy["tabs"][1]["panes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let idle = parsed(&run(
        &caller,
        &["tab", "create", "--no-focus", "--dir", directory, "--json"],
    ));
    let idle_tab = idle["tabs"][2]["id"].as_str().unwrap().to_owned();
    let first = quit_terminal_caller(Path::new(
        idle["tabs"][2]["panes"][0]["endpoint"].as_str().unwrap(),
    ));
    let idle = parsed(&run(&first, &["pane", "create", "--no-focus", "--json"]));
    let idle_pane = idle["tabs"][2]["panes"][1].clone();
    show_agent(&control, "t1-agent", "t1");

    // With a complete busy list naming only other tabs' terminals, the idle
    // pane closes through JSON without --yes, and then its idle tab for a
    // person without a terminal, neither asking anything.
    for (args, target, ended) in [
        (
            vec!["pane", "close", idle_pane["id"].as_str().unwrap(), "--json"],
            idle_pane["id"].as_str().unwrap().to_owned(),
            vec![idle_pane["endpoint"].as_str().unwrap().to_owned()],
        ),
        (
            vec!["tab", "close", &idle_tab],
            idle_tab.clone(),
            vec![
                idle["tabs"][2]["panes"][0]["endpoint"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            ],
        ),
    ] {
        let settled = wait_for_settled_workspace(&root, &["popup", &busy_pane]);
        let before = presented(&settled);
        assert!(before["tabs"][0]["selected_popup"].is_string());
        let endpoints = live_terminals(&terminal_endpoints(&parsed(&settled)));
        let closed = run(&caller, &args);
        assert_eq!(closed.status.code(), Some(0), "{closed:?}");
        assert!(closed.stderr.is_empty(), "{closed:?}");
        let after = presented(&run(&caller, &["session", "--json"]));
        assert_closed_only(&before, &after, &target);
        assert_only_ended(&endpoints, &ended);
    }

    let stopped = quit_command(&root)
        .args(["window", "stop", "default", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    wait_for_successful_exit(&mut default.child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_close_preserves_workspace_scope_and_final_close_results() {
    let (root, mut default) = quit_workspace();
    let runtime = root.join("r");
    let generation = generation_runtime(&runtime);
    fs::write(root.join("foreground"), "child").unwrap();
    let w1_caller = quit_caller(&runtime);
    let run_from = |caller_bytes: Option<&[u8]>, args: &[&str]| {
        let mut cmd = quit_command(&root);
        if let Some(caller) = caller_bytes {
            cmd.env("EON_TERMINAL", OsStr::from_bytes(caller));
        }
        cmd.args(args).stdin(Stdio::null()).output().unwrap()
    };
    let parsed = |output: &Output| -> serde_json::Value {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let shown_workspace = || {
        let val = parsed(&run_from(None, &["session", "ls", "--json"]));
        let shown = val["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["shown"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let count = val["sessions"].as_array().unwrap().len();
        (shown, count)
    };

    // The hidden s2 gets a second pane and a second tab; s1 stays shown.
    parsed(&run_from(
        Some(&w1_caller),
        &["session", "new", "--no-focus", "--name", "notes", "--json"],
    ));
    let w2_caller = quit_terminal_caller(&generation.join("terminal-2.sock"));
    parsed(&run_from(
        Some(&w2_caller),
        &["pane", "create", "--no-focus", "--json"],
    ));
    let w2_snap = parsed(&run_from(
        Some(&w2_caller),
        &[
            "tab",
            "create",
            "--no-focus",
            "--dir",
            root.to_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(w2_snap["session"], "s2");
    assert_eq!(shown_workspace(), ("s1".to_owned(), 2));
    let w2_t2 = w2_snap["tabs"][1]["id"].as_str().unwrap().to_owned();
    let w2_p1 = w2_snap["tabs"][0]["panes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let w2_p2 = w2_snap["tabs"][0]["panes"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_for_busy_panes(&root, None, &[]);
    // The shown s1 without the session list, whose tab counts change.
    let shown_s1 = || {
        let mut value = presented(&run_from(None, &["session", "--json"]));
        value.as_object_mut().unwrap().remove("sessions");
        value
    };
    let s1 = shown_s1();
    let w1_endpoints = terminal_endpoints(&parsed(&run_from(None, &["session", "--json"])));
    let w2_endpoints = terminal_endpoints(&w2_snap);
    let w1_terminals = live_terminals(&w1_endpoints);
    let w2_terminals = live_terminals(&w2_endpoints);
    let w1_w2 = [w1_terminals.clone(), w2_terminals].concat();

    // Neither s1's caller nor a caller-less command reaches s2's IDs, nor
    // s2's caller s1's: each exits 3 and changes nothing.
    let refused = |caller: Option<&[u8]>, kind: &str, target: &str, code: &str| {
        let foreign = run_from(caller, &[kind, "close", target, "--yes", "--json"]);
        assert_eq!(foreign.status.code(), Some(3), "{foreign:?}");
        assert_eq!(
            stdout(&foreign),
            format!(
                "{{\"error\":{{\"code\":\"{code}\",\"detail\":\"{kind} {target} is not live\"}}}}\n"
            )
        );
    };
    for caller in [Some(&w1_caller[..]), None] {
        refused(caller, "tab", &w2_t2, "unknown-tab");
        refused(caller, "pane", &w2_p2, "unknown-pane");
    }
    refused(Some(&w2_caller), "tab", "t1", "unknown-tab");
    refused(Some(&w2_caller), "pane", "p1", "unknown-pane");
    assert_eq!(shown_s1(), s1);
    assert_only_ended(&w1_w2, &[]);

    // s2's caller closes its pane and its background tab without showing s2;
    // the replies project s2.
    for (kind, target) in [("pane", &w2_p2), ("tab", &w2_t2)] {
        let closed = parsed(&run_from(
            Some(&w2_caller),
            &[kind, "close", target, "--yes", "--json"],
        ));
        assert_eq!(closed["session"], "s2");
        assert_eq!(shown_workspace(), ("s1".to_owned(), 2));
        assert_eq!(shown_s1(), s1);
    }
    let w2_left = parsed(&run_from(Some(&w2_caller), &["session", "--json"]));
    assert_eq!(w2_left["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(w2_left["tabs"][0]["panes"].as_array().unwrap().len(), 1);
    assert_only_ended(&w1_w2, &w2_endpoints[1..]);

    // Closing the hidden s2's final tab's last pane removes only s2; the
    // reply projects the shown s1, which is unchanged.
    let removed = parsed(&run_from(
        Some(&w2_caller),
        &["pane", "close", &w2_p1, "--yes", "--json"],
    ));
    assert_eq!(removed["session"], "s1");
    assert_eq!(shown_workspace(), ("s1".to_owned(), 1));
    assert_eq!(shown_s1(), s1);
    assert_only_ended(&w1_w2, &w2_endpoints);

    // So does closing a shown s3's final tab by its ID; s1 is shown again.
    parsed(&run_from(None, &["session", "new", "--json"]));
    let s3 = parsed(&run_from(None, &["session", "--json"]));
    assert_eq!(s3["session"], "s3");
    let s3_tab = s3["tabs"][0]["id"].as_str().unwrap().to_owned();
    let s3_endpoints = terminal_endpoints(&s3);
    let w1_w3 = [w1_terminals.clone(), live_terminals(&s3_endpoints)].concat();
    let removed = parsed(&run_from(
        None,
        &["tab", "close", &s3_tab, "--yes", "--json"],
    ));
    assert_eq!(removed["session"], "s1");
    assert_eq!(shown_workspace(), ("s1".to_owned(), 1));
    assert_only_ended(&w1_w3, &s3_endpoints);

    // The last workspace's last pane ends the window with the existing Ended
    // result, and with it the supervisor and its terminals.
    let ended = run_from(
        Some(&w1_caller),
        &["pane", "close", "p1", "--yes", "--json"],
    );
    assert_eq!(ended.status.code(), Some(0), "{ended:?}");
    let generation_id = generation.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        stdout(&ended),
        format!(
            "{{\"ended\":{{\"generation\":\"{generation_id}\"}},\"generation\":\"{generation_id}\",\"window\":null}}\n"
        )
    );
    wait_for_successful_exit(&mut default.child);
    assert_only_ended(&w1_terminals, &w1_endpoints);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_confirmed_close_inspects_first_and_a_failed_inspection_closes_nothing() {
    let root = temporary_directory();
    let runtime = root.join("runtime");
    let config = root.join("config");
    configure_mock_desktop(&config);
    let id = format!("g1-{:032x}", 12);
    let listener = generation_listener(&runtime, &id);
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(runtime.join(format!("supervisor-{id}.lock")))
        .unwrap();
    held.lock().unwrap();
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let received = Arc::new(Mutex::new(Vec::new()));
    let (server_done, server_received, server_id) = (done.clone(), received.clone(), id.clone());
    // A supervisor that answers lifecycle inspection but refuses to inspect
    // its workspace, recording every other request.
    let server = thread::spawn(move || {
        while !server_done.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(peer) => peer,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            let action = decode_request(&read_request(&mut stream)).unwrap().action;
            let response = match &action {
                ProtocolAction::Workspace(Action::InspectPresentation | Action::InspectRuntime) => {
                    encode_lifecycle_response(&LifecycleResponse::Runtime(Runtime {
                        generation: server_id.clone(),
                        name: None,
                        started_at: 0,
                        eon_version: "0.1.0".into(),
                        workspace_protocol: VERSION,
                        component_report: "test components".into(),
                        sessions: vec!["terminal-1".into()],
                        attach: Availability {
                            available: true,
                            reason: "test presentation".into(),
                        },
                        stop: Availability {
                            available: true,
                            reason: "supervisor owns these Sessions".into(),
                        },
                        busy: None,
                    }))
                    .unwrap()
                }
                _ => {
                    server_received.lock().unwrap().push(action);
                    encode_response(&Response::Failure(ProtocolFailure {
                        code: "unavailable".into(),
                        detail: "test supervisor".into(),
                    }))
                    .unwrap()
                }
            };
            stream.write_all(&response).unwrap();
        }
    });

    // --yes skips the question, never the inspection that precedes it.
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_eon"));
    for (arguments, close) in [
        (
            ["tab", "close", "t1"],
            ProtocolAction::Workspace(Action::CloseTab { tab: "t1".into() }),
        ),
        (
            ["pane", "close", "p1"],
            ProtocolAction::ClosePane { pane: "p1".into() },
        ),
    ] {
        received.lock().unwrap().clear();
        let closed = invoke(
            &binary,
            &runtime,
            &config,
            &[&arguments[..], &["--window", &id[3..], "--yes", "--json"]].concat(),
        );
        assert_eq!(closed.status.code(), Some(1), "{closed:?}");
        assert_eq!(
            stdout(&closed),
            "{\"error\":{\"code\":\"unavailable\",\"detail\":\"test supervisor\"}}\n"
        );
        let received = received.lock().unwrap();
        assert_eq!(
            *received,
            [ProtocolAction::Workspace(Action::Inspect)],
            "{close:?}"
        );
    }
    done.store(true, Ordering::SeqCst);
    server.join().unwrap();
    drop(held);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn update_direct_checks_and_installs_only_through_the_owned_payload_helper() {
    let root = temporary_directory();
    let binary = Path::new(env!("CARGO_BIN_EXE_eon"));
    let versions = Command::new(binary).arg("versions").output().unwrap();
    let text = stdout(&versions);
    let identity: Vec<_> = text.lines().next().unwrap().split_whitespace().collect();
    let (version, build) = (identity[1], identity[2]);
    let hash = "a".repeat(64);
    let prefix = root.join("managed prefix ' with spaces");
    let payload = prefix.join("versions").join(&hash);
    let package = payload.join("nix/store/product");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::create_dir(package.join("share")).unwrap();
    fs::create_dir(prefix.join("bin")).unwrap();
    for path in [
        &prefix,
        &prefix.join("versions"),
        &prefix.join("bin"),
        &payload,
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    executable(&package.join("bin/eon"), fs::read(binary).unwrap());
    fs::write(package.join("share/eon-source-receipt.json"), serde_json::json!({
        "schema_version":1,"repository":"https://github.com/Yazelix/eon","revision":"1".repeat(40),
        "build":build,"platform":"x86_64-linux","attribute":null
    }).to_string()).unwrap();
    fs::write(prefix.join(".eon-install"), "eon-install-v1\n").unwrap();
    let archive_url = format!(
        "https://github.com/Yazelix/eon/releases/download/v{version}/eon-{version}-x86_64-linux.tar.zst"
    );
    fs::write(
        payload.join(".eon-source"),
        format!("eon-install-v1\n{hash}\n{archive_url}\n"),
    )
    .unwrap();
    symlink("nix/store/product/bin/eon", payload.join("eon")).unwrap();
    symlink(format!("versions/{hash}"), prefix.join("current")).unwrap();
    symlink("../current/eon", prefix.join("bin/eon")).unwrap();
    let installer = package.join("bin/eon-install");
    executable(&installer, "#!/bin/sh\nexit 99\n");
    let metadata = root.join("releases.json");
    let asset = "eon-999.0.0-x86_64-linux.tar.zst";
    let assets: Vec<_> = [asset,"SHA256SUMS","SHA256SUMS.sig"].iter().map(|name| serde_json::json!({
        "name":name,"state":"uploaded","size":123,"digest":format!("sha256:{hash}"),
        "browser_download_url":format!("https://github.com/Yazelix/eon/releases/download/v999.0.0/{name}")
    })).collect();
    fs::write(
        &metadata,
        serde_json::json!([{"tag_name":"v999.0.0","draft":false,"prerelease":false,
        "published_at":"2026-11-08T12:00:00Z","assets":assets}])
        .to_string(),
    )
    .unwrap();
    let curl = root.join("curl-fixture");
    executable(
        &curl,
        "#!/bin/sh\nfor url; do :; done\ncase \"$url\" in\nhttps://api.github.com/repos/Yazelix/eon/releases\\?*) cat \"$EON_UPDATE_FIXTURE\"; printf '\\n200' ;;\n*) exit 99 ;;\nesac\n",
    );
    let run = |program: &Path, explicit: bool, helper: &Path, human: bool| {
        let mut command = eon_command(program);
        command.args(["update", "--check"]);
        if !human {
            command.arg("--json");
        }
        if explicit {
            command.arg("--prefix").arg(&prefix);
        }
        command
            .env("EON_CURL", &curl)
            .env("EON_UPDATE_FIXTURE", &metadata)
            .env("EON_INSTALLER", helper)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let before = fs::read_link(prefix.join("current")).unwrap();
    let output = run(&prefix.join("bin/eon"), false, &installer, false);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["target"], serde_json::json!({"prefix":prefix}));
    assert_eq!(report["install_kind"], "eon-install");
    assert_eq!(report["source"]["archive_url"], archive_url);
    assert_eq!(report["availability"]["state"], "available");
    assert_eq!(
        report["availability"]["candidate"]["archive_url"],
        format!("https://github.com/Yazelix/eon/releases/download/v999.0.0/{asset}")
    );
    assert_eq!(report["capability"]["install"], false);
    assert_eq!(report["capability"]["reason"], "trust-unconfigured");
    assert_eq!(report["selection"], serde_json::Value::Null);
    let output = run(binary, true, &installer, false);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["capability"]["reason"], "installer-unavailable",
        "an inherited helper cannot own a different running executable"
    );
    let output = run(&prefix.join("bin/eon"), false, binary, true);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("installer-unavailable"));
    assert!(stdout(&output).contains("verifies no signature or archive bytes"));
    assert_eq!(fs::read_link(prefix.join("current")).unwrap(), before);
    assert_eq!(fs::read_dir(prefix.join("versions")).unwrap().count(), 1);

    let config = root.join("direct-config");
    fs::create_dir(&config).unwrap();
    let signers = root.join("person's signers");
    fs::write(&signers, "independently provisioned fixture signer\n").unwrap();
    fs::write(
        config.join("config.toml"),
        format!(
            "[updates]\nallowed_signers = {:?}\n",
            signers.to_str().unwrap()
        ),
    )
    .unwrap();
    let args_file = root.join("installer-argv");
    let second_hash = "b".repeat(64);
    let second = prefix.join("versions").join(&second_hash);
    fs::create_dir_all(second.join("nix/store/product/bin")).unwrap();
    fs::create_dir(second.join("nix/store/product/share")).unwrap();
    fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(binary, second.join("nix/store/product/bin/eon")).unwrap();
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("share/eon-source-receipt.json")).unwrap())
            .unwrap();
    receipt["build"] = "g1-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    fs::write(
        second.join("nix/store/product/share/eon-source-receipt.json"),
        receipt.to_string(),
    )
    .unwrap();
    symlink("nix/store/product/bin/eon", second.join("eon")).unwrap();
    let second_url = format!("https://github.com/Yazelix/eon/releases/download/v999.0.0/{asset}");
    fs::write(
        second.join(".eon-source"),
        format!("eon-install-v1\n{second_hash}\n{second_url}\n"),
    )
    .unwrap();
    let mut metadata_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    metadata_value[0]["assets"][0]["digest"] = format!("sha256:{second_hash}").into();
    fs::write(&metadata, metadata_value.to_string()).unwrap();
    executable(
        &installer,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$EON_UPDATE_ARGS"
case "$EON_UPDATE_MODE" in
  select|selected-failure|selected-interrupt|unknown)
    ln -sfn "versions/$EON_UPDATE_HASH" "$EON_UPDATE_PREFIX/current" ;;
esac
case "$EON_UPDATE_MODE" in
  selected-failure) exit 1 ;;
  unknown) unlink "$EON_UPDATE_PREFIX/current" ;;
  selected-interrupt) kill -INT "$PPID"; sleep 30 ;;
  descendant) sleep 30 & echo $! > "$EON_UPDATE_PREFIX/descendant.pid"; exit 11 ;;
  fail-*) exit "${EON_UPDATE_MODE#fail-}" ;;
esac
"#,
    );
    let install = |selector: bool, yes: bool, mode: &str, configured: bool| {
        let mut command = eon_command(&package.join("bin/eon"));
        command.args(["update", "--install", "--json"]);
        if selector {
            command.arg("--prefix").arg(&prefix);
        }
        if yes {
            command.arg("--yes");
        }
        command
            .env("EON_CURL", &curl)
            .env("EON_UPDATE_FIXTURE", &metadata)
            .env("EON_INSTALLER", &installer)
            .env("EON_UPDATE_ARGS", &args_file)
            .env("EON_UPDATE_MODE", mode)
            .env("EON_UPDATE_HASH", &second_hash)
            .env("EON_UPDATE_PREFIX", &prefix);
        if configured {
            command.env("EON_CONFIG_HOME", &config);
        }
        command.stdin(Stdio::null()).output().unwrap()
    };
    for (selector, yes, configured, status, code) in [
        (false, true, true, 4, "install-ambiguous"),
        (true, true, false, 4, "trust-unconfigured"),
        (true, false, true, 1, "not-confirmed"),
    ] {
        let output = install(selector, yes, "select", configured);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["error"]["code"], code);
        assert_eq!(report["selection"]["state"], "unchanged");
        assert!(!args_file.exists());
    }
    let human_install = |answer: Option<&[u8]>| {
        let mut command = eon_command(&package.join("bin/eon"));
        command
            .args(["update", "--install", "--prefix"])
            .arg(&prefix)
            .env("EON_CURL", &curl)
            .env("EON_UPDATE_FIXTURE", &metadata)
            .env("EON_INSTALLER", &installer)
            .env("EON_UPDATE_ARGS", &args_file)
            .env("EON_UPDATE_MODE", "select")
            .env("EON_UPDATE_HASH", &second_hash)
            .env("EON_UPDATE_PREFIX", &prefix)
            .env("EON_CONFIG_HOME", &config);
        // Consent reads this private controlling terminal, never the test runner's.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        at_terminal(&mut command, answer, || {})
    };
    for answer in [b"n\n".as_slice(), b"\x04".as_slice()] {
        let output = human_install(Some(answer));
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(stdout(&output).contains("cancelled"), "{output:?}");
        assert!(!args_file.exists());
        assert_eq!(fs::read_link(prefix.join("current")).unwrap(), before);
    }
    let output = human_install(Some(b"y\n"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("installed"), "{output:?}");
    assert!(stdout(&output).contains(&second_hash), "{output:?}");
    fs::remove_file(&args_file).unwrap();
    fs::remove_file(prefix.join("current")).unwrap();
    symlink(format!("versions/{hash}"), prefix.join("current")).unwrap();
    // A current release needs no human question; unattended consent still applies.
    let newer_metadata = fs::read(&metadata).unwrap();
    let mut current_metadata: serde_json::Value = serde_json::from_slice(&newer_metadata).unwrap();
    current_metadata[0]["tag_name"] = format!("v{version}").into();
    for asset in current_metadata[0]["assets"].as_array_mut().unwrap() {
        let name = asset["name"].as_str().unwrap().replace("999.0.0", version);
        asset["name"] = name.clone().into();
        asset["browser_download_url"] =
            format!("https://github.com/Yazelix/eon/releases/download/v{version}/{name}").into();
        asset["digest"] = format!("sha256:{hash}").into();
    }
    fs::write(&metadata, current_metadata.to_string()).unwrap();
    let output = install(true, true, "select", true);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "unchanged");
    assert!(!args_file.exists());
    let output = human_install(None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(stdout(&output).contains("unchanged"), "{output:?}");
    assert!(!output.stderr.windows(6).any(|word| word == b"[y/N] "));
    assert!(!args_file.exists());
    let output = install(true, false, "select", true);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stdout(&output).contains("not-confirmed"));
    fs::write(&metadata, newer_metadata).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(prefix.join(".eon-update.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let output = install(true, true, "select", true);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "operation-busy");
    drop(lock);
    let original_curl = fs::read(&curl).unwrap();
    executable(
        &curl,
        "#!/bin/sh\nln -sfn \"versions/$EON_UPDATE_HASH\" \"$EON_UPDATE_PREFIX/current\"\ncat \"$EON_UPDATE_FIXTURE\"\nprintf '\\n200'\n",
    );
    let output = install(true, true, "select", true);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "target-changed");
    assert_eq!(report["selection"]["source"]["archive_sha256"], second_hash);
    assert!(!args_file.exists());
    fs::remove_file(prefix.join("current")).unwrap();
    symlink(format!("versions/{hash}"), prefix.join("current")).unwrap();
    executable(&curl, original_curl);
    for (mode, status, code, state) in [
        ("fail-10", 1, Some("verification-failed"), "unchanged"),
        ("fail-11", 1, Some("preflight-failed"), "unchanged"),
        ("descendant", 1, Some("preflight-failed"), "unchanged"),
        ("fail-12", 4, Some("operation-busy"), "unchanged"),
        ("fail-13", 1, Some("remote-unavailable"), "unchanged"),
        ("fail-14", 4, Some("trust-unconfigured"), "unchanged"),
        ("fail-1", 1, Some("selection-failed"), "unchanged"),
        ("selected-failure", 1, Some("selection-failed"), "changed"),
        ("selected-interrupt", 130, Some("interrupted"), "changed"),
        ("unknown", 1, Some("selection-failed"), "unknown"),
        ("select", 0, None, "changed"),
    ] {
        let output = install(true, true, mode, true);
        assert_eq!(output.status.code(), Some(status), "{mode}: {output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["selection"]["state"], state, "{mode}: {report}");
        if let Some(code) = code {
            assert_eq!(report["error"]["code"], code);
        } else {
            assert_eq!(report["outcome"], "installed");
        }
        assert_eq!(report["installed"]["build"], build);
        if mode == "descendant" {
            let pid = fs::read_to_string(prefix.join("descendant.pid")).unwrap();
            let executable = PathBuf::from(format!("/proc/{}/exe", pid.trim()));
            let deadline = Instant::now() + Duration::from_secs(2);
            while fs::read_link(&executable).is_ok() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(
                fs::read_link(executable).is_err(),
                "installer descendant survived"
            );
        }
        if state == "changed" {
            assert_eq!(report["selection"]["source"]["archive_sha256"], second_hash);
        }
        assert_eq!(
            fs::read_to_string(&args_file)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![
                "install",
                &second_url,
                "--signers",
                signers.to_str().unwrap(),
                "--prefix",
                prefix.to_str().unwrap()
            ]
        );
        if prefix.join("current").exists() {
            fs::remove_file(prefix.join("current")).unwrap();
        }
        symlink(format!("versions/{hash}"), prefix.join("current")).unwrap();
    }
    fs::write(
        config.join("config.toml"),
        format!(
            "[updates]\nbundle_prefix = {:?}\nallowed_signers = {:?}\n",
            prefix.to_str().unwrap(),
            signers.to_str().unwrap()
        ),
    )
    .unwrap();
    let output = install(false, true, "select", true);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "installed");
    assert!(payload.exists() && second.exists());
    fs::remove_dir_all(root).unwrap();
}
