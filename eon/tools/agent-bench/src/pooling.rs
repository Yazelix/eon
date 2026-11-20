//! Pooling declarations: rows under two different hashes count as one cell only when a
//! declaration says the change between them does not alter what the agent sees.
//!
//! A declaration (`eon-bench-pooling/1`) lists old → new hash pairs of one kind, the changed
//! field paths (computed here, never typed in), the reason, who signed it and when. Only these
//! fields may change:
//!
//! - configurations: the infrastructure fields of the bench plan's Q4b ([`CONFIG_FIELDS`]);
//! - tasks: `scorer.commit` (the micro tier's extension, [`TASK_FIELDS`]).
//!
//! A declaration is written before the new hash has any row, under the store lock. Loading
//! re-checks everything that stays true for its whole life (records, kinds, the diff, the
//! allowed fields) and that no row under a new hash started before the declaration.
//!
//! Judge configurations never pool: a changed judge is a new judge.

use crate::diff::diff;
use crate::record::{Digest, Kind, Text, Utc};
use crate::rows::RunRow;
use crate::store::Store;
use crate::{Result, fail, jcs, sha256_hex, time};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const POOLING_SCHEMA: &str = "eon-bench-pooling/1";
pub const CONFIG_FIELDS: &[&str] = &[
    "bench.commit",
    "tools.heavy_sh_sha256",
    "tools.jail_profile_sha256",
    "tools.rota_commit",
    "tools.rota_backend_sha256",
];
pub const TASK_FIELDS: &[&str] = &["scorer.commit"];
const DIR: &str = "pooling";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolKind {
    Config,
    Task,
}

impl PoolKind {
    fn record_kind(self) -> Kind {
        match self {
            Self::Config => Kind::Config,
            Self::Task => Kind::Task,
        }
    }

    fn allowed(self) -> &'static [&'static str] {
        match self {
            Self::Config => CONFIG_FIELDS,
            Self::Task => TASK_FIELDS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub old: Digest,
    pub new: Digest,
    pub changed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub schema: Text,
    pub kind: PoolKind,
    pub pairs: Vec<Pair>,
    pub reason: Text,
    pub signed_by: Text,
    pub declared_at: Utc,
}

impl Store {
    /// Writes a declaration for `pairs` (old, new), returning its hash.
    pub fn declare(
        &self,
        kind: PoolKind,
        pairs: &[(String, String)],
        reason: &str,
        signed_by: &str,
    ) -> Result<(String, Declaration)> {
        let _lock = self.lock()?;
        let mut built = Vec::new();
        for (old, new) in pairs {
            let old = Digest::new(old.clone())?;
            let new = Digest::new(new.clone())?;
            let changed = self.changed(kind, &old, &new)?;
            built.push(Pair { old, new, changed });
        }
        let declaration = Declaration {
            schema: Text::new(POOLING_SCHEMA)?,
            kind,
            pairs: built,
            reason: Text::new(reason)?,
            signed_by: Text::new(signed_by)?,
            declared_at: Utc::new(time::now_utc())?,
        };
        self.check_declaration(&declaration)?;
        let runs = self.runs()?;
        for pair in &declaration.pairs {
            if runs.iter().any(|run| uses(kind, run, &pair.new)) {
                fail!(
                    "{} already has rows; declare pooling before the next tier runs",
                    crate::short(pair.new.as_str())
                );
            }
        }
        let canonical = jcs::canonical(&serde_json::to_value(&declaration)?)?;
        let hash = sha256_hex(canonical.as_bytes());
        self.write_canonical(DIR, &hash, &canonical)?;
        Ok((hash, declaration))
    }

    /// Every stored declaration, re-checked.
    pub fn declarations(&self) -> Result<Vec<Declaration>> {
        let mut out = Vec::new();
        for hash in self.hashes(DIR)? {
            let bytes = self.read_canonical(DIR, &hash)?;
            let declaration: Declaration = serde_json::from_slice(&bytes)
                .map_err(|error| crate::Error(format!("{DIR}/{hash}.json: {error}")))?;
            self.check_declaration(&declaration)
                .map_err(|error| crate::Error(format!("{DIR}/{hash}.json: {error}")))?;
            out.push(declaration);
        }
        Ok(out)
    }

    /// The live, non-drifted runs for (config, task), including runs under hashes declared
    /// equivalent to either, ordered by repeat. Two pooled rows for one repeat are an error.
    ///
    /// A row under a declared `new` hash that started before that declaration (a run in
    /// flight when it was written, or clock skew between hosts) is not pooled: pooling is
    /// declared before the runs it covers. It is returned in `excluded` instead, so it is
    /// never dropped silently, and stays a live row of its own hash.
    pub fn pooled(&self, config: &str, task: &str) -> Result<Pooled> {
        let declarations = self.declarations()?;
        let configs = class(&declarations, PoolKind::Config, config);
        let tasks = class(&declarations, PoolKind::Task, task);
        let (mut rows, mut excluded): (Vec<RunRow>, Vec<RunRow>) = self
            .live_runs()?
            .into_iter()
            .filter(|run| {
                !run.config_drift
                    && configs.contains(run.key.config.as_str())
                    && tasks.contains(run.key.task.as_str())
            })
            .partition(|run| !predates_a_declaration(&declarations, run));
        rows.sort_by_key(|run| run.key.repeat);
        excluded.sort_by_key(|run| run.key.repeat);
        for pair in rows.windows(2) {
            if pair[0].key.repeat == pair[1].key.repeat {
                fail!(
                    "repeat {} has two pooled rows: {} and {}",
                    pair[0].key.repeat,
                    pair[0].run_id,
                    pair[1].run_id
                );
            }
        }
        Ok(Pooled { rows, excluded })
    }

    fn changed(&self, kind: PoolKind, old: &Digest, new: &Digest) -> Result<Vec<String>> {
        let old = self
            .get(kind.record_kind(), old.as_str())?
            .record
            .to_value();
        let new = self
            .get(kind.record_kind(), new.as_str())?
            .record
            .to_value();
        Ok(diff(&old, &new)
            .into_iter()
            .map(|change| change.path)
            .collect())
    }

    fn check_declaration(&self, declaration: &Declaration) -> Result<()> {
        if declaration.schema.as_str() != POOLING_SCHEMA {
            fail!("schema {} is not {POOLING_SCHEMA}", declaration.schema);
        }
        if declaration.pairs.is_empty() {
            fail!("a declaration names at least one old/new pair");
        }
        let kind = declaration.kind;
        for pair in &declaration.pairs {
            let changed = self.changed(kind, &pair.old, &pair.new)?;
            if changed.is_empty() {
                fail!("{} and {} are the same record", pair.old, pair.new);
            }
            if changed != pair.changed {
                fail!("the recorded field diff does not match the records");
            }
            let outside: Vec<&String> = changed
                .iter()
                .filter(|path| !kind.allowed().contains(&path.as_str()))
                .collect();
            if !outside.is_empty() {
                fail!(
                    "only {} may change between pooled hashes; this changes {}",
                    kind.allowed().join(", "),
                    outside
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        Ok(())
    }
}

/// The result of [`Store::pooled`].
#[derive(Debug, Serialize)]
pub struct Pooled {
    /// The pooled rows, ordered by repeat.
    pub rows: Vec<RunRow>,
    /// Rows in the class left out because they started before a declaration that names
    /// their hash as `new`, ordered by repeat.
    pub excluded: Vec<RunRow>,
}

/// Whether `run` started before a declaration that names its config or task hash as `new`.
fn predates_a_declaration(declarations: &[Declaration], run: &RunRow) -> bool {
    declarations.iter().any(|declaration| {
        run.started_at < declaration.declared_at
            && declaration
                .pairs
                .iter()
                .any(|pair| uses(declaration.kind, run, &pair.new))
    })
}

fn uses(kind: PoolKind, run: &RunRow, hash: &Digest) -> bool {
    match kind {
        PoolKind::Config => &run.key.config == hash,
        PoolKind::Task => &run.key.task == hash,
    }
}

/// The hashes connected to `start` through declared pairs of `kind`.
fn class(declarations: &[Declaration], kind: PoolKind, start: &str) -> BTreeSet<String> {
    let edges: Vec<(&str, &str)> = declarations
        .iter()
        .filter(|d| d.kind == kind)
        .flat_map(|d| &d.pairs)
        .map(|pair| (pair.old.as_str(), pair.new.as_str()))
        .collect();
    let mut found = BTreeSet::from([start.to_owned()]);
    loop {
        let before = found.len();
        for (a, b) in &edges {
            if found.contains(*a) || found.contains(*b) {
                found.insert((*a).to_owned());
                found.insert((*b).to_owned());
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::{CONFIG, JUDGE, TASK};
    use crate::rows::tests::{fixture, run};
    use serde_json::{Value, json};

    fn put_edited(store: &Store, fixture: &str, edit: impl Fn(&mut Value)) -> String {
        let mut value: Value = serde_json::from_str(fixture).unwrap();
        edit(&mut value);
        store
            .put(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .hash
    }

    fn repeats(rows: &[RunRow]) -> Vec<u64> {
        rows.iter().map(|row| row.key.repeat).collect()
    }

    #[test]
    fn repeat_three_pools_with_one_and_two() {
        let f = fixture();
        f.store
            .append_run(run(&f.config, &f.task, 1, "micro-1"))
            .unwrap();
        f.store
            .append_run(run(&f.config, &f.task, 2, "micro-2"))
            .unwrap();
        assert_eq!(
            repeats(&f.store.pooled(&f.config, &f.task).unwrap().rows),
            [1, 2]
        );
        // Mini adds repeat 3 under the same hashes, later: no declaration needed.
        let mut later = run(&f.config, &f.task, 3, "mini-3");
        later.started_at = Utc::new("2026-12-08T10:00:00Z").unwrap();
        later.finished_at = Utc::new("2026-12-08T10:50:00Z").unwrap();
        f.store.append_run(later).unwrap();
        let pooled = f.store.pooled(&f.config, &f.task).unwrap().rows;
        assert_eq!(repeats(&pooled), [1, 2, 3]);
        assert_eq!(pooled[2].run_id.as_str(), "mini-3");
    }

    #[test]
    fn superseded_and_drifted_rows_do_not_pool() {
        let f = fixture();
        f.store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        let mut rerun = run(&f.config, &f.task, 1, "r1b");
        rerun.supersedes = Some(Text::new("r1").unwrap());
        f.store.append_run(rerun).unwrap();
        let mut drifted = run(&f.config, &f.task, 2, "r2");
        drifted.server_model_id = Text::new("other-model").unwrap();
        f.store.append_run(drifted).unwrap();
        let pooled = f.store.pooled(&f.config, &f.task).unwrap().rows;
        assert_eq!(pooled.len(), 1);
        assert_eq!(pooled[0].run_id.as_str(), "r1b");
    }

    #[test]
    fn declared_infrastructure_changes_pool() {
        let f = fixture();
        f.store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        let new_config = put_edited(&f.store, CONFIG, |v| {
            v["bench"]["commit"] = json!("3".repeat(40));
            v["tools"]["heavy_sh_sha256"] = json!("7".repeat(64));
        });
        let new_task = put_edited(&f.store, TASK, |v| {
            v["scorer"]["commit"] = json!("4".repeat(40))
        });
        let (_, config_decl) = f
            .store
            .declare(
                PoolKind::Config,
                &[(f.config.clone(), new_config.clone())],
                "bench fix that the agent never sees",
                "eon-lead",
            )
            .unwrap();
        assert_eq!(
            config_decl.pairs[0].changed,
            ["bench.commit", "tools.heavy_sh_sha256"]
        );
        f.store
            .declare(
                PoolKind::Task,
                &[(f.task.clone(), new_task.clone())],
                "scorer fix",
                "eon-lead",
            )
            .unwrap();
        // Rows appended under the new hashes after declaring still pool, after a reload too.
        let mut later = run(&new_config, &new_task, 2, "r2");
        later.started_at = Utc::new("2099-01-01T00:00:00Z").unwrap();
        later.finished_at = Utc::new("2099-01-01T01:00:00Z").unwrap();
        f.store.append_run(later).unwrap();
        let reloaded = Store::new(f.dir.path());
        assert_eq!(
            repeats(&reloaded.pooled(&f.config, &f.task).unwrap().rows),
            [1, 2]
        );
        assert_eq!(
            repeats(&reloaded.pooled(&new_config, &new_task).unwrap().rows),
            [1, 2]
        );
        // The same repeat under both hashes is refused rather than picked.
        let mut clash = run(&new_config, &new_task, 1, "r1-again");
        clash.started_at = Utc::new("2099-01-02T00:00:00Z").unwrap();
        clash.finished_at = Utc::new("2099-01-02T01:00:00Z").unwrap();
        f.store.append_run(clash).unwrap();
        assert!(reloaded.pooled(&f.config, &f.task).is_err());
    }

    #[test]
    fn a_run_in_flight_at_declaration_is_excluded_not_fatal() {
        let f = fixture();
        f.store
            .append_run(run(&f.config, &f.task, 1, "r1"))
            .unwrap();
        let new_config = put_edited(&f.store, CONFIG, |v| {
            v["bench"]["commit"] = json!("3".repeat(40))
        });
        f.store
            .declare(
                PoolKind::Config,
                &[(f.config.clone(), new_config.clone())],
                "bench fix",
                "eon-lead",
            )
            .unwrap();
        // Started under the new hash before the declaration, appended after it.
        let mut in_flight = run(&new_config, &f.task, 2, "r2");
        in_flight.started_at = Utc::new("2026-01-01T00:00:00Z").unwrap();
        in_flight.finished_at = Utc::new("2026-01-01T01:00:00Z").unwrap();
        f.store.append_run(in_flight).unwrap();
        let mut later = run(&new_config, &f.task, 3, "r3");
        later.started_at = Utc::new("2099-01-01T00:00:00Z").unwrap();
        later.finished_at = Utc::new("2099-01-01T01:00:00Z").unwrap();
        f.store.append_run(later).unwrap();

        let reloaded = Store::new(f.dir.path());
        assert_eq!(reloaded.declarations().unwrap().len(), 1);
        for config in [&f.config, &new_config] {
            let pooled = reloaded.pooled(config, &f.task).unwrap();
            assert_eq!(repeats(&pooled.rows), [1, 3]);
            assert_eq!(repeats(&pooled.excluded), [2]);
        }
        assert!(
            reloaded
                .live_runs()
                .unwrap()
                .iter()
                .any(|row| row.run_id.as_str() == "r2")
        );
    }

    #[test]
    fn declarations_are_refused_outside_the_rule() {
        let f = fixture();
        let model_change = put_edited(&f.store, CONFIG, |v| v["effort"]["value"] = json!("max"));
        let task_change = put_edited(&f.store, TASK, |v| v["wall_cap_s"] = json!(7200));
        let refuse = |kind, old: &str, new: &str| {
            f.store
                .declare(kind, &[(old.to_owned(), new.to_owned())], "r", "eon-lead")
                .is_err()
        };
        assert!(refuse(PoolKind::Config, &f.config, &model_change));
        assert!(refuse(PoolKind::Task, &f.task, &task_change));
        assert!(refuse(PoolKind::Config, &f.config, &f.config));
        assert!(refuse(PoolKind::Config, &f.config, &f.task));
        // Two judge configurations differing only in an allowed field: judges never pool.
        let other_judge = put_edited(&f.store, JUDGE, |v| {
            v["bench"]["commit"] = json!("8".repeat(40))
        });
        assert!(refuse(PoolKind::Config, &f.judge, &other_judge));
        assert!(
            f.store
                .declare(PoolKind::Config, &[], "r", "eon-lead")
                .is_err()
        );

        // A new hash that already ran cannot be declared after the fact.
        let ran = put_edited(&f.store, CONFIG, |v| {
            v["bench"]["commit"] = json!("5".repeat(40))
        });
        f.store.append_run(run(&ran, &f.task, 1, "early")).unwrap();
        assert!(refuse(PoolKind::Config, &f.config, &ran));

        // A hand-widened declaration fails to load.
        let fine = put_edited(&f.store, CONFIG, |v| {
            v["bench"]["commit"] = json!("6".repeat(40))
        });
        let (hash, mut declaration) = f
            .store
            .declare(
                PoolKind::Config,
                &[(f.config.clone(), fine)],
                "r",
                "eon-lead",
            )
            .unwrap();
        assert_eq!(f.store.declarations().unwrap().len(), 1);
        declaration.pairs[0].new = Digest::new(model_change).unwrap();
        let canonical = jcs::canonical(&serde_json::to_value(&declaration).unwrap()).unwrap();
        let widened = sha256_hex(canonical.as_bytes());
        f.store.write_canonical(DIR, &widened, &canonical).unwrap();
        assert!(f.store.declarations().is_err());
        assert_ne!(hash, widened);
    }
}
