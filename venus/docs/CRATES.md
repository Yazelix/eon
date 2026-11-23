# Venus crate decisions

Orbit, Venus, Eon and Eon Runtime commit hashes from before 2026-10-14 in this
file identify revisions outside this repository: history, not proof.
Upstream project revisions keep their usual meaning.

`ven-upt.1` records the user-selected thin winit, wgpu, glyphon, and AccessKit
shape. `ven-upt.2` implements it with exact direct versions and features in
`Cargo.toml`. `ven-4sn` adds the minimum native text-clipboard owner.
`ven-consume-terminal-clipboard-writes-zgh` advances the canonical session
consumer to ORBS v3. `ven-native-clipboard-paste-uas` reuses that clipboard
owner for explicit reads. `ven-c87` adds the exact Eon-owned EONW consumer
without a local protocol mirror. `ven-adopt-orbs-v4-without-interaction-expansion-zdc`
advances the same canonical session consumer to ORBS v4.
`ven-render-live-pane-title-cwd-8c7` advances it to ORBS v5 for read-only pane
metadata without another schema or runtime dependency.
`ven-present-tab-directory-picker-a6v` advances the exact Eon consumer to EONW
v3 for one semantic picker action and one authoritative modal endpoint.
`ven-consume-picker-first-eonw-v4-zd1` advances it to EONW v4 so an active
picker-bound tab may have no panes or selected pane.
`ven-shared-popup-surface-ehk` consumes EONW v5's shared popup projection and
exact targeted actions through the same package and single attachment owner.
`ven-venus-kinetic-touchpad-scroll-5sl` advances it to ORBS v7 for bounded
multi-row preview windows and compositor-paced fractional scrolling.
`ven-venus-native-selection-gestures-d3x` advances it to ORBS v10 for
Orbit-routed native left-pointer selection, authoritative completion revisions,
and destination-tagged copy results.
`ven-accept-continuous-output-scrollback-a1s` advances the exact Orbit source
under unchanged ORBS v10 so relative scrolling remains usable during output.
`ven-select-during-live-output-752` advances it again so Orbit-owned selection
remains usable during compatible output and Begin accepts a nonfuture presented
revision, without changing the protocol package or adding a dependency.
`ven-venus-linux-wayland-only-ndj` selects native Linux Wayland and Vulkan as
the sole host and replaces the clipboard adapter's X11 fallback with its
already-locked Wayland data-control owner.

`ven-scrollback-position-ddo` consumes authoritative display-row distance through
ORBF v2 / ORBS v11 using the same dependency-free canonical package.
`ven-scrollback-return-to-live-j8u` advances the same exact package to ORBS v12
for one zero-payload semantic action without a new dependency or adapter.

## Current decisions

### Native hyperlink dispatch

- **Selected shape:** Existing Linux host `gio open -- URI`, invoked with Rust
  `Command`; no new direct crate or Nix package.
- **Owner consequence:** GIO owns desktop handler selection and launch. Venus
  owns its HTTP/HTTPS admission policy and one dispatcher with a ten-second
  deadline. Missing GIO or a handler produces a visible, accessible failure.
  The existing Wayland clipboard owner copies link targets independently.
- **Rejected:** A browser/opener crate, direct portal dependency, custom D-Bus
  plumbing, a URL-discovery crate (Scene's own scanner detects plain-text
  targets under `eon-nlcy`), and `xdg-open` paths that wait for browser
  lifetime.
  Revisit only if a supported Linux desktop cannot dispatch through host GIO.

### Peer-window Quit admission

- **Selected shape:** serde_json 1.0.151, the exact version already pinned by
  Eon and Runtime, parses a fresh `eon window ls --json` response immediately
  before peer Stop (`eon-7cf0y.6`). No new lockfile package or native dependency.
- **Owner consequence:** Runtime owns the inventory and Stop. Venus checks the
  returned ID/generation pairs only for destructive-action admission; EONW
  remains the sole workspace topology and menu-data owner. This approved
  one-shot check does not replace the workspace protocol with CLI polling.
- **Rejected:** Handwritten JSON parsing, another parser, a shell parsing tool,
  and linking Runtime into Venus. Remove this direct dependency when an accepted
  atomic generation-bound CLI Stop replaces the read/Stop check, if no other
  consumer needs it. The separate read and Stop retain a scheduling race.

### Eon workspace protocol consumer

- **Selected shape:** The in-tree path dependency on the dependency-free,
  publish-false, Apache-2.0 `eon-workspace-protocol` 0.1.0 package (EONW v49);
  before consolidation Venus pinned Eon Runtime revision
  `d0a184bb4d45d3e50787f7c17606df652727fb9a`.
- **Status:** Active for internal development
- **Owner consequence:** Eon Runtime owns canonical EONW values and validation.
  The runtime owns session topology, pending tabs, tab launch directories,
  popup lifecycle, selection, targeted actions and endpoint mappings; Eon
  supplies product policy and composition. Venus owns the request worker and
  native projection.
- **Package identity:** The initial transferred codec tree
  `8409c419a3496d6f01eeef65295f5d95b943313e` equals the former Eon source
  `f41a41c9aecc4c436edfa2f394b832aa6d8711ad`, including its manifest and
  v2–v7 exports. No dependency, feature, build script or workspace inheritance
  is added. Later runtime-only changes may retain this exact codec pin when
  complete package/build inputs remain unchanged; version labels alone do not
  prove equality. `eon-runtime-venus-rebind-tbmn` records source acceptance.
  EONW v9 at the selected revision permits absent provider resets in fresh
  and stale quota windows while preserving historical validators and bytes.
  The dependency/build shape stays unchanged; `eon-codexbar-quota-backend-ioqy`
  records its installed native producer and consumer proof.

### Orbit protocol consumer

- **Selected shape:** The in-tree path dependency on the dependency-free,
  publish-false `orbit-protocol` 0.1.0 package; before consolidation Venus pinned
  Orbit revision `6bc269c40b18f08b95778939518f77556ba91c67`
- **Status:** Active
- **Owner consequence:** Orbit alone owns ORBS v14, ORBF v3, semantic values, history, its display-row position and absolute return-to-live,
  terminal-versus-host left-pointer routing, selection and completion revisions, typed wheel
  outcomes, signed scroll batches, bounded row-window previews, destination-tagged copied
  text, compatible-live-output gesture preservation, terminal clipboard effects, read-only
  pane metadata, bounds, revision reduction, and the accepted lifecycle contracts. Venus
  keeps no mirror terminal schema or viewport authority; its native host supplies events and
  applies clipboard effects.
  The four-click ladder remains Orbit-owned; this exact refresh preserves
  package tree `d1151ce365f371cd06498c8d65fbcf6831ee0afe`, wire versions,
  package version and dependency capability.

### Native host

- **Selected shape:** winit 0.30.13 with Wayland, dynamic Wayland loading, and
  raw-window-handle 0.6 only, patched to exact `Yazelix/winit-0.30` commit
  `c1dacc9d455ff836a415fc97e1e1f15b6c50f0c9`
- **Status:** Active on native Linux Wayland
- **Owner consequence:** The host owns window and event-loop lifecycle, native input and IME
  collection, compositor blur protocol selection, resize, surface recovery, socket
  scheduling, and bounded client failure UX. X11 and Xwayland compatibility are unsupported.
  The Wayland backend keeps each precision scroll axis active until it stops
  and clears both on pointer focus departure. Venus owns scroll momentum.
  Replace the patch with the first accepted stable winit containing the
  background-effect backport, marked-text source-transition correction,
  and the Wayland precision scroll fix.
- **Next IME pin:** The macOS IME integration (eon-yp6vg.47) and root pin
  move (eon-yp6vg.57) must keep this Wayland revision in their winit
  ancestry. Check the proposed IME revision with
  `git merge-base --is-ancestor` and repeat the split-axis and compact-frame
  scroll regressions before moving the root pin.

### GPU and text

- **Selected shape:** wgpu 30.0.1 with WGSL shaders and only the Vulkan backend
  (Metal on macOS) for rendering; glyphon 0.12.0 with its cosmic-text 0.19.0
  re-export; pollster 1.0.1 for bounded initialization. wgpu's `noop` backend is
  also compiled in, so tests reuse the release build's wgpu: wgpu uses it only when
  it is enabled explicitly, which production Venus never does; the headless
  tests in `src/render/tests/text_clip.rs` and `src/render/tests/device.rs` do
- **Status:** Active on Linux/Vulkan
- **Owner consequence:** Venus owns a small rectangle/decorations pipeline and a
  textured-quad pipeline for terminal images (VEN-C31), which draws each terminal's images
  between its cell backgrounds and its foreground and creates image textures inside wgpu
  error scopes. Glyphon owns shaping, fallback, clipping, raster cache, atlas, and text
  preparation. None sees transport or terminal state. Venus can prepare and
  draw on a host's device/queue without owning its presentation or device-loss
  callbacks; standalone rendering uses the same path.
- **Version policy:** wgpu is exact at 30.0.1. The lock selects wgpu-core and
  wgpu-hal 30.0.1 on both native targets (eon-yp6vg.7.12), alongside
  wgpu-types, wgpu-naga-bridge, naga and naga-types 30.0.0. Keep those versions
  until a reviewed GPU update repeats the affected native proof. The core
  patch changes package metadata, and the HAL patch removes the Vulkan image
  acquire fence on non-Windows targets and resolves Metal color-space symbols
  at runtime through the already locked libloading 0.8.9. That edge enters
  the Apple Silicon Venus graph. The noop backend and device-loss code are
  unchanged. The lock keeps wasm-only js-sys/web-sys at 0.3.105, wasm-bindgen and
  its macro, macro-support and shared crates at 0.2.128, and
  wasm-bindgen-futures at 0.4.78. Neither native target builds those seven
  crates. The selected native backends and the cfg_aliases 0.2.2 build
  dependency stay unchanged.
  [wgpu 30.0.1 release notes](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.1)
- **Shared storage:** HarfRust, wgpu and the Linux Wayland stack reach
  smallvec 1.16.2 through the workspace lock (eon-yp6vg.7.13). This is
  the minimum selected for the future host's Parley and Fontique 0.12.0.
  Smallvec owns inline storage and heap spill under MIT OR Apache-2.0.
  Advance it through a reviewed lock update and repeat the affected
  rendering proof. Cargo drops the entry when its last consumer goes.

### Ordered startup font fallback

- **Selected:** direct `unicode-script = "=0.5.8"`, already present through
  cosmic-text 0.19.0; approved for `VEN-C19`.
- **Capability:** name `Script` in cosmic-text's public `Fallback` trait. The
  selected shaper keeps family matching, cluster shaping and platform fallback.
  Neither glyphon nor cosmic-text re-exports this required type.
- **Cost:** one dependency edge; no new package, version, feature, native/build
  dependency or Nix input. MIT OR Apache-2.0. About 40 lines of provider glue
  prepend the bounded startup list; no separate font resolver is introduced.
- **Rejected:** stdlib/fontdb alone cannot own cluster-aware fallback;
  fontconfig does not control cosmic-text's subsequent iterator; a local
  per-character resolver duplicates shaping policy. No additional text stack.
- **Lifetime:** the API requires static family names. At most nine startup
  names remain allocated for the process; live reload is outside this contract.
- **Maintenance:** align the exact type version with cosmic-text. Remove this
  edge/provider when upstream re-exports `Script` or offers ordered runtime
  lists. No new Wayland, IME, accessibility or browser owner is introduced.
- **Check:** a controlled font database reverses the actual selected glyph
  font ID with fallback order, including regular-only symbols under bold text.

### Accessibility

- **Selected shape:** AccessKit 0.25.2 and accesskit_winit 0.34.2 with the Unix async-io
  adapter
- **Status:** Active
- **Owner consequence:** Venus derives native accessibility updates from each accepted
  immutable scene without creating another presentation model. The locked Linux
  adapter exposes standard Click actions but not model-level custom actions, so
  distinct Open and Copy nodes use that supported path.

The Linux cache-signal correction ships in accesskit_unix 0.24.1 (upstream
`bffadac`, AccessKit PR #819). Its signal helper wraps each cache structure as
one D-Bus argument, preserving the libatspi cache contract. All AccessKit
packages come from crates.io at exact versions; the Git source and its Nix
fixed-output hash are gone. `ven-resolve-reader-focus-after-return-to-live-lmt`
owns the Orca return-to-live proof.
Venus's tests use the pinned `accesskit_consumer` as
a dev-dependency, so they feed tree updates through the same validation the
adapters apply.

### Native text clipboard

- **Selected shape:** wl-clipboard-rs 0.9.3 on Linux and target-only arboard
  3.6.1 on macOS, both with default features disabled
- **Status:** Active on native Linux Wayland; implemented on unsupported Apple
  Silicon macOS, with a partial native proof in this repository (`VEN-C16`)
- **Owner consequence:** The host reads ordinary clipboard text once after an explicit paste
  shortcut and writes canonical bounded `CopiedText` and `ClipboardWrite` effects through
  Wayland data-control or the general macOS pasteboard. All macOS destinations collapse to
  that one pasteboard. Orbit owns paste encoding and terminal text. Native unavailability
  remains a visible bounded failure; there is no X11 or macOS primary-selection fallback.

The ORBF v2 / ORBS v11 advance keeps one direct package with no transitive, native, build,
feature, or Nix change. Orbit's canonical codec owns bounded metadata values,
read-only observation, scroll preview windows, and routed left-pointer values;
Venus consumes them without a backport, dual decoder, or adapter. Replacement
remains one exact pin plus exhaustive consumer matches.
Orbit's project-owned source is Apache-2.0 under its root license. This
dependency decision does not establish public-distribution proof.

### Local time zone

- **Selected shape:** jiff 0.2.38 with default features disabled and `std`,
  `tz-system` and `tzdb-zoneinfo` enabled (`eon-igae.4`, the Eon Bar clock in
  [PRESENTATION.md](PRESENTATION.md#eon-bar)). Linux and
  macOS compile only jiff and jiff-core. The lockfile also lists jiff-static,
  behind a `cfg(any())` gate that never builds, and defmt, an optional
  dependency left disabled. Unlicense or MIT.
- **Owner consequence:** jiff resolves the system zone from `TZ`, then
  `/etc/localtime` (by symlink name through the zoneinfo database, else its
  TZif bytes), and rechecks it after five minutes. Venus asks for the zone
  without a fallback, so an unknown zone shows no clock rather than UTC.
- **Rejected:** libc `localtime_r`, because Venus keeps unsafe code to its
  macOS CPU and memory readings; chrono's `Local`, which adds iana-time-zone
  and its own parser for the same job; and the time crate, which refuses local
  offsets in multithreaded Unix processes. Remove jiff if Venus drops the
  clock or the native host gains a safe local time source.

### macOS CPU and memory readings

- **Selected shape:** target-only libc 0.2.189 on macOS, the workspace's exact
  pin, already in Venus's macOS build through core-foundation, so the lock
  gains one edge and no package (`eon-mac-launch-kz4p.3`, the Eon Bar CPU and
  RAM labels in [PRESENTATION.md](PRESENTATION.md#eon-bar)).
- **Owner consequence:** Venus's manifest denies unsafe code for every target
  instead of forbidding it. Six functions of the macOS-only `host` module in
  `src/system.rs` allow it, each with its own attribute (`port`, `cpu_load`,
  `cpu_name`, `load_average`, `per_cpu_load`, `memory`), so a new function there
  does not inherit the allowance. Their eight unsafe blocks cover the host port
  taken once per process, `host_statistics`, `sysctlbyname` for the CPU name,
  `getloadavg`, `host_processor_info` with its borrowed slice and release, and
  `host_statistics64` into an explicitly initialized `vm_statistics64`, each
  with a safety note. The page size comes from rustix's safe `page_size` and the
  physical memory from `NSProcessInfo`, with no unsafe code. Venus tests hold no
  unsafe code. [docs/UNSAFE.md](../../docs/UNSAFE.md) registers the file and
  its allowances. Their results feed the same pure, Linux-tested arithmetic.
  The binary and integration-test roots forbid unsafe code. libc marks its
  Mach calls deprecated in favour of mach2; `port` and `per_cpu_load` allow
  that warning.
- **Rejected:** mach2, a new package whose calls are just as unsafe;
  spawning `vm_stat`, `sysctl` or `top` every 2 seconds, since macOS has no
  cheap command for cumulative CPU ticks; and sysinfo, a second owner for
  readings Venus already parses.
- **Exit:** `eon-b4vh` tracks removing the unsafe code. Remove it when a safe
  standard-library or existing-dependency API covers these readings; if libc
  drops `mach_host_self`, move to mach2 through the dependency gate.

### Disk use reading

- **Selected shape:** direct `rustix = "=1.1.4"` with default features
  disabled and `fs` and `std` enabled (and `event`, below), the exact version
  already in the lock,
  declared for Linux and macOS only (`eon-d7itl`, the Eon Bar disk chip in
  [PRESENTATION.md](PRESENTATION.md#eon-bar)). Approved by Lucca 2026-10-31
  (relayed by eon-overseer). The lock gains one edge and no package or version.
  On Linux rustix 1.1.4 is already in Venus's build through async-io,
  async-process, async-signal, polling (which calloop reaches it through),
  wayland-backend, wayland-client, wayland-cursor, wl-clipboard-rs and zbus, so
  nothing more compiles; on macOS it and errno, both already locked, enter
  Venus's build. `std` lets `statvfs` take a `&Path`.
  MIT OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception.
- **Capability:** a safe `statvfs` for the disk chip on Linux and macOS: the
  filesystem's fragment size and its total, free and unprivileged-available
  block counts. Venus relies only on rustix's documented `fs::statvfs`
  returning its own `StatVfs` with `u64` counts on both backends. Its result
  feeds pure, tested arithmetic (`DiskUse::of`) that computes use as GNU df
  does, used over used plus available, rounded up, and rejects empty or
  overflowing readings; std's `MetadataExt::dev` finds the mount point. Venus
  adds no unsafe code. Other platforms have no reading.
- **Rejected:** the standard library, which has no `statvfs`; libc's unsafe
  `statvfs`, since Venus is removing its unsafe code under `eon-b4vh`; the nix
  crate, a new package for the same call; spawning `df` or `stat -f` every
  reading, a process per reading with a fragile output format; and sysinfo, a
  second owner for readings Venus already takes.
- **Exit:** replacing it costs one call site in `src/system.rs`. Remove the
  edge if Venus drops the disk chip or the standard library gains `statvfs`;
  follow the version the rest of the lock resolves, through the dependency
  gate.

### macOS application menu

- **Selected shape:** target-only macOS `objc2 = "0.6.4"`, `objc2-app-kit =
  "=0.3.2"` (default features disabled; `std`, `NSApplication`, `NSMenu`,
  `NSMenuItem`, `NSResponder`) and `objc2-foundation = "=0.3.2"` (default
  features disabled; `std`, `NSArray`, `NSEnumerator`, `NSString`)
  (`eon-7cf0y.15`, Command+Q under VEN-C8). Approved 2026-11-06 under Lucca's
  standing approval rule, relayed by eon-overseer. Venus also reaches these
  crates through arboard on macOS. objc2 and objc2-app-kit are MIT, and
  objc2-foundation is Zlib OR Apache-2.0 OR MIT.
- **Capability:** in an Eon workspace window, on the main thread, find the
  main-menu item whose action is `terminate:` and clear its key equivalent, so
  Command+Q reaches Venus as a key. Venus relies on AppKit's documented
  `NSApplication`, `NSMenu` and `NSMenuItem` API and on winit building its
  default menu before the first `resumed`. These 0.3 bindings are safe, so
  Venus adds no unsafe code.
- **Rejected:** winit's own objc2-app-kit 0.2.2, whose menu calls are
  `unsafe fn` and would add a second unsafe site; `with_default_menu(false)`,
  which drops the whole app menu (About, Services, Hide, Show All);
  `applicationShouldTerminate:` interception, which also catches the Dock's
  Quit and logout and needs runtime class changes; and raw libobjc calls
  through libc.
- **Version policy:** the workspace lock selects objc2 0.6.5 for Venus
  and mac-proof (eon-yp6vg.7.13), meeting the future host's Fontique 0.12.0
  minimum. Objc2 0.6.5 corrects retention of autoreleased Objective-C
  results. mac-proof pins it exactly. Venus's compatible range is an
  approved exception to the exact manifest pin in
  [Verification](../../docs/AGENT-RULES.md#verification), as approved by Lucca
  for `eon-1cyzr` on 2026-11-11 (relayed through eon-desk: "i agree").
  Advance the locked release through a reviewed update with native proof.
- **Exit:** replacing the menu binding costs one macOS function in
  `src/application.rs`. Remove the menu edges when winit can leave
  Command+Q off its default menu or Venus owns the menu. Reassess the
  shared generation when arboard moves.

### Subscription readiness

- **Selected shape:** the `event` feature of the rustix edge above
  (`eon-61efs.1`); no package, version or lock change. On Linux the crates
  listed above already build rustix with it.
- **Capability:** `event::poll` waits until Venus's EON-C39 subscription is
  readable without holding the lock under which Venus reads it, so the window
  can read the same socket before it applies an Orbit frame. Venus adds no
  unsafe code.
- **Rejected:** a blocking read outside the lock, which can hold bytes the
  window cannot see; a short read timeout under the lock, which stalls the
  window; libc's unsafe `poll`; and polling the socket on a timer.
- **Exit:** one call site in `src/transport.rs`. Drop the feature if the
  standard library gains a readiness wait or peek on `UnixStream`.

### Rune candidate dependency

- **Selected shape:** an optional `rune` Git dependency behind the
  non-default `candidate-rune` feature (`eon-yp6vg.7.6`), with
  `default-features = false` and `features = ["accessibility"]`. Shipping
  packages, ordinary tests and archive producers use the feature-off graph.
  The Nix graph check resolves the candidate separately. Its first user is the
  private entry from eon-yp6vg.7.7: the `rune_candidate` module and the
  `venus-rune-candidate` binary and test, each behind the feature
  (`required-features` for the binary and test). The single switch
  (eon-yp6vg.54) removes the feature.
- **Source:** `https://github.com/Yazelix/eon-rune` at
  `c22cc2d459cc6b80a3f4ecf45b136eed1d205507` (tree
  `1c804c14b592c85afcf9551f023b380f81c96322`, protected branch
  `eon-pin-yp6vg-64`). It applies the typed skip report from `d191ba64`
  (eon-yp6vg.28) on `77cc200d`. That parent carries the X11 gate
  (`7134a9fb`, eon-irxtu) and synthetic-clock overlay tests
  (`488484bc`, eon-yp6vg.62) on eon-yp6vg.7.5's `App::attention_wait`
  commit `52f6f6c9`. For `FrameOutcome::Skipped`,
  `FrameEncoded::skipped()` provides the `FrameSkip` reason
  (`Covered`, `Timeout`, or `Surface`).
  Venus's response to these reasons belongs to eon-yp6vg.7.10.
  Rune's own `Cargo.lock` is unchanged. The stack keeps `caeb0472`'s winit
  configuration without `wayland-csd-adwaita-notitle`. With that feature on,
  sctk-adwaita's `tiny-skia` brought the published `tiny-skia-path 0.11.4`
  beside Rune's vendored copy of the same name and version. nixpkgs
  `importCargoLock` vendors by name and version, so it failed for the whole
  workspace. A root patch onto Rune's copy does not compile either (E0004:
  the copy adds `PathSegment::ConicTo`). The base `caeb0472` follows
  `e267be6e`, the fork rebased onto upstream `55ef5268` and qualified
  under eon-yp6vg.46.
- **Features:** `accessibility` is on, because the chrome needs it.
  Off:
  - `embedded-fonts`: Venus registers its own packaged fonts.
  - `dbus` and `svg`: no Venus surface uses them.
  - `x11`: Eon uses native Wayland on Linux. Rune keeps X11 in its own defaults.
  - `metal-backend` and `ogg-audio`: refused.
  - `tooling`: debug builds keep upstream's tooling paths.
  - `frame-profile`: frame proofs opt in (eon-yp6vg.7.8).
- **Root patch:** `parley_engine` from the same rev in the root
  `[patch.crates-io]`. Only the `candidate-rune` graph reaches it. Registry
  `parley_engine 0.12.0` lacks Rune's `line-dictionaries`, and Cargo ignores
  a dependency's own `[patch]`. The patch moves with the pin. Rune's five
  workspace patches and their dispositions:

  | Rune patch | Changes | Disposition here |
  | --- | --- | --- |
  | `parley_engine` | UAX #9 L1, shaping reuse; needed to resolve | root patch, same Git rev |
  | `wgpu-hal` | DXGI/DX12 only | not applied: Windows only |
  | `accesskit_windows` | Windows only | not applied |
  | `winit` | Windows taskbar and placement; macOS IME APIs (`set_ime_text`, `take_ime_clauses`, `take_ime_replacement`); Wayland finger and continuous scroll gain ×12 | not applied. The IME APIs belong to eon-yp6vg.47/.57, whose winit `78b8f28` has them. Root winit `c1dacc9d` and `78b8f28` both lack the scroll gain. Root winit is shared with shipping Venus, so the decision is eon-yp6vg.7.2's |
  | `accesskit_macos` | `AXHeading`, a main-thread borrow panic guard, text runs past 64 KiB | not applied. A root patch would change shipping Darwin Venus's `accesskit_macos 0.27.2`. Gap for Darwin qualification (eon-yp6vg.58) |

  `tiny-skia-path` is a path dependency inside the Rune Git source, so it is
  vendored from the same rev.
- **Lock:** 472 packages. This pin changes only the source revision of
  the six Git packages. The earlier eon-5a9kx lock change removed
  `as-raw-xcb-connection 1.0.1` and
  `x11-dl 2.21.0` and dropped dependency edges from `arboard`, `rune`,
  `winit` and `x11rb`. Remaining registry versions and checksums match the
  prior lock. The all-target lock still records `x11rb` and
  `x11rb-protocol` through `arboard`. Neither selected Eon graph reaches
  them. Each package name and version appears once.
- **Versions:** in the original 7.6 pin, each added crates.io package
  is set to the version in Rune's own lock, the versions eon-yp6vg.46
  qualified. The minimal resolution chose `image 0.25.9` for the workspace's
  lowest declared Rust (1.85), and it lacks the `LoopCount` API that Rune
  calls. The resolver keeps 15 crates newer than Rune's lock, at
  semver-compatible versions:
  - `icu_collections`, `icu_locale_core`, `icu_locale_fallback`,
    `icu_locale_fallback_data`, `icu_normalizer`, `icu_normalizer_data`,
    `icu_properties`, `icu_properties_data`, `icu_segmenter` and
    `icu_segmenter_data`: 2.4.0, where Rune has 2.3.0;
  - `icu_provider`: 2.4.0, where Rune has 2.3.1;
  - `tinystr` 0.8.5, `yoke` 0.8.4, `zerovec` 0.11.9 and `zerovec-derive`
    0.11.7.
- **Nix:** `workspaceCargoLock.outputHashes."rune-0.0.1"` =
  `sha256-YzugFz3uEg7vGK/+N/YfJyGm0nHfmkSy/ZFmPzudNQg=` (`c22cc2d`), which
  covers the six crates from the rev. Their vendored contents equal `git archive` of
  the rev, apart from the files nixpkgs adds (`.cargo-checksum.json`,
  `.cargo-config`) and its workspace-value substitution in `Cargo.toml`.
- **Licenses:** Rune is MIT. The pin adds no registry packages or license
  terms. The two packages removed from the lock were MIT
  (`x11-dl`) and MIT OR Apache-2.0 (`as-raw-xcb-connection`). The original
  83 packages added in 7.6 had these licenses:

  | Licenses | Count |
  | --- | ---: |
  | Unicode-3.0 (ICU4X) | 22 |
  | MIT | 16 |
  | MIT and Apache-2.0 combinations | 24 |
  | Zlib, MIT and Apache-2.0 combinations | 10 |
  | BSD-3-Clause or BSD-2-Clause, alone or combined | 4 |
  | Unlicense OR MIT | 2 |
  | CC0-1.0 combinations | 2 |
  | 0BSD combination | 1 |
  | Zlib | 1 |
  | MPL-2.0 | 1 |

  The one MPL-2.0 package, `spellbook 0.4.2`, is a wasm32-only Rune
  dependency. It is locked but built for no Eon target.
- **Rust version:** the highest declared `rust-version` among the added
  packages is 1.88 (the parley family and `image 0.25.10`). Rune declares
  none. The workspace builds the enabled graph with Rust 1.96.1: Venus
  clippy and tests with `candidate-rune`. The fork's own locked builds and
  overlay tests use its pinned Rust 1.99.1. The fork's native Apple Silicon
  qualification used the same toolchain with defaults off and accessibility
  on. It exercised all three hidden acquisition arms with injected misses and
  observed a subsequent hidden draw for each.
  [Native Rune qualification](https://github.com/Yazelix/eon/actions/runs/45128427810).
- **Feature-off identity:** with the feature off, `cargo tree --locked` is
  identical to the parent on `x86_64-unknown-linux-gnu` and
  `aarch64-apple-darwin`. That covers 68 trees: every workspace package and
  the workspace, with edges `normal,build` and `normal,build,dev`. No tree
  contains `rune`, and no workspace package depends on Venus. The shipping
  Linux closure, binaries, notices and source companion match the parent
  under the 7.6 comparison rules. Darwin package and workspace evidence
  follows the lead's landing checks.
- **Enabled graph** (normal edges, unique packages):
  - **Linux:** 210 feature-off, 277 with the candidate, down from 284 at
    `52f6f6c9`. The candidate graph drops `arboard`,
    `as-raw-xcb-connection`, `gethostname`, `percent-encoding`, `x11-dl`,
    `x11rb` and `x11rb-protocol`. winit uses Wayland with its X11 and
    client-side decoration features off. Rune and Venus share the existing
    `wl-clipboard-rs 0.9.3` dependency. wgpu gains `parking_lot` and
    `metal`, with Vulkan still the Linux backend. `accesskit_unix` remains
    the source of `zbus`.
  - **Darwin:** 130 → 202, from resolution only, because eon-yp6vg.57
    blocks the build. wgpu gains `custom`, and shipping `accesskit_macos
    0.27.2` gets Rune as a second dependent.
- **Graph guard:** eon-5a9kx replaces the temporary eon-yp6vg.7.14 X11
  waiver. The `candidate-rune-graph` check in `flake.nix` resolves the
  candidate against the locked vendor for the platform. It rejects Rune's
  `default` and `x11` features, winit's `x11` feature and X11 client crates.
  The Linux eon-tests check and the Darwin package check both run it.
  The Linux candidate entry still requires `WAYLAND_DISPLAY`.
  Related work:
  - eon-yp6vg.58 owns the `accesskit_macos` gap.
  - eon-yp6vg.7.2 owns the scroll gain.
- **Version policy:** the pin moves only by a reviewed fork commit on a
  protected branch. Each move updates the `rev` in both manifests and the
  Nix hash, and repeats the lock alignment and the checks above.
- **Rejected:**
  - Pinning `e267be6e` unchanged: its CSD feature breaks Nix vendoring.
  - A root `tiny-skia-path` patch: it does not compile.
  - Renaming Rune's vendored crate: this works too, but it changes a
    package name that upstream Rune shares.
  - Switching the workspace to `fetchCargoVendor`: it changes every Rust
    vendor and the notice tooling's paths.
  - Copying Rune's other patches to the root: they change shipping
    identities.
  - A default or packaged feature: the host is not qualified yet.
- **Exit:** the single switch (eon-yp6vg.54) replaces the feature with the
  shipping host, or removes the dependency if the Rune host is dropped. The
  `parley_engine` root patch goes with it.

## Measured comparison

Minimal Rust 2024 scratch binaries were resolved and checked on Rust 1.96.0.
Counts include the scratch root; lock counts include cross-target entries, while
tree counts are unique Linux normal/build output lines. The implemented lock
contains 336 packages including Venus, `eon-workspace-protocol`, and
`orbit-protocol`; its current Linux
normal/build tree has 272 unique lines.

| Complete shape | Exact releases | Lock packages | Linux tree | Disposition |
|---|---|---:|---:|---|
| winit + wgpu + glyphon + AccessKit | 0.30.13, 30.0.0, 0.12.0, 0.24.1/0.33.2 | 324 | 267 | Selected: four packages over the owned-atlas shape remove its highest-risk custom subsystem. |
| winit + wgpu + cosmic-text + owned atlas | 0.30.13, 30.0.0, 0.19.0 | 320 | 262 | Text fallback only; it adds an estimated 700–1,200 specialized atlas, shader, upload, and cache LOC. |
| winit + softbuffer + cosmic-text + tiny-skia + AccessKit | 0.30.13, 0.4.8, 0.19.0, 0.12.0 | 297 | 232 | Rejected as the primary renderer because it makes HiDPI composition and full-frame upload CPU work and weakens the intended GPU path. |
| winit + Vello + Parley + AccessKit | 0.30.13, 0.9.0, 0.11.0 | 336 | 301 | Rejected because it imports general vector/rich-layout policy, ICU and native fontconfig, and uses wgpu 29 rather than 30. |

## Ranked tradeoffs

1. **winit + wgpu + glyphon + AccessKit** best fits the first Venus slice.
   Glyphon removes the risky custom atlas while preserving a narrow scene and
   renderer seam. The costs are a 324-package lock, wgpu device and surface
   recovery, and a required native proof for glyph fidelity and redraw latency.
2. **winit + wgpu + cosmic-text + an owned atlas** gives Venus control over
   cell placement, cache policy, uploads, and batching. It saves four lock
   packages but adds an estimated 700–1,200 specialized GPU and cache LOC. Use
   it only after a focused glyphon failure.
3. **winit + softbuffer + cosmic-text + tiny-skia + AccessKit** removes the GPU
   requirement and exposes a direct CPU framebuffer with damage support. CPU
   rasterization, HiDPI bandwidth, platform-dependent presentation copies, and
   a later renderer replacement make it weaker as the product architecture.
4. **winit + Vello + Parley + AccessKit** offers a rich vector scene and text
   layout foundation. Its alpha renderer, compute requirement, wgpu version
   skew, and general UI policy exceed the accepted one-window surface.

All four shapes passed `cargo check --locked`. Shared features enable only raw
window handles, wgpu's standard library, and WGSL. Linux adds Wayland with
dynamic loading, Vulkan, AccessKit Unix, and wl-clipboard-rs. Apple Silicon
macOS adds Metal and arboard while AccessKit selects its AppKit adapter automatically.
Wayland, xkbcommon, the Vulkan loader/driver, font discovery, and AT-SPI/D-Bus
remain Linux runtime/Nix surfaces. AppKit, Metal, and the system accessibility
API are native macOS surfaces; the stack adds no C++, Zig, or vendored engine.

Full UI frameworks Iced 0.14.0 and Slint 1.17.1 own unnecessary widget,
layout, runtime, and renderer policy for one custom surface. Skia-safe 0.99.0
adds a large C++ binary/build surface; femtovg 0.26.0 makes OpenGL and another
text stack architectural. Sugarloaf at Rio 0.4.5 remains conditional on a
proved glyphon failure. Ghostling and libghostty-derived shapes import terminal
authority or demo-grade game rendering. A completely owned stack would
duplicate windowing, GPU, shaping, font fallback, and accessibility.

## Native background blur decision

Stable winit 0.30.13 exposes blur but binds only KDE's Wayland protocol. The
selected exact fork retains `fb45fbf901fbe70cc9a877b5d651d0b60c206b08`,
which backports the merged upstream `ext-background-effect-v1` implementation:
200 additions and 30 deletions across eight winit files. The fork also carries
the AppKit IME guard and the Wayland precision scroll fixes. No new Venus
dependency, native library, build tool, service, unsafe code, or event-loop
owner is added. One winit remains shared with `accesskit_winit`.

The Git workspace resolves `dpi` 0.1.1 instead of registry 0.1.2. The older
source lacks only the later no-std and inset additions, which neither winit
0.30.13, AccessKit, nor Venus consumes. The locked package and Linux tree counts
remain unchanged. Direct Wayland protocol ownership, upstream master, and a
winit replacement were rejected because each adds substantially more lifecycle
and portability cost for the same request. Exact Venus proof
`7fc7e4ba97aaf48b586002934a580ef2d1c31694` resolves one shared winit, passes
the complete locked suite, and visibly applies blur on COSMIC Wayland 1.0.0 at
`091583ac84abac02967ae358cf9570ddfef63b31`. Other compositors remain best
effort because the public API exposes neither capability nor acceptance.

## Native AppKit IME decision

The selected fork adds one condition to winit's existing input-source
transition: while AppKit reports marked text, keep the current IME state until
`interpretKeyEvents` can commit or update that composition. Source changes with
no marked text retain the existing `Ime::Disabled` event. Physical Kotoeri
proof converted `にほん` and committed selected `日本` exactly once to Orbit.
Upstream PR 4087 remains rejected because its broader rewrite routes ordinary
input through IME commit; corrected two-Enter A/B showed PR 4650 unnecessary.
The IME-qualified commit `7c209ec5bebbd2963f75a39f79b320af6b03f72c` has the
same tree as guard-only commit `92722876415d5f2e3e97b2897b84c9a768c79b82`.
The current winit pin adds the Wayland precision scroll fixes on top of that
tree. The package graph, public API, and input owners stay the same.

## Native clipboard decision

wl-clipboard-rs 0.9.3 owns the operation the Rust standard library does not
provide: read and host native plain-text clipboard values through Wayland
data-control. The package was already locked behind arboard, so selecting it
directly adds no resolved package and removes arboard's unconditional X11
implementation and fallback. Venus maps Orbit's standard destination to the
regular clipboard and selection or primary to the primary clipboard.
Ctrl+Shift+V or the native Paste key reads at most the canonical 1 MiB limit
plus one byte, validates UTF-8, and sends one bounded paste to Orbit. Missing
data-control remains a visible bounded failure.

External `wl-copy`, `xclip`, and `xsel` commands were rejected as undeclared
Linux runtime dependencies. A cross-platform arboard edge was rejected because
its Linux backend can carry and fall back to X11; the selected edge exists only
on macOS. There, arboard requests and writes only the general pasteboard string
type. It adds two active Apple-target packages, though six other platform
packages enter the shared lock. Reads materialize the complete native string
before Venus enforces Orbit's 1 MiB delivery limit; the user accepted that
allocation ceiling for this proof, and no partial value reaches Orbit.
`pbcopy`/`pbpaste` were rejected after a real M1 probe showed rich-format
fallback and classification that violate exact plain-text transfer. Direct
AppKit would add local unsafe code. smithay-clipboard would add another package
and raw-display lifecycle for no current Linux contract gap. Handwritten
Wayland ownership was rejected as unsafe protocol code for one bounded effect.
A future browser client replaces this isolated effect with the browser
clipboard API rather than carrying a native backend across the boundary.

M1 run 34832511106 at Venus source
`0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` proved one Unicode terminal
clipboard write through arboard and one Unicode Command+V paste into exact
Orbit input. M1 run 34831678536 displayed distinct failure notices for PNG-only
and oversized values without sending PTY bytes. Deterministic checks cover
empty, non-text, oversized, and invalid UTF-8 admission. Native Command+V and
the empty or non-text notice are re-proved in this repository (`VEN-C16`); the
host-access failure notice remains open.

## Eon workspace protocol decision

The canonical Eon Runtime package adds one direct lock entry and no transitive, native,
or build dependency. Copying it would duplicate every EONW tag, bound, identity rule, and snapshot
invariant. Parsing Eon CLI output for topology, shelling out for topology, and
consuming the older private control format remain rejected. The peer-window
Quit admission check above is a separate, approved destructive-action guard.
Provider relocation keeps the complete package
unchanged and adds no dependency cost; an adapter or local mirror is not a
replacement path.

## Selected owner seam

One pure scene reducer maps canonical complete Orbit frames to immutable,
revision-tagged quads, exact grapheme runs, cursor data, and accessibility
inputs. It owns no socket, window, GPU, or terminal handle.
The native host maps winit events only into `orbit-protocol` values, keeps
transport outside the renderer, and sends typed wakeups through
`EventLoopProxy`. The selected shape adds no general async application runtime.
The same host owns one native clipboard adapter. It reads after an explicit
paste shortcut and writes the exact `CopiedText` or `ClipboardWrite` effect
returned by Orbit.

Venus derives drawing, accessibility, and deterministic contract snapshots from
the latest accepted scene. After a successful GPU present, the renderer
enables pointer events until resize or surface recovery invalidates that
presentation. AccessKit may publish the accepted scene while the surface is
occluded or recovering because assistive technology must not wait for GPU
presentation. Both views derive from the same scene and keep no independent
content model.

The renderer caches shaped cell layers for equal rows at the same terminal and
row slot. Fractional scrolling rebuilds placement and clipping from those layers;
changed rows reshape independently. Cache entries retain only visible rows and
partial clip edges, with no terminal history. Each changed frame still prepares
the complete text-area stream and rectangle scene. Damage is derived locally
after measurement and never becomes wire state. The implementation is smaller
than the gate estimate; the README scorecard records the exact owned size.

Cursor bodies and the selected cursor's trail use the existing dynamic rectangle
batch, with each terminal's clips. Cursor-only blink phases reuse cached grid
rectangles and text preparation; SGR-blinking foregrounds retain the phase in
the grid key. Native and hosted presentation share this preparation path.

If corpus and native checks show that glyphon cannot preserve exact graphemes,
cell alignment, clipping, color glyphs, or acceptable sustained redraw latency,
retain the protocol, scene, winit, wgpu, and AccessKit owners and reopen only
the text-renderer gate against an owned cosmic-text atlas and current
Sugarloaf. Do not replace the application architecture.
