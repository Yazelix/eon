use super::*;

fn header() -> Header {
    Header {
        namespace: "/r".into(),
        generation: format!("g1-{}", "1".repeat(32)),
        created_build: format!("g1-{}", "2".repeat(32)),
        active_build: format!("g1-{}", "3".repeat(32)),
    }
}

fn empty() -> Checkpoint {
    Checkpoint {
        header: header(),
        core: Core {
            revision: 7,
            sessions: vec![],
            shown_session: None,
            terminal_id_high_water: 0,
        },
        sections: vec![],
    }
}

// The approved EONR v1 field order, including the 19-byte empty core.
fn golden() -> Vec<u8> {
    [
        b"EONR\x01\0\x02\0/r".as_slice(),
        b"g1-11111111111111111111111111111111",
        b"g1-22222222222222222222222222222222",
        b"g1-33333333333333333333333333333333",
        b"\x13\0\0\0\x07\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    ]
    .concat()
}

#[test]
fn checkpoint_v1_golden_decodes_and_encodes_exactly() {
    assert_eq!(
        Checkpoint::decode(&golden(), &header(), &[]).unwrap(),
        empty()
    );
    assert_eq!(empty().encode().unwrap(), golden());
}

#[test]
fn checkpoint_nested_v1_field_order_has_independent_golden_bytes() {
    // These bytes follow the approved record table, independently of the writer.
    let core = [
        b"\x07\0\0\0\0\0\0\0\x01\0\x02\0s1\x01\x30\0".as_slice(),
        b"000000000000000000000000000000000000000000000001",
        b"\x01\0\x02\0t1\0\x02\0/\xff\x01\0\x01\0",
        b"\x02\0p1\x01\x01\0x\x01\x01\0y\x01\x0a\0terminal-1\x03\0r/1",
        b"\x02\0p1\x01\0\x02\0u1\x05\0files\x05\0Files\x05\0/gone",
        b"\0\x0a\0terminal-2\x03\0r/2\x01\x02\0p1\x01\x02\0u1",
        b"\x01\x02\0t1\x01\x02\0s1\x02\0\0\0\0\0\0\0",
    ]
    .concat();
    let mut bytes = golden()[..115].to_vec();
    bytes.extend_from_slice(&(core.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&core);
    bytes.extend_from_slice(&[0, 0]);
    let expected = Checkpoint {
        header: header(),
        core: Core {
            revision: 7,
            shown_session: Some("s1".into()),
            terminal_id_high_water: 2,
            sessions: vec![Session {
                id: "s1".into(),
                persistent_id: Some(format!("{:048x}", 1)),
                active_tab: Some("t1".into()),
                tabs: vec![Tab {
                    id: "t1".into(),
                    directory: b"/\xff".to_vec(),
                    columns: vec![Column {
                        expanded: "p1".into(),
                        panes: vec![Pane {
                            id: "p1".into(),
                            pinned_name: Some("x".into()),
                            tag: Some("y".into()),
                            run: Run {
                                kind: RunKind::Confirmed,
                                terminal_id: "terminal-1".into(),
                                run_id: "r/1".into(),
                            },
                        }],
                    }],
                    popups: vec![Popup {
                        id: "u1".into(),
                        entry: "files".into(),
                        label: "Files".into(),
                        launch_directory: b"/gone".to_vec(),
                        run: Run {
                            kind: RunKind::StartIntent,
                            terminal_id: "terminal-2".into(),
                            run_id: "r/2".into(),
                        },
                    }],
                    selected_pane: Some("p1".into()),
                    selected_popup: Some("u1".into()),
                }],
            }],
        },
        sections: vec![],
    };
    assert_eq!(
        Checkpoint::decode(&bytes, &header(), &[]).unwrap(),
        expected
    );
    assert_eq!(expected.encode().unwrap(), bytes);
}

fn fixture() -> Checkpoint {
    let mut checkpoint = empty();
    checkpoint.core.shown_session = Some("s1".into());
    checkpoint.core.terminal_id_high_water = 50;
    for n in 1..=3 {
        let run = |terminal, kind| Run {
            kind,
            terminal_id: format!("terminal-{terminal}"),
            run_id: format!("opaque run/{terminal} λ"),
        };
        checkpoint.core.sessions.push(Session {
            id: format!("s{n}"),
            persistent_id: Some(format!("{n:048x}")),
            active_tab: Some(format!("t{n}")),
            tabs: vec![Tab {
                id: format!("t{n}"),
                directory: b"/gone/\xff".to_vec(),
                columns: vec![Column {
                    panes: vec![
                        Pane {
                            id: format!("p{}", 2 * n - 1),
                            pinned_name: Some("Pinned λ".into()),
                            tag: Some("review".into()),
                            run: run(3 * n - 2, RunKind::Confirmed),
                        },
                        Pane {
                            id: format!("p{}", 2 * n),
                            pinned_name: None,
                            tag: Some("independent tag".into()),
                            run: run(3 * n - 1, RunKind::StartIntent),
                        },
                    ],
                    expanded: format!("p{}", 2 * n),
                }],
                popups: vec![Popup {
                    id: format!("u{n}"),
                    entry: "files".into(),
                    label: "Files λ".into(),
                    launch_directory: b"/not-present/\xfe".to_vec(),
                    run: run(3 * n, RunKind::Confirmed),
                }],
                selected_pane: Some(format!("p{}", 2 * n)),
                selected_popup: Some(format!("u{n}")),
            }],
        });
        checkpoint.core.sessions.last_mut().unwrap().tabs[0]
            .columns
            .push(Column {
                expanded: format!("p{}", 100 + n),
                panes: vec![Pane {
                    id: format!("p{}", 100 + n),
                    pinned_name: None,
                    tag: None,
                    run: run(20 + n, RunKind::Confirmed),
                }],
            });
    }
    checkpoint.sections = vec![Section {
        name: "future-data".into(),
        required: false,
        payload: vec![0, 255, 0, 128],
    }];
    checkpoint
}

#[test]
fn checkpoint_roundtrip_keeps_private_topology_and_optional_bytes() {
    let original = fixture();
    let encoded = original.encode().unwrap();
    let decoded = Checkpoint::decode(&encoded, &header(), &[]).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(decoded.encode().unwrap(), encoded);
}

fn unchecked(checkpoint: &Checkpoint) -> Vec<u8> {
    let mut out = Encoder(Vec::new());
    checkpoint.write(&mut out).unwrap();
    out.0
}

#[test]
fn checkpoint_rejects_invalid_models_on_both_codec_sides() {
    type Corrupt = fn(&mut Checkpoint);
    let cases: &[(&str, Corrupt)] = &[
        ("relative namespace", |c| {
            c.header.namespace = "relative".into()
        }),
        ("namespace NUL", |c| c.header.namespace.push('\0')),
        ("bad generation", |c| {
            c.header.generation.replace_range(3..4, "A")
        }),
        ("bad creation build", |c| {
            c.header.created_build.replace_range(3..4, "z")
        }),
        ("bad active build", |c| {
            c.header.active_build.replace_range(0..1, "G")
        }),
        ("zero session", |c| {
            c.core.sessions[0].id = "s0".into();
            c.core.shown_session = Some("s0".into());
        }),
        ("leading-zero tab", |c| {
            c.core.sessions[0].tabs[0].id = "t01".into();
            c.core.sessions[0].active_tab = Some("t01".into());
        }),
        ("overflow pane", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].id = "p18446744073709551616".into()
        }),
        ("wrong popup prefix", |c| {
            c.core.sessions[0].tabs[0].popups[0].id = "p99".into();
            c.core.sessions[0].tabs[0].selected_popup = Some("p99".into());
        }),
        ("invalid persistent id", |c| {
            c.core.sessions[0].persistent_id = Some("not-hex".into())
        }),
        ("duplicate session", |c| c.core.sessions[1].id = "s1".into()),
        ("duplicate persistent id", |c| {
            c.core.sessions[1].persistent_id = c.core.sessions[0].persistent_id.clone()
        }),
        ("duplicate tab across parents", |c| {
            c.core.sessions[1].tabs[0].id = "t1".into();
            c.core.sessions[1].active_tab = Some("t1".into());
        }),
        ("duplicate pane across parents", |c| {
            c.core.sessions[1].tabs[0].columns[0].panes[0].id = "p1".into()
        }),
        ("duplicate popup across parents", |c| {
            c.core.sessions[1].tabs[0].popups[0].id = "u1".into();
            c.core.sessions[1].tabs[0].selected_popup = Some("u1".into());
        }),
        ("duplicate terminal/run pair", |c| {
            c.core.sessions[1].tabs[0].popups[0].run =
                c.core.sessions[0].tabs[0].popups[0].run.clone()
        }),
        ("terminal with different run in two slots", |c| {
            c.core.sessions[1].tabs[0].popups[0].run.terminal_id = "terminal-1".into()
        }),
        ("foreign active tab", |c| {
            c.core.sessions[0].active_tab = Some("t2".into())
        }),
        ("foreign selected pane", |c| {
            c.core.sessions[0].tabs[0].selected_pane = Some("p4".into())
        }),
        ("hidden selected pane", |c| {
            c.core.sessions[0].tabs[0].selected_pane = Some("p1".into())
        }),
        ("foreign selected popup", |c| {
            c.core.sessions[0].tabs[0].selected_popup = Some("u2".into())
        }),
        ("missing shown session", |c| {
            c.core.shown_session = Some("s99".into())
        }),
        ("empty column", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes.clear()
        }),
        ("foreign expanded pane", |c| {
            c.core.sessions[0].tabs[0].columns[0].expanded = "p4".into();
            c.core.sessions[0].tabs[0].selected_pane = None;
        }),
        ("empty pin", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].pinned_name = Some(String::new())
        }),
        ("untrimmed pin", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].pinned_name = Some(" pin".into())
        }),
        ("control tag", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].tag = Some("a\nb".into())
        }),
        ("invisible tag", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].tag = Some("a\u{202e}b".into())
        }),
        ("overlong pin", |c| {
            c.core.sessions[0].tabs[0].columns[0].panes[0].pinned_name = Some("x".repeat(81))
        }),
        ("relative directory", |c| {
            c.core.sessions[0].tabs[0].directory = b"relative".to_vec()
        }),
        ("directory NUL", |c| {
            c.core.sessions[0].tabs[0].directory = b"/a\0b".to_vec()
        }),
        ("overlong directory", |c| {
            c.core.sessions[0].tabs[0].directory = vec![b'/'; MAX_DIRECTORY_BYTES + 1]
        }),
        ("relative popup directory", |c| {
            c.core.sessions[0].tabs[0].popups[0].launch_directory = b"relative".to_vec()
        }),
        ("invalid popup key", |c| {
            c.core.sessions[0].tabs[0].popups[0].entry = "not.a.key".into()
        }),
        ("duplicate popup entry", |c| {
            let mut popup = c.core.sessions[0].tabs[0].popups[0].clone();
            popup.id = "u99".into();
            popup.run = Run {
                kind: RunKind::Confirmed,
                terminal_id: "terminal-99".into(),
                run_id: "run-99".into(),
            };
            c.core.terminal_id_high_water = 99;
            c.core.sessions[0].tabs[0].popups.push(popup);
        }),
        ("empty popup label", |c| {
            c.core.sessions[0].tabs[0].popups[0].label.clear()
        }),
        ("control popup label", |c| {
            c.core.sessions[0].tabs[0].popups[0].label = "bad\nlabel".into()
        }),
        ("overlong popup label", |c| {
            c.core.sessions[0].tabs[0].popups[0].label = "x".repeat(MAX_LABEL_BYTES + 1)
        }),
        ("zero terminal", |c| {
            c.core.sessions[0].tabs[0].popups[0].run.terminal_id = "terminal-0".into()
        }),
        ("noncanonical terminal", |c| {
            c.core.sessions[0].tabs[0].popups[0].run.terminal_id = "terminal-03".into()
        }),
        ("terminal overflow", |c| {
            c.core.sessions[0].tabs[0].popups[0].run.terminal_id =
                "terminal-18446744073709551616".into()
        }),
        ("empty run", |c| {
            c.core.sessions[0].tabs[0].popups[0].run.run_id.clear()
        }),
        ("overlong run", |c| {
            c.core.sessions[0].tabs[0].popups[0].run.run_id = "x".repeat(MAX_ID_BYTES + 1)
        }),
        ("low high-water", |c| c.core.terminal_id_high_water = 8),
        ("duplicate section", |c| {
            c.sections.push(c.sections[0].clone())
        }),
        ("empty section name", |c| c.sections[0].name.clear()),
        ("overlong section name", |c| {
            c.sections[0].name = "x".repeat(MAX_ID_BYTES + 1)
        }),
    ];
    for (why, corrupt) in cases {
        let mut bad = fixture();
        corrupt(&mut bad);
        assert!(bad.encode().is_err(), "encoder accepted {why}");
        let wire = unchecked(&bad);
        assert!(
            Checkpoint::decode(&wire, &header(), &[]).is_err(),
            "decoder accepted {why}"
        );
    }
}

#[test]
fn checkpoint_checks_each_independent_scope_role_and_required_section() {
    let original = fixture();
    let bytes = original.encode().unwrap();
    let fields = ["namespace", "generation", "created_build", "active_build"];
    for field in fields {
        let mut expected = header();
        match field {
            "namespace" => expected.namespace = "/other".into(),
            "generation" => expected.generation = format!("g1-{}", "4".repeat(32)),
            "created_build" => expected.created_build = format!("g1-{}", "4".repeat(32)),
            "active_build" => expected.active_build = format!("g1-{}", "4".repeat(32)),
            _ => unreachable!(),
        }
        assert_eq!(
            Checkpoint::decode(&bytes, &expected, &[]),
            Err(Error::ScopeMismatch(field))
        );
    }
    let mut required = original;
    required.sections[0].required = true;
    let bytes = required.encode().unwrap();
    assert_eq!(
        Checkpoint::decode(&bytes, &header(), &[]),
        Err(Error::UnknownRequiredSection("future-data".into()))
    );
    assert_eq!(
        Checkpoint::decode(&bytes, &header(), &["future-data"]).unwrap(),
        required
    );
    assert!(Checkpoint::decode(&bytes, &header(), &["future"]).is_err());
}

fn offset(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|part| part == needle)
        .unwrap()
}

#[test]
fn checkpoint_rejects_truncation_invalid_tags_and_unconsumed_bytes() {
    let original = fixture().encode().unwrap();
    for length in 0..original.len() {
        assert!(
            Checkpoint::decode(&original[..length], &header(), &[]).is_err(),
            "accepted prefix {length}"
        );
    }
    let mut bad = original.clone();
    bad.extend_from_slice(b"trailing");
    assert!(Checkpoint::decode(&bad, &header(), &[]).is_err());

    for (at, value) in [
        (0, b'X'),
        (offset(&original, b"\x02\0t1") + 4, 1), // Only fixed directory policy 0 exists.
        (offset(&original, b"\x02\0p1") + 4, 2), // Pin presence.
        (offset(&original, b"terminal-1") - 3, 2), // Run kind.
        (offset(&original, b"future-data") + "future-data".len(), 2), // Section required flag.
        (offset(&original, b"Files \xce\xbb"), 0xff), // UTF-8 label.
    ] {
        let mut bad = original.clone();
        bad[at] = value;
        assert!(
            Checkpoint::decode(&bad, &header(), &[]).is_err(),
            "accepted bad byte at {at}"
        );
    }
    for version in [0_u16, 2, u16::MAX] {
        let mut bad = original.clone();
        bad[4..6].copy_from_slice(&version.to_le_bytes());
        assert_eq!(
            Checkpoint::decode(&bad, &header(), &[]),
            Err(Error::UnsupportedVersion(version))
        );
    }
    let mut bad = original.clone();
    let payload_length = offset(&bad, b"future-data") + "future-data".len() + 1;
    bad[payload_length..payload_length + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Checkpoint::decode(&bad, &header(), &[]).is_err());

    // The empty-core golden independently locates the core length, count and presence tag.
    for (range, replacement) in [
        (115..119, u32::MAX.to_le_bytes().to_vec()),
        (127..129, u16::MAX.to_le_bytes().to_vec()),
        (129..130, vec![2]),
    ] {
        let mut bad = golden();
        bad[range].copy_from_slice(&replacement);
        assert!(Checkpoint::decode(&bad, &header(), &[]).is_err());
    }
    let mut bad = golden();
    bad.insert(138, 0); // Trailing byte inside, not after, the length-delimited core.
    bad[115..119].copy_from_slice(&20_u32.to_le_bytes());
    assert_eq!(
        Checkpoint::decode(&bad, &header(), &[]),
        Err(Error::Invalid("trailing bytes"))
    );
}

fn full_slots(length: usize) -> Checkpoint {
    let mut checkpoint = fixture();
    checkpoint.core.sessions.truncate(1);
    checkpoint.sections.clear();
    checkpoint.core.terminal_id_high_water = length as u64;
    let tab = &mut checkpoint.core.sessions[0].tabs[0];
    tab.popups.clear();
    tab.selected_popup = None;
    tab.selected_pane = Some("p1".into());
    tab.columns = (0..2)
        .map(|column| {
            let panes = (1..=length)
                .filter(|n| (n - 1) % 2 == column)
                .map(|n| Pane {
                    id: format!("p{n}"),
                    pinned_name: None,
                    tag: None,
                    run: Run {
                        kind: RunKind::StartIntent,
                        terminal_id: format!("terminal-{n}"),
                        run_id: format!("r{n}"),
                    },
                })
                .collect::<Vec<_>>();
            Column {
                expanded: panes[0].id.clone(),
                panes,
            }
        })
        .collect();
    checkpoint
}

#[test]
fn checkpoint_bounds_total_slots_before_decode_allocation_and_preserves_exhaustion() {
    let checkpoint = full_slots(MAX_SESSIONS);
    let bytes = checkpoint.encode().unwrap();
    assert_eq!(
        Checkpoint::decode(&bytes, &header(), &[]).unwrap(),
        checkpoint
    );
    let too_many = full_slots(MAX_SESSIONS + 1);
    assert!(too_many.encode().is_err());
    assert_eq!(
        Checkpoint::decode(&unchecked(&too_many), &header(), &[]),
        Err(Error::Invalid("terminal slots"))
    );

    let mut maximum = empty();
    maximum.core.revision = u64::MAX;
    maximum.core.terminal_id_high_water = u64::MAX;
    assert_eq!(
        Checkpoint::decode(&maximum.encode().unwrap(), &header(), &[]).unwrap(),
        maximum
    );
    let mut maximum = fixture();
    let pane = &mut maximum.core.sessions[0].tabs[0].columns[0].panes[0];
    pane.id = format!("p{}", u64::MAX);
    pane.run.terminal_id = format!("terminal-{}", u64::MAX);
    maximum.core.terminal_id_high_water = u64::MAX;
    assert_eq!(
        Checkpoint::decode(&maximum.encode().unwrap(), &header(), &[]).unwrap(),
        maximum
    );
}

#[test]
fn checkpoint_resource_guards_run_before_reading_or_allocating_payloads() {
    let mut input = Decoder::new(&[1, 0, 0]);
    input.slots = MAX_SESSIONS;
    assert_eq!(
        input.slot_count("panes", MAX_SESSIONS),
        Err(Error::Invalid("terminal slots"))
    );
    assert_eq!(input.bytes, &[0]); // The nonexistent pane was never read.

    let length = ((MAX_DIRECTORY_BYTES + 1) as u16).to_le_bytes();
    assert_eq!(
        Decoder::new(&length).raw(MAX_DIRECTORY_BYTES),
        Err(Error::Invalid("string size"))
    );
    assert_eq!(
        Decoder::new(&u32::MAX.to_le_bytes()).blob(),
        Err(Error::Invalid("blob size"))
    );
}

#[test]
fn checkpoint_accepts_exact_limits_and_rejects_the_next_value() {
    let mut maximum = fixture();
    let pane = &mut maximum.core.sessions[0].tabs[0].columns[0].panes[0];
    pane.pinned_name = Some("\u{1f680}".repeat(80));
    pane.tag = Some("x".repeat(80));
    pane.run.run_id = "x".repeat(MAX_ID_BYTES);
    let tab = &mut maximum.core.sessions[0].tabs[0];
    tab.directory = vec![b'/'; MAX_DIRECTORY_BYTES];
    tab.popups[0].entry = "a".repeat(MAX_ENTRY_ID_BYTES);
    tab.popups[0].label = "a".repeat(MAX_LABEL_BYTES);
    maximum.sections[0].name = "x".repeat(MAX_ID_BYTES);
    assert_eq!(
        Checkpoint::decode(&maximum.encode().unwrap(), &header(), &[]).unwrap(),
        maximum
    );

    let mut maximum = empty();
    maximum.core.sessions = (1..=MAX_WORKSPACES)
        .map(|n| Session {
            id: format!("s{n}"),
            persistent_id: None,
            tabs: vec![],
            active_tab: None,
        })
        .collect();
    maximum.core.sessions[0].tabs = (1..=MAX_TABS)
        .map(|n| Tab {
            id: format!("t{n}"),
            directory: b"/gone".to_vec(),
            columns: vec![],
            popups: vec![],
            selected_pane: None,
            selected_popup: None,
        })
        .collect();
    maximum.sections = (0..MAX_SECTIONS)
        .map(|n| Section {
            name: format!("optional-{n}"),
            required: false,
            payload: vec![],
        })
        .collect();
    assert_eq!(
        Checkpoint::decode(&maximum.encode().unwrap(), &header(), &[]).unwrap(),
        maximum
    );
    let mut bad = maximum.clone();
    bad.core.sessions.push(Session {
        id: "s33".into(),
        persistent_id: None,
        tabs: vec![],
        active_tab: None,
    });
    assert!(bad.encode().is_err());
    assert!(Checkpoint::decode(&unchecked(&bad), &header(), &[]).is_err());
    let mut bad = maximum.clone();
    let mut tab = bad.core.sessions[0].tabs[0].clone();
    tab.id = "t65".into();
    bad.core.sessions[0].tabs.push(tab);
    assert!(bad.encode().is_err());
    assert!(Checkpoint::decode(&unchecked(&bad), &header(), &[]).is_err());
    let mut bad = maximum;
    bad.sections.push(Section {
        name: "overflow".into(),
        required: false,
        payload: vec![],
    });
    assert!(bad.encode().is_err());
    assert!(Checkpoint::decode(&unchecked(&bad), &header(), &[]).is_err());
}

#[test]
fn checkpoint_enforces_the_combined_byte_ceiling_and_declared_lengths() {
    let mut checkpoint = empty();
    checkpoint.sections.push(Section {
        name: "padding".into(),
        required: false,
        payload: vec![],
    });
    let overhead = checkpoint.encode().unwrap().len();
    checkpoint.sections[0].payload = vec![0xa5; MAX_BYTES - overhead];
    let bytes = checkpoint.encode().unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    assert_eq!(
        Checkpoint::decode(&bytes, &header(), &[]).unwrap(),
        checkpoint
    );
    checkpoint.sections[0].payload.push(0);
    assert!(checkpoint.encode().is_err());
    let mut over = bytes;
    over.push(0);
    assert!(Checkpoint::decode(&over, &header(), &[]).is_err());
}

#[test]
fn checkpoint_bounds_columns_and_popup_slots_independently() {
    let mut checkpoint = full_slots(MAX_COLUMNS);
    let tab = &mut checkpoint.core.sessions[0].tabs[0];
    tab.columns = std::mem::take(&mut tab.columns)
        .into_iter()
        .flat_map(|column| column.panes)
        .map(|pane| Column {
            expanded: pane.id.clone(),
            panes: vec![pane],
        })
        .collect();
    tab.popups = (1..=MAX_ENTRIES)
        .map(|n| Popup {
            id: format!("u{n}"),
            entry: format!("tool-{n}"),
            label: format!("Tool {n}"),
            launch_directory: b"/gone".to_vec(),
            run: Run {
                kind: RunKind::Confirmed,
                terminal_id: format!("terminal-{}", 100 + n),
                run_id: format!("run-{}", 100 + n),
            },
        })
        .collect();
    checkpoint.core.terminal_id_high_water = 999;
    assert_eq!(
        Checkpoint::decode(&checkpoint.encode().unwrap(), &header(), &[]).unwrap(),
        checkpoint
    );

    let mut bad = checkpoint.clone();
    bad.core.sessions[0].tabs[0].columns.push(Column {
        expanded: "p999".into(),
        panes: vec![Pane {
            id: "p999".into(),
            pinned_name: None,
            tag: None,
            run: Run {
                kind: RunKind::StartIntent,
                terminal_id: "terminal-999".into(),
                run_id: "r999".into(),
            },
        }],
    });
    assert!(bad.encode().is_err());
    assert_eq!(
        Checkpoint::decode(&unchecked(&bad), &header(), &[]),
        Err(Error::Invalid("columns"))
    );

    let mut bad = checkpoint;
    bad.core.sessions[0].tabs[0].popups.push(Popup {
        id: "u999".into(),
        entry: "overflow".into(),
        label: "Overflow".into(),
        launch_directory: b"/gone".to_vec(),
        run: Run {
            kind: RunKind::Confirmed,
            terminal_id: "terminal-999".into(),
            run_id: "r999".into(),
        },
    });
    assert!(bad.encode().is_err());
    assert_eq!(
        Checkpoint::decode(&unchecked(&bad), &header(), &[]),
        Err(Error::Invalid("popups"))
    );
}
