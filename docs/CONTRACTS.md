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
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, traverse the topology directly, and have ended Sessions leave no dead pane or empty tab behind | Eon | Proven | Composition `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; Session-exit pruning `7ede475992528be1b6643035abe4da9560d50a21` |
| EON-C9 | Eon supplies one configurable exact managed environment across Nushell, Bash, Zsh, and Fish while native shell and tool configuration remain user-owned | Eon | Proven | `6dfcb473beccadd6e145235009240c81dd570fe5`; proof below |
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
- Result: Eon selects exact Nushell, Bash, Zsh, Fish, Starship, Zoxide, Atuin,
  Carapace, Helix, Yazi, and LazyGit artifacts. Stable `eon-*` commands expose
  managed shells and product tools outside Eon without shadowing the user
  toolchain. One child-private PATH exposes accepted unprefixed executable
  names in Sessions and managed shell launchers. The config may define one
  direct argv command and independent Starship, Zoxide, Atuin, and Carapace
  booleans. Defaults are `command = ["eon-nu"]` and `true` for every integration.
  Settings are read for each new Session and never rewrite a running shell.

  Managed `eon-nu`, `eon-bash`, `eon-zsh`, and `eon-fish` load normal native
  user configuration before bounded Eon activation. Each enabled integration
  uses the pinned executable and preserves an existing user prompt, external
  completer, or same-tool hook. A disabled integration suppresses only Eon's
  activation. Nushell's native user autoload remains last; managed Nushell
  suppresses its stock startup banner. Starship retains normal
  `~/.config/starship.toml` discovery because Eon does not set
  `STARSHIP_CONFIG`. Atuin honors `ATUIN_NOBIND` and retains ownership of its
  account, sync, import, daemon, pty proxy, AI, configuration, and data policy.
- Important failures: an invalid or unreadable Eon configuration, empty command,
  missing artifact, manifest mismatch, or shell launch failure is explicit and
  starts no replacement Session. Optional integration initialization reports a
  bounded warning without preventing the shell from opening. Eon never mutates
  global user configuration, shell startup files, aliases, or PATH, and does not
  redirect Session child processes through its private XDG configuration root.
- Ownership: Eon owns component selection, exact versions, the five shell
  settings, launch policy, its private configuration root, managed command
  names, Session PATH projection, guarded fallback activation, and identity
  reporting. Each shell owns native configuration and startup semantics;
  Starship, Zoxide, Atuin, and Carapace own their behavior, schemas, and state.
  Each other selected tool retains its native behavior and configuration
  schema. Orbit retains process, PTY, terminal-state, and Session authority;
  Venus owns no shell or tool policy.
- Boundary: the accepted slice adds no global alias mode, user-file mutation,
  Eon prompt or completion schema, shell framework, automatic Direnv or Mise
  activation, history import or sync policy, plugin or MCP surface, declarative
  profile, distribution channel, remote behavior, or editor replacement.
  Arbitrary direct argv commands remain unmanaged, and `eon run -- COMMAND...`
  remains the explicit child-command escape hatch.
- Approval: the original managed environment was approved on 2026-08-08;
  native configuration and guarded defaults were approved on 2026-08-10; the
  four-shell command and integration settings above were explicitly approved on
  2026-08-10.

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

- Proof revision: `1ffd6b3c852fc6420d265d00fd4598854d1fb10b` on x86_64
  Linux through the Nix alpha package `eon-0.1.0` at
  `/nix/store/syjk4m39vxi51263zab65x7ff0p5vbcf-eon-0.1.0`, NAR hash
  `sha256-zX5Y+wkVaiclq2SiGgw06EHBshCM4xEhu+L3ZGRtp4k=`.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `d2a3d2de2984dd7080f33df728b31ee50d0088cf5d6ef0189215c58ada19f35f`.
  It selects Orbit source `64db581445bafca1a08a6530f8e44f9c1edbc169`,
  ORBF v1 and ORBS v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`,
  ORB-C10 proof `292b2451c9a1d99390334771a681c7f481c996f2`, and Venus source
  `6ef19afddaedcbe2b9ed0996e31bc02df5d854ac` with VEN-C1 proof
  `e7bda96822274727faacb51731ae19181295e1cd`, VEN-C3 and VEN-C4 proof at
  the selected revision, and VEN-C8 proof `33a3d9af9f4c6015301ad6829fe733413c5b683d`.
- Checks: canonical manifest validation; locked Rust format, check, 21-test, and
  clippy suites; exact Nix build with 62 Venus tests; flake evaluation; installed
  version and store-path comparison; live-process survival; and
  `git diff --check`.
- Exercised behavior: native Eon topology proof remains at
  `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; the selected Venus retains the
  last coherent frame and retries a lost selected live endpoint until a fresh
  frame arrives. The active profile selects this package without restarting
  the existing supervisor or Orbit Sessions.

### EON-C8 — Session-exit pruning

- Proof revision: `7ede475992528be1b6643035abe4da9560d50a21` on x86_64 Linux.
- Artifact: Nix alpha package `eon-0.1.0`, NAR hash
  `sha256:1948aw0vwgi8yyjkx18g0q9gdh30idzwfzrsddr84bzqkjh1j2fq`.
- Checks: locked Rust format, check, 19-test, and clippy suites; exact Nix build
  and flake evaluation; profile artifact comparison; and `git diff --check`.
- Exercised behavior: ended Sessions remove their panes without identity reuse;
  selection moves to the nearest survivor; empty tabs disappear; later Sessions
  survive the initial Session; and the final Session closes Venus and Eon.

### EON-C9 — managed environment

- Proof revision: `6dfcb473beccadd6e145235009240c81dd570fe5` on x86_64 Linux.
- Artifact: `/nix/store/5a8jksj398s2nxk8ylln61ly8iil0m6n-eon-0.1.0`
  on x86_64 Linux, NAR hash
  `sha256-KuB+bQHj7BkHhlN+lrdtiia+Iegu7qnXK9v76IdCZ/w=`, NAR size 1,629,928
  bytes, closure size 1,808,754,888 bytes.
- Checks: locked Rust format, check, 21-test workspace suite, and
  Clippy; exact Nix package build and flake check, including Bash/Zsh revision
  binding and the managed-Zsh no-state check; packaged wrapper checks for direct
  argv, unmanaged commands, native startup order, guarded user hooks,
  native interactive semantics, normal configuration paths, disabled
  integrations, `ATUIN_NOBIND`, nonfatal integration failures, and Fish state
  isolation; installed-license inspection; and active profile artifact
  comparison. The compiled TOML dependency excludes its unused writer. `git
  diff --check` is clean.
- Exercised behavior: managed Nushell, Bash, Zsh, and Fish load their native
  configuration before optional pinned integrations. Existing primary and right
  prompts, per-command completers, Zoxide hooks, and Atuin hooks survive while
  Carapace covers unclaimed commands. Nushell user autoload runs last and its
  stock banner stays hidden. Each false setting suppresses Eon's integration,
  `ATUIN_NOBIND` selects a binding-free Atuin initializer, noninteractive shell
  commands remain noninteractive, noninteractive Zsh restores native `ZDOTDIR`,
  Bash resolves `~/.bashrc` natively, Eon's Zsh completion fallback writes no
  dump file, Fish Carapace changes only the current PATH, and a Session PATH
  contains one Eon private-bin entry.

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
  no explicit pane, tab, or Session-stop action. Machine restart does not
  preserve undeclared process or layout state, and EONW v1 provides no event
  stream, remote transport, authorization layer, plugin surface, or durable
  restoration.
- Closing Venus detaches the client while Orbit and its PTY remain alive. Bare
  `eon` and `eon attach` reconnect through the running Eon supervisor.
