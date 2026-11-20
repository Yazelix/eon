//! The task environment: one directory per task that every jail of the
//! task mounts through a [`TaskEnv`].
//!
//! ```text
//! <task-env>/
//!   env.json     dev shell, kept variables, PATH dirs, test templates, dev closure (hashed)
//!   tools.json   jail tool roots and their closure (config facts, outside the task hash)
//!   warm.json    agent-bench writes it once the warm build exits 0
//!   store/       chroot store: store/nix/store is mounted at /nix/store, store/nix/var never
//!   cargo/       CARGO_HOME base (lower layer)
//!   target/      CARGO_TARGET_DIR base (lower layer, the warm cache)
//!   .lock        flock: exclusive for store builds, --base-rw and clean; shared for runs
//! ```
//!
//! The CLI builds a [`TaskEnv`] only with [`TaskEnv::load`]. Tests build one
//! with [`TaskEnv::host_for_tests`], whose store is the host's and whose
//! record says so (`store.kind = "host-test"`), which no bench record accepts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

pub const ENV_SCHEMA: &str = "agent-jail-task-env/1";
pub const TOOLS_SCHEMA: &str = "agent-jail-task-tools/1";
pub const STORE: &str = "/nix/store";
/// Where the jail mounts the Cargo home and the target directory.
pub const CARGO_HOME: &str = "/bench/cargo";
pub const TARGET_DIR: &str = "/bench/target";
/// The overlayfs hard-link directory `nix copy` may create in a store.
const LINKS: &str = ".links";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Test {
    pub command: Vec<String>,
    pub build: Vec<String>,
}

/// `env.json`. Field order is the file's; every unordered list is sorted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvFile {
    pub schema: String,
    pub dev_shell: String,
    pub exclude: Vec<String>,
    pub pass: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub dropped: Vec<String>,
    /// The dev shell's PATH dirs, in order.
    pub path: Vec<String>,
    pub test: Test,
    pub roots: Vec<String>,
    pub closure: Vec<String>,
}

/// `tools.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsFile {
    pub schema: String,
    pub roots: Vec<String>,
    pub closure: Vec<String>,
}

/// Pretty JSON with a trailing newline: the bytes `env_sha256` covers.
pub fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(dir: &Path, name: &str) -> Result<(Vec<u8>, T)> {
    use std::io::Read;
    let mut bytes = Vec::new();
    crate::run_dir::open_inside(dir, Path::new(name))?
        .take(64 << 20)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("{name}: {err}"))?;
    let value = serde_json::from_slice(&bytes).map_err(|err| format!("{name}: {err}"))?;
    Ok((bytes, value))
}

impl EnvFile {
    pub fn read(dir: &Path) -> Result<(Vec<u8>, Self)> {
        let (bytes, env): (Vec<u8>, Self) = read_json(dir, "env.json")?;
        if env.schema != ENV_SCHEMA {
            return Err(format!(
                "env.json: schema {} is not {ENV_SCHEMA}",
                env.schema
            ));
        }
        Ok((bytes, env))
    }
}

impl ToolsFile {
    pub fn read(dir: &Path) -> Result<Self> {
        let (_, tools): (Vec<u8>, Self) = read_json(dir, "tools.json")?;
        if tools.schema != TOOLS_SCHEMA {
            return Err(format!(
                "tools.json: schema {} is not {TOOLS_SCHEMA}",
                tools.schema
            ));
        }
        Ok(tools)
    }
}

/// Which store a run mounted, for `meta/run.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Store {
    Task { env_sha256: String },
    HostTest,
}

/// The test template `command` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Build,
    Test,
}

impl Template {
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "build" => Ok(Self::Build),
            "test" => Ok(Self::Test),
            other => Err(format!("--template must be build or test, not {other}")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaskEnv {
    dir: PathBuf,
    /// The host directory mounted at /nix/store.
    store_dir: PathBuf,
    store: Store,
    variables: BTreeMap<String, String>,
    path: Vec<String>,
    exclude: Vec<String>,
    test: Test,
    cargo: PathBuf,
    target: PathBuf,
    /// tools.json's roots; `None` for the host-test store.
    tool_roots: Option<BTreeSet<String>>,
}

impl TaskEnv {
    /// Loads `dir` and verifies its store holds exactly the recorded
    /// closures (top-level names only).
    pub fn load(dir: &Path) -> Result<Self> {
        let dir = std::path::absolute(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let store_dir = dir.join("store/nix/store");
        for path in [&dir, &store_dir, &dir.join("cargo"), &dir.join("target")] {
            refuse_links(path)?;
            if !path.is_dir() {
                return Err(format!("{} is not a directory", path.display()));
            }
        }
        let (bytes, env) = EnvFile::read(&dir)?;
        let tools = ToolsFile::read(&dir)?;
        let expected: BTreeSet<&str> = env
            .closure
            .iter()
            .chain(&tools.closure)
            .map(String::as_str)
            .collect();
        verify_store(&store_dir, &expected)?;
        Ok(Self {
            store: Store::Task {
                env_sha256: crate::sha256_hex(&bytes),
            },
            cargo: dir.join("cargo"),
            target: dir.join("target"),
            store_dir,
            variables: env.variables,
            path: env.path,
            exclude: env.exclude,
            test: env.test,
            tool_roots: Some(tools.roots.into_iter().collect()),
            dir,
        })
    }

    /// A task environment on the host store for `tests/jail.rs`: the dev
    /// shell's cargo and rustc dirs from `PATH`, the test environment's
    /// `CARGO_HOME` as the Cargo lower layer (eon-test's `cargo fetch
    /// --locked` filled it), an empty target base under `dir`, and no
    /// excluded package. Its runs record `store.kind = "host-test"`.
    pub fn host_for_tests(dir: &Path, test: Test) -> Result<Self> {
        let search = std::env::var_os("PATH").ok_or("PATH is unset")?;
        let mut path = Vec::new();
        for name in ["cargo", "rustc"] {
            let found = std::env::split_paths(&search)
                .find(|bin| bin.join(name).is_file())
                .ok_or(format!(
                    "no {name} on PATH; run the tests as nix develop -c eon-test"
                ))?
                .display()
                .to_string();
            if !found.starts_with(&format!("{STORE}/")) {
                return Err(format!("{name} is in {found}, not the dev shell's store"));
            }
            if !path.contains(&found) {
                path.push(found);
            }
        }
        let cargo = PathBuf::from(
            std::env::var_os("CARGO_HOME")
                .ok_or("CARGO_HOME is unset; run the tests as nix develop -c eon-test")?,
        );
        if !cargo.is_dir() {
            return Err(format!("CARGO_HOME {} is not a directory", cargo.display()));
        }
        let target = dir.join("target");
        std::fs::create_dir_all(&target).map_err(|err| format!("{}: {err}", target.display()))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            store_dir: PathBuf::from(STORE),
            store: Store::HostTest,
            variables: BTreeMap::new(),
            path,
            exclude: Vec::new(),
            test,
            cargo,
            target,
            tool_roots: None,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The host directory mounted at /nix/store.
    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn variables(&self) -> &BTreeMap<String, String> {
        &self.variables
    }

    /// The dev shell's PATH dirs (in-jail store paths).
    pub fn path(&self) -> &[String] {
        &self.path
    }

    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }

    /// The Cargo home and target bases (lower layers).
    pub fn cargo(&self) -> &Path {
        &self.cargo
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn template(&self, which: Template) -> &[String] {
        match which {
            Template::Build => &self.test.build,
            Template::Test => &self.test.command,
        }
    }

    /// `<task-env>/.lock`, held for the returned file's lifetime; never waits.
    pub fn lock(&self, exclusive: bool) -> Result<File> {
        lock(&self.dir, exclusive)
    }

    /// `--base-rw`'s refusals: no link on the way to either base, an empty
    /// target and no warm.json. The caller holds the exclusive lock.
    pub fn check_base_rw(&self) -> Result {
        refuse_links(&self.target)?;
        refuse_links(&self.cargo)?;
        let warm = self.dir.join("warm.json");
        if std::fs::symlink_metadata(&warm).is_ok() {
            return Err(format!(
                "{} exists; the warm build already ran",
                warm.display()
            ));
        }
        let mut entries = std::fs::read_dir(&self.target)
            .map_err(|err| format!("{}: {err}", self.target.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} is not empty; a warm build needs a fresh task environment",
                self.target.display()
            ));
        }
        Ok(())
    }

    /// The jail's tool roots (pins and agent-jail's own ELF references)
    /// must be the ones tools.json recorded.
    pub fn check_tools(&self, self_exe: &Path) -> Result {
        let Some(recorded) = &self.tool_roots else {
            return Ok(());
        };
        let current = tool_roots(self_exe)?;
        if &current != recorded {
            let missing: Vec<&String> = current.difference(recorded).collect();
            return Err(format!(
                "tools.json does not record the jail's tool roots (missing {missing:?}); re-run agent-jail task-env store"
            ));
        }
        Ok(())
    }
}

/// Opens `<dir>/.lock` (never through a link) and flocks it without waiting.
pub fn lock(dir: &Path, exclusive: bool) -> Result<File> {
    use rustix::fs::FlockOperation;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(".lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    let operation = if exclusive {
        FlockOperation::NonBlockingLockExclusive
    } else {
        FlockOperation::NonBlockingLockShared
    };
    rustix::fs::flock(&file, operation).map_err(|err| {
        format!(
            "{} is busy ({err}): another build, warm build or clean holds it",
            dir.display()
        )
    })?;
    Ok(file)
}

/// Refuses `path` when it or any ancestor is a symbolic link, or cannot be
/// looked at; only a missing one is fine.
pub fn refuse_links(path: &Path) -> Result {
    let path = std::path::absolute(path).map_err(|err| format!("{}: {err}", path.display()))?;
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "{} is a symbolic link; the task environment refuses it",
                    ancestor.display()
                ));
            }
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {err}", ancestor.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The top-level entries of `store_dir` (by lstat; `.links` aside) must be
/// exactly `expected` (full store paths).
pub fn verify_store(store_dir: &Path, expected: &BTreeSet<&str>) -> Result {
    let mut present = BTreeSet::new();
    for entry in
        std::fs::read_dir(store_dir).map_err(|err| format!("{}: {err}", store_dir.display()))?
    {
        let entry = entry.map_err(|err| format!("{}: {err}", store_dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == LINKS
            && std::fs::symlink_metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())
        {
            continue;
        }
        present.insert(format!("{STORE}/{name}"));
    }
    let expected: BTreeSet<String> = expected.iter().map(|path| (*path).to_owned()).collect();
    if present == expected {
        return Ok(());
    }
    let extra: Vec<&String> = present.difference(&expected).take(5).collect();
    let missing: Vec<&String> = expected.difference(&present).take(5).collect();
    Err(format!(
        "{} holds {} paths, not the recorded {}: extra {extra:?}, missing {missing:?}",
        store_dir.display(),
        present.len(),
        expected.len()
    ))
}

/// The store package a path lies in: `/nix/store/<hash>-<name>`.
pub fn package_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix(&format!("{STORE}/"))?;
    let name = rest.split('/').next()?;
    (name.len() > 33 && name.as_bytes()[32] == b'-').then(|| format!("{STORE}/{name}"))
}

/// Every store package named anywhere in `text`.
pub fn store_paths_in(text: &str) -> BTreeSet<String> {
    let prefix = format!("{STORE}/");
    let mut found = BTreeSet::new();
    for (at, _) in text.match_indices(&prefix) {
        let name: String = text[at + prefix.len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || "+-._?=".contains(*c))
            .collect();
        if let Some(package) = package_of(&format!("{prefix}{name}")) {
            found.insert(package);
        }
    }
    found
}

/// Whether the store path (or bare store name) is package `pname`, matched
/// on the whole Nix name: `<hash>-br` or `<hash>-br-<version>`, never
/// `<hash>-brotli-…`.
pub fn is_package(path: &str, pname: &str) -> bool {
    let name = path
        .strip_prefix(&format!("{STORE}/"))
        .unwrap_or(path)
        .split('/')
        .next()
        .unwrap_or_default();
    let Some(name) = name
        .get(33..)
        .filter(|_| name.as_bytes().get(32) == Some(&b'-'))
    else {
        return false;
    };
    name == pname
        || name
            .strip_prefix(pname)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
}

/// The jail's tool roots: the packages of every store pin and inner
/// package, and of agent-jail's ELF interpreter and run paths.
pub fn tool_roots(self_exe: &Path) -> Result<BTreeSet<String>> {
    let mut roots = crate::pins::store_packages()?;
    let refs = crate::elf::read(self_exe)?;
    for path in refs.interpreter.iter().chain(&refs.runpath) {
        if path.starts_with("$ORIGIN") {
            continue;
        }
        roots.insert(package_of(path).ok_or(format!(
            "{} loads from {path}, which is not in {STORE}",
            self_exe.display()
        ))?);
    }
    Ok(roots)
}

/// `agent-jail task-env clean <dir>`: refuses a directory that is not a
/// task environment (a valid env.json and the store/, cargo/ and target/
/// directories), takes the exclusive lock, opens every directory up
/// (following no link) and removes the tree.
pub fn clean(dir: &Path) -> Result {
    refuse_links(dir)?;
    let not_task_env = |why: String| {
        format!(
            "{}: {why}; clean removes only task environments",
            dir.display()
        )
    };
    EnvFile::read(dir).map_err(not_task_env)?;
    for sub in ["store", "cargo", "target"] {
        if !std::fs::symlink_metadata(dir.join(sub)).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(not_task_env(format!("{sub}/ is not a directory")));
        }
    }
    let _lock = lock(dir, true)?;
    let errors = crate::run_dir::open_up(dir);
    std::fs::remove_dir_all(dir)
        .map_err(|err| format!("{}: {err} (opening up: {errors:?})", dir.display()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const HASH: &str = "0123456789abcdfghijklmnpqrsvwxyz";

    pub(crate) fn path(name: &str) -> String {
        format!("{STORE}/{HASH}-{name}")
    }

    /// A task environment with a fake store holding `names`.
    pub(crate) fn fixture(dir: &Path, names: &[&str]) -> TaskEnv {
        let store = dir.join("store/nix/store");
        std::fs::create_dir_all(&store).unwrap();
        for name in names {
            std::fs::create_dir(store.join(format!("{HASH}-{name}"))).unwrap();
        }
        for sub in ["cargo", "target"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let env = EnvFile {
            schema: ENV_SCHEMA.into(),
            dev_shell: "devShells.x86_64-linux.default".into(),
            exclude: vec!["br".into()],
            pass: vec![],
            variables: [("CC".to_owned(), format!("{}/bin/cc", path("cc")))].into(),
            dropped: vec![],
            path: vec![format!("{}/bin", path("cc"))],
            test: Test {
                command: vec!["eon-test".into()],
                build: vec!["eon-test".into(), "list".into()],
            },
            roots: vec![path("cc")],
            closure: vec![path("cc")],
        };
        std::fs::write(dir.join("env.json"), json_bytes(&env).unwrap()).unwrap();
        let tools = ToolsFile {
            schema: TOOLS_SCHEMA.into(),
            roots: vec![path("tool")],
            closure: vec![path("tool")],
        };
        std::fs::write(dir.join("tools.json"), json_bytes(&tools).unwrap()).unwrap();
        TaskEnv::load(dir).unwrap()
    }

    #[test]
    fn a_store_must_hold_exactly_the_recorded_closures() {
        let dir = crate::test_dir("task-env-exact");
        let env = fixture(&dir, &["cc", "tool"]);
        assert!(matches!(env.store(), Store::Task { env_sha256 } if env_sha256.len() == 64));
        assert_eq!(env.template(Template::Build), ["eon-test", "list"]);
        // nix copy's .links directory is not a store path.
        std::fs::create_dir(dir.join("store/nix/store/.links")).unwrap();
        assert!(TaskEnv::load(&dir).is_ok());
        // An extra path is refused.
        std::fs::create_dir(dir.join(format!("store/nix/store/{HASH}-extra"))).unwrap();
        assert!(TaskEnv::load(&dir).unwrap_err().contains("extra"));
        std::fs::remove_dir(dir.join(format!("store/nix/store/{HASH}-extra"))).unwrap();
        // A missing path is refused.
        std::fs::remove_dir(dir.join(format!("store/nix/store/{HASH}-tool"))).unwrap();
        assert!(TaskEnv::load(&dir).unwrap_err().contains("missing"));
        // A link entry counts as a name, and a .links that is a link is refused.
        std::os::unix::fs::symlink(
            "/nix/store",
            dir.join(format!("store/nix/store/{HASH}-tool")),
        )
        .unwrap();
        assert!(TaskEnv::load(&dir).is_ok());
        std::fs::remove_dir(dir.join("store/nix/store/.links")).unwrap();
        std::os::unix::fs::symlink("/tmp", dir.join("store/nix/store/.links")).unwrap();
        assert!(TaskEnv::load(&dir).unwrap_err().contains(".links"));
    }

    #[test]
    fn task_environments_behind_links_are_refused() {
        let dir = crate::test_dir("task-env-link");
        fixture(&dir.join("real"), &["cc", "tool"]);
        std::os::unix::fs::symlink(dir.join("real"), dir.join("alias")).unwrap();
        assert!(
            TaskEnv::load(&dir.join("alias"))
                .unwrap_err()
                .contains("symbolic link")
        );
        assert!(TaskEnv::load(&dir.join("real")).is_ok());
        // Only a missing ancestor is fine: one that cannot be looked at refuses.
        assert!(refuse_links(&dir.join("absent/env")).is_ok());
        std::fs::write(dir.join("file"), "").unwrap();
        assert!(
            refuse_links(&dir.join("file/env"))
                .unwrap_err()
                .contains("Not a directory")
        );
    }

    #[test]
    fn base_rw_needs_an_empty_target_and_no_warm_json() {
        let dir = crate::test_dir("task-env-base-rw");
        let env = fixture(&dir, &["cc", "tool"]);
        let _lock = env.lock(true).unwrap();
        assert!(env.lock(false).unwrap_err().contains("busy"));
        assert!(env.check_base_rw().is_ok());
        std::fs::write(dir.join("target/x"), "").unwrap();
        assert!(env.check_base_rw().unwrap_err().contains("not empty"));
        std::fs::remove_file(dir.join("target/x")).unwrap();
        std::fs::write(dir.join("warm.json"), "{}").unwrap();
        assert!(env.check_base_rw().unwrap_err().contains("warm.json"));
        std::fs::remove_file(dir.join("warm.json")).unwrap();
        std::fs::remove_dir(dir.join("cargo")).unwrap();
        std::os::unix::fs::symlink(dir.join("target"), dir.join("cargo")).unwrap();
        assert!(env.check_base_rw().unwrap_err().contains("symbolic link"));
    }

    #[test]
    fn shared_locks_coexist_and_exclude_an_exclusive_one() {
        let dir = crate::test_dir("task-env-locks");
        let first = lock(&dir, false).unwrap();
        let _second = lock(&dir, false).unwrap();
        assert!(lock(&dir, true).is_err());
        drop(first);
        drop(_second);
        assert!(lock(&dir, true).is_ok());
    }

    #[test]
    fn packages_match_on_the_whole_nix_name() {
        assert!(is_package(&path("br-0.7.2"), "br"));
        assert!(is_package(&format!("{}/bin/br", path("br-0.7.2")), "br"));
        assert!(is_package(&format!("{HASH}-br"), "br"));
        assert!(!is_package(&path("brotli-1.2.0"), "br"));
        assert!(!is_package(&path("br-unwrapped-1"), "br"));
        assert!(!is_package(&path("bv-0.25.2"), "br"));
        assert!(!is_package("/usr/bin/br", "br"));
        assert_eq!(
            package_of(&format!("{}/lib/x.so", path("glibc-2.42"))),
            Some(path("glibc-2.42"))
        );
        assert_eq!(package_of("/nix/store/short"), None);
        assert_eq!(package_of("/usr/lib"), None);
    }

    #[test]
    fn store_paths_are_found_in_values() {
        let text = format!(
            "-isystem {}/include -L{}/lib:{}/bin",
            path("a-dev"),
            path("b-1.0"),
            path("c")
        );
        assert_eq!(
            store_paths_in(&text),
            [path("a-dev"), path("b-1.0"), path("c")]
                .into_iter()
                .collect()
        );
        assert!(store_paths_in("/nix/store/ and /nix/store/x").is_empty());
    }

    #[test]
    fn clean_removes_only_task_environments() {
        let dir = crate::test_dir("task-env-clean");
        std::fs::write(dir.join("precious"), "x").unwrap();
        assert!(clean(&dir).unwrap_err().contains("env.json"));
        assert!(dir.join("precious").exists());
        // A project that happens to hold an env.json, even with the layout.
        for sub in ["store", "cargo", "target"] {
            std::fs::create_dir(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("env.json"), "{\"name\": \"my-project\"}\n").unwrap();
        assert!(clean(&dir).unwrap_err().contains("env.json"));
        assert!(dir.join("precious").exists());
        // A valid env.json without the layout: no store/, or a linked one.
        let partial = crate::test_dir("task-env-clean-partial");
        fixture(&partial, &["cc", "tool"]);
        std::fs::write(partial.join("precious"), "x").unwrap();
        std::fs::rename(partial.join("store"), partial.join("elsewhere")).unwrap();
        assert!(clean(&partial).unwrap_err().contains("store/"));
        std::os::unix::fs::symlink(partial.join("elsewhere"), partial.join("store")).unwrap();
        assert!(clean(&partial).unwrap_err().contains("store/"));
        assert!(partial.join("precious").exists());
        assert!(partial.join("elsewhere/nix/store").exists());
        let env = crate::test_dir("task-env-clean-env");
        fixture(&env, &["cc", "tool"]);
        use std::os::unix::fs::PermissionsExt;
        let locked = env.join(format!("store/nix/store/{HASH}-cc"));
        std::fs::write(locked.join("file"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        clean(&env).unwrap();
        assert!(!env.exists());
    }
}
