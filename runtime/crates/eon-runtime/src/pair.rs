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
use eon_workspace_protocol::v48::{InvokeIntent, PROJECT_ENTRY};
use orbit_protocol::{
    management::LeaseInput,
    session::{Modifiers, PhysicalKey},
};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, LazyLock, Mutex, MutexGuard, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Failure recovery for a launcher or plugin that cannot settle its request.
const START_RECOVERY: Duration = Duration::from_secs(45);
pub(crate) const INITIAL_REQUEST: &str = "initial";

/// How often a leave writes its key again while fzf has not ended: about ten
/// times fzf's own leave (become, `sh`, exit). Internal recovery timing.
const RESEND: Duration = Duration::from_millis(250);

/// How long a leave keeps writing its key before it gives up and says so.
/// Internal recovery timing.
const LEAVE_BOUND: Duration = Duration::from_secs(2);

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
/// terminal in raw mode and before it reads input, writes the request token
/// to `$EON_QUICK_MARKER`, and the leave key ends fzf with status 11. Both run
/// under `/bin/sh`, whatever the person's shell.
pub(crate) const FZF_ARGUMENTS: [&str; 3] = [
    "--with-shell=/bin/sh -c",
    "--bind=start:execute-silent(printf '%s' \"$EON_QUICK_TOKEN\" > \"$EON_QUICK_MARKER\")",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkerKind {
    Ready,
    Pending,
}

/// Names one startup request so a late cleanup cannot remove a later one.
fn pending_marker(marker: &Path, token: &str) -> PathBuf {
    let mut path = marker.as_os_str().to_os_string();
    path.push(format!(".{token}.pending"));
    path.into()
}

pub(crate) fn marker_endpoint(path: &Path) -> Option<(PathBuf, MarkerKind)> {
    let bytes = path.as_os_str().as_bytes();
    let (stem, kind) = if let Some(stem) = bytes.strip_suffix(b".quick") {
        (stem, MarkerKind::Ready)
    } else {
        let name = bytes.strip_suffix(b".pending")?;
        let separator = name.iter().rposition(|&byte| byte == b'.')?;
        let (prefix, token) = name.split_at(separator);
        let token = &token[1..];
        if token != INITIAL_REQUEST.as_bytes()
            && (token.len() > 80
                || token.split(|&byte| byte == b'-').count() != 3
                || !token
                    .split(|&byte| byte == b'-')
                    .all(|part| !part.is_empty() && part.iter().all(u8::is_ascii_digit)))
        {
            return None;
        }
        (prefix.strip_suffix(b".quick")?, MarkerKind::Pending)
    };
    Some((
        PathBuf::from(OsStr::from_bytes(&[stem, b".sock"].concat())),
        kind,
    ))
}

pub(crate) fn remove_marker(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(format!(
            "cannot remove quick search marker {}: {error}",
            path.display()
        )),
        _ => Ok(()),
    }
}

fn marker_identity(marker: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(marker).ok()?;
    metadata
        .file_type()
        .is_file()
        .then(|| (metadata.dev(), metadata.ino()))
}

fn ready_for(marker: &Path, token: &str) -> bool {
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(marker)
    else {
        return false;
    };
    let mut bytes = Vec::new();
    file.take((token.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .is_ok()
        && bytes == token.as_bytes()
}

struct Startup {
    token: String,
    pending: PathBuf,
    deadline: Option<Instant>,
}

#[derive(Default)]
struct Pairs {
    starting: HashMap<PathBuf, Startup>,
    leaving: HashMap<PathBuf, Leave>,
}

static PAIRS: LazyLock<Mutex<Pairs>> = LazyLock::new(|| Mutex::new(Pairs::default()));

fn pairs() -> MutexGuard<'static, Pairs> {
    PAIRS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Pairs {
    fn finish_start(&mut self, marker: &Path) -> Result<(), String> {
        if let Some(startup) = self.starting.get(marker) {
            remove_marker(&startup.pending)?;
            self.starting.remove(marker);
        }
        Ok(())
    }

    fn hide(&mut self, marker: &Path, log: &DiagnosticLog) -> Option<LeaveStep> {
        if let Err(error) = self.finish_start(marker) {
            log.note_once(&error);
        }
        self.leaving.remove(marker).map(|_| LeaveStep::Done)
    }

    fn screen(&mut self, marker: &Path, now: Instant, log: &DiagnosticLog) -> Screen {
        let ready = marker_identity(marker);
        if let Some(startup) = self.starting.get(marker) {
            let waiting = marker_identity(&startup.pending).is_some()
                && !(ready.is_some() && ready_for(marker, &startup.token));
            if waiting {
                if startup.deadline.is_none_or(|deadline| now < deadline) {
                    return ready.map_or(Screen::Starting, Screen::Quick);
                }
                log.note_once(&format!(
                    "pair {}: quick search still pending after {} s without fzf readiness; clearing request {}",
                    marker.display(),
                    START_RECOVERY.as_secs(),
                    startup.token,
                ));
            }
            if let Err(error) = self.finish_start(marker) {
                log.note_once(&error);
            }
        }
        ready.map_or(Screen::Files, Screen::Quick)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Files,
    Starting,
    Quick((u64, u64)),
}

/// Registers a request before any asynchronous launch or dispatch.
pub(crate) fn quick_search_requested(
    marker: &Path,
    token: &str,
    admitted: Option<Instant>,
) -> Result<(), String> {
    let mut pairs = pairs();
    pairs.finish_start(marker)?;
    let pending = pending_marker(marker, token);
    fs::File::create_new(&pending).map_err(|error| {
        format!(
            "cannot record quick search request {}: {error}",
            pending.display()
        )
    })?;
    pairs.leaving.remove(marker);
    pairs.starting.insert(
        marker.to_path_buf(),
        Startup {
            token: token.to_owned(),
            pending,
            deadline: admitted.map(|now| now + START_RECOVERY),
        },
    );
    Ok(())
}

/// Starts the recovery clock once Orbit admits the initial launch.
pub(crate) fn quick_search_admitted(marker: &Path, now: Instant) {
    if let Some(startup) = pairs().starting.get_mut(marker) {
        startup.deadline = Some(now + START_RECOVERY);
    }
}

pub(crate) fn hide(marker: &Path, log: &DiagnosticLog) {
    pairs().hide(marker, log);
}

pub(crate) fn quick_search_settled(marker: &Path) -> Result<(), String> {
    pairs().finish_start(marker)
}

pub(crate) fn terminal_ended(endpoint: &Path) -> Result<(), String> {
    let marker = marker(endpoint);
    remove_marker(&marker)?;
    let mut pairs = pairs();
    pairs.finish_start(&marker)?;
    pairs.leaving.remove(&marker);
    Ok(())
}

pub(crate) fn any_pending() -> bool {
    let pairs = pairs();
    !pairs.starting.is_empty() || !pairs.leaving.is_empty()
}

/// Holds `marker`'s pair as starting until `forget`.
#[cfg(test)]
pub(crate) fn hold_starting(marker: &Path) {
    pairs().starting.insert(
        marker.to_owned(),
        Startup {
            token: "test".into(),
            pending: marker.with_extension("pending"),
            deadline: None,
        },
    );
}

/// Holds `marker`'s pair as leaving until `forget`.
#[cfg(test)]
pub(crate) fn hold_leaving(marker: &Path) {
    pairs().leaving.insert(marker.to_owned(), Leave::Waiting);
}

#[cfg(test)]
pub(crate) fn forget(marker: &Path) {
    let mut pairs = pairs();
    pairs.starting.remove(marker);
    pairs.leaving.remove(marker);
}

/// Whether a pair whose marker lies in `runtime` is starting or leaving.
pub(crate) fn any_pending_in(runtime: &Path) -> bool {
    let pairs = pairs();
    pairs
        .starting
        .keys()
        .chain(pairs.leaving.keys())
        .any(|marker| marker.parent() == Some(runtime))
}

/// Whether the pair is on quick search and whether it is still starting.
pub(crate) fn quick_search_state(marker: &Path, now: Instant, log: &DiagnosticLog) -> (bool, bool) {
    match pairs().screen(marker, now, log) {
        Screen::Files => (false, false),
        Screen::Starting => (true, true),
        Screen::Quick(_) => (true, false),
    }
}

/// A leave of quick search, until fzf has ended. It waits, while quick search
/// starts, for fzf to mark itself up, so a marker not there yet never reads as
/// fzf having ended; once it has seen the marker, it writes the key, and
/// again every `RESEND`, until that marker is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leave {
    Waiting,
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
    fn step(&mut self, screen: Screen, now: Instant) -> LeaveStep {
        match *self {
            Self::Waiting => match screen {
                Screen::Quick(marker) => {
                    *self = Self::Sending {
                        marker,
                        first: now,
                        last: now,
                        attempts: 1,
                        written: 0,
                    };
                    LeaveStep::Send
                }
                Screen::Starting => LeaveStep::Wait,
                Screen::Files => {
                    LeaveStep::Expired("quick search request ended without fzf readiness".into())
                }
            },
            Self::Sending {
                marker: seen,
                first,
                last,
                attempts,
                written,
            } => {
                if screen != Screen::Quick(seen) {
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

/// A pending leave waits for the current request to reach fzf or end.
fn leave_requested(marker: &Path) {
    pairs()
        .leaving
        .entry(marker.to_path_buf())
        .or_insert(Leave::Waiting);
}

fn advance_leave(
    marker: &Path,
    shown: bool,
    now: Instant,
    log: &DiagnosticLog,
) -> Option<LeaveStep> {
    let mut pairs = pairs();
    if !shown {
        return pairs.hide(marker, log);
    }
    let screen = pairs.screen(marker, now, log);
    let step = pairs.leaving.get_mut(marker)?.step(screen, now);
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
    let step = advance_leave(&marker, shown, now, log)?;
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

/// Forgets ended pairs in `runtime`; owned endpoints, including recovered
/// ones, are direct children of this generation directory.
pub(crate) fn retain_live(runtime: &Path, live: &[PathBuf], log: &DiagnosticLog) {
    let owned_and_gone =
        |marker: &PathBuf| marker.parent() == Some(runtime) && !live.contains(marker);
    let mut pairs = pairs();
    pairs.leaving.retain(|marker, _| !owned_and_gone(marker));
    pairs.starting.retain(|marker, startup| {
        if !owned_and_gone(marker) {
            return true;
        }
        if let Err(error) = remove_marker(&startup.pending) {
            log.note_once(&error);
            return true;
        }
        false
    });
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

/// DDS jobs owned by one supervisor. Normal turns discard only finished
/// workers. Shutdown cancels all workers before joining any of them.
#[derive(Default)]
pub(crate) struct Dispatches(Vec<(mpsc::Sender<()>, thread::JoinHandle<()>)>);

impl Drop for Dispatches {
    fn drop(&mut self) {
        for (cancel, _) in &self.0 {
            let _ = cancel.send(());
        }
        for (_, worker) in self.0.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Dispatches {
    /// One dispatch that runs until it is cancelled.
    #[cfg(test)]
    pub(crate) fn running() -> Self {
        let (cancel, cancelled) = mpsc::channel::<()>();
        let worker = thread::spawn(move || {
            let _ = cancelled.recv();
        });
        Self(vec![(cancel, worker)])
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn reap(&mut self) {
        self.0.retain(|(_, worker)| !worker.is_finished());
    }

    /// Runs the accepted switch. A leave is driven by the supervisor's turns.
    pub(crate) fn run(
        &mut self,
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
                let now = Instant::now();
                if quick_search_state(&marker, now, log).0 {
                    return;
                }
                let Some(ya) = ya else {
                    return log.record("pair switch skipped: this Eon has no packaged ya");
                };
                let token = super::supervisor::request_id();
                if let Err(error) = quick_search_requested(&marker, &token, Some(now)) {
                    return log.record(&error);
                }
                let pending = pending_marker(&marker, &token);
                let mut command = Command::new(ya);
                command
                    .args(["emit-to", &id.to_string(), "plugin", "quick-search", &token])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                self.start(command, pending, now + START_RECOVERY, log);
            }
        }
    }

    fn start(
        &mut self,
        command: Command,
        pending: PathBuf,
        deadline: Instant,
        log: &Arc<DiagnosticLog>,
    ) {
        self.reap();
        let worker_log = Arc::clone(log);
        let ticket = pending.clone();
        let (cancel, cancelled) = mpsc::channel();
        match thread::Builder::new().spawn(move || {
            if let Err(error) = dispatch(command, deadline, &cancelled) {
                fail_dispatch(&ticket, &error, &worker_log);
            }
        }) {
            Ok(worker) => self.0.push((cancel, worker)),
            Err(error) => fail_dispatch(&pending, &error.to_string(), log),
        }
    }
}

fn fail_dispatch(pending: &Path, error: &str, log: &DiagnosticLog) {
    log.record(&format!(
        "pair switch {} failed: {error}",
        pending.display()
    ));
    if let Err(error) = remove_marker(pending) {
        log.record(&error);
    }
}

fn dispatch(
    mut command: Command,
    deadline: Instant,
    cancelled: &mpsc::Receiver<()>,
) -> Result<(), String> {
    super::platform::require_waitable_children().map_err(|error| error.to_string())?;
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    wait_for_dispatch(&mut child, deadline, cancelled)
}

fn wait_for_dispatch(
    child: &mut Child,
    deadline: Instant,
    cancelled: &mpsc::Receiver<()>,
) -> Result<(), String> {
    let failure = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("ya emit-to {status}")),
            Ok(None) => {}
            Err(error) => return Err(format!("cannot wait for ya emit-to: {error}")),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break "ya emit-to timed out";
        }
        match cancelled.recv_timeout(remaining.min(Duration::from_millis(25))) {
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            _ => break "ya emit-to cancelled",
        }
    };
    // Automatic reaping was excluded before spawn. No other code waits for
    // this child, so its PID remains ours after try_wait reported it live.
    if let Err(error) = child.kill()
        && error.raw_os_error() != Some(libc::ESRCH)
    {
        return Err(format!("{failure}, cannot stop ya: {error}"));
    }
    child
        .wait()
        .map_err(|error| format!("{failure}, cannot reap ya: {error}"))?;
    Err(failure.to_owned())
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
        assert_eq!(
            marker_endpoint(&pair),
            Some((endpoint.into(), MarkerKind::Ready))
        );
        for token in [INITIAL_REQUEST, "14-123456789-0"] {
            assert_eq!(
                marker_endpoint(&pending_marker(&pair, token)),
                Some((endpoint.into(), MarkerKind::Pending))
            );
        }
        for other in [
            "terminal-2.sock",
            "orbit.sock",
            "terminal-2.sock.pending",
            "terminal-2.sock.record",
            "terminal-2.sock.management",
            "terminal-2.quick.sock",
            "terminal-2.quick.pending",
            "terminal-2.quick.fake.pending",
            "terminal-2.quick.2--0.pending",
            "terminal-2.quick.2-1-0.sock",
        ] {
            assert_eq!(marker_endpoint(&endpoint.with_file_name(other)), None);
        }
    }

    #[test]
    fn startup_follows_the_request_through_anima_until_ready_or_ended() {
        let root = scratch("startup");
        let path = root.join("terminal-2.quick");
        let now = Instant::now();
        let admitted = now + Duration::from_secs(14);
        let delayed = admitted + Duration::from_secs(34);
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        quick_search_requested(&path, INITIAL_REQUEST, None).unwrap();
        leave_requested(&path);
        quick_search_admitted(&path, admitted);
        assert_eq!(
            advance_leave(&path, true, delayed, &log),
            Some(LeaveStep::Wait)
        );
        assert_eq!(quick_search_state(&path, delayed, &log), (true, true));

        fs::create_dir(&path).unwrap();
        assert_eq!(quick_search_state(&path, delayed, &log), (true, true));
        fs::remove_dir(&path).unwrap();
        fs::write(&path, INITIAL_REQUEST).unwrap();
        assert_eq!(
            advance_leave(&path, true, delayed, &log),
            Some(LeaveStep::Send)
        );
        assert!(!pending_marker(&path, INITIAL_REQUEST).exists());
        fs::remove_file(&path).unwrap();
        assert_eq!(
            advance_leave(&path, true, delayed, &log),
            Some(LeaveStep::Done)
        );

        let token = "1-1-1";
        quick_search_requested(&path, token, Some(now)).unwrap();
        leave_requested(&path);
        assert!(matches!(
            advance_leave(&path, true, now + START_RECOVERY, &log),
            Some(LeaveStep::Expired(_))
        ));
        assert!(!pending_marker(&path, token).exists());
        fs::write(&path, b"").unwrap();
        assert_eq!(
            quick_search_state(&path, now + START_RECOVERY, &log),
            (true, false)
        );
        assert_eq!(advance_leave(&path, true, now + START_RECOVERY, &log), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_older_ready_marker_does_not_retire_a_newer_request() {
        let root = scratch("stale-ready");
        let path = root.join("terminal-2.quick");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let now = Instant::now();
        let current = "1-1-1";
        quick_search_requested(&path, current, Some(now)).unwrap();
        let pending = pending_marker(&path, current);

        fs::write(&path, INITIAL_REQUEST).unwrap();
        assert_eq!(quick_search_state(&path, now, &log), (true, false));
        assert!(pending.exists(), "the older fzf cleared the new request");
        fs::remove_file(&path).unwrap();
        assert_eq!(quick_search_state(&path, now, &log), (true, true));

        fs::write(&path, current).unwrap();
        assert_eq!(quick_search_state(&path, now, &log), (true, false));
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_dispatch_ends_only_its_own_request() {
        let root = scratch("dispatch");
        let path = root.join("terminal-2.quick");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let now = Instant::now();
        let old = "1-1-0";
        let current = "1-1-1";
        quick_search_requested(&path, old, Some(now)).unwrap();
        quick_search_requested(&path, current, Some(now)).unwrap();
        leave_requested(&path);
        fail_dispatch(&pending_marker(&path, old), "late failure", &log);
        assert_eq!(advance_leave(&path, true, now, &log), Some(LeaveStep::Wait));

        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 300"]);
        let (_sender, cancelled) = mpsc::channel();
        let failure = dispatch(command, now, &cancelled).unwrap_err();
        assert_eq!(failure, "ya emit-to timed out");
        fail_dispatch(&pending_marker(&path, current), &failure, &log);
        assert!(matches!(
            advance_leave(&path, true, now, &log),
            Some(LeaveStep::Expired(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shutting_down_dispatches_reaps_a_wedged_child() {
        use std::os::{fd::OwnedFd, unix::net::UnixStream};

        let root = scratch("shutdown-dispatch");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let pid_file = root.join("pid");
        let pending = root.join("terminal-2.quick.1-1-1.pending");
        fs::File::create_new(&pending).unwrap();
        let (_keep_open, child_input) = UnixStream::pair().unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf '%s\\n' \"$$\" > \"$1\"; exec cat", "sh"])
            .arg(&pid_file)
            .stdin(Stdio::from(OwnedFd::from(child_input)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut dispatches = Dispatches::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        dispatches.start(
            command,
            pending.clone(),
            Instant::now() + START_RECOVERY,
            &log,
        );
        let pid = loop {
            if let Ok(pid) = fs::read_to_string(&pid_file)
                && let Ok(pid) = pid.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "ya never reported its PID");
            thread::yield_now();
        };
        assert!(crate::platform::process(pid).unwrap().is_some());
        drop(dispatches);
        assert!(crate::platform::process(pid).unwrap().is_none());
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_already_reaped_dispatch_reports_loss_of_ownership() {
        use rustix::process::{Pid, WaitOptions, waitpid};
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        waitpid(Some(pid), WaitOptions::empty()).unwrap().unwrap();

        let (_sender, cancelled) = mpsc::channel();
        let error = wait_for_dispatch(&mut child, Instant::now(), &cancelled).unwrap_err();
        assert!(error.starts_with("cannot wait for ya emit-to:"), "{error}");
    }

    /// A leave waits for fzf to mark itself up, then writes its key at once
    /// and every `RESEND` until that marker is gone; a marker removed and
    /// made again is a later quick search, which the leave does not touch.
    #[test]
    fn a_leave_sends_until_fzf_ends_and_never_reads_a_missing_marker_as_ended() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let up = Screen::Quick((1, 7));
        let mut leave = Leave::Waiting;
        assert_eq!(leave.step(Screen::Starting, at(0)), LeaveStep::Wait);
        assert_eq!(leave.step(up, at(1999)), LeaveStep::Send);
        assert_eq!(leave.step(up, at(2100)), LeaveStep::Wait);
        assert_eq!(leave.step(up, at(2249)), LeaveStep::Send);
        assert_eq!(leave.step(up, at(2300)), LeaveStep::Wait);
        assert_eq!(leave.step(Screen::Files, at(2300)), LeaveStep::Done);

        let mut leave = Leave::Waiting;
        assert_eq!(leave.step(up, at(5)), LeaveStep::Send);
        assert_eq!(leave.step(Screen::Quick((1, 8)), at(10)), LeaveStep::Done);
    }

    /// A leave that never sees fzf end gives up after `LEAVE_BOUND`, having
    /// asked for its key every `RESEND`, and says how often Orbit wrote it:
    /// here every other time, the rest refused for now.
    #[test]
    fn a_leave_gives_up_after_its_bound_and_says_so() {
        let start = Instant::now();
        let up = Screen::Quick((1, 7));
        let mut leave = Leave::Waiting;
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
    /// quick search starts waits for readiness. A replacement, a hide or a
    /// terminal ending cancels the leave.
    #[test]
    fn a_pending_leave_is_kept_once_and_ended_by_hiding_or_a_new_quick_search() {
        let root = scratch("leave");
        let path = root.join("terminal-2.quick");
        let now = Instant::now();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        assert_eq!(advance_leave(&path, true, now, &log), None);
        quick_search_requested(&path, INITIAL_REQUEST, Some(now)).unwrap();
        leave_requested(&path);
        assert_eq!(advance_leave(&path, true, now, &log), Some(LeaveStep::Wait));
        fs::write(&path, INITIAL_REQUEST).unwrap();
        assert_eq!(advance_leave(&path, true, now, &log), Some(LeaveStep::Send));
        leave_requested(&path);
        assert_eq!(advance_leave(&path, true, now, &log), Some(LeaveStep::Wait));
        assert!(any_pending());
        assert_eq!(
            advance_leave(&path, false, now, &log),
            Some(LeaveStep::Done)
        );
        assert_eq!(advance_leave(&path, true, now, &log), None);
        fs::remove_file(&path).unwrap();

        leave_requested(&path);
        quick_search_requested(&path, "1-1-1", Some(now)).unwrap();
        assert_eq!(advance_leave(&path, true, now, &log), None);
        assert!(pending_marker(&path, "1-1-1").exists());
        leave_requested(&path);
        assert_eq!(
            advance_leave(&path, false, now, &log),
            Some(LeaveStep::Done)
        );
        assert!(!pending_marker(&path, "1-1-1").exists());
        assert_eq!(quick_search_state(&path, now, &log), (false, false));

        leave_requested(&path);
        retain_live(&root, &[], &log);
        assert_eq!(advance_leave(&path, true, now, &log), None);
        leave_requested(&path);
        forget_leave(&path);
        assert_eq!(advance_leave(&path, true, now, &log), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A supervisor prunes only its runtime, even with no live pairs; another
    /// runtime with the same path prefix keeps its pending leave unchanged.
    #[test]
    fn pruning_ended_pairs_preserves_other_runtimes_requests_and_leaves() {
        let root = scratch("prune");
        let runtime = root.join("runtime");
        let live = runtime.join("terminal-2.quick");
        let ended = runtime.join("terminal-3.quick");
        let other = root.join("runtime-other/terminal-2.quick");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let now = Instant::now();
        fs::create_dir(&runtime).unwrap();
        fs::create_dir(other.parent().unwrap()).unwrap();
        for marker in [&live, &ended, &other] {
            quick_search_requested(marker, INITIAL_REQUEST, Some(now)).unwrap();
            leave_requested(marker);
        }
        let live_leave = pairs().leaving[&live];
        let other_leave = pairs().leaving[&other];

        retain_live(&runtime, std::slice::from_ref(&live), &log);
        assert!(pending_marker(&live, INITIAL_REQUEST).exists());
        assert!(!pending_marker(&ended, INITIAL_REQUEST).exists());
        assert!(pending_marker(&other, INITIAL_REQUEST).exists());
        {
            let pairs = pairs();
            assert_eq!(pairs.leaving.get(&live), Some(&live_leave));
            assert!(!pairs.leaving.contains_key(&ended));
            assert_eq!(pairs.leaving.get(&other), Some(&other_leave));
        }
        retain_live(&runtime, &[], &log);
        {
            let pairs = pairs();
            assert!(!pairs.leaving.contains_key(&live));
            assert_eq!(pairs.leaving.get(&other), Some(&other_leave));
        }
        terminal_ended(&other.with_extension("sock")).unwrap();
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
        assert!(FZF_ARGUMENTS[1].contains("\"$EON_QUICK_TOKEN\""));
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
