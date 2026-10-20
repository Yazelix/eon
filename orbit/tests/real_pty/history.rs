use crate::support::*;

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "history-gate");
    let shell = script(&directory, "HISTORY_SHELL", "history-shell");
    let mut server =
        Server::start(&directory, &shell, &[("ORBIT_HISTORY_GATE", gate.path())]);
    let mut first = Peer::attach(&server.path, 10.0);
    let initial = first.frame().scroll_position;
    assert_eq!((initial.rows_from_live, initial.history_rows), (0, 0));
    gate.release();
    let done = first.frame_titled("HISTORY DONE").scroll_position;
    drop(first);
    // 20,001 printed lines on a 24-row screen; the old 1,024-byte budget kept far fewer.
    assert_eq!(done.rows_from_live, 0);
    assert!(
        (19_950..=20_001).contains(&done.history_rows),
        "{}",
        done.history_rows
    );

    let mut second = Peer::attach(&server.path, 10.0);
    let current = second.frame();
    assert_eq!(
        (
            current.title.as_str(),
            current.scroll_position.rows_from_live,
            current.scroll_position.history_rows,
        ),
        ("HISTORY DONE", 0, done.history_rows)
    );
    gate.release();
    second.exited(7);
    drop(second);

    server.passed(7, 10.0);
}
