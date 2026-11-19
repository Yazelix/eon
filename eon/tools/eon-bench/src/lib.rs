//! Eon's benchmark harness (docs/DEVELOPMENT.md), a development tool outside
//! Eon's closure. Each suite writes its evidence under `--out`.

pub mod child;
pub mod corpus;
mod environment;
pub mod floor;
pub mod frames;
pub mod gate;
pub mod headless;
mod isolation;
pub mod journey;
mod json;
mod kitten;
mod launcher;
pub mod memory;
pub mod orbit;
mod picture;
pub mod pressure;
pub mod pty;
pub mod response;
mod result;
mod sha256;
mod stats;
mod suite;
#[allow(unsafe_code)]
mod sys;
pub mod terminals;
mod vtebench;
pub mod writer;

/// Resolves the monotonic clock before anything is timed: rustix finds the
/// vDSO clock on its first use, so the bench pays that once at startup
/// rather than inside its first recorded stamp.
pub fn warm_clock() {
    environment::monotonic_ns();
}
