# Presentation and ownership

## Native window and workspace

Venus presents one native Wayland window on Linux. It can show one local Orbit
Session or an Eon workspace. In workspace mode Eon supplies the tabs, pane
order, selection, and popups; Venus draws the visible scene.

### Eon Bar

- Tabs show their one-based position and the directory leaf or `/` from Eon's tab
  launch directory. Long labels use middle ellipsis; hover shows the full path.
  Selection brightens the tab, and keyboard focus adds an outline.
- Tab pills and quota, CPU, RAM and clock widgets share one height and centre
  line, starting half the card-title height below the window top. The allocated
  card areas start lower so the gap to the tops of the titles' capital letters
  equals the space above the pills. This extra space yields first on short
  windows, before the bar's top gap and pill height. Tabs keep their fills;
  widgets have no fill and use rounded outlines
  in the pane cards' neutral colour and `border_width`. The plain version label
  and header controls share their centre. Hover and accessibility bounds stay
  unchanged.
- New tab sits beside the last tab and pins to the tab-region edge when tabs
  overflow. Shortcut help and Close stay at the right. A small empty area lets
  users drag an undecorated window without taking clicks from tabs or controls.
- Codex and Claude facts show one read-only chip per window, five-hour then
  seven-day, each with its provider's mark (the monochrome OpenAI Blossom, or
  Claude's splat in its terracotta `#D97757`),
  its elapsed/total window position and remaining percentage. A window without
  a reset shows no position. Hover and accessibility name the provider and
  that window, and say how far into it the observation is, or that its reset
  time is unavailable. Chips disappear one by one, the one closest to the tabs
  first, before tabs, controls or dragging lose space. `old` stays visible on
  every stale chip; blocked and unknown Codex observations show one chip with
  their state. Missing facts and transport loss remove the affected chips.
  A window whose length is not a whole number of hours shows it in exact
  minutes, such as `90m`. A stale chip that cannot fit its label with `old`
  disappears rather than drop `old`. Hover and accessibility describe times as
  positions within the window, never as raw epoch timestamps.
- A fresh window with a reset, past its first tenth, colours its chip's label by
  pace on a continuous green, yellow, orange and red scale, projecting the
  snapshot's used and elapsed shares of the window linearly to the reset: green
  is comfortably under pace, exactly on pace is a light green turning toward
  yellow, and full red runs out at least half a window before the reset. A
  used-up window is always coloured. Stale, blocked, unknown, reset-less and
  younger windows keep the neutral text colour. Hover and accessibility say in
  words that at this pace the window lasts until reset, runs out about a stated
  time before it, or is used up, so colour is never the only signal. Every
  colour keeps at least 4.5:1 contrast on the bar.
- `CPU 12%` and `RAM 43%` show whole-machine use, refreshed every 2 seconds. CPU
  covers the last 2 seconds and first appears 2 seconds after the window opens;
  on macOS user, system and nice time count as busy. RAM is MemTotal minus
  MemAvailable over MemTotal on Linux, and Activity Monitor's Memory Used over
  physical memory on macOS, and appears with its first reading, when the window
  opens. Each chip keeps the width of `CPU 100%` with its text centred, so
  neighbouring items never shift. Hover and accessibility say, for example, `CPU
  use 12% across 16 logical CPUs, over the last 2 seconds` and `RAM use 43% of
  31.3 GiB`. A reading that cannot be taken shows no label, never 0%, until the
  next valid one. Other platforms show neither label.
- The clock shows local 24-hour `HH:MM` and changes within a second of each
  minute; after suspend or a system clock change it corrects within a minute.
  Hover and accessibility name the time, date and UTC offset, such as `Local
  time 14:05, Wednesday 21 October 2026, UTC−03:00`. When the local time zone
  cannot be determined, no clock shows, never UTC in its place.
- `EON <version>` names the supervising Eon that serves the window, in regular
  terminal monospace and softened cyan, with the full version on hover and in
  its accessible label. Updating the installation does not relabel a running
  supervisor. Without a version from Eon, no label shows.
- Eon's `[bar] widgets` chooses which of these items show and their order, left
  to right between the drag region and Shortcuts and Close. By default the order
  is Codex quota, Claude quota, CPU, RAM, clock and version. A left-out item has
  no chip, hover or accessible node, and a left-out CPU, RAM or clock is never
  read and never wakes Venus. Accessibility order follows the visual order. The
  items use only spare drag-region space and keep its minimum. Whatever the
  order, narrow windows hide the clock first, then RAM, CPU and the version,
  then quota chips, before tabs or controls. No item has an action or a keyboard
  focus target, and none drags the window. A change in quota, a reading or the
  clock never moves the tabs, cancels scrolling or resizes terminals.

### Panes and popups

A tab shows one to four equal-width columns side by side. In each column one
pane expands in a vertical stack while every fitting pane header stays
visible. Each pane is its own rounded card, stacked without gaps in its
column's allocated card area. Even outer gaps surround and separate the
allocated column areas. Collapsed panes show a rounded title edge with corners turning toward the
expanded pane and sides continuing to the next card in that direction. The
expanded pane has a full rounded frame with its title in the top edge.
`border_width` sizes neutral outlines; `focus_border_width` sizes the selected
pane's outline. The visible shortcut footer sits bare in the margin below
the allocated column areas.
Selection, hover, and keyboard focus have distinct fills or outlines.

With pane frames on, each column has one continuous terminal-background fill,
from its first pane's rounded top edge down to the column's rounded bottom.
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
Stacked title rows and the space between cards share the expanded terminal's
background colour and opacity, except that extended top padding fills an
unzoomed expanded pane's band below its title (`padding_color`). Above that edge
and outside the outer corners, the gaps and footer stay the opaque bar colour.
The first title sits on that boundary. Popup and Agents fills follow their
rounded outer edges in the same way, with their titles on the boundary. Header
scrolling keeps the fill's top fixed. Frameless mode keeps terminal-background
strips across the window below the bar, including the sidebar and footer.

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

A fresh workspace or tab can begin with a Project popup and no pane. Tool popups
and the chooser draw as one card over every column, with Eon's
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
is opaque. Explicit cell backgrounds, selection, inverse video, notices, focus
outlines, text and the cursor keep their own colours, and the Eon Bar stays the
opaque bar colour. With pane frames on, so do the gaps and footer outside the
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

## Ownership

```text
Eon                     -> product policy, composition, distribution
Eon Runtime             -> workspace mechanisms, canonical EONW
Venus                   -> native presentation, interaction, client failure UX
Orbit                   -> PTYs, terminal state, session lifetime, wire authority
```

Venus consumes EONW v25 through the in-tree `eon-workspace-protocol` 0.1.0
crate. Eon supplies popup definitions,
commands and geometry defaults. The runtime validates configuration and owns
live workspace and popup state, Session mappings, and provider quota collection
and normalization. Venus consumes
the in-tree `orbit-protocol` 0.1.0 crate with ORBF v3 and ORBS v14. One reducer turns complete canonical
frames into immutable scene data used by drawing and accessibility. The native
host owns the local socket, window, input mapping, and redraw lifecycle; it owns
no terminal state.

Each Eon workspace pane references an independent Orbit Session. Venus keeps
the EONW connection for workspace state/actions and one presentation connection
to the selected popup or each column's expanded pane. Runtime owns read-only
metadata observers for panes in every tab and workspace and supplies semantic
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
