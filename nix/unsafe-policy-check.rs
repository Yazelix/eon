#![forbid(unsafe_code)]
//! The repository's unsafe-code policy (docs/UNSAFE.md), checked from source text so that code
//! for every platform and test configuration counts, whichever host runs the check.
//!
//! `unsafe-policy-check <root>` prints each violation and exits 1; `--report <root>` prints the
//! registry cells each file needs and the per-package totals.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

const REGISTRY: &str = "docs/UNSAFE.md";
/// Lints whose level only manifests set; any other mention in build configuration is refused.
const POLICY_LINTS: [&str; 9] = [
    "unsafe_code",
    "undocumented_unsafe_blocks",
    "unsafe_op_in_unsafe_fn",
    "clippy::restriction",
    "unsafe-code",
    "undocumented-unsafe-blocks",
    "unsafe-op-in-unsafe-fn",
    "rust_2024_compatibility",
    "--cap-lints",
];
/// `rustc` sources that name no file directly: (build file, argument, root it compiles).
const INDIRECT_RUSTC_SOURCES: &[(&str, &str, &str)] = &[
    (
        "nix/update-launch-environment-check.nix",
        "${source}",
        "nix/update-launch-environment-check.rs",
    ),
    (
        "nix/orbit-checks.nix",
        "${boundarySource}/tools/boundary.rs",
        "orbit/tools/boundary.rs",
    ),
];
/// Forms that a statement-level allowance's target may not contain anywhere.
const COMPOUND_KEYWORDS: [&str; 13] = [
    "if", "match", "loop", "while", "for", "fn", "impl", "mod", "struct", "enum", "trait", "async",
    "move",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (report, root) = match args.as_slice() {
        [root] => (false, root),
        [flag, root] if flag == "--report" => (true, root),
        _ => {
            eprintln!("usage: unsafe-policy-check [--report] <repository root>");
            return ExitCode::from(2);
        }
    };
    let repo = match Repo::load(Path::new(root)) {
        Ok(repo) => repo,
        Err(error) => {
            eprintln!("unsafe-policy-check: {error}");
            return ExitCode::from(2);
        }
    };
    if report {
        print!("{}", repo.report());
        return ExitCode::SUCCESS;
    }
    let violations = repo.check();
    for violation in &violations {
        println!("{violation}");
    }
    if violations.is_empty() {
        ExitCode::SUCCESS
    } else {
        println!(
            "{} unsafe-policy violation(s); see {REGISTRY}",
            violations.len()
        );
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------------------------
// Lexing: comments and literals become spaces in `code`; comment text is kept in `comments`.

struct Source {
    code: Vec<u8>,
    comments: Vec<u8>,
    tokens: Vec<Token>,
    line_starts: Vec<usize>,
}

#[derive(Clone, Debug)]
struct Token {
    start: usize,
    text: String,
}

impl Token {
    fn is(&self, text: &str) -> bool {
        self.text == text
    }
}

fn ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

impl Source {
    fn new(text: String) -> Self {
        let bytes = text.as_bytes();
        let mut code = bytes.to_vec();
        let mut comments = vec![b' '; bytes.len()];
        let blank = |code: &mut Vec<u8>, from: usize, to: usize| {
            for byte in &mut code[from..to] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        };
        let mut i = 0;
        while i < bytes.len() {
            let prev_ident = i > 0 && ident_byte(bytes[i - 1]);
            let end = if bytes[i..].starts_with(b"//") {
                let end = bytes[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(bytes.len(), |n| i + n);
                comments[i..end].copy_from_slice(&bytes[i..end]);
                end
            } else if bytes[i..].starts_with(b"/*") {
                let (mut depth, mut j) = (1, i + 2);
                while j < bytes.len() && depth > 0 {
                    if bytes[j..].starts_with(b"/*") {
                        depth += 1;
                        j += 2;
                    } else if bytes[j..].starts_with(b"*/") {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                comments[i..j].copy_from_slice(&bytes[i..j]);
                j
            } else if let Some(end) = raw_string_end(bytes, i, prev_ident) {
                end
            } else if bytes[i] == b'"' {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                (j + 1).min(bytes.len())
            } else if bytes[i] == b'\''
                && (!prev_ident || (bytes[i - 1] == b'b' && (i < 2 || !ident_byte(bytes[i - 2]))))
            {
                char_literal_end(&text, i).unwrap_or(i + 1)
            } else {
                i + 1
            };
            if end > i + 1 || bytes[i] == b'"' {
                blank(&mut code, i, end);
            }
            i = end;
        }
        let tokens = tokenize(&code);
        let line_starts = std::iter::once(0)
            .chain(
                bytes
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| **b == b'\n')
                    .map(|(n, _)| n + 1),
            )
            .collect();
        Source {
            code,
            comments,
            tokens,
            line_starts,
        }
    }

    fn line(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= offset)
    }

    fn comment_between(&self, from: usize, to: usize, needles: &[&str]) -> bool {
        let text = String::from_utf8_lossy(&self.comments[from..to]);
        needles.iter().any(|needle| text.contains(needle))
    }

    /// The token index of the bracket that closes the one at `open`.
    fn matching(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (n, token) in self.tokens.iter().enumerate().skip(open) {
            match token.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(n);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

fn raw_string_end(bytes: &[u8], i: usize, prev_ident: bool) -> Option<usize> {
    let mut j = i;
    if matches!(bytes.get(j), Some(b'b' | b'c')) && bytes.get(j + 1) == Some(&b'r') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'r') || prev_ident {
        return None;
    }
    j += 1;
    let hashes = bytes[j..].iter().take_while(|&&b| b == b'#').count();
    j += hashes;
    if bytes.get(j) != Some(&b'"') {
        return None;
    }
    let mut close = vec![b'"'];
    close.extend(std::iter::repeat_n(b'#', hashes));
    let body = j + 1;
    Some(
        bytes[body..]
            .windows(close.len())
            .position(|window| window == close.as_slice())
            .map_or(bytes.len(), |n| body + n + close.len()),
    )
}

/// The end of a char literal at `i`, or None for a lifetime or label.
fn char_literal_end(text: &str, i: usize) -> Option<usize> {
    let rest = &text[i + 1..];
    if let Some(escaped) = rest.strip_prefix('\\') {
        let first = escaped.chars().next()?;
        let after = &escaped[first.len_utf8()..];
        return after.find('\'').map(|n| i + 2 + first.len_utf8() + n + 1);
    }
    let ch = rest.chars().next()?;
    (rest[ch.len_utf8()..].starts_with('\'')).then(|| i + 1 + ch.len_utf8() + 1)
}

fn tokenize(code: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if code[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let end = if ident_byte(code[i]) {
            i + code[i..].iter().take_while(|&&b| ident_byte(b)).count()
        } else {
            i + 1
        };
        tokens.push(Token {
            start: i,
            text: String::from_utf8_lossy(&code[i..end]).into_owned(),
        });
        i = end;
    }
    tokens
}

// ---------------------------------------------------------------------------------------------
// What a Rust file holds: unsafe sites, allowances and policy lint lowerings.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Block,
    Fn,
    ExternBlock,
    Impl,
    Trait,
    Attribute,
}

impl Kind {
    fn name(self, plural: bool) -> &'static str {
        match (self, plural) {
            (Kind::Block, false) => "block",
            (Kind::Block, true) => "blocks",
            (Kind::Fn, false) => "unsafe fn",
            (Kind::Fn, true) => "unsafe fns",
            (Kind::ExternBlock, false) => "extern block",
            (Kind::ExternBlock, true) => "extern blocks",
            (Kind::Impl, false) => "unsafe impl",
            (Kind::Impl, true) => "unsafe impls",
            (Kind::Trait, false) => "unsafe trait",
            (Kind::Trait, true) => "unsafe traits",
            (Kind::Attribute, false) => "unsafe attribute",
            (Kind::Attribute, true) => "unsafe attributes",
        }
    }
}

struct Site {
    kind: Kind,
    test: bool,
    line: usize,
    documented: bool,
}

struct Attribute {
    /// Token indexes of `#` and the closing `]`.
    hash: usize,
    close: usize,
    inner: bool,
    idents: Vec<String>,
}

struct Scanned {
    sites: Vec<Site>,
    /// Allowance targets, one entry per attribute that lowers `unsafe_code`.
    allowances: Vec<String>,
    violations: Vec<String>,
    forbids_at_root: bool,
}

fn scan(path: &str, source: &Source, crate_root: bool) -> Scanned {
    let tokens = &source.tokens;
    let test_path = path
        .split('/')
        .any(|part| part == "tests" || part == "benches");
    let attributes = attributes(source);
    let test_spans = test_spans(source, &attributes);
    let fn_bodies = fn_bodies(source);
    let mut scanned = Scanned {
        sites: Vec::new(),
        allowances: Vec::new(),
        violations: Vec::new(),
        forbids_at_root: false,
    };
    for (n, token) in tokens.iter().enumerate() {
        if !token.is("unsafe") {
            continue;
        }
        let next = |k: usize| tokens.get(n + k).map_or("", |t| t.text.as_str());
        let kind = match next(1) {
            "{" => Kind::Block,
            "fn" => Kind::Fn,
            "impl" => Kind::Impl,
            "trait" => Kind::Trait,
            "(" if n > 0 && tokens[n - 1].is("[") => Kind::Attribute,
            "extern" if next(2) == "{" => Kind::ExternBlock,
            "extern" if next(2) == "fn" => Kind::Fn,
            _ => continue,
        };
        let boundary = source.code[..token.start]
            .iter()
            .rposition(|b| matches!(b, b';' | b'{' | b'}'))
            .map_or(0, |n| n + 1);
        let needles: &[&str] = if kind == Kind::Block {
            &["SAFETY:"]
        } else {
            &["SAFETY:", "# Safety"]
        };
        scanned.sites.push(Site {
            kind,
            test: test_path || test_spans.iter().any(|&(a, b)| a <= n && n <= b),
            line: source.line(token.start),
            documented: source.comment_between(boundary, token.start, needles),
        });
    }
    for attribute in &attributes {
        let has = |name: &str| attribute.idents.iter().any(|ident| ident == name);
        let lowers = ["allow", "warn", "expect"].iter().any(|level| has(level));
        let line = source.line(tokens[attribute.hash].start);
        if attribute.inner
            && has("forbid")
            && has("unsafe_code")
            && !has("cfg_attr")
            && !lowers
            && depth_at(source, attribute.hash) == 0
        {
            scanned.forbids_at_root = true;
        }
        if !lowers {
            continue;
        }
        for lint in [
            "undocumented_unsafe_blocks",
            "unsafe_op_in_unsafe_fn",
            "restriction",
            "rust_2024_compatibility",
        ] {
            if has(lint) {
                scanned.violations.push(format!(
                    "{path}:{line}: an attribute lowers `{lint}`; the policy allows no exception"
                ));
            }
        }
        if !has("unsafe_code") {
            continue;
        }
        let target = if attribute.inner {
            match depth_at(source, attribute.hash) {
                0 if crate_root => "crate".to_owned(),
                0 => "module".to_owned(),
                // An inner attribute in a function body allows the whole function.
                _ => match enclosing(&fn_bodies, attribute.hash) {
                    scope if scope.starts_with("fn ") => scope,
                    _ => "inner block".to_owned(),
                },
            }
        } else {
            match outer_target(source, &attributes, attribute) {
                Target::Item(item) => item,
                Target::Statement(at) => {
                    if let Err(reason) = statement_target_ok(source, at) {
                        scanned.violations.push(format!(
                            "{path}:{line}: a statement allowance must sit on a `let` or expression \
                             statement with one unsafe block and no compound form ({reason})"
                        ));
                    }
                    format!("in {}: statement", enclosing(&fn_bodies, at))
                }
            }
        };
        scanned.allowances.push(target);
    }
    scanned
}

fn attributes(source: &Source) -> Vec<Attribute> {
    let tokens = &source.tokens;
    let mut found = Vec::new();
    for (n, token) in tokens.iter().enumerate() {
        if !token.is("#") {
            continue;
        }
        let inner = tokens.get(n + 1).is_some_and(|t| t.is("!"));
        let open = n + 1 + usize::from(inner);
        if !tokens.get(open).is_some_and(|t| t.is("[")) {
            continue;
        }
        let Some(close) = source.matching(open) else {
            continue;
        };
        let idents = tokens[open + 1..close]
            .iter()
            .filter(|t| t.text.as_bytes()[0].is_ascii_alphabetic() || t.text.starts_with('_'))
            .map(|t| t.text.clone())
            .collect();
        found.push(Attribute {
            hash: n,
            close,
            inner,
            idents,
        });
    }
    found
}

/// Brace depth at a token, counting only `{` and `}`.
fn depth_at(source: &Source, index: usize) -> i32 {
    source.tokens[..index]
        .iter()
        .fold(0, |depth, t| match t.text.as_str() {
            "{" => depth + 1,
            "}" => depth - 1,
            _ => depth,
        })
}

/// The first token after an attribute and any attributes that directly follow it.
fn after_attributes(attributes: &[Attribute], mut at: usize) -> usize {
    while let Some(next) = attributes.iter().find(|a| a.hash == at && !a.inner) {
        at = next.close + 1;
    }
    at
}

enum Target {
    Item(String),
    Statement(usize),
}

fn outer_target(source: &Source, attributes: &[Attribute], attribute: &Attribute) -> Target {
    let tokens = &source.tokens;
    let start = after_attributes(attributes, attribute.close + 1);
    let text = |k: usize| tokens.get(k).map_or("", |t| t.text.as_str());
    let mut at = start;
    if text(at) == "pub" {
        at += 1;
        if text(at) == "(" {
            at = source.matching(at).map_or(at, |close| close + 1);
        }
    }
    let item_start = at;
    while matches!(
        text(at),
        "const" | "async" | "unsafe" | "extern" | "default"
    ) && matches!(
        text(at + 1),
        "fn" | "const" | "async" | "unsafe" | "extern" | "impl" | "trait" | "{"
    ) && !(text(at) == "unsafe" && text(at + 1) == "{")
    {
        at += 1;
    }
    let name = if text(at) == "macro_rules" {
        text(at + 2)
    } else {
        text(at + 1)
    };
    match text(at) {
        "mod" | "fn" | "trait" | "struct" | "enum" | "union" | "type" | "static"
        | "macro_rules" => Target::Item(format!("{} {name}", text(at))),
        "const" if name != "{" => Target::Item(format!("const {name}")),
        "impl" => Target::Item("impl".to_owned()),
        "use" => Target::Item("use".to_owned()),
        "{" if at > item_start && text(at - 1) == "extern" => {
            Target::Item("extern block".to_owned())
        }
        _ => Target::Statement(start),
    }
}

/// A statement allowance's target: a `let` or expression statement whose whole extent, its
/// unsafe block included, holds exactly one unsafe block and no compound form.
fn statement_target_ok(source: &Source, start: usize) -> Result<(), String> {
    let tokens = &source.tokens;
    let mut depth = 0i32;
    let mut angles = 0i32;
    let mut unsafe_blocks = 0;
    for n in start..tokens.len() {
        let token = &tokens[n];
        let after_path = n >= 2 && tokens[n - 1].is(":") && tokens[n - 2].is(":");
        match token.text.as_str() {
            "<" if after_path || angles > 0 => angles += 1,
            ">" if angles > 0 => angles -= 1,
            "," if angles > 0 => {}
            ";" | "," if depth == 0 => break,
            "(" | "[" => depth += 1,
            ")" | "]" => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            "{" => {
                let previous = n.checked_sub(1).map_or("", |p| tokens[p].text.as_str());
                if previous == "unsafe" {
                    unsafe_blocks += 1;
                } else if previous != "!" {
                    return Err("a block, struct literal or closure body".to_owned());
                }
                depth += 1;
            }
            "}" => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
                // A block-like expression statement (`unsafe { .. }` with no `;`) ends with
                // its block unless an operator, call or `?` continues the expression.
                let next = tokens.get(n + 1).map_or("", |t| t.text.as_str());
                if depth == 0 && next == "else" {
                    return Err("an `else` block".to_owned());
                }
                if depth == 0
                    && next != "as"
                    && (next == "#" || next.bytes().next().is_some_and(ident_byte))
                {
                    break;
                }
            }
            "|" => return Err("a closure or `|` operator".to_owned()),
            word if COMPOUND_KEYWORDS.contains(&word) => return Err(format!("`{word}`")),
            _ => {}
        }
    }
    match unsafe_blocks {
        1 => Ok(()),
        0 => Err("no unsafe block".to_owned()),
        _ => Err("more than one unsafe block".to_owned()),
    }
}

/// Spans (token indexes) of items gated by `cfg(test)` or `cfg(all(.., test, ..))`.
fn test_spans(source: &Source, attributes: &[Attribute]) -> Vec<(usize, usize)> {
    let tokens = &source.tokens;
    let mut spans = Vec::new();
    for attribute in attributes.iter().filter(|a| !a.inner) {
        let body = &tokens[attribute.hash + 2..attribute.close];
        if !body.first().is_some_and(|t| t.is("cfg")) || !requires_test(&body[1..]) {
            continue;
        }
        let start = after_attributes(attributes, attribute.close + 1);
        let mut depth = 0i32;
        let mut end = None;
        for (n, token) in tokens.iter().enumerate().skip(start) {
            match token.text.as_str() {
                "(" | "[" => depth += 1,
                ")" | "]" => depth -= 1,
                ";" if depth == 0 => {
                    end = Some(n);
                    break;
                }
                "{" if depth == 0 => {
                    end = source.matching(n);
                    break;
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            spans.push((start, end));
        }
    }
    spans
}

/// Whether a cfg predicate (its parenthesized tokens) can hold only under `test`.
fn requires_test(predicate: &[Token]) -> bool {
    let inner = match predicate {
        [open, rest @ .., close] if open.is("(") && close.is(")") => rest,
        _ => return false,
    };
    match inner {
        [only] => only.is("test"),
        [all, rest @ ..] if all.is("all") => {
            let mut depth = 0;
            rest.iter().any(|t| {
                match t.text.as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                depth == 1 && t.is("test")
            })
        }
        _ => false,
    }
}

/// Named function bodies as (name, open brace, close brace) token indexes.
fn fn_bodies(source: &Source) -> Vec<(String, usize, usize)> {
    let tokens = &source.tokens;
    let mut bodies = Vec::new();
    for (n, token) in tokens.iter().enumerate() {
        let Some(name) = tokens.get(n + 1) else { break };
        if !token.is("fn") || !ident_byte(name.text.as_bytes()[0]) {
            continue;
        }
        let mut depth = 0i32;
        for (k, t) in tokens.iter().enumerate().skip(n + 2) {
            match t.text.as_str() {
                "(" | "[" | "<" => depth += 1,
                ")" | "]" | ">" => depth -= 1,
                ";" if depth <= 0 => break,
                "{" if depth <= 0 => {
                    if let Some(close) = source.matching(k) {
                        bodies.push((name.text.clone(), k, close));
                    }
                    break;
                }
                _ => {}
            }
        }
    }
    bodies
}

fn enclosing(bodies: &[(String, usize, usize)], at: usize) -> String {
    if let Some((name, _, _)) = bodies
        .iter()
        .filter(|(_, open, close)| *open < at && at < *close)
        .max_by_key(|(_, open, _)| *open)
    {
        return format!("fn {name}");
    }
    "file".to_owned()
}

// ---------------------------------------------------------------------------------------------
// Repository model: packages, Rust files, build files and the registry.

struct Package {
    dir: String,
    level: Option<String>,
    violations: Vec<String>,
    targets: Vec<String>,
}

struct RustFile {
    path: String,
    scanned: Scanned,
}

struct Row {
    line: usize,
    sites: String,
    allowances: String,
}

struct Repo {
    packages: Vec<Package>,
    files: Vec<RustFile>,
    build_files: Vec<(String, String)>,
    rows: BTreeMap<String, Vec<Row>>,
    registry_missing: bool,
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            let hidden = name.starts_with('.') && name != ".github" && name != ".cargo";
            let build_output = name == "target" && dir.join("Cargo.toml").exists();
            if !hidden && !build_output {
                walk(root, &entry.path(), found)?;
            }
        } else if let Ok(relative) = entry.path().strip_prefix(root) {
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn is_build_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.ends_with(".nix")
        || (path.starts_with(".github/workflows/")
            && (name.ends_with(".yml") || name.ends_with(".yaml")))
        || ((name == "config" || name == "config.toml") && path.split('/').any(|p| p == ".cargo"))
}

impl Repo {
    fn load(root: &Path) -> Result<Self, String> {
        let mut paths = Vec::new();
        walk(root, root, &mut paths).map_err(|e| format!("cannot walk {}: {e}", root.display()))?;
        let read = |path: &str| {
            fs::read_to_string(root.join(path)).map_err(|e| format!("cannot read {path}: {e}"))
        };
        let mut packages = Vec::new();
        for path in paths
            .iter()
            .filter(|p| p.rsplit('/').next() == Some("Cargo.toml"))
        {
            if let Some(package) = Package::parse(path, &read(path)?) {
                packages.push(package);
            }
        }
        packages.sort_by_key(|p| std::cmp::Reverse(p.dir.len()));
        let build_files = paths
            .iter()
            .filter(|p| is_build_file(p))
            .map(|p| Ok((p.clone(), read(p)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let mut files = Vec::new();
        for path in paths.iter().filter(|p| p.ends_with(".rs")) {
            let source = Source::new(read(path)?);
            let crate_root = crate_root(path, &packages);
            let scanned = scan(path, &source, crate_root);
            files.push(RustFile {
                path: path.clone(),
                scanned,
            });
        }
        let registry_missing = !paths.iter().any(|p| p == REGISTRY);
        let rows = if registry_missing {
            BTreeMap::new()
        } else {
            parse_rows(&read(REGISTRY)?)
        };
        Ok(Repo {
            packages,
            files,
            build_files,
            rows,
            registry_missing,
        })
    }

    fn package_of(&self, path: &str) -> Option<&Package> {
        package_of(&self.packages, path)
    }

    fn check(&self) -> Vec<String> {
        let mut violations = Vec::new();
        if self.registry_missing {
            violations.push(format!("{REGISTRY}: missing"));
        }
        let scanned = |what: &str, found: bool| {
            (!found).then(|| format!("scanned no {what}; the check needs the whole repository"))
        };
        violations.extend(
            [
                scanned("Cargo package", !self.packages.is_empty()),
                scanned("Rust file", !self.files.is_empty()),
                scanned(&format!("{REGISTRY} row"), !self.rows.is_empty()),
                scanned(
                    "Nix file",
                    self.build_files.iter().any(|(p, _)| p.ends_with(".nix")),
                ),
                scanned(
                    "workflow",
                    self.build_files
                        .iter()
                        .any(|(p, _)| p.starts_with(".github/workflows/")),
                ),
            ]
            .into_iter()
            .flatten(),
        );
        for package in &self.packages {
            violations.extend(package.violations.iter().cloned());
        }
        for (path, rows) in &self.rows {
            if rows.len() > 1 {
                violations.push(format!(
                    "{REGISTRY}:{}: `{path}` has more than one row",
                    rows[1].line
                ));
            }
            if !self.files.iter().any(|f| &f.path == path) {
                violations.push(format!(
                    "{REGISTRY}:{}: `{path}` does not exist",
                    rows[0].line
                ));
            }
        }
        for file in &self.files {
            let scanned = &file.scanned;
            violations.extend(scanned.violations.iter().cloned());
            for site in scanned.sites.iter().filter(|s| !s.documented) {
                let what = if site.kind == Kind::Block {
                    "unsafe block has no `// SAFETY:` comment".to_owned()
                } else {
                    format!(
                        "{} has no `SAFETY:` comment or `# Safety` doc",
                        site.kind.name(false)
                    )
                };
                violations.push(format!("{}:{}: {what}", file.path, site.line));
            }
            let (sites, allowances) = cells(scanned);
            match self.rows.get(&file.path).and_then(|rows| rows.first()) {
                None if sites == "none" && allowances == "none" => {}
                None => violations.push(format!(
                    "{}: unregistered unsafe code; add `| `{}` | {sites} | {allowances} | why |` to {REGISTRY}",
                    file.path, file.path
                )),
                Some(row) => {
                    if row.sites != sites {
                        violations.push(format!(
                            "{REGISTRY}:{}: `{}` sites are `{sites}`, the row says `{}`",
                            row.line, file.path, row.sites
                        ));
                    }
                    if row.allowances != allowances {
                        violations.push(format!(
                            "{REGISTRY}:{}: `{}` allowances are `{allowances}`, the row says `{}`",
                            row.line, file.path, row.allowances
                        ));
                    }
                }
            }
        }
        for package in &self.packages {
            let owns_unsafe = self.files.iter().any(|f| {
                !f.scanned.sites.is_empty()
                    && self
                        .package_of(&f.path)
                        .is_some_and(|p| p.dir == package.dir)
            });
            match package.level.as_deref() {
                Some("deny") if !owns_unsafe => violations.push(format!(
                    "{}: denies unsafe_code but holds no unsafe code; forbid it",
                    package.manifest()
                )),
                Some("forbid") if owns_unsafe => violations.push(format!(
                    "{}: forbids unsafe_code but holds registered unsafe code",
                    package.manifest()
                )),
                _ => {}
            }
        }
        let mut roots: BTreeSet<String> = self
            .files
            .iter()
            .filter(|f| !self.in_cargo_target(&f.path))
            .map(|f| f.path.clone())
            .collect();
        for (path, text) in &self.build_files {
            for lint in POLICY_LINTS {
                if mentions(text, lint) {
                    violations.push(format!(
                        "{path}: mentions `{lint}`; its level is set only in manifests"
                    ));
                }
            }
            match rustc_sources(path, text) {
                Ok(sources) => roots.extend(sources),
                Err(mut unresolved) => violations.append(&mut unresolved),
            }
        }
        for root in roots {
            match self.files.iter().find(|f| f.path == root) {
                Some(file) if file.scanned.forbids_at_root => {}
                Some(_) => violations.push(format!(
                    "{root}: compiled as its own crate root, so it needs `#![forbid(unsafe_code)]`"
                )),
                None => {
                    violations.push(format!("{root}: rustc compiles it, but it does not exist"))
                }
            }
        }
        violations
    }

    fn in_cargo_target(&self, path: &str) -> bool {
        let Some(package) = self.package_of(path) else {
            return false;
        };
        let local = package.local(path);
        ["src/", "tests/", "benches/", "examples/"]
            .iter()
            .any(|dir| local.starts_with(dir))
            || local == "build.rs"
            || package.targets.iter().any(|target| target == local)
    }

    fn report(&self) -> String {
        let mut out = String::new();
        let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let _ = writeln!(out, "| File | Sites | Allowances |\n| --- | --- | --- |");
        for file in &self.files {
            let (sites, allowances) = cells(&file.scanned);
            if sites == "none" && allowances == "none" {
                continue;
            }
            let _ = writeln!(out, "| `{}` | {sites} | {allowances} |", file.path);
            let package = self
                .package_of(&file.path)
                .map_or("(standalone)".to_owned(), |p| p.manifest());
            let total = totals.entry(package).or_default();
            for site in &file.scanned.sites {
                if site.test {
                    total.1 += 1;
                } else {
                    total.0 += 1;
                }
            }
        }
        let _ = writeln!(out, "\npackage\tproduction\ttests");
        let (mut production, mut tests) = (0, 0);
        for (package, (p, t)) in &totals {
            let _ = writeln!(out, "{package}\t{p}\t{t}");
            production += p;
            tests += t;
        }
        let _ = writeln!(out, "total\t{production}\t{tests}");
        out
    }
}

/// The registry cells a file's scan implies, in canonical form.
fn cells(scanned: &Scanned) -> (String, String) {
    let mut sites: BTreeMap<(bool, Kind), usize> = BTreeMap::new();
    for site in &scanned.sites {
        *sites.entry((site.test, site.kind)).or_default() += 1;
    }
    let sites: Vec<String> = sites
        .iter()
        .map(|(&(test, kind), &count)| {
            let test = if test { "test " } else { "" };
            format!("{count} {test}{}", kind.name(count != 1))
        })
        .collect();
    let mut items: Vec<String> = Vec::new();
    let mut statements: BTreeMap<&str, usize> = BTreeMap::new();
    for allowance in &scanned.allowances {
        match allowance.strip_suffix(": statement") {
            Some(scope) => *statements.entry(scope).or_default() += 1,
            None => items.push(allowance.clone()),
        }
    }
    items.sort();
    items.extend(statements.iter().map(|(scope, count)| {
        format!(
            "{scope}: {count} statement{}",
            if *count == 1 { "" } else { "s" }
        )
    }));
    let join = |parts: Vec<String>| {
        if parts.is_empty() {
            "none".to_owned()
        } else {
            parts.join(", ")
        }
    };
    (join(sites), join(items))
}

fn parse_rows(text: &str) -> BTreeMap<String, Vec<Row>> {
    let mut rows: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let cells: Vec<&str> = line.trim().split('|').map(str::trim).collect();
        let [_, file, sites, allowances, ..] = cells.as_slice() else {
            continue;
        };
        let Some(path) = file.strip_prefix('`').and_then(|f| f.strip_suffix('`')) else {
            continue;
        };
        if !path.ends_with(".rs") {
            continue;
        }
        rows.entry(path.to_owned()).or_default().push(Row {
            line: n + 1,
            sites: (*sites).to_owned(),
            allowances: (*allowances).to_owned(),
        });
    }
    rows
}

fn package_of<'a>(packages: &'a [Package], path: &str) -> Option<&'a Package> {
    packages
        .iter()
        .find(|p| p.dir.is_empty() || path.starts_with(&format!("{}/", p.dir)))
}

fn crate_root(path: &str, packages: &[Package]) -> bool {
    let Some(package) = package_of(packages, path) else {
        return true;
    };
    let local = package.local(path);
    let top = |dir: &str| {
        local
            .strip_prefix(dir)
            .is_some_and(|rest| !rest.contains('/'))
    };
    matches!(local, "src/lib.rs" | "src/main.rs" | "build.rs")
        || top("src/bin/")
        || top("tests/")
        || top("benches/")
        || top("examples/")
        || package.targets.iter().any(|target| target == local)
        || !["src/", "tests/", "benches/", "examples/"]
            .iter()
            .any(|dir| local.starts_with(dir))
}

fn mentions(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at]
            .bytes()
            .next_back()
            .is_none_or(|b| !ident_byte(b));
        let after = text[at + word.len()..]
            .bytes()
            .next()
            .is_none_or(|b| !ident_byte(b));
        before && after
    })
}

/// The `.rs` files that `rustc` command lines in a build file compile.
fn rustc_sources(path: &str, text: &str) -> Result<Vec<String>, Vec<String>> {
    const VALUE_FLAGS: [&str; 14] = [
        "--edition",
        "-o",
        "-D",
        "-A",
        "-W",
        "-C",
        "-L",
        "--extern",
        "--cfg",
        "--crate-type",
        "--crate-name",
        "--target",
        "--emit",
        "--out-dir",
    ];
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let joined = text.replace("\\\n", " ");
    let (mut sources, mut unresolved) = (Vec::new(), Vec::new());
    for (n, line) in joined.lines().enumerate() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(at) = words.iter().position(|w| {
            w.trim_start_matches(['\'', '"', '(']).rsplit('/').next() == Some("rustc")
        }) else {
            continue;
        };
        if words
            .get(at + 1)
            .is_none_or(|w| w.starts_with('=') || w.starts_with(';'))
        {
            continue;
        }
        let mut args = words[at + 1..]
            .iter()
            .take_while(|w| !matches!(**w, "&&" | "||" | "|" | ";" | ">" | "2>&1"))
            .map(|w| w.trim_matches('"'));
        while let Some(arg) = args.next() {
            if VALUE_FLAGS.contains(&arg) {
                args.next();
                continue;
            }
            if arg.starts_with('-') {
                continue;
            }
            if let Some(local) = arg.strip_prefix("${").and_then(|a| a.strip_suffix('}')) {
                if local.starts_with("./") || local.starts_with("../") {
                    sources.push(normalize(dir, local));
                    continue;
                }
            }
            if !arg.contains('$') && arg.ends_with(".rs") {
                sources.push(normalize("", arg));
                continue;
            }
            match INDIRECT_RUSTC_SOURCES
                .iter()
                .find(|(file, argument, _)| *file == path && *argument == arg)
            {
                Some((_, _, root)) => sources.push((*root).to_owned()),
                None => unresolved.push(format!(
                    "{path}:{}: rustc compiles `{arg}`, which names no file; register it in \
                     INDIRECT_RUSTC_SOURCES (nix/unsafe-policy-check.rs) with the root it compiles",
                    n + 1
                )),
            }
        }
    }
    if unresolved.is_empty() {
        Ok(sources)
    } else {
        Err(unresolved)
    }
}

fn normalize(dir: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

impl Package {
    fn parse(path: &str, text: &str) -> Option<Self> {
        let dir = path
            .strip_suffix("Cargo.toml")?
            .trim_end_matches('/')
            .to_owned();
        let mut section = String::new();
        let mut values: BTreeMap<(String, String), String> = BTreeMap::new();
        let mut targets = Vec::new();
        let mut is_package = false;
        for line in text.lines() {
            let line = line.split(" #").next().unwrap_or("").trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').map(|l| l.trim_matches(['[', ']'])) {
                section = name.trim().to_owned();
                is_package |= section == "package";
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if matches!(
                section.as_str(),
                "lib" | "bin" | "test" | "bench" | "example"
            ) && key == "path"
                || section == "package" && key == "build"
            {
                targets.push(value.trim_matches('"').to_owned());
            }
            let level = value
                .split_once("level")
                .map_or(value, |(_, rest)| rest.trim_start_matches([' ', '=']))
                .split('"')
                .nth(1)
                .unwrap_or(value)
                .to_owned();
            values.insert((section.clone(), key.to_owned()), level);
        }
        if !is_package {
            return None;
        }
        let get =
            |section: &str, key: &str| values.get(&(section.to_owned(), key.to_owned())).cloned();
        let level = get("lints.rust", "unsafe_code");
        let mut violations = Vec::new();
        if get("lints", "workspace").is_some() {
            violations.push(format!(
                "{path}: `[lints] workspace = true`; set the unsafe lints in this manifest"
            ));
        }
        match level.as_deref() {
            Some("forbid" | "deny") => {}
            other => violations.push(format!(
                "{path}: `[lints.rust] unsafe_code` must be \"forbid\" or \"deny\", found {other:?}"
            )),
        }
        if get("lints.clippy", "undocumented_unsafe_blocks").as_deref() != Some("deny") {
            violations.push(format!(
                "{path}: `[lints.clippy] undocumented_unsafe_blocks` must be \"deny\""
            ));
        }
        if level.as_deref() == Some("deny")
            && get("lints.rust", "unsafe_op_in_unsafe_fn").as_deref() != Some("deny")
        {
            violations.push(format!(
                "{path}: a deny package needs `[lints.rust] unsafe_op_in_unsafe_fn = \"deny\"`"
            ));
        }
        for (section, group) in [
            ("lints.rust", "rust_2024_compatibility"),
            ("lints.clippy", "restriction"),
        ] {
            if get(section, group).is_some_and(|level| level != "deny" && level != "forbid") {
                violations.push(format!(
                    "{path}: `[{section}] {group}` lowers a policy lint; the policy allows no exception"
                ));
            }
        }
        Some(Package {
            dir,
            level,
            violations,
            targets,
        })
    }

    fn local<'a>(&self, path: &'a str) -> &'a str {
        if self.dir.is_empty() {
            path
        } else {
            &path[self.dir.len() + 1..]
        }
    }

    fn manifest(&self) -> String {
        if self.dir.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{}/Cargo.toml", self.dir)
        }
    }
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const MANIFEST_DENY: &str = "[package]\nname = \"a\"\n\n[lints.rust]\nunsafe_code = \"deny\"\nunsafe_op_in_unsafe_fn = \"deny\"\n\n[lints.clippy]\nundocumented_unsafe_blocks = \"deny\"\n";
    const MANIFEST_FORBID: &str = "[package]\nname = \"b\"\n\n[lints.rust]\nunsafe_code = { level = \"forbid\", priority = 0 }\n\n[lints.clippy]\nundocumented_unsafe_blocks = \"deny\"\n";
    const LIB: &str = "#[allow(unsafe_code)]\nmod platform;\n";
    const PLATFORM: &str =
        "pub fn f() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n";
    const REGISTRY_ROWS: &str = "| File | Sites | Allowances | Why |\n| --- | --- | --- | --- |\n| `a/src/lib.rs` | none | mod platform | fence |\n| `a/src/platform.rs` | 1 block | none | g |\n";

    struct Fixture(std::path::PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A valid repository with the given files replaced or added (empty content removes one).
    fn repo(changes: &[(&str, &str)]) -> (Fixture, Vec<String>) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "unsafe-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut files: BTreeMap<&str, &str> = [
            ("a/Cargo.toml", MANIFEST_DENY),
            ("a/src/lib.rs", LIB),
            ("a/src/platform.rs", PLATFORM),
            ("b/Cargo.toml", MANIFEST_FORBID),
            ("b/src/lib.rs", "pub fn h() {}\n"),
            ("tools/run.rs", "#![forbid(unsafe_code)]\nfn main() {}\n"),
            ("docs/UNSAFE.md", REGISTRY_ROWS),
            ("flake.nix", "{ }\n"),
            (".github/workflows/ci.yml", "on: push\n"),
        ]
        .into_iter()
        .collect();
        for (path, text) in changes {
            if text.is_empty() {
                files.remove(path);
            } else {
                files.insert(path, text);
            }
        }
        for (path, text) in &files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let violations = Repo::load(&dir).unwrap().check();
        (Fixture(dir), violations)
    }

    fn fails(changes: &[(&str, &str)], needle: &str) {
        let (_dir, violations) = repo(changes);
        assert!(
            violations.iter().any(|v| v.contains(needle)),
            "expected a violation containing {needle:?}, got {violations:#?}"
        );
    }

    #[test]
    fn the_base_fixture_passes() {
        let (_dir, violations) = repo(&[]);
        assert_eq!(violations, Vec::<String>::new());
    }

    #[test]
    fn literals_comments_and_lifetimes_hold_no_sites() {
        let text = "fn f<'a>(x: &'a str) -> char {\n    let _ = \"unsafe { }\";\n    let _ = r#\"unsafe { \"# ;\n    let _ = br\"unsafe {\";\n    /* unsafe { /* nested */ unsafe { } */\n    // unsafe { }\n    let _ = '\\'';\n    let _ = '{';\n    'x'\n}\n";
        let source = Source::new(text.to_owned());
        assert!(scan("a/src/x.rs", &source, false).sites.is_empty());
        assert!(!source.tokens.iter().any(|t| t.is("unsafe")));
        assert!(
            source.tokens.iter().any(|t| t.is("a")),
            "a lifetime stays code"
        );
        // An escaped quote closes after its escape, so `'"'` that follows stays a char.
        let compact = "fn f(c: char) -> bool {\n    matches!(c, '\\''|'\"')\n}\n#[allow(unsafe_code)]\nfn g() {\n    // SAFETY: reads nothing.\n    unsafe { h() }\n}\n";
        let scanned = scan("a/src/y.rs", &Source::new(compact.to_owned()), false);
        assert_eq!(
            scanned.sites.len(),
            1,
            "the block after the literals is code"
        );
        assert_eq!(scanned.allowances, ["fn g"]);
    }

    #[test]
    fn a_second_allowed_module_fails() {
        fails(
            &[
                (
                    "a/src/lib.rs",
                    "#[allow(unsafe_code)]\nmod platform;\n#[allow(unsafe_code)]\nmod extra;\n",
                ),
                ("a/src/extra.rs", PLATFORM),
            ],
            "unregistered unsafe code",
        );
    }

    #[test]
    fn a_cfg_gated_allowed_module_fails() {
        fails(
            &[
                (
                    "a/src/lib.rs",
                    "#[allow(unsafe_code)]\nmod platform;\n#[cfg(target_os = \"macos\")]\n#[allow(unsafe_code)]\nmod mac;\n",
                ),
                ("a/src/mac.rs", PLATFORM),
            ],
            "a/src/lib.rs` allowances are `mod mac, mod platform`",
        );
    }

    #[test]
    fn widening_a_module_allowance_to_the_crate_fails() {
        fails(
            &[("a/src/lib.rs", "#![allow(unsafe_code)]\nmod platform;\n")],
            "allowances are `crate`",
        );
    }

    #[test]
    fn a_module_wide_allowance_without_a_new_site_fails() {
        fails(
            &[(
                "a/src/platform.rs",
                &format!("#![allow(unsafe_code)]\n{PLATFORM}"),
            )],
            "allowances are `module`",
        );
    }

    #[test]
    fn statement_allowances_record_their_function() {
        let text = "pub fn f() -> i32 {\n    #[allow(unsafe_code)]\n    // SAFETY: reads nothing.\n    let x = unsafe { g() };\n    x\n}\n";
        let (sites, allowances) = cells(&scan("a/src/p.rs", &Source::new(text.to_owned()), false));
        assert_eq!(
            (sites.as_str(), allowances.as_str()),
            ("1 block", "in fn f: 1 statement")
        );
        let inner = "pub fn f() -> i32 {\n    #![allow(unsafe_code)]\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n";
        let (_, allowances) = cells(&scan("a/src/p.rs", &Source::new(inner.to_owned()), false));
        assert_eq!(allowances, "fn f", "an inner allowance names its function");
    }

    #[test]
    fn statement_allowances_admit_no_compound_form() {
        for (form, reason) in [
            ("if rotation { let _ = unsafe { g() }; }", "`if`"),
            ("if unsafe { g() } { found.push(1); }", "`if`"),
            ("let _: () = unsafe { if rotation { g() } };", "`if`"),
            ("let _: () = unsafe { { g() } };", "a block"),
            (
                "let _ = unsafe { g() } + unsafe { g() };",
                "more than one unsafe block",
            ),
            ("let _ = list.iter().map(|x| unsafe { g(x) });", "a closure"),
            ("match x { _ => unsafe { g() } };", "`match`"),
            ("let _ = 1;", "no unsafe block"),
            (
                "let Some(x) = unsafe { g() } else { return };",
                "an `else` block",
            ),
            (
                "let v = unsafe { g() } + h::<A, B>(if c { 1 } else { 2 });",
                "`if`",
            ),
        ] {
            let text = format!(
                "pub fn f() {{\n    #[allow(unsafe_code)]\n    // SAFETY: reads nothing.\n    {form}\n}}\n"
            );
            let scanned = scan("a/src/p.rs", &Source::new(text), false);
            assert!(
                scanned.violations.iter().any(|v| v.contains(reason)),
                "{form}: {:?}",
                scanned.violations
            );
        }
        for ok in [
            "let applied: bool = unsafe { let _: () = msg_send![a, b: c]; msg_send![a, d] };",
            // A block statement ends with its block; the next statement is not its target.
            "unsafe { g() }\n    if rotation { let _ = 1; }",
            "unsafe { g() }\n    #[allow(unsafe_code)]\n    // SAFETY: reads nothing.\n    let _ = unsafe { g() };",
            "let n = unsafe { g() }.max(1) as u32;",
        ] {
            let text = format!(
                "pub fn f() {{\n    #[allow(unsafe_code)]\n    // SAFETY: reads nothing.\n    {ok}\n}}\n"
            );
            let scanned = scan("a/src/p.rs", &Source::new(text), false);
            assert!(
                scanned.violations.is_empty(),
                "{ok}: {:?}",
                scanned.violations
            );
        }
    }

    #[test]
    fn lowering_the_safety_lints_fails() {
        fails(
            &[(
                "a/src/platform.rs",
                &format!("#![allow(clippy::undocumented_unsafe_blocks)]\n{PLATFORM}"),
            )],
            "lowers `undocumented_unsafe_blocks`",
        );
        fails(
            &[(
                "a/src/platform.rs",
                &format!("#[cfg_attr(test, allow(unsafe_op_in_unsafe_fn))]\n{PLATFORM}"),
            )],
            "lowers `unsafe_op_in_unsafe_fn`",
        );
    }

    #[test]
    fn lint_flags_in_build_configuration_fail() {
        fails(
            &[("flake.nix", "{ RUSTFLAGS = \"-A unsafe_code\"; }\n")],
            "flake.nix: mentions `unsafe_code`",
        );
        fails(
            &[(
                ".cargo/config.toml",
                "[build]\nrustflags = [\"-A\", \"unsafe_code\"]\n",
            )],
            ".cargo/config.toml: mentions",
        );
        fails(
            &[(
                ".cargo/config",
                "[build]\nrustflags = [\"-A\", \"clippy::restriction\"]\n",
            )],
            ".cargo/config: mentions `clippy::restriction`",
        );
        fails(
            &[(
                ".github/workflows/x.yml",
                "run: cargo clippy -- -A undocumented_unsafe_blocks\n",
            )],
            "x.yml: mentions",
        );
    }

    #[test]
    fn other_lint_spellings_and_groups_fail() {
        fails(
            &[("nix/x.nix", "{ f = \"-A unsafe-code\"; }\n")],
            "mentions `unsafe-code`",
        );
        fails(
            &[(
                ".github/workflows/x.yml",
                "run: cargo build --config build.rustflags=['--cap-lints','warn']\n",
            )],
            "mentions `--cap-lints`",
        );
        fails(
            &[(
                "a/src/platform.rs",
                &format!("#![allow(rust_2024_compatibility)]\n{PLATFORM}"),
            )],
            "lowers `rust_2024_compatibility`",
        );
        fails(
            &[(
                "a/Cargo.toml",
                &MANIFEST_DENY.replace(
                    "[lints.clippy]",
                    "rust_2024_compatibility = { level = \"allow\", priority = -1 }\n\n[lints.clippy]",
                ),
            )],
            "`[lints.rust] rust_2024_compatibility` lowers a policy lint",
        );
    }

    #[test]
    fn only_build_output_target_directories_are_skipped() {
        fails(
            &[(
                "a/src/target/mod.rs",
                "pub fn t() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n",
            )],
            "a/src/target/mod.rs: unregistered unsafe code",
        );
        let (_dir, ok) = repo(&[(
            "a/target/debug/build.rs",
            "fn main() {\n    unsafe { g() }\n}\n",
        )]);
        assert_eq!(ok, Vec::<String>::new());
    }

    #[test]
    fn an_empty_or_partial_scan_fails() {
        fails(
            &[("flake.nix", ""), ("nix/check.nix", "")],
            "scanned no Nix file",
        );
        fails(&[(".github/workflows/ci.yml", "")], "scanned no workflow");
        fails(
            &[("docs/UNSAFE.md", "# Unsafe code\n\nNo table.\n")],
            "scanned no docs/UNSAFE.md row",
        );
        let dir = std::env::temp_dir().join(format!("unsafe-policy-{}-empty", std::process::id()));
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join(REGISTRY), REGISTRY_ROWS).unwrap();
        let _cleanup = Fixture(dir.clone());
        let violations = Repo::load(&dir).unwrap().check();
        for what in ["Cargo package", "Rust file", "Nix file", "workflow"] {
            assert!(
                violations
                    .iter()
                    .any(|v| v.contains(&format!("scanned no {what}"))),
                "{what}: {violations:#?}"
            );
        }
    }

    #[test]
    fn only_an_unconditional_forbid_marks_a_root() {
        fails(
            &[(
                "tools/run.rs",
                "#![cfg_attr(not(test), forbid(unsafe_code))]\nfn main() {}\n",
            )],
            "tools/run.rs: compiled as its own crate root",
        );
        fails(
            &[
                (
                    "nix/check.nix",
                    "''\n  ${toolchain}/bin/rustc ${../a/src/solo.rs} -o out\n''\n",
                ),
                ("a/src/solo.rs", "pub fn s() {}\n"),
            ],
            "a/src/solo.rs: compiled as its own crate root",
        );
    }

    #[test]
    fn manifests_need_every_lint() {
        fails(
            &[(
                "a/Cargo.toml",
                &MANIFEST_DENY.replace("\"deny\"\nunsafe_op", "\"warn\"\nunsafe_op"),
            )],
            "must be \"forbid\" or \"deny\"",
        );
        fails(
            &[(
                "b/Cargo.toml",
                "[package]\nname = \"b\"\n\n[lints.rust]\nunsafe_code = \"forbid\"\n",
            )],
            "undocumented_unsafe_blocks` must be",
        );
        fails(
            &[(
                "a/Cargo.toml",
                &MANIFEST_DENY.replace("unsafe_op_in_unsafe_fn = \"deny\"\n", ""),
            )],
            "unsafe_op_in_unsafe_fn = \"deny\"",
        );
        fails(
            &[(
                "b/Cargo.toml",
                "[package]\nname = \"b\"\n\n[lints]\nworkspace = true\n",
            )],
            "workspace = true",
        );
    }

    #[test]
    fn a_deny_package_without_unsafe_must_forbid() {
        fails(
            &[("b/Cargo.toml", &MANIFEST_DENY.replace("\"a\"", "\"b\""))],
            "b/Cargo.toml: denies unsafe_code but holds no unsafe code",
        );
    }

    #[test]
    fn standalone_and_rustc_roots_must_forbid() {
        fails(
            &[("tools/run.rs", "fn main() {}\n")],
            "tools/run.rs: compiled as its own crate root",
        );
        fails(
            &[
                (
                    "nix/check.nix",
                    "''\n  rustc --edition=2024 --test ${../a/src/solo.rs} -o unit\n''\n",
                ),
                ("a/src/solo.rs", "pub fn s() {}\n"),
            ],
            "a/src/solo.rs: compiled as its own crate root",
        );
        let (_dir, ok) = repo(&[
            (
                "nix/check.nix",
                "{\n  rustc = toolchain;\n  x = ''rustc --edition 2024 ${../a/src/solo.rs} -o \"$out\"'';\n}\n",
            ),
            ("a/src/solo.rs", "#![forbid(unsafe_code)]\npub fn s() {}\n"),
        ]);
        assert_eq!(ok, Vec::<String>::new());
    }

    #[test]
    fn an_unresolved_rustc_source_must_be_registered() {
        fails(
            &[("nix/check.nix", "''\n  rustc ${someVar} -o out\n''\n")],
            "register it",
        );
    }

    #[test]
    fn the_registry_must_match_exactly() {
        fails(
            &[(
                "docs/UNSAFE.md",
                &REGISTRY_ROWS.replace("1 block", "2 blocks"),
            )],
            "sites are `1 block`, the row says `2 blocks`",
        );
        fails(
            &[(
                "docs/UNSAFE.md",
                &format!("{REGISTRY_ROWS}| `a/src/gone.rs` | 1 block | none | x |\n"),
            )],
            "does not exist",
        );
        fails(
            &[("a/src/platform.rs", "pub fn f() {}\n")],
            "denies unsafe_code but holds no unsafe code",
        );
        fails(
            &[("a/src/platform.rs", "pub fn f() {}\n")],
            "sites are `none`, the row says `1 block`",
        );
    }

    #[test]
    fn test_status_is_part_of_the_count() {
        let gated = "#[cfg(all(test, target_os = \"macos\"))]\npub fn f() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n";
        fails(&[("a/src/platform.rs", gated)], "sites are `1 test block`");
        let either = "#[cfg(any(test, target_os = \"macos\"))]\npub fn f() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n";
        let (_dir, ok) = repo(&[("a/src/platform.rs", either)]);
        assert_eq!(
            ok,
            Vec::<String>::new(),
            "cfg(any(test, ..)) also builds outside tests"
        );
    }

    #[test]
    fn every_site_needs_its_safety_note() {
        fails(
            &[(
                "a/src/platform.rs",
                "pub fn f() -> i32 {\n    unsafe { g() }\n}\n",
            )],
            "unsafe block has no `// SAFETY:` comment",
        );
        fails(
            &[
                (
                    "a/src/platform.rs",
                    "/// Calls g.\npub unsafe fn f() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n",
                ),
                (
                    "docs/UNSAFE.md",
                    &REGISTRY_ROWS.replace("| 1 block |", "| 1 block, 1 unsafe fn |"),
                ),
            ],
            "unsafe fn has no `SAFETY:` comment",
        );
        let documented = "/// # Safety\n/// Single-threaded callers only.\npub unsafe fn f() -> i32 {\n    // SAFETY: reads nothing.\n    unsafe { g() }\n}\n#[allow(unsafe_code)]\n// SAFETY: integer-only C functions.\nunsafe extern \"C\" {\n    safe fn g() -> i32;\n}\n";
        let (_dir, ok) = repo(&[
            ("a/src/platform.rs", documented),
            (
                "docs/UNSAFE.md",
                &REGISTRY_ROWS.replace(
                    "| 1 block | none |",
                    "| 1 block, 1 unsafe fn, 1 extern block | extern block |",
                ),
            ),
        ]);
        assert_eq!(ok, Vec::<String>::new());
    }
}
