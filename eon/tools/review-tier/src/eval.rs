//! The review eval: past changes that should have been stopped, a blind
//! review setup for each, and scoring of a reviewer's report.
//!
//! A blocker or should-fix citing within five lines of an expected range
//! locates; matching every mechanism group catches, some groups disputes. A
//! ruling in `RULINGS` for the case and report SHA-256 replaces the outcome.
use crate::Result;
use crate::classify::{self, DarwinCheck, Verdict};
use crate::git::Git;
use crate::record::sha256_hex;
use crate::rules::{DarwinLevel, Tier};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const CASES: &str = "eon/tools/review-tier/evals/cases.toml";
const RULINGS: &str = "eon/tools/review-tier/evals/rulings.toml";

const CLASSES: &[&str] = &[
    "live-terminals",
    "contract-drift",
    "wire-version",
    "cli-parity",
    "configurability",
    "small-screens",
    "desktop-focus",
    "tier-rules",
    "companion-code",
    "false-success",
    "macos-portability",
    "nixos-portability",
    "racy-test",
    "gate-cannot-fail",
    "missing-gate-evidence",
    "release-flip",
    "config-compat",
    "trust-boundary",
    "legacy-shim",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cases {
    case: Vec<Case>,
    #[serde(default)]
    darwin: Vec<DarwinCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DarwinCase {
    commit: String,
    darwin: DarwinLevel,
    #[serde(default)]
    build: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    id: String,
    #[allow(dead_code, reason = "validated by serde; never shown to a reviewer")]
    kind: Kind,
    commit: String,
    fixed_by: String,
    class: String,
    tier: Tier,
    #[allow(
        dead_code,
        reason = "for people reading the cases; never shown to a reviewer"
    )]
    summary: String,
    #[allow(
        dead_code,
        reason = "for people reading the cases; never shown to a reviewer"
    )]
    evidence: String,
    mechanism: Vec<Vec<String>>,
    expect: Vec<Expect>,
    /// Kept so cited IDs stay valid and still checked, but never scored.
    #[serde(default)]
    retired: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rulings {
    #[serde(default)]
    ruling: Vec<Ruling>,
}

/// Judges' outcome for one case and one report, by its SHA-256.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ruling {
    case: String,
    report: String,
    outcome: Ruled,
    judges: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Ruled {
    Caught,
    Missed,
    Split,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Kind {
    Revert,
    FixForward,
    FailedLanding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    file: String,
    lines: Option<[u32; 2]>,
}

/// Parses and validates the cases file text.
fn parse(text: &str) -> Result<Cases> {
    let cases: Cases = toml::from_str(text).map_err(|error| format!("{CASES}: {error}"))?;
    let mut ids = BTreeSet::new();
    for case in &cases.case {
        validate(case).map_err(|error| format!("{CASES}: case {}: {error}", case.id))?;
        if !ids.insert(case.id.as_str()) {
            return Err(format!("{CASES}: duplicate case id {}", case.id).into());
        }
    }
    let mut commits = BTreeSet::new();
    for case in &cases.darwin {
        validate_hex(&case.commit, 40).map_err(|error| format!("{CASES}: Darwin case: {error}"))?;
        if !commits.insert(case.commit.as_str()) {
            return Err(format!("{CASES}: duplicate Darwin case {}", case.commit).into());
        }
        if case.darwin == DarwinLevel::None && case.build {
            return Err(format!("{CASES}: Darwin case {}: none has no legs", case.commit).into());
        }
    }
    Ok(cases)
}

/// Parses and validates the rulings file text against the cases.
fn parse_rulings(text: &str, cases: &[Case]) -> Result<Vec<Ruling>> {
    let rulings: Rulings = toml::from_str(text).map_err(|error| format!("{RULINGS}: {error}"))?;
    let mut keys = BTreeSet::new();
    for ruling in &rulings.ruling {
        let judges: BTreeSet<&str> = ruling.judges.iter().map(String::as_str).collect();
        let error = if find(cases, &ruling.case).is_err() {
            format!("no case {}", ruling.case)
        } else if let Err(error) = validate_hex(&ruling.report, 64) {
            error
        } else if judges.len() < 2 || judges.len() != ruling.judges.len() {
            "needs two or more distinct judges".into()
        } else if judges.iter().any(|judge| {
            judge.is_empty() || judge.contains(|c: char| c.is_whitespace() || c == ',' || c == '=')
        }) {
            "a judge is empty or holds whitespace, `,` or `=`".into()
        } else if !keys.insert((ruling.case.as_str(), ruling.report.as_str())) {
            "duplicate ruling".into()
        } else {
            continue;
        };
        return Err(format!("{RULINGS}: case {}: {error}", ruling.case).into());
    }
    Ok(rulings.ruling)
}

fn validate_hex(sha: &str, len: usize) -> std::result::Result<(), String> {
    if !(sha.len() == len
        && sha
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    {
        return Err(format!("{sha:?} is not {len} lowercase hex digits"));
    }
    Ok(())
}

fn validate(case: &Case) -> std::result::Result<(), String> {
    let id = case.id.as_bytes();
    let digits = id.get(1..).unwrap_or_default();
    if !(matches!(id.first(), Some(b'R' | b'F'))
        && !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit))
    {
        return Err("id must be R or F followed by digits".into());
    }
    for sha in [&case.commit, &case.fixed_by] {
        validate_hex(sha, 40)?;
    }
    if !CLASSES.contains(&case.class.as_str()) {
        return Err(format!("unknown class {:?}", case.class));
    }
    if case.tier == Tier::Low {
        return Err("tier must be medium or high".into());
    }
    if case.mechanism.is_empty() || case.mechanism.iter().any(Vec::is_empty) {
        return Err("mechanism needs one or more groups of one or more words".into());
    }
    for word in case.mechanism.iter().flatten() {
        if normalize(word).is_empty() || *word != word.to_lowercase() {
            return Err(format!("mechanism word {word:?} is empty or not lowercase"));
        }
    }
    if case.expect.is_empty() {
        return Err("needs at least one expect".into());
    }
    for expect in &case.expect {
        if expect.file.is_empty() {
            return Err("expect file is empty".into());
        }
        if let Some([start, end]) = expect.lines
            && (start == 0 || start > end)
        {
            return Err(format!(
                "lines [{start}, {end}] for {} are not ordered",
                expect.file
            ));
        }
    }
    Ok(())
}

fn read(git: &Git, file: &str) -> Result<String> {
    let path = git.root().join(file);
    Ok(fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?)
}

/// The cases, the rulings and the cases file's text.
fn load(git: &Git) -> Result<(Cases, Vec<Ruling>, String)> {
    let text = read(git, CASES)?;
    let cases = parse(&text)?;
    let rulings = parse_rulings(&read(git, RULINGS)?, &cases.case)?;
    Ok((cases, rulings, text))
}

fn find<'a>(cases: &'a [Case], id: &str) -> Result<&'a Case> {
    cases
        .iter()
        .find(|case| case.id == id)
        .ok_or_else(|| format!("no case {id:?} in {CASES}").into())
}

/// A case commit, its first parent, and the classification between them.
struct Classified {
    commit: String,
    base: String,
    verdicts: Vec<Verdict>,
}

fn classify_commit(git: &Git, revision: &str) -> Result<Classified> {
    let commit = git.commit(revision)?;
    let base = git.first_parent(&commit)?;
    let verdicts = crate::classify_trees(git, &git.tree(&base)?, &git.tree(&commit)?)?;
    Ok(Classified {
        commit,
        base,
        verdicts,
    })
}

/// `eval check`: one line per case; returns whether every case passed.
pub fn check(git: &Git) -> Result<bool> {
    let (cases, ..) = load(git)?;
    let mut passed = true;
    for case in &cases.case {
        match check_case(git, case) {
            Ok((tier, None)) => println!("{} {tier} ok", case.id),
            Ok((tier, Some(reason))) => {
                passed = false;
                println!("{} {tier} FAIL: {reason}", case.id);
            }
            Err(error) => {
                passed = false;
                println!("{} - FAIL: {error}", case.id);
            }
        }
    }
    for case in cases.darwin {
        let commit = &case.commit;
        let expected = DarwinCheck {
            level: case.darwin,
            build: case.build,
        };
        match classify_commit(git, commit).map(|case| classify::slice_darwin(&case.verdicts)) {
            Ok(actual) if actual == expected => println!("darwin {commit} {actual} ok"),
            Ok(actual) => {
                passed = false;
                println!("darwin {commit} {actual} FAIL: expected {expected}");
            }
            Err(error) => {
                passed = false;
                println!("darwin {commit} - FAIL: {error}");
            }
        }
    }
    Ok(passed)
}

fn check_case(git: &Git, case: &Case) -> Result<(Tier, Option<String>)> {
    let classified = classify_commit(git, &case.commit)?;
    let tier = classify::slice_tier(&classified.verdicts);
    // An omission case may cite a companion file the commit left stale, so an
    // expected file need only exist at the commit; at least one must be in
    // the diff. Its lines must exist there too.
    let mut absent = Vec::new();
    let mut short = Vec::new();
    for expect in &case.expect {
        let object = format!("{}:{}", classified.commit, expect.file);
        if git.rev_parse(&object).is_err() {
            absent.push(expect.file.as_str());
        } else if let Some([start, end]) = expect.lines
            && let Some(count) = past_end(&git.run(&["cat-file", "blob", &object])?, end)
        {
            short.push(format!(
                "{} has {count} lines, not {start}-{end}",
                expect.file
            ));
        }
    }
    let touched = case.expect.iter().any(|expect| {
        classified
            .verdicts
            .iter()
            .any(|verdict| verdict.path == expect.file)
    });
    let failure = if !absent.is_empty() {
        Some(format!(
            "expected files absent at the commit: {}",
            absent.join(", ")
        ))
    } else if !short.is_empty() {
        Some(format!("expected lines past the end: {}", short.join(", ")))
    } else if !touched {
        Some("no expected file is in the commit's diff".to_owned())
    } else if tier == Tier::Low || tier < case.tier {
        Some(format!("classified {tier}, below {}", case.tier))
    } else {
        None
    };
    Ok((tier, failure))
}

/// The line count of `content` when line `end` is past its end.
fn past_end(content: &[u8], end: u32) -> Option<usize> {
    let count = content.iter().filter(|&&byte| byte == b'\n').count()
        + usize::from(!content.is_empty() && !content.ends_with(b"\n"));
    (usize::try_from(end).is_ok_and(|end| end > count)).then_some(count)
}

/// `eval prepare`: makes `dir` a repository holding only the case commit and
/// its ancestors, with the current REVIEW.md, and returns the case brief.
/// With `export`, it also writes the case diff to `change.diff` and the
/// brief, which then needs no git, to `brief.md`.
pub fn prepare(git: &Git, id: &str, dir: &Path, export: bool) -> Result<String> {
    let (cases, ..) = load(git)?;
    let case = find(&cases.case, id)?;
    let review = fs::read(git.root().join("REVIEW.md"))
        .map_err(|error| format!("cannot read REVIEW.md at the repository root: {error}"))?;
    let dir = std::path::absolute(dir)?;
    match fs::read_dir(&dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(format!("{} is not empty", dir.display()).into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot use {}: {error}", dir.display()).into()),
    }
    let classified = classify_commit(git, &case.commit)?;
    Git::init(&dir)?;
    let scratch = Git::at(&dir);
    let root = git.root().to_str().ok_or("repository path is not UTF-8")?;
    // Protocol v2 serves any reachable commit by id, not only ref tips.
    scratch.run(&[
        "-c",
        "protocol.version=2",
        "fetch",
        "-q",
        "--no-tags",
        "--no-write-fetch-head",
        root,
        &classified.commit,
    ])?;
    scratch.run(&["checkout", "-q", "--detach", &classified.commit])?;
    fs::write(dir.join("REVIEW.md"), review)?;
    let brief = brief(&classified, export);
    if export {
        // Pinned so that no git configuration changes the exported bytes.
        let mut args: Vec<&str> = "diff --no-ext-diff --no-textconv --no-color -U3 \
            --inter-hunk-context=0 --diff-algorithm=myers --indent-heuristic -O/dev/null \
            --src-prefix=a/ --dst-prefix=b/"
            .split_whitespace()
            .collect();
        args.extend([classified.base.as_str(), &classified.commit]);
        fs::write(dir.join("change.diff"), git.run(&args)?)?;
        fs::write(dir.join("brief.md"), &brief)?;
    }
    Ok(brief)
}

fn brief(classified: &Classified, export: bool) -> String {
    let diff = if export {
        "The diff is change.diff in this directory. Read files in this directory.".to_owned()
    } else {
        format!(
            "The diff is `git diff {} HEAD`. Read files at HEAD.",
            classified.base
        )
    };
    format!(
        "Review this change. Follow REVIEW.md at the repository root.\n\
         candidate: {}\n\
         base: {}\n\
         tier: {}\n\
         {}\
         {diff} Do not build or run anything.\n",
        classified.commit,
        classified.base,
        classify::slice_tier(&classified.verdicts),
        classify::render(&classified.verdicts),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Blocker,
    ShouldFix,
    Nit,
}

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    severity: Severity,
    citations: Vec<(String, u32, u32)>,
    text: String,
}

/// Findings in REVIEW.md's format: lines starting (after trimming and an
/// optional `- ` or `* `) with `[blocker]`, `[should-fix]` or `[nit]`. The
/// text after the tag, without its citation tokens and in `normalize`'s
/// form, is what mechanism words must match, so a cited path such as
/// `Cargo.lock` never supplies one.
fn findings(report: &str) -> Vec<Finding> {
    let tags = [
        ("[blocker]", Severity::Blocker),
        ("[should-fix]", Severity::ShouldFix),
        ("[nit]", Severity::Nit),
    ];
    report
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let body = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .unwrap_or(trimmed);
            tags.iter().find_map(|(tag, severity)| {
                let (cited, words): (Vec<&str>, Vec<&str>) = body
                    .strip_prefix(tag)?
                    .split_whitespace()
                    .partition(|word| citation(word).is_some());
                Some(Finding {
                    severity: *severity,
                    citations: cited.into_iter().filter_map(citation).collect(),
                    text: normalize(&words.join(" ")),
                })
            })
        })
        .collect()
}

/// Lowercase, with hyphens, underscores and whitespace runs as one space.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `path:line` or `path:line-line`, with surrounding backticks, a leading
/// `(` or `[`, trailing `,;:.)]`, leading `./` and one git `a/` or `b/`
/// prefix removed.
fn citation(token: &str) -> Option<(String, u32, u32)> {
    let punctuation: &[char] = &[',', ';', ':', '.', ')', ']'];
    let token = token
        .trim_start_matches(['(', '['])
        .trim_end_matches(punctuation)
        .trim_matches('`')
        .trim_end_matches(punctuation);
    let (path, range) = token.rsplit_once(':')?;
    let path = path.trim_start_matches("./");
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    let number = |text: &str| {
        (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse::<u32>().ok())
            .flatten()
    };
    let (start, end) = match range.split_once('-') {
        Some((start, end)) => (number(start)?, number(end)?),
        None => (number(range)?, number(range)?),
    };
    (!path.is_empty() && start <= end).then(|| (path.into(), start, end))
}

/// `eval score`: `<id> caught|disputed|located|missed|split blocker=<n>
/// should-fix=<n> nit=<n> report=<sha256> [ruled=<judges>] cases=<12 hex>
/// scorer=<12 hex>`, or `<id> retired cases=… scorer=…`.
pub fn score(git: &Git, id: &str, report: &Path) -> Result<String> {
    let (cases, rulings, text) = load(git)?;
    let case = find(&cases.case, id)?;
    let line = if case.retired {
        format!("{} retired", case.id)
    } else {
        let report = fs::read_to_string(report)
            .map_err(|error| format!("cannot read {}: {error}", report.display()))?;
        score_report(case, &report, &rulings)
    };
    Ok(format!("{line} {}", provenance(&text)))
}

/// The SHA-256, 12 hex digits each, of the cases file and of this scorer's
/// source as compiled.
fn provenance(cases: &str) -> String {
    let short = |text: &str| sha256_hex(text.as_bytes())[..12].to_owned();
    format!(
        "cases={} scorer={}",
        short(cases),
        short(include_str!("eval.rs"))
    )
}

fn score_report(case: &Case, report: &str, rulings: &[Ruling]) -> String {
    let findings = findings(report);
    let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
    let groups = |finding: &Finding| {
        case.mechanism
            .iter()
            .filter(|group| {
                group
                    .iter()
                    .any(|word| finding.text.contains(&normalize(word)))
            })
            .count()
    };
    let best = findings
        .iter()
        .filter(|finding| finding.severity != Severity::Nit && locates(case, finding))
        .map(groups)
        .max();
    let digest = sha256_hex(report.as_bytes());
    let ruling = rulings
        .iter()
        .find(|ruling| ruling.case == case.id && ruling.report == digest);
    let outcome = match (ruling.map(|ruling| ruling.outcome), best) {
        (Some(Ruled::Caught), _) => "caught",
        (Some(Ruled::Missed), _) | (None, None) => "missed",
        (Some(Ruled::Split), _) => "split",
        (None, Some(0)) => "located",
        (None, Some(groups)) if groups == case.mechanism.len() => "caught",
        (None, Some(_)) => "disputed",
    };
    let ruled = ruling.map_or(String::new(), |ruling| {
        format!(" ruled={}", ruling.judges.join(","))
    });
    format!(
        "{} {outcome} blocker={} should-fix={} nit={} report={digest}{ruled}",
        case.id,
        count(Severity::Blocker),
        count(Severity::ShouldFix),
        count(Severity::Nit)
    )
}

fn locates(case: &Case, finding: &Finding) -> bool {
    finding.citations.iter().any(|(path, start, end)| {
        case.expect.iter().any(|expect| {
            expect.file == *path
                && expect.lines.is_none_or(|[low, high]| {
                    *start <= high.saturating_add(5) && *end >= low.saturating_sub(5)
                })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::Repo;

    const SHA: &str = "ff0b5fe4bc8819b4834f609e57ad5a201b3ea269";

    fn case(lines: Option<[u32; 2]>) -> Case {
        Case {
            id: "R01".into(),
            kind: Kind::Revert,
            commit: SHA.into(),
            fixed_by: SHA.into(),
            class: "release-flip".into(),
            tier: Tier::High,
            summary: "s".into(),
            evidence: "e".into(),
            mechanism: vec![
                vec!["beta".into(), "0.2.0".into()],
                vec!["cut".into(), "release".into()],
            ],
            expect: vec![Expect {
                file: "eon/crates/eon/src/product.rs".into(),
                lines,
            }],
            retired: false,
        }
    }

    fn case_text(sha: &str) -> String {
        format!(
            "[[case]]\nid = \"R01\"\nkind = \"revert\"\ncommit = \"{sha}\"\nfixed_by = \"{sha}\"\n\
             class = \"release-flip\"\ntier = \"high\"\nsummary = \"s\"\nevidence = \"e\"\n\
             mechanism = [[\"beta\"]]\n[[case.expect]]\nfile = \"a.rs\"\nlines = [3, 4]\n"
        )
    }

    #[test]
    fn the_real_cases_file_validates() {
        let cases = parse(include_str!("../evals/cases.toml")).unwrap();
        assert!(!cases.case.is_empty());
        assert!(!cases.darwin.is_empty());
    }

    #[test]
    fn invalid_cases_are_rejected() {
        let base = case_text(SHA);
        assert_eq!(parse(&base).unwrap().case.len(), 1);
        let bad = [
            base.replace("id = \"R01\"", "id = \"X01\""),
            base.replace("id = \"R01\"", "id = \"R\""),
            base.replace("\"revert\"", "\"reverted\""),
            base.replace("tier = \"high\"", "tier = \"low\""),
            base.replace("tier = \"high\"", "tier = \"urgent\""),
            base.replace(&format!("commit = \"{SHA}\""), "commit = \"ff0b5fe4\""),
            base.replace("release-flip", "release"),
            base.replace("[[\"beta\"]]", "[]"),
            base.replace("[[\"beta\"]]", "[[]]"),
            base.replace("[[\"beta\"]]", "[[\"\"]]"),
            base.replace("[[\"beta\"]]", "[[\"Beta\"]]"),
            base.replace("[[\"beta\"]]", "[[\"-\"]]"),
            base.replace("[3, 4]", "[4, 3]"),
            base.replace("[3, 4]", "[3]"),
            base.replace("summary", "note = \"x\"\nsummary"),
            base.split("[[case.expect]]")
                .next()
                .unwrap_or_default()
                .into(),
            format!("{base}{base}"),
            format!("{base}\n[[darwin]]\ncommit = \"{SHA}\"\ndarwin = \"none\"\nbuild = true\n"),
        ];
        for text in bad {
            assert!(parse(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn report_findings_parse() {
        let report = "# Review\n\
            - [blocker] `eon/crates/eon/src/product.rs:12-14`, flips beta at the cut.\n\
            * [should-fix] see venus/src/a.rs:7: racy\n\
            \x20\x20- [nit] wording.\n\
            [nit]a.rs:1\n\
            - [blockers] not a finding\n\
            text [blocker] not a finding\n";
        let found = findings(report);
        assert_eq!(found.len(), 4);
        assert_eq!(found[0].severity, Severity::Blocker);
        assert_eq!(
            found[0].citations,
            at("eon/crates/eon/src/product.rs", 12, 14)
        );
        assert_eq!(found[1].severity, Severity::ShouldFix);
        assert_eq!(found[1].citations, at("venus/src/a.rs", 7, 7));
        assert_eq!(found[2].severity, Severity::Nit);
        assert!(found[2].citations.is_empty());
        assert_eq!(found[3].citations, at("a.rs", 1, 1));
        assert_eq!(citation("a.rs:9-3"), None);
        assert_eq!(citation("a.rs:x"), None);
        assert_eq!(citation(":3"), None);
        assert_eq!(citation("http://x"), None);
        assert_eq!(citation("a.rs:3."), Some(("a.rs".into(), 3, 3)));
    }

    #[test]
    fn scores_caught_located_and_missed() {
        let lined = case(Some([20, 30]));
        let caught = "- [blocker] eon/crates/eon/src/product.rs:35 flips to Beta outside the release cut\n[nit] x";
        assert_eq!(
            line(&lined, caught),
            "R01 caught blocker=1 should-fix=0 nit=1"
        );
        let located = "[should-fix] eon/crates/eon/src/product.rs:15 changes the version";
        assert_eq!(
            line(&lined, located),
            "R01 located blocker=0 should-fix=1 nit=0"
        );
        let far = "[blocker] eon/crates/eon/src/product.rs:36 beta at the cut";
        assert_eq!(line(&lined, far), "R01 missed blocker=1 should-fix=0 nit=0");
        let nit = "[nit] eon/crates/eon/src/product.rs:25 beta at the cut";
        assert_eq!(line(&lined, nit), "R01 missed blocker=0 should-fix=0 nit=1");
        let other_file = "[blocker] eon/crates/eon/src/main.rs:25 beta at the cut";
        assert_eq!(
            line(&lined, other_file),
            "R01 missed blocker=1 should-fix=0 nit=0"
        );
        let anywhere = "[blocker] eon/crates/eon/src/product.rs:900 0.2.0 lands before the cut";
        assert_eq!(
            line(&case(None), anywhere),
            "R01 caught blocker=1 should-fix=0 nit=0"
        );
        assert_eq!(
            line(&case(None), ""),
            "R01 missed blocker=0 should-fix=0 nit=0"
        );
    }

    #[test]
    fn the_cited_path_never_supplies_a_mechanism_word() {
        let mut product = case(None);
        product.mechanism = vec![vec!["beta".into()], vec!["product".into()]];
        let in_path = "[blocker] `eon/crates/eon/src/product.rs:3`, beta at the cut";
        assert_eq!(
            line(&product, in_path),
            "R01 disputed blocker=1 should-fix=0 nit=0"
        );
        let in_text = "[blocker] eon/crates/eon/src/product.rs:3 beta in the product";
        assert_eq!(
            line(&product, in_text),
            "R01 caught blocker=1 should-fix=0 nit=0"
        );
    }

    #[test]
    fn expected_lines_must_exist_in_the_file() {
        assert_eq!(past_end(b"a\nb\nc\n", 3), None);
        assert_eq!(past_end(b"a\nb\nc\n", 4), Some(3));
        assert_eq!(past_end(b"a\nb\nc", 3), None);
        assert_eq!(past_end(b"a\nb\nc", 4), Some(3));
        assert_eq!(past_end(b"", 1), Some(0));
        assert_eq!(past_end(b"\n", 1), None);
    }

    /// The score line up to the report hash.
    fn line(case: &Case, report: &str) -> String {
        let line = score_report(case, report, &[]);
        line.split(" report=").next().unwrap_or_default().into()
    }

    fn at(path: &str, start: u32, end: u32) -> Vec<(String, u32, u32)> {
        vec![(path.into(), start, end)]
    }

    #[test]
    fn every_citation_locates_and_none_supplies_a_word() {
        let mut lined = case(Some([20, 30]));
        let report = "[blocker] eon/crates/eon/src/main.rs:3 and (`./eon/crates/eon/src/product.rs:21`), \
                      flips beta at the cut";
        assert_eq!(
            line(&lined, report),
            "R01 caught blocker=1 should-fix=0 nit=0"
        );
        lined.mechanism = vec![vec!["beta".into()], vec!["main".into()]];
        assert_eq!(
            line(&lined, report),
            "R01 disputed blocker=1 should-fix=0 nit=0"
        );
        let found = findings(report);
        assert_eq!(found[0].text, "and flips beta at the cut");
        assert_eq!(found[0].citations.len(), 2);
    }

    #[test]
    fn cited_paths_lose_dot_and_diff_prefixes() {
        for token in [
            "./a.rs:3",
            "././a.rs:3",
            "a/a.rs:3",
            "b/a.rs:3",
            "`./a.rs:3`.",
            "(a.rs:3)",
            "(`b/a.rs:3`).",
            "[a.rs:3],",
        ] {
            assert_eq!(citation(token), Some(("a.rs".into(), 3, 3)), "{token}");
        }
        assert_eq!(citation("./:3"), None);
        assert_eq!(citation("a/:3"), None);
    }

    #[test]
    fn a_dot_prefixed_citation_rescores_as_caught() {
        let cases = parse(include_str!("../evals/cases.toml")).unwrap();
        let r02 = find(&cases.case, "R02").unwrap();
        let stored = "[blocker] venus/src/scene.rs:210 — tab_max_width is still sized for the old \
                      offset, so the point after the first pill hits New tab; inset it too.\n\
                      verdict: changes needed\n";
        assert_eq!(line(r02, stored), "R02 caught blocker=1 should-fix=0 nit=0");
        let prefixed = stored.replace(" venus/", " ./venus/");
        assert_eq!(
            line(r02, &prefixed),
            "R02 caught blocker=1 should-fix=0 nit=0"
        );
    }

    #[test]
    fn hyphen_underscore_and_space_are_one_separator() {
        let mut focus = case(None);
        focus.mechanism = vec![vec!["link-focus".into()], vec!["no-op".into()]];
        for text in [
            "link_focus is a no op",
            "link focus is a no_op",
            "Link--Focus is a NO-OP",
        ] {
            let report = format!("[blocker] eon/crates/eon/src/product.rs:1 {text}");
            assert_eq!(
                line(&focus, &report),
                "R01 caught blocker=1 should-fix=0 nit=0",
                "{text}"
            );
        }
    }

    #[test]
    fn a_located_finding_matching_some_groups_is_disputed() {
        let lined = case(Some([20, 30]));
        let report = "[blocker] eon/crates/eon/src/product.rs:22 renames a field\n\
                      [should-fix] eon/crates/eon/src/product.rs:23 beta lands here\n\
                      [should-fix] eon/crates/eon/src/main.rs:23 beta at the cut";
        assert_eq!(
            line(&lined, report),
            "R01 disputed blocker=1 should-fix=2 nit=0"
        );
    }

    #[test]
    fn retired_cases_still_validate() {
        let text = include_str!("../evals/cases.toml");
        let text = text.replacen("id = \"R01\"", "id = \"R01\"\nretired = true", 1);
        let cases = parse(&text).unwrap();
        assert!(cases.case[0].retired && !cases.case[1].retired);
        let bad = text.replacen("retired = true", "retired = \"yes\"", 1);
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn rulings_validate_and_replace_the_outcome() {
        let cases = [case(Some([20, 30]))];
        let report = "[blocker] eon/crates/eon/src/product.rs:22 beta lands here";
        let digest = sha256_hex(report.as_bytes());
        let good = format!(
            "[[ruling]]\ncase = \"R01\"\nreport = \"{digest}\"\noutcome = \"caught\"\n\
             judges = [\"opus\", \"neon\"]\n"
        );
        let rulings = parse_rulings(&good, &cases).unwrap();
        assert_eq!(
            score_report(&cases[0], report, &rulings),
            format!("R01 caught blocker=1 should-fix=0 nit=0 report={digest} ruled=opus,neon")
        );
        for outcome in ["missed", "split"] {
            let ruled = parse_rulings(&good.replace("caught", outcome), &cases).unwrap();
            let line = score_report(&cases[0], report, &ruled);
            assert!(line.starts_with(&format!("R01 {outcome} ")), "{line}");
        }
        let other = format!("{report}\n");
        assert_eq!(
            score_report(&cases[0], &other, &rulings),
            format!(
                "R01 disputed blocker=1 should-fix=0 nit=0 report={}",
                sha256_hex(other.as_bytes())
            )
        );
        for bad in [
            good.replace("\"R01\"", "\"R99\""),
            good.replace(&digest, &digest[..40]),
            good.replace(&digest, &digest.to_uppercase()),
            good.replace("\"opus\", \"neon\"", "\"opus\""),
            good.replace("\"neon\"", "\"opus\""),
            good.replace("\"neon\"", "\"gemini neon\""),
            good.replace("\"neon\"", "\"a,b\""),
            good.replace("\"neon\"", "\"\""),
            good.replace("\"caught\"", "\"gated\""),
            good.replace("judges", "note = \"x\"\njudges"),
            format!("{good}{good}"),
        ] {
            assert!(parse_rulings(&bad, &cases).is_err(), "{bad}");
        }
        let real = parse(include_str!("../evals/cases.toml")).unwrap();
        parse_rulings(include_str!("../evals/rulings.toml"), &real.case).unwrap();
    }

    #[test]
    fn score_lines_end_with_the_cases_and_scorer_hashes() {
        let tail = provenance("x");
        let (cases, scorer) = tail
            .strip_prefix("cases=")
            .and_then(|rest| rest.split_once(" scorer="))
            .unwrap();
        assert_eq!(cases, &sha256_hex(b"x")[..12]);
        assert_eq!(scorer.len(), 12);
    }

    #[test]
    fn prepare_exports_the_diff_and_score_skips_a_retired_case() {
        let repo = Repo::new("eval");
        repo.write("REVIEW.md", "review\n");
        repo.write("a.rs", "fn a() {}\n");
        repo.commit("base");
        repo.write("a.rs", "fn b() {}\n");
        let commit = repo.commit("candidate");
        let cases = case_text(&commit).replace("tier", "retired = true\ntier");
        repo.write(CASES, &cases);
        repo.write(RULINGS, "");
        let out = repo.dir.join("exported");
        let brief = prepare(&repo.git, "R01", &out, true).unwrap();
        assert_eq!(fs::read_to_string(out.join("brief.md")).unwrap(), brief);
        assert!(
            brief.contains("The diff is change.diff in this directory.") && !brief.contains("git")
        );
        let diff = fs::read_to_string(out.join("change.diff")).unwrap();
        assert!(
            diff.contains("--- a/a.rs\n+++ b/a.rs\n") && diff.contains("-fn a() {}\n+fn b() {}\n")
        );
        assert_eq!(
            fs::read_to_string(out.join("REVIEW.md")).unwrap(),
            "review\n"
        );
        let plain = repo.dir.join("plain");
        let brief = prepare(&repo.git, "R01", &plain, false).unwrap();
        assert!(brief.contains("The diff is `git diff "), "{brief}");
        assert!(!plain.join("change.diff").exists() && !plain.join("brief.md").exists());
        let line = score(&repo.git, "R01", &repo.dir.join("absent.md")).unwrap();
        assert_eq!(line, format!("R01 retired {}", provenance(&cases)));
    }
}
