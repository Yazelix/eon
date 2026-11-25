//! The rules that decide a changed path's review tier and Darwin check level.
//!
//! Every rule lives here, in decision order, and `review-tier rules` prints
//! it. A change that lowers any tier (moves a pattern down, adds to the Medium
//! or Low list, removes a High pattern, mode rule or risky token) needs
//! Lucca's recorded approval, like a contract change; REVIEW.md makes it a
//! blocker without that approval, and this crate's own path is High.
//! Lowering any Darwin level also needs Lucca's recorded approval.
use crate::Result;
use crate::classify::Change;
use serde::Deserialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Low,
    Medium,
    High,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A High rule over the file modes and status letter, checked before paths.
pub struct ModeRule {
    pub description: &'static str,
    pub reason: &'static str,
    pub matches: fn(&Change) -> bool,
}

pub const MODE_RULES: &[ModeRule] = &[
    ModeRule {
        description: "old and new modes differ, both non-zero",
        reason: "mode change",
        matches: |change| {
            change.old_mode != change.new_mode
                && change.old_mode != ABSENT
                && change.new_mode != ABSENT
        },
    },
    ModeRule {
        description: "either mode 120000",
        reason: "symlink",
        matches: |change| change.old_mode == SYMLINK || change.new_mode == SYMLINK,
    },
    ModeRule {
        description: "either mode 160000",
        reason: "submodule",
        matches: |change| change.old_mode == SUBMODULE || change.new_mode == SUBMODULE,
    },
    ModeRule {
        description: "status T",
        reason: "type change",
        matches: |change| change.status == 'T',
    },
    ModeRule {
        description: "status U",
        reason: "unmerged",
        matches: |change| change.status == 'U',
    },
];

pub const ABSENT: &str = "000000";
pub const REGULAR: &str = "100644";
const SYMLINK: &str = "120000";
const SUBMODULE: &str = "160000";

/// A path rule: the first rule with a matching pattern decides the path.
pub struct Rule {
    pub tier: Tier,
    pub reason: &'static str,
    pub patterns: &'static [&'static str],
}

const fn rule(tier: Tier, reason: &'static str, patterns: &'static [&'static str]) -> Rule {
    Rule {
        tier,
        reason,
        patterns,
    }
}

pub const RULES: &[Rule] = &[
    rule(Tier::High, "contract", &["**/CONTRACTS.md"]),
    rule(
        Tier::High,
        "wire and protocol",
        &[
            "runtime/crates/eon-workspace-protocol/**",
            "orbit/crates/protocol/**",
            "orbit/src/boundary.rs",
            "orbit/src/boundary.schema",
            "orbit/src/socket.rs",
            "orbit/src/management.rs",
            "venus/src/transport.rs",
            "runtime/crates/eon-runtime/src/control.rs",
        ],
    ),
    rule(
        Tier::High,
        "Bend policy and proofs",
        &["**/*.bend", "orbit/proof/**"],
    ),
    rule(
        Tier::High,
        "lifecycle",
        &[
            "runtime/crates/eon-runtime/src/supervisor.rs",
            "runtime/crates/eon-runtime/src/running.rs",
            "runtime/crates/eon-runtime/src/generation.rs",
            "runtime/crates/eon-runtime/src/generation/**",
            "runtime/crates/eon-runtime/src/session_registry.rs",
            "runtime/crates/eon-runtime/src/session_registry/**",
            "orbit/src/host.rs",
            "orbit/src/pty.rs",
            "eon/crates/eon/src/update.rs",
            "eon/crates/eon/src/update/**",
            "eon/crates/eon-agent/src/cleanup.rs",
            "eon/crates/eon-agent/src/cleanup/**",
        ],
    ),
    rule(
        Tier::High,
        "trust boundary",
        &[
            "runtime/crates/eon-runtime/src/private_file.rs",
            "runtime/crates/eon-runtime/src/launch_environment.rs",
            "eon/crates/eon-agent/src/launch.rs",
            "installer/**",
            "launcher/**",
            "relink/**",
        ],
    ),
    rule(
        Tier::High,
        "dependencies and composition",
        &[
            "**/Cargo.toml",
            "Cargo.lock",
            "flake.nix",
            "flake.lock",
            "nix/**",
            ".github/**",
            "components/**",
            "**/.gitattributes",
        ],
    ),
    rule(
        Tier::High,
        "gate or validator tool",
        &["eon/tools/**", "orbit/tools/**"],
    ),
    rule(
        Tier::High,
        "agent policy",
        &[
            "**/AGENTS.md",
            ".agent-protocols*",
            "docs/AGENT-*.md",
            "REVIEW.md",
        ],
    ),
    rule(
        Tier::Medium,
        "ordinary code",
        &[
            "venus/src/**",
            "orbit/src/**",
            "runtime/crates/eon-runtime/src/**",
            "eon/crates/*/src/**",
            "eon/skills/**",
            "demo/**",
            "eon/crates/*/*.example.toml",
        ],
    ),
    rule(Tier::Medium, "test", &["**/tests/**", "**/testdata/**"]),
    rule(Tier::Medium, "asset", &["venus/assets/**", "assets/**"]),
    rule(
        Tier::Low,
        "planning",
        &[
            ".beads/issues.jsonl",
            ".beads/config.yaml",
            ".beads/metadata.json",
        ],
    ),
    rule(Tier::Low, "docs", &["**/*.md"]),
];

/// A Medium `.rs` path whose added or removed lines contain one of these
/// (plain substring) is High.
pub const RISKY_TOKENS: &[&str] = &[
    "unsafe",
    "pre_exec",
    "kill(",
    "killpg",
    "SIGKILL",
    "SIGTERM",
    "set_permissions",
    "from_mode",
    "UnixListener::bind",
    "UnixDatagram::bind",
    "TcpListener::bind",
    "UdpSocket::bind",
];

pub const READ_BY_CODE: &str = "Markdown read by code";
pub const LOW_MODE: &str = "mode";
pub const UNLISTED: &str = "not on the Medium or Low lists";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DarwinLevel {
    None,
    Ci,
    Proof,
}

impl DarwinLevel {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ci => "ci",
            Self::Proof => "proof",
        }
    }
}

struct DarwinRule {
    level: DarwinLevel,
    reason: &'static str,
    patterns: &'static [&'static str],
    unreferenced_markdown: bool,
}

/// Explicit paths take precedence over the Markdown rule and workspace default.
const DARWIN_RULES: &[DarwinRule] = &[
    DarwinRule {
        level: DarwinLevel::Proof,
        reason: "native macOS surface",
        patterns: &[
            "venus/src/application.rs",
            "venus/src/application/**",
            "venus/src/launch.rs",
            "venus/src/accessibility.rs",
            "venus/src/main.rs",
            "venus/Cargo.toml",
            "launcher/**",
            "**/macos/**",
            "**/macos.rs",
            "**/darwin/**",
            "**/darwin.rs",
            "**/platform.rs",
            "nix/macos-app.nix",
            "eon/tools/mac-proof/**",
            "orbit/terminfo/eon-darwin.terminfo",
            ".github/workflows/mac-proof.yml",
        ],
        unreferenced_markdown: false,
    },
    DarwinRule {
        level: DarwinLevel::Proof,
        reason: "macOS-sensitive boundary",
        patterns: &[
            "orbit/src/socket.rs",
            "venus/src/transport.rs",
            "runtime/crates/eon-runtime/src/control.rs",
            "runtime/crates/eon-runtime/src/editor.rs",
            "runtime/crates/eon-runtime/src/supervisor/update/handoff.rs",
            "runtime/crates/eon-runtime/src/supervisor/update/handoff/**",
        ],
        unreferenced_markdown: false,
    },
    DarwinRule {
        level: DarwinLevel::None,
        reason: "planning",
        patterns: &[".beads/**"],
        unreferenced_markdown: false,
    },
    DarwinRule {
        level: DarwinLevel::None,
        reason: "Markdown not read by code",
        patterns: &["**/*.md"],
        unreferenced_markdown: true,
    },
    DarwinRule {
        level: DarwinLevel::Ci,
        reason: READ_BY_CODE,
        patterns: &["**/*.md"],
        unreferenced_markdown: false,
    },
    DarwinRule {
        level: DarwinLevel::Ci,
        reason: "workspace change",
        patterns: &["**"],
        unreferenced_markdown: false,
    },
];

const DARWIN_BUILD: &[&str] = &[
    "nix/**",
    "flake.nix",
    "flake.lock",
    "Cargo.lock",
    "**/Cargo.toml",
    "components/**",
    "launcher/**",
    ".github/workflows/darwin-build.yml",
];

pub fn darwin_rule(
    path: &str,
    mut read_by_code: impl FnMut() -> Result<bool>,
) -> Result<(DarwinLevel, &'static str)> {
    for rule in DARWIN_RULES {
        if rule.patterns.iter().any(|pattern| glob(pattern, path))
            && (!rule.unreferenced_markdown || !read_by_code()?)
        {
            return Ok((rule.level, rule.reason));
        }
    }
    Err(format!("no Darwin rule matches {path:?}").into())
}

pub fn darwin_build(path: &str) -> bool {
    DARWIN_BUILD.iter().any(|pattern| glob(pattern, path))
}

const DARWIN_SENSITIVE: &[&str] = &[
    "target_os=\"macos\"",
    "target_vendor=\"apple\"",
    "not(target_os=\"linux\")",
    "aarch64-darwin",
    "/private/tmp",
    "canonicalize",
    "read_link",
    "symlink",
    "sockaddr",
    "socket",
    "unixstream",
    "unixlistener",
    "unixdatagram",
    "o_nonblock",
    "set_nonblocking(",
    "accept4",
    "sock_cloexec",
    ".sock",
    "runtime_dir",
    "temp_dir",
    "tmpdir",
    "tempdir",
    "tempfile",
    "tmpfile",
    "/dev/shm",
    "shm_open",
    "shm_unlink",
    "shmget",
    "shmat",
    "memfd",
    "mknodat",
    "platform_label",
    "platform_key",
    "key_label",
    "proc_pidinfo",
    "/proc/",
    "process::",
    "command::new",
    "posix_spawn",
    "fork(",
    "execve(",
    "waitpid(",
    "kill(",
];

pub fn darwin_source(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension,
            "rs" | "nix" | "sh" | "c" | "h" | "m" | "mm" | "swift" | "toml" | "yml" | "yaml"
        )
    }) && !path.starts_with(".beads/")
        && !path.starts_with("eon/tools/review-tier/evals/")
}

pub fn darwin_sensitive(line: &str) -> bool {
    let line: String = line
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    DARWIN_SENSITIVE.iter().any(|token| line.contains(token))
}

/// The first path rule matching `path`, if any.
pub fn path_rule(path: &str) -> Option<&'static Rule> {
    RULES
        .iter()
        .find(|rule| rule.patterns.iter().any(|pattern| glob(pattern, path)))
}

/// The whole table, in decision order, as `review-tier rules` prints it.
pub fn render() -> String {
    let mut rows: Vec<(Tier, String, String)> = Vec::new();
    for mode in MODE_RULES {
        rows.push((
            Tier::High,
            format!("mode: {}", mode.description),
            mode.reason.into(),
        ));
    }
    for (index, rule) in RULES.iter().enumerate() {
        for pattern in rule.patterns {
            rows.push((rule.tier, format!("path: {pattern}"), rule.reason.into()));
        }
        let last_medium = rule.tier == Tier::Medium
            && RULES
                .get(index + 1)
                .is_none_or(|next| next.tier != Tier::Medium);
        if last_medium {
            for token in RISKY_TOKENS {
                rows.push((
                    Tier::High,
                    format!("token in a medium .rs line: {token}"),
                    format!("risky token: {token}"),
                ));
            }
        }
    }
    rows.push((
        Tier::High,
        "low path with a mode other than 100644 or 000000".into(),
        LOW_MODE.into(),
    ));
    rows.push((
        Tier::Medium,
        "low .md path named in a .rs or .nix file".into(),
        READ_BY_CODE.into(),
    ));
    rows.push((Tier::High, "anything else".into(), UNLISTED.into()));
    let width = rows.iter().map(|row| row.1.len()).max().unwrap_or(0);
    let mut out = String::from(
        "# First match wins. Lowering any tier needs Lucca's recorded approval, like a contract.\n",
    );
    for (tier, rule, reason) in rows {
        out.push_str(&format!("{:<6}  {rule:<width$}  {reason}\n", tier.name()));
    }
    out
}

pub fn render_darwin() -> String {
    let mut out = String::from(
        "# Darwin levels. First match per path, then highest level across the change.\n\
         # Lowering any Darwin level needs Lucca's recorded approval.\n",
    );
    for rule in DARWIN_RULES {
        let condition = if rule.unreferenced_markdown {
            " without a name match in .rs or .nix"
        } else {
            ""
        };
        for pattern in rule.patterns {
            out.push_str(&format!(
                "{:<5}  path: {pattern}{condition}  {}\n",
                rule.level.name(),
                rule.reason,
            ));
        }
    }
    out.push_str("# ci and proof run workspace-tests. Add build if any changed path matches:\n");
    for pattern in DARWIN_BUILD {
        out.push_str(&format!("build  path: {pattern}\n"));
    }
    out.push_str(
        "# A changed Rust file with a macOS or non-Linux cfg in either tree requires proof.\n",
    );
    out.push_str("# A source file type change requires proof.\n");
    out.push_str("# Raise ci to proof for these tokens in added or removed source lines:\n");
    for token in DARWIN_SENSITIVE {
        out.push_str(&format!("proof  token: {token}\n"));
    }
    out
}

/// Whole-path glob: `*` matches within one segment, a leading or inner `**/`
/// matches zero or more whole segments, and a trailing `/**` matches one or
/// more segments.
pub fn glob(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", [])) => !path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((first, rest)) => match path.split_first() {
            Some((segment, path)) => segment_glob(first, segment) && segments(rest, path),
            None => false,
        },
    }
}

fn segment_glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, rest)) => {
            let Some(text) = text.strip_prefix(prefix) else {
                return false;
            };
            text.char_indices()
                .map(|(index, _)| index)
                .chain([text.len()])
                .any(|index| segment_glob(rest, &text[index..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_whole_paths() {
        assert!(glob("flake.nix", "flake.nix"));
        assert!(!glob("flake.nix", "nix/flake.nix"));
        assert!(glob("**/Cargo.toml", "Cargo.toml"));
        assert!(glob("**/Cargo.toml", "eon/crates/eon/Cargo.toml"));
        assert!(!glob("**/Cargo.toml", "eon/Cargo.toml.orig"));
        assert!(glob("nix/**", "nix/a.nix"));
        assert!(glob("nix/**", "nix/a/b.nix"));
        assert!(!glob("nix/**", "nix"));
        assert!(!glob("nix/**", "nixos/a.nix"));
        assert!(glob("**/tests/**", "tests/a.rs"));
        assert!(glob("**/tests/**", "venus/tests/a/b.rs"));
        assert!(!glob("**/tests/**", "venus/tests"));
        assert!(glob("eon/crates/*/src/**", "eon/crates/eon/src/main.rs"));
        assert!(!glob("eon/crates/*/src/**", "eon/crates/a/b/src/main.rs"));
        assert!(glob(".agent-protocols*", ".agent-protocols.json"));
        assert!(glob(".agent-protocols*", ".agent-protocols"));
        assert!(!glob(".agent-protocols*", "x/.agent-protocols.json"));
        assert!(glob("docs/AGENT-*.md", "docs/AGENT-RULES.md"));
        assert!(!glob("docs/AGENT-*.md", "docs/AGENT-RULES.txt"));
        assert!(glob("**/*.md", "README.md"));
        assert!(glob("**/*.md", "docs/a/b.md"));
        assert!(!glob("**/*.md", "docs/a.mdx"));
        assert!(glob("a/**/b", "a/b"));
        assert!(glob("a/**/b", "a/x/y/b"));
        assert!(!glob("a/**/b", "a/x/c"));
        assert!(glob("*a*b*", "xxaybz"));
        assert!(!glob("*a*b*", "xxbya"));
        assert!(glob("é*", "éa"));
    }

    /// The tier and Darwin sections of `review-tier rules` have separate
    /// goldens. Copy each section to its own file when updating the rules.
    #[test]
    fn the_rule_tables_match_their_goldens() {
        assert_eq!(render(), include_str!("rules.golden"));
        assert_eq!(render_darwin(), include_str!("darwin-rules.golden"));
    }

    #[test]
    fn every_pattern_sits_in_one_rule_with_a_reason() {
        for rule in RULES {
            assert!(!rule.reason.is_empty());
            assert!(!rule.patterns.is_empty());
        }
        let rendered = render();
        for rule in RULES {
            for pattern in rule.patterns {
                assert!(rendered.contains(&format!("path: {pattern} ")));
            }
        }
        for token in RISKY_TOKENS {
            assert!(rendered.contains(&format!("line: {token} ")));
        }
    }
}
