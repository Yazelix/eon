# Presentation and ownership

## Native window and workspace

Venus presents one native Wayland window on Linux. It can show one local Orbit
Session or an Eon workspace. In workspace mode Eon supplies the tabs, pane
order, selection, and popups; Venus draws the visible scene.

### Eon Bar

- Tabs show their one-based position and the directory leaf or `/` from Eon's tab
  launch directory. Long labels use middle ellipsis; hover shows the full path.
  Selection brightens the tab, and keyboard focus adds an outline.
- New tab sits beside the last tab and pins to the tab-region edge when tabs
  overflow. Shortcut help and Close stay at the right. A small empty area lets
  users drag an undecorated window without taking clicks from tabs or controls.
- When Eon supplies Codex quota facts, a read-only chip shows the OpenAI Blossom,
  elapsed/total window positions, and remaining percentages. It condenses to
  the lowest-remaining window, then disappears before tabs or controls lose
  space. Stale, blocked, and unknown states remain explicit; hover gives human
  time.
- Claude facts show one chip per window, five-hour then seven-day, each with
  the Claude splat, its elapsed/total window position and remaining
  percentage, like Codex; a window without a reset shows no position. Hover
  and accessibility name Claude and that window, and say how far into it the
  observation is, or that its reset time is unavailable. Chips condense, then
  disappear one by one, the one closest to the tabs first, before tabs,
  controls or dragging lose space. `old` stays visible for stale facts. Missing facts and transport loss remove the affected chips.

### Panes and popups

A tab shows one to four equal-width columns side by side. In each column one
pane expands in a vertical stack while every fitting pane header stays
visible. Each column is its own rounded card, with an even gap around and
between the cards. The visible shortcut footer sits bare in the margin below
them.
Selection, hover, and keyboard focus have distinct fills or outlines.
Visible headers pair Eon's opaque pane
identity with Orbit's working directory. Home displays `~`, and its descendants
use `~/`; without `HOME`, paths stay absolute. Every column's expanded pane
presents live, unless Alt+Shift+Z zooms the selected column across the tab;
only the selected pane receives input. Ctrl+Alt+M adds
another column unless one would get fewer than 40 terminal columns.

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

## Ownership

```text
Eon                     -> product policy, composition, distribution
Eon Runtime             -> workspace mechanisms, canonical EONW
Venus                   -> native presentation, interaction, client failure UX
Orbit                   -> PTYs, terminal state, session lifetime, wire authority
```

Venus consumes EONW v15 through the in-tree `eon-workspace-protocol` 0.1.0
crate. Eon supplies popup definitions,
commands and geometry defaults. The runtime validates configuration and owns
live workspace and popup state, Session mappings, and provider quota collection
and normalization. Venus consumes
the in-tree `orbit-protocol` 0.1.0 crate with ORBF v2 and ORBS v13. One reducer turns complete canonical
frames into immutable scene data used by drawing and accessibility. The native
host owns the local socket, window, input mapping, and redraw lifecycle; it owns
no terminal state.

Each Eon workspace pane references an independent Orbit Session. Venus keeps
the EONW connection for workspace state
and actions, one presentation connection to the selected popup or to each
column's expanded pane, and one read-only
metadata observer for each visible live pane. Sessions in hidden tabs and offline
Sessions have no Venus observer and remain alive independently; the columns a
zoom hides keep their observers and connections.

Terminal accessibility text and selection come from the same accepted Scene.
Atomic units retain their terminal columns, and AccessKit projects their
positions and widths through the rendered grid and visible clipping. Scoped
Linux Orca line-review evidence predates this repository and awaits re-proof
under VEN-C1; complete composed screen-reader qualification and native reader-key delivery remain separate.

## Platform and limits

This slice excludes arbitrary split trees, vertical splits, column resizing,
sidebars, popup command/lifetime policy, persistent Venus configuration,
background images, plugins, remote or web access, packaging, and distribution.
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
