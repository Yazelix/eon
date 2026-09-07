# Eon contract index

This is the canonical current state of Eon product behavior, ownership, proof,
and remaining limitations. Owning Beads and Git retain execution history;
`CHANGELOG.md` retains accepted user-visible chronology.

## Status

- **Planned:** accepted intent without implementation evidence
- **Candidate:** implemented and mechanically verified without an accepted final
  proof revision
- **Partially proved:** one exact useful slice is proved with a required gap
- **Proven:** an exact check, component set, platform, and artifact cover the
  contract
- **Retired:** explicitly replaced or removed; its ID is never reused

The current accepted composition selects Orbit
`64b225eb249490c9075894814942652a8b9d6192` with canonical ORBS v10 and Venus
`7f325a31d0052d84a1a09ff065c0e9103563c0e8`. The active Eon profile resolves to
`/nix/store/f46g210pffvzlyix6nv0rh3dsl5arv5m-eon-0.1.0`. The active EonTerm
profile remains `/nix/store/n5bmygd82arnfhzlcgz6llydap8rr7lw-eonterm-0.1.0`.
Existing live supervisors were not restarted.

## EON-C1 — Exact compatible component launch

- **Status:** Proven
- **Consumer:** A user launching Eon or EonTerm from one accepted product
  generation.
- **Trigger:** Eon starts a new composed runtime or adopts an exact live run.
- **Result:**
  - Eon validates one compatible component set, launches only that set, crosses
    each required Ready boundary before publication, and reports every exact
    component revision.
  - `assets/eon.png` is the transparent canonical application icon; Nix derives
    exact hicolor sizes and installed desktop metadata names `eon`.
  - The installed launcher action is named `Open Eon` and invokes the exact
    packaged executable. It may start or present the exact current generation,
    so it promises neither a new window nor a new Session; live surfaces retain
    terminal-authored titles.
  - Compatible terminal output leaves scrolling, selection, and full-Eon tab
    and pane focus usable between repaints. Orbit continues parsing output and
    answering terminal queries while the user reads retained history; history
    bounds, geometry validation, and synchronized presentation remain intact.
- **Important failures:** Missing, malformed, incompatible, replaced, or
  non-ready components fail before publishing a usable workspace or terminal.
- **Owner:** Eon launch validation, orchestration, icon selection, and desktop
  metadata; Nix only derives and installs artifacts; child components own their
  internal readiness and runtime state.
- **Consumes:** The versioned Eon component manifest, Orbit management/ORBS, and
  Venus native launch contracts.
- **Boundary:** Eon does not infer compatibility from executables, store paths,
  process names, or moving branches.
- **Proof:** `e1a5a9e02f7102cef48b25a83f740ea716647fa1`
  - **Environment:** Nix-built x86_64 Linux Wayland alpha
  - **Evidence:** Exact graph validation, Ready-boundary failure/recovery,
    current Venus/fzf reporting, and composed package checks; accepted base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70`, ORB-C13/ORBS v4 refresh
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, and Ready base
    `7cbcf1d4ce8ac186dc3ceff48240f04c437709af`; icon selection
    `32936b92e9c20a000d21f123869e0b749ff39618`, canonical master SHA-256
    `33ec3062f72a732290dcd6c6f40a2d5f535d6a7cbfcaf7455a0a5c4f03a52e0d`,
    launcher action `32c768e0d9a62984640a516c34d0cb0db16e8adb`, and accepted
    installed native-Wayland launch and scrollback dogfood. Exact Orbit
    `a65e199e16e97330175e314cacf791fa00f53069` and Venus proof
    `d3e63ae6e9fa0b72df426dbfc9bd29a96148577a` passed 30 ms continuous-output
    retained-scrollback and hard-fling acceptance without queue failure,
    starvation, snap-to-live, or timeout-truncated momentum. The current
    acceptance composes Orbit `64b225eb249490c9075894814942652a8b9d6192`
    with Venus `7f325a31d0052d84a1a09ff065c0e9103563c0e8`, passes the complete locked
    Rust and Nix checks, refreshes the profile above, and points the split 16 KiB
    DEC 2026 real-PTY regression at its installed Orbit binary. The held signed
    scroll resolves once after normal release, and held preview resolves after
    watchdog release without restarting any existing supervisor.

## EON-C2 — One component compatibility graph

- **Status:** Proven
- **Consumer:** Every Eon distribution and runtime launch path.
- **Trigger:** A product generation resolves or validates its component set.
- **Result:** One versioned manifest defines component identity, compatibility,
  and abstract artifacts for every channel without encoding launch policy.
- **Important failures:** Unknown components, duplicate identities, incompatible
  protocol versions, malformed revisions, or channel-specific graph drift fail
  validation.
- **Owner:** Eon's component manifest and validator.
- **Boundary:** The manifest does not own process lifecycle, platform launch
  mechanics, package-manager policy, or child state.
- **Proof:** `e1a5a9e02f7102cef48b25a83f740ea716647fa1`
  - **Environment:** x86_64 Linux Nix alpha
  - **Evidence:** Manifest parser/compatibility checks, exact graph consumption
    by installed Eon and EonTerm, and prior Eonova proof; accepted manifest base
    `234ad77ced4924d95400b5c39822b3fb9b928c95` and ORBS v7 graph refresh
    `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`; the current graph selects the
    exact accepted Orbit/Venus pair above with unchanged ORBS v10 and ORBF v1.

## EON-C3 — Distribution-neutral runtime inputs

- **Status:** Proven
- **Consumer:** Eon launch and every distribution channel.
- **Trigger:** A channel supplies exact component artifacts to Eon.
- **Result:** Eon receives explicit component paths, treats Nix store paths as
  opaque inputs, and invokes no Nix evaluator during normal runtime.
- **Important failures:** Missing, non-executable, incompatible, or substituted
  artifacts fail before accepted launch.
- **Owner:** Eon launch validation; the distribution supplies artifacts.
- **Consumes:** EON-C2's component graph.
- **Boundary:** Runtime code does not construct, persist, inspect, or derive
  identity from Nix store paths.
- **Proof:** `71e8f5a8cac938ed7f065890c83d9376551f6014`
  - **Environment:** Nix-built x86_64 Linux alpha
  - **Evidence:** Exact protocol-source substitution, package tests, and
    evaluator-absence checks; accepted launch-input base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70` and exact installed ORBS v7
    composition `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`;
    the current installed artifacts are the exact opaque store paths indexed
    above

## EON-C4 — Thin orchestrator ownership

- **Status:** Proven
- **Consumer:** Eon, Orbit, Venus, Helix, Yazi, and Ratconfig integrations.
- **Trigger:** Eon composes or controls a product surface.
- **Result:** Eon owns launch policy, topology, component selection, product
  configuration, updates, and distribution without duplicating terminal,
  rendering, editor, file-manager, or configuration state.
- **Important failures:** A missing child contract returns to that child instead
  of being reconstructed in Eon.
- **Owner:** Eon orchestration and product policy; each child keeps its subsystem
  authority.
- **Consumes:** Accepted child contracts through EON-C2's exact graph.
- **Boundary:** No copied child schema, hidden fork, compatibility adapter, or
  second terminal/rendering owner.
- **Proof:** `71e8f5a8cac938ed7f065890c83d9376551f6014`
  - **Environment:** x86_64 Linux composed alpha
  - **Evidence:** Component-boundary checks, management consumer proof, package
    closure inspection, and accepted installed native-Wayland runtime dogfood;
    accepted composition base `a2792cc77f8254cc277d64eb41f65725b69ccf70`;
    the current acceptance adds no Eon scrolling implementation, compatibility
    adapter, dependency, or second component graph

## EON-C5 — Direct bundle lifecycle

- **Status:** Planned
- **Consumer:** A user installing Eon without adopting an unrelated toolchain.
- **Trigger:** Install, upgrade, inspect, or remove a future direct Eon bundle.
- **Result:** The operation affects only the selected Eon bundle and its owned
  integration points.
- **Important failures:** Partial installation, incompatible upgrade, or removal
  must not replace or damage unrelated tools or user configuration.
- **Owner:** Future Eon direct distribution.
- **Boundary:** No direct bundle, installer, signing, or release format is active
  during Nix-only alpha.
- **Proof:** None.
- **Open proof:** Distribution graduation requires explicit user activation.

## EON-C6 — Shared graph across distribution channels

- **Status:** Planned
- **Consumer:** Nix alpha and any later approved distribution channel.
- **Trigger:** A channel composes an Eon product artifact.
- **Result:** Every channel consumes the same accepted EON-C2 component graph
  without changing runtime semantics.
- **Important failures:** A channel-specific compatibility graph or hidden
  component substitution is rejected.
- **Owner:** Eon distribution composition consuming EON-C2.
- **Boundary:** Later channels remain inactive during Nix-only alpha.
- **Proof:** None.
- **Open proof:** Requires an approved second distribution channel.

## EON-C7 — Native Wayland without init-system lock-in

- **Status:** Planned
- **Consumer:** A user launching a supported Eon distribution on Linux.
- **Trigger:** Build, install, or launch an approved Eon channel.
- **Result:** Eon targets native Wayland without requiring a particular init or
  service manager, and each channel names its proved architectures and launch
  environments.
- **Important failures:** Systemd-only lifecycle assumptions, X11 fallback, or
  unproved architecture claims are rejected.
- **Owner:** Eon platform and distribution policy; child-owned kernel
  capabilities remain explicit child contracts.
- **Boundary:** Current alpha is x86_64 Linux native Wayland; X11, Xwayland, and
  macOS are unsupported.
- **Proof:** None.
- **Open proof:** Installed non-systemd Wayland dogfood is required before a
  non-systemd support claim.

## EON-C8 — Durable tab and pane workspace

- **Status:** Candidate
- **Consumer:** One local Eon workspace user.
- **Trigger:** Launch, create or close a tab, create a pane, traverse focus,
  reorder the active tab or selected pane, receive Session exit, or recover
  accepted same-boot runs.
- **Result:**
  - Independent durable Sessions appear as horizontal tabs containing vertical
    accordion panes with exactly one expanded pane.
  - Tabs use stable `tN` identities. Each tab header shows its number plus the
    leaf, `~`, or `/` derived from Eon's authoritative launch directory; actions
    retain `tN`, and accessibility includes bounded full-path context.
  - Each pane is identified as `pN`, two ASCII spaces, and a compact working-
    directory label; home uses the packaged marker, descendants use `~/`, and
    external paths remain absolute.
  - Left/right tab traversal and up/down pane traversal wrap at ordered edges
    when the axis has at least two targets; singleton axes remain unavailable.
    Left/right remains available while a directory picker is open: the picker
    stays bound to its original live tab while another active tab presents its
    selected pane. Users can create new panes or tabs through the accepted
    semantic actions only when no picker is open.
  - Moving left/right swaps the active tab with exactly one adjacent tab;
    moving up/down swaps the selected pane with exactly one adjacent pane in
    the active tab. The moved stable identity remains selected. Movement stops
    at ordered edges and remains unavailable while a picker is open.
    Venus maps Ctrl+Alt+H/L and Ctrl+Alt+K/J directly to these actions without a
    mode.
  - Closing names the expected active `tN` and requires another live tab. A
    non-final pending tab first stops its exact picker, then disappears and
    restores its prior tab. A durable tab closes only while no picker exists:
    Eon stops its Orbit Sessions one at a time in pane order and prunes each
    confirmed end through the normal Session-exit owner. Complete success
    removes the tab with the same deterministic focus rule as natural exit.
    If a later stop fails, no later Session is stopped, the action reports
    failure, and every remaining live Session stays represented in the
    partially pruned tab. A Session that ended naturally during the request is
    reconciled and pruned as an exit rather than reported as a successful Stop.
    Venus maps Ctrl+Shift+W directly to this stable-target action; lowercase
    Ctrl+W remains terminal input.
  - Ended Sessions leave no dead pane or empty tab; focus moves deterministically
    to the same identity when possible, otherwise the following sibling at the
    removed index, otherwise the preceding sibling; an empty workspace closes
    Venus and the supervisor.
  - Same-boot recovered runs project numerically into synthetic `t1` as `pN`
    without claiming restoration of prior topology or launch-directory policy.
  - New current-generation full Eon surfaces omit the redundant native title
    bar while retaining the selected terminal title for compositor semantics.
- **Important failures:** Unknown or stale identity, a close target other than
  the active tab, final-tab close, durable close while any picker exists,
  invalid transition, a directional axis without another target, movement at
  an ordered edge, unavailable endpoint, duplicate identity, or partial
  recovery leaves accepted topology unchanged. A repeated request ID never
  repeats a stop, and a replay naming a removed tab cannot close its successor.
  Picker-stop failure changes no topology; durable partial-stop behavior is the
  explicit result above. Exhausted pane numbers fail before Session start;
  unknown or repeated Session-exit notices remove nothing; losing a view never
  silently stops or substitutes a Session.
- **Owner:** Eon workspace topology, identity, focus, pruning, and action policy;
  Orbit owns Sessions and Venus owns native materialization.
- **Consumes:** EONW v4, Orbit Session identities/endpoints, and Venus `VEN-C8`.
- **Boundary:** No arbitrary split tree, simultaneous expanded panes, arbitrary
  reorder target or cross-tab pane movement, picker relocation, durable layout
  restoration, a user-facing per-pane Session stop/restart action, terminal
  content/history, provider state, plugin surface, remote access, or
  reconstructed Session state.
- **Proof:** `5f23a7bac127785d913a718e5fb1afc53d9d91ae`
  - **Environment:** x86_64 Linux Nix candidate and installed profile
  - **Evidence:** Deterministic and process-level wrapped traversal, workspace
    composition, creation, exit pruning, `tN`/`pN` projection, exact tab
    launch-directory labels and accessibility, full locked Rust/Nix checks, and
    installed native user acceptance; prior topology proof
    `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`, compact-header proof
    `e77e842fe8c7070a96047dff1bf028ccbd49b788` and exact Venus consumer
    `19d7d28a2aba09c0afe3173d25bbb76ec3edce49`
  - **Movement revision:** dogfooded installed package
    `/nix/store/dbxxlkrh6ygvksra9rxdvqzzj4228jbq-eon-0.1.0`, built from the
    reviewed working tree based on `9f420ad7f7d4a09ec4294e5ecb191ca2a3d2b7d3`
  - **Movement evidence:** Full locked Rust and Nix checks plus an isolated
    installed CLI run moved `t2` and `p2` by one adjacent position, retained
    all three Session mappings, rejected edge moves without mutation, restored
    both orders, and stopped exactly its isolated Sessions. The profile refresh
    preserved the older live two-Session supervisor without restarting it.
  - **Close and shortcut revision:** EONW owner
    `c305453bba4fe50c29f65e829b9cd65af31ced8a`, exact Venus consumer
    `d2d798099934dcf8037bfad6ab856e40c9b989fe`, and installed profile
    `/nix/store/x408dv5djxnia3sh88jwalhpqxfbmh2r-eon-0.1.0`
  - **Close and shortcut evidence:** Full locked Rust and Nix checks prove
    stable-target close, ordered moves, edge rejection, deduplication, and
    partial-stop topology. Isolated native Wayland input emitted EONW actions
    14/15/16/17/18 for Ctrl+Alt+H/L/K/J and Ctrl+Shift+W; close removed `t2`'s
    pending picker and restored durable `t1`. The isolated generation stopped
    cleanly, and both pre-existing live supervisors retained their PIDs.
- **Open proof:** Native current-generation tab-label acceptance remains pending.

## EON-C9 — Managed shell environment

- **Status:** Proven
- **Consumer:** Eon Sessions launched under Nushell, Bash, Zsh, or Fish.
- **Trigger:** Eon constructs a child launch environment.
- **Result:**
  - Eon selects exact Nushell, Bash, Zsh, Fish, Starship, Zoxide, fzf, Atuin,
    Carapace, Helix, Yazi, and LazyGit artifacts.
  - Stable `eon-*` commands expose managed shells and tools outside Eon; one
    child-private PATH exposes accepted unprefixed names inside Sessions.
  - Configuration accepts one direct argv command and independent Starship,
    Zoxide, Atuin, and Carapace booleans. Defaults are `command = ["eon-nu"]`
    and `true`; each new Session rereads them without rewriting running shells.
  - Managed shells load native user configuration before bounded Eon activation.
    Disabled integrations suppress only Eon's hook; existing prompts,
    completers, same-tool hooks, normal Starship discovery, and Atuin policy
    remain child-owned.
  - Nushell native user autoload remains last and its stock startup banner is
    suppressed. Eon does not set `STARSHIP_CONFIG`, and Atuin continues to honor
    `ATUIN_NOBIND`.
- **Important failures:** Invalid or unreadable configuration, empty command,
  missing artifact, manifest mismatch, or shell launch fails explicitly without
  a replacement Session. Optional integration failure warns without preventing
  shell launch. Eon never mutates global configuration, startup files, aliases,
  or PATH.
- **Owner:** Eon launch-environment policy; shells and tools retain their native
  configuration.
- **Boundary:** No global aliases, user-file mutation, prompt/completion schema,
  shell framework, automatic Direnv/Mise, history/sync policy, plugin surface,
  or editor replacement. Arbitrary argv remains unmanaged and
  `eon run -- COMMAND...` remains the explicit escape hatch.
- **Proof:** `9f6599d103162061ee666126c2eddce03383afb6`
  - **Environment:** x86_64 Linux Nix alpha
  - **Evidence:** Four-shell environment checks, private Session PATH, fzf option
    isolation, tool glyphs, Fish preservation, and packaged command dogfood; accepted base
    `6dfcb473beccadd6e145235009240c81dd570fe5`, glyph proof
    `fb95671d855fa944c3717103cb13bd0135f8aec8`, and private PATH
    `c0d044c69318a921f9f9139bcaf2de9afce683d3`
## EON-C10 — Typed workspace control

- **Status:** Proven
- **Consumer:** One local CLI or approved composition controlling a live Eon
  supervisor.
- **Trigger:** The client submits one versioned workspace or supervisor-lifecycle
  action.
- **Result:**
  - Eon returns one complete typed result from the sole live workspace owner;
    human-readable CLI output and deterministic structured output project the
    same result.
  - Workspace results carry ordered tab, pane, and Session identities, active
    and selected identities, liveness, exact raw tab launch-directory bytes,
    and exact opaque Orbit endpoint bytes. Lifecycle results carry generation,
    component/protocol identity, live Sessions, and owner-authored attach/stop
    availability.
  - CLI JSON emits opaque directories and endpoints as ordered byte arrays
    without changing EONW. Each connection carries one length-delimited request
    and response and does not use EOF as framing.
  - The shared 2 MiB frame ceiling admits every snapshot valid at EONW's field
    and workspace-count bounds.
- **Important failures:** Malformed, oversized, incompatible, unavailable,
  rejected, timed-out, partially read, or response-lost operations fail within
  the shared bound without reconstructing hidden state or reviving completed
  Stop. Accepted Stop names affected live Sessions, sends canonical Stop through
  every validated management lease before waiting, and returns only after every
  exact terminal record is complete.
- **Owner:** Eon supervisor workspace/action state, EONW result ordering, and CLI
  projection.
- **Consumes:** EONW v4 and accepted Orbit management operations.
- **Boundary:** Additive lifecycle tags do not widen the pinned Venus workspace
  consumer. There is no event stream, subscription policy, remote transport,
  plugin/MCP API, authorization framework, durable restoration, terminal
  content, or direct child-protocol escape hatch.
- **Proof:** `9f6599d103162061ee666126c2eddce03383afb6`
  - **Environment:** x86_64 Linux Nix package and installed profile
  - **Evidence:** EONW v2/v3 codec/version rejection, raw directory snapshot and
    retarget action, CLI projection, absolute connection/read deadlines,
    management Stop, response-loss finality, and source ownership; EONW v3 seed
    `96119f29ca2e3ec4ad19bbe272708b07d588429a`, v2 base `7bb50873ae27e09dfebd8a6ca2f8075bac07afe8`, lifecycle
    actions `fd6b348494111a0d18e241787da14ea99ee117a9`, CLI proof
    `d0c2208d628fa0dc1e186899b45e0b73534ff9bc`, deadline hardening
    `a390fc5c007900c3fc8c9c49df85f9b8acc06d4a`, management Stop
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, response-loss correction
    `56fcae2d00baecf9b69e4650882a69e50419f56b`, and maximum-bound snapshot
    correction `10c29edf861fac28db48f41a4546165f79777ee6`
## EON-C11 — Runtime generations and presentation

- **Status:** Candidate
- **Consumer:** Eon and EonTerm users launching or targeting a product generation.
- **Trigger:** Launch current generation, inspect an older live generation,
  present a compatible existing surface, or explicitly stop a generation.
- **Result:**
  - Eon and EonTerm use separate runtime namespaces and default to their exact
    current generation.
  - Generation identity derives from runtime source, dependency lock, canonical
    component manifest, and EONW source; each generation owns one private
    directory.
  - Older live generations remain discoverable, explicitly presentable when
    compatible, and explicitly stoppable through their own supervisor.
  - Implicit attach selects only a live supervisor reporting the exact current
    identity; otherwise Eon starts current without stopping older work. Explicit
    attach never substitutes another generation, and discovery distinguishes
    current, previous, legacy, dead, incompatible, and corrupt candidates.
  - One per-generation lifecycle lock covers listener and runtime teardown so a
    replacement cannot overlap cleanup; concurrent launch converges on one
    supervisor and Session set.
  - Present requests native presentation without replacing the live Venus
    process, Orbit attachment, or Session.
  - Dead exact residue is removed only after the recorded process is absent or
    dead; a later launch may then create one fresh run.
  - A current launch overlapping clean exit of the final Session waits for the
    retiring supervisor, then starts one fresh Session instead of reporting
    presentation success against ended state.
- **Important failures:** Incompatible generation, stale or mismatched process
  identity, lost control, partial Stop, or live foreign residue fails closed
  without PID/process-name fallback or duplicate Orbit launch. Before forking
  Orbit, Eon waits within the shared five-second deadline until its current
  cgroup-v2 directory is user-owned, not writable by group or others, and
  contains Eon's process; failure is explicit and uses no compositor, init, or
  service-manager API.
- **Owner:** Eon generation selection, runtime namespaces, discovery, Present,
  explicit Stop, and exact owned residue.
- **Consumes:** Orbit management v1 and Venus `VEN-C14`.
- **Boundary:** No manager daemon, persisted topology, machine-restart recovery,
  PTY handoff, old-generation backport, updater, Nix evaluation, package-channel
  identity, compatibility window, remote runtime, plugin API, service-manager
  requirement, or automatic eviction.
- **Proof:** `10c29edf861fac28db48f41a4546165f79777ee6`
  - **Environment:** x86_64 Linux Nix package and installed profile
  - **Evidence:** Separate namespaces, repeated launch, native Present,
    management Stop, supervisor-loss replacement, and dead-residue correction;
    accepted base `3b84d83d807c6249efa340fabf9e3d0d0d3ef310`, attachment
    `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`, generated identity
    `4beb441301d84039570dd07138da2a6f70b3ed23`, Stop owner
    `dbffad4018f0339bd3d35ea03579d0e09cff73bf`, convergence
    `36c95ad04cb9519f88b907642c8147d95f56ee83`, EonTerm lifecycle
    `63686a12b752c9423b2096d5e32aa5842f2184fc`, same-boot adoption
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, Venus presentation
    `50b7ef7f6c9d5b531b79ecca67c9c8fdf40f355f`, readiness correction
    `cfcb38e6e711e971ed6004528987761ddd7c87e4`, exact EONW v2 generation
    activation `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`, and codec-source sensitivity
    correction `3c1d6e08d223df753eb88ca031d108ed1abe8ee3`, and bounded-frame correction
    `10c29edf861fac28db48f41a4546165f79777ee6`; installed artifact
    `/nix/store/dbiv438iwn9mljrwfg6d29ypg96hp00c-eon-0.1.0`
- **Open proof:** Current-generation native acceptance remains candidate evidence.

## EON-C12 — Standalone exact-command terminal

- **Status:** Proven
- **Consumer:** A local user or approved composition needing one native terminal
  surface without Eon workspace actions.
- **Trigger:** EonTerm launches one exact command after `--`.
- **Result:**
  - `eonterm -- COMMAND...` launches one exact nonempty argv in one Orbit Session
    and one Venus surface without workspace topology. Optional
    `--no-decorations` is fixed for the supervisor lifetime.
  - `eonterm generations`, `attach`, and `stop` expose EON-C11 within EonTerm's
    separate namespace.
  - Repeated launch presents an attached surface; after detachment it opens one
    replacement against the same live Session with the original decoration
    choice. Later invocations do not mutate that choice.
  - Workspace actions return `workspace-unavailable` and create no hidden
    topology.
- **Important failures:** Missing command, invalid identity, incompatible
  component, startup failure, supervisor loss, or explicit Stop follows the same
  bounded ownership and cleanup rules without creating workspace state.
- **Owner:** EonTerm launch/generation policy; Orbit owns the Session and Venus
  owns the surface.
- **Consumes:** EON-C1 through EON-C4, EON-C11, Orbit, and Venus.
- **Boundary:** EonTerm consumes no EON-C9 managed environment and adds no tabs,
  panes, key remapping, preset, wrapper, persisted mode marker, workspace,
  managed-tool/default-shell command, remote access, focus guarantee, or Zellij
  policy. Bare `eon` and `eon run` retain workspace meanings.
- **Proof:** `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`
  - **Environment:** x86_64 Linux Nix package and installed runtime
  - **Evidence:** Exact argv, environment, vivid palette, repeated launch,
    supervisor-loss recovery, and clean Stop; accepted product
    `816372ea9ffe90f08ff442b5763a3b5413b7906c`, palette
    `285c6bf48fb402a673eb997594b6eb1bbcb1429b`, and lifecycle
    `63686a12b752c9423b2096d5e32aa5842f2184fc`

## EON-C13 — Terminal-background opacity policy

- **Status:** Proven
- **Consumer:** Every Eon or EonTerm Venus surface.
- **Trigger:** Eon creates or reopens a surface while
  `$EON_CONFIG_HOME/config.toml` contains optional
  `terminal.background_opacity`.
- **Result:** Eon accepts one finite `f32` in `0.0..=1.0`, defaults absence to
  `0.80`, and passes the exact value as `--background-opacity` before Venus
  endpoints. Eon and EonTerm share it. A live Venus is unchanged; replacement
  reads one current configuration snapshot while Orbit and its PTY remain live.
- **Important failures:** Invalid initial configuration names the field and
  starts no supervisor, Orbit, or Venus. Invalid replacement configuration
  returns bounded presentation failure without stopping Orbit or its child.
- **Owner:** Eon configuration/default and launch projection; Venus owns native
  rendering.
- **Consumes:** Venus `VEN-C11` at
  `74ab5a0b661210f0afec94086f5358fe50b01f05`.
- **Boundary:** Opacity affects only terminal default background and padding, not
  explicit cells, text, cursor, selection, chrome, decorations, input, hit
  testing, accessibility, or click-through. No profile, public override,
  watcher, live reload, Orbit restart, non-Wayland claim, or renderer fallback.
- **Proof:** `7303ee5cca3925939b84267bd54587a2cfb223a6`
  - **Environment:** x86_64 Linux COSMIC Wayland composition
  - **Evidence:** Config admission, Eon and EonTerm argv projection, opacity
    values, live presentation, and native visual acceptance

## EON-C14 — Default compositor blur policy

- **Status:** Proven
- **Consumer:** Every Eon or EonTerm Venus surface.
- **Trigger:** Eon creates or reopens a surface while
  `$EON_CONFIG_HOME/config.toml` contains optional strict boolean
  `terminal.background_blur`.
- **Result:** Absence defaults to `true`; true passes exactly one
  `--background-blur` before Venus endpoints and false omits it. Eon and EonTerm
  share the setting. A live Venus is unchanged; replacement uses one current
  configuration snapshot while Orbit and its PTY remain live. Blur remains
  independent from opacity.
- **Important failures:** Invalid initial configuration names the field and
  starts no supervisor, Orbit, or Venus. Invalid replacement configuration is
  bounded without stopping Orbit. Unsupported compositor policy remains
  Venus/compositor best effort and does not make Eon launch fail.
- **Owner:** Eon configuration/default and launch projection; Venus and the
  compositor own native behavior.
- **Consumes:** Venus `VEN-C15` at
  `7fc7e4ba97aaf48b586002934a580ef2d1c31694` and EON-C13.
- **Boundary:** No capability probe, automatic opacity, blur strength, public
  override, watcher, live reload, fallback renderer, Eonova-specific policy, or
  non-Wayland visual claim.
- **Proof:** `b41be3a00a8e47a435521509c3d060d80e0524a5`
  - **Environment:** x86_64 Linux COSMIC Wayland composition
  - **Evidence:** Default-on and explicit-off config, independent opacity, exact
    launch argv, and native visual acceptance

## EON-C15 — Same-boot Orbit recovery

- **Status:** Candidate
- **Consumer:** One local Eon or EonTerm user on the same boot and login.
- **Trigger:** An Eon-launched Orbit crosses Ready, its supervisor disappears, a
  replacement targets the same exact generation, or the user requests Stop.
- **Result:**
  - Eon assigns canonical `session-N`, one fresh opaque run ID, and the exact
    Orbit component revision, then validates complete live identity and acquires
    the sole management lease within one shared five-second operation.
  - Replacement EonTerm recovers exactly one live run; replacement full Eon
    acquires at most 256 valid runs, sorts positive canonical N numerically, and
    projects them into synthetic `t1` as `pN` with the lowest selected. That tab
    receives the replacement supervisor's launch directory as fresh policy.
  - New creation continues at checked `max(N)+1`; prior grouping, focus, argv,
    launch directories, and request history are not restored. After recovered
    `t1`, later creation starts at `t2`.
  - Before spawn, Eon creates and retains the exact empty owned mode-0600
    management record inode and holds its exclusive lock through rollback or
    Stop. Only winning that still-empty inode authorizes local Child rollback;
    after Orbit marks and atomically publishes Ready, cleanup is management-only.
  - Explicit generation Stop uses canonical management for every acquired run
    and succeeds only after matching tombstones and cleanup.
  - Eon removes exact owned dead residue only after the recorded process is
    absent or dead.
- **Important failures:** Unsafe, missing, replaced, malformed, oversized,
  incompatible, duplicate, zero, noncanonical, overflowing, wrong-generation,
  wrong-UID, wrong-process, Busy, partial acquisition, Orbit exit, deadline, or
  transport failures publish no partial workspace, launch no duplicate run, and
  authorize no PID, signal, pathname, process-name, group, cgroup, or pidfd
  fallback. A lost response after validated Stop cannot revive the supervisor.
  A marked Live record rejected after spawn is stopped through canonical
  management using its published identity and is never published as a Session.
- **Owner:** Eon generation/recovery policy, bounded enumeration, projection,
  lease collection, deadline, EONW ordering, retained launch claim, and exact
  residue; Orbit `ORB-C13` owns live identity, Ready, lease, Session state,
  `ORB-C12` cleanup, tombstone, and endpoint cleanup.
- **Consumes:** Orbit `ORB-C12`, `ORB-C13`, management v1, and Venus `VEN-C14`.
- **Boundary:** Same boot and login only; no prior-topology persistence, Orbit or
  machine restart recovery, logout/reboot survival, remote authority, service
  manager, broker, or compatibility adapter. Tombstones grant no authority and
  must be reconciled before their workspace slot is reused.
- **Proof:** `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`
  - **Environment:** x86_64 Linux packaged installed recovery and adversarial
    lifecycle checks
  - **Evidence:** Ready serialization, supervisor kill, exact adoption, Busy and
    saturated-listener bounds, partial-response deadline, response-loss Stop,
    explicit tombstones, and dead-residue relaunch; base
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, response loss
    `56fcae2d00baecf9b69e4650882a69e50419f56b`, connection deadline
    `a4f0122862b7293326c22530f787124a47d0f560`, response deadline
    `ced9e4ae11ed21a0f05d50cd470491adffa73b54`, Ready base
    `7cbcf1d4ce8ac186dc3ceff48240f04c437709af`, Ready correction
    `871c9f639e519c7e3201fa5d9755d0a77a4cb0df`, and compact recovery
    correction `cfcb38e6e711e971ed6004528987761ddd7c87e4`, and synthetic-tab
    launch-directory fallback `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`
- **Open proof:** Final current-source user acceptance remains candidate evidence.

## EON-C16 — Caller-owned application identity

- **Status:** Proven
- **Consumer:** Full Eon, standalone EonTerm, or one approved EonTerm composition.
- **Trigger:** The caller launches a new Venus surface with its selected validated
  desktop application ID.
- **Result:** Full Eon supplies `eon`, standalone EonTerm supplies `eonterm`, and
  an approved composition passes its value unchanged to `VEN-C17` before window
  creation; identity never enters PTY argv/environment and remains independent
  from terminal-authored titles.
- **Important failures:** Empty, non-UTF-8, oversized, or non-token identities
  fail before Orbit or Venus starts; profile refresh does not mutate a live
  surface's selected identity.
- **Owner:** Eon owns defaults, validation, and EonTerm CLI; the embedding
  composition owns its identifier and desktop metadata; Venus owns native
  mapping only.
- **Consumes:** Venus `VEN-C17` at
  `ab24961bd6b2f9403736e52ebbac8cc266488a41`.
- **Boundary:** Immutable launch metadata only; no runtime protocol, branding
  provider, desktop discovery, child inspection, or wider platform promise.
- **Proof:** `6a3236bb342c535aca16acdf563ff66386e8f6e5`
  - **Environment:** x86_64 Linux COSMIC Wayland with installed Eonova
  - **Evidence:** CLI validation, exact Venus argv mapping, distinct Eon/Eonova
    grouping, repeated-launch convergence, and unchanged Session identity

## EON-C17 — Tab launch directory

- **Status:** Candidate
- **Consumer:** One local Eon workspace user creating tabs and panes.
- **Trigger:** Eon creates or recovers a workspace, creates a tab or pane, or an
  approved same-user client explicitly retargets one live tab.
- **Result:**
  - Every live tab has one stable `tN` identity and one authoritative absolute
    launch directory. Panes retain `pN` identities.
  - Fresh `t1` validates the supervisor's absolute launch directory before any
    Session recovery or startup. A new tab inherits the active tab's value and
    uses it for its first Session.
  - `SetTabDirectory` changes exactly one addressed tab. Later panes in that tab
    start with the accepted directory; existing Sessions and terminal CWDs do
    not change.
  - EONW v4 returns the exact accepted raw path bytes. Venus derives a bounded
    `N  leaf`, `N  ~`, or `N  /` label while retaining `tN` for actions and
    exposing identity plus bounded path context to accessibility.
  - Same-boot recovery assigns synthetic `t1` the replacement supervisor's
    launch directory as fresh policy and never infers prior policy from a pane.
- **Important failures:** Empty, relative, NUL-containing, oversized, missing,
  non-directory, unknown-tab, or stale-tab updates change nothing. Once a path
  passes validation, later disappearance leaves its policy value stored; a
  later Session startup fails without committing pane topology or leaving
  launch residue.
- **Owner:** Eon owns tab identity, launch-directory state, validation, mutation,
  inheritance, and recovery fallback; Orbit owns each Session's terminal CWD;
  Venus owns only native projection of Eon's state.
- **Consumes:** EONW v4, Orbit Session startup, and Venus `VEN-C8`.
- **Boundary:** No manual names, path canonicalization, retained directory
  handles, filesystem watching, shell-`cd` tracking, pane-CWD inference,
  retargeting of existing processes, persistence, picker UI, or compatibility
  service accepting EONW v1 and v2.
- **Proof:** `9f6599d103162061ee666126c2eddce03383afb6`
  - **Environment:** x86_64 Linux Nix package and installed Eon profile
  - **Evidence:** EONW v3 directory seed
    `96119f29ca2e3ec4ad19bbe272708b07d588429a`, EONW v4 pending-tab seed
    `aaafc9127c054e683abfceb3c8fcaae201a7a763`; exact Venus consumer
    `2622c124be5ba8a62037c6de952ebf3928347387`; focused codec, workspace,
    CLI, launch-command, startup-disappearance, and outer multi-tab CWD/rollback
    checks; complete locked Rust/Nix gates, manifest validation, generation
    sensitivity, maximum-bound EONW snapshot round trip, and exact installed
    startup-disappearance proof; profile artifact
    `/nix/store/j4h0bmyvkndzvhi3l9r96b7f57jxlmm8-eon-0.1.0` reports generation
    `g1-a9a0102de0a2c56d1d5bdda6c25bb3dc`
- **Open proof:** Native visual and AT-SPI dogfood begin after the next normal Eon
  restart; the profile refresh intentionally preserved the older live supervisor.

## EON-C18 — Picker-first tab directories

- **Status:** Proven
- **Consumer:** One person starting or using full Eon.
- **Trigger:** Eon needs the first pane for a new tab, or the person presses
  Alt+Z in an existing tab while no directory picker is already open.
- **Result:**
  - A fresh workspace and every later new tab begin as one active pending tab
    with no durable pane. Eon starts one transient Orbit Session running the
    exact packaged ranked-directory picker at the inherited launch directory.
    Ambient fzf default options cannot alter its command, layout, or bindings.
  - Accepting a valid choice atomically commits the tab directory and starts
    exactly one first `pN` Session there. First-tab cancellation falls back to
    Eon's validated launch directory; later-tab cancellation abandons the
    pending tab and restores the prior tab and focus.
  - Alt+Z keeps the existing explicit retarget behavior after a tab has panes;
    it never changes or restarts a running pane.
  - EONW exposes that picker as one explicit modal endpoint bound to one live
    tab, never as a normal `pN` pane. The picker tab may remain live while
    another durable tab is active. EONW v4 represents a picker-owned pending tab
    as the sole pane-free tab, with an absent selected pane instead of a sentinel
    or placeholder process.
    Lifecycle inspection and Stop may report zero durable Sessions during the
    initial picker; the transient picker remains excluded from that list.
  - Venus can start a full-Eon presentation from the workspace endpoint alone
    while the initial tab is pending; the picker endpoint in its first snapshot
    is the sole terminal attachment.
  - Venus keeps the tab bar visible and replaces the tab body with the picker,
    inset by one terminal cell on every side when the grid permits it. The
    previous terminal is not composited underneath. Attach and recovery never
    create an automatic picker; recovering only a stale initial picker stops it
    and applies the validated launch-directory fallback.
  - Existing left/right focus actions continue to traverse and wrap live tabs
    while the picker stays bound to its original tab. Another active tab shows
    its selected pane; returning shows the same picker for normal acceptance or
    cancellation. Ctrl+Shift+W closing is admitted only for the active non-final
    pending tab owned by that picker; it stops the transient Session before
    removing the tab and restoring its prior focus. Up/down, focus by identity,
    creation, a second picker, durable-tab close, and unrelated topology changes
    remain unavailable until the picker exits.
- **Important failures:** Cancel, empty or invalid selection, picker launch or
  exit, target disappearance, duplicate invocation, origin-tab loss, or
  presentation detachment follows the first- or later-tab fallback without a
  partial tab, consumed pane or durable Session identity, changed existing
  process, or transient process, endpoint, record, pane, or modal residue.
  Left/right or close with only one live tab remains unavailable without
  mutation. A duplicate close cannot stop the picker twice or remove another
  tab.
- **Owner:** Eon owns picker policy, tab binding, command selection, lifecycle,
  validation, mutation, and cleanup. Venus owns full-Eon shortcut precedence,
  modal geometry, focus, input, notices, rendering, and accessibility. Orbit
  owns the transient PTY and child. Zoxide owns ranking and fzf owns interactive
  terminal selection.
- **Consumes:** EON-C17; accepted EONW v4 pending-tab boundary, Orbit's accepted
  Session startup, attachment, exit, and stop contracts, and exact Venus v4
  consumer `d2d798099934dcf8037bfad6ab856e40c9b989fe`.
- **Boundary:** No Yazi dependency, normal-pane identity, arbitrary-command or
  generic-popup API, simultaneous terminal composition, native Venus picker,
  placeholder shell, pane replacement, current-process `cd`, persisted pending
  tab, EonTerm action, or additional platform.
- **Proof:** EONW v4 owner source
  `c305453bba4fe50c29f65e829b9cd65af31ced8a` admits one inactive live picker
  tab, including the sole pane-free pending tab, with focused red/green codec,
  complete locked Rust, exact pinned Venus consumer, and full Nix checks on
  x86_64 Linux. Installed native Wayland input through Venus
  `d2d798099934dcf8037bfad6ab856e40c9b989fe` used Ctrl+Shift+W to stop and
  remove a pending `t2`, restore durable `t1`, and leave no picker residue in
  profile `/nix/store/x408dv5djxnia3sh88jwalhpqxfbmh2r-eon-0.1.0`. Candidate
  source `9af359e964e6cb1fc53548d6779456c11343c7e1` preserves picker identity and
  the pane-free pending tab across owner- and process-level wrapped traversal;
  complete locked Rust and Nix checks produced profile artifact
  `/nix/store/w2f3wd5vz40p0m95miph0x7a64mzlxxa-eon-0.1.0`, generation
  `g1-e4cae4b6bcd7489a7f604f229736f8aa`. Runtime source
  `53004af9a18be68a58713ef9461a2cd336e246e5` consumes original EONW v4 seed
  `aaafc9127c054e683abfceb3c8fcaae201a7a763`.
  Existing Alt+Z runtime proof remains source
  `9f6599d103162061ee666126c2eddce03383afb6`; installed profile artifact
  `/nix/store/nqcjvnxyjrnjm1hwl4lcwqzi0jmyfvli-eon-0.1.0`, generation
  `g1-5cf7a15fab599220be3121a7b3a764a8`
  - **Environment:** x86_64 Linux COSMIC Wayland, installed Eon profile, and an
    isolated delegated user scope
  - **Evidence:** Exact Orbit
    `70861097a825c2fbfaea53a8ca9437f45e8602eb`, exact Venus
    `2622c124be5ba8a62037c6de952ebf3928347387`, focused picker and replacement-
    recovery checks including bounded invalid-selection visibility and no
    mutation, complete locked Rust and Nix gates, exact packaged Zoxide and fzf
    identities, focused long-root launch, first-Stop-failure retry, and hostile-default isolation,
    installed modal presentation-loss cleanup with the durable Session retained,
    one-batch generation Stop with a durable-only public result, transient
    endpoint, record, process, and runtime cleanup, and user-accepted native
    Wayland presentation. An isolated installed run committed `t1` and `t2` to
    their selected directories, started only `session-1` and `session-2`, and
    stopped both through the durable-only generation result. The user accepted
    exact-current native Alt+H/L traversal with the picker retained at source
    `9af359e964e6cb1fc53548d6779456c11343c7e1` and the profile above.

## Rules

- Each contract uses one `## EON-CN — Name` heading and the required fields
  `Status`, `Consumer`, `Trigger`, `Result`, `Important failures`, `Owner`,
  `Boundary`, and `Proof`.
- Add optional `Consumes` and `Open proof` fields when applicable; nest
  `Environment` and `Evidence` under `Proof`, and use nested bullets instead of
  prose table cells.
- Contract IDs are stable and repository-qualified. Never renumber or reuse an
  ID; mark an explicitly removed contract retired.
- Only current user-visible behavior, correctness boundaries, ownership
  invariants, and cross-repository interfaces belong here.
- Every implementation Bead names the contracts it changes, proves, consumes,
  hardens, or preserves and records exact child revisions.
- Historical execution evidence belongs in Beads and Git, not this current-state
  index. User-visible chronology belongs in `CHANGELOG.md`.
- Later work touching a proven owner reruns its indexed checks and advances the
  proof revision or records the remaining gap.
