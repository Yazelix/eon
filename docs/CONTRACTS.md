# Eon contract index

This is the canonical current state of Eon product behavior, ownership, proof,
and remaining limitations. Owning Beads and Git retain execution history;
`CHANGELOG.md` retains accepted user-visible chronology.

## Status

- **Planned:** accepted intent without implementation evidence
- **Candidate:** implemented and mechanically verified without an accepted final
  proof revision
- **Partially proved:** one exact useful slice is proved with a required gap
- **Proven:** an exact check, component set, platform and artifact cover the
  contract
- **Retired:** explicitly replaced or removed; its ID is never reused

The [component manifest](../components/eon-alpha-v3.json) owns selected source
and compatibility identities. EON-C4 records the external runtime composition;
EON-C7 records the distinct tagged public alpha. Every proof below applies only
to its named source, artifact, environment and surface; it does not promote a
later pin or another platform. Profile refresh preserves live supervisors and
terminals; existing work retains its exact runtime until an explicit restart.

## Rules

Product vocabulary follows the [naming boundary](ARCHITECTURE.md#naming-boundary):
workspace, tab, pane and terminal. Human help, output and diagnostics use
terminal for each persistent Orbit-backed instance. Technical session IDs,
socket names, JSON fields and protocol identifiers retain their spellings.

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
    so it promises neither a new window nor a new terminal; live surfaces retain
    terminal-authored titles.
  - Compatible terminal output leaves scrolling, selection, and full-Eon tab
    and pane focus usable between repaints. Orbit continues parsing output and
    answering terminal queries while the user reads retained history; history
    bounds, geometry validation, and synchronized presentation remain intact.
    Venus shows a bounded scrollback-position indicator; activating its visible
    ReturnToLive control returns to live rows without Eon owning viewport state.
    Orbit owns words at two clicks, space/tab-delimited punctuation spans at
    three and logical lines at four or later, with native drag and frozen copy.
  - Full Eon can invoke the manifest-pinned Anima executable for the optional
    fresh-workspace welcome step and `eon anima [style] [child options]`.
    The direct command uses the caller's terminal and returns the child's
    exit result without starting or attaching a workspace.
  - Optional `[terminal] cursor_trail_color` is forwarded unchanged on new or
    reopened surfaces. Omission emits no override; Venus chooses one random
    preset per surface. Venus owns `random`, `preset:<name>` and
    `custom:#RRGGBB` parsing, colors and rendering; Orbit cursor authority remains.
    Native admission rejects an invalid choice before a new terminal starts;
    rejected reopen preserves the existing terminal and command.
  - Standalone `eon -h` and `eon --help` print grouped command descriptions and
    examples to stdout and exit successfully without starting children or
    preparing configuration/runtime state. Terminal output uses color unless
    `NO_COLOR` is nonempty or `TERM=dumb`; redirected output has no ANSI escapes.
    Invalid syntax remains a concise stderr failure pointing to `eon --help`.
    Child arguments, including Anima help, retain their existing forwarding.
  - Eon owns the cursor-tail duration default `1.5` and the optional
    `[terminal] cursor_trail_duration` multiplier in the same config file.
    Finite values from `0.25` through `4.0` apply on new/reopened Eon and EonTerm
    surfaces. Invalid duration fails before children; live surfaces retain their
    startup snapshot. Venus consumes the launch value and owns bounded motion,
    including healthy pane/tab/layout continuity. No separate child configuration.
- **Important failures:** Missing, malformed, incompatible, replaced, or
  non-ready components fail before publishing a usable workspace or terminal.
- **Owner:** Eon graph validation, icon selection and desktop metadata; Eon
  Runtime implements launch orchestration. Nix derives/installs artifacts;
  child components own their internal readiness and state.
- **Consumes:** The versioned Eon component manifest, Orbit management/ORBS, and
  Venus native launch contracts.
- **Boundary:** Eon does not infer compatibility from executables, store paths,
  process names, or moving branches.
- **Product terminology proof:** Accepted on x86_64 Linux, 2026-10-07, under
  `eon-workspace-terminal-terminology-f65b`, from Eon
  `cf1dda4b48eecf796f2d991424f91aa19ce098ac` and Runtime
  `7c886d87551225c7581892901f95ac1b853952c7`. EONW remains
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, with exact codec tree
  `8409c419a3496d6f01eeef65295f5d95b943313e`; Orbit and Venus remain
  `6bc269c40b18f08b95778939518f77556ba91c67` and
  `a27aad23822d3bd0b0e467ffc32c578bbde8b00d`.
  Prepared-source locked Rust tests (115), formatting, Clippy, the exact package
  gate and complete `path:` flake check passed. Profile refresh selects
  `/nix/store/9s4mnhbcpnrqykp1fkpld1b6gcqwz8rl-eon-0.1.0` and
  `/nix/store/hvmafa29k3z4z46p05wazymfagx6zvzv-eonterm-0.1.0`.
  Installed piped/80-column PTY help, color gates and launcher metadata use
  terminal wording. Private Sway 1.12 headless/pixman with Mesa Vulkan proves
  human workspace/generation output, declined Stop confirmation, and both
  products' detach messages. Full-Eon reopen retains exact `session-N` JSON
  identities, endpoints and Orbit process identity. Technical fields, codes,
  socket names and codec package bytes remain unchanged. Earlier build attempts
  hit lifecycle deadlines under disk load; clean serial Rust and delivery runs
  passed with unchanged deadlines. Only Eon/EonTerm profile elements changed;
  original user processes remain alive at exact prior identities. This proves
  product wording and its installed consumers, without extending native UI,
  platform or lifecycle guarantees. Existing work retains its prior runtime.
- **CLI help proof:** Accepted on x86_64 Linux, 2026-10-07, under
  `eon-readable-cli-help-5yh4`, from Eon source
  `43cd92879b20dca21c3d5a12fa16060c267bc129` and Runtime
  `9780ccff908a7e5e506351ad017f219697e17681`; the remaining component graph
  is unchanged. Prepared-source locked workspace tests (115), formatting and
  Clippy passed, as did the exact package gate and the complete `path:` flake
  check with one build job and two cores. Refreshed installed profiles resolve to
  `/nix/store/y01wzxbgwnqsqs8jamq1hryvbiyd3pgq-eon-0.1.0` and
  `/nix/store/hkiy054zj6hap9fya7adxlkd62w4hd8d-eonterm-0.1.0`.
  Both installed help flags returned 0 with empty stderr, 43 identical plain
  lines, aligned descriptions and a 79-column maximum. An 80-column stdout PTY
  proved color and the empty/nonempty `NO_COLOR` and `TERM=dumb` gates; piping
  stayed escape-free even with terminal stdin. Private invalid/missing config
  and runtime state remained untouched. Captured ANSI was reviewed on
  representative dark/light palettes. Anima help forwarding and EonTerm syntax
  passed; three baseline live child identities and unrelated profiles stayed
  unchanged. This CLI proof does not extend existing native presentation proofs.
- **Assembly boundary:** `product.rs` supplies fallible validated component
  facts to the current runtime; graph failures precede runtime ownership effects,
  including `window new` parent-directory creation. The independent producer
  owns the consumed runtime. See the
  [runtime input contract](ARCHITECTURE.md#ownership-and-invocation).
- **Proof:** Eon `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`,
  `eon-accept-live-output-input-nxh`.
  - **Environment:** Installed x86_64 Linux, isolated Sway 1.12 native Wayland;
    Orbit `91999d79546422b49bdbc124166a65859d0bd872`, Venus
    `e13970e90289d0d86f0adcbf350e4b9c1d5e5219`, ORBF v1 / ORBS v10 / EONW v4.
  - **Evidence:** Manifest/lock identity, both Nix package/flake routes and
    profile equality; artifacts
    `/nix/store/jy82yqqn0sinvjkl9n405742q8483f5y-eon-0.1.0` and
    `/nix/store/pd5c82936b6z24ss03dhdvrb0zmq8v90-eonterm-0.1.0`.
    Installed EonTerm retained anchored history and terminal-query replies
    during appends, split 16 KiB DEC 2026 batches and active-screen redraws;
    cell/word/line click-drag, both clipboards, frozen copy and return to live
    passed. Full Eon's two tabs/three panes accepted native directional,
    identity and pointer focus during output. No performance bound is claimed.
  - **Launch and icon:** Ready base `7cbcf1d4ce8ac186dc3ceff48240f04c437709af`,
    icon selection `32936b92e9c20a000d21f123869e0b749ff39618` with master SHA-256
    `33ec3062f72a732290dcd6c6f40a2d5f535d6a7cbfcaf7455a0a5c4f03a52e0d`,
    and exact launcher action `32c768e0d9a62984640a516c34d0cb0db16e8adb`.
    Earlier continuous-output/synchronized-scroll evidence is retained in nxh
    and Git; the current exact external composition is proved in EON-C4.
  - **Cursor configuration:** `eon-compose-yazelix-cursors-bcc` at Eon
    `06402130ddccc3a02298eb3131fbe9f53d9b99a0`, Venus
    `73195f5812cdc8f32841eab644c1bf4b3af9f756`, EONW v5: focused launch/rejection,
    locked Rust and both Nix/flake routes; installed private Sway 1.12 / Mesa
    26.1.2 lavapipe proves omission, `preset:forest`, `custom:#12ABCF` and
    invalid `preset:volt` rejection before Orbit. Artifacts
    `/nix/store/2hwrjd93r0qy84r42xkmq82il50i9w3x-eon-0.1.0` and
    `/nix/store/2fk1w9rvkc469ahgi5grr6p89k2l68gm-eonterm-0.1.0`.
    Eon `b8cb7feceb6df71868d9de46a0bd843d68a14a14` / Venus
    `f6c1ebdb16df2960077dbd88ab6728a921b789d8` proves the visible black fallback
    outline in installed EonTerm
    `/nix/store/92cfax22s9f3v5n3sqjmkspkpzgakhj9-eonterm-0.1.0`.
    Venus VEN-C1 owns rendering, explicit Orbit-color precedence and the
    block body's existing 0.55 alpha; these are distinct scoped proofs.
  - **Child presentation integrations:** Upward-selection acceptance at Eon
    `00a180c5940f0af84fc84cd0e4134a01824a61a7` / Venus
    `773c6bab7d3e21e0b0d7d942c9bba72260a08b42` / Orbit
    `b6cecf8f2ee35570b41cfdc578b095889d917fe2` (ORBS v13)
    on private scale-1 Sway copies
    78 contiguous Unicode lines in installed
    `/nix/store/p3wslkacd971fpw3vyghn6cx32kicizn-eon-0.1.0`.
    `eon-accept-scrollback-position-j7f` at Eon
    `a9841c740d048e52f199cfc7ad965404dacf0cb1`, Orbit
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5` (ORBS v11), Venus
    `247dcb2dc853e604d51ac727471e1e65e1535f1d` proves 10→16 scrolled rows,
    then no indicator at live bottom in installed
    `/nix/store/h1d86xgn75mp5qny96iz5ysbkhp4gxx3-eon-0.1.0` and
    `/nix/store/bk39s868z0jj1grm1psaq2c4k1mjw3dd-eonterm-0.1.0`.
    `eon-deliver-return-to-live-eon-14g` at Eon
    `f4dc4824d422d2ec0d926e45036c35e2f95522ed`, Orbit
    `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` (ORBS v12), Venus
    `ead00dfcb3510eb595173c02f4627d2cd8b36b0a` proves 10→16 retained rows and
    visible native ReturnToLive clicks in installed
    `/nix/store/ils9na81wh4gd22dk8cla7095wy0y3gn-eon-0.1.0` and
    `/nix/store/3ldfxmkyjvq1xymzpi74l5dziysfmgic-eonterm-0.1.0`.
    Both use x86_64 Linux, private Sway 1.12 headless/pixman, Mesa 26.1.2
    lavapipe at scale 1. Hidden/stale hit testing remains Venus's source proof;
    Orbit/Venus retain state, rendering and hit-test authority.
  - **Limits:** These checks do not promote later pins, fractional scale,
    broader compositors, screen-reader use or non-systemd support. EON-C7
    records public-alpha qualification; EON-C18 retains picker capture limits.

## EON-C2 — One component compatibility graph

- **Status:** Proven
- **Consumer:** Every Eon distribution and runtime launch path.
- **Trigger:** A product generation resolves or validates its component set.
- **Result:** One versioned manifest defines component identity, compatibility,
  and abstract artifacts for every channel without encoding launch policy.
  Anima is a pinned tool in that graph, with one executable artifact.
- **Important failures:** Unknown components, duplicate identities, incompatible
  protocol versions, malformed revisions, or channel-specific graph drift fail
  validation.
- **Owner:** Eon's component manifest and validator.
- **Boundary:** The manifest does not own process lifecycle, platform launch
  mechanics, package-manager policy, or child state.
- **Package selection:** Separate runtime and EONW package records use the
  existing schema-3 graph and exact per-component checks; unchanged codec
  identity permits distinct source commits. The
  [package selection contract](ARCHITECTURE.md#independent-package-selection)
  is enforced by the shared Nix preparation gate; EON-C4 records cutover proof.
- **Proof:** Eon `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`;
  exact external package selection and source preparation in EON-C4.
  - **Environment:** x86_64 Linux Nix alpha.
  - **Evidence:** Manifest parser/compatibility checks, installed graph
    consumption by Eon and EonTerm, and historical Eonova consumption;
    manifest base `234ad77ced4924d95400b5c39822b3fb9b928c95`, ORBS v7 refresh
    `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`. The 91c6ed5 proof selects
    Orbit `91999d79546422b49bdbc124166a65859d0bd872` and Venus
    `e13970e90289d0d86f0adcbf350e4b9c1d5e5219`, ORBF v1 / ORBS v10.
    EON-C4 names the cutover graph, artifacts and runtime-only A/B checks.

## EON-C3 — Distribution-neutral runtime inputs

- **Status:** Proven
- **Consumer:** Eon launch and every distribution channel.
- **Trigger:** A channel supplies exact component artifacts to Eon.
- **Result:** Eon receives explicit component paths, treats Nix store paths as
  opaque inputs, and invokes no Nix evaluator during normal runtime. The Nix
  full-Eon launcher supplies the pinned Anima executable as an opaque input.
- **Important failures:** Missing, non-executable, incompatible, or substituted
  artifacts fail before accepted launch.
- **Owner:** Eon supplies validated component facts; the distribution supplies
  artifacts; Eon Runtime validates and consumes opaque launch inputs.
- **Consumes:** EON-C2's component graph.
- **Boundary:** Runtime code does not construct, persist, inspect, or derive
  identity from Nix store paths.
- **Proof:** Eon `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`;
  external runtime composition in EON-C4.
  - **Environment:** Nix-built x86_64 Linux alpha.
  - **Evidence:** Exact protocol-source substitution, package checks and
    evaluator-absence checks; launch-input base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70` and ORBS v7 composition
    `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`. EON-C1 names the 91c6ed5
    component set and artifacts; EON-C4 records independent package inputs.

## EON-C4 — Thin orchestrator ownership

- **Status:** Proven
- **Consumer:** Eon, Orbit, Venus, Helix, Yazi, and Ratconfig integrations.
- **Trigger:** Eon composes or controls a product surface.
- **Result:** Eon owns product launch/topology policy, component selection,
  defaults, configuration policy, updates and distribution without duplicating
  terminal, rendering, editor, file-manager, or configuration state.
  Native desktop launches preserve host XDG preferences and use
  `EON_CONFIG_HOME` for the product configuration root.
  Eon selects Anima startup eligibility and forwards CLI arguments; Anima owns
  its styles, random choice, browsing, rendering, and terminal-mode cleanup.
- **Important failures:** A missing child contract returns to that child instead
  of being reconstructed in Eon.
- **Owner:** Eon assembly and product policy; Eon Runtime owns runtime
  mechanisms and state. Orbit, Venus and tools retain their subsystem authority.
- **Consumes:** Accepted child contracts through EON-C2's exact graph.
- **Installed cursor-tail configuration and continuity proof:** Eon source
  `1ca418d9370061b4ba5ac03c0225236ad2b1461d` selects Venus
  `a27aad23822d3bd0b0e467ffc32c578bbde8b00d` and Runtime
  `fb3c9c1b08c7a8c03ca4a8cc57628579c09bdeb4`, preserving Orbit
  `6bc269c40b18f08b95778939518f77556ba91c67` and codec
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa` (ORBF2/ORBS13/EONW7).
  - **Environment and artifacts:** x86_64 Linux, private Sway 1.12
    headless/pixman, Nix Mesa 26.1.2 lavapipe, scale 1. Prepared source
    `/nix/store/ran5w3mj2gqnxfv725d7fhx7yrxdaqnx-eon-source`;
    installed Eon `/nix/store/40bb9d041r46qiplaisd1rav5cl0w2ng-eon-0.1.0`
    and EonTerm `/nix/store/qn4g3bgb1zdaba8bly9rqwxygqai02zf-eonterm-0.1.0`.
  - **Evidence:** Prepared-source fmt/check/test/clippy (114 tests), package
    selection, both Nix products and closure/co-installation pass. Active
    profiles match the rebuilt path-working-tree artifacts. At the Eon-owned
    default `1.5`, keyboard/pointer pane/tab, layout and rapid-switch captures
    all show visible motion and settle. Installed duration rejection creates no
    fresh terminal in either product. EonTerm reopens from default `1.5` to
    configured `2.5` with the same Orbit process and terminal command. Source and
    binary hashes, commands, logs, setup negatives and captures are retained in
    `~/.local/state/eon/proofs/eon-cursor-tail-duration-1ca418d-2026-10-07/report.json`.
  - **Limits:** This advances EON-C1/C2/C4 only for this product configuration
    and exact child adoption. Four existing user component processes remain
    unchanged. Live user terminals retain their prior runtime until explicit
    restart. No macOS, broader compositor, latency, promotion or release proof.
- **Installed viewport-request adoption proof:**
  `eon-adopt-pending-viewport-owner-9k4y`, 2026-10-06 path working tree based
  on Eon `3c8d1e0fe1a3c3bd3cec88c32d94ce775771cdb8`, prepared source
  `/nix/store/46ff5h10am2clgg7aqqq7shxclc4dimn-eon-source`, selects Venus
  `c7a0d07612e710854275a31500aaec651cc50a6f`. Orbit `6bc269c`, Runtime
  `a57e105`, EONW7 codec `b8f18b4` and ORBF2/ORBS13 remain exact.
  - **Environment and artifacts:** x86_64 Linux, private native Wayland Sway
    1.12 headless/pixman, Mesa 26.1.2 lavapipe, scale 1; installed Eon
    `/nix/store/952w0qbh5446hm0vbnlp6rv529hmy383-eon-0.1.0` and EonTerm
    `/nix/store/0iv18b4yqf5q2ka95yrv4wlbg4wl81c4-eonterm-0.1.0`.
  - **Evidence:** Manifest/lock identity, both Nix products and EonTerm
    closure/co-installation pass; active profiles equal the artifacts. Native
    wheel, held upward selection/autoscroll, wheel after release and ReturnToLive
    pass in both products with real Orbit/Venus and fresh clipboard oracles.
    Full Eon's workspace/input isolation, user process/profile preservation and
    disposable-generation cleanup pass. Exact commands, test results, captures,
    setup negatives and fresh-directory replay instructions are retained in
    `~/.local/state/eon/proofs/eon-adopt-pending-viewport-owner-2026-10-06/REPORT.md`.
  - **Limits:** This proves installed EON-C1/C2/C4 delivery of the accepted child
    revision. Queue saturation and pending-response ordering retain the child's
    scoped proof. No accessibility, macOS or wider-platform proof is added.
    Existing terminals keep their prior runtime until an explicit restart.
- **Child adoption proof:** `eon-adopt-click-ladder-and-shortcut-spacing-h1nv`,
  Eon source `0d02d4b2e6310f863de6bd3d0164c7c3ec08811d`, selects Orbit
  `6bc269c40b18f08b95778939518f77556ba91c67` and Venus
  `c02e371c44909aa64cb3f2d77284b8a8ab2b4cc5`, retaining Runtime
  `a57e105a5da87d9c65402c1c4c8ea8b5dc8adc45` and EONW7 at
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa` with ORBF2/ORBS13 unchanged.
  - **Environment and artifacts:** x86_64 Linux, private native Wayland Sway
    1.12 headless/pixman, Mesa 26.1.2 lavapipe, scale 1; installed Eon
    `/nix/store/ij6ad279x1pjmgwwk992ig6rm1fckq84-eon-0.1.0` and EonTerm
    `/nix/store/n2izc97m5i1zai5558ra7m7jnmg63s47-eonterm-0.1.0`.
  - **Evidence:** Prepared locked Rust fmt/check, 114 tests and strict Clippy;
    checked source preparation rejects changed Orbit codec bytes or inherited
    Cargo configuration; both Nix products and all 12 flake checks pass.
    Profile elements equal these artifacts and report the exact graph. Native
    EonTerm pairs select words/spans/fourth-and-fifth-click lines, including
    16-column soft wraps, delayed single drag, span drag during live output and
    frozen explicit Copy after replacement. Full Eon Shortcuts passes physical
    Alt+Slash, normal/compact header layout, scrolling, workspace/input isolation
    and Escape focus restoration. Five pre-existing product identities and
    unrelated profile elements survive; disposable generations stop and reap.
    Retained scripts, clipboard text, captures and logs:
    `~/.local/state/eon/proofs/eon-adopt-click-ladder-and-shortcut-spacing-h1nv-2026-10-06/REPORT.md`.
  - **Limits:** This refreshes EON-C1/C2/C4's selected-child delivery and C20's
    header spacing only. Older source-specific proofs below remain scoped.
    Existing terminals retain their prior runtime until an explicit restart.
    Child wide trailing-spacer span truncation, copy trimming and unwritten
    tab-gap limits remain; no screen-reader, macOS or wider-platform proof.
- **Boundary:** No copied child schema, hidden fork, compatibility adapter, or
  second terminal/rendering owner.
- **Assembly boundary:** `crates/eon/src/product.rs` owns concrete product inputs;
  Eon invokes exact external runtime/codec packages. The independent Eon Runtime
  repository owns the nine runtime mechanisms, parsing,
  validation, mutable state and canonical EONW. Eon retains assembly, defaults,
  graph, distribution and process exit handling. The
  [runtime boundary](ARCHITECTURE.md#runtime-library-boundary) records accepted
  producer transfer and composed package selection.
- **Proof:** External cutover in Eon
  `51666e2fcb925a7d2abde204ac402ecf83d1769f`, recorded by
  `eon-runtime-cutover-y1wy` on the exact candidate based on
  `7cc82a2a7e3737086dcbb7b0756a4ff2e5a4f6af`.
  - **Environment:** x86_64 Linux, Nix; isolated Sway 1.12 native Wayland / Mesa
    Vulkan. Runtime and codec `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`,
    Venus `bbf4cf41b289f441683eb7a3f225a26a4d3edacc`, Orbit
    `b6cecf8f2ee35570b41cfdc578b095889d917fe2`, ORBF v2 / ORBS v13 / EONW v7.
  - **Evidence:** Locked Eon/manifest Rust route (39 tests), both Nix products,
    flake checks, package-selection/source-normalization and slim-closure plus
    co-installation checks. Installed artifacts
    `/nix/store/jlciixci56c353yijns5lidz1za1sp6a-eon-0.1.0` and
    `/nix/store/2r1h7h5dz8kl0dd80872bnxvm2rxr91c-eonterm-0.1.0` use
    `g1-b5f3ee783f374176d716de3488946ef4`.
    Both old products survive profile refresh; retained exact executables
    reattach, and the current client inspects and owner-stops old disposable
    terminals while new work remains alive. EonTerm rereads config without
    replacing its terminal. User terminals and unrelated profile elements remain.
  - **Package boundary:** Standalone producer acceptance in
    `eon-runtime-producer-68hq` retains runtime tree
    `3dcfef6bf31758233f2a33eb31d5184ebeae9843` and codec tree
    `8409c419a3496d6f01eeef65295f5d95b943313e`, with 50 producer tests and
    35 real-Eon process tests. Product, runtime and codec versions retain
    separate owners; no reverse dependency or local production copy remains.
    `eon-runtime-venus-rebind-tbmn` and `eon-runtime-package-identity-dmbj`
    retain the one-time provider rebind and fail-closed package gate evidence.
  - **Independent revision check:** Mechanically verified runtime-only
    `c117b5eca2cee0125c8d7201e5f69cab13c1c45b` with fixed codec/Venus passes
    locked prepared Rust and both Nix routes, producing a different generation.
    This is not an accepted production pin or a separate native proof. Codec
    drift under unchanged labels fails through both production gates before
    substitution. The private A/B check consumes canonical inputs.
  - **Child allocation ownership:** `eon-integrate-venus-cell-buffer-memory-o5b`
    at Eon `7b7382395dad6608fdda40814c2dbf33806dd0e3` selects Venus
    `541a9cb43c155b8b97069904593dc81c73682613`; the child owns the correction.
    Manifest and both Nix builds pass; installed Eon
    `/nix/store/5bw42dbymran4nwv5p64izm7nnfp45ha-eon-0.1.0` and EonTerm
    `/nix/store/nqdpbgmp0a1w102s22ldnf8r97vgm1pg-eonterm-0.1.0` pass three
    paired fresh-process runs per product on x86_64 Pop!_OS 24.04 / private
    Sway 1.12 headless/pixman / Mesa 26.1.2 lavapipe, scale 1. A fixed 10,030-row
    Unicode workload at 120×30 cells, terminal-query barrier and three-second
    settle gives median paired PSS savings 4.48 MiB (4.20–4.70) for Eon and
    4.66 MiB (2.64–4.73) for EonTerm; all 12 captures match their baseline bytes.
    Ranges are observed minima/maxima, not confidence intervals.
    Exact old/new packages, raw measurements and reproduction remain in that
    issue's proof archive. Small warm-host samples with unfrozen shared mappings
    prove no GPU, throughput, long-term growth or history-efficiency claim and
    do not replace the COSMIC/Intel benchmark.
  - **Native launch boundary:** Eon
    `be9d0e37c7d0029d6832ff609082faa885280762`, Orbit
    `91999d79546422b49bdbc124166a65859d0bd872`, Venus
    `d1223463b2b513c04242df50a4bd948d0533cfde`, ORBF v1 / ORBS v10 / EONW v4:
    launcher-environment regression, locked Rust/Nix checks and installed
    private Sway 1.12 / Mesa 26.1.2 / GIO 2.80.0 proof in
    `/nix/store/8r53v42plymxhys76mv582za20l382qa-eon-0.1.0` and
    `/nix/store/q441sjfvgvkq00yim4gd97f540pmnbg5-eonterm-0.1.0` preserves
    host XDG handler preferences and distinct product config. Exact copy,
    registered-handler dispatch, native failure and captured input pass;
    handler delivery does not prove browser content loading. Input correction
    `07867f99d176012a9d34a61aa37b5de9ce0888af` / Venus
    `cf3a9169f2cd95d42c689134a52d51d9147ff37c` retains key press/repeat/release
    isolation; its installed artifacts remain in Git at that proof.
  - **Limits:** Source transfer and package equality change no wire/API,
    native platform claim or child authority. Presentation still requires exact
    graph identity; old work may need its prior executable. Earlier assembly,
    seam and composition proofs remain in their owning Beads and Git.

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

## EON-C7 — Native Linux Wayland and Apple Silicon macOS

- **Status:** Partially proved
- **Consumer:** A user launching a supported Eon distribution on an approved
  native platform.
- **Trigger:** Build, install, or launch an approved Eon channel.
- **Result:** Eon uses native Wayland on x86_64 Linux and the native macOS host
  on `aarch64-darwin`, without requiring a particular init or service manager.
  Both consume the same component graph, orchestration path, workspace
  contracts, and Nix-only alpha channel while platform mechanics remain with
  their existing owners. Each channel names its proved architectures and launch
  environments.
- **Important failures:** Systemd-only lifecycle assumptions, X11 fallback, a
  second component graph, hidden child substitution, or an unproved
  architecture or compile-only runtime claim is rejected.
- **Owner:** Eon platform and distribution policy; child-owned kernel
  capabilities remain explicit child contracts.
- **Boundary:** Current accepted support remains x86_64 Linux native Wayland.
  Orbit `ORB-C14` has native Apple Silicon proof; Venus `VEN-C16` has partial
  M1 proof, including physical Scaleway runs. Apple Silicon macOS remains
  unsupported until Venus and full-Eon native proofs pass. X11, Xwayland,
  Intel macOS, signing, notarization, direct bundles, and public macOS
  distribution remain outside this contract.
- **Proof:** Linux qualification in `eon-qualify-linux-public-alpha-8ue`;
  the combined Linux/macOS contract is not proved.
  - **Environment:** Clean `nobody` account, x86_64 Linux, isolated Sway 1.12
    native Wayland, pixman / Mesa lavapipe; no checkout, user credentials or
    readable netrc. Candidate Eon
    `3b4f388e63f5f9fc5e234727e4f8b52ea4c9ca74`, Orbit
    `b6cecf8f2ee35570b41cfdc578b095889d917fe2`, Venus
    `773c6bab7d3e21e0b0d7d942c9bba72260a08b42`, EONW v7.
  - **Evidence:** Manifest, locked Rust and flake checks; public-source install,
    picker/help, tab/pane creation/movement/close, directory inheritance and
    retargeting, invalid-path failure, detach/attach, diagnostics and Stop.
    Supervisor loss preserves three full-Eon terminals and EonTerm's one terminal;
    fresh supervisors adopt them without duplicates. Real previous EONW v6
    Stop with current unstarted, then current `stop all`, pass. Candidate
    artifacts `/nix/store/y4bi72b9rb9yyxcb247gi87w7zkdwclk-eon-0.1.0` and
    `/nix/store/lwii3cff1i8z3v7lqjg6zzf38wcbcpjn-eonterm-0.1.0` use
    `g1-c432a14b396ca1060a2d65be3d7f28ba`.
  - **Published slice:** `linux-alpha-2026-09-24` points to Eon
    `3550368ac2a7ecd689df75a75aff7007523b7dbf`; fresh public-tag install gives
    `/nix/store/kkilnkvxcjymxswy3bz5ryrjd3qwmf1v-eon-0.1.0` with the same
    graph and native-tested binary SHA-256
    `1a2bb60fb17ead6ecdbc8658dce5af8b21b002e9f06c33076fe0ea49e6ad2f61`.
    The tag predates edge Anima, independent windows and external runtime
    composition; their proofs belong to C18, C23 and C4.
- **Open proof:** Current-alpha AT-SPI application tree was unavailable in that
  clean account; current labels and screen-reader workflows remain unproved.
  Wider compositors, fractional/HiDPI coverage, proprietary NVIDIA and installed
  non-systemd behavior remain qualified. C19 proves only its named initial-grid,
  typography, IME and AT-SPI slice. Machine-restart recovery is not supported.
  Native Apple Silicon proof follows Orbit ORB-C14, Venus VEN-C16, then
  `eon-prove-full-eon-apple-silicon-macos-t8o`; compile/evaluation is insufficient.

## EON-C8 — Durable tab and pane workspace

- **Status:** Partially proved
- **Consumer:** One local Eon workspace user.
- **Trigger:** Launch, create or close a tab, create a pane, traverse focus,
  reorder the active tab or selected pane, receive terminal exit, or recover
  accepted same-boot runs.
- **Result:**
  - A workspace contains horizontal tabs with vertical accordion panes. Each
    pane views an independent persistent terminal. When the pane stack is
    visible, exactly one pane is expanded.
  - Tabs use stable `tN` identities. Each tab header shows its number plus the
    leaf, `~`, or `/` derived from Eon's authoritative launch directory; actions
    retain `tN`, and accessibility includes full launch-path context. Venus
    renders pill-shaped, separated tabs sized to shaped labels, capped near 280
    logical pixels at default typography. Long names use middle ellipsis;
    narrow tabs retain their number whenever it fits. Hover previews the launch
    path, and wheel scrolling works across the whole strip, including gaps.
    Selected fill and brighter text identify the active tab without an underline;
    keyboard focus adds a separate rounded outline.
  - Each pane is identified as `pN`, two ASCII spaces, and a compact working-
    directory label; home uses the packaged marker, descendants use `~/`, and
    external paths remain absolute.
  - On each new or reopened full-Eon surface, `[terminal] pane_frames` in
    `config.toml` defaults to `true` and requests one rounded border around the
    visible pane stack, with full-width separators and a small bottom gap (4
    logical pixels at default typography) using terminal background color/opacity
    under the existing surface-wide blur request. Selected and hovered headers
    follow the stack corners, with square internal edges. `false` removes
    the border and separators while retaining headers, selection, hover and
    keyboard focus. Both modes preserve terminal-grid and picker geometry. EonTerm
    receives no workspace-frame override. Requested initial grids account for
    the small bottom margin. Live surfaces retain their settings;
    malformed or duplicate values reject a new launch or replacement without
    stopping existing terminals. Eon supplies the default; Eon Runtime owns the
    strict boolean and launch projection; Venus owns geometry, rendering,
    input and accessibility.
  - Left/right tab traversal and up/down pane traversal wrap at ordered edges
    when the axis has at least two targets. Singleton traversal and selecting
    the already-selected live identity succeed with an unchanged snapshot.
    Left/right remains available while a directory picker is open: the picker
    stays bound to its original live tab while another active tab presents its
    selected pane. Tab navigation by identity also remains available. Users can
    create panes in another tab and create additional pending tabs, each with
    its own picker under EONW v7.
  - Moving left/right swaps the active tab with exactly one adjacent tab;
    moving up/down swaps the selected pane with exactly one adjacent pane in
    the active tab. The moved stable identity remains selected. Movement stops
    quietly at ordered edges and remains unavailable in the picker-bound active tab.
    Venus maps Ctrl+Alt+H/L and Ctrl+Alt+K/J directly to these actions without a
    mode.
  - Closing names the expected active `tN` and requires another live tab. A
    non-final pending tab first stops its exact picker, then disappears and
    restores its prior tab. Eon stops the tab's pane and popup terminals one at
    a time and prunes each
    confirmed end through the normal terminal-exit owner. Complete success
    removes the tab with the same deterministic focus rule as natural exit.
    If a later stop fails, no later terminal is stopped, the action reports
    failure, and every remaining live terminal stays represented in the
    partially pruned tab. A terminal that ended naturally during the request is
    reconciled and pruned as an exit rather than reported as a successful Stop.
    Venus maps Alt+Shift+W directly to this stable-target action; lowercase
    Ctrl+W remains terminal input.
  - Ended terminals leave no dead pane or empty tab; focus moves deterministically
    to the same identity when possible, otherwise the following sibling at the
    removed index, otherwise the preceding sibling; an empty workspace closes
    Venus and the supervisor.
  - Same-boot recovered runs project numerically into synthetic `t1` as `pN`
    without claiming restoration of prior topology or launch-directory policy.
  - New current-generation full Eon surfaces omit the redundant native title
    bar while retaining the selected terminal title for compositor semantics.
- **Important failures:** Unknown or stale identity, a close target other than
  the active tab, final-tab close, durable close in the picker-bound tab,
  invalid transition, unavailable endpoint, duplicate identity, or partial
  recovery leaves accepted topology unchanged. A repeated request ID never
  repeats a stop, and a replay naming a removed tab cannot close its successor.
  Picker-stop failure changes no topology; durable partial-stop behavior is the
  explicit result above. Exhausted pane numbers fail before terminal start;
  unknown or repeated terminal-exit notices remove nothing; losing a view never
  silently stops or substitutes a terminal.
- **Owner:** Eon Runtime workspace topology, identity, focus, pruning and action
  enforcement; Eon supplies product defaults/policy. Orbit owns terminals and
  Venus owns native materialization.
- **Consumes:** Current EONW v7 workspace/popup values, Orbit terminal
  identities/endpoints and Venus `VEN-C8`; v5 origin proofs remain scoped below.
- **Boundary:** No arbitrary split tree, simultaneous expanded panes, arbitrary
  reorder target or cross-tab pane movement, picker relocation, durable layout
  restoration, a user-facing per-pane terminal stop/restart action, terminal
  content/history, provider state, plugin surface, remote access, or
  reconstructed terminal state.
- **Proof:** Eon `5f23a7bac127785d913a718e5fb1afc53d9d91ae` and the
  following distinct accepted or dogfooded slices.
  - **Environment:** x86_64 Linux Nix packages and isolated native Sway;
    visual refinements use Sway 1.12 / Mesa 26.1.2 lavapipe at scale 1.
  - **Evidence:** Wrapped traversal, composition, creation, exit pruning,
    `tN`/`pN` projection, launch-directory labels and focused accessibility;
    topology base `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`, compact headers
    `e77e842fe8c7070a96047dff1bf028ccbd49b788` with exact Venus
    `19d7d28a2aba09c0afe3173d25bbb76ec3edce49`. Movement candidate over
    `9f420ad7f7d4a09ec4294e5ecb191ca2a3d2b7d3` in
    `/nix/store/dbxxlkrh6ygvksra9rxdvqzzj4228jbq-eon-0.1.0` proves adjacent
    swaps and stable terminal mappings through locked Rust/Nix and installed CLI.
  - **Close and shortcuts:** EONW
    `c305453bba4fe50c29f65e829b9cd65af31ced8a`, Venus
    `d2d798099934dcf8037bfad6ab856e40c9b989fe`, artifact
    `/nix/store/x408dv5djxnia3sh88jwalhpqxfbmh2r-eon-0.1.0` prove ordered
    moves, stable close, deduplication and partial-stop topology. Current Nova
    shortcut acceptance at Eon `fea679260dd81fcdbb907e804156b00244af2324`,
    Venus `0791f00926cd5cc4fedcc7737aeac7bb68569aff`, artifact
    `/nix/store/578xxkf17cin8sdn6pb5rr10hxz4yfqg-eon-0.1.0` proves native
    Alt+Shift+T/W, pending/durable close, final-tab protection and unchanged
    Ctrl+T/Ctrl+Shift+W terminal bytes (`ven-yp4`).
  - **Quiet navigation:** `eon-quiet-workspace-navigation-y7a`, Eon
    `07dc210a49cd30f3b58fa65fd9e9a45b421e9b36`, Venus
    `b7404dae6c59a8e6de6ca9efbf2c907eedf1262b`, Orbit
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, EONW v4:
    locked Rust/Nix plus installed
    `/nix/store/8mykzkf3fp5d8f3xpyk8ax3ysirnr2xz-eon-0.1.0` proves unchanged
    no-op snapshots, native focus and stale-target rejection. Venus's real
    Application check proves quiet connection phases and visible offline failure.
  - **Pane frames:** `eon-pane-stack-delivery-jfo` and `ven-connected-pane-stack-0ub`
    retain dogfood of omitted/true/false, rejection and reopen, native focus/input,
    picker/grid geometry and no EonTerm override. Corrected candidate over Eon
    `b29f0d7d205c03a2d9b8d81ee600499dfe2f9af6`, Venus
    `bc5a2bd3b2363abdea69c4cd89953b612bc970e8`, Orbit
    `91999d79546422b49bdbc124166a65859d0bd872`, EONW v4 uses
    `/nix/store/52xxap7klyzcg1fdvgwnz7w30v24q2gx-eon-0.1.0` and
    `/nix/store/xbkp5n2m3hz9da7xcspk61aidp6x3brh-eonterm-0.1.0`.
    Scale-1 colored-underlay/pixel checks prove stack-shaped highlights,
    full-width separators and a terminal-color/alpha 4px gap; reopened 100×30
    PTYs retain the same terminals. Sway proves alpha/color, not compositor blur.
  - **Tab presentation:** `eon-install-rounded-adaptive-tabs-5eu`, Eon
    `b25da77fb61ed1e82a237d22ffd0c7e8394ce7df`, Venus
    `f5c679443b2fbda1a7a8f91c98d312811bee2cdf`, artifact
    `/nix/store/7b710jwv0rfikvviq9pfsj7axrcncc23-eon-0.1.0` proves natural/capped
    rounded labels, full-path hover, pointer/keyboard selection, stable launch
    directories, overflow and whole-strip wheel routing without workspace
    mutation. Header correction `0f2b76d2153db8e5716d0cccd78b26802684b5be` /
    Venus `94b15af20d1798b648f4d9945fd6bb647f10add8` retains long-label proof
    in `ven-2fq`. Picker-scoped actions and v7 pending tabs are proved in C18;
    public-alpha native topology and recovery are proved in C7.
- **Open proof:** Current-alpha AT-SPI labels and screen-reader use remain open.
  Blank/incomplete headless picker captures occurred on both compared tab
  artifacts; their cause remains unresolved. These slices do not establish
  uninterrupted picker rendering, fractional-scale, wider compositor,
  non-systemd or new EonTerm native interaction coverage.

## EON-C9 — Managed shell environment

- **Status:** Proven
- **Consumer:** Eon terminals launched under Nushell, Bash, Zsh, or Fish.
- **Trigger:** Eon constructs a child launch environment.
- **Result:**
  - Eon selects exact Nushell, Bash, Zsh, Fish, Starship, Zoxide, fzf, Atuin,
    Carapace, Helix, Yazi, and LazyGit artifacts.
  - Stable `eon-*` commands expose managed shells and tools outside Eon; one
    child-private PATH exposes accepted unprefixed names inside terminals.
  - Configuration accepts one direct argv command and independent Starship,
    Zoxide, Atuin, and Carapace booleans. Defaults are `command = ["eon-nu"]`
    and `true`; each new terminal rereads them without rewriting running shells.
  - Managed shells load native user configuration before bounded Eon activation.
    Disabled integrations suppress only Eon's hook; existing prompts,
    completers, same-tool hooks, normal Starship discovery, and Atuin policy
    remain child-owned.
  - Nushell native user autoload remains last and its stock startup banner is
    suppressed. Eon does not set `STARSHIP_CONFIG`, and Atuin continues to honor
    `ATUIN_NOBIND`.
  - Managed Nushell enables native `clip copy` and `clip paste`. Its interactive
    startup supplies `clc` and `clp` aliases for those commands; native user
    autoload may override the aliases afterward.
- **Important failures:** Invalid or unreadable configuration, empty command,
  missing artifact, manifest mismatch, or shell launch fails explicitly without
  a replacement terminal. Optional integration failure warns without preventing
  shell launch. Unavailable native clipboard access reports Nushell's own error.
  Eon never mutates global configuration, startup files, aliases, or PATH.
- **Owner:** Eon supplies defaults, component selection and Nix shell adapters;
  Eon Runtime owns strict configuration and private launch-environment
  enforcement. Shells/tools retain native configuration and behavior.
- **Boundary:** No global aliases, user-file mutation, prompt/completion schema,
  shell framework, automatic Direnv/Mise, history/sync policy, plugin surface,
  editor replacement, or remote/headless clipboard bridge. Arbitrary argv
  remains unmanaged and `eon run -- COMMAND...` remains the explicit escape
  hatch.
- **Proof:** Base `9f6599d103162061ee666126c2eddce03383afb6`;
  native clipboard `99c8e44385424775648238459bdb66f3607d9bfe` in
  `eon-nushell-native-clipboard-jwvp`.
  - **Environment:** x86_64 Linux Nix alpha; private native Wayland for clipboard.
  - **Evidence:** Four-shell environment/activation checks, native hook and Fish
    preservation, private terminal PATH, fzf-option isolation and tool glyphs;
    base `6dfcb473beccadd6e145235009240c81dd570fe5`, glyphs
    `fb95671d855fa944c3717103cb13bd0135f8aec8`, private PATH
    `c0d044c69318a921f9f9139bcaf2de9afce683d3`. Clipboard's managed-command,
    shell and Nix checks plus installed
    `/nix/store/60814358az00zr420rmbai7lz5qy1a1x-eon-0.1.0` round-trip Unicode
    through commands/aliases and `wl-paste`; native user autoload overrides and
    the no-display native error remain intact. C4 proves external composition.

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
  - Workspace results carry ordered tab, pane, and terminal identities, active
    and selected identities, liveness, exact raw tab launch-directory bytes,
    and exact opaque Orbit endpoint bytes. Lifecycle results carry generation,
    component/protocol identity, live terminals, and owner-authored attach/stop
    availability.
  - CLI JSON emits opaque directories and endpoints as ordered byte arrays
    without changing EONW. Each connection carries one length-delimited request
    and response and does not use EOF as framing.
  - The shared 2 MiB frame ceiling admits every snapshot valid at EONW's field
    and workspace-count bounds.
  - Harmless navigation no-ops return the unchanged Snapshot as success;
    request deduplication and picker/lifecycle restrictions still apply.
- **Important failures:** Malformed, oversized, incompatible, unavailable,
  rejected, timed-out, partially read, or response-lost operations fail within
  the shared bound without reconstructing hidden state or reviving completed
  Stop. Accepted Stop names affected live terminals, sends canonical Stop through
  every validated management lease before waiting, and returns only after every
  exact terminal record is complete.
- **Owner:** Eon Runtime supervisor workspace/action state and CLI projection;
  its canonical EONW package owns types, codec and result ordering. Eon supplies
  validated component facts and product defaults.
- **Consumes:** Current EONW v7 and accepted Orbit management operations; the
  v5 popup shape and historical lifecycle codecs retain their scoped proofs.
- **Boundary:** Additive lifecycle tags do not widen the pinned Venus workspace
  consumer. There is no event stream, subscription policy, remote transport,
  plugin/MCP API, authorization framework, durable restoration, terminal
  content, or direct child-protocol escape hatch.
- **Package owner:** The canonical EONW crate lives in `eon-runtime`
  with its accepted package/API and v2–v7 bytes. Venus consumes its exact pin; later
  runtime-only changes with unchanged codec identity retain its exact pin.
  See [package selection](ARCHITECTURE.md#independent-package-selection).
- **Runtime policy:** Project is required on Alt+Z and uses Eon's existing
  transient chooser command. Git defaults to `eon-lazygit` on Alt+Shift+J;
  Agent defaults to the first available provider in the accepted Nova order on
  Alt+Shift+L. Git and Agent remain live when hidden. Reopening at the same tab
  directory preserves identity; explicit retargeting changes no process until
  the next invocation, which stops the exact old terminal before starting a
  fresh one at the new directory. A stop failure preserves the old instance; a
  start failure records no replacement.
  Anima defaults to Alt+Shift+A and launches the pinned random native animation
  as a transient popup. A dismissal, replacement, or tab close stops its Orbit
  terminal, restores the prior tab focus, and leaves no hidden animation running.
- **Configuration:** `[popup]` owns finite side and vertical margins, defaulting
  to 8 and 4 logical pixels. `[popups.<id>]` owns direct argv, physical shortcut,
  label, enabled state and keep-alive lifetime. Only Agent accepts `"auto"`.
  Eon rejects unknown fields, malformed or colliding shortcuts, unbounded
  values and unavailable executables before mutating workspace state. It does
  not install providers, cache selection, interpret shell strings, or expose
  commands over EONW. Bare executables resolve through the terminal PATH;
  relative executable paths resolve against the owning tab's launch directory.
- **Target safety:** Invocation names the tab, entry and expected instance
  (absence means no instance exists). Focus-only and toggle intents are
  distinct. Dismissal and chooser completion require the exact live instance;
  a replaced instance cannot inherit an old request. Instance identities must
  not be reused after supervisor recovery. The runtime owner validates targets
  against its current Snapshot before applying lifecycle policy; decoding checks
  syntax, not staleness. Ordinary `SetTabDirectory`
  remains explicit retargeting, not chooser completion.
- **Bounds/failures:** Preserve the 2 MiB envelope, 64 tabs, raw Unix directory
  and endpoint bytes. At most 32 enabled entries and 256 combined pane/popup
  terminals are represented. Reject malformed framing, invalid or aliased
  identities/endpoints, broken selections, duplicate entry/chord assignments,
  invalid geometry and stale popup targets. No terminal contents cross EONW.
- **Popup snapshot:** The catalog carries enabled entries, normalized physical
  shortcuts and logical-pixel margins; per-tab popup terminals/selection and
  pending-tab state are explicit. Popup-only tabs are valid. Commands, provider
  selection, cwd and lifetime remain runtime policy and never cross EONW. The
  retired v4 picker opcode is unavailable; Project uses ordinary popup invocation.
  Older fixed-namespace work may be inspectable without being presentable. There
  is no negotiation or adapter.
- **Proof:** Base `9f6599d103162061ee666126c2eddce03383afb6`;
  v5 seed `0cc8f477298681ae3945903e8fdb5852d487c5ab` and activation
  `2214a4f592f78437c2f74aff0a1df8d1cc7ee5a4` with Venus
  `d212ff911c18cf0c1cd0f6b7e3f48a2e01d78d86`.
  - **Environment:** x86_64 Linux Nix package and installed profile.
  - **Evidence:** Codec/version rejection, raw directories/endpoints, CLI
    projection, absolute connection/read deadlines, management Stop and
    response-loss finality. Origin proofs: v2
    `7bb50873ae27e09dfebd8a6ca2f8075bac07afe8`, v3
    `96119f29ca2e3ec4ad19bbe272708b07d588429a`, lifecycle tags
    `fd6b348494111a0d18e241787da14ea99ee117a9`, CLI
    `d0c22080647a97b425b2448fe612c41be0357e57`, deadlines
    `a390fc5c007900c3fc8c9c49df85f9b8acc06d4a`, management Stop
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, lost response
    `56fcae2d00baecf9b69e4650882a69e50419f56b`, maximum-bound snapshot
    `10c29edf861fac28db48f41a4546165f79777ee6`.
    v5 codec checks protect exact/common action bytes, retired picker-opcode
    refusal, lifecycle results, stale/absent/wrong-tab popup targets,
    popup-only/pending tabs, ownership/geometry and frame bounds. The seed's
    mechanical proof is distinct from installed activation in C18. C18/C22/C23
    name v7 consumers; C4 proves the external v2–v7 package owner.

## EON-C11 — Runtime generations and presentation

- **Status:** Proven on x86_64 Linux native Wayland
- **Consumer:** Eon and EonTerm users launching or targeting a product generation.
- **Trigger:** Launch current generation, inspect an older live generation,
  present a compatible existing surface, stop one generation, or stop a selected
  set of generations.
- **Result:**
  - Eon and EonTerm use separate runtime namespaces and default to their exact
    current generation.
  - Generation identity combines immutable runtime/EONW source and build
    inputs with Eon assembly/default/validator/graph/Cargo/Nix-generated inputs;
    each generation owns one private directory. Store/profile paths, mutable
    configuration, PIDs and live state do not define identity.
  - Older live generations remain discoverable, explicitly presentable when
    compatible, and explicitly stoppable through their own supervisor. Generation
    inspection and Stop use the supervisor's known EONW v2 through v7 lifecycle
    codec; presentation still requires the exact current EONW and component graph.
  - `eon stop previous` attempts every non-dead generation except the exact
    current one, whether current is live or not. Fixed-namespace legacy work
    reports its own unavailable reason. `eon stop all` includes current.
    Each selected generation uses the existing validated
    Stop; human mode confirms each target, while `--json` returns a result array
    without confirmation. Declining one target reports that no Stop was sent for
    that generation; earlier outcomes remain and later targets continue.
    Failed or unavailable targets do not suppress later targets while the CLI
    remains running.
    A Stop of the caller's own terminal can terminate the CLI before it receives
    that result or attempts later targets; run from outside selected generations
    when a complete result is required. When the CLI completes, it exits
    nonzero if any Stop attempt fails. `all` stops current last so earlier
    targets can finish when invoked inside a current terminal.
    Neither command forcibly kills a process or treats fixed-namespace legacy
    work as stoppable.
  - Implicit attach selects only a live supervisor reporting the exact current
    identity; otherwise Eon starts current without stopping older work. Explicit
    attach never substitutes another generation, and discovery distinguishes
    current, previous, legacy, dead, incompatible, and corrupt candidates.
  - One per-generation lifecycle lock covers listener and runtime teardown so a
    replacement cannot overlap cleanup; concurrent launch converges on one
    supervisor and terminal set.
  - Present requests native presentation without replacing the live Venus
    process, Orbit attachment, or terminal.
  - Dead exact residue is removed only after the recorded process is absent or
    dead; a later launch may then create one fresh run.
  - A current launch overlapping clean exit of the final terminal waits for the
    retiring supervisor, then starts one fresh terminal instead of reporting
    presentation success against ended state.
- **Important failures:** Unknown EONW version, incompatible presentation,
  stale or mismatched process identity, lost control, partial Stop, or live
  foreign residue fails closed
  without PID/process-name fallback or duplicate Orbit launch. Orbit startup
  and Ready negotiation remain bounded by the shared five-second deadline.
  Eon requires no cgroup membership or delegation; Orbit owns bounded PTY
  shutdown through its accepted ORB-C12/ORB-C13 management boundary.
- **Owner:** Eon Runtime generation identity/selection, namespaces, discovery,
  Present, explicit Stop and exact owned residue; Eon supplies immutable
  assembly/default/validator/graph/Cargo/Nix-generated inputs.
- **Consumes:** Orbit management v1 and Venus `VEN-C14`.
- **Boundary:** No manager daemon, persisted topology, machine-restart recovery,
  PTY handoff, old-generation backport, updater, Nix evaluation, package-channel
  identity, presentation compatibility window, remote runtime, plugin API, service-manager
  requirement, or automatic eviction.
- **Assembly boundary:** The external `eon-runtime` generation owner consumes immutable
  runtime/EONW bytes and the supplied Eon assembly contribution, including
  generated-command inputs. Exact presentation checks remain;
  retain the prior executable to reattach incompatible old work. See
  [generation inputs](ARCHITECTURE.md#defaults-overrides-and-generation) and
  [cutover acceptance](ARCHITECTURE.md#acceptance-boundaries). EON-C4 records
  the local-library, independent-producer and installed consumer cutover proofs.
- **Proof:** Base `10c29edf861fac28db48f41a4546165f79777ee6`, artifact
  `/nix/store/dbiv438iwn9mljrwfg6d29ypg96hp00c-eon-0.1.0`.
  - **Environment:** x86_64 Linux Nix packages and installed profiles;
    native slices use isolated Sway 1.12 Wayland.
  - **Evidence:** Separate namespaces, repeated launch, native Present,
    owner-routed Stop, supervisor-loss replacement, dead-residue correction
    and concurrent convergence. Generated identity
    `4beb441301d84039570dd07138da2a6f70b3ed23`, convergence
    `36c95ad04cb9519f88b907642c8147d95f56ee83`, EonTerm lifecycle
    `63686a12b752c9423b2096d5e32aa5842f2184fc`, codec sensitivity
    `3c1d6e08d223df753eb88ca031d108ed1abe8ee3` and bounded frames at the base
    retain their scoped checks in Git. C15 owns exact same-boot recovery;
    C4 proves both immutable contributions and cutover old/new generation behavior.
  - **Cgroup-free launch:** `eon-uj7`, Eon
    `76b9f5635d9ae545dadb0326976f309afae8fd25`, artifacts
    `/nix/store/p4hbh2jlr6i22mah8r3f5g2lc696m0r6-eon-0.1.0` and
    `/nix/store/i53rvda2crdyv4fzsy6vyawzgvd4yb9f-eonterm-0.1.0` pass locked
    Rust/Nix and native launch, detach/reopen and Stop with `/sys/fs/cgroup`
    hidden. This proves no cgroup requirement, not installed non-systemd support
    or whole-descendant cleanup.
  - **Historical codecs:** `eon-stop-older-eonw-generations-k40`, Eon
    `446d8a7e4f537d2ee439c5441a6b20203d4ea71d`, artifact
    `/nix/store/0rir9q0a42d77jhh9z615acz3qch8z83-eon-0.1.0`: v2–v6
    owner-routed fixtures, locked Rust/Nix and real installed v6 Stop including
    an empty isolated supervisor. v2–v5 are fixture-verified only; legacy has
    no authoritative Stop. The public qualification in C7 adds native previous
    v6/current Stop; C4 adds exact v7 cutover migration.
  - **Batch selection/cancellation:** `eon-stop-generations-batch-gdb` and Git
    `f4e6a4dfd31c1b9f84aefdbb70a3619c649df4e1` retain final current-unstarted,
    legacy/corrupt target, caller-lifetime and human-decline regression proof.
    Installed `/nix/store/s28ypi05i5jw7n49sdskzzc65kmr6m72-eon-0.1.0`
    successfully stops a private v6 fixture with current unstarted; these
    installed substitute-owner checks are not native Stop acceptance.
    C23 proves one piped confirmation per prompt and caller-last window Stop.

## EON-C12 — Standalone exact-command terminal

- **Status:** Proven
- **Consumer:** A local user or approved composition needing one native terminal
  surface without Eon workspace actions.
- **Trigger:** EonTerm launches one exact command after `--`.
- **Result:**
  - `eonterm -- COMMAND...` launches one exact nonempty argv in one Orbit terminal
    and one Venus surface without workspace topology. Optional
    `--no-decorations` is fixed for the supervisor lifetime.
  - `eonterm generations`, `attach`, and `stop` expose EON-C11 within EonTerm's
    separate namespace.
  - Repeated launch presents an attached surface; after detachment it opens one
    replacement against the same live terminal with the original decoration
    choice. Later invocations do not mutate that choice.
  - Workspace actions return `workspace-unavailable` and create no hidden
    topology.
- **Important failures:** Missing command, invalid identity, incompatible
  component, startup failure, supervisor loss, or explicit Stop follows the same
  bounded ownership and cleanup rules without creating workspace state.
- **Owner:** Eon defines and packages the standalone product; Eon Runtime
  implements exact-command launch/generation mechanisms. Orbit owns the terminal and
  Venus owns the surface.
- **Consumes:** EON-C1 through EON-C4, EON-C11, Orbit, and Venus.
- **Boundary:** EonTerm consumes no EON-C9 managed environment and adds no tabs,
  panes, key remapping, preset, wrapper, persisted mode marker, workspace,
  managed-tool/default-shell command, remote access, focus guarantee, or Zellij
  policy. Bare `eon` and `eon run` retain workspace meanings.
- **Proof:** `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`.
  - **Environment:** x86_64 Linux Nix package and installed runtime.
  - **Evidence:** Exact argv/environment, vivid palette, repeated launch,
    supervisor-loss recovery and clean Stop; product
    `816372ea9ffe90f08ff442b5763a3b5413b7906c`, palette
    `285c6bf48fb402a673eb997594b6eb1bbcb1429b`, lifecycle
    `63686a12b752c9423b2096d5e32aa5842f2184fc`. C7 names the public native
    artifact and same-terminal recovery; C4 names external package/closure,
    co-installation and config-reread proof for EonTerm.

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
- **Owner:** Eon supplies the default; Eon Runtime owns strict configuration
  and launch projection; Venus owns native rendering.
- **Consumes:** Venus `VEN-C11`; its original exact acceptance is scoped below.
- **Boundary:** Opacity affects only terminal default background and padding, not
  explicit cells, text, cursor, selection, chrome, decorations, input, hit
  testing, accessibility, or click-through. No profile, public override,
  watcher, live reload, Orbit restart, non-Wayland claim, or renderer fallback.
- **Proof:** `7303ee5cca3925939b84267bd54587a2cfb223a6`,
  `eon-default-opacity-0-80-eonova-rollout-elu`.
  - **Environment:** x86_64 Linux COSMIC Wayland; Venus VEN-C11
    `74ab5a0b661210f0afec94086f5358fe50b01f05`, Orbit
    `6de95296d252c119d4fdba2d9b03cec1a09355ae`, ORBS v3 / EONW v1.
  - **Evidence:** Config admission/rejection, both products' argv, values and
    live/replacement behavior; locked Rust/Nix checks. Config-free installed
    EonTerm `/nix/store/8a31d8p6vcxld23j05wjzisw8b0n68bp-eonterm-0.1.0`
    launches real Venus with `--background-opacity 0.8`; Eon artifact
    `/nix/store/3b7zzsn9ah79qm4r49rdxmcp9fsdfq56-eon-0.1.0`.
    Earlier explicit-value/reopen and native visual acceptance remains in
    `eon-eonterm-presentation-e4f`; C4 adds cutover reread evidence without a
    new compositor visual claim.

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
- **Owner:** Eon supplies the default; Eon Runtime owns strict configuration
  and launch projection; Venus and the compositor own native behavior.
- **Consumes:** Venus `VEN-C15` and EON-C13; original acceptance is scoped below.
- **Boundary:** No capability probe, automatic opacity, blur strength, public
  override, watcher, live reload, fallback renderer, Eonova-specific policy, or
  non-Wayland visual claim.
- **Proof:** `b41be3a00a8e47a435521509c3d060d80e0524a5`,
  `eon-consume-venus-background-blur-k8o`.
  - **Environment:** x86_64 Linux COSMIC Wayland; Venus VEN-C15
    `7fc7e4ba97aaf48b586002934a580ef2d1c31694`, Orbit
    `6de95296d252c119d4fdba2d9b03cec1a09355ae`, ORBF v1 / ORBS v3 / EONW v1.
  - **Evidence:** Default/true/off admission, independent opacity, exact argv
    and native visual acceptance; locked Rust/Nix checks and installed
    `/nix/store/ky1f6a1q5iz2lg95ph81kgs1b6ragms3-eon-0.1.0` and
    `/nix/store/qh2gsxryzlyypva6087sw0bp0ninyhj3-eonterm-0.1.0`.
    Config-free real Venus receives one blur token; unsupported compositor
    behavior remains best effort, and Sway alpha checks do not prove blur.

## EON-C15 — Same-boot Orbit recovery

- **Status:** Proven on x86_64 Linux native Wayland
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
    management record inode. Orbit locks it, marks publication and atomically
    replaces it with Ready. Local Child rollback requires winning the lock on
    that unchanged still-empty inode; marking revokes signal authority even
    before publication completes. After publication, cleanup is management-only.
  - Explicit generation Stop uses canonical management for every acquired run
    and discovers untracked runs through the same bounded generation record
    and lease validation used for recovery, including late-Ready pickers.
    It succeeds only after matching tombstones and cleanup.
  - Eon removes exact owned dead residue only after the recorded process is
    absent or dead.
- **Important failures:** Unsafe, missing, replaced, malformed, oversized,
  incompatible, duplicate, zero, noncanonical, overflowing, wrong-generation,
  wrong-UID, wrong-process, Busy, partial acquisition, Orbit exit, deadline, or
  transport failures publish no partial workspace, launch no duplicate run, and
  authorize no PID, signal, pathname, process-name, group, cgroup, or pidfd
  fallback. A lost response after completed Stop cannot revive the supervisor.
  A marked Live record rejected after spawn is stopped through canonical
  management using its published identity and is never published as a terminal.
  Failed Stop retains its supervisor and unfinished leases for explicit retry;
  completed runs leave live inventory. Workspace mutations and presentation
  remain unavailable until cleanup succeeds. Failed natural-exit reconciliation
  still collects other results and restores retryable leases. Discovery reaps
  ended untracked direct children before retiring their canonical records.
  During failed cleanup, it still reaps an exited presentation child without
  changing the workspace or launching replacement terminals.
  Stop's result read timeout allows one preceding five-second operation plus
  five-second cleanup and a one-second margin; excessive queues can still fail.
- **Owner:** Eon Runtime generation/recovery enforcement, bounded enumeration,
  projection, lease collection, deadline, EONW ordering, retained launch claim,
  and exact residue; Orbit `ORB-C13` owns live identity, Ready, lease, terminal state,
  `ORB-C12` cleanup, tombstone, and endpoint cleanup.
- **Consumes:** Orbit `ORB-C12`, `ORB-C13`, management v1, and Venus `VEN-C14`.
- **Boundary:** Same boot and login only; no prior-topology persistence, Orbit or
  machine restart recovery, logout/reboot survival, remote authority, service
  manager, broker, or compatibility adapter. Tombstones grant no authority and
  must be reconciled before their workspace slot is reused.
- **Proof:** `6b3c64d13c2fee205a6f1c218b1c4fc107507f1e`.
  - **Stop repair:** `eon-repair-stop-completion-picker-cleanup-1xxu`, Runtime
    `eceac276803a1ec819a402905c685e1c2a2e5865`, unchanged EONW
    `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, Orbit
    `6bc269c40b18f08b95778939518f77556ba91c67` and Venus
    `a27aad23822d3bd0b0e467ffc32c578bbde8b00d`. The Eon working-tree candidate
    over `92e0d7b379a14d003f37e5c2df9c468cb43e74b7` passes complete producer
    and composed locked Rust checks, both Nix products and package identity gate.
    Installed `/nix/store/w9pd9b36yb0ndz007sjhfjwgjzq3lm5a-eon-0.1.0`
    on isolated Sway 1.12 / Mesa software Vulkan proves two-pane window Stop,
    active-picker cleanup and preservation of peer/older work. Exact candidate
    file hashes, regression negatives, native process identities and profile
    proof remain in `eon-stop-1xxu-2026-10-07-222631` under Eon's proof root.
  - **Reviewed correction:** Runtime `4d8074a51022e828872abb127f77c27e26d5f2eb`
    collects all Stop results after a natural-exit cleanup failure and simplifies
    finished-run iteration. Its focused red/green regression, complete locked
    Rust checks, both Nix products and installed native window/picker/older-work
    proof pass. Installed artifact:
    `/nix/store/i15qjws0vqrc5jmngvdqffbqzyq6s5bz-eon-0.1.0`; exact candidate
    hashes and the host Nix cache negative remain in
    `eon-stop-1xxu-review-2026-10-07-225703` under Eon's proof root.
  - **Ended-discovery correction:** Runtime
    `9bc86e08f2bd2c45b32b8e74068a28e1cb03f675` shares the existing child reaper
    with discovery, reaping ended untracked direct children before record
    retirement and rejecting wrong-start identities. Its focused red/green
    regression, complete producer/composed Rust checks, both Nix products and
    installed native window/picker/older-work proof pass. Installed artifact:
    `/nix/store/41jhpaaqdawq0ka7mgp8z0xcy98l7w78-eon-0.1.0`; exact candidate
    hashes and retained host/build negatives remain in
    `eon-stop-1xxu-review2-2026-10-07` under Eon's proof root.
  - **Presentation retry correction:** Runtime
    `02865fdf908d8139d4c94180703381f6029c0598` reaps exited Venus while
    incomplete Stop retains its owner and unfinished runs, without workspace
    mutation or replacement launches. The extended regression is red/green;
    full producer/composed Rust, both Nix artifacts and installed native
    close-before-retry checks pass. EonTerm uses per-build executable RAM scratch
    with unchanged `/build` paths/tests; host deadline negatives remain retained.
    Installed Eon: `/nix/store/qnfhshxx9kdzbazgx90cjv3xhb62g1ds-eon-0.1.0`;
    exact hashes/evidence: `eon-stop-1xxu-review3-2026-10-08` in Eon's proof root.
  - **Environment:** x86_64 Linux packaged installed recovery and adversarial
    lifecycle checks.
  - **Evidence:** Ready serialization, supervisor loss, exact adoption, Busy and
    saturated-listener bounds, partial-response deadlines, lost-response Stop,
    tombstones and dead-residue relaunch; base
    `9f9b5c3144bf0f8e4cb8e0ec2b5b09d254b04910`, lost response
    `56fcae2d00baecf9b69e4650882a69e50419f56b`, connect deadline
    `a4f0122862b7293326c22530f787124a47d0f560`, response deadline
    `ced9e4ae11ed21a0f05d50cd470491adffa73b54`, Ready serialization
    `871c9f639e519c7e3201fa5d9755d0a77a4cb0df`, compact recovery
    `cfcb38e4284843f163c1a04f8c71052f31c0ac5b` and synthetic-tab directory at
    the base. C7 names exact native artifacts recovering three full-Eon and one
    EonTerm terminal without duplicates. Prior adversarial checks keep their
    scope; no logout, Orbit-restart or machine-restart recovery is proved.

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
- **Owner:** Eon supplies defaults; Eon Runtime validates and projects EonTerm
  CLI input; the embedding composition owns its identifier/desktop metadata;
  Venus owns native mapping only.
- **Consumes:** Venus `VEN-C17`; original exact acceptance is scoped below.
- **Boundary:** Immutable launch metadata only; no runtime protocol, branding
  provider, desktop discovery, child inspection, or wider platform promise.
- **Proof:** Identity source `6a3236bb342c535aca16acdf563ff66386e8f6e5`
  and dead-residue correction `5631d8dc4de49bfd3831aa3abef300b7734ad36c`,
  `eon-eonterm-application-identity-xwi`.
  - **Environment:** x86_64 Linux COSMIC Wayland; Venus VEN-C17
    `ab24961bd6b2f9403736e52ebbac8cc266488a41`; exact historical Eonova
    embedding `a9d5946686ab28e51b2c10cc9f2d3f6fb1282b52`.
  - **Evidence:** CLI rejection, exact Venus argv and distinct launcher grouping,
    repeated-launch convergence and unchanged terminal; Eon
    `/nix/store/q5lbbk5nqh778yqz7al0zxfin5x7vp0r-eon-0.1.0`, EonTerm
    `/nix/store/a3rf1w4fq615sa153plp7kihqnqdm25a-eonterm-0.1.0`, embedding
    `/nix/store/vjrwggm7655zhfv9x2fj3h5ny8sflixk-eonova-0.1.0`.
    This retains caller-owned identity proof, not activation of Nova integration.

## EON-C17 — Tab launch directory

- **Status:** Partially proved
- **Consumer:** One local Eon workspace user creating tabs and panes.
- **Trigger:** Eon creates or recovers a workspace, creates a tab or pane, or an
  approved same-user client explicitly retargets one live tab.
- **Result:**
  - Every live tab has one stable `tN` identity and one authoritative absolute
    launch directory. Panes retain `pN` identities.
  - Fresh `t1` validates the supervisor's absolute launch directory before any
    terminal recovery or startup. A new tab inherits the active tab's value;
    EON-C18's pending picker commits its first-pane directory or uses the
    validated value for the sole-tab cancellation fallback.
  - `SetTabDirectory` changes exactly one addressed tab. Later panes in that tab
    start with the accepted directory; existing terminals and terminal CWDs do
    not change.
  - Current EONW v7 returns the exact accepted raw path bytes. Venus derives a
    bounded `N  leaf`, `N  ~`, or `N  /` label while retaining `tN` for actions and
    exposing identity plus bounded path context to accessibility.
  - Same-boot recovery assigns synthetic `t1` the replacement supervisor's
    launch directory as fresh policy and never infers prior policy from a pane.
- **Important failures:** Empty, relative, NUL-containing, oversized, missing,
  non-directory, unknown-tab, or stale-tab updates change nothing. Once a path
  passes validation, later disappearance leaves its policy value stored; a
  later terminal startup fails without committing pane topology or leaving
  launch residue.
- **Owner:** Eon Runtime owns tab identity, launch-directory state, validation,
  mutation, inheritance and recovery fallback. Orbit owns each terminal's
  CWD; Venus owns native projection of the authoritative workspace state.
- **Consumes:** Current EONW v7, Orbit terminal startup, and Venus `VEN-C8`.
- **Boundary:** No manual names, path canonicalization, retained directory
  handles, filesystem watching, shell-`cd` tracking, pane-CWD inference,
  retargeting of existing processes, persistence, picker UI, or compatibility
  service accepting EONW v1 and v2.
- **Proof:** `9f6599d103162061ee666126c2eddce03383afb6`.
  - **Environment:** x86_64 Linux Nix package and installed Eon profile;
    Venus `2622c124be5ba8a62037c6de952ebf3928347387`.
  - **Evidence:** Raw-directory v3 seed
    `96119f29ca2e3ec4ad19bbe272708b07d588429a`, v4 pending seed
    `aaafc9127c054e683abfceb3c8fcaae201a7a763`; codec/workspace/CLI, actual
    multi-tab CWD, rollback/disappearance and maximum-bound snapshot checks;
    locked Rust/Nix, manifest and generation checks. Installed
    `/nix/store/j4h0bmyvkndzvhi3l9r96b7f57jxlmm8-eon-0.1.0` uses
    `g1-a9a0102de0a2c56d1d5bdda6c25bb3dc`. C7 adds native selection,
    inheritance/retargeting, existing terminal CWD preservation, later-pane CWD,
    invalid-path rejection and synthetic recovery fallback.
- **Open proof:** Current-alpha AT-SPI launch-path context remains unproved;
  prior focused path-context checks are not a screen-reader workflow.

## EON-C18 — Picker-first tab directories

- **Status:** Proven for shared popups and v7 multi-pending tabs on x86_64
  Linux native Wayland; distinct original/extension proofs remain below.
- **Consumer:** One person starting or using full Eon.
- **Trigger:** Eon needs the first pane for a new tab, or the person presses
  Alt+Z in an existing tab.
- **Result:**
  - On a fresh full-Eon workspace, after Venus accepts the initial presentation,
    Eon may play one short Anima animation in the initial transient Project
    terminal before starting the normal directory picker. The default is enabled,
    a random native style, and three seconds; `[anima].enabled = false` skips it.
    Any key other than Anima's style-browsing keys dismisses playback without
    becoming picker input. This step does not run for later tabs, attachment,
    reconnection, same-boot recovery, or EonTerm.
  - A fresh workspace and every later new tab begin as one active pending tab
    with no durable pane. Eon starts one transient Orbit terminal running the
    exact packaged ranked-directory picker at the inherited launch directory.
    Ambient fzf default options cannot alter its command, layout, or bindings.
    Each picker instance has a distinct endpoint within that workspace, including
    repeated retargeting of the same tab. A polling client can distinguish the
    replacement even if it misses the intervening no-picker snapshot.
  - Quick search ranks Zoxide history through fzf. Enter accepts one result;
    Tab opens packaged Yazi without cancelling the picker; Esc and Ctrl+C cancel.
    Every quick-search exit stops and reaps a still-running history query;
    accepting, browsing and cancelling do not wait for the remaining results.
    Empty history still shows the Browse action. Arrows move through results.
  - Yazi begins at the inherited tab directory. Arrows/hjkl browse parent and
    child directories; Shift+Z searches history and only moves the browser.
    Enter uses the highlighted folder; Tab returns to quick search and preserves
    the browser directory for the next visit. Esc, q, Q, and Ctrl+C cancel
    from the main folder list. Within input and help, Esc follows Yazi's native
    mode/filter handling before closing. Startup, new-tab and Alt+Z pickers
    share these keys. Native browsing, find, sorting and previews remain.
    `g h` reaches home, `g /` root, `g Space` accepts a typed path, and `.` toggles
    hidden entries. `a` uses Yazi's native `create`: a plain name creates a file,
    and a name ending in `/` creates a directory. Creation highlights the item
    without accepting the picker; Enter accepts only directories. Native input
    cancellation and existing-target confirmation remain. Cancelling the picker
    leaves successful creations on disk; pinned Yazi returns quietly on failure.
    The fixed chooser-mode open returns the highlighted path without invoking
    a file opener. Creation is the only enabled file-management mutation.
    External file opening, interactive shell commands and suspension remain excluded.
    Files cannot be accepted as a tab directory or opened by the chooser.
    Browsing does not add Zoxide history.
    Yazi owns directory enumeration, navigation and creation; Eon performs no
    recursive filesystem scan. A vanished or inaccessible final folder fails
    EON-C17 validation without mutation.
  - Each selection screen shows its own persistent footer: quick search offers
    Use directory, Browse folders, and Cancel; Yazi offers Use highlighted folder,
    Create, Quick search, Shift+Z Jump, Cancel, and Help; nested jump search
    offers Jump and Back to folders. Browser shortcuts are absent from quick search. Eon's fixed Yazi
    status configuration replaces file metadata with these picker actions.
    Browser hints appear only while the folder list owns input; native Yazi
    prompts and overlays hide them until the folder list regains focus.
  - Accepting a valid choice atomically commits the tab directory and starts
    exactly one first `pN` terminal there. Cancelling a pending picker starts
    its first pane at that tab's validated directory only when it is the
    workspace's sole tab; otherwise it removes only the pending tab.
  - Alt+Z keeps the existing explicit retarget behavior after a tab has panes;
    it never changes or restarts a running pane.
  - Current EONW v7 exposes Project as a popup endpoint bound to one live
    tab, never as a normal `pN` pane. The picker tab may remain live while
    another durable tab is active. A picker-owned pending tab remains pane-free,
    with an absent selected pane instead of a sentinel or placeholder process.
    Lifecycle inspection and Stop may report zero durable terminals during the
    initial picker; the transient picker remains excluded from that list.
  - Venus can start a full-Eon presentation from the workspace endpoint alone
    while the initial tab is pending; the picker endpoint in its first snapshot
    is the sole terminal attachment.
  - Venus keeps the tab bar visible and replaces the pane stack with the picker,
    using Eon's shared configured popup margins. The
    previous terminal is not composited underneath. Attach and recovery never
    create an automatic picker; recovering only a stale initial picker stops it
    and applies the validated launch-directory fallback.
  - Left/right focus actions continue to traverse and wrap live tabs
    while the picker stays bound to its original tab. Another active tab shows
    its selected pane; returning shows the same picker for normal acceptance or
    cancellation. Tab navigation by identity remains available. Other tabs
    allow pane creation, pane focus by direction or identity, pane/tab movement,
    explicit directory updates, and stable-target close. These actions preserve
    the picker's binding and terminal. Mutations of the picker-bound tab remain
    blocked except directory acceptance and closing its non-final pending tab;
    that close stops the transient terminal before removing the tab. Cancellation
    of the selected pending tab restores its previous tab if that tab survives,
    otherwise the nearest surviving tab. Background picker cancellation leaves
    the selected tab in focus. A workspace with no running terminals ends.
    Pickers remain attached across tab switches; established tabs may open
    their own popup while another picker remains live elsewhere. EONW v7 permits
    multiple pending tabs, each retaining its exact Project picker across switches;
    CreateTab always creates a distinct tab. Alt+Shift+W closes the selected
    non-final pending tab and stops only its picker.
- **Important failures:** Cancel, empty or invalid selection, picker launch or
  exit, target disappearance, duplicate invocation, origin-tab loss, or
  presentation detachment follows the sole-tab or remaining-tab fallback without a
  partial tab, consumed pane or durable terminal identity, changed existing
  process, or transient process, endpoint, record, pane, or modal residue.
  Left/right with only one live tab succeeds unchanged; final-tab close remains
  unavailable. A duplicate close cannot stop the picker twice or remove another
  tab.
  Invalid Anima settings fail before a fresh workspace starts. Missing, failed,
  or timed-out playback reports a short diagnostic and continues to the picker.
- **Owner:** Eon supplies picker/default policy and packaged commands; Eon
  Runtime owns tab binding, command selection, lifecycle, validation, mutation
  and cleanup. Venus owns full-Eon shortcut precedence,
  modal geometry, focus, input, notices, rendering, and accessibility. Orbit
  owns the transient PTY and child. Zoxide owns ranking, fzf owns quick
  selection, Yazi owns filesystem browsing, and Anima owns style selection,
  rendering, input dismissal, playback timing, and terminal restoration. Eon
  consumes one directory result without interpreting Yazi's navigation state.
- **Consumes:** EON-C17, current EONW v7 popup/pending-tab values, Orbit's
  accepted terminal startup/attachment/exit/stop contracts and the selected exact
  Venus consumer; original v5 acceptance is scoped in Proof.
- **Boundary:** No custom filesystem walker, editor/file-opening integration,
  copied Nova plugin, normal-pane identity, client-supplied command,
  simultaneous terminal composition, native Venus picker,
  placeholder shell, pane replacement, current-process `cd`, persisted pending
  tab, EonTerm action, or additional platform.
- **Proof:** Distinct v5 activation, v7 multi-pending and picker/tool slices.
  - **Environment:** x86_64 Linux Nix packages, isolated native Sway 1.12
    Wayland; named Mesa/headless qualifications remain attached to each slice.
  - **Evidence:** The following exact slices retain their distinct codec,
    runtime, installed native or packaged-tool checks and acceptance limits.
  - **v5 activation:** Eon `2214a4f592f78437c2f74aff0a1df8d1cc7ee5a4`, Venus
    `d212ff911c18cf0c1cd0f6b7e3f48a2e01d78d86`, Orbit
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5` (ORBF v2 / ORBS v11), artifact
    `/nix/store/4bni992l878fsl8jqgqh61zkfvvdisph-eon-0.1.0` passes locked
    Rust/Nix, nine installed popup states and user acceptance in
    `eon-tool-popups-e13.3`. Original Alt+Z COSMIC proof at
    `9f6599d103162061ee666126c2eddce03383afb6`, Orbit
    `70861097a825c2fbfaea53a8ca9437f45e8602eb`, Venus
    `2622c124be5ba8a62037c6de952ebf3928347387`, artifact
    `/nix/store/nqcjvnxyjrnjm1hwl4lcwqzi0jmyfvli-eon-0.1.0` retains exact
    cleanup, recovery, hostile-default and durable-only Stop proof in Git.
  - **Mode and footer:** `eon-picker-mode-switching-6bo` at
    `f75c910a6a8488bdad3c614544769080261e4560`, Orbit
    `91999d79546422b49bdbc124166a65859d0bd872`, Venus
    `541a9cb43c155b8b97069904593dc81c73682613`, artifact
    `/nix/store/024j1lxhwmxwgmkh4r4yxkz0hd2p1qyh-eon-0.1.0` proves the shared
    Tab/Enter/cancel/nested-Esc behavior, raw-path roundtrips and later-pane CWD
    without changing existing process CWD. `eon-picker-context-hints-j57`
    retains exact candidate hashes over `9cf712e031924f874a681036d931c24aa906b29d`,
    the same Orbit and Venus `e13970e90289d0d86f0adcbf350e4b9c1d5e5219`,
    artifact `/nix/store/qyxb0m5f19kplh6qbxknm3b63gzhyw31-eon-0.1.0`: private
    1100×750/720×600 native quick/browser/jump/help checks and nested-prompt
    regression prove contextual hints without premature directory commitment.
  - **Selection and cleanup:** `eon-yazi-highlighted-directory-d5d`, prepared
    source `/nix/store/zdwl036r2r9gd9knrwfm835mj0ra16lz-eon-source`, artifact
    `/nix/store/hij0is9qld30dgwag1hfa03cbzj4jahn-eon-0.1.0` proves distinct
    raw non-UTF-8 highlighted/parent results by regression and packaged Yazi PTY;
    this was mechanically verified, not a fresh full-Eon native observation.
    Review at `4298fbb8868752e3d6c8eb4fd79fae067ab3e2a1`, Orbit
    `91999d79546422b49bdbc124166a65859d0bd872`, Venus
    `e13970e90289d0d86f0adcbf350e4b9c1d5e5219`, artifact
    `/nix/store/008jbiqsprwvyn2w7bfnc7rqhd1vk36x-eon-0.1.0` proves blocked
    history-child reaping, exact distinct picker endpoints, immediate reopen,
    empty-history Browse, Shift+Z, untracked-child choice, new-pane CWD and
    native Ctrl+C cleanup. Native raw-path/home/root/q/Q and inherited-PWD
    checks remain at `e1896db32e1780ad8c29db40a3f22fa9b6bbf252`; its earlier
    Esc-to-Browse behavior is superseded by the accepted mode-switch contract.
  - **Other-tab actions:** `eon-confine-picker-blocking-to-its-tab-7g5`, Eon
    `6879dbc78975c135f9d794759b0fa6485ad207eb`, the same 91999d7/e13970e pair,
    artifact `/nix/store/23shhmi4q15blg0y2a56718s6xyfz821-eon-0.1.0` proves
    native/CLI other-tab actions, exact picker binding, late acceptance and final
    cancellation. Its one-pending restriction is historical, not current v7.
  - **v7 multi-pending:** `eon-allow-pending-tabs-with-pickers-gnz`, producer
    `f41a41c9aecc4c436edfa2f394b832aa6d8711ad`, Venus
    `479d7cef29aaa40d2d93d610567517357479877c`, Orbit
    `f8ad14e5195109ba8cb421f30e5ae4a9619a1419`, artifact
    `/nix/store/0m637rzcg5ksm27c5q0p6pd8gk2mw9yc-eon-0.1.0` passes locked
    Rust/Nix and native physical Alt+Shift+T, tab switching, independent browsing
    and exact selected-picker close. Background-exit correction
    `d0cf639a0edfb9fffd8142a709c5a858d34ec7e2`, artifact
    `/nix/store/svdyvf9s861vdgb85gpli0pdwvww30ny-eon-0.1.0`, preserves the
    selected tab when another pending picker exits; wire and child pins stay exact.
  - **Anima:** `eon-anima-startup-5el`, Eon
    `4ff62dbd1b4645ee8f2bd927d233388230f06a10`, Anima
    `b3133f057fa0029b3e06c85301161168c48bb799`, Orbit
    `b6cecf8f2ee35570b41cfdc578b095889d917fe2`, Venus
    `773c6bab7d3e21e0b0d7d942c9bba72260a08b42`; the tested working trees
    pass locked Rust/Nix. Simplified prepared source
    `/nix/store/5xjs4w72z0drwm1s99vzgp259lky3030-eon-source`
    and installed `/nix/store/9zwwy9jxnig5cy90s0j6xcrkqznq888d-eon-0.1.0`
    prove native startup then picker; the initial installed
    `/nix/store/2axg20ry5dl48dflcc8bq57yg07cp1bd-eon-0.1.0` proves early key
    dismissal without picker leakage, disabled startup, popup rendering/removal,
    same-pane reattachment without replay and caller-PTY `eon anima`.
    Alt+Shift+A is advertised and routed by the accepted physical-shortcut path;
    that private compositor run did not supply a physical keyboard observation.
  - **Native creation:** `eon-decide-yazi-picker-creation-e7z`, base
    `5566a7a7c591132af2f20315712fc217673f8663`, prepared source
    `/nix/store/pibi7h41h3ndkdhm3zgniq3xd68s937b-eon-source`, installed
    `/nix/store/n5p1k52p079i1a7405w6l74lgyg1h592-eon-0.1.0`, Yazi
    `aa526434f00bb44e2e902d9a4ac5f810da1018b9` passes file and trailing-slash
    directory creation, separate acceptance, input/overwrite cancellation and
    retained creation after picker cancellation without changing the live pane.
    Exact profile identity, preserved user processes and private cleanup are in
    `/home/lucca/.local/state/eon/proofs/eon-decide-yazi-picker-creation-e7z-native-create-2026-10-06/REPORT.json`.
  - **Native browsing defaults:** The same bead's prepared source
    `/nix/store/rs47x5kw6sh5ylznjb1n980njn24y54p-eon-source`, installed
    `/nix/store/k82afnzs7khf8b5c3iym0b1jmvggdl96-eon-0.1.0`, same child pins,
    passes inherited navigation/history, find/sort, modal input/help Esc and
    text preview. Native creation, separate directory acceptance, Tab mode
    switching and Ctrl+C cancellation preserve the existing pane. The installed
    native report, exact candidate, captures, profile identity and cleanup are in
    `/home/lucca/.local/state/eon/proofs/eon-decide-yazi-picker-creation-e7z-native-defaults-2026-10-06/REPORT.json`.
- **Open proof:** Fractional scale, broader compositors, actual blur, current
  AT-SPI/screen-reader use and broad user environments remain qualified.
  Headless Mesa picker screenshots may intermittently omit a body even on the
  compared prior artifact; accepted frames/scenes were nonblank. No capture
  stability or latency bound is claimed. The tagged public alpha predates Anima.

## EON-C19 — Terminal typography and initial geometry configuration

- **Status:** Proven
- **Consumer:** Eon and EonTerm users opening a native surface.
- **Trigger:** Create or reopen Venus using one `$EON_CONFIG_HOME/config.toml`
  snapshot.
- **Result:**
  - Optional `[terminal]` fields are `font_family`, ordered `font_fallbacks`
    (at most eight), `font_size` (finite 6–96 logical px), `line_height` (finite
    1–3 multiplier) and integer `columns`/`rows` (1–65,535; a supplied pair at
    most 100,000 cells). Names are nonempty trimmed UTF-8, at most 128 bytes
    without controls. The primary family must be installed and monospace;
    requested fallback families must be installed.
  - Omission emits no override, preserving Venus font selection, nominal
    16 px / 1.125 line height and 960×600 logical window. Each omitted dimension
    retains its initial window dimension. Explicit grids include workspace
    chrome; later compositor/user resizing remains authoritative. Reopen reads
    current settings without replacing Orbit or its PTY; live Venus keeps its
    snapshot.
  - The authoritative startup workspace Snapshot transfer and native readiness
    share one absolute five-second deadline, including large snapshots. Ordinary
    Present messages retain their 250 ms write timeout.
- **Important failures:** Eon Runtime rejects malformed configuration with field
  diagnostics before children. Venus admits fonts and actual initial native
  geometry before any new terminal or user command. Rejected, lost or timed-out
  admission closes only the attempted presentation and preserves durable terminals.
- **Owner:** Eon supplies terminal defaults; Eon Runtime owns strict parsing,
  argv projection and authoritative startup Snapshot transfer; Venus owns font
  resolution, native readiness and geometry.
- **Consumes:** Exact accepted Venus VEN-C19 startup admission.
- **Boundary:** Native Linux Wayland; no font installation, live reload, Eon
  renderer, second schema, dependency or default appearance change.
- **Proof:** Initial Eon `b693a877cd4d630fe15aabf852de4e978fc166dc` and
  startup-transfer correction `0b0b6312531229270c157df31ba49800a6daef03`,
  `eon-terminal-typography-geometry-config-b63`.
  - **Environment:** x86_64 Linux, isolated Sway 1.12 / lavapipe 26.1.1; Venus
    `ec80e36625dec73544c0cb64becf4b135c932a63`, Orbit
    `91999d79546422b49bdbc124166a65859d0bd872`, unchanged wire contracts.
  - **Evidence:** Locked Rust, manifest, both Nix packages and flake checks;
    initial EOF/malformed readiness/timeout create no terminal; invalid config
    and rejected reopen preserve the command. Initial artifacts
    `/nix/store/p86cvamyyk5iri47xpdc77kdm8pa267v-eon-0.1.0` and
    `/nix/store/vh88p7zlvmfsag96zd6w5shnnkxi5ydw-eonterm-0.1.0` admit DejaVu
    Sans Mono / Symbols Nerd Font Mono, 20 px / 1.5 and actual 100×30 PTYs,
    including the full-Eon picker. Reopen applies 24 px / 1.25 and 90×28
    with the same Orbit/command; live presentation keeps its snapshot.
    Missing-font failure, initial scales 1/1.25/1.5/2, later resize, physical
    input, exact `Regular` selection/copy, `é界` preedit, `ime-é界` commit,
    AT-SPI text/bounds and recovered-workspace admission pass.
  - **Transfer deadline:** Delayed-reader/stalled-transfer regression, locked
    Rust/Nix and installed
    `/nix/store/cig8r1rifpl2lzswi4ga6l744y3v0ck9-eon-0.1.0` and
    `/nix/store/95x87mc3gdc22fvk920dz6jg6mfr3qxs-eonterm-0.1.0` prove one
    absolute five-second snapshot-write/readiness budget, not the 250 ms normal
    Present timeout. Scale-1.25 real grids and same-terminal reopen pass.
  - **Limits:** This proves the named configuration/startup/typography/IME/AT-SPI
    slice at these revisions. It does not establish current-alpha screen-reader
    use, full fractional/HiDPI coverage, stable-seat IME, broader compositors or
    installed non-systemd support; C7 retains those gaps. Earlier child startup
    refinements remain scoped to their exact Git/Beads records.

## EON-C20 — Discoverable native shortcuts

- **Status:** Proven
- **Consumer:** One installed full-Eon user on native Linux Wayland.
- **Trigger:** The user presses physical Alt+Slash from the Eon workspace.
- **Result:** The exact accepted Venus client toggles one native, read-only
  shortcut viewer containing its fixed Eon-surface bindings and the current
  enabled popup catalog supplied through current EONW v7. Opening, refreshing,
  scrolling, resizing, and closing the viewer change no workspace or Orbit
  terminal state. Title, subtitle and first group heading occupy separate
  readable vertical space at normal and compact window sizes.
- **Important failures:** Eon rejects configured popup shortcuts that collide
  with Alt+Slash or positional Alt+0 through Alt+9 before workspace mutation.
  Invalid or stale component identity fails before profile activation. Profile
  refresh preserves running supervisors and terminals; they retain their older
  client until a normal restart.
- **Owner:** Eon owns component/default selection, profile delivery and
  installed acceptance; Eon Runtime enforces configured popup collisions.
  Venus owns viewer state, input, rendering, focus, and accessibility.
- **Consumes:** Venus VEN-C20 and VEN-C5, current EONW v7 and Orbit terminals;
  the original v5 installed proof remains scoped below.
- **Boundary:** No Eon-rendered viewer, second shortcut catalog, EONW change,
  Orbit update, child-application bindings, rebinding UI, command palette,
  tutorial, telemetry, momentary mode, dependency, or platform expansion.
- **Header-spacing proof:** EON-C4's exact `0d02d4b` child-adoption source and
  installed Eon artifact, consuming Venus VEN-C20 proof
  `8af87bb0d07bc975413b05e3bdea9404ed6c1e57` through selected `c02e371`.
  Native 1100×650 and 320×240 scale-1 captures show separate readable title,
  subtitle and first heading. Scrolling preserves the header/footer; workspace
  state and PTY input remain unchanged, and Escape restores terminal input.
  The original broader viewer proof remains scoped below.
- **Proof:** `eon-deliver-native-shortcut-viewer-rif`, exact working-tree
  candidate over `7b69fb250723fab813446315396f2bf5b04466de` with corrected
  VEN-C5 graph identity.
  - **Environment:** x86_64 Linux, private Sway 1.12 headless/pixman / Mesa
    26.1.2 lavapipe, scale 1; EONW v5, generation
    `g1-584db38a02d9c93b630a6efe99a0802f`. Venus source
    `2f05a408e499117c1ddb57f99255dcdb4a236bdf`, VEN-C5 proof
    `3730201d6dd27e15fdcd1a8b0662aa476a587eb6`, codec source
    `0cc8f477298681ae3945903e8fdb5852d487c5ab`, Orbit
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`. Eon artifact
    `/nix/store/c4cgyhpza8417gpll3qqx2pyg767gr72-eon-0.1.0`, Venus
    `/nix/store/zjmfc4azyw6vlpky11x0sw7a3f3d10h6-yazelix-venus-0.1.0`.
  - **Evidence:** Collision red/green, locked Rust/Nix, both packages and exact
    profile equality; native Alt+/ from pending Project, terminal, chrome,
    custom/rebound Agent popups and pending-tab focus. Enabled/disabled/rebound
    rows, scroll/resize, dismissal/focus restoration, input/workspace isolation
    and AT-SPI projection pass. Child proof owns maximal catalog/live refresh.
    No EonTerm, fractional-scale, screen-reader workflow, wider compositor,
    non-systemd, macOS or direct-distribution proof is added.

## EON-C21 — Native workspace header

- **Status:** Proven
- **Consumer:** One installed full-Eon user on native x86_64 Linux Wayland.
- **Trigger:** The user points at, focuses, scrolls, or activates the top
  workspace row, its tabs, New tab, Shortcuts, active-tab Close, or blank drag
  region.
- **Result:** Full Eon presents one native Eon Bar at the existing tab-strip
  height. Eon-authored tab identity and action policy feed Venus's tab-first
  responsive layout, New tab beside the rightmost tab or pinned at the
  tab-region edge during overflow, fixed Shortcuts and active-tab Close,
  shortcut tooltips, visible keyboard focus, AccessKit buttons, and bounded
  compositor-owned window dragging. The existing CreateTab and exact active
  CloseTab actions and local VEN-C20 viewer remain their sole behavior owners.
  The accessibility tree places New tab before the optional quota chip.
- **Important failures:** Stale component identity fails before activation.
  Rejected Eon actions never mutate another tab. Narrow surfaces retain the
  active tab and required controls without overlap. A failed native drag causes
  no workspace action, terminal input, crash, or state mutation. Controls, tab
  gaps, pane chrome, terminal content, popups, tooltips, and the viewer never
  become drag targets or leak a terminal pointer sequence.
- **Owner:** Eon owns the exact graph, product policy, profile delivery and
  installed acceptance; Eon Runtime owns workspace identity/actions. Venus
  owns header geometry, rendering, input, native drag, tooltips, and accessibility.
- **Consumes:** Venus VEN-C21, current EONW v7 and Orbit terminals; distinct
  header/quota refinements and original component identities are scoped below.
- **Boundary:** C22 separately owns the optional quota projection. The core
  header adds no Eon-rendered chrome, second row, widget framework, agent status,
  polling, configuration, protocol change, inactive-tab close, native window
  controls, macOS claim or distribution expansion.
- **Proof:** Initial Eon `1edb1ed47f2e42a265cc48ced59d8d97e83d9414`,
  `eon-deliver-native-eon-bar-w4d`, and distinct VEN-C21 refinements.
  - **Environment:** x86_64 Linux, private Sway 1.12 headless/pixman / Mesa
    26.1.2 lavapipe at scale 1. Initial Venus
    `2ce3594b9ca0f62a2723d5ce5a005ed3560aea64`, Orbit
    `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`, EONW v5.
  - **Evidence:** Locked Rust/Nix, exact installed
    `/nix/store/fp80c2r8mqh2l9p7fsnn41qpizhbsja9-eon-0.1.0`; pointer,
    keyboard and AT-SPI controls, exact actions, local viewer, drag exclusions,
    real compositor movement and post-drag retirement; eight-tab overflow and
    960/320/100-pixel layouts retain the active tab and all required controls.
  - **Placement and accessibility:** Venus
    `562cacf74e101ee9ea18460c21e47d86eec2376d`, EONW v6
    `3103a00a904c347472899549ce611da765a71650`, Orbit ORBS v12
    `f8ad14e5195109ba8cb421f30e5ae4a9619a1419`, composed artifact
    `/nix/store/31mwvisf7s1aiyp42agslc2wjbz47lms-eon-0.1.0` proves pointer
    CreateTab and those three widths. Accessibility correction at Venus
    `6036430ffed1b84fd2995525e0a19f071809c588`, artifact
    `/nix/store/mzdr9g0qb4z6k5kvrqmshas6i5dzf079-eon-0.1.0`, proves AT-SPI
    order: tabs, New tab, quota, Shortcuts, Close tab, pane panel.
  - **Quota gap:** Venus `6aa79f6405229cd64bd1d55e01c5b11b96657c71`, artifact
    `/nix/store/0j919y26rigssy25w2mz96n9kpxiq7hv-eon-0.1.0`, passes the
    geometry red/green, locked Venus and exact Nix routes; native 960px capture
    and real drag from the restored blank region prove placement beside
    Shortcuts without a broken drag span (`ven-place-new-tab-beside-tabs-pct`).
  - **Limits:** AT-SPI tree/order is not a screen-reader workflow. Native proofs
    retain their exact sources; macOS and wider platform coverage remain open.
    C22 owns quota semantics; no generic widget or agent-status contract follows.

## EON-C22 — Trustworthy Codex quota in the Eon Bar

- **Status:** Proven
- **Consumer:** One full-Eon user with a compatible authenticated `codex`
  executable on the inherited user `PATH`.
- **Trigger:** A full-Eon supervisor starts and a quota refresh becomes due.
- **Result:** Eon starts one dedicated `codex app-server --stdio` child and
  reads `account/rateLimits/read` no faster than once per minute. EONW v7
  carries an optional observation with fresh, stale, blocked, or unknown state
  and at most the Codex bucket's primary and secondary windows. Each window
  contains duration minutes, remaining percentage, and a provider reset time
  when Codex supplies one. Venus presents the exact monochrome OpenAI Blossom
  before the human quota label while hover and accessibility name Codex.
- **Important failures:** A missing executable, auth failure, incompatible or
  oversized response, invalid value, timeout, EOF, provider exit, or network
  failure leaves quota absent or retains reset-bounded last-good windows.
  Retained values say `old` only after a failure and when the last successful
  quota observation is strictly more than one hour old. A younger failed
  observation keeps its label; age alone does not mark a healthy observation
  old. Eon clears failed windows at their reset. An account update clears prior
  quota when authentication ends or becomes unsupported. An authenticated
  ChatGPT update retains the recorded observation until the next scheduled
  read without marking it old. Each verified read replaces the complete observation;
  a response already in flight at an account update cannot republish prior
  facts. Provider I/O stays off startup, workspace actions, and terminal
  attachment; shutdown cleanup is bounded.
- **Owner:** Eon Runtime owns provider process lifecycle, refresh/retry
  enforcement, account-update handling, normalization, freshness and
  the canonical EONW field. Codex owns authentication and rate-limit semantics.
  Venus owns the native chip's layout, exact mark, text, tooltip and accessibility under
  `VEN-C22`.
- **Consumes:** Stable notification/read schemas generated by official installed
  Codex CLI 0.160.1, EONW v7, EON-C21 and Venus VEN-C22.
- **Boundary:** Eon requests no reset-credit details, declines Luna Reserve
  support, and sends no provider mutation. Credentials, plan, credits, upsell,
  transcripts, token history, raw JSON, and other provider-private fields are
  never logged, persisted, or carried over EONW. The worker does not retain a
  private account identifier or merge quota windows across observations.
  This contract adds no bundled Codex, direct HTTP, tokenusage, daemon,
  persistent cache, provider framework, configuration surface, other provider,
  or v5 adapter.
- **Proof:** `eon-codex-quota-chip-disappears-oiw`,
  2026-10-06; Runtime `a57e105a5da87d9c65402c1c4c8ea8b5dc8adc45`, codec
  `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, Venus
  `bbf4cf41b289f441683eb7a3f225a26a4d3edacc`, Orbit
  `b6cecf8f2ee35570b41cfdc578b095889d917fe2`; EONW7 / ORBF2 / ORBS13.
  - **Mechanically verified:** Focused red/green proves failed data has no old
    label at 3,600s and has it at 3,601s, retains failure evidence between
    snapshots, and expires at reset. Age alone leaves healthy data fresh.
    Worker lifecycle covers notifications, obsolete replies, verified
    replacement, sign-out, young failures, retry and cleanup. Full Runtime
    Rust and prepared Eon Rust checks, both Nix product builds, full flake and
    package-selection checks pass; product builds cover 35 Eon and 38 Runtime
    tests. The codec package and selected pin are unchanged.
  - **Installed dogfood:** Eon
    `/nix/store/y7gc7qmvfvmy6plvzgkjl4i51shfs179-eon-0.1.0`, generation
    `g1-1f68a789ecc52c93f99fca7166b9b8b4`; x86_64 Linux, private native
    Wayland Sway 1.12/pixman and DBus. At 960px the same client retains `75%`
    without `old` after an authenticated notification and a failed second
    quota read, spaced at least 60s from the first. Width 960→180→960 hides
    and restores the chip; tabs and fixed controls remain intact. Rendered
    values and accessibility agree. The one-hour boundary uses deterministic
    owner proof, without waiting an hour or changing the host clock.
  - **Delivery and limits:** Active profile elements resolve to the built
    Eon artifact above and EonTerm
    `/nix/store/1yml5hjg8krz03qqq3k33nj9qg2fjag3-eonterm-0.1.0`.
    Private processes/directories were removed; live user terminals were not
    restarted. The original recurring episodes remain uncorrelated; the
    notification gap was reproduced and corrected at the owning worker.

## EON-C23 — Independent Eon windows

- **Status:** Proven
- **Consumer:** A full-Eon user on native x86_64 Linux Wayland.
- **Trigger:** `eon window new`, the desktop launcher's New Eon Window action,
  or Venus's Alt+Shift+N shortcut.
- **Result:** A fresh directory picker opens in a second native window with its
  own Eon supervisor, tabs, focus, and durable Orbit terminals. The original
  window and its terminals remain unchanged. `window new` reports a stable ID;
  `eon windows` discovers it, `eon window attach ID` presents its current
  generation after detach, and `eon window stop ID` stops only its terminals.
  `eon window stop all` covers independent windows and attempts the caller's
  own window last; existing `eon stop all` remains scoped to the caller's
  current runtime namespace. Piped confirmations remain available to later
  windows, one answer per prompt.
- **Important failures:** Identity collision, unsafe runtime path, failed
  startup, or stale target neither replaces an existing window nor stops or
  retargets its terminals. An uncertain startup retains its ID, process, and
  private log for inspection instead of killing possible live terminals.
- **Owner:** Eon Runtime owns window identity, namespace and launch/lifecycle
  enforcement; Eon supplies product composition. Venus invokes the exact Eon
  executable supplied by its supervisor and presents one window per process;
  Orbit owns each terminal and its single presentation attachment.
- **Consumes:** EON-C1, EON-C8, EON-C11, EON-C15, EONW v7, and VEN-C23.
- **Boundary:** Bare `eon` retains its current presentation behavior. Windows
  have independent workspaces; no terminal, tab, pane, or focus state is shared.
  The eight-hex-digit ID is unique by atomic directory reservation, not a
  secret. This does not establish macOS or another distribution channel.
- **Proof:** Eon `bdbe9b5436e4ff8a734210cd25c581983f6f2ed0`,
  `eon-new-eon-window-vkwl`, Orbit
  `b6cecf8f2ee35570b41cfdc578b095889d917fe2`, Venus
  `a23f9eed95120ca4acea8789bf7d32d2472d84ce`, EONW v7.
  - **Environment:** x86_64 Linux Nix and isolated native Sway 1.12 / Mesa
    software Vulkan; installed
    `/nix/store/c3fs2ssgkzzf94x2g8a76hs7y91ckwj1-eon-0.1.0` with exact
    launcher action.
  - **Evidence:** Focused batch-lifecycle red/green, locked Clippy and complete
    flake/package routes; two native windows with two pre-supplied confirmations
    stop the other window before the caller's, producing counts 1 → 2 → 0.
    Earlier same-Venus packaged native proof exercises physical Alt+Shift+N,
    distinct Orbit endpoints, detach/reattach and exact Stop. Evidence remains
    in `eon-c23-batch-order-2026-09-25/` and `eon-c23-native-2026-09-25/` under
    the owning issue's proof root. Failed earlier batch candidates are not
    accepted; C4's source transfer changes no window or confirmation contract.
