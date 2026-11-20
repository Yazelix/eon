//! The jail spec: what `enter` tells init and the wrappers, written to
//! `/bench/jail/spec.json`, read-only in both layers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Result;

pub const SPEC_PATH: &str = "/bench/jail/spec.json";

/// Environment variables the inner layer never sees.
pub const PROXY_VARS: &[&str] = &[
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "https_proxy",
    "http_proxy",
    "all_proxy",
    "no_proxy",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// The profile's harness, argv from its template.
    Harness,
    /// The probe's stand-in harness: `agent-jail probe-harness <profile>`.
    StandIn,
    /// Trusted host work (the warm build, the scorer): a test template run
    /// through `/bin/sh` with no credential, proxy or bridge.
    Command,
}

/// `/bin/sh` runs a command template as positional arguments of this
/// constant script, so no template text is ever parsed by a shell.
pub const COMMAND_SCRIPT: [&str; 3] = ["-c", "exec \"$@\"", "agent-jail-command"];

/// What the jail's store is, for the probe's store checks.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSpec {
    pub store: crate::task_env::Store,
    /// Packages that must be in neither the store nor any PATH.
    pub exclude: Vec<String>,
    /// Mount roots under the host's /nix/store the jail may hold: the
    /// agent-jail binary when it is a store path and, for the host-test
    /// store, the single files bound out of it.
    pub allowed_roots: Vec<String>,
    /// The pinned nix the probe tries to fetch with.
    pub nix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookCheck {
    /// What the check proves, for the refusal message.
    pub name: String,
    /// The command exactly as the harness runs it, through `/bin/sh -c`.
    pub command: String,
    /// The hook input on stdin.
    pub input: String,
    /// True when the hook must deny.
    pub deny: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub profile: String,
    pub mode: Mode,
    /// The template's slot values (`prompt`, `model`, `effort`).
    pub slots: BTreeMap<String, String>,
    /// The harness executable in the jail, then its arguments.
    pub exe: String,
    pub argv: Vec<String>,
    /// Wrapper path in the jail to the real tool it stands for.
    pub tools: BTreeMap<String, String>,
    /// The credential's path in the jail.
    pub auth_path: String,
    /// Paths that must sit on read-only mounts (guard condition 7).
    pub read_only: Vec<String>,
    /// Policy files the guard checks by hash: path to SHA-256.
    pub policy: BTreeMap<String, String>,
    /// Hook checks run before the harness starts (guard condition 3).
    pub hooks: Vec<HookCheck>,
    /// Directories the inner layer covers with a fresh tmpfs.
    pub inner_masks: Vec<String>,
    /// The pinned dcg the dcg shim runs.
    pub dcg: String,
    /// The probe control variant, `none` for real runs.
    #[serde(default)]
    pub control: crate::control::Control,
    pub store: StoreSpec,
    /// The dev shell's PATH dirs, after `/bench/bin` on the inner PATH.
    pub inner_path: Vec<String>,
    /// Command mode's template, from env.json (empty otherwise).
    pub template: Vec<String>,
}

impl Spec {
    pub fn read() -> Result<Self> {
        Self::read_from(std::path::Path::new(SPEC_PATH))
    }

    pub fn read_from(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
    }
}
