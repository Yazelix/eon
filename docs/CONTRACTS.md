# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Candidate` means
the working tree is implemented and verified but has no proof-bearing Git
revision. `Partially proved` names an exact verified slice and its remaining
gap. `Proven` requires an exact check, component revisions, platform, and
artifact.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| EON-C1 | Eon launches one compatible component set and reports every exact component revision | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; direct-workspace composition proof below |
| EON-C2 | One versioned manifest defines the component graph for every distribution channel | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; direct-workspace composition proof below |
| EON-C3 | Eon receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; direct-workspace composition proof below |
| EON-C4 | Eon preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; direct-workspace composition proof below |
| EON-C5 | A user can install, upgrade, inspect, and remove a direct Eon bundle without replacing an existing unrelated toolchain | Eon | Planned | None |
| EON-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Eon | Planned | None |
| EON-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Eon and Venus | Planned | None |
| EON-C8 | A user can organize independent durable Sessions as horizontal tabs containing vertical accordion panes, keep one pane expanded, and traverse the topology directly through Eon-owned semantic actions | Eon | Proven | `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`; direct-workspace composition proof below |
| EON-C9 | Eon supplies one exact managed interactive environment through prefixed external commands and Session-private unprefixed tool names without changing the user's global toolchain | Eon | Proven | `99c410a1081b88dd8db2b7f9e26394a38acd175a`; managed-environment proof below |
| EON-C10 | A local client can submit versioned Eon workspace actions and receive one complete accepted workspace snapshot without reconstructing topology or terminal state | Eon | Proven | `4af395aea06c230ee6b18cf0755ae25915c0b88d`; EONW v1 producer proof below |

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
- Approval: explicitly approved by the user on 2026-08-08. Eon ownership,
  native Venus materialization, and exact composition are proved separately.
  Eon composition `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2` pins and proves
  Venus's accepted direct navigation, pane creation, tab creation, fitting
  pane-header materialization, and bounded external-mutation refresh.

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
- Update order: Eon producer `4af395aea06c230ee6b18cf0755ae25915c0b88d`
  preceded base consumer `932be5ce68a42b72e8b0bf5953c74f7a8af51f1e`
  and refresh consumer `2dfe364fdc6a86ec183bf846d1b233e7a64de0dd`, then direct
  workspace consumer `33a3d9af9f4c6015301ad6829fe733413c5b683d`; Eon composition
  `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2` selects the latter and supplies
  its workspace socket.
- Approval: explicitly approved by the user on 2026-08-09 by directing Eon to
  fix the named versioned workspace snapshot/action prerequisite.

## Approved desktop-icon identity under EON-C1

- Consumer: the Eon README, Linux desktop entry, launcher, dock, and app switcher.
- Trigger: a surface displays Eon's canonical application icon.
- Result: `assets/eon.png` is the clean transparent canonical master prepared from
  `assets/icon-concepts/icon_eon_4.png`. Nix derives exact raster sizes for the
  hicolor theme; other retained concepts are not runtime identities.
- Important failures: the master contains an opaque background or isolated
  off-palette edge pixels, a native launcher size gains a bright fringe, an
  installed raster differs from its build output, or desktop metadata stops
  naming the canonical `eon` icon.
- Ownership: Eon owns the canonical master and product selection. Nix owns only
  deterministic size derivation and installation from that master.
- Approval: the user selected prepared `icon_eon_4.png` on 2026-08-09 after
  native-size inspection found cleaner low-frequency geometry and edges.

## Proof record

### Direct-workspace composition proof `3b8e5f884f156d3d7f7ee294fb4e1c5d8c6cb5d2`

- Contracts: EON-C1 through EON-C4 and EON-C8 are proved at this exact
  revision; EON-C9 and EON-C10 retain their existing owner proofs.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `f73a8a00c6c9430eff7f8846746dff872f2957ce43e7692e9a03eca304a84312`,
  selects Venus and VEN-C8 proof
  `33a3d9af9f4c6015301ad6829fe733413c5b683d`. Orbit advances to accepted source
  `64db581445bafca1a08a6530f8e44f9c1edbc169` while retaining ORB-C1 through
  ORB-C9 and ORBF v1/ORBS v2 proof
  `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757` and ORB-C10 proof
  `292b2451c9a1d99390334771a681c7f481c996f2`; its change from the prior source
  revision touches only policy, planning, and proof documentation.
- Package: `/nix/store/2cywvf377bbxbwk34kds5a37jr1a5xq0-eon-0.1.0`, NAR hash
  `sha256-chr5FD0q429sO7zVg7CpYEBU0c4kzSZzmeCq44/kkC0=`, NAR size 1,301,496
  bytes, closure size 1,559,511,768 bytes. Its exact Orbit and Venus artifacts
  are `/nix/store/ixny4cqaarxq0694vsxd0w02fcj19pzs-yazelix-orbit-0.1.0` and
  `/nix/store/mxy0fmsx46xvmn4zaiby3qdlg6g0r1gg-yazelix-venus-0.1.0`.
- Checks: canonical manifest validation; locked Rust format, check, 17-test,
  and clippy suites; clean exact-revision Nix build and uncached flake
  evaluation; exact version report; LOC and `git diff --check`. The Nix build
  also ran the selected child checks.
- Native dogfood: an isolated Linux/Wayland package launched three independent
  Orbit Sessions. Physical Alt+M and Ctrl+T created and selected pane 2 and tab
  2; Alt+H/L traversed both tabs and Alt+K/J traversed both panes, with every
  transition observed through EONW JSON. The unchanged Venus window showed
  both pane headers around the selected body; screenshot SHA-256
  `981907b2e5015cf15106d52d403953dedfa03f1fd577f1c66287e8970dfcbf8a`.
  All isolated processes stopped without touching the user's live supervisor.
- Constraining negative: the first pre-build `nix flake check --no-build`
  lacked the new Venus derivation; its cached failure persisted after the
  build. Repeating with `--no-eval-cache` passed all package, app, and check
  schema evaluations.

### Refreshed workspace composition proof `2e71ef41fbbfa8352bd2a965c8facbcbf0d5a4c0`

- Contracts: EON-C1 through EON-C4 and EON-C8 are proved at this exact revision;
  EON-C9 and EON-C10 retain their existing owner proofs.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `a7e5b74beeb4b79732e886092f3a3bf47719a5185e166b4ebb47bf0bf136751f`,
  selects Venus and VEN-C8 proof `2dfe364fdc6a86ec183bf846d1b233e7a64de0dd`.
  Orbit remains `00b136318bea13e3f08d490468f069de6f6b9bd2` with ORBF v1 and ORBS
  v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`.
- Package: `/nix/store/s21rscz2qmzkis2q80a4fi2dqqjc3jxf-eon-0.1.0`, NAR hash
  `sha256-RAny9KZVjuXm03aMKUqkge6kqyxQkI6ENT6XsHj0u04=`, NAR size
  1,301,496 bytes, closure size 1,559,510,800 bytes. Its exact Venus artifact is
  `/nix/store/zinwbqj4yvxsljj12n0zncdcharrkl9l-yazelix-venus-0.1.0`.
- Checks: canonical manifest validation; locked Rust format, check, 17-test,
  clippy suites, and `git diff --check`; exact-revision Nix build and flake
  evaluation. The Nix build also ran 54 locked Venus tests.
- Native dogfood: an isolated Linux/Wayland Eon launched the exact package,
  Venus, and Orbit artifacts. A second CLI created a pane and tab, then focused
  pane 2; within one second the unchanged Venus window showed tab 2 and pane 2
  selected. The screenshot SHA-256 is
  `4b266931522620a76b7f356655d6736a7362a7ba5fda971663eb9d884f5c7fcc`.
  The isolated processes were stopped without touching the user's supervisor.
- Boundary: bounded 250 ms re-inspection is Venus-owned client policy. EONW v1
  still adds no event stream, subscription, or polling policy.

### Workspace-aware composition proof `89823e5eb308e2b27a03eafdee25ce55112038b4`

- Contracts: EON-C1 through EON-C4 remain proved. EON-C8 advances its exact
  composition slice while remaining partially proved. EON-C9 and EON-C10 keep
  their existing owner proofs.
- Component graph: `components/eon-alpha-v1.json`, SHA-256
  `aad379ed82b630be782f42b51f45c680fd37800c29b3ce7bea829df2be176214`,
  selects Venus `932be5ce68a42b72e8b0bf5953c74f7a8af51f1e` with VEN-C8 at that
  proof revision and records both the Orbit and Eon workspace socket inputs.
  Orbit remains `00b136318bea13e3f08d490468f069de6f6b9bd2` with ORBF v1 and ORBS
  v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`.
- Launch behavior: supervisor startup and `eon attach` pass the initial Orbit
  endpoint followed by `eon.sock`. Bare `eon` reconnects through the live Eon
  supervisor even if the initial Orbit Session has exited, allowing Venus to
  select another live pane through EONW v1.
- Package: `/nix/store/skbp88k9k016qxqhypa7q81iphmzx8x5-eon-0.1.0`, NAR
  hash `sha256-8j3D7TcfVxZD5rOGF8l/qZxQg8oFQrwTrgJ0RkMSHQ8=`, NAR size
  1,301,496 bytes, closure size 1,559,511,376 bytes. Its Venus artifact is
  `/nix/store/hkknqslwjg7fnzsw0wrqc209wmjpg3nw-yazelix-venus-0.1.0`.
- Checks: canonical locked Rust format, check, 17-test, and clippy suites;
  manifest validation; source-backed agent-policy validation; locked Nix build
  and flake evaluation; `git diff --check`. The Nix build also ran 53 locked
  Venus tests.
- Native dogfood: isolated Linux/Wayland composition launched two tabs, three
  live independent Orbit Sessions, and Venus with both exact socket inputs.
  Venus rendered the workspace chrome, switched to pane 1, and managed Nushell
  accepted `ls`. The user's existing runtime was not interrupted.
- Limits: EONW v1 intentionally has no event stream, subscription, or polling
  policy. A tab or pane created by another CLI after Venus reads its snapshot
  requires reopening Eon Desktop; a separately approved refresh mechanism and
  its composed proof are required before EON-C8 becomes fully proved. Public
  Venus distribution also waits for Eon Desktop to record the user-approved
  Apache-2.0 license in that source repository.

### EONW v1 producer proof `4af395aea06c230ee6b18cf0755ae25915c0b88d`

- Contract: EON-C10 is proved at this exact revision on x86_64 Linux. It
  preserves EON-C1 through EON-C4, EON-C9, and the partial EON-C8 owner slice.
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
  path:.`. The exact package output is
  `/nix/store/905hr7d4912i75rzhcrzv3hhqciph04h-eon-0.1.0`. The real
  three-Session control test sends complete requests without half-closing the
  socket, decodes `unsupported-version`, then proves the accepted workspace is
  still available. A focused process test proves missing supervisors retain the
  structured JSON failure and exit-2 contract.
- Limits: this revision adds no event stream, subscription, polling policy,
  remote transport, authorization layer, plugin surface, AgentRun state,
  persistence, terminal content, or Venus rendering. The canonical manifest
  still truthfully selects Venus `2d36c72dc87ca5416e22d6afdc35c6ab4e2fb832`
  with one Orbit socket. EONW requirement and launch wiring wait for an exact
  Venus consumer proof.

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

### Superseded desktop icon proof `712c0834691c58b07e56494de25f98670b6a882e`

- Contract: EON-C1 advances only the packaged desktop-icon identity. EON-C2,
  EON-C3, EON-C4, runtime behavior, component revisions, and compatibility remain
  proved by their earlier revisions above.
- Artifact: `/nix/store/5v2i3wp7q52zmnl5p4y9m68wwrfmclsq-eon-0.1.0`,
  NAR hash `sha256-otl7TOldLCDVHbiNfYjNHJoPx0P6JBCfyJJzcZz6p98=`, NAR size
  2,973,512 bytes, closure size 1,457,883,088 bytes.
- Desktop icon at that revision: `assets/eon.png`, a 1254 by 1254 RGBA PNG with SHA-256
  `540d33604b703b69e80531d228d2bef9887ec561954c0ceecd59c2b9b511a9b7`.
  The installed icon is byte-identical, and the desktop entry retains `Name=Eon`,
  `Icon=eon`, and `StartupWMClass=yazelix-venus`.
- Checks: canonical and installed icon byte/hash equality; image identity;
  README target; locked Cargo format, check, test, manifest, and clippy commands;
  `nix flake check --no-build`; exact package build; desktop-file validation;
  live COSMIC launcher inspection; and `git diff --check`.

### Superseded V32 desktop icon proof

- Contract: the user selected V32 as Eon's canonical desktop identity and
  closed the icon comparison on 2026-08-09. This record proves icon identity
  only; the separate managed-environment candidate records its runtime changes.
- Artifact: `/nix/store/cj0pbmk9607893khiwl8wmy031x88a3x-eon-0.1.0`,
  NAR hash `sha256-tn0Wrxr6rku4puzvvV+WPqTb5MMjo1EjwqWG06vgjCA=`, NAR size
  2,343,344 bytes.
- Desktop icon: the retained V32 source, `assets/eon.png`, and the installed
  hicolor icon are byte-identical 1254 by 1254 sRGB RGB PNGs with SHA-256
  `0972bed46f27b3123e17d8676fbde9da0d0b94877f3d2ffd131aa33f31ca6476`.
  Retained concepts remain design options and are not runtime identities.
- Checks: source/canonical/installed `cmp` and SHA-256 equality; image identity;
  README target; `nix flake check --no-build`; exact package build;
  `desktop-file-validate`; and exact `Name=Eon`, `Icon=eon`, and
  `StartupWMClass=yazelix-venus` inspection.

### Superseded icon 3 replacement candidate

- Contract: the user replaced V32 after live COSMIC inspection exposed
  small-size edge artifacts. This candidate changes only the EON-C1 canonical
  desktop-icon identity and packaging; runtime behavior, component revisions,
  commands, configuration, and compatibility remain unchanged.
- Source and master: `assets/icon-concepts/icon_eon_3.png`, SHA-256
  `1bc63f18c821520e9a3b5e09f1c8bd5e33b5a87da85179523cb22bc1fb0d9786`,
  informed the rebuilt `assets/eon.png`, a 1254 by 1254 sRGB RGBA PNG with
  SHA-256 `5c64ab628a60ba2108f23100abe5a61c4c1fe03822416b81392da4de6d3a2bf5`.
  Its canvas corners and center aperture are transparent.
- Package: `/nix/store/rl8l700ikh02d75dwbw6wfsbs9nwlfdx-eon-0.1.0`,
  NAR hash `sha256-jmILqhnrrRdNFm5g0Mb5dkWR3vdoOPXXdfbN+x4pBdA=`, NAR size
  1,316,912 bytes. Pinned ImageMagick derives 16, 24, 32, 48, 64, 128, 256,
  and 512 pixel PNGs with Box downsampling under matching hicolor directories;
  no raster is installed under `scalable`, and no alternate desktop entry remains.
- Checks: all eight packaged sizes have alpha and contain zero visible white,
  yellow, cyan, or green pixels. Integer-zoom inspection on light and dark
  backgrounds found no isolated bright edge pixels at 16, 24, 32, or 48 pixels.
  `nix flake check --no-build --no-update-lock-file path:.`, exact package build,
  `desktop-file-validate`, installed desktop metadata inspection, user-local icon
  cache refresh, and `git diff --check` passed.
- Limit: no vector master exists, so Nix retains one build-only raster resizer.
  The 16-pixel rendering is necessarily simpler than the larger source.

### Canonical icon 4 replacement candidate

- Contract: the user selected icon 4 after native-size inspection. This
  candidate changes only the EON-C1 canonical desktop-icon identity; runtime
  behavior, component revisions, commands, configuration, and compatibility
  remain unchanged.
- Source and master: `assets/icon-concepts/icon_eon_4.png`, SHA-256
  `9a4149f59f91514562f44fec4718b3c0ef90f5d61d54237a42e53aa831b3250b`,
  informed `assets/eon.png`, a 1254 by 1254 sRGB RGBA PNG with SHA-256
  `33ec3062f72a732290dcd6c6f40a2d5f535d6a7cbfcaf7455a0a5c4f03a52e0d`.
  Imagegen preserved the selected geometry and violet gradient while replacing
  only the opaque background with a chroma matte; the imagegen skill helper
  removed that matte with soft edges, despill, and one-pixel edge contraction.
- Package: `/nix/store/najyqyv74fwmx2fz3rrzi552sq7jiicw-eon-0.1.0`,
  NAR hash `sha256-uyPRRcPY7wpuxNNCW8huFRkLWnl6IcSJCz/PuOgoRd0=`, NAR size
  1,257,936 bytes. The existing pinned ImageMagick path derives 16, 24, 32, 48,
  64, 128, 256, and 512 pixel PNGs under matching hicolor directories; no
  runtime resizer or alternate desktop entry is added.
- Checks: all eight packaged sizes have alpha and contain zero visible white,
  yellow, cyan, or green pixels. Inspection on light and dark backgrounds found
  no isolated bright edge pixels at 16, 24, 32, or 48 pixels. `nix flake check
  --no-build --no-update-lock-file .`, exact package build,
  `desktop-file-validate`, installed desktop metadata inspection, byte equality
  for all installed package rasters, user-local cache refresh, and `git diff
  --check` passed.
- Limit: no vector master exists, so Nix retains one build-only raster resizer.
  The 16-pixel rendering is necessarily simpler than the larger source.

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
