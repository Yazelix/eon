//! The hashed records: configurations (`eon-bench-config/2`, with `eon-bench-config/1` kept
//! readable for comparison; judges included) and tasks (`eon-bench-task/1`).
//!
//! Every field is required, unknown fields are refused, and every string is validated, so a
//! record that parses is complete: there is no partial hash. A nullable field must still be
//! present (`null` is a value, absence is not). The hash is SHA-256 over the RFC 8785
//! canonical JSON of the parsed record, and the canonical bytes are what the store keeps.
//!
//! `eon-bench-config/2` adds `containment` (`clone` or `jail`) and `plan_review` (`"none"`, or
//! `{"reviewer_config": "<64-hex>", "max_rounds": <u64 >= 1>}`). At `jail`, the four jail,
//! proxy and rota digests (`permissions.proxy_allowlist_sha256`, `tools.rota_commit`,
//! `tools.rota_backend_sha256`, `tools.jail_profile_sha256`) are required non-null; at `clone`,
//! all four must be `null` so a clone run never fakes them.
//!
//! A judge configuration is a configuration with `role = "judge"` and `rubric_sha256`; any
//! other role refuses the `rubric_sha256` key, even as `null`. The task record has no prompt
//! field, so the role wrapper never reaches a task hash.

use crate::{Result, fail, is_lower_hex, jcs, sha256_hex, time};
use serde::de::{Deserializer, Error as _, MapAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::fmt;

pub const CONFIG_SCHEMA_V1: &str = "eon-bench-config/1";
pub const CONFIG_SCHEMA: &str = "eon-bench-config/2";
pub const TASK_SCHEMA: &str = "eon-bench-task/1";

/// Defines a string newtype whose deserializer refuses values `$valid` rejects.
macro_rules! checked_string {
    ($(#[$doc:meta])* $name:ident, $valid:expr, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Result<Self> {
                let text = text.into();
                let valid: fn(&str) -> bool = $valid;
                if valid(&text) {
                    Ok(Self(text))
                } else {
                    fail!(concat!("not ", $what, ": {:?}"), text)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                Self::new(String::deserialize(deserializer)?).map_err(|error| D::Error::custom(error.0))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

checked_string!(
    /// A SHA-256 digest: 64 lowercase hex digits.
    Digest, |t| t.len() == 64 && is_lower_hex(t), "a SHA-256 hex digest"
);
checked_string!(
    /// A git object id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
    Commit, |t| (t.len() == 40 || t.len() == 64) && is_lower_hex(t), "a git commit id"
);
checked_string!(
    /// Non-empty text with no surrounding whitespace.
    Text, |t| !t.is_empty() && t.trim() == t, "non-empty text without surrounding whitespace"
);
checked_string!(
    /// A UTC timestamp, `YYYY-MM-DDTHH:MM:SSZ`.
    Utc, time::is_utc, "a YYYY-MM-DDTHH:MM:SSZ timestamp"
);
checked_string!(
    /// A semantic version: `MAJOR.MINOR.PATCH` with an optional `-pre` and `+build`.
    Semver, is_semver, "a semantic version"
);

fn is_semver(text: &str) -> bool {
    let (core, suffix) = match text.find(['-', '+']) {
        Some(at) => text.split_at(at),
        None => (text, ""),
    };
    let numeric = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| numeric(part))
        && (suffix.is_empty()
            || (suffix.len() > 1
                && suffix[1..]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Builder,
    Reviewer,
    Planner,
    Judge,
}

/// A task's role: never `judge`, which only configurations have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskRole {
    Builder,
    Reviewer,
    Planner,
}

impl TaskRole {
    pub fn matches(self, role: Role) -> bool {
        matches!(
            (self, role),
            (Self::Builder, Role::Builder)
                | (Self::Reviewer, Role::Reviewer)
                | (Self::Planner, Role::Planner)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Micro,
    Mini,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Containment {
    Clone,
    Jail,
}

impl fmt::Display for Containment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clone => f.write_str("clone"),
            Self::Jail => f.write_str("jail"),
        }
    }
}

/// Plan-review configuration on `eon-bench-config/2`: `"none"`, or the reviewer arm's config
/// hash and a round cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanReview {
    None,
    Enabled {
        reviewer_config: Digest,
        max_rounds: u64,
    },
}

impl Serialize for PlanReview {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_str("none"),
            Self::Enabled {
                reviewer_config,
                max_rounds,
            } => {
                #[derive(Serialize)]
                struct PlanReviewObject<'a> {
                    reviewer_config: &'a Digest,
                    max_rounds: u64,
                }
                PlanReviewObject {
                    reviewer_config,
                    max_rounds: *max_rounds,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for PlanReview {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PlanReviewSpec {
            reviewer_config: Digest,
            max_rounds: u64,
        }

        struct PlanReviewVisitor;

        impl<'de> Visitor<'de> for PlanReviewVisitor {
            type Value = PlanReview;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("\"none\" or an object with `reviewer_config` and `max_rounds`")
            }

            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<PlanReview, E> {
                if value == "none" {
                    Ok(PlanReview::None)
                } else {
                    Err(E::custom(format!(
                        "plan_review must be \"none\" or an object with `reviewer_config` and `max_rounds`, not {value:?}"
                    )))
                }
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<PlanReview, A::Error> {
                let spec = PlanReviewSpec::deserialize(MapAccessDeserializer::new(map))?;
                Ok(PlanReview::Enabled {
                    reviewer_config: spec.reviewer_config,
                    max_rounds: spec.max_rounds,
                })
            }
        }

        deserializer.deserialize_any(PlanReviewVisitor)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: Text,
    pub bench: Bench,
    pub role: Role,
    /// Required on `eon-bench-config/2`; refused on `eon-bench-config/1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containment: Option<Containment>,
    /// Required on `eon-bench-config/2`; refused on `eon-bench-config/1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review: Option<PlanReview>,
    pub model: Model,
    pub harness: Harness,
    pub effort: Effort,
    pub prompt: Prompt,
    pub harness_home: HarnessHome,
    pub permissions: Permissions,
    pub limits: Limits,
    pub tools: Tools,
    /// Judges only; see the module documentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric_sha256: Option<Digest>,
}

impl Config {
    /// Whether this record uses the historical `eon-bench-config/1` schema.
    pub fn is_v1(&self) -> bool {
        self.schema.as_str() == CONFIG_SCHEMA_V1
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bench {
    pub version: Semver,
    pub commit: Commit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub family: Text,
    /// The model id as passed to the harness; a run's server-reported id is compared to it.
    pub id: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    pub name: Text,
    pub version: Text,
    pub binary_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effort {
    /// The native value passed to the harness.
    pub value: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
    /// The wrapper template assembled with `task_text.md`.
    pub role_wrapper_sha256: Digest,
    /// In load order, which can change behavior.
    pub fleet_rules: Vec<NamedDigest>,
    pub repo_overlays: Vec<PathDigest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedDigest {
    pub name: Text,
    pub sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathDigest {
    pub path: Text,
    pub sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessHome {
    /// Settings, instructions, skills, plugins, MCP and hooks, each hashed inside the manifest.
    pub manifest_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Harness-native values: they differ per harness, so they are text, not enums.
    pub mode: Text,
    pub sandbox: Text,
    pub web_tools: Text,
    /// Required non-null at `jail` (and on `/1`); must be `null` at `clone`. Key always required.
    #[serde(deserialize_with = "Option::deserialize")]
    pub proxy_allowlist_sha256: Option<Digest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Above 0 and at most 1000, with at most three decimal places.
    pub wall_cap_multiplier: f64,
    /// At least 1, or `null` for no limit; the key is required either way.
    #[serde(deserialize_with = "Option::deserialize")]
    pub max_turns: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// Required non-null at `jail` (and on `/1`); must be `null` at `clone`. Key always required.
    #[serde(deserialize_with = "Option::deserialize")]
    pub rota_commit: Option<Commit>,
    /// Required non-null at `jail` (and on `/1`); must be `null` at `clone`. Key always required.
    #[serde(deserialize_with = "Option::deserialize")]
    pub rota_backend_sha256: Option<Digest>,
    pub rtk: Rtk,
    pub dcg: Dcg,
    pub heavy_sh_sha256: Digest,
    /// Required non-null at `jail` (and on `/1`); must be `null` at `clone`. Key always required.
    #[serde(deserialize_with = "Option::deserialize")]
    pub jail_profile_sha256: Option<Digest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rtk {
    pub enabled: bool,
    /// `null` if and only if `enabled` is false.
    #[serde(deserialize_with = "Option::deserialize")]
    pub version: Option<Text>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dcg {
    pub version: Text,
    pub config_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub schema: Text,
    pub repo: Text,
    pub role: TaskRole,
    pub task_id: Text,
    pub tier_introduced: Tier,
    pub base_commit: Commit,
    /// The synthetic workspace's object set (`agent-bench workspace create` prints it).
    pub snapshot_object_set_sha256: Digest,
    pub landed_commit: Commit,
    pub landed_at: Utc,
    pub task_text: TaskText,
    pub key: Key,
    pub env: Env,
    pub test: Test,
    pub scope_paths: Vec<Text>,
    pub protected_paths: Vec<Text>,
    pub scorer: Scorer,
    pub key_edits_sha256: Digest,
    /// Seconds, at least 1.
    pub wall_cap_s: u64,
    pub key_author_family: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskText {
    /// `task_text.md`, which never holds the role wrapper.
    pub sha256: Digest,
    pub source_commit: Commit,
    pub cutoff: Utc,
    pub cutoff_evidence: Text,
    /// An empty addendum is the digest of zero bytes, still computable.
    pub addendum_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub files: Vec<PathDigest>,
    pub f2p: Vec<Text>,
    pub p2p: Vec<Text>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Env {
    pub dev_shell: Text,
    /// Package names left out of the task store and PATH.
    pub exclude: Vec<Text>,
    /// Name patterns of the dev shell's variables the jail keeps.
    pub pass: Vec<Text>,
    /// SHA-256 of the task environment's `env.json` (closure, variables, test templates).
    pub env_sha256: Digest,
    /// Lockfiles staged into the task's offline Cargo home.
    pub lockfiles: Vec<Lockfile>,
    /// Tree hash ([`crate::tree`]) of the task's warm target cache, identical for every arm.
    pub warm_target_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub path: Text,
    pub base_sha256: Digest,
    pub landed_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Test {
    /// Argv template, at least the program.
    pub command: Vec<Text>,
    /// The warm build's argv template, at least the program.
    pub build: Vec<Text>,
    pub junit_path: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scorer {
    pub commit: Commit,
    /// Argv template, at least the program.
    pub command: Vec<Text>,
}

/// Where a record lives in the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Config,
    Judge,
    Task,
}

impl Kind {
    pub fn dir(self) -> &'static str {
        match self {
            Self::Config => "configs",
            Self::Judge => "judges",
            Self::Task => "tasks",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Judge => "judge",
            Self::Task => "task",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Config(Config),
    Task(Task),
}

/// A validated record with its canonical bytes and their hash.
#[derive(Clone, Debug, PartialEq)]
pub struct Hashed {
    pub record: Record,
    pub canonical: String,
    pub hash: String,
}

impl Hashed {
    pub fn kind(&self) -> Kind {
        self.record.kind()
    }
}

impl Record {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Config(config) if config.role == Role::Judge => Kind::Judge,
            Self::Config(_) => Kind::Config,
            Self::Task(_) => Kind::Task,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Config(config) => serde_json::to_value(config),
            Self::Task(task) => serde_json::to_value(task),
        }
        .expect("records serialize")
    }
}

/// Parses, validates and hashes one record. The only way to get a hash: an input that is
/// missing a field, or has one that cannot be computed, gets an error instead.
pub fn hash(input: &[u8]) -> Result<Hashed> {
    jcs::reject_duplicate_names(input)?;
    let value: Value = serde_json::from_slice(input)?;
    hash_value(value)
}

/// Parses, validates and hashes one record already parsed from duplicate-checked JSON.
pub(crate) fn hash_value(value: Value) -> Result<Hashed> {
    let record = parse(value)?;
    let canonical = jcs::canonical(&record.to_value())?;
    let hash = sha256_hex(canonical.as_bytes());
    Ok(Hashed {
        record,
        canonical,
        hash,
    })
}

/// Parses and validates a task record.
pub fn task(input: &[u8]) -> Result<Task> {
    match hash(input)?.record {
        Record::Task(task) => Ok(task),
        Record::Config(_) => fail!("not a task record"),
    }
}

/// Verifies a stored configuration referenced by `plan_review.reviewer_config`: it must use
/// `eon-bench-config/2`, have `role == "reviewer"`, and match `expected` containment.
pub(crate) fn validate_plan_reviewer(
    reviewer: &Config,
    hash: &str,
    expected: Containment,
) -> Result<()> {
    if reviewer.is_v1() {
        fail!(
            "plan_review.reviewer_config {} is {}; plan review requires {}",
            crate::short(hash),
            CONFIG_SCHEMA_V1,
            CONFIG_SCHEMA
        );
    }
    if reviewer.role != Role::Reviewer {
        fail!(
            "plan_review.reviewer_config {} has role {:?}, expected reviewer",
            crate::short(hash),
            reviewer.role
        );
    }
    match reviewer.containment {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => fail!(
            "plan_review.reviewer_config {} has containment {actual}, expected {expected}",
            crate::short(hash)
        ),
        None => fail!(
            "plan_review.reviewer_config {} is missing containment",
            crate::short(hash)
        ),
    }
}

fn parse(value: Value) -> Result<Record> {
    let Some(object) = value.as_object() else {
        fail!("a record is a JSON object");
    };
    let schema = object.get("schema").and_then(Value::as_str);
    match schema {
        Some(CONFIG_SCHEMA_V1 | CONFIG_SCHEMA) => {
            if schema == Some(CONFIG_SCHEMA_V1) {
                if object.contains_key("containment") {
                    fail!("containment: {CONFIG_SCHEMA_V1} does not allow `containment`");
                }
                if object.contains_key("plan_review") {
                    fail!("plan_review: {CONFIG_SCHEMA_V1} does not allow `plan_review`");
                }
            }
            let judge = object.get("role").and_then(Value::as_str) == Some("judge");
            if !judge && object.contains_key("rubric_sha256") {
                fail!("rubric_sha256: only a judge configuration has a rubric");
            }
            let config: Config = jcs::from_value_objects(value)?;
            validate_config(&config)?;
            Ok(Record::Config(config))
        }
        Some(TASK_SCHEMA) => {
            let task: Task = jcs::from_value_objects(value)?;
            validate_task(&task)?;
            Ok(Record::Task(task))
        }
        Some(other) => fail!("unknown schema {other:?}"),
        None => fail!("missing field `schema`"),
    }
}

fn validate_config(config: &Config) -> Result<()> {
    if config.role == Role::Judge && config.rubric_sha256.is_none() {
        fail!("rubric_sha256: a judge configuration needs its rubric hash");
    }
    let containment_fields = [
        (
            "permissions.proxy_allowlist_sha256",
            config.permissions.proxy_allowlist_sha256.is_some(),
        ),
        ("tools.rota_commit", config.tools.rota_commit.is_some()),
        (
            "tools.rota_backend_sha256",
            config.tools.rota_backend_sha256.is_some(),
        ),
        (
            "tools.jail_profile_sha256",
            config.tools.jail_profile_sha256.is_some(),
        ),
    ];
    if config.is_v1() {
        for (field, present) in containment_fields {
            if !present {
                fail!("{field}: {CONFIG_SCHEMA_V1} requires a non-null value");
            }
        }
    } else {
        let Some(containment) = config.containment else {
            fail!("containment: {CONFIG_SCHEMA} requires `containment` (`clone` or `jail`)");
        };
        let Some(plan_review) = &config.plan_review else {
            fail!("plan_review: {CONFIG_SCHEMA} requires `plan_review` (\"none\" or an object)");
        };
        for (field, present) in containment_fields {
            match (containment, present) {
                (Containment::Jail, false) => {
                    fail!("{field}: required non-null for jail containment");
                }
                (Containment::Clone, true) => {
                    fail!("{field}: must be null for clone containment");
                }
                _ => {}
            }
        }
        if let PlanReview::Enabled { max_rounds, .. } = plan_review {
            if *max_rounds == 0 {
                fail!("plan_review.max_rounds must be at least 1");
            }
            if matches!(config.role, Role::Reviewer | Role::Judge) {
                fail!("plan_review: only builder or planner configurations may enable plan_review");
            }
        }
    }
    // Three decimal places at most, so serde_json parses it exactly (see `jcs`).
    let multiplier = config.limits.wall_cap_multiplier;
    if !(multiplier > 0.0 && multiplier <= 1000.0)
        || format!("{multiplier:.3}").parse::<f64>() != Ok(multiplier)
    {
        fail!(
            "limits.wall_cap_multiplier must be above 0 and at most 1000, with at most three \
             decimal places, not {multiplier}"
        );
    }
    if config.limits.max_turns == Some(0) {
        fail!("limits.max_turns must be at least 1 or null");
    }
    match (config.tools.rtk.enabled, &config.tools.rtk.version) {
        (true, None) => fail!("tools.rtk.version: an enabled RTK names its version"),
        (false, Some(_)) => fail!("tools.rtk.version must be null while RTK is disabled"),
        _ => Ok(()),
    }
}

fn validate_task(task: &Task) -> Result<()> {
    if task.wall_cap_s == 0 {
        fail!("wall_cap_s must be at least 1");
    }
    if task.test.command.is_empty() {
        fail!("test.command needs at least the program");
    }
    if task.test.build.is_empty() {
        fail!("test.build needs at least the program");
    }
    if task.scorer.command.is_empty() {
        fail!("scorer.command needs at least the program");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) const CONFIG: &str = include_str!("fixtures/config.json");
    pub(crate) const CONFIG_CLONE: &str = include_str!("fixtures/config_clone.json");
    pub(crate) const CONFIG_V1: &str = include_str!("fixtures/config_v1.json");
    pub(crate) const JUDGE: &str = include_str!("fixtures/judge.json");
    pub(crate) const JUDGE_V1: &str = include_str!("fixtures/judge_v1.json");
    pub(crate) const TASK: &str = include_str!("fixtures/task.json");

    /// Pinned digests of the fixtures, cross-checked with Python's `json.dumps(sort_keys=True,
    /// separators=(',', ':'))` (the fixtures' keys are ASCII and their numbers plain). A change
    /// here means every stored hash changes too.
    pub(crate) const GOLDEN_CONFIG: &str =
        "de7d3b13a5db79a285fc02b3f9a051fc97fffc3086e2b24ba659f2da6361a50b";
    pub(crate) const GOLDEN_CONFIG_CLONE: &str =
        "b599be3855e7f25d5fd06a39705bb85bf93a48e2c4cb017e2d9d20b092da1449";
    const GOLDEN_CONFIG_V1: &str =
        "0a463fa2288e89c6aecd89bb1819ace99c8ea073e9440b3feb43c43509101100";
    const GOLDEN_JUDGE: &str = "32ac75b106be17f601bf4b5d78982b5e8e2d57217b9cec10222c36a36bbd4f40";
    const GOLDEN_JUDGE_V1: &str =
        "5b0e5dd30093f54cb9ab26b19bc448dfcf9743ae263ca9fcb9e167a6f5c4a789";
    pub(crate) const GOLDEN_TASK: &str =
        "4b99405a84bf4646a1dddc5013b03778d7e4bafd0dafd887fa4de6bb3154f430";

    fn value(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn hash_json(value: &Value) -> Result<Hashed> {
        hash(&serde_json::to_vec(value).unwrap())
    }

    fn config_with_plan_review() -> Value {
        let mut reviewed = value(CONFIG);
        reviewed["plan_review"] = json!({
            "reviewer_config": "a".repeat(64),
            "max_rounds": 2
        });
        reviewed
    }

    fn all_records() -> Vec<(&'static str, Value)> {
        vec![
            ("config_v2_jail", value(CONFIG)),
            ("config_v2_clone", value(CONFIG_CLONE)),
            ("config_v2_plan_review", config_with_plan_review()),
            ("judge_v2", value(JUDGE)),
            ("config_v1", value(CONFIG_V1)),
            ("judge_v1", value(JUDGE_V1)),
            ("task_v1", value(TASK)),
        ]
    }

    #[test]
    fn golden_hashes_are_stable() {
        for (fixture, golden, kind) in [
            (CONFIG, GOLDEN_CONFIG, Kind::Config),
            (CONFIG_CLONE, GOLDEN_CONFIG_CLONE, Kind::Config),
            (CONFIG_V1, GOLDEN_CONFIG_V1, Kind::Config),
            (JUDGE, GOLDEN_JUDGE, Kind::Judge),
            (JUDGE_V1, GOLDEN_JUDGE_V1, Kind::Judge),
            (TASK, GOLDEN_TASK, Kind::Task),
        ] {
            let first = hash(fixture.as_bytes()).unwrap();
            let second = hash(fixture.as_bytes()).unwrap();
            assert_eq!(first, second);
            assert_eq!(first.kind(), kind);
            assert_eq!(first.hash, golden, "{}", first.canonical);
            assert_eq!(sha256_hex(first.canonical.as_bytes()), first.hash);
        }
    }

    #[test]
    fn member_order_and_whitespace_do_not_change_the_hash() {
        let compact = serde_json::to_vec(&value(TASK)).unwrap();
        assert_eq!(
            hash(&compact).unwrap().hash,
            hash(TASK.as_bytes()).unwrap().hash
        );
    }

    /// Every leaf path of `value`, with empty containers as leaves.
    fn leaves(value: &Value, path: Vec<PathStep>, out: &mut Vec<Vec<PathStep>>) {
        match value {
            Value::Object(members) if !members.is_empty() => {
                for (name, item) in members {
                    let mut next = path.clone();
                    next.push(PathStep::Name(name.clone()));
                    leaves(item, next, out);
                }
            }
            Value::Array(items) if !items.is_empty() => {
                for (index, item) in items.iter().enumerate() {
                    let mut next = path.clone();
                    next.push(PathStep::Index(index));
                    leaves(item, next, out);
                }
            }
            _ => out.push(path),
        }
    }

    /// Every member path (objects and leaves, not array entries) of `value`.
    fn members(value: &Value, path: Vec<PathStep>, out: &mut Vec<Vec<PathStep>>) {
        match value {
            Value::Object(items) => {
                for (name, item) in items {
                    let mut next = path.clone();
                    next.push(PathStep::Name(name.clone()));
                    out.push(next.clone());
                    members(item, next, out);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let mut next = path.clone();
                    next.push(PathStep::Index(index));
                    members(item, next, out);
                }
            }
            _ => {}
        }
    }

    #[derive(Clone, Debug)]
    enum PathStep {
        Name(String),
        Index(usize),
    }

    fn at<'a>(value: &'a mut Value, path: &[PathStep]) -> &'a mut Value {
        path.iter().fold(value, |value, step| match step {
            PathStep::Name(name) => &mut value[name.as_str()],
            PathStep::Index(index) => &mut value[*index],
        })
    }

    fn dotted(path: &[PathStep]) -> String {
        path.iter()
            .map(|step| match step {
                PathStep::Name(name) => format!(".{name}"),
                PathStep::Index(index) => format!("[{index}]"),
            })
            .collect()
    }

    /// Candidate replacements for a leaf, within its valid domain.
    fn mutations(leaf: &Value) -> Vec<Value> {
        match leaf {
            Value::Bool(flag) => vec![json!(!flag)],
            Value::Null => vec![
                json!(7),
                json!("9.9.9"),
                json!("2".repeat(40)),
                json!("4".repeat(64)),
            ],
            Value::Number(number) => match number.as_u64() {
                Some(integer) => vec![json!(integer + 1)],
                None => vec![json!(number.as_f64().unwrap() + 1.0)],
            },
            Value::String(text) => {
                let mut out = Vec::new();
                // Change the last digit or hex digit, which keeps digests, commits, versions
                // and timestamps well-formed.
                if let Some(at) = text.rfind(|c: char| c.is_ascii_hexdigit()) {
                    let old = text.as_bytes()[at];
                    let new = if old == b'0' { '1' } else { '0' };
                    let mut changed = text.clone();
                    changed.replace_range(at..=at, &new.to_string());
                    out.push(json!(changed));
                }
                out.push(json!(format!("{text}x")));
                for other in [
                    "builder", "reviewer", "planner", "micro", "mini", "clone", "jail",
                ] {
                    if other != text {
                        out.push(json!(other));
                    }
                }
                if text == "none" {
                    out.push(json!({
                        "reviewer_config": "a".repeat(64),
                        "max_rounds": 2
                    }));
                }
                out
            }
            Value::Array(_) => vec![
                json!(["added"]),
                json!([{"path": "added", "sha256": "0".repeat(64)}]),
                json!([{"name": "added", "sha256": "0".repeat(64)}]),
            ],
            Value::Object(_) => vec![],
        }
    }

    /// Populates any still-null jail/proxy/rota digest fields on `edited` with valid values.
    fn fill_jail_digests(edited: &mut Value) {
        if edited["permissions"]["proxy_allowlist_sha256"].is_null() {
            edited["permissions"]["proxy_allowlist_sha256"] = json!("0".repeat(64));
        }
        if edited["tools"]["rota_commit"].is_null() {
            edited["tools"]["rota_commit"] = json!("2".repeat(40));
        }
        if edited["tools"]["rota_backend_sha256"].is_null() {
            edited["tools"]["rota_backend_sha256"] = json!("1".repeat(64));
        }
        if edited["tools"]["jail_profile_sha256"].is_null() {
            edited["tools"]["jail_profile_sha256"] = json!("4".repeat(64));
        }
    }

    /// Sets all four jail/proxy/rota digest fields on `edited` to `null`.
    fn clear_jail_digests(edited: &mut Value) {
        edited["permissions"]["proxy_allowlist_sha256"] = Value::Null;
        edited["tools"]["rota_commit"] = Value::Null;
        edited["tools"]["rota_backend_sha256"] = Value::Null;
        edited["tools"]["jail_profile_sha256"] = Value::Null;
    }

    /// Fields whose validity depends on another field move with it.
    fn couple(edited: &mut Value, path: &str) {
        match path {
            // RTK's version is null exactly when RTK is off.
            ".tools.rtk.enabled" => {
                let rtk = &mut edited["tools"]["rtk"];
                rtk["version"] = if rtk["enabled"] == json!(true) {
                    json!("1.0.0")
                } else {
                    Value::Null
                };
            }
            ".tools.rtk.version" => {
                let rtk = &mut edited["tools"]["rtk"];
                rtk["enabled"] = json!(!rtk["version"].is_null());
            }
            // Only a judge has a rubric, and reviewer/judge cannot enable plan_review.
            ".role" => {
                if edited["role"] != json!("judge") {
                    edited.as_object_mut().unwrap().remove("rubric_sha256");
                }
                if matches!(edited["role"].as_str(), Some("reviewer" | "judge"))
                    && edited.get("plan_review").is_some_and(Value::is_object)
                {
                    edited["plan_review"] = json!("none");
                }
            }
            // Containment governs the four jail/proxy/rota digests.
            ".containment" => {
                if edited["containment"] == json!("clone") {
                    clear_jail_digests(edited);
                } else if edited["containment"] == json!("jail") {
                    fill_jail_digests(edited);
                }
            }
            // On a clone fixture, turning any of the four null digests non-null requires
            // switching containment to jail and filling the remaining three digests.
            ".permissions.proxy_allowlist_sha256"
            | ".tools.rota_commit"
            | ".tools.rota_backend_sha256"
            | ".tools.jail_profile_sha256"
                if edited.get("containment") == Some(&json!("clone")) =>
            {
                edited["containment"] = json!("jail");
                fill_jail_digests(edited);
            }
            // Enabling plan_review on a judge or reviewer fixture requires a builder/planner role
            // (and dropping judge-only rubric_sha256).
            ".plan_review"
                if edited["plan_review"].is_object()
                    && matches!(edited["role"].as_str(), Some("judge" | "reviewer")) =>
            {
                edited["role"] = json!("builder");
                edited.as_object_mut().unwrap().remove("rubric_sha256");
            }
            _ => {}
        }
    }

    #[test]
    fn every_leaf_change_changes_the_hash() {
        for (label, original) in all_records() {
            let base = hash_json(&original).unwrap().hash;
            let mut paths = Vec::new();
            leaves(&original, Vec::new(), &mut paths);
            assert!(paths.len() > 20, "{label}");
            for path in paths {
                // The schema names the record type; another value is another record, not a
                // changed field.
                if dotted(&path) == ".schema" {
                    continue;
                }
                let leaf = at(&mut original.clone(), &path).clone();
                let changed = mutations(&leaf).into_iter().find_map(|replacement| {
                    let mut edited = original.clone();
                    *at(&mut edited, &path) = replacement;
                    couple(&mut edited, &dotted(&path));
                    hash_json(&edited).ok()
                });
                let changed = changed
                    .unwrap_or_else(|| panic!("{label}: no valid mutation for {}", dotted(&path)));
                assert_ne!(
                    changed.hash,
                    base,
                    "{label}: {} did not change the hash",
                    dotted(&path)
                );
            }
        }
    }

    #[test]
    fn a_repeated_member_is_refused() {
        // Without the check, the second `wall_cap_s` would be hashed and the first dropped.
        let repeated = TASK.replacen('{', r#"{"wall_cap_s": 1, "#, 1);
        let error = hash(repeated.as_bytes()).unwrap_err().0;
        assert!(error.contains("appears twice"), "{error}");
        let nested = CONFIG.replacen(r#""effort": {"#, r#""effort": {"value": "low", "#, 1);
        assert_ne!(nested, CONFIG);
        assert!(hash(nested.as_bytes()).is_err());
    }

    #[test]
    fn every_missing_field_is_refused() {
        for (label, original) in all_records() {
            let mut paths = Vec::new();
            members(&original, Vec::new(), &mut paths);
            for path in paths {
                let mut edited = original.clone();
                let (last, parent) = path.split_last().unwrap();
                let PathStep::Name(name) = last else {
                    unreachable!()
                };
                at(&mut edited, parent)
                    .as_object_mut()
                    .unwrap()
                    .remove(name);
                assert!(
                    hash_json(&edited).is_err(),
                    "{label}: removing {} was accepted",
                    dotted(&path)
                );
            }
        }
    }

    #[test]
    fn invalid_values_are_refused() {
        let config = value(CONFIG);
        let clone = value(CONFIG_CLONE);
        let v1 = value(CONFIG_V1);
        let judge = value(JUDGE);
        let task = value(TASK);
        type Case = (&'static str, Value, fn(&mut Value));
        let cases: Vec<Case> = vec![
            ("empty text", config.clone(), |v| {
                v["model"]["id"] = json!("")
            }),
            ("padded text", config.clone(), |v| {
                v["model"]["id"] = json!(" opus")
            }),
            ("short digest", config.clone(), |v| {
                v["harness"]["binary_sha256"] = json!("abc")
            }),
            ("uppercase digest", config.clone(), |v| {
                v["harness"]["binary_sha256"] = json!("A".repeat(64))
            }),
            ("bad commit", config.clone(), |v| {
                v["bench"]["commit"] = json!("1234")
            }),
            ("bad semver", config.clone(), |v| {
                v["bench"]["version"] = json!("1.2")
            }),
            ("unknown key", config.clone(), |v| {
                v["model"]["extra"] = json!("x")
            }),
            ("unknown role", config.clone(), |v| {
                v["role"] = json!("tester")
            }),
            ("wrong schema", config.clone(), |v| {
                v["schema"] = json!("eon-bench-config/3")
            }),
            ("null containment on v2", config.clone(), |v| {
                v["containment"] = Value::Null
            }),
            ("unknown containment on v2", config.clone(), |v| {
                v["containment"] = json!("vm")
            }),
            ("null plan_review on v2", config.clone(), |v| {
                v["plan_review"] = Value::Null
            }),
            ("unknown plan_review string", config.clone(), |v| {
                v["plan_review"] = json!("always")
            }),
            ("plan_review zero rounds", config.clone(), |v| {
                v["plan_review"] = json!({"reviewer_config": "a".repeat(64), "max_rounds": 0})
            }),
            (
                "plan_review unknown field",
                config.clone(),
                |v| {
                    v["plan_review"] =
                        json!({"reviewer_config": "a".repeat(64), "max_rounds": 1, "extra": true})
                },
            ),
            ("plan_review missing rounds", config.clone(), |v| {
                v["plan_review"] = json!({"reviewer_config": "a".repeat(64)})
            }),
            ("plan_review bad digest", config.clone(), |v| {
                v["plan_review"] = json!({"reviewer_config": "short", "max_rounds": 1})
            }),
            ("plan_review on reviewer role", config.clone(), |v| {
                v["role"] = json!("reviewer");
                v["plan_review"] = json!({"reviewer_config": "a".repeat(64), "max_rounds": 1});
            }),
            ("plan_review on judge role", judge.clone(), |v| {
                v["plan_review"] = json!({"reviewer_config": "a".repeat(64), "max_rounds": 1});
            }),
            ("jail with null proxy allowlist", config.clone(), |v| {
                v["permissions"]["proxy_allowlist_sha256"] = Value::Null
            }),
            ("jail with null rota commit", config.clone(), |v| {
                v["tools"]["rota_commit"] = Value::Null
            }),
            ("jail with null rota backend", config.clone(), |v| {
                v["tools"]["rota_backend_sha256"] = Value::Null
            }),
            ("jail with null jail profile", config.clone(), |v| {
                v["tools"]["jail_profile_sha256"] = Value::Null
            }),
            ("clone with non-null proxy allowlist", clone.clone(), |v| {
                v["permissions"]["proxy_allowlist_sha256"] = json!("0".repeat(64))
            }),
            ("clone with non-null rota commit", clone.clone(), |v| {
                v["tools"]["rota_commit"] = json!("2".repeat(40))
            }),
            ("clone with non-null rota backend", clone.clone(), |v| {
                v["tools"]["rota_backend_sha256"] = json!("1".repeat(64))
            }),
            ("clone with non-null jail profile", clone.clone(), |v| {
                v["tools"]["jail_profile_sha256"] = json!("4".repeat(64))
            }),
            ("v1 with containment", v1.clone(), |v| {
                v["containment"] = json!("jail")
            }),
            ("v1 with null containment", v1.clone(), |v| {
                v["containment"] = Value::Null
            }),
            ("v1 with plan_review", v1.clone(), |v| {
                v["plan_review"] = json!("none")
            }),
            ("v1 with null plan_review", v1.clone(), |v| {
                v["plan_review"] = Value::Null
            }),
            ("v1 with null jail profile", v1.clone(), |v| {
                v["tools"]["jail_profile_sha256"] = Value::Null
            }),
            (
                "null max_turns is fine but zero is not",
                config.clone(),
                |v| v["limits"]["max_turns"] = json!(0),
            ),
            ("zero multiplier", config.clone(), |v| {
                v["limits"]["wall_cap_multiplier"] = json!(0)
            }),
            ("long multiplier", config.clone(), |v| {
                v["limits"]["wall_cap_multiplier"] = json!(1.2345)
            }),
            ("enabled RTK without version", config.clone(), |v| {
                v["tools"]["rtk"] = json!({"enabled": true, "version": null})
            }),
            ("disabled RTK with version", config.clone(), |v| {
                v["tools"]["rtk"] = json!({"enabled": false, "version": "1.0.0"})
            }),
            ("builder with rubric", config.clone(), |v| {
                v["rubric_sha256"] = json!("0".repeat(64))
            }),
            ("builder with null rubric", config.clone(), |v| {
                v["rubric_sha256"] = Value::Null
            }),
            ("judge with null rubric", config.clone(), |v| {
                v["role"] = json!("judge");
                v["rubric_sha256"] = Value::Null;
            }),
            ("judge without rubric", config.clone(), |v| {
                v["role"] = json!("judge")
            }),
            ("task with judge role", task.clone(), |v| {
                v["role"] = json!("judge")
            }),
            ("bad timestamp", task.clone(), |v| {
                v["landed_at"] = json!("2026-11-19")
            }),
            ("empty scorer command", task.clone(), |v| {
                v["scorer"]["command"] = json!([])
            }),
            ("empty test command", task.clone(), |v| {
                v["test"]["command"] = json!([])
            }),
            ("empty test build", task.clone(), |v| {
                v["test"]["build"] = json!([])
            }),
            ("old lockfile shape", task.clone(), |v| {
                v["env"]["lockfiles"] = json!([{"path": "Cargo.lock", "sha256": "0".repeat(64)}])
            }),
            ("zero wall cap", task.clone(), |v| {
                v["wall_cap_s"] = json!(0)
            }),
            ("negative wall cap", task.clone(), |v| {
                v["wall_cap_s"] = json!(-1)
            }),
            ("unsafe integer", task.clone(), |v| {
                v["wall_cap_s"] = json!(1u64 << 53)
            }),
            ("null addendum", task.clone(), |v| {
                v["task_text"]["addendum_sha256"] = Value::Null
            }),
            ("not an object", json!([]), |_| {}),
            ("positional array for config.bench", config.clone(), |v| {
                v["bench"] = json!(["0.1.0", "1".repeat(40)])
            }),
            ("positional array for config.limits", config.clone(), |v| {
                v["limits"] = json!([1.5, 200])
            }),
            (
                "positional array for config.tools.rtk",
                config.clone(),
                |v| v["tools"]["rtk"] = json!([true, "0.9.0"]),
            ),
            (
                "positional array for config.prompt.fleet_rules[0]",
                config.clone(),
                |v| v["prompt"]["fleet_rules"][0] = json!(["common-rules", "c".repeat(64)]),
            ),
            ("positional array for task.scorer", task.clone(), |v| {
                v["scorer"] = json!(["f".repeat(40), ["agent-bench", "score"]])
            }),
        ];
        for (name, mut record, edit) in cases {
            edit(&mut record);
            assert!(hash_json(&record).is_err(), "{name} was accepted");
        }
        // Absent and null differ: max_turns may be null, but must be present.
        let mut unlimited = config.clone();
        unlimited["limits"]["max_turns"] = Value::Null;
        assert!(hash_json(&unlimited).is_ok());

        // Enabled plan_review is accepted on both builder and planner configurations.
        let reviewed = config_with_plan_review();
        assert!(hash_json(&reviewed).is_ok());
        let mut planner_reviewed = reviewed;
        planner_reviewed["role"] = json!("planner");
        assert!(hash_json(&planner_reviewed).is_ok());
    }

    #[test]
    fn wrapper_change_keeps_the_task_hash() {
        let task = hash(TASK.as_bytes()).unwrap();
        let first = hash(CONFIG.as_bytes()).unwrap();
        let mut wrapped = value(CONFIG);
        wrapped["prompt"]["role_wrapper_sha256"] = json!("e".repeat(64));
        let second = hash_json(&wrapped).unwrap();
        assert_ne!(first.hash, second.hash);
        // The task record has no prompt field, so no wrapper can reach its hash.
        assert!(!task.canonical.contains("wrapper"));
        assert!(!task.canonical.contains("\"prompt\""));
        assert_eq!(hash(TASK.as_bytes()).unwrap(), task);
        // Serialising the parsed task drops nothing: a wrapper field is refused, not ignored.
        let mut smuggled = value(TASK);
        smuggled["role_wrapper_sha256"] = json!("e".repeat(64));
        assert!(hash_json(&smuggled).is_err());
    }
}
