use super::*;

#[test]
fn orbit_retry_schedule_is_bounded_and_resettable() {
    let now = Instant::now();
    let mut retry = OrbitRetry::default();

    for delay in [250, 500, 1_000, 2_000, 4_000, 5_000, 5_000] {
        retry.schedule(now);
        assert_eq!(retry.deadline, Some(now + Duration::from_millis(delay)));
        retry.schedule(now + Duration::from_millis(1));
        assert_eq!(retry.deadline, Some(now + Duration::from_millis(delay)));
        assert!(!retry.take_due(now + Duration::from_millis(delay - 1)));
        assert!(retry.take_due(now + Duration::from_millis(delay)));
    }

    retry.schedule(now);
    retry.reset();
    assert_eq!(retry.deadline, None);
    retry.schedule(now);
    assert_eq!(retry.deadline, Some(now + Duration::from_millis(250)));
}

#[test]
fn a_refused_image_resyncs_once_until_its_placements_change_or_the_budget_gains_room() {
    let shown = vec![SceneRect {
        left: 1.0,
        top: 2.0,
        width: 4.0,
        height: 2.0,
    }];
    let mut allowance = RefusalResync::default();
    assert!(!allowance.admit(None, shown.clone(), 100));
    // The budget still has no room: a reattach would be refused again.
    assert!(!allowance.admit(Some(ImageResync::NoRoom), shown.clone(), 100));
    assert!(allowance.admit(Some(ImageResync::Refused), shown.clone(), 100));
    // Reattaching frees the old attachment's textures before the next refusal, which is the
    // budget the allowance waits to see improve.
    assert!(!allowance.admit(None, shown.clone(), 40));
    assert!(!allowance.admit(Some(ImageResync::NoRoom), shown.clone(), 60));
    assert!(!allowance.admit(Some(ImageResync::Refused), shown.clone(), 60));
    assert!(allowance.admit(Some(ImageResync::Refused), shown.clone(), 59));
    // Spent again; a change of the terminal's placements renews it.
    assert!(!allowance.admit(Some(ImageResync::Refused), shown.clone(), 59));
    let resized = vec![SceneRect {
        height: 3.0,
        ..shown[0]
    }];
    assert!(allowance.admit(Some(ImageResync::Refused), resized, 59));
}

#[test]
fn workspace_recovery_requires_the_same_selected_live_endpoint() {
    let first = b"/run/eon/session-1.sock";
    let second = b"/run/eon/session-2.sock";

    assert!(retry_is_allowed(false, None, None));
    assert!(retry_is_allowed(true, Some((first, true)), Some(first)));
    assert!(!retry_is_allowed(true, Some((first, false)), Some(first)));
    assert!(!retry_is_allowed(true, Some((second, true)), Some(first)));
}

#[test]
fn only_supervised_orbit_busy_is_retryable() {
    let busy = TransportEvent::Server(ServerMessage::Busy);

    assert!(managed_busy_is_retryable(true, &busy));
    assert!(!managed_busy_is_retryable(false, &busy));
    assert!(!managed_busy_is_retryable(
        true,
        &TransportEvent::Server(ServerMessage::Accepted),
    ));
}

#[test]
fn protocol_and_terminal_failures_do_not_become_socket_retries() {
    use orbit_protocol::session::Failure;

    for (code, suppressed) in [
        (FailureCode::InvalidInput, false),
        (FailureCode::Protocol, true),
        (FailureCode::Terminal, true),
    ] {
        assert_eq!(
            server_failure_suppresses_retry(&ServerMessage::Failure(Failure {
                code,
                detail: "bounded".into(),
            })),
            suppressed
        );
    }
}

#[test]
fn retryable_loss_records_state_independently_of_retry_policy() {
    use orbit_protocol::session::Failure;

    let attached = || {
        let mut model = SessionModel::new();
        model.apply(ServerMessage::Attached).unwrap();
        model
    };
    let fail = |model: &mut SessionModel, code, detail: &str| {
        model
            .apply(ServerMessage::Failure(Failure {
                code,
                detail: detail.into(),
            }))
            .unwrap();
    };
    let mut suppressed = attached();
    fail(&mut suppressed, FailureCode::Terminal, "request rejected");
    assert_eq!(
        suppressed.notice(),
        Some("Orbit terminal failure: request rejected")
    );
    suppressed.apply(ServerMessage::Accepted).unwrap();
    assert_eq!(suppressed.notice(), None);
    fail(
        &mut suppressed,
        FailureCode::InvalidInput,
        "later rejected input",
    );
    assert_eq!(
        suppressed.notice(),
        Some("Orbit rejected input: later rejected input")
    );

    assert!(!apply_retryable_loss(
        &mut suppressed,
        "Orbit closed the local session".into(),
        true,
    ));
    assert!(matches!(
        suppressed.connection(),
        ConnectionState::Lost { detail } if detail == "Orbit closed the local session"
    ));
    assert!(!suppressed.is_attached());
    assert_eq!(suppressed.notice(), None);

    let mut protocol_failure = attached();
    fail(
        &mut protocol_failure,
        FailureCode::Protocol,
        "invalid mouse coordinates",
    );
    protocol_failure.set_venus_notice(LocalNoticeSource::Input, "late local notice");
    assert!(!apply_retryable_loss(
        &mut protocol_failure,
        "Orbit closed the local session".into(),
        true,
    ));
    assert_eq!(
        protocol_failure.notice(),
        Some("Orbit protocol failure: invalid mouse coordinates")
    );
    assert!(matches!(
        protocol_failure.connection(),
        ConnectionState::Lost { .. }
    ));

    let mut eligible = attached();
    assert!(apply_retryable_loss(
        &mut eligible,
        "Cannot read from Orbit: connection reset".into(),
        false,
    ));
    assert!(matches!(
        eligible.connection(),
        ConnectionState::Lost { detail }
            if detail == "Cannot read from Orbit: connection reset"
    ));
}

#[test]
fn attachment_revision_collision_waits_for_the_new_generation() {
    let mut presentation = PresentationState::default();
    let attachment_a = presentation.candidate(Some(1));
    presentation.publish(attachment_a);
    assert_eq!(presentation.presented_revision(), Some(1));

    presentation.invalidate();
    let attachment_b = presentation.candidate(Some(1));
    assert_ne!(attachment_b, attachment_a);
    assert_eq!(presentation.presented_revision(), None);

    presentation.publish(attachment_b);
    assert_eq!(presentation.presented_revision(), Some(1));
}
