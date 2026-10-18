# Orbit

Orbit is Eon's terminal-session engine: the runtime that keeps a terminal
process alive after its client disconnects. Orbit owns the PTY, libghostty
terminal state, and local attachment protocol. [Venus](../venus/README.md)
renders its frames; [Eon](../README.md) composes both.

## Demo

[![Animated Eon demo showing two Orbit Sessions in a native Wayland window](../assets/demo/eon-demo.gif)](../assets/demo/eon-demo.mp4)

[Watch the MP4](../assets/demo/eon-demo.mp4) of Eon on Linux.

## Run locally

Orbit's server is proved on x86_64 Linux and natively on Apple Silicon macOS.
Its diagnostic client is proved on Linux; its macOS proof awaits re-proof in
this repository. The composed Eon product remains Linux-only.

```sh
tic -x terminfo/eon.terminfo
cargo run --locked -- serve
```

In another terminal:

```sh
cargo run --locked -- client
```

The client can detach and reconnect while the server keeps the Session alive.
The default private socket is `$XDG_RUNTIME_DIR/yazelix-orbit/orbit.sock`, with a
per-user `/tmp` fallback. [Session lifecycle](docs/LIFECYCLE.md) explains
management, Stop, and shutdown limits.

## Read more

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
| Product Rust source and tests | 15,227 |
| Governance Rust tool and tests | 767 |
| Eon terminfo source | 2 |
| Manual performance harness | 647 |
| Shell test fixtures | 104 |
| **Total owned Rust** | **15,994** |
| **Total owned implementation source** | **16,747** |
