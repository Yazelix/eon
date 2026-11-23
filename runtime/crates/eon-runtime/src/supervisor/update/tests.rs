//! A supervisor exports only at a settled boundary, and the export is the
//! owners' model plus the supervisor's own facts.

use super::super::{RegistryPending, StopReason, tests::admission_state};
use super::*;
use crate::{
    attention::{Attention, checkpoint::tests::seeded},
    checkpoint::update::{decode, model},
    control::failure,
    editor::Sockets,
    scrollback::Snapshots,
    session_registry::{Operation, Registry, Worker},
    sessions::{
        FailedStarts, RunningSession,
        tests::{finish, keep_status, tombstoned_session},
    },
    start_pump::{Capture, StartPump},
    supervisor::temporary_directory,
    workspace::{
        Session, SessionEnd, WorkspaceSet,
        checkpoint::tests::{Source, source_a, source_b},
        terminal_endpoint,
        tests::catalog,
    },
};
use eon_workspace_protocol::v47::{Action, Request, Response, WorkspaceAction, encode_request};
use orbit_protocol::management::{
    Foreground, ProcessIdentity, ProgramProcesses, ProgramRecord, ProgramState, ProgramStatus,
    TerminationReason,
};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::Command,
    sync::Arc,
};

const BOOT: Duration = Duration::from_secs(4_000);

/// A full-Eon supervisor over `set`, one leased row per slot, and the
/// three supervisor facts set.
struct Window {
    state: SupervisorState,
    root: PathBuf,
    runtime: PathBuf,
    _peers: Vec<UnixStream>,
}

impl Window {
    fn export(&mut self, now: Instant) -> Result<Vec<u8>> {
        self.state
            .export(&self.root, now, BOOT)
            .map(|export| export.bytes)
    }

    fn row(&mut self, terminal: &str) -> &mut RunningSession {
        self.state
            .sessions
            .iter_mut()
            .find(|session| session.id == terminal)
            .unwrap()
    }
}

fn window(source: Source) -> Window {
    let (core, update) = source.capture().unwrap();
    let mut sessions = Vec::new();
    let mut peers = Vec::new();
    for slot in update.slots(&core).unwrap() {
        let number = slot.run.terminal_id["terminal-".len()..].parse().unwrap();
        let (session, peer, _) =
            tombstoned_session(&source.runtime, number, TerminationReason::NaturalExit);
        sessions.push(session);
        peers.push(peer);
    }
    windowed(source.set, sessions, peers, source.root, source.runtime)
}

/// One session and tab whose panes are the leased `terminals`.
fn recovered(terminals: &[usize]) -> Window {
    let root = temporary_directory();
    let mut sessions = Vec::new();
    let mut peers = Vec::new();
    let mut panes = Vec::new();
    for &number in terminals {
        let (session, peer, _) = tombstoned_session(&root, number, TerminationReason::NaturalExit);
        panes.push((
            number,
            Session {
                id: session.id.clone(),
                endpoint: session.endpoint.clone(),
            },
        ));
        sessions.push(session);
        peers.push(peer);
    }
    let set =
        WorkspaceSet::with_recovered_sessions(root.clone(), root.clone(), None, panes, catalog())
            .unwrap();
    windowed(set, sessions, peers, root.clone(), root)
}

fn windowed(
    set: WorkspaceSet,
    sessions: Vec<RunningSession>,
    peers: Vec<UnixStream>,
    root: PathBuf,
    runtime: PathBuf,
) -> Window {
    let mut state = admission_state(&root, Some(set), sessions);
    state.attention = Some(Attention::new(true));
    state.started_at = 1_763_590_000_000;
    state.initial_child = ["sh", "-c", ""].map(OsString::from).to_vec();
    state.initial_status = Some(130);
    Window {
        state,
        root,
        runtime,
        _peers: peers,
    }
}

fn boot_ns() -> u64 {
    u64::try_from(BOOT.as_nanos()).unwrap()
}

#[test]
fn export_is_the_owners_model_and_the_supervisors_facts_for_its_scope() {
    for source in [source_a(), source_b()] {
        // Attention with records and raises on every running (not pending)
        // terminal.
        let (core, update) = source.capture().unwrap();
        let runs: Vec<_> = update
            .slots(&core)
            .unwrap()
            .iter()
            .filter(|slot| !slot.pending)
            .map(|slot| (slot.run.terminal_id.clone(), slot.run.run_id.clone()))
            .collect();
        let runs: Vec<_> = runs
            .iter()
            .map(|(terminal, run)| (terminal.as_str(), run.as_str()))
            .collect();
        let mut window = window(source);
        window.state.attention = Some(seeded(Instant::now(), &runs));
        let now = Instant::now();
        let export = window.state.export(&window.root, now, BOOT).unwrap();
        let (checkpoint, decoded) = decode(&export.bytes, &export.scope).unwrap();

        let state = &window.state;
        let (core, mut expected) = model::capture(
            state.workspaces.as_ref().unwrap(),
            &state.sessions,
            state.attention.as_ref(),
            1,
            now,
            boot_ns(),
        )
        .unwrap();
        expected.supervisor = Supervisor {
            started_at_ms: 1_763_590_000_000,
            initial_command: vec![b"sh".to_vec(), b"-c".to_vec(), Vec::new()],
            initial_status: Some(130),
        };
        assert_eq!((checkpoint.core, &decoded), (core, &expected));
        assert_eq!(
            checkpoint.header,
            Header {
                namespace: window.root.to_str().unwrap().into(),
                generation: state.component_generation.clone(),
                created_build: state.created_build.clone(),
                active_build: state.active_build.clone(),
            }
        );
        assert_eq!(export.scope.header, checkpoint.header);
        assert_eq!(export.scope.range, decoded.allocator.range);
        assert_eq!(decoded.captured_monotonic_ns, boot_ns());
        let attention = decoded.attention.as_ref().unwrap();
        assert_eq!(attention.terminals.len(), runs.len());
        assert!(
            attention
                .terminals
                .iter()
                .all(|terminal| !terminal.records.is_empty())
        );
    }
}

/// `seed` makes the settled window unsettled; export refuses with `what`;
/// after `clear` it exports the same bytes as before, so the refusal
/// changed nothing.
#[track_caller]
fn refuses<T>(
    what: &'static str,
    seed: impl FnOnce(&mut Window) -> T,
    clear: impl FnOnce(&mut Window, T),
) {
    let mut window = recovered(&[1]);
    let now = Instant::now();
    let before = window.export(now).unwrap();
    let seeded = seed(&mut window);
    assert_eq!(window.export(now), Err(Error::Unsettled(what)));
    clear(&mut window, seeded);
    assert_eq!(window.export(now).unwrap(), before);
}

#[test]
fn export_refuses_a_window_that_is_not_full_eon_stopping_or_holding_an_error() {
    refuses(
        "only a full Eon window can be updated",
        |window| window.state.attention.take(),
        |window, attention| window.state.attention = attention,
    );
    refuses(
        "the window is stopping",
        |window| window.state.stopping = Some(StopReason::StopRequired),
        |window, ()| window.state.stopping = None,
    );
    refuses(
        "a terminal end has an unreported error",
        |window| window.state.reconciliation = Some("terminal-1 lost its owner".into()),
        |window, ()| {
            // Read, never taken: the loop's next turn still reports it.
            assert_eq!(
                window.state.reconciliation.take().as_deref(),
                Some("terminal-1 lost its owner")
            );
        },
    );
    refuses(
        "the window has no terminal left",
        |window| std::mem::take(&mut window.state.sessions),
        |window, sessions| window.state.sessions = sessions,
    );
}

#[test]
fn export_refuses_registry_work_in_flight() {
    refuses(
        "a session registration is in progress",
        |window| window.state.registry_pending = Some(RegistryPending::Recovery("s1".into())),
        |window, ()| window.state.registry_pending = None,
    );
    refuses(
        "a session registration is in progress",
        |window| {
            let directory = temporary_directory().join("registry");
            let mut worker = Worker::start(Registry::in_test_directory(directory)).unwrap();
            worker
                .submit(Operation::Rename {
                    id: "missing".into(),
                    name: None,
                })
                .unwrap();
            window.state.registry_worker = Some(worker);
        },
        |window, ()| window.state.registry_worker = None,
    );
}

#[test]
fn export_refuses_a_held_client_and_failed_starts_and_pair_work() {
    refuses(
        "a control client is waiting for a terminal start",
        |window| {
            let path = window.root.join("pump.sock");
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let mut pump = StartPump::new(Some(listener), Arc::clone(&window.state.log));
            let mut client = UnixStream::connect(&path).unwrap();
            client
                .write_all(
                    &encode_request(&Request {
                        id: "held".into(),
                        caller: None,
                        action: Action::Workspace(WorkspaceAction::Inspect),
                    })
                    .unwrap(),
                )
                .unwrap();
            pump.wait(Instant::now() + Duration::from_millis(200));
            window.state.start_pump = pump;
            client
        },
        |window, _client| window.state.start_pump = StartPump::default(),
    );
    refuses(
        "a control client is waiting for a terminal start",
        |window| {
            window.state.start_pump.arm(Some(Capture {
                shown: Response::Failure(failure("captured", "shown")),
                workspaces: Vec::new(),
                callers: Vec::new(),
                runtime: None,
                stale_busy: None,
                fresh_until: None,
            }))
        },
        |window, ()| window.state.start_pump.disarm(),
    );
    refuses(
        "a failed terminal start is still being cleaned up",
        |window| {
            let child = Command::new("sleep").arg("0.2").spawn().unwrap();
            let private = window.root.join("claims");
            fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
            let claim = private.join("terminal-9.ready");
            window.state.failed_starts = FailedStarts::retained(child, &claim);
        },
        |window, ()| {
            let log = Arc::clone(&window.state.log);
            window
                .state
                .failed_starts
                .finish(Instant::now() + Duration::from_secs(5), &log)
                .unwrap();
        },
    );
    refuses(
        "a Files pair dispatch is running",
        |window| window.state.pair_dispatches = pair::Dispatches::running(),
        |window, ()| window.state.pair_dispatches = pair::Dispatches::default(),
    );
    refuses(
        "a Files pair is starting or leaving",
        |window| {
            let marker = pair::marker(&terminal_endpoint(&window.runtime, 9));
            pair::hold_starting(&marker);
            marker
        },
        |_, marker| pair::forget(&marker),
    );
    refuses(
        "a Files pair is starting or leaving",
        |window| {
            let marker = pair::marker(&terminal_endpoint(&window.runtime, 9));
            pair::hold_leaving(&marker);
            marker
        },
        |_, marker| pair::forget(&marker),
    );
}

#[test]
fn a_pair_of_another_window_does_not_unsettle_this_one() {
    let mut window = recovered(&[1]);
    let other = pair::marker(&terminal_endpoint(&temporary_directory(), 2));
    pair::hold_starting(&other);
    let exported = window.export(Instant::now());
    pair::forget(&other);
    assert!(exported.is_ok(), "{exported:?}");
}

#[test]
fn export_refuses_unsettled_snapshots_and_editor_sockets() {
    refuses(
        "a scrollback snapshot is not settled",
        |window| {
            let endpoint = terminal_endpoint(&window.runtime, 9);
            let directory = window.root.join("snapshot");
            window
                .state
                .scrollback
                .own_uncertain("terminal-9", directory, endpoint);
        },
        |window, ()| window.state.scrollback = Snapshots::new(&window.root.join("scrollback")),
    );
    refuses(
        "a scrollback snapshot is not settled",
        |window| {
            let directory = window.root.join("removed-snapshot");
            window.state.scrollback.retry_removal(directory);
        },
        |window, ()| window.state.scrollback = Snapshots::new(&window.root.join("scrollback")),
    );
    refuses(
        "an Editor socket is not settled",
        |window| {
            let endpoint = terminal_endpoint(&window.runtime, 9);
            window.state.editor_sockets = Sockets::tracking(endpoint);
        },
        |window, ()| window.state.editor_sockets = Sockets::default(),
    );
    // A listener whose terminal is a live row is settled.
    let mut window = recovered(&[1]);
    let endpoint = window.row("terminal-1").endpoint.clone();
    window.state.editor_sockets = Sockets::tracking(endpoint);
    assert!(window.export(Instant::now()).is_ok());
}

#[test]
fn export_refuses_supervisor_facts_the_checkpoint_cannot_carry() {
    for status in [328, -1] {
        refuses(
            "terminal-1's exit status does not fit the checkpoint",
            |window| window.state.initial_status = Some(status),
            |window, ()| window.state.initial_status = Some(130),
        );
    }
    refuses(
        "the window's initial command exceeds the checkpoint bounds",
        |window| {
            let command = vec![OsString::from("a"); 257];
            std::mem::replace(&mut window.state.initial_child, command)
        },
        |window, command| window.state.initial_child = command,
    );
}

fn status(update: u64, state: ProgramState) -> ProgramStatus {
    ProgramStatus {
        count: 1,
        background: 0,
        processes: Default::default(),
        foreground: Foreground::Child,
        update,
        terminal_title: String::new(),
        omitted: Vec::new(),
        records: vec![ProgramRecord {
            id: "agent".into(),
            state,
            kind: None,
            progress: None,
            app: "claude-code".into(),
            title: String::new(),
            message: String::new(),
            updated: update,
            created: update,
            changed: update,
            previous: None,
            updated_age_ms: 0,
            changed_age_ms: 0,
        }],
    }
}

/// A `state` status that needs a helper inspection on Linux: one counted
/// background process, a complete walk and a done record.
fn inspectable(update: u64, state: ProgramState) -> ProgramStatus {
    let identity = ProcessIdentity {
        pid: std::process::id(),
        start: 1,
    };
    let mut status = status(update, state);
    status.background = 1;
    status.processes = ProgramProcesses {
        background: vec![identity],
        complete: true,
        foreground: Some(identity),
    };
    status
}

/// The update number of terminal-1's attention in `export`.
fn attention_update(export: &Export) -> Option<u64> {
    let (_, update) = decode(&export.bytes, &export.scope).unwrap();
    update
        .attention
        .unwrap()
        .terminals
        .iter()
        .find(|terminal| terminal.terminal == "terminal-1")
        .map(|terminal| terminal.update)
}

/// C03: a kept answer refuses export until the turn's own poll consumes it
/// with its original deadline; it is never dropped.
#[test]
fn a_kept_status_answer_settles_through_the_turns_poll_never_dropped() {
    // An expired original deadline: the turn applies it at once, although
    // on Linux the status needs a helper inspection. A refreshed deadline
    // would start a sample instead, and export would refuse A1.
    let mut window = recovered(&[1]);
    keep_status(
        window.row("terminal-1"),
        &inspectable(5, ProgramState::Done),
        Instant::now() - Duration::from_millis(1),
    );
    assert_eq!(
        window.export(Instant::now()),
        Err(Error::Unsettled("a terminal status answer is not consumed"))
    );
    window.state.follow_attention();
    let export = window.state.export(&window.root, Instant::now(), BOOT);
    assert_eq!(attention_update(&export.unwrap()), Some(5));
}

/// A1: a helper sample in flight refuses export until a turn's poll applies
/// its result. The test holds the sample and delivers its result itself, so
/// no deadline or scheduling picks the branch.
#[test]
fn a_helper_sample_in_flight_refuses_until_its_result_is_applied() {
    use crate::attention;

    // Polls at `turn`, before the attention's first poll is due, so no poll
    // sends a request that would unsettle the row.
    let turn = Instant::now();
    let mut window = recovered(&[1]);
    let (sample, results) = window.state.attention.as_mut().unwrap().hold_sample(
        "terminal-1",
        "run-1",
        inspectable(6, ProgramState::Done),
        turn,
    );
    assert_eq!(
        window.export(Instant::now()),
        Err(Error::Unsettled("an attention helper sample is in flight"))
    );
    let targets = {
        let snapshots = window.state.workspaces.as_ref().unwrap().snapshots();
        attention::targets(&snapshots).0
    };
    // The turn's poll reads the delivered result before it expires any
    // sample by its deadline.
    results.send((sample, false)).unwrap();
    window
        .state
        .attention
        .as_mut()
        .unwrap()
        .poll(&targets, &mut window.state.sessions, turn);
    let export = window.state.export(&window.root, Instant::now(), BOOT);
    assert_eq!(attention_update(&export.unwrap()), Some(6));
}

/// The call site follows `tend`: a terminal a turn ended leaves attention
/// in the next turn's `tend`, before export sees it.
#[test]
fn a_terminal_ended_in_a_turn_is_pruned_by_the_next_tend_before_export() {
    let mut window = recovered(&[1, 2]);
    // Kept answers settle both terminals in one poll that sends nothing.
    for terminal in ["terminal-1", "terminal-2"] {
        keep_status(
            window.row(terminal),
            &status(3, ProgramState::Working),
            Instant::now() - Duration::from_millis(1),
        );
    }
    window.state.follow_attention();
    // The turn closes terminal-2 as a close request does.
    window
        .state
        .workspaces
        .as_mut()
        .unwrap()
        .session_ended("terminal-2", SessionEnd::Closed, |_| Ok(()))
        .unwrap();
    window
        .state
        .sessions
        .retain(|session| session.id != "terminal-2");
    assert_eq!(
        window.export(Instant::now()),
        Err(Error::Unsettled("attention holds a run no terminal has"))
    );
    window.state.tend(Instant::now(), None);
    assert!(window.export(Instant::now()).is_ok());
}

/// Export settles nothing itself: an ended pending pair refuses and its tab
/// stays pending, with no terminal started.
#[test]
fn export_starts_nothing_and_leaves_an_ended_pending_pair_to_the_turn() {
    let mut window = window(source_b());
    let pair = window.state.sessions[0].id.clone();
    finish(window.row(&pair));
    let rows = window.state.sessions.len();
    assert_eq!(
        window.export(Instant::now()),
        Err(Error::Unsettled("a terminal has ended"))
    );
    assert_eq!(window.state.sessions.len(), rows);
    let snapshot = window.state.workspaces.as_ref().unwrap().snapshot();
    assert!(snapshot.tabs[0].pending);
}

/// P10 observes an unreaped presentation exit without reporting it or
/// waiting for its relay; the turn's reap still sees it.
#[cfg(target_os = "linux")]
#[test]
fn an_unreaped_presentation_exit_refuses_without_waiting_or_reporting() {
    use std::thread;

    let mut window = recovered(&[1]);
    let now = Instant::now();
    let before = window.export(now).unwrap();
    // The child exits at once; a grandchild keeps its stderr open until the
    // test writes to its FIFO, so the relay stays unfinished until then.
    let fifo = window.root.join("relay.fifo");
    crate::fixtures::make_fifo(&fifo);
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "cat \"$1\" >/dev/null & exit 0", "sh"])
        .arg(&fifo);
    let mut venus = super::super::PresentationProcess::start(
        command,
        false,
        None,
        super::super::PRESENTATION_ADMISSION_TIMEOUT,
        &mut [],
        &window.state.log,
    )
    .unwrap();
    // A hang watchdog only.
    let watchdog = Instant::now() + Duration::from_secs(30);
    while !venus.has_exited().unwrap() {
        assert!(Instant::now() < watchdog, "presentation did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    window.state.venus = Some(venus);

    assert_eq!(
        window.export(now),
        Err(Error::Unsettled(
            "the presentation exited and is not yet reaped"
        ))
    );
    // Neither reported nor waited for: the relay is still running.
    let venus = window.state.venus.as_mut().unwrap();
    assert!(!venus.reported);
    assert!(
        venus
            .relay
            .as_ref()
            .is_some_and(|relay| !relay.is_finished())
    );
    // Released, the relay ends; the reap's own observation still sees the
    // exit and reports it once.
    drop(fs::OpenOptions::new().write(true).open(&fifo).unwrap());
    assert!(venus.exited().unwrap().is_some());
    assert!(venus.reported);

    window.state.venus = None;
    assert_eq!(window.export(now).unwrap(), before);
}
mod restore;
