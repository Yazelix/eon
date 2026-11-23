//! Bench v2 trial (`eon-bench-trial/1`) and local lanes (`eon-bench-lanes/1`) files, plus
//! planned-run expansion (`agent-bench trial check`, outputting `eon-bench-trial-plan/1`).
//!
//! Both files are strict JSON: duplicate member names are refused via
//! [`crate::jcs::reject_duplicate_names`], and every struct denies unknown fields. Neither file
//! owns `wall_cap_s`, `wall_cap_multiplier`, or `env.pass`/`env.exclude`; those remain owned by
//! [`crate::record::Task`] and [`crate::record::Config`]. No lanes field defaults to a machine
//! path or account name.

use crate::record::{
    self, CONFIG_SCHEMA, CONFIG_SCHEMA_V1, Config, Containment, Kind, PlanReview, Record, Role,
    TASK_SCHEMA, Task, TaskRole, Text, validate_plan_reviewer,
};
use crate::store::Store;
use crate::{
    Result, check_non_negative_finite, check_positive_finite, fail, is_lower_hex, jcs, short,
};
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub const TRIAL_SCHEMA: &str = "eon-bench-trial/1";
pub const LANES_SCHEMA: &str = "eon-bench-lanes/1";
pub const TRIAL_PLAN_SCHEMA: &str = "eon-bench-trial-plan/1";

/// Hard cap on the number of initial planned runs expanded by `agent-bench trial check`,
/// preventing overflow or unbounded memory allocation from an unreasonable `repeats.start`.
/// Fixed rather than configurable because it is a validator resource-exhaustion guard, not a
/// user-facing bench parameter.
pub const MAX_PLANNED_RUNS: u64 = 100_000;

/// Hard cap on the estimated expanded size of `planned_runs` (in bytes) checked before
/// allocation, bounding memory and stdout even when arm, lane, pool, or task strings are large.
/// Fixed rather than configurable for the same validator-safety reason as [`MAX_PLANNED_RUNS`].
pub const MAX_PLANNED_BYTES: u64 = 64 * 1024 * 1024;

const PLANNED_RUN_FIXED_OVERHEAD: u64 = 256;
const PLANNED_REVIEW_FIXED_OVERHEAD: u64 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrialRole {
    Builder,
    Reviewer,
}

impl TrialRole {
    fn matches_config(self, role: Role) -> bool {
        matches!(
            (self, role),
            (Self::Builder, Role::Builder) | (Self::Reviewer, Role::Reviewer)
        )
    }

    fn matches_task(self, role: TaskRole) -> bool {
        matches!(
            (self, role),
            (Self::Builder, TaskRole::Builder) | (Self::Reviewer, TaskRole::Reviewer)
        )
    }
}

impl fmt::Display for TrialRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builder => f.write_str("builder"),
            Self::Reviewer => f.write_str("reviewer"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeats {
    pub start: u64,
    pub cap: u64,
}

/// An inline record JSON object or a lowercase hex hash/prefix resolved through `--store`.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordRef {
    Inline(Value),
    Hash(String),
}

impl Serialize for RecordRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Inline(value) => value.serialize(serializer),
            Self::Hash(hash) => serializer.serialize_str(hash),
        }
    }
}

impl<'de> Deserialize<'de> for RecordRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct RecordRefVisitor;

        impl<'de> Visitor<'de> for RecordRefVisitor {
            type Value = RecordRef;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a record JSON object or a lowercase hex hash or prefix")
            }

            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<RecordRef, E> {
                if value.is_empty() || !is_lower_hex(value) {
                    return Err(E::custom(format!(
                        "{value:?} is not a lowercase hex hash or prefix"
                    )));
                }
                Ok(RecordRef::Hash(value.to_owned()))
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<RecordRef, A::Error> {
                let object =
                    serde_json::Map::<String, Value>::deserialize(MapAccessDeserializer::new(map))?;
                Ok(RecordRef::Inline(Value::Object(object)))
            }
        }

        deserializer.deserialize_any(RecordRefVisitor)
    }
}

fn wrap_de_error<E: serde::de::Error>(prefix: impl fmt::Display, error: E) -> E {
    let text = error.to_string();
    let base = jcs::strip_json_location(&text);
    E::custom(format!("{prefix}: {base}"))
}

fn deserialize_present<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn deserialize_arms_seq<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<TrialArm>, D::Error> {
    struct ArmsVisitor;

    impl<'de> Visitor<'de> for ArmsVisitor {
        type Value = Vec<TrialArm>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an array of trial arm objects")
        }

        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut arms = Vec::new();
            let mut index = 0usize;
            while let Some(arm) = seq
                .next_element::<TrialArm>()
                .map_err(|error| wrap_de_error(format_args!("arms[{index}]"), error))?
            {
                arms.push(arm);
                index += 1;
            }
            Ok(arms)
        }
    }

    deserializer.deserialize_seq(ArmsVisitor)
}

fn deserialize_usage_read<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<UsageRead, D::Error> {
    UsageRead::deserialize(deserializer).map_err(|error| wrap_de_error("usage_read", error))
}

fn deserialize_live_runs_counter<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<LiveRunsCounter, D::Error> {
    LiveRunsCounter::deserialize(deserializer)
        .map_err(|error| wrap_de_error("live_runs_counter", error))
}

fn deserialize_opt_live_runs_counter<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<LiveRunsCounter>, D::Error> {
    Option::<LiveRunsCounter>::deserialize(deserializer)
        .map_err(|error| wrap_de_error("pool_live_runs_counter", error))
}

fn deserialize_opt_machine<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<MachineLimits>, D::Error> {
    Option::<MachineLimits>::deserialize(deserializer)
        .map_err(|error| wrap_de_error("machine", error))
}

fn deserialize_lanes_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<Text, Lane>, D::Error> {
    struct LanesMapVisitor;

    impl<'de> Visitor<'de> for LanesMapVisitor {
        type Value = BTreeMap<Text, Lane>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map from lane name to lane object")
        }

        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut lanes = BTreeMap::new();
            while let Some(name) = map.next_key::<Text>()? {
                let lane = map
                    .next_value::<Lane>()
                    .map_err(|error| wrap_de_error(format_args!("lanes.{name}"), error))?;
                lanes.insert(name, lane);
            }
            Ok(lanes)
        }
    }

    deserializer.deserialize_map(LanesMapVisitor)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialArm {
    pub name: Text,
    pub lane: Text,
    pub config: RecordRef,
    #[serde(default)]
    pub plan_review_lane: Option<Text>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialFile {
    pub schema: Text,
    pub name: Text,
    pub role: TrialRole,
    pub containment: Containment,
    #[serde(deserialize_with = "deserialize_arms_seq")]
    pub arms: Vec<TrialArm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<RecordRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cases: Option<Vec<RecordRef>>,
    pub repeats: Repeats,
    pub pairing_window_hours: u64,
    pub budget_caps: BTreeMap<Text, BTreeMap<Text, f64>>,
}

/// Wire helper for [`parse_trial`] so `"cases": null` on a builder trial (and `"tasks": null` on
/// a reviewer trial) is distinguished from key absence while streaming directly from bytes.
#[derive(Deserialize)]
#[serde(rename = "TrialFile", deny_unknown_fields)]
struct RawTrialFile {
    schema: Text,
    name: Text,
    role: TrialRole,
    containment: Containment,
    #[serde(deserialize_with = "deserialize_arms_seq")]
    arms: Vec<TrialArm>,
    #[serde(default, deserialize_with = "deserialize_present")]
    tasks: Option<Option<Vec<RecordRef>>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    cases: Option<Option<Vec<RecordRef>>>,
    repeats: Repeats,
    pairing_window_hours: u64,
    budget_caps: BTreeMap<Text, BTreeMap<Text, f64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageParser {
    Claude,
    Codex,
    Agy,
    Json,
    KeyValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRead {
    pub command: Vec<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_slot_command: Option<Vec<Text>>,
    pub parser: UsageParser,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveRunsCounterKind {
    LockFile,
    CapsFile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRunsCounter {
    pub kind: LiveRunsCounterKind,
    pub path: Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineLimits {
    pub max_load_1m: f64,
    pub min_available_mem_mib: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub pool: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Text>,
    pub launch_template: Vec<Text>,
    #[serde(deserialize_with = "deserialize_usage_read")]
    pub usage_read: UsageRead,
    pub floors: BTreeMap<Text, f64>,
    pub run_cap: u64,
    #[serde(deserialize_with = "deserialize_live_runs_counter")]
    pub live_runs_counter: LiveRunsCounter,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_opt_live_runs_counter"
    )]
    pub pool_live_runs_counter: Option<LiveRunsCounter>,
    pub stagger_s: u64,
    pub heavy_queue_command: Vec<Text>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub context_handoff_tokens: Option<u64>,
    pub extra_env: Vec<Text>,
    pub approved_containment: Vec<Containment>,
    #[serde(default)]
    pub home_settings_files: Vec<Text>,
    #[serde(default)]
    pub audit_dirs: Vec<Text>,
}

impl Lane {
    /// Account key used for stagger enforcement (explicit `account`, falling back to `lane_name`).
    pub fn account_key<'a>(&'a self, lane_name: &'a str) -> &'a str {
        self.account.as_ref().map_or(lane_name, Text::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanesFile {
    pub schema: Text,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_opt_machine"
    )]
    pub machine: Option<MachineLimits>,
    #[serde(deserialize_with = "deserialize_lanes_map")]
    pub lanes: BTreeMap<Text, Lane>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPlanReview {
    pub reviewer_config_hash: String,
    pub lane: String,
    pub pool: String,
    pub max_rounds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedRun {
    pub arm: String,
    pub lane: String,
    pub pool: String,
    pub config_hash: String,
    pub task_id: String,
    pub task_hash: String,
    pub repeat: u64,
    pub effective_wall_cap_s: u64,
    pub plan_review: Option<PlannedPlanReview>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialPlan {
    pub schema: String,
    pub trial: String,
    pub role: TrialRole,
    pub containment: Containment,
    pub repeats: Repeats,
    pub pairing_window_hours: u64,
    pub initial_runs: u64,
    pub max_runs: u64,
    pub planned_runs: Vec<PlannedRun>,
}

/// Parses and validates a trial JSON file (`eon-bench-trial/1`).
pub fn parse_trial(input: &[u8]) -> Result<TrialFile> {
    let raw: RawTrialFile = jcs::from_slice_objects(input)?;
    match raw.role {
        TrialRole::Builder if raw.cases.is_some() => {
            fail!("cases: only a reviewer trial may specify `cases`");
        }
        TrialRole::Reviewer if raw.tasks.is_some() => {
            fail!("tasks: only a builder trial may specify `tasks`");
        }
        _ => {}
    }
    let trial = TrialFile {
        schema: raw.schema,
        name: raw.name,
        role: raw.role,
        containment: raw.containment,
        arms: raw.arms,
        tasks: raw.tasks.flatten(),
        cases: raw.cases.flatten(),
        repeats: raw.repeats,
        pairing_window_hours: raw.pairing_window_hours,
        budget_caps: raw.budget_caps,
    };
    validate_trial(&trial)?;
    Ok(trial)
}

/// Parses and validates a local lanes JSON file (`eon-bench-lanes/1`).
pub fn parse_lanes(input: &[u8]) -> Result<LanesFile> {
    let lanes: LanesFile = jcs::from_slice_objects(input)?;
    validate_lanes(&lanes)?;
    Ok(lanes)
}

/// Parses, validates, resolves and expands a trial and lanes pair into planned runs.
pub fn check(trial_bytes: &[u8], lanes_bytes: &[u8], store: Option<&Store>) -> Result<TrialPlan> {
    let trial = parse_trial(trial_bytes)?;
    let lanes = parse_lanes(lanes_bytes)?;
    plan_checked(&trial, &lanes, store)
}

/// Resolves all records and lanes for a trial and expands its initial planned runs.
pub fn plan(trial: &TrialFile, lanes: &LanesFile, store: Option<&Store>) -> Result<TrialPlan> {
    validate_trial(trial)?;
    validate_lanes(lanes)?;
    plan_checked(trial, lanes, store)
}

/// Expands already-validated trial and lanes inputs without re-running their validators.
pub(crate) fn plan_checked(
    trial: &TrialFile,
    lanes: &LanesFile,
    store: Option<&Store>,
) -> Result<TrialPlan> {
    let mut seen_arms = HashSet::new();
    let mut seen_configs = HashSet::new();
    let mut resolved_arms = Vec::with_capacity(trial.arms.len());

    for arm in &trial.arms {
        if !seen_arms.insert(arm.name.as_str()) {
            fail!("duplicate arm name {:?}", arm.name.as_str());
        }
        let lane = resolve_lane(
            lanes,
            trial,
            &arm.lane,
            &format!("arm {:?}", arm.name.as_str()),
        )?;
        let (config, config_hash) = resolve_arm_config(arm, store)?;
        if !trial.role.matches_config(config.role) {
            fail!(
                "arm {:?} config {} has role {:?}, expected {}",
                arm.name.as_str(),
                short(&config_hash),
                config.role,
                trial.role
            );
        }
        if config.containment != Some(trial.containment) {
            fail!(
                "arm {:?} config {} has containment {:?}, expected {}",
                arm.name.as_str(),
                short(&config_hash),
                config.containment,
                trial.containment
            );
        }
        if !seen_configs.insert(config_hash.clone()) {
            fail!(
                "duplicate config hash {} across trial arms",
                short(&config_hash)
            );
        }
        let plan_review = match (&config.plan_review, &arm.plan_review_lane) {
            (Some(PlanReview::None), None) => None,
            (Some(PlanReview::None), Some(pr_lane)) => {
                fail!(
                    "arm {:?} sets plan_review_lane {:?} while config {} has plan_review \"none\"",
                    arm.name.as_str(),
                    pr_lane.as_str(),
                    short(&config_hash)
                );
            }
            (Some(PlanReview::Enabled { .. }), None) => {
                fail!(
                    "arm {:?} enables plan_review and requires `plan_review_lane`",
                    arm.name.as_str()
                );
            }
            (
                Some(PlanReview::Enabled {
                    reviewer_config,
                    max_rounds,
                }),
                Some(pr_lane_name),
            ) => {
                let pr_lane = resolve_lane(
                    lanes,
                    trial,
                    pr_lane_name,
                    &format!("arm {:?} plan_review_lane", arm.name.as_str()),
                )?;
                let Some(store) = store else {
                    fail!(
                        "arm {:?} enables plan_review with reviewer_config {}, which requires --store DIR",
                        arm.name.as_str(),
                        short(reviewer_config.as_str())
                    );
                };
                let reviewer = store.config(reviewer_config.as_str()).map_err(|error| {
                    crate::Error(format!(
                        "arm {:?} plan_review.reviewer_config: {}",
                        arm.name.as_str(),
                        error.0
                    ))
                })?;
                validate_plan_reviewer(&reviewer, reviewer_config.as_str(), trial.containment)
                    .map_err(|error| {
                        crate::Error(format!("arm {:?}: {}", arm.name.as_str(), error.0))
                    })?;
                Some(PlannedPlanReview {
                    reviewer_config_hash: reviewer_config.as_str().to_owned(),
                    lane: pr_lane_name.as_str().to_owned(),
                    pool: pr_lane.pool.as_str().to_owned(),
                    max_rounds: *max_rounds,
                })
            }
            (None, _) => {
                fail!(
                    "arm {:?} config {} is missing plan_review",
                    arm.name.as_str(),
                    short(&config_hash)
                );
            }
        };
        resolved_arms.push(ResolvedArm {
            name: arm.name.as_str().to_owned(),
            lane: arm.lane.as_str().to_owned(),
            pool: lane.pool.as_str().to_owned(),
            config_hash,
            config,
            plan_review,
        });
    }

    let (item_label, refs) =
        match trial.role {
            TrialRole::Builder => (
                "task",
                trial.tasks.as_deref().ok_or_else(|| {
                    crate::Error("tasks: a builder trial requires `tasks`".into())
                })?,
            ),
            TrialRole::Reviewer => (
                "case",
                trial.cases.as_deref().ok_or_else(|| {
                    crate::Error("cases: a reviewer trial requires `cases`".into())
                })?,
            ),
        };

    let mut seen_task_hashes = HashSet::new();
    let mut seen_task_ids = HashSet::new();
    let mut resolved_tasks = Vec::with_capacity(refs.len());

    for (index, item) in refs.iter().enumerate() {
        let (task, task_hash) = resolve_task(item, item_label, index, store)?;
        if !trial.role.matches_task(task.role) {
            fail!(
                "{item_label}[{index}] ({}) has role {:?}, expected {}",
                task.task_id,
                task.role,
                trial.role
            );
        }
        if !seen_task_hashes.insert(task_hash.clone()) {
            fail!("duplicate {item_label} hash {} in trial", short(&task_hash));
        }
        if !seen_task_ids.insert(task.task_id.as_str().to_owned()) {
            fail!("duplicate task_id {:?} in trial", task.task_id.as_str());
        }
        resolved_tasks.push(ResolvedTask { task_hash, task });
    }

    let (initial_runs, max_runs) =
        compute_run_counts(resolved_arms.len(), resolved_tasks.len(), trial.repeats)?;

    let mut bytes_per_repeat: u64 = 0;
    for task in &resolved_tasks {
        for arm in &resolved_arms {
            let per_run = estimated_planned_run_bytes(arm, task)?;
            bytes_per_repeat = bytes_per_repeat
                .checked_add(per_run)
                .ok_or_else(|| crate::Error("planned output byte estimate overflows u64".into()))?;
        }
    }
    let estimated_bytes = bytes_per_repeat
        .checked_mul(trial.repeats.start)
        .ok_or_else(|| crate::Error("planned output byte estimate overflows u64".into()))?;
    if estimated_bytes > MAX_PLANNED_BYTES {
        fail!(
            "estimated planned output size ({estimated_bytes} bytes) exceeds hard cap of {MAX_PLANNED_BYTES} bytes"
        );
    }

    let mut wall_caps = Vec::with_capacity(resolved_tasks.len());
    for task in &resolved_tasks {
        let mut row = Vec::with_capacity(resolved_arms.len());
        for arm in &resolved_arms {
            row.push(effective_wall_cap_s(
                task.task.wall_cap_s,
                arm.config.limits.wall_cap_multiplier,
            )?);
        }
        wall_caps.push(row);
    }

    let capacity = usize::try_from(initial_runs).map_err(|_| {
        crate::Error(format!(
            "initial_runs {initial_runs} exceeds platform usize"
        ))
    })?;
    let mut planned_runs = Vec::with_capacity(capacity);
    for repeat in 1..=trial.repeats.start {
        for (task_index, task) in resolved_tasks.iter().enumerate() {
            for (arm_index, arm) in resolved_arms.iter().enumerate() {
                planned_runs.push(PlannedRun {
                    arm: arm.name.clone(),
                    lane: arm.lane.clone(),
                    pool: arm.pool.clone(),
                    config_hash: arm.config_hash.clone(),
                    task_id: task.task.task_id.as_str().to_owned(),
                    task_hash: task.task_hash.clone(),
                    repeat,
                    effective_wall_cap_s: wall_caps[task_index][arm_index],
                    plan_review: arm.plan_review.clone(),
                });
            }
        }
    }

    Ok(TrialPlan {
        schema: TRIAL_PLAN_SCHEMA.to_owned(),
        trial: trial.name.as_str().to_owned(),
        role: trial.role,
        containment: trial.containment,
        repeats: trial.repeats,
        pairing_window_hours: trial.pairing_window_hours,
        initial_runs,
        max_runs,
        planned_runs,
    })
}

struct ResolvedArm {
    name: String,
    lane: String,
    pool: String,
    config_hash: String,
    config: Config,
    plan_review: Option<PlannedPlanReview>,
}

struct ResolvedTask {
    task_hash: String,
    task: Task,
}

fn validate_trial(trial: &TrialFile) -> Result<()> {
    if trial.schema.as_str() != TRIAL_SCHEMA {
        fail!("schema {} is not {TRIAL_SCHEMA}", trial.schema);
    }
    if trial.arms.is_empty() {
        fail!("arms: a trial must define at least one arm");
    }
    match trial.role {
        TrialRole::Builder => {
            if trial.cases.is_some() {
                fail!("cases: only a reviewer trial may specify `cases`");
            }
            match &trial.tasks {
                Some(tasks) if !tasks.is_empty() => {}
                Some(_) => fail!("tasks: a builder trial must list at least one task"),
                None => fail!("tasks: a builder trial requires `tasks`"),
            }
        }
        TrialRole::Reviewer => {
            if trial.tasks.is_some() {
                fail!("tasks: only a builder trial may specify `tasks`");
            }
            match &trial.cases {
                Some(cases) if !cases.is_empty() => {}
                Some(_) => fail!("cases: a reviewer trial must list at least one case"),
                None => fail!("cases: a reviewer trial requires `cases`"),
            }
        }
    }
    if trial.repeats.start == 0 {
        fail!("repeats.start must be at least 1");
    }
    if trial.repeats.start > jcs::MAX_EXACT {
        fail!("repeats.start exceeds exact integer range (2^53 - 1)");
    }
    if trial.repeats.cap < trial.repeats.start {
        fail!(
            "repeats.cap ({}) must be at least repeats.start ({})",
            trial.repeats.cap,
            trial.repeats.start
        );
    }
    if trial.repeats.cap > jcs::MAX_EXACT {
        fail!("repeats.cap exceeds exact integer range (2^53 - 1)");
    }
    if trial.pairing_window_hours == 0 {
        fail!("pairing_window_hours must be at least 1");
    }
    if trial.pairing_window_hours > jcs::MAX_EXACT {
        fail!("pairing_window_hours exceeds exact integer range (2^53 - 1)");
    }
    if trial.budget_caps.is_empty() {
        fail!("budget_caps must name at least one pool");
    }
    for (pool, meters) in &trial.budget_caps {
        if meters.is_empty() {
            fail!("budget_caps.{pool} must name at least one meter");
        }
        for (meter, cap) in meters {
            check_positive_finite(*cap, &format!("budget_caps.{pool}.{meter}"))?;
        }
    }
    Ok(())
}

pub(crate) fn validate_lanes(lanes: &LanesFile) -> Result<()> {
    if lanes.schema.as_str() != LANES_SCHEMA {
        fail!("schema {} is not {LANES_SCHEMA}", lanes.schema);
    }
    if let Some(machine) = &lanes.machine {
        check_positive_finite(machine.max_load_1m, "machine.max_load_1m")?;
        if machine.min_available_mem_mib == 0 {
            fail!("machine.min_available_mem_mib must be at least 1");
        }
        if machine.min_available_mem_mib > jcs::MAX_EXACT {
            fail!("machine.min_available_mem_mib exceeds exact integer range (2^53 - 1)");
        }
    }
    if lanes.lanes.is_empty() {
        fail!("lanes: must define at least one lane");
    }
    for (name, lane) in &lanes.lanes {
        validate_lane(name.as_str(), lane)?;
    }
    Ok(())
}

fn validate_lane(name: &str, lane: &Lane) -> Result<()> {
    if let Some(account) = &lane.account {
        check_no_nul(account, &format!("lanes.{name}.account"))?;
    }
    if lane.launch_template.is_empty() {
        fail!("lanes.{name}.launch_template needs at least the program");
    }
    check_no_nul_list(
        &lane.launch_template,
        &format!("lanes.{name}.launch_template"),
    )?;
    if lane.usage_read.command.is_empty() {
        fail!("lanes.{name}.usage_read.command needs at least the program");
    }
    check_no_nul_list(
        &lane.usage_read.command,
        &format!("lanes.{name}.usage_read.command"),
    )?;
    if let Some(in_slot) = &lane.usage_read.in_slot_command {
        if in_slot.is_empty() {
            fail!("lanes.{name}.usage_read.in_slot_command needs at least the program");
        }
        check_no_nul_list(in_slot, &format!("lanes.{name}.usage_read.in_slot_command"))?;
    }
    if let Some(group) = &lane.usage_read.group {
        if lane.usage_read.parser != UsageParser::Agy {
            fail!("lanes.{name}.usage_read.group is only valid with the `agy` parser");
        }
        if group
            .as_str()
            .bytes()
            .any(|b| matches!(b, b'\0' | b'\n' | b'\r' | b'\t'))
        {
            fail!("lanes.{name}.usage_read.group must not contain NUL, tab, or newline characters");
        }
    }
    for (meter, floor) in &lane.floors {
        check_non_negative_finite(*floor, &format!("lanes.{name}.floors.{meter}"))?;
    }
    if lane.run_cap == 0 {
        fail!("lanes.{name}.run_cap must be at least 1");
    }
    if lane.run_cap > jcs::MAX_EXACT {
        fail!("lanes.{name}.run_cap exceeds exact integer range (2^53 - 1)");
    }
    check_no_nul(
        &lane.live_runs_counter.path,
        &format!("lanes.{name}.live_runs_counter.path"),
    )?;
    if let Some(pool_counter) = &lane.pool_live_runs_counter {
        check_no_nul(
            &pool_counter.path,
            &format!("lanes.{name}.pool_live_runs_counter.path"),
        )?;
    }
    if lane.stagger_s > jcs::MAX_EXACT {
        fail!("lanes.{name}.stagger_s exceeds exact integer range (2^53 - 1)");
    }
    if lane.heavy_queue_command.is_empty() {
        fail!("lanes.{name}.heavy_queue_command needs at least the program");
    }
    check_no_nul_list(
        &lane.heavy_queue_command,
        &format!("lanes.{name}.heavy_queue_command"),
    )?;
    if let Some(tokens) = lane.context_handoff_tokens {
        if tokens == 0 {
            fail!("lanes.{name}.context_handoff_tokens must be at least 1 or null");
        }
        if tokens > jcs::MAX_EXACT {
            fail!("lanes.{name}.context_handoff_tokens exceeds exact integer range (2^53 - 1)");
        }
    }
    let mut seen_env = HashSet::new();
    for env_name in &lane.extra_env {
        if !is_env_name(env_name.as_str()) {
            fail!(
                "lanes.{name}.extra_env entry {:?} is not an environment variable name",
                env_name.as_str()
            );
        }
        if !seen_env.insert(env_name.as_str()) {
            fail!("lanes.{name}.extra_env duplicates {:?}", env_name.as_str());
        }
    }
    if lane.approved_containment.is_empty() {
        fail!("lanes.{name}.approved_containment must list at least one containment level");
    }
    let mut seen_containment = HashSet::new();
    for &level in &lane.approved_containment {
        if !seen_containment.insert(level) {
            fail!("lanes.{name}.approved_containment duplicates {level}");
        }
    }
    check_no_nul_list(
        &lane.home_settings_files,
        &format!("lanes.{name}.home_settings_files"),
    )?;
    check_no_nul_list(&lane.audit_dirs, &format!("lanes.{name}.audit_dirs"))?;
    Ok(())
}

fn check_no_nul(text: &Text, field: &str) -> Result<()> {
    if text.as_str().contains('\0') {
        fail!("{field} must not contain NUL bytes");
    }
    Ok(())
}

fn check_no_nul_list(items: &[Text], field: &str) -> Result<()> {
    for (index, item) in items.iter().enumerate() {
        check_no_nul(item, &format!("{field}[{index}]"))?;
    }
    Ok(())
}

fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn resolve_lane<'a>(
    lanes: &'a LanesFile,
    trial: &TrialFile,
    lane_name: &Text,
    context: &str,
) -> Result<&'a Lane> {
    let Some(lane) = lanes.lanes.get(lane_name) else {
        fail!("{context} references unknown lane {:?}", lane_name.as_str());
    };
    if !lane.approved_containment.contains(&trial.containment) {
        fail!(
            "{context} uses lane {:?}, which is not approved for {} containment",
            lane_name.as_str(),
            trial.containment
        );
    }
    if !trial.budget_caps.contains_key(&lane.pool) {
        fail!(
            "{context} uses lane {:?} on pool {:?}, which has no entry in budget_caps",
            lane_name.as_str(),
            lane.pool.as_str()
        );
    }
    Ok(lane)
}

fn resolve_arm_config(arm: &TrialArm, store: Option<&Store>) -> Result<(Config, String)> {
    let hashed = match &arm.config {
        RecordRef::Inline(value) => {
            if value.get("schema").and_then(Value::as_str) == Some(CONFIG_SCHEMA_V1) {
                fail!(
                    "arm {:?} uses {CONFIG_SCHEMA_V1}; trials require {CONFIG_SCHEMA}",
                    arm.name.as_str()
                );
            }
            record::hash_value(value.clone())
                .map_err(|error| crate::Error(format!("arm {:?} config: {}", arm.name, error.0)))?
        }
        RecordRef::Hash(prefix) => {
            let Some(store) = store else {
                fail!(
                    "arm {:?} references config hash {prefix:?}, which requires --store DIR",
                    arm.name.as_str()
                );
            };
            let (_, hash) = store
                .resolve(&[Kind::Config], prefix)
                .map_err(|error| crate::Error(format!("arm {:?} config: {}", arm.name, error.0)))?;
            store.get(Kind::Config, &hash)?
        }
    };
    match hashed.record {
        Record::Config(config) => {
            if config.is_v1() {
                fail!(
                    "arm {:?} references {CONFIG_SCHEMA_V1} record {}; trials require {CONFIG_SCHEMA}",
                    arm.name.as_str(),
                    short(&hashed.hash)
                );
            }
            Ok((config, hashed.hash))
        }
        Record::Task(_) => {
            fail!(
                "arm {:?} config must be a {CONFIG_SCHEMA} record, not {TASK_SCHEMA}",
                arm.name.as_str()
            );
        }
    }
}

fn resolve_task(
    item: &RecordRef,
    label: &str,
    index: usize,
    store: Option<&Store>,
) -> Result<(Task, String)> {
    let hashed = match item {
        RecordRef::Inline(value) => record::hash_value(value.clone())
            .map_err(|error| crate::Error(format!("{label}[{index}]: {}", error.0)))?,
        RecordRef::Hash(prefix) => {
            let Some(store) = store else {
                fail!("{label}[{index}] references hash {prefix:?}, which requires --store DIR");
            };
            let (_, hash) = store
                .resolve(&[Kind::Task], prefix)
                .map_err(|error| crate::Error(format!("{label}[{index}]: {}", error.0)))?;
            store.get(Kind::Task, &hash)?
        }
    };
    match hashed.record {
        Record::Task(task) => Ok((task, hashed.hash)),
        Record::Config(_) => {
            fail!("{label}[{index}] must be a {TASK_SCHEMA} record, not a configuration record");
        }
    }
}

fn estimated_planned_run_bytes(arm: &ResolvedArm, task: &ResolvedTask) -> Result<u64> {
    let mut bytes = PLANNED_RUN_FIXED_OVERHEAD;
    for len in [
        arm.name.len(),
        arm.lane.len(),
        arm.pool.len(),
        arm.config_hash.len(),
        task.task.task_id.as_str().len(),
        task.task_hash.len(),
    ] {
        let len = u64::try_from(len)
            .map_err(|_| crate::Error("planned run string length exceeds u64".into()))?;
        bytes = bytes
            .checked_add(len)
            .ok_or_else(|| crate::Error("planned output byte estimate overflows u64".into()))?;
    }
    if let Some(pr) = &arm.plan_review {
        bytes = bytes
            .checked_add(PLANNED_REVIEW_FIXED_OVERHEAD)
            .ok_or_else(|| crate::Error("planned output byte estimate overflows u64".into()))?;
        for len in [pr.reviewer_config_hash.len(), pr.lane.len(), pr.pool.len()] {
            let len = u64::try_from(len)
                .map_err(|_| crate::Error("planned run string length exceeds u64".into()))?;
            bytes = bytes
                .checked_add(len)
                .ok_or_else(|| crate::Error("planned output byte estimate overflows u64".into()))?;
        }
    }
    Ok(bytes)
}

fn compute_run_counts(arms: usize, tasks: usize, repeats: Repeats) -> Result<(u64, u64)> {
    let arm_count =
        u64::try_from(arms).map_err(|_| crate::Error("arm count exceeds u64".into()))?;
    let task_count =
        u64::try_from(tasks).map_err(|_| crate::Error("task count exceeds u64".into()))?;
    let per_repeat = arm_count
        .checked_mul(task_count)
        .ok_or_else(|| crate::Error("arms * tasks overflows u64".into()))?;
    let initial_runs = per_repeat.checked_mul(repeats.start).ok_or_else(|| {
        crate::Error("initial_runs (arms * tasks * repeats.start) overflows u64".into())
    })?;
    if initial_runs > MAX_PLANNED_RUNS {
        fail!("initial_runs ({initial_runs}) exceeds hard cap of {MAX_PLANNED_RUNS} planned runs");
    }
    let max_runs = per_repeat.checked_mul(repeats.cap).ok_or_else(|| {
        crate::Error("max_runs (arms * tasks * repeats.cap) overflows u64".into())
    })?;
    if max_runs > jcs::MAX_EXACT {
        fail!("max_runs ({max_runs}) exceeds exact integer range (2^53 - 1)");
    }
    Ok((initial_runs, max_runs))
}

/// Computes `ceil(wall_cap_s * multiplier)` exactly in `u128` milli-units and converts to `u64`,
/// refusing on `u64` overflow or values outside I-JSON's exact integer range (`2^53 - 1`).
fn effective_wall_cap_s(wall_cap_s: u64, multiplier: f64) -> Result<u64> {
    if !multiplier.is_finite() || multiplier <= 0.0 {
        fail!("invalid wall_cap_multiplier {multiplier}");
    }
    let milli: u128 = format!("{multiplier:.3}")
        .replace('.', "")
        .parse()
        .map_err(|_| crate::Error(format!("invalid wall_cap_multiplier {multiplier}")))?;
    if milli == 0 {
        fail!("invalid wall_cap_multiplier {multiplier}");
    }
    let scaled = u128::from(wall_cap_s)
        .checked_mul(milli)
        .ok_or_else(|| crate::Error("effective_wall_cap_s overflows u128".into()))?;
    let effective = scaled.div_ceil(1000);
    let effective = u64::try_from(effective)
        .map_err(|_| crate::Error(format!("effective_wall_cap_s ({effective}) overflows u64")))?;
    if effective > jcs::MAX_EXACT {
        fail!("effective_wall_cap_s ({effective}) exceeds exact integer range (2^53 - 1)");
    }
    Ok(effective)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::tests::{
        CONFIG, CONFIG_CLONE, CONFIG_V1, GOLDEN_CONFIG_CLONE, GOLDEN_TASK, JUDGE, TASK,
    };
    use serde_json::json;

    pub(crate) const TRIAL: &str = include_str!("fixtures/trial.json");
    pub(crate) const LANES: &str = include_str!("fixtures/lanes.json");
    const GOLDEN_TRIAL_ARM_B: &str =
        "b660b59ddb98cf9eb7531cc8ef0ccdf55da272d8ec939f1d873715dad78c27a6";

    fn json_val(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn check_values(trial: &Value, lanes: &Value, store: Option<&Store>) -> Result<TrialPlan> {
        check(
            &serde_json::to_vec(trial).unwrap(),
            &serde_json::to_vec(lanes).unwrap(),
            store,
        )
    }

    #[test]
    fn valid_inline_builder_trial_expands_in_paired_block_order() {
        let plan = check(TRIAL.as_bytes(), LANES.as_bytes(), None).unwrap();
        assert_eq!(plan.schema, TRIAL_PLAN_SCHEMA);
        assert_eq!(plan.trial, "clone-builder-mini");
        assert_eq!(plan.role, TrialRole::Builder);
        assert_eq!(plan.containment, Containment::Clone);
        assert_eq!(plan.repeats, Repeats { start: 2, cap: 4 });
        assert_eq!(plan.pairing_window_hours, 72);
        assert_eq!(plan.initial_runs, 4);
        assert_eq!(plan.max_runs, 8);
        assert_eq!(plan.planned_runs.len(), 4);

        // Paired-block order: repeat 1 (arm-a, arm-b), repeat 2 (arm-a, arm-b).
        let order: Vec<(u64, &str, &str, u64)> = plan
            .planned_runs
            .iter()
            .map(|r| {
                (
                    r.repeat,
                    r.arm.as_str(),
                    r.pool.as_str(),
                    r.effective_wall_cap_s,
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                (1, "arm-a", "gemini-pro", 5400),
                (1, "arm-b", "claude-pro", 3600),
                (2, "arm-a", "gemini-pro", 5400),
                (2, "arm-b", "claude-pro", 3600),
            ]
        );
        let hashes: Vec<&str> = plan
            .planned_runs
            .iter()
            .map(|r| r.config_hash.as_str())
            .collect();
        assert_eq!(
            hashes,
            [
                GOLDEN_CONFIG_CLONE,
                GOLDEN_TRIAL_ARM_B,
                GOLDEN_CONFIG_CLONE,
                GOLDEN_TRIAL_ARM_B,
            ]
        );
        for run in &plan.planned_runs {
            assert_eq!(run.task_id, "eon-7aw48");
            assert_eq!(run.task_hash, GOLDEN_TASK);
            assert_eq!(run.plan_review, None);
        }
    }

    #[test]
    fn trial_file_round_trips_for_both_roles() {
        let builder = parse_trial(TRIAL.as_bytes()).unwrap();
        let builder_bytes = serde_json::to_vec(&builder).unwrap();
        let builder_val: Value = serde_json::from_slice(&builder_bytes).unwrap();
        assert!(builder_val.get("cases").is_none());
        assert_eq!(parse_trial(&builder_bytes).unwrap(), builder);

        let mut rev_cfg = json_val(CONFIG_CLONE);
        rev_cfg["role"] = json!("reviewer");
        let mut rev_case = json_val(TASK);
        rev_case["role"] = json!("reviewer");
        let reviewer_json = serde_json::to_vec(&json!({
            "schema": TRIAL_SCHEMA,
            "name": "reviewer-round-trip",
            "role": "reviewer",
            "containment": "clone",
            "arms": [{
                "name": "rev-a",
                "lane": "lane-a",
                "config": rev_cfg
            }],
            "cases": [rev_case],
            "repeats": {"start": 1, "cap": 2},
            "pairing_window_hours": 72,
            "budget_caps": {"gemini-pro": {"week_pct": 10.0}}
        }))
        .unwrap();
        let reviewer = parse_trial(&reviewer_json).unwrap();
        let reviewer_bytes = serde_json::to_vec(&reviewer).unwrap();
        let reviewer_val: Value = serde_json::from_slice(&reviewer_bytes).unwrap();
        assert!(reviewer_val.get("tasks").is_none());
        assert_eq!(parse_trial(&reviewer_bytes).unwrap(), reviewer);
    }

    #[test]
    fn store_hashes_plan_review_and_reviewer_cases_expand() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());

        // Store a jail reviewer config and a jail builder config that uses it for plan_review.
        let mut rev_cfg = json_val(CONFIG);
        rev_cfg["role"] = json!("reviewer");
        let rev_hash = store
            .put(&serde_json::to_vec(&rev_cfg).unwrap())
            .unwrap()
            .hash;

        let mut builder_pr = json_val(CONFIG);
        builder_pr["plan_review"] = json!({
            "reviewer_config": rev_hash,
            "max_rounds": 3
        });
        let builder_pr_hash = store
            .put(&serde_json::to_vec(&builder_pr).unwrap())
            .unwrap()
            .hash;

        let task_hash = store.put(TASK.as_bytes()).unwrap().hash;

        let trial = json!({
            "schema": TRIAL_SCHEMA,
            "name": "jail-plan-review",
            "role": "builder",
            "containment": "jail",
            "arms": [{
                "name": "reviewed",
                "lane": "lane-a",
                "config": &builder_pr_hash[..12],
                "plan_review_lane": "lane-a"
            }],
            "tasks": [&task_hash[..12]],
            "repeats": {"start": 1, "cap": 2},
            "pairing_window_hours": 72,
            "budget_caps": {"gemini-pro": {"week_pct": 30.0}}
        });
        let lanes = json_val(LANES);
        // Without --store, hash references are refused.
        assert!(check_values(&trial, &lanes, None).is_err());

        let plan = check_values(&trial, &lanes, Some(&store)).unwrap();
        assert_eq!(plan.initial_runs, 1);
        assert_eq!(plan.max_runs, 2);
        let run = &plan.planned_runs[0];
        assert_eq!(run.config_hash, builder_pr_hash);
        assert_eq!(run.task_hash, task_hash);
        assert_eq!(
            run.plan_review,
            Some(PlannedPlanReview {
                reviewer_config_hash: rev_hash.clone(),
                lane: "lane-a".into(),
                pool: "gemini-pro".into(),
                max_rounds: 3,
            })
        );

        // Reviewer trial using `cases` (inline reviewer task) and stored reviewer config hash.
        let mut rev_task = json_val(TASK);
        rev_task["role"] = json!("reviewer");
        rev_task["task_id"] = json!("R20");
        let rev_trial = json!({
            "schema": TRIAL_SCHEMA,
            "name": "reviewer-eval",
            "role": "reviewer",
            "containment": "jail",
            "arms": [{
                "name": "rev-a",
                "lane": "lane-a",
                "config": &rev_hash[..12]
            }],
            "cases": [rev_task],
            "repeats": {"start": 1, "cap": 2},
            "pairing_window_hours": 72,
            "budget_caps": {"gemini-pro": {"week_pct": 10.0}}
        });
        let rev_plan = check_values(&rev_trial, &lanes, Some(&store)).unwrap();
        assert_eq!(rev_plan.role, TrialRole::Reviewer);
        assert_eq!(rev_plan.planned_runs.len(), 1);
        assert_eq!(rev_plan.planned_runs[0].task_id, "R20");
        assert_eq!(rev_plan.planned_runs[0].config_hash, rev_hash);
    }

    #[test]
    fn wall_cap_and_run_count_overflow_and_cap_checks() {
        // Exact milli arithmetic for large wall_cap_s up to 2^53 - 1 (would overflow u64 * milli).
        assert_eq!(effective_wall_cap_s(3600, 1.5).unwrap(), 5400);
        assert_eq!(effective_wall_cap_s(100, 1.234).unwrap(), 124);
        assert_eq!(effective_wall_cap_s(1, 0.001).unwrap(), 1);
        let max_exact = jcs::MAX_EXACT;
        assert_eq!(effective_wall_cap_s(max_exact, 1.0).unwrap(), max_exact);
        assert_eq!(
            effective_wall_cap_s(max_exact / 1000, 1000.0).unwrap(),
            (max_exact / 1000) * 1000
        );
        // Values exceeding 2^53 - 1 or overflowing u64 are refused instead of wrapping or losing precision.
        let err = effective_wall_cap_s(max_exact, 1000.0).unwrap_err().0;
        assert!(err.contains("exact integer range"), "{err}");
        let err = effective_wall_cap_s(u64::MAX / 2, 10.0).unwrap_err().0;
        assert!(err.contains("overflows u64"), "{err}");

        // Planned run cap, exact-range cap, and checked_mul overflow checks.
        assert_eq!(
            compute_run_counts(2, 5, Repeats { start: 2, cap: 4 }).unwrap(),
            (20, 40)
        );
        assert_eq!(
            compute_run_counts(
                1,
                1,
                Repeats {
                    start: MAX_PLANNED_RUNS,
                    cap: MAX_PLANNED_RUNS,
                }
            )
            .unwrap(),
            (MAX_PLANNED_RUNS, MAX_PLANNED_RUNS)
        );
        assert!(
            compute_run_counts(
                1,
                1,
                Repeats {
                    start: MAX_PLANNED_RUNS + 1,
                    cap: MAX_PLANNED_RUNS + 1,
                }
            )
            .is_err()
        );
        assert!(compute_run_counts(usize::MAX, 2, Repeats { start: 1, cap: 1 }).is_err());
        let err = compute_run_counts(
            2,
            1,
            Repeats {
                start: 1,
                cap: jcs::MAX_EXACT,
            },
        )
        .unwrap_err()
        .0;
        assert!(err.contains("exact integer range"), "{err}");
        let err = compute_run_counts(
            2,
            1025,
            Repeats {
                start: 1,
                cap: jcs::MAX_EXACT,
            },
        )
        .unwrap_err()
        .0;
        assert!(err.contains("overflows u64"), "{err}");

        // End-to-end refusal when repeats.start pushes initial_runs over MAX_PLANNED_RUNS.
        let mut huge = json_val(TRIAL);
        huge["repeats"] = json!({"start": 60_000, "cap": 60_000});
        let err = check_values(&huge, &json_val(LANES), None).unwrap_err().0;
        assert!(err.contains("exceeds hard cap"), "{err}");

        // End-to-end refusal when expanded bytes exceed MAX_PLANNED_BYTES even below MAX_PLANNED_RUNS.
        let mut huge_bytes = json_val(TRIAL);
        huge_bytes["arms"] = json!([huge_bytes["arms"][0].clone()]);
        huge_bytes["arms"][0]["name"] = json!("a".repeat(128 * 1024));
        huge_bytes["repeats"] = json!({"start": 600, "cap": 600});
        let err = check_values(&huge_bytes, &json_val(LANES), None)
            .unwrap_err()
            .0;
        assert!(
            err.contains("exceeds hard cap") && err.contains("bytes"),
            "{err}"
        );
    }

    #[test]
    fn trial_and_lanes_refusal_cases() {
        let base_trial = json_val(TRIAL);
        let base_lanes = json_val(LANES);

        // 1. Duplicate JSON member names and unknown fields refused.
        let dup_trial = TRIAL.replacen('{', r#"{"name": "dup", "#, 1);
        assert!(check(dup_trial.as_bytes(), LANES.as_bytes(), None).is_err());
        let dup_lanes = LANES.replacen('{', r#"{"schema": "eon-bench-lanes/1", "#, 1);
        assert!(check(TRIAL.as_bytes(), dup_lanes.as_bytes(), None).is_err());

        let mut unknown_trial = base_trial.clone();
        unknown_trial["wall_cap_s"] = json!(100);
        assert!(check_values(&unknown_trial, &base_lanes, None).is_err());

        let mut unknown_lane = base_lanes.clone();
        unknown_lane["lanes"]["lane-a"]["wall_cap_multiplier"] = json!(1.5);
        assert!(check_values(&base_trial, &unknown_lane, None).is_err());

        // 2. Missing required fields in trial, arm, and lane.
        for key in [
            "schema",
            "name",
            "role",
            "containment",
            "arms",
            "tasks",
            "repeats",
            "pairing_window_hours",
            "budget_caps",
        ] {
            let mut missing = base_trial.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                check_values(&missing, &base_lanes, None).is_err(),
                "missing trial.{key} accepted"
            );
        }
        for key in ["name", "lane", "config"] {
            let mut missing = base_trial.clone();
            missing["arms"][0].as_object_mut().unwrap().remove(key);
            assert!(
                check_values(&missing, &base_lanes, None).is_err(),
                "missing arm.{key} accepted"
            );
        }
        for key in [
            "pool",
            "launch_template",
            "usage_read",
            "floors",
            "run_cap",
            "live_runs_counter",
            "stagger_s",
            "heavy_queue_command",
            "context_handoff_tokens",
            "extra_env",
            "approved_containment",
        ] {
            let mut missing = base_lanes.clone();
            missing["lanes"]["lane-a"]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                check_values(&base_trial, &missing, None).is_err(),
                "missing lane.{key} accepted"
            );
        }

        // 3. Wrong schemas and `/1` configs (inline and stored).
        let mut bad_trial_schema = base_trial.clone();
        bad_trial_schema["schema"] = json!("eon-bench-trial/2");
        assert!(check_values(&bad_trial_schema, &base_lanes, None).is_err());

        let mut bad_lanes_schema = base_lanes.clone();
        bad_lanes_schema["schema"] = json!("eon-bench-lanes/2");
        assert!(check_values(&base_trial, &bad_lanes_schema, None).is_err());

        let mut v1_inline = base_trial.clone();
        v1_inline["arms"][0]["config"] = json_val(CONFIG_V1);
        let err = check_values(&v1_inline, &base_lanes, None).unwrap_err().0;
        assert!(err.contains("eon-bench-config/1"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let v1_hashed = record::hash(CONFIG_V1.as_bytes()).unwrap();
        store
            .write_canonical("configs", &v1_hashed.hash, &v1_hashed.canonical)
            .unwrap();
        let mut v1_stored = base_trial.clone();
        v1_stored["arms"][0]["config"] = json!(v1_hashed.hash);
        let err = check_values(&v1_stored, &base_lanes, Some(&store))
            .unwrap_err()
            .0;
        assert!(
            err.contains("eon-bench-config/1") && err.contains("eon-bench-config/2"),
            "{err}"
        );

        // 4. Empty arms/lanes, duplicate arm names, duplicate config hashes, duplicate task hashes/ids.
        let mut empty_arms = base_trial.clone();
        empty_arms["arms"] = json!([]);
        assert!(check_values(&empty_arms, &base_lanes, None).is_err());

        let mut empty_lanes = base_lanes.clone();
        empty_lanes["lanes"] = json!({});
        assert!(check_values(&base_trial, &empty_lanes, None).is_err());

        let mut dup_arm_name = base_trial.clone();
        dup_arm_name["arms"][1]["name"] = json!("arm-a");
        assert!(check_values(&dup_arm_name, &base_lanes, None).is_err());

        let mut dup_config_hash = base_trial.clone();
        dup_config_hash["arms"][1]["config"] = dup_config_hash["arms"][0]["config"].clone();
        assert!(check_values(&dup_config_hash, &base_lanes, None).is_err());

        let mut dup_task_hash = base_trial.clone();
        let task0 = dup_task_hash["tasks"][0].clone();
        dup_task_hash["tasks"].as_array_mut().unwrap().push(task0);
        assert!(check_values(&dup_task_hash, &base_lanes, None).is_err());

        let mut dup_task_id = base_trial.clone();
        let mut task_same_id = dup_task_id["tasks"][0].clone();
        task_same_id["wall_cap_s"] = json!(7200);
        dup_task_id["tasks"]
            .as_array_mut()
            .unwrap()
            .push(task_same_id);
        assert!(check_values(&dup_task_id, &base_lanes, None).is_err());

        // 5. Role-specific `tasks` vs `cases` (including explicit `null`), one mutation per base.
        let mut builder_with_cases_null = base_trial.clone();
        builder_with_cases_null["cases"] = Value::Null;
        let err = check_values(&builder_with_cases_null, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("only a reviewer trial"), "{err}");

        let mut builder_empty_tasks = base_trial.clone();
        builder_empty_tasks["tasks"] = json!([]);
        let err = check_values(&builder_empty_tasks, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("at least one task"), "{err}");

        let mut rev_clone_cfg = json_val(CONFIG_CLONE);
        rev_clone_cfg["role"] = json!("reviewer");
        let mut rev_case = json_val(TASK);
        rev_case["role"] = json!("reviewer");
        let valid_reviewer_trial = json!({
            "schema": TRIAL_SCHEMA,
            "name": "clone-reviewer-base",
            "role": "reviewer",
            "containment": "clone",
            "arms": [{
                "name": "rev-a",
                "lane": "lane-a",
                "config": rev_clone_cfg
            }],
            "cases": [rev_case],
            "repeats": {"start": 1, "cap": 2},
            "pairing_window_hours": 72,
            "budget_caps": {"gemini-pro": {"week_pct": 15.0}}
        });
        assert!(check_values(&valid_reviewer_trial, &base_lanes, None).is_ok());

        let mut reviewer_with_tasks_null = valid_reviewer_trial.clone();
        reviewer_with_tasks_null["tasks"] = Value::Null;
        let err = check_values(&reviewer_with_tasks_null, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("only a builder trial"), "{err}");

        let mut reviewer_empty_cases = valid_reviewer_trial.clone();
        reviewer_empty_cases["cases"] = json!([]);
        let err = check_values(&reviewer_empty_cases, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("at least one case"), "{err}");

        let mut reviewer_missing_cases = valid_reviewer_trial.clone();
        reviewer_missing_cases
            .as_object_mut()
            .unwrap()
            .remove("cases");
        let err = check_values(&reviewer_missing_cases, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("requires `cases`"), "{err}");

        // 6. Repeats, pairing_window_hours, budget_caps.
        for bad_repeats in [
            json!({"start": 0, "cap": 2}),
            json!({"start": 3, "cap": 2}),
            json!({"start": 1, "cap": 1u64 << 53}),
        ] {
            let mut t = base_trial.clone();
            t["repeats"] = bad_repeats;
            assert!(check_values(&t, &base_lanes, None).is_err());
        }
        let mut zero_window = base_trial.clone();
        zero_window["pairing_window_hours"] = json!(0);
        assert!(check_values(&zero_window, &base_lanes, None).is_err());

        for bad_caps in [
            json!({}),
            json!({"gemini-pro": {}, "claude-pro": {"week_pct": 10.0}}),
            json!({"gemini-pro": {"week_pct": 0.0}, "claude-pro": {"week_pct": 10.0}}),
            json!({"gemini-pro": {"week_pct": -5.0}, "claude-pro": {"week_pct": 10.0}}),
        ] {
            let mut t = base_trial.clone();
            t["budget_caps"] = bad_caps;
            assert!(check_values(&t, &base_lanes, None).is_err());
        }

        // 7. Invalid inline configs: clone with jail digest, jail without jail digest, judge arm,
        //    mismatched role, mismatched containment, task given where config expected.
        let mut clone_with_jail_digest = base_trial.clone();
        clone_with_jail_digest["arms"][0]["config"]["tools"]["jail_profile_sha256"] =
            json!("4".repeat(64));
        assert!(check_values(&clone_with_jail_digest, &base_lanes, None).is_err());

        let mut jail_trial = base_trial.clone();
        jail_trial["containment"] = json!("jail");
        jail_trial["arms"] = json!([{
            "name": "jail-arm",
            "lane": "lane-a",
            "config": json_val(CONFIG)
        }]);
        assert!(check_values(&jail_trial, &base_lanes, None).is_ok());

        let mut jail_without_digest = jail_trial.clone();
        jail_without_digest["arms"][0]["config"]["tools"]["jail_profile_sha256"] = Value::Null;
        assert!(check_values(&jail_without_digest, &base_lanes, None).is_err());

        let mut judge_arm = jail_trial.clone();
        judge_arm["arms"][0]["config"] = json_val(JUDGE);
        assert!(check_values(&judge_arm, &base_lanes, None).is_err());

        let mut wrong_containment = jail_trial.clone();
        wrong_containment["arms"][0]["config"] = json_val(CONFIG_CLONE);
        assert!(check_values(&wrong_containment, &base_lanes, None).is_err());

        let mut wrong_task_role = base_trial.clone();
        wrong_task_role["tasks"][0]["role"] = json!("reviewer");
        assert!(check_values(&wrong_task_role, &base_lanes, None).is_err());

        // 8. plan_review_lane mismatch and invalid plan_review.reviewer_config (v1 or wrong role),
        //    each tested as a single mutation from a validated base.
        let mut unexpected_pr_lane = base_trial.clone();
        unexpected_pr_lane["arms"][0]["plan_review_lane"] = json!("lane-a");
        let err = check_values(&unexpected_pr_lane, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("plan_review \"none\""), "{err}");

        let mut rev_jail_cfg = json_val(CONFIG);
        rev_jail_cfg["role"] = json!("reviewer");
        let rev_jail_hash = store
            .put(&serde_json::to_vec(&rev_jail_cfg).unwrap())
            .unwrap()
            .hash;
        let mut valid_pr_trial = jail_trial.clone();
        valid_pr_trial["arms"][0]["plan_review_lane"] = json!("lane-a");
        valid_pr_trial["arms"][0]["config"]["plan_review"] =
            json!({"reviewer_config": rev_jail_hash, "max_rounds": 1});
        assert!(check_values(&valid_pr_trial, &base_lanes, Some(&store)).is_ok());

        let mut missing_pr_lane = valid_pr_trial.clone();
        missing_pr_lane["arms"][0]
            .as_object_mut()
            .unwrap()
            .remove("plan_review_lane");
        let err = check_values(&missing_pr_lane, &base_lanes, Some(&store))
            .unwrap_err()
            .0;
        assert!(err.contains("requires `plan_review_lane`"), "{err}");

        let mut v1_rev_cfg = json_val(CONFIG_V1);
        v1_rev_cfg["role"] = json!("reviewer");
        let v1_rev_hashed = record::hash(&serde_json::to_vec(&v1_rev_cfg).unwrap()).unwrap();
        store
            .write_canonical("configs", &v1_rev_hashed.hash, &v1_rev_hashed.canonical)
            .unwrap();
        let mut v1_pr_reviewer = valid_pr_trial.clone();
        v1_pr_reviewer["arms"][0]["config"]["plan_review"]["reviewer_config"] =
            json!(v1_rev_hashed.hash);
        let err = check_values(&v1_pr_reviewer, &base_lanes, Some(&store))
            .unwrap_err()
            .0;
        assert!(
            err.contains("eon-bench-config/1") && err.contains("eon-bench-config/2"),
            "{err}"
        );

        let builder_stored = store.put(CONFIG.as_bytes()).unwrap().hash;
        let mut wrong_role_pr_reviewer = valid_pr_trial.clone();
        wrong_role_pr_reviewer["arms"][0]["config"]["plan_review"]["reviewer_config"] =
            json!(builder_stored);
        let err = check_values(&wrong_role_pr_reviewer, &base_lanes, Some(&store))
            .unwrap_err()
            .0;
        assert!(err.contains("expected reviewer"), "{err}");

        // Missing stored plan_review.reviewer_config includes arm and field context.
        let mut missing_pr_reviewer = valid_pr_trial.clone();
        missing_pr_reviewer["arms"][0]["config"]["plan_review"]["reviewer_config"] =
            json!("a".repeat(64));
        let err = check_values(&missing_pr_reviewer, &base_lanes, Some(&store))
            .unwrap_err()
            .0;
        assert!(
            err.contains("jail-arm") && err.contains("plan_review.reviewer_config"),
            "{err}"
        );

        // 9. Positional arrays for structs at trial, lanes, and nested record boundaries.
        let mut repeats_array = base_trial.clone();
        repeats_array["repeats"] = json!([2, 4]);
        let err = check_values(&repeats_array, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("expected a JSON object"), "{err}");

        let mut arm_array = base_trial.clone();
        arm_array["arms"][0] = json!(["arm-a", "lane-a", json_val(CONFIG_CLONE), null]);
        let err = check_values(&arm_array, &base_lanes, None).unwrap_err().0;
        assert!(
            err.contains("arms[0]") && err.contains("expected a JSON object"),
            "{err}"
        );

        let mut inline_limits_array = base_trial.clone();
        inline_limits_array["arms"][0]["config"]["limits"] = json!([1.5, 200]);
        let err = check_values(&inline_limits_array, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("expected a JSON object"), "{err}");

        let mut usage_read_array = base_lanes.clone();
        usage_read_array["lanes"]["lane-a"]["usage_read"] = json!([["bench-usage"], "agy"]);
        let err = check_values(&base_trial, &usage_read_array, None)
            .unwrap_err()
            .0;
        assert!(
            err.contains("lanes.lane-a")
                && err.contains("usage_read")
                && err.contains("expected a JSON object"),
            "{err}"
        );

        let mut counter_array = base_lanes.clone();
        counter_array["lanes"]["lane-a"]["live_runs_counter"] =
            json!(["lock_file", "/var/lib/bench/locks/lane-a.lock"]);
        let err = check_values(&base_trial, &counter_array, None)
            .unwrap_err()
            .0;
        assert!(
            err.contains("lanes.lane-a")
                && err.contains("live_runs_counter")
                && err.contains("expected a JSON object"),
            "{err}"
        );

        // Lane parse errors preserve lane/field context and line/column position.
        let bad_parser_lanes = LANES.replacen("\"agy\"", "\"other\"", 1);
        let err = check(TRIAL.as_bytes(), bad_parser_lanes.as_bytes(), None)
            .unwrap_err()
            .0;
        assert!(
            err.contains("lanes.lane-a") && err.contains("usage_read") && err.contains("line "),
            "{err}"
        );

        let missing_pool_lanes = LANES.replacen("\"pool\": \"gemini-pro\",", "", 1);
        let err = check(TRIAL.as_bytes(), missing_pool_lanes.as_bytes(), None)
            .unwrap_err()
            .0;
        assert!(
            err.contains("lanes.lane-a")
                && err.contains("missing field `pool`")
                && err.contains("line "),
            "{err}"
        );

        // 10. Lane validation errors (including NUL bytes and unused lanes).
        for (field, edit) in [
            (
                "lanes.lane-a.launch_template[0]",
                (|l: &mut Value| l["launch_template"][0] = json!("bench\0harness"))
                    as fn(&mut Value),
            ),
            ("lanes.lane-a.usage_read.command[0]", |l: &mut Value| {
                l["usage_read"]["command"][0] = json!("bench\0usage")
            }),
            ("lanes.lane-a.heavy_queue_command[0]", |l: &mut Value| {
                l["heavy_queue_command"][0] = json!("bench\0heavy")
            }),
            ("lanes.lane-a.live_runs_counter.path", |l: &mut Value| {
                l["live_runs_counter"]["path"] = json!("/var/lib/bench/\0lock")
            }),
            ("lanes.lane-a.home_settings_files[0]", |l: &mut Value| {
                l["home_settings_files"][0] = json!("settings\0.json")
            }),
            ("lanes.lane-a.audit_dirs[0]", |l: &mut Value| {
                l["audit_dirs"][0] = json!("/var/lib/\0audit")
            }),
        ] {
            let mut l = base_lanes.clone();
            edit(&mut l["lanes"]["lane-a"]);
            let err = check_values(&base_trial, &l, None).unwrap_err().0;
            assert!(err.contains(field) && err.contains("NUL"), "{field}: {err}");
        }

        let mut lane_cases: Vec<(&str, Value)> = Vec::new();
        let mut add_lane_case = |label: &'static str, edit: fn(&mut Value)| {
            let mut l = base_lanes.clone();
            edit(&mut l["lanes"]["lane-a"]);
            lane_cases.push((label, l));
        };
        add_lane_case("empty launch_template", |l| {
            l["launch_template"] = json!([])
        });
        add_lane_case("empty usage_read.command", |l| {
            l["usage_read"]["command"] = json!([])
        });
        add_lane_case("unknown usage parser", |l| {
            l["usage_read"]["parser"] = json!("other")
        });
        add_lane_case("negative floor", |l| l["floors"]["week_pct"] = json!(-0.1));
        add_lane_case("zero run_cap", |l| l["run_cap"] = json!(0));
        add_lane_case("unknown counter kind", |l| {
            l["live_runs_counter"]["kind"] = json!("socket")
        });
        add_lane_case("empty heavy_queue_command", |l| {
            l["heavy_queue_command"] = json!([])
        });
        add_lane_case("zero context_handoff_tokens", |l| {
            l["context_handoff_tokens"] = json!(0)
        });
        add_lane_case("KEY=VALUE in extra_env", |l| {
            l["extra_env"] = json!(["SECRET=1"])
        });
        add_lane_case("duplicate extra_env", |l| {
            l["extra_env"] = json!(["FOO", "FOO"])
        });
        add_lane_case("empty approved_containment", |l| {
            l["approved_containment"] = json!([])
        });
        add_lane_case("duplicate approved_containment", |l| {
            l["approved_containment"] = json!(["clone", "clone"])
        });
        for (label, bad_lanes) in lane_cases {
            assert!(
                check_values(&base_trial, &bad_lanes, None).is_err(),
                "{label} was accepted"
            );
        }

        // 10. Unknown lane, unapproved containment, uncapped pool.
        let mut missing_lane = base_trial.clone();
        missing_lane["arms"][0]["lane"] = json!("lane-missing");
        assert!(check_values(&missing_lane, &base_lanes, None).is_err());

        // lane-b is approved for clone only, so a jail trial on lane-b is refused.
        let mut unapproved_lane = jail_trial.clone();
        unapproved_lane["arms"][0]["lane"] = json!("lane-b");
        let err = check_values(&unapproved_lane, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("not approved for jail"), "{err}");

        let mut uncapped_pool = base_trial.clone();
        uncapped_pool["budget_caps"]
            .as_object_mut()
            .unwrap()
            .remove("claude-pro");
        let err = check_values(&uncapped_pool, &base_lanes, None)
            .unwrap_err()
            .0;
        assert!(err.contains("budget_caps"), "{err}");
    }

    #[test]
    fn lanes_machine_account_group_and_key_value_validation() {
        let base_lanes = json_val(LANES);

        // Canonical fixture includes machine limits and parses cleanly.
        let parsed = parse_lanes(LANES.as_bytes()).unwrap();
        assert_eq!(
            parsed.machine,
            Some(MachineLimits {
                max_load_1m: 16.0,
                min_available_mem_mib: 8192,
            })
        );

        // Backward compatibility: lanes file without `machine` still parses for `trial check`.
        let mut without_machine = base_lanes.clone();
        without_machine.as_object_mut().unwrap().remove("machine");
        let parsed_no_machine =
            parse_lanes(&serde_json::to_vec(&without_machine).unwrap()).unwrap();
        assert_eq!(parsed_no_machine.machine, None);

        // Optional `account`, `usage_read.group` (with `agy`), `usage_read.in_slot_command`,
        // `pool_live_runs_counter`, and `key_value` parser succeed.
        let mut extended = base_lanes.clone();
        extended["lanes"]["lane-a"]["account"] = json!("acct-shared");
        extended["lanes"]["lane-a"]["usage_read"]["group"] = json!("gemini-pro");
        extended["lanes"]["lane-a"]["usage_read"]["in_slot_command"] =
            json!(["bench-usage-inner", "--lane", "lane-a"]);
        extended["lanes"]["lane-a"]["pool_live_runs_counter"] = json!({
            "kind": "caps_file",
            "path": "/var/lib/bench/caps/pool-runs.json"
        });
        extended["lanes"]["lane-b"]["usage_read"]["parser"] = json!("key_value");
        let parsed_ext = parse_lanes(&serde_json::to_vec(&extended).unwrap()).unwrap();
        let lane_a = &parsed_ext.lanes[&Text::new("lane-a").unwrap()];
        let lane_b = &parsed_ext.lanes[&Text::new("lane-b").unwrap()];
        assert_eq!(lane_a.account_key("lane-a"), "acct-shared");
        assert_eq!(lane_b.account_key("lane-b"), "lane-b");
        assert_eq!(
            lane_a.usage_read.group.as_ref().map(Text::as_str),
            Some("gemini-pro")
        );
        assert_eq!(
            lane_a
                .usage_read
                .in_slot_command
                .as_ref()
                .map(|c| c.iter().map(Text::as_str).collect::<Vec<_>>()),
            Some(vec!["bench-usage-inner", "--lane", "lane-a"])
        );
        assert_eq!(
            lane_a.pool_live_runs_counter.as_ref().map(|c| c.kind),
            Some(LiveRunsCounterKind::CapsFile)
        );
        assert_eq!(lane_b.usage_read.parser, UsageParser::KeyValue);
        // Round-trips cleanly.
        let round_trip = serde_json::to_vec(&parsed_ext).unwrap();
        assert_eq!(parse_lanes(&round_trip).unwrap(), parsed_ext);

        // Empty or NUL `usage_read.in_slot_command` and invalid `pool_live_runs_counter` are refused.
        let mut empty_in_slot = base_lanes.clone();
        empty_in_slot["lanes"]["lane-a"]["usage_read"]["in_slot_command"] = json!([]);
        assert!(
            parse_lanes(&serde_json::to_vec(&empty_in_slot).unwrap())
                .unwrap_err()
                .0
                .contains("usage_read.in_slot_command")
        );
        let mut nul_in_slot = base_lanes.clone();
        nul_in_slot["lanes"]["lane-a"]["usage_read"]["in_slot_command"] = json!(["cmd\0bad"]);
        assert!(
            parse_lanes(&serde_json::to_vec(&nul_in_slot).unwrap())
                .unwrap_err()
                .0
                .contains("NUL")
        );
        let mut bad_pool_counter = base_lanes.clone();
        bad_pool_counter["lanes"]["lane-a"]["pool_live_runs_counter"] =
            json!(["caps_file", "/tmp/a"]);
        assert!(
            parse_lanes(&serde_json::to_vec(&bad_pool_counter).unwrap())
                .unwrap_err()
                .0
                .contains("pool_live_runs_counter")
        );
        let mut nul_pool_counter = base_lanes.clone();
        nul_pool_counter["lanes"]["lane-a"]["pool_live_runs_counter"] = json!({
            "kind": "caps_file",
            "path": "/var/lib/\0pool.json"
        });
        assert!(
            parse_lanes(&serde_json::to_vec(&nul_pool_counter).unwrap())
                .unwrap_err()
                .0
                .contains("pool_live_runs_counter.path")
        );

        // Invalid `machine` limits are refused.
        for (label, value, needle) in [
            (
                "zero load",
                json!({"max_load_1m": 0.0, "min_available_mem_mib": 1024}),
                "max_load_1m",
            ),
            (
                "negative load",
                json!({"max_load_1m": -1.5, "min_available_mem_mib": 1024}),
                "max_load_1m",
            ),
            (
                "zero memory",
                json!({"max_load_1m": 8.0, "min_available_mem_mib": 0}),
                "min_available_mem_mib",
            ),
            (
                "memory > MAX_EXACT",
                json!({"max_load_1m": 8.0, "min_available_mem_mib": 1u64 << 53}),
                "exact integer range",
            ),
            (
                "unknown machine field",
                json!({"max_load_1m": 8.0, "min_available_mem_mib": 1024, "extra": 1}),
                "machine",
            ),
            (
                "machine array",
                json!([8.0, 1024]),
                "expected a JSON object",
            ),
        ] {
            let mut bad = base_lanes.clone();
            bad["machine"] = value;
            let err = parse_lanes(&serde_json::to_vec(&bad).unwrap())
                .unwrap_err()
                .0;
            assert!(err.contains(needle), "{label}: {err}");
        }

        // Non-finite `max_load_1m` constructed in Rust is refused by `validate_lanes`.
        let mut nan_lanes = parsed.clone();
        nan_lanes.machine.as_mut().unwrap().max_load_1m = f64::NAN;
        assert!(validate_lanes(&nan_lanes).is_err());

        // `usage_read.group` on non-`agy` parser or with control characters is refused.
        let mut group_on_claude = base_lanes.clone();
        group_on_claude["lanes"]["lane-b"]["usage_read"]["group"] = json!("claude");
        let err = parse_lanes(&serde_json::to_vec(&group_on_claude).unwrap())
            .unwrap_err()
            .0;
        assert!(err.contains("only valid with the `agy` parser"), "{err}");

        for bad_group in ["a\tb", "a\nb", "a\rb", "a\0b"] {
            let mut bad = base_lanes.clone();
            bad["lanes"]["lane-a"]["usage_read"]["group"] = json!(bad_group);
            let err = parse_lanes(&serde_json::to_vec(&bad).unwrap())
                .unwrap_err()
                .0;
            assert!(err.contains("usage_read.group"), "{bad_group:?}: {err}");
        }

        // NUL in `account` is refused.
        let mut bad_account = base_lanes.clone();
        bad_account["lanes"]["lane-a"]["account"] = json!("acct\0bad");
        let err = parse_lanes(&serde_json::to_vec(&bad_account).unwrap())
            .unwrap_err()
            .0;
        assert!(
            err.contains("lanes.lane-a.account") && err.contains("NUL"),
            "{err}"
        );
    }
}
