# Venus contract index

This is the canonical current state of Venus behavior, ownership, proof, and
remaining limitations. Beads and Git retain execution history; `CHANGELOG.md`
retains accepted user-visible chronology.

`VEN-C16` owns platform scope. The current locked consumer uses ORBF v2 /
ORBS v13 at Orbit `6bc269c40b18f08b95778939518f77556ba91c67`, with source
acceptance indexed in `VEN-C7`, and EONW v7 at Eon Runtime
`b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, with provider evidence in `VEN-C8`.
Older consumed revisions below
identify their named proofs; they do not select the current dependency.

## VEN-C19 — Startup typography and terminal geometry

- **Status:** Proved
- **Consumer:** One direct or caller-launched Venus surface.
- **Trigger:** Optional `--font-family`, repeatable `--font-fallback`,
  `--font-size`, `--line-height`, `--columns`, and `--rows` startup options.
- **Result:** Installed primary monospace and ordered fallback families feed the
  existing shaper. One `CellMetrics` grid owns rendering, resize, pointer and
  selection geometry, scrolling, cursor, IME and accessibility. Family names
  are nonempty trimmed UTF-8 without controls, at most 128 bytes; at most eight
  fallbacks are accepted. Nominal font size is 6–96 logical px (default 16);
  line height is 1–3 times that size (default 1.125). Named overrides precede
  specialty/platform fallback. No font override preserves existing font choice
  and specialty handling; no overrides preserve 10 by 18 logical cells and the
  960 by 600 logical initial window.
- **Accelerando specialty-font proof:** Source
  `74d1ca05913ebe7ac639b887a78ed038a0761045` routes U+100000–U+101FFF
  through the existing specialty path only when no font override is supplied.
  Locked fmt/check/test (147 ordinary tests), clippy and release build pass.
  Anima `395016b` native-size and mixed playback measure 17.67 and 17.87 distinct
  updates/s versus 3.51 before the correction; maximum steady gap is 100 ms.
  The native specimen has identical pixels and terminal grid. Resize, browse,
  keyboard exit and timed exit pass on private Sway 1.12, pixman/lavapipe,
  scale 1; host GPU and broader compositor performance remain unproved.
  Exact raw receipts: `ven-shape-cache-2026-10-11-ztqchs9d` and
  `anima-stutter-2026-10-11-c71d0vjg` under Eon's local proof root.
- **Initial geometry:** Optional positive `u16` columns/rows request terminal
  dimensions, with workspace headers, stack bottom margin and popup margins
  supplied by the existing Scene owner. Omitted dimensions retain the existing
  initial window dimension.
  Requests must fit Orbit's 100,000-cell limit and native pixel/GPU bounds.
  Compositor sizing policy may override the request; actual resize remains
  authoritative. Later user resizing is not constrained to the initial grid.
  Stack bottom-margin accounting is accepted at source
  `bc5a2bd3b2363abdea69c4cd89953b612bc970e8`: the existing initial-grid check
  passes for standalone/workspace/picker at simulated scales 1/1.25/1.5/2.
  Native installed proof is indexed with the pane-stack polish below; previous
  typography and fractional-scale qualifications remain in force.
- **Important failures:** Invalid values, duplicate singleton options, absent
  named families, a non-monospace primary, or impossible initial dimensions
  fail with bounded diagnostics before terminal presentation/attachment.
  Native output admission can require an initial window buffer to learn scale.
  Missing named fonts are not silently substituted. Availability does not
  promise every Unicode glyph or style variant. No live reload is provided.
- **Owner:** Venus launch, font fitter, `CellMetrics`, and workspace Scene;
  Orbit retains terminal authority and Eon retains product configuration.
- **Consumes:** ORBF v2 / ORBS v11 at Orbit
  `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, EONW v5 at Eon
  `0cc8f477298681ae3945903e8fdb5852d487c5ab`, and the selected glyphon 0.12.0 /
  cosmic-text 0.19.0 APIs with exact unicode-script 0.5.8.
- **Compatibility order:** Prove Venus first with unchanged defaults and wire
  contracts, then let the separate Eon configuration issue consume that exact
  accepted source for Eon and EonTerm.
- **Supervised startup admission:**
  `EON_VENUS_PRESENTATION_CONTROL=stdin-ready-v1` uses private stdin/stdout.
  Workspace mode first reads one bounded canonical EONW v7 Snapshot from stdin;
  standalone mode has no snapshot prefix. After font/renderer initialization
  and real-window initial native-scale admission, Venus writes exactly
  `ready-v1` (eight bytes) and starts attachment. The caller may then start
  Orbit and the user command. Invalid fonts, geometry or snapshot produce no
  readiness; the caller bounds startup and closes a failed attempt. Existing
  `present\n` and EOF control retain their meanings. Later user/compositor
  resizing is outside this initial admission. An empty popup-only tab admits
  usable body space without requiring or inventing an attachment. The v5
  consumer refinement is indexed in VEN-C18; older startup proofs remain scoped
  to their exact snapshots and component revisions.
- **Startup-admission proof:** `ec80e36625dec73544c0cb64becf4b135c932a63`.
  Locked fmt/check/test/clippy pass (120 ordinary tests); five existing native
  checks pass in separate processes on x86_64 Linux, Sway 1.12 / lavapipe
  26.1.1 at 1.25 scale. The actual standalone/workspace window reports readiness
  for defaults and explicit grids without an Orbit process. Missing fonts,
  impossible workspace geometry and malformed snapshots exit without readiness;
  Present and EOF shutdown pass. The final native grid is checked before Ready.
  Reproduction inputs and observations are retained under
  `~/.local/state/eon/proofs/ven-c19-ec80e36625dec73544c0cb64becf4b135c932a63/`.
  Eon `b693a877cd4d630fe15aabf852de4e978fc166dc` accepts the exact Nix artifact
  `/nix/store/37v7m8kb88fgs4lgwk7ajcbf9hjb8pn7-yazelix-venus-0.1.0` through
  both refreshed product profiles. Installed configured initial/reopened grids,
  missing-font rejection before commands, native input, exact selection/copy,
  IME preedit/commit and accessible text/bounds pass. Eon's EON-C19 index owns
  exact artifacts, scale observations and unchanged Orbit/command identities.
  Earlier typography, fractional-scale, compositor and other partial evidence
  remains qualified by its recorded identities and observations below.
- **Proof:** `1b32e5ba7105d14f654136578a65c97a25b53fc1` (startup correction).
  Locked fmt/check/test/clippy and Nix package checks pass: 118 ordinary tests
  and five native regressions. Initial explicit-grid and font-only startup
  complete without input at scales 1, 1.25, 1.5 and 2 on Sway 1.12/lavapipe
  26.1.1. The watchdog can only fail startup; it cannot supply a successful
  admission wakeup as the earlier probe did.
  Package `/nix/store/k1pdjvzha5skyrp2nsvn0ih6gz6lj2j6-yazelix-venus-0.1.0`
  produces real PTY grids of 73 by 18 standalone, 73 by 12 in a three-pane
  workspace, and 71 by 15 in the picker at 1.25 with 20 px / 1.5 typography
  and omitted columns/rows. Both workspace modes reject 96 px / 3 typography
  with exit 1 and zero observed Orbit connections.
  Reproduction inputs and costly build evidence are retained under
  `~/.local/state/eon/proofs/ven-c19-1b32e5ba7105d14f654136578a65c97a25b53fc1/`.
  Eon `e10a01ee93d6d32d7312d1c82ff6f524aca31583` consumes this exact source;
  both refreshed profiles match their builds. Installed EonTerm PTY startup
  and Eon directory-picker presentation pass at 1.25. Eon's contract index
  records the artifacts and observation limits.
- **Typography matrix proof:** `9157f7fbed0318d94a0697c01a23de2bed86946a`;
  x86_64 Linux, isolated Sway 1.12,
  Mesa 26.1.1 lavapipe, DejaVu Sans Mono and Symbols Nerd Font Mono.
  - Locked fmt/check/test/clippy passed: 118 ordinary tests. All three ignored
    native tests passed, including configured live-output input/selection and
    fractional initial grid admission before attachment. Nix package tests pass.
  - Default and configured (20 px, 1.5 line height) standalone, plus configured
    three-pane and picker surfaces, produced exactly 100 by 30 in a real Orbit
    PTY at 1, 1.25, 1.5 and 2 scale. Latin, Unicode, wide/combining text,
    regular/bold/italic, Nerd symbols, Braille, boxes and cursor were inspected.
  - Missing primary/fallback, proportional primary, NaN and physical native-size
    overflow exited nonzero without any connection to the observed Orbit socket.
    A later manual resize and scale change retained the user-selected geometry.
  - Nix artifact `/nix/store/yc8w1y1q4g8m4wwbhvl749pkpa1vcnim-yazelix-venus-0.1.0`
    passed configured picker dogfood at 1.25 scale: native drag and exact copy
    of `Regular`, Wayland IME preedit `é界` and exact commit `ime-é界`, and
    AT-SPI text with terminal bounds `[15, 105, 1530, 1170]`, matching the
    physical 15 by 38 grid, padding and picker inset. IME cursor requests use
    the corresponding logical coordinates; the preedit capture was inspected.
  - **Downstream acceptance:** Eon `0461737272447fa976d13bd3376c8a894cebf324`
    consumes this exact source. Both installed profiles match their builds and
    pass isolated native default-geometry/input observations; Eon’s contract
    index owns the exact artifacts. Persistent typography configuration remains
    its separate issue.
  - Reproduction inputs, observations, captures and costly Nix log are retained
    under `~/.local/state/eon/proofs/ven-c19-9157f7fbed0318d94a0697c01a23de2bed86946a/`.
- **Limits:** This proves the named typography/geometry slice on Sway; it does
  not widen earlier fractional/HiDPI, VEN-C8, VEN-C18 or compositor coverage.
  The pinned winit destroys text input when a seat capability disappears;
  native IME proof uses a stable keyboard/pointer seat. That pre-existing
  hot-unplug limitation is outside this typography change.

The current proof advances `VEN-C1`, `VEN-C2` and `VEN-C4` only for startup.
Their distinct earlier proof identities and remaining limits stay qualified.

## Status

- **Planned:** accepted contract with no sufficient implementation evidence
- **Candidate:** implemented and verified without an accepted proof-bearing
  revision
- **Partially proved:** a useful exact slice is proved but required evidence is
  still open
- **Proved:** the listed immutable revision and evidence cover the contract
- **Retired:** explicitly replaced or removed; its ID is never reused

## VEN-C5 — Explicit native hyperlinks

- **Status:** Proved on x86_64 Linux; partially proved on Apple Silicon macOS.
- **Consumer/trigger:** A Venus user hovers an Orbit-authored OSC 8 link or
  explicitly activates one.
- **Result:** The hovered link is highlighted and its escaped target is shown
  with Open and Copy actions. Linux uses `Ctrl+click` and `Ctrl+Shift+C`; macOS
  uses `Command+click` and `Command+C`. Opening requires a press and release on
  the same current presented target. Copy uses the current hovered target unless
  terminal selection retains precedence. AccessKit exposes each current visible
  target as an Open link followed by a Copy button, both using the native Click
  action. Ordinary terminal clicks, input and selection retain their owners.
- **Failures:** Revision, attachment, geometry, presentation, focus, occlusion,
  or scroll changes retire link actions. A captured release without Ctrl is
  consumed without activation. Opening accepts at most 4096 bytes of ASCII
  HTTP/HTTPS URI syntax with a host and without credentials. Copy accepts bounded
  target text without control characters. Unsupported or malformed targets,
  missing native handlers, and launch failures/timeouts have bounded visual and
  accessible notices. No URI is opened on hover or keyboard focus.
- **Owners:** Orbit owns URI attributes; Scene derives spans; Venus presentation
  identity gates actions; the existing renderer, input, AccessKit status and
  native clipboard owners project them. Linux `gio open` on the host PATH
  dispatches the exact URI as one argument to the desktop's registered handler.
  One launch runs at a time with a ten-second dispatcher deadline.
- **Consumes:** Accepted Orbit and `ORB-C6` proof
  `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, ORBF v2 / ORBS v11.
- **Boundary:** No heuristic detection, file/custom-scheme opening, URI rewrite,
  browser embedding, new crate, or protocol change. Host GIO and a registered
  handler are required for Linux opening; copy remains available without them.
  macOS uses fixed `/usr/bin/open -u` and the general pasteboard. Product-path
  accessibility activation remains unaccepted on macOS.
- **Proof:** Accepted source
  `3730201d6dd27e15fdcd1a8b0662aa476a587eb6`.
  - **User-accepted simplification:** Nix package
    `/nix/store/g8yhz7ha4drfkcraqg9bjyb34j6bgvv4-yazelix-venus-0.1.0`
    ran as the sole presentation client for a live Eon workspace on native
    COSMIC Wayland. The user confirmed the two-action hover hint, exact
    Ctrl+Shift+C copy, and Ctrl+left-click GIO open against
    `https://example.com/eon-link-test?source=venus-candidate#open-copy` while
    the Eon supervisor and nine Orbit processes remained live.
  - **Mechanical and isolated native evidence:** Focused checks prove selection
    precedence, exact AccessKit Open/Copy controls and stale-node retirement;
    the complete locked Rust suite and isolated native input-pairing regression
    pass. A private static EonTerm on native COSMIC Wayland exposed the exact
    Link and Copy button through AT-SPI with one Click action each. Copy produced
    the exact Wayland clipboard target; Open invoked exact `gio open -- URI`
    arguments through a disposable recorder. Real GIO opening was already
    accepted in user dogfood. AT-SPI was enabled before launch; activation after
    an already-published frame remains unqualified. Downstream installed Eon
    composition remains separate and unproved.
  - **Retired inspection correction:** The real Application regression fails before
    this revision and passes after it: unidentified native presses are captured,
    their repeats/releases stay captured after Escape, and fresh presses return
    to terminal routing. Locked checks (115 ordinary tests) and both native
    checks pass. Earlier hyperlink dogfood below retains its exact source.
  - **Installed correction:** Eon
    `07867f99d176012a9d34a61aa37b5de9ce0888af` selects this exact source;
    both profiles launch `/nix/store/fxf4xyadmiwn7m8ndc0i0gxzcbrq5r2p-yazelix-venus-0.1.0/bin/yazelix-venus`.
    Installed EonTerm on Sway 1.12/Nix Mesa 26.1.2 captured real native keycode
    240 carrying text during inspection, admitted it before/after inspection,
    and copied the exact link. Eon's contract index records both artifacts.
  - **Environment:** x86_64 Linux, private Sway 1.12, Vulkan Mesa lavapipe,
    host GIO 2.80.0, Wayland clipboard and AT-SPI.
  - **Checks:** Locked fmt/check/test/clippy; 115 ordinary tests and both native
    renderer/Application checks. The native Application check rejects unpainted
    and replaced link targets, preserves wide tails, refreshes hover after
    presentation and retires the attachment. A real stalled child proves the
    dispatcher deadline and reaping.
  - **Initial dogfood (`d1223463b2b513c04242df50a4bd948d0533cfde`):**
    Native keyboard traversal/paging, exact clipboard and real GIO
    dispatch of `https://example.com/exact?x=%26&y=2#part`, unsupported-scheme
    and malformed-percent refusal, mouse-reporting press/release preservation,
    explicit click capture, bottom-row hover, replacement during a held click,
    and native missing-handler failure. Target and failure text were read
    through AT-SPI.
  - **Integration:** Accepted Eon
    `be9d0e37c7d0029d6832ff609082faa885280762` installed source
    `d1223463b2b513c04242df50a4bd948d0533cfde` as
    `/nix/store/746dv1wk3pmmizz18d4v238p0s69qmf9-yazelix-venus-0.1.0`.
    Both Eon and EonTerm profiles matched their built artifacts; installed
    native checks passed on Sway 1.12/Nix Mesa 26.1.2 lavapipe. EonTerm repeated
    the target, mouse, failure and stale-click dogfood above; full Eon proved
    picker cancellation, target inspection, exact copy and native dispatch.
    Host-only XDG handler preferences were preserved. Eon's contract index and
    `ven-lq4` record the exact artifacts. Fractional scale and broader compositor
    quality remain unproved.
  - **Apple Silicon partial proof:** Exact Venus source
    `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` passed the deterministic
    shortcut and URI-dispatch checks. M1 run 34832511106 copied an exact hovered
    URI with Command+C. On a physical Macmini9,1 M1 running macOS 26.6.1 build
    25G76, vncdotool 1.4.2 hovered the exact OSC 8 target and product-path
    Command+click opened Safari at that exact URI. M1 mechanism run 34820510780
    separately proved `/usr/bin/open -u` dispatch.

The `VEN-C1`, `VEN-C2` and `VEN-C4` proof revisions advance for this exact
hyperlink slice. Their distinct earlier evidence and listed proof gaps remain
qualified by the identities and boundaries below.

## VEN-C1 — Authoritative native presentation

- **Status:** Proven on x86_64 Linux; broader native quality remains unproved.
- **Consumer:** One Venus native surface consuming canonical Orbit frames and an
  optional cursor-tail color choice.
- **Trigger:** Venus accepts an initial frame, an ordered later revision, or a
  cursor-tail startup value.
- **Result:**
  - Venus validates and materializes one coherent Orbit-authored structured
    frame into native draw inputs without terminal authority.
  - An omitted color choice selects one of eight Venus presets once per launch
    and uses it for that process at internal fallback duration `1.0`. Eon owns
    product defaults and user configuration. One selector accepts
    `random`, `preset:<name>`, or `custom:#RRGGBB`; `none` is static, and the
    duration flag can override the default for any color choice. Durations must
    be finite multipliers from `0.25` through `4.0`; the complete lower-level
    `tail` profile supplies one validated color and duration.
  - A bounded four-corner trail approaches Orbit's exact cursor destination.
    Healthy pane/tab switches and workspace layout changes retain window-local
    cosmetic motion toward the newly selected pane's fresh cursor. Attachment
    gaps publish no old terminal content or trail and schedule no animation;
    only the bounded cosmetic position survives. First presentation, hidden
    cursors, invalid geometry, transport failure, focus loss/occlusion, native
    resize/scale changes, and GPU recovery retain safe reset behavior.
    Its solid fill receives a one-logical-pixel tint or shade with at least 3:1
    fill-to-outline contrast while preserving authoritative cursor shape,
    visibility, blink, and wide-cell geometry. The cursor body matches that
    selected color and receives the same shape-aware contrasting edge when
    Orbit supplies no cursor color; an explicit Orbit cursor color takes
    precedence without that edge. With `none`, the static body uses Orbit's
    color or foreground fallback.
  - The immutable Scene alone owns accessible text and selection. Each visible,
    nonblank canonical head cell is one selectable UTF-8 unit; text above the
    AccessKit 255-byte unit limit becomes one U+FFFD accessibility unit without
    changing visual or protocol text. Without selected cells, the authoritative
    viewport cursor supplies a collapsed accessible caret in those same atomic
    units. Trimmed padding stops before the row newline; an absent viewport
    cursor supplies no caret.
- **Important failures:** Invalid frames, topology, cursor profiles, renderer
  admission, focus, occlusion, or presentation failure cannot publish stale draw
  or accessibility state.
- **Owner:** Venus Scene, renderer, and cursor-animation state; Orbit retains
  terminal and cursor authority.
- **Consumes:** Orbit `ORB-C4` and `ORB-C6` through canonical ORBF v2 / ORBS v13
  and `orbit-protocol` 0.1.0 at accepted source
  `6bc269c40b18f08b95778939518f77556ba91c67`; those contracts retain proof
  `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
- **Boundary:** Fractional/HiDPI native quality remains outside the proof.
- **Terminal caret proof:** Accepted source
  `58f2eaa8057627bdbf73262fdb10a1dc8b681ffc`
  (`ven-expose-authoritative-accessible-caret-zki`) supplies the existing Orbit
  viewport cursor through AccessKit's collapsed text selection. Focused
  red/green coverage, all 146 ordinary locked Rust tests, strict Clippy and the
  affected Nix package pass. Native selected-text inspection reports exact
  `é 界`; line review reaches live and history rows, and Return to live restores
  the real caret without inventing one in history. Native Orca at
  `24c64c15c1e1a75f95d74f7d623f62ab028fd3a9` receives caret movement and reports
  the picker caret at 27, but Say All retains its previous text context and
  reads only the footer. The same failure reproduces with a full prior text
  buffer in GNOME Terminal 3.60.0/VTE 0.84.0 and real fzf 0.73.1, without Eon
  or Venus. The earlier short-buffer GNOME comparison passed and does not
  establish that the defect is Venus-specific. Native logs, SSIP/PCM and the
  standalone reproduction are retained in
  `~/.local/state/eon/proofs/venus-picker-trace-2026-10-10-b3lmrz6w/`.
  On 2026-10-10 the user accepted this Venus-owned repair while retaining the
  [Orca #757 Say All limitation](https://gitlab.gnome.org/GNOME/orca/-/work_items/757).
  The separately tested Orca correction restores query/full-path speech on
  this exact Venus package; its evidence is retained in
  `~/.local/state/eon/proofs/orca-caret-fix-2026-10-10-ss8xua84/submission/validation.json`.
  No patched reader is installed or selected by Venus. Acceptance covers the
  caret, selection and history boundary; unmodified-reader Say All remains
  red. Full reader qualification remains open and Eon adoption is separate.
  The acceptance record and unchanged-source identity check are in
  `~/.local/state/eon/proofs/ven-caret-acceptance-2026-10-10-s9mhcd4x/acceptance.json`.
- **Terminal text-geometry proof:** Source
  `217f18a231d511f0b379ceee807769ad31c0a047` (`ven-restore-orca-terminal-line-review-gyu`)
  hardens VEN-C1/C2/C7 through the existing Scene and shared cell metrics.
  Accessible atomic units retain their canonical column spans; AccessKit text
  positions and widths use the clipped row origin and viewport. Text, selection,
  wide cells, trailing trim, zero-width newlines and oversized-cell degradation
  keep their existing owners and semantics. Focused red/green checks cover
  atomic/wide units and partial/full clipping at scales 1/1.25/1.5/2. Locked
  formatting, check, all 146 ordinary tests, strict Clippy and build pass.
  The native Application input regression also passes on a separate private
  D-Bus session. On private native Sway 1.12/headless/pixman, scale 1,
  host Mesa 26.1.6 lavapipe, Orca 49.4 and AT-SPI 2.58.1, real reader commands
  review all bounded corpus rows and ASCII markers in order, report exact
  `é 界` selection, and continue after history/AT return-to-live. Actual SSIP
  and synthesized PCM are retained in
  `~/.local/state/eon/proofs/ven-orca-line-review-gyu-2026-10-10/acceptance.json`.
  Reader handlers were invoked through Orca D-Bus. That geometry observation
  does not prove retired-control reader focus; its repair is indexed under
  VEN-C2. Native reader-key delivery, composed Eon qualification, other
  compositor/native fractional-scale quality and macOS remain unaccepted.
- **Proof:** `bac1d8d155c04840d672f2d1bb299a830dcb625f`
  - **Half-block rendering refinement:** Full locked Rust checks pass with 145
    ordinary tests, plus the native Application input regression on private
    Sway 1.12/headless/GLES2 and host Intel Vulkan. Exact cell rectangles bypass
    shaping for narrow upper/lower half blocks while retaining resolved styles
    and preview clipping. Deterministic checks cover scales 1/1.25/1.5/2;
    independent native pixel checks cover the named fixture at 1 and 1.25.
    Repeated same-toolchain release comparisons of 160×48 Anima plasma show
    4.77–5.08 → 22.62–23.38 visible changes/s, p95 gaps 233 → 50 ms, and Venus
    CPU 91 → 10–13% of one core. These are fixed-workload observations, not a
    general FPS or fractional-quality guarantee. Exact identities, recordings,
    profiles, checks and limitations are retained in
    `~/.local/state/eon/proofs/ven-anima-half-blocks-2026-10-08/report.txt`.
    Eon package/profile delivery and live COSMIC verification remain pending.
  - **Earlier recovery freshness and duration launch refinement at
    `a27aad23822d3bd0b0e467ffc32c578bbde8b00d`:** Full locked Rust
    checks pass (144 ordinary tests), along with the ignored native Application
    regression on private Sway 1.12/headless/pixman and host lavapipe at scale 1.
    Red/green checks cover a redraw between Attached and the first current frame,
    retained failure-frame layout changes, and pane handoff after failure. Those
    paths remain static until a fresh authoritative frame arrives. The existing
    launch parameter accepts duration-only and ordinary color-plus-duration
    overrides; Eon owns its product default and user configuration. Exact source
    and binary hashes and logs are in
    `~/.local/state/eon/proofs/ven-cursor-tail-review-a27aad2-2026-10-07/`.
  - **Installed consumer continuity:** Eon source
    `1ca418d9370061b4ba5ac03c0225236ad2b1461d` is qualified by Eon's EON-C4
    installed proof. Its Eon-owned `1.5` default shows keyboard/pointer pane/tab,
    layout and rapid-switch continuity with eventual settlement on private
    Sway 1.12/headless/pixman, Nix Mesa 26.1.2 lavapipe and scale 1. The source,
    artifacts, captures and configuration/reopen observations are in
    `~/.local/state/eon/proofs/eon-cursor-tail-duration-1ca418d-2026-10-07/report.json`.
  - **Earlier continuity source:** `3535f7b9bac28f54e567751abfad4b4ab164337b`.
  - **Workspace continuity refinement:** Focused red/green spring and native
    Application checks prove fresh-frame handoff, missing-frame suspension,
    rapid retargeting, and failure/reconnect resets. The complete locked Rust
    route passes with 144 tests; the ignored native Application regression also
    passes at duration `1.0`. Private Sway 1.12 headless/pixman, scale 1, and
    host lavapipe against the exact Orbit source above show continuous keyboard
    and pointer pane/tab navigation, layout movement, and rapid switching.
    All six captured motions settle without an opaque trail. Capture duration
    `4.0` makes motion observable; this is no timing or broader platform claim.
    The isolated Eon workspace overrides only its Venus executable and does not
    adopt this source into Eon's manifest or installed profiles. Source hashes,
    red/green logs, native captures, and environment limits are retained at
    `~/.local/state/eon/proofs/ven-cursor-tail-workspace-continuity-zln-2026-10-07/`.
  - **Fallback visibility refinement at
    `f6c1ebdb16df2960077dbd88ab6728a921b789d8`:** The selected fill has a
    shape-aware one-logical-pixel contrasting edge for block, bar, underline,
    and hollow stationary cursors. Focused red/green draw checks cover that
    edge, explicit Orbit color, blink, and moving trail; the full locked Rust
    route passes on x86_64 Linux. Private Sway 1.12 headless/pixman with exact
    ORBS v12 and host lavapipe showed `custom:#000000` invisible on black
    before this revision and visibly bounded afterward. Captures are
    `~/.local/state/eon/proofs/ven-phm-2026-09-19/review/before-black.png`
    (SHA-256 `8a4383a79496ca12dfbf68e179b15837f83d76a51ac3b1044fe666018028fd3d`)
    and `after-black.png` (SHA-256
    `8b8631443c372e1553453706b4fbfdac189e5c16f65797acd49bc7f3392b053f`).
    Static `none` and explicit Orbit cursor colors keep their prior draw paths.
  - **Cursor-body color refinement:** Focused red/green draw-byte and canonical
    frame checks prove that an absent Orbit cursor color uses the chosen trail
    RGB, an explicit color wins, and static rendering keeps its foreground
    fallback. The complete locked Rust route passes on x86_64 Linux. Private
    Sway 1.12 headless/pixman and host lavapipe rendered `preset:ice` against
    exact ORBS v12 Orbit; the native capture is
    `~/.local/state/eon/proofs/ven-phm-2026-09-19/candidate-ice.png` (SHA-256
    `c2f6753f51ad116385ef2280c595807223b82d29027d5e03db51c62e9e02e948`).
    Eon `a8540ff575e1ca6f3ab3825af7fd92d810e20ba6` pins the cursor-body source
    `f0885517cbd63f6fac966e913e48aecccfc212eb`, and
    its installed EonTerm proof is recorded at
    `ee2a8fda1146b1f4481a5214f478d6c03104f622`. The block cursor keeps its
    existing 0.55 alpha, so its displayed pixel is darker than the opaque
    trail despite sharing RGB. Existing VEN-C1 quality limits remain.
  - **Earlier cursor color slice:** Focused parsing, exact preset, deterministic random,
    custom RGB, conflict, contrast, inset, clipping, lifecycle, and draw-order
    checks pass in the complete locked Rust suite. On a private Sway 1.12
    headless Wayland output, `preset:ice` rendered `#7DDCFF` with outline
    `#467B8F`; custom black rendered outline `#5A5A5A`; an omitted choice kept
    random Nebula `#A970FF` with outline `#503579` across two moves; and explicit
    `none` rendered no colored tail. The separate Orbit cursor body remained
    authoritative.
  - **Startup slice:** Exact typography, geometry and failure evidence is
    indexed in VEN-C19 above; earlier distinct evidence below is retained.
  - **Environment:** x86_64 Linux Wayland, with retained deterministic and native
    evidence from the accepted presentation lineage
  - **Evidence:**
    - Static presentation `74ab5a0b661210f0afec94086f5358fe50b01f05`
      with native predecessor `534e47908b7bb7e8286949fe39bf4b0f7cc811f9`
    - Typography `846daf8fb7846b0e8dc227e533aa8d51a691f76f` and
      cell placement `99539163ae901413c56388ae8959c49c608dd46e`, retaining
      `457b3da837f07d186a99ca20930f2146c3d78f36`,
      `3e27bb7e377da9d9eb7e5cbdbfcfa5add3de0cad`, and
      `038cc2129d7cb4047350a37bb4aa6e4c3b93ccee`
    - Cursor tail `254beec194fe5cac208cced34d346014b0484319`, native
      dogfood `c618cfd087da7c64adf8cd093c3f4b458e8fd5d4`, and wide
      timing comparison `9a1658561a130f6d4656a3049a707feb7cecaa72`
    - Omitted-profile tail and explicit-`none` proof at
      `5d22b09e323212693a8e54c4c63089784b660cad`
    - Cell-buffer allocation proof at
      `541a9cb43c155b8b97069904593dc81c73682613`: the
      [memory comparison](benchmarks/venus-memory-2026-09-08.md) records
      repeated PSS/USS savings and identical baseline/candidate pixels on
      isolated Sway/software Vulkan. It preserves the earlier presentation
      proofs and does not extend COSMIC, fractional-scale or compositor coverage.
- **Open proof:** Fractional scale and broader Linux compositor coverage remain
  outside the proof.

## VEN-C2 — Native semantic interaction

- **Status:** Proved on x86_64 Linux; partially proved on Apple Silicon macOS.
- **Consumer:** One focused Venus terminal surface.
- **Trigger:** Native key, mouse, focus, paste, input-method, or resize activity.
- **Result:** Venus maps native activity to canonical Orbit semantic input and
  resize messages using one shared scaled geometry owner. Compatible output
  advances render content without withdrawing the last presented input geometry.
- **Important failures:** Invalid, stale, unsettled, or non-presented geometry
  cannot reach Orbit; capture and focus transitions withdraw obsolete input.
- **Bounded text-geometry proof:** See the scoped Venus source and reader
  observation under VEN-C1; broader reader qualification remains separate.
- **Terminal caret proof:** See the accepted source and scoped native caret,
  selection and history proof under VEN-C1, retaining the Orca #757 limitation.
- **Reader-focus cache repair:** Source
  `5dca0688644ef11f7fa8e845bb42842603189d32`
  (`ven-resolve-reader-focus-after-return-to-live-lmt`) pins AccessKit to
  `0f9d186ec6f5e66fe4ccc131ca318dc2725c12f5`. Its Linux cache signals retain
  each structure as one D-Bus argument; Venus adds no focus workaround.
  The dependency's encoded-wire regression fails before and passes after;
  all six adapter tests, strict Clippy, Venus's locked checks and 146 ordinary
  tests, the isolated native Application regression, and the Nix package with
  146 release tests pass. On private native Sway 1.12/headless/pixman, scale 1,
  Nix Mesa 26.1.2 lavapipe, Orca 49.4 and AT-SPI 2.58.1, the reader keeps its
  terminal script/locus through history/Say All and assistive Return to live.
  Native selection `é 界` at 206..210 is synthesized as selected text without
  a post-return F6/Escape repair. The retired control is absent and the current
  terminal is focused. Exact Nix artifact, SSIP, PCM/WAV and cleanup are in
  `~/.local/state/eon/proofs/ven-reader-focus-lmt-adoption-2026-10-10/acceptance.json`.
  This hardens VEN-C2/C7/C8 and preserves VEN-C1/C4; Eon adoption and full
  qualification, native reader-key delivery, other compositors, fractional
  native quality and macOS remain separate.
- **Owner:** Venus native input and geometry owners; Orbit encodes terminal
  behavior.
- **Consumes:** Orbit `ORB-C5` proof
  `c905bf9610581747f1b07565814b501ca66cfaa6` through canonical ORBS v4 at
  `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`.
- **Boundary:** Physical mixed-monitor hardware and broader Linux Wayland compositor
  behavior remain manual quality surfaces. On macOS, only VEN-C16's Kotoeri
  Roman-to-kanji IME path is accepted; held-key repeat, dead keys, and other IMEs
  remain unaccepted. Physical trackpad gesture quality remains with VEN-C7.
- **Proof:** `ec80e36625dec73544c0cb64becf4b135c932a63`
  - **Startup slice:** Exact typography, geometry and failure evidence is
    indexed in VEN-C19 above; earlier distinct evidence below is retained.
  - **Environment:** Accepted deterministic host coverage plus retained native
    geometry and input evidence
  - **Evidence:** Native Application input-between-frames regression;
    Sway focus/selection/scroll checks; retained geometry proof
    `c0766532c7669a2ae9dac6a26ae94d467110896b`. Shared geometry,
    resize-settlement, semantic mapping,
    preedit, focus cleanup, and fullscreen checks, retaining typography proof
    `846daf8fb7846b0e8dc227e533aa8d51a691f76f`, semantic-input proof
    `a17d100d11d38acf515cf96af34df6bbfef9850b`, and focused geometry proofs
    `f84493781956a1f5c24be3dab148bef02748cdfe`,
    `6df14c7e2ca8c27d587d6c138200f4d39d6b12a9`,
    `6f919a6e3d2f51b661f76be644a4354a003a3404`,
    `ba7177ca9380f2f5800bcea20da3f81e96b090cd`, and
    `8929c9f9d151641a343813ddeb6005cb9c771286`
  - **Apple Silicon partial proof:** M1 run 34832511106 at exact Venus source
    `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` delivered ordinary and Unicode
    key text plus Command+V as exact Orbit input. The selected deterministic
    checks cover shared IME, repeat, focus, pointer, and scroll ownership. On a
    physical Macmini9,1 M1 running macOS 26.6.1 build 25G76, the same source
    delivered `F`, suppressed `X` after Finder took focus, and delivered `R`
    after Venus reactivation. A native drag selected one exact 24-byte terminal
    row and Command+C wrote those exact bytes to the general pasteboard. A three-second
    VNC key-down delivered only one byte; Option-E then E delivered plain `e`.
    VEN-C16 later proves Kotoeri only; repeat, dead keys, and other IMEs remain open.

## VEN-C3 — Transient client recovery

- **Status:** Proved
- **Consumer:** Standalone or Eon-supervised Venus attached to one selected live
  Orbit endpoint.
- **Trigger:** Client close or crash, retryable connection loss, control-stream
  loss, or replacement attachment.
- **Result:**
  - Client loss never owns or terminates the Orbit Session.
  - Reopening materializes the coherent current state.
  - An open client retries the same live endpoint with bounded backoff while
    retaining the last coherent scene.
  - In supervised mode, control loss exits Venus and a replacement may retry
    transient Busy while the departing attachment releases.
  - Endpoint replacement, non-live workspace state, successful attachment, or
    exit cancels obsolete retry state.
- **Important failures:** Standalone Busy and incompatible, exited, malformed,
  or replaced endpoints remain terminal rather than creating duplicate clients.
- **Owner:** Venus connection/retry and native event-loop lifecycle; Orbit owns
  Session survival.
- **Consumes:** Orbit `ORB-C1`, `ORB-C3`, `ORB-C4`, and `ORB-C7` through canonical
  ORBS v4 at `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`.
- **Boundary:** Broader Linux compositor coverage remains unproved.
- **Proof:** `a768e9a1bcb61eac5a21d25b7463c9dc44aa2df8`
  - **Environment:** x86_64 Linux with deterministic host and composed Eon
    acceptance
  - **Evidence:** Supervised control EOF, managed-only Busy retry, exact current
    reattachment, and unrelated-Session survival; native predecessor
    `754eb057ea71ddee57b2f543d85d08d66edc953e`, prior loss proof
    `8e2d22a36ae6f6cab74d3556204a2a537265bff7`, exact Orbit
    `86aa130629c09dce61d0f232150298656fa5cef4`, and composed Eon acceptance
    `0bf0b165d06b4a8162be497011070f61f6c2000a`

## VEN-C4 — Bounded explicit failure UX

- **Status:** Proved
- **Consumer:** A Venus user observing connection, protocol, model, renderer, or
  Orbit failure.
- **Trigger:** Attachment rejection, incompatibility, invalid frame, Orbit loss,
  renderer failure, or retry transition.
- **Result:** Venus exposes bounded truthful failure notices without inventing
  state. Retryable loss remains visible during backoff. Routine connection and
  first-frame progress is quiet inside an accepted workspace; standalone
  attachment retains progress messages. Actual attachment and rendering failures
  remain visible in both modes.
- **Important failures:** Standalone Busy is terminal; only supervised
  replacement retries it. Incompatible, Exited, model, queue, invalid-input,
  worker-start, and persistent-renderer failures remain terminal and withdraw
  presented input.
- **Owner:** Venus connection state, notice projection, and renderer lifecycle.
- **Consumes:** Orbit `ORB-C3`, `ORB-C4`, and `ORB-C7` through canonical ORBS v11
  at `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
- **Boundary:** Compositor title-bar visibility and broader Linux compositor
  proof remain outside the accepted slice.
- **Workspace action failures:** VEN-C8 records the retained-notice and native
  reader announcement proof at `dd31c0e74628674c772b1494590ce31cacce1ef0`.
- **Quiet workspace proof:** `b7404dae6c59a8e6de6ca9efbf2c907eedf1262b`.
  Locked fmt/check/test/clippy/build pass (124 ordinary tests). The existing
  native Application input/presentation check goes red for routine connection
  progress, then passes with silent workspace Connecting and first-frame wait,
  visible offline failure and retained standalone progress on x86_64 Linux,
  private Sway 1.12 / host lavapipe at scale 1. Source and native evidence lives
  under `~/.local/state/eon/proofs/eon-quiet-workspace-navigation-y7a/venus/`.
  Consumes accepted Orbit `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`
  (ORBF v2 / ORBS v11). This does not widen VEN-C8 fractional-scale,
  compositor, screen-reader or earlier distinct failure evidence.
  Eon `07dc210a49cd30f3b58fa65fd9e9a45b421e9b36` accepts this source through
  both refreshed products. Its contract index owns installed identities,
  navigation observations and the retained intermittent headless-capture limits.
- **Proof:** `ec80e36625dec73544c0cb64becf4b135c932a63`
  - **Startup slice:** Exact typography, geometry and failure evidence is
    indexed in VEN-C19 above; earlier distinct evidence below is retained.
  - **Environment:** x86_64 Linux deterministic host coverage with retained
    native title and supervised-loss evidence
  - **Evidence:** First-renderer-error retention, best-effort diagnostics,
    presented-input withdrawal, terminal-versus-retryable classification, and
    later recovery; native title proof `4c95652b2fe1af28df5964b5686daf44676a171e`,
    exact winit `fb45fbf901fbe70cc9a877b5d651d0b60c206b08`,
    supervised proof `a768e9a1bcb61eac5a21d25b7463c9dc44aa2df8`, native
    predecessor `754eb057ea71ddee57b2f543d85d08d66edc953e`, prior loss
    proof `8e2d22a36ae6f6cab74d3556204a2a537265bff7`, and composed Eon
    acceptance `0bf0b165d06b4a8162be497011070f61f6c2000a`

## VEN-C7 — Authoritative history and selection interaction

- **Status:** Proved on x86_64 Linux; partially proved on Apple Silicon macOS.
- **Consumer:** One presented Venus terminal surface.
- **Trigger:** Native wheel or touchpad movement, one left-pointer sequence
  including a held drag in the top visible row, explicit copy, or activation of
  the visible scrollback pill.
- **Result:**
  - While away from live output, Venus displays `↓ N rows` (`↓ 1 row` for one)
    from the accepted frame's authoritative wrapped display-row distance. Live
    bottom, alternate screen, recovery and known terminal-owned routing hide it.
    Activating the whole visible pill requests one semantic return to live;
    Orbit alone moves the viewport and publishes its new position. An
    unpresented, hidden, or occluded pill is inert.
    Idle preview cleanup and rejected input preserve routing evidence until a
    fresh frame or preview replaces it.
    Preview movement never changes the count; accessibility exposes committed
    position as a description and the visible return action as a button, not an
    alert. The selected pane header
    reserves space for the count and fits its directory label into the remaining
    width using the existing cluster-safe header fitter. Standalone and picker
    terminals use a small top-right overlay that preserves grid dimensions and yields to selection,
    links, notices, picker tab previews and an overlapping terminal cursor.
    Pending reflow hides the old count;
    labels that cannot fit in full remain accessible without clipped digits.
  - Precision movement translates and clips an accepted complete Orbit frame
    plus its bounded revision-bound row window at two presented pixels per
    native input pixel.
  - Venus retains a sub-row remainder, commits crossed rows through at most one
    coalesced signed Orbit batch, and installs only its atomic authoritative
    frame and next preview.
  - Lagging relative preview and commit revisions remain admissible while PTY
    output advances; after history entry, later output leaves the authoritative
    viewport pinned and further relative movement remains available.
  - When a later output frame preserves screen, geometry, default cell colors,
    and palette while fractional movement remains, Venus keeps only that
    bounded revision-bound preview until Orbit supplies its replacement instead
    of presenting the remainder as zero or freezing finger movement. Every
    plain frame retires transient terminal-routing evidence.
  - A phase-complete precision gesture may continue with bounded elapsed-time
    exponential decay; discrete wheel input moves three rows per logical step
    without synthetic momentum.
  - A transient presentation timeout preserves gesture estimation, kinetic
    velocity, and distance; an occluded surface still cancels them.
  - Ordinary output preserving terminal dimensions and active screen keeps
    wheel start/movement/end and pointer phases admissible between repaints.
    Input carries an actually presented revision; render caching still advances
    on each accepted frame. Orbit keeps parsing and owns anchored history.
  - Redraw follows native compositor callbacks.
  - Venus sends every left-pointer phase to Orbit. Orbit routes uncaptured
    input to cell selection, words at two clicks, space/tab-delimited spans at
    three clicks, and logical lines at four or later clicks. Orbit preserves
    native repeat timing, distance and drag behavior, terminal mouse capture,
    and Shift as a host-selection override.
  - While a held host selection stays in the top visible row, Venus sends one
    upward selection tick at a time. Orbit moves at most one history row per
    tick and extends the same gesture; Venus waits for the accepted frame
    before the next tick and stops at the oldest edge or on release.
  - Pointer phases and explicit copy that arrive before Orbit completes the
    preceding sequence retain their order. Venus resumes them only after
    presenting the authoritative completion revision reported by Orbit.
  - A successful selection release writes Orbit's frozen text to both the
    ordinary Wayland clipboard and primary selection. `Ctrl+Shift+C` remains an
    explicit ordinary-clipboard copy. On macOS, every destination maps to the
    general pasteboard.
- **Important failures:** Future authority, stale coordinate- or phase-bound
  input, failed gesture admission, resize, capture loss, lifecycle change,
  terminal-owned routing, history edge, or Orbit rejection cancels synthetic
  motion and cannot fabricate cells, selection, copied text, or viewport state.
  Pointer/focus loss, stale presentation, attachment change, and reflow stop
  selection ticks; invalid synthetic ticks keep the native release without a
  routine error notice. Terminal and protocol failures remain visible.
  A change to screen, geometry, default cell colors, or palette retires the
  bounded preview before presentation. An unsolicited or mismatched scroll
  outcome is a protocol-order failure.
- **Bounded text-geometry proof:** See the scoped Venus source and reader
  observation under VEN-C1; broader reader qualification remains separate.
- **Reader-focus cache repair:** See the exact source and native selection
  observation under VEN-C2.
- **Owner:** Venus owns native fractional presentation, bounded kinetic state,
  gesture cancellation, and clipboard effects; Orbit alone owns history,
  viewport movement, routing, cells, revisions, selection, and copied text.
- **Consumes:** Orbit `ORB-C4`, `ORB-C5`, `ORB-C6`, `ORB-C8`, and `ORB-C9`
  through canonical ORBF v2 / ORBS v13 at
  `6bc269c40b18f08b95778939518f77556ba91c67`; exact patched winit
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`, wgpu 30.0.0, and glyphon
  0.12.0.
- **Boundary:** Client-owned history caches, bounce, device/source heuristics,
  public physics tuning, presentation feedback, unreleased winit, GPU-layer
  translation, and additional platform support are outside this contract.
  Apple Silicon physical trackpad gesture quality remains unaccepted.
- **Pending-request ownership proof:**
  `c7a0d07612e710854275a31500aaec651cc50a6f` preserves this contract with one
  Application-owned pending scroll commit or selection tick. Queue admission
  installs its origin; cancellation retains correlation until an ordered
  response or attachment retirement. Preview state remains separate. Locked
  fmt/check/test/Clippy and release build pass (144 ordinary tests). Existing
  deterministic checks cover ordinary batching/cancellation, the two `-1`
  origins, selection cadence, and wheel preservation after a rejected tick.
  The native Application regression passes on x86_64 Linux, private Sway 1.12
  headless/pixman at scale 1 and host lavapipe Vulkan. Queue checks pause the
  writer while full and cover closed admission; both origins are rejected
  without pending state. Encoded ReturnToLive waits for a
  tick result or ordinary rejection and remains absent after local encoding
  failure. All seven ambient product identities survive; private proof
  processes are reaped. Exact candidate hashes, recipe and native logs are
  retained in the `review-2026-10-06/` subdirectory of
  `~/.local/state/eon/proofs/ven-pending-viewport-request-owner-9gt-2026-10-06/`.
  Physics, native scheduling, canonical authority, wire bytes and pins remain
  unchanged. This source proof uses a canonical test producer with private
  accessibility disabled; installed Eon, macOS, screen-reader and physical
  trackpad acceptance remain separate.
- **Four-click consumer proof:** `c02e371c44909aa64cb3f2d77284b8a8ab2b4cc5`
  consumes accepted Orbit `6bc269c40b18f08b95778939518f77556ba91c67` with
  unchanged protocol package tree `d1151ce365f371cd06498c8d65fbcf6831ee0afe`
  and no Venus Rust change. Locked fmt/check/test/clippy/release pass (144
  ordinary tests). On private Sway 1.12, native scale 1 and host lavapipe,
  complete native pairs select word/span/line at 2/3/4/5; fourth/fifth clicks
  select across a 16-column soft wrap. Sentinel replacements prove fresh copy
  to both clipboard destinations. Single-click reset, span drag, compatible
  busy output and frozen explicit copy after row replacement pass. A held
  triple-span drag copies exactly 1,195 UTF-8 bytes across 92 contiguous
  `LINE-059-界` through `LINE-150-界` history lines. Native terminal mouse
  press/release and Shift override pass, as does the existing native Application
  regression with accessibility disabled. All five pre-existing product process
  identities survive. Recipes, identities, captures, bytes, logs and harness
  corrections are retained under
  `~/.local/state/eon/proofs/ven-adopt-orbit-click-selection-ladder-23e-2026-10-06/`.
  This is source consumer proof; installed Eon, macOS and AT remain separate.
  Orbit retains copy trimming and unwritten tab-gap behavior;
  the inherited libghostty wide-spacer limit can truncate a span:
  `a;界,é` selects `a;界`.
- **Upward selection source proof:** `773c6bab7d3e21e0b0d7d942c9bba72260a08b42`
  corrects the native response-order fixture from
  `59c94f6e48891b9717697a76b0e9381ccbe7a963` and
  consumes accepted Orbit `b6cecf8f2ee35570b41cfdc578b095889d917fe2`
  (ORBF v2 / ORBS v13). Locked Rust fmt, check, tests (80 library, 46 binary,
  17 integration), clippy, host release build, and Nix package build with
  locked tests pass on x86_64 Linux. The focused native Application check first
  rejected an impossible empty, non-edge preview, then passed with a canonical
  ORBS v13 response while retaining the ReturnToLive ordering assertions.
  With exact Orbit and Nix-packaged Venus on private Sway 1.12, a held drag
  crossed more than two screens of real history; release copied exactly 949
  Unicode bytes spanning contiguous lines `LINE-082-界` through
  `LINE-154-界`. Terminal mouse mode preserved SGR press and release without
  an error notice. Recipes, captures, copied bytes, and Nix build logs are
  kept under `~/.local/state/eon/proofs/ven-scroll-fixture-2026-09-24/`.
  This is Venus source proof; installed Eon and macOS remain separate.
- **Return-to-live consumer proof:** `ead00dfcb3510eb595173c02f4627d2cd8b36b0a`
  consumes accepted Orbit `ORB-C8` at
  `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` (ORBF v2 / ORBS v12).
  Locked fmt/check/test/clippy/build pass (140 ordinary tests), including
  canonical label and routing checks, painted-bounds and AccessKit Click
  checks. In private Sway 1.12 / host lavapipe on x86_64 Linux, the native
  renderer and Application checks pass; the latter observes exactly one
  encoded `ReturnToLive` for a complete click, none after a wheel, and no hit
  before presentation. Prior real Orbit/Venus dogfood showed `↓ 10 rows` above
  rows 57–90; clicking the pill restores live rows 66–99 and hides it, without
  PTY input.
  Captures and bounded process logs are retained under
  `~/.local/state/eon/proofs/ven-scrollback-return-to-live-2026-09-19/attempt3/`.
  This source proof does not accept installed Eon, macOS, or screen-reader
  behavior; paired installed delivery remains Eon's responsibility.
- **Scrollback review proof:** `74d22276e9039dc3ee6151d69b9283114ab02ea1`
  - Idle cleanup and rejected input preserve terminal-routing evidence; the
    selected pane label keeps its directory ending beside the count. Existing
    canonical-model and native shaped-header checks reproduce both regressions
    before the corrections and pass afterward. Locked Rust checks pass (124
    ordinary tests), as do the native header and continuous-output checks.
  - Private Sway 1.12, scale 1 and host lavapipe repeat the real-Orbit scenarios
    below. Terminal-owned wheel input hides the count at an unchanged distance
    of 15, and a fresh frame restores it. The narrowed workspace preserves the
    long path's `/leaf` ending beside counts 10 / 20 / 10 across pane switches.
    The reused workspace projection preserves geometry and input admission.
    Source/lock and binary hashes, logs, captures and process-preservation
    records are retained in
    `~/.local/state/eon/proofs/ven-scrollback-position-ddo-review-2026-09-09/`.
    All 19 pre-existing product process identities survive; owned processes
    are reaped. The source-consumer, accessibility and platform limits below
    remain unchanged; this does not prove installed Eon delivery.
- **Initial scrollback indicator proof:** `2bbb7029a84de783357ef2db0a68a145086b37b2`
  - **Environment:** Rust 1.96.0, x86_64 Linux, private Sway 1.12 at native scale
    1 and host lavapipe Vulkan; real exact Orbit sessions with a canonical EONW
    v4 workspace fixture.
  - **Evidence:** Locked fmt/check/test/clippy/build pass, including 124 ordinary
    tests. The canonical codec/model regression covers exact distance, singular
    wording, the full `u64` range, preview, routing, alternate screen and recovery.
    The native renderer check covers header spacing, complete counts, clipping
    and cursor collision; the existing native continuous-output Application
    regression passes. Real native counts track output (10 to 15), reflow (63),
    history top (177), pruning (36,135), live bottom and clear (zero), reconnect,
    and pane switches (10 / 20 / 10). Selection and link previews hide the
    standalone overlay. Source/lock hashes, scripts, captures, observations and
    process-preservation records are retained under
    `~/.local/state/eon/proofs/ven-scrollback-position-ddo-2026-09-09/` and
    indexed by `ven-scrollback-position-ddo`.
  - **Limits:** This is source consumer acceptance. Paired installed Eon delivery
    belongs to `eon-accept-scrollback-position-j7f`; the workspace topology here
    is a fixture. AccessKit is tree-verified, not screen-reader dogfood. Native
    fractional-scale and broader-compositor qualifications remain unchanged;
    this proof makes no new latency or kinetic-performance claim.
- **Retained gesture proof:** `e13970e90289d0d86f0adcbf350e4b9c1d5e5219`
  - **Environment:** Locked deterministic checks and optimized x86_64 Linux
    native Wayland dogfood
  - **Evidence:** Complete Rust checks and Nix build; native Application red/green
    admission regression; standalone cell/word/line drag and repeat-click
    gestures, both clipboard destinations and explicit copy during output;
    anchored history with continuing terminal replies under appends, active
    redraws, and DEC 2026. Native artifacts and exact observations are recorded
    in `ven-select-during-live-output-752`. Retained scroll proof
    `d3e63ae6e9fa0b72df426dbfc9bd29a96148577a` covers the exact ORBS v10
    pin; bounded multi-row
    conversion, clipping, lagging commit, compatible viewport retention,
    terminal-routing and incompatible-frame retirement, exact in-flight outcome
    admission, edge, cancellation, routed cell/word/line gestures, terminal
    capture with Shift override, separate primary and ordinary clipboard
    effects, resize, reattachment, and client-loss coverage. Bead comment 711
    records user acceptance of optimized native continuous-output hard flings on
    `d3e63ae6e9fa0b72df426dbfc9bd29a96148577a`; that revision preserves
    transient timeout motion while cancelling an occluded surface explicitly.
    Normal Eon manifest/profile adoption is accepted at Eon
    `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f` in
    `eon-accept-live-output-input-nxh`: both installed products use this exact
    Venus source and Orbit `91999d79546422b49bdbc124166a65859d0bd872`.
    Isolated Sway 1.12 proof covers anchored history, cell/word/line selection,
    both clipboard destinations, explicit frozen copy, and busy-workspace
    focus under appends, DEC 2026 batches, and active-screen redraws. The
    separate Eon Input Dogfood launcher remains prior evidence.
- **Apple Silicon state:** Exact deterministic checks preserve the shared
  pointer, scroll, selection, and destination owners at source
  `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee`. M1 run 34832511106 posted
  process-targeted pointer motion and precise-scroll events while the process
  and attachment stayed live, but its synthetic button events produced no
  gesture result. A later physical Macmini9,1 M1 run through Scaleway ARD/VNC
  delivered a native left-button drag that selected exactly
  `SELECT-THIS-TEXT-EXACTLY`; Command+C wrote its exact 24 bytes to the general
  pasteboard at SHA-256
  `34d2a7bacbb20cc60132ac65035667da81034ab0e52fd62743a795efec689c10`.
  ARD/VNC does not expose Apple trackpad touch or momentum phases, so trackpad
  gesture quality remains open.

## VEN-C8 — Eon workspace presentation

- **Status:** Partially proved
- **Reader-focus cache repair:** See the exact source and native workspace
  observation under VEN-C2; broader qualification remains separate.
- **Consumer:** One Venus surface controlled by an Eon workspace.
- **Trigger:** Outside visible `VEN-C18` popup presentation, Eon supplies
  durable topology or a workspace action changes tab, pane, focus, order,
  lifetime, liveness, endpoint, or metadata.
- **Result:**
  - Venus materializes ordered horizontal tabs and every fitting header for the
    active tab around exactly one expanded pane. Each tab shows its current
    one-based position plus the directory leaf or `/` derived from Eon's
    authoritative launch directory; positions update when Eon reorders tabs.
  - Tabs have pill-shaped corners and separated hit targets. Width follows shaped
    label text plus padding, capped near 280 logical pixels at default
    typography. Long labels use a cluster-safe middle ellipsis; when even that
    cannot fit, preserve the positional number whenever it fits. Overflow stays
    horizontally scrollable across the whole strip, including its gaps, and
    selection reveals the active tab. Hover shows
    its control-sanitized launch path within available window space; the
    accessible name retains that path. Selection and keyboard focus remain
    distinct: selected fill and brighter text identify the active tab without
    an underline; keyboard focus adds a rounded outline. Pane shell navigation
    does not rename or resize tabs.
  - Every visible live pane endpoint has one bounded read-only metadata observer.
  - Pane frames connect the visible accordion within one thin rounded outer
    border, with horizontal separators reaching its sides and a small gap below
    the stack (4 logical pixels at default typography). The gap shares terminal
    background color/opacity and the surface-wide blur request. Selected and
    hovered header fills follow the stack's corners, with square internal edges.
    Separators follow the rounded corners during scrolling;
    the shared border stays fixed while overflowing headers scroll within it.
    `--pane-frames true|false` defaults to `true`; `false` removes the shared
    border and separators while retaining headers, a lighter selected fill,
    hover feedback and a distinct rounded keyboard-focus outline. Both modes
    preserve the same terminal grid, header hit targets and AccessKit bounds.
    Initial requested grids include the bottom gap in their window overhead;
    the gap accepts no pane or terminal input.
    Changed pane lists resize the terminal even when the endpoint stays selected.
    The option has no visual effect in standalone or popup presentation.
    Missing, duplicate or invalid values fail before window creation. Eon owns
    persistent configuration and composed-launch policy.
  - Headers and AccessKit names show the opaque `pN` identity, two ASCII spaces,
    and one compact working-directory label.
  - Home displays `~`; descendants use `~/`; paths outside
    home remain absolute; overlong paths elide from the left to preserve the
    leaf; an unset or empty `HOME` keeps absolute paths.
  - Terminal title remains available to the selected window and is not repeated
    in pane chrome. Empty, offline, unavailable, or incompatible metadata falls
    back to pane identity without exposing mapped Session identity.
  - Hidden tabs have no observers, obsolete observations retire, and a 250 ms
    reinspection exposes accepted external workspace changes without native
    input.
  - Pointer input, Alt+H/L tab traversal, Alt+K/J pane traversal, Alt+M pane
    creation, Alt+Shift+T tab requests, focused arrow traversal, and
    selected-terminal attachment retain their existing owners. Alt+H/L remains
    admitted while a popup is visible; inactive popup selections do not replace
    the active tab's projection. Popup-only tabs may have no pane or selected
    pane. Hiding their last visible popup yields an empty body with actionable
    tabs and catalog shortcuts (VEN-C18).
  - Physical Alt+1 through Alt+9 focus the corresponding current tab position;
    Alt+0 focuses position 10. Venus resolves the position from the accepted
    snapshot and sends the existing stable `tN` through `FocusId`. A missing
    position is consumed without an action or terminal input. Alt+H/L remains
    the traversal path for tabs after position 10.
  - Ctrl+Alt+H/L sends one non-repeating semantic move for the active tab;
    Ctrl+Alt+K/J does the same for its selected pane. Alt+Shift+W sends one
    non-repeating close naming the snapshot's active stable `tN`. Venus consumes
    each exact press and matching release without forwarding either to the
    selected terminal. Ctrl+T, Ctrl+Shift+W, and lowercase Ctrl+W remain terminal
    input, and none of these shortcuts enters a mode.
  - Compatible terminal output does not withdraw tab/pane focus actions or
    presented header hit targets while a repaint is pending.
  - Hit testing, AccessKit node identity, and actions retain the exact `tN`.
    AccessKit names pair the current position with a bounded full launch path,
    so duplicate leaves remain distinguishable without storing another tab
    name.
- **Important failures:** Workspace loss, Orbit exit, endpoint replacement,
  liveness change, rejected close or movement, or incompatible metadata retires
  stale observations and never grants Venus topology or Session-lifecycle
  ownership. Repeats create no second structural action. Rejected-action notices
  remain available through unchanged and quota-only refreshes until workspace
  structure changes. Connection-unavailable notices clear on a successful
  snapshot. Terminal-status alerts announce their actual error without moving
  focus; routine polling does not repeat the announcement.
- **Owner:** Venus workspace materialization, bounded metadata observation,
  clipping, and accessibility projection; Eon owns topology and Orbit owns
  terminal metadata and Session lifetime.
- **Consumes:** Eon `EON-C10`, `EON-C17`, `EON-C18`, EONW v5, and
  `eon-workspace-protocol` 0.1.0 at
  `0cc8f477298681ae3945903e8fdb5852d487c5ab`; Orbit metadata observation in
  ORBS v11 and `orbit-protocol` 0.1.0 at
  `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
- **Boundary:** Other Linux Wayland compositors remain outside the current
  proof.
- **Codec provider relocation:** Source
  `bbf4cf41b289f441683eb7a3f225a26a4d3edacc` selects Eon Runtime
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`. Complete codec tree
  `8409c419a3496d6f01eeef65295f5d95b943313e` equals the former Eon
  `f41a41c9aecc4c436edfa2f394b832aa6d8711ad` dependency, retaining package
  `0.1.0`, Rust 1.95, Apache-2.0 and v2–v7 bytes/exports. Locked format, check,
  all 144 ordinary tests, strict Clippy and release build pass on x86_64 Linux
  / Rust 1.96.0. All other resolved package identities/features and the native
  source, tests and assets are unchanged. `eon-runtime-venus-rebind-tbmn`
  retains exact metadata, logs and artifact under
  `~/.local/state/eon/proofs/eon-runtime-venus-rebind-tbmn-2026-10-05/`.
  This verifies source relocation, preserving VEN-C18/C20/C21/C22/C23 and
  historical native proofs without a new native/macOS claim or Eon activation.
  Runtime-only changes with unchanged complete codec/build identity retain
  Venus's exact codec pin; that pin's reuse alone requires no new native proof.
- **Workspace failure-notice proof:** Source
  `dd31c0e74628674c772b1494590ce31cacce1ef0`
  (`ven-retain-workspace-failure-notices-5sl`) passes focused red/green checks,
  locked fmt/check/test/strict Clippy/build and the affected Nix package build
  (146 ordinary tests, five existing ignored). Two existing tests are
  strengthened; no maintained test is added. The exact package is
  `/nix/store/ilrx1wws293b4zx5r6x4csikjabmn0g6-yazelix-venus-0.1.0`.
  On x86_64 Linux native Wayland, private COSMIC/Sway and Mesa 26.1.2 lavapipe,
  normal Orca at upstream `24c64c15c1e1a75f95d74f7d623f62ab028fd3a9`
  announces an actual missing-directory pane-launch error once through SSIP
  with non-silent captured PCM. The notice remains available for 12.90 seconds
  across unchanged polls without repeated speech or terminal focus loss.
  Restoring the directory and retrying natively creates exactly the next pane
  and retires the notice; healthy terminal reading/input and both original
  Orbit peers survive. Cleanup preserves all four ambient process identities,
  the active profile and user focus. Exact dependencies remain AccessKit
  `0f9d186ec6f5e66fe4ccc131ca318dc2725c12f5`, Orbit
  `6bc269c40b18f08b95778939518f77556ba91c67` (ORBF 2 / ORBS 13) and EONW 7
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`.
  Evidence is retained in
  `~/.local/state/eon/proofs/ven-workspace-failure-notice-2026-10-10-3eeeed8v/acceptance.json`
  (SHA-256 `5fcc2d9460c4845ae8b68a9aec7b33ad9cf39018089ea91477646db59bb4fded`).
  This is Venus package proof through a private Eon launcher; installed Eon
  adoption and full reader qualification remain separate. The native route
  uses the previously verified pane-focus-to-picker transition. An initial
  terminal-focus transition after Say All read only the picker footer; that
  transition was not accepted by this proof. The subsequent caret repair and
  upstream Say All limitation are indexed under VEN-C1. No human listening,
  macOS or broader compositor claim.
- **Proof:** `0791f00926cd5cc4fedcc7737aeac7bb68569aff` for tab shortcuts;
  `94b15af20d1798b648f4d9945fd6bb647f10add8` for one-line header layout;
  `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` for workspace interaction
  - Ordinary tab basenames and text home pane paths are accepted at source
    `42a5ad291995847d5fa36bb62344065f95f6866f`
    (`eon-decide-tab-house-icon-lff4`). Focused red/green checks prove home
    basename, pane `~`, and root normalization including `///`, retaining full
    launch-path context, bounded metadata and existing fallbacks. Locked format,
    check, all 145 ordinary tests (five ignored) and strict Clippy pass on
    x86_64 Linux / Rust 1.96.0. This proves scene-label projection; Eon's
    delivery owns installed native acceptance. Evidence is retained in
    `~/.local/state/eon/proofs/eon-home-labels-option-1-2026-10-09/`.
    Other compositor, fractional-scale, screen-reader and macOS limits remain.
  - Positional labels and direct Alt-digit focus are accepted at source
    `f4845f1d0dd5c31301fd0c64b28fa8a907d1eae0`
    (`ven-positional-tabs-alt-digit-b3j`). The focused red/green checks and the
    complete locked Rust route pass with 127 ordinary tests. This source proof
    covers deterministic scene, shortcut, tooltip and AccessKit projection; it
    does not claim native input, screen-reader, installed Eon or macOS proof.
  - EONW v5 shared-popup and empty-body projection is accepted at
    `f7fb5a071edc446d04e629837b6e277f28d93709`; VEN-C18 indexes its exact
    Rust/Nix and isolated native checks. This consumer proof does not widen
    fractional-scale or full-Eon acceptance.
  - Scrollback count reservation preserves the selected pane directory ending
    at source `74d22276e9039dc3ee6151d69b9283114ab02ea1`. The focused shaped-header
    red/green check and private native pane-switch captures are indexed in the
    VEN-C7 review proof. Fractional scale and the other limits remain qualified.
  - Small painted bottom spacing and stack-shaped header fills are accepted
    at source `bc5a2bd3b2363abdea69c4cd89953b612bc970e8`. The focused margin
    regression fails on the preceding source and passes on this candidate;
    all 123 ordinary Rust tests and strict clippy pass. Private Sway 1.12 /
    Vulkan lavapipe scale-1 compares the rejected `ff426a9` baseline and this
    source over a colored underlay in both frame modes. Pixels prove a 4px gap
    matching terminal background color/opacity, top outer corners and square
    internal selected/hovered edges, and full-width dividers. Pointer/keyboard
    input, focus, overflow/reveal, picker and actual PTY grids pass. All 33
    pre-existing runtime identities survive; private process cleanup is empty.
    Evidence:
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-corrections-2026-09-09/REPORT.md`.
    Installed Eon/EonTerm generation `g1-5626ef23db3adb92c987e2c6179a793c` passes
    default/off pixels, native interaction and real initial/reopened 100x30 PTYs.
    Reopened workspaces retain the same supervisor, Orbit Sessions and command;
    standalone retains its grid without workspace chrome. Both profile elements
    and every observed Venus launch match the new build. All 33 pre-existing
    runtime identities survive installed proof; private process cleanup is empty.
    Sway does not prove compositor blur; pinned winit's surface-wide blur request
    is unchanged. The colored underlay exposes the alpha-zero hole hidden by the
    earlier black proof background. Fractional native scale, other compositors,
    screen-reader usage and offline lifecycle remain qualified.
  - Earlier polish proof at source
    `ff426a9f3d6e1acbb7ae1ce2388df7c75f7bb271` covered full-width separators,
    bottom spacing and lighter selected headers. Its margin and highlight
    design is superseded by the correction above.
    Locked Rust checks pass (123 tests), with a red/green margin/hit regression.
    Private Sway 1.12 / Vulkan lavapipe scale-1 true/false proof covers selection,
    hover, native focus/input, long labels, narrow overflow, picker and PTY grids.
    Pixel checks prove side-to-side separators, selected fill in both modes and
    the 12-logical-pixel bottom gap at default typography. All 30 existing runtime
    identities survive; private processes stop cleanly. Inputs, captures and
    limits are in
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-polish-2026-09-09/REPORT.md`.
    Installed Eon/EonTerm generation `g1-23256a5c404a78d4a68164edae640415`
    passes default/off visual and native interaction checks. Reopened 100x30
    workspace PTYs preserve their supervisor, Orbit Sessions and command;
    standalone remains 100x30 without workspace chrome. All 28 pre-existing
    runtime identities survive installed proof. Fractional native scale, other
    compositors, screen-reader usage and offline lifecycle remain qualified.
  - Connected pane stacks are accepted at source
    `8389cc011adbbf9c390a912d6c93b0040e47e21d`
    (`ven-connected-pane-stack-0ub`). Locked Rust checks (123 ordinary tests)
    and the native header regression pass. Private Sway 1.12 / Vulkan lavapipe
    at scale 1 proves top/middle/bottom selection, native focus/input, long
    labels, overflow, picker and unchanged PTY grids. Five frame-off captures
    and the single-pane frame-on capture match the prior renderer exactly;
    multi-pane captures show a continuous side border and two inset separators.
    Exact inputs, captures and limits are retained in
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-2026-09-09/REPORT.md`.
    Installed Eon and EonTerm delivery passes with generation
    `g1-216b179a560fe9af5c70a15dce222bf1`: exact profiles, default/off,
    connected border pixels, focus/input, overflow, picker and real PTY grids.
    All 25 existing runtime processes survive; private processes are reaped.
    Fractional native scale, other compositors and actual screen-reader use
    remain qualified.
  - Pane-frame review is accepted at source
    `5c23b0fe206fb2ed9e027603ae61ba366913cbe1`. Selected offline labels stay
    distinct with frames off; rounded fills and outlines share edge coverage.
    Locked Rust checks (123 ordinary tests) and the native renderer regression
    pass. Eight scale-1 Sway captures match the prior live-pane appearance
    pixel-for-pixel in both modes. Offline selection has native renderer-fixture
    red/green proof; it is not an installed Eon offline-lifecycle claim. Evidence:
    `~/.local/state/eon/proofs/ven-pane-stack-chrome-suu-review-2026-09-09/REPORT.md`.
  - Rounded pane frames and their startup option are accepted at source
    `bc3a01e0cae503429ab513e81fe2c95ea516b949` (`ven-pane-stack-chrome-suu`).
    Checks pass: locked Rust (123 ordinary tests), native header/input and Sway
    scale-1 default/on/off, focus, overflow and picker observations. Rapid changes
    resize retained PTYs correctly. Exact hashes and qualifications are in
    `~/.local/state/eon/proofs/ven-pane-stack-chrome-suu-2026-09-09/REPORT.md`.
    Initial Eon delivery is recorded in
    `~/.local/state/eon/proofs/eon-pane-stack-delivery-jfo-2026-09-09/REPORT.md`.
    Source acceptance covers the recorded native boundary; earlier proofs do not
    cover this change or widen its scale/compositor/accessibility evidence.
  - Pill-shaped corners without a selection underline are accepted at source
    `8211e8f773140429398154ca3f2230bd09fed1df`
    (`ven-pill-workspace-tabs-rg4`). Locked Rust checks pass, including 121
    ordinary tests. Private Sway 1.12/Vulkan at scale 1 shows selected/idle tabs,
    rounded focus, hover paths, overflow scrolling and narrow clipping.
    Exact source/binary hashes, captures and cleanup are retained in
    `~/.local/state/eon/proofs/ven-pill-workspace-tabs-rg4/REPORT.md`.
    This substituted debug-child check does not update installed Eon or extend
    fractional-scale, compositor or accessibility proof. Prior rounded-tab
    evidence remains scoped to its source below.
    Installed Eon acceptance at `57fb875cc02c7fe8c2cd1134315e777a17f15f73`
    verifies this exact child through both package builds/profile identities and
    one private scale-1 native style/focus observation, without substitution.
    Evidence is retained in the same archive's `installed/REPORT.md`; wider
    compositor, fractional-scale and accessibility qualifications remain.
  - Rounded adaptive tabs and hover paths are accepted at source
    `f5c679443b2fbda1a7a8f91c98d312811bee2cdf`
    (`ven-rounded-adaptive-tabs-vdf`). Retained evidence is in
    `~/.local/state/eon/proofs/ven-rounded-adaptive-tabs-vdf-2026-09-08-review-2/`:
    exact source/binary hashes, 121 ordinary Rust tests, native application
    red/green for wheel scrolling in padding and between tabs, and an Eon-backed
    wheel observation. Rendering, hover, focus, resize, picker and 1x/1.25x/2x
    observations in the sibling `-review-1/` directory remain prior-candidate
    evidence. All native observations use private Sway 1.12/Vulkan; they do not
    close the existing fractional-scale or other compositor gaps. Substituted
    debug-child proof does not establish installed Eon pin/profile behavior.
  - **Environment:** Exact-source x86_64 Linux checks and isolated Sway;
    COSMIC observations belong to the retained prior proof.
  - **Evidence:** Complete locked Rust checks and isolated Nix workspace
    acceptance: two completed tabs without a picker, native Alt+H/L, F6 and
    tab/pane arrows, and tab clicks while an attached Session appends. Retained
    proof `d2d798099934dcf8037bfad6ab856e40c9b989fe` covers exact EONW v4;
    bounded
    active-tab projection, metadata, action, scene, AccessKit, and direct
    structural-shortcut checks while a picker remains bound to another tab;
    isolated native picker-to-durable-to-picker attachment, AT-SPI projection,
    and installed Eon input for all five structural chords; prior complete EONW v3 proof
    `9b2a527d3581f40569f58e24130cc8f8f1222162`
  - **Header evidence:** `ven-2fq` records actual-renderer red/green checks for
    long directory labels and underscore ink inside the header, preserved
    multiline failures, complete locked Rust checks, and native Sway 1.12
    scale-1 installed Eon screenshot acceptance of two real Orbit-backed tabs.
    Eon `/nix/store/j4n3j5r9mfj7xpkj6sjnb9cns11ffsly-eon-0.1.0` consumes
    the header proof source with Orbit `91999d79546422b49bdbc124166a65859d0bd872`.
    Long labels remain on one horizontally clipped line; this does not extend
    scale proof.
  - **Tab shortcut evidence:** `ven-yp4` records focused red/green and complete
    locked Rust checks plus native Sway 1.12 scale-1 composition. Alt+Shift+T
    opens an inherited-directory pending tab; Alt+Shift+W cancels it or closes
    an accepted durable tab while preserving the original Session and final-tab
    protection. Ctrl+T and Ctrl+Shift+W produce exactly `14 17` PTY bytes, with
    no workspace mutation or new-shortcut leakage. Eon source
    `fea679260dd81fcdbb907e804156b00244af2324` accepts the same observations
    through installed `/nix/store/578xxkf17cin8sdn6pb5rr10hxz4yfqg-eon-0.1.0`
    with unchanged Orbit `91999d79546422b49bdbc124166a65859d0bd872`; both Eon
    profiles resolve to their current-tree builds. This changes no scale proof.
- **Open proof:** Fractional native scale remains to be dogfooded.

## VEN-C9 — Optional native decorations

- **Status:** Proved
- **Consumer:** A user launching Venus.
- **Trigger:** Launch with or without one `--no-decorations` option.
- **Result:** The option requests an undecorated native window before creation;
  omission retains decorations.
- **Important failures:** Unknown, duplicate, or excess arguments fail with
  bounded usage before window or transport creation.
- **Owner:** Venus CLI admission and winit native window attributes.
- **Consumes:** Exact locked winit 0.30.13; no Orbit or Eon protocol boundary.
- **Boundary:** Runtime decoration changes are outside this launch-only contract.
- **Proof:** `90988f6ebcde68338e202a9c637c59398aafe93d`
  - **Environment:** x86_64 Linux Wayland
  - **Evidence:** Decorated default, both socket forms, invalid-input rejection,
    and pre-window attribute application

## VEN-C10 — Native paste

- **Status:** Partially proved
- **Consumer:** One attached focused Venus terminal surface.
- **Trigger:** Native Paste, Linux Ctrl+Shift+V, or macOS Command+V.
- **Result:** Venus reads ordinary native clipboard text once, admits at most
  1 MiB, and submits exactly one canonical semantic paste to Orbit; Orbit alone
  owns normal and bracketed terminal encoding.
- **Important failures:** Oversized, invalid, unavailable, unfocused, detached,
  or overlapping shortcut input fails visibly without duplicate submission.
- **Owner:** Venus shortcut precedence, native clipboard read, admission bound,
  and visible failure.
- **Consumes:** Orbit `ORB-C5` and canonical semantic paste at
  `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`, exact winit 0.30.13, and the
  selected native clipboard owner: wl-clipboard-rs 0.9.3 on Linux or target-only
  arboard 3.6.1 on macOS.
- **Boundary:** A hardware Paste key and native Wayland without data-control
  remain unproved. A macOS host-access failure notice and overlap behavior
  remain unaccepted.
- **Proof:** `9c56eb17613e10ef7712a1852b049ed63cf22b18`
  - **Environment:** Deterministic checks plus isolated Sway 1.12 Wayland proof
  - **Evidence:** Overlapping shortcuts, layout-independent release pairing,
    multiline Unicode, normal mode, bracketed mode, and reattachment; native
    Sway source `eb67dac509d0c6033e4373caf8c63eb6e4c88868`
- **Apple Silicon partial proof:** M1 run 34832511106 at exact Venus source
  `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` delivered Command+V Unicode text
  as the exact Orbit-owned terminal bytes, including newline-to-carriage-return
  encoding. M1 run 34831678536 displayed distinct PNG-only and oversized paste
  failure notices without sending PTY bytes. Deterministic tests cover all
  admission and failure-notice paths. A physical Macmini9,1 M1 run found that
  source cleared the correct empty-paste notice on the Command-key release.
  Exact correction `cd881e701183a83c5212d171551d0eae452d9351` retains clipboard
  notices across key releases while clearing them on substantive input. Its
  Mach-O arm64 artifact
  `8231c07af04580646e82db77fb507aa585bb1cfd9aba1bfe222039c3305e6b71`
  passed all 42 macOS application tests. An ordinary Command+V press and release
  against the empty general pasteboard left the exact failure visible with zero
  PTY bytes; the following `Z` cleared it and delivered exactly byte `5a`.
  Native host-access failure remains open.
- **Open proof:** Manual Zellij, Helix, and Yazi acceptance remains assigned to
  the user.

## VEN-C11 — Terminal-background opacity

- **Status:** Proved
- **Consumer:** One Venus native surface.
- **Trigger:** Launch with an optional finite `--background-opacity VALUE` in
  `0.0..=1.0`.
- **Result:** Omission equals `1.0`; the value applies only to terminal default
  background, later Orbit-authored background changes, and terminal padding.
  Explicit cells, selection, inverse video, workspace chrome, notices, focus,
  foreground effects, input, hit testing, and accessibility remain unchanged.
- **Important failures:** Invalid values fail before window creation; values
  below `1.0` require a proved premultiplied surface and otherwise fail before
  presentation.
- **Owner:** Venus launch policy and renderer surface composition.
- **Consumes:** Exact winit 0.30.13, wgpu 30.0.0, glyphon 0.12.0, ORBS v4 at
  `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`, and EONW v3 at
  `96119f29ca2e3ec4ad19bbe272708b07d588429a`.
- **Boundary:** Broader Linux compositor proof remains open.
- **Proof:** `74ab5a0b661210f0afec94086f5358fe50b01f05`
  - **Environment:** x86_64 Linux deterministic checks with retained COSMIC
    native opacity dogfood
  - **Evidence:** Default/background provenance, later background revision,
    opaque chrome and notices, selection, resize, reattachment, and invalid NaN
    with native predecessor `534e47908b7bb7e8286949fe39bf4b0f7cc811f9`

## VEN-C13 — Terminal-authored clipboard delivery

- **Status:** Partially proved
- **Consumer:** One attached Venus client and its native clipboard owner.
- **Trigger:** Orbit emits one canonical terminal clipboard-write effect.
- **Result:** Venus delivers it once without storing, replaying, parsing, or
  reconstructing terminal text; Linux standard maps to ordinary clipboard and
  selection or primary maps to primary selection. macOS maps every destination
  to the general pasteboard.
- **Important failures:** Pre-attachment effect, native failure, invalid
  destination, or Orbit rejection remains bounded and visible without changing
  Scene state or selection-copy behavior.
- **Owner:** Venus native clipboard effect delivery; Orbit owns terminal
  interpretation and text.
- **Consumes:** Orbit `ORB-C11` through canonical ORBS v4 at
  `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c` and the accepted Linux native
  clipboard owner; the current ORBS v11 consumer uses target-only arboard 3.6.1
  on macOS.
- **Boundary:** Ordinary clipboard delivery, Wayland without data-control, and
  broader Linux compositor coverage remain unproved. macOS primary-selection
  emulation stays outside scope, and native write-failure UX remains unaccepted.
- **Proof:** `2d3498258920736eb1bdae2b8869b6547b9735d4`
  - **Environment:** Deterministic Venus checks with accepted x86_64 Linux
    Wayland primary-selection composition
  - **Evidence:** One-shot destination/text forwarding, unchanged Scene,
    preserved selection copy, and exact primary-selection dogfood through Eon
    `0e25ebc2311d7e41edf90c940f8211dd5839bb83` and Eonova
    `4fda9b67b0faa33561624633229135e5e2d579ea`
- **Apple Silicon partial proof:** M1 run 34832511106 at exact Venus source
  `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` delivered one OSC 52 Unicode
  terminal clipboard-write to the general pasteboard. Deterministic tests cover
  all destination mappings and failure ownership.

## VEN-C14 — Supervised native presentation lifecycle

- **Status:** Proved
- **Consumer:** One Eon-supervised Venus process.
- **Trigger:** A complete private Present command, control EOF, or terminal read
  failure.
- **Result:** Present preserves process and terminal attachment while requesting
  presentation of the existing native window; control loss exits through the
  native event loop, releases attachments, and leaves Orbit live; replacement
  may retry transient Busy while the old client releases.
- **Important failures:** Malformed or partial commands do not present; standalone
  Busy remains terminal; Present followed by EOF preserves exact Present-then-
  Exit ordering.
- **Owner:** Venus supervised control stream and native event-loop lifecycle.
- **Consumes:** Eon `EON-C11`, one bounded private byte stream, Unix EOF, and
  exact winit 0.30.13 event/exit APIs.
- **Boundary:** Direct focus and unminimize are unavailable through winit;
  xdg activation remains compositor-controlled.
- **Proof:** `a768e9a1bcb61eac5a21d25b7463c9dc44aa2df8`
  - **Environment:** x86_64 Linux deterministic host and composed Eon acceptance
  - **Evidence:** Complete-command admission, malformed/partial rejection,
    Present-then-Exit, ORBS release, replacement presentation, and live Orbit;
    native predecessor `754eb057ea71ddee57b2f543d85d08d66edc953e`, exact
    Orbit `86aa130629c09dce61d0f232150298656fa5cef4`, prior native
    Present proof `50b7ef7f6c9d5b531b79ecca67c9c8fdf40f355f`, and composed
    Eon acceptance `0bf0b165d06b4a8162be497011070f61f6c2000a`

## VEN-C15 — Compositor-owned background blur

- **Status:** Proved
- **Consumer:** One Venus native Wayland surface.
- **Trigger:** Launch with an optional `--background-blur` flag.
- **Result:** Before showing the window, Venus asks its native host for full-
  surface compositor blur; omission makes no request. Translucent terminal
  pixels reveal compositor blur while explicit cells, selection, inverse,
  chrome, notices, focus, text, cursor, input, IME, and accessibility retain
  their owners. The compositor owns capability, algorithm, strength, and policy.
- **Important failures:** Unsupported or policy-disabled compositors remain best
  effort without failing launch; duplicate, unknown, or excess arguments fail
  before window creation.
- **Owner:** Venus owns one boolean request, winit owns native protocol lifecycle,
  and the compositor owns the effect.
- **Consumes:** Exact patched winit 0.30.13 at
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`, retaining the
  `fb45fbf901fbe70cc9a877b5d651d0b60c206b08` backport of upstream
  `c4afadbfabf7b1e7989b40b493db1a4c7bd8ff4e`; unchanged `VEN-C11`, ORBS v4,
  and EONW v3.
- **Boundary:** Stable crates.io winit lacks the mechanism; non-COSMIC Linux
  compositors remain unproved; no API reports capability or compositor
  acceptance or controls strength.
- **Proof:** `7fc7e4ba97aaf48b586002934a580ef2d1c31694`
  - **Environment:** COSMIC Wayland 1.0.0 on x86_64 Linux
  - **Evidence:** Flag admission, hidden-window attributes, blur off/on at
    opacity `0.0`, `0.65`, and `0.88`, and preserved terminal/chrome semantics
    on COSMIC revision `091583ac84abac02967ae358cf9570ddfef63b31`
- **Open proof:** A populated current Eon workspace was not re-dogfooded; this
  proof reuses accepted VEN-C11 composition evidence.

## VEN-C16 — Native Linux Wayland and Apple Silicon macOS platforms

- **Status:** Proved for x86_64 Linux native Wayland; Apple Silicon macOS has a
  proved opaque foundation and a partial native-interaction proof, but remains
  unsupported.
- **Consumer:** Every packaged Venus application launch.
- **Trigger:** Build or launch the native application on an approved target.
- **Result:** Venus uses native Wayland with Vulkan on Linux and the native
  AppKit window lifecycle with Metal on `aarch64-darwin`. Both
  targets share protocol validation, model, scene, draw inputs, renderer,
  accessibility tree, semantic actions, and workspace composition; only native
  host mechanics vary.
- **Important failures:** An unavailable native host or graphics backend,
  invalid platform mapping, or unsupported target fails before presentation
  attachment with a bounded explicit error. Evaluation or compilation alone is
  not native runtime proof.
- **Owner:** Venus native host and dependency feature selection.
- **Consumes:** User-approved `EON-C7`; current source uses exact winit 0.30.13 at
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`, wgpu 30.0.0, and
  wl-clipboard-rs 0.9.3 with ORBF v2 / ORBS v11 and EONW v6. The macOS path
  consumes Orbit `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, whose accepted
  `ORB-C14` source proof is
  `0233f4d34b294a50c5bc7f373859cf4ec04d2414`. Target-scoped AppKit/AccessKit
  and Metal features were selected by
  `ven-qualify-venus-stack-apple-silicon-macos-ili`; the interaction slice adds
  target-only arboard 3.6.1 for general-pasteboard text. The IME correction
  changes only the exact fork revision and retains the dependency graph.
- **Boundary:** X11, Xwayland, Intel macOS, signing, notarization, packaging,
  distribution, and other native platforms remain unsupported. Apple Silicon
  IME behavior beyond the proved Kotoeri Roman-to-kanji path, held-key repeat,
  physical trackpad gesture quality, a host-access failure notice,
  accessibility, lifecycle identity, effects, and Eon composition remain
  unaccepted.
- **Linux proof:** `e033efadf023492ae02eb1e9036de98ad93d2f98`
  - **Environment:** x86_64 Linux native Wayland
  - **Evidence:** Locked format/check/test/Clippy, exact dependency features,
    no-Wayland failure, and live native event-loop launch.
    Installed Eon/EonTerm integration is accepted at Eon
    `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`, consuming Venus
    `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` with the same Wayland/Vulkan
    host. `eon-accept-live-output-input-nxh` records exact package/profile
    identity and isolated Sway 1.12 selection-copy delivery to ordinary and
    primary clipboards. VEN-C10 native-paste and VEN-C13 terminal-authored
    clipboard gaps remain separate; this does not prove non-systemd delivery
    or broader compositor coverage.
- **Open macOS proof:** `ven-prove-venus-apple-silicon-macos-nm7` must record
  exact hardware, OS, toolchain, artifacts, automated checks, interactive
  observations, failures, and limitations. Its first implementation child,
  `ven-bring-up-opaque-appkit-metal-g4a`, owns the hidden AppKit window,
  AccessKit initialization, Metal surface, opaque rendering, and target graph;
  `ven-deliver-macos-input-pasteboard-links-zfw` owns the partial interaction
  evidence below. The remaining native interactions, accessibility acceptance,
  lifecycle identity, transparency/blur, exact Orbit dogfood, and support claim
  stay open.
- **Opaque foundation proof:** `af3804a890ce4898f1ebd75eccf44075fc063559`
  - **Environment:** Standard GitHub `macos-15` M1, macOS 15.7.9 build 24G830,
    aarch64, rustc/cargo 1.98.1.
  - **Evidence:** [Run 34819451958](https://github.com/Yazelix/eon-desktop/actions/runs/34819451958)
    passed locked metadata, target-tree assertions, all-target check, native
    build, and a real hidden-to-visible AppKit/AccessKit/Metal launch against
    accepted Orbit runtime `0233f4d34b294a50c5bc7f373859cf4ec04d2414`.
    With `UID` and `XDG_RUNTIME_DIR` absent, Orbit and Venus found the same
    effective-UID fallback socket without an explicit socket argument. Startup
    admitted an 84 by 27 native-scale grid; a 976 by 650 screenshot captured
    the deterministic text/color corpus (PNG SHA-256
    `201d443bf2a1f5dd83cf897f37491ef7b4577db9a26b0d8e46ee707cf0b91151`);
    presentation-control EOF closed Venus cleanly while Orbit remained alive.
    The exact source also passed the locked Linux suite (131 tests, five
    native-only checks ignored) and strict local Apple-target Clippy.
  - **Boundary:** This proves only the first opaque foundation child. It does
    not accept the remaining macOS behavior or change VEN-C16 support status.
- **Native interaction partial proof:**
  `0f0a9ab945f4fbc492b79468a6b4d877a19c90ee`, with empty-paste notice
  correction `cd881e701183a83c5212d171551d0eae452d9351`
  - **Environment:** Standard GitHub `macos-15` M1, macOS 15.7.9 build 24G830,
    aarch64, rustc/cargo 1.98.1, exact Orbit runtime
    `0233f4d34b294a50c5bc7f373859cf4ec04d2414`.
  - **Evidence:** [Run 34832511106](https://github.com/Yazelix/eon-desktop/actions/runs/34832511106)
    passed 73 selected tests, the Apple all-target check, and native builds for
    Venus and Orbit. One AppKit/Metal window delivered an OSC 52 Unicode value
    to the general pasteboard, copied an exact hovered URI with Command+C, and
    sent exact bytes `Aé界rrpaste-é界\r` from native key text plus Command+V.
    Presentation-control EOF closed Venus with status zero while Orbit stayed
    alive. Run 34831678536 displayed distinct PNG-only and oversized paste
    failure notices without PTY input. Local Rust 1.96.0 checks passed 74
    library, 42 application, and 17 integration tests plus strict Linux and
    Apple-target Clippy.
  - **Hosted limits:** CI could not deliver mouse-button selection or a reliable focus
    transition and could not distinguish native repeat from discrete key input.
    The empty-paste capture retained stale link feedback, and the proof harness
    used a non-failing negated `cmp`. Corrected run 34833225890 did not receive
    a runner after GitHub reported a failed payment or spending limit.
  - **Physical-host extension:** A Scaleway physical Macmini9,1 with Apple M1,
    8 GiB RAM, and macOS 26.6.1 build 25G76 ran exact Orbit
    `0233f4d34b294a50c5bc7f373859cf4ec04d2414` and Venus through vncdotool
    1.4.2 over native ARD/VNC at 1920 by 1080. Source `0f0a9ab` proved focus
    loss and resumption, exact click-drag selection and Command+C copy, and
    product-path Command+click opening the exact hovered URI in Safari. It also
    exposed the empty-paste notice release defect. Exact correction `cd881e7`
    passed all 42 macOS application tests and a native regression: ordinary
    Command+V against an empty general pasteboard kept the exact notice visible
    after both keys were released, sent zero PTY bytes, and let the next `Z`
    dismiss the notice while delivering exactly byte `5a`. Both supervised
    pairs exited zero with their sockets removed and no process left. Captures,
    logs, inputs, source identities, and SHA-256 sums are retained in
    `~/.local/state/eon/proofs/ven-macos-scaleway-m1-2026-09-16/`.
  - **Kotoeri composition extension:** Venus candidate
    `f0cf58f1783c7ef33462ff114158eba9d3f29ee1`, tree
    `6f7c81c5750ef71a13f4fd51ae1a1703c16601b8`, consumes selected winit fork
    `7c209ec5bebbd2963f75a39f79b320af6b03f72c`. Its source archive SHA-256 is
    `28e8e420dfb4dda30fdaa3a7f55f8536a5ffd7106bfffbdc78e44eed383fc320`,
    Cargo.lock SHA-256 is
    `552dc37ce50caa9eebf9035ac84c15058beb787b3e5d08258278e5dfe3d33d1f`, and
    physical-M1 arm64 release artifact SHA-256 is
    `e6c615793c0ee5532b5bc6bdee714294e785a109ed2b22676576546471837263`.
    Local locked checks passed 74 library, 42 application, and 17 integration
    tests plus strict Linux and Apple-target Clippy; the M1 passed all 42
    application tests. Native Kotoeri converted `にほん` to the selected `日本`
    candidate and, after its separate select and commit Enters, Orbit received
    exactly one six-byte UTF-8 value `e697a5e69cac`, SHA-256
    `cf2abf0c5be326cb922a70f8163f91079c4d9aa8655c60ead89ad545c9de2e92`.
    It remained six bytes after three seconds. The isolated Venus and Orbit
    processes stopped, the private socket was removed, and the temporary host
    credential was restored.
  - **Physical-host limits:** ARD/VNC delivered only one byte for a held key and
    plain `e` for Option-E then E, so native repeat and dead-key behavior remain
    unaccepted. The route cannot prove physical trackpad touch or momentum, and
    no host-pasteboard access failure was induced. Other IMEs, accessibility,
    lifecycle identity, effects, Eon composition, and macOS support remain open.

## VEN-C17 — Caller-owned native application identity

- **Status:** Proved
- **Consumer:** Eon, EonTerm, or one approved EonTerm composition.
- **Trigger:** The caller supplies one optional validated application ID before
  Venus creates its window.
- **Result:** The accepted value becomes winit's Wayland general name and native
  xdg-toplevel app ID; omission remains `eon`; Orbit-authored titles remain
  independent presentation state.
- **Important failures:** Empty, non-UTF-8, oversized, or non-token values fail
  before window creation; Venus never infers identity from child argv,
  environment, titles, executable names, runtime paths, or desktop files.
- **Owner:** The caller owns identity choice and desktop metadata; Venus owns
  direct CLI validation and native Wayland materialization.
- **Consumes:** Exact winit 0.30.13 commit
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`; Eon owns the selected value.
- **Boundary:** Native mapping only; no branding framework or mutable identity.
- **Proof:** `ab24961bd6b2f9403736e52ebbac8cc266488a41`
  - **Environment:** x86_64 Linux Wayland with installed Eonova acceptance
  - **Evidence:** Locked format/check/test/Clippy and exact live
    `--application-id eonova` grouping without Session duplication

## VEN-C18 — Shared popup and picker-first presentation

- **Status:** Proven for EONW v5 and v7 on x86_64 Linux Wayland
- **Consumer:** One person using a Venus workspace supplied through EONW v5.
- **Trigger:** The active tab selects a popup, its selection changes, or the
  person invokes an Eon-supplied catalog shortcut.
- **Result:** Workspace-only startup waits for a canonical snapshot before
  attaching its sole terminal endpoint. A selected popup replaces the entire
  pane stack while tabs remain visible and actionable. Each inactive tab's
  selection and hidden popup instances remain Eon-owned. A popup-only tab may
  have an empty body: no placeholder pane, attachment, resize failure, or alert.
  Its selected tab remains the only keyboard and accessibility focus target;
  reopening work restores terminal focus.
  A catalog shortcut names the exact tab, entry and current instance, with
  Toggle intent from terminal focus and Focus intent from workspace chrome.
  Structural repeats remain consumed without sending duplicate actions.
  Tab traversal and structural actions use the existing semantic workspace
  path; Eon decides their admission. F6 cycles visible focus regions, and
  Escape returns chrome focus to the terminal. While the terminal is focused,
  Escape, Ctrl+C, Tab and Enter reach its application through ordinary Orbit
  input. Popup entry labels also name the accessible tab panel.
- **v7 expansion (proved on x86_64 Linux Wayland):** Venus accepts multiple
  pending tabs and their independent Project popup endpoints from one canonical
  EONW v7 snapshot.
  It attaches only the active tab's selected picker and preserves the others
  across tab switches. New tab and Alt+Shift+W retain the existing semantic
  CreateTab and exact-active-tab CloseTab actions. No local cancellation state
  or native picker is added. The exact Eon producer
  `f41a41c9aecc4c436edfa2f394b832aa6d8711ad`, Venus consumer
  `479d7cef29aaa40d2d93d610567517357479877c`, and Orbit
  `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` passed locked Rust and Nix
  checks. Isolated native Sway input on composed Eon artifact
  `/nix/store/0m637rzcg5ksm27c5q0p6pd8gk2mw9yc-eon-0.1.0` switched between
  two pending picker endpoints, exercised both, and closed only the selected
  tab. A headless Mesa screenshot sometimes misses picker body text on both
  this candidate and the previously installed single-picker Eon; the canonical
  frames and Venus scenes contain the text. This is not a clean-frame claim.
- **Geometry:** A selected popup reuses the pane stack's exact outer rounded
  frame, so toggling preserves every chrome edge. Shared Eon-supplied logical
  margins inset the popup terminal content within that stable shell. A compact
  border label keeps a small top gutter before terminal padding. Scene owns
  both rectangles: chrome drives the outline, while terminal drives resize,
  clipping, pointer input, IME and accessibility. Margins and the title gutter
  shrink to preserve a usable cell grid; tiny surfaces omit the label. Exposed
  margins share the terminal default background and opacity, with the existing
  best-effort surface-wide compositor blur request. Pane-frame configuration
  has no effect on the popup outline.
- **Important failures:** Hidden or replaced endpoints retain no active
  presentation or input. Stale popup actions carry exact instance guards;
  canonical EONW validation and failure responses preserve the last coherent
  snapshot. Invalid frames, aliased endpoints, missing selections and malformed
  catalog values fail in the owner codec. Unavailable attachments retain the
  existing bounded recovery/failure path. Standalone Venus preserves raw
  workspace and catalog keys. Invalid or mixed launch forms fail before a
  window or transport starts.
- **Owner:** Venus owns native projection, shortcut dispatch and focus. Eon owns
  catalog, geometry settings, commands, cwd, popup/tab lifecycle, selection,
  action admission and cleanup. Orbit owns terminal Sessions and PTYs.
- **Consumes:** EONW v5 and `eon-workspace-protocol` 0.1.0 at exact Eon source
  `0cc8f477298681ae3945903e8fdb5852d487c5ab`; ORBF v2 / ORBS v11 at unchanged
  Orbit `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
- **Boundary:** No local command launch, chooser-mode recognition, cwd or
  dismissal policy, second schema, simultaneous terminals, compatibility
  negotiation, Eon runtime activation, or additional platform.
- **Proof:** `2d81d50d6decb4e3702e5cd7ff84e0a7c5642c00`.
  - **Environment:** x86_64 Linux, isolated Sway 1.12 scale 1 and Nix Mesa
    26.1.2 lavapipe; bounded canonical EONW v5 producer with real Orbit Sessions.
  - **Evidence:** Locked fmt/check/test/Clippy/build and the affected Nix build
    pass all 125 ordinary tests. Canonical projection, targeted shortcuts,
    AccessKit actions/focus, empty-body focus and failure-alert regressions pass.
    The native Nix artifact
    `/nix/store/88k1kx4ilckx3yk841zp2rlc21rb9i2j-venus-popup-proof-0.1.0`
    passes ordinary/Agent/Project switching, Focus/Toggle, application-owned
    Escape/Tab/Ctrl+C, normal/narrow/tiny geometry, replacement endpoints and
    supervised empty-body startup/reopen. The existing native application
    regression passes. Margins share background/opacity; final captures have
    no stale empty-body terminal or resize notice. All 12 observed live product
    process identities survive. Reproduction inputs, logs and captures:
    `~/.local/state/eon/proofs/ven-shared-popup-surface-ehk-2d81d50/REPORT.md`.
- **Open proof:** Full Eon command/lifetime and installed popup acceptance
  remain downstream in `eon-tool-popups-e13.2`.
  Native fractional scale, other compositors, actual compositor blur and
  screen-reader interaction remain qualified by their separately indexed proof.
  Previous v4 picker acceptance is retained in Git and
  `ven-consume-picker-first-eonw-v4-zd1`: Venus
  `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` and Eon distinct-endpoint
  acceptance `4298fbb8868752e3d6c8eb4fd79fae067ab3e2a1` do not prove v5.

## VEN-C20 — Native shortcut viewer

- **Status:** Proved
- **Consumer:** One person using a Venus surface attached to an Eon workspace.
- **Trigger:** Physical Alt+Slash toggles the viewer; Escape closes it.
- **Result:** Venus presents one ephemeral, read-only native dialog containing
  grouped native workspace, host, and current Eon popup shortcuts. Fixed
  bindings share their descriptor owner with dispatch; popup rows use the exact
  enabled label and normalized physical shortcut from the current Eon
  snapshot. Wheel and navigation keys scroll bounded content. Opening,
  refreshing, scrolling, and closing send no terminal bytes or workspace
  action; matching releases remain captured and prior terminal/chrome focus is
  restored on dismissal. The title, subtitle, and first group heading occupy
  separate readable vertical space.
- **Important failures:** Repeated toggle events do not oscillate the viewer.
  Workspace loss, empty or maximum-size catalogs, long labels, resize, scale,
  focus loss, and tiny surfaces retain local dismissal and clipped, scrollable
  presentation. While open, keyboard, pointer, IME, clipboard, link, and
  workspace actions do not reach their normal targets.
- **Owner:** Venus owns viewer state, fixed native binding descriptors, scene,
  rendering, input precedence, and AccessKit projection. Eon owns the enabled
  popup catalog and must reserve Alt+Slash before full-product delivery.
- **Initial consumes:** EONW v5 and `eon-workspace-protocol` 0.1.0 at exact Eon source
  `0cc8f477298681ae3945903e8fdb5852d487c5ab`.
- **Boundary:** No Orbit Session, Eon configuration parsing, child-application
  shortcuts, action execution from rows, search, filtering, rebinding,
  tutorial, telemetry, persistence, momentary timer, second renderer, generic
  modal framework, dependency, or platform-support expansion.
- **Initial viewer proof:** Accepted source
  `2f05a408e499117c1ddb57f99255dcdb4a236bdf`:
  - **Environment:** Exact package source
    `/nix/store/v4kyjq0z34kfgrhz8js9gh7fkhd9lk3n-venus-popup-source`;
    x86_64 Linux, Sway 1.12 headless native Wayland at 1440x900 scale 1,
    wgpu Vulkan on Mesa 26.1.2 lavapipe; EONW v5 fixture at
    `0cc8f477298681ae3945903e8fdb5852d487c5ab`; Orbit artifact from
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
  - **Evidence:** Locked fmt/check/test/Clippy and the Nix build pass all 131
    ordinary tests. The native artifact
    `/nix/store/q8pxjwq26z2qnazm1pbjbfg39yggbpkb-venus-popup-proof-0.1.0`
    opens, scrolls, survives a live v5 refresh and 320x240 resize, and closes
    through Escape and Alt+Slash without terminal bytes or workspace actions;
    subsequent `a` and `b` reach the same Orbit Session. Native AT-SPI
    traversal finds one modal, focusable dialog, every group, the fixed rows,
    and both live Agent/Project rows with their accessible names; the focused
    source projection is protected by its AccessKit regression. Every Eon
    product process identity observed at proof start survives. Reproduction
    inputs, logs, accessibility tree, and captures are in
    `~/.local/state/eon/proofs/ven-toggleable-shortcut-viewer-s41-2026-09-12/`.
- **Header-spacing proof:** Accepted source
  `8af87bb0d07bc975413b05e3bdea9404ed6c1e57`:
  - **Environment:** x86_64 Linux, private Sway 1.12 native Wayland, Mesa 26.1.2
    lavapipe; exact ORBF2/ORBS13 at Orbit
    `b6cecf8f2ee35570b41cfdc578b095889d917fe2` and canonical EONW7 at Runtime
    `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`.
  - **Evidence:** Locked fmt/check/test/Clippy/build and the standalone Nix build
    pass all 144 ordinary tests. The existing geometry regression covers normal,
    compact, and tiny bounds at scales 1, 1.25, 1.5, and 2; scene-owned title and
    subtitle rectangles reserve their complete footprint before group content.
    The native artifact
    `/nix/store/0qb4k6xb271nmqhspsb7rzbl7g6qvixv-venus-header-proof-0.1.0`
    shows clear title/subtitle/Navigate separation and a complete first row at
    1100x650 and 320x240, including compact scale 1.25. At 320x180, the header
    remains readable while content clips and scrolls. PageDown/Home/End, resize,
    Escape/Alt+Slash dismissal, fixed header/footer space, and AT-SPI group/row
    order pass without viewer terminal bytes or workspace actions; subsequent
    `a` and `b` reach the same Orbit Session. The existing native live-input
    regression passes at scale 1 with accessibility disabled. Its synthetic
    fixture panics in AccessKit with accessibility enabled on both the pre-fix
    baseline and this source; the real viewer's enabled AT-SPI check passes.
    Inputs, identities, logs, baseline failure, and captures are retained in
    `~/.local/state/eon/proofs/ven-shortcut-viewer-header-spacing-ol8-2026-10-06/REPORT.md`.
- **Installed delivery:** Eon `EON-C20` proves the original full-product
  interaction through `eon-deliver-native-shortcut-viewer-rif`. Installed
  adoption of the header correction requires a separate accepted Venus pin
  refresh. Apple Silicon macOS remains under its separately approved proof chain.

## VEN-C21 — Native Eon Bar workspace header

- **Status:** Proven
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace.
- **Trigger:** The person points at, focuses, or activates the workspace header;
  uses New tab, Keyboard shortcuts, or active-tab Close; scrolls the tab region;
  or left-presses its empty drag region.
- **Result:** The existing workspace tab strip remains one row at its existing
  height. Scrollable tabs keep the leading space and retain their labels,
  overflow, reveal, selection, hover path, stable identity, and accessibility.
  New tab follows the rightmost tab when tabs fit and stays at the tab-region
  edge during overflow. Keyboard shortcuts (when VEN-C24 requires the fallback)
  and active-tab Close remain trailing
  controls. The buttons use the existing CreateTab and exact-active-tab CloseTab
  EONW actions and the existing local VEN-C20 viewer. Their tooltips and
  accessible names are
  `New tab — Alt+Shift+T`, `Keyboard shortcuts — Alt+/`, and
  `Close tab — Alt+Shift+W`. A bounded empty region between New tab and the
  trailing controls starts compositor-owned window movement only from its
  immediate left press.
  The accessibility tree orders tabs, New tab, optional quota, Shortcuts,
  then Close tab.
  It yields before tabs or controls when width contracts. Tab and Shift+Tab
  traverse focused header controls; Enter and Space activate them. Hover,
  pressed, and focus states remain distinct, and focus includes an outline.
- **Important failures:** Buttons, tab gaps, pane chrome, terminal content,
  popup content, tooltips, and the shortcut viewer never become drag targets.
  Narrow surfaces preserve an active-tab region and all visible controls without
  overlap. A failed native drag is consumed without workspace action, terminal
  input, crash, or state mutation. Existing stale-presentation and stable-target
  gates continue to reject obsolete actions.
- **Owner:** Venus Scene owns header geometry and hit identity; the renderer
  owns its native presentation; the application owns input, local viewer state,
  and action routing; AccessKit projects the same rectangles and identities.
  Eon retains workspace topology and action-policy ownership; winit and the
  compositor own native window movement.
- **Initial consumes:** VEN-C8, VEN-C20, EONW v5 at exact source
  `0cc8f477298681ae3945903e8fdb5852d487c5ab`, Orbit ORBF v2 / ORBS v11 at
  `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, and patched winit 0.30.13 at
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`.
- **Boundary:** No second row, widget framework, status/quota data, polling,
  configuration, Nova compatibility, inactive-tab close, protocol change,
  dependency, native-decoration policy, or platform-support expansion.
- **Proof:** Source `2ce3594b9ca0f62a2723d5ce5a005ed3560aea64` has runtime diff
  `5004ec5447592ebfd582e67e1a752c9a154a8daa8a32a0d05c7e17284c886632`
  over `d447255686ec85e0b42781f34989aa5cf754cd55` and is content-identical
  to package source `94c9d6874590be0cc175c1f438362c779fe27fa8`. Locked format,
  check, 137 ordinary tests, strict all-target Clippy, Eon manifest/package
  build, and full flake check pass. Installed candidate
  `/nix/store/38iax08krzlc5gfl67skqiwlhbrk0jmf-eon-0.1.0` passed isolated
  Sway 1.12 scale-1 pointer, keyboard, AT-SPI, exact-action, exclusion, and
  real compositor-drag checks. An exact EONW v5 fixture passed eight-tab
  overflow and interaction at 960, 320, and 100 pixels; the 100-pixel capture
  retains the active tab number and all three controls. Both runs preserved all
  11 ambient product identities and cleaned their private processes;
  evidence is retained in
  `~/.local/state/eon/proofs/ven-native-eon-bar-header-iqj-2026-09-17/`.
  The user authorized the proof-bearing source commit on 2026-09-17. The native
  boundary is x86_64 Linux Wayland; Apple Silicon macOS remains under its
  separate proof chain.
- **Placement follow-up:** Source `562cacf74e101ee9ea18460c21e47d86eec2376d`
  consumes EONW v6 `3103a00a904c347472899549ce611da765a71650` and Orbit
  ORBS v12 `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` without changing
  either. It moves only Scene-owned New tab geometry. Focused ordinary,
  overflow, and narrow hit-target checks passed with the locked Venus suite (80 library, 44
  application, 17 integration tests), check, format, and strict Clippy. The
  exact Eon candidate `/nix/store/31mwvisf7s1aiyp42agslc2wjbz47lms-eon-0.1.0`
  built and ran under private Sway 1.12 headless/pixman with Mesa 26.1.2
  lavapipe at scale 1. A real New tab pointer click created a second tab;
  960-, 320-, and 100-pixel captures show the adjacent or pinned button and
  retained active tab and controls. Evidence is retained under
  `~/.local/state/eon/proofs/ven-place-new-tab-beside-tabs-pct/`. Private
  processes stopped and the live Eon supervisor and Codex Session survived.
- **Accessibility correction:** Source `6036430ffed1b84fd2995525e0a19f071809c588`
  makes the AccessKit child order match the visual order with quota present.
  The focused assertion failed on the former order and passed after the fix;
  locked format, check, 141 ordinary Rust tests, strict Clippy, and the exact
  composed Eon build pass. Under private Sway 1.12 headless/pixman, the native
  AT-SPI window returned tabs, New tab, Codex quota, Shortcuts, Close tab, and
  pane panel in that order. New tab's extent starts at x=78, quota at x=738.
  The private run stopped; the ambient supervisor and Codex Session survived.
  This proves AT-SPI tree order, not a screen-reader workflow or macOS.
- **Quota-gap correction:** Source `6aa79f6405229cd64bd1d55e01c5b11b96657c71`
  moves the optional quota chip into the slot vacated by New tab, keeping the
  blank drag region continuous and the chip adjacent to Shortcuts. The focused
  geometry assertion failed against the prior source and passed after the fix;
  locked format, check, 141 ordinary Rust tests, and strict Clippy passed.
  Exact Eon `/nix/store/0j919y26rigssy25w2mz96n9kpxiq7hv-eon-0.1.0`
  ran under private Sway 1.12 headless/pixman at scale 1. Its 960-pixel capture
  shows the new layout; a real drag from the restored blank region moved the
  window from (100, 100) to (200, 170). The private generation stopped and
  ambient Sessions survived. The capture is retained under
  `~/.local/state/eon/proofs/ven-place-new-tab-beside-tabs-pct-review-2026-09-19/`.

- **Shortcut-hints refinement:** VEN-C24 at source
  `1897526455a23bf8ed390bb2e144f87280968d6c` owns conditional Shortcuts
  visibility. Its native proof covers incomplete, complete, hidden and focused
  fallback states; the prior header proofs remain scoped to their mechanisms.

## VEN-C22 — Codex quota fact chip

- **Status:** Proven
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace.
- **Trigger:** The current complete EONW v6 snapshot supplies, changes, or
  removes its optional Codex quota observation.
- **Result:** Venus projects one noninteractive fact chip into the existing
  single-row Eon Bar. Fresh facts show the exact monochrome OpenAI Blossom
  followed by every available elapsed/total window position and remaining
  percentage at ordinary width, such as `[mark] 3h/5h 74% · 6d4h/7d 42%`.
  Under pressure the chip keeps the mark and complete lowest-remaining window,
  then disappears before active-tab identity, fixed controls, or the bounded
  drag region lose their accepted space. Stale labels include `old`; blocked
  and unknown observations show the mark followed by `blocked` or `unknown`.
  Hover text and the static AccessKit label continue to name Codex explicitly and expose every
  window's human-readable position and remaining percentage, reset availability,
  and source freshness or permission state without raw epoch timestamps.
- **Important failures:** An absent observation creates no rectangle, gap, hit,
  action, focus target, or accessibility node. Missing resets remain explicit;
  unusual durations use exact minute labels. A stale label that cannot retain
  `old` disappears. Narrow and tiny widths retain the VEN-C21 tab and control
  priority. Quota-only refreshes do not recenter workspace navigation, cancel
  terminal scroll, replace the attachment, or send an Orbit resize. Venus has
  no quota cache outside the current EONW snapshot and does not refresh or infer
  provider state.
- **Owner:** Eon owns Codex process, authentication, account invalidation,
  refresh, retry, normalization, freshness, permission state, and the optional
  EONW field. Venus Scene owns chip text and responsive geometry; the existing
  glyphon renderer owns the exact monochrome mark; hover and AccessKit own the
  explicit provider name.
- **Consumes:** EONW v6 at exact Eon source
  `3103a00a904c347472899549ce611da765a71650`; VEN-C21 at exact accepted source
  `2ce3594b9ca0f62a2723d5ce5a005ed3560aea64`; unchanged Orbit ORBF v2 / ORBS
  v11 at `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`.
- **Boundary:** One fixed monochrome service mark adds no generic icon or image
  system, decoder, provider parsing, process, filesystem or network access,
  timer, cache, quota mutation, account identity, click action, focus target,
  alert, threshold color, chart, configuration, widget framework, second row,
  other provider, Orbit change, or platform expansion. Workspace transport loss
  expires the optional quota while retaining the last useful workspace.
- **Proof:** Source `247dcb2dc853e604d51ac727471e1e65e1535f1d` has runtime diff
  `37c537a2ec501fd37ed2a1f4654e5a20c0638b4e71c7971c9cb4b9a0b6ce2125`
  over `76f0f4cd53b275114323f5458cd5e73f379b9202`. Locked format, check,
  139 ordinary tests, and strict all-target Clippy pass. Test-only source
  `eb0127bb5cc80553156a0ac185469986d483eb77` corrects the mask-origin
  assertion and repeats those checks; runtime code is unchanged. Exact package
  `/nix/store/qcpnwl8hhy73x3v93ic5mv3h3cj7cwm4-yazelix-venus-openai-blossom-proof-0.1.0`
  passed isolated Sway 1.12 scale-1 native wide, compact, hidden, stale,
  blocked, unknown, absent, real-Codex, Eon-loss, hover, focus, and AT-SPI
  checks. The wide and compact captures retain the exact monochrome Blossom;
  all 27 ambient product identities survived and every private process was
  cleaned. Evidence is retained in
  `~/.local/state/eon/proofs/eon-openai-blossom-eon-bar-2026-09-18/`.
  Installed full-product composition remains Eon's responsibility; the native
  boundary is x86_64 Linux Wayland and macOS remains outside VEN-C22.

## VEN-C23 — New Eon window shortcut

- **Status:** Proved on x86_64 Linux Wayland.
- **Consumer:** A Venus window supervised by full Eon on x86_64 Linux Wayland.
- **Trigger:** Physical Alt+Shift+N, including while Shortcuts is open.
- **Result:** Venus runs the exact Eon executable supplied by its supervisor
  with `window new` on a worker thread. The action appears in native Shortcuts;
  the current terminal does not receive the key. Eon owns the new workspace,
  process, runtime identity, and Sessions.
- **Important failures:** A missing executable, process launch failure, or
  nonzero command status produces a bounded local notice and leaves the current
  Venus window usable. The command receives fixed arguments without a shell or
  PATH lookup.
- **Owner:** Venus owns native key capture, help, command invocation, and local
  failure notice. Eon owns `EON_NEW_WINDOW_EXECUTABLE` and all window policy.
- **Consumes:** EON-C23 and EONW v7 at Eon
  `fdefc640eae66c7e2781930b609e87159bde82e0`; Orbit ORBF v2 / ORBS v13
  at `b6cecf8f2ee35570b41cfdc578b095889d917fe2` remains the unchanged
  Session owner.
- **Boundary:** This creates another Venus process and Eon workspace, not a
  second view of an Orbit Session or a multi-window Venus event loop. The
  composed macOS product remains unproved.
- **Proof:** Source `a23f9eed95120ca4acea8789bf7d32d2472d84ce` passed
  locked fmt/check/test/clippy and the exact-command check. Nix built Venus
  `/nix/store/b2g7c3blbk2jyizb4pnhn3kd3ndsj9ly-yazelix-venus-0.1.0`
  into candidate full Eon
  `/nix/store/h7dcdf0zz2bha4j0dc8b5n0vlh30762w-eon-0.1.0` with Orbit
  `b6cecf8f2ee35570b41cfdc578b095889d917fe2` and EONW v7. On private
  Sway 1.12, native x86_64 Linux Wayland with Mesa lavapipe, raw physical
  Alt+Shift+N opened a second native window; `eon window new` opened a third.
  A durable Session in the shortcut window survived Venus detach and reattach
  with the same tab and endpoint; exact Stop of the CLI window left the other
  two windows live. The private processes were stopped afterward. Observation
  and logs are retained at
  `~/.local/state/eon/proofs/eon-c23-native-2026-09-25/`.

## VEN-C24 — Contextual bottom shortcut hints

- **Status:** Proved on x86_64 Linux native Wayland.
- **Consumer:** One person using a Venus window attached to an Eon workspace.
- **Trigger:** A fresh workspace window, physical Alt+Shift+B, current native
  focus or popup catalog changes, and window size or scale changes.
- **Result:** A fresh window shows one compact, read-only native footer. Its
  window-local visibility choice survives tab switches, popups and resize.
  Showing reserves footer space; hiding returns it to terminal content.
  Initial configured terminal rows include the footer overhead. Hints derive
  from fixed dispatch descriptors and the current enabled popup catalog, group
  shared modifiers once in a continuous row, and retain complete key/action
  pairs in a stable order. Compact groups spread across the bar with equal gaps,
  outer groups near its edges and a lone group centered. Modifier headers alone
  have padded rounded backgrounds without braces; group bodies stay plain.
  Insets, badge padding and minimum gaps participate in whole-hint fitting.
  The main region and footer share side insets and one continuous outer frame,
  with matching upper/lower corner radii, the hidden main frame's bottom inset
  and a straight separator at the join. The outer bottom gap is reserved in
  addition to the compact hints row height. The row is at least the corner
  radius tall, preserving the straight separator at small font sizes. The bar uses the inactive-tab
  background, coral red modifier headers, mint keys and gray labels.
  Copy, Paste and Open link stay in full help and dispatch, without footer hints.
  Compact modifier headers use C/A/S aliases joined by hyphens, such as A-S.
  The compact tab range is 1..9; full help retains the tenth-tab Alt+0 binding.
  Short labels use keys and hints; one move hint groups horizontal and vertical keys.
  Full help and accessible labels retain complete chords and action names.
  Creation labels name the tab, pane or window explicitly.
  Width pressure drops low-priority hints before help and the visibility
  toggle. Native focus-specific keys appear only in their applicable context.
  F6 stays in full help and keyboard navigation, without a footer hint.
  Project is omitted from hints; Alt+Z and its full-help entry remain available.
  The header Shortcuts button is omitted only when the visible footer covers
  every applicable hint and that button lacks keyboard focus; otherwise it
  remains. Alt+/ and the complete viewer
  remain available, including the hints toggle entry. The viewer retains its
  existing modal input isolation.
- **Important failures:** A press toggles once; repeats and releases do not
  leak to terminals. Toggling preserves focus, selected identities and live
  processes; normal terminal resizing is its only terminal effect. Tiny bounds
  omit whole hints and preserve valid Scene geometry. Hidden hints leave no
  accessible footer. Visible hints expose complete chords as read-only text,
  without extra keyboard-focus targets or live announcements.
- **Owner:** Venus owns native visibility, projection, fitting, Scene geometry,
  rendering, input and AccessKit. Eon owns product selection and delivery;
  Runtime reserves the toggle against popup collisions; Orbit owns resizing.
- **Consumes:** Existing VEN-C19/C20/C21, canonical EONW v7 at Runtime
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, ORBF v2 / ORBS v13 at Orbit
  `b6cecf8f2ee35570b41cfdc578b095889d917fe2`.
- **Boundary:** No new input mode, child-application key discovery, interactive
  hints, durable preference, generic widget registry, dependency, wire change
  or platform expansion. Future Eon modes supply hints from their own actual
  bindings and replace contents at the same footer height.
- **Proof:** Source `dc5a4fb7008ae612cdc6935267a39aec3441060b`, owned by Eon
  `eon-bottom-hints-bar-vmum`, passes locked fmt/check/tests, strict all-target
  Clippy and debug build: 152 tests passed, five existing native/Vulkan ignores.
  - **Environment:** Private Sway 1.12 headless/pixman and software Vulkan,
    x86_64 Linux native Wayland, scales 1/1.25, accepted Orbit
    `427cf2d074cd1db905a2cc2602a5475065d1c790` and the EONW v7 fixture at
    `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`.
  - **Layout:** Existing consuming Scene regression is red on the former
    flush footer and green with matching hidden-main/footer bottom bounds
    across tiny-to-wide widths and scales 1/1.25/1.5/2. Short surfaces remain
    bounded. Normal hints row height is unchanged; initial grid overhead includes
    the bottom gap. A minimum-font consuming Scene regression and native
    size6/line-height1 lower-border comparison fail on the previous short row
    and pass with the row at least the existing corner radius tall. Six native raster checks at widths 320/600/1100/3000 verify
    continuous sides, one straight separator, inset lower border and matching
    lower corner geometry. Hidden-footer bottom pixels match the previous
    accepted build exactly. Normal, narrow and fractional captures are inspected.
  - **Native behavior:** Conditional/focused help, context, full viewer,
    read-only AT-SPI lifecycle, physical F6 and Alt+Shift+B pass. Visibility
    restores the same terminal grid, 107×29→107×31→107×29. Removed compact
    entries remain in full help. No shortcut bytes or workspace actions precede
    ordinary terminal `a`; the same terminal remains live. Private process,
    Wayland and DBus cleanup pass.
  - **Limits:** Eon owns composed package and installed adoption. No macOS,
    performance or new screen-reader workflow claim. Exact commands, captures,
    runnable whole-frame raster regression and cleanup:
    `~/.local/state/eon/proofs/eon-hints-rounded-bottom-2026-10-12/`.

## Rules

- Each contract uses one `## VEN-CN — Name` heading and the required fields
  `Status`, `Consumer`, `Trigger`, `Result`, `Important failures`, `Owner`,
  `Boundary`, and `Proof`.
- Add optional `Consumes` and `Open proof` fields when applicable; nest
  `Environment` and `Evidence` under `Proof`, and use nested bullets instead of
  prose table cells.
- Contract IDs are stable and repository-qualified. Never renumber or reuse an
  ID; mark an explicitly removed contract retired.
- Only current user-visible behavior, correctness boundaries, ownership
  invariants, and cross-repository interfaces belong here.
- Every implementation Bead names contracts it changes, proves, consumes,
  hardens, or preserves and maps consumed Orbit contracts to exact revisions.
- Historical execution evidence belongs in Beads and Git, not this current-state
  index. User-visible chronology belongs in `CHANGELOG.md`.
- Later work touching a proved owner reruns its indexed checks and advances the
  proof revision or records the remaining gap.

## VEN-C25 — Native Eon version label

- **Status:** Proven on native x86_64 Linux Wayland; restored header order
  accepted 2026-10-13.
- **Consumer:** Eon Runtime and native workspace users.
- **Trigger:** A workspace client receives an optional supervising Eon version
  through `--eon-version`.
- **Result:** One read-only `EON <version>` label in regular terminal monospace and
  softened cyan joins the existing top row, before shortcut help when visible
  and before close. Its full version is available through hover and a static AccessKit
  label; it adds no focus target or action and is not a window-drag target.
  Width fitting hides it before required controls and active-tab reservation.
- **Important failures:** Absent input means no label; malformed or standalone input is
  rejected. Existing workspace/terminal behavior and footer bounds remain owned
  by VEN-C21/C24.
- **Owner:** Runtime supplies immutable product identity; Venus owns validation
  and native projection.
- **Boundary:** No EONW change, widget framework, version discovery, polling,
  dependency or platform expansion.
- **Restored header-order proof:** Source `3b9f57820ec2cfcc1cab691a765ec6ff68e5111b` places the
  version before shortcut help, close last, with matching static AccessKit order.
  The existing consuming matrix is red before and green after the change,
  covering widths 100–3000, scales 1/1.25/2, 1/10 tabs, either/both quota
  providers and visible/hidden help. Controls, tabs and terminal bounds remain
  unchanged. Full fmt, locked check/test, strict all-target Clippy and debug
  build pass (154 passed, 5 existing ignored). Private Sway 1.12/software Vulkan
  observations at scales 1 and 1.25 verify order, moved-help pointer activation,
  hover, narrow/absent hiding, static badge and ordinary input to the same Orbit
  terminal. Captures inspected; private processes and DBus cleaned. The fixture
  uses canonical EONW v8 `2f9630723254ad7d9401ae40e459cef1fbfe8a17` and the
  unchanged accepted Orbit `427cf2d074cd1db905a2cc2602a5475065d1c790`.
  An initial fixture directory permission failure was corrected before native
  acceptance. No production correction was needed. Evidence:
  `~/.local/state/eon/proofs/eon-version-order-restore-2026-10-13/native/`.
- **Earlier header-order refinement proof:** Source `8df32d93e2bd7455ff9bbcea7571f36efda99da3` places shortcut help
  before the version label and close after it, with matching AccessKit order.
  The existing consuming width/scale/quota/tab matrix is red before the change
  and green afterward, including complete-footer omission of help. Full fmt,
  locked check/test, strict all-target Clippy and debug build pass (152 passed,
  5 existing ignored). Private Sway 1.12/software Vulkan native observations
  at scales 1 and 1.25 confirm order, static identity and pointer activation of
  the moved help button. Narrow/absent identity hiding, no badge action/drag/input,
  and subsequent ordinary input to the same Orbit terminal pass. Captures are
  visually inspected and private processes/DBus cleaned. One disposable test
  script naming collision was corrected; no source correction was needed.
  Evidence: `~/.local/state/eon/proofs/eon-version-order-2026-10-12/native/`.
- **Presentation refinement proof:** Source `13c35c04dd0aa9511712d6cc6627bd4b03f19215` uses
  regular terminal monospace and #7CC4D8 cyan. Full fmt/check/test and strict
  all-target Clippy pass (152 passed, 5 existing ignored), as does debug build.
  The existing emitted-glyph grid check includes the ASCII version label;
  width/AccessKit checks preserve tab/control/terminal geometry and drag space.
  Private native Sway/software Vulkan observations at scales 1 and 1.25 show
  complete supplied identity and hover, narrow/absent hiding and no label
  action, drag or PTY input. Ordinary input reaches the same surviving terminal;
  captures visually inspected and private processes/DBus cleaned. Evidence:
  `~/.local/state/eon/proofs/eon-version-badge-style-2026-10-12/native/`.
- **Proof:** Source `29199f67c79d8b43b0304df36fd64649b8d89fdc` passes fmt, locked check/test,
  strict all-target Clippy and debug build (152 passed, 5 existing ignored).
  - **Environment:** Private Sway 1.12 headless/pixman, software Vulkan,
    native x86_64 Linux Wayland, scales 1 and 1.25; accepted Orbit
    `427cf2d074cd1db905a2cc2602a5475065d1c790`, canonical EONW v7 codec
    `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa` and its fixture producer.
  - **Evidence:** Launch red/green checks reject malformed, duplicate and
    standalone identity. Layout/AccessKit checks cover widths 100–3000,
    scales 1/1.25/2, quota presence and 1/10 tabs, preserving tab, control,
    pane and terminal geometry, drag reservation and static no-action labels.
    Native captures show supplied `9.8.7-alpha.2+proof` rather than Venus's
    `0.1.0`, full hover text, hidden identity at 320 pixels and at absent input,
    and fractional rendering. Clicking/dragging the label sends no workspace
    action or PTY byte and does not move the window; ordinary input reaches
    the same surviving Orbit terminal after presentation replacement.
    Captures are visually inspected; private processes and DBus state cleaned.
    Evidence: `~/.local/state/eon/proofs/eon-version-bar-2026-10-12/native/`.
  - **Limits:** Eon owns exact installed adoption. No performance, macOS or
    new screen-reader workflow claim.

## VEN-C26 — Claude quota beside Codex

- **Status:** Proven for the native Linux Wayland consumer; installed Eon
  composition remains Eon's responsibility.
- **Trigger and result:** A complete canonical EONW v8 snapshot supplies an
  optional Claude observation. Venus shows a read-only C monogram chip with
  the available five-hour/seven-day remaining percentages. Hover and a static
  accessible label name Claude, every window, freshness and reset interval as
  of the observation. Stale text retains `old` even in compact presentation.
- **Ownership:** Runtime alone supplies source facts, age and invalidation.
  Venus uses its existing Scene, renderer, hover and AccessKit owners. Both
  provider chips remain in the existing row; full labels become compact, then
  trailing optional chips disappear before required tabs, controls or drag
  space. Codex's mark, labels and provider policy remain unchanged.
- **Failure:** Absent facts have no geometry or accessible node. Workspace
  transport loss removes both observations while keeping the last useful
  topology. Quota-only changes do not replace attachments, scroll positions or
  navigation. Neither chip has an action or keyboard focus target.
- **Consumes:** Canonical EONW v8 at Runtime
  `2f9630723254ad7d9401ae40e459cef1fbfe8a17`; existing Orbit codec
  `6bc269c40b18f08b95778939518f77556ba91c67` and accepted Eon Orbit source
  `427cf2d074cd1db905a2cc2602a5475065d1c790` stay unchanged.
- **Boundary:** `eon-claude-quota-eon-bar-zm5f` owns this slice. No
  provider parsing, credential access, client polling, new module, dependency,
  framework, row or platform promise.
- **Proof:** Source `0d2317fdae0d6fea7af48c43a5b525aa8d09b70f` passes
  format, locked check/build, 154 ordinary tests (five existing native checks
  ignored), and strict all-target Clippy. The two-provider Scene/AccessKit
  regression is red before implementation and green for both/one/no providers,
  fresh/stale facts, widths 100–1800 and scales 1/1.25/2. Existing Codex checks
  retain their labels and the workspace failure check withdraws both providers.
  - Private Sway 1.12 headless/pixman with host lavapipe Vulkan, native x86_64
    Linux Wayland scale 1, accepted Orbit and canonical v8 fixture: both chips
    render full/compact labels, then the trailing chip yields, then both hide.
    Claude-only, absent, stale and transport-loss observations preserve required
    targets. Hover/AT-SPI identify Claude and reset details; chips have no action
    or focus target. Pointer interaction emits no action or PTY input; ordinary
    `a` reaches the same surviving terminal. Captures are visually inspected.
  - Claude percentages/resets come from a real 2.1.159 Pro status-line response;
    fixture freshness states and Codex values are controlled substitutes. This
    proves native consumption, not Runtime collection or installed composition.
    Evidence: `~/.local/state/eon/proofs/eon-claude-delivery-2026-10-13-43q58bbf/venus-native/source-2/`.
    Initial Sway launch failed from a host/Nix library-path collision before
    Venus started; the corrected isolated environment passed. Private processes,
    runtime and DBus state are cleaned; all 74 observed ambient product
    identities survive. No hardware GPU, macOS, performance, physical input or
    screen-reader workflow qualification is claimed.

### VEN-C22/VEN-C26 — Optional-reset consumer refinement

- **Status:** Proven at source `59482a93ab78b29789fecb9a37b101152b5be8c2`.
- **Contract:** Consume canonical EONW v9 at
  `d0a184bb4d45d3e50787f7c17606df652727fb9a` with unchanged quota chips.
  Fresh and stale provider windows may lack a reset. Show their actual remaining
  percentage and state; hover/accessibility say reset time unavailable.
  Historical codec validators remain unchanged. No provider mechanism, new
  field, presentation owner or platform is introduced.
- **Preservation:** Orbit `427cf2d074cd1db905a2cc2602a5475065d1c790` and its existing
  codec remain exact; preserve restored version/help order at accepted Venus
  `3b9f57820ec2cfcc1cab691a765ec6ff68e5111b`.

- **Proof:** Format, locked check/build/tests and strict all-target Clippy pass.
  The consuming missing-reset check preserves fresh/stale labels and unavailable
  reset details. Exact installed Eon native proof uses the same Venus source:
  private Sway 1.12/pixman, Mesa 26.1.2 lavapipe, Linux Wayland scale 1.
  Real Claude renders 5h 100% without reset and 7d 98%; real Codex renders weekly
  86%. Hover/accessibility retain supplied facts, widths 1100/500/180 preserve
  required controls, and a failed Codex refresh shows `old` without disrupting
  Claude, topology or input to the same terminal. Version/help order is preserved.
  Eight ambient product identities survive; private processes/Wayland/DBus and
  credential copies are removed. This is software-GPU scripted input, without
  hardware GPU, physical input, reader workflow or macOS qualification.
  Evidence: `~/.local/state/eon/proofs/eon-codexbar-delivery-2026-10-14-nwwclgck/`.

### VEN-C26 — Claude splat icon

- **Status:** Proven at source `d0642ed9fc190539811168553c972107336fa213`.
- **Contract:** Replace the quota chip C monogram with Claude's recognizable
  splat in its original terracotta orange (`#D97757`), preserving its logo bounds,
  text, both available windows, hover/accessibility identity, freshness, compact
  and hidden states, and static no-action behavior.
- **Owner:** Venus's existing custom-glyph renderer and bundled alpha mask.
  No provider, protocol, dependency, module or platform change.

- **Proof:** The extended custom-glyph regression is red before Claude support
  and green at 16/20/32/104 pixels, preserving both original masks at source size.
  Full format, locked check/build/tests and strict all-target Clippy pass.
  Installed Eon and EonTerm Nix builds pass with canonical child-owned notices.
  Private Sway 1.12/pixman, Mesa 26.1.2 lavapipe native Wayland at scales 1 and
  1.25 shows the splat with real Claude 5h 100% / 7d 98% and unchanged Codex mark.
  Wide/compact captures contain 17 original-color pixels and fractional capture
  40 (`#D97757`); full/compact/hidden layout, hover and static accessibility pass.
  Ordinary input reaches the same surviving Orbit terminal. Captures are visually
  inspected; ambient product identities and original credential bytes survive.
  Private processes, copied credentials, Wayland and DBus state are removed.
  Evidence: `~/.local/state/eon/proofs/eon-claude-icon-2026-10-14-kz3_jctj/`.
  No hardware GPU, physical input, reader workflow or macOS qualification.
