//! Host tools by absolute path and SHA-256 (`host-tools.toml`). A pin in
//! /nix/store resolves inside the store a jail mounts there (the task
//! store, or the host's for tests), following links only within it, and its
//! SHA-256 is checked on that copy. Other pins (the Codex release, the CA
//! bundle) are host paths.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Result;
use crate::task_env::{STORE, package_of};

/// The shipped pins file.
pub const HOST_TOOLS: &str = include_str!("../host-tools.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinsFile {
    tools: BTreeMap<String, Tool>,
    inner: Inner,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    path: String,
    sha256: String,
    #[serde(default)]
    version: Option<String>,
}

fn pins_file() -> Result<PinsFile> {
    toml::from_str(HOST_TOOLS).map_err(|err| format!("host-tools.toml: {err}"))
}

/// The version a pin records, for records (`tools.dcg.version`).
pub fn pinned_version(name: &str) -> Result<String> {
    pins_file()?
        .tools
        .get(name)
        .and_then(|tool| tool.version.clone())
        .ok_or(format!("host-tools.toml records no version for {name}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inner {
    packages: Vec<String>,
}

/// The store package of every store pin and every inner package: the
/// task store's tool roots, with agent-jail's own runtime references.
pub fn store_packages() -> Result<BTreeSet<String>> {
    let file = pins_file()?;
    let mut packages = BTreeSet::new();
    for path in file
        .tools
        .values()
        .map(|tool| &tool.path)
        .chain(&file.inner.packages)
        .filter(|path| path.starts_with(&format!("{STORE}/")))
    {
        packages.insert(package_of(path).ok_or(format!("pin {path} is not in a store package"))?);
    }
    Ok(packages)
}

/// Verified tools. The CLI builds this only with [`Pins::load`]; tests
/// fill the fields directly.
#[derive(Clone, Debug)]
pub struct Pins {
    /// The host directory mounted at /nix/store.
    pub store: PathBuf,
    /// Tool name (`bwrap`, `dcg`, `bash`, `nix`, `claude`, `codex`,
    /// `ca-bundle`) to its path: a /nix/store path as the jail sees it, or
    /// a host path for a pin outside the store.
    pub tools: BTreeMap<String, PathBuf>,
    /// The inner layer's tools: wrapper name to the real executable, as the
    /// jail sees it.
    pub inner: BTreeMap<String, PathBuf>,
}

impl Pins {
    /// Parses the shipped pins and verifies the tools in `needed` against
    /// `store` (the host directory mounted at /nix/store); the inner
    /// packages are always resolved.
    pub fn load(needed: &[&str], store: &Path) -> Result<Self> {
        let file = pins_file()?;
        let home = passwd_home()?;
        let mut tools = BTreeMap::new();
        for name in needed {
            let tool = file
                .tools
                .get(*name)
                .ok_or(format!("host-tools.toml pins no tool named {name}"))?;
            let pinned = expand(&tool.path, &home)?;
            let path = if tool.path.starts_with(&format!("{STORE}/")) {
                PathBuf::from(resolve(store, &tool.path)?)
            } else {
                pinned
            };
            let host = host_path(store, &path);
            let metadata = std::fs::metadata(&host)
                .map_err(|err| format!("pinned {name} {}: {err}", host.display()))?;
            if !metadata.is_file() {
                return Err(format!("pinned {name} {} is not a file", host.display()));
            }
            if tool.sha256 == "any" {
                if *name != "ca-bundle" {
                    return Err(format!("pinned {name} must name a SHA-256"));
                }
            } else {
                let actual = crate::sha256_file(&host)?;
                if actual != tool.sha256 {
                    return Err(format!(
                        "pinned {name} {} has SHA-256 {actual}, not the pinned {}",
                        host.display(),
                        tool.sha256
                    ));
                }
            }
            if *name == "bwrap" && !path.starts_with(STORE) {
                return Err("the pinned bwrap must live in /nix/store".into());
            }
            tools.insert((*name).to_owned(), path);
        }
        Ok(Self {
            inner: inner_tools(&file.inner.packages, store)?,
            store: store.to_path_buf(),
            tools,
        })
    }

    /// The tool's path as the jail sees it (a host path off the store).
    pub fn tool(&self, name: &str) -> Result<&Path> {
        self.tools
            .get(name)
            .map(PathBuf::as_path)
            .ok_or(format!("no pinned tool named {name}"))
    }

    /// The tool's file on the host.
    pub fn host(&self, name: &str) -> Result<PathBuf> {
        Ok(host_path(&self.store, self.tool(name)?))
    }
}

/// The host file behind `path`: a /nix/store path lies under `store`.
pub fn host_path(store: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(STORE) {
        Ok(rest) if path.starts_with(format!("{STORE}/")) => store.join(rest),
        _ => path.to_path_buf(),
    }
}

/// Resolves the /nix/store `path` inside `store`, following links only
/// while they stay in the store, and returns the final path as the jail
/// sees it.
pub fn resolve(store: &Path, path: &str) -> Result<String> {
    let leaves = |what: &str| format!("{path}: {what} leaves {STORE}");
    let inside = |text: &str| -> Result<String> {
        text.strip_prefix(&format!("{STORE}/"))
            .map(str::to_owned)
            .ok_or_else(|| leaves(text))
    };
    let mut pending: Vec<String> = inside(path)?
        .split('/')
        .filter(|part| !part.is_empty())
        .rev()
        .map(str::to_owned)
        .collect();
    let mut done: Vec<String> = Vec::new();
    let mut links = 0;
    while let Some(part) = pending.pop() {
        match part.as_str() {
            "." => continue,
            ".." => {
                done.pop().ok_or_else(|| leaves(".."))?;
                continue;
            }
            _ => {}
        }
        let host = store.join(done.join("/")).join(&part);
        let metadata = std::fs::symlink_metadata(&host)
            .map_err(|err| format!("{path}: {}: {err}", host.display()))?;
        if !metadata.file_type().is_symlink() {
            done.push(part);
            continue;
        }
        links += 1;
        if links > 40 {
            return Err(format!("{path}: too many links"));
        }
        let target =
            std::fs::read_link(&host).map_err(|err| format!("{}: {err}", host.display()))?;
        let target = target.to_str().ok_or_else(|| leaves("a non-UTF-8 link"))?;
        let rest = if target.starts_with('/') {
            done.clear();
            inside(target)?
        } else {
            target.to_owned()
        };
        pending.extend(
            rest.split('/')
                .filter(|part| !part.is_empty())
                .rev()
                .map(str::to_owned),
        );
    }
    if done.is_empty() {
        return Err(leaves("the store itself"));
    }
    Ok(format!("{STORE}/{}", done.join("/")))
}

/// Every executable in each package's `bin/`, first package first, as the
/// jail sees it; a `bin/` link that leaves the store is refused.
pub fn inner_tools(packages: &[String], store: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut tools = BTreeMap::new();
    for package in packages {
        if package_of(package).as_deref() != Some(package.as_str()) {
            return Err(format!("inner package {package} is not a store package"));
        }
        let bin = host_path(store, &Path::new(package).join("bin"));
        let entries = std::fs::read_dir(&bin)
            .map_err(|err| format!("inner package {}: {err}", bin.display()))?;
        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !is_tool_name(&name) {
                continue;
            }
            let real = PathBuf::from(resolve(store, &format!("{package}/bin/{name}"))?);
            let executable = std::fs::metadata(host_path(store, &real)).is_ok_and(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            });
            if executable {
                names.push((name, real));
            }
        }
        names.sort();
        for (name, real) in names {
            tools.entry(name).or_insert(real);
        }
    }
    Ok(tools)
}

/// Names that can be a file in `/bench/bin`: no slash, no dot-only names.
pub fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+' | b'[')
        })
}

fn expand(path: &str, home: &Path) -> Result<PathBuf> {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    };
    if !expanded.is_absolute() {
        return Err(format!("pinned path {path} is not absolute"));
    }
    Ok(expanded)
}

/// The invoking user's home from `/etc/passwd`.
pub fn passwd_home() -> Result<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let passwd =
        std::fs::read_to_string("/etc/passwd").map_err(|err| format!("/etc/passwd: {err}"))?;
    passwd
        .lines()
        .find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid))
                .then(|| PathBuf::from(fields[5]))
        })
        .ok_or(format!("/etc/passwd has no entry for uid {uid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_pins_parse() {
        let file: PinsFile = toml::from_str(HOST_TOOLS).unwrap();
        for name in [
            "bwrap",
            "dcg",
            "bash",
            "nix",
            "claude",
            "codex",
            "ca-bundle",
        ] {
            assert!(file.tools.contains_key(name), "{name}");
        }
        for (name, tool) in &file.tools {
            assert!(
                tool.sha256 == "any" && name == "ca-bundle" || tool.sha256.len() == 64,
                "{name}"
            );
        }
    }

    #[test]
    fn home_expansion_uses_the_password_database() {
        let home = Path::new("/home/someone");
        assert_eq!(
            expand("~/x/y", home).unwrap(),
            Path::new("/home/someone/x/y")
        );
        assert_eq!(
            expand("/nix/store/a", home).unwrap(),
            Path::new("/nix/store/a")
        );
        assert!(expand("relative", home).is_err());
    }

    #[test]
    fn store_packages_cover_every_store_pin() {
        let packages = store_packages().unwrap();
        assert!(
            packages
                .iter()
                .all(|package| package_of(package).as_deref() == Some(package.as_str()))
        );
        for needle in ["-bubblewrap-", "-dcg-", "-bash-", "-nix-", "-coreutils-"] {
            assert!(
                packages.iter().any(|package| package.contains(needle)),
                "{needle}"
            );
        }
        assert!(!packages.iter().any(|package| package.contains("codex")));
    }

    #[test]
    fn links_resolve_only_inside_the_store() {
        let store = crate::test_dir("pins-resolve");
        let hash = crate::task_env::tests::HASH;
        let package = store.join(format!("{hash}-tool/bin"));
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir_all(store.join(format!("{hash}-other/bin"))).unwrap();
        std::fs::write(store.join(format!("{hash}-other/bin/real")), "x").unwrap();
        let link = |name: &str, target: &str| {
            std::os::unix::fs::symlink(target, package.join(name)).unwrap()
        };
        link("relative", &format!("../../{hash}-other/bin/real"));
        link("absolute", &format!("/nix/store/{hash}-other/bin/real"));
        link("host", "/etc/passwd");
        link("climb", "../../../../etc/passwd");
        let tool = |name: &str| format!("/nix/store/{hash}-tool/bin/{name}");
        let real = format!("/nix/store/{hash}-other/bin/real");
        assert_eq!(resolve(&store, &tool("relative")).unwrap(), real);
        assert_eq!(resolve(&store, &tool("absolute")).unwrap(), real);
        assert!(
            resolve(&store, &tool("host"))
                .unwrap_err()
                .contains("leaves")
        );
        assert!(
            resolve(&store, &tool("climb"))
                .unwrap_err()
                .contains("leaves")
        );
        assert!(resolve(&store, "/usr/bin/env").is_err());
        assert_eq!(
            host_path(&store, Path::new(&real)),
            store.join(format!("{hash}-other/bin/real"))
        );
        assert_eq!(host_path(&store, Path::new("/etc/x")), Path::new("/etc/x"));
        // A bin/ link out of the store makes the package unusable.
        assert!(inner_tools(&[format!("/nix/store/{hash}-tool")], &store).is_err());
        std::fs::remove_file(package.join("host")).unwrap();
        std::fs::remove_file(package.join("climb")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            store.join(format!("{hash}-other/bin/real")),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let tools = inner_tools(&[format!("/nix/store/{hash}-tool")], &store).unwrap();
        assert_eq!(tools["relative"], Path::new(&real));
    }

    #[test]
    fn tool_names_are_plain() {
        assert!(is_tool_name("git-upload-pack") && is_tool_name("[") && is_tool_name("g++"));
        assert!(!is_tool_name("a/b") && !is_tool_name("..") && !is_tool_name(""));
    }
}
