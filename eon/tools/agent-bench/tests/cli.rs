//! The `agent-bench` binary: every subcommand's output and exit status (0 done, 1 refused,
//! 2 usage error).

use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = include_str!("../src/fixtures/config.json");
const CONFIG_CLONE: &str = include_str!("../src/fixtures/config_clone.json");
const CONFIG_V1: &str = include_str!("../src/fixtures/config_v1.json");
const TASK: &str = include_str!("../src/fixtures/task.json");
const TRIAL: &str = include_str!("../src/fixtures/trial.json");
const LANES: &str = include_str!("../src/fixtures/lanes.json");

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
        &["trial", "check"],
        &["trial", "check", "trial.json"],
        &["trial", "check", "--lanes", "lanes.json"],
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
        "agent-bench trial check FILE --lanes FILE [--store DIR]",
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

#[test]
fn v1_config_is_refused_on_record_and_diffable_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");

    // `agent-bench record` refuses `eon-bench-config/1`.
    let v1_file = dir.path().join("config_v1.json");
    std::fs::write(&v1_file, CONFIG_V1).unwrap();
    let refused = bench(&["record", "--store", path(&store), path(&v1_file)]);
    assert_eq!(code(&refused), 1);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("eon-bench-config/1"), "{stderr}");
    assert!(stderr.contains("eon-bench-config/2"), "{stderr}");

    // Store a `/2` config and plant the historical `/1` canonical record directly in `configs/`.
    let v2_hash = record(&store, "config_v2.json", CONFIG, |_| {});
    let v1_hashed = agent_bench::record::hash(CONFIG_V1.as_bytes()).unwrap();
    std::fs::write(
        store
            .join("configs")
            .join(format!("{}.json", v1_hashed.hash)),
        &v1_hashed.canonical,
    )
    .unwrap();

    let diff_out = bench(&[
        "config",
        "diff",
        "--store",
        path(&store),
        &v1_hashed.hash[..12],
        &v2_hash[..12],
    ]);
    assert_eq!(code(&diff_out), 0, "{diff_out:?}");
    assert_eq!(
        stdout(&diff_out),
        concat!(
            "containment: (absent) -> \"jail\"\n",
            "plan_review: (absent) -> \"none\"\n",
            "schema: \"eon-bench-config/1\" -> \"eon-bench-config/2\"\n",
        )
    );
}

#[test]
fn trial_check_expands_and_refuses_invalid_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let lanes_file = dir.path().join("lanes.json");
    std::fs::write(&lanes_file, LANES).unwrap();

    // 1. Happy path with inline configs and tasks (no --store needed).
    let trial_file = dir.path().join("trial.json");
    std::fs::write(&trial_file, TRIAL).unwrap();
    let ok = bench(&[
        "trial",
        "check",
        path(&trial_file),
        "--lanes",
        path(&lanes_file),
    ]);
    assert_eq!(code(&ok), 0, "{ok:?}");
    let out = stdout(&ok);
    assert!(out.ends_with('\n'));
    let plan: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(plan["schema"], "eon-bench-trial-plan/1");
    assert_eq!(plan["initial_runs"], 4);
    assert_eq!(plan["max_runs"], 8);
    let runs = plan["planned_runs"].as_array().unwrap();
    assert_eq!(runs.len(), 4);
    assert_eq!(
        runs[0]["config_hash"],
        "b599be3855e7f25d5fd06a39705bb85bf93a48e2c4cb017e2d9d20b092da1449"
    );
    assert_eq!(
        runs[0]["task_hash"],
        "4b99405a84bf4646a1dddc5013b03778d7e4bafd0dafd887fa4de6bb3154f430"
    );
    assert_eq!(runs[0]["repeat"], 1);
    assert_eq!(runs[0]["effective_wall_cap_s"], 5400);
    assert_eq!(runs[2]["repeat"], 2);

    // 2. Happy path resolving stored config + task hashes via `--store`.
    let store = dir.path().join("store");
    let clone_hash = record(&store, "clone.json", CONFIG_CLONE, |_| {});
    let task_hash = record(&store, "task.json", TASK, |_| {});
    let stored_trial_file = dir.path().join("stored_trial.json");
    let stored_trial = serde_json::json!({
        "schema": "eon-bench-trial/1",
        "name": "stored-clone",
        "role": "builder",
        "containment": "clone",
        "arms": [{
            "name": "arm-a",
            "lane": "lane-a",
            "config": &clone_hash[..12]
        }],
        "tasks": [&task_hash[..12]],
        "repeats": {"start": 1, "cap": 3},
        "pairing_window_hours": 72,
        "budget_caps": {"gemini-pro": {"week_pct": 20.0}}
    });
    std::fs::write(
        &stored_trial_file,
        serde_json::to_vec_pretty(&stored_trial).unwrap(),
    )
    .unwrap();
    let stored_ok = bench(&[
        "trial",
        "check",
        path(&stored_trial_file),
        "--lanes",
        path(&lanes_file),
        "--store",
        path(&store),
    ]);
    assert_eq!(code(&stored_ok), 0, "{stored_ok:?}");
    let stored_plan: serde_json::Value = serde_json::from_str(&stdout(&stored_ok)).unwrap();
    assert_eq!(stored_plan["planned_runs"][0]["config_hash"], clone_hash);
    assert_eq!(stored_plan["planned_runs"][0]["task_hash"], task_hash);

    // 3. Refusal cases: exit 1, clear stderr, empty stdout.
    let base_trial: serde_json::Value = serde_json::from_str(TRIAL).unwrap();
    let jail_config: serde_json::Value = serde_json::from_str(CONFIG).unwrap();

    let mut missing_field = base_trial.clone();
    missing_field
        .as_object_mut()
        .unwrap()
        .remove("pairing_window_hours");

    let mut clone_with_jail_digest = base_trial.clone();
    clone_with_jail_digest["arms"][0]["config"]["tools"]["jail_profile_sha256"] =
        serde_json::json!("4".repeat(64));

    let mut jail_without_digest = base_trial.clone();
    jail_without_digest["containment"] = serde_json::json!("jail");
    let mut bad_jail_cfg = jail_config.clone();
    bad_jail_cfg["tools"]["jail_profile_sha256"] = serde_json::Value::Null;
    jail_without_digest["arms"] = serde_json::json!([{
        "name": "jail-arm",
        "lane": "lane-a",
        "config": bad_jail_cfg
    }]);

    let mut unapproved_containment = base_trial.clone();
    unapproved_containment["containment"] = serde_json::json!("jail");
    unapproved_containment["arms"] = serde_json::json!([{
        "name": "jail-on-clone-lane",
        "lane": "lane-b",
        "config": jail_config
    }]);

    for (name, value, needle) in [
        ("missing_field.json", missing_field, "pairing_window_hours"),
        (
            "clone_with_jail_digest.json",
            clone_with_jail_digest,
            "must be null for clone containment",
        ),
        (
            "jail_without_digest.json",
            jail_without_digest,
            "required non-null for jail containment",
        ),
        (
            "unapproved_containment.json",
            unapproved_containment,
            "not approved for jail",
        ),
    ] {
        let file = dir.path().join(name);
        std::fs::write(&file, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let output = bench(&["trial", "check", path(&file), "--lanes", path(&lanes_file)]);
        assert_eq!(code(&output), 1, "{name}: {output:?}");
        assert!(stdout(&output).is_empty(), "{name}: unexpected stdout");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{name}: stderr was {stderr:?}");
    }
}
