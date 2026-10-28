use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

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
            "session=planned",
            "launch=planned",
            "identity=planned",
            "retina=planned",
            "picker=planned",
            "output=planned",
            "panes=planned",
            "focus=planned",
            "edit=planned",
            "kitty=planned",
            "keys=planned",
            "views=planned",
            "detach=planned",
            "crash=planned",
            "adoption=planned",
            "stop=planned",
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

/// A final dry run consumes a store-like, read-only source archive and stdin,
/// without bundling the source, private roots or the caller's HOME.
#[test]
fn a_final_dry_run_archives_answers_and_preserves_the_source() {
    let scratch = std::env::temp_dir().join(format!("mac-proof-final-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let repository = scratch.join("repo");
    let source = scratch.join("source");
    let out = scratch.join("evidence");
    fs::create_dir_all(repository.join("eon/tools/mac-proof")).unwrap();
    fs::create_dir_all(&source).unwrap();
    for name in ["flake.nix", "Cargo.toml", "eon/tools/mac-proof/Cargo.toml"] {
        fs::write(repository.join(name), "fixture\n").unwrap();
    }
    fs::write(
        repository.join("private-home-secret"),
        "never bundle the source\n",
    )
    .unwrap();
    git(&repository, &["init", "--quiet"]);
    git(&repository, &["add", "--all"]);
    git(&repository, &["commit", "--quiet", "--message", "fixture"]);
    let tree = git(&repository, &["rev-parse", "HEAD^{tree}"]);
    let archive = Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["archive", "HEAD"])
        .output()
        .unwrap();
    assert!(archive.status.success());
    let mut tar = Command::new("tar")
        .arg("-xf")
        .arg("-")
        .arg("-C")
        .arg(&source)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    tar.stdin
        .take()
        .unwrap()
        .write_all(&archive.stdout)
        .unwrap();
    assert!(tar.wait().unwrap().success());
    fs::set_permissions(source.join("flake.nix"), fs::Permissions::from_mode(0o444)).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_mac-proof"))
        .args(["final", "--dry-run", "--out"])
        .arg(&out)
        .env("MAC_PROOF_SOURCE", &source)
        .env("MAC_PROOF_REV", "dirty")
        .current_dir(&scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"\nP\n\n\nf\nblurred text\n\nskip\n\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("final.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["tree"], tree);
    assert_eq!(manifest["rev"], format!("tree-{}", &tree[..12]));
    assert_eq!(manifest["dry_run"], true);
    assert_eq!(manifest["run_result"], "pass");
    let answers: Vec<_> = manifest["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["answer"].as_str().unwrap())
        .collect();
    assert_eq!(
        answers,
        [
            "pass",
            "fail",
            "skip",
            "unanswered",
            "unanswered",
            "unanswered",
            "unanswered",
            "unanswered"
        ]
    );
    assert_eq!(manifest["items"][1]["note"], "blurred text");
    assert!(
        manifest["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["capture"].is_null())
    );
    let listing = Command::new("tar")
        .arg("-tzf")
        .arg(scratch.join("evidence.tar.gz"))
        .output()
        .unwrap();
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    for required in [
        "final.json",
        "summary.txt",
        "versions.txt",
        "displays.json",
        "run/summary.json",
        "manual/final.log",
    ] {
        assert!(
            listing
                .lines()
                .any(|line| line == format!("evidence/{required}")),
            "{listing}"
        );
    }
    assert!(
        !listing.contains("private-home-secret")
            && !listing.contains("/src/")
            && !listing.contains("/home/")
            && !listing.contains(".git")
    );
    assert!(!source.join(".git").exists());
    assert_eq!(
        fs::metadata(source.join("flake.nix"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o444
    );
    let again = Command::new(env!("CARGO_BIN_EXE_mac-proof"))
        .args(["final", "--dry-run", "--out"])
        .arg(&out)
        .env("MAC_PROOF_SOURCE", &source)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(2));
    fs::remove_dir_all(scratch).unwrap();
}
