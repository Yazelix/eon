//! The whole logical model through bytes and back, judged by what the restored
//! window then does.

use super::super::tests::header;
use super::super::{Scope, TerminalExtra, decode, encode};
use super::*;
use crate::attention::{self, STALE_AFTER, checkpoint::tests::seeded};
use crate::sessions::tests::{finish, rerun, tombstoned_session};
use crate::workspace::checkpoint::tests::{Source, source_a, source_b};
use crate::workspace::tests::{catalog, invoke, new_pane, prepared};
use crate::workspace::{SessionEnd, SessionOperation};
use eon_workspace_protocol::v50::{Action, WorkspaceAction};
use orbit_protocol::management::TerminationReason;
use std::io::Read;
use std::os::unix::{ffi::OsStrExt, net::UnixStream};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The window's re-adopted runs: one leased row per slot, `run-N` on
/// `terminal-N` at its derived endpoint, and the lease peers.
struct Rows {
    sessions: Vec<RunningSession>,
    peers: Vec<UnixStream>,
}

impl Rows {
    fn of(source: &Source) -> Self {
        let (core, update) = source.capture().unwrap();
        let mut rows = Rows {
            sessions: Vec::new(),
            peers: Vec::new(),
        };
        for slot in update.slots(&core).unwrap() {
            let number = slot.run.terminal_id["terminal-".len()..].parse().unwrap();
            rows.add(&source.runtime, number);
        }
        rows
    }

    fn add(&mut self, runtime: &Path, number: usize) {
        let (session, peer, _) =
            tombstoned_session(runtime, number, TerminationReason::NaturalExit);
        self.sessions.push(session);
        self.peers.push(peer);
    }

    fn extras(&self) -> Vec<TerminalExtra> {
        self.sessions
            .iter()
            .map(|session| session.checkpoint_extra().unwrap())
            .collect()
    }

    /// Forgets the extras, as a freshly re-adopted run has none.
    fn forget(&mut self) {
        for session in &mut self.sessions {
            let terminal = session.id.clone();
            session.restore_extra(&TerminalExtra {
                terminal,
                default_shell: false,
                named: None,
            });
        }
    }
}

fn pane_runs(source: &Source) -> Vec<(String, String)> {
    let (core, update) = source.capture().unwrap();
    update
        .slots(&core)
        .unwrap()
        .iter()
        .filter(|slot| !slot.pending)
        .map(|slot| (slot.run.terminal_id.clone(), slot.run.run_id.clone()))
        .collect()
}

fn attention_for(source: &Source, now: Instant) -> Attention {
    let live = pane_runs(source);
    let live: Vec<_> = live.iter().map(|(t, r)| (t.as_str(), r.as_str())).collect();
    seeded(now, &live[..2])
}

/// Capture, encode, decode: the bytes a target reads.
fn bytes(
    source: &Source,
    rows: &Rows,
    attention: Option<&Attention>,
    anchor: Instant,
) -> (Vec<u8>, Scope) {
    let (core, update) = capture(&source.set, &rows.sessions, attention, 7, anchor, 99).unwrap();
    let scope = Scope {
        header: header(),
        range: update.allocator.range,
    };
    (encode(header(), core, &update, Vec::new()).unwrap(), scope)
}

fn restore_bytes(
    bytes: &[u8],
    scope: &Scope,
    source: &Source,
    registry: Registry,
    rows: &mut Rows,
    now: Instant,
    elapsed: Duration,
) -> Result<(WorkspaceSet, Option<Attention>)> {
    let (checkpoint, update) = decode(bytes, scope).unwrap();
    rows.sessions.reverse();
    restore(
        &checkpoint.core,
        &update,
        source.runtime.clone(),
        Some(registry),
        catalog(),
        &mut rows.sessions,
        now,
        elapsed,
    )
}

/// R8, R1-R7, then the extras: the restore half of an import, as one call.
#[allow(clippy::too_many_arguments)]
fn restore(
    core: &Core,
    update: &Update,
    runtime: PathBuf,
    registry: Option<Registry>,
    catalog: PopupCatalog,
    sessions: &mut [RunningSession],
    now: Instant,
    elapsed: Duration,
) -> Result<(WorkspaceSet, Option<Attention>)> {
    let order = match_rows(core, update, &runtime, sessions)?;
    let restored = validate(
        core, update, runtime, registry, catalog, now, elapsed, false,
    )?;
    apply_extras(sessions, &order, update);
    Ok(restored)
}

/// A registry directory copied from `source`'s, so two windows can each
/// create the same next session.
fn copied_registry(source: &Source) -> Registry {
    let copy = crate::supervisor::temporary_directory().join("sessions");
    fs::create_dir_all(&copy).unwrap();
    let original = source.root.join("sessions");
    fs::set_permissions(&copy, fs::metadata(&original).unwrap().permissions()).unwrap();
    for (path, _) in source.registry_bytes() {
        fs::copy(&path, copy.join(path.file_name().unwrap())).unwrap();
    }
    Registry::in_test_directory(copy)
}

#[test]
fn the_whole_model_roundtrips_through_bytes_with_its_extras() {
    let source = source_a();
    let mut rows = Rows::of(&source);
    let terminal = rows.sessions[0].id.clone();
    rows.sessions[0].restore_extra(&TerminalExtra {
        terminal,
        default_shell: true,
        named: Some(super::super::Named {
            process_id: 41,
            process_start: 9,
        }),
    });
    let extras = rows.extras();
    let anchor = Instant::now();
    let attention = attention_for(&source, anchor);
    let (bytes, scope) = bytes(&source, &rows, Some(&attention), anchor);

    rows.forget();
    let (restored, restored_attention) = restore_bytes(
        &bytes,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        anchor,
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(restored, source.set);
    rows.sessions.reverse();
    assert_eq!(rows.extras(), extras);
    let first = &restored.pane_sessions()[0];
    assert_eq!(
        (first.id.as_str(), first.endpoint.clone()),
        ("terminal-1", source.runtime.join("orbit.sock"))
    );

    // The restored model captures to the same bytes.
    let again = Source {
        set: restored,
        registry: source.registry.clone(),
        root: source.root.clone(),
        runtime: source.runtime.clone(),
    };
    assert_eq!(
        self::bytes(&again, &rows, restored_attention.as_ref(), anchor).0,
        bytes
    );
}

/// Each operation the windows' `operate` saw.
fn recorded(
    set: &mut WorkspaceSet,
    request: &str,
    action: Action,
) -> (std::result::Result<(), String>, Vec<SessionOperation>) {
    let mut operations = Vec::new();
    let result = set
        .dispatch(request, None, action, prepared, |operation| {
            operations.push(operation);
            Ok(())
        })
        .map_err(|failure| failure.code);
    (result, operations)
}

#[test]
fn a_restored_pending_window_picks_and_creates_the_same_ids() {
    let source = source_b();
    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, None, Instant::now());
    let registry = copied_registry(&source);
    let (mut restored, _) = restore_bytes(
        &bytes,
        &scope,
        &source,
        registry,
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    let mut original = source.set.clone();

    let tab = original.snapshot().tabs[0].id.clone();
    let pick = Action::Workspace(WorkspaceAction::SetTabDirectory {
        tab,
        directory: source.root.as_os_str().as_bytes().to_vec(),
    });
    let (result, operations) = recorded(&mut restored, "pick", pick.clone());
    result.unwrap();
    assert_eq!(recorded(&mut original, "pick", pick).1, operations);
    // The same pair commits once, and the first pane takes reserved terminal-1.
    let marks: Vec<_> = operations
        .iter()
        .filter(|operation| matches!(operation, SessionOperation::MarkPair { .. }))
        .collect();
    assert_eq!(
        marks,
        [&SessionOperation::MarkPair {
            session: "terminal-2".into(),
            pending: false,
        }]
    );
    assert!(operations.iter().any(|operation| matches!(
        operation,
        SessionOperation::Start { session, .. }
            if session.id == "terminal-1" && session.endpoint == source.runtime.join("orbit.sock")
    )));

    creates_the_same_ids(&source, &mut original, &mut restored);
}

#[test]
fn a_restored_window_creates_the_same_ids() {
    let source = source_a();
    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, None, Instant::now());
    let (mut restored, _) = restore_bytes(
        &bytes,
        &scope,
        &source,
        copied_registry(&source),
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    let mut original = source.set.clone();
    creates_the_same_ids(&source, &mut original, &mut restored);
}

/// A pane, a column, a popup, a tab and a session: each succeeds with the
/// same operations on both windows, and the windows stay equal.
fn creates_the_same_ids(source: &Source, original: &mut WorkspaceSet, restored: &mut WorkspaceSet) {
    let tab = restored.snapshot().active_tab;
    for (request, action) in [
        ("pane", new_pane(true)),
        ("column", Action::CreateColumn),
        ("popup", invoke(&tab, "git")),
        ("tab", Action::Workspace(WorkspaceAction::CreateTab)),
    ] {
        let (result, operations) = recorded(restored, request, action.clone());
        assert_eq!(result, Ok(()), "{request}");
        assert_eq!(
            recorded(original, request, action).1,
            operations,
            "{request}"
        );
    }
    // One registration for both: its random ID is the registry's, not the window's.
    let registration = source.registry.create(None).unwrap();
    for set in [&mut *original, &mut *restored] {
        assert_eq!(new_session(set, Some(registration.clone())), Ok(()));
    }
    // The sets differ only in which registry directory each window uses.
    assert_eq!(restored.snapshots(), original.snapshots());
}

#[test]
fn a_replayed_request_is_a_duplicate_and_starts_nothing() {
    let source = source_a();
    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, None, Instant::now());
    let (mut restored, _) = restore_bytes(
        &bytes,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    let (_, update) = source.capture().unwrap();
    let mutation = update.recent_mutations.last().unwrap();
    let inspection = update.recent_inspections.last().unwrap();
    for replayed in [mutation, inspection] {
        let (result, operations) = recorded(&mut restored, replayed, new_pane(true));
        assert_eq!(result, Err("duplicate-request".into()), "{replayed}");
        assert!(operations.is_empty());
    }
}

#[test]
fn a_gap_over_the_stale_bound_makes_attention_stale_and_polling_due() {
    for (gap, stale) in [
        (STALE_AFTER + Duration::from_millis(1), true),
        (Duration::ZERO, false),
    ] {
        // Fresh rows each time: a row with a status request in flight is
        // never asked again, so a reused one would hide a due poll.
        let source = source_a();
        let anchor = Instant::now();
        let attention = attention_for(&source, anchor);
        let mut rows = Rows::of(&source);
        let (bytes, scope) = bytes(&source, &rows, Some(&attention), anchor);
        let now = anchor + Duration::from_secs(60);
        let (restored, attention) = restore_bytes(
            &bytes,
            &scope,
            &source,
            source.registry.clone(),
            &mut rows,
            now,
            gap,
        )
        .unwrap();
        let mut attention = attention.unwrap();
        assert_eq!(attention.fresh_until(now).is_none(), stale);
        let (targets, _) = attention::targets(&restored.snapshots());
        attention.poll(&targets, &mut rows.sessions, now);
        // A due poll asks each target's Orbit for its status.
        let mut asked = false;
        for peer in &mut rows.peers {
            peer.set_nonblocking(true).unwrap();
            while peer.read(&mut [0; 4096]).is_ok_and(|read| read > 0) {
                asked = true;
            }
        }
        assert_eq!(asked, stale, "gap {gap:?}");
    }
}

#[test]
fn the_next_attention_jump_starts_at_the_queue_front() {
    let source = source_a();
    let anchor = Instant::now();
    let mut attention = attention_for(&source, anchor);
    let snapshots = source.set.snapshots();
    let (_, front, key) = attention.jump(&snapshots, true).unwrap();
    attention.handled(&front, Some(key), &snapshots, false, false);
    let (_, second, _) = attention.jump(&snapshots, true).unwrap();
    assert_ne!(second, front);

    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, Some(&attention), anchor);
    let (restored, restored_attention) = restore_bytes(
        &bytes,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        anchor,
        Duration::ZERO,
    )
    .unwrap();
    let (_, next, _) = restored_attention
        .unwrap()
        .jump(&restored.snapshots(), true)
        .unwrap();
    assert_eq!(next, front);
}

#[test]
fn a_covered_popup_that_ended_restores_without_its_cover() {
    let mut source = source_a();
    let tab = &source.set.snapshots()[1].tabs[0];
    let terminal = |entry: &str| {
        tab.popups
            .iter()
            .find(|popup| popup.entry == entry)
            .unwrap()
            .session
            .clone()
    };
    let (agent, commands) = (terminal("agent"), terminal("commands"));
    source
        .set
        .session_ended(&agent, SessionEnd::Exited, |_| Ok(()))
        .unwrap();
    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, None, Instant::now());
    let (_, update) = decode(&bytes, &scope).unwrap();
    assert_eq!(update.tab_extras[1].covered_popup, None);
    let (mut restored, _) = restore_bytes(
        &bytes,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    // When the transient ends, both show the same popup: none.
    for set in [&mut source.set, &mut restored] {
        set.session_ended(&commands, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
    }
    assert_eq!(restored.snapshots(), source.set.snapshots());
    assert_eq!(restored.snapshots()[1].tabs[0].selected_popup, None);
}

fn new_session(
    set: &mut WorkspaceSet,
    registration: Option<crate::session_registry::Registration>,
) -> std::result::Result<(), String> {
    set.apply_action(
        "new-session",
        None,
        Action::Workspace(WorkspaceAction::NewWorkspace {
            name: None,
            focus: true,
        }),
        registration,
        prepared,
        |_| Ok(()),
    )
    .map_err(|failure| failure.code)
}

#[test]
fn a_home_the_wire_refuses_restores_none_and_an_absent_one_is_kept() {
    let mut source = source_a();
    source.set_home(1, Some("rel".into()));
    let mut rows = Rows::of(&source);
    let (bytes, scope) = bytes(&source, &rows, None, Instant::now());
    let (mut restored, _) = restore_bytes(
        &bytes,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    let (_, update) = decode(&bytes, &scope).unwrap();
    assert_eq!(update.session_extras[1].home, None);
    for set in [&mut source.set, &mut restored] {
        assert_eq!(new_session(set, None), Err("invalid-directory".into()));
    }

    let mut source = source_a();
    let home = source.root.join("home-returns");
    source.set_home(1, Some(home.clone()));
    let mut rows = Rows::of(&source);
    let (saved, scope) = self::bytes(&source, &rows, None, Instant::now());
    let (mut restored, _) = restore_bytes(
        &saved,
        &scope,
        &source,
        source.registry.clone(),
        &mut rows,
        Instant::now(),
        Duration::ZERO,
    )
    .unwrap();
    fs::create_dir_all(&home).unwrap();
    for set in [&mut source.set, &mut restored] {
        let registration = source.registry.create(None).unwrap();
        assert_eq!(new_session(set, Some(registration)), Ok(()));
    }
}

#[test]
fn rows_and_slots_must_be_one_to_one_for_capture() {
    // Each case has its own window: one runtime holds one record per terminal.
    let refuse = |change: fn(&Source, &mut Rows), what| {
        let source = source_a();
        let mut rows = Rows::of(&source);
        change(&source, &mut rows);
        let before = source.set.clone();
        let result = capture(&source.set, &rows.sessions, None, 7, Instant::now(), 99);
        assert_eq!(result.map(|_| ()), Err(Error::Unsettled(what)));
        assert_eq!(source.set, before);
    };
    refuse(
        |source, rows| rows.add(&source.runtime, 50),
        "a terminal has no place in the window",
    );
    refuse(
        |source, rows| {
            rows.add(&source.runtime, 50);
            finish(rows.sessions.last_mut().unwrap());
        },
        "a terminal has ended",
    );
    refuse(
        |_, rows| finish(&mut rows.sessions[0]),
        "a terminal has ended",
    );
    refuse(
        |_, rows| drop(rows.sessions.remove(0)),
        "a terminal has no live run",
    );
    refuse(
        |_, rows| {
            let number = rows.sessions[0].number;
            rows.add(&crate::supervisor::temporary_directory(), number);
        },
        "two terminal rows share a terminal",
    );
}

/// What a refused restore is given beyond the checkpoint's own window.
struct Target {
    runtime: Option<PathBuf>,
    registry: bool,
    elapsed: Duration,
}

const SAME: Target = Target {
    runtime: None,
    registry: true,
    elapsed: Duration::ZERO,
};

/// Restores a capture whose first row carried extras into rows that have
/// forgotten them, after `change`; asserts the refusal and that no row and no
/// registry byte changed.
fn refused(change: fn(&Source, &mut Rows), target: Target, what: &'static str) {
    // Each case has its own window: one runtime holds one record per terminal.
    let source = source_a();
    let mut rows = Rows::of(&source);
    let terminal = rows.sessions[0].id.clone();
    rows.sessions[0].restore_extra(&TerminalExtra {
        terminal,
        default_shell: true,
        named: Some(super::super::Named {
            process_id: 41,
            process_start: 9,
        }),
    });
    let anchor = Instant::now();
    let attention = attention_for(&source, anchor);
    let (bytes, scope) = bytes(&source, &rows, Some(&attention), anchor);
    rows.forget();
    change(&source, &mut rows);
    let registry = source.registry_bytes();
    let (checkpoint, update) = decode(&bytes, &scope).unwrap();
    let result = restore(
        &checkpoint.core,
        &update,
        target.runtime.unwrap_or_else(|| source.runtime.clone()),
        target.registry.then(|| source.registry.clone()),
        catalog(),
        &mut rows.sessions,
        anchor,
        target.elapsed,
    );
    assert_eq!(result.map(|_| ()), Err(Error::Unrestorable(what)));
    assert!(
        rows.sessions
            .iter()
            .filter_map(|row| row.checkpoint_extra().ok())
            .all(|extra| !extra.default_shell && extra.named.is_none()),
        "{what}"
    );
    assert_eq!(source.registry_bytes(), registry);
}

#[test]
fn restore_refuses_rows_that_differ_from_the_slots_and_changes_nothing() {
    let readopted = "the re-adopted terminals differ from the checkpoint";
    // Another runtime: every derived endpoint differs from the rows'.
    refused(
        |_, _| {},
        Target {
            runtime: Some(crate::supervisor::temporary_directory()),
            ..SAME
        },
        readopted,
    );
    refused(
        |source, rows| rows.add(&source.runtime, 50),
        SAME,
        readopted,
    );
    refused(|_, rows| drop(rows.sessions.pop()), SAME, readopted);
    refused(|_, rows| finish(&mut rows.sessions[1]), SAME, readopted);
    // Another run of a slot's terminal at its endpoint.
    refused(
        |_, rows| rerun(&mut rows.sessions[0], "run-new"),
        SAME,
        readopted,
    );
    // Two rows on one terminal, the count kept.
    refused(
        |_, rows| {
            let number = rows.sessions[0].number;
            rows.sessions.pop();
            rows.add(&crate::supervisor::temporary_directory(), number);
        },
        SAME,
        readopted,
    );
}

#[test]
fn a_restore_refused_after_re_adoption_still_changes_no_row() {
    // R7: attention times this platform cannot hold.
    refused(
        |_, _| {},
        Target {
            elapsed: Duration::MAX,
            ..SAME
        },
        "an attention time is out of range",
    );
    // R1: the window that saved had a registry, this target has none.
    refused(
        |_, _| {},
        Target {
            registry: false,
            ..SAME
        },
        "the session registry availability differs",
    );
}
