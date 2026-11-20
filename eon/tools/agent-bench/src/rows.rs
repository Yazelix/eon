//! Append-only run and grade rows.
//!
//! A run row is keyed by (config hash, task hash, repeat); a grade row by (run key, judge
//! config hash). Everything else on a row sits beside the key and outside every hash. A rerun
//! adds a row that names the row it replaces in `supersedes`; the old row stays in the file and
//! drops out of every reader. At most one row per key is live.
//!
//! `config_drift` is set by [`Store::append_run`] / [`Store::append_grade`], never by the
//! caller: it is true when the server-reported model id differs from the configuration's
//! `model.id`. Loading recomputes it and refuses a row whose flag disagrees. Drifted rows are
//! kept but never pool.

use crate::record::{Digest, Text, Utc};
use crate::store::Store;
use crate::{Result, fail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const RUN_SCHEMA: &str = "eon-bench-run/1";
pub const GRADE_SCHEMA: &str = "eon-bench-grade/1";
const RUNS: &str = "results.jsonl";
const GRADES: &str = "grades.jsonl";

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
    if [load.one, load.five, load.fifteen]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        fail!("load averages must be finite and non-negative");
    }
    Ok(())
}

impl Store {
    /// Every run row, validated, in file order.
    pub fn runs(&self) -> Result<Vec<RunRow>> {
        let mut models = HashMap::new();
        let mut rows = Vec::new();
        for (index, line) in self.lines(RUNS)?.iter().enumerate() {
            let row: RunRow = serde_json::from_str(line)
                .map_err(|error| crate::Error(format!("{RUNS}:{}: {error}", index + 1)))?;
            self.check_run(&row, &mut models)
                .and_then(|()| admit(&rows, &row))
                .map_err(|error| crate::Error(format!("{RUNS}:{}: {error}", index + 1)))?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// Every grade row, validated, in file order.
    pub fn grades(&self) -> Result<Vec<GradeRow>> {
        let mut models = HashMap::new();
        let mut rows = Vec::new();
        for (index, line) in self.lines(GRADES)?.iter().enumerate() {
            let row: GradeRow = serde_json::from_str(line)
                .map_err(|error| crate::Error(format!("{GRADES}:{}: {error}", index + 1)))?;
            self.check_grade(&row, &mut models)
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
        self.check_run(&row, &mut HashMap::new())?;
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
        self.check_grade(&row, &mut HashMap::new())?;
        admit(&self.grades()?, &row)?;
        self.append_line(GRADES, &serde_json::to_string(&row)?)?;
        Ok(row)
    }

    fn check_run(&self, row: &RunRow, models: &mut HashMap<String, String>) -> Result<()> {
        if row.schema.as_str() != RUN_SCHEMA {
            fail!("schema {} is not {RUN_SCHEMA}", row.schema);
        }
        if row.key.repeat == 0 {
            fail!("repeat counts from 1");
        }
        if row
            .credits
            .is_some_and(|credits| !credits.is_finite() || credits < 0.0)
        {
            fail!("credits must be finite and non-negative");
        }
        check_times(&row.started_at, &row.finished_at, &row.load)?;
        let model = self.model_id(models, row.key.config.as_str(), false)?;
        if row.config_drift != (row.server_model_id.as_str() != model) {
            fail!("config_drift disagrees with the configuration's model id");
        }
        Ok(())
    }

    fn check_grade(&self, row: &GradeRow, models: &mut HashMap<String, String>) -> Result<()> {
        if row.schema.as_str() != GRADE_SCHEMA {
            fail!("schema {} is not {GRADE_SCHEMA}", row.schema);
        }
        check_times(&row.started_at, &row.finished_at, &row.load)?;
        let model = self.model_id(models, row.key.judge_config.as_str(), true)?;
        if row.config_drift != (row.server_model_id.as_str() != model) {
            fail!("config_drift disagrees with the judge configuration's model id");
        }
        Ok(())
    }

    fn model_id(
        &self,
        cache: &mut HashMap<String, String>,
        hash: &str,
        judge: bool,
    ) -> Result<String> {
        if let Some(model) = cache.get(hash) {
            return Ok(model.clone());
        }
        let config = if judge {
            self.judge(hash)?
        } else {
            self.config(hash)?
        };
        let model = config.model.id.as_str().to_owned();
        cache.insert(hash.to_owned(), model.clone());
        Ok(model)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::tests::{CONFIG, JUDGE, TASK};
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
            "config_drift": false
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
}
