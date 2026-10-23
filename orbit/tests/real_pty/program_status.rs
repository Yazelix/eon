use crate::management_lease::{Manager, Reply};
use crate::management_record::suffixed;
use crate::support::*;
use orbit_protocol::management::{
    ClientMessage, ProgramKind, ProgramState, ProgramStatus, ServerMessage,
};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

const MANAGED: [&str; 4] = ["--management-v2", "session-3", "run-3", "component-3"];

/// A managed run of a bash script, with `env` and a lease on its management
/// socket.
fn managed(directory: &Path, script: &str, env: &[(&str, &Path)]) -> (Server, Manager) {
    let shell = executable(
        directory,
        "child.sh",
        &format!("#!{}\n{script}", bash().display()),
    );
    let server = Server::launch(directory, &[shell.as_os_str()], env, &[], &MANAGED);
    let record = suffixed(&server.path, ".record");
    wait_for(&record);
    let mut lease = Manager::new(&record);
    assert!(
        matches!(lease.acquire(), Reply::Message(ServerMessage::Lease(_))),
        "no lease"
    );
    (server, lease)
}

fn status(lease: &mut Manager) -> ProgramStatus {
    match lease.send(&ClientMessage::ProgramStatus) {
        Reply::Message(ServerMessage::ProgramStatus(status)) => status,
        other => panic!("expected ProgramStatus, received {other:?}"),
    }
}

/// The reply's ids and states, in its order.
fn records(status: &ProgramStatus) -> Vec<(&str, ProgramState)> {
    status
        .records
        .iter()
        .map(|record| (record.id.as_str(), record.state))
        .collect()
}

/// Asks until the reply holds a record with `id`.
fn until_reported(lease: &mut Manager, id: &str) -> ProgramStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = status(lease);
        if status.records.iter().any(|record| record.id == id) {
            return status;
        }
        assert!(Instant::now() < deadline, "{id} never arrived: {status:?}");
        sleep(0.02);
    }
}

/// Whether the terminal's foreground group is the child's own, from /proc.
fn child_leads(child: i32) -> bool {
    let stat = fs::read_to_string(format!("/proc/{child}/stat")).expect("stat");
    let (_, fields) = stat.rsplit_once(')').expect("stat fields");
    let tpgid: i32 = fields.split_whitespace().nth(5).expect("tpgid").parse().expect("a number");
    tpgid == child
}

fn until_leads(child: i32, leads: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child_leads(child) != leads {
        assert!(Instant::now() < deadline, "the foreground never moved");
        sleep(0.01);
    }
}

/// Stop through the lease ends the run.
fn stop(mut server: Server, mut lease: Manager) {
    let reply = lease.send(&ClientMessage::Stop);
    assert!(
        matches!(reply, Reply::Message(ServerMessage::Stopped(_))),
        "{reply:?}"
    );
    drop(lease);
    server.communicate(10.0).assert_silent(0);
}

pub fn run() {
    prompt_marks_drop_in_stream_order();
    foreground_returns_drop_once();
    detached_turns_end_at_a_return();
    answer_time_under_load();
}

/// A prompt mark drops the `working` and `blocked` records before it in the
/// stream, and a report after it in the same write survives. The reply
/// carries the window title the child set, and an empty one, without ending
/// the run, for a title whose 1024-byte cut in libghostty splits a character.
fn prompt_marks_drop_in_stream_order() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "gate");
    let (server, mut lease) = managed(
        &directory,
        "read -r _ < \"$ORBIT_GATE\"\n\
         printf '\\033]0;eon-9a\\007\\033]7501;state=working:id=a\\007\\033]7501;state=blocked:kind=permission:id=b\\007\
         \\033]7501;state=done:id=c\\007\\033]133;A\\007\\033]7501;state=working:id=d\\007'\n\
         read -r _ < \"$ORBIT_GATE\"\n\
         t=$(printf '%1023s' '')\n\
         printf '\\033]0;%s\\303\\251\\007\\033]7501;state=working:id=e\\007' \"${t// /a}\"\n\
         read -r _ < \"$ORBIT_GATE\"\n",
        &[("ORBIT_GATE", gate.path())],
    );
    let empty = status(&mut lease);
    assert_eq!((empty.count, empty.omitted.len(), empty.records.len()), (0, 0, 0), "{empty:?}");
    assert_eq!(empty.terminal_title, "");
    gate.release();
    let status = until_reported(&mut lease, "d");
    assert_eq!(status.terminal_title, "eon-9a");
    assert_eq!(
        records(&status),
        [("c", ProgramState::Done), ("d", ProgramState::Working)],
        "{status:?}"
    );
    assert_eq!((status.count, status.omitted.len()), (2, 0));
    gate.release();
    let status = until_reported(&mut lease, "e");
    assert_eq!(status.terminal_title, "", "{status:?}");
    stop(server, lease);
}

/// Without prompt marks, the foreground's return to the child's group drops
/// `working` records once: a report after the return survives later samples.
fn foreground_returns_drop_once() {
    let directory = TempDir::new();
    let job = Gate::new(&directory, "job");
    let back = Gate::new(&directory, "back");
    let (server, mut lease) = managed(
        &directory,
        "set -m\n\
         printf '\\033]7501;state=working:app=make:id=w\\007'\n\
         cat \"$ORBIT_JOB\" > /dev/null\n\
         read -r _ < \"$ORBIT_BACK\"\n\
         printf '\\033]7501;state=working:id=v\\007'\n\
         read -r _ < \"$ORBIT_BACK\"\n",
        &[("ORBIT_JOB", job.path()), ("ORBIT_BACK", back.path())],
    );
    // A request samples the job's group: the record stays.
    until_leads(server.child, false);
    let before = until_reported(&mut lease, "w");
    assert_eq!(records(&before), [("w", ProgramState::Working)]);
    assert_eq!(before.records[0].app, "make");

    job.release();
    until_leads(server.child, true);
    let returned = status(&mut lease);
    assert_eq!((returned.count, returned.records.len()), (0, 0), "{returned:?}");
    assert_eq!(returned.update, before.update + 1, "the drop is an update");

    back.release();
    until_reported(&mut lease, "v");
    sleep(0.1);
    let again = status(&mut lease);
    assert_eq!(records(&again), [("v", ProgramState::Working)], "{again:?}");
    stop(server, lease);
}

/// A detached turn reads up to 64 times; when the foreground returns to the
/// child mid-turn, the turn ends there. A job's report before the return is
/// dropped and the shell's report after it, in the same stream of output,
/// survives. The job leaves a writer in its group that keeps the PTY full
/// across the return, so the return lands inside a turn. The writer's output,
/// screen fills and colored wide graphemes, costs the terminal far more than
/// the writer, and it writes 512 bytes at a time, so the shell's report
/// queues behind little output and lands in the turn that the return ended
/// early. Without that early end, the first round failed in each of three
/// runs; three rounds keep a miss unlikely on a busier machine.
fn detached_turns_end_at_a_return() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "gate");
    let filler = directory.join("filler");
    fs::write(
        &filler,
        "\x1b#8\x1b[38;5;196;48;5;21m\u{1F600}\x1b[0m".repeat(160 * 1024),
    )
    .expect("filler");
    let (server, mut lease) = managed(
        &directory,
        "set -m\n\
         stty -tostop\n\
         for round in 1 2 3; do\n\
           \"$BASH\" -c 'printf \"\\033]7501;state=blocked:kind=question:id=job%s\\007\" \"$0\"\n\
             read -r _ < \"$ORBIT_GATE\"\n\
             { dd if=\"$ORBIT_FILLER\" bs=512 status=none; : > \"$ORBIT_DIR/streamed$0\"; \
               printf \"\\033]7501;state=done:id=end%s\\007\" \"$0\"; } &\n\
             sleep 0.05' \"$round\"\n\
           printf '\\033]7501;state=working:id=shell%s\\007' \"$round\"\n\
           : > \"$ORBIT_DIR/reported$round\"\n\
           read -r _ < \"$ORBIT_GATE\"\n\
         done\n\
         read -r _ < \"$ORBIT_GATE\"\n",
        &[
            ("ORBIT_GATE", gate.path()),
            ("ORBIT_DIR", &directory),
            ("ORBIT_FILLER", &filler),
        ],
    );
    for round in 1..=3 {
        until_leads(server.child, false);
        let job = format!("job{round}");
        let before = until_reported(&mut lease, &job);
        assert!(
            before.records.iter().any(|record| record.id == job
                && record.kind == Some(ProgramKind::Question)),
            "{before:?}"
        );

        // No request samples the foreground until the whole stream is in the
        // PTY, long after the return; the writer's last report ends it.
        gate.release();
        wait_for(&directory.join(format!("reported{round}")));
        wait_for(&directory.join(format!("streamed{round}")));
        until_reported(&mut lease, &format!("end{round}"));
        let shell = format!("shell{round}");
        let after = until_reported(&mut lease, &shell);
        // Every writer's `done` report stays, the most recent first.
        let ends: Vec<_> = (1..=round).rev().map(|end| format!("end{end}")).collect();
        let mut expected: Vec<_> = ends
            .iter()
            .map(|end| (end.as_str(), ProgramState::Done))
            .collect();
        expected.push((shell.as_str(), ProgramState::Working));
        assert_eq!(records(&after), expected, "round {round}: {after:?}");
        gate.release();
    }
    stop(server, lease);
}

/// ProgramStatus answers while the child floods the PTY. Prints the
/// distribution of answer times; each must fall within the worker-stall
/// bound of two seconds.
fn answer_time_under_load() {
    let directory = TempDir::new();
    let (server, mut lease) = managed(
        &directory,
        "printf '\\033]7501;state=working:id=load\\007'\n\
         yes 0123456789abcdef\n",
        &[],
    );
    until_reported(&mut lease, "load");
    let mut times: Vec<_> = (0..200)
        .map(|_| {
            let started = Instant::now();
            let status = status(&mut lease);
            let elapsed = started.elapsed();
            assert_eq!(records(&status), [("load", ProgramState::Working)]);
            sleep(0.01);
            elapsed
        })
        .collect();
    times.sort();
    let at = |quantile: f64| times[((times.len() - 1) as f64 * quantile) as usize];
    // Straight to stderr, which the test harness does not capture, so a
    // passing check's log keeps the distribution.
    writeln!(
        std::io::stderr(),
        "ProgramStatus answer under PTY load, {} requests: min {:?}, median {:?}, p95 {:?}, p99 {:?}, max {:?}",
        times.len(),
        times[0],
        at(0.5),
        at(0.95),
        at(0.99),
        times[times.len() - 1]
    )
    .expect("stderr");
    assert!(times[times.len() - 1] < Duration::from_secs(2), "{times:?}");
    stop(server, lease);
}
