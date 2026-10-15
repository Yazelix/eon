# Development

Eon owns orchestration, product configuration, component selection, updates,
and distribution. Orbit owns persistent terminal state and attachment; Venus
owns native presentation and input. Yazelix Nova remains a separate
product line.

Enter the pinned x86_64-linux development environment with `nix develop`.
It provides Rust and Cargo, formatting and lint tools, Git, `br` 0.7.2,
`bv` 0.25.2, the pinned Ghostty source, Zig, native libraries and fonts that
Orbit and Venus build and test with, and `eon-test`. The Beads binaries come
from hash-verified release archives and retain their full upstream licenses
under `share/licenses/<tool>/LICENSE`, including the OpenAI/Anthropic rider.

One Cargo workspace holds Orbit (`orbit/`), Venus (`venus/`), Eon Runtime with
EONW (`runtime/`) and Eon (`eon/`). `eon/crates/eon` owns executable assembly
and defaults; `runtime/` owns the runtime mechanisms behind `run(Inputs)` and
the canonical EONW package. `eon/crates/eon/tests/workspace_control.rs`
exercises the real Eon binary, aliases and self-launches.

Run the test suites with `nix develop -c eon-test`. It fetches locked crates,
then runs `cargo test --locked --offline` in the bubblewrap sandbox that
`nix/test-sandbox.nix` defines, so it is safe from inside a live Eon terminal.
Without arguments it runs the workspace suites, then the installer's, stopping
at the first failing suite; with arguments it passes them to `cargo test`, for
example `eon-test -p yazelix-venus`. It needs unprivileged user namespaces and supports
x86_64 Linux only. It is proved on a non-NixOS host; on NixOS the hidden `/run`
also hides `/run/current-system` and `/run/wrappers`.

## Sources of truth

- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — naming, ownership, and sequencing;
- [`docs/CONTRACTS.md`](CONTRACTS.md) — indexed product contracts and proofs;
- [`docs/DISTRIBUTION.md`](DISTRIBUTION.md) — composition and release policy;
- [`docs/REFERENCES.md`](REFERENCES.md) — routed primary and comparable sources;
- [`docs/VISUAL-REFERENCES.md`](VISUAL-REFERENCES.md) — discovery-only visual references;
- [terminal memory benchmark](benchmarks/eon-zlf-2026-09-08.md) — the measured
  Eon, EonTerm, Foot, and Ghostty comparison; and
- [`CHANGELOG.md`](../CHANGELOG.md) — accepted user-visible chronology.

## Checks

[`components/eon-alpha-v3.json`](../components/eon-alpha-v3.json) is the canonical
distribution-neutral component graph. Validate it with:

```sh
cargo run --locked -p eon-manifest -- components/eon-alpha-v3.json
```

Nix builds Orbit, Venus, Eon and EonTerm from the in-tree crates with the root
`Cargo.lock`. A workspace component's path must hold the expected Cargo package
at its manifest version. Any in-tree change selects a new generation through
Eon's [source-tree digest](ARCHITECTURE.md#in-tree-package-selection); a Cargo
checkout also hashes its docs and assets. Cargo needs every workspace member,
so Nix rebuilds Orbit and Venus after edits under `eon/` or `runtime/` too.

`nix build .#manual-bundle` assembles the unpublished Linux direct candidate
from the same package. Its [manual delivery contract](MANUAL-BUNDLE.md) records
verification, host prerequisites, native proof and publication limits.
`bundle/` is a separate dependency-free Rust workspace for the static Linux
entrypoint; `nix/linux-bundle.nix` owns archive assembly. Package builds check
static bootstraps; the retained assert-based native probe covers namespace
relocation outside Nix's build sandbox.

`nix build .#installer` builds the separate static Rust `installer/` workspace.
Its seven focused tests exercise real archive boundaries, filesystem
selection/removal, timeout child cleanup and CLI environment fallback;
`eon-test` runs them with the workspace suites. `nix/installer.nix` builds/tests the native musl artifact
and rejects a dynamic bootstrap. Signed TLS delivery and native active-work
proofs remain with `eon-prove-thin-linux-bundle-installer-8r7n`.
The [installer contract](INSTALLER.md) records target tools and recovery.

Build both installed products with `nix build --no-link .#default .#eonterm`.

Linux package checks use the sandbox's executable `/dev/shm` for ephemeral test
state, keeping lifecycle records and sockets off the builder's disk journal.
Sources and compilation stay in the ordinary build directory.

`nix build --no-link .#checks.x86_64-linux.helix-grammars` runs the pinned
Helix source-delivery check. It rejects evaluation-time Codeberg and SourceHut
fetches and compiles all ten bundled grammars through Helix's production builder.

Use Beads for implementation plans and deferred decisions:

```sh
bv --robot-triage
br ready
br show <id>
```

## Demo media

From the repository root, `nix run .#record-demo` records a scripted Eon
workspace on an isolated Wayland display. Install Codex CLI and sign in with
`codex login` first; the Agent popup uses that login for a fresh, read-only
session. The capture uses a soft focus wallpaper behind translucent terminals
and writes the GIF, MP4, and PNG poster to `assets/demo/`.
