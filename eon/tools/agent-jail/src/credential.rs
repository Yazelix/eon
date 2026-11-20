//! The start gate. The jail never refreshes a credential (Lucca's live
//! sessions share these files), so `enter` refuses unless the bound token
//! outlives the run's wall cap plus a margin. Only expiry fields are read
//! for the decision; token values stay in memory for the post-run scan and
//! never appear in a message.

use serde_json::Value;

use crate::Result;
use crate::scan::{Secret, base64_decode};

/// Minutes a token must outlive the wall cap.
pub const MARGIN_MINUTES: i64 = 15;

/// Codex refreshes a ChatGPT login proactively when its access token's
/// `exp` is this close (codex-rs/login/src/auth/manager.rs,
/// `proactive_refresh_reason`, at 0.170.0). It falls back to an age of
/// `last_refresh` only when the token has no `exp`; the gate requires one.
pub const CODEX_REFRESH_LEAD_SECONDS: i64 = 5 * 60;

/// What the gate learned from one credential file.
pub struct Checked {
    pub secrets: Vec<Secret>,
    pub account_ids: Vec<String>,
    /// Minutes left until the first expiry the run would hit.
    pub minutes_left: i64,
}

fn parse(harness: &str, text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|_| format!("{harness}: the credential file is not JSON"))
}

/// Every token string in a parsed credential file, and Codex's account ID.
fn found(harness: &str, json: &Value) -> (Vec<Secret>, Vec<String>) {
    let mut secrets = Vec::new();
    collect(json, harness, &mut secrets);
    let account_ids = json
        .pointer("/tokens/account_id")
        .and_then(Value::as_str)
        .filter(|_| harness == "codex")
        .map(str::to_owned)
        .into_iter()
        .collect();
    (secrets, account_ids)
}

/// The needles a scan looks for in `text` (the credential file of
/// `harness`): its tokens and account IDs, without the gate's expiry check.
pub fn secrets(harness: &str, text: &str) -> Result<(Vec<Secret>, Vec<String>)> {
    Ok(found(harness, &parse(harness, text)?))
}

/// Checks `text` (the credential file of `harness`) at `now` (Unix seconds)
/// for a run of `wall_cap` minutes.
pub fn gate(harness: &str, text: &str, now: i64, wall_cap: i64) -> Result<Checked> {
    let json = parse(harness, text)?;
    let (secrets, account_ids) = found(harness, &json);
    let expiry = match harness {
        "claude" => {
            let expires_ms = json
                .pointer("/claudeAiOauth/expiresAt")
                .and_then(Value::as_i64)
                .ok_or("claude: the credential file has no claudeAiOauth.expiresAt")?;
            expires_ms / 1000
        }
        "codex" => {
            let access = json
                .pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .ok_or("codex: auth.json has no tokens.access_token")?;
            let exp = jwt_claim(access, "exp")
                .and_then(|claim| claim.as_i64())
                .ok_or("codex: the access token has no exp claim")?;
            exp - CODEX_REFRESH_LEAD_SECONDS
        }
        other => return Err(format!("no credential gate for harness {other}")),
    };
    let minutes_left = (expiry - now) / 60;
    let needed = wall_cap + MARGIN_MINUTES;
    if minutes_left < needed {
        return Err(format!(
            "{harness}: the login has {minutes_left} min left and this run needs {needed} min \
             (wall cap {wall_cap} + {MARGIN_MINUTES}); {} min missing. Refresh it outside the jail first.",
            needed - minutes_left
        ));
    }
    Ok(Checked {
        secrets,
        account_ids,
        minutes_left,
    })
}

/// Collects every string under a key naming a token, secret or key.
fn collect(value: &Value, path: &str, out: &mut Vec<Secret>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}.{key}");
                let lower = key.to_ascii_lowercase();
                match child {
                    Value::String(text)
                        if (lower.contains("token")
                            || lower.contains("secret")
                            || lower.ends_with("key"))
                            && text.len() >= 12 =>
                    {
                        out.push(Secret {
                            name: child_path,
                            value: text.clone(),
                        });
                    }
                    _ => collect(child, &child_path, out),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect(item, &format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

/// One claim of a JWT's payload; the signature is not checked.
pub fn jwt_claim(token: &str, claim: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let json: Value = serde_json::from_slice(&base64_decode(payload)?).ok()?;
    json.get(claim).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::base64_encode;

    const NOW: i64 = 1_800_000_000;
    const ACCESS: &str = "sx-fake-access-token-0123456789";

    fn claude(expires_in: i64) -> String {
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{ACCESS}","refreshToken":"sx-fake-refresh-token-0123456789","expiresAt":{},"scopes":["user:inference"]}},"mcpOAuth":{{"srv":{{"accessToken":"sx-fake-mcp-token-0123456789","serverUrl":"https://mcp.example.com/very/long/url"}}}}}}"#,
            (NOW + expires_in) * 1000
        )
    }

    fn codex(exp_in: i64, refreshed_ago: i64) -> String {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let payload = base64_encode(
            format!(r#"{{"exp":{}}}"#, NOW + exp_in).as_bytes(),
            alphabet,
        );
        let refreshed = NOW - refreshed_ago;
        let days = refreshed.div_euclid(86_400);
        let seconds = refreshed.rem_euclid(86_400);
        // 2027-01-15 is day 20833 since the epoch; build the date from days.
        let (year, month, day) = civil_from_days(days);
        format!(
            r#"{{"OPENAI_API_KEY":null,"tokens":{{"id_token":"eyJh.e30.sig-id-token-123","access_token":"eyJhbGciOiJub25lIn0.{payload}.sig","refresh_token":"rt-fake-refresh-0123456789","account_id":"acct-0000-1111"}},"last_refresh":"{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.123456Z"}}"#,
            seconds / 3600,
            seconds % 3600 / 60,
            seconds % 60
        )
    }

    fn civil_from_days(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(month <= 2), month, day)
    }

    #[test]
    fn claude_gate_passes_and_refuses_without_leaking() {
        let checked = gate("claude", &claude(3 * 3600), NOW, 60).unwrap();
        assert_eq!(checked.minutes_left, 180);
        let names: Vec<&str> = checked
            .secrets
            .iter()
            .map(|secret| secret.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "claude.claudeAiOauth.accessToken",
                "claude.claudeAiOauth.refreshToken",
                "claude.mcpOAuth.srv.accessToken"
            ]
        );
        let err = gate("claude", &claude(3600), NOW, 60).err().unwrap();
        assert!(err.contains("15 min missing"), "{err}");
        assert!(!err.contains("sx-fake"), "{err}");
    }

    #[test]
    fn codex_gate_counts_the_refresh_lead() {
        let fresh = gate("codex", &codex(10 * 86_400, 3600), NOW, 60).unwrap();
        assert_eq!(fresh.account_ids, ["acct-0000-1111"]);
        assert_eq!(fresh.secrets.len(), 3);
        // 80 minutes of token, less Codex's 5-minute refresh lead, is 75.
        assert!(gate("codex", &codex(80 * 60, 0), NOW, 60).is_ok());
        let err = gate("codex", &codex(79 * 60, 0), NOW, 60).err().unwrap();
        assert!(err.contains("1 min missing"), "{err}");
        let err = gate("codex", &codex(1200, 0), NOW, 60).err().unwrap();
        assert!(err.contains("60 min missing"), "{err}");
        assert!(!err.contains("eyJ") && !err.contains("rt-fake"), "{err}");
    }

    #[test]
    fn malformed_files_are_refused_by_name_only() {
        for text in ["not json", "{}", r#"{"claudeAiOauth":{}}"#] {
            let err = gate("claude", text, NOW, 60).err().unwrap();
            assert!(err.starts_with("claude: "), "{err}");
        }
    }
}
