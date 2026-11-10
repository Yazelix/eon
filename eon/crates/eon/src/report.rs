//! `eon report` (EON-C1): one Markdown bug report for the issue template, printed to stdout.
//! It gathers what the template asks for without starting an Eon component, checking the
//! network, or creating, changing or removing configuration or runtime state; it may run
//! `nix profile list` and, on macOS, `sw_vers`, each under a time limit. One redactor runs over
//! the whole report so that a path or value from any section is covered.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

use crate::update;

/// How long `sw_vers` may take: a fixed safety budget for a local read.
const SW_VERS_LIMIT: Duration = Duration::from_secs(5);
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
    let facts = eon_runtime::report_facts(inputs);
    let install = update::install_kind(&arguments[0], invoked);
    let macos = cfg!(target_os = "macos");
    let distribution = if macos {
        macos_version()
    } else {
        linux_distribution(OS_RELEASE)
    };
    let compositor = compositor(
        macos,
        env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        env::var("XDG_SESSION_TYPE").ok().as_deref(),
    );
    let report = assemble(
        &facts,
        install,
        distribution,
        &compositor,
        &Redaction::of_this_process(),
    );
    match io::stdout().lock().write_all(report.as_bytes()) {
        Ok(()) => 0,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(error) => {
            eprintln!("eon: cannot write stdout: {error}");
            1
        }
    }
}

/// The report, with `redaction` over all of it.
fn assemble(
    facts: &eon_runtime::ReportFacts,
    install: Result<&str, String>,
    distribution: Result<String, String>,
    compositor: &str,
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
        Err(reason) => unknown(&reason),
    };
    let distribution = distribution.unwrap_or_else(|reason| unknown(&reason));
    let warnings = match &facts.config_warnings {
        Ok(warnings) if warnings.is_empty() => "none".into(),
        Ok(warnings) => fenced("text", &warnings.join("\n")),
        Err(reason) => unknown(reason),
    };
    let log = match &facts.log {
        Ok((path, tail)) => fenced("text", &format!("{}\n{tail}", path.display())),
        Err(reason) => unknown(reason),
    };
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
    let output = update::bounded(command, SW_VERS_LIMIT).map_err(|error| {
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

/// The desktop and its session type, as the session names them.
fn compositor(macos: bool, desktop: Option<&str>, session: Option<&str>) -> String {
    if macos {
        return "macOS".into();
    }
    match (desktop.filter(|desktop| !desktop.is_empty()), session) {
        (Some(desktop), Some(session)) if !session.is_empty() => format!("{desktop} ({session})"),
        (Some(desktop), _) => desktop.into(),
        (None, _) => "unknown (XDG_CURRENT_DESKTOP is unset)".into(),
    }
}

#[derive(Clone, Copy)]
enum Kind {
    /// `HOME`, raw or canonical, at a path boundary: `~`.
    Home,
    /// Any other environment value of `SHORTEST_SECRET` bytes or more, anywhere.
    Value,
    /// `USER` or `LOGNAME`, as a whole word.
    Name,
}

/// `value` as the report can print it: raw, and as JSON and Rust string escaping write it
/// (window JSON, configuration warnings), each form once.
fn printed_forms(value: &str) -> Vec<String> {
    let mut json = String::new();
    for ch in value.chars() {
        match ch {
            '"' => json.push_str(r#"\""#),
            '\\' => json.push_str(r"\\"),
            '\n' => json.push_str(r"\n"),
            '\r' => json.push_str(r"\r"),
            '\t' => json.push_str(r"\t"),
            ch if ch.is_control() => json.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => json.push(ch),
        }
    }
    let debug = format!("{value:?}");
    let debug = debug[1..debug.len() - 1].to_owned();
    let mut forms = vec![value.to_owned()];
    for form in [json, value.escape_debug().to_string(), debug] {
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    forms
}

/// What the report removes, longest first so that a value containing another goes whole.
struct Redaction {
    patterns: Vec<(String, Kind)>,
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
        let mut found = Vec::new();
        for (name, value) in &environment {
            if name == "HOME" || value.is_empty() || shown_values.contains(&value.as_str()) {
                continue;
            }
            if matches!(name.as_str(), "USER" | "LOGNAME") {
                found.push((value.as_str(), Kind::Name));
            }
            if value.len() >= SHORTEST_SECRET {
                found.push((value.as_str(), Kind::Value));
            }
        }
        let mut patterns: Vec<(String, Kind)> = homes
            .iter()
            .map(|home| (home.as_str(), Kind::Home))
            .chain(found)
            .flat_map(|(value, kind)| {
                printed_forms(value)
                    .into_iter()
                    .map(move |form| (form, kind))
            })
            .collect();
        // Stable: of equal length, the home goes first.
        patterns.sort_by_key(|(pattern, _)| std::cmp::Reverse(pattern.len()));
        Self { patterns }
    }

    fn apply(&self, text: &str) -> String {
        // A user name ends where letters and digits do: `lucca-desktop` holds `lucca`.
        let word = |ch: char| ch.is_alphanumeric();
        // A home ends at whatever cannot continue its last path segment's name.
        let path_end = |ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-'));
        let mut result = String::with_capacity(text.len());
        let mut index = 0;
        while let Some(ch) = text[index..].chars().next() {
            let rest = &text[index..];
            let before = text[..index].chars().next_back();
            let found = self.patterns.iter().find(|(pattern, kind)| {
                let Some(after) = rest.strip_prefix(pattern.as_str()) else {
                    return false;
                };
                let after = after.chars().next();
                match kind {
                    Kind::Home => after.is_none_or(path_end),
                    Kind::Value => true,
                    Kind::Name => !before.is_some_and(word) && !after.is_some_and(word),
                }
            });
            match found {
                Some((pattern, kind)) => {
                    result.push_str(match kind {
                        Kind::Home => "~",
                        Kind::Value => "[redacted]",
                        Kind::Name => "[user]",
                    });
                    index += pattern.len();
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

#[cfg(test)]
mod tests {
    use super::{
        Redaction, assemble, compositor, install_route, linux_distribution, macos_name, pretty_name,
    };
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
            config_warnings: Err(
                "cannot parse config.toml: TOML parse error at line 2\n  |\n2 | command = \"secret\"\n"
                    .into(),
            ),
            log: Err("no log\nsecond".into()),
        };
        let report = assemble(
            &facts,
            Ok("unmanaged"),
            Err("one\ntwo".into()),
            "sway",
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
            config_warnings: Ok(Vec::new()),
            log: Err("no log".into()),
        };
        let report = assemble(
            &facts,
            Err("nix profile list failed: SECRET_TOKEN=abcdefgh\nijklmnop".into()),
            Ok("NixOS".into()),
            "sway",
            &redaction(&[("SECRET_TOKEN", "abcdefgh\nijklmnop")], None),
        );
        assert!(
            report.contains(
                "### Install route\nunknown (nix profile list failed: SECRET_TOKEN=[redacted])\n"
            ),
            "{report}"
        );
        assert!(!report.contains("abcdefgh"), "{report}");
        assert!(!report.contains("ijklmnop"), "{report}");
    }
}
