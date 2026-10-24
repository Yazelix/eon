# Venus reference routing

Orbit, Venus, Eon and Eon Runtime commit hashes from before 2026-10-14 in this
file identify revisions outside this repository: history, not proof.
Upstream project revisions keep their usual meaning.

References answer a named question; they do not select dependencies or authorize
source reuse. Before code, the implementing Bead records exact releases or
commits, useful mechanisms, rejected surrounding scope, license compatibility,
and consequences for ownership, code shape, and the first check.

## Rounded adaptive workspace tabs

`ven-rounded-adaptive-tabs-vdf` inspects Qt qtbase 6.8.3
`c07c2d5a527a644d36e7853d55132ae38921682f`, `src/widgets/widgets/qtabbar.cpp`:
natural font-metric sizing, cumulative tab layout, display-text elision and
selected-rectangle reveal. Qt's widget, style, drag and animation systems are
outside Venus scope. LGPL/GPL/commercial source inspection only; no code copied.
The existing cosmic-text 0.19.0 shaper supplies measured widths and glyph-cluster
byte boundaries with the same attributes used by glyphon 0.12.0 for header
rendering. Scene rectangles retain layout, clipping, hit and AccessKit ownership.
The existing rectangle batch supplies rounded fills; clipped text areas keep
terminal glyphs out of path previews without another renderer or dependency.
Different label widths, valid cluster cuts, overflow/reveal, accessible clipped
bounds and isolated native hover/focus rendering are the falsifiers.

## Native Eon Bar workspace header

`ven-native-eon-bar-header-iqj` inspects Nova Bar at exact Apache-2.0 source
`bfbf8a7be28b69f9d822c0843bf2dd5066dd49e3`. Its runtime template keeps tabs
on the leading side, assigns deterministic left/center/right precedence, and
hides secondary content on overlength. Venus adopts only that tab-first priority
evidence; it reuses no Nova source, Zellij/zjstatus/WASM mechanism, KDL,
configuration, widget tray, polling, styling, notification, or controller.

[GNOME header-bar guidance](https://developer.gnome.org/hig/patterns/containers/header-bars.html)
supports a small set of primary controls, tooltips, narrow-width restraint, and
some blank draggable space.
[Apple toolbar guidance](https://developer.apple.com/design/human-interface-guidelines/toolbars)
supports deliberate item count, logical grouping, and retaining important
trailing actions under width pressure.
[VS Code status-bar guidance](https://code.visualstudio.com/api/ux-guidelines/status-bar)
supports short labels, clear icon metaphors, and restrained secondary content.
[Ghostty's official configuration reference](https://ghostty.org/docs/config/reference)
supplies comparison evidence that integrated titlebar tabs save vertical space
and leave only non-tab titlebar areas draggable. These are UX comparisons, not
dependencies or implementation sources.

Exact patched winit 0.30.13 at
`7c209ec5bebbd2963f75a39f79b320af6b03f72c` owns native movement. Its
`Window::drag_window` contract requires an immediately preceding left press;
the Wayland implementation uses the latest pointer-button serial and requires
the cursor inside the window. Venus therefore exposes one Scene-owned empty
rectangle, calls winit directly from its immediate left press, and consumes a
returned failure without workspace or terminal input. Existing Scene rectangles,
renderer batches, EONW actions, VEN-C20 viewer state, and AccessKit nodes remain
the only owners. A second row, generic toolbar, dependency, protocol change,
inactive-tab close, copied mechanism, and macOS claim are rejected. Ordinary,
overflow, narrow, and tiny layout/hit/action/accessibility checks plus isolated
native Wayland interaction are the falsifiers.

### Bottom shortcut hints

`eon-bottom-hints-bar-vmum` inspects Nova `38a5e29eb078e939d9729dfdbbdf3362292c20b9`
and its pinned zjhints `709d56292217920a5b7e2702302cd66b60ed6477` documentation
and modifier-grouping patch. Compact grouping, visible startup, reclaimed space
and whole-hint priority fitting inform VEN-C24. Nova is Apache-2.0 and zjhints
MIT; inspection only, with no code copied or adapted. Venus uses actual native
binding descriptors and popup facts, existing cosmic-text measurement, Scene
rectangles, rendering and AccessKit. Plugin panes, terminal-cell width guesses,
configuration formats and a speculative minor-mode system are rejected.
Nova `7cec652becb1f2e415b7c23355abb963aa5a5820` layout and runtime notes
also inform compact C/A/S headers, short action labels and one movement hint
for both axes. Explicit creation targets remain because Eon has no mode label.
Nova `85db9eefdbeae48fe29938ed23e28111ea5629d6` modifier formatting also
informs modifier grouping and background roles. Venus uses unbraced badge
headers, such as A-S, measured with their padding through the existing shaper.
Compact grouping and modifier-only background formatting inform the footer;
equal inter-group distribution and badge padding are Eon's product choices.
The footer is the bare margin below the pane cards, with its hints centered in
a band that reaches the window's bottom edge. The existing initial grid overhead includes the
footer's whole reserved height. The row keeps at least the edge padding height
at minimum fonts.
Existing rounded outline/fill
primitives suffice. Nova's #FF5252
modifier color informs Eon's softer #FF6B6B coral beside mint keys and gray labels.
Native fit, focus, resize, input isolation and accessibility are the falsifiers.

### Codex quota fact chip

`ven-render-codex-quota-eon-bar-wfe` and `eon-codex-logo-eon-bar-p9e` consume
Eon's bounded EONW v6 facts at
exact source `3103a00a904c347472899549ce611da765a71650`. Eon remains the sole
owner of Codex process, authentication, account invalidation, refresh, retry,
freshness, permission, and normalization policy. Venus derives only
elapsed/total labels from the normalized duration, reset, and observation facts
and projects the current snapshot through the existing Scene, renderer, hover,
and AccessKit owners.

Nova Bar `bfbf8a7be28b69f9d822c0843bf2dd5066dd49e3` supplies comparison-only
evidence for concise elapsed/total reset-window labels beside remaining
percentage, explicit stale presentation, and secondary content yielding before
tabs. Its source text, separators, provider commands, cache, polling, styled
strings, Zellij integration, configuration, and widget model are rejected. The
existing GNOME, VS Code, and Ghostty guidance above supports the same restrained
one-row priority. Exact shaped width selects full text, then a truthful compact
label, then no chip; no provider worker, timer, action, focus target, dependency,
module, or copied source is added.

For provider identity, OpenAI's current
[brand guidelines](https://openai.com/brand/) permit an official mark when it
directly identifies the related OpenAI service, require exact artwork without
alteration, prohibit added Blossom colors, and prohibit implied endorsement.
Venus uses the exact monochrome OpenAI logomark from OpenAI's own
`openai/openai-cookbook` at commit
`263b2d5b7b63836c4ea30ee9709e65b7a50cbf6f`, file
`examples/agents_sdk/deployment_manager/frontend/src/openai-logomark.svg`;
source SVG SHA-256 is
`a41e8e53b14ef686319ed1529066d6a3391aca77d7f6eaaa9f0783acc717375f`.
The checked-in 104×104 alpha mask has SHA-256
`7bf5bf047a493ef0d76177d74a3de81783b5d1038dd13fcc6cd9b1db20edec91`.
The mark remains OpenAI's property and is subordinate to Eon's identity. The
existing glyphon 0.12 custom-glyph hook scales and caches that fixed mask;
image generation, a decoder, second GPU pipeline, generic icon API, altered
mark, and text-shaped lookalike are rejected.

VEN-C26 consumes typed Claude facts from canonical EONW v8 at Runtime
`2f9630723254ad7d9401ae40e459cef1fbfe8a17`. The existing Codex chip is the
native comparison: reuse measurement, row priority, hover and static AccessKit
projection. Claude uses a plain C monogram, without copied brand artwork.
Provider collection and freshness remain Runtime responsibilities.

### CPU and memory use

VEN-C28 (`eon-cpu-widget-eon-bar-acxs`) compares btop v1.4.7
(`6e39144aaf5a6bc01b9f795010b0914431067183`,
`src/linux/btop_collect.cpp`) and Waybar 0.15.0
(`90b209add8937514d0a987aa842e701bd8f1232e`,
`src/modules/cpu_usage/linux.cpp` and `common.cpp`) against Linux v7.3's
`Documentation/filesystems/proc.rst` and `kernel/sched/cputime.c`. Venus
keeps btop's formula: idle is idle plus iowait, total is the first eight
fields, so steal counts as busy and guest time, which the kernel already adds
to user and nice, is not counted twice as Waybar does. It reads only the
aggregate line, which covers every possible CPU and so needs no hotplug
handling. It rejects btop's clamping of a counter that went backwards into a
false 0% or 100%, Waybar's 32-bit total and unclamped casts, and both
projects' since-boot or 100 ms first value: the first reading is only a
baseline, and a failed or non-advancing reading shows nothing.

Memory use (`eon-igae.3`) compares btop v1.4.7's Linux memory collection and
Waybar 0.15.0's `src/modules/memory/` at the same revisions against Linux
v7.3's `Documentation/filesystems/proc.rst` and `fs/proc/meminfo.c`. Venus
keeps btop's used figure, MemTotal minus MemAvailable, rounded to nearest,
with fields read by name. It rejects both projects' estimates for a kernel
without MemAvailable (every kernel since 3.14 has it), and btop's MemFree
substitution and Waybar's unchecked wrap when MemAvailable exceeds MemTotal:
such readings show nothing. Waybar's ZFS ARC adjustment is out of scope.

### Local clock

VEN-C27 (`eon-igae.4`) compares Waybar's clock module at tag 0.14.0
(`41de8964f1e3278edf07902ad68ca5e01e7abeeb`, `src/modules/clock.cpp`), which
sleeps until the next whole interval of wall-clock time (default 60 seconds)
and formats the moment in the local zone. Venus adopts only the wake at the
next local minute, computed from the wall clock rather than by adding a fixed
delay, as one more deadline in its existing event-loop wake list. Waybar's
thread, locale and format configuration, and calendar tooltip are rejected.
The clock reuses the version label's spare-space rule and its hover and static
AccessKit label, and yields before every other Eon Bar item.

## Rounded pane frames

`ven-pane-stack-chrome-suu` inspects Nova
`e6c29312fc8ca11da41fe9da1809ea0e9a9116c8`: `defaults/zellij/config.kdl`,
`layout.swap.kdl`, and the `yzx-config` catalog/model show full pane frames,
stacked panes and separate frame visibility/rounded-corner settings. Apache-2.0
inspection only; no source, Zellij configuration or live-update mechanism is
reused. Eon uses rounded frames enabled by default, with one startup boolean;
radius remains presentation detail. `eon-gapped-rounded-cards-txco` takes its
visual reference from Superlogical Rex's per-pane cards (macOS screenshot in
Bitdoze's article, https://www.bitdoze.com/superlogical-rex/, checked
2026-10-18; look only, no source).

Each pane is its own rounded card, stacked without gaps in its column's
allocated card area. A collapsed pane shows a rounded title edge whose corners turn toward the
expanded pane; its sides join the next card in that direction. The expanded
pane has a full rounded frame with its title in the top edge.
`border_width` sizes neutral outlines and `focus_border_width` sizes the
selected pane's whole outline. Scene owns viewport/header/body allocation,
padding, whole-cell grid sizing, hit targets and AccessKit bounds.
Scene owns the outer gap around and between allocated column areas and includes
it in initial grid overhead. Scene owns each column's fill from its first
rounded top edge to its rounded bottom. The allocated area stays fixed during
header scrolling.
The last collapsed pane's label sits in that bottom edge, closing the stack with
one line. At the unscrolled bottom, columns and the Agents card share one snapped
painted bottom line. Its inset is half the title height, bounded by terminal
bottom padding minus the widest stroke, so frames stay clear of grid cells.
After resize rounding, the shared line moves outward as needed to leave a
full stroke beyond every visible terminal grid, within the allocated bottom.
A last expanded frame ends there inside its bottom padding when space permits.
If a short, scrolled viewport cannot fit its bottom stroke below the grid, the
stroke stays below the grid and the card clips it. Terminal layout,
grids, header bands, hit targets and AccessKit bounds stay unchanged, as do
popup and frameless paint. Last collapsed labels center on the line, clamped
so their text boxes end at or above the bottom of their unchanged title bands. One
continuous terminal background at terminal opacity covers title rows and the
space between stacked cards; Renderer paints it once. Popup and sidebar fills
follow their own rounded outer edges. Above and outside those fills, the gaps
and footer take the header's opaque color. Hover adds
a label pill; keyboard focus adds a mint pill outline.
Pinned winit `7c209ec5bebbd2963f75a39f79b320af6b03f72c`, Wayland
`window/state.rs::set_blur`, already requests blur over the entire surface; its
`types/bgr_effects.rs` routes compositor support. The regression uses a colored
underlay to expose missing paint.
This proves pixel coverage and alpha, not blur on a blur-capable compositor.
The existing selected palette, header hover and focus paths suffice. Another shader, dependency, layout model, geometry setting
or persistent Venus configuration is rejected. Strict launch admission,
unpainted stroke interiors/corners, clipped bounds and isolated native
default/off interaction are the falsifiers. Eon delivery remains separate.

## Explicit hyperlink interaction

`ven-lq4` follows the
[OSC 8 specification](https://iterm2.com/feature-reporting/Hyperlinks_in_Terminal_Emulators.html)
for cell-authored targets and inspection of the actual URI. The preserved
2020-05-31 document supplies protocol semantics, not current terminal support
claims. Current Ghostty and Kitty documentation supplied UX comparisons only.
The exact selected winit `7c209ec5bebbd2963f75a39f79b320af6b03f72c` cursor and
event APIs own the native pointer boundary. Foot
`85655c74a4ded119392ea8b632626c3920042807` (`url-mode.c`, `doc/foot.ini.5.scd`)
demonstrates explicit keyboard traversal, clipboard activation and a distinct
launcher. Venus rejects its traversal mode, URL discovery, launch templates and
file rewriting; the chosen interaction keeps only pointer opening and hovered
target copying.

`eon-nlcy` later added plain-text HTTP/HTTPS detection at Lucca's request.
Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083` (`src/config/url.zig`),
WezTerm `5046fc225992db6ba2ef8812743fadfdfe4b184a` (`default_hyperlink_rules`)
and foot `85655c74a4ded119392ea8b632626c3920042807` (`url-mode.c`) all join
soft-wrapped rows. Venus keeps a count-based form of foot's bracket trimming,
which drops a closing `)` or `]` the target does not open, and foot's rule
that an OSC 8 link overrides any overlapping detected target. It rejects Ghostty's
other schemes, file paths and Ctrl-held highlighting, and WezTerm's
longest-match rule order, which links `(https://x)` with its closing
parenthesis. A hand-written Scene scanner limited to the opener's URI bytes
replaces a regex dependency, and only targets the opener accepts are
detected.

AccessKit models custom actions, but the locked Linux adapter's exact
[`accesskit_atspi_common` source](https://github.com/AccessKit/accesskit/blob/c88605b96d04431f9c3c792464a0f2f253480e94/platforms/atspi-common/src/node.rs)
maps actionable nodes only to the standard Click action. Venus therefore
publishes one Click-backed Open link and one Click-backed Copy button per
visible target instead of an inert Linux custom action.

[GLib 2.84.4's GIO dispatcher](https://github.com/GNOME/glib/blob/2.84.4/gio/gio-tool-open.c)
preserves a URI argument and completes after dispatch to the registered handler.
Its `gdesktopappinfo.c` launches a separate process, so Venus can retire its own
dispatcher without owning the browser lifetime. The existing host `gio open`
command is the native seam; no crate or packaged dependency is added.
`xdg-open` was rejected because some paths wait for the browser process itself;
opener crates and direct portal bindings would add an unnecessary owner here.
Official Apple `NSWorkspace.open` behavior was inspected for the macOS path.
Hosted M1 run 34820510780 then proved fixed `/usr/bin/open -u URI` under a
minimal environment, with immediate handoff. The Apple candidate uses that
command behind the same validation, timeout, and child-reaping owner; it adds
no shell, browser lifetime, or dependency. Native acceptance remains gated by
`VEN-C16`.

Foot is MIT and GLib LGPL-2.1-or-later. This is mechanism inspection, with no
source copied into Venus. Native GIO, exact clipboard bytes, pointer/keyboard
capture, stale-frame refusal and AT-SPI target/error names are the falsifiers.

## Startup typography and initial geometry

`VEN-C19` uses the selected glyphon 0.12.0 / cosmic-text 0.19.0 `FontSystem`,
`Fallback`, `Attrs` and platform fallback sources, fontdb 0.23.0 family queries,
and unicode-script 0.5.8's `Script`. Explicit primary names preserve ordered
fallback before generic family search. Keep the existing font fitter and
`CellMetrics`; reject another resolver or geometry owner.

Patched winit `7c209ec5bebbd2963f75a39f79b320af6b03f72c` owns native sizing.
Its Wayland event loop consumes `ScaleFactorChanged`'s `InnerSizeWriter`
synchronously; use that one admission to preserve the requested grid when the
initial output scale arrives after the first buffer. Later user size/scale
changes remain authoritative. Workspace Scene owns header and popup overhead.

Rio `e1946a7b98a5a5a4074f384437f0256abf1df75b`,
`sugarloaf/src/font/mod.rs` and `rio-backend/src/config/window.rs`, supplies
comparison evidence for ordered face lookup and independent columns/rows.
Its font cache, terminal state, renderer, config framework and platform layers
are rejected scope. MIT inspection only; no source copied or adapted.

The first falsifiers are actual glyph font-ID ordering, missing named fonts,
and exact terminal grid sizes. Native default/configured rendering and
interaction remain separate evidence from pure shaping and geometry checks.

## Required before the first client implementation

- Orbit `ORB-C1` through `ORB-C6`, ORBF v1, ORBS v1, and the dependency-free
  `orbit-protocol` 0.1.0 package were accepted at archive revision
  `c905bf9610581747f1b07565814b501ca66cfaa6`, when `ORB-C7` was partially
  proved. Venus now consumes the in-tree package and must not mirror either
  codec or its semantic values.
- [FrankenTUI](https://github.com/Dicklesworthstone/frankentui) is required for
  backend boundaries, deterministic rendering transitions, and test
  architecture. Inspect its
  [presenter-emission ADR](https://github.com/Dicklesworthstone/frankentui/blob/main/docs/adr/ADR-002-presenter-emission.md),
  [terminal-backend ADR](https://github.com/Dicklesworthstone/frankentui/blob/main/docs/adr/ADR-003-terminal-backend.md),
  [backend-strategy ADR](https://github.com/Dicklesworthstone/frankentui/blob/main/docs/adr/ADR-008-terminal-backend-strategy.md),
  backend traits, headless backend, golden checks, and property-test structure.
  Venus accepts independently justified backend seams, deterministic patch or
  damage transitions, and test patterns—not its TUI, widget, ANSI-presenter, or
  runtime architecture.
- [OpenTUI Rust](https://github.com/Dicklesworthstone/opentui_rust) is required
  composition-model evidence. Inspect its buffer layering, clipping, alpha,
  draw-buffer composition, hit grid, double buffering, and separation between
  the rendering engine and the host application loop. Venus does not assume a
  cell-grid renderer, terminal output backend, or OpenTUI dependency.
- [Ghostling](https://github.com/ghostty-org/ghostling) and
  [libghostty-rs](https://github.com/Uzaaft/libghostty-rs) remain required
  ownership references for native windows, glyphs, GPU integration, callbacks,
  and render-loop boundaries. They do not authorize a second terminal state in
  Venus.

`ven-upt.1` completed this gate at FrankenTUI
`479436597890a14e82676d0067e3917b2a9de8f5`, OpenTUI Rust
`a37bed1d2569fdd5d3bc33ec19c0678cd0cb1cd8`, Ghostling
`f9034e43a50a2f3a8101e35497f486090c1ddd6e`, and libghostty-rs
`72ac98f292879bf9f788fcbb11238c562a1eebe6`. Implementation work must reread
the complete Bead evidence rather than treating these identities as selected
dependencies.

The Jeffrey Emanuel references carry nonstandard licensing. The user explicitly
directed exact-commit inspection for architecture ideas and independently
directs and benefits from it; tool choice alone does not satisfy a restriction
on acting for a named provider. Inspection does not authorize copying,
adaptation, redistribution, incorporation, or dependency selection, which
remain separate license and user-approval gates.

## Selected crate evidence and comparisons

Automatic Accelerando selection uses Venus's existing cell specialty-font path,
alongside Nerd Font and Braille handling. Anima `395016b6340ca83b59b9921834fc9221b49c7005`
owns the Accelerando Terminal font and its U+100000–U+101FFF mapping. One canonical
cell grapheme receives a family hint only when no primary or ordered fallbacks
were supplied; named overrides, default cell metrics and fitting remain intact.
The selected cosmic-text 0.19.0 shaper retains glyph and style resolution.
Complete glyph/style and native pixel/grid comparisons plus fixed-workload
cadence prove the route; a second cache or resolver is outside Venus ownership.

- [winit 0.30.13](https://docs.rs/winit/0.30.13/winit/) owns the window and
  native-event boundary. Its
  [`EventLoopProxy`](https://docs.rs/winit/0.30.13/winit/event_loop/struct.EventLoopProxy.html)
  carries typed wakeups from transport work into the application loop.
- [wgpu 30.0.0](https://docs.rs/wgpu/30.0.0/wgpu/) owns GPU access. Its
  [`CurrentSurfaceTexture`](https://docs.rs/wgpu/30.0.0/wgpu/enum.CurrentSurfaceTexture.html)
  distinguishes success, occlusion, timeout, outdated configuration, and loss
  for bounded renderer recovery.
- [glyphon 0.12.0](https://docs.rs/glyphon/0.12.0/glyphon/) integrates text
  preparation, clipping, raster caches, and mask or color atlases with wgpu 30.
  Its [cosmic-text 0.19.0](https://docs.rs/cosmic-text/0.19.0/cosmic_text/)
  re-export supplies advanced shaping and font fallback. Corpus and native
  checks must still prove terminal-cell fidelity and atlas failure behavior.
- [COSMIC Terminal](https://github.com/pop-os/cosmic-term/tree/7daf10e3b540f612cbc48973469656ecd1635dfc)
  at `7daf10e3b540f612cbc48973469656ecd1635dfc` is conditional comparison-only
  evidence for terminal shaping, bidirectional or ligature behavior, wide-cell
  projection, decoration geometry, input methods, and renderer backend parity.
  Its pinned [`terminal.rs`](https://github.com/pop-os/cosmic-term/blob/7daf10e3b540f612cbc48973469656ecd1635dfc/src/terminal.rs)
  and [`terminal_box.rs`](https://github.com/pop-os/cosmic-term/blob/7daf10e3b540f612cbc48973469656ecd1635dfc/src/terminal_box.rs)
  demonstrate measured monospace width, bounded shape-run caching, and glyph-run
  decoration projection. Venus rejects its PTY, parser, terminal-grid, workspace,
  selection, clipboard, search, configuration, and unfinished damage ownership.
  COSMIC Terminal is [GPL-3.0-only](https://github.com/pop-os/cosmic-term/blob/7daf10e3b540f612cbc48973469656ecd1635dfc/LICENSE):
  inspection does not authorize copying, adaptation, incorporation, or dependency
  selection, each of which requires its own license and dependency gate.
- [foot](https://codeberg.org/dnkl/foot/src/commit/85655c74a4ded119392ea8b632626c3920042807)
  at `85655c74a4ded119392ea8b632626c3920042807` is MIT-licensed comparison
  evidence for a Wayland-only terminal host and its headless Sway/Cage test
  boundary. Venus rejects foot's PTY, terminal, configuration, renderer, and
  test-harness ownership; the reference does not select another compositor or
  terminal dependency.
- Eon's
  [central terminal-product comparison route](../../docs/REFERENCES.md#terminal-product-comparisons)
  owns Monstar's exact release identity, license, useful mechanisms, and product
  rejections. A Venus Bead follows that route only for a named native-host,
  terminal-interaction, or performance question, then records the exact source
  it inspects. Venus retains Orbit's PTY and terminal authority; this repository
  keeps no duplicate Monstar pin.
- [Sugarloaf](https://github.com/raphamorim/rio/tree/main/sugarloaf) is
  conditional on a demonstrated need for its renderer shape.
- [AccessKit 0.24.1](https://docs.rs/accesskit/0.24.1/accesskit/) and
  [accesskit_winit 0.33.2](https://docs.rs/accesskit_winit/0.33.2/accesskit_winit/)
  own native accessibility adaptation. The
  [`ActivationHandler`](https://docs.rs/accesskit/0.24.1/accesskit/trait.ActivationHandler.html)
  requires a real tree by the next display refresh even when the application
  would skip rendering. Venus derives that tree from the accepted scene;
  pointer hit testing follows the last-presented scene.
  The Linux cache correction is pinned to
  [AccessKit `0f9d186`](https://github.com/Yazelix/accesskit/commit/0f9d186ec6f5e66fe4ccc131ca318dc2725c12f5),
  based on upstream `c88605b96d04431f9c3c792464a0f2f253480e94`.
  [AT-SPI 2.58.1's cache handlers](https://gitlab.gnome.org/GNOME/at-spi2-core/-/blob/1c403959184b36a2d146840254b7a69dd080ec40/atspi/atspi-misc.c)
  require each AddAccessible/RemoveAccessible structure as one signal argument.
  The dependency's encoded-wire check and real Orca return-to-live selection
  oracle prove this boundary; `docs/CRATES.md` owns the pin and exit decision.
- [softbuffer 0.4.8](https://docs.rs/softbuffer/0.4.8/softbuffer/) remains the
  software comparison. Its CPU buffer supports damage on selected platforms,
  while AppKit presentation requires a blocking copy.
- [Vello 0.9.0](https://docs.rs/vello/0.9.0/vello/) with
  [Parley 0.11.0](https://docs.rs/parley/0.11.0/parley/) remains the rich-vector
  comparison. Vello identifies its renderer as alpha and requires GPU compute;
  Parley supplies broader rich-text layout than the accepted grid needs.

Compare complete ownership shapes, not isolated crates. The gate measures text
correctness, input methods, accessibility, native Linux Wayland behavior,
future browser implications, owned LOC, dependency/build cost, and maintenance.

The user selected exact-revision Orbit protocol consumption with winit 0.30.13,
wgpu 30.0.0, glyphon 0.12.0 and its cosmic-text 0.19.0 re-export, AccessKit
0.24.1 with accesskit_winit 0.33.2, and pollster 1.0.1. `ven-upt.2` completed the
reference and crate gates before implementing that shape.

## Required for native history, selection, and copy

`ven-4sn` consumed Orbit `ORB-C8` at proof
`840a67c0cb32b334ed54888321d5ca77e58117b0` and `ORB-C9` plus canonical ORBS v2
at proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`. The exact package defines
revision-bound Begin, Update, Finish, and Copy actions and the bounded
server-only `CopiedText` effect. Venus adopted those messages directly and did
not add a protocol mirror, history cache, terminal selection, or compatibility
adapter.

Exact winit 0.30.13 line and pixel wheel events support cell-normalized bounded
native accumulation. Exact AccessKit 0.24.1 text runs and text positions support
selection derived from the accepted scene. Exact wl-clipboard-rs 0.9.3 supplies
the isolated native Wayland text effect selected in `docs/CRATES.md`.

## Required for exact ORBS v4 consumption

`ven-adopt-orbs-v4-without-interaction-expansion-zdc` consumes canonical ORBS
v4 and proved `ORB-C1` through `ORB-C12` from exact Orbit source
`7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`; accepted proof metadata is at
`4a8da857bafd980c199e6c49d2718ad9cae26dc0`. The exact diff from the prior
`3ee7c80005f3d2bbe81e539799327803716f6174` consumer replaces version-range
Hello/Attached/Incompatible negotiation with exact-header Hello and Attached,
and adds read-only vertical previews plus typed committed-wheel outcomes.

Venus sends no preview request and rejects an unsolicited preview at its model
ordering boundary. Terminal-routed wheel outcomes reuse the accepted-result
owner; viewport outcomes enter the existing complete-frame reducer and bounded
frame queue. An unsupported header version retains explicit incompatibility UX
without decoding another revision. No input, renderer, gesture, retry, schema,
adapter, or compatibility path is added.

## Required for read-only pane metadata

`ven-render-live-pane-title-cwd-8c7` consumes canonical ORBS v5 from exact
Orbit source `69c402737799f03e615473956954a043647a4713` (pre-consolidation
history). Orbit owns bounded title and
working-directory values plus the read-only `ObserveMetadata` role; Venus sends
that one request, rejects every other role or message order, and retains only
the latest accepted update for each visible live endpoint.
Within Eon's local-only product boundary, the projection treats a `file://`
authority as transport decoration and displays only the absolute path. Remote
working-directory semantics and percent decoding remain outside the contract.

Exact EONW v5 protocol source `0cc8f477298681ae3945903e8fdb5852d487c5ab`
is authoritative for popup selection, active-tab identity, pending-tab state,
tab launch directory, pane liveness, endpoint identity, and optional selection.
Venus starts no hidden or offline observer and retires obsolete endpoints
directly from each accepted snapshot, and defers the selected endpoint's
observer until its presentation attachment has completed.
That deferral preserves Orbit's single pending-negotiation slot without adding
retry policy or another lifecycle owner.

[Zellij](https://github.com/zellij-org/zellij/tree/b9637022eaddb22855dc9914a0cc06762a124b8c)
at `b9637022eaddb22855dc9914a0cc06762a124b8c` is MIT-licensed comparison-only
evidence for showing a pane title within the available header width. Venus
adopts that narrow outcome through its existing header clipping and rejects
Zellij's terminal, tab, layout, plugin, configuration, and title-policy owners.
No dependency or copied source is added.

## Required for native paste

`ven-native-clipboard-paste-uas` consumes Orbit `ORB-C5` at accepted proof
`9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`. Orbit accepts one canonical paste
up to 1 MiB and applies normal or bracketed terminal encoding. Venus passes that
message unchanged and keeps no terminal paste policy.

Exact winit 0.30.13 supplies logical Ctrl+Shift+V and `NamedKey::Paste`. Exact
wl-clipboard-rs 0.9.3 returns ordinary Wayland clipboard bytes for a selected
text MIME. Venus bounds and UTF-8-validates them; empty or non-text content stays
visible. Mars `21109e3ebc24b63da11bae644dfb9bab28ce0e18` confirms the same Linux
shortcuts. Venus reuses no Mars binding, clipboard, or terminal code.

`ven-consume-terminal-clipboard-writes-zgh` first consumed Orbit `ORB-C11` and
canonical ORBS v3 at proof `3ee7c80005f3d2bbe81e539799327803716f6174`.
Orbit supplies bounded UTF-8 and the normalized standard, selection, or primary
destination. Venus accepts the effect only while attached and passes it to the
existing native clipboard owner without a local decoder or replay state. The
ORBS v4 gate above preserves that unchanged effect at `7f067b30`.

Exact wl-clipboard-rs 0.9.3 exposes the ordinary and primary Linux clipboards
through Wayland data-control. The pinned Ghostty comparable maps standard to the
ordinary clipboard and maps both selection and primary to the platform primary
clipboard. Venus uses that map and has no secondary or compatibility fallback.

Alacritty 0.15.1 at
`0c405d53e74ace2980fc5e6c6d5b710c144bc075` was comparison-only evidence for
wheel residuals, Shift selection precedence, Ctrl+Shift+C, clipboard isolation,
and backend-specific IME cursor-area comparison. Venus reused no source
and rejected Alacritty's terminal, grid, selection, configuration, auto-copy,
primary-selection, search, and raw-display clipboard ownership.

## Canonical workspace protocol provider

The current canonical package is the in-tree
[`eon-workspace-protocol`](../../runtime/crates/eon-workspace-protocol) crate.
Inspect its complete manifest and v19 source for bounded values, validation
and framing. [Crate decisions](CRATES.md#eon-workspace-protocol-consumer) record
exact package identity and independent codec selection. The historical routes
below retain their original semantic and native proofs.

## Native workspace foundation (historical v4 evidence)

`ven-c87`, `ven-present-tab-directory-picker-a6v`, and
`ven-consume-picker-first-eonw-v4-zd1` consume `EON-C10`, `EON-C17`, `EON-C18`,
and dependency-free EONW v4 from exact Eon source
`c305453bba4fe50c29f65e829b9cd65af31ced8a`; prior workspace proof
documentation is recorded at `93c3c723e5da95c92882571b2904d4c9a4b9f2ff`.
The owner package supplies complete
ordered snapshots, stable tab, pane, and Session identities, authoritative raw
tab launch directories, exact opaque Orbit endpoints, liveness, structured
failures, and semantic actions. Venus consumes those values directly, derives
bounded tab labels and accessibility context, and retains `tN` for action routing.
The `ven-c87.1` refresh hardening
reuses bounded `Inspect` every 250 ms without adding a protocol message, CLI
adapter, hidden-state reader, or second workspace schema.

Canario/Rio at `3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098`
was MIT-licensed comparison-only evidence for bounded header scrolling, direct
labeled-row activation, distinct selected state, and one selected content
surface. Venus reused no source and rejected its terminal ownership, split and
reorder behavior, previews, sidebar, persistence, command palette, and effects.
Nova at `57b0c8621894c59d058d7ba7d91464864b7c4917` is Apache-2.0
comparison-only evidence for shared non-modal `Alt+m` pane creation and
`Alt+h/l` focus-or-tab traversal. The user selected the complete Eon keymap;
Venus reuses no Nova source, Zellij configuration, plugin, or pane ownership.
Exact winit, wgpu, glyphon, and AccessKit versions already selected above own the
native event, clipping, drawing, tab semantics, expanded state, and accessibility
action mechanisms used by the workspace projection.

The v4 picker state contributes only one existing-tab identity and one validated
endpoint; its bound pending tab may have no panes or selected pane. Venus
derives visibility from equality with the active tab and reuses its single
terminal attachment, scene, renderer, input, and AccessKit owners. One explicit
`--workspace` option selects the existing EONW startup path without an initial
Orbit endpoint; one physical Alt+Z maps to the semantic action, and existing
Alt+H/L actions remain Eon-owned traversal. Nova
`f1beb34fe6060cfa2c0201d7f8095f6ef707f467`, Zellij
`910339e219b17f4e2ffd4a2cb6e35cccc7246493`, and Kitty
`54416498c89e1d07e5079c49d15470dd0d947ce7` remain comparison-only for the
accepted shortcut, inset, and terminal-backed lifecycle. Venus reuses no source
and rejects a native picker, inferred modal state, simultaneous terminals, and
a generic popup or modal API.

## Required for shared popup presentation

`ven-shared-popup-surface-ehk` consumes the exact Eon-owned EONW v5 seed at
`0cc8f477298681ae3945903e8fdb5852d487c5ab` (pre-consolidation history).
The question is how Project and tools share one stack-covering surface without
moving product or terminal policy into Venus. The dependency-free owner codec
supplies the catalog, logical margins, per-tab instances/selection and guarded
actions. Existing Venus attachment, Scene, renderer, input and accessibility
owners consume it directly; no second schema or launch mechanism is added.

Comparison-only sources inspected for this slice:

- [Nova](https://github.com/Yazelix/nova/tree/6575ae476a8d4b2fa8b4aa8d0f3839bdba6467e1)
  `defaults/config.toml` and `crates/yzx-config/src/custom_popups.rs`: named popup
  entries and shared geometry; command and cwd policy stay with Eon.
- [Zellij Popup](https://github.com/Yazelix/zellij-popup/tree/f9424823b01e5223dadff9e6c5b87c0131a902f8)
  `src/lib.rs`, `src/main.rs::show_popup` and
  `src/popup_contract.rs::should_restart_popup_for_cwd`: clamp margins for a
  usable surface and consume authoritative visibility. Venus adopts no plugin,
  process lookup, cwd restart or sidebar lifecycle.
- [Kitty](https://github.com/kovidgoyal/kitty/tree/54416498c89e1d07e5079c49d15470dd0d947ce7)
  `kitty/launch.py` and `kitty/tabs.py`: overlays identify an exact target and
  reuse terminal presentation. Per-pane overlays and local PTY launch are
  rejected because Eon selects one tab-body endpoint.

Nova and Zellij Popup are Apache-2.0; Kitty is GPL-3.0. All were inspected without
copying or adapting source. The chosen check combines canonical v5 roundtrips,
focus/geometry/accessibility regressions and an isolated native Wayland consumer
using real Orbit Sessions with a bounded canonical workspace producer. It does
not establish Eon command/lifetime behavior or composed runtime activation.

## Required for pixel and kinetic retained-history scrolling

`ven-venus-kinetic-touchpad-scroll-5sl` consumes Orbit `ORB-C8` and `ORB-C9`
through exact ORBS v7 source
`baf8aa28dcaa50484cd221aa7730defedc2356bb`. Orbit supplies one bounded
revision-bound nearest-first row window and one signed atomic viewport commit;
Venus does not infer routing, edges, cells, or revisions.

Patched winit 0.30.13 at
`7c209ec5bebbd2963f75a39f79b320af6b03f72c` maps Wayland discrete axes to
`LineDelta`, continuous axes to physical `PixelDelta`, and available axis-stop
to `Ended`. It discards axis source and hardware event time and documents that
discrete wheel sequences may lack `Ended`. Venus therefore gives momentum only
to phase-complete pixel sequences and adds no device or source heuristic.
`Window::request_redraw` with `pre_present_notify` owns compositor callback
pacing. Wgpu 30.0.0 retains guaranteed FIFO and its default desired frame
latency of two until installed measurements justify a change. Glyphon 0.12 and
the existing rectangle batch accept fractional positions and clipping.

GTK main `04275027fc556b1b0f3935e2f502fc356e88227b` is comparison-only
evidence for a 150 ms recent-sample velocity estimate, frame-clock advancement,
and elapsed-time exponential friction of 4 s^-1. Kitty
`7f71461a9de44f71f631db29d50bcb3e866988b8` is comparison-only evidence for
sub-row terminal presentation and precision momentum. Venus reuses no source
and rejects overshoot, bounce, velocity stacking, fixed-rate timers,
frame-dependent decay, platform source code, and public tuning.

## Required for native selection gestures

`ven-venus-native-selection-gestures-d3x` consumes Orbit `ORB-C5` and `ORB-C9`
through exact accepted ORBS v10 source
`59975e9176f5caf8b78dc3273e88d9ecbb75dc3f`. Orbit chooses terminal mouse
input or host selection from authoritative terminal state, pins that route for
the sequence, applies cell, word, and logical-line gestures, freezes copy text,
tags release copy separately from explicit ordinary copy, and reports the
authoritative completion revision. Venus supplies native surface position,
modifiers, event-delivery time, ordered buffering, and clipboard effects.

The accepted reference gate in Bead `ven-venus-native-selection-gestures-d3x`
compares WezTerm, Kitty, Alacritty, Zellij, and Ghostty and adopts their common
uncaptured selection plus Shift bypass behavior. Venus adds no click recognizer,
terminal-state inference, dependency, or unreleased winit API.

`ven-adopt-orbit-click-selection-ladder-23e` consumes accepted Orbit
`6bc269c40b18f08b95778939518f77556ba91c67` through unchanged ORBF v2 / ORBS v13
and package tree `d1151ce365f371cd06498c8d65fbcf6831ee0afe`. Exact patched winit
`7c209ec5bebbd2963f75a39f79b320af6b03f72c` delivers native button phases;
Venus forwards position, modifiers and monotonic delivery time through its
existing release-presentation gate. Orbit owns words at two clicks, punctuation
spans at three and logical lines at four or later, including drag/upward-tick
granularity and frozen copy. Reuse the existing native clipboard effects and
private Wayland/Vulkan proof route; no client classifier, boundary parser,
source copying or dependency capability is added. Retain Orbit's documented
wide-spacer span limitation rather than introducing a second terminal owner.

## Required for continuous-output scrollback acceptance

`ven-accept-continuous-output-scrollback-a1s` consumes Orbit `ORB-C8` through
exact accepted source `a65e199e16e97330175e314cacf791fa00f53069` under unchanged
ORBS v10. Orbit resolves lagging relative preview and signed-scroll requests
against current terminal authority while future revisions and unrelated stale
input remain strict. Venus retains one pending preview, one in-flight batch,
fractional and kinetic input, canonical frame reduction, and truthful failures.
While screen, geometry, default cell colors, and palette remain compatible, it
keeps the last bounded revision-bound row window as presentation input, submits
crossed rows against that exact lagging revision, accepts only the matching
in-flight outcome, and never carries terminal-routing evidence across a plain
frame. It adds no retry, failure-string policy, history cache, or revision
owner.

## Required for live-output selection acceptance

`ven-select-during-live-output-752` consumes Orbit `ORB-C5/C8/C9` through
accepted source `91999d79546422b49bdbc124166a65859d0bd872` under unchanged
ORBS v10. Orbit admits a nonfuture presented Begin revision, owns the native
tracked selection and history anchors, and preserves a gesture through
compatible output. The protocol tree remains
`10b417879f4c7ca0caa48f684448010264ab9086`.

Venus's presentation owner separates content dirtiness from input geometry:
ordinary frames with unchanged dimensions/screen advance the renderer cache
without withdrawing the last actually presented revision. Structural changes
still invalidate publication. Finish retains its authoritative presentation
gate. Patched winit `7c209ec5bebbd2963f75a39f79b320af6b03f72c`
`Window::request_redraw` gives no strong ordering guarantee, so input eligibility
cannot depend on the next output frame being repainted first.

The reference gate compares Ghostty
`a887df42c56f6de86c0fe6da9c4eeca37931e083` tracked viewport pins,
Alacritty `d692748d3f61253ebe9f5094320120d22f6a046f` grid offsets,
kitty `d124d4f0976720a127c022e967d30f05df99399e` history offsets,
tmux `578e07fcbc66dc60822b55b88ba12f518df57374` copy view with continuing
parsing, and Zellij `af38660c5884f50bb3726682fb92961326c4268f` deferred PTY
bytes while scrolled. Native Orbit/Ghostty anchoring retains one terminal owner;
Venus adds no frozen history view, raw-output queue, parser pause, or dependency.
These are inspected mechanisms, with no source copied or adapted. The native
Application regression proves input admission between frames and repaint;
actual Orbit dogfood separately checks anchoring and copied contents.

## Required for supervisor-owned native presentation

`eon-cyt` consumes Eon's existing `EON-C11` Present action and exact child-process
ownership without changing EONW. Exact winit 0.30.13 owns
`Window::set_minimized`, `Window::focus_window`, and
`Window::request_user_attention`. Locked winit documents direct unminimize and
focus as unsupported on Wayland; its attention path requests and applies
`xdg_activation_v1` for the existing surface under compositor policy. Venus
accepts only the bounded private supervisor signal and remains the sole native
activation owner.

The selected shape rejects a duplicate window, Venus replacement, platform token
in EONW, compositor-specific commands, D-Bus application infrastructure, polling
files, and a signal-handler dependency. The compositor retains final activation
policy, so native acceptance is platform-specific.

## Required for native cursor-tail materialization

The current cursor path consumes canonical ORBF v2 / ORBS v13 cursor values and
`orbit-protocol` 0.1.0 at accepted Orbit source
`6bc269c40b18f08b95778939518f77556ba91c67`. Orbit remains authoritative for
destination, shape, visibility, blink state, wide-tail state, and explicit cursor
color. When no cursor color is present, Venus selects the cursor-body color from
its chosen trail color; the static `none` profile retains Orbit's foreground
fallback.

[Rio](https://github.com/raphamorim/rio/tree/e019a9325b59a025cffa03a21d0788168514d502)
at `e019a9325b59a025cffa03a21d0788168514d502` is MIT-licensed required evidence
for its complete trail cursor, spring helper, renderer glue, route reset, and
redraw scheduling. Venus adopts the independently useful four-corner critically
damped spring, 40 ms short-horizontal and 150 ms longer timing classes, 100 ms
delta cap, first-frame snap, direction-ranked lag, two-triangle quad
geometry, and redraw only while unsettled. It reuses no Rio source and rejects
Rio's
terminal state, Sugarloaf, renderer framework, configuration, panels, and
polling.

Window-local continuity supersedes Rio's route snap for healthy workspace
navigation: Venus preserves bounded cosmetic spring state across the attachment
gap, hides its draw inputs and deadlines, and retargets only after the selected
attachment supplies a fresh frame. Route and workspace viewport identities do
not reset motion; terminal authority and failure resets remain separate.

[WCAG 2.2 relative luminance](https://www.w3.org/TR/WCAG22/#dfn-relative-luminance)
defines the sRGB transfer and weighted luminance calculation. The W3C
[non-text contrast explanation](https://www.w3.org/WAI/WCAG22/understanding/non-text-contrast.html)
treats 3:1 as an unrounded threshold. Venus uses those calculations only to
derive the nearest integer tint or shade along the higher-contrast black/white
direction for a tail outline and the fallback-color cursor body's shape-aware
edge; this is not a web-conformance claim.

The existing launch parser, standard-library cosmetic entropy, wgpu rectangle
pipeline, dynamic buffer upload, and scale-aware metrics are sufficient. The
conditional wgpu post-processing comparison is rejected: no shader ABI, new
dependency, renderer replacement, palette registry, general effects engine, or
persistent Venus setting is introduced.

## Required for compositor-owned background blur

The current exact winit 0.30.13 patch is `Yazelix/winit-0.30` commit
`7c209ec5bebbd2963f75a39f79b320af6b03f72c`. It retains ancestor
`fb45fbf901fbe70cc9a877b5d651d0b60c206b08`, directly above the v0.30.13 tag
and an exact backport of verified merged upstream commit
`c4afadbfabf7b1e7989b40b493db1a4c7bd8ff4e`. That ancestor prefers
`ext-background-effect-v1`, retains the KDE fallback, and leaves the public
`WindowAttributes::with_blur` contract unchanged. The fork and upstream remain
Apache-2.0.

## Required for caller-owned application identity

Exact patched winit 0.30.13 commit
`7c209ec5bebbd2963f75a39f79b320af6b03f72c` documents and implements
`WindowAttributesExtWayland::with_name`: its general name becomes the Wayland
application ID and should match the distributed desktop-file ID; its instance
name is unused on Wayland. VEN-C17 reuses only this existing native window
attribute seam before creation. It rejects desktop-file loading, title-based
grouping, product-specific branches, mutable identity, and another window or
event-loop owner.

Wayland Protocols 1.46 at peeled commit
`6141e1154303dadd5c3e480bc4a16e26f1dcb2af` is the first corrected protocol
reference and is byte-identical to the XML bundled by locked
`wayland-protocols` 0.32.13. Release 1.45 is rejected because it encodes the
blur capability as zero. The protocol makes the region surface-local and
double-buffered while leaving the algorithm and policy to the compositor.

Installed COSMIC compositor 1.0.0 at
`091583ac84abac02967ae358cf9570ddfef63b31` advertises the corrected blur
capability, commits the requested region with surface state, and owns the
rendering algorithm. Its source is GPL-3.0 evidence only; Venus copies and
incorporates none of it. Venus adds one launch boolean at its existing window
creation boundary. It rejects direct Wayland ownership, capability caching,
blur strength, renderer effects, live updates, Eon persistence, and another
window or event-loop owner.

## Required for numeric scrollback position

`ven-scrollback-position-ddo` consumes Orbit
`ea9fd28ce0908f218cf65d4e6df368f0a4e565f5`: ORBF v2 / ORBS v11 carries
`ScrollPosition` with wrapped display-row distance and retained history size.
Its Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083` projection owns
reflow, pruning and alternate-screen semantics. Venus formats the accepted
reducer frame directly; wheel deltas and preview pixels cannot update the count.

[foot](https://codeberg.org/dnkl/foot/src/commit/8c2687196647a7593987547ae40586ffc238189d)
provides the required numeric-versus-percentage and fixed-versus-moving placement
comparison in `render_scrollback_position` and `foot.ini`. Venus chooses fixed
placement and explicit distance above live output; percentages would change as
history grows or prunes. MIT source was inspected without reuse.
[Ghostty 1.3.0](https://ghostty.org/docs/install/release-notes/1-3-0) is comparison
only for a small overlay preserving the grid; dragging and a scrollbar are out
of scope. Existing glyphon 0.12.0 shaping/clipping and AccessKit 0.24.1 terminal
descriptions suffice; there is no new widget, history cache, live alert or crate.
FrankenTUI `479436597890a14e82676d0067e3917b2a9de8f5` ADR-002 supplies
required complete-state projection evidence. Its MIT rider was evaluated for
independent user-directed inspection; no source was copied, adapted or executed.

`ven-scrollback-return-to-live-j8u` consumes Orbit `ORB-C8` at exact producer
proof `f8ad14e5195109ba8cb421f30e5ae4a9619a1419`: ORBS v12 adds a
zero-payload `ReturnToLive` action and Orbit owns the authoritative jump and
terminal-routing rejection. Venus reuses its existing painted pill bounds for
pointer and AccessKit Click routing. The foot and Ghostty references above
remain placement comparisons, not sources for another scrollbar or widget.

## Apple Silicon native-window foundation

`ven-bring-up-opaque-appkit-metal-g4a` inspected the exact locked
[winit 0.30.13 fork](https://github.com/chiyuki0325/winit-0.30/tree/fb45fbf901fbe70cc9a877b5d651d0b60c206b08),
AccessKit 0.24.1 / accesskit_winit 0.33.2, wgpu 30.0.0, glyphon 0.12.0,
and raw-window-handle 0.6.2 sources. Winit's AppKit backend maps the existing
hidden/show API to `orderOut` and `makeKeyAndOrderFront`;
[AccessKit](https://docs.rs/accesskit_winit/0.33.2/accesskit_winit/struct.Adapter.html#method.with_mixed_handlers)
requires its adapter before the first show; and wgpu creates its Metal surface
from the AppKit `NSView` through a `CAMetalLayer`.
[Apple's `NSWindow`](https://developer.apple.com/documentation/appkit/nswindow/makekeyandorderfront%28_%3A%29)
and [`CAMetalLayer`](https://developer.apple.com/documentation/quartzcore/cametallayer/drawablesize)
documentation confirms the visibility and backing-pixel behavior. The existing
`ApplicationHandler`, `WindowEvent`, and `Renderer` owners therefore suffice for
native lifecycle, scale, resize, and surface recovery. No question remained
that justified importing Rio or Ghostty architecture, raw AppKit code, a second
renderer, or a platform framework.

## Apple Silicon native interaction candidate

`ven-deliver-macos-input-pasteboard-links-zfw` inspected exact winit fork
`7c209ec5bebbd2963f75a39f79b320af6b03f72c` and its AppKit event path. It maps
Command, Option, and Control independently to
`SUPER`, `ALT`, and `CTRL`; clears preedit before committed IME text; updates
pointer position before button and wheel delivery; preserves precise pixel
deltas and native touch or momentum phases. Venus therefore keeps its shared
input, IME, pointer, scroll, focus, and repeat owners. Only the existing Copy,
Paste, and Open-link gestures select macOS Command; custom Alt and Control
bindings keep their actions and display Apple key names. Apple's keyboard
guidance supplies the standard Command-C/Command-V convention and the warning
against repurposing platform shortcuts.

Physical M1 tracing found that Kotoeri reached marked text `にほん`, then winit
treated a selected-input-source identity change as immediate IME deactivation
before AppKit interpreted the commit key. Upstream PR 4087 removed the same
transition but also routed ordinary input through IME commit and was rejected.
Guard-only fork candidate `92722876415d5f2e3e97b2897b84c9a768c79b82`
added `!hasMarkedText()` to that deactivation guard. An initial one-Enter
diagnostic stopped after candidate selection and incorrectly suggested the
separate `unmarkText`/`insertText` change in upstream PR 4650 was also needed.
The corrected Kotoeri sequence uses one Enter to select and a second to commit;
an A/B run then proved the guard-only candidate delivered exact `日本`. The
published fork tip reverts the temporary PR 4650 addition and has the same tree
`4e4784e6bbfee70d901000fa4c47470ff6ef52d2` as the guard-only commit. AppKit
remains the marked-text owner, non-composition source changes keep their prior
behavior, and Venus gains no platform-specific input branch.

On the physical Macmini9,1 M1, exact final Venus candidate `f0cf58f` with fork
`7c209ec5` preserved Kotoeri conversion from `にほん` to the selected `日本`
candidate. Kotoeri used one Enter to accept the candidate and a second to
commit; Orbit then received exactly one UTF-8 `日本` (`e697a5e69cac`) and no
duplicate.

Hosted M1 runs 34820414136 and 34820510780 reject `pbcopy`/`pbpaste`: `pbpaste`
emitted raw RTF-only bytes as preferred text, while `pbcopy` advertised literal
RTF-looking terminal text as rich and plain types. Target-only arboard 3.6.1,
checksum `0348a1c054491f4bfe6ab86a7b6ab1e44e45d899005de92f58b3df180b36ddaf`,
instead requests and writes only `NSPasteboardTypeString` through the general
pasteboard. Its native unsafe code remains inside the dependency. Venus maps
standard, selection, and primary destinations to that one macOS pasteboard and
keeps its existing empty, UTF-8, and 1 MiB delivery checks. The dependency
materializes a complete platform string before that bound; the user accepted
this practical allocation tradeoff. Direct AppKit, runtime Swift/JXA, rich
clipboard ownership, primary-selection emulation, and a second keymap remain
rejected.

Hosted M1 run 34832511106 exercised exact Venus source
`0f0a9ab945f4fbc492b79468a6b4d877a19c90ee` against Orbit runtime
`0233f4d34b294a50c5bc7f373859cf4ec04d2414` on macOS 15.7.9. Seventy-three
selected tests, the Apple all-target check, and the native build passed. The
native window delivered an OSC 52 Unicode value to the general pasteboard,
copied one hovered URI with Command+C, and sent exact bytes
`Aé界rrpaste-é界\r` from key text plus Command+V. Presentation-control EOF
closed Venus with status zero while Orbit stayed alive.

The hosted runner could not accept click/drag selection, focus loss, repeat,
IME candidate behavior, or product-path Command+click Open. Process-targeted
mouse button events did not reach winit, and a synthetic focus-away event still
allowed input. M1 run 34831678536 retained distinct PNG-only and oversized paste
failure notices and sent no PTY bytes; its empty-paste capture retained stale
link feedback. Run 34832511106 exposed a proof-harness defect: Bash did not fail
on negated `cmp`, and its failure screenshots retained that stale feedback. The
corrected workflow failed before runner allocation in run 34833225890 because
the account reached its payment or spending limit. That hosted evidence left
native empty-paste and host-access failure notices open. VEN-C16's native
proof in this repository observes the empty or non-text paste notice; the
host-access failure notice remains open.

## Watchlist

- [MetalTerm](https://metalterm.dev/) is source-unavailable comparison evidence
  to revisit only for a named Venus question about idle redraw, GPU measurement,
  OSC 133 command blocks, or grapheme and cell storage. Its Metal renderer and
  native macOS packaging are outside Venus's platform scope. Its
  [site repository and issues](https://github.com/pioner92/metalterm-site)
  and the creator's [X feed](https://x.com/pioner_dev) are discovery surfaces,
  not implementation authority. No application source or license was public
  when checked on 2026-08-21; pin a stable artifact and reproduce performance
  claims before using them at a reference gate.

## Rejected initial routing

- Rio VT is not a Venus dependency; replacing Orbit's terminal engine is an
  Orbit decision after a concrete engine failure.
- librio matters only for a separately authorized non-Rust consumer.
- Ratatui may support a lossy diagnostic client but cannot prove the native
  Venus or rich-presentation contracts.
- Qwertty targets controlling-terminal applications and does not provide the
  native client foundation.

## Running Eon identity

`eon-eon-version-bar-widget-5s62` inspects Nova
`38a5e29eb078e939d9729dfdbbdf3362292c20b9` packaging/bar-render-request.nix
and Nova Bar `7295e9976803de8d912f766f1b00ad12652e3e9b`
src/bin/nova_bar_widget.rs:74–147. Apache-2.0 inspection only; no code reused.
The useful constraint is identity from the running product, rather than a newly
installed binary. Runtime supplies its immutable product version through the
existing supervised launch seam; Venus does not poll, parse CLI output, inspect
store paths or copy the JSON/widget/channel machinery. The routed GNOME and
VS Code guidance supports a quiet short label using spare header space; Apple
JS-only guidance is comparison-only and no claim depends on it. One native
static label, existing text measurement, rectangles, hit testing, hover and
AccessKit suffice. EONW, dependencies and terminal geometry stay unchanged.

The Nova badge screenshot and exact Nova Bar template/format at `7295e997`
also inform uppercase monospace identity. Venus uses regular terminal typography
and a softer #7CC4D8 cyan, with calibrated cell spacing for bounded ASCII identity.
Nova's bold #00CCFF, channel formatting and widget machinery remain comparison
evidence only. Existing width/AccessKit and emitted-glyph checks plus native
captures verify the projection without another font or style owner.
