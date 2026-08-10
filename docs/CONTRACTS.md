# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Candidate` means
the working tree is implemented and verified but has no proof-bearing Git
revision. `Partially proved` names an exact verified slice and its remaining
gap. `Proven` requires an exact check, component revisions, platform, and
artifact.

This index describes only the accepted current state. Owning Beads retain
detailed execution evidence, and Git history retains superseded states.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; current composition proof below |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; current composition proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; current composition proof below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; current composition proof below |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, traverse the topology directly, and have ended Sessions leave no dead pane or empty tab behind | Eon | Candidate | Working tree; locked Rust checks and Nix build passed; proof revision pending |
| EON-C9 | Eon supplies one exact managed interactive environment through prefixed external commands and Session-private unprefixed tool names without changing the user's global toolchain | Eon | Proven | `99c410a1081b88dd8db2b7f9e26394a38acd175a`; managed-environment proof below |
| EON-C10 | A local client can submit versioned Eon workspace actions and receive one complete accepted workspace snapshot without reconstructing topology or terminal state | Eon | Proven | `4af395aea06c230ee6b18cf0755ae25915c0b88d`; EONW v1 producer proof below |

## Approved workspace contract EON-C8

- Consumer: one local Eon user, the canonical `eon` action projection, and a
  separately approved Venus consumer.
- Trigger: the user creates or focuses a tab or pane, or invokes direct left,
  right, up, or down workspace traversal.
- Result: Eon owns one live ordered topology of stable tab and pane identities
  and explicit mappings from panes to independent Orbit Session identities.
  Tabs form the horizontal axis; each tab contains one ordered vertical pane
  stack. Eon selects one pane in the active tab, and Venus materializes that
  selection as the only expanded accordion pane. When a Session exits, Eon
  removes its pane. If that empties a tab, Eon removes the tab; if it empties
  the workspace, Eon closes Venus and the supervisor. Selection stays on the
  same identity when possible, otherwise moves to the following sibling at the
  removed index or the preceding sibling when no following sibling exists.
- Important failures: an unknown or stale identity, invalid transition, or
  direction without a target returns a bounded explicit failure and leaves the
  last accepted topology unchanged. An unknown or repeated Session-exit
  notification removes nothing. Removing or losing a view never silently stops
  or substitutes another live Orbit Session.
- Ownership: Eon owns topology, selection, mappings, and semantic workspace
  actions. Orbit remains the sole owner of each process, PTY, terminal state,
  and Session lifetime. Venus owns native geometry, rendering, focus,
  accessibility, hit testing, and accordion materialization without
  reconstructing Eon state.
- Boundary: the accepted slice adds no arbitrary split tree, picker-based
  ordinary traversal, simultaneous expanded panes, durable layout restoration,
  AgentRun or provider semantics, terminal observation, managed-tool defaults,
  explicit close or stop action, exit-status persistence, restart or reopen
  action, plugin or MCP surface, isolation target, remote access, or appearance
  effect.
- Approval: topology was approved on 2026-08-08; Session-exit pruning was
  approved on 2026-08-10. Eon composition `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`
  consumes Venus's direct navigation, creation, pane headers, and bounded refresh.

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
- Boundary: the accepted slice adds no global alias mode, ornamental wrappers,
  shell framework, automatic Direnv or Mise activation, Carapace, plugin or MCP
  surface, declarative profile, distribution channel, remote behavior, or
  editor replacement. `eon run -- COMMAND...` remains the explicit
  child-command escape hatch.
- Approval: explicitly approved by the user on 2026-08-08.

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
- Update order: producer `4af395aea06c230ee6b18cf0755ae25915c0b88d`
  precedes current consumer `33a3d9af9f4c6015301ad6829fe733413c5b683d`;
  composition `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2` selects the consumer and
  supplies its workspace socket.
- Approval: explicitly approved by the user on 2026-08-09.

## Approved desktop-icon identity under EON-C1

- Consumer: the Eon README, Linux desktop entry, launcher, dock, and app switcher.
- Trigger: a surface displays Eon's canonical application icon.
- Result: `assets/eon.png` is the transparent canonical master prepared from
  `assets/icon-concepts/icon_eon_4.png`. Nix derives exact hicolor raster sizes;
  other retained concepts are not runtime identities.
- Important failures: the master contains an opaque background or isolated
  off-palette edge pixels, a native launcher size gains a bright fringe, an
  installed raster differs from its build output, or desktop metadata stops
  naming the canonical `eon` icon.
- Ownership: Eon owns the canonical master and product selection. Nix owns only
  deterministic size derivation and installation from that master.
- Proof: source and package selection `32936b92e9c20a000d21f123869e0b749ff39618`;
  current master SHA-256
  `33ec3062f72a732290dcd6c6f40a2d5f535d6a7cbfcaf7455a0a5c4f03a52e0d`.
- Approval: the user selected icon 4 on 2026-08-09 after native-size inspection.

## Current accepted proofs

### EON-C1 through EON-C4 and the current EON-C8 topology — composition

- Proof revision: `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2` on x86_64
  Linux through the Nix alpha package `eon-0.1.0`, NAR hash
  `sha256-chr5FD0q429sO7zVg7CpYEBU0c4kzSZzmeCq44/kkC0=`.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `f73a8a00c6c9430eff7f8846746dff872f2957ce43e7692e9a03eca304a84312`.
  It selects Orbit source `64db581445bafca1a08a6530f8e44f9c1edbc169`,
  ORBF v1 and ORBS v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`,
  ORB-C10 proof `292b2451c9a1d99390334771a681c7f481c996f2`, and Venus
  source and VEN-C8 proof `33a3d9af9f4c6015301ad6829fe733413c5b683d`.
- Checks: canonical manifest validation; locked Rust format, check, 17-test, and
  clippy suites; exact Nix build and uncached flake evaluation; version report;
  native Linux/Wayland dogfood; and `git diff --check`.
- Exercised behavior: the package launches independent Orbit Sessions; creates
  tabs and panes; traverses tabs with Alt+H/L and panes with Alt+K/J; creates a
  pane with Alt+M and a tab with Ctrl+T; renders every fitting pane header around
  one selected body; and reflects external workspace actions within one second.

### EON-C9 — managed environment

- Proof revision: `99c410a1081b88dd8db2b7f9e26394a38acd175a` on x86_64 Linux.
- Artifact: the canonical manifest selects the exact managed tool revisions;
  `eon versions` reports them, and the Nix alpha package retains their licenses.
- Checks: locked Rust format, check, tests, and clippy; Nix flake evaluation and
  exact package build; real managed-shell and composed-runtime dogfood; native
  configuration-precedence and isolation checks; and `git diff --check`.
- Exercised behavior: prefixed managed commands preserve the ambient toolchain
  outside Eon; Session-private unprefixed commands and configuration remain
  rooted under Eon; explicit child argv and native configuration overrides win.

### EON-C10 — EONW v1 producer

- Proof revision: `4af395aea06c230ee6b18cf0755ae25915c0b88d` on x86_64 Linux.
- Artifact: the dependency-free `eon-workspace-protocol` crate and the Eon Nix
  alpha package built from that exact revision.
- Checks: locked workspace format, check, 16-test, and clippy suites; canonical
  manifest validation; Nix flake check; real three-Session control and process
  failure tests; and `git diff --check`.
- Exercised behavior: one mode-`0600` control socket carries bounded complete
  requests and responses; the shared codec rejects incompatible or malformed
  input; every accepted inspect or semantic action returns a complete snapshot;
  rejected input preserves the accepted workspace.

## Current gaps

- EON-C5 through EON-C7 remain planned. The accepted artifact is a Nix-only
  x86_64 Linux alpha; direct bundles and native signed and notarized macOS
  distribution are unproved.
- Packaged graphics are proved on Intel Mesa. Proprietary NVIDIA remains
  unproved, and the packaged font set can show fallback boxes for some emoji and
  native tool glyphs.
- Workspace topology is live only and bounded to 64 tabs and 256 panes. It has
  no explicit pane, tab, or Session-stop action. Automatic pruning after Session
  exit is implemented and mechanically verified in the working tree; its
  proof-bearing commit remains pending. Machine restart does not preserve
  undeclared process or layout state, and EONW v1 provides no event stream,
  remote transport, authorization layer, plugin surface, or durable restoration.
- Closing Venus detaches the client while Orbit and its PTY remain alive. Bare
  `eon` and `eon attach` reconnect through the running Eon supervisor.
