use super::super::{Column, Pane, Session, Tab};
use super::*;
use orbit_protocol::management::{ProgramKind, ProgramState};

/// The first local number of identity range 3.
const F: u64 = 3 * RANGE_IDS + 1;

pub(crate) fn header() -> Header {
    Header {
        namespace: "/r".into(),
        generation: format!("g1-{}", "1".repeat(32)),
        created_build: format!("g1-{}", "2".repeat(32)),
        active_build: format!("g1-{}", "3".repeat(32)),
    }
}

fn scope() -> Scope {
    Scope {
        header: header(),
        range: 3,
    }
}

fn id(prefix: &str, n: u64) -> String {
    format!("{prefix}{}", F + n)
}

fn run(terminal: u64) -> Run {
    Run {
        kind: RunKind::Confirmed,
        terminal_id: format!("terminal-{terminal}"),
        run_id: format!("run-{terminal}"),
    }
}

fn pane(n: u64, terminal: u64) -> Pane {
    Pane {
        id: id("p", n),
        pinned_name: None,
        tag: None,
        run: run(terminal),
    }
}

fn popup(n: u64, entry: &str, label: &str, terminal: u64) -> Popup {
    Popup {
        id: id("u", n),
        entry: entry.into(),
        label: label.into(),
        launch_directory: b"/work/\xff".to_vec(),
        run: run(terminal),
    }
}

pub(crate) fn record(created: u64, state: ProgramState) -> ProgramRecord {
    ProgramRecord {
        id: if created == 1 {
            String::new()
        } else {
            format!("task-{created}")
        },
        state,
        kind: (state == ProgramState::Blocked).then_some(ProgramKind::Question),
        progress: Some(40),
        app: "claude".into(),
        title: "Review λ".into(),
        message: "needs an answer".into(),
        updated: created + 5,
        created,
        changed: created + 3,
        previous: Some(ProgramState::Working),
        updated_age_ms: 12,
        changed_age_ms: 900,
    }
}

/// Three sessions: a committed tab with hidden panes, a durable popup covered
/// by a shown transient, a pending tab with its Files pair, a registry
/// failure, a closed-terminal hole, rings, recency and every attention form.
fn fixture() -> (Core, Update) {
    let core = Core {
        revision: 9,
        shown_session: Some(id("s", 0)),
        terminal_id_high_water: 11,
        sessions: vec![
            Session {
                id: id("s", 0),
                persistent_id: Some(format!("{:048x}", 1)),
                active_tab: Some(id("t", 0)),
                tabs: vec![
                    Tab {
                        id: id("t", 0),
                        directory: b"/work".to_vec(),
                        columns: vec![
                            Column {
                                panes: vec![
                                    Pane {
                                        pinned_name: Some("Build λ".into()),
                                        tag: Some("ci".into()),
                                        ..pane(0, 1)
                                    },
                                    pane(1, 2),
                                ],
                                expanded: id("p", 0),
                            },
                            Column {
                                panes: vec![pane(2, 3)],
                                expanded: id("p", 2),
                            },
                        ],
                        popups: vec![
                            popup(0, "files", "Files", 4),
                            popup(4, "notes", "Notes", 11),
                        ],
                        selected_pane: Some(id("p", 0)),
                        selected_popup: None,
                    },
                    Tab {
                        id: id("t", 1),
                        directory: b"/".to_vec(),
                        columns: vec![],
                        popups: vec![],
                        selected_pane: None,
                        selected_popup: None,
                    },
                ],
            },
            Session {
                id: id("s", 1),
                persistent_id: None,
                active_tab: Some(id("t", 2)),
                tabs: vec![Tab {
                    id: id("t", 2),
                    directory: b"/srv".to_vec(),
                    columns: vec![Column {
                        panes: vec![pane(3, 8)],
                        expanded: id("p", 3),
                    }],
                    popups: vec![popup(3, "agent", "Agent", 9)],
                    selected_pane: Some(id("p", 3)),
                    selected_popup: Some(id("u", 3)),
                }],
            },
            Session {
                id: id("s", 2),
                persistent_id: Some(format!("{:048x}", 3)),
                active_tab: Some(id("t", 3)),
                tabs: vec![Tab {
                    id: id("t", 3),
                    directory: b"/home".to_vec(),
                    columns: vec![Column {
                        panes: vec![pane(4, 10)],
                        expanded: id("p", 4),
                    }],
                    popups: vec![],
                    selected_pane: Some(id("p", 4)),
                    selected_popup: None,
                }],
            },
        ],
    };
    let update = Update {
        captured_monotonic_ns: 77_000_000_000,
        allocator: Allocator {
            range: 3,
            next_session: F + 3,
            next_tab: F + 4,
            next_pane: F + 5,
            next_terminal: 12,
            next_popup: F + 5,
            first_reserved: false,
        },
        registry_available: true,
        session_extras: vec![
            SessionExtra {
                session: id("s", 0),
                home: Some(b"/home/u".to_vec()),
                registration_error: None,
            },
            SessionExtra {
                session: id("s", 1),
                home: None,
                registration_error: Some(RegistrationError {
                    code: "session-registry".into(),
                    detail: "recovered registration is missing".into(),
                }),
            },
            SessionExtra {
                session: id("s", 2),
                home: Some(b"/home/\xff".to_vec()),
                registration_error: None,
            },
        ],
        tab_extras: vec![
            TabExtra {
                tab: id("t", 0),
                pending: false,
                popup_order: vec![id("u", 0), id("u", 1), id("u", 4)],
                extra_popups: vec![ExtraPopup {
                    role: Role::Transient,
                    popup: popup(1, "git", "Git", 5),
                }],
                selected_popup: Some(id("u", 1)),
                covered_popup: Some(id("u", 0)),
            },
            TabExtra {
                tab: id("t", 1),
                pending: true,
                popup_order: vec![id("u", 2)],
                extra_popups: vec![ExtraPopup {
                    role: Role::PendingPair,
                    popup: popup(2, "files", "Files", 6),
                }],
                selected_popup: Some(id("u", 2)),
                covered_popup: None,
            },
            TabExtra {
                tab: id("t", 2),
                pending: false,
                popup_order: vec![id("u", 3)],
                extra_popups: vec![],
                selected_popup: Some(id("u", 3)),
                covered_popup: None,
            },
            TabExtra {
                tab: id("t", 3),
                pending: false,
                popup_order: vec![],
                extra_popups: vec![],
                selected_popup: None,
                covered_popup: None,
            },
        ],
        recent_inspections: vec!["inspect-1".into(), "inspect-2".into()],
        recent_mutations: vec!["mutate-1".into()],
        recent_sessions: vec![id("s", 0), id("s", 2), id("s", 1)],
        terminal_extras: [1, 2, 3, 4, 5, 11, 6, 8, 9, 10]
            .into_iter()
            .map(|terminal| TerminalExtra {
                terminal: format!("terminal-{terminal}"),
                default_shell: terminal == 1,
                named: (terminal == 1).then_some(Named {
                    process_id: 4242,
                    process_start: 991_133,
                }),
            })
            .collect(),
        attention: Some(Attention {
            terminals: vec![
                AttentionTerminal {
                    terminal: "terminal-1".into(),
                    run_id: "run-1".into(),
                    update: 40,
                    count: 4,
                    background: 1,
                    foreground: Foreground::Child,
                    name: "vim λ".into(),
                    records: vec![
                        AttentionRecord {
                            record: record(1, ProgramState::Done),
                            changed_age_ns: 3_000_000_000,
                            raise: Some(Raise {
                                age_ns: 2_000_000_000,
                                changed: 4,
                                seen: false,
                                notified: true,
                            }),
                            blocked_notified: false,
                        },
                        AttentionRecord {
                            record: record(7, ProgramState::Blocked),
                            changed_age_ns: 0,
                            raise: None,
                            blocked_notified: true,
                        },
                    ],
                    omitted: vec![AttentionRecord {
                        record: record(9, ProgramState::Error),
                        changed_age_ns: MAX_STAMP_NS,
                        raise: Some(Raise {
                            age_ns: 0,
                            changed: 12,
                            seen: true,
                            notified: false,
                        }),
                        blocked_notified: false,
                    }],
                    replied_age_ns: 500_000_000,
                },
                AttentionTerminal {
                    terminal: "terminal-9".into(),
                    run_id: "run-9".into(),
                    update: 3,
                    count: 0,
                    background: 0,
                    foreground: Foreground::Other,
                    name: String::new(),
                    records: vec![],
                    omitted: vec![],
                    replied_age_ns: 0,
                },
            ],
            recovered: vec![
                Recovered {
                    run_id: "run-3".into(),
                    baseline: Some(7),
                },
                Recovered {
                    run_id: "run-8".into(),
                    baseline: None,
                },
            ],
            next_poll_due_ns: 200_000_000,
            sidebar: Sidebar::Closed,
            epoch_age_ns: 9_000_000_000,
            epoch_wall_ms: 1_763_600_000_000,
        }),
        supervisor: Supervisor {
            started_at_ms: 1_763_590_000_000,
            // Empty and non-UTF-8 arguments are commands too.
            initial_command: vec![b"sh".to_vec(), b"-c".to_vec(), Vec::new(), vec![0xff, b'x']],
            initial_status: Some(130),
        },
    };
    (core, update)
}

fn checkpoint(core: Core, payload: Vec<u8>) -> Vec<u8> {
    let checkpoint = Checkpoint {
        header: header(),
        core,
        sections: vec![Section {
            name: SECTION.into(),
            required: true,
            payload,
        }],
    };
    let mut out = Encoder(Vec::new());
    checkpoint.write(&mut out).unwrap();
    out.0
}

/// The fixture's update payload, written without validation.
fn payload() -> Vec<u8> {
    fixture().1.write().unwrap()
}

/// Encode refuses the mutated model with `expected`, and decode refuses the
/// same model written without validation.
#[track_caller]
fn red(expected: Error, mutate: impl FnOnce(&mut Core, &mut Update)) {
    red_as(expected, None, mutate);
}

/// As `red`, for a value the decoder's length bound refuses before the model
/// validator sees it.
#[track_caller]
fn red_bound(expected: Error, mutate: impl FnOnce(&mut Core, &mut Update)) {
    red_as(expected, Some(invalid("string size")), mutate);
}

#[track_caller]
fn red_as(expected: Error, decoded: Option<Error>, mutate: impl FnOnce(&mut Core, &mut Update)) {
    let (mut core, mut update) = fixture();
    mutate(&mut core, &mut update);
    assert_eq!(
        encode(header(), core.clone(), &update, vec![]),
        Err(expected.clone()),
        "encode"
    );
    // A model the writer cannot even express is refused on both paths.
    if let Ok(payload) = update.write() {
        assert_eq!(
            decode(&checkpoint(core, payload), &scope()).map(|_| ()),
            Err(decoded.unwrap_or(expected)),
            "decode"
        );
    }
}

#[track_caller]
fn green(mutate: impl FnOnce(&mut Core, &mut Update)) {
    let (mut core, mut update) = fixture();
    mutate(&mut core, &mut update);
    let bytes = encode(header(), core.clone(), &update, vec![]).unwrap();
    let (decoded, restored) = decode(&bytes, &scope()).unwrap();
    assert_eq!((decoded.core, restored), (core, update));
}

#[track_caller]
fn red_payload(expected: Error, payload: Vec<u8>) {
    let bytes = checkpoint(fixture().0, payload);
    assert_eq!(decode(&bytes, &scope()).map(|_| ()), Err(expected));
}

fn invalid(field: &'static str) -> Error {
    Error::Invalid(field)
}

/// Replaces the first occurrence of `from` in `bytes` with `to`.
fn splice(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .position(|window| window == from)
        .expect("pattern in payload");
    [&bytes[..at], to, &bytes[at + from.len()..]].concat()
}

fn blob(frame: &[u8]) -> Vec<u8> {
    [&(frame.len() as u32).to_le_bytes()[..], frame].concat()
}

fn attention(update: &mut Update) -> &mut Attention {
    update.attention.as_mut().unwrap()
}

#[test]
fn update_schema_1_fixture_decodes_and_encodes_exactly() {
    // Committed bytes, never rewritten by a test: an older build's checkpoint.
    let bytes = include_bytes!("update-schema-1.eonr");
    let (core, update) = fixture();
    let (checkpoint, decoded) = decode(bytes, &scope()).unwrap();
    assert_eq!((checkpoint.core.clone(), decoded.clone()), (core, update));
    assert_eq!(
        encode(header(), checkpoint.core, &decoded, vec![]).unwrap(),
        bytes
    );
}

#[test]
fn update_section_framing_refuses_missing_optional_trailing_and_bad_tags() {
    let (core, update) = fixture();
    let without = Checkpoint {
        header: header(),
        core: core.clone(),
        sections: vec![],
    };
    assert_eq!(
        decode(&without.encode().unwrap(), &scope()).map(|_| ()),
        Err(invalid("update section"))
    );
    let optional = Checkpoint {
        header: header(),
        core: core.clone(),
        sections: vec![Section {
            name: SECTION.into(),
            required: false,
            payload: update.write().unwrap(),
        }],
    };
    assert_eq!(
        decode(&optional.encode().unwrap(), &scope()).map(|_| ()),
        Err(invalid("update section"))
    );
    let mut trailing = payload();
    trailing.push(0);
    red_payload(invalid("trailing bytes"), trailing);
    // first_reserved, then registry_available, sit after schema, anchor and
    // six allocator counters.
    for at in [58, 59] {
        let mut tag = payload();
        tag[at] = 2;
        red_payload(invalid("presence/boolean tag"), tag);
    }
    // The sidebar tag sits 17 bytes before the attention part's end, which
    // the 26-byte supervisor record follows.
    let sidebar = payload().len() - 17 - 26;
    let mut tag = payload();
    tag[sidebar] = 3;
    red_payload(invalid("sidebar"), tag);
    // terminal-1's attention entry: id, run, update, count, background, then
    // its foreground tag.
    let entry = b"\x0a\0terminal-1\x05\0run-1";
    let at = payload()
        .windows(entry.len())
        .position(|window| window == entry)
        .unwrap()
        + entry.len()
        + 11;
    let mut tag = payload();
    assert_eq!(tag[at], 1);
    tag[at] = 3;
    red_payload(invalid("foreground"), tag);
    // The transient's record: its role byte, then its id and entry.
    let role = [b"\x08\0".as_slice(), id("u", 1).as_bytes(), b"\x03\0git"].concat();
    let mut bad = payload();
    let at = bad.windows(role.len()).position(|w| w == role).unwrap() - 1;
    assert_eq!(bad[at], 0);
    bad[at] = 2;
    red_payload(invalid("popup role"), bad);
}

#[test]
fn update_schema_window_admits_only_floor_through_current_with_a_reason() {
    assert_eq!((SCHEMA_FLOOR, SCHEMA), (1, 1));
    for found in [0, 2, u16::MAX] {
        let mut bytes = payload();
        bytes[..2].copy_from_slice(&found.to_le_bytes());
        let error = Error::UnsupportedUpdateSchema {
            found,
            floor: 1,
            current: 1,
        };
        assert!(error.to_string().contains(&format!(
            "schema {found}: this build accepts schemas 1 through 1"
        )));
        assert!(error.to_string().contains("by hand"));
        red_payload(error, bytes);
    }
}

#[test]
fn update_optional_sections_survive_and_required_ones_are_refused() {
    let (core, update) = fixture();
    let later = Section {
        name: "later".into(),
        required: false,
        payload: b"\x00\xffopaque".to_vec(),
    };
    let bytes = encode(header(), core.clone(), &update, vec![later.clone()]).unwrap();
    let (checkpoint, _) = decode(&bytes, &scope()).unwrap();
    assert_eq!(checkpoint.sections[1], later);
    for section in [
        Section {
            required: true,
            ..later.clone()
        },
        Section {
            name: SECTION.into(),
            ..later
        },
    ] {
        assert_eq!(
            encode(header(), core.clone(), &update, vec![section]),
            Err(invalid("optional sections"))
        );
    }
}

#[test]
fn update_scope_refuses_a_different_identity_range() {
    let (core, update) = fixture();
    let bytes = encode(header(), core, &update, vec![]).unwrap();
    for range in [0, 2, 4] {
        assert_eq!(
            decode(&bytes, &Scope { range, ..scope() }).map(|_| ()),
            Err(Error::ScopeMismatch("identity range"))
        );
    }
    let mut other = scope();
    other.header.active_build = format!("g1-{}", "4".repeat(32));
    assert_eq!(
        decode(&bytes, &other).map(|_| ()),
        Err(Error::ScopeMismatch("active_build"))
    );
}

#[test]
fn update_extras_cover_the_core_and_required_presence() {
    red(invalid("session extras"), |_, u| {
        u.session_extras.pop();
    });
    red(invalid("session extras"), |_, u| {
        u.session_extras[1].session = id("s", 2);
    });
    red(invalid("tab extras"), |_, u| {
        u.tab_extras.pop();
    });
    red(invalid("tab extras"), |_, u| u.tab_extras.swap(2, 3));
    red(invalid("shown session"), |c, _| c.shown_session = None);
    red(invalid("shown session"), |c, u| {
        c.sessions.clear();
        c.shown_session = None;
        u.session_extras.clear();
    });
    red(invalid("active tab"), |c, _| {
        c.sessions[2].active_tab = None
    });
    red(invalid("selected pane"), |c, _| {
        c.sessions[2].tabs[0].selected_pane = None
    });
    // Core catches a pane selection in a tab without columns.
    red(invalid("selected pane"), |c, _| {
        c.sessions[0].tabs[1].selected_pane = Some(id("p", 0))
    });
    red(invalid("home"), |_, u| {
        u.session_extras[0].home = Some(b"relative".to_vec())
    });
    red(invalid("registration error code"), |_, u| {
        u.session_extras[1]
            .registration_error
            .as_mut()
            .unwrap()
            .code = String::new()
    });
    red_bound(invalid("registration error detail"), |_, u| {
        u.session_extras[1]
            .registration_error
            .as_mut()
            .unwrap()
            .detail = "d".repeat(1025)
    });
    red(invalid("registration error"), |_, u| {
        u.session_extras[0].registration_error = u.session_extras[1].registration_error.clone()
    });
    red(invalid("registry availability"), |_, u| {
        u.registry_available = false
    });
    // The owner keeps a session's last tab with no terminal.
    green(|c, u| {
        let tab = &mut c.sessions[2].tabs[0];
        tab.columns.clear();
        tab.selected_pane = None;
        u.terminal_extras
            .retain(|extra| extra.terminal != "terminal-10");
    });
    green(|c, u| {
        c.sessions[1].persistent_id = None;
        c.sessions[0].persistent_id = None;
        c.sessions[2].persistent_id = None;
        u.registry_available = false;
    });
}

fn transient(update: &mut Update) -> &mut Popup {
    &mut update.tab_extras[0].extra_popups[0].popup
}

#[test]
fn update_extension_popups_obey_every_shared_record_rule() {
    red(invalid("popup id"), |_, u| {
        transient(u).id = "x5".into();
        u.tab_extras[0].popup_order[1] = "x5".into();
    });
    red(invalid("duplicate local id"), |_, u| {
        transient(u).id = id("u", 0)
    });
    red(invalid("popup id"), |_, u| {
        transient(u).id = id("u", 5);
        u.tab_extras[0].popup_order[1] = id("u", 5);
    });
    red(invalid("terminal id"), |_, u| {
        transient(u).run.terminal_id = "term-5".into()
    });
    red(invalid("terminal high-water"), |_, u| {
        transient(u).run.terminal_id = "terminal-12".into()
    });
    red(invalid("run id"), |_, u| {
        transient(u).run.run_id = String::new()
    });
    red_bound(invalid("run id"), |_, u| {
        transient(u).run.run_id = "r".repeat(129)
    });
    red(invalid("popup entry"), |_, u| {
        transient(u).entry = "g it".into()
    });
    red(invalid("popup label"), |_, u| {
        transient(u).label = "G\nit".into()
    });
    red_bound(invalid("popup label"), |_, u| {
        transient(u).label = "g".repeat(129)
    });
    red(invalid("popup directory"), |_, u| {
        transient(u).launch_directory = b"work".to_vec()
    });
    red(invalid("duplicate popup entry"), |_, u| {
        transient(u).entry = "notes".into()
    });
    red(invalid("duplicate terminal/run ownership"), |_, u| {
        transient(u).run.terminal_id = "terminal-4".into()
    });
    red(invalid("duplicate run id"), |_, u| {
        transient(u).run.run_id = "run-10".into()
    });
    red(invalid("start intent"), |c, _| {
        c.sessions[0].tabs[0].columns[0].panes[1].run.kind = RunKind::StartIntent
    });
    red(invalid("start intent"), |c, _| {
        c.sessions[1].tabs[0].popups[0].run.kind = RunKind::StartIntent
    });
    red(invalid("start intent"), |_, u| {
        transient(u).run.kind = RunKind::StartIntent
    });
    red(invalid("popups"), |_, u| {
        let extra = u.tab_extras[0].extra_popups[0].clone();
        u.tab_extras[0].extra_popups.extend(vec![extra; 30]);
    });
}

#[test]
fn update_slot_limit_counts_core_and_extension_together() {
    // 255 core slots plus the transient and the pair are one slot too many.
    let fill = |c: &mut Core, u: &mut Update, panes: u64| {
        let column = &mut c.sessions[2].tabs[0].columns[0].panes;
        for n in 0..panes {
            column.push(pane(5 + n, 12 + n));
        }
        c.terminal_id_high_water = 11 + panes;
        u.allocator.next_terminal = 12 + panes;
        u.allocator.next_pane = F + 5 + panes;
        u.terminal_extras.extend((0..panes).map(|n| TerminalExtra {
            terminal: format!("terminal-{}", 12 + n),
            default_shell: false,
            named: None,
        }));
    };
    // Decode meets the 256-entry terminal-extras list bound first.
    red_as(
        invalid("terminal slots"),
        Some(invalid("terminal extras")),
        |c, u| fill(c, u, 247),
    );
    green(|c, u| fill(c, u, 246));
}

#[test]
fn update_pending_tabs_and_popup_order() {
    red(invalid("pending pair"), |_, u| {
        u.tab_extras[0].extra_popups[0].role = Role::PendingPair
    });
    red(invalid("pending tab"), |_, u| {
        u.tab_extras[1].extra_popups[0].popup.entry = "git".into()
    });
    red(invalid("pending tab"), |_, u| {
        u.tab_extras[1].extra_popups[0].role = Role::Transient
    });
    red(invalid("pending tab"), |c, _| {
        // Move s2's only pane into the pending tab.
        let moved = c.sessions[2].tabs[0].columns.remove(0);
        c.sessions[2].tabs[0].selected_pane = None;
        let tab = &mut c.sessions[0].tabs[1];
        tab.selected_pane = Some(moved.expanded.clone());
        tab.columns.push(moved);
    });
    red(invalid("pending tab"), |_, u| {
        // A tool beside the pair.
        u.allocator.next_popup = F + 6;
        let pending = &mut u.tab_extras[1];
        pending.extra_popups.push(ExtraPopup {
            role: Role::Transient,
            popup: popup(5, "git", "Git", 7),
        });
        pending.popup_order.push(id("u", 5));
    });
    red(invalid("pending tab"), |c, u| {
        // A core popup in a pending tab.
        let moved = c.sessions[1].tabs[0].popups.remove(0);
        c.sessions[1].tabs[0].selected_popup = None;
        u.tab_extras[2].selected_popup = None;
        u.tab_extras[2].popup_order.clear();
        c.sessions[0].tabs[1].popups.push(moved);
    });
    red(invalid("pending tab selection"), |_, u| {
        u.tab_extras[1].selected_popup = None
    });
    red(invalid("popup order"), |_, u| {
        u.tab_extras[0].popup_order.pop();
    });
    red(invalid("popup order"), |_, u| {
        u.tab_extras[0].popup_order.push(id("u", 1));
    });
    red(invalid("popup order"), |_, u| {
        // Durable popups out of their core order.
        u.tab_extras[0].popup_order = vec![id("u", 4), id("u", 1), id("u", 0)];
    });
    green(|_, u| {
        // Terminal extras follow the slots, so they move with the popups.
        u.tab_extras[0].popup_order = vec![id("u", 1), id("u", 0), id("u", 4)];
        u.terminal_extras.swap(3, 4);
    });
}

#[test]
fn update_full_selection_and_coverage() {
    red(invalid("selected popup"), |_, u| {
        u.tab_extras[0].selected_popup = Some(id("u", 3))
    });
    red(invalid("selected popup"), |c, _| {
        c.sessions[0].tabs[0].selected_popup = Some(id("u", 0))
    });
    red(invalid("selected popup"), |c, _| {
        c.sessions[1].tabs[0].selected_popup = None
    });
    red(invalid("covered popup"), |_, u| {
        u.tab_extras[0].covered_popup = Some(id("u", 1))
    });
    red(invalid("covered popup"), |_, u| {
        u.tab_extras[0].covered_popup = Some(id("u", 9))
    });
    red(invalid("covered popup"), |_, u| {
        u.tab_extras[0].covered_popup = Some(id("u", 3))
    });
    red(invalid("covered popup"), |c, u| {
        drop_transient(u);
        c.sessions[0].tabs[0].selected_popup = Some(id("u", 4));
        u.tab_extras[0].selected_popup = Some(id("u", 4));
        u.tab_extras[0].covered_popup = Some(id("u", 0));
    });
    green(|c, u| {
        drop_transient(u);
        c.sessions[0].tabs[0].selected_popup = Some(id("u", 4));
        u.tab_extras[0].selected_popup = Some(id("u", 4));
        u.tab_extras[0].covered_popup = None;
    });
}

/// Removes t0's shown transient, which ends the cover beneath it.
fn drop_transient(update: &mut Update) {
    let tab = &mut update.tab_extras[0];
    tab.extra_popups.clear();
    tab.popup_order.retain(|popup| *popup != id("u", 1));
    tab.selected_popup = None;
    tab.covered_popup = None;
    update
        .terminal_extras
        .retain(|extra| extra.terminal != "terminal-5");
}

#[test]
fn update_transients_exist_only_while_shown() {
    red(invalid("hidden transient"), |_, u| {
        u.tab_extras[0].selected_popup = None;
        u.tab_extras[0].covered_popup = None;
    });
    red(invalid("hidden transient"), |c, u| {
        c.sessions[0].tabs[0].selected_popup = Some(id("u", 0));
        u.tab_extras[0].selected_popup = Some(id("u", 0));
        u.tab_extras[0].covered_popup = None;
    });
    red(invalid("hidden transient"), |c, u| {
        // A second transient beside the shown one, with its terminal extra,
        // so only the shown-transient rule refuses it.
        u.allocator.next_popup = F + 6;
        let tab = &mut u.tab_extras[0];
        tab.extra_popups.push(ExtraPopup {
            role: Role::Transient,
            popup: popup(5, "lazygit", "LazyGit", 7),
        });
        tab.popup_order.push(id("u", 5));
        let at = u
            .slots(c)
            .unwrap()
            .iter()
            .position(|slot| slot.run.terminal_id == "terminal-7")
            .unwrap();
        u.terminal_extras.insert(
            at,
            TerminalExtra {
                terminal: "terminal-7".into(),
                default_shell: false,
                named: None,
            },
        );
    });
}

/// One session whose only tab is empty: no slot, no terminal ever started.
fn empty_window() -> (Core, Update) {
    let (mut core, mut update) = fixture();
    core.sessions.truncate(1);
    let session = &mut core.sessions[0];
    session.tabs.truncate(1);
    session.persistent_id = None;
    let tab = &mut session.tabs[0];
    tab.columns.clear();
    tab.popups.clear();
    tab.selected_pane = None;
    core.terminal_id_high_water = 0;
    update.allocator.next_terminal = 1;
    update.session_extras.truncate(1);
    update.tab_extras = vec![TabExtra {
        tab: id("t", 0),
        pending: false,
        popup_order: vec![],
        extra_popups: vec![],
        selected_popup: None,
        covered_popup: None,
    }];
    update.recent_sessions = vec![id("s", 0)];
    update.terminal_extras.clear();
    update.attention = None;
    (core, update)
}

#[test]
fn update_reserved_terminal_needs_a_next_terminal_beyond_it() {
    let (core, update) = empty_window();
    let bytes = encode(header(), core.clone(), &update, vec![]).unwrap();
    assert_eq!(decode(&bytes, &scope()).unwrap().1, update);
    let mut reserved = update.clone();
    reserved.allocator.first_reserved = true;
    assert_eq!(
        encode(header(), core.clone(), &reserved, vec![]),
        Err(invalid("reserved terminal"))
    );
    assert_eq!(
        decode(
            &checkpoint(core.clone(), reserved.write().unwrap()),
            &scope()
        )
        .map(|_| ()),
        Err(invalid("reserved terminal"))
    );
    // The owner's real reservation: terminal-2 taken, next is 3.
    reserved.allocator.next_terminal = 3;
    let mut core = core;
    core.terminal_id_high_water = 2;
    encode(header(), core, &reserved, vec![]).unwrap();
}

#[test]
fn update_terminal_extras_cover_each_slot_in_window_order() {
    red(invalid("terminal extras"), |_, u| {
        u.terminal_extras.pop();
    });
    red(invalid("terminal extras"), |_, u| {
        u.terminal_extras.swap(0, 1)
    });
    red(invalid("terminal extras"), |_, u| {
        u.terminal_extras[9].terminal = "terminal-7".into()
    });
    red(invalid("attach name"), |_, u| {
        u.terminal_extras[0].named.as_mut().unwrap().process_id = 0
    });
    red(invalid("attach name"), |_, u| {
        u.terminal_extras[0].named.as_mut().unwrap().process_start = 0
    });
}

#[test]
fn update_allocator_range_counters_and_high_water() {
    red(invalid("identity range"), |_, u| {
        u.allocator.range = u64::MAX
    });
    red(invalid("allocator counter"), |_, u| {
        u.allocator.next_tab = F - 1
    });
    red(invalid("allocator counter"), |_, u| {
        u.allocator.next_popup = F + RANGE_IDS + 1
    });
    red(invalid("pane id"), |_, u| u.allocator.next_pane = F + 4);
    red(invalid("session id"), |_, u| {
        u.allocator.next_session = F + 2
    });
    red(invalid("tab id"), |_, u| u.allocator.next_tab = F + 3);
    red(invalid("popup id"), |_, u| u.allocator.next_popup = F + 4);
    red(invalid("terminal counter"), |_, u| {
        u.allocator.next_terminal = 13
    });
    red(invalid("terminal counter"), |c, u| {
        c.terminal_id_high_water = u64::MAX;
        u.allocator.next_terminal = 0;
    });
    red(invalid("reserved terminal"), |_, u| {
        u.allocator.first_reserved = true
    });
    // An ID a crash recovery kept from an earlier partition.
    green(|c, _| {
        let tab = &mut c.sessions[2].tabs[0];
        tab.columns[0].panes[0].id = "p7".into();
        tab.columns[0].expanded = "p7".into();
        tab.selected_pane = Some("p7".into());
    });
    // Exhausted counters and the reserved terminal-1, held by no slot.
    green(|c, u| {
        u.allocator.next_popup = F + RANGE_IDS;
        u.allocator.next_pane = F + RANGE_IDS;
        u.allocator.first_reserved = true;
        let tab = &mut c.sessions[0].tabs[0];
        tab.columns[0].panes.remove(0);
        tab.columns[0].expanded = id("p", 1);
        tab.selected_pane = Some(id("p", 1));
        u.terminal_extras.remove(0);
        attention(u).terminals.remove(0);
    });
}

#[test]
fn update_recency_and_replay_rings() {
    red(invalid("recent sessions"), |_, u| {
        u.recent_sessions.pop();
    });
    red(invalid("recent sessions"), |_, u| {
        u.recent_sessions.swap(0, 1)
    });
    red(invalid("recent sessions"), |_, u| {
        u.recent_sessions[2] = id("s", 0)
    });
    red(invalid("recent requests"), |_, u| {
        u.recent_mutations = (0..257).map(|n| format!("m{n}")).collect()
    });
    red(invalid("request id"), |_, u| {
        u.recent_mutations[0] = String::new()
    });
    red_bound(invalid("request id"), |_, u| {
        u.recent_mutations[0] = "m".repeat(129)
    });
    red(invalid("duplicate request id"), |_, u| {
        u.recent_mutations[0] = "inspect-1".into()
    });
    red(invalid("duplicate request id"), |_, u| {
        u.recent_inspections[1] = "inspect-1".into()
    });
    green(|_, u| u.recent_mutations = (0..256).map(|n| format!("m{n}")).collect());
}

#[test]
fn update_attention_references_and_counts() {
    red(invalid("attention terminal"), |_, u| {
        attention(u).terminals[0].run_id = "run-old".into()
    });
    red(invalid("attention terminal"), |_, u| {
        let terminal = &mut attention(u).terminals[1];
        terminal.terminal = "terminal-6".into();
        terminal.run_id = "run-6".into();
    });
    red(invalid("attention terminal"), |_, u| {
        let copy = attention(u).terminals[1].clone();
        attention(u).terminals.push(copy);
    });
    red(invalid("recovered run"), |_, u| {
        attention(u).recovered[0].run_id = "run-7".into()
    });
    red(invalid("recovered run"), |_, u| {
        let copy = attention(u).recovered[0].clone();
        attention(u).recovered.push(copy);
    });
    red(invalid("attention record counts"), |_, u| {
        attention(u).terminals[0].count = 257
    });
    red(invalid("attention record counts"), |_, u| {
        attention(u).terminals[0].count = 2
    });
    red(invalid("attention record counts"), |_, u| {
        attention(u).terminals[1].background = 1
    });
    red_bound(invalid("terminal title"), |_, u| {
        attention(u).terminals[1].name = "t".repeat(65)
    });
    red(invalid("duplicate attention record"), |_, u| {
        attention(u).terminals[0].omitted[0].record.created = 7
    });
    red(invalid("attention record"), |_, u| {
        attention(u).terminals[0].records[0].record.progress = Some(101)
    });
    green(|_, u| attention(u).terminals[1].name = "t".repeat(64));
    green(|_, u| u.attention = None);
}

#[test]
fn update_attention_record_frames_are_canonical() {
    let canonical = record_frame(&record(7, ProgramState::Blocked)).unwrap();
    let variant = |change: fn(&mut ProgramStatus)| {
        let mut status = ProgramStatus {
            count: 1,
            background: 0,
            processes: ProgramProcesses {
                background: Vec::new(),
                complete: false,
                foreground: None,
            },
            foreground: Foreground::Unknown,
            update: 0,
            terminal_title: String::new(),
            omitted: Vec::new(),
            records: vec![record(7, ProgramState::Blocked)],
        };
        change(&mut status);
        management::encode_server_message(&ServerMessage::ProgramStatus(status)).unwrap()
    };
    let variants = [
        variant(|status| {
            status.count = 2;
            status.omitted = vec![99];
        }),
        variant(|status| status.background = 1),
        variant(|status| status.processes.complete = true),
        variant(|status| status.foreground = Foreground::Child),
        variant(|status| status.update = 1),
        variant(|status| status.terminal_title = "x".into()),
    ];
    for frame in variants {
        assert_ne!(frame, canonical);
        red_payload(
            invalid("attention record frame"),
            splice(&payload(), &blob(&canonical), &blob(&frame)),
        );
    }
    let busy = management::encode_server_message(&ServerMessage::Busy).unwrap();
    red_payload(
        invalid("attention record"),
        splice(&payload(), &blob(&canonical), &blob(&busy)),
    );
    let two = variant(|status| {
        status.count = 2;
        status.records.push(record(8, ProgramState::Idle));
    });
    red_payload(
        invalid("attention record"),
        splice(&payload(), &blob(&canonical), &blob(&two)),
    );
    let trailing = [canonical.as_slice(), b"\0"].concat();
    let bytes = checkpoint(
        fixture().0,
        splice(&payload(), &blob(&canonical), &blob(&trailing)),
    );
    assert!(decode(&bytes, &scope()).is_err());
    red_payload(
        invalid("attention record size"),
        splice(
            &payload(),
            &blob(&canonical),
            &blob(&vec![0; MAX_MESSAGE_BYTES + 1]),
        ),
    );
}

#[test]
fn update_stamps_are_bounded_on_every_field() {
    let fields: [fn(&mut Attention); 5] = [
        |a| a.terminals[0].records[0].changed_age_ns = MAX_STAMP_NS + 1,
        |a| a.terminals[0].records[0].raise.as_mut().unwrap().age_ns = MAX_STAMP_NS + 1,
        |a| a.terminals[0].replied_age_ns = MAX_STAMP_NS + 1,
        |a| a.epoch_age_ns = MAX_STAMP_NS + 1,
        |a| a.next_poll_due_ns = MAX_STAMP_NS + 1,
    ];
    for field in fields {
        red(invalid("stamp"), |_, u| field(attention(u)));
    }
    green(|_, u| {
        let attention = attention(u);
        attention.epoch_age_ns = MAX_STAMP_NS;
        attention.next_poll_due_ns = 0;
    });
}

#[test]
fn update_truncations_and_hostile_lengths_are_refused() {
    let bytes = payload();
    for length in 0..bytes.len() {
        assert!(
            decode(&checkpoint(fixture().0, bytes[..length].to_vec()), &scope()).is_err(),
            "prefix {length}"
        );
    }
    // A record blob claiming 4 GiB with nothing behind it.
    let canonical = record_frame(&record(1, ProgramState::Done)).unwrap();
    let at = bytes
        .windows(canonical.len())
        .position(|window| window == canonical)
        .unwrap()
        - 4;
    let mut hostile = bytes[..at].to_vec();
    hostile.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&checkpoint(fixture().0, hostile), &scope()).is_err());
}

#[test]
fn update_single_byte_mutations_never_panic() {
    let (core, update) = fixture();
    let bytes = encode(header(), core, &update, vec![]).unwrap();
    for at in 0..bytes.len() {
        for value in [bytes[at] ^ 0xff, 0, 0xff] {
            let mut mutated = bytes.clone();
            mutated[at] = value;
            let _ = decode(&mutated, &scope());
        }
    }
}

#[test]
fn update_supervisor_record_bounds_its_initial_command() {
    let arguments = |count: usize, bytes: usize| vec![vec![b'a'; bytes]; count];
    green(|_, update| update.supervisor.initial_command = arguments(MAX_COMMAND_ARGUMENTS, 1));
    green(|_, update| {
        update.supervisor.initial_command = arguments(1, MAX_COMMAND_ARGUMENT_BYTES);
    });
    green(|_, update| update.supervisor = Supervisor::default());
    red(invalid("initial command"), |_, update| {
        update.supervisor.initial_command = arguments(MAX_COMMAND_ARGUMENTS + 1, 0);
    });
    red_bound(invalid("initial command"), |_, update| {
        update.supervisor.initial_command = arguments(1, MAX_COMMAND_ARGUMENT_BYTES + 1);
    });
    red(invalid("initial command"), |_, update| {
        update.supervisor.initial_command[0] = b"s\0h".to_vec();
    });
}

#[test]
fn update_supervisor_record_refuses_bad_tags_and_trailing_bytes() {
    // The payload ends with the status's presence tag (1) and value (130).
    let mut tagged = payload();
    let at = tagged.len() - 2;
    assert_eq!(tagged[at..], [1, 130]);
    tagged[at] = 2;
    red_payload(invalid("presence/boolean tag"), tagged);
    let mut trailing = payload();
    trailing.push(0);
    red_payload(invalid("trailing bytes"), trailing);
}
