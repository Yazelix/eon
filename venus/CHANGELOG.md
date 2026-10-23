# Changelog

## Unreleased

- VEN-C5 and VEN-C8: a link success notice ("Link handed to the desktop
  handler." or "Link copied.") clears on its own after about 2 seconds;
  failure notices still wait for input. No notice disables link hover, open or
  copy. The status line shows the newer of a workspace notice and a link
  result, and a hovered link's hint shows over a workspace notice, which
  returns when the pointer leaves (`eon-link-notice-lifetime-454q`).

- VEN-C21: the working badge, in Eon Bar tabs and VEN-C30 sidebar rows, is
  an antialiased open arc of 270° that turns clockwise in 45° steps. As
  before, it repaints at most 5 times a second, and only the badge. It
  replaces the segmented ring. On a software (CPU) adapter it is the same
  arc, drawn without turning (`eon-arc-spinner-t6li`).

- VEN-C30: Venus shows Eon's `no-attention` rejection of Alt+G or Alt+Shift+G
  as the bare VEN-C8 notice "No agent needs attention", without taking focus.
  Unlike other rejected-action notices, it also clears when a terminal's
  attention state changes and on the next input Venus sends to the terminal,
  as the clipboard notice does (`eon-empty-attention-notice-opy3`).

- VEN-C30: within a tab, a sidebar row whose source is a popup comes first,
  above the pane rows, which keep pane order. It carries the popup marker, a
  small window glyph drawn from quads, after its catalog label, whether or not
  the popup is shown, and its accessible name adds "popup". When a listed tab
  shows a popup (its `selected_popup` is set), its heading ends with the same
  glyph and its accessible name adds "popup shown" (`eon-agent-sidebar-yx8.16`,
  `eon-popup-marker-always-52rk`).

- VEN-C21: on a software (CPU) wgpu adapter the working badge, in Eon Bar
  tabs and VEN-C30 sidebar rows, is a still ring with every segment lit, and
  no spinner tick or redraw is scheduled. Other adapters keep the 5 Hz
  spinner (`eon-agent-sidebar-yx8.15`).

- VEN-C21 and EON-C21: while the agent sidebar is not visible, each Eon Bar
  tab whose EON-C31 rollup is needs input, error, finished or working shows a
  corner badge (◆, ✕, ✓ or a spinner). The badge is dimmed when every terminal
  in that state is stale, and it changes neither tab width nor label. The
  spinner turns at most 5 times a second by rewriting only its own vertices.
  Each tab-overflow edge shows the most urgent badge hidden past it, and the
  tab's accessible name appends its state (`eon-agent-sidebar-yx8.2`).

- VEN-C30 (Partially proved): the agent sidebar. While the snapshot's
  `sidebar` is `Open` and the window keeps at least 80 cells right of it, a
  28-cell sidebar sits below the Eon Bar and the cards share the content area
  to its right (VEN-C8 side gaps and CreateColumn check, VEN-C18 popup card
  area). Alt+Shift+H sends `ToggleSidebar`; it is unbound, and missing from
  the viewer and hints, under `Off` (VEN-C20, VEN-C24). The sidebar is an
  AccessKit region between the header and the panes, with an empty state when
  no tab has attention (`eon-agent-sidebar-yx8.3`). Each tab with attention
  gets a heading and one row per terminal with attention: state glyph, the
  VEN-C26 splat or VEN-C22 mark or the `app` verbatim, title or directory
  leaf, age from `changed_at`, source `pN` or popup label, a needs-input `msg`
  line, "+N" and a dimmed "status stale". A click, or Enter on the row F6 and
  Up/Down focus, sends `FocusAttention`; focus follows the result, and returns
  to the terminal when the focused row or the sidebar goes away. Rows are
  AccessKit buttons under tab headings. The card takes the column cards'
  outline, and the leftmost column's terminal background at the
  terminal-background opacity, which VEN-C11 now covers
  (`eon-agent-sidebar-yx8.13`). Alt+G and Alt+Shift+G send `NextAttention`
  and `PreviousAttention` under every `sidebar`, `Off` included, and move no
  keyboard focus themselves; a configured popup on either key takes it. The
  viewer lists them as "Jump to the next agent needing attention" and "Jump to
  the previous agent needing attention", and the hints as "next agent" and
  "previous agent" (VEN-C20, VEN-C24, `eon-agent-sidebar-yx8.8`).

- Venus consumes EONW v19, whose attention carries `name`, the terminal
  title. A sidebar row's first line shows one label: for `app=claude-code`
  the name, which is the session's name, else the record title, else the
  app's name (`Claude`, `Codex`, or the `app` verbatim). Other apps'
  terminal titles, such as Codex's spinner title, never lead a row, and the
  row no longer falls back to the tab's directory leaf, which the tab's
  heading shows. The row's accessible name includes Claude Code's session
  name (VEN-C30, `eon-agent-sidebar-yx8.11`).

- EONW v18 (now v19, above) adds EON-C31 agent attention to panes,
  popups and tabs, the window's sidebar visibility, and the `ToggleSidebar`,
  `FocusAttention`, `NextAttention` and `PreviousAttention` actions; the
  VEN-C21 and VEN-C30 entries above render and send them. VEN-C8 and VEN-C22: a
  snapshot that differs only in attention or sidebar visibility keeps a
  rejected-action notice and disturbs no navigation, scroll or attachment
  (`eon-agent-sidebar-yx8.7`).
- VEN-C8: a final close from Alt+Shift+W, the Eon Bar's Close tab or Alt+W
  first shows a native "Close the last tab?" alert dialog listing the tab's
  panes and popups; Enter sends the close and Escape dismisses it. The new
  `--warning-close-last-tab true|false` launch argument, true by default,
  turns it off. VEN-C14: Alt+Shift+D detaches like a compositor close request.
  VEN-C20/VEN-C24: Shortcuts and the footer list Alt+Shift+D
  (`eon-final-close-prompt-and-detach-key-m798`).
- VEN-C3, VEN-C4: when Venus's bounded Orbit event queue is full, Venus stops
  reading from Orbit until its event loop drains the queue, in place of
  dropping the attachment with `Orbit event queue exceeded its bounded
  capacity`; Orbit's slow-reader handling keeps the newest frame
  (`eon-10qt`).
- VEN-C5: the link highlight and hint are derived from the frame being
  presented, so output no longer blinks them while the pointer rests on a
  link, and a `Ctrl+click` press survives output revisions that keep the same
  URI over the same cells, in place of every revision retiring both
  (`eon-1u9e`).
- Venus consumes EONW v17, whose `Ended` reply answers a close that stopped
  the window's last terminal; Venus exits on it as on control loss
  (`eon-final-tab-close-ends-window-fwnp`).
- VEN-C8: Alt+Shift+Z toggles window-local zoom of the selected column across
  the tab's cards; hidden columns stay attached at their column size, and a
  change of the active tab, column count or selected column ends the zoom
  (`eon-zoom-selected-pane-e2pv`).
- VEN-C8: Ctrl+Alt+M sends `CreateColumn`, with the same 40-terminal-column
  refusal, in place of Alt+Shift+M (`eon-0ka6`).
- VEN-C26: Claude gets one read-only chip per reported window, five-hour then
  seven-day, each with its own label, hover and accessible node; under width
  pressure, after compaction, quota chips disappear starting with the one
  closest to the tabs, in place of the trailing one
  (`eon-claude-chip-split-u3ik`).
- VEN-C2: when Shift is the only held modifier and Space carries other text,
  such as `"` composed from a dead key under `us+intl`, Venus reports Shift as
  consumed, so a program using the kitty keyboard protocol without report-all
  receives the text rather than Shift+Space; Linux dead keys and compose
  sequences from the locale's compose table are accepted (`eon-e2vl`).
- VEN-C28: the CPU and RAM labels put one space between name and value
  (`CPU 12%`) and center it in a chip that keeps the width of `CPU 100%`, in
  place of padding the value to four cells (`eon-igae.14.4`).
- VEN-C29: an optional workspace-only `--bar-items` launch input sets the
  order of the Eon Bar's quota chips, CPU and RAM labels, clock and version
  label, and leaves out the items it does not name; Venus does not read or
  wake for a left-out reading. Without it the bar is unchanged
  (`eon-igae.14.1`).
- VEN-C5: Venus detects `http://` and `https://` URLs printed as plain text
  in visible rows, joined across soft wraps and trimmed of trailing
  punctuation and unmatched closing brackets, and gives the ones its opener
  accepts the same hover, preview, Open, Copy and accessibility actions as
  OSC 8 links; an OSC 8 link on any of their cells wins (`eon-nlcy`).
- VEN-C2: a named key the active keymap produces (Escape, Return, Tab, arrows,
  function and modifier keys) outside the numpad replaces the physical key for
  terminal input and Venus's own key handling; letter, digit, punctuation and
  numpad keys stay physical (`eon-venus-keysym-named-keys-chnh`).
- Venus consumes EONW v16, which adds a focus flag to both pane-create
  requests; Venus always sends a focused pane (`eon-zhjj`).
- Venus consumes EONW v15, which adds a command-pane request that Venus never
  sends (`eon-n6ga`).
- Venus consumes EONW v14, which adds a file-edit request that Venus never
  sends (`eon-ecne.2`).
- Venus consumes EONW v13, whose requests may name a calling terminal; Venus
  sends none, so its actions keep targeting the active tab (`eon-8mnr.2`).
- VEN-C28, VEN-C27, VEN-C25: a read-only `RAM 43%` label, from
  `/proc/meminfo` on the CPU label's 2-second wake, joins the Eon Bar after
  the CPU label and yields right after the clock. The CPU and RAM labels and
  the clock take the quota chips' rounded fill, and the version label moves
  to the edge before Shortcuts and Close (`eon-igae.3`).
- VEN-C28: a read-only `CPU 12%` label joins the Eon Bar before the clock,
  sampled from `/proc/stat` every 2 seconds on Linux. It uses only spare space
  and yields right after the clock (`eon-cpu-widget-eon-bar-acxs`).
- VEN-C5, VEN-C20, VEN-C21, VEN-C23, VEN-C24: a current Eon popup on the key of
  a fixed binding other than Alt+Slash and Alt+0 through Alt+9 takes that key;
  the footer omits the fixed hint, Shortcuts lists it as overridden by the
  popup, and header buttons and the link hint stop advertising it
  (`eon-igae.10`).
- VEN-C27: a read-only local 24-hour clock joins the Eon Bar, before Shortcuts
  and Close. It uses only spare space, yields before every other header item,
  and adds the jiff dependency for the local time zone (`eon-igae.4`).
- Venus consumes EONW v12. VEN-C8: Ctrl+Alt+H/L sends one non-repeating
  `MovePaneSideways` for the selected pane, applying Alt+Shift+M's 40-column
  refusal when the move would open a column; moving the active tab moves to
  Ctrl+Alt+Shift+H/L (`eon-sideways-pane-moves-k9ca`).
- Venus consumes EONW v11. VEN-C8: Alt+W sends one non-repeating `ClosePane`
  naming the active tab's selected pane and is consumed without an action when
  the tab has none (`eon-close-pane-onwi`).
- Venus consumes EONW v10, whose tabs hold up to four pane columns.
- VEN-C8: Venus lays out the active tab's columns side by side, each with its
  own headers and a live Orbit attachment sized to that column. Keyboard
  input, the cursor trail, selection, links and status follow the selected
  column; a click on another column's terminal selects it. Alt+Shift+M sends
  EONW v10 `CreateColumn`; in a tab with one to three columns, Venus refuses it
  locally when a column would get fewer than 40 terminal columns (`eon-splits-opwv` slices 2–4).
- VEN-C8, VEN-C18, VEN-C24: pane frames draw each column as its own rounded
  card, with one even gap (8 logical pixels at default typography, also the
  corner radius) from the window's sides, between cards and below the tab
  pills; popups draw as one card, and the footer's hints sit bare in the
  margin below the cards. Everything outside the cards takes the header's
  opaque color, and each card fills with its terminal's background
  (`eon-tubj`). Tab pills keep the card gap from the window's top and left
  edges, so the header grows by 8 pixels (`eon-gv1r`). Column width and the 40-column admission check use
  the card geometry, and initial grids include the gaps
  (`eon-gapped-rounded-cards-txco`; it replaces the unreleased flush workspace
  sheet of `eon-flush-workspace-sheet-5779` and its follow-ups).
- VEN-C1: an omitted or `random` cursor-tail color is chosen once per live
  endpoint, from the device, inode and change time of the workspace or Orbit
  socket, instead of once per launch, so a reattached window keeps its color.
  A socket absent at launch still gets a per-process choice
  (`eon-cursor-color-after-attach-lbbo`).
- VEN-C26: the Claude quota chip shows each window's elapsed position, such as
  `3d2h/7d 40%`, like the Codex chip. Hover and accessibility say how far into
  each window the observation is (`eon-claude-chip-position-iubd`).

## Before this repository

Venus was developed as Eon Desktop until 2026-10-14; that history is kept in a
private archive. It arrives here as the native Linux Wayland and Vulkan client
for ORBS v13 and EONW v9: authoritative terminal presentation, semantic input,
scrollback, selection, paste and hyperlinks, transient recovery and bounded
failure UX, the Eon workspace with tabs, accordion panes, shared popups and the
picker, the Eon Bar with version label and Codex and Claude quota chips, the
shortcut viewer and bottom hints, startup typography, opacity and compositor
blur, AccessKit accessibility, and a partial Apple Silicon macOS foundation.
