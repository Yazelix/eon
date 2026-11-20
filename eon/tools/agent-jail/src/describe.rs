//! `agent-jail describe <profile>`: the jail's part of a bench config
//! record (eon-2r02l.1, M8). It refuses a probe-only profile unless asked
//! with `--probe-only`, which marks the record `bench_ready: false`.

use serde_json::{Value, json};

use crate::Result;
use crate::manifest::Home;

/// The SHA-256 over sorted `name sha256` lines of everything that shapes a
/// jail: profile, allowlist, home manifest, host pins and the tool version.
pub fn jail_profile_sha256(name: &str) -> Result<String> {
    let loaded = crate::profile::load(name)?;
    let home = Home::shipped(&loaded.profile.harness)?;
    let mut lines = [
        format!("allowlist {}", loaded.allowlist.sha256()),
        format!("agent-jail {}", env!("CARGO_PKG_VERSION")),
        format!(
            "host-tools {}",
            crate::sha256_hex(crate::pins::HOST_TOOLS.as_bytes())
        ),
        format!("manifest {}", home.sha256),
        format!("profile {}", loaded.sha256),
    ];
    lines.sort();
    Ok(crate::sha256_hex(lines.join("\n").as_bytes()))
}

pub fn describe(name: &str, probe_only: bool) -> Result<Value> {
    let loaded = crate::profile::load(name)?;
    if loaded.probe_only() && !probe_only {
        return Err(format!(
            "profile {name} is probe-only in M2 (the host store is visible); describe it with --probe-only"
        ));
    }
    let home = Home::shipped(&loaded.profile.harness)?;
    let profile = &loaded.profile;
    Ok(json!({
        "bench_ready": !loaded.probe_only(),
        "permissions": {
            "mode": profile.mode,
            "sandbox": "agent-jail-inner/1",
            "web_tools": {
                "denied": profile.web_tools_denied,
                "mechanism": profile.web_tools_mechanism,
            },
            "proxy_allowlist_sha256": loaded.allowlist.sha256(),
        },
        "harness_home": { "manifest_sha256": home.sha256 },
        "tools": {
            "jail_profile_sha256": jail_profile_sha256(name)?,
            "dcg": {
                "version": crate::pins::pinned_version("dcg")?,
                "config_sha256": crate::sha256_hex(include_str!("../dcg-config.toml").as_bytes()),
            },
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_only_profiles_need_the_flag() {
        for name in ["claude", "codex"] {
            assert!(describe(name, false).unwrap_err().contains("--probe-only"));
            let record = describe(name, true).unwrap();
            assert_eq!(record["bench_ready"], false);
            assert_eq!(record["permissions"]["sandbox"], "agent-jail-inner/1");
            for pointer in [
                "/permissions/proxy_allowlist_sha256",
                "/harness_home/manifest_sha256",
                "/tools/jail_profile_sha256",
                "/tools/dcg/config_sha256",
            ] {
                assert_eq!(
                    record
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .map(str::len),
                    Some(64),
                    "{pointer}"
                );
            }
            assert_eq!(record["tools"]["dcg"]["version"], "0.16.0");
        }
    }

    #[test]
    fn profiles_have_distinct_digests() {
        assert_ne!(
            jail_profile_sha256("claude").unwrap(),
            jail_profile_sha256("codex").unwrap()
        );
    }
}
