# Venus contract index

This is the canonical current state of Venus behavior, ownership, proof, and
remaining limitations. Beads and Git retain execution history; `CHANGELOG.md`
retains accepted user-visible chronology.

`VEN-C16` owns platform scope. Venus consumes ORBF v2 / ORBS v13 through the
in-tree `orbit-protocol` crate and canonical EONW v20 through the in-tree
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

## VEN-C5 — Native hyperlinks

- **Status:** Proved on x86_64 Linux by the automated tier; native Linux
  hyperlink interaction is pending re-proof in this repository. Apple Silicon
  hover, Command+C, and Command+click are observed natively under `VEN-C16`.
- **Consumer/trigger:** A Venus user hovers an Orbit-authored OSC 8 link or a
  plain-text HTTP/HTTPS URL in visible terminal text, or explicitly activates
  one.
- **Result:** Scene detects plain-text targets in each frame's visible rows: an
  `http://` or `https://` start, in any case, at a line start or after a byte
  that is not an ASCII letter or digit, extended over the opener's URI bytes and
  joined across soft-wrapped rows. A trailing `.`, `,`, `:`, `;`, `!`, `?` or
  `'` is trimmed, as is a closing `)` or `]` the target does not open. Only
  targets the opener accepts are detected, and a target touching any OSC 8 cell
  is not, so the explicit link wins. Each row segment of a wrapped target is one
  target with the full URI. The hovered link is highlighted and its escaped
  target is shown with Open and Copy actions. The highlight and hint are derived
  from the frame being presented, so a target that stays under a still pointer
  stays highlighted while output arrives. Linux uses `Ctrl+click` and
  `Ctrl+Shift+C`; macOS uses `Command+click` and `Command+C`. When a current Eon
  popup takes the Copy key under VEN-C20, the hint omits Copy and that key
  invokes the popup; the AccessKit Copy button remains. Opening requires a press
  and release on the same target: the same URI over the same cells, presented
  at both; output revisions between them keep the press, and every other change
  in Failures retires it. Copy uses the current hovered target unless terminal
  selection retains precedence. AccessKit exposes each current visible target as
  an Open link followed by a Copy button, both using the native Click action.
  Ordinary terminal clicks, input and selection retain their owners.
- **Failures:** Attachment, geometry, presentation, focus, occlusion, or scroll
  changes retire link actions. An output revision keeps a target that stays
  over the same cells with the same URI and retires any other; the hovered
  target is re-derived in the new frame. A captured release without Ctrl is
  consumed without activation. Opening accepts at most 4096 bytes of ASCII
  HTTP/HTTPS URI syntax with a host and without credentials. Copy accepts bounded
  target text without control characters. Unsupported or malformed targets,
  missing native handlers, and launch failures/timeouts have bounded visual and
  accessible notices. No URI is opened on hover or keyboard focus. A success
  notice ('Link handed to the desktop handler.' and 'Link copied.') clears about
  2 seconds after it appears, through one scheduled redraw and no repeating
  ticks, or sooner on the input that clears it today. Failure and timeout
  notices keep that input-bound lifetime. The 2-second value is fixed: a UX
  timing with no accessibility or tool-convention reason for users to differ
  (Configurability).
- **Owners:** Orbit owns URI attributes; Scene derives explicit spans and
  detects plain-text targets from the projected visible cell text; Venus
  presentation identity gates actions; the existing renderer, input, AccessKit
  status and native clipboard owners project them. Linux `gio open` on the host
  PATH dispatches the exact URI as one argument to the desktop's registered
  handler. One launch runs at a time with a ten-second dispatcher deadline.
- **Consumes:** Orbit `ORB-C6` through ORBF v2 / ORBS v13.
- **Boundary:** No detection of other schemes, bare hosts, file paths, or a URL
  whose start is above the visible rows; no file/custom-scheme opening, URI
  rewrite, browser embedding, new crate, or protocol change. Host GIO and a
  registered handler are required for Linux opening; copy remains available
  without them. macOS uses fixed `/usr/bin/open -u` and the general pasteboard.
  Product-path accessibility activation remains unaccepted on macOS.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `hyperlink_actions_require_current_presentation_and_safe_exact_uri`,
    `stalled_link_dispatcher_is_bounded_and_reaped`,
    `hyperlinks_follow_visible_heads_and_wide_cells`,
    `visible_links_offer_only_open_and_copy_and_retire_with_the_scene`,
    `clipboard_failure_outranks_stale_link_notice`, and
    `macos_uses_command_for_standard_host_actions_only` in the workspace suite.
  - **Plain-text detection evidence (`eon-nlcy`):**
    `plain_text_links_trim_punctuation_and_unopened_brackets`,
    `plain_text_links_join_soft_wrapped_rows_only`, and
    `explicit_links_and_non_uri_cells_bound_plain_text_links` in the Venus
    suite; one isolated installed hover and Ctrl+click observation on headless
    Sway recorded in `eon-nlcy`.
  - **Steady-under-output evidence (`eon-1u9e`):** the link steps of the
    ignored `live_output_keeps_application_input_admitted_before_repaint` on a
    private headless Sway; one isolated installed observation on headless Sway
    recorded in `eon-1u9e`: hover and hint stay steady while a pane redraws
    every 50 ms, and a Ctrl+click held across that output dispatches the URI.
  - **Override evidence (`eon-igae.10`):**
    `configured_popups_take_fixed_bindings_except_reserved_keys`; the installed
    observation is pending.
- **Open proof:** Pending re-proof in this repository: installed hyperlink open
  and copy (dogfood item 9 was not exercised), AT-SPI Open/Copy
  activation, and native missing-handler failure. Fractional scale and other
  compositors remain unproved.

## VEN-C1 — Authoritative native presentation

- **Status:** Proven on x86_64 Linux; broader native quality remains unproved.
- **Consumer:** One Venus native surface consuming canonical Orbit frames and an
  optional cursor-tail color choice.
- **Trigger:** Venus accepts an initial frame, an ordered later revision, or a
  cursor-tail startup value.
- **Result:**
  - Venus validates and materializes one coherent Orbit-authored structured
    frame into native draw inputs without terminal authority.
  - An omitted or `random` color choice selects one of eight Venus presets once
    per live endpoint: the device, inode and change time of the workspace or
    Orbit socket Venus attaches to pick it, so every launch against that live
    socket shows the same preset and a recreated socket picks afresh. A socket
    absent at launch falls back to one choice for that process. The internal
    fallback duration is `1.0`. Eon owns
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
  broader Linux compositor coverage remain outside the proof. The per-endpoint
  preset (`eon-cursor-color-after-attach-lbbo`) is covered by
  `random_cursor_preset_is_stable_per_live_endpoint` and that bead's installed
  full-Eon reattach dogfood; eonbase reopens are not dogfooded.

## VEN-C2 — Native semantic interaction

- **Status:** Proved on x86_64 Linux; Apple Silicon ordinary typing and
  selection are observed natively under `VEN-C16`, and the rest of its macOS
  proof is pending re-proof in this repository.
- **Consumer:** One focused Venus terminal surface.
- **Trigger:** Native key, mouse, focus, paste, input-method, or resize activity.
- **Result:** Venus maps native activity to canonical Orbit semantic input and
  resize messages using one shared scaled geometry owner. Compatible output
  advances render content without withdrawing the last presented input geometry.
  Venus resolves each native key event to one key. When the active keymap
  produces a named key (such as Escape, Return, Tab, an arrow, a function or
  modifier key) outside the numpad and Orbit has a key for it, Venus uses that
  key; otherwise it uses the physical key, so letter, digit, punctuation and
  numpad keys stay physical. Venus's terminal input and its own named-key
  actions (focus return, chrome traversal, the shortcut viewer, header
  activation) use the resolved key. When Shift is the only held modifier and
  a key's text differs from its unshifted text, Venus reports Shift as
  consumed by that text. Space's unshifted text is a space, so Space finishing
  a dead key with Shift held, such as `"` under `us+intl`, reports its text
  with Shift consumed; a program using the kitty keyboard protocol without
  report-all receives the text, not Shift+Space.
- **Important failures:** Invalid, stale, unsettled, or non-presented geometry
  cannot reach Orbit; capture and focus transitions withdraw obsolete input.
- **Owner:** Venus native input and geometry owners; Orbit encodes terminal
  behavior.
- **Consumes:** Orbit `ORB-C5` through canonical ORBS v13.
- **Boundary:** Letter, digit and punctuation keys stay physical, so letter
  shortcuts stay layout-independent, and keymaps that place letters on
  unrelated keycodes (some virtual keyboards) remain unsupported for chords.
  Physical mixed-monitor hardware and broader Linux Wayland compositor
  behavior remain manual quality surfaces. On Linux, dead keys and compose
  sequences from the locale's compose table are accepted. On macOS, only
  VEN-C16's Kotoeri Roman-to-kanji IME path is accepted; held-key repeat, dead
  keys, and other IMEs remain unaccepted. Physical trackpad gesture quality
  remains with VEN-C7.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, with the
  `eon-venus-keysym-named-keys-chnh` commit for named-key resolution and the
  `eon-e2vl` commit for Linux dead keys and the Shift-consumed rule
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
    dogfood checklist items 1, 3 and 4 (Enter, Esc and Tab reached the picker);
    for named-key resolution,
    `named_keys_follow_the_keymap_and_writing_keys_stay_physical` and
    `a_press_pins_its_resolution_until_release` in the Venus suite, and an
    installed observation of tree `acb2a2f2cc859b642f864da8cb25bfa102c8798a`
    (`/nix/store/gaf0asn9dmpwl9h2jgslafpwd13avkp8-eon-0.1.1`) on x86_64 Linux,
    private headless Sway 1.12 with wdotool 0.5.3 and wtype 0.4 virtual
    keyboards: under the US layout, `caps:swapescape` (Escape from the CapsLock
    key) and wtype's own keymap, a raw-mode pane received Escape, Return, Tab,
    BackSpace and Up as `033`, `\r`, `\t`, `177` and `ESC [ A`, that Escape
    closed the shortcut viewer, and Alt+Shift+T opened a tab under the US
    layout and `caps:swapescape`; for dead keys and the Shift-consumed rule,
    `derives_layout_metadata_without_a_keyboard_map` in the Venus suite, and
    an installed observation of tree `85e094d0e9f0aa1cc120d36579b7387ee7f63613`
    (`/nix/store/qp01mka6i8xicnc6wfxczyb2qcygvlsp-eon-0.1.1`) on x86_64
    Linux, private headless Sway 1.12 with a wdotool 0.5.3 `us+intl` keymap:
    after `CSI >1u`, the pane echoed `"`, `'`, `~`, `` ` ``, `é`, `ã` and
    `ä` for Shift+dead key then Shift+Space, dead key then Space, and dead key
    then a letter, plain Shift+Space still sent `ESC[32;2u`, nushell's legacy
    prompt received the same characters, Alt+Shift+T opened a tab and Ctrl+C
    cleared the prompt. Multi_key (Compose-key) sequences were not exercised.
- **Open proof:** Pending re-proof in this repository: installed pointer
  input, selection and paste (dogfood item 9), the ignored native
  input-between-frames regression, native IME preedit/commit, Orca reader focus
  after return to live, and the Apple Silicon Option/Control modifier mapping,
  focus, and Kotoeri observations (ordinary typing and selection are observed
  under `VEN-C16`).

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
  - A burst of Orbit output never costs the attachment. When Venus's bounded
    event queue is full, Venus stops reading from Orbit until its event loop
    drains the queue. Orbit's bounded slow-reader handling (`ORB-C7`) then
    delivers the newest frame, so the pane resumes at the current terminal
    state.
- **Important failures:** Standalone Busy and incompatible, exited, malformed,
  or replaced endpoints remain terminal rather than creating duplicate clients.
- **Owner:** Venus connection/retry and native event-loop lifecycle; Orbit owns
  Session survival.
- **Consumes:** Orbit `ORB-C1`, `ORB-C3`, `ORB-C4`, and `ORB-C7` through canonical
  ORBS v13.
- **Boundary:** Broader Linux compositor coverage remains unproved.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, with the `eon-10qt`
  commit for output-burst backpressure
  - **Environment:** Automated and installed.
  - **Evidence:** `orbit_retry_schedule_is_bounded_and_resettable`,
    `workspace_recovery_requires_the_same_selected_live_endpoint`,
    `only_supervised_orbit_busy_is_retryable`,
    `protocol_and_terminal_failures_do_not_become_socket_retries`,
    `retryable_loss_records_state_independently_of_retry_policy`,
    `reconnect_preserves_the_last_scene_and_accepts_a_fresh_revision`,
    `missing_and_dropped_sockets_can_attach_on_later_attempts`,
    `typed_attachment_is_duplex_and_client_drop_leaves_server_listener`, and
    `full_event_queue_holds_the_reader_until_drained_or_dropped` in the
    workspace suite; dogfood checklist items 2 and 11; the `eon-10qt`
    installed burst-while-scrolling observation.

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
  replacement retries it. Incompatible, Exited, model, invalid-input,
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

- **Status:** Proved on x86_64 Linux by the automated tier; native Linux
  pointer interaction is pending re-proof in this repository. Apple Silicon
  drag selection, Command+C copy, and wheel history are observed natively under
  `VEN-C16`.
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
    reserves space for the count and fits its pane label into the remaining
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
  continuous output. Screen-reader acceptance remains separate. Apple Silicon
  trackpad gesture quality stays open.

## VEN-C8 — Eon workspace presentation

- **Status:** Partially proved
- **Consumer:** One Venus surface controlled by an Eon workspace.
- **Trigger:** Outside visible `VEN-C18` popup presentation, Eon supplies
  durable topology, a workspace action changes tab, pane, focus, order,
  lifetime, liveness, endpoint, or metadata, or the user toggles zoom.
- **Result:**
  - Venus materializes ordered horizontal tabs and lays the active tab's one to
    four columns side by side at equal widths, the last taking the remainder.
    Each column shows every fitting header around its one expanded pane. Each
    tab shows its current one-based position plus the directory leaf or `/`
    derived from Eon's authoritative launch directory; positions update when
    Eon reorders tabs.
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
  - Runtime supplies labels for every workspace pane, including collapsed panes and panes in
    background tabs. Venus consumes those labels without reconstructing their naming policy.
  - While no popup covers the tab, and except as zoom below hides columns,
    each column's live expanded pane is an Orbit attachment resized to its
    own column. Keyboard input, IME, the cursor trail, selection, links, the
    scrollback pill and status notices belong to the selected column; other
    columns draw a static cursor over their own terminal background. A left press on another column's terminal sends
    `FocusId` for its expanded pane and focuses the terminal without
    forwarding the click; the wheel over another column does nothing.
    Accessibility lists each column's panes left to right and exposes
    terminal text only for the selected column.
  - Ctrl+Alt+M sends one non-repeating `CreateColumn`. In a tab with one to
    three columns, Venus refuses it locally with a rejected-action notice when
    one more column, at the current window width, would leave a column fewer
    than 40 terminal columns wide; with four, Eon's capacity refusal applies.
    Shrinking the window later narrows existing columns without removing
    them.
  - Alt+Shift+Z toggles window-local zoom of the active tab's selected pane
    once per press and sends no workspace action. While zoomed, Scene lays out
    only that pane, as one rounded card with its label in the top edge spanning
    the area all of the tab's columns cover; the other panes' attachments stay
    open at their last size and are not resized, drawn, hit-tested or listed to
    accessibility. An accepted snapshot that changes the active tab, its panes
    or columns, or its selected pane ends the zoom, and unzooming sizes
    every column for the current window again, resizing only where that size
    changed. The active tab's label gains a zoom-in magnifier glyph (Nerd Font U+F00E)
    after its name and its accessible name
    `, zoomed`; the pane panel is named `Zoomed pane <label>` and lists only
    that pane. In a tab with one pane the key is consumed with a
    rejected-action notice. While a popup covers the tab it is consumed
    without effect; the popup hides the zoomed layout and its marker, and both
    return when the popup hides. A current Eon
    popup on the key takes it (VEN-C20).
  - Pane frames draw each pane as its own rounded card at the border width,
    stacked without gaps in its column's allocated card area. A collapsed pane
    is one title row showing a rounded edge with its label in it; its corners turn
    toward the column's expanded pane and its sides continue to the next card in
    that direction; when the column's last pane is collapsed, its
    label sits in the column's bottom edge instead, so the stack closes with one line;
    at the unscrolled bottom that line shares one snapped line with the agent sidebar card's bottom;
    the expanded pane is a full rounded frame with its label in the top edge.
    Pane titles use the same shared
    chrome title metric as the agent sidebar title (VEN-C30). Scene owns one card gap (8
    logical pixels at default typography), which is also the corner radius:
    allocated column card areas keep it from the window's sides, from each other,
    and below them the footer's band is the margin, or one gap when hints are hidden. In both modes the tab pills
    keep the gap from the window's left edge, so the first pill lines up with
    the first card. Pill height follows VEN-C21; the gap between pills and
    header button width stay as before; on a short window the extra inset
    yields before the pills do. On a window too small for the gaps they
    yield before a terminal cell does. Each pane's top edge passes through the
    middle of its title row, breaks around its label and follows its own rounded
    corners. At the unscrolled top, each column's first pane, the popup card and
    the agent sidebar card share one snapped top line, subject to VEN-C18's
    tiny-window popup exception. Each column's fill starts at its first pane's top
    edge and follows that pane's rounded top corners down to the column's rounded
    bottom; inside it, title rows and the space between stacked pane cards share the
    expanded terminal's background color/opacity under the surface-wide blur request,
    so the stack has one background. In framed, unzoomed expanded panes, the band
    below the title rule follows VEN-C11's top-padding color and opacity rules
    instead of the shared title-row background; title text, cutouts and outlines
    stay above it, while collapsed rows, popups, zoom and frameless headers retain
    their existing backgrounds. Nothing above that top edge or outside those
    outer corners takes the terminal background: everything below the tab bar outside
    the column fills, including the gaps and footer band, takes the header's opaque
    color, so header, gaps and footer form one frame. The first pane's label sits on
    the boundary between the two. The selected pane in the selected column has a
    brighter label and, with pane frames enabled, its whole frame in the focus
    color at the focus border width. Eon supplies both
    widths in logical pixels. Hover adds a pill behind the label;
    keyboard focus adds a distinct accent pill outline. Title rules stay
    straight during scrolling; overflowing headers scroll within fixed allocated
    column card areas. `--pane-frames true|false` defaults to
    `true`; `false` removes card borders and title rules, leaving label-only
     pane headers with the selection, hover and keyboard-focus feedback
    described above. Both modes preserve the same terminal grid, header hit
    targets and AccessKit bounds.
    Initial requested grids include the card gaps in their window overhead;
    the gaps accept no pane or terminal input. Column width and the 40-column
    admission check both come from the allocated column card areas.
    Changed pane lists resize the terminal even when the endpoint stays selected.
    The option has no visual effect in standalone presentation. A popup
    draws its outlined card in both modes; only its surroundings differ, the
    opaque frame with `true` and terminal background with `false`.
    Missing, duplicate or invalid values fail before window creation. Eon owns
    persistent configuration and composed-launch policy.
  - While the agent sidebar is visible (VEN-C30), the cards share the content
    area to its right: the side gaps and the 40-column CreateColumn check are
    measured from that area, and the first tab pill keeps its window-edge
    position, so its alignment with the first card is suspended.
  - Headers show Runtime's bounded pane label. Fitting retains the literal pane identity whenever it
    fits and middle-elides the remaining text at shaping-cluster boundaries; AccessKit and final-close
    pane listings retain the full bounded label. The ID may be dimmer than the name, with 4.5:1 contrast
    as the target against the resolved fill at the configured opacity; when dimming compromises
    readability, it uses the name's colour. Terminal title remains available for selected-window
    compositor semantics. Labels never expose mapped Session identity.
  - Venus opens no separate workspace-label metadata observers. Removed panes retire their label
    projections, and a 250 ms reinspection exposes accepted external workspace changes without native
    input.
  - Pointer input, Alt+H/L column and tab traversal, Alt+K/J pane traversal,
    Alt+M pane creation, Alt+Shift+T tab requests, focused arrow traversal, and
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
  - Ctrl+Alt+Shift+H/L sends one non-repeating semantic move for the active
    tab; Ctrl+Alt+K/J does the same for its selected pane within its column, and
    Ctrl+Alt+H/L sends one EONW v12 sideways move of that pane into the
    neighbouring column. When that move would open a column at the tab's edge,
    Venus applies Ctrl+Alt+M's 40-terminal-column refusal. Alt+Shift+W sends one
    non-repeating close naming the snapshot's active stable `tN`; Alt+W sends
    one EONW v11 close naming the active tab's selected `pN` and is consumed
    without an action when the tab has none. Both may target the final tab and
    its last pane. While the snapshot lists other workspaces (EON-C33), that
    close only removes the shown workspace and Venus sends it without a
    prompt. Otherwise such a final close, when the `--warning-close-last-tab`
    launch argument is true (its default), first shows the EON-C8 prompt, which
    Venus owns: Enter sends the held close, first focusing its tab if another
    tab became active, Escape dismisses it, Alt+Shift+D detaches, the wheel and
    the viewer's navigation keys scroll a long list, and other keys and pointer
    input reach nothing while it is open.
    The prompt closes itself when its named tab or pane leaves the snapshot or
    the workspace becomes unavailable; it is an AccessKit alert dialog. When the
    argument is false, the close is sent at once. Alt+Shift+D is a
    non-repeating key that detaches by the same path as a compositor close
    request (VEN-C14). Venus consumes
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
  remain available through unchanged, quota-only, and EON-C31 attention- or
  sidebar-only refreshes until workspace structure changes, except the EON-C31
  empty-queue notice, whose lifetime VEN-C30 sets.
  Connection-unavailable notices clear on a successful snapshot.
  No notice disables link hover, highlight, open or copy: VEN-C5 links work
  while any workspace or link notice shows. The status line shows the newer of
  a workspace notice and a VEN-C5 link result. With no pending result, a
  hovered link's hint outranks the workspace notice, which returns when the
  pointer leaves the link. A render failure notice still outranks both, and a
  terminal notice still outranks a link result or hint.
  Terminal-status alerts announce their actual error without moving focus;
  routine polling does not repeat the announcement.
- **Owner:** Venus owns workspace materialization, label fitting, rendering and accessibility; Runtime owns
  semantic pane labels and pins; Orbit owns terminal metadata and Session lifetime.
- **Consumes:** Eon `EON-C10`, `EON-C17`, `EON-C18`, canonical EONW through the
  in-tree `eon-workspace-protocol` 0.1.0, and Orbit terminal presentation in
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
  - **Final close (`eon-final-close-prompt-and-detach-key-m798`):** the
    prompt's trigger, key answers and Alt+Shift+D mapping by
    `close_prompt_asks_only_for_a_window_ending_close`, its scene and
    AccessKit projection by
    `shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space` and
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog`, and the
    launch argument by `pane_frame_option_is_a_strict_single_boolean`.
    Installed proof (`eon-prove-close-and-detach-batch-bf8t`): edge
    `22915af5dca41f87e706ad548c0506f79c17648f`, installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, x86_64 Linux,
    private headless Sway at 1280x720. With the warning on, Alt+Shift+W, the
    bar's `×` and Alt+W on the last pane each showed the prompt listing `p1`;
    Escape kept the window, Enter ended it, and Alt+Shift+D from the prompt
    detached it. With `warning_close_last_tab = false`, Alt+Shift+W ended the
    window at once.
  - **Agent sidebar layout (`eon-agent-sidebar-yx8.13`):**
    `agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state`
    and the installed dogfood recorded under VEN-C30: the sidebar pushed the
    columns, Alt+Shift+H gave its width back, and at 1000x720 it hid for
    width.
- **Open proof:** Columns (`eon-splits-opwv`): layout, column attachments and
  the 40-column threshold are covered by
  `columns_sit_side_by_side_and_keep_the_selected_column_primary`,
  `other_columns_attach_their_expanded_panes_until_a_popup_covers_the_tab`,
  `direct_workspace_shortcuts_use_stable_eon_actions`,
  `structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit` and
  `each_column_keeps_forty_terminal_columns`. Isolated installed dogfood on a
  private headless Sway (`eon-splits-opwv` slice 4) observed live columns,
  attachment swap and reconcile, per-column resize and both refusal notices.
  Pane close (`eon-close-pane-onwi`): the Alt+W mapping and its non-repeat by
  `direct_workspace_shortcuts_use_stable_eon_actions` and
  `structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit`; isolated
  dogfood of the built package on a private headless Sway
  (`eon-close-pane-onwi` slice B, 1280x720) showed the `close pane` hint and
  the refusal notice.
  Sideways pane moves (`eon-sideways-pane-moves-k9ca`): the Ctrl+Alt+H/L and
  Ctrl+Alt+Shift+H/L mappings and the edge-column predicate by
  `direct_workspace_shortcuts_use_stable_eon_actions` and
  `sideways_moves_open_a_column_only_for_a_stacked_outer_pane`; isolated
  dogfood of the built package on a private headless Sway (slice B, 1280x720)
  delivered both chords with exact modifiers and showed the 40-column notice
  for a move that would open a third column.
  Fractional native scale remains to be dogfooded. Cards
  (`eon-gapped-rounded-cards-txco`, whose commit is the revision) are covered by
  `column_backgrounds_cover_the_gaps_between_cards` and the card geometry in
  the scene tests above, and by isolated captures of the built package on a
  private headless Sway at scale 1 in both frame modes. The header-colored
  frame outside the cards (`eon-tubj`, whose commit is the revision) is covered
  by `frame_fills_everything_outside_the_cards`,
  `framed_card_corners_stay_opaque` and the same captures. The tab inset
  (`eon-gv1r`, whose commit is the revision) is covered by
  `popup_outer_chrome_reuses_the_pane_stack_rectangle` at default and small
  typography, the original tiny-window cases in the scene tests above, and
  isolated captures. Zoom (`eon-zoom-selected-pane-e2pv`, whose commit is the
  revision) is covered by `zoom_names_the_tab_column_count_and_selected_column`,
  the zoomed scene in `columns_sit_side_by_side_and_keep_the_selected_column_primary`
  and `zoomed_tab_names_the_zoom_and_lists_only_its_column`; isolated dogfood of the built package on a private headless Sway
  (1280x720) zoomed the second of two columns, showed the `· zoom` pill, sent
  typed input to the zoomed terminal, kept a counter in the hidden column
  printing (8 before, 24 after), restored both columns on Alt+Shift+Z, and
  ended the zoom on Alt+H. Pending
  re-proof in this repository: the ignored native long-label header regression and reader
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
  9), native bracketed-mode and overlapping-shortcut observation, and the
  Apple Silicon PNG-only and oversize-pasteboard notices (Command+V and the
  empty or non-text pasteboard notice are observed under `VEN-C16`).

## VEN-C11 — Terminal-background opacity

- **Status:** Proved
- **Consumer:** One Venus native surface.
- **Trigger:** Launch with an optional finite `--background-opacity VALUE` in
  `0.0..=1.0`.
- **Result:** Omission equals `1.0`; the value applies only to terminal default
  background, later Orbit-authored background changes, and terminal padding.
  The agent sidebar's card background (VEN-C30) follows it too, as the
  background of the terminal in the leftmost visible column. Explicit cells,
  selection, inverse video, workspace chrome, notices, focus, foreground
  effects, input, hit testing, and accessibility remain unchanged.
  Terminal padding follows `[terminal] padding_color`. `extend` (default) gives
  the padding beside each edge row and column the background of its nearest
  cell. On the primary screen it does not extend vertically when the nearest
  row has a default-background cell or a full-height powerline glyph.
  `extend-always` extends without those exceptions. `background` paints the
  terminal default background as before. Background opacity applies to the
  padding as today, and the padding stays inside the pane rounded card.
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
  - **Agent sidebar card (`eon-agent-sidebar-yx8.13`):**
    `card_backdrop_keeps_cards_like_the_agent_sidebar_at_terminal_opacity` and
    the installed dogfood recorded under VEN-C30: at Eon's default 0.80 the
    card's background measured the same as the pane's, with and without pane
    frames.

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
- **Trigger:** A complete private Present command, control EOF, terminal read
  failure, Alt+Shift+D, a compositor close request, or an EONW `Ended` reply.
- **Result:** Present preserves process and terminal attachment while requesting
  presentation of the existing native window; control loss exits through the
  native event loop, releases attachments, and leaves Orbit live; replacement
  may retry transient Busy while the old client releases. Alt+Shift+D and the
  compositor close request both detach; an EONW `Ended` reply, which answers a
  close that stopped the window's last terminal, exits like control loss,
  leaving no Orbit terminal of that generation.
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
  - **Final close and detach (`eon-prove-close-and-detach-batch-bf8t`):**
    isolated installed dogfood (edge
    `22915af5dca41f87e706ad548c0506f79c17648f`, installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, private headless
    Sway, 1280x720): Venus exited after Enter in the final-close prompt, and
    no Orbit terminal of the generation remained; Alt+Shift+D closed the
    window with its terminal still live.
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

- **Status:** Proved for x86_64 Linux native Wayland; the Apple Silicon macOS
  foundation and interaction slices are partially re-proved natively in this
  repository, and macOS remains unsupported.
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
    and Vulkan event loop through installed Eon and eonbase.
- **Proof:** `e0aa47e7186dc3e2a88f826d3ed0feb171a1b0cb` (Apple Silicon, partial)
  - **Environment:** Apple M2, macOS 26.7.1 (25G241), one 1920x1080 scale-1
    display, `aarch64-apple-darwin`; native
    `cargo build --locked -p yazelix-venus -p yazelix-orbit --all-targets` and
    `cargo test --locked -p yazelix-venus --no-fail-fast` with Rust 1.96.0,
    Zig 0.15.2 against the macOS 15.4 Command Line Tools SDK, Ghostty
    `a887df42c56f6de86c0fe6da9c4eeca37931e083`, and
    `LIBGHOSTTY_VT_SYS_CPU=baseline`. The debug `yazelix-orbit serve` and
    `yazelix-venus` binaries from that build ran in a logged-in session over
    VNC. Every observation is recorded in `eon-z4n6`.
  - **Evidence:**
    - The binary and `tests/canonical_scene.rs` tests pass, and the library
      tests pass once the Nix-pinned DejaVu, Symbols Nerd Font, and Accelerando
      fixture fonts are installed for the user; stock macOS lacks them and
      fails only the six library tests that need them.
    - Clippy with `-D warnings` reports one macOS-only lint: an unused
      `TouchPhase` import in the binary's tests.
    - An AppKit window with opaque Metal presentation shows the bounded Orbit
      connection notice, then attaches when Orbit appears.
    - Window zoom resizes the Orbit PTY. Latin accents, kana, Hangul, colour
      emoji, box drawing, braille, and combining accents render.
    - Typing, drag selection, Command copy and paste, the empty or non-text
      pasteboard notice, OSC 8 hover, Command+C on a hovered link, and
      Command+click, which opened Safari with the desktop-handler notice,
      work.
    - Wheel scrolling through history works.
    - Command+Q exits Venus while Orbit and its shell survive, and a relaunched
      Venus reattaches.
- **Open proof:** Pending re-proof in this repository: the no-Wayland failure
  observation. Not observed natively at the revision above: Kotoeri
  composition, Retina-scale redraw, renderer recovery, the PNG-only and
  oversize-pasteboard notices, Option/Control modifier mapping, held-key
  repeat, physical trackpad gestures, focus loss, and AccessKit beyond window
  creation. With stock macOS fonts at that revision, Han ideographs render
  blank, stacked combining marks are dropped, emoji modifier and flag sequences
  don't compose, and a long notice truncates at the window edge; the cause is
  not diagnosed. `ven-prove-venus-apple-silicon-macos-nm7` owns these and the
  rest of the macOS proof: accessibility acceptance, lifecycle identity,
  transparency/blur, exact Orbit dogfood, and the support claim.

## VEN-C17 — Caller-owned native application identity

- **Status:** Proved
- **Consumer:** Eon, eonbase, or one approved eonbase composition.
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
- **Geometry:** A selected popup draws as one rounded card over the area
  allocated to the columns, so toggling preserves the outer side and bottom
  bounds and the shared top line at any column count. On a window too short to
  fit one padded terminal cell below that line, the popup's top edge rises just
  enough to keep that cell. Shared Eon-supplied logical
  margins inset the popup terminal content within that stable shell. The popup
  uses the border width and carries its catalog label, such as "Files", in its
  rounded top edge in the pane-title style, using the shared chrome title
  metric. It keeps a small top gutter before terminal padding. Scene owns
  both rectangles: chrome drives the outline, while terminal drives resize,
  clipping, pointer input, IME and accessibility. Margins and the title gutter
  shrink to preserve a usable cell grid; tiny surfaces omit the label. The popup's
  fill starts at its rounded top edge; inside it, its exposed margins share the
  terminal default background and opacity, with the existing best-effort surface-wide
  compositor blur request, and its title sits on the boundary. Pane-frame
  configuration has no effect on the popup outline. While the agent sidebar is visible, the
  card area is the content area to its right (VEN-C30), so popups never cover
  the sidebar.
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
  - **Beside the agent sidebar (`eon-agent-sidebar-yx8.13`):**
    `agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state`
    and the installed dogfood recorded under VEN-C30, where the Git popup's
    card sat beside the visible sidebar.
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
  snapshot. A popup on the key of a fixed binding other than Alt+Slash and
  Alt+0 through Alt+9 takes that key: dispatch invokes the popup, the footer
  omits the fixed hint, and the viewer row reads "<action> (overridden by
  <popup label>)". Wheel and navigation keys scroll bounded content. Opening,
  refreshing, scrolling, and closing send no terminal bytes or workspace
  action; matching releases remain captured and prior terminal/chrome focus is
  restored on dismissal. The title, subtitle, and first group heading occupy
  separate readable vertical space. The fixed bindings include Alt+Shift+D,
  listed as "Detach: close the window, keep terminals running", and
  Alt+Shift+H, listed as "Show or hide the agent sidebar" (VEN-C30) and
  omitted while the EONW snapshot's `sidebar` is `Off`, and Alt+G and
  Alt+Shift+G, listed as "Jump to the next agent needing attention" and "Jump
  to the previous agent needing attention" (VEN-C30) under every `sidebar`.
- **Important failures:** Repeated toggle events do not oscillate the viewer.
  Workspace loss, empty or maximum-size catalogs, long labels, resize, scale,
  focus loss, and tiny surfaces retain local dismissal and clipped, scrollable
  presentation. While open, keyboard, pointer, IME, clipboard, link, and
  workspace actions do not reach their normal targets.
- **Owner:** Venus owns viewer state, fixed native binding descriptors, scene,
  rendering, input precedence, and AccessKit projection. Eon owns the enabled
  popup catalog and must reserve Alt+Slash and Alt+0 through Alt+9 before
  full-product delivery.
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
  - **Override evidence (`eon-igae.10`):**
    `configured_popups_take_fixed_bindings_except_reserved_keys`; isolated
    dogfood of the built package on a private headless Sway (1280x720), with a
    Files popup configured on Alt+Shift+N: the key opened and hid the popup
    without a second window, and Alt+/ listed `Open new Eon window (overridden
    by Files)`.
  - **Detach row (`eon-prove-close-and-detach-batch-bf8t`):** isolated
    installed dogfood (edge `22915af5dca41f87e706ad548c0506f79c17648f`,
    installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, private headless
    Sway, 1280x720): the viewer listed `Alt+Shift+D` `Detach: close the
    window, keep terminals running` under Host.
  - **Sidebar rows (`eon-agent-sidebar-yx8.13`):** the installed dogfood
    recorded under VEN-C30: the viewer listed `Alt+Shift+H` `Show or hide the
    agent sidebar` under Host. F6 read `Cycle terminal, agent sidebar, tabs,
    and panes`, and Up and Down read `Previous pane or agent row while panes
    or the sidebar are focused` and `Next pane or agent row while panes or
    the sidebar are focused`.
  - **Attention-key rows (`eon-agent-sidebar-yx8.8`):** the installed dogfood
    recorded under VEN-C30: the viewer listed `Alt+G` `Jump to the next agent
    needing attention` and `Alt+Shift+G` `Jump to the previous agent needing
    attention` under Navigate, also with `sidebar` `Off`.
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
  height. Tabs keep their filled pills. Tab pills and the quota, CPU, RAM and clock widgets share
  one height and centre line, starting half the card title height below the window's top edge; the
  allocated card areas start below them so that the space between the pills and the tops of the card
  titles' capital letters equals the space above the pills. The widgets draw no fill: each is a
  rounded outline at the pane cards' neutral border width and outline color, with its text and icons on the opaque bar
  background. The version label stays plain text, and New tab, Keyboard shortcuts and Close centre
  on the same line. Hover and accessibility rectangles, order, width fitting and the blank drag
  region stay unchanged. Scrollable tabs keep the leading space and retain their labels,
  overflow, reveal, selection, hover path, stable identity, and accessibility.
  New tab follows the rightmost tab when tabs fit and stays at the tab-region
  edge during overflow. Keyboard shortcuts (when VEN-C24 requires the fallback)
  and active-tab Close remain trailing
  controls. Close tab is available on the final tab and goes through the same
  VEN-C8 final-close prompt as Alt+Shift+W. The buttons use the existing CreateTab and exact-active-tab CloseTab
  EONW actions and the existing local VEN-C20 viewer. Their tooltips and
  accessible names are
  `New tab — Alt+Shift+T`, `Keyboard shortcuts — Alt+/`, and
  `Close tab — Alt+Shift+W`. A button whose key a current Eon popup takes
  under VEN-C20 drops the chord from its tooltip and accessible name
  (`New tab`, `Close tab`). A bounded empty region between New tab and the
  trailing controls starts compositor-owned window movement only from its
  immediate left press.
  The accessibility tree orders tabs, New tab, optional quota, Shortcuts,
  then Close tab.
  It yields before tabs or controls when width contracts. Tab and Shift+Tab
  traverse focused header controls; Enter and Space activate them. Hover,
  pressed, and focus states remain distinct, and focus includes an outline.
- **Attention badges (EON-C31):** These rules apply while the agent sidebar is
  not visible (`Closed`, `Off`, or hidden for width):
  - every tab whose rollup is needs input, error, finished or working shows a
    corner badge (◆, ✕, ✓, or a spinner), the active tab included. A stale
    tab's badge is dimmed;
  - on a software (CPU) graphics adapter the working badge is a still open arc
    that never repaints to animate; otherwise the arc turns, repainting at no
    more than 5 Hz under AP-PERF-001, and only the badge;
  - the badge changes neither the tab's width nor its label text;
  - with tab overflow, the scroll edge shows the most urgent hidden badge;
  - a tab's accessible name appends its state, for example
    `Tab 3 of 5  /home/me/site, needs input`;
  - clicking a badged tab behaves like any tab click.
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
  dependency, native-decoration policy, or platform-support expansion. The one
  exception is the EON-C31 tab attention marker, read from the EONW v20
  snapshot that Venus already polls.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `eon_bar_controls_reuse_existing_actions_and_local_viewer`,
    `eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure`,
    `eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation`,
    and `workspace_accessibility_order_matches_tabs_then_one_expanded_pane` in
    the workspace suite; dogfood checklist items 3 and 6.
  - **Override evidence (`eon-igae.10`):**
    `eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation`; the
    installed observation is pending.
  - **Final-tab close (`eon-prove-close-and-detach-batch-bf8t`):** isolated
    installed dogfood (edge `22915af5dca41f87e706ad548c0506f79c17648f`,
    installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, private headless
    Sway, 1280x720): Close tab (`×`, tooltip `Close tab — Alt+Shift+W`) on
    the only tab showed the final-close prompt; Escape kept the tab and Enter
    ended the window.
  - **Attention badges (`eon-agent-sidebar-yx8.2`):** automated proof in
    `attention_badges_mark_tab_corners_without_moving_tabs`,
    `only_badges_wholly_inside_the_tab_viewport_are_drawn`,
    `spinner_phases_rewrite_only_alpha_in_place`,
    `spinner_turns_at_most_five_times_a_second_and_only_while_shown`, and the
    AccessKit label in
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`;
    isolated installed dogfood (edge `08e52ca`, the candidate's `nix build`
    of `/nix/store/i0j3p4bds3na8hha9j2f5f1gc828ga2v-eon-0.1.1`, private
    headless Sway at 1280x720 and 320x480):
    - with the sidebar closed, the four badge states showed. A real Codex
      0.163 turn in a background tab badged it working, then finished.
    - at 320x480, each scroll edge showed the most urgent badge hidden past it.
    - two captures 300 ms apart differed only in the 10x10 spinner box.
    - with the sidebar open the badges were hidden, and they returned when it
      hid for width.
    - Venus CPU over 20 s with and without a spinner: 14 and 6 ticks of
      10 ms with Sway rendering on the Intel GPU; 738 and 51 with Sway
      compositing in software (pixman).
  - **Still badge on software adapters (`eon-agent-sidebar-yx8.15`):**
    automated proof in
    `software_adapters_draw_working_badges_still_and_never_tick`; isolated
    installed observation (tree `bf7c28b` on edge `2112ff3`, the candidate's
    `nix build` of `/nix/store/mf0ypf0qpdbm8bjdb9rx3p6i0s6xm17a-eon-0.1.1`,
    private headless Sway with the pixman renderer at 1280x720, sidebar
    closed, one tab reporting working). Venus ran on lavapipe (16 llvmpipe
    threads, no render-engine time on its render node). The tab showed the
    still ring, and ten captures 100 ms apart were identical. Venus CPU over
    20 s: 62 ticks of 10 ms with the working badge and 65 without, against
    545 and 605 with the spinner and 62 and 47 without on the pre-change
    installed build.
  - **Both adapters on the installed product (`eon-agent-sidebar-yx8.9`):**
    isolated installed observation (edge
    `c8c62cb63938b145b13cfa4fe161ea5689807652`, installed `eon` profile
    `/nix/store/m5mzxy8cikz3f6y7k0ldgf7qxmc64bji-eon-0.1.1`, private headless
    Sway 1.12 at 1280x720, sidebar closed, one tab reporting working). In
    both runs, ten captures were taken 100 ms apart.
    - Under the pixman renderer, Venus ran on lavapipe (16 llvmpipe threads,
      no render-engine time). The tab showed the still ring, and the captures
      were identical.
    - Under Sway's Vulkan renderer on the Intel GPU, Venus had no llvmpipe
      threads and render-engine time on `renderD129`. The spinner turned,
      giving 7 distinct frames, and Venus used 11 ticks of 10 ms over 20 s.
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
  priority. Quota-only refreshes, and refreshes that differ only in EON-C31
  attention or sidebar visibility, do not recenter workspace navigation, cancel
  terminal scroll, replace the attachment, or send an Orbit resize, except that
  a change in whether the agent sidebar shows resizes the columns (VEN-C30). Venus has
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
  other provider, Orbit change, or platform expansion. One exception: the mark
  may also sit in the clickable, focusable rows of the agent sidebar
  (VEN-C30). Workspace transport loss
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
- **Trigger:** Physical Alt+Shift+N, including while Shortcuts is open, unless
  a current Eon popup takes that key under VEN-C20.
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
  - **Override evidence (`eon-igae.10`):**
    `configured_popups_take_fixed_bindings_except_reserved_keys`; isolated
    dogfood of the built package on a private headless Sway (1280x720), with a
    Files popup configured on Alt+Shift+N: the key opened and hid the popup
    without a second window, and Alt+/ listed `Open new Eon window (overridden
    by Files)`.

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
  The footer is the margin below the pane cards: its hints sit on the window
  background without a frame, band fill or separator, across the window's width,
  and its band reaches the window's bottom edge. The row is at least the edge
  padding tall. Hints use steel-blue modifier headers, mint keys and gray labels.
  Copy, Paste and Open link stay in full help and dispatch, without footer hints.
  Compact modifier headers use C/A/S aliases joined by hyphens, such as A-S.
  The compact tab range is 1..9; full help retains the tenth-tab Alt+0 binding.
  Short labels use keys and hints; one pane-move hint groups horizontal and vertical
  keys, and one tab-move hint holds the tab keys.
  Full help and accessible labels retain complete chords and action names.
  Creation labels name the tab, pane, column or window explicitly.
  Width pressure drops low-priority hints before help and the visibility
  toggle. Native focus-specific keys appear only in their applicable context.
  F6 stays in full help and keyboard navigation, without a footer hint.
  Project is omitted from hints; Alt+Z and its full-help entry remain available.
  Alt+Shift+H has the hint "sidebar", omitted while the EONW snapshot's
  `sidebar` is `Off` (VEN-C30). Alt+G and Alt+Shift+G have the hints "next
  agent" and "previous agent" under every `sidebar`.
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
  a current Eon popup may take the toggle's key under VEN-C20; Orbit owns
  resizing.
- **Consumes:** Existing VEN-C19/C20/C21, canonical EONW, and ORBF v2 / ORBS v13.
- **Boundary:** No new input mode, child-application key discovery, interactive
  hints, durable preference, generic widget registry, dependency, wire change
  or platform expansion, except that the hints read `sidebar` from EONW v20.
  Future Eon modes supply hints from their own actual bindings and replace
  contents at the same footer height.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated and installed.
  - **Evidence:** `shortcut_hints_follow_bindings_catalog_and_native_focus`,
    `shortcut_hints_drop_whole_pairs_and_rebuild_modifier_headers`, and
    `shortcut_hints_are_read_only_and_retire_hidden_help_actions` in the
    workspace suite; dogfood checklist item 5.
  - **Override evidence (`eon-igae.10`):**
    `configured_popups_take_fixed_bindings_except_reserved_keys`; the installed
    observation is pending.
  - **Sidebar hints (`eon-agent-sidebar-yx8.13`):** the installed dogfood
    recorded under VEN-C30: the footer showed `H sidebar` in the Alt+Shift
    group, and `Escape terminal` and `Up/Down agents` while a row had focus.
  - **Attention-key hints (`eon-agent-sidebar-yx8.8`):** the installed
    dogfood recorded under VEN-C30: the footer showed `G next agent` in the
    Alt group and `G previous agent` in the Alt+Shift group, also with
    `sidebar` `Off`.
- **Open proof:** Pending re-proof in this repository: native raster checks of
  the footer below the cards across widths and fractional scale, and read-only
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
  softened cyan joins the existing top row, by default (VEN-C29) at its edge,
  right before shortcut help when visible and before close. Its full version is
  available through hover and a static AccessKit label; it adds no focus target
  or action and is not a window-drag target.
  Width fitting hides it before required controls and active-tab reservation.
- **Important failures:** Absent input means no label; malformed or standalone input is
  rejected. Existing workspace/terminal behavior and footer bounds remain owned
  by VEN-C21/C24.
- **Owner:** Runtime supplies immutable product identity; Venus owns validation
  and native projection.
- **Boundary:** No EONW change, widget framework, version discovery, polling,
  dependency or platform expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-igae.3`
  - **Environment:** Automated and installed.
  - **Evidence:** `eon_version_is_bounded_workspace_identity` and
    `version_label_uses_only_spare_space_and_has_no_actions` in the workspace
    suite; dogfood checklist item 6; the edge placement after the readings in
    the installed observation recorded in `eon-igae.3`.
- **Open proof:** Pending re-proof in this repository: native narrow-width
  hiding, label pointer/drag inertness, and AT-SPI order.

## VEN-C26 — Claude quota beside Codex

- **Status:** Proven on x86_64 Linux by the automated tier; native quota
  rendering is pending re-proof in this repository. Installed Eon composition
  remains Eon's responsibility.
- **Trigger and result:** A complete canonical EONW snapshot supplies an
  optional Claude observation. Venus shows one read-only Claude chip per window
  Claude reports, five-hour then seven-day. Each chip has the Claude splat and
  that window's remaining percentage, led by its elapsed position such as
  `3d2h/7d` when Claude reports a reset, as on Codex's chip; a window without a
  reset shows no position. Each chip's hover and static accessible label name
  Claude, that window, freshness and how far into the window the observation
  is, or that its reset time is unavailable. Stale text keeps `old` on every
  chip, even in compact presentation.
- **Ownership:** Runtime alone supplies source facts, age and invalidation.
  Venus uses its existing Scene, renderer, hover and AccessKit owners. All
  provider chips stay in the existing row. Full labels become compact, then
  chips disappear one by one, the one closest to the tabs first, before
  required tabs, controls or drag space. Codex's mark, labels and provider
  policy remain unchanged.
- **Failure:** Absent facts, and any window Claude does not report, have no
  chip, gap or accessible node. Workspace
  transport loss removes both observations while keeping the last useful
  topology. Quota-only changes do not replace attachments, scroll positions or
  navigation. No quota chip has an action or keyboard focus target.
- **Consumes:** Canonical EONW quota fields; the in-tree Orbit codec stays
  unchanged.
- **Boundary:** `eon-claude-quota-eon-bar-zm5f` owns this slice. No
  provider parsing, credential access, client polling, new module, dependency,
  framework, row or platform promise.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `two_provider_chips_share_layout_without_losing_identity_or_freshness`
    and `quota_icons_preserve_transparency_and_shape_across_scales` in the
    workspace suite. One chip per Claude window and the closest-to-the-tabs
    falling order (`eon-claude-chip-split-u3ik`) are proved by the updated
    `two_provider_chips_share_layout_without_losing_identity_or_freshness`;
    that bead's commit is its revision.
- **Open proof:** Pending re-proof in this repository: the installed Claude
  chips beside Codex, native stale,
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
  No provider, protocol, dependency, module or platform change. One exception
  to that and to the static no-action behavior: the splat may also sit in the
  clickable, focusable rows of the agent sidebar (VEN-C30).
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`
  - **Environment:** Automated.
  - **Evidence:** `quota_icons_preserve_transparency_and_shape_across_scales` in
    the workspace suite.
- **Open proof:** Pending re-proof in this repository: the installed splat
  icon.

## VEN-C27 — Local clock in the Eon Bar

- **Status:** Proven on native x86_64 Linux Wayland.
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace.
- **Trigger:** A workspace surface is presented, or the local wall-clock minute
  changes.
- **Result:** One read-only `HH:MM` label in terminal monospace shows local
  24-hour time in the existing top row in the quota chips' rounded outline, by
  default (VEN-C29) after the quota chips and the CPU and RAM labels and before
  the version label, Shortcuts and Close. It changes within a second of
  each minute boundary. Hover and a static AccessKit label name the time, date
  and UTC offset, such as `Local time 14:05, Wednesday 21 October 2026,
  UTC−03:00`.
  The clock uses only spare drag-region space and keeps that region's minimum;
  it appears only while every quota chip is at full width, so width fitting
  hides it before the version label, quota compaction, tabs or controls.
- **Important failures:** An undeterminable local time zone shows no clock,
  never UTC in its place. A clock-only change does not replace attachments,
  recenter navigation, cancel scrolling or send an Orbit resize. The clock has
  no action, focus target or drag behavior. After suspend or a system clock
  change it corrects within one minute.
- **Owner:** Venus Scene owns the text and geometry, the application owns the
  minute wake, and jiff resolves the system time zone from `TZ` or
  `/etc/localtime`.
- **Boundary:** No EONW or runtime change, configuration, 12-hour mode,
  calendar, other time zone, widget framework or platform expansion. Its one
  dependency is `jiff` (`docs/CRATES.md`).
- **Proof:** `eon-igae.4`, `eon-igae.3`
  - **Environment:** Automated and installed.
  - **Evidence:** `local_clock_names_minute_date_and_offset` and
    `local_clock_yields_first_and_has_no_actions` in the workspace suite;
    isolated installed observations recorded in `eon-igae.4` and, for the
    rounded fill and placement before the version label, `eon-igae.3`.

## VEN-C28 — CPU and memory use in the Eon Bar

- **Status:** Proven on native x86_64 Linux Wayland.
- **Consumer:** One person using a Venus surface controlled by a full Eon
  workspace on Linux.
- **Trigger:** A workspace surface is presented, and every 2 seconds after.
- **Result:** Two read-only labels in terminal monospace, `CPU 12%` then
  `RAM 43%`, sit in the existing top row, by default (VEN-C29) after the
  quota chips and before the clock, each in the quota chips' rounded outline.
  Each chip keeps the width of `CPU 100%` whatever the value, with its text
  centered, so neighbouring items never shift. The CPU label shows
  whole-machine CPU use over the last 2 seconds as a whole percentage from `/proc/stat`; it
  first appears one interval after the surface opens. The RAM label shows
  whole-machine memory use, MemTotal minus MemAvailable over MemTotal from
  `/proc/meminfo`, as a whole percentage read on the same wake; it appears with
  the first reading. Hover and static AccessKit labels say `CPU use 12% across
  16 logical CPUs, over the last 2 seconds`, counting the later sample's
  per-CPU `/proc/stat` lines, and `RAM use 43% of 31.3 GiB`. The labels
  use only spare drag-region space and keep that region's minimum; they appear
  only while every quota chip is at full width, so width fitting hides the
  clock first, then the RAM label, then the CPU label, before the version
  label, quota compaction, tabs or controls.
- **Important failures:** An unreadable, malformed or non-advancing CPU
  sample, one without per-CPU lines, or an unreadable or malformed memory
  reading, one without MemTotal or MemAvailable, with a zero MemTotal or with
  MemAvailable above MemTotal, shows no label for that reading, never 0%; a
  hidden label returns with the next valid reading. A change to either reading
  does not replace attachments, recenter navigation, cancel scrolling or send
  an Orbit resize. Neither label has an action, focus target or drag behavior.
  Other platforms show neither label.
- **Owner:** Venus Scene owns the text and geometry, the application owns the
  2-second wake, and a Venus module reads and parses `/proc/stat` and
  `/proc/meminfo` on Linux.
- **Boundary:** No EONW or runtime change, configuration, per-core view, swap,
  per-process or GPU memory, history, graph, dependency or platform expansion.
- **Proof:** `eon-cpu-widget-eon-bar-acxs`, `eon-igae.3`, `eon-igae.14.4`,
  `eon-icoh`
  - **Environment:** Automated and installed.
  - **Evidence:** `cpu_use_counts_iowait_idle_and_guest_once`,
    `cpu_use_rejects_bad_readings_instead_of_showing_zero`,
    `ram_use_is_total_minus_available_over_total`,
    `ram_use_rejects_bad_readings_instead_of_showing_zero` and
    `readings_yield_after_the_clock_and_have_no_actions` in the Venus suite;
    isolated installed observations recorded in
    `eon-cpu-widget-eon-bar-acxs`, `eon-igae.3`, `eon-igae.14.4`, for the
    one-space label centered in its fixed-width chip, and `eon-icoh`, for the
    hover text naming the logical CPU count.

## VEN-C29 — Eon Bar item order

- **Status:** Proven by the automated tier; installed placement of a chosen
  order is pending `eon-igae.14.2`, which makes Eon pass the input.
- **Consumer:** Eon Runtime launching a Venus surface for a full Eon workspace.
- **Trigger:** A workspace launch with an optional `--bar-items ITEM,...`
  naming any of `codex_quota`, `claude_quota`, `cpu`, `ram`, `clock` and
  `version`, each at most once.
- **Result:** The named items sit left to right in that order, between the
  drag region and Shortcuts and Close. An item left out gets no chip, hit
  target, hover or AccessKit node, and Venus neither reads nor wakes for a
  left-out CPU, RAM or clock. An empty value shows no items. Without the input
  the order is `codex_quota`, `claude_quota`, `cpu`, `ram`, `clock`, `version`.
  AccessKit order follows the visual order. Width fitting keeps its fixed
  priority whatever the order: the clock hides first, then RAM, then CPU, then
  the version label, then quota compaction, before tabs or controls.
- **Important failures:** A missing value, an unknown, repeated or empty name,
  a repeated flag, or the input without `--workspace` fails before window
  creation. A named item without data (no quota from EONW, no `--eon-version`,
  no valid reading) shows nothing, as its own contract says.
- **Owner:** Eon owns the choice and passes it at launch; Venus owns
  validation, placement and fitting.
- **Boundary:** No EONW change, configuration file, per-item option,
  configurable hiding priority or new item.
- **Proof:** `eon-igae.14.1`
  - **Environment:** Automated; the default bar observed installed.
  - **Evidence:** `bar_items_are_an_ordered_workspace_choice` and
    `eon_orders_and_omits_bar_items` in the Venus suite; the existing
    VEN-C25, VEN-C27 and VEN-C28 tests for the default order; the installed
    observation recorded in `eon-igae.14.1`.

## VEN-C30 — Agent sidebar

- **Status:** Partially proved: on x86_64 Linux Wayland every clause is proved
  except compositor blur through the card, which waits on VEN-C15's native
  backdrop re-proof, and the current row's outline over a hovered row's
  highlight, which no isolated capture could produce
  (`eon-sidebar-you-are-here-3743`). Approved by Lucca on 2026-10-21 (draft 15 on
  `eon-agent-sidebar-yx8.1`, there named VEN-C29). `eon-agent-sidebar-yx8.3`
  implements visibility, layout and hiding for width, Alt+Shift+H, the
  sidebar's AccessKit region and the empty state;
  `eon-agent-sidebar-yx8.13` implements contents, row navigation, F6 and row
  accessibility; `eon-agent-sidebar-yx8.8` implements Alt+G and
  Alt+Shift+G; `eon-agent-sidebar-yx8.11` adds the name;
  `eon-agent-sidebar-yx8.9` proves the epic on the installed product;
  `eon-agent-sidebar-yx8.16` adds the shown-popup marker, approved by Lucca
  on 2026-10-23; `eon-popup-marker-always-52rk` keeps the marker on every
  popup's row and lists popup rows first, approved by Lucca through
  eon-overseer the same day; `eon-sidebar-you-are-here-3743` replaces the
  header marker with the current row, approved by Lucca through eon-overseer
  on 2026-10-23; after installed dogfood the same day, Lucca replaced its
  first tint with an outline.
- **Consumer:** One person using a Venus window attached to a full Eon
  workspace. The sidebar itself needs `sidebar` other than `Off`. Alt+G and
  Alt+Shift+G work under `Off` too.
- **Trigger:**
  - Alt+Shift+H;
  - a new EONW v20 snapshot;
  - the pointer, keyboard or AccessKit acts on the sidebar;
  - the window is resized.
- **Result:**
  - **Visibility.** The sidebar shows while the snapshot's `sidebar` is
    `Open`, on every tab of the window. Alt+Shift+H sends `ToggleSidebar`, so
    one press always flips the choice between open and closed. That shows on
    screen unless the window is too narrow (see Layout).
  - **Layout.**
    - A left sidebar 28 cells wide sits below the Eon Bar. The bar keeps its
      full width, its controls and its drag region.
    - Its card uses the border width and the pane cards' corner radius, with
      "Agents" in its rounded top edge in the pane-title style, using the shared
      chrome title metric. Where no row is highlighted, its fill, from its rounded
      top edge down, takes the background of the terminal in the leftmost visible
      column at the terminal-background opacity (VEN-C11), so it reads like the panes
      and compositor blur (VEN-C15) shows through it as through them. Focused
      and hovered rows take an
      opaque highlight. Without pane frames, where pane cards have no outline
      either, that column's background reaches across the sidebar's area.
    - It pushes the tab columns, so opening or closing it resizes the visible
      terminals once.
    - Venus hides it when the total remaining column area would drop below 80
      cells. This leaves `sidebar` unchanged, and VEN-C21's markers then show.
    - Above that threshold, individual columns may still drop below 40 cells,
      as VEN-C8 already allows when the window shrinks.
    - While the sidebar is hidden for width, Alt+Shift+H still toggles
      `sidebar`. The new value applies once the window is wide enough.
    - **VEN-C8 and VEN-C18 amendment.** While the sidebar is visible:
      - the side gaps and the 40-column CreateColumn check are measured from
        the content area to its right;
      - the first tab pill keeps its window-edge position, and its alignment
        with the first card is suspended;
      - VEN-C18's popup card area is that content area, so popups never cover
        the sidebar.
  - **Contents.**
    - It lists, in tab order, only tabs that have attention or an Agent popup.
    - Each such tab gets a header (`3  site`), followed by one row per
      terminal with attention.
    - A row shows:
      - the state glyph;
      - the app icon: Venus maps `app=claude-code` to the Claude splat
        (VEN-C26) and `app=codex` to the Codex mark (VEN-C22);
      - one label, the first that is not empty of: the attention `name`, for
        `app=claude-code` only, which Claude Code sets to its session name or
        title; the record title; and the app's name (`Claude`, `Codex`, or any
        other `app` verbatim), else `agent`. Venus maps `app` to an icon and to whether its
        terminal title names the session; other programs' terminal titles,
        such as Codex's spinner title or a shell's command line, never lead a
        row. It never falls back to the tab's directory leaf, which the tab's
        header shows. An Agent popup without records shows "status
        unavailable";
      - the age;
      - the source terminal, by the pane's `pN` identity as in pane headers
        and AccessKit (VEN-C8), or, for a popup, its catalog label such as
        "Agent popup", because the spec says shown status should name its
        source terminal.
    - A stale record is dimmed and labelled "status stale".
    - A needs-input row adds one plain-text, truncated `msg` line.
    - Other records of a terminal fold into "+N".
    - Rows never reorder by priority.
    - Within a tab, a row whose source terminal is a popup comes first; pane
      rows follow in pane order.
    - **Popup marker.** A row whose source terminal is a popup always shows
      the popup marker glyph after its catalog label, and its accessible label
      adds "popup". Tab headers carry no marker.
    - **Current row.** The row whose source terminal is the active tab's
      focused terminal (its shown popup, else its selected pane) takes a
      one-pixel outline in the pane cards' outline color. It draws over the
      opaque highlight of a focused or hovered row, and a focused row's accent
      outline draws over it; the row keeps the outline while the sidebar holds
      keyboard focus. Rows in other tabs never take it. Its accessible label
      adds "current".
  - **Navigation.**
    - Activating a row sends `FocusAttention`, and keyboard focus moves to the
      target terminal, or stays on the tab when the covered-target rule only
      selects the tab. It never stops a terminal.
    - F6 cycles Terminal → Sidebar → Tabs → Panes, and skips Sidebar while it
      is not visible or has no rows, since the empty state holds no focus
      target. This amends the F6 order in VEN-C18 and
      `venus/docs/INTERACTION.md`, and the F6 label in
      `venus/src/application/shortcuts.rs`.
    - Arrows traverse rows, Enter activates and Esc returns focus to the
      terminal.
    - **Focus rule.** Alt+Shift+H never moves keyboard focus into the sidebar;
      focus stays on the terminal. Keyboard focus returns to the terminal in
      three cases:
      - the sidebar closes while it holds focus;
      - it is hidden for width while it holds focus;
      - the focused row's target is removed.
    - Alt+G and Alt+Shift+G send `NextAttention` and `PreviousAttention`,
      whether the sidebar is visible or not.
    - Venus shows the `no-attention` rejection as the VEN-C8 notice `No agent needs
      attention`, without taking focus. Unlike other rejected-action notices, it also clears
      when a terminal's attention state changes and on the next input Venus sends to the
      terminal, as the clipboard notice does.
  - **Accessibility.** The sidebar is one named AccessKit region after the
    header and before the panes. Each row's accessible name gives the state
    word, app, Claude Code's session name, tab, source terminal and message.
- **Important failures:**
  - With no attention, the sidebar shows an explanatory empty state.
  - The sidebar never takes desktop focus and never changes Orbit state.
  - Record text renders only as plain text, never as markup or links.
- **Owner:**
  - **Venus** owns layout, hiding for width, rendering, input and
    accessibility.
  - **Eon** owns state, visibility and actions (EON-C31).
- **Attention-only refreshes.** A snapshot that differs only in attention or
  `sidebar` keeps rejected-action notices (VEN-C8) and disturbs no navigation,
  scroll, attachment or resize, as quota-only refreshes already do (VEN-C22);
  a change in the sidebar's visibility resizes the columns as Layout says.
  VEN-C8, VEN-C22 and EON-C8 carry this clause.
- **Consumes:** EONW v20 and the VEN-C22 and VEN-C26 icons, under the
  exceptions those contracts record for sidebar rows.
- **Boundary:**
  - no interpretation of provider events, terminal output or process data,
    except mapping the reported `app` identifier to an icon and to whether
    its terminal title names the session;
  - no drag resizing, compact rail or vertical tabs;
  - no previews, no priority reordering and no notifications.
- **Proof:** The checks and observations below; Open proof lists what they
  leave out.
  - **Automated:** `eon-agent-sidebar-yx8.3` adds
    `agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state`,
    `agent_sidebar_region_follows_the_header_and_names_its_empty_state`,
    `direct_workspace_shortcuts_use_stable_eon_actions` and
    `shortcut_hints_follow_bindings_catalog_and_native_focus`;
    `eon-agent-sidebar-yx8.13` adds
    `agent_sidebar_rows_show_attention_in_tab_order_and_reveal_the_focused_row`,
    `agent_sidebar_ages_count_whole_minutes_hours_and_days`,
    `card_backdrop_keeps_cards_like_the_agent_sidebar_at_terminal_opacity`,
    `sidebar_rows_take_f6_focus_step_by_arrow_and_follow_their_jump`, and the
    row names in
    `agent_sidebar_region_follows_the_header_and_names_its_empty_state`;
    `quota_refresh_and_workspace_failures_preserve_structure` covers
    attention-only refreshes. These automated checks alone prove AccessKit
    names, "status stale" included; their pure helpers alone prove Up between
    rows and
    focus ending on the tab strip after a covered-target jump.
    `eon-agent-sidebar-yx8.8` extends
    `direct_workspace_shortcuts_use_stable_eon_actions`,
    `shortcut_hints_follow_bindings_catalog_and_native_focus` and
    `configured_popups_take_fixed_bindings_except_reserved_keys` to Alt+G and
    Alt+Shift+G. `eon-agent-sidebar-yx8.16` extends
    `agent_sidebar_rows_show_attention_in_tab_order_and_reveal_the_focused_row`
    to the shown-popup marker: which heading and row carry it, where it falls
    in the row's detail line, and the "popup shown" names, and adds
    `sidebar_marker_follows_the_shaped_ink_of_either_direction` for its
    placement after right-to-left text. `eon-popup-marker-always-52rk`
    changes the first test so a popup's row comes before its tab's pane rows,
    a hidden popup's row keeps the marker, and every popup row's name adds
    "popup". `eon-sidebar-you-are-here-3743` changes it so no heading carries
    the marker, and extends it to the current row: none while the active tab
    focuses a terminal without attention, the focused pane's row while the
    sidebar's own focus is on another row, the shown popup's row, never a row
    in another tab, and the "current" names; it extends
    `agent_sidebar_region_follows_the_header_and_names_its_empty_state` to
    the "current" name in the AccessKit tree.
  - **Installed (`eon-agent-sidebar-yx8.13`, on the sidebar batch's final
    revision):** isolated installed dogfood (edge `d6ce117`, code tree
    `a0d664d`, whose `nix build` gave
    `/nix/store/p6mzlpdlk24k6v12plxcc99hyq9m1ghi-eon-0.1.1`, x86_64 Linux,
    private headless Sway at 1280x720, Eon's default `background_opacity`
    0.80; three runs: the main run, one with `pane_frames = false`, and one
    for the focus returns). Three tabs ran a
    Claude Code fixture that needs input with a title, a `msg` and a second
    record, a real Codex 0.163 turn, and an idle `app=aider` fixture.
    - Rows showed the splat, the Codex mark, `aider · w`, the `msg` line,
      `+1`, and the needs-input, working, finished and idle glyphs.
    - The card's outline matched the pane card's. Its background measured
      (4,5,7) over Sway's #141821, the same as the pane's, while the frame
      stayed (10,13,20). Without pane frames, the first column's background
      ran across the sidebar's area. The g in "Agents" was whole. A hovered
      or focused row read (28,43,58), its highlight color unblended, and
      went back to the card's (4,5,7) when the pointer or focus left.
    - A click on a tab-1 row from tab 3 activated tab 1, and a typed character
      echoed in its terminal.
    - F6 focused the first row, Down the next, and Enter jumped to its tab,
      where typing echoed. F6 then returned to that row and next to the tabs,
      and Escape returned to the terminal. The footer showed `H sidebar` in
      its Alt+Shift group, and `Escape terminal` and `Up/Down agents` while
      a row had focus.
    - Alt+Shift+H hid and showed the sidebar without moving terminal focus,
      and from a focused row it returned focus to the terminal.
    - At 1000x720 the sidebar hid for width, the tab badge returned and F6
      skipped it; at 1280x720 it came back.
    - With a row focused, hiding for width and `eon tab close` of the row's
      target tab each returned focus to the terminal, where typing echoed.
    - In an idle window, a `now` row read `1m` a minute later.
    - The Git popup's card sat beside the sidebar.
    - With no attention left, the empty state showed and F6 skipped the
      sidebar.
  - **Installed (`eon-agent-sidebar-yx8.8`):** isolated installed dogfood
    (edge `0e3912b`, code tree `bce96f9`, whose `nix build` gave
    `/nix/store/r18mya95p393gbd2jsmhmjw785cazk4g-eon-0.1.1`, x86_64 Linux,
    private headless Sway, 1280x720; two runs, the second
    with `[sidebar] enabled = false`). Tabs 1 and 2 each ran a Claude Code
    fixture that needs input, and tab 3, the active one, an idle `app=aider`
    fixture.
    - From tab 3, Alt+G selected tab 1, then tab 2, then tab 1 again;
      Alt+Shift+G then selected tab 2 and tab 1. `eon workspace` read the
      active tab after each press.
    - After Alt+G reached tab 1, a typed `x` echoed in its terminal, where
      no `g` appeared.
    - With the cursor on tab 1, Alt+3 selected tab 3, and the next Alt+G
      started again at tab 1 rather than going on to tab 2.
    - The footer showed `G next agent` in its Alt group and `G previous
      agent` in its Alt+Shift group. Alt+/ listed `Alt+G` `Jump to the next
      agent needing attention` and `Alt+Shift+G` `Jump to the previous agent
      needing attention` under Navigate.
    - With `sidebar` `Off`, no sidebar showed and the tab badges did; Alt+G
      selected tab 1 and then tab 2, and Alt+Shift+G selected tab 1. The
      footer and viewer listed both keys, and the footer omitted `H sidebar`.
  - **Installed (`eon-agent-sidebar-yx8.9`):** isolated installed dogfood
    (edge `c8c62cb63938b145b13cfa4fe161ea5689807652`, installed `eon` profile
    `/nix/store/m5mzxy8cikz3f6y7k0ldgf7qxmc64bji-eon-0.1.1`, x86_64 Linux,
    private headless Sway 1.12 at 1280x720, the pixman renderer, where Venus
    used lavapipe; two further runs: one with no attention and one with a
    Markdown fixture).

    The main run had six tabs:
    1. a real Claude Code 2.1.303 (`--name proof-claude`);
    2. a real Codex 0.164.0;
    3. a burst fixture, and a fixture that needs input whose title holds HTML
       and a Markdown link and whose `msg` holds a URL, HTML and Markdown;
    4. a quiet pane;
    5. a working fixture;
    6. a fixture that needs input, raised after a delay.

    Tabs 3 and 4 each showed a keep-alive Agent popup running Claude Code
    2.1.293.

    Observations:
    - Rows showed `proof-claude` and `Codex` needing input and, after
      approval, Codex finished. `eon workspace` read the working and finished
      states that came between captures. The Agent popup row read
      `status unavailable`.
    - The fixture's `msg` printed `Open https://example.com/<i` as plain text,
      cut at the row width, with no link styling. In the Markdown run, a
      Codex-app fixture's title `# H **t**` led its row and its `msg`
      ``**b** _i_ `c` [l](u)`` filled the message line, both as literal
      characters in the row's plain font, with no heading, emphasis, code or
      link rendering.
    - A stopped Orbit host's row dimmed with `status stale` and recovered.
    - F6 focused a row, Down moved to the Codex row, and Space jumped to its
      tab, where the footer returned to terminal hints.
    - Alt+Shift+H, and hiding at 1000x720, each left typing echoing in the
      active terminal. With the sidebar closed, tab 3 showed its ◆ badge.
    - A keep-alive popup covering the target was hidden by Alt+G and kept
      running.
    - On an empty queue, the empty state showed and Alt+G changed nothing.
    - Desktop focus and Orbit state:
      - A foot window held desktop focus on another Sway workspace while
        attention changed. Every Sway sample kept focus on foot and showed
        no urgent window.
      - With the sidebar closed, tab 6's needs input neither opened the
        sidebar nor changed the active tab.
      - At the last process check every terminal, agent and popup was still
        running.
    - Tab 5's working row was the still ring shared with VEN-C21's badge. One
      set of 10 captures with the sidebar open, taken 100 ms apart, gave 1
      distinct frame. An earlier set, taken right after the tab opened,
      spanned 10.6 s instead of 1 s and gave 2; its cause was not determined.
  - **Installed (`eon-agent-sidebar-yx8.16`):** isolated installed dogfood
    (edge `c94787b` with code tree `9cd204a`, whose `nix build` gave
    `/nix/store/5cb6sdx4irfd8kd87nrldmz9ry94bi0v-eon-0.1.1`, x86_64 Linux,
    private headless Sway 1.12 at 1280x720, the pixman renderer). The
    commit's rebase onto edge `899818f` brought in only Runtime CLI, Nix,
    Orbit and docs changes, none in Venus. Tab 2's Agent popup ran a fixture reporting
    `app=claude-code` working.
    - With the popup shown in tab 2, its heading read `2  w` and then the
      window marker, and its row read `Claude` over `now · Agent` and then
      the marker.
    - After Alt+1, `eon workspace` read tab 1 active and tab 2 `popup=u3`,
      and tab 2's heading and row still showed the marker.
    - After the popup was hidden in tab 2 and Alt+1 pressed again, `eon
      workspace` read tab 2 `popup=none`, still listing popup `u3` with its
      working attention. Its row was still listed, and neither the heading nor the row showed a marker.
  - **Installed (`eon-popup-marker-always-52rk`):** isolated installed
    dogfood (edge `6ec880b` with code tree `cc5bded`, whose `nix build` gave
    `/nix/store/gw7yy7vznacjk1nm5537jhr2zzpbg53f-eon-0.1.1`, x86_64 Linux,
    private headless Sway 1.12 at 1280x720, the pixman renderer). Tab 2's
    Agent popup ran a fixture reporting `app=claude-code` working, and its
    pane `p2` ran one reporting `app=codex` working. The
    `eon-agent-sidebar-yx8.16` record's unmarked hidden-popup row no longer
    holds.
    - With the popup shown in tab 2 (`eon workspace`: `popup=u3`), its
      heading read `2  w` and then the marker; the first row read `Claude`
      over `now · Agent` and then the marker, and the second row read
      `Codex` over `now · p2`, with no marker.
    - After the popup was hidden (`popup=none`, still listing `u3` with its
      working attention), the heading read `2  w` with no marker, and the
      Agent row stayed first and kept its marker, above the `p2` row. After
      Alt+1, with tab 1 active, the sidebar showed the same.
  - **Installed (`eon-sidebar-you-are-here-3743`):** isolated installed
    dogfood (edge `f6ff36e`, the `eon` profile at
    `/nix/store/9lpcb6pxy4z4hq046b0j5cbxfgxa7xb9-eon-0.1.1`, x86_64 Linux,
    private headless Sway 1.12 at 1280x720, the pixman renderer, the pointer
    moved off the sidebar), with the same fixtures in tab 2's Agent popup and
    pane `p2`.
    - With the popup shown (`eon workspace`: `selected=p2 popup=u3`), the
      Agent row's edge sampled (58,75,91), the pane cards' outline color,
      and the `p2` row's edges the card's (4,5,7).
    - After the popup was hidden (`popup=none`), the `p2` row's edge sampled
      (58,75,91) and the Agent row's the card's (4,5,7), and the heading read
      `2  w` with no marker.
- **Open proof:**
  - Compositor blur through the card: headless Sway has no blur, so only its
    translucency was measured. It waits on VEN-C15's native backdrop
    re-proof.
  - The current row's outline drawn over a hovered row's highlight
    (`eon-sidebar-you-are-here-3743`): in the isolated captures neither a
    `swaymsg` pointer warp nor a relative move gave a Venus row hover, so
    only review of `venus/src/render/chrome.rs` checks that order.
