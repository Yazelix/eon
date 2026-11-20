//! The seam between the slot queue and a host's remote route (Eon's Boat).
//!
//! Standalone rota has no route: every command runs on a local slot. A host passes one
//! through [`crate::cli::Host::route`]; the queue then asks it where each command goes,
//! lets it take a command when that command heads the remote queue, and runs the
//! commands it keeps local through it.
use super::memory::Limit;
use crate::config::SlotConfig;
use crate::platform::{ROUTE_SIGNAL, RouteSignals};
use rustix::process::Pid;
use serde_json::Value;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsFd;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::atomic::Ordering;

/// A host's remote route for `slot run`.
///
/// A route may prepare a *source* (for Boat, the identity of the exact tree it captured)
/// and hand it back with a fallback. The queue keeps it opaque: it stores it, writes it
/// into the `_slot-local-exec` handoff, and gives it back to [`Route::verify`] and
/// [`Route::run_local`]. A source is any JSON value except `null`; it reaches the route
/// again as `serde_json` re-reads its own text, so routes keep floats out of it.
pub trait Route {
    /// Reads and checks the route's own keys (the core keeps `[slots.boat]` as an opaque
    /// table) from the whole configuration file's `text`, right after the core has parsed
    /// it, for every command. An error refuses the file (exit 2); slot commands print the
    /// warnings.
    fn check_config(&self, text: &str) -> Result<Vec<String>, String> {
        let _ = text;
        Ok(Vec::new())
    }

    /// The `where` value of the commands it runs (`slot ls`, the event log), and the name
    /// of its state directory under the slot root.
    fn name(&self) -> &'static str;

    /// Where a command goes: when it queues ([`Stage::Queued`]), and again on each wait
    /// while it is still bound for the route ([`Stage::Waiting`]).
    fn place(&self, request: &Request<'_>, stage: Stage) -> Placement;

    /// Tries the remote run once this command heads the remote queue and no quiet window
    /// holds it.
    ///
    /// Returns [`Remote::Wait`] only before calling `granted`. `granted` is called once,
    /// when the route holds its capacity and before the command starts; the queue then
    /// logs the `grant` event and releases its queue lock. A [`Remote::Failed`] before
    /// `granted` exits 125 with no `grant` event. The route may fill `source`; it follows
    /// a [`Remote::Local`] fallback into the local run.
    fn run_remote(
        &self,
        request: &Request<'_>,
        granted: &mut dyn FnMut(),
        source: &mut Option<Value>,
    ) -> Remote;

    /// Runs a command the route kept local (a fallback other than
    /// [`Kind::Declined`]), or the `local_runner` delegate for any command.
    fn run_local(&self, local: Local<'_>) -> io::Result<ExitStatus>;

    /// Checks a prepared source against the working directory, right before the command
    /// itself runs. An error refuses the command (exit 125).
    fn verify(&self, source: &Value) -> Result<(), String>;

    /// The route's part of `slot ls`. `lock_owner` answers for a lock file's inode:
    /// `None` when nobody holds it, `Some(pid)` when someone does (`pid` when the
    /// kernel names them). An error fails `slot ls`.
    fn listing(
        &self,
        root: &Path,
        lock_owner: &dyn Fn(&fs::Metadata) -> Option<Option<u32>>,
    ) -> Result<Listing, String>;
}

/// One `slot run` command, as the route sees it.
pub struct Request<'a> {
    pub config: &'a SlotConfig,
    /// The slot state root (`<state dir>/slots`).
    pub root: &'a Path,
    pub argv: &'a [String],
    pub quiet_exempt: bool,
    /// The command's memory class and cap, read again on each wait.
    pub limit: Limit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Queued,
    /// `limit_changed`: the memory limit differs from the one seen on the previous check.
    Waiting {
        limit_changed: bool,
    },
}

#[derive(Debug)]
pub enum Placement {
    /// The route takes the command when its turn comes.
    Remote,
    /// The command runs locally; `None` when the route has nothing to say about it.
    Local(Option<Fallback>),
}

/// Why a command the route saw runs locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    /// Recorded as the ticket's `fallback_reason`.
    pub reason: String,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The route never takes this command: it runs as any local slot command.
    Declined,
    /// The route is switched off: a local run through the route, with no note when the
    /// command queues.
    Off,
    /// Kept local by the route's policy: a local run through the route, noted.
    Kept,
    /// The remote attempt gave the command back: a local run through the route, and
    /// failures before it starts exit 125.
    Returned,
}

impl Kind {
    /// Whether the command runs through [`Route::run_local`] rather than as a plain child.
    pub fn routed(self) -> bool {
        self != Kind::Declined
    }
}

#[derive(Debug)]
pub enum Remote {
    /// Not now (capacity busy, or the route cannot start it yet): keep waiting.
    Wait,
    /// The command ran remotely and exited with `code`; `memory` joins the exit event.
    Exit { code: u8, memory: Option<Value> },
    /// The outcome is unknown or the route failed: exit 125.
    Failed {
        reason: String,
        memory: Option<Value>,
    },
    /// The route gave the command back to run locally.
    Local(Fallback),
}

/// A local run through the route.
pub struct Local<'a> {
    /// The slot state root.
    pub root: &'a Path,
    pub command: &'a mut Command,
    pub fallback: Option<&'a Fallback>,
    pub source: Option<&'a Value>,
    /// The host program's name, for message lines.
    pub program: &'a str,
}

/// The route's part of `slot ls`.
#[derive(Debug, Default)]
pub struct Listing {
    /// Lines printed before the slot holders.
    pub lines: Vec<String>,
    /// Entries appended to the JSON `holders`.
    pub holders: Vec<Value>,
    /// Fields added to the JSON object.
    pub fields: serde_json::Map<String, Value>,
    /// PIDs the route is running or holding capacity for; they are not listed as waiters.
    pub busy: Vec<u32>,
}

/// SIGINT, SIGTERM and SIGHUP caught while a route runs a command. One process-wide
/// handler: [`Signals::install`] clears the caught signal and installs it, and dropping the
/// guard restores the previous dispositions. Guards are used one at a time, never nested
/// or concurrently.
pub struct Signals {
    _installed: RouteSignals,
}

impl Signals {
    pub fn install() -> Self {
        Self {
            _installed: RouteSignals::install(),
        }
    }

    /// The last signal caught since [`Signals::install`], or 0.
    pub fn caught() -> i32 {
        ROUTE_SIGNAL.load(Ordering::Relaxed)
    }
}

/// Makes `group` the foreground process group of the terminal `tty`, ignoring SIGTTOU
/// for the call.
pub fn foreground(tty: impl AsFd, group: Pid) -> io::Result<()> {
    crate::platform::foreground(tty, group)
}

/// Makes `command`'s child the foreground process group of `tty` just before it runs.
pub fn foreground_on_exec(command: &mut Command, tty: &File) -> io::Result<()> {
    crate::platform::foreground_on_exec(command, tty)
}
