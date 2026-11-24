use serde_json::Value;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_eon"))
        .env_clear()
        .env("HOME", root)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("EON_CONFIG_HOME", root.join("config"))
        .env("EON_RUNTIME_DIR", root.join("runtime"))
        .current_dir(root)
        .arg("tutor")
        .args(arguments)
        .output()
        .unwrap()
}

fn json(root: &Path, arguments: &[&str]) -> Value {
    let output = run(root, arguments);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn tutor_navigation_json_and_reset_preserve_other_work() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let list = json(root, &["list", "--json"]);
    assert_eq!(list["schema_version"], 1);
    assert_eq!(list["current_lesson"], 1);
    assert_eq!(list["lessons"].as_array().unwrap().len(), 10);
    assert!(
        !root.join("state").exists(),
        "list must not create progress"
    );
    assert_eq!(json(root, &["--json"])["lessons"][0]["read"], true);
    let jumped = json(root, &["--json", "5"]);
    assert_eq!(jumped["current_lesson"], 5);
    assert_eq!(jumped["lessons"][1]["read"], false);
    assert_eq!(jumped["lessons"][4]["read"], true);
    assert_eq!(jumped["check_available"], false);
    assert!(jumped["check_result"].is_null());
    assert_eq!(json(root, &["next", "--json"])["current_lesson"], 6);
    assert_eq!(
        json(root, &["10", "--json"])["lessons"][9]["checkable"],
        false
    );
    assert_eq!(json(root, &["next", "--json"])["current_lesson"], 10);
    let state = root.join("state/eon/tutor-v1.json");
    let before = fs::read(&state).unwrap();
    json(root, &["list", "--json"]);
    assert_eq!(fs::read(&state).unwrap(), before);
    fs::write(root.join("practice.txt"), "keep my work").unwrap();
    fs::write(root.join("state/eon/discovery.json"), "seen").unwrap();
    let reset = json(root, &["--reset", "--json"]);
    assert_eq!(reset["current_lesson"], 1);
    for (index, lesson) in reset["lessons"].as_array().unwrap().iter().enumerate() {
        assert_eq!(lesson["read"], index == 0);
        assert_eq!(lesson["verified"], false);
    }
    assert_eq!(
        fs::read_to_string(root.join("practice.txt")).unwrap(),
        "keep my work"
    );
    assert_eq!(
        fs::read_to_string(root.join("state/eon/discovery.json")).unwrap(),
        "seen"
    );
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(state.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert!(!root.join("runtime").exists());
    assert!(!root.join("config").exists());
}

#[test]
fn tutor_refuses_bad_arguments_and_state_without_overwriting() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    for args in [
        vec!["0"],
        vec!["11"],
        vec!["-1"],
        vec!["+1"],
        vec!["1", "2"],
        vec!["next", "list"],
        vec!["--reset", "next"],
        vec!["--json", "--json"],
        vec!["check"],
        vec!["unknown"],
    ] {
        let mut args = args;
        args.push("--json");
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(error["error"]["code"].is_string());
        assert!(!root.join("state").exists());
    }
    let first = json(root, &["--json"]);
    let state = root.join("state/eon/tutor-v1.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    saved["verified"] = serde_json::json!([first["lessons"][0]["id"]]);
    fs::write(&state, serde_json::to_vec(&saved).unwrap()).unwrap();
    assert_eq!(
        json(root, &["next", "--json"])["lessons"][0]["verified"],
        true
    );
    let mut oversized = serde_json::to_vec(&saved).unwrap();
    oversized.extend([b' '; 16385]);
    let mut newer = saved.clone();
    newer["version"] = serde_json::json!(2);
    let newer = serde_json::to_vec(&newer).unwrap();
    for bad in [b"not JSON".as_slice(), &newer, &oversized] {
        fs::write(&state, bad).unwrap();
        let output = run(root, &["--reset", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(&state).unwrap(), bad);
    }
}

#[test]
fn tutor_rejects_symlinks_permissions_and_contended_locks() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    json(root, &["--json"]);
    let state = root.join("state/eon/tutor-v1.json");
    let before = fs::read(&state).unwrap();
    let victim = root.join("victim");
    fs::write(&victim, "untouched").unwrap();
    fs::remove_file(&state).unwrap();
    symlink(&victim, &state).unwrap();
    assert_eq!(run(root, &["next", "--json"]).status.code(), Some(1));
    assert_eq!(fs::read_to_string(&victim).unwrap(), "untouched");
    fs::remove_file(&state).unwrap();
    fs::write(&state, &before).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(run(root, &["next", "--json"]).status.code(), Some(1));
    assert_eq!(fs::read(&state).unwrap(), before);
    fs::set_permissions(&state, fs::Permissions::from_mode(0o600)).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("state/eon/tutor-v1.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(run(root, &["next", "--json"]).status.code(), Some(1));
    assert_eq!(fs::read(&state).unwrap(), before);
    drop(lock);
    let old_directory = root.join("previous");
    fs::rename(root.join("state/eon"), &old_directory).unwrap();
    let redirected = root.join("redirected");
    fs::create_dir(&redirected).unwrap();
    fs::set_permissions(&redirected, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(redirected.join("tutor-v1.json"), &before).unwrap();
    fs::set_permissions(
        redirected.join("tutor-v1.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    symlink(&redirected, root.join("state/eon")).unwrap();
    assert_eq!(run(root, &["--reset", "--json"]).status.code(), Some(1));
    assert_eq!(fs::read(redirected.join("tutor-v1.json")).unwrap(), before);
    assert_eq!(
        fs::read(old_directory.join("tutor-v1.json")).unwrap(),
        before
    );
}

#[test]
fn tutor_accepts_owned_readable_directories_but_rejects_shared_writes() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let directory = root.join("state/eon");
    fs::create_dir_all(&directory).unwrap();
    for mode in [0o755, 0o750, 0o711] {
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        json(root, &["next", "--json"]);
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            mode
        );
        assert_eq!(
            fs::metadata(directory.join("tutor-v1.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let before = fs::read(directory.join("tutor-v1.json")).unwrap();
    for mode in [0o775, 0o777] {
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        let output = run(root, &["next", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains(directory.to_str().unwrap()), "{message}");
        assert!(message.contains("chmod go-w"), "{message}");
        assert_eq!(fs::read(directory.join("tutor-v1.json")).unwrap(), before);
    }
}

#[test]
fn tutor_does_not_take_over_eonbase() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("eonbase");
    symlink(env!("CARGO_BIN_EXE_eon"), &base).unwrap();
    let output = Command::new(base)
        .env_clear()
        .env("HOME", root.path())
        .env("EON_RUNTIME_DIR", root.path().join("runtime"))
        .args(["tutor", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!root.path().join(".local").exists());
}

#[test]
fn initial_picker_header_claims_once_and_only_with_ready_context() {
    const COMMAND: [&str; 2] = ["--eon-initial-picker-header", "initial"];
    const PEER: &str = "1 other Eon window is still running";
    const COMBINED: &str = "1 other Eon window is still running\nNew to Eon? Run eon tutor.";
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let ready = root.join("terminal.quick");
    let state = root.join("state/eon/tutor-discovery-v1.seen");
    let eon = Path::new(env!("CARGO_BIN_EXE_eon"));
    let eonbase = root.join("eonbase");
    symlink(eon, &eonbase).unwrap();
    let command = |program: &Path| {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("HOME", root)
            .env("XDG_STATE_HOME", root.join("state"))
            .env("EON_FILES_START", "quick")
            .env("EON_QUICK_TOKEN", "initial")
            .env("EON_QUICK_MARKER", &ready)
            .env("EON_FILES_NOTICE", PEER)
            .stdin(Stdio::null());
        command
    };
    assert_eq!(
        command(eon).args(COMMAND).output().unwrap().status.code(),
        Some(2)
    );
    fs::write(&ready, "").unwrap();
    assert_eq!(
        command(eon).args(COMMAND).output().unwrap().status.code(),
        Some(2)
    );
    fs::write(&ready, "initial").unwrap();
    for output in [
        command(&eonbase).args(COMMAND).output().unwrap(),
        command(eon).args(COMMAND).arg("extra").output().unwrap(),
        command(eon)
            .args(COMMAND)
            .env_remove("EON_FILES_NOTICE")
            .output()
            .unwrap(),
        command(eon)
            .args(COMMAND)
            .env("EON_QUICK_TOKEN", "later")
            .output()
            .unwrap(),
    ] {
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
    for args in [&["--help"][..], &["tutor", "--help"]] {
        let help = command(eon).args(args).output().unwrap();
        assert!(help.status.success(), "{help:?}");
        assert!(!String::from_utf8_lossy(&help.stdout).contains(COMMAND[0]));
    }
    assert!(!state.exists());
    let first = command(eon)
        .args(COMMAND)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let second = command(eon).args(COMMAND).output().unwrap();
    let first = first.wait_with_output().unwrap();
    let mut headers = [first, second].map(|output| {
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    });
    headers.sort();
    assert_eq!(headers, [PEER, COMBINED]);
    let metadata = fs::symlink_metadata(&state).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.len(), 0);

    fs::remove_file(&state).unwrap();
    let victim = root.join("unrelated");
    fs::write(&victim, "preserve").unwrap();
    symlink(&victim, &state).unwrap();
    let output = command(eon).args(COMMAND).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, PEER.as_bytes());
    assert_eq!(fs::read_link(&state).unwrap(), victim);
    assert_eq!(fs::read_to_string(victim).unwrap(), "preserve");
}

#[test]
fn tutor_prints_every_complete_lesson_within_the_budget_offline() {
    let root = tempfile::tempdir().unwrap();
    for number in 1..=10 {
        let output = run(root.path(), &[&number.to_string()]);
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.lines().count() <= 20, "{number}: {text}");
        assert!(
            text.lines().all(|line| line.chars().count() <= 44),
            "{number}: {text}"
        );
        assert!(text.contains("eon tutor next"));
        assert!(!text.contains("eon tutor check"));
    }
    let help = run(root.path(), &["--help"]);
    assert!(help.status.success());
    assert!(!String::from_utf8(help.stdout).unwrap().contains(" check"));
}
