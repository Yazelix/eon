# Presentation and ownership

## Native window and workspace

Venus presents one native Wayland window on Linux. It can show one local Orbit
terminal or an Eon window. In workspace mode Eon supplies the tabs, pane
order, selection, and popups; Venus draws the visible scene.

### Eon Bar

- Tabs show their one-based position and the directory leaf or `/` from Eon's tab
  launch directory. Long labels use middle ellipsis; hover shows the full path.
  Selection brightens the tab, and keyboard focus adds an outline.
- Tab pills and quota, CPU, RAM and disk widgets share one height and
  centre line, starting half the card-title height below the window top. The
  allocated card areas start lower so the tops of the card title tabs sit one
  card gap below the pills. This extra space yields first on
  short windows, before the bar's top gap and pill height. Tabs keep their
  fills; widgets have no fill and use rounded outlines in the pane cards'
  neutral colour and `border_width`. The header controls share their centre.
  Hover and accessibility bounds stay unchanged.
- The Eon logo, in its own colours and two logical pixels shorter than the tab
  pills, leads the bar before the session label and tabs. Its centre is as far
  from the window's left edge as the bar's centre line is below the window top,
  and the next item starts as far after it as it is from the left edge. With a
  running-windows summary it is the **Window menu** button, with hover,
  pressed and keyboard-focus feedback and an accessible name. It keeps its slot when tabs overflow and tabs scroll beside it.
  Quota chips and the session label yield before the icon. Only a tiny width
  that cannot fit the icon, the minimum tab area and the trailing controls
  hides it; existing minimum geometry sets that threshold. Without a readable icon from Eon there is none and no gap.
- The Window menu opens by clicking the Eon icon, activating its accessible
  button, or pressing F6 then Tab to focus it and Enter or Space. It shows this
  window's name, ID and live-terminal count, Detach and Quit, and the other
  running windows with their counts and a Quit action each. These are Runtime's
  running-window facts; unknown counts and unresponsive windows say so. The
  menu stays available with shortcut hints and the sidebar off. Its icon keeps
  the minimum-width rule above. The session pill still opens the Sessions picker;
  the hints-bar version stays read-only.
  With Commands enabled, **Updates** opens that configured popup; search
  `update` to check or install through the product CLI's target and consent
  flow. It shares pointer, keyboard and accessible menu activation.
  Pointer activation waits for release over the pressed row; hover never scrolls.
  Arrow keys or Tab/Shift+Tab choose an action; Enter or Space activates it.
  Escape or a press outside closes the menu without activating what is behind
  it. Longer lists scroll; accessible buttons retain their target identities
  as windows appear or disappear. Detach keeps the window's terminals running.
  This window's Quit uses the existing busy-terminal prompt; another window's
  Quit always confirms its name, ID and count before `eon window stop ID`.
  ID and count have their own confirmation rows; only the name is clipped.
  Independent namespaces with several generations, this window, or unknown
  scope are refused with the CLI commands to inspect and stop the namespace.
  An ended target dismisses its prompt; failures appear as a local notice.
  `eon window ls` reads the inventory, `eon quit` quits the caller's window,
  and `eon window stop ID` quits an addressed window. Detach and opening this
  native menu are local view actions.
- New tab sits beside the last tab and pins to the tab-region edge when tabs
  overflow. Shortcut help, a round outlined button as tall as the tab pills
  with its `?` centred in the circle, then Close stay at the right; after a
  chip or reading, the button keeps the space chips keep
  between them. Close's `×` lies as far from the window's right edge as from
  its top. A small empty area lets users drag an undecorated window without
  taking clicks from tabs or controls.
- Codex and Claude facts show one read-only chip per window, five-hour then
  seven-day, each with its provider's mark (the monochrome OpenAI Blossom, or
  Claude's splat in its terracotta `#D97757`),
  its elapsed/total window position and remaining percentage. A window without
  a reset shows no position. Accessibility names the provider and that window,
  its remaining percentage, and how far into it the observation is, or that its
  reset time is unavailable. Hover lists the provider and window, then when it
  resets in local time, in the clock's time zone: the time alone today (`Resets
  at 18:30`), with the weekday within the next six days (`Resets Tue 18:30`),
  and with the date after that (`Resets 14 Nov 18:30`), so it stays right
  however old the reading is; `Has reset since this reading` once that time has
  passed, with no pace line then, or `Reset time unavailable`. Without a local
  time zone hover leaves the reset out, as the bar leaves out the clock. A hover
  shown across midnight keeps its wording until the bar next redraws, and the
  next hover reads the new day. Claude's model-scoped weekly
  windows follow its unscoped chips, one chip each in EONW order, with the
  scope before the position (`Fable 3d/7d 60%`); hover and accessibility name
  it, as in `Fable-only 7d window`. Chips disappear one by one, the one
  closest to the tabs first, before tabs, controls or dragging lose space.
  `old` stays visible on every stale chip; blocked and unknown Codex
  observations show one chip with their state. Missing facts and transport
  loss remove the affected chips.
  A window whose length is not a whole number of hours shows it in exact
  minutes, such as `90m`. A stale chip that cannot fit its label with `old`
  disappears rather than drop `old`. Hover and accessibility never show raw
  epoch timestamps.
- A fresh window with a reset, past its first tenth, colours its chip's label by
  pace on a continuous green, yellow, orange and red scale, projecting the
  snapshot's used and elapsed shares of the window linearly to the reset: green
  is comfortably under pace, exactly on pace is a light green turning toward
  yellow, and a window that runs out a twentieth of a window before the reset
  is yellow, a tenth orange, and a quarter or more full red. A used-up window
  is always coloured. Stale, blocked, unknown, reset-less and younger windows
  keep the neutral text colour. Hover and accessibility say in words that at
  this pace the window lasts until reset, runs out about a stated time before
  it, or is used up, so colour is never the only signal. Every colour keeps at
  least 4.5:1 contrast on the bar.
- The CPU and RAM chips show whole-machine use as an icon and a percentage,
  such as `12%`, refreshed every 2 seconds. CPU covers the last 2 seconds and
  first appears 2 seconds after the window opens; on macOS user, system and
  nice time count as busy. RAM is MemTotal minus MemAvailable over MemTotal on
  Linux, and Activity Monitor's Memory Used over physical memory on macOS, and
  appears with its first reading, when the window opens. The icons are Symbols
  Nerd Font's `nf-oct-cpu` (U+F4BC) for CPU, `nf-fa-memory` (U+EFC5) for RAM
  and `nf-fa-hard_drive` (U+F0A0) for disk, drawn from that bundled face
  whatever the terminal font and its fallbacks; hover and accessibility name
  each reading in words. Each reading chip keeps the width of its icon and
  ` 100%`, six cells, with its text centred, so neighbouring items never shift.
  Accessibility says, for example, `CPU use 12% across 16 logical CPUs, over
  the last 2 seconds` and `RAM use 43%, 13.5 GiB of 31.3 GiB`; hover lists
  `CPU` then `16 logical CPUs`, and `RAM` then `13.5 GiB used of 31.3 GiB`, the
  same use the percentage comes from. A reading that cannot be taken
  shows no label, never 0%, until the next valid one. Other platforms show
  neither label. CPU hover's title appends the CPU name when available, as
  `CPU · 13th Gen Intel Core i5-13450HX`, with `(R)`, `(TM)` and `(C)` marks
  removed and spaces collapsed. The name is read once: Linux's `/proc/cpuinfo`
  `model name`, macOS's `machdep.cpu.brand_string`. The logical-CPU count line
  adds ` · 59 °C` when Linux hwmon exposes coretemp's `Package id 0` or
  k10temp's `Tctl` (or `Tdie` without Tctl); ACPI and board sensors are not CPU
  readings. macOS has no temperature reading. After the average, hover lists
  the busiest logical CPU over the same 2-second interval, `CPU 3 at 97%`,
  numbered from zero, then `Load 1.2 over 1 min`, from Linux's `/proc/loadavg`
  or macOS's `getloadavg`. A changed online CPU set or invalid per-CPU delta
  omits the busiest line; an unavailable detail omits only that detail.
  Accessibility appends the same name, temperature, busiest CPU and load,
  separated by semicolons.
- The disk chip shows, as its icon and a percentage, the used share of the
  filesystem holding the path Eon's `[bar] disk_path` names, by default the
  home directory, as GNU df computes it from `statvfs` on Linux and macOS: used
  blocks over used plus available ones, so space reserved for root counts as
  neither, rounded up. It appears with its first reading when the window opens
  and refreshes every 30 seconds, since disk use changes slowly. The reading
  runs off the window's thread, so a slow or hung filesystem never freezes the
  window; the chip keeps its last reading meanwhile.
  Accessibility says, for example, `Disk use 80% of 929.4 GiB on /, 181.2 GiB
  free`, and hover lists `Disk · /` then `181.2 GiB free of 929.4 GiB`, naming
  the topmost directory above the path on the same filesystem (device),
  usually its mount point, or the path itself when that cannot be found.
  Sizes keep one decimal, like RAM's. A path that does not exist, or a
  reading that cannot be taken, shows no label, never 0%, until the next valid
  one. Other platforms show no label.
- CPU, RAM and disk chips share three inclusive use grades: yellow from 60%,
  orange from 75% and red from 90%, neutral below 60%. The grade follows the
  whole percentage displayed, so CPU bursts show. A supported Linux CPU
  temperature also colours the CPU chip orange from 85 °C and red from 95 °C,
  using the stronger temperature or utilization severity. Hover adds
  `Running hot` and accessibility appends `; running hot` at either thermal
  level. Each chip names yellow as `Elevated use` in hover and `: elevated use`
  in accessibility. CPU names orange `Under high load` and red
  `Under heavy load`; RAM names orange `Under pressure` and red
  `Under heavy pressure`; disk names orange `Filling up` and red `Nearly full`.
  Accessibility uses the same words in lowercase after a colon, so colour is
  never the only signal. CPU also gives its average over the last 30 seconds:
  accessibility appends `; 45% on average over the last 30 seconds`, and hover
  lists `45% on average over the last 30 s` before the load line. The average
  needs a full window of unbroken readings, so it starts over after a failed
  reading or a stall. Yellow, orange and red are the quota pace scale's, at
  4.5:1 or more on the opaque bar, including over light terminal themes.
  Eon's `[bar]` sets each chip's notice, warning and critical percentage and
  the averaging window; equal percentage levels select the stronger grade.
  macOS uses the same levels.
- The clock shows local 24-hour `HH:MM` and changes within a second of each
  minute; after suspend or a system clock change it corrects within a minute.
  It sits at the left end of the bottom shortcut hints, one padding from the
  window edge, as plain text in the version label's terminal monospace and
  softened cyan, with no outline, on the version's row, as far from the left
  edge as the version ends from the right; it is five cells of terminal
  monospace wide so it never shifts the keycaps; it hides with the hints, and
  in windows too narrow for it beside their keycaps once the version has
  hidden. Its hover opens above it.
  Accessibility names the time, date and UTC offset, such as `Local time 14:05,
  Wednesday 21 October 2026, UTC−03:00`; hover lists the date then the offset,
  `Wednesday 21 October 2026` and `UTC−03:00`. When the local time zone
  cannot be determined, no clock shows, never UTC in its place.
- `EON <version>` names the supervising Eon that serves the window, in regular
  terminal monospace and softened cyan, at the right end of the bottom shortcut
  hints, one padding from the window edge; it hides with the hints, and in
  windows too narrow for it beside their keycaps. Hover reads `Eon version
  0.1.18`, above the label, and its accessible label names the version too. Updating the installation does
  not relabel a running supervisor. Without a version from Eon, no label shows.
- Each item's hover is a short list, one fact per line: the first line names
  the item, except the clock's, whose date is self-evident, and the rest give
  only facts its chip does not show. Normal states stay quiet; stale data,
  blocked or unknown permission, high load, memory pressure and a filling or
  nearly full disk each add their own line. A quota hover reads, for example,
  `Codex 7d window`, `Resets Sun 21:00`, `Runs out about 1d18h before reset`
  (or `Lasts until reset at this pace`, or `Used up until reset`), then `Data
  is stale`, `Permission blocked` or `Permission unknown` when so; blocked and
  unknown observations have no window, so their first line is `Codex quota`.
  Accessibility keeps one sentence per item with every fact, including what
  the chip shows.
- Eon's `[bar] widgets` chooses which of these items show and their order, left
  to right between the drag region and Shortcuts; the clock always sits at the
  left end of the shortcut hints and the version at their right end. By default
  the order
  is Codex 5h and 7d, Claude 5h and 7d then its scoped windows, CPU, RAM, disk,
  clock and version. A left-out quota window takes no room, including the
  logo's reserved room. `_5h` and `_7d` select only unscoped windows
  of those durations, and `claude_quota_scoped` selects every model-scoped
  Claude window. Codex's 5h widget also shows its one blocked or unknown
  observation when no windows are available. A left-out
  item has no chip, hover or accessible node, and a left-out CPU, RAM, disk or
  clock is never read and never wakes Venus. Accessibility order follows the
  visual order, so screen readers meet the clock after the panes, before the
  hints. The header items use only spare drag-region space and keep its minimum.
  Whatever the order, narrow windows hide the Eon logo first, then disk, RAM and
  CPU, then quota chips, before tabs or controls. No item has an action or a
  keyboard focus target, and none drags the window. A change in a quota's numbers, a reading or the clock never moves
  the tabs, cancels scrolling or resizes terminals, and neither does a window's
  reset. The one exception is the Eon logo: it can show or hide once, shifting
  the tabs, when a provider's quota first appears or goes away (as it does when
  all its windows reset before a read), or when a read finds the provider has
  gained or lost a window.

### Panes and popups

A tab shows one to four equal-width columns side by side. In each column one
pane expands in a vertical stack while every fitting pane header stays
visible. Each pane is its own rounded card, stacked without gaps in its
column's allocated card area. Even outer gaps surround and separate the
allocated column areas. Collapsed panes show a rounded title edge with corners turning toward the
expanded pane and sides continuing to the next card in that direction. The
expanded pane has a full rounded frame. Every pane title is a compact gap in its
rounded edge; the column card's top line above the first pane carries its Stack
tab.
`border_width` sizes neutral outlines; `focus_border_width` sizes the selected
pane's outline. The visible shortcut footer sits bare in the margin below
the allocated column areas.
Selection, hover, and keyboard focus have distinct fills or outlines.

With pane frames on, each column has one continuous terminal-background fill,
from its Stack tab's contour down to the column's rounded bottom.
The last collapsed pane's label sits in that bottom edge, closing the stack with
one line. At the unscrolled bottom, columns and the Agents card share one snapped
painted bottom line. Its inset is half the title height, bounded by terminal
bottom padding minus the widest stroke, so frames stay clear of grid cells.
After resize rounding, the shared line moves outward as needed to leave a
full stroke beyond every visible terminal grid, within the allocated bottom.
A last expanded frame ends there inside its bottom padding when space permits.
When collapsed rows follow it, the expanded frame's painted bottom instead
sits one title row above the next label line. The same padding and rounded grid
bounds limit how far it rises; its rounded joints and the rows' sides meet
the raised edge. The column keeps its continuous background fill.
While a resize is pending, both edges also clear the retained frame's displayed
rows; the full rise returns when the resized frame arrives.
If a short, scrolled viewport cannot fit its bottom stroke below the grid, the
stroke stays below the grid and the card clips it. Terminal layout,
grids, header bands, hit targets and AccessKit bounds stay unchanged, as do
popup and frameless paint.
Last collapsed labels center on the line, clamped so their text boxes end at
or above the bottom of their unchanged title bands.
The Stack band, stacked title rows and the space between cards share the
expanded terminal's background colour and opacity. Extended top padding fills
an unzoomed expanded pane's band below its title (`padding_color`). Outside the shared title contour and outer corners, the gaps and
footer stay the opaque bar colour. The Agents fill follows the same contour.
Header scrolling keeps the fill's top fixed. Frameless mode keeps terminal
background strips across the window below the bar, including the sidebar and
footer.

Each column card's top line carries a Stack tab, labelled `Stack` when the tab
has one column and `Stack 1`, `Stack 2`, ... from left to right when it has
several; Stack N holds the panes whose `column` is N in `eon session --json`.
The first pane's top edge sits one stack band lower: one title row plus the
tab's rise, a third of the title padding (24 px at the default font). Every
pane title, the first, a lone and a zoomed pane's included, is a compact
border-gap title in its rounded edge. The column's sides run from the Stack line
down to the first pane's rounded top corners in the neutral border colour and
width, even while that pane is selected. Headers scroll below the band. On a
window too short for the band, a first header and one padded terminal row, the
band yields whole and the label goes.

Stack tabs, popups, Agents and native titled dialogs use one shared title
style. The outline rises seamlessly from the left border over the fitted name,
then returns into the top edge with a tangent S curve spanning 1.5 times its
rise. The curve begins beside the visible text, including after truncation.
Stack, popup and Agents tab tops sit one card gap (8 px) below the bar's pills.
A popup, Agents or dialog tab adds no height or terminal rows inside its card.
A tab keeps its stroke clear of text and neighbouring chrome; where that
clearance cannot fit, the title keeps a compact gap in the top frame line.
A title is omitted only when it cannot fit. A tab keeps full accessibility
names and adds no input targets; the Stack label is a visual heading only.
Popup tabs keep their outline with pane frames off; Stack and Agents labels show
plain then, and the Stack band stays reserved. On a light terminal background a
Stack label darkens only as far as 4.5:1 contrast needs. Dialog tabs stay inside
their existing dismissal bounds and use the dialog's opaque background; other
tabs inherit their terminal background colour and opacity.

Visible headers pair Eon's opaque pane
identity with Orbit's working directory. Home displays `~`, and its descendants
use `~/`; without `HOME`, paths stay absolute. Every column's expanded pane
presents live, unless Alt+Shift+Z shows the selected pane alone across the tab;
only the selected pane receives input. Ctrl+Alt+M adds
another column unless one would get fewer than 40 terminal columns.

Zoom shows one rounded card with the selected pane's label in its top edge,
including in a one-column tab with several panes. The other panes keep running
at their last sizes. Changing the active tab, selected pane or pane/column layout
ends zoom; toggling it again restores the columns at the current window size.
A one-pane tab refuses the toggle with a notice. A zoom-in magnifier (Nerd Font
U+F00E) follows the tab's name in its label colour and on the same baseline;
it participates in tab width fitting. If the bundled Symbols Nerd Font face
is unavailable, the marker falls back to ` · zoom`. The marker
and `, zoomed` accessible name show while zoom is visible, and the pane panel
is named `Zoomed pane <label>`. Popups cover zoom and hide its marker until they close.

A fresh session or tab can begin with its Files popup, on quick search, and no
pane. Popups draw as one card over every column, with Eon's
logical margins around their terminal. Tabs remain available. A tab with only hidden popups has an
empty body and keeps focus on its tab until a catalog shortcut reopens work.
Venus recovers retryable socket loss and detaches without ending Sessions.

The cursor tail follows pane, column and tab switches and workspace layout
changes within the window; only the selected column draws it. During
attachment it waits without drawing the old pane; motion resumes toward the
new pane's fresh cursor. Failures, hidden cursors,
focus loss, occlusion, native resize, and GPU recovery reset the effect.

### Window background and decorations

Eon workspace windows have no native title bar; the Eon Bar's empty area moves
them. eonbase keeps the desktop's decorations unless started with
`--no-decorations`.

Background opacity, Eon's `background_opacity`, applies to each terminal's
default background, including later default-background changes from the program,
to terminal padding, to the title rows and card gaps that share that background
(Panes and popups, above), and to the agent sidebar card, which takes the
background of the terminal in the leftmost visible column. A bare Venus launch
is opaque. Popup-row tints stay opaque. Explicit cell backgrounds, selection,
inverse video, notices, focus outlines, text and the cursor keep their own
colours, and the Eon Bar stays the opaque bar colour. With pane frames on,
so do the gaps and footer outside the
cards; with them off, the frameless terminal-background strips below the bar,
sidebar and footer included, take this opacity. See-through areas still take
clicks and keys, and accessibility is unchanged. A display that cannot present
premultiplied transparency refuses a translucent window before showing it rather
than drawing it opaque.

Background blur, Eon's `background_blur`, asks the compositor to blur what
shows through translucent pixels before the window first appears. It is
independent of opacity: an opaque background hides it. The compositor decides
whether and how strongly to blur; one that does not support blur ignores the
request, and the window still opens.

## Agents sidebar

An agent row uses the pane's pinned name when it has one, ahead of the agent's
own reported name. The app icon stays beside it. `eon pane rename` sets or clears
that pin; without one, rows keep their existing fallback: Claude Code's reported
session name, then the record title, then the app name (`Claude`, `Codex`, or the
reported app verbatim), else `agent`. Other apps' terminal titles are not session
names. With no record and no pin, the row says `status unavailable`.

The attention inbox uses the same name for panes in the shown session. Rows of
another session retain the reported-name/title/app fallback because the queue
carries no pane pin. Popup agents have no pane pin and keep that fallback too.
Rows retain their state, app icon, age, source pane ID or popup label and
needs-input message; their accessible names include the pin when present.

Popup-source rows from the shown session in the sidebar and attention inbox have
an opaque tint of the terminal focus colour, including while hovered or
keyboard-focused. The resting tint leaves a visible step for hover, including
with light focus colours. The tint keeps the row's labels readable over light
or dark terminal backgrounds; the
current-row and keyboard-focus outlines draw above it. The inline window mark
after the popup's catalog label remains, as does its accessible "popup" name.

## Ownership

```text
Eon                     -> product policy, composition, distribution
Eon Runtime             -> session mechanisms, canonical EONW
Venus                   -> native presentation, interaction, client failure UX
Orbit                   -> PTYs, terminal state, session lifetime, wire authority
```

Venus consumes EONW v38 through the in-tree `eon-workspace-protocol` 0.1.0
crate. Eon supplies popup definitions,
commands and geometry defaults. The runtime validates configuration and owns
live session and popup state, Session mappings, and provider quota collection
and normalization. Venus consumes
the in-tree `orbit-protocol` 0.1.0 crate with ORBF v3 and ORBS v14. One reducer turns complete canonical
frames into immutable scene data used by drawing and accessibility. The native
host owns the local socket, window, input mapping, and redraw lifecycle; it owns
no terminal state.

Each Eon pane references an independent Orbit terminal. Venus keeps
the EONW connection for workspace state/actions and one presentation connection
to the selected popup or each column's expanded pane. Runtime owns read-only
metadata observers for panes in every tab and session and supplies semantic
labels over EONW. Venus opens no separate label observers. Hidden terminals keep
running; the columns that zoom hides retain their presentation connections.

Terminal accessibility text and selection come from the same accepted Scene.
Atomic units retain their terminal columns, and AccessKit projects their
positions and widths through the rendered grid and visible clipping. Scoped
Linux Orca line-review evidence predates this repository and awaits re-proof
under VEN-C1; complete composed screen-reader qualification and native reader-key delivery remain separate.

Pane labels come from Runtime: the pane ID, then a pinned name, otherwise its
sanitized terminal title, otherwise its compact working directory. A differing
terminal directory adds ` · DIRECTORY`, using a relative or shorter compact path.
Labels keep explicit connecting, unavailable and offline states. The ID is dimmer
when the resolved background, including an extended header band, and opacity
permit readable contrast; narrow labels
keep the ID when it fits, while accessibility and close listings retain the full
bounded label. Metadata and names are display text, never command input.

## Platform and limits

This slice excludes arbitrary split trees, vertical splits, column resizing,
sidebars other than the agent sidebar (VEN-C30), popup command/lifetime
policy, persistent Venus configuration, background images, plugins, remote or
web access, packaging, and distribution.
Blur strength and live blur changes are also outside its scope.

The proved Linux host uses winit, wgpu, glyphon, AccessKit, and wl-clipboard-rs
with native Wayland and Vulkan. The unsupported Apple Silicon host reuses the
same renderer/model path with AppKit, AccessKit, Metal, and target-only arboard.
Its opaque foundation and a bounded clipboard/key slice are partially
re-proved natively in this repository (`VEN-C16`). Retina-scale redraw,
renderer recovery, Kotoeri and other IMEs, dead keys, held-key repeat, focus
loss, physical trackpad gesture quality, the PNG-only and oversize-pasteboard
notices, the OSC 52 write, a host-access failure notice, lifecycle acceptance,
effects, and Eon composition remain open.
