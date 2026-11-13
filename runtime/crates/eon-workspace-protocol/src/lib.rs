#![forbid(unsafe_code)]

//! Eon-owned EONW workspace protocol: the versioned codec, and the Reserved
//! Stop, whose bytes and the six-byte prefix of every message (`EONW` and
//! the little-endian version) no revision changes.

pub mod shortcuts;
pub mod stop;
pub mod v44;

use std::time::Duration;

/// One new Orbit terminal's spawn, Ready publication and lease acquisition.
pub const TERMINAL_START_TIMEOUT: Duration = Duration::from_secs(15);
/// Recovery or cleanup of a terminal, independently of its startup allowance.
pub const SESSION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
/// Native presentation admission before the first terminal starts.
pub const PRESENTATION_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
/// Admission, both start/rollback attempts, failed presentation teardown, and a reply margin.
pub const CONTROL_TIMEOUT: Duration = PRESENTATION_ADMISSION_TIMEOUT
    .saturating_add(TERMINAL_START_TIMEOUT)
    .saturating_add(SESSION_CLEANUP_TIMEOUT)
    .saturating_add(TERMINAL_START_TIMEOUT)
    .saturating_add(SESSION_CLEANUP_TIMEOUT)
    .saturating_add(SESSION_CLEANUP_TIMEOUT)
    .saturating_add(Duration::from_secs(1));
/// A Reserved Stop can wait behind startup, then perform its own cleanup.
pub const STOP_TIMEOUT: Duration = CONTROL_TIMEOUT.saturating_add(SESSION_CLEANUP_TIMEOUT);
