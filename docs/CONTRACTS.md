# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Candidate` means
the working tree is implemented and verified but has no proof-bearing Git
revision. `Partially proved` names an exact verified slice and its remaining
gap. `Proven` requires an exact check, component revisions, platform, and
artifact.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Proven | `712c0834691c58b07e56494de25f98670b6a882e`; selected-icon proof and preserved Linux composition proof below |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Proven | `c9e898c9ff889c2be4d4381a20676291bab65b49`; manifest proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Proven | `c9e898c9ff889c2be4d4381a20676291bab65b49` manifest proof and `eae70e8d3ed4348b389f320dfd49d7db29478546` Linux composition proof below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Proven | `eae70e8d3ed4348b389f320dfd49d7db29478546`; Linux composition proof below |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, and traverse the topology directly through Eon-owned semantic actions | Eon | Partially proved | `1a00a9f6de171e944c3c31a9f72ee2cc92d8fa15`; workspace-owner proof below |
| EON-C9 | Eon supplies one exact managed interactive environment through prefixed external commands and Session-private unprefixed tool names without changing the user's global toolchain | Eon | Proven | `99c410a1081b88dd8db2b7f9e26394a38acd175a`; managed-environment proof below |
| EON-C10 | A local client can submit versioned Eon workspace actions and receive one complete accepted workspace snapshot without reconstructing topology or terminal state | Eon | Candidate | Working tree; EONW v1 producer candidate below |

## Approved workspace contract EON-C8

- Consumer: one local Eon user, the canonical `eon` action projection, and a
  separately approved Venus consumer.
- Trigger: the user creates or focuses a tab or pane, or invokes direct left,
  right, up, or down workspace traversal.
- Result: Eon owns one live ordered topology of stable tab and pane identities
  and explicit mappings from panes to independent Orbit session identities.
  Tabs form the horizontal axis; each tab contains one ordered vertical pane
  stack. Eon selects one pane in the active tab, and Venus materializes that
  selection as the only expanded accordion pane.
- Important failures: an unknown or stale identity, unavailable Orbit session,
  invalid transition, or direction without a target returns a bounded explicit
  failure and leaves the last accepted topology unchanged. Removing or losing a
  view never silently stops or substitutes another Orbit session.
- Ownership: Eon owns topology, selection, mappings, and semantic workspace
  actions. Orbit remains the sole owner of each process, PTY, terminal state,
  and session lifetime. Venus owns native geometry, rendering, focus,
  accessibility, hit testing, and accordion materialization without
  reconstructing Eon state.
- Boundary: the first slice adds no arbitrary split tree, picker-based ordinary
  traversal, simultaneous expanded panes, durable layout restoration, AgentRun
  or provider semantics, terminal observation, managed tool defaults, plugin or
  MCP surface, isolation target, remote access, or appearance effect.
- Approval: explicitly approved by the user on 2026-08-08. The Eon-owned model,
  supervisor, and CLI action projection are partially proved. Native Venus
  materialization and cross-repository dogfood remain unproved.

## Approved managed-environment contract EON-C9

- Consumer: one local Eon user and subprocesses or agents launched inside an
  Eon Session.
- Trigger: the user starts a default interactive Session, invokes an
  Eon-managed command outside Eon, or resolves a managed tool from inside an
  Eon Session.
- Result: Eon selects exact Nushell, Starship, Zoxide, Helix, Yazi, and LazyGit
  artifacts. Stable `eon-*` commands expose them outside Eon without shadowing
  the user toolchain. One Session-private PATH exposes the accepted unprefixed
  executable names to processes inside Eon, managed Nushell supplies `lg`, and
  the default no-command Session starts managed Nushell.
- Important failures: a missing artifact, invalid private native
  configuration, manifest mismatch, or launch failure returns a bounded
  explicit error. Eon never mutates global user configuration, shell startup
  files, aliases, or PATH.
- Ownership: Eon owns component selection, exact versions, launch policy, its
  private configuration root, managed command names, Session PATH projection,
  and identity reporting. Each selected tool retains its native behavior and
  configuration schema. Orbit retains process, PTY, terminal-state, and
  Session authority; Venus owns no shell or tool policy.
- Boundary: the first slice adds no global alias mode, ornamental wrappers,
  shell framework, automatic Direnv or Mise activation, Carapace, plugin or
  MCP surface, declarative profile, distribution channel, remote behavior, or
  editor replacement. `eon run -- COMMAND...` remains the explicit child-command
  escape hatch.
- Approval: explicitly approved by the user on 2026-08-08 before implementation.

## Approved workspace protocol contract EON-C10

- Consumer: the canonical `eon` CLI and an independently released Venus client
  at an exact revision that explicitly consumes EONW v1.
- Trigger: a local client connects to Eon's private workspace endpoint and sends
  a versioned inspect request or one of EON-C8's accepted semantic actions.
- Result: the running Eon supervisor validates the request, remains the sole
  live action and topology owner, and returns one complete accepted snapshot.
  The snapshot carries stable ordered tab, pane, and Session identities; active
  and selected identities; Session liveness; and exact opaque Orbit endpoint
  bytes.
- Important failures: an unsupported version, malformed or oversized message,
  invalid snapshot shape, unavailable action, unknown identity, unrepresentable
  accepted state, or missing supervisor returns a bounded structured failure. A
  rejected action leaves the prior accepted workspace unchanged.
- Ownership: Eon owns EONW v1, the semantic action vocabulary, topology,
  mappings, acceptance, and complete snapshot. Orbit retains process, PTY,
  terminal-state, and Session authority. The CLI and Venus decode the same
  Eon-owned values and never mirror the schema or reconstruct hidden state.
- Boundary: EONW v1 reuses the private local Eon socket. Each connection carries
  one length-delimited request and response; neither depends on EOF to delimit a
  message. It adds no event stream, subscription or polling policy, remote
  transport, general plugin or MCP surface, authorization framework, durable
  restoration, AgentRun state, raw terminal content, or Venus rendering.
- Update order: prove the Eon producer first; update Venus against that exact
  revision; only then update Eon's canonical component requirement and explicit
  Venus launch input against the exact consumer revision. The currently selected
  Venus revision remains unchanged until it actually consumes EONW.
- Approval: explicitly approved by the user on 2026-08-09 by directing Eon to
  fix the named versioned workspace snapshot/action prerequisite.

## Proof record

### EONW v1 producer candidate

- Contract: EON-C10 is implemented and mechanically verified in the working
  tree. It remains a candidate until an exact proof-bearing Git revision exists.
- Owner: `eon-workspace-protocol` is the only EONW v1 schema and codec owner.
  It defines dependency-free bounded framing, semantic workspace actions,
  complete snapshots, exact opaque endpoint bytes, structured failures, and
  validation of version, size, identities, ordering references, uniqueness,
  liveness tags, and trailing data.
- Runtime: the existing mode-`0600` `eon.sock` carries one length-delimited EONW
  request and response per connection without using EOF as a message delimiter.
  The CLI encodes the same shared actions, decodes the complete response, and
  then renders the preserved human or JSON projection. The supervisor alone
  calls `Workspace::dispatch`; every accepted action and inspect returns a
  complete snapshot.
- Checks: `cargo fmt --all --check`; `cargo check --locked --workspace`; `cargo
  test --locked --workspace` with 16 tests; `cargo clippy --locked --workspace
  --all-targets -- -D warnings`; canonical manifest validation; and `git diff
  --check`; plus `nix flake check --no-update-lock-file --print-build-logs
  path:.`. The real three-Session control test sends complete requests without
  half-closing the socket, decodes `unsupported-version`, then proves the
  accepted workspace is still available. A focused process test proves missing
  supervisors retain the structured JSON failure and exit-2 contract.
- Limits: this candidate adds no event stream, subscription, polling policy,
  remote transport, authorization layer, plugin surface, AgentRun state,
  persistence, terminal content, or Venus rendering. The canonical manifest
  still truthfully selects Venus `2d36c72dc87ca5416e22d6afdc35c6ab4e2fb832`
  with one Orbit socket. EONW graph registration waits for the producer proof;
  Venus requirement and launch wiring wait for its exact consumer proof.

### Managed-environment proof `99c410a1081b88dd8db2b7f9e26394a38acd175a`

- Contract: EON-C9 is proved. The revision preserves EON-C1 through EON-C4 and
  the partial EON-C8 owner slice without advancing their proof revisions.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `2a577d38d32196852a9131f1b139665325ef483a45c973f0debf95ef7e2e27b5`;
  Nushell 0.113.1 `7b7df4aa68e957cf38b9d8157c35fa7523f44a6d`, Starship
  1.25.1 `8758daa7767d4e73874330b1e262fca66a7ffd30`, Zoxide 0.9.9
  `9cdc6aa3740b4d8a9d62406c99e84c5de49645e9`, Helix 25.7.1
  `7e6cd307d00783c16ad4cff99ed71936d34f6572`, Yazi 26.5.6
  `aa526434f00bb44e2e902d9a4ac5f810da1018b9`, and LazyGit 0.62.2
  `009c8975beb322f9374789476ac65dfa02321fce`. `eon versions` reports
  these identities, and the package retains each upstream license.
- Command behavior: the package exposes only `eon-nu`, `eon-hx`, `eon-yazi`,
  `eon-ya`, `eon-lazygit`, and `eon-lg` for the managed set. Its private Session
  PATH exposes `nu`, `hx`, `yazi`, `ya`, and `lazygit` through the same Eon
  dispatcher, while managed Nushell owns an `lg` alias to the exact Eon LazyGit
  wrapper. Prefixed commands outside a Session retain the ambient PATH. A
  no-command Session starts `eon-nu`; `eon run -- COMMAND...` retains the exact
  explicit argv.
- Configuration behavior: managed commands receive one private root through
  both `EON_CONFIG_HOME` and `XDG_CONFIG_HOME`. A relative `EON_CONFIG_HOME` is
  resolved once to an absolute root before propagation. Nushell receives
  immutable native config and env files generated by the package; exact
  Starship and Zoxide binaries generate their own Nushell initialization.
  Managed Helix removes ambient `CARGO_MANIFEST_DIR`, `HELIX_RUNTIME`, and
  `HELIX_STEEL_CONFIG`; managed Yazi removes `YAZI_CONFIG_HOME`; managed LazyGit
  and `lg` remove `CONFIG_DIR` and `LG_CONFIG_FILE` and restrict
  `XDG_CONFIG_DIRS` to the private root. Real startup proved that all three
  resolve configuration beneath the Eon root, while explicit Helix and LazyGit
  configuration arguments still win. The violet `∴` marker without a leading
  blank line, native `z`, same-root nested `nu`, Session-private command lookup,
  ambient PATH retention outside Sessions, an untouched sentinel user config,
  unchanged parent PATH, and a newly created root with mode `0700` remain
  proved.
- Artifact: `/nix/store/4z0xppxivbwvd8yf5kzdrdlwivr07qxw-eon-0.1.0`,
  NAR hash `sha256-ScOx8I7EdPYPO9YXcyuU58mFguSG9CXmwiAI1sTAWa4=`, NAR size
  3,088,144 bytes, closure size 1,561,188,912 bytes across 380 paths. Against
  the exact proof-tree pre-environment artifact
  `/nix/store/13rpbajlpqskc66kz4jiqcw68pv18yxy-eon-0.1.0`, the managed slice
  adds 103,237,824 bytes (98.46 MiB, 7.08%); package glue beyond the approved
  dependencies adds 52,136 bytes. The configuration-precedence correction adds
  9,944 bytes and absolute-root resolution adds another 344 bytes over their
  respective preceding same-tree EON-C9 candidates; later Helix and
  `XDG_CONFIG_DIRS` hardening plus exact empty-override semantics reduce the
  measured candidate by 8 bytes.
- Process proof: real `eon run -- nu -c ...` launched the exact Orbit and Venus
  artifacts, ran managed Nushell inside the PTY, preserved the same private root
  through nested dispatch, and removed the Orbit socket on exit. Focused Rust
  checks prove bounded invocation names, exact managed argv/environment and
  native-override removal, one-time absolute resolution of a relative root,
  Session-only PATH projection, and default-versus-explicit Orbit child
  selection.
- Environment and checks: Linux 7.0.11 x86_64, Rust 1.96.0, Nix 2.34.7;
  `cargo fmt --all --check`; `cargo check --locked`; `cargo test --locked`;
  `cargo clippy --locked --all-targets -- -D warnings`; `nix flake check
  --no-update-lock-file --print-build-logs path:$PWD`; exact staged-tree package
  build and artifact inspection; real managed-shell and composed-runtime
  dogfood; `git diff --check`.
- Limits: the package and proof remain x86_64 Linux only. Child data and caches
  retain their native locations. LazyGit relies on an ambient Git executable,
  which the alpha installation prerequisite already supplies. The packaged
  DejaVu-only Venus font configuration may render some native Starship, emoji,
  or Yazi glyphs as fallback boxes. A native macOS bundle must replace Nix store
  injection and Unix symlinks with bundle-relative paths and links. Venus still
  does not materialize EON-C8 tabs or accordion panes.

### Workspace-owner proof `1a00a9f6de171e944c3c31a9f72ee2cc92d8fa15`

- Contract: EON-C8 is partially proved for the Eon-owned live topology,
  semantic actions, private control boundary, and independent child-process
  mapping. EON-C1 through EON-C4 and the component manifest are unchanged.
- Owner behavior: one foreground Eon supervisor owns stable tab, pane, and
  Session identities; horizontal tab and vertical pane order; active and
  selected identities; explicit Orbit endpoint mappings; and bounded semantic
  action results. Directions do not wrap. Rejected, malformed, duplicate, or
  unavailable actions leave the accepted topology unchanged.
- Process evidence: `second_cli_controls_three_live_sessions_without_owning_them`
  drove one live supervisor from separate `eon` processes, created two tabs and
  three distinct substitute Orbit processes/endpoints, traversed both axes,
  compared human and deterministic JSON projections, rejected a boundary move
  without mutation, dropped a control client, and observed all three children
  still alive. It also proved control-socket mode `0600` while live and removal
  on supervisor exit. `control_listener_preserves_a_replacement_socket` proved
  that cleanup leaves a replacement socket untouched. Focus never signaled a child.
- Consumed Orbit evidence: artifact source
  `00b136318bea13e3f08d490468f069de6f6b9bd2`, ORB-C1 and ORB-C3 proof
  `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`. Each pane starts one independent
  `serve` process with an explicit private endpoint; no Orbit protocol or
  multi-session daemon was added.
- Environment: Linux 7.0.11 x86_64, Rust 1.96.0, Nix 2.34.7.
- Checks at the proof commit: `cargo fmt --all --check`; `cargo check --locked`;
  `cargo test --locked`; `cargo clippy --locked --all-targets -- -D warnings`;
  `nix flake check --no-build`; `git diff --check`.
- Limits: topology is live only and bounded to 64 tabs and 256 panes. The
  private Unix control transport is not a public compatibility surface. This
  slice has no removal, stop, persistence, picker, arbitrary layout, defaults,
  plugin, provider, observation, remote, isolation, or appearance behavior.
  Venus still attaches only to the initial Orbit endpoint and does not yet
  render horizontal tabs or vertical accordion panes; native materialization,
  accessibility, interaction, and fresh composed dogfood remain required for
  full EON-C8 proof.

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

### Linux composition proof `eae70e8d3ed4348b389f320dfd49d7db29478546`

- Contracts: EON-C1, EON-C3, and EON-C4 are proved for the Nix-only x86_64
  Linux alpha. This proof changes no agent-provider or recovery contract.
- Artifact: `/nix/store/fqgiydas2k44zx4mzhi2a63i7jhjg8qp-eon-0.1.0`,
  NAR hash `sha256-2R4PIprl6QQWCzVXto9NoI8gYWiiuZuUBqslBhNqB+g=`, NAR size
  2,417,208 bytes, closure size 1,457,326,784 bytes. The exact Git revision build
  produced this path through the locked flake.
- Desktop icon: `assets/eon.png`, SHA-256
  `00d3e16cf2f0f732ad2b6c9c3098db702e839396a804d50054e87cdca5456580`.
  The package installs the exact PNG as the hicolor `eon` application icon and
  matches its desktop entry to Venus's `yazelix-venus` X11/Xwayland WM class.
- Graphics runtime: the Venus wrapper exposes pinned Mesa 26.1.2 through the
  Vulkan loader's standard XDG driver-discovery path without shell-only variables.
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
  matching; bare-launch reattachment from a desktop-like environment without
  Vulkan overrides; `git diff --check`. Nix ran the accepted Orbit, Venus, and
  Eon test suites.
- Lifecycle dogfood: this exact artifact launched one Orbit and a bounded
  long-running process. Killing Venus left Eon, Orbit, and the process alive;
  a second bare `eon` launched Venus against the same socket with exactly one
  Orbit server. The earlier Codex detach/reattach evidence still covers the
  unchanged explicit `eon run` and `eon attach` paths.
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
  bare `eon` or `eon attach` reconnects. Orbit restart and machine restart
  preserve no undeclared process state. Three synthetic wheel events with delta
  80 disconnected Venus; rapid unit-delta input passed, Orbit and the PTY
  survived, and a fresh Venus reattached. Some emoji and Yazi icon glyphs
  rendered as fallback boxes in the packaged font configuration. Packaged Mesa
  is proved on Intel Raptor Lake graphics; proprietary NVIDIA remains unproved.

### Selected desktop icon proof `712c0834691c58b07e56494de25f98670b6a882e`

- Contract: EON-C1 advances only the packaged desktop-icon identity. EON-C2,
  EON-C3, EON-C4, runtime behavior, component revisions, and compatibility remain
  proved by their earlier revisions above.
- Artifact: `/nix/store/5v2i3wp7q52zmnl5p4y9m68wwrfmclsq-eon-0.1.0`,
  NAR hash `sha256-otl7TOldLCDVHbiNfYjNHJoPx0P6JBCfyJJzcZz6p98=`, NAR size
  2,973,512 bytes, closure size 1,457,883,088 bytes.
- Desktop icon: `assets/eon.png`, a 1254 by 1254 RGBA PNG with SHA-256
  `540d33604b703b69e80531d228d2bef9887ec561954c0ceecd59c2b9b511a9b7`.
  The installed icon is byte-identical, and the desktop entry retains `Name=Eon`,
  `Icon=eon`, and `StartupWMClass=yazelix-venus`.
- Checks: canonical and installed icon byte/hash equality; image identity;
  README target; locked Cargo format, check, test, manifest, and clippy commands;
  `nix flake check --no-build`; exact package build; desktop-file validation;
  live COSMIC launcher inspection; and `git diff --check`.

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
