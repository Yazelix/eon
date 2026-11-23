//! An import rebuilds the exported supervisor in the target build, or refuses
//! with the rows as they came, nothing sent to them, and nothing started,
//! written or removed.

use super::*;
use crate::{
    attention::{self, STALE_AFTER},
    checkpoint::update::TerminalExtra,
    cli::Inputs,
    editor::ListenerKind,
    generation::{current_generation, generation_directory},
    quota::Quotas,
    running,
    scrollback::tests::{OWNER_FILE as OWNER, history},
    session_registry::{self, Fault, WORKER_ENDED},
    supervisor::{ControlListener, LaunchMode, LaunchOptions, WindowSettings},
    workspace::checkpoint::tests::GENERATION,
};
use orbit_protocol::management::Screen;
use std::{
    collections::BTreeMap,
    io::{ErrorKind, Read},
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::Path,
    thread,
};

const ANOTHER_GENERATION: &str = "g1-00000000000000000000000000000002";

/// What the target build knows of one lifetime from its own launch.
struct Lifetime {
    root: PathBuf,
    generation: String,
    base: PathBuf,
    inputs: Inputs,
    settings: WindowSettings,
    registry: Option<Registry>,
    log: Arc<DiagnosticLog>,
    listener: ControlListener,
}

impl Lifetime {
    fn new(root: &Path, generation: &str, registry: Option<Registry>) -> Self {
        let inputs = crate::fixtures::inputs();
        let runtime = generation_directory(root, generation);
        fs::create_dir_all(&runtime).unwrap();
        let base = root.join(format!("base-{generation}"));
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        let config = root.join(format!("config-{generation}"));
        fs::create_dir(&config).unwrap();
        let log = DiagnosticLog::open_in(Ok(root.join("target-logs")), generation, "eon");
        let mut settings = WindowSettings::read(
            config,
            LaunchMode::Workspace,
            &inputs,
            &LaunchOptions::default(),
            Some(&log),
        )
        .unwrap();
        settings.display.popups = Some(catalog());
        let listener = ControlListener::bind(&runtime.join("eon.sock")).unwrap();
        Self {
            root: root.to_owned(),
            generation: generation.to_owned(),
            base,
            inputs,
            settings,
            registry,
            log,
            listener,
        }
    }

    fn target(&self) -> Target<'_> {
        Target {
            inputs: &self.inputs,
            settings: &self.settings,
            root: &self.root,
            generation: &self.generation,
            base: &self.base,
            registry: self.registry.clone(),
            log: Some(&self.log),
        }
    }

    fn runtime(&self) -> PathBuf {
        generation_directory(&self.root, &self.generation)
    }
}

/// Leftovers an import removes only once it has accepted the checkpoint: a
/// snapshot this process wrote for a terminal no row runs, and an Editor
/// socket pair of another such terminal.
struct Seeds {
    snapshot: PathBuf,
    socket: PathBuf,
}

impl Seeds {
    fn new(runtime: &Path) -> Self {
        let snapshot = Snapshots::new(runtime)
            .write("u99", &history(Screen::Primary, 0, 0))
            .unwrap();
        snapshot.mark("terminal-99").unwrap();
        let socket = Sockets::default()
            .prepare(&terminal_endpoint(runtime, 98), &[], ListenerKind::Helix)
            .unwrap();
        drop(UnixListener::bind(&socket).unwrap());
        Self {
            snapshot: snapshot.directory().to_owned(),
            socket,
        }
    }

    fn present(&self) -> bool {
        self.snapshot.exists()
            && self.socket.exists()
            && self.socket.with_extension("owner").exists()
    }
}

/// A window exported in its lifetime: the source supervisor (its rows
/// taken), the checkpoint, the rows and the target's side of that lifetime.
struct Exported {
    window: Window,
    bytes: Vec<u8>,
    scope: Scope,
    rows: Vec<RunningSession>,
    seeds: Seeds,
    now: Instant,
}

/// `source`'s window in its lifetime, its snapshots under its runtime.
fn source_window(source: Source) -> (Window, Registry) {
    let registry = source.registry.clone();
    let mut window = window(source);
    assert_eq!(window.state.component_generation, GENERATION);
    assert_eq!(
        window.runtime,
        generation_directory(&window.root, GENERATION)
    );
    window.state.scrollback = Snapshots::new(&window.runtime);
    (window, registry)
}

fn exported(source: Source) -> (Exported, Lifetime) {
    let (window, registry) = source_window(source);
    exported_window(window, Some(registry))
}

/// Every row carries a terminal extra the checkpoint restores, and the rows
/// moved to the target have it cleared, so an applied extra shows.
fn exported_window(mut window: Window, registry: Option<Registry>) -> (Exported, Lifetime) {
    for row in &mut window.state.sessions {
        row.restore_extra(&extra(&row.id, true));
    }
    let now = Instant::now();
    let export = window.state.export(&window.root, now, BOOT).unwrap();
    let mut rows = std::mem::take(&mut window.state.sessions);
    for row in &mut rows {
        row.restore_extra(&extra(&row.id, false));
    }
    let lifetime = Lifetime::new(&window.root, GENERATION, registry);
    let seeds = Seeds::new(&lifetime.runtime());
    let exported = Exported {
        window,
        bytes: export.bytes,
        scope: export.scope,
        rows,
        seeds,
        now,
    };
    (exported, lifetime)
}

fn extra(terminal: &str, default_shell: bool) -> TerminalExtra {
    TerminalExtra {
        terminal: terminal.into(),
        default_shell,
        named: None,
    }
}

impl Exported {
    fn import(
        &mut self,
        lifetime: &Lifetime,
        target: &Target,
        boot: Duration,
    ) -> std::result::Result<SupervisorState, Error> {
        let rows = std::mem::take(&mut self.rows);
        import(
            target,
            &self.bytes,
            &self.scope,
            rows,
            &lifetime.listener,
            self.now,
            boot,
        )
        .map_err(|(error, rows)| {
            self.rows = rows;
            error
        })
    }

    fn imported(&mut self, lifetime: &Lifetime) -> SupervisorState {
        self.import(lifetime, &lifetime.target(), BOOT).unwrap()
    }

    /// No Stop and no request reached any row's Orbit.
    fn rows_heard_nothing(&mut self) -> bool {
        self.window._peers.iter_mut().all(|peer| {
            peer.set_nonblocking(true).unwrap();
            matches!(peer.read(&mut [0; 64]), Err(error) if error.kind() == ErrorKind::WouldBlock)
        })
    }
}

/// Each row's terminal, run, endpoint and terminal extra.
fn rows_view(rows: &[RunningSession]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            format!(
                "{} {} {} {:?}",
                row.id,
                row.run_id(),
                row.endpoint.display(),
                row.checkpoint_extra()
            )
        })
        .collect()
}

type Tree = BTreeMap<PathBuf, (u32, Vec<u8>)>;

/// Every path under `directory` with its mode and, for files, its bytes.
fn tree(directory: &Path) -> Tree {
    let mut tree = BTreeMap::new();
    let mut pending = vec![directory.to_owned()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            pending.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        }
        let bytes = if metadata.is_file() {
            fs::read(&path).unwrap()
        } else {
            Vec::new()
        };
        tree.insert(path, (metadata.mode(), bytes));
    }
    tree
}

fn registry_tree(lifetime: &Lifetime) -> Tree {
    tree(&lifetime.root.join("sessions"))
}

fn starts() -> (usize, usize) {
    (session_registry::worker_starts(), running::tests::starts())
}

/// `import` with `target`, `bytes`, `scope` and `listener` refuses `what`
/// and returns the rows as they came; no row heard anything; no worker or
/// `Running` start was tried; the runtime and the registry are unchanged and
/// the seeds survive. With `validates`, `validate_target` refuses the same.
#[allow(clippy::too_many_arguments)]
#[track_caller]
fn refuses(
    exported: &mut Exported,
    lifetime: &Lifetime,
    target: &Target,
    bytes: &[u8],
    scope: &Scope,
    listener: Option<&ControlListener>,
    boot: Duration,
    what: Error,
    validates: bool,
) {
    let runtime = tree(&lifetime.runtime());
    let registry = registry_tree(lifetime);
    let started = starts();
    let rows = std::mem::take(&mut exported.rows);
    let view = rows_view(&rows);
    let listener = listener.unwrap_or(&lifetime.listener);
    let result = import(target, bytes, scope, rows, listener, exported.now, boot);
    let Err((error, rows)) = result else {
        panic!("import accepted a checkpoint it must refuse with {what}");
    };
    assert_eq!(error, what);
    assert_eq!(rows_view(&rows), view);
    exported.rows = rows;
    if validates {
        assert_eq!(
            validate_target(target, bytes, scope, exported.now, boot),
            Err(what)
        );
    }
    assert_eq!(starts(), started, "a worker or Running start was tried");
    assert_eq!(tree(&lifetime.runtime()), runtime);
    assert_eq!(registry_tree(lifetime), registry);
    assert!(exported.seeds.present());
    assert!(exported.rows_heard_nothing());
}

/// The bytes of `exported`'s checkpoint with its header or update changed.
fn reencoded(exported: &Exported, change: impl FnOnce(&mut Header, &mut Update)) -> Vec<u8> {
    let (checkpoint, mut update) = decode(&exported.bytes, &exported.scope).unwrap();
    let mut header = checkpoint.header;
    change(&mut header, &mut update);
    update::encode(header, checkpoint.core, &update, Vec::new()).unwrap()
}

/// Export, import, export again: the second checkpoint is the first, so the
/// import restored every carried owner, the allocator field by field, the
/// replay rings, the registrations and their errors, attention and the
/// Supervisor record. The target build is the active build; the registry is
/// read only and nothing registers.
#[test]
fn an_imported_window_exports_the_checkpoint_it_was_imported_from() {
    for source in [source_a(), source_b()] {
        let (mut window, registry) = source_window(source);
        let initial_child = vec![
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from_vec(b"printf '\xff'".to_vec()),
        ];
        window.state.initial_child = initial_child.clone();
        let (mut exported, lifetime) = exported_window(window, Some(registry));
        let registry = registry_tree(&lifetime);

        let mut state = exported.imported(&lifetime);
        assert_eq!(registry_tree(&lifetime), registry);
        assert!(state.registry_worker.as_ref().unwrap().is_idle());
        assert!(state.registry_pending.is_none());
        assert_eq!(state.started_at, 1_763_590_000_000);
        assert_eq!(state.initial_child, initial_child);
        assert_eq!(state.initial_status, Some(130));
        let (original, update) = decode(&exported.bytes, &exported.scope).unwrap();
        assert_eq!(state.created_build, original.header.created_build);
        let target_build = current_generation(&lifetime.inputs).unwrap();
        assert_ne!(target_build, original.header.active_build);
        assert_eq!(state.active_build, target_build);
        assert_eq!(state.component_generation, GENERATION);
        assert_eq!(state.application_id, WORKSPACE_APPLICATION_ID);
        assert_eq!(state.decorations, WORKSPACE_DECORATIONS);
        assert!(state.venus.is_none() && state.venus_path.is_none());
        assert!(
            state
                .sessions
                .iter()
                .all(|row| row.checkpoint_extra().unwrap().default_shell)
        );

        let again = state
            .export(&exported.window.root, exported.now, BOOT)
            .unwrap();
        let scope = Scope {
            header: Header {
                active_build: target_build,
                ..original.header.clone()
            },
            range: again.scope.range,
        };
        let (restored, restored_update) = decode(&again.bytes, &scope).unwrap();
        assert_eq!(restored.core, original.core);
        assert_eq!(restored_update.allocator, update.allocator);
        assert_eq!(restored_update, update);
        assert!(exported.rows_heard_nothing());
    }
}

/// Malformed bytes or a scope they do not match refuse through both entries
/// before anything is adopted, swept, started or registered.
#[test]
fn malformed_checkpoints_refuse_with_everything_untouched() {
    let (mut exported, lifetime) = exported(source_a());
    let other = format!("g1-{}", "9".repeat(32));
    let mut cases: Vec<(Vec<u8>, Scope)> = (0..4)
        .map(|field| {
            let other = other.clone();
            let bytes = reencoded(&exported, |header, _| match field {
                0 => header.generation = other,
                1 => header.created_build = other,
                2 => header.active_build = other,
                _ => header.namespace = "/elsewhere".into(),
            });
            (bytes, exported.scope.clone())
        })
        .collect();
    let mut range = exported.scope.clone();
    range.range += 1;
    cases.push((exported.bytes.clone(), range));
    let truncated = exported.bytes[..exported.bytes.len() - 1].to_vec();
    cases.push((truncated, exported.scope.clone()));
    let mut magic = exported.bytes.clone();
    magic[0] ^= 0xff;
    cases.push((magic, exported.scope.clone()));
    for (bytes, scope) in cases {
        let what = decode(&bytes, &scope).unwrap_err();
        refuses(
            &mut exported,
            &lifetime,
            &lifetime.target(),
            &bytes,
            &scope,
            None,
            BOOT,
            what,
            true,
        );
    }
    assert_eq!(starts(), (0, 0));
}

/// A valid checkpoint whose window or owners this target cannot restore
/// refuses before any start: eonbase's (no attention), a registration the
/// target registry lacks (R2), a popup entry its catalog lacks (R4), and a
/// missing row (R8).
#[test]
fn unrestorable_checkpoints_refuse_before_any_start() {
    let (mut exported, mut lifetime) = exported(source_a());
    let (bytes, scope) = (exported.bytes.clone(), exported.scope.clone());
    let eonbase = reencoded(&exported, |_, update| update.attention = None);
    let full_eon = Error::Unrestorable("only a full Eon window can be restored");
    refuses(
        &mut exported,
        &lifetime,
        &lifetime.target(),
        &eonbase,
        &scope,
        None,
        BOOT,
        full_eon,
        true,
    );

    let missing = Target {
        registry: Some(Registry::in_test_directory(
            lifetime.root.join("empty-sessions"),
        )),
        ..lifetime.target()
    };
    let what = Error::Unrestorable("a session's registration is missing");
    refuses(
        &mut exported,
        &lifetime,
        &missing,
        &bytes,
        &scope,
        None,
        BOOT,
        what,
        true,
    );

    let popups = lifetime.settings.display.popups.as_mut().unwrap();
    let entries = std::mem::take(&mut popups.entries);
    let what = Error::Unrestorable("a popup's entry is not in the target popup catalog");
    refuses(
        &mut exported,
        &lifetime,
        &lifetime.target(),
        &bytes,
        &scope,
        None,
        BOOT,
        what,
        true,
    );
    lifetime.settings.display.popups.as_mut().unwrap().entries = entries;

    let row = exported.rows.pop().unwrap();
    let what = Error::Unrestorable("the re-adopted terminals differ from the checkpoint");
    refuses(
        &mut exported,
        &lifetime,
        &lifetime.target(),
        &bytes,
        &scope,
        None,
        BOOT,
        what,
        false,
    );
    exported.rows.push(row);
    assert_eq!(starts(), (0, 0));
    exported.imported(&lifetime);
    assert_eq!(starts(), (1, 1));
}

/// `validate_target` accepts a valid checkpoint without rows, and reads only.
#[test]
fn validation_reads_only() {
    let (mut exported, lifetime) = exported(source_a());
    let runtime = tree(&lifetime.runtime());
    let registry = registry_tree(&lifetime);
    let now = exported.now;
    validate_target(
        &lifetime.target(),
        &exported.bytes,
        &exported.scope,
        now,
        BOOT,
    )
    .unwrap();
    assert_eq!(tree(&lifetime.runtime()), runtime);
    assert_eq!(registry_tree(&lifetime), registry);
    assert!(exported.seeds.present());
    assert_eq!(starts(), (0, 0));
    assert!(exported.rows_heard_nothing());
}

/// The lock file of `lifetime`'s registry, held exclusively.
fn hold_registry(lifetime: &Lifetime) -> fs::File {
    let lock = fs::File::open(lifetime.root.join("sessions/registry.lock")).unwrap();
    lock.lock().unwrap();
    lock
}

fn with_fault(lifetime: &Lifetime, fault: Fault) -> Target<'_> {
    let mut registry = lifetime.registry.clone().unwrap();
    registry.inject(Some(fault));
    Target {
        registry: Some(registry),
        ..lifetime.target()
    }
}

/// A busy registry is retried: the import waits for the holder and
/// succeeds. The hand-off is in the retry's pause, after which one more
/// attempt always runs, and the first busy answer comes after the
/// production retry limit, so the test races no clock.
#[test]
fn a_busy_registry_is_retried_until_it_is_free() {
    let (mut exported, lifetime) = exported(source_a());
    session_registry::FIRST_ATTEMPT_DELAY
        .with(|delay| delay.set(Some(Duration::from_millis(2_100))));
    let lock = hold_registry(&lifetime);
    let sessions = lifetime.root.join("sessions");
    let releaser = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !sessions.join(".tmp-test-retrying").exists() {
            if Instant::now() >= deadline {
                return Err("import never retried a busy registry");
            }
            thread::sleep(Duration::from_millis(5));
        }
        drop(lock);
        fs::write(sessions.join(".tmp-test-release"), b"release").unwrap();
        Ok(())
    });
    let target = with_fault(&lifetime, Fault::HandOffBusyRetry);
    let imported = exported.import(&lifetime, &target, BOOT);
    releaser.join().unwrap().unwrap();
    imported.unwrap();
}

/// A registry that stays busy past the retry limit refuses (R3).
#[test]
fn a_registry_busy_past_the_limit_refuses() {
    let (mut exported, lifetime) = exported(source_a());
    let lock = hold_registry(&lifetime);
    let target = with_fault(&lifetime, Fault::BusyRetryLimitZero);
    let (bytes, scope) = (exported.bytes.clone(), exported.scope.clone());
    let what = Error::Unrestorable("the session registry cannot be read");
    refuses(
        &mut exported,
        &lifetime,
        &target,
        &bytes,
        &scope,
        None,
        BOOT,
        what,
        false,
    );
    assert_eq!(
        validate_target(&target, &bytes, &scope, exported.now, BOOT),
        Ok(())
    );
    drop(lock);
    fs::remove_file(lifetime.root.join("sessions/registry.lock")).unwrap();
    assert_eq!(
        validate_target(&target, &bytes, &scope, exported.now, BOOT),
        Err(Error::Unrestorable("the session registry cannot be read"))
    );
}

/// The gap between capture and restore ages attention (C21): past the stale
/// bound it is stale and a poll is due at `now`; with no gap neither; a
/// capture after the restore's clock refuses.
#[test]
fn the_elapsed_gap_ages_attention_and_never_freshens_it() {
    for (gap, stale) in [
        (STALE_AFTER + Duration::from_millis(1), true),
        (Duration::ZERO, false),
    ] {
        let source = source_a();
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
        let (mut window, registry) = source_window(source);
        window.state.attention = Some(seeded(Instant::now(), &runs));
        let (mut exported, lifetime) = exported_window(window, Some(registry));
        let mut state = exported
            .import(&lifetime, &lifetime.target(), BOOT + gap)
            .unwrap();
        let now = exported.now;
        let attention = state.attention.as_mut().unwrap();
        assert_eq!(attention.fresh_until(now).is_none(), stale, "gap {gap:?}");
        let (targets, _) = attention::targets(&state.workspaces.as_ref().unwrap().snapshots());
        attention.poll(&targets, &mut state.sessions, now);
        assert_eq!(!exported.rows_heard_nothing(), stale, "gap {gap:?}");
    }
    let (mut exported, lifetime) = exported(source_a());
    let (bytes, scope) = (exported.bytes.clone(), exported.scope.clone());
    let early = BOOT - Duration::from_nanos(1);
    let what = Error::Unrestorable("the checkpoint was captured after the restore's clock");
    refuses(
        &mut exported,
        &lifetime,
        &lifetime.target(),
        &bytes,
        &scope,
        None,
        early,
        what,
        true,
    );
}

/// The snapshot store is adopted: an own snapshot of a live popup and a dead
/// writer's of another stay owned until their rows end, an own one of no
/// row is removed, and the next snapshot is numbered past every own one.
#[test]
fn scrollback_snapshots_are_adopted_by_the_imported_supervisor() {
    let (mut window, registry) = source_window(source_a());
    let terminals: Vec<String> = window
        .state
        .sessions
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let (live, dead_owner) = (terminals[1].clone(), terminals[2].clone());
    let snapshot = window
        .state
        .scrollback
        .write("u1", &history(Screen::Primary, 0, 0))
        .unwrap();
    snapshot.mark(&live).unwrap();
    let own = snapshot.directory().to_owned();
    window.state.scrollback.own(snapshot, live.clone(), None);
    let dead = window
        .runtime
        .join("snapshots")
        .join(format!("{}-1-1", u32::MAX));
    fs::create_dir(&dead).unwrap();
    fs::write(dead.join(OWNER), dead_owner.as_bytes()).unwrap();
    let (mut exported, lifetime) = exported_window(window, Some(registry));
    let mut state = exported.imported(&lifetime);
    assert!(own.exists() && dead.exists());
    assert!(!exported.seeds.snapshot.exists());
    let name = own.file_name().unwrap().to_str().unwrap().to_owned();
    let prefix = name.rsplit_once('-').unwrap().0;
    let next = state
        .scrollback
        .write("u2", &history(Screen::Primary, 0, 0))
        .unwrap();
    assert!(
        next.directory().ends_with(format!("{prefix}-3")),
        "{}",
        next.directory().display()
    );
    state.scrollback.discard(next).unwrap();
    let log = Arc::clone(&state.log);
    state
        .scrollback
        .retain(|terminal, _| terminal != live, &log);
    assert!(!own.exists() && dead.exists());
    state.scrollback.retain(
        |terminal, _| terminal != dead_owner && terminal != live,
        &log,
    );
    assert!(!dead.exists());
}

/// Whether `socket` fits macOS's 103-byte socket paths under the longest
/// test root there: eon-test's TMPDIR is at most 7 bytes and canonically
/// under /private, PIDs have up to 5 digits, and `temporary_directory` adds
/// its counter.
fn fits_every_platform(socket: &Path) -> bool {
    const LONGEST_ROOT: &str = "/private/tmp/e0/eon-test-99999-9999";
    let root = socket
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("eon-test-"))
        })
        .expect("test sockets live under a test root");
    LONGEST_ROOT.len() + 1 + socket.strip_prefix(root).unwrap().as_os_str().len() <= 103
}

/// An Editor listener of a live terminal is tracked by the imported
/// supervisor, and removed once that terminal's row ends.
#[test]
fn editor_listeners_are_tracked_by_the_imported_supervisor() {
    // The socket must fit macOS's limit under its longest test root, where
    // this fixture's terminal-3 leaves one byte.
    let (mut exported, lifetime) = exported(source_a());
    let row = exported
        .rows
        .iter()
        .find(|row| row.id != "terminal-1")
        .unwrap();
    let socket = Sockets::default()
        .prepare(&row.endpoint, &[], ListenerKind::Helix)
        .unwrap();
    drop(UnixListener::bind(&socket).unwrap());
    assert!(fits_every_platform(&socket), "{}", socket.display());
    let terminal = row.id.clone();
    let mut state = exported.imported(&lifetime);
    assert!(socket.exists() && socket.with_extension("owner").exists());
    assert!(!exported.seeds.socket.exists());
    let index = state
        .sessions
        .iter()
        .position(|row| row.id == terminal)
        .unwrap();
    let ended = state.sessions.remove(index);
    let log = Arc::clone(&state.log);
    state
        .editor_sockets
        .retain(&state.sessions, Instant::now(), &log);
    assert!(!socket.exists() && !socket.with_extension("owner").exists());
    state.sessions.push(ended);
}

/// The quota readers follow the target's `[bar]` widgets as a launch's do.
#[test]
fn quotas_follow_the_target_bar() {
    let (mut exported, mut lifetime) = exported(source_a());
    let widgets = vec!["codex_quota_5h".to_owned()];
    lifetime.settings.display.bar.widgets = widgets.clone();
    let state = exported.imported(&lifetime);
    let expected = Quotas::start(&lifetime.inputs.codexbar, &widgets).polled();
    assert!(!expected.is_empty());
    assert_eq!(state.quotas.as_ref().unwrap().polled(), expected);
}

/// A worker that cannot start, or a `Running` that fails after the worker
/// started, refuses with the rows as they came and nothing sent to them;
/// the started worker ends. The returned rows then import.
#[test]
fn a_failed_start_refuses_and_the_rows_import_again() {
    let (mut exported, lifetime) = exported(source_a());
    let what = Error::Unrestorable("the restored supervisor cannot start its workers");
    for fault in [Fault::WorkerSpawn, Fault::SignalWorkerEnd] {
        if fault == Fault::SignalWorkerEnd {
            running::tests::fail_next_start();
        }
        let target = with_fault(&lifetime, fault);
        let view = rows_view(&exported.rows);
        let (workers, runnings) = starts();
        assert_eq!(
            exported.import(&lifetime, &target, BOOT).err(),
            Some(what.clone())
        );
        assert_eq!(rows_view(&exported.rows), view);
        assert!(exported.seeds.present());
        assert!(exported.rows_heard_nothing());
        let after = starts();
        assert_eq!(after.0, workers + 1);
        if fault == Fault::WorkerSpawn {
            assert_eq!(after.1, runnings, "Running started after the worker failed");
        } else {
            assert_eq!(after.1, runnings + 1);
            let ended = lifetime.root.join("sessions").join(WORKER_ENDED);
            let deadline = Instant::now() + Duration::from_secs(30);
            while !ended.exists() {
                assert!(Instant::now() < deadline, "the started worker never ended");
                thread::sleep(Duration::from_millis(5));
            }
            fs::remove_file(ended).unwrap();
        }
    }
    exported.imported(&lifetime);
}

/// A target accepts only its own lifetime: a checkpoint whose bytes, scope
/// and rows agree with each other but belong to another lifetime refuses,
/// and so does a listener bound in another lifetime's runtime.
#[test]
fn a_checkpoint_of_another_lifetime_refuses() {
    let (mut exported, lifetime) = exported(source_a());
    let (bytes, scope) = (exported.bytes.clone(), exported.scope.clone());
    let elsewhere = temporary_directory();
    let others = [
        Lifetime::new(
            &lifetime.root,
            ANOTHER_GENERATION,
            lifetime.registry.clone(),
        ),
        Lifetime::new(&elsewhere, GENERATION, lifetime.registry.clone()),
    ];
    let another = || Error::Unrestorable("the checkpoint is for another window");
    for other in &others {
        // Another lifetime's target, this lifetime's checkpoint and rows.
        let now = exported.now;
        assert_eq!(
            validate_target(&other.target(), &bytes, &scope, now, BOOT),
            Err(another())
        );
        let listener = Some(&other.listener);
        refuses(
            &mut exported,
            &lifetime,
            &other.target(),
            &bytes,
            &scope,
            listener,
            BOOT,
            another(),
            false,
        );
        // This lifetime's target, checkpoint and rows, another's listener.
        refuses(
            &mut exported,
            &lifetime,
            &lifetime.target(),
            &bytes,
            &scope,
            listener,
            BOOT,
            another(),
            false,
        );
    }
    exported.imported(&lifetime);
}

/// A new tab in `directory` running the window's shell (a command tab needs
/// a caller in an Eon terminal).
#[cfg(target_os = "linux")]
fn launch_tab(directory: &Path) -> Action {
    Action::LaunchTab {
        directory: directory.as_os_str().as_encoded_bytes().to_vec(),
        argv: Vec::new(),
        focus: false,
    }
}

#[cfg(target_os = "linux")]
fn dispatch(
    state: &mut SupervisorState,
    lifetime: &Lifetime,
    id: &str,
    action: Action,
) -> crate::control::ControlResponse {
    let request = crate::supervisor::Request {
        id: id.into(),
        caller: None,
        action,
    };
    let runtime = lifetime.runtime();
    let config = lifetime.settings.config.clone();
    crate::supervisor::dispatch_control_request(
        request,
        state,
        &lifetime.inputs,
        &config,
        &runtime,
        &lifetime.generation,
    )
    .0
}

#[cfg(target_os = "linux")]
fn snapshot(state: &SupervisorState) -> eon_workspace_protocol::v49::Snapshot {
    state.workspaces.as_ref().unwrap().snapshot()
}

/// The Orbit hosts a real-terminal test started, by exact PID. If the test
/// fails before its own cleanup stops them, they are killed when this drops;
/// a passing test stops them itself.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct Hosts(Vec<i32>);

#[cfg(target_os = "linux")]
impl Hosts {
    fn track(&mut self, rows: &[RunningSession]) {
        for row in rows {
            let pid = crate::sessions::tests::orbit_pid(row);
            if !self.0.contains(&pid) {
                self.0.push(pid);
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Hosts {
    fn drop(&mut self) {
        if !thread::panicking() {
            return;
        }
        for &pid in &self.0 {
            if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn running(pid: i32) -> bool {
    matches!(crate::platform::process(pid.unsigned_abs()), Ok(Some(process)) if !process.ended)
}

/// A real terminal of these tests lives past a pause until it is stopped,
/// and a test that fails before stopping it leaves no host behind.
#[cfg(target_os = "linux")]
#[test]
fn a_failing_real_terminal_test_leaves_no_host() {
    let runtime = temporary_directory();
    let row =
        crate::sessions::tests::leased_orbit_at(&runtime, 2, "exec cat", Duration::from_secs(15));
    let pid = crate::sessions::tests::orbit_pid(&row);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut hosts = Hosts::default();
        hosts.track(std::slice::from_ref(&row));
        thread::sleep(Duration::from_secs(2));
        assert!(running(pid), "the terminal ended on its own");
        panic!("a failing assertion before cleanup");
    }));
    assert!(failed.is_err());
    let deadline = Instant::now() + Duration::from_secs(30);
    while running(pid) {
        assert!(
            Instant::now() < deadline,
            "the host outlived the failed test"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(row);
}

/// A pending window imported in place completes its picker exactly once
/// (C04): the dismissal starts the reserved `terminal-1` with the saved
/// initial command in the next saved pane, settles the pair once, and later
/// identities continue the saved counters. A second Update then carries the
/// completed request: replayed, it is a duplicate and changes nothing.
/// Terminals are real Orbit runs that never exit on their own; the test
/// stops them, or `Hosts` kills them if it fails first.
#[cfg(target_os = "linux")]
#[test]
fn a_pending_picker_completes_once_after_import_and_requests_stay_deduplicated() {
    use crate::sessions::{
        mark_pending_pair, pending_pair_marker, stop_managed_sessions, tests::leased_orbit_at,
    };
    use eon_workspace_protocol::v49::PopupTarget;

    let mut source = source_b();
    source.advance_ids(10);
    let registry = source.registry.clone();
    let number: usize = source.set.pairs().next().unwrap().0["terminal-".len()..]
        .parse()
        .unwrap();
    let pair = leased_orbit_at(&source.runtime, number, "exec cat", Duration::from_secs(15));
    let mut hosts = Hosts::default();
    hosts.track(std::slice::from_ref(&pair));
    mark_pending_pair(&pair.endpoint).unwrap();
    let marker = pending_pair_marker(&pair.endpoint);
    let sentinel = source.root.join("picked");
    let command = format!(
        "printf picked > {0}.tmp && mv {0}.tmp {0}; exec cat",
        sentinel.display()
    );
    let (root, runtime) = (source.root.clone(), source.runtime.clone());
    let mut window = windowed(source.set, vec![pair], Vec::new(), root, runtime.clone());
    window.state.scrollback = Snapshots::new(&runtime);
    window.state.initial_child = ["sh", "-c", &command].map(OsString::from).to_vec();
    window.state.initial_status = None;
    let pending = snapshot(&window.state)
        .tabs
        .into_iter()
        .find(|tab| tab.pending)
        .unwrap();
    let pick = PopupTarget {
        tab: pending.id.clone(),
        instance: pending.popups[0].id.clone(),
    };
    let (mut exported, mut lifetime) = exported_window(window, Some(registry));
    lifetime.inputs.orbit = std::env::var_os("ORBIT_BIN").unwrap().into();
    lifetime.inputs.defaults.shell.command = ["sh", "-c", "exec cat"].map(String::from).into();
    let saved = decode(&exported.bytes, &exported.scope)
        .unwrap()
        .1
        .allocator;
    let mut state = exported.imported(&lifetime);

    let settled = crate::supervisor::tests::SETTLED_PAIRS.with(std::cell::Cell::get);
    dispatch(&mut state, &lifetime, "pick", Action::DismissPopup(pick));
    hosts.track(&state.sessions);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sentinel.exists() {
        assert!(
            Instant::now() < deadline,
            "terminal-1 never ran the initial command"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(&sentinel).unwrap(), b"picked");
    let settled = crate::supervisor::tests::SETTLED_PAIRS.with(std::cell::Cell::get) - settled;
    assert_eq!(settled, 1);
    assert!(!marker.exists());
    assert!(state.initial_child.is_empty());
    let terminals: Vec<_> = state.sessions.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        terminals,
        [format!("terminal-{number}"), "terminal-1".into()]
    );
    assert_eq!(state.sessions[1].endpoint, terminal_endpoint(&runtime, 1));
    let picked = snapshot(&state);
    let tab = picked.tabs.iter().find(|tab| tab.id == pending.id).unwrap();
    assert!(!tab.pending);
    assert_eq!(tab.popups.len(), 1);
    assert_eq!(tab.columns[0].panes[0].session, "terminal-1");
    assert_eq!(tab.columns[0].panes[0].id, format!("p{}", saved.next_pane));

    let reply = dispatch(&mut state, &lifetime, "launch", launch_tab(&lifetime.root));
    hosts.track(&state.sessions);
    let launched = snapshot(&state);
    let tab = launched
        .tabs
        .iter()
        .find(|tab| tab.id == format!("t{}", saved.next_tab))
        .unwrap_or_else(|| panic!("no t{} in {launched:?} after {reply:?}", saved.next_tab));
    assert_eq!(
        tab.columns[0].panes[0].id,
        format!("p{}", saved.next_pane + 1)
    );
    assert_eq!(
        tab.columns[0].panes[0].session,
        format!("terminal-{}", saved.next_terminal)
    );

    // A second Update of the same window.
    let now = Instant::now();
    let export = state.export(&lifetime.root, now, BOOT).unwrap();
    let rows = std::mem::take(&mut state.sessions);
    drop(state);
    let target = lifetime.target();
    let imported = import(
        &target,
        &export.bytes,
        &export.scope,
        rows,
        &lifetime.listener,
        now,
        BOOT,
    );
    let mut state = imported.map_err(|(error, _)| error).unwrap();
    let before = (snapshot(&state), state.sessions.len());
    let replay = dispatch(&mut state, &lifetime, "launch", launch_tab(&lifetime.root));
    hosts.track(&state.sessions);
    assert!(
        matches!(
            &replay,
            crate::control::ControlResponse::Workspace(Response::Failure(failure))
                if failure.code == "duplicate-request"
        ),
        "{replay:?}"
    );
    assert_eq!((snapshot(&state), state.sessions.len()), before);
    dispatch(
        &mut state,
        &lifetime,
        "launch-again",
        launch_tab(&lifetime.root),
    );
    hosts.track(&state.sessions);
    assert_eq!(snapshot(&state).tabs.len(), before.0.tabs.len() + 1);
    assert_eq!(state.sessions.len(), before.1 + 1);
    stop_managed_sessions(&mut state.sessions, Duration::from_secs(10)).unwrap();
}
