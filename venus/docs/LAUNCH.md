# Running Venus

## Run a standalone client

Pass the Eon Sessions Orbit Unix socket to the Venus client:

```sh
cargo run --locked -- /path/to/orbit.sock
```

Linux requires native Wayland and fails before attaching when it is unavailable.
[Platform and proof limits](PRESENTATION.md#platform-and-limits) cover macOS and
other hosts.

### Typography and initial size

Startup options apply to standalone and workspace surfaces:

```sh
cargo run --locked -- \
  --font-family 'DejaVu Sans Mono' \
  --font-fallback 'Symbols Nerd Font Mono' \
  --font-size 20 --line-height 1.5 \
  --columns 100 --rows 30 /path/to/orbit.sock
```

The primary family must be installed and monospace. Repeat `--font-fallback`
for up to eight installed families in preference order; platform fallback
follows them. Names are nonempty trimmed UTF-8, at most 128 bytes without
controls. Missing named fonts fail launch. Font availability does not promise
all glyphs or style variants; Venus does not install fonts.

`--font-size` accepts 6–96 nominal logical pixels, and `--line-height` accepts
1–3 times that size. Absence preserves 16 px nominal text and 10 by 18 logical
cells. One rounded physical cell grid drives text, cursor, pointer, selection,
scrolling, IME and accessibility.

Positive `--columns` and `--rows` request a terminal grid, including space for
workspace headers or popup margins. An omitted dimension retains its existing
initial window dimension; the default window is 960 by 600 logical pixels.
Requests must fit Orbit's 100,000-cell and native surface limits. The compositor
may override initial sizing, and later user resizing remains unconstrained.
Typography must leave room for a terminal cell in the initial window, including
workspace headers, even when columns and rows are omitted.
These are startup options; Eon owns persistent product configuration.

## Attachment and recovery

Without an argument, Venus uses
`$XDG_RUNTIME_DIR/yazelix-orbit/orbit.sock`, or
`/tmp/yazelix-orbit-{effective-uid}/orbit.sock` otherwise.
Only one presentation client can attach to an Orbit session at a time.
Venus may start before Orbit. A missing, refused, reset, or dropped local socket
retries after 250 ms, 500 ms, 1 s, 2 s, 4 s, and then every 5 s. Venus retains
the last coherent scene during recovery and replaces it only with a fresh
complete frame. Busy, incompatible, exited, invalid protocol or model, resource,
queue, input, and worker-start failures remain terminal and visible.
Routine workspace attachment and first-frame progress are silent; standalone
attachment retains progress messages.

### Workspace mode

For an Eon workspace, select workspace mode with only the EONW socket:

```sh
cargo run --locked -- --workspace /path/to/eon.sock
```

The supervising Eon may pass `--eon-version VERSION` to identify its running
product. Absence hides the native label; the input is workspace-only, at most
128 ASCII letters/digits or `.`, `-`, `+`, and starts with a digit. It is an
immutable launch fact, not a lookup of the installed executable or Venus version.
Eon owns the `[bar] show_version` preference. Narrow layouts hide the full label
when there is insufficient spare header space.

Pass `--pane-frames false` to hide decorative pane borders. The default is
`true`; both modes keep the same terminal grid, pane headers, click targets and
accessible bounds. Keyboard focus and hover remain visible with frames off.
Missing, duplicate or invalid boolean values fail before window creation.
Standalone and popup presentation have no pane frames; popups retain their own
rounded outline. This is a startup option; Eon owns persistent product configuration.

Workspace mode opens the EONW transport first and waits for its accepted
snapshot before attaching its terminal endpoint. The first snapshot may select
a popup or have no visible terminal; no initial Orbit endpoint, durable pane,
or placeholder Session is required.

### Window identity and decoration

Pass `--application-id ID` before the launch target when the caller owns a
distinct desktop identity. The bounded ASCII token becomes the Wayland app ID
before window creation and does not replace Orbit-authored window titles.
Direct Venus launches use `eon`:

```sh
cargo run --locked -- --application-id eonova /path/to/orbit.sock
```

Pass `--no-decorations` to request a window without its native title bar. The
default remains decorated:

```sh
cargo run --locked -- --no-decorations --workspace /path/to/eon.sock
```

### Background and cursor

Pass `--background-opacity VALUE` with a finite value from `0.0` through `1.0`
to control the terminal default background. Venus uses `1.0` when the option is
absent. This example uses the Nova-selected opacity:

```sh
cargo run --locked -- --background-opacity 0.88 /path/to/orbit.sock
```

The opacity follows Orbit-authored default background changes and covers empty
terminal padding and exposed popup margins. Explicit cell backgrounds, selection, inverse video,
workspace chrome, notices, focus borders, text, and cursors retain their
existing presentation. Visual transparency does not change pointer or keyboard
input. Venus rejects invalid values during launch and rejects translucent
launches before presenting its first frame when the native surface lacks
premultiplied alpha support. Winit does not expose Wayland window visibility
control. Eon owns persistence and product defaults for composed launches.

Pass `--background-blur` to ask the native compositor for full-surface blur at
window creation, before the first frame. Blur and opacity are independent: an
opaque default background hides the effect, while lower opacity reveals it. This
COSMIC Wayland example leaves terminal-default pixels fully transparent:

```sh
cargo run --locked -- \
  --background-opacity 0.0 \
  --background-blur \
  /path/to/orbit.sock
```

COSMIC owns the blur algorithm and strength. Unsupported or policy-disabled
Wayland compositors may ignore the best-effort request without failing launch.
Venus does not configure blur strength.

Venus chooses one random built-in cursor-tail color per launch by default and
keeps it for that process, with an internal fallback multiplier of `1.0`.
Eon owns product defaults and configuration in its `config.toml`; the parameters
below are the component launch interface. Choose `random`,
a named preset, or a custom color:

```sh
cargo run --locked -- --cursor-trail-color preset:ice /path/to/orbit.sock
cargo run --locked -- --cursor-trail-color 'custom:#12ABCF' /path/to/orbit.sock
```

The presets are `magma` (`#FF3B30`), `solar` (`#FFD23F`), `lime` (`#B7F34A`),
`forest` (`#35C978`), `ice` (`#7DDCFF`), `ocean` (`#5271FF`), `nebula`
(`#A970FF`), and `bubblegum` (`#FF5DA2`). Every tail gets an automatic
one-logical-pixel contrasting outline. The fill and outline animate and clip
together. The cursor body uses the chosen color and a shape-aware contrasting
edge unless Orbit supplies an explicit cursor color; Orbit still owns its shape,
visibility, and blink state.

Pass `--cursor-trail-duration-v1 VALUE` to override the duration with any color
choice, or omit the color choice to keep a random color. Values must be finite
multipliers from `0.25` through `4.0`:

```sh
cargo run --locked -- --cursor-trail-color preset:ice \
  --cursor-trail-duration-v1 2.0 /path/to/orbit.sock
```

Pass `--cursor-effect-v1 none` for a static cursor. The lower-level complete
tail profile remains available for an exact color and a finite duration
multiplier from `0.25` through `4.0`:

```sh
cargo run --locked -- \
  --cursor-effect-v1 tail \
  --cursor-trail-color-v1 '#89b4fa' \
  --cursor-trail-duration-v1 1.0 \
  /path/to/orbit.sock
```

Venus rejects unknown presets, malformed custom colors, duplicate choices,
conflicting color/effect options, incomplete profiles, non-finite
durations, and out-of-range durations before opening a window.

### Supervised startup

In Eon's supervised mode, Venus reads one private bounded presentation stream.
The `stdin-ready-v1` mode consumes one canonical EONW v8 startup snapshot for
workspace launches, then reports `ready-v1` on stdout after admitting fonts and
the actual window's initial native geometry, before terminal attachment. Eon
bounds that exchange and starts new commands only after readiness.
Each complete Present command keeps the same Venus process and terminal
attachment and asks the native window system to present its existing window.
If that stream closes or fails, Venus exits without stopping Orbit and releases
its presentation attachment; an immediate supervised replacement retries a
transient Busy while the departing client releases it. Standalone Busy remains
terminal. Direct focus and unminimize are unavailable through winit on Wayland;
xdg activation remains compositor-controlled.
