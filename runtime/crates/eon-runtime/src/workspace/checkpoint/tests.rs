use super::super::tests::{catalog, invoke, new_pane, prepared};
use super::*;
use crate::supervisor::temporary_directory;
use eon_workspace_protocol::v49::{Action, WorkspaceAction};
use std::fs;

/// A window built through the owner's own operations, its registry, and the
/// directory its runtime and rows use.
pub(crate) struct Source {
    pub set: WorkspaceSet,
    pub registry: Registry,
    pub root: PathBuf,
    pub runtime: PathBuf,
}

impl Source {
    pub(crate) fn dispatch(&mut self, action: Action) {
        let id = format!("request-{}", self.set.recent_mutations.len());
        self.set
            .dispatch(&id, None, action, prepared, |_| Ok(()))
            .unwrap();
    }

    /// Every slot's terminal, its run is `run-N`.
    pub(crate) fn run(terminal: &str) -> Option<String> {
        terminal
            .strip_prefix("terminal-")
            .map(|number| format!("run-{number}"))
    }

    /// Sets session `index`'s home as the owner stores it: unchecked.
    pub(crate) fn set_home(&mut self, index: usize, home: Option<PathBuf>) {
        self.set.workspaces[index].home = home;
    }

    /// Moves every identity counter `by` past where it stands, as a window
    /// that has lived a while has them. The Linux-only picker restore test uses it.
    #[cfg(target_os = "linux")]
    pub(crate) fn advance_ids(&mut self, by: usize) {
        let ids = &mut self.set.ids;
        for counter in [
            &mut ids.next_workspace,
            &mut ids.next_tab,
            &mut ids.next_pane,
            &mut ids.next_session,
            &mut ids.next_popup,
        ] {
            *counter += by;
        }
    }

    pub(crate) fn capture(&self) -> Result<(Core, Update)> {
        self.set.checkpoint(7, &Self::run)
    }

    pub(crate) fn restore(&self, core: &Core, update: &Update) -> Result<WorkspaceSet> {
        WorkspaceSet::restore(
            core,
            update,
            self.runtime.clone(),
            Some(self.registry.clone()),
            catalog(),
            false,
        )
    }

    /// The bytes of every registry file, to show a restore wrote none.
    pub(crate) fn registry_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(self.root.join("sessions"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| {
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        files.sort();
        files
    }
}

/// The lifetime the sources' runtimes belong to, so a supervisor can
/// export and import them.
pub(crate) const GENERATION: &str = "g1-00000000000000000000000000000001";

fn empty_root() -> (PathBuf, PathBuf, Registry) {
    let root = temporary_directory();
    let runtime = crate::generation::generation_directory(&root, GENERATION);
    fs::create_dir_all(&runtime).unwrap();
    let registry = Registry::in_test_directory(root.join("sessions"));
    (root, runtime, registry)
}

/// Source A: a recovered first session on terminal-1 (orbit.sock) whose
/// registration came back missing, then a committed session with a hidden
/// pane, a second column, a pinned and tagged pane, a durable popup covered by
/// a shown transient, and a pending third session; a nonzero identity range,
/// rings and recency.
pub(crate) fn source_a() -> Source {
    let (root, runtime, registry) = empty_root();
    let first = Session {
        id: "terminal-1".into(),
        endpoint: terminal_endpoint(&runtime, 1),
    };
    let set = WorkspaceSet::registered_recovered(
        Some(registry.clone()),
        None,
        IdAllocator::new(runtime.clone(), 3).unwrap(),
        root.clone(),
        Some(root.clone()),
        vec![(1, first)],
        catalog(),
    )
    .unwrap();
    let mut source = Source {
        set,
        registry,
        root: root.clone(),
        runtime,
    };
    let recovered = source.set.workspaces[0].id.clone();
    source.set.registered_recovery(&recovered, Ok(None));

    let registration = source.registry.create(Some("fresh")).unwrap();
    source
        .set
        .insert_registered_pending(
            registration,
            root.clone(),
            Some(root.clone()),
            catalog(),
            true,
            prepared,
            |_| Ok(()),
        )
        .unwrap();
    source.set.shown = 1;
    source.set.recent_workspaces.rotate_left(1);
    let tab = source.set.workspaces[1].tabs[0].id.clone();
    source.dispatch(Action::Workspace(WorkspaceAction::SetTabDirectory {
        tab: tab.clone(),
        directory: root.as_os_str().as_bytes().to_vec(),
    }));
    source.dispatch(new_pane(true));
    source.dispatch(Action::CreateColumn);
    let pane = source.set.workspaces[1].tabs[0].columns[0].panes[0]
        .id
        .clone();
    source.dispatch(Action::RenamePane {
        pane,
        name: Some("build".into()),
        tag: Some("ci".into()),
    });
    source.dispatch(invoke(&tab, "agent"));
    source.dispatch(invoke(&tab, "commands"));
    source
        .set
        .dispatch(
            "inspect-0",
            None,
            Action::Workspace(WorkspaceAction::Inspect),
            prepared,
            |_| panic!("inspection operates on no terminals"),
        )
        .unwrap();

    let registration = source.registry.create(None).unwrap();
    source
        .set
        .insert_registered_pending(
            registration,
            root.clone(),
            Some(root),
            catalog(),
            true,
            prepared,
            |_| Ok(()),
        )
        .unwrap();
    source
}

/// Source B: a fresh window whose only tab is pending, its Files pair on
/// terminal-2 while terminal-1 waits for the first pane.
pub(crate) fn source_b() -> Source {
    let (root, runtime, registry) = empty_root();
    let registration = registry.create(None).unwrap();
    let set = WorkspaceSet::registered_pending(
        Some(registry.clone()),
        registration,
        IdAllocator::new(runtime.clone(), 0).unwrap(),
        root.clone(),
        Some(root.clone()),
        catalog(),
        true,
        prepared,
        |_| Ok(()),
    )
    .unwrap();
    Source {
        set,
        registry,
        root,
        runtime,
    }
}

#[test]
fn source_a_has_every_shape_the_section_carries() {
    let source = source_a();
    let (core, update) = source.capture().unwrap();
    let tab = &update.tab_extras[1];
    assert_eq!(core.sessions.len(), 3);
    assert_eq!(
        core.sessions[0].tabs[0].columns[0].panes[0].run.terminal_id,
        "terminal-1"
    );
    assert_eq!(core.sessions[1].tabs[0].columns.len(), 2);
    assert_eq!(core.sessions[1].tabs[0].columns[0].panes.len(), 2);
    // Its Files pair stays on as a durable popup beside the agent.
    assert_eq!(core.sessions[1].tabs[0].popups.len(), 2);
    assert_eq!(tab.extra_popups[0].role, Role::Transient);
    assert!(tab.covered_popup.is_some());
    assert!(update.tab_extras[2].pending);
    assert!(update.session_extras[0].registration_error.is_some());
    assert!(update.allocator.range == 3 && !update.allocator.first_reserved);
    assert!(!update.recent_mutations.is_empty());
    assert_eq!(update.recent_inspections, ["inspect-0"]);
    assert_eq!(core.revision, 7);
}

#[test]
fn a_capture_restores_equal_to_its_source_with_terminal_one_at_orbit_sock() {
    for source in [source_a(), source_b()] {
        let (core, update) = source.capture().unwrap();
        let restored = source.restore(&core, &update).unwrap();
        assert_eq!(restored, source.set);
        assert_eq!(
            restored.checkpoint(7, &Source::run).unwrap(),
            (core, update)
        );
    }
    let restored = {
        let source = source_a();
        let (core, update) = source.capture().unwrap();
        source.restore(&core, &update).unwrap()
    };
    let first = &restored.workspaces[0].tabs[0].columns[0].panes[0].session;
    assert_eq!(first.id, "terminal-1");
    assert_eq!(first.endpoint, restored.ids.runtime.join("orbit.sock"));
}

#[test]
fn names_come_only_from_the_target_registry() {
    let mut source = source_a();
    source.set.workspaces[1].name = Some("stale".into());
    let (core, update) = source.capture().unwrap();
    let before = source.registry_bytes();
    let restored = source.restore(&core, &update).unwrap();
    assert_eq!(restored.workspaces[1].name.as_deref(), Some("fresh"));
    assert_eq!(source.registry_bytes(), before);
}

fn unsettled(source: &Source, what: &'static str) {
    let before = source.set.clone();
    assert_eq!(source.capture(), Err(Error::Unsettled(what)));
    assert_eq!(source.set, before);
}

#[test]
fn unsettled_topology_refuses_capture() {
    let mut source = source_a();
    source.set.reserve_registration("in-flight").unwrap();
    unsettled(&source, "a session request is in flight");

    let mut source = source_a();
    source.set.workspaces[1]
        .catalog
        .entries
        .retain(|entry| entry.id != "agent");
    unsettled(
        &source,
        "a popup's entry is not in its session's popup catalog",
    );

    let source = source_a();
    let before = source.set.clone();
    let run = |terminal: &str| {
        (terminal != "terminal-1")
            .then(|| Source::run(terminal))
            .flatten()
    };
    assert_eq!(
        source.set.checkpoint(7, &run),
        Err(Error::Unsettled("a terminal has no live run"))
    );
    assert_eq!(source.set, before);

    // Recovery's registration still being read.
    let (root, runtime, registry) = empty_root();
    let set = WorkspaceSet::registered_recovered(
        Some(registry.clone()),
        None,
        IdAllocator::new(runtime.clone(), 0).unwrap(),
        root.clone(),
        None,
        vec![(
            1,
            Session {
                id: "terminal-1".into(),
                endpoint: terminal_endpoint(&runtime, 1),
            },
        )],
        catalog(),
    )
    .unwrap();
    let source = Source {
        set,
        registry,
        root,
        runtime,
    };
    unsettled(&source, "a session's registration is still being recovered");
}

#[test]
fn a_pending_tab_without_its_pair_refuses_until_its_directory_is_chosen() {
    let mut source = source_b();
    let pair = source.set.workspaces[0].tabs[0].popups[0]
        .session
        .id
        .clone();
    source
        .set
        .session_ended(&pair, crate::workspace::SessionEnd::Closed, |_| Ok(()))
        .unwrap();
    assert!(source.set.workspaces[0].tabs[0].pending);
    unsettled(
        &source,
        "the pending tab has no Files pair; choose its directory",
    );
    assert_eq!(
        Error::Unsettled("the pending tab has no Files pair; choose its directory").to_string(),
        "cannot checkpoint: the pending tab has no Files pair; choose its directory"
    );

    let tab = source.set.workspaces[0].tabs[0].id.clone();
    let root = source.root.clone();
    source.dispatch(Action::Workspace(WorkspaceAction::SetTabDirectory {
        tab,
        directory: root.as_os_str().as_bytes().to_vec(),
    }));
    let (core, update) = source.capture().unwrap();
    assert_eq!(source.restore(&core, &update).unwrap(), source.set);
}

#[test]
fn a_dangling_cover_and_a_home_the_wire_refuses_capture_as_none() {
    let mut source = source_a();
    let tab = &mut source.set.workspaces[1].tabs[0];
    tab.covered_popup = Some("u404".into());
    for home in [&b"rel"[..], b"", b"/with\0nul", &[b'/'; 4097][..]] {
        source.set.workspaces[1].home = Some(path_buf(home.to_vec()));
        let (core, update) = source.capture().unwrap();
        assert_eq!(update.session_extras[1].home, None);
        assert_eq!(update.tab_extras[1].covered_popup, None);
        assert_eq!(core.sessions[1].tabs[0].popups.len(), 2);
    }
    // An absolute home that is missing now is kept: it may return.
    let missing = source.root.join("not-yet");
    source.set.workspaces[1].home = Some(missing.clone());
    let (_, update) = source.capture().unwrap();
    assert_eq!(
        update.session_extras[1].home.as_deref(),
        Some(missing.as_os_str().as_bytes())
    );
}

fn unrestorable(
    source: &Source,
    core: &Core,
    update: &Update,
    catalog: PopupCatalog,
    what: &'static str,
) {
    let before = source.registry_bytes();
    let result = WorkspaceSet::restore(
        core,
        update,
        source.runtime.clone(),
        Some(source.registry.clone()),
        catalog,
        false,
    );
    assert_eq!(result, Err(Error::Unrestorable(what)));
    assert_eq!(source.registry_bytes(), before);
}

#[test]
fn restore_refuses_a_registry_or_catalog_the_window_cannot_use() {
    let source = source_a();
    let (core, update) = source.capture().unwrap();

    let without = WorkspaceSet::restore(
        &core,
        &update,
        source.runtime.clone(),
        None,
        catalog(),
        false,
    );
    assert_eq!(
        without,
        Err(Error::Unrestorable(
            "the session registry availability differs"
        ))
    );
    let mut unavailable = update.clone();
    unavailable.registry_available = false;
    unrestorable(
        &source,
        &core,
        &unavailable,
        catalog(),
        "the session registry availability differs",
    );

    let mut missing = core.clone();
    // A registration only another registry holds.
    let foreign = Registry::in_test_directory(temporary_directory().join("sessions"));
    missing.sessions[1].persistent_id = Some(foreign.create(None).unwrap().id);
    unrestorable(
        &source,
        &missing,
        &update,
        catalog(),
        "a session's registration is missing",
    );

    // The core's durable popup, the transient and the pending pair each need
    // their entry, kept alive as their placement says.
    for (entry, keep_alive) in [("agent", false), ("commands", true), ("files", false)] {
        let mut gone = catalog();
        gone.entries.retain(|definition| definition.id != entry);
        unrestorable(
            &source,
            &core,
            &update,
            gone,
            "a popup's entry is not in the target popup catalog",
        );
        let mut moved = catalog();
        for definition in &mut moved.entries {
            if definition.id == entry {
                definition.keep_alive = keep_alive;
            }
        }
        unrestorable(
            &source,
            &core,
            &update,
            moved,
            "a popup's keep_alive differs in the target popup catalog",
        );
    }
}

#[test]
fn a_pending_pair_needs_a_kept_files_entry_in_the_target() {
    // Source B holds only the pair, so no other popup's rule refuses first:
    // after its pick the pair stays on as the tab's hidden Files popup.
    let source = source_b();
    let (core, update) = source.capture().unwrap();
    let mut transient = catalog();
    for definition in &mut transient.entries {
        if definition.id == "files" {
            definition.keep_alive = false;
        }
    }
    unrestorable(
        &source,
        &core,
        &update,
        transient,
        "a popup's keep_alive differs in the target popup catalog",
    );
}

#[test]
fn restore_refuses_an_unreadable_registry_without_writing_it() {
    use std::os::unix::fs::PermissionsExt;
    let source = source_a();
    let (core, update) = source.capture().unwrap();
    let directory = source.root.join("sessions");
    let mode = fs::metadata(&directory).unwrap().permissions().mode();
    fs::set_permissions(&directory, fs::Permissions::from_mode(mode | 0o022)).unwrap();
    unrestorable(
        &source,
        &core,
        &update,
        catalog(),
        "the session registry cannot be read",
    );
    fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
}
