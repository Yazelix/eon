#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! Byte-exact output of the standalone `rota` host. The goldens under tests/fixtures/golden were
//! captured from the binary before the host seam (eon-eon-agent-lr8sx.39.1), updated for the
//! quota gate (`.39.2`: `--role lead`, the reading's `quota_problem` and `source`, the `lead`
//! status field, and unread candidates shown as not checked), and without the per-backend
//! `warnings` field (eon-0nn1p.27); the rota host must keep every byte. `eon/crates/eon/tests/agent.rs` holds the `eon agent` twins.

use std::{fs, path::Path, process::Command};

fn golden(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/golden")
            .join(name),
    )
    .unwrap()
}

/// A scratch root holding the golden configuration with `<ROOT>` filled in.
fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("state")).unwrap();
    fs::create_dir_all(root.join("home")).unwrap();
    let config = golden("config.toml").replace("<ROOT>", root.to_str().unwrap());
    fs::write(root.join("config.toml"), config).unwrap();
    fs::write(root.join("brief"), "brief text\n").unwrap();
    dir
}

fn rota(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rota"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg"))
        .env("ROTA_CONFIG", root.join("config.toml"))
        .current_dir(root)
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
fn rota_help_texts_are_unchanged() {
    let dir = root();
    let help = rota(dir.path(), &["--help"]);
    assert!(help.status.success());
    assert_eq!(
        String::from_utf8(help.stdout).unwrap(),
        golden("rota-help.txt")
    );
    let help = rota(dir.path(), &["retire", "--help"]);
    assert!(help.status.success());
    assert_eq!(
        String::from_utf8(help.stdout).unwrap(),
        golden("rota-retire-help.txt")
    );
}

#[test]
fn rota_status_dry_run_and_decision_record_are_unchanged() {
    let dir = root();
    let root = dir.path();
    let status = rota(root, &["status", "--json"]);
    assert!(status.status.success(), "{status:?}");
    assert_eq!(normalized(root, &status.stdout), golden("rota-status.json"));
    let dry = rota(root, &["launch", "--dry-run", "brief"]);
    assert!(dry.status.success(), "{dry:?}");
    assert_eq!(normalized(root, &dry.stdout), golden("rota-dry-run.stdout"));
    assert_eq!(normalized(root, &dry.stderr), golden("rota-dry-run.stderr"));
    // A real launch execs the fixture backend (`true`) and writes one decision record.
    let launch = rota(root, &["launch", "brief"]);
    assert!(launch.status.success(), "{launch:?}");
    let record = fs::read(root.join("state/decisions.jsonl")).unwrap();
    assert_eq!(normalized(root, &record), golden("rota-decision.jsonl"));
}

#[test]
fn rota_slot_delegate_reinvokes_rota_without_a_host_prefix() {
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
    let config = fs::read_to_string(root.join("config.toml")).unwrap();
    fs::write(
        root.join("config.toml"),
        format!(
            "{config}\n[slots]\nlocal_runner = ['{}']\n",
            runner.display()
        ),
    )
    .unwrap();
    let output = rota(root, &["slot", "run", "--", "sh", "-c", "exit 7"]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let argv = fs::read_to_string(root.join("argv")).unwrap();
    let argv: Vec<_> = argv.lines().collect();
    assert_eq!(argv[0], env!("CARGO_BIN_EXE_rota"));
    assert_eq!(argv[1], "_slot-local-exec");
    assert_eq!(argv[3..], ["--", "sh", "-c", "exit 7"]);
}

#[test]
fn rota_config_errors_name_rota() {
    let dir = root();
    let root = dir.path();
    let config = fs::read_to_string(root.join("config.toml")).unwrap();
    fs::write(
        root.join("config.toml"),
        format!("{config}\n[run]\nunset_env = ['XDG_RUNTIME_DIR']\n"),
    )
    .unwrap();
    let status = rota(root, &["status"]);
    assert!(!status.status.success());
    assert_eq!(
        String::from_utf8(status.stderr).unwrap(),
        "rota: [run] must keep XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS: rota run reaches the systemd user manager through them\n"
    );
}
