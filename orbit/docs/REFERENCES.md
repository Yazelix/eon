# Orbit reference map

Orbit studies existing work at the implementation boundary where it can answer
a concrete question. A reference is evidence, not an architectural foundation,
dependency decision, or authorization to inherit its feature set.

When claiming an implementation bead, follow the evidence-first gate in
`AGENTS.md`. The bead classifies each source as required before code,
conditional on a named failure, comparison only, or rejected scope. Planning
routing does not count as implementation evidence.

1. Inspect the current source and documentation for every required reference,
   recording a release or commit when behavior matters.
2. Extract the smallest relevant contract, check, failure mode, or ownership
   lesson.
3. Record what Orbit uses, rejects, or still cannot prove in an append-only
   pre-implementation `Reference gate` comment.
4. Name the affected `ORB-C*` contracts and state how the evidence constrains
   ownership, dependencies, code shape, and the first contract check before
   editing code, tests, or manifests.
5. Ask before adding an unplanned dependency, compatibility surface, fork, or
   feature.

When a slice may change Cargo dependencies, follow the separate crate gate in
`AGENTS.md` and record the durable outcome in [the crate decision
index](CRATES.md). Compare credible implementation shapes rather than choosing
the first crate encountered in reference research. Normally include the
no-new-crate or existing-dependency path and at least two viable crate-backed
alternatives, with a smaller set only when the recorded search finds fewer
credible choices. Architecture fit, ownership, owned LOC, transitive cost,
maintenance, platform and build consequences, and focused measurements decide
the recommendation.

The implementation remains clean-room. Orbit may reproduce user-visible
behavior and independently derive checks, but it does not copy source from
Mars, Mars Next, or another project by default.

## Platform portability

- Inspect the exact `libc` release's Linux and Apple target definitions plus
  recorded official Darwin or macOS documentation for `openpty`, controlling
  terminals, process groups, signals, polling, nonblocking I/O, Unix sockets,
  permissions, and EOF or error behavior before defining the platform seam.
- Use `portable-pty`, `rustix`, and `rustix-openpty` as crate and API-shape
  comparisons, not automatic portability choices. The crate gate decides
  whether their abstraction and dependency cost improve Orbit.
- Keep terminal authority, semantic input, presentation framing, ordering, and
  client compatibility independent of the host OS. Do not treat building a
  macOS backend or CI as part of the initial portability audit.

## Attachment and terminal state

- Scroll-position extraction uses `libghostty-vt` 0.2.1 `Terminal::scrollbar`
  and Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083`, public
  `terminal.h` and `PageList.zig::scrollbar`. The engine owns cached viewport
  offsets and invalidates them after pruning and reflow; its no-history screen
  reports total equal to viewport length even when page capacity has spare rows.
  Orbit reads these values with each complete frame and projects only distance
  from live output and retained history, using checked subtraction. Separate
  position events, client counters, raw page internals and another history store
  are rejected. The canonical wire and authoritative publication tests in
  `orb-scrollback-position-cpb` prove the selected boundary. Ghostty's
  [1.3.0 scrollbar](https://ghostty.org/docs/install/release-notes/1-3-0#scrollbars)
  is UX comparison only; its native widget, dragging and rendering stay outside
  this producer slice. No dependency or source reuse is introduced.
- [FrankenTUI](https://github.com/Dicklesworthstone/frankentui) is required
  evidence for Orbit slices that change structured presentation generation,
  revision or damage semantics, or their proof architecture. Study its
  deterministic buffer-to-diff-to-present pipeline, headless backends, golden
  and property checks, and explicit one-writer ownership. Its
  [presenter-emission ADR](https://github.com/Dicklesworthstone/frankentui/blob/main/docs/adr/ADR-002-presenter-emission.md)
  is the closest patch-stream decision record. Orbit extracts lessons about
  deterministic state transitions and tests without adopting a terminal UI,
  widget framework, ANSI presenter, or FrankenTUI dependency.
- [libghostty-vt 0.2.1](https://docs.rs/libghostty-vt/0.2.1/libghostty_vt/) is the
  terminal-semantics owner under test. Study `Terminal`, `RenderState`, effects,
  input encoding, thread ownership, and the
  [VT formatter options](https://docs.rs/libghostty-vt/0.2.1/libghostty_vt/fmt/struct.FormatterOptions.html).
  The Formatter proof records why hidden terminal state cannot form an exact
  checkpoint. Study the public read-only surface for dimensions, styled cells
  and graphemes, cursor, title, working directory, hyperlinks, images, and
  explicit capability handling while inactive screens, parser continuation,
  scrollback semantics, and PTY effects remain authoritative in Orbit.
  The safe wrapper's
  [render module](https://docs.rs/libghostty-vt/0.2.1/libghostty_vt/render/index.html)
  and Ghostty's public
  [render header](https://github.com/ghostty-org/ghostty/blob/45db2c2551ecc016f9746e8e2855f4f8a3871e7b/include/ghostty/vt/render.h)
  expose global and per-row dirty state. This makes incremental presentation a
  future existing-dependency path rather than a reason to add a diff crate.
  For the ORBS v4 adjacent-row decision, release-mode `GridRef` extraction over
  400 interleaved samples stayed below 12 microseconds p95 at 80 and 240 columns
  with 32 and 1,000 retained rows. Complete-frame extraction ranged from 46.5
  to 173.8 microseconds p95 on the same 24-row terminal, and every lookup left
  scrollbar state unchanged. Orbit therefore uses the existing read-only grid
  API and rejects temporary viewport mutation, a new dependency, and a second
  row schema.
- Host-selection gestures use the existing
  [`libghostty-vt` 0.2.1 gesture API](https://docs.rs/libghostty-vt/0.2.1/libghostty_vt/selection/gesture/index.html)
  and pinned Ghostty
  [`a887df42c56f6de86c0fe6da9c4eeca37931e083`](https://github.com/ghostty-org/ghostty/tree/a887df42c56f6de86c0fe6da9c4eeca37931e083).
  `SelectionGesture.zig`, its C binding, and `Surface.zig` establish the adopted
  cell, word, and line behavior table, matching drag granularity, tracked-anchor
  reset, one-cell repeat distance, and 500-millisecond Linux repeat interval.
  Orbit supplies bounded surface position and monotonic press time, derives the
  grid reference from authoritative geometry, and reuses those APIs unchanged.
  `orb-click-selection-ladder-325` uses the native count (which saturates at
  three) and public event behavior/boundary setters to choose a space/tab-bounded
  word gesture on the third press, then a native line gesture on later repeats.
  Native validation still resets late, distant, backwards, or cancelled
  sequences. Matching span drag and upward-tick events keep the same tracked
  anchor; ordinary events retain native default word boundaries. No second
  click counter, text scanner, or dependency is introduced.
- WezTerm
  [`f93d90350075d3e42566e0557ca36e82ffdcbec1`](https://github.com/wez/wezterm/blob/f93d90350075d3e42566e0557ca36e82ffdcbec1/docs/config/mouse.md),
  Kitty
  [`479872838f7536ab87b8133471eb49d06804951b`](https://github.com/kovidgoyal/kitty/blob/479872838f7536ab87b8133471eb49d06804951b/docs/overview.rst),
  and Alacritty
  [`7dd7b5b09e06ca58daadfb12bd45a9aa8fa716f2`](https://github.com/alacritty/alacritty/blob/7dd7b5b09e06ca58daadfb12bd45a9aa8fa716f2/docs/features.md)
  are comparison evidence for single-cell, double-word, triple-line, matching
  drag granularity, and Shift bypass of terminal mouse reporting. Ghostty
  [`8867c37c55b578b9eb4cfaba41cb9023e557176d`](https://github.com/ghostty-org/ghostty/blob/8867c37c55b578b9eb4cfaba41cb9023e557176d/src/Surface.zig)
  additionally confirms release-time copy-on-select and semantic selection
  clipboard targeting. Orbit owns terminal-versus-host routing and copy
  destination tags; Venus owns native event and clipboard delivery. Orbit
  adopts no binding or configuration surface from these clients.
- Zellij
  [`b0bd3e1e7f530db8879e8cbde79de245e6101a8a`](https://github.com/zellij-org/zellij/blob/b0bd3e1e7f530db8879e8cbde79de245e6101a8a/zellij-server/src/tab/mouse_handler.rs)
  is comparison evidence for copy-on-release and terminal-owned mouse routing.
  Its configurable separators and OSC 133 command-output selection are
  deliberately excluded until dogfood establishes a separate need.
- [Ghostty at `d760ee96e546`](https://github.com/ghostty-org/ghostty/tree/d760ee96e54657416eb427b793c7e839f003df7d)
  is conditional evidence when a slice proposes WebAssembly, terminal-model
  reconstruction, or checkpoint-plus-tail replication. Its
  [snapshot API](https://github.com/ghostty-org/ghostty/blob/d760ee96e54657416eb427b793c7e839f003df7d/include/ghostty/vt/snapshot.h)
  encodes a CRC-protected renderable prefix, unfinished parser continuation,
  and incrementally restorable history. Snapshot format v1 has no binary-
  compatibility guarantee and omits Kitty image and placement state. The
  [WebAssembly example](https://github.com/ghostty-org/ghostty/tree/d760ee96e54657416eb427b793c7e839f003df7d/example/wasm-vt)
  exposes the low-level C ABI but no renderer, PTY, transport, authentication,
  font, or accessibility owner. Signed optimized artifacts come from a moving
  [nightly workflow](https://github.com/ghostty-org/ghostty/blob/d760ee96e54657416eb427b793c7e839f003df7d/.github/workflows/release-tip.yml),
  and upstream still describes API signatures as in flux. Preserve canonical
  structured frames and Orbit's sole authority by default. A snapshot replica
  requires a separately accepted replication contract after measured need;
  terminal-model restoration never implies PTY or process survival.
- [WezTerm at `577474d89ee6`](https://github.com/wezterm/wezterm/tree/577474d89ee61aef4a48145cdec82a638d874751)
  is the closest public structured-state replication comparison. Its
  [protocol types](https://github.com/wezterm/wezterm/blob/577474d89ee61aef4a48145cdec82a638d874751/codec/src/lib.rs),
  [server change calculation](https://github.com/wezterm/wezterm/blob/577474d89ee61aef4a48145cdec82a638d874751/wezterm-mux-server-impl/src/sessionhandler.rs),
  and
  [client row cache](https://github.com/wezterm/wezterm/blob/577474d89ee61aef4a48145cdec82a638d874751/wezterm-client/src/pane/renderable.rs)
  combine dirty line ranges, stable row indices, viewport-adjacent rows, lazy
  history fetches, and separate image-cell hydration. Orbit may borrow those
  protocol ideas without adopting WezTerm code, its terminal engine, or its
  coupled multiplexer surface.
- [ptyZZZ at `d23e046dff3b`](https://github.com/cablehead/ptyZZZ/tree/d23e046dff3b389d0a8a70929087bbfb2007ee5e)
  is conditional comparison evidence when measured presentation or transport cost
  justifies incremental frames, or an authorized browser slice needs a thin client.
  Its [protocol](https://github.com/cablehead/ptyZZZ/blob/d23e046dff3b389d0a8a70929087bbfb2007ee5e/PROTOCOL.md)
  and [emitter and regression tests](https://github.com/cablehead/ptyZZZ/blob/d23e046dff3b389d0a8a70929087bbfb2007ee5e/src/main.rs)
  keep one emulator beside the PTY, track stable-row damage, suppress identical
  redraws, and combine keyframes with diffs. A diff's `base` names the last frame
  actually emitted, not the terminal damage counter advanced by suppressed output.
  Study explicit resynchronization and fresh keyframes requested only after a
  subscriber is following live output. The bundled
  [web adapter](https://github.com/cablehead/ptyZZZ/blob/d23e046dff3b389d0a8a70929087bbfb2007ee5e/serve.nu)
  applies diffs without checking `base`; its demo does not establish the protocol's
  gap-rejection guarantee. Independently prove skipped-frame detection, no-op
  suppression, fresh attachment and slow-reader convergence before adopting a delta
  contract. Preserve Orbit's bounded complete frames until measured need, sole
  libghostty authority and rich structured presentation. MIT source inspection only;
  reject HTML as Orbit's wire format, WezTerm/portable-pty adoption, cross.stream
  storage, periodic healing as a substitute for coherent attachment, inherited
  multi-viewer scope, and performance extrapolation from the web demo.
- [Mosh](https://mosh.org/) and its
  [state-synchronization paper](https://mosh.org/mosh-paper-draft.pdf) show how
  a receiver can converge on recent state while the sender skips obsolete
  intermediate states. Mosh's [FAQ](https://mosh.org/#faq) also demonstrates
  the cost of synchronizing the visible screen: server-side scrollback does not
  reach the client as normal terminal history. Orbit may borrow state
  coalescing and resync semantics while retaining structured rich state,
  ordered effects, and a separate host-owned history plane.
- [zmx](https://github.com/neurosnap/zmx) is the closest implementation
  reference for Orbit's headless half. It owns persistent PTYs, communicates
  over Unix sockets, feeds output into libghostty, and emits a VT bootstrap when
  a client attaches. Study its formatter use, socket and process lifecycle,
  tests, and documented restoration failures. Independently prove Orbit's
  single-owner attach ordering, authoritative handling of alternate screens and
  split sequences, and one-client backpressure. Do not inherit its bootstrap
  plus raw-tail contract, multiple clients, SSH workflow, labels, commands, or
  session-management surface.
- [vt100](https://docs.rs/vt100/latest/vt100/) provides a small example of
  formatted screen state and state diffs. It is useful as a test oracle or
  counterexample, not as Orbit's production terminal engine.
- [termwiz](https://docs.rs/termwiz/latest/termwiz/) models screen changes and
  terminal capabilities. Study its change representation only if Orbit needs a
  comparison for state-diff testing; adopting its broader terminal stack is not
  part of the initial plan.
- Microsoft's
  [RDP Graphics Frame Acknowledgement](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/d64cfae6-f30a-47e7-9655-d019d3d8fb0f)
  is conditional evidence for client feedback, queue depth, and frame
  throttling. Consult it if local slow-client measurements require receiver
  feedback or the user authorizes remote transport.

## PTY and session lifetime

- [FrankenTerm](https://github.com/Dicklesworthstone/frankenterm) is required
  evidence before changing Orbit session lifetime, recovery, transport, or
  failure semantics. Study its explicit lifecycle states, protocol recovery,
  bounded failure handling, crash and reconnect checks, and separation between
  session authority and transient presentation. Reject its WezTerm, agent-swarm,
  storage, orchestration, multiple-pane, and remote-control surface unless the
  user separately chooses one of those product directions.
- [zmx](https://zmx.sh/) demonstrates the closest end-to-end lifecycle and
  explicitly separates session persistence from window management.
- [Foot 1.27.0 at `de998602dbc0`](https://codeberg.org/dnkl/foot/src/commit/de998602dbc00c8862a6823d553cbb1df91c676d)
  is comparison evidence for a terminal server process, local Unix-socket
  clients, and distinct client/server failure handling. Its
  [footclient manual](https://man.archlinux.org/man/footclient.1.en) describes
  the public connection contract. Orbit does not infer durable detach,
  presentation replication, or a dependency from Foot's server mode.
- [shpool](https://github.com/shell-pool/shpool) and
  [abduco](https://github.com/martanne/abduco) are smaller prior art for durable
  shell sessions and detach/reattach ownership. Study process reaping, signals,
  socket cleanup, and failure semantics rather than their command surfaces.
- [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) and
  [rustix](https://docs.rs/rustix/latest/rustix/) are candidate implementation
  levels for the Linux PTY slice. Compare their owned LOC, lifecycle control,
  dependency footprint, and ability to express the required checks before
  proposing either dependency.
- [tastty](https://docs.rs/tastty/latest/tastty/) is a compact Rust vertical
  slice joining PTY I/O, a background parser, and optional Ratatui rendering.
  Study its API boundaries and tests without inheriting its terminal engine,
  Tokio requirement, or drop-based session lifetime.
- [Process Triage](https://github.com/Dicklesworthstone/process_triage) is a
  required process-lifecycle reference before changing child discovery,
  liveness classification, signal escalation, reaping, or cleanup policy.
  Study its observe-plan-apply separation, protected-process boundaries,
  explainable classification, staged actions, and verification. Orbit owns a
  much narrower known child tree, so it must not import Bayesian classification,
  general host cleanup, fleet management, or automatic killing by default.

## Execution runtime gate

Orbit's current one-session proof uses an owned synchronous poll/event loop.
Do not replace or generalize it without a measured contract pressure and a
crate gate that compares all four credible shapes:

- [Asupersync 0.3.10 at `876110528f81`](https://github.com/Dicklesworthstone/asupersync/tree/876110528f810c3334a99450a521daa27995a3ec)
  for region-owned tasks, explicit cancellation and quiescence, bounded
  cleanup, and deterministic schedule or replay tests. Its unstable 0.x API,
  pinned-nightly default, broad runtime surface, and nonstandard MIT rider are
  crate-gate costs; its distributed RaptorQ snapshot machinery does not fit
  Orbit's reliable local stream.
- [Tokio](https://tokio.rs/) for ecosystem maturity, I/O coverage, diagnostics,
  and operational familiarity.
- [smol](https://github.com/smol-rs/smol) for a smaller composable async stack.
- the owned synchronous/event-loop design for minimum dependencies, explicit
  ordering, and the smallest authority surface.

The gate measures contract fit, cancellation and shutdown semantics,
deterministic-test quality, project-owned LOC, dependency and build cost,
macOS feasibility, and integration pressure from required crates. A runtime is
not selected because another reference uses it, and an async runtime is not a
prerequisite for the current single-session architecture.

## Native client and rendering

- [Wayland `wl_pointer` v10](https://wayland.app/protocols/wayland),
  [winit 0.30.13](https://docs.rs/winit/0.30.13/winit/event/enum.WindowEvent.html),
  [Ghostty at `45db2c2551`](https://github.com/ghostty-org/ghostty/tree/45db2c2551ecc016f9746e8e2855f4f8a3871e7b),
  [Rio at `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098),
  and WezTerm's pinned structured-state references above constrain vertical
  direct manipulation. Venus owns surface pixels, touch phases, accumulated
  remainders, velocity, and kinetic policy. Orbit exposes only one
  revision-bound bounded canonical row window and one typed whole-row commit
  outcome.
  It does not expose arbitrary history or adopt the comparison projects'
  same-process engines, caches, multiplexers, or renderers.
- OpenAI Codex
  [`0d226929622ce177b56e35d09cf39dd001721466`](https://github.com/openai/codex/blob/0d226929622ce177b56e35d09cf39dd001721466/codex-rs/tui/src/tui.rs)
  wraps TUI renders in crossterm synchronized updates. It supplies the producer
  sequence for the ORB-C8 held-scroll proof. Alacritty
  [`ede2ac144da4dec4c075bfa803aacf3b3739bce6`](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/event.rs)
  applies scroll through its terminal grid, while WezTerm
  [`27d55bef144f34a73e23585302838a36ec3aa30e`](https://github.com/wezterm/wezterm/blob/27d55bef144f34a73e23585302838a36ec3aa30e/term/src/screen.rs)
  keeps stable row identity as history advances. Orbit preserves that owner-side
  serialization by holding one existing protocol request through synchronized
  presentation; it adds no client history, retry outcome, or dependency.

- [Ghostling](https://github.com/ghostty-org/ghostling) and
  [libghostty-rs](https://github.com/Uzaaft/libghostty-rs) show small clients
  built around libghostty. Study how they bridge `RenderState`, glyphs, input,
  resize, callbacks, and a native render loop. Determine which rendering pieces
  can consume Orbit-authored presentation state without giving Venus a second
  terminal authority. Use measured evidence to choose Venus's window, font,
  and renderer owners.
- [rio-vt at `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/rio-vt) is the strongest
  Rust-native alternative terminal engine. Compare complete-state access,
  parser ownership, modern protocol support, API maturity, and dependency cost
  if libghostty cannot satisfy Orbit's contract. It replaces libghostty in that
  comparison; it is not an additional terminal engine.
- [Sugarloaf at `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/sugarloaf) is Rio's
  renderer and is relevant only when evaluating a compatible rendering stack.
  [librio at the same commit](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/librio)
  owns a PTY, Rio VT state, semantic input, callbacks, selection, and a
  host-pulled materialized render snapshot with dirty rows behind a C ABI. Its
  [design specification](https://github.com/raphamorim/rio/blob/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/specs/librio.md)
  is useful as a compact same-process engine/frontend comparison. Venus is a
  Rust client of Orbit's independently released protocol, so librio is neither
  a dependency nor a substitute for Orbit's detach boundary.
- [justerm-core](https://github.com/kihyun1998/justerm) is a reference for a
  server-authoritative design that sends structured grid and damage frames to
  thin native or web renderers. Study its frame ownership, damage model,
  canonical web decoder, `FrameSource` seam, and protocol costs while
  independently keeping Orbit's initial implementation to bounded complete
  frames. Orbit accepts responsibility for a presentation boundary but must not
  copy a text-grid ceiling, mirror justerm's schema, or adopt a speculative
  general protocol framework.
- [winit](https://github.com/rust-windowing/winit),
  [wgpu](https://github.com/gfx-rs/wgpu),
  [glyphon](https://github.com/grovesNL/glyphon), and
  [cosmic-text](https://github.com/pop-os/cosmic-text) form one credible
  Rust-native window, GPU, glyph, shaping, and text-rendering candidate set for
  the Venus crate gate. Compare exact releases or commits, owned LOC,
  dependency and build cost, input methods, accessibility, Linux behavior,
  macOS feasibility, and browser implications before selecting any part of the
  set. WGSL is relevant only if the selected GPU design requires shaders.
- [Unpeel Surface at `a3004c88a8b`](https://github.com/unpeel-com/unpeel-surface/tree/a3004c88a8b871837dac4820f48cf8a184a7276d),
  MIT, is comparison evidence for hybrid cell and pixel composition. Surface
  keeps menus, inputs, and navigation in terminal cells while placing rendered
  RGBA content beneath glyphs and explicit cell backgrounds through the Kitty
  graphics protocol. Its local file presenter requires a shared temporary
  filesystem and uses an mmap frame ring, synchronized updates, negative
  z-order, and bounded full-frame or damage-tile presentation. Orbit may use
  Surface as a later producer and layering oracle after the static resource
  plane is active. Orbit and Venus do not adopt its renderer, scene or WASM
  ABI, temporary-file transport, shared-filesystem assumption, frame ring, or
  damage policy. Surface runs on the application side and cannot prove
  libghostty image extraction or the Orbit-to-Venus resource contract; Yazi
  remains the first accepted producer path.

## Cross-stack runtimes and extensions

- The WebAssembly Component Model's
  [WIT](https://component-model.bytecodealliance.org/design/wit.html) and
  [composition model](https://component-model.bytecodealliance.org/composing-and-distributing/composing.html)
  are the primary references if repeated Eon or Venus extension cases justify
  a language-neutral capability contract. Study interface, resource, lifecycle,
  composition, and versioning behavior before selecting a runtime. They do not
  authorize plugins in the initial experiment or inside Orbit's owner loop.
- [Wasmtime's component API](https://docs.wasmtime.dev/api/wasmtime/component/index.html)
  and [Extism](https://extism.org/docs/concepts/plug-in-system/) are future host
  candidates, not selected dependencies. Compare them with an out-of-process
  standard-input/output contract and no general plugin framework after a WIT
  interface or equivalent capability boundary has been justified.
- [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) and the browser
  [WebAssembly JavaScript API](https://developer.mozilla.org/en-US/docs/WebAssembly/Guides/Using_the_JavaScript_API)
  are required references only for an authorized Eon Web slice. The preferred
  first comparison compiles Orbit's canonical Rust frame decoder to WebAssembly
  while TypeScript owns browser APIs; it does not assume the whole renderer is
  shared or authorize remote transport.
- [SwiftUI](https://developer.apple.com/documentation/SwiftUI) is conditional
  evidence for an authorized Apple-native Venus shell when Rust-native
  integration cannot satisfy measured lifecycle, input, menu, or accessibility
  requirements within the accepted cost. It is not part of the Linux-first
  proof.
- [Yazi's Lua plugin system](https://yazi-rs.github.io/docs/plugins/overview/)
  remains owned by Yazi. Eon may consume and pin that supported ecosystem;
  the reference does not justify embedding Lua or adopting it as a cross-stack
  plugin language.

## Terminal-facing utilities

- [Qwertty](https://github.com/joshka/qwertty) is useful for studying ordered
  terminal I/O, raw-mode restoration, query correlation, typed input, and
  policy gates for sensitive terminal features. It targets applications that
  own a controlling terminal, so it is not part of Venus's native client or
  Orbit's PTY core by default.
- [Ratatui](https://github.com/ratatui/ratatui) can support an optional
  diagnostic or administration TUI after the core contract is proven. It does
  not provide PTY ownership, terminal-emulator state, native windowing, or the
  graphical renderer required by Venus. Any Ratatui client is an explicitly
  lossy projection: it may exercise attachment, ordering, input, and reconnect,
  but it cannot prove `ORB-C6` rich-presentation fidelity.
- [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) is a conditional local
  tool for hardening a versioned frame decoder against malformed, truncated,
  reordered, or allocation-hostile input after deterministic checks establish
  a concrete decoder risk. Its nightly and sanitizer requirements do not belong
  in Orbit's default cost-bounded CI, and studying it does not approve a new
  direct dependency or fuzzing surface.

## Product and scope references

- [Amp Orbs](https://ampcode.com/manual/orbs) is a product-layer reference for
  client-independent work that can continue on a remote machine and be resumed
  from web, terminal, or mobile clients. An Orb combines a repository, agent
  thread, tools, terminal, services, and artifacts; its shared terminal
  currently runs inside tmux. Study its user model, setup and resume hooks,
  agent-readable logs and readiness surfaces, service portals, and separation
  between a durable environment and transient clients. It does not answer
  Orbit's PTY ownership, terminal-state convergence, or native-rendering
  questions.
- Amp's remote-machine provisioning, repository synchronization, secrets and
  identity, billing, wake policy, event system, multiplayer, and workspace
  orchestration belong above Orbit if Yazelix ever chooses that product scope.
  They remain excluded from the initial experiment. If Orbit grows from an
  internal runtime into a remote workspace product, revisit the naming
  proximity between Orbit and Orbs.
- VS Code's
  [Agent Host](https://code.visualstudio.com/docs/agents/concepts/agent-host)
  keeps agent execution independent of the client and reconnects UI through
  snapshots and ordered actions. [Zed Remote
  Development](https://zed.dev/docs/remote-development) keeps local UI apart
  from workspace processes beside the project. These projects corroborate the
  Eon, Venus, and Orbit ownership split. They do not define Orbit's terminal
  protocol or authorize remote workspaces.
- [TUIOS](https://tuios.gaurav.zip/) is a warning and inspiration for
  terminal-as-workspace scope. Orbit takes no tiling, workspace, theme, or
  scripting features from it during the initial experiment.
- [Herdr](https://github.com/ogulcancelik/herdr) demonstrates durable agent
  terminals in a Rust client/server multiplexer. Orbit studies its user-visible
  detach and reattach behavior while excluding layouts, remote access,
  multiplayer, and agent-management policy.
- [Canario](https://rioterm.com/canario) publishes signed builds through
  [`canarioterm/releases`](https://github.com/canarioterm/releases/tree/fcadcfeea7e337114d28beec0e0cbe89493a654d).
  That repository identifies the application source as private, and release
  [`v2026.08.07.15`](https://github.com/canarioterm/releases/releases/tag/v2026.08.07.15)
  records a build from private revision `canario@af91699`. Rio commit
  [`ff8efcc4ad03`](https://github.com/raphamorim/rio/tree/ff8efcc4ad0359889de44b1baacea7014cdada30/frontends/canario),
  the final public tree before
  [Canario's extraction](https://github.com/raphamorim/rio/commit/e69b5fffef25abd5e649bf4ac205636551e54482),
  remains the inspectable comparison for a browser-inspired native terminal
  workspace: spaces, splits, a command palette, CWD-based filing, a global
  quick terminal, and on-demand live pane previews. Its SwiftUI/AppKit frontend
  owns those policies around same-process librio surfaces. The
  [session store](https://github.com/raphamorim/rio/blob/ff8efcc4ad0359889de44b1baacea7014cdada30/frontends/canario/Sources/SessionStore.swift)
  persists layout, CWD, title, and plain-text scrollback;
  [RioEngine](https://github.com/raphamorim/rio/blob/ff8efcc4ad0359889de44b1baacea7014cdada30/frontends/canario/Sources/RioEngine.swift)
  starts a fresh shell and replays that text into a new display rather than
  preserving the process. The feature page retains Rio source and download
  links, so Orbit uses the pinned `canarioterm/releases` revision as distribution
  evidence and the final public Rio tree as mechanism evidence. Eon may study
  command-driven navigation and on-demand previews. Orbit rejects same-process
  UI/PTY lifetime and textual restoration as substitutes for `ORB-C1` detach
  survival.
- The [Superlogical terminal multiplexer](https://www.superlogical.com/) is the
  company's published first product; Mitchell Hashimoto's
  [company announcement](https://mitchellh.com/writing/superlogical),
  [architecture explanation](https://x.com/mitchellh/status/2082936029426892960),
  [native tabs and splits
  demonstration](https://x.com/mitchellh/status/2084630173954326672),
  and the
  [libghostty roadmap](https://mitchellh.com/writing/libghostty-is-coming)
  are comparison evidence for durable terminal ownership and transient native
  clients. Its published architecture pauses the authoritative libghostty
  server at attach,
  sends a custom binary reconstruction snapshot, reaches a ready state before
  older scrollback completes, then distributes raw PTY bytes to terminal
  emulators in its clients. Orbit accepts the server-owned PTY, authoritative
  terminal, and coherent attachment barrier as corroborating mechanisms. It
  does not adopt client-side terminal replicas, checkpoint-plus-tail transfer,
  incomplete initial history, compatibility rendering, multiple clients,
  remote transport, splits, tabs, or Superlogical's broader production-work
  scope. Orbit relies only on published behavior and independently verified
  APIs.
- Zellij, Mars, and Mars Next remain comparison and behavioral references for
  Eon dogfood and graduation. Their existing ownership or compatibility
  surfaces do not define Eon, Venus, or Orbit.
