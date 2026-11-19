//! The close record's data, its rendered comment text, and the hashes it
//! carries. Rendering reads nothing but the record, so a landed record's
//! text never depends on today's policy.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "eon-close-record/1";
pub const HEADER: &str = "Close record eon-close-record/1";
/// The only close reason a landed bead may carry; the record says the rest.
pub const REASON: &str = "Landed; see the close record.";
/// The rules every session worked under, hashed into its configuration.
pub const RULES_FILES: [&str; 4] = [
    "AGENTS.md",
    "REVIEW.md",
    "docs/AGENT-PROTOCOLS.md",
    "docs/AGENT-RULES.md",
];
const CONFIG_SCHEME: &str = "eon-config/provisional-1";
const CONFIG_PREFIX: &str = "cfg-p1:";
pub const SOURCE_PREFIX: &str = "src-1:";

/// A close record. The lead's input is the same shape without `schema`,
/// `landing.source` and the `config` hashes, which `render` computes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub route: String,
    pub tier: String,
    pub landing: Landing,
    pub candidate: Candidate,
    #[serde(default)]
    pub proof: Vec<Proof>,
    pub builder: Vec<Session>,
    #[serde(default)]
    pub review: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Landing {
    /// The edge commit the landing applies onto: the landing commit's parent.
    pub base: String,
    /// The tree the lead's landing checks ran on (attribution; not retained).
    pub proof_tree: String,
    /// `src-1:` digest of `proof_tree` without `.beads/issues.jsonl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    /// The builder's base commit, an ancestor of `landing.base`.
    pub base: String,
    /// The reviewed tree (attribution; not retained).
    pub tree: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub on: String,
    pub check: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub platform: String,
    pub exit: i32,
    /// nextest's counts; all three or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    /// For a nonzero exit, an open bead that tracks the failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_failure: Option<String>,
}

/// A builder session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub harness: String,
    pub harness_version: String,
    pub model: String,
    pub effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub stage: String,
    pub harness: String,
    pub harness_version: String,
    pub model: String,
    pub rounds: Vec<Round>,
    pub round1_blockers: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Round {
    pub verdict: String,
    pub effort: String,
}

impl Review {
    /// The effort of the first, full review; re-checks may run lower.
    pub fn effort(&self) -> &str {
        self.rounds
            .first()
            .map_or("", |round| round.effort.as_str())
    }

    pub fn role(&self) -> &'static str {
        if self.stage == "plan" {
            "plan-reviewer"
        } else {
            "code-reviewer"
        }
    }
}

impl Record {
    /// Whether any field `render` computes is present.
    pub fn has_computed_fields(&self) -> bool {
        self.schema.is_some()
            || self.landing.source.is_some()
            || self.builder.iter().any(|session| session.config.is_some())
            || self.review.iter().any(|review| review.config.is_some())
    }

    /// Fills the computed fields from the landing's source digest and the
    /// rules blobs at `candidate.base`.
    pub fn complete(&mut self, source: String, rules: &[String; 4]) {
        self.schema = Some(SCHEMA.into());
        self.landing.source = Some(source);
        for session in &mut self.builder {
            session.config = Some(config_hash(&ConfigInputs {
                role: "builder",
                harness: &session.harness,
                harness_version: &session.harness_version,
                model: &session.model,
                effort: &session.effort,
                rules,
            }));
        }
        for review in &mut self.review {
            review.config = Some(config_hash(&ConfigInputs {
                role: review.role(),
                harness: &review.harness,
                harness_version: &review.harness_version,
                model: &review.model,
                effort: review.effort(),
                rules,
            }));
        }
    }

    /// The comment text: header, readable lines, and the authoritative JSON
    /// line. No trailing newline.
    pub fn render(&self) -> String {
        let source = self.landing.source.as_deref().unwrap_or("");
        let mut lines = vec![
            HEADER.to_string(),
            format!(
                "Landed onto {} with source {}; route {}; tier {}.",
                short(&self.landing.base),
                short_label(source),
                self.route,
                self.tier
            ),
            format!(
                "Proof tree {}; reviewed tree {} on {}.",
                short(&self.landing.proof_tree),
                short(&self.candidate.tree),
                short(&self.candidate.base)
            ),
        ];
        for proof in &self.proof {
            let tree = if proof.on == "landed" {
                "landed"
            } else {
                "reviewed"
            };
            let target = proof
                .target
                .as_deref()
                .map_or(String::new(), |target| format!(" {target}"));
            let mut line = format!(
                "Proof, {tree} tree: {}{target}, {}: exit {}",
                proof.check, proof.platform, proof.exit
            );
            if let (Some(passed), Some(failed), Some(skipped)) =
                (proof.passed, proof.failed, proof.skipped)
            {
                line.push_str(&format!(
                    ", {passed} passed, {failed} failed, {skipped} skipped"
                ));
            }
            if let Some(bead) = &proof.known_failure {
                line.push_str(&format!("; known failure {bead}"));
            }
            lines.push(line);
        }
        for session in &self.builder {
            lines.push(format!(
                "Builder: {} {}, {}, effort {}, {}",
                session.harness,
                session.harness_version,
                session.model,
                session.effort,
                short_label(session.config.as_deref().unwrap_or(""))
            ));
        }
        for review in &self.review {
            let stage = if review.stage == "plan" {
                "Plan"
            } else {
                "Code"
            };
            let rounds: Vec<String> = review
                .rounds
                .iter()
                .map(|round| format!("{} ({})", round.verdict, round.effort))
                .collect();
            let plural = if review.round1_blockers == 1 { "" } else { "s" };
            lines.push(format!(
                "{stage} review: {} {}, {}, {}: {}; {} round-1 blocker{plural}",
                review.harness,
                review.harness_version,
                review.model,
                short_label(review.config.as_deref().unwrap_or("")),
                rounds.join(", "),
                review.round1_blockers
            ));
        }
        lines.push(serde_json::to_string(self).expect("a record always serializes"));
        lines.join("\n")
    }
}

pub struct ConfigInputs<'a> {
    pub role: &'a str,
    pub harness: &'a str,
    pub harness_version: &'a str,
    pub model: &'a str,
    pub effort: &'a str,
    pub rules: &'a [String; 4],
}

/// The provisional configuration record: sorted keys, no whitespace. Written
/// out by hand so that serde features elsewhere in the build cannot reorder
/// it.
pub fn config_json(inputs: &ConfigInputs) -> String {
    let q = |text: &str| serde_json::to_string(text).expect("strings serialize");
    let rules: Vec<String> = RULES_FILES
        .iter()
        .zip(inputs.rules)
        .map(|(path, blob)| format!("{}:{}", q(path), q(blob)))
        .collect();
    format!(
        "{{\"effort\":{},\"harness\":{},\"harness_version\":{},\"model\":{},\"role\":{},\"rules\":{{{}}},\"scheme\":{}}}",
        q(inputs.effort),
        q(inputs.harness),
        q(inputs.harness_version),
        q(inputs.model),
        q(inputs.role),
        rules.join(","),
        q(CONFIG_SCHEME)
    )
}

pub fn config_hash(inputs: &ConfigInputs) -> String {
    format!(
        "{CONFIG_PREFIX}{}",
        sha256_hex(config_json(inputs).as_bytes())
    )
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

/// `cfg-p1:` or `src-1:` plus the first 12 hex digits.
fn short_label(label: &str) -> String {
    match label.split_once(':') {
        Some((scheme, hex)) => format!("{scheme}:{}", short(hex)),
        None => label.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    pub const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    pub fn rules() -> [String; 4] {
        ["1", "2", "3", "4"].map(|digit| digit.repeat(40))
    }

    pub fn input() -> Record {
        toml::from_str(&format!(
            r#"
route = "logic-regression"
tier = "low"
[landing]
base = "{A}"
proof_tree = "{B}"
[candidate]
base = "{A}"
tree = "{B}"
[[proof]]
on = "candidate"
check = "eon-test"
target = "workspace"
platform = "x86_64-linux"
exit = 0
passed = 10
failed = 0
skipped = 1
[[proof]]
on = "landed"
check = "nix-build"
target = ".#default"
platform = "x86_64-linux"
exit = 0
[[builder]]
harness = "claude-code"
harness_version = "2.1.326"
model = "claude-opus-5-5"
effort = "high"
[[review]]
stage = "code"
harness = "codex-cli"
harness_version = "0.170.0"
model = "gpt-6.2-sol"
rounds = [{{ verdict = "changes-needed", effort = "high" }}, {{ verdict = "approve", effort = "high" }}]
round1_blockers = 1
"#
        ))
        .unwrap()
    }

    #[test]
    fn the_config_record_is_canonical_and_its_hash_fixed() {
        let rules = rules();
        let inputs = ConfigInputs {
            role: "builder",
            harness: "claude-code",
            harness_version: "2.1.326",
            model: "claude-opus-5-5",
            effort: "high",
            rules: &rules,
        };
        assert_eq!(
            config_json(&inputs),
            format!(
                "{{\"effort\":\"high\",\"harness\":\"claude-code\",\"harness_version\":\"2.1.326\",\
                 \"model\":\"claude-opus-5-5\",\"role\":\"builder\",\"rules\":{{\"AGENTS.md\":\"{}\",\
                 \"REVIEW.md\":\"{}\",\"docs/AGENT-PROTOCOLS.md\":\"{}\",\"docs/AGENT-RULES.md\":\"{}\"}},\
                 \"scheme\":\"eon-config/provisional-1\"}}",
                rules[0], rules[1], rules[2], rules[3]
            )
        );
        // The JSON parses, and its keys are in sorted order.
        let value: serde_json::Value = serde_json::from_str(&config_json(&inputs)).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert_eq!(
            config_hash(&inputs),
            format!("cfg-p1:{}", sha256_hex(config_json(&inputs).as_bytes()))
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_rendered_comment_is_fixed_and_its_json_line_round_trips() {
        let mut record = input();
        assert!(!record.has_computed_fields());
        record.complete(format!("src-1:{}", "c".repeat(64)), &rules());
        assert!(record.has_computed_fields());
        let text = record.render();
        let lines: Vec<&str> = text.lines().collect();
        let builder = record.builder[0].config.as_deref().unwrap();
        let reviewer = record.review[0].config.as_deref().unwrap();
        assert_eq!(
            lines[..7],
            [
                "Close record eon-close-record/1".to_string(),
                "Landed onto aaaaaaaaaaaa with source src-1:cccccccccccc; route logic-regression; tier low."
                    .into(),
                "Proof tree bbbbbbbbbbbb; reviewed tree bbbbbbbbbbbb on aaaaaaaaaaaa.".into(),
                "Proof, reviewed tree: eon-test workspace, x86_64-linux: exit 0, 10 passed, 0 failed, 1 skipped"
                    .into(),
                "Proof, landed tree: nix-build .#default, x86_64-linux: exit 0".into(),
                format!(
                    "Builder: claude-code 2.1.326, claude-opus-5-5, effort high, cfg-p1:{}",
                    &builder[7..19]
                ),
                format!(
                    "Code review: codex-cli 0.170.0, gpt-6.2-sol, cfg-p1:{}: changes-needed (high), \
                     approve (high); 1 round-1 blocker",
                    &reviewer[7..19]
                ),
            ]
        );
        assert_eq!(lines.len(), 8);
        assert!(lines[7].starts_with("{\"schema\":\"eon-close-record/1\",\"route\":"));
        let parsed: Record = serde_json::from_str(lines[7]).unwrap();
        assert_eq!(parsed, record);
        assert_eq!(parsed.render(), text);
        assert_ne!(builder, reviewer);
    }
}
