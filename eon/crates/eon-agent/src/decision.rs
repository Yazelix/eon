use crate::config::{Backend, Config, Limits, Role};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Reading {
    pub remaining: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub resets: BTreeMap<String, String>,
    pub running: usize,
    /// Why the backend cannot be used at all: disabled, excluded folder, guard, account check,
    /// cap, or not checked. Lead launches honour it.
    pub problem: Option<String>,
    /// Why its quota cannot admit: an unreadable or untimed reading, or a spent observation.
    /// Lead launches ignore it.
    pub quota_problem: Option<String>,
    pub detail: String,
    /// Whose usage this is and when it was fetched; set on a successful quota reading, and on a
    /// native one whose host names its source even when its facts cannot be used.
    pub source: Option<Source>,
}

/// The account a quota reading describes and when its provider data was fetched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Source {
    pub account: String,
    /// When the provider data was fetched, in Unix milliseconds, as the reader reports it;
    /// None when the reader reports no time.
    #[serde(rename = "observed_at")]
    pub observed_at_ms: Option<u64>,
    /// Where the host found the facts (`window ID` or `store`), when it says; Eon's native
    /// readers do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
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
    // Lead launches are exempt from the quota gate entirely.
    if role == Role::Lead {
        return Ok(());
    }
    if let Some(problem) = &reading.quota_problem {
        return Err(problem.clone());
    }
    let account = backend.account();
    let Some(remaining) = &reading.remaining else {
        return if backend.allow_unknown_quota {
            Ok(())
        } else {
            Err(format!(
                "quota unknown on account {account} (not explicitly allowed)"
            ))
        };
    };
    if remaining.is_empty() && !backend.allow_unknown_quota {
        return Err(format!(
            "quota empty on account {account} (not explicitly allowed)"
        ));
    }
    if let Some(limits) = backend.limits.get(&role) {
        check_limits(
            remaining,
            &reading.resets,
            limits,
            backend.absent_windows(),
            account,
        )?;
    }
    Ok(())
}

/// Whether a limit or `absent_windows` key names `window`: the same name, or a duration key
/// `DURATIONm` with no index, which names every native window `DURATIONm/INDEX` of that duration.
fn covers(key: &str, window: &str) -> bool {
    key == window
        || key.strip_suffix('m').is_some_and(|d| {
            !d.is_empty()
                && d.bytes().all(|b| b.is_ascii_digit())
                && window.strip_prefix(key).is_some_and(|rest| {
                    rest.strip_prefix('/')
                        .is_some_and(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()))
                })
        })
}

fn percent_cmp(value: f64, threshold: f64) -> std::cmp::Ordering {
    // Complementary percentages can differ at an exact decimal cutoff by a rounding error.
    if (value - threshold).abs() <= 100.0 * f64::EPSILON {
        std::cmp::Ordering::Equal
    } else {
        value.total_cmp(&threshold)
    }
}

/// A percentage for a refusal, without the rounding error of a complement (`100 - 97.1`).
fn percent_text(value: f64) -> String {
    format!("{}", (value * 1e6).round() / 1e6)
}

/// Checks each window a limit names. A missing window holds unless `absent` declares it.
///
/// A refusal names the account and, for each window under a limit, its value, the rule and the
/// threshold the rule needs: `max_used < N%` (admits strictly below), `min_remaining >= N%`
/// (inclusive floor) or `min_remaining > N%` (with `strictly_above_min`), then the reset time
/// when the reader reports one.
fn check_limits(
    remaining: &BTreeMap<String, f64>,
    resets: &BTreeMap<String, String>,
    limits: &Limits,
    absent: &[String],
    account: &str,
) -> Result<(), String> {
    use std::cmp::Ordering::{Greater, Less};
    let mut failed = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for (used, rules) in [(false, &limits.min_remaining), (true, &limits.max_used)] {
        for (window, threshold) in rules {
            let values: Vec<_> = remaining
                .iter()
                .filter(|(key, _)| window == "*" || covers(window, key))
                .collect();
            if values.is_empty() && !absent.iter().any(|a| covers(a, window)) {
                missing.push(window.as_str());
            }
            let threshold_text = percent_text(*threshold);
            for (key, value) in values {
                let (under, part) = if used {
                    (
                        percent_cmp(*value, 100.0 - threshold) != Greater,
                        format!(
                            "{key} used {}% (max_used < {threshold_text}%",
                            percent_text(100.0 - value)
                        ),
                    )
                } else {
                    let (under, op) = if limits.strictly_above_min {
                        (percent_cmp(*value, *threshold) != Greater, ">")
                    } else {
                        (percent_cmp(*value, *threshold) == Less, ">=")
                    };
                    (
                        under,
                        format!(
                            "{key} left {}% (min_remaining {op} {threshold_text}%",
                            percent_text(*value)
                        ),
                    )
                };
                if under {
                    let reset = resets
                        .get(key)
                        .map_or(String::new(), |reset| format!(", resets {reset}"));
                    failed.push((key, format!("{part}{reset})")));
                }
            }
        }
    }
    missing.sort_unstable();
    missing.dedup();
    let missing = missing.join(", ");
    if failed.is_empty() {
        return match missing.as_str() {
            "" => Ok(()),
            windows => Err(format!(
                "missing quota window {windows} on account {account}"
            )),
        };
    }
    failed.sort();
    failed.dedup();
    let parts: Vec<_> = failed.into_iter().map(|(_, part)| part).collect();
    let mut text = format!("quota limit on account {account}: {}", parts.join("; "));
    if !missing.is_empty() {
        text.push_str(&format!("; missing quota window {missing}"));
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

/// The first candidate `admits` accepts, in configured order, then the fallback. Candidates are
/// tried one at a time, so `admits` may read each one lazily: none after the pick is tried.
fn first(candidates: &Candidates, mut admits: impl FnMut(usize) -> bool) -> Option<Pick> {
    if let Some(index) = candidates.normal.iter().copied().find(|i| admits(*i)) {
        return Some(Pick {
            index,
            fallback: false,
        });
    }
    candidates
        .fallback
        .filter(|i| admits(*i))
        .map(|index| Pick {
            index,
            fallback: true,
        })
}

fn refusal() -> Refusal {
    Refusal {
        code: 3,
        message: "no backend available, refusing".into(),
    }
}

/// Admission's selection: reads `candidates` with `read` one at a time in configured order,
/// storing each reading in `readings`, and picks the first available one, then the fallback.
/// Candidates after the pick are never read and keep the readings they had.
pub fn select(
    config: &Config,
    candidates: &Candidates,
    role: Role,
    readings: &mut [Reading],
    mut read: impl FnMut(usize) -> Reading,
) -> Result<Pick, Refusal> {
    first(candidates, |index| {
        readings[index] = read(index);
        availability(&config.backends[index], &readings[index], role).is_ok()
    })
    .ok_or_else(refusal)
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
    /// Admission's selection as `launch` runs it, each candidate "read" from `prepared` when
    /// tried.
    fn select_from(
        config: &Config,
        prepared: &[Reading],
        role: Role,
        only: Option<&str>,
        popup_backend: Option<&str>,
    ) -> Result<Pick, Refusal> {
        let candidates = candidates(config, role, only, popup_backend)?;
        let mut readings = vec![Reading::default(); config.backends.len()];
        select(config, &candidates, role, &mut readings, |i| {
            prepared[i].clone()
        })
    }
    #[test]
    fn picks_the_first_available_in_configured_order() {
        let config = config();
        let mut r = readings(&config);
        let pick = |r: &[Reading], role, only| select_from(&config, r, role, only, None);
        // The same first backend every time: nothing rotates.
        for _ in 0..3 {
            assert_eq!(pick(&r, Role::Implement, None).unwrap().index, 0);
            assert_eq!(pick(&r, Role::Review, None).unwrap().index, 0);
        }
        r[0].running = 1;
        assert_eq!(pick(&r, Role::Review, None).unwrap().index, 1);
        r[1].quota_problem = Some("quota unreadable".into());
        assert_eq!(pick(&r, Role::Implement, None).unwrap().index, 2);
        assert_eq!(pick(&r, Role::Review, Some("code")).unwrap_err().code, 2);
        // Leads: only backends that opt in, quota problems ignored, access problems not.
        assert_eq!(pick(&r, Role::Lead, None).unwrap().index, 1);
        r[1].problem = Some("command guard missing or failed".into());
        assert_eq!(pick(&r, Role::Lead, None).unwrap_err().code, 3);
    }
    #[test]
    fn first_tries_candidates_one_at_a_time_and_stops_at_the_pick() {
        let candidates = Candidates {
            normal: vec![0, 1, 2],
            fallback: Some(4),
        };
        let mut tried = Vec::new();
        let pick = first(&candidates, |i| {
            tried.push(i);
            i == 1
        });
        assert_eq!(
            pick,
            Some(Pick {
                index: 1,
                fallback: false
            })
        );
        assert_eq!(tried, [0, 1]);
        tried.clear();
        let pick = first(&candidates, |i| {
            tried.push(i);
            i == 4
        });
        assert_eq!(
            pick,
            Some(Pick {
                index: 4,
                fallback: true
            })
        );
        assert_eq!(tried, [0, 1, 2, 4]);
        assert_eq!(first(&candidates, |_| false), None);
    }
    #[test]
    fn refuses_or_explicitly_falls_back() {
        let config = config();
        let mut r = readings(&config);
        for reading in &mut r[..3] {
            reading.problem = Some("unavailable".into());
        }
        assert_eq!(
            select_from(&config, &r, Role::Review, None, None)
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            select_from(&config, &r, Role::Implement, Some("code"), None)
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            select_from(&config, &r, Role::Implement, None, None).unwrap(),
            Pick {
                index: 4,
                fallback: true
            }
        );
        // Leads never fall back.
        assert_eq!(
            select_from(&config, &r, Role::Lead, None, None)
                .unwrap_err()
                .code,
            3
        );
        r[4].problem = Some("guard missing".into());
        assert_eq!(
            select_from(&config, &r, Role::Implement, None, None)
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
            select_from(&config, &r, Role::Implement, None, Some("alpha"))
                .unwrap_err()
                .code,
            3
        );
        assert_eq!(
            select_from(&config, &r, Role::Implement, None, Some("main")).unwrap(),
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
        let secondary = &config.backends[1];
        // max_used 60 at implement: 40% left holds, 40.1% admits; review allows 85.
        r[1].remaining = Some([("five_hour".into(), 40.1), ("seven_day".into(), 90.0)].into());
        assert!(availability(secondary, &r[1], Role::Implement).is_ok());
        r[1].remaining = Some([("five_hour".into(), 40.0), ("seven_day".into(), 90.0)].into());
        assert!(availability(secondary, &r[1], Role::Implement).is_err());
        assert!(availability(secondary, &r[1], Role::Review).is_ok());
        // five_hour is declared absent; seven_day is not.
        r[1].remaining = Some([("seven_day".into(), 90.0)].into());
        assert!(availability(secondary, &r[1], Role::Implement).is_ok());
        r[1].remaining = Some([("five_hour".into(), 90.0)].into());
        assert_eq!(
            availability(secondary, &r[1], Role::Implement),
            Err("missing quota window seven_day on account secondary".into())
        );
        r[1].remaining = Some(BTreeMap::new());
        assert!(availability(secondary, &r[1], Role::Implement).is_err());
        r[1].remaining = None;
        assert!(availability(secondary, &r[1], Role::Implement).is_err());
        r[2].remaining = Some([("window".into(), 0.0)].into());
        assert!(availability(&config.backends[2], &r[2], Role::Implement).is_err());
        // A backend without limits still needs a reading with windows.
        r[0].remaining = Some(BTreeMap::new());
        assert!(availability(&config.backends[0], &r[0], Role::Implement).is_err());
        r[4].remaining = None;
        assert!(availability(&config.backends[4], &r[4], Role::Implement).is_ok());
    }
    #[test]
    fn lead_checks_access_and_cap_but_no_quota() {
        let mut config = config();
        let reading = Reading {
            quota_problem: Some(
                "quota unreadable on account secondary: provider failure (deadline)".into(),
            ),
            ..Reading::default()
        };
        let secondary = &config.backends[1];
        assert_eq!(availability(secondary, &reading, Role::Lead), Ok(()));
        assert_eq!(
            availability(secondary, &reading, Role::Implement),
            Err("quota unreadable on account secondary: provider failure (deadline)".into())
        );
        let busy = Reading {
            running: secondary.cap.as_ref().unwrap().max,
            ..Reading::default()
        };
        assert_eq!(
            availability(secondary, &busy, Role::Lead),
            Err("busy (1 running)".into())
        );
        let guard = Reading {
            problem: Some("command guard missing or failed".into()),
            ..Reading::default()
        };
        assert!(availability(secondary, &guard, Role::Lead).is_err());
        config.backends[1].enabled = false;
        assert_eq!(
            availability(&config.backends[1], &Reading::default(), Role::Lead),
            Err("disabled".into())
        );
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
            check_limits(&remaining, &resets, &limits, &[], "acct-1").unwrap_err(),
            "quota limit on account acct-1: five_hour left 0% (min_remaining >= 10%); \
             weekly left 0% (min_remaining >= 10%, resets 2030-01-08T14:00:00Z)"
        );
        let missing = Limits {
            min_remaining: [("absent".into(), 1.0)].into(),
            ..Limits::default()
        };
        assert_eq!(
            check_limits(&remaining, &resets, &missing, &[], "acct-1").unwrap_err(),
            "missing quota window absent on account acct-1"
        );
    }

    #[test]
    fn missing_window_does_not_hide_exhausted_window() {
        let remaining: BTreeMap<String, f64> = BTreeMap::from([("weekly".into(), 0.0)]);
        let resets: BTreeMap<String, String> =
            BTreeMap::from([("weekly".into(), "2030-01-08T14:00:00Z".into())]);
        let limits = Limits {
            min_remaining: [("five_hour".into(), 10.0), ("weekly".into(), 10.0)].into(),
            ..Limits::default()
        };
        assert_eq!(
            check_limits(&remaining, &resets, &limits, &[], "acct-1"),
            Err("quota limit on account acct-1: weekly left 0% \
                 (min_remaining >= 10%, resets 2030-01-08T14:00:00Z); missing quota window five_hour"
                .into())
        );
    }

    /// A limited backend from TOML: `extra` holds its quota table and limits.
    fn limited(extra: &str) -> Backend {
        toml::from_str(&format!(
            "name = 'wrapper-a'\naccount = 'acct-1'\nroles = ['implement', 'review']\nguard = ['true']\n\
             commands = {{}}\n{extra}"
        ))
        .unwrap()
    }

    fn parsed(parser: crate::config::Parser, text: &str) -> Reading {
        let quota = crate::quota::parse(parser, None, text).unwrap();
        Reading {
            remaining: Some(quota.remaining),
            resets: quota.resets,
            ..Reading::default()
        }
    }

    /// Eon's Claude facts with `windows` as (duration minutes, remaining percent), in order.
    fn native(windows: &[(u64, f64)]) -> Reading {
        let windows: Vec<_> = windows
            .iter()
            .map(|(d, r)| serde_json::json!({"duration_minutes": d, "remaining_percent": r}))
            .collect();
        let facts = serde_json::json!({"claude_quota": {"state": "fresh", "windows": windows}});
        parsed(crate::config::Parser::EonClaude, &facts.to_string())
    }

    #[test]
    fn claude_text_boundaries_at_the_fleet_ceilings() {
        let backend = limited(
            "[quota]\ncommand = ['true']\nparser = 'claude_text'\n\
             [limits.review.max_used]\nfive_hour = 50\nseven_day = 40\n\
             [limits.implement.max_used]\nfive_hour = 30\n",
        );
        for (text, review, implement) in [
            ("49 39", true, false),
            ("50 39", false, false),
            ("49 40", false, false),
            ("29 90", false, true),
            ("30 0", true, false),
        ] {
            let reading = parsed(crate::config::Parser::ClaudeText, text);
            assert_eq!(
                availability(&backend, &reading, Role::Review).is_ok(),
                review,
                "{text}"
            );
            assert_eq!(
                availability(&backend, &reading, Role::Implement).is_ok(),
                implement,
                "{text}"
            );
        }
    }

    #[test]
    fn an_account_without_a_five_hour_window_is_gated_on_its_week() {
        let limits = "[limits.review.max_used]\nfive_hour = 50\nseven_day = 40\n";
        let declared = limited(&format!(
            "[quota]\ncommand = ['true']\nparser = 'claude_text'\nabsent_windows = ['five_hour']\n{limits}"
        ));
        let undeclared = limited(&format!(
            "[quota]\ncommand = ['true']\nparser = 'claude_text'\n{limits}"
        ));
        let week = |used: &str| parsed(crate::config::Parser::ClaudeText, &format!("None {used}"));
        assert!(availability(&declared, &week("39"), Role::Review).is_ok());
        assert_eq!(
            availability(&declared, &week("40"), Role::Review),
            Err("quota limit on account acct-1: seven_day used 40% (max_used < 40%)".into())
        );
        assert_eq!(
            availability(&undeclared, &week("39"), Role::Review),
            Err("missing quota window five_hour on account acct-1".into())
        );
        // The week is not declared absent: without it the account holds.
        let no_week = parsed(crate::config::Parser::ClaudeText, "10 None");
        assert_eq!(
            availability(&declared, &no_week, Role::Review),
            Err("missing quota window seven_day on account acct-1".into())
        );
        // A declared-absent window that does appear is checked normally.
        let both = parsed(crate::config::Parser::ClaudeText, "50 10");
        assert_eq!(
            availability(&declared, &both, Role::Review),
            Err("quota limit on account acct-1: five_hour used 50% (max_used < 50%)".into())
        );
    }

    #[test]
    fn a_weekly_floor_by_duration_covers_every_weekly_window_in_any_order() {
        let backend = limited(
            "[quota]\nparser = 'eon_claude'\n[limits.implement.min_remaining]\n'10080m' = 3\n",
        );
        let admits = |reading: &Reading| availability(&backend, reading, Role::Implement).is_ok();
        // Weekly only: the floor is inclusive.
        assert!(admits(&native(&[(10080, 3.0)])));
        assert!(!admits(&native(&[(10080, 2.9)])));
        // A five-hour window beside the week leaves the result to the week alone.
        for windows in [[(300, 0.0), (10080, 3.0)], [(10080, 3.0), (300, 0.0)]] {
            assert!(admits(&native(&windows)), "{windows:?}");
        }
        for windows in [[(300, 90.0), (10080, 2.9)], [(10080, 2.9), (300, 90.0)]] {
            assert!(!admits(&native(&windows)), "{windows:?}");
        }
        // A scoped second weekly window under the floor holds, whichever comes first.
        for windows in [[(10080, 80.0), (10080, 2.0)], [(10080, 2.0), (10080, 80.0)]] {
            assert!(!admits(&native(&windows)), "{windows:?}");
        }
        // No weekly window at all holds.
        assert_eq!(
            availability(&backend, &native(&[(300, 90.0)]), Role::Implement),
            Err("missing quota window 10080m on account acct-1".into())
        );
    }

    #[test]
    fn duration_keys_declare_native_windows_absent() {
        let backend = limited(
            "[quota]\nparser = 'eon_claude'\nabsent_windows = ['300m']\n\
             [limits.implement.max_used]\n'300m' = 50\n",
        );
        let admits = |reading: &Reading| availability(&backend, reading, Role::Implement).is_ok();
        assert!(admits(&native(&[(10080, 80.0)])));
        assert!(!admits(&native(&[(10080, 80.0), (300, 50.0)])));
        assert!(admits(&native(&[(300, 51.0), (10080, 80.0)])));
        let undeclared =
            limited("[quota]\nparser = 'eon_claude'\n[limits.implement.max_used]\n'300m' = 50\n");
        assert_eq!(
            availability(&undeclared, &native(&[(10080, 80.0)]), Role::Implement),
            Err("missing quota window 300m on account acct-1".into())
        );
    }

    #[test]
    fn quota_refusals_name_the_account_the_rule_and_its_threshold() {
        let backend = limited(
            "[quota]\ncommand = ['true']\nparser = 'remaining_json'\n\
             [limits.implement.min_remaining]\n'10080m' = 3\n\
             [limits.review]\nstrictly_above_min = true\n\
             [limits.review.min_remaining]\nweek = 20\n\
             [limits.review.max_used]\nfive_hour = 50\n",
        );
        let reading = |windows: &[(&str, f64)]| Reading {
            remaining: Some(windows.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
            resets: [("five_hour".into(), "2026-11-17T08:00:00Z".into())].into(),
            ..Reading::default()
        };
        for (role, windows, expected) in [
            // Inclusive floor, value without a complement's rounding error.
            (
                Role::Implement,
                &[("10080m/1", 2.9), ("10080m/0", 50.0)][..],
                "quota limit on account acct-1: 10080m/1 left 2.9% (min_remaining >= 3%)",
            ),
            // Used share, from remaining 45%, with the reset; strict floor at its threshold.
            (
                Role::Review,
                &[("five_hour", 45.0), ("week", 20.0)][..],
                "quota limit on account acct-1: five_hour used 55% \
                 (max_used < 50%, resets 2026-11-17T08:00:00Z); \
                 week left 20% (min_remaining > 20%)",
            ),
            (
                Role::Review,
                &[("five_hour", 97.1)][..],
                "missing quota window week on account acct-1",
            ),
            (
                Role::Review,
                &[("five_hour", 2.9), ("other", 1.0)][..],
                "quota limit on account acct-1: five_hour used 97.1% \
                 (max_used < 50%, resets 2026-11-17T08:00:00Z); missing quota window week",
            ),
        ] {
            assert_eq!(
                availability(&backend, &reading(windows), role),
                Err(expected.to_string()),
                "{windows:?}"
            );
        }
        let mut unlimited = limited("");
        unlimited.limits.clear();
        assert_eq!(
            availability(&unlimited, &Reading::default(), Role::Implement),
            Err("quota unknown on account acct-1 (not explicitly allowed)".into())
        );
        assert_eq!(
            availability(&unlimited, &reading(&[]), Role::Implement),
            Err("quota empty on account acct-1 (not explicitly allowed)".into())
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
        reading.source = Some(Source {
            account: "acct".into(),
            observed_at_ms: Some(1000),
            via: None,
        });
        let json = serde_json::to_value(&reading).unwrap();
        assert_eq!(
            json["source"],
            serde_json::json!({"account": "acct", "observed_at": 1000})
        );
        reading.source.as_mut().unwrap().via = Some("store".into());
        assert_eq!(
            serde_json::to_value(&reading).unwrap()["source"],
            serde_json::json!({"account": "acct", "observed_at": 1000, "via": "store"})
        );
        assert!(json["quota_problem"].is_null());
    }

    #[test]
    fn duration_keys_cover_only_their_indexed_windows() {
        assert!(covers("10080m", "10080m"));
        assert!(covers("10080m", "10080m/0"));
        assert!(covers("10080m", "10080m/12"));
        assert!(covers("five_hour", "five_hour"));
        for (key, window) in [
            ("10080m", "1008m/0"),
            ("10080m", "10080m/"),
            ("10080m", "10080m/x"),
            ("10080m", "10080m/0/1"),
            ("10080m/0", "10080m/1"),
            ("m", "m/0"),
            ("five_hour", "five_hour/0"),
        ] {
            assert!(!covers(key, window), "{key} {window}");
        }
    }
}
