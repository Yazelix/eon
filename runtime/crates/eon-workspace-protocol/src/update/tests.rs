use super::*;

const LIFETIME: &str = "g1-0123456789abcdef0123456789abcdef";
const BUILD: &str = "g1-fedcba9876543210fedcba9876543210";
const REQUEST: RequestId = [0x9f, 0x3c, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0x01];

fn host(number: usize, width: usize) -> Host {
    let id = |prefix: &str| {
        let mut id = format!("{prefix}-{number}");
        id.extend(std::iter::repeat_n('x', width.saturating_sub(id.len())));
        id
    };
    Host {
        terminal: id("terminal"),
        run: id("run"),
        origin: Some(BUILD.into()),
        management: 9,
        record_generation: 24,
        orbs: Some(14),
        orbf: Some(3),
    }
}

fn inspected() -> Inspected {
    Inspected {
        request: REQUEST,
        pid: 1234,
        namespace: b"/run/user/1000/eon".to_vec(),
        lifetime: LIFETIME.into(),
        created_build: BUILD.into(),
        active_build: BUILD.into(),
        active_launcher: None,
        eonw: 47,
        eonr: 1,
        update_schema: 1,
        presentation: Presentation::Attached,
        hosts: vec![Host {
            terminal: "terminal-1".into(),
            run: "run-1".into(),
            ..host(1, 0)
        }],
        last: None,
    }
}

/// The largest INSPECTED v1 allows.
fn largest_inspected() -> Inspected {
    Inspected {
        namespace: [b"/".as_slice(), &[b'n'; MAX_PATH_BYTES - 1]].concat(),
        active_launcher: Some([b"/".as_slice(), &[b'l'; MAX_PATH_BYTES - 1]].concat()),
        hosts: (0..MAX_HOSTS)
            .map(|number| host(number, MAX_ID_BYTES))
            .collect(),
        last: Some(Outcome {
            request: REQUEST,
            operation: Operation::Commit,
            refusal: Some(Code::Internal),
            detail: "d".repeat(MAX_DETAIL_BYTES),
        }),
        ..inspected()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Golden frames: any byte change here is an EONU v1 change, which goes back
// to Lucca, and once a production window binds update.sock, a v2.
#[test]
fn golden_inspect_frame() {
    let bytes = Inspect { request: REQUEST }.encode().unwrap();
    assert_eq!(
        hex(&bytes),
        "454f4e5501000100100000009f3c0203040506070809".to_owned() + "0a0b0c0d0e01"
    );
    assert_eq!(bytes.len(), 28);
    assert_eq!(Inspect::decode(&bytes).unwrap().request, REQUEST);
}

#[test]
fn golden_refused_frame() {
    let refused = Refused {
        request: [0; 16],
        code: Code::Version,
        detail: "this window speaks Update v1".into(),
    };
    let bytes = refused.encode().unwrap();
    assert_eq!(
        hex(&bytes),
        "454f4e5501008200".to_owned()
            + "2f000000"
            + "00000000000000000000000000000000"
            + "02"
            + "1c00"
            + &hex(b"this window speaks Update v1")
    );
    assert_eq!(&bytes[..8], b"EONU\x01\x00\x82\x00");
    assert_eq!(Refused::decode(&bytes, Direction::Reply).unwrap(), refused);
}

#[test]
fn golden_inspected_frame() {
    let bytes = inspected().encode().unwrap();
    let expected = [
        hex(b"EONU\x01\x00\x81\x00"),
        // 221 = 16 + 4 + (2 + 18) + 3 * 35 + 1 + 3 * 2 + 1 + 2 + (2 + 10)
        //     + (2 + 5) + (1 + 35) + 2 * 2 + 3 + 3 + 1
        "dd000000".into(),
        hex(&REQUEST),
        "d2040000".into(),
        "1200".to_owned() + &hex(b"/run/user/1000/eon"),
        hex(LIFETIME.as_bytes()),
        hex(BUILD.as_bytes()),
        hex(BUILD.as_bytes()),
        "00".into(),
        "2f00".into(),
        "01000100".into(),
        "01".into(),
        "0100".into(),
        "0a00".to_owned() + &hex(b"terminal-1"),
        "0500".to_owned() + &hex(b"run-1"),
        "01".to_owned() + &hex(BUILD.as_bytes()),
        "09001800".into(),
        "010e00".into(),
        "010300".into(),
        "00".into(),
    ]
    .concat();
    assert_eq!(hex(&bytes), expected);
    assert_eq!(Inspected::decode(&bytes).unwrap(), inspected());
}

#[test]
fn golden_inspected_frame_with_a_launcher_and_an_outcome() {
    let value = Inspected {
        active_launcher: Some(b"/nix/eon".to_vec()),
        presentation: Presentation::Detached,
        hosts: Vec::new(),
        last: Some(Outcome {
            request: [0xaa; 16],
            operation: Operation::Preflight,
            refusal: Some(Code::Busy),
            detail: "busy".into(),
        }),
        ..inspected()
    };
    let bytes = value.encode().unwrap();
    // 190 = 16 + 4 + (2 + 18) + 3 * 35 + (1 + 2 + 8) + 3 * 2 + 1 + 2
    //     + (1 + 16 + 1 + 1 + 2 + 4)
    let expected = [
        "454f4e5501008100".to_owned(),
        "be000000".into(),
        hex(&REQUEST),
        "d2040000".into(),
        "1200".to_owned() + &hex(b"/run/user/1000/eon"),
        hex(LIFETIME.as_bytes()),
        hex(BUILD.as_bytes()),
        hex(BUILD.as_bytes()),
        "01".to_owned() + "0800" + &hex(b"/nix/eon"),
        "2f00".into(),
        "01000100".into(),
        "00".into(),
        "0000".into(),
        "01".to_owned() + &"aa".repeat(16) + "02" + "06" + "0400" + &hex(b"busy"),
    ]
    .concat();
    assert_eq!(hex(&bytes), expected);
    assert_eq!(Inspected::decode(&bytes).unwrap(), value);
}

#[test]
fn every_frame_round_trips_at_its_smallest_and_largest() {
    let smallest = Inspected {
        hosts: Vec::new(),
        namespace: b"/".to_vec(),
        presentation: Presentation::Detached,
        ..inspected()
    };
    for value in [smallest, inspected(), largest_inspected()] {
        let bytes = value.encode().unwrap();
        assert!(bytes.len() - HEADER_BYTES <= Kind::Inspected.bound());
        assert_eq!(Inspected::decode(&bytes).unwrap(), value);
    }
    // The approved page's estimate: 256 hosts take about 78 KiB.
    let largest = largest_inspected().encode().unwrap().len() - HEADER_BYTES;
    assert!(largest < 96 * 1024, "{largest}");
    for refused in [
        Refused {
            request: REQUEST,
            code: Code::Malformed,
            detail: String::new(),
        },
        Refused {
            request: REQUEST,
            code: Code::Internal,
            detail: "d".repeat(MAX_DETAIL_BYTES),
        },
    ] {
        let bytes = refused.encode().unwrap();
        assert_eq!(Refused::decode(&bytes, Direction::Reply).unwrap(), refused);
        assert_eq!(
            Refused::decode(&bytes, Direction::TargetReply).unwrap(),
            refused
        );
    }
}

#[test]
fn every_truncation_of_every_frame_is_refused() {
    let frames = [
        Inspect { request: REQUEST }.encode().unwrap(),
        inspected().encode().unwrap(),
        largest_inspected().encode().unwrap(),
        Refused {
            request: REQUEST,
            code: Code::Busy,
            detail: "busy".into(),
        }
        .encode()
        .unwrap(),
    ];
    for frame in &frames {
        for cut in 0..frame.len() {
            let short = &frame[..cut];
            let result = match kind_of(frame, Direction::Reply) {
                Ok(Kind::Inspected) => Inspected::decode(short).map(drop),
                Ok(Kind::Refused) => Refused::decode(short, Direction::Reply).map(drop),
                _ => Inspect::decode(short).map(drop),
            };
            assert_eq!(result, Err(Error::Truncated), "cut at {cut}");
        }
    }
}

fn head(version: u16, kind: u8, flags: u8, length: u32) -> [u8; HEADER_BYTES] {
    let mut bytes = [0; HEADER_BYTES];
    bytes[..4].copy_from_slice(b"EONU");
    bytes[4..6].copy_from_slice(&version.to_le_bytes());
    bytes[6] = kind;
    bytes[7] = flags;
    bytes[8..].copy_from_slice(&length.to_le_bytes());
    bytes
}

#[test]
fn the_prefix_alone_decides_the_version() {
    assert_eq!(version(b"EONU\x01\x00"), Ok(1));
    assert_eq!(version(b"EONU\x02\x00"), Ok(2));
    assert_eq!(version(b"EONW\x2f\x00"), Err(Error::Magic));
    for other in [0, 2, 0xffff] {
        assert_eq!(
            header(&head(other, 1, 0, 16), Direction::Request),
            Err(Error::Version(other))
        );
    }
}

#[test]
fn headers_refuse_flags_unknown_kinds_directions_and_oversize_first() {
    assert_eq!(
        header(&head(1, 1, 1, 16), Direction::Request),
        Err(Error::Malformed("flags"))
    );
    let known = [1, 2, 3, 64, 65, 129, 130, 131, 132, 192];
    for kind in 0..=u8::MAX {
        let result = header(&head(1, kind, 0, 0), Direction::Request);
        if !known.contains(&kind) {
            assert_eq!(result, Err(Error::Malformed("kind")), "{kind}");
        } else if kind > 3 {
            assert_eq!(result, Err(Error::Malformed("direction")), "{kind}");
        }
    }
    // A 16 MiB declaration of a kind that never travels to the window is
    // refused by direction, before its bound applies.
    for kind in [64, 65] {
        let declared = (16 * 1024 * 1024) as u32;
        assert_eq!(
            header(&head(1, kind, 0, declared), Direction::Request),
            Err(Error::Malformed("direction"))
        );
    }
    for (kind, bound) in [
        (Kind::Inspect, 16),
        (Kind::Preflight, 8192),
        (Kind::Commit, 8192),
    ] {
        let ok = header(&head(1, kind as u8, 0, bound), Direction::Request).unwrap();
        assert_eq!(
            ok,
            Header {
                kind,
                length: 8192.min(bound as usize)
            }
        );
        assert_eq!(
            header(&head(1, kind as u8, 0, bound + 1), Direction::Request),
            Err(Error::Oversized)
        );
    }
    for (kind, bound) in [(Kind::Inspected, 128 * 1024), (Kind::Refused, 2048)] {
        assert!(header(&head(1, kind as u8, 0, bound), Direction::Reply).is_ok());
        assert_eq!(
            header(&head(1, kind as u8, 0, bound + 1), Direction::Reply),
            Err(Error::Oversized)
        );
    }
    assert_eq!(Kind::TargetCheck.bound(), 16 * 1024 * 1024 + 128 * 1024);
    assert_eq!(Kind::Carry.bound(), 8192);
    for kind in [Kind::Preflighted, Kind::Updated, Kind::TargetVerdict] {
        assert_eq!(kind.bound(), 1024);
    }
}

/// `frame` with byte `at` replaced by `value`.
fn with(frame: &[u8], at: usize, value: u8) -> Vec<u8> {
    let mut changed = frame.to_vec();
    changed[at] = value;
    changed
}

/// `payload` framed as INSPECTED.
fn inspected_frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = head(1, 129, 0, payload.len() as u32).to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn payloads_refuse_every_malformed_field() {
    let good = inspected().encode().unwrap();
    let payload = &good[HEADER_BYTES..];
    // Offsets in the golden INSPECTED payload.
    let namespace = 16 + 4;
    let lifetime = namespace + 2 + 18;
    let launcher = lifetime + 3 * 35;
    let presentation = launcher + 1 + 6;
    let hosts = presentation + 1;
    let last = payload.len() - 1;

    let mut trailing = payload.to_vec();
    trailing.push(0);
    assert_eq!(
        Inspected::decode(&inspected_frame(&trailing)),
        Err(Error::Malformed("trailing bytes"))
    );
    for (at, value, error) in [
        (launcher, 2, "presence tag"),
        (presentation, 2, "presentation"),
        (lifetime, b'G', "identity"),
        (lifetime + 3, b'A', "identity"),
        (namespace + 2, b'r', "path"),
        (namespace + 3, 0, "path"),
        (hosts, 0x02, "host count"),
        (hosts + 1, 0x01, "host count"),
        (last, 2, "presence tag"),
    ] {
        let changed = with(payload, at, value);
        let result = Inspected::decode(&inspected_frame(&changed));
        match error {
            // A larger count runs out of bytes before it can be checked.
            "host count" if at == hosts => assert_eq!(result, Err(Error::Truncated)),
            _ => assert_eq!(result, Err(Error::Malformed(error)), "{at}"),
        }
    }
    // A count over the bound, as a whole frame.
    let mut over = payload[..hosts].to_vec();
    over.extend_from_slice(&257u16.to_le_bytes());
    assert_eq!(
        Inspected::decode(&inspected_frame(&over)),
        Err(Error::Malformed("host count"))
    );
    // An empty namespace path.
    let mut empty = payload[..namespace].to_vec();
    empty.extend_from_slice(&[0, 0]);
    empty.extend_from_slice(&payload[namespace + 2 + 18..]);
    assert_eq!(
        Inspected::decode(&inspected_frame(&empty)),
        Err(Error::Malformed("path"))
    );
    // Invalid UTF-8 in a terminal ID.
    let terminal = hosts + 2 + 2;
    assert_eq!(
        Inspected::decode(&inspected_frame(&with(payload, terminal, 0xff))),
        Err(Error::Malformed("text encoding"))
    );
    // A text over its bound.
    let mut long = payload[..hosts + 2].to_vec();
    long.extend_from_slice(&129u16.to_le_bytes());
    long.extend(std::iter::repeat_n(b'x', 129));
    assert_eq!(
        Inspected::decode(&inspected_frame(&long)),
        Err(Error::Malformed("length"))
    );
    // The wrong kind for the decoder.
    assert_eq!(
        Refused::decode(&good, Direction::Reply),
        Err(Error::Malformed("kind"))
    );
}

#[test]
fn outcome_values_outside_the_approved_sets_are_malformed() {
    let mut value = inspected();
    value.last = Some(Outcome {
        request: REQUEST,
        operation: Operation::Preflight,
        refusal: None,
        detail: String::new(),
    });
    let good = value.encode().unwrap();
    assert_eq!(Inspected::decode(&good).unwrap(), value);
    let payload = &good[HEADER_BYTES..];
    let operation = payload.len() - 2 - 1 - 1;
    for bad in [0, 1, 4, 129, 255] {
        assert_eq!(
            Inspected::decode(&inspected_frame(&with(payload, operation, bad))),
            Err(Error::Malformed("outcome operation"))
        );
    }
    for code in 1..=14 {
        assert!(Inspected::decode(&inspected_frame(&with(payload, operation + 1, code))).is_ok());
    }
    for bad in [15, 128, 255] {
        assert_eq!(
            Inspected::decode(&inspected_frame(&with(payload, operation + 1, bad))),
            Err(Error::Malformed("outcome result"))
        );
    }
    let refused = Refused {
        request: REQUEST,
        code: Code::Busy,
        detail: String::new(),
    }
    .encode()
    .unwrap();
    for bad in [0, 15, 255] {
        assert_eq!(
            Refused::decode(&with(&refused, HEADER_BYTES + 16, bad), Direction::Reply),
            Err(Error::Malformed("refusal code"))
        );
    }
}

#[test]
fn encoders_refuse_what_decoders_would() {
    let mut value = inspected();
    value.lifetime = "g1-NOT-HEX".into();
    assert_eq!(value.encode(), Err(Error::Malformed("identity")));
    let mut value = inspected();
    value.namespace = b"relative".to_vec();
    assert_eq!(value.encode(), Err(Error::Malformed("path")));
    let mut value = inspected();
    value.hosts = (0..=MAX_HOSTS).map(|number| host(number, 0)).collect();
    assert_eq!(value.encode(), Err(Error::Malformed("host count")));
    let refused = Refused {
        request: REQUEST,
        code: Code::Busy,
        detail: "d".repeat(MAX_DETAIL_BYTES + 1),
    };
    assert_eq!(refused.encode(), Err(Error::Malformed("text length")));
}

#[test]
fn codec_names_no_workspace_module() {
    // EONU must never grow a workspace decoder: its source names no
    // versioned module path (`v47::`, `crate::v47`, any version).
    let source = include_str!("../update.rs");
    let bytes = source.as_bytes();
    for (at, _) in source.match_indices('v') {
        let digits = bytes[at + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let after = &source[at + 1 + digits..];
        assert!(
            digits == 0 || !after.starts_with("::"),
            "update.rs names a versioned module at byte {at}"
        );
    }
    assert!(!source.contains("crate::v"));
    assert!(!source.contains("use crate"));
    assert!(!source.contains("super::"), "the codec uses only std");
}

#[test]
fn golden_preflight_and_target_frames() {
    let request = Preflight {
        request: REQUEST,
        lifetime: LIFETIME.into(),
        expected_active_build: BUILD.into(),
        target_wrapper: b"/bin/e".to_vec(),
        target_build: LIFETIME.into(),
        caller: Some("t".into()),
    };
    let reply = Preflighted {
        request: REQUEST,
        caller_origin: Some(BUILD.into()),
        target_build: LIFETIME.into(),
        eonw: 48,
    };
    let check = TargetCheck {
        lifetime: LIFETIME.into(),
        namespace: b"/w".to_vec(),
        created_build: BUILD.into(),
        active_build: BUILD.into(),
        identity_range: 0x0102_0304_0506_0708,
        target_build: LIFETIME.into(),
        hosts: Vec::new(),
        checkpoint: vec![0xa0, 0xb0],
    };
    let verdict = TargetVerdict {
        target_build: LIFETIME.into(),
        eonw: 48,
        update_schema: 1,
    };
    let preflight = [
        b"EONU\x01\0\x02\0\x85\0\0\0".as_slice(),
        &REQUEST,
        LIFETIME.as_bytes(),
        BUILD.as_bytes(),
        b"\x06\0/bin/e",
        LIFETIME.as_bytes(),
        b"\x01\x01\0t",
    ]
    .concat();
    let preflighted = [
        b"EONU\x01\0\x83\0\x59\0\0\0".as_slice(),
        &REQUEST,
        b"\x01",
        BUILD.as_bytes(),
        LIFETIME.as_bytes(),
        b"\x30\0",
    ]
    .concat();
    let target_check = [
        b"EONU\x01\0\x40\0\xa0\0\0\0".as_slice(),
        LIFETIME.as_bytes(),
        b"\x02\0/w",
        BUILD.as_bytes(),
        BUILD.as_bytes(),
        b"\x08\x07\x06\x05\x04\x03\x02\x01",
        LIFETIME.as_bytes(),
        b"\0\0\x02\0\0\0\xa0\xb0",
    ]
    .concat();
    let target_verdict = [
        b"EONU\x01\0\xc0\0\x27\0\0\0".as_slice(),
        LIFETIME.as_bytes(),
        b"\x30\0\x01\0",
    ]
    .concat();
    assert_eq!(request.encode().unwrap(), preflight);
    assert_eq!(reply.encode().unwrap(), preflighted);
    assert_eq!(check.encode().unwrap(), target_check);
    assert_eq!(verdict.encode().unwrap(), target_verdict);
    assert_eq!(Preflight::decode(&preflight), Ok(request));
    assert_eq!(Preflighted::decode(&preflighted), Ok(reply));
    assert_eq!(TargetCheck::decode(&target_check).unwrap(), check);
    assert_eq!(TargetVerdict::decode(&target_verdict), Ok(verdict));

    let with_hosts = TargetCheck {
        hosts: vec![host(1, 0)],
        ..check
    };
    let with_hosts_frame = with_hosts.encode().unwrap();
    assert_eq!(TargetCheck::decode(&with_hosts_frame).unwrap(), with_hosts);
    type Decode = fn(&[u8]) -> Result<()>;
    for (bytes, decode) in [
        (
            preflight.as_slice(),
            (|frame| Preflight::decode(frame).map(drop)) as Decode,
        ),
        (&preflighted, |frame| Preflighted::decode(frame).map(drop)),
        (&with_hosts_frame, |frame| {
            TargetCheck::decode(frame).map(drop)
        }),
        (&target_verdict, |frame| {
            TargetVerdict::decode(frame).map(drop)
        }),
    ] {
        for cut in 0..bytes.len() {
            assert_eq!(decode(&bytes[..cut]), Err(Error::Truncated), "cut at {cut}");
        }
    }

    let mut bad_caller = preflight;
    bad_caller[HEADER_BYTES + 16 + 3 * 35 + 8] = 2;
    assert_eq!(
        Preflight::decode(&bad_caller),
        Err(Error::Malformed("presence tag"))
    );
    assert_eq!(
        Preflighted::decode(&preflighted[..preflighted.len() - 1]),
        Err(Error::Truncated)
    );
    let mut oversized = target_check;
    let length_at = oversized.len() - 6;
    oversized[length_at..length_at + 4].copy_from_slice(&0x0100_0001_u32.to_le_bytes());
    assert_eq!(TargetCheck::decode(&oversized), Err(Error::Oversized));
    let mut trailing = target_verdict;
    trailing[8] += 1;
    trailing.push(0);
    assert_eq!(
        TargetVerdict::decode(&trailing),
        Err(Error::Malformed("trailing bytes"))
    );
}

#[test]
fn golden_commit_carry_and_updated() {
    let request = Preflight {
        request: REQUEST,
        lifetime: LIFETIME.into(),
        expected_active_build: BUILD.into(),
        target_wrapper: b"/bin/e".to_vec(),
        target_build: LIFETIME.into(),
        caller: Some("t".into()),
    };
    let carry = Carry {
        request: REQUEST,
        lifetime: LIFETIME.into(),
        from_build: BUILD.into(),
        target_build: LIFETIME.into(),
        target_wrapper: b"/bin/e".to_vec(),
        caller: Some("t".into()),
        caller_origin: Some(BUILD.into()),
        reply_sent: false,
    };
    let updated = Updated {
        request: REQUEST,
        lifetime: LIFETIME.into(),
        from_build: BUILD.into(),
        to_build: LIFETIME.into(),
        changed: true,
        presentation: UpdatedPresentation::Detached,
    };
    let commit_bytes = [
        b"EONU\x01\0\x03\0\x85\0\0\0".as_slice(),
        &REQUEST,
        LIFETIME.as_bytes(),
        BUILD.as_bytes(),
        b"\x06\0/bin/e",
        LIFETIME.as_bytes(),
        b"\x01\x01\0t",
    ]
    .concat();
    let carry_bytes = [
        b"EONU\x01\0\x41\0\xab\0\0\0".as_slice(),
        &REQUEST,
        b"\x03",
        LIFETIME.as_bytes(),
        BUILD.as_bytes(),
        LIFETIME.as_bytes(),
        b"\x06\0/bin/e\x01\x01\0t\x01",
        BUILD.as_bytes(),
        b"\0",
    ]
    .concat();
    let updated_bytes = [
        b"EONU\x01\0\x84\0\x7b\0\0\0".as_slice(),
        &REQUEST,
        LIFETIME.as_bytes(),
        BUILD.as_bytes(),
        LIFETIME.as_bytes(),
        b"\x01\0",
    ]
    .concat();
    assert_eq!(request.encode_commit().unwrap(), commit_bytes);
    assert_eq!(carry.encode().unwrap(), carry_bytes);
    assert_eq!(updated.encode().unwrap(), updated_bytes);
    assert_eq!(Preflight::decode_commit(&commit_bytes), Ok(request));
    assert_eq!(Carry::decode(&carry_bytes).as_ref(), Ok(&carry));
    assert_eq!(Updated::decode(&updated_bytes), Ok(updated));
    let progress_offset = carry_bytes.len() - 1;
    let sent = Carry {
        reply_sent: true,
        ..carry
    };
    let sent_bytes = with(&carry_bytes, progress_offset, 1);
    assert_eq!(sent.encode().unwrap(), sent_bytes);
    assert_eq!(Carry::decode(&sent_bytes), Ok(sent));
    assert!(Carry::decode(&with(&carry_bytes, progress_offset, 2)).is_err());

    assert!(Preflight::decode(&commit_bytes).is_err());
    assert!(kind_of(&carry_bytes, Direction::Request).is_err());
    assert!(Carry::decode(&with(&carry_bytes, HEADER_BYTES + 16, 2)).is_err());
    for size in 0..carry_bytes.len() {
        assert!(Carry::decode(&carry_bytes[..size]).is_err());
    }
    for offset in [updated_bytes.len() - 2, updated_bytes.len() - 1] {
        assert!(Updated::decode(&with(&updated_bytes, offset, 2)).is_err());
    }
}
