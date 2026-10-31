//! Eon's runtime mechanisms, invoked with assembly-owned product inputs.

mod attention;
mod cli;
mod control;
mod diagnostics;
mod generation;
mod managed_environment;
mod notification;
mod pane_labels;
mod platform;
mod quota;
mod sessions;
mod supervisor;
mod windows;
mod workspace;

pub use cli::{Inputs, run};
pub use control::Error;
pub use managed_environment::{
    BarLoad, Defaults, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand, PopupDefinition,
    ShellConfig, StartupAnimation, TerminalAccess, TerminalConfig,
};

#[cfg(test)]
mod fixtures;
