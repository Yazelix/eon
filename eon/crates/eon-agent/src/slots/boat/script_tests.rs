use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn runner(refuse_caps: bool, delayed_log: bool) -> (tempfile::TempDir, Command) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let source = home.join("eon");
    let run = home.join("eon-run");
    let bin = home.join(".nix-profile/bin");
    for path in [&source, &run, &bin] {
        fs::create_dir_all(path).unwrap();
    }
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
    for (name, body) in [
        ("nix", "exit 0"),
        (
            "setpriv",
            if refuse_caps {
                "exit 97"
            } else {
                "shift 2; exec \"$@\""
            },
        ),
        (
            "tee",
            if delayed_log {
                "sleep 1; exec \"$REAL_TEE\" \"$@\""
            } else {
                "exec \"$REAL_TEE\" \"$@\""
            },
        ),
    ] {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(run.join("go"), "").unwrap();
    for name in [".bashrc", ".bash_profile", ".profile"] {
        fs::write(home.join(name), format!("export ASCII_TOKEN=DO_NOT_READ_RC\nexport BOAT_ID=DO_NOT_READ_RC\ntouch '{}'/rc-read\n", home.display())).unwrap();
    }
    let script = include_str!("run.sh").replace("/home/user", home.to_str().unwrap());
    let mut command = Command::new("bash");
    command
        .env_clear()
        .env("HOME", &home)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("REAL_TEE", tee.trim())
        .args(["--noprofile", "--norc", "-c"])
        .arg(script)
        .args([
            "test",
            tree.trim(),
            "",
            "4",
            "11111111111111111111111111111111",
            "0",
            "unused",
            "bash",
            "-c",
            "printf 'stdout-tail\\n'; printf 'stderr-tail\\n' >&2",
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
