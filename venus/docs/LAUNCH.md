# Running Venus

## Run a standalone client

Pass the Orbit Unix socket to the Venus client:

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
input, and worker-start failures remain terminal and visible.
Routine workspace attachment and first-frame progress are silent; standalone
attachment retains progress messages.

### Workspace mode

For an Eon workspace, select workspace mode with only the EONW socket:

```sh
cargo run --locked -- --workspace /path/to/eon.sock
```

The supervising Eon may pass `--eon-version VERSION` with
`--eon-release-stage alpha|beta|stable` to identify its running product. A stage
requires a version; both inputs are workspace-only and given at most once.
Unknown stages fail launch. Older supervisors may supply only a version,
which shows without a stage glyph so they can reload Venus. No version hides
the native label. The version is at most 128 ASCII letters/digits or `.`, `-`,
`+`, and starts with a digit. It is an immutable launch fact, as is the stage:
Venus never infers a stage from version
digits or suffixes, or looks up the installed executable or Venus version.
Eon owns the `[bar] widgets` preference. Narrow layouts hide the full label
when there is insufficient spare space beside the bottom shortcut hints.

The supervising Eon may pass `--eon-icon ABSOLUTE_PATH`, the Eon icon that
leads the Eon Bar. The input is workspace-only and given at most once; a
relative or empty path fails launch. Venus reads the file once at startup: a
square image, four bytes per pixel row by row (red, green and blue
premultiplied by the fourth, alpha), 16 to 512 pixels a side. A file Venus
cannot read or that is not such an image shows no icon and one line on stderr,
and launch goes on. Venus area-averages the image to two logical pixels less
than the tab pills' height, in whole pixels, and draws it in its own colours.
The icon keeps its slot while tabs overflow and scroll beside it. Quota chips
and the session label yield before the icon. Only the existing tiny-width
minimum, where the icon, one tab's minimum and trailing controls cannot fit,
hides it. Changes to quota and reading values do not hide it.

The supervising Eon may pass repeatable `--bar-item ITEM` pairs to choose
which Eon Bar items show and in what order
([Eon Bar](PRESENTATION.md#eon-bar)). Each pair carries one whole name;
commas and colons are never list separators. Fixed names are
`codex_quota_5h`, `codex_quota_7d`, `claude_quota_5h`, `claude_quota_7d`,
`claude_quota_scoped`, `cpu`, `ram`, `disk`, `gpu`, `clock` and `version`.
Google selectors are `google_quota:<scope>:5h` and `google_quota:<scope>:7d`:
separate the final suffix at the last colon. Scope is exact, nonempty,
control-free and at most 32 UTF-8 bytes. For example,
`--bar-item 'google_quota:A,B:C:7d'` carries scope `A,B:C` unchanged.
Each Google selector shows its matching active model-group window as a Google
text chip; an unreported window shows nothing.
There is no bare provider alias.

With neither input, Venus uses the ten fixed items other than opt-in `gpu`
in the order above.
`--bar-empty` is a valueless marker for an explicitly empty list. It cannot
repeat or appear beside any `--bar-item`, in either order. Both inputs are
workspace-only. A missing, empty, non-UTF-8, unknown or repeated item fails
before window creation. The old `--bar-items` flag is not accepted.
Duration names select unscoped windows only; `claude_quota_scoped` selects
all scoped Claude windows. An item without data still shows nothing.

With each of the `cpu`, `ram` and `disk` items Eon may pass
`--<chip>-notice-percent N`, `--<chip>-warning-percent N` and
`--<chip>-critical-percent N`: the displayed use percentages at which the chip
turns yellow, orange and red. Each accepts 1 through 100, with notice not
above warning and warning not above critical; equal levels skip the lower
colour. With all three omitted, defaults are 60, 75 and 90. Omitted levels
adjust around explicit ones: an omitted warning is raised to an explicit
notice or lowered to an explicit critical, an omitted notice is the lower of
60 and the resolved warning, and an omitted critical is the higher of 90 and
the resolved warning. Explicit inversions still fail. With `cpu`, Eon may
also pass `--cpu-average-seconds N` (2 through 600, default 30), the window of
the average in hover and accessibility ([Eon Bar](PRESENTATION.md#eon-bar)),
and `--cpu-temperature-warning-celsius N` and
`--cpu-temperature-critical-celsius N`. Temperature levels are whole degrees
from 30 through 120, critical not below warning, defaulting to 85 and 95.
They colour the CPU chip only with a supported Linux CPU sensor; the stronger
utilization or temperature colour wins. These inputs are workspace-only;
missing, out-of-range, explicitly inverted or repeated values fail before window
creation. Eon owns the `[bar]` settings.

With `gpu` Eon passes `--gpu-interval-seconds N`, a whole number from 5 through
3600 (omission defaults to 30). It is workspace-only and cannot repeat. Venus
uses the larger of that interval and a readable kernel autosuspend delay plus
one second, coordinated across windows. NVIDIA sampling is Linux-only; other
platforms have no GPU reading. This flag adds no headless mode or EONW field;
`eon-agent-parity-a9g0.12` owns publishing the displayed sample to agents.

With `disk` Eon may also pass `--disk-path PATH`, the path whose filesystem it
measures. The path must be absolute, since Eon expands `~` from HOME, and may
be any bytes. Absence keeps HOME; an unset or relative HOME leaves no reading.
The input is workspace-only, and a missing, relative or repeated value fails
before window creation. A path that does not exist is not a launch failure:
the chip shows no reading.

Pass `--pane-frames false` to hide decorative pane borders. The default is
`true`; both modes keep the same terminal grid, pane headers, click targets and
accessible bounds. Keyboard focus and hover remain visible with frames off.
Missing, duplicate or invalid boolean values fail before window creation.
Standalone and popup presentation have no pane frames; popups retain their own
rounded card outline. This is a startup option; Eon owns persistent product configuration.

Pass `--focus-color preset:NAME|custom:#RRGGBB` to colour the selected pane's
whole outline in the selected column and the working and background attention
badges' dots. Every attention glyph adapts its colour only as needed for 3:1
against its surface; bar badges retain contrast against both their pill and
the surrounding bar. A translucent row's glyph adapts against the card at the
configured background opacity over both black and white. Only where no single
colour can reach 3:1 against both does the source glyph take a thin black or
white outline, whichever contrasts more with the glyph. Badges draw no backing
shape at any opacity. Stale glyphs and outlines keep their intentional 0.45
alpha after colour adaptation and may read below 3:1.
The default is `custom:#b207fb`; presets are the cursor colours listed below,
and `random` is invalid. Missing, duplicate or invalid values fail before window
creation. F6 keyboard-focus pills stay mint. With pane frames off, no pane
outline is drawn. Eon passes its `[terminal] focus_color` when configured.

Pass `--border-width N` and `--focus-border-width N` to size neutral card
outlines/title rules and the selected pane's outline, respectively. Values are
integers from 1 through 3 logical pixels; defaults are 1 and 1. Each width
becomes `max(1, round(width * scale))` physical pixels (defaults at scale 1.3:
1 and 1; at scale 2: 2 and 2). Neutral cards include panes, the agent sidebar
and popups. Pane frames off hides pane/sidebar outlines and title rules;
popups keep their outline. The current agent-row and F6 pill outlines remain one
physical pixel. Missing, duplicate, non-integer or out-of-range values fail
before window creation. Eon supplies its corresponding `[terminal]` settings
only for workspace launches when configured.

Pass `--panel-heading notch|connected` for panel heading geometry (VEN-C18).
The default is `connected`, which joins the notches at their top height with
the card's usual rounded corners; title and action positions do not change.
Missing, duplicate or invalid values fail before window creation. Eon Runtime
resolves `[panels] heading` and supplies it only on workspace launches, including
reopens and reloads. A running Venus retains its launch value and reads no
configuration.

Pass `--warning-close-last-tab false` to send a final-tab close at once instead
of first showing the close prompt (VEN-C8). The default is `true`. Missing,
duplicate or invalid boolean values fail before window creation. Eon passes its
`[session] warning_close_last_tab` setting on every workspace launch.

Pass `--animate-working-badge false` to keep the Working badge at the still braille
frame ⠋ and Background's three round dots steadily lit (VEN-C21). With the default
`true`, Working steps through ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ at 10 Hz and Background pulses more slowly on
the same tick. A software (CPU) graphics adapter keeps both still either way. Missing,
duplicate or invalid boolean values fail before window creation. Eon passes its
`[session] animate_working_badge` setting on every workspace launch.

Pass `--number-keys false` to leave Alt+Shift+1 through Alt+Shift+9 and
Alt+Shift+0 to the terminal instead of showing session positions 1 to 10
(VEN-C8); the Shortcuts viewer then leaves them out. The default is `true`.
Missing, duplicate or invalid boolean values fail before window creation. Eon
passes its `[session] number_keys` setting on every workspace launch.

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

By default, or with `random`, Venus chooses one built-in cursor-tail color per
live endpoint: the device, inode and change time of the workspace or Orbit
socket pick it, so relaunching against the same live socket keeps the color.
A socket that does not exist yet at launch gets one random choice for that
process. The internal fallback multiplier is `1.0`.
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
choice, or omit the color choice to keep a random color. Zero disables motion
while retaining the cursor's chosen color and contrasting edge. Otherwise,
values must be finite multipliers from `0.25` through `4.0`:

```sh
cargo run --locked -- --cursor-trail-color preset:ice \
  --cursor-trail-duration-v1 2.0 /path/to/orbit.sock
```

Pass `--cursor-effect-v1 none` for a static cursor. The lower-level complete
tail profile remains available for an exact color and a duration of zero or a
finite multiplier from `0.25` through `4.0`:

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

### Links

Pass `--detect-local-hosts false` to stop detecting plain-text loopback hosts
with a port, such as `localhost:3000`, as links; URLs with a scheme are still
detected ([Hyperlinks](INTERACTION.md#hyperlinks)). The default is `true` in
both modes. Missing, duplicate or invalid boolean values fail before window
creation. Eon passes its `[terminal] detect_local_hosts`.

### Supervised startup

In Eon's supervised mode, `EON_VENUS_PRESENTATION_CONTROL` names one of two
private modes, and stdin and stdout share one private bounded presentation
stream. The `stdin-ready-focus-v1` mode consumes one current canonical EONW
startup snapshot for workspace launches, then reports `ready-v1` on stdout
after admitting fonts and the actual window's initial native geometry, before
terminal attachment. Eon bounds that exchange and starts new commands only
after readiness. The `stdin-focus-v1` mode has no startup exchange. Any other
value leaves Venus unsupervised. In both modes stdout is reserved for this
stream: anything else Venus writes there would corrupt it.

In a supervised workspace, Venus then reports its native window focus on
stdout (VEN-C14), from a worker thread that keeps only the latest focus not
yet sent and never writes the same focus twice in a row. A failed write
closes only that direction and logs `Cannot report Venus window focus:
ERROR`. eonbase and standalone Venus report no focus.

| Phase | Bytes | Meaning |
| --- | --- | --- |
| Admission only, first | `ready-v1` | Startup readiness |
| After admission, or at once without it | `focus:0\n` | Natively unfocused |
| Same | `focus:1\n` | Natively focused |

Each complete Present command keeps the same Venus process and terminal
attachment and asks the native window system to present its existing window.
If that stream closes or fails, Venus exits without stopping Orbit and releases
its presentation attachment; an immediate supervised replacement retries a
transient Busy while the departing client releases it. Standalone Busy remains
terminal. Direct focus and unminimize are unavailable through winit on Wayland;
xdg activation remains compositor-controlled.
