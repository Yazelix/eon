use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

const OUTPUT_BOUND: usize = 12_587_045; // Bend's output_bound_bytes

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let stop = directory.join("stop");
    let shell = script(&directory, "INTERACTIVE_PRESSURE_SHELL", "interactive-pressure-shell");
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[("ORBIT_PID_FILE", &pid_file), ("ORBIT_STOP", &stop)],
        &[&pid_file],
        &[],
    );
    let pid = read_pid(&pid_file);

    let mut stalled = Peer::connect(&server.path, 10.0);
    stalled.send(&ClientMessage::Hello);
    stalled.expect(ServerMessage::Attached);
    stalled.frame();

    let mut observer = Peer::connect(&server.path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);

    // The interactive client stops reading and keeps sending requests that
    // Bend answers with Failure, until the server closes.
    let sent = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let sender = {
        let mut stream = stalled.stream.try_clone().expect("clone");
        let (sent, stopped) = (Arc::clone(&sent), Arc::clone(&stopped));
        thread::spawn(move || {
            let batch = encode(&idle_tick()).repeat(4096);
            loop {
                match stream.write_all(&batch) {
                    Ok(()) => sent.fetch_add(4096, Ordering::Relaxed),
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                        ) =>
                    {
                        stopped.store(true, Ordering::Relaxed);
                        return;
                    }
                    Err(error) => panic!("flood: {error}"),
                };
            }
        })
    };

    // The observer keeps receiving metadata while the client does not read.
    // Orbit serves an observer only while a client is attached.
    let mut titles = HashSet::new();
    let deadline = Instant::now() + seconds(60.0);
    observer.set_timeout(0.5);
    while !stopped.load(Ordering::Relaxed) {
        assert!(
            Instant::now() < deadline,
            "no detach after {} requests",
            sent.load(Ordering::Relaxed)
        );
        match observer.receive_within() {
            None => continue,
            Some(ServerMessage::Metadata(metadata)) => {
                titles.insert(metadata.title);
            }
            Some(other) => panic!("expected metadata, received {}", brief(&other)),
        }
    }
    assert!(titles.len() > 3, "{titles:?}");
    let _ = sender.join();
    drop(observer);

    // Bend detached the client: what it reads is whole messages, then at most
    // one cut message. Closing with unread requests may reset instead.
    stalled.set_timeout(10.0);
    let (data, reset) = stalled.read_until_closed(true);
    drop(stalled);
    let (messages, cut) = split(&data);
    let failures = messages
        .iter()
        .filter(|message| matches!(message, ServerMessage::Failure(_)))
        .count();
    for message in &messages {
        assert!(
            matches!(message, ServerMessage::Frame(_) | ServerMessage::Failure(_)),
            "{}",
            brief(message)
        );
    }
    assert!(data.len() < 2 * OUTPUT_BOUND, "{}", data.len());
    assert!(alive(pid), "the child stopped with the client");

    // A new client reattaches to the same child and receives a current frame.
    let mut client = Peer::connect(&server.path, 10.0);
    client.send(&ClientMessage::Hello);
    client.expect(ServerMessage::Attached);
    let current = client.frame();
    assert!(current.title.starts_with("TICK "), "{:?}", current.title);
    fs::write(&stop, "").expect("stop");
    client.exited(9);
    drop(client);

    assert_eq!(server.passed(9, 10.0), pid);
    println!(
        "detached after {} requests; read {} bytes ({failures} Failure), {}, cut tail {cut}",
        sent.load(Ordering::Relaxed),
        data.len(),
        if reset { "reset" } else { "end of stream" },
    );
}
