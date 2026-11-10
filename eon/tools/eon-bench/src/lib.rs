//! Eon's benchmark harness (docs/DEVELOPMENT.md), a development tool outside
//! Eon's closure. Each suite writes its evidence under `--out`.

pub mod child;
pub mod corpus;
mod environment;
pub mod floor;
pub mod gate;
pub mod headless;
mod isolation;
mod json;
mod kitten;
mod launcher;
pub mod memory;
pub mod orbit;
pub mod pressure;
pub mod pty;
pub mod response;
mod result;
mod sha256;
mod stats;
mod suite;
pub mod terminals;
mod vtebench;
pub mod writer;
