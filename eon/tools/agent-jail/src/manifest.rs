//! Fresh harness homes from `homes/<harness>/manifest.json`
//! (`agent-jail-home/1`). Policy entries are read-only file mounts; only
//! `state` entries are writable; `/home/bench` is remounted read-only.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

pub const SCHEMA: &str = "agent-jail-home/1";

/// A shipped home: harness, manifest, then each source file by name.
pub type ShippedHome = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// The shipped homes.
pub const HOMES: &[ShippedHome] = &[
    (
        "claude",
        include_str!("../homes/claude/manifest.json"),
        &[
            (
                "managed-settings.json",
                include_str!("../homes/claude/managed-settings.json"),
            ),
            (
                "settings.json",
                include_str!("../homes/claude/settings.json"),
            ),
            ("claude.json", include_str!("../homes/claude/claude.json")),
        ],
    ),
    (
        "codex",
        include_str!("../homes/codex/manifest.json"),
        &[
            (
                "requirements.toml",
                include_str!("../homes/codex/requirements.toml"),
            ),
            (
                "managed_config.toml",
                include_str!("../homes/codex/managed_config.toml"),
            ),
            (
                "installation_id",
                include_str!("../homes/codex/installation_id"),
            ),
        ],
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Settings,
    Hooks,
    Instructions,
    Skills,
    Plugins,
    Mcp,
    State,
    Dir,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub kind: Kind,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind_of_file: Option<String>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind_of_file.as_deref() == Some("dir")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub harness: String,
    pub entries: Vec<Entry>,
    pub auth: String,
    pub managed: Vec<String>,
    pub skills: Vec<String>,
    pub plugins: Vec<String>,
    pub mcp: Vec<String>,
    pub instructions: Vec<String>,
}

/// Keys a state file may not hold: they would be policy.
const POLICY_KEYS: &[&str] = &[
    "permissions",
    "hooks",
    "mcpServers",
    "mcp_servers",
    "enabledMcpjsonServers",
    "allowedTools",
    "disallowedTools",
    "apiKeyHelper",
    "env",
    "sandbox",
    "sandbox_mode",
    "approval_policy",
    "web_search",
    "tools",
    "features",
    "profiles",
    "model_providers",
    "enabledPlugins",
];

/// A loaded home: the manifest, its bytes' digest and its source files.
#[derive(Clone, Debug)]
pub struct Home {
    pub manifest: Manifest,
    pub sha256: String,
    files: Vec<(String, String)>,
}

impl Home {
    /// Loads and validates the shipped home for `harness`.
    pub fn shipped(harness: &str) -> Result<Self> {
        let (_, text, files) = HOMES
            .iter()
            .find(|(name, _, _)| *name == harness)
            .ok_or(format!("no home for harness {harness}"))?;
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|err| format!("{harness} manifest: {err}"))?;
        let home = Self {
            manifest,
            sha256: crate::sha256_hex(text.as_bytes()),
            files: files
                .iter()
                .map(|(name, body)| ((*name).to_owned(), (*body).to_owned()))
                .collect(),
        };
        home.validate()?;
        Ok(home)
    }

    fn source(&self, name: &str) -> Result<&str> {
        self.files
            .iter()
            .find(|(file, _)| file == name)
            .map(|(_, body)| body.as_str())
            .ok_or(format!(
                "{} home has no source file {name}",
                self.manifest.harness
            ))
    }

    /// The text of source file `name`.
    pub fn source_text(&self, name: &str) -> Result<String> {
        self.source(name).map(str::to_owned)
    }

    /// Replaces source `name` with `body` (probe control variants only),
    /// updating its entries' hashes and the home's digest, and revalidates.
    pub fn replace_source(&mut self, name: &str, body: String) -> Result {
        let digest = crate::sha256_hex(body.as_bytes());
        let slot = self
            .files
            .iter_mut()
            .find(|(file, _)| file == name)
            .ok_or(format!(
                "{} home has no source file {name}",
                self.manifest.harness
            ))?;
        slot.1 = body;
        for entry in &mut self.manifest.entries {
            if entry.source.as_deref() == Some(name) {
                entry.sha256 = Some(digest.clone());
            }
        }
        self.sha256 = crate::sha256_hex(format!("{} {name} {digest}", self.sha256).as_bytes());
        self.validate()
    }

    fn validate(&self) -> Result {
        let manifest = &self.manifest;
        let harness = &manifest.harness;
        if manifest.schema != SCHEMA {
            return Err(format!(
                "{harness} manifest schema is {}, not {SCHEMA}",
                manifest.schema
            ));
        }
        let mut seen = BTreeSet::new();
        for entry in &manifest.entries {
            let path = &entry.path;
            check_path(path)?;
            if !seen.insert(path.clone()) {
                return Err(format!("{harness} manifest lists {path} twice"));
            }
            let writable = entry.mode == "rw";
            if entry.mode != "rw" && entry.mode != "ro" {
                return Err(format!("{path}: mode must be ro or rw"));
            }
            if writable != (entry.kind == Kind::State) {
                return Err(format!(
                    "{path}: only state entries are rw, and state entries are rw"
                ));
            }
            if entry.is_dir() {
                if entry.source.is_some() || entry.sha256.is_some() {
                    return Err(format!("{path}: a directory has no source"));
                }
                continue;
            }
            let (Some(source), Some(sha256)) = (&entry.source, &entry.sha256) else {
                return Err(format!("{path}: a file entry needs a source and a sha256"));
            };
            let body = self.source(source)?;
            let actual = crate::sha256_hex(body.as_bytes());
            if &actual != sha256 {
                return Err(format!(
                    "{path}: source {source} drifted (SHA-256 {actual}, manifest {sha256})"
                ));
            }
            if entry.kind == Kind::State {
                check_state(path, body)?;
            }
        }
        check_path(&manifest.auth)?;
        for list in [
            &manifest.managed,
            &manifest.skills,
            &manifest.plugins,
            &manifest.mcp,
            &manifest.instructions,
        ] {
            for path in list {
                if !seen.contains(path) {
                    return Err(format!(
                        "{harness} manifest names {path} outside its entries"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Writes the home under `root` (`<run>/home`), which must not exist,
    /// and checks the result holds exactly the manifest's paths.
    pub fn build(&self, root: &Path) -> Result {
        if std::fs::symlink_metadata(root).is_ok() {
            return Err(format!(
                "{} already exists; every run needs a fresh home",
                root.display()
            ));
        }
        std::fs::create_dir_all(root).map_err(|err| format!("{}: {err}", root.display()))?;
        for entry in self.sorted() {
            let host = host_path(root, &entry.path);
            if entry.is_dir() {
                std::fs::create_dir_all(&host)
                    .map_err(|err| format!("{}: {err}", host.display()))?;
            } else {
                if let Some(parent) = host.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|err| format!("{}: {err}", parent.display()))?;
                }
                let body = self.source(entry.source.as_deref().unwrap_or_default())?;
                std::fs::write(&host, body).map_err(|err| format!("{}: {err}", host.display()))?;
            }
        }
        let auth = host_path(root, &self.manifest.auth);
        if let Some(parent) = auth.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        std::fs::write(&auth, "").map_err(|err| format!("{}: {err}", auth.display()))?;
        self.verify(root)
    }

    /// Checks `root` holds exactly the manifest's paths (and their parents)
    /// plus the empty auth mount point, by `lstat`.
    pub fn verify(&self, root: &Path) -> Result {
        let mut expected: BTreeSet<(PathBuf, bool)> = BTreeSet::new();
        let add_parents = |path: &Path, expected: &mut BTreeSet<(PathBuf, bool)>| {
            for ancestor in path.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                expected.insert((ancestor.to_path_buf(), true));
            }
        };
        for entry in &self.manifest.entries {
            let relative = PathBuf::from(entry.path.trim_start_matches('/'));
            add_parents(&relative, &mut expected);
            expected.insert((relative, entry.is_dir()));
        }
        let auth = PathBuf::from(self.manifest.auth.trim_start_matches('/'));
        add_parents(&auth, &mut expected);
        expected.insert((auth, false));
        let mut found = BTreeSet::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            for item in std::fs::read_dir(&directory)
                .map_err(|err| format!("{}: {err}", directory.display()))?
            {
                let path = item.map_err(|err| err.to_string())?.path();
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|err| format!("{}: {err}", path.display()))?;
                let relative = path
                    .strip_prefix(root)
                    .map_err(|err| err.to_string())?
                    .to_path_buf();
                if metadata.is_dir() {
                    stack.push(path);
                    found.insert((relative, true));
                } else if metadata.is_file() {
                    found.insert((relative, false));
                } else {
                    return Err(format!(
                        "{}: the home may hold only files and directories",
                        relative.display()
                    ));
                }
            }
        }
        if found != expected {
            let extra: Vec<_> = found
                .difference(&expected)
                .map(|(path, _)| path.display().to_string())
                .collect();
            let missing: Vec<_> = expected
                .difference(&found)
                .map(|(path, _)| path.display().to_string())
                .collect();
            return Err(format!(
                "the home differs from its manifest: extra {extra:?}, missing {missing:?}"
            ));
        }
        Ok(())
    }

    fn sorted(&self) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> = self.manifest.entries.iter().collect();
        entries.sort_by_key(|entry| (entry.path.matches('/').count(), entry.path.clone()));
        entries
    }

    /// bwrap arguments mounting the home built under `root`, the credential
    /// `auth` read-only, then remounting `/home/bench` read-only.
    pub fn mounts(&self, root: &Path, auth: &Path) -> Vec<String> {
        let mut args: Vec<String> = vec!["--tmpfs".into(), crate::HOME.into()];
        let entries = self.sorted();
        let state_dirs: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.kind == Kind::State && entry.is_dir())
            .map(|entry| entry.path.as_str())
            .collect();
        let inside_state = |path: &str| {
            state_dirs
                .iter()
                .any(|dir| path != *dir && Path::new(path).starts_with(dir))
        };
        for entry in &entries {
            let host = host_path(root, &entry.path).display().to_string();
            if entry.kind == Kind::State {
                if !inside_state(&entry.path) {
                    args.extend(["--bind".into(), host, entry.path.clone()]);
                }
            } else if entry.is_dir() {
                args.extend(["--dir".into(), entry.path.clone()]);
            } else {
                args.extend(["--ro-bind".into(), host, entry.path.clone()]);
            }
        }
        args.extend([
            "--ro-bind".into(),
            auth.display().to_string(),
            self.manifest.auth.clone(),
            "--remount-ro".into(),
            crate::HOME.into(),
        ]);
        args
    }

    /// Paths the guard checks for read-only mounts.
    pub fn read_only(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .manifest
            .entries
            .iter()
            .filter(|entry| entry.mode == "ro")
            .map(|entry| entry.path.clone())
            .collect();
        paths.push(crate::HOME.into());
        paths.push(self.manifest.auth.clone());
        paths
    }

    /// Read-only policy files and their SHA-256.
    pub fn policy(&self) -> Vec<(String, String)> {
        self.manifest
            .entries
            .iter()
            .filter(|entry| entry.mode == "ro" && !entry.is_dir())
            .filter_map(|entry| Some((entry.path.clone(), entry.sha256.clone()?)))
            .collect()
    }
}

/// `<run>/home` plus the jail path without its leading slash.
pub fn host_path(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

fn check_path(path: &str) -> Result {
    let plain = Path::new(path)
        .components()
        .skip(1)
        .all(|component| matches!(component, std::path::Component::Normal(_)));
    let allowed = ["/home/bench/", "/etc/claude-code/", "/etc/codex/"]
        .iter()
        .any(|root| path.starts_with(root));
    if !path.starts_with('/') || !plain || !allowed || path.ends_with('/') {
        return Err(format!(
            "manifest path {path} must be a plain absolute path under /home/bench, /etc/claude-code or /etc/codex"
        ));
    }
    Ok(())
}

/// Refuses a state file holding a policy key at its top level.
fn check_state(path: &str, body: &str) -> Result {
    let keys: Vec<String> = if path.ends_with(".json") {
        let value: serde_json::Value =
            serde_json::from_str(body).map_err(|err| format!("{path}: {err}"))?;
        value
            .as_object()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    } else if path.ends_with(".toml") {
        let value: toml::Table = toml::from_str(body).map_err(|err| format!("{path}: {err}"))?;
        value.keys().cloned().collect()
    } else {
        Vec::new()
    };
    match keys.iter().find(|key| POLICY_KEYS.contains(&key.as_str())) {
        Some(key) => Err(format!(
            "{path}: a state file may not hold the policy key {key}"
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_homes_validate() {
        for (harness, text, _) in HOMES {
            let home = Home::shipped(harness).unwrap();
            assert_eq!(home.sha256, crate::sha256_hex(text.as_bytes()));
            assert!(home.manifest.skills.is_empty() && home.manifest.plugins.is_empty());
            assert!(home.manifest.mcp.is_empty() && home.manifest.instructions.is_empty());
        }
    }

    #[test]
    fn drift_and_policy_in_state_are_refused() {
        let mut home = Home::shipped("claude").unwrap();
        home.files[0].1 = "{}".into();
        assert!(home.validate().unwrap_err().contains("drifted"));
        assert!(check_state("/home/bench/.claude/settings.json", r#"{"hooks":{}}"#).is_err());
        assert!(
            check_state(
                "/home/bench/.codex/state.toml",
                "[mcp_servers.x]\ncommand = \"y\"\n"
            )
            .is_err()
        );
        assert!(check_state("/home/bench/.claude/.claude.json", r#"{"projects":{}}"#).is_ok());
    }

    #[test]
    fn a_fresh_home_matches_and_an_existing_one_is_refused() {
        for (harness, _, _) in HOMES {
            let home = Home::shipped(harness).unwrap();
            let root = crate::test_dir("home").join("home");
            home.build(&root).unwrap();
            assert!(home.build(&root).unwrap_err().contains("fresh home"));
            std::fs::write(root.join("home/bench/extra"), "x").unwrap();
            assert!(home.verify(&root).unwrap_err().contains("extra"));
        }
    }

    #[test]
    fn mounts_bind_state_rw_policy_ro_and_remount_home() {
        let home = Home::shipped("claude").unwrap();
        let args = home
            .mounts(Path::new("/run/x/home"), Path::new("/host/creds.json"))
            .join(" ");
        assert!(args.starts_with("--tmpfs /home/bench"));
        assert!(args.contains("--ro-bind /run/x/home/etc/claude-code/managed-settings.json /etc/claude-code/managed-settings.json"));
        assert!(args.contains("--bind /run/x/home/home/bench/.claude /home/bench/.claude"));
        assert!(!args.contains("/home/bench/.claude/settings.json"));
        assert!(args.ends_with("--ro-bind /host/creds.json /home/bench/.claude/.credentials.json --remount-ro /home/bench"));
    }
}
