# Development

Eon owns orchestration, product configuration, component selection, updates,
and distribution. Eon Sessions owns persistent terminal state and attachment;
Eon Desktop owns native presentation and input. Yazelix Nova remains a separate
product line.

`crates/eon` owns executable assembly and defaults. The independent
[Eon Runtime](https://github.com/Yazelix/eon-runtime) repository owns the runtime
mechanisms behind `run(Inputs)` and the canonical EONW package. Eon pins both
packages separately; its workspace contains only Eon and `eon-manifest`.
`crates/eon/tests/workspace_control.rs` exercises the real Eon binary, aliases
and self-launches. Runtime and codec unit tests live with their producer.

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

The package-selection and source-preparation check preserves production pins:

```sh
nix build --impure --no-link --file nix/workspace-package-check.nix
```

This check consumes the selected immutable runtime, codec and Venus sources
and accepted codec proof. It rejects package/build/provenance drift and checks
separate runtime/codec records through the existing graph validator. Its lock
fixture uses production preparation, supporting equal or distinct source pins.
Both production consumers bind
[`nix/workspace-package.nix`](../nix/workspace-package.nix)'s returned codec path
before substitution. Nix prepares one codec identity even when Cargo resolves
identical packages at different runtime/codec Git revisions. Locked Rust checks
for that composition consume the prepared source; Nix remains the alpha's
sole composition channel.

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
