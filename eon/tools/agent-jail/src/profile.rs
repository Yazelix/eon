//! Harness profiles (`profiles/<harness>.toml`) and their argv templates.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::Result;
use crate::allowlist::Allowlist;

pub const PROFILES: &[(&str, &str)] = &[
    ("claude", include_str!("../profiles/claude.toml")),
    ("codex", include_str!("../profiles/codex.toml")),
];

pub const ALLOWLISTS: &[(&str, &str)] = &[
    ("claude", include_str!("../allowlists/claude.txt")),
    ("codex", include_str!("../allowlists/codex.txt")),
];

/// The template slots a run fills.
pub const SLOTS: &[&str] = &["prompt", "model", "effort"];

/// Flags no argv may carry, whatever the template says.
pub const FORBIDDEN_FLAGS: &[&str] = &[
    "--dangerously-skip-permissions",
    "--dangerously-bypass-approvals-and-sandbox",
    "--dangerously-bypass-hook-trust",
    "--allow-dangerously-skip-permissions",
    "--settings",
    "--add-dir",
    "--plugin-dir",
    "--search",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub harness: String,
    /// The `host-tools.toml` pin of the harness binary.
    pub binary: String,
    /// The executable in the jail; `pinned` means the pinned path itself.
    pub exe: String,
    /// Where the pinned binary's release directory (two levels up) is bound.
    #[serde(default)]
    pub release_mount: Option<String>,
    pub allowlist: String,
    pub store: String,
    pub argv: Vec<String>,
    pub web_tools_denied: Vec<String>,
    pub web_tools_mechanism: Vec<String>,
    pub mode: String,
    pub env: BTreeMap<String, String>,
}

/// A profile with its digest and allowlist.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub name: String,
    pub profile: Profile,
    pub sha256: String,
    pub allowlist: Allowlist,
}

pub fn load(name: &str) -> Result<Loaded> {
    let (_, text) = PROFILES
        .iter()
        .find(|(profile, _)| *profile == name)
        .ok_or(format!("no profile named {name}"))?;
    let profile: Profile = toml::from_str(text).map_err(|err| format!("profile {name}: {err}"))?;
    let (_, list) = ALLOWLISTS
        .iter()
        .find(|(list, _)| *list == profile.allowlist)
        .ok_or(format!(
            "profile {name} names no shipped allowlist {}",
            profile.allowlist
        ))?;
    if profile.store != "host-probe-only" {
        return Err(format!(
            "profile {name}: store {} is not built yet (M3)",
            profile.store
        ));
    }
    for flag in &profile.argv {
        if FORBIDDEN_FLAGS.contains(&flag.as_str()) {
            return Err(format!("profile {name}: the template may not carry {flag}"));
        }
    }
    Ok(Loaded {
        name: name.to_owned(),
        sha256: crate::sha256_hex(text.as_bytes()),
        allowlist: Allowlist::parse(list)?,
        profile,
    })
}

impl Loaded {
    pub fn probe_only(&self) -> bool {
        self.profile.store == "host-probe-only"
    }

    /// The argv from the template with `slots` filled in.
    pub fn fill(&self, slots: &BTreeMap<String, String>) -> Result<Vec<String>> {
        let names: Vec<&str> = slots.keys().map(String::as_str).collect();
        if names != {
            let mut sorted = SLOTS.to_vec();
            sorted.sort_unstable();
            sorted
        } {
            return Err(format!(
                "argv slots must be exactly {SLOTS:?}, got {names:?}"
            ));
        }
        for (name, value) in slots {
            check_slot(name, value)?;
        }
        Ok(self
            .profile
            .argv
            .iter()
            .map(|part| {
                slots.iter().fold(part.clone(), |text, (name, value)| {
                    text.replace(&format!("{{{name}}}"), value)
                })
            })
            .collect())
    }

    /// Guard condition 5: `argv` is exactly the template filled with
    /// `slots`, and carries no forbidden spelling.
    pub fn check_argv(&self, slots: &BTreeMap<String, String>, argv: &[String]) -> Result {
        let expected = self.fill(slots)?;
        if argv != expected.as_slice() {
            return Err("argv differs from the profile template".into());
        }
        let prompt_index = self.profile.argv.iter().position(|part| part == "{prompt}");
        for (index, part) in argv.iter().enumerate() {
            if Some(index) == prompt_index {
                continue;
            }
            if FORBIDDEN_FLAGS
                .iter()
                .any(|flag| part == flag || part.starts_with(&format!("{flag}=")))
            {
                return Err(format!("argv carries the forbidden flag {part}"));
            }
            if part.starts_with("--") && part.contains('=') {
                return Err(format!("argv carries a --flag=value spelling: {part}"));
            }
        }
        Ok(())
    }
}

/// Slot values: a prompt is any text that cannot read as a flag; model and
/// effort are short plain names.
pub fn check_slot(name: &str, value: &str) -> Result {
    let ok = match name {
        "prompt" => {
            !value.is_empty()
                && !value.starts_with('-')
                && !value.contains('\0')
                && value.len() <= 64 * 1024
        }
        "model" | "effort" => {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'[' | b']')
                })
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "slot {name} has a value the template does not accept"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(prompt: &str) -> BTreeMap<String, String> {
        [
            ("prompt", prompt),
            ("model", "claude-opus-5-5"),
            ("effort", "high"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .into()
    }

    #[test]
    fn shipped_profiles_load() {
        for (name, _) in PROFILES {
            let loaded = load(name).unwrap();
            assert!(loaded.probe_only());
            let argv = loaded.fill(&slots("Reply with exactly: jail-ok")).unwrap();
            loaded
                .check_argv(&slots("Reply with exactly: jail-ok"), &argv)
                .unwrap();
            assert!(argv.iter().all(|part| !part.contains('{')), "{argv:?}");
        }
    }

    #[test]
    fn bad_slots_and_argv_are_refused() {
        let loaded = load("claude").unwrap();
        assert!(
            loaded
                .fill(&slots("--dangerously-skip-permissions"))
                .is_err()
        );
        let mut extra = slots("hi");
        extra.insert("other".into(), "x".into());
        assert!(loaded.fill(&extra).is_err());
        let mut bad_model = slots("hi");
        bad_model.insert("model".into(), "x --settings y".into());
        assert!(loaded.fill(&bad_model).is_err());

        let good = loaded.fill(&slots("hi")).unwrap();
        let mut appended = good.clone();
        appended.push("--dangerously-skip-permissions".into());
        assert!(loaded.check_argv(&slots("hi"), &appended).is_err());
        let mut equals = good.clone();
        equals[2] = "--model=x".into();
        assert!(loaded.check_argv(&slots("hi"), &equals).is_err());
        let mut positional = good;
        positional.push("extra".into());
        assert!(loaded.check_argv(&slots("hi"), &positional).is_err());
        // A prompt that merely mentions a flag is text, not a flag.
        let mention = slots("explain --dangerously-skip-permissions");
        let argv = loaded.fill(&mention).unwrap();
        loaded.check_argv(&mention, &argv).unwrap();
    }
}
