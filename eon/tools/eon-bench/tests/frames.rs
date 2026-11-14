//! Failed preflight is evidence, and existing evidence is never overwritten.
#![cfg(target_os = "linux")]
use std::{fs, path::PathBuf, process::Command};

#[test]
fn unsupported_candidate_retains_failure_and_an_existing_run_is_never_reused() {
    let root = std::env::temp_dir().join(format!("frames-failed-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let out = root.join("out");
    let program = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("true"))
        .find(|p| p.is_file())
        .unwrap();
    let program = fs::canonicalize(program).unwrap();
    assert!(program.starts_with("/nix/store"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_eon-bench"));
    command
        .args([
            "frames",
            "--mode",
            "check",
            "--source-tree",
            &"0".repeat(40),
            "--eon",
            &format!("candidate={}", program.display()),
            "--revision",
            &format!("candidate={}", "0".repeat(40)),
            "--candidate-tree",
            &format!("candidate={}", "0".repeat(40)),
            "--out",
        ])
        .arg(&out);
    let failed = command.output().unwrap();
    assert!(!failed.status.success());
    let path: PathBuf = out.join("result.json");
    let evidence = fs::read_to_string(&path).unwrap();
    assert!(evidence.contains("\"result\": \"fail\""), "{evidence}");
    assert!(evidence.contains("no measurements"), "{evidence}");
    assert!(out.join("SHA256SUMS").is_file());
    assert!(!command.output().unwrap().status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), evidence);
    fs::remove_dir_all(root).unwrap();
}
