//! Probe-only control variants (plan section 8, checks 6 and 7). A control
//! run changes exactly one thing against the real profile: web tools on
//! (check 6, which must find what check 4 must not), or a jail hook that
//! crashes or hangs on one marked command (check 7). `enter` refuses a
//! control without `--probe`, and every edit must find the text it
//! replaces, so a control can never silently equal the real profile.

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::manifest::Home;
use crate::profile::Loaded;

/// The text a check-7 command carries to trigger the failing hook.
pub const FAILURE_MARKER: &str = "agent-jail-hook-failure-probe";

/// How long the hanging hook sleeps: past both harnesses' 30 s timeout.
pub const HANG_SECONDS: u64 = 45;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Control {
    #[default]
    None,
    Web,
    HookCrash,
    HookTimeout,
    /// The jail hook without its link check, as if a race had beaten it:
    /// the harness's real file tools then meet workspace links, and only
    /// the kernel seal stands between them and the harness's own files.
    Links,
}

fn replace(text: &str, from: &str, to: &str) -> Result<String> {
    if !text.contains(from) {
        return Err(format!("control edit: {from:?} not found"));
    }
    Ok(text.replace(from, to))
}

impl Control {
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "none" => Ok(Self::None),
            "web" => Ok(Self::Web),
            "hook-crash" => Ok(Self::HookCrash),
            "hook-timeout" => Ok(Self::HookTimeout),
            "links" => Ok(Self::Links),
            other => Err(format!("unknown control {other}")),
        }
    }

    /// The jail hook's command under this control.
    pub fn jail_hook_command(self, harness: &str) -> String {
        let jail = crate::JAIL_DIR;
        match self {
            Self::HookCrash => format!("{jail}/agent-jail control-hook crash {harness}"),
            Self::HookTimeout => format!("{jail}/agent-jail control-hook hang {harness}"),
            Self::Web => format!("{jail}/agent-jail control-hook web {harness}"),
            Self::Links => format!("{jail}/agent-jail control-hook links {harness}"),
            Self::None => format!("{jail}/agent-jail jail-hook {harness}"),
        }
    }

    pub fn apply_profile(self, loaded: &mut Loaded) -> Result {
        if self != Self::Web {
            return Ok(());
        }
        let argv = &mut loaded.profile.argv;
        match loaded.profile.harness.as_str() {
            "claude" => {
                let at = argv
                    .iter()
                    .position(|part| part == "--disallowedTools")
                    .ok_or("control edit: --disallowedTools not found")?;
                argv.drain(at..at + 2);
            }
            _ => {
                let part = argv
                    .iter_mut()
                    .find(|part| *part == "web_search=\"disabled\"")
                    .ok_or("control edit: web_search=\"disabled\" not found")?;
                *part = "web_search=\"live\"".into();
            }
        }
        loaded.sha256 = crate::sha256_hex(format!("{} control web", loaded.sha256).as_bytes());
        Ok(())
    }

    pub fn apply_home(self, home: &mut Home) -> Result {
        let harness = home.manifest.harness.clone();
        let edits: Vec<(&str, &str, String)> = match (self, harness.as_str()) {
            (Self::None, _) => vec![],
            (Self::Web, "claude") => vec![
                (
                    "managed-settings.json",
                    "      \"WebSearch\",\n      \"WebFetch\",\n",
                    String::new(),
                ),
                (
                    "managed-settings.json",
                    "\"/bench/jail/agent-jail jail-hook claude\"",
                    format!("\"{}\"", self.jail_hook_command("claude")),
                ),
            ],
            (Self::Web, _) => vec![
                (
                    "requirements.toml",
                    "allowed_web_search_modes = [\"disabled\"]",
                    "allowed_web_search_modes = [\"disabled\", \"live\"]".into(),
                ),
                (
                    "managed_config.toml",
                    "web_search = \"disabled\"",
                    "web_search = \"live\"".into(),
                ),
                (
                    "requirements.toml",
                    "\"/bench/jail/agent-jail jail-hook codex\"",
                    format!("\"{}\"", self.jail_hook_command("codex")),
                ),
            ],
            // Only the kernel seal stands: no hook link check, and none of
            // Claude's own checks on the resolved path (the outside-directory
            // blocks and the deny rules for the harness's home).
            (Self::Links, "claude") => vec![
                (
                    "managed-settings.json",
                    "    \"blockReadsOutsideWorkingDirectories\": true,\n    \"blockWritesOutsideWorkingDirectories\": true,\n",
                    String::new(),
                ),
                (
                    "managed-settings.json",
                    "      \"Read(//home/bench/**)\",\n      \"Edit(//home/bench/**)\",\n",
                    String::new(),
                ),
                (
                    "managed-settings.json",
                    "\"/bench/jail/agent-jail jail-hook claude\"",
                    format!("\"{}\"", self.jail_hook_command("claude")),
                ),
            ],
            (_, "claude") => vec![(
                "managed-settings.json",
                "\"/bench/jail/agent-jail jail-hook claude\"",
                format!("\"{}\"", self.jail_hook_command("claude")),
            )],
            (_, _) => vec![(
                "requirements.toml",
                "\"/bench/jail/agent-jail jail-hook codex\"",
                format!("\"{}\"", self.jail_hook_command("codex")),
            )],
        };
        for (source, from, to) in edits {
            let text = home.source_text(source)?;
            home.replace_source(source, replace(&text, from, &to)?)?;
        }
        Ok(())
    }
}

/// Web tool names the web control's hook lets through.
pub const WEB_TOOLS: &[&str] = &["webrun", "web_search", "WebSearch", "WebFetch"];

/// `agent-jail control-hook crash|hang|web <harness>`: the jail hook,
/// except that input carrying [`FAILURE_MARKER`] makes it crash (exit 1, as
/// a panic would) or hang past the harness's hook timeout, and that the web
/// control allows the web tools.
pub fn run_control_hook(kind: &str, harness: &str) -> i32 {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin()
        .take(4 << 20)
        .read_to_string(&mut input)
        .is_err()
    {
        return crate::hook::DENY_EXIT;
    }
    if kind == "web" {
        let tool = serde_json::from_str::<serde_json::Value>(&input)
            .ok()
            .and_then(|value| value["tool_name"].as_str().map(str::to_owned))
            .unwrap_or_default();
        if WEB_TOOLS.contains(&tool.as_str()) {
            return 0;
        }
    } else if input.contains(FAILURE_MARKER) {
        if kind == "hang" {
            std::thread::sleep(std::time::Duration::from_secs(HANG_SECONDS));
            return 0;
        }
        eprintln!("agent-jail control hook: simulated crash");
        return 1;
    }
    crate::hook::respond(crate::hook::decide_with(
        harness,
        &input,
        std::path::Path::new("/"),
        kind != "links",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_changes_what_it_names() {
        for harness in ["claude", "codex"] {
            let base = Home::shipped(harness).unwrap();
            for control in [
                Control::Web,
                Control::HookCrash,
                Control::HookTimeout,
                Control::Links,
            ] {
                let mut home = base.clone();
                control.apply_home(&mut home).unwrap();
                assert_ne!(home.sha256, base.sha256, "{harness} {control:?}");
                assert_ne!(home.policy(), base.policy(), "{harness} {control:?}");
            }
            let mut loaded = crate::profile::load(harness).unwrap();
            let before = loaded.profile.argv.clone();
            Control::Web.apply_profile(&mut loaded).unwrap();
            assert_ne!(loaded.profile.argv, before);
            assert!(
                !loaded
                    .profile
                    .argv
                    .iter()
                    .any(|part| part.contains("WebSearch") || part.contains("\"disabled\""))
            );
            let mut same = crate::profile::load(harness).unwrap();
            Control::HookCrash.apply_profile(&mut same).unwrap();
            assert_eq!(same.profile.argv, before);
        }
    }

    #[test]
    fn hook_controls_rewrite_only_the_jail_hook() {
        let mut home = Home::shipped("codex").unwrap();
        Control::HookCrash.apply_home(&mut home).unwrap();
        let text = home.source_text("requirements.toml").unwrap();
        assert!(text.contains("agent-jail control-hook crash codex"));
        assert!(text.contains("agent-jail dcg-hook codex"));
        assert!(!text.contains("agent-jail jail-hook codex"));
    }

    #[test]
    fn missing_text_fails_the_edit() {
        assert!(replace("abc", "x", "y").is_err());
        assert!(Control::parse("other").is_err());
    }
}
