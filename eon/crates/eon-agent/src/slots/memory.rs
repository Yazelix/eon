//! Shared heavy-run memory policy. It never opens slot state or contacts Boat.
use crate::config::BoatConfig;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(super) const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Machine {
    #[default]
    Default,
    Large,
}

impl Machine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Large => "large",
        }
    }
    pub fn weight(self) -> f64 {
        match self {
            Self::Default => 1.0,
            Self::Large => 2.0,
        }
    }
    pub fn margin(self) -> u64 {
        match self {
            Self::Default => GIB,
            Self::Large => 2 * GIB,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Class {
    Default,
    Nix,
    Rune,
    Bench,
}

#[derive(Default)]
struct Inputs {
    target: Option<String>,
    cargo_target: Option<String>,
    cap: Option<String>,
    cwd_hint: Option<String>,
}

pub(super) struct Request {
    class: Class,
    forced: Option<u8>,
    pub target: String,
    policy: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct Limit {
    pub class: Class,
    pub cap_gib: u8,
}

impl Request {
    fn new(
        argv: &[String],
        cwd: &Path,
        quiet: bool,
        policy: Option<PathBuf>,
        inputs: Inputs,
    ) -> Result<Self, String> {
        let forced = match inputs.cap.as_deref().filter(|v| !v.is_empty()) {
            Some(value) => Some(
                gigabytes(value, 4)
                    .ok_or("heavy.sh: HEAVY_MEM must be whole gigabytes 1G..12G (12G ceiling)")?,
            ),
            None => None,
        };
        let mut target_arg = None;
        let mut cargo_arg = None;
        let mut previous = "";
        for word in argv {
            if let Some(value) = word.strip_prefix("--target-dir=").filter(|v| !v.is_empty()) {
                target_arg = Some(value);
            }
            if let Some(value) = word
                .strip_prefix("CARGO_TARGET_DIR=")
                .filter(|v| !v.is_empty())
            {
                cargo_arg = Some(value);
            }
            if previous == "--target-dir" {
                target_arg = Some(word);
            }
            previous = word;
        }
        let target = inputs
            .target
            .as_deref()
            .filter(|v| !v.is_empty())
            .or(target_arg.filter(|v| !v.is_empty()))
            .or(cargo_arg)
            .or(inputs.cargo_target.as_deref())
            .unwrap_or("");
        let target = if target.is_empty() {
            String::new()
        } else {
            std::process::Command::new("realpath")
                .current_dir(cwd)
                .args(["-m", "--", target])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim_end_matches('\n').to_owned())
                .unwrap_or_else(|| target.to_owned())
        };
        let command = format!(" {} ", argv.join(" "));
        let class_cwd = inputs
            .cwd_hint
            .unwrap_or_else(|| cwd.to_string_lossy().into_owned());
        let class = if quiet {
            Class::Bench
        } else if command.contains("nix build ") || command.contains(" nix profile ") {
            Class::Nix
        } else if target.contains("eon-rune") || class_cwd.contains("eon-rune") {
            Class::Rune
        } else {
            Class::Default
        };
        Ok(Self {
            class,
            forced,
            target,
            policy,
        })
    }

    pub fn from_env(
        argv: &[String],
        cwd: &Path,
        quiet: bool,
        policy: Option<PathBuf>,
    ) -> Result<Self, String> {
        Self::new(
            argv,
            cwd,
            quiet,
            policy,
            Inputs {
                target: std::env::var("HEAVY_TARGET").ok(),
                cargo_target: std::env::var("CARGO_TARGET_DIR").ok(),
                cap: std::env::var("HEAVY_MEM").map_or_else(
                    |error| match error {
                        std::env::VarError::NotPresent => Ok(None),
                        std::env::VarError::NotUnicode(_) => Err("heavy.sh: invalid HEAVY_MEM"),
                    },
                    |value| Ok(Some(value)),
                )?,
                cwd_hint: std::env::var("PWD").ok(),
            },
        )
    }

    pub fn limit(&self) -> Limit {
        let mut cap = match self.class {
            Class::Nix | Class::Rune => 8,
            Class::Bench => 4,
            Class::Default => 6,
        };
        if let Some(path) = &self.policy
            && let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(path)
            && file
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= 65536)
        {
            let mut text = String::new();
            if file.take(65537).read_to_string(&mut text).is_ok() && text.len() <= 65536 {
                for line in text.lines() {
                    let mut words = line.split_whitespace();
                    if words.next() == Some(self.class.as_str())
                        && let Some(value) = words.next().and_then(|v| gigabytes(v, 2))
                    {
                        cap = value;
                    }
                }
            }
        }
        Limit {
            class: self.class,
            cap_gib: self.forced.unwrap_or(cap),
        }
    }
}

fn gigabytes(value: &str, digits: usize) -> Option<u8> {
    let number = value.strip_suffix('G')?;
    if number.is_empty()
        || number.len() > digits
        || number.starts_with('0')
        || !number.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    number.parse::<u8>().ok().filter(|v| (1..=12).contains(v))
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Nix => "nix",
            Self::Rune => "rune",
            Self::Bench => "bench",
        }
    }
}

impl Limit {
    pub fn machine(self, config: &BoatConfig) -> Option<Machine> {
        if self.cap_gib <= 6 {
            Some(Machine::Default)
        } else if self.cap_gib <= config.large_max_memory_gib {
            Some(Machine::Large)
        } else {
            None
        }
    }
    pub fn bytes(self) -> u64 {
        u64::from(self.cap_gib) * GIB
    }
    pub fn high(self) -> Option<u64> {
        (self.class == Class::Rune && self.cap_gib >= 2).then(|| u64::from(self.cap_gib - 1) * GIB)
    }
    pub fn oom_message(self) -> String {
        format!(
            "heavy.sh: OOM-KILLED at cap {}G (class {}); fix the growth or rerun with HEAVY_MEM=<higher>",
            self.cap_gib,
            self.class.as_str()
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Cause {
    None,
    JobLimit,
    VmWide,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Observation {
    max_bytes: u64,
    high_bytes: Option<u64>,
    swap_bytes: u64,
    available: u64,
    total: u64,
    ceiling: u64,
    scope_oom: u64,
    scope_kills: u64,
    guest_kills: u64,
    own_events: u64,
    global_events: u64,
    other_events: u64,
    complete: bool,
    #[serde(default)]
    vmstat_readable: bool,
    #[serde(default)]
    drained: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Report {
    pub limit: Limit,
    pub machine: Machine,
    pub cause: Cause,
    pub vm_oom: Option<bool>,
    pub preparation: bool,
    evidence: Observation,
}

impl Observation {
    pub fn report(
        self,
        limit: Limit,
        machine: Machine,
        preparation: bool,
    ) -> Result<Report, &'static str> {
        if self.max_bytes != limit.bytes()
            || self.high_bytes != limit.high()
            || self.swap_bytes != 0
            || self.available > self.total
            || self.ceiling < limit.bytes()
            || self.ceiling > self.available.saturating_sub(machine.margin())
        {
            return Err("remote-memory-unverified");
        }
        let cause = if self.global_events > 0 {
            Cause::VmWide
        } else if !self.drained || !self.vmstat_readable || self.other_events > 0 {
            Cause::Unknown
        } else if self.scope_oom == 0
            && self.scope_kills == 0
            && self.guest_kills == 0
            && self.own_events == 0
        {
            Cause::None
        } else if self.complete
            && self.scope_oom > 0
            && self.own_events > 0
            && self.scope_kills >= self.own_events
            && self.guest_kills == self.scope_kills
        {
            Cause::JobLimit
        } else {
            Cause::Unknown
        };
        Ok(Report {
            limit,
            machine,
            cause,
            preparation,
            vm_oom: match cause {
                Cause::VmWide => Some(true),
                Cause::Unknown => None,
                _ => Some(false),
            },
            evidence: self,
        })
    }
}

pub(crate) fn helper(args: &[String]) -> Result<u8, (u8, String)> {
    if let [message, class, cap] = args
        && message == "--oom-message"
    {
        let class: Class = serde_json::from_value(serde_json::Value::String(class.clone()))
            .map_err(|_| (2, "invalid memory class".into()))?;
        let cap_gib = gigabytes(cap, 4).ok_or_else(|| (2, "invalid memory cap".into()))?;
        eprintln!("{}", Limit { class, cap_gib }.oom_message());
        return Ok(0);
    }
    let [policy, quiet, separator, argv @ ..] = args else {
        return Err((2, "invalid internal memory request".into()));
    };
    if separator != "--" || !matches!(quiet.as_str(), "0" | "1") || argv.is_empty() {
        return Err((2, "invalid internal memory request".into()));
    }
    let cwd = std::env::current_dir().map_err(|e| (2, e.to_string()))?;
    let request = Request::from_env(
        argv,
        &cwd,
        quiet == "1",
        (!policy.is_empty()).then(|| PathBuf::from(policy)),
    )
    .map_err(|e| (2, e))?;
    let limit = request.limit();
    write!(
        std::io::stdout(),
        "1\0{}\0{}\0{}\0{}\0",
        limit.class.as_str(),
        limit.cap_gib,
        limit.high().unwrap_or(0) / GIB,
        request.target
    )
    .map_err(|e| (2, e.to_string()))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(command: &[&str], cwd: &Path, quiet: bool, inputs: Inputs) -> Request {
        Request::new(
            &command.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            cwd,
            quiet,
            None,
            inputs,
        )
        .unwrap()
    }

    #[test]
    fn memory_policy_preserves_class_precedence_and_exact_overrides() {
        let cwd = Path::new("/tmp/eon-memory-fixture");
        let plain = request(
            &["nix", "develop", "-c", "eon-test"],
            cwd,
            false,
            Inputs::default(),
        );
        assert_eq!(
            plain.limit(),
            Limit {
                class: Class::Default,
                cap_gib: 6
            }
        );
        let nix = request(
            &["timeout", "85m", "nix", "build", ".#default"],
            cwd,
            false,
            Inputs::default(),
        );
        assert_eq!(
            nix.limit(),
            Limit {
                class: Class::Nix,
                cap_gib: 8
            }
        );
        let rune = request(
            &["cargo", "build"],
            Path::new("/tmp/eon-rune/source"),
            false,
            Inputs::default(),
        );
        assert_eq!(
            rune.limit(),
            Limit {
                class: Class::Rune,
                cap_gib: 8
            }
        );
        let bench = request(
            &["nix", "build", ".#default"],
            cwd,
            true,
            Inputs {
                cap: Some("1G".into()),
                ..Inputs::default()
            },
        );
        assert_eq!(
            bench.limit(),
            Limit {
                class: Class::Bench,
                cap_gib: 1
            }
        );
        for value in ["", "0G", "01G", "13G", "8", "8GB", "-1G", "1.5G"] {
            if value.is_empty() {
                continue;
            } // An unset/empty override uses policy.
            assert!(
                Request::new(
                    &["true".into()],
                    cwd,
                    false,
                    None,
                    Inputs {
                        cap: Some(value.into()),
                        ..Inputs::default()
                    }
                )
                .is_err(),
                "{value}"
            );
        }
        assert_eq!(
            request(
                &["true"],
                cwd,
                false,
                Inputs {
                    cap: Some("12G".into()),
                    ..Inputs::default()
                }
            )
            .limit()
            .cap_gib,
            12
        );
    }

    #[test]
    fn memory_policy_uses_cargo_target_precedence_and_refreshes_last_valid_policy() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path();
        let mut queued = request(
            &[
                "env",
                "CARGO_TARGET_DIR=/tmp/eon-rune-target",
                "cargo",
                "build",
                "--target-dir",
                "/tmp/plain",
            ],
            cwd,
            false,
            Inputs {
                cargo_target: Some("/tmp/eon-rune-env".into()),
                ..Inputs::default()
            },
        );
        assert_eq!(queued.limit().class, Class::Default);
        assert_eq!(queued.target, "/tmp/plain");
        let policy = cwd.join("mem-caps");
        std::fs::write(
            &policy,
            "default 9G\ndefault 13G\ndefault 3G trailing comment\ndefault nope\n",
        )
        .unwrap();
        queued.policy = Some(policy.clone());
        assert_eq!(queued.limit().cap_gib, 3);
        std::fs::write(policy, "default 5G\n").unwrap();
        assert_eq!(queued.limit().cap_gib, 5);
        let forced = request(
            &["cargo", "build", "--target-dir=/tmp/plain"],
            cwd,
            false,
            Inputs {
                target: Some("/tmp/eon-rune-override".into()),
                cap: Some("4G".into()),
                ..Inputs::default()
            },
        );
        assert_eq!(
            forced.limit(),
            Limit {
                class: Class::Rune,
                cap_gib: 4
            }
        );
    }
}
