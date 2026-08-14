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
| EON-C1 | Eon validates and launches one compatible component set and reports every exact component revision | Eon | Proven | `a2792cc77f8254cc277d64eb41f65725b69ccf70`; proof below |
| EON-C2 | One versioned manifest defines component identity, compatibility, and abstract artifacts—but not launch policy—for every distribution channel | Eon | Proven | `234ad77ced4924d95400b5c39822b3fb9b928c95`; proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Proven | `a2792cc77f8254cc277d64eb41f65725b69ccf70`; proof below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Proven | `a2792cc77f8254cc277d64eb41f65725b69ccf70`; proof below |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, traverse the topology directly, and have ended Sessions leave no dead pane or empty tab behind | Eon | Candidate | Accepted composition `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; Session-exit pruning `7ede475992528be1b6643035abe4da9560d50a21`; candidate `e77e842fe8c7070a96047dff1bf028ccbd49b788` |
| EON-C9 | Eon supplies one configurable exact managed environment across Nushell, Bash, Zsh, and Fish while native shell and tool configuration remain user-owned | Eon | Proven | Base `6dfcb473beccadd6e145235009240c81dd570fe5`; tool glyphs `fb95671d855fa944c3717103cb13bd0135f8aec8`; private Session PATH `c0d044c69318a921f9f9139bcaf2de9afce683d3`; Fish preservation `194076f66f91c8823b03c3ee6d3a1706eb8ea4e4`; proof below |
| EON-C10 | A local client can submit versioned Eon workspace and supervisor-lifecycle actions and receive one complete typed result without reconstructing hidden state | Eon | Proven | Accepted workspace actions `4af395aea06c230ee6b18cf0755ae25915c0b88d` and lifecycle actions `fd6b348494111a0d18e241787da14ea99ee117a9`; current CLI proof `d0c2208d628fa0dc1e186899b45e0b73534ff9bc`; deadline hardening `a390fc5c007900c3fc8c9c49df85f9b8acc06d4a`; proof below |
| EON-C11 | Eon and EonTerm default to the exact current generation in separate runtime namespaces while older live generations remain discoverable, explicitly presentable when compatible, and explicitly stoppable through their supervisor; presenting an existing surface requests native presentation | Eon | Candidate | Accepted base `3b84d83d807c6249efa340fabf9e3d0d0d3ef310` and attachment `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`; generated-runtime identity `4beb441301d84039570dd07138da2a6f70b3ed23`; stop owner `dbffad4018f0339bd3d35ea03579d0e09cff73bf`; startup convergence `36c95ad04cb9519f88b907642c8147d95f56ee83`; EonTerm lifecycle correction `63686a12b752c9423b2096d5e32aa5842f2184fc`; native activation candidate consumes Venus `50b7ef7f6c9d5b531b79ecca67c9c8fdf40f355f` |
| EON-C12 | A local user or composition can host one exact command in one native terminal surface without Eon workspace actions while Eon retains generation and lifecycle authority | Eon | Proven | Accepted product `816372ea9ffe90f08ff442b5763a3b5413b7906c`; vivid palette `285c6bf48fb402a673eb997594b6eb1bbcb1429b`; lifecycle correction `63686a12b752c9423b2096d5e32aa5842f2184fc`; proof below |
| EON-C13 | Eon applies one bounded terminal-background opacity from its canonical configuration whenever it creates an Eon or EonTerm Venus surface, while preserving live presentation and Orbit Session ownership | Eon | Proven | `b58f463849cc233ce8137737d61d61bee11b11ae`; proof below |

## Approved terminal-presentation contract EON-C13

- Consumer: one local Eon or EonTerm user and an approved composition that pins
  this contract.
- Trigger: Eon creates or reopens a Venus surface while
  `$EON_CONFIG_HOME/config.toml` contains `[terminal]
  background_opacity = VALUE`, or contains no such value.
- Result: Eon accepts one finite `f32` in `0.0..=1.0`, defaults absence to
  `1.0`, and passes the exact value to Venus as `--background-opacity VALUE`
  before its socket endpoints. Eon and EonTerm share the setting. A live Venus
  remains unchanged; after it exits, presentation reopens with the current
  value while the Orbit Session and PTY child remain live.
- Important failures: invalid configuration identifies
  `terminal.background_opacity` and starts no initial supervisor, Orbit, or
  Venus process. Invalid configuration read during presentation replacement
  returns a bounded failure without stopping Orbit or its child.
- Ownership: Eon owns the configuration schema, validation, default, apply
  timing, launch argument, generation policy, and exact Venus compatibility.
  Venus `VEN-C11` owns native validation and terminal-background
  materialization. Orbit remains unaware of opacity.
- Boundary: the setting affects only Venus's terminal default background and
  padding. It does not affect explicit cell backgrounds, text, cursor,
  selection, workspace chrome, decorations, input, hit testing, accessibility,
  or click-through. This slice adds no profile, public CLI override, watcher,
  live reload, Orbit restart, or macOS transparency claim.
- Update order: Venus source `74ab5a0b661210f0afec94086f5358fe50b01f05`
  proves `VEN-C11`; Eon then proves `EON-C13` against that exact revision; an
  approved composition may then map its product setting to Eon's field. There
  is no adapter, feature probe, dual write, or fallback.
- Approval: the user approved this contract on 2026-08-14 for
  `eon-eonterm-presentation-e4f`.

## Approved EonTerm contract EON-C12

- Consumer: one local Eon user or an explicitly approved composition that needs
  one native terminal surface without Eon's tab and pane model.
- Trigger: the consumer invokes `eonterm -- COMMAND...` with one non-empty
  exact argv. An optional `--no-decorations` before `--` requests a native
  surface without window-system decorations. `eonterm generations`,
  `eonterm attach`, and `eonterm stop` expose the same bounded EON-C11
  lifecycle within EonTerm's runtime namespace.
- Result: Eon starts one Orbit-owned Session for the exact command and launches
  Venus with only that Session endpoint. The supervisor remains the sole Venus
  process launcher: a repeated invocation preserves an attached surface and asks
  Venus to request native presentation. An invocation after detachment opens one
  replacement against the same live Session with the supervisor's original
  decoration choice. Later invocations do not mutate a live supervisor's choice.
  The current runtime generation remains authoritative for presentation, inspection,
  explicit older-generation selection, owner-routed stop, concurrent-launch
  convergence, child exit, and cleanup.
- Important failures: a missing command, incompatible or corrupt supervisor,
  component mismatch, partial startup, or a live supervisor in the other launch
  mode fails explicitly. Workspace actions against EonTerm mode return the
  bounded `workspace-unavailable` failure and create no hidden topology.
- Ownership: Eon owns launch mode, generation, composition, and lifecycle.
  Orbit remains the sole owner of the process, PTY, terminal state, Session,
  and transport. Venus remains a transient native presentation and input client.
- Boundary: EonTerm adds no Workspace value, tabs, panes, key remapping,
  preset, wrapper process, persisted mode marker, remote access, compositor-specific
  focus mechanism or guarantee, or Zellij policy. It exposes no workspace,
  configuration, managed-tool, or default-shell command. Bare `eon` and
  `eon run` retain their workspace meanings. Decorated surfaces remain the
  default for Eon and EonTerm. EonTerm uses its own default runtime namespace
  and consumes no EON-C9 managed-environment policy.
- Approval: the user approved the lifecycle on 2026-08-10 for `eon-4is.1` and
  the permanent EonTerm product and command identity on 2026-08-12 for
  `eon-4is.4`.

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
  owner-authored attach and stop availability. The CLI JSON projection emits
  each opaque endpoint as an ordered integer byte array without changing EONW.
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

- Consumer: one local Eon or EonTerm user and the explicit generation
  inspection, attach, and stop commands.
- Trigger: an attach-capable product launch, or the user lists, attaches to, or
  stops one exact generation in that product's runtime namespace.
- Result: Eon derives one opaque distribution-neutral identity from its runtime
  source, dependency lock, canonical component manifest, and EONW source. Each
  product selects a separate runtime namespace, and each generation owns a
  private directory below it. An implicit attach selects only a live supervisor
  that authoritatively reports the exact current identity; otherwise it starts
  the current generation without stopping older work. Discovery distinguishes
  current, previous, legacy, dead, incompatible, and corrupt candidates. Explicit attach
  never substitutes another generation. Explicit stop is sent through the
  selected generation's EONW owner and names every affected live Session.
  Concurrent attach-capable launches for one generation converge on one
  supervisor and one initial Session. Presenting a generation with a live Venus
  child preserves that process and its Sessions, then sends one bounded private
  signal for Venus to request native presentation of its existing window.
- Important failures: an invalid component graph, unsafe permissions, symlinks,
  invalid names, corrupt or oversized responses, unsupported EONW, identity
  mismatch, unavailable legacy lifecycle support, timeout, partial startup,
  unavailable presentation control, or an incompatible competing launch produces
  a bounded explanation. Wayland does not support direct focus or unminimize
  through locked winit; a compositor may decline a native attention request.
  Failure or
  cancelled confirmation preserves every Session. Eon removes only its own dead
  control socket or an empty stopped generation directory and never kills a
  process inferred from a PID, pathname, or process tree.
- Ownership: Eon owns generation identity, product namespace selection, discovery,
  compatibility policy, supervisor-routed presentation and stop, and bounded
  cleanup. Each live supervisor remains authoritative for its topology and
  lifecycle response.
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
  listener acquisition, validation, and cleanup tests; concurrent process and
  A-to-B upgrade tests; locked Rust and Nix checks; and live profile-upgrade
  dogfood that preserves the older workspace.
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

- Accepted refresh proof `59d8cebf9392051a5b37ac09dc23e90ad7443209` on
  x86_64 Linux: schema 3 graph
  `components/eon-alpha-v3.json`, SHA-256
  `b694e85cc89e7d18b2319bcfc6be8a81403b15778c579732de42eb70263ea7d8`,
  selects accepted ORBS v3 Orbit source `6de95296d252c119d4fdba2d9b03cec1a09355ae`;
  uses ORB-C1/ORB-C2 proof `9bc87191dd90fd3d7db939127f1ebedfccd2b48d`
  and that source for ORB-C3 through ORB-C11; selects Venus source and VEN-C1
  proof `74ab5a0b661210f0afec94086f5358fe50b01f05`; retains VEN-C2 proof
  `846daf8fb7846b0e8dc227e533aa8d51a691f76f`, ORBF v1, ORBS v3, EONW v1,
  and every unrelated identity. Exact packages are
  `/nix/store/2rvbvzprjrxdj1h260kny5qfqa8l87yc-eon-0.1.0` and
  `/nix/store/df186d9d92l2vvlfnpwa1lcax1dbhqgi-eonterm-0.1.0`; Eon NAR hash
  `sha256-0cMyGlIZEAVjOSq0beVBRlFsDkkzDsv9Hah2NJRhSm8=`; generation
  `g1-d81f6398c46f9f0aefb7de56b5922854`. Locked Rust, manifest/report, exact
  graph, Nix flake, and both-package build checks pass. The active `eon` profile
  resolves the exact Eon package without changing recorded live process
  identities. ORBS v4 and VEN-C11 activation remain outside this refresh.
- Current EON-C1 through EON-C4 composition proof:
  `0d86cf0488f7899ce565de39347b0420a0cb9129` on x86_64 Linux. The EON-C2
  graph proof remains `da0d160801c5db6b6b982967b5bdc28a11f9b5fb`. Schema 3 graph
  `components/eon-alpha-v3.json`, SHA-256
  `9c2145ab34b1ca8dfa3dea00e4fb301bac3ebc67b29255fdbc9d11b5b9f41254`,
  selects Orbit source with ORB-C2 and ORB-C6 proof at
  `f4f0b0a82333d088ad40e2b63108d4905466e8f2`, retains ORB-C11 and canonical
  ORBS v3 proof at `3ee7c80005f3d2bbe81e539799327803716f6174`, selects Venus source and
  VEN-C1 proof at `457b3da837f07d186a99ca20930f2146c3d78f36`, retains VEN-C8 and VEN-C10 at
  `9c56eb17613e10ef7712a1852b049ed63cf22b18`, while retaining VEN-C13 at
  `2d3498258920736eb1bdae2b8869b6547b9735d4`. Generation:
  `g1-01e141bdbaae3253f716b755fe9e210b`.
- Exact full Eon package: `/nix/store/m5wcv3d41658srcfin4r0whrrllw10dg-eon-0.1.0`,
  NAR hash `sha256-EfSvrh8zfJB13krVc2ZsDSMAf3EpLiIYEdyf1yqhUQE=`. The active
  profile resolves this artifact without restarting live supervisors or Sessions.
- Eonova rollout source `c4e68e77301ea5d1816d955587c162be7524dff4` packages exact EonTerm
  `/nix/store/bsrcyizp1km7d3i9llylxyp2227nvfbb-eonterm-0.1.0` with unchanged
  Mars-free Nova at `/nix/store/d0hchzi9kcz2mkxf9n1lcgvbxj4a21jf-eonova-0.1.0`,
  NAR hash `sha256-C2qyrJ+tTYn9UxJ84TTNPbJdEzEWg7tmjkeT7nNG1As=`. Native COSMIC
  Wayland dogfood renders adjacent full blocks without seams or color bleed.
- Prior terminal-clipboard-write composition proof:
  `0e25ebc2311d7e41edf90c940f8211dd5839bb83` on x86_64 Linux. Schema 3
  graph `components/eon-alpha-v3.json`, SHA-256
  `351c406d5439029bbbefec2151cbf1eca5f3bf2d7b1c821cb5c36f5ae328d285`,
  selects Orbit source, ORB-C11, and canonical ORBS v3 proof
  `3ee7c80005f3d2bbe81e539799327803716f6174` plus Venus source and VEN-C13
  proof `2d3498258920736eb1bdae2b8869b6547b9735d4`, while retaining every other
  accepted component proof. Generation: `g1-dd77de2a596dccc968f08127cb801081`.
- Exact package: `/nix/store/0q1fsf7mjijvgxzj3z94vf80xjjgf9dn-eon-0.1.0`,
  NAR hash `sha256-GDkZsHi+D1udfusoKpgvzCKANlWgEbkRX4HUC1qCgRQ=`. The active
  profile resolves this artifact without restarting live supervisors or Sessions.
- Eonova rollout source `4fda9b67b0faa33561624633229135e5e2d579ea` packages the exact Eon
  artifact at `/nix/store/hgvf1lr9i0sc1ql3qq0nhf9y81wzgkkj-eonova-0.1.0`,
  NAR hash `sha256-nsxak7RGzTpWBd+WOtvXl4olsUvK4dh+bDHYtBGfJnc=`, with the
  unchanged Mars-free Nova input. Its active profile resolves that artifact.
- Proof revision: `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c` on x86_64
  Linux through the Nix alpha package `eon-0.1.0` at
  `/nix/store/04sbq6wc79h2hf1rjq9y9bcvaz5v92hb-eon-0.1.0`, NAR hash
  `sha256-uSidJ5Xf047x0vxjiscmnAHlVZiIvm0ugN6mQldawpc=`.
- Prior incomplete EON-C2 proof: `60e9145aa204d387a28d917be8a4fe1f5ddcd4ef`
  on x86_64 Linux, packaged at
  `/nix/store/n5vd1xr219b5xhmsf7vb68bwk93bwl6v-eon-0.1.0`, generation
  `g1-61f104609121df6698b476433a852dd9`, NAR hash
  `sha256-9VKowtFbrmqR4F7GGrGQQ0yDTS9u2wFoZbQVvIkdsbs=`.
- Component graph at that proof: `components/eon-alpha-v2.json`, SHA-256
  `cfe9ccc4932e7260afa17bbdec211cfafadc2e7a59e6bf349983840be19edf5b`.
  It selects Orbit source `64db581445bafca1a08a6530f8e44f9c1edbc169`,
  ORBF v1 and ORBS v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`,
  ORB-C10 proof `292b2451c9a1d99390334771a681c7f481c996f2`, and Venus source
  `90988f6ebcde68338e202a9c637c59398aafe93d` with VEN-C1 proof
  `e7bda96822274727faacb51731ae19181295e1cd`, VEN-C3 and VEN-C4 proof
  `6ef19afddaedcbe2b9ed0996e31bc02df5d854ac`, VEN-C8 proof
  `33a3d9af9f4c6015301ad6829fe733413c5b683d`, and VEN-C9 proof
  `90988f6ebcde68338e202a9c637c59398aafe93d`.
- Checks: canonical manifest validation and removed-field mutations; locked Rust
  format, check, 34-test, and Clippy suites; exact Eon and Eonova Nix builds and
  flake checks; Eonova Mars/Rio closure, desktop, license, and wrapper checks;
  invalid-graph command matrix; installed version, commit, profile, and store-path
  comparison; workspace and presentation-owner regressions; README LOC; and
  `git diff --check`.
- Dogfood: an isolated Eonova launch delivered exact terminal-emitted UTF-8 through
  the Wayland primary selection, restored the prior selection, and stopped only its
  isolated namespace. The profile refresh preserved the three live Eonova processes.

### EON-C12 — EonTerm exact-command lifecycle

- Vivid-palette proof revision: `285c6bf48fb402a673eb997594b6eb1bbcb1429b`
  on x86_64 Linux; runtime generation `g1-158c60e05545becfe824b29fb31c7637`.
  The active EonTerm profile resolves
  `/nix/store/if65frajh884m0fjn06kvpxa0hrzn097-eonterm-0.1.0`, NAR hash
  `sha256-vokhFgnu2vY7YRsRrmJmz0tDzEVrnBv+GIh4xVqgRDQ=`.
- Native dogfood: one isolated COSMIC Wayland EonTerm surface rendered the
  accepted ANSI indices 0 through 15 as distinct vivid swatches. Structured
  stop removed only that generation. Four pre-existing Eonova processes kept
  their prior store paths across the profile refreshes.
- Accepted product revision: `816372ea9ffe90f08ff442b5763a3b5413b7906c`;
  lifecycle correction: `63686a12b752c9423b2096d5e32aa5842f2184fc`
  on x86_64 Linux; runtime generation
  `g1-7aac2def4359287dc80d068f91ecb7cc`.
- EonTerm artifact: `/nix/store/nmacs55wiqmz8qyf26wykipmgraqgdm3-eonterm-0.1.0`,
  NAR hash `sha256-Pv0U16WrOPAaEHEXubqjf7WIJDInudY51pSOOhh3evc=`, NAR size
  1,686,888 bytes, closure size 1,139,215,720 bytes. Full Eon is
  `/nix/store/q0ql16r9amlwj1n6ipbgs8drr9n51ifv-eon-0.1.0`, closure size
  1,824,446,960 bytes.
- Checks: locked format, 34-test workspace, and Clippy suites; canonical
  manifest validation; exact-revision full flake check; both packages and apps;
  full Eon managed checks; exact Orbit/Venus and managed-tool closure exclusion;
  basename-selected command, separate namespace, lifecycle routing, exact
  reconnect guidance, and removed-name regressions; `git diff --check`; and
  README LOC.
- Exact installed dogfood: the corrected CLI listed published generation
  `g1-13665e1746ba54269a4d66cfe646324f` as previous and live, attached Venus
  491558 to its unchanged Orbit 491527, and stopped it through supervisor
  491522. A corrected supervisor 491577 launched Orbit 491582 and Venus 491588,
  then reported `eonterm attach g1-7aac2def4359287dc80d068f91ecb7cc`
  after Venus detached. Structured stop removed both isolated generations and
  every child with status 0. Named `eon` and `eonterm` profiles resolve to the
  exact artifacts above; existing Eon and Eonova processes retained their PIDs
  and start times without a restart.
- Native dogfood at `4353e94ebc015453c77914a2be013a950f49c8cc`: one COSMIC Wayland surface
  forwarded Alt-h/j/k/l/m and Ctrl-t as exact bytes `1b 68 1b 6a 1b 6b 1b 6c 1b 6d 14`. After the
  supervisor-owned client detached, a second invocation reopened the same
  Session; command exit closed both invocations, removed the generation, and
  left no Venus process.
- Installed Eonova dogfood at `e77e842fe8c7070a96047dff1bf028ccbd49b788`
  launched Nova through `--no-decorations` in an isolated COSMIC Wayland
  namespace and showed no native title bar. The namespace was stopped through
  its supervisor without touching existing Eon or Eonova generations.
- Remaining gap: this proof makes no macOS claim.

### EON-C13 — terminal-background opacity

- Proof revision: `b58f463849cc233ce8137737d61d61bee11b11ae` on
  x86_64 Linux. Canonical schema-3 graph SHA-256
  `76252ee2ccbca91694526117b35ebd47a8bb5d6b31490d775dad4432c212cbe7`
  selects Venus source and proved `VEN-C11`
  `74ab5a0b661210f0afec94086f5358fe50b01f05`, unchanged Orbit source
  `6de95296d252c119d4fdba2d9b03cec1a09355ae`, ORBS v3, and EONW v1.
- Exact artifacts: `/nix/store/nsz17zka86yb1z8lwa968kby5117913h-eon-0.1.0`,
  NAR hash `sha256-W+rr0uFvIoAcGIfoIrj/JCqIR7VNoS8zLmpucE8roKo=`, NAR size
  1,981,280 bytes and closure size 1,824,543,536 bytes; and
  `/nix/store/sxvgmdgc96yqr3qyzr1fw2qalf2zxj0h-eonterm-0.1.0`, NAR hash
  `sha256-pJGH1pwuxJgK7Y4CTfK3eGJRtCi7/PeKxa41KxVhu6Q=`, NAR size
  1,725,528 bytes and closure size 1,139,312,296 bytes. Both report generation
  `g1-4ab6a37dee3542f0586275646af4625a`.
- Checks: TDD first rejected the missing configuration accessor and launch
  input; focused parser, argv, failure-ordering, and process-lifecycle checks;
  locked format, check, 38-test workspace, and all-target Clippy suites;
  canonical manifest validation; all five Nix flake checks; both exact package
  builds; README LOC; and `git diff --check` pass.
- Exercised behavior: no file and absent field pass `--background-opacity 1`;
  `0.0`, `0.88`, and `1.0` are accepted; non-finite, out-of-range, wrong-type,
  duplicate, and unknown terminal fields identify
  `terminal.background_opacity`. Initial rejection creates no generation,
  socket, Orbit, or Venus process. Every decorated, undecorated, workspace,
  EonTerm, and legacy Venus launch puts the exact flag before socket endpoints.
- Installed COSMIC Wayland dogfood launched decorated Eon at the opaque default,
  then undecorated EonTerm at `0.88`. After Venus exited, an invalid `1.01`
  edit returned a bounded presentation failure while supervisor, Orbit, and
  child PIDs `1970972`, `1970978`, and `1970979` retained their start times. A
  `0.0` edit reopened Venus PID `1972363` against those same processes. Exact
  supervisor stop removed only the isolated generation; the fixture moved to
  trash. The accepted Venus proof supplies native resize, reconnect, selection,
  IME, accessibility, background-layer, decorated/undecorated, and compositor
  behavior for the exact consumed child source.
- The active `eon` profile resolves the exact artifact above. Existing Eonova
  supervisor, Orbit, and Venus PIDs `1769216`, `1769218`, and `1769231` retain
  their 2026-08-13 23:11:51 start time; no live process was restarted.
- Dependency disposition: no dependency, profile framework, watcher, public
  CLI option, environment-per-setting boundary, or second graph was added.
  Broader Linux compositor behavior and macOS transparency remain limited by
  `VEN-C11`; Eon makes no wider platform claim.

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
- Fish-completion correction: `194076f66f91c8823b03c3ee6d3a1706eb8ea4e4`
  on x86_64 Linux. Artifact `/nix/store/bxh2j3dncrjxlgg01jga2n6jkk38jkq2-eon-0.1.0`,
  generation `g1-67aff726fa2116fa13e7adc1c560d083`, NAR hash
  `sha256-cO36qXeHO1EN0wdCuBjYpTM0EULvgA/VWp+qiGXa+MM=` (1,933,512 bytes;
  closure 1,824,265,896 bytes). Its exact archive passed the 34-test locked Rust
  matrix, schema-3 validation, focused Fish regression, full Nix checks, and
  installed enabled/disabled wrapper checks. Native and fallback completions
  coexist, repeat activation is stable, and disabled Carapace remains inert.
- Tool-glyph proof revision: `fb95671d855fa944c3717103cb13bd0135f8aec8`
  on x86_64 Linux. Artifact
  `/nix/store/dirnp4ri16smqcr2vd00wkp454b27kcg-eon-0.1.0` has NAR hash
  `sha256-u6Q8XqSXdzsqe+v2mJfUD0CMVTBKFuCQ5O2a4uXW+j0=`, NAR size 1,880,008
  bytes, and closure size 1,824,211,920 bytes.
- Tool-glyph checks: exact Nix build and flake check; installed wrapper,
  fontconfig, closure, face, and representative private-use charset inspection;
  fresh managed Yazi dogfood; and `git diff --check`.
- Private Session command-path proof revision:
  `c0d044c69318a921f9f9139bcaf2de9afce683d3` on x86_64 Linux.
- Artifact: `/nix/store/j0mgxpx9s7rsqhxzhqsxg2vmb413n2gn-eon-0.1.0`,
  generation `g1-da0fcf014f5fd6bd9755505e6c2e5d34`, NAR hash
  `sha256-AOoech5U7yhz/HmS1eB9C5M2EwROvGANJg7sJhBL3CI=`, NAR size 1,939,312
  bytes, and closure size 1,824,271,384 bytes.
- Checks: one exact-package private-PATH check compares every prefixed managed
  shell with its pinned unprefixed alias and launches the default `eon-nu`
  through packaged Orbit; locked Rust format, check, 31-test, and Clippy suites;
  canonical manifest validation; exact Nix build and flake check; and
  `git diff --check`.
- Exercised behavior: the Session-private PATH resolves `eon-nu`, `eon-bash`,
  `eon-zsh`, and `eon-fish` without an ambient profile while preserving the
  unprefixed aliases, exact selected programs, and one private PATH prefix.

### EON-C10 — EONW v1 producer and CLI projection

- Producer proof revision: `4af395aea06c230ee6b18cf0755ae25915c0b88d`.
  Exact CLI endpoint-byte projection: `858e840cc5a7fd235ae62376172424a25cc1f422`
  on x86_64 Linux. Current local-client proof:
  `d0c2208d628fa0dc1e186899b45e0b73534ff9bc` on x86_64 Linux.
- Artifact: `/nix/store/bc26a5gbzw2d8rd9nikbi4310i2m3dlj-eon-0.1.0`,
  generation `g1-ab14af5dc43b561febe6bbd4e370e2d5`, NAR hash
  `sha256-UN+jn2QaFqyQdsH2fKa3+NbMuAEkiJ4dlGEPi38fjM4=`, NAR size 1,947,416
  bytes, and closure size 1,824,510,712 bytes.
- Checks: locked workspace format, check, 35-test, and clippy suites; canonical
  manifest validation; exact-revision Nix build and flake check; installed
  invalid-UTF-8 endpoint-byte reconstruction; and `git diff --check`.
- Exercised behavior: one mode-`0600` control socket carries bounded complete
  requests and responses; the shared codec rejects incompatible or malformed
  input; every accepted inspect or semantic action returns a complete snapshot;
  empty and overlong focus identities fail locally as `malformed-action` before
  any endpoint connection; valid missing-supervisor classification remains
  unchanged; rejected input preserves the accepted workspace; and successful CLI
  JSON preserves each exact opaque endpoint byte as an ordered integer array.
- Deadline-hardening proof revision: `a390fc5c007900c3fc8c9c49df85f9b8acc06d4a`
  on x86_64 Linux. The client I/O deadline derives from the longer accepted
  Session-start work bound, and one delayed action returns its committed
  two-Session snapshot.

### EON-C10 lifecycle and EON-C11 — runtime generations

- Lifecycle protocol proof: `fd6b348494111a0d18e241787da14ea99ee117a9`.
  Accepted EON-C11 attach proof: `ece0f1bc151eeccd15b0172c5fbdbe8ab32c502c`
  on x86_64 Linux. Generated managed-runtime identity proof
  `4beb441301d84039570dd07138da2a6f70b3ed23`; stop-owner proof
  `dbffad4018f0339bd3d35ea03579d0e09cff73bf`; startup-convergence proof
  `36c95ad04cb9519f88b907642c8147d95f56ee83`; EonTerm namespace proof
  `63686a12b752c9423b2096d5e32aa5842f2184fc`; native presentation proof
  `9942aa87e56da022bbcf9ae0e32b0ce5b640505d` with exact Venus
  `50b7ef7f6c9d5b531b79ecca67c9c8fdf40f355f`.
- Artifact: `/nix/store/wyf20989i804v6gn0k761g4xkivwk19x-eon-0.1.0`,
  generation `g1-216e217e59b4fd6738b148d4bc82b45e`, NAR hash
  `sha256-p4wvvG3zhucdQbg0My6RUiEjIYpmFDrgxKQIIWDvHL0=`, NAR size 1,947,272
  bytes, and closure size 1,824,510,568 bytes.
- Checks: locked format, check, 33-test, and Clippy suites; canonical manifest
  validation; exact Nix build and flake check; deterministic listener-acquisition,
  generation, presentation-capability, generated-behavior-identity, and
  replacement-owner tests; 200 consecutive real-process concurrent starts;
  installed two-launch convergence dogfood; retained live upgrade dogfood at
  `3b84d83d807c6249efa340fabf9e3d0d0d3ef310`; profile comparison; and
  `git diff --check`.
- Exercised behavior: bare launch selects only the exact current generation;
  concurrent starts produce one reachable supervisor and one initial Session;
  current and fixed-namespace legacy work coexist and can be presented explicitly;
  supervisor-routed stop names and removes only the selected owner and refuses a
  replacement without touching its Sessions; exact attach and stop ignore unrelated
  over-limit generation entries; profile refresh or generated managed-runtime change
  selects a fresh generation while leaving older supervisors and Sessions running.
  Installed Eonova `/nix/store/39rqav6bkspfkyvdv4p3b6i1jy258l33-eonova-0.1.0`
  on native COSMIC Wayland preserved supervisor, Orbit, and Venus PIDs across a
  repeated desktop launch; the locked Venus Wayland path uses xdg activation.

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
