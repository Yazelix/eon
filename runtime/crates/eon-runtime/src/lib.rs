//! Eon's runtime mechanisms, invoked with assembly-owned product inputs.

mod attention;
mod cli;
mod control;
mod diagnostics;
mod editor;
mod events;
mod generation;
mod managed_environment;
mod notification;
mod pair;
mod pane_labels;
mod platform;
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

pub use cli::{Inputs, ReleaseStage, run};
pub use control::Error;
pub use generation::current_generation as build_identity;
pub use managed_environment::{
    BarLoad, Defaults, EditorKind, EditorOpenIn, ManagedPrograms, PaddingColor, PanelHeading,
    PopupCatalog, PopupCommand, PopupDefinition, ShellConfig, StartupAnimation, TerminalAccess,
    TerminalConfig, UpdateSettings, update_target,
};
pub use report::{ReportFacts, report_facts};
pub use slots::{SlotClass, SlotPolicy};

#[cfg(test)]
mod fixtures;
