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
also hides `/run/current-system` and `/run/wrappers`. The Apple Silicon dev
shell has an unproven macOS branch without namespaces: it refuses while an Eon
supervisor of the same user runs, gives the suites a private HOME, TMPDIR and
runtime root, and skips the Linux-only installer suite.

## Sources of truth

- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — naming, ownership, and sequencing;
- [`docs/CONTRACTS.md`](CONTRACTS.md) — indexed product contracts and proofs;
- [`docs/DISTRIBUTION.md`](DISTRIBUTION.md) — composition and release policy;
- [`docs/REFERENCES.md`](REFERENCES.md) — routed primary and comparable sources;
- [`docs/VISUAL-REFERENCES.md`](VISUAL-REFERENCES.md) — discovery-only visual references;
- [terminal memory benchmark](benchmarks/eon-zlf-2026-09-08.md) — the measured
  Eon, eonbase, Foot, and Ghostty comparison; and
- [`CHANGELOG.md`](../CHANGELOG.md) — accepted user-visible chronology.

## Checks

[`components/eon-v5.json`](../components/eon-v5.json) is the canonical
distribution-neutral component graph. Validate it with:

```sh
cargo run --locked -p eon-manifest -- components/eon-v5.json
```

Nix builds Orbit, Venus, Eon and eonbase from the in-tree crates with the root
`Cargo.lock`. A workspace component's path must hold the expected Cargo package
at its manifest version. Any in-tree change selects a new generation through
Eon's [source-tree digest](ARCHITECTURE.md#in-tree-package-selection); a Cargo
checkout also hashes its docs and assets. Cargo needs every workspace member,
so Nix rebuilds Orbit and Venus after edits under `eon/` or `runtime/` too.

`nix build .#manual-bundle` assembles the unpublished Linux direct candidate
from the relinked tree described below. Its [manual delivery
contract](MANUAL-BUNDLE.md) records verification, host prerequisites, native
proof and publication limits.
`nix/linux-bundle.nix` owns archive assembly: the tree, an `eon` entrypoint
linked to the tree's launcher, and the receipts. `provenance.json` names the
glibc floor, the excluded store paths and the substitutes, and
`closure-paths.txt` lists the store paths the tree ships.

`nix build .#installer` builds the separate static Rust `installer/` workspace.
Its seven focused tests exercise real archive boundaries, filesystem
selection/removal, timeout child cleanup and CLI environment fallback;
`eon-test` runs them with the workspace suites. `nix/installer.nix` builds/tests the native musl artifact
and rejects a dynamic bootstrap. Signed TLS delivery and native active-work
proofs remain with `eon-prove-thin-linux-bundle-installer-8r7n`.
The [installer contract](INSTALLER.md) records target tools and recovery.

`nix build --no-link .#checks.x86_64-linux.bundle-relink` builds the
namespace-free tree the direct candidate ships. The separate
dependency-free Rust `relink/` workspace copies what Eon reaches without glibc,
Mesa and Wayland, points program interpreters at the x86-64 psABI path, makes
store RUNPATH entries `$ORIGIN`-relative and store symlinks relative, and drops
Venus's RUNPATH so the host GPU driver shares the host's gcc runtime. It then
checks that layout and gates every ELF on the glibc floor and on finding each
NEEDED library in the tree or among the declared host libraries. A substitute
ships a rebuild under a closure path's name: ncurses, built with
`nix/ncurses-termios-speed.h` so it needs no glibc 2.42 symbol.
`nix/bundle-relink.nix` owns that policy. The crate's tests run in that build
and with `eon-test --manifest-path relink/Cargo.toml`.

That build also replaces each makeWrapper script in the tree with a copy of the
static launcher from the separate `launcher/` workspace. `flake.nix` defines each
wrapper environment once: makeWrapper renders it for the Nix packages, and the
launcher compiles it in and applies it relative to the tree's location.
`nix build --no-link .#checks.x86_64-linux.bundle-launch` runs the tree from two
locations under bwrap, which stands in for a host without Nix. The stand-in host
glibc is the only store path visible, and the check fails if the tree looks up
any other besides the compiled-in data paths the tree stands in for. It also
compares each launcher's environment with makeWrapper's. It needs unprivileged
user namespaces inside the Nix build sandbox.

Build both installed products with `nix build --no-link .#default .#eonbase`.

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

`nix run .#record-demo -- features` records the clips and stills behind
eonterm.com's Features page into `assets/demo/features/`, as web-encoded MP4
clips and WebP stills. The clips and the Eon Bar still show the live Codex and
Claude quota chips, so sign in to both first. The recorder reads those logins
in place through `CODEX_HOME` and `CLAUDE_CONFIG_DIR`. A Claude rate limit
hides or ages those chips, so re-record when it clears.
