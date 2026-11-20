//! The `agent-bench` binary: every subcommand's output and exit status (0 done, 1 refused,
//! 2 usage error).

use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = include_str!("../src/fixtures/config.json");
const TASK: &str = include_str!("../src/fixtures/task.json");

fn bench(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-bench"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Writes `text` with `edit` applied to a file and stores it, returning the hash.
fn record(store: &Path, name: &str, text: &str, edit: impl Fn(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(text).unwrap();
    edit(&mut value);
    let file = store.parent().unwrap().join(name);
    std::fs::write(&file, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let output = bench(&["record", "--store", path(store), path(&file)]);
    assert_eq!(code(&output), 0, "{output:?}");
    let line = stdout(&output);
    let (_, hash) = line.trim().split_once(' ').unwrap();
    hash.to_owned()
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        &[][..],
        &["frobnicate"],
        &["record", "file.json"],
        &["record", "--store"],
        &["config", "diff", "--store", "s", "a"],
        &["pool", "declare", "--store", "s", "--kind", "config"],
        &["workspace", "check", "dir"],
        // The task group goes together.
        &[
            "workspace",
            "check",
            "dir",
            "--object-set",
            "x",
            "--task",
            "t",
        ],
        &[
            "workspace",
            "check",
            "dir",
            "--object-set",
            "x",
            "--task",
            "t",
            "--task-env",
            "e",
        ],
        &[
            "workspace",
            "check",
            "dir",
            "--object-set",
            "x",
            "--private-layer",
            "p",
        ],
        &["tree-hash"],
        &["task-env", "warm", "--agent-jail", "j"],
        &[
            "workspace",
            "create",
            "--source",
            "s",
            "--base",
            "b",
            "--out",
            "o",
            "--bogus",
            "x",
        ],
    ] {
        let output = bench(args);
        assert_eq!(code(&output), 2, "{args:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
    }
    let help = bench(&["help"]);
    assert_eq!(code(&help), 0);
    for command in [
        "agent-bench workspace check",
        "--private-layer DIR",
        "agent-bench tree-hash DIR",
        "agent-bench fingerprints",
        "agent-bench task-env warm",
    ] {
        assert!(stdout(&help).contains(command), "{command}");
    }
}

#[test]
fn tree_hash_prints_the_digest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f"), "x").unwrap();
    let output = bench(&["tree-hash", path(dir.path())]);
    assert_eq!(code(&output), 0, "{output:?}");
    let digest = stdout(&output);
    assert_eq!(digest.trim().len(), 64);
    assert!(digest.ends_with('\n'));
    let missing = bench(&["tree-hash", path(&dir.path().join("missing"))]);
    assert_eq!(code(&missing), 1);
}

#[test]
fn record_diff_and_pool() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");

    let config = record(&store, "config.json", CONFIG, |_| {});
    assert_eq!(config.len(), 64);
    assert!(
        store
            .join("configs")
            .join(format!("{config}.json"))
            .exists()
    );
    let task = record(&store, "task.json", TASK, |_| {});

    // A record missing a field is refused with exit 1 and nothing stored.
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, CONFIG.replace("\"effort\"", "\"effort_\"")).unwrap();
    let refused = bench(&["record", "--store", path(&store), path(&broken)]);
    assert_eq!(code(&refused), 1);
    assert!(String::from_utf8_lossy(&refused.stderr).contains("broken.json"));
    assert_eq!(std::fs::read_dir(store.join("configs")).unwrap().count(), 1);

    // config diff by 12-digit prefixes prints one line per changed path.
    let moved = record(&store, "moved.json", CONFIG, |v| {
        v["bench"]["commit"] = serde_json::json!("3".repeat(40));
        v["tools"]["rtk"] = serde_json::json!({"enabled": false, "version": null});
    });
    let output = bench(&[
        "config",
        "diff",
        "--store",
        path(&store),
        &config[..12],
        &moved[..12],
    ]);
    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output),
        format!(
            "bench.commit: \"{}\" -> \"{}\"\ntools.rtk.enabled: true -> false\ntools.rtk.version: \"0.9.0\" -> null\n",
            "1".repeat(40),
            "3".repeat(40)
        )
    );
    let same = bench(&["config", "diff", "--store", path(&store), &config, &config]);
    assert_eq!((code(&same), stdout(&same)), (0, String::new()));
    let unknown = bench(&["config", "diff", "--store", path(&store), "abcdef", &config]);
    assert_eq!(code(&unknown), 1);

    // A pooling declaration outside the allowed fields is refused; an allowed one is written.
    let declare = |new: &str| {
        bench(&[
            "pool",
            "declare",
            "--store",
            path(&store),
            "--kind",
            "config",
            "--pair",
            &config[..12],
            new,
            "--reason",
            "bench fix",
            "--signed-by",
            "eon-lead",
        ])
    };
    assert_eq!(code(&declare(&moved)), 1);
    let infra = record(&store, "infra.json", CONFIG, |v| {
        v["bench"]["commit"] = serde_json::json!("3".repeat(40))
    });
    let output = declare(&infra);
    assert_eq!(code(&output), 0, "{output:?}");
    let text = stdout(&output);
    assert!(text.starts_with("pooling "));
    assert!(text.contains(": bench.commit\n"));

    // pool show prints one object with no rows for an empty cell and exits 0.
    let output = bench(&[
        "pool",
        "show",
        "--store",
        path(&store),
        &infra[..12],
        &task[..12],
    ]);
    assert_eq!(
        (code(&output), stdout(&output)),
        (0, "{\"rows\":[],\"excluded\":[]}\n".to_owned())
    );
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn workspace_create_and_check() {
    let dir = tempfile::tempdir().unwrap();
    // Canonical: the check refuses private layers under a symlink, and macOS's temp dir is
    // behind one (`/var -> /private/var`).
    let root = dir.path().canonicalize().unwrap();
    let source = root.as_path().join("source");
    std::fs::create_dir_all(source.join(".beads")).unwrap();
    git(&source, &["init", "-q"]);
    std::fs::write(source.join("a.txt"), "a\n").unwrap();
    std::fs::write(source.join(".beads/issues.jsonl"), "{}\n").unwrap();
    git(&source, &["add", "-A"]);
    git(&source, &["commit", "-q", "-m", "one"]);

    let out = root.as_path().join("ws");
    let output = bench(&[
        "workspace",
        "create",
        "--source",
        path(&source),
        "--base",
        "HEAD",
        "--out",
        path(&out),
    ]);
    assert_eq!(code(&output), 0, "{output:?}");
    let snapshot: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(snapshot["branch"], "main");
    let object_set = snapshot["object_set_sha256"].as_str().unwrap().to_owned();

    let output = bench(&[
        "workspace",
        "check",
        path(&out),
        "--object-set",
        &object_set,
    ]);
    assert_eq!((code(&output), stdout(&output)), (0, "ok\n".to_owned()));

    // With the task group, an empty task environment fails the task-environment checks.
    let mut task: serde_json::Value = serde_json::from_str(TASK).unwrap();
    task["snapshot_object_set_sha256"] = serde_json::json!(object_set);
    let task_file = root.as_path().join("task.json");
    std::fs::write(&task_file, serde_json::to_vec(&task).unwrap()).unwrap();
    let layer = root.as_path().join("upper");
    std::fs::create_dir(&layer).unwrap();
    let output = bench(&[
        "workspace",
        "check",
        path(&out),
        "--object-set",
        &object_set,
        "--task",
        path(&task_file),
        "--task-env",
        path(&root.as_path().join("task-env")),
        "--private-layer",
        path(&layer),
    ]);
    assert_eq!(code(&output), 1, "{output:?}");
    let text = stdout(&output);
    for needle in ["env.json", "warm.json", "target:", "cargo:"] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    assert!(!text.contains("snapshot_object_set_sha256"), "{text}");
    assert!(!text.contains("private layer"), "{text}");

    // A clone of the source fails with its violations listed.
    let clone = root.as_path().join("clone");
    git(
        root.as_path(),
        &["clone", "-q", path(&source), path(&clone)],
    );
    let output = bench(&[
        "workspace",
        "check",
        path(&clone),
        "--object-set",
        &object_set,
    ]);
    assert_eq!(code(&output), 1);
    let text = stdout(&output);
    assert!(
        text.contains("config key not allowed: remote.origin.url"),
        "{text}"
    );
    assert!(text.contains("git checks skipped"), "{text}");

    // Creating over an existing directory is refused.
    let again = bench(&[
        "workspace",
        "create",
        "--source",
        path(&source),
        "--base",
        "HEAD",
        "--out",
        path(&out),
    ]);
    assert_eq!(code(&again), 1);
}
