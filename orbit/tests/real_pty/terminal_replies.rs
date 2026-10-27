use crate::support::*;
use std::time::Instant;

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "reply-gate");
    let answers = directory.join("replies");
    let shell = script(&directory, "REPLY_SHELL", "terminal-replies-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_REPLY_GATE", gate.path()),
            ("ORBIT_REPLIES", &answers),
        ],
    );
    // The terminal answers the child's cursor-position query and its program
    // status probe while no client is attached: Orbit, not a client, owns
    // terminal replies.
    let deadline = Instant::now() + seconds(10.0);
    while !answers.exists() || !replies(&answers).contains_key("probe") {
        assert!(
            Instant::now() < deadline,
            "the detached queries got no answer line"
        );
        assert!(server.alive(), "server exited before the child asked");
        sleep(0.05);
    }
    let got = replies(&answers);
    assert_eq!(got["detached"], "$'\\E[2;1'", "{got:?}");
    // The program status probe is answered before the device attributes
    // query that follows it (ORB-C15).
    let probe = &got["probe"];
    let answer = "$'\\E]7501;?:states=idle,working,done,blocked,error\
                  :kinds=permission,question,auth\\E\\\\\\E[?";
    assert!(probe.starts_with(answer), "{got:?}");
    assert!(probe.ends_with("c'"), "{got:?}");

    let mut client = Peer::attach(&server.path, 10.0);
    gate.release();
    client.frame_titled("REPLIED");
    let got = replies(&answers);
    assert_eq!(got["position"], "$'\\E[2;1'", "{got:?}");
    // Device attributes name a VT100-family terminal: ESC [ ? ... c.
    assert!(got["attributes"].starts_with("$'\\E[?"), "{got:?}");
    gate.release();
    client.exited(6);
    drop(client);

    server.passed(6, 10.0);
}
