use super::codec::{
    CLIENT_HELLO, CLIENT_PASTE, SERVER_BUSY, SERVER_CLIPBOARD_WRITE, SERVER_COPIED_TEXT,
    SERVER_FAILURE, SERVER_FRAME, SERVER_SCROLL_VIEWPORT, mouse_action_tag,
};
use super::*;
use crate::{
    Capabilities, Cell, CellStyle, CellWidth, Colors, Cursor, CursorShape, Dimensions, Frame,
    MAX_FRAME_BYTES, MIN_FRAME_BYTES, PALETTE_LEN, Rgb, Row, Screen, StyleColor, Underline,
};

fn frame() -> Frame {
    Frame {
        revision: 7,
        dimensions: Dimensions { cols: 1, rows: 1 },
        screen: Screen::Primary,
        scroll_position: crate::ScrollPosition {
            rows_from_live: 3,
            history_rows: 9,
        },
        title: "session".into(),
        working_directory: "/tmp".into(),
        capabilities: Capabilities {
            hyperlinks: true,
            kitty_graphics: false,
        },
        colors: Colors {
            background: Rgb::BLACK,
            foreground: Rgb {
                r: 240,
                g: 240,
                b: 240,
            },
            cursor: None,
            palette: [Rgb::BLACK; PALETTE_LEN],
        },
        cursor: Cursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            viewport: None,
        },
        rows: vec![Row {
            wrapped: false,
            wrap_continuation: false,
            kitty_virtual_placeholder: false,
            cells: vec![Cell {
                width: CellWidth::Narrow,
                style: CellStyle {
                    foreground: StyleColor::None,
                    background: StyleColor::None,
                    underline_color: StyleColor::None,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    inverse: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                    selected: false,
                    protected: false,
                    underline: Underline::None,
                },
                text: "界".into(),
                hyperlink: "https://example.test".into(),
            }],
        }],
    }
}

fn header(kind: u8, payload: usize) -> [u8; HEADER_BYTES] {
    let mut header = [0; HEADER_BYTES];
    header[..4].copy_from_slice(MAGIC);
    header[4..6].copy_from_slice(&VERSION.to_le_bytes());
    header[6] = kind;
    header[8..12].copy_from_slice(&(payload as u32).to_le_bytes());
    header
}

#[test]
fn every_client_message_round_trips() {
    let messages = [
        ClientMessage::Hello,
        ClientMessage::ObserveMetadata,
        ClientMessage::ReturnToLive,
        ClientMessage::Key(KeyEvent {
            action: KeyAction::Repeat,
            key: PhysicalKey::A,
            modifiers: Modifiers::CTRL
                .union(Modifiers::SHIFT)
                .union(Modifiers::SHIFT_SIDE),
            consumed_modifiers: Modifiers::SHIFT,
            composing: true,
            text: Some("e\u{301}".into()),
            unshifted_codepoint: Some('e'),
        }),
        ClientMessage::Mouse(MouseEvent {
            action: MouseAction::Motion,
            button: Some(MouseButton::Eleven),
            modifiers: Modifiers::ALT,
            x: 12.5,
            y: 4.25,
        }),
        ClientMessage::Focus(FocusEvent::Gained),
        ClientMessage::Paste(b"first\nsecond\0\xff".to_vec()),
        ClientMessage::Resize(SurfaceSize {
            cols: 100,
            rows: 40,
            screen_width: 920,
            screen_height: 740,
            cell_width: 9,
            cell_height: 18,
            padding_top: 10,
            padding_bottom: 10,
            padding_left: 10,
            padding_right: 10,
        }),
        ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 42,
            position: SelectionPosition { x: 31.5, y: 72.25 },
            time_ns: 1_000_000_000,
            modifiers: Modifiers::SHIFT,
        }),
        ClientMessage::Selection(SelectionAction::Update {
            position: SelectionPosition { x: 49.5, y: 108.25 },
            modifiers: Modifiers::CTRL,
        }),
        ClientMessage::Selection(SelectionAction::Finish {
            position: SelectionPosition { x: 67.5, y: 144.25 },
            modifiers: Modifiers::ALT,
        }),
        ClientMessage::Selection(SelectionAction::AutoscrollUp {
            position: SelectionPosition { x: 12.5, y: 0.25 },
        }),
        ClientMessage::Selection(SelectionAction::Cancel),
        ClientMessage::Selection(SelectionAction::Copy),
    ];

    for message in messages {
        let encoded = encode_client_message(&message).unwrap();
        assert_eq!(client_message_len(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(decode_client_message(&encoded).unwrap(), message);
    }
}

#[test]
fn every_server_message_round_trips() {
    let messages = [
        ServerMessage::Attached,
        ServerMessage::ObservingMetadata,
        ServerMessage::Metadata(Metadata {
            revision: 8,
            title: "Codex ◐".into(),
            working_directory: "file:///tmp/eon session".into(),
        }),
        ServerMessage::Busy,
        ServerMessage::Frame(Box::new(frame())),
        ServerMessage::Accepted,
        ServerMessage::SelectionFinished { frame_revision: 9 },
        ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "bad key".into(),
        }),
        ServerMessage::CopiedText {
            location: ClipboardLocation::Selection,
            text: "first\n界e\u{301}".into(),
        },
        ServerMessage::CopiedText {
            location: ClipboardLocation::Standard,
            text: String::new(),
        },
        ServerMessage::ClipboardWrite {
            location: ClipboardLocation::Primary,
            text: "copied by zellij".into(),
        },
        ServerMessage::Exited { code: 17 },
    ];

    for message in messages {
        let encoded = encode_server_message(&message).unwrap();
        assert_eq!(server_message_len(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(decode_server_message(&encoded).unwrap(), message);
    }
}

#[test]
fn every_mouse_button_round_trips() {
    for button in [
        MouseButton::Unknown,
        MouseButton::Left,
        MouseButton::Middle,
        MouseButton::Right,
        MouseButton::Four,
        MouseButton::Five,
        MouseButton::Six,
        MouseButton::Seven,
        MouseButton::Eight,
        MouseButton::Nine,
        MouseButton::Ten,
        MouseButton::Eleven,
    ] {
        let message = ClientMessage::Mouse(MouseEvent {
            action: MouseAction::Press,
            button: Some(button),
            modifiers: Modifiers::empty(),
            x: 1.0,
            y: 2.0,
        });
        assert_eq!(
            decode_client_message(&encode_client_message(&message).unwrap()).unwrap(),
            message
        );
    }

    let invalid = ClientMessage::Mouse(MouseEvent {
        action: MouseAction::Motion,
        button: None,
        modifiers: Modifiers::SHIFT_SIDE,
        x: 1.0,
        y: 2.0,
    });
    assert_eq!(
        encode_client_message(&invalid),
        Err(Error::InvalidValue { field: "modifiers" })
    );
}

#[test]
fn mouse_action_button_combinations_are_canonical() {
    let mouse = |action, button| {
        ClientMessage::Mouse(MouseEvent {
            action,
            button,
            modifiers: Modifiers::empty(),
            x: 1.0,
            y: 2.0,
        })
    };

    for action in [MouseAction::Press, MouseAction::Release] {
        assert_eq!(
            encode_client_message(&mouse(action, None)),
            Err(Error::InvalidValue {
                field: "mouse button"
            })
        );
    }

    let mut invalid = encode_client_message(&mouse(MouseAction::Motion, None)).unwrap();
    invalid[HEADER_BYTES] = mouse_action_tag(MouseAction::Press);
    assert_eq!(
        decode_client_message(&invalid),
        Err(Error::InvalidValue {
            field: "mouse button"
        })
    );

    for action in [MouseAction::Release, MouseAction::Motion] {
        assert_eq!(
            encode_client_message(&mouse(action, Some(MouseButton::Four))),
            Err(Error::InvalidValue {
                field: "mouse action"
            })
        );

        let mut invalid =
            encode_client_message(&mouse(MouseAction::Press, Some(MouseButton::Four))).unwrap();
        invalid[HEADER_BYTES] = mouse_action_tag(action);
        assert_eq!(
            decode_client_message(&invalid),
            Err(Error::InvalidValue {
                field: "mouse action"
            })
        );
    }
}

#[test]
fn framing_is_incremental_strict_and_bounded() {
    let encoded = encode_client_message(&ClientMessage::Hello).unwrap();
    for end in 0..HEADER_BYTES {
        assert_eq!(client_message_len(&encoded[..end]).unwrap(), None);
    }
    for end in 0..encoded.len() {
        assert_eq!(
            decode_client_message(&encoded[..end]),
            Err(Error::Truncated)
        );
    }

    let mut corrupt = encoded.clone();
    corrupt[4..6].copy_from_slice(&12_u16.to_le_bytes());
    assert_eq!(
        client_message_len(&corrupt),
        Err(Error::UnsupportedVersion { version: 12 })
    );
    corrupt = encoded.clone();
    corrupt[0] ^= 1;
    assert_eq!(client_message_len(&corrupt), Err(Error::InvalidMagic));
    corrupt = encoded.clone();
    corrupt[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
    assert_eq!(
        client_message_len(&corrupt),
        Err(Error::UnsupportedVersion {
            version: VERSION + 1,
        })
    );
    corrupt = encoded.clone();
    corrupt[7] = 1;
    assert_eq!(
        client_message_len(&corrupt),
        Err(Error::InvalidFlags { value: 1 })
    );
    let mut trailing = encoded;
    trailing.push(0);
    assert_eq!(
        decode_client_message(&trailing),
        Err(Error::TrailingBytes { count: 1 })
    );
}

#[test]
fn declared_payloads_are_validated_from_the_header() {
    assert_eq!(
        client_message_len(&header(u8::MAX, 0)),
        Err(Error::InvalidTag {
            field: "client message",
            value: u8::MAX,
        })
    );
    assert_eq!(
        client_message_len(&header(CLIENT_PASTE, MAX_PASTE_BYTES + 1)),
        Err(Error::PayloadTooLarge {
            size: MAX_PASTE_BYTES + 1,
            maximum: MAX_PASTE_BYTES,
        })
    );
    assert_eq!(
        client_message_len(&header(CLIENT_HELLO, 1)),
        Err(Error::PayloadTooLarge {
            size: 1,
            maximum: 0,
        })
    );
    assert_eq!(
        client_message_len(&header(SERVER_BUSY, 0)),
        Err(Error::InvalidTag {
            field: "client message",
            value: SERVER_BUSY,
        })
    );
    assert_eq!(
        server_message_len(&header(CLIENT_HELLO, 0)),
        Err(Error::InvalidTag {
            field: "server message",
            value: CLIENT_HELLO,
        })
    );
    assert_eq!(
        server_message_len(&header(SERVER_FAILURE, MAX_FAILURE_BYTES + 6)),
        Err(Error::PayloadTooLarge {
            size: MAX_FAILURE_BYTES + 6,
            maximum: MAX_FAILURE_BYTES + 5,
        })
    );
    let mut minimum_frame = frame();
    minimum_frame.title.clear();
    minimum_frame.working_directory.clear();
    minimum_frame.rows[0].cells[0].text.clear();
    minimum_frame.rows[0].cells[0].hyperlink.clear();
    let message = ServerMessage::Frame(Box::new(minimum_frame));
    let encoded = encode_server_message(&message).unwrap();
    assert_eq!(encoded.len(), HEADER_BYTES + MIN_FRAME_BYTES);
    assert_eq!(decode_server_message(&encoded), Ok(message));

    assert_eq!(
        server_message_len(&header(SERVER_FRAME, MIN_FRAME_BYTES - 1)),
        Err(Error::InvalidValue {
            field: "message payload length",
        })
    );
    assert_eq!(
        server_message_len(&header(SERVER_FRAME, MAX_FRAME_BYTES + 1)),
        Err(Error::PayloadTooLarge {
            size: MAX_FRAME_BYTES + 1,
            maximum: MAX_FRAME_BYTES,
        })
    );
    assert_eq!(
        server_message_len(&header(SERVER_SCROLL_VIEWPORT, MAX_PAYLOAD_BYTES + 1)),
        Err(Error::PayloadTooLarge {
            size: MAX_PAYLOAD_BYTES + 1,
            maximum: MAX_PAYLOAD_BYTES,
        })
    );
}

#[test]
fn semantic_values_reject_invalid_states() {
    assert!(PhysicalKey::from_raw(PhysicalKey::MAX_RAW).is_some());
    assert!(PhysicalKey::from_raw(PhysicalKey::MAX_RAW + 1).is_none());
    assert!(Modifiers::from_bits(Modifiers::MASK).is_some());
    assert!(Modifiers::from_bits(Modifiers::MASK + 1).is_none());
    assert!(Modifiers::from_bits(Modifiers::SHIFT_SIDE.bits()).is_none());

    let oversized = ClientMessage::Paste(vec![0; MAX_PASTE_BYTES + 1]);
    assert_eq!(
        encode_client_message(&oversized),
        Err(Error::PayloadTooLarge {
            size: MAX_PASTE_BYTES + 1,
            maximum: MAX_PASTE_BYTES,
        })
    );

    let oversized = ServerMessage::CopiedText {
        location: ClipboardLocation::Standard,
        text: "x".repeat(MAX_COPY_BYTES + 1),
    };
    assert_eq!(
        encode_server_message(&oversized),
        Err(Error::PayloadTooLarge {
            size: MAX_COPY_BYTES + 1,
            maximum: MAX_COPY_BYTES,
        })
    );

    let oversized = ServerMessage::Metadata(Metadata {
        revision: 1,
        title: "x".repeat(MAX_METADATA_BYTES),
        working_directory: String::new(),
    });
    assert!(matches!(
        encode_server_message(&oversized),
        Err(Error::PayloadTooLarge {
            maximum: MAX_METADATA_BYTES,
            ..
        })
    ));

    for invalid in [
        ServerMessage::ClipboardWrite {
            location: ClipboardLocation::Standard,
            text: String::new(),
        },
        ServerMessage::ClipboardWrite {
            location: ClipboardLocation::Selection,
            text: "contains\0nul".into(),
        },
    ] {
        assert_eq!(
            encode_server_message(&invalid),
            Err(Error::InvalidValue {
                field: "clipboard text",
            })
        );
    }

    let oversized = ServerMessage::ClipboardWrite {
        location: ClipboardLocation::Primary,
        text: "x".repeat(MAX_COPY_BYTES + 1),
    };
    assert_eq!(
        encode_server_message(&oversized),
        Err(Error::PayloadTooLarge {
            size: MAX_COPY_BYTES + 1,
            maximum: MAX_COPY_BYTES,
        })
    );

    let invalid = ClientMessage::Key(KeyEvent {
        action: KeyAction::Press,
        key: PhysicalKey::A,
        modifiers: Modifiers::empty(),
        consumed_modifiers: Modifiers::CTRL,
        composing: false,
        text: Some("a".into()),
        unshifted_codepoint: Some('a'),
    });
    assert_eq!(
        encode_client_message(&invalid),
        Err(Error::ConsumedModifiersNotActive)
    );

    let invalid = ClientMessage::Key(KeyEvent {
        action: KeyAction::Press,
        key: PhysicalKey::A,
        modifiers: Modifiers::SHIFT_SIDE,
        consumed_modifiers: Modifiers::empty(),
        composing: false,
        text: Some("A".into()),
        unshifted_codepoint: Some('a'),
    });
    assert_eq!(
        encode_client_message(&invalid),
        Err(Error::InvalidValue { field: "modifiers" })
    );

    for text in ["\u{1b}", "\u{7f}", "\u{f700}"] {
        let invalid = ClientMessage::Key(KeyEvent {
            action: KeyAction::Press,
            key: PhysicalKey::UNIDENTIFIED,
            modifiers: Modifiers::empty(),
            consumed_modifiers: Modifiers::empty(),
            composing: false,
            text: Some(text.into()),
            unshifted_codepoint: None,
        });
        assert_eq!(
            encode_client_message(&invalid),
            Err(Error::InvalidValue { field: "key text" })
        );
    }

    let invalid = ClientMessage::Mouse(MouseEvent {
        action: MouseAction::Motion,
        button: None,
        modifiers: Modifiers::empty(),
        x: f32::NAN,
        y: 0.0,
    });
    assert_eq!(
        encode_client_message(&invalid),
        Err(Error::InvalidCoordinates)
    );

    let mut mouse = MouseEvent {
        action: MouseAction::Motion,
        button: None,
        modifiers: Modifiers::empty(),
        x: f32::from(u16::MAX),
        y: f32::from(u16::MAX),
    };
    assert!(encode_client_message(&ClientMessage::Mouse(mouse)).is_ok());
    mouse.x += 1.0;
    assert_eq!(
        encode_client_message(&ClientMessage::Mouse(mouse)),
        Err(Error::InvalidCoordinates)
    );

    let mut size = SurfaceSize {
        cols: 1,
        rows: 1,
        screen_width: u32::from(u16::MAX),
        screen_height: 1,
        cell_width: 1,
        cell_height: 1,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    };
    assert!(encode_client_message(&ClientMessage::Resize(size)).is_ok());
    size.screen_width += 1;
    assert_eq!(
        encode_client_message(&ClientMessage::Resize(size)),
        Err(Error::InvalidSurfaceSize)
    );

    let invalid = ClientMessage::Resize(SurfaceSize {
        cols: 100,
        rows: 40,
        screen_width: 899,
        screen_height: 720,
        cell_width: 9,
        cell_height: 18,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    });
    assert_eq!(
        encode_client_message(&invalid),
        Err(Error::InvalidSurfaceSize)
    );
}

#[test]
fn malformed_typed_payloads_are_rejected() {
    let key = ClientMessage::Key(KeyEvent {
        action: KeyAction::Press,
        key: PhysicalKey::A,
        modifiers: Modifiers::SHIFT,
        consumed_modifiers: Modifiers::empty(),
        composing: false,
        text: Some("a".into()),
        unshifted_codepoint: Some('a'),
    });
    let encoded = encode_client_message(&key).unwrap();

    let mut corrupt = encoded.clone();
    corrupt[6] = u8::MAX;
    assert!(matches!(
        decode_client_message(&corrupt),
        Err(Error::InvalidTag {
            field: "client message",
            ..
        })
    ));

    let mut corrupt = encoded.clone();
    corrupt[15..17].copy_from_slice(&(Modifiers::MASK + 1).to_le_bytes());
    assert_eq!(
        decode_client_message(&corrupt),
        Err(Error::InvalidValue { field: "modifiers" })
    );

    let mut corrupt = encoded.clone();
    corrupt[24] = b'\x1b';
    assert_eq!(
        decode_client_message(&corrupt),
        Err(Error::InvalidValue { field: "key text" })
    );

    let mut corrupt = encoded.clone();
    corrupt[24] = 0xff;
    assert_eq!(
        decode_client_message(&corrupt),
        Err(Error::InvalidUtf8 { field: "key text" })
    );

    let mut corrupt = encoded;
    corrupt[25..29].copy_from_slice(&0xd800_u32.to_le_bytes());
    assert_eq!(
        decode_client_message(&corrupt),
        Err(Error::InvalidValue {
            field: "unshifted codepoint"
        })
    );

    let mut corrupt = encode_client_message(&ClientMessage::Resize(SurfaceSize {
        cols: 1,
        rows: 1,
        screen_width: 8,
        screen_height: 16,
        cell_width: 8,
        cell_height: 16,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    }))
    .unwrap();
    corrupt[12..14].fill(0);
    assert_eq!(
        decode_client_message(&corrupt),
        Err(Error::InvalidSurfaceSize)
    );

    let server = encode_server_message(&ServerMessage::Busy).unwrap();
    assert!(matches!(
        decode_client_message(&server),
        Err(Error::InvalidTag {
            field: "client message",
            ..
        })
    ));

    let mut selection = encode_client_message(&ClientMessage::Selection(SelectionAction::Begin {
        frame_revision: 1,
        position: SelectionPosition { x: 0.0, y: 0.0 },
        time_ns: 0,
        modifiers: Modifiers::empty(),
    }))
    .unwrap();
    selection[HEADER_BYTES] = u8::MAX;
    assert_eq!(
        decode_client_message(&selection),
        Err(Error::InvalidTag {
            field: "selection action",
            value: u8::MAX,
        })
    );

    let invalid_selection = ClientMessage::Selection(SelectionAction::Update {
        position: SelectionPosition {
            x: f32::NAN,
            y: 0.0,
        },
        modifiers: Modifiers::empty(),
    });
    assert_eq!(
        encode_client_message(&invalid_selection),
        Err(Error::InvalidCoordinates)
    );

    assert_eq!(
        encode_client_message(&ClientMessage::Selection(SelectionAction::AutoscrollUp {
            position: SelectionPosition {
                x: f32::NAN,
                y: 0.0,
            },
        })),
        Err(Error::InvalidCoordinates)
    );

    let mut invalid_selection =
        encode_client_message(&ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 1,
            position: SelectionPosition { x: 0.0, y: 0.0 },
            time_ns: 0,
            modifiers: Modifiers::empty(),
        }))
        .unwrap();
    invalid_selection[HEADER_BYTES + 19..HEADER_BYTES + 23]
        .copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert_eq!(
        decode_client_message(&invalid_selection),
        Err(Error::InvalidCoordinates)
    );

    let mut invalid_modifiers =
        encode_client_message(&ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 1,
            position: SelectionPosition { x: 0.0, y: 0.0 },
            time_ns: 0,
            modifiers: Modifiers::empty(),
        }))
        .unwrap();
    invalid_modifiers[HEADER_BYTES + 17..HEADER_BYTES + 19]
        .copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        decode_client_message(&invalid_modifiers),
        Err(Error::InvalidValue { field: "modifiers" })
    );

    let mut copied = encode_server_message(&ServerMessage::CopiedText {
        location: ClipboardLocation::Selection,
        text: "text".into(),
    })
    .unwrap();
    let mut invalid_copied_location = copied.clone();
    invalid_copied_location[HEADER_BYTES] = u8::MAX;
    assert_eq!(
        decode_server_message(&invalid_copied_location),
        Err(Error::InvalidTag {
            field: "clipboard location",
            value: u8::MAX,
        })
    );
    copied[HEADER_BYTES + 1] = 0xff;
    assert_eq!(
        decode_server_message(&copied),
        Err(Error::InvalidUtf8 {
            field: "copied text"
        })
    );

    let mut metadata = encode_server_message(&ServerMessage::Metadata(Metadata {
        revision: 1,
        title: "text".into(),
        working_directory: "file:///tmp".into(),
    }))
    .unwrap();
    metadata[HEADER_BYTES + 12] = 0xff;
    assert_eq!(
        decode_server_message(&metadata),
        Err(Error::InvalidUtf8 {
            field: "metadata title",
        })
    );

    let clipboard = ServerMessage::ClipboardWrite {
        location: ClipboardLocation::Standard,
        text: "text".into(),
    };
    let mut invalid = encode_server_message(&clipboard).unwrap();
    invalid[HEADER_BYTES] = u8::MAX;
    assert_eq!(
        decode_server_message(&invalid),
        Err(Error::InvalidTag {
            field: "clipboard location",
            value: u8::MAX,
        })
    );

    let mut invalid = encode_server_message(&clipboard).unwrap();
    invalid[HEADER_BYTES + 1] = 0xff;
    assert_eq!(
        decode_server_message(&invalid),
        Err(Error::InvalidUtf8 {
            field: "clipboard text",
        })
    );

    assert_eq!(
        server_message_len(&header(SERVER_COPIED_TEXT, MAX_COPY_BYTES + 2)),
        Err(Error::PayloadTooLarge {
            size: MAX_COPY_BYTES + 2,
            maximum: MAX_COPY_BYTES + 1,
        })
    );
    assert_eq!(
        server_message_len(&header(SERVER_CLIPBOARD_WRITE, MAX_COPY_BYTES + 2)),
        Err(Error::PayloadTooLarge {
            size: MAX_COPY_BYTES + 2,
            maximum: MAX_COPY_BYTES + 1,
        })
    );
}

#[test]
fn vertical_preview_and_wheel_outcomes_are_canonical() {
    let request = ClientMessage::PreviewVertical {
        frame_revision: 7,
        direction: VerticalDirection::Up,
    };
    assert_eq!(
        decode_client_message(&encode_client_message(&request).unwrap()).unwrap(),
        request
    );
    let mut invalid_direction = encode_client_message(&request).unwrap();
    invalid_direction[HEADER_BYTES + 8] = u8::MAX;
    assert_eq!(
        decode_client_message(&invalid_direction),
        Err(Error::InvalidTag {
            field: "vertical direction",
            value: u8::MAX,
        })
    );

    let row = frame().rows.remove(0);
    let mut older = row.clone();
    older.cells[0].text = "older".into();
    for message in [
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::TerminalRouted,
        }),
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Down,
            outcome: PreviewOutcome::Viewport {
                cols: 1,
                edge_reached: false,
                rows: vec![row.clone(), older.clone()],
            },
        }),
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Down,
            outcome: PreviewOutcome::Viewport {
                cols: 1,
                edge_reached: true,
                rows: Vec::new(),
            },
        }),
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 1,
                edge_reached: true,
                rows: vec![older.clone()],
            },
        }),
        ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted),
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: -1,
            frame: Box::new(frame()),
        }),
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: 0,
            frame: Box::new(frame()),
        }),
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: 1,
            frame: Box::new(frame()),
        }),
    ] {
        let encoded = encode_server_message(&message).unwrap();
        assert_eq!(server_message_len(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(decode_server_message(&encoded).unwrap(), message);
    }

    for invalid in [
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 1,
                edge_reached: false,
                rows: Vec::new(),
            },
        }),
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 0,
                edge_reached: true,
                rows: Vec::new(),
            },
        }),
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 2,
                edge_reached: false,
                rows: vec![row],
            },
        }),
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: 2,
            frame: Box::new(frame()),
        }),
    ] {
        assert!(encode_server_message(&invalid).is_err());
    }

    let mut oversized = frame().rows.remove(0);
    oversized.cells[0].text = "x".repeat(MAX_FRAME_BYTES);
    assert!(matches!(
        encode_server_message(&ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 1,
                edge_reached: false,
                rows: vec![oversized],
            },
        })),
        Err(Error::Frame(crate::Error::FrameTooLarge { .. }))
    ));
    let mut maximum_row = frame().rows.remove(0);
    let remaining = MAX_FRAME_BYTES - crate::encode_canonical_row(&maximum_row, 1).unwrap().len();
    maximum_row.cells[0].text.push_str(&"x".repeat(remaining));
    let maximum = encode_server_message(&ServerMessage::VerticalPreview(VerticalPreview {
        frame_revision: 7,
        direction: VerticalDirection::Up,
        outcome: PreviewOutcome::Viewport {
            cols: 1,
            edge_reached: false,
            rows: vec![maximum_row],
        },
    }))
    .unwrap();
    assert_eq!(maximum.len(), HEADER_BYTES + MAX_FRAME_BYTES + 15);

    let preview = ServerMessage::VerticalPreview(VerticalPreview {
        frame_revision: 7,
        direction: VerticalDirection::Down,
        outcome: PreviewOutcome::Viewport {
            cols: 1,
            edge_reached: false,
            rows: vec![frame().rows.remove(0)],
        },
    });
    let encoded = encode_server_message(&preview).unwrap();
    let mut inconsistent = encoded.clone();
    inconsistent[HEADER_BYTES + 10..HEADER_BYTES + 12].copy_from_slice(&2_u16.to_le_bytes());
    assert!(matches!(
        decode_server_message(&inconsistent),
        Err(Error::Frame(crate::Error::Truncated))
    ));
    let mut invalid_edge = encoded.clone();
    invalid_edge[HEADER_BYTES + 12] = 2;
    assert_eq!(
        decode_server_message(&invalid_edge),
        Err(Error::InvalidTag {
            field: "preview edge",
            value: 2,
        })
    );
    let mut invalid_count = encoded.clone();
    invalid_count[HEADER_BYTES + 13..HEADER_BYTES + 15].copy_from_slice(&2_u16.to_le_bytes());
    assert!(matches!(
        decode_server_message(&invalid_count),
        Err(Error::Frame(crate::Error::Truncated))
    ));
    let mut invalid_cells = encoded.clone();
    invalid_cells[HEADER_BYTES + 10..HEADER_BYTES + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    invalid_cells[HEADER_BYTES + 13..HEADER_BYTES + 15].copy_from_slice(&2_u16.to_le_bytes());
    assert_eq!(
        decode_server_message(&invalid_cells),
        Err(Error::InvalidValue {
            field: "preview cell count",
        })
    );
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(
        decode_server_message(&trailing),
        Err(Error::TrailingBytes { count: 1 })
    );
    assert_eq!(
        decode_server_message(&encoded[..encoded.len() - 1]),
        Err(Error::Truncated)
    );
}

#[test]
fn vertical_scroll_batches_are_bounded_and_canonical() {
    for rows in [-MAX_SCROLL_ROWS, -1, 1, MAX_SCROLL_ROWS] {
        let request = ClientMessage::ScrollVertical {
            frame_revision: 7,
            rows,
        };
        assert_eq!(
            decode_client_message(&encode_client_message(&request).unwrap()).unwrap(),
            request
        );
    }
    for rows in [0, -MAX_SCROLL_ROWS - 1, MAX_SCROLL_ROWS + 1] {
        assert!(
            encode_client_message(&ClientMessage::ScrollVertical {
                frame_revision: 7,
                rows,
            })
            .is_err()
        );
    }

    let terminal_owned = ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned {
        requested_rows: -MAX_SCROLL_ROWS,
    });
    let viewport = ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
        requested_rows: -MAX_SCROLL_ROWS,
        applied_rows: -3,
        frame: Box::new(frame()),
        next: PreviewOutcome::Viewport {
            cols: 1,
            edge_reached: false,
            rows: vec![frame().rows.remove(0)],
        },
    });
    for message in [terminal_owned, viewport.clone()] {
        let encoded = encode_server_message(&message).unwrap();
        assert_eq!(server_message_len(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(decode_server_message(&encoded).unwrap(), message);
    }
    let mut invalid = encode_server_message(&viewport).unwrap();
    invalid[HEADER_BYTES + 2..HEADER_BYTES + 4].copy_from_slice(&1_i16.to_le_bytes());
    assert_eq!(
        decode_server_message(&invalid),
        Err(Error::InvalidValue {
            field: "applied scroll rows",
        })
    );

    for (requested_rows, applied_rows) in [(0, 0), (2, 3), (2, -1), (-2, 1)] {
        assert!(
            encode_server_message(&ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                requested_rows,
                applied_rows,
                frame: Box::new(frame()),
                next: PreviewOutcome::Viewport {
                    cols: 1,
                    edge_reached: true,
                    rows: Vec::new(),
                },
            }))
            .is_err()
        );
    }
    assert!(
        encode_server_message(&ServerMessage::ScrollOutcome(
            ScrollOutcome::TerminalOwned { requested_rows: 0 }
        ))
        .is_err()
    );
    assert!(
        encode_server_message(&ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows: 1,
            applied_rows: 1,
            frame: Box::new(frame()),
            next: PreviewOutcome::TerminalRouted,
        }))
        .is_err()
    );

    let request = ClientMessage::ScrollVertical {
        frame_revision: 7,
        rows: 1,
    };
    let mut invalid_request = encode_client_message(&request).unwrap();
    invalid_request[HEADER_BYTES + 8..HEADER_BYTES + 10]
        .copy_from_slice(&(MAX_SCROLL_ROWS + 1).to_le_bytes());
    assert_eq!(
        decode_client_message(&invalid_request),
        Err(Error::InvalidValue {
            field: "vertical scroll rows",
        })
    );
}

#[test]
fn return_to_live_has_zero_payload() {
    assert_eq!(
        encode_client_message(&ClientMessage::ReturnToLive)
            .unwrap()
            .len(),
        HEADER_BYTES
    );
}
