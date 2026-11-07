//! Eon's benchmark harness (docs/DEVELOPMENT.md), a development tool outside
//! Eon's closure. Each suite writes its evidence under `--out`.

pub mod corpus;
mod environment;
pub mod floor;
mod json;
pub mod pty;
mod result;
mod sha256;
mod stats;
pub mod writer;
