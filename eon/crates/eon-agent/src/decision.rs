use crate::config::{Backend, Config, Limits, Role};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize)]
pub struct Reading {
    pub remaining: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub resets: BTreeMap<String, String>,
    pub running: usize,
    pub problem: Option<String>,
    pub detail: String,
    /// Whose usage this is and when it was fetched; set on a successful quota reading.
    /// Not serialized: nothing consumes it yet.
    #[serde(skip)]
    pub source: Option<Source>,
}

/// The account a quota reading describes and when its provider data was fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub account: String,
    /// When the provider data was fetched, in Unix milliseconds, as the reader reports it;
    /// None when the reader reports no time.
    pub observed_at_ms: Option<u64>,
}

pub fn availability(backend: &Backend, reading: &Reading, role: Role) -> Result<(), String> {
    if !backend.enabled {
        return Err("disabled".into());
    }
    if let Some(problem) = &reading.problem {
        return Err(problem.clone());
    }
    if backend
        .cap
        .as_ref()
        .is_some_and(|cap| reading.running >= cap.max)
    {
        return Err(format!("busy ({} running)", reading.running));
    }
    let Some(remaining) = &reading.remaining else {
        return if backend.allow_unknown_quota {
            Ok(())
        } else {
            Err("quota unknown (not explicitly allowed)".into())
        };
    };
    if remaining.is_empty()
        && !backend.allow_unknown_quota
        && !backend.limits.get(&role).is_some_and(|l| l.allow_missing)
    {
        return Err("quota empty (not explicitly allowed)".into());
    }
    if let Some(limits) = backend.limits.get(&role) {
        check_limits(remaining, &reading.resets, limits)?;
    }
    Ok(())
}

fn percent_cmp(value: f64, threshold: f64) -> std::cmp::Ordering {
    // Complementary percentages can differ at an exact decimal cutoff by a rounding error.
    if (value - threshold).abs() <= 100.0 * f64::EPSILON {
        std::cmp::Ordering::Equal
    } else {
        value.total_cmp(&threshold)
    }
}

fn check_limits(
    remaining: &BTreeMap<String, f64>,
    resets: &BTreeMap<String, String>,
    limits: &Limits,
) -> Result<(), String> {
    let mut failed = BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for (used, rules) in [(false, &limits.min_remaining), (true, &limits.max_used)] {
        for (window, threshold) in rules {
            let values: Vec<_> = if window == "*" {
                remaining.iter().collect()
            } else {
                remaining.get_key_value(window).into_iter().collect()
            };
            if values.is_empty() && !limits.allow_missing {
                missing.push(window.as_str());
            }
            for (key, value) in values {
                let under = if used {
                    percent_cmp(*value, 100.0 - threshold) != std::cmp::Ordering::Greater
                } else if limits.strictly_above_min {
                    percent_cmp(*value, *threshold) != std::cmp::Ordering::Greater
                } else {
                    percent_cmp(*value, *threshold) == std::cmp::Ordering::Less
                };
                if under {
                    failed.insert(key, *value);
                }
            }
        }
    }
    if failed.is_empty() {
        return match missing.first() {
            Some(window) => Err(format!("missing quota window {window}")),
            None => Ok(()),
        };
    }
    let parts: Vec<_> = failed
        .iter()
        .map(|(key, value)| match resets.get(*key) {
            Some(reset) => format!("{key}_left={value}% (resets {reset})"),
            None => format!("{key}_left={value}%"),
        })
        .collect();
    let mut text = format!("quota limit: {}", parts.join(", "));
    if !missing.is_empty() {
        text.push_str(&format!("; missing quota window {}", missing.join(", ")));
    }
    Err(text)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pick {
    pub index: usize,
    pub fallback: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: u8,
    pub message: String,
}

pub struct Candidates {
    pub normal: Vec<usize>,
    pub fallback: Option<usize>,
}

pub fn candidates(
    config: &Config,
    role: Role,
    only: Option<&str>,
    popup_backend: Option<&str>,
) -> Result<Candidates, Refusal> {
    let pinned_fallback = role == Role::Implement
        && only.is_none()
        && popup_backend.is_some()
        && popup_backend == config.fallback.as_deref();
    let normal = if pinned_fallback {
        Vec::new()
    } else if let Some(name) = only.or(popup_backend) {
        let Some(index) = config
            .backends
            .iter()
            .position(|b| b.name == name && b.roles.contains(&role))
        else {
            return Err(Refusal {
                code: 2,
                message: format!("--only {name} is not allowed for role {role:?}"),
            });
        };
        vec![index]
    } else {
        config
            .backends
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.roles.contains(&role).then_some(i))
            .collect()
    };
    let fallback = if role == Role::Implement
        && only.is_none()
        && (popup_backend.is_none() || pinned_fallback)
    {
        config
            .backends
            .iter()
            .position(|b| Some(&b.name) == config.fallback.as_ref())
    } else {
        None
    };
    Ok(Candidates { normal, fallback })
}

/// The first available backend in `allowed`, in rotation order after `last`.
pub fn rotate(
    config: &Config,
    allowed: &[usize],
    readings: &[Reading],
    role: Role,
    last: Option<&str>,
) -> Option<usize> {
    let start = last
        .and_then(|name| config.backends.iter().position(|b| b.name == name))
        .map_or(0, |i| i + 1);
    (0..config.backends.len())
        .map(|offset| (start + offset) % config.backends.len())
        .find(|index| {
            allowed.contains(index)
                && readings
                    .get(*index)
                    .is_some_and(|r| availability(&config.backends[*index], r, role).is_ok())
        })
}

pub fn choose(
    config: &Config,
    readings: &[Reading],
    role: Role,
    only: Option<&str>,
    last: Option<&str>,
    popup_backend: Option<&str>,
) -> Result<Pick, Refusal> {
    let candidates = candidates(config, role, only, popup_backend)?;
    if let Some(index) = rotate(config, &candidates.normal, readings, role, last) {
        return Ok(Pick {
            index,
            fallback: false,
        });
    }
    if let Some(index) = candidates.fallback
        && readings
            .get(index)
            .is_some_and(|r| availability(&config.backends[index], r, role).is_ok())
    {
        return Ok(Pick {
            index,
            fallback: true,
        });
    }
    Err(Refusal {
        code: 3,
        message: "no backend available, refusing".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config::parse(include_str!("../config.example.toml"), "rota").unwrap()
    }
    fn readings(config: &Config) -> Vec<Reading> {
        config
            .backends
            .iter()
            .map(|_| Reading {
                remaining: Some(
                    [
                        ("five_hour".into(), 90.0),
                        ("weekly".into(), 90.0),
                        ("seven_day".into(), 90.0),
                    ]
                    .into(),
                ),
                ..Reading::default()
            })
            .collect()
    }
    #[test]
    fn rotates_filters_caps_and_handles_unknown_last() {
        let config = config();
        let mut r = readings(&config);
        assert_eq!(
            choose(&config, &r, Role::Implement, None, None, None)
                .unwrap()
                .index,
            0
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, None, Some("alpha"), None)
                .unwrap()
                .index,
            1
        );
        assert_eq!(
            choose(&config, &r, Role::Review, None, Some("secondary"), None)
                .unwrap()
                .index,
            0
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, None, Some("code"), None)
                .unwrap()
                .index,
            0
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, None, Some("deleted"), None)
                .unwrap()
                .index,
            0
        );
        r[0].running = 1;
        assert_eq!(
            choose(&config, &r, Role::Review, None, None, None)
                .unwrap()
                .index,
            1
        );
        assert_eq!(
            choose(&config, &r, Role::Review, Some("code"), None, None)
                .unwrap_err()
                .code,
            2
        );
    }
    #[test]
    fn rotate_uses_the_given_list_not_roles() {
        let config = config();
        let mut r = readings(&config);
        assert_eq!(rotate(&config, &[2], &r, Role::Review, None), Some(2));
        assert_eq!(
            rotate(&config, &[0, 2], &r, Role::Review, Some("alpha")),
            Some(2)
        );
        r[2].problem = Some("guard missing".into());
        assert_eq!(rotate(&config, &[2], &r, Role::Review, None), None);
    }
    #[test]
    fn refuses_or_explicitly_falls_back() {
        let config = config();
        let mut r = readings(&config);
        for reading in &mut r[..3] {
            reading.problem = Some("unavailable".into());
        }
        assert_eq!(
            choose(&config, &r, Role::Review, None, None, None)
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, Some("code"), None, None)
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, None, None, None).unwrap(),
            Pick {
                index: 4,
                fallback: true
            }
        );
        r[4].problem = Some("guard missing".into());
        assert_eq!(
            choose(&config, &r, Role::Implement, None, None, None)
                .unwrap_err()
                .code,
            3
        );
    }
    #[test]
    fn popup_pin_refuses_a_cap_even_when_other_readings_and_fallback_are_available() {
        let config = config();
        let mut r = readings(&config);
        r[0].running = config.backends[0].cap.as_ref().unwrap().max;
        assert_eq!(
            choose(&config, &r, Role::Implement, None, None, Some("alpha"))
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            choose(&config, &r, Role::Implement, None, None, Some("main")).unwrap(),
            Pick {
                index: 4,
                fallback: true
            }
        );
    }
    #[test]
    fn quota_boundaries_missing_unknown_and_role_limits() {
        let config = config();
        let mut r = readings(&config);
        r[0].remaining
            .as_mut()
            .unwrap()
            .insert("five_hour".into(), 30.0);
        assert!(availability(&config.backends[0], &r[0], Role::Implement).is_ok());
        r[0].remaining
            .as_mut()
            .unwrap()
            .insert("five_hour".into(), 29.9);
        assert!(availability(&config.backends[0], &r[0], Role::Implement).is_err());
        assert!(availability(&config.backends[0], &r[0], Role::Review).is_ok());
        r[1].remaining = Some([("five_hour".into(), 40.0)].into());
        assert!(availability(&config.backends[1], &r[1], Role::Implement).is_err());
        r[1].remaining = Some(BTreeMap::new());
        assert!(availability(&config.backends[1], &r[1], Role::Implement).is_ok());
        r[0].remaining = Some(BTreeMap::new());
        assert!(availability(&config.backends[0], &r[0], Role::Implement).is_err());
        r[2].remaining = Some([("window".into(), 0.0)].into());
        assert!(availability(&config.backends[2], &r[2], Role::Implement).is_err());
        r[0].remaining = None;
        assert!(availability(&config.backends[0], &r[0], Role::Implement).is_err());
        r[4].remaining = None;
        assert!(availability(&config.backends[4], &r[4], Role::Implement).is_ok());
    }
    #[test]
    fn empty_readings_need_explicit_permission_and_decimal_cutoffs_are_exclusive() {
        let mut config = config();
        let backend = &mut config.backends[1];
        backend.limits.clear();
        let mut reading = Reading {
            remaining: Some(BTreeMap::new()),
            ..Reading::default()
        };
        assert!(availability(backend, &reading, Role::Implement).is_err());
        backend.allow_unknown_quota = true;
        assert!(availability(backend, &reading, Role::Implement).is_ok());
        backend.allow_unknown_quota = false;
        for threshold in [30.1, 99.9] {
            backend.limits.insert(
                Role::Implement,
                Limits {
                    max_used: [("five_hour".into(), threshold)].into(),
                    ..Limits::default()
                },
            );
            for used in [threshold - 0.001, threshold, threshold + 0.001] {
                reading.remaining = Some(
                    crate::quota::parse(
                        crate::config::Parser::ClaudeText,
                        None,
                        &format!("{used} None"),
                    )
                    .unwrap()
                    .remaining,
                );
                assert_eq!(
                    availability(backend, &reading, Role::Implement).is_ok(),
                    used < threshold
                );
            }
        }
    }

    #[test]
    fn limit_refusal_lists_failing_windows_with_resets() {
        let remaining: BTreeMap<String, f64> = BTreeMap::from([
            ("five_hour".into(), 0.0),
            ("other".into(), 50.0),
            ("weekly".into(), 0.0),
        ]);
        let resets: BTreeMap<String, String> =
            BTreeMap::from([("weekly".into(), "2030-01-08T14:00:00Z".into())]);
        let limits = Limits {
            min_remaining: [("*".into(), 10.0), ("weekly".into(), 10.0)].into(),
            ..Limits::default()
        };
        assert_eq!(
            check_limits(&remaining, &resets, &limits).unwrap_err(),
            "quota limit: five_hour_left=0%, weekly_left=0% (resets 2030-01-08T14:00:00Z)"
        );
        let missing = Limits {
            min_remaining: [("absent".into(), 1.0)].into(),
            ..Limits::default()
        };
        assert_eq!(
            check_limits(&remaining, &resets, &missing).unwrap_err(),
            "missing quota window absent"
        );
    }

    #[test]
    fn missing_window_does_not_hide_exhausted_window() {
        let remaining: BTreeMap<String, f64> = BTreeMap::from([("weekly".into(), 0.0)]);
        let resets: BTreeMap<String, String> =
            BTreeMap::from([("weekly".into(), "2030-01-08T14:00:00Z".into())]);
        let limits = Limits {
            min_remaining: [("five_hour".into(), 10.0), ("weekly".into(), 10.0)].into(),
            allow_missing: false,
            ..Limits::default()
        };
        assert_eq!(
            check_limits(&remaining, &resets, &limits),
            Err("quota limit: weekly_left=0% (resets 2030-01-08T14:00:00Z); missing quota window five_hour".into())
        );
    }

    #[test]
    fn reading_serializes_resets_only_when_present() {
        let mut reading = Reading::default();
        assert!(
            serde_json::to_value(&reading)
                .unwrap()
                .get("resets")
                .is_none()
        );
        reading
            .resets
            .insert("weekly".into(), "2030-01-08T14:00:00Z".into());
        assert_eq!(
            serde_json::to_value(&reading)
                .unwrap()
                .get("resets")
                .unwrap()["weekly"],
            "2030-01-08T14:00:00Z"
        );
    }
}
