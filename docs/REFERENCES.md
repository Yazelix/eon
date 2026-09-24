# Reference Routing

Use references to answer a named design question. Read only the matching route,
start with `Read first`, and inspect an additional source only when its condition
applies. Adjacent routes are not required reading. `Owner` identifies the
subsystem that may adopt the evidence; a reference does not transfer ownership.

Eon keeps cross-product comparisons and cross-repository routes here. Child
repositories keep evidence for their own contracts and link here instead of
duplicating product pins. Implementing Beads record exact inspected revisions.

## Orbit and terminal architecture

### Does an Eon design question depend on terminal service, engine, attachment, or diagnostic-client internals?

- **Owner:** Orbit through Eon Sessions
- **Read first:** [Eon Sessions
  contracts](https://github.com/Yazelix/eon-sessions/blob/edge/docs/CONTRACTS.md) and
  [reference routing](https://github.com/Yazelix/eon-sessions/blob/edge/docs/REFERENCES.md)
- **Read additionally only if:** Follow only the child route matching a demonstrated Orbit
  contract gap.
- **Preserve / reject:** Consume accepted Orbit contracts. Do not repeat or reopen engine
  and diagnostic-client decisions in Eon.

### If Eon for the web is activated, should its browser client consume canonical Orbit frames or restore a libghostty-vt WebAssembly snapshot plus ordered tail?

- **Owner:** Eon client policy and Orbit protocol ownership
- **Read first:** [Ghostty at
  `d760ee96e546`](https://github.com/ghostty-org/ghostty/tree/d760ee96e54657416eb427b793c7e839f003df7d)
  and its [WebAssembly
  example](https://github.com/ghostty-org/ghostty/tree/d760ee96e54657416eb427b793c7e839f003df7d/example/wasm-vt)
- **Read additionally only if:** Read the [snapshot
  API](https://github.com/ghostty-org/ghostty/blob/d760ee96e54657416eb427b793c7e839f003df7d/include/ghostty/vt/snapshot.h)
  only when checkpoint-plus-tail replication is a measured candidate; inspect the [signed
  nightly artifact
  workflow](https://github.com/ghostty-org/ghostty/blob/d760ee96e54657416eb427b793c7e839f003df7d/.github/workflows/release-tip.yml)
  only when packaging a prebuilt artifact is in scope.
- **Preserve / reject:** Keep canonical Orbit frames and one terminal authority as the
  default. Compare a non-authoritative snapshot replica only after measured transport or
  rendering pressure. Reject benchmark extrapolation to complete browser UX, moving nightly
  artifacts as stable dependencies, snapshot v1 as a durable compatibility promise, a second
  terminal-query owner, and implied Eon-web scope.

### If a web experiment is activated, how should an HTTP adapter stream authoritative state to browser clients?

- **Owner:** Eon web composition; Orbit retains terminal authority
- **Read first:** [http-nu at
  `a8dab1b85bd1`](https://github.com/cablehead/http-nu/blob/a8dab1b85bd1c65a31ade1981389296c3ef2af2b/README.md)
- **Read additionally only if:** Inspect its [local event
  bus](https://github.com/cablehead/http-nu/blob/a8dab1b85bd1c65a31ade1981389296c3ef2af2b/src/bus.rs)
  for slow-subscriber handling and [server
  lifecycle](https://github.com/cablehead/http-nu/blob/a8dab1b85bd1c65a31ade1981389296c3ef2af2b/src/main.rs)
  for reload or shutdown behavior.
- **Preserve / reject:** Study Nushell-scripted HTTP/SSE composition, terminating lagged
  subscriptions, and cancelling old streams on handler reload. Reconnection still needs
  an authoritative fresh-state contract. Keep this a conditional prototype reference;
  it selects no dependency, embedded Nushell runtime, Datastar renderer, cross.stream
  store or service owner, and activates no web, remote-control, or native-UI scope.

## Terminal product comparisons

### How should Eon compare terminal completeness, native Linux integration, performance evidence, and direct distribution without importing another terminal's ownership?

- **Owner:** Eon product policy; Orbit and Venus retain their accepted mechanisms
- **Read first:** [Monstar 1.0.1 at
  `b801befec37b`](https://github.com/rockorager/monstar/tree/b801befec37b1ef82302caf6542bb8bb133c0ff1),
  its [documented
  surface](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/README.md),
  and [MIT
  license](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/LICENSE)
- **Read additionally only if:** Read its exact
  [`Window.zig`](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/src/Window.zig),
  [`App.zig`](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/src/App.zig),
  or
  [`Pty.zig`](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/src/Pty.zig)
  only for a named host, interaction, or performance question. Read its [release
  workflow](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/.github/workflows/release.yml),
  [binary
  assembler](https://github.com/rockorager/monstar/blob/b801befec37b1ef82302caf6542bb8bb133c0ff1/packaging/build-binary-dist.sh),
  and [AUR
  templates](https://github.com/rockorager/monstar/tree/b801befec37b1ef82302caf6542bb8bb133c0ff1/packaging/arch)
  only after Eon activates distribution graduation.
- **Preserve / reject:** Study terminal search, graphics, URI handling, native Wayland
  integration, damage, benchmark methods, stripped archives, and package publication.
  Preserve durable Orbit Sessions, Venus presentation, Eon workspace authority, and the
  canonical component graph. Reject Monstar's PTY and terminal ownership, single-process
  architecture, renderer, Zig toolchain, direct dependency selection, feature-parity
  roadmap, and size or performance extrapolation across Eon's process boundary.

## Session workspace experience

### How should Eon organize and switch among durable terminal sessions without absorbing terminal or rendering state?

- **Owner:** Eon workspace policy and Venus presentation
- **Read first:** [Canario](https://rapha.land/canario/) and its [frontend at Rio
  `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario)
- **Read additionally only if:** Read its [session
  store](https://github.com/raphamorim/rio/blob/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario/Sources/SessionStore.swift)
  only when comparing restart restoration with process survival.
- **Preserve / reject:** Study spaces, navigation, filing, quick terminals, and previews.
  Consume exact Orbit identities; do not simulate survival, copy source, or move PTY
  lifetime into Venus.

### How should a native client borrow durable sessions across local and SSH hosts without taking ownership of their state?

- **Owner:** Eon lifecycle policy, Orbit sessions, and Venus presentation
- **Read first:** [Ghosthub `0.7.0` at
  `072cc83559bd`](https://github.com/kenn-io/ghosthub/tree/072cc83559bd590ed5d759af05ec0a1868dbf359)
- **Read additionally only if:** Read its
  [architecture](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/architecture.md),
  [terminal-session
  contract](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/terminal-sessions.md),
  or [threat
  model](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/threat-model.md)
  only for the corresponding ownership, reconnect, remote-helper, or security question.
- **Preserve / reject:** Preserve backend authority, detach-only close, fresh attach,
  generation checks, per-host degradation, and supervised reconnect. Reject tmux, Herdr, or
  kwt adoption; merged inventories; activity heuristics as truth; macOS-only shape; and
  copied AGPL source.

### How should Eon distinguish controller loss, host loss, presentation replay, provider resume, and remote transport without duplicating terminal authority?

- **Owner:** Eon lifecycle policy, Orbit sessions, and Venus presentation
- **Read first:** [Unpeel `0.2.0` at
  `9580a83f009f`](https://github.com/unpeel-com/unpeel/tree/9580a83f009fe19775116cacb46e74570958ca56)
- **Read additionally only if:** Read its [Host/Controller transport
  plan](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/docs/plans/host-controller-transports.md)
  when controller or transport semantics are in scope; inspect its [session
  host](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/crates/unpeel-core/src/session_host.rs)
  and [terminal
  viewport](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/crates/unpeel-core/src/terminal_viewport.rs)
  only when comparing recovery or terminal authority.
- **Preserve / reject:** Study one capability-advertising Host contract across direct, SSH,
  and relay transports; committed output cursors; connection generations; and explicit
  restart-with-resume. Preserve Orbit as sole terminal-state authority. Reject raw-byte
  journals as Eon terminal truth, duplicate emulators, terminal-query filtering, implied
  process resurrection, remote-control-as-remote-compute, and inherited relay or mobile
  scope.

### How should Eon separate live Session continuity from workspace reconstruction after the Session owner exits?

- **Owner:** Eon durability policy over Orbit lifecycle
- **Read first:** [TUIOS at
  `0047c409185f`](https://github.com/Gaurav-Gosain/tuios/tree/0047c409185f37e568933a566e0253b2a6e96a96)
  and its [Session
  semantics](https://github.com/Gaurav-Gosain/tuios/blob/0047c409185f37e568933a566e0253b2a6e96a96/docs/SESSIONS.md)
- **Read additionally only if:** Read its
  [architecture](https://github.com/Gaurav-Gosain/tuios/blob/0047c409185f37e568933a566e0253b2a6e96a96/docs/ARCHITECTURE.md)
  only when comparing daemon ownership with Orbit.
- **Preserve / reject:** Preserve the distinction between live detach, clean owner shutdown,
  crash, reboot, and metadata reconstruction with fresh processes. Reject restored layout or
  working directory as process, scrollback, or terminal-state survival; reject TUIOS daemon,
  emulator, multi-client, and terminal-in-terminal ownership.

### How should one tab choose the launch directory for future Sessions without treating shell navigation as product policy?

- **Owner:** Eon workspace policy
- **Read first:** Rust
  [`Command::current_dir`](https://doc.rust-lang.org/std/process/struct.Command.html#method.current_dir)
  and Nova's [`zellij-pane-orchestrator` at
  `e76ae565f0dc`](https://github.com/Yazelix/zellij-pane-orchestrator/tree/e76ae565f0dc25844293544bf9b58a6dcf127947)
- **Read additionally only if:** Read Nova's exact Zellij source only when inaccessible-path
  behavior or native new-tab CWD semantics are in question.
- **Preserve / reject:** Preserve one explicit per-tab root, spawn-boundary enforcement, and
  derived presentation. Reject shell injection, pane-CWD inference, canonicalization,
  persistence, and copied Zellij ownership.

### How should full Eon choose every new tab directory before starting its first pane without creating a general popup system?

- **Owner:** Eon pending-tab and picker lifecycle; Venus modal presentation
- **Read first:** [Zoxide 0.9.9 at
  `9cdc6aa`](https://github.com/ajeetdsouza/zoxide/tree/9cdc6aa3740b4d8a9d62406c99e84c5de49645e9),
  [Zellij in-place commands at
  `910339e`](https://github.com/zellij-org/zellij/tree/910339e219b17f4e2ffd4a2cb6e35cccc7246493),
  and [Kitty overlays at
  `5441649`](https://github.com/kovidgoyal/kitty/tree/54416498c89e1d07e5079c49d15470dd0d947ce7)
- **Read additionally only if:** Read Nova at `f1beb34fe606` only for the accepted Alt+Z
  outcome and margin comparison; read [WezTerm InputSelector at
  `f8d241c`](https://github.com/wez/wezterm/tree/f8d241c0eee5786ba286316ba07061e910e6a719)
  only if a native Venus list becomes a measured candidate; inspect exact fzf only at its
  dependency gate.
- **Preserve / reject:** Preserve one Eon-owned transient picker per pane-free pending tab,
  exact tab binding, and one Venus-rendered active modal endpoint. Reject
  a placeholder shell, pane restart, `cd` injection, empty-string sentinel, copied Lua,
  client-supplied commands, native directory ranking, simultaneous terminal composition, and
  a generic popup protocol.

### How should shared popups extend the workspace wire contract without breaking the installed client?

- **Owner:** Eon's shared workspace protocol; Venus consumes its exact crate.
- **Read first:** `eon-tool-popups-e13.1`, EON-C10's v5 seed contract and
  `crates/eon-workspace-protocol/src/{v2,v3,v4,v5}.rs`. Inspect existing Eon
  control/workspace consumers and Venus model/transport/scene/input consumers.
- **Comparable implementations:** Eon's additive v3 seed
  `96119f29ca2e3ec4ad19bbe272708b07d588429a` and v4 seed
  `aaafc9127c054e683abfceb3c8fcaae201a7a763`. The decision records exact
  Nova, Zellij Popup and Kitty evidence for the unchanged interaction choices.
- **Preserve / reject:** Reuse bounded framing, common actions and lifecycle
  codecs; represent exact popup identities and per-tab selection. Seed the new
  version before Venus consumption and Eon activation. Reject duplicated
  commands/lifetime policy in Venus, runtime negotiation, a second schema and
  claims that codec tests prove installed popup behavior. The earlier
  picker-only route describes the still-active v4 contract.

### How should a tab picker reach folders absent from Zoxide history?

- **Owner:** Eon selection policy; Yazi filesystem navigation
- **Read first:** [Yazi 26.5.6](https://github.com/sxyazi/yazi/tree/aa526434f00bb44e2e902d9a4ac5f810da1018b9),
  especially its default keymap, built-in Zoxide plugin, `--cwd-file`, quit action,
  and terminal writer; [fzf 0.73.1](https://github.com/junegunn/fzf/tree/ce4bef75954bebd87e0886435bcf8c6904328ab0)
  expected-key and NUL-output behavior; Zoxide's pinned query/list behavior above.
- **Comparable implementation:** Nova `29514faf4a25` Yazi keymap and
  `zoxide-editor` plugin distinguish Alt+Z workspace commitment from Shift+Z
  browser navigation. Inspect the distinction; do not copy Nova coordination.
- **Preserve / reject:** Preserve default quick search, explicit Browse versus
  Cancel, navigation before commitment, one validated directory, and EON-C18
  lifecycle. Prefer packaged Yazi over an Eon walker; reject typed-path-only
  discovery, recursive root scans, ambient picker keymaps, shell-evaluated paths,
  and file-opening scope. Prove actual packaged terminal and directory-output
  behavior before claiming installed integration.

### How should Eon show Anima before a fresh picker and in an on-demand popup?

- **Owner:** Eon launch and popup policy; Anima playback; Orbit terminal Session;
  Venus presentation.
- **Read first:** [Anima 0.2.0 at `b3133f0`](https://github.com/Yazelix/anima/tree/b3133f057fa0029b3e06c85301161168c48bb799),
  especially its README, CLI entrypoint and screen runner; EON-C18 and Eon's
  current readiness, picker and popup owners.
- **Comparable implementation:** [Nova at `e476ca8`](https://github.com/Yazelix/nova/tree/e476ca8065cc8b2c02ea611ae7edeeb0989c0c23)
  delegates `yzx anima` and presents a transient random animation on Alt+Shift+A.
- **Preserve / reject:** Use one exact Anima executable through the component
  graph. Wait for Venus readiness before startup playback and reuse the
  transient popup Session for on-demand playback. Do not copy style lists,
  rendering, terminal-mode handling, or Nova's Zellij mechanism.


## Managed shell environment

### How should Eon activate a consistent optional tool set after native Nushell, Bash, Zsh, and Fish configuration?

- **Owner:** Eon managed-environment adapter; each child owns its native semantics
- **Read first:** The affected exact upstream: [Nushell
  0.113.1](https://github.com/nushell/nushell/tree/7b7df4aa68e957cf38b9d8157c35fa7523f44a6d),
  [Bash 5.3](https://git.savannah.gnu.org/cgit/bash.git/tag/?h=bash-5.3), [Zsh
  5.9.1](https://github.com/zsh-users/zsh/tree/0e0d4ea11731c47f57bad042fbe75e3979d8a1d2),
  [Fish
  4.7.1](https://github.com/fish-shell/fish-shell/tree/efb0223da10367031b7c887a3e40eccdf9bf7b06),
  [Starship
  1.25.1](https://github.com/starship/starship/tree/8758daa7767d4e73874330b1e262fca66a7ffd30),
  [Zoxide
  0.9.9](https://github.com/ajeetdsouza/zoxide/tree/9cdc6aa3740b4d8a9d62406c99e84c5de49645e9),
  [Atuin
  18.16.1](https://github.com/atuinsh/atuin/tree/671f96b60dac49d1d2de73cc0812986a5e22ce7b),
  or [Carapace
  1.6.3](https://github.com/carapace-sh/carapace-bin/tree/e4ed2a5ae661848b228224ad7edb20ea678d33d4).
- **Read additionally only if:** Read [Nova startup at
  `f1beb34f`](https://github.com/Yazelix/nova/tree/f1beb34fe6060cfa2c0201d7f8095f6ef707f467)
  only when comparing cross-shell activation shape.
- **Preserve / reject:** Reuse native startup and generated init; preserve native config and
  state; guard user hooks. Do not copy Nova or inherit its Mise, environment, settings, or
  compatibility scope.

## Agent work orchestration

### How should Eon integrate existing coding agents without parsing terminal output or treating a provider conversation as Eon work?

- **Owner:** Eon provider adapter
- **Read first:** [Agent Client Protocol at
  `9ef3e3800b40`](https://github.com/agentclientprotocol/agent-client-protocol/tree/9ef3e3800b4070632b54846b5ddf310fc4b35b03)
- **Read additionally only if:** Read
  [initialization](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/initialization.mdx),
  [session
  setup](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/session-setup.mdx),
  or [prompt-turn
  lifecycle](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/prompt-turn.mdx)
  only when that protocol phase is in scope.
- **Preserve / reject:** Keep provider sessions adapter-owned and distinct from Eon work and
  Orbit Sessions. Reject universal-support assumptions, invented semantics, and ACP
  conversation state as Eon work.

### How should a headless agent expose live activity separately from conversation context?

- **Owner:** Eon provider adapters and attention semantics; Venus presents their state
- **Read first:** [yoke at
  `90b5fb91b4c5`](https://github.com/cablehead/yoke/blob/90b5fb91b4c513fb0851fc808e61c5c94ba940bf/README.md)
- **Read additionally only if:** Inspect its [event projection and input
  parser](https://github.com/cablehead/yoke/blob/90b5fb91b4c513fb0851fc808e61c5c94ba940bf/src/main.rs)
  when defining an agent-event adapter; inspect its [render
  consumer](https://github.com/cablehead/yoke/blob/90b5fb91b4c513fb0851fc808e61c5c94ba940bf/ux/render.nu)
  when comparing progress, tool-result and usage presentation.
- **Preserve / reject:** Study one agent turn as a process, JSONL context messages
  distinct from live observations, and explicit turn/tool start, end, progress and error
  events. These are useful sidebar inputs without parsing terminal output. Preserve
  provider evidence and keep conversation context distinct from Beads work and Orbit
  Sessions. A process or turn ending is not proof that the work succeeded. This reference
  selects no yoke/yoagent harness, transcript schema, provider support, embedded Nushell
  engine, web UI, or persistence mechanism.

### How should Eon retain source-aware provider activity and expose compact agent actions without creating another action owner?

- **Owner:** Eon provider adapter and semantic action registry
- **Read first:** Unpeel `0.2.0` [session
  activity](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/docs/agents/session-activity.md)
  and [Sessions
  MCP](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/docs/agents/sessions-mcp.md)
  at `9580a83f009f`
- **Read additionally only if:** Inspect its [MCP
  host](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/crates/unpeel-core/src/mcp_host.rs)
  only when schema size, authorization, or reconnect behavior is in scope; read its
  [shared-core
  plan](https://github.com/unpeel-com/unpeel/blob/9580a83f009fe19775116cacb46e74570958ca56/docs/plans/shared-core.md)
  only when multiple Eon clients risk duplicating policy.
- **Preserve / reject:** Study persisted evidence source and freshness, provider-specific
  hook semantics, compact domain actions, and lazy help. Keep the Eon registry
  authoritative. Reject output-growth or rendered-menu heuristics as semantic truth,
  cooperative same-user checks as security isolation, duplicate client policy, broad
  provider compatibility, and inherited browser, relay, or MCP product scope.

### How should Eon expose agent-readable actions and source-labeled attention without creating another terminal or action owner?

- **Owner:** Eon semantic action registry and provider adapters
- **Read first:** TUIOS [control
  protocol](https://github.com/Gaurav-Gosain/tuios/blob/0047c409185f37e568933a566e0253b2a6e96a96/docs/protocol.md)
  and [agent-state
  model](https://github.com/Gaurav-Gosain/tuios/blob/0047c409185f37e568933a566e0253b2a6e96a96/docs/AGENT_STATE.md)
  at `0047c409185f`
- **Read additionally only if:** Read its [tape
  language](https://github.com/Gaurav-Gosain/tuios/blob/0047c409185f37e568933a566e0253b2a6e96a96/docs/TAPE_SCRIPTING.md)
  only when terminal testing is active; inspect the [Claude Code
  hook](https://github.com/Gaurav-Gosain/tuios/tree/0047c409185f37e568933a566e0253b2a6e96a96/integrations/claude-code)
  only for provider-reported state.
- **Preserve / reject:** Preserve one registry for dispatch, typed discovery, examples,
  stable errors, and bounded waits; retain explicit evidence source and freshness. Keep
  unsupported provider state unknown. Reject screen and stall classification as semantic
  truth, capture-pane or send-keys as a general agent API, and inherited PTY, emulator,
  tape, SSH, web, or multi-client scope.

### If cross-device agent control is activated, how should Eon separate host execution, durable intent, and device-local view state without mistaking persistence for eventual completion?

- **Owner:** Eon agent-action and durability policy over accepted Orbit and Venus contracts
- **Read first:** [Zeron `v0.2.3` at
  `9ab250ceb631`](https://github.com/zeronsh/comet/tree/9ab250ceb6317d080a8429435cb15a9eaef5663e)
  and its
  [architecture](https://github.com/zeronsh/comet/blob/9ab250ceb6317d080a8429435cb15a9eaef5663e/ARCHITECTURE.md)
- **Read additionally only if:** Read its [processed-command crash
  regression](https://github.com/zeronsh/comet/blob/9ab250ceb6317d080a8429435cb15a9eaef5663e/crates/engine/tests/e2e.rs#L529-L568)
  when command liveness is in scope; inspect its [iOS
  viewport](https://github.com/zeronsh/comet/blob/9ab250ceb6317d080a8429435cb15a9eaef5663e/apps/ios/README.md)
  and [parity
  gaps](https://github.com/zeronsh/comet/blob/9ab250ceb6317d080a8429435cb15a9eaef5663e/docs/PARITY.md)
  only after remote or mobile work is activated.
- **Preserve / reject:** Study one engine authority behind equivalent in-process and remote
  RPC, immutable local-versus-synced startup scope, device-local tabs distinct from durable
  archive, and host-only outcome writes. A processed-but-pending command is evidence of an
  unresolved effect, not eventual delivery; require bounded reconciliation or an explicit
  unknown or aborted outcome. Reject Zeron's PTY, emulator, provider harness, repository,
  diff, CRDT, cloud relay, auth, E2EE, mobile, and compatibility scope.

### How should Eon coordinate durable agent work without equating work, attempts, provider sessions, terminals, or processes?

- **Owner:** Eon work and orchestration policy
- **Read first:** [Orca orchestration guide at
  `8859e73980d2`](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skill-guides/orchestration.md)
- **Read additionally only if:** Read its [state
  types](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/src/main/runtime/orchestration/types.ts)
  or [skill
  stub](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skills/orchestration/SKILL.md)
  only when identity shape or release-matched guidance is in scope.
- **Preserve / reject:** Study separate Run, Task, Dispatch, and worker identities, durable
  delivery, decisions, and handoff. Preserve Eon, Orbit, and Venus ownership; reject Orca
  product, provider, autonomy, and CLI scope.

### How should Eon package provider-neutral operating guidance and liaison policy around existing coding agents?

- **Owner:** Eon agent guidance and execution policy
- **Read first:** [firstmate at
  `85e750ab9b76`](https://github.com/kunchenguid/firstmate/blob/85e750ab9b76df275c1f6b9e2bc95b671955bae9/README.md)
- **Read additionally only if:** None by default.
- **Preserve / reject:** Treat firstmate as agent-distribution evidence, not a skill or
  harness template. Study one liaison, visible backends, isolated attempts, and escalation;
  reject copied mechanisms, hidden crews, role hierarchy, and compatibility scope.

### How should Beads-authorized work route to visible agent sessions while coordination remains distinct from planning truth?

- **Owner:** Eon authorization and coordination policy
- **Read first:** [NTM at
  `1110bce247d4`](https://github.com/Dicklesworthstone/ntm/blob/1110bce247d4bd90247301bbea8a01915159c86b/README.md)
- **Read additionally only if:** Read [Agent Mail at
  `ec835d2dace0`](https://github.com/dicklesworthstone/mcp_agent_mail/blob/ec835d2dace0d89549d51bc273cb68939e2cda63/README.md)
  only if inboxes, acknowledgement, threads, or advisory leases are in scope.
- **Preserve / reject:** Preserve Beads as sole work truth and Orbit as terminal owner.
  Reject copied tmux control, a second issue graph, automatic queue consumption, owned mail
  archives, and mandatory messaging infrastructure.

### How should Venus direct human attention across many agent-bearing workspaces without becoming their source of truth?

- **Owner:** Eon attention semantics and Venus presentation
- **Read first:** [cmux at
  `a35880b1031e`](https://github.com/manaflow-ai/cmux/blob/a35880b1031e673e5e006f0ccb78c3ffa12c4bcb/README.md)
- **Read additionally only if:** None by default.
- **Preserve / reject:** Study compact context, unread attention, waiting reasons, feeds,
  notifications, and jump-to-work. Prefer structured events and exact identities; reject
  GUI-owned PTYs, screen or process inference, browser automation, remote control, and cmux
  compatibility.

### If provider-native handoff is unavailable and a terminal fallback is explicitly approved, what safety rules should constrain it?

- **Owner:** Eon provider-adapter policy over the Orbit input boundary
- **Read first:** [ccmux handoff guide at
  `950f7a152947`](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/docs/handoff.md)
- **Read additionally only if:** Read the [ccmux
  README](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/README.md)
  only when its command integration is being compared.
- **Preserve / reject:** Preserve idle-only delivery, refusal of waiting agents, bounded
  queues, explicit targets, and visible failure. Reject screen scraping as prompt input and
  generic keystroke injection as an Eon agent contract.

### How should Eon present an isolated agent attempt from launch through review without equating it with the durable work item?

- **Owner:** Eon attempt lifecycle and Venus presentation
- **Read first:** [Superset at
  `bf2e078632e2`](https://github.com/superset-sh/superset/blob/bf2e078632e260ce9e534832a03efe40960c034b/README.md)
- **Read additionally only if:** None by default.
- **Preserve / reject:** Study work-to-worktree, terminal, environment, diff, feedback, and
  explicit handoff. Keep the Bead independent; reject an Eon-owned editor, Git, dev server,
  browser, chat, scheduler, cloud relay, PR service, or merge policy.

### How should Eon support cross-agent fresh-eyes review and handoff without manufacturing consensus or normalizing provider transcripts?

- **Owner:** Eon review policy and provider adapters
- **Read first:** [MCO at
  `9eff964825e4`](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/README.md)
  and its [invocation
  contract](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/docs/contracts/invocation-runtime-v1.md)
- **Read additionally only if:** Read [Codex for Claude Code at
  `db52e28f4d9d`](https://github.com/openai/codex-plugin-cc/blob/db52e28f4d9ded852ab3942cea316258ae4ef346/README.md)
  only when provider-native persistent handoff is in scope.
- **Preserve / reject:** Preserve raw answers, disagreement, explicit selection, and bounded
  execution. Reject inferred teams, consensus synthesis, vote-driven action, unbounded
  loops, canonicalized transcripts, and provider rescue commands as product contracts.

## Recovery experience

### How should Eon help a user recover an interrupted product session without guessing or silently reopening the wrong work?

- **Owner:** Eon recovery policy over accepted Orbit and Venus contracts
- **Read first:** [Power Failure Resumer at
  `318d24fbab1b`](https://github.com/Dicklesworthstone/power_failure_resumer/tree/318d24fbab1b5c0ab1242ee19892419b034faa0e)
- **Read additionally only if:** None by default.
- **Preserve / reject:** Study discovery, ambiguity, previewable plans, idempotence,
  verified reopen, and partial failure. Reject product-specific mechanics, automatic
  unknown-process resume, and claims beyond exact child contracts.

Composition and release references are routed by [Distribution and
Composition](DISTRIBUTION.md#reference-routing), their owning policy document.

## Review record template

Add this evidence to the implementing bead:

```text
Question:
Contract:
Sources and revisions:
Constraints preserved:
Options compared:
Decision:
Rejected alternatives and failure conditions:
Cheapest proof:
Exit condition:
```
