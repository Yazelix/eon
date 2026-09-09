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
`91999d79546422b49bdbc124166a65859d0bd872` with canonical ORBS v10 and Venus
`8211e8f773140429398154ca3f2230bd09fed1df`. Exact source, installed artifact,
environment, and remaining gaps belong to each contract's proof below.
Profile refreshes preserve existing live supervisors and their Sessions.

## Pill-shaped tab integration

- **Accepted source:** Eon `57fb875cc02c7fe8c2cd1134315e777a17f15f73`, owned by
  Eon Desktop's `ven-pill-workspace-tabs-rg4`. Only Venus source and
  its qualified `VEN-C8` proof advance to `8211e8f773140429398154ca3f2230bd09fed1df`.
- **Build and installation:** Exact manifest/lock checks and both Nix builds
  pass, including 121 Venus tests and 60 Eon tests per product. Refreshed profiles
  match Eon `/nix/store/afyf5irjfla637z331dh7kai9y4kf8g8-eon-0.1.0` and EonTerm
  `/nix/store/h3yjpcwxpfynibjjfl15k3kv5bhzhj77-eonterm-0.1.0`, sharing Venus
  `/nix/store/3scn7dx9k8f4716n8dcwdlh4r92acwaf-yazelix-venus-0.1.0`.
  Installed generation: `g1-403a8d6820e4e49db056f00774fb8c75`.
- **Installed observation:** Private Sway 1.12 headless/pixman, packaged Mesa
  26.1.2 lavapipe, x86_64 Linux at scale 1 shows pill ends without an underline,
  selected/idle fills, readable labels, native selection, hover and rounded tab
  focus. All 21 pre-existing runtime PID/start identities survive; tests exited.
- **Limits:** Fractional-scale, wider compositor, accessibility and non-systemd
  qualifications remain. This refinement does not re-prove picker rendering or
  resolve the intermittent captures recorded below. EonTerm receives shared
  child artifact/build proof, without a separate native interaction observation.
- **Evidence:** Exact source/artifact hashes, logs, captures and cleanup are in
  `~/.local/state/eon/proofs/ven-pill-workspace-tabs-rg4/installed/REPORT.md`.

## Rounded adaptive tab integration

- **Accepted source:** `eon-install-rounded-adaptive-tabs-5eu` at Eon
  `b25da77fb61ed1e82a237d22ffd0c7e8394ce7df` selects accepted Venus
  `f5c679443b2fbda1a7a8f91c98d312811bee2cdf` and its qualified `VEN-C8` proof.
  All other component/proof identities remain unchanged. Venus owns
  presentation, while Eon owns composition.
- **Mechanical proof:** Manifest/lock identity and manifest validation pass.
  Both Nix packages build, including 121 Venus tests and 60 Eon tests per product.
- **Installed artifacts:** Both refreshed profiles match the working-tree builds:
  Eon `/nix/store/7b710jwv0rfikvviq9pfsj7axrcncc23-eon-0.1.0` and EonTerm
  `/nix/store/jw92cbrz254fpxm33imj943lfh8klks5-eonterm-0.1.0`, sharing Venus
  `/nix/store/kjgxd1l76xkasrsl9xm201rfxfciwagp-yazelix-venus-0.1.0`.
  Eon's installed generation is `g1-7609c80491099c03586945f4320f9cbd`.
- **Installed observation:** x86_64 Linux, private Sway 1.12 headless/pixman,
  packaged Mesa 26.1.2 lavapipe, scale 1. Rounded natural/capped tabs, full-path
  hover, pointer/keyboard selection, stable launch directories after shell `cd`,
  overflow and whole-strip wheel routing pass. Gap scrolling changes 14,992
  strip pixels while preserving the workspace snapshot and pane-header pixels.
  Tab switches picker modes and Escape cancels; rendering limits remain below.
- **Limits:** Intermittently blank or incomplete picker captures occur in both
  the preceding installed package and this source; their client/compositor
  cause remains unresolved. This does not prove uninterrupted picker rendering.
  Existing fractional-scale, broader compositor, accessibility and non-systemd
  qualifications remain. No new EonTerm native interaction proof is claimed.
  All 15 pre-existing runtime PID/start identities survive; private tests exited.
- **Evidence:** Source/artifact identities, logs, captures, baseline comparison,
  reproduction scripts and cleanup are retained in
  `~/.local/state/eon/proofs/eon-install-rounded-adaptive-tabs-5eu/REPORT.md`.

## Venus cell-buffer allocation integration

- **Accepted source:** `7b7382395dad6608fdda40814c2dbf33806dd0e3` selects
  Venus `541a9cb43c155b8b97069904593dc81c73682613`. Only the child source pin
  changes; all existing protocol and contract-proof identities remain exact.
  Venus owns the allocation correction; Eon owns its composition.
- **Mechanical proof:** Manifest/lock identity and manifest validation pass.
  `nix build path:.#default path:.#eonterm --no-link --json` passes, including
  120 ordinary Venus tests and 60 Eon tests in each product build.
- **Installed artifacts:** `nix profile upgrade eon eonterm` matches both
  working-tree outputs: Eon
  `/nix/store/5bw42dbymran4nwv5p64izm7nnfp45ha-eon-0.1.0` and EonTerm
  `/nix/store/nqdpbgmp0a1w102s22ldnf8r97vgm1pg-eonterm-0.1.0`, using Venus
  `/nix/store/2gaa5ky5fxnal39hkj4yd3kr4lkznjc7-yazelix-venus-0.1.0`.
  Installed wrappers and live generation reports confirm the selected child.
- **Installed observation:** Three paired fresh-process runs per product on
  x86_64 Pop!_OS 24.04, isolated Sway 1.12 headless/pixman and Mesa 26.1.2
  lavapipe, scale 1. The fixed Unicode payload emits 10,030 rows; actual grids
  are 120 by 30. A terminal-query barrier and three-second settle precede PSS
  and USS sampling. All 12 full-display captures match their product baseline
  byte for byte; both products' captures were inspected. Full Eon also crosses
  picker cancellation before starting the workload.

Venus process memory in MiB; before/after values are medians. Paired PSS savings
show the median and observed minimum–maximum, not a confidence interval.

| Product | PSS before → after | Paired PSS saved | USS before → after |
|---|---:|---:|---:|
| Eon | 122.45 → 118.10 | 4.48 (4.20–4.70) | 119.02 → 114.65 |
| EonTerm | 121.65 → 117.01 | 4.66 (2.64–4.73) | 118.19 → 113.55 |

- **Evidence:** Exact old/new package identities, raw rollups/maps, 48 process rows,
  captures, build logs, cleanup, `measure.py` and `analyze.py` are retained at
  `~/.local/state/eon/proofs/eon-venus-memory-7b7382395dad6608fdda40814c2dbf33806dd0e3/`.
  The archive's `README.md` gives the reproduction commands using a fresh
  short disposable root and the recorded packages on a private display.
- **Limits:** Small samples on a warm host with unfrozen shared mappings;
  software Vulkan only. These results do not replace the COSMIC/Intel benchmark
  or prove GPU allocation, throughput, long-term growth or retained-history
  efficiency. Earlier fractional-scale, broader compositor, input/IME,
  accessibility and non-systemd qualifications remain unchanged. All 12
  pre-existing runtime PID/start identities survive; owned test generations and
  compositor are gone. Existing supervisors retain their old runtime until
  restarted; the profile refresh preserves their live Sessions.

## EON-C19 — Terminal typography and initial geometry configuration

- **Status:** Proven; initial composition at
  `b693a877cd4d630fe15aabf852de4e978fc166dc`, startup-transfer correction at
  `0b0b6312531229270c157df31ba49800a6daef03`.
- **Consumer/trigger:** Eon and EonTerm create or reopen a Venus surface using
  one `$EON_CONFIG_HOME/config.toml` snapshot.
- **Fields:** Optional `[terminal]` `font_family`, ordered `font_fallbacks`
  (at most eight), `font_size` (finite 6–96 logical px), `line_height` (finite
  1–3 multiplier), and integer `columns`/`rows` (1–65,535; a supplied pair
  at most 100,000 cells). Names are nonempty trimmed UTF-8, at most 128 bytes
  without controls. A primary family must be installed and monospace;
  requested fallback families must be installed.
- **Result:** Omitted settings emit no override, preserving Venus's font
  selection, nominal 16 px / 1.125 line height and 960 by 600 logical window.
  Each omitted dimension retains its initial window dimension. Explicit grid
  requests include existing workspace chrome; later compositor/user resizing
  remains authoritative. Reopen applies current settings without replacing
  Orbit or its PTY child; a live Venus retains its snapshot.
- **Failures:** Eon rejects malformed configuration with field diagnostics
  before starting children. Venus must admit fonts and the real window's
  initial native geometry before Eon starts a new Orbit Session or user
  command. Rejected, lost or timed-out admission closes only the attempted
  presentation and preserves existing durable Sessions.
- **Owners:** Existing Eon terminal configuration and direct argv projection;
  Venus owns font resolution, native readiness and geometry. Eon supplies an
  authoritative startup workspace snapshot through its existing constructors.
- **Boundary:** Native Linux Wayland; no font installation, live reload,
  renderer inside Eon, second schema, dependency, or default appearance change.
  Consumes VEN-C19 startup admission only after exact child acceptance.
- **Proof:** Accepted Venus `ec80e36625dec73544c0cb64becf4b135c932a63`;
  Orbit `91999d79546422b49bdbc124166a65859d0bd872`, unchanged wire contracts.
  Locked fmt/check/test/clippy and manifest validation pass (70 Eon tests).
  All four declared Nix checks and both working-tree profile builds pass.
  Initial EOF, malformed readiness and timeout create no Session in either
  product; invalid config and failed reopen preserve the existing command.
- **Installed artifacts:** Refreshed Eon
  `/nix/store/p86cvamyyk5iri47xpdc77kdm8pa267v-eon-0.1.0` and EonTerm
  `/nix/store/vh88p7zlvmfsag96zd6w5shnnkxi5ydw-eonterm-0.1.0` match their
  exact working-tree outputs, using Venus
  `/nix/store/37v7m8kb88fgs4lgwk7ajcbf9hjb8pn7-yazelix-venus-0.1.0`.
- **Installed observations:** x86_64 Linux, isolated Sway 1.12 / lavapipe
  26.1.1. Both products accept DejaVu Sans Mono with Symbols Nerd Font Mono,
  20 px / 1.5 and a real 100 by 30 PTY grid, including Eon's initial picker.
  Reopen applies 24 px / 1.25 and 90 by 28 while preserving Orbit and command
  PID/start identities; live presentation keeps its prior settings. Missing
  fonts fail initial launch before the command and reject replacement without
  losing the existing Session. Fresh PTY resize observations cover initial
  scales 1, 1.25, 1.5 and 2; later manual resize remains effective.
  Native physical input, exact `Regular` selection/copy, `é界` preedit and
  `ime-é界` commit, and AT-SPI terminal text/bounds pass in both products.
  Eon also admits its recovered workspace after isolated supervisor loss,
  retaining the same Orbit and command. Captures were inspected.
- **Evidence and limits:** Retained under
  `~/.local/state/eon/proofs/eon-c19-b693a877cd4d630fe15aabf852de4e978fc166dc/`.
  All seven pre-existing supervisor/Orbit identities remain unchanged.
  Existing supervisors retain their old runtime until restarted. This proves
  the named configuration/startup slice; earlier fractional/HiDPI, compositor,
  non-systemd and stable-seat IME qualifications remain unchanged.
- **Startup-transfer correction:** Workspace snapshot writes and readiness
  reads share the absolute five-second startup deadline. Large snapshots do
  not inherit the 250 ms timeout used for ordinary Present messages. At
  `0b0b6312531229270c157df31ba49800a6daef03`, a delayed reader receives the
  complete canonical snapshot; a stalled transfer remains bounded. Focused
  red/green, all 71 Rust tests, locked check/clippy, manifest validation, and
  all four Nix checks pass. Both refreshed working-tree outputs match their
  installed profiles: Eon
  `/nix/store/cig8r1rifpl2lzswi4ga6l744y3v0ck9-eon-0.1.0` and EonTerm
  `/nix/store/95x87mc3gdc22fvk920dz6jg6mfr3qxs-eonterm-0.1.0`.
  Isolated Sway 1.12 / lavapipe 26.1.1 at 1.25 scale admits a real 100 by 30
  PTY in both products, including Eon's picker. EonTerm reopens at 90 by 28
  with unchanged Orbit/command identities. All seven pre-existing user
  supervisor/Orbit identities remain unchanged. Venus and Orbit pins and the
  earlier typography/IME/accessibility proof and qualifications above remain
  unchanged. Evidence:
  `~/.local/state/eon/proofs/eon-c19-0b0b6312531229270c157df31ba49800a6daef03/`.

## Venus startup correction acceptance

- **Accepted source:** `e10a01ee93d6d32d7312d1c82ff6f524aca31583` selects
  Venus `1b32e5ba7105d14f654136578a65c97a25b53fc1`. VEN-C19 and the
  VEN-C1/C2/C4 startup slices advance for font-only geometry rejection and
  input-free native admission. Orbit, wire versions and other proof identities
  remain unchanged.
- **Mechanical proof:** Manifest/lock identity, manifest validation and all
  four declared `nix flake check path:.` checks pass.
- **Installed artifacts:** `nix profile upgrade eon eonterm` resolves both
  elements to their exact working-tree builds:
  Eon `/nix/store/zkf8s7d7bmyiigr6j6yh8rbvlhskr5ri-eon-0.1.0` and
  EonTerm `/nix/store/8chmkj9mcpz56r90389lskai8w4lqv47-eonterm-0.1.0`.
  Both launch `/nix/store/k1pdjvzha5skyrp2nsvn0ih6gz6lj2j6-yazelix-venus-0.1.0/bin/yazelix-venus`;
  generation `g1-9808cd2409ce97f6bfcc4fe1fec3bbaa`.
- **Installed observation:** x86_64 Linux, isolated Sway 1.12 / Mesa 26.1.1
  lavapipe at 1.25 scale. EonTerm yields 90 by 31 cells in its real Orbit PTY.
  Eon visibly renders its initial directory picker in a 960 by 600 logical
  window; its workspace snapshot identifies the picker with no durable pane.
  Private generations stop through their supervisors. All seven pre-existing
  user runtime processes retain their original start identities.
  Reproduction inputs, observations and capture are retained under
  `~/.local/state/eon/proofs/ven-c19-1b32e5ba7105d14f654136578a65c97a25b53fc1/`.
- **Scope:** This correction proves startup. Earlier post-picker input, text,
  IME and accessibility evidence retains its exact revisions below and in
  VEN-C19. Partial fractional-scale, broader compositor and non-systemd claims
  remain qualified. Persistent typography configuration is a separate issue.

## Venus typography component acceptance

- **Accepted source:** `0461737272447fa976d13bd3376c8a894cebf324`, selecting
  Venus `9157f7fbed0318d94a0697c01a23de2bed86946a` and its VEN-C19 proof.
  VEN-C1/C2/C4 advance only for the child’s startup slice; other child proof
  identities, Orbit, ORBS v10 and EONW v4 remain unchanged.
- **Mechanical proof:** Manifest/lock identity and the manifest validator pass.
  `nix flake check path:.` passes all four declared checks, including both
  affected package builds and their Rust checks.
- **Installed artifacts:** After `nix profile upgrade eon eonterm`, both profile
  elements resolve to the exact working-tree builds:
  Eon `/nix/store/p415mdfn9m962n6znywgb9pp02w0i7zi-eon-0.1.0` and
  EonTerm `/nix/store/n4w4dqckvyqlldb5sfxy2apsilfk0ya3-eonterm-0.1.0`.
  Both select `/nix/store/8bpsbn12ss9xbh1adacwq11vix674hxw-yazelix-venus-0.1.0/bin/yazelix-venus`;
  generation `g1-d2013f41b2900ffafdce030f4f904bd1`.
- **Installed observation:** x86_64 Linux, isolated Sway 1.12 / Mesa 26.1.1
  lavapipe at 1.25 scale, real Orbit PTYs and native keyboard input. The
  default 960 by 600 windows yield 90 by 31 terminal cells in EonTerm and
  90 by 28 below Eon’s tab/pane headers; both deliver exact `ok` plus newline.
  Task-owned generations stop through their supervisors. Existing user
  processes retain their original identities and remain running.
- **Scope:** This refresh consumes the accepted child capability; persistent
  typography configuration is the separate Eon issue. Venus’s configured-grid,
  fallback, IME and accessibility evidence stays in VEN-C19. Earlier partial
  fractional-scale, broader compositor and non-systemd claims remain qualified.

## Hyperlink inspection input correction

- **Accepted source:** `07867f99d176012a9d34a61aa37b5de9ce0888af`, selecting
  Venus `cf3a9169f2cd95d42c689134a52d51d9147ff37c` for `VEN-C2`/`VEN-C5`.
  Other child proof identities, Orbit and wire versions remain unchanged.
- **Proof:** Exact manifest/lock validation and all four Nix checks passed.
  Both profile elements matched their built artifacts after refresh:
  Eon `/nix/store/cnjiijnybw8xdx675jni951g3xs1q647-eon-0.1.0` and
  EonTerm `/nix/store/2ap1p5p42809c0i00ih9lncmrdq6kdi3-eonterm-0.1.0`.
  Both launch `/nix/store/fxf4xyadmiwn7m8ndc0i0gxzcbrq5r2p-yazelix-venus-0.1.0/bin/yazelix-venus`;
  generation `g1-e6c0ad9f6f66837f05c233ec7490e1d0`.
- **Installed observation:** Isolated x86_64 Linux/Sway 1.12, Nix Mesa 26.1.2
  lavapipe, native virtual-keyboard input and a real Orbit PTY. Keycode 240
  carrying `a` typed outside inspection, was captured inside it, and typed again
  after Escape. Exact link copy passed. Venus's Application regression separately
  proves captured repeat/release pairing through dismissal. Existing live user
  processes were preserved. Earlier distinct hyperlink and partial proofs below
  retain their exact revisions and limits.

## Native hyperlink composition acceptance

- **Accepted source:** `be9d0e37c7d0029d6832ff609082faa885280762`, consuming
  Orbit `91999d79546422b49bdbc124166a65859d0bd872` and Venus
  `d1223463b2b513c04242df50a4bd948d0533cfde` with `VEN-C5`;
  ORBS v10, ORBF v1 and EONW v4
  remain unchanged. This acceptance covers the changed launch, graph and native
  interaction surfaces of `EON-C1` through `EON-C4`; distinct earlier evidence
  below retains its recorded revisions.
- **Environment:** x86_64 Linux, isolated Sway 1.12 native Wayland, Nix Mesa
  26.1.2 lavapipe, host GIO 2.80.0, Wayland clipboard and AT-SPI.
- **Mechanical proof:** Launcher environment regression red/green, locked
  fmt/check/test/clippy (68 tests), manifest validation, and all four
  `nix flake check path:.` checks passed.
- **Installed artifacts:** Both active profile elements matched
  `nix path-info path:.#default` and `path:.#eonterm` after
  `nix profile upgrade eon eonterm`:
  - Eon: `/nix/store/8r53v42plymxhys76mv582za20l382qa-eon-0.1.0`
  - EonTerm: `/nix/store/q441sjfvgvkq00yim4gd97f540pmnbg5-eonterm-0.1.0`
  - Venus: `/nix/store/746dv1wk3pmmizz18d4v238p0s69qmf9-yazelix-venus-0.1.0`
  - Generation: `g1-c08957b2a62711a8cc8e3366417366cb`
- **Installed dogfood:** EonTerm passed keyboard inspection and target paging,
  exact clipboard and real registered-handler dispatch, unsupported/malformed
  target refusal, wide-tail hover, ordinary terminal mouse reports, captured
  modifier-click, bottom-row notice placement, stale held-click rejection, and
  accessible native handler failure. Full Eon passed startup-picker cancellation
  into `t1/p1`, target inspection, exact copy and native dispatch. The handler
  preference existed only in host `XDG_CONFIG_HOME/mimeapps.list`; Venus retained
  that root and received the distinct product `EON_CONFIG_HOME`.
- **Limits:** Dispatch proves delivery to the registered handler, not browser
  content loading. Fractional scale, broader compositor quality and non-systemd
  installed support remain unproved. Existing live supervisors were preserved;
  they retain their old components until the user restarts them.

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
- **Proof:** `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`
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
    starvation, snap-to-live, or timeout-truncated momentum. Synchronized-scroll
    composition proof `e1a5a9e02f7102cef48b25a83f740ea716647fa1` remains retained.
    The current acceptance validates the exact graph and both installed profile
    artifacts, passes `nix flake check` (including both package test surfaces),
    and exercises the installed commands on isolated Sway 1.12 native Wayland.
    EonTerm preserves anchored history while PTY query replies continue during
    appends, split 16 KiB DEC 2026 output, and active-screen redraws. Cell drag,
    word/line repeat-click and drag, both clipboards, explicit frozen copy,
    further history movement, and return to live pass. Full Eon has two durable
    tabs and three panes; native Alt+H/L, F6 and tab/pane arrows, and displayed
    tab clicks reach the expected EONW identities during output. Exact
    observations and inherited regression evidence live in
    `eon-accept-live-output-input-nxh`; no performance bound is claimed.

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
- **Proof:** `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`
  - **Environment:** x86_64 Linux Nix alpha
  - **Evidence:** Manifest parser/compatibility checks, exact graph consumption
    by installed Eon and EonTerm, and prior Eonova proof; accepted manifest base
    `234ad77ced4924d95400b5c39822b3fb9b928c95` and ORBS v7 graph refresh
    `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`. The listed proof revision selected Orbit
    `91999d79546422b49bdbc124166a65859d0bd872` and Venus
    `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` with ORBS v10 and ORBF v1.
    The subsequent acceptance records above identify the current graph.

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
- **Proof:** `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`
  - **Environment:** Nix-built x86_64 Linux alpha
  - **Evidence:** Exact protocol-source substitution, package tests, and
    evaluator-absence checks; accepted launch-input base
    `a2792cc77f8254cc277d64eb41f65725b69ccf70` and exact installed ORBS v7
    composition `b44d968e38474fc5b75a41bcde2ad750da0d1e3e`;
    exact installed artifacts are recorded in `eon-accept-live-output-input-nxh`.

## EON-C4 — Thin orchestrator ownership

- **Status:** Proven
- **Consumer:** Eon, Orbit, Venus, Helix, Yazi, and Ratconfig integrations.
- **Trigger:** Eon composes or controls a product surface.
- **Result:** Eon owns launch policy, topology, component selection, product
  configuration, updates, and distribution without duplicating terminal,
  rendering, editor, file-manager, or configuration state.
  Native desktop launches preserve host XDG preferences and use
  `EON_CONFIG_HOME` for the product configuration root.
- **Important failures:** A missing child contract returns to that child instead
  of being reconstructed in Eon.
- **Owner:** Eon orchestration and product policy; each child keeps its subsystem
  authority.
- **Consumes:** Accepted child contracts through EON-C2's exact graph.
- **Boundary:** No copied child schema, hidden fork, compatibility adapter, or
  second terminal/rendering owner.
- **Proof:** `91c6ed5d51b5a09d5c9e1d2e30aebc191c10223f`
  - **Environment:** x86_64 Linux composed alpha
  - **Evidence:** Component-boundary checks, management consumer proof, package
    closure inspection, and accepted installed native-Wayland runtime dogfood;
    accepted composition base `a2792cc77f8254cc277d64eb41f65725b69ccf70`;
    the current acceptance adds no Eon scrolling or input implementation,
    compatibility adapter, dependency, or second component graph

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
    stopping existing Sessions. Eon owns the strict boolean and launch
    projection; Venus owns geometry, rendering, input and accessibility.
  - Left/right tab traversal and up/down pane traversal wrap at ordered edges
    when the axis has at least two targets; singleton axes remain unavailable.
    Left/right remains available while a directory picker is open: the picker
    stays bound to its original live tab while another active tab presents its
    selected pane. Tab navigation by identity also remains available. Users can
    create panes in another tab; creating a tab requires no open picker.
  - Moving left/right swaps the active tab with exactly one adjacent tab;
    moving up/down swaps the selected pane with exactly one adjacent pane in
    the active tab. The moved stable identity remains selected. Movement stops
    at ordered edges and remains unavailable in the picker-bound active tab.
    Venus maps Ctrl+Alt+H/L and Ctrl+Alt+K/J directly to these actions without a
    mode.
  - Closing names the expected active `tN` and requires another live tab. A
    non-final pending tab first stops its exact picker, then disappears and
    restores its prior tab. A durable tab closes unless it owns the picker:
    Eon stops its Orbit Sessions one at a time in pane order and prunes each
    confirmed end through the normal Session-exit owner. Complete success
    removes the tab with the same deterministic focus rule as natural exit.
    If a later stop fails, no later Session is stopped, the action reports
    failure, and every remaining live Session stays represented in the
    partially pruned tab. A Session that ended naturally during the request is
    reconciled and pruned as an exit rather than reported as a successful Stop.
    Venus maps Alt+Shift+W directly to this stable-target action; lowercase
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
  the active tab, final-tab close, durable close in the picker-bound tab,
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
- **Picker scope evidence:** EON-C18 records the installed other-tab action and
  close proof while a picker remains bound to its own tab.
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
  - **Header correction (accepted):** Source
    `0f2b76d2153db8e5716d0cccd78b26802684b5be` consumes accepted Venus
    `94b15af20d1798b648f4d9945fd6bb647f10add8` and unchanged Orbit `91999d7`.
    Installed Eon `/nix/store/j4n3j5r9mfj7xpkj6sjnb9cns11ffsly-eon-0.1.0`
    matches that source output. `ven-2fq` in Eon Desktop records exact
    source hashes, package checks, and isolated Sway 1.12 scale-1 observation:
    two real Sessions retain their directory identities and the long second
    label stays visible with underscores. The private generation stops cleanly;
    existing user Sessions remain running. Fractional scale remains unproved.

  - **Nova tab shortcuts (accepted):** Eon source
    `fea679260dd81fcdbb907e804156b00244af2324` consumes Venus
    `0791f00926cd5cc4fedcc7737aeac7bb68569aff` with unchanged Orbit `91999d7`.
    Nix manifest/lock checks and both package builds pass. Installed Eon
    `/nix/store/578xxkf17cin8sdn6pb5rr10hxz4yfqg-eon-0.1.0` and EonTerm
    `/nix/store/1xg4yr3fpppgnblgqhizx831bfa9cibv-eonterm-0.1.0` match the
    current-tree outputs. `ven-yp4` records isolated Sway 1.12 scale-1 native
    Alt+Shift+T creation, inherited directory, Alt+Shift+W pending/durable
    close, final-tab protection, and exact Ctrl+T/Ctrl+Shift+W terminal bytes.
    The private generation stops cleanly; existing user processes and workspace
    remain unchanged. Fractional scale and other partial proofs remain open.

  - **Pane frames (dogfooded, 2026-09-09):** Working-tree candidate over
    `4f0650764874f3c7d9ab211f057522f2ecc92de4` consumes accepted Venus
    `5c23b0fe206fb2ed9e027603ae61ba366913cbe1`, unchanged Orbit `91999d7`
    and EONW v4. Exact input hashes, installed artifacts, commands, captures
    and cleanup are retained under
    `~/.local/state/eon/proofs/eon-pane-stack-delivery-jfo-2026-09-09/`.
    Locked Rust checks pass (72 tests), as do all four declared Nix checks.
    Refreshed Eon and EonTerm profiles match their working-tree build outputs.
    Private Sway 1.12 / Mesa lavapipe 26.1.2 at scale 1 proves omitted/true/false,
    native keyboard and pointer focus, terminal input, long labels, overflow,
    picker layout and initial grid accounting. Omitted and explicit true have
    identical one/three-pane captures. Invalid configuration rejects a fresh
    launch and a replacement; valid reopen applies frames without replacing
    the supervisor, six Orbit Sessions or the test command. EonTerm emits no
    frame override and has no workspace chrome. All 23 pre-existing runtime
    PID/start identities survive; older supervisors retain their prior runtime.
    Fractional native scale, other compositors and screen-reader qualifications
    remain unchanged.

  - **Connected pane stack (dogfooded, 2026-09-09):** Working-tree candidate
    over `4f0650764874f3c7d9ab211f057522f2ecc92de4` consumes accepted Venus
    `8389cc011adbbf9c390a912d6c93b0040e47e21d`, unchanged Orbit `91999d7`
    and EONW v4. Eon Rust inputs are unchanged from the preceding pane-frame
    delivery. Manifest validation and both Nix package builds pass. Profiles
    resolve exactly to Eon
    `/nix/store/8mh0zv0njykwvfk9cirmmc30lpjc1jbx-eon-0.1.0` and EonTerm
    `/nix/store/y74fg1g2smzlaxqz530s0l7pxk5bwczy-eonterm-0.1.0`, generation
    `g1-216b179a560fe9af5c70a15dce222bf1`. Private Sway 1.12 / Mesa lavapipe
    26.1.2 at scale 1 proves default/off, top/middle/bottom selection, native
    focus/input, overflow scrolling, picker and unchanged real PTY grids.
    Pixel checks prove the continuous side border and inset separators;
    frame-off hides both. EonTerm has no frame argument or workspace chrome.
    All 25 existing runtime PID/start identities survive; private generations
    stop normally and their process scan is empty. Exact inputs, artifacts,
    commands, captures and cleanup are retained in
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-2026-09-09/REPORT.md`.
    Older supervisors retain their previous runtime. Fractional native scale,
    other compositors and screen-reader qualifications remain unchanged.

  - **Earlier pane-stack polish (dogfooded, 2026-09-09):** Its margin and
    highlight design is superseded by the correction below. Working-tree candidate over
    `b29f0d7d205c03a2d9b8d81ee600499dfe2f9af6` consumes accepted Venus
    `ff426a9f3d6e1acbb7ae1ce2388df7c75f7bb271`, unchanged Orbit `91999d7`
    and EONW v4. Eon Rust inputs remain unchanged. Manifest validation and both
    Nix builds pass; profiles and executable symlinks match Eon
    `/nix/store/540j6gnfbkh9k79ba27zg5jm1np83d1d-eon-0.1.0` and EonTerm
    `/nix/store/lrvdp6d9vm91v641k5sginr0l6rzbfli-eonterm-0.1.0`, generation
    `g1-23256a5c404a78d4a68164edae640415`. Private Sway 1.12 / Mesa lavapipe
    26.1.2 scale-1 installed checks prove full-width separators, bottom spacing,
    lighter selected headers, default/off, native focus/input, overflow and
    picker. Real reopened workspace grids are 100x30 in both modes, preserving
    the same supervisor, Orbit Sessions and PTY command; standalone remains
    100x30 without workspace chrome. All 28 pre-existing runtime PID/start
    identities survive; private generations stop normally and cleanup is empty.
    Exact inputs, artifacts, captures and checks:
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-polish-2026-09-09/REPORT.md`.
    Existing supervisors retain their previous runtime. Fractional native scale,
    other compositors, screen-reader usage and offline lifecycle remain qualified.

  - **Pane-stack corrections (dogfooded, 2026-09-09):** Working-tree candidate
    over `b29f0d7d205c03a2d9b8d81ee600499dfe2f9af6` consumes accepted Venus
    `bc5a2bd3b2363abdea69c4cd89953b612bc970e8`; Orbit, EONW v4 and Eon Rust
    inputs remain unchanged. Manifest validation and both Nix builds pass.
    Profiles and executable symlinks match Eon
    `/nix/store/52xxap7klyzcg1fdvgwnz7w30v24q2gx-eon-0.1.0` and EonTerm
    `/nix/store/xbkp5n2m3hz9da7xcspk61aidp6x3brh-eonterm-0.1.0`, generation
    `g1-5626ef23db3adb92c987e2c6179a793c`. Private Sway 1.12 / Mesa 26.1.2
    scale-1 proof over a colored underlay confirms a 4px gap matching terminal
    color/opacity, stack-shaped selected headers and full-width separators in
    default/off modes. Native focus/input, overflow and picker pass. Real 100x30
    reopened workspace PTYs preserve their supervisor, Orbit Sessions and command;
    standalone remains 100x30 without workspace chrome. All 33 pre-existing
    runtime identities survive; private generations stop and cleanup is empty.
    Exact inputs, artifacts, pixel oracles, captures and limits:
    `~/.local/state/eon/proofs/ven-connected-pane-stack-0ub-corrections-2026-09-09/REPORT.md`.
    Sway proves alpha/color coverage, not compositor blur; the existing full-surface
    blur request is unchanged. Fractional native scale, other compositors,
    screen-reader usage and offline lifecycle remain qualified. Existing user
    supervisors and Sessions were preserved; live surfaces need reopening.

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
  without PID/process-name fallback or duplicate Orbit launch. Orbit startup
  and Ready negotiation remain bounded by the shared five-second deadline.
  Eon requires no cgroup membership or delegation; Orbit owns bounded PTY
  shutdown through its accepted ORB-C12/ORB-C13 management boundary.
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
- **Cgroup-free launch (accepted):** Source
  `76b9f5635d9ae545dadb0326976f309afae8fd25`, with exact runtime/test hashes
  recorded in `eon-uj7`; generation `g1-0052f7c409b6505fc4180b69142cc5fe`.
  Eon `/nix/store/p4hbh2jlr6i22mah8r3f5g2lc696m0r6-eon-0.1.0`
  and EonTerm `/nix/store/i53rvda2crdyv4fzsy6vyawzgvd4yb9f-eonterm-0.1.0`
  were verified against that candidate's Nix outputs. Locked Rust checks and Nix flake
  checks pass. With `/sys/fs/cgroup` hidden by an empty private tmpfs, both
  installed products launch real Orbit PTYs and native Sway 1.12 Wayland
  surfaces, preserve exact Orbit/child identities across detach and reopen,
  and complete management Stop with child reaping and generation cleanup.
  Existing user Sessions remain running. This does not promote non-systemd
  support, fractional scale, whole-descendant cleanup, or other candidate
  contracts; Orbit's bounded shutdown remains authoritative.

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

- **Status:** Proven; Tab switching and cancellation are accepted at the exact
  runtime revision and native Wayland scope recorded below.
- **Consumer:** One person starting or using full Eon.
- **Trigger:** Eon needs the first pane for a new tab, or the person presses
  Alt+Z in an existing tab while no directory picker is already open.
- **Result:**
  - A fresh workspace and every later new tab begin as one active pending tab
    with no durable pane. Eon starts one transient Orbit Session running the
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
    Enter uses the current folder; Tab returns to quick search and preserves
    the browser directory for the next visit. Esc, q, Q, and Ctrl+C cancel
    from the main folder list. Esc first dismisses a nested prompt, help screen,
    or search. Startup, new-tab and Alt+Z pickers share these keys.
    `g h` reaches home, `g /` root, `g Space` accepts a typed path, and `.` toggles
    hidden entries. The fixed
    picker keymap excludes file opening and file-management operations.
    Files remain visible for orientation. Browsing does not add Zoxide history.
    Yazi owns directory enumeration and navigation; Eon performs no recursive
    filesystem scan. A vanished or inaccessible final folder fails EON-C17
    validation without mutation.
  - Each selection screen shows its own persistent footer: quick search offers
    Use directory, Browse folders, and Cancel; Yazi offers Use current folder,
    Quick search, Shift+Z Jump, Cancel, and Help; nested jump search offers Jump
    and Back to folders. Browser shortcuts are absent from quick search. Eon's fixed Yazi
    status configuration replaces file metadata with these picker actions.
    Browser hints appear only while the folder list owns input; native Yazi
    prompts and overlays hide them until the folder list regains focus.
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
  - Left/right focus actions continue to traverse and wrap live tabs
    while the picker stays bound to its original tab. Another active tab shows
    its selected pane; returning shows the same picker for normal acceptance or
    cancellation. Tab navigation by identity remains available. Other tabs
    allow pane creation, pane focus by direction or identity, pane/tab movement,
    explicit directory updates, and stable-target close. These actions preserve
    the picker's binding and Session. Mutations of the picker-bound tab remain
    blocked except directory acceptance and closing its non-final pending tab;
    that close stops the transient Session before removing the tab. Cancellation
    restores the previous tab if it survives, otherwise the nearest surviving
    tab; with no tabs left, the workspace ends. Creating a second picker or a
    picker-first tab remains unavailable until the current picker exits.
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
  owns the transient PTY and child. Zoxide owns ranking, fzf owns quick
  selection, and Yazi owns filesystem browsing. Eon consumes one directory
  result without interpreting Yazi's navigation state.
- **Consumes:** EON-C17; accepted EONW v4 pending-tab boundary, Orbit's accepted
  Session startup, attachment, exit, and stop contracts, and exact Venus v4
  consumer `d2d798099934dcf8037bfad6ab856e40c9b989fe`.
- **Boundary:** No custom filesystem walker, editor/file-opening integration,
  copied Nova plugin, normal-pane identity, arbitrary-command or
  generic-popup API, simultaneous terminal composition, native Venus picker,
  placeholder shell, pane replacement, current-process `cd`, persisted pending
  tab, EonTerm action, or additional platform.
- **Tab switching proof (accepted, 2026-09-08):**
  `eon-picker-mode-switching-6bo` verifies the shared startup, new-tab and Alt+Z
  behavior at `f75c910a6a8488bdad3c614544769080261e4560`. Focused red/green
  process checks cover raw-path roundtrips and cancellation; the Nix package
  passed 32 unit and 28 integration tests. The installed profile resolves to
  `/nix/store/024j1lxhwmxwgmkh4r4yxkz0hd2p1qyh-eon-0.1.0`, generation
  `g1-d63f3ad400d96f42927e95ad88b4a7a8`. A private Sway 1.12 native Wayland
  check exercised both modes, nested Esc, Enter and cancellation, preserved the
  existing process CWD, and started a later pane in the retargeted directory.
  Source hashes, physical-key proof, captures and cleanup are retained at
  `~/.local/state/eon/proofs/eon-picker-mode-switching-6bo-2026-09-08/`.
  All 13 pre-existing runtime process identities survived; their runtime was
  not restarted. This acceptance does not refresh fractional-scale, broader
  compositor or accessibility proof.
- **Contextual footer proof (dogfooded):** Working-tree candidate over
  `9cf712e031924f874a681036d931c24aa906b29d`, with exact runtime file hashes in
  `eon-picker-context-hints-j57`; regular Eon artifact
  `/nix/store/qyxb0m5f19kplh6qbxknm3b63gzhyw31-eon-0.1.0`, generation
  `g1-16cf396352bdc4d42084df844387477b`, unchanged Orbit `91999d7` and Venus
  `e13970e`. `nix build path:.#default --no-link` passed 31 unit and 27
  integration tests; the refreshed profile equals `nix path-info path:.#default`.
  On x86_64 Linux with isolated Sway 1.12 native Wayland, quick search resized
  from 1100×750 to 720×600 with a readable footer. At 720×600, Yazi and nested
  jump search showed their respective hints without overlap; Esc restored the
  browser footer and F1 opened help. Filter and change-directory prompts hid
  the browser hints; submitting or dismissing input restored the hints without
  committing the tab. Jump without commitment, untracked-child
  acceptance, actual new-pane CWD, ranked acceptance, and Ctrl+C cancellation
  passed. Both fzf footers use the terminal foreground after the initial visual
  check exposed unreadable default footer text. Private proof processes were
  stopped; existing user Sessions and the EonTerm profile were preserved.
- **Quick-search exit proof (dogfooded):** Runtime source
  `9a49d214e708f97a4c14c42e11d89f7d7b8cb7c5`, installed regular Eon artifact
  `/nix/store/02bfniixmyzxbqgknx3njw0fm96iyzmw-eon-0.1.0`, generation
  `g1-f95b129cfb05da8addc0c1cd9481e51a`, unchanged accepted Orbit `91999d7`
  and Venus `e13970e`, on x86_64 Linux with isolated Sway 1.12 native Wayland.
  The focused Rust regression and final `nix flake check path:.` pass;
  `nix profile upgrade eon` resolves to the exact working-tree artifact.
  Blocked-history regression cases cover Ctrl+C, Esc, Enter and picker failure,
  including reaping, browser launch, exact selection and original diagnostics.
  Installed native Esc reaps real packaged Zoxide blocked opening a private
  FIFO and opens packaged Yazi while retaining the pending workspace. Enter
  commits the folder and launches its pane at the observed shell CWD; the
  existing pane and private history remain unchanged. The proof generation and
  compositor were stopped; user Sessions were preserved. The fixture uses the
  build environment's tool PATH, so its blocked producer also runs inside Nix.
- **Picker review proof (dogfooded):** Runtime source
  `4298fbb8868752e3d6c8eb4fd79fae067ab3e2a1`, regular Eon artifact
  `/nix/store/008jbiqsprwvyn2w7bfnc7rqhd1vk36x-eon-0.1.0`, generation
  `g1-dce7143d7806f6edcd9ccb535267e0bb`, with unchanged Orbit `91999d7`
  and Venus `e13970e` on x86_64 Linux, isolated Sway 1.12 native Wayland.
  The locked Rust workspace passes 69 tests; `nix flake check path:.`,
  `nix profile upgrade eon`, and exact installed-artifact comparison pass.
  Focused regressions prove cancellation reaps a blocked history child,
  successive picker instances have distinct endpoints, and managed cleanup
  and crash recovery consume those exact endpoints. Installed empty-history
  Browse, Shift+Z, untracked-child commitment, new-pane CWD, browser cancel,
  ranked acceptance and quick cancel pass. Immediate cancel/reopen remains
  attached to the current picker; native Ctrl+C reaps actual packaged Zoxide
  blocked opening a private FIFO and restores the prior workspace. Two native
  Alt+Z/Ctrl+C retarget cycles in the same tab use distinct endpoints and retain
  all panes. The earlier reused-endpoint failure and faulty natural-exit test
  fixture are recorded in the owning bead; no latency bound is claimed.
- **Browser discovery proof (dogfooded):** Runtime source
  `e1896db32e1780ad8c29db40a3f22fa9b6bbf252`, installed regular Eon profile
  `/nix/store/7mw0bjk12r5mbw5r5br3sg77nm0f62s5-eon-0.1.0`, generation
  `g1-4548313276c17a701bd60a3f5a480af9`, on x86_64 Linux with native Wayland
  under an isolated headless Sway compositor. `nix profile upgrade eon` built
  the affected package and passed 31 unit plus 27 process/integration tests;
  `nix path-info path:.#default` matched the installed profile artifact.
  - Empty-history Esc opened Yazi at the launch directory while retaining the
    exact pending-tab snapshot. Shift+Z jumped to a seeded parent without
    commitment; navigating into an untracked child and pressing Enter started
    `p1` there. Creating `p2` used the same directory; both actual shell CWDs
    were observed through isolated launch logging.
  - q in a later tab's browser restored the previous tab and pane. Ordinary
    quick-search Enter created a tab at the ranked parent; Ctrl+C abandoned
    the next pending tab. The isolated Zoxide history still contained only the
    deliberately seeded parent. Hostile ambient fzf/Yazi picker options did
    not change these outcomes. Stop removed the three exact proof Sessions
    and their generation; existing user supervisors and Sessions were preserved.
  - Exact packaged Yazi PTY checks additionally covered home/root anchors,
    raw non-UTF-8 and trailing-newline directory output, and q/Q/Ctrl+C
    cancellation. The browser clears inherited `PWD` because Yazi otherwise
    prefers it over the actual Session CWD. A red/green regression protects
    fzf's explicit Esc result with status 1 when there are no matches; other
    picker failures never become implicit Browse actions.
- **Picker scope proof (dogfooded):** Runtime source
  `6879dbc78975c135f9d794759b0fa6485ad207eb`, installed profile artifact
  `/nix/store/23shhmi4q15blg0y2a56718s6xyfz821-eon-0.1.0`, generation
  `g1-06d7a5a01f5f07e88f9191987ac6c19a`, exact Orbit
  `91999d79546422b49bdbc124166a65859d0bd872` and Venus
  `e13970e90289d0d86f0adcbf350e4b9c1d5e5219` on x86_64 Linux, isolated
  Sway Wayland. Focused red/green owner checks and all Nix flake checks passed.
  Installed native pane creation, focus, pane/tab movement, header clicks, and
  Ctrl+Shift+W close preserved the other tab's exact picker record. CLI directory
  updates succeeded and a second picker-first tab failed without mutation.
  Native Enter accepted after the previous tab closed; native Escape in a later
  final pending tab ended the empty workspace and removed its runtime. Existing
  user supervisors and Sessions retained their process identities.
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
