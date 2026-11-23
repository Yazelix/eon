//! Private Rune host candidate entry. Built only with the non-default `candidate-rune` feature.
//!
//! - `venus-rune-candidate [--host rune|venus]`, with `EON_VENUS_PRESENTATION_CONTROL` unset
//!   (eon-yp6vg.7.7): the canonical Venus grid fixtures in one Rune device and pass. Each stdin
//!   line advances one fixture step; after each step's first presentation it prints
//!   `drawn <cycle> <step> <name>`. Exit 0 when the window closes, 1 on a render, GPU, geometry
//!   or admission failure, 2 on refusal or invalid arguments.
//! - Any other invocation (eon-yp6vg.7.9) takes `yazelix-venus`'s own launch arguments and
//!   presentation control, and runs Venus's `Application` with Rune as the native host. Exit 1
//!   on a startup, render or admission failure, as `yazelix-venus`.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use yazelix_venus::rune_candidate::{Host, run};

fn main() -> ExitCode {
    // Both candidate hosts need a native Wayland display. Refuse before creating an event loop.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|display| display.is_empty()) {
        eprintln!(
            "venus-rune-candidate needs a native Wayland display: WAYLAND_DISPLAY is not set \
             (X11 is not supported)"
        );
        return ExitCode::from(2);
    }
    let mut arguments = std::env::args_os().skip(1);
    let fixture = match (arguments.next(), arguments.next(), arguments.next()) {
        (None, ..) if std::env::var_os("EON_VENUS_PRESENTATION_CONTROL").is_none() => {
            Some(Host::Rune)
        }
        (Some(flag), Some(host), None) if flag == "--host" => match host.to_str() {
            Some("rune") => Some(Host::Rune),
            Some("venus") => Some(Host::Venus),
            _ => {
                eprintln!("usage: venus-rune-candidate [--host rune|venus]");
                return ExitCode::from(2);
            }
        },
        _ => None,
    };
    let Some(host) = fixture else {
        return match yazelix_venus::run_rune_candidate() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("venus-rune-candidate: {error}");
                ExitCode::from(1)
            }
        };
    };
    match run(host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("venus-rune-candidate: {error}");
            ExitCode::from(1)
        }
    }
}
