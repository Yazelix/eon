use super::{Failure, MAX_BYTES};
use eon_workspace_protocol::v50::{
    GoogleQuota, GoogleQuotaState, MAX_GOOGLE_QUOTA_WINDOWS, MAX_QUOTA_SCOPE_BYTES, QuotaWindow,
};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// An agy version: three numbers parsed from `agy --version`, never its raw output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgyVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl AgyVersion {
    const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// A report that is exactly `MAJOR.MINOR.PATCH` around ASCII whitespace, each part
    /// decimal digits without a leading zero that fit `u32`; anything else is unrecognized.
    fn parse(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes)
            .ok()?
            .trim_matches(|c: char| c.is_ascii_whitespace());
        let mut parts = text.split('.').map(|part| {
            let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
            (digits && (part == "0" || !part.starts_with('0')))
                .then(|| part.parse().ok())
                .flatten()
        });
        let version = Self::new(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for AgyVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What `eon doctor`'s agy check finds (EON-C1). Google collection admits any version
/// whose `/usage` report validates (EON-C27), so the version is only reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgyCheck {
    /// `agy --version` succeeded: the version, or None when it is not a plain
    /// `MAJOR.MINOR.PATCH`.
    Found(Option<AgyVersion>),
    /// Why the check could not finish.
    Unavailable(&'static str),
}

/// Runs `agy --version` from `path` with the collector's resolution and bounds, and
/// `limit` as its deadline.
pub(super) fn version_check_on(path: Option<&OsStr>, limit: Duration) -> AgyCheck {
    let Ok(program) = resolve(path) else {
        return AgyCheck::Unavailable("no executable agy on an absolute PATH entry");
    };
    let deadline = Instant::now() + limit;
    // Keep the sender alive: a disconnected stop channel reads as a stop request.
    let (_running, stop) = mpsc::channel();
    let report = in_private_directory(|directory| {
        let mut remaining = MAX_BYTES;
        super::run_command(
            command(&program, directory, &["--version"]),
            deadline,
            &mut remaining,
            &stop,
        )
    });
    match report {
        Ok(Some((bytes, status))) if status.success() => AgyCheck::Found(AgyVersion::parse(&bytes)),
        Ok(None) => AgyCheck::Unavailable("agy --version was stopped"),
        Err(Failure::Deadline) => AgyCheck::Unavailable("agy --version did not finish in time"),
        Err(Failure::OutputLimit) => AgyCheck::Unavailable("agy --version wrote too much output"),
        Ok(Some(_)) | Err(_) => AgyCheck::Unavailable("agy --version could not run or failed"),
    }
}

/// [`version_check_on`] with this process's PATH.
pub fn agy_version_check(limit: Duration) -> AgyCheck {
    version_check_on(std::env::var_os("PATH").as_deref(), limit)
}

/// Runs `body` in a new empty private directory, removed afterwards.
fn in_private_directory<T>(body: impl FnOnce(&Path) -> Result<T, Failure>) -> Result<T, Failure> {
    let directory =
        std::env::temp_dir().join(format!("eon-agy-{}", crate::supervisor::request_id()));
    if !directory.is_absolute() {
        return Err(Failure::Start);
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| Failure::Start)?;
    let result = body(&directory);
    let _ = fs::remove_dir_all(directory);
    result
}

/// Unattended collection never searches the workspace through relative PATH entries.
pub(super) fn resolve(path: Option<&OsStr>) -> Result<PathBuf, Failure> {
    path.map(std::env::split_paths)
        .into_iter()
        .flatten()
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("agy"))
        .find_map(|program| executable(&program).ok())
        .ok_or(Failure::Start)
}

fn executable(program: &Path) -> Result<PathBuf, Failure> {
    let program = fs::canonicalize(program).map_err(|_| Failure::Start)?;
    if !program.is_file()
        || rustix::fs::accessat(
            rustix::fs::CWD,
            &program,
            rustix::fs::Access::EXEC_OK,
            rustix::fs::AtFlags::EACCESS,
        )
        .is_err()
    {
        return Err(Failure::Start);
    }
    Ok(program)
}

pub(super) fn command(program: &Path, directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    // agy's version and usage reports do not depend on the locale, and `eon doctor` runs
    // this command too, which must start nothing but agy (EON-C1).
    crate::managed_environment::remove_agent_session_without_desktop(&mut command)
        .env_remove(crate::sessions::TERMINAL_VARIABLE)
        .env_remove(crate::sessions::TAB_VARIABLE)
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    command
}

pub(super) fn fetch(
    program: &Path,
    timeout: Duration,
    stop: &mpsc::Receiver<()>,
) -> Result<Option<Vec<u8>>, Failure> {
    let deadline = Instant::now() + timeout;
    fetch_with(program, move |command, remaining| {
        super::run_command(command, deadline, remaining, stop)
    })
}

fn fetch_with(
    program: &Path,
    mut run: impl FnMut(Command, &mut usize) -> super::CommandResult,
) -> Result<Option<Vec<u8>>, Failure> {
    let program = if program == Path::new("agy") {
        resolve(std::env::var_os("PATH").as_deref())?
    } else {
        executable(program)?
    };
    in_private_directory(|directory| {
        let mut remaining = MAX_BYTES;
        let Some((bytes, status)) = run(
            command(
                &program,
                directory,
                &["-p", "/usage", "--output-format", "json"],
            ),
            &mut remaining,
        )?
        else {
            return Ok(None);
        };
        if !status.success() {
            let reported_error = serde_json::from_slice::<Value>(&bytes)
                .is_ok_and(|report| report.is_object() && report["status"] == "ERROR");
            return Err(if reported_error {
                Failure::ProviderError
            } else {
                Failure::ExitStatus
            });
        }
        Ok(Some(bytes))
    })
}

#[derive(PartialEq)]
struct Bucket<'a> {
    scope: &'a str,
    duration_minutes: u32,
    fraction: Option<f64>,
    reset: Option<OffsetDateTime>,
    disabled: bool,
}

/// A successful empty report clears facts; an error must preserve the last good reading.
pub(super) fn normalize(bytes: &[u8], observed_at: u64) -> Result<Option<GoogleQuota>, Failure> {
    if bytes.len() > MAX_BYTES {
        return Err(Failure::OutputLimit);
    }
    let report: Value = serde_json::from_slice(bytes).map_err(|_| Failure::Malformed)?;
    match report["status"].as_str().ok_or(Failure::Malformed)? {
        "ERROR" => return Err(Failure::ProviderError),
        "SUCCESS" => {}
        _ => return Err(Failure::Malformed),
    }
    // Turns first: a prompt turn's report has no `command`, and must back off (EON-C27).
    let turns = report["num_turns"].as_u64().ok_or(Failure::Malformed)?;
    let usage = report["usage"].as_object().ok_or(Failure::Malformed)?;
    let mut tokens = false;
    for key in [
        "input_tokens",
        "output_tokens",
        "thinking_tokens",
        "cache_read_tokens",
        "total_tokens",
    ] {
        tokens |= usage
            .get(key)
            .and_then(Value::as_u64)
            .ok_or(Failure::Malformed)?
            != 0;
    }
    if turns != 0 || tokens {
        return Err(Failure::ProviderError);
    }
    let command = report["command"].as_object().ok_or(Failure::Malformed)?;
    if command
        .get("name")
        .and_then(Value::as_str)
        .ok_or(Failure::Malformed)?
        != "usage"
    {
        return Err(Failure::ProviderError);
    }
    if observed_at == 0 {
        return Err(Failure::ObservationTime);
    }
    let groups = report["command"]["data"]["groups"]
        .as_array()
        .ok_or(Failure::Malformed)?;
    let mut supplied: Vec<Bucket<'_>> = Vec::new();
    for group in groups {
        let group = group.as_object().ok_or(Failure::Malformed)?;
        let scope = group
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= MAX_QUOTA_SCOPE_BYTES
                    && !s.chars().any(char::is_control)
            })
            .ok_or(Failure::InvalidWindow)?;
        let buckets = group
            .get("buckets")
            .and_then(Value::as_array)
            .ok_or(Failure::Malformed)?;
        for bucket in buckets {
            let bucket = bucket.as_object().ok_or(Failure::Malformed)?;
            let duration_minutes = match bucket.get("window").and_then(Value::as_str) {
                Some("5h") => 300,
                Some("weekly") => 10080,
                _ => return Err(Failure::InvalidWindow),
            };
            let disabled = bucket
                .get("disabled")
                .map(|v| v.as_bool().ok_or(Failure::InvalidWindow))
                .transpose()?
                .unwrap_or(false);
            let fraction = bucket
                .get("remaining_fraction")
                .map(|v| {
                    v.as_f64()
                        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
                        .ok_or(Failure::InvalidWindow)
                })
                .transpose()?;
            if !disabled && fraction.is_none() {
                return Err(Failure::InvalidWindow);
            }
            let reset = bucket
                .get("reset_time")
                .filter(|v| !v.is_null())
                .map(|v| {
                    OffsetDateTime::parse(v.as_str().ok_or(Failure::InvalidWindow)?, &Rfc3339)
                        .map_err(|_| Failure::InvalidWindow)
                })
                .transpose()?;
            let next = Bucket {
                scope,
                duration_minutes,
                fraction,
                reset,
                disabled,
            };
            // Compare source values before flooring or omitting expired/disabled windows.
            match supplied
                .iter()
                .find(|first| first.scope == scope && first.duration_minutes == duration_minutes)
            {
                Some(first) if *first == next => {}
                Some(_) => return Err(Failure::InvalidWindow),
                None => supplied.push(next),
            }
        }
    }
    let mut windows = Vec::new();
    for bucket in supplied {
        if bucket.disabled
            || bucket
                .reset
                .is_some_and(|date| i128::from(date.unix_timestamp()) <= i128::from(observed_at))
        {
            continue;
        }
        if windows.len() == MAX_GOOGLE_QUOTA_WINDOWS {
            return Err(Failure::InvalidWindow);
        }
        windows.push(QuotaWindow {
            duration_minutes: bucket.duration_minutes,
            remaining_percent: (100.0 * bucket.fraction.expect("active fraction validated")).floor()
                as u8,
            resets_at: bucket.reset.map(|date| {
                u64::try_from(date.unix_timestamp()).expect("unexpired reset is positive")
            }),
            scope: Some(bucket.scope.to_owned()),
        });
    }
    Ok((!windows.is_empty()).then_some(GoogleQuota {
        state: GoogleQuotaState::Fresh,
        observed_at,
        windows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    // Unchanged agy 1.4.1 report supplied on eon-w31ps; no secrets/account identity.
    // SHA-256: 325e6ac206b1f1e18ee2e1787eb6b8cd9f4cfa24d47187dcdebd257eb1ef283b
    const REPORT: &[u8] = include_bytes!("../../testdata/agy-usage-1.4.1.json");
    const NOW: u64 = 1793905200; // 2026-11-05T19:00:00Z, before the sample resets.
    // agy 1.5.6 report supplied on eon-9iriq, scrubbed by allow-list to the parsed fields with
    // neutral fractions and resets; the shape and group names are the source's.
    // SHA-256: d6c77d417f3a6493e049fa065a061d7b0ffc03f6ce904b57fb46037b635eae6f
    const REPORT_1_5_6: &[u8] = include_bytes!("../../testdata/agy-usage-1.5.6.json");
    const NOW_1_5_6: u64 = 1795132800; // 2026-11-20T00:00:00Z, before the fixture resets.
    // Synthetic report with agy 1.5.17's fields (eon-n588l): invented values, no identity.
    const REPORT_1_5_17: &[u8] = include_bytes!("../../testdata/agy-usage-1.5.17.json");
    const NOW_1_5_17: u64 = 1795629600; // 2026-11-25T18:00:00Z, before the fixture resets.

    fn sample() -> Value {
        serde_json::from_slice(REPORT).unwrap()
    }

    fn one_bucket() -> Value {
        let mut report = sample();
        let groups = report["command"]["data"]["groups"].as_array_mut().unwrap();
        groups.truncate(1);
        groups[0]["buckets"].as_array_mut().unwrap().truncate(1);
        groups[0]["buckets"][0]
            .as_object_mut()
            .unwrap()
            .remove("reset_time");
        report
    }

    fn single() -> GoogleQuota {
        GoogleQuota {
            state: GoogleQuotaState::Fresh,
            observed_at: NOW,
            windows: vec![QuotaWindow {
                duration_minutes: 10080,
                remaining_percent: 100,
                resets_at: None,
                scope: Some("Gemini Models".into()),
            }],
        }
    }

    // Collect all table mismatches so the compiling-stub red run exercises every case.
    fn check<T: PartialEq + std::fmt::Debug>(
        failures: &mut Vec<String>,
        label: &str,
        actual: T,
        expected: T,
    ) {
        if actual != expected {
            failures.push(format!("{label}: expected {expected:?}, got {actual:?}"));
        }
    }

    fn supplied_fact() -> GoogleQuota {
        GoogleQuota {
            state: GoogleQuotaState::Fresh,
            observed_at: NOW,
            windows: [
                ("Gemini Models", 10080, 100, 1794509107),
                ("Gemini Models", 300, 100, 1793922307),
                ("Claude and GPT models", 10080, 0, 1794407335),
                ("Gemini 4 Argon", 10080, 93, 1794406782),
            ]
            .map(
                |(scope, duration_minutes, remaining_percent, reset)| QuotaWindow {
                    scope: Some(scope.into()),
                    duration_minutes,
                    remaining_percent,
                    resets_at: Some(reset),
                },
            )
            .into(),
        }
    }

    #[test]
    fn doctor_preflight_reports_the_version_with_the_collector_bounds() {
        let root = crate::supervisor::temporary_directory();
        let marker = root.join("ran-usage");
        // A fresh directory per agy, so no script is rewritten while it may still run.
        let agy = |name: &str, body: &str| {
            let bin = root.join(name);
            fs::create_dir(&bin).unwrap();
            super::super::tests::script(&bin.join("agy"), body);
            bin
        };
        let limit = Duration::from_secs(5);
        let mut failures = Vec::new();
        for (label, version, expected) in [
            (
                "1.5.17",
                "printf '1.5.17\\n'",
                AgyCheck::Found(Some(AgyVersion::new(1, 5, 17))),
            ),
            (
                "2.0.0",
                "printf 2.0.0",
                AgyCheck::Found(Some(AgyVersion::new(2, 0, 0))),
            ),
            ("unrecognized", "printf unknown", AgyCheck::Found(None)),
            (
                "failed",
                "printf 1.5.6; exit 3",
                AgyCheck::Unavailable("agy --version could not run or failed"),
            ),
            (
                "output cap",
                "head -c 70000 /dev/zero",
                AgyCheck::Unavailable("agy --version wrote too much output"),
            ),
        ] {
            let bin = agy(
                label,
                &format!(
                    "[ \"$*\" = --version ] || {{ : > '{}'; exit 8; }}\n{version}",
                    marker.display()
                ),
            );
            check(
                &mut failures,
                label,
                version_check_on(Some(bin.as_os_str()), limit),
                expected,
            );
        }
        let bin = agy("slow", "sleep 5");
        let started = Instant::now();
        check(
            &mut failures,
            "deadline",
            version_check_on(Some(bin.as_os_str()), Duration::from_millis(200)),
            AgyCheck::Unavailable("agy --version did not finish in time"),
        );
        check(
            &mut failures,
            "deadline is bounded",
            started.elapsed() < Duration::from_secs(3),
            true,
        );
        let unavailable = AgyCheck::Unavailable("no executable agy on an absolute PATH entry");
        for path in [
            None,
            Some(OsStr::new("")),
            Some(OsStr::new("bin:.:relative")),
        ] {
            check(
                &mut failures,
                &format!("path {path:?}"),
                version_check_on(path, limit),
                unavailable,
            );
        }
        check(
            &mut failures,
            "missing agy",
            version_check_on(Some(root.join("missing").as_os_str()), limit),
            unavailable,
        );
        check(&mut failures, "usage never runs", marker.exists(), false);
        fs::remove_dir_all(root).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn collection_runs_only_usage_within_an_inclusive_cap() {
        use std::os::unix::process::ExitStatusExt;

        let root = crate::supervisor::temporary_directory();
        let program = root.join("agy");
        super::super::tests::script(&program, "exit 91");
        let mut failures = Vec::new();
        for (label, usage, expected) in [
            ("exact cap", MAX_BYTES, Ok(Some(MAX_BYTES))),
            ("over cap", MAX_BYTES + 1, Err(Failure::OutputLimit)),
        ] {
            let mut calls = 0;
            let result = fetch_with(&program, |command, remaining| {
                let arguments = command
                    .get_args()
                    .map(|argument| argument.to_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(arguments, ["-p", "/usage", "--output-format", "json"]);
                calls += 1;
                super::super::consume_output(remaining, usage)?;
                Ok(Some((
                    vec![b'0'; usage],
                    std::process::ExitStatus::from_raw(0),
                )))
            });
            check(
                &mut failures,
                label,
                result.map(|bytes| bytes.map(|bytes| bytes.len())),
                expected,
            );
            check(&mut failures, &format!("{label} calls"), calls, 1);
        }
        fs::remove_dir_all(root).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn supplied_report_preserves_groups_and_uses_read_completion() {
        let fact = supplied_fact();
        let mut failures = Vec::new();
        check(
            &mut failures,
            "real report",
            normalize(REPORT, NOW),
            Ok(Some(fact.clone())),
        );
        let mut extras = sample();
        extras["updatedAt"] = json!("1900-01-01T00:00:00Z");
        extras["unknown"] = json!({"anything": true});
        check(
            &mut failures,
            "extras and completion",
            normalize(&serde_json::to_vec(&extras).unwrap(), NOW + 1),
            Ok(Some(GoogleQuota {
                observed_at: NOW + 1,
                ..fact.clone()
            })),
        );
        let mut active = sample();
        active["command"]["data"]["groups"][1]["buckets"][1]["disabled"] = json!(false);
        let mut five = fact;
        five.windows.insert(
            3,
            QuotaWindow {
                duration_minutes: 300,
                remaining_percent: 100,
                resets_at: None,
                scope: Some("Claude and GPT models".into()),
            },
        );
        check(
            &mut failures,
            "all five active",
            normalize(&serde_json::to_vec(&active).unwrap(), NOW),
            Ok(Some(five)),
        );
        check(
            &mut failures,
            "agy 1.5.6 report",
            normalize(REPORT_1_5_6, NOW_1_5_6),
            Ok(Some(GoogleQuota {
                state: GoogleQuotaState::Fresh,
                observed_at: NOW_1_5_6,
                windows: [
                    ("Gemini Models", 10080, 50, 1795741200),
                    ("Gemini Models", 300, 28, 1795140000),
                    ("Claude and GPT models", 10080, 75, 1795748400),
                    ("Claude and GPT models", 300, 100, 1795147200),
                    ("Gemini 4 Argon", 10080, 90, 1795755600),
                ]
                .map(
                    |(scope, duration_minutes, remaining_percent, reset)| QuotaWindow {
                        scope: Some(scope.into()),
                        duration_minutes,
                        remaining_percent,
                        resets_at: Some(reset),
                    },
                )
                .into(),
            })),
        );
        check(
            &mut failures,
            "agy 1.5.17 report",
            normalize(REPORT_1_5_17, NOW_1_5_17),
            Ok(Some(GoogleQuota {
                state: GoogleQuotaState::Fresh,
                observed_at: NOW_1_5_17,
                windows: [
                    ("Gemini Models", 10080, 40, 1796180400),
                    ("Gemini Models", 300, 85, 1795640400),
                    ("Claude and GPT models", 10080, 60, 1796115600),
                    ("Gemini 4 Argon", 10080, 70, 1796119200),
                ]
                .map(
                    |(scope, duration_minutes, remaining_percent, reset)| QuotaWindow {
                        scope: Some(scope.into()),
                        duration_minutes,
                        remaining_percent,
                        resets_at: Some(reset),
                    },
                )
                .into(),
            })),
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn windows_normalize_boundaries_without_inventing_values() {
        let mut failures = Vec::new();
        for (label, fraction, percent) in [
            ("zero", 0.0, 0),
            ("one", 1.0, 100),
            ("floor", 0.9344604015350342, 93),
            ("binary floor", 0.29, 28),
        ] {
            let mut report = one_bucket();
            report["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] =
                json!(fraction);
            let mut fact = single();
            fact.windows[0].remaining_percent = percent;
            check(
                &mut failures,
                label,
                normalize(&serde_json::to_vec(&report).unwrap(), NOW),
                Ok(Some(fact)),
            );
        }
        for (label, reset, seconds) in [
            ("absent reset", None, None),
            ("null reset", Some(Value::Null), None),
            (
                "offset reset",
                Some(json!("2026-11-05T21:00:01+02:00")),
                Some(NOW + 1),
            ),
            (
                "future reset",
                Some(json!("2026-11-05T19:00:01Z")),
                Some(NOW + 1),
            ),
        ] {
            let mut report = one_bucket();
            if let Some(reset) = reset {
                report["command"]["data"]["groups"][0]["buckets"][0]["reset_time"] = reset;
            }
            let mut fact = single();
            fact.windows[0].resets_at = seconds;
            check(
                &mut failures,
                label,
                normalize(&serde_json::to_vec(&report).unwrap(), NOW),
                Ok(Some(fact)),
            );
        }
        for scope in [
            "x".repeat(32),
            "é".repeat(16),
            "exact, scope:with spaces ".into(),
        ] {
            let mut report = one_bucket();
            report["command"]["data"]["groups"][0]["name"] = json!(scope);
            let mut fact = single();
            fact.windows[0].scope = Some(scope.clone());
            check(
                &mut failures,
                &scope,
                normalize(&serde_json::to_vec(&report).unwrap(), NOW),
                Ok(Some(fact)),
            );
        }
        let mut six = one_bucket();
        let group = six["command"]["data"]["groups"][0].clone();
        six["command"]["data"]["groups"] = json!(
            (0..6)
                .map(|i| {
                    let mut g = group.clone();
                    g["name"] = json!(format!("group {i}"));
                    g
                })
                .collect::<Vec<_>>()
        );
        let mut six_fact = single();
        six_fact.windows = (0..6)
            .map(|i| QuotaWindow {
                scope: Some(format!("group {i}")),
                ..single().windows.remove(0)
            })
            .collect();
        check(
            &mut failures,
            "six windows",
            normalize(&serde_json::to_vec(&six).unwrap(), NOW),
            Ok(Some(six_fact.clone())),
        );
        let mut omitted = group.clone();
        omitted["name"] = json!("disabled");
        omitted["buckets"][0]["disabled"] = json!(true);
        six["command"]["data"]["groups"]
            .as_array_mut()
            .unwrap()
            .push(omitted.clone());
        omitted["name"] = json!("expired");
        omitted["buckets"][0]["disabled"] = json!(false);
        omitted["buckets"][0]["reset_time"] = json!("2026-11-05T19:00:00Z");
        six["command"]["data"]["groups"]
            .as_array_mut()
            .unwrap()
            .push(omitted);
        check(
            &mut failures,
            "omitted windows do not count",
            normalize(&serde_json::to_vec(&six).unwrap(), NOW),
            Ok(Some(six_fact)),
        );
        let mut repeated = sample();
        let duplicate = repeated["command"]["data"]["groups"][0].clone();
        repeated["command"]["data"]["groups"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        check(
            &mut failures,
            "identical repeat",
            normalize(&serde_json::to_vec(&repeated).unwrap(), NOW),
            Ok(Some(supplied_fact())),
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn valid_empty_reports_are_distinct_from_failures() {
        let mut failures = Vec::new();
        let mut empty = one_bucket();
        empty["command"]["data"]["groups"] = json!([]);
        check(
            &mut failures,
            "empty groups",
            normalize(&serde_json::to_vec(&empty).unwrap(), NOW),
            Ok(None),
        );
        empty = one_bucket();
        empty["command"]["data"]["groups"][0]["buckets"] = json!([]);
        check(
            &mut failures,
            "empty buckets",
            normalize(&serde_json::to_vec(&empty).unwrap(), NOW),
            Ok(None),
        );
        for reset in [
            "1969-12-31T23:59:59Z",
            "2026-11-05T18:59:59Z",
            "2026-11-05T19:00:00Z",
            "2026-11-05T19:00:00.5Z",
        ] {
            let mut report = one_bucket();
            report["command"]["data"]["groups"][0]["buckets"][0]["reset_time"] = json!(reset);
            check(
                &mut failures,
                reset,
                normalize(&serde_json::to_vec(&report).unwrap(), NOW),
                Ok(None),
            );
        }
        for fraction_present in [true, false] {
            let mut report = one_bucket();
            let b = report["command"]["data"]["groups"][0]["buckets"][0]
                .as_object_mut()
                .unwrap();
            b.insert("disabled".into(), json!(true));
            if !fraction_present {
                b.remove("remaining_fraction");
            }
            check(
                &mut failures,
                &format!("disabled fraction present {fraction_present}"),
                normalize(&serde_json::to_vec(&report).unwrap(), NOW),
                Ok(None),
            );
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn invalid_reports_reject_the_whole_read_with_bounded_reasons() {
        let mut failures = Vec::new();
        let mut cases = vec![
            ("not json", b"garbage".to_vec(), Failure::Malformed),
            ("root UTF-8", vec![0xff], Failure::Malformed),
            (
                "truncated object",
                REPORT[..REPORT.len() / 2].to_vec(),
                Failure::Malformed,
            ),
            (
                "fraction overflow",
                String::from_utf8(REPORT.to_vec())
                    .unwrap()
                    .replacen("0.9344604015350342", "1e400", 1)
                    .into_bytes(),
                Failure::Malformed,
            ),
            (
                "turn overflow",
                serde_json::to_string(&one_bucket())
                    .unwrap()
                    .replacen("\"num_turns\":0", "\"num_turns\":18446744073709551616", 1)
                    .into_bytes(),
                Failure::Malformed,
            ),
        ];
        let mut scope_utf8 = serde_json::to_vec(&one_bucket()).unwrap();
        // Match the group name field, not the earlier name inside the response table.
        let scope_field = b"\"name\":\"Gemini Models\"";
        let at = scope_utf8
            .windows(scope_field.len())
            .position(|s| s == scope_field)
            .unwrap();
        scope_utf8[at + b"\"name\":\"".len()] = 0xff;
        cases.push(("scope UTF-8", scope_utf8, Failure::Malformed));
        let mut cutoff = b"{\"padding\":\"".to_vec();
        cutoff.resize(MAX_BYTES, b'x');
        cases.push(("cut at 64 KiB", cutoff.clone(), Failure::Malformed));
        cutoff.push(b'x');
        cases.push(("above cap", cutoff, Failure::OutputLimit));
        for (reason, mutations) in [
            (
                Failure::Malformed,
                vec![
                    ("", json!([])),
                    ("/status", json!("UNKNOWN")),
                    ("/command", json!([])),
                    ("/command/name", json!(null)),
                    ("/command/data", json!([])),
                    ("/command/data/groups", json!({})),
                    ("/command/data/groups/0", json!(null)),
                    ("/command/data/groups/0/buckets", json!({})),
                    ("/command/data/groups/0/buckets/0", json!([])),
                    ("/num_turns", json!("0")),
                    ("/usage/input_tokens", json!(null)),
                ],
            ),
            (
                Failure::ProviderError,
                vec![
                    ("/status", json!("ERROR")),
                    ("/command/name", json!("chat")),
                    ("/num_turns", json!(1)),
                    ("/usage/total_tokens", json!(1)),
                ],
            ),
            (
                Failure::InvalidWindow,
                vec![
                    ("/command/data/groups/0/name", json!("")),
                    ("/command/data/groups/0/name", json!("bad\n")),
                    ("/command/data/groups/0/name", json!(7)),
                    ("/command/data/groups/0/name", json!("x".repeat(33))),
                    ("/command/data/groups/0/name", json!("é".repeat(17))),
                    ("/command/data/groups/0/buckets/0/window", json!("daily")),
                    (
                        "/command/data/groups/0/buckets/0/remaining_fraction",
                        json!(-0.01),
                    ),
                    (
                        "/command/data/groups/0/buckets/0/remaining_fraction",
                        json!(1.01),
                    ),
                    (
                        "/command/data/groups/0/buckets/0/remaining_fraction",
                        json!("1"),
                    ),
                    (
                        "/command/data/groups/0/buckets/0/remaining_fraction",
                        json!(null),
                    ),
                    ("/command/data/groups/0/buckets/0/disabled", json!("true")),
                    ("/command/data/groups/0/buckets/0/disabled", json!(null)),
                    (
                        "/command/data/groups/0/buckets/0/reset_time",
                        json!("2026-02-30T00:00:00Z"),
                    ),
                    ("/command/data/groups/0/buckets/0/reset_time", json!(1)),
                ],
            ),
        ] {
            for (pointer, value) in mutations {
                let label = if pointer == "/command/name" && value.is_null() {
                    "null command.name"
                } else {
                    pointer
                };
                let mut report = one_bucket();
                if let Some(target) = report.pointer_mut(pointer) {
                    *target = value;
                } else {
                    let (parent, key) = pointer.rsplit_once('/').unwrap();
                    report
                        .pointer_mut(parent)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .insert(key.into(), value);
                }
                cases.push((label, serde_json::to_vec(&report).unwrap(), reason));
            }
        }
        for (pointer, reason) in [
            ("/status", Failure::Malformed),
            ("/command", Failure::Malformed),
            ("/command/name", Failure::Malformed),
            ("/num_turns", Failure::Malformed),
            ("/usage", Failure::Malformed),
            ("/usage/thinking_tokens", Failure::Malformed),
            ("/command/data/groups", Failure::Malformed),
            ("/command/data/groups/0/buckets", Failure::Malformed),
            ("/command/data/groups/0/name", Failure::InvalidWindow),
            (
                "/command/data/groups/0/buckets/0/window",
                Failure::InvalidWindow,
            ),
            (
                "/command/data/groups/0/buckets/0/remaining_fraction",
                Failure::InvalidWindow,
            ),
        ] {
            let mut report = one_bucket();
            let (parent, field) = pointer.rsplit_once('/').unwrap();
            report
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            let label = if pointer == "/command/name" {
                "missing command.name"
            } else {
                pointer
            };
            cases.push((label, serde_json::to_vec(&report).unwrap(), reason));
        }
        for field in ["remaining_fraction", "reset_time", "disabled"] {
            let mut report = one_bucket();
            let first = report["command"]["data"]["groups"][0]["buckets"][0].clone();
            let mut repeat = first.clone();
            repeat[field] = match field {
                "remaining_fraction" => json!(0.999),
                "reset_time" => json!("1969-12-31T23:59:59Z"),
                _ => json!(true),
            };
            report["command"]["data"]["groups"][0]["buckets"] = json!([first, repeat]);
            cases.push((
                field,
                serde_json::to_vec(&report).unwrap(),
                Failure::InvalidWindow,
            ));
        }
        let mut equal_floor = one_bucket();
        let mut first = equal_floor["command"]["data"]["groups"][0]["buckets"][0].clone();
        first["remaining_fraction"] = json!(0.934);
        let mut repeat = first.clone();
        repeat["remaining_fraction"] = json!(0.939);
        equal_floor["command"]["data"]["groups"][0]["buckets"] = json!([first, repeat]);
        cases.push((
            "conflicting equal-floor fractions",
            serde_json::to_vec(&equal_floor).unwrap(),
            Failure::InvalidWindow,
        ));
        let mut invalid_expired = one_bucket();
        invalid_expired["command"]["data"]["groups"][0]["buckets"][0]["reset_time"] =
            json!("1969-12-31T23:59:59Z");
        invalid_expired["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] =
            json!(2);
        cases.push((
            "invalid expired fraction",
            serde_json::to_vec(&invalid_expired).unwrap(),
            Failure::InvalidWindow,
        ));
        for (field, bad) in [
            ("remaining_fraction", json!(2)),
            ("window", json!("daily")),
            ("reset_time", json!("bad")),
        ] {
            let mut report = one_bucket();
            report["command"]["data"]["groups"][0]["buckets"][0]["disabled"] = json!(true);
            report["command"]["data"]["groups"][0]["buckets"][0][field] = bad;
            cases.push((
                field,
                serde_json::to_vec(&report).unwrap(),
                Failure::InvalidWindow,
            ));
        }
        // A prompt turn's report has no `command`; it must still back off as a provider error.
        let mut prompt = one_bucket();
        prompt["num_turns"] = json!(1);
        prompt.as_object_mut().unwrap().remove("command");
        cases.push((
            "turn without command",
            serde_json::to_vec(&prompt).unwrap(),
            Failure::ProviderError,
        ));
        let mut seven = one_bucket();
        let group = seven["command"]["data"]["groups"][0].clone();
        seven["command"]["data"]["groups"] = json!(
            (0..7)
                .map(|i| {
                    let mut g = group.clone();
                    g["name"] = json!(format!("group {i}"));
                    g
                })
                .collect::<Vec<_>>()
        );
        cases.push((
            "seven active windows",
            serde_json::to_vec(&seven).unwrap(),
            Failure::InvalidWindow,
        ));
        for (label, bytes, reason) in cases {
            check(&mut failures, label, normalize(&bytes, NOW), Err(reason));
        }
        check(
            &mut failures,
            "zero completion",
            normalize(REPORT, 0),
            Err(Failure::ObservationTime),
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn agy_versions_parse_only_plain_numbers_and_the_input_cap_is_inclusive() {
        let mut failures = Vec::new();
        // agy 1.5.6's `--version` prints exactly `1.5.6\n` (od -c, eon-9iriq).
        for (bytes, found) in [
            (b"1.4.1".as_slice(), (1, 4, 1)),
            (b"1.4.1\n", (1, 4, 1)),
            (b" 1.4.1\r\n", (1, 4, 1)),
            (b"1.5.6\n", (1, 5, 6)),
            (b"\t1.5.17 ", (1, 5, 17)),
            (b"1.4.10", (1, 4, 10)),
            (b"2.0.0", (2, 0, 0)),
            (b"0.0.0", (0, 0, 0)),
            (b"4294967295.0.0", (u32::MAX, 0, 0)),
        ] {
            check(
                &mut failures,
                &format!("version {bytes:?}"),
                AgyVersion::parse(bytes),
                Some(AgyVersion::new(found.0, found.1, found.2)),
            );
        }
        for bytes in [
            b"".as_slice(),
            b"v1.4.1",
            b"1.4.1-beta",
            b"agy 1.4.1",
            b"1.4.1\n1.4.1",
            b"\xff",
            b"01.4.1",
            b"1.04.1",
            b"1.4",
            b"1.4.1.0",
            b"1..1",
            b"+1.4.1",
            b"4294967296.0.0",
        ] {
            check(
                &mut failures,
                &format!("unrecognized version {bytes:?}"),
                AgyVersion::parse(bytes),
                None,
            );
        }
        check(
            &mut failures,
            "display",
            AgyVersion::new(1, 5, 6).to_string(),
            "1.5.6".to_owned(),
        );
        let mut report = REPORT.to_vec();
        report.resize(MAX_BYTES, b' ');
        check(
            &mut failures,
            "inclusive cap",
            normalize(&report, NOW),
            Ok(Some(supplied_fact())),
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
