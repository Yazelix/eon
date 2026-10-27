# Orbit contract index

This is the canonical index of Orbit's accepted behavioral contracts,
ownership boundaries, proof status, and remaining gaps. `AGENTS.md` governs how
agents work, Beads govern authorized changes, and this index records what the
system must preserve.

Orbit builds `libghostty-vt` and `libghostty-vt-sys` 0.2.1 from the
upstream `Uzaaft/libghostty-rs` commit that [`CRATES.md`](CRATES.md) records. The
native build explicitly passes `-Dcpu=baseline` by default, building Ghostty
`521cf2c5d30bd48028ca61546c64e7913764073d`, with ORBF v3 / ORBS v14. This
preserves the headless terminal contract checks; it does not establish
older-CPU compatibility for a published cache artifact.

## Status

- **Planned:** accepted contract with no sufficient implementation evidence
- **Partially proved:** useful evidence exists, but a required behavior,
  integration boundary, review, or decision remains open
- **Proved:** the owning Bead is accepted and the listed immutable revision and
  checks cover the contract
- **Retired:** the user explicitly replaced or removed the contract; its ID is
  never reused

Proofs cite revisions of this repository. "Pending re-proof in this
repository" marks an earlier proof that was not repeated after consolidation;
it is re-proved when that platform or feature is next touched.

Canonical ORBF v3 carries two little-endian `u64` fields after cursor metadata
and before rows: `ScrollPosition.rows_from_live` and
`ScrollPosition.history_rows`. The latter excludes the active viewport; the
former cannot exceed it and is zero at live bottom. Both are zero on the
alternate screen. Counts describe current wrapped display rows, not stable
logical lines, and are extracted with each complete frame through the existing
synchronized publication owner. Output, scrolling, reflow, pruning,
clear/reset, screen switches and reattachment therefore carry position at that
frame's revision. Failed engine extraction does not fabricate zero or publish a
partial frame. The 100,000-cell bound and the 4 MiB frame budget for the
header, rows, cells and links remain; ORBF v3's graphics block lies outside
that budget under its own bound (ORB-C16).

ORBS v7 through v14 are explicitly breaking, exact-version boundaries with no
compatibility window or adapter; each replaced its predecessor. Orbit's
real-PTY checks and lifecycle tests, Venus and Eon consume the canonical crate
in this repository. Each contract below states the current ORBF v3 in ORBS v14
behavior, including ORB-C16, whose values ORBF v3 and ORBS v14 carry. Its
PTY pixel size rule needs no wire change.

## ORB-C1 — Session survival across client loss

- **Status:** Proved
- **Consumer:** One local graphical client attached to a running Orbit Session.
- **Trigger:** The client disconnects and a later client attaches.
- **Result:**
  - The PTY and foreground process remain live while Orbit remains running.
  - The later client reaches the same live Session.
- **Important failures:** Transient Linux PTY close/reopen, portable PTY EOF,
  partial final writes, and simultaneous stale socket claims recover or fail
  explicitly without losing the reachable owner.
- **Owner:** Orbit's Bend session policy, which decides attachment and the
  lifecycle, and its Rust host, which owns the PTY child and its mechanics.
- **Boundary:** Persistence across Orbit or machine restart is excluded.
- **Proof:** the `eon-awmq.3` import commit
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified);
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` of the previous Rust server at
    `4910b81e9d68942a65ad96ad21e03385de7144f5` (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`transient_pty_eio_recovers_when_the_live_child_reopens_the_terminal`](../tests/lifecycle.rs)
    - [`simultaneous_stale_socket_claim_has_one_reachable_owner`](../tests/lifecycle.rs)
    - [`shell_survives_detach_and_one_client_reattaches`](../tests/lifecycle.rs)
    - Real-PTY checks [`orbit-three-attach`](../tests/real_pty/three_attach.rs),
      [`orbit-pty-reopen`](../tests/real_pty/pty_reopen.rs) and
      [`orbit-stale-socket`](../tests/real_pty/stale_socket.rs)
    - Laws [`detach_preserves_pid`](../LAWS.bend),
      [`reconnect_preserves_pid`](../LAWS.bend) and
      [`lifecycle_follows_its_contracts`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite
    - On macOS the stale-socket check passed for the previous Rust server; the
      transient PTY EIO check is Linux-only.

## ORB-C2 — Sole terminal-state authority

- **Status:** Proved
- **Consumer:** Every Orbit runtime and attached client.
- **Trigger:** PTY bytes, client interaction, or terminal-generated responses
  change terminal state.
- **Result:** Orbit alone owns libghostty terminal state and alone sends
  terminal-generated responses to the PTY.
- **Important failures:** Client failure cannot duplicate a terminal response or
  transfer parser authority.
- **Owner:** Orbit's Rust host, the sole holder of libghostty terminal state.
- **Boundary:** Ordered client-visible effects beyond the current transport
  contract remain unproved.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.1` commit
  for the palette reset
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`palettes_are_exactly_sixteen_hex_colors`](../src/cli.rs)
    - [`ansi_palette_launch_is_visible_and_survives_reattachment`](../tests/lifecycle.rs)
    - [`orbit-command-line`](../tests/real_pty/command_line.rs)
    - [`conformance_c2_parser_state_and_terminal_replies_survive_client_failure`](../tests/lifecycle.rs)
    - [`orbit-terminal-replies`](../tests/real_pty/terminal_replies.rs)
    - [`orbit-palette-reset`](../tests/real_pty/palette_reset.rs)
    - Workspace suite

## ORB-C3 — Bounded interactive and metadata attachment

- **Status:** Proved
- **Consumer:** One local input-capable client and one local read-only metadata
  observer.
- **Trigger:** A peer negotiates an explicit role on the private presentation
  endpoint.
- **Result:**
  - One client may own input while one observer receives only initial and changed
    terminal-authored title, raw working directory, presentation revision, and
    lifecycle.
  - The observer cannot send terminal, presentation, process, or lifecycle
    mutations.
- **Important failures:**
  - Excess roles, incompatible or malformed peers, slow readers, disconnect,
    and exit fail deterministically without disturbing the PTY, child, other
    role, or cleanup.
  - Invalid UTF-8 terminal metadata, such as a title libghostty cuts inside a
    character or a non-UTF-8 OSC 7 directory, reaches the client and observer
    as empty and never ends the terminal.
- **Owner:** Orbit's Bend admission policy and Rust socket transport with
  canonical `orbit-protocol` ORBS v7.
- **Consumes:** Canonical ORBS v7 behavior, preserved through ORBS v14; Venus
  and Eon consume it in this repository.
- **Boundary:** Visible text observation, agent observation, multiple observers,
  multiple interactive clients, remote access, and process authority are
  excluded.
- **Proof:** the `eon-awmq.3` import commit, with the
  `eon-title-utf8-ends-terminal-atq2` commit for invalid metadata
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`metadata_observer_is_canonical_read_only_and_reports_changes`](../src/socket.rs)
    - [`nonreading_observer_converges_on_the_latest_metadata`](../src/socket.rs)
    - [`metadata_observer_streams_inactive_title_and_cwd_without_owning_input`](../tests/lifecycle.rs)
    - Real-PTY checks [`orbit-slow-observer`](../tests/real_pty/slow_observer.rs),
      [`orbit-late-observer`](../tests/real_pty/late_observer.rs),
      [`orbit-detached-observer`](../tests/real_pty/detached_observer.rs),
      [`orbit-unsupported`](../tests/real_pty/unsupported.rs) and
      [`orbit-silent-peer`](../tests/real_pty/silent_peer.rs)
    - Law [`admission_answers_are_accounted`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite

## ORB-C4 — Coherent ordered presentation

- **Status:** Proved
- **Consumer:** One attached exact-version presentation client.
- **Trigger:** Attachment begins or authoritative terminal state changes.
- **Result:** Attachment starts with one complete frame at revision N and then
  receives strictly ordered later complete-frame revisions with no stale final
  presentation.
- **Important failures:** Split or synchronized output and a slow reader cannot
  publish partial state or violate final convergence.
- **Owner:** Orbit's Bend session policy, which decides each publication and
  its revision, and its Rust host's frame extraction and socket transport, with
  canonical session and complete-frame codecs.
- **Consumes:** Canonical ORBF v3 in ORBS v14; Venus and Eon consume it in this
  repository.
- **Boundary:** Native presentation quality remains Venus-owned.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.2` commit
  for held attachment and exact reattachment
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`orbit-synchronized-output`](../tests/real_pty/synchronized_output.rs)
      holds a split update, bounds it with the watchdog, and releases it at
      child exit.
    - [`orbit-three-attach`](../tests/real_pty/three_attach.rs)
    - [`orbit-held-attach`](../tests/real_pty/held_attach.rs) attaches during
      a held update, whose first frame waits for the release or the watchdog,
      and reattaches to a rich alternate screen with an identical frame.
    - [`output_queue_replaces_unstarted_presentations_within_its_bound`](../src/socket.rs)
    - Laws [`revision_next_is_ahead`](../LAWS.bend) and
      [`scrolling_and_selection_follow_their_contracts`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite

## ORB-C5 — Semantic terminal interaction

- **Status:** Proved
- **Consumer:** Attached clients sending key, mouse, focus, paste, or resize
  interaction.
- **Trigger:** Orbit accepts one canonical semantic input message or begins one
  revision-bound left-pointer sequence.
- **Result:**
  - Orbit encodes terminal interaction against its authoritative terminal
    state.
  - When mouse tracking is active and Shift is absent, Orbit pins the
    left-pointer sequence to terminal input and encodes each positional phase
    once through its existing terminal-aware mouse encoder.
  - Otherwise Orbit pins the sequence to host selection; Shift always selects
    host text.
- **Important failures:** Invalid or terminal-forbidden input fails without
  fallback text or duplicate insertion. Invalid phase order, pressure, cancel,
  endpoint loss, or a terminal-mode change cannot switch an active sequence,
  emit partial input, or synthesize terminal input.
- **Owner:** Canonical platform-neutral `orbit-protocol` values, Orbit's Bend
  selection policy for the pointer route, and its Rust host's libghostty input
  encoding.
- **Consumes:** Canonical ORBS v9 semantic input, preserved through ORBS v14;
  Venus and Eon consume it in this repository.
- **Boundary:** Candidate-list IME and native input quality remain Venus-owned.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.1` commit
  for the Kitty release
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`canonical_input_uses_libghostty_encoding`](../src/input.rs)
    - [`kitty_release_never_falls_back_to_text`](../src/input.rs)
    - [`conformance_c5_authoritative_input_modes_and_resize_survive_detach`](../tests/lifecycle.rs)
    - Real-PTY checks [`orbit-selection`](../tests/real_pty/selection.rs),
      [`orbit-mouse`](../tests/real_pty/mouse.rs),
      [`orbit-key-live`](../tests/real_pty/key_live.rs) and
      [`orbit-paste-flood`](../tests/real_pty/paste_flood.rs)
    - Laws [`selection_answers_are_accounted`](../LAWS.bend) and
      [`selection_shift_selects_on_the_host`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite

## ORB-C6 — Rich presentation without silent degradation

- **Status:** Proved
- **Consumer:** Every canonical presentation client.
- **Trigger:** Orbit extracts or encodes terminal presentation state.
- **Result:** The boundary carries rich terminal state or explicitly declares an
  unsupported capability; it never silently collapses to a plain-text grid.
- **Important failures:**
  - Malformed cell-width topology fails at every admission boundary.
  - A hyperlink URI that is not valid UTF-8 reaches the client as no link: the
    cell keeps its text, in frames and previews, and the terminal keeps
    running.
  - When a frame would pass the frame budget, every cell keeps its first
    codepoint; full cell text and then hyperlinks are kept in reading order
    while they fit; the rest reach the client as their first codepoint and as
    no link, and the terminal keeps running.
- **Owner:** Orbit's authoritative extraction and synchronized publication with
  canonical `orbit-protocol` values and codecs.
- **Consumes:** Canonical ORBF v3 in ORBS v14; Venus and Eon consume it in this
  repository.
- **Boundary:** A frame states `kitty_graphics` true exactly when it carries
  ORB-C16's graphics state; one that does not, such as every ORBF v2 frame,
  declares Kitty graphics unsupported. Kitty features outside ORB-C16 are its
  recorded gaps, and ORB-C16's Boundary names each, including those nothing
  marks.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.2` commit
  for direct rows, width roles, protection and reflow, the
  `eon-osc8-invalid-utf8-ymts` commit for invalid hyperlink URIs, and the
  `eon-frame-budget-links-m62b` and `eon-frame-budget-text-kzze` commits for
  hyperlinks and cell text past the frame budget
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`terminal_frame_uses_the_canonical_protocol_owner`](../src/frame.rs)
    - [`vertical_preview_reads_history_nearest_first`](../src/frame.rs)
    - [`direct_rows_match_rich_canonical_frame_rows`](../src/frame.rs)
    - [`orbit-held-attach`](../tests/real_pty/held_attach.rs) reads bold, a
      hyperlink, wide and combining graphemes and the cursor from a real PTY's
      rich alternate screen, and reattaches to an identical frame.
    - [`orbf_v2_cell_width_topology_is_validated_at_every_acceptance_boundary`](../crates/protocol/src/lib.rs)
    - [`orbit-late-observer`](../tests/real_pty/late_observer.rs) prints an
      OSC 8 link whose URI is not UTF-8 and reads its text without the link,
      with the terminal running; `vertical_preview_reads_history_nearest_first`
      reads the same in a preview row.
    - [`hyperlinks_over_the_frame_budget_keep_the_frame`](../src/frame.rs)
      reads a frame whose one 2 KB link over most of the screen exceeds the
      budget: every cell keeps its text and a reading-order prefix of the
      links.
    - [`cell_text_over_the_frame_budget_keeps_first_codepoints`](../src/frame.rs)
      reads cells with 64 combining marks against a budget their text
      exceeds: full text in reading order while it fits, the first codepoint
      in the rest, and links only after the text.
    - Canonical ORBF-in-ORBS codec checks in the workspace suite
- **Graphics proof:** Static graphics joined this contract on 2026-10-27 under
  Lucca's standing approval of 2026-10-26, relayed by eon-overseer
  (`orb-j2i.2`), and the proof above predates them. From `orb-j2i.11`, frames
  state `kitty_graphics` true, carry placement entries and blank placeholder
  cells, and Venus draws the images. ORB-C16's proof covers that: its real-PTY
  checks on Orbit's side, and `orb-j2i.12`'s installed proof of Yazi's
  previews.

## ORB-C7 — Bounded attachment pressure

- **Status:** Proved
- **Consumer:** Interactive attachments and metadata observers.
- **Trigger:** A client reads slowly, breaks, or disconnects while Orbit continues
  processing PTY state.
- **Result:** Authoritative PTY processing, the other role, and cleanup remain
  live with bounded output buffering and history. One partially transmitted
  maximum frame still leaves bounded capacity for one atomic maximum scroll
  outcome.
- **Important failures:** Pressure disconnects the failing client instead of
  blocking the owner loop or admitting a partial result.
- **Owner:** Orbit's Rust socket transport, with output bounds and pressure
  decisions from its Bend session policy.
- **Boundary:** The proof covers one local input client plus one metadata
  observer.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.2` commit
  for the worker's output bound
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`output_queue_replaces_unstarted_presentations_within_its_bound`](../src/socket.rs)
    - [`output_queue_keeps_scroll_outcomes_and_admits_them_before_moving`](../src/socket.rs)
    - [`pressured_interactive_client_is_reported_without_waiting`](../src/socket.rs)
    - [`worker_bound_admits_a_maximum_scroll_outcome_behind_a_started_maximum_frame`](../src/socket.rs),
      at the bound the built worker sends
    - [`observer_pressure_is_metadata_its_queue_cannot_take`](../src/socket.rs)
    - [`authoritative_viewport_survives_detach_and_slow_reader_pressure`](../tests/lifecycle.rs)
    - [`metadata_observer_streams_inactive_title_and_cwd_without_owning_input`](../tests/lifecycle.rs)
    - Real-PTY checks
      [`orbit-interactive-pressure`](../tests/real_pty/interactive_pressure.rs),
      [`orbit-slow-observer`](../tests/real_pty/slow_observer.rs),
      [`orbit-refused-flush`](../tests/real_pty/refused_flush.rs),
      [`orbit-lifecycle`](../tests/real_pty/lifecycle.rs) and
      [`orbit-scroll`](../tests/real_pty/scroll.rs)
    - Workspace suite

## ORB-C8 — Retained history and authoritative scrolling

- **Status:** Partially proved
- **Consumer:** One healthy exact-version attached client.
- **Trigger:** The client previews or commits relative vertical movement from a
  complete-frame revision it has presented, or requests an absolute return to
  live output, or ticks an active host selection at the top row; PTY output may
  advance Orbit before the request is handled.
- **Result:**
  - Each Session configures libghostty with a 16 MiB primary-history byte budget;
    retained rows are content-dependent and the active viewport may exceed it.
  - Every complete frame includes the bounded display-row position described
    above, including frames carried in wheel and signed-scroll outcomes.
  - Preview returns up to one active viewport of canonical rows, nearest first
    in the requested direction, without mutation, and names the current
    authoritative revision whose state it inspected.
  - Commit accepts a nonzero signed distance from -1,024 through 1,024 rows,
    resolves it against current authoritative terminal state, applies it once,
    clears active selection, advances authority once, and returns requested and
    applied rows, one authoritative frame, and the next bounded row window or
    edge.
  - During DEC 2026 synchronized output, Orbit holds at most one unresolved
    preview or commit until DEC 2026 ends or the existing one-second watchdog
    fires. Orbit then re-evaluates authoritative routing and resolves the
    request once without exposing partial state.
  - Negative movement goes toward older history, positive movement goes toward
    the live area, and movement clamps at history boundaries.
  - An explicit return-to-live action moves any retained primary viewport to
    zero in one authoritative complete frame without sending PTY input. It
    clears host selection and does not require a client-side row counter.
  - During an active primary-screen host selection, one ORBS v14 upward tick
    advances at most one row toward older retained history and returns its
    applied distance, complete frame, and next bounded preview without clearing
    the tracked gesture. Zero applied rows reports the oldest edge.
  - A successful semantic key that emits PTY bytes returns primary history to
    the live area; detach does not transfer, reconstruct, or reset viewport
    ownership.
- **Important failures:**
  - Terminal-owned mouse tracking or alternate-screen mode 1007 returns
    terminal-owned without PTY input, mutation, or revision advance.
  - Future authority, a second unresolved request, malformed or out-of-range
    distance, and pressure fail before mutation. Lagging relative preview,
    scroll, and selection revisions resolve against current authority; future
    revisions and unrelated coordinate- or phase-bound input remain strict.
  - Return-to-live rejects terminal-owned or alternate-screen routing; under
    synchronized output it waits for the existing release/watchdog boundary,
    and full client output pressure fails before viewport mutation.
  - An upward selection tick rejects absent or terminal-routed gestures,
    positions outside the top row, and alternate-screen state without PTY input
    or viewport mutation. Synchronized output holds at most one unresolved tick;
    bounded output pressure fails before movement.
- **Owner:** Orbit's Bend vertical policy and its Rust host's synchronized
  output handling, with libghostty's native viewport, byte-budgeted history,
  one canonical row extractor, and bounded output queue.
- **Consumes:** Canonical ORBF v3 in accepted ORBS v14; Venus and Eon consume
  it in this repository.
- **Boundary:** Arbitrary line-count guarantees, gesture cadence, downward
  selection autoscroll, velocity, kinetic effects, graphics, and restart
  persistence are excluded; physical wheels retain their existing typed
  terminal-routed or one-row behavior. This does not accept Eonova, or any
  ORBS v14 client on Apple Silicon (Venus or Eon); ORB-C14 covers Orbit's own
  native behavior.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.3` commit
  for scroll position, selection clearing and granular ticks
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - Real-PTY checks [`orbit-history`](../tests/real_pty/history.rs),
      [`orbit-preview`](../tests/real_pty/preview.rs),
      [`orbit-scroll`](../tests/real_pty/scroll.rs),
      [`orbit-return-to-live`](../tests/real_pty/return_to_live.rs),
      [`orbit-mouse`](../tests/real_pty/mouse.rs),
      [`orbit-key-live`](../tests/real_pty/key_live.rs) and
      [`orbit-selection-tick`](../tests/real_pty/selection_tick.rs)
    - [`orbit-selection-tick`](../tests/real_pty/selection_tick.rs) also
      finds a presented selection cleared by a commit toward live and by a
      return to live, with rows of it still in view.
    - [`vertical_preview_reads_history_nearest_first`](../src/frame.rs)
    - [`scroll_position_follows_reflow_eviction_and_clears`](../src/frame.rs)
      follows the position through output below a held viewport, the alternate
      screen, reflow, byte-budget eviction, `CSI 3 J` and reset.
    - [`upward_ticks_extend_cells_spans_and_lines_with_unicode_copy`](../src/pointer.rs)
    - [`output_queue_keeps_scroll_outcomes_and_admits_them_before_moving`](../src/socket.rs)
    - Laws [`vertical_answers_are_accounted`](../LAWS.bend),
      [`vertical_future_requests_fail`](../LAWS.bend),
      [`vertical_routed_requests_answer_at_once`](../LAWS.bend),
      [`vertical_nothing_resolves_while_held`](../LAWS.bend),
      [`vertical_release_resolves_waiting`](../LAWS.bend) and
      [`scrolling_and_selection_follow_their_contracts`](../LAWS.bend)
      (`orbit-laws`)
    - [`scroll_position_is_canonical_at_every_acceptance_boundary`](../crates/protocol/src/lib.rs)
    - [`vertical_scroll_batches_are_bounded_and_canonical`](../crates/protocol/src/session/tests.rs)
    - [`return_to_live_has_zero_payload`](../crates/protocol/src/session/tests.rs)
    - [`authoritative_viewport_survives_detach_and_slow_reader_pressure`](../tests/lifecycle.rs)
    - Workspace suite
- **Open proof:**
  - Pending re-proof in this repository: installed scrollback and
    ReturnToLive through Venus (`eon-monorepo-2oko.6` dogfood item 9 was not
    exercised).

## ORB-C9 — Authoritative bounded selection and copy

- **Status:** Partially proved
- **Consumer:** One healthy exact-version ORBS v14 attachment for upward ticks.
- **Trigger:** The client begins, updates, finishes, cancels, or copies one
  left-pointer sequence; a held host gesture may tick upward with a top-row
  surface position. Pointer phases carry current modifiers; Begin names a frame
  the client presented and a monotonic press timestamp.
- **Result:**
  - Orbit chooses host selection when authoritative terminal mouse tracking is
    absent or Shift is present, and keeps that route through Finish or Cancel.
  - Orbit alone resolves pointer positions, click repetition, cell, word,
    space-delimited span, and logical-line boundaries, selected presentation,
    and copied text through libghostty's tracked gesture APIs.
  - A Begin from an already-presented, non-future revision resolves once against
    current authoritative terminal state, so intervening PTY output cannot
    starve selection admission.
  - One press-drag selects cells, two select words using unchanged default word
    boundaries, three select space-delimited spans including punctuation, and
    four or more select logical lines. Drags extend at the corresponding
    granularity; spans and logical lines cross soft wraps. ASCII space/tab,
    unwritten cells, and hard line breaks delimit spans; other Unicode
    whitespace remains text. Clicking written space/tab selects its contiguous
    delimiter run, subject to the existing copy trimming. The repeat distance
    is one cell width and the repeat interval is 500 milliseconds.
  - Release freezes one bounded plain-text value that later output, resize,
    reflow, or screen transition cannot reinterpret or erase, and returns it
    for the semantic selection clipboard. Explicit Copy returns the same frozen
    text for the ordinary clipboard.
  - An accepted Finish returns one ordered typed completion naming the
    authoritative presentation revision at sequence completion. A client may
    send another press immediately when it already presented that revision;
    otherwise it waits for the exact frame.
  - Orbit discards wholly unsent presentation frames from earlier host drag
    phases before publishing the current phase, so release cannot replay an
    obsolete selection-frame backlog.
  - Compatible PTY output preserves an active host gesture, repeat-click
    classification, and libghostty-tracked selection as content moves, including
    into scrollback. Output that newly enables terminal mouse tracking or leaves
    no valid tracked gesture anchor, including on a different active screen,
    clears the active host selection and resets gesture state; read-only preview
    does not.
  - An explicit upward tick while the host gesture is held extends that same
    libghostty-tracked selection into the newly visible top row, including
    across multiple viewports, and release freezes the resulting exact text.
  - An ordered client cancel resets an abandoned gesture and clears its partial
    selected presentation without producing copied text.
  - A newer valid press replaces prior selected presentation and frozen text;
    client loss or exit clears them without resetting the ORB-C8 viewport.
- **Important failures:**
  - A backwards press timestamp safely starts a new single-click sequence.
  - Future revisions, invalid surface coordinates or phase order, formatting
    overflow, pressure, focus loss, or pointer capture loss reject or explicitly
    cancel the gesture without partial state, text, mixed routing, or invented
    terminal input.
  - Synchronized output may defer a host Finish frame or a terminal Begin frame
    that clears prior host selection; the completion preserves the exact held
    revision until presentation resumes or the attachment ends.
  - Selection work and buffering remain bounded and cannot block authoritative
    PTY processing or cleanup.
  - A partially written frame remains ordered and completes before a newer
    selection frame.
  - An upward tick with no valid host gesture or top-row position fails without
    movement or PTY input; an invalidated tracked anchor cancels the gesture.
- **Owner:** Orbit's Bend selection policy and its Rust host's pointer
  mechanism, with authoritative mouse modes, libghostty current-viewport
  gesture selection, and canonical ORBS v14. Venus owns native event delivery
  and clipboard effects.
- **Consumes:** Accepted ORBS v14 and ORB-C7 bounded-pressure behavior; Venus
  and Eon consume it in this repository.
- **Boundary:** Configurable word separators or click thresholds, block selection,
  downward autoscroll, cadence, semantic command-output selection, search,
  graphics, and restart persistence are excluded.
  Spans retain native word-selection limits: a wide-character trailing spacer
  can stop the selected span (`a;界,é` selects `a;界`). This inherited
  libghostty limitation is outside the click-ladder correction.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.3` commit
  for history, span delimiters, drag-back, highlighted cells and pressure
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`the_click_ladder_selects_cells_words_spans_and_lines_across_wraps`](../src/pointer.rs)
      checks soft wraps, compatible live output between press and release,
      fourth/fifth clicks, a late reset and exactly which cells are
      highlighted.
    - [`spans_end_only_at_ascii_space_tab_and_unwritten_cells`](../src/pointer.rs),
      [`a_drag_back_to_the_press_cell_clears_the_selection`](../src/pointer.rs)
      and [`a_held_gesture_survives_output_that_scrolls_it_into_history`](../src/pointer.rs)
    - [`orbit-selection`](../tests/real_pty/selection.rs) checks the route,
      Shift, Cancel, the click ladder's frozen text for both copy effects,
      Finish revisions, terminal-routed mouse bytes and client-scoped copy
      through canonical messages, and that the child's exit clears a
      presented selection.
    - [`orbit-selection-tick`](../tests/real_pty/selection_tick.rs) covers more
      than one viewport, synchronized deferral, oldest-edge stop, and exact
      release copy.
    - [`orbit-return-to-live`](../tests/real_pty/return_to_live.rs) refuses
      a host press for output pressure, leaving no sequence.
    - [`output_that_enables_mouse_tracking_unanchors_the_host_selection`](../src/pointer.rs)
    - [`output_queue_replaces_unstarted_presentations_within_its_bound`](../src/socket.rs)
    - Laws [`selection_answers_are_accounted`](../LAWS.bend),
      [`selection_shift_selects_on_the_host`](../LAWS.bend),
      [`selection_future_press_fails`](../LAWS.bend) and
      [`scrolling_and_selection_follow_their_contracts`](../LAWS.bend)
      (`orbit-laws`)
    - [`every_client_message_round_trips`](../crates/protocol/src/session/tests.rs)
      and [`malformed_typed_payloads_are_rejected`](../crates/protocol/src/session/tests.rs)
      cover the exact ORBS v13 action and invalid positions.
    - [`conformance_c9_selection_copy_is_authoritative_bounded_and_client_scoped`](../tests/lifecycle.rs)
    - Workspace suite
- **Open proof:**
  - Pending re-proof in this repository: the installed click selection
    ladder, drag selection and copy/paste through Venus
    (`eon-monorepo-2oko.6` dogfood item 9 was not exercised).

## ORB-C10 — Eon terminal identity

- **Status:** Proved
- **Consumer:** Every Orbit-owned PTY child and its launch environment.
- **Trigger:** Orbit launches a PTY child.
- **Result:** The child receives `TERM=eon` and `COLORTERM=truecolor`; the launch
  environment supplies the repository-owned compiled `eon` terminfo entry
  inheriting `xterm-256color`, so lookup and `clear` succeed.
- **Important failures:** Missing or invalid terminfo is exposed by lookup and
  real-PTY behavior checks rather than hidden by a fallback identity.
- **Owner:** Orbit's Rust host, which spawns the PTY child, and
  [`terminfo/eon.terminfo`](../terminfo/eon.terminfo); Eon installs the entry.
- **Boundary:** Other distribution formats remain outside this contract's proof.
- **Proof:** the `eon-awmq.3` import commit
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - `orbit-terminfo` checks that the Nix build's `tic`-compiled entry, with
      only the package's `share/terminfo`, resolves through `infocmp -x eon`
      and that `TERM=eon tput clear` emits the clear sequence it inherits from
      `xterm-256color`; without that directory `eon` is unknown.
    - [`orbit-terminal-identity`](../tests/real_pty/terminal_identity.rs)
      checks through a real PTY that the child sees `TERM=eon`,
      `COLORTERM=truecolor` and only the package's terminfo, and that `clear`
      exits 0 and blanks the next frame. The host sets `TERM` and `COLORTERM`
      when it spawns the child in [`src/pty.rs`](../src/pty.rs).

## ORB-C11 — Ordered terminal-authored clipboard writes

- **Status:** Proved
- **Consumer:** One healthy ORBS v7 attachment and its native clipboard owner.
- **Trigger:** The terminal emits one clipboard write with exactly one nonempty
  UTF-8 `text/plain` representation.
- **Result:** Orbit transports the normalized standard, selection, or primary
  destination once and in order before the complete frame from the same PTY
  turn; Orbit owns interpretation and transport while the client owns native
  policy and delivery.
- **Important failures:** Invalid UTF-8 or NUL text, clears, unsupported
  representations, payloads above 1 MiB, detached writes, and bounded pressure
  fail without retention, replay, or silent coalescing; pressure disconnects the
  client.
- **Owner:** Orbit's Rust host, which captures and transports the writes, and
  its Bend session policy, which orders and bounds them, with libghostty
  normalized effects and canonical ORBS v7.
- **Consumes:** Canonical ORBS v7.
- **Boundary:** Ordinary clipboard delivery, Wayland without data-control,
  broader compositors, and non-Linux policy remain unproved.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.4` commit
  for clipboard writes under output pressure
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified)
  - **Evidence:**
    - [`src/socket.rs`](../src/socket.rs) ordered bounded output checks
    - [`orbit-clipboard`](../tests/real_pty/clipboard.rs) checks order before
      the frame of the same read, a write during a held update, refused clear
      and detached writes, and an overflowing read that ends the attachment
      with no write and no replay.
    - [`clipboard_capture_is_attached_typed_and_bounded`](../src/pty.rs)
    - [`orbit-clipboard-pressure`](../tests/real_pty/clipboard_pressure.rs)
      fills a non-reading client's queue with writes, which ends the
      attachment; a new client receives none of them.
    - Workspace suite

## ORB-C12 — Bounded Session shutdown

- **Status:** Proved
- **Consumer:** Orbit lifecycle signals and canonical management Stop.
- **Trigger:** Orbit accepts SIGINT, SIGTERM, SIGHUP, or management Stop after
  launching one PTY Session.
- **Result:**
  - Orbit succeeds only after reaping its direct PTY child.
  - An already-exited child is reaped without signaling a recyclable PID.
  - Otherwise Orbit sends SIGHUP to the initial PTY process group and, when
    distinct and available, the terminal's current foreground process group,
    then gives the direct child up to 500 ms to exit.
  - If the child remains unreaped, Orbit SIGKILLs its still-stable initial group
    and rereads the current foreground group before escalation. If the child
    exits during grace, Orbit sends no later signal to a cached foreground-group
    number.
  - After escalation, management Stop succeeds once the direct child is reaped;
    a child not reaped within two seconds keeps the run and its live record
    until it is reaped, then Stop succeeds with its outcome. After a
    termination signal, Orbit requires the direct child to be reaped within two
    seconds.
- **Important failures:**
  - An already-gone signal target is benign; every other signaling or reaping
    failure is bounded non-success.
  - PID reuse, foreground-group changes, child exit during grace, and writing
    descendants cannot cause delayed signaling, and only a child the kernel
    never releases after a management Stop extends server lifetime.
  - Client disconnection remains the non-destructive ORB-C1 detach path.
- **Owner:** Orbit's Bend session policy decides the stop and its result; its
  Rust host carries out graceful timing, escalation, and direct-child reaping
  with its Unix PTY and process-group mechanics.
- **Boundary:**
  - Deliberately detached processes, non-foreground groups outside the initial
    group, and SIGHUP-ignoring foreground jobs whose shell exits during grace may
    survive.
  - Orbit scans no process names, ancestry, session IDs, or procfs and claims no
    whole-descendant cleanup. Direct-child HUP and reaping were proved natively
    on Apple Silicon macOS for the previous Rust server (ORB-C14); the special
    detached and foreground-group topology remains Linux-proved only.
- **Proof:** the `eon-awmq.3` import commit, with the
  `eon-orbit-stop-not-reaped-7vb4` commit for a management Stop whose child is
  reaped after escalation
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer,
    `tests/lifecycle.rs` unmodified);
    the previous Rust server installed as
    `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0` driven
    headlessly by a Kinestra recipe on a private headless Sway (wlroots, pixman);
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` of the previous Rust server at
    `4910b81e9d68942a65ad96ad21e03385de7144f5` (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`shutdown_hups_the_unreaped_direct_child`](../tests/lifecycle.rs)
    - [`shutdown_escalates_foreground_and_permits_a_detached_process`](../tests/lifecycle.rs)
    - [`stale_socket_and_safe_signal_shutdown`](../tests/lifecycle.rs)
    - [`writing_descendant_cannot_hold_server_open_after_known_child_exit`](../tests/lifecycle.rs)
    - Real-PTY checks
      [`orbit-guardian-containment`](../tests/real_pty/guardian_containment.rs),
      [`orbit-descendant-exit`](../tests/real_pty/descendant_exit.rs),
      [`orbit-observer-finish`](../tests/real_pty/observer_finish.rs),
      [`orbit-stopped-client-exit`](../tests/real_pty/stopped_client_exit.rs) and
      [`orbit-unreaped-stop`](../tests/real_pty/unreaped_stop.rs)
    - Laws [`bounded_stop_preserves_pid`](../LAWS.bend),
      [`pre_reap_events_preserve_stopped`](../LAWS.bend) and
      [`lifecycle_follows_its_contracts`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite
    - Eon consumes this shutdown boundary; `eon-monorepo-2oko.6` dogfood
      checklist item 12 stops the installed generation.
    - On macOS the direct-child HUP, safe signal shutdown and management Stop
      checks passed for the previous Rust server; the escalation and
      writing-descendant checks are Linux-only.

## ORB-C13 — Same-boot management ownership

- **Status:** Proved
- **Consumer:** One local Eon launcher and one later replacement on the same boot
  and login.
- **Trigger:** A launcher prepares an owned mode-0600 record, Orbit crosses Ready,
  the original supervisor disappears, or a replacement acquires management.
- **Result:**
  - The launcher may retain an empty exact owned mode-0600 record inode before
    spawn. Orbit serializes Ready by attempting one exclusive lock; a held claim
    stops startup without marking or publishing.
  - After winning, Orbit revalidates and marks the retained inode, then
    atomically replaces its pathname with the canonical live record.
  - Only a launcher winning the still-empty retained inode may use pre-Ready
    local-child rollback.
  - After Ready, supervisor loss does not stop or hold open the Orbit run.
  - One replacement may acquire the sole private management lease only after a
    live handshake matches:
    - the bounded logical Session ID and fresh run ID;
    - the exact record plus component and management generations;
    - Orbit PID and start identity, both endpoint identities, and expected peer
      UID.
  - Files and process metadata locate or corroborate a candidate but never
    authorize attach or Stop.
  - Lease disconnect before Stop is non-destructive. Two replacements produce
    one winner and one Busy loser; accepted Stop is not cancelled by disconnect.
  - Management carries only bounded identity, live status, the ORB-C15
    `ProgramStatus` request, and explicit Stop, with one request and response
    in flight.
  - Canonical ORBS remains a separate one-input-client plus
    one-metadata-observer boundary retaining ORB-C1 through ORB-C11.
  - Stop succeeds only through the validated lease after ORB-C12 succeeds.
  - Natural child exit and successful Stop atomically replace the live record
    with one non-authoritative terminal tombstone containing termination reason
    and exact exit-code-or-signal outcome, then remove only exact endpoints.
- **Important failures:**
  - Validation or transport failure authorizes no PID, process group, process
    name, pathname, procfs scan, cgroup, or pidfd fallback.
  - Bounds are explicit:
    - Eon start and recovery: five seconds;
    - Orbit incomplete handshake: one second;
    - each management message and record: 4 KiB;
    - each identity: 128 UTF-8 bytes;
    - failure detail: 1 KiB.
  - Runtime directories must be exact UID-owned mode-0700 directories; sockets
    and regular records must be exact UID-owned mode-0600 objects.
  - Symlinks, unexpected types, wrong ownership or mode, replacement, overflow,
    malformed or incompatible state, and slow peers fail closed without blocking
    PTY, presentation, child observation, or shutdown.
- **Owner:** Orbit's private management owner and canonical protocol package;
  platform mechanics remain isolated, while Eon owns consumer policy.
- **Consumes:** Canonical management v3 and ORB-C12.
- **Boundary:**
  - The contract is per live Orbit run. Eon retains topology, enumeration,
    cleanup, and recovery policy.
  - No workspace topology, retained log, Orbit or machine restart, logout or
    reboot survival, multiplayer, remote access, public protocol,
    service-manager requirement, or same-UID sandbox is promised.
  - Platform-neutral identity, lease, status, and failure values use shared
    endpoint, permission, record, and polling mechanics with isolated Linux and
    Darwin peer-credential and process-start identity mechanisms.
- **Proof:** the `eon-awmq.3` import commit, with the `eon-awmq.7.4` commit
  for untrusted records and runtime directories and the
  `eon-agent-sidebar-yx8.5` commit for management v2
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, `orbit-terminfo`, every `orbit-<name>` real-PTY
    check); sandboxed `nix develop -c eon-test` (workspace and installer;
    `tests/lifecycle.rs` unmodified by the import, and changed only for the
    `--management-v2` flag in `eon-agent-sidebar-yx8.5`);
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` of the previous Rust server at
    `4910b81e9d68942a65ad96ad21e03385de7144f5` (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`managed_run_survives_launcher_loss_and_has_one_replacement_owner`](../tests/lifecycle.rs)
    - [`management_authority_negatives_fail_closed_without_stopping_session`](../tests/lifecycle.rs)
    - Real-PTY checks
      [`orbit-management-record`](../tests/real_pty/management_record.rs),
      which also refuses symlinked, FIFO, wrong-mode and directory records and
      wrong-mode or symlinked runtime directories before publishing, and
      [`orbit-management-lease`](../tests/real_pty/management_lease.rs)
    - Laws [`management_answers_are_accounted`](../LAWS.bend),
      [`management_stop_needs_the_lease`](../LAWS.bend) and
      [`management_follows_its_contract`](../LAWS.bend) (`orbit-laws`)
    - Workspace suite
    - Eon consumes management v2 in this repository.
    - Both lifecycle checks passed on macOS for the previous Rust server; the
      managed run's `/proc` check that the PTY child inherits no Orbit
      listener is Linux-only.

## ORB-C14 — Apple Silicon macOS platform

- **Status:** Partially proved
- **Consumer:** A native Orbit user on `aarch64-darwin`.
- **Trigger:** The Orbit server launches one ordinary or managed PTY Session
  with local Unix endpoints on Apple Silicon macOS.
- **Result:** The same Rust host, Bend worker, terminal authority, semantic
  input, canonical protocols, and presentation frames used on Linux preserve the
  native platform-sensitive parts of ORB-C1 through ORB-C13. Shared Unix logic
  owns PTY, process-group, signal, polling, descriptor, resize, EOF, socket,
  permission, and cleanup behavior; libc supplies the OS ioctl values and
  isolated Darwin peer and process-start identity calls. ORB-C13 management
  preserves the same platform-neutral authority and failure contract.
- **Important failures:** Unsupported Darwin mechanics, unsafe endpoint or peer
  identity, incompatible libghostty behavior, lifecycle mismatch, or incomplete
  native evidence fails explicitly. Target evaluation or compilation alone is
  not a runtime or support claim.
- **Owner:** Orbit's Rust host owns native mechanics; its Bend session policy
  and the protocol owners retain every product invariant.
- **Boundary:** The contract covers ordinary and managed Orbit Sessions,
  including bounded direct-child shutdown. Venus and Eon integration, Nix
  packaging, `x86_64-darwin`, signing, notarization, direct distribution,
  remote transport, and restart persistence remain unsupported.
- **Proof:** Not proved for this tree. The previous Rust server proved it at
  `4910b81e9d68942a65ad96ad21e03385de7144f5`, and bend-orbit, which this tree
  imports, ran natively at `b017e4f02f2e85f2566957c6d503e330491aabd4`.
  - **Environment:** Apple M2 (16 GiB), macOS 26.7.1 (25G241),
    `aarch64-apple-darwin`. The previous server: native
    `cargo test --locked -p yazelix-orbit -p orbit-protocol` with Rust 1.96.0,
    recorded in `eon-orb-c14-mac-reproof-l8rz`. bend-orbit: native build with
    Rust 1.95.0, zig 0.15.2, bun 1.3.13 and Bend
    `234cb9b950800f20b15feafcb4ce2f43e76e7fae`, recorded in
    `borb-orbit-in-bend-lj8.79` in the bend-orbit repository. Both used the
    macOS 15.4 Command Line Tools SDK and Ghostty
    `a887df42c56f6de86c0fe6da9c4eeca37931e083`.
  - **Evidence:**
    - The previous server passed all 74 protocol, management, unit and
      lifecycle tests natively, including the `conformance_` corpus.
    - bend-orbit printed `ALL PROOFS CHECK` for `PROOF.bend` and passed its 24
      Rust unit tests, all 36 real-PTY checks, and 15 of
      [`tests/lifecycle.rs`](../tests/lifecycle.rs)'s 18 tests; the other
      three are Linux-only.
    - [`managed_run_survives_launcher_loss_and_has_one_replacement_owner`](../tests/lifecycle.rs)
      checks a managed run's Darwin `proc_pidinfo` process-start identity
      against an independent read, and
      [`orbit-management-lease`](../tests/real_pty/management_lease.rs)
      exercises the Darwin `getpeereid` peer identity; on macOS the real-PTY
      checks use `lsof` where Linux reads `/proc`.
- **Open proof:** This tree differs from bend-orbit `b017e4f0` in its names,
  boundary schema digest, worker lookup and `orbit-protocol` path dependency
  (`eon-awmq.3`), so the bend-orbit run is evidence about the design, not a
  proof of this revision. Native re-proof runs `orbit-laws`, the unit tests,
  every real-PTY check except
  [`orbit-unreaped-stop`](../tests/real_pty/unreaped_stop.rs), and
  `tests/lifecycle.rs` on Apple Silicon. That check holds the child unreaped
  with Linux's `PTRACE_SEIZE`, which macOS lacks, so a Stop whose child cannot
  be reaped is checked on Linux only. Tests and assertions that read Linux-only
  process state do not run on macOS. Eon records
  the same macOS gap under `EON-C7` in
  [`docs/CONTRACTS.md`](../../docs/CONTRACTS.md).

## ORB-C15 — Program status records

- **Status:** Proved. Approved on 2026-10-21 (draft 15 on
  `eon-agent-sidebar-yx8.1`). `eon-agent-sidebar-yx8.4` stores the records
  and decides their lifetime; `eon-agent-sidebar-yx8.5` serves the
  `ProgramStatus` reply; `eon-agent-sidebar-yx8.11` adds the terminal title
  to the reply. Amended on 2026-10-27 (approved by Lucca, relayed by
  eon-overseer) by `eon-agent-background-work-yte0`: the reply counts
  background processes.
- **Consumer:**
  - a program in an Orbit terminal that speaks the Program Status Protocol
    (OSC 7501, rev 0.4), such as Claude Code 2.1.295 or later, or Codex 0.163
    or later;
  - Eon Runtime, which reads the records through management (ORB-C13).
- **Trigger:**
  - PTY output carries the probe `OSC 7501 ; ? ST` or a report
    `OSC 7501 ; pairs ST`;
  - an OSC 133 `A` prompt mark arrives;
  - the PTY foreground process group changes;
  - the child exits;
  - RIS arrives, which libghostty reports as a program-status clear with an
    empty id;
  - a management `ProgramStatus` request arrives.
- **Result:**
  - **Probe.** While Orbit sets libghostty's program-status callback and its
    PTY write callback, the library answers the probe. The answer is written
    before the reply to a DA1 query that follows the probe.
  - **Deviations from spec rev 0.4.** There are three:
    - the foreground-group drop under Lifetime, which is not a spec event;
    - child exit ends the Orbit run, so `done` and `error` records end with
      it, although the spec says they survive process exit (EON-C31 accepts
      this);
    - CLI read-back, which EON-C31 covers.
  - **Reports.** Everything else follows spec rev 0.4:
    - An unknown key, or a malformed pair, is skipped; the rest of the report
      applies.
    - Orbit discards the whole report if any of these holds:
      - it breaks a spec limit: a key over 16 bytes; `msg` over 2732 encoded
        or 2048 decoded bytes; `title` over 256 encoded or 192 decoded bytes;
        `app` over 32 bytes; `id` over 128 bytes, a segment over 32, or more
        than 8 levels;
      - a base64 value fails to decode;
      - the decoded `title` or `msg` contains control characters;
      - the `id` is invalid (Orbit never falls back to the root record);
      - `state` is missing or unknown.
    - Some invalid values are treated as absent and do not discard the
      report: an `app` with characters outside its set, an unknown `kind`,
      and an out-of-range `progress`.
    - Orbit disarms bidi-override and invisible format characters.
    - A valid report replaces the record with its `id`; an absent `id` means
      the root record. A record holds `state`
      (`idle | working | done | blocked | error`), `kind` (blocked only),
      `progress`, `app`, `title` and `msg`. A record without `app` inherits
      the `app` of its nearest ancestor when Orbit answers.
    - OSC 7501 is the sole source of program status, so no precedence among
      sources applies.
  - **Clearing.** `state=clear:id=X` removes X and every record beneath it.
    `state=clear` with no `id` removes all records, and so does RIS.
  - **Neutral per-record facts.** Orbit derives no attention meaning.
    - **Update sequence.** One counter per terminal increases by one on each
      accepted report, clear and lifetime drop, so the Runtime sees removals.
    - **Creation sequence.** The update sequence of the report that created
      the record. Later reports keep it; a record that is cleared, dropped or
      evicted and then reported again is a new record with a new one.
    - **Change sequence.** The update sequence at which the record's state
      last changed. Creating a record counts as a change; a same-state
      re-report keeps it.
    - **Previous state.** The state before that change, or none for a newly
      created record.
    - **Ages.** The Orbit-monotonic ages, in milliseconds, of the last update
      and of the last state change.
  - **Background processes.** On each `ProgramStatus` request, while Orbit
    holds at least one record, Orbit counts the terminal's background
    processes: processes that descend, by parent process, from a member of
    the PTY foreground process group, are not in that group, are neither
    stopped nor zombies, and started at least 1 s before the request. Orbit
    walks at most 512 processes below its child and counts at most 255; a
    walk that reaches either bound reports the bound. The count is 0 while
    Orbit holds no records. It is a neutral terminal fact, like the terminal
    title: Orbit gives it no status meaning. A process that leaves the tree,
    such as a double fork that init adopts, is not counted. Linux reads
    `/proc`; macOS uses libproc, unproved until ORB-C14's native proof.
  - **Lifetime.** Orbit removes `working` and `blocked` records on any of
    these:
    - the child exits, which ends the run and every record with it;
    - an OSC 133 `A` prompt mark arrives;
    - the PTY foreground process group changes to the child's own group from
      a different group.
      - This applies only on terminals that have never sent OSC 133 `A`. The
        latch is permanent, so one nested `nu` in a Bash pane turns the rule
        off for the rest of that run. This is accepted for 1.0.
      - It is edge-triggered. It is sampled at the start of each PTY read,
        before that read's bytes are processed, and on each `ProgramStatus`
        request. A report in the same read therefore comes after the edge and
        survives it.
      - A popup tool that is the child never triggers it while it keeps the
        foreground group.
      - With prompt marks (Nushell emits them natively), a shell script that
        reports its own work (`status working; rsync …; status done`) keeps
        its record until the prompt.
      - Without prompt marks (Eon's managed Bash today),
        `status working; make; make test` loses its `working` record when a
        job returns to the shell. Sampling can miss a short return, so this
        happens unpredictably. This is accepted for 1.0; shipping OSC 133 for
        Bash is a follow-up.

    Lifetime drops apply in byte-stream order: a prompt mark or foreground
    edge drops only the `working` and `blocked` records that exist at that
    point in the stream, and a report after it, even in the same PTY read,
    survives. `done`, `error` and `idle` records remain until the program
    replaces or clears them, or the run ends. Orbit uses no expiry and no
    heartbeat. The spec forbids requiring re-sends and leaves to the terminal
    when to stop showing `done` or `error`; EON-C31 decides that.
  - **ProgramStatus reply.** One request gets one reply of at most 4 KiB. It
    carries the record count, the background process count, the terminal's
    latest update sequence, the terminal title, the creation sequence of each
    record omitted for size, and as many records as fit in a neutral fill
    order: first `blocked`, `error` and `done`, most recent change first; then
    every other record, most recent change first.
    Each record carries its full `id`, `state`, `kind`, `progress`, `app`
    (32 bytes or less), `title` (96 or less) and `msg` (256 or less), its
    update sequence, creation sequence, change sequence, previous state and
    both ages. Orbit truncates only `title` and `msg`, at UTF-8 boundaries.
    The omitted creation sequences count toward the 4 KiB and let a reader
    tell a record omitted for size from one Orbit no longer holds. The
    terminal title is the window title that ORBS metadata publishes (OSC 0
    and 2), empty when the terminal has none or its title is not valid
    UTF-8. Orbit removes its control characters and the invisible characters
    it removes from record text, and cuts it to 64 bytes at a UTF-8
    boundary. It is a neutral terminal fact, not a record, and Orbit gives
    it no status meaning. With every record and
    the terminal title at their bounds, a reply carries up to 7 records while
    the terminal holds 14 or fewer, and at least 3 when it holds 256.
- **Important failures:**
  - Orbit ignores any report over 4096 bytes.
  - Past 256 records, Orbit evicts the least-recently-updated one.
  - Status work never blocks PTY reading. A report causes no frame or
    metadata publication beyond what the PTY read that carried it already
    causes (ORB-C4 unchanged).
- **Owner:**
  - The Rust host owns raw observation and mechanism
    ([HOST.md](HOST.md#what-rust-holds)): it parses reports through
    libghostty, stores records with their neutral facts, applies the spec's
    eviction, and assembles reply content, the terminal title included, by
    the fixed fill order.
  - The Bend worker decides from one-line numeric facts:
    - whether a prompt mark or foreground-group edge drops the `working` and
      `blocked` records;
    - whether to answer a `ProgramStatus` request or refuse it with Busy. It
      never defers. Orbit targets an answer within 250 ms. The worker-stall
      bound ([HOST.md](HOST.md#processes-and-pipes)) caps each fact and
      decision exchange at 2 s, and an answer needs at most two exchanges: a
      foreground-edge drop decision, then the answer. Host turn scheduling
      can add to that. A stalled worker ends that terminal's run, as it does
      for every exchange. The Runtime's own 250 ms poll deadline covers a
      miss: a miss keeps the last good result, and attention becomes stale
      only after 3 s without a good reply.

    Bend also holds the state its decisions read: whether the terminal has
    ever sent OSC 133 `A`, and the last sampled foreground group
    ([src/status.bend](../src/status.bend)). The host keeps no session state
    that Bend decides from.
  - The program owns what its report means.
- **Consumes:** libghostty-vt with its program-status and semantic-prompt
  options; ORB-C2's terminal authority.
- **Boundary:** No inference from the screen, process names or elapsed
  silence; no change to ORBS or ORB-C3; no event stream, no reading back by
  the program in Orbit, and no notifications; no OSC 9, 99 or 777 handling; no
  records across Orbit restarts. Portability: the background count uses an
  isolated Linux (`/proc`) and Darwin (libproc) mechanism in Orbit's platform
  seam; everything else is POSIX (`tcgetpgrp`, the PTY), and macOS stays
  deferred.
- **Proof:** the `eon-agent-sidebar-yx8.5` commit, with the
  `eon-agent-sidebar-yx8.4` commit for the records and their lifetime, the
  `eon-agent-sidebar-yx8.12` commit for creation sequences, the
  `eon-agent-sidebar-yx8.11` commit for the terminal title and the
  `eon-agent-background-work-yte0.1` commit for the background process count
  - **Environment:** x86_64 Linux; `nix flake check -L` (`orbit-laws`,
    `orbit-boundary`, every `orbit-<name>` real-PTY check); sandboxed
    `nix develop -c eon-test` (workspace and installer)
  - **Evidence:**
    - Real-PTY check [`orbit-program-status`](../tests/real_pty/program_status.rs):
      through a management lease, the reply carries the child's window title,
      a prompt mark drops in stream order, a foreground return drops once on
      a terminal without prompt marks, a return inside a detached turn ends
      the turn there, requests are answered within the worker-stall
      bound while the child floods the PTY
      ([BEHAVIOR.md](BEHAVIOR.md#program-status) records the measured
      answer times), and the reply counts a foreground job's processes in
      their own groups after 1 s and only while a record exists, never the
      shell's own job, an in-group or a stopped process
    - Real-PTY check [`orbit-terminal-replies`](../tests/real_pty/terminal_replies.rs):
      the probe's answer arrives before the device attributes reply
    - Host unit tests in [src/status.rs](../src/status.rs) on real
      libghostty terminals: the report rules and limits, the neutral facts,
      clears and RIS, app inheritance and disarmed text, eviction past 256
      records, drops in stream order, a real PTY child's reports, Claude
      Code's and Codex's recorded reports, the reply's fill order, cuts
      and omitted creation sequences, and its cleaned and cut terminal title;
      `the_worker_leads_each_output_with_its_status_lifetime`
      ([src/host.rs](../src/host.rs)) against the built worker
    - Protocol test `program_status_replies_keep_their_bounds` and the
      management round trip ([crates/protocol/tests/management.rs](../crates/protocol/tests/management.rs))
    - Laws `status_prompt_marks_drop`, `status_foreground_returns_drop`,
      `status_foreground_drops_once`, `status_unknown_samples_keep_the_last`,
      `status_prompt_marks_end_foreground_drops`,
      `management_program_status_needs_a_live_lease` and
      `management_follows_its_contract` ([LAWS.bend](../LAWS.bend),
      `orbit-laws`); a turn's fact carries only its foreground sample and
      whether it had a prompt mark, so reports add no publication beyond the
      laws over `step` (ORB-C4)
    - Claude Code 2.1.303 and Codex 0.163.0 inside an isolated Orbit on
      2026-10-21 ([BEHAVIOR.md](BEHAVIOR.md#program-status))
    - Installed (`eon-agent-sidebar-yx8.9`). The run used the installed Eon
      package `/nix/store/m5mzxy8cikz3f6y7k0ldgf7qxmc64bji-eon-0.1.1` (edge
      `c8c62cb63938b145b13cfa4fe161ea5689807652`) under private headless
      Sway.
      - Through Eon's poll, Claude Code 2.1.303 and Codex 0.164.0 each
        reported working, blocked and a finish.
      - Claude Code 2.1.293 reported nothing.
      - A clear and an identical `done` sent back to back read as a new
        record.
      - A terminal host held under SIGSTOP answered nothing, and after
        SIGCONT it served its records unchanged, with its child still running.

## ORB-C16 — Static graphics resource plane

- **Status:** Proved
- **Consumer:**
  - a program in an Orbit terminal that shows static images through the Kitty
    graphics protocol, directly or with Unicode placeholders, such as Yazi
    image previews;
  - the one attached client, Venus, which draws them under VEN-C31.
- **Trigger:**
  - PTY output transmits, places, replaces or deletes a Kitty image, prints or
    erases Unicode placeholder cells, scrolls, clears, resets or switches
    between the primary and alternate screens;
  - the client attaches, resizes, scrolls the viewport, reads slowly, detaches
    or reattaches;
  - the child exits.
- **Result:**
  - **Storage.** libghostty alone parses Kitty graphics and stores images.
    Orbit sets its storage limit to 10,000,000 decoded bytes for each screen,
    accepts only the direct transmission medium, and installs no PNG decoder.
    A stored image is therefore 8-bit RGB or RGBA.
  - **Capability.** An internal Orbit switch, which is not user configuration,
    gates this contract. While it is on, every complete frame states
    `kitty_graphics` true and carries the graphics state below. While it is
    off, frames state false, carry no entries and keep placeholder cells as
    they are, and Orbit sends no resources. It is on in the shipped build
    from `orb-j2i.11`, the change that makes Venus draw images; only Orbit's
    unit tests build frames with it off. The PTY pixel size rule below does
    not depend on it.
  - **Placements.** Each frame lists the placements that libghostty's render
    state resolves for the viewport, with no overscan rows: fixed, relative and
    Unicode-placeholder placements, the latter as one entry per run of
    neighbouring placeholder cells in a row. Each entry gives:
    - the resource identity and libghostty's layer and z-index;
    - the viewport cell of its top-left corner, which for fixed and relative
      placements can lie above or left of the viewport;
    - its grid size in cells;
    - its pixel offset from that cell's top-left corner, which can exceed one
      cell when libghostty centres an image in its placeholder cells;
    - its drawn pixel size and its source rectangle in image pixels.
    The frame states the cell width and height in pixels that this geometry
    uses. Entries are listed in drawing order: by layer, then z-index, then
    Kitty image id, lower first (the specification's order for equal
    z-indexes), then libghostty's order.
  - **Entry states.** Each entry also carries a state:
    - **ready**, when its resource's last chunk has been written to the
      socket in this attachment;
    - **pending**, while that has not happened;
    - **unavailable**, when the image breaks the resource limits below and is
      never sent.
    When a pending resource's last chunk is written, Orbit publishes a new
    complete frame at the next revision (ORB-C4), after any synchronized
    output hold ends, so its entries become ready without further PTY output.
  - **Placeholder cells.** Rows no longer carry the ORBF v2
    `kitty_virtual_placeholder` flag. Frames and previews carry Unicode
    placeholder cells (U+10EEEE and their diacritics) as blank cells that keep
    only their background, so a client never draws or interprets placeholder
    codepoints or their decorations.
  - **Resources.** A resource is one image's decoded pixels at one libghostty
    image generation. Orbit assigns a resource its 64-bit identity number when
    it first queues a frame that references it. Numbers strictly increase
    within an attachment and are never reused, and Orbit starts resources in
    identity order, so announcements carry increasing identities. A resource
    that is skipped and later referenced again gets a fresh number. Replacing
    an image's pixels, even under the same Kitty image id, makes a new resource
    with a new identity.
    - **Sending.** Orbit sends a resource as one announcement, giving identity,
      libghostty generation, format (RGB or RGBA), width, height, byte length
      and chunk count, followed by its chunks in order. Each chunk names its
      resource's identity and its index, counting from 0. Every chunk except
      the last carries exactly 262,144 bytes, and the last carries the rest.
      Orbit sends one resource at a time: all of one resource's chunks precede
      the next announcement. Frames and other messages may arrive between
      chunks.
    - **Order and copies.** Orbit sends each resource at most once per
      attachment, and none before the attachment's first complete frame. After
      that it starts a resource once it has queued the first frame that
      references it, when the resource queue below has room. It copies the
      pixels out of libghostty storage once, when it starts the announcement,
      and finishes the transfer from that copy even if storage later replaces
      or drops the image.
    - **Pending.** While the resource queue is full, Orbit sends frames with
      the entry pending. Once room returns it starts the resource from
      libghostty storage if a current frame still references it, and starts
      nothing if storage has dropped that generation or no frame references it.
    - **Retirement.** When a sent resource leaves the storage of the screen
      that holds it, including when a new generation replaces it, Orbit retires
      it with one message. If that happens during its own transfer, Orbit
      finishes the transfer first. A retirement travels in the resource queue,
      in order with announcements and chunks. Orbit writes it only after it has
      written a complete frame that no longer references the resource, and
      writes no later frame that does. Orbit sees a screen's storage only while
      that screen is active, so a resource of the inactive screen stays
      unretired until the screen is active again or the attachment ends.
  - **Screens.** A frame references only the active screen's images. Each
    screen keeps its own storage, which survives switching away, so returning
    to a screen can reference images again that were sent earlier in the same
    attachment.
  - **PTY pixel size.** The PTY's pixel fields hold the text area, columns
    times cell width by rows times cell height, and Orbit answers CSI 14t,
    16t and 18t from the same measurements. A program that derives its cell
    size from them, such as Yazi, then sizes its placeholder grid to
    libghostty's cells.
  - **Limits.**
    - A resource is 1 to 4,096 pixels on each side and at most 10,000,000
      decoded bytes.
    - A frame carries at most 4,096 placement entries referencing at most 256
      resources. Entries do not count toward ORB-C6's frame budget for cells
      and links; ORBF v3 bounds them separately.
    - Per attachment, Orbit holds at most one resource copy outside
      libghostty storage, the one transfer in progress, whose queued chunks
      count toward the resource queue below.
    - Resource messages (announcements, chunks and retirements) wait in their
      own resource queue, apart from every other message. They never take
      ORB-C7's frame and scroll-outcome room. A frame waits at most for the
      rest of the one resource message already being written. At most 1,048,576
      bytes of chunk payload are queued per attachment, besides announcements
      and retirements, which the count below bounds. This bound is internal to
      Orbit; Venus does not check it.
    - At most 1,024 resources are announced and not retired per attachment. A
      resource that would pass that count is not started until one retires, so
      its entries stay pending.
- **Important failures:**
  - An image outside the limits stays in libghostty storage and is never
    sent. Its entries are unavailable, so the client marks them instead of
    dropping them silently.
  - Placements beyond 4,096 entries or 256 resources: the list stops at the
    first entry, in drawing order, that would pass either cap, and the frame
    states that placements were omitted.
  - A full resource queue makes entries pending. Resource messages never cause
    ORB-C7's disconnect, block PTY processing or grow the queue; every other
    message keeps ORB-C7's behavior unchanged.
  - Failed extraction publishes no partial frame; the frame fails as ORB-C6
    already requires. A failed copy starts no resource, so its entries stay
    pending; when Orbit tries again, the resource takes a fresh identity.
  - Detach discards every per-attachment identity and copy. The next
    attachment starts with no resources sent: its first complete frame lists
    every visible placement as pending or unavailable, and the resources
    follow as above.
  - On child exit, Orbit drops the resource messages it has not started
    writing, finishes the one it is writing, and then sends `Exited`. Exit ends
    the run and its images; nothing persists across an Orbit or machine
    restart.
- **Owner:** Orbit.
  - Its Bend session policy decides when resources start, which keeps entries
    pending until then, and decides retirement and pressure, within budgets
    that a law bounds.
  - Its Rust host reads libghostty's render-state image list and storage. It
    assigns identities, applies the fixed resource limits that make entries
    unavailable, copies pixels out of storage when the policy starts a resource
    (which needs no PTY output), and writes resources. Pixel bytes never cross
    the worker pipe.
  - Canonical `orbit-protocol` values and codecs own the wire.
- **Consumes:**
  - libghostty-vt's render-state image iterator and Kitty graphics storage
    at the upstream `Uzaaft/libghostty-rs` commit that CRATES.md records;
    `orb-j2i.1` proved the interface at
    `9ede35b8ea94a55ed3608408d67e102883129f1e`;
  - the Kitty graphics protocol as specified at `kovidgoyal/kitty`
    `b2cb871254fbdac19a085188813c8dfb258b95cd`,
    `docs/graphics-protocol.rst`.
  - The wire is ORBF v3 in ORBS v14, a breaking, exact-version revision with
    no adapter or compatibility window. Venus, Eon Runtime and Eon consume it
    in this repository. Orbit lands first, then Venus, in one Eon build.
- **Boundary:**
  - Static images only. Orbit does not drive libghostty's animation tick, so
    an animated image shows whichever frame libghostty holds as current;
    animation playback and frame composition are not presented.
  - Clients draw every layer in one band, between cell backgrounds and text
    (VEN-C31). Kitty's other layers are a recorded gap: an image placed above
    text draws below it, and one placed below cell backgrounds draws above
    them.
  - Scrollback previews (ORB-C8) carry no placements and blank their
    placeholder cells. A viewport scrolled into history is a live frame and
    carries its placements.
  - A displayed image that libghostty's storage limit evicts disappears with
    its placements, and nothing marks it. This is a recorded gap.
  - Placeholder cells whose image libghostty cannot resolve, for example
    after the image is deleted, are blanked like any other placeholder cell
    and carry no entry, so nothing marks them. This is a recorded gap.
  - ORB-C9 copies text from libghostty, which keeps placeholder codepoints,
    so copying across placeholder cells yields U+10EEEE and its diacritics.
    This is a recorded gap.
  - At the cap of 1,024 unretired resources, entries of further resources
    stay pending until one retires. This is a recorded gap for programs that
    keep many images stored.
  - File, temporary-file and shared-memory transmission, PNG, Sixel, iTerm2
    images, a client request for a missing resource, content addressing and
    restart persistence are excluded.
- **Proof:** Adopted on 2026-10-27 under Lucca's standing
  approval of 2026-10-26, relayed by eon-overseer (`orb-j2i.2`).
  `orb-j2i.4` sizes the PTY; `orb-j2i.5` adds the wire revisions;
  `orb-j2i.6` and `orb-j2i.7` extract placements and send resources;
  `orb-j2i.8` proves the lifecycle. With the switch on, which it is in the
  shipped build from `orb-j2i.11`, the real-PTY checks
  [`orbit-kitty-placements`](../tests/real_pty/kitty_placements.rs),
  [`orbit-kitty-replacement`](../tests/real_pty/kitty_replacement.rs),
  [`orbit-kitty-erase`](../tests/real_pty/kitty_erase.rs),
  [`orbit-kitty-scroll`](../tests/real_pty/kitty_scroll.rs),
  [`orbit-kitty-screens`](../tests/real_pty/kitty_screens.rs),
  [`orbit-kitty-reattach`](../tests/real_pty/kitty_reattach.rs),
  [`orbit-kitty-pressure`](../tests/real_pty/kitty_pressure.rs),
  [`orbit-kitty-exit`](../tests/real_pty/kitty_exit.rs) and
  [`orbit-kitty-cap`](../tests/real_pty/kitty_cap.rs) cover transfer,
  replacement, deletion, clear, scroll, resize, both screens, reattachment,
  pressure, exit and the cap
  ([BEHAVIOR.md](BEHAVIOR.md#kitty-graphics-placements)). They passed on
  x86_64 Linux, built as Nix checks of the flake with every other check, at
  the commit that records `orb-j2i.11`'s proof. `orb-j2i.12` proved Yazi's
  previews on the installed Eon
  `/nix/store/77w9f35fj1p8a9hz89j5wdcbii2b8bhc-eon-0.1.5`, built from
  `7dc3509` with Orbit
  `/nix/store/bf4if4kd62xdrsvx6kb78bzf1kqdcdp7-yazelix-orbit-0.1.0` and Yazi
  26.5.6 (`aa526434f00bb44e2e902d9a4ac5f810da1018b9`), on x86_64 Linux in an
  isolated headless Sway 1.12 on its Vulkan renderer. In a pane and in the
  Files popup, Yazi's previews of RGB and RGBA files reached Venus as RGB and
  RGBA resources, drew at Yazi's place and size, and went away on a text file,
  and in the pane when Yazi quit. A direct RGBA placement on the primary screen scrolled off with
  its rows, came back clipped when the viewport scrolled into history, stayed
  hidden while Yazi held the alternate screen, and came back when Yazi quit.
  Resizes, a tab switch and back, and window detach and reattach redrew
  Yazi's preview; hiding and showing the Files popup redrew the pane's image
  and then the popup's preview. A trace of Orbit's socket showed one
  announcement and its chunks for each resource, with byte and chunk counts
  that follow from its format and size, and one retirement for each replaced
  image; the bead records the measurements.

## Focused terminal conformance corpus

The current corpus groups three accepted tests in
[`tests/lifecycle.rs`](../tests/lifecycle.rs) under one offline, headless
command without copying their stimuli or expectations:

```sh
nix develop -c eon-test -p yazelix-orbit --test lifecycle conformance_
```

The command runs the pinned `libghostty-vt` path backed by Ghostty
`521cf2c5d30bd48028ca61546c64e7913764073d`. Protocol-only checks remain in
the normal suite because they do not exercise that terminal engine.

| Case | Stimulus | Orbit-visible observation | Assertion policy | Owner |
| --- | --- | --- | --- | --- |
| `conformance_c2_parser_state_and_terminal_replies_survive_client_failure` | Split DEC 1049 input across client loss, then cursor visibility and position query | The authoritative parser completes the alternate-screen transition, emits one PTY response, and later restores primary | Exact reply bytes and single emission; semantic screen and cursor state | `ORB-C2` |
| `conformance_c5_authoritative_input_modes_and_resize_survive_detach` | Resize plus key, mouse, focus, paste, Kitty-keyboard, mode change, and reattach | The PTY receives terminal-state-aware input once and retains the requested dimensions | Exact PTY bytes and `stty` dimensions | `ORB-C5` |
| `conformance_c9_selection_copy_is_authoritative_bounded_and_client_scoped` | Wide and combining text, viewport movement, reversed selection, detach, alternate-screen output, and resize | Selected presentation is client-scoped while finished copy stays frozen across later terminal mutations | Exact copied Unicode text; semantic selection visibility, scope, and freeze | `ORB-C9` |

ORB-C4, ORB-C6 and ORB-C8 have no corpus case: the real-PTY checks
`orbit-three-attach`, `orbit-held-attach`, `orbit-mouse`, `orbit-scroll`,
`orbit-return-to-live` and `orbit-history`, and the host unit test
`direct_rows_match_rich_canonical_frame_rows`, exercise them; ORB-C8's open
proof names what they do not cover.

A mismatch is classified from the smallest reproducer: an Orbit regression if
the adapter or ownership path changed, an upstream behavior change or defect if
only the pinned engine changed, an unsupported candidate capability in a
separately authorized engine spike, or a corpus expectation error when the
assertion exceeds the indexed contract. A mismatch does not broaden product
compatibility; product fixes and upstream submissions require their own
authorized work.

The inventory is the minimum test-only boundary a future candidate-engine spike
may consume: stimulus, Orbit-visible observation, and assertion policy. It does
not authorize a shared runtime trait, production adapter, second engine, public
trace, golden corpus, or dependency. External VT and Unicode certification
remains owned by `orb-vt-unicode-baselines-i32`; performance remains owned by
`orb-1sd`.

## Rules

- Each contract uses one `## ORB-CN — Name` heading and the required fields
  `Status`, `Consumer`, `Trigger`, `Result`, `Important failures`, `Owner`,
  `Boundary`, and `Proof`.
- Add optional `Consumes` and `Open proof` fields when applicable; nest
  `Environment` and `Evidence` under `Proof`, and use nested bullets instead of
  prose table cells.
- Contract IDs are stable and repository-qualified. Never renumber or reuse an
  ID; mark an explicitly removed contract retired.
- Only user-approved user-visible behavior, correctness boundaries, ownership
  invariants, and interfaces between subsystems receive contract IDs.
- Do not index functions, modules, dependencies, configuration literals,
  individual tests, or speculative future features.
- Every product implementation Bead names the contract IDs it creates, changes,
  proves, consumes, hardens, or preserves. If requested behavior has no indexed
  contract, stop for user approval before changing code or the index.
- The Bead's Reference gate records how evidence constrains the implementation;
  this index records the durable contract rather than duplicating research
  notes.
- Closing an implementation Bead updates the status, owner, checks, and gap for
  every affected contract.
- A `Proved` contract names its exact proof-bearing commit, or its owning Bead
  when that Bead's own commit is the proof's revision, as in Eon's index. A later
  change to its owner or implementation surface reruns the indexed checks and
  advances that revision, or downgrades the status and records the gap.

## Consumer compatibility

A repository consuming Orbit records each contract ID and its exact accepted
proof revision. Boundary changes are classified as compatible or breaking and
name every known consumer and update order. A client does not hide an Orbit gap
behind an adapter, compatibility window, second decoder, or second source of
authority without an explicit user decision.

## Outside the initial index

Orbit's accepted long-term direction includes host-owned terminal-session state
and native, web, or mobile clients. Eon owns product workspace topology and
policy. Multiple sessions, windows, tabs, splits, restart recovery, remote
transport, web, mobile, and multiplayer are not initial Orbit contracts and
receive no IDs until the user authorizes their implementation scope. ORB-C14 is
the sole authorized platform expansion; it covers Orbit alone, and Venus and
Eon have not adopted macOS.
