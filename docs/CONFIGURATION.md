# Configuration

Settings live in `config.toml`; `eon config path` prints its full path, normally
`~/.config/eon/config.toml`. `eon config dir` prints the configuration root
that holds it, which also holds the managed tools' configuration directories.
After editing, run `eon config check` to check for settings Eon would reject at
startup or during later use. It needs no running Eon and changes no state; a
missing file uses this build's defaults. It exits 0 when valid, 1 on read or
validation failure, and 2 for invalid arguments. Errors name the file and the
key or source location when available. `eon config check --json` returns
`{path, valid, error}`, with `error` null or `{code, detail}`; see
[Exit status](USAGE.md#exit-status) for the error codes. Success does not mean
every key was recognised: unknown/ignored settings retain their usual behavior,
and this command reports no warnings. It does not check availability of external
programs, files or services.

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
detect_local_hosts = true # Treat plain-text localhost, 127.0.0.1 and [::1] with a port as links.
# focus_border_width: unset. Use Venus's focus outline width when omitted.
# focus_color: unset. Use Venus's focus colour when omitted.
font_fallbacks = ["Symbols Nerd Font Mono", "Accelerando Terminal"] # Ordered fallback font families.
# font_family: unset. Let Venus choose the font family when omitted.
# font_size: unset. Let Venus choose the font size when omitted.
# line_height: unset. Let Venus choose the line height when omitted.
padding_color = "extend" # How terminal padding takes its background colour.
pane_frames = true # Draw rounded pane frames.
# rows: unset. Let the window choose initial rows when omitted.

[panels]
heading = "notch" # Panel heading position: notch or connected. Applies on open or reopen.

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
# bundle_prefix: unset. Absolute existing managed prefix; excludes the Nix pair.
# allowed_signers: unset. Absolute independently trusted OpenSSH signer file.

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

[popups.editor]
# Editor uses [editor] command and keeps running when hidden.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+O" # Popup shortcut using physical keys.
label = "Editor" # Popup display name.

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

Header widgets and the session label use the unselected tab's filled capsule,
without resting outlines. CPU, RAM and disk percentages share the interface
font with tab labels and quota chips; each keeps a fixed slot sized for its
widest value. The icon and value stay left-aligned. The Shortcuts button is a
plain `?` with the other bar controls' hover and mint keyboard-focus treatment.
These fills and glyphs follow the fixed chrome palette and typography policy;
there is no separate widget style setting. Clock and version keep their
bottom-corner monospace text and cyan colour.

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
when pane frames are on, the current Agents row's outline, and the working and
background dots of agent attention badges on tabs and in the agent sidebar.
Where the colour would read below 3:1 contrast on what lies under the dots,
such as a dark custom colour on the bar or a white one on a white terminal
background, the dots blend it toward black or white just enough to reach 3:1.
Under a translucent
terminal background (`background_opacity` below 1), resting sidebar rows
depend on the desktop behind the window, so their dots sit on the bar's dark
disc, as on a tab. The 3:1 holds for a current agent's badge; a stale agent's
badge is dimmed on purpose and reads below it. The other badges keep their
own colours. It accepts
`preset:<name>` from the cursor presets above or `custom:#RRGGBB`; `random` is
invalid. The setting is omitted by default, leaving Venus's built-in focus
colour in use. The example selects `custom:#b207fb`. F6 keyboard focus colours a framed pane title's edge span mint;
other chrome controls keep their mint pill outline. This setting applies to full-Eon surfaces;
eonbase has no pane outline.

`border_width` sizes neutral pane-card, agent-sidebar-card, popup and native dialog outlines,
including their title edges and the popup marker in Agents and the attention inbox.
Eon Bar widgets are filled capsules without neutral outlines; `border_width`
does not affect them. `focus_border_width` sizes the selected pane's
whole outline. Both accept integers from 1 through 3 logical pixels; defaults
are omitted, leaving Venus's built-in widths in use. With the example's widths,
physical widths are `max(1, round(width * scale))`:
at scale 1.3 they draw at 1 and 1 physical pixels, and at scale 2 at 2
and 2. Pane frames off hides pane and sidebar outlines and title rules;
popups keep their outline. These settings apply to full-Eon surfaces;
eonbase receives neither override. F6 title focus uses the pane header's configured
stroke; other focus pills keep their one-physical-pixel outlines. The current
agent row has a one-physical-pixel outline in `focus_color`, adapted only as
needed for 3:1 against the sidebar background, without a selection fill. Hover
or keyboard focus adds the dark highlight fill to any row, adapted only as
needed for 1.5:1 against the background; the current outline keeps its colour.
Its mint keyboard-focus outline is one physical pixel, inset one pixel on a
current row and adapted only as needed for 3:1 against the row fill. Both row
strokes stay one physical pixel to match keyboard focus and the approved look;
pane/card border settings do not size them. Current text retains 4.5:1 against
the background or highlight fill. Selected tabs keep their fill.
Resting numbers and IDs are dimmer than names where 4.5:1 remains possible;
the selected tab label uses one primary text role. These colours follow the
fixed chrome palette; colour settings belong to the themes work.

`cursor_trail_duration` defaults to `1.5`. Set it to `0` (or `0.0`) to turn
the trail off while keeping the cursor's color and contrasting edge. To enable
the trail, use a finite multiplier from `0.25` through `4.0`; larger values
keep it visible longer. It applies when an Eon or eonbase window next opens or
reopens. Eon owns this setting for both products; no Venus or Orbit
configuration is needed.

With pane frames on, each column has one continuous terminal-background fill,
from its Stack tab's contour down to the column's rounded bottom.
The Stack band, stacked title rows and the space between cards share the
expanded terminal's background colour and opacity. Pane header bands, including
lone and zoomed panes, keep that shared fill; terminal edge colours never extend
into header chrome. Outside the shared title contour and outer corners, the gaps and
footer stay the opaque bar colour. The Agents fill follows the same contour.
Header scrolling keeps the fill's top fixed. Frameless mode keeps terminal
background strips across the window below the bar, including the sidebar and
footer.

Each column card's top line carries a Stack tab, labelled `Stack` when the tab
has one column and `Stack 1`, `Stack 2`, ... from left to right when it has
several; Stack N holds the panes whose `column` is N in `eon session --json`.
The Stack band is one title row plus the tab's rise, a third of the title
padding (24 px at the default font). A first collapsed pane's title row starts
that far below the band's top (14 px below the Stack line at the default
font). The expanded pane's framed header, a lone and a zoomed pane's included,
has one continuous raised band: the edge runs straight over the fitted title,
empty middle and right action group between rounded outer corners. A collapsed
pane shows its title and any visible buttons in gaps in its row's line. The
last collapsed title sits in the column's bottom edge. When room permits, the
expanded band starts one collapsed row's height below the line of the
collapsed title above it, or below the Stack line when the first pane is
expanded (20 px at the default font), including a lone or zoomed pane. For the
first expanded pane, the space needed for that top gap precedes its header and
the remaining reserve follows its terminal. A column's terminal keeps one
height whichever pane is expanded. Tight text or stroke clearance leaves the
expanded title in a compact gap. The column's sides run from the Stack line
down to the first pane's rounded top corners in the neutral border colour and
width, even while that pane is selected. Headers scroll below the band. On a
window too short for the band, a first header and one padded terminal row, the
band yields whole and the label goes.

Each pane header has a right action group: Zoom (`□`, Restore `▣` while zoomed)
and Close (`×`). The zoomed tab also shows `▣` after its name. Zoom/Restore
glyphs are fixed together so the tab and button keep the same meaning. The expanded pane's framed header keeps both in its band and a
collapsed row keeps them in a gap in its line; frameless headers keep the
controls inline. The title and scrollback indicator fit to its
left. Both controls disappear together if two full targets cannot fit; tight
clearances retain a compact gap. Stack, Agents, popup and
dialog headings have no pane actions, and a popup covers the underlying headers.
Popup and Agents headings have a Hide button of their own, and each Stack tab
a Close button; both are described below.
Hover and press highlight the individual button. A one-pane tab keeps a dimmed,
disabled Zoom button in its slot.

F6 cycles to pane chrome. Tab and Shift+Tab cycle the selected pane's title, Zoom
and Close, skipping disabled Zoom, then the selected Stack's Close while it
shows; Enter or Space activates a button. Up/Down
select another pane and return focus to its title. Action focus uses a transient
accent pill outline. Zoom first selects the named pane if necessary and then uses
Alt+Shift+Z's local view behavior; another action or zoom toggle cancels a pending
selection. Close targets its own pane exactly as Alt+W does, with the same
final-pane confirmation. After a keyboard Close, focus returns to the selected
pane's title. Escape cancels that prompt; Enter closes its held target.
Agents can use `eon pane close PANE`; zoom is window-local view state.
Action size, spacing and header corners derive from the configured typography
and existing title padding. Border widths, focus color and background opacity
apply to the continuous band. There are no separate action-geometry settings;
the controls keep full targets and text/curve clearance across supported fonts and scales.
The Hide buttons on popup and Agents headings and the Close button on each
Stack tab follow the same rule: their size, glyph and labels are fixed and
derive from the configured typography and title padding.

`[panels] heading` selects the heading position for Stacks, popups, Agents and
native dialogs. `"notch"` (the default) keeps the double notches described below.
`"connected"` joins them with a straight top edge at notch height and the card's
usual rounded outer corners, enclosing the existing title and actions without a
heading fill or separator. Titles, action targets, terminal rows and pane header
bands keep their positions. Tight headings retain the same compact fallback.
The raised top outline stays neutral at `border_width`; the selected pane's
focus colour and width begin below the existing card-corner join. With pane
frames off, Stack and Agents labels remain plain; popup and dialog frames follow
the selected position. The setting applies when a window opens or reopens;
existing windows retain their position until reopened.

In the default notch position, popup and Agents headings carry a second notch
at the right, mirroring the
title's, with one Hide button (`–`); the name fits to its left. The popup's
button hides the shown popup exactly as `eon popup hide POPUP` does, and the
Agents button hides the sidebar as Alt+Shift+H and `eon sidebar hide` do. Hover
fills the button's pill, a held press turns it accent, and releasing over it
activates it; a press held while another popup takes the heading does nothing.
With pane frames off, the popup keeps its outline and both notches, and the
Agents button shows beside its plain label with no notch or outline. With
frames on, a heading without clearance for its tab keeps a compact title and
the button sits in its own gap in the top edge. A heading too narrow for a
full button shows none. With keyboard focus in the sidebar, Tab and Shift+Tab
move between its rows and Hide, Enter or Space activates it, and Up or Down
returns to the rows. F6 skips a sidebar with no rows, so Hide has no keyboard
stop there and Alt+Shift+H hides it. The popup's button has no keyboard stop.
Most popups still hide on their own shortcut; Sessions' key confirms its
selection instead, and quick search and Files switch to each other. Screen
readers list `Hide popup <name>` and `Hide agent sidebar` as buttons.

In the default notch position, each Stack tab carries the same right notch with one Close button (`×`),
which always asks before it closes anything; [Usage](USAGE.md) describes its
prompt.

The expanded pane's header uses the continuous band above; collapsed pane
titles sit in their row's line. Stack tabs, popups, Agents and
native titled dialogs share a fitted tab style. Their outline rises seamlessly
from the left border over the fitted name,
then returns into the top edge with a tangent S curve spanning 1.5 times its
rise. The curve begins beside the visible text, including after truncation.
Stack, popup and Agents tab tops sit one card gap (8 px) below the bar's pills.
Popup, Agents and dialog tabs add no height or terminal rows inside their cards.
Each pane header uses the shared title row height. A Stack with several panes
reserves half a title row plus one tab rise (14 px at the default font) from
the terminal. A lone or zoomed pane reserves only half a title row (10 px at
the default font), costing about one terminal row. Multi-pane terminal
capacity is unchanged by the first-pane gap.
The reservation yields before the last padded terminal row on short windows.
A tab keeps its stroke clear of text and neighbouring chrome; where that
clearance cannot fit, the title keeps a compact gap in the top frame line.
A title is omitted only when it cannot fit. A tab keeps full accessibility
names and adds no input targets other than the Hide and Stack Close buttons
above; decorative
pane caps outside a header row do
not extend that row's click or accessibility bounds. The Stack label is a visual
heading only. Keyboard title focus colours the title's span of the continuous
pane edge; hover fills a rounded title pill. Button focus uses its accent pill
outline. Compact and frameless titles retain their focus pill.
Popup tabs keep their outline with pane frames off; Stack and Agents labels show
plain then, and the Stack band stays reserved. On a light terminal background a
Stack label darkens only as far as 4.5:1 contrast needs. Dialog tabs stay inside
their existing dismissal bounds and use the dialog's opaque background; other
tabs inherit their terminal background colour and opacity.

The pane band and the dimensions of both panel heading positions follow the configured font
size and line height (defaults: 16 px and 1.125), existing title padding, and `border_width`
(default: 1). A selected pane retains its `focus_border_width` and focus colour.
Terminal-backed fills use `background_opacity` (default: 0.8). The fixed policy
and its support-cost rationale are recorded in `eon-27deg`, `eon-2ibdd`,
`eon-u5uv2`, `eon-xaq6c` and `eon-yjrru`. The gap above a band follows the
title row height. Deriving both from the same typography keeps their spacing
in step
and gives short windows a single clearance rule. The Stack tabs are
fixed the same way: their labels (`Stack`, `Stack N`), the stack band (one title
row plus a third of the title padding: 24 px at the defaults) and the card gap
between the bar's pills and the tab tops (default: 8 px) derive from the font
size, line height, title padding and card gap. Their position is selected by
`[panels] heading`; their dimensions stay derived from these shared metrics
(`eon-whjhd`, `eon-6ui6p`).

`padding_color` controls terminal padding in panes, popups and eonbase.
`extend` uses the nearest edge cell's background on each side and
at the corners. On the primary screen, top or bottom padding keeps the default
background if that edge row contains any default-background cell (including an
explicit color matching the default) or a full-height powerline glyph.
Alternate-screen TUIs always extend. `extend-always` skips those exceptions;
`background` keeps the terminal default background throughout the padding.
Default backgrounds retain the configured opacity; explicit backgrounds paint
as they do in cells. Padding stays inside the rounded card and below outlines.
Terminal padding (around the grid, popup margins and frameless gutters) follows
`padding_color`; it never reaches above the terminal's top edge into header
chrome. Pane header bands keep the shared column background and opacity,
including lone and zoomed panes. The setting applies when a window opens or
reopens.

`background_opacity` sets how opaque each terminal's default background is, in
Eon and eonbase alike. It also covers the terminal padding, the title rows and
gaps that share that background (above), and the agent sidebar card. Hovered and
keyboard-focused agent rows stay opaque. Explicit cell backgrounds, selection,
text and the cursor keep their own colours, and the Eon Bar stays the opaque
bar colour. With pane frames on, so do the gaps and footer outside the cards;
with them off, the frameless terminal-background
strips below the bar, sidebar and footer included, take this opacity.
See-through areas still take clicks. `background_blur` asks the compositor to
blur what shows through. It is independent of opacity, so a fully opaque
background hides it, and a compositor without blur ignores it. See [Window
background and
decorations](../venus/docs/PRESENTATION.md#window-background-and-decorations).

`detect_local_hosts`, in Eon and eonbase alike, makes plain-text
`localhost:3000`, `127.0.0.1:8080` and `[::1]:5173`, with an optional path, a
link like a printed `http://` URL: Ctrl+click opens and Ctrl+Shift+C copies
`http://` followed by the text. Set it to `false` to link only URLs with a
scheme. See [Hyperlinks](../venus/docs/INTERACTION.md#hyperlinks).

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. The built-in order is listed above, so alien
glyphs avoid searching all installed fonts.
An explicit list replaces that order. The terminal package includes fonts for
ordinary text, Nerd Font icons, Accelerando and, on Linux, Noto Color Emoji; it
does not install them system-wide. The archive built without Accelerando omits
that font and defaults to `["Symbols Nerd Font Mono"]` for `font_fallbacks`.
Compositors may ignore blur or initial window
sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each terminal. Invalid startup configuration creates no fresh terminal;
a failed replacement leaves existing terminals and commands alive.

On a fresh full-Eon window, Anima plays after the desktop is ready and before
the first tab's quick search. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback, and
`eon --no-anima` (also on `eon run` and `eon window new`) skips it for one
launch; `style` is passed to the pinned Anima executable, and
`duration_seconds` accepts 1–30. Anima owns its style names; run
`eon anima --help` to see them. A failed animation continues to quick search.
Attachment, recovery, later tabs, eonbase, and a launch with `--dir` do not play
the startup animation.

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

A Stack close always asks, whatever this setting is: `eon column close` needs
`--yes`, and a Stack tab's Close button shows its prompt even for one idle
pane. The prompt has no off switch, because its answer is the consent that
binds the close to the exact terminals listed. Enter in that prompt is
ignored for 400 ms after the list first appears; this confirmation delay is
fixed, a safety floor that keeps a key pressed before the list could be read
from approving it.

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
keybinding = "Alt+Shift+U"
label = "Monitor"
keep_alive = true
```

Built-in popup commands, labels and shortcuts are listed in the defaults above.
Every built-in popup accepts `keybinding` and `label`. Git, Agent, Commands and
Sessions also accept `command`, `enabled` and `keep_alive`, and
Agent's `command` may also be `"auto"`. Anima and Scrollback also accept
`enabled`, and they reject `command` and `keep_alive = true`. Editor also
accepts `enabled`. It uses `[editor] command` and keeps running when hidden.
Eon warns and ignores `command` and `keep_alive` in `[popups.editor]`.
Editor counts toward the limit of 32 enabled popup entries. If your
configuration already uses all 32, disable Editor with
`[popups.editor] enabled = false` or disable another popup.
Project has no
command of its own:
it shows the tab's Files on quick search, and Alt+Z hides it again when quick
search is already shown. It rejects `command`, `enabled = false` and
`keep_alive = true`, since it is required. A
custom popup needs `command` and `keybinding`; its `label` defaults to its ID
and its `keep_alive` to `true`.
Agent's `auto` command runs the first of `codex resume --no-daemon`, `grok`,
`opencode`, `pi` and `claude --resume` whose executable is on the terminal
PATH. Codex runs without its shared daemon, so its `eon` commands act for the
Agent popup's own terminal (see
[Codex's shared daemon](USAGE.md#sending-to-a-terminal)).
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
`["hx"]`, is Eon's packaged Helix. The Eon package without Helix (`no-helix`)
has no default (see below). Any terminal editor works:

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

In the package without Helix, while `command` is unset each edit runs
`$VISUAL`, or else `$EDITOR`, as they were set when the window started. Eon
splits the value on ASCII whitespace and runs no shell, so `nvim -p` works, and
an editor whose path has a space, or that needs quoting, goes in `command`.
The first nonblank value decides; a blank value falls through to the next. If
the editor it names is not on the terminal PATH, the edit fails rather than
trying the other. With neither naming an editor, the edit fails with
`edit-command` and names this setting; Eon picks no editor for you.
`eon config defaults` shows `command` as unset there.

The Editor popup (Alt+Shift+O) uses the same editor resolution when it starts.
The editor keeps its buffers and working directory when the popup hides or
the tab directory changes. When the popup starts the packaged Helix, Eon
supplies its private listener socket. Remove Helix's own `--listen` option
from `[editor] command` to use the popup. An ordinary `eon edit` can still
pass that option to Helix.

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
  the client's 76-second exchange allowance, the window may report Eon as
  unavailable while the popup still opens.

Terminal startup has a fixed 15-second lifecycle budget for spawning Orbit,
publishing Ready and acquiring its management lease. A failed start gets a
separate five seconds to roll back. Recovery and Stop retain five-second
operation budgets. These bounds are not settings: they bound workspace
blocking and keep the CLI and window's waits consistent. The window reports
slow replies after two seconds while keeping its current workspace visible;
its exchange allowance is 76 seconds and Stop's is 81 seconds. A failed start
reports observed startup stages and whether cleanup completed. The command may
already have run, so repeat it only if replaying it is safe; never retry while
cleanup is unconfirmed.

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

When Eon activates Starship in `eon-nu` or `eon-bash`, its prompt omits the
leading blank line unless you have your own Starship config. Each prompt call
uses a packaged config containing only `add_newline = false` when
`STARSHIP_CONFIG` is unset and `$HOME/.config/starship.toml` is missing.
Starship does not use `XDG_CONFIG_HOME` for this file. Create
`~/.config/starship.toml` or set `STARSHIP_CONFIG` in your shell configuration
to use your own settings. In Bash, use
`export STARSHIP_CONFIG=/path/to/starship.toml`. Setting `add_newline = true`
restores the blank line. A config created while the shell is running takes
effect at the next prompt. Eon never edits your file or exports the fallback to other commands,
so Starship's configuration commands keep their native behavior. Other shells
and user-owned prompts receive no Eon Starship default.

Managed Bash reports command start and exit status through OSC 133 C/D marks,
including commands that fail or are interrupted. Pipelines report the shell's
final status. Eon preserves your prompt and tool hooks, and skips its Bash
marks when the prompt or its directly named hooks already contain C/D
reporting. Bash sends its marks to the prompt terminal or `/dev/tty`, so they
stay out of redirected output; with no terminal it emits none. Bash supplies
no prompt-start A mark. Managed Nushell uses its own
OSC 133 reporting, including when Eon starts from a VS Code terminal;
`TERM_PROGRAM` stays available to programs with its inherited value. Native
Nushell user autoload runs afterward and can override shell integration settings.
An OSC 133 opt-out in Nushell's `config.nu`,
`$env.config.shell_integration.osc133 = false`, is kept.
Eon disables Nushell's native OSC 133 before every prompt, including the first, when
stdout is not a terminal, so those marks stay out of pipes and captures.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` terminals also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-yazi`, `eon-ya`, `eon-lazygit`, and `eon-lg`.
Inside terminals their unprefixed names resolve to the pinned tools.
Packages with Helix also expose `eon-hx` and provide `hx` in their terminals.
In the `no-helix` package, `hx` resolves through your PATH.
Eon's Helix reads its configuration from `$(eon config dir)/helix/`. It is
built without Steel, so it never runs `helix.scm` or `init.scm`, whether they
are in that directory or in a repository's `.helix` directory.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.

## Installation updates

`updates.nix_profile` and `updates.nix_entry` are unset by default. Set both to
choose the absolute Nix user-profile path and its exact entry name for
`eon update --check` and `--install`. A CLI `--profile ABS --entry NAME`
replaces this pair. With neither, Eon discovers only the invoking installation.
An incomplete pair, a relative profile, a `nix_profile` path containing `..`,
or an empty name fails without mutation.
`updates.bundle_prefix`, also unset, selects an existing absolute installer-owned
Linux prefix instead of the Nix pair. A CLI `--prefix ABS` or complete profile
pair replaces the entire configured target. `updates.allowed_signers` is unset
until independent signer setup supplies an absolute OpenSSH allowed-signers
file outside the managed prefix; direct Install refuses without one. Eon never
supplies or fetches a key.
Relative paths, parent traversal and mixed targets fail. A CLI Nix pair bypasses
configuration loading. Other calls, including `--check --prefix`, read
configuration and the signer setting; parse errors fail with
`invalid-configuration`. A CLI prefix bypasses
configured target validation while retaining the signer setting.

```toml
[updates]
nix_profile = "/home/me/.local/state/nix/profiles/profile"
nix_entry = "eon"
```

For a managed prefix:

```toml
[updates]
bundle_prefix = "/home/me/.local/share/eon/install"
allowed_signers = "/home/me/.config/eon/allowed_signers"
```

See [independent signer setup](INSTALLER.md#install-a-release).
Checks run only when requested. Install requires consent; in JSON or without
a terminal, pass `--yes` and set an explicit or configured target. Channel,
automatic installation/restart and custom manager commands are fixed by the
installation's ownership and live-work safety rules. Cancellation gives only
the helper's children a fixed five-second termination grace before killing
and reaping them. This safety budget bounds shutdown and lets Nix release
resources; it is not a configurable build timeout. The shared Nix daemon and
running Eon processes receive no signal.
