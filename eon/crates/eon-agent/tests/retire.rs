#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(projects: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("agents");
        fs::create_dir(&root).unwrap();
        let config = temp.path().join("config.toml");
        let base = temp.path().display();
        let text = format!(
            "state_file = '{base}/state/last'\nlog_file = '{base}/state/log'\n\
             [[backends]]\nname = 'fake'\nroles = []\nguard = ['true']\n\
             allow_unknown_quota = true\n[backends.commands]\n\
             medium = ['unused', '{{prompt}}']\nhigh = ['unused', '{{prompt}}']\n\
             {projects}"
        );
        fs::write(&config, text.replace("/FIXTURE", &base.to_string())).unwrap();
        Self {
            _temp: temp,
            root,
            config,
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rota"))
            .env("ROTA_CONFIG", &self.config)
            .args(args)
            .output()
            .unwrap()
    }
}

const PROJECT: &str = "[projects.alpha]\nrepo = '/FIXTURE'\nagents_root = '/FIXTURE/agents'\n\
    [projects.alpha.done]\ncommand = ['false', '{bead}']\npointers = ['/status']\nequals = 'closed'\n";

#[test]
fn retire_dispatch_help_projects_and_names() {
    let f = Fixture::new(PROJECT);
    let output = f.run(&["--help"]);
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("rota retire NAME"));
    assert!(help.contains("rota slot run"));
    assert!(help.contains("rota slot ls"));
    let output = f.run(&["retire", "--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("resolved"));
    assert!(help.contains("cwd directory identity"));
    assert!(help.contains("Bind-mount aliases"));
    assert!(help.contains("unsupported"));
    let output = f.run(&["retire", "a", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["paths"], serde_json::json!([]));
    assert_eq!(json["freed_bytes"], 0);
    for args in [
        vec!["retire", "../outside"],
        vec!["retire", "a/b"],
        vec!["retire", ".hidden"],
        vec!["retire", "a", "--project", "unknown"],
        vec!["retire", "a", "--bead"],
    ] {
        assert_eq!(f.run(&args).status.code(), Some(2), "{args:?}");
    }
    let f = Fixture::new(&format!(
        "{PROJECT}\n[projects.beta]\nrepo = '/FIXTURE'\nagents_root = '/FIXTURE/agents'\n"
    ));
    assert_eq!(f.run(&["retire", "a"]).status.code(), Some(2));
    assert_eq!(
        f.run(&["retire", "a", "--project", "alpha"]).status.code(),
        Some(0)
    );
}

#[test]
fn failed_done_check_keeps_everything_and_emits_json_in_both_modes() {
    let f = Fixture::new(PROJECT);
    let target = f.root.join("a-target");
    fs::create_dir(&target).unwrap();
    fs::write(
        target.join("CACHEDIR.TAG"),
        b"Signature: 8a477f597d28d172789f06886806bc55",
    )
    .unwrap();
    fs::write(target.join("output"), b"build").unwrap();
    let work = f.root.join("a-work");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("evidence"), b"proof").unwrap();
    symlink("/fictional/store/output", work.join("store")).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o555)).unwrap();
    for target_only in [false, true] {
        let mut args = vec!["retire", "a", "--bead", "task-1", "--json", "--dry-run"];
        if target_only {
            args.push("--target-only");
        }
        let output = f.run(&args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["freed_bytes"], 0);
        assert_eq!(json["dry_run"], true);
        assert!(
            json["error"]
                .as_str()
                .is_some_and(|s| s.contains("check failed"))
        );
        let paths = json["paths"].as_array().unwrap();
        assert!(paths.iter().all(|p| p["outcome"] == "kept"));
        assert!(
            paths
                .iter()
                .any(|p| p["reason"].as_str().is_some_and(|s| s.contains("failed")))
        );
        assert_eq!(fs::read(target.join("output")).unwrap(), b"build");
        assert_eq!(fs::read(work.join("evidence")).unwrap(), b"proof");
        assert!(work.join("store").is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o555
        );
    }
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn directory_scan_errors_exit_one_and_keep_the_target() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let f = Fixture::new(PROJECT);
    let target = f.root.join("a-target");
    fs::create_dir(&target).unwrap();
    fs::write(
        target.join("CACHEDIR.TAG"),
        b"Signature: 8a477f597d28d172789f06886806bc55",
    )
    .unwrap();
    fs::write(target.join("output"), b"keep").unwrap();
    fs::create_dir(target.join("unreadable")).unwrap();
    fs::set_permissions(target.join("unreadable"), fs::Permissions::from_mode(0o0)).unwrap();
    let output = f.run(&["retire", "a", "--target-only", "--json"]);
    fs::set_permissions(target.join("unreadable"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["error"].is_string());
    assert_eq!(fs::read(target.join("output")).unwrap(), b"keep");
    assert!(target.join("CACHEDIR.TAG").exists());
}
