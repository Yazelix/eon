//! Eon's runtime mechanisms, invoked with assembly-owned product inputs.

mod attention;
mod cli;
mod control;
mod diagnostics;
mod events;
mod generation;
mod managed_environment;
mod notification;
mod pair;
mod pane_labels;
mod platform;
mod quota;
mod running;
mod scrollback;
mod sessions;
mod subscriptions;
mod supervisor;
mod windows;
mod workspace;

pub use cli::{Inputs, run};
pub use control::Error;
pub use generation::current_generation as build_identity;
pub use managed_environment::{
    BarLoad, Defaults, ManagedPrograms, PaddingColor, PopupCatalog, PopupCommand, PopupDefinition,
    ShellConfig, StartupAnimation, TerminalAccess, TerminalConfig,
};

#[cfg(test)]
mod fixtures;
