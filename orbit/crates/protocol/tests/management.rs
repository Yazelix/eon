use orbit_protocol::management::{
    self, ClientMessage, EndpointIdentity, Error, Failure, FailureCode, LiveIdentity,
    ObjectIdentity, ProcessOutcome, Record, ServerMessage, TerminationReason, Tombstone,
};

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

    let mut newer = bytes.clone();
    newer[4..6].copy_from_slice(&(management::VERSION + 1).to_le_bytes());
    assert!(matches!(
        management::decode_client_message(&newer),
        Err(Error::UnsupportedVersion { .. })
    ));

    let mut invalid_tag = bytes;
    invalid_tag[6] = u8::MAX;
    assert!(matches!(
        management::decode_client_message(&invalid_tag),
        Err(Error::InvalidTag { .. })
    ));
}
