//! Private Rune host candidate entry. Built only with the non-default `candidate-rune` feature.
//!
//! - `venus-rune-candidate [--fixture header-pilot] [--host rune|venus]` runs the
//!   canonical fixtures with `EON_VENUS_PRESENTATION_CONTROL` unset. Each stdin line
//!   advances one step. Presentations print `drawn CYCLE STEP NAME`. The header pilot
//!   appends `WIDTH HEIGHT SCALE FOLLOW_UPS`. Exit 0 on window close, 1 on render, GPU,
//!   geometry or admission failure, and 2 on refusal or invalid arguments.
//! - Any other invocation (eon-yp6vg.7.9) takes `yazelix-venus`'s own launch arguments and
//!   presentation control, and runs Venus's `Application` with Rune as the native host. Exit 1
//!   on a startup, render or admission failure, as `yazelix-venus`.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use yazelix_venus::rune_candidate::{Host, run, run_header_pilot};

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
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let (pilot, flags) = match arguments.as_slice() {
        [flag, name, rest @ ..] if flag == "--fixture" && name == "header-pilot" => (true, rest),
        flags => (false, flags),
    };
    let host = match flags {
        [flag, host] if flag == "--host" => host.to_str(),
        [] if !pilot && std::env::var_os("EON_VENUS_PRESENTATION_CONTROL").is_none() => {
            Some("rune")
        }
        _ => None,
    };
    let fixture = match host {
        Some("rune") => Some(Host::Rune),
        Some("venus") => Some(Host::Venus),
        _ if pilot || matches!(flags, [flag, _] if flag == "--host") => {
            eprintln!("usage: venus-rune-candidate [--fixture header-pilot] --host rune|venus");
            return ExitCode::from(2);
        }
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
    match if pilot {
        run_header_pilot(host)
    } else {
        run(host)
    } {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("venus-rune-candidate: {error}");
            ExitCode::from(1)
        }
    }
}
