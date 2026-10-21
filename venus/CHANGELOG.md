# Changelog

## Unreleased

- VEN-C5: Venus detects `http://` and `https://` URLs printed as plain text
  in visible rows, joined across soft wraps and trimmed of trailing
  punctuation and unmatched closing brackets, and gives the ones its opener
  accepts the same hover, preview, Open, Copy and accessibility actions as
  OSC 8 links; an OSC 8 link on any of their cells wins (`eon-nlcy`).
- VEN-C2: a named key the active keymap produces (Escape, Return, Tab, arrows,
  function and modifier keys) outside the numpad replaces the physical key for
  terminal input and Venus's own key handling; letter, digit, punctuation and
  numpad keys stay physical (`eon-venus-keysym-named-keys-chnh`).
- Venus consumes EONW v14, which adds a file-edit request that Venus never
  sends (`eon-ecne.2`).
- Venus consumes EONW v13, whose requests may name a calling terminal; Venus
  sends none, so its actions keep targeting the active tab (`eon-8mnr.2`).
- VEN-C28, VEN-C27, VEN-C25: a read-only `RAM  43%` label, from
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
