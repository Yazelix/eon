//! Proxy allowlists: one lowercase DNS name per line, sorted and unique, with
//! a never-allow list enforced at load.

use crate::Result;

/// Hosts no allowlist may name: code hosting (the bench's answers live
/// there), the harnesses' token refresh and sign-in hosts, and connector
/// proxies. A name is refused when it equals an entry or ends with
/// `.<entry>`.
pub const NEVER_ALLOW: &[&str] = &[
    "github.com",
    "githubusercontent.com",
    "github.io",
    "githubassets.com",
    "ghcr.io",
    "platform.claude.com",
    "console.anthropic.com",
    "claude.ai",
    "claude.com",
    "auth.openai.com",
    "mcp-proxy.anthropic.com",
];

/// A parsed allowlist and the SHA-256 of its exact bytes.
#[derive(Clone, Debug)]
pub struct Allowlist {
    hosts: Vec<String>,
    sha256: String,
}

impl Allowlist {
    /// Parses `text`, refusing anything but the canonical form.
    pub fn parse(text: &str) -> Result<Self> {
        if !text.is_empty() && !text.ends_with('\n') {
            return Err("allowlist must end with a newline".into());
        }
        let mut hosts: Vec<String> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            if !is_dns_name(line) {
                return Err(format!(
                    "allowlist line {number} is not a lowercase DNS name"
                ));
            }
            if let Some(blocked) = never_allowed(line) {
                return Err(format!(
                    "allowlist line {number} names {line}, which the never-allow entry {blocked} refuses"
                ));
            }
            if let Some(last) = hosts.last()
                && last.as_str() >= line
            {
                return Err(format!(
                    "allowlist line {number} is out of order or repeated"
                ));
            }
            hosts.push(line.to_owned());
        }
        Ok(Self {
            hosts,
            sha256: crate::sha256_hex(text.as_bytes()),
        })
    }

    /// True when `host` matches one line exactly.
    pub fn allows(&self, host: &str) -> bool {
        self.hosts
            .binary_search_by(|line| line.as_str().cmp(host))
            .is_ok()
    }

    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// The never-allow entry that refuses `host`, if any.
pub fn never_allowed(host: &str) -> Option<&'static str> {
    NEVER_ALLOW.iter().copied().find(|blocked| {
        host == *blocked
            || host
                .strip_suffix(blocked)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// A lowercase DNS name with at least two labels and no IP literal.
pub fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    // The last label must hold a letter, so no dotted number passes.
    labels.iter().all(|label| label_ok(label))
        && labels
            .last()
            .is_some_and(|tld| tld.bytes().any(|byte| byte.is_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn digest(text: &str) -> String {
        crate::hex(&Sha256::digest(text.as_bytes()))
    }

    #[test]
    fn canonical_lists_parse() {
        let list = Allowlist::parse("api.anthropic.com\nchatgpt.com\n").unwrap();
        assert!(list.allows("api.anthropic.com"));
        assert!(!list.allows("API.anthropic.com"));
        assert!(!list.allows("anthropic.com"));
        assert_eq!(list.sha256(), digest("api.anthropic.com\nchatgpt.com\n"));
    }

    #[test]
    fn non_canonical_lists_are_refused() {
        for text in [
            "chatgpt.com",
            "chatgpt.com\napi.anthropic.com\n",
            "chatgpt.com\nchatgpt.com\n",
            "ChatGPT.com\n",
            "\n",
            "# comment\n",
            "*.anthropic.com\n",
            "1.1.1.1\n",
            "localhost\n",
            "api.anthropic.com \n",
        ] {
            assert!(Allowlist::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn never_allow_hosts_are_refused() {
        for host in [
            "github.com",
            "api.github.com",
            "raw.githubusercontent.com",
            "platform.claude.com",
            "claude.ai",
            "auth.openai.com",
            "mcp-proxy.anthropic.com",
        ] {
            let err = Allowlist::parse(&format!("{host}\n")).unwrap_err();
            assert!(err.contains("never-allow"), "{host}: {err}");
        }
        assert!(never_allowed("notgithub.com").is_none());
    }

    #[test]
    fn shipped_lists_are_canonical() {
        for (name, text) in crate::profile::ALLOWLISTS {
            Allowlist::parse(text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}
