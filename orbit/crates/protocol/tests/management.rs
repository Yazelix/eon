use orbit_protocol::management::{
    self, ClientMessage, EndpointIdentity, Error, Failure, FailureCode, LiveIdentity,
    ObjectIdentity, ProcessOutcome, ProgramKind, ProgramRecord, ProgramState, ProgramStatus,
    Record, ServerMessage, TerminationReason, Tombstone,
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
            update: 9,
            omitted: 1,
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

/// Orbit fills a reply by `program_record_len`, so it must match the
/// encoding; at the reply's text bounds seven records fit in 4 KiB.
#[test]
fn program_status_replies_keep_their_bounds() {
    let full = ProgramRecord {
        id: "a".repeat(management::MAX_PROGRAM_ID_BYTES),
        app: "a".repeat(management::MAX_PROGRAM_APP_BYTES),
        title: "a".repeat(management::MAX_PROGRAM_TITLE_BYTES),
        message: "a".repeat(management::MAX_PROGRAM_MESSAGE_BYTES),
        ..program_record("", ProgramState::Blocked)
    };
    let reply = |records: Vec<ProgramRecord>| {
        ServerMessage::ProgramStatus(ProgramStatus {
            count: records.len() as u16,
            update: 1,
            omitted: 0,
            records,
        })
    };
    let seven = management::encode_server_message(&reply(vec![full.clone(); 7])).unwrap();
    assert_eq!(
        seven.len(),
        management::PROGRAM_STATUS_HEADER_BYTES + 7 * management::program_record_len(&full)
    );
    assert!(matches!(
        management::encode_server_message(&reply(vec![full.clone(); 8])),
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
                update: 1,
                omitted: 0,
                records
            }),
            Err(Error::InvalidValue { .. })
        ));
    }
    assert!(matches!(
        refused(ProgramStatus {
            count: 2,
            update: 1,
            omitted: 0,
            records: vec![full]
        }),
        Err(Error::InvalidValue { .. })
    ));
}
