//! Close records: what a landed bead's close stores, in public.
//!
//! The lead closes each landed bead with `br close --reason "Landed; see the
//! close record." --transition-comment "<record>"`, where `<record>` is
//! `review-tier record render <input.toml>`'s output. br writes the comment
//! in the same transaction as the close and exports it to
//! `.beads/issues.jsonl`, which the public repository carries.
//!
//! # Format (`eon-close-record/1`)
//!
//! The comment's first line is `Close record eon-close-record/1`, its last
//! line a compact JSON object, the authoritative copy, and the lines between
//! are rendered from that JSON for readers. `record check` re-renders the
//! JSON and requires the same text, so the readable lines cannot drift.
//!
//! The JSON holds `schema`, `route` (a proof route of docs/AGENT-RULES.md
//! "Verification"), `tier` (the landing's review tier), and:
//!
//! - `landing.base`: the edge commit the landing applies onto, which is the
//!   landing commit's parent. A commit cannot contain its own hash, and the
//!   landing commit carries this record, so the record names its parent and
//!   `record check` finds the landing commit as that parent's first-parent
//!   child.
//! - `landing.source`: `src-1:` + SHA-256 of `git ls-tree -r -z --full-tree`
//!   of the tree the lead's checks ran on, without the `.beads/issues.jsonl`
//!   entry. The landing commit differs from that tree only in the tracker
//!   export, so any clone recomputes the digest from committed data.
//! - `landing.proof_tree` and `candidate.tree`: the trees the lead's and the
//!   builder's checks ran on. `render` requires the proof tree to carry the
//!   reviewed change onto `landing.base` and nothing else: each path the
//!   reviewed change touches changes the same way, down to its `-U0` hunks
//!   in order (line numbers aside, so edge's edits elsewhere in the same
//!   file pass), and no other path outside `.beads/` changes. Nobody
//!   retains either tree afterwards.
//! - `candidate.base`: the builder's base, an ancestor of `landing.base`.
//! - `proof`: each check with the tree it ran on (`candidate` is the
//!   reviewed tree, `landed` the landing's), its check ID and target, the
//!   platform, the exit status as it was, nextest's counts for `eon-test`,
//!   and optionally, for a nonzero exit, the open bead that tracks it.
//! - `builder` and `review`: harness, version, model and effort, each review's
//!   rounds with verdict and effort, round-1 blockers, and a `cfg-p1:` hash.
//!
//! `cfg-p1` is provisional until eon-2r02l defines the canonical
//! configuration hash: SHA-256 of a canonical JSON object holding the role,
//! harness, harness version, model, effort (a review's first round) and the
//! blob IDs of AGENTS.md, REVIEW.md, docs/AGENT-PROTOCOLS.md and
//! docs/AGENT-RULES.md at `candidate.base`. Every input is public, and the
//! record carries all of them, so anyone can recompute it.
//!
//! # What may appear
//!
//! The issues file is public, so a record holds no free text: every string is
//! a value `record-policy.toml` lists, a workspace package name in the tree
//! its proof ran on (so a landing may name a crate it adds), a bead ID present in the tracker, an object ID, or a
//! digits-only version. `render` enforces that against the policy compiled
//! into the tool. `check` reads a landed record for its format and its
//! landing's identity only, so a later policy edit never fails history.
//! Whether a landing showed enough evidence and the right reviewers is the
//! close gate's question (eon-3n73k.4), not this module's.
mod model;
mod repo;
mod validate;

use crate::Result;
use crate::git::Git;
use model::{HEADER, REASON, Record, SCHEMA};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use validate::{Facts, Policy};

/// `record render`: the comment text for the lead's input file, or an error
/// listing every problem.
pub fn render(git: &Git, input: &Path) -> Result<String> {
    let text = fs::read_to_string(input)
        .map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    let mut record: Record =
        toml::from_str(&text).map_err(|error| format!("{}: {error}", input.display()))?;
    if record.has_computed_fields() {
        return Err(
            "the input must not hold schema, landing.source or config: render computes them".into(),
        );
    }
    let ids = validate::id_problems(&record);
    if !ids.is_empty() {
        return Err(ids.join("\n").into());
    }
    let landing = &record.landing;
    let candidate = &record.candidate;
    for (name, id, kind) in [
        ("landing.base", &landing.base, "commit"),
        ("landing.proof_tree", &landing.proof_tree, "tree"),
        ("candidate.base", &candidate.base, "commit"),
        ("candidate.tree", &candidate.tree, "tree"),
    ] {
        let found = repo::object_type(git, id)
            .map_err(|_| format!("{name} {id} is not in this repository"))?;
        if found != kind {
            return Err(format!("{name} {id} is a {found}, not a {kind}").into());
        }
    }
    if !repo::is_ancestor(git, &candidate.base, &landing.base)? {
        return Err(format!(
            "candidate.base {} is not an ancestor of landing.base {}",
            candidate.base, landing.base
        )
        .into());
    }
    let working = fs::read_to_string(git.root().join(repo::TRACKER)).unwrap_or_default();
    let facts = Facts {
        candidate_packages: repo::workspace_packages(git, &candidate.tree)?,
        landed_packages: repo::workspace_packages(git, &landing.proof_tree)?,
        beads: repo::issues(&working, &validate::named_beads(&record))?
            .into_iter()
            .map(|(id, issue)| (id, issue.is_open()))
            .collect(),
    };
    let mut problems = repo::identity_problems(git, &record)?;
    problems.extend(validate::problems(&record, &Policy::load()?, &facts));
    if !problems.is_empty() {
        return Err(problems.join("\n").into());
    }
    let source = repo::source_digest(git, &landing.proof_tree)?;
    let rules = repo::rules_blobs(git, &candidate.base)?;
    record.complete(source, &rules);
    Ok(record.render())
}

/// `record check`: `Ok(Ok(summary))` when `bead`'s newest close record at
/// `revision` is well formed and pins its landing commit, `Ok(Err(findings))`
/// when it is missing, malformed or does not match, `Err` when git fails.
pub fn check(
    git: &Git,
    bead: &str,
    revision: &str,
) -> Result<std::result::Result<String, Vec<String>>> {
    let tip = git.commit(revision)?;
    let ids = BTreeSet::from([bead.to_string()]);
    let at_tip = repo::issues(&repo::tracker_at(git, &tip)?, &ids)?;
    let Some(issue) = at_tip.get(bead) else {
        return finding(format!("{bead} is not in {} at {tip}", repo::TRACKER));
    };
    if issue.is_open() {
        return finding(format!("{bead} is not closed at {tip}"));
    }
    if issue.close_reason.as_deref() != Some(REASON) {
        return finding(format!("{bead}'s close reason is not {REASON:?}"));
    }
    let Some(comment) = issue
        .comments
        .iter()
        .rev()
        .find(|c| c.text.starts_with(HEADER))
    else {
        return finding(format!("{bead} has no close record"));
    };
    let text = comment.text.trim_end();
    let line = text.lines().last().unwrap_or_default();
    let record: Record = match serde_json::from_str(line) {
        Ok(record) => record,
        Err(error) => return finding(format!("the record's JSON line does not parse: {error}")),
    };
    // A missing source or config hash fails the comparisons below.
    if record.schema.as_deref() != Some(SCHEMA) {
        return finding(format!("the record's schema is not {SCHEMA}"));
    }
    if record.render() != text {
        return finding("the comment text is not the rendering of its JSON line".into());
    }
    let id_problems = validate::id_problems(&record);
    if !id_problems.is_empty() {
        return Ok(Err(id_problems));
    }
    let base = &record.landing.base;
    if repo::object_type(git, base).ok().as_deref() != Some("commit") {
        return finding(format!(
            "landing.base {base} is not a commit in this repository"
        ));
    }
    let landed = match repo::landing_commit(git, base, &tip)? {
        Ok(commit) => commit,
        Err(why) => return finding(why),
    };
    let mut findings = Vec::new();
    let at_landing = if landed == tip {
        at_tip.clone()
    } else {
        repo::issues(&repo::tracker_at(git, &landed)?, &ids)?
    };
    match at_landing.get(bead) {
        Some(then) if !then.is_open() && then.comments.iter().any(|c| c.text == comment.text) => {
            if then.closed_at != issue.closed_at {
                findings.push(format!(
                    "{bead} was closed again after its landing commit {landed} without a new record"
                ));
            }
        }
        _ => findings.push(format!(
            "the landing commit {landed} does not close {bead} with this record"
        )),
    }
    let at_base = repo::issues(&repo::tracker_at(git, base)?, &ids)?;
    if let Some(before) = at_base.get(bead) {
        if before.comments.iter().any(|c| c.text == comment.text) {
            findings.push(format!(
                "the record is already at {base}, so {landed} did not introduce it"
            ));
        } else if !before.is_open() && before.closed_at == issue.closed_at {
            findings.push(format!(
                "{bead} was already closed at {base}, so {landed} did not close it"
            ));
        }
    }
    let source = repo::source_digest(git, &git.tree(&landed)?)?;
    if record.landing.source.as_deref() != Some(source.as_str()) {
        findings.push(format!(
            "the landing commit {landed}'s source {source} differs from the proof tree's"
        ));
    }
    let candidate_base = &record.candidate.base;
    if repo::object_type(git, candidate_base).ok().as_deref() != Some("commit")
        || !repo::is_ancestor(git, candidate_base, base)?
    {
        findings.push(format!(
            "candidate.base {candidate_base} is not an ancestor of {base}"
        ));
    } else {
        let mut fresh = record.clone();
        fresh.complete(source, &repo::rules_blobs(git, candidate_base)?);
        fresh.landing.source.clone_from(&record.landing.source);
        if fresh != record {
            findings.push("a config hash does not match its inputs".into());
        }
    }
    if !findings.is_empty() {
        return Ok(Err(findings));
    }
    Ok(Ok(format!(
        "{bead}: landed {landed} onto {base}; route {}, tier {}; source matches; \
         {} proof checks, {} builder sessions, {} reviews",
        record.route,
        record.tier,
        record.proof.len(),
        record.builder.len(),
        record.review.len()
    )))
}

fn finding(text: String) -> Result<std::result::Result<String, Vec<String>>> {
    Ok(Err(vec![text]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A scratch repository, removed on drop.
    struct Repo {
        dir: PathBuf,
        git: Git,
    }

    impl Repo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("review-tier-record-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Git::init(&dir).unwrap();
            let git = Git::at(&dir);
            Self { dir, git }
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn stage_tree(&self) -> String {
            self.git.run(&["add", "-A"]).unwrap();
            String::from_utf8(self.git.run(&["write-tree"]).unwrap())
                .unwrap()
                .trim()
                .to_string()
        }

        fn commit(&self, message: &str) -> String {
            self.git.run(&["add", "-A"]).unwrap();
            self.git
                .run(&[
                    "-c",
                    "user.name=test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    message,
                ])
                .unwrap();
            self.git.rev_parse("HEAD").unwrap()
        }

        fn input(&self, toml: &str) -> PathBuf {
            let path = self.dir.with_extension("toml");
            fs::write(&path, toml).unwrap();
            path
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
            let _ = fs::remove_file(self.dir.with_extension("toml"));
        }
    }

    fn tracker(issues: &[serde_json::Value]) -> String {
        issues.iter().map(|issue| format!("{issue}\n")).collect()
    }

    fn bead(id: &str, status: &str, closed_at: &str, comments: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "title": format!("bead {id}"),
            "status": status,
            "close_reason": if status == "closed" { Some(REASON) } else { None },
            "closed_at": closed_at,
            "comments": comments.iter().map(|text| serde_json::json!({"text": text})).collect::<Vec<_>>(),
        })
    }

    /// The base commit: rules files, a one-crate workspace, an open bead.
    fn base(repo: &Repo) -> String {
        for path in ["AGENTS.md", "REVIEW.md", "docs/AGENT-PROTOCOLS.md"] {
            repo.write(path, &format!("{path}\n"));
        }
        repo.write("docs/AGENT-RULES.md", "rules\n");
        repo.write("Cargo.toml", &workspace(&["crate-a"]));
        repo.write("crate-a/Cargo.toml", "[package]\nname = \"crate-a\"\n");
        repo.write("crate-a/src/lib.rs", "pub fn one() {}\n");
        repo.write(repo::TRACKER, &tracker(&[bead("t-1", "open", "", &[])]));
        repo.commit("base")
    }

    /// A workspace manifest, one member per line, so changes that add
    /// different members touch different lines.
    fn workspace(members: &[&str]) -> String {
        let lines: String = members
            .iter()
            .map(|member| format!("    \"{member}\",\n"))
            .collect();
        format!("[workspace]\nmembers = [\n{lines}]\n")
    }

    /// A step adding the crate `name` as the last workspace member after
    /// `before`.
    fn add_crate(repo: &Repo, before: &[&str], name: &str) {
        let mut members = before.to_vec();
        members.push(name);
        repo.write("Cargo.toml", &workspace(&members));
        repo.write(
            &format!("{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\n"),
        );
        repo.write(&format!("{name}/src/lib.rs"), "pub fn one() {}\n");
    }

    /// A passing proof of `check` on the `on` tree naming `target`.
    fn package_proof(on: &str, check: &str, target: &str) -> String {
        let counts = if check == "eon-test" {
            "passed = 1\nfailed = 0\nskipped = 0\n"
        } else {
            ""
        };
        format!(
            "[[proof]]\non = \"{on}\"\ncheck = \"{check}\"\ntarget = \"{target}\"\n\
             platform = \"x86_64-linux\"\nexit = 0\n{counts}"
        )
    }

    fn code_input(base: &str, proof_tree: &str, extra: &str) -> String {
        format!(
            r#"route = "logic-regression"
tier = "low"
[landing]
base = "{base}"
proof_tree = "{proof_tree}"
[candidate]
base = "{base}"
tree = "{proof_tree}"
[[proof]]
on = "candidate"
check = "eon-test"
target = "workspace"
platform = "x86_64-linux"
exit = 0
passed = 3
failed = 0
skipped = 0
[[proof]]
on = "landed"
check = "nix-build"
target = ".#checks.x86_64-linux.default"
platform = "x86_64-linux"
exit = 0
{extra}
[[builder]]
harness = "codex-cli"
harness_version = "0.170.0"
model = "gpt-6.2-sol"
effort = "xhigh"
[[review]]
stage = "code"
harness = "codex-cli"
harness_version = "0.170.0"
model = "gpt-6.2-sol"
rounds = [{{ verdict = "approve", effort = "high" }}]
round1_blockers = 0
"#
        )
    }

    const DEFAULT_BUILD: &str = "[[proof]]\non = \"landed\"\ncheck = \"nix-build\"\n\
                                 target = \".#default\"\nplatform = \"x86_64-linux\"\nexit = 0\n";

    /// A code landing: base, a proof tree changing the crate, the rendered
    /// record. Returns (base, proof tree, record text).
    fn prepared(repo: &Repo, extra: &str) -> (String, String, String) {
        let base = base(repo);
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        let proof_tree = repo.stage_tree();
        let input = repo.input(&code_input(&base, &proof_tree, extra));
        let text = render(&repo.git, &input).unwrap();
        (base, proof_tree, text)
    }

    fn land(repo: &Repo, text: &str) -> String {
        repo.write(
            repo::TRACKER,
            &tracker(&[bead("t-1", "closed", "2026-11-19T00:00:00Z", &[text])]),
        );
        repo.commit("land t-1")
    }

    fn findings(repo: &Repo) -> Vec<String> {
        check(&repo.git, "t-1", "HEAD").unwrap().unwrap_err()
    }

    fn assert_finding(repo: &Repo, needle: &str) {
        let found = findings(repo);
        assert!(
            found.iter().any(|finding| finding.contains(needle)),
            "no finding containing {needle:?} in {found:#?}"
        );
    }

    #[test]
    fn a_landing_checks_in_a_clone_without_the_proof_tree() {
        let repo = Repo::new("clone");
        let (base, proof_tree, text) = prepared(&repo, DEFAULT_BUILD);
        assert!(text.starts_with(HEADER));
        let landed = land(&repo, &text);
        let summary = check(&repo.git, "t-1", "HEAD").unwrap().unwrap();
        assert!(
            summary.starts_with(&format!("t-1: landed {landed} onto {base};")),
            "{summary}"
        );

        let clone = Repo::new("clone-copy");
        fs::remove_dir_all(&clone.dir).unwrap();
        let source = repo.dir.to_str().unwrap();
        let target = clone.dir.to_str().unwrap();
        repo.git
            .run(&["clone", "-q", "--no-local", source, target])
            .unwrap();
        assert!(repo::object_type(&clone.git, &proof_tree).is_err());
        let summary = check(&clone.git, "t-1", "HEAD").unwrap().unwrap();
        assert!(summary.contains(&format!("landed {landed}")), "{summary}");
    }

    #[test]
    fn render_refuses_bad_input() {
        let repo = Repo::new("render");
        let base = base(&repo);
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        let proof_tree = repo.stage_tree();
        let render_err = |toml: String| {
            render(&repo.git, &repo.input(&toml))
                .unwrap_err()
                .to_string()
        };
        let good = code_input(&base, &proof_tree, DEFAULT_BUILD);
        let error = render_err(good.replace("tier = \"low\"", "tier = \"low\"\nschema = \"x\""));
        assert!(error.contains("render computes them"), "{error}");
        let error = render_err(good.replace(
            &format!("proof_tree = \"{proof_tree}\""),
            &format!("proof_tree = \"{base}\""),
        ));
        assert!(error.contains("is a commit, not a tree"), "{error}");
        let error = render_err(good.replace("route = ", "pane = 3\nroute = "));
        assert!(error.contains("unknown field"), "{error}");
        // The proof tree must be exactly the reviewed change on the base.
        let reviewed = proof_tree.clone();
        repo.write("crate-a/src/lib.rs", "pub fn other() {}\n");
        repo.write("notes.txt", "unreviewed\n");
        let drifted = repo.stage_tree();
        let error = render_err(good.replace(
            &format!("proof_tree = \"{reviewed}\""),
            &format!("proof_tree = \"{drifted}\""),
        ));
        assert!(
            error.contains(
                "the landed change to crate-a/src/lib.rs differs from the reviewed change"
            ),
            "{error}"
        );
        assert!(
            error.contains("the landing changes notes.txt, which the reviewed change does not"),
            "{error}"
        );
        let error = render_err(good.replace(
            "exit = 0\npassed = 3\nfailed = 0",
            "exit = 0\npassed = 3\nfailed = 1",
        ));
        assert!(
            error.contains("the counts and the exit disagree"),
            "{error}"
        );
        let later = repo.commit("later");
        let error = render_err(good.replace(
            &format!("[candidate]\nbase = \"{base}\""),
            &format!("[candidate]\nbase = \"{later}\""),
        ));
        assert!(error.contains("is not an ancestor"), "{error}");
    }

    #[test]
    fn a_landing_may_name_the_crate_it_adds() {
        let repo = Repo::new("adds-crate");
        let base = base(&repo);
        add_crate(&repo, &["crate-a"], "crate-b");
        let proof_tree = repo.stage_tree();
        let named = |target: &str| {
            let extra = package_proof("candidate", "eon-test", target)
                + &package_proof("landed", "clippy", target);
            render(
                &repo.git,
                &repo.input(&code_input(&base, &proof_tree, &extra)),
            )
        };
        assert!(named("crate-b").unwrap().starts_with(HEADER));
        let error = named("crate-c").unwrap_err().to_string();
        for on in ["candidate", "landed"] {
            assert!(
                error.contains(&format!(
                    "\"crate-c\" is not a workspace package in the {on} tree"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn each_proof_names_packages_of_its_own_tree() {
        // The candidate adds crate-b; edge adds crate-e meanwhile, so only
        // the proof tree has crate-e.
        let reviewed = |repo: &Repo| add_crate(repo, &["crate-a"], "crate-b");
        let edge = |repo: &Repo| add_crate(repo, &["crate-a"], "crate-e");
        let landed = |repo: &Repo| add_crate(repo, &["crate-a", "crate-e"], "crate-b");
        let named = |name: &str, extra: &str| {
            let repo = Repo::new(name);
            render_steps_with(&repo, &reviewed, &edge, &landed, extra)
        };
        let both = package_proof("candidate", "eon-test", "crate-b")
            + &package_proof("landed", "clippy", "crate-b");
        assert!(named("own-tree-b", &both).unwrap().starts_with(HEADER));
        let landed_e = package_proof("landed", "clippy", "crate-e");
        assert!(named("own-tree-e", &landed_e).unwrap().starts_with(HEADER));
        refused(
            named(
                "own-tree-ce",
                &package_proof("candidate", "eon-test", "crate-e"),
            ),
            "\"crate-e\" is not a workspace package in the candidate tree",
        );
    }

    const CHANGELOG: &str = "# Changelog\n\n## Unreleased\n\n- old entry\n";
    const TEXT: &str = "a\nb\nc\nd\ne\nf\n";

    /// A landing onto a moved base: the base commit with CHANGELOG.md and
    /// crate-a/src/text.rs, the candidate tree after `reviewed` on it, edge's
    /// commit after `edge` on it as the landing base, and the proof tree after
    /// `landed` on that.
    fn render_steps(
        repo: &Repo,
        reviewed: &dyn Fn(&Repo),
        edge: &dyn Fn(&Repo),
        landed: &dyn Fn(&Repo),
    ) -> Result<String> {
        render_steps_with(repo, reviewed, edge, landed, DEFAULT_BUILD)
    }

    /// [`render_steps`] with `extra` input after the fixed proofs.
    fn render_steps_with(
        repo: &Repo,
        reviewed: &dyn Fn(&Repo),
        edge: &dyn Fn(&Repo),
        landed: &dyn Fn(&Repo),
        extra: &str,
    ) -> Result<String> {
        repo.write("CHANGELOG.md", CHANGELOG);
        repo.write("crate-a/src/text.rs", TEXT);
        let candidate_base = base(repo);
        reviewed(repo);
        let candidate_tree = repo.stage_tree();
        repo.git
            .run(&["reset", "-q", "--hard", &candidate_base])
            .unwrap();
        edge(repo);
        let landing_base = repo.commit("edge");
        landed(repo);
        let proof_tree = repo.stage_tree();
        let input = code_input(&landing_base, &proof_tree, extra).replace(
            &format!("[candidate]\nbase = \"{landing_base}\"\ntree = \"{proof_tree}\""),
            &format!("[candidate]\nbase = \"{candidate_base}\"\ntree = \"{candidate_tree}\""),
        );
        render(&repo.git, &repo.input(&input))
    }

    /// [`render_steps`] where each step writes files' whole new text.
    fn render_moved(
        repo: &Repo,
        reviewed: &[(&str, &str)],
        edge: &[(&str, &str)],
        landed: &[(&str, &str)],
    ) -> Result<String> {
        render_steps(repo, &writes(reviewed), &writes(edge), &writes(landed))
    }

    /// A step writing each file's whole new text.
    fn writes<'a>(files: &'a [(&'a str, &'a str)]) -> impl Fn(&Repo) + 'a {
        move |repo| {
            for (path, text) in files {
                repo.write(path, text);
            }
        }
    }

    fn changelog(entries: &str) -> String {
        CHANGELOG.replace("- old entry\n", &format!("{entries}- old entry\n"))
    }

    fn refused(result: Result<String>, needle: &str) {
        let error = result.unwrap_err().to_string();
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn edge_edits_beside_the_candidates_lines_pass() {
        // Today's case: edge adds its CHANGELOG entry above the candidate's
        // and edits another line of a file the candidate also changed.
        let repo = Repo::new("moved");
        let text = render_moved(
            &repo,
            &[
                ("CHANGELOG.md", &changelog("- mine\n")),
                ("crate-a/src/text.rs", &TEXT.replace('b', "B")),
            ],
            &[
                ("CHANGELOG.md", &changelog("- edge\n")),
                ("crate-a/src/text.rs", &TEXT.replace('e', "E")),
            ],
            &[
                ("CHANGELOG.md", &changelog("- edge\n- mine\n")),
                (
                    "crate-a/src/text.rs",
                    &TEXT.replace('b', "B").replace('e', "E"),
                ),
            ],
        )
        .unwrap();
        assert!(text.starts_with(HEADER));
    }

    #[test]
    fn a_change_to_the_candidates_own_lines_is_refused() {
        let edge = changelog("- edge\n");
        for (name, reviewed, landed) in [
            ("edited", "- mine\n", "- edge\n- mine, edited\n"),
            ("indented", "- mine\n", "- edge\n  - mine\n"),
            // One reviewed hunk landed as two around edge's line.
            ("split", "- one\n- two\n", "- one\n- edge\n- two\n"),
        ] {
            let repo = Repo::new(name);
            let result = render_moved(
                &repo,
                &[("CHANGELOG.md", &changelog(reviewed))],
                &[("CHANGELOG.md", &edge)],
                &[("CHANGELOG.md", &changelog(landed))],
            );
            refused(
                result,
                "the landed change to CHANGELOG.md differs from the reviewed change",
            );
        }
    }

    #[test]
    fn reordered_hunks_are_refused() {
        // The candidate adds X after a and Y after f; the resolution puts Y
        // after a and X after f, beside edge's edit to d.
        let repo = Repo::new("reordered");
        let result = render_moved(
            &repo,
            &[("crate-a/src/text.rs", "a\nX\nb\nc\nd\ne\nf\nY\n")],
            &[("crate-a/src/text.rs", "a\nb\nc\nD\ne\nf\n")],
            &[("crate-a/src/text.rs", "a\nY\nb\nc\nD\ne\nf\nX\n")],
        );
        refused(
            result,
            "the landed change to crate-a/src/text.rs differs from the reviewed change",
        );
    }

    #[test]
    fn a_landing_that_drops_edges_edit_or_the_candidates_change_is_refused() {
        let mine = TEXT.replace('b', "B");
        // The proof tree's file is the reviewed tree's, so it reverts edge.
        let repo = Repo::new("revert");
        let result = render_moved(
            &repo,
            &[("crate-a/src/text.rs", &mine)],
            &[("crate-a/src/text.rs", &TEXT.replace('e', "E"))],
            &[("crate-a/src/text.rs", &mine)],
        );
        refused(
            result,
            "the landed change to crate-a/src/text.rs differs from the reviewed change",
        );
        // Edge already carries the change, so the landing does not.
        let repo = Repo::new("carried");
        let result = render_moved(
            &repo,
            &[("crate-a/src/text.rs", &mine)],
            &[("crate-a/src/text.rs", &mine)],
            &[],
        );
        refused(
            result,
            "the landing does not change crate-a/src/text.rs, which the reviewed change does",
        );
    }

    #[test]
    fn a_landing_keeps_the_mode_changes_of_both_sides() {
        use std::os::unix::fs::PermissionsExt;
        let edit = |text: &'static str, executable: bool| {
            move |repo: &Repo| {
                repo.write("crate-a/src/text.rs", text);
                let mode = if executable { 0o755 } else { 0o644 };
                let path = repo.dir.join("crate-a/src/text.rs");
                fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            }
        };
        let (mine, edge, both) = (
            "a\nB\nc\nd\ne\nf\n",
            "a\nb\nc\nd\nE\nf\n",
            "a\nB\nc\nd\nE\nf\n",
        );
        // Edge makes the file executable; the landing keeps that.
        let repo = Repo::new("mode-kept");
        render_steps(
            &repo,
            &edit(mine, false),
            &edit(edge, true),
            &edit(both, true),
        )
        .unwrap();
        let differs = "the landed change to crate-a/src/text.rs differs from the reviewed change";
        // Edge only makes the file executable; the landing keeps that.
        let repo = Repo::new("mode-only");
        render_steps(
            &repo,
            &edit(mine, false),
            &edit(TEXT, true),
            &edit(mine, true),
        )
        .unwrap();
        // ... or drops it.
        let repo = Repo::new("mode-only-dropped");
        let result = render_steps(
            &repo,
            &edit(mine, false),
            &edit(TEXT, true),
            &edit(mine, false),
        );
        refused(result, differs);
        // The landing drops edge's executable bit.
        let repo = Repo::new("mode-dropped");
        let result = render_steps(
            &repo,
            &edit(mine, false),
            &edit(edge, true),
            &edit(both, false),
        );
        refused(result, differs);
        // The landing drops the candidate's executable bit.
        let repo = Repo::new("mode-lost");
        let result = render_steps(
            &repo,
            &edit(mine, true),
            &edit(edge, false),
            &edit(both, false),
        );
        refused(result, differs);
    }

    #[test]
    fn configured_inter_hunk_context_cannot_merge_hunks() {
        // With diff.interHunkContext = 1, edge's line between the candidate's
        // two edits would join both hunks as context and differ.
        let repo = Repo::new("inter-hunk");
        repo.git
            .run(&["config", "diff.interHunkContext", "1"])
            .unwrap();
        render_moved(
            &repo,
            &[("crate-a/src/text.rs", "a\nB\nc\nD\ne\nf\n")],
            &[("crate-a/src/text.rs", "a\nb\nC\nd\ne\nf\n")],
            &[("crate-a/src/text.rs", "a\nB\nC\nD\ne\nf\n")],
        )
        .unwrap();
    }

    #[test]
    fn positions_count_in_a_file_edge_did_not_touch() {
        // Edge changes only CHANGELOG.md; the landing moves the candidate's
        // line elsewhere in text.rs.
        let repo = Repo::new("positions");
        let result = render_moved(
            &repo,
            &[("crate-a/src/text.rs", "a\nX\nb\nc\nd\ne\nf\n")],
            &[("CHANGELOG.md", &changelog("- edge\n"))],
            &[("crate-a/src/text.rs", "a\nb\nc\nd\ne\nX\nf\n")],
        );
        refused(
            result,
            "the landed change to crate-a/src/text.rs differs from the reviewed change",
        );
    }

    #[test]
    fn a_type_change_onto_a_file_edge_edited_is_refused_by_name() {
        let link = |repo: &Repo| {
            let path = repo.dir.join("crate-a/src/text.rs");
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink("elsewhere", path).unwrap();
        };
        let repo = Repo::new("typechange");
        let result = render_steps(
            &repo,
            &link,
            &writes(&[("crate-a/src/text.rs", "a\nb\nc\nd\nE\nf\n")]),
            &link,
        );
        refused(
            result,
            "the landed change to crate-a/src/text.rs differs from the reviewed change",
        );
    }

    #[test]
    fn a_changed_landing_tree_or_edited_text_fails() {
        let repo = Repo::new("edits");
        let (_, _, text) = prepared(&repo, DEFAULT_BUILD);
        repo.write("crate-a/src/lib.rs", "pub fn three() {}\n");
        land(&repo, &text);
        assert_finding(&repo, "differs from the proof tree's");

        let repo = Repo::new("text");
        let (_, _, text) = prepared(&repo, DEFAULT_BUILD);
        land(&repo, &text.replace("tier low.", "tier high."));
        assert_finding(&repo, "not the rendering of its JSON line");
    }

    #[test]
    fn the_bead_must_be_closed_with_the_fixed_reason_and_a_record() {
        let repo = Repo::new("closed");
        let (_, _, text) = prepared(&repo, DEFAULT_BUILD);
        repo.write(repo::TRACKER, &tracker(&[bead("t-1", "closed", "x", &[])]));
        repo.commit("closed without a record");
        assert_finding(&repo, "has no close record");
        let mut open = bead("t-1", "open", "", &[&text]);
        repo.write(repo::TRACKER, &tracker(&[open.clone()]));
        repo.commit("open");
        assert_finding(&repo, "is not closed");
        open["status"] = "closed".into();
        open["close_reason"] = "Landed: anything".into();
        repo.write(repo::TRACKER, &tracker(&[open]));
        repo.commit("other reason");
        assert_finding(&repo, "close reason is not");
    }

    #[test]
    fn the_landing_commit_must_introduce_the_record() {
        // A metadata-only child of the base with the same source, and the
        // record in the commit after it.
        let repo = Repo::new("late");
        let (_, _, text) = prepared(&repo, DEFAULT_BUILD);
        repo.commit("the proof tree's source, without the record");
        land(&repo, &text);
        assert_finding(&repo, "does not close t-1 with this record");
    }

    #[test]
    fn a_base_off_the_first_parent_line_fails() {
        let repo = Repo::new("side");
        let base = base(&repo);
        repo.write("side.md", "side\n");
        let side = repo.commit("side");
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        let proof_tree = repo.stage_tree();
        let input = code_input(&side, &proof_tree, DEFAULT_BUILD);
        let text = render(&repo.git, &repo.input(&input)).unwrap();
        repo.git.run(&["reset", "-q", "--hard", &base]).unwrap();
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        repo.write("side.md", "side\n");
        land(&repo, &text);
        assert_finding(&repo, "first-parent line");
    }

    #[test]
    fn a_reclose_without_a_new_record_fails() {
        let repo = Repo::new("reclose");
        let (_, _, text) = prepared(&repo, DEFAULT_BUILD);
        land(&repo, &text);
        repo.write(
            repo::TRACKER,
            &tracker(&[bead("t-1", "closed", "2026-11-20T00:00:00Z", &[&text])]),
        );
        repo.commit("reopened and closed again");
        assert_finding(&repo, "closed again after its landing commit");
    }

    #[test]
    fn a_second_record_without_a_reopen_fails() {
        let repo = Repo::new("second");
        let (_, _, first) = prepared(&repo, DEFAULT_BUILD);
        land(&repo, &first);
        repo.write("crate-a/src/lib.rs", "pub fn three() {}\n");
        let proof_tree = repo.stage_tree();
        let base = repo.git.rev_parse("HEAD").unwrap();
        let second = render(
            &repo.git,
            &repo.input(&code_input(&base, &proof_tree, DEFAULT_BUILD)),
        )
        .unwrap();
        repo.write(
            repo::TRACKER,
            &tracker(&[bead(
                "t-1",
                "closed",
                "2026-11-19T00:00:00Z",
                &[&first, &second],
            )]),
        );
        repo.commit("a second record as a plain comment");
        assert_finding(&repo, "was already closed at");
    }

    #[test]
    fn a_landed_record_checks_whatever_the_policy_and_beads_say_later() {
        // Rendered with an open known-failure bead, landed with it closed.
        let repo = Repo::new("history");
        let base = base(&repo);
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        let proof_tree = repo.stage_tree();
        let failing = DEFAULT_BUILD.replace("exit = 0", "exit = 1\nknown_failure = \"t-kf\"");
        let tracking = bead("t-kf", "open", "", &[]);
        repo.write(
            repo::TRACKER,
            &tracker(&[bead("t-1", "open", "", &[]), tracking.clone()]),
        );
        let input = code_input(&base, &proof_tree, &failing);
        let text = render(&repo.git, &repo.input(&input)).unwrap();
        assert!(text.contains("exit 1; known failure t-kf"));
        let mut closed = tracking;
        closed["status"] = "closed".into();
        let land_with = |text: &str| {
            repo.write(
                repo::TRACKER,
                &tracker(&[
                    bead("t-1", "closed", "2026-11-19T00:00:00Z", &[text]),
                    closed.clone(),
                ]),
            );
            repo.commit("land")
        };
        land_with(&text);
        assert!(check(&repo.git, "t-1", "HEAD").unwrap().is_ok());
        // A model the policy no longer lists, as after a policy edit.
        repo.git.run(&["reset", "-q", "--hard", &base]).unwrap();
        repo.write("crate-a/src/lib.rs", "pub fn two() {}\n");
        let mut record: Record = toml::from_str(&input).unwrap();
        record.review[0].model = "retired-model-x".into();
        record.complete(
            repo::source_digest(&repo.git, &proof_tree).unwrap(),
            &repo::rules_blobs(&repo.git, &base).unwrap(),
        );
        land_with(&record.render());
        assert!(check(&repo.git, "t-1", "HEAD").unwrap().is_ok());
    }

    #[test]
    fn the_source_digest_of_a_known_tree_is_fixed() {
        let repo = Repo::new("digest");
        repo.write("a.txt", "a\n");
        repo.write("dir/b.rs", "b\n");
        repo.write(repo::TRACKER, "x\n");
        let tree = repo.stage_tree();
        assert_eq!(tree, "36d83f8f8f2fa333167c609748aef65e3fe9b38e");
        assert_eq!(
            repo::source_digest(&repo.git, &tree).unwrap(),
            "src-1:c6e5f76cedfe7d48ef0df963f819fe8b11d40c0ed2bb5495c2152cc5b02c072a"
        );
    }
}
