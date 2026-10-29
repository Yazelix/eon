use super::*;

#[test]
fn stalled_link_dispatcher_is_bounded_and_reaped() {
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let now = Instant::now();
    let mut opener = LinkOpener {
        child,
        deadline: now + Duration::from_secs(10),
    };
    assert!(opener.poll(now).is_none());
    assert!(
        opener
            .poll(now + Duration::from_secs(10))
            .unwrap()
            .unwrap_err()
            .contains("timed out")
    );
    drop(opener);
    // Signal 0 reaches an unreaped zombie and fails once the child is reaped.
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let reaped = !status.success();
    assert!(reaped, "dispatcher must be reaped");
}

#[test]
fn link_success_notice_clears_on_its_own_and_failures_persist() {
    let now = Instant::now();
    let mut opener = LinkOpener {
        child: Command::new("true").spawn().unwrap(),
        deadline: now + Duration::from_secs(10),
    };
    let handed = loop {
        if let Some(result) = opener.poll(now) {
            break result;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(handed, Ok("Link handed to the desktop handler."));
    let mut links = LinkInteraction::default();
    links.show_notice(handed.unwrap(), true, now, 0);
    let deadline = links.notice_deadline().unwrap();
    assert_eq!(deadline, now + LINK_SUCCESS_NOTICE);
    assert!(!links.expire_notice(deadline - Duration::from_millis(1)));
    assert!(
        links.expire_notice(deadline),
        "success clears with no input"
    );
    assert_eq!((links.notice(), links.notice_deadline()), (None, None));
    links.show_notice("Could not open link.", false, now, 0);
    assert!(!links.expire_notice(now + Duration::from_secs(3600)));
    assert_eq!(links.notice(), Some("Could not open link."));
}

#[test]
fn hyperlink_actions_require_current_presentation_and_safe_exact_uri() {
    let mut presentation = PresentationState::default();
    let first = presentation.candidate(Some(7));
    assert!(!presentation.is_current(first));
    presentation.publish(first);
    assert!(presentation.is_current(first));
    presentation.content_changed();
    assert!(!presentation.is_current(first));
    let second = presentation.candidate(Some(8));
    assert!(!presentation.is_current(second));
    presentation.publish(second);
    assert!(presentation.is_current(second));
    presentation.unpublish();
    assert!(!presentation.is_current(second));
    presentation.invalidate();
    presentation.publish(first);
    assert!(!presentation.is_current(first));

    for uri in [
        "https://example.com/exact?x=%26&y=2#part",
        "HTTP://localhost:8080/",
        "https://[::1]:443/a",
    ] {
        assert_eq!(validate_open_uri(uri), Ok(()), "{uri}");
    }
    for uri in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https:///bad",
        "https://user@example.com/",
        "https://example.com\\@evil.test",
        "https://example.com/%zz",
        "https://example.com/%0",
        "https://example.com/\n",
        "https://example.com/\u{202e}",
        "https://[bad]/",
        "https://example.com:99999/",
        "https://-bad.example/",
        "https://example.com/ space",
    ] {
        assert!(validate_open_uri(uri).is_err(), "{uri:?}");
    }
    assert!(validate_open_uri(&format!("https://example.com/{}", "x".repeat(4096))).is_err());
    assert!(validate_copy_uri("file:///tmp/a").is_ok());
    assert!(validate_copy_uri("https://example.com/\n").is_err());
    let target = "https://example.com/\u{202e}abc";
    assert_eq!(
        link_hint_for(NativePlatform::Linux, target, &[]),
        "https://example.com/\\u{202e}abc\nCtrl+click Open · Ctrl+Shift+C Copy"
    );
    let copy_modifiers = copy_paste_modifiers(NATIVE_PLATFORM);
    assert!(link_copy_shortcut(
        PhysicalKey::Code(KeyCode::KeyC),
        copy_modifiers,
        &[],
        false,
        true,
    ));
    assert!(!link_copy_shortcut(
        PhysicalKey::Code(KeyCode::KeyC),
        copy_modifiers,
        &[],
        true,
        true,
    ));
    for key in [
        KeyCode::KeyO,
        KeyCode::Tab,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::Enter,
        KeyCode::NumpadEnter,
        KeyCode::Escape,
    ] {
        assert!(!link_copy_shortcut(
            PhysicalKey::Code(key),
            copy_modifiers,
            &[],
            false,
            true,
        ));
    }
}

#[test]
fn revision_bound_selection_waits_for_a_recovered_surface() {
    let mouse = ClientMessage::Mouse(orbit_protocol::session::MouseEvent {
        action: orbit_protocol::session::MouseAction::Motion,
        button: None,
        modifiers: orbit_protocol::session::Modifiers::empty(),
        x: 0.0,
        y: 0.0,
    });
    let selection = ClientMessage::Selection(SelectionAction::Begin {
        frame_revision: 7,
        position: orbit_protocol::session::SelectionPosition { x: 0.0, y: 0.0 },
        time_ns: 0,
        modifiers: orbit_protocol::session::Modifiers::empty(),
    });

    assert!(can_follow_implicit_resize(&mouse));
    assert!(!can_follow_implicit_resize(&selection));
}

#[test]
fn copy_is_revision_free_but_waits_for_selection_finish() {
    use winit::event::ElementState::{Pressed, Released};

    let copy = ClientMessage::Selection(SelectionAction::Copy);
    assert!(selection_presentation_is_ready(&copy, false));
    assert!(!copy_is_ready(true, Pressed, false));
    assert!(copy_is_ready(false, Pressed, false));
    assert!(!copy_is_ready(false, Pressed, true));
    assert!(!copy_is_ready(false, Released, false));
}

#[test]
fn key_release_does_not_dismiss_clipboard_result() {
    let key = |action| {
        ClientMessage::Key(session::KeyEvent {
            action,
            key: session::PhysicalKey::UNIDENTIFIED,
            modifiers: session::Modifiers::empty(),
            consumed_modifiers: session::Modifiers::empty(),
            composing: false,
            text: None,
            unshifted_codepoint: None,
        })
    };

    assert!(!dismisses_clipboard_notice(
        LocalNoticeSource::Resize,
        &key(session::KeyAction::Press)
    ));
    assert!(!dismisses_clipboard_notice(
        LocalNoticeSource::Input,
        &key(session::KeyAction::Release)
    ));
    assert!(dismisses_clipboard_notice(
        LocalNoticeSource::Input,
        &key(session::KeyAction::Press)
    ));
}

#[test]
fn clipboard_failure_outranks_stale_link_notice() {
    assert_eq!(
        status_notice(Some("paste failed"), Some("link copied")),
        Some("paste failed")
    );
}

#[test]
fn clipboard_results_are_attributed_without_copying_terminal_cells() {
    use NativeClipboard::{Both, Standard};

    assert_eq!(
        clipboard_notice(Standard, Ok::<(), &str>(())),
        "Text copied to the native clipboard."
    );
    assert_eq!(
        clipboard_notice(Both, Ok::<(), &str>(())),
        "Text copied to the native clipboard and primary selection."
    );
    assert_eq!(
        clipboard_notice(Standard, Err("display unavailable")),
        "Venus could not write the native clipboard: display unavailable"
    );
}

#[test]
fn deferred_selection_preserves_order_bounds_and_cancel_ownership() {
    let begin = |revision| {
        ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: revision,
            position: orbit_protocol::session::SelectionPosition { x: 1.0, y: 2.0 },
            time_ns: 3,
            modifiers: orbit_protocol::session::Modifiers::empty(),
        })
    };
    let finish = ClientMessage::Selection(SelectionAction::Finish {
        position: orbit_protocol::session::SelectionPosition { x: 1.0, y: 2.0 },
        modifiers: orbit_protocol::session::Modifiers::empty(),
    });
    let mut pending = VecDeque::from([
        begin(1),
        finish.clone(),
        begin(1),
        finish.clone(),
        ClientMessage::Selection(SelectionAction::Copy),
    ]);

    let first = take_deferred_selection(&mut pending, 7);
    assert!(matches!(
        first.as_slice(),
        [
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: 7,
                ..
            }),
            ClientMessage::Selection(SelectionAction::Finish { .. })
        ]
    ));
    assert!(matches!(
        pending.front(),
        Some(ClientMessage::Selection(SelectionAction::Begin { .. }))
    ));

    assert_eq!(take_deferred_selection(&mut pending, 8).len(), 2);
    assert_eq!(
        take_deferred_selection(&mut pending, 9),
        [ClientMessage::Selection(SelectionAction::Copy)]
    );
    assert!(pending.is_empty());

    pending.resize(
        MAX_DEFERRED_SELECTION_MESSAGES,
        ClientMessage::Selection(SelectionAction::Copy),
    );
    assert!(!queue_deferred_selection(
        &mut pending,
        ClientMessage::Selection(SelectionAction::Copy)
    ));

    let local_begin = VecDeque::from([ClientMessage::Selection(SelectionAction::Copy), begin(1)]);
    let server_finish = VecDeque::from([finish]);
    let ready = SelectionGate::Ready;
    let empty = VecDeque::new();
    assert!(!pointer_sequence_needs_cancel(&ready, true, &local_begin));
    assert!(pointer_sequence_needs_cancel(&ready, false, &server_finish));
    assert!(pointer_sequence_needs_cancel(
        &SelectionGate::AwaitingFinish,
        false,
        &empty
    ));
    assert!(pointer_sequence_needs_cancel(&ready, true, &empty));
    assert!(!pointer_sequence_needs_cancel(&ready, false, &empty));
}

#[test]
fn selection_gate_waits_for_finish_and_its_authoritative_presentation() {
    let mut gate = SelectionGate::Ready;

    assert!(gate.admits(7));
    gate.finish_sent();
    assert!(!gate.admits(7));

    gate.finish_received(8);
    assert!(!gate.admits(7));
    assert!(gate.admits(8));

    gate.finish_sent();
    gate.finish_received(8);
    assert!(gate.admits(9));
}

#[test]
fn native_clipboard_text_becomes_one_bounded_semantic_paste() {
    let text = "first\n界\0second";
    assert_eq!(
        clipboard_paste_message(text.as_bytes()),
        Ok(ClientMessage::Paste(text.as_bytes().to_vec()))
    );
    assert_eq!(
        clipboard_paste_message(&b""[..]).unwrap_err(),
        "The native clipboard contains no text"
    );
    let mut oversized = io::Cursor::new(vec![b'x'; session::MAX_PASTE_BYTES + 2]);
    assert_eq!(
        clipboard_paste_message(&mut oversized).unwrap_err(),
        format!(
            "Native clipboard text exceeds the {} byte paste limit",
            session::MAX_PASTE_BYTES
        )
    );
    assert_eq!(oversized.position(), session::MAX_PASTE_BYTES as u64 + 1);
    assert_eq!(
        clipboard_paste_message(&[0xff][..]).unwrap_err(),
        "stream did not contain valid UTF-8"
    );
    assert_eq!(
        clipboard_paste_notice("display unavailable"),
        "Venus could not paste from the native clipboard: display unavailable"
    );
}

#[test]
fn clipboard_effects_preserve_native_targets() {
    use NativeClipboard::{Both, Primary, Standard};

    assert!(matches!(
        native_clipboard_for(
            NativePlatform::Linux,
            &ClipboardEffect::SelectionCopy {
                location: ClipboardLocation::Selection,
                text: String::new(),
            }
        ),
        Both
    ));
    assert!(matches!(
        native_clipboard_for(
            NativePlatform::Linux,
            &ClipboardEffect::SelectionCopy {
                location: ClipboardLocation::Standard,
                text: String::new(),
            }
        ),
        Standard
    ));
    assert!(matches!(
        native_clipboard_for(
            NativePlatform::Linux,
            &ClipboardEffect::TerminalWrite {
                location: ClipboardLocation::Selection,
                text: String::new(),
            }
        ),
        Primary
    ));
    for effect in [
        ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Selection,
            text: String::new(),
        },
        ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Primary,
            text: String::new(),
        },
        ClipboardEffect::TerminalWrite {
            location: ClipboardLocation::Standard,
            text: String::new(),
        },
    ] {
        assert_eq!(
            native_clipboard_for(NativePlatform::Macos, &effect),
            Standard
        );
    }
}
