//! Host tools by absolute path and SHA-256 (`host-tools.toml`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Result;

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

/// The version a pin records, for records (`tools.dcg.version`).
pub fn pinned_version(name: &str) -> Result<String> {
    let file: PinsFile =
        toml::from_str(HOST_TOOLS).map_err(|err| format!("host-tools.toml: {err}"))?;
    file.tools
        .get(name)
        .and_then(|tool| tool.version.clone())
        .ok_or(format!("host-tools.toml records no version for {name}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inner {
    packages: Vec<String>,
}

/// Verified host tools. The CLI builds this only with [`Pins::load`]; tests
/// fill the fields directly.
#[derive(Clone, Debug)]
pub struct Pins {
    /// Tool name (`bwrap`, `dcg`, `claude`, `codex`, `ca-bundle`) to path.
    pub tools: BTreeMap<String, PathBuf>,
    /// The inner layer's tools: wrapper name to the real executable.
    pub inner: BTreeMap<String, PathBuf>,
}

impl Pins {
    /// Parses the shipped pins and verifies the tools in `needed`; the inner
    /// packages are always resolved.
    pub fn load(needed: &[&str]) -> Result<Self> {
        let file: PinsFile =
            toml::from_str(HOST_TOOLS).map_err(|err| format!("host-tools.toml: {err}"))?;
        let home = passwd_home()?;
        let mut tools = BTreeMap::new();
        for name in needed {
            let tool = file
                .tools
                .get(*name)
                .ok_or(format!("host-tools.toml pins no tool named {name}"))?;
            let path = expand(&tool.path, &home)?;
            let metadata = std::fs::metadata(&path)
                .map_err(|err| format!("pinned {name} {}: {err}", path.display()))?;
            if !metadata.is_file() {
                return Err(format!("pinned {name} {} is not a file", path.display()));
            }
            if tool.sha256 == "any" {
                if *name != "ca-bundle" {
                    return Err(format!("pinned {name} must name a SHA-256"));
                }
            } else {
                let actual = crate::sha256_file(&path)?;
                if actual != tool.sha256 {
                    return Err(format!(
                        "pinned {name} {} has SHA-256 {actual}, not the pinned {}",
                        path.display(),
                        tool.sha256
                    ));
                }
            }
            if *name == "bwrap" && !path.starts_with("/nix/store") {
                return Err("the pinned bwrap must live in /nix/store".into());
            }
            tools.insert((*name).to_owned(), path);
        }
        let packages: Vec<PathBuf> = file.inner.packages.iter().map(PathBuf::from).collect();
        Ok(Self {
            tools,
            inner: inner_tools(&packages)?,
        })
    }

    pub fn tool(&self, name: &str) -> Result<&Path> {
        self.tools
            .get(name)
            .map(PathBuf::as_path)
            .ok_or(format!("no pinned tool named {name}"))
    }
}

/// Every executable in each package's `bin/`, first package first.
pub fn inner_tools(packages: &[PathBuf]) -> Result<BTreeMap<String, PathBuf>> {
    let mut tools = BTreeMap::new();
    for package in packages {
        if !package.starts_with("/nix/store") {
            return Err(format!(
                "inner package {} is not in /nix/store",
                package.display()
            ));
        }
        let bin = package.join("bin");
        let entries = std::fs::read_dir(&bin)
            .map_err(|err| format!("inner package {}: {err}", bin.display()))?;
        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(real) = std::fs::canonicalize(entry.path()) else {
                continue;
            };
            let executable = std::fs::metadata(&real).is_ok_and(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            });
            if executable && is_tool_name(&name) {
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
        for name in ["bwrap", "dcg", "claude", "codex", "ca-bundle"] {
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
    fn tool_names_are_plain() {
        assert!(is_tool_name("git-upload-pack") && is_tool_name("[") && is_tool_name("g++"));
        assert!(!is_tool_name("a/b") && !is_tool_name("..") && !is_tool_name(""));
    }
}
