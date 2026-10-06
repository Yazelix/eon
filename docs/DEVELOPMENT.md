# Development

Eon owns orchestration, product configuration, component selection, updates,
and distribution. Eon Sessions owns persistent terminal state and attachment;
Eon Desktop owns native presentation and input. Yazelix Nova remains a separate
product line.

`crates/eon` owns executable assembly and defaults; `crates/eon-runtime` owns
the cohesive runtime mechanisms behind `run(Inputs)`. Runtime unit tests use
test-only inputs. `crates/eon/tests/workspace_control.rs` exercises the real Eon
binary, aliases and self-launches. EONW remains in `crates/eon-workspace-protocol`.
The local library is the checkpoint before independent runtime transfer.

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
