//! The Files pair's two screens (EON-C18). Each pair Yazi gets a DDS client
//! ID from its terminal's endpoint, and its quick search's fzf listens on a
//! socket beside that endpoint that answers only while quick search is shown.
//! Alt+Z and Alt+Shift+Y switch screens through them: `ya emit-to` shows quick
//! search, and a `become(exit 11)` POST to fzf leaves it for Files.

use super::diagnostics::DiagnosticLog;
use eon_workspace_protocol::v43::{InvokeIntent, PROJECT_ENTRY};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    os::unix::{ffi::OsStrExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// How long a switch waits on fzf's socket.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);

/// How long after it can start quick search counts as shown before its fzf
/// listens. fzf takes about 0.6 s to start; a key in that time acts on the
/// screen that is coming, not on Files. Not user-visible, so fixed.
const STARTUP_GRACE: Duration = Duration::from_secs(2);

/// The longest the window's first pair waits for the other-windows notice
/// before its Yazi starts (cli.rs `startup_notice`).
const NOTICE_WAIT: Duration = Duration::from_secs(1);

/// The longest the startup Anima in the window's first pair runs past its
/// duration: eon-files stops it 2 s late and waits 0.25 s more for it to end
/// (cli.rs `play_startup_anima`).
const ANIMA_OVERRUN: Duration = Duration::from_millis(2250);

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

/// The socket the quick search of the pair at `endpoint` listens on: beside
/// that endpoint, in the window's own runtime directory, and never longer than
/// its `.management` socket, whose length the window's runtime root is bounded
/// by. fzf takes a `--listen` address as a Unix socket only when it ends in
/// `.sock` (else it reads a port and exits), so `terminal-N.sock` pairs with
/// `terminal-N.quick.sock`.
pub(crate) fn socket(endpoint: &Path) -> PathBuf {
    let bytes = endpoint.as_os_str().as_bytes();
    let mut path = OsString::from(OsStr::from_bytes(
        bytes.strip_suffix(b".sock").unwrap_or(bytes),
    ));
    path.push(".quick.sock");
    path.into()
}

/// The terminal endpoint whose pair socket is at `path`, if `path` names one;
/// never a terminal endpoint itself.
pub(crate) fn socket_endpoint(path: &Path) -> Option<PathBuf> {
    let stem = path.as_os_str().as_bytes().strip_suffix(b".quick.sock")?;
    Some(PathBuf::from(OsStr::from_bytes(&[stem, b".sock"].concat())))
}

/// When the startup grace of each pair's requested quick search ends, by
/// socket, until its fzf listens, the pair is hidden or picks, or it ends.
static REQUESTS: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

fn requests<T>(use_them: impl FnOnce(&mut HashMap<PathBuf, Instant>) -> T) -> T {
    let mut requests = REQUESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let requests = requests.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    requests.retain(|_, end| now < *end);
    use_them(requests)
}

/// How long the window's first pair takes before its quick search can start:
/// its startup Anima of `anima_seconds`, if any, then the notice.
pub(crate) fn fresh_start(anima_seconds: Option<u64>) -> Duration {
    NOTICE_WAIT
        + anima_seconds.map_or(Duration::ZERO, |seconds| {
            Duration::from_secs(seconds) + ANIMA_OVERRUN
        })
}

/// Records that the pair at `socket` was asked for quick search, which can
/// start after `delay`: started on it, or sent a switch to it.
pub(crate) fn quick_search_requested(socket: &Path, delay: Duration) {
    requests(|requests| {
        requests.insert(socket.to_path_buf(), Instant::now() + delay + STARTUP_GRACE)
    });
}

/// Ends the startup grace of the pair at `socket`: its quick search is up and
/// answers, or it has ended.
pub(crate) fn quick_search_settled(socket: &Path) {
    requests(|requests| requests.remove(socket));
}

/// Whether any pair's quick search is starting.
pub(crate) fn any_starting() -> bool {
    requests(|requests| !requests.is_empty())
}

/// Ends the startup grace of the pair at `socket` once its fzf listens or the
/// pair is not `shown`: a key can then no longer act on a quick search still
/// to come.
pub(crate) fn settle_starting(socket: &Path, shown: bool) {
    requests(|requests| {
        if requests.contains_key(socket) && (!shown || socket.exists()) {
            requests.remove(socket);
        }
    });
}

/// The end of the startup grace of a quick search requested of `socket` whose
/// fzf has not answered yet.
pub(crate) fn starting(socket: &Path) -> Option<Instant> {
    requests(|requests| requests.get(socket).copied())
}

/// Whether quick search is shown: its fzf accepts on the socket, or it was
/// requested within the startup grace and is still starting.
pub(crate) fn quick_search_shown(socket: &Path) -> bool {
    if UnixStream::connect(socket).is_ok() {
        quick_search_settled(socket);
        return true;
    }
    starting(socket).is_some()
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

/// Runs `switch` for the pair with client `id` beside the supervisor; a
/// failure is logged and changes nothing else.
pub(crate) fn run(
    switch: Switch,
    ya: Option<&Path>,
    id: u64,
    socket: PathBuf,
    log: &Arc<DiagnosticLog>,
) {
    let worker = Arc::clone(log);
    let started = match switch {
        Switch::None => return,
        Switch::ShowQuickSearch => {
            // A second quick search would wait behind the shown one (EON-C18).
            if quick_search_shown(&socket) {
                return;
            }
            quick_search_requested(&socket, Duration::ZERO);
            let Some(ya) = ya else {
                return log.record("pair switch skipped: this Eon has no packaged ya");
            };
            let mut command = Command::new(ya);
            command
                .args(["emit-to", &id.to_string(), "plugin", "quick-search"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            thread::Builder::new().spawn(move || {
                let failure = match command.status() {
                    Ok(status) if status.success() => return,
                    Ok(status) => format!("ya emit-to {status}"),
                    Err(error) => error.to_string(),
                };
                worker.record(&format!("pair switch to quick search failed: {failure}"));
            })
        }
        Switch::LeaveQuickSearch => thread::Builder::new().spawn(move || {
            if let Err(error) = leave_quick_search(&socket) {
                worker.record(&format!("pair switch to Files failed: {error}"));
            }
        }),
    };
    if let Err(error) = started {
        log.record(&format!("pair switch failed: {error}"));
    }
}

/// Asks the shown quick search to end with status 11, which returns its pair
/// to Files without moving it. A quick search still starting gets until the
/// end of its startup grace to listen.
fn leave_quick_search(socket: &Path) -> Result<(), String> {
    const ACTION: &str = "become(exit 11)";
    let deadline = starting(socket);
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(stream) => break stream,
            Err(_) if deadline.is_some_and(|deadline| Instant::now() < deadline) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    quick_search_settled(socket);
    stream
        .set_read_timeout(Some(SOCKET_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(SOCKET_TIMEOUT)))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(
            format!(
                "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{ACTION}",
                ACTION.len()
            )
            .as_bytes(),
        )
        .map_err(|error| error.to_string())?;
    let mut reply = [0; 12];
    let mut read = 0;
    while read < reply.len() {
        match stream.read(&mut reply[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) => return Err(error.to_string()),
        }
    }
    if reply[..read].starts_with(b"HTTP/1.1 200") {
        Ok(())
    } else {
        Err(format!(
            "fzf answered {:?}",
            String::from_utf8_lossy(&reply[..read])
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fzf 0.73 reads a `--listen` address as a Unix socket only when it ends
    /// in `.sock`; any other name is taken for a port and fzf exits 2. The
    /// recovery sweep maps the socket back to its terminal and never takes a
    /// terminal endpoint, record or marker for a pair socket.
    #[test]
    fn a_pair_socket_ends_in_sock_and_maps_back_to_its_terminal_only() {
        let endpoint = Path::new("/run/eon/w/0000000a/generations/g1/terminal-2.sock");
        let pair = socket(endpoint);
        assert!(pair.as_os_str().as_bytes().ends_with(b".sock"));
        assert_eq!(pair.file_name().unwrap(), "terminal-2.quick.sock");
        assert_eq!(socket_endpoint(&pair).as_deref(), Some(endpoint));
        for other in [
            "terminal-2.sock",
            "orbit.sock",
            "terminal-2.sock.pending",
            "terminal-2.sock.record",
            "terminal-2.sock.management",
        ] {
            assert_eq!(socket_endpoint(&endpoint.with_file_name(other)), None);
        }
    }

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
    /// sockets, each in its own window's directory.
    #[test]
    fn pairs_of_two_windows_sharing_a_runtime_base_do_not_collide() {
        let base = Path::new("/run/user/1000/eon");
        let generation = crate::generation::generation_directory;
        let first = generation(&base.join("w/0000000a"), "g1").join("terminal-2.sock");
        let second = generation(&base.join("w/0000000b"), "g1").join("terminal-2.sock");
        assert_ne!(client_id(&first), client_id(&second));
        assert_ne!(socket(&first), socket(&second));
        for endpoint in [&first, &second] {
            assert_eq!(socket(endpoint).parent(), endpoint.parent());
        }
        assert_eq!(socket(&first).file_name().unwrap(), "terminal-2.quick.sock");
    }

    /// A window is admitted only if its `.management` sockets fit (new_window),
    /// so a pair socket must fit for the longest root admitted: on macOS that
    /// root is under `/tmp/eon-UID/w`, and `sun_path` holds 103 bytes.
    #[test]
    fn a_pair_socket_fits_for_the_longest_admitted_window_root() {
        use crate::platform::socket_path_fits;
        let generation = format!("g1-{}", "f".repeat(32));
        let management = |root: &Path| {
            crate::generation::generation_directory(root, &generation)
                .join("terminal-9999.sock.management")
        };
        let mut root = PathBuf::from("/tmp/eon-501/w/");
        while socket_path_fits(&management(&root.join("x"))) {
            root.as_mut_os_string().push("x");
        }
        assert!(socket_path_fits(&management(&root)));
        let endpoint =
            crate::generation::generation_directory(&root, &generation).join("terminal-9999.sock");
        let pair = socket(&endpoint);
        assert!(
            socket_path_fits(&pair),
            "{} is {} bytes",
            pair.display(),
            pair.as_os_str().len()
        );
    }

    /// A key pressed while a requested quick search starts acts on quick
    /// search: it counts as shown until its fzf listens or the grace ends,
    /// and a switch to Files waits for it to listen.
    #[test]
    fn a_starting_quick_search_counts_as_shown_and_waits_for_a_switch_to_files() {
        let root = std::env::temp_dir().join(format!("eon-pair-start-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("terminal-2.quick.sock");
        let _ = std::fs::remove_file(&path);
        assert!(!quick_search_shown(&path));
        quick_search_requested(&path, Duration::ZERO);
        assert!(quick_search_shown(&path));
        assert_eq!(
            plan(
                "files",
                InvokeIntent::Toggle,
                true,
                quick_search_shown(&path),
                true
            ),
            (InvokeIntent::Focus, Switch::LeaveQuickSearch)
        );
        // fzf starts listening only after the switch was sent.
        let listening = path.clone();
        let fzf = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            let listener = std::os::unix::net::UnixListener::bind(&listening).unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 256];
            while !request.ends_with(b"become(exit 11)") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
        });
        assert_eq!(leave_quick_search(&path), Ok(()));
        fzf.join().unwrap();
        // Once fzf has answered, only the socket says whether it is shown.
        std::fs::remove_file(&path).unwrap();
        assert!(!quick_search_shown(&path));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A starting quick search stops counting as shown once its pair is
    /// hidden, so Alt+Z, Esc, Alt+Z asks for quick search again, or once its
    /// fzf listens, so a quick search left with Tab is not taken for starting.
    #[test]
    fn hiding_the_pair_or_its_fzf_listening_ends_the_startup_grace() {
        let root = std::env::temp_dir().join(format!("eon-pair-settle-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("terminal-2.quick.sock");
        let _ = std::fs::remove_file(&path);
        quick_search_requested(&path, Duration::ZERO);
        settle_starting(&path, true);
        assert!(quick_search_shown(&path));
        settle_starting(&path, false);
        assert!(!quick_search_shown(&path));
        assert_eq!(
            plan(PROJECT_ENTRY, InvokeIntent::Toggle, false, false, false),
            (InvokeIntent::Toggle, Switch::ShowQuickSearch)
        );
        quick_search_requested(&path, Duration::ZERO);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        settle_starting(&path, true);
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        assert!(starting(&path).is_none());
        std::fs::remove_dir_all(root).unwrap();
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
            // A starting quick search is left once it listens.
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

    #[test]
    fn leaving_quick_search_posts_become_exit_11_to_fzf() {
        let root = std::env::temp_dir().join(format!("eon-pair-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("fzf.sock");
        let _ = std::fs::remove_file(&path);
        assert!(!quick_search_shown(&path));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(quick_search_shown(&path));
        let server = thread::spawn(move || {
            // The probe's connection, then the POST's.
            drop(listener.accept().unwrap());
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 256];
            while !request.ends_with(b"become(exit 11)") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "{}", String::from_utf8_lossy(&request));
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
            String::from_utf8(request).unwrap()
        });
        assert_eq!(leave_quick_search(&path), Ok(()));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST / HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("Content-Length: 15\r\n"), "{request}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
