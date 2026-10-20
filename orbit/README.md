# Orbit

Orbit is Eon's terminal-session engine: the runtime that keeps a terminal
process alive after its client disconnects. Orbit owns the PTY, libghostty
terminal state, and local attachment protocol. [Venus](../venus/README.md)
renders its frames; [Eon](../README.md) composes both.

Orbit runs as two processes built and shipped together. A Rust host owns the
PTY, the sockets, libghostty and the event loop, and a Bend worker decides
every answer as a pure step whose laws [PROOF.bend](PROOF.bend) proves
([host and worker](docs/HOST.md)).

## Demo

[![Animated Eon demo showing two Orbit Sessions in a native Wayland window](../assets/demo/eon-demo.gif)](../assets/demo/eon-demo.mp4)

[Watch the MP4](../assets/demo/eon-demo.mp4) of Eon on Linux.

## Run locally

Orbit's server is proved on x86_64 Linux; [ORB-C14](docs/CONTRACTS.md) records
Apple Silicon macOS. The composed Eon product remains Linux-only. Inside
`nix develop`, which supplies the Bend compiler, bun and clang that build the
worker:

```sh
tic -x terminfo/eon.terminfo
cargo run --locked -- serve
```

[Venus](../venus/README.md) attaches to the Session; Orbit has no client of its
own. The server keeps the Session alive while clients detach and reconnect. The
default private socket is `$XDG_RUNTIME_DIR/yazelix-orbit/orbit.sock`, with a
per-user `/tmp` fallback. [Session lifecycle](docs/LIFECYCLE.md) explains
management, Stop, and shutdown limits.

## Read more

- [Behavior](docs/BEHAVIOR.md): what Orbit does, claim by claim, with the check or law that covers each.
- [Host and worker](docs/HOST.md): the fact and decision protocol between the Rust host and the Bend worker.
- [Writing Bend](docs/BEND.md): practices for the worker and the laws.
- [Terminal state and attachment](docs/TERMINAL.md): libghostty, frames, selection, and scrollback.
- [Session protocol](docs/PROTOCOL.md): exact wire versions and consumer contract.
- [Development and performance checks](docs/DEVELOPMENT.md): local checks, benchmark, and CI.
- [Contract index](docs/CONTRACTS.md): accepted behavior, revisions, proofs, and gaps.
- [Rationale](docs/RATIONALE.md), [references](docs/REFERENCES.md), and [crate decisions](docs/CRATES.md): design evidence.

## License

The copyright holder offers the project-owned source history under
[Apache-2.0](LICENSE).

## LOC scorecard

| Surface | Lines |
| --- | ---: |
| Rust host and mechanisms, with the generated boundary | 7,317 |
| Bend worker, with the generated boundary | 3,470 |
| Host and worker boundary schema | 414 |
| Bend laws and proofs (`LAWS.bend`, `PROOF.bend`) | 2,541 |
| Canonical protocol crate and tests | 5,276 |
| Real-PTY checks and the lifecycle suite | 7,402 |
| Rust tools: boundary generator, proof generator, headless throughput | 2,863 |
| Governance Rust tool and tests | 901 |
| Shell child scripts for the real-PTY checks | 366 |
| Manual performance harness | 647 |
| Eon terminfo source | 2 |
| **Total owned Rust** | **23,759** |
| **Total owned Bend** | **6,011** |
| **Total owned implementation source** | **31,199** |

`proof/` holds a further 174,578 lines of per-fact lemmas that
[tools/step-proof.rs](tools/step-proof.rs) generates; they are not counted.
