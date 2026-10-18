# Changelog

## Unreleased

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
  margin below the cards. Column width and the 40-column admission check use
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
