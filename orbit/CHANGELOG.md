# Changelog

Orbit records accepted user- and consumer-visible changes here. Contracts
remain canonical in `docs/CONTRACTS.md`, dependency decisions in
`docs/CRATES.md`, and implementation evidence in Beads.

## Unreleased

- Accepted Orbit `427cf2d074cd1db905a2cc2602a5475065d1c790` pins
  libghostty-vt 0.2.2, whose vendored build targets Zig's portable baseline CPU
  by default instead of inheriting native build-host instructions. The pinned
  Ghostty engine, generated bindings and ORBF v2 / ORBS v13 stay unchanged.
  Downstream Eon package and cache adoption are separate.

- Accepted Orbit `6bc269c40b18f08b95778939518f77556ba91c67` changes
  `ORB-C9` under unchanged ORBF v2 / ORBS v13: double-click selects a native
  word, triple-click selects a space/tab-delimited span including punctuation,
  and fourth-or-later clicks select logical lines. Dragging and upward ticks
  extend at the same granularity while live output continues. Spans retain
  libghostty's native wide-character selection limitation. Venus/Eon delivery
  remains a separate accepted-source adoption.

- Accepted Orbit `b6cecf8f2ee35570b41cfdc578b095889d917fe2` extends
  `ORB-C8` and `ORB-C9` under exact-version ORBS v13: a held host selection can
  tick upward through retained history one row at a time, preserving the same
  tracked gesture and returning the applied distance with a complete frame.
  ORBF v2 is unchanged. Venus and Eon retain their ORBS v12 pins until separate
  adoption; native macOS proof of the new action remains open.

- Accepted Orbit `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` changes
  `ORB-C8` under exact-version ORBS v12: one `ReturnToLive` request moves the
  primary viewport to live output without PTY input or repeated row batches.
  ORBF v2 is unchanged. Venus, Eon, and Eonova retain their ORBS v11 pins
  until separate adoption; native macOS proof of the new action remains open.

- Accepted Orbit `0233f4d34b294a50c5bc7f373859cf4ec04d2414` proves ordinary and
  managed real-PTY Sessions on Apple Silicon macOS with the shared Unix runtime
  path, including attach, input, resize, detach/reconnect, EOF, private
  endpoints, bounded signal shutdown, management replacement and Stop, and
  fail-closed identity checks. Venus, Eon, packaging, and distribution adopt
  macOS separately (ORB-C14).

- Publish authoritative scrollback distance and retained-history display-row counts
  in every complete frame, including output, reflow, eviction and reattachment.
  ORBF v2 / ORBS v11 is an explicitly breaking exact-version boundary without a
  compatibility layer (ORB-C4/C6/C8, `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`).
  Venus/Eon consumption and the visible indicator remain separate deliveries.

- Accepted Orbit `91999d79546422b49bdbc124166a65859d0bd872` hardens `ORB-C9`
  under unchanged ORBS v10: a selection Begin naming an already-presented,
  non-future frame resolves once against current authoritative state instead of
  racing continuous PTY output. Future revisions and existing geometry,
  routing, pressure, and cancellation checks remain strict. Venus
  `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` and installed Eon
  `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f` complete `ORB-C9` native
  selection and copy acceptance during live output.
- Accepted Orbit `61c1dc0bc4c9fc3b592058ff8fa5f6cd7b957046` changes `ORB-C9`
  under unchanged ORBS v10: compatible PTY output preserves active host
  selection and repeat-click classification while libghostty tracks content
  movement, including into scrollback. Newly enabled terminal mouse tracking
  or an invalid tracked anchor still cancels the gesture safely; Venus and Eon
  adopt the owner proof separately.
- Accepted Orbit `64b225eb249490c9075894814942652a8b9d6192` changes `ORB-C8`
  under unchanged ORBS v10: one relative preview or signed-scroll request held
  during DEC 2026 synchronized output resolves once after normal or watchdog
  release. PTY parsing continues, partial presentation stays hidden, and a
  second unresolved request fails without replacing the first.
- Accepted Orbit `a65e199e16e97330175e314cacf791fa00f53069` hardens `ORB-C7`
  and `ORB-C8` under unchanged ORBS v10: a partially transmitted maximum frame
  no longer falsely exhausts the bounded capacity required by one atomic
  maximum scroll outcome. Ordering and replacement semantics are unchanged;
  Venus and Eon adopt the owner proof separately.
- Accepted Orbit `4e03fe280a714c3f3e29d31d0ec6374eaaf4ecea` changes `ORB-C8`
  under unchanged ORBS v10: lagging relative preview and signed-scroll requests
  resolve against current authoritative terminal state, keeping scrollback
  usable while PTY output advances. Future revisions and unrelated stale input
  remain rejected; Venus and Eon adopt the owner proof separately.
- Accepted Orbit `70861097a825c2fbfaea53a8ca9437f45e8602eb` hardens `ORB-C9`
  under unchanged ORBS v10: host selection keeps only the latest wholly unsent
  presentation frame while retaining ordered acknowledgements, completion, and
  copied text. Touchpad drag tracks promptly and stops at release instead of
  replaying obsolete frames.
- Accepted ORBS v10 at `59975e9176f5caf8b78dc3273e88d9ecbb75dc3f`
  completes `ORB-C9`: every accepted pointer Finish reports the authoritative
  presentation revision, including host Finish and terminal Begin frames held
  by synchronized output. Consumers can serialize rapid pointer sequences
  without route or timing inference; no adapter or compatibility window is
  retained.
- Accepted ORBS v9 at `aed0bcb7e9ad08c8e3e086c7dad0a0eb3ef16672`
  moves terminal-versus-host left-pointer routing into Orbit, uses Shift as the
  host-selection bypass, pins each route through completion, and tags frozen
  text for the semantic selection or ordinary clipboard. Terminal phases reuse
  Orbit's existing terminal-aware mouse encoder; native delivery remains
  Venus-owned.
- Accepted ORBS v8 at `d9b22eb294f8f42b4f49324fd5467eab239c2917`
  replaces client-resolved selection cells with bounded surface positions and
  monotonic press time. Orbit delegates cell, word, logical-line, and matching
  drag behavior to pinned libghostty, while explicit cancel resets abandoned
  gestures. Native input routing and clipboard effects remain Venus-owned.
- The accepted `baf8aa28dcaa50484cd221aa7730defedc2356bb` proof changes
  `ORB-C8` and replaces ORBS v6 with exact-version ORBS v7. Revision-bound
  preview and signed-scroll results expose up to one active viewport of
  nearest-first canonical rows while preserving Orbit's history, routing,
  revision, mutation, and payload authority. Venus and Eon adopt the owner
  proof separately without an adapter, dual-version window, or dependency
  change.
- The accepted `780f5d746175b4a9b71df57c51ed4bfcc4c4c375` proof changes
  `ORB-C8` and replaces ORBS v5 with exact-version ORBS v6. One revision-bound
  request commits up to 1,024 signed whole rows in one native viewport
  operation and returns exact requested/applied rows, one complete frame, and
  the next adjacent row or edge. A routing change returns terminal-owned
  without PTY input or viewport mutation. Venus and Eon adopt the owner proof
  separately without an adapter, dual-version window, or dependency change.
- The accepted `e86a036a4481ea2be55012a38c13f75204b81278` proof extends
  `ORB-C8`: each Session uses a 16 MiB primary-history byte budget. History is
  allocated as needed, retained row count depends on content, and libghostty
  may exceed the budget to preserve the active viewport.
- The accepted `345fa7b87d0b5f332038140ce8d0e06d3faa9df3` proof replaces
  `ORB-C12` and refreshes `ORB-C13`: ordinary Linux PTY Sessions no longer
  require delegated cgroup access. Explicit Stop performs bounded process-group
  cleanup and direct-child reaping without delayed forced signaling after the
  shell exits during grace. Detached processes and SIGHUP-ignoring foreground
  jobs may survive; client disconnection remains non-destructive.
- The accepted `0ca0cc93b83793d08eadcca7eac6e947a80ce25d` proof hardens
  `ORB-C12`: Orbit permits up to one second for an external launcher to finish
  placing it in a cgroup before PTY startup, while retaining the exact parent
  ownership, mode, and membership checks and bounded fail-before-exec behavior.
- The accepted `69c402737799f03e615473956954a043647a4713` proof changes
  `ORB-C3` and replaces ORBS v4 with exact-version ORBS v5. The existing
  private presentation endpoint admits one input-capable attachment and one
  bounded read-only observer for initial and changed terminal title, raw
  working directory, exact presentation revision, and lifecycle. Observer
  state is coalesced, capped by an 8 KiB metadata envelope, and grants no
  terminal input, presentation, process, or lifecycle authority. Venus and
  Eon adopt the owner proof in that order without an adapter, dual-version
  window, or dependency change; current composed ORBS v4 sources remain valid
  until adoption.
- The accepted `b2fbfe1a718b77dbd37d9c82370bec83584384ae` proof hardens
  `ORB-C5`: under Kitty keyboard reporting, a text-bearing Space release is
  encoded as a release sequence instead of a second literal Space.
- The accepted `2518512758c7848bbd87f907b105c8bcf0fc4e1b` owner proof hardens
  `ORB-C13`: an optional exact empty record claim serializes the Ready boundary.
  A launcher-held claim prevents publication; an Orbit-held claim is marked
  before Live replacement so later record removal cannot restore pre-Ready
  local-child authority. Eon and Eonova adopt the owner revision separately.
- Prove `ORB-C13` through exact Eon consumer acceptance at
  `0bf0b165d06b4a8162be497011070f61f6c2000a`: same-boot supervisor loss,
  multi-Session and EonTerm recovery, contending replacements, and owner-routed
  Stop preserve Orbit authority without fallback or duplicate ownership.
- The accepted `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c` proof adds
  `ORB-C12`: explicit Linux Session shutdown reaps the direct PTY child and
  empties the exact Orbit-owned cgroup before reporting success. Processes
  deliberately handed to another lifecycle scope are outside Orbit ownership;
  graphical-client disconnection remains non-destructive under `ORB-C1`.
- The accepted `2e41997cbec202afc5222f084ffb98973a01b0f0` proof hardens
  `ORB-C4` and `ORB-C6`: canonical ORBF v1 rejects malformed wide-cell,
  spacer-tail, and wrapped-edge topology before encoder, decoder, or reducer
  acceptance. Valid ORBF v1 and ORBS v4 bytes are unchanged.
- The accepted `9c0617a97612cdd045ed67b5cd7c87244eb888e6` proof changes
  `ORB-C8` and replaces ORBS v3 with exact-version ORBS v4. A client may preview
  one revision-bound adjacent canonical row without moving Orbit's viewport or
  sending PTY input. Each vertical wheel returns a typed terminal-routed result
  or one atomic applied-row count and newer complete frame. Venus and the pinned
  Eon/Eonova runtime require separate owner-first adoption; no adapter, dual
  decoder, feature probe, or compatibility window is provided. ORBF remains v1.
- The accepted `6de95296d252c119d4fdba2d9b03cec1a09355ae` proof hardens
  `ORB-C6`: canonical ORBF v1 rejects capability values that contradict its
  advertised hyperlink support or explicit lack of Kitty graphics before
  encoder, decoder, or reducer acceptance. Valid ORBF v1 and ORBS v3 bytes are
  unchanged.
- The accepted `9bc87191dd90fd3d7db939127f1ebedfccd2b48d` proof hardens
  `ORB-C1`: Orbit serializes stale-socket inspection, removal, bind, and setup
  through the validated private parent directory. Simultaneous launches leave
  one reachable owner and one explicit loser, without allowing the loser to
  unlink the winner's socket. ORBF v1 and ORBS v3 are unchanged.
- The accepted `cb0703010c3a4944980404b665abff795be09fb4` proof hardens
  `ORB-C1`: Linux PTY-master `EIO` remains retryable while Orbit's direct child
  is alive, so a temporary zero-slave interval no longer closes the Session.
  Recovery is bounded to avoid CPU spin, and later presentation and semantic
  input continue through the same authoritative terminal. ORBF v1 and ORBS v3
  are unchanged.
- The accepted `83d700882ee156bac0de0112b3f1daade049b9fb` proof hardens
  `ORB-C1` and `ORB-C7`: PTY-side output pressure may disconnect a concurrently
  ready client without letting the stale poll result panic Orbit. The live PTY
  session remains available for later attachment. ORBF v1 and ORBS v3 are
  unchanged.
- The accepted `26b4b9465b3f0e74f091a0aae93fddc61412b893` proof hardens
  `ORB-C4` and `ORB-C6`: Orbit honors authoritative DEC 2026 synchronized
  output, continuing terminal parsing, replies, and ordered effects while
  holding intermediate frames. Normal completion publishes the latest complete
  frame; a one-second watchdog releases abandoned holds, and a new client's
  initial frame waits for completion or timeout. ORBF v1 and ORBS v3 are
  unchanged.
- Orbit accepts one optional, versioned 16-color ANSI palette from its
  component launcher before PTY startup. The supplied defaults appear in the
  existing ORBS frames and remain the target of OSC 104 resets; standalone
  launches retain libghostty defaults.
- Orbit PTY children advertise `TERM=eon`. The repository-owned terminfo entry
  inherits the conservative `xterm-256color` baseline and is installed with
  standard `tic` before source-run sessions.
- Orbit owns one real Linux PTY shell and the sole authoritative libghostty
  terminal state in a foreground process.
- The bounded local client can detach and reattach to the same live shell,
  while Orbit admits exactly one client and owns semantic input encoding.
- The private Unix socket and child process have explicit stale-path,
  permission, child-exit, foreground-process, and signal-shutdown behavior.
- Attachment begins with a bounded, versioned, complete structured
  presentation frame followed by ordered revisions. Frames carry rich terminal
  presentation state and explicitly declare Kitty graphics unsupported.
- The accepted `840a67c0cb32b334ed54888321d5ca77e58117b0` proof adds
  `ORB-C8`: Orbit owns the libghostty viewport across client disconnection,
  routes vertical wheel input through authoritative mouse and alternate-screen
  modes or retained history, and returns scrolled primary output to the live
  area after terminal-producing key input.
- ORBF v1 has one dependency-free canonical Rust package for Orbit and pinned
  consumers, with owned frame values, strict bounded decoding and encoding,
  and rejection of stale or duplicate complete-frame revisions.
- ORBS v1 adds the dependency-free canonical attachment, frame-envelope,
  lifecycle, semantic key, mouse, focus, arbitrary paste, and resize codec at
  `838b67652c4df1979e599b9c401ee664ffac66bd`, hardening `ORB-C3` and `ORB-C4`
  and changing `ORB-C5` to the reusable Venus boundary. This intentionally
  replaces the private unaccepted line-oriented diagnostic syntax without an
  adapter or compatibility window; Orbit's in-repository consumers moved
  atomically, and Venus pins the accepted revision. Role-incompatible kinds and
  message-specific invalid lengths, including a frame shorter than any
  canonical ORBF value, fail from the header before payload-sized buffering,
  and PTY resize preserves the accepted full surface pixel geometry
  instead of reconstructing only the cell grid. Key-associated text rejects C0,
  DEL, and macOS function-key PUA values before terminal encoding; clients send
  those inputs as semantic keys rather than text. Positional modifier bits are
  rejected unless their corresponding logical modifier is active, consistently
  on encoding and decoding. Buttonless mouse input is valid only for bare
  motion, wheel directions are valid only as momentary press events, and the
  terminal mapper distinguishes those ticks from held-button motion outside the
  viewport. Mouse axes and surface pixel dimensions are bounded to the selected
  terminal mapper's `u16` domain before native coordinate conversion. After a
  fatal negotiation or protocol response is queued, Orbit
  stops accepting input and publishing later frames or lifecycle messages to
  that connection while the bounded response drains, and immediately releases
  unread client bytes and their retained input storage. After Orbit observes
  PTY closure, later semantic requests receive a terminal failure instead of a
  false acknowledgement, and already queued undeliverable bytes and their
  retained queue storage are released.
- The accepted `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757` proof replaces
  ORBS v1 with ORBS v2 and adds `ORB-C9`: revision-bound current-viewport
  selection, authoritative selected presentation, and bounded plain-text copy.
  A successful Finish freezes one immutable client-scoped value that survives
  later terminal output, resize or reflow, and active-screen transitions; a
  newer selection, client loss, or exit clears it. Venus consumes ORBS v2
  through `ven-4sn` without a v1 adapter or compatibility window.
- The accepted `3ee7c80005f3d2bbe81e539799327803716f6174` proof replaces
  the Orbit producer boundary with ORBS v3 and adds `ORB-C11`: bounded,
  normalized terminal clipboard writes are delivered once and in order to the
  attached client before the frame from the same PTY turn. Invalid, unsupported,
  detached, or pressured writes fail closed. Venus native delivery updates
  separately without a v2 adapter or compatibility window.
- The authoritative owner loop yields after each productive PTY read and caps
  the shared terminal-response and semantic-input backlog. A whole semantic
  input that exceeds the remaining bound receives a terminal failure and closes
  that client without partial admission; terminal-response overflow fails the
  session instead of silently losing an authoritative reply. The accepted
  `fdd55e1e8bf6b932c2949ea3c897601ff63c0743` pressure proof hardens `ORB-C3`
  and `ORB-C4` while advancing the transport evidence for `ORB-C7`.
- The accepted `9e9b136969dc583d365daa75402ec03c265080f1`
  lifecycle proof hardens `ORB-C1` and `ORB-C3` and proves `ORB-C7` for the
  local one-client boundary. Silent pre-attachment peers release the only slot
  after a bounded deadline, simultaneous peers receive deterministic Busy,
  reattachment receives the retained authoritative dimensions, and known-child
  exit stops owned process groups before a fixed final PTY-read budget. A
  continuously writing descendant cannot hold Orbit or its socket open.
- The accepted `8e3ba0beeb18157dc5a48c68c38812fa8d8fb779`
  terminal-authority proof hardens `ORB-C2`. Partial private-mode parser state
  survives client failure without export or reconstruction; Orbit alone sends
  the ordered terminal reply while detached, and canonical attachment cannot
  duplicate it. Ordered client-visible effects remain a separate transport gap.
- Venus is the accepted first external ORBS v1 and ORBF v1 consumer. Its
  accepted relationship is recorded at
  `8149002c7275a00db1c08fded171e09f249dd977`, and it pins Orbit
  `c905bf9610581747f1b07565814b501ca66cfaa6`; the later Orbit proof
  `838b67652c4df1979e599b9c401ee664ffac66bd` is compatible without a manifest
  migration because the protocol package and Cargo metadata are byte-identical.
