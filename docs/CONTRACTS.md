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

The current composition candidate selects Orbit
`199ee1a6f00efb8a080b31ffbfc7c89b2e857ee6`, canonical ORBS v6 proof
`780f5d746175b4a9b71df57c51ed4bfcc4c4c375`, and Venus
`e5e37a3df119ee2bcfa2493a2ce5a46307493732`. `EON-C1` through `EON-C4`
retain their prior proof records but remain Candidate until the exact installed
profile and native Wayland behavior are accepted.

## EON-C1 — Exact compatible component launch

- **Status:** Candidate
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
- **Important failures:** Missing, malformed, incompatible, replaced, or
  non-ready components fail before publishing a usable workspace or terminal.
- **Owner:** Eon launch validation, orchestration, icon selection, and desktop
  metadata; Nix only derives and installs artifacts; child components own their
  internal readiness and runtime state.
- **Consumes:** The versioned Eon component manifest, Orbit management/ORBS, and
  Venus native launch contracts.
- **Boundary:** Eon does not infer compatibility from executables, store paths,
  process names, or moving branches.
- **Proof:** `871c9f639e519c7e3201fa5d9755d0a77a4cb0df`
  - **Environment:** Nix-built x86_64 Linux Wayland alpha
  - **Evidence:** Exact graph validation, Ready-boundary failure/recovery,
    component revision reporting, and composed package checks; accepted base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70`, ORB-C13/ORBS v4 refresh
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, and Ready base
    `7cbcf1d4ce8ac186dc3ceff48240f04c437709af`; icon selection
    `32936b92e9c20a000d21f123869e0b749ff39618`, canonical master SHA-256
    `33ec3062f72a732290dcd6c6f40a2d5f535d6a7cbfcaf7455a0a5c4f03a52e0d`,
    and launcher action `32c768e0d9a62984640a516c34d0cb0db16e8adb`

## EON-C2 — One component compatibility graph

- **Status:** Candidate
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
- **Proof:** `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`
  - **Environment:** x86_64 Linux Nix alpha
  - **Evidence:** Manifest parser/compatibility checks and exact current graph
    consumption by Eon and EonTerm; accepted manifest base
    `234ad77ced4924d95400b5c39822b3fb9b928c95`

## EON-C3 — Distribution-neutral runtime inputs

- **Status:** Candidate
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
- **Proof:** `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`
  - **Environment:** Nix-built x86_64 Linux alpha
  - **Evidence:** Exact protocol-source substitution, package tests, and
    evaluator-absence checks; accepted launch-input base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70`

## EON-C4 — Thin orchestrator ownership

- **Status:** Candidate
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
- **Proof:** `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`
  - **Environment:** x86_64 Linux composed alpha
  - **Evidence:** Component-boundary checks, management consumer proof, package
    closure inspection, and composed runtime dogfood; accepted composition base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70`

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
- **Trigger:** Launch, create tab or pane, traverse focus, receive Session exit,
  or recover accepted same-boot runs.
- **Result:**
  - Independent durable Sessions appear as horizontal tabs containing vertical
    accordion panes with exactly one expanded pane.
  - Each pane is identified as `pN`, two ASCII spaces, and a compact working-
    directory label; home uses the packaged marker, descendants use `~/`, and
    external paths remain absolute.
  - Users can traverse tabs and panes directly and create new panes or tabs
    through the accepted semantic actions.
  - Ended Sessions leave no dead pane or empty tab; focus moves deterministically
    to the same identity when possible, otherwise the following sibling at the
    removed index, otherwise the preceding sibling; an empty workspace closes
    Venus and the supervisor.
  - Same-boot recovered runs project numerically into `tab-1` as `pN` without
    claiming restoration of prior topology.
  - New current-generation full Eon surfaces omit the redundant native title
    bar while retaining the selected terminal title for compositor semantics.
- **Important failures:** Unknown or stale identity, invalid transition,
  direction without a target, unavailable endpoint, duplicate identity, or
  partial recovery leaves the last accepted topology unchanged. Exhausted pane
  numbers fail before Session start; unknown or repeated Session-exit notices
  remove nothing; losing a view never silently stops or substitutes a Session.
- **Owner:** Eon workspace topology, identity, focus, pruning, and action policy;
  Orbit owns Sessions and Venus owns native materialization.
- **Consumes:** EONW v1, Orbit Session identities/endpoints, and Venus `VEN-C8`.
- **Boundary:** No arbitrary split tree, simultaneous expanded panes,
  reordering, picker-based ordinary traversal, durable layout restoration,
  per-pane Session stop/restart, terminal content/history, provider state,
  plugin surface, remote access, or reconstructed Session state.
- **Proof:** `e77e842fe8c7070a96047dff1bf028ccbd49b788`
  - **Environment:** x86_64 Linux candidate with installed compact-header and
    Session-exit dogfood
  - **Evidence:** Workspace composition, traversal and creation, `pN` projection,
    exact compact path labels, accessibility parity, and exit pruning; accepted
    composition `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`, exit pruning
    `7ede475992528be1b6643035abe4da9560d50a21`, and compact identity/header
    correction `cfcb38e6e711e971ed6004528987761ddd7c87e4`
- **Open proof:** Final current-source acceptance revision remains pending.

## EON-C9 — Managed shell environment

- **Status:** Proven
- **Consumer:** Eon Sessions launched under Nushell, Bash, Zsh, or Fish.
- **Trigger:** Eon constructs a child launch environment.
- **Result:**
  - Eon selects exact Nushell, Bash, Zsh, Fish, Starship, Zoxide, Atuin,
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
- **Proof:** `194076f66f91c8823b03c3ee6d3a1706eb8ea4e4`
  - **Environment:** x86_64 Linux Nix alpha
  - **Evidence:** Four-shell environment checks, private Session PATH, tool
    glyphs, Fish preservation, and packaged command dogfood; accepted base
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
    and selected identities, liveness, and exact opaque Orbit endpoint bytes.
    Lifecycle results carry generation, component/protocol identity, live
    Sessions, and owner-authored attach/stop availability.
  - CLI JSON emits opaque endpoints as ordered byte arrays without changing
    EONW. Each connection carries one length-delimited request and response and
    does not use EOF as framing.
- **Important failures:** Malformed, oversized, incompatible, unavailable,
  rejected, timed-out, partially read, or response-lost operations fail within
  the shared bound without reconstructing hidden state or reviving completed
  Stop. Accepted Stop names affected live Sessions, sends canonical Stop through
  every validated management lease before waiting, and returns only after every
  exact terminal record is complete.
- **Owner:** Eon supervisor workspace/action state, EONW result ordering, and CLI
  projection.
- **Consumes:** EONW v1 and accepted Orbit management operations.
- **Boundary:** Additive lifecycle tags do not widen the pinned Venus workspace
  consumer. There is no event stream, subscription policy, remote transport,
  plugin/MCP API, authorization framework, durable restoration, terminal
  content, or direct child-protocol escape hatch.
- **Proof:** `abf2513b51c8871b5bfea602e8369bd2596609ef`
  - **Environment:** x86_64 Linux packaged and installed lifecycle checks
  - **Evidence:** Workspace actions, CLI projection, absolute connection/read
    deadlines, management Stop, response-loss finality, and source ownership;
    workspace actions `4af395aea06c230ee6b18cf0755ae25915c0b88d`, lifecycle
    actions `fd6b348494111a0d18e241787da14ea99ee117a9`, CLI proof
    `d0c2208d628fa0dc1e186899b45e0b73534ff9bc`, deadline hardening
    `a390fc5c007900c3fc8c9c49df85f9b8acc06d4a`, management Stop
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, and response-loss
    correction `56fcae2d00baecf9b69e4650882a69e50419f56b`

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
- **Proof:** `5631d8dc4de49bfd3831aa3abef300b7734ad36c`
  - **Environment:** x86_64 Linux packaged and installed recovery tests
  - **Evidence:** Separate namespaces, repeated launch, native Present,
    management Stop, supervisor-loss replacement, and dead-residue correction;
    accepted base `3b84d83d807c6249efa340fabf9e3d0d0d3ef310`, attachment
    `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`, generated identity
    `4beb441301d84039570dd07138da2a6f70b3ed23`, Stop owner
    `dbffad4018f0339bd3d35ea03579d0e09cff73bf`, convergence
    `36c95ad04cb9519f88b907642c8147d95f56ee83`, EonTerm lifecycle
    `63686a12b752c9423b2096d5e32aa5842f2184fc`, same-boot adoption
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, Venus presentation
    `50b7ef7f6c9d5b531b79ecca67c9c8fdf40f355f`, and readiness correction
    `cfcb38e6e711e971ed6004528987761ddd7c87e4`
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
    projects them into `tab-1` as `pN` with the lowest selected.
  - New creation continues at checked `max(N)+1`; prior grouping, focus, argv,
    and request history are not restored. After recovered `tab-1`, later
    creation starts at `tab-2`.
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
- **Proof:** `5631d8dc4de49bfd3831aa3abef300b7734ad36c`
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
    correction `cfcb38e6e711e971ed6004528987761ddd7c87e4`
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
