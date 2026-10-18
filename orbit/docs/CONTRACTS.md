# Orbit contract index

This is the canonical index of Orbit's accepted behavioral contracts,
ownership boundaries, proof status, and remaining gaps. `AGENTS.md` governs how
agents work, Beads govern authorized changes, and this index records what the
system must preserve.

Orbit builds `libghostty-vt` and `libghostty-vt-sys` 0.2.2. The native build
explicitly passes `-Dcpu=baseline` by default, preserving Ghostty
`a887df42c56f6de86c0fe6da9c4eeca37931e083`, generated bindings and
ORBF v2 / ORBS v13. This preserves the headless terminal contract checks; it
does not establish older-CPU compatibility for a published cache artifact.

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

Canonical ORBF v2 carries two little-endian `u64` fields after cursor metadata
and before rows: `ScrollPosition.rows_from_live` and
`ScrollPosition.history_rows`. The latter excludes the active viewport; the
former cannot exceed it and is zero at live bottom. Both are zero on the
alternate screen. Counts describe current wrapped display rows, not stable
logical lines, and are extracted with each complete frame through the existing
synchronized publication owner. Output, scrolling, reflow, pruning,
clear/reset, screen switches and reattachment therefore carry position at that
frame's revision. Failed engine extraction does not fabricate zero or publish a
partial frame. Existing 4 MiB/100,000-cell bounds remain.

ORBS v7 through v13 are explicitly breaking, exact-version boundaries with no
compatibility window or adapter; each replaced its predecessor. The diagnostic
client, owned test fixtures, Venus and Eon consume the canonical crate in this
repository. Each contract below states the current ORBS v13 behavior.

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
- **Owner:** Orbit's concrete runtime coordinator with the isolated platform PTY
  lifecycle.
- **Boundary:** Persistence across Orbit or machine restart is excluded.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`;
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` at `4910b81e9d68942a65ad96ad21e03385de7144f5`
    (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`transient_pty_eio_recovers_when_the_live_child_reopens_the_terminal`](../tests/lifecycle.rs)
    - [`simultaneous_stale_socket_claim_has_one_reachable_owner`](../tests/lifecycle.rs)
    - Workspace suite
    - On macOS the stale-socket check passes; the transient PTY EIO check is
      Linux-only.

## ORB-C2 — Sole terminal-state authority

- **Status:** Proved
- **Consumer:** Every Orbit runtime and attached client.
- **Trigger:** PTY bytes, client interaction, or terminal-generated responses
  change terminal state.
- **Result:** Orbit alone owns libghostty terminal state and alone sends
  terminal-generated responses to the PTY.
- **Important failures:** Client failure cannot duplicate a terminal response or
  transfer parser authority.
- **Owner:** Orbit's platform-neutral concrete runtime coordinator.
- **Boundary:** Ordered client-visible effects beyond the current transport
  contract remain unproved.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`ansi_palette_is_bounded_and_resets_to_supplied_defaults`](../src/main.rs)
    - [`conformance_c2_parser_state_and_terminal_replies_survive_client_failure`](../tests/lifecycle.rs)
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
- **Important failures:** Excess roles, incompatible or malformed peers, slow
  readers, disconnect, and exit fail deterministically without disturbing the
  PTY, child, other role, or cleanup.
- **Owner:** Orbit attachment transport and runtime coordinator with canonical
  `orbit-protocol` ORBS v7.
- **Consumes:** Canonical ORBS v7 behavior, preserved through ORBS v13; Venus
  and Eon consume it in this repository.
- **Boundary:** Visible text observation, agent observation, multiple observers,
  multiple interactive clients, remote access, and process authority are
  excluded.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`metadata_observer_is_read_only_and_keeps_only_the_latest_change`](../src/attachment.rs)
    - [`metadata_observer_streams_inactive_title_and_cwd_without_owning_input`](../tests/lifecycle.rs)
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
- **Owner:** Orbit attachment transport and synchronized runtime publication
  with canonical session and complete-frame codecs.
- **Consumes:** Canonical ORBF v2 in ORBS v13; Venus and Eon consume it in this
  repository.
- **Boundary:** Native presentation quality remains Venus-owned.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`synchronized_presentation_coalesces_defers_and_times_out`](../src/runtime.rs)
    - [`real_pty_synchronized_output_holds_split_large_update`](../src/presentation.rs)
    - [`conformance_c4_real_pty_reattach_converges_through_complete_ordered_frames`](../src/presentation.rs)
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
- **Owner:** Canonical platform-neutral `orbit-protocol` values and Orbit's
  semantic interaction owner.
- **Consumes:** Canonical ORBS v9 semantic input, preserved through ORBS v13;
  Venus and Eon consume it in this repository.
- **Boundary:** Candidate-list IME and native input quality remain Venus-owned.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`kitty_release_never_falls_back_to_text`](../src/interaction.rs)
    - [`conformance_c5_authoritative_input_modes_and_resize_survive_detach`](../tests/lifecycle.rs)
    - [`authoritative_left_pointer_route_is_pinned_and_shift_selects`](../src/interaction.rs)
    - Workspace suite

## ORB-C6 — Rich presentation without silent degradation

- **Status:** Proved
- **Consumer:** Every canonical presentation client.
- **Trigger:** Orbit extracts or encodes terminal presentation state.
- **Result:** The boundary carries rich terminal state or explicitly declares an
  unsupported capability; it never silently collapses to a plain-text grid.
- **Important failures:** Malformed cell-width topology fails at every admission
  boundary.
- **Owner:** Orbit's authoritative extraction and synchronized publication with
  canonical `orbit-protocol` values and codecs.
- **Consumes:** Canonical ORBF v2 in ORBS v13; Venus and Eon consume it in this
  repository.
- **Boundary:** Kitty graphics remain explicitly unsupported.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`conformance_c6_direct_rows_match_rich_canonical_frame_rows`](../src/presentation.rs)
    - [`orbf_v2_cell_width_topology_is_validated_at_every_acceptance_boundary`](../crates/protocol/src/lib.rs)
    - Canonical ORBF-in-ORBS codec checks in the workspace suite

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
- **Owner:** Orbit attachment transport in the concrete runtime coordinator.
- **Boundary:** The proof covers one local input client plus one metadata
  observer.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`pending_presentations_keep_scroll_outcomes_and_latest_replaceable_revision`](../src/attachment.rs)
    - [`authoritative_viewport_survives_detach_and_slow_reader_pressure`](../tests/lifecycle.rs)
    - [`metadata_observer_streams_inactive_title_and_cwd_without_owning_input`](../tests/lifecycle.rs)
    - Workspace suite

## ORB-C8 — Retained history and authoritative scrolling

- **Status:** Proved
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
  - During an active primary-screen host selection, one ORBS v13 upward tick
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
- **Owner:** Orbit's semantic interaction and synchronized-presentation owners
  with libghostty's native viewport, byte-budgeted history, one canonical row
  extractor, and bounded output queue.
- **Consumes:** Canonical ORBF v2 in accepted ORBS v13; Venus and Eon consume
  it in this repository.
- **Boundary:** Arbitrary line-count guarantees, gesture cadence, downward
  selection autoscroll, velocity, kinetic effects, graphics, and restart
  persistence are excluded; physical wheels retain their existing typed
  terminal-routed or one-row behavior. This does not accept Eonova, or any
  ORBS v13 client on Apple Silicon (Venus, Eon, the diagnostic client or
  bend-orbit); ORB-C14 covers Orbit's own native ORBS v13 behavior.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`conformance_c8_scroll_position_follows_published_terminal_state`](../src/runtime.rs)
    - [`conformance_c8_authoritative_viewport_routes_wheel_and_key_from_terminal_state`](../src/interaction.rs)
    - [`scroll_position_is_canonical_at_every_acceptance_boundary`](../crates/protocol/src/lib.rs)
    - [`vertical_scroll_batches_are_bounded_and_canonical`](../crates/protocol/src/session/tests.rs)
    - [`pending_presentations_keep_scroll_outcomes_and_latest_replaceable_revision`](../src/attachment.rs)
    - [`conformance_c8_signed_scroll_batch_is_atomic_bounded_and_authoritative`](../src/interaction.rs)
    - [`conformance_c8_return_to_live_is_one_authoritative_jump_without_pty_input`](../src/interaction.rs)
    - [`conformance_c9_upward_selection_scroll_keeps_one_gesture_and_exact_copy`](../src/interaction.rs)
      covers cell, span, and line granularity with unchanged deferral, bounds,
      ordered outcomes, and exact Unicode copy.
    - [`return_to_live_has_zero_payload`](../crates/protocol/src/session/tests.rs)
    - [`authoritative_viewport_survives_detach_and_slow_reader_pressure`](../tests/lifecycle.rs)
    - [`real_pty_synchronized_output_holds_split_large_update`](../src/presentation.rs)
    - Workspace suite
- **Open proof:** Pending re-proof in this repository: installed scrollback and
  ReturnToLive through Venus (`eon-monorepo-2oko.6` dogfood item 9 was not
  exercised).

## ORB-C9 — Authoritative bounded selection and copy

- **Status:** Proved
- **Consumer:** One healthy exact-version ORBS v13 attachment for upward ticks.
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
- **Owner:** Orbit's semantic interaction owner with authoritative mouse modes,
  libghostty current-viewport gesture selection, and canonical ORBS v13. Venus
  owns native event delivery and clipboard effects.
- **Consumes:** Accepted ORBS v13 and ORB-C7 bounded-pressure behavior; Venus
  and Eon consume it in this repository.
- **Boundary:** Configurable word separators or click thresholds, block selection,
  downward autoscroll, cadence, semantic command-output selection, search,
  graphics, and restart persistence are excluded.
  Spans retain native word-selection limits: a wide-character trailing spacer
  can stop the selected span (`a;界,é` selects `a;界`). This inherited
  libghostty limitation is outside the click-ladder correction.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`authoritative_selection_click_ladder_preserves_wrapped_text_and_copy`](../src/interaction.rs)
      checks exact highlighted cells, frozen text for both copy effects,
      soft wraps, compatible live output, fourth/fifth clicks and late resets
      through canonical encoded results and frames.
    - [`conformance_c9_upward_selection_scroll_keeps_one_gesture_and_exact_copy`](../src/interaction.rs)
      covers more than two viewports, synchronized deferral, oldest-edge stop,
      and exact Unicode release copy.
    - [`every_client_message_round_trips`](../crates/protocol/src/session/tests.rs)
      and [`malformed_typed_payloads_are_rejected`](../crates/protocol/src/session/tests.rs)
      cover the exact ORBS v13 action and invalid positions.
    - [`authoritative_selection_accepts_presented_input_and_freezes_copy`](../src/interaction.rs)
    - [`active_selection_tracks_scrolling_pty_output`](../src/interaction.rs)
    - [`authoritative_selection_uses_libghostty_click_and_drag_granularity`](../src/interaction.rs)
    - [`authoritative_left_pointer_route_is_pinned_and_shift_selects`](../src/interaction.rs)
    - [`host_selection_supersedes_unpresented_drag_frames_before_release`](../src/interaction.rs)
    - [`conformance_c9_selection_copy_is_authoritative_bounded_and_client_scoped`](../tests/lifecycle.rs)
    - Workspace suite
- **Open proof:** Pending re-proof in this repository: the installed click
  selection ladder, drag selection and copy/paste through Venus
  (`eon-monorepo-2oko.6` dogfood item 9 was not exercised).

## ORB-C10 — Eon terminal identity

- **Status:** Partially proved
- **Consumer:** Every Orbit-owned PTY child and its launch environment.
- **Trigger:** Orbit launches a PTY child.
- **Result:** The child receives `TERM=eon` and `COLORTERM=truecolor`; the launch
  environment supplies the repository-owned compiled `eon` terminfo entry
  inheriting `xterm-256color`, so lookup and `clear` succeed.
- **Important failures:** Missing or invalid terminfo is exposed by lookup and
  real-PTY behavior checks rather than hidden by a fallback identity.
- **Owner:** Orbit's platform PTY seam and
  [`terminfo/eon.terminfo`](../terminfo/eon.terminfo); Eon installs the entry.
- **Boundary:** Other distribution formats remain outside this contract's proof.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; `nix build .#default`, installed
    `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0`
  - **Evidence:** The Nix build compiles the entry with `tic`; with only the
    installed Orbit package's `share/terminfo`, `infocmp -x eon` resolves the
    entry and `TERM=eon tput clear` emits the clear sequence. The platform seam
    sets `TERM` and `COLORTERM` in [`src/platform.rs`](../src/platform.rs).
- **Open proof:** Pending re-proof in this repository: the isolated real-PTY
  `TERM` and post-`clear` check.

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
- **Owner:** Orbit attachment transport and concrete runtime clipboard
  interpretation with libghostty normalized effects and canonical ORBS v7.
- **Consumes:** Canonical ORBS v7.
- **Boundary:** Ordinary clipboard delivery, Wayland without data-control,
  broader compositors, and non-Linux policy remain unproved.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`
  - **Evidence:**
    - [`src/attachment.rs`](../src/attachment.rs) ordered bounded output checks
    - [`real_pty_clipboard_write_precedes_its_frame`](../src/presentation.rs)
    - [`clipboard_capture_is_attached_typed_and_bounded`](../src/runtime.rs)
    - [`clipboard_output_pressure_disconnects_without_retaining_effects`](../src/runtime.rs)
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
  - After escalation, Orbit requires the direct child to be reaped within two
    seconds.
- **Important failures:**
  - An already-gone signal target is benign; every other signaling or reaping
    failure is bounded non-success.
  - PID reuse, foreground-group changes, child exit during grace, and writing
    descendants cannot cause delayed signaling or unbounded server lifetime.
  - Client disconnection remains the non-destructive ORB-C1 detach path.
- **Owner:** Orbit's concrete runtime coordinator owns graceful timing,
  escalation, direct-child reaping, and result truth; Unix PTY and process-group
  mechanics stay in the platform seam.
- **Boundary:**
  - Deliberately detached processes, non-foreground groups outside the initial
    group, and SIGHUP-ignoring foreground jobs whose shell exits during grace may
    survive.
  - Orbit scans no process names, ancestry, session IDs, or procfs and claims no
    whole-descendant cleanup. Direct-child HUP and reaping are also proved
    natively on Apple Silicon macOS; the special detached and foreground-group
    topology remains Linux-proved only.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`;
    installed `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0` driven
    headlessly by a Kinestra recipe on a private headless Sway (wlroots, pixman);
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` at `4910b81e9d68942a65ad96ad21e03385de7144f5`
    (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`shutdown_hups_the_unreaped_direct_child`](../tests/lifecycle.rs)
    - [`shutdown_escalates_foreground_and_permits_a_detached_process`](../tests/lifecycle.rs)
    - [`stale_socket_and_safe_signal_shutdown`](../tests/lifecycle.rs)
    - [`writing_descendant_cannot_hold_server_open_after_known_child_exit`](../tests/lifecycle.rs)
    - Workspace suite
    - Eon consumes this shutdown boundary; `eon-monorepo-2oko.6` dogfood
      checklist item 12 stops the installed generation.
    - On macOS the direct-child HUP, safe signal shutdown and management Stop
      checks pass; the escalation and writing-descendant checks are Linux-only.

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
  - Management carries only bounded identity, live status, and explicit Stop,
    with one request and response in flight.
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
- **Consumes:** Canonical management v1 and ORB-C12.
- **Boundary:**
  - The contract is per live Orbit run. Eon retains topology, enumeration,
    cleanup, and recovery policy.
  - No workspace topology, retained log, Orbit or machine restart, logout or
    reboot survival, multiplayer, remote access, public protocol,
    service-manager requirement, or same-UID sandbox is promised.
  - Platform-neutral identity, lease, status, and failure values use shared
    endpoint, permission, record, and polling mechanics with isolated Linux and
    Darwin peer-credential and process-start identity mechanisms.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** x86_64 Linux; sandboxed `cargo test --locked --offline --workspace`;
    Apple M2 macOS 26.7.1 native `cargo test --locked -p yazelix-orbit -p
    orbit-protocol` at `4910b81e9d68942a65ad96ad21e03385de7144f5`
    (`eon-orb-c14-mac-reproof-l8rz`)
  - **Evidence:**
    - [`managed_run_survives_launcher_loss_and_has_one_replacement_owner`](../tests/lifecycle.rs)
    - [`management_authority_negatives_fail_closed_without_stopping_session`](../tests/lifecycle.rs)
    - Workspace suite
    - Eon consumes management v1 in this repository.
    - Both checks pass on macOS; the managed run's `/proc` check that the PTY
      child inherits no Orbit listener is Linux-only.

## ORB-C14 — Apple Silicon macOS platform

- **Status:** Proved
- **Consumer:** A native Orbit user on `aarch64-darwin`.
- **Trigger:** The Orbit server launches one ordinary or managed PTY Session
  with local Unix endpoints on Apple Silicon macOS.
- **Result:** The same binary, owner loop, terminal authority, semantic input,
  canonical protocols, and presentation frames used on Linux preserve the
  native platform-sensitive parts of ORB-C1 through ORB-C13. Shared Unix logic
  owns PTY, process-group, signal, polling, descriptor, resize, EOF, socket,
  permission, and cleanup behavior; libc supplies the OS ioctl values and
  isolated Darwin peer and process-start identity calls. ORB-C13 management
  preserves the same platform-neutral authority and failure contract.
- **Important failures:** Unsupported Darwin mechanics, unsafe endpoint or peer
  identity, incompatible libghostty behavior, lifecycle mismatch, or incomplete
  native evidence fails explicitly. Target evaluation or compilation alone is
  not a runtime or support claim.
- **Owner:** Orbit's existing platform seam owns native mechanics; the shared
  runtime and protocol owners retain every product invariant.
- **Boundary:** The contract covers ordinary and managed Orbit Sessions,
  including bounded direct-child shutdown. Venus and Eon integration, Nix
  packaging, `x86_64-darwin`, signing, notarization, direct distribution,
  remote transport, and restart persistence remain unsupported.
- **Proof:** `4910b81e9d68942a65ad96ad21e03385de7144f5`
  - **Environment:** Apple M2 (16 GiB), macOS 26.7.1 (25G241),
    `aarch64-apple-darwin`; native `cargo build --locked -p yazelix-orbit`,
    `cargo test --locked -p yazelix-orbit -p orbit-protocol` and
    `cargo test --locked -p yazelix-orbit conformance_`
    with Rust 1.96.0, Zig 0.15.2 against the macOS 15.4 Command Line Tools
    SDK, and Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083`. The run and
    toolchain are recorded in `eon-orb-c14-mac-reproof-l8rz`. The shared Unix
    seam was first re-proved on x86_64 Linux at
    `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`.
  - **Evidence:**
    - All 74 protocol, management, unit and lifecycle tests pass natively,
      including the `conformance_` corpus; the two opt-in history measurements
      stay ignored, as everywhere.
    - [`management_artifacts_and_kernel_identity_fail_closed`](../src/platform.rs)
      runs the Darwin `getpeereid` peer identity.
    - [`managed_run_survives_launcher_loss_and_has_one_replacement_owner`](../tests/lifecycle.rs)
      checks a managed run's Darwin `proc_pidinfo` process-start identity
      against an independent read, then ends it with management Stop.
    - [`tests/lifecycle.rs`](../tests/lifecycle.rs) also covers attachment
      negotiation, real PTY input and resize, detach/reconnect, bounded
      pressure, metadata, owner-only endpoint permissions, EOF, stale-socket
      ownership, direct-child HUP/reaping, signal cleanup, and management
      launch, replacement, Busy, status, Stop, tombstones, and fail-closed
      identity and permission checks.
    - Linux-gated checks do not run on macOS: the lifecycle tests for transient
      PTY EIO, a writing descendant, and the detached and foreground-group
      shutdown topology; the unit test
      `exited_child_output_is_drained_into_authoritative_state`; and the managed
      run's `/proc` assertion that the PTY child inherits no Orbit listener.

## Focused terminal conformance corpus

The current corpus groups six accepted `orb-8s0` tests and the accepted x12
test under one offline, headless command without copying their stimuli or
expectations:

```sh
cargo test --locked conformance_
```

The command runs the pinned `libghostty-vt` 0.2.2 path backed by Ghostty
`a887df42c56f6de86c0fe6da9c4eeca37931e083`. Protocol-only checks remain in
the normal suite because they do not exercise that terminal engine.

| Case | Stimulus | Orbit-visible observation | Assertion policy | Owner |
| --- | --- | --- | --- | --- |
| `conformance_c2_parser_state_and_terminal_replies_survive_client_failure` | Split DEC 1049 input across client loss, then cursor visibility and position query | The authoritative parser completes the alternate-screen transition, emits one PTY response, and later restores primary | Exact reply bytes and single emission; semantic screen and cursor state | `ORB-C2` |
| `conformance_c4_real_pty_reattach_converges_through_complete_ordered_frames` | Primary pending wrap, rich alternate screen, resize, detach under continuous output, responsive semantic interrupt, then split CSI, UTF-8, and APC tails | Reattachment starts at the complete current frame; later revisions strictly increase and converge on the final primary state | Exact reattached-frame equality and revision order; semantic final convergence | `ORB-C4` |
| `conformance_c5_authoritative_input_modes_and_resize_survive_detach` | Resize plus key, mouse, focus, paste, Kitty-keyboard, mode change, and reattach | The PTY receives terminal-state-aware input once and retains the requested dimensions | Exact PTY bytes and `stty` dimensions | `ORB-C5` |
| `conformance_c6_direct_rows_match_rich_canonical_frame_rows` | Wrapped styled text, RGB and palette colors, hyperlink, protection, combining and wide graphemes, viewport movement, and resize | Direct row extraction equals the corresponding canonical frame rows before and after reflow | Exact canonical-row equality; semantic presence of accepted rich fields and width roles | `ORB-C6` |
| `conformance_c8_authoritative_viewport_routes_wheel_and_key_from_terminal_state` | Preview and wheel at history edges, mouse tracking, alternate screen, synchronized output, semantic key, and pressure | Orbit returns typed terminal or viewport outcomes without forbidden mutation and routes exact terminal input when required | Exact typed outcomes, revisions, and PTY bytes; semantic no-mutation and boundedness | `ORB-C8` |
| `conformance_c8_signed_scroll_batch_is_atomic_bounded_and_authoritative` | One large signed commit, both history edges, lagging and future authority, routing changes, synchronized output, selection, and pressure | Orbit performs one native viewport movement and returns one result with exact requested/applied rows, one frame, and the next preview | Exact result count, rows, revision, adjacent preview, PTY absence, and failure non-mutation | `ORB-C8` |
| `conformance_c8_scroll_position_follows_published_terminal_state` | Output while anchored, alternate screen, reflow, actual byte-budget eviction, fresh attachment, live bottom, clear and reset | Complete frames carry authoritative display-row distance and history bounds at their own revision | Exact position/content assertions through the canonical session codec | `ORB-C4`, `ORB-C6`, `ORB-C8` |
| `conformance_c9_selection_copy_is_authoritative_bounded_and_client_scoped` | Wide and combining text, viewport movement, reversed selection, detach, alternate-screen output, and resize | Selected presentation is client-scoped while finished copy stays frozen across later terminal mutations | Exact copied Unicode text; semantic selection visibility, scope, and freeze | `ORB-C9` |

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
- A `Proved` contract names the exact accepted proof-bearing commit. A later
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
the sole authorized platform expansion; it proves Orbit alone, and Venus and
Eon have not adopted macOS.
