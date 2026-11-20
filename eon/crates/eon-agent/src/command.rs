//! Command templates: the placeholders launch and run commands share, their validation, and
//! one expander that both exec and dry-run display use.
//!
//! Whole-argument placeholders expand to zero or more arguments: `{prompt}` (launch, exactly
//! one), `{add_dirs}` (the backend's `add_dir` template once per `--add-dir`) and `{overlay}`
//! (the project role's overlay for the command's executable). Every other placeholder, such as
//! `{name}` or `{cwd}`, is filled inside its argument in one left-to-right pass, so substituted
//! text is never rescanned.
use crate::launch::Failure;
use std::fs;
use std::path::{Path, PathBuf};

pub const PROMPT: &str = "{prompt}";
pub const ADD_DIRS: &str = "{add_dirs}";
pub const OVERLAY: &str = "{overlay}";
pub const DIR: &str = "{dir}";
pub const NAME: &str = "{name}";
/// Every placeholder any command fills; none may appear in a program.
const ALL: [&str; 9] = [
    PROMPT, ADD_DIRS, OVERLAY, DIR, NAME, "{cwd}", "{brief}", "{report}", "{effort}",
];

/// Which placeholders a kind of command accepts.
#[derive(Clone, Copy)]
pub struct Shape {
    /// Exactly one whole `{prompt}` argument (launch level commands).
    pub prompt: bool,
    /// `{add_dirs}` is allowed: the command has an `add_dir` template.
    pub add_dirs: bool,
    /// `{overlay}` and `{name}` are allowed (level and run commands, not `popup_command`).
    pub overlay_and_name: bool,
}

/// Why `argv` breaks `shape`, or `None`.
pub fn problem(argv: &[String], shape: Shape) -> Option<&'static str> {
    if argv
        .first()
        .is_some_and(|program| ALL.iter().any(|p| program.contains(p)))
    {
        return Some("a placeholder cannot be the program");
    }
    let whole = |placeholder: &str| {
        argv.iter()
            .filter(|a| a.contains(placeholder))
            .all(|a| a == placeholder)
    };
    let count = |placeholder: &str| argv.iter().filter(|a| *a == placeholder).count();
    if shape.prompt {
        if !whole(PROMPT) || count(PROMPT) != 1 {
            return Some("needs exactly one whole {prompt} argument");
        }
    } else if argv.iter().any(|a| a.contains(PROMPT)) {
        return Some("takes no {prompt}");
    }
    if !whole(ADD_DIRS) || count(ADD_DIRS) > 1 {
        return Some("{add_dirs} must be one whole argument");
    }
    if count(ADD_DIRS) == 1 && !shape.add_dirs {
        return Some("{add_dirs} needs an add_dir template with one {dir}");
    }
    if !whole(OVERLAY) || count(OVERLAY) > 1 {
        return Some("{overlay} must be one whole argument");
    }
    if !shape.overlay_and_name && argv.iter().any(|a| a.contains(OVERLAY) || a.contains(NAME)) {
        return Some("takes no {overlay} or {name}");
    }
    if argv.iter().any(|a| a.contains(DIR)) {
        return Some("{dir} belongs only in an add_dir template");
    }
    None
}

/// Whether an `add_dir` template is valid: exactly one `{dir}` in total, and no NUL or other
/// whole-argument placeholder.
pub fn template_valid(template: &[String]) -> bool {
    template
        .iter()
        .map(|a| a.matches(DIR).count())
        .sum::<usize>()
        == 1
        && template.iter().all(|a| {
            !a.contains('\0') && ![PROMPT, ADD_DIRS, OVERLAY].iter().any(|p| a.contains(p))
        })
}

/// Replaces each `{name}` in one left-to-right pass, so substituted text is never rescanned.
pub fn fill(arg: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = arg;
    'scan: while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        for (name, value) in values {
            if let Some(after) = rest.strip_prefix(name) {
                out.push_str(value);
                rest = after;
                continue 'scan;
            }
        }
        out.push('{');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// One expanded argument: text, or the launch prompt, which exec and display render apart.
#[derive(Debug, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Prompt,
}

/// What a command expands with.
pub struct Values<'a> {
    /// Placeholders filled inside arguments, such as `("{cwd}", "/w")`.
    pub fills: &'a [(&'a str, &'a str)],
    /// The backend's `add_dir` template for this kind of command.
    pub template: Option<&'a [String]>,
    pub add_dirs: &'a [&'a str],
    pub overlay: &'a [String],
}

pub fn expand(argv: &[String], values: &Values<'_>) -> Vec<Part> {
    let mut parts = Vec::new();
    for arg in argv {
        match arg.as_str() {
            PROMPT => parts.push(Part::Prompt),
            ADD_DIRS => {
                for dir in values.add_dirs {
                    for part in values.template.into_iter().flatten() {
                        parts.push(Part::Text(fill(part, &[(DIR, dir)])));
                    }
                }
            }
            OVERLAY => parts.extend(
                values
                    .overlay
                    .iter()
                    .map(|arg| Part::Text(fill(arg, values.fills))),
            ),
            _ => parts.push(Part::Text(fill(arg, values.fills))),
        }
    }
    parts
}

/// The argv to exec: `prompt` replaces the `{prompt}` argument.
pub fn argv(parts: Vec<Part>, prompt: &str) -> Vec<String> {
    parts
        .into_iter()
        .map(|part| match part {
            Part::Text(text) => text,
            Part::Prompt => prompt.to_owned(),
        })
        .collect()
}

/// The executable's base name, by which project overlays are keyed.
pub fn executable(argv: &[String]) -> &str {
    argv.first()
        .and_then(|program| Path::new(program).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// Why a command cannot take these options, or `None`: `--add-dir` needs `{add_dirs}`, and
/// `{name}` in the command or its overlay needs `--name`.
pub fn unsupported(
    argv: &[String],
    overlay: &[String],
    add_dirs: bool,
    name: bool,
    what: &str,
) -> Option<String> {
    if add_dirs && !argv.iter().any(|a| a == ADD_DIRS) {
        return Some(format!("the {what} command takes no --add-dir"));
    }
    if !name && argv.iter().chain(overlay).any(|a| a.contains(NAME)) {
        return Some(format!("the {what} command needs --name"));
    }
    None
}

/// A `--name` value: 1-64 letters, digits, '.', '_' or '-', not starting with '.' or '-'.
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

pub const BAD_NAME: &str =
    "bad --name (use 1-64 letters, digits, '.', '_' or '-', not starting with '.' or '-')";

/// Canonical `--add-dir` directories, each required to be a directory.
pub fn canonical_dirs(dirs: &[PathBuf]) -> Result<Vec<PathBuf>, Failure> {
    dirs.iter()
        .map(|dir| {
            let dir =
                fs::canonicalize(dir).map_err(|e| (2, format!("cannot use --add-dir: {e}")))?;
            if dir.is_dir() {
                Ok(dir)
            } else {
                Err((2, "--add-dir must be a directory".to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const LAUNCH: Shape = Shape {
        prompt: true,
        add_dirs: true,
        overlay_and_name: true,
    };

    #[test]
    fn fill_is_one_pass_and_never_rescans() {
        let values = [("{cwd}", "/w/{name}"), ("{name}", "n")];
        assert_eq!(fill("{cwd}/{name}/{x}", &values), "/w/{name}/n/{x}");
        assert_eq!(fill("{{name}}", &values), "{n}");
    }

    #[test]
    fn expands_every_placeholder_kind() {
        let argv = args(&[
            "claude",
            "-n",
            "{name}",
            "--session=x-{name}",
            "{overlay}",
            "{add_dirs}",
            "--",
            "{prompt}",
        ]);
        assert_eq!(problem(&argv, LAUNCH), None);
        let template = args(&["--add-dir={dir}"]);
        let overlay = args(&["--settings", "{cwd}/s.json"]);
        let parts = expand(
            &argv,
            &Values {
                fills: &[("{cwd}", "/w"), ("{name}", "tide-1")],
                template: Some(&template),
                add_dirs: &["/a", "/b{name}"],
                overlay: &overlay,
            },
        );
        assert_eq!(
            argv_of(parts, "{name} `x`"),
            [
                "claude",
                "-n",
                "tide-1",
                "--session=x-tide-1",
                "--settings",
                "/w/s.json",
                "--add-dir=/a",
                "--add-dir=/b{name}",
                "--",
                "{name} `x`"
            ]
        );
        // A two-part template repeats both parts; no directories and no overlay add nothing.
        let template = args(&["--add-dir", "{dir}"]);
        let values = Values {
            fills: &[],
            template: Some(&template),
            add_dirs: &["/a", "/b"],
            overlay: &[],
        };
        let parts = expand(&args(&["x", "{overlay}", "{add_dirs}"]), &values);
        assert_eq!(
            argv_of(parts, ""),
            ["x", "--add-dir", "/a", "--add-dir", "/b"]
        );
        let values = Values {
            add_dirs: &[],
            ..values
        };
        let parts = expand(&args(&["x", "{overlay}", "{add_dirs}"]), &values);
        assert_eq!(argv_of(parts, ""), ["x"]);
    }

    fn argv_of(parts: Vec<Part>, prompt: &str) -> Vec<String> {
        argv(parts, prompt)
    }

    #[test]
    fn refuses_misplaced_placeholders() {
        for (argv, shape, why) in [
            (&["{prompt}"][..], LAUNCH, "program"),
            (&["{overlay}", "{prompt}"], LAUNCH, "program"),
            (&["x"], LAUNCH, "exactly one"),
            (&["x", "{prompt}", "{prompt}"], LAUNCH, "exactly one"),
            (&["x", "{prompt}", "a{prompt}"], LAUNCH, "exactly one"),
            (&["x", "a{add_dirs}", "{prompt}"], LAUNCH, "{add_dirs} must"),
            (
                &["x", "{add_dirs}", "{add_dirs}", "{prompt}"],
                LAUNCH,
                "{add_dirs} must",
            ),
            (
                &["x", "{overlay}", "{overlay}", "{prompt}"],
                LAUNCH,
                "{overlay} must",
            ),
            (&["x", "-{overlay}", "{prompt}"], LAUNCH, "{overlay} must"),
            (&["x", "{dir}", "{prompt}"], LAUNCH, "{dir}"),
        ] {
            let found = problem(&args(argv), shape).unwrap_or_default();
            assert!(found.contains(why), "{argv:?}: {found}");
        }
        let no_template = Shape {
            add_dirs: false,
            ..LAUNCH
        };
        assert!(
            problem(&args(&["x", "{add_dirs}", "{prompt}"]), no_template)
                .unwrap()
                .contains("needs an add_dir template")
        );
        let popup = Shape {
            prompt: false,
            add_dirs: false,
            overlay_and_name: false,
        };
        for argv in [
            &["claude", "{prompt}"][..],
            &["claude", "{overlay}"],
            &["c", "-n={name}"],
        ] {
            assert!(problem(&args(argv), popup).is_some(), "{argv:?}");
        }
        assert_eq!(
            problem(&args(&["claude", "--resume", "{cwd}"]), popup),
            None
        );
    }

    #[test]
    fn templates_need_exactly_one_dir() {
        assert!(template_valid(&args(&["--add-dir={dir}"])));
        assert!(template_valid(&args(&["--add-dir", "{dir}"])));
        for bad in [
            &[][..],
            &["--add-dir"],
            &["{dir}", "{dir}"],
            &["{dir}={dir}"],
            &["{dir}", "{overlay}"],
        ] {
            assert!(!template_valid(&args(bad)), "{bad:?}");
        }
    }

    #[test]
    fn unsupported_names_the_missing_option() {
        let argv = args(&["c", "{add_dirs}", "{prompt}"]);
        assert_eq!(unsupported(&argv, &[], true, false, "high"), None);
        let plain = args(&["c", "{prompt}"]);
        assert_eq!(
            unsupported(&plain, &[], true, true, "high").unwrap(),
            "the high command takes no --add-dir"
        );
        let named = args(&["c", "-n", "{name}", "{prompt}"]);
        assert_eq!(
            unsupported(&named, &[], false, false, "high").unwrap(),
            "the high command needs --name"
        );
        assert_eq!(unsupported(&named, &[], false, true, "high"), None);
        // An overlay alone can need --name.
        let overlay = args(&["--session={name}"]);
        assert_eq!(
            unsupported(&plain, &overlay, false, false, "high").unwrap(),
            "the high command needs --name"
        );
    }

    #[test]
    fn names_and_executables() {
        assert!(valid_name("tide-1.a_b"));
        for bad in ["", ".x", "-x", "a b", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert_eq!(executable(&args(&["/usr/bin/codex", "exec"])), "codex");
        assert_eq!(executable(&args(&["claude"])), "claude");
    }
}
