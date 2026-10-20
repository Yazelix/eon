//! The real-PTY checks: each starts the Orbit at `ORBIT_BIN` on
//! Orbit's command line, drives it through ORBS v13 and management
//! messages encoded and decoded by the canonical `orbit-protocol` codec, and
//! checks how the session, its child and its socket end. The differential
//! check also runs the old Orbit, at `OLD_ORBIT_BIN`, and compares.
//!
//! The checks are ignored by default, so `cargo test` stays quick; nix/orbit-checks.nix
//! runs each as its own check, `real-pty --ignored --exact NAME`. A check's
//! child script, tests/NAME-shell.sh, comes from the variable nix/orbit-checks.nix sets,
//! or from the source tree.

mod support;

macro_rules! checks {
    ($($check:ident),* $(,)?) => {$(
        mod $check;

        #[test]
        #[ignore = "drives the Orbit at ORBIT_BIN through a real PTY"]
        fn $check() {
            support::serial(stringify!($check), $check::run);
        }
    )*};
}

checks!(
    clipboard,
    clipboard_pressure,
    command_line,
    delayed_output,
    descendant_exit,
    detached_exit,
    detached_observer,
    differential,
    guardian_containment,
    held_attach,
    history,
    interactive_pressure,
    key_live,
    late_observer,
    lifecycle,
    management_lease,
    management_record,
    mouse,
    observer_finish,
    palette_reset,
    paste_flood,
    preview,
    pty_reopen,
    refused_flush,
    request_half_close,
    return_to_live,
    schema_mismatch,
    scroll,
    selection,
    selection_tick,
    silent_peer,
    slow_observer,
    stale_socket,
    stopped_client_exit,
    synchronized_output,
    terminal_identity,
    terminal_replies,
    three_attach,
    unsupported,
);
