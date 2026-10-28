use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["-c", "user.name=test", "-c", "user.email=test@localhost"])
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A dry run through a local host sends the candidate tree, untracked and
/// ignored-but-tracked files included, proves it arrived intact and brings
/// the evidence back, without touching the checkout's index.
#[test]
fn a_local_dry_run_round_trips_the_candidate_tree() {
    let scratch = std::env::temp_dir().join(format!("mac-proof-local-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (repository, home, out) = (
        scratch.join("repo"),
        scratch.join("home"),
        scratch.join("out"),
    );
    fs::create_dir_all(repository.join("bin")).unwrap();
    fs::create_dir_all(&home).unwrap();
    git(&repository, &["init", "--quiet"]);
    fs::write(repository.join(".gitignore"), "*.log\n").unwrap();
    fs::write(repository.join("kept.log"), "tracked though ignored\n").unwrap();
    fs::write(repository.join("bin/tool"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(
        repository.join("bin/tool"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(
        &repository,
        &["add", "--force", ".gitignore", "kept.log", "bin/tool"],
    );
    git(&repository, &["commit", "--quiet", "--message", "base"]);
    fs::write(repository.join("new.txt"), "untracked\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mac-proof"))
        .args(["remote", "--local"])
        .arg(&home)
        .arg("--out")
        .arg(&out)
        .arg("--dry-run")
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(git(&repository, &["status", "--porcelain"]), "?? new.txt");

    git(&repository, &["add", "--all"]);
    let tree = git(&repository, &["write-tree"]);
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["result"], "pass");
    assert_eq!(summary["tree"], tree.as_str());
    assert_eq!(summary["dry_run"], true);
    let statuses: Vec<_> = summary["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            format!(
                "{}={}",
                step["name"].as_str().unwrap(),
                step["status"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            "host=planned",
            "source=pass",
            "package=planned",
            "terminfo=planned",
            "check-list=planned",
            "workspace=planned",
            "leftovers=planned"
        ]
    );
    assert!(
        fs::read_to_string(out.join("remote.log"))
            .unwrap()
            .contains(&format!("tree: {tree}"))
    );
    assert!(out.join("summary.txt").is_file() && out.join("steps/02-source.log").is_file());
    let runs: Vec<_> = fs::read_dir(home.join("eon-proof")).unwrap().collect();
    assert_eq!(runs.len(), 1);
    fs::remove_dir_all(scratch).unwrap();
}

/// A host that refuses the source ends the transfer at once, however large
/// the archive, instead of waiting out the transfer deadline.
#[test]
fn a_host_that_refuses_the_source_fails_the_round_trip_at_once() {
    let scratch = std::env::temp_dir().join(format!("mac-proof-refused-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (repository, home, out) = (
        scratch.join("repo"),
        scratch.join("home"),
        scratch.join("out"),
    );
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&home).unwrap();
    // A file where the run directories go makes the host's mkdir fail.
    fs::write(home.join("eon-proof"), "").unwrap();
    git(&repository, &["init", "--quiet"]);
    fs::write(repository.join("large"), vec![b'x'; 4 << 20]).unwrap();
    git(&repository, &["add", "large"]);
    git(&repository, &["commit", "--quiet", "--message", "base"]);

    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_mac-proof"))
        .args(["remote", "--local"])
        .arg(&home)
        .arg("--out")
        .arg(&out)
        .arg("--dry-run")
        .current_dir(&repository)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(started.elapsed().as_secs() < 60);
    assert!(String::from_utf8_lossy(&output.stderr).contains("transfer failed"));
    fs::remove_dir_all(scratch).unwrap();
}
