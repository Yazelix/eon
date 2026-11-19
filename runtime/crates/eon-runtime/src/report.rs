//! The runtime facts `eon report` prints (EON-C1). Each is read without starting a process or
//! creating, changing or removing configuration or runtime state; the product assembles and
//! redacts the report.

use super::{
    cli::{Inputs, calling_terminal, versions},
    diagnostics::{newest_log, orbit_run_generation},
    supervisor::{base_runtime_directory, runtime_directory},
    windows::listed_window,
};
use std::path::PathBuf;

/// How many lines of the supervisor log the report shows. It stays fixed so that maintainers
/// see the same view from everyone; the report names the log for anyone who needs more.
const LOG_LINES: usize = 40;

/// What `eon report` prints of this runtime: each fact, or why it is unknown.
pub struct ReportFacts {
    /// `eon versions`'s output.
    pub versions: Result<String, String>,
    /// The caller's window, `default` outside Eon, as one `eon window ls --json` object without
    /// its labels (window name, busy terminals' names): the caller's generation, else the
    /// window's one running generation; none when none is running.
    pub window: Result<Option<String>, String>,
    /// The newest supervisor log of the caller's window generation, else the newest of any,
    /// with its last lines.
    pub log: Result<(PathBuf, String), String>,
}

pub(super) struct ListedWindow {
    pub generation: String,
    pub caller_build: Option<String>,
    pub json: String,
}

pub enum CallerBuild {
    Outside,
    Unknown,
    Known(String),
}

pub struct DiagnosticWindow {
    pub caller_build: CallerBuild,
    caller_generation: Option<String>,
    listed: Result<Option<ListedWindow>, String>,
}

/// The window the report describes and its generation, chosen from local records alone: a
/// supervisor that validates a caller first reconciles its terminals, which would change
/// runtime state. For `caller`, an `EON_TERMINAL` value, it is the window this terminal runs in
/// and the lifetime whose Orbit log names the terminal; without one, normal
/// active-build/sole-live selection applies. Outside Eon the namespace is default.
fn reported_window(caller: Option<&str>) -> (PathBuf, Option<String>) {
    match caller {
        None => (base_runtime_directory("eon"), None),
        Some(caller) => (runtime_directory("eon"), orbit_run_generation(caller)),
    }
}

/// Inspection without a caller on the wire. Outside Eon only the report needs a window.
pub fn diagnostic_window(inputs: &Inputs, include_default: bool) -> DiagnosticWindow {
    let (caller_build, caller_generation, listed) = match calling_terminal() {
        Ok(None) if !include_default => (CallerBuild::Outside, None, Ok(None)),
        Ok(caller) => {
            let (root, generation) = reported_window(caller.as_deref());
            let listed = if include_default || generation.is_some() {
                listed_window(inputs, &root, generation.as_deref(), caller.as_deref())
            } else {
                Ok(None)
            };
            let caller_build = if caller.is_none() {
                CallerBuild::Outside
            } else {
                listed
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .filter(|window| Some(&window.generation) == generation.as_ref())
                    .and_then(|window| window.caller_build.clone())
                    .map_or(CallerBuild::Unknown, CallerBuild::Known)
            };
            (caller_build, generation, listed)
        }
        Err(failure) => (CallerBuild::Unknown, None, Err(failure.detail)),
    };
    DiagnosticWindow {
        caller_build,
        caller_generation,
        listed,
    }
}

pub fn report_facts(inputs: &Inputs, window: DiagnosticWindow) -> ReportFacts {
    let log = newest_log(
        window.caller_generation.as_deref().or_else(|| {
            window
                .listed
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .map(|window| window.generation.as_str())
        }),
        LOG_LINES,
    );
    ReportFacts {
        versions: versions(inputs),
        window: window.listed.map(|window| window.map(|window| window.json)),
        log,
    }
}
