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

### Eon workspace protocol consumer

- **Selected shape:** The in-tree path dependency on the dependency-free,
  publish-false, Apache-2.0 `eon-workspace-protocol` 0.1.0 package (EONW v15);
  before consolidation Venus pinned Eon Runtime revision
  `d0a184bb4d45d3e50787f7c17606df652727fb9a`.
- **Status:** Active for internal development
- **Owner consequence:** Eon Runtime owns canonical EONW values and validation.
  The runtime owns workspace topology, pending tabs, tab launch directories,
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
- **Owner consequence:** Orbit alone owns ORBS v13, ORBF v2, semantic values, history, its display-row position and absolute return-to-live,
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
  `7c209ec5bebbd2963f75a39f79b320af6b03f72c`
- **Status:** Active on native Linux Wayland
- **Owner consequence:** The host owns window and event-loop lifecycle, native input and IME
  collection, compositor blur protocol selection, resize, surface recovery, socket
  scheduling, and bounded client failure UX. X11 and Xwayland compatibility are unsupported.
  Replace the patch with the first accepted stable winit containing the
  background-effect backport and marked-text source-transition correction.

### GPU and text

- **Selected shape:** wgpu 30.0.0 with Vulkan and WGSL only; glyphon 0.12.0 with its
  cosmic-text 0.19.0 re-export; pollster 1.0.1 for bounded initialization
- **Status:** Active on Linux/Vulkan
- **Owner consequence:** Venus owns a small rectangle/decorations pipeline. Glyphon owns
  shaping, fallback, clipping, raster cache, atlas, and text preparation. Neither sees
  transport or terminal state.

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

- **Selected shape:** AccessKit 0.24.1 and accesskit_winit 0.33.2 with the Unix async-io
  adapter
- **Status:** Active
- **Owner consequence:** Venus derives native accessibility updates from each accepted
  immutable scene without creating another presentation model. The locked Linux
  adapter exposes standard Click actions but not model-level custom actions, so
  distinct Open and Copy nodes use that supported path.

The Linux cache adapter is patched to exact `Yazelix/accesskit` revision
`0f9d186ec6f5e66fe4ccc131ca318dc2725c12f5`, based on upstream
`c88605b96d04431f9c3c792464a0f2f253480e94`. Its shared signal helper wraps
each cache structure as one D-Bus argument, preserving the libatspi cache
contract. One wire regression checks both encoded signal headers and values.
The four existing packages `accesskit`, `accesskit_consumer`,
`accesskit_atspi_common` and `accesskit_unix` share this pin so sibling paths
retain one Rust type identity. Their versions, features and dependency edges
are unchanged; the three sibling source trees equal the registry releases.
No Venus focus workaround, extra package, native library or build tool is added.
Nix consumers need one fixed-output hash for the shared Git source. Replace
this patch with an accepted upstream release passing the cache-wire and real
Orca return-to-live selection checks. `ven-resolve-reader-focus-after-return-to-live-lmt`
owns the decision and scoped delivery proof.

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
  `tz-system` and `tzdb-zoneinfo` enabled (`eon-igae.4`, VEN-C27). Linux and
  macOS compile only jiff and jiff-core. The lockfile also lists jiff-static,
  behind a `cfg(any())` gate that never builds, and defmt, an optional
  dependency left disabled. Unlicense or MIT.
- **Owner consequence:** jiff resolves the system zone from `TZ`, then
  `/etc/localtime` (by symlink name through the zoneinfo database, else its
  TZif bytes), and rechecks it after five minutes. Venus asks for the zone
  without a fallback, so an unknown zone shows no clock rather than UTC.
- **Rejected:** libc `localtime_r`, because Venus forbids unsafe code; chrono's
  `Local`, which adds iana-time-zone and its own parser for the same job; and
  the time crate, which refuses local offsets in multithreaded Unix processes.
  Remove jiff if Venus drops the clock or the native host gains a safe local
  time source.

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
200 additions and 30 deletions across eight winit files. Its only additional
change is the AppKit IME guard below. No new Venus dependency, native library,
build tool, service, unsafe code, or event-loop owner is added. One winit
remains shared with `accesskit_winit`.

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
The selected tree is identical to guard-only commit
`92722876415d5f2e3e97b2897b84c9a768c79b82` and adds no package, API, Venus
branch, or second input owner.

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
invariant. Parsing Eon CLI output, shelling out, and consuming the older private
control format remain rejected. Provider relocation keeps the complete package
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

The first slice rebuilds and redraws the complete scene. Damage is derived only
after measurement and never becomes wire state. The implementation is smaller
than the gate estimate; the README scorecard records the exact owned size.

If corpus and native checks show that glyphon cannot preserve exact graphemes,
cell alignment, clipping, color glyphs, or acceptable sustained redraw latency,
retain the protocol, scene, winit, wgpu, and AccessKit owners and reopen only
the text-renderer gate against an owned cosmic-text atlas and current
Sugarloaf. Do not replace the application architecture.
