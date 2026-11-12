use orbit_protocol::management::{
    self, ClientMessage, EndpointIdentity, Error, Failure, FailureCode, Foreground, HistoryText,
    LeaseInput, LiveIdentity, ObjectIdentity, ProcessOutcome, ProgramKind, ProgramRecord,
    ProgramState, ProgramStatus, Record, Refusal, Screen, ServerMessage, TerminationReason,
    Tombstone, VisibleText,
};

fn program_record(id: &str, state: ProgramState) -> ProgramRecord {
    ProgramRecord {
        id: id.into(),
        state,
        kind: (state == ProgramState::Blocked).then_some(ProgramKind::Permission),
        progress: Some(40),
        app: "claude-code".into(),
        title: "Fix the build".into(),
        message: "Apply these changes?".into(),
        updated: 9,
        created: 3,
        changed: 7,
        previous: Some(ProgramState::Working),
        updated_age_ms: 120,
        changed_age_ms: 900,
    }
}

fn live_identity() -> LiveIdentity {
    LiveIdentity {
        session_id: "session-1".into(),
        run_id: "run-1".into(),
        component_generation: "component-1".into(),
        record_generation: management::RECORD_GENERATION,
        management_generation: management::VERSION,
        process_id: 42,
        process_start: 99,
        uid: 1000,
        presentation: EndpointIdentity {
            path: b"/run/user/1000/orbit.sock".to_vec(),
            object: ObjectIdentity {
                device: 7,
                inode: 11,
            },
        },
        management: EndpointIdentity {
            path: b"/run/user/1000/orbit.sock.management".to_vec(),
            object: ObjectIdentity {
                device: 7,
                inode: 12,
            },
        },
    }
}

fn visible() -> VisibleText {
    VisibleText {
        revision: 12,
        screen: Screen::Primary,
        rows: 24,
        columns: 80,
        title: "build 🪐".into(),
        directory: b"/home/user/project".to_vec(),
        omitted_rows: 0,
        lossy: true,
        text: "$ cargo test\n   Compiling 漢字 e\u{301}\n".into(),
    }
}

fn history() -> HistoryText {
    HistoryText {
        revision: 12,
        screen: Screen::Primary,
        rows: 24,
        columns: 80,
        title: "build 🪐".into(),
        directory: b"/home/user/project".to_vec(),
        omitted_lines: 0,
        cut_bytes: 0,
        lossy: true,
        text: "$ seq 3\n1\n2\n3\n   Compiling 漢字 e\u{301}".into(),
    }
}

fn tombstone() -> Tombstone {
    Tombstone {
        identity: live_identity(),
        reason: TerminationReason::ExplicitStop,
        outcome: ProcessOutcome::Signal(1),
    }
}

#[test]
fn management_messages_and_records_round_trip_canonically() {
    let identity = live_identity();
    let client_messages = [
        ClientMessage::Acquire {
            expected: identity.clone(),
            record: ObjectIdentity {
                device: 7,
                inode: 13,
            },
        },
        ClientMessage::Status,
        ClientMessage::Stop,
        ClientMessage::ProgramStatus,
        ClientMessage::ReadVisible,
        ClientMessage::ReadHistory,
        ClientMessage::Input {
            input: LeaseInput::Paste("echo ok\tdone\r\n🪐\u{1b}".into()),
            quiet_ms: 0,
        },
        ClientMessage::Input {
            input: LeaseInput::Enter,
            quiet_ms: 2000,
        },
        ClientMessage::Input {
            input: LeaseInput::Paste("x".into()),
            quiet_ms: u16::MAX,
        },
        ClientMessage::AttachAllow {
            process_id: 4242,
            process_start: 987_654_321,
        },
    ];
    for message in client_messages {
        let bytes = management::encode_client_message(&message).unwrap();
        assert_eq!(
            management::client_message_len(&bytes).unwrap(),
            Some(bytes.len())
        );
        assert_eq!(management::decode_client_message(&bytes).unwrap(), message);
    }

    let server_messages = [
        ServerMessage::Lease(identity.clone()),
        ServerMessage::Status(identity.clone()),
        ServerMessage::Busy,
        ServerMessage::Stopped(tombstone()),
        ServerMessage::Failure(Failure {
            code: FailureCode::InvalidIdentity,
            detail: "identity mismatch".into(),
        }),
        ServerMessage::ProgramStatus(ProgramStatus {
            count: 3,
            background: 2,
            processes: Default::default(),
            foreground: Foreground::Other,
            update: 9,
            terminal_title: "eon-9a 🪐".into(),
            omitted: vec![2],
            records: vec![
                program_record("agent/1", ProgramState::Blocked),
                ProgramRecord {
                    kind: None,
                    progress: None,
                    previous: None,
                    ..program_record("", ProgramState::Idle)
                },
            ],
        }),
        ServerMessage::Visible(visible()),
        ServerMessage::Visible(VisibleText {
            screen: Screen::Alternate,
            title: String::new(),
            directory: b"/tmp/\xff".to_vec(),
            omitted_rows: 7,
            text: String::new(),
            ..visible()
        }),
        ServerMessage::History(history()),
        ServerMessage::History(HistoryText {
            screen: Screen::Alternate,
            title: String::new(),
            directory: b"/tmp/\xff".to_vec(),
            omitted_lines: 7,
            cut_bytes: u32::MAX,
            text: String::new(),
            ..history()
        }),
        ServerMessage::Input { bracketed: true },
        ServerMessage::Input { bracketed: false },
        ServerMessage::Allowed,
        ServerMessage::Refused(Refusal::InvalidRequest),
        ServerMessage::Refused(Refusal::UnbracketedNewline),
        ServerMessage::Refused(Refusal::InvalidInput),
        ServerMessage::Refused(Refusal::InputFull),
        ServerMessage::Refused(Refusal::ReadFailed),
        ServerMessage::Refused(Refusal::Closed),
        ServerMessage::Refused(Refusal::Typing),
    ];
    for message in server_messages {
        let bytes = management::encode_server_message(&message).unwrap();
        assert_eq!(
            management::server_message_len(&bytes).unwrap(),
            Some(bytes.len())
        );
        assert_eq!(management::decode_server_message(&bytes).unwrap(), message);
    }

    for record in [Record::Live(identity), Record::Tombstone(tombstone())] {
        let bytes = management::encode_record(&record).unwrap();
        assert!(bytes.len() <= management::MAX_RECORD_BYTES);
        assert_eq!(management::decode_record(&bytes).unwrap(), record);
    }
}

#[test]
fn management_codec_enforces_identity_detail_and_frame_bounds() {
    let mut identity = live_identity();
    identity.session_id.clear();
    assert!(matches!(
        management::encode_record(&Record::Live(identity)),
        Err(Error::InvalidValue {
            field: "session ID"
        })
    ));

    let mut identity = live_identity();
    identity.run_id = "x".repeat(management::MAX_ID_BYTES + 1);
    assert!(matches!(
        management::encode_record(&Record::Live(identity)),
        Err(Error::ValueTooLarge {
            field: "run ID",
            ..
        })
    ));

    let oversized_detail = ServerMessage::Failure(Failure {
        code: FailureCode::InvalidRequest,
        detail: "x".repeat(management::MAX_DETAIL_BYTES + 1),
    });
    assert!(matches!(
        management::encode_server_message(&oversized_detail),
        Err(Error::ValueTooLarge {
            field: "failure detail",
            ..
        })
    ));

    let mut invalid_outcome = tombstone();
    invalid_outcome.outcome = ProcessOutcome::ExitCode(-1);
    assert!(matches!(
        management::encode_record(&Record::Tombstone(invalid_outcome)),
        Err(Error::InvalidValue { field: "exit code" })
    ));

    let mut header = [0; management::HEADER_BYTES];
    header[..4].copy_from_slice(b"ORBM");
    header[4..6].copy_from_slice(&management::VERSION.to_le_bytes());
    header[6] = 2;
    header[8..12].copy_from_slice(&(management::MAX_MESSAGE_BYTES as u32).to_le_bytes());
    assert!(matches!(
        management::client_message_len(&header),
        Err(Error::PayloadTooLarge { .. })
    ));
}

#[test]
fn management_codec_rejects_incompatible_or_noncanonical_bytes() {
    let bytes = management::encode_client_message(&ClientMessage::Status).unwrap();
    assert_eq!(management::client_message_len(&bytes[..4]).unwrap(), None);
    assert!(matches!(
        management::decode_client_message(&bytes[..bytes.len() - 1]),
        Err(Error::Truncated)
    ));

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(matches!(
        management::decode_client_message(&trailing),
        Err(Error::TrailingBytes { count: 1 })
    ));

    // Generations are exact: an older or a newer peer is refused.
    for generation in [management::VERSION - 1, management::VERSION + 1] {
        let mut other = bytes.clone();
        other[4..6].copy_from_slice(&generation.to_le_bytes());
        assert!(matches!(
            management::decode_client_message(&other),
            Err(Error::UnsupportedVersion { .. })
        ));
    }

    let mut invalid_tag = bytes;
    invalid_tag[6] = u8::MAX;
    assert!(matches!(
        management::decode_client_message(&invalid_tag),
        Err(Error::InvalidTag { .. })
    ));
}

/// Orbit fills a reply by `PROGRAM_STATUS_HEADER_BYTES`, `program_record_len`
/// and `PROGRAM_OMITTED_BYTES`, so they must match the encoding; at the
/// reply's text and process bounds six records fit in 4 KiB beside eight
/// omitted creation sequences, and three beside 253 (ORB-C15).
#[test]
fn program_status_replies_keep_their_bounds() {
    let full = ProgramRecord {
        id: "a".repeat(management::MAX_PROGRAM_ID_BYTES),
        app: "a".repeat(management::MAX_PROGRAM_APP_BYTES),
        title: "a".repeat(management::MAX_PROGRAM_TITLE_BYTES),
        message: "a".repeat(management::MAX_PROGRAM_MESSAGE_BYTES),
        ..program_record("", ProgramState::Blocked)
    };
    let title = "a".repeat(management::MAX_TERMINAL_TITLE_BYTES);
    let reply = |records: Vec<ProgramRecord>, omitted: usize| {
        ServerMessage::ProgramStatus(ProgramStatus {
            count: (records.len() + omitted) as u16,
            background: u8::MAX,
            processes: management::ProgramProcesses {
                background: (1..=16)
                    .map(|pid| management::ProcessIdentity { pid, start: 1 })
                    .collect(),
                complete: false,
                foreground: Some(management::ProcessIdentity { pid: 20, start: 1 }),
            },
            foreground: Foreground::Child,
            update: 1,
            terminal_title: title.clone(),
            omitted: (0..omitted as u64).collect(),
            records,
        })
    };
    let six = management::encode_server_message(&reply(vec![full.clone(); 6], 8)).unwrap();
    assert_eq!(six.len(), 3747);
    assert!(matches!(
        management::encode_server_message(&reply(vec![full.clone(); 7], 7)),
        Err(Error::PayloadTooLarge { .. })
    ));
    let three = reply(vec![full.clone(); 3], 253);
    let bytes = management::encode_server_message(&three).unwrap();
    assert_eq!(bytes.len(), 4015);
    assert_eq!(management::decode_server_message(&bytes).unwrap(), three);
    assert!(matches!(
        management::encode_server_message(&reply(vec![full.clone(); 4], 252)),
        Err(Error::PayloadTooLarge { .. })
    ));

    let refused = |status: ProgramStatus| {
        management::encode_server_message(&ServerMessage::ProgramStatus(status))
    };
    let mut kind_without_block = program_record("", ProgramState::Working);
    kind_without_block.kind = Some(ProgramKind::Question);
    let mut overfull = program_record("", ProgramState::Done);
    overfull.progress = Some(101);
    for records in [vec![kind_without_block], vec![overfull]] {
        assert!(matches!(
            refused(ProgramStatus {
                count: 1,
                background: 0,
                processes: Default::default(),
                foreground: Foreground::Child,
                update: 1,
                terminal_title: String::new(),
                omitted: Vec::new(),
                records
            }),
            Err(Error::InvalidValue { .. })
        ));
    }
    assert!(matches!(
        refused(ProgramStatus {
            count: 2,
            background: 0,
            processes: Default::default(),
            foreground: Foreground::Child,
            update: 1,
            terminal_title: String::new(),
            omitted: Vec::new(),
            records: vec![full]
        }),
        Err(Error::InvalidValue { .. })
    ));
    assert!(matches!(
        refused(ProgramStatus {
            count: 0,
            background: 0,
            processes: Default::default(),
            foreground: Foreground::Child,
            update: 1,
            terminal_title: "a".repeat(management::MAX_TERMINAL_TITLE_BYTES + 1),
            omitted: Vec::new(),
            records: Vec::new()
        }),
        Err(Error::ValueTooLarge { .. })
    ));
}

/// Management v4's new messages keep their exact bytes under generation 8,
/// except lease input, which carries its quiet window after its kind (since
/// v6); v7 added ReadHistory and History; v8 adds the process census: the header's generation, each
/// kind's tag, and its payload.
#[test]
fn management_v8_messages_have_canonical_bytes() {
    assert_eq!(management::VERSION, 8);
    let framed = |kind: u8, payload: &[u8]| {
        let mut bytes = b"ORBM\x08\x00".to_vec();
        bytes.extend([kind, 0]);
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    };
    let client = |message| management::encode_client_message(&message).unwrap();
    let server = |message| management::encode_server_message(&message).unwrap();
    assert_eq!(client(ClientMessage::ReadVisible), framed(5, &[]));
    assert_eq!(client(ClientMessage::ReadHistory), framed(8, &[]));
    assert_eq!(
        client(ClientMessage::Input {
            input: LeaseInput::Enter,
            quiet_ms: 0,
        }),
        framed(6, &[2, 0, 0])
    );
    assert_eq!(
        client(ClientMessage::Input {
            input: LeaseInput::Paste("ls".into()),
            quiet_ms: 2000,
        }),
        framed(6, b"\x01\xd0\x07ls")
    );
    let mut allow = 7u32.to_le_bytes().to_vec();
    allow.extend(9u64.to_le_bytes());
    assert_eq!(
        client(ClientMessage::AttachAllow {
            process_id: 7,
            process_start: 9,
        }),
        framed(7, &allow)
    );
    assert_eq!(
        server(ServerMessage::Input { bracketed: true }),
        framed(73, &[1])
    );
    assert_eq!(server(ServerMessage::Allowed), framed(74, &[]));
    assert_eq!(
        server(ServerMessage::Refused(Refusal::ReadFailed)),
        framed(75, &[5])
    );
    assert_eq!(
        server(ServerMessage::Refused(Refusal::Closed)),
        framed(75, &[6])
    );
    assert_eq!(
        server(ServerMessage::Refused(Refusal::Typing)),
        framed(75, &[7])
    );
    let mut payload = 3u64.to_le_bytes().to_vec();
    payload.push(2);
    payload.extend(2u16.to_le_bytes());
    payload.extend(4u16.to_le_bytes());
    payload.extend(1u32.to_le_bytes());
    payload.push(b't');
    payload.extend(2u32.to_le_bytes());
    payload.extend(b"/x");
    payload.extend(5u32.to_le_bytes());
    payload.push(1);
    payload.extend(3u32.to_le_bytes());
    payload.extend(b"a\nb");
    assert_eq!(
        server(ServerMessage::Visible(VisibleText {
            revision: 3,
            screen: Screen::Alternate,
            rows: 2,
            columns: 4,
            title: "t".into(),
            directory: b"/x".to_vec(),
            omitted_rows: 5,
            lossy: true,
            text: "a\nb".into(),
        })),
        framed(72, &payload)
    );
    let mut payload = 3u64.to_le_bytes().to_vec();
    payload.push(1);
    payload.extend(2u16.to_le_bytes());
    payload.extend(4u16.to_le_bytes());
    payload.extend(1u32.to_le_bytes());
    payload.push(b't');
    payload.extend(2u32.to_le_bytes());
    payload.extend(b"/x");
    payload.extend(5u32.to_le_bytes());
    payload.extend(6u32.to_le_bytes());
    payload.push(1);
    payload.extend(3u32.to_le_bytes());
    payload.extend(b"a\nb");
    assert_eq!(
        server(ServerMessage::History(HistoryText {
            revision: 3,
            screen: Screen::Primary,
            rows: 2,
            columns: 4,
            title: "t".into(),
            directory: b"/x".to_vec(),
            omitted_lines: 5,
            cut_bytes: 6,
            lossy: true,
            text: "a\nb".into(),
        })),
        framed(76, &payload)
    );
}

/// A `ProgramStatus` reply carries the terminal's foreground as one byte
/// after the background count (since management v5): each value keeps its tag and
/// round-trips, and a decoder refuses any other tag.
#[test]
fn process_census_is_bounded_complete_and_identity_checked() {
    use management::{ProcessIdentity, ProgramProcesses};
    let identity = |pid| ProcessIdentity { pid, start: 10 };
    let status = ProgramStatus {
        count: 1,
        background: 2,
        processes: ProgramProcesses {
            background: vec![identity(2), identity(3)],
            complete: true,
            foreground: Some(identity(1)),
        },
        foreground: Foreground::Child,
        update: 1,
        terminal_title: String::new(),
        omitted: Vec::new(),
        records: vec![program_record("", ProgramState::Idle)],
    };
    let message = ServerMessage::ProgramStatus(status.clone());
    let bytes = management::encode_server_message(&message).unwrap();
    assert_eq!(management::decode_server_message(&bytes).unwrap(), message);
    for processes in [
        ProgramProcesses {
            background: vec![identity(2)],
            ..status.processes.clone()
        },
        ProgramProcesses {
            background: vec![identity(2), identity(2)],
            ..status.processes.clone()
        },
        ProgramProcesses {
            background: vec![identity(0), identity(2)],
            ..status.processes.clone()
        },
        ProgramProcesses {
            background: (1..=17).map(identity).collect(),
            complete: false,
            foreground: None,
        },
    ] {
        assert!(
            management::encode_server_message(&ServerMessage::ProgramStatus(ProgramStatus {
                processes,
                ..status.clone()
            }))
            .is_err()
        );
    }
    for offset in [management::HEADER_BYTES + 4, management::HEADER_BYTES + 5] {
        let mut bad = bytes.clone();
        bad[offset] = 2;
        assert!(management::decode_server_message(&bad).is_err());
    }
    let saturated = ProgramStatus {
        background: 255,
        ..status.clone()
    };
    assert!(management::encode_server_message(&ServerMessage::ProgramStatus(saturated)).is_err());
    let incomplete = ServerMessage::ProgramStatus(ProgramStatus {
        background: 255,
        processes: ProgramProcesses {
            complete: false,
            ..status.processes
        },
        ..status
    });
    assert!(management::encode_server_message(&incomplete).is_ok());
}

#[test]
fn program_status_carries_the_foreground() {
    let status = |foreground| ProgramStatus {
        count: 1,
        background: 2,
        processes: Default::default(),
        foreground,
        update: 9,
        terminal_title: "t".into(),
        omitted: vec![4],
        records: Vec::new(),
    };
    let mut payload = 1u16.to_le_bytes().to_vec();
    payload.push(2);
    payload.extend([0, 0, 0]);
    payload.push(2);
    payload.extend(9u64.to_le_bytes());
    payload.extend(1u16.to_le_bytes());
    payload.push(b't');
    payload.extend(1u16.to_le_bytes());
    payload.extend(4u64.to_le_bytes());
    let mut golden = b"ORBM\x08\x00".to_vec();
    golden.extend([71, 0]);
    golden.extend((payload.len() as u32).to_le_bytes());
    golden.extend(&payload);
    assert_eq!(
        management::encode_server_message(&ServerMessage::ProgramStatus(status(Foreground::Other)))
            .unwrap(),
        golden
    );

    let at = management::HEADER_BYTES + 2 + 1 + 3;
    for (foreground, tag) in [
        (Foreground::Unknown, 0),
        (Foreground::Child, 1),
        (Foreground::Other, 2),
    ] {
        let message = ServerMessage::ProgramStatus(status(foreground));
        let bytes = management::encode_server_message(&message).unwrap();
        assert_eq!(bytes[at], tag);
        assert_eq!(management::decode_server_message(&bytes).unwrap(), message);
    }
    golden[at] = 3;
    assert!(matches!(
        management::decode_server_message(&golden),
        Err(Error::InvalidTag {
            field: "foreground",
            value: 3
        })
    ));
}

/// A paste is bounded at 64 KiB, a Visible reply's text at 1 MiB and a
/// History reply's at 8 MiB, each by its own kind's frame bound; every other message keeps 4 KiB, unknown kinds
/// included, so they arrive whole and are refused by the decoder.
#[test]
fn management_v4_bounds_are_per_kind() {
    let full = "a".repeat(management::MAX_INPUT_PASTE_BYTES);
    let paste = |text: String| ClientMessage::Input {
        input: LeaseInput::Paste(text),
        quiet_ms: 0,
    };
    let bytes = management::encode_client_message(&paste(full.clone())).unwrap();
    assert_eq!(bytes.len(), management::MAX_INPUT_MESSAGE_BYTES);
    assert_eq!(
        management::client_message_len(&bytes).unwrap(),
        Some(bytes.len())
    );
    assert_eq!(
        management::decode_client_message(&bytes).unwrap(),
        paste(full.clone())
    );
    assert!(matches!(
        management::encode_client_message(&paste(full.clone() + "a")),
        Err(Error::ValueTooLarge {
            field: "paste text",
            ..
        })
    ));
    assert!(matches!(
        management::encode_client_message(&paste(String::new())),
        Err(Error::InvalidValue {
            field: "paste text"
        })
    ));

    let header = |kind: u8, payload: usize| {
        let mut header = [0; management::HEADER_BYTES];
        header[..4].copy_from_slice(b"ORBM");
        header[4..6].copy_from_slice(&management::VERSION.to_le_bytes());
        header[6] = kind;
        header[8..12].copy_from_slice(&(payload as u32).to_le_bytes());
        header
    };
    let input_payload = management::MAX_INPUT_MESSAGE_BYTES - management::HEADER_BYTES;
    assert_eq!(
        management::client_message_len(&header(6, input_payload)).unwrap(),
        Some(management::MAX_INPUT_MESSAGE_BYTES)
    );
    assert!(matches!(
        management::client_message_len(&header(6, input_payload + 1)),
        Err(Error::PayloadTooLarge { .. })
    ));
    let small = management::MAX_MESSAGE_BYTES - management::HEADER_BYTES;
    for kind in [1, 4, 5, 7, 8, 200] {
        assert_eq!(
            management::client_message_len(&header(kind, small)).unwrap(),
            Some(management::MAX_MESSAGE_BYTES)
        );
        assert!(matches!(
            management::client_message_len(&header(kind, small + 1)),
            Err(Error::PayloadTooLarge { .. })
        ));
    }
    let visible_payload = management::MAX_VISIBLE_MESSAGE_BYTES - management::HEADER_BYTES;
    assert_eq!(
        management::server_message_len(&header(72, visible_payload)).unwrap(),
        Some(management::MAX_VISIBLE_MESSAGE_BYTES)
    );
    assert!(matches!(
        management::server_message_len(&header(72, visible_payload + 1)),
        Err(Error::PayloadTooLarge { .. })
    ));
    assert!(matches!(
        management::server_message_len(&header(73, small + 1)),
        Err(Error::PayloadTooLarge { .. })
    ));

    // The largest reply: 1 MiB of text beside the largest title and
    // directory.
    let largest = VisibleText {
        title: "t".repeat(orbit_protocol::session::MAX_METADATA_BYTES / 2),
        directory: vec![b'/'; orbit_protocol::session::MAX_METADATA_BYTES / 2],
        text: "a".repeat(management::MAX_VISIBLE_TEXT_BYTES),
        ..visible()
    };
    let bytes =
        management::encode_server_message(&ServerMessage::Visible(largest.clone())).unwrap();
    assert_eq!(bytes.len(), management::MAX_VISIBLE_MESSAGE_BYTES);
    assert_eq!(
        management::decode_server_message(&bytes).unwrap(),
        ServerMessage::Visible(largest.clone())
    );
    let refused = |visible: VisibleText| {
        management::encode_server_message(&ServerMessage::Visible(visible)).unwrap_err()
    };
    assert!(matches!(
        refused(VisibleText {
            text: largest.text.clone() + "a",
            ..largest.clone()
        }),
        Error::ValueTooLarge {
            field: "visible text",
            ..
        }
    ));
    assert!(matches!(
        refused(VisibleText {
            directory: [largest.directory.clone(), b"/".to_vec()].concat(),
            ..largest.clone()
        }),
        Error::ValueTooLarge {
            field: "visible title and directory",
            ..
        }
    ));

    let history_payload = management::MAX_HISTORY_MESSAGE_BYTES - management::HEADER_BYTES;
    assert_eq!(
        management::server_message_len(&header(76, history_payload)).unwrap(),
        Some(management::MAX_HISTORY_MESSAGE_BYTES)
    );
    assert!(matches!(
        management::server_message_len(&header(76, history_payload + 1)),
        Err(Error::PayloadTooLarge { .. })
    ));
    // The largest History reply: 8 MiB of text beside the largest title and
    // directory; one byte more is refused.
    let largest = HistoryText {
        title: "t".repeat(orbit_protocol::session::MAX_METADATA_BYTES / 2),
        directory: vec![b'/'; orbit_protocol::session::MAX_METADATA_BYTES / 2],
        text: "a".repeat(management::MAX_HISTORY_TEXT_BYTES),
        ..history()
    };
    let bytes =
        management::encode_server_message(&ServerMessage::History(largest.clone())).unwrap();
    assert_eq!(bytes.len(), management::MAX_HISTORY_MESSAGE_BYTES);
    assert_eq!(
        management::decode_server_message(&bytes).unwrap(),
        ServerMessage::History(largest.clone())
    );
    let refused = |history: HistoryText| {
        management::encode_server_message(&ServerMessage::History(history)).unwrap_err()
    };
    assert!(matches!(
        refused(HistoryText {
            text: largest.text.clone() + "a",
            ..largest.clone()
        }),
        Error::ValueTooLarge {
            field: "history text",
            ..
        }
    ));
    assert!(matches!(
        refused(HistoryText {
            directory: [largest.directory.clone(), b"/".to_vec()].concat(),
            ..largest.clone()
        }),
        Error::ValueTooLarge {
            field: "history title and directory",
            ..
        }
    ));
}

/// Decoders reject what an encoder never writes: false `lossy`, control
/// characters in visible text, unknown tags, a zero process name, and a paste
/// that is empty or not UTF-8.
#[test]
fn management_v4_decoders_reject_invalid_values() {
    let encoded = |message| management::encode_server_message(&message).unwrap();
    // The lossy byte follows the fixed fields, the title and the directory.
    let plain = visible();
    let lossy_at = management::HEADER_BYTES
        + 8
        + 1
        + 2
        + 2
        + 4
        + plain.title.len()
        + 4
        + plain.directory.len()
        + 4;
    let mut bytes = encoded(ServerMessage::Visible(plain.clone()));
    assert_eq!(bytes[lossy_at], 1);
    bytes[lossy_at] = 0;
    assert!(matches!(
        management::decode_server_message(&bytes),
        Err(Error::InvalidValue { field: "lossy" })
    ));
    assert!(
        management::encode_server_message(&ServerMessage::Visible(VisibleText {
            lossy: false,
            ..plain.clone()
        }))
        .is_err()
    );
    let mut screen = encoded(ServerMessage::Visible(plain.clone()));
    screen[management::HEADER_BYTES + 8] = 3;
    assert!(matches!(
        management::decode_server_message(&screen),
        Err(Error::InvalidTag {
            field: "screen",
            ..
        })
    ));

    // Visible text holds no control character but line feed: the decoder
    // checks bytes an encoder would refuse.
    for control in ['\u{1b}', '\r', '\t', '\0', '\u{7f}', '\u{85}', '\u{9f}'] {
        let text = format!("a{control}b");
        assert!(
            management::encode_server_message(&ServerMessage::Visible(VisibleText {
                text: text.clone(),
                ..plain.clone()
            }))
            .is_err()
        );
        let mut bytes = encoded(ServerMessage::Visible(VisibleText {
            text: "a".repeat(text.len()),
            ..plain.clone()
        }));
        let at = bytes.len() - text.len();
        bytes[at..].copy_from_slice(text.as_bytes());
        assert!(matches!(
            management::decode_server_message(&bytes),
            Err(Error::InvalidValue {
                field: "visible text"
            })
        ));
    }

    // A History reply's lossy byte follows the two counts; its text holds no
    // control character but line feed either.
    let plain = history();
    let lossy_at = management::HEADER_BYTES
        + 8
        + 1
        + 2
        + 2
        + 4
        + plain.title.len()
        + 4
        + plain.directory.len()
        + 4
        + 4;
    let mut bytes = encoded(ServerMessage::History(plain.clone()));
    assert_eq!(bytes[lossy_at], 1);
    bytes[lossy_at] = 0;
    assert!(matches!(
        management::decode_server_message(&bytes),
        Err(Error::InvalidValue { field: "lossy" })
    ));
    for control in ['\u{1b}', '\r', '\t', '\u{85}'] {
        let text = format!("a{control}b");
        assert!(
            management::encode_server_message(&ServerMessage::History(HistoryText {
                text: text.clone(),
                ..plain.clone()
            }))
            .is_err()
        );
        let mut bytes = encoded(ServerMessage::History(HistoryText {
            text: "a".repeat(text.len()),
            ..plain.clone()
        }));
        let at = bytes.len() - text.len();
        bytes[at..].copy_from_slice(text.as_bytes());
        assert!(matches!(
            management::decode_server_message(&bytes),
            Err(Error::InvalidValue {
                field: "history text"
            })
        ));
    }

    let mut bracketed = encoded(ServerMessage::Input { bracketed: true });
    *bracketed.last_mut().unwrap() = 2;
    assert!(matches!(
        management::decode_server_message(&bracketed),
        Err(Error::InvalidTag { .. })
    ));
    let mut refusal = encoded(ServerMessage::Refused(Refusal::InputFull));
    *refusal.last_mut().unwrap() = 8;
    assert!(matches!(
        management::decode_server_message(&refusal),
        Err(Error::InvalidTag {
            field: "refusal",
            ..
        })
    ));

    let client = |message| management::encode_client_message(&message);
    for (process_id, process_start) in [(0, 9), (7, 0)] {
        assert!(matches!(
            client(ClientMessage::AttachAllow {
                process_id,
                process_start,
            }),
            Err(Error::InvalidValue { .. })
        ));
    }
    let mut zero_start = client(ClientMessage::AttachAllow {
        process_id: 7,
        process_start: 9,
    })
    .unwrap();
    let len = zero_start.len();
    zero_start[len - 8..].copy_from_slice(&0u64.to_le_bytes());
    assert!(matches!(
        management::decode_client_message(&zero_start),
        Err(Error::InvalidValue {
            field: "process start identity"
        })
    ));
    for read in [ClientMessage::ReadVisible, ClientMessage::ReadHistory] {
        let mut with_payload = client(read).unwrap();
        with_payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        with_payload.push(0);
        assert!(matches!(
            management::decode_client_message(&with_payload),
            Err(Error::TrailingBytes { count: 1 })
        ));
    }

    let input = |input| ClientMessage::Input { input, quiet_ms: 0 };
    let mut paste = client(input(LeaseInput::Paste("ab".into()))).unwrap();
    let len = paste.len();
    paste[len - 1] = 0xff;
    assert!(matches!(
        management::decode_client_message(&paste),
        Err(Error::InvalidUtf8 {
            field: "paste text"
        })
    ));
    let mut tag = paste.clone();
    tag[management::HEADER_BYTES] = 3;
    assert!(matches!(
        management::decode_client_message(&tag),
        Err(Error::InvalidTag {
            field: "lease input",
            ..
        })
    ));
    let mut empty = client(input(LeaseInput::Enter)).unwrap();
    empty[management::HEADER_BYTES] = 1;
    assert!(matches!(
        management::decode_client_message(&empty),
        Err(Error::InvalidValue {
            field: "paste text"
        })
    ));
    let mut enter_with_text = client(input(LeaseInput::Enter)).unwrap();
    enter_with_text[8..12].copy_from_slice(&4u32.to_le_bytes());
    enter_with_text.push(b'x');
    assert!(matches!(
        management::decode_client_message(&enter_with_text),
        Err(Error::TrailingBytes { count: 1 })
    ));
    let mut no_window = client(input(LeaseInput::Enter)).unwrap();
    no_window.pop();
    no_window[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        management::decode_client_message(&no_window),
        Err(Error::Truncated)
    ));
}
