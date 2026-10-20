use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "slow-gate");
    let shell = script(&directory, "SLOW_SHELL", "slow-observer-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_SLOW_GATE", gate.path())]);
    let mut observer = Peer::connect(&server.path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    observer.metadata();

    // The observer stops reading while the child changes its title a
    // thousand times, far past the socket's buffer.
    gate.release();
    sleep(2.0);

    // It stays attached, and its queue kept only the latest change behind
    // what the socket already held.
    let mut numbered = Vec::new();
    loop {
        let title = observer.metadata().title;
        if title == "SLOW LAST" {
            break;
        }
        numbered.push(title[..4].parse::<u32>().expect("a numbered title"));
    }
    assert!(
        numbered.windows(2).all(|pair| pair[0] < pair[1]),
        "{:?}",
        &numbered[..numbered.len().min(20)]
    );
    assert!(numbered.len() < 1000, "{}", numbered.len());
    observer.quiet(0.3, "metadata after the latest change");

    // Output that leaves the title and working directory as they were
    // publishes no metadata.
    gate.release();
    observer.quiet(0.3, "metadata that repeats the last");

    gate.release();
    observer.expect(ServerMessage::Exited { code: 7 });
    drop(observer);

    server.passed(7, 10.0);
}
