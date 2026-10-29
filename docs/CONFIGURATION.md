# Configuration

`eon config-path` prints the configuration root, normally
`~/.config/eon`. Settings live in `config.toml`. This example chooses a font,
cursor preset and 100x30 grid; it is not the empty-configuration default:

```toml
[bar]
widgets = ["codex_quota", "claude_quota", "cpu", "ram", "clock", "version"]

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

[workspace]
warning_close_last_tab = true
animate_working_badge = true

[sidebar]
enabled = true
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
widgets = ["codex_quota", "claude_quota", "cpu", "ram", "clock", "version"] # Eon Bar widgets, in display order.

[terminal]
background_blur = true # Request compositor background blur.
background_opacity = 0.8 # Opacity of the terminal's default background.
# border_width: unset. Use Venus's neutral outline width when omitted.
# columns: unset. Let the window choose initial columns when omitted.
# cursor_trail_color: unset. Omission retains the workspace's random cursor colour.
cursor_trail_duration = 1.5 # Cursor tail duration multiplier.
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
zoxide = true # Enable the managed Zoxide directory integration.

[anima]
duration_seconds = 3 # Startup animation duration in seconds.
enabled = true # Enable this feature or popup.
style = "random" # Startup animation style.

[workspace]
animate_working_badge = true # Animate the working badge; false keeps it still.
warning_close_last_tab = true # Ask before stopping the final tab's terminals.

[sidebar]
enabled = true # Enable this feature or popup.

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
command = ["eon-files"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+Y" # Popup shortcut using physical keys.
label = "Files" # Popup display name.

[popups.git]
command = ["eon-lazygit"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+J" # Popup shortcut using physical keys.
label = "Git" # Popup display name.

[popups.project]
# Project is the required chooser.
keybinding = "Alt+Z" # Popup shortcut using physical keys.
label = "Project" # Popup display name.

[popups.workspaces]
command = ["eon-workspaces"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = false # Keep this popup running when hidden.
keybinding = "Alt+Shift+S" # Popup shortcut using physical keys.
label = "Workspaces" # Popup display name.
```
<!-- defaults:end -->

## Configuration behavior

Each build ignores keys it does not know, with one warning per key and read
naming its dotted path and the configuration file. This lets an older running
window keep using the settings it knows after a newer build adds a key. Known
keys with wrong types or values still fail, including unsupported widget names.

`[bar] widgets` chooses which Eon Bar widgets show and their order, left to
right between the tabs and the Shortcuts and Close buttons: the Codex and
Claude quota chips, CPU and RAM use, the local clock and `EON <version>`. Each
name appears at most once. A widget left out doesn't show, `widgets = []`
leaves only the tabs and buttons, and omitting the key uses the built-in list
above. The list applies when a window next opens or reopens, and an
unknown or repeated name is a configuration error. Narrow windows hide the
clock first, then RAM, CPU and the version, then shorten the quota chips,
whatever the order; then quota chips disappear one by one, the one closest to
the tabs first. The version belongs to its running supervisor; updating
the installation does not change it.

Omitting `cursor_trail_color` chooses a random color that lasts as long as the
running workspace, so `eon attach` reopens with the same color; a new workspace
picks again. eonbase keeps one color across reopens, though its first window
may differ. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically. The cursor body
uses the chosen color with a shape-aware contrasting edge unless Orbit supplies
an explicit cursor color.

`focus_color` colours the selected pane's whole outline in the selected column
when pane frames are on. It accepts
`preset:<name>` from the cursor presets above or `custom:#RRGGBB`; `random` is
invalid. The setting is omitted by default, leaving Venus's built-in focus
colour in use. The example selects `custom:#b207fb`. F6 keyboard focus in the chrome keeps
its mint pill outline. This setting applies to full-Eon workspace surfaces;
eonbase has no pane outline.

`border_width` sizes neutral pane-card, agent-sidebar-card and popup outlines,
including their title edges, and the Eon Bar widget outlines. `focus_border_width` sizes the selected pane's
whole outline. Both accept integers from 1 through 3 logical pixels; defaults
are omitted, leaving Venus's built-in widths in use. With the example's widths,
physical widths are `max(1, round(width * scale))`:
at scale 1.3 they draw at 1 and 1 physical pixels, and at scale 2 at 2
and 2. Pane frames off hides pane and sidebar outlines and title rules;
popups keep their outline. These settings apply to full-Eon workspace surfaces;
eonbase receives neither override. The current agent row and F6 focus pills
keep their one-physical-pixel outlines.

`cursor_trail_duration` is a finite multiplier from `0.25` through `4.0`;
its built-in value is listed above. Larger values keep the tail visible longer. Eon owns this
setting for both products; no Venus or Orbit configuration is needed.

With pane frames on, each column has one continuous terminal-background fill,
from its first pane's rounded top edge down to the column's rounded bottom.
Stacked title rows and the space between cards share the expanded terminal's
background colour and opacity, except that extended top padding fills an
unzoomed expanded pane's band below its title (`padding_color`). Above that edge
and outside the outer corners, the gaps and footer stay the opaque bar colour.
The first title sits on that boundary. Popup and Agents fills follow their
rounded outer edges in the same way, with their titles on the boundary. Header
scrolling keeps the fill's top fixed. Frameless mode keeps terminal-background
strips across the window below the bar, including the sidebar and footer.

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

On a fresh full-Eon workspace, Anima plays after the desktop is ready and before
the initial directory picker. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback;
`style` is passed to the pinned Anima executable, and `duration_seconds` accepts
1–30. Anima owns its style names; run `eon anima --help` to see them. A failed
animation continues to the picker. Attachment, recovery, later tabs, and
eonbase do not play the startup animation.

Closing the last tab quits its window and stops its terminals. While the
window holds other workspaces, it closes only that workspace.
`warning_close_last_tab = true` makes `eon tab close` and
`eon pane close` ask first when run from a terminal, listing the terminals that
will stop and, while other workspaces remain, naming the workspace that
closes; `--json` never asks. In the window, Alt+Shift+W, the bar's `×` and
Alt+W on the last pane show a prompt listing the same terminals: Enter quits,
or closes the workspace while others remain, Escape cancels, and Alt+Shift+D
keeps them running instead. With `false`, both close at once. The window reads
the setting when it starts.

`animate_working_badge = true` steps the working attention badge, on tabs and
in the agent sidebar, through the braille dots ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ten times a
second. `false` keeps it still at ⠋, for people who prefer no motion. Without
GPU rendering (a software graphics adapter) the badge is always still. The
window reads the setting when it starts.

`[sidebar] enabled = true` offers the agent sidebar, which lists
the coding agents in the window's tabs that report their status. `false` turns
it off; Alt+G and Alt+Shift+G still jump between agents. The window reads the
setting when it starts. Alt+Shift+H shows or hides the sidebar; it also steps
aside while the window leaves fewer than 80 cells for the terminals beside it.
See [Agent sidebar](USAGE.md#agent-sidebar).

Pane labels use a fixed policy: a pin, then sanitized terminal title, then compact
working directory, with differing-directory context. Commands and `eon pane rename`
set or clear a pin; pins are workspace state rather than configuration. The pane
ID dims only when the resolved fill and configured opacity permit readable contrast.

## Agent quota

Eon bundles pinned CodexBar CLI for Codex and Claude quota.
Codex uses your authenticated `codex` executable on `PATH`; Claude uses its local
OAuth credentials. Explicit sources avoid browser fallback and model prompts.
Eon does not change provider settings or manage accounts. CodexBar owns credential
selection and its local authentication/cache mechanisms.

Codex shows its available five-hour and seven-day windows in one chip; Claude
shows one chip per available window. Only reported windows appear; an idle
Claude window can lack a reset time. Hover and accessible labels report that
absence. Narrow windows may condense Codex's chip to its lower remaining
percentage, then hide quota chips one by one, the one closest to the tabs
first, before tabs and controls.

Eon reads a provider's quota only while `[bar] widgets` lists its chip, checked
when a window opens or reopens. A chip listed again shows its provider's
stored reading at once, if there is one, and otherwise appears after its
provider's next read.

Codex refreshes at least one minute apart and Claude at least five minutes
apart, also after a failed read or a chip shown again. A failed read keeps the last good values; they become `old` once they are more than an hour old and
disappear after 24 hours. Each supplied reset expires its own window. Missing
facts do not mean zero usage. Sign-out is not detected; the last reading ages
like any other and disappears after 24 hours.

Each provider's last good reading is also stored in `$XDG_STATE_HOME/eon/quota`
(`~/.local/state/eon/quota` by default), shared by all your Eon windows, so a
newly started window shows it until its own read succeeds. The stored reading
holds only windows, remaining percentages, resets and when they were read.

Claude needs no status-line setup.

Existing Eon windows retain their running build after installation updates;
start a fresh workspace with the updated command to use this backend.

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
Every built-in popup accepts `keybinding` and `label`. Git, Agent, Files,
Commands and Workspaces also accept `command`, `enabled` and `keep_alive`, and
Agent's `command` may also be `"auto"`. Anima also accepts `enabled`; it
rejects `command` and `keep_alive = true`. Project rejects `command`,
`enabled = false` and `keep_alive = true`, since it is the required chooser. A
custom popup needs `command` and `keybinding`; its `label` defaults to its ID
and its `keep_alive` to `true`.
Agent's `auto` command runs the first of `codex resume`, `grok`, `opencode`,
`pi` and `claude --resume` whose executable is on the terminal PATH.
Files opens Yazi with Eon's own popup keys on Alt+Shift+Y and reads no Yazi
configuration of yours. Setting `[popups.files].command` replaces it, for
example with `["eon-yazi"]`, which reads the `yazi` directory under Eon's
configuration root.
Anima opens a random animation and stops when dismissed or replaced.
Commands searches and runs Eon commands.
Workspaces lists this window's workspaces most recently shown first, the shown
one last, marking one that needs input: Enter shows the highlighted one, and
Enter on a name that matches none creates a workspace with that name.
Set `enabled = false` on Git, Agent, Files, Anima, Commands, Workspaces, or a
custom popup to disable it.

A `keybinding` you configure, on a custom popup or a built-in one, takes its
key first. A built-in popup's default key applies only while no configured
popup holds it; otherwise the built-in popup stays without a key, and Alt+/
lists it under its default key as overridden by the popup that holds it.
`keybinding = ""` leaves any popup, Project included, without a key. Until the
CLI path `eon-agent-parity-a9g0.1` lands, a popup without a key cannot be
opened from the window, except that Project still opens for a new tab's folder
choice and the Eon Bar's workspace label still opens Workspaces.
Two configured popups on one key fail at load. A popup shortcut on the key of a
built-in Eon shortcut takes that key, and Alt+/ lists the built-in one as
overridden.
Alt+/ and Alt+0 through Alt+9 are reserved and cannot be bound.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

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
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.
