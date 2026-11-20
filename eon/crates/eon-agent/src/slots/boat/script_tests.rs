use super::super::memory::Class;
use super::{Cause, Limit, Machine, Observation};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_ID: &str = "11111111111111111111111111111111";
const CGROUP: &str = "/user.slice/eon-boat-11111111111111111111111111111111.scope";

fn runner(refuse_caps: bool, delayed_log: bool) -> (tempfile::TempDir, Command) {
    runner_with_limits(refuse_caps, delayed_log, "6442450944", "", "1073741824")
}

fn runner_with_limits(
    refuse_caps: bool,
    delayed_log: bool,
    cap: &str,
    high: &str,
    margin: &str,
) -> (tempfile::TempDir, Command) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let source = home.join("eon");
    let run = home.join("eon-run");
    let bin = home.join(".nix-profile/bin");
    let cgroup = root.path().join("cgroup");
    let unit = cgroup.join(&CGROUP[1..]);
    let proc = root.path().join("proc");
    for path in [
        &source,
        &run,
        &bin,
        &home.join("nix/store"),
        &home.join("runtime"),
        &unit,
        &proc.join("self"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    UnixListener::bind(home.join("runtime/bus")).unwrap();
    for path in [&unit, &cgroup.join("user.slice")] {
        for control in ["memory.max", "memory.high"] {
            fs::write(path.join(control), "max\n").unwrap();
        }
    }
    fs::write(
        proc.join("meminfo"),
        "MemTotal: 16777216 kB\nMemAvailable: 12582912 kB\n",
    )
    .unwrap();
    fs::write(proc.join("vmstat"), "oom_kill 5\n").unwrap();
    fs::write(proc.join("kmsg"), "<6>[1.0] guest boot\n").unwrap();
    fs::write(source.join("source"), "captured").unwrap();
    for args in [["init", "-q"], ["add", "."]] {
        assert!(
            Command::new("git")
                .current_dir(&source)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let tree = Command::new("git")
        .current_dir(&source)
        .arg("write-tree")
        .output()
        .unwrap();
    assert!(tree.status.success());
    let tree = String::from_utf8(tree.stdout).unwrap();
    let tee = Command::new("sh")
        .args(["-c", "command -v tee"])
        .output()
        .unwrap();
    assert!(tee.status.success());
    let tee = String::from_utf8(tee.stdout).unwrap();
    // The test sandbox does not assume /usr/bin/env. Bash also preserves the
    // exported controller/workload functions through the launcher boundary.
    let bash = Command::new("sh")
        .args(["-c", "command -v bash"])
        .output()
        .unwrap();
    assert!(bash.status.success());
    let bash = String::from_utf8(bash.stdout).unwrap();
    let shim = bin.join("scope-fixture");
    let body = include_str!("../../../tests/fixtures/boat-scope.sh");
    fs::write(&shim, format!("#!{}\n{body}", bash.trim())).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    for name in [
        "nix",
        "systemd-run",
        "cat",
        "mkdir",
        "rmdir",
        "dmesg",
        "stat",
        "setpriv",
        "bwrap",
    ] {
        symlink(&shim, bin.join(name)).unwrap();
    }
    let tee_path = bin.join("tee");
    let delay = if delayed_log { "sleep 1\n" } else { "" };
    fs::write(
        &tee_path,
        format!("#!/bin/sh\n{delay}exec \"$REAL_TEE\" \"$@\"\n"),
    )
    .unwrap();
    fs::set_permissions(tee_path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(run.join("go"), "").unwrap();
    for name in [".bashrc", ".bash_profile", ".profile"] {
        fs::write(home.join(name), format!("export ASCII_TOKEN=DO_NOT_READ_RC\nexport BOAT_ID=DO_NOT_READ_RC\ntouch '{}'/rc-read\n", home.display())).unwrap();
    }
    let script = concat!(include_str!("scope.sh"), "\n", include_str!("run.sh"))
        .replace(
            "manager_runtime=/run/user/$(id -u)",
            "manager_runtime=/home/user/runtime",
        )
        .replace("/nix/store", "/home/user/nix/store")
        .replace(
            "/nix/var/nix/daemon-socket/socket",
            "/home/user/nix/daemon-socket",
        )
        .replace("/sys/fs/cgroup", cgroup.to_str().unwrap())
        .replace(
            "/proc/self/cgroup",
            proc.join("self/cgroup").to_str().unwrap(),
        )
        .replace(
            "/proc/self/oom_score_adj",
            proc.join("self/oom_score_adj").to_str().unwrap(),
        )
        .replace("/proc/meminfo", proc.join("meminfo").to_str().unwrap())
        .replace("/proc/vmstat", proc.join("vmstat").to_str().unwrap())
        .replace("/usr/bin/python3", "python3")
        .replace("/home/user", home.to_str().unwrap());
    // Only the passed copy changes; the shipped reader stays the qualified bytes.
    let reader = include_str!("read-kernel.py")
        .replace(
            "/run/eon-boat-observer/kernel.sock",
            root.path().join("observer.sock").to_str().unwrap(),
        )
        .replace("/usr/bin/timeout", "timeout")
        .replace("/usr/bin/dmesg", "dmesg");
    let path = std::env::var("PATH").unwrap();
    let mut command = Command::new("bash");
    command
        .env_clear()
        .env("HOME", &home)
        .env("PATH", format!("{}:{path}", bin.display()))
        .env("REAL_TEE", tee.trim())
        .env("FIXTURE_REAL_PATH", path)
        .env("FIXTURE_CGROUP", cgroup)
        .env("FIXTURE_SCOPE", CGROUP)
        .env("FIXTURE_PROC", proc)
        .env("FIXTURE_REFUSE_CAPS", if refuse_caps { "1" } else { "0" })
        .args(["--noprofile", "--norc", "-c"])
        .arg(script)
        .args([
            "test",
            tree.trim(),
            "",
            "4",
            RUN_ID,
            "0",
            "unused",
            cap,
            high,
            margin,
            &reader,
            "bash",
            "-c",
            "[ -z \"${XDG_RUNTIME_DIR+x}${DBUS_SESSION_BUS_ADDRESS+x}\" ] || exit 99; \
             if (: >&5) 2>/dev/null; then exit 99; fi; \
             printf 'stdout-tail\\n'; printf 'stderr-tail\\n' >&2; exit \"${FIXTURE_GATE_CODE:-0}\"",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    (root, command)
}

#[test]
fn review_runner_waits_for_both_log_tails_before_publishing_a_result() {
    let (root, mut command) = runner(false, true);
    assert!(command.status().unwrap().success());
    let log = root.path().join("home/eon-run/output.log");
    let at_result = fs::read_to_string(&log).unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(5);
    // Reap delayed test writers before asserting the old runner's early publication.
    while !fs::read_to_string(&log)
        .unwrap_or_default()
        .contains("stderr-tail")
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        at_result.contains("stdout-tail"),
        "stdout tail missing when runner exited"
    );
    assert!(
        at_result.contains("stderr-tail"),
        "stderr tail missing when runner exited"
    );
    assert!(at_result.contains("phase=complete"));
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("home/eon-run/result.json")).unwrap())
            .unwrap();
    assert_eq!(result["log_complete"], true);
    assert_eq!(result["build_identity_absent"], true);
    assert!(!root.path().join("home/rc-read").exists());
}

#[test]
fn review_capability_refusal_is_preparation_failure_not_a_gate_result() {
    let (root, mut command) = runner(true, false);
    assert!(command.status().unwrap().success());
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("home/eon-run/result.json")).unwrap())
            .unwrap();
    assert_eq!(result["source_verified"], true);
    assert_eq!(result["exit_code"], 97);
    assert_eq!(result["gate_ran"], false);
}

#[test]
fn runner_refuses_daemon_socket_and_nonlocal_nix_store_before_preparation() {
    for store in [None, Some("daemon"), Some("ssh://foreign-store")] {
        let (root, mut command) = runner(false, false);
        let home = root.path().join("home");
        let _daemon = match store {
            Some(store) => {
                command.env("FIXTURE_NIX_STORE", store);
                None
            }
            None => Some(UnixListener::bind(home.join("nix/daemon-socket")).unwrap()),
        };
        assert!(command.status().unwrap().success());
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("eon-run/result.json")).unwrap()).unwrap();
        assert_eq!(result["exit_code"], 96);
        assert_eq!(result["preparation_started"], false);
        assert_eq!(result["source_verified"], false);
    }
}

fn run_result(root: &tempfile::TempDir, command: &mut Command) -> serde_json::Value {
    assert!(command.status().unwrap().success());
    let run = root.path().join("home/eon-run");
    serde_json::from_slice(&fs::read(run.join("result.json")).unwrap_or_else(|error| {
        panic!(
            "{error}: {}",
            fs::read_to_string(run.join("output.log")).unwrap_or_default()
        )
    }))
    .unwrap()
}

#[test]
fn scope_and_nix_refusals_are_explicit_and_pre_work() {
    for (key, value) in [
        ("FIXTURE_LAUNCH", "fail"),
        ("FIXTURE_NIX_STORE", "fail"),
        ("FIXTURE_NIX_BUILDERS", "fail"),
        ("FIXTURE_NIX_BUILDERS", "ssh://foreign-builder"),
        ("FIXTURE_SCRATCH_FS", "tmpfs"),
        ("FIXTURE_SCRATCH_FS", "ramfs"),
    ] {
        let (root, mut command) = runner(false, false);
        let result = run_result(&root, command.env(key, value));
        assert_eq!(result["exit_code"], 96, "{key}={value}: {result}");
        assert_eq!(result["preparation_started"], false);
        assert_eq!(result["gate_ran"], false);
        assert_eq!(result["source_verified"], false);
        assert_eq!(result["log_complete"], true);
        assert!(result["memory"].is_null());
    }
    let (root, mut command) = runner(false, false);
    let result = run_result(&root, command.env("FIXTURE_GATE_CODE", "96"));
    assert_eq!(result["exit_code"], 96);
    assert_eq!(result["preparation_started"], true);
    assert_eq!(result["gate_ran"], true);
}

#[test]
fn bubblewrap_setup_failure_is_a_pre_work_refusal() {
    let (root, mut command) = runner(false, false);
    let result = run_result(&root, command.env("FIXTURE_BWRAP", "fail"));
    assert!(root.path().join("proc/bwrap-args").is_file());
    assert_eq!(result["exit_code"], 96, "{result}");
    assert_eq!(result["preparation_started"], false);
    assert_eq!(result["source_verified"], false);
    assert_eq!(result["gate_ran"], false);
    assert_eq!(result["log_complete"], true);
    assert_eq!(result["memory"]["drained"], true);
    assert_eq!(
        fs::metadata(root.path().join("home/eon-run/preparation-started"))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn scope_sets_limits_and_keeps_the_guest_manager_outside_the_workload() {
    for (cap, high, margin, expected) in [
        ("6442450944", "", "1073741824", ["6442450944\n", "max\n"]),
        (
            "8589934592",
            "7516192768",
            "2147483648",
            ["8589934592\n", "7516192768\n"],
        ),
    ] {
        let (root, mut command) = runner_with_limits(false, false, cap, high, margin);
        let result = run_result(&root, &mut command);
        assert_eq!(result["exit_code"], 0, "{result}");
        let unit = root.path().join(format!("cgroup{CGROUP}"));
        assert!(
            fs::metadata(unit.join("controller/cgroup.procs"))
                .unwrap()
                .len()
                > 0
        );
        assert!(
            fs::metadata(unit.join("workload/cgroup.procs"))
                .unwrap()
                .len()
                > 0
        );
        assert_eq!(
            fs::read_to_string(unit.join("cgroup.subtree_control")).unwrap(),
            "+memory\n"
        );
        for (key, value) in [
            ("memory.max", expected[0]),
            ("memory.high", expected[1]),
            ("memory.swap.max", "0\n"),
            ("memory.oom.group", "1\n"),
        ] {
            assert_eq!(
                fs::read_to_string(unit.join("workload").join(key)).unwrap(),
                value,
                "{key}"
            );
        }
        let arguments = fs::read_to_string(root.path().join("proc/bwrap-args")).unwrap();
        assert!(arguments.contains("--tmpfs\n/run\n"));
        let scratch = root.path().join("home/eon-run/workload/tmp");
        assert!(arguments.contains(&format!("--bind\n{}\n/tmp\n", scratch.display())));
        assert!(arguments.contains("--unshare-pid\n"));
        let cgroup = root.path().join("cgroup");
        assert!(arguments.contains(&format!("--ro-bind\n{0}\n{0}\n", cgroup.display())));
        assert!(arguments.contains("--unsetenv\nXDG_RUNTIME_DIR\n"));
        assert!(arguments.contains("--unsetenv\nDBUS_SESSION_BUS_ADDRESS\n"));
    }
}

#[test]
fn scope_refuses_a_lower_ancestor_or_insufficient_headroom() {
    for (file, contents, ceiling) in [
        (
            "cgroup/user.slice/memory.high",
            "6442450943\n",
            6442450943u64,
        ),
        (
            "proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 7340031 kB\n",
            6442449920,
        ),
    ] {
        let (root, mut command) = runner(false, false);
        fs::write(root.path().join(file), contents).unwrap();
        let result = run_result(&root, &mut command);
        assert_eq!(result["exit_code"], 96, "{result}");
        assert_eq!(result["preparation_started"], false);
        assert_eq!(result["headroom_failed"]["ceiling"], ceiling);
        assert!(!root.path().join("proc/bwrap-args").exists());
    }
}

#[test]
fn scope_reads_kernel_evidence_through_the_observer_socket_outside_the_workload() {
    use std::io::Write;
    let (root, mut command) = runner(false, false);
    let listener = UnixListener::bind(root.path().join("observer.sock")).unwrap();
    let kmsg = root.path().join("proc/kmsg");
    listener.set_nonblocking(true).unwrap();
    let observer = std::thread::spawn(move || {
        // Bounded, so a runner that never connects fails the assertion instead of hanging.
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut served = 0;
        while served < 2 && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut sample = fs::read(&kmsg).unwrap();
                    sample.push(0);
                    stream.write_all(&sample).unwrap();
                    served += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("{error}"),
            }
        }
        served
    });
    let result = run_result(&root, command.env("FIXTURE_OOM", "job"));
    assert_eq!(observer.join().unwrap(), 2, "{result}");
    let observation: Observation = serde_json::from_value(result["memory"].clone()).unwrap();
    let limit = Limit {
        class: Class::Default,
        cap_gib: 6,
    };
    assert_eq!(
        observation
            .report(limit, Machine::Default, false)
            .unwrap()
            .cause,
        Cause::JobLimit,
        "{result}"
    );
    let home = root.path().join("home");
    for directory in [home.join("eon-run"), home.join("runtime")] {
        for entry in fs::read_dir(&directory).unwrap() {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with("kernel-") && !name.starts_with("eon-boat-kernel."),
                "{name}"
            );
        }
    }
}

#[test]
fn shipped_kernel_reader_is_the_qualified_reader() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(include_str!("read-kernel.py"))),
        "30b8b3ffe52442b0c7a936cb152f10ba88f333b3af85f78f74b7ba3921d0cf78"
    );
}

#[test]
fn scope_reports_oom_from_raw_kernel_evidence_and_waits_for_drain() {
    for (mode, cause) in [
        ("job", Cause::JobLimit),
        ("vm", Cause::VmWide),
        ("mixed", Cause::VmWide),
        ("unknown", Cause::Unknown),
        ("wrapped", Cause::None),
        ("unreadable", Cause::Unknown),
        ("undrained", Cause::Unknown),
    ] {
        let (root, mut command) = runner(false, false);
        if mode == "job" {
            command.env("FIXTURE_RMDIR", "fail");
        }
        let result = run_result(&root, command.env("FIXTURE_OOM", mode));
        let evidence = &result["memory"];
        assert_eq!(evidence["drained"], mode != "undrained", "{mode}: {result}");
        if mode == "wrapped" || mode == "unreadable" {
            assert_eq!(evidence["complete"], false);
        }
        let observation: Observation = serde_json::from_value(evidence.clone()).unwrap();
        let limit = Limit {
            class: Class::Default,
            cap_gib: 6,
        };
        assert_eq!(
            observation
                .report(limit, Machine::Default, false)
                .unwrap()
                .cause,
            cause,
            "{mode}: {result}"
        );
        if mode == "job" {
            assert_eq!(result["exit_code"], 137);
            assert_eq!(evidence["scope_kills"], 2);
            assert_eq!(evidence["own_events"], 1);
        }
    }
}

#[test]
fn review_probe_refuses_generic_token_names_without_disclosing_values() {
    let root = tempfile::tempdir().unwrap();
    let script = include_str!("probe.sh").replace("/home/user", root.path().to_str().unwrap());
    let invoke = |variable: Option<(&str, &str)>| {
        let mut command = Command::new("bash");
        command
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .args(["-c", &script]);
        if let Some((variable, value)) = variable {
            command.env(variable, value);
        }
        command.output().unwrap()
    };
    assert!(invoke(None).status.success());
    assert!(invoke(Some(("ASCII_TOKEN", ""))).status.success());
    for provider in ["ASCII_TOKEN", "BOAT_ID"] {
        let output = invoke(Some((provider, "NEVER_PRINT_THIS_VALUE")));
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT_THIS_VALUE"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT_THIS_VALUE"));
    }
    for variable in [
        "DEPLOY_TOKEN",
        "SERVICE_AUTH_TOKEN",
        "MY_ASCII_TOKEN",
        "BOAT_TOKEN",
        "BOAT_ID_TOKEN",
        "ascii_token",
        "Ascii_Token",
    ] {
        let output = invoke(Some((variable, "NEVER_PRINT_THIS_VALUE")));
        assert_eq!(
            output.status.code(),
            Some(97),
            "{variable}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT_THIS_VALUE"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT_THIS_VALUE"));
    }
    let multiline = invoke(Some((
        "SAFE_TEXT",
        "description\nDEPLOY_TOKEN=NEVER_PRINT_THIS_VALUE",
    )));
    assert!(multiline.status.success());
    assert!(!String::from_utf8_lossy(&multiline.stdout).contains("NEVER_PRINT_THIS_VALUE"));
}

#[test]
fn review_runner_rejects_provider_identity_in_the_build_even_when_empty() {
    for name in ["ASCII_TOKEN", "BOAT_ID"] {
        let (root, mut command) = runner(false, false);
        command.env(name, "");
        assert!(command.status().unwrap().success());
        let result: serde_json::Value = serde_json::from_slice(
            &fs::read(root.path().join("home/eon-run/result.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(result["exit_code"], 97);
        assert_eq!(result["gate_ran"], false);
        assert_eq!(result["build_identity_absent"], false);
    }
}

#[test]
fn review_probe_refuses_unclassified_boat_file_without_disclosing_contents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".config/ascii/boat/config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "NEVER_PRINT_FILE_CONTENTS").unwrap();
    let script = include_str!("probe.sh").replace("/home/user", root.path().to_str().unwrap());
    let output = Command::new("bash")
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", std::env::var_os("PATH").unwrap())
        .args(["-c", &script])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(97));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT_FILE_CONTENTS"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT_FILE_CONTENTS"));
}

#[test]
fn review_probe_allows_only_the_approved_neutral_boat_config_schema() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".config/ascii/boat/config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let script = include_str!("probe.sh").replace("/home/user", root.path().to_str().unwrap());
    let invoke = || {
        Command::new("bash")
            .env_clear()
            .env("HOME", root.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .args(["-c", &script])
            .output()
            .unwrap()
    };
    let neutral = r#"{"api_url":"NEVER_PRINT_METADATA","channel":"NEVER_PRINT_METADATA"}"#;
    fs::write(&path, neutral).unwrap();
    let output = invoke();
    assert!(output.status.success(), "neutral metadata refused");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT"));
    for invalid in [
        r#"{"api_url":"NEVER_PRINT_METADATA","channel":true}"#,
        r#"{"api_url":"NEVER_PRINT_METADATA"}"#,
        r#"["NEVER_PRINT_METADATA"]"#,
        r#"{"api_url":"NEVER_PRINT_METADATA","api_url":"duplicate","channel":"stable"}"#,
        "NEVER_PRINT_INVALID_JSON",
    ] {
        fs::write(&path, invalid).unwrap();
        let output = invoke();
        assert_eq!(output.status.code(), Some(97));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT"));
    }
    for key in [
        "API_KEY",
        "ToKeN",
        "secret",
        "auth",
        "password",
        "credential",
        "unknown",
    ] {
        for value in ["NEVER_PRINT_CREDENTIAL", ""] {
            fs::write(
                &path,
                format!(r#"{{"api_url":"neutral","channel":"stable","{key}":"{value}"}}"#),
            )
            .unwrap();
            let output = invoke();
            assert_eq!(
                output.status.code(),
                Some(97),
                "unexpected field {key} accepted"
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT"));
        }
    }
    fs::remove_file(&path).unwrap();
    let target = root.path().join("neutral.json");
    fs::write(&target, neutral).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert_eq!(invoke().status.code(), Some(97));
    fs::remove_file(target).unwrap();
    assert_eq!(invoke().status.code(), Some(97), "dangling link accepted");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(invoke().status.code(), Some(97));
    fs::remove_dir(&path).unwrap();
    fs::write(&path, neutral).unwrap();
    let login = root.path().join(".netrc");
    fs::write(&login, "NEVER_PRINT_CALLER_SECRET").unwrap();
    let output = invoke();
    assert_eq!(output.status.code(), Some(97));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_PRINT"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_PRINT"));
    fs::remove_file(login).unwrap();
    let python = root.path().join("python3");
    fs::write(&python, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(python, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(invoke().status.code(), Some(97), "failed parser accepted");
}

#[test]
fn review_probe_refuses_if_environment_name_enumeration_fails() {
    let root = tempfile::tempdir().unwrap();
    let script = format!("printf() {{ return 1; }}\n{}", include_str!("probe.sh"))
        .replace("/home/user", root.path().to_str().unwrap());
    let output = Command::new("bash")
        .env_clear()
        .env("HOME", root.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .args(["-c", &script])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(97));
}

#[test]
fn review_probe_enumerates_names_without_a_value_dump() {
    let root = tempfile::tempdir().unwrap();
    let env = root.path().join("env");
    fs::write(&env, "#!/bin/sh\ntouch \"$HOME/env-called\"\nexit 1\n").unwrap();
    fs::set_permissions(env, fs::Permissions::from_mode(0o755)).unwrap();
    let script = include_str!("probe.sh").replace("/home/user", root.path().to_str().unwrap());
    let output = Command::new("bash")
        .env_clear()
        .env("HOME", root.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("ASCII_TOKEN", "NEVER_READ_OR_PRINT_PROVIDER_VALUE")
        .env("BOAT_ID", "NEVER_READ_OR_PRINT_PROVIDER_VALUE")
        .args(["-c", &script])
        .output()
        .unwrap();
    assert!(
        !root.path().join("env-called").exists(),
        "value-dumping env invoked"
    );
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_READ_OR_PRINT"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEVER_READ_OR_PRINT"));
}
