# Eon contract index

This is the canonical current state of Eon product behavior, ownership, proof,
and remaining limitations. Owning Beads and Git retain execution history;
`CHANGELOG.md` retains accepted user-visible chronology.

## Status

- **Planned:** accepted intent without implementation evidence
- **Candidate:** implemented and mechanically verified without an accepted final
  proof revision
- **Partially proved:** one exact useful slice is proved with a required gap
- **Proven:** an exact check, component set, platform and artifact cover the
  contract
- **Retired:** explicitly replaced or removed; its ID is never reused

Proofs cite revisions of this repository. "Pending re-proof in this repository"
marks an earlier proof that was not repeated after consolidation; it is
re-proved when that platform or feature is next touched.

The [component manifest](../components/eon-alpha-v3.json) owns selected source
and compatibility identities. Every proof below applies only to its named
revision, artifact, environment and surface; it does not promote a later pin or
another platform. Profile refresh preserves live supervisors and terminals;
existing work retains its exact runtime until an explicit restart.

The consolidation re-proof at `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, owned
by `eon-monorepo-2oko.6`, has two tiers. The automated tier is sandboxed
`cargo test --locked --offline --workspace` and `nix flake check` (checks
`default`, `eonterm-closure`, `helix-grammars`, `managed-command-path` and
`shell-environment`) on x86_64 Linux. The installed tier is one dogfood session
of the installed Nix profile `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0`
and `/nix/store/hk9v6q6c42x4sagpzrk20q32znw9sal1-eonterm-0.1.0`, driven
headlessly by a Kinestra recipe on a private headless Sway (wlroots, pixman) on
x86_64 Linux, following that bead's numbered checklist and recorded results.

## Rules

Product vocabulary follows the [naming boundary](ARCHITECTURE.md#naming-boundary):
workspace, tab, pane and terminal. Human help, output and diagnostics use
terminal for each persistent Orbit-backed instance. Technical session IDs,
socket names, JSON fields and protocol identifiers retain their spellings.

- Each contract uses one `## EON-CN — Name` heading and the required fields
  `Status`, `Consumer`, `Trigger`, `Result`, `Important failures`, `Owner`,
  `Boundary`, and `Proof`.
- A retired contract keeps only its heading, a `Status` naming the date,
  approver and replacement, and a `Reason`.
- Add optional `Consumes` and `Open proof` fields when applicable; nest
  `Environment` and `Evidence` under `Proof`, and use nested bullets instead of
  prose table cells.
- Contract IDs are stable and repository-qualified. Never renumber or reuse an
  ID; mark an explicitly removed contract retired.
- Only current user-visible behavior, correctness boundaries, ownership
  invariants, and interfaces between subsystems belong here.
- Every implementation Bead names the contracts it changes, proves, consumes,
  hardens, or preserves and records exact child revisions.
- Historical execution evidence belongs in Beads and Git, not this current-state
  index. User-visible chronology belongs in `CHANGELOG.md`.
- Later work touching a proven owner reruns its indexed checks. Its owning Bead
  records the result in the commit that carries the change. Edit this index only
  when a contract, its checks, status, or gap changes; existing proof records
  stay valid as written.

## EON-C1 — Exact compatible component launch

- **Status:** Proven
- **Consumer:** A user launching Eon or EonTerm from one accepted product
  generation.
- **Trigger:** Eon starts a new composed runtime or adopts an exact live run.
- **Result:**
  - Eon validates one compatible component set, launches only that set, crosses
    each required Ready boundary before publication, and reports every exact
    external component revision and the generation of its in-tree build.
  - `assets/eon.png` is the transparent canonical application icon; Nix derives
    exact hicolor sizes and installed desktop metadata names `eon`.
  - The installed launcher action is named `Open Eon` and invokes the exact
    packaged executable. It may start or present the exact current generation,
    so it promises neither a new window nor a new terminal; live surfaces retain
    terminal-authored titles.
  - Compatible terminal output leaves scrolling, selection, and full-Eon tab
    and pane focus usable between repaints. Orbit continues parsing output and
    answering terminal queries while the user reads retained history; history
    bounds, geometry validation, and synchronized presentation remain intact.
    Venus shows a bounded scrollback-position indicator; activating its visible
    ReturnToLive control returns to live rows without Eon owning viewport state.
    Orbit owns words at two clicks, space/tab-delimited punctuation spans at
    three and logical lines at four or later, with native drag and frozen copy.
  - Full Eon can invoke the manifest-pinned Anima executable for the optional
    fresh-workspace welcome step and `eon anima [style] [child options]`.
    The direct command uses the caller's terminal and returns the child's
    exit result without starting or attaching a workspace.
  - Optional `[terminal] cursor_trail_color` is forwarded unchanged on new or
    reopened surfaces. Omission emits no override; Venus keeps one random
    preset per live supervisor endpoint, so a full-Eon window reopened by
    `eon attach` keeps its cursor color. EonTerm's first window opens before
    its terminal endpoint exists, so it may differ from later reopens, which
    share one preset. Venus owns `random`, `preset:<name>` and
    `custom:#RRGGBB` parsing, colors and rendering; Orbit cursor authority remains.
    Native admission rejects an invalid choice before a new terminal starts;
    rejected reopen preserves the existing terminal and command.
  - Standalone `eon -h` and `eon --help` print grouped command descriptions and
    examples to stdout and exit successfully without starting children or
    preparing configuration/runtime state. Terminal output uses color unless
    `NO_COLOR` is nonempty or `TERM=dumb`; redirected output has no ANSI escapes.
    Invalid syntax remains a concise stderr failure pointing to `eon --help`.
    Child arguments, including Anima help, retain their existing forwarding.
  - Eon owns the cursor-tail duration default `1.5` and the optional
    `[terminal] cursor_trail_duration` multiplier in the same config file.
    Finite values from `0.25` through `4.0` apply on new/reopened Eon and EonTerm
    surfaces. Invalid duration fails before children; live surfaces retain their
    startup snapshot. Venus consumes the launch value and owns bounded motion,
    including healthy pane/tab/layout continuity. No separate child configuration.
- **Important failures:** Missing, malformed, incompatible, replaced, or
  non-ready components fail before publishing a usable workspace or terminal.
- **Owner:** Eon graph validation, icon selection and desktop metadata; Eon
  Runtime implements launch orchestration. Nix derives/installs artifacts;
  child components own their internal readiness and state.
- **Consumes:** The versioned Eon component manifest, Orbit management/ORBS, and
  Venus native launch contracts.
- **Boundary:** Eon does not infer compatibility from executables, store paths,
  process names, or moving branches.
- **Assembly boundary:** `product.rs` supplies fallible validated component
  facts to the current runtime; graph failures precede runtime ownership effects,
  including `window new` parent-directory creation. Eon Runtime in `runtime/`
  owns the consumed runtime. See the
  [runtime input contract](ARCHITECTURE.md#ownership-and-invocation).
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Graph validation and version report
    (`canonical_manifest_is_valid`, `invalid_graphs_are_rejected`,
    `version_report_contains_identity_without_resolved_paths`); help
    (`help_is_readable_and_does_not_start_or_prepare_a_workspace`); Anima
    forwarding (`anima_command_preserves_child_arguments_status_and_avoids_workspace`);
    cursor-tail launch values (`configured_cursor_tail_reaches_both_product_launches`);
    failed native admission (`rejected_native_startup_creates_no_session_in_either_product`);
    the `default` and `eonterm-closure` Nix checks. Installed checklist items
    1 (Anima welcome, picker, first terminal) and 7 (terminal text including
    box drawing, Braille and CJK); Lucca opened the installed build from the
    `Open Eon` desktop entry on COSMIC (recorded in `eon-monorepo-2oko.6`).
- **Open proof:** Pending re-proof in this repository: installed
  `eon anima accelerando` playback, installed scrollback,
  ReturnToLive, click-selection ladder and copy/paste, installed help color
  gates (`NO_COLOR`, `TERM=dumb`, 80-column PTY), explicit cursor-trail colors
  and durations on new and reopened surfaces, hicolor icon sizes, retained
  history and terminal-query replies during synchronized-output bursts, and
  COSMIC sessions. Fractional scale, broader compositors and screen-reader use
  remain unproved.

## EON-C2 — One component compatibility graph

- **Status:** Candidate
- **Consumer:** Every Eon distribution and runtime launch path.
- **Trigger:** A product generation resolves or validates its component set.
- **Result:** One versioned manifest defines component identity, compatibility,
  and abstract artifacts for every channel without encoding launch policy.
  Orbit, Venus, Eon Runtime and EONW are in-tree components: each names its
  workspace path, and the repository revision identifies it. External
  components keep an exact Git revision; Anima is a pinned tool with one
  executable artifact. `eon versions` reports `workspace` as the revision of an
  in-tree component.
- **Important failures:** Unknown components, duplicate identities, incompatible
  protocol versions, malformed or missing external revisions, a revision on an
  in-tree component or on any requirement, invalid workspace paths, or
  channel-specific graph drift fail validation.
- **Owner:** Eon's component manifest and validator.
- **Boundary:** The manifest does not own process lifecycle, platform launch
  mechanics, package-manager policy, or child state. In-tree contract and
  interface proofs remain on their components.
- **Proof:** Candidate; mechanically verified at
  `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier on x86_64 Linux.
  - **Evidence:** `canonical_manifest_is_valid`,
    `removed_graph_fields_are_rejected`, `invalid_graphs_are_rejected` and
    `version_report_contains_identity_without_resolved_paths`; both Nix
    products build from the one graph.

## EON-C3 — Distribution-neutral runtime inputs

- **Status:** Candidate
- **Consumer:** Eon launch and every distribution channel.
- **Trigger:** A channel supplies exact component artifacts to Eon.
- **Result:** Eon receives explicit component paths, treats Nix store paths as
  opaque inputs, and invokes no Nix evaluator during normal runtime. The Nix
  full-Eon launcher supplies the pinned Anima executable as an opaque input.
  Nix builds Orbit, Venus, Eon and EonTerm from the in-tree crates. Generation
  identity covers every regular file in the Eon build's source tree outside dot
  entries and `target` directories; in Nix, that is the in-tree workspace
  source set.
- **Important failures:** Missing, non-executable, incompatible, or substituted
  artifacts fail before accepted launch.
- **Owner:** Eon supplies validated component facts and its source digest; the
  distribution supplies artifacts; Eon Runtime validates and consumes opaque
  launch inputs.
- **Consumes:** EON-C2's component graph.
- **Boundary:** Runtime code does not construct, persist, inspect, or derive
  identity from Nix store paths. The source digest reads workspace files, never
  store or profile paths.
- **Proof:** Candidate; mechanically verified at
  `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier on x86_64 Linux.
  - **Evidence:** `generation_identity_is_stable_bounded_and_namespaced` and
    `current_generation_consumes_supplied_assembly`; the `default` and
    `eonterm-closure` Nix checks build Orbit, Venus, Eon and EonTerm from the
    in-tree crates.

## EON-C4 — Thin orchestrator ownership

- **Status:** Proven
- **Consumer:** Eon, Orbit, Venus, Helix, Yazi, and Ratconfig integrations.
- **Trigger:** Eon composes or controls a product surface.
- **Result:** Eon owns product launch/topology policy, component selection,
  defaults, configuration policy, updates and distribution without duplicating
  terminal, rendering, editor, file-manager, or configuration state.
  Native desktop launches preserve host XDG preferences and use
  `EON_CONFIG_HOME` for the product configuration root.
  Eon selects Anima startup eligibility and forwards CLI arguments; Anima owns
  its styles, random choice, browsing, rendering, and terminal-mode cleanup.
- **Important failures:** A missing child contract returns to that child instead
  of being reconstructed in Eon.
- **Owner:** Eon assembly and product policy; Eon Runtime owns runtime
  mechanisms and state. Orbit, Venus and tools retain their subsystem authority.
- **Consumes:** Accepted child contracts through EON-C2's exact graph.
- **Boundary:** No copied child schema, hidden fork, compatibility adapter, or
  second terminal/rendering owner.
- **Assembly boundary:** `eon/crates/eon/src/product.rs` owns concrete product
  inputs; Eon invokes the in-tree runtime and codec crates. Eon Runtime in
  `runtime/` owns the nine runtime mechanisms, parsing, validation, mutable
  state and canonical EONW. Eon retains assembly, defaults,
  graph, distribution and process exit handling. The
  [runtime boundary](ARCHITECTURE.md#runtime-library-boundary) records the
  runtime seam and in-tree package selection.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** The workspace suites run with in-tree Orbit, Venus, Eon
    Runtime and EONW; `orbit-governance`'s
    `workspace_crates_keep_their_dependency_direction` enforces the crate
    ownership edges; the `eonterm-closure` check keeps Eon-only tools out of
    EonTerm. Installed checklist items 1, 4 and 11 exercise Eon policy over
    child-owned terminals, popups and tools.
- **Open proof:** Pending re-proof in this repository: portable-CPU Orbit under
  QEMU `qemu64`, Cachix publication and signed substitution from an empty
  store, host XDG handler preservation for desktop launches, memory
  measurements. Physical Ryzen, native macOS and performance remain
  unqualified.

## EON-C5 — Direct bundle lifecycle

- **Status:** Pending re-proof in this repository (proven before consolidation
  for the unpublished Linux slice)
- **Consumer:** A user installing Eon without adopting an unrelated toolchain.
- **Trigger:** Install, upgrade, inspect, roll back or remove a direct Eon bundle.
- **Result:** The operation affects only the selected Eon bundle and its owned
  integration points.
- **Important failures:** Partial installation, incompatible upgrade, or removal
  must not replace or damage unrelated tools or user configuration.
- **Owner:** Eon direct distribution.
- **Installer:** The static Rust `eon-install` consumes an exact HTTPS archive,
  verifies signed checksums against an independently trusted signer, and
  extracts into a private versioned user prefix. One atomic `current` selector
  chooses the default. Rollback selects an installed digest; removal deletes
  owned selectors. All payloads remain at their original paths to preserve
  live work; no automatic pruning or runtime-liveness owner is introduced.
- **Boundary:** `eon-p93` approves one Linux bundle and installer development
  sequence. Normal installation remains Nix-based; public release trust,
  qualification and publication retain separate gates and explicit approval.
- **Proof:** Pending re-proof in this repository.
  - **Environment:** Not repeated after consolidation.
  - **Evidence:** Before consolidation, `eon-prove-manual-nix-free-linux-bundle-216b`
    and `eon-prove-thin-linux-bundle-installer-8r7n` proved relocated
    extraction, signature and checksum rejection, interrupted extraction,
    upgrade, rollback and removal with live work preserved. See
    [manual bundle](MANUAL-BUNDLE.md) and [installer recovery](INSTALLER.md).
    At `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, sandboxed
    `cargo test --locked --offline --manifest-path installer/Cargo.toml`
    passes the installer's extraction, checksum, selection, removal and locking
    tests; bundle delivery is not repeated.
- **Open gates:** Publication, release trust, exact redistribution obligations
  and qualification of the Nix-free install remain separate.

## EON-C6 — Shared graph across distribution channels

- **Status:** Pending re-proof in this repository (proven before consolidation
  for the manual and installed Linux slices)
- **Consumer:** The Nix channel and any later approved distribution channel.
- **Trigger:** A channel composes an Eon product artifact.
- **Result:** Every channel consumes the same accepted EON-C2 component graph
  without changing runtime semantics.
- **Important failures:** A channel-specific compatibility graph or hidden
  component substitution is rejected.
- **Owner:** Eon distribution composition consuming EON-C2.
- **Boundary:** The manual and installed candidates remain unpublished;
  ordinary installation uses Nix. Nix-free delivery retains its own proof.
- **Proof:** Pending re-proof in this repository.
  - **Environment:** Not repeated after consolidation.
  - **Evidence:** Before consolidation, both EON-C5 archives carried the
    canonical manifest and lock bytes unchanged and reported the same exact
    diagnostics as the Nix product; see
    [manual proof limits](MANUAL-BUNDLE.md#retained-development-proof). The
    Nix channel's use of the one graph is EON-C2's automated evidence.

## EON-C7 — Native Linux Wayland and Apple Silicon macOS

- **Status:** Partially proved
- **Consumer:** A user launching a supported Eon distribution on an approved
  native platform.
- **Trigger:** Build, install, or launch an approved Eon channel.
- **Result:** Eon uses native Wayland on x86_64 Linux and the native macOS host
  on `aarch64-darwin`, without requiring a particular init or service manager.
  Both consume the same component graph, orchestration path, workspace
  contracts, and Nix-only channel while platform mechanics remain with
  their existing owners. Each channel names its proved architectures and launch
  environments.
- **Important failures:** Systemd-only lifecycle assumptions, X11 fallback, a
  second component graph, hidden child substitution, or an unproved
  architecture or compile-only runtime claim is rejected.
- **Owner:** Eon platform and distribution policy; child-owned kernel
  capabilities remain explicit child contracts.
- **Boundary:** Current accepted support remains x86_64 Linux native Wayland.
  Native Apple Silicon proofs for Orbit `ORB-C14` and Venus `VEN-C16` are
  pending re-proof in this repository. Apple Silicon macOS remains
  unsupported until Venus and full-Eon native proofs pass. X11, Xwayland,
  Intel macOS, signing, notarization, direct bundles, and public macOS
  distribution remain outside this contract.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`;
  the combined Linux/macOS contract is not proved.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Installed checklist items 1 through 5 and 10 through 12 cover
    native launch, picker, tabs and panes, popups, shortcuts and hints, detach
    and attach, generations, independent windows, EonTerm and exact-generation
    Stop on headless Sway; items 6 through 8 pass for the Eon Bar and version
    label, terminal text and opacity. Lucca opened the build from the desktop
    entry on COSMIC and reported a general visual pass.
- **Open proof:** Pending re-proof in this repository: installed quota chip
  detail, Accelerando playback, blur, pointer interaction (item 9), a full
  COSMIC checklist, install from this repository's first tag, clean-account public-source qualification,
  supervisor-loss adoption, and native Stop of a previous-EONW generation.
  EON-C8 records reader qualification as pending. Wider compositors,
  fractional/HiDPI coverage, proprietary NVIDIA and installed non-systemd
  behavior remain unproved. Machine-restart recovery is not supported.
  Native Apple Silicon proof follows Orbit ORB-C14, Venus VEN-C16, then
  `eon-prove-full-eon-apple-silicon-macos-t8o`; compile/evaluation is insufficient.

## EON-C8 — Durable tab and pane workspace

- **Status:** Partially proved
- **Consumer:** One local Eon workspace user.
- **Trigger:** Launch, create or close a tab, create a pane or column, traverse
  focus, reorder the active tab or selected pane, receive terminal exit, or recover
  accepted same-boot runs.
- **Result:**
  - A workspace contains horizontal tabs. Each established tab with panes
    holds an ordered row of one to four columns (pending and popup-only tabs
    hold none); each column is a vertical accordion of panes with exactly
    one expanded pane. Each pane views an independent persistent terminal.
    Every column's expanded pane is visible at once; exactly one of them is the
    tab's selected pane and receives keyboard input. A one-column tab behaves
    as a single accordion.
  - Tabs use stable `tN` identities. Each tab header shows its number plus the
    directory leaf or `/` derived from Eon's authoritative launch directory;
    actions retain `tN`, and accessibility includes full launch-path context. Venus
    renders pill-shaped, separated tabs sized to shaped labels, capped near 280
    logical pixels at default typography. Long names use middle ellipsis;
    narrow tabs retain their number whenever it fits. Hover previews the launch
    path, and wheel scrolling works across the whole strip, including gaps.
    Selected fill and brighter text identify the active tab without an underline;
    keyboard focus adds a separate rounded outline.
  - Each pane is identified as `pN`, two ASCII spaces, and a compact working-
    directory label; home uses `~`, descendants use `~/`, and
    external paths remain absolute.
  - On each new or reopened full-Eon surface, `[terminal] pane_frames` in
    `config.toml` defaults to `true` and requests one thin rounded workspace
    sheet: its sides and bottom sit on the window's edges, adjacent columns
    share one straight seam that meets the sheet's top edge and the footer (or
    the sheet's bottom edge when hints are hidden) in square T-junctions, and
    separators spanning each column divide its visible pane stack, with a small
    bottom gap (4 logical pixels at default typography) using terminal
    background color/opacity under the existing surface-wide blur request. Only
    the sheet's outer corners are rounded, about one pane header high, so the
    top curves meet the window's sides at the first pane header's bottom; the
    first tab pill's arc is centered over the top-left curve when the tab strip
    has room and the corner exceeds the pill by at least the tab gap. Selected
    and
    hovered headers follow the corners where they touch, with square edges at
    seams, and labels and footer hints keep clear of the curves. `false`
    removes the sheet border, seams and separators while retaining headers,
    selection, hover and keyboard focus; the footer and popup outlines keep the
    sheet's corner radius. Both modes
    preserve terminal-grid and picker geometry. EonTerm receives no
    workspace-frame override. Requested initial grids account for
    the small bottom margin. Live surfaces retain their settings;
    malformed or duplicate values reject a new launch or replacement without
    stopping existing terminals. Eon supplies the default; Eon Runtime owns the
    strict boolean and launch projection; Venus owns geometry, rendering,
    input and accessibility.
  - Left/right traversal treats the visible columns of all tabs as one ordered
    strip: it moves between the active tab's columns, continues from the first
    or last column to the previous or next tab, and enters that tab at its
    nearest edge column (rightmost when moving left, leftmost when moving
    right), wrapping at the strip's ends; with one tab it wraps between that
    tab's columns. While a popup or picker covers a tab, traversal moves
    between tabs directly. Tab focus by position or identity keeps the tab's
    last selected column. Up/down
    traversal moves within the selected column and wraps when it has at least
    two panes. Singleton traversal and selecting
    the already-selected live identity succeed with an unchanged snapshot.
    Left/right remains available while a directory picker is open: the picker
    stays bound to its original live tab while another active tab presents its
    selected pane. Tab navigation by identity also remains available. Users can
    create panes in another tab and create additional pending tabs, each with
    its own picker under EONW v7.
  - Creating a column adds a column immediately right of the selected column,
    holding one new pane in the tab's launch directory, and selects it; in a
    tab without columns it acts exactly as creating a pane. Venus maps
    Alt+Shift+M to it and, in a tab with one to three columns, refuses it with
    a notice when a column would get fewer than 40 terminal columns. Alt+M adds
    a pane to the selected column.
  - Moving left/right swaps the active tab with exactly one adjacent tab;
    moving up/down swaps the selected pane with exactly one adjacent pane in
    the selected column. The moved stable identity remains selected. Movement stops
    quietly at ordered edges and remains unavailable in the picker-bound active tab.
    Venus maps Ctrl+Alt+H/L and Ctrl+Alt+K/J directly to these actions without a
    mode.
  - Closing names the expected active `tN` and requires another live tab. A
    non-final pending tab first stops its exact picker, then disappears and
    restores its prior tab. Eon stops the tab's pane and popup terminals one at
    a time and prunes each
    confirmed end through the normal terminal-exit owner. Complete success
    removes the tab with the same deterministic focus rule as natural exit.
    If a later stop fails, no later terminal is stopped, the action reports
    failure, and every remaining live terminal stays represented in the
    partially pruned tab. A terminal that ended naturally during the request is
    reconciled and pruned as an exit rather than reported as a successful Stop.
    Venus maps Alt+Shift+W directly to this stable-target action; lowercase
    Ctrl+W remains terminal input.
  - Ended terminals leave no dead pane or empty tab; focus moves deterministically
    to the same identity when possible, otherwise the following sibling at the
    removed index, otherwise the preceding sibling; an empty workspace closes
    Venus and the supervisor. A column whose last pane ends disappears; if it
    held the selection, selection moves to the following column, otherwise the
    preceding one.
  - Same-boot recovered runs project numerically into one column of synthetic
    `t1` as `pN`
    without claiming restoration of prior topology or launch-directory policy.
  - New current-generation full Eon surfaces omit the redundant native title
    bar while retaining the selected terminal title for compositor semantics.
- **Important failures:** Unknown or stale identity, a close target other than
  the active tab, final-tab close, creating a fifth column, durable close in the picker-bound tab,
  invalid transition, unavailable endpoint, duplicate identity, or partial
  recovery leaves accepted topology unchanged. A repeated request ID never
  repeats a stop, and a replay naming a removed tab cannot close its successor.
  Picker-stop failure changes no topology; durable partial-stop behavior is the
  explicit result above. Exhausted pane numbers fail before terminal start;
  unknown or repeated terminal-exit notices remove nothing; losing a view never
  silently stops or substitutes a terminal. Rejected workspace actions retain
  their explanation across unchanged polls and announce it without moving
  terminal focus; a changed workspace retires the notice.
- **Owner:** Eon Runtime workspace topology, identity, focus, pruning and action
  enforcement; Eon supplies product defaults/policy. Orbit owns terminals and
  Venus owns native materialization.
- **Consumes:** Current EONW v10 workspace/popup values, Orbit terminal
  identities/endpoints and Venus `VEN-C8`.
- **Boundary:** No arbitrary split tree, vertical splits, column resizing,
  arbitrary reorder target, cross-column or cross-tab pane movement, picker relocation, durable layout
  restoration, a user-facing per-pane terminal stop/restart action, terminal
  content/history, provider state, plugin surface, remote access, or
  reconstructed terminal state.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Topology and pruning
    (`delayed_second_cli_receives_committed_workspace_and_controls_three_sessions`,
    `session_exit_prunes_the_workspace_and_the_last_exit_closes_eon`,
    `replacement_eon_adopts_exact_runs_and_projects_numeric_workspace`,
    `pending_tabs_keep_independent_pickers_and_close_only_the_selected_tab`);
    movement and close (`round_trips_v4_movement_without_widening_v3`,
    `round_trips_v4_tab_close_without_widening_v3`,
    `reorder_parser_accepts_only_each_workspace_axis`,
    `tab_close_parser_requires_one_stable_identity`); pane frames
    (`configured_pane_frames_reach_only_workspace_launches`,
    `pane_frame_option_is_a_strict_single_boolean`); Venus labels and
    accessibility order (`tab_directory_labels_use_current_position_and_bounded_path_context`,
    `pane_metadata_label_is_bounded_safe_and_has_honest_fallbacks`,
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`).
    Installed checklist items 3 and 4.
- **Open proof:** Columns (`eon-splits-opwv`): runtime topology and EONW v10
  are covered by `columns_hold_accordions_traverse_into_tabs_and_collapse_on_exit`,
  `create_column_in_a_popup_only_tab_never_overfills_a_neighbor`,
  `column_moves_stay_in_their_column_popups_skip_columns_and_the_cli_reports_positions`
  and the v10 codec tests; Venus layout, the Alt+Shift+M mapping and the 40-column
  threshold by `columns_sit_side_by_side_and_keep_the_selected_column_primary`,
  `direct_workspace_shortcuts_use_stable_eon_actions` and
  `each_column_keeps_forty_terminal_columns`; isolated installed dogfood on a
  private headless Sway (`eon-splits-opwv` slice 4) observed live columns,
  traversal into tabs, click selection, per-column resize, column removal,
  detach and reattach, popups over columns, and both refusal notices.
  Pending re-proof in this repository: bounded Linux reader
  qualification with unmodified upstream Orca
  `d13f83dcf230fe1e66f1ed6dbfbf057840f55a6a`, which includes
  [the #757 correction](https://gitlab.gnome.org/GNOME/orca/-/commit/f0189cbf9e97208b7aceada7602bf00d2172aad3);
  native selected-Orbit crash continuation and the rejected-action notice; pixel
  checks of pane frames. Installed AT-SPI labels and broader reader use remain open. These checks do not establish uninterrupted picker rendering,
  fractional-scale, wider compositor or non-systemd coverage.

## EON-C9 — Managed shell environment

- **Status:** Proven
- **Consumer:** Eon terminals launched under Nushell, Bash, Zsh, or Fish.
- **Trigger:** Eon constructs a child launch environment.
- **Result:**
  - Eon selects exact Nushell, Bash, Zsh, Fish, Starship, Zoxide, fzf, Atuin,
    Carapace, Helix, Yazi, and LazyGit artifacts.
  - Stable `eon-*` commands expose managed shells and tools outside Eon; one
    child-private PATH exposes accepted unprefixed names inside terminals.
  - Configuration accepts one direct argv command and independent Starship,
    Zoxide, Atuin, and Carapace booleans. Defaults are `command = ["eon-nu"]`
    and `true`; each new terminal rereads them without rewriting running shells.
  - Managed shells load native user configuration before bounded Eon activation.
    Disabled integrations suppress only Eon's hook; existing prompts,
    completers, same-tool hooks, normal Starship discovery, and Atuin policy
    remain child-owned.
  - Nushell native user autoload remains last and its stock startup banner is
    suppressed. Eon does not set `STARSHIP_CONFIG`, and Atuin continues to honor
    `ATUIN_NOBIND`.
  - Managed Nushell enables native `clip copy` and `clip paste`. Its interactive
    startup supplies `clc` and `clp` aliases for those commands; native user
    autoload may override the aliases afterward.
- **Important failures:** Invalid or unreadable configuration, empty command,
  missing artifact, manifest mismatch, or shell launch fails explicitly without
  a replacement terminal. Optional integration failure warns without preventing
  shell launch. Unavailable native clipboard access reports Nushell's own error.
  Eon never mutates global configuration, startup files, aliases, or PATH.
- **Owner:** Eon supplies defaults, component selection and Nix shell adapters;
  Eon Runtime owns strict configuration and private launch-environment
  enforcement. Shells/tools retain native configuration and behavior.
- **Boundary:** No global aliases, user-file mutation, prompt/completion schema,
  shell framework, automatic Direnv/Mise, history/sync policy, plugin surface,
  editor replacement, or remote/headless clipboard bridge. Arbitrary argv
  remains unmanaged and `eon run -- COMMAND...` remains the explicit escape
  hatch.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier on x86_64 Linux.
  - **Evidence:** The `shell-environment` and `managed-command-path` Nix checks
    (version output from all four managed shells, Zsh and Fish Carapace
    completion, `eon-*` commands, native `clip copy`/`clip paste` and the
    `clc`/`clp` aliases);
    `managed_commands_use_exact_programs_and_private_configuration`,
    `session_path_contains_one_managed_prefix`,
    `shell_configuration_is_strict_and_defaults_without_a_file` and
    `managed_invocation_names_are_bounded`.
- **Open proof:** Pending re-proof in this repository: interactive managed
  startup of each shell, and a native Wayland clipboard round trip through
  `clip copy`, the aliases and `wl-paste`.

## EON-C10 — Typed workspace control

- **Status:** Proven
- **Consumer:** One local CLI or approved composition controlling a live Eon
  supervisor.
- **Trigger:** The client submits one versioned workspace or supervisor-lifecycle
  action.
- **Result:**
  - Eon returns one complete typed result from the sole live workspace owner;
    human-readable CLI output and deterministic structured output project the
    same result.
  - Workspace results carry ordered tab, pane, and terminal identities, active
    and selected identities, liveness, exact raw tab launch-directory bytes,
    and exact opaque Orbit endpoint bytes. Lifecycle results carry generation,
    component/protocol identity, live terminals, and owner-authored attach/stop
    availability.
  - CLI JSON emits opaque directories and endpoints as ordered byte arrays
    without changing EONW. Each connection carries one length-delimited request
    and response and does not use EOF as framing.
  - The shared 2 MiB frame ceiling admits every snapshot valid at EONW's field
    and workspace-count bounds.
  - Harmless navigation no-ops return the unchanged Snapshot as success;
    request deduplication and picker/lifecycle restrictions still apply.
- **Important failures:** Malformed, oversized, incompatible, unavailable,
  rejected, timed-out, partially read, or response-lost operations fail within
  the shared bound without reconstructing hidden state or reviving completed
  Stop. Accepted Stop names affected live terminals, sends canonical Stop through
  every validated management lease before waiting, and returns only after every
  exact terminal record is complete.
- **Owner:** Eon Runtime supervisor workspace/action state and CLI projection;
  its canonical EONW package owns types, codec and result ordering. Eon supplies
  validated component facts and product defaults.
- **Consumes:** Current EONW v7 and accepted Orbit management operations; the
  v5 popup shape and historical lifecycle codecs are covered by the codec tests
  cited in Proof.
- **Boundary:** Additive lifecycle tags do not widen the pinned Venus workspace
  consumer. There is no event stream, subscription policy, remote transport,
  plugin/MCP API, authorization framework, durable restoration, terminal
  content, or direct child-protocol escape hatch.
- **Package owner:** The canonical EONW crate lives in
  `runtime/crates/eon-workspace-protocol` with its accepted package/API and
  v2–v10 bytes. Venus, Eon Runtime and Eon consume it by path.
  See [package selection](ARCHITECTURE.md#in-tree-package-selection).
- **Runtime policy:** Project is required on Alt+Z and uses Eon's existing
  transient chooser command. Git defaults to `eon-lazygit` on Alt+Shift+J;
  Agent defaults to the first available provider in the accepted Nova order on
  Alt+Shift+L. Git and Agent remain live when hidden. Reopening at the same tab
  directory preserves identity; explicit retargeting changes no process until
  the next invocation, which stops the exact old terminal before starting a
  fresh one at the new directory. A stop failure preserves the old instance; a
  start failure records no replacement.
  Anima defaults to Alt+Shift+A and launches the pinned random native animation
  as a transient popup. A dismissal, replacement, or tab close stops its Orbit
  terminal, restores the prior tab focus, and leaves no hidden animation running.
- **Configuration:** `[popup]` owns finite side and vertical margins, defaulting
  to 8 and 4 logical pixels. `[popups.<id>]` owns direct argv, physical shortcut,
  label, enabled state and keep-alive lifetime. Only Agent accepts `"auto"`.
  Eon rejects unknown fields, malformed or colliding shortcuts, unbounded
  values and unavailable executables before mutating workspace state. It does
  not install providers, cache selection, interpret shell strings, or expose
  commands over EONW. Bare executables resolve through the terminal PATH;
  relative executable paths resolve against the owning tab's launch directory.
- **Target safety:** Invocation names the tab, entry and expected instance
  (absence means no instance exists). Focus-only and toggle intents are
  distinct. Dismissal and chooser completion require the exact live instance;
  a replaced instance cannot inherit an old request. Instance identities must
  not be reused after supervisor recovery. The runtime owner validates targets
  against its current Snapshot before applying lifecycle policy; decoding checks
  syntax, not staleness. Ordinary `SetTabDirectory`
  remains explicit retargeting, not chooser completion.
- **Bounds/failures:** Preserve the 2 MiB envelope, 64 tabs, raw Unix directory
  and endpoint bytes. At most 32 enabled entries and 256 combined pane/popup
  terminals are represented. Reject malformed framing, invalid or aliased
  identities/endpoints, broken selections, duplicate entry/chord assignments,
  invalid geometry and stale popup targets. No terminal contents cross EONW.
- **Popup snapshot:** The catalog carries enabled entries, normalized physical
  shortcuts and logical-pixel margins; per-tab popup terminals/selection and
  pending-tab state are explicit. Popup-only tabs are valid. Commands, provider
  selection, cwd and lifetime remain runtime policy and never cross EONW. The
  retired v4 picker opcode is unavailable; Project uses ordinary popup invocation.
  Older fixed-namespace work may be inspectable without being presentable. There
  is no negotiation or adapter.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** EONW codecs (`request_actions_have_stable_v2_wire_bytes`,
    `round_trips_v2_bounds_and_rejects_invalid_inputs`,
    `common_actions_stay_exact_and_new_requests_are_bounded`,
    `popup_targets_round_trip_and_do_not_follow_replacements`,
    `snapshots_preserve_hidden_and_inactive_work_and_reject_broken_ownership`,
    `every_legal_snapshot_fits_the_frame_budget`); popup policy
    (`popup_configuration_owns_defaults_overrides_and_collisions`,
    `popup_executable_preflight_uses_the_session_context`,
    `popup_identity_reuses_hides_and_replaces_only_after_retarget`,
    `chooser_result_requires_its_exact_live_invocation`); deadlines and
    failures (`unix_connection_attempt_obeys_timeout`,
    `management_response_obeys_one_deadline_across_partial_reads`,
    `lost_stop_response_still_finishes_the_supervisor`,
    `missing_supervisor_is_a_structured_workspace_failure`,
    `invalid_focus_is_rejected_before_supervisor_connection`,
    `closed_stdout_pipe_is_not_a_panic`). Installed checklist items 3 and 4
    (`eon workspace` against the window; Project, Git, Agent and Anima popups).

## EON-C11 — Runtime generations and presentation

- **Status:** Proven on x86_64 Linux native Wayland
- **Consumer:** Eon and EonTerm users launching or targeting a product generation.
- **Trigger:** Launch current generation, inspect an older live generation,
  present a compatible existing surface, stop one generation, or stop a selected
  set of generations.
- **Result:**
  - Eon and EonTerm use separate runtime namespaces and default to their exact
    current generation.
  - Generation identity combines immutable runtime/EONW source and build
    inputs with Eon assembly/default/validator/graph/Cargo/Nix-generated inputs;
    each generation owns one private directory. Store/profile paths, mutable
    configuration, PIDs and live state do not define identity.
  - Older live generations remain discoverable, explicitly presentable when
    compatible, and explicitly stoppable through their own supervisor. Generation
    inspection and Stop use the supervisor's known EONW v2 through v7 lifecycle
    codec; presentation still requires the exact current EONW and component graph.
  - `eon stop previous` attempts every non-dead generation except the exact
    current one, whether current is live or not. Fixed-namespace legacy work
    reports its own unavailable reason. `eon stop all` includes current.
    Each selected generation uses the existing validated
    Stop; human mode confirms each target, while `--json` returns a result array
    without confirmation. Declining one target reports that no Stop was sent for
    that generation; earlier outcomes remain and later targets continue.
    Failed or unavailable targets do not suppress later targets while the CLI
    remains running.
    A Stop of the caller's own terminal can terminate the CLI before it receives
    that result or attempts later targets; run from outside selected generations
    when a complete result is required. When the CLI completes, it exits
    nonzero if any Stop attempt fails. `all` stops current last so earlier
    targets can finish when invoked inside a current terminal.
    Neither command forcibly kills a process or treats fixed-namespace legacy
    work as stoppable.
  - Implicit attach selects only a live supervisor reporting the exact current
    identity; otherwise Eon starts current without stopping older work. Explicit
    attach never substitutes another generation, and discovery distinguishes
    current, previous, legacy, dead, incompatible, and corrupt candidates.
  - One per-generation lifecycle lock covers listener and runtime teardown so a
    replacement cannot overlap cleanup; concurrent launch converges on one
    supervisor and terminal set.
  - Present requests native presentation without replacing the live Venus
    process, Orbit attachment, or terminal.
  - Dead exact residue is removed only after the recorded process is absent or
    dead; a later launch may then create one fresh run.
  - A current launch overlapping clean exit of the final terminal waits for the
    retiring supervisor, then starts one fresh terminal instead of reporting
    presentation success against ended state.
- **Important failures:** Unknown EONW version, incompatible presentation,
  stale or mismatched process identity, lost control, partial Stop, or live
  foreign residue fails closed
  without PID/process-name fallback or duplicate Orbit launch. Orbit startup
  and Ready negotiation remain bounded by the shared five-second deadline.
  Eon requires no cgroup membership or delegation; Orbit owns bounded PTY
  shutdown through its accepted ORB-C12/ORB-C13 management boundary.
- **Owner:** Eon Runtime generation identity/selection, namespaces, discovery,
  Present, explicit Stop and exact owned residue; Eon supplies immutable
  assembly/default/validator/graph/Cargo/Nix-generated inputs.
- **Consumes:** Orbit management v1 and Venus `VEN-C14`.
- **Boundary:** No manager daemon, persisted topology, machine-restart recovery,
  PTY handoff, old-generation backport, updater, Nix evaluation, package-channel
  identity, presentation compatibility window, remote runtime, plugin API, service-manager
  requirement, or automatic eviction.
- **Assembly boundary:** The `eon-runtime` generation owner consumes immutable
  runtime/EONW bytes and Eon's source-tree digest, which includes
  generated-command inputs. Exact presentation checks remain;
  retain the prior executable to reattach incompatible old work. See
  [generation inputs](ARCHITECTURE.md#defaults-overrides-and-generation) and
  [cutover acceptance](ARCHITECTURE.md#acceptance-boundaries).
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `generation_identity_is_stable_bounded_and_namespaced`,
    `discovery_bounds_dead_cleanup_and_rejects_unsafe_metadata`,
    `concurrent_launches_converge_and_generation_stop_is_owner_routed`,
    `bare_eon_attaches_only_to_the_live_current_generation`,
    `launch_overlapping_last_session_exit_starts_a_fresh_session`,
    `old_supervisor_remains_inspectable_but_not_presentable`,
    `previous_supervisors_keep_inspection_and_stop_without_presentation`,
    `current_cli_stops_known_previous_eonw_generations_through_their_supervisors`,
    `previous_and_all_stop_older_generations_when_current_is_unstarted`,
    `batch_stop_preserves_current_until_all_and_reports_unavailable_generations`,
    `legacy_workspace_is_visible_but_not_attachable_or_stoppable` and
    `stop_confirmation_refuses_a_replacement_supervisor`. Installed checklist
    items 2 and 12: detach and attach of one live generation, and Stop of that
    one exact generation.
- **Open proof:** Pending re-proof in this repository: native Present of a live
  surface, launch with `/sys/fs/cgroup` hidden, and native Stop of a real
  previous-EONW supervisor. Fixture-only historical codecs remain
  fixture-verified.

## EON-C12 — Standalone exact-command terminal

- **Status:** Proven
- **Consumer:** A local user or approved composition needing one native terminal
  surface without Eon workspace actions.
- **Trigger:** EonTerm launches one exact command after `--`.
- **Result:**
  - `eonterm -- COMMAND...` launches one exact nonempty argv in one Orbit terminal
    and one Venus surface without workspace topology. Optional
    `--no-decorations` is fixed for the supervisor lifetime.
  - `eonterm generations`, `attach`, and `stop` expose EON-C11 within EonTerm's
    separate namespace.
  - Repeated launch presents an attached surface; after detachment it opens one
    replacement against the same live terminal with the original decoration
    choice. Later invocations do not mutate that choice.
  - Workspace actions return `workspace-unavailable` and create no hidden
    topology.
- **Important failures:** Missing command, invalid identity, incompatible
  component, startup failure, supervisor loss, or explicit Stop follows the same
  bounded ownership and cleanup rules without creating workspace state.
- **Owner:** Eon defines and packages the standalone product; Eon Runtime
  implements exact-command launch/generation mechanisms. Orbit owns the terminal and
  Venus owns the surface.
- **Consumes:** EON-C1 through EON-C4, EON-C11, Orbit, and Venus.
- **Boundary:** EonTerm consumes no EON-C9 managed environment and adds no tabs,
  panes, key remapping, preset, wrapper, persisted mode marker, workspace,
  managed-tool/default-shell command, remote access, focus guarantee, or Zellij
  policy. Bare `eon` and `eon run` retain workspace meanings.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `eonterm_reopens_without_workspace_or_a_second_session`,
    `eonterm_replaces_exact_residue_after_its_supervisor_and_session_die` and
    `rejected_native_startup_creates_no_session_in_either_product`; the
    `eonterm-closure` Nix check. Installed checklist item 11: launch, detach,
    `eonterm attach` and exact Stop; `--no-decorations` started.
- **Open proof:** Pending re-proof in this repository: native supervisor-loss
  recovery of the same EonTerm terminal, and `--no-decorations` rendering
  under a decorating compositor.

## EON-C13 — Terminal-background opacity policy

- **Status:** Proven
- **Consumer:** Every Eon or EonTerm Venus surface.
- **Trigger:** Eon creates or reopens a surface while
  `$EON_CONFIG_HOME/config.toml` contains optional
  `terminal.background_opacity`.
- **Result:** Eon accepts one finite `f32` in `0.0..=1.0`, defaults absence to
  `0.80`, and passes the exact value as `--background-opacity` before Venus
  endpoints. Eon and EonTerm share it. A live Venus is unchanged; replacement
  reads one current configuration snapshot while Orbit and its PTY remain live.
- **Important failures:** Invalid initial configuration names the field and
  starts no supervisor, Orbit, or Venus. Invalid replacement configuration
  returns bounded presentation failure without stopping Orbit or its child.
- **Owner:** Eon supplies the default; Eon Runtime owns strict configuration
  and launch projection; Venus owns native rendering.
- **Consumes:** Venus `VEN-C11`.
- **Boundary:** Opacity affects only terminal default background and padding, not
  explicit cells, text, cursor, selection, chrome, decorations, input, hit
  testing, accessibility, or click-through. No profile, public override,
  watcher, live reload, Orbit restart, non-Wayland claim, or renderer fallback.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `bare_eon_attaches_only_to_the_live_current_generation` checks
    the default `--background-opacity 0.8`;
    `eonterm_reopens_without_workspace_or_a_second_session` rereads an explicit
    value on reopen;
    `invalid_presentation_configuration_precedes_supervisor_generation_and_children`
    rejects invalid values; Venus `background_opacity_selects_only_proved_surface_modes`,
    `background_opacity_premultiplies_clear_colors` and
    `background_opacity_skips_only_default_cell_backgrounds`. Installed checklist
    item 8 at the `0.80` default.

## EON-C14 — Default compositor blur policy

- **Status:** Proven by the automated tier; the installed backdrop is pending
  re-proof in this repository.
- **Consumer:** Every Eon or EonTerm Venus surface.
- **Trigger:** Eon creates or reopens a surface while
  `$EON_CONFIG_HOME/config.toml` contains optional strict boolean
  `terminal.background_blur`.
- **Result:** Absence defaults to `true`; true passes exactly one
  `--background-blur` before Venus endpoints and false omits it. Eon and EonTerm
  share the setting. A live Venus is unchanged; replacement uses one current
  configuration snapshot while Orbit and its PTY remain live. Blur remains
  independent from opacity.
- **Important failures:** Invalid initial configuration names the field and
  starts no supervisor, Orbit, or Venus. Invalid replacement configuration is
  bounded without stopping Orbit. Unsupported compositor policy remains
  Venus/compositor best effort and does not make Eon launch fail.
- **Owner:** Eon supplies the default; Eon Runtime owns strict configuration
  and launch projection; Venus and the compositor own native behavior.
- **Consumes:** Venus `VEN-C15` and EON-C13.
- **Boundary:** No capability probe, automatic opacity, blur strength, public
  override, watcher, live reload, fallback renderer, Eonova-specific policy, or
  non-Wayland visual claim.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier.
  - **Evidence:** `bare_eon_attaches_only_to_the_live_current_generation` checks
    one default `--background-blur`; `eonterm_reopens_without_workspace_or_a_second_session`
    omits blur under `background_blur = false`, then rereads the configuration
    on reopen and restores the default blur once the setting is removed;
    `invalid_presentation_configuration_precedes_supervisor_generation_and_children`
    rejects invalid values.
- **Open proof:** Pending re-proof in this repository: the installed frosted
  backdrop on a blur-capable compositor such as COSMIC; headless Sway has no
  blur protocol.

## EON-C15 — Same-boot Orbit recovery

- **Status:** Proven on x86_64 Linux by the automated tier; installed
  supervisor-loss adoption is pending re-proof in this repository.
- **Consumer:** One local Eon or EonTerm user on the same boot and login.
- **Trigger:** An Eon-launched Orbit crosses Ready, its supervisor disappears,
  its acquired owner dies, a replacement targets the same exact generation,
  or the user requests Stop.
- **Result:**
  - Eon assigns canonical `session-N`, one fresh opaque run ID, and the runtime
    generation ID as component generation, then validates complete live identity
    and acquires the sole management lease within one shared five-second
    operation.
  - Replacement EonTerm recovers exactly one live run; replacement full Eon
    acquires at most 256 valid runs, sorts positive canonical N numerically, and
    projects them into synthetic `t1` as `pN` with the lowest selected. That tab
    receives the replacement supervisor's launch directory as fresh policy.
  - New creation continues at checked `max(N)+1`; prior grouping, focus, argv,
    launch directories, and request history are not restored. After recovered
    `t1`, later creation starts at `t2`.
  - Before spawn, Eon creates and retains the exact empty owned mode-0600
    management record inode. Orbit locks it, marks publication and atomically
    replaces it with Ready. Local Child rollback requires winning the lock on
    that unchanged still-empty inode; marking revokes signal authority even
    before publication completes. After publication, cleanup is management-only.
  - Explicit generation Stop uses canonical management for every acquired run
    and discovers untracked runs through the same bounded generation record
    and lease validation used for recovery, including late-Ready pickers.
    It succeeds only after matching tombstones and cleanup.
  - Eon removes exact owned dead residue only after the recorded process is
    absent or dead.
  - When an acquired Orbit dies without publishing a
    tombstone, Runtime retires only its exact validated Live record and dead
    endpoints and reaps its ended child. Full Eon applies normal ended-pane
    pruning; healthy terminals and their native workspace remain usable.
    Owner loss is a local failure, not a canonical terminal outcome or
    successful Stop.
- **Important failures:** Unsafe, missing, replaced, malformed, oversized,
  incompatible, duplicate, zero, noncanonical, overflowing, wrong-generation,
  wrong-UID, wrong-process, Busy, partial acquisition, unreconciled Orbit exit,
  deadline, or transport failures publish no partial workspace, launch no
  duplicate run, and authorize no PID, signal, pathname, process-name, group, cgroup, or pidfd
  fallback. A lost response after completed Stop cannot revive the supervisor.
  A marked Live record rejected after spawn is stopped through canonical
  management using its published identity and is never published as a terminal.
  Failed Stop retains its supervisor and unfinished leases for explicit retry;
  completed runs leave live inventory. Workspace mutations and presentation
  remain unavailable until cleanup succeeds. Failed natural-exit reconciliation
  still collects other results and restores retryable leases. Discovery reaps
  ended untracked direct children before retiring their canonical records.
  During failed cleanup, it still reaps an exited presentation child without
  changing the workspace or launching replacement terminals.
  Stop's result read timeout allows one preceding five-second operation plus
  five-second cleanup and a one-second margin; excessive queues can still fail.
- **Owner:** Eon Runtime generation/recovery enforcement, bounded enumeration,
  projection, lease collection, deadline, EONW ordering, retained launch claim,
  and exact residue; Orbit `ORB-C13` owns live identity, Ready, lease, terminal state,
  `ORB-C12` cleanup, tombstone, and endpoint cleanup.
- **Consumes:** Orbit `ORB-C12`, `ORB-C13`, management v1, and Venus `VEN-C14`.
- **Boundary:** Same boot and login only; no prior-topology persistence, Orbit or
  machine restart recovery, logout/reboot survival, remote authority, service
  manager, broker, or compatibility adapter. Tombstones grant no authority and
  must be reconciled before their workspace slot is reused.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`;
  upgrade and rollback: installed builds
  `/nix/store/gg71kiz3k854f4xihasr1pwligypznl3-eon-0.1.0` of
  `795d735a8f61ef439c56d68e1cc4d3929af0dfcf` and
  `/nix/store/vscla3lxvsn3mm5hv51lpcqjn1w97fvp-eon-0.1.0` of
  `a3cc5e0d687c73bb482dd6f7887a49c9c9c23c0a`, `eon-upgrade-continuity-kgba`.
  - **Environment:** Automated tier; installed tier. Upgrade and rollback: a
    throwaway Nix profile on private headless Sway 1.12 (wlroots, pixman) in
    bubblewrap, x86_64 Linux.
  - **Evidence:** Identity and Ready
    (`session_identity_is_positive_canonical_decimal`,
    `marked_ready_claim_never_falls_back_to_child_stop`,
    `rejected_ready_identity_stops_the_spawned_session_through_management`,
    `desktop_launch_failure_stops_the_ready_session_through_management`);
    replacement and recovery
    (`replacement_eon_adopts_exact_runs_and_projects_numeric_workspace`,
    `replacement_reconciles_exact_tombstone_endpoints_without_deleting_replacements`,
    `replacement_with_only_a_stale_initial_picker_falls_back_without_reopening_it`,
    `eonterm_replaces_exact_residue_after_its_supervisor_and_session_die`);
    Stop and cleanup
    (`stop_accounts_for_late_ready_picker_and_retains_partial_cleanup_owner`,
    `stop_result_covers_queued_operation_and_managed_cleanup`,
    `lost_stop_response_still_finishes_the_supervisor`,
    `failed_directory_picker_stop_is_retried_during_supervisor_cleanup`,
    `failed_natural_cleanup_still_finalizes_other_runs_and_restores_leases`);
    owner loss (`lost_owner_cleanup_requires_death_and_original_owned_objects`,
    `discovery_reaps_untracked_ended_children_before_removing_records`).
    Installed checklist items 2 and 12. Installed upgrade between two
    generations sharing EONW v9 and identical Orbit and Venus builds, then
    rollback: the older generation's terminals kept running and stayed listed
    as previous; the newer generation launched alongside them; after rollback,
    bare `eon attach` presented the original terminals; `eon stop all` stopped
    both generations.
- **Open proof:** Pending re-proof in this repository: installed native
  selected-pane crash isolation with healthy peers, supervisor-loss adoption of
  several full-Eon terminals, and upgrade and rollback that change the Orbit or
  Venus build or the EONW version. No logout, Orbit-restart or machine-restart
  recovery is proved.

## EON-C16 — Caller-owned application identity

- **Status:** Proven
- **Consumer:** Full Eon, standalone EonTerm, or one approved EonTerm composition.
- **Trigger:** The caller launches a new Venus surface with its selected validated
  desktop application ID.
- **Result:** Full Eon supplies `eon`, standalone EonTerm supplies `eonterm`, and
  an approved composition passes its value unchanged to `VEN-C17` before window
  creation; identity never enters PTY argv/environment and remains independent
  from terminal-authored titles.
- **Important failures:** Empty, non-UTF-8, oversized, or non-token identities
  fail before Orbit or Venus starts; profile refresh does not mutate a live
  surface's selected identity.
- **Owner:** Eon supplies defaults; Eon Runtime validates and projects EonTerm
  CLI input; the embedding composition owns its identifier/desktop metadata;
  Venus owns native mapping only.
- **Consumes:** Venus `VEN-C17`.
- **Boundary:** Immutable launch metadata only; no runtime protocol, branding
  provider, desktop discovery, child inspection, or wider platform promise.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `venus_receives_workspace_endpoint_only_in_workspace_mode`
    checks the projected `--application-id` for both products. Installed
    checklist items 1 and 11 launch Eon and EonTerm; item 1's launch waited for
    the Sway tree to show the native app ID `eon`.
- **Open proof:** Pending re-proof in this repository: installed rejection of
  invalid EonTerm identities, distinct launcher grouping, and an approved
  embedding's caller-owned identity.

## EON-C17 — Tab launch directory

- **Status:** Partially proved
- **Consumer:** One local Eon workspace user creating tabs and panes.
- **Trigger:** Eon creates or recovers a workspace, creates a tab or pane, or an
  approved same-user client explicitly retargets one live tab.
- **Result:**
  - Every live tab has one stable `tN` identity and one authoritative absolute
    launch directory. Panes retain `pN` identities.
  - Fresh `t1` validates the supervisor's absolute launch directory before any
    terminal recovery or startup. Later new tabs start at the current user's
    home directory, resolved by the native home-directory API and validated before creating the tab. EON-C18's
    pending picker commits the selected first-pane directory or starts a pane
    at the tab's starting directory when dismissed.
  - `SetTabDirectory` changes exactly one addressed tab. Later panes in that tab
    start with the accepted directory; existing terminals and terminal CWDs do
    not change.
  - Current EONW v7 returns the exact accepted raw path bytes. Venus derives a
    bounded `N  leaf` or `N  /` label while retaining `tN` for actions and
    exposing identity plus bounded path context to accessibility.
  - Same-boot recovery assigns synthetic `t1` the replacement supervisor's
    launch directory as fresh policy and never infers prior policy from a pane.
- **Important failures:** Empty, relative, NUL-containing, oversized, missing,
  non-directory, unknown-tab, or stale-tab updates change nothing. Once a path
  passes validation, later disappearance leaves its policy value stored; a
  later terminal startup fails without committing pane topology or leaving
  launch residue.
- **Owner:** Eon Runtime owns tab identity, launch-directory state, validation,
  mutation, new-tab home resolution and recovery fallback. Orbit owns each
  terminal's CWD; Venus owns native projection of the authoritative workspace state.
- **Consumes:** Current EONW v7, Orbit terminal startup, and Venus `VEN-C8`.
- **Boundary:** No manual names, path canonicalization, retained directory
  handles, filesystem watching, shell-`cd` tracking, pane-CWD inference,
  retargeting of existing processes, persistence, picker UI, or compatibility
  service accepting EONW v1 and v2.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `launch_directory_can_disappear_after_the_initial_session_starts`,
    `tab_directory_retargets_future_sessions_without_crossing_tabs`,
    `new_tabs_start_at_home_and_reject_unavailable_home_without_mutation`,
    `tab_directory_parser_preserves_one_opaque_path_argument`,
    `replacement_eon_adopts_exact_runs_and_projects_numeric_workspace` and
    `round_trips_v2_bounds_and_rejects_invalid_inputs`; Venus
    `tab_directory_labels_use_current_position_and_bounded_path_context`.
    Installed checklist items 3 and 4.
- **Open proof:** AT-SPI launch-path context on the current build remains
  unproved; prior focused path-context checks are not a screen-reader workflow.

## EON-C18 — Picker-first tab directories

- **Status:** Proven for home-directory tabs, picker dismissal and shared popups
  on x86_64 Linux native Wayland; v7 multi-pending tabs are proven by the
  automated tier.
- **Consumer:** One person starting or using full Eon.
- **Trigger:** Eon needs the first pane for a new tab, or the person presses
  Alt+Z in an existing tab.
- **Result:**
  - On a fresh full-Eon workspace, after Venus accepts the initial presentation,
    Eon may play one short Anima animation in the initial transient Project
    terminal before starting the normal directory picker. The default is enabled,
    a random native style, and three seconds; `[anima].enabled = false` skips it.
    Any key other than Anima's style-browsing keys dismisses playback without
    becoming picker input. This step does not run for later tabs, attachment,
    reconnection, same-boot recovery, or EonTerm.
  - A fresh workspace and every later new tab begin as one active pending tab
    with no durable pane. Eon starts one transient Orbit terminal running the
    exact packaged ranked-directory picker at that tab's starting directory
    (the initial launch directory for t1, the user's home for later tabs).
    Ambient fzf default options cannot alter its command, layout, or bindings.
    Each picker instance has a distinct endpoint within that workspace, including
    repeated retargeting of the same tab. A polling client can distinguish the
    replacement even if it misses the intervening no-picker snapshot.
  - Quick search ranks Zoxide history through fzf. Enter accepts one result;
    Tab opens packaged Yazi without cancelling the picker; Esc and Ctrl+C cancel.
    Every quick-search exit stops and reaps a still-running history query;
    accepting, browsing and cancelling do not wait for the remaining results.
    Empty history still shows the Browse action. Arrows move through results.
  - Yazi begins at the tab's starting directory. Arrows/hjkl browse parent and
    child directories; Shift+Z searches history and only moves the browser.
    Enter uses the highlighted folder; Tab returns to quick search and preserves
    the browser directory for the next visit. Esc, q, Q, and Ctrl+C cancel
    from the main folder list. Within input and help, Esc follows Yazi's native
    mode/filter handling before closing. Startup, new-tab and Alt+Z pickers
    share these keys. Native browsing, find, sorting and previews remain.
    `g h` reaches home, `g /` root, `g Space` accepts a typed path, and `.` toggles
    hidden entries. `a` uses Yazi's native `create`: a plain name creates a file,
    and a name ending in `/` creates a directory. Creation highlights the item
    without accepting the picker; Enter accepts only directories. Native input
    cancellation and existing-target confirmation remain. Cancelling the picker
    leaves successful creations on disk; pinned Yazi returns quietly on failure.
    The fixed chooser-mode open returns the highlighted path without invoking
    a file opener. Creation is the only enabled file-management mutation.
    External file opening, interactive shell commands and suspension remain excluded.
    Files cannot be accepted as a tab directory or opened by the chooser.
    Browsing does not add Zoxide history.
    Yazi owns directory enumeration, navigation and creation; Eon performs no
    recursive filesystem scan. A vanished or inaccessible final folder fails
    EON-C17 validation without mutation.
  - Each selection screen shows its own persistent footer: quick search offers
    Use directory, Browse folders, and Cancel; Yazi offers Use highlighted folder,
    Create, Quick search, Shift+Z Jump, Cancel, and Help; nested jump search
    offers Jump and Back to folders. Browser shortcuts are absent from quick search. Eon's fixed Yazi
    status configuration replaces file metadata with these picker actions.
    Browser hints appear only while the folder list owns input; native Yazi
    prompts and overlays hide them until the folder list regains focus.
  - Accepting a valid choice atomically commits the tab directory and starts
    exactly one first `pN` terminal there. Dismissing a pending picker keeps
    the tab and starts its first pane at that tab's validated starting directory.
    Explicitly closing a non-final pending tab stops its picker and removes
    that tab without starting a pane.
  - Alt+Z keeps the existing explicit retarget behavior after a tab has panes;
    it never changes or restarts a running pane.
  - Current EONW v7 exposes Project as a popup endpoint bound to one live
    tab, never as a normal `pN` pane. The picker tab may remain live while
    another durable tab is active. A picker-owned pending tab remains pane-free,
    with an absent selected pane instead of a sentinel or placeholder process.
    Lifecycle inspection and Stop may report zero durable terminals during the
    initial picker; the transient picker remains excluded from that list.
  - Venus can start a full-Eon presentation from the workspace endpoint alone
    while the initial tab is pending; the picker endpoint in its first snapshot
    is the sole terminal attachment.
  - Venus keeps the tab bar visible and replaces the pane stack with the picker,
    using Eon's shared configured popup margins. The
    previous terminal is not composited underneath. Attach and recovery never
    create an automatic picker; recovering only a stale initial picker stops it
    and applies the validated launch-directory fallback.
  - Left/right focus actions continue to traverse and wrap live tabs
    while the picker stays bound to its original tab. Another active tab shows
    its selected pane; returning shows the same picker for normal acceptance or
    cancellation. Tab navigation by identity remains available. Other tabs
    allow pane creation, pane focus by direction or identity, pane/tab movement,
    explicit directory updates, and stable-target close. These actions preserve
    the picker's binding and terminal. Mutations of the picker-bound tab remain
    blocked except directory acceptance and closing its non-final pending tab;
    that close stops the transient terminal before removing the tab. Dismissing
    a background pending picker starts that tab's first pane while leaving
    the selected tab in focus. Explicit tab close selects the nearest
    surviving tab. A workspace with no running terminals ends.
    Pickers remain attached across tab switches; established tabs may open
    their own popup while another picker remains live elsewhere. EONW v7 permits
    multiple pending tabs, each retaining its exact Project picker across switches;
    CreateTab always creates a distinct tab. Alt+Shift+W closes the selected
    non-final pending tab and stops only its picker.
- **Important failures:** Unresolvable or invalid home, or a failed new-tab
  picker launch, creates no tab. Invalid selection never commits its path;
  duplicate or stale results cannot mutate another tab or its terminals.
  Dismissal, picker exit or presentation detachment cleans the transient picker
  and attempts the first pane at the pending tab's starting directory.
  If that pane cannot start, report the error and remove only the unstartable
  tab; other tabs and terminals remain.
  Left/right with only one live tab succeeds unchanged; final-tab close remains
  unavailable. A duplicate close cannot stop the picker twice or remove another
  tab.
  Invalid Anima settings fail before a fresh workspace starts. Missing, failed,
  or timed-out playback reports a short diagnostic and continues to the picker.
- **Owner:** Eon supplies picker/default policy and packaged commands; Eon
  Runtime owns tab binding, command selection, lifecycle, validation, mutation
  and cleanup. Venus owns full-Eon shortcut precedence,
  modal geometry, focus, input, notices, rendering, and accessibility. Orbit
  owns the transient PTY and child. Zoxide owns ranking, fzf owns quick
  selection, Yazi owns filesystem browsing, and Anima owns style selection,
  rendering, input dismissal, playback timing, and terminal restoration. Eon
  consumes one directory result without interpreting Yazi's navigation state.
- **Consumes:** EON-C17, current EONW v7 popup/pending-tab values, Orbit's
  accepted terminal startup/attachment/exit/stop contracts and the selected exact
  Venus consumer.
- **Boundary:** No custom filesystem walker, editor/file-opening integration,
  copied Nova plugin, normal-pane identity, client-supplied command,
  simultaneous terminal composition, native Venus picker,
  placeholder shell, pane replacement, current-process `cd`, persisted pending
  tab, EonTerm action, or additional platform.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Picker lifecycle
    (`directory_picker_browse_preserves_raw_directory_and_cancellation`,
    `directory_picker_retargets_cancels_and_stops_with_venus`,
    `failed_directory_picker_stop_is_retried_during_supervisor_cleanup`,
    `replacement_with_only_a_stale_initial_picker_falls_back_without_reopening_it`);
    home and pending tabs (`new_tabs_start_at_home_and_reject_unavailable_home_without_mutation`,
    `pending_tabs_keep_independent_pickers_and_close_only_the_selected_tab`,
    `background_pending_picker_exit_keeps_the_selected_tab`,
    `two_pending_tabs_roundtrip_without_relaxing_v6_or_identity_checks`);
    popups (`popup_switching_stops_transients_without_leaving_empty_tabs`,
    `chooser_result_requires_its_exact_live_invocation`); Anima
    (`anima_waits_for_native_readiness_then_hands_off_picker_and_popup_stops_on_dismissal`,
    `failed_or_stuck_startup_anima_reports_and_continues_to_picker`,
    `startup_animation_settings_are_validated_and_can_be_disabled`); Venus
    `independent_pending_picker_tabs_attach_only_the_active_endpoint` and
    `shared_popup_projection_keeps_hidden_work_and_one_input_surface`.
    Installed checklist items 1, 3 and 4.
- **Open proof:** Pending re-proof in this repository: installed
  multi-pending tabs; Yazi's native find, sort, help and Esc handling; raw non-UTF-8 directory selection in the packaged
  picker; installed Accelerando playback. Fractional scale,
  broader compositors, actual blur, current AT-SPI/screen-reader use and broad
  user environments remain unqualified. No capture stability or latency bound
  is claimed.

## EON-C19 — Terminal typography and initial geometry configuration

- **Status:** Proven by the automated tier; installed configured typography and
  geometry are pending re-proof in this repository.
- **Consumer:** Eon and EonTerm users opening a native surface.
- **Trigger:** Create or reopen Venus using one `$EON_CONFIG_HOME/config.toml`
  snapshot.
- **Result:**
  - Optional `[terminal]` fields are `font_family`, ordered `font_fallbacks`
    (at most eight), `font_size` (finite 6–96 logical px), `line_height` (finite
    1–3 multiplier) and integer `columns`/`rows` (1–65,535; a supplied pair at
    most 100,000 cells). Names are nonempty trimmed UTF-8, at most 128 bytes
    without controls. The primary family must be installed and monospace;
    requested fallback families must be installed.
  - Omission emits no override, preserving Venus font selection, nominal
    16 px / 1.125 line height and 960×600 logical window. Each omitted dimension
    retains its initial window dimension. Explicit grids include workspace
    chrome; later compositor/user resizing remains authoritative. Reopen reads
    current settings without replacing Orbit or its PTY; live Venus keeps its
    snapshot.
  - The authoritative startup workspace Snapshot transfer and native readiness
    share one absolute five-second deadline, including large snapshots. Ordinary
    Present messages retain their 250 ms write timeout.
- **Important failures:** Eon Runtime rejects malformed configuration with field
  diagnostics before children. Venus admits fonts and actual initial native
  geometry before any new terminal or user command. Rejected, lost or timed-out
  admission closes only the attempted presentation and preserves durable terminals.
- **Owner:** Eon supplies terminal defaults; Eon Runtime owns strict parsing,
  argv projection and authoritative startup Snapshot transfer; Venus owns font
  resolution, native readiness and geometry.
- **Consumes:** Exact accepted Venus VEN-C19 startup admission.
- **Packaged fonts:** The Venus font configuration includes DejaVu, Symbols
  Nerd Font Mono and Anima's font-only Accelerando Terminal artifact. Eon and
  EonTerm share that configuration; EonTerm does not consume the Anima executable.
  The exact Anima component pin owns the font revision. Its U+100000–U+101FFF
  mapping preserves Venus's Nerd Font ranges and the primary text family.
- **Boundary:** Native Linux Wayland; no system-wide font installation, live
  reload, Eon renderer, second schema or primary-font default change.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `terminal_presentation_is_strict_and_bounded`,
    `configured_typography_reaches_both_product_launches`,
    `startup_snapshot_transfer_uses_the_admission_deadline`,
    `rejected_native_startup_creates_no_session_in_either_product` and
    `invalid_presentation_configuration_precedes_supervisor_generation_and_children`;
    Venus `configured_fonts_resolve_fallbacks_and_scale_one_grid`,
    `typography_options_are_bounded_and_composable`,
    `default_font_matches_nova_at_supported_scales`,
    `nerd_font_symbols_use_the_packaged_face_and_bounded_blank_space` and
    `accelerando_specialty_routing_preserves_explicit_fonts_and_styles`.
    Installed checklist item 7: default terminal text, box drawing, Braille and
    CJK.
- **Open proof:** Pending re-proof in this repository: configured fonts, grid
  and reopen on an installed surface; missing-font failure; initial scales
  1.25, 1.5 and 2; IME preedit and commit; AT-SPI text and bounds; generic UI
  font fallback with ambient fonts hidden. Screen-reader use on the current
  build, full fractional/HiDPI coverage, stable-seat IME, broader compositors
  and installed non-systemd support remain unproved.

## EON-C20 — Discoverable native shortcuts

- **Status:** Proven
- **Consumer:** One installed full-Eon user on native Linux Wayland.
- **Trigger:** The user presses physical Alt+Slash from the Eon workspace.
- **Result:** The exact accepted Venus client toggles one native, read-only
  shortcut viewer containing its fixed Eon-surface bindings and the current
  enabled popup catalog supplied through current EONW v7. Opening, refreshing,
  scrolling, resizing, and closing the viewer change no workspace or Orbit
  terminal state. Title, subtitle and first group heading occupy separate
  readable vertical space at normal and compact window sizes.
- **Important failures:** Eon rejects configured popup shortcuts that collide
  with Alt+Slash or positional Alt+0 through Alt+9 before workspace mutation.
  Invalid or stale component identity fails before profile activation. Profile
  refresh preserves running supervisors and terminals; they retain their older
  client until a normal restart.
- **Owner:** Eon owns component/default selection, profile delivery and
  installed acceptance; Eon Runtime enforces configured popup collisions.
  Venus owns viewer state, input, rendering, focus, and accessibility.
- **Consumes:** Venus VEN-C20 and VEN-C5, current EONW v7 and Orbit terminals.
- **Boundary:** No Eon-rendered viewer, second shortcut catalog, EONW change,
  Orbit update, child-application bindings, rebinding UI, command palette,
  tutorial, telemetry, momentary mode, dependency, or platform expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `popup_configuration_owns_defaults_overrides_and_collisions`;
    Venus `shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space`,
    `shortcut_viewer_uses_live_rows_and_captures_its_local_sequence`,
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog` and
    `recognized_shortcuts_are_one_shot`. Installed checklist item 5.
- **Open proof:** Pending re-proof in this repository: header spacing at
  compact sizes, rebound and disabled popup rows, and AT-SPI projection on an
  installed surface. No EonTerm, fractional-scale, screen-reader workflow,
  wider compositor, non-systemd, macOS or direct-distribution proof is added.

## EON-C21 — Native workspace header

- **Status:** Proven
- **Consumer:** One installed full-Eon user on native x86_64 Linux Wayland.
- **Trigger:** The user points at, focuses, scrolls, or activates the top
  workspace row, its tabs, New tab, Shortcuts, active-tab Close, or blank drag
  region.
- **Result:** Full Eon presents one native Eon Bar at the existing tab-strip
  height. Eon-authored tab identity and action policy feed Venus's tab-first
  responsive layout, New tab beside the rightmost tab or pinned at the
  tab-region edge during overflow, Shortcuts when EON-C24 needs its fallback,
  and active-tab Close,
  shortcut tooltips, visible keyboard focus, AccessKit buttons, and bounded
  compositor-owned window dragging. The existing CreateTab and exact active
  CloseTab actions and local VEN-C20 viewer remain their sole behavior owners.
  The accessibility tree places New tab before the optional quota chip.
- **Important failures:** Stale component identity fails before activation.
  Rejected Eon actions never mutate another tab. Narrow surfaces retain the
  active tab and required controls without overlap. A failed native drag causes
  no workspace action, terminal input, crash, or state mutation. Controls, tab
  gaps, pane chrome, terminal content, popups, tooltips, and the viewer never
  become drag targets or leak a terminal pointer sequence.
- **Owner:** Eon owns the exact graph, product policy, profile delivery and
  installed acceptance; Eon Runtime owns workspace identity/actions. Venus
  owns header geometry, rendering, input, native drag, tooltips, and accessibility.
- **Consumes:** Venus VEN-C21, current EONW v7 and Orbit terminals.
- **Boundary:** EON-C27 separately owns the optional quota projection. The core
  header adds no Eon-rendered chrome, second row, widget framework, agent status,
  polling, configuration, protocol change, inactive-tab close, native window
  controls, macOS claim or distribution expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Venus
    `eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure`,
    `eon_bar_controls_reuse_existing_actions_and_local_viewer`,
    `eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation`,
    `tab_text_fits_shaped_width_without_splitting_combining_clusters` and
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`.
    Installed checklist items 3 and 6.
- **Open proof:** Pending re-proof in this repository: real compositor window
  dragging, eight-tab overflow, narrow widths and AT-SPI order on an installed
  surface. AT-SPI tree order is not a screen-reader workflow; macOS and wider
  platform coverage remain open.

## EON-C22 — Trustworthy Codex quota in the Eon Bar

- **Status:** Retired 2026-10-14 by Lucca. EON-C27 owns Codex quota collection
  through CodexBar and the chip policy; Venus `VEN-C22` owns the Codex chip.
- **Reason:** The direct `codex app-server` collector this contract described no
  longer exists, so it cannot be proved in this repository.

## EON-C23 — Independent Eon windows

- **Status:** Proven
- **Consumer:** A full-Eon user on native x86_64 Linux Wayland.
- **Trigger:** `eon window new`, the desktop launcher's New Eon Window action,
  or Venus's Alt+Shift+N shortcut.
- **Result:** A fresh directory picker opens in a second native window with its
  own Eon supervisor, tabs, focus, and durable Orbit terminals. The original
  window and its terminals remain unchanged. `window new` reports a stable ID;
  `eon windows` discovers it, `eon window attach ID` presents its current
  generation after detach, and `eon window stop ID` stops only its terminals.
  `eon window stop all` covers independent windows and attempts the caller's
  own window last; existing `eon stop all` remains scoped to the caller's
  current runtime namespace. Piped confirmations remain available to later
  windows, one answer per prompt.
- **Important failures:** Identity collision, unsafe runtime path, failed
  startup, or stale target neither replaces an existing window nor stops or
  retargets its terminals. An uncertain startup retains its ID, process, and
  private log for inspection instead of killing possible live terminals.
- **Owner:** Eon Runtime owns window identity, namespace and launch/lifecycle
  enforcement; Eon supplies product composition. Venus invokes the exact Eon
  executable supplied by its supervisor and presents one window per process;
  Orbit owns each terminal and its single presentation attachment.
- **Consumes:** EON-C1, EON-C8, EON-C11, EON-C15, EONW v7, and VEN-C23.
- **Boundary:** Bare `eon` retains its current presentation behavior. Windows
  have independent workspaces; no terminal, tab, pane, or focus state is shared.
  The eight-hex-digit ID is unique by atomic directory reservation, not a
  secret. This does not establish macOS or another distribution channel.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `new_windows_keep_independent_supervisors_and_stop_by_window_id`
    and Venus `new_window_shortcut_invokes_the_supplied_eon_command`. Installed
    checklist item 10.
- **Open proof:** Pending re-proof in this repository: the launcher's New Eon
  Window action, and native `eon window stop all` with piped confirmations
  stopping the caller's window last.

## EON-C24 — Bottom shortcut hints

- **Status:** Proven on x86_64 Linux native Wayland.
- **Consumer:** A full-Eon user on native x86_64 Linux Wayland.
- **Trigger:** A fresh window, physical Alt+Shift+B, native focus, current
  enabled popup catalog, or window bounds/scale changes.
- **Result:** One compact native footer starts visible. Its window-local
  visibility survives tab and popup switches and resize; hiding returns its
  space to terminal content. Grouped hints derive from actual bindings and
  enabled/remapped popup entries, with one badge header per modifier group.
  Compact groups spread across the bar with equal gaps,
  outer groups near its edges and a lone group centered. Modifier headers alone
  have padded rounded backgrounds without braces; group bodies stay plain.
  Insets, badge padding and minimum gaps participate in whole-hint fitting.
  The footer shares the sheet's side edges and one continuous outer frame, with
  matching upper/lower corner radii, its bottom on the window's bottom edge and
  a straight separator at the join; column seams end at that separator in square
  T-junctions. Below its compact row height, the footer also reserves the space
  down to the window's bottom edge. The row is at least the corner radius tall,
  preserving the straight separator at small font sizes. The bar uses the
  inactive-tab background, coral red modifier headers, mint keys and gray labels.
  Copy, Paste and Open link retain their bindings and full-help
  entries, without footer hints. Compact headers use C/A/S aliases joined by
  hyphens, such as A-S; the compact tab range is 1..9, while full help retains
  Alt+0 for the tenth tab. Short labels use keys/hints and one move hint for both axes. Full
  help and accessible names retain complete chords and actions; creation
  labels name the tab, pane or window explicitly. Width pressure drops whole hints,
  retaining help and the visibility toggle longest. The header Shortcuts button is
  omitted only when all applicable hints fit and the button lacks keyboard
  focus. Alt+/ and the complete modal viewer remain available. F6 stays in full
  help and keyboard navigation, without a footer hint.
  Project is omitted from hints; Alt+Z and its full-help entry remain available.
- **Important failures:** The toggle is captured once per press, without repeat
  or release leakage. Visibility changes preserve focus, selected identities
  and live terminals; normal resizing is their only terminal effect. Popup
  configuration that collides with Alt+Shift+B is rejected before mutation.
  Tiny bounds preserve valid geometry and whole chords. Accessible hints are
  read-only, without extra focus stops or announcements.
- **Owner:** Venus owns presentation and window-local visibility under
  VEN-C24. Eon Runtime owns popup collision enforcement. Orbit owns terminal
  resizing. Eon owns product composition and exact accepted component pins.
- **Consumes:** EON-C19/C20/C21, VEN-C24, existing EONW v7 and ORBF2/ORBS13.
- **Boundary:** No minor-mode framework, child-application key discovery,
  interactive hints, durable preference, configuration option, dependency,
  wire change or platform expansion. Future real modes replace contents from
  their actual bindings at the same footer height.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `popup_configuration_owns_defaults_overrides_and_collisions`;
    Venus `shortcut_hints_follow_bindings_catalog_and_native_focus`,
    `shortcut_hints_drop_whole_pairs_and_rebuild_modifier_headers`,
    `shortcut_hints_are_read_only_and_retire_hidden_help_actions` and
    `workspace_shortcut_capture_starts_once_and_retains_release`. Installed
    checklist item 5.
- **Open proof:** Pending re-proof in this repository: frame alignment and
  corner geometry across widths, fractional scales and minimum fonts on an
  installed surface. No performance, macOS or screen-reader workflow claim.

## EON-C25 — Running Eon version badge

- **Status:** Proven on x86_64 Linux Wayland.
- **Consumer:** Full-Eon workspace users.
- **Trigger:** A workspace window opens or reopens.
- **Result:** The native top bar shows a read-only `EON <version>` label in regular terminal
  monospace and softened cyan from the supervisor serving that window. `[bar] show_version = false` hides the
  complete label; omission inherits Eon's `true` default. Settings apply when
  opening/reopening a presentation. Updating an installation cannot relabel a
  running supervisor. When visible, the badge precedes shortcut help and close
  remains last.
- **Important failures:** Missing identity produces no badge; invalid launch identity is
  rejected. Narrow layouts hide secondary information before required controls
  and the active tab lose their reserved space. Invalid configuration fails
  through existing startup/replacement handling, preserving live terminals.
- **Owner:** Eon supplies product version/default and exact composition; Runtime
  parses configuration and supplies immutable supervised launch inputs; Venus
  owns native rendering, width fitting, hover and static accessibility.
- **Boundary:** One badge and one boolean; no widget system, polling,
  EONW change, channel/build-ID promise, settings UI or platform expansion.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `version_badge_uses_supervising_product_and_visibility_at_each_launch`;
    Venus `eon_version_is_bounded_workspace_identity` and
    `version_label_uses_only_spare_space_and_has_no_actions`. Installed
    checklist item 6 shows `EON 0.1.0`.
- **Open proof:** Pending re-proof in this repository: `show_version`
  false/true on reopen, narrow hiding, scale 1.25, header order with shortcut
  help and close, and a supervisor whose version differs from its client. No
  performance, macOS, screen-reader workflow or Nix-free delivery claim.

## EON-C26 — Passive Claude quota in the Eon Bar

- **Status:** Retired 2026-10-14 by Lucca. EON-C27 owns Claude quota collection
  through CodexBar and the chip policy; Venus `VEN-C26` owns the Claude chip.
  `eon claude-statusline` remains a bounded silent sink under EON-C27.
- **Reason:** The status-line collector this contract described no longer
  exists, so it cannot be proved in this repository.

## EON-C27 — CodexBar quota collection

- **Status:** Proven on x86_64 Linux by the automated tier; installed quota
  observation is pending re-proof in this repository.
- **Consumer, trigger and result:** Full Eon starts independent, noninteractive
  CodexBar CLI reads for Codex (`--source cli`) and Claude (`--source oauth`).
  Codex reads start at least one minute apart and Claude reads at least five
  minutes apart, including after a failed read; reads run off workspace I/O.
  The existing native chips show every supplied supported window and remaining
  percentage, with provider periods, optional resets and observation time.
  Claude needs no status-line setup. No reset is invented for an idle window.
- **Ownership:** Pinned CodexBar owns provider collection and credential access;
  Runtime owns bounded subprocesses, validation and freshness; canonical EONW
  v9 permits absent resets, preserving all historical validators; Venus consumes
  typed facts through its existing chip owners. Eon owns exact Nix composition.
- **Failure policy:** Each read has a ten-second deadline and 64 KiB output cap.
  A failed read keeps retained data unchanged; age alone marks it old after one
  hour. Readings disappear after 24 hours and windows at their reset.
  Successful missing quota clears prior facts. Invalid, future, estimated or
  unsupported facts never become zero usage. Provider failures are independent.
- **Boundary:** One selected account per provider, Linux Wayland and Nix only.
  Explicit sources avoid browser fallback, interactive login and model turns.
  No credential, identity, raw output or billing data enters EONW or Eon logs.
  Sign-out is not detected: CodexBar reports it as a failed read, so the last
  reading ages like any other (Lucca, 2026-10-16). Blocked/unknown labels are
  not guaranteed.
  CodexBar owns its local authentication/cache mechanisms. Eon does not rewrite
  provider configuration; `claude-statusline` remains a bounded silent sink for
  existing wrappers and no longer owns quota or persistent cache state.
- **Source choice:** CodexBar v0.74.1, source
  `7c08c0ae783b72040b728757eb85a6cfe4fc2443`, MIT; upstream musl x86_64 release
  and resources are hash-pinned. This accepts its Swift binary size to remove
  Eon-owned provider mechanisms. Go/Grok delivery and possible Muse remain
  outside this first migration slice.
- **Decision record:** `eon-codexbar-quota-backend-ioqy`; earlier evaluation
  `eon-evaluate-codexbar-quota-backend-r2ks` supplies source evidence, not installed
  acceptance. User approval supersedes that evaluation's retain-current decision.

- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier.
  - **Evidence:** CodexBar-backed workers
    (`actual_periods_optional_resets_and_missing_quota_are_preserved`,
    `retained_observations_age_and_expire_without_invented_resets`,
    `invalid_source_values_and_timestamps_never_become_fresh_usage`,
    `workers_isolate_failure_and_bound_output_timeout_and_shutdown`); EONW
    `optional_resets_roundtrip_without_relaxing_historical_or_value_boundaries`;
    Venus `quota_without_reset_retains_percent_and_accessible_freshness` and
    `quota_icons_preserve_transparency_and_shape_across_scales`; the
    `eonterm-closure` check keeps CodexBar out of EonTerm. The failure-policy
    change (`eon-quota-old-after-hour-knaq`) is proved by the updated
    `retained_observations_age_and_expire_without_invented_resets` and
    `workers_isolate_failure_and_bound_output_timeout_and_shutdown`; that
    bead's commit is its revision. The Claude cadence change (`eon-flf4`) is
    proved by `claude_waits_300_units_after_a_failed_read_while_codex_reads_every_60`;
    that bead's commit is its revision.
- **Open proof:** Pending re-proof in this repository: real installed Codex and
  Claude windows with reset details on hover; on an installed surface, a
  controlled source failure leaving a recent reading unchanged and a retained
  reading turning `old` after one hour; installed Claude reads at least five
  minutes apart; and optional chips hiding at narrow widths. No hardware GPU,
  reader workflow or macOS proof is claimed.
- **Claude icon:** Venus shows Claude's splat in its original orange
  (`#D97757`) within the existing quota chip bounds and owns the immutable mask
  and tint. Quota facts, layout, hover, accessibility and provider mechanisms
  remain under their existing contracts.
