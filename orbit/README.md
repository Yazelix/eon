# Eon Sessions

Eon Sessions contains Orbit, the runtime that keeps a terminal process alive
after its client disconnects. Orbit owns the PTY, libghostty terminal state,
and local attachment protocol. [Eon Desktop](https://github.com/Yazelix/eon-desktop)
renders its frames; [Eon](https://github.com/Yazelix/eon) composes both projects.

## Demo

[![Animated Eon demo showing two Orbit Sessions in a native Wayland window](https://raw.githubusercontent.com/Yazelix/eon/ffa06c6a414921a1a0570c3ea3c1e49b1a3210da/assets/demo/eon-demo.gif)](https://github.com/Yazelix/eon/blob/ffa06c6a414921a1a0570c3ea3c1e49b1a3210da/assets/demo/eon-demo.mp4)

[Watch the MP4](https://github.com/Yazelix/eon/blob/ffa06c6a414921a1a0570c3ea3c1e49b1a3210da/assets/demo/eon-demo.mp4) of the composed Linux alpha.

## Run locally

Orbit's server and diagnostic client are proved on x86_64 Linux and Apple
Silicon macOS. The composed Eon product remains Linux-only.

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
- [Session protocol](docs/PROTOCOL.md): exact wire versions and cross-repository contract.
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
