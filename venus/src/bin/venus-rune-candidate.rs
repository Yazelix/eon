//! Private Rune host candidate entry (eon-yp6vg.7.7): the canonical Venus grid drawn in one
//! Rune device and pass. Built only with the non-default `candidate-rune` feature.
//!
//! `venus-rune-candidate [--host rune|venus]`. Each stdin line advances one fixture step;
//! after each step's first presentation it prints `drawn <cycle> <step> <name>`. Exit 0 when
//! the window closes, 1 on a render, GPU, geometry or admission failure, 2 on refusal or
//! invalid arguments.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use yazelix_venus::rune_candidate::{Host, run};

fn main() -> ExitCode {
    // Before anything else, and before any event loop exists: the Rune pin compiles winit's
    // X11 backend, which winit picks when no Wayland display is set (lead ruling 2026-11-20).
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|display| display.is_empty()) {
        eprintln!(
            "venus-rune-candidate needs a native Wayland display: WAYLAND_DISPLAY is not set \
             (X11 is not supported)"
        );
        return ExitCode::from(2);
    }
    let mut arguments = std::env::args().skip(1);
    let host = match (
        arguments.next().as_deref(),
        arguments.next().as_deref(),
        arguments.next(),
    ) {
        (None, ..) => Host::Rune,
        (Some("--host"), Some("rune"), None) => Host::Rune,
        (Some("--host"), Some("venus"), None) => Host::Venus,
        _ => {
            eprintln!("usage: venus-rune-candidate [--host rune|venus]");
            return ExitCode::from(2);
        }
    };
    match run(host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("venus-rune-candidate: {error}");
            ExitCode::from(1)
        }
    }
}
