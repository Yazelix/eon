//! A thin layer over the `git` executable: argument vectors, never a shell,
//! and every call with `core.quotepath` off, diff prefixes pinned and the
//! environment that could change git's output removed.
use crate::Result;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Options every diff passes so that user configuration cannot change the
/// text this tool parses.
const DIFF: &[&str] = &[
    "--no-ext-diff",
    "--no-textconv",
    "--text",
    "--no-renames",
    "--no-color",
    "--ignore-submodules=none",
];

/// Environment variables that change what git reports for the same
/// arguments: pathspec magic, diff options, and configuration injected
/// around the `-c` settings each call passes.
const UNSET: &[&str] = &[
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_DIFF_OPTS",
    "GIT_EXTERNAL_DIFF",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];

pub struct Git {
    root: PathBuf,
}

impl Git {
    /// The repository holding the current directory, at its toplevel.
    pub fn current() -> Result<Self> {
        let output = Self::command(None)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        let root = text(&checked(&["rev-parse", "--show-toplevel"], output)?)?;
        Ok(Self {
            root: PathBuf::from(root.trim_end_matches('\n')),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn command(root: Option<&Path>) -> Command {
        let mut command = Command::new("git");
        if let Some(root) = root {
            command.arg("-C").arg(root);
        }
        command
            .args([
                "-c",
                "core.quotepath=off",
                "-c",
                "diff.noprefix=false",
                "-c",
                "diff.mnemonicPrefix=false",
                "-c",
                "diff.ignoreSubmodules=none",
                "-c",
                "diff.renames=false",
            ])
            .stdin(Stdio::null());
        for name in UNSET {
            command.env_remove(name);
        }
        command
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        Self::command(Some(&self.root))
            .args(args)
            .output()
            .map_err(|error| format!("cannot run git {}: {error}", args.join(" ")).into())
    }

    /// Runs git and returns its standard output, or an error on failure.
    pub fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        checked(args, self.output(args)?)
    }

    fn line(&self, args: &[&str]) -> Result<String> {
        Ok(text(&self.run(args)?)?.trim_end_matches('\n').into())
    }

    /// Resolves a user-supplied revision to a commit id.
    pub fn commit(&self, revision: &str) -> Result<String> {
        check_revision(revision)?;
        self.line(&[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ])
        .map_err(|_| format!("{revision:?} is not a commit").into())
    }

    /// Resolves an object name such as `<commit>^{tree}` or `<commit>:<path>`.
    pub fn rev_parse(&self, object: &str) -> Result<String> {
        self.line(&["rev-parse", "--verify", "--end-of-options", object])
    }

    pub fn tree(&self, commit: &str) -> Result<String> {
        self.rev_parse(&format!("{commit}^{{tree}}"))
    }

    pub fn first_parent(&self, commit: &str) -> Result<String> {
        self.rev_parse(&format!("{commit}^1^{{commit}}"))
            .map_err(|_| format!("{commit} has no parent").into())
    }

    pub fn merge_base(&self, left: &str, right: &str) -> Result<String> {
        self.line(&["merge-base", "--end-of-options", left, right])
    }

    /// Fails unless the working tree's `dir` matches `commit` exactly, with
    /// no untracked files.
    pub fn require_clean(&self, commit: &str, dir: &str) -> Result<()> {
        let mut args = vec!["diff", "--quiet"];
        args.extend(DIFF);
        args.extend([commit, "--", dir]);
        let output = self.output(&args)?;
        let differs = match output.status.code() {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(failure(&args, &output).into()),
        };
        let untracked = self.run(&["ls-files", "--others", "--exclude-standard", "--", dir])?;
        if differs || !untracked.is_empty() {
            return Err(format!("classifier source differs from {commit}").into());
        }
        Ok(())
    }

    /// `git diff --raw -z` between two trees.
    pub fn raw_diff(&self, base: &str, candidate: &str) -> Result<Vec<u8>> {
        self.run(&raw_diff_args(base, candidate))
    }

    /// `git diff -U0` between two trees, limited to `paths` taken literally.
    pub fn line_diff(&self, base: &str, candidate: &str, paths: &[&str]) -> Result<Vec<u8>> {
        self.run(&line_diff_args(base, candidate, paths))
    }

    /// Whether any `.rs` or `.nix` file in `tree` contains `path` or its
    /// file name.
    pub fn names(&self, tree: &str, path: &str) -> Result<bool> {
        let args = names_args(tree, path);
        let output = self.output(&args)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failure(&args, &output).into()),
        }
    }

    /// `git init -q <dir>`.
    pub fn init(dir: &Path) -> Result<()> {
        let output = Self::command(None)
            .args(["init", "-q"])
            .arg(dir)
            .output()
            .map_err(|error| format!("cannot run git init: {error}"))?;
        checked(&["init"], output).map(drop)
    }

    /// Opens `dir` as a repository without searching parents.
    pub fn at(dir: &Path) -> Self {
        Self {
            root: dir.to_path_buf(),
        }
    }
}

fn raw_diff_args<'a>(base: &'a str, candidate: &'a str) -> Vec<&'a str> {
    let mut args = vec!["diff", "--raw", "-z", "--no-abbrev"];
    args.extend(DIFF);
    args.extend([base, candidate]);
    args
}

fn line_diff_args<'a>(base: &'a str, candidate: &'a str, paths: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "-U0",
        // Hunk bodies are compared across diffs, so nothing configurable may
        // merge hunks or move their boundaries.
        "--inter-hunk-context=0",
        "--diff-algorithm=myers",
        "--indent-heuristic",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    args.extend(DIFF);
    args.extend([base, candidate, "--"]);
    args.extend(paths);
    args
}

fn names_args<'a>(tree: &'a str, path: &'a str) -> [&'a str; 11] {
    let name = path.rsplit('/').next().unwrap_or(path);
    [
        "grep", "-F", "-l", "-e", path, "-e", name, tree, "--", "*.rs", "*.nix",
    ]
}

/// Rejects a revision argument that git could read as an option.
pub fn check_revision(revision: &str) -> Result<()> {
    if revision.is_empty() || revision.starts_with('-') {
        return Err(format!("invalid revision {revision:?}").into());
    }
    Ok(())
}

fn checked(args: &[&str], output: Output) -> Result<Vec<u8>> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failure(args, &output).into())
    }
}

fn failure(args: &[&str], output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = args
        .iter()
        .find(|arg| !arg.starts_with('-') && !arg.contains('='))
        .unwrap_or(&"");
    format!("git {first} failed ({}): {}", output.status, stderr.trim())
}

fn text(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| "git printed non-UTF-8 output".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_like_revisions_are_rejected() {
        assert!(check_revision("-n").is_err());
        assert!(check_revision("--output=x").is_err());
        assert!(check_revision("").is_err());
        assert!(check_revision("HEAD~3").is_ok());
        assert!(check_revision("edge").is_ok());
    }

    #[test]
    fn every_command_drops_environment_that_changes_git_output() {
        let command = Git::command(None);
        let removed: Vec<&std::ffi::OsStr> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name)
            .collect();
        for name in [
            "GIT_LITERAL_PATHSPECS",
            "GIT_GLOB_PATHSPECS",
            "GIT_NOGLOB_PATHSPECS",
            "GIT_ICASE_PATHSPECS",
            "GIT_DIFF_OPTS",
            "GIT_EXTERNAL_DIFF",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ] {
            assert!(removed.contains(&std::ffi::OsStr::new(name)), "{name}");
        }
    }

    /// Each pin overrides configuration that would otherwise change the diff
    /// text this tool parses.
    #[test]
    fn a_reviewer_config_with_diff_ignore_submodules_all_cannot_hide_a_gitlink_change() {
        let command = Git::command(None);
        let pinned: Vec<_> = command.get_args().collect();
        assert_eq!(
            pinned,
            [
                "-c",
                "core.quotepath=off",
                "-c",
                "diff.noprefix=false",
                "-c",
                "diff.mnemonicPrefix=false",
                "-c",
                "diff.ignoreSubmodules=none",
                "-c",
                "diff.renames=false",
            ]
        );
        let raw = raw_diff_args("B", "C");
        let lines = line_diff_args("B", "C", &["a.rs"]);
        for args in [&raw, &lines] {
            for option in [
                "--ignore-submodules=none",
                "--no-ext-diff",
                "--no-textconv",
                "--text",
                "--no-renames",
                "--no-color",
            ] {
                assert!(args.contains(&option), "{option} in {args:?}");
            }
        }
        for option in [
            "-U0",
            "--inter-hunk-context=0",
            "--diff-algorithm=myers",
            "--indent-heuristic",
        ] {
            assert!(lines.contains(&option), "{option} in {lines:?}");
        }
        assert_eq!(raw[..4], ["diff", "--raw", "-z", "--no-abbrev"]);
        assert_eq!(raw[raw.len() - 2..], ["B", "C"]);
        assert_eq!(lines[..2], ["--literal-pathspecs", "diff"]);
        assert_eq!(lines[lines.len() - 4..], ["B", "C", "--", "a.rs"]);
    }

    #[test]
    fn markdown_search_takes_full_path_and_file_name_in_rust_and_nix() {
        assert_eq!(
            names_args("T", "docs/guide/a b.md"),
            [
                "grep",
                "-F",
                "-l",
                "-e",
                "docs/guide/a b.md",
                "-e",
                "a b.md",
                "T",
                "--",
                "*.rs",
                "*.nix",
            ]
        );
        assert_eq!(
            names_args("T", "README.md")[4..7],
            ["README.md", "-e", "README.md"]
        );
    }
}
