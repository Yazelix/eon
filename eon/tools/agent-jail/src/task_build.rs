//! The host-side builders of a task environment: `task-env store` (the
//! task-local Nix store and env.json) and `task-env cargo` (the offline
//! Cargo home). Every child process goes through a [`Runner`], so tests can
//! fake nix and cargo; nix is the host's CLI, from `PATH`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::Result;
use crate::task_env::{
    ENV_SCHEMA, EnvFile, STORE, TOOLS_SCHEMA, Test, ToolsFile, is_package, json_bytes, lock,
    package_of, refuse_links, store_paths_in, verify_store,
};

/// Free space a store build or a run must leave on its filesystem.
pub const DISK_FLOOR: u64 = 20 << 30;

/// One child process: `env: None` inherits the caller's environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<PathBuf>,
}

/// Runs a [`Cmd`] and returns its standard output; a non-zero exit fails.
pub type Runner<'a> = dyn FnMut(&Cmd) -> Result<Vec<u8>> + 'a;

/// The real runner: standard error goes to the caller's.
pub fn system(cmd: &Cmd) -> Result<Vec<u8>> {
    let mut command = std::process::Command::new(&cmd.program);
    command.args(&cmd.args).stdin(std::process::Stdio::null());
    if let Some(env) = &cmd.env {
        command.env_clear().envs(env);
    }
    if let Some(cwd) = &cmd.cwd {
        command.current_dir(cwd);
    }
    let output = command
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|err| format!("cannot run {}: {err}", cmd.program.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} {} failed ({})",
            cmd.program.display(),
            cmd.args.first().map(String::as_str).unwrap_or_default(),
            output.status
        ));
    }
    Ok(output.stdout)
}

fn nix(args: impl IntoIterator<Item = String>) -> Cmd {
    let mut all = vec![
        "--extra-experimental-features".to_owned(),
        "nix-command flakes".to_owned(),
    ];
    all.extend(args);
    Cmd {
        program: "nix".into(),
        args: all,
        env: None,
        cwd: None,
    }
}

const TOOLCHAIN: &[&str] = &[
    "AR", "AS", "CC", "CXX", "LD", "NM", "OBJCOPY", "OBJDUMP", "RANLIB", "READELF", "SIZE",
    "STRINGS", "STRIP",
];
const ALLOWED: &[&str] = &[
    "NIX_HARDENING_ENABLE",
    // Not `NIX_ENFORCE_PURITY`: `nix develop` unsets it, and with it the cc wrapper refuses to
    // link anything under /bench.
    "NIX_ENFORCE_NO_NATIVE",
    "NIX_STORE",
    "NIX_BUILD_CORES",
    "CONFIG_SHELL",
    "IN_NIX_SHELL",
    "SOURCE_DATE_EPOCH",
    "TZ",
    "XDG_DATA_DIRS",
];
const ALLOWED_PREFIXES: &[&str] = &[
    "PKG_CONFIG",
    "RUST",
    "NIX_CC",
    "NIX_BINTOOLS",
    "NIX_CFLAGS_",
    "NIX_LDFLAGS",
    "NIX_PKG_CONFIG_WRAPPER_",
];
/// Never kept, whatever matches them: the jail owns these, or they would
/// route Nix, TLS, the loader or a shell somewhere else.
const DENIED: &[&str] = &[
    "SSL_CERT_FILE",
    "BASH_ENV",
    "ENV",
    "HOME",
    "PATH",
    "SHELL",
    "TMPDIR",
    "TERM",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_JOBS",
    "CARGO_INCREMENTAL",
];

fn allowed(name: &str) -> bool {
    TOOLCHAIN.contains(&name)
        || name
            .strip_suffix("_FOR_BUILD")
            .is_some_and(|base| TOOLCHAIN.contains(&base))
        || ALLOWED.contains(&name)
        || ALLOWED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn denied(name: &str) -> bool {
    DENIED.contains(&name)
        || (name.starts_with("NIX_") && !allowed(name))
        // The loader's variables; `LD_FOR_BUILD` is the build linker, a toolchain name.
        || (name.starts_with("LD_") && !allowed(name))
        || name.starts_with("CARGO_NET_")
        || name.to_ascii_lowercase().ends_with("_proxy")
}

/// A `--pass` pattern: a variable name, or a name prefix ending in `*`.
pub fn check_pattern(pattern: &str) -> Result {
    let name = pattern.strip_suffix('*').unwrap_or(pattern);
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!("--pass {pattern} is not NAME or PREFIX*"));
    }
    Ok(())
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// The first part of `value` that names an absolute path outside the store.
pub fn outside_store(value: &str) -> Option<&str> {
    value
        .split(|c: char| c == ':' || c == '=' || c == ',' || c == ';' || c.is_whitespace())
        .find(|token| {
            let path = if token.starts_with('/') {
                Some(*token)
            } else if token.starts_with('-') {
                token.find('/').map(|at| &token[at..])
            } else {
                None
            };
            path.is_some_and(|path| path != STORE && !path.starts_with(&format!("{STORE}/")))
        })
}

fn excluded(entry: &str, exclude: &[String]) -> bool {
    exclude.iter().any(|pname| is_package(entry, pname))
}

/// What the dev shell's variables become in env.json.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Filtered {
    pub variables: BTreeMap<String, String>,
    pub dropped: Vec<String>,
    pub path: Vec<String>,
}

/// Filters `nix print-dev-env --json` output: the allowlist and the
/// `pass` patterns keep a variable, a hard deny refuses one either would
/// keep, everything else exported is dropped (and listed). Excluded
/// packages leave every colon-separated value; a kept value naming an
/// absolute path outside the store, and a non-empty shellHook, are refused.
pub fn filter(dev: &Value, pass: &[String], exclude: &[String]) -> Result<Filtered> {
    let variables = dev
        .get("variables")
        .and_then(Value::as_object)
        .ok_or("print-dev-env printed no variables")?;
    if variables
        .get("shellHook")
        .and_then(|hook| hook.get("value"))
        .and_then(Value::as_str)
        .is_some_and(|hook| !hook.trim().is_empty())
    {
        return Err(
            "the dev shell has a shellHook; the jail cannot run it, so it is refused".into(),
        );
    }
    // The shell's own output (`nix-shell-env`) is a dump of every shell input, so it is never
    // a root: stdenv's self-rpath (`-rpath $out/lib`) is removed, and any other kept value
    // naming it is refused.
    let own = variables
        .get("out")
        .and_then(|out| out.get("value"))
        .and_then(Value::as_str)
        .filter(|out| !out.is_empty());
    let self_rpath = own.map(|out| format!("-rpath {out}/lib"));
    let mut filtered = Filtered::default();
    for (name, variable) in variables {
        if variable.get("type").and_then(Value::as_str) != Some("exported") {
            continue;
        }
        let value = variable
            .get("value")
            .and_then(Value::as_str)
            .ok_or(format!("exported variable {name} is not a string"))?;
        let kept: Vec<&str> = value
            .split(':')
            .filter(|entry| !excluded(entry, exclude))
            .collect();
        if name == "PATH" {
            for dir in kept.iter().filter(|dir| !dir.is_empty()) {
                if !dir.starts_with(&format!("{STORE}/")) {
                    return Err(format!("the dev shell's PATH holds {dir}, outside {STORE}"));
                }
                if !filtered.path.iter().any(|known| known == dir) {
                    filtered.path.push((*dir).to_owned());
                }
            }
            continue;
        }
        let wanted = allowed(name) || pass.iter().any(|pattern| matches(pattern, name));
        if denied(name) {
            if wanted {
                return Err(format!(
                    "{name} is never passed to the jail, but a pattern keeps it"
                ));
            }
            filtered.dropped.push(name.clone());
            continue;
        }
        if !wanted {
            filtered.dropped.push(name.clone());
            continue;
        }
        let mut value = kept.join(":");
        if let Some(self_rpath) = &self_rpath {
            value = value.replace(self_rpath.as_str(), "").trim().to_owned();
        }
        if let Some(out) = own.filter(|out| value.contains(out)) {
            return Err(format!("{name} names the dev shell's own output {out}"));
        }
        if name == "NIX_STORE" && value != STORE {
            return Err(format!("NIX_STORE is {value}, not {STORE}"));
        }
        if let Some(path) = outside_store(&value) {
            return Err(format!("{name} names {path}, outside {STORE}"));
        }
        filtered.variables.insert(name.clone(), value);
    }
    Ok(filtered)
}

/// `nix path-info -r --json` output as store path to NAR size. Older Nix
/// prints an array of objects with `path`, newer an object keyed by path
/// (with or without the store directory).
pub fn path_info(json: &Value) -> Result<BTreeMap<String, u64>> {
    let entries: Vec<(String, &Value)> = match json {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let path = item.get("path").and_then(Value::as_str).unwrap_or_default();
                (path.to_owned(), item)
            })
            .collect(),
        Value::Object(map) => map.iter().map(|(key, item)| (key.clone(), item)).collect(),
        _ => return Err("nix path-info printed neither an array nor an object".into()),
    };
    let mut sizes = BTreeMap::new();
    for (key, item) in entries {
        let path = if key.starts_with('/') {
            key
        } else {
            format!("{STORE}/{key}")
        };
        if package_of(&path).as_deref() != Some(path.as_str()) {
            return Err(format!("nix path-info named {path}, not a store path"));
        }
        let size = item.get("narSize").and_then(Value::as_u64).ok_or(format!(
            "nix path-info has no narSize for {path} (is it valid?)"
        ))?;
        sizes.insert(path, size);
    }
    Ok(sizes)
}

fn closure(run: &mut Runner, roots: &BTreeSet<String>) -> Result<BTreeMap<String, u64>> {
    if roots.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut args: Vec<String> = ["path-info", "-r", "--json"].map(String::from).into();
    args.extend(roots.iter().cloned());
    let text = run(&nix(args))?;
    let sizes =
        path_info(&serde_json::from_slice(&text).map_err(|err| format!("nix path-info: {err}"))?)?;
    if let Some(root) = roots.iter().find(|root| !sizes.contains_key(*root)) {
        return Err(format!("nix path-info left out the root {root}"));
    }
    Ok(sizes)
}

fn store_names(store_dir: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    if !store_dir.exists() {
        return Ok(names);
    }
    for entry in
        std::fs::read_dir(store_dir).map_err(|err| format!("{}: {err}", store_dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != ".links" {
            names.insert(format!("{STORE}/{name}"));
        }
    }
    Ok(names)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).map_err(|err| format!("{}: {err}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|err| format!("{}: {err}", path.display()))
}

pub struct StoreOptions {
    pub flake: PathBuf,
    pub dev_shell: String,
    pub out: PathBuf,
    pub exclude: Vec<String>,
    pub pass: Vec<String>,
    pub test: Test,
}

/// `agent-jail task-env store`: evaluates the dev shell on the host, writes
/// env.json and tools.json, and copies both closures into
/// `<out>/store`, which then holds exactly them. A re-run may change only
/// tools.json: it copies new tool paths and deletes stale ones.
pub fn store(
    options: &StoreOptions,
    self_exe: &Path,
    floor: u64,
    run: &mut Runner,
) -> Result<Value> {
    for pname in &options.exclude {
        if pname.is_empty()
            || !pname
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte))
        {
            return Err(format!("--exclude {pname} is not a package name"));
        }
    }
    for pattern in &options.pass {
        check_pattern(pattern)?;
    }
    if options.test.command.is_empty() || options.test.build.is_empty() {
        return Err("--test-command and --test-build must each name a command".into());
    }
    let sorted = |list: &[String]| -> Vec<String> {
        list.iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let (exclude, pass) = (sorted(&options.exclude), sorted(&options.pass));
    refuse_links(&options.out)?;
    std::fs::create_dir_all(&options.out)
        .map_err(|err| format!("{}: {err}", options.out.display()))?;
    let out = std::path::absolute(&options.out).map_err(|err| err.to_string())?;
    let _lock = lock(&out, true)?;

    let flake = std::path::absolute(&options.flake).map_err(|err| err.to_string())?;
    let dev = run(&nix([
        "print-dev-env".to_owned(),
        "--json".to_owned(),
        format!("{}#{}", flake.display(), options.dev_shell),
    ]))?;
    let dev: Value = serde_json::from_slice(&dev).map_err(|err| format!("print-dev-env: {err}"))?;
    let filtered = filter(&dev, &pass, &exclude)?;
    let mut dev_roots = BTreeSet::new();
    for text in filtered.variables.values().chain(&filtered.path) {
        dev_roots.extend(store_paths_in(text));
    }
    let dev_closure = closure(run, &dev_roots)?;
    let tool_roots = crate::task_env::tool_roots(self_exe)?;
    let tool_closure = closure(run, &tool_roots)?;
    for (what, paths) in [("dev", &dev_closure), ("tool", &tool_closure)] {
        if let Some(path) = paths.keys().find(|path| excluded(path, &exclude)) {
            return Err(format!(
                "the {what} closure still holds the excluded {path}"
            ));
        }
    }

    let env = EnvFile {
        schema: ENV_SCHEMA.into(),
        dev_shell: options.dev_shell.clone(),
        exclude,
        pass,
        variables: filtered.variables,
        dropped: filtered.dropped,
        path: filtered.path,
        test: options.test.clone(),
        roots: dev_roots.iter().cloned().collect(),
        closure: dev_closure.keys().cloned().collect(),
    };
    let env_bytes = json_bytes(&env)?;
    let env_path = out.join("env.json");
    if std::fs::symlink_metadata(&env_path).is_ok() {
        let (on_disk, _) = EnvFile::read(&out)?;
        if on_disk != env_bytes {
            return Err(format!(
                "{} differs from the recomputed one; a re-run may change only tools.json (make a new task environment)",
                env_path.display()
            ));
        }
    }
    let mut union = dev_closure.clone();
    union.extend(
        tool_closure
            .iter()
            .map(|(path, size)| (path.clone(), *size)),
    );
    let bytes: u64 = union.values().sum();
    let free = crate::free_bytes(&out)?;
    if free < bytes.saturating_add(floor) {
        return Err(format!(
            "{} has {free} bytes free; the closure needs {bytes} plus the {floor}-byte floor",
            out.display()
        ));
    }

    let root = out.join("store");
    let url = format!("local?root={}", root.display());
    let mut copy: Vec<String> = ["copy", "--no-check-sigs", "--to", url.as_str()]
        .map(String::from)
        .into();
    copy.extend(dev_roots.iter().chain(&tool_roots).cloned());
    run(&nix(copy))?;
    let store_dir = root.join("nix/store");
    let stale: Vec<String> = store_names(&store_dir)?
        .into_iter()
        .filter(|path| !union.contains_key(path))
        .collect();
    if !stale.is_empty() {
        // Nothing kept refers to a stale path (the kept set is closed under
        // references), so liveness has nothing to protect here.
        let mut delete: Vec<String> = [
            "store",
            "delete",
            "--ignore-liveness",
            "--store",
            url.as_str(),
        ]
        .map(String::from)
        .into();
        delete.extend(stale.iter().cloned());
        run(&nix(delete))?;
    }
    verify_store(&store_dir, &union.keys().map(String::as_str).collect())?;
    // Before env.json: a directory with one always has the whole layout,
    // which `task-env clean` asks for.
    for base in ["cargo", "target"] {
        std::fs::create_dir_all(out.join(base)).map_err(|err| format!("{base}: {err}"))?;
    }
    if std::fs::symlink_metadata(&env_path).is_err() {
        write_atomic(&env_path, &env_bytes)?;
    }
    let tools = ToolsFile {
        schema: TOOLS_SCHEMA.into(),
        roots: tool_roots.into_iter().collect(),
        closure: tool_closure.keys().cloned().collect(),
    };
    write_atomic(&out.join("tools.json"), &json_bytes(&tools)?)?;
    Ok(json!({
        "env_sha256": crate::sha256_hex(&env_bytes),
        "dev_paths": dev_closure.len(),
        "tool_paths": tool_closure.len(),
        "bytes": bytes,
    }))
}

pub struct CargoOptions {
    pub out: PathBuf,
    pub lockfiles: Vec<String>,
    /// Tree name (`base`, `landed`) to its directory.
    pub trees: Vec<(String, PathBuf)>,
}

/// The `.cargo/config*` files Cargo would read above `dir`.
pub fn ancestor_configs(dir: &Path) -> Vec<PathBuf> {
    dir.ancestors()
        .skip(1)
        .flat_map(|ancestor| {
            ["config", "config.toml"].map(|name| ancestor.join(".cargo").join(name))
        })
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .collect()
}

fn bin_dir(path: &[String], name: &str) -> Result<String> {
    path.iter()
        .find(|dir| Path::new(dir).join(name).is_file())
        .cloned()
        .ok_or(format!("env.json's PATH has no {name}"))
}

/// `agent-jail task-env cargo`: fills `<out>/cargo` with every lockfile of
/// every tree, fetched on the host with a pinned environment: nothing but
/// env.json's cargo and git on PATH, an empty HOME, no host Git config, the
/// host CA bundle, and the tree as the working directory, whose ancestors
/// may hold no Cargo config. Returns each tree's lockfile digests.
pub fn cargo(options: &CargoOptions, ca_bundle: &Path, run: &mut Runner) -> Result<Value> {
    refuse_links(&options.out)?;
    let out = std::path::absolute(&options.out).map_err(|err| err.to_string())?;
    let _lock = lock(&out, true)?;
    let (_, env) = EnvFile::read(&out)?;
    let cargo_dir = bin_dir(&env.path, "cargo")?;
    let git_dir = bin_dir(&env.path, "git")?;
    let path = if cargo_dir == git_dir {
        cargo_dir.clone()
    } else {
        format!("{cargo_dir}:{git_dir}")
    };
    let cargo_home = out.join("cargo");
    std::fs::create_dir_all(&cargo_home)
        .map_err(|err| format!("{}: {err}", cargo_home.display()))?;
    for lockfile in &options.lockfiles {
        let plain = Path::new(lockfile)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !plain || lockfile.is_empty() {
            return Err(format!(
                "--lockfile {lockfile} must be a plain relative path"
            ));
        }
    }
    if options.lockfiles.is_empty() || options.trees.is_empty() {
        return Err("task-env cargo needs at least one --lockfile and one --tree".into());
    }
    let mut digests: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (name, dir) in &options.trees {
        if digests.contains_key(name) {
            return Err(format!("--tree {name} given twice"));
        }
        let tree = std::fs::canonicalize(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let configs = ancestor_configs(&tree);
        if !configs.is_empty() {
            return Err(format!(
                "{configs:?} would steer the fetch for {}; the jail never sees them, so they are refused",
                tree.display()
            ));
        }
        let mut sides = BTreeMap::new();
        for lockfile in &options.lockfiles {
            let lock_path = tree.join(lockfile);
            sides.insert(lockfile.clone(), crate::sha256_file(&lock_path)?);
            let manifest = lock_path
                .parent()
                .map(|parent| parent.join("Cargo.toml"))
                .ok_or("a lockfile has no directory")?;
            let home = out.join(".fetch-home");
            if std::fs::symlink_metadata(&home).is_ok() {
                std::fs::remove_dir_all(&home)
                    .map_err(|err| format!("{}: {err}", home.display()))?;
            }
            std::fs::create_dir(&home).map_err(|err| format!("{}: {err}", home.display()))?;
            let fetched = run(&Cmd {
                program: Path::new(&cargo_dir).join("cargo"),
                args: vec![
                    "fetch".into(),
                    "--locked".into(),
                    "--manifest-path".into(),
                    manifest.display().to_string(),
                ],
                env: Some(
                    [
                        ("PATH", path.clone()),
                        ("HOME", home.display().to_string()),
                        ("CARGO_HOME", cargo_home.display().to_string()),
                        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
                        ("GIT_CONFIG_NOSYSTEM", "1".into()),
                        ("SSL_CERT_FILE", ca_bundle.display().to_string()),
                    ]
                    .map(|(key, value)| (key.to_owned(), value))
                    .into(),
                ),
                cwd: Some(tree.clone()),
            });
            let removed = std::fs::remove_dir_all(&home);
            fetched?;
            removed.map_err(|err| format!("{}: {err}", home.display()))?;
        }
        digests.insert(name.clone(), sides);
    }
    Ok(json!(digests))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_env::tests::{HASH, path};

    fn dev(variables: Value) -> Value {
        json!({ "variables": variables, "bashFunctions": {} })
    }

    fn exported(value: &str) -> Value {
        json!({ "type": "exported", "value": value })
    }

    fn excludes() -> Vec<String> {
        vec!["br".into(), "bv".into()]
    }

    #[test]
    fn the_allowlist_keeps_and_everything_else_drops() {
        let filtered = filter(
            &dev(json!({
                "CC": exported(&format!("{}/bin/cc", path("gcc-wrapper"))),
                "CC_FOR_BUILD": exported("gcc"),
                "LD_FOR_BUILD": exported("ld"),
                "NIX_CFLAGS_COMPILE": exported(&format!("-isystem {}/include", path("zlib-dev"))),
                "NIX_REMOTE": exported("daemon"),
                "NIX_ENFORCE_NO_NATIVE": exported("1"),
                "NIX_ENFORCE_PURITY": exported("1"),
                "GHOSTTY_SOURCE_DIR": exported(&path("source")),
                "PATH": exported(&format!("{0}/bin:{1}/bin:{2}/bin:{0}/bin", path("rust"), path("br-0.7.2"), path("brotli-1.2.0"))),
                "SOME_OTHER": exported("x"),
                "shellHook": json!({ "type": "var", "value": "" }),
                "ARRAY": json!({ "type": "array", "value": ["a"] }),
            })),
            &["GHOSTTY_*".into()],
            &excludes(),
        )
        .unwrap();
        assert_eq!(
            filtered.variables.keys().collect::<Vec<_>>(),
            [
                "CC",
                "CC_FOR_BUILD",
                "GHOSTTY_SOURCE_DIR",
                "LD_FOR_BUILD",
                "NIX_CFLAGS_COMPILE",
                "NIX_ENFORCE_NO_NATIVE"
            ]
        );
        assert_eq!(
            filtered.dropped,
            ["NIX_ENFORCE_PURITY", "NIX_REMOTE", "SOME_OTHER"]
        );
        // br leaves PATH, brotli stays, duplicates go.
        assert_eq!(
            filtered.path,
            [
                format!("{}/bin", path("rust")),
                format!("{}/bin", path("brotli-1.2.0"))
            ]
        );
    }

    #[test]
    fn a_hard_deny_beats_a_pattern_and_bad_values_are_refused() {
        let refused = |variables: Value, pass: &[&str]| {
            let pass: Vec<String> = pass.iter().map(|p| (*p).to_owned()).collect();
            filter(&dev(variables), &pass, &excludes()).unwrap_err()
        };
        assert!(
            refused(json!({"CARGO_HOME": exported("/x")}), &["CARGO_*"]).contains("CARGO_HOME")
        );
        assert!(refused(json!({"NIX_PATH": exported("x")}), &["NIX_*"]).contains("NIX_PATH"));
        assert!(refused(json!({"LD_PRELOAD": exported("x")}), &["LD_*"]).contains("LD_PRELOAD"));
        assert!(
            refused(json!({"https_proxy": exported("x")}), &["https_proxy"])
                .contains("https_proxy")
        );
        assert!(
            refused(json!({"EON_X": exported("/home/lucca/x")}), &["EON_*"]).contains("outside")
        );
        assert!(refused(json!({"NIX_LDFLAGS": exported("-L/usr/lib")}), &[]).contains("/usr/lib"));
        assert!(
            refused(json!({"NIX_STORE": exported("/elsewhere/store")}), &[]).contains("NIX_STORE")
        );
        assert!(refused(json!({"PATH": exported("/usr/bin")}), &[]).contains("PATH"));
        assert!(
            refused(
                json!({"shellHook": {"type": "var", "value": "echo hi"}}),
                &[]
            )
            .contains("shellHook")
        );
        // A denied name nothing keeps is only dropped.
        let quiet = filter(
            &dev(json!({"HOME": exported("/homeless-shelter")})),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(quiet.dropped, ["HOME"]);
        // The shell's own output: its self-rpath goes, any other mention is refused.
        let shell = path("nix-shell-env");
        let own = filter(
            &dev(json!({
                "out": { "type": "exported", "value": shell },
                "NIX_LDFLAGS": exported(&format!("-rpath {shell}/lib  -L{}/lib", path("zlib"))),
            })),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            own.variables["NIX_LDFLAGS"],
            format!("-L{}/lib", path("zlib"))
        );
        assert!(
            refused(
                json!({
                    "out": { "type": "exported", "value": shell },
                    "NIX_CFLAGS_COMPILE": exported(&format!("-I{shell}/include")),
                }),
                &[]
            )
            .contains("own output")
        );
        assert!(check_pattern("lower*").is_err() && check_pattern("*").is_err());
        assert!(check_pattern("EON_*").is_ok() && check_pattern("FONTCONFIG_FILE").is_ok());
    }

    #[test]
    fn path_info_reads_both_formats() {
        let array = json!([{ "path": path("a"), "narSize": 10 }]);
        let object = json!({ path("a"): { "narSize": 10 } });
        let bare = json!({ format!("{HASH}-a"): { "narSize": 10 } });
        for json in [array, object, bare] {
            assert_eq!(
                path_info(&json).unwrap(),
                BTreeMap::from([(path("a"), 10u64)])
            );
        }
        assert!(path_info(&json!({ path("a"): null })).is_err());
        assert!(path_info(&json!({ format!("{}/bin", path("a")): { "narSize": 1 } })).is_err());
    }

    /// A fake host: nix answers from fixed closures and `nix copy`
    /// materializes them; every command is recorded.
    struct FakeNix {
        dev: Value,
        closures: BTreeMap<String, Vec<String>>,
        commands: Vec<Cmd>,
        extra_on_copy: Option<String>,
    }

    impl FakeNix {
        fn run(&mut self, cmd: &Cmd) -> Result<Vec<u8>> {
            self.commands.push(cmd.clone());
            let args: Vec<&str> = cmd.args.iter().skip(2).map(String::as_str).collect();
            match args.first().copied() {
                Some("print-dev-env") => Ok(self.dev.to_string().into_bytes()),
                Some("path-info") => {
                    let mut sizes = serde_json::Map::new();
                    for root in &args[3..] {
                        for path in self
                            .closures
                            .get(*root)
                            .ok_or(format!("unknown root {root}"))?
                        {
                            sizes.insert(path.clone(), json!({ "narSize": 100 }));
                        }
                    }
                    Ok(Value::Object(sizes).to_string().into_bytes())
                }
                Some("copy") => {
                    let root = args[3].strip_prefix("local?root=").unwrap();
                    let store = Path::new(root).join("nix/store");
                    for path in args[4..].iter().flat_map(|root| &self.closures[*root]) {
                        let name = path.strip_prefix("/nix/store/").unwrap();
                        std::fs::create_dir_all(store.join(name)).unwrap();
                    }
                    if let Some(extra) = self.extra_on_copy.take() {
                        std::fs::create_dir_all(store.join(extra)).unwrap();
                    }
                    Ok(Vec::new())
                }
                Some("store") => {
                    let root = args[4].strip_prefix("local?root=").unwrap();
                    for path in &args[5..] {
                        let name = path.strip_prefix("/nix/store/").unwrap();
                        std::fs::remove_dir(Path::new(root).join("nix/store").join(name)).unwrap();
                    }
                    Ok(Vec::new())
                }
                _ => Err(format!("unexpected {args:?}")),
            }
        }
    }

    fn self_exe(dir: &Path) -> PathBuf {
        let exe = dir.join("agent-jail");
        let glibc = format!("{}/lib", path("glibc-2.42"));
        std::fs::write(
            &exe,
            crate::elf::tests::elf(
                Some(&format!("{glibc}/ld-linux-x86-64.so.2")),
                &[(29, &glibc)],
            ),
        )
        .unwrap();
        exe
    }

    fn fake(tool_roots: &BTreeSet<String>) -> FakeNix {
        let mut closures: BTreeMap<String, Vec<String>> = tool_roots
            .iter()
            .map(|root| (root.clone(), vec![root.clone()]))
            .collect();
        closures.insert(path("rust"), vec![path("rust"), path("glibc-2.42")]);
        closures.insert(path("cc"), vec![path("cc")]);
        FakeNix {
            dev: dev(json!({
                "CC": exported(&format!("{}/bin/cc", path("cc"))),
                "PATH": exported(&format!("{}/bin", path("rust"))),
            })),
            closures,
            commands: Vec::new(),
            extra_on_copy: None,
        }
    }

    fn options(out: &Path) -> StoreOptions {
        StoreOptions {
            flake: out.join("flake"),
            dev_shell: "devShells.x86_64-linux.default".into(),
            out: out.to_path_buf(),
            exclude: vec!["bv".into(), "br".into()],
            pass: vec![],
            test: Test {
                command: vec!["eon-test".into()],
                build: vec!["eon-test".into(), "list".into()],
            },
        }
    }

    #[test]
    fn a_store_build_writes_env_json_and_holds_exactly_its_closures() {
        let dir = crate::test_dir("task-store");
        let exe = self_exe(&dir);
        let tool_roots = crate::task_env::tool_roots(&exe).unwrap();
        let out = dir.join("env");
        let mut nix = fake(&tool_roots);
        nix.extra_on_copy = Some(format!("{HASH}-leftover"));
        let printed = store(&options(&out), &exe, 0, &mut |cmd| nix.run(cmd)).unwrap();
        // The extra path a copy left was deleted again.
        assert!(
            nix.commands
                .iter()
                .any(|cmd| cmd.args.contains(&"--ignore-liveness".to_owned())
                    && cmd.args.contains(&format!("/nix/store/{HASH}-leftover")))
        );
        let env = crate::task_env::TaskEnv::load(&out).unwrap();
        assert_eq!(env.path(), [format!("{}/bin", path("rust"))]);
        assert_eq!(env.exclude(), ["br", "bv"]);
        let bytes = std::fs::read(out.join("env.json")).unwrap();
        assert_eq!(printed["env_sha256"], crate::sha256_hex(&bytes));
        assert!(bytes.ends_with(b"}\n"));
        let text = String::from_utf8(bytes.clone()).unwrap();
        let order: Vec<usize> = [
            "\"schema\"",
            "\"dev_shell\"",
            "\"exclude\"",
            "\"pass\"",
            "\"variables\"",
            "\"dropped\"",
            "\"path\"",
            "\"test\"",
            "\"roots\"",
            "\"closure\"",
        ]
        .iter()
        .map(|key| text.find(key).unwrap())
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(env.check_tools(&exe).is_ok());

        // A re-run with the same inputs is a no-op on env.json.
        let mut again = fake(&tool_roots);
        store(&options(&out), &exe, 0, &mut |cmd| again.run(cmd)).unwrap();
        assert_eq!(std::fs::read(out.join("env.json")).unwrap(), bytes);
        // A re-run whose env.json would differ is refused.
        let mut changed = fake(&tool_roots);
        changed.dev = dev(json!({ "PATH": exported(&format!("{}/bin", path("cc"))) }));
        assert!(
            store(&options(&out), &exe, 0, &mut |cmd| changed.run(cmd))
                .unwrap_err()
                .contains("differs")
        );
        // The free-space floor refuses before anything is copied.
        let mut big = fake(&tool_roots);
        assert!(
            store(&options(&out), &exe, u64::MAX / 2, &mut |cmd| big.run(cmd))
                .unwrap_err()
                .contains("free")
        );
        assert!(
            !big.commands
                .iter()
                .any(|cmd| cmd.args.contains(&"copy".to_owned()))
        );
    }

    #[test]
    fn a_closure_still_holding_an_excluded_package_is_refused() {
        let dir = crate::test_dir("task-store-exclude");
        let exe = self_exe(&dir);
        let tool_roots = crate::task_env::tool_roots(&exe).unwrap();
        let mut nix = fake(&tool_roots);
        nix.closures
            .insert(path("rust"), vec![path("rust"), path("br-0.7.2")]);
        let err = store(&options(&dir.join("env")), &exe, 0, &mut |cmd| nix.run(cmd)).unwrap_err();
        assert!(err.contains("br-0.7.2"), "{err}");
    }

    #[test]
    fn the_cargo_fetch_runs_in_a_pinned_environment() {
        let dir = crate::test_dir("task-cargo");
        let env = crate::task_env::tests::fixture(&dir.join("env"), &["cc", "tool"]);
        // env.json names a PATH dir with cargo and git.
        let bin = dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        for name in ["cargo", "git"] {
            std::fs::write(bin.join(name), "").unwrap();
        }
        let (_, mut file) = EnvFile::read(env.dir()).unwrap();
        file.path = vec![bin.display().to_string()];
        std::fs::write(env.dir().join("env.json"), json_bytes(&file).unwrap()).unwrap();
        let tree = dir.join("base");
        std::fs::create_dir_all(tree.join("installer")).unwrap();
        std::fs::write(tree.join("Cargo.lock"), "a").unwrap();
        std::fs::write(tree.join("installer/Cargo.lock"), "b").unwrap();
        let mut seen = Vec::new();
        let options = CargoOptions {
            out: env.dir().to_path_buf(),
            lockfiles: vec!["Cargo.lock".into(), "installer/Cargo.lock".into()],
            trees: vec![("base".into(), tree.clone())],
        };
        let printed = cargo(&options, Path::new("/etc/ssl/ca.crt"), &mut |cmd| {
            seen.push(cmd.clone());
            Ok(Vec::new())
        })
        .unwrap();
        assert_eq!(printed["base"]["Cargo.lock"], crate::sha256_hex(b"a"));
        assert_eq!(
            printed["base"]["installer/Cargo.lock"],
            crate::sha256_hex(b"b")
        );
        assert_eq!(seen.len(), 2);
        let tree = std::fs::canonicalize(&tree).unwrap();
        let home = env.dir().join(".fetch-home").display().to_string();
        let expected: BTreeMap<String, String> = [
            ("PATH", bin.display().to_string()),
            ("HOME", home),
            ("CARGO_HOME", env.dir().join("cargo").display().to_string()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("SSL_CERT_FILE", "/etc/ssl/ca.crt".into()),
        ]
        .map(|(key, value)| (key.to_owned(), value))
        .into();
        assert_eq!(seen[1].env.as_ref(), Some(&expected));
        assert_eq!(seen[1].cwd.as_deref(), Some(tree.as_path()));
        let manifest = tree.join("installer/Cargo.toml").display().to_string();
        assert_eq!(
            seen[1].args,
            ["fetch", "--locked", "--manifest-path", manifest.as_str()]
        );
        assert_eq!(seen[1].program, bin.join("cargo"));
        assert!(!env.dir().join(".fetch-home").exists());

        // A Cargo config above the tree is refused before any fetch.
        let nested = dir.join("outer/inner");
        std::fs::create_dir_all(dir.join("outer/.cargo")).unwrap();
        std::fs::write(dir.join("outer/.cargo/config.toml"), "").unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Cargo.lock"), "a").unwrap();
        let refused = CargoOptions {
            out: env.dir().to_path_buf(),
            lockfiles: vec!["Cargo.lock".into()],
            trees: vec![("landed".into(), nested)],
        };
        let mut ran = false;
        let err = cargo(&refused, Path::new("/x"), &mut |_| {
            ran = true;
            Ok(Vec::new())
        })
        .unwrap_err();
        assert!(err.contains("config.toml") && !ran, "{err}");
        let escape = CargoOptions {
            lockfiles: vec!["../Cargo.lock".into()],
            ..options
        };
        assert!(cargo(&escape, Path::new("/x"), &mut |_| Ok(Vec::new())).is_err());
    }
}
