//! Eon's runtime mechanisms, invoked with assembly-owned product inputs.

mod admission;
mod attention;
mod checkpoint;
mod cli;
mod control;
mod diagnostics;
mod editor;
mod events;
mod generation;
mod launch_environment;
mod managed_environment;
mod notification;
mod pair;
mod pane_labels;
mod platform;
mod private_file;
mod quota;
mod report;
mod running;
mod scrollback;
mod session_registry;
mod sessions;
mod slots;
mod subscriptions;
mod supervisor;
mod windows;
mod workspace;

pub use cli::{Inputs, ReleaseStage, run, session_json};
pub use control::Error;
pub use generation::current_generation as build_identity;
pub use launch_environment::{
    CAPTURE_PREFIX as LAUNCH_CAPTURE_PREFIX, FACTS_LIMIT as LAUNCH_FACTS_LIMIT,
    FACTS_VARIABLE as LAUNCH_FACTS_VARIABLE, LaunchFacts, LaunchInputs,
};
pub use managed_environment::{
    BarLoad, Defaults, EditorKind, EditorOpenIn, ManagedPrograms, PaddingColor, PanelHeading,
    PopupCatalog, PopupCommand, PopupDefinition, ShellConfig, StartupAnimation, TerminalAccess,
    TerminalConfig, UpdateSettings, update_target, update_target_configured,
};
pub use report::{ReportFacts, report_facts};
pub use slots::{SlotClass, SlotPolicy};
pub use supervisor::configuration_directory;

#[cfg(test)]
mod fixtures;
