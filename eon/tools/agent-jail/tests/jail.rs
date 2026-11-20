//! Jail integration (plan section 11): the real jail, bwrap and all, with
//! the probe's stand-in harness, a fake credential and a fake model host,
//! on the host-test store (the host's /nix/store, the dev shell's cargo,
//! the test environment's CARGO_HOME as the Cargo lower layer). No model
//! and no real credential is involved.
#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_jail::control::Control;
use agent_jail::enter::{Host, Options};
use agent_jail::pins::Pins;
use agent_jail::proxy::{Limits, Resolver};
use agent_jail::spec::Mode;
use agent_jail::task_env::{TaskEnv, Template, Test};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("AGENT_JAIL_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("agent-jail-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A loopback upstream that accepts connections and reads until closed.
fn fake_upstream() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut sink = [0u8; 1024];
                while matches!(stream.read(&mut sink), Ok(count) if count > 0) {}
            });
        }
    });
    address
}

/// The jail runs on the pinned host tools (`host-tools.toml`) from the
/// host store. A machine without those store paths skips the jail tests
/// with a message, unless AGENT_JAIL_REQUIRE_PINS is set.
fn pins() -> Option<Pins> {
    match Pins::load(&["bwrap", "dcg", "bash", "nix"], Path::new("/nix/store")) {
        Ok(pins) => Some(pins),
        Err(err) if std::env::var_os("AGENT_JAIL_REQUIRE_PINS").is_none() => {
            eprintln!("skipping: the pinned host tools are not here ({err})");
            None
        }
        Err(err) => panic!("AGENT_JAIL_REQUIRE_PINS is set and the pins are missing: {err}"),
    }
}

/// The host-test task env under `dir`, whose build template is `build`.
fn task(dir: &Path, build: &str) -> TaskEnv {
    TaskEnv::host_for_tests(
        &dir.join("env"),
        Test {
            command: vec!["env".into()],
            build: vec!["bash".into(), "-c".into(), build.into()],
        },
    )
    .unwrap()
}

fn test_host(dir: &Path, profile: &str, mut pins: Pins, task: TaskEnv) -> Host {
    // The CA bundle is a host file the test sandbox may lack; any file will do.
    let bundle = dir.join("ca.crt");
    std::fs::write(&bundle, "").unwrap();
    pins.tools.insert("ca-bundle".into(), bundle);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let auth = dir.join("credential.json");
    let credential = if profile == "claude" {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat01-integration-fake-access-0123456789","refreshToken":"sk-ant-ort01-integration-fake-refresh-0123456789","expiresAt":{}}}}}"#,
            (now + 86_400) * 1000
        )
    } else {
        unreachable!("the integration test runs the claude profile")
    };
    std::fs::write(&auth, credential).unwrap();
    let upstream = fake_upstream();
    let resolver: Resolver = Arc::new(move |_, _| Ok(vec![upstream]));
    Host {
        pins,
        self_exe: PathBuf::from(env!("CARGO_BIN_EXE_agent-jail")),
        auth,
        resolver,
        limits: Limits::default(),
        now,
        task,
        disk_floor: agent_jail::enter::DiskFloor::new(0),
    }
}

fn options(dir: &Path, mode: Mode, template: Option<Template>) -> Options {
    Options {
        profile: "claude".into(),
        run: dir.join("run"),
        wall_cap: 5,
        probe: mode == Mode::StandIn,
        mode,
        slots: BTreeMap::new(),
        control: Control::None,
        template,
        base_rw: false,
    }
}

#[test]
fn the_stand_in_probe_passes_in_a_real_jail() {
    let Some(pins) = pins() else { return };
    let dir = scratch("probe");
    let host = test_host(&dir, "claude", pins, task(&dir, "true"));
    // A descriptor a heavy.sh or rota flock would leave open: it must be
    // closed before the jail (the outer probe and guard condition 10).
    let (inherited, _keep) = std::io::pipe().unwrap();
    rustix::io::fcntl_setfd(&inherited, rustix::io::FdFlags::empty()).unwrap();
    let options = options(&dir, Mode::StandIn, None);
    let report = agent_jail::probe::run(&options, &host).unwrap();
    let failed: Vec<&serde_json::Value> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|check| check["pass"] != true)
        .collect();
    assert!(failed.is_empty(), "failed checks: {failed:#?}");
    assert_eq!(report["pass"], true);
    // The fetch checks' observations, for the proof record.
    for check in report["checks"].as_array().unwrap() {
        let name = check["name"].as_str().unwrap_or_default();
        if ["store nix", "default store", "cargo fetch"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            eprintln!("{name}: {}", check["observed"]);
        }
    }
    // A second run directory gets a fresh home; the first is refused.
    let again = agent_jail::probe::run(&options, &host);
    assert!(again.unwrap_err().contains("fresh"));
    // The harness's output went to the run directory, not to ours.
    let out = std::fs::read_to_string(dir.join("run/meta/harness.out")).unwrap();
    assert!(out.contains("\"layer\":\"inner\""));
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("run/meta/run.json")).unwrap())
            .unwrap();
    assert_eq!(run["store"], serde_json::json!({"kind": "host-test"}));
    assert_eq!(run["overlay_left"], serde_json::json!([]));
    assert!(!dir.join("run/overlay").exists());
}

/// Runs one `command` jail and returns its outcome and its standard output.
fn command(
    dir: &Path,
    build: &str,
    template: Template,
) -> (agent_jail::enter::Outcome, String, TaskEnv) {
    let pins = pins().expect("checked by the caller");
    let task = task(dir, build);
    let host = test_host(dir, "claude", pins, task.clone());
    let outcome =
        agent_jail::enter::enter(&options(dir, Mode::Command, Some(template)), &host).unwrap();
    let out = std::fs::read_to_string(dir.join("run/meta/harness.out")).unwrap();
    (outcome, out, task)
}

#[test]
fn a_command_runs_its_template_with_the_dev_shell_path_and_no_proxy() {
    if pins().is_none() {
        return;
    }
    let dir = scratch("command-test");
    let (outcome, out, task) = command(&dir, "true", Template::Test);
    let errors = std::fs::read_to_string(dir.join("run/meta/harness.err")).unwrap_or_default();
    assert_eq!(outcome.exit, 0, "{out}\n{errors}");
    assert!(!outcome.quarantined && outcome.minutes_left.is_none());
    let lines: Vec<&str> = out.lines().collect();
    let path = format!("PATH=/bench/bin:{}", task.path().join(":"));
    for want in [
        path.as_str(),
        "CARGO_HOME=/bench/cargo",
        "CARGO_TARGET_DIR=/bench/target",
        "CARGO_NET_OFFLINE=true",
        "CARGO_INCREMENTAL=0",
    ] {
        assert!(lines.contains(&want), "{want} missing from {out}");
    }
    for proxy in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "ALL_PROXY"] {
        assert!(
            !out.contains(&format!("{proxy}=")),
            "{proxy} reached the command: {out}"
        );
    }
    assert!(!dir.join("run/overlay").exists());
}

#[test]
fn a_command_writes_only_its_private_layers() {
    if pins().is_none() {
        return;
    }
    let dir = scratch("command-build");
    let marker = format!("agent-jail-it-{}", std::process::id());
    // Deleting a lower-layer file leaves an overlay whiteout (a 0:0
    // character device) in the upper layer, which the scan lets pass. It is
    // deleted before the marker exists, so the loop can only pick a lower file.
    let build = format!(
        "for f in /bench/cargo/.[!.]* /bench/cargo/*; do if [ -f \"$f\" ]; then rm \"$f\" && echo \"${{f#/bench/cargo/}}\"; break; fi; done && echo built > /bench/target/{marker} && echo cached > /bench/cargo/{marker} && cat /bench/target/{marker} /bench/cargo/{marker}"
    );
    let (outcome, out, task) = command(&dir, &build, Template::Build);
    let errors = std::fs::read_to_string(dir.join("run/meta/harness.err")).unwrap_or_default();
    assert_eq!(outcome.exit, 0, "{out}\n{errors}");
    let removed = out
        .strip_suffix("built\ncached\n")
        .and_then(|rest| rest.strip_suffix('\n'))
        .filter(|name| !name.is_empty() && !name.contains('\n'))
        .unwrap_or_else(|| panic!("no lower-layer file was deleted: {out:?}"));
    assert!(task.cargo().join(removed).is_file(), "{removed}");
    assert!(!outcome.quarantined);
    assert!(
        outcome.overlay_left.is_empty(),
        "{:?}",
        outcome.overlay_left
    );
    // The bases are unchanged, the private layers gone.
    assert!(!task.target().join(&marker).exists());
    assert_eq!(std::fs::read_dir(task.target()).unwrap().count(), 0);
    assert!(!task.cargo().join(&marker).exists());
    assert!(!dir.join("run/overlay").exists());
}

#[test]
fn locked_down_private_layers_are_opened_scanned_and_deleted() {
    if pins().is_none() {
        return;
    }
    let dir = scratch("command-token");
    // A token-shaped string in a directory the command locks down: the
    // host must still read it (open up), quarantine, and delete the layers.
    let build = format!(
        "mkdir -p /bench/target/a/b && echo sk-ant-oat01-{} > /bench/target/a/b/t && chmod 000 /bench/target/a/b && chmod 0500 /bench/target/a",
        "x".repeat(48)
    );
    let (outcome, _, _) = command(&dir, &build, Template::Build);
    assert_eq!(outcome.exit, 0);
    assert!(outcome.quarantined);
    assert_ne!(agent_jail::enter::exit_status(&outcome), 0);
    assert!(
        outcome.overlay_left.is_empty(),
        "{:?}",
        outcome.overlay_left
    );
    assert!(!dir.join("run/overlay").exists());
    let report = std::fs::read_to_string(dir.join("run/meta/token-scan.json")).unwrap();
    assert!(report.contains("overlay/target/upper/a/b/t"), "{report}");
}

#[test]
fn init_outside_a_jail_is_refused_by_the_guard() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_agent-jail"))
        .arg("init")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(agent_jail::GUARD_EXIT));
    assert!(String::from_utf8_lossy(&status.stderr).contains("agent-jail guard"));
}

#[test]
fn hook_commands_block_on_their_own_errors() {
    // Outside a jail there is no spec: the dcg shim must block (exit 2),
    // never exit 1, which Codex treats as allow. Bad arguments block too.
    for args in [
        &["dcg-hook", "codex"][..],
        &["jail-hook"][..],
        &["control-hook", "crash"][..],
        &["jail-hook", "a", "b"][..],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agent-jail"))
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(agent_jail::hook::DENY_EXIT),
            "{args:?}"
        );
    }
}

#[test]
fn a_control_variant_needs_a_harness_probe() {
    let Some(pins) = pins() else { return };
    let dir = scratch("control");
    let host = test_host(&dir, "claude", pins, task(&dir, "true"));
    let options = Options {
        control: Control::Web,
        ..options(&dir, Mode::StandIn, None)
    };
    assert!(
        agent_jail::enter::enter(&options, &host)
            .unwrap_err()
            .contains("control")
    );
    let not_probe = Options {
        probe: false,
        mode: Mode::Harness,
        ..options
    };
    assert!(agent_jail::enter::enter(&not_probe, &host).is_err());
}

#[test]
fn the_wall_cap_tears_the_whole_jail_down() {
    let Some(pins) = pins() else { return };
    let dir = scratch("cap");
    let beat = dir.join("beat");
    // PID 1 starts a grandchild that keeps writing, as a harness's command
    // would; the cap kill of bwrap alone must stop it.
    let script = format!(
        "(while :; do date +%s%N > {}; sleep 0.05; done) & wait",
        beat.display()
    );
    let mut child = std::process::Command::new(pins.tool("bwrap").unwrap())
        .args(agent_jail::enter::OUTER_LIFETIME)
        .args([
            "--unshare-user",
            "--bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
        ])
        .args(["--", "/bin/sh", "-c", &script])
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let waited = agent_jail::enter::wait_capped(
        &mut child,
        started,
        std::time::Duration::from_millis(800),
        None,
    )
    .unwrap();
    assert!(waited.killed_at_cap && !waited.killed_at_disk_floor);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let before = std::fs::read_to_string(&beat).expect("the grandchild ran");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        std::fs::read_to_string(&beat).unwrap(),
        before,
        "a jail process outlived the cap"
    );
}

#[test]
fn a_wrapper_that_cannot_start_its_inner_layer_exits_2() {
    // Outside a jail the wrapper finds no spec, a setup failure: it must
    // report 2, which harnesses read as a failed command, never the tool's.
    assert_eq!(
        agent_jail::wrapper::run("/bench/bin/ls", "ls".into(), vec![]),
        agent_jail::wrapper::SETUP_FAILED
    );
    assert_eq!(agent_jail::wrapper::SETUP_FAILED, 2);
}
