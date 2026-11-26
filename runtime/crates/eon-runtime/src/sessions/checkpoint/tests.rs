use super::super::{
    AttachName, LeaseRequest, Naming, RequestKind, RunningSession, SessionEnd,
    tests::tombstoned_session,
};
use crate::checkpoint::{
    Error,
    update::{Named, TerminalExtra},
};
use crate::supervisor::temporary_directory;
use orbit_protocol::management::TerminationReason;
use std::{os::unix::net::UnixStream, time::Instant};

/// A leased row and the lease's other end, which must outlive it.
fn row() -> (RunningSession, UnixStream) {
    let (session, peer, _) =
        tombstoned_session(&temporary_directory(), 3, TerminationReason::NaturalExit);
    (session, peer)
}

const NAME: AttachName = AttachName {
    process_id: 41,
    process_start: 7,
};

#[test]
fn a_settled_row_carries_its_default_shell_and_name() {
    let (mut session, _peer) = row();
    session.default_shell = true;
    session.naming = Some(Naming::Named(NAME));
    let extra = session.checkpoint_extra().unwrap();
    assert_eq!(
        extra,
        TerminalExtra {
            terminal: "terminal-3".into(),
            default_shell: true,
            named: Some(Named {
                process_id: 41,
                process_start: 7,
            }),
        }
    );

    let (mut adopted, _adopted_peer) = row();
    adopted.restore_extra(&extra);
    assert!(adopted.default_shell);
    assert!(matches!(adopted.naming, Some(Naming::Named(name)) if name == NAME));
    assert_eq!(adopted.checkpoint_extra().unwrap(), extra);

    session.naming = None;
    session.default_shell = false;
    adopted.restore_extra(&session.checkpoint_extra().unwrap());
    assert!(adopted.naming.is_none() && !adopted.default_shell);
}

/// An unsettled state's refusal and how to reach it.
type Case = (&'static str, fn(&mut RunningSession));

#[test]
fn an_unsettled_row_refuses_capture() {
    let cases: [Case; 6] = [
        ("a terminal request is in flight", |session| {
            session.request = Some(LeaseRequest {
                kind: RequestKind::ProgramStatus,
                unsent: Vec::new(),
                reply: Vec::new(),
                deadline: Instant::now(),
                reader: super::super::Reader::Lease,
            });
        }),
        ("a terminal status answer is not consumed", |session| {
            session.answered_status = Some((Vec::new(), Instant::now()));
        }),
        ("a terminal naming retry is pending", |session| {
            session.naming = Some(Naming::Failed {
                name: NAME,
                retry: Instant::now(),
            });
        }),
        ("a terminal is stopping", |session| session.stop_sent = true),
        ("a terminal has ended", |session| {
            session.finished = Some(SessionEnd::OwnerLost(None));
        }),
        ("a terminal's end is not reconciled", |session| {
            session.reap_failure_reported = true;
        }),
    ];
    for (what, unsettle) in cases {
        let (mut session, _peer) = row();
        unsettle(&mut session);
        assert_eq!(session.checkpoint_extra(), Err(Error::Unsettled(what)));
    }
}
