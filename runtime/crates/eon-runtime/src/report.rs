//! The runtime facts `eon report` prints (EON-C1). Each is read without starting a process or
//! creating, changing or removing configuration or runtime state; the product assembles and
//! redacts the report.

use super::{
    cli::{Inputs, calling_terminal, versions},
    diagnostics::{newest_log, orbit_run_generation},
    generation::current_generation,
    managed_environment::configuration_warnings,
    supervisor::{base_runtime_directory, runtime_directory},
    windows::listed_window_json,
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
    /// Each warning Eon gives for its configuration.
    pub config_warnings: Result<Vec<String>, String>,
    /// The newest supervisor log of the caller's window generation, else the newest of any,
    /// with its last lines.
    pub log: Result<(PathBuf, String), String>,
}

/// The window the report describes and its generation, chosen from local records alone: a
/// supervisor that validates a caller first reconciles its terminals, which would change
/// runtime state. For `caller`, an `EON_TERMINAL` value, it is the window this terminal runs in
/// and the generation whose Orbit log names the terminal, else this build's; outside Eon, the
/// `default` window and this build's generation.
fn reported_window(inputs: &Inputs, caller: Option<&str>) -> Result<(PathBuf, String), String> {
    let current = current_generation(inputs)?;
    Ok(match caller {
        None => (base_runtime_directory("eon"), current),
        Some(caller) => (
            runtime_directory("eon"),
            orbit_run_generation(caller).unwrap_or(current),
        ),
    })
}

pub fn report_facts(inputs: &Inputs) -> ReportFacts {
    let own = calling_terminal()
        .map_err(|failure| failure.detail)
        .and_then(|caller| reported_window(inputs, caller.as_deref()));
    ReportFacts {
        versions: versions(inputs),
        window: own
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|(root, generation)| listed_window_json(inputs, root, generation)),
        config_warnings: configuration_warnings(&inputs.defaults),
        log: newest_log(
            own.as_ref().ok().map(|(_, generation)| generation.as_str()),
            LOG_LINES,
        ),
    }
}
