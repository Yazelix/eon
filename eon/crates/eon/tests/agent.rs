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
        Some(2)
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
        Some(3)
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
    assert_eq!(output.stderr, b"rota: arguments must be UTF-8\n");
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
