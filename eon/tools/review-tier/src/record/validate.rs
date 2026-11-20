//! What `render` accepts: public values from `record-policy.toml` only, and
//! exit statuses recorded as they were. Nothing here judges whether a
//! landing showed enough evidence or the right reviewers; the close gate
//! (eon-3n73k.4) does that.
use super::model::{Proof, Record, Review};
use crate::Result;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const TEXT: &str = include_str!("../../record-policy.toml");

const TIERS: [&str; 3] = ["high", "medium", "low"];
const STAGES: [&str; 2] = ["plan", "code"];
const VERDICTS: [&str; 2] = ["approve", "changes-needed"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    efforts: Vec<String>,
    platforms: Vec<String>,
    routes: Vec<String>,
    attrs: Vec<String>,
    checks: BTreeMap<String, TargetKind>,
    harness: BTreeMap<String, Harness>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Harness {
    models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TargetKind {
    /// A workspace package name, or `workspace`.
    Package,
    /// One of the policy's flake attributes.
    Attr,
    /// No target.
    None,
}

impl Policy {
    /// The policy compiled into this tool.
    pub fn load() -> Result<Self> {
        toml::from_str(TEXT).map_err(|error| format!("record-policy.toml: {error}").into())
    }
}

/// What validation needs from the repository and the tracker.
#[derive(Debug, Default)]
pub struct Facts {
    /// Workspace package names in `candidate.tree`, where `candidate`
    /// proofs ran.
    pub candidate_packages: BTreeSet<String>,
    /// Workspace package names in `landing.proof_tree`, where `landed`
    /// proofs ran.
    pub landed_packages: BTreeSet<String>,
    /// Whether each bead the record names is open; absent means it is not in
    /// the tracker.
    pub beads: BTreeMap<String, bool>,
}

/// Bead IDs the record names, which the caller looks up for `Facts::beads`.
pub fn named_beads(record: &Record) -> BTreeSet<String> {
    record
        .proof
        .iter()
        .filter_map(|proof| proof.known_failure.clone())
        .collect()
}

/// Object IDs that must be full hex before anything asks git about them.
pub fn id_problems(record: &Record) -> Vec<String> {
    [
        ("landing.base", &record.landing.base),
        ("landing.proof_tree", &record.landing.proof_tree),
        ("candidate.base", &record.candidate.base),
        ("candidate.tree", &record.candidate.tree),
    ]
    .into_iter()
    .filter(|(_, id)| !is_hex(id))
    .map(|(name, id)| format!("{name} {id:?} is not a full 40-hex object ID"))
    .collect()
}

fn is_hex(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Every problem with `record`; empty when it is valid.
pub fn problems(record: &Record, policy: &Policy, facts: &Facts) -> Vec<String> {
    let mut problems = id_problems(record);
    if !policy.routes.contains(&record.route) {
        problems.push(format!("unknown route {:?}", record.route));
    }
    if !TIERS.contains(&record.tier.as_str()) {
        problems.push(format!("unknown tier {:?}", record.tier));
    }
    for (index, proof) in record.proof.iter().enumerate() {
        for problem in proof_problems(proof, policy, facts) {
            problems.push(format!("proof {}: {problem}", index + 1));
        }
    }
    if record.builder.is_empty() {
        problems.push("no builder".into());
    }
    for (index, session) in record.builder.iter().enumerate() {
        let name = format!("builder {}", index + 1);
        problems.extend(session_problems(
            &name,
            policy,
            [&session.harness, &session.harness_version, &session.model],
        ));
        if !policy.efforts.contains(&session.effort) {
            problems.push(format!("{name}: unknown effort {:?}", session.effort));
        }
    }
    for (index, review) in record.review.iter().enumerate() {
        problems.extend(review_problems(
            &format!("review {}", index + 1),
            review,
            policy,
        ));
    }
    problems
}

fn proof_problems(proof: &Proof, policy: &Policy, facts: &Facts) -> Vec<String> {
    let mut problems = Vec::new();
    // A proof's package must exist in the tree it ran on, so a landing may
    // name a crate it adds.
    let packages = match proof.on.as_str() {
        "candidate" => Some(&facts.candidate_packages),
        "landed" => Some(&facts.landed_packages),
        _ => {
            problems.push(format!(
                "`on` must be candidate or landed, not {:?}",
                proof.on
            ));
            None
        }
    };
    if !policy.platforms.contains(&proof.platform) {
        problems.push(format!("unknown platform {:?}", proof.platform));
    }
    match (policy.checks.get(&proof.check), proof.target.as_deref()) {
        (None, _) => problems.push(format!("unknown check {:?}", proof.check)),
        (Some(TargetKind::None), None) => {}
        (Some(TargetKind::None), Some(target)) => {
            problems.push(format!("{} takes no target, got {target:?}", proof.check));
        }
        (Some(_), None) => problems.push(format!("{} needs a target", proof.check)),
        (Some(TargetKind::Package), Some(target)) => {
            if let Some(packages) = packages
                && target != "workspace"
                && !packages.contains(target)
            {
                problems.push(format!(
                    "{target:?} is not a workspace package in the {} tree",
                    proof.on
                ));
            }
        }
        (Some(TargetKind::Attr), Some(target)) => {
            if !policy.attrs.iter().any(|attr| attr == target) {
                problems.push(format!("{target:?} is not a listed flake attribute"));
            }
        }
    }
    // The exit status is the record; nothing may claim success beside it.
    match (proof.passed, proof.failed, proof.skipped) {
        (None, None, None) => {
            if proof.check == "eon-test" && proof.exit == 0 {
                problems.push("a passing eon-test needs nextest's counts".into());
            }
        }
        (Some(_), Some(failed), Some(_)) => {
            if proof.check != "eon-test" {
                problems.push("only eon-test has counts".into());
            }
            if (proof.exit == 0) != (failed == 0) {
                problems.push(format!(
                    "exit {} with {failed} failed: the counts and the exit disagree",
                    proof.exit
                ));
            }
        }
        _ => problems.push("`passed`, `failed` and `skipped` go together".into()),
    }
    if let Some(bead) = &proof.known_failure {
        if proof.exit == 0 {
            problems.push("`known_failure` is only for a nonzero exit".into());
        }
        match facts.beads.get(bead) {
            None => problems.push(format!("known failure {bead} is not in the tracker")),
            Some(false) => problems.push(format!("known failure {bead} is not open")),
            Some(true) => {}
        }
    }
    problems
}

fn session_problems(
    name: &str,
    policy: &Policy,
    [harness, version, model]: [&String; 3],
) -> Vec<String> {
    let mut problems = Vec::new();
    match policy.harness.get(harness) {
        None => problems.push(format!("{name}: unknown harness {harness:?}")),
        Some(listed) if !listed.models.contains(model) => {
            problems.push(format!("{name}: {model:?} is not a listed {harness} model"));
        }
        Some(_) => {}
    }
    let parts: Vec<&str> = version.split('.').collect();
    let numeric = parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 6 && part.bytes().all(|b| b.is_ascii_digit())
        });
    if !numeric {
        problems.push(format!(
            "{name}: harness_version {version:?} is not digits.digits.digits"
        ));
    }
    problems
}

fn review_problems(name: &str, review: &Review, policy: &Policy) -> Vec<String> {
    let mut problems = session_problems(
        name,
        policy,
        [&review.harness, &review.harness_version, &review.model],
    );
    if !STAGES.contains(&review.stage.as_str()) {
        problems.push(format!("{name}: stage must be plan or code"));
    }
    if review.rounds.is_empty() {
        problems.push(format!("{name}: no rounds"));
    }
    for round in &review.rounds {
        if !VERDICTS.contains(&round.verdict.as_str()) {
            problems.push(format!("{name}: unknown verdict {:?}", round.verdict));
        }
        if !policy.efforts.contains(&round.effort) {
            problems.push(format!("{name}: unknown effort {:?}", round.effort));
        }
    }
    let first = review.rounds.first().map(|round| round.verdict.as_str());
    if review.round1_blockers > 0 && first == Some("approve") {
        problems.push(format!(
            "{name}: round-1 blockers with a first verdict `approve`"
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::super::model::tests::input;
    use super::*;

    fn facts() -> Facts {
        let packages: BTreeSet<String> = ["eon", "eon-runtime"].map(String::from).into();
        Facts {
            candidate_packages: packages.clone(),
            landed_packages: packages,
            beads: BTreeMap::new(),
        }
    }

    fn problems_of(record: &Record, facts: &Facts) -> Vec<String> {
        problems(record, &Policy::load().unwrap(), facts)
    }

    fn assert_problem(record: &Record, facts: &Facts, needle: &str) {
        let found = problems_of(record, facts);
        assert!(
            found.iter().any(|problem| problem.contains(needle)),
            "no problem containing {needle:?} in {found:#?}"
        );
    }

    #[test]
    fn the_shipped_policy_loads_and_the_fixture_is_valid() {
        assert_eq!(problems_of(&input(), &facts()), Vec::<String>::new());
    }

    #[test]
    fn values_outside_the_allowlists_are_refused() {
        let facts = facts();
        let mut record = input();
        record.builder[0].model = "gpt-6-privateaccount".into();
        assert_problem(
            &record,
            &facts,
            "\"gpt-6-privateaccount\" is not a listed claude-code model",
        );
        record.builder[0].model = "gpt-6.2-sol".into();
        assert_problem(
            &record,
            &facts,
            "\"gpt-6.2-sol\" is not a listed claude-code model",
        );
        record.builder[0].harness = "acct-private-a".into();
        assert_problem(&record, &facts, "unknown harness \"acct-private-a\"");
        let mut record = input();
        record.builder[0].harness_version = "2.1.326-acct".into();
        assert_problem(&record, &facts, "is not digits.digits.digits");
        record.builder[0].effort = "turbo".into();
        assert_problem(&record, &facts, "unknown effort \"turbo\"");
        let mut record = input();
        record.proof[1].target = Some(".#private-account".into());
        assert_problem(
            &record,
            &facts,
            "\".#private-account\" is not a listed flake attribute",
        );
        record.proof[0].target = Some("acct-private-a".into());
        assert_problem(
            &record,
            &facts,
            "\"acct-private-a\" is not a workspace package",
        );
        record.proof[0].platform = "my-laptop".into();
        assert_problem(&record, &facts, "unknown platform");
        record.proof[0].check = "custom".into();
        assert_problem(&record, &facts, "unknown check \"custom\"");
        let mut record = input();
        record.route = "anything".into();
        assert_problem(&record, &facts, "unknown route");
        record.tier = "urgent".into();
        assert_problem(&record, &facts, "unknown tier");
        record.landing.base = "abc".into();
        assert_problem(&record, &facts, "landing.base \"abc\" is not a full 40-hex");
        record.candidate.tree = "A".repeat(40);
        assert_problem(&record, &facts, "candidate.tree");
    }

    #[test]
    fn a_failed_check_is_recorded_as_failed() {
        let mut facts = facts();
        let mut record = input();
        // A nonzero exit is a valid record of a failure, with or without a
        // tracking bead.
        record.proof[0].exit = 100;
        record.proof[0].failed = Some(1);
        assert_eq!(problems_of(&record, &facts), Vec::<String>::new());
        assert!(
            record
                .render()
                .contains("exit 100, 10 passed, 1 failed, 1 skipped")
        );
        // Nothing beside the exit may claim success.
        record.proof[0].failed = Some(0);
        assert_problem(&record, &facts, "exit 100 with 0 failed");
        record.proof[0].exit = 0;
        record.proof[0].failed = Some(2);
        assert_problem(&record, &facts, "exit 0 with 2 failed");
        record.proof[0].failed = None;
        assert_problem(&record, &facts, "go together");
        record.proof[0].passed = None;
        record.proof[0].skipped = None;
        assert_problem(&record, &facts, "a passing eon-test needs nextest's counts");
        let mut record = input();
        record.proof[1].passed = Some(1);
        record.proof[1].failed = Some(0);
        record.proof[1].skipped = Some(0);
        assert_problem(&record, &facts, "only eon-test has counts");
        // A known failure names an open bead, and only for a nonzero exit.
        let mut record = input();
        record.proof[1].exit = 1;
        record.proof[1].known_failure = Some("eon-x1".into());
        assert_problem(
            &record,
            &facts,
            "known failure eon-x1 is not in the tracker",
        );
        facts.beads.insert("eon-x1".into(), false);
        assert_problem(&record, &facts, "known failure eon-x1 is not open");
        facts.beads.insert("eon-x1".into(), true);
        assert_eq!(problems_of(&record, &facts), Vec::<String>::new());
        record.proof[1].exit = 0;
        assert_problem(&record, &facts, "only for a nonzero exit");
    }

    #[test]
    fn reviews_and_builders_are_well_formed() {
        let facts = facts();
        let mut record = input();
        record.review[0].rounds[0].verdict = "approve".into();
        assert_problem(
            &record,
            &facts,
            "round-1 blockers with a first verdict `approve`",
        );
        record.review[0].rounds[0].verdict = "lgtm".into();
        assert_problem(&record, &facts, "unknown verdict");
        record.review[0].rounds[0].effort = "turbo".into();
        assert_problem(&record, &facts, "review 1: unknown effort");
        record.review[0].stage = "final".into();
        assert_problem(&record, &facts, "stage must be plan or code");
        record.review[0].rounds.clear();
        assert_problem(&record, &facts, "no rounds");
        let mut record = input();
        record.builder.clear();
        assert_problem(&record, &facts, "no builder");
        // A review that never approved is recorded as it was.
        let mut record = input();
        record.review[0].rounds.pop();
        assert_eq!(problems_of(&record, &facts), Vec::<String>::new());
    }

    #[test]
    fn a_proof_names_packages_of_its_own_tree() {
        let mut facts = facts();
        facts.candidate_packages.insert("only-candidate".into());
        facts.landed_packages.insert("only-landed".into());
        let named = |on: &str, target: &str| {
            let mut record = input();
            record.proof[0].on = on.into();
            record.proof[0].target = Some(target.into());
            problems_of(&record, &facts)
        };
        assert_eq!(named("candidate", "only-candidate"), Vec::<String>::new());
        assert_eq!(named("landed", "only-landed"), Vec::<String>::new());
        assert_eq!(
            named("landed", "only-candidate"),
            ["proof 1: \"only-candidate\" is not a workspace package in the landed tree"]
        );
        assert_eq!(
            named("candidate", "only-landed"),
            ["proof 1: \"only-landed\" is not a workspace package in the candidate tree"]
        );
        for on in ["candidate", "landed"] {
            assert_eq!(
                named(on, "nowhere"),
                [format!(
                    "proof 1: \"nowhere\" is not a workspace package in the {on} tree"
                )]
            );
        }
        // An unknown tree is reported once, with no package verdict beside it.
        assert_eq!(
            named("base", "nowhere"),
            ["proof 1: `on` must be candidate or landed, not \"base\""]
        );
    }
}
