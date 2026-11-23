//! `eon report` (EON-C1): one Markdown bug report for the issue template, printed to stdout.
//! It shares Doctor's read-only observations and may run a bounded `sw_vers` on macOS.
//! One redactor covers every section.

use std::{
    collections::VecDeque,
    env,
    ffi::{OsStr, OsString},
    fs,
    ops::Range,
    path::{Component, Path, PathBuf},
    process::Command,
};

use crate::{doctor, update};

/// Environment values shorter than this stay: they are too common to identify anyone. Fixed,
/// since a configurable redactor invites leaks.
const SHORTEST_SECRET: usize = 8;
/// Whether the report may show variable `name`'s value: the Compositor section's, `HOME`,
/// which becomes `~`, Eon's own identifiers, and an XDG base directory `under_home`, which the
/// `HOME` rule then shows as a `~` path so the report still says where its log and
/// configuration are.
fn shown(name: &str, under_home: bool) -> bool {
    match name {
        "XDG_CURRENT_DESKTOP" | "XDG_SESSION_TYPE" | "HOME" => true,
        "XDG_CONFIG_HOME" | "XDG_DATA_HOME" | "XDG_CACHE_HOME" | "XDG_STATE_HOME" => under_home,
        _ => name.starts_with("EON_"),
    }
}

/// Whether this command line is `eon report`, which the product answers before Runtime sees
/// it. With extra arguments or help it stays with Runtime, which refuses or explains it.
pub(crate) fn wanted(arguments: &[OsString]) -> bool {
    matches!(arguments, [program, command]
        if Path::new(program).file_name() == Some(OsStr::new("eon")) && command == "report")
}

/// Prints the report and returns the exit status.
pub(crate) fn run(
    arguments: &[OsString],
    invoked: update::Build,
    inputs: &eon_runtime::Inputs,
) -> u8 {
    let diagnostics = doctor::Diagnostics::collect(&arguments[0], invoked, inputs, true);
    let checks = diagnostics.human();
    let facts = eon_runtime::report_facts(inputs, diagnostics.window);
    let install = match &diagnostics.installation.error {
        Some(reason) => Err(reason.as_str()),
        None => Ok(diagnostics.installation.kind),
    };
    let distribution = if cfg!(target_os = "macos") {
        macos_version()
    } else {
        linux_distribution(OS_RELEASE)
    };
    let report = assemble(
        &facts,
        install,
        distribution,
        &diagnostics.compositor,
        &diagnostics.configuration_warnings,
        &checks,
        &Redaction::of_this_process(),
    );
    doctor::write_stdout(&report, 0)
}

/// The report, with `redaction` over all of it.
fn assemble(
    facts: &eon_runtime::ReportFacts,
    install: Result<&str, &str>,
    distribution: Result<String, String>,
    compositor: &str,
    config_warnings: &Result<Vec<String>, String>,
    checks: &str,
    redaction: &Redaction,
) -> String {
    // A reason's first line says what failed; later ones can quote the input, such as a
    // configuration line. It is redacted whole first, so a value the cut splits still goes.
    let unknown = |reason: &str| {
        let reason = redaction.apply(reason);
        format!("unknown ({})", reason.lines().next().unwrap_or(""))
    };
    let fenced = |language: &str, text: &str| {
        let text = text.strip_suffix('\n').unwrap_or(text);
        format!("```{language}\n{text}\n```")
    };
    let versions = match &facts.versions {
        Ok(versions) => fenced("text", versions),
        Err(reason) => unknown(reason),
    };
    let window = match &facts.window {
        Ok(Some(window)) => fenced("json", window),
        Ok(None) => "none running".into(),
        Err(reason) => unknown(reason),
    };
    let install = match install {
        Ok(kind) => format!("{} (install_kind: {kind})", install_route(kind)),
        Err(reason) => unknown(reason),
    };
    let distribution = distribution.unwrap_or_else(|reason| unknown(&reason));
    let warnings = match config_warnings {
        Ok(warnings) if warnings.is_empty() => "none".into(),
        Ok(warnings) => fenced("text", &warnings.join("\n")),
        Err(reason) => unknown(reason),
    };
    let log = match &facts.log {
        Ok((path, tail)) => fenced("text", &format!("{}\n{tail}", path.display())),
        Err(reason) => unknown(reason),
    };
    let checks = fenced("text", checks);
    redaction.apply(&format!(
        "<!-- eon report: read this before sharing it. Paths under your home are shown as ~;
     long environment values (8 bytes or more) are removed. The log tail may still hold other
     private text: read it before sharing. Add steps, expected and actual results yourself. -->
### Eon version
{versions}
### Window
{window}
### Install route
{install}
### Distribution
{distribution}
### Compositor
{compositor}
### GPU and driver
not detected yet; please fill in
### Configuration warnings
{warnings}
### Doctor
{checks}
### Logs
{log}
"
    ))
}

/// The bug template's Install route option for an EON-C41 `install_kind`.
fn install_route(kind: &str) -> &'static str {
    match kind {
        "nix-profile" => "Nix profile (nix profile add)",
        "externally-managed" => "Flake input (NixOS, Home Manager or another flake)",
        "eon-install" => "Eon installer (eon-install)",
        _ => "Other",
    }
}

/// Where Linux keeps os-release, in the order the report reads them.
const OS_RELEASE: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

/// os-release's `PRETTY_NAME`, from the first of the two paths that can be read.
fn linux_distribution([first, second]: [&str; 2]) -> Result<String, String> {
    let (path, text) = [first, second]
        .into_iter()
        .find_map(|path| Some((path, fs::read_to_string(path).ok()?)))
        .ok_or_else(|| format!("neither {first} nor {second} is readable"))?;
    pretty_name(&text).ok_or_else(|| format!("{path} has no PRETTY_NAME"))
}

fn pretty_name(os_release: &str) -> Option<String> {
    let value = os_release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?
        .trim();
    let value = ['"', '\'']
        .into_iter()
        .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(value);
    (!value.is_empty()).then(|| value.to_owned())
}

/// `macOS <ProductVersion>` from `sw_vers`.
fn macos_version() -> Result<String, String> {
    let mut command = Command::new("/usr/bin/sw_vers");
    command.arg("-productVersion");
    let output = update::bounded(command, doctor::LOCAL_PROBE_LIMIT).map_err(|error| {
        error
            .trouble(["missing", "timed-out", "failed"], "sw_vers")
            .1
    })?;
    if !output.status.success() {
        return Err(format!("sw_vers failed: {}", output.status));
    }
    macos_name(&output.stdout).ok_or_else(|| "sw_vers printed no version".into())
}

fn macos_name(sw_vers: &[u8]) -> Option<String> {
    let version = std::str::from_utf8(sw_vers).ok()?.trim();
    (!version.is_empty()).then(|| format!("macOS {version}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `HOME`, raw or canonical, at a path boundary: `~`.
    Home,
    /// `USER` or `LOGNAME`, as a whole word.
    Name,
    /// Any other environment value of `SHORTEST_SECRET` bytes or more, anywhere.
    Value,
}

#[derive(PartialEq, Eq)]
enum Segment {
    Literal(Vec<char>),
    Slashes(usize),
}

struct Pattern {
    segments: Vec<Segment>,
    first: char,
    min_len: usize,
    kind: Kind,
}

impl Pattern {
    fn literal_only(&self) -> Option<&[char]> {
        match self.segments.as_slice() {
            [Segment::Literal(chars)] => Some(chars),
            _ => None,
        }
    }
}

fn compile_segments(chars: &[char]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let start = i;
            while i < chars.len() && chars[i] == '\\' {
                i += 1;
            }
            segments.push(Segment::Slashes(i - start));
        } else {
            let start = i;
            while i < chars.len() && chars[i] != '\\' {
                i += 1;
            }
            segments.push(Segment::Literal(chars[start..i].to_vec()));
        }
    }
    segments
}

#[cfg(test)]
thread_local! {
    static MATCH_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_work(steps: usize) {
    MATCH_WORK.with(|work| work.set(work.get() + steps));
}

#[cfg(not(test))]
#[inline(always)]
fn count_work(_steps: usize) {}

/// What the report removes; at each position the longest valid match wins, with ties broken in
/// pattern priority order (`Home`, then `Name`, then `Value`).
struct Redaction {
    patterns: Vec<Pattern>,
}

impl Redaction {
    fn of_this_process() -> Self {
        let canonical_home = env::var_os("HOME").and_then(|home| fs::canonicalize(home).ok());
        Self::new(env::vars_os(), canonical_home)
    }

    fn new(
        environment: impl IntoIterator<Item = (OsString, OsString)>,
        canonical_home: Option<PathBuf>,
    ) -> Self {
        let environment: Vec<(String, String)> = environment
            .into_iter()
            .map(|(name, value)| {
                let value = value.to_string_lossy().into_owned();
                (name.to_string_lossy().into_owned(), value)
            })
            .collect();
        let homes: Vec<String> = environment
            .iter()
            .filter(|(name, _)| name == "HOME")
            .map(|(_, home)| home.clone())
            .chain(canonical_home.map(|home| home.to_string_lossy().into_owned()))
            .map(|home| home.trim_end_matches('/').to_owned())
            // A home of `/` would turn every absolute path into `~`.
            .filter(|home| !home.is_empty())
            .collect();
        // Literally, as the home rule masks it: `/srv//x/al/state` keeps `/srv//x/al` whole.
        let under_home = |value: &str| {
            homes.iter().any(|home| {
                value.strip_prefix(home.as_str()).is_some_and(|rest| {
                    (rest.is_empty() || rest.starts_with('/'))
                        && !Path::new(rest)
                            .components()
                            .any(|part| part == Component::ParentDir)
                })
            })
        };
        // A value the report shows stays even where another variable holds it too.
        let shown_values: Vec<&str> = environment
            .iter()
            .filter(|(name, value)| shown(name, under_home(value)))
            .map(|(_, value)| value.as_str())
            .collect();
        let mut patterns = Vec::new();
        let mut push_pattern = |value: &str, kind: Kind| {
            let mut chars = value.chars();
            let Some(first) = chars.next() else {
                return;
            };
            let chars: Vec<char> = std::iter::once(first).chain(chars).collect();
            let segments = compile_segments(&chars);
            if !patterns
                .iter()
                .any(|existing: &Pattern| existing.kind == kind && existing.segments == segments)
            {
                patterns.push(Pattern {
                    segments,
                    first,
                    min_len: value.len(),
                    kind,
                });
            }
        };
        for home in &homes {
            push_pattern(home, Kind::Home);
        }
        for (name, value) in &environment {
            if name == "HOME" || value.is_empty() || shown_values.contains(&value.as_str()) {
                continue;
            }
            if matches!(name.as_str(), "USER" | "LOGNAME") {
                push_pattern(value, Kind::Name);
            }
        }
        for (name, value) in &environment {
            if name == "HOME" || value.is_empty() || shown_values.contains(&value.as_str()) {
                continue;
            }
            if value.len() >= SHORTEST_SECRET {
                push_pattern(value, Kind::Value);
            }
        }
        Self { patterns }
    }

    fn apply(&self, text: &str) -> String {
        // A user name ends where letters and digits do: `lucca-desktop` holds `lucca`.
        let word = |ch: char| ch.is_alphanumeric();
        // A home ends at whatever cannot continue its last path segment's name.
        let path_end = |ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-'));
        let accept_end = |kind: Kind, abs_end: usize| {
            let after = next_decoded_char(&text[abs_end..]);
            match kind {
                Kind::Home => after.is_none_or(path_end),
                Kind::Value => true,
                Kind::Name => !after.is_some_and(word),
            }
        };
        let text_has_backslash = text.contains('\\');
        let mut slash_index: Option<SlashIndex> = None;
        let mut slash_matches: Vec<Option<SlashMatches>> = vec![None; self.patterns.len()];
        let mut result = String::with_capacity(text.len());
        let mut index = 0;
        while let Some(ch) = text[index..].chars().next() {
            let rest = &text[index..];
            let before = prev_decoded_char(&text[..index]);
            let mut best: Option<(usize, Kind)> = None;
            for (pat_idx, pattern) in self.patterns.iter().enumerate() {
                count_work(1);
                if rest.len() < pattern.min_len || (ch != pattern.first && ch != '\\') {
                    continue;
                }
                if pattern.kind == Kind::Name && before.is_some_and(word) {
                    continue;
                }
                let matched_len = if let Some(chars) = pattern.literal_only() {
                    match_literal(chars, rest).filter(|&len| accept_end(pattern.kind, index + len))
                } else {
                    if !text_has_backslash {
                        continue;
                    }
                    let cached = slash_matches[pat_idx].get_or_insert_with(|| {
                        let slash_index = slash_index.get_or_insert_with(|| SlashIndex::new(text));
                        SlashMatches::new(match_slash_pattern(
                            &pattern.segments,
                            text,
                            slash_index,
                            |abs_end| accept_end(pattern.kind, abs_end),
                        ))
                    });
                    cached.len_at(index)
                };
                if let Some(len) = matched_len
                    && best.is_none_or(|(best_len, _)| len > best_len)
                {
                    best = Some((len, pattern.kind));
                }
            }
            match best {
                Some((len, kind)) => {
                    result.push_str(match kind {
                        Kind::Home => "~",
                        Kind::Value => "[redacted]",
                        Kind::Name => "[user]",
                    });
                    index += len;
                }
                None => {
                    result.push(ch);
                    index += ch.len_utf8();
                }
            }
        }
        result
    }
}

/// Decodes the first logical character at the start of `rest`, interpreting a single- or
/// double-escaped Rust/JSON/hex escape token when present so boundary checks see the underlying
/// character rather than the leading backslash of the next escape.
fn next_decoded_char(rest: &str) -> Option<char> {
    let first = rest.chars().next()?;
    if first != '\\' {
        return Some(first);
    }
    let after1 = &rest[1..];
    if let Some(after2) = after1.strip_prefix('\\')
        && let Some((ch, _)) = decode_escape_body(after2, true)
    {
        return Some(ch);
    }
    if let Some((ch, _)) = decode_escape_body(after1, false) {
        return Some(ch);
    }
    Some('\\')
}

/// Decodes the logical character immediately before the end of `prefix` when it ends with a
/// single- or double-escaped `\u{...}` sequence (whose trailing `}` would otherwise look like
/// punctuation to `Kind::Name`'s word boundary check).
fn prev_decoded_char(prefix: &str) -> Option<char> {
    let last = prefix.chars().next_back()?;
    if last == '}'
        && let Some(body) = prefix.strip_suffix('}')
    {
        let hex_start = body
            .as_bytes()
            .iter()
            .rposition(|b| !b.is_ascii_hexdigit())
            .map_or(0, |idx| idx + 1);
        let hex = &body.as_bytes()[hex_start..];
        if (1..=6).contains(&hex.len())
            && let Some(before_u) = body[..hex_start].strip_suffix("u{")
        {
            let slashes = before_u
                .as_bytes()
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count();
            count_work(slashes);
            if slashes % 4 != 0
                && let Some(ch) = parse_hex_u32(hex).and_then(char::from_u32)
            {
                return Some(ch);
            }
        }
    }
    Some(last)
}

/// Matches one non-backslash character `target` at the start of `rest`: raw, single-escaped, or
/// double-escaped. Because `target != '\\'`, at most one token length can match.
fn match_non_backslash_token(rest: &str, target: char) -> Option<usize> {
    debug_assert_ne!(target, '\\');
    if rest.starts_with(target) {
        return Some(target.len_utf8());
    }
    let after1 = rest.strip_prefix('\\')?;
    if let Some(after2) = after1.strip_prefix('\\') {
        decode_escape_body(after2, true)
            .filter(|&(ch, _)| ch == target)
            .map(|(_, len)| 2 + len)
    } else {
        decode_escape_body(after1, false)
            .filter(|&(ch, _)| ch == target)
            .map(|(_, len)| 1 + len)
    }
}

fn match_literal(chars: &[char], rest: &str) -> Option<usize> {
    let mut offset = 0;
    for &target in chars {
        count_work(1);
        offset += match_non_backslash_token(&rest[offset..], target)?;
    }
    Some(offset)
}

/// Decodes a single- or double-escaped token body (after the leading `\` or `\\`) into its
/// logical scalar and body byte length.
fn decode_escape_body(body: &str, doubled: bool) -> Option<(char, usize)> {
    let bytes = body.as_bytes();
    match *bytes.first()? {
        b'\\' if !doubled => Some(('\\', 1)),
        b'\\' if doubled => match bytes.get(1).copied()? {
            b'\\' => Some(('\\', 2)),
            b'"' => Some(('"', 2)),
            b'\'' => Some(('\'', 2)),
            _ => None,
        },
        b'"' if !doubled => Some(('"', 1)),
        b'\'' => Some(('\'', 1)),
        b'n' => Some(('\n', 1)),
        b'r' => Some(('\r', 1)),
        b't' => Some(('\t', 1)),
        b'0' => Some(('\0', 1)),
        b'b' => Some(('\u{08}', 1)),
        b'f' => Some(('\u{0C}', 1)),
        b'/' => Some(('/', 1)),
        b'u' => {
            if bytes.get(1) == Some(&b'{') {
                decode_rust_unicode_escape(body)
            } else {
                decode_json_unicode_escape(body, doubled)
            }
        }
        b'x' => decode_hex_byte_escape(body),
        _ => None,
    }
}

fn escaped_backslash_body_len(body: &str) -> Option<usize> {
    if body.starts_with('\\') {
        return None;
    }
    decode_escape_body(body, false)
        .filter(|&(ch, _)| ch == '\\')
        .map(|(_, len)| len)
}

fn parse_hex_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(bytes).ok()?, 16).ok()
}

fn decode_rust_unicode_escape(body: &str) -> Option<(char, usize)> {
    let bytes = body.strip_prefix("u{")?.as_bytes();
    let close = bytes.iter().take(7).position(|&b| b == b'}')?;
    if close == 0 || close > 6 {
        return None;
    }
    let ch = char::from_u32(parse_hex_u32(&bytes[..close])?)?;
    Some((ch, 2 + close + 1))
}

fn decode_json_unicode_escape(body: &str, doubled: bool) -> Option<(char, usize)> {
    let rest = body.strip_prefix('u')?.as_bytes();
    let first = parse_hex_u32(rest.get(..4)?)?;
    if let Some(ch) = char::from_u32(first) {
        return Some((ch, 5));
    }
    if !(0xD800..=0xDBFF).contains(&first) {
        return None;
    }
    let sep: &[u8] = if doubled { br"\\u" } else { br"\u" };
    let tail = rest.get(4..)?.strip_prefix(sep)?;
    let second = parse_hex_u32(tail.get(..4)?)?;
    if !(0xDC00..=0xDFFF).contains(&second) {
        return None;
    }
    let scalar = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
    Some((char::from_u32(scalar)?, 5 + sep.len() + 4))
}

fn decode_hex_byte_escape(body: &str) -> Option<(char, usize)> {
    let rest = body.strip_prefix('x')?.as_bytes();
    let ch = char::from_u32(parse_hex_u32(rest.get(..2)?)?)?;
    Some((ch, 3))
}

/// Minimum number of backslash tokens (`\`, `\\`, `\\\\`) needed to consume `r` literal
/// backslash bytes. Valid counts for a fixed `r` form the contiguous interval
/// `[k_min_literal(r), r]` (no internal gaps).
fn k_min_literal(r: usize) -> usize {
    if r == 0 {
        0
    } else if r % 4 == 3 {
        r / 4 + 2
    } else {
        r.div_ceil(4)
    }
}

/// Minimum number of backslash tokens needed to consume `r >= 1` literal backslash bytes
/// followed immediately by an escaped-backslash body (`u{5c}`, `u005c`, or `x5c`), ending the
/// last token as `\body` or `\\body`. Valid counts form the contiguous interval
/// `[k_min_escaped(r), r]`, and `k_min_escaped` is monotonically non-decreasing in `r`.
fn k_min_escaped(r: usize) -> usize {
    debug_assert!(r >= 1);
    if r <= 2 { 1 } else { (r - 3) / 4 + 2 }
}

struct SlashUnit {
    start: usize,
    body_start: usize,
    end: usize,
    p_min: usize,
    p_max: usize,
}

struct SlashChain {
    unit_range: Range<usize>,
    start: usize,
    end: usize,
}

/// Maximal chains of literal backslashes and escaped-backslash tokens (`\u{5c}`, `\u005c`,
/// `\x5c`) in `text`, pre-indexed once per `apply` call so backslash-heavy patterns match in
/// bounded time without repeating quadratic work across starting positions.
struct SlashIndex {
    units: Vec<SlashUnit>,
    chains: Vec<SlashChain>,
}

impl SlashIndex {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut units = Vec::new();
        let mut chains = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            count_work(1);
            if bytes[i] != b'\\' {
                i += 1;
                continue;
            }
            let unit_start = units.len();
            let chain_start = i;
            let mut p_min = 0;
            let mut p_max = 0;
            loop {
                let start = i;
                while i < bytes.len() && bytes[i] == b'\\' {
                    count_work(1);
                    i += 1;
                }
                let body_start = i;
                let run = body_start - start;
                if let Some(body_len) = escaped_backslash_body_len(&text[body_start..]) {
                    let end = body_start + body_len;
                    units.push(SlashUnit {
                        start,
                        body_start,
                        end,
                        p_min,
                        p_max,
                    });
                    p_min += k_min_escaped(run);
                    p_max += run;
                    i = end;
                    if i < bytes.len() && bytes[i] == b'\\' {
                        continue;
                    }
                    // Empty sentinel tail so `end` is a valid target position in the chain.
                    units.push(SlashUnit {
                        start: i,
                        body_start: i,
                        end: i,
                        p_min,
                        p_max,
                    });
                } else {
                    units.push(SlashUnit {
                        start,
                        body_start,
                        end: body_start,
                        p_min,
                        p_max,
                    });
                }
                break;
            }
            let chain_end = units.last().unwrap().end;
            chains.push(SlashChain {
                unit_range: unit_start..units.len(),
                start: chain_start,
                end: chain_end,
            });
        }
        Self { units, chains }
    }
}

#[derive(Clone, Copy)]
struct SlashTarget {
    pos: usize,
    best_end: usize,
    unit: usize,
    a_min: usize,
    a_max: usize,
    mod4_is_3: bool,
}

#[derive(Clone)]
struct SlashMatches {
    spans: Vec<(usize, usize)>,
    cursor: usize,
}

impl SlashMatches {
    fn new(spans: Vec<(usize, usize)>) -> Self {
        Self { spans, cursor: 0 }
    }

    fn len_at(&mut self, index: usize) -> Option<usize> {
        while self.cursor < self.spans.len() && self.spans[self.cursor].0 < index {
            count_work(1);
            self.cursor += 1;
        }
        self.spans
            .get(self.cursor)
            .filter(|&&(start, _)| start == index)
            .map(|&(_, end)| end - index)
    }
}

/// Computes `(start_byte, longest_valid_end_byte)` for every starting offset in `text` where
/// a backslash-containing pattern matches, via one backward pass over `segments`.
fn match_slash_pattern(
    segments: &[Segment],
    text: &str,
    slash_index: &SlashIndex,
    accept_end: impl Fn(usize) -> bool,
) -> Vec<(usize, usize)> {
    let Some((last, prefix)) = segments.split_last() else {
        return Vec::new();
    };
    let mut dp = match last {
        Segment::Literal(chars) => match_last_literal(text, chars, &accept_end),
        Segment::Slashes(k) => {
            step_slashes_backward(slash_index, *k, None, !prefix.is_empty(), &accept_end)
        }
    };
    for (seg_idx, segment) in prefix.iter().enumerate().rev() {
        if dp.is_empty() {
            return Vec::new();
        }
        dp = match segment {
            Segment::Literal(chars) => step_literal_backward(text, chars, &dp),
            Segment::Slashes(k) => {
                step_slashes_backward(slash_index, *k, Some(&dp), seg_idx > 0, &accept_end)
            }
        };
    }
    dp
}

fn match_last_literal(
    text: &str,
    chars: &[char],
    accept_end: &impl Fn(usize) -> bool,
) -> Vec<(usize, usize)> {
    let first = chars[0];
    let min_len: usize = chars.iter().map(|ch| ch.len_utf8()).sum();
    let Some(max_start) = text.len().checked_sub(min_len) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (pos, ch) in text.char_indices() {
        count_work(1);
        if pos > max_start {
            break;
        }
        if (ch == first || ch == '\\')
            && let Some(len) = match_literal(chars, &text[pos..])
            && accept_end(pos + len)
        {
            out.push((pos, pos + len));
        }
    }
    out
}

fn step_literal_backward(
    text: &str,
    chars: &[char],
    next_dp: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    if next_dp.is_empty() {
        return Vec::new();
    }
    let first = chars[0];
    let min_len: usize = chars.iter().map(|ch| ch.len_utf8()).sum();
    // Double-escaped JSON surrogate pair is the longest single-character token (14 bytes).
    let max_len = chars.len().saturating_mul(14);
    let mut out = Vec::new();
    let mut scan_from = 0;
    for &(target_pos, _) in next_dp {
        count_work(1);
        if target_pos < min_len {
            continue;
        }
        let hi = target_pos - min_len;
        let lo = target_pos.saturating_sub(max_len).max(scan_from);
        if lo > hi {
            continue;
        }
        let mut start = lo;
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut pos = start;
        while pos <= hi {
            count_work(1);
            let Some(ch) = text[pos..].chars().next() else {
                break;
            };
            if (ch == first || ch == '\\')
                && let Some(len) = match_literal(chars, &text[pos..])
                && let Ok(idx) = next_dp.binary_search_by_key(&(pos + len), |&(t, _)| t)
            {
                out.push((pos, next_dp[idx].1));
            }
            pos += ch.len_utf8();
        }
        scan_from = pos;
    }
    out
}

fn step_slashes_backward(
    slash_index: &SlashIndex,
    k: usize,
    next_dp: Option<&[(usize, usize)]>,
    starts_only_unit_heads: bool,
    accept_end: &impl Fn(usize) -> bool,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut targets = Vec::new();
    let mut dp_idx = 0;
    for chain in &slash_index.chains {
        let units = &slash_index.units[chain.unit_range.clone()];
        targets.clear();
        match next_dp {
            Some(next_dp) => {
                while dp_idx < next_dp.len() && next_dp[dp_idx].0 < chain.start {
                    count_work(1);
                    dp_idx += 1;
                }
                let mut u_idx = 0;
                while dp_idx < next_dp.len() && next_dp[dp_idx].0 <= chain.end {
                    count_work(1);
                    let (pos, best_end) = next_dp[dp_idx];
                    dp_idx += 1;
                    while u_idx + 1 < units.len() && units[u_idx + 1].start <= pos {
                        count_work(1);
                        u_idx += 1;
                    }
                    if pos >= units[u_idx].start && pos <= units[u_idx].body_start {
                        let r = pos - units[u_idx].start;
                        targets.push(SlashTarget {
                            pos,
                            best_end,
                            unit: u_idx,
                            a_min: units[u_idx].p_min + k_min_literal(r),
                            a_max: units[u_idx].p_max + r,
                            mod4_is_3: r % 4 == 3,
                        });
                    }
                }
            }
            None => {
                for (u_idx, unit) in units.iter().enumerate() {
                    for pos in unit.start..=unit.body_start {
                        count_work(1);
                        if accept_end(pos) {
                            let r = pos - unit.start;
                            targets.push(SlashTarget {
                                pos,
                                best_end: pos,
                                unit: u_idx,
                                a_min: unit.p_min + k_min_literal(r),
                                a_max: unit.p_max + r,
                                mod4_is_3: r % 4 == 3,
                            });
                        }
                    }
                }
            }
        }
        sweep_slashes_chain(units, k, &targets, starts_only_unit_heads, &mut out);
    }
    out
}

fn push_max_deque(deque: &mut VecDeque<(usize, usize)>, idx: usize, end: usize) {
    while deque.back().is_some_and(|&(_, prev)| prev <= end) {
        count_work(1);
        deque.pop_back();
    }
    count_work(1);
    deque.push_back((idx, end));
}

fn sweep_slashes_chain(
    units: &[SlashUnit],
    k: usize,
    targets: &[SlashTarget],
    starts_only_unit_heads: bool,
    out: &mut Vec<(usize, usize)>,
) {
    if targets.is_empty() {
        return;
    }
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for (u_idx, unit) in units.iter().enumerate() {
        if unit.start == unit.body_start {
            continue;
        }
        if starts_only_unit_heads {
            count_work(1);
            starts.push((unit.start, u_idx));
        } else {
            for pos in unit.start..unit.body_start {
                count_work(1);
                starts.push((pos, u_idx));
            }
        }
    }
    if starts.is_empty() {
        return;
    }
    let mut best: Vec<Option<usize>> = vec![None; starts.len()];
    let mut deque: VecDeque<(usize, usize)> = VecDeque::new();

    // Sweep 1: same-unit literal backslash transitions (byte deltas in `[k, 4k] \ {4k - 1}`).
    let mut s_base = 0;
    let mut t_base = 0;
    while s_base < starts.len() && t_base < targets.len() {
        let s_unit = starts[s_base].1;
        let t_unit = targets[t_base].unit;
        if s_unit < t_unit {
            while s_base < starts.len() && starts[s_base].1 == s_unit {
                count_work(1);
                s_base += 1;
            }
            continue;
        }
        if t_unit < s_unit {
            while t_base < targets.len() && targets[t_base].unit == t_unit {
                count_work(1);
                t_base += 1;
            }
            continue;
        }
        let s_end = s_base
            + starts[s_base..]
                .iter()
                .take_while(|&&(_, u)| u == s_unit)
                .count();
        let t_end = t_base
            + targets[t_base..]
                .iter()
                .take_while(|t| t.unit == t_unit)
                .count();
        let unit_targets = &targets[t_base..t_end];
        deque.clear();
        let mut lo = 0;
        let mut hi = 0;
        let mut exact = 0;
        let span_hi = k.saturating_mul(4).saturating_sub(2);
        let span_exact = k.saturating_mul(4);
        for (slot, &(pos, _)) in best[s_base..s_end].iter_mut().zip(&starts[s_base..s_end]) {
            count_work(1);
            let win_lo = pos.saturating_add(k);
            let win_hi = pos.saturating_add(span_hi);
            let exact_pos = pos.saturating_add(span_exact);
            while hi < unit_targets.len() && unit_targets[hi].pos <= win_hi {
                push_max_deque(&mut deque, hi, unit_targets[hi].best_end);
                hi += 1;
            }
            while lo < hi && unit_targets[lo].pos < win_lo {
                count_work(1);
                lo += 1;
            }
            while deque.front().is_some_and(|&(idx, _)| idx < lo) {
                count_work(1);
                deque.pop_front();
            }
            while exact < unit_targets.len() && unit_targets[exact].pos < exact_pos {
                count_work(1);
                exact += 1;
            }
            let mut candidate = deque.front().map(|&(_, e)| e);
            if exact < unit_targets.len() && unit_targets[exact].pos == exact_pos {
                let end = unit_targets[exact].best_end;
                candidate = Some(candidate.map_or(end, |prev| prev.max(end)));
            }
            if let Some(end) = candidate {
                *slot = Some(slot.map_or(end, |prev| prev.max(end)));
            }
        }
        s_base = s_end;
        t_base = t_end;
    }

    // Sweep 2: cross-unit transitions (`a < b`). Partitioning targets by `(r % 4 == 3)` makes
    // `unit`, `a_min`, and `a_max` simultaneously monotonically non-decreasing in each slice, so
    // the reachable targets for each start form a single right-moving sliding window.
    if units.len() > 1 {
        let mut sub = Vec::new();
        for want_mod4_3 in [false, true] {
            count_work(targets.len());
            sub.clear();
            sub.extend(
                targets
                    .iter()
                    .copied()
                    .filter(|t| t.mod4_is_3 == want_mod4_3),
            );
            if sub.is_empty() {
                continue;
            }
            deque.clear();
            let mut lo = 0;
            let mut hi = 0;
            for (slot, &(pos, a)) in best.iter_mut().zip(&starts) {
                count_work(1);
                if a + 1 >= units.len() {
                    break;
                }
                let r_first = units[a].body_start - pos;
                let b_min = units[a + 1].p_min - k_min_escaped(r_first);
                let b_max = units[a + 1].p_max - r_first;
                let max_a_min = b_min.saturating_add(k);
                let min_a_max = b_max.saturating_add(k);
                while hi < sub.len() && sub[hi].a_min <= max_a_min {
                    push_max_deque(&mut deque, hi, sub[hi].best_end);
                    hi += 1;
                }
                while lo < hi && (sub[lo].unit <= a || sub[lo].a_max < min_a_max) {
                    count_work(1);
                    lo += 1;
                }
                while deque.front().is_some_and(|&(idx, _)| idx < lo) {
                    count_work(1);
                    deque.pop_front();
                }
                if let Some(&(_, end)) = deque.front() {
                    *slot = Some(slot.map_or(end, |prev| prev.max(end)));
                }
            }
        }
    }

    count_work(starts.len());
    for ((pos, _), end) in starts.into_iter().zip(best) {
        if let Some(end) = end {
            out.push((pos, end));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Redaction, assemble, install_route, linux_distribution, macos_name, pretty_name};
    use crate::doctor::compositor;
    use std::{ffi::OsString, path::PathBuf};

    fn redaction(environment: &[(&str, &str)], canonical_home: Option<&str>) -> Redaction {
        Redaction::new(
            environment
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value))),
            canonical_home.map(PathBuf::from),
        )
    }

    #[test]
    fn home_becomes_tilde_only_at_path_boundaries_raw_and_canonical() {
        let home = redaction(&[("HOME", "/home/al")], Some("/data/home/al"));
        assert_eq!(
            home.apply("/home/al/x /home/alice/y \"/home/al\" /data/home/al/z /home/al"),
            "~/x /home/alice/y \"~\" ~/z ~"
        );
        // Whatever cannot continue the home's last name ends it.
        assert_eq!(
            home.apply("/home/al. </home/al> a=/home/al|b /home/al_x /home/al-x"),
            "~. <~> a=~|b /home/al_x /home/al-x"
        );
        // A home of `/` would make every absolute path `~`.
        let root = redaction(&[("HOME", "/")], Some("/"));
        assert_eq!(root.apply("/etc/os-release"), "/etc/os-release");
    }

    #[test]
    fn long_environment_values_go_longest_first_except_the_shown_ones() {
        let redaction = redaction(
            &[
                ("HOME", "/home/al"),
                ("TOKEN", "s3cr3t-value"),
                ("LONGER", "s3cr3t-value-123"),
                ("SHORT", "abc1234"),
                ("XDG_CURRENT_DESKTOP", "COSMIC-desktop"),
                ("XDG_SESSION_TYPE", "wayland-x"),
                ("EON_TERMINAL", "s1.t2.run-abcdef"),
                ("PWD", "/home/al/project"),
            ],
            None,
        );
        assert_eq!(
            redaction.apply(
                "s3cr3t-value-123 s3cr3t-value abc1234 COSMIC-desktop wayland-x \
                 s1.t2.run-abcdef /home/al/project/a /home/al/b"
            ),
            "[redacted] [redacted] abc1234 COSMIC-desktop wayland-x s1.t2.run-abcdef \
             [redacted]/a ~/b"
        );
    }

    #[test]
    fn xdg_base_directories_show_as_home_paths_other_home_paths_stay_redacted() {
        let inside = redaction(
            &[
                ("HOME", "/home/al"),
                ("XDG_STATE_HOME", "/home/al/.local/state"),
                ("XDG_CONFIG_HOME", "/home/al/.config"),
                ("PWD", "/home/al/client-project"),
                ("API_KEY", "/opt/keys/s3cr3t-key"),
            ],
            None,
        );
        assert_eq!(
            inside.apply(
                "/home/al/.local/state/eon/logs/g1.log /home/al/.config/eon/config.toml \
                 /home/al/client-project/x /opt/keys/s3cr3t-key"
            ),
            "~/.local/state/eon/logs/g1.log ~/.config/eon/config.toml [redacted]/x [redacted]"
        );
        // Outside the home as the home rule matches it, or leaving it, an XDG base directory
        // is a value like any other.
        let outside = redaction(
            &[
                ("HOME", "/home/al"),
                ("XDG_STATE_HOME", "/srv/private-client/state"),
                ("XDG_DATA_HOME", "/home/al/../../srv/data"),
                ("XDG_CACHE_HOME", "/home/alice/.cache"),
            ],
            None,
        );
        assert_eq!(
            outside.apply(
                "/srv/private-client/state/eon /home/al/../../srv/data /home/alice/.cache/x"
            ),
            "[redacted]/eon [redacted] [redacted]/x"
        );
        let unnormalized = redaction(
            &[
                ("HOME", "/srv/private-client/al"),
                ("XDG_STATE_HOME", "/srv//private-client/al/state"),
            ],
            None,
        );
        assert_eq!(
            unnormalized.apply("/srv//private-client/al/state/eon/logs/g1.log"),
            "[redacted]/eon/logs/g1.log"
        );
    }

    #[test]
    fn a_shown_value_stays_where_another_variable_holds_it_too() {
        let redaction = redaction(
            &[
                ("XDG_SESSION_DESKTOP", "Hyprland"),
                ("XDG_CURRENT_DESKTOP", "Hyprland"),
                ("XDG_SESSION_TYPE", "wayland"),
                ("DESKTOP_SESSION", "Hyprland"),
            ],
            None,
        );
        assert_eq!(redaction.apply("Hyprland (wayland)"), "Hyprland (wayland)");
    }

    #[test]
    fn escaped_values_and_homes_go_as_json_and_warnings_print_them() {
        let redaction = redaction(
            &[
                ("HOME", "/home/a\"l"),
                ("SECRET_TOKEN", "abc\"defgh"),
                ("TAB_TOKEN", "tab\there-1"),
            ],
            None,
        );
        assert_eq!(
            redaction.apply(
                r#"{"name":"abc\"defgh"} unknown bar.widgets name "abc\"defgh" tab\there-1 "/home/a\"l/x" /home/a"l/y"#
            ),
            r#"{"name":"[redacted]"} unknown bar.widgets name "[redacted]" [redacted] "~/x" ~/y"#
        );
    }

    #[test]
    fn retained_log_redaction_matches_escape_tokens_across_rust_versions_and_double_escaping() {
        // Unicode 18.0 scalar U+0558 (escaped as `\u{558}` by Rust 1.96.1, raw in Rust 1.100.0)
        // at the end of HOME, with path boundaries preserved.
        let unicode18_home = "/home/user\u{0558}";
        let unicode18 = redaction(&[("HOME", unicode18_home)], None);
        let u18_log = concat!(
            r#"eon: ignoring unknown setting foo in /home/user\u{558}/.config/eon/config.toml"#,
            "\n",
            "eon: ignoring unknown setting bar in /home/user\u{0558}/.config/eon/config.toml\n",
            r#"{"error":"bad path /home/user\\u{558}/x"}"#,
            "\n",
            r#"other user /home/user\u{558}2/x and /home/user"#,
            "\u{0558}_x\n",
        );
        assert_eq!(
            unicode18.apply(u18_log),
            concat!(
                "eon: ignoring unknown setting foo in ~/.config/eon/config.toml\n",
                "eon: ignoring unknown setting bar in ~/.config/eon/config.toml\n",
                r#"{"error":"bad path ~/x"}"#,
                "\n",
                r#"other user /home/user\u{558}2/x and /home/user"#,
                "\u{0558}_x\n",
            )
        );

        // Combining accent (raw and escaped), escaped apostrophe, quote, control escapes, and
        // doubled-escaped (JSON-in-Rust-debug) values.
        let home = "/home/O'Brien/Cafe\u{0301}";
        let retained = redaction(
            &[
                ("HOME", home),
                ("QUOTED_HOME", "/home/a\"b/dir"),
                ("LEADING_MARK", "\u{0301}secret'value"),
                ("HANGUL_FILLER", "sec\u{115F}ret'val"),
                ("ESCAPED_SECRET", "sec\"ret\x1b\n\\1"),
                ("EMOJI_SECRET", "sec\u{1F600}ret1"),
                ("BACKSLASHES", r"a\\b\\c\\d"),
            ],
            Some("/data/a\"b"),
        );
        let json_payload = serde_json::json!({
            "cfg": "/home/a\"b/dir",
            "dir": "/data/a\"b/work",
            "tok": "sec\"ret\x1b\n\\1",
        })
        .to_string();
        let json_in_rust_debug = format!("{json_payload:?}");
        assert_eq!(
            retained.apply(&json_in_rust_debug),
            r#""{\"cfg\":\"[redacted]\",\"dir\":\"~/work\",\"tok\":\"[redacted]\"}""#,
        );

        let retained_log = concat!(
            "eon: ignoring unknown setting foo in /home/O\\'Brien/Cafe\u{0301}/.config/eon/config.toml\n",
            r#"eon: ignoring unknown setting bar in /home/O\'Brien/Cafe\u{301}/.config/eon/config.toml"#,
            "\n",
            r#"cannot resolve "/home/O'Brien/Cafe\u{301}/project": permission denied"#,
            "\n",
            r#"{"err":"/home/O\\'Brien/Cafe\\u{301}/x"}"#,
            "\n",
            "/home/O'Brien/Cafe\u{0301}/.local/state/eon/logs/g1.log\n",
            r#"raw /data/a"b/y and 1x "/data/a\"b/z""#,
            "\n",
            r#"ctrl sec\"ret\x1b\n\\1 sec\"ret\u001b\n\\1 sec\"ret\u{1b}\n\\1"#,
            "\n",
            "warn \\u{301}secret\\'value sec\u{115F}ret\\'val sec\\u{115f}ret\\'val sec\\u115fret\\\\'val\n",
            r#"emoji sec\ud83d\ude00ret1 sec\\ud83d\\ude00ret1 sec\u{1f600}ret1"#,
            "\n",
            r#"slashes a\\b\\c\\d a\\\\b\\\\c\\\\d a\\\\\\\\b\\\\\\\\c\\\\\\\\d"#,
            "\n",
        );
        let redacted = retained.apply(retained_log);
        assert_eq!(
            redacted,
            concat!(
                "eon: ignoring unknown setting foo in ~/.config/eon/config.toml\n",
                "eon: ignoring unknown setting bar in ~/.config/eon/config.toml\n",
                "cannot resolve \"~/project\": permission denied\n",
                r#"{"err":"~/x"}"#,
                "\n",
                "~/.local/state/eon/logs/g1.log\n",
                "raw ~/y and 1x \"~/z\"\n",
                "ctrl [redacted] [redacted] [redacted]\n",
                "warn [redacted] [redacted] [redacted] [redacted]\n",
                "emoji [redacted] [redacted] [redacted]\n",
                "slashes [redacted] [redacted] [redacted]\n",
            )
        );
        assert!(!redacted.contains("Brien"), "{redacted}");
        assert!(!redacted.contains("Cafe"), "{redacted}");

        // Backslash-heavy inputs stay polynomial (no exponential backtracking).
        let many_slashes = "\\".repeat(32);
        let slash_redaction = redaction(&[("SLASHES", &many_slashes)], None);
        let mismatch = format!("{}x", "\\".repeat(31));
        assert_eq!(slash_redaction.apply(&mismatch), mismatch);
        assert_eq!(slash_redaction.apply(&"\\".repeat(64)), "[redacted]");
    }

    #[test]
    fn escaped_sibling_paths_and_words_respect_decoded_boundaries() {
        assert!('\u{0558}'.is_alphanumeric());
        let unicode18 = redaction(&[("HOME", "/home/user\u{0558}")], None);
        // Escaped sibling continuations decode to alphanumeric / `_` / `-`, so HOME must stay
        // untouched instead of treating the next escape's leading `\` as path punctuation.
        for untouched in [
            "/home/user\u{0558}\u{0558}/x",
            r"/home/user\u{558}\u{558}/x",
            r#"{"err":"/home/user\\u{558}\\u{558}/x"}"#,
            r"/home/user\u0558\u0558/x",
            r"/home/user\\u0558\\u0558/x",
            "/home/user\u{0558}\\u{558}/x",
            concat!(r"/home/user\u{558}", "\u{0558}/x"),
            r"/home/user\u{558}\u{5f}x",
            r"/home/user\u{558}\x2dx",
            r"/home/user\u{558}\u0032/x",
            r"/home/user\u{558}\\x61/x",
            r"/home/user\u{558}\ud800\udc00/x",
            r"/home/user\u{558}\\ud800\\udc00/x",
        ] {
            assert_eq!(unicode18.apply(untouched), untouched, "{untouched}");
        }

        // Escaped punctuation, separators, controls, and non-alphanumeric emoji still end HOME.
        for (input, expected) in [
            (r"/home/user\u{558}\/x", r"~\/x"),
            (r#""/home/user\u{558}\""#, r#""~\""#),
            (r"/home/user\u{558}\u{2f}x", r"~\u{2f}x"),
            (r"/home/user\u{558}\n", r"~\n"),
            (r#"/home/user\u{558}\\""#, r#"~\\""#),
            (r"/home/user\u{558}\ud83d\ude00", r"~\ud83d\ude00"),
            (r#"{"p":"/home/user\\u{558}"}"#, r#"{"p":"~"}"#),
            (r#""{\"p\":\"/home/user\\u{558}\"}""#, r#""{\"p\":\"~\"}""#),
        ] {
            assert_eq!(unicode18.apply(input), expected, "{input}");
        }

        // Whole-word user names also check decoded characters on both boundaries.
        let names = redaction(&[("USER", "user\u{0558}"), ("LOGNAME", "al")], None);
        assert_eq!(
            names.apply(r"user\u{558}\u{558} \u{558}al \\u{558}al user\u{558}\n \u{20}al"),
            r"user\u{558}\u{558} \u{558}al \\u{558}al [user]\n \u{20}[user]"
        );
    }

    #[test]
    fn backslash_matcher_matches_nfa_oracle_and_stays_bounded_on_large_near_matches() {
        use super::{Kind, MATCH_WORK, decode_escape_body, next_decoded_char, prev_decoded_char};

        // Reference NFA matcher (direct character-by-character token expansion) used as a
        // correctness oracle for small adversarial backslash/escape combinations.
        fn oracle_match_tokens(rest: &str, target: char, mut push: impl FnMut(usize)) {
            if rest.starts_with(target) {
                push(target.len_utf8());
            }
            let Some(after1) = rest.strip_prefix('\\') else {
                return;
            };
            if let Some((ch, len)) = decode_escape_body(after1, false)
                && ch == target
            {
                push(1 + len);
            }
            if let Some(after2) = after1.strip_prefix('\\')
                && let Some((ch, len)) = decode_escape_body(after2, true)
                && ch == target
            {
                push(2 + len);
            }
        }
        fn oracle_apply(patterns: &[(Kind, &str)], text: &str) -> String {
            let word = |ch: char| ch.is_alphanumeric();
            let path_end = |ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-'));
            let mut out = String::with_capacity(text.len());
            let mut index = 0;
            while let Some(ch) = text[index..].chars().next() {
                let rest = &text[index..];
                let before = prev_decoded_char(&text[..index]);
                let mut best: Option<(usize, Kind)> = None;
                for &(kind, pat) in patterns {
                    if kind == Kind::Name && before.is_some_and(word) {
                        continue;
                    }
                    let mut states = vec![0usize];
                    let mut next = Vec::new();
                    for target in pat.chars() {
                        next.clear();
                        for &offset in &states {
                            oracle_match_tokens(&rest[offset..], target, |len| {
                                next.push(offset + len);
                            });
                        }
                        next.sort_unstable();
                        next.dedup();
                        std::mem::swap(&mut states, &mut next);
                        if states.is_empty() {
                            break;
                        }
                    }
                    let matched = states.into_iter().rev().find(|&end| {
                        let after = next_decoded_char(&rest[end..]);
                        match kind {
                            Kind::Home => after.is_none_or(path_end),
                            Kind::Value => true,
                            Kind::Name => !after.is_some_and(word),
                        }
                    });
                    if let Some(len) = matched
                        && best.is_none_or(|(best_len, _)| len > best_len)
                    {
                        best = Some((len, kind));
                    }
                }
                match best {
                    Some((len, kind)) => {
                        out.push_str(match kind {
                            Kind::Home => "~",
                            Kind::Value => "[redacted]",
                            Kind::Name => "[user]",
                        });
                        index += len;
                    }
                    None => {
                        out.push(ch);
                        index += ch.len_utf8();
                    }
                }
            }
            out
        }

        let oracle_env = [
            ("HOME", r"/home/a\"),
            ("USER", r"a\b"),
            ("S1", r"\\x5c\\x"),
            ("S2", r"\\\\a\\\\"),
            ("S3", r"\a\b\c\d"),
            ("S4", r#"\"\"\"\""#),
        ];
        let oracle_patterns = [
            (Kind::Home, r"/home/a\"),
            (Kind::Name, r"a\b"),
            (Kind::Value, r"\\x5c\\x"),
            (Kind::Value, r"\\\\a\\\\"),
            (Kind::Value, r"\a\b\c\d"),
            (Kind::Value, r#"\"\"\"\""#),
        ];
        let actual_redaction = redaction(&oracle_env, None);
        for sample in [
            r"/home/a\\/x /home/a\\u{558}/x /home/a\\\u{558}/x /home/a\u{5c}/x /home/a\\u{5c}/x",
            r"a\b a\\b a\\\\b a\x5cb a\\x5cb \x61\x5c\x62 \\x61\\x5c\\x62 xa\b a\bx",
            r"\\x5c\\x \\\x5c\\x \\\\x5c\\\\x \\x5c\\x5c\\x \u{5c}\x5c\x5c\x5cx",
            r"\\\\a\\\\ \\\\\\a\\\\\\ \\\\\\\\a\\\\\\\\ \\u{5c}\x5c\u005ca\u{5c}\\x5c\\\\",
            r"\a\b\c\d \\a\\b\\c\\d \\\\a\\\\b\\\\c\\\\d \x5ca\u{5c}b\u005cc\\x5cd \x5c\x61\x5c\x62\x5c\x63\x5c\x64 \\x5c\\x61\\x5c\\x62\\x5c\\x63\\x5c\\x64",
            r#"\"\"\"\" \\\"\\\"\\\"\\\" \\\\\"\\\\\"\\\\\"\\\\\""#,
            r"\\x5c\\x5c\\x5c\\x \\u{5c}\\u{5c}\\u{5c}\\u{5c} \\\\\\\\",
        ] {
            assert_eq!(
                actual_redaction.apply(sample),
                oracle_apply(&oracle_patterns, sample),
                "mismatch on {sample:?}"
            );
        }

        // 8 KiB log lines vs 4 KiB near-matches (literal backslashes and escaped-backslash chains)
        // must stay bounded while preserving exact longest-match redaction.
        let near_slashes = format!("{}x", "\\".repeat(4095));
        let exact_slashes = "\\".repeat(4096);
        let near_escaped = format!("{}xy", "\\".repeat(2000));
        let multi_seg = format!("{}z", r"\\x5c".repeat(800));
        let bounded = redaction(
            &[
                ("NEAR_SLASHES", &near_slashes),
                ("NEAR_ESCAPED", &near_escaped),
                ("MULTI_SEG", &multi_seg),
            ],
            None,
        );
        let exact_only = redaction(&[("EXACT_SLASHES", &exact_slashes)], None);

        let line_8k_slashes = "\\".repeat(8192);
        let line_8k_match = format!("{}x", "\\".repeat(8191));
        let line_8k_x5c = r"\\x5c".repeat(1600);
        let line_8k_x5c_match = format!("{}z", r"\\x5c".repeat(1600));
        let line_8k_u5c = r"\\u{5c}".repeat(1200);

        let apply_measured = |redaction: &Redaction, input: &str| {
            MATCH_WORK.with(|work| work.set(0));
            let output = redaction.apply(input);
            let work = MATCH_WORK.with(|work| work.get());
            assert!(work > 0, "work counter must record matcher steps");
            (output, work)
        };

        // Single/two-segment active patterns stay strictly O(n) in input length.
        for (name, redaction, input, expected) in [
            (
                "8k_slashes_near",
                &bounded,
                line_8k_slashes.as_str(),
                line_8k_slashes.as_str(),
            ),
            (
                "8k_slashes_match",
                &bounded,
                line_8k_match.as_str(),
                "[redacted]",
            ),
            (
                "8k_slashes_exact",
                &exact_only,
                line_8k_slashes.as_str(),
                "[redacted]",
            ),
            (
                "8k_x5c_near",
                &bounded,
                line_8k_x5c.as_str(),
                line_8k_x5c.as_str(),
            ),
            (
                "8k_u5c_near",
                &bounded,
                line_8k_u5c.as_str(),
                line_8k_u5c.as_str(),
            ),
        ] {
            let (output, work) = apply_measured(redaction, input);
            assert_eq!(output, expected, "{name}");
            let limit = input.len() * 32;
            assert!(
                work <= limit,
                "{name}: expected O(n) matcher work <= {limit}, got {work} for {} bytes",
                input.len()
            );
        }

        // Multi-segment pattern (1600 segments) stays strictly O(segments * n), not exponential.
        let multi_seg_segments = bounded
            .patterns
            .iter()
            .map(|pattern| pattern.segments.len())
            .max()
            .unwrap();
        let (output, work) = apply_measured(&bounded, &line_8k_x5c_match);
        assert_eq!(output, "[redacted]", "8k_x5c_match");
        let multi_seg_limit = multi_seg_segments * line_8k_x5c_match.len() * 2;
        assert!(
            work <= multi_seg_limit,
            "8k_x5c_match: expected O(segments * n) matcher work <= {multi_seg_limit}, got {work} for {multi_seg_segments} segments and {} bytes",
            line_8k_x5c_match.len()
        );
    }

    #[test]
    fn user_names_go_as_whole_words_whatever_their_length() {
        let short = redaction(&[("USER", "al"), ("LOGNAME", "bo")], None);
        assert_eq!(
            short.apply("al@host /tmp/al-x/ally bo, alone al_b"),
            "[user]@host /tmp/[user]-x/ally [user], alone [user]_b"
        );
        let hyphenated = redaction(&[("USER", "lucca")], None);
        assert_eq!(
            hyphenated.apply("lucca-desktop luccas"),
            "[user]-desktop luccas"
        );
        let long = redaction(&[("USER", "longuser"), ("LOGNAME", "O'Brien")], None);
        assert_eq!(
            long.apply(r"longuser-host xlonguser2 O\'Brien O\\'Brien"),
            "[user]-host x[redacted]2 [user] [user]"
        );
    }

    #[test]
    fn a_missing_os_release_reads_as_a_reason_not_a_failure() {
        let directory = std::env::temp_dir().join(format!("eon-report-{}", std::process::id()));
        let (absent, empty) = (directory.join("absent"), directory.join("empty"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&empty, "NAME=Arch\n").unwrap();
        let (absent, empty) = (absent.to_str().unwrap(), empty.to_str().unwrap());
        assert_eq!(
            linux_distribution([absent, absent]),
            Err(format!("neither {absent} nor {absent} is readable"))
        );
        assert_eq!(
            linux_distribution([absent, empty]),
            Err(format!("{empty} has no PRETTY_NAME"))
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn distribution_names_come_from_os_release_and_sw_vers() {
        assert_eq!(
            pretty_name("NAME=Fedora\nPRETTY_NAME=\"Fedora Linux 43\"\n").as_deref(),
            Some("Fedora Linux 43")
        );
        assert_eq!(
            pretty_name("PRETTY_NAME='Pop!_OS 24.04'").as_deref(),
            Some("Pop!_OS 24.04")
        );
        assert_eq!(pretty_name("PRETTY_NAME=NixOS").as_deref(), Some("NixOS"));
        assert_eq!(pretty_name("NAME=Arch\nPRETTY_NAME=\"\"\n"), None);
        assert_eq!(pretty_name("NAME=Arch\n"), None);
        assert_eq!(macos_name(b"26.1\n").as_deref(), Some("macOS 26.1"));
        assert_eq!(macos_name(b"\n"), None);
        assert_eq!(compositor(true, Some("sway"), None), "macOS");
        assert_eq!(
            compositor(false, Some("COSMIC"), Some("wayland")),
            "COSMIC (wayland)"
        );
        assert_eq!(compositor(false, Some("sway"), Some("")), "sway");
        assert_eq!(
            compositor(false, None, Some("wayland")),
            "unknown (XDG_CURRENT_DESKTOP is unset)"
        );
    }

    #[test]
    fn install_kinds_map_to_the_template_options() {
        for (kind, option) in [
            ("nix-profile", "Nix profile (nix profile add)"),
            (
                "externally-managed",
                "Flake input (NixOS, Home Manager or another flake)",
            ),
            ("eon-install", "Eon installer (eon-install)"),
            ("unmanaged", "Other"),
            ("unknown", "Other"),
        ] {
            assert_eq!(install_route(kind), option, "{kind}");
        }
    }

    #[test]
    fn an_unknown_section_keeps_only_the_first_line_of_its_reason() {
        let facts = eon_runtime::ReportFacts {
            versions: Err("no versions".into()),
            window: Ok(None),
            log: Err("no log\nsecond".into()),
        };
        let report = assemble(
            &facts,
            Ok("unmanaged"),
            Err("one\ntwo".into()),
            "sway",
            &Err("cannot parse config.toml: TOML parse error at line 2\n  |\n2 | command = \"secret\"\n".into()),
            "",
            &redaction(&[], None),
        );
        assert!(report.contains(
            "### Configuration warnings\nunknown (cannot parse config.toml: TOML parse error at line 2)\n"
        ));
        assert!(report.contains("### Distribution\nunknown (one)\n"));
        assert!(report.ends_with("### Logs\nunknown (no log)\n"));
        assert!(!report.contains("secret"));
    }

    #[test]
    fn a_reason_is_redacted_whole_before_its_first_line_is_kept() {
        let facts = eon_runtime::ReportFacts {
            versions: Ok("eon 0.1.0\n".into()),
            window: Ok(None),
            log: Err("no log".into()),
        };
        let report = assemble(
            &facts,
            Err("cannot read profile: SECRET_TOKEN=abcdefgh\nijklmnop"),
            Ok("NixOS".into()),
            "sway",
            &Ok(Vec::new()),
            "WARN config — abcdefgh\nijklmnop\n",
            &redaction(&[("SECRET_TOKEN", "abcdefgh\nijklmnop")], None),
        );
        assert!(
            report.contains(
                "### Install route\nunknown (cannot read profile: SECRET_TOKEN=[redacted])\n"
            ),
            "{report}"
        );
        assert!(!report.contains("abcdefgh"), "{report}");
        assert!(!report.contains("ijklmnop"), "{report}");
        assert!(
            report.contains("### Doctor\n```text\nWARN config — [redacted]\n```"),
            "{report}"
        );
    }
}
