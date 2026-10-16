# Venus contract index

This is the canonical current state of Venus behavior, ownership, proof, and
remaining limitations. Beads and Git retain execution history; `CHANGELOG.md`
retains accepted user-visible chronology.

`VEN-C16` owns platform scope. Venus consumes ORBF v2 / ORBS v13 through the
in-tree `orbit-protocol` crate and canonical EONW v10 through the in-tree
`eon-workspace-protocol` crate. A protocol version named in a contract below
records when that behavior entered the wire; the current consumer is the
in-tree version.

## Proof environments

- **Automated:** x86_64 Linux; sandboxed `cargo test --locked --offline
  --workspace` with the packaged Venus fonts.conf. Native tests marked ignored
  do not run in this route.
- **Installed:** installed Eon
  `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0` and EonTerm
  `/nix/store/hk9v6q6c42x4sagpzrk20q32znw9sal1-eonterm-0.1.0`, driven
  headlessly by a Kinestra recipe on a private headless Sway (wlroots, pixman)
  on x86_64 Linux, through the `eon-monorepo-2oko.6` dogfood checklist and its
  recorded results.

## VEN-C19 — Startup typography and terminal geometry

- **Status:** Proved by the automated tier; installed configured typography and
  geometry are pending re-proof in this repository.
- **Consumer:** One direct or caller-launched Venus surface.
- **Trigger:** Optional `--font-family`, repeatable `--font-fallback`,
  `--font-size`, `--line-height`, `--columns`, and `--rows` startup options.
- **Result:** Installed primary monospace and ordered fallback families feed the
  existing shaper. One `CellMetrics` grid owns rendering, resize, pointer and
  selection geometry, scrolling, cursor, IME and accessibility. Family names
  are nonempty trimmed UTF-8 without controls, at most 128 bytes; at most eight
  fallbacks are accepted. Nominal font size is 6–96 logical px (default 16);
  line height is 1–3 times that size (default 1.125). Named overrides precede
  specialty/platform fallback. No font override preserves existing font choice
  and specialty handling; no overrides preserve 10 by 18 logical cells and the
  960 by 600 logical initial window.
- **Accelerando specialty font:** U+100000–U+101FFF route through the existing
  specialty path only when no font override is supplied.
- **Initial geometry:** Optional positive `u16` columns/rows request terminal
  dimensions, with workspace headers, stack bottom margin and popup margins
  supplied by the existing Scene owner. Omitted dimensions retain the existing
  initial window dimension.
  Requests must fit Orbit's 100,000-cell limit and native pixel/GPU bounds.
  Compositor sizing policy may override the request; actual resize remains
  authoritative. Later user resizing is not constrained to the initial grid.
- **Important failures:** Invalid values, duplicate singleton options, absent
  named families, a non-monospace primary, or impossible initial dimensions
  fail with bounded diagnostics before terminal presentation/attachment.
  Native output admission can require an initial window buffer to learn scale.
  Missing named fonts are not silently substituted. Availability does not
  promise every Unicode glyph or style variant. No live reload is provided.
- **Owner:** Venus launch, font fitter, `CellMetrics`, and workspace Scene;
  Orbit retains terminal authority and Eon retains product configuration.
- **Consumes:** ORBF v2 / ORBS v13, canonical EONW, and the selected glyphon
  0.12.0 / cosmic-text 0.19.0 APIs with exact unicode-script 0.5.8.
- **Supervised startup admission:**
  `EON_VENUS_PRESENTATION_CONTROL=stdin-ready-v1` uses private stdin/stdout.
  Workspace mode first reads one bounded canonical EONW Snapshot from stdin;
  standalone mode has no snapshot prefix. After font/renderer initialization
  and real-window initial native-scale admission, Venus writes exactly
  `ready-v1` (eight bytes) and starts attachment. The caller may then start
  Orbit and the user command. Invalid fonts, geometry or snapshot produce no
  readiness; the caller bounds startup and closes a failed attempt. Existing
  `present\n` and EOF control retain their meanings. Later user/compositor
  resizing is outside this initial admission. An empty popup-only tab admits
  usable body space without requiring or inventing an attachment.
- **Boundary:** The pinned winit destroys text input when a seat capability
  disappears; that hot-unplug limitation is outside this contract.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `typography_options_are_bounded_and_composable`,
    `startup_admission_keeps_presentation_control_enabled`,
    `configured_fonts_resolve_fallbacks_and_scale_one_grid`,
    `accelerando_specialty_routing_preserves_explicit_fonts_and_styles`,
    `ordered_terminal_fallbacks_preserve_readable_generic_ui_text`,
    `initial_grid_includes_workspace_chrome_at_each_scale`, and
    `startup_snapshot_leaves_following_presentation_control_unread` in the
    workspace suite; dogfood checklist items 1 and 7 (default terminal text,
    box drawing, Braille and CJK).
- **Open proof:** Pending re-proof in this repository: native startup admission
  and real PTY grids at fractional scales, configured typography and
  missing-font rejection in installed products, native IME and AT-SPI geometry,
  and installed Accelerando playback and its performance.

## Status

- **Planned:** accepted contract with no sufficient implementation evidence
- **Candidate:** implemented and verified without an accepted proof-bearing
  revision
- **Partially proved:** a useful exact slice is proved but required evidence is
  still open
- **Proved:** the listed immutable revision and evidence cover the contract
- **Retired:** explicitly replaced or removed; its ID is never reused

Proofs cite revisions of this repository. "Pending re-proof in this repository"
marks an earlier proof that was not repeated after consolidation; it is
re-proved when that platform or feature is next touched.

## VEN-C5 — Explicit native hyperlinks

- **Status:** Proved on x86_64 Linux by the automated tier; native hyperlink
  interaction and Apple Silicon macOS proof are pending re-proof in this
  repository.
- **Consumer/trigger:** A Venus user hovers an Orbit-authored OSC 8 link or
  explicitly activates one.
- **Result:** The hovered link is highlighted and its escaped target is shown
  with Open and Copy actions. Linux uses `Ctrl+click` and `Ctrl+Shift+C`; macOS
  uses `Command+click` and `Command+C`. Opening requires a press and release on
  the same current presented target. Copy uses the current hovered target unless
  terminal selection retains precedence. AccessKit exposes each current visible
  target as an Open link followed by a Copy button, both using the native Click
  action. Ordinary terminal clicks, input and selection retain their owners.
- **Failures:** Revision, attachment, geometry, presentation, focus, occlusion,
  or scroll changes retire link actions. A captured release without Ctrl is
  consumed without activation. Opening accepts at most 4096 bytes of ASCII
  HTTP/HTTPS URI syntax with a host and without credentials. Copy accepts bounded
  target text without control characters. Unsupported or malformed targets,
  missing native handlers, and launch failures/timeouts have bounded visual and
  accessible notices. No URI is opened on hover or keyboard focus.
- **Owners:** Orbit owns URI attributes; Scene derives spans; Venus presentation
  identity gates actions; the existing renderer, input, AccessKit status and
  native clipboard owners project them. Linux `gio open` on the host PATH
  dispatches the exact URI as one argument to the desktop's registered handler.
  One launch runs at a time with a ten-second dispatcher deadline.
- **Consumes:** Orbit `ORB-C6` through ORBF v2 / ORBS v13.
- **Boundary:** No heuristic detection, file/custom-scheme opening, URI rewrite,
  browser embedding, new crate, or protocol change. Host GIO and a registered
  handler are required for Linux opening; copy remains available without them.
  macOS uses fixed `/usr/bin/open -u` and the general pasteboard. Product-path
  accessibility activation remains unaccepted on macOS.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `hyperlink_actions_require_current_presentation_and_safe_exact_uri`,
    `stalled_link_dispatcher_is_bounded_and_reaped`,
    `hyperlinks_follow_visible_heads_and_wide_cells`,
    `visible_links_offer_only_open_and_copy_and_retire_with_the_scene`,
    `clipboard_failure_outranks_stale_link_notice`, and
    `macos_uses_command_for_standard_host_actions_only` in the workspace suite.
- **Open proof:** Pending re-proof in this repository: installed hyperlink open
  and copy (dogfood item 9 was not exercised), AT-SPI Open/Copy
  activation, native missing-handler failure, and the Apple Silicon Command+C copy and
  Command+click open. Fractional scale and other compositors remain unproved.

## VEN-C1 — Authoritative native presentation

- **Status:** Proven on x86_64 Linux; broader native quality remains unproved.
- **Consumer:** One Venus native surface consuming canonical Orbit frames and an
  optional cursor-tail color choice.
- **Trigger:** Venus accepts an initial frame, an ordered later revision, or a
  cursor-tail startup value.
- **Result:**
  - Venus validates and materializes one coherent Orbit-authored structured
    frame into native draw inputs without terminal authority.
  - An omitted color choice selects one of eight Venus presets once per launch
    and uses it for that process at internal fallback duration `1.0`. Eon owns
    product defaults and user configuration. One selector accepts
    `random`, `preset:<name>`, or `custom:#RRGGBB`; `none` is static, and the
    duration flag can override the default for any color choice. Durations must
    be finite multipliers from `0.25` through `4.0`; the complete lower-level
    `tail` profile supplies one validated color and duration.
  - A bounded four-corner trail approaches Orbit's exact cursor destination.
    Healthy pane/tab switches and workspace layout changes retain window-local
    cosmetic motion toward the newly selected pane's fresh cursor. Attachment
    gaps publish no old terminal content or trail and schedule no animation;
    only the bounded cosmetic position survives. First presentation, hidden
    cursors, invalid geometry, transport failure, focus loss/occlusion, native
    resize/scale changes, and GPU recovery retain safe reset behavior.
    Its solid fill receives a one-logical-pixel tint or shade with at least 3:1
    fill-to-outline contrast while preserving authoritative cursor shape,
    visibility, blink, and wide-cell geometry. The cursor body matches that
    selected color and receives the same shape-aware contrasting edge when
    Orbit supplies no cursor color; an explicit Orbit cursor color takes
    precedence without that edge. With `none`, the static body uses Orbit's
    color or foreground fallback.
  - The immutable Scene alone owns accessible text and selection. Each visible,
    nonblank canonical head cell is one selectable UTF-8 unit; text above the
    AccessKit 255-byte unit limit becomes one U+FFFD accessibility unit without
    changing visual or protocol text. Without selected cells, the authoritative
    viewport cursor supplies a collapsed accessible caret in those same atomic
    units. Trimmed padding stops before the row newline; an absent viewport
    cursor supplies no caret.
- **Important failures:** Invalid frames, topology, cursor profiles, renderer
  admission, focus, occlusion, or presentation failure cannot publish stale draw
  or accessibility state.
- **Owner:** Venus Scene, renderer, and cursor-animation state; Orbit retains
  terminal and cursor authority.
- **Consumes:** Orbit `ORB-C4` and `ORB-C6` through canonical ORBF v2 / ORBS v13
  and the in-tree `orbit-protocol` 0.1.0.
- **Boundary:** Fractional/HiDPI native quality remains outside the proof.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `canonical_orbit_frame_becomes_one_deterministic_scene`,
    `ordered_frames_reject_stale_revisions_without_replacing_state`,
    `invalid_session_and_frame_bytes_never_reach_draw_state`,
    `concealed_cells_stay_out_of_drawing_and_accessibility`,
    `block_cells_use_exact_cell_rectangles`,
    `half_blocks_preserve_clipped_preview_styles`,
    `cursor_spring_snaps_then_distinguishes_short_long_and_scaled_motion`,
    `cursor_outline_is_contrasting_and_one_logical_pixel`,
    `tail_cursor_outlines_one_color_and_resets_when_hidden_or_blinking_off`,
    `wide_cell_cursors_cover_the_complete_glyph`,
    `authoritative_selection_uses_accesskit_text_positions`,
    `text_geometry_preserves_atomic_cell_spans`, and
    `accessibility_projection_keeps_cells_atomic_and_marks_oversized_text` in
    the workspace suite; dogfood checklist items 3 and 7.
- **Open proof:** Pending re-proof in this repository: installed pointer
  selection and scrollback presentation (dogfood item 9), native cursor-tail
  continuity captures, Orca caret and line review (Orca releases before the
  [#757](https://gitlab.gnome.org/GNOME/orca/-/work_items/757) correction limit
  Say All),
  and half-block and cell-buffer performance measurements. Fractional scale and
  broader Linux compositor coverage remain outside the proof.

## VEN-C2 — Native semantic interaction

- **Status:** Proved on x86_64 Linux; Apple Silicon macOS proof is pending
  re-proof in this repository.
- **Consumer:** One focused Venus terminal surface.
- **Trigger:** Native key, mouse, focus, paste, input-method, or resize activity.
- **Result:** Venus maps native activity to canonical Orbit semantic input and
  resize messages using one shared scaled geometry owner. Compatible output
  advances render content without withdrawing the last presented input geometry.
- **Important failures:** Invalid, stale, unsettled, or non-presented geometry
  cannot reach Orbit; capture and focus transitions withdraw obsolete input.
- **Owner:** Venus native input and geometry owners; Orbit encodes terminal
  behavior.
- **Consumes:** Orbit `ORB-C5` through canonical ORBS v13.
- **Boundary:** Physical mixed-monitor hardware and broader Linux Wayland compositor
  behavior remain manual quality surfaces. On macOS, only VEN-C16's Kotoeri
  Roman-to-kanji IME path is accepted; held-key repeat, dead keys, and other IMEs
  remain unaccepted. Physical trackpad gesture quality remains with VEN-C7.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `maps_layout_independent_physical_keys`,
    `ime_distinguishes_committed_rejected_and_state_only_text`,
    `focus_loss_clears_transient_native_input_state`,
    `button_pairing_survives_presentation_and_queue_gates`,
    `pointer_coordinates_stay_inside_orbit_domain`,
    `output_keeps_presented_geometry_until_a_structural_transition`,
    `surface_measurements_match_orbit_invariants`,
    `ime_requires_current_attached_terminal_but_always_cleans_up`, and
    `orbit_focus_requires_window_and_terminal_focus` in the workspace suite;
    dogfood checklist items 1, 3 and 4 (Enter, Esc and Tab reached the picker).
- **Open proof:** Pending re-proof in this repository: installed pointer
  input, selection and paste (dogfood item 9), the ignored native
  input-between-frames regression, native IME preedit/commit, Orca reader focus
  after return to live, and the Apple Silicon key, focus, selection and Kotoeri
  observations.

## VEN-C3 — Transient client recovery

- **Status:** Proved
- **Consumer:** Standalone or Eon-supervised Venus attached to one selected live
  Orbit endpoint.
- **Trigger:** Client close or crash, retryable connection loss, control-stream
  loss, or replacement attachment.
- **Result:**
  - Client loss never owns or terminates the Orbit Session.
  - Reopening materializes the coherent current state.
  - An open client retries the same live endpoint with bounded backoff while
    retaining the last coherent scene.
  - In supervised mode, control loss exits Venus and a replacement may retry
    transient Busy while the departing attachment releases.
  - Endpoint replacement, non-live workspace state, successful attachment, or
    exit cancels obsolete retry state.
- **Important failures:** Standalone Busy and incompatible, exited, malformed,
  or replaced endpoints remain terminal rather than creating duplicate clients.
- **Owner:** Venus connection/retry and native event-loop lifecycle; Orbit owns
  Session survival.
- **Consumes:** Orbit `ORB-C1`, `ORB-C3`, `ORB-C4`, and `ORB-C7` through canonical
  ORBS v13.
- **Boundary:** Broader Linux compositor coverage remains unproved.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `orbit_retry_schedule_is_bounded_and_resettable`,
    `workspace_recovery_requires_the_same_selected_live_endpoint`,
    `only_supervised_orbit_busy_is_retryable`,
    `protocol_and_terminal_failures_do_not_become_socket_retries`,
    `retryable_loss_records_state_independently_of_retry_policy`,
    `reconnect_preserves_the_last_scene_and_accepts_a_fresh_revision`,
    `missing_and_dropped_sockets_can_attach_on_later_attempts`, and
    `typed_attachment_is_duplex_and_client_drop_leaves_server_listener` in the
    workspace suite; dogfood checklist items 2 and 11.

## VEN-C4 — Bounded explicit failure UX

- **Status:** Proved by the automated tier; native failure observations are
  pending re-proof in this repository.
- **Consumer:** A Venus user observing connection, protocol, model, renderer, or
  Orbit failure.
- **Trigger:** Attachment rejection, incompatibility, invalid frame, Orbit loss,
  renderer failure, or retry transition.
- **Result:** Venus exposes bounded truthful failure notices without inventing
  state. Retryable loss remains visible during backoff. Routine connection and
  first-frame progress is quiet inside an accepted workspace; standalone
  attachment retains progress messages. Actual attachment and rendering failures
  remain visible in both modes.
- **Important failures:** Standalone Busy is terminal; only supervised
  replacement retries it. Incompatible, Exited, model, queue, invalid-input,
  worker-start, and persistent-renderer failures remain terminal and withdraw
  presented input.
- **Owner:** Venus connection state, notice projection, and renderer lifecycle.
- **Consumes:** Orbit `ORB-C3`, `ORB-C4`, and `ORB-C7` through canonical ORBS v13.
- **Boundary:** Compositor title-bar visibility and broader Linux compositor
  proof remain outside the accepted slice.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `renderer_failure_uses_non_gpu_diagnostics_until_success`,
    `uncaptured_errors_remain_fatal_until_device_loss`,
    `device_loss_blocks_surface_recovery_but_destroy_does_not`,
    `attachment_and_server_failures_are_explicit_and_bounded`,
    `notices_recover_independently_by_source`,
    `local_failures_name_their_owner`,
    `scene_notice_preserves_terminal_content_and_adds_an_alert`, and
    `quota_refresh_and_workspace_failures_preserve_structure` in the workspace
    suite.
- **Open proof:** Pending re-proof in this repository: native quiet-workspace
  and offline-failure observation, native title, and reader announcement of
  workspace action failures.

## VEN-C7 — Authoritative history and selection interaction

- **Status:** Proved on x86_64 Linux by the automated tier; native pointer
  interaction and Apple Silicon macOS proof are pending re-proof in this
  repository.
- **Consumer:** One presented Venus terminal surface.
- **Trigger:** Native wheel or touchpad movement, one left-pointer sequence
  including a held drag in the top visible row, explicit copy, or activation of
  the visible scrollback pill.
- **Result:**
  - While away from live output, Venus displays `↓ N rows` (`↓ 1 row` for one)
    from the accepted frame's authoritative wrapped display-row distance. Live
    bottom, alternate screen, recovery and known terminal-owned routing hide it.
    Activating the whole visible pill requests one semantic return to live;
    Orbit alone moves the viewport and publishes its new position. An
    unpresented, hidden, or occluded pill is inert.
    Idle preview cleanup and rejected input preserve routing evidence until a
    fresh frame or preview replaces it.
    Preview movement never changes the count; accessibility exposes committed
    position as a description and the visible return action as a button, not an
    alert. The selected pane header
    reserves space for the count and fits its directory label into the remaining
    width using the existing cluster-safe header fitter. Standalone and picker
    terminals use a small top-right overlay that preserves grid dimensions and yields to selection,
    links, notices, picker tab previews and an overlapping terminal cursor.
    Pending reflow hides the old count;
    labels that cannot fit in full remain accessible without clipped digits.
  - Precision movement translates and clips an accepted complete Orbit frame
    plus its bounded revision-bound row window at two presented pixels per
    native input pixel.
  - Venus retains a sub-row remainder, commits crossed rows through at most one
    coalesced signed Orbit batch, and installs only its atomic authoritative
    frame and next preview.
  - Lagging relative preview and commit revisions remain admissible while PTY
    output advances; after history entry, later output leaves the authoritative
    viewport pinned and further relative movement remains available.
  - When a later output frame preserves screen, geometry, default cell colors,
    and palette while fractional movement remains, Venus keeps only that
    bounded revision-bound preview until Orbit supplies its replacement instead
    of presenting the remainder as zero or freezing finger movement. Every
    plain frame retires transient terminal-routing evidence.
  - A phase-complete precision gesture may continue with bounded elapsed-time
    exponential decay; discrete wheel input moves three rows per logical step
    without synthetic momentum.
  - A transient presentation timeout preserves gesture estimation, kinetic
    velocity, and distance; an occluded surface still cancels them.
  - Ordinary output preserving terminal dimensions and active screen keeps
    wheel start/movement/end and pointer phases admissible between repaints.
    Input carries an actually presented revision; render caching still advances
    on each accepted frame. Orbit keeps parsing and owns anchored history.
  - Redraw follows native compositor callbacks.
  - Venus sends every left-pointer phase to Orbit. Orbit routes uncaptured
    input to cell selection, words at two clicks, space/tab-delimited spans at
    three clicks, and logical lines at four or later clicks. Orbit preserves
    native repeat timing, distance and drag behavior, terminal mouse capture,
    and Shift as a host-selection override.
  - While a held host selection stays in the top visible row, Venus sends one
    upward selection tick at a time. Orbit moves at most one history row per
    tick and extends the same gesture; Venus waits for the accepted frame
    before the next tick and stops at the oldest edge or on release.
  - Pointer phases and explicit copy that arrive before Orbit completes the
    preceding sequence retain their order. Venus resumes them only after
    presenting the authoritative completion revision reported by Orbit.
  - A successful selection release writes Orbit's frozen text to both the
    ordinary Wayland clipboard and primary selection. `Ctrl+Shift+C` remains an
    explicit ordinary-clipboard copy. On macOS, every destination maps to the
    general pasteboard.
- **Important failures:** Future authority, stale coordinate- or phase-bound
  input, failed gesture admission, resize, capture loss, lifecycle change,
  terminal-owned routing, history edge, or Orbit rejection cancels synthetic
  motion and cannot fabricate cells, selection, copied text, or viewport state.
  Pointer/focus loss, stale presentation, attachment change, and reflow stop
  selection ticks; invalid synthetic ticks keep the native release without a
  routine error notice. Terminal and protocol failures remain visible.
  A change to screen, geometry, default cell colors, or palette retires the
  bounded preview before presentation. An unsolicited or mismatched scroll
  outcome is a protocol-order failure.
- **Owner:** Venus owns native fractional presentation, bounded kinetic state,
  gesture cancellation, and clipboard effects; Orbit alone owns history,
  viewport movement, routing, cells, revisions, selection, and copied text.
- **Consumes:** Orbit `ORB-C4`, `ORB-C5`, `ORB-C6`, `ORB-C8`, and `ORB-C9`
  through canonical ORBF v2 / ORBS v13; the workspace's patched winit 0.30.13,
  wgpu 30.0.0, and glyphon 0.12.0.
- **Boundary:** Client-owned history caches, bounce, device/source heuristics,
  public physics tuning, presentation feedback, unreleased winit, GPU-layer
  translation, and additional platform support are outside this contract.
  Apple Silicon physical trackpad gesture quality remains unaccepted.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `terminal_scroll_keeps_fractional_input_and_elapsed_time_physics`,
    `terminal_scroll_coalesces_one_signed_batch_without_losing_distance`,
    `scrollback_pill_activates_only_a_complete_left_click`,
    `held_top_edge_waits_for_each_authoritative_frame_and_stops`,
    `rejected_selection_tick_preserves_wheel_input_after_release`,
    `deferred_selection_preserves_order_bounds_and_cancel_ownership`,
    `selection_gate_waits_for_finish_and_its_authoritative_presentation`,
    `copy_is_revision_free_but_waits_for_selection_finish`,
    `clipboard_effects_preserve_native_targets`,
    `wheel_accumulates_fractional_deltas_caps_bursts_and_resets`,
    `left_drag_and_copy_are_explicit_revision_bound_actions`,
    `bounded_preview_rows_cover_multi_row_fractional_offsets`,
    `revision_bound_scroll_previews_follow_atomic_frames`,
    `continuous_frames_keep_only_compatible_scroll_previews`,
    `canonical_scroll_outcomes_reuse_the_existing_result_and_frame_owners`,
    `scrollback_label_follows_only_current_authoritative_frames`, and
    `scrollback_is_a_terminal_description_without_live_alerts` in the
    workspace suite.
- **Open proof:** Pending re-proof in this repository: installed scrollback,
  ReturnToLive, click-selection ladder and copy (dogfood item 9), held-drag
  selection across real history, primary-selection delivery, kinetic flings under
  continuous output, and the Apple Silicon drag selection and pasteboard copy.
  Screen-reader acceptance remains separate. Apple Silicon trackpad gesture quality stays open.

## VEN-C8 — Eon workspace presentation

- **Status:** Partially proved
- **Consumer:** One Venus surface controlled by an Eon workspace.
- **Trigger:** Outside visible `VEN-C18` popup presentation, Eon supplies
  durable topology or a workspace action changes tab, pane, focus, order,
  lifetime, liveness, endpoint, or metadata.
- **Result:**
  - Venus materializes ordered horizontal tabs and every fitting header for the
    active tab around exactly one expanded pane. Each tab shows its current
    one-based position plus the directory leaf or `/` derived from Eon's
    authoritative launch directory; positions update when Eon reorders tabs.
  - Tabs have pill-shaped corners and separated hit targets. Width follows shaped
    label text plus padding, capped near 280 logical pixels at default
    typography. Long labels use a cluster-safe middle ellipsis; when even that
    cannot fit, preserve the positional number whenever it fits. Overflow stays
    horizontally scrollable across the whole strip, including its gaps, and
    selection reveals the active tab. Hover shows
    its control-sanitized launch path within available window space; the
    accessible name retains that path. Selection and keyboard focus remain
    distinct: selected fill and brighter text identify the active tab without
    an underline; keyboard focus adds a rounded outline. Pane shell navigation
    does not rename or resize tabs.
  - Every visible live pane endpoint has one bounded read-only metadata observer.
  - Pane frames connect the visible accordion within one thin rounded outer
    border, with horizontal separators reaching its sides and a small gap below
    the stack (4 logical pixels at default typography). The gap shares terminal
    background color/opacity and the surface-wide blur request. Selected and
    hovered header fills follow the stack's corners, with square internal edges.
    Separators follow the rounded corners during scrolling;
    the shared border stays fixed while overflowing headers scroll within it.
    `--pane-frames true|false` defaults to `true`; `false` removes the shared
    border and separators while retaining headers, a lighter selected fill,
    hover feedback and a distinct rounded keyboard-focus outline. Both modes
    preserve the same terminal grid, header hit targets and AccessKit bounds.
    Initial requested grids include the bottom gap in their window overhead;
    the gap accepts no pane or terminal input.
    Changed pane lists resize the terminal even when the endpoint stays selected.
    The option has no visual effect in standalone or popup presentation.
    Missing, duplicate or invalid values fail before window creation. Eon owns
    persistent configuration and composed-launch policy.
  - Headers and AccessKit names show the opaque `pN` identity, two ASCII spaces,
    and one compact working-directory label.
  - Home displays `~`; descendants use `~/`; paths outside
    home remain absolute; overlong paths elide from the left to preserve the
    leaf; an unset or empty `HOME` keeps absolute paths.
  - Terminal title remains available to the selected window and is not repeated
    in pane chrome. Empty, offline, unavailable, or incompatible metadata falls
    back to pane identity without exposing mapped Session identity.
  - Hidden tabs have no observers, obsolete observations retire, and a 250 ms
    reinspection exposes accepted external workspace changes without native
    input.
  - Pointer input, Alt+H/L tab traversal, Alt+K/J pane traversal, Alt+M pane
    creation, Alt+Shift+T tab requests, focused arrow traversal, and
    selected-terminal attachment retain their existing owners. Alt+H/L remains
    admitted while a popup is visible; inactive popup selections do not replace
    the active tab's projection. Popup-only tabs may have no pane or selected
    pane. Hiding their last visible popup yields an empty body with actionable
    tabs and catalog shortcuts (VEN-C18).
  - Physical Alt+1 through Alt+9 focus the corresponding current tab position;
    Alt+0 focuses position 10. Venus resolves the position from the accepted
    snapshot and sends the existing stable `tN` through `FocusId`. A missing
    position is consumed without an action or terminal input. Alt+H/L remains
    the traversal path for tabs after position 10.
  - Ctrl+Alt+H/L sends one non-repeating semantic move for the active tab;
    Ctrl+Alt+K/J does the same for its selected pane. Alt+Shift+W sends one
    non-repeating close naming the snapshot's active stable `tN`. Venus consumes
    each exact press and matching release without forwarding either to the
    selected terminal. Ctrl+T, Ctrl+Shift+W, and lowercase Ctrl+W remain terminal
    input, and none of these shortcuts enters a mode.
  - Compatible terminal output does not withdraw tab/pane focus actions or
    presented header hit targets while a repaint is pending.
  - Hit testing, AccessKit node identity, and actions retain the exact `tN`.
    AccessKit names pair the current position with a bounded full launch path,
    so duplicate leaves remain distinguishable without storing another tab
    name.
- **Important failures:** Workspace loss, Orbit exit, endpoint replacement,
  liveness change, rejected close or movement, or incompatible metadata retires
  stale observations and never grants Venus topology or Session-lifecycle
  ownership. Repeats create no second structural action. Rejected-action notices
  remain available through unchanged and quota-only refreshes until workspace
  structure changes. Connection-unavailable notices clear on a successful
  snapshot. Terminal-status alerts announce their actual error without moving
  focus; routine polling does not repeat the announcement.
- **Owner:** Venus workspace materialization, bounded metadata observation,
  clipping, and accessibility projection; Eon owns topology and Orbit owns
  terminal metadata and Session lifetime.
- **Consumes:** Eon `EON-C10`, `EON-C17`, `EON-C18`, canonical EONW through the
  in-tree `eon-workspace-protocol` 0.1.0, and Orbit metadata observation in
  ORBS v13 through the in-tree `orbit-protocol` 0.1.0.
- **Boundary:** Other Linux Wayland compositors remain outside the current
  proof.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `eon_workspace_becomes_one_bounded_native_accordion`,
    `tab_directory_labels_use_current_position_and_bounded_path_context`,
    `pane_metadata_label_is_bounded_safe_and_has_honest_fallbacks`,
    `tab_text_fits_shaped_width_without_splitting_combining_clusters`,
    `rounded_outline_keeps_corners_and_interior_clear_when_clipped`,
    `metadata_endpoints_are_only_live_panes_in_the_active_tab`,
    `metadata_observer_is_read_only_latest_only_and_releases_on_drop`,
    `direct_workspace_shortcuts_use_stable_eon_actions`,
    `alt_digits_resolve_current_tab_positions_without_leaking_missing_targets`,
    `structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit`,
    `workspace_chrome_preserves_terminal_release_pairing`,
    `workspace_accessibility_uses_visible_intersections`,
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`,
    `workspace_nodes_keep_identity_and_retire_removed_actions`,
    `pane_frame_option_is_a_strict_single_boolean`,
    `workspace_worker_refreshes_without_a_queued_action`,
    `incompatible_workspace_response_is_rejected_before_model_state`, and
    `quota_refresh_and_workspace_failures_preserve_structure` in the workspace
    suite; dogfood checklist items 3, 4 and 10.
- **Open proof:** Fractional native scale remains to be dogfooded. Pending
  re-proof in this repository: `--pane-frames false` and pane-stack pixel
  checks, the ignored native long-label header regression, and reader
  announcement of a workspace failure notice.

## VEN-C9 — Optional native decorations

- **Status:** Proved by the automated tier; native decoration rendering is
  pending re-proof in this repository.
- **Consumer:** A user launching Venus.
- **Trigger:** Launch with or without one `--no-decorations` option.
- **Result:** The option requests an undecorated native window before creation;
  omission retains decorations.
- **Important failures:** Unknown, duplicate, or excess arguments fail with
  bounded usage before window or transport creation.
- **Owner:** Venus CLI admission and winit native window attributes.
- **Consumes:** Exact locked winit 0.30.13; no Orbit or Eon protocol boundary.
- **Boundary:** Runtime decoration changes are outside this launch-only contract.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `arguments_are_complete_bounded_and_defaulted` and
    `launch_values_project_to_native_window_attributes` in the workspace suite.
- **Open proof:** Pending re-proof in this repository: installed
  `--no-decorations` rendering under a decorating compositor; dogfood item 11
  started it but did not judge decorations.

## VEN-C10 — Native paste

- **Status:** Partially proved
- **Consumer:** One attached focused Venus terminal surface.
- **Trigger:** Native Paste, Linux Ctrl+Shift+V, or macOS Command+V.
- **Result:** Venus reads ordinary native clipboard text once, admits at most
  1 MiB, and submits exactly one canonical semantic paste to Orbit; Orbit alone
  owns normal and bracketed terminal encoding.
- **Important failures:** Oversized, invalid, unavailable, unfocused, detached,
  or overlapping shortcut input fails visibly without duplicate submission.
- **Owner:** Venus shortcut precedence, native clipboard read, admission bound,
  and visible failure.
- **Consumes:** Orbit `ORB-C5` and canonical semantic paste, exact winit 0.30.13,
  and the selected native clipboard owner: wl-clipboard-rs 0.9.3 on Linux or
  target-only arboard 3.6.1 on macOS.
- **Boundary:** A hardware Paste key and native Wayland without data-control
  remain unproved. A macOS host-access failure notice and overlap behavior
  remain unaccepted.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `native_clipboard_text_becomes_one_bounded_semantic_paste`,
    `key_release_does_not_dismiss_clipboard_result`,
    `clipboard_results_are_attributed_without_copying_terminal_cells`,
    `recognized_shortcuts_are_one_shot`, and
    `macos_uses_command_for_standard_host_actions_only` in the workspace suite.
- **Open proof:** Manual Zellij, Helix, and Yazi acceptance remains assigned to
  the user. Pending re-proof in this repository: installed paste (dogfood item
  9), native bracketed-mode and
  overlapping-shortcut observation, and the Apple Silicon Command+V, paste
  failure-notice and empty-pasteboard observations.

## VEN-C11 — Terminal-background opacity

- **Status:** Proved
- **Consumer:** One Venus native surface.
- **Trigger:** Launch with an optional finite `--background-opacity VALUE` in
  `0.0..=1.0`.
- **Result:** Omission equals `1.0`; the value applies only to terminal default
  background, later Orbit-authored background changes, and terminal padding.
  Explicit cells, selection, inverse video, workspace chrome, notices, focus,
  foreground effects, input, hit testing, and accessibility remain unchanged.
- **Important failures:** Invalid values fail before window creation; values
  below `1.0` require a proved premultiplied surface and otherwise fail before
  presentation.
- **Owner:** Venus launch policy and renderer surface composition.
- **Consumes:** Exact winit 0.30.13, wgpu 30.0.0, glyphon 0.12.0, canonical
  ORBS, and canonical EONW.
- **Boundary:** Broader Linux compositor proof remains open.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `background_opacity_selects_only_proved_surface_modes`,
    `background_opacity_premultiplies_clear_colors`,
    `background_opacity_skips_only_default_cell_backgrounds`,
    `background_opacity_provenance_distinguishes_default_explicit_selection_and_inverse`,
    and `arguments_are_complete_bounded_and_defaulted` in the workspace suite;
    dogfood checklist item 8.

## VEN-C13 — Terminal-authored clipboard delivery

- **Status:** Partially proved
- **Consumer:** One attached Venus client and its native clipboard owner.
- **Trigger:** Orbit emits one canonical terminal clipboard-write effect.
- **Result:** Venus delivers it once without storing, replaying, parsing, or
  reconstructing terminal text; Linux standard maps to ordinary clipboard and
  selection or primary maps to primary selection. macOS maps every destination
  to the general pasteboard.
- **Important failures:** Pre-attachment effect, native failure, invalid
  destination, or Orbit rejection remains bounded and visible without changing
  Scene state or selection-copy behavior.
- **Owner:** Venus native clipboard effect delivery; Orbit owns terminal
  interpretation and text.
- **Consumes:** Orbit `ORB-C11` through canonical ORBS v13 and the accepted
  Linux native clipboard owner; target-only arboard 3.6.1 on macOS.
- **Boundary:** Ordinary clipboard delivery, Wayland without data-control, and
  broader Linux compositor coverage remain unproved. macOS primary-selection
  emulation stays outside scope, and native write-failure UX remains unaccepted.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `clipboard_text_is_an_attached_one_shot_effect_not_presentation_state`
    and `clipboard_effects_preserve_native_targets` in the workspace suite.
- **Open proof:** Pending re-proof in this repository: native primary-selection
  delivery through installed Eon and the Apple Silicon OSC 52 pasteboard write.

## VEN-C14 — Supervised native presentation lifecycle

- **Status:** Proved
- **Consumer:** One Eon-supervised Venus process.
- **Trigger:** A complete private Present command, control EOF, or terminal read
  failure.
- **Result:** Present preserves process and terminal attachment while requesting
  presentation of the existing native window; control loss exits through the
  native event loop, releases attachments, and leaves Orbit live; replacement
  may retry transient Busy while the old client releases.
- **Important failures:** Malformed or partial commands do not present; standalone
  Busy remains terminal; Present followed by EOF preserves exact Present-then-
  Exit ordering.
- **Owner:** Venus supervised control stream and native event-loop lifecycle.
- **Consumes:** Eon `EON-C11`, one bounded private byte stream, Unix EOF, and
  exact winit 0.30.13 event/exit APIs.
- **Boundary:** Direct focus and unminimize are unavailable through winit;
  xdg activation remains compositor-controlled.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `presentation_control_emits_complete_commands_then_exit`,
    `startup_admission_keeps_presentation_control_enabled`,
    `startup_snapshot_leaves_following_presentation_control_unread`, and
    `only_supervised_orbit_busy_is_retryable` in the workspace suite; dogfood
    checklist items 1 and 2.
- **Open proof:** Pending re-proof in this repository: native Present of an
  existing window that preserves its process and terminal attachment.

## VEN-C15 — Compositor-owned background blur

- **Status:** Proved by the automated tier; the native compositor backdrop is
  pending re-proof in this repository.
- **Consumer:** One Venus native Wayland surface.
- **Trigger:** Launch with an optional `--background-blur` flag.
- **Result:** Before showing the window, Venus asks its native host for full-
  surface compositor blur; omission makes no request. Translucent terminal
  pixels reveal compositor blur while explicit cells, selection, inverse,
  chrome, notices, focus, text, cursor, input, IME, and accessibility retain
  their owners. The compositor owns capability, algorithm, strength, and policy.
- **Important failures:** Unsupported or policy-disabled compositors remain best
  effort without failing launch; duplicate, unknown, or excess arguments fail
  before window creation.
- **Owner:** Venus owns one boolean request, winit owns native protocol lifecycle,
  and the compositor owns the effect.
- **Consumes:** The workspace's patched winit 0.30.13, which carries the
  backport of upstream winit `c4afadbfabf7b1e7989b40b493db1a4c7bd8ff4e`;
  unchanged `VEN-C11`, canonical ORBS, and canonical EONW.
- **Boundary:** Stable crates.io winit lacks the mechanism; non-COSMIC Linux
  compositors remain unproved; no API reports capability or compositor
  acceptance or controls strength.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `arguments_are_complete_bounded_and_defaulted` and
    `launch_values_project_to_native_window_attributes` in the workspace suite.
- **Open proof:** Pending re-proof in this repository: the installed backdrop
  on a blur-capable compositor such as COSMIC (headless Sway has no blur
  protocol), and blur off/on comparison across explicit opacity values.

## VEN-C16 — Native Linux Wayland and Apple Silicon macOS platforms

- **Status:** Proved for x86_64 Linux native Wayland; Apple Silicon macOS proofs
  are pending re-proof in this repository, and macOS remains unsupported.
- **Consumer:** Every packaged Venus application launch.
- **Trigger:** Build or launch the native application on an approved target.
- **Result:** Venus uses native Wayland with Vulkan on Linux and the native
  AppKit window lifecycle with Metal on `aarch64-darwin`. Both
  targets share protocol validation, model, scene, draw inputs, renderer,
  accessibility tree, semantic actions, and workspace composition; only native
  host mechanics vary.
- **Important failures:** An unavailable native host or graphics backend,
  invalid platform mapping, or unsupported target fails before presentation
  attachment with a bounded explicit error. Evaluation or compilation alone is
  not native runtime proof.
- **Owner:** Venus native host and dependency feature selection.
- **Consumes:** User-approved `EON-C7`; the workspace's patched winit 0.30.13,
  wgpu 30.0.0, and wl-clipboard-rs 0.9.3 with ORBF v2 / ORBS v13 and canonical
  EONW. The macOS path consumes Orbit's `ORB-C14` platform seam.
  Target-scoped AppKit/AccessKit and Metal features were selected by
  `ven-qualify-venus-stack-apple-silicon-macos-ili`; the interaction slice adds
  target-only arboard 3.6.1 for general-pasteboard text.
- **Boundary:** X11, Xwayland, Intel macOS, signing, notarization, packaging,
  distribution, and other native platforms remain unsupported. Apple Silicon
  IME behavior beyond the Kotoeri Roman-to-kanji path, held-key repeat,
  physical trackpad gesture quality, a host-access failure notice,
  accessibility, lifecycle identity, effects, and Eon composition remain
  unaccepted.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf` (x86_64 Linux)
  - **Environment:** Automated and installed.
  - **Evidence:** The workspace suite with exact dependency features from the
    locked graph; dogfood checklist items 1 and 11 launch the native Wayland
    and Vulkan event loop through installed Eon and EonTerm.
- **Open proof:** Pending re-proof in this repository: the no-Wayland failure
  observation, and every Apple Silicon proof (the opaque AppKit/AccessKit/Metal
  foundation, the native interaction partial proof, and Kotoeri composition).
  `ven-prove-venus-apple-silicon-macos-nm7` owns the macOS proof: exact
  hardware, OS, toolchain, artifacts, automated checks, interactive
  observations, failures, and limitations. The remaining native interactions,
  accessibility acceptance, lifecycle identity, transparency/blur, exact Orbit
  dogfood, and support claim stay open.

## VEN-C17 — Caller-owned native application identity

- **Status:** Proved
- **Consumer:** Eon, EonTerm, or one approved EonTerm composition.
- **Trigger:** The caller supplies one optional validated application ID before
  Venus creates its window.
- **Result:** The accepted value becomes winit's Wayland general name and native
  xdg-toplevel app ID; omission remains `eon`; Orbit-authored titles remain
  independent presentation state.
- **Important failures:** Empty, non-UTF-8, oversized, or non-token values fail
  before window creation; Venus never infers identity from child argv,
  environment, titles, executable names, runtime paths, or desktop files.
- **Owner:** The caller owns identity choice and desktop metadata; Venus owns
  direct CLI validation and native Wayland materialization.
- **Consumes:** The workspace's patched winit 0.30.13; Eon owns the selected
  value.
- **Boundary:** Native mapping only; no branding framework or mutable identity.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `arguments_are_complete_bounded_and_defaulted` and
    `launch_values_project_to_native_window_attributes` in the workspace suite;
    dogfood checklist item 1, whose launch waited for the Sway tree to show the
    native app ID `eon` that Eon supplies through `--application-id`.
- **Open proof:** Pending re-proof in this repository: native xdg-toplevel
  grouping for a caller-supplied application ID.

## VEN-C18 — Shared popup and picker-first presentation

- **Status:** Proven on x86_64 Linux Wayland
- **Consumer:** One person using a Venus workspace supplied through EONW v5.
- **Trigger:** The active tab selects a popup, its selection changes, or the
  person invokes an Eon-supplied catalog shortcut.
- **Result:** Workspace-only startup waits for a canonical snapshot before
  attaching its sole terminal endpoint. A selected popup replaces the entire
  pane stack while tabs remain visible and actionable. Each inactive tab's
  selection and hidden popup instances remain Eon-owned. A popup-only tab may
  have an empty body: no placeholder pane, attachment, resize failure, or alert.
  Its selected tab remains the only keyboard and accessibility focus target;
  reopening work restores terminal focus.
  A catalog shortcut names the exact tab, entry and current instance, with
  Toggle intent from terminal focus and Focus intent from workspace chrome.
  Structural repeats remain consumed without sending duplicate actions.
  Tab traversal and structural actions use the existing semantic workspace
  path; Eon decides their admission. F6 cycles visible focus regions, and
  Escape returns chrome focus to the terminal. While the terminal is focused,
  Escape, Ctrl+C, Tab and Enter reach its application through ordinary Orbit
  input. Popup entry labels also name the accessible tab panel.
- **v7 expansion:** Venus accepts multiple pending tabs and their independent
  Project popup endpoints from one canonical snapshot. It attaches only the
  active tab's selected picker and preserves the others across tab switches.
  New tab and Alt+Shift+W retain the existing semantic CreateTab and
  exact-active-tab CloseTab actions. No local cancellation state or native
  picker is added.
- **Geometry:** A selected popup reuses the pane stack's exact outer rounded
  frame, so toggling preserves every chrome edge. Shared Eon-supplied logical
  margins inset the popup terminal content within that stable shell. A compact
  border label keeps a small top gutter before terminal padding. Scene owns
  both rectangles: chrome drives the outline, while terminal drives resize,
  clipping, pointer input, IME and accessibility. Margins and the title gutter
  shrink to preserve a usable cell grid; tiny surfaces omit the label. Exposed
  margins share the terminal default background and opacity, with the existing
  best-effort surface-wide compositor blur request. Pane-frame configuration
  has no effect on the popup outline.
- **Important failures:** Hidden or replaced endpoints retain no active
  presentation or input. Stale popup actions carry exact instance guards;
  canonical EONW validation and failure responses preserve the last coherent
  snapshot. Invalid frames, aliased endpoints, missing selections and malformed
  catalog values fail in the owner codec. Unavailable attachments retain the
  existing bounded recovery/failure path. Standalone Venus preserves raw
  workspace and catalog keys. Invalid or mixed launch forms fail before a
  window or transport starts.
- **Owner:** Venus owns native projection, shortcut dispatch and focus. Eon owns
  catalog, geometry settings, commands, cwd, popup/tab lifecycle, selection,
  action admission and cleanup. Orbit owns terminal Sessions and PTYs.
- **Consumes:** Canonical EONW through the in-tree `eon-workspace-protocol`
  0.1.0; ORBF v2 / ORBS v13 through the in-tree `orbit-protocol`.
- **Boundary:** No local command launch, chooser-mode recognition, cwd or
  dismissal policy, second schema, simultaneous terminals, compatibility
  negotiation, Eon runtime activation, or additional platform.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `shared_popup_projection_keeps_hidden_work_and_one_input_surface`,
    `popup_outer_chrome_reuses_the_pane_stack_rectangle`,
    `popup_and_empty_body_expose_only_visible_accessibility_targets`,
    `independent_pending_picker_tabs_attach_only_the_active_endpoint`,
    `workspace_worker_carries_actions_and_popup_transition`, and
    `structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit` in the
    workspace suite; dogfood checklist items 1, 3 and 4.
- **Open proof:** Pending re-proof in this repository: installed v7
  multi-pending tabs, narrow and tiny popup geometry, and supervised empty-body
  startup and reopen. Native fractional
  scale, other compositors and screen-reader interaction remain unproved.

## VEN-C20 — Native shortcut viewer

- **Status:** Proved
- **Consumer:** One person using a Venus surface attached to an Eon workspace.
- **Trigger:** Physical Alt+Slash toggles the viewer; Escape closes it.
- **Result:** Venus presents one ephemeral, read-only native dialog containing
  grouped native workspace, host, and current Eon popup shortcuts. Fixed
  bindings share their descriptor owner with dispatch; popup rows use the exact
  enabled label and normalized physical shortcut from the current Eon
  snapshot. Wheel and navigation keys scroll bounded content. Opening,
  refreshing, scrolling, and closing send no terminal bytes or workspace
  action; matching releases remain captured and prior terminal/chrome focus is
  restored on dismissal. The title, subtitle, and first group heading occupy
  separate readable vertical space.
- **Important failures:** Repeated toggle events do not oscillate the viewer.
  Workspace loss, empty or maximum-size catalogs, long labels, resize, scale,
  focus loss, and tiny surfaces retain local dismissal and clipped, scrollable
  presentation. While open, keyboard, pointer, IME, clipboard, link, and
  workspace actions do not reach their normal targets.
- **Owner:** Venus owns viewer state, fixed native binding descriptors, scene,
  rendering, input precedence, and AccessKit projection. Eon owns the enabled
  popup catalog and must reserve Alt+Slash before full-product delivery.
- **Consumes:** Canonical EONW through the in-tree `eon-workspace-protocol`
  0.1.0.
- **Boundary:** No Orbit Session, Eon configuration parsing, child-application
  shortcuts, action execution from rows, search, filtering, rebinding,
  tutorial, telemetry, persistence, momentary timer, second renderer, generic
  modal framework, dependency, or platform-support expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `shortcut_viewer_uses_live_rows_and_captures_its_local_sequence`,
    `shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space`, and
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog` in the
    workspace suite; dogfood checklist item 5.
- **Open proof:** Pending re-proof in this repository: AT-SPI dialog traversal
  and tiny-surface clipping.

## VEN-C21 — Native Eon Bar workspace header

- **Status:** Proven
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace.
- **Trigger:** The person points at, focuses, or activates the workspace header;
  uses New tab, Keyboard shortcuts, or active-tab Close; scrolls the tab region;
  or left-presses its empty drag region.
- **Result:** The existing workspace tab strip remains one row at its existing
  height. Scrollable tabs keep the leading space and retain their labels,
  overflow, reveal, selection, hover path, stable identity, and accessibility.
  New tab follows the rightmost tab when tabs fit and stays at the tab-region
  edge during overflow. Keyboard shortcuts (when VEN-C24 requires the fallback)
  and active-tab Close remain trailing
  controls. The buttons use the existing CreateTab and exact-active-tab CloseTab
  EONW actions and the existing local VEN-C20 viewer. Their tooltips and
  accessible names are
  `New tab — Alt+Shift+T`, `Keyboard shortcuts — Alt+/`, and
  `Close tab — Alt+Shift+W`. A bounded empty region between New tab and the
  trailing controls starts compositor-owned window movement only from its
  immediate left press.
  The accessibility tree orders tabs, New tab, optional quota, Shortcuts,
  then Close tab.
  It yields before tabs or controls when width contracts. Tab and Shift+Tab
  traverse focused header controls; Enter and Space activate them. Hover,
  pressed, and focus states remain distinct, and focus includes an outline.
- **Quota placement:** The optional quota chip sits in the slot New tab
  vacates, adjacent to Shortcuts, so the blank drag region stays continuous.
- **Important failures:** Buttons, tab gaps, pane chrome, terminal content,
  popup content, tooltips, and the shortcut viewer never become drag targets.
  Narrow surfaces preserve an active-tab region and all visible controls without
  overlap. A failed native drag is consumed without workspace action, terminal
  input, crash, or state mutation. Existing stale-presentation and stable-target
  gates continue to reject obsolete actions.
- **Owner:** Venus Scene owns header geometry and hit identity; the renderer
  owns its native presentation; the application owns input, local viewer state,
  and action routing; AccessKit projects the same rectangles and identities.
  Eon retains workspace topology and action-policy ownership; winit and the
  compositor own native window movement. VEN-C24 owns conditional Shortcuts
  visibility.
- **Consumes:** VEN-C8, VEN-C20, canonical EONW, ORBF v2 / ORBS v13, and the
  workspace's patched winit 0.30.13.
- **Boundary:** No second row, widget framework, status/quota data, polling,
  configuration, Nova compatibility, inactive-tab close, protocol change,
  dependency, native-decoration policy, or platform-support expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `eon_bar_controls_reuse_existing_actions_and_local_viewer`,
    `eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure`,
    `eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation`,
    and `workspace_accessibility_order_matches_tabs_then_one_expanded_pane` in
    the workspace suite; dogfood checklist items 3 and 6.
- **Open proof:** Pending re-proof in this repository: compositor window drag
  from the empty header region, narrow-width captures, and AT-SPI header order.

## VEN-C22 — Codex quota fact chip

- **Status:** Proven on x86_64 Linux by the automated tier; native quota
  rendering is pending re-proof in this repository.
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace.
- **Trigger:** The current complete EONW v6 snapshot supplies, changes, or
  removes its optional Codex quota observation.
- **Result:** Venus projects one noninteractive fact chip into the existing
  single-row Eon Bar. Fresh facts show the exact monochrome OpenAI Blossom
  followed by every available elapsed/total window position and remaining
  percentage at ordinary width, such as `[mark] 3h/5h 74% · 6d4h/7d 42%`.
  Under pressure the chip keeps the mark and complete lowest-remaining window,
  then disappears before active-tab identity, fixed controls, or the bounded
  drag region lose their accepted space. Stale labels include `old`; blocked
  and unknown observations show the mark followed by `blocked` or `unknown`.
  Hover text and the static AccessKit label continue to name Codex explicitly and expose every
  window's human-readable position and remaining percentage, reset availability,
  and source freshness or permission state without raw epoch timestamps.
- **Important failures:** An absent observation creates no rectangle, gap, hit,
  action, focus target, or accessibility node. Missing resets remain explicit;
  unusual durations use exact minute labels. A stale label that cannot retain
  `old` disappears. Narrow and tiny widths retain the VEN-C21 tab and control
  priority. Quota-only refreshes do not recenter workspace navigation, cancel
  terminal scroll, replace the attachment, or send an Orbit resize. Venus has
  no quota cache outside the current EONW snapshot and does not refresh or infer
  provider state.
- **Owner:** Under EON-C27, pinned CodexBar owns provider collection; Eon
  Runtime owns bounded subprocesses, validation, freshness and the optional EONW
  field. Venus Scene owns chip text and responsive geometry; the existing
  glyphon renderer owns the exact monochrome mark; hover and AccessKit own the
  explicit provider name.
- **Consumes:** Canonical EONW quota fields; VEN-C21; unchanged ORBF v2 / ORBS
  v13.
- **Boundary:** One fixed monochrome service mark adds no generic icon or image
  system, decoder, provider parsing, process, filesystem or network access,
  timer, cache, quota mutation, account identity, click action, focus target,
  alert, threshold color, chart, configuration, widget framework, second row,
  other provider, Orbit change, or platform expansion. Workspace transport loss
  expires the optional quota while retaining the last useful workspace.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `quota_chip_preserves_truth_layout_and_static_accessibility`,
    `quota_icons_preserve_transparency_and_shape_across_scales`, and
    `quota_refresh_and_workspace_failures_preserve_structure` in the workspace
    suite.
- **Open proof:** Pending re-proof in this repository: an installed Codex chip
  (dogfood item 6 had no provider authentication), native stale, blocked,
  unknown and absent states, and AT-SPI chip labels.

## VEN-C23 — New Eon window shortcut

- **Status:** Proved on x86_64 Linux Wayland.
- **Consumer:** A Venus window supervised by full Eon on x86_64 Linux Wayland.
- **Trigger:** Physical Alt+Shift+N, including while Shortcuts is open.
- **Result:** Venus runs the exact Eon executable supplied by its supervisor
  with `window new` on a worker thread. The action appears in native Shortcuts;
  the current terminal does not receive the key. Eon owns the new workspace,
  process, runtime identity, and Sessions.
- **Important failures:** A missing executable, process launch failure, or
  nonzero command status produces a bounded local notice and leaves the current
  Venus window usable. The command receives fixed arguments without a shell or
  PATH lookup.
- **Owner:** Venus owns native key capture, help, command invocation, and local
  failure notice. Eon owns `EON_NEW_WINDOW_EXECUTABLE` and all window policy.
- **Consumes:** EON-C23 and canonical EONW; Orbit ORBF v2 / ORBS v13 remains the
  unchanged Session owner.
- **Boundary:** This creates another Venus process and Eon workspace, not a
  second view of an Orbit Session or a multi-window Venus event loop. The
  composed macOS product remains unproved.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `new_window_shortcut_invokes_the_supplied_eon_command` in the
    workspace suite; dogfood checklist item 10.

## VEN-C24 — Contextual bottom shortcut hints

- **Status:** Proved on x86_64 Linux native Wayland.
- **Consumer:** One person using a Venus window attached to an Eon workspace.
- **Trigger:** A fresh workspace window, physical Alt+Shift+B, current native
  focus or popup catalog changes, and window size or scale changes.
- **Result:** A fresh window shows one compact, read-only native footer. Its
  window-local visibility choice survives tab switches, popups and resize.
  Showing reserves footer space; hiding returns it to terminal content.
  Initial configured terminal rows include the footer overhead. Hints derive
  from fixed dispatch descriptors and the current enabled popup catalog, group
  shared modifiers once in a continuous row, and retain complete key/action
  pairs in a stable order. Compact groups spread across the bar with equal gaps,
  outer groups near its edges and a lone group centered. Modifier headers alone
  have padded rounded backgrounds without braces; group bodies stay plain.
  Insets, badge padding and minimum gaps participate in whole-hint fitting.
  The main region and footer share side insets and one continuous outer frame,
  with matching upper/lower corner radii, the hidden main frame's bottom inset
  and a straight separator at the join. The outer bottom gap is reserved in
  addition to the compact hints row height. The row is at least the corner
  radius tall, preserving the straight separator at small font sizes. The bar uses the inactive-tab
  background, coral red modifier headers, mint keys and gray labels.
  Copy, Paste and Open link stay in full help and dispatch, without footer hints.
  Compact modifier headers use C/A/S aliases joined by hyphens, such as A-S.
  The compact tab range is 1..9; full help retains the tenth-tab Alt+0 binding.
  Short labels use keys and hints; one move hint groups horizontal and vertical keys.
  Full help and accessible labels retain complete chords and action names.
  Creation labels name the tab, pane or window explicitly.
  Width pressure drops low-priority hints before help and the visibility
  toggle. Native focus-specific keys appear only in their applicable context.
  F6 stays in full help and keyboard navigation, without a footer hint.
  Project is omitted from hints; Alt+Z and its full-help entry remain available.
  The header Shortcuts button is omitted only when the visible footer covers
  every applicable hint and that button lacks keyboard focus; otherwise it
  remains. Alt+/ and the complete viewer
  remain available, including the hints toggle entry. The viewer retains its
  existing modal input isolation.
- **Important failures:** A press toggles once; repeats and releases do not
  leak to terminals. Toggling preserves focus, selected identities and live
  processes; normal terminal resizing is its only terminal effect. Tiny bounds
  omit whole hints and preserve valid Scene geometry. Hidden hints leave no
  accessible footer. Visible hints expose complete chords as read-only text,
  without extra keyboard-focus targets or live announcements.
- **Owner:** Venus owns native visibility, projection, fitting, Scene geometry,
  rendering, input and AccessKit. Eon owns product selection and delivery;
  Runtime reserves the toggle against popup collisions; Orbit owns resizing.
- **Consumes:** Existing VEN-C19/C20/C21, canonical EONW, and ORBF v2 / ORBS v13.
- **Boundary:** No new input mode, child-application key discovery, interactive
  hints, durable preference, generic widget registry, dependency, wire change
  or platform expansion. Future Eon modes supply hints from their own actual
  bindings and replace contents at the same footer height.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `shortcut_hints_follow_bindings_catalog_and_native_focus`,
    `shortcut_hints_drop_whole_pairs_and_rebuild_modifier_headers`, and
    `shortcut_hints_are_read_only_and_retire_hidden_help_actions` in the
    workspace suite; dogfood checklist item 5.
- **Open proof:** Pending re-proof in this repository: native raster checks of
  the rounded footer frame across widths and fractional scale, and read-only
  AT-SPI footer lifecycle.

## Rules

- Each contract uses one `## VEN-CN — Name` heading and the required fields
  `Status`, `Consumer`, `Trigger`, `Result`, `Important failures`, `Owner`,
  `Boundary`, and `Proof`.
- Add optional `Consumes` and `Open proof` fields when applicable; nest
  `Environment` and `Evidence` under `Proof`, and use nested bullets instead of
  prose table cells.
- Contract IDs are stable and repository-qualified. Never renumber or reuse an
  ID; mark an explicitly removed contract retired.
- Only current user-visible behavior, correctness boundaries, ownership
  invariants, and interfaces with Orbit and Eon belong here.
- Every implementation Bead names contracts it changes, proves, consumes,
  hardens, or preserves.
- Historical execution evidence belongs in Beads and Git, not this current-state
  index. User-visible chronology belongs in `CHANGELOG.md`.
- Later work touching a proved owner reruns its indexed checks and advances the
  proof revision or records the remaining gap.

## VEN-C25 — Native Eon version label

- **Status:** Proven on native x86_64 Linux Wayland.
- **Consumer:** Eon Runtime and native workspace users.
- **Trigger:** A workspace client receives an optional supervising Eon version
  through `--eon-version`.
- **Result:** One read-only `EON <version>` label in regular terminal monospace and
  softened cyan joins the existing top row, before shortcut help when visible
  and before close. Its full version is available through hover and a static AccessKit
  label; it adds no focus target or action and is not a window-drag target.
  Width fitting hides it before required controls and active-tab reservation.
- **Important failures:** Absent input means no label; malformed or standalone input is
  rejected. Existing workspace/terminal behavior and footer bounds remain owned
  by VEN-C21/C24.
- **Owner:** Runtime supplies immutable product identity; Venus owns validation
  and native projection.
- **Boundary:** No EONW change, widget framework, version discovery, polling,
  dependency or platform expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `eon_version_is_bounded_workspace_identity` and
    `version_label_uses_only_spare_space_and_has_no_actions` in the workspace
    suite; dogfood checklist item 6.
- **Open proof:** Pending re-proof in this repository: native narrow-width
  hiding, label pointer/drag inertness, and AT-SPI order.

## VEN-C26 — Claude quota beside Codex

- **Status:** Proven on x86_64 Linux by the automated tier; native quota
  rendering is pending re-proof in this repository. Installed Eon composition
  remains Eon's responsibility.
- **Trigger and result:** A complete canonical EONW v8 snapshot supplies an
  optional Claude observation. Venus shows a read-only Claude chip with
  the available five-hour/seven-day remaining percentages, each led by its
  elapsed position such as `3d2h/7d` when Claude reports a reset, as Codex's
  chip does; a window without a reset shows no position. Hover and a static
  accessible label name Claude, every window, freshness and how far into each
  window the observation is, or that its reset time is unavailable. Stale text
  retains `old` even in compact presentation.
- **Ownership:** Runtime alone supplies source facts, age and invalidation.
  Venus uses its existing Scene, renderer, hover and AccessKit owners. Both
  provider chips remain in the existing row; full labels become compact, then
  trailing optional chips disappear before required tabs, controls or drag
  space. Codex's mark, labels and provider policy remain unchanged.
- **Failure:** Absent facts have no geometry or accessible node. Workspace
  transport loss removes both observations while keeping the last useful
  topology. Quota-only changes do not replace attachments, scroll positions or
  navigation. Neither chip has an action or keyboard focus target.
- **Consumes:** Canonical EONW quota fields; the in-tree Orbit codec stays
  unchanged.
- **Boundary:** `eon-claude-quota-eon-bar-zm5f` owns this slice. No
  provider parsing, credential access, client polling, new module, dependency,
  framework, row or platform promise.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `two_provider_chips_share_layout_without_losing_identity_or_freshness`
    and `quota_icons_preserve_transparency_and_shape_across_scales` in the
    workspace suite.
- **Open proof:** Pending re-proof in this repository: the installed Claude
  chip beside Codex, native stale,
  Claude-only and transport-loss states, and AT-SPI provider labels. The
  elapsed-position labels (`eon-claude-chip-position-iubd`) are proved by the
  automated tier only; their installed view is pending.

### VEN-C22/VEN-C26 — Optional-reset consumer refinement

- **Status:** Proven by the automated tier; native rendering is pending
  re-proof in this repository.
- **Contract:** Consume canonical EONW v9 with unchanged quota chips.
  Fresh and stale provider windows may lack a reset. Show their actual remaining
  percentage and state; hover/accessibility say reset time unavailable.
  Historical codec validators remain unchanged. No provider mechanism, new
  field, presentation owner or platform is introduced.
- **Preservation:** The in-tree Orbit codec remains unchanged; preserve the
  restored version/help order of VEN-C25.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `quota_without_reset_retains_percent_and_accessible_freshness`
    in the workspace suite.
- **Open proof:** Pending re-proof in this repository: installed chips without
  a reset.

### VEN-C26 — Claude splat icon

- **Status:** Proven by the automated tier; native rendering is pending
  re-proof in this repository.
- **Contract:** Replace the quota chip C monogram with Claude's recognizable
  splat in its original terracotta orange (`#D97757`), preserving its logo bounds,
  text, both available windows, hover/accessibility identity, freshness, compact
  and hidden states, and static no-action behavior.
- **Owner:** Venus's existing custom-glyph renderer and bundled alpha mask.
  No provider, protocol, dependency, module or platform change.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `quota_icons_preserve_transparency_and_shape_across_scales` in
    the workspace suite.
- **Open proof:** Pending re-proof in this repository: the installed splat
  icon.
