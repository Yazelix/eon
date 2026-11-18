//! The review eval: past changes that should have been stopped, a blind
//! review setup for each, and scoring of a reviewer's report.
use crate::Result;
use crate::classify::{self, Verdict};
use crate::git::Git;
use crate::rules::Tier;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const CASES: &str = "eon/tools/review-tier/evals/cases.toml";

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
pub fn parse(text: &str) -> Result<Vec<Case>> {
    let cases: Cases = toml::from_str(text).map_err(|error| format!("{CASES}: {error}"))?;
    let mut ids = BTreeSet::new();
    for case in &cases.case {
        validate(case).map_err(|error| format!("{CASES}: case {}: {error}", case.id))?;
        if !ids.insert(case.id.as_str()) {
            return Err(format!("{CASES}: duplicate case id {}", case.id).into());
        }
    }
    Ok(cases.case)
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
        if !(sha.len() == 40
            && sha
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
        {
            return Err(format!("{sha:?} is not a 40-hex commit id"));
        }
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
        if word.trim().is_empty() || *word != word.to_lowercase() {
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

pub fn load(git: &Git) -> Result<Vec<Case>> {
    let path = git.root().join(CASES);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse(&text)
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

fn classify_case(git: &Git, case: &Case) -> Result<Classified> {
    let commit = git.commit(&case.commit)?;
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
    let mut passed = true;
    for case in load(git)? {
        match check_case(git, &case) {
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
    Ok(passed)
}

fn check_case(git: &Git, case: &Case) -> Result<(Tier, Option<String>)> {
    let classified = classify_case(git, case)?;
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
/// its ancestors, with the current REVIEW.md, and returns the case block.
pub fn prepare(git: &Git, id: &str, dir: &Path) -> Result<String> {
    let cases = load(git)?;
    let case = find(&cases, id)?;
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
    let classified = classify_case(git, case)?;
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
    Ok(format!(
        "Review this change. Follow REVIEW.md at the repository root.\n\
         candidate: {}\n\
         base: {}\n\
         tier: {}\n\
         {}\
         The diff is `git diff {} HEAD`. Read files at HEAD. Do not build or run anything.\n",
        classified.commit,
        classified.base,
        classify::slice_tier(&classified.verdicts),
        classify::render(&classified.verdicts),
        classified.base,
    ))
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
    citation: Option<(String, u32, u32)>,
    text: String,
}

/// Findings in REVIEW.md's format: lines starting (after trimming and an
/// optional `- ` or `* `) with `[blocker]`, `[should-fix]` or `[nit]`. The
/// text after the tag, without the citation token, is what mechanism words
/// must match, so a cited path such as `Cargo.lock` never supplies one.
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
                let mut words: Vec<&str> = body.strip_prefix(tag)?.split_whitespace().collect();
                let cited = words
                    .iter()
                    .position(|word| citation(word).is_some())
                    .map(|index| words.remove(index));
                Some(Finding {
                    severity: *severity,
                    citation: cited.and_then(citation),
                    text: words.join(" ").to_lowercase(),
                })
            })
        })
        .collect()
}

/// `path:line` or `path:line-line`, with surrounding backticks and trailing
/// `,;:.` removed.
fn citation(token: &str) -> Option<(String, u32, u32)> {
    let punctuation: &[char] = &[',', ';', ':', '.'];
    let token = token
        .trim_end_matches(punctuation)
        .trim_matches('`')
        .trim_end_matches(punctuation);
    let (path, range) = token.rsplit_once(':')?;
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

/// `eval score`: `<id> caught|located|missed blocker=<n> should-fix=<n> nit=<n>`.
pub fn score(git: &Git, id: &str, report: &Path) -> Result<String> {
    let cases = load(git)?;
    let case = find(&cases, id)?;
    let report = fs::read_to_string(report)
        .map_err(|error| format!("cannot read {}: {error}", report.display()))?;
    Ok(score_report(case, &report))
}

fn score_report(case: &Case, report: &str) -> String {
    let findings = findings(report);
    let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
    let located: Vec<&Finding> = findings
        .iter()
        .filter(|finding| finding.severity != Severity::Nit && locates(case, finding))
        .collect();
    let caught = located.iter().any(|finding| {
        case.mechanism.iter().all(|group| {
            group
                .iter()
                .any(|word| finding.text.contains(word.as_str()))
        })
    });
    let outcome = if caught {
        "caught"
    } else if located.is_empty() {
        "missed"
    } else {
        "located"
    };
    format!(
        "{} {outcome} blocker={} should-fix={} nit={}",
        case.id,
        count(Severity::Blocker),
        count(Severity::ShouldFix),
        count(Severity::Nit)
    )
}

fn locates(case: &Case, finding: &Finding) -> bool {
    let Some((path, start, end)) = &finding.citation else {
        return false;
    };
    case.expect.iter().any(|expect| {
        expect.file == *path
            && expect.lines.is_none_or(|[low, high]| {
                *start <= high.saturating_add(5) && *end >= low.saturating_sub(5)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    #[test]
    fn the_real_cases_file_validates() {
        let cases = parse(include_str!("../evals/cases.toml")).unwrap();
        assert!(!cases.is_empty());
    }

    #[test]
    fn invalid_cases_are_rejected() {
        let base = format!(
            "[[case]]\nid = \"R01\"\nkind = \"revert\"\ncommit = \"{SHA}\"\nfixed_by = \"{SHA}\"\n\
             class = \"release-flip\"\ntier = \"high\"\nsummary = \"s\"\nevidence = \"e\"\n\
             mechanism = [[\"beta\"]]\n[[case.expect]]\nfile = \"a.rs\"\nlines = [3, 4]\n"
        );
        assert_eq!(parse(&base).unwrap().len(), 1);
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
            base.replace("[3, 4]", "[4, 3]"),
            base.replace("[3, 4]", "[3]"),
            base.replace("summary", "note = \"x\"\nsummary"),
            base.split("[[case.expect]]")
                .next()
                .unwrap_or_default()
                .into(),
            format!("{base}{base}"),
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
            found[0].citation,
            Some(("eon/crates/eon/src/product.rs".into(), 12, 14))
        );
        assert_eq!(found[1].severity, Severity::ShouldFix);
        assert_eq!(found[1].citation, Some(("venus/src/a.rs".into(), 7, 7)));
        assert_eq!(found[2].severity, Severity::Nit);
        assert_eq!(found[2].citation, None);
        assert_eq!(found[3].citation, Some(("a.rs".into(), 1, 1)));
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
            score_report(&lined, caught),
            "R01 caught blocker=1 should-fix=0 nit=1"
        );
        let located = "[should-fix] eon/crates/eon/src/product.rs:15 changes the version";
        assert_eq!(
            score_report(&lined, located),
            "R01 located blocker=0 should-fix=1 nit=0"
        );
        let far = "[blocker] eon/crates/eon/src/product.rs:36 beta at the cut";
        assert_eq!(
            score_report(&lined, far),
            "R01 missed blocker=1 should-fix=0 nit=0"
        );
        let nit = "[nit] eon/crates/eon/src/product.rs:25 beta at the cut";
        assert_eq!(
            score_report(&lined, nit),
            "R01 missed blocker=0 should-fix=0 nit=1"
        );
        let other_file = "[blocker] eon/crates/eon/src/main.rs:25 beta at the cut";
        assert_eq!(
            score_report(&lined, other_file),
            "R01 missed blocker=1 should-fix=0 nit=0"
        );
        let anywhere = "[blocker] eon/crates/eon/src/product.rs:900 0.2.0 lands before the cut";
        assert_eq!(
            score_report(&case(None), anywhere),
            "R01 caught blocker=1 should-fix=0 nit=0"
        );
        assert_eq!(
            score_report(&case(None), ""),
            "R01 missed blocker=0 should-fix=0 nit=0"
        );
    }

    #[test]
    fn the_cited_path_never_supplies_a_mechanism_word() {
        let mut product = case(None);
        product.mechanism = vec![vec!["beta".into()], vec!["product".into()]];
        let in_path = "[blocker] `eon/crates/eon/src/product.rs:3`, beta at the cut";
        assert_eq!(
            score_report(&product, in_path),
            "R01 located blocker=1 should-fix=0 nit=0"
        );
        let in_text = "[blocker] eon/crates/eon/src/product.rs:3 beta in the product";
        assert_eq!(
            score_report(&product, in_text),
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
}
