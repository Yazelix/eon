# Decision changelog

Product and design decisions Lucca asked to keep here, newest first. An entry
is added only when he explicitly asks for a decision to be saved in this
changelog; other decisions live in their beads and contracts. Each entry
records the decision, why, and the alternatives it rejected.

Each entry has a unique `EON-D<number>` index, the next unused number. Never
renumber or reuse an index; a later entry that replaces a decision names the
index it supersedes.

## EON-D1 — Popups keep a neutral outline

**Date:** 2026-10-26

**Decision:** A shown popup keeps its neutral card outline, the agent
sidebar's outline color. It gets no focus outline, and no popup marker in its
title or in the tab pill.

**Why:** A popup is one neutral card over the columns, with no focus-outlined
pane inside it; that is what tells it apart from panes. With pane frames on,
the focus color marks the selected pane in the selected column. A shown popup
takes that pane's place for typing but, like the agent sidebar, is the only
card in its area, and its neutral outline reads as "this whole area is the
popup". The focus outline would remove that cue, so it would need another way
to tell popups apart, and each option below costs more than it gains.

**Rejected:**

- Focus outline like a pane: removes the neutral outline that identifies
  popups.
- Focus outline plus a popup marker in the tab pill, like zoom's magnifier:
  two changes to recover a distinction the outline already gives. The marker
  spends tab label width, as the magnifier does, and needs a rule for live but
  hidden popups.
- Focus outline plus the popup marker before the popup's label: markers in the
  tabs, the sidebar and popup titles at once would read as clutter. The marker
  stays only in the agent sidebar, where it tells popup rows from pane rows.
- A darker shade of the focus color: `focus_color` is configurable, so a
  derived shade could be hard to tell from the focus color itself, and the
  shade and the focus color never appear side by side while a popup covers
  the panes.

Bead: `eon-3bhr` (comment). Contract: VEN-C18 unchanged.
