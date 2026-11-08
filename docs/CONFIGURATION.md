# Configuration

Settings live in `config.toml`; `eon config path` prints its full path, normally
`~/.config/eon/config.toml`. `eon config dir` prints the configuration root
that holds it, which also holds the managed tools' configuration directories.
This example chooses a font, cursor preset and 100x30 grid; it is not the
empty-configuration default:

```toml
[bar]
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "claude_quota_scoped", "cpu", "ram", "disk", "clock", "version"]

[terminal]
background_opacity = 0.80
background_blur = true
padding_color = "extend"
pane_frames = true
focus_color = "custom:#b207fb"
border_width = 1
focus_border_width = 1
cursor_trail_color = "preset:ice"
cursor_trail_duration = 1.5
font_family = "Iosevka"
font_size = 16
line_height = 1.125
columns = 100
rows = 30

[shell]
command = ["eon-nu"]
starship = true
zoxide = true
atuin = true
carapace = true
mise = true

[anima]
enabled = true
style = "random"
duration_seconds = 3

[session]
warning_close_last_tab = true
animate_working_badge = true
number_keys = true

[sidebar]
enabled = true

[agents]
terminal_access = "window"
send_enter_delay_ms = 300
send_typing_quiet_ms = 2000
```

## Built-in defaults

`eon config defaults` prints this build's defaults as commented TOML;
`eon config defaults --json` prints the same configurable values as a nested
object. Neither reads your configuration or creates directories, so both work
even when `config.toml` is broken. Unset settings remain comments in TOML and
are omitted in JSON. Eon-owned popup settings are omitted as editable keys.
The following block is checked against the command by the test suite.

<!-- defaults:start -->
```toml
# Built-in defaults. Unset and Eon-owned settings remain omitted.

[bar]
cpu_average_seconds = 30 # Seconds of CPU use that the hover's average covers.
cpu_critical_percent = 90 # CPU use from which the CPU chip turns red.
cpu_notice_percent = 60 # CPU use from which the CPU chip turns yellow.
cpu_temperature_critical_celsius = 95 # CPU temperature in Celsius from which the CPU chip turns red (Linux CPU sensors only).
cpu_temperature_warning_celsius = 85 # CPU temperature in Celsius from which the CPU chip turns orange (Linux CPU sensors only).
cpu_warning_percent = 75 # CPU use from which the CPU chip turns orange.
disk_critical_percent = 90 # Disk use from which the disk chip turns red.
disk_notice_percent = 60 # Disk use from which the disk chip turns yellow.
disk_path = "~" # Path whose filesystem the disk chip measures.
disk_warning_percent = 75 # Disk use from which the disk chip turns orange.
ram_critical_percent = 90 # RAM use from which the RAM chip turns red.
ram_notice_percent = 60 # RAM use from which the RAM chip turns yellow.
ram_warning_percent = 75 # RAM use from which the RAM chip turns orange.
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "claude_quota_scoped", "cpu", "ram", "disk", "clock", "version"] # Eon Bar widgets: header widgets in display order; the clock sits at the bottom bar's left end and the version at its right end, wherever they are listed. Each quota widget selects a window; claude_quota_scoped selects all model-scoped Claude windows.

[terminal]
background_blur = true # Request compositor background blur.
background_opacity = 0.8 # Opacity of the terminal's default background.
# border_width: unset. Use Venus's neutral outline width when omitted.
# columns: unset. Let the window choose initial columns when omitted.
# cursor_trail_color: unset. Omission retains the session's random cursor colour.
cursor_trail_duration = 1.5 # Cursor tail duration multiplier; 0 turns the trail off.
# focus_border_width: unset. Use Venus's focus outline width when omitted.
# focus_color: unset. Use Venus's focus colour when omitted.
font_fallbacks = ["Symbols Nerd Font Mono", "Accelerando Terminal"] # Ordered fallback font families.
# font_family: unset. Let Venus choose the font family when omitted.
# font_size: unset. Let Venus choose the font size when omitted.
# line_height: unset. Let Venus choose the line height when omitted.
padding_color = "extend" # How terminal padding takes its background colour.
pane_frames = true # Draw rounded pane frames.
# rows: unset. Let the window choose initial rows when omitted.

[shell]
atuin = true # Enable the managed Atuin history integration.
carapace = true # Enable the managed Carapace completion integration.
command = ["eon-nu"] # Direct argv for terminals without an explicit command.
mise = true # Activate host mise when available and not already active.
starship = true # Enable the managed Starship prompt integration.
zoxide = true # Enable the managed Zoxide integration and record tab-directory picks.

[anima]
duration_seconds = 3 # Startup animation duration in seconds.
enabled = true # Enable this feature or popup.
style = "random" # Startup animation style.

[session]
animate_working_badge = true # Animate the working badge; false keeps it still.
number_keys = true # Alt+Shift+1..9 and Alt+Shift+0 show sessions 1..10; false leaves the keys to the terminal.
warning_close_last_tab = true # Ask before stopping the final tab's terminals.

[sidebar]
enabled = true # Enable this feature or popup.

[agents]
send_enter_delay_ms = 300 # Wait between `eon pane send` text and its --enter.
send_typing_quiet_ms = 2000 # Refuse unforced sends within this many ms of typing; 0 disables (0 to 60000).
terminal_access = "window" # Let `eon pane read`, `history` and `send` reach this window's terminals; "none" refuses all three.

[editor]
command = ["hx"] # Direct argv that `eon edit` and Files run with the file appended.

# [updates]
# nix_profile: unset. Absolute profile path; set with nix_entry.
# nix_entry: unset. Exact profile entry name; set with nix_profile.

[popups.agent]
command = "auto" # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+L" # Popup shortcut using physical keys.
label = "Agent" # Popup display name.

[popups.anima]
# command and keep_alive are Eon-owned.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+A" # Popup shortcut using physical keys.
label = "Anima" # Popup display name.

[popups.commands]
command = ["eon-commands"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = false # Keep this popup running when hidden.
keybinding = "Alt+Shift+M" # Popup shortcut using physical keys.
label = "Commands" # Popup display name.

[popups.files]
# Files is required: it is the tab's directory pair.
keybinding = "Alt+Shift+Y" # Popup shortcut using physical keys.
label = "Files" # Popup display name.

[popups.git]
command = ["eon-lazygit"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+J" # Popup shortcut using physical keys.
label = "Git" # Popup display name.

[popups.project]
# Project is required and shows the tab's Files on quick search.
keybinding = "Alt+Z" # Popup shortcut using physical keys.
label = "Quick search" # Popup display name.

[popups.scrollback]
# command and keep_alive are Eon-owned.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+E" # Popup shortcut using physical keys.
label = "Scrollback" # Popup display name.

[popups.sessions]
command = ["eon-sessions"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = false # Keep this popup running when hidden.
keybinding = "Alt+Shift+S" # Popup shortcut using physical keys.
label = "Sessions" # Popup display name.
```
<!-- defaults:end -->

## Configuration behavior

Each build ignores keys it does not know, with one warning per key and read
naming its dotted path and the configuration file. This lets an older running
window keep using the settings it knows after a newer build adds a key. Known
keys with wrong types or values still fail, including unsupported widget names.

A key that a later release deprecated still loads, read as its replacement,
with one warning that names the replacement; Eon never rewrites
`config.toml`. When the replacement is set too, it wins and the deprecated key
is ignored with a warning. `[bar] show_version`, deprecated in v0.1.2, reads as
`[bar] widgets`: `false` gives the built-in widget list without `version`, and
`true` gives the built-in list. Any other value is a configuration error.

A table that a later release renamed has no effect: Eon ignores it, whatever
it holds, with one warning that names the new table, and never fails to start
over it. v0.1.20 renamed `[workspace]` to `[session]` and
`[popups.workspaces]` to `[popups.sessions]`; move their keys by hand.

`[bar] widgets` chooses which Eon Bar widgets show and their order, left to
right between the tabs and the Shortcuts button, except `clock` and `version`,
which sit at the left and right ends of the bottom shortcut hints while they
show, wherever they are in the list: the Codex and Claude quota chips, CPU, RAM
and disk use, the local clock and `EON <version>`. The Eon logo at the bar's
left end is not a widget and shows whatever the list holds. Each name
appears at most once. A widget left out doesn't show, `widgets = []` leaves
only the logo, the tabs and the buttons, and omitting the key uses the built-in
list above. The list applies when a window next opens or reopens. Unknown names
are ignored with a warning; repeated known names are a configuration error. Narrow windows hide the Eon
logo first, then disk, RAM and CPU, whatever the order, then quota chips one by
one, the one closest to the tabs first; in the shortcut hints the version hides
first and then the clock, once the keycaps no longer fit beside them. The logo
also hides whenever the tabs would not all fit beside it, however wide the
window, and it leaves room for a chip for each selected window each provider's latest
read reported and for every reading, all at their widest, so their changing
values and a window's reset never show or hide it; it can change once when a
provider's quota first appears or goes away (as it does when all its windows
reset before a read), or when a read finds the provider has gained or lost a
window. The version belongs to its
running supervisor; updating the installation does not change it.

The CPU, RAM and disk chips use the same three grades by default: yellow
from 60% use, orange from 75% and red from 90%, otherwise neutral. For each
chip, `<chip>_notice_percent`, `<chip>_warning_percent` and
`<chip>_critical_percent` set those levels (`cpu`, `ram` or `disk`). CPU
compares the 2-second use it shows, so bursts show. On Linux with a supported
CPU sensor, it also turns orange from `cpu_temperature_warning_celsius`
(85 °C) and red from `cpu_temperature_critical_celsius` (95 °C); the stronger
colour wins. Both temperature levels accept whole degrees from 30 through
120, with critical not below warning; equal levels skip orange. macOS has no
CPU temperature reading. Hover and accessibility say in words why a chip is
coloured, and also give CPU's average over the last `cpu_average_seconds`.
Use levels compare the whole percentage the chip shows; temperature levels
compare the whole degrees in its hover. Percentages accept whole numbers from
1 through 100, with notice not above warning and warning not above critical
for each chip; equal levels skip the lower colour. Omitted percentage levels
start at their defaults and adjust only as needed around explicit levels: an
omitted warning is raised to an explicit notice or lowered to an explicit
critical when needed, an omitted notice is the lower of its default and the
resolved warning, and an omitted critical is the higher of its default and the
resolved warning. Explicitly inverted levels fail. For example,
`cpu_warning_percent = 95` alone resolves to 60/95/95; `ram_warning_percent = 50`
with `ram_critical_percent = 85` resolves to 50/50/85. `cpu_average_seconds`
accepts 2 through 600. The defaults are listed above. Like `widgets`, they
apply when a window next opens or reopens, and an out-of-range value is a
configuration error.

CPU hover also names the CPU, gives its temperature when available, the
busiest logical CPU over the same 2 seconds (numbered from zero), and the
1-minute load average to one decimal. Linux temperature comes only from
coretemp’s Package id 0 or k10temp’s Tctl (Tdie when Tctl is unavailable),
never from a board or ACPI zone. Missing details are omitted.
See [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar).

The disk chip shows how full the filesystem holding `disk_path` is, read every
30 seconds: used space over used plus available space, rounded up as GNU df
rounds it. `disk_path` is an absolute path, or `~` or a path starting with
`~/` for your home directory and paths under it; empty, relative and `~user`
paths are configuration errors. A path that doesn't exist is not an error: the
chip shows no reading. The chip uses the same notice, warning and critical
grades described above. The 30-second
refresh is fixed, since disk use changes slowly. Like the other `[bar]` keys,
they apply when a window next opens or reopens.

Omitting `cursor_trail_color` chooses a random color that lasts as long as the
running window, so `eon window attach` reopens with the same color; a new
window picks again. eonbase keeps one color across reopens, though its
first window may differ. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically. The cursor body
uses the chosen color with a shape-aware contrasting edge unless Orbit supplies
an explicit cursor color.

`focus_color` colours the selected pane's whole outline in the selected column
when pane frames are on, and the working and background dots of agent
attention badges on tabs and in the agent sidebar. Where the colour would
read below 3:1 contrast on what lies under the dots, such as a dark custom
colour on the bar or a white one on a white terminal background, the dots
blend it toward black or white just enough to reach 3:1. Under a translucent
terminal background (`background_opacity` below 1), untinted sidebar rows
depend on the desktop behind the window, so their dots sit on the bar's dark
disc, as on a tab. The 3:1 holds for a current agent's badge; a stale agent's
badge is dimmed on purpose and reads below it. The other badges keep their
own colours. It accepts
`preset:<name>` from the cursor presets above or `custom:#RRGGBB`; `random` is
invalid. The setting is omitted by default, leaving Venus's built-in focus
colour in use. The example selects `custom:#b207fb`. F6 keyboard focus in the chrome keeps
its mint pill outline. This setting applies to full-Eon surfaces;
eonbase has no pane outline.

Popup rows from the shown session in the Agents sidebar and attention inbox
also take a tint of `focus_color`. Their fill stays opaque, including under
hover or keyboard focus. The resting tint stays distinct from the dark chrome
reference and gets lighter on hover, including with light focus colours. Popup
labels brighten as needed to keep at least 4.5:1 contrast with their fill.
The inline popup mark and the current-row and keyboard-focus outlines remain.
Tint intensity is fixed; the existing
`focus_color` controls its hue.

`border_width` sizes neutral pane-card, agent-sidebar-card and popup outlines,
including their title edges, and the Eon Bar widget outlines. `focus_border_width` sizes the selected pane's
whole outline. Both accept integers from 1 through 3 logical pixels; defaults
are omitted, leaving Venus's built-in widths in use. With the example's widths,
physical widths are `max(1, round(width * scale))`:
at scale 1.3 they draw at 1 and 1 physical pixels, and at scale 2 at 2
and 2. Pane frames off hides pane and sidebar outlines and title rules;
popups keep their outline. These settings apply to full-Eon surfaces;
eonbase receives neither override. The current agent row and F6 focus pills
keep their one-physical-pixel outlines.

`cursor_trail_duration` defaults to `1.5`. Set it to `0` (or `0.0`) to turn
the trail off while keeping the cursor's color and contrasting edge. To enable
the trail, use a finite multiplier from `0.25` through `4.0`; larger values
keep it visible longer. It applies when an Eon or eonbase window next opens or
reopens. Eon owns this setting for both products; no Venus or Orbit
configuration is needed.

With pane frames on, each column has one continuous terminal-background fill,
from its first pane's rounded top edge down to the column's rounded bottom.
Stacked title rows and the space between cards share the expanded terminal's
background colour and opacity, except that extended top padding fills an
unzoomed expanded pane's band below its title (`padding_color`). Above that edge
and outside the outer corners, the gaps and footer stay the opaque bar colour.
The first title sits on that boundary. The Agents fill follows its rounded
outer edge, with its title on the boundary. Header scrolling keeps the fill's
top fixed. Frameless mode keeps terminal-background strips across the window
below the bar, including the sidebar and footer.

A popup's outline rises from its left border over the name in a rounded tab,
then curves smoothly back into the top edge. The whole name sits on the popup
terminal's background colour and opacity. The tab keeps its stroke clear of the
text and the bar; when that clearance cannot fit, the name keeps its original
gap in the top frame line. This also applies with pane frames
off; stacked pane headers keep their boundary titles. Long popup names fit with
an ellipsis, and the title is omitted when the window cannot fit it. The full
name remains available to accessibility. Terminal layout and input bounds stay
unchanged.
The popup tab shape is fixed and follows the configured font size and line
height (defaults: 16 px and 1.125), existing title padding, and `border_width`
(default: 1). Its fill uses `background_opacity` (default: 0.8).

`padding_color` controls terminal padding in panes, popups and eonbase.
`extend` uses the nearest edge cell's background on each side and
at the corners. On the primary screen, top or bottom padding keeps the default
background if that edge row contains any default-background cell (including an
explicit color matching the default) or a full-height powerline glyph.
Alternate-screen TUIs always extend. `extend-always` skips those exceptions;
`background` keeps the terminal default background throughout the padding.
Default backgrounds retain the configured opacity; explicit backgrounds paint
as they do in cells. Padding stays inside the rounded card and below outlines.
In an expanded pane with frames on and no zoom, the band below the title line
follows the top padding's colour and opacity instead of the shared title-row
background, under the title text and outline; collapsed title rows, popups,
zoom and frameless mode keep their existing backgrounds. The setting applies
when a window opens or reopens.

`background_opacity` sets how opaque each terminal's default background is, in
Eon and eonbase alike. It also covers the terminal padding, the title rows and
gaps that share that background (above), and the agent sidebar card. Popup-row
tints stay opaque. Explicit cell backgrounds, selection, text and the cursor
keep their own colours, and the Eon Bar stays the opaque bar colour. With pane
frames on, so do the gaps and
footer outside the cards; with them off, the frameless terminal-background
strips below the bar, sidebar and footer included, take this opacity.
See-through areas still take clicks. `background_blur` asks the compositor to
blur what shows through. It is independent of opacity, so a fully opaque
background hides it, and a compositor without blur ignores it. See [Window
background and
decorations](../venus/docs/PRESENTATION.md#window-background-and-decorations).

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. The built-in order is listed above, so alien
glyphs avoid searching all installed fonts.
An explicit list replaces that order. The terminal package includes fonts for
ordinary text, Nerd Font icons, Accelerando and, on Linux, Noto Color Emoji; it
does not install them system-wide. Compositors may ignore blur or initial window
sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each terminal. Invalid startup configuration creates no fresh terminal;
a failed replacement leaves existing terminals and commands alive.

On a fresh full-Eon window, Anima plays after the desktop is ready and before
the first tab's quick search. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback;
`style` is passed to the pinned Anima executable, and `duration_seconds` accepts
1–30. Anima owns its style names; run `eon anima --help` to see them. A failed
animation continues to quick search. Attachment, recovery, later tabs, and
eonbase do not play the startup animation.

Closing the last tab quits its window and stops its terminals. While the
window holds other sessions, it closes only that session.
`warning_close_last_tab = true` makes `eon tab close` and
`eon pane close` ask first when run from a terminal, listing the terminals that
will stop and, while other sessions remain, naming the session that
closes; `--json` never asks. In the window, Alt+Shift+W, the bar's `×` and
Alt+W on the last pane show a prompt listing the same terminals: Enter quits,
or closes the session while others remain, Escape cancels, and Alt+Shift+D
keeps them running instead. With `false`, both close an idle final tab at
once. The CLI's busy confirmation for `eon tab close` and `eon pane close` does
not depend on this setting: when a terminal the close stops is busy or its busy
state is unknown, it still asks, naming the final consequence, or with `--json`
requires `--yes` ([CLI commands](USAGE.md#cli-commands)). It stays fixed
because stopping busy work needs explicit consent. The window reads the setting
when it starts.

`animate_working_badge = true` steps the working attention badge, on tabs and
in the agent sidebar, through the braille dots ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ten times a
second. `false` keeps it still at ⠋, for people who prefer no motion. Without
GPU rendering (a software graphics adapter) the badge is always still. The
window reads the setting when it starts.

`number_keys = true` makes Alt+Shift+1 through Alt+Shift+9 show the window's
first through ninth session, in `eon session ls` order, and Alt+Shift+0 the
tenth. `false` leaves those keys to the program in the terminal, for example
Emacs's M-% on Alt+Shift+5. A popup configured on one of them takes only that
key either way; with `true`, Alt+/ lists that key as overridden. The window reads the setting when it starts.

`[sidebar] enabled = true` offers the agent sidebar, which lists
the coding agents in the window's tabs that report their status. `false` turns
it off; Alt+G and Alt+Shift+G still jump between agents. The window reads the
setting when it starts. Alt+Shift+H shows or hides the sidebar; it also steps
aside while the window leaves fewer than 80 cells for the terminals beside it.
See [Agent sidebar](USAGE.md#agent-sidebar).

`[agents] terminal_access` decides whether `eon pane read`, `eon pane history`
(with `--edit` too) and `eon pane send` may reach this window's terminals.

- `"window"`, the default, lets any process running as you read or send to any
  pane or popup of the window, in any session, shown or hidden.
- `"none"` refuses all three with `terminal-access-off` and reveals no
  terminal content. It also refuses a terminal's request to open Scrollback;
  the window's Scrollback key keeps working.

The window reads the setting when it starts. It is a preference and a kill
switch, not a security boundary: Eon's socket is private to your user, whose
processes can already read and drive terminals.

`[agents] send_enter_delay_ms`, 300 by default, is how long
`eon pane send --enter` waits after the text before sending Enter, so agent
prompts take the whole paste first. It must be 0 to 5000.

`[agents] send_typing_quiet_ms`, 2000 by default, refuses each unforced CLI
paste or Enter while attached-client typing is queued or within this many
milliseconds of typing. It must be 0 to 60000; 0 disables the check.
`eon pane send --force` bypasses only this check. CLI sends never extend the
interval. Like the other agent settings, the window reads it when it starts.

Pane labels use a fixed policy: a pin, then sanitized terminal title, then compact
working directory, with differing-directory context. Commands and `eon pane rename`
set or clear a pin; pins are session state rather than configuration. The pane
ID dims only when the resolved fill and configured opacity permit readable contrast.

## Agent quota

Eon bundles pinned CodexBar CLI for Codex and Claude quota.
Codex uses your authenticated `codex` executable on `PATH`; Claude uses its local
OAuth credentials. Explicit sources avoid browser fallback and model prompts.
Eon does not change provider settings or manage accounts. CodexBar owns credential
selection and its local authentication/cache mechanisms.

Codex and Claude show one chip per available window, five-hour then seven-day.
Claude's model-scoped weekly windows, such as `Fable`, follow as their own
chips. Choose chips with `codex_quota_5h`, `codex_quota_7d`,
`claude_quota_5h`, `claude_quota_7d` or `claude_quota_scoped` in `[bar] widgets`.
The duration names select unscoped windows only; `claude_quota_scoped` selects
all model-scoped Claude windows, in provider order. All five widgets are listed
by default. Codex's 5h widget also shows its blocked or unknown observation
when no windows are available.
Unscoped Codex windows of other durations remain in session JSON but have no bar widget.
For example, to leave out Fable
and other scoped Claude chips:

```toml
[bar]
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "cpu", "ram", "disk", "clock", "version"]
```

Selection affects the bar, its hover and accessibility, and the logo's room
reservation only. `eon session --json` still reports every window read for
each enabled provider, including hidden scoped windows.
Only reported windows appear; an idle Claude window can lack a reset time.
Hover and accessible labels report that absence. Narrow windows hide quota
chips one by one, the one closest to the tabs first, before tabs and controls.

Eon reads a provider's quota only while `[bar] widgets` lists at least one of
its quota widgets, checked
when a window opens or reopens. A chip listed again shows its provider's
stored reading at once, if there is one, and otherwise appears after its
provider's next read.

Codex refreshes at least one minute apart and Claude at least five minutes
apart, also after a failed read or a chip shown again. Eon windows share one
Claude read every five minutes when the shared schedule is usable; otherwise
each keeps its own schedule. Older Eon versions still read on their own.
After the provider refuses a Claude read (for example a rate limit), the next
one waits 30 minutes, then an hour after a second refusal and two hours after
each later one, until a read succeeds. A failed read keeps the last good
values; they become `old` once they are more than an hour old and disappear
after 24 hours. Each supplied reset expires its own window. Missing facts do
not mean zero usage. Sign-out is not detected; the last reading ages like any
other and disappears after 24 hours.

Each provider's last good reading is also stored in `$XDG_STATE_HOME/eon/quota`
(`~/.local/state/eon/quota` by default), shared by all your Eon windows, so a
newly started window shows it until its own read succeeds. The stored reading
holds only windows, remaining percentages, resets and when they were read.

Claude needs no status-line setup.

Existing Eon windows retain their running build after installation updates;
start a fresh window with the updated command to use this backend.

## Popups

Popup entries use the same file. Popup margins are fixed at 8 logical pixels
at the sides and 4 above and below; a `[popup]` table is ignored with a warning.

```toml
[popups.monitor]
command = ["btop"]
keybinding = "Alt+Shift+O"
label = "Monitor"
keep_alive = true
```

Built-in popup commands, labels and shortcuts are listed in the defaults above.
Every built-in popup accepts `keybinding` and `label`. Git, Agent, Commands and
Sessions also accept `command`, `enabled` and `keep_alive`, and
Agent's `command` may also be `"auto"`. Anima and Scrollback also accept
`enabled`; they reject `command` and `keep_alive = true`. Project has no
command of its own:
it shows the tab's Files on quick search, and Alt+Z hides it again when quick
search is already shown. It rejects `command`, `enabled = false` and
`keep_alive = true`, since it is required. A
custom popup needs `command` and `keybinding`; its `label` defaults to its ID
and its `keep_alive` to `true`.
Agent's `auto` command runs the first of `codex resume`, `grok`, `opencode`,
`pi` and `claude --resume` whose executable is on the terminal PATH.
Files opens Yazi with Eon's own popup keys on Alt+Shift+Y and reads no Yazi
configuration of yours. It is the tab's directory pair, so it is required: Eon
ignores `command`, `enabled = false` and `keep_alive = false` there, with a
configuration warning. For a Yazi that reads the `yazi` directory under Eon's
configuration root, add a custom popup such as `[popups.yazi]` with
`command = ["eon-yazi"]` and a free `keybinding`.
Anima opens a random animation and stops when dismissed or replaced.
Scrollback opens what the tab shows, its shown popup or else its selected
pane, in `[editor] command` (see [Editor](#editor)): the terminal's retained
history followed by its screen, as plain text in a private snapshot file that
Eon removes when the popup ends. Its key again dismisses it, which stops the
editor. The snapshot is limited to 8 MiB, the oldest lines dropped first, and
its first line says what was left out; a full-screen app's terminal shows only
its screen, and the first line says so. The bound, the plain-text layout, the
file's private 0600 mode in a 0700 directory, and its removal at the popup's
end are fixed for safety and by Orbit's history contract (ORB-C17).
`eon pane history TARGET --edit` opens Scrollback on TARGET from the CLI (see
[Reading a terminal](USAGE.md#reading-a-terminal)); with Scrollback disabled
it exits 3 with `unknown-popup`.
Commands searches and runs Eon commands, including **Check for updates** and
**Install update** (search `update`). These run `eon update --check` and
`eon update --install`, retaining results until Enter and using `[updates]`
target settings and the CLI's own target/source confirmation. With Commands
enabled, the Window menu offers **Updates** to open this popup. Its description
uses Commands' configured `label`. These fixed action names identify the
documented CLI operations; they add no shortcut or automatic check.
Sessions lists this window's sessions most recently shown first, the shown
one last, marking one that needs input: Enter shows the highlighted one, and
Enter on a name that matches none creates a session with that name.
Set `enabled = false` on Git, Agent, Anima, Scrollback, Commands, Sessions,
or a custom popup to disable it.

A `keybinding` you configure, on a custom popup or a built-in one, takes its
key first. A built-in popup's default key applies only while no configured
popup holds it; otherwise the built-in popup stays without a key, and Alt+/
lists it under its default key as overridden by the popup that holds it.
`keybinding = ""` leaves any popup, Project included, without a key. Until the
CLI path `eon-agent-parity-a9g0.1` lands, a popup without a key cannot be
opened from the window, except that a new tab still opens on quick search and
the Eon Bar's session label still opens Sessions.
Two configured popups on one key fail at load, except on a key reserved for
moving focus, where both load without a key. A popup shortcut on the key of a
built-in Eon shortcut takes that key, and Alt+/ lists the built-in one as
overridden.
Alt+/ and Alt+0 through Alt+9 are reserved and cannot be bound. Alt+H, Alt+J,
Alt+K and Alt+L are reserved for moving focus: a popup configured on one of
them still loads but gets no key, and Eon logs a configuration warning naming
the popup and the key.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

## Editor

`eon edit FILE`, and Enter on a text file in Files, open the file in a new pane
running `[editor] command` with the file's absolute path appended. The default,
`["hx"]`, is Eon's packaged Helix. Any terminal editor works:

```toml
[editor]
command = ["nvim"]
```

`command` is a direct argv array, like a popup's: no shell runs it, so write
`["emacs", "-nw"]` rather than `"emacs -nw"`. A bare name resolves through the
terminal PATH, as popup commands do, and a relative path such as `./bin/ed`
against the tab's directory. Each edit reads the setting again, so a
change applies to the next file you open without restarting the window. When
the editor is not on the terminal PATH, the edit fails with `edit-command` and
opens no pane; Eon does not fall back to Helix.

The Scrollback popup (Alt+Shift+E, or `eon pane history TARGET --edit`) opens
its snapshot file with the same command, in a popup instead of a pane. When
the editor is missing, it fails with `popup-command` and opens nothing.

- The editor must stay in the foreground until you are done. A GUI editor
  needs its wait flag, for example `["code", "--wait"]`; otherwise it returns
  at once, the popup ends, and Eon removes the snapshot.
- The snapshot is discarded when the popup ends. Dismissing or replacing the
  popup stops the editor and loses unsaved edits, so to keep edits, save them
  under another name first.
- Eon removes the snapshot's directory, but not state your editor keeps
  elsewhere, such as Neovim's swap, undo and shada files under
  `~/.local/state/nvim`.
- If a window's supervisor dies with a Scrollback popup open, the recovering
  supervisor adopts a still-running editor's snapshot and removes it when
  that terminal ends; the next start of that window's generation removes only
  snapshots no live editor holds. One whose generation never starts again,
  for example after an Eon update, stays until you log out on Linux, where
  the runtime directory is in memory. The macOS fallback under `/tmp` is not
  in memory, so there it stays until the system clears `/tmp`.
- Opening Scrollback reads the history and then starts the editor, each
  within its own deadline. On a machine too loaded to finish both within
  6 seconds, the window may report Eon as slow or unavailable while the
  popup still opens.

## Managed tools

A terminal without an explicit command uses `[shell].command`, listed in the
defaults above. `eon-nu` selects pinned Nushell. Select managed
Bash with `eon-bash` in `[shell].command`. Any other shell, such as a host
`zsh` or `fish`, runs as an unmanaged command without Eon's integrations.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations. Last, if your PATH has
[mise](https://mise.jdx.dev), managed Nushell and Bash activate it the way
`mise activate` does in your own shell, so a project's mise tools and
environment apply from the first prompt. Eon skips it when mise is missing,
fails, or your configuration already activates it, and `mise = false` turns
it off. Eon never installs mise or trusts its configuration for you.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` terminals also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-hx`, `eon-yazi`, `eon-ya`, `eon-lazygit`, and
`eon-lg`. Inside terminals their unprefixed names resolve to the pinned tools.
Eon's Helix reads its configuration, including the Steel files `helix.scm` and
`init.scm`, from `$(eon config dir)/helix/`, and never runs Steel files from a
repository's `.helix` directory. `hx --config-dir DIR` reads `config.toml` from
DIR but still takes `helix.scm` and `init.scm` from `$(eon config dir)/helix/`.
Eon does not start Helix when the configuration root's path is not valid UTF-8.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.

## Installation updates

`updates.nix_profile` and `updates.nix_entry` are unset by default. Set both to
choose the absolute Nix user-profile path and its exact entry name for
`eon update --check` and `--install`. A CLI `--profile ABS --entry NAME`
replaces this pair. With neither, Eon discovers only the invoking installation.
An incomplete pair, a relative profile or an empty name fails without mutation.
Without a CLI pair, any `config.toml` error, including in another section,
fails both modes with `invalid-configuration`. A complete CLI pair bypasses
configuration loading for these commands.

```toml
[updates]
nix_profile = "/home/me/.local/state/nix/profiles/profile"
nix_entry = "eon"
```

Checks run only when requested. Install requires consent; in JSON or without
a terminal, pass `--yes` and set an explicit or configured target. Channel,
automatic installation/restart and custom manager commands are fixed by the
installation's ownership and live-work safety rules. Cancellation gives only
the helper's children a fixed five-second termination grace before killing
and reaping them. This safety budget bounds shutdown and lets Nix release
resources; it is not a configurable build timeout. The shared Nix daemon and
running Eon processes receive no signal.
