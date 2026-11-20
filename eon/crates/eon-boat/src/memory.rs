//! Boat's remote memory evidence: the machine sizes and the report a remote run's scope
//! observation becomes.
use rota::slots::Limit;
use serde::{Deserialize, Serialize};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Machine {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Cause {
    None,
    JobLimit,
    VmWide,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Observation {
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
pub(crate) struct Report {
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
