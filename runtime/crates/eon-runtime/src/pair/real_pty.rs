//! Quick search composed for real (EON-C18, EON-C10): the packaged Files
//! configuration's Yazi and quick-search plugin, fzf and Zoxide, under a real
//! Orbit that this Runtime leases, driven turn by turn as the supervisor
//! drives a pair. eon-test and the eon-tests check name the tools; a run
//! without them fails rather than skips.

use super::*;
use crate::sessions::{
    self, AttachName, RunningSession,
    tests::{leased_orbit, script},
};
use orbit_protocol::session::{self as orbs, KeyAction, KeyEvent};
use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::atomic::{AtomicBool, Ordering},
};

/// The tool eon-test names in `variable`.
fn tool(variable: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(variable)
            .unwrap_or_else(|| panic!("{variable} is unset: run the suites through eon-test")),
    )
}

/// A shown Files pair: real Yazi in a real Orbit run, leased here.
struct Pair {
    root: PathBuf,
    session: RunningSession,
    log: Arc<DiagnosticLog>,
    marker: PathBuf,
    id: u64,
    ya: PathBuf,
}

/// How a test's pair starts.
#[derive(Default)]
struct Start {
    /// On quick search, as a pending tab's pair does.
    quick: bool,
    /// Seconds fzf waits before it starts.
    slow_fzf: Option<f32>,
}

impl Pair {
    fn start(name: &str, start: Start) -> Self {
        let root = crate::supervisor::temporary_directory();
        fs::write(root.join("test"), name).unwrap();
        let endpoint = root.join("orbit.sock");
        let marker = marker(&endpoint);
        let id = client_id(&endpoint);
        let bin = root.join("bin");
        // Yazi lists the parent folder too, so the files sit two levels below
        // the run's own files, the marker among them.
        let files = root.join("w/files");
        for directory in [
            bin.clone(),
            files.join("alpha/a-inside"),
            files.join("beta/b-inside"),
            root.join("zoxide"),
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(files.join("note.txt"), "a note\n").unwrap();
        let fzf = tool("EON_TEST_QUICK_FZF");
        match start.slow_fzf {
            Some(seconds) => script(
                &bin.join("fzf"),
                &format!(
                    "#!/bin/sh\nsleep {seconds}\nexec '{}' \"$@\"\n",
                    fzf.display()
                ),
            ),
            None => std::os::unix::fs::symlink(&fzf, bin.join("fzf")).unwrap(),
        }
        let zoxide = tool("EON_TEST_QUICK_ZOXIDE");
        std::os::unix::fs::symlink(&zoxide, bin.join("zoxide")).unwrap();
        for folder in ["alpha", "beta"] {
            let status = Command::new(&zoxide)
                .args(["add", "--"])
                .arg(files.join(folder))
                .env("_ZO_DATA_DIR", root.join("zoxide"))
                .status()
                .unwrap();
            assert!(status.success(), "zoxide add: {status}");
        }
        // Eon's CLI as the tab-folder plugin calls it: its snapshot lists
        // this terminal as popup u1 of tab t1, and every call is logged.
        script(
            &bin.join("eon"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n\
                 [ \"$1\" = session ] && \
                 printf '{{\"tabs\":[{{\"id\":\"t1\",\"popups\":[{{\"id\":\"u1\",\"terminal\":\"terminal-1\"}}]}}]}}\\n'\n\
                 exit 0\n",
                root.join("eon.log").display()
            ),
        );
        fs::write(root.join("fzf-args"), FZF_ARGUMENTS.join("\n")).unwrap();
        let yazi = tool("EON_TEST_QUICK_YAZI");
        // Each pair's Yazi and `ya` share a bus of their own, which no other
        // test's Yazi hosts or ends.
        fs::create_dir_all(root.join("r")).unwrap();
        script(
            &bin.join("ya"),
            &format!(
                "#!/bin/sh\nXDG_RUNTIME_DIR='{}' exec '{}' \"$@\"\n",
                root.join("r").display(),
                yazi.with_file_name("ya").display()
            ),
        );
        let launch = root.join("launch");
        script(
            &launch,
            &format!(
                "#!/bin/sh\n\
                 export XDG_RUNTIME_DIR='{root}/r' YAZI_CONFIG_HOME='{config}' _ZO_DATA_DIR='{root}/zoxide'\n\
                 export PATH='{root}/bin':\"$PATH\"\n\
                 export EON_QUICK_MARKER='{marker}' EON_QUICK_FZF_ARGS=\"$(cat '{root}/fzf-args')\"\n\
                 export EON_FILES_CLI='{root}/bin/eon' EON_TAB=t1 EON_TERMINAL=terminal-1.g1\n\
                 {quick}\
                 cd '{files}' && exec '{yazi}' --client-id {id} '{files}'\n",
                root = root.display(),
                config = tool("EON_TEST_QUICK_FILES_CONFIG").display(),
                marker = marker.display(),
                files = files.display(),
                yazi = yazi.display(),
                quick = if start.quick {
                    "export EON_FILES_START=quick\n"
                } else {
                    ""
                },
            ),
        );
        if start.quick {
            quick_search_requested(&marker, Duration::ZERO);
        }
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let session = leased_orbit(&root, &format!("exec '{}'", launch.display()));
        Self {
            ya: bin.join("ya"),
            root,
            session,
            log,
            marker,
            id,
        }
    }

    fn screen(&mut self) -> String {
        sessions::read_visible(&mut self.session, &self.log)
            .unwrap()
            .text
    }

    /// Reads the screen until `wanted` holds of it, a supervisor turn
    /// between reads.
    fn until(&mut self, what: &str, wanted: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.turn();
            let text = self.screen();
            if wanted(&text) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "never {what}:\n{text}\nlog:\n{}",
                self.diagnostics()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn diagnostics(&self) -> String {
        let mut text = format!("eon calls: {:?}\n", self.eon_calls());
        for entry in fs::read_dir(self.root.join("logs")).into_iter().flatten() {
            text += &fs::read_to_string(entry.unwrap().path()).unwrap_or_default();
        }
        text
    }

    /// Files settled in the fixture's folder. Yazi draws the parent column,
    /// the listing and the hovered folder's preview on turns of their own, so
    /// all three must be on the first entry row (alpha, previewing
    /// a-inside), and no plugin task may still count as running ("1 left").
    fn files_shown(&mut self) -> String {
        self.until("Files", |text| {
            text.lines()
                .find(|line| line.matches('│').count() == 2)
                .is_some_and(|row| {
                    let columns: Vec<&str> = row.split('│').collect();
                    columns[0].contains("files")
                        && columns[1].contains("alpha")
                        && columns[2].contains("a-inside")
                })
                && text.contains("Tab Search")
                && text.contains("note.txt")
                && !text.contains("Loading...")
                && !text.contains("Quick search >")
                && !text.trim_end().ends_with(" left")
        })
    }

    fn quick_search_shown(&mut self) -> String {
        self.until("quick search", |text| text.contains("Quick search >"))
    }

    /// One supervisor turn for this shown pair: a quick search that is up
    /// ends its startup grace, and a pending leave goes on.
    fn turn(&mut self) -> Option<LeaveStep> {
        settle_starting(&self.marker, true);
        drive_leave(&mut self.session, true, Instant::now(), &self.log)
    }

    /// Alt+Z on Files: `ya emit-to` asks the pair's Yazi for quick search.
    fn show_quick_search(&mut self) {
        run(
            Switch::ShowQuickSearch,
            Some(&self.ya),
            self.id,
            self.marker.clone(),
            &self.log,
        );
    }

    /// Alt+Shift+Y on quick search, then the supervisor's turns until the
    /// leave is over; returns the copies of the key it wrote.
    fn leave(&mut self) -> u32 {
        run(
            Switch::LeaveQuickSearch,
            None,
            self.id,
            self.marker.clone(),
            &self.log,
        );
        let mut copies = 0;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.turn() {
                Some(LeaveStep::Send) => copies += 1,
                Some(LeaveStep::Wait) => {}
                Some(LeaveStep::Done) | None => return copies,
                Some(LeaveStep::Expired(detail)) => {
                    panic!("{detail}\n{}\nlog:\n{}", self.screen(), self.diagnostics())
                }
            }
            assert!(Instant::now() < deadline, "the leave never ended");
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// A Yazi command, as `ya emit-to` sends it, once Yazi takes commands.
    fn emit(&self, arguments: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let output = Command::new(&self.ya)
                .arg("emit-to")
                .arg(self.id.to_string())
                .args(arguments)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            if output.status.success() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "ya emit-to {arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Reads the screen until it is `screen` again.
    fn back_to(&mut self, what: &str, screen: &str) {
        self.until(what, |text| text == screen);
    }

    /// The leave key, written as a stale or forged copy would be, outside any
    /// leave.
    fn stray_key(&mut self) {
        assert_eq!(
            sessions::send_input(&mut self.session, leave_input(), 0, &self.log),
            Ok(false)
        );
    }

    /// An attached client, which types as a person does.
    fn keyboard(&mut self) -> Keyboard {
        sessions::name_presentation(
            std::slice::from_mut(&mut self.session),
            Some(AttachName::of(std::process::id()).unwrap()),
            true,
            &self.log,
        );
        let mut stream = UnixStream::connect(&self.session.endpoint).unwrap();
        stream
            .write_all(&orbs::encode_client_message(&orbs::ClientMessage::Hello).unwrap())
            .unwrap();
        let mut reader = stream.try_clone().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        // Frames are read and dropped, so Orbit never finds the client slow.
        let drain = {
            let done = Arc::clone(&done);
            thread::spawn(move || {
                let mut buffer = [0; 65536];
                while !done.load(Ordering::Relaxed) {
                    if matches!(reader.read(&mut buffer), Ok(0) | Err(_)) {
                        return;
                    }
                }
            })
        };
        Keyboard {
            stream,
            done,
            drain: Some(drain),
        }
    }

    fn eon_calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("eon.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.starts_with("session"))
            .map(str::to_owned)
            .collect()
    }

    fn stop(mut self) {
        let _ = sessions::stop_managed_sessions(
            std::slice::from_mut(&mut self.session),
            Duration::from_secs(5),
        );
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Keyboard {
    stream: UnixStream,
    done: Arc<AtomicBool>,
    drain: Option<thread::JoinHandle<()>>,
}

impl Keyboard {
    fn key(&mut self, key: PhysicalKey, modifiers: Modifiers, text: Option<char>) {
        for action in [KeyAction::Press, KeyAction::Release] {
            let event = KeyEvent {
                action,
                key,
                modifiers,
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: text
                    .filter(|_| action == KeyAction::Press)
                    .map(String::from),
                unshifted_codepoint: text,
            };
            self.stream
                .write_all(&orbs::encode_client_message(&orbs::ClientMessage::Key(event)).unwrap())
                .unwrap();
        }
        thread::sleep(Duration::from_millis(30));
    }

    /// Types lowercase letters.
    fn text(&mut self, text: &str) {
        for letter in text.chars() {
            let key =
                PhysicalKey::from_raw(PhysicalKey::A.raw() + (letter as u16 - u16::from(b'a')))
                    .unwrap();
            self.key(key, Modifiers::empty(), Some(letter));
        }
    }

    fn press(&mut self, key: PhysicalKey) {
        self.key(key, Modifiers::empty(), None);
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

/// Busy threads that keep the machine loaded while they live.
struct Load(Arc<AtomicBool>, Vec<thread::JoinHandle<()>>);

impl Load {
    fn start(threads: usize) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let workers = (0..threads)
            .map(|_| {
                let done = Arc::clone(&done);
                thread::spawn(move || {
                    let mut x = 0u64;
                    while !done.load(Ordering::Relaxed) {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                })
            })
            .collect();
        Self(done, workers)
    }
}

impl Drop for Load {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
        for worker in self.1.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Alt+Shift+Y right after quick search is asked for, in a pair that starts
/// on it and then 200 times under load, always reaches Files, with Yazi's
/// own reader stopping as fzf takes the terminal: the key never reaches Yazi
/// for anything to change, and Files is as it was each time. Its busy threads
/// would starve the parallel suites' timed tests, so the ignored pass runs it
/// alone.
#[test]
#[ignore = "loads the machine, so it runs alone"]
fn a_leave_right_after_open_reaches_files_every_time_under_load() {
    let mut pair = Pair::start(
        "immediate",
        Start {
            quick: true,
            ..Start::default()
        },
    );
    assert!(pair.leave() >= 1);
    let files = pair.files_shown();
    assert!(!pair.marker.exists());
    let _load = Load::start(2);
    for iteration in 0..200 {
        pair.show_quick_search();
        let copies = pair.leave();
        assert!(copies >= 1, "iteration {iteration}: no key written");
        pair.back_to(&format!("iteration {iteration}: Files as it was"), &files);
        assert!(!pair.marker.exists(), "iteration {iteration}");
    }
    assert!(pair.eon_calls().is_empty(), "{:?}", pair.eon_calls());
    pair.stop();
}

/// A leave asked for before fzf has started waits for fzf to mark itself up,
/// writes nothing meanwhile, and then reaches Files.
#[test]
fn a_leave_waits_for_a_slow_fzf() {
    let mut pair = Pair::start(
        "slow",
        Start {
            slow_fzf: Some(1.0),
            ..Start::default()
        },
    );
    let files = pair.files_shown();
    pair.show_quick_search();
    let started = Instant::now();
    let copies = pair.leave();
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert!(copies >= 1);
    pair.back_to("Files as it was", &files);
    pair.stop();
}

/// The leave key reaching Files after fzf has ended, or written by anyone
/// while Files is shown, changes nothing there.
#[test]
fn a_stray_leave_key_changes_nothing_in_files() {
    let mut pair = Pair::start("stray", Start::default());
    let files = pair.files_shown();
    assert!(files.contains("a-inside"), "{files}");
    for _ in 0..3 {
        pair.stray_key();
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(pair.screen(), files);
    pair.stop();
}

/// An open filter's input box keeps its text when the leave key reaches it.
#[test]
fn a_stray_leave_key_keeps_a_filters_text() {
    let mut pair = Pair::start("filter", Start::default());
    pair.files_shown();
    pair.emit(&["filter"]);
    // `ya emit-to` returns once Yazi has the command, not once it has opened
    // the input; pasted earlier, the text can reach Yazi first and be lost.
    pair.until("the filter input", |text| text.contains("Filter:"));
    assert!(
        sessions::send_input(
            &mut pair.session,
            LeaseInput::Paste("alp".into()),
            0,
            &pair.log
        )
        .is_ok()
    );
    let filtering = pair.until("the filter text", |text| text.contains("(filter: alp)"));
    pair.stray_key();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(pair.screen(), filtering);
    pair.stop();
}

/// A pending chord's hint is the one thing the leave key changes: it is
/// dismissed, and the chord works when typed again.
#[test]
fn a_stray_leave_key_dismisses_only_a_pending_chords_hint() {
    let mut pair = Pair::start("chord", Start::default());
    pair.files_shown();
    let mut keyboard = pair.keyboard();
    pair.emit(&["arrow", "1"]);
    let on_beta = pair.until("beta hovered", |text| text.contains("b-inside"));
    keyboard.text("g");
    pair.until("the chord's hint", |text| text != on_beta);
    pair.stray_key();
    pair.until("the hint dismissed", |text| text == on_beta);
    keyboard.text("gg");
    pair.until("alpha hovered", |text| text.contains("a-inside"));
    drop(keyboard);
    pair.stop();
}

/// A marker no fzf made draws the leave's copies into Yazi for the bound,
/// all inert, then the leave stops and says it gave up.
#[test]
fn a_forged_marker_draws_inert_copies_for_the_bound() {
    let mut pair = Pair::start("forged", Start::default());
    let files = pair.files_shown();
    fs::write(&pair.marker, b"").unwrap();
    run(
        Switch::LeaveQuickSearch,
        None,
        pair.id,
        pair.marker.clone(),
        &pair.log,
    );
    let started = Instant::now();
    let mut copies = 0;
    let expired = loop {
        match pair.turn() {
            Some(LeaveStep::Send) => copies += 1,
            Some(LeaveStep::Expired(detail)) => break detail,
            step => assert!(step == Some(LeaveStep::Wait), "{step:?}"),
        }
        thread::sleep(Duration::from_millis(25));
    };
    assert!(started.elapsed() >= LEAVE_BOUND);
    assert!((2..=8).contains(&copies), "{copies}");
    assert!(
        expired.ends_with(&format!(
            "did not leave within 2 s; Orbit wrote its key {copies} of {copies} times"
        )),
        "{expired}"
    );
    assert!(pair.diagnostics().contains("did not leave within 2 s"));
    assert_eq!(pair.screen(), files);
    pair.stop();
}

/// Quick search's own keys behave as before (EON-C18): Enter picks the
/// highlighted folder for the tab, Tab moves Files there, Esc closes Files,
/// and a leave with a query typed reaches Files unchanged. No fzf listens.
#[test]
fn quick_search_keys_pick_browse_close_and_leave_with_a_query() {
    let mut pair = Pair::start(
        "keys",
        Start {
            quick: true,
            ..Start::default()
        },
    );
    let quick = pair.quick_search_shown();
    let mut keyboard = pair.keyboard();
    assert!(quick.contains("beta") && quick.contains("alpha"), "{quick}");
    assert!(pair.marker.exists());
    #[cfg(target_os = "linux")]
    {
        let mut seen = 0;
        for entry in fs::read_dir("/proc").unwrap().flatten() {
            let command = fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let command = String::from_utf8_lossy(&command);
            if command.contains(&*pair.marker.to_string_lossy())
                || command.contains("ctrl-alt-shift-page-down:become")
            {
                seen += 1;
                assert!(!command.contains("--listen"), "{command}");
            }
        }
        assert!(seen >= 1, "found no quick search fzf");
    }
    let sockets: Vec<_> = fs::read_dir(&pair.root)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().into_string().unwrap())
        .filter(|name| name.contains("quick"))
        .collect();
    assert_eq!(sockets, ["orbit.quick"]);

    // A query typed, then Alt+Shift+Y.
    keyboard.text("bet");
    pair.until("the query", |text| text.contains("> bet"));
    assert!(pair.leave() >= 1);
    pair.files_shown();
    pair.until("alpha's preview", |text| text.contains("a-inside"));

    // Enter picks the highlighted folder.
    pair.show_quick_search();
    pair.quick_search_shown();
    keyboard.text("bet");
    pair.until("the query", |text| text.contains("> bet"));
    keyboard.press(PhysicalKey::ENTER);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pair
        .eon_calls()
        .iter()
        .any(|call| call.starts_with("tab directory t1 -- ") && call.ends_with("/w/files/beta"))
    {
        assert!(Instant::now() < deadline, "{:?}", pair.eon_calls());
        thread::sleep(Duration::from_millis(20));
    }
    pair.files_shown();
    assert!(!pair.marker.exists());

    // Tab moves Files to the highlighted folder.
    pair.show_quick_search();
    pair.quick_search_shown();
    keyboard.text("alp");
    pair.until("the query", |text| text.contains("> alp"));
    keyboard.press(PhysicalKey::TAB);
    pair.until("Files in alpha", |text| {
        text.contains("Tab Search") && text.contains("files/alpha")
    });
    assert!(!pair.marker.exists());

    // Esc closes Files.
    pair.show_quick_search();
    pair.quick_search_shown();
    keyboard.press(PhysicalKey::ESCAPE);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pair.eon_calls().contains(&"popup hide u1".to_owned()) {
        assert!(Instant::now() < deadline, "{:?}", pair.eon_calls());
        thread::sleep(Duration::from_millis(20));
    }
    pair.until("Files in alpha again", |text| {
        text.contains("Tab Search") && text.contains("files/alpha")
    });
    assert!(!pair.marker.exists());
    drop(keyboard);
    pair.stop();
}
