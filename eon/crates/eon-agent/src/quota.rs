use crate::config::Parser;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) mod codexbar;

pub type Windows = BTreeMap<String, f64>;
pub type Resets = BTreeMap<String, String>;

pub const DEFAULT_AGY_GROUP: &str = "Claude and GPT models";

#[derive(Debug, Default, PartialEq)]
pub struct Quota {
    pub remaining: Windows,
    pub resets: Resets,
    /// When the provider data was fetched, in Unix milliseconds, as the reader reports it.
    pub observed_at_ms: Option<u64>,
    /// The host's last poll of this provider, when it failed after these facts were fetched.
    pub failure: Option<Failure>,
    /// Where the host found these facts, when it says.
    pub via: Option<String>,
    pub identity: Option<Identity>,
}

/// A native host's failed poll: its reason and, when the host gives a valid one, when it
/// failed, in Unix milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub reason: String,
    pub at_ms: Option<u64>,
}

/// Account details supplied by a quota command, exposed only in status JSON.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Identity {
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_label: Option<String>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

/// The value of a JSON reading's reserved `observed_at` key: Unix milliseconds, an unsigned
/// integer.
fn json_time(value: Option<&serde_json::Value>) -> Result<Option<u64>, String> {
    value
        .map(|v| v.as_u64().ok_or_else(|| "invalid observed_at".to_string()))
        .transpose()
}

fn percent(value: f64) -> Result<f64, String> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err("invalid percentage".into())
    }
}

/// Accepts `YYYY-MM-DDTHH:MM:SS[.fraction](Z|+HH:MM|-HH:MM)`; anything else is no reset time.
fn rfc3339(text: &str) -> bool {
    let b = text.as_bytes();
    let num = |at: usize, len: usize| -> Option<u32> {
        let digits = b.get(at..at + len)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    let sep = |at: usize, c: u8| b.get(at) == Some(&c);
    let date_time = num(0, 4).is_some()
        && sep(4, b'-')
        && num(5, 2).is_some_and(|m| (1..=12).contains(&m))
        && sep(7, b'-')
        && num(8, 2).is_some_and(|d| (1..=31).contains(&d))
        && matches!(b.get(10), Some(b'T' | b't'))
        && num(11, 2).is_some_and(|h| h <= 23)
        && sep(13, b':')
        && num(14, 2).is_some_and(|m| m <= 59)
        && sep(16, b':')
        && num(17, 2).is_some_and(|s| s <= 60);
    if !date_time {
        return false;
    }
    let mut at = 19;
    if sep(at, b'.') {
        let len = b[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if len == 0 {
            return false;
        }
        at += 1 + len;
    }
    match b.get(at) {
        Some(b'Z' | b'z') => b.len() == at + 1,
        Some(b'+' | b'-') => {
            b.len() == at + 6
                && num(at + 1, 2).is_some_and(|h| h <= 23)
                && sep(at + 3, b':')
                && num(at + 4, 2).is_some_and(|m| m <= 59)
        }
        _ => false,
    }
}

/// What the host's facts say about a native parser's provider besides its windows: where they
/// came from, the failure beside them and the snapshot's time.
#[derive(Debug, Default, PartialEq)]
pub struct Origin {
    pub via: Option<String>,
    pub failure: Option<Failure>,
    pub observed_at_ms: Option<u64>,
}

fn native_provider(parser: Parser) -> Option<&'static str> {
    match parser {
        Parser::EonCodex => Some("codex"),
        Parser::EonClaude => Some("claude"),
        _ => None,
    }
}

fn native_origin(json: &serde_json::Value, provider: &str) -> Origin {
    let key = format!("{provider}_quota");
    Origin {
        via: json
            .get("quota_source")
            .and_then(|sources| sources.get(provider))
            .and_then(|source| source.as_str())
            .map(Into::into),
        failure: json
            .get(format!("{key}_failure"))
            .filter(|f| !f.is_null())
            .map(|f| Failure {
                reason: f
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                at_ms: f.get("at").and_then(|at| at.as_u64()),
            }),
        observed_at_ms: json
            .get(&key)
            .and_then(|snapshot| snapshot.get("observed_at"))
            .and_then(|at| at.as_u64()),
    }
}

/// What a native reading's host facts say beside its windows, whether or not [`parse`] can
/// use them; None for other readings and for text that is not JSON.
pub fn origin(parser: Parser, text: &str) -> Option<Origin> {
    let provider = native_provider(parser)?;
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(native_origin(&json, provider))
}

/// Parses one reading. Every format but `agy` may carry `observed_at`, when the provider data
/// was fetched, in Unix milliseconds: `claude_text` as an `observed_at=MS` token after its two
/// fields, `claude_json` and `remaining_json` as a top-level `"observed_at"`, and the host's
/// facts in their snapshot (see [`crate::readings::SessionReader`]). An invalid time fails the
/// reading.
pub fn parse(parser: Parser, agy_group: Option<&str>, text: &str) -> Result<Quota, String> {
    let mut windows = Windows::new();
    let mut resets = Resets::new();
    let mut observed_at_ms = None;
    let mut failure = None;
    let mut via = None;
    match parser {
        Parser::Agy => {
            let group = agy_group.unwrap_or(DEFAULT_AGY_GROUP);
            for line in text.lines() {
                let fields: Vec<_> = line.split('\t').map(str::trim).collect();
                if fields.len() < 3 || !fields[0].eq_ignore_ascii_case(group) {
                    continue;
                }
                let name = fields[1].to_lowercase();
                let key = if name.contains("five hour") {
                    "five_hour"
                } else if name.contains("weekly") {
                    "weekly"
                } else {
                    continue;
                };
                let value: f64 = if fields[2].eq_ignore_ascii_case("disabled") {
                    0.0
                } else {
                    fields[2]
                        .strip_suffix('%')
                        .and_then(|v| v.parse().ok())
                        .ok_or("invalid agy percentage")?
                };
                if windows.insert(key.into(), percent(value)?).is_some() {
                    return Err("duplicate agy window".into());
                }
                if let Some(reset) = fields.get(3).copied().filter(|r| rfc3339(r)) {
                    resets.insert(key.into(), reset.into());
                }
            }
            if !windows.contains_key("weekly") {
                return Err(format!("agy usage has no weekly row for group {group:?}"));
            }
        }
        Parser::ClaudeText => {
            let fields: Vec<_> = text.split_whitespace().collect();
            if fields.len() < 2 {
                return Err("Claude usage needs two fields".into());
            }
            for (key, value) in ["five_hour", "seven_day"].into_iter().zip(&fields) {
                if *value != "None" {
                    let used = percent(value.parse().map_err(|_| "invalid Claude percentage")?)?;
                    windows.insert(key.into(), 100.0 - used);
                }
            }
            // Later fields are ignored, except one `observed_at=MS` token.
            for field in &fields[2..] {
                if let Some(time) = field.strip_prefix("observed_at=") {
                    if observed_at_ms.is_some() {
                        return Err("duplicate observed_at".into());
                    }
                    observed_at_ms = Some(time.parse().map_err(|_| "invalid observed_at")?);
                }
            }
        }
        Parser::ClaudeJson => {
            let json: serde_json::Value =
                serde_json::from_str(text).map_err(|_| "invalid quota JSON")?;
            if !json.is_object()
                || !["five_hour", "seven_day"]
                    .iter()
                    .any(|k| json.get(k).is_some())
            {
                return Err("not a Claude usage response".into());
            }
            observed_at_ms = json_time(json.get("observed_at"))?;
            for key in ["five_hour", "seven_day"] {
                if let Some(window) = json.get(key).filter(|v| !v.is_null()) {
                    let value = window.get("utilization").ok_or("missing utilization")?;
                    if !value.is_null() {
                        windows.insert(
                            key.into(),
                            100.0 - percent(value.as_f64().ok_or("invalid utilization")?)?,
                        );
                    }
                }
            }
        }
        Parser::Codexbar => return codexbar::parse(text),
        Parser::EonCodex | Parser::EonClaude => {
            let json: serde_json::Value =
                serde_json::from_str(text).map_err(|_| "invalid workspace JSON")?;
            let provider = native_provider(parser).expect("a native parser");
            let key = format!("{provider}_quota");
            if let Some(reason) = json
                .get("quota_unreadable")
                .and_then(|reasons| reasons.get(provider))
                .and_then(|reason| reason.as_str())
            {
                return Err(reason.into());
            }
            let origin = native_origin(&json, provider);
            via = origin.via;
            failure = origin.failure;
            // A failure beside facts that cannot be read is the reason they are missing.
            let unusable = |problem: &str| match &failure {
                Some(Failure { reason, .. }) => format!("provider failure ({reason}); {problem}"),
                None => problem.to_owned(),
            };
            let snapshot = json
                .get(&key)
                .ok_or_else(|| unusable("quota snapshot unavailable"))?;
            if snapshot.get("state").and_then(|s| s.as_str()) != Some("fresh") {
                return Err(unusable("quota snapshot is not fresh"));
            }
            observed_at_ms = json_time(snapshot.get("observed_at"))?;
            let items = snapshot
                .get("windows")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
                .ok_or("quota windows unavailable")?;
            for (i, window) in items.iter().enumerate() {
                let value = percent(
                    window
                        .get("remaining_percent")
                        .and_then(|v| v.as_f64())
                        .ok_or("invalid remaining_percent")?,
                )?;
                let duration = window
                    .get("duration_minutes")
                    .and_then(|v| v.as_u64())
                    .ok_or("invalid duration_minutes")?;
                // Include the index: scoped windows may share the same duration.
                windows.insert(format!("{duration}m/{i}"), value);
            }
        }
        Parser::RemainingJson => {
            let mut values: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text)
                .map_err(|_| "expected JSON object of remaining percentages")?;
            observed_at_ms = json_time(values.remove("observed_at").as_ref())?;
            let values: BTreeMap<String, f64> =
                serde_json::from_value(serde_json::Value::Object(values))
                    .map_err(|_| "expected JSON object of remaining percentages")?;
            if values.is_empty() {
                return Err("no quota windows".into());
            }
            for (key, value) in values {
                windows.insert(key, percent(value)?);
            }
        }
    }
    Ok(Quota {
        remaining: windows,
        resets,
        observed_at_ms,
        failure,
        via,
        identity: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXHAUSTED: &str = include_str!("../tests/fixtures/agy-exhausted.tsv");

    #[test]
    fn fixture_readers() {
        let agy = parse(Parser::Agy, None, include_str!("../tests/fixtures/agy.tsv")).unwrap();
        assert_eq!(agy.remaining["five_hour"], 67.5);
        assert_eq!(agy.remaining["weekly"], 48.0);
        assert!(agy.resets.is_empty());
        let claude = parse(
            Parser::ClaudeText,
            None,
            include_str!("../tests/fixtures/claude.txt"),
        )
        .unwrap();
        assert_eq!(claude.remaining["five_hour"], 76.5);
        assert!(!claude.remaining.contains_key("seven_day"));
        assert_eq!(
            claude,
            parse(
                Parser::ClaudeJson,
                None,
                include_str!("../tests/fixtures/claude.json")
            )
            .unwrap()
        );
        let workspace = include_str!("../tests/fixtures/workspace.json");
        assert_eq!(
            parse(Parser::EonCodex, None, workspace).unwrap().remaining["9000m/1"],
            42.0
        );
        assert_eq!(
            parse(Parser::EonClaude, None, workspace).unwrap().remaining["240m/0"],
            76.5
        );
        assert_eq!(
            parse(Parser::RemainingJson, None, "{\"week\": 43}")
                .unwrap()
                .remaining["week"],
            43.0
        );
    }
    #[test]
    fn malformed_missing_or_out_of_range_refuses() {
        for (parser, text) in [
            (Parser::Agy, ""),
            (Parser::Agy, "Claude and GPT models\tWeekly\tNaN%"),
            (
                Parser::Agy,
                "Claude and GPT models\tWeekly Limit Remaining\tn/a\t2030-01-08T14:00:00Z",
            ),
            (
                Parser::Agy,
                "Claude and GPT models\tFive Hour Limit Remaining\t50%",
            ),
            (Parser::ClaudeText, "probe-failed"),
            (Parser::ClaudeText, "101 3"),
            (Parser::ClaudeJson, "{}"),
            (
                Parser::ClaudeJson,
                "{\"five_hour\":{\"utilization\":\"oops\"}}",
            ),
            (Parser::EonCodex, "{\"codex_quota\":{\"windows\":[]}}"),
            (
                Parser::EonCodex,
                "{\"codex_quota\":{\"windows\":[{\"duration_minutes\":240,\"remaining_percent\":null}]}}",
            ),
            (Parser::RemainingJson, "{\"week\":-1}"),
        ] {
            assert!(parse(parser, None, text).is_err(), "{text}");
        }
        assert!(
            parse(Parser::ClaudeText, None, "None None")
                .unwrap()
                .remaining
                .is_empty()
        );
    }
    #[test]
    fn eon_rejects_unusable_snapshots_even_with_remaining_quota() {
        for parser in [Parser::EonCodex, Parser::EonClaude] {
            for state in [Some("stale"), Some("error"), Some("unknown"), None] {
                let mut snapshot = serde_json::json!({"windows":[{"duration_minutes":240,"remaining_percent":80}]});
                if let Some(state) = state {
                    snapshot["state"] = state.into();
                }
                let text = serde_json::json!({"codex_quota":snapshot, "claude_quota":snapshot})
                    .to_string();
                assert!(parse(parser, None, &text).is_err());
            }
        }
    }

    #[test]
    fn agy_disabled_row_reads_as_exhausted_and_keeps_reset() {
        let quota = parse(Parser::Agy, None, EXHAUSTED).unwrap();
        assert_eq!(
            quota.remaining,
            Windows::from([("five_hour".into(), 0.0), ("weekly".into(), 0.0)])
        );
        assert_eq!(
            quota.resets,
            Resets::from([("weekly".into(), "2030-01-08T14:00:00Z".into())])
        );
    }

    #[test]
    fn agy_group_selects_rows_and_weekly_only_groups_parse() {
        let gemini = parse(Parser::Agy, Some("Gemini Models"), EXHAUSTED).unwrap();
        assert_eq!(
            gemini.remaining,
            Windows::from([("five_hour".into(), 55.0), ("weekly".into(), 80.0)])
        );
        assert_eq!(gemini.resets["five_hour"], "2030-01-05T12:00:00Z");
        let model = parse(Parser::Agy, Some("example model"), EXHAUSTED).unwrap();
        assert_eq!(model.remaining, Windows::from([("weekly".into(), 90.0)]));
        assert_eq!(
            model.resets,
            Resets::from([("weekly".into(), "2030-01-07T09:30:00Z".into())])
        );
        let error = parse(Parser::Agy, Some("Absent Group"), EXHAUSTED).unwrap_err();
        assert!(error.contains("no weekly row"), "{error}");
    }

    #[test]
    fn agy_row_without_reset_field_has_no_reset() {
        let quota = parse(
            Parser::Agy,
            None,
            "Claude and GPT models\tWeekly Limit Remaining\t40%",
        )
        .unwrap();
        assert_eq!(quota.remaining, Windows::from([("weekly".into(), 40.0)]));
        assert!(quota.resets.is_empty());
    }

    #[test]
    fn every_timed_format_reports_observed_at() {
        let text = parse(Parser::ClaudeText, None, "12 34 observed_at=1720000001000").unwrap();
        assert_eq!(text.observed_at_ms, Some(1_720_000_001_000));
        assert_eq!(text.remaining["five_hour"], 88.0);
        // The fleet script's third field (a reset time) is still ignored, before or after it.
        for line in [
            "12 34 2030-01-08T14:00:00Z observed_at=5",
            "12 34 observed_at=5 2030-01-08T14:00:00Z",
        ] {
            assert_eq!(
                parse(Parser::ClaudeText, None, line)
                    .unwrap()
                    .observed_at_ms,
                Some(5)
            );
        }
        assert_eq!(
            parse(Parser::ClaudeText, None, "12 34")
                .unwrap()
                .observed_at_ms,
            None
        );
        let json = parse(
            Parser::ClaudeJson,
            None,
            r#"{"five_hour":{"utilization":10},"observed_at":7}"#,
        )
        .unwrap();
        assert_eq!(json.observed_at_ms, Some(7));
        let remaining = parse(
            Parser::RemainingJson,
            None,
            r#"{"week":43,"observed_at":8}"#,
        )
        .unwrap();
        assert_eq!(remaining.observed_at_ms, Some(8));
        // Reserved, never a window.
        assert_eq!(remaining.remaining, Windows::from([("week".into(), 43.0)]));
        assert!(parse(Parser::RemainingJson, None, r#"{"observed_at":8}"#).is_err());
        let native = parse(
            Parser::EonCodex,
            None,
            r#"{"codex_quota":{"state":"fresh","observed_at":9,
                "windows":[{"duration_minutes":300,"remaining_percent":50}]},
                "codex_quota_failure":{"at":1,"reason":"deadline"}}"#,
        )
        .unwrap();
        assert_eq!(native.observed_at_ms, Some(9));
        assert_eq!(
            native.failure,
            Some(Failure {
                reason: "deadline".into(),
                at_ms: Some(1)
            })
        );
        // A failure without a usable time keeps its reason and no time.
        for at in ["\"soon\"", "-1", "null"] {
            let native = parse(
                Parser::EonClaude,
                None,
                &format!(
                    r#"{{"claude_quota":{{"state":"fresh","observed_at":9,
                    "windows":[{{"duration_minutes":300,"remaining_percent":50}}]}},
                    "claude_quota_failure":{{"at":{at},"reason":"provider_error"}}}}"#
                ),
            )
            .unwrap();
            assert_eq!(
                native.failure,
                Some(Failure {
                    reason: "provider_error".into(),
                    at_ms: None
                })
            );
        }
        assert_eq!(
            parse(Parser::Agy, None, include_str!("../tests/fixtures/agy.tsv"))
                .unwrap()
                .observed_at_ms,
            None
        );
    }

    #[test]
    fn an_invalid_observed_at_fails_the_reading() {
        for (parser, text) in [
            (Parser::ClaudeText, "12 34 observed_at=soon"),
            (Parser::ClaudeText, "12 34 observed_at=-1"),
            (Parser::ClaudeText, "12 34 observed_at=1 observed_at=2"),
            (
                Parser::ClaudeJson,
                r#"{"five_hour":{"utilization":10},"observed_at":"7"}"#,
            ),
            (Parser::RemainingJson, r#"{"week":43,"observed_at":1.5}"#),
            (Parser::RemainingJson, r#"{"week":43,"observed_at":-1}"#),
            (
                Parser::EonClaude,
                r#"{"claude_quota":{"state":"fresh","observed_at":null,
                    "windows":[{"duration_minutes":300,"remaining_percent":50}]}}"#,
            ),
        ] {
            let error = parse(parser, None, text).unwrap_err();
            assert!(error.ends_with(" observed_at"), "{text}: {error}");
        }
    }

    #[test]
    fn reset_times_must_be_rfc3339() {
        for text in [
            "2030-01-08T14:00:00Z",
            "2030-01-08t14:00:00z",
            "2030-01-08T14:00:00.5+02:00",
        ] {
            assert!(rfc3339(text), "{text}");
        }
        for text in [
            "",
            "later",
            "2030-13-08T14:00:00Z",
            "2030-01-00T14:00:00Z",
            "2030-01-08 14:00:00Z",
            "2030-01-08T14:00:00",
            "2030-01-08T14:00:00.Z",
            "2030-01-08T14:00:00+2:00",
            "2030-01-08T14:00:00Zjunk",
        ] {
            assert!(!rfc3339(text), "{text}");
        }
    }
}
