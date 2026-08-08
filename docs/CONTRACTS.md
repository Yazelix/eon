# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Partially proved`
names an exact verified slice and its remaining gap. `Proven` requires an exact
check, component revisions, platform, and artifact.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Proven | `532711a529216453375d341560168fcd9adca57b`; Linux composition proof below |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Proven | `c9e898c9ff889c2be4d4381a20676291bab65b49`; manifest proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Proven | `c9e898c9ff889c2be4d4381a20676291bab65b49` manifest proof and `532711a529216453375d341560168fcd9adca57b` Linux composition proof below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Proven | `532711a529216453375d341560168fcd9adca57b`; Linux composition proof below |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |

## Proof record

### Manifest proof `c9e898c9ff889c2be4d4381a20676291bab65b49`

- Contracts: EON-C2 is proved. EON-C3 is proved only for stable component
  identity, named HTTPS Git sources, relative artifacts, semantic launch inputs, and
  rejection of persisted Nix store paths and NUL-containing strings.
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
  interfaces; exact diagnostics for revision-mismatch, malformed-source, store-path, null-OID, and NUL rejection; `git diff --check`.
- This proof covers the distribution-neutral identity layer. The Linux
  composition proof covers resolved path injection and runtime behavior.

### Linux composition proof `532711a529216453375d341560168fcd9adca57b`

- Contracts: EON-C1, EON-C3, and EON-C4 are proved for the Nix-only x86_64
  Linux alpha. This proof changes no agent-provider or recovery contract.
- Artifact: `/nix/store/g0mpnjxn3d5mnk31ll1q04zqljqdz8x9-eon-0.1.0`,
  NAR hash `sha256-pwEBPAspkxPEiO4LaWbD8zV+J0hDjnT7pypO25JFXyw=`, NAR size
  2,417,032 bytes, closure size 424,719,896 bytes. The exact Git revision build
  produced this path through the locked flake.
- Desktop icon: `assets/eon.png`, SHA-256
  `00d3e16cf2f0f732ad2b6c9c3098db702e839396a804d50054e87cdca5456580`.
  The package installs the exact PNG as the hicolor `eon` application icon and
  matches its desktop entry to Venus's `yazelix-venus` X11/Xwayland WM class.
- Component graph: manifest SHA-256
  `d8d3035b9b62d00267c6ea81dd40d402fd918ef5295071d220c0f1a100689faf`;
  Orbit `00b136318bea13e3f08d490468f069de6f6b9bd2`; Venus
  `2d36c72dc87ca5416e22d6afdc35c6ab4e2fb832`; Helix
  `7e6cd307d00783c16ad4cff99ed71936d34f6572`; Yazi
  `aa526434f00bb44e2e902d9a4ac5f810da1018b9`; Ratconfig
  `e6ec2ebfe84b2358186410680cbcaf0564eb59a2`.
- Environment: Linux 7.0.11 x86_64, Rust 1.96.0, Nix 2.34.7. The pinned
  Nixpkgs build uses Rust 1.95.0.
- Checks: `cargo fmt --all --check`; `cargo check --locked`; `cargo test
  --locked`; `cargo clippy --locked --all-targets -- -D warnings`; `nix flake
  check --no-build`; exact-revision `nix build`; clean temporary `nix profile
  install`; manifest version and configuration checks; private configuration
  creation without existing-permission mutation; relative-XDG fallback and
  unsafe-runtime rejection; Nix source-boundary inspection; `strace -f -e
  trace=execve` on `eon versions`; desktop-file validation; live X11 WM class
  matching; `git diff --check`. Nix ran the accepted Orbit, Venus, and Eon test
  suites.
- Lifecycle dogfood: the compiled supervisor is byte-identical to proof
  `6c9612913b5455b9fc5a03b33fbafe1d184b28ae`, and its normalized wrapper
  differs only by the artifact self path. That proof's Codex detach/reattach
  workload therefore still covers the unchanged runtime. A bounded launch of
  this exact artifact confirmed live `WM_CLASS="yazelix-venus"` matches its
  desktop entry.
- Build boundary: the package source contains only Cargo metadata, Rust crates,
  and the canonical component manifest. The downstream documentation revision
  evaluates to the same package derivation.
- Preserved child evidence: the exact Orbit and Venus artifacts did not change.
  Their accepted terminal checks covered 3,000 primary-history rows, rapid unit
  wheel input, byte-producing-key return to live, wrapped Unicode,
  wide-grapheme Shift-drag selection and explicit copy, resize and reflow,
  Helix, Neovim, and Yazi alternate screens, Yazi mouse input, DEC 1007 bytes,
  Venus loss, and reattachment.
- Limits: the alpha provides one live local session. Closing Venus detaches it;
  `eon attach` reconnects. Orbit restart, Eon relaunch, and machine restart
  preserve no undeclared process state. Three synthetic wheel events with delta
  80 disconnected Venus; rapid unit-delta input passed, Orbit and the PTY
  survived, and a fresh Venus reattached. Some emoji and Yazi icon glyphs
  rendered as fallback boxes in the packaged font configuration.

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
