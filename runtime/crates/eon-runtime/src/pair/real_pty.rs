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
    dispatches: Dispatches,
}

/// How a test's pair starts.
#[derive(Default)]
struct Start {
    /// On quick search, as a pending tab's pair does.
    quick: bool,
    /// Supply the startup header to exercise fzf's tutor action.
    tutor_notice: bool,
    /// Seconds fzf waits before it starts.
    slow_fzf: Option<f32>,
    /// Hold Yazi's first DDS registration until one real ya command has exited.
    wait_for_first_ya: bool,
    /// Use another pair's bus while this one's Yazi has yet to start.
    shared_bus: Option<PathBuf>,
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
                 if [ \"$1\" = --eon-initial-picker-header ]; then\n\
                   IFS= read -r token < \"$EON_QUICK_MARKER\"\n\
                   [ \"$2\" = initial ] && [ \"$token\" = initial ] && printf 'Tutor header fixture\\n'\n\
                   exit\n\
                 fi\n\
                 [ \"$1\" = session ] && \
                 printf '{{\"tabs\":[{{\"id\":\"t1\",\"popups\":[{{\"id\":\"u1\",\"terminal\":\"terminal-1\"}}]}}]}}\\n'\n\
                 exit 0\n",
                root.join("eon.log").display()
            ),
        );
        fs::write(root.join("fzf-args"), fzf_arguments().join("\n")).unwrap();
        let yazi = tool("EON_TEST_QUICK_YAZI");
        // Only the two clients within a shared-bus test use the same socket.
        let bus = start.shared_bus.unwrap_or_else(|| root.join("r"));
        fs::create_dir_all(&bus).unwrap();
        let gate = root.join("hold-yazi");
        if start.wait_for_first_ya {
            fs::write(&gate, "").unwrap();
        }
        script(
            &bin.join("ya"),
            &format!(
                "#!/bin/sh\n\
                 export XDG_RUNTIME_DIR='{bus}'\n\
                 if [ -e '{gate}' ]; then\n\
                   '{ya}' \"$@\"\n\
                   code=$?\n\
                   printf '%s\\n' \"$code\" > '{status}'\n\
                   rm -f '{gate}'\n\
                   exit \"$code\"\n\
                 fi\n\
                 exec '{ya}' \"$@\"\n",
                bus = bus.display(),
                gate = gate.display(),
                status = root.join("first-ya-status").display(),
                ya = yazi.with_file_name("ya").display()
            ),
        );
        let launch = root.join("launch");
        script(
            &launch,
            &format!(
                "#!/bin/sh\n\
                 export XDG_RUNTIME_DIR='{bus}' YAZI_CONFIG_HOME='{config}' _ZO_DATA_DIR='{root}/zoxide'\n\
                 export PATH='{root}/bin':\"$PATH\"\n\
                 export {marker_var}='{marker}' {fzf_args_var}=\"$(cat '{root}/fzf-args')\" {leave_status_var}='{leave_status}'\n\
                 export EON_FILES_CLI='{root}/bin/eon' EON_TAB=t1 EON_TERMINAL=terminal-1.g1\n\
                 {quick}{notice}\
                 while [ -e '{gate}' ]; do sleep 0.05; done\n\
                 cd '{files}' && exec '{yazi}' --client-id {id} '{files}'\n",
                root = root.display(),
                bus = bus.display(),
                gate = gate.display(),
                config = tool("EON_TEST_QUICK_FILES_CONFIG").display(),
                marker_var = crate::managed_environment::QUICK_MARKER_VARIABLE,
                fzf_args_var = crate::managed_environment::QUICK_FZF_ARGS_VARIABLE,
                leave_status_var = crate::managed_environment::QUICK_LEAVE_STATUS_VARIABLE,
                leave_status = LEAVE_STATUS,
                marker = marker.display(),
                files = files.display(),
                yazi = yazi.display(),
                quick = if start.quick {
                    "export EON_FILES_START=quick\n"
                } else {
                    ""
                },
                notice = if start.tutor_notice {
                    "export EON_FILES_NOTICE=''\n"
                } else {
                    ""
                },
            ),
        );
        if start.quick {
            quick_search_requested(&marker, INITIAL_REQUEST, None).unwrap();
        }
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let session = leased_orbit(&root, &format!("exec '{}'", launch.display()));
        if start.quick {
            quick_search_admitted(&marker, Instant::now());
        }
        Self {
            ya: bin.join("ya"),
            root,
            session,
            log,
            marker,
            id,
            dispatches: Dispatches::default(),
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
        self.wait_for(wanted)
            .unwrap_or_else(|observed| self.fail(what, &observed, None))
    }

    fn wait_for(&mut self, wanted: impl Fn(&str) -> bool) -> Result<String, String> {
        self.poll(Self::screen, |text| wanted(text))
    }

    /// Looks at the pair until `done` holds of what it sees, a supervisor
    /// turn before each look, for at most 10 s; the error is the last look.
    fn poll<T>(
        &mut self,
        observe: impl Fn(&mut Self) -> T,
        done: impl Fn(&T) -> bool,
    ) -> Result<T, T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.turn();
            let seen = observe(self);
            if done(&seen) {
                return Ok(seen);
            }
            if Instant::now() >= deadline {
                return Err(seen);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Polls something other than the screen until `done` holds of it, or
    /// fails naming `what` and the last thing seen.
    fn turns_until<T: std::fmt::Debug>(
        &mut self,
        what: &str,
        observe: impl Fn(&mut Self) -> T,
        done: impl Fn(&T) -> bool,
    ) -> T {
        self.poll(observe, done).unwrap_or_else(|seen| {
            panic!(
                "Expected {what}\nLast seen: {seen:?}\n{}",
                self.diagnostics()
            )
        })
    }

    fn fail(&self, expected: &str, observed: &str, baseline: Option<&str>) -> ! {
        let difference = baseline.map_or_else(String::new, |baseline| {
            let mut expected_rows = baseline.lines();
            let mut observed_rows = observed.lines();
            let mut rows = Vec::new();
            for row in 1.. {
                let lines = (expected_rows.next(), observed_rows.next());
                match lines {
                    (None, None) => break,
                    (before, after) if before != after => {
                        rows.push(format!("{row}: {before:?} -> {after:?}"));
                    }
                    _ => {}
                }
            }
            format!(
                "\nExpected Files:\n{baseline}\nChanged rows:\n{}",
                rows.join("\n")
            )
        });
        panic!(
            "Expected {expected}\nObserved screen:\n{observed}{difference}\n{}",
            self.diagnostics()
        );
    }

    fn diagnostics(&self) -> String {
        let markers: Vec<_> = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| marker_endpoint(&entry.path()).map(|_| entry.file_name()))
            .collect();
        let mut text = format!(
            "pair markers: {markers:?}\neon calls: {:?}\n",
            self.eon_calls()
        );
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
        self.until("settled Files with the alpha preview", Self::files_settled)
    }

    fn files_settled(text: &str) -> bool {
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
    }

    /// Quick search up for its keys: this search's fzf has written the ready
    /// marker, and its rows are exactly `folders`. The marker tells this
    /// search from the last one's kept frame (`--no-clear`); the rows, from
    /// a prompt fzf draws before it has read the history.
    fn quick_search_shown(&mut self, folders: &[&str]) {
        let shown = self.poll(
            |pair| (pair.marker.exists(), pair.screen()),
            |(ready, text)| *ready && Self::quick_search_lists(text, folders),
        );
        if let Err((_, text)) = shown {
            self.fail(&format!("quick search listing {folders:?}"), &text, None);
        }
    }

    /// A query typed into quick search, once fzf has narrowed its rows to
    /// `folders`, so Enter and Tab act on the matching row.
    fn queried(&mut self, query: &str, folders: &[&str]) {
        self.until(&format!("the query {query} listing {folders:?}"), |text| {
            text.contains(&format!("> {query}")) && Self::quick_search_lists(text, folders)
        });
    }

    /// Whether the rows below quick search's prompt name exactly `folders`,
    /// in any order, since Zoxide's order for the fixture's equal entries is
    /// not fixed. A row keeps
    /// its right end under `--keep-right`, so its folder is what follows
    /// the last `/w/files/`.
    fn quick_search_lists(text: &str, folders: &[&str]) -> bool {
        let mut rows: Vec<_> = text
            .lines()
            .skip_while(|line| !line.contains("Quick search >"))
            .skip(1)
            .filter_map(|line| Some(line.trim_end().rsplit_once("/w/files/")?.1))
            .collect();
        let mut folders = folders.to_vec();
        rows.sort_unstable();
        folders.sort_unstable();
        !folders.is_empty() && rows == folders
    }

    /// One supervisor turn for this shown pair.
    fn turn(&mut self) -> Option<LeaveStep> {
        drive_leave(&mut self.session, true, Instant::now(), &self.log)
    }

    /// Alt+Z on Files: `ya emit-to` asks the pair's Yazi for quick search.
    fn show_quick_search(&mut self) {
        self.dispatches.run(
            Switch::ShowQuickSearch,
            Some(&self.ya),
            self.id,
            self.marker.clone(),
            &self.log,
        );
    }

    /// Alt+Shift+Y on quick search, then the supervisor's turns until the
    /// leave is over; returns the copies of the key it wrote.
    fn leave(&mut self, what: &str, baseline: Option<&str>) -> u32 {
        self.dispatches.run(
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
                    let observed = self.screen();
                    self.fail(
                        &format!("{what}: fzf readiness and a completed leave ({detail})"),
                        &observed,
                        baseline,
                    );
                }
            }
            if Instant::now() >= deadline {
                let observed = self.screen();
                self.fail(
                    &format!("{what}: fzf readiness and a completed leave by the deadline"),
                    &observed,
                    baseline,
                );
            }
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
        self.wait_for(|text| Self::files_settled(text) && text == screen)
            .unwrap_or_else(|observed| self.fail(what, &observed, Some(screen)));
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

    /// The entries beside the pair's endpoint that quick search names, sorted.
    fn quick_entries(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().into_string().unwrap())
            .filter(|name| name.contains("quick"))
            .collect();
        names.sort();
        names
    }

    fn eon_calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("eon.log"))
            .unwrap_or_default()
            .lines()
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
    assert!(pair.leave("initial quick search", None) >= 1);
    let files = pair.files_shown();
    assert!(!pair.marker.exists());
    let _load = Load::start(2);
    for iteration in 0..200 {
        pair.show_quick_search();
        let what = format!("iteration {iteration}: Files as it was");
        let copies = pair.leave(&what, Some(&files));
        if copies == 0 {
            let observed = pair.screen();
            pair.fail(
                &format!("{what}: at least one leave key"),
                &observed,
                Some(&files),
            );
        }
        pair.back_to(&what, &files);
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
    let copies = pair.leave("slow fzf", Some(&files));
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
    pair.dispatches.run(
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

#[test]
fn reveal_navigates_the_live_pair_from_project_to_a_cold_file() {
    let mut pair = Pair::start(
        "reveal",
        Start {
            quick: true,
            ..Start::default()
        },
    );
    pair.quick_search_shown(&["alpha", "beta"]);
    leave_requested(&pair.marker);
    let cold = pair.root.join("w/unvisited/cold");
    fs::create_dir_all(&cold).unwrap();
    fs::write(cold.join("0-first.txt"), "WRONG").unwrap();
    let file = cold.join("z-hé ' \" $HOME\n.txt");
    fs::write(&file, "REVEALED").unwrap();
    let mut pump = StartPump::default();
    reveal(
        &mut pair.session,
        &RevealPath::validate(file.to_str().unwrap()).unwrap(),
        false,
        &pair.ya,
        &mut pump,
        &pair.log,
    )
    .unwrap_or_else(|error| panic!("{error}\n{}", pair.diagnostics()));
    assert!(!pairs().leaving.contains_key(&pair.marker));
    pair.until("the exact file preview in Files", |text| {
        text.contains("REVEALED") && !text.contains("Quick search >")
    });
    assert!(!pair.marker.exists());
    assert!(pair.eon_calls().is_empty());

    let directory = pair.root.join("w/unvisited/$HOME");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("directory.txt"), "DIRECTORY").unwrap();
    reveal(
        &mut pair.session,
        &RevealPath::validate(directory.to_str().unwrap()).unwrap(),
        true,
        &pair.ya,
        &mut pump,
        &pair.log,
    )
    .unwrap();
    pair.until("the literal directory and its listing", |text| {
        text.contains("directory.txt") && text.contains("$HOME")
    });
    let dotfile = directory.join(".hidden.txt");
    fs::write(&dotfile, "DOTFILE").unwrap();
    reveal(
        &mut pair.session,
        &RevealPath::validate(dotfile.to_str().unwrap()).unwrap(),
        false,
        &pair.ya,
        &mut pump,
        &pair.log,
    )
    .unwrap();
    pair.until("the hidden target preview", |text| text.contains("DOTFILE"));
    pair.stop();
}

#[test]
fn reveal_waits_for_the_exact_pair_on_an_empty_and_a_shared_dds_bus() {
    let mut first = Pair::start(
        "first cold Files pair",
        Start {
            wait_for_first_ya: true,
            ..Start::default()
        },
    );
    reveal_before_yazi_starts(&mut first, false, "FIRST REVEAL");

    let mut second = Pair::start(
        "second cold Files pair",
        Start {
            wait_for_first_ya: true,
            shared_bus: Some(first.root.join("r")),
            ..Start::default()
        },
    );
    assert_ne!(second.id, first.id);
    reveal_before_yazi_starts(&mut second, true, "SECOND REVEAL");
    assert!(first.screen().contains("FIRST REVEAL"));
    second.stop();
    first.stop();
}

fn reveal_before_yazi_starts(pair: &mut Pair, fresh: bool, preview: &str) {
    assert!(pair.root.join("hold-yazi").exists());
    let file = pair.root.join("w/files/alpha/z-target.txt");
    fs::write(&file, preview).unwrap();
    let result = reveal(
        &mut pair.session,
        &RevealPath::validate(file.to_str().unwrap()).unwrap(),
        fresh,
        &pair.ya,
        &mut StartPump::default(),
        &pair.log,
    );
    assert_eq!(
        fs::read_to_string(pair.root.join("first-ya-status")).unwrap(),
        "1\n",
        "an unrelated DDS peer must not acknowledge the requested instance"
    );
    result.unwrap_or_else(|error| panic!("{error}\n{}", pair.diagnostics()));
    pair.until("the requested preview", |text| text.contains(preview));
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
            tutor_notice: true,
            ..Start::default()
        },
    );
    pair.quick_search_shown(&["alpha", "beta"]);
    pair.until("the tutor header after readiness", |text| {
        text.contains("Tutor header fixture")
    });
    let mut keyboard = pair.keyboard();
    // The initial request lasts until the first turn after fzf's start
    // action writes the ready marker. Then the marker
    // is quick search's only entry beside the endpoint: no socket.
    pair.turns_until(
        "the ready marker as quick search's only entry",
        |pair| pair.quick_entries(),
        |entries| *entries == ["orbit.quick"],
    );
    assert!(fs::symlink_metadata(&pair.marker).unwrap().is_file());
    #[cfg(target_os = "linux")]
    {
        let mut seen = 0;
        for entry in fs::read_dir("/proc").unwrap().flatten() {
            let command = fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let command = String::from_utf8_lossy(&command);
            if command.contains(&*pair.marker.to_string_lossy())
                || command.contains(&fzf_arguments()[2])
            {
                seen += 1;
                assert!(!command.contains("--listen"), "{command}");
            }
        }
        assert!(seen >= 1, "found no quick search fzf");
    }

    // A query typed, then Alt+Shift+Y.
    keyboard.text("bet");
    pair.queried("bet", &["beta"]);
    assert!(pair.leave("quick search with a query", None) >= 1);
    pair.files_shown();
    pair.until("alpha's preview", |text| text.contains("a-inside"));

    // Enter picks the highlighted folder.
    pair.show_quick_search();
    pair.quick_search_shown(&["alpha", "beta"]);
    keyboard.text("bet");
    pair.queried("bet", &["beta"]);
    keyboard.press(PhysicalKey::ENTER);
    pair.turns_until(
        "the tab's directory set to beta",
        |pair| pair.eon_calls(),
        |calls| {
            calls.iter().any(|call| {
                call.starts_with("tab directory t1 -- ") && call.ends_with("/w/files/beta")
            })
        },
    );
    pair.files_shown();
    assert!(!pair.marker.exists());

    // Tab moves Files to the highlighted folder.
    pair.show_quick_search();
    pair.quick_search_shown(&["alpha", "beta"]);
    keyboard.text("alp");
    pair.queried("alp", &["alpha"]);
    keyboard.press(PhysicalKey::TAB);
    pair.until("Files in alpha", |text| {
        text.contains("Tab Search") && text.contains("files/alpha")
    });
    assert!(!pair.marker.exists());

    // Esc closes Files.
    pair.show_quick_search();
    // Tab left Files in alpha, which quick search leaves out.
    pair.quick_search_shown(&["beta"]);
    keyboard.press(PhysicalKey::ESCAPE);
    pair.turns_until(
        "Files hidden",
        |pair| pair.eon_calls(),
        |calls| calls.iter().any(|call| call == "popup hide u1"),
    );
    pair.until("Files in alpha again", |text| {
        text.contains("Tab Search") && text.contains("files/alpha")
    });
    assert!(!pair.marker.exists());
    drop(keyboard);
    pair.stop();
}
