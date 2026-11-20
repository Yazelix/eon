//! The task environment's half of the launched-workspace check, and the warm build.
//!
//! A task environment (made by `agent-jail task-env`) holds `env.json` (written by agent-jail;
//! its SHA-256 is task.json `env.env_sha256`), the offline Cargo home `cargo/`, the warm target
//! cache `target/`, and `warm.json`, which [`warm`] writes once the warm build has exited 0.
//! A run mounts `cargo/` and `target/` as overlay lower layers, with private upper and work
//! layers that must start empty.
//!
//! [`check`] verifies all of that against the task record before a run starts. [`warm`] runs
//! `agent-jail command … --template build --base-rw` to fill `target/` and `cargo/`, then
//! records their tree hashes ([`crate::tree`]) in `warm.json`.

use crate::record::{Digest, Task, Text};
use crate::{Result, fail, lock, sha256_hex, tree, workspace, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const WARM_SCHEMA: &str = "agent-bench-warm/1";

/// `<task-env>/warm.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Warm {
    pub schema: Text,
    pub target_sha256: Digest,
    /// Integrity only: Cargo's `.global-cache` last-use database and the git checkouts'
    /// `.git/{config,index,logs}` differ from one warm build to the next.
    pub cargo_sha256: Digest,
    /// The workspace the warm build ran in.
    pub object_set_sha256: Digest,
}

/// The part of agent-jail's `env.json` this crate reads; everything else is agent-jail's.
#[derive(Deserialize)]
struct EnvJson {
    exclude: Vec<String>,
    pass: Vec<String>,
    test: EnvTest,
}

#[derive(Deserialize)]
struct EnvTest {
    command: Vec<String>,
    build: Vec<String>,
}

/// Checks the task environment `task_env` for a run of `task` in a workspace with object set
/// `object_set`, and its private overlay layers. Returns one line per violation.
pub fn check(
    task: &Task,
    task_env: &Path,
    object_set: &str,
    private_layers: &[PathBuf],
) -> Vec<String> {
    let mut violations = Vec::new();
    let expected_set = task.snapshot_object_set_sha256.as_str();
    if object_set != expected_set {
        violations.push(format!(
            "--object-set {object_set} is not task.json snapshot_object_set_sha256 {expected_set}"
        ));
    }

    let env_path = task_env.join("env.json");
    match fs::read(&env_path) {
        Err(error) => violations.push(format!("{}: {error}", env_path.display())),
        Ok(bytes) => {
            let digest = sha256_hex(&bytes);
            if digest != task.env.env_sha256.as_str() {
                violations.push(format!(
                    "env.json hashes to {digest}, not task.json env.env_sha256 {}",
                    task.env.env_sha256
                ));
            }
            match serde_json::from_slice::<EnvJson>(&bytes) {
                Err(error) => violations.push(format!("env.json: {error}")),
                Ok(env) => {
                    for (name, ours, theirs) in [
                        ("command", &task.test.command, &env.test.command),
                        ("build", &task.test.build, &env.test.build),
                    ] {
                        if !ours
                            .iter()
                            .map(Text::as_str)
                            .eq(theirs.iter().map(String::as_str))
                        {
                            violations.push(format!(
                                "env.json test.{name} {theirs:?} is not task.json's {:?}",
                                ours.iter().map(Text::as_str).collect::<Vec<_>>()
                            ));
                        }
                    }
                    for (name, ours, theirs) in [
                        ("exclude", &task.env.exclude, &env.exclude),
                        ("pass", &task.env.pass, &env.pass),
                    ] {
                        let mut ours: Vec<&str> = ours.iter().map(Text::as_str).collect();
                        let mut theirs: Vec<&str> = theirs.iter().map(String::as_str).collect();
                        ours.sort_unstable();
                        theirs.sort_unstable();
                        if ours != theirs {
                            violations.push(format!(
                                "env.json {name} {theirs:?} is not task.json env.{name} {ours:?}"
                            ));
                        }
                    }
                }
            }
        }
    }

    let warm = match read_warm(task_env) {
        Ok(warm) => Some(warm),
        Err(error) => {
            violations.push(error.0);
            None
        }
    };
    if let Some(warm) = &warm
        && warm.object_set_sha256.as_str() != object_set
    {
        violations.push(format!(
            "warm.json object_set_sha256 {} is not --object-set {object_set}",
            warm.object_set_sha256
        ));
    }
    match tree::hash(&task_env.join("target")) {
        Err(error) => violations.push(format!("target: {}", error.0)),
        Ok(target) => {
            if target != task.env.warm_target_sha256.as_str() {
                violations.push(format!(
                    "target tree hashes to {target}, not task.json env.warm_target_sha256 {}",
                    task.env.warm_target_sha256
                ));
            }
            if let Some(warm) = &warm
                && target != warm.target_sha256.as_str()
            {
                violations.push(format!(
                    "target tree hashes to {target}, not warm.json target_sha256 {}",
                    warm.target_sha256
                ));
            }
        }
    }
    match tree::hash(&task_env.join("cargo")) {
        Err(error) => violations.push(format!("cargo: {}", error.0)),
        Ok(cargo) => {
            if let Some(warm) = &warm
                && cargo != warm.cargo_sha256.as_str()
            {
                violations.push(format!(
                    "cargo tree hashes to {cargo}, not warm.json cargo_sha256 {}",
                    warm.cargo_sha256
                ));
            }
        }
    }

    for layer in private_layers {
        let shown = layer.display();
        if let Some(violation) = linked_ancestor(layer) {
            violations.push(format!("private layer {shown}: {violation}"));
        }
        match fs::symlink_metadata(layer) {
            Err(error) => violations.push(format!("private layer {shown}: {error}")),
            Ok(meta) if meta.file_type().is_symlink() => {
                violations.push(format!("private layer {shown} is a symlink"))
            }
            Ok(meta) if !meta.is_dir() => {
                violations.push(format!("private layer {shown} is not a directory"))
            }
            Ok(_) => match fs::read_dir(layer) {
                Err(error) => violations.push(format!("private layer {shown}: {error}")),
                Ok(mut entries) => {
                    if entries.next().is_some() {
                        violations.push(format!("private layer {shown} is not empty"));
                    }
                }
            },
        }
    }
    violations
}

/// The first ancestor of `path` that is a symlink or cannot be looked at (a missing one is
/// left to the caller), as a violation.
fn linked_ancestor(path: &Path) -> Option<String> {
    let path = match std::path::absolute(path) {
        Ok(path) => path,
        Err(error) => return Some(error.to_string()),
    };
    path.ancestors()
        .skip(1)
        .find_map(|ancestor| match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                Some(format!("{} is a symlink", ancestor.display()))
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Some(format!("{}: {error}", ancestor.display()))
            }
            _ => None,
        })
}

fn read_warm(task_env: &Path) -> Result<Warm> {
    let path = task_env.join("warm.json");
    let bytes =
        fs::read(&path).map_err(|error| crate::Error(format!("{}: {error}", path.display())))?;
    let warm: Warm = serde_json::from_slice(&bytes)
        .map_err(|error| crate::Error(format!("warm.json: {error}")))?;
    if warm.schema.as_str() != WARM_SCHEMA {
        fail!("warm.json schema is {}, not {WARM_SCHEMA}", warm.schema);
    }
    Ok(warm)
}

/// The arguments of [`warm`].
pub struct WarmBuild<'a> {
    /// agent-jail, to which [`warm`] adds `command …`.
    pub agent_jail: Command,
    pub profile: &'a str,
    pub task_env: &'a Path,
    pub run: &'a Path,
    pub object_set: &'a str,
    pub wall_cap: &'a str,
}

/// Runs the warm build and writes `warm.json`, printing the workspace check's violations
/// (when it refuses) or the written `warm.json` to `out`.
pub fn warm(build: WarmBuild, out: &mut dyn Write) -> Result<Warm> {
    let object_set = Digest::new(build.object_set)?;
    let report = workspace::check(&build.run.join("workspace"), build.object_set)?;
    if !report.passed() {
        for violation in &report.violations {
            writeln!(out, "{violation}")?;
        }
        fail!(
            "the run workspace fails its check: {} violation(s)",
            report.violations.len()
        );
    }
    let warm_path = build.task_env.join("warm.json");
    let refuse_existing = || -> Result<()> {
        if fs::symlink_metadata(&warm_path).is_ok() {
            fail!(
                "{} exists: a task environment is warmed once",
                warm_path.display()
            );
        }
        Ok(())
    };
    refuse_existing()?;

    out.flush()?;
    let mut jail = build.agent_jail;
    let status = jail
        .arg("command")
        .arg(build.profile)
        .arg("--task-env")
        .arg(build.task_env)
        .arg("--run")
        .arg(build.run)
        .args([
            "--wall-cap",
            build.wall_cap,
            "--template",
            "build",
            "--base-rw",
        ])
        .stdin(Stdio::null())
        .status()
        .map_err(|error| {
            crate::Error(format!(
                "cannot run {}: {error}",
                Path::new(jail.get_program()).display()
            ))
        })?;
    if !status.success() {
        fail!("agent-jail command {status}; no warm.json written");
    }

    let _lock = lock(&build.task_env.join(".lock"))?;
    refuse_existing()?;
    let warm = Warm {
        schema: Text::new(WARM_SCHEMA)?,
        target_sha256: Digest::new(tree::hash(&build.task_env.join("target"))?)?,
        cargo_sha256: Digest::new(tree::hash(&build.task_env.join("cargo"))?)?,
        object_set_sha256: object_set,
    };
    let mut json = serde_json::to_string_pretty(&warm)?;
    json.push('\n');
    write_atomic(&warm_path, json.as_bytes())?;
    out.write_all(json.as_bytes())?;
    Ok(warm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::TASK;
    use crate::workspace::tests::source;
    use serde_json::json;
    use std::os::unix::fs::symlink;

    /// A fake agent-jail: a script that records its arguments in `<run>/args`, then runs
    /// `body`. `/bin/sh` reads it; nothing executes a file this test process wrote, so a
    /// sibling test's fork holding it open cannot fail the exec with ETXTBSY.
    fn fake_jail(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("agent-jail.sh");
        fs::write(
            &path,
            format!("printf '%s\\n' \"$@\" > \"$6/args\"\n{body}\n"),
        )
        .unwrap();
        path
    }

    struct Fixture {
        _source: tempfile::TempDir,
        _dir: tempfile::TempDir,
        /// The temp dir, canonical: the check refuses a private layer under a symlink, and a
        /// platform temp dir may sit behind one (`/var` is a link to `/private/var` on macOS).
        root: PathBuf,
        task_env: PathBuf,
        run: PathBuf,
        object_set: String,
    }

    /// A run whose workspace is a synthetic workspace, and an empty task environment.
    fn fixture() -> Fixture {
        fixture_in(&std::env::temp_dir())
    }

    fn fixture_in(base: &Path) -> Fixture {
        let source = source();
        let dir = tempfile::tempdir_in(base).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let run = root.join("run");
        fs::create_dir(&run).unwrap();
        let snapshot =
            workspace::create(source.path(), "HEAD", &run.join("workspace"), &[]).unwrap();
        let task_env = root.join("task-env");
        for sub in ["target", "cargo"] {
            fs::create_dir_all(task_env.join(sub)).unwrap();
        }
        Fixture {
            _source: source,
            _dir: dir,
            root,
            task_env,
            run,
            object_set: snapshot.object_set_sha256,
        }
    }

    fn run_warm(f: &Fixture, jail: &Path) -> (Result<Warm>, String) {
        let mut out = Vec::new();
        let mut agent_jail = Command::new("/bin/sh");
        agent_jail.arg(jail);
        let result = warm(
            WarmBuild {
                agent_jail,
                profile: "claude",
                task_env: &f.task_env,
                run: &f.run,
                object_set: &f.object_set,
                wall_cap: "30",
            },
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }

    const FILL: &str = "printf built > \"$4/target/out\"; mkdir \"$4/cargo/registry\"";

    #[test]
    fn a_failed_warm_build_writes_nothing() {
        let f = fixture();
        let jail = fake_jail(f.root.as_path(), "exit 1");
        let error = run_warm(&f, &jail).0.unwrap_err().0;
        assert!(error.contains("exit status: 1"), "{error}");
        assert!(!f.task_env.join("warm.json").exists());
    }

    #[test]
    fn a_warm_build_records_the_tree_hashes() {
        let f = fixture();
        let jail = fake_jail(f.root.as_path(), FILL);
        let (warm, printed) = run_warm(&f, &jail);
        let warm = warm.unwrap();
        assert_eq!(
            fs::read_to_string(f.run.join("args")).unwrap(),
            format!(
                "command\nclaude\n--task-env\n{}\n--run\n{}\n--wall-cap\n30\n--template\nbuild\n--base-rw\n",
                f.task_env.display(),
                f.run.display()
            )
        );
        let written = fs::read_to_string(f.task_env.join("warm.json")).unwrap();
        assert_eq!(written, printed);
        assert!(written.ends_with("}\n"));
        assert_eq!(read_warm(&f.task_env).unwrap(), warm);
        assert_eq!(warm.schema.as_str(), WARM_SCHEMA);
        assert_eq!(warm.object_set_sha256.as_str(), f.object_set);
        let expected_target =
            crate::sha256_hex(format!("f - 5 {} out\n", crate::sha256_hex(b"built")).as_bytes());
        assert_eq!(warm.target_sha256.as_str(), expected_target);
        assert_eq!(
            warm.cargo_sha256.as_str(),
            crate::sha256_hex(b"d - 0 - registry\n")
        );
        // A second warm build on a filled task environment is refused before agent-jail runs.
        fs::remove_file(f.run.join("args")).unwrap();
        assert!(run_warm(&f, &jail).0.is_err());
        assert!(!f.run.join("args").exists());
    }

    #[test]
    fn a_workspace_that_fails_its_check_is_refused_before_agent_jail() {
        let f = fixture();
        fs::write(f.run.join("workspace/stray"), "x").unwrap();
        let jail = fake_jail(f.root.as_path(), FILL);
        let (result, printed) = run_warm(&f, &jail);
        assert!(result.is_err());
        assert!(printed.contains("stray"), "{printed}");
        assert!(!f.run.join("args").exists());
        assert!(!f.task_env.join("warm.json").exists());
    }

    /// A warmed task environment, its env.json, a task record matching both, and two empty
    /// private layers.
    fn warmed() -> (Fixture, Task, Vec<PathBuf>) {
        warmed_in(&std::env::temp_dir())
    }

    fn warmed_in(base: &Path) -> (Fixture, Task, Vec<PathBuf>) {
        let f = fixture_in(base);
        let jail = fake_jail(f.root.as_path(), FILL);
        let warm = run_warm(&f, &jail).0.unwrap();
        let env = serde_json::to_vec_pretty(&json!({
            "schema": "agent-jail-task-env/1",
            "dev_shell": "devShells.x86_64-linux.default",
            "exclude": ["br", "bv"],
            "pass": ["EON_*", "GHOSTTY_*"],
            "variables": {"CC": "/nix/store/x-cc/bin/cc"},
            "test": {
                "command": ["eon-test", "-p", "yazelix-venus"],
                "build": ["eon-test", "list", "-p", "yazelix-venus"],
            },
        }))
        .unwrap();
        fs::write(f.task_env.join("env.json"), &env).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(TASK).unwrap();
        value["snapshot_object_set_sha256"] = json!(f.object_set);
        value["env"]["env_sha256"] = json!(sha256_hex(&env));
        value["env"]["warm_target_sha256"] = json!(warm.target_sha256);
        let task = crate::record::task(&serde_json::to_vec(&value).unwrap()).unwrap();
        let layers: Vec<PathBuf> = ["upper", "work"]
            .iter()
            .map(|name| f.run.join("overlay").join(name))
            .collect();
        for layer in &layers {
            fs::create_dir_all(layer).unwrap();
        }
        (f, task, layers)
    }

    #[test]
    fn a_matching_task_environment_passes() {
        let (f, task, layers) = warmed();
        let violations = check(&task, &f.task_env, &f.object_set, &layers);
        assert!(violations.is_empty(), "{violations:?}");
    }

    /// Darwin's temp dir sits behind `/var -> /private/var`; the fixture's canonical root keeps
    /// the strict ancestor check passing there. Reproduced here with a linked temp base.
    #[test]
    fn a_task_environment_under_a_linked_temp_dir_passes() {
        let real = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let base = links.path().join("tmp");
        std::os::unix::fs::symlink(real.path(), &base).unwrap();
        let (f, task, layers) = warmed_in(&base);
        let violations = check(&task, &f.task_env, &f.object_set, &layers);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn each_tampered_condition_is_caught() {
        type Tamper = fn(&Fixture, &mut Task, &mut Vec<PathBuf>, &mut String);
        let cases: Vec<(&str, Tamper, &str)> = vec![
            (
                "object set differs from the task",
                |_, _, _, object_set| *object_set = "0".repeat(64),
                "is not task.json snapshot_object_set_sha256",
            ),
            (
                "env.json edited",
                |f, _, _, _| {
                    let path = f.task_env.join("env.json");
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.push(b'\n');
                    fs::write(path, bytes).unwrap();
                },
                "not task.json env.env_sha256",
            ),
            (
                "env.json missing",
                |f, _, _, _| fs::remove_file(f.task_env.join("env.json")).unwrap(),
                "env.json: No such file",
            ),
            (
                "test command differs",
                |_, task, _, _| task.test.command.push(Text::new("--extra").unwrap()),
                "env.json test.command",
            ),
            (
                "test build differs",
                |_, task, _, _| task.test.build.pop().map(drop).unwrap(),
                "env.json test.build",
            ),
            (
                "exclude differs",
                |_, task, _, _| task.env.exclude.pop().map(drop).unwrap(),
                "env.json exclude",
            ),
            (
                "pass differs",
                |_, task, _, _| task.env.pass.push(Text::new("NIX_*").unwrap()),
                "env.json pass",
            ),
            (
                "warm.json missing",
                |f, _, _, _| fs::remove_file(f.task_env.join("warm.json")).unwrap(),
                "warm.json: No such file",
            ),
            (
                "warm.json from another workspace",
                |f, _, _, _| edit_warm(f, "object_set_sha256", &"1".repeat(64)),
                "warm.json object_set_sha256",
            ),
            (
                "warm.json with an unknown schema",
                |f, _, _, _| edit_warm(f, "schema", "agent-bench-warm/2"),
                "warm.json schema",
            ),
            (
                "target changed",
                |f, _, _, _| fs::write(f.task_env.join("target/out"), "other").unwrap(),
                "not task.json env.warm_target_sha256",
            ),
            (
                "target disagrees with warm.json",
                |f, _, _, _| edit_warm(f, "target_sha256", &"2".repeat(64)),
                "not warm.json target_sha256",
            ),
            (
                "cargo changed",
                |f, _, _, _| fs::write(f.task_env.join("cargo/extra"), "x").unwrap(),
                "not warm.json cargo_sha256",
            ),
            (
                "private layer not empty",
                |_, _, layers, _| fs::write(layers[0].join("planted"), "x").unwrap(),
                "is not empty",
            ),
            (
                "private layer missing",
                |_, _, layers, _| fs::remove_dir(&layers[1]).unwrap(),
                "No such file",
            ),
            (
                "private layer a symlink",
                |f, _, layers, _| {
                    let elsewhere = f.root.as_path().join("elsewhere");
                    fs::create_dir(&elsewhere).unwrap();
                    fs::remove_dir(&layers[0]).unwrap();
                    symlink(&elsewhere, &layers[0]).unwrap();
                },
                "is a symlink",
            ),
            (
                "private layer under a symlink",
                |f, _, _, _| {
                    let overlay = f.run.join("overlay");
                    let elsewhere = f.root.as_path().join("elsewhere");
                    fs::rename(&overlay, &elsewhere).unwrap();
                    symlink(&elsewhere, &overlay).unwrap();
                },
                "/overlay is a symlink",
            ),
            (
                "private layer a file",
                |_, _, layers, _| {
                    fs::remove_dir(&layers[0]).unwrap();
                    fs::write(&layers[0], "").unwrap();
                },
                "is not a directory",
            ),
        ];
        for (name, tamper, needle) in cases {
            let (f, mut task, mut layers) = warmed();
            let mut object_set = f.object_set.clone();
            tamper(&f, &mut task, &mut layers, &mut object_set);
            let violations = check(&task, &f.task_env, &object_set, &layers);
            assert!(
                violations.iter().any(|v| v.contains(needle)),
                "{name}: expected {needle:?} in {violations:?}"
            );
        }
    }

    fn edit_warm(f: &Fixture, field: &str, value: &str) {
        let path = f.task_env.join("warm.json");
        let mut warm: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        warm[field] = json!(value);
        fs::write(path, serde_json::to_vec(&warm).unwrap()).unwrap();
    }
}
