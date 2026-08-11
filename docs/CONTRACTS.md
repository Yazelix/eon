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
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Candidate | Accepted base `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Candidate | Accepted base `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Candidate | Accepted base `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Candidate | Accepted base `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, traverse the topology directly, and have ended Sessions leave no dead pane or empty tab behind | Eon | Candidate | Accepted composition `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; Session-exit pruning `7ede475992528be1b6643035abe4da9560d50a21`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C9 | Eon supplies one configurable exact managed environment across Nushell, Bash, Zsh, and Fish while native shell and tool configuration remain user-owned | Eon | Proven | Base `6dfcb473beccadd6e145235009240c81dd570fe5`; tool glyphs `fb95671d855fa944c3717103cb13bd0135f8aec8`; proof below |
| EON-C10 | A local client can submit versioned Eon workspace and supervisor-lifecycle actions and receive one complete typed result without reconstructing hidden state | Eon | Candidate | Accepted workspace actions `4af395aea06c230ee6b18cf0755ae25915c0b88d` and lifecycle actions `fd6b348494111a0d18e241787da14ea99ee117a9`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C11 | Eon defaults to the exact current runtime generation while older live generations remain discoverable, explicitly presentable when they advertise the compatible presentation lifecycle, and explicitly stoppable through their supervisor | Eon | Candidate | Accepted base `3b84d83d807c6249efa340fabf9e3d0d0d3ef310` and attachment `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |
| EON-C12 | A local user or composition can host one exact command in one native terminal surface without Eon workspace actions while Eon retains generation and lifecycle authority | Eon | Candidate | Accepted base `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; candidate `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2` |

## Approved terminal-host contract EON-C12

- Consumer: one local Eon user or an explicitly approved composition that needs
  one native terminal surface without Eon's tab and pane model.
- Trigger: the consumer invokes `eon terminal -- COMMAND...` with one non-empty
  exact argv. An optional `--no-decorations` before `--` requests a native
  surface without window-system decorations.
- Result: Eon starts one Orbit-owned Session for the exact command and launches
  Venus with only that Session endpoint. The supervisor remains the sole Venus
  process launcher: a repeated invocation preserves an attached surface, while
  an invocation after detachment opens one replacement against the same live
  Session and with the supervisor's original decoration choice. A later invocation
  does not mutate a live supervisor's choice. The current runtime generation
  remains authoritative for presentation, inspection, stop, concurrent-launch
  convergence, child exit, and cleanup.
- Important failures: a missing command, incompatible or corrupt supervisor,
  component mismatch, partial startup, or a live supervisor in the other launch
  mode fails explicitly. Workspace actions against terminal mode return the
  bounded `workspace-unavailable` failure and create no hidden topology.
- Ownership: Eon owns launch mode, generation, composition, and lifecycle.
  Orbit remains the sole owner of the process, PTY, terminal state, Session,
  and transport. Venus remains a transient native presentation and input client.
- Boundary: terminal mode adds no Workspace value, tabs, panes, key remapping,
  preset, wrapper process, persisted mode marker, remote access,
  second app identity, compositor focus guarantee, or Zellij policy. Bare `eon`
  and `eon run` retain their workspace meanings. Decorated surfaces remain the
  default for workspace mode and terminal mode.
- Approval: the user approved this contract and the exact command spelling on
  2026-08-10 for `eon-4is.1`.

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
  per-pane or per-Session close or stop action, exit-status persistence,
  restart or reopen action, plugin or MCP surface, isolation target, remote
  access, or appearance effect. EON-C11 separately owns explicit whole-
  generation stop.
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
  a versioned inspect request, one of EON-C8's accepted semantic actions, or an
  EON-C11 runtime identity or whole-generation stop action.
- Result: the running Eon supervisor validates the request, remains the sole
  live action and topology owner, and returns one complete typed result.
  Workspace results carry stable ordered tab, pane, and Session identities;
  active and selected identities; Session liveness; and exact opaque Orbit
  endpoint bytes. Lifecycle results carry the supervisor's stable generation,
  component report, protocol identity, exact live Session identities, and
  owner-authored attach and stop availability.
- Important failures: an unsupported version, malformed or oversized message,
  invalid result shape, unavailable action, unknown or mismatched generation,
  unrepresentable accepted state, missing supervisor, or an unconfirmed stop
  returns a bounded structured failure. A rejected action leaves the prior
  accepted workspace and every Session unchanged. An accepted stop names the
  affected live Sessions before the supervisor terminates only its own children.
- Ownership: Eon owns EONW v1, the semantic action vocabulary, topology,
  mappings, acceptance, and complete snapshot. Orbit retains process, PTY,
  terminal-state, and Session authority. The CLI and Venus decode the same
  Eon-owned values and never mirror the schema or reconstruct hidden state.
- Boundary: EONW v1 reuses the private local Eon socket. Its lifecycle tags are
  additive: the accepted Venus consumer continues to send only workspace tags
  and can never receive a lifecycle result. Each connection carries one length-
  delimited request and response; neither depends on EOF to delimit a message.
  It adds no event stream, subscription or polling policy, remote transport,
  general plugin or MCP surface, authorization framework, durable restoration,
  AgentRun state, raw terminal content, or Venus rendering.
- Update order: workspace producer `4af395aea06c230ee6b18cf0755ae25915c0b88d`
  precedes consumer `33a3d9af9f4c6015301ad6829fe733413c5b683d`;
  composition `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2` selects that consumer.
  Lifecycle producer `fd6b348494111a0d18e241787da14ea99ee117a9`
  retains the workspace response type consumed by pinned Venus.
- Approval: workspace actions were approved on 2026-08-09; additive runtime
  identity and stop actions were approved on 2026-08-10.

## Approved runtime-generation contract EON-C11

- Consumer: one local Eon user, the bare launcher, and the explicit generation
  inspection, attach, and stop commands.
- Trigger: Eon launches without a subcommand, or the user lists, attaches to, or
  stops one exact runtime generation.
- Result: Eon derives one opaque distribution-neutral identity from its runtime
  source, dependency lock, canonical component manifest, and EONW source. Each
  generation owns a separate private directory below Eon's runtime root. A bare
  launch attaches only to a live supervisor that authoritatively reports the
  exact current identity; otherwise it starts the current generation without
  stopping older work. Discovery deterministically distinguishes current,
  previous, legacy, dead, incompatible, and corrupt candidates. Explicit attach
  never substitutes another generation. Explicit stop is sent through the
  selected generation's EONW owner and names every affected live Session.
- Important failures: unsafe permissions, symlinks, invalid names, corrupt or
  oversized responses, unsupported EONW, identity mismatch, unavailable legacy
  lifecycle support, timeout, partial startup, or a concurrent launch produces
  a bounded explanation. Failure or cancelled confirmation preserves every
  Session. Eon removes only its own dead control socket or an empty stopped
  generation directory and never kills a process inferred from a PID, pathname,
  or process tree.
- Ownership: Eon owns generation identity, namespace selection, discovery,
  compatibility policy, supervisor-routed stop, and bounded cleanup. Each live
  supervisor remains authoritative for its topology and lifecycle response.
  Orbit retains process, PTY, terminal-state, and Session authority. Venus is a
  transient client of one explicitly compatible Eon/Orbit endpoint pair. A
  distributor injects artifacts but does not define runtime identity or policy.
- Boundary: the accepted slice adds no manager daemon, persisted topology,
  machine-restart recovery, live supervisor or PTY handoff, background updater,
  Nix evaluation, package-channel identity, compatibility window, remote
  runtime, plugin API, or automatic age/count eviction. A fixed-namespace legacy
  supervisor may be inspected and attached through accepted EONW v1, but reports
  generation identity and stop as unavailable.
- Checks: protocol round trips; deterministic identity, discovery, selection,
  validation, and cleanup tests; concurrent process and A-to-B upgrade tests;
  locked Rust and Nix checks; and live profile-upgrade dogfood that preserves the
  older workspace.
- Approval: explicitly approved by the user on 2026-08-10.

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

## Current proof state

### EON-C1 through EON-C4 and the current EON-C8 topology — composition

- Proof revision: `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c` on x86_64
  Linux through the Nix alpha package `eon-0.1.0` at
  `/nix/store/04sbq6wc79h2hf1rjq9y9bcvaz5v92hb-eon-0.1.0`, NAR hash
  `sha256-uSidJ5Xf047x0vxjiscmnAHlVZiIvm0ugN6mQldawpc=`.
- Current candidate: presentation-owner correction at
  `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2`, packaged at
  `/nix/store/slrmd85fpdf8sd8xxzyalakayy7hlar7-eon-0.1.0`, NAR hash
  `sha256-Vx8TX7FUqF91+vs+Gn0K7n5HW+rxxzLB4cPk7v7/Pmc=`.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `036992de61e047f76c1bd9ec56cd74483a6a5af0dab7971d3fb44f43e99566e7`.
  It selects Orbit source `64db581445bafca1a08a6530f8e44f9c1edbc169`,
  ORBF v1 and ORBS v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`,
  ORB-C10 proof `292b2451c9a1d99390334771a681c7f481c996f2`, and Venus source
  `90988f6ebcde68338e202a9c637c59398aafe93d` with VEN-C1 proof
  `e7bda96822274727faacb51731ae19181295e1cd`, VEN-C3 and VEN-C4 proof
  `6ef19afddaedcbe2b9ed0996e31bc02df5d854ac`, VEN-C8 proof
  `33a3d9af9f4c6015301ad6829fe733413c5b683d`, and VEN-C9 proof
  `90988f6ebcde68338e202a9c637c59398aafe93d`.
- Checks: canonical manifest validation; locked Rust format, check, 30-test, and
  Clippy suites; exact Nix build and flake check; installed version, commit, and
  store-path comparison; workspace and presentation-owner regression checks; README
  LOC checks; and `git diff --check`.
- Retained dogfood: native standalone input and live-process survival passed at
  `4353e94ebc015453c77914a2be013a950f49c8cc`; the profile refresh at this proof
  revision preserved the previous three-Session generation and legacy workspace.

### EON-C12 — single-surface terminal host

- Accepted base: `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c` and
  `/nix/store/04sbq6wc79h2hf1rjq9y9bcvaz5v92hb-eon-0.1.0` on x86_64 Linux.
- Current candidate: revision, artifact, and NAR hash above.
- Checks: deterministic workspace-versus-terminal Venus argv; real-process
  exact-command startup, active-surface preservation, detached-surface replacement,
  one-Session invariant,
  wrong-mode rejection, structured unavailable actions, partial-startup cleanup,
  concurrent launch, and command-exit cleanup.
- Native dogfood at `4353e94ebc015453c77914a2be013a950f49c8cc`: one COSMIC Wayland surface
  forwarded Alt-h/j/k/l/m and Ctrl-t as exact bytes `1b 68 1b 6a 1b 6b 1b 6c 1b 6d 14`. After the
  supervisor-owned client detached, a second invocation reopened the same
  Session; command exit closed both invocations, removed the generation, and
  left no Venus process.
- Remaining gap: this proof makes no macOS claim.

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
- Tool-glyph proof revision: `fb95671d855fa944c3717103cb13bd0135f8aec8`
  on x86_64 Linux. Artifact
  `/nix/store/dirnp4ri16smqcr2vd00wkp454b27kcg-eon-0.1.0` has NAR hash
  `sha256-u6Q8XqSXdzsqe+v2mJfUD0CMVTBKFuCQ5O2a4uXW+j0=`, NAR size 1,880,008
  bytes, and closure size 1,824,211,920 bytes.
- Tool-glyph checks: exact Nix build and flake check; installed wrapper,
  fontconfig, closure, face, and representative private-use charset inspection;
  fresh managed Yazi dogfood; and `git diff --check`.

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

### EON-C10 lifecycle and EON-C11 — runtime generations

- Lifecycle protocol proof: `fd6b348494111a0d18e241787da14ea99ee117a9`.
  Accepted EON-C11 attach proof: `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`
  on x86_64 Linux. The current presentation-owner candidate is
  `90a8b8ac44afc78c9bf5a08521b8f6bf853cbea2`.
- Artifact: `/nix/store/04sbq6wc79h2hf1rjq9y9bcvaz5v92hb-eon-0.1.0`,
  generation `g1-f7615184ba28993ce1c1c779751c0d7d`, NAR hash
  `sha256-uSidJ5Xf047x0vxjiscmnAHlVZiIvm0ugN6mQldawpc=`.
- Checks: locked format, check, 30-test, and Clippy suites; canonical manifest
  validation; exact Nix build and flake check; deterministic generation and
  presentation capability probing, preservation, and replacement process tests;
  retained live upgrade dogfood at
  `3b84d83d807c6249efa340fabf9e3d0d0d3ef310`; profile comparison; and `git diff --check`.
- Exercised behavior: bare launch selects only the exact current generation;
  current and fixed-namespace legacy work coexist and can be presented explicitly;
  supervisor-routed stop names and removes only the selected current generation;
  exact attach and stop ignore unrelated over-limit generation entries; profile
  refresh leaves the older supervisor and Orbit Sessions running.

## Current gaps

- EON-C5 through EON-C7 remain planned. The accepted artifact is a Nix-only
  x86_64 Linux alpha; direct bundles and native signed and notarized macOS
  distribution are unproved.
- Packaged graphics are proved on Intel Mesa. Proprietary NVIDIA remains
  unproved. The packaged font set supplies Symbols Nerd Font Mono for Yazi's
  supported private-use icons; unsupported emoji and symbols may still render
  as fallback boxes.
- Workspace topology is live only and bounded to 64 tabs and 256 panes. It has
  no explicit pane, tab, or individual Session-stop action. Whole-generation
  stop remains deliberately all-or-nothing. Machine restart does not preserve
  undeclared process or layout state, and EONW v1 provides no event stream,
  remote transport, authorization layer, plugin surface, or durable restoration.
- Generation discovery retains no manager, durable registry, compatibility
  window, live handoff, automatic eviction, or machine-restart recovery. Legacy
  workspaces expose no authoritative component identity or stop action.
- Closing Venus detaches the client while Orbit and its PTY remain alive. Bare
  `eon` and `eon attach` reconnect only to the exact current generation;
  `eon attach GENERATION` explicitly selects another compatible generation.
