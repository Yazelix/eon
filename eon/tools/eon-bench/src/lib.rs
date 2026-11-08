//! Eon's benchmark harness (docs/DEVELOPMENT.md), a development tool outside
//! Eon's closure. Each suite writes its evidence under `--out`.

pub mod child;
pub mod corpus;
mod environment;
pub mod floor;
mod json;
mod kitten;
pub mod orbit;
pub mod pty;
pub mod response;
mod result;
mod sha256;
mod stats;
mod suite;
mod vtebench;
pub mod writer;
