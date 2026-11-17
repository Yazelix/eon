//! Review tier classifier for slices landing on `edge`.
//!
//! `review-tier classify <edge> <candidate>` decides how much review a
//! candidate commit needs (high, medium or low) from the paths and lines it
//! changes since its merge base with `edge`. The classifier that judges is
//! the one committed on the landing target: the tool refuses to run unless
//! its own source matches `<edge>:eon/tools/review-tier`, so run it from a
//! checkout of that edge commit with `nix develop -c cargo run -q -p
//! review-tier -- classify …`. REVIEW.md says what each tier requires, and
//! docs/AGENT-RULES.md "Proof and Review" binds it to landing.
//!
//! Exit codes: 0 success; 1 when `eval check` finds a failing case; 2 for a
//! usage, git, parse or provenance error, with `error: …` on stderr and
//! nothing on stdout. Callers treat 2 as High.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

mod classify;
mod eval;
mod git;
mod rules;

use classify::Verdict;
use git::Git;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// This crate's path in the repository, which `classify` requires to match
/// the edge commit.
const SOURCE: &str = "eon/tools/review-tier";

const USAGE: &str = "usage: review-tier classify <edge> <candidate> | rules | eval check \
                     | eval prepare <case-id> <dir> | eval score <case-id> <report-file>";

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string())
        .collect()
    {
        Ok(args) => args,
        Err(_) => return fail("arguments must be UTF-8".into()),
    };
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => fail(error),
    }
}

fn fail(error: Box<dyn std::error::Error>) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::from(2)
}

/// Runs one command; `Ok(false)` means `eval check` found a failing case.
fn run(args: &[String]) -> Result<bool> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = match args.as_slice() {
        ["classify", edge, candidate] => {
            git::check_revision(edge)?;
            git::check_revision(candidate)?;
            classify(&Git::current()?, edge, candidate)?
        }
        ["rules"] => rules::render(),
        ["eval", "check"] => return eval::check(&Git::current()?),
        ["eval", "prepare", id, dir] => eval::prepare(&Git::current()?, id, Path::new(dir))?,
        ["eval", "score", id, report] => {
            format!(
                "{}\n",
                eval::score(&Git::current()?, id, Path::new(report))?
            )
        }
        _ => return Err(USAGE.into()),
    };
    print!("{output}");
    Ok(true)
}

fn classify(git: &Git, edge: &str, candidate: &str) -> Result<String> {
    let edge = git.commit(edge)?;
    let candidate = git.commit(candidate)?;
    check_provenance(git, &edge)?;
    let classifier = git.rev_parse(&format!("{edge}:{SOURCE}"))?;
    let base = git.merge_base(&edge, &candidate)?;
    let base_tree = git.tree(&base)?;
    let candidate_tree = git.tree(&candidate)?;
    let verdicts = classify_trees(git, &base_tree, &candidate_tree)?;
    Ok(format!(
        "tier: {}\nclassifier: edge {edge} review-tier tree {classifier}\n\
         base: {base} tree {base_tree}\ncandidate: {candidate} tree {candidate_tree}\n{}",
        classify::slice_tier(&verdicts),
        classify::render(&verdicts),
    ))
}

/// Requires this binary's source directory to be the repository's
/// `eon/tools/review-tier` with no difference from `edge`.
fn check_provenance(git: &Git, edge: &str) -> Result<()> {
    check_source_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize()?,
        &git.root().canonicalize()?,
    )?;
    git.require_clean(edge, SOURCE)
}

/// Fails unless `source`, relative to the repository `root`, is `SOURCE`.
fn check_source_dir(source: &Path, root: &Path) -> Result<()> {
    if source.strip_prefix(root).ok() != Some(Path::new(SOURCE)) {
        return Err(format!(
            "classifier source {} is not {SOURCE} in {}",
            source.display(),
            root.display()
        )
        .into());
    }
    Ok(())
}

/// Classifies the diff between two trees; shared by `classify` and the eval.
fn classify_trees(git: &Git, base_tree: &str, candidate_tree: &str) -> Result<Vec<Verdict>> {
    let changes = classify::parse_raw(&git.raw_diff(base_tree, candidate_tree)?)?;
    let rust: Vec<&str> = changes
        .iter()
        .filter(|change| classify::needs_lines(change))
        .map(|change| change.path.as_str())
        .collect();
    let lines = if rust.is_empty() {
        BTreeMap::new()
    } else {
        classify::parse_line_diff(&git.line_diff(base_tree, candidate_tree, &rust)?, &rust)?
    };
    classify::classify(&changes, &lines, |path| {
        Ok(git.names(base_tree, path)? || git.names(candidate_tree, path)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> Result<bool> {
        run(&args
            .iter()
            .map(|arg| String::from(*arg))
            .collect::<Vec<_>>())
    }

    #[test]
    fn usage_and_option_like_arguments_are_errors() {
        assert!(run_args(&[]).is_err());
        assert!(run_args(&["classify", "edge"]).is_err());
        assert!(run_args(&["eval", "unknown"]).is_err());
        let error = run_args(&["classify", "--output=/tmp/x", "HEAD"]).unwrap_err();
        assert!(error.to_string().contains("invalid revision"));
        let error = run_args(&["classify", "HEAD", "-n"]).unwrap_err();
        assert!(error.to_string().contains("invalid revision"));
    }

    #[test]
    fn classifier_source_must_be_its_path_under_the_toplevel() {
        let root = Path::new("/repo");
        assert!(check_source_dir(Path::new("/repo/eon/tools/review-tier"), root).is_ok());
        for source in [
            "/other/eon/tools/review-tier",
            "/repo/x/eon/tools/review-tier",
            "/repo/eon/tools/review-tier/src",
            "/repo/eon/tools/archive-notices",
            "/repo",
        ] {
            let error = check_source_dir(Path::new(source), root).unwrap_err();
            assert!(error.to_string().contains("is not eon/tools/review-tier"));
        }
    }
}
