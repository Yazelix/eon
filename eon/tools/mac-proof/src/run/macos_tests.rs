//! Keeps `LINUX_ONLY_TEST_PACKAGES` equal to the workspace packages that compile
//! no tests on macOS, by reading their source on any system.
//!
//! The Mac only trusts the list: a listed package's verified empty result is
//! not-applicable. This check makes the list exact on Linux first, so a new
//! Linux-only package fails here until it is listed, and a listed package that
//! gains macOS tests fails here until it is removed.
//!
//! The reading is a lexer, not a compiler. It follows what decides whether a
//! test exists on aarch64-darwin: crate-level and module-level `cfg`s, file and
//! inline modules, `#[path]`, and `#[test]` functions with their `cfg`s and
//! `#[ignore]`, with `cfg_attr` expanded for macOS. The Mac runs every package
//! with its default features, so `feature = "x"` holds when the package's
//! `default` enables x. A `cfg` predicate it cannot
//! evaluate, a module file it cannot find, a path outside the repository, an
//! unbalanced group or an unterminated literal panics rather than guesses.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Punct(char),
}

/// Rust tokens without comments, with string and raw-string literals as one
/// token each, raw identifiers kept with their `r#` so they never match a
/// keyword, and character literals and lifetimes dropped.
fn lex(text: &str, file: &Path) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize| chars.get(index).copied();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(c) = at(index) {
        if c.is_whitespace() {
            index += 1;
        } else if c == '/' && at(index + 1) == Some('/') {
            while at(index).is_some_and(|c| c != '\n') {
                index += 1;
            }
        } else if c == '/' && at(index + 1) == Some('*') {
            let mut depth = 0;
            loop {
                match (at(index), at(index + 1)) {
                    (Some('/'), Some('*')) => {
                        depth += 1;
                        index += 2;
                    }
                    (Some('*'), Some('/')) => {
                        depth -= 1;
                        index += 2;
                        if depth == 0 {
                            break;
                        }
                    }
                    (Some(_), _) => index += 1,
                    (None, _) => panic!("{}: unterminated block comment", file.display()),
                }
            }
        } else if c == '"' {
            index += 1;
            let start = index;
            loop {
                match at(index) {
                    Some('\\') => index += 2,
                    Some('"') => break,
                    Some(_) => index += 1,
                    None => panic!("{}: unterminated string", file.display()),
                }
            }
            tokens.push(Token::Str(chars[start..index].iter().collect()));
            index += 1;
        } else if c == '\'' {
            if at(index + 1) == Some('\\') {
                index += 3;
                while at(index).is_some_and(|c| c != '\'') {
                    index += 1;
                }
                if at(index).is_none() {
                    panic!("{}: unterminated character literal", file.display());
                }
                index += 1;
            } else if at(index + 2) == Some('\'') {
                index += 3;
            } else {
                // A lifetime or label.
                index += 1;
                while at(index).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    index += 1;
                }
            }
        } else if c.is_alphanumeric() || c == '_' {
            let start = index;
            while at(index).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            let mut hashes = 0;
            while at(index + hashes) == Some('#') {
                hashes += 1;
            }
            let raw = matches!(word.as_str(), "r" | "br" | "cr");
            if raw && at(index + hashes) == Some('"') {
                index += hashes + 1;
                let start = index;
                loop {
                    match at(index) {
                        Some('"') if (1..=hashes).all(|n| at(index + n) == Some('#')) => break,
                        Some(_) => index += 1,
                        None => panic!("{}: unterminated raw string", file.display()),
                    }
                }
                tokens.push(Token::Str(chars[start..index].iter().collect()));
                index += hashes + 1;
            } else if word == "r" && hashes == 1 {
                // A raw identifier such as r#type.
                index += 1;
                let start = index;
                while at(index).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    index += 1;
                }
                let name: String = chars[start..index].iter().collect();
                tokens.push(Token::Ident(format!("r#{name}")));
            } else if matches!(word.as_str(), "b" | "c") && at(index) == Some('"') {
                // A byte or C string: the next round reads the literal.
            } else if word == "b" && at(index) == Some('\'') {
                // A byte character: the next round drops it.
            } else {
                tokens.push(Token::Ident(word));
            }
        } else {
            tokens.push(Token::Punct(c));
            index += 1;
        }
    }
    tokens
}

fn ident(token: Option<&Token>) -> Option<&str> {
    match token {
        Some(Token::Ident(word)) => Some(word),
        _ => None,
    }
}

/// The tokens inside the bracketed group that opens at `*at`, which then
/// points past its close.
fn group(tokens: &[Token], at: &mut usize, open: char, close: char, file: &Path) -> Vec<Token> {
    assert_eq!(tokens.get(*at), Some(&Token::Punct(open)));
    let start = *at + 1;
    let mut depth = 0;
    while let Some(token) = tokens.get(*at) {
        *at += 1;
        if *token == Token::Punct(open) {
            depth += 1;
        } else if *token == Token::Punct(close) {
            depth -= 1;
            if depth == 0 {
                return tokens[start..*at - 1].to_vec();
            }
        }
    }
    panic!("{}: unclosed {open}", file.display())
}

/// The inner attributes (`#![...]`) starting at `*at`, as the tokens inside
/// their brackets.
fn inner_attributes(tokens: &[Token], at: &mut usize, file: &Path) -> Vec<Vec<Token>> {
    let mut attributes = Vec::new();
    while tokens.get(*at) == Some(&Token::Punct('#'))
        && tokens.get(*at + 1) == Some(&Token::Punct('!'))
        && tokens.get(*at + 2) == Some(&Token::Punct('['))
    {
        *at += 2;
        attributes.push(group(tokens, at, '[', ']', file));
    }
    attributes
}

/// Whether a `cfg(...)` attribute holds on aarch64-darwin under test; any
/// other attribute holds.
fn holds_on_macos(attribute: &[Token], file: &Path, features: &Features) -> bool {
    if ident(attribute.first()) != Some("cfg") {
        return true;
    }
    let mut at = 1;
    let predicate = group(attribute, &mut at, '(', ')', file);
    match split_list(&predicate).as_slice() {
        [predicate] if at == attribute.len() => predicate_holds(predicate, file, features),
        _ => panic!("{}: malformed cfg {attribute:?}", file.display()),
    }
}

/// An attribute as macOS sees it: a `cfg_attr` becomes the attributes it
/// applies when its predicate holds on macOS, and nothing otherwise.
fn expand(attribute: Vec<Token>, file: &Path, features: &Features) -> Vec<Vec<Token>> {
    if ident(attribute.first()) != Some("cfg_attr") {
        return vec![attribute];
    }
    let mut at = 1;
    let list = group(&attribute, &mut at, '(', ')', file);
    let mut parts = split_list(&list).into_iter();
    match parts.next() {
        Some(predicate) if at == attribute.len() => {
            if predicate_holds(&predicate, file, features) {
                parts
                    .flat_map(|part| expand(part, file, features))
                    .collect()
            } else {
                Vec::new()
            }
        }
        _ => panic!("{}: malformed cfg_attr {attribute:?}", file.display()),
    }
}

/// Whether every attribute, expanded for macOS, holds there.
fn all_hold(attributes: &[Vec<Token>], file: &Path, features: &Features) -> bool {
    attributes
        .iter()
        .flat_map(|attribute| expand(attribute.clone(), file, features))
        .all(|attribute| holds_on_macos(&attribute, file, features))
}

fn predicate_holds(predicate: &[Token], file: &Path, features: &Features) -> bool {
    let unknown = || panic!("{}: cannot evaluate cfg({predicate:?})", file.display());
    match predicate {
        [Token::Ident(name)] => match name.as_str() {
            "test" | "unix" | "debug_assertions" => true,
            "windows" => false,
            _ => unknown(),
        },
        [Token::Ident(name), Token::Punct('='), Token::Str(value)] => match name.as_str() {
            "target_os" => value == "macos",
            "target_arch" => value == "aarch64",
            "target_family" => value == "unix",
            "target_vendor" => value == "apple",
            "feature" => match features.holds(value) {
                Some(on) => on,
                None => unknown(),
            },
            _ => unknown(),
        },
        [Token::Ident(name), Token::Punct('('), .., Token::Punct(')')] => {
            let mut at = 1;
            let inner = group(predicate, &mut at, '(', ')', file);
            if at != predicate.len() {
                unknown();
            }
            let parts = split_list(&inner);
            match name.as_str() {
                "not" if parts.len() == 1 => !predicate_holds(&parts[0], file, features),
                "any" => parts
                    .iter()
                    .any(|part| predicate_holds(part, file, features)),
                "all" => parts
                    .iter()
                    .all(|part| predicate_holds(part, file, features)),
                _ => unknown(),
            }
        }
        _ => unknown(),
    }
}

/// The comma-separated items of a predicate list, split at its top level.
fn split_list(list: &[Token]) -> Vec<Vec<Token>> {
    let mut parts = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0;
    for token in list {
        match token {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => depth -= 1,
            Token::Punct(',') if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => (),
        }
        current.push(token.clone());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// `path` resolved, required to lie inside `root`.
fn inside(root: &Path, path: &Path) -> PathBuf {
    let resolved = path
        .canonicalize()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        resolved.starts_with(root),
        "{} resolves outside {}",
        path.display(),
        root.display()
    );
    resolved
}

/// The features a package declares, and those its `default` enables.
struct Features {
    declared: BTreeSet<String>,
    default: BTreeSet<String>,
}

impl Features {
    /// From a `cargo metadata` package; no `features` table declares none.
    fn of(package: &serde_json::Value) -> Self {
        let table = package["features"].as_object();
        let declared: BTreeSet<String> = table
            .into_iter()
            .flatten()
            .map(|(name, _)| name.clone())
            .collect();
        let mut default = BTreeSet::new();
        let mut pending = vec!["default".to_owned()];
        while let Some(name) = pending.pop() {
            if !declared.contains(&name) || !default.insert(name.clone()) {
                continue;
            }
            for entry in table
                .and_then(|table| table[&name].as_array())
                .into_iter()
                .flatten()
            {
                let entry = entry.as_str().expect("feature entries are strings");
                // `x/f` also turns on the optional dependency x and its implicit feature;
                // `dep:x` and `x?/f` turn on no feature of this package.
                if let Some((dependency, _)) = entry.split_once('/') {
                    pending.push(dependency.to_owned());
                } else if !entry.starts_with("dep:") {
                    pending.push(entry.to_owned());
                }
            }
        }
        Self { declared, default }
    }

    /// Whether a target's `required-features` list (absent: none) is all on
    /// with default features.
    fn enabled_by_default(&self, required: &serde_json::Value) -> bool {
        required.as_array().into_iter().flatten().all(|name| {
            let name = name.as_str().expect("required features are strings");
            self.default.contains(name)
        })
    }

    /// Whether `name` is on with default features; `None` when the package
    /// does not declare it, so a misspelled feature cannot pass as off.
    fn holds(&self, name: &str) -> Option<bool> {
        self.declared
            .contains(name)
            .then(|| self.default.contains(name))
    }
}

struct Walk<'a> {
    root: &'a Path,
    features: &'a Features,
    tests: usize,
}

impl Walk<'_> {
    /// Counts the macOS tests in a module file and the modules it declares,
    /// and returns the file's inner attributes. `dir` holds its child modules.
    fn file(&mut self, path: &Path, dir: &Path) -> Vec<Vec<Token>> {
        let features = self.features;
        let path = inside(self.root, path);
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let tokens = lex(&text, &path);
        let mut at = 0;
        let attributes = inner_attributes(&tokens, &mut at, &path);
        if all_hold(&attributes, &path, features) {
            self.items(&tokens, &mut at, dir, &path, false);
        }
        attributes
    }

    /// Reads items up to the end of the file or, `inline`, to the brace that
    /// closes the inline module.
    fn items(&mut self, tokens: &[Token], at: &mut usize, dir: &Path, file: &Path, inline: bool) {
        let features = self.features;
        let mut pending: Vec<Vec<Token>> = Vec::new();
        let mut depth = 0usize;
        while let Some(token) = tokens.get(*at) {
            match token {
                Token::Punct('#') if tokens.get(*at + 1) == Some(&Token::Punct('[')) => {
                    *at += 1;
                    let attribute = group(tokens, at, '[', ']', file);
                    pending.extend(expand(attribute, file, features));
                }
                Token::Punct('#')
                    if tokens.get(*at + 1) == Some(&Token::Punct('!'))
                        && tokens.get(*at + 2) == Some(&Token::Punct('[')) =>
                {
                    *at += 2;
                    group(tokens, at, '[', ']', file);
                }
                Token::Ident(word) if word == "mod" => {
                    let name = ident(tokens.get(*at + 1))
                        .unwrap_or_else(|| panic!("{}: mod without a name", file.display()));
                    let name = name.strip_prefix("r#").unwrap_or(name).to_owned();
                    *at += 2;
                    let enabled = pending
                        .iter()
                        .all(|attribute| holds_on_macos(attribute, file, features));
                    let path = pending
                        .iter()
                        .find_map(|attribute| match attribute.as_slice() {
                            [Token::Ident(key), Token::Punct('='), Token::Str(value)]
                                if key == "path" =>
                            {
                                Some(value.clone())
                            }
                            _ => None,
                        });
                    pending.clear();
                    // rustc reads #[path] beside the declaring file, or inside
                    // the enclosing inline module's directory.
                    let base = if inline {
                        dir
                    } else {
                        file.parent().expect("a source file has a directory")
                    };
                    match tokens.get(*at) {
                        Some(Token::Punct(';')) => {
                            *at += 1;
                            if enabled {
                                let child = match &path {
                                    Some(path) => base.join(path),
                                    None => {
                                        let flat = dir.join(format!("{name}.rs"));
                                        if flat.exists() {
                                            flat
                                        } else {
                                            dir.join(&name).join("mod.rs")
                                        }
                                    }
                                };
                                let child_dir = match path {
                                    Some(_) => child.parent().unwrap().to_owned(),
                                    None => dir.join(&name),
                                };
                                self.file(&child, &child_dir);
                            }
                        }
                        Some(Token::Punct('{')) => {
                            let open = *at;
                            *at += 1;
                            if enabled
                                && all_hold(&inner_attributes(tokens, at, file), file, features)
                            {
                                let child_dir = match &path {
                                    Some(path) => base.join(path),
                                    None => dir.join(&name),
                                };
                                self.items(tokens, at, &child_dir, file, true);
                            } else {
                                *at = open;
                                group(tokens, at, '{', '}', file);
                            }
                        }
                        _ => panic!("{}: mod {name} has neither ; nor {{", file.display()),
                    }
                }
                Token::Ident(word)
                    if word == "macro_rules" && tokens.get(*at + 1) == Some(&Token::Punct('!')) =>
                {
                    // A macro's body declares nothing until it is invoked, and
                    // what its invocations declare is not followed.
                    *at += 3;
                    let (open, close) = match tokens.get(*at) {
                        Some(Token::Punct('(')) => ('(', ')'),
                        Some(Token::Punct('[')) => ('[', ']'),
                        _ => ('{', '}'),
                    };
                    group(tokens, at, open, close, file);
                    pending.clear();
                }
                Token::Ident(word) if word == "fn" => {
                    let test = pending
                        .iter()
                        .any(|attribute| matches!(attribute.as_slice(), [Token::Ident(word)] if word == "test"));
                    let ignored = pending
                        .iter()
                        .any(|attribute| ident(attribute.first()) == Some("ignore"));
                    if test
                        && !ignored
                        && pending
                            .iter()
                            .all(|attribute| holds_on_macos(attribute, file, features))
                    {
                        self.tests += 1;
                    }
                    pending.clear();
                    *at += 1;
                }
                Token::Punct('{') => {
                    depth += 1;
                    pending.clear();
                    *at += 1;
                }
                Token::Punct('}') => {
                    pending.clear();
                    *at += 1;
                    if depth == 0 {
                        assert!(inline, "{}: unbalanced }}", file.display());
                        return;
                    }
                    depth -= 1;
                }
                Token::Punct(';') => {
                    pending.clear();
                    *at += 1;
                }
                _ => *at += 1,
            }
        }
        assert!(!inline, "{}: unclosed inline module", file.display());
        assert_eq!(depth, 0, "{}: unclosed {{", file.display());
    }
}

/// What the source says about one package on macOS.
#[derive(Debug, PartialEq)]
struct Verdict {
    /// Some test target compiles a `#[test]` that runs on aarch64-darwin.
    macos_tests: bool,
    /// Some test target's root carries exactly `#![cfg(target_os = "linux")]`.
    linux_gated: bool,
}

/// Each workspace package in `cargo metadata` output, with its verdict. Panics
/// on malformed metadata or on a source path outside `root`.
fn verdicts(metadata: &serde_json::Value, root: &Path) -> Vec<(String, Verdict)> {
    let root = root
        .canonicalize()
        .unwrap_or_else(|error| panic!("{}: {error}", root.display()));
    let gate = lex(r#"cfg(target_os = "linux")"#, Path::new("gate"));
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("package name").to_owned();
            let features = Features::of(package);
            let mut walk = Walk {
                root: &root,
                features: &features,
                tests: 0,
            };
            let mut linux_gated = false;
            for target in package["targets"].as_array().expect("package targets") {
                let kinds: Vec<&str> = target["kind"]
                    .as_array()
                    .expect("target kind")
                    .iter()
                    .map(|kind| kind.as_str().expect("target kind"))
                    .collect();
                let path = Path::new(target["src_path"].as_str().expect("target src_path"));
                // nextest collects tests from exactly the targets with test set, and Cargo
                // skips a target whose required features the default build lacks.
                if !target["test"].as_bool().expect("target test")
                    || kinds == ["custom-build"]
                    || !features.enabled_by_default(&target["required-features"])
                {
                    continue;
                }
                let dir = path.parent().expect("target src_path has a directory");
                linux_gated |= walk.file(path, dir).contains(&gate);
            }
            (
                name,
                Verdict {
                    macos_tests: walk.tests > 0,
                    linux_gated,
                },
            )
        })
        .collect()
}

/// Why `listed` does not equal the gated packages without macOS tests.
fn problems(verdicts: &[(String, Verdict)], listed: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    for name in listed {
        if !verdicts.iter().any(|(package, _)| package == name) {
            problems.push(format!(
                "{name} is in LINUX_ONLY_TEST_PACKAGES but not in the workspace; remove it"
            ));
        }
    }
    for (name, verdict) in verdicts {
        let on_list = listed.contains(&name.as_str());
        if on_list && verdict.macos_tests {
            problems.push(format!(
                "{name} has macOS tests; remove it from LINUX_ONLY_TEST_PACKAGES"
            ));
        } else if on_list && !verdict.linux_gated {
            problems.push(format!(
                "{name} is in LINUX_ONLY_TEST_PACKAGES, but none of its test targets is gated \
                 with #![cfg(target_os = \"linux\")]"
            ));
        } else if !on_list && !verdict.macos_tests {
            problems.push(format!(
                "{name} would run no tests on macOS: give it a macOS test, or gate it with \
                 #![cfg(target_os = \"linux\")] and add it to mac-proof's \
                 LINUX_ONLY_TEST_PACKAGES, which removes it from the macOS proof"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::LINUX_ONLY_TEST_PACKAGES;
    use serde_json::json;
    use std::{os::unix::fs::symlink, process::Command};

    #[test]
    fn linux_only_list_matches_this_workspace() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        let output = Command::new(env!("CARGO"))
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
                "--locked",
                "--manifest-path",
            ])
            .arg(root.join("Cargo.toml"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let verdicts = verdicts(&metadata, &root);
        assert!(verdicts.len() > 10, "{verdicts:?}");
        assert_eq!(
            problems(&verdicts, LINUX_ONLY_TEST_PACKAGES),
            Vec::<String>::new()
        );
    }

    /// A fixture repository: files by relative path, and packages as
    /// (name, [(kinds, root, test)]).
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "mac-proof-macos-tests-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            for (path, text) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            Self { root }
        }

        fn metadata(&self, targets: &[(&[&str], &str, bool)]) -> serde_json::Value {
            json!({"packages": [{"name": "p", "targets": targets.iter().map(|(kind, path, test)| {
                json!({"kind": kind, "src_path": self.root.join(path), "test": test})
            }).collect::<Vec<_>>()}]})
        }

        fn verdict(&self, targets: &[(&[&str], &str, bool)]) -> Verdict {
            let mut verdicts = verdicts(&self.metadata(targets), &self.root);
            verdicts.pop().unwrap().1
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const GATE: &str = "#![cfg(target_os = \"linux\")]\n";
    const TEST: &str = "#[test]\nfn t() {}\n";
    const STUB: &str = "fn main() {}\n";

    fn verdict(macos_tests: bool, linux_gated: bool) -> Verdict {
        Verdict {
            macos_tests,
            linux_gated,
        }
    }

    /// A package of a gated or ungated lib, one integration test and a stub
    /// bin, with `lib` as the library's text.
    fn lib_package(name: &str, lib: &str) -> Verdict {
        let test = format!("{GATE}{TEST}");
        let fixture = Fixture::new(
            name,
            &[
                ("src/lib.rs", lib),
                ("tests/t.rs", &test),
                ("src/main.rs", STUB),
            ],
        );
        fixture.verdict(&[
            (&["lib"], "src/lib.rs", true),
            (&["test"], "tests/t.rs", true),
            (&["bin"], "src/main.rs", true),
        ])
    }

    #[test]
    fn crate_level_gates_are_read_exactly() {
        // Today's rota-cli and agent-jail shape: docs and other inner
        // attributes before the gate, every root gated, a stub bin.
        assert_eq!(
            lib_package(
                "shape",
                &format!(
                    "//! Docs.\n//!\n#![forbid(unsafe_code)]\n#![doc = \"a ] b\"]\n{GATE}{TEST}"
                )
            ),
            verdict(false, true)
        );
        for (index, lib) in [
            format!("// {GATE}{TEST}"),
            format!("//! Gate with {GATE}{TEST}"),
            format!("/* {GATE} */\n{TEST}"),
            format!("#![doc = \"\n{}\"]\n{TEST}", GATE.replace('"', "\\\"")),
            format!("#![doc = \"x]\n{}\"]\n{TEST}", GATE.replace('"', "\\\"")),
            format!("#![doc = \"x\"] /* ]\n{GATE} */\n{TEST}"),
            format!("use std::fs;\n{GATE}{TEST}"),
        ]
        .iter()
        .enumerate()
        {
            // A lone library: its test is visible and nothing is gated.
            let fixture = Fixture::new(&format!("ungated-{index}"), &[("src/lib.rs", lib)]);
            assert_eq!(
                fixture.verdict(&[(&["lib"], "src/lib.rs", true)]),
                verdict(true, false),
                "{lib}"
            );
        }
    }

    #[test]
    fn crate_level_cfgs_are_evaluated_for_macos() {
        for (index, (cfg, visible)) in [
            ("any(target_os = \"linux\")", false),
            ("not(target_os = \"macos\")", false),
            ("all(unix, not(target_os = \"macos\"))", false),
            ("target_os = \"macos\"", true),
            ("not(target_os = \"linux\")", true),
            ("any(target_os = \"linux\", target_os = \"macos\")", true),
            ("all(test, unix, target_arch = \"aarch64\")", true),
        ]
        .into_iter()
        .enumerate()
        {
            let root = format!("#![cfg({cfg})]\n{TEST}");
            let fixture = Fixture::new(&format!("cfg-{index}"), &[("tests/t.rs", &root)]);
            assert_eq!(
                fixture.verdict(&[(&["test"], "tests/t.rs", true)]),
                verdict(visible, false),
                "{cfg}"
            );
        }
    }

    /// Fixture files, as (relative path, text).
    type Files<'a> = &'a [(&'a str, &'a str)];

    #[test]
    fn modules_follow_rustc_resolution_and_cfg() {
        let cases: &[(&str, Files, bool)] = &[
            (
                "file",
                &[
                    ("src/main.rs", "mod a;\nfn main() {}\n"),
                    ("src/a.rs", TEST),
                ],
                true,
            ),
            (
                "nested",
                &[
                    ("src/main.rs", "pub(crate) mod a;\n"),
                    ("src/a.rs", "mod b;\n"),
                    ("src/a/b/mod.rs", TEST),
                ],
                true,
            ),
            (
                "path",
                &[
                    ("src/main.rs", "#[path = \"other/x.rs\"]\nmod a;\n"),
                    ("src/other/x.rs", "mod y;\n"),
                    ("src/other/y.rs", TEST),
                ],
                true,
            ),
            (
                "inline",
                &[
                    ("src/main.rs", "mod inner {\n    mod sub;\n}\n"),
                    ("src/inner/sub.rs", TEST),
                ],
                true,
            ),
            (
                "inline-in-file",
                &[
                    ("src/main.rs", "mod a;\n"),
                    ("src/a.rs", "#[cfg(test)]\nmod tests {\n    mod more;\n}\n"),
                    ("src/a/tests/more.rs", TEST),
                ],
                true,
            ),
            (
                "gated-file-module",
                &[
                    ("src/main.rs", "#[cfg(target_os = \"linux\")]\nmod a;\n"),
                    ("src/a.rs", TEST),
                ],
                false,
            ),
            (
                "gated-multiline-with-doc",
                &[
                    (
                        "src/main.rs",
                        "#[cfg(\n    target_os = \"linux\",\n)]\n/// Docs.\nmod a;\n",
                    ),
                    ("src/a.rs", TEST),
                ],
                false,
            ),
            (
                "not-macos-module",
                &[
                    (
                        "src/main.rs",
                        "#[cfg(not(target_os = \"macos\"))]\nmod a;\n",
                    ),
                    ("src/a.rs", TEST),
                ],
                false,
            ),
            (
                "not-linux-module",
                &[
                    (
                        "src/main.rs",
                        "#[cfg(not(target_os = \"linux\"))]\nmod a;\n",
                    ),
                    ("src/a.rs", TEST),
                ],
                true,
            ),
            (
                "gated-inline-tests",
                &[(
                    "src/main.rs",
                    "#[cfg(all(test, target_os = \"linux\"))]\nmod tests {\n    #[test]\n    fn t() {}\n}\n",
                )],
                false,
            ),
            (
                "inline-tests",
                &[(
                    "src/main.rs",
                    "#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t() {}\n}\n",
                )],
                true,
            ),
            (
                "inner-gated-inline",
                &[(
                    "src/main.rs",
                    "mod tests {\n    #![cfg(target_os = \"linux\")]\n    #[test]\n    fn t() {}\n}\n",
                )],
                false,
            ),
            (
                "gated-file",
                &[
                    ("src/main.rs", "mod a;\n"),
                    ("src/a.rs", &*format!("{GATE}{TEST}")),
                ],
                false,
            ),
            (
                "gated-function",
                &[(
                    "src/main.rs",
                    "#[test]\n#[cfg(target_os = \"linux\")]\nfn t() {}\n",
                )],
                false,
            ),
            (
                "ignored",
                &[("src/main.rs", "#[test]\n#[ignore = \"slow\"]\nfn t() {}\n")],
                false,
            ),
            (
                "async-and-attributes",
                &[(
                    "src/main.rs",
                    "#[test]\n#[should_panic]\npub(crate) fn t() {}\n",
                )],
                true,
            ),
            (
                "raw-identifiers",
                &[(
                    "src/main.rs",
                    "fn f() { let r#mod = 1; let r#fn = 2; let r#macro_rules = 3; }\nstruct S { r#mod: u8 }\n",
                )],
                false,
            ),
            (
                "raw-module-name",
                &[("src/main.rs", "mod r#type;\n"), ("src/type.rs", TEST)],
                true,
            ),
            (
                "path-on-inline-in-file",
                &[
                    ("src/main.rs", "mod a;\n"),
                    (
                        "src/a.rs",
                        "#[path = \"custom\"]\nmod inner {\n    mod sub;\n}\n",
                    ),
                    ("src/custom/sub.rs", TEST),
                ],
                true,
            ),
            (
                "cfg-attr-ignore-on-linux",
                &[(
                    "src/main.rs",
                    "#[test]\n#[cfg_attr(target_os = \"linux\", ignore)]\nfn t() {}\n",
                )],
                true,
            ),
            (
                "cfg-attr-ignore-on-macos",
                &[(
                    "src/main.rs",
                    "#[test]\n#[cfg_attr(target_os = \"macos\", ignore)]\nfn t() {}\n",
                )],
                false,
            ),
            (
                "cfg-attr-cfg-on-module",
                &[
                    (
                        "src/main.rs",
                        "#[cfg_attr(unix, cfg(target_os = \"linux\"))]\nmod a;\n",
                    ),
                    ("src/a.rs", TEST),
                ],
                false,
            ),
            (
                "cfg-attr-path-not-on-macos",
                &[
                    (
                        "src/main.rs",
                        "#[cfg_attr(not(target_os = \"macos\"), path = \"linux.rs\")]\nmod a;\n",
                    ),
                    ("src/a.rs", TEST),
                ],
                true,
            ),
            (
                "cfg-attr-inner",
                &[
                    ("src/main.rs", "mod a;\n"),
                    (
                        "src/a.rs",
                        &*format!("#![cfg_attr(test, cfg(target_os = \"linux\"))]\n{TEST}"),
                    ),
                ],
                false,
            ),
            (
                "literals-and-comments",
                &[(
                    "src/main.rs",
                    "const A: &str = r#\"\nmod missing;\n#[test]\nfn t() {}\n\"#;\n/* mod gone;\n#[test]\nfn u() {} /* nested */ */\nconst B: char = '{';\nconst C: &str = \"}\";\nfn f<'a>(x: &'a str) -> &'a str { x }\n",
                )],
                false,
            ),
        ];
        for (name, files, visible) in cases {
            let fixture = Fixture::new(name, files);
            assert_eq!(
                fixture
                    .verdict(&[(&["bin"], "src/main.rs", true)])
                    .macos_tests,
                *visible,
                "{name}"
            );
        }
    }

    #[test]
    fn every_collected_target_counts() {
        let gated = format!("{GATE}{TEST}");
        let fixture = Fixture::new(
            "targets",
            &[
                ("src/lib.rs", &gated),
                ("tests/t.rs", &gated),
                ("tests/common.rs", "pub fn helper() {}\n"),
                ("src/main.rs", STUB),
                ("src/bin/tool.rs", &format!("mod tests;\n{STUB}")),
                ("src/bin/tests.rs", TEST),
                ("src/plain.rs", TEST),
                ("benches/b.rs", TEST),
            ],
        );
        let gated_package: &[(&[&str], &str, bool)] = &[
            (&["lib"], "src/lib.rs", true),
            (&["test"], "tests/t.rs", true),
            (&["bin"], "src/main.rs", true),
        ];
        assert_eq!(fixture.verdict(gated_package), verdict(false, true));
        // An ungated helper test file without tests changes nothing.
        let mut with_helper = gated_package.to_vec();
        with_helper.push((&["test"], "tests/common.rs", true));
        assert_eq!(fixture.verdict(&with_helper), verdict(false, true));
        // A lib-less package: its bin holds tests beside gated integration tests.
        assert_eq!(
            fixture.verdict(&[
                (&["bin"], "src/bin/tool.rs", true),
                (&["test"], "tests/t.rs", true)
            ]),
            verdict(true, true)
        );
        // Any library kind, a second bin, and a bench that nextest collects.
        for extra in [
            (&["cdylib", "rlib"][..], "src/plain.rs", true),
            (&["proc-macro"][..], "src/plain.rs", true),
            (&["bin"][..], "src/bin/tool.rs", true),
            (&["bench"][..], "benches/b.rs", true),
        ] {
            let mut targets = gated_package.to_vec();
            targets.push(extra);
            assert_eq!(fixture.verdict(&targets), verdict(true, true), "{extra:?}");
        }
        // A bench nextest does not collect.
        let mut targets = gated_package.to_vec();
        targets.push((&["bench"], "benches/b.rs", false));
        assert_eq!(fixture.verdict(&targets), verdict(false, true));
        // A build script never counts.
        let mut targets = gated_package.to_vec();
        targets.push((&["custom-build"], "src/plain.rs", true));
        assert_eq!(fixture.verdict(&targets), verdict(false, true));
    }

    /// The panic message of `verdicts`, which must panic.
    fn panic_message(metadata: &serde_json::Value, root: &Path) -> String {
        let payload = std::panic::catch_unwind(|| verdicts(metadata, root))
            .expect_err("verdicts should panic");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default()
    }

    fn bin_target(path: PathBuf) -> serde_json::Value {
        json!({"packages": [{"name": "p", "targets": [
            {"kind": ["bin"], "src_path": path, "test": true}
        ]}]})
    }

    #[test]
    fn features_hold_as_the_default_features_enable_them() {
        let features = json!({
            "default": ["base"],
            "base": ["extra", "dep:hidden", "maybe?/f", "optional/f"],
            "extra": [],
            "candidate": ["dep:rune"],
            "maybe": ["dep:maybe"],
            "optional": ["dep:optional"],
        });
        for (cfg, macos_tests) in [
            ("feature = \"base\"", true),
            ("feature = \"extra\"", true),
            ("feature = \"optional\"", true),
            ("feature = \"candidate\"", false),
            ("feature = \"maybe\"", false),
            ("not(feature = \"candidate\")", true),
            ("all(test, feature = \"candidate\")", false),
        ] {
            let fixture = Fixture::new(
                "features",
                &[
                    ("src/lib.rs", &format!("#[cfg({cfg})]\nmod a;\n")),
                    ("src/a.rs", TEST),
                ],
            );
            let metadata = json!({"packages": [{"name": "p", "features": features, "targets": [
                {"kind": ["lib"], "src_path": fixture.root.join("src/lib.rs"), "test": true}
            ]}]});
            let verdict = verdicts(&metadata, &fixture.root).pop().unwrap().1;
            assert_eq!(verdict.macos_tests, macos_tests, "{cfg}");
        }
        // A target the default build skips for its required features compiles no test.
        let fixture = Fixture::new(
            "required-features",
            &[("src/lib.rs", ""), ("tests/needs.rs", TEST)],
        );
        for (required, macos_tests) in [(json!(["candidate"]), false), (json!(["extra"]), true)] {
            let metadata = json!({"packages": [{"name": "p", "features": features, "targets": [
                {"kind": ["lib"], "src_path": fixture.root.join("src/lib.rs"), "test": true},
                {"kind": ["test"], "src_path": fixture.root.join("tests/needs.rs"), "test": true,
                 "required-features": required}
            ]}]});
            let verdict = verdicts(&metadata, &fixture.root).pop().unwrap().1;
            assert_eq!(verdict.macos_tests, macos_tests, "{required}");
        }

        // A feature the package does not declare is not taken as off.
        let fixture = Fixture::new(
            "undeclared-feature",
            &[("src/lib.rs", "#[cfg(feature = \"nope\")]\nmod a;\n")],
        );
        let metadata = json!({"packages": [{"name": "p", "features": features, "targets": [
            {"kind": ["lib"], "src_path": fixture.root.join("src/lib.rs"), "test": true}
        ]}]});
        assert!(panic_message(&metadata, &fixture.root).contains("cannot evaluate cfg"));
    }

    #[test]
    fn unreadable_malformed_or_outside_sources_panic() {
        let outside = Fixture::new("outside-target", &[("x.rs", TEST)]);
        let fixture = Fixture::new(
            "outside",
            &[
                ("src/missing.rs", "mod missing;\n"),
                ("src/unknown.rs", "#[cfg(feature = \"x\")]\nmod a;\n"),
                (
                    "src/unknown_attr.rs",
                    "#[cfg_attr(feature = \"x\", ignore)]\n#[test]\nfn t() {}\n",
                ),
                ("src/open_brace.rs", "fn f() {\n"),
                ("src/close_brace.rs", "}\n"),
                ("src/open_attribute.rs", "#[test\nfn t() {}\n"),
                ("src/open_string.rs", "const S: &str = \"x;\n"),
                ("src/open_comment.rs", "/* unclosed /* nested */\n"),
                ("src/open_char.rs", "const C: char = '\\n"),
            ],
        );
        symlink(outside.root.join("x.rs"), fixture.root.join("src/link.rs")).unwrap();
        for (file, message) in [
            ("missing.rs", "No such file"),
            ("unknown.rs", "cannot evaluate cfg"),
            ("unknown_attr.rs", "cannot evaluate cfg"),
            ("open_brace.rs", "unclosed {"),
            ("close_brace.rs", "unbalanced }"),
            ("open_attribute.rs", "unclosed ["),
            ("open_string.rs", "unterminated string"),
            ("open_comment.rs", "unterminated block comment"),
            ("open_char.rs", "unterminated character literal"),
            ("link.rs", "resolves outside"),
        ] {
            let target = bin_target(fixture.root.join("src").join(file));
            let panic = panic_message(&target, &fixture.root);
            assert!(panic.contains(message), "{file}: {panic}");
        }
        let panic = panic_message(&bin_target(outside.root.join("x.rs")), &fixture.root);
        assert!(panic.contains("resolves outside"), "{panic}");
        let main = fixture.root.join("src/missing.rs");
        for (metadata, message) in [
            (
                json!({"packages": [{"name": "p", "targets": [
                    {"kind": ["bin"], "src_path": main}
                ]}]}),
                "target test",
            ),
            (json!({"packages": [{"targets": []}]}), "package name"),
            (json!({"packages": [{"name": "p"}]}), "package targets"),
            (json!({}), "metadata packages"),
        ] {
            let panic = panic_message(&metadata, &fixture.root);
            assert!(panic.contains(message), "{message}: {panic}");
        }
    }

    /// macOS's /tmp and /var are symlinks into /private, so the fixture roots
    /// there are aliases of their canonical paths.
    #[test]
    fn a_symlinked_root_is_read_through_its_canonical_path() {
        let fixture = Fixture::new(
            "alias",
            &[
                ("src/main.rs", &format!("mod a;\n{STUB}")),
                ("src/a.rs", TEST),
            ],
        );
        let outside = Fixture::new("alias-outside", &[("x.rs", TEST)]);
        let alias = std::env::temp_dir().join(format!(
            "mac-proof-macos-tests-alias-link-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&alias);
        symlink(&fixture.root, &alias).unwrap();
        symlink(outside.root.join("x.rs"), fixture.root.join("src/link.rs")).unwrap();
        let verdicts = verdicts(&bin_target(alias.join("src/main.rs")), &alias);
        assert_eq!(verdicts, [("p".to_owned(), verdict(true, false))]);
        let panic = panic_message(&bin_target(alias.join("src/link.rs")), &alias);
        assert!(panic.contains("resolves outside"), "{panic}");
        fs::remove_file(alias).unwrap();
    }

    #[test]
    fn problems_name_each_mismatch() {
        let verdicts = vec![
            ("listed-ok".to_owned(), verdict(false, true)),
            ("listed-with-tests".to_owned(), verdict(true, true)),
            ("listed-ungated".to_owned(), verdict(false, false)),
            ("unlisted-ok".to_owned(), verdict(true, false)),
            ("unlisted-gated-ok".to_owned(), verdict(true, true)),
            ("new-linux-tool".to_owned(), verdict(false, true)),
            ("no-tests".to_owned(), verdict(false, false)),
        ];
        let problems = problems(
            &verdicts,
            &["listed-ok", "listed-with-tests", "listed-ungated", "gone"],
        );
        let named = |name: &str| {
            problems
                .iter()
                .filter(|problem| problem.starts_with(name))
                .count()
        };
        assert_eq!(problems.len(), 5, "{problems:?}");
        for name in [
            "gone",
            "listed-with-tests",
            "listed-ungated",
            "new-linux-tool",
            "no-tests",
        ] {
            assert_eq!(named(name), 1, "{name}: {problems:?}");
        }
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("add it to mac-proof's"))
        );
    }
}
