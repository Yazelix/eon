//! The Files pair's two screens (EON-C18). Each pair Yazi gets a DDS client
//! ID from its terminal's endpoint, and its quick search's fzf marks that it
//! holds the terminal by creating a marker file beside that endpoint, which
//! the plugin removes once fzf ends. Alt+Z and Alt+Shift+Y switch screens: `ya
//! emit-to` shows quick search, and a reserved key that produces no text,
//! written through Orbit's lease input, leaves it for Files. Quick search runs
//! no listener (EON-C10).

use super::{
    diagnostics::DiagnosticLog,
    sessions::{self, RunningSession},
};
use eon_workspace_protocol::v47::{InvokeIntent, PROJECT_ENTRY};
use orbit_protocol::{
    management::LeaseInput,
    session::{Modifiers, PhysicalKey},
};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

/// How long after it can start quick search counts as shown before its fzf
/// marks it up. fzf takes about 0.6 s to start; a key in that time acts on the
/// screen that is coming, not on Files. Not user-visible, so fixed.
const STARTUP_GRACE: Duration = Duration::from_secs(2);

/// How often a leave writes its key again while fzf has not ended: about ten
/// times fzf's own leave (become, `sh`, exit). Internal recovery timing.
const RESEND: Duration = Duration::from_millis(250);

/// How long a leave keeps writing its key before it gives up and says so.
/// Internal recovery timing.
const LEAVE_BOUND: Duration = Duration::from_secs(2);

/// The longest the window's first pair waits for the other-windows notice
/// before its Yazi starts (cli.rs `startup_notice`).
const NOTICE_WAIT: Duration = Duration::from_secs(1);

/// The longest the startup Anima in the window's first pair runs past its
/// duration: eon-files stops it 2 s late and waits 0.25 s more for it to end
/// (cli.rs `play_startup_anima`).
const ANIMA_OVERRUN: Duration = Duration::from_millis(2250);

/// The key that leaves quick search for Files: Ctrl+Alt+Shift+PageDown. It
/// produces no text and no packaged Yazi binding uses it, so a copy that
/// reaches Yazi instead of fzf changes nothing there.
const LEAVE_KEY: (PhysicalKey, Modifiers) = (
    PhysicalKey::PAGE_DOWN,
    Modifiers::CTRL
        .union(Modifiers::ALT)
        .union(Modifiers::SHIFT),
);

/// The lease input that leaves quick search.
pub(crate) fn leave_input() -> LeaseInput {
    LeaseInput::key(LEAVE_KEY.0, LEAVE_KEY.1)
}

/// The fzf arguments quick search adds, one per line in
/// `EON_QUICK_FZF_ARGS`: fzf's start action, which runs once fzf holds the
/// terminal in raw mode and before it reads input, creates the marker at
/// `$EON_QUICK_MARKER`, and the leave key ends fzf with status 11. Both run
/// under `/bin/sh`, whatever the person's shell.
pub(crate) const FZF_ARGUMENTS: [&str; 3] = [
    "--with-shell=/bin/sh -c",
    "--bind=start:execute-silent(: > \"$EON_QUICK_MARKER\")",
    "--bind=ctrl-alt-shift-page-down:become(exit 11)",
];

/// The Yazi DDS client ID of the pair whose terminal listens at `endpoint`.
/// The endpoint names the window, generation and terminal, so pairs of
/// different windows differ. Yazi's own IDs are microsecond timestamps, which
/// reach 2^62 only in some 146,000 years; these lie in [2^62, 2^63), which Lua
/// plugins still read as integers.
pub(crate) fn client_id(endpoint: &Path) -> u64 {
    // FNV-1a, which every Eon build computes alike.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in endpoint.as_os_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    1 << 62 | hash & ((1 << 62) - 1)
}

/// The marker the quick search of the pair at `endpoint` creates while its
/// fzf runs: a regular file beside that endpoint, in the window's own runtime
/// directory, so `terminal-N.sock` pairs with `terminal-N.quick`.
pub(crate) fn marker(endpoint: &Path) -> PathBuf {
    let bytes = endpoint.as_os_str().as_bytes();
    let mut path = OsString::from(OsStr::from_bytes(
        bytes.strip_suffix(b".sock").unwrap_or(bytes),
    ));
    path.push(".quick");
    path.into()
}

/// The terminal endpoint whose pair marker is at `path`, if `path` names one;
/// never a terminal endpoint itself.
pub(crate) fn marker_endpoint(path: &Path) -> Option<PathBuf> {
    let stem = path.as_os_str().as_bytes().strip_suffix(b".quick")?;
    Some(PathBuf::from(OsStr::from_bytes(&[stem, b".sock"].concat())))
}

/// The marker's file identity, while it exists.
fn marker_identity(marker: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::symlink_metadata(marker).ok()?;
    metadata
        .file_type()
        .is_file()
        .then(|| (metadata.dev(), metadata.ino()))
}

/// Per pair, by marker: when the startup grace of a requested quick search
/// ends, until its fzf marks it up, the pair is hidden or picks, or it ends;
/// and its leave, until fzf has ended or the leave gives up.
#[derive(Default)]
struct Pairs {
    starting: HashMap<PathBuf, Instant>,
    leaving: HashMap<PathBuf, Leave>,
}

static PAIRS: Mutex<Option<Pairs>> = Mutex::new(None);

fn pairs() -> impl std::ops::DerefMut<Target = Pairs> {
    struct Guard(MutexGuard<'static, Option<Pairs>>);
    impl std::ops::Deref for Guard {
        type Target = Pairs;
        fn deref(&self) -> &Pairs {
            self.0.as_ref().expect("initialized")
        }
    }
    impl std::ops::DerefMut for Guard {
        fn deref_mut(&mut self) -> &mut Pairs {
            self.0.as_mut().expect("initialized")
        }
    }
    let mut guard = PAIRS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    guard
        .get_or_insert_with(Pairs::default)
        .starting
        .retain(|_, end| now < *end);
    Guard(guard)
}

/// How long the window's first pair takes before its quick search can start:
/// its startup Anima of `anima_seconds`, if any, then the notice.
pub(crate) fn fresh_start(anima_seconds: Option<u64>) -> Duration {
    NOTICE_WAIT
        + anima_seconds.map_or(Duration::ZERO, |seconds| {
            Duration::from_secs(seconds) + ANIMA_OVERRUN
        })
}

/// Records that the pair at `marker` was asked for quick search, which can
/// start after `delay`: started on it, or sent a switch to it. A leave still
/// pending for it is over.
pub(crate) fn quick_search_requested(marker: &Path, delay: Duration) {
    let mut pairs = pairs();
    pairs.leaving.remove(marker);
    pairs
        .starting
        .insert(marker.to_path_buf(), Instant::now() + delay + STARTUP_GRACE);
}

/// Ends the startup grace of the pair at `marker`: its quick search is up,
/// or it has ended.
pub(crate) fn quick_search_settled(marker: &Path) {
    pairs().starting.remove(marker);
}

/// Whether any pair's quick search is starting or leaving.
pub(crate) fn any_pending() -> bool {
    let pairs = pairs();
    !pairs.starting.is_empty() || !pairs.leaving.is_empty()
}

/// Ends the startup grace of the pair at `marker` once its fzf has marked it
/// up or the pair is not `shown`: a key can then no longer act on a quick
/// search still to come.
pub(crate) fn settle_starting(marker: &Path, shown: bool) {
    let mut pairs = pairs();
    if pairs.starting.contains_key(marker) && (!shown || marker_identity(marker).is_some()) {
        pairs.starting.remove(marker);
    }
}

/// The end of the startup grace of a quick search requested of `marker` whose
/// fzf has not marked it up yet.
pub(crate) fn starting(marker: &Path) -> Option<Instant> {
    pairs().starting.get(marker).copied()
}

/// Whether quick search is shown: its fzf has marked it up, or it was
/// requested within the startup grace and is still starting.
pub(crate) fn quick_search_shown(marker: &Path) -> bool {
    if marker_identity(marker).is_some() {
        quick_search_settled(marker);
        return true;
    }
    starting(marker).is_some()
}

/// A leave of quick search, until fzf has ended. It waits, while quick search
/// starts, for fzf to mark itself up, so a marker not there yet never reads as
/// fzf having ended; once it has seen the marker, it writes the key, and
/// again every `RESEND`, until that marker is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leave {
    Waiting {
        deadline: Instant,
    },
    Sending {
        marker: (u64, u64),
        first: Instant,
        last: Instant,
        /// Times the key was asked to be written.
        attempts: u32,
        /// Times Orbit wrote it.
        written: u32,
    },
}

/// What a pair's leave asks of the supervisor this turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeaveStep {
    /// Nothing yet.
    Wait,
    /// Write the leave key into the pair's terminal.
    Send,
    /// fzf has ended; the leave is over.
    Done,
    /// The leave gave up; the detail says why.
    Expired(String),
}

impl Leave {
    /// One turn of the leave, with the marker's identity as it is now.
    fn step(&mut self, marker: Option<(u64, u64)>, now: Instant) -> LeaveStep {
        match *self {
            Self::Waiting { deadline } => match marker {
                Some(marker) => {
                    *self = Self::Sending {
                        marker,
                        first: now,
                        last: now,
                        attempts: 1,
                        written: 0,
                    };
                    LeaveStep::Send
                }
                None if now >= deadline => LeaveStep::Expired("quick search never started".into()),
                None => LeaveStep::Wait,
            },
            Self::Sending {
                marker: seen,
                first,
                last,
                attempts,
                written,
            } => {
                if marker != Some(seen) {
                    LeaveStep::Done
                } else if now.duration_since(first) >= LEAVE_BOUND {
                    LeaveStep::Expired(format!(
                        "quick search did not leave within {} s; Orbit wrote its key {written} of {attempts} times",
                        LEAVE_BOUND.as_secs()
                    ))
                } else if now.duration_since(last) >= RESEND {
                    *self = Self::Sending {
                        marker: seen,
                        first,
                        last: now,
                        attempts: attempts + 1,
                        written,
                    };
                    LeaveStep::Send
                } else {
                    LeaveStep::Wait
                }
            }
        }
    }
}

impl Leave {
    /// Orbit wrote the key this turn's `Send` asked for.
    fn wrote(&mut self) {
        if let Self::Sending { written, .. } = self {
            *written += 1;
        }
    }
}

/// Records that Orbit wrote the leave key of the pair at `marker`.
fn leave_written(marker: &Path) {
    if let Some(leave) = pairs().leaving.get_mut(marker) {
        leave.wrote();
    }
}

/// Asks the pair at `marker` to leave quick search for Files. A leave already
/// pending stays as it is; one for a quick search still starting waits until
/// the end of its startup grace for fzf to mark it up.
pub(crate) fn leave_requested(marker: &Path) {
    let mut pairs = pairs();
    let deadline = pairs
        .starting
        .get(marker)
        .copied()
        .unwrap_or_else(Instant::now);
    pairs
        .leaving
        .entry(marker.to_path_buf())
        .or_insert(Leave::Waiting { deadline });
}

/// Advances the leave of the pair at `marker`, if it has one, this turn. A
/// pair no longer `shown` has nothing to leave. A finished or expired leave
/// is forgotten.
pub(crate) fn advance_leave(marker: &Path, shown: bool, now: Instant) -> Option<LeaveStep> {
    let mut pairs = pairs();
    let leave = pairs.leaving.get_mut(marker)?;
    let step = if shown {
        leave.step(marker_identity(marker), now)
    } else {
        LeaveStep::Done
    };
    if matches!(step, LeaveStep::Done | LeaveStep::Expired(_)) {
        pairs.leaving.remove(marker);
    }
    Some(step)
}

/// One turn of the pending leave of the pair in `session` (EON-C18): writes
/// the leave key through the lease, with no quiet window, while fzf has not
/// ended. A person typing or a full input queue keeps the leave for its next
/// copy; any other failure ends it. A leave that ends without fzf ending is
/// logged. Returns the turn's step, or none when no leave is pending.
pub(crate) fn drive_leave(
    session: &mut RunningSession,
    shown: bool,
    now: Instant,
    log: &DiagnosticLog,
) -> Option<LeaveStep> {
    let marker = marker(&session.endpoint);
    let step = advance_leave(&marker, shown, now)?;
    match &step {
        LeaveStep::Send => match sessions::send_input(session, leave_input(), 0, log) {
            Ok(_) => leave_written(&marker),
            Err(failure) if sessions::input_refused_for_now(&failure) => {}
            Err(failure) => {
                forget_leave(&marker);
                log.record(&format!(
                    "pair {} did not take its quick search leave key: {}",
                    session.id, failure.detail
                ));
            }
        },
        LeaveStep::Expired(detail) => log.record(&format!("pair {}: {detail}", session.id)),
        LeaveStep::Wait | LeaveStep::Done => {}
    }
    Some(step)
}

/// Forgets the leave of the pair at `marker`: writing its key failed for good.
pub(crate) fn forget_leave(marker: &Path) {
    pairs().leaving.remove(marker);
}

/// Forgets the leaves of pairs whose markers `live` does not name: their
/// terminals ended.
pub(crate) fn retain_leaves(live: &[PathBuf]) {
    pairs().leaving.retain(|marker, _| live.contains(marker));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Switch {
    None,
    ShowQuickSearch,
    LeaveQuickSearch,
}

/// The intent to dispatch for invoking `entry` on a pair that is `shown` with
/// quick search up or not, and the switch to run once it is accepted. Alt+Z
/// (Project) wants quick search and Alt+Shift+Y (Files) wants Files; a key on
/// its own shown screen keeps the requested intent, which hides the pair on
/// Toggle, and on the other screen switches and keeps the pair shown. A quick
/// search still `starting` is not shown yet: Alt+Z does not ask for it again,
/// and shows the pair on it rather than hiding it.
pub(crate) fn plan(
    entry: &str,
    intent: InvokeIntent,
    shown: bool,
    quick: bool,
    starting: bool,
) -> (InvokeIntent, Switch) {
    let wants_quick = entry == PROJECT_ENTRY;
    let switch = match (wants_quick, quick) {
        (true, false) => Switch::ShowQuickSearch,
        (false, true) => Switch::LeaveQuickSearch,
        _ => Switch::None,
    };
    if shown && (switch != Switch::None || wants_quick && starting) {
        (InvokeIntent::Focus, switch)
    } else if shown {
        (intent, Switch::None)
    } else {
        (intent, switch)
    }
}

/// Runs `switch` for the pair with client `id` and quick search `marker`
/// beside the supervisor; a failure is logged and changes nothing else. A
/// leave is written by the supervisor's turns (`advance_leave`).
pub(crate) fn run(
    switch: Switch,
    ya: Option<&Path>,
    id: u64,
    marker: PathBuf,
    log: &Arc<DiagnosticLog>,
) {
    match switch {
        Switch::None => {}
        Switch::LeaveQuickSearch => leave_requested(&marker),
        Switch::ShowQuickSearch => {
            // A second quick search would wait behind the shown one (EON-C18).
            if quick_search_shown(&marker) {
                return;
            }
            quick_search_requested(&marker, Duration::ZERO);
            let Some(ya) = ya else {
                return log.record("pair switch skipped: this Eon has no packaged ya");
            };
            let mut command = Command::new(ya);
            command
                .args(["emit-to", &id.to_string(), "plugin", "quick-search"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let worker = Arc::clone(log);
            if let Err(error) = thread::Builder::new().spawn(move || {
                let failure = match command.status() {
                    Ok(status) if status.success() => return,
                    Ok(status) => format!("ya emit-to {status}"),
                    Err(error) => error.to_string(),
                };
                worker.record(&format!("pair switch to quick search failed: {failure}"));
            }) {
                log.record(&format!("pair switch failed: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_are_stable_distinct_and_outside_timestamps() {
        let endpoint = Path::new("/run/eon/w/0000000a/generations/g1/terminal-2.sock");
        let id = client_id(endpoint);
        assert_eq!(id, client_id(endpoint));
        assert_ne!(
            id,
            client_id(Path::new(
                "/run/eon/w/0000000a/generations/g1/terminal-3.sock"
            ))
        );
        assert_ne!(
            id,
            client_id(Path::new(
                "/run/eon/w/0000000a/generations/g2/terminal-2.sock"
            ))
        );
        for id in [id, client_id(Path::new("")), client_id(Path::new("/s"))] {
            assert!((1 << 62..1 << 63).contains(&id), "{id}");
        }
    }

    /// Two windows of one build share the generation name, terminal numbering
    /// and runtime base; their pairs still get their own client IDs and
    /// markers, each in its own window's directory.
    #[test]
    fn pairs_of_two_windows_sharing_a_runtime_base_do_not_collide() {
        let base = Path::new("/run/user/1000/eon");
        let generation = crate::generation::generation_directory;
        let first = generation(&base.join("w/0000000a"), "g1").join("terminal-2.sock");
        let second = generation(&base.join("w/0000000b"), "g1").join("terminal-2.sock");
        assert_ne!(client_id(&first), client_id(&second));
        assert_ne!(marker(&first), marker(&second));
        for endpoint in [&first, &second] {
            assert_eq!(marker(endpoint).parent(), endpoint.parent());
        }
        assert_eq!(marker(&first).file_name().unwrap(), "terminal-2.quick");
    }

    /// A test's own directory, empty.
    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("eon-pair-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// The recovery sweep maps a marker back to its terminal and never takes
    /// a terminal endpoint, record or management socket for a marker.
    #[test]
    fn a_marker_maps_back_to_its_terminal_only() {
        let endpoint = Path::new("/run/eon/w/0000000a/generations/g1/terminal-2.sock");
        let pair = marker(endpoint);
        assert_eq!(pair.file_name().unwrap(), "terminal-2.quick");
        assert_eq!(marker_endpoint(&pair).as_deref(), Some(endpoint));
        for other in [
            "terminal-2.sock",
            "orbit.sock",
            "terminal-2.sock.pending",
            "terminal-2.sock.record",
            "terminal-2.sock.management",
            "terminal-2.quick.sock",
        ] {
            assert_eq!(marker_endpoint(&endpoint.with_file_name(other)), None);
        }
    }

    /// A starting quick search stops counting as shown once its pair is
    /// hidden, so Alt+Z, Esc, Alt+Z asks for quick search again, or once its
    /// fzf marks it up, so a quick search left with Tab is not taken for
    /// starting. Only a regular file is a marker.
    #[test]
    fn hiding_the_pair_or_its_fzf_marking_it_up_ends_the_startup_grace() {
        let root = scratch("settle");
        let path = root.join("terminal-2.quick");
        assert!(!quick_search_shown(&path));
        quick_search_requested(&path, Duration::ZERO);
        assert!(quick_search_shown(&path));
        settle_starting(&path, true);
        assert!(quick_search_shown(&path));
        settle_starting(&path, false);
        assert!(!quick_search_shown(&path));
        assert_eq!(
            plan(PROJECT_ENTRY, InvokeIntent::Toggle, false, false, false),
            (InvokeIntent::Toggle, Switch::ShowQuickSearch)
        );
        quick_search_requested(&path, Duration::ZERO);
        std::fs::create_dir(&path).unwrap();
        settle_starting(&path, true);
        assert!(starting(&path).is_some(), "a directory marked it up");
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"").unwrap();
        settle_starting(&path, true);
        assert!(starting(&path).is_none());
        assert!(quick_search_shown(&path));
        std::fs::remove_file(&path).unwrap();
        assert!(!quick_search_shown(&path));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A leave waits for fzf to mark itself up, then writes its key at once
    /// and every `RESEND` until that marker is gone; a marker removed and
    /// made again is a later quick search, which the leave does not touch.
    #[test]
    fn a_leave_sends_until_fzf_ends_and_never_reads_a_missing_marker_as_ended() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let up = Some((1, 7));
        let mut leave = Leave::Waiting { deadline: at(2000) };
        assert_eq!(leave.step(None, at(0)), LeaveStep::Wait);
        assert_eq!(leave.step(None, at(1999)), LeaveStep::Wait);
        assert_eq!(leave.step(up, at(1999)), LeaveStep::Send);
        assert_eq!(leave.step(up, at(2100)), LeaveStep::Wait);
        assert_eq!(leave.step(up, at(2249)), LeaveStep::Send);
        assert_eq!(leave.step(up, at(2300)), LeaveStep::Wait);
        assert_eq!(leave.step(None, at(2300)), LeaveStep::Done);

        let mut leave = Leave::Waiting { deadline: at(0) };
        assert_eq!(leave.step(up, at(5)), LeaveStep::Send);
        assert_eq!(leave.step(Some((1, 8)), at(10)), LeaveStep::Done);

        let mut never = Leave::Waiting { deadline: at(100) };
        assert_eq!(
            never.step(None, at(100)),
            LeaveStep::Expired("quick search never started".into())
        );
    }

    /// A leave that never sees fzf end gives up after `LEAVE_BOUND`, having
    /// asked for its key every `RESEND`, and says how often Orbit wrote it:
    /// here every other time, the rest refused for now.
    #[test]
    fn a_leave_gives_up_after_its_bound_and_says_so() {
        let start = Instant::now();
        let up = Some((1, 7));
        let mut leave = Leave::Waiting { deadline: start };
        let mut sent = 0;
        let mut now = start;
        let expired = loop {
            match leave.step(up, now) {
                LeaveStep::Send => {
                    sent += 1;
                    if sent % 2 == 0 {
                        leave.wrote();
                    }
                }
                LeaveStep::Wait => {}
                LeaveStep::Expired(detail) => break detail,
                LeaveStep::Done => panic!("done with the marker still there"),
            }
            now += Duration::from_millis(25);
        };
        assert_eq!(now - start, LEAVE_BOUND);
        assert_eq!(sent, 8);
        assert_eq!(
            expired,
            "quick search did not leave within 2 s; Orbit wrote its key 4 of 8 times"
        );
    }

    /// A leave asked for twice keeps its first state; one asked for while
    /// quick search starts waits until its grace ends; asking for quick search
    /// again, the pair hiding, or its terminal ending are each the end of it.
    #[test]
    fn a_pending_leave_is_kept_once_and_ended_by_hiding_or_a_new_quick_search() {
        let root = scratch("leave");
        let path = root.join("terminal-2.quick");
        let now = Instant::now();
        assert_eq!(advance_leave(&path, true, now), None);
        quick_search_requested(&path, Duration::from_secs(1));
        leave_requested(&path);
        assert_eq!(advance_leave(&path, true, now), Some(LeaveStep::Wait));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(advance_leave(&path, true, now), Some(LeaveStep::Send));
        leave_requested(&path);
        assert_eq!(advance_leave(&path, true, now), Some(LeaveStep::Wait));
        assert!(any_pending());
        assert_eq!(advance_leave(&path, false, now), Some(LeaveStep::Done));
        assert_eq!(advance_leave(&path, true, now), None);

        leave_requested(&path);
        quick_search_requested(&path, Duration::ZERO);
        assert_eq!(advance_leave(&path, true, now), None);
        quick_search_settled(&path);

        leave_requested(&path);
        retain_leaves(&[]);
        assert_eq!(advance_leave(&path, true, now), None);
        leave_requested(&path);
        forget_leave(&path);
        assert_eq!(advance_leave(&path, true, now), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The fzf arguments quick search adds: its start marks it up and the
    /// reserved key ends it with status 11, both under `/bin/sh`; and the key
    /// Runtime writes is the one fzf binds, which Orbit's lease takes.
    #[test]
    fn the_leave_key_is_the_one_fzf_binds_and_the_lease_takes() {
        assert_eq!(FZF_ARGUMENTS[0], "--with-shell=/bin/sh -c");
        assert!(FZF_ARGUMENTS[1].starts_with("--bind=start:execute-silent("));
        assert!(FZF_ARGUMENTS[1].contains("\"$EON_QUICK_MARKER\""));
        assert_eq!(
            FZF_ARGUMENTS[2],
            "--bind=ctrl-alt-shift-page-down:become(exit 11)"
        );
        let LeaseInput::Key { key, modifiers } = leave_input() else {
            panic!("the leave is a key");
        };
        assert_eq!(
            LeaseInput::allowed_key(key, modifiers),
            Some((
                PhysicalKey::PAGE_DOWN,
                Modifiers::CTRL
                    .union(Modifiers::ALT)
                    .union(Modifiers::SHIFT)
            ))
        );
        for argument in FZF_ARGUMENTS {
            assert!(!argument.contains('\n'));
        }
    }

    #[test]
    fn hotkeys_switch_screens_or_hide_on_their_own_screen() {
        use InvokeIntent::{Focus, Toggle};
        use Switch::{LeaveQuickSearch as Leave, None, ShowQuickSearch as Show};
        // (entry, intent, shown, quick, starting) → (dispatched intent, switch)
        let cases = [
            // Alt+Z: hidden shows quick search, unless it is already up.
            (PROJECT_ENTRY, Toggle, false, false, false, Toggle, Show),
            (PROJECT_ENTRY, Toggle, false, true, false, Toggle, None),
            // On quick search it hides; on Files it switches and stays shown.
            (PROJECT_ENTRY, Toggle, true, true, false, Toggle, None),
            (PROJECT_ENTRY, Toggle, true, false, false, Focus, Show),
            (PROJECT_ENTRY, Focus, true, true, false, Focus, None),
            // A starting quick search is not asked for again, and Alt+Z shows
            // the pair on it, never hides it.
            (PROJECT_ENTRY, Toggle, false, true, true, Toggle, None),
            (PROJECT_ENTRY, Toggle, true, true, true, Focus, None),
            // Alt+Shift+Y: hidden shows Files, leaving a quick search left up.
            ("files", Toggle, false, false, false, Toggle, None),
            ("files", Toggle, false, true, false, Toggle, Leave),
            // On Files it hides; on quick search it switches and stays shown.
            ("files", Toggle, true, false, false, Toggle, None),
            ("files", Toggle, true, true, false, Focus, Leave),
            ("files", Focus, true, false, false, Focus, None),
            // A starting quick search is left once fzf marks it up.
            ("files", Toggle, false, true, true, Toggle, Leave),
            ("files", Toggle, true, true, true, Focus, Leave),
        ];
        for (entry, intent, shown, quick, starting, dispatched, switch) in cases {
            assert_eq!(
                plan(entry, intent, shown, quick, starting),
                (dispatched, switch),
                "{entry} {intent:?} shown={shown} quick={quick} starting={starting}"
            );
        }
    }
}

#[cfg(test)]
mod real_pty;
