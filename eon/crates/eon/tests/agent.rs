#![cfg(target_os = "linux")]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn config(path: &Path, backend: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            r#"
state_file = '{}'
log_file = '{}'
[[backends]]
name = '{backend}'
roles = ['implement', 'review']
guard = ['true']
allow_unknown_quota = true
[backends.commands]
medium = ['never-start-this', '{{prompt}}']
high = ['never-start-this', '{{prompt}}']
"#,
            path.with_extension("state").display(),
            path.with_extension("log").display()
        ),
    )
    .unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_eon"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("xdg"))
        .env("EON_CONFIG_HOME", root.join("eon"))
        .env("ROTA_CONFIG", root.join("override.toml"))
        .current_dir(root)
        .arg("agent")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn slot_specific_config_overrides_primary_without_leaking_into_command() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("eon")).unwrap();
    fs::write(root.join("eon/agent.toml"), "invalid primary config").unwrap();
    let dedicated = root.join("slots.toml");
    config(&dedicated, "slots-only");
    let output = run(
        root,
        &[
            "slot",
            "--config",
            dedicated.to_str().unwrap(),
            "run",
            "--",
            "sh",
            "-c",
            "printf '%s' \"$ROTA_CONFIG\"; exit 9",
        ],
    );
    assert_eq!(output.status.code(), Some(9), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        root.join("override.toml").to_str().unwrap()
    );
    assert!(
        run(
            root,
            &[
                "slot",
                "--config",
                dedicated.to_str().unwrap(),
                "ls",
                "--json"
            ]
        )
        .status
        .success()
    );
    assert!(
        !run(root, &["slot", "--config", "missing", "run", "--", "true"])
            .status
            .success()
    );
}

#[test]
fn launch_capture_never_reaches_managed_shells_or_anima() {
    use std::os::unix::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let script = "test -z \"${!EON_LAUNCH_@}\" && printf clean";
    for tool in ["eon-bash", "anima"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_eon"));
        command
            .env("EON_CONFIG_HOME", root.path())
            .env("EON_BASH", "bash")
            .env("EON_ANIMA", "bash")
            .env("EON_LAUNCH_FACTS", "/missing/capture")
            .env("EON_LAUNCH_SAVED_TEST", "+secret");
        if tool == "eon-bash" {
            command.arg0(tool);
        } else {
            command.arg("anima");
        }
        let output = command
            .args(["--noprofile", "--norc", "-c", script])
            .output()
            .unwrap();
        assert!(output.status.success(), "{tool}: {output:?}");
        assert_eq!(output.stdout, b"clean");
    }
}

#[test]
fn launch_capture_never_reaches_agent_or_slot_exec() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let probe = root.join("probe");
    fs::write(
        &probe,
        "#!/bin/sh\n! env | grep '^EON_LAUNCH_' && printf clean\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    let config_path = root.join("override.toml");
    config(&config_path, "fake");
    fs::write(
        &config_path,
        fs::read_to_string(&config_path)
            .unwrap()
            .replace("never-start-this", probe.to_str().unwrap()),
    )
    .unwrap();
    fs::write(root.join("brief"), "test").unwrap();
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&config_path)
            .unwrap(),
        "\n[slots]\nnice = 0\n[[slots.classes]]\nname = 'probe'\nslots = [1]"
    )
    .unwrap();
    for args in [
        vec!["launch", "--only", "fake", "brief"],
        // The unclassified Nix fast path still executes a real external command.
        vec!["slot", "run", "--", "nix", "eval", "--expr", "1"],
        vec![
            "slot",
            "run",
            "--class",
            "probe",
            "--",
            probe.to_str().unwrap(),
        ],
    ] {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&probe, bin.join("nix")).ok();
        let output = Command::new(env!("CARGO_BIN_EXE_eon"))
            .env_clear()
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("HOME", root)
            .env("EON_CONFIG_HOME", root.join("eon"))
            .env("ROTA_CONFIG", &config_path)
            .env("EON_LAUNCH_FACTS", "/missing/capture")
            .env("EON_LAUNCH_SAVED_TEST", "+secret")
            .current_dir(root)
            .arg("agent")
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.ends_with(b"clean"), "{output:?}");
    }
}

#[test]
fn agent_config_prefers_eon_then_rota_and_preserves_bad_primary() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let primary = root.join("eon/agent.toml");
    let fallback = root.join("override.toml");
    config(&fallback, "fallback");
    let status = |path: &Path, backend: &str| {
        let output = run(root, &["status", "--json"]);
        assert!(output.status.success(), "{:?}", output);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json[0]["backend"], backend);
        assert_eq!(json[0]["config_file"], path.to_str().unwrap());
    };
    status(&fallback, "fallback");
    config(&primary, "primary");
    status(&primary, "primary");
    let output = run(root, &["status"]);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with(&format!("config: {}\n", primary.display()))
    );
    fs::write(&primary, "not valid TOML").unwrap();
    let output = run(root, &["status", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    fs::remove_file(&primary).unwrap();
    fs::create_dir(&primary).unwrap();
    assert_eq!(run(root, &["status"]).status.code(), Some(2));
    assert!(!primary.with_extension("state").exists());
    assert!(!fallback.with_extension("log").exists());
}

#[test]
fn agent_dispatch_keeps_arguments_exit_codes_and_eonbase_boundary() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let root = tempfile::tempdir().unwrap();
    config(&root.path().join("override.toml"), "fake");
    fs::write(root.path().join("brief"), "secret").unwrap();
    let output = run(
        root.path(),
        &["launch", "--dry-run", "--only", "fake", "brief"],
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("pick: fake\ncommand: never-start-this ")
    );
    assert_eq!(
        run(
            root.path(),
            &["launch", "--dry-run", "--only", "missing", "brief"]
        )
        .status
        .code(),
        Some(69)
    );
    let path = root.path().join("override.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("['true']", "['false']"),
    )
    .unwrap();
    assert_eq!(
        run(root.path(), &["launch", "--dry-run", "brief"])
            .status
            .code(),
        Some(75)
    );
    let help = Command::new(env!("CARGO_BIN_EXE_eon"))
        .env_clear()
        .args(["agent", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .starts_with("Usage: eon agent launch")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
        .arg("agent")
        .arg(std::ffi::OsString::from_vec(vec![255]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stderr, b"eon agent: arguments must be UTF-8\n");
    let alias = root.path().join("eonbase");
    symlink(env!("CARGO_BIN_EXE_eon"), &alias).unwrap();
    let output = Command::new(alias)
        .args(["agent", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn agent_config_uses_eons_xdg_home_and_rotas_default_fallback() {
    let root = tempfile::tempdir().unwrap();
    let xdg = root.path().join("xdg");
    let primary = xdg.join("eon/agent.toml");
    let fallback = xdg.join("rota/config.toml");
    config(&fallback, "fallback");
    let status = || {
        Command::new(env!("CARGO_BIN_EXE_eon"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", &xdg)
            .args(["agent", "status", "--json"])
            .output()
            .unwrap()
    };
    let output = status();
    assert!(output.status.success(), "{:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json[0]["config_file"], fallback.to_str().unwrap());
    config(&primary, "primary");
    let output = status();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json[0]["backend"], "primary");
    assert_eq!(json[0]["config_file"], primary.to_str().unwrap());
}

#[test]
fn agent_config_provenance_preserves_filename_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(std::ffi::OsString::from_vec(vec![255]));
    let path = root.join("eon/agent.toml");
    config(&path, "fake");
    let output = run(&root, &["status", "--json"]);
    assert!(output.status.success(), "{:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json[0]["config_file"],
        serde_json::json!(path.as_os_str().as_bytes())
    );
}

/// The `eon agent` twins of rota-cli's tests/golden.rs: goldens captured before the host seam
/// (eon-eon-agent-lr8sx.39.1). The only differences are the host's program name in the listed
/// message rows, asserted here explicitly.
mod golden {
    use std::{fs, path::Path, process::Command};

    fn golden(name: &str) -> String {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../eon-agent/tests/fixtures/golden")
                .join(name),
        )
        .unwrap()
    }

    fn root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for child in ["state", "home", "eon"] {
            fs::create_dir_all(root.join(child)).unwrap();
        }
        let config = golden("config.toml").replace("<ROOT>", root.to_str().unwrap());
        fs::write(root.join("eon/agent.toml"), config).unwrap();
        fs::write(root.join("brief"), "brief text\n").unwrap();
        dir
    }

    fn eon_agent(root: &Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_eon"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("xdg"))
            .env("EON_CONFIG_HOME", root.join("eon"))
            .current_dir(root)
            .arg("agent")
            .args(args)
            .output()
            .unwrap()
    }

    /// Replaces the scratch root with `<ROOT>` and every decision time with 0.
    fn normalized(root: &Path, bytes: &[u8]) -> String {
        let text = String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace(root.to_str().unwrap(), "<ROOT>");
        let mut out = String::new();
        let mut rest = text.as_str();
        while let Some(at) = rest.find("\"time\":") {
            let (head, tail) = rest.split_at(at + "\"time\":".len());
            out += head;
            out += "0";
            rest = tail.trim_start_matches(|c: char| c.is_ascii_digit());
        }
        out + rest
    }

    #[test]
    fn eon_agent_help_texts_are_unchanged() {
        let dir = root();
        let help = eon_agent(dir.path(), &["--help"]);
        assert!(help.status.success());
        assert_eq!(
            String::from_utf8(help.stdout).unwrap(),
            golden("eon-agent-help.txt")
        );
        let help = eon_agent(dir.path(), &["retire", "--help"]);
        assert!(help.status.success());
        assert_eq!(
            String::from_utf8(help.stdout).unwrap(),
            golden("eon-agent-retire-help.txt")
        );
    }

    #[test]
    fn eon_agent_names_itself_where_rota_did() {
        let dir = root();
        let root = dir.path();
        let status = eon_agent(root, &["status", "--json"]);
        assert!(status.status.success(), "{status:?}");
        // Listed exception: a run-only backend's availability names the host program.
        assert_eq!(
            normalized(root, &status.stdout),
            golden("eon-agent-status.json")
                .replace("available for rota run", "available for eon agent run")
        );
        let dry = eon_agent(root, &["launch", "--dry-run", "brief"]);
        assert!(dry.status.success(), "{dry:?}");
        assert_eq!(
            normalized(root, &dry.stdout),
            golden("eon-agent-dry-run.stdout")
        );
        // Listed exception (closes eon-eon-agent-lr8sx.31): decision lines say `eon agent:`.
        let stderr = normalized(root, &dry.stderr);
        assert!(!stderr.contains("rota"), "{stderr}");
        assert_eq!(
            stderr,
            golden("eon-agent-dry-run.stderr").replace("rota: ", "eon agent: ")
        );
        // The decision record names no program: a real launch writes rota's record exactly.
        let launch = eon_agent(root, &["launch", "brief"]);
        assert!(launch.status.success(), "{launch:?}");
        let record = fs::read(root.join("state/decisions.jsonl")).unwrap();
        assert_eq!(normalized(root, &record), golden("rota-decision.jsonl"));
    }

    #[test]
    fn eon_agent_config_errors_name_eon_agent() {
        let dir = root();
        let root = dir.path();
        let config = fs::read_to_string(root.join("eon/agent.toml")).unwrap();
        fs::write(
            root.join("eon/agent.toml"),
            format!("{config}\n[run]\nunset_env = ['XDG_RUNTIME_DIR']\n"),
        )
        .unwrap();
        let status = eon_agent(root, &["status"]);
        assert!(!status.status.success());
        assert_eq!(
            String::from_utf8(status.stderr).unwrap(),
            "eon agent: [run] must keep XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS: eon agent run reaches the systemd user manager through them\n"
        );
    }

    #[test]
    fn eon_agent_slot_delegate_reinvokes_eon_agent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = root();
        let root = dir.path();
        let runner = root.join("runner.sh");
        fs::write(
            &runner,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec \"$@\"\n",
                root.join("argv").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
        // Slot state must be private (owner-only), as Boat checks.
        fs::set_permissions(root.join("state"), fs::Permissions::from_mode(0o700)).unwrap();
        let config = fs::read_to_string(root.join("eon/agent.toml")).unwrap();
        fs::write(
            root.join("eon/agent.toml"),
            format!(
                "{config}\n[slots]\nlocal_runner = ['{}']\n",
                runner.display()
            ),
        )
        .unwrap();
        let output = eon_agent(root, &["slot", "run", "--", "sh", "-c", "exit 7"]);
        assert_eq!(output.status.code(), Some(7), "{output:?}");
        let argv = fs::read_to_string(root.join("argv")).unwrap();
        let argv: Vec<_> = argv.lines().collect();
        assert_eq!(argv[0], env!("CARGO_BIN_EXE_eon"));
        assert_eq!(argv[1..3], ["agent", "_slot-local-exec"]);
        assert_eq!(argv[4..], ["--", "sh", "-c", "exit 7"]);
    }
}

/// `eon agent` runs slot commands through Boat (from the `eon-boat` crate): a permitted
/// shape meets Boat's switch, Boat's `slot ls` fields appear, and Boat checks its own keys in
/// `[slots.boat]` on every command while rota keeps the table opaque.
#[test]
fn boat_route_is_wired_into_eon_agent() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // A routed run keeps its state (here the root) owner-only.
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.join("override.toml");
    let with_boat = |boat: &str| {
        config(&path, "fake");
        let text = fs::read_to_string(&path).unwrap();
        let text = text.replacen(
            "[[backends]]",
            &format!("[slots.boat]\n{boat}\n[[backends]]"),
            1,
        );
        fs::write(&path, text).unwrap();
    };

    with_boat("allow_shapes = ['nix-gate']");
    // No `boat-on` switch: Boat keeps the permitted gate local and runs it through its
    // own local path, without calling the Boat CLI.
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    // A sibling test's fork can inherit a writer this process opens, and exec then fails
    // with ETXTBSY, so a child process writes the stub.
    let mut writer = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(bin.join("nix"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        &mut writer.stdin.take().unwrap(),
        b"#!/bin/sh\nprintf 'stub %s' \"$*\"\n",
    )
    .unwrap();
    assert!(writer.wait().unwrap().success());
    let output = Command::new(env!("CARGO_BIN_EXE_eon"))
        .env_clear()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("xdg"))
        .env("EON_CONFIG_HOME", root.join("eon"))
        .env("ROTA_CONFIG", &path)
        .current_dir(root)
        .args(["agent", "slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"stub build .#default");
    let log = fs::read_to_string(root.join("slots/log.jsonl")).unwrap();
    let events: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|event| event["fallback_reason"] == "boat-not-enabled"),
        "{log}"
    );
    let output = run(root, &["slot", "ls", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listing["boat_lease_held"], false, "{listing}");

    let config = path.to_str().unwrap();
    // Boat refuses its own bad keys on slot and non-slot commands alike (exit 2).
    for (boat, message) in [
        (
            "allow_shapes = ['nix-gate']\njobs = 9",
            "invalid slots.boat policy",
        ),
        (
            "allow_shapes = ['nix-gate']\nbogus = 1",
            "unknown field `bogus`",
        ),
    ] {
        with_boat(boat);
        // Both ways a file is loaded: the primary lookup and `slot --config FILE`.
        for args in [
            &["slot", "ls"][..],
            &["slot", "--config", config, "ls"],
            &["status"],
        ] {
            let output = run(root, args);
            assert_eq!(output.status.code(), Some(2), "{boat} {args:?}: {output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(message), "{boat} {args:?}: {stderr}");
        }
    }

    // The removed snapshot key warns on slot commands only.
    with_boat("allow_shapes = ['nix-gate']\nsnapshot = ''");
    let tombstone = "eon agent: warning: slots.boat.snapshot is removed and ignored";
    for args in [&["slot", "ls"][..], &["slot", "--config", config, "ls"]] {
        let output = run(root, args);
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(tombstone),
            "{args:?}: {output:?}"
        );
    }
    let output = run(root, &["status"]);
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("snapshot"),
        "{output:?}"
    );
}
