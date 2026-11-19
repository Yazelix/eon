use super::{Identity, Quota, now_ms, percent};
use serde_json::Value;
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const FIVE_HOURS: u64 = 300;
const SEVEN_DAYS: u64 = 10_080;
const FIVE_HOUR: &str = "five_hour";
const SEVEN_DAY: &str = "seven_day";
pub(crate) const BASE_WINDOWS: &[&str] = &[FIVE_HOUR, SEVEN_DAY];
const SCOPED_WEEKLY: &str = "claude-weekly-scoped-";

fn timestamp(text: &str) -> Option<u64> {
    let date = OffsetDateTime::parse(text, &Rfc3339).ok()?;
    u64::try_from(date.unix_timestamp_nanos().div_euclid(1_000_000)).ok()
}

#[derive(Clone, Copy, PartialEq)]
struct Window<'a> {
    minutes: u64,
    used: f64,
    reset: Option<(&'a str, u64)>,
}

impl Window<'_> {
    fn record(self, quota: &mut Quota, key: &str, now: u64) {
        if let Some((reset, at)) = self.reset {
            if at <= now {
                return;
            }
            quota.resets.insert(key.into(), reset.into());
        }
        quota.remaining.insert(key.into(), 100.0 - self.used);
    }

    fn key(self) -> &'static str {
        if self.minutes == FIVE_HOURS {
            FIVE_HOUR
        } else {
            SEVEN_DAY
        }
    }
}

fn read_window(value: &Value) -> Result<Option<Window<'_>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    if !matches!(
        &value["isSyntheticPlaceholder"],
        Value::Null | Value::Bool(false)
    ) {
        return Err("invalid CodexBar window placeholder".into());
    }
    let minutes = value["windowMinutes"]
        .as_u64()
        .filter(|minutes| matches!(*minutes, FIVE_HOURS | SEVEN_DAYS))
        .ok_or("invalid CodexBar window duration")?;
    let used = percent(value["usedPercent"].as_f64().ok_or("invalid percentage")?)?;
    let reset = &value["resetsAt"];
    let reset = if reset.is_null() {
        None
    } else {
        Some(
            reset
                .as_str()
                .and_then(|text| timestamp(text).map(|at| (text, at)))
                .ok_or("invalid CodexBar reset time")?,
        )
    };
    Ok(Some(Window {
        minutes,
        used,
        reset,
    }))
}

fn read_identity(report: &Value, usage: &Value) -> Option<Identity> {
    let field = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.chars().any(char::is_control))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let identity = &usage["identity"];
    let account_id = field(&identity["accountID"]);
    let account_email = field(&identity["accountEmail"]).or_else(|| field(&usage["accountEmail"]));
    let account_label = field(&report["account"]);
    (account_id.is_some() || account_email.is_some() || account_label.is_some()).then_some(
        Identity {
            account_id,
            account_email,
            account_label,
        },
    )
}

fn record_scoped(quota: &mut Quota, extras: &Value, now: u64) {
    let mut scoped = BTreeMap::<&str, Option<Window<'_>>>::new();
    for extra in extras.as_array().into_iter().flatten() {
        let Some(id) = extra["id"]
            .as_str()
            .and_then(|id| id.strip_prefix(SCOPED_WEEKLY))
            .filter(|id| {
                !id.is_empty()
                    && id
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
            })
        else {
            continue;
        };
        let window = if matches!(&extra["usageKnown"], Value::Null | Value::Bool(true)) {
            read_window(&extra["window"])
                .ok()
                .flatten()
                .filter(|window| window.minutes == SEVEN_DAYS)
        } else {
            None
        };
        scoped
            .entry(id)
            .and_modify(|known| {
                if *known != window {
                    *known = None;
                }
            })
            .or_insert(window);
    }
    for (id, window) in scoped {
        if let Some(window) = window {
            window.record(quota, &format!("seven_day_{id}"), now);
        }
    }
}

pub(super) fn parse(text: &str) -> Result<Quota, String> {
    let [report]: [Value; 1] =
        serde_json::from_str(text).map_err(|_| "expected one CodexBar usage result")?;
    if !report["error"].is_null() {
        return Err("CodexBar reported an error".into());
    }
    let claude = match (report["provider"].as_str(), report["source"].as_str()) {
        (Some("codex"), Some("codex-cli")) => false,
        (Some("claude"), Some("oauth")) => true,
        _ => return Err("unsupported CodexBar provider or source".into()),
    };
    let usage = &report["usage"];
    let confidence = &usage["dataConfidence"];
    if !confidence.is_null() && !matches!(confidence.as_str(), Some("exact" | "percentOnly")) {
        return Err("unreadable CodexBar usage confidence".into());
    }
    let observed_at_ms = usage["updatedAt"]
        .as_str()
        .and_then(timestamp)
        .filter(|&at| at != 0)
        .ok_or("invalid CodexBar observation time")?;
    if !claude && !usage["tertiary"].is_null() {
        return Err("unexpected CodexBar Codex tertiary window".into());
    }
    let primary = read_window(&usage["primary"])?;
    let secondary = read_window(&usage["secondary"])?;
    if let (Some(primary), Some(secondary)) = (primary, secondary)
        && primary.minutes == secondary.minutes
        && primary != secondary
    {
        return Err("conflicting CodexBar windows".into());
    }
    if claude && secondary.is_some_and(|window| window.minutes != SEVEN_DAYS) {
        return Err("invalid CodexBar Claude weekly window".into());
    }

    let now = now_ms();
    let mut quota = Quota {
        observed_at_ms: Some(observed_at_ms),
        identity: read_identity(&report, usage),
        ..Quota::default()
    };
    // Claude OAuth can fall back to a model's weekly quota in primary. Secondary owns the
    // all-models week, including when CodexBar repeats it in primary.
    if let Some(window) = primary.filter(|window| !claude || window.minutes == FIVE_HOURS) {
        window.record(&mut quota, window.key(), now);
    }
    if let Some(window) = secondary {
        window.record(&mut quota, window.key(), now);
    }
    if claude {
        record_scoped(&mut quota, &usage["extraRateWindows"], now);
    }
    if quota.remaining.is_empty() {
        return Err("no usable CodexBar quota windows".into());
    }
    Ok(quota)
}
