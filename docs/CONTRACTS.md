# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Partially proved`
names an exact verified slice and its remaining gap. `Proven` requires an exact
check, component revisions, platform, and artifact.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Planned | None |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Proven | `3a21fb7cd0f91bcc3fb7299d9a3d7287af5fef23`; manifest proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Partially proved | `3a21fb7cd0f91bcc3fb7299d9a3d7287af5fef23` proves stable identity and path rejection; runtime gap below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Planned | None |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |

## Proof record

### Manifest proof `3a21fb7cd0f91bcc3fb7299d9a3d7287af5fef23`

- Contracts: EON-C2 is proved. EON-C3 is proved only for stable component
  identity, relative artifact declarations, semantic launch inputs, and
  rejection of persisted Nix store paths.
- Artifact: `components/eon-alpha-v1.json`, SHA-256
  `d8d3035b9b62d00267c6ea81dd40d402fd918ef5295071d220c0f1a100689faf`.
- Component revisions: Orbit `00b136318bea13e3f08d490468f069de6f6b9bd2`;
  Venus `2d36c72dc87ca5416e22d6afdc35c6ab4e2fb832`; Helix
  `7e6cd307d00783c16ad4cff99ed71936d34f6572`; Yazi
  `aa526434f00bb44e2e902d9a4ac5f810da1018b9`; Ratconfig
  `e6ec2ebfe84b2358186410680cbcaf0564eb59a2`. The artifact records every
  accepted contract proof and the exact ORBF v1 / ORBS v2 compatibility edge.
- Environment: Linux 7.0.11 x86_64, Rust 1.96.0, Nix 2.34.7.
- Checks: `cargo fmt --check`; `cargo check --locked --workspace`;
  `cargo test --locked --workspace`; `cargo clippy --locked --workspace
  --all-targets -- -D warnings`; validator CLI on the artifact; Nix
  `builtins.fromJSON` assertions for schema, components, contracts, and
  interfaces; `git diff --check`.
- Remaining EON-C3 gap: no Eon runtime or Nix composition exists yet, so this
  proof does not cover resolved path injection or absence of Nix evaluation
  during normal operation. `astra-db8.3` owns that proof.

When a bead proves a contract, update its row with:

- the test or dogfood command;
- exact Git revisions and component manifest;
- operating system and architecture;
- produced artifact and checksum;
- limitations that remain inside the contract.

Do not change a row to `Proven` from a design review, mock, or unexecuted test.

## Change rule

A contract change needs an explicit bead. Record the user-visible consequence,
affected owners, compatibility decision, test change, and rejected alternative.
If another repository owns the contract, land and verify the owner change before
Eon consumes it.
