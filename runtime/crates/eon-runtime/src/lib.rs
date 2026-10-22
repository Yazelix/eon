//! Eon's runtime mechanisms, invoked with assembly-owned product inputs.

mod cli;
mod control;
mod diagnostics;
mod generation;
mod managed_environment;
mod quota;
mod sessions;
mod supervisor;
mod windows;
mod workspace;

pub use cli::{Inputs, run};
pub use control::Error;
pub use managed_environment::{
    Defaults, ManagedPrograms, PopupCatalog, PopupCommand, PopupDefinition, ShellConfig,
    StartupAnimation, TerminalConfig,
};

#[cfg(test)]
mod fixtures;
