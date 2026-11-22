//! Append-only run and grade rows.
//!
//! A run row is keyed by (config hash, task hash, repeat); a grade row by (run key, judge
//! config hash). Everything else on a row sits beside the key and outside every hash. A rerun
//! adds a row that names the row it replaces in `supersedes`; the old row stays in the file and
//! drops out of every reader. At most one row per key is live.
//!
//! Because at most one row per `(config, task, repeat)` is live, a multi-stage attempt (such as a
//! `builder` or `planner` with `plan_review` enabled) writes one run row for the whole attempt:
//! `stage` records the final stage the attempt reached, while per-stage costs and events across
//! earlier stages live in `tokens` and `events`.
//!
//! `config_drift` is set by [`Store::append_run`] / [`Store::append_grade`], never by the
//! caller: it is true when the server-reported model id differs from the configuration's
//! `model.id`. Loading recomputes it and refuses a row whose flag disagrees. Drifted rows,
//! audit-flagged rows, and infrastructure failures stay in the store but never pool.

use crate::record::{
    CONFIG_SCHEMA, CONFIG_SCHEMA_V1, Config, Containment, Digest, PlanReview, Role, Task, Text,
    Utc, validate_plan_reviewer,
};
use crate::store::Store;
use crate::{Result, check_non_negative_finite, fail, jcs};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

pub const RUN_SCHEMA: &str = "eon-bench-run/1";
pub const GRADE_SCHEMA: &str = "eon-bench-grade/1";
const RUNS: &str = "results.jsonl";
const GRADES: &str = "grades.jsonl";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Plan,
    Review,
    Build,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan => f.write_str("plan"),
            Self::Review => f.write_str("review"),
            Self::Build => f.write_str("build"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStatus {
    Pass,
    Flagged,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditResult {
    pub status: AuditStatus,
    pub findings: Vec<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    EarlyStop,
    Handoff,
    Stall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEvent {
    pub kind: RunEventKind,
    pub stage: Stage,
    pub at: Utc,
    pub reason: Text,
    #[serde(deserialize_with = "Option::deserialize")]
    pub context_tokens: Option<u64>,
    pub resumed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageTokens {
    pub stage: Stage,
    pub round: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolSnapshot {
    pub at: Utc,
    pub meters: BTreeMap<Text, f64>,
    pub other_live_runs: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolReading {
    pub pool: Text,
    #[serde(deserialize_with = "Option::deserialize")]
    pub before: Option<PoolSnapshot>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub after: Option<PoolSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewerVerdict {
    Caught,
    Located,
    Missed,
    Disputed,
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Builder {
        resolved: bool,
        f2p_passed: u64,
        f2p_total: u64,
        p2p_passed: u64,
        p2p_total: u64,
        protected_paths_touched: Vec<Text>,
    },
    Reviewer {
        verdict: ReviewerVerdict,
        blockers: u64,
        should_fix: u64,
        nits: u64,
        pairing_window_waived: bool,
    },
    Planner {
        completed: bool,
    },
    AuditFlagged {
        reason: Text,
    },
    InfraFailure {
        reason: Text,
    },
}

impl Outcome {
    /// Whether this outcome counts in pooled results (`audit_flagged` and `infra_failure` stay
    /// in the store as audit records and are re-run rather than counted).
    pub fn is_countable(&self) -> bool {
        !matches!(self, Self::AuditFlagged { .. } | Self::InfraFailure { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunKey {
    pub config: Digest,
    pub task: Digest,
    /// 1, 2, 3, …
    pub repeat: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradeKey {
    pub run: RunKey,
    pub judge_config: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRow {
    pub schema: Text,
    pub key: RunKey,
    pub run_id: Text,
    #[serde(deserialize_with = "Option::deserialize")]
    pub supersedes: Option<Text>,
    pub pool_label: Text,
    pub started_at: Utc,
    pub finished_at: Utc,
    pub host: Text,
    pub load: Load,
    pub server_model_id: Text,
    pub assembled_prompt_sha256: Digest,
    /// Codex credits spent, or `null` for arms that spend none.
    #[serde(deserialize_with = "Option::deserialize")]
    pub credits: Option<f64>,
    pub config_drift: bool,
    /// The final stage reached by this attempt (one row per `(config, task, repeat)`; earlier
    /// stages of a plan-reviewed attempt appear in `tokens` and `events`).
    pub stage: Stage,
    pub containment: Containment,
    pub residual_risks: Vec<Text>,
    pub audit: AuditResult,
    pub events: Vec<RunEvent>,
    pub tokens: Vec<StageTokens>,
    pub pool_readings: Vec<PoolReading>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradeRow {
    pub schema: Text,
    pub key: GradeKey,
    /// The graded run row; a grade is live only while this is the live run for its key.
    pub run_id: Text,
    pub grade_id: Text,
    #[serde(deserialize_with = "Option::deserialize")]
    pub supersedes: Option<Text>,
    pub pool_label: Text,
    pub started_at: Utc,
    pub finished_at: Utc,
    pub host: Text,
    pub load: Load,
    pub server_model_id: Text,
    pub config_drift: bool,
}

/// What [`live`] and [`admit`] need from a row.
trait Row {
    type Key: PartialEq;
    fn id(&self) -> &Text;
    fn key(&self) -> &Self::Key;
    fn supersedes(&self) -> Option<&Text>;
}

impl Row for RunRow {
    type Key = RunKey;
    fn id(&self) -> &Text {
        &self.run_id
    }
    fn key(&self) -> &RunKey {
        &self.key
    }
    fn supersedes(&self) -> Option<&Text> {
        self.supersedes.as_ref()
    }
}

impl Row for GradeRow {
    type Key = GradeKey;
    fn id(&self) -> &Text {
        &self.grade_id
    }
    fn key(&self) -> &GradeKey {
        &self.key
    }
    fn supersedes(&self) -> Option<&Text> {
        self.supersedes.as_ref()
    }
}

/// Rows that no other row supersedes.
fn live<R: Row>(rows: &[R]) -> Vec<&R> {
    let superseded: HashSet<&Text> = rows.iter().filter_map(Row::supersedes).collect();
    rows.iter()
        .filter(|row| !superseded.contains(row.id()))
        .collect()
}

/// Refuses `new` unless its id is fresh and it supersedes exactly the live row for its key,
/// or there is none and it supersedes nothing.
fn admit<R: Row>(rows: &[R], new: &R) -> Result<()> {
    if rows.iter().any(|row| row.id() == new.id()) {
        fail!("row id {} is already in use", new.id());
    }
    let current = live(rows).into_iter().find(|row| row.key() == new.key());
    match (current, new.supersedes()) {
        (None, None) => Ok(()),
        (Some(row), Some(old)) if row.id() == old => Ok(()),
        (Some(row), _) => fail!(
            "the key already has live row {}; a rerun names it in `supersedes`",
            row.id()
        ),
        (None, Some(old)) => {
            fail!("`supersedes` names {old}, which is not a live row for this key")
        }
    }
}

fn check_times(started: &Utc, finished: &Utc, load: &Load) -> Result<()> {
    if finished < started {
        fail!("finished_at {finished} is before started_at {started}");
    }
    for value in [load.one, load.five, load.fifteen] {
        check_non_negative_finite(value, "load averages")?;
    }
    Ok(())
}

fn stage_allowed(config: &Config, stage: Stage) -> bool {
    match config.role {
        Role::Builder => match config.plan_review {
            Some(PlanReview::Enabled { .. }) => true,
            _ => stage == Stage::Build,
        },
        Role::Reviewer => stage == Stage::Review,
        Role::Planner => match config.plan_review {
            Some(PlanReview::Enabled { .. }) => matches!(stage, Stage::Plan | Stage::Review),
            _ => stage == Stage::Plan,
        },
        Role::Judge => false,
    }
}

fn check_snapshot(snapshot: &PoolSnapshot, pool: &Text, side: &str) -> Result<()> {
    if snapshot.meters.is_empty() {
        fail!("pool_readings.{pool}.{side}.meters must not be empty");
    }
    for (meter, value) in &snapshot.meters {
        check_non_negative_finite(
            *value,
            &format!("pool_readings.{pool}.{side}.meters.{meter}"),
        )?;
    }
    Ok(())
}

fn check_outcome(
    config: &Config,
    task: &Task,
    audit: &AuditResult,
    outcome: &Outcome,
) -> Result<()> {
    let flagged = audit.status == AuditStatus::Flagged;
    match (flagged, outcome) {
        (true, Outcome::AuditFlagged { .. }) => return Ok(()),
        (true, _) => {
            fail!("an audit-flagged run must record outcome kind `audit_flagged`");
        }
        (false, Outcome::AuditFlagged { .. }) => {
            fail!("outcome kind `audit_flagged` requires audit.status == `flagged`");
        }
        (false, _) => {}
    }
    match outcome {
        Outcome::Builder {
            resolved,
            f2p_passed,
            f2p_total,
            p2p_passed,
            p2p_total,
            protected_paths_touched: _,
        } => {
            if config.role != Role::Builder {
                fail!(
                    "builder outcome is only valid for a builder configuration, not {:?}",
                    config.role
                );
            }
            let expected_f2p = u64::try_from(task.key.f2p.len())
                .map_err(|_| crate::Error("task key.f2p length exceeds u64".into()))?;
            let expected_p2p = u64::try_from(task.key.p2p.len())
                .map_err(|_| crate::Error("task key.p2p length exceeds u64".into()))?;
            if *f2p_total != expected_f2p {
                fail!(
                    "outcome.f2p_total ({f2p_total}) disagrees with task {} key.f2p count ({expected_f2p})",
                    task.task_id
                );
            }
            if *p2p_total != expected_p2p {
                fail!(
                    "outcome.p2p_total ({p2p_total}) disagrees with task {} key.p2p count ({expected_p2p})",
                    task.task_id
                );
            }
            if f2p_passed > f2p_total {
                fail!("outcome.f2p_passed ({f2p_passed}) exceeds f2p_total ({f2p_total})");
            }
            if p2p_passed > p2p_total {
                fail!("outcome.p2p_passed ({p2p_passed}) exceeds p2p_total ({p2p_total})");
            }
            // Scoring reverts and flags protected_paths edits before F2P/P2P run, and may still
            // record resolved = false when another task check fails; resolved = true always
            // requires at least one F2P test and every F2P and P2P test passing.
            if *resolved && (*f2p_total == 0 || f2p_passed != f2p_total || p2p_passed != p2p_total)
            {
                fail!(
                    "outcome.resolved cannot be true unless f2p_total > 0 and every F2P and P2P test passes"
                );
            }
            Ok(())
        }
        Outcome::Reviewer { .. } => {
            if config.role != Role::Reviewer {
                fail!(
                    "reviewer outcome is only valid for a reviewer configuration, not {:?}",
                    config.role
                );
            }
            Ok(())
        }
        Outcome::Planner { .. } => {
            if config.role != Role::Planner {
                fail!(
                    "planner outcome is only valid for a planner configuration, not {:?}",
                    config.role
                );
            }
            Ok(())
        }
        Outcome::AuditFlagged { .. } | Outcome::InfraFailure { .. } => Ok(()),
    }
}

impl Store {
    /// Every run row, validated, in file order.
    pub fn runs(&self) -> Result<Vec<RunRow>> {
        let mut configs = HashMap::new();
        let mut tasks = HashMap::new();
        let mut rows = Vec::new();
        for (index, line) in self.lines(RUNS)?.iter().enumerate() {
            let row: RunRow = jcs::from_slice_objects(line.as_bytes())
                .map_err(|error| crate::Error(format!("{RUNS}:{}: {error}", index + 1)))?;
            self.check_run(&row, &mut configs, &mut tasks)
                .and_then(|()| admit(&rows, &row))
                .map_err(|error| crate::Error(format!("{RUNS}:{}: {error}", index + 1)))?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// Every grade row, validated, in file order.
    pub fn grades(&self) -> Result<Vec<GradeRow>> {
        let mut judges = HashMap::new();
        let mut rows = Vec::new();
        for (index, line) in self.lines(GRADES)?.iter().enumerate() {
            let row: GradeRow = jcs::from_slice_objects(line.as_bytes())
                .map_err(|error| crate::Error(format!("{GRADES}:{}: {error}", index + 1)))?;
            self.check_grade(&row, &mut judges)
                .and_then(|()| admit(&rows, &row))
                .map_err(|error| crate::Error(format!("{GRADES}:{}: {error}", index + 1)))?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// The live run rows.
    pub fn live_runs(&self) -> Result<Vec<RunRow>> {
        Ok(live(&self.runs()?).into_iter().cloned().collect())
    }

    /// The live grade rows: not superseded, and grading the run that is live for their key.
    pub fn live_grades(&self) -> Result<Vec<GradeRow>> {
        let runs = self.live_runs()?;
        let grades = self.grades()?;
        Ok(live(&grades)
            .into_iter()
            .filter(|grade| {
                runs.iter()
                    .any(|run| run.key == grade.key.run && run.run_id == grade.run_id)
            })
            .cloned()
            .collect())
    }

    /// Appends a run row after computing its `config_drift`; returns the row as written.
    pub fn append_run(&self, mut row: RunRow) -> Result<RunRow> {
        let _lock = self.lock()?;
        let config = self.config(row.key.config.as_str())?;
        let task = self.task(row.key.task.as_str())?;
        if !task.role.matches(config.role) {
            fail!(
                "config {} is a {:?} configuration but task {} is a {:?} task",
                crate::short(row.key.config.as_str()),
                config.role,
                crate::short(row.key.task.as_str()),
                task.role
            );
        }
        row.config_drift = row.server_model_id != config.model.id;
        let mut configs = HashMap::from([(row.key.config.as_str().to_owned(), config)]);
        let mut tasks = HashMap::from([(row.key.task.as_str().to_owned(), task)]);
        self.check_run(&row, &mut configs, &mut tasks)?;
        admit(&self.runs()?, &row)?;
        self.append_line(RUNS, &serde_json::to_string(&row)?)?;
        Ok(row)
    }

    /// Appends a grade row after computing its `config_drift`; returns the row as written.
    pub fn append_grade(&self, mut row: GradeRow) -> Result<GradeRow> {
        let _lock = self.lock()?;
        let judge = self.judge(row.key.judge_config.as_str())?;
        let runs = self.live_runs()?;
        let Some(run) = runs.iter().find(|run| run.key == row.key.run) else {
            fail!("no live run row for the graded key");
        };
        if run.run_id != row.run_id {
            fail!(
                "run_id {} is not the live run {} for the graded key",
                row.run_id,
                run.run_id
            );
        }
        row.config_drift = row.server_model_id != judge.model.id;
        let mut judges = HashMap::from([(row.key.judge_config.as_str().to_owned(), judge)]);
        self.check_grade(&row, &mut judges)?;
        admit(&self.grades()?, &row)?;
        self.append_line(GRADES, &serde_json::to_string(&row)?)?;
        Ok(row)
    }

    fn check_run(
        &self,
        row: &RunRow,
        configs: &mut HashMap<String, Config>,
        tasks: &mut HashMap<String, Task>,
    ) -> Result<()> {
        if row.schema.as_str() != RUN_SCHEMA {
            fail!("schema {} is not {RUN_SCHEMA}", row.schema);
        }
        if row.key.repeat == 0 {
            fail!("repeat counts from 1");
        }
        if let Some(credits) = row.credits {
            check_non_negative_finite(credits, "credits")?;
        }
        check_times(&row.started_at, &row.finished_at, &row.load)?;
        let config = self.cached_config(configs, row.key.config.as_str(), false)?;
        let task = self.cached_task(tasks, row.key.task.as_str())?;
        if !task.role.matches(config.role) {
            fail!(
                "config {} is a {:?} configuration but task {} is a {:?} task",
                crate::short(row.key.config.as_str()),
                config.role,
                crate::short(row.key.task.as_str()),
                task.role
            );
        }
        if config.is_v1() {
            fail!(
                "run rows require {CONFIG_SCHEMA} configurations; {} is {CONFIG_SCHEMA_V1}",
                crate::short(row.key.config.as_str())
            );
        }
        if config.containment != Some(row.containment) {
            fail!(
                "row containment {} disagrees with configuration containment {:?}",
                row.containment,
                config.containment
            );
        }
        if row.config_drift != (row.server_model_id != config.model.id) {
            fail!("config_drift disagrees with the configuration's model id");
        }
        if !stage_allowed(config, row.stage) {
            fail!(
                "stage {} is not allowed for {:?} configuration {}",
                row.stage,
                config.role,
                crate::short(row.key.config.as_str())
            );
        }
        match row.audit.status {
            AuditStatus::Pass | AuditStatus::Skipped if !row.audit.findings.is_empty() => {
                fail!(
                    "audit.findings must be empty when audit.status is {:?}",
                    row.audit.status
                );
            }
            AuditStatus::Flagged if row.audit.findings.is_empty() => {
                fail!("audit.findings must be non-empty when audit.status is flagged");
            }
            _ => {}
        }
        for event in &row.events {
            if event.at < row.started_at || event.at > row.finished_at {
                fail!(
                    "event timestamp {} is outside run window [{}, {}]",
                    event.at,
                    row.started_at,
                    row.finished_at
                );
            }
            if !stage_allowed(config, event.stage) {
                fail!(
                    "event stage {} is not allowed for {:?} configuration {}",
                    event.stage,
                    config.role,
                    crate::short(row.key.config.as_str())
                );
            }
        }
        let mut token_keys = HashSet::new();
        for entry in &row.tokens {
            if entry.round == 0 {
                fail!("tokens round counts from 1");
            }
            if !stage_allowed(config, entry.stage) {
                fail!(
                    "tokens stage {} is not allowed for {:?} configuration {}",
                    entry.stage,
                    config.role,
                    crate::short(row.key.config.as_str())
                );
            }
            if entry.stage == Stage::Review
                && let Some(PlanReview::Enabled { max_rounds, .. }) = &config.plan_review
                && entry.round > *max_rounds
            {
                fail!(
                    "tokens review round {} exceeds plan_review.max_rounds ({max_rounds})",
                    entry.round
                );
            }
            if !token_keys.insert((entry.stage, entry.round)) {
                fail!(
                    "duplicate tokens entry for stage {} round {}",
                    entry.stage,
                    entry.round
                );
            }
        }
        let mut pools = HashSet::new();
        for reading in &row.pool_readings {
            if !pools.insert(reading.pool.as_str()) {
                fail!("duplicate pool_readings entry for pool {}", reading.pool);
            }
            if reading.before.is_none() && reading.after.is_none() {
                fail!(
                    "pool_readings entry for {} needs at least one of before or after",
                    reading.pool
                );
            }
            if let Some(before) = &reading.before {
                check_snapshot(before, &reading.pool, "before")?;
            }
            if let Some(after) = &reading.after {
                check_snapshot(after, &reading.pool, "after")?;
            }
            if let (Some(before), Some(after)) = (&reading.before, &reading.after)
                && after.at < before.at
            {
                fail!(
                    "pool_readings.{}: after.at {} is before before.at {}",
                    reading.pool,
                    after.at,
                    before.at
                );
            }
        }
        check_outcome(config, task, &row.audit, &row.outcome)?;
        let plan_reviewer = match &config.plan_review {
            Some(PlanReview::Enabled {
                reviewer_config, ..
            }) => Some(reviewer_config.as_str().to_owned()),
            _ => None,
        };
        if let Some(reviewer_hash) = plan_reviewer {
            let reviewer = self.cached_config(configs, &reviewer_hash, false)?;
            validate_plan_reviewer(reviewer, &reviewer_hash, row.containment)?;
        }
        Ok(())
    }

    fn check_grade(&self, row: &GradeRow, judges: &mut HashMap<String, Config>) -> Result<()> {
        if row.schema.as_str() != GRADE_SCHEMA {
            fail!("schema {} is not {GRADE_SCHEMA}", row.schema);
        }
        check_times(&row.started_at, &row.finished_at, &row.load)?;
        let judge = self.cached_config(judges, row.key.judge_config.as_str(), true)?;
        if judge.is_v1() {
            fail!(
                "grade rows require {CONFIG_SCHEMA} judge configurations; {} is {CONFIG_SCHEMA_V1}",
                crate::short(row.key.judge_config.as_str())
            );
        }
        if row.config_drift != (row.server_model_id != judge.model.id) {
            fail!("config_drift disagrees with the judge configuration's model id");
        }
        Ok(())
    }

    fn cached_config<'a>(
        &self,
        cache: &'a mut HashMap<String, Config>,
        hash: &str,
        judge: bool,
    ) -> Result<&'a Config> {
        match cache.entry(hash.to_owned()) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let config = if judge {
                    self.judge(hash)?
                } else {
                    self.config(hash)?
                };
                Ok(entry.insert(config))
            }
        }
    }

    fn cached_task<'a>(
        &self,
        cache: &'a mut HashMap<String, Task>,
        hash: &str,
    ) -> Result<&'a Task> {
        match cache.entry(hash.to_owned()) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => Ok(entry.insert(self.task(hash)?)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::tests::{CONFIG, CONFIG_CLONE, CONFIG_V1, JUDGE, JUDGE_V1, TASK};
    use serde_json::json;

    pub(crate) struct Fixture {
        pub dir: tempfile::TempDir,
        pub store: Store,
        pub config: String,
        pub judge: String,
        pub task: String,
    }

    pub(crate) fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let config = store.put(CONFIG.as_bytes()).unwrap().hash;
        let judge = store.put(JUDGE.as_bytes()).unwrap().hash;
        let task = store.put(TASK.as_bytes()).unwrap().hash;
        Fixture {
            dir,
            store,
            config,
            judge,
            task,
        }
    }

    pub(crate) fn run(config: &str, task: &str, repeat: u64, id: &str) -> RunRow {
        serde_json::from_value(json!({
            "schema": RUN_SCHEMA,
            "key": {"config": config, "task": task, "repeat": repeat},
            "run_id": id,
            "supersedes": null,
            "pool_label": "uclaude",
            "started_at": "2026-12-01T10:00:00Z",
            "finished_at": "2026-12-01T10:40:00Z",
            "host": "bench-host",
            "load": {"one": 1.5, "five": 1.25, "fifteen": 1.0},
            "server_model_id": "claude-opus-5-5",
            "assembled_prompt_sha256": "9".repeat(64),
            "credits": null,
            "config_drift": false,
            "stage": "build",
            "containment": "jail",
            "residual_risks": [],
            "audit": {"status": "pass", "findings": []},
            "events": [],
            "tokens": [{
                "stage": "build",
                "round": 1,
                "input": 12000,
                "output": 3400,
                "cache_read": 8000,
                "cache_write": 1000,
                "reasoning": 500
            }],
            "pool_readings": [{
                "pool": "claude-max",
                "before": {
                    "at": "2026-12-01T09:59:55Z",
                    "meters": {"week_pct": 80.0, "five_hour_pct": 95.0},
                    "other_live_runs": 0
                },
                "after": {
                    "at": "2026-12-01T10:40:05Z",
                    "meters": {"week_pct": 78.5, "five_hour_pct": 90.0},
                    "other_live_runs": 1
                }
            }],
            "outcome": {
                "kind": "builder",
                "resolved": true,
                "f2p_passed": 1,
                "f2p_total": 1,
                "p2p_passed": 1,
                "p2p_total": 1,
                "protected_paths_touched": []
            }
        }))
        .unwrap()
    }

    fn grade(f: &Fixture, run: &RunRow, id: &str) -> GradeRow {
        serde_json::from_value(json!({
            "schema": GRADE_SCHEMA,
            "key": {"run": run.key, "judge_config": f.judge},
            "run_id": run.run_id,
            "grade_id": id,
            "supersedes": null,
            "pool_label": "codex",
            "started_at": "2026-12-01T11:00:00Z",
            "finished_at": "2026-12-01T11:05:00Z",
            "host": "bench-host",
            "load": {"one": 0.5, "five": 0.5, "fifteen": 0.5},
            "server_model_id": "gpt-6.2-sol",
            "config_drift": false
        }))
        .unwrap()
    }

    #[test]
    fn one_live_row_per_key_and_reruns_supersede() {
        let f = fixture();
        let first = f
            .store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        assert!(!first.config_drift);
        // A second row for the key must name the live one.
        assert!(
            f.store
                .append_run(run(&f.config, &f.task, 1, "r2"))
                .is_err()
        );
        let mut rerun = run(&f.config, &f.task, 1, "r2");
        rerun.supersedes = Some(Text::new("r1").unwrap());
        f.store.append_run(rerun.clone()).unwrap();
        // r1 is no longer live, so superseding it again is refused, and ids are unique.
        let mut stale = run(&f.config, &f.task, 1, "r3");
        stale.supersedes = Some(Text::new("r1").unwrap());
        assert!(f.store.append_run(stale).is_err());
        assert!(
            f.store
                .append_run(run(&f.config, &f.task, 2, "r2"))
                .is_err()
        );
        let live: Vec<String> = f
            .store
            .live_runs()
            .unwrap()
            .iter()
            .map(|row| row.run_id.to_string())
            .collect();
        assert_eq!(live, ["r2"]);
        assert_eq!(f.store.runs().unwrap().len(), 2);
    }

    #[test]
    fn drift_is_computed_and_checked() {
        let f = fixture();
        let mut row = run(&f.config, &f.task, 1, "r1");
        row.server_model_id = Text::new("claude-opus-5-5-20261201").unwrap();
        row.config_drift = false;
        assert!(f.store.append_run(row).unwrap().config_drift);
        // A hand-edited flag fails to load.
        let path = f.dir.path().join(RUNS);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace("\"config_drift\":true", "\"config_drift\":false"),
        )
        .unwrap();
        assert!(f.store.runs().is_err());
    }

    #[test]
    fn hand_appended_rows_are_held_to_the_append_rules() {
        let f = fixture();
        f.store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        // A row whose `supersedes` names another key's run would drop r1 from `live`.
        let mut stray = run(&f.config, &f.task, 2, "r2");
        stray.supersedes = Some(Text::new("r1").unwrap());
        f.store
            .append_line(RUNS, &serde_json::to_string(&stray).unwrap())
            .unwrap();
        let error = f.store.runs().unwrap_err().0;
        assert!(error.contains("results.jsonl:2"), "{error}");
        assert!(f.store.live_runs().is_err());

        // A reused id in grades.
        let g = fixture();
        let r1 = g
            .store
            .append_run(run(&g.config, &g.task, 1, "r1"))
            .unwrap();
        g.store.append_grade(grade(&g, &r1, "g1")).unwrap();
        let line = serde_json::to_string(&grade(&g, &r1, "g1")).unwrap();
        g.store.append_line(GRADES, &line).unwrap();
        assert!(g.store.grades().is_err());
    }

    #[test]
    fn appends_check_records_and_roles() {
        let f = fixture();
        // Unknown hashes, a judge config as a run config, and a role mismatch are refused.
        assert!(
            f.store
                .append_run(run(&"0".repeat(64), &f.task, 1, "r1"))
                .is_err()
        );
        assert!(f.store.append_run(run(&f.judge, &f.task, 1, "r1")).is_err());
        let mut reviewer: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        reviewer["role"] = json!("reviewer");
        let reviewer = f
            .store
            .put(&serde_json::to_vec(&reviewer).unwrap())
            .unwrap()
            .hash;
        assert!(
            f.store
                .append_run(run(&reviewer, &f.task, 1, "r1"))
                .is_err()
        );
        // Bad row values.
        assert!(
            f.store
                .append_run(run(&f.config, &f.task, 0, "r1"))
                .is_err()
        );
        let mut backwards = run(&f.config, &f.task, 1, "r1");
        backwards.finished_at = Utc::new("2026-12-01T09:00:00Z").unwrap();
        assert!(f.store.append_run(backwards).is_err());
        assert!(f.store.runs().unwrap().is_empty());
    }

    #[test]
    fn grades_follow_the_live_run() {
        let f = fixture();
        let r1 = f
            .store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        // The graded run must exist and be live, under its own id.
        let missing = run(&f.config, &f.task, 2, "r9");
        assert!(f.store.append_grade(grade(&f, &missing, "g0")).is_err());
        let mut wrong_id = grade(&f, &r1, "g0");
        wrong_id.run_id = Text::new("r9").unwrap();
        assert!(f.store.append_grade(wrong_id).is_err());
        // A builder config is not a judge.
        let mut not_judge = grade(&f, &r1, "g0");
        not_judge.key.judge_config = Digest::new(f.config.clone()).unwrap();
        assert!(f.store.append_grade(not_judge).is_err());

        f.store.append_grade(grade(&f, &r1, "g1")).unwrap();
        assert_eq!(f.store.live_grades().unwrap().len(), 1);
        // A rerun leaves r1's grade behind instead of attaching it to r2.
        let mut r2 = run(&f.config, &f.task, 1, "r2");
        r2.supersedes = Some(Text::new("r1").unwrap());
        let r2 = f.store.append_run(r2).unwrap();
        assert!(f.store.live_grades().unwrap().is_empty());
        // The new run's grade is a new key's live grade (r1's grade no longer blocks it, but
        // the key is the same, so it supersedes g1).
        assert!(f.store.append_grade(grade(&f, &r2, "g2")).is_err());
        let mut g2 = grade(&f, &r2, "g2");
        g2.supersedes = Some(Text::new("g1").unwrap());
        f.store.append_grade(g2).unwrap();
        let live: Vec<String> = f
            .store
            .live_grades()
            .unwrap()
            .iter()
            .map(|g| g.grade_id.to_string())
            .collect();
        assert_eq!(live, ["g2"]);
    }

    #[test]
    fn bench_v2_row_invariants_are_enforced() {
        let f = fixture();

        // Containment mismatch with config is refused.
        let mut wrong_containment = run(&f.config, &f.task, 1, "r-bad-c");
        wrong_containment.containment = Containment::Clone;
        let err = f.store.append_run(wrong_containment).unwrap_err().0;
        assert!(
            err.contains("disagrees with configuration containment"),
            "{err}"
        );

        // Historical `/1` config planted on disk cannot receive new run rows.
        let v1 = crate::record::hash(CONFIG_V1.as_bytes()).unwrap();
        f.store
            .write_canonical("configs", &v1.hash, &v1.canonical)
            .unwrap();
        let err = f
            .store
            .append_run(run(&v1.hash, &f.task, 1, "r-v1"))
            .unwrap_err()
            .0;
        assert!(
            err.contains("eon-bench-config/1") && err.contains("eon-bench-config/2"),
            "{err}"
        );

        // Builder with plan_review == "none" refuses stage "plan" and "review".
        for stage in [Stage::Plan, Stage::Review] {
            let mut bad_stage = run(&f.config, &f.task, 1, "r-stage");
            bad_stage.stage = stage;
            assert!(f.store.append_run(bad_stage).is_err());
        }

        // Audit findings invariant and coupling with Outcome::AuditFlagged.
        let mut pass_with_findings = run(&f.config, &f.task, 1, "r-a1");
        pass_with_findings.audit.findings = vec![Text::new("leak").unwrap()];
        assert!(f.store.append_run(pass_with_findings).is_err());

        let mut skipped_with_findings = run(&f.config, &f.task, 1, "r-a2");
        skipped_with_findings.audit = AuditResult {
            status: AuditStatus::Skipped,
            findings: vec![Text::new("note").unwrap()],
        };
        assert!(f.store.append_run(skipped_with_findings).is_err());

        let mut flagged_empty = run(&f.config, &f.task, 1, "r-a3");
        flagged_empty.audit = AuditResult {
            status: AuditStatus::Flagged,
            findings: vec![],
        };
        flagged_empty.outcome = Outcome::AuditFlagged {
            reason: Text::new("outside write").unwrap(),
        };
        assert!(f.store.append_run(flagged_empty).is_err());

        let mut flagged_scored = run(&f.config, &f.task, 1, "r-a4");
        flagged_scored.audit = AuditResult {
            status: AuditStatus::Flagged,
            findings: vec![Text::new("modified /tmp/other").unwrap()],
        };
        assert!(f.store.append_run(flagged_scored).is_err());

        let mut pass_flagged_outcome = run(&f.config, &f.task, 1, "r-a5");
        pass_flagged_outcome.outcome = Outcome::AuditFlagged {
            reason: Text::new("spurious").unwrap(),
        };
        assert!(f.store.append_run(pass_flagged_outcome).is_err());

        // Valid audit_flagged row is accepted.
        let mut valid_flagged = run(&f.config, &f.task, 1, "r-flagged");
        valid_flagged.audit = AuditResult {
            status: AuditStatus::Flagged,
            findings: vec![Text::new("touched ../other-checkout").unwrap()],
        };
        valid_flagged.outcome = Outcome::AuditFlagged {
            reason: Text::new("outside path access").unwrap(),
        };
        f.store.append_run(valid_flagged).unwrap();

        // Builder outcome invariants.
        for (bad_outcome, needle) in [
            (
                Outcome::Builder {
                    resolved: false,
                    f2p_passed: 2,
                    f2p_total: 1,
                    p2p_passed: 1,
                    p2p_total: 1,
                    protected_paths_touched: vec![],
                },
                "f2p_passed",
            ),
            (
                Outcome::Builder {
                    resolved: false,
                    f2p_passed: 1,
                    f2p_total: 1,
                    p2p_passed: 2,
                    p2p_total: 1,
                    protected_paths_touched: vec![],
                },
                "p2p_passed",
            ),
            (
                Outcome::Builder {
                    resolved: false,
                    f2p_passed: 1,
                    f2p_total: 2,
                    p2p_passed: 1,
                    p2p_total: 1,
                    protected_paths_touched: vec![],
                },
                "key.f2p count",
            ),
            (
                Outcome::Builder {
                    resolved: false,
                    f2p_passed: 1,
                    f2p_total: 1,
                    p2p_passed: 1,
                    p2p_total: 2,
                    protected_paths_touched: vec![],
                },
                "key.p2p count",
            ),
            (
                Outcome::Builder {
                    resolved: true,
                    f2p_passed: 0,
                    f2p_total: 1,
                    p2p_passed: 1,
                    p2p_total: 1,
                    protected_paths_touched: vec![],
                },
                "outcome.resolved cannot be true",
            ),
            (
                Outcome::Builder {
                    resolved: true,
                    f2p_passed: 1,
                    f2p_total: 1,
                    p2p_passed: 0,
                    p2p_total: 1,
                    protected_paths_touched: vec![],
                },
                "outcome.resolved cannot be true",
            ),
            (
                Outcome::Reviewer {
                    verdict: ReviewerVerdict::Caught,
                    blockers: 1,
                    should_fix: 0,
                    nits: 0,
                    pairing_window_waived: false,
                },
                "reviewer outcome is only valid for a reviewer configuration",
            ),
            (
                Outcome::Planner { completed: true },
                "planner outcome is only valid for a planner configuration",
            ),
        ] {
            let mut row = run(&f.config, &f.task, 2, "r-bad-o");
            row.outcome = bad_outcome;
            let err = f.store.append_run(row).unwrap_err().0;
            assert!(err.contains(needle), "{err}");
        }

        // Scoring reverts and flags protected_paths edits before F2P/P2P run, so resolved = true
        // with protected_paths_touched non-empty is valid when every F2P/P2P passes.
        let mut resolved_with_protected = run(&f.config, &f.task, 2, "r-prot");
        resolved_with_protected.outcome = Outcome::Builder {
            resolved: true,
            f2p_passed: 1,
            f2p_total: 1,
            p2p_passed: 1,
            p2p_total: 1,
            protected_paths_touched: vec![Text::new("venus/src/scene/tests/bar.rs").unwrap()],
        };
        f.store.append_run(resolved_with_protected).unwrap();

        // Scorer may also record resolved = false even when F2P/P2P pass (e.g., another check fails).
        let mut disqualified = run(&f.config, &f.task, 2, "r-disq");
        disqualified.supersedes = Some(Text::new("r-prot").unwrap());
        disqualified.outcome = Outcome::Builder {
            resolved: false,
            f2p_passed: 1,
            f2p_total: 1,
            p2p_passed: 1,
            p2p_total: 1,
            protected_paths_touched: vec![],
        };
        f.store.append_run(disqualified).unwrap();

        // Unknown field inside tagged outcome is refused.
        let mut row_json = serde_json::to_value(run(&f.config, &f.task, 3, "r-extra")).unwrap();
        row_json["outcome"]["bogus"] = json!(true);
        assert!(serde_json::from_value::<RunRow>(row_json).is_err());

        // Events timestamp bounds and stage validation.
        let mut early_event = run(&f.config, &f.task, 3, "r-ev1");
        early_event.events.push(RunEvent {
            kind: RunEventKind::Handoff,
            stage: Stage::Build,
            at: Utc::new("2026-12-01T09:59:59Z").unwrap(),
            reason: Text::new("before start").unwrap(),
            context_tokens: Some(400_000),
            resumed: true,
        });
        assert!(f.store.append_run(early_event).is_err());

        let mut late_event = run(&f.config, &f.task, 3, "r-ev2");
        late_event.events.push(RunEvent {
            kind: RunEventKind::Stall,
            stage: Stage::Build,
            at: Utc::new("2026-12-01T10:40:01Z").unwrap(),
            reason: Text::new("after finish").unwrap(),
            context_tokens: None,
            resumed: false,
        });
        assert!(f.store.append_run(late_event).is_err());

        let mut bad_event_stage = run(&f.config, &f.task, 3, "r-ev3");
        bad_event_stage.events.push(RunEvent {
            kind: RunEventKind::EarlyStop,
            stage: Stage::Plan,
            at: Utc::new("2026-12-01T10:10:00Z").unwrap(),
            reason: Text::new("empty final").unwrap(),
            context_tokens: None,
            resumed: true,
        });
        assert!(f.store.append_run(bad_event_stage).is_err());

        // Tokens round >= 1 and unique (stage, round).
        let mut zero_round = run(&f.config, &f.task, 3, "r-tok1");
        zero_round.tokens[0].round = 0;
        assert!(f.store.append_run(zero_round).is_err());

        let mut dup_tokens = run(&f.config, &f.task, 3, "r-tok2");
        let first_tok = dup_tokens.tokens[0].clone();
        dup_tokens.tokens.push(first_tok);
        assert!(f.store.append_run(dup_tokens).is_err());

        // Pool readings checks: duplicate pool, both null, after < before, empty meters, negative meter.
        let mut dup_pool = run(&f.config, &f.task, 3, "r-p1");
        let first_pool = dup_pool.pool_readings[0].clone();
        dup_pool.pool_readings.push(first_pool);
        assert!(f.store.append_run(dup_pool).is_err());

        let mut empty_reading = run(&f.config, &f.task, 3, "r-p2");
        empty_reading.pool_readings[0].before = None;
        empty_reading.pool_readings[0].after = None;
        assert!(f.store.append_run(empty_reading).is_err());

        let mut inverted_pool = run(&f.config, &f.task, 3, "r-p3");
        inverted_pool.pool_readings[0].after.as_mut().unwrap().at =
            Utc::new("2026-12-01T09:59:00Z").unwrap();
        assert!(f.store.append_run(inverted_pool).is_err());

        let mut empty_meters = run(&f.config, &f.task, 3, "r-p4");
        empty_meters.pool_readings[0]
            .before
            .as_mut()
            .unwrap()
            .meters
            .clear();
        assert!(f.store.append_run(empty_meters).is_err());

        let mut neg_meter = run(&f.config, &f.task, 3, "r-p5");
        neg_meter.pool_readings[0]
            .before
            .as_mut()
            .unwrap()
            .meters
            .insert(Text::new("week_pct").unwrap(), -1.0);
        assert!(f.store.append_run(neg_meter).is_err());

        // Single-sided pool reading + infra_failure with empty tokens + valid event inside window.
        let mut infra = run(&f.config, &f.task, 3, "r-infra");
        infra.tokens.clear();
        infra.pool_readings[0].after = None;
        infra.events.push(RunEvent {
            kind: RunEventKind::Stall,
            stage: Stage::Build,
            at: Utc::new("2026-12-01T10:15:00Z").unwrap(),
            reason: Text::new("15m without events").unwrap(),
            context_tokens: Some(120_000),
            resumed: false,
        });
        infra.outcome = Outcome::InfraFailure {
            reason: Text::new("stalled").unwrap(),
        };
        f.store.append_run(infra).unwrap();
    }

    #[test]
    fn reviewer_and_plan_review_rows_validate_roles_and_reviewer_config() {
        let f = fixture();

        // Reviewer task and reviewer config (jail and clone).
        let mut rev_task_val: serde_json::Value = serde_json::from_str(TASK).unwrap();
        rev_task_val["role"] = json!("reviewer");
        let rev_task = f
            .store
            .put(&serde_json::to_vec(&rev_task_val).unwrap())
            .unwrap()
            .hash;

        let mut rev_jail_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        rev_jail_val["role"] = json!("reviewer");
        let rev_jail = f
            .store
            .put(&serde_json::to_vec(&rev_jail_val).unwrap())
            .unwrap()
            .hash;

        let mut rev_clone_val: serde_json::Value = serde_json::from_str(CONFIG_CLONE).unwrap();
        rev_clone_val["role"] = json!("reviewer");
        let rev_clone = f
            .store
            .put(&serde_json::to_vec(&rev_clone_val).unwrap())
            .unwrap()
            .hash;

        // Valid reviewer row.
        let mut rev_row = run(&rev_jail, &rev_task, 1, "rev-1");
        rev_row.stage = Stage::Review;
        rev_row.tokens[0].stage = Stage::Review;
        rev_row.outcome = Outcome::Reviewer {
            verdict: ReviewerVerdict::Caught,
            blockers: 2,
            should_fix: 1,
            nits: 0,
            pairing_window_waived: false,
        };
        f.store.append_run(rev_row).unwrap();

        // Builder config referencing missing reviewer_config is refused on append_run.
        let mut missing_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        missing_pr_val["plan_review"] = json!({
            "reviewer_config": "a".repeat(64),
            "max_rounds": 2
        });
        let missing_pr = f
            .store
            .put(&serde_json::to_vec(&missing_pr_val).unwrap())
            .unwrap()
            .hash;
        assert!(
            f.store
                .append_run(run(&missing_pr, &f.task, 1, "pr-missing"))
                .is_err()
        );

        // Builder config referencing a non-reviewer config as reviewer_config is refused.
        let mut wrong_role_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        wrong_role_pr_val["plan_review"] = json!({
            "reviewer_config": f.config,
            "max_rounds": 2
        });
        let wrong_role_pr = f
            .store
            .put(&serde_json::to_vec(&wrong_role_pr_val).unwrap())
            .unwrap()
            .hash;
        assert!(
            f.store
                .append_run(run(&wrong_role_pr, &f.task, 1, "pr-wrong-role"))
                .is_err()
        );

        // Jail builder config referencing clone reviewer_config is refused.
        let mut wrong_cont_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        wrong_cont_pr_val["plan_review"] = json!({
            "reviewer_config": rev_clone,
            "max_rounds": 2
        });
        let wrong_cont_pr = f
            .store
            .put(&serde_json::to_vec(&wrong_cont_pr_val).unwrap())
            .unwrap()
            .hash;
        assert!(
            f.store
                .append_run(run(&wrong_cont_pr, &f.task, 1, "pr-wrong-cont"))
                .is_err()
        );

        // Valid plan_review builder config allows plan, review and build stages in tokens/events.
        let mut valid_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        valid_pr_val["plan_review"] = json!({
            "reviewer_config": rev_jail,
            "max_rounds": 2
        });
        let valid_pr = f
            .store
            .put(&serde_json::to_vec(&valid_pr_val).unwrap())
            .unwrap()
            .hash;
        let mut pr_row = run(&valid_pr, &f.task, 1, "pr-ok");
        pr_row.tokens = vec![
            StageTokens {
                stage: Stage::Plan,
                round: 1,
                input: 4000,
                output: 1200,
                cache_read: 0,
                cache_write: 0,
                reasoning: 300,
            },
            StageTokens {
                stage: Stage::Review,
                round: 1,
                input: 5000,
                output: 800,
                cache_read: 0,
                cache_write: 0,
                reasoning: 200,
            },
            StageTokens {
                stage: Stage::Build,
                round: 1,
                input: 10000,
                output: 3000,
                cache_read: 4000,
                cache_write: 500,
                reasoning: 400,
            },
        ];
        f.store.append_run(pr_row).unwrap();
        assert_eq!(f.store.runs().unwrap().len(), 2);

        // Planner with plan_review enabled allows plan and review stages, and refuses build.
        let mut planner_task_val: serde_json::Value = serde_json::from_str(TASK).unwrap();
        planner_task_val["role"] = json!("planner");
        let planner_task = f
            .store
            .put(&serde_json::to_vec(&planner_task_val).unwrap())
            .unwrap()
            .hash;

        let mut planner_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        planner_pr_val["role"] = json!("planner");
        planner_pr_val["plan_review"] = json!({
            "reviewer_config": rev_jail,
            "max_rounds": 2
        });
        let planner_pr = f
            .store
            .put(&serde_json::to_vec(&planner_pr_val).unwrap())
            .unwrap()
            .hash;

        let mut planner_pr_row = run(&planner_pr, &planner_task, 1, "plan-pr-ok");
        planner_pr_row.stage = Stage::Review;
        planner_pr_row.events.push(RunEvent {
            kind: RunEventKind::Handoff,
            stage: Stage::Review,
            at: Utc::new("2026-12-01T10:15:00Z").unwrap(),
            reason: Text::new("plan review handoff").unwrap(),
            context_tokens: Some(150_000),
            resumed: true,
        });
        planner_pr_row.tokens = vec![
            StageTokens {
                stage: Stage::Plan,
                round: 1,
                input: 3000,
                output: 900,
                cache_read: 0,
                cache_write: 0,
                reasoning: 100,
            },
            StageTokens {
                stage: Stage::Review,
                round: 1,
                input: 3500,
                output: 700,
                cache_read: 0,
                cache_write: 0,
                reasoning: 150,
            },
        ];
        planner_pr_row.outcome = Outcome::Planner { completed: true };
        f.store.append_run(planner_pr_row.clone()).unwrap();

        let mut planner_build = planner_pr_row;
        planner_build.key.repeat = 2;
        planner_build.run_id = Text::new("plan-pr-build").unwrap();
        planner_build.stage = Stage::Build;
        let err = f.store.append_run(planner_build).unwrap_err().0;
        assert!(err.contains("stage build is not allowed"), "{err}");

        // Planner without plan_review refuses review stage.
        let mut planner_plain_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        planner_plain_val["role"] = json!("planner");
        let planner_plain = f
            .store
            .put(&serde_json::to_vec(&planner_plain_val).unwrap())
            .unwrap()
            .hash;
        let mut planner_plain_review = run(&planner_plain, &planner_task, 1, "plan-no-pr-rev");
        planner_plain_review.stage = Stage::Review;
        planner_plain_review.tokens[0].stage = Stage::Plan;
        planner_plain_review.outcome = Outcome::Planner { completed: true };
        let err = f.store.append_run(planner_plain_review).unwrap_err().0;
        assert!(err.contains("stage review is not allowed"), "{err}");

        // Historical `/1` judge planted on disk cannot append new grade rows.
        let live_run = f.store.live_runs().unwrap().remove(0);
        let judge_v1 = crate::record::hash(JUDGE_V1.as_bytes()).unwrap();
        f.store
            .write_canonical("judges", &judge_v1.hash, &judge_v1.canonical)
            .unwrap();
        let mut v1_grade = grade(&f, &live_run, "g-v1");
        v1_grade.key.judge_config = Digest::new(judge_v1.hash).unwrap();
        let err = f.store.append_grade(v1_grade).unwrap_err().0;
        assert!(
            err.contains("eon-bench-config/1") && err.contains("eon-bench-config/2"),
            "{err}"
        );

        // Review token rounds above `plan_review.max_rounds` are refused.
        let mut capped_pr_val: serde_json::Value = serde_json::from_str(CONFIG).unwrap();
        capped_pr_val["plan_review"] = json!({
            "reviewer_config": rev_jail,
            "max_rounds": 1
        });
        let capped_pr = f
            .store
            .put(&serde_json::to_vec(&capped_pr_val).unwrap())
            .unwrap()
            .hash;
        let mut over_cap = run(&capped_pr, &f.task, 1, "pr-over-cap");
        over_cap.tokens = vec![
            StageTokens {
                stage: Stage::Review,
                round: 1,
                input: 100,
                output: 50,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            StageTokens {
                stage: Stage::Review,
                round: 2,
                input: 100,
                output: 50,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
        ];
        let err = f.store.append_run(over_cap).unwrap_err().0;
        assert!(err.contains("exceeds plan_review.max_rounds"), "{err}");

        // Zero-F2P task still refuses resolved: true when f2p_total == 0.
        let mut zero_f2p_val: serde_json::Value = serde_json::from_str(TASK).unwrap();
        zero_f2p_val["key"]["f2p"] = json!([]);
        let zero_f2p_task = f
            .store
            .put(&serde_json::to_vec(&zero_f2p_val).unwrap())
            .unwrap()
            .hash;
        let mut zero_f2p_row = run(&f.config, &zero_f2p_task, 1, "r-zero-f2p");
        zero_f2p_row.outcome = Outcome::Builder {
            resolved: true,
            f2p_passed: 0,
            f2p_total: 0,
            p2p_passed: 1,
            p2p_total: 1,
            protected_paths_touched: vec![],
        };
        let err = f.store.append_run(zero_f2p_row).unwrap_err().0;
        assert!(err.contains("outcome.resolved cannot be true"), "{err}");

        // Two-F2P/two-P2P task refuses 1/1 totals on both append_run and Store::runs().
        let g = fixture();
        let mut two_test_val: serde_json::Value = serde_json::from_str(TASK).unwrap();
        two_test_val["key"]["f2p"] = json!(["t::f2p_1", "t::f2p_2"]);
        two_test_val["key"]["p2p"] = json!(["t::p2p_1", "t::p2p_2"]);
        let two_test_task = g
            .store
            .put(&serde_json::to_vec(&two_test_val).unwrap())
            .unwrap()
            .hash;
        let bogus_totals = run(&g.config, &two_test_task, 1, "r-1of1");
        let err = g.store.append_run(bogus_totals.clone()).unwrap_err().0;
        assert!(err.contains("key.f2p count (2)"), "{err}");

        // Hand-appended row claiming 1/1 on the 2-test task fails on read as well.
        g.store
            .append_line(RUNS, &serde_json::to_string(&bogus_totals).unwrap())
            .unwrap();
        let err = g.store.runs().unwrap_err().0;
        assert!(err.contains("key.f2p count (2)"), "{err}");

        // Positional array for nested struct in hand-appended results.jsonl fails on read.
        let h = fixture();
        let mut bad_row_json = serde_json::to_value(run(&h.config, &h.task, 1, "r-arr")).unwrap();
        bad_row_json["load"] = json!([1.5, 1.25, 1.0]);
        h.store
            .append_line(RUNS, &serde_json::to_string(&bad_row_json).unwrap())
            .unwrap();
        let err = h.store.runs().unwrap_err().0;
        assert!(err.contains("expected a JSON object"), "{err}");
    }
}
