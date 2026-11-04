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
- **Retired:** explicitly replaced, removed, or demoted to a feature; its ID is
  never reused

Proofs cite revisions of this repository. "Pending re-proof in this repository"
marks an earlier proof that was not repeated after consolidation; it is
re-proved when that platform or feature is next touched.

The [component manifest](../components/eon-v5.json) owns selected source
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

`eon-rename-eonterm-eonbase-jpbm` renamed EonTerm to eonbase, with its command,
package, closure check and tests; contracts below use the new names. Records of
earlier revisions keep the artifacts and check names they ran.

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
- Only behavior that meets the
  [contract criterion](AGENT-RULES.md#contract-driven-method) belongs here: a
  wire or format, lifecycle, trust rule, ownership invariant, script or agent
  surface, or platform scope. Presentation is a feature, kept in its bead and
  the user documentation.
- Every implementation Bead names the contracts it changes, proves, consumes,
  hardens, or preserves and records exact child revisions.
- Historical execution evidence belongs in Beads and Git, not this current-state
  index. User-visible chronology belongs in `CHANGELOG.md`.
- Later work touching a proven owner reruns its indexed checks. Its owning Bead
  records the result in the commit that carries the change. Edit this index only
  when a contract, its checks, status, or gap changes; existing proof records
  stay valid as written.

## EON-C1 — Exact compatible component launch

- **Status:** Proven. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-369s`: Nix derives the Eon Bar's icon image.
- **Consumer:** A user launching Eon or eonbase from one accepted product
  generation.
- **Trigger:** Eon starts a new composed runtime or adopts an exact live run.
- **Result:**
  - Eon validates one compatible component set, launches only that set, crosses
    each required Ready boundary before publication, and reports every exact
    external component revision and the generation of its in-tree build.
  - `assets/eon.png` is the transparent canonical application icon; Nix derives
    exact hicolor sizes and the Eon Bar's icon image from it, and installed
    desktop metadata names `eon`.
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
    `eon attach` keeps its cursor color. eonbase's first window opens before
    its terminal endpoint exists, so it may differ from later reopens, which
    share one preset. Venus owns `random`, `preset:<name>` and
    `custom:#RRGGBB` parsing, colors and rendering; Orbit cursor authority remains.
    Native admission rejects an invalid choice before a new terminal starts;
    rejected reopen preserves the existing terminal and command.
  - Standalone `eon -h` and `eon --help` print grouped command descriptions and
    examples to stdout and exit successfully without starting children or
    preparing configuration/runtime state. `eon FAMILY -h`, `eon FAMILY --help`,
    `eon FAMILY COMMAND -h` and `eon FAMILY COMMAND --help` print that family's
    or command's lines from the same descriptions and exit the same way; an
    unknown family or command remains invalid syntax. Terminal output uses
    color unless `NO_COLOR` is nonempty or `TERM=dumb`; redirected output has
    no ANSI escapes.
    Invalid syntax remains a concise stderr failure pointing to `eon --help`.
    Child arguments, including Anima help, retain their existing forwarding.
  - Standalone `eon --skill` prints the agent skill this build ships
    (Eon's `eon/skills/eon/SKILL.md`, embedded at build time) to stdout
    unchanged and exits successfully without starting children or preparing
    configuration/runtime state; extra arguments are invalid syntax.
    `eon --help` lists it and ends with a line pointing agents to it.
  - Eon owns the cursor-tail duration default `1.5` and the optional
    `[terminal] cursor_trail_duration` multiplier in the same config file.
    Zero disables the trail while retaining the cursor color and contrasting
    edge; finite values from `0.25` through `4.0` enable it. These values apply
    on new/reopened Eon and eonbase surfaces. Invalid duration fails before
    children; live surfaces retain their startup snapshot. Venus consumes the
    launch value and owns bounded motion,
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
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`;
  `eon --skill`: `eon-ivge0`, whose commit is its revision.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Graph validation and version report
    (`canonical_manifest_is_valid`, `invalid_graphs_are_rejected`,
    `version_report_contains_identity_without_resolved_paths`); help
    (`help_is_readable_and_does_not_start_or_prepare_a_workspace`); Anima
    forwarding (`anima_command_preserves_child_arguments_status_and_avoids_workspace`);
    cursor-tail launch values (`configured_cursor_tail_reaches_both_product_launches`);
    failed native admission (`rejected_native_startup_creates_no_session_in_either_product`);
    the `default` and `eonbase-closure` Nix checks. Installed checklist items
    1 (Anima welcome, picker, first terminal) and 7 (terminal text including
    box drawing, Braille and CJK); Lucca opened the installed build from the
    `Open Eon` desktop entry on COSMIC (recorded in `eon-monorepo-2oko.6`).
  - **`eon --skill` (`eon-ivge0`):** automated tier on x86_64 Linux: sandboxed
    `nix develop -c eon-test -p eon-runtime -p eon`, including
    `skill_prints_the_shipped_skill_and_help_points_agents_to_it`, and the
    `default` package build, whose `bin/eon --skill` matched
    `eon/skills/eon/SKILL.md` byte for byte.
- **Open proof:** Pending re-proof in this repository: installed
  `eon anima accelerando` playback, installed scrollback,
  ReturnToLive, click-selection ladder and copy/paste, installed help color
  gates (`NO_COLOR`, `TERM=dumb`, 80-column PTY), explicit cursor-trail colors
  and durations on new and reopened surfaces, hicolor icon sizes, retained
  history and terminal-query replies during synchronized-output bursts,
  installed `eon --skill` and `eon --help` from a refreshed `eon` profile
  (`eon-ivge0`), and COSMIC sessions. Fractional scale, broader compositors
  and screen-reader use remain unproved.

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

- **Status:** Candidate. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-369s`: the Eon icon's image is an opaque input.
- **Consumer:** Eon launch and every distribution channel.
- **Trigger:** A channel supplies exact component artifacts to Eon.
- **Result:** Eon receives explicit component paths, treats Nix store paths as
  opaque inputs, and invokes no Nix evaluator during normal runtime. The Nix
  full-Eon launcher supplies the pinned Anima executable and the Eon icon's
  colour image as opaque inputs.
  Nix builds Orbit, Venus, Eon and eonbase from the in-tree crates. Generation
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
    `eonbase-closure` Nix checks build Orbit, Venus, Eon and eonbase from the
    in-tree crates. For the icon image (`eon-369s`), the `default` check's
    image size assertion and the extended
    `version_badge_uses_supervising_product_and_visibility_at_each_launch`.

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
  `runtime/` owns the ten runtime mechanisms, parsing, validation, mutable
  state and canonical EONW. Eon retains assembly, defaults,
  graph, distribution and process exit handling. The
  [runtime boundary](ARCHITECTURE.md#runtime-library-boundary) records the
  runtime seam and in-tree package selection.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** The workspace suites run with in-tree Orbit, Venus, Eon
    Runtime and EONW; `orbit-governance`'s
    `workspace_crates_keep_their_dependency_direction` enforces the crate
    ownership edges; the `eonbase-closure` check keeps Eon-only tools out of
    eonbase. Installed checklist items 1, 4 and 11 exercise Eon policy over
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
- **Host inputs:** The direct Linux bundle takes glibc (its loader,
  libraries, NSS modules and locales), `libgcc_s`, Venus's
  `libwayland-client` and the Vulkan driver from the host as declared
  inputs; the Nix channel pins them. Component identities and the graph
  are unchanged: the bundle ships the same store paths, relinked for the
  host glibc, with the canonical manifest and lock bytes. The
  [manual bundle](MANUAL-BUNDLE.md#execution-and-host-boundary) declares
  the glibc floor.
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
  Orbit `ORB-C14` is proved natively on Apple Silicon at
  `0e47d26df690c7ff4b2f1a000e174b307a17e656`; Venus `VEN-C16` and full Eon have
  partial native proofs in this repository. Apple Silicon macOS remains
  unsupported until the Venus and full-Eon native proofs pass.
  X11, Xwayland, Intel macOS, signing, notarization, direct bundles, and public
  macOS distribution remain outside this contract.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`;
  Apple Silicon, partial, full Eon:
  `0e47d26df690c7ff4b2f1a000e174b307a17e656`, `eon-mac-launch-kz4p.12`; the
  combined Linux/macOS contract is not proved.
  - **Environment:** Automated tier; installed tier. Apple Silicon: Apple M2
    Mac mini (Mac14,3, 16 GiB) rented from Scaleway, macOS 27.2 (26B123),
    `aarch64-apple-darwin`, one 1920x1080 scale-1 display offering HiDPI
    modes. Nix 2.35.4; the dev shell's Rust 1.95.0. `mac-proof final` ran from
    `github:Yazelix/eon/0e47d26df690c7ff4b2f1a000e174b307a17e656` in a
    logged-in GUI session, driven over SSH, with Screen Recording and
    Accessibility granted. Keys were posted by a CGEvent helper; no physical
    keyboard or pointer was used. Archive
    `eon-mac-final-0e47d26df690-2026-10-29.tar.gz`, sha256
    `1b9512a7e1330868dc94748cd3248626f01345e690cdc3032c0af703bdb3c652`.
  - **Evidence:**
    - x86_64 Linux: installed checklist items 1 through 5 and 10 through 12
      cover native launch, picker, tabs and panes, popups, shortcuts and
      hints, detach and attach, generations, independent windows, eonbase and
      exact-generation Stop on headless Sway; items 6 through 8 pass for the
      Eon Bar and version label, terminal text and opacity. Lucca opened the
      build from the desktop entry on COSMIC and reported a general visual
      pass.
    - Apple Silicon at `0e47d26`:
      - The automatic part ran 90 steps with none failed or skipped, and
        `eon versions` names `aarch64-darwin` for Eon 0.1.8 and every
        component.
      - The package and every `checks.aarch64-darwin` output at this revision
        built natively. `cargo test` passed for `eon` (50), `eon-runtime`
        (140, 1 ignored), `eon-workspace-protocol` (14) and `eon-manifest`
        (4).
      - In the GUI scenario, `eon run` started with a private HOME,
        configuration and runtime root, though codexbar still wrote its files
        into the real HOME (`eon-mac-launch-kz4p.45`); after Escape at the
        directory picker, its command ran as the only pane, in the launch
        directory.
      - A pane sees `TERM=eon`, which macOS's `/usr/bin/tput` resolves.
      - `eon pane create` worked from outside and inside Eon, `eon tab
        create` from outside, and `eon focus` moved the selection.
      - `eon edit` opened a pane running Helix that stayed live.
      - Option+Shift+Y showed and hid Yazi, and Option+Shift+Z and Option+/
        changed the window and a second press restored it.
      - Option+Shift+D detached and `eon attach` reattached. After Venus was
        killed, `eon attach` showed the same terminals again. After the
        supervisor was killed, the Orbit hosts outlived it and a new `eon`
        adopted every terminal (`EON-C15`).
      - `eon stop all` left no process behind.
      - Manual items, judged from captures, passed for fonts (Han, kana and
        accents render, and 👍🏽 and 🇯🇵 each show as one glyph), the keys the
        Shortcuts viewer lists (a tab, a pane, moving between panes and the
        Project popup, each also checked in `eon workspace --json`), and
        Yazi's image preview.
- **Open proof:** Pending re-proof in this repository: installed quota chip
  detail, Accelerando playback, blur, pointer interaction (item 9), a full
  COSMIC checklist, install from this repository's first tag, clean-account public-source qualification,
  and supervisor-loss adoption.
  EON-C8 records reader qualification as pending. Wider compositors,
  fractional/HiDPI coverage, proprietary NVIDIA and installed non-systemd
  behavior remain unproved. Machine-restart recovery is not supported.
  On Apple Silicon, a physical Retina panel, a second display, input methods,
  the agent sidebar with Claude Code and Command+C to another app are
  unobserved, as are VEN-C16's open items;
  `eon-prove-full-eon-apple-silicon-macos-t8o` owns the full-Eon proof, and
  compile/evaluation is insufficient.

## EON-C8 — Durable tab and pane workspace

- **Status:** Partially proved
- **Consumer:** One local Eon workspace user.
- **Trigger:** Launch, create or close a tab, create, close or rename a pane, create a column, traverse focus,
  reorder the active tab or selected pane, move the selected pane into a neighbouring column, zoom the
  selected pane, receive terminal exit, quit the window, or recover accepted same-boot runs.
- **Result:**
  - A workspace contains horizontal tabs. Each established tab with panes
    holds an ordered row of one to four columns (pending and popup-only tabs
    hold none); each column is a vertical accordion of panes with exactly
    one expanded pane. Each pane views an independent persistent terminal.
    Unless the tab is zoomed, every column's expanded pane is visible at once;
    exactly one of them is the tab's selected pane and receives keyboard
    input. A one-column tab behaves as a single accordion.
  - Tabs use stable `tN` identities. Each tab header shows its number plus the
    directory leaf or `/` derived from Eon's authoritative launch directory;
    actions retain `tN`, and accessibility includes full launch-path context. Venus
    renders pill-shaped, separated tabs sized to shaped labels, capped near 280
    logical pixels at default typography. Long names use middle ellipsis;
    narrow tabs retain their number whenever it fits. Hover previews the launch
    path, and wheel scrolling works across the whole strip, including gaps.
    Selected fill and brighter text identify the active tab without an underline;
    keyboard focus adds a separate rounded outline.
  - Each pane retains its stable `pN` identity. Runtime supplies its label: the identity, two ASCII
    spaces, then the pinned name, otherwise the sanitized terminal title, otherwise the compact working
    directory, otherwise no name. The default style appends ` · DIRECTORY` when the terminal directory
    differs lexically from the tab's launch directory, without repeating a directory already used as the
    name. Context uses a relative path, or the compact path when shorter; compact paths use `~` for home
    and `~/` for its descendants. Labels preserve explicit offline, connecting and unavailable states.
    Program metadata is untrusted display text, never identity or command input.
  - `eon pane rename PANE NAME [--json]` pins a name; `eon pane rename PANE --clear [--json]` restores automatic naming. `--` permits an option-looking NAME. Names are trimmed; an empty result clears the pin. Invalid names fail without mutation. Rename addresses an explicit pane anywhere in the addressed workspace and changes no workspace, tab, pane or popup selection or desktop focus. Window rename and clear actions use the same mutation. Pins follow pane moves, survive presentation detach and reattach, and end with the pane or supervisor; recovery does not restore them. Workspace inspection and action replies expose each pane's `label` and nullable `pinned_name`, independently of attention records.
  - On each new or reopened full-Eon surface, `[terminal] pane_frames` in
    `config.toml` defaults to `true` and draws each pane as its own card,
    stacked without gaps in its column's allocated card area: a rounded border
    `[terminal] border_width` logical pixels
    wide (1 to 3, default 1) with pane-title rules as VEN-C8 describes, using
    its shared chrome title metric. Pane-card titles and the shared top line
    follow VEN-C8, including VEN-C18's tiny-window popup exception.
    Allocated column card areas keep one even gap (8 logical pixels at default
    typography, also their corner radius) from the window's sides and from
    each other; below them the footer's band
    is the margin, or one gap when hints are hidden. The tab pills keep the
    same gap from the window's left edge, in both modes, and are centred
    with the Eon Bar widgets as VEN-C21 describes. Card fills start at
    each card's rounded top edge and use terminal background color/opacity as VEN-C8,
    VEN-C18 and VEN-C30 describe, under the existing surface-wide blur request.
    Everything below the tab bar outside those fills, the gaps and footer band
    included, is the top bar's opaque color, so the bar, gaps and hints read as one
    frame. On a window too small for them the gaps
    yield before a terminal cell does. The selected pane's outline is
    `[terminal] focus_border_width` logical pixels wide (1 to 3, default 1).
    The selected pane's outline uses `[terminal] focus_color`, `preset:<name>` or `custom:#RRGGBB`, default
    `custom:#b207fb`. Pane hover and keyboard focus use VEN-C8's pill
    feedback. A selected popup or
    picker draws as one card over the same allocated area. `false` removes
    the card borders, title rules and frame (a popup keeps its outline over
    terminal background) while retaining label-only headers and their
    selection, hover and keyboard-focus feedback. Both modes
    preserve terminal-grid and picker geometry. eonbase receives no
    workspace-frame override. Requested initial grids account for
    the card gaps. Live surfaces retain their settings;
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
    Ctrl+Alt+M to it and, in a tab with one to three columns, refuses it with
    a notice when a column would get fewer than 40 terminal columns. Alt+M adds
    a pane to the selected column.
  - Zooming shows the selected pane alone across the area all of the tab's cards
    cover, as one card with its label in the top edge; the tab's other panes keep
    running, hidden. Venus maps Alt+Shift+Z to it as transient
    window state that no workspace action, EONW field or attachment carries.
    Zoom ends with any change of the active tab, its panes or columns, or its
    selected pane, so Alt+H/J/K/L unzooms and then moves, and adding, closing or
    moving a pane ends it. Toggling again restores
    the same columns, widths and selection at the current window size. A tab
    with one pane refuses it with a notice; while a popup or picker covers the tab it does nothing,
    and a popup over a zoomed tab keeps its usual geometry. The active tab's
    pill and accessible name mark the zoom while it shows, not while a popup
    covers it. Like other fixed bindings, a popup configured on the key takes
    it. Venus owns zoom state, geometry, rendering, input and accessibility.
  - Moving the tab left/right swaps the active tab with exactly one adjacent
    tab; moving up/down swaps the selected pane with exactly one adjacent pane
    in the selected column. Moving the selected pane sideways places it in the
    neighbouring column's stack directly below that column's expanded pane,
    expanded; the source column expands its following pane, otherwise its
    preceding one, and disappears when emptied. Past the tab's outer column a
    pane from a stack opens a new column at that edge while the tab has fewer
    than four columns; a pane alone there, or a fifth column, stops quietly.
    Moving a pane sideways, up or down is unavailable while a popup or picker
    covers the tab.
    The moved stable identity remains selected and keeps its terminal. Movement
    stops quietly at ordered edges and remains unavailable in the picker-bound
    active tab. Venus maps Ctrl+Alt+Shift+H/L, Ctrl+Alt+K/J and Ctrl+Alt+H/L
    directly to these actions without a mode; `eon pane move left|right` sends
    the same EONW v12 sideways move. Run from an Eon terminal, `eon tab move`,
    `eon pane move`, `eon pane create` and `eon pane close` act on the calling
    terminal's tab and pane instead of the active tab and its selected pane
    ([EON-C10](#eon-c10--typed-workspace-control)).
  - Closing names the expected active `tN`. A non-final pending tab first
    stops its exact picker, then disappears and restores its prior tab. Eon
    stops the tab's pane and popup terminals together, under one shared
    five-second bound, and prunes each confirmed end, in tab order, through
    the normal terminal-exit owner. Complete success removes the tab with the
    same deterministic focus rule as natural exit. If any stop fails, the
    action reports failure, and every terminal whose end was not confirmed
    stays represented in the partially pruned tab. A terminal that ended
    naturally during the request is reconciled and pruned as an exit rather
    than reported as a successful Stop.
    Closing the final tab of the window's last workspace (EON-C33) ends the
    window. Eon stops its pane and popup
    terminals as above, then, as Stop does, any run of the generation that no
    tab tracks; once every stop is confirmed it ends the generation, as when
    the last terminal exits. The reply is EONW `Ended`, and Venus then exits.
    While other workspaces remain, closing a workspace's final tab stops that
    tab's pane and popup terminals and removes the workspace; if it was shown, the
    most recently shown remaining workspace is shown. The reply is that
    snapshot, and the window stays.
    A failed tab stop keeps the window
    and its remaining terminals, as a partial close does; a failed stop of an
    untracked run fails as Stop does. `[workspace] warning_close_last_tab`,
    true by default, makes the CLI ask first (EON-C10) and makes a final close
    from Alt+Shift+W, the bar's Close tab button or Alt+W first show a Venus
    prompt: "Close the last tab? This quits the window and stops N
    terminals." It lists the tab's panes and popups by title from the current
    snapshot, then "Enter Quit · Esc Cancel", then the hint line "Alt+Shift+D
    keeps it running instead", which is left out while a popup takes
    Alt+Shift+D (EON-C20); Alt+Shift+D works while the prompt is open. While
    other workspaces remain, the prompt shows too, with EON-C33's wording:
    "This closes workspace NAME and stops N terminals." and "Enter Close ·
    Esc Cancel". The prompt follows the current snapshot, so an open prompt
    re-words when the workspace count changes. If the snapshot gains a tab
    while the prompt is open, Enter closes only the named tab and the window
    stays. When the setting is false, a final close happens at once.
    Venus receives the setting as a launch argument (VEN-C8).
    Venus maps Alt+Shift+W directly to this stable-target action; lowercase
    Ctrl+W remains terminal input. Alt+Shift+D detaches every workspace
    together without asking; attach then shows the last shown workspace
    (EON-C11). Closing the window itself quits it as Alt+Shift+Q does, prompt
    included, and a close request while the quit prompt is open leaves it
    open; where there is no Quit key (eonbase), while the workspace is
    unavailable, or while it refuses refreshes after a failed Stop, closing the
    window detaches as Alt+Shift+D does. A close request carries no reason, so
    a desktop that closes each window at logout, such as KDE Plasma, quits an
    idle window and shows the prompt on a busy one; there, Alt+Shift+D before
    logging out keeps the terminals. A desktop that ends the session without
    closing windows first detaches the window as control loss does (VEN-C14).
    Each time a full-Eon window detaches this way, its supervisor sends one
    non-activating desktop notification titled "Eon window NAME is still
    running", NAME being the window's EON-C23 name. Its body says how many
    live terminals keep running, counted as EON-C23's `this` terminals, and
    how to return: "Open Eon to return." in the default namespace, or "Run
    `eon window attach ID` to return." in an independent window. When the
    notification service advertises the `actions` capability, the
    notification offers Quit; otherwise its body ends "Run `eon window stop
    ID` to stop it." in an independent window, or "Run `eon stop GENERATION`
    to stop it." in the default namespace, naming that window's generation.
    A notification that offers actions carries the `resident` hint.
    Quit shows a confirmation titled "Quit NAME?" that says how many
    terminals stop and offers Quit and Keep running. The confirmation
    replaces the notification in place, or arrives as a new notification
    when the service removed the notification on the action. Quit stops the
    window with the Stop that `eon quit` uses (EON-C11); a failed Stop says
    so in a notification, and the window stays for a retried Stop (EON-C11).
    Keep running closes the notification and stops nothing. Eon acts only on
    actions sent by the bus connection that issued the notification, for
    the notifications this supervisor sent; it replaces and closes a
    notification only there, and forgets it when that connection gives up
    the service's bus name. A later detach of the same window replaces its
    notification, and attaching the window closes it; a close the service
    fails is retried at the next attach or end, and a detach before then
    replaces the notification. Ending the window closes it as a best effort,
    waiting at most 500 ms. A missing or failing notification service sends
    nothing and does not delay the detach, and a later detach tries again. A
    Venus crash sends none, nor does a detach while the supervisor is
    stopping. Linux only; on macOS a detach sends no notification.
    Alt+Shift+Q quits the window: Venus sends the Stop that `eon quit` uses
    (EON-C11) for the window's generation, which ends every workspace, and
    exits once it is confirmed. While the snapshot lists a busy terminal
    (EON-C11), Venus first shows a prompt in the final-close prompt's style:
    "Quit this window?", "This stops N terminals; M still running." (the
    second part is left out while nothing is busy), the busy terminals by name
    from the current snapshot and how many more are busy, then "Enter Quit ·
    Esc Cancel" and the hint line "Alt+Shift+D keeps it running instead",
    which is left out while a popup takes Alt+Shift+D (EON-C20); Alt+Shift+D
    works while the prompt is open. N counts the window's live terminals
    (EON-C23); without that count the subtitle reads "This stops every
    terminal of this window.". While no terminal is busy it quits without
    asking, as `eon quit` does, deciding on the next snapshot so that a
    command started since the last one still asks;
    `[workspace] warning_close_last_tab` does not apply. While a Stop is in
    flight, Alt+Shift+Q and Enter do nothing. A failed Stop keeps the window
    and shows the failure, and Alt+Shift+Q then tries again.
    eonbase has no Quit key.
  - Closing a pane names a `pN` in the active tab, or in the calling
    terminal's tab under EON-C10. Eon stops that pane's
    terminal and prunes the confirmed end through the normal terminal-exit
    owner, so its column, its tab and focus follow the ended-terminal rules
    below. Closing the final tab's last pane acts as final-tab close does,
    stopping that tab's popups with it. Closing is unavailable
    while a popup or picker covers the tab, except that a calling terminal
    of that tab, pane or popup (EON-C10), may close a named pane other than
    the tab's last; the popup stays shown and selected, and the tab's pane
    selection follows the ended-terminal rules below. A failed stop reports
    failure and keeps the pane; a terminal that ended naturally during the
    request is reconciled as an exit. Venus maps Alt+W directly to the
    selected pane without a mode; `eon pane close PANE` sends the same EONW
    v11 action.
  - Ended terminals leave no dead pane or empty tab; focus moves deterministically
    to the same identity when possible, otherwise the following sibling at the
    removed index, otherwise the preceding sibling. A workspace left empty
    while other workspaces remain is removed, and if it was shown, the most
    recently shown remaining workspace is shown (EON-C33); the last workspace
    left empty closes Venus and the supervisor. A column whose last pane ends disappears; if it
    held the selection, selection moves to the following column, otherwise the
    preceding one.
  - Same-boot recovered runs project numerically into one column of synthetic
    `t1` as `pN`
    without claiming restoration of prior topology or launch-directory policy.
  - New current-generation full Eon surfaces omit the redundant native title
    bar while retaining the selected terminal title for compositor semantics.
- **Important failures:** Unknown or stale identity, a close target other than
  the active tab, a pane outside the active or calling tab,
  pane close under a popup or picker other than a calling terminal's close
  of a non-last pane in its tab, creating a fifth column,
  durable close in the picker-bound tab,
  invalid transition, unavailable endpoint, duplicate identity, or partial
  recovery leaves accepted topology unchanged. A request ID repeated within
  the last 256 accepted mutations never repeats a stop, and a replay naming a removed tab cannot close its successor.
  A final close never ends the generation until every terminal stop is
  confirmed, and a repeated request ID never ends it twice.
  Picker-stop failure changes no topology; durable partial-stop behavior is the
  explicit result above. Exhausted pane numbers fail before terminal start;
  unknown or repeated terminal-exit notices remove nothing; losing a view never
  silently stops or substitutes a terminal. Rejected workspace actions retain
  their explanation across unchanged polls and announce it without moving
  terminal focus; a changed workspace retires the notice, but a snapshot that
  differs only in EON-C31 attention or `sidebar` keeps it and disturbs no
  navigation, scroll or attachment.
- **Owner:** Eon Runtime workspace topology, identity, focus, pruning and action
  enforcement; Eon supplies product defaults/policy. Orbit owns terminals and
  Venus owns native materialization.
- **Consumes:** Current EONW v31 workspace/popup values, Orbit terminal
  identities/endpoints and Venus `VEN-C8`.
- **Boundary:** No arbitrary split tree, vertical splits, column resizing,
  arbitrary reorder target, cross-tab pane movement, picker relocation, durable layout
  restoration, a per-pane terminal restart action, closing panes outside the
  active or calling tab, terminal
  content/history, provider state (except EON-C31's attention values), plugin
  surface, remote access, reconstructed terminal state, or notifications other
  than the detach notice.
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
    `close_parsers_require_one_stable_identity`); pane frames
    (`configured_pane_frames_reach_only_workspace_launches`,
    `pane_frame_option_is_a_strict_single_boolean`); Venus labels and
    accessibility order (`tab_directory_labels_use_current_position_and_bounded_path_context`,
    `pane_metadata_label_is_bounded_safe_and_has_honest_fallbacks`,
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`).
    Installed checklist items 3 and 4.
  - **Final close and detach (`eon-pg67`):** Final tab and final pane close,
    ending the window, are covered by
    `closing_the_final_tab_ends_the_window_and_its_terminals`,
    `delayed_second_cli_receives_committed_workspace_and_controls_three_sessions`
    and `pane_close_targets_the_active_tab_and_prunes_like_an_exit`; the Venus
    prompt and its launch argument by
    `close_prompt_asks_only_for_a_window_ending_close` (renamed
    `close_prompt_asks_for_a_final_close` by
    `eon-workspace-switching-shn4.7`),
    `shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space`,
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog`,
    `pane_frame_option_is_a_strict_single_boolean` and
    `venus_receives_workspace_endpoint_only_in_workspace_mode`. The
    first-detach notice (`eon-first-detach-notification-a0p3`) has no
    automated test, since the test sandbox has no D-Bus. Installed proof
    (`eon-prove-close-and-detach-batch-bf8t`): edge
    `22915af5dca41f87e706ad548c0506f79c17648f`, whose build is the installed
    `eon` profile `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`
    (EONW 18), on x86_64 Linux in a private headless Sway (1280x720) with a
    private D-Bus. With the warning on,
    Alt+Shift+W, the bar's `×` and Alt+W on the final tab's last pane each
    showed the prompt listing `p1`; Escape kept the window and its terminal,
    and Enter ended the window with no Orbit terminal of that generation
    left. With `warning_close_last_tab = false`, Alt+Shift+W ended the window
    at once. Alt+Shift+D from the prompt and from the workspace detached the
    window with its terminal live, and `eon attach` returned to the same
    `session-1` each time. On a fresh state directory, with a stub
    notification server recording each request, the first detach sent one
    notification with no actions, and the second sent none. In a second fresh
    run with the mako 1.11.0 notification daemon, detaching an independent
    window displayed the notice while keyboard focus stayed on the remaining
    window, whose terminal took the next typed command; a second detach added
    no notification. In an
    independent window, Commands > Quit this window asked to stop that
    window's terminals and, confirmed, ended only that window; the default
    window kept its terminal and its Orbit process.
  - **Stops together (`eon-8wh7h`):** the stop step every close shares
    stops its terminals together: seven that each take Orbit's 500 ms grace
    stop in under 2.5 s, the confirmed ends come back in close order, and
    only an unreconcilable terminal stays tracked, in its place
    (`a_close_stops_its_terminals_together_and_keeps_only_the_unended`).
    Two runs whose cleanup never completes hold it for one shared
    five-second bound, not one each
    (`a_close_waits_for_unreconciled_ends_within_one_shared_bound`).
  - **Quit key (`eon-7cf0y.3`):** the busy rule, the Stop, its retry after
    `stop-incomplete` and the Enter hold by `quit_asks_only_while_busy`,
    `quit_progress_follows_workspace_events` and
    `a_second_enter_reaches_nothing_while_a_stop_is_in_flight`; the prompt's
    wording and rows by `quit_prompt_words_counts_and_lists_busy_terminals`
    and its AccessKit alert by
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog`; Stop's
    lifecycle reply by `workspace_worker_reads_stop_as_a_lifecycle_reply`;
    the ignored native regression
    `quit_prompt_refreshes_and_a_stop_in_flight_holds_enter` on a private
    headless Sway. On code tree
    `bceda181484faa57079edec69284981de748d7e0` (edge `6908903`),
    `eon-test -p yazelix-venus` passed 170 + 75 + 21. Full `eon-test` passed
    every target across two runs at load 25 to 31: two timing-bound tests
    outside this change, `bound_io_accepts_einval_only_after_peer_hangup` and
    `concurrent_launches_converge_and_generation_stop_is_owner_routed`, each
    failed in one run and passed in the other and alone. Clippy and the
    Darwin clippy of `yazelix-venus` were clean. Installed proof: that tree's
    `nix build` gave
    `/nix/store/n907id3sg8sramk8sd2bb1qgaqqsvi44-eon-0.1.13`, x86_64 Linux,
    private headless Sway at 1280x720. On an idle window Alt+Shift+Q ended
    the window with no prompt. With p1 running `sleep 600` it showed "Quit
    this window?", "This stops 1 terminal; 1 still running." and
    `Pane  p1  ~/w> ^sleep`, and Escape kept the window. With the Agent
    popup busy too the prompt read "This stops 2 terminals; 2 still
    running.", re-worded to "1 still running" when p1's command ended, and
    Alt+Shift+D detached with the terminals live. After `eon attach`,
    Alt+Shift+Q and Enter ended the window. The session had no notification
    daemon, so whether quitting sends a desktop notification was not
    observed.
- **Open proof:** Columns (`eon-splits-opwv`): runtime topology and EONW v10
  are covered by `columns_hold_accordions_traverse_into_tabs_and_collapse_on_exit`,
  `create_column_in_a_popup_only_tab_never_overfills_a_neighbor`,
  `column_moves_stay_in_their_column_popups_skip_columns_and_the_cli_reports_positions`
  and the v10 codec tests; Venus layout, the create-column mapping and the 40-column
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
  native selected-Orbit crash continuation and the rejected-action notice.
  Cards (`eon-gapped-rounded-cards-txco`, whose commit is the revision): Venus
  `column_backgrounds_cover_the_gaps_between_cards` and the card geometry in
  `columns_sit_side_by_side_and_keep_the_selected_column_primary`,
  `eon_workspace_becomes_one_bounded_native_accordion`,
  `shared_popup_projection_keeps_hidden_work_and_one_input_surface` and
  `each_column_keeps_forty_terminal_columns`; isolated captures of the built
  package on a private headless Sway (1920x1080, scale 1) observed one, two,
  stacked and four columns, hidden hints, a popup and `false`, with painted
  gaps and no corner meeting a window edge, tab pill, label or the footer.
  The bar-colored frame (`eon-tubj`, whose commit is the revision): Venus
  `frame_fills_everything_outside_the_cards` and
  `framed_card_corners_stay_opaque`; the same isolated captures
  sampled the top bar, gaps, outside the corner curves and the footer band at
  `#0a0d14`, the cards darker, and `false` unchanged. The tab inset
  (`eon-gv1r`, whose commit is the revision): Venus
  `popup_outer_chrome_reuses_the_pane_stack_rectangle` at default and small
  typography plus the original tiny-window cases; captures put the first pill
  and first card both 8 pixels from the left edge, the pill 8 from the top and
  8 above the cards.
  Pane close (`eon-close-pane-onwi`): runtime
  `pane_close_targets_the_active_tab_and_prunes_like_an_exit`, the real-binary
  Stop, deduplication and final-pane refusal in
  `delayed_second_cli_receives_committed_workspace_and_controls_three_sessions`,
  `messages_keep_their_wire_bytes`,
  `requests_keep_their_tags_and_reject_malformed_actions`,
  `close_parsers_require_one_stable_identity` and Venus
  `direct_workspace_shortcuts_use_stable_eon_actions`; isolated dogfood of the
  built package on a private headless Sway (1280x720) closed a column's only
  pane, a stacked pane and a non-final tab's only pane with Alt+W, each pruned
  as a natural exit, and refused the final pane with the rejected-action
  notice.
  Sideways pane moves (`eon-sideways-pane-moves-k9ca`): runtime
  `sideways_pane_moves_join_neighbor_stacks_split_at_edges_and_drop_emptied_columns`,
  `pane_moves_refuse_covering_popups_without_changing_tabs` (up/down too since
  `eon-popup-covered-pane-reorder-iw2k`),
  the real-binary `eon pane move right` in
  `delayed_second_cli_receives_committed_workspace_and_controls_three_sessions`,
  `sideways_pane_moves_reject_malformed_requests`,
  `messages_keep_their_wire_bytes`,
  `reorder_parser_accepts_only_each_workspace_axis` and Venus
  `direct_workspace_shortcuts_use_stable_eon_actions` and
  `sideways_moves_open_a_column_only_for_a_stacked_outer_pane`; isolated
  dogfood of the built package on a private headless Sway (1280x720) split
  stacked panes into new right and left columns, left a lone edge pane in
  place, joined a lone pane below the neighbour's expanded pane, showed
  the 40-column notice for a third column, refused a move under the Git popup
  with the rejected-action notice, and moved a tab left and back with
  Ctrl+Alt+Shift+H/L.
  Zoom (`eon-zoom-selected-pane-e2pv`, whose commit is the revision): Venus
  `zoom_names_the_tab_column_count_and_selected_column`, the zoomed scene in
  `columns_sit_side_by_side_and_keep_the_selected_column_primary` and
  `zoomed_tab_names_the_zoom_and_lists_only_its_column`; isolated dogfood of the built package on a private headless Sway
  (1280x720) zoomed the second of two columns, showed the `· zoom` pill, sent
  typed input to the zoomed terminal, kept a counter in the hidden column
  printing (8 before, 24 after), restored both columns on Alt+Shift+Z, and
  ended the zoom on Alt+H.
  Installed AT-SPI labels and broader reader use remain open. These checks do not establish uninterrupted picker rendering,
  fractional-scale, wider compositor or non-systemd coverage.

## EON-C9 — Managed shell environment

- **Status:** Proven
- **Consumer:** People and agents inspecting Eon configuration defaults and
  paths, Eon terminals launched under Nushell or Bash, and every terminal,
  popup, Venus and quota reader the supervisor starts.
- **Trigger:** Eon prints configuration defaults or paths, or constructs a child launch environment.
- **Result:**
  - `eon config defaults` prints a TOML configuration representing this build's
    defaults, with comments explaining defaults that require omission or remain
    Eon-owned. `--json` prints the same configurable values as a nested JSON
    object using the configuration's table and key names. Optional fields whose
    default is omission remain omitted in both formats. Loading the printed
    TOML produces the same configuration policy as an empty configuration. The
    command does not read the user's configuration, prepare directories, contact
    a supervisor or start children. Successful output exits 0 with no stderr
    output; invalid syntax exits 2 and output failure exits 1.
  - `eon config-path` creates the configuration root if it is missing and
    prints the absolute path of the settings file `config.toml` inside it,
    without creating or reading that file. `eon config-dir` creates the root
    the same way and prints its absolute path. On success both exit 0 with one
    line on stdout and no stderr output; other arguments exit 2. A root that
    cannot be resolved or created, or a stdout write that fails for any reason
    but a closed pipe, exits 1 with the reason on stderr; a closed stdout pipe
    still exits 0 with no stderr output. `eon --help` names `config.toml`
    beside `config-path` and `config defaults`.
  - Eon selects exact Nushell, Bash, Starship, Zoxide, fzf, Atuin, Carapace,
    Helix, Yazi, and LazyGit artifacts.
  - Helix that Eon starts loads its Steel configuration files, `helix.scm` and
    `init.scm`, only from Eon's Helix config directory, never from a
    repository's `.helix` directory.
  - Stable `eon-*` commands expose managed shells and tools outside Eon; one
    child-private PATH exposes accepted unprefixed names inside terminals.
  - Configuration accepts one direct argv command and independent Starship,
    Zoxide, Atuin, Carapace, and mise booleans. Defaults are
    `command = ["eon-nu"]` and `true`; each new terminal rereads them without
    rewriting running shells.
  - An Eon build ignores a configuration key it does not know, with one warning
    naming the key and file, so a newer setting never breaks an older running
    window. A known key with a wrong type or value still fails.
  - An unknown name in `[bar] widgets` is ignored with a warning naming the
    value and file; known repeated names and wrong types still fail.
  - When mise is enabled and the inherited PATH has a `mise` executable, an
    interactive managed shell evaluates that host mise's own `mise activate`
    output after Eon's other integrations, so mise-managed tools and
    environment resolve per directory from the first prompt. Nushell runs
    `--execute` startup commands before that prompt. A shell whose own
    configuration activates mise is left alone.
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
  - So that an agent whose shell started Eon does not pass its session on,
    Eon removes exactly these names from the environment of every terminal,
    popup, Venus and quota reader the supervisor starts: `CLAUDECODE`,
    `CLAUDE_PID`, `CLAUDE_EFFORT`, `CLAUDE_CODE_CHILD_SESSION`,
    `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_SESSION_ATTENDED`,
    `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_EXECPATH`,
    `CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`,
    `CLAUDE_CODE_SESSION_ACCESS_TOKEN`, `CLAUDE_SESSION_INGRESS_TOKEN_FILE`,
    `CLAUDE_CODE_REMOTE_SESSION_ID`, `CLAUDE_CODE_REMOTE_SESSION_UUID`,
    `CLAUDE_CODE_CLOUD_SESSION_ID`, `CLAUDE_CODE_BRIDGE_SESSION_ID`,
    `CLAUDE_CODE_HOST_SESSION_ID`, `CLAUDE_CODE_HOST_WORKTREE`,
    `CLAUDE_CODE_HOST_WORKTREE_FENCE`, `AI_AGENT`, `CODEX_SESSION_ID`,
    `CODEX_THREAD_ID`, `CODEX_TOOL_CALL_ID`, `CODEX_VERSION`,
    `CODEX_PERMISSION_PROFILE`, `CODEX_CI`, `OPENCODE`, `OPENCODE_SESSION_ID`,
    `OPENCODE_TERMINAL`, `PI_CODING_AGENT`, `PI_SESSION_ID`, `PI_SESSION_FILE`,
    `PI_REASONING_LEVEL` and `GROK_SESSION_ID`. The list was checked against
    Claude Code 2.1.303, Codex 0.166.0, OpenCode 2.0.32, Pi coding-agent 1.4.0
    and Grok 1.0.62. The same children also lose the generic settings an
    agent's tool runner gives every command it runs, but only a name that
    arrived with exactly the runner's value while that runner's marker also
    arrived as `1`: under `CLAUDECODE`, `GIT_EDITOR=true` and
    `COREPACK_ENABLE_AUTO_PIN=0` (Claude Code 2.1.303); under `CODEX_CI`,
    `NO_COLOR=1`, `PAGER=cat`, `GIT_PAGER=cat` and `GH_PAGER=cat` (Codex
    0.166.0). Under `CODEX_CI`, each of `LANG`, `LC_CTYPE` and `LC_ALL` that
    arrived as `C.UTF-8` takes the desktop session's value instead, read
    once per supervisor through `systemctl --user show-environment`, whose
    output Eon drains up to 4 MiB and whose process group it kills and reaps
    after one second at most; without one, `LC_CTYPE` and `LC_ALL` are
    removed and `LANG` stays `C.UTF-8` so the locale remains UTF-8. Any other
    value, or the same value without the marker, passes through. Eon cannot
    tell a person's own identical value from the runner's: a person who sets
    one of these on purpose, such as `PAGER=cat`, and starts Eon from such an
    agent shell loses it in command panes, popups and the Editor unless their
    shell configuration or the command itself sets it. Every other inherited
    name, agent settings such as `CLAUDE_CONFIG_DIR`,
    `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE` and API keys included, keeps the
    treatment described above.
- **Important failures:** Invalid or unreadable configuration, empty command,
  missing artifact, manifest mismatch, or shell launch fails explicitly without
  a replacement terminal. Optional integration failure warns without preventing
  shell launch. A missing or failing host mise is skipped silently; mise reports
  its own per-project errors, such as an untrusted configuration. Unavailable
  native clipboard access reports Nushell's own error. A configuration root
  whose path is not valid UTF-8 fails Helix launch, since Helix cannot read
  its Steel configuration directory there.
  Eon never mutates global configuration, startup files, aliases, or PATH.
- **Owner:** Eon supplies defaults, component selection and Nix shell adapters;
  Eon Runtime owns strict configuration and private launch-environment
  enforcement. Shells/tools retain native configuration and behavior.
  Eon supplies product defaults; Runtime prints them through the configuration
  model used by its resolver. Documentation examples are generated from, or
  checked against, that output.
- **Boundary:** No global aliases, user-file mutation, prompt/completion schema,
  shell framework, automatic Direnv, mise installation or trust management,
  history/sync policy, plugin surface,
  editor replacement, or remote/headless clipboard bridge. Apart from the
  agent-session and tool-runner removal above, arbitrary argv remains
  unmanaged and `eon run -- COMMAND...` remains the explicit escape hatch.
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
  - **Mise:** `eon-nf7g` adds to `managed-command-path` a fake host mise that
    both shells activate, and that each skips when it is missing, fails, or
    `mise = false`; Nushell's pre_prompt hook and its condition are evaluated
    outside a REPL. The bead records an installed observation of a real mise
    in an isolated Eon terminal.
  - **Agent session:** `eon-geigu` adds
    `children_drop_the_launching_agent_session_and_keep_agent_settings`
    (every removed name, and kept settings and markers),
    `orbit_uses_configured_argv_only_for_default_sessions` (default shells and
    explicit commands), `venus_receives_workspace_endpoint_only_in_workspace_mode`
    (both Venus modes) and `quota_reads_carry_no_terminal_or_agent_session`.
    Installed observation, x86_64 Linux, private headless Sway in an isolated
    root, twice: a supervisor started with ten removed names and three agent
    settings in its environment; Venus, the default Nushell terminal, a CLI
    pane, the Files and Commands popups and both quota readers carry none of
    the removed names and all three settings with their values.
  - **Tool runner:** `eon-yxo2y` adds
    `children_drop_an_agents_tool_settings_only_under_its_marker` (each
    runner's settings under its own marker alone and under both; a desktop
    `LANG=pt_BR.UTF-8` replacing Codex's; `LANG` kept UTF-8 without a desktop
    session; nothing changed without the marker or with a person's other
    value; `EDITOR` and `LC_TIME` kept),
    `terminals_drop_the_launching_agents_tool_settings` (a real supervisor
    started with both markers and their values, beside a stand-in
    `systemctl` whose desktop session has `LANG=pt_BR.UTF-8`, gives its first
    terminal none of them, that `LANG` and the person's `EDITOR`),
    `the_user_manager_read_drains_its_output_and_leaves_nothing_by_its_deadline`
    (a `systemctl` that never answers is reaped and a helper it started is
    gone by the deadline; over 1 MiB of output still yields its `LANG`;
    output past 4 MiB, endless, failing and missing ones give no locale), and
    the `CLAUDE_CODE_HOST_WORKTREE` pair to
    `children_drop_the_launching_agent_session_and_keep_agent_settings`.
  - **Config path:** `eon-wt5v8` and `eon-hb4zp` add
    `config_path_prints_the_settings_file_and_config_dir_the_root`
    (`config-path` and `config-dir` output, root created, file not created,
    `--file` and other arguments exit 2), run both commands in
    `closed_stdout_pipe_is_not_a_panic` (exit 0, no stderr) and have
    `help_is_readable_and_does_not_start_or_prepare_a_workspace` check that
    the `config-path` and `config defaults` help entries name `config.toml`;
    `relative_configuration_root_is_resolved_once` covers absolute output for
    both commands.
  - **Steel trust:** `eon-2yj90` adds to `managed-command-path`, on Linux, the
    real `hx` on Eon's managed PATH, under a pseudo-terminal in a repository
    that holds `.helix/helix.scm` and `.helix/init.scm`: the `init.scm` in the
    configuration root's `helix` directory runs and the repository's does
    not. `managed_commands_use_exact_programs_and_private_configuration`
    checks that Helix gets `HELIX_STEEL_CONFIG`, and
    `helix_does_not_start_under_a_configuration_root_that_is_not_utf8` that
    such a root fails Helix launch.
  - **Note:** `eon-8olu` removed the managed Zsh and Fish shells. The
    `shell-environment` check went with them; its Zsh and Fish evidence above
    no longer applies. The remaining checks cover Nushell and Bash launch
    and versions.
- **Open proof:** Pending re-proof in this repository: interactive managed
  startup of each shell, and a native Wayland clipboard round trip through
  `clip copy`, the aliases and `wl-paste`. Agent session (`eon-geigu`): a
  Claude Code started in such a terminal shows no transcript-saving warning.
  Steel trust (`eon-2yj90`): no real Helix run on macOS.

## EON-C10 — Typed workspace control

- **Status:** Proven. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-ciwn.1`: EON-C39's planned event subscription is the
  one connection that carries more than one response. Amended on 2026-11-03
  (approved by Lucca, relayed by eon-overseer) by
  `eon-quick-search-parity-23y9.3`: the Files quick-search keys, and
  `eon popup hide`. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-quick-search-parity-23y9.2`: Project and Files show
  the tab's pair (EON-C18), and `eon tab directory` on a pending tab is a
  pick. Amended on 2026-11-04 (approved by Lucca, relayed by eon-overseer) by
  `eon-knowp`: a stopped transient popup's tab shows again the popup it
  covered. Amended on 2026-11-04 (approved by eon-overseer under Lucca's
  standing rule) by `eon-vhhon`: a Toggle of the selected `eon-workspaces`
  popup presses Enter in it.
- **Consumer:** One local CLI or approved composition controlling a live Eon
  supervisor.
- **Trigger:** The client submits one versioned workspace or supervisor-lifecycle
  action.
- **Result:**
  - Eon returns one complete typed result from the sole live workspace owner;
    human-readable CLI output and deterministic structured output project the
    same result.
  - `eon pane rename PANE NAME [--json]` pins a name; `eon pane rename PANE --clear [--json]` restores automatic naming. `--` permits an option-looking NAME. Names are trimmed; an empty result clears the pin. Invalid names fail without mutation. Rename addresses an explicit pane anywhere in the addressed workspace and changes no workspace, tab, pane or popup selection or desktop focus. Window rename and clear actions use the same mutation. Pins follow pane moves, survive presentation detach and reattach, and end with the pane or supervisor; recovery does not restore them. Workspace inspection and action replies expose each pane's `label` and nullable `pinned_name`, independently of attention records.
  - Workspace results carry ordered tab, pane, and terminal identities, active
    and selected identities, liveness, exact raw tab launch-directory bytes,
    and exact opaque Orbit endpoint bytes. Lifecycle results carry generation,
    component/protocol identity, live terminals, busy terminals (EON-C11), the
    window's name and supervisor start time (EON-C23), and owner-authored
    attach/stop availability.
  - CLI JSON emits each directory and endpoint as a JSON string when its bytes
    are valid UTF-8, and otherwise as an ordered byte array, without changing
    EONW. Each connection carries one length-delimited request
    and response, except an EON-C39 subscription, which carries one request
    and then a sequence of length-delimited messages; no connection uses
    EOF as framing.
  - The shared 2 MiB frame ceiling admits every snapshot valid at EONW's field
    and workspace-count bounds.
  - Harmless navigation no-ops return the unchanged Snapshot as success
    (EON-C31's empty attention queue is a rejection instead);
    request deduplication and picker/lifecycle restrictions still apply.
  - `eon` and `eonbase` exit 0 on success, 1 when a command is refused or
    fails, 2 for invalid command lines and argument syntax (including
    `invalid-tab` and `malformed-action`), 3 when a named tab, pane, terminal,
    popup, generation or window is missing or stale, 4 when the supervisor is
    missing or unreachable, and 130 on interrupt, identically with and without
    `--json`. The CLI projection derives the status from the failure code.
    Multi-target Stop exits with its largest status; commands that run another
    program pass on its status.
  - A CLI confirmation (`eon stop`, `eon quit`, `eon window stop`,
    `eon workspace close`, and the final close of `eon tab close` and
    `eon pane close`) proceeds only
    on a yes answer (`y`, `Y`, `yes` or `YES`). Any other answer changes
    nothing. At a terminal it prints the cancel note and exits 0. When stdin
    is not a terminal, it exits 1 with `not-confirmed`, end of input included,
    and a piped yes still confirms. `--json` never asks. The final close asks
    only at a terminal and otherwise closes.
  - Every Eon terminal's environment carries `EON_TERMINAL`, an opaque value
    naming that terminal's exact Orbit run; the supervisor's other children
    never inherit one. For workspace control, `EON_TERMINAL` selects the live supervisor and workspace that own that
    exact terminal, independently of the invoked CLI build's current generation. Within that workspace,
    `pane create`, `edit`, `pane close PANE`, `pane move` and `tab move` retain their calling-tab,
    column and pane rules: creation uses the caller's column, moves act on its pane or tab, and pane
    close resolves its target in the caller's tab. A popup terminal acts for its tab and that tab's selected column and
    pane. Within that tab each action behaves as it does for the active tab,
    so `pane create` selects the new pane and dismisses the tab's popup;
    `pane close PANE` also works while a popup covers the caller's tab and
    leaves that popup shown (EON-C8); the active tab and desktop focus never
    change. Other workspace-scoped commands use the caller's workspace and retain their existing per-tab
    rules. Without a calling terminal or explicit target, workspace control uses the invoked build's
    current generation in the selected runtime namespace and its shown workspace. Launch, lifecycle and
    local information commands retain their existing targeting. Every popup terminal's environment
    also carries `EON_TAB`, the `tN` identity of the tab that owned it when it
    started.
  - Workspace commands accept `--generation GENERATION` and `--window ID` before any `--` operand
    separator. `--window` selects an independent window's runtime namespace; otherwise the existing
    runtime namespace applies. `--generation` selects an exact generation there. With only `--window`,
    selection follows `window attach`'s current-or-sole-live-generation rule. If `EON_TERMINAL` is set,
    the selected supervisor must validate that caller; explicit targeting does not waive caller
    validation. Workspace CLI results name the described `generation` and `window`, with `window` null
    for the default namespace, and identify the projected workspace through shn4.3's workspace
    discriminator.
  - `eon workspace` run from an Eon terminal also names that calling terminal. In JSON, `caller`
    holds its `terminal` (`session-N`), its `tab`, and its `pane`, or its `popup` with `pane` null.
    The human output prints a matching `caller` line. Without `EON_TERMINAL`, JSON `caller` is null
    and the line is absent. `EON_TERMINAL` stays opaque; `caller` is the supported way to find the
    calling pane.
  - Every Eon terminal's PATH names that terminal's own Eon build as `eon`,
    ahead of any installed one, so `eon` run by name there acts on that
    terminal's own window and generation after the installed profile changes.
    An `eon` run by path, or put first by a shell or script, is that build's.
  - `eon edit [--json] -- FILE` opens FILE in the configured editor in a new
    pane, as `pane create` would for the same caller, with the tab directory
    as its working directory: the pane runs `[editor] command` with FILE's
    absolute path appended as its last argument. A relative FILE resolves
    against the caller's working directory. The editor's argv is runtime
    policy and does not cross EONW.
  - `eon pane create [--json] -- COMMAND [ARGS...]`, run from an Eon terminal,
    opens a new pane exactly where `pane create` would for that caller, with
    the tab directory as its working directory, running COMMAND and its
    arguments as an exact argv with no shell: at most 256 arguments of up to
    32 KiB each, within the shared frame ceiling. A bare COMMAND resolves
    through the terminal PATH and a relative one against the tab directory, as
    popup executables do. When COMMAND exits, the pane ends like any ended
    terminal. This and `tab create --directory DIRECTORY -- COMMAND` are the
    only requests that carry a command over EONW, and the runtime accepts them
    only from a caller naming a live terminal of the addressed supervisor.
    That check states intent, not security: the socket is private to the user,
    whose processes can already run any command and read terminal identities.
  - `eon pane create --no-focus [--json]`, with or without `-- COMMAND`, adds
    the same pane in the same place but takes no focus: the tab's selected
    column and pane stay as they were, a shown popup stays shown, and the new
    pane joins the end of its column collapsed. In a tab without panes it
    becomes the tab's only, selected pane, and a shown popup still stays shown.
    Without `--no-focus`, `pane create` selects the new pane and dismisses the
    tab's popup, as above.
  - `eon tab create [--json] [--no-focus] --directory DIRECTORY [-- COMMAND [ARGS...]]`
    adds a new tab to the addressed workspace with DIRECTORY as its directory
    and no picker, and starts its one pane there. That pane is the default pane
    a picker's accepted choice would start or, with `-- COMMAND`, COMMAND run as
    `pane create -- COMMAND` runs it: exact argv, no shell, the same bounds, a
    relative COMMAND resolved against DIRECTORY, and accepted only from a
    calling terminal. A relative DIRECTORY resolves against the caller's working
    directory. Without `--no-focus` the new tab becomes the active tab, as a
    bare `tab create` does. With `--no-focus`, the active tab, every tab's
    selected pane, any shown popup and desktop focus stay as they are. The
    reply's tab list ends with the new tab. An unavailable DIRECTORY, the tab
    limit, or a command that cannot start refuses the command with exit 1 and
    changes nothing. `--no-focus` and `-- COMMAND` require `--directory`.
  - `eon tab directory TAB [--json] -- DIRECTORY` on a pending tab is a pick:
    it commits exactly as a pair pick does (EON-C18).
  - `eon popup hide POPUP [--json]` hides popup `uN` of the addressed
    workspace, as its owning window's dismissal would:

    - a keep-alive popup is hidden and keeps running;
    - a transient popup stops;
    - dismissing a pending tab's pair starts its first pane (EON-C18).

    The CLI reads the owning tab from workspace inspection, then sends EONW
    `DismissPopup`. It changes no other tab, pane, popup or desktop focus,
    except that a stopped transient popup's tab shows again the popup it
    covered, as the window's own dismissal does (EON-C33). A POPUP that the
    addressed workspace does not list fails with
    `unknown-popup`; one that leaves between the read and the dismissal fails
    with `stale-popup`. Both exit 3 and change nothing.
  - `eon tab close TAB` and `eon pane close PANE` may name the final tab or
    its last pane, which ends the window (EON-C8), or, while other workspaces
    remain, removes that workspace (EON-C33). Before either final close, when
    `[workspace] warning_close_last_tab` is true, its default, and stdin is a
    terminal, the CLI first lists the terminals that will stop and asks, as
    `eon stop` does. While other workspaces remain, it words the question as
    EON-C33 does, `Close the last tab? This closes workspace NAME and stops N
    terminals [...]. [y/N]`, and a cancel prints `cancelled; workspace NAME
    stays open`. `--json` never asks, and neither does the CLI when the
    setting is false. Run from a terminal, a close that cannot first inspect
    the workspace fails without closing. A close that ends the window prints
    that the generation ended, or with `--json`
    `{"ended":{"generation":...}}`, and exits 0. The caller's own
    terminal may end with it, as with Stop. Eon rejects a non-boolean
    `warning_close_last_tab` with the rest of `config.toml`.
  - CLI JSON carries each pane's and popup's full EON-C31 attention record, the window's
    EON-C31 attention queue in order, and the window's Codex and Claude quota readings
    (EON-C27), as EON-C31's CLI JSON rule lists them. It also carries the window's
    running-windows summary (EON-C23) as `running`, with `this`, `others` and
    `omitted`; each window holds `window`, `generation`, `name`, `started_at` and
    `live_terminals`, and each of `others` also `state` (`live`, `incompatible` or
    `unreachable`). The human projection prints one `running` line per window.
    It also carries the window's busy terminals (EON-C11) as `busy`, with
    `terminals` and `omitted`; each terminal holds `workspace`, `tab`, `pane`
    or `popup` (the other null), `terminal` and `name`. The human projection
    prints one `busy` line per terminal.
    The human projection carries each terminal's attention state, `app` and
    `name` only, and no queue or quotas.
  - **Several workspaces in one window** (EON-C33):
    - **Status:** The workspace requests and targeting below are implemented
      on EONW v20, with their CLI front end
      (`eon-workspace-switching-shn4.5`), the Workspaces picker and the
      Commands entries (`eon-workspace-switching-shn4.6`).
    - **Listing.** `eon workspaces [--json]` lists this window's workspaces in
      order, each with `id`, `name` (null when unnamed), `shown`, `tabs`
      (the tab count), `attention` (EON-C33's rollup, spelled as
      `attention_rollup` is, null when none; the text form shows
      `attention=none`) and `recency` (EON-C33's rank, 0 for the shown
      workspace, which orders the picker), after the `generation`, `window` and caller's
      `workspace` that every result names. This window means the caller's
      window inside Eon, and otherwise the window `eon workspace` targets
      (`attention` and `recency` since `eon-workspace-switching-shn4.8`, on
      EONW v22).
    - **New.** `eon workspace new [--name NAME] [--no-focus]` and Commands >
      New workspace add a workspace after the shown one. It opens with one
      pending tab and a fresh directory picker, as a new window does (EON-C23),
      and becomes the shown workspace unless `--no-focus` is given, as for
      `pane create`. The reply names the new `wN` in `created`, whether or
      not it is shown, alongside the caller's projected `workspace`.
    - **Switch.** `eon workspace switch WS` shows WS. Switching to the shown
      workspace returns the unchanged snapshot (EON-C10). Each workspace keeps
      its active tab, selected pane and popups across switches; zoom ends, as on
      any tab change (EON-C8).
    - **Rename.** `eon workspace rename [WS] NAME` and Commands > Rename
      workspace set the name; an empty NAME clears it. Without WS it renames the
      caller's workspace. The CLI trims surrounding whitespace from NAME and
      from `--name`, as for pane names, so a blank NAME clears the name and a
      blank `--name` is malformed (exit 2).
    - **Close.** `eon workspace close WS` first asks for confirmation, as
      `eon stop` does. The prompt names the `wN` and name that WS resolves
      to, says that closing stops every terminal in it, and that it quits the
      window when it is the last workspace. A WS that resolves to no workspace
      or several fails before asking, with the failures below. Any answer but
      yes changes nothing. After a yes, the CLI checks again that the `wN` is
      live and otherwise changes nothing and exits 3; the check and the close
      are not atomic, so a workspace closed by another client in between
      leaves Runtime to resolve that `wN` as any WS, which may match another
      workspace named that `wN`. `--json` asks nothing. Closing
      then stops every terminal of that workspace, as closing each of its
      tabs does (EON-C8), and removes it. Closing the last workspace is the
      window's final close (EON-C8).
      After a successful close removes the caller's workspace, the reply
      describes the shown remaining workspace; closing the last workspace
      returns Ended.
    - **Targeting.** Run from an Eon terminal, every workspace-scoped command
      acts in that terminal's workspace, found through `EON_TERMINAL`, and
      EON-C10's per-tab rules apply inside it. Run outside Eon, commands act on
      the shown workspace. WS resolves an exact `wN` first, then an exact name.
    - **Important failures:** These leave every workspace unchanged and return
      EON-C10's typed failure: an unknown or stale `wN` or a name that matches
      nothing (exit 3); a name that matches more than one workspace
      (`ambiguous-workspace`, exit 1, naming the matching IDs); a 33rd workspace
      (`workspace-limit`, exit 1); a malformed name (exit 2); and a tab, pane or
      popup identity from another workspace (exit 3), except a `FocusAttention`
      target (EON-C31). A request ID repeated
      within the last 256 accepted mutations never repeats a switch, creation
      or close. A failed stop while closing a
      workspace keeps the workspace and its remaining terminals, as a partial tab
      close does (EON-C8). Switching never moves desktop focus.

- **Important failures:** A caller that the addressed supervisor rejects as
  naming no live terminal (ended, closed, from another generation or window,
  empty, or malformed) changes nothing and exits 3 (`unknown-caller`), with no
  fallback to the active tab. Absence of this CLI build's supervisor does not
  prevent control of the caller's compatible supervisor. Caller resolution uses
  the complete opaque terminal identity and never falls back to another
  workspace. A malformed caller exits 3; a stale caller exits 3 when discovery
  completes without validating it. When no owner can be found because discovery
  cannot inspect a supervisor, it exits 4 (`supervisor-unavailable`). Failed or
  ambiguous resolution changes nothing. An unavailable or incompatible selected
  supervisor fails explicitly without starting, presenting or restarting it. In
  a tab without panes, a popup caller's `pane move` refuses, and so does its
  `pane create` without `--no-focus` when dismissing that popup would end the
  tab's last terminal; both change nothing and exit 1. While a popup covers the
  caller's tab, `pane close` naming that tab's last pane changes nothing and
  exits 1, and so does a close sent without a calling terminal, including
  Alt+W. An unavailable editor changes nothing and exits 1. An empty COMMAND is
  a usage error (exit 2); a command pane requested without a calling terminal,
  or whose program is missing or not executable, changes nothing and exits 1.
  Malformed, oversized, incompatible, unavailable, rejected, timed-out,
  partially read, or response-lost operations fail within the shared bound
  without reconstructing hidden state or reviving completed Stop. Accepted Stop
  names affected live terminals, sends canonical Stop through every validated
  management lease before waiting, and returns only after every exact terminal
  record is complete.
- **Owner:** Eon Runtime supervisor workspace/action state and CLI projection;
  its canonical EONW package owns types, codec and result ordering. Eon supplies
  validated component facts and product defaults.
- **Consumes:** Current EONW v31 and accepted Orbit management operations.
- **Boundary:** Additive lifecycle tags do not widen the pinned Venus workspace
  consumer. There is no event stream or subscription policy other than
  EON-C39's event subscription, and no remote transport,
  plugin/MCP API, authorization framework, durable restoration, terminal
  content (except a program's own EON-C31 status report text and terminal
  title, EON-C35's read of visible text, title and working directory, and
  EON-C36's input), or direct
  child-protocol escape hatch.
- **Package owner:** The canonical EONW crate lives in
  `runtime/crates/eon-workspace-protocol` with its accepted package/API and
  v31 bytes. Venus, Eon Runtime and Eon consume it by path.
  See [package selection](ARCHITECTURE.md#in-tree-package-selection).
- **Runtime policy:** Project is required and defaults to Alt+Z; it has no
  command of its own and shows the tab's pair (EON-C18) on quick search. Git
  defaults to `eon-lazygit` on Alt+Shift+J;
  Agent defaults to the first available provider in the accepted Nova order on
  Alt+Shift+L. Git and Agent remain live when hidden. Reopening at the same tab
  directory preserves identity. When a tab's directory changes explicitly
  (a pair pick or `SetTabDirectory`), a live Agent terminal
  launched at another directory is parked in the same step: it leaves the popup
  and becomes an ordinary pane at the end of the tab's selected column, or the
  first column of a tab without panes, keeping its Orbit terminal, process and
  directory. It is the selected, expanded pane when its popup was shown or the
  tab had no panes; otherwise the tab's selection is unchanged. Parking starts
  and stops nothing and cannot fail on terminal or column capacity. The next
  Agent invocation starts a fresh agent at the new directory. Files defaults to
  Alt+Shift+Y and runs the pinned Yazi at the tab directory with Eon's packaged
  popup configuration. Files is the tab's pair (EON-C18), whose key table,
  footers and Zoxide rules apply.

  Files remains live when hidden and keeps its process, location and cursor
  across tab-directory changes; only after its process exits does the next
  invocation start a fresh instance at the tab directory.
  In Files, Yazi's edit opener runs `eon edit`, so Enter on a text file opens
  it in a new pane running the configured editor and hides the popup. Enter
  on a folder enters it, and other file types keep Yazi's default openers. For
  Git and other keep-alive popups except Files, retargeting changes no process
  until the next invocation, which stops the exact old terminal before starting
  a fresh one at the new directory. A stop failure preserves the old instance;
  a start failure records no replacement.
  Anima defaults to Alt+Shift+A and launches the pinned random native animation
  as a transient popup. A dismissal, replacement, or tab close stops its Orbit
  terminal, restores the prior tab focus, and leaves no hidden animation running.
  Commands defaults to Alt+Shift+M and runs a transient fzf list of existing
  `eon` commands: tab, pane and window creation, closing or moving the popup's
  own tab, quitting this window, switching, creating, renaming or closing a
  workspace (EON-C33), the workspace, window and generation
  listings, `stop previous`, `versions`, `config-path` and `--help`. Each line
  names the command it runs. Enter runs it in the popup's terminal, with that
  terminal's `EON_TERMINAL` and `EON_TAB`: pane creation and tab moves act for
  the popup's tab under the caller rules above, closing names `EON_TAB`, and
  every command keeps the CLI's own validation, targets and Stop and
  final-close confirmations; it adds no EONW action. A successful action
  ends the popup, except a close from the workspace list, which shows the
  list again; a listing, `stop previous`, or a failure reported while the
  popup is still open, waits for Enter. "Quit this window" runs `eon quit`
  (EON-C11) as the popup's terminal, so it stops only the popup's own window,
  in an independent window too (EON-C23), with `eon quit`'s confirmation; it
  adds no EONW action. New pane dismisses the popup before it
  starts the pane, as Alt+M does, so a failure after that dismissal is not
  shown. Esc, Ctrl+C or no match runs nothing. The Commands popup follows the
  popup catalog's key precedence below.
  Workspaces defaults to Alt+Shift+S and runs `eon-workspaces`, a transient
  fzf list of the popup's window's workspaces (EON-C33's picker); Commands >
  Switch workspace runs the same list in the Commands popup. Each row shows
  the workspace's name, or its `wN` when unnamed, its tab count, and
  ` · needs input` when its EONW v31 `attention` is needs input. Rows run by
  EONW v31 `recency`, most recently shown first, with the shown workspace last
  and marked ` · shown`, and the query matches only that name or `wN`. Enter on the highlighted row,
  including on an empty query, runs `eon workspace switch wN` once the CLI
  has checked again that the `wN` is live; otherwise it changes nothing and
  exits 3, and the check and the switch are not atomic. Enter on a non-blank
  query that matches no row runs `eon workspace new --name QUERY`. Both run in
  the popup's terminal as for Commands, so the CLI keeps its validation,
  targets and failures; a failure waits for Enter. Ctrl+X on the highlighted
  row runs `eon workspace close wN` in the popup's terminal once the CLI has
  checked again that the `wN` is live (otherwise it changes nothing and exits
  3), with its confirmation, and then shows the list again unless that close
  stopped the popup. Esc or Ctrl+C runs nothing.
  While a popup whose command is `["eon-workspaces"]` is its tab's selected
  popup and was launched in its tab's current directory, a Toggle invocation
  of its entry dispatches as Focus, and once accepted the supervisor sends
  Enter to that popup's terminal as one key press over its lease, whatever it
  shows (at a close confirmation with nothing typed Enter answers no; a typed
  answer is submitted). This is the person's key, not EON-C36 input, so
  `[agents] terminal_access` does not apply. A failed send is logged and the
  popup stays shown; input whose send timed out may still arrive. A Focus
  invocation, an entry with another command, a picker that the invocation
  relaunches in its tab's new directory, and `eon popup hide` keep their
  meaning.
  Commands > New workspace runs `eon workspace new`, Rename workspace reads a
  name and runs `eon workspace rename` for the popup's workspace (an empty
  name clears it), and Close this workspace runs `eon workspace close` for the
  popup's own `wN`, keeping its confirmation. None adds an EONW action.
- **Configuration:** Eon fixes popup margins at 8 side and 4 vertical logical
  pixels; a `[popup]` table is an unknown key and warns as described in EON-C9.
  `[popups.<id>]` owns direct argv, physical shortcut,
  label, enabled state and keep-alive lifetime. A configured `keybinding`,
  every custom popup's and a built-in's that the user set, takes its key
  first. A built-in popup's default key applies only while no configured popup
  holds it; otherwise the built-in stays in the catalog without a key,
  overridden by that popup. `keybinding = ""` leaves any popup, Project
  included, in the catalog without a key. Only Agent accepts `"auto"`.
  Eon ignores unknown fields with the warning described in EON-C9. It rejects
  malformed shortcuts, reserved shortcuts, two configured popups on one
  shortcut, unbounded values and unavailable executables before mutating
  workspace state; a configured popup on Alt+H, Alt+J, Alt+K or Alt+L instead
  loads without a key (EON-C20). It does
  not install providers, cache selection, interpret shell strings, or expose
  commands over EONW beyond a calling terminal's `pane create -- COMMAND` and
  `tab create --directory DIRECTORY -- COMMAND`. Bare executables resolve
  through the terminal PATH; relative executable paths resolve against the
  owning tab's launch directory.
  `[editor] command` is a direct argv, by default `["hx"]`, the packaged
  Helix, with the bounds of a popup command; each `eon edit` rereads it. An
  editor executable that is unavailable on the terminal PATH refuses the
  edit with `edit-command` and opens no pane; Eon never falls back to Helix.
- **Target safety:** Invocation names the tab, entry and expected instance
  (absence means no instance exists). Focus-only and toggle intents are
  distinct. Dismissal requires the exact live instance;
  a replaced instance cannot inherit an old request. Instance identities must
  not be reused after supervisor recovery. The runtime owner validates targets
  against its current Snapshot before applying lifecycle policy; decoding checks
  syntax, not staleness. `SetTabDirectory` is a pick on a pending tab and
  explicit retargeting on an established one.
- **Bounds/failures:** Preserve the 2 MiB envelope, 64 tabs, raw Unix directory
  and endpoint bytes. At most 32 enabled entries and 256 combined pane/popup
  terminals are represented. Reject malformed framing, invalid or aliased
  identities/endpoints, broken selections, duplicate entry/chord assignments,
  invalid geometry and stale popup targets. No terminal contents cross EONW,
  except a program's own status report text (`app`, `title`, `msg`) and its
  terminal title (`name`), which are not screen content: Orbit has validated,
  sanitized and bounded them under ORB-C15; and EON-C35's read of visible
  text, title and working directory and EON-C36's input, which cross only in
  the replies and requests those contracts define.
- **Popup snapshot:** The catalog carries enabled entries, each with its
  normalized physical shortcut or, without one, whether the configuration
  unbound it or another entry holds its default shortcut, and logical-pixel
  margins; per-tab popup terminals/selection and
  pending-tab state are explicit. Popup-only tabs are valid. Popup commands,
  provider selection, cwd and lifetime remain runtime policy and never cross EONW;
  the EON-C31 `app` that crosses is the program's own report, not Eon's
  provider selection. The
  retired v4 picker opcode and v30's `CommitDirectory` are unavailable;
  Project invokes the tab's pair.
  There is no negotiation or adapter.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`;
  exit statuses `eon-8mnr.1` and `eon-8mnr.3` (automated tier); calling-terminal
  targets `eon-8mnr.2` (automated and installed tiers); Agent parking on
  retarget `eon-tbn1` (automated and installed tiers); the Files popup
  `eon-ecne.1` (automated and installed tiers); `EON_TAB` and Files Alt+Enter
  `eon-ecne.4` (automated and installed tiers); `eon edit` and Files Enter
  `eon-ecne.2` (automated and installed tiers); the configured editor
  (`[editor] command`) for `eon edit` and Files Enter `eon-ecupy` (automated
  and installed tiers); `pane create -- COMMAND`
  `eon-n6ga` (automated and installed tiers); `pane create --no-focus`
  `eon-zhjj` (automated and installed tiers); UTF-8 paths as JSON strings
  `eon-readable-windows-and-child-notices-jf6f` (automated and installed
  tiers); final-tab `tab close` and `pane close`
  `eon-prove-close-and-detach-batch-bf8t` (installed tier); EONW v19 attention
  fields `eon-agent-sidebar-yx8.9` (installed tier); `eon workspace` `caller`
  `eon-ed9d.1` (automated and installed tiers).
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** EONW codecs (`request_actions_have_stable_v2_wire_bytes`,
    `round_trips_v2_bounds_and_rejects_invalid_inputs`,
    `common_actions_stay_exact_and_new_requests_are_bounded`,
    `popup_targets_round_trip_and_do_not_follow_replacements`,
    `snapshots_preserve_hidden_and_inactive_work_and_reject_broken_ownership`,
    `every_legal_snapshot_fits_the_frame_budget`); popup policy
    (`popup_configuration_owns_defaults_overrides_and_collisions`,
    `popup_executable_preflight_uses_the_session_context`,
    `retarget_parks_the_agent_and_restarts_git_only_on_its_next_invocation`,
    `chooser_retarget_parks_a_hidden_agent_as_the_first_pane_of_a_paneless_tab`,
    `files_keeps_its_terminal_across_retargets`,
    `files_edits_a_file_in_a_new_helix_pane_of_its_tab`,
    `a_command_pane_runs_its_exact_argv_only_for_a_calling_terminal`,
    `orbit_uses_configured_argv_only_for_default_sessions`,
    `chooser_result_requires_its_exact_live_invocation`); deadlines and
    failures (`unix_connection_attempt_obeys_timeout`,
    `management_response_obeys_one_deadline_across_partial_reads`,
    `lost_stop_response_still_finishes_the_supervisor`,
    `failure_kinds_exit_with_distinct_statuses_with_and_without_json`,
    `invalid_focus_is_rejected_before_supervisor_connection`,
    `closed_stdout_pipe_is_not_a_panic`); calling terminals
    (`messages_keep_their_wire_bytes`,
    `callers_act_in_their_own_tab_without_changing_the_active_tab`,
    `a_transient_popup_caller_never_removes_its_paneless_tab`,
    `commands_from_a_terminal_target_its_column_and_stale_callers_change_nothing`,
    `edit_from_files_runs_the_configured_editor_in_its_tab_and_hides_files`,
    `edit_parser_sends_one_absolute_opaque_file`,
    `command_pane_parser_passes_the_argv_through`,
    `command_panes_bound_their_argv`,
    `unfocused_panes_keep_the_selection_and_the_shown_popup`,
    `terminal_session_inverts_the_terminal_identity`,
    `caller_output_finds_a_popup_and_omits_an_absent_terminal`); CLI JSON paths
    (`json_paths_are_strings_unless_their_bytes_are_not_utf8`,
    `launch_directory_can_disappear_after_the_initial_session_starts`).
    Installed checklist items 3 and 4 (`eon workspace` against the window;
    Project, Git, Agent and Anima popups). Installed `eon-8mnr.2` observation on
    isolated headless Sway: from a terminal in an inactive tab, `eon pane create`
    adds its pane there while the new tab stays active, and a stale
    `EON_TERMINAL` exits 3 with the workspace unchanged. Installed `eon-tbn1`
    observation on isolated headless Sway with a stub Agent provider: a shown
    Agent retargeted by `eon tab directory` became the selected, expanded pane
    with the same process, transcript and input; the next Alt+Shift+L started a
    fresh agent at the new directory; hidden and retargeted through Alt+Z, that
    agent parked collapsed at the column's end with the selection unchanged,
    and both agents stayed live. Installed `eon-ecne.1` observation on isolated
    headless Sway: Alt+Shift+Y opened Files at the tab directory with the
    search and Spot footer, its Yazi reading only the packaged configuration;
    Tab and Shift+Z searches moved Yazi; after `eon tab directory` the same
    Yazi process kept its location; `d` trashed a file; and after `q` the next
    Alt+Shift+Y opened a fresh Yazi at the new directory. Installed `eon-ecne.4`
    observation on isolated headless Sway: Alt+Enter on a hovered folder and on
    a search result set tab t1's directory without moving Yazi; Alt+Enter on a
    search with no match changed nothing quietly; after a supervisor kill and
    recovery, Alt+Enter in the recovered Files Yazi refused and changed
    nothing. Installed `eon-ecne.2` observation on isolated headless Sway:
    in Files, Enter on a folder entered it; Enter on a text file opened a new
    selected pane running the packaged Helix on that file at the tab
    directory and hid Files, which reopened at the same place. Installed
    `eon-ecupy` observation on isolated headless Sway, with `[editor] command
    = ["<root>/my-editor", "--wait"]` naming a recording stand-in editor: in
    Files, Enter on a text file opened a new selected pane p2 running
    `my-editor --wait <absolute path>` at the tab directory and hid Files,
    which stayed in the tab. Installed
    `eon-n6ga` observation on isolated headless Sway: `eon pane create --json
    -- sh -c ...` without a calling terminal exited 1 with `caller-required`;
    as the first pane's `EON_TERMINAL` it added selected pane p2 in that
    column, whose Orbit child ran exactly `sh -c <script>` at the tab
    directory and showed the script's output. Installed `eon-zhjj`
    observation on isolated headless Sway with a stub Agent provider: as the
    shown Agent popup's `EON_TERMINAL`, `eon pane create --no-focus --json --
    sh -c ...` exited 0 and added p2 collapsed at the end of p1's column, with
    p1 still selected and the Agent popup still shown; its Orbit child ran
    exactly `sh -c <script>` at the tab directory. The same caller's default
    `eon pane create --json` then selected the new p3 and dismissed the
    popup. Installed `eon-readable-windows-and-child-notices-jf6f`
    observation on isolated headless Sway: `eon workspace --json` printed the
    tab directory and each pane endpoint as JSON strings; human `eon windows`
    printed the independent window's generation as `eon generations` does,
    with `--json` unchanged. Installed
    `eon-prove-close-and-detach-batch-bf8t` observation (edge
    `22915af5dca41f87e706ad548c0506f79c17648f`, installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`) on isolated
    headless Sway, from a pseudo-terminal: `eon tab close t1` on the final
    tab asked `Close the last tab? This quits the window and stops 1
    terminal [p1]. [y/N]`, and `n` left the window open; `eon pane close p1`
    on the final tab's last pane asked the same, and `y` ended the window;
    `eon tab close t1 --json` asked nothing, printed
    `{"ended":{"generation":...}}` and ended the window.
    Installed `eon-agent-sidebar-yx8.9` observation (edge
    `c8c62cb63938b145b13cfa4fe161ea5689807652`, installed `eon` profile
    `/nix/store/m5mzxy8cikz3f6y7k0ldgf7qxmc64bji-eon-0.1.1`, EONW 19) on
    isolated headless Sway:
    - `eon workspace --json` gave each pane and popup an `attention` of
      exactly `state`, `app` and `name`, or null. Examples are
      `{"state":"idle","app":"codex","name":"Create proof file | w"}` and
      `{"state":"unavailable","app":null,"name":null}`.
    - It gave each tab an `attention_rollup` and the snapshot `sidebar`.
    - Human `eon workspace` printed `attention=`, `app=` and `name=` per
      terminal, `attention=` per tab and a `sidebar` line.
    Installed `eon-ed9d.1` observation (this change on edge
    `f02f2214c8b8c543f00c65102781d99d9adf1e92`, tested tree
    `0f9daedcf28c67edbff54106922f3f5c4700535b`, package
    `/nix/store/cbm43x2541v809cgibah2qqapi3vbs11-eon-0.1.7`, EONW 24) on
    x86_64 Linux native Wayland, isolated headless Sway:
    - From pane p1, `eon workspace --json` gave
      `"caller":{"terminal":"session-1","tab":"t1","pane":"p1","popup":null}`.
    - A fresh `claude -p` (Claude Code 2.1.303) and a fresh `codex exec` (Codex
      0.165), each with only the packaged skill installed project-locally,
      were asked to start `top` beside them without disturbing the person. Each
      ran `eon pane create --no-focus --json`.
    - p2 and p3 joined column 1, and the active tab t1 and the selected pane p1
      stayed.
  - **Covered popup (`eon-knowp`):** an ended transient popup shows again the
    popup it covered, after a workspace switch and in place, keeps it when a
    pane action's stop of the transient fails, shows it after `eon popup hide`
    of the transient, and returns to the pane when it covered none or a pane
    action dismissed the transient
    (`an_ended_transient_popup_shows_the_popup_it_covered_again`).

## EON-C11 — Runtime generations and presentation

- **Status:** Proven on x86_64 Linux native Wayland. Amended on 2026-10-30
  (approved by Lucca, relayed by eon-overseer) by
  `eon-agent-parity-a9g0.15`: only the Venus a supervisor starts attaches to
  its window's terminals, proven by its amendment proof below. Amended on
  2026-11-03 (approved under eon-7cf0y's approval mode, relayed by
  eon-overseer) by `eon-7cf0y.12`: `eon quit` asks only while a terminal is
  busy, and runtime results and workspace snapshots carry the busy terminals,
  proven by its amendment proof below. Amended on 2026-11-03 (approved under
  eon-7cf0y's approval mode, relayed by eon-overseer) by `eon-7cf0y.9`: a
  generation from another EONW version is stopped through the `eon` it
  recorded, proven by its amendment proof below.
- **Consumer:** Eon and eonbase users launching or targeting a product generation.
- **Trigger:** Launch current generation, inspect an older live generation,
  present a compatible existing surface, stop one generation, stop a selected
  set of generations, or quit one window or every window.
- **Result:**
  - Eon and eonbase use separate runtime namespaces. Launch and lifecycle defaults use their exact
    current generation; Eon workspace control follows EON-C10's caller and explicit-target rules.
  - Generation identity is Runtime's `g1-` hash of Eon's source-tree digest,
    which covers runtime/EONW source and build inputs and Eon
    assembly/default/validator/graph/Cargo/Nix-generated inputs; each
    generation owns one private directory. Store/profile paths, mutable
    configuration, PIDs and live state do not define identity.
  - Older live generations that speak the current EONW remain discoverable,
    presentable when compatible, and stoppable through their own supervisor;
    presentation still requires the exact current EONW and component graph. A
    generation from a build with another EONW version is listed as
    incompatible, and this `eon` does not inspect it. When that generation
    recorded its `eon` (below) and the recorded file exists and is executable,
    `eon generations` and `eon windows` report its `stop` as available.

    Every Stop of it, through `eon stop`, `eon window stop`, `eon quit` or
    `eon quit --all`, runs that `eon` as `stop GENERATION --json` in the
    generation's namespace, without standard input, after this `eon` asks any
    question the command asks. Its live and busy terminals are unknown here,
    so human-mode `eon quit` and `eon quit --all` always ask: `Quit window
    GENERATION? It runs another Eon build, so its terminals are unknown.
    [y/N]`.

    This `eon` reports that `eon`'s Stop result as its own. Output that is not
    one Stop result, a Stop that a signal ends, and a `stopped` result from an
    `eon` that exits unsuccessfully are `stop-failed`, and a delegated Stop
    never delegates the same generation again.

    Without a record, the Stop is `stop-unavailable` and says the build that
    started it can stop it. With a record whose file is missing or not
    executable, the Stop is `stop-unavailable`, names the recorded path, and
    says to restore that build and run its `eon stop GENERATION` in the
    generation's namespace.
  - A generation records the `eon` executable of the build that started it, an
    opaque launch input, so a later build can name or run it; `eon attach
    GENERATION` delegates to it when this build cannot present that
    generation.
  - `eon stop previous` attempts every non-dead generation except the exact
    current one, whether current is live or not. `eon stop all` includes
    current. Each selected generation uses the existing validated
    Stop; human mode confirms each target, while `--json` returns a result array
    without confirmation. Declining one target reports that no Stop was sent for
    that generation; without a terminal that target is `not-confirmed` (exit 1),
    and the command exits with the largest status. Earlier outcomes remain and
    later targets continue.
    Failed or unavailable targets do not suppress later targets while the CLI
    remains running.
    A Stop of the caller's own terminal can terminate the CLI before it receives
    that result or attempts later targets; run from outside selected generations
    when a complete result is required. When the CLI completes, it exits
    nonzero if any Stop attempt fails. `all` stops current last so earlier
    targets can finish when invoked inside a current terminal.
    Neither command forcibly kills a process.
  - `eon quit [--json]` stops one window with the existing validated Stop of
    one generation and Stop's JSON result. In human mode it asks first only
    while a terminal of that window other than the calling terminal is busy
    (below). The question is `Quit this window? It stops N terminals; M still
    running: NAME, NAME and K more. [y/N]`, where N counts the live terminals
    Stop ends and each NAME is a busy terminal's name; outside Eon it says
    `Quit window WINDOW?` with the window's name (EON-C23), else its
    generation. A decline prints `cancelled; this window keeps running` (or
    `window WINDOW`) and stops nothing; EON-C10's confirmation rules apply.
    While no terminal is busy it stops without asking. When it cannot inspect
    the window's busy terminals, it asks and names every live terminal of the
    window by its session ID, the calling terminal included. Run from an Eon
    terminal, it stops the generation whose supervisor validates that
    terminal's `EON_TERMINAL` (EON-C10); when caller discovery completes and
    no supervisor validates the caller, it fails with `unknown-caller` and
    stops nothing; when no owner can be found because discovery cannot inspect
    a supervisor, EON-C10's `supervisor-unavailable` failure is preserved and
    nothing stops. Run without `EON_TERMINAL`, it considers every non-dead
    generation of the default namespace and of every independent window
    (EON-C23): it stops the only one; with none it
    fails with `missing-supervisor`; with several it fails with
    `ambiguous-window`, stops nothing, and names each one's window ID or the
    default namespace, generation, state, live terminal count when inspectable
    (otherwise unknown) and the `eon window stop ID` or
    `env -u EON_WINDOW_RUNTIME_DIR eon stop GENERATION` command that stops it,
    or, when this `eon` cannot stop it, `cannot stop: REASON`.
    `eon quit --all [--json]` attempts Stop of every non-dead generation in the
    default namespace and in every independent window, in `eon stop all`'s
    order, the caller's own namespace last, asking for each window as
    `eon quit` does, except that every question, the caller's own window's
    included, says `Quit window WINDOW?`; outside Eon, the default namespace
    counts as the caller's. With `--json` it
    prints one array holding one `{"window": ID, "results": [...]}` object per
    namespace, with `window` null for the default namespace, and it exits with
    the largest status. Neither form forcibly kills a process, and quitting the
    caller's own window can end the CLI before it reports, as Stop can.
  - **Busy terminals.** A window's pane or popup terminal, directory pickers
    excepted, is idle only while all of these hold: Eon started it with the
    window's default shell (`[shell] command`), not an explicit command
    (`pane create -- COMMAND`, `tab create … -- COMMAND`,
    `eon run -- COMMAND`, or a popup); its last good Orbit `ProgramStatus`
    reply (ORB-C15) found the child's own group in the terminal's foreground;
    and that reply is less than 3 s old, EON-C31's stale bound. The Files and
    Git popups count as idle whatever they run, because their operations are
    short and rare and counting them would make every quit ask once Files was
    opened. Every other live terminal is busy, including a terminal
    recovered under EON-C15, whose start Eon does not know, and one whose
    foreground Orbit could not read. Eon does not see a program that runs in
    the shell's own process group, such as a shell builtin or any command
    under a shell without job control, nor a background job of an idle shell.
    The supervisor derives the list from EON-C31's cached replies and never
    waits on a terminal for it. Each runtime result (a lifecycle inspection
    or Present) and each workspace snapshot of a full-Eon window carries its busy terminals in window order,
    at most 32, each with its workspace, tab, pane or popup, terminal and name
    (a pane's label, a popup's entry label), and how many more are busy; a
    lifecycle inspection whose request names a live terminal of the window
    leaves that terminal out. eonbase carries none.
  - Implicit attach selects only a live supervisor reporting the exact current
    identity; otherwise Eon starts current without stopping older work. Explicit
    attach never substitutes another generation, and discovery distinguishes
    current, previous, dead, incompatible, and corrupt candidates. A live full-Eon
    candidate also reports its window name and the Unix millisecond time its
    supervisor started (EON-C23); `eon generations --json` and `eon windows --json`
    print them as `name` and `started_at`, and its busy terminals as `busy`,
    null for any other candidate.
  - One per-generation lifecycle lock covers listener and runtime teardown so a
    replacement cannot overlap cleanup; concurrent launch converges on one
    supervisor and terminal set.
  - Present requests native presentation without replacing the live Venus
    process, Orbit attachment, or terminal. A detach keeps every workspace of
    the window, and a Venus that Present starts shows the last shown workspace
    (EON-C33; its installed proof is `eon-workspace-switching-shn4.9`).
  - Dead exact residue is removed only after the recorded process is absent or
    dead; a later launch may then create one fresh run.
  - A current launch overlapping clean exit of the final terminal waits for the
    retiring supervisor, then starts one fresh terminal instead of reporting
    presentation success against ended state.
  - Only the Venus process a supervisor starts to present its window may attach
    to that window's terminals as their interactive client. The supervisor
    names that process, by process ID and process start identity, to each live
    terminal's Orbit through ORB-C13 `AttachAllow`:
    - when it starts that Venus, at launch and at every later presentation
      (`eon attach`, a window reattached after detach);
    - for each terminal it starts, or recovers under EON-C15, while that Venus
      runs.

    A terminal it cannot name stays unattached, and Venus reports it as it
    reports a refused attachment (`Busy`) and retries. No other process can
    attach to a terminal of a managed window, whether or not the window shows
    it.
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
- **Consumes:** Orbit management v5 (`ORB-C15` foreground), `ORB-C3`
  admission, EON-C31 polling, and Venus `VEN-C14`.
- **Boundary:** No manager daemon, persisted topology, machine-restart recovery,
  PTY handoff, old-generation backport, updater, Nix evaluation, package-channel
  identity, presentation compatibility window, remote runtime, plugin API, service-manager
  requirement, or automatic eviction. Naming the presenting Venus keeps stray
  same-user processes from taking a terminal's interactive attachment. It is
  not a defence against same-user code that controls the supervisor or the
  named Venus.
- **Assembly boundary:** The `eon-runtime` generation owner hashes Eon's
  source-tree digest, which includes runtime/EONW source and generated-command
  inputs. Exact presentation checks remain; retain the prior executable to
  reattach incompatible old work. See
  [generation inputs](ARCHITECTURE.md#defaults-overrides-and-generation).
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `generation_identity_is_stable_bounded_and_namespaced`,
    `discovery_bounds_dead_cleanup_and_rejects_unsafe_metadata`,
    `concurrent_launches_converge_and_generation_stop_is_owner_routed`,
    `bare_eon_attaches_only_to_the_live_current_generation`,
    `launch_overlapping_last_session_exit_starts_a_fresh_session`,
    `previous_and_all_stop_older_generations_when_current_is_unstarted`,
    `batch_stop_preserves_current_until_all_and_reports_unavailable_generations`,
    and `stop_confirmation_refuses_a_replacement_supervisor`; from
    `eon-igae.12.4`,
    `generations_from_another_eonw_are_incompatible_and_not_stopped`.
    Installed checklist items 2 and 12: detach and attach of one live
    generation, and Stop of that one exact generation.
- **Launcher record proof:** `eon-reattach-after-upgrade-n5qv`; that bead's
  commit is its revision.
  - **Environment:** Automated tier; isolated installed tier (private headless
    Sway, x86_64 Linux), builds of that revision and of it plus a comment-only
    change.
  - **Evidence:** `window_attach_runs_the_build_that_started_its_generation`.
    Installed: a detached window was presented again by a later build's
    `eon window attach ID` and by the `eon` its generation recorded; see
    EON-C23.
- **Amendment proof:** The `eon-agent-parity-a9g0.15` amendment is checked by
  `attach_allow_shares_the_request_slot_and_names_the_one_peer_orbit_admits` in
  `runtime/crates/eon-runtime/src/sessions.rs`, against a real Orbit, and by
  `a_workspace_final_close_shows_the_recent_one_and_attach_keeps_it` and
  `replacement_eon_adopts_exact_runs_and_projects_numeric_workspace` in
  `eon/crates/eon/tests/workspace_control.rs` (launch, a terminal created
  afterwards, `eon attach` after detach, and recovered terminals). It landed in
  `9c90032`. Installed observation: the `eon-agent-parity-a9g0.19` commit, in
  the isolated installed Eon recorded under
  [ORB-C3](../orbit/docs/CONTRACTS.md#orb-c3--bounded-interactive-and-metadata-attachment).
  Venus attached at launch, to a later tab, a hidden workspace's tab and a
  keep-alive popup when shown, and to each again after Alt+Shift+D detached the
  window and `eon attach` presented it; typing reached the later tab, the
  hidden workspace's tab and the popup, and the first tab's terminal echoed a
  key. `eon-ahzkt`'s ORBS probe got `Busy`, so its command was never sent, on a
  hidden tab's and a hidden popup's terminal, and on the shown tab's terminal
  while the window was detached.
- **Amendment proof (`eon-7cf0y.12`, busy terminals):** the `eon-7cf0y.12`
  commit.
  - **Environment:** x86_64 Linux; sandboxed `nix develop -c eon-test`; the
    installed package `/nix/store/rzx4c0kf162zv2k32rjh09iz7dpikf3d-eon-0.1.13`
    in an isolated private headless Sway.
  - **Evidence:**
    - Runtime: `busy_needs_the_default_shell_a_child_foreground_and_a_fresh_reply`,
      `quit_asks_only_while_busy_in_window_language` and
      `cli_json_carries_busy_terminals`.
    - EONW v29: `busy_terminals_round_trip_and_reject_forged_bounds` and
      `busy_terminals_keep_their_wire_bytes`.
    - Integration in `eon/crates/eon/tests/workspace_control.rs`:
      `quit_without_a_busy_terminal_stops_without_asking`,
      `quit_asks_in_window_language_and_names_busy_terminals`,
      `quit_counts_a_silent_terminal_busy_after_3_s` (whose
      `eon stop GENERATION --json` on a busy window also covers the Stop path
      the close notice shares), `stop_keeps_its_always_ask_question` and
      `quit_all_asks_only_busy_windows`.
    - Installed observation: an idle default shell is not busy; a Nushell
      external command (`^sleep`) makes it busy within about 1 s, and a
      builtin (`sleep 8sec`) does not; an explicit-command pane and the Agent
      popup are busy, and the Files popup is not; a terminal whose Orbit was
      stopped turns busy once its last reply is 3 s old; `eon quit` from a
      terminal asks `Quit this window? It stops 3 terminals; 1 still running:
      p2. [y/N]`, naming neither itself nor Files, and quits without asking
      once nothing is busy, inside and outside Eon.
  - **Gap:** the close notice's Quit is covered by construction only
    (`eon-7cf0y.14`); macOS is unproved.
- **Older-EONW Stop proof:** `eon-7cf0y.9`; that bead's commit is its revision.
  - **Environment:** automated tier; isolated installed tier (private headless
    Sway, x86_64 Linux) with Eon 0.1.10 (EONW 27) and 0.1.12 (EONW 28) windows.
  - **Evidence:** `older_eonw_generations_stop_through_their_recorded_eon` and
    `quit_listing_names_only_working_stop_commands`; the installed steps are
    recorded on the bead.
- **Open proof:** Pending re-proof in this repository: native Present of a live
  surface and launch with `/sys/fs/cgroup` hidden.

## EON-C12 — Standalone exact-command terminal

- **Status:** Proven
- **Consumer:** A local user or approved composition needing one native terminal
  surface without Eon workspace actions.
- **Trigger:** eonbase launches one exact command after `--`.
- **Result:**
  - `eonbase -- COMMAND...` launches one exact nonempty argv in one Orbit terminal
    and one Venus surface without workspace topology. Optional
    `--no-decorations` is fixed for the supervisor lifetime.
  - `eonbase generations`, `attach`, and `stop` expose EON-C11 within eonbase's
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
- **Boundary:** eonbase consumes no EON-C9 managed environment and adds no tabs,
  panes, key remapping, preset, wrapper, persisted mode marker, workspace,
  managed-tool/default-shell command, remote access, focus guarantee, or Zellij
  policy. Bare `eon` and `eon run` retain workspace meanings.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `eonbase_reopens_without_workspace_or_a_second_session`,
    `eonbase_replaces_exact_residue_after_its_supervisor_and_session_die` and
    `rejected_native_startup_creates_no_session_in_either_product`; the
    `eonbase-closure` Nix check. Installed checklist item 11: launch, detach,
    `eonbase attach` and exact Stop; `--no-decorations` started.
- **Rename proof:** `eon-rename-eonterm-eonbase-jpbm`; that bead's commit is its
  revision.
  - **Environment:** Automated tier; isolated installed eonbase on a private
    headless Sway 1.12 on x86_64 Linux.
  - **Evidence:** the renamed tests and `eonbase-closure` check;
    `eonbase_reopens_without_workspace_or_a_second_session` also rejects the
    retired `eonterm` name. Installed: launch, detach, `eonbase attach`, exact
    Stop, `--no-decorations` launch and Stop, and `eon window new` through the
    Nix wrapper.
- **Open proof:** Pending re-proof in this repository: native supervisor-loss
  recovery of the same eonbase terminal, and `--no-decorations` rendering
  under a decorating compositor.

## EON-C13 — Terminal-background opacity policy

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Configuration behavior](CONFIGURATION.md#configuration-behavior); proof in
  `eon-monorepo-2oko.6`, `eon-vn0nk`.
- **Reason:** Background opacity is presentation that only a person looking at
  the window relies on. Its `[terminal] background_opacity` key and default stay
  documented in Configuration, which EON-C28 covers.

## EON-C14 — Default compositor blur policy

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Configuration behavior](CONFIGURATION.md#configuration-behavior); proof in
  `eon-monorepo-2oko.6`, `eon-vn0nk`.
- **Reason:** Background blur is presentation that only a person looking at the
  window relies on. Its `[terminal] background_blur` key and default stay
  documented in Configuration, which EON-C28 covers.

## EON-C15 — Same-boot Orbit recovery

- **Status:** Proven on x86_64 Linux by the automated tier; installed
  supervisor-loss adoption is pending re-proof in this repository.
- **Consumer:** One local Eon or eonbase user on the same boot and login.
- **Trigger:** An Eon-launched Orbit crosses Ready, its supervisor disappears,
  its acquired owner dies, a replacement targets the same exact generation,
  or the user requests Stop.
- **Result:**
  - Eon assigns canonical `session-N`, one fresh opaque run ID, and the runtime
    generation ID as component generation, then validates complete live identity
    and acquires the sole management lease within one shared five-second
    operation.
  - Replacement eonbase recovers exactly one live run; replacement full Eon
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
  - When an acquired Orbit stops on its own termination signal (`ORB-C12`),
    Runtime reconciles its stop tombstone as it does a natural exit. Full Eon
    logs the terminal's status and prunes the ended pane as usual. This is not
    a Stop that Eon requested.
- **Important failures:** Unsafe, missing, replaced, malformed, oversized,
  incompatible, duplicate, zero, noncanonical, overflowing, wrong-generation,
  wrong-UID, wrong-process, Busy, partial acquisition, unreconciled Orbit exit,
  deadline, or transport failures publish no partial workspace, launch no
  duplicate run, and authorize no PID, signal, pathname, process-name, group, cgroup, or pidfd
  fallback. A lost response after completed Stop cannot revive the supervisor.
  Unsolicited, undecodable or unexpected management bytes on one terminal's
  lease fail only that terminal's in-flight request. Runtime discards them,
  keeps the lease, and still reaps, polls and stops that terminal. They never
  fail the supervisor or affect other terminals.
  A marked Live record rejected after spawn is stopped through canonical
  management using its published identity and is never published as a terminal.
  Failed Stop retains its supervisor and unfinished leases for explicit retry;
  completed runs leave live inventory. Workspace mutations and presentation
  remain unavailable until cleanup succeeds. Failed natural-exit reconciliation
  still collects other results and restores retryable leases. Discovery reaps
  ended untracked direct children before retiring their canonical records.
  During failed cleanup, it still reaps an exited presentation child without
  changing the workspace or launching replacement terminals.
  Stop's result read timeout allows one preceding thirty-five-second startup
  (thirty-second presentation admission, then a five-second first terminal
  start) plus five-second cleanup and a one-second margin; excessive queues can
  still fail.
- **Owner:** Eon Runtime generation/recovery enforcement, bounded enumeration,
  projection, lease collection, deadline, EONW ordering, retained launch claim,
  and exact residue; Orbit `ORB-C13` owns live identity, Ready, lease, terminal state,
  `ORB-C12` cleanup, tombstone, and endpoint cleanup.
- **Consumes:** Orbit `ORB-C12`, `ORB-C13`, management v5, and Venus `VEN-C14`.
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
    `eonbase_replaces_exact_residue_after_its_supervisor_and_session_die`);
    Stop and cleanup
    (`stop_accounts_for_late_ready_picker_and_retains_partial_cleanup_owner`,
    `stop_result_covers_queued_operation_and_managed_cleanup`,
    `lost_stop_response_still_finishes_the_supervisor`,
    `failed_directory_picker_stop_is_retried_during_supervisor_cleanup`,
    `failed_natural_cleanup_still_finalizes_other_runs_and_restores_leases`);
    owner loss (`lost_owner_cleanup_requires_death_and_original_owned_objects`,
    `discovery_reaps_untracked_ended_children_before_removing_records`);
    self-stop and lease anomalies, `eon-wt6l`
    (`an_orbit_stopped_by_a_signal_ends_only_its_terminal`,
    `an_undecodable_or_unsolicited_result_ends_only_its_request`).
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
- **Consumer:** Full Eon, standalone eonbase, or one approved eonbase composition.
- **Trigger:** The caller launches a new Venus surface with its selected validated
  desktop application ID.
- **Result:** Full Eon supplies `eon`, standalone eonbase supplies `eonbase`, and
  an approved composition passes its value unchanged to `VEN-C17` before window
  creation; identity never enters PTY argv/environment and remains independent
  from terminal-authored titles.
- **Important failures:** Empty, non-UTF-8, oversized, or non-token identities
  fail before Orbit or Venus starts; profile refresh does not mutate a live
  surface's selected identity.
- **Owner:** Eon supplies defaults; Eon Runtime validates and projects eonbase
  CLI input; the embedding composition owns its identifier/desktop metadata;
  Venus owns native mapping only.
- **Consumes:** Venus `VEN-C17`.
- **Boundary:** Immutable launch metadata only; no runtime protocol, branding
  provider, desktop discovery, child inspection, or wider platform promise.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `venus_receives_workspace_endpoint_only_in_workspace_mode`
    checks the projected `--application-id` for both products. Installed
    checklist items 1 and 11 launch Eon and eonbase; item 1's launch waited for
    the Sway tree to show the native app ID `eon`.
- **Open proof:** Pending re-proof in this repository: installed rejection of
  invalid eonbase identities, distinct launcher grouping, and an approved
  embedding's caller-owned identity.

## EON-C17 — Tab launch directory

- **Status:** Partially proved. Amended on 2026-11-03 (approved by Lucca,
  relayed by eon-overseer) by `eon-quick-search-parity-23y9.3`: the Zoxide
  visit of an accepted `SetTabDirectory`. Amended on 2026-11-03 (approved by
  Lucca, relayed by eon-overseer) by `eon-quick-search-parity-23y9.2`:
  `SetTabDirectory` on a pending tab is a pick of EON-C18's pair.
- **Consumer:** One local Eon workspace user creating tabs and panes.
- **Trigger:** Eon creates or recovers a workspace, creates a tab or pane, or an
  approved same-user client explicitly retargets one live tab.
- **Result:**
  - Every live tab has one stable `tN` identity and one authoritative absolute
    launch directory. Panes retain `pN` identities.
  - Fresh `t1` validates the supervisor's absolute launch directory before any
    terminal recovery or startup. Later new tabs start at the current user's
    home directory, resolved by the native home-directory API and validated before creating the tab. EON-C18's
    pending pair commits the selected first-pane directory or starts a pane
    at the tab's starting directory when dismissed.
  - `SetTabDirectory` changes exactly one addressed tab. On an established tab,
    later panes start with the accepted directory; existing terminals and
    terminal CWDs do not change. On a pending tab it is a pick (EON-C18): it
    commits the directory and starts exactly one first pane there. Each
    accepted `SetTabDirectory` then adds one Zoxide visit for the accepted
    directory through Eon's packaged Zoxide, unless `[shell] zoxide = false`.
    Zoxide's own exclusions apply: by default it records no visit for the home
    directory. The visit runs beside the supervisor; its failure is logged and
    changes nothing else. A refused `SetTabDirectory` adds no visit.
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
  automated tier. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-quick-search-parity-23y9.2`: the Project chooser and
  Files become one directory pair. Re-proof is pending at v31 under this bead.
- **Consumer:** One person starting or using full Eon.
- **Trigger:** Eon needs the first pane for a new tab, or the person presses
  Alt+Z or Alt+Shift+Y.
- **Result:**
  - On a fresh full-Eon workspace, after Venus accepts the initial presentation,
    Eon may play one short Anima animation in the initial pair's terminal
    before its Files Yazi starts. The default is enabled,
    a random native style, and three seconds; `[anima].enabled = false` skips it.
    Any key other than Anima's style-browsing keys dismisses playback without
    becoming quick-search input. This step does not run for later tabs,
    attachment, reconnection, same-boot recovery, or eonbase.
  - A fresh workspace and every later new tab not created with a directory
    (EON-C10's `tab create --directory`) begin as one active pending tab
    with no durable pane. Eon starts the tab's pair: the exact packaged Files
    Yazi in one Orbit popup terminal at that tab's starting directory (the
    initial launch directory for t1, the user's home for later tabs), showing
    quick search first. The same terminal stays the tab's Files popup after the
    tab gets its first pane. Ambient fzf default options cannot alter its
    command, layout, or bindings. Other contracts call a pending tab's pair its
    directory picker.
  - The pair has two screens. Quick search ranks Zoxide history through fzf.
    - In quick search, Enter picks the highlighted folder. Tab moves Files to
      the highlighted result without picking; with empty history, Files stays
      where it is. Esc and Ctrl+C close the pair. Every quick-search exit stops
      and reaps a still-running history query.
    - In Files, Enter on a hovered folder enters it, and Enter on a file opens
      it. Alt+Enter picks the hovered folder, else Yazi's current folder. Tab
      returns to quick search without moving Files. Shift+Tab spots the hovered
      file. Esc and Ctrl+C first leave a visual mode, selection, filter, find
      or search, and otherwise close the pair. Z, q and Q do nothing. Every
      other key keeps Yazi's native behavior, including file operations.
    - Quick search and Files each show one persistent footer: "Enter Use · Tab
      Browse from here · Esc Close" and "Enter Open · Alt+Enter Use folder ·
      Tab Search · Esc Close".
  - Alt+Z shows the pair on quick search and Alt+Shift+Y shows it on Files;
    each hides the pair when its own screen is already shown.
  - A pick sets the owning tab's directory, as `eon tab directory` with its
    `EON_TAB` would; Yazi does not move. It refuses when that tab no longer
    lists the pair, as after a supervisor recovery. A notification reports a
    refusal or failure, and a failure changes nothing.
  - Closing the pair runs `eon popup hide` with its popup ID. A pair Yazi that
    its tab no longer lists as a popup, as after a supervisor recovery, quits
    instead.
  - Opening, browsing, quick search, screen switches and closing add no Zoxide
    history. A pick adds one visit, as EON-C17 describes.
  - A pick on a pending tab atomically commits the tab directory, starts
    exactly one first `pN` terminal there, and hides the pair. A pick on an
    established tab changes its directory and leaves running panes alone.
    Closing the pair on a pending tab is dismissal: the tab keeps the pair as
    its Files popup and starts its first pane at the validated starting
    directory. On an established tab, closing hides the pair and keeps its
    state. Explicitly closing a pending tab stops its pair and removes that tab
    without starting a pane; when it is the final tab, the close acts as
    EON-C8's final-tab close.
  - `eon tab create --directory DIRECTORY` skips the pending pair: its tab is
    never pending and its first pane starts at DIRECTORY (EON-C10). A bare `eon
    tab create` and Venus's new-tab action keep the pending pair.
  - EONW v31 exposes the pair as the tab's `files` popup endpoint, never as a
    normal `pN` pane; a pending tab holds exactly that one selected popup and
    no pane. The pair tab may remain live while another durable tab is active.
    A pair-owned pending tab remains pane-free, with an absent selected pane
    instead of a sentinel or placeholder process. Lifecycle inspection and Stop
    may report zero durable terminals during the initial pair; the pending pair
    is a popup and is not counted among them.
  - Venus can start a full-Eon presentation from the workspace endpoint alone
    while the initial tab is pending; the pair's endpoint in its first snapshot
    is the sole terminal attachment.
  - Venus keeps the tab bar visible and replaces the pane stack with the pair,
    using Eon's fixed popup margins. The
    previous terminal is not composited underneath. Attach and recovery never
    create an automatic pair; recovering only a stale initial pending pair stops
    it and applies the validated launch-directory fallback.
  - Left/right focus actions continue to traverse and wrap live tabs
    while the pair stays bound to its original tab. Another active tab shows
    its selected pane; returning shows the same pair for normal acceptance or
    cancellation. Tab navigation by identity remains available. Other tabs
    allow pane creation, pane focus by direction or identity, pane/tab movement,
    explicit directory updates, and stable-target close. These actions preserve
    the pair's binding and terminal. Mutations of the pair-bound tab remain
    blocked except directory acceptance and closing its pending tab;
    that close stops the pair's terminal before removing the tab. Dismissing
    a background pending pair starts that tab's first pane while leaving
    the selected tab in focus. Explicit tab close selects the nearest
    surviving tab. A workspace with no running terminals ends.
    Pairs remain attached across tab switches; established tabs may open
    their own popup while another pair remains live elsewhere. EONW v7 permits
    multiple pending tabs, each retaining its pair across switches;
    CreateTab always creates a distinct tab. Alt+Shift+W closes the selected
    pending tab and stops only its pair, ending the window when it is the
    final tab.
- **Important failures:** Unresolvable or invalid home, or a failed new-tab pair
  launch, creates no tab. Invalid selection never commits its path; duplicate or
  stale results cannot mutate another tab or its terminals. Dismissal, pair exit
  or presentation detachment dismisses the pending pair and attempts the first
  pane at the pending tab's starting directory. If that pane cannot start,
  report the error and remove only the unstartable tab; other tabs and terminals
  remain. Left/right with only one live tab succeeds unchanged. A duplicate
  close cannot stop the pair twice or remove another tab. Invalid Anima settings
  fail before a fresh workspace starts. Missing, failed, or timed-out playback
  reports a short diagnostic and continues to the pair.
- **Owner:** Eon supplies pair/default policy and packaged commands; Eon
  Runtime owns tab binding, command selection, lifecycle, validation, mutation
  and cleanup. Venus owns full-Eon shortcut precedence,
  modal geometry, focus, input, notices, rendering, and accessibility. Orbit
  owns the pair's PTY and child. Zoxide owns ranking, fzf owns quick selection,
  Yazi owns filesystem browsing, and Anima owns style selection, rendering,
  input dismissal, playback timing, and terminal restoration. Eon consumes one
  picked directory per pick without interpreting Yazi's navigation state.
- **Consumes:** EON-C17, current EONW v31 popup/pending-tab values, Orbit's
  accepted terminal startup/attachment/exit/stop contracts and the selected exact
  Venus consumer.
- **Boundary:** No custom filesystem walker, file opening from a pending tab's
  pair (its `eon edit` is refused until the tab has a pane), copied Nova plugin,
  normal-pane identity, client-supplied command, simultaneous terminal
  composition, native Venus picker, placeholder shell, pane replacement,
  current-process `cd`, persisted pending tab, eonbase action, or additional
  platform.
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
- **Consumer:** Eon and eonbase users opening a native surface.
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
  - `[terminal] padding_color` accepts `extend` (default), `extend-always`, or
    `background`; Eon Runtime resolves it and supplies Venus's `--padding-color`
    launch input for terminal padding as
    [Configuration behavior](CONFIGURATION.md#configuration-behavior)
    describes.
  - The authoritative startup workspace Snapshot transfer and native readiness
    share one absolute thirty-second deadline, including large snapshots, so a
    busy machine does not close a slow but healthy window. A missed deadline
    reports that Venus was not ready within it. Ordinary Present messages retain
    their 250 ms write timeout.
- **Important failures:** Eon Runtime rejects malformed configuration with field
  diagnostics before children. Venus admits fonts and actual initial native
  geometry before any new terminal or user command. Rejected, lost or timed-out
  admission closes only the attempted presentation and preserves durable terminals.
- **Owner:** Eon supplies terminal defaults; Eon Runtime owns strict parsing,
  argv projection and authoritative startup Snapshot transfer; Venus owns font
  resolution, native readiness and geometry.
- **Consumes:** Exact accepted Venus VEN-C19 startup admission.
- **Packaged fonts:** The Venus font configuration includes DejaVu, Symbols
  Nerd Font Mono, Anima's font-only Accelerando Terminal artifact and, on Linux,
  Noto Color Emoji. Eon and eonbase share that configuration; eonbase does not
  consume the Anima executable.
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

- **Status:** Proven.
  Amended on 2026-11-04 (approved by eon-overseer under Lucca's standing
  rule) by `eon-hbpkq`: a popup on one workspace position key takes only that
  key.
- **Consumer:** One installed full-Eon user on native Linux Wayland.
- **Trigger:** The user presses physical Alt+Slash from the Eon workspace.
- **Result:** The exact accepted Venus client toggles one native, read-only
  shortcut viewer containing its fixed Eon-surface bindings and the current
  enabled popup catalog supplied through current EONW v7. Opening, refreshing,
  scrolling, resizing, and closing the viewer change no workspace or Orbit
  terminal state. Title, subtitle and first group heading occupy separate
  readable vertical space at normal and compact window sizes. A configured
  popup shortcut on the key of any other fixed binding, except Alt+H, Alt+J,
  Alt+K and Alt+L, takes that key; the fixed binding is unbound while that
  configuration is active, and the viewer lists it as overridden by the popup.
  On Alt+Shift+0 through Alt+Shift+9 a popup takes only its own key: the other
  workspace position keys stay bound, and the viewer names the taken key as
  overridden by the popup.
  A built-in popup whose default key a configured popup holds is listed with
  that key as overridden by the configured popup; a popup configured without a
  key has no row. The fixed bindings include
  Alt+Shift+D, listed as "Detach: hide the window, keep terminals running",
  Alt+Shift+Q, listed as "Quit: close the window, stop its terminals" (EON-C8)
  and omitted in eonbase,
  Alt+Shift+H, listed as "Show or hide the agent sidebar" (EON-C31,
  VEN-C30) and omitted while `sidebar` is `Off`, and Alt+G and Alt+Shift+G,
  listed as "Jump to the next agent needing attention" and "Jump to the
  previous agent needing attention" (EON-C31, VEN-C30) under every `sidebar`,
  and Ctrl+Tab and Ctrl+Shift+Tab, listed as "Focus next tab" and "Focus
  previous tab" (VEN-C8).
- **Important failures:** Eon rejects configured popup shortcuts on Alt+Slash
  or positional Alt+0 through Alt+9, a reserved set no 1.x release enlarges,
  before workspace mutation. A configured popup shortcut on Alt+H, Alt+J,
  Alt+K or Alt+L instead loads without a key, with a logged configuration
  warning naming the popup and the key; no 1.x release enlarges this set
  either.
  Invalid or stale component identity fails before profile activation. Profile
  refresh preserves running supervisors and terminals; they retain their older
  client until a normal restart.
- **Owner:** Eon owns component/default selection, profile delivery and
  installed acceptance; Eon Runtime resolves popup keys and enforces
  configured popup collisions.
  Venus owns viewer state, input, rendering, focus, accessibility, and a
  configured popup's precedence over its other fixed bindings.
- **Consumes:** Venus VEN-C20 and VEN-C5, current EONW v7 and Orbit terminals.
- **Boundary:** No Eon-rendered viewer, second shortcut catalog, EONW change,
  Orbit update, child-application bindings, rebinding UI, command palette,
  tutorial, telemetry, momentary mode, dependency, or platform expansion.
  Venus derives overrides from the popup catalog it already receives over
  EONW. Exceptions: the viewer reads `sidebar` and each popup entry's override
  from EONW v31.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `popup_configuration_owns_defaults_overrides_and_collisions`;
    Venus `shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space`,
    `shortcut_viewer_uses_live_rows_and_captures_its_local_sequence`,
    `shortcut_viewer_replaces_terminal_tree_with_a_focused_dialog` and
    `recognized_shortcuts_are_one_shot`. Installed checklist item 5.
  - **Override evidence (`eon-igae.10`):** Runtime
    `popup_configuration_owns_defaults_overrides_and_collisions` and Venus
    `configured_popups_take_fixed_bindings_except_reserved_keys`; isolated
    dogfood of the built package on a private headless Sway (1280x720), with a
    Files popup configured on Alt+Shift+N: the key opened and hid the popup
    without a second window, and Alt+/ listed `Open new Eon window (overridden
    by Files)`.
  - **Focus keys (`eon-0gllz`):** Runtime
    `popup_configuration_owns_defaults_overrides_and_collisions` loads custom
    and built-in popups on Alt+H/J/K/L without a key, one warning each, and
    `configuration_corpus_loads_from_the_migration_floor` loads
    `v0.1.9-focus-key`; proof record in the bead.
  - **Detach row (`eon-prove-close-and-detach-batch-bf8t`):** isolated
    installed dogfood (edge `22915af5dca41f87e706ad548c0506f79c17648f`,
    installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, private headless
    Sway, 1280x720): Alt+/ listed `Alt+Shift+D` `Detach: close the window,
    keep terminals running` under Host.
  - **Sidebar row (`eon-agent-sidebar-yx8.13`):** the installed dogfood
    recorded under Venus VEN-C30 (edge `d6ce117`, code tree `a0d664d`, whose
    `nix build` gave `/nix/store/p6mzlpdlk24k6v12plxcc99hyq9m1ghi-eon-0.1.1`,
    x86_64 Linux, private headless Sway, 1280x720): Alt+/ listed
    `Alt+Shift+H` `Show or hide the agent sidebar` under Host.
  - **Attention-key rows (`eon-agent-sidebar-yx8.8`):** the installed dogfood
    recorded under Venus VEN-C30 (edge `0e3912b`, code tree `bce96f9`, whose
    `nix build` gave `/nix/store/r18mya95p393gbd2jsmhmjw785cazk4g-eon-0.1.1`,
    x86_64 Linux, private headless Sway, 1280x720): Alt+/ listed `Alt+G`
    `Jump to the next agent needing attention` and `Alt+Shift+G` `Jump to the
    previous agent needing attention` under Navigate, also with
    `[sidebar] enabled = false`.
  - **Tab-cycling rows (`eon-ctrl-tab-next-tab-q6vj`):** the installed
    dogfood recorded under Venus VEN-C8 (edge `62c8cb5`, code tree `1c304b4`, whose `nix build` gave
    `/nix/store/wgrz0ladwg0r4c3addmjiqkijwah8900-eon-0.1.8`, x86_64 Linux,
    private headless Sway, 1280x720):
    Alt+/ listed `Ctrl+Tab` `Focus next tab` and `Ctrl+Shift+Tab` `Focus
    previous tab` under Navigate.
  - **Quit row (`eon-7cf0y.3`):** `quit_key_lists_hints_and_yields_to_a_popup`
    and the installed check recorded under EON-C8 (code tree
    `bceda181484faa57079edec69284981de748d7e0`, whose `nix build` gave
    `/nix/store/n907id3sg8sramk8sd2bb1qgaqqsvi44-eon-0.1.13`, x86_64 Linux,
    private headless Sway, 1280x720): Alt+/ listed
    `Alt+Shift+Q` `Quit: close the window, stop its terminals` under Host
    after Detach; the hints bar showed `Q quit` at 1280 px and dropped it at
    640 px; with `[popups.monitor] keybinding = "Alt+Shift+Q"` the key opened
    Monitor and the row read "(overridden by Monitor)".
- **Open proof:** Pending re-proof in this repository: header spacing at
  compact sizes, rebound and disabled popup rows, and AT-SPI projection on an
  installed surface. No eonbase, fractional-scale, screen-reader workflow,
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
  tab-region edge during overflow, Shortcuts when the bottom shortcut hints
  ([Eon Bar and tabs](USAGE.md#eon-bar-and-tabs)) need its fallback,
  and active-tab Close, which is available on the final tab and goes through
  the final-close prompt (EON-C8),
  shortcut tooltips, visible keyboard focus, AccessKit buttons, and bounded
  compositor-owned window dragging. The existing CreateTab and exact active
  CloseTab actions and local VEN-C20 viewer remain their sole behavior owners.
  The accessibility tree places New tab before the optional quota chip.
  While the agent sidebar is not visible, Venus badges each tab's EON-C31
  attention rollup under VEN-C21.
- **Important failures:** Stale component identity fails before activation.
  Rejected Eon actions never mutate another tab. Narrow surfaces retain the
  active tab and required controls without overlap. A failed native drag causes
  no workspace action, terminal input, crash, or state mutation. Controls, tab
  gaps, pane chrome, terminal content, popups, tooltips, and the viewer never
  become drag targets or leak a terminal pointer sequence.
- **Owner:** Eon owns the exact graph, product policy, profile delivery and
  installed acceptance; Eon Runtime owns workspace identity/actions. Venus
  owns header geometry, rendering, input, native drag, tooltips, and accessibility.
- **Consumes:** Venus VEN-C21, EONW v7 (v31 for the EON-C31 tab attention
  marker) and Orbit terminals.
- **Boundary:** EON-C27 separately owns the optional quota projection. The core
  header adds no Eon-rendered chrome, second row, widget framework, agent status,
  polling, configuration, protocol change, inactive-tab close, native window
  controls, macOS claim or distribution expansion. The one exception is the
  EON-C31 tab attention marker, which Venus reads from the EONW v31 snapshot it
  already polls.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** Venus
    `eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure`,
    `eon_bar_controls_reuse_existing_actions_and_local_viewer`,
    `eon_bar_buttons_share_scene_bounds_and_separate_focus_from_activation`,
    `tab_text_fits_shaped_width_without_splitting_combining_clusters` and
    `workspace_accessibility_order_matches_tabs_then_one_expanded_pane`.
    Installed checklist items 3 and 6.
  - **Final-tab close (`eon-prove-close-and-detach-batch-bf8t`):** isolated
    installed dogfood (edge `22915af5dca41f87e706ad548c0506f79c17648f`,
    installed `eon` profile
    `/nix/store/gfg385rm8mvdv6cfxqxnmc0bn033w7hv-eon-0.1.1`, private headless
    Sway, 1280x720): the bar's `×` on the only tab showed the final-close
    prompt; Escape kept the tab and Enter ended the window.
  - **Tab attention marker (`eon-agent-sidebar-yx8.2`):** isolated installed
    dogfood (edge `08e52ca`, the candidate's `nix build` of
    `/nix/store/i0j3p4bds3na8hha9j2f5f1gc828ga2v-eon-0.1.1`, private headless
    Sway at 1280x720 and 320x480): with the sidebar closed by Alt+Shift+H,
    tabs showed ◆, ✕, ✓ and a spinner for needs input, error, finished and
    working, the active tab included. A real Codex 0.163 turn in a background
    tab showed the spinner and then ✓. With the sidebar open the badges were
    hidden, and they returned when it hid for width.
- **Open proof:** Pending re-proof in this repository: real compositor window
  dragging, eight-tab overflow, narrow widths and AT-SPI order on an installed
  surface. AT-SPI tree order is not a screen-reader workflow; macOS and wider
  platform coverage remain open.

## EON-C22 — Trustworthy Codex quota in the Eon Bar

- **Status:** Retired 2026-10-14 by Lucca. EON-C27 owns Codex quota collection
  through CodexBar and the chip policy; Venus's Codex chip is described in
  [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar).
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
  `eon windows` discovers it and `eon window stop ID` stops only its
  terminals. `eon window attach ID` presents the window's live generation,
  whichever Eon build started it. When this `eon` cannot present it (another
  EONW version or component graph), the command runs the `eon` of the build
  that started it, as that generation recorded it, to present it. When no such
  record exists, it exits 1 and names the generation, the versions it
  reports, and that the build that started it can attach or stop it.
  `eon windows` shows each window's live generation, and lists a window with
  no live supervisor as ended instead of as the current generation.
  Each full-Eon window keeps a running-windows summary. `this` names the
  window: its window ID (null in the default namespace), generation, name, the
  Unix millisecond time its supervisor started, and its live terminals, which
  are the terminals Stop would end. `others` lists every other live full-Eon
  supervisor of this user under the same runtime directory, in either
  namespace, with the same fields and a `state`. A window's name is its shown
  workspace's name, else the last component of that workspace's active tab
  directory (`~` for the home directory), at most 64 characters. Characters
  excluded from pane-label plain text are replaced with U+FFFD before applying
  the limit. A supervisor from another EONW version, or one that does not
  answer, is listed as `incompatible` or `unreachable` with null name, start
  time and live terminals; a generation with no live supervisor is left out.
  `others` lists at most 32 supervisors, ordered by window ID (default
  namespace first) then generation, and `omitted` counts the rest of the
  inspected portion. Discovery considers at most 256 entries under `w/` in
  addition to the default namespace, skips unsafe or over-limit namespaces,
  and diagnoses these limits once. Supervisors outside that inspected portion
  are not counted in `others` or `omitted`. `others` is null until the
  window's first inspection completes. Identified eonbase supervisors are
  excluded. The summary refreshes after another supervisor starts, ends or
  dies, or another window's terminals start or end or its name changes. The
  control loop checks for changes once a second and each peer inspection has a
  one-second deadline, so among responsive windows the summary is normally
  current within a few seconds; a busy control loop or a large or unresponsive
  inventory can delay completion. Snapshots and CLI reads use the latest
  completed inspection. Keeping it current wakes no idle process and sends
  nothing to another supervisor while nothing changes, and background
  inspection never removes another generation's files. Venus receives the
  summary in every EONW v31 snapshot; the CLI prints it as `running`
  (EON-C10).
  `eon window stop all` covers independent windows and attempts the caller's
  own window last; existing `eon stop all` remains scoped to the caller's
  current runtime namespace. `eon quit --all` covers both (EON-C11). Piped
  confirmations remain available to later windows, one answer per prompt.
  With `--json`, `eon window stop all` asks for
  no confirmation and prints one JSON array holding one
  `{"window": ID, "results": [...]}` object per window, in stop order with the
  caller's own window last; `results` is that window's `eon stop all --json`
  result array. It exits with the highest status among those Stops.
- **Important failures:** Identity collision, unsafe runtime path, failed
  startup, or stale target neither replaces an existing window nor stops or
  retargets its terminals. An uncertain startup retains its ID, process, and
  private log for inspection instead of killing possible live terminals. A
  window whose Stop yields no result array is reported with a `stop-failed`
  failure in its `results`; the remaining windows are still stopped.
- **Owner:** Eon Runtime owns window identity, namespace and launch/lifecycle
  enforcement; Eon supplies product composition. Venus invokes the exact Eon
  executable supplied by its supervisor and presents one window per process;
  Orbit owns each terminal and its single presentation attachment.
- **Consumes:** EON-C1, EON-C8, EON-C11, EON-C15, EONW v7 (v28 for the running-windows summary), and VEN-C23.
- **Boundary:** Bare `eon` retains its current presentation behavior. Windows
  have independent workspaces; no terminal, tab, pane, or focus state is shared.
  The eight-hex-digit ID is unique by atomic directory reservation, not a
  secret. This does not establish macOS or another distribution channel.
- **Proof:** `795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, `eon-monorepo-2oko.6`.
  - **Environment:** Automated tier; installed tier.
  - **Evidence:** `new_windows_keep_independent_supervisors_and_stop_by_window_id`
    and Venus `new_window_shortcut_invokes_the_supplied_eon_command`. Installed
    checklist item 10.
- **JSON batch Stop proof:** `eon-igae.13`; that bead's commit is its revision.
  - **Environment:** Automated tier; isolated installed tier (private headless
    Sway, x86_64 Linux).
  - **Evidence:** The JSON batch in
    `new_windows_keep_independent_supervisors_and_stop_by_window_id`. Installed:
    `eon window stop all --json` stopped two windows with one result object
    each, then reported each stopped window with empty `results`; with no
    windows it printed `[]`. The `stop-failed` path has no automated check.
- **Reattach across builds proof:** `eon-reattach-after-upgrade-n5qv`; that
  bead's commit is its revision.
  - **Environment:** Automated tier; isolated installed tier (private headless
    Sway, x86_64 Linux), with a build of that revision (B1), B1 plus a
    comment-only change for a new generation (B2), and a pre-change EONW 18
    build.
  - **Evidence:** `window_attach_runs_the_build_that_started_its_generation`
    and the record assertions in
    `new_windows_keep_independent_supervisors_and_stop_by_window_id`.
    Installed: B1's window recorded B1's session-bin `eon`. After detach, B2
    listed the window's B1 generation as live, and `eon window attach ID` from
    B2 presented it again, as did the recorded `eon` run as `attach
    GENERATION`. A window from the EONW 18 build made B2's attach exit 1 naming
    the recovery. A stopped window was listed as ended and its attach exited 4.
- **Open proof:** Pending re-proof in this repository: the launcher's New Eon
  Window action, and native `eon window stop all` with piped confirmations
  stopping the caller's window last.

## EON-C24 — Bottom shortcut hints

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Eon Bar and tabs](USAGE.md#eon-bar-and-tabs); proof in `eon-monorepo-2oko.6`,
  `eon-igae.10`, `eon-prove-close-and-detach-batch-bf8t`,
  `eon-agent-sidebar-yx8.13`, `eon-agent-sidebar-yx8.8`,
  `eon-ctrl-tab-next-tab-q6vj`, `eon-vn0nk`.
- **Reason:** The footer hints are presentation. Their toggle and every other
  binding stay listed in the Alt+/ viewer under EON-C20, and popup precedence
  over the toggle key stays there too.

## EON-C25 — Running Eon version badge

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Eon Bar and tabs](USAGE.md#eon-bar-and-tabs); proof in `eon-monorepo-2oko.6`,
  `eon-vn0nk`.
- **Reason:** The version label is presentation. EON-C32 owns whether it shows
  and its place in the bar.

## EON-C26 — Passive Claude quota in the Eon Bar

- **Status:** Retired 2026-10-14 by Lucca. EON-C27 owns Claude quota collection
  through CodexBar and the chip policy; Venus's Claude chip is described in
  [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar).
- **Reason:** The status-line collector this contract described no longer
  exists, so it cannot be proved in this repository.

## EON-C27 — CodexBar quota collection

- **Status:** Proven on x86_64 Linux by the automated tier; installed quota
  observation is pending re-proof in this repository.
- **Consumer, trigger and result:** Full Eon reads quota through independent,
  noninteractive CodexBar CLI calls for Codex (`--source cli`) and Claude
  (`--source oauth`) only while `[bar] widgets` (EON-C32) lists at least one of
  that provider's quota widgets. The list is checked whenever a window opens or reopens: a
  newly listed provider starts reading, and a newly unlisted one stops, ending
  any read in progress, and drops its last reading. Codex reads start at least
  one minute apart and Claude reads at least five minutes apart, including
  after a failed read and across a stop and restart; reads run off workspace
  I/O. Claude is read once for the user across all Eon supervisors and
  windows: they share Claude's read schedule beside the stored readings, under
  a file lock, and a supervisor whose turn has not come shows the stored
  reading and the shared last failure instead of reading. After a read that
  fails with `provider_error`, as a rate-limited read does, the next Claude
  read waits 30 minutes, then one hour, then two hours for each further such
  failure with no successful read between; a successful read restores the
  five-minute spacing. A schedule that is unreadable, or set further ahead
  than two hours, is ignored, and a supervisor that cannot read, lock or write
  it keeps its own schedule. Codex reads stay per supervisor and never back
  off.
  The native chips show each selected supplied supported window and its remaining
  percentage, one chip per window, in `[bar] widgets` order
  ([Eon Bar](../venus/docs/PRESENTATION.md#eon-bar)), with provider periods,
  optional resets and observation time. Claude's model-scoped weekly windows,
  the CodexBar `extraRateWindows` entries whose `id` starts with
  `claude-weekly-scoped-` and whose usage is known, are windows too, after the
  unscoped ones, validated as the others are. Each carries its scope: the
  entry's `title` without CodexBar's trailing ` only` (`Fable` for `Fable
  only`), cut to at most 32 bytes at a character boundary. A scoped window
  whose scope is empty or holds control characters, and any past the fourth,
  is left out without failing the reading. A scope whose repeats differ is left
  out the same way. Identical repeats count once, and a scoped window with an
  invalid value or unsupported duration is left out without failing the
  reading. Other extra windows are not read.
  Claude needs no status-line setup. No reset is invented for an idle window.
- **Ownership:** Pinned CodexBar owns provider collection and credential access;
  Runtime owns bounded subprocesses, validation, freshness and the stored
  readings; canonical EONW permits absent resets; Venus consumes typed facts
  through its existing chip owners. Eon owns exact Nix composition. EONW v31
  carries each window's scope and each provider's last failure.
- **Failure policy:** Each read has a ten-second deadline and 64 KiB output cap.
  A failed read keeps retained data unchanged; age alone marks it old after one
  hour. Readings disappear after 24 hours and windows at their reset.
  Successful missing quota clears prior facts. Invalid, future, estimated or
  unsupported facts never become zero usage. Provider failures are independent.
  Retained data outlives the supervisor: each successful read replaces one
  stored reading per provider, shared by every Eon generation of the user, and
  a provider that starts reading shows that stored reading until its own read
  succeeds. A stored reading that is unreadable, malformed, dated in the future
  or in another format version is ignored, never shown as zero usage.
  While Runtime reads a provider, it keeps that provider's last failed read in
  memory: its wall-clock end and one reason, `start` (CodexBar could not be
  started or read), `deadline`, `output_limit`, `exit_status`, `malformed`
  (not one result of the expected JSON shape), `provider_error` (another
  provider or source, or a reported error, also on a nonzero CodexBar exit
  whose one result carries it), `estimated`, `observation_time`
  (missing, invalid, future or over an hour old) or `invalid_window` (a window
  value, duration, count or conflicting repeat). The next successful read,
  including one that finds no quota, clears it, as unlisting the provider does.
  It carries no CodexBar output and never enters the stored readings; Claude's
  is shared with its read schedule.
- **Boundary:** One selected account per provider, Linux Wayland and Nix only.
  Explicit sources avoid browser fallback, interactive login and model turns.
  No credential, identity, raw output or billing data enters EONW, Eon logs or
  the stored readings.
  Sign-out is not detected: CodexBar reports it as a failed read, so the last
  reading ages like any other (Lucca, 2026-10-16). Blocked/unknown labels are
  not guaranteed.
  CodexBar owns its local authentication/cache mechanisms. Eon does not rewrite
  provider configuration. Eon generations older than this schedule keep
  reading Claude on their own.
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
    `eonbase-closure` check keeps CodexBar out of eonbase. The failure-policy
    change (`eon-quota-old-after-hour-knaq`) is proved by the updated
    `retained_observations_age_and_expire_without_invented_resets` and
    `workers_isolate_failure_and_bound_output_timeout_and_shutdown`; that
    bead's commit is its revision. The Claude cadence change (`eon-flf4`) is
    proved by `claude_waits_300_units_after_a_failed_read_while_codex_reads_every_60`;
    that bead's commit is its revision. Reading only the providers whose chips
    `[bar] widgets` lists (`eon-igae.14.3`) is proved by the same test, extended
    to hide and show chips (a chip shown again keeps its provider's spacing),
    and by one isolated installed observation recorded in that bead: a hidden
    provider is never called, a provider first shown on reopen reads at once,
    and hiding every chip stops every read; that bead's commit is its revision.
    Stored readings that outlive the supervisor
    (`eon-eon-quota-survives-restart-dfof`) are proved by
    `stored_readings_outlive_the_supervisor_and_bad_ones_are_ignored` and by
    one isolated installed observation recorded in that bead: a supervisor
    whose every read fails with Claude's rate-limit error shows the Codex and
    Claude readings an earlier supervisor stored; that bead's commit is its
    revision.
    Claude's scoped windows (`eon-wqkjc`) and the last failure
    (`eon-d0kyj`) are proved by the extended
    `actual_periods_optional_resets_and_missing_quota_are_preserved`,
    `invalid_source_values_and_timestamps_never_become_fresh_usage`,
    `workers_isolate_failure_and_bound_output_timeout_and_shutdown` and
    `stored_readings_outlive_the_supervisor_and_bad_ones_are_ignored`, the
    EONW v27 codec tests,
    `cli_json_carries_the_full_attention_record_and_quotas_but_human_output_does_not`
    and Venus's
    `two_provider_chips_share_layout_without_losing_identity_or_freshness`;
    those beads' commit is their revision. Reading Claude once per user with
    backoff (`eon-agsni`) is proved by
    `claude_is_read_once_across_supervisors_and_backs_off_after_provider_errors`,
    `claude_peers_adopt_newer_readings_and_clear_only_successful_missing_quota`,
    `invalid_claude_schedules_allow_an_immediate_read`,
    `claude_schedule_preserves_reasons_bounds_and_backoff`,
    `failed_schedule_rewrites_keep_the_in_memory_backoff`,
    `claude_provider_errors_back_off_without_a_usable_schedule` and the
    extended `workers_isolate_failure_and_bound_output_timeout_and_shutdown`;
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

## EON-C28 — 1.x stability

- **Status:** Planned (`eon-igae.1`, approved by Lucca 2026-10-20). It binds
  from v1.0.0; 0.x releases make no stability promise. Amended on 2026-10-31
  (approved by Lucca, relayed by eon-overseer) by `eon-c8ytm`: delegated
  lifecycle for older generations, newer list and trust values, and no
  rewrite of `config.toml`.
- **Consumer:** People, scripts and agents that use Eon 1.0 on x86_64 Linux on
  native Wayland and upgrade within 1.x.
- **Trigger:** Installing a later 1.x release over 1.0 or another 1.x release.
- **Result:**
  - **Commands:** every `eon` command and flag that `eon --help` or
    [Using Eon](USAGE.md) lists at 1.0 keeps its name, arguments and meaning.
    Every `eon-` managed command name that [Configuration](CONFIGURATION.md)
    documents, such as `eon-nu`, `eon-bash`, `eon-hx`, `eon-yazi` and
    `eon-lazygit`, keeps its name. Later releases may add commands and flags.
  - **Exit statuses:** success stays 0, and each documented failure status
    (`eon-8mnr.1`) keeps its meaning, the same with and without `--json`.
  - **JSON output:** every field that a listed command's `--json` output
    prints at 1.0 keeps its name, type and meaning; the 1.0 JSON fixture
    (`eon-igae.11`) is the documented record of those fields. Later releases
    may add fields and values; consumers ignore what they don't know.
  - **Configuration:** a `config.toml` under `eon config-dir` that loads in
    1.0 loads in every 1.x with the same meaning, and the documented
    `EON_CONFIG_HOME` and `EON_RUNTIME_DIR` variables keep theirs. A setting
    deprecated in 1.x still loads and warns (the first deprecation adds the
    warning); none is removed before 2.0. Eon never rewrites `config.toml` on
    its own; 2.0 ships `eon config migrate` for 1.x files. A built-in popup
    added in 1.x never changes the meaning of a configured `[popups.<id>]`
    with the same ID. Appearance means the `[bar]`, `[terminal]` and `[anima]`
    settings; they and their 1.0 values keep loading, though how they render
    may change. Every other default changes only with a changelog entry and a
    setting that restores the old value.
  - **Shortcuts:** every default shortcut the Alt+/ viewer lists at 1.0 keeps
    its action. A later default never makes a 1.0 configuration fail: a
    shortcut the user configured takes precedence over a default on the same
    key (`eon-igae.10` for fixed shortcuts and `eon-configurability-qkq8.1`
    for built-in popups, required before 1.0), except the reserved keys
    [EON-C20](#eon-c20--discoverable-native-shortcuts) names (Alt+Slash and
    Alt+0 through Alt+9, and Alt+H, Alt+J, Alt+K and Alt+L), which 1.x never
    enlarges.
  - **Upgrades:** installing a later 1.x never stops live terminals. The new
    `eon` lists and stops every older 1.x generation through
    [EON-C11](#eon-c11--runtime-generations-and-presentation), with its own
    codec or by running the generation's recorded `eon`.
    `eon attach GENERATION` presents one only when its presentation is
    compatible, which 1.x does not guarantee.
- **Important failures:** A 1.x release that rejects a 1.0 configuration,
  renames or removes a listed command, managed command name, flag, JSON field
  or environment variable, changes a documented exit status or a default
  shortcut's action, drops a 1.x lifecycle codec, or stops live terminals on
  install breaks this contract; such a change waits for 2.0. Unknown
  configuration keys are ignored with a warning naming the key and file, so
  typos stay visible. In a configuration written for a newer 1.x, an older
  release skips list values it does not know, with a warning; a trust setting
  with an unknown value takes its most restrictive choice; any other unknown
  value of a known key may still fail.
- **Owner:** Eon owns the promise, the release review against it and the
  managed command names its Nix composition installs. Eon Runtime owns the
  commands, JSON, exit statuses, lifecycle codecs and configuration parsing
  that carry it; Venus owns its fixed workspace bindings under EON-C20.
- **Boundary:** Not covered: how appearance and layout render; behavior that
  no document states; the ORBS and ORBF protocols, EONW beyond the lifecycle
  codecs kept above, and the component manifest format, which carry their own
  versions; internal helpers that no document lists; the unprefixed tool names
  inside terminals; Anima's styles and options; eonbase; the behavior of pinned
  tools such as Helix, Yazi and Nushell when their pins update; reattaching
  across presentation-incompatible releases, which needs the earlier release's
  executable (EON-C11); and every platform other than x86_64 Linux on native
  Wayland, including the macOS preview.
- **Proof:** None yet.
- **Open proof:** Before v1.0.0, `eon-igae.11` adds these fixtures to the
  workspace suite (`nix develop -c eon-test`), so every 1.x build runs them: a
  configuration setting every 1.0 key, loaded with the documented environment
  variables; a parse corpus of every listed 1.0 command and flag; the 1.0
  fields of each `--json` output; the documented exit status of each failure
  class; and the 1.0 default shortcut actions, with one configured override.
  It also extends the `managed-command-path` flake check to every documented
  `eon-` managed command name. Each later EONW revision adds its predecessor
  to the previous-generation Stop test. Each 1.x release's proof adds a
  review of changed defaults against the changelog and one installed upgrade
  observation in which the previous release's live terminals keep running and
  the new release lists and stops them.

## EON-C29 — Local clock in the Eon Bar

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Eon Bar and tabs](USAGE.md#eon-bar-and-tabs); proof in `eon-igae.4`,
  `eon-vn0nk`.
- **Reason:** The clock is presentation. EON-C32 owns whether it shows and its
  place in the bar.

## EON-C30 — CPU and memory use in the Eon Bar

- **Status:** Retired 2026-10-30 by Lucca: demoted to a feature; behavior in
  [Eon Bar and tabs](USAGE.md#eon-bar-and-tabs); proof in
  `eon-cpu-widget-eon-bar-acxs`, `eon-igae.3`, `eon-vn0nk`.
- **Reason:** The CPU and RAM labels are presentation. EON-C32 owns whether they
  show and their place in the bar; the proof record keeps the 2026-10-27 macOS
  amendment.

## EON-C31 — Agent attention

- **Status:** Proven on x86_64 Linux Wayland. Approved by Lucca on 2026-10-21
  (draft 15 on `eon-agent-sidebar-yx8.1`, which records decisions D1–D11 and
  Q1–Q8, every Q with its recommendation). `eon-agent-sidebar-yx8.6`
  implements the Runtime model; `eon-agent-sidebar-yx8.7` carries it over
  EONW; `eon-agent-sidebar-yx8.9` proves it on the installed product.
  Amended on 2026-10-27 (approved by Lucca, relayed by eon-overseer) by
  `eon-eon-attention-working-hides-finish-7df0`: newer work sees a finish.
  Amended on 2026-10-27 (approved by Lucca, relayed by eon-overseer) by
  `eon-agent-background-work-yte0`: waiting on background work.
  Amended on 2026-10-29 (approved by Lucca, relayed by eon-overseer) by
  `eon-agent-parity-a9g0.2`: the full record and quotas in CLI JSON.
  Amended on 2026-10-29 (approved by Lucca, relayed by eon-overseer) by
  `eon-ngwf.1`: the queue in order over EONW v25 and in CLI JSON.
  Amended on 2026-10-29 (approved by Lucca, relayed by eon-overseer) by
  `eon-ngwf.2`: `FocusAttention` across workspaces.
  Amended on 2026-11-03 (approved by Lucca, relayed by eon-overseer) by
  `eon-ciwn.1`: EON-C39 plans a stream of its changes.
- **Consumer:** One person supervising coding agents across the tabs and workspaces (EON-C33)
  of one full-Eon window.
  eonbase has no poller and no sidebar; its `sidebar` is `Off`.
- **Trigger:**
  - the attention poller's timer fires;
  - focus moves to a pane or popup;
  - a Venus attaches;
  - an attention action arrives: `FocusAttention`, `NextAttention`, `PreviousAttention` or
    `ToggleSidebar`.
- **Result:**
  - **Polling.** A Runtime attention poller runs off the request path, on its own timer, at no
    more than 2 Hz.
    - It polls only pane and popup terminals, of every workspace of the window,
      never a pending tab's pair.
    - It polls peers concurrently, each with a 250 ms deadline.
    - Each lease has exactly one reader and at most one request in flight. The EOF probe and
      Stop first drain or await an in-flight `ProgramStatus` reply, so no reply is ever read as
      an unsolicited or stop result.
    - Replies are tagged by type, so a late reply can never be mistaken for a Stop result.
    - A slow poll never drops or replaces a lease.
    - A Stop or EOF probe that waits for an in-flight reply is delayed by Orbit's answer time:
      normally well under 250 ms, and at most two worker exchanges plus host turn scheduling.
      There is no new failure path. `eon-agent-sidebar-yx8.10` asserts this under PTY load.
    - A Busy refusal counts as a missed poll deadline.
    - A peer that misses the poll deadline only goes stale. Its lease stays as it is.
    - A lease with a Stop pending is not polled.
    - Inspect and actions read only the poller's latest cached result and never wait on a peer.
  - **Stale.** A missed deadline keeps the last good result, which avoids flicker.
    - Once a terminal's last good result is older than 3 s, its attention is marked stale. It is
      shown in its last state, dimmed, with "status stale" in its accessible name, and it stays
      in the Alt+G queue.
    - Stale clears on the next good reply.
    - When the run ends or the target is removed, the attention is removed.
  - **Identity and time.** State is keyed by the exact Orbit run ID plus record `id` and creation
    sequence (ORB-C15), so a record cleared and reported again is a new record and inherits no
    cached raise.
    - A new run starts empty.
    - A reply whose update sequence is lower than the cached one for the same run is discarded.
    - On first sight of a (run, `id`, change sequence), the Runtime fixes the change time on its
      own monotonic clock: receipt time minus the change age. Polls never move that time, so
      queue keys and row ages stay stable across polls, and ages from different terminals share
      one clock.
  - **Raise.** Attention meaning is derived here, not in Orbit. A record *raises* at its change
    sequence in these cases:
    - it changes into `done` or `error`, including being created in them;
    - it changes into `idle` from `working`, as D4 decided.

    An `idle` from `blocked` (for example, a declined permission prompt) does not raise **[Q6]**.
    - An `idle` from `done` keeps the cached raise of that `done`. If the Runtime never saw the
      `done`, the `idle` raises at its own change sequence.
    - An `idle` from `error` does not raise: the program has moved on.
    - Same-state re-reports never raise.
    - An `idle` raised from `working` covers Codex 0.163, which has no `done`. It also counts an
      interrupted turn as finished, which D4 accepted.
  - **Normalization.** Each record maps to one state, in this priority order:

    | State | Source |
    | --- | --- |
    | needs input | `blocked`, with its `kind` |
    | error | `error` whose raise is unseen |
    | finished | `done`, or a raised `idle`, whose raise is unseen |
    | working | `working`, with `progress` |
    | background | `done` or `idle` whose terminal's reply counts background processes (ORB-C15) |
    | idle | any other `idle`; or `done`/`error`/raised `idle` whose raise has been seen |
    | unavailable | an Agent popup with no records |

    Background takes precedence over finished and idle for that record: while the count is
    above 0 the record maps to background, not finished or idle.

  - **Seen.** Seen is keyed by (run, `id`, raise). A raise becomes seen when a Venus is attached
    and its exact terminal is the focused target (the selected pane of the shown workspace's
    active tab, or its shown popup) at one of these moments:
    - focus arrives on that terminal, including via `FocusAttention` to a terminal that
      already has focus, or when its workspace is shown with that terminal focused;
    - a new raise arrives while that terminal is focused;
    - a Venus attaches while that terminal is focused.

    A focus action applies seen to the raise cached when the action was handled, and a poll in
    flight cannot undo it. Needs input is never cleared by viewing. A terminal in a hidden
    workspace is never the focused target, so focus never sees its raises until its workspace
    is shown and that exact terminal has focus.

    Newer work also sees a finish, with or without a Venus attached and in any workspace. When
    a `working` record in a terminal's reply changed later, by ORB-C15 change sequence, than
    the raise of a finished record of the same terminal, that raise is seen: it no longer
    shows, and it does not return when the work stops. An `idle` that keeps its `done`'s raise
    compares at that `done`. A finish raised after the work's last change still shows
    finished. An error and needs input are never seen this way.

    Background holds a raise: it neither sees nor clears it, and focus sees it as usual. When
    the count returns to 0, the record maps as before, so an unseen finish shows finished unless
    newer work saw it. Claude Code goes `working` when its background task ends, so its next
    `done` raises a fresh finish.
  - **Rollup.** A tab's rollup is the highest-priority state among its terminals, and a
    workspace's rollup the highest among all its terminals (EON-C33), which EONW v22 carries
    as `attention` on each `WorkspaceInfo`. A stale terminal counts with its last state, and
    its badge is dimmed.
  - **Sidebar visibility [Q3].** The Runtime keeps one value per window: `Open` (the default),
    `Closed`, or `Off` (when `[sidebar] enabled = false`).
    - The value lives in memory while the supervisor runs, so it survives detach and reattach.
      It resets to `Open` after a supervisor restart.
    - `ToggleSidebar` flips `Open` and `Closed`. A toggle, rather than a set, cannot race a stale
      snapshot. In `Off` it is a no-op.
  - **EONW v18 and v19** (amend EON-C10) add:
    - an optional `attention` on `Pane` and `Popup`, holding:
      - `state`: the *normalized* EON-C31 state of the terminal's most urgent record, one of
        needs input, error, finished, working, background, idle or unavailable. EONW v23
        adds background. Venus never normalizes, and seen is already folded into this state;
      - from that record: kind, progress, `app`, `title` and `msg`;
      - `name`, which EONW v19 adds: the terminal title from the same reply (ORB-C15), passed
        through unchanged, so rows can tell agents apart. Claude Code sets it to its session
        name (`--name` or `/rename`), else to its session title. It is empty when the terminal
        has none;
      - its `changed_at`, as wall-clock Unix milliseconds. The Runtime computes it once from its
        fixed change time, so the snapshot does not change as time passes, and Venus computes
        the age shown;
      - the stale flag;
      - the count of other records.

      For unavailable, only `state` is present.

      Its bounds are those of the ORB-C15 reply: `app` ≤ 32, `name` ≤ 64, `title` ≤ 96 and
      `msg` ≤ 256 bytes. A whole snapshot still fits the 2 MiB frame.
    - `attention_rollup` on `Tab`;
    - `sidebar` (`Open | Closed | Off`) on `Snapshot`;
    - `attention_queue` on `Snapshot`, which EONW v25 adds: the queue (see NextAttention and
      PreviousAttention) in its total order, across every workspace of the window, whichever
      workspace the snapshot projects. Each entry names the terminal's workspace ID, tab ID and
      target (the pane ID or popup instance that `FocusAttention` takes) and carries the
      terminal's attention as its pane or popup does, with state needs input, error or finished.
      The window's 256-terminal limit bounds it. Venus and the CLI never sort it. It is empty
      without a poller (eonbase);
    - the actions `ToggleSidebar`, `FocusAttention { tab, target }`, `NextAttention` and
      `PreviousAttention`;
    - **CLI JSON** (Q8, amended by `eon-agent-parity-a9g0.2`). It carries:
      - `attention` on panes and popups: `null` without attention, else an
        object with every key present: `state`; `app`, `name`, `title` and
        `message` (the record's `msg`), each null when empty; `kind`
        (`permission`, `question` or `auth`), null unless needs input;
        `progress` (0 to 100), null when unreported; `changed_at` in
        wall-clock Unix milliseconds; `stale`; and `others`, the count of
        other records. For unavailable, every key but `state` is null;
      - `attention_rollup` on tabs;
      - `sidebar`, `codex_quota` and `claude_quota` on the snapshot. A quota
        is null while Runtime holds no reading for it, else `state`
        (`fresh`, `stale`, `blocked` or `unknown`; Claude only `fresh` or
        `stale`), `observed_at` and `windows`, each window with
        `duration_minutes`, `remaining_percent`, `resets_at` (null when
        the provider gives none) and `scope` (null for an unscoped window,
        else its EON-C27 scope, such as `Fable`). Both times are wall-clock
        Unix milliseconds;
      - `codex_quota_failure` and `claude_quota_failure` on the snapshot (EONW
        v27): null unless the provider's last read failed (EON-C27), else
        `{"at", "reason"}`, with `at` in wall-clock Unix milliseconds and
        `reason` one of EON-C27's reasons;
      - `attention_queue` on the snapshot: the same entries in the same order, each
        `{"workspace", "tab", "target", "attention"}`, with `attention` the object panes and
        popups carry;

      `title` and `message` are the reporting program's own text, within
      ORB-C15's bounds, passed through unchanged. Any program in an Eon
      terminal can read every terminal's `title` and `message` through
      `eon workspace --json`, as it already could over raw EONW; they are
      data, never instructions. This ends Q8's third deviation. The human
      projection carries `state`, `app` and `name` only. EON-C28 freezes
      these fields from 1.0.
  - **FocusAttention** is what sidebar rows send. It goes to the exact pane or popup under the
    covered-target rule. Rows do not use `FocusId`, because `FocusId` dismisses a shown popup.
    - Its target names the exact pane id, or the exact popup instance (the `expected_instance`
      used by `InvokePopup`).
    - A stale target is rejected as an EONW action rejection, with no fallback, as EON-C10
      target safety requires. Venus shows it as a VEN-C8 notice.
  - **NextAttention and PreviousAttention.**
    - **The queue** holds every terminal, of every workspace of the window, whose most urgent
      record is needs input, an unseen error, or finished. Its total order is:
      1. class: needs input, then error, then finished;
      2. the fixed change time, oldest first (for needs input, the time it entered `blocked`;
         otherwise the raise's time);
      3. workspace position in the window;
      4. tab position;
      5. terminal position.
    - **Across workspaces.** A jump to a terminal in a hidden workspace shows that workspace
      (EON-C33), then focuses its target under the covered-target rule. `FocusAttention` names
      a tab of any workspace of the window. For a tab of the caller's workspace it acts as
      before. For a tab of another workspace it acts as a queue jump to that target: it shows
      that workspace, then focuses the target under the covered-target rule, and a stale
      target changes nothing. Unlike a queue jump, it resets the cursor, as other focus moves
      do.
    - **The cursor.** The Runtime keeps one cursor per window: the order key of the last jump
      target.
      - `NextAttention` goes to the first entry whose key is greater than the cursor, wrapping
        to the smallest key. `PreviousAttention` does the reverse.
      - The cursor advances even when the jump only selected a tab.
      - The cursor keeps working after its entry leaves the queue, so repeated presses walk
        every entry.
      - **Reset [Q7].** The cursor resets, so the next press starts at the front of the queue,
        in two cases:
        - focus moves by any other means;
        - an entry of a higher class than the cursor's appears, for example a needs input
          while the cursor sits on a finished entry.

        This keeps D9's rule that the most urgent work comes first.
    - Seen raises leave the queue.
    - On an empty queue, both actions are rejected with the EONW action failure `no-attention`,
      detail `No agent needs attention`. Focus, selection and the cursor are unchanged, and no
      Orbit state changes.
  - **Covered-target rule.** It applies to `FocusAttention` and both queue actions, for pane and
    popup targets alike. If the target's tab is showing a popup that is not the target:
    - a keep-alive popup is hidden (it survives), and then the target is focused or shown;
    - a transient popup (one whose dismissal stops its terminal, EON-C10), or a
      pending directory picker, is left alone. The action only selects the tab and never stops a
      terminal.
  - **Configuration.** `[sidebar] enabled = true` is the default. Setting it to `false` sets
    `sidebar` to `Off`:
    - Venus shows no sidebar;
    - Alt+Shift+H is unbound;
    - tab markers always show.
- **Important failures:**
  - **No attention.** Before its first good reply, a peer gives no attention. A terminal without
    records also gives none, except an Agent popup, which gives unavailable, including before its
    first good reply. After the first good
    reply, slowness only makes a peer stale. The poller never guesses a state.
  - **Failures stay contained.** A poll failure never blocks Inspect or actions. It delays Stop
    by at most Orbit's answer time for one in-flight request (see Polling).
  - **Removed targets.** A removed pane or popup's records disappear with it.
  - **Restarts.**
    - A supervisor restart or recovery loses the seen bits and visibility, so `done` and `error`
      records reappear unseen. This is accepted for 1.0.
    - A provider that restarts in the same pane replaces its records with new reports.
  - **Leftover idle records.** An agent killed by SIGKILL in a shell pane can leave an `idle`
    record, which shows as an idle row until the run ends or the record is replaced. The spec
    allows dropping `idle` on exit, but keeping it lets a finish raise. This is accepted for 1.0.
    If that shell then runs jobs of its own in the background, such as `sleep 100 &`, they count
    as background processes (ORB-C15), so the row shows background until they end.
  - **Finishes that vanish with their record.** An unseen finish disappears with its record in
    these cases:
    - Claude Code clears its records on exit, and Codex clears its record on exit, on suspend,
      and when it hands the terminal to another program such as an editor;
    - in the Agent popup the agent is the PTY child, so its exit ends the Orbit run, and its
      `done` and `error` records end with the run, even though the spec says they survive
      process exit;
    - the ProgramStatus reply holds 7 records at their bounds, more when they are shorter, and as
      few as 3 when a terminal holds many, so a terminal with as many `blocked`, `error` or `done`
      records as fit (Claude subagents, for example) can crowd out a raised `idle` from the reply.
      That finish never shows.

    All three are accepted for 1.0. In the first two the user is usually interacting with that
    terminal; the third needs an unusual number of records.
  - **A cycle between two polls.** A record that goes `working`, `done` and `idle` between two
    polls arrives as an `idle` from `done` and keeps the cached `done`'s raise. So the finish
    of that cycle does not show when the cached raise was already seen, or when a `working`
    record changed after the cached `done`. Polls at 2 Hz make this rare. This is accepted
    for 1.0.
  - **Background work Orbit cannot count.** Work that leaves the process tree (a double fork,
    Claude Code's "Move to background and exit") or that runs no process (Claude Code's
    ScheduleWakeup and cron waits) is not counted, so its terminal shows finished or idle. A
    helper that lives past 1 s outside the agent's process group counts as background work;
    none was found in Claude Code 2.1.303 with Lucca's configuration. This is accepted for 1.0.
  - **Window focus.** Eon does not know desktop window focus, so a raise in the focused terminal
    counts as seen even while the window sits behind another app. This is accepted under D5.
  - **Detached windows.** While no Venus is attached, only newer work makes a raise seen. The
    supervisor knows this because it holds the control stream (VEN-C14).
  - **Older generations.** The new `eon` treats a still-running EONW 17 generation as
    incompatible, exactly as it has since eon-igae.12.4. Nothing changes here.
    `eon-agent-sidebar-yx8.7`'s rollout follows igae.12.4's precedent.
  - **No side effects.** Status changes never change focus, open the sidebar, raise the window or
    send terminal input.
- **Owner:**
  - **Eon Runtime** owns the poller, raise, normalization, seen, rollup, the queue and cursor,
    sidebar visibility and `[sidebar]` configuration.
  - **Orbit** owns the records, their neutral facts and the terminal title.
  - **Venus** owns presentation and hiding for width (VEN-C30).
- **Consumes:** ORB-C15, ORB-C13 generation 5, EON-C8 topology, and EON-C10
  (EONW v31, request and response). EON-C39 plans a stream of its changes.
- **Boundary:**
  - no hooks and no edits to agent configuration;
  - no provider parsing; the Runtime passes `app` and `name` through unchanged, and Venus maps
    `app` for display (VEN-C30);
  - no notifications or sound (eon-iwmz consumes this later);
  - no agent launching, orchestration or worktrees;
  - agents under tmux report nothing; this is documented.
- **Proof:** `eon-agent-sidebar-yx8.9`
  - **Environment:** Automated tier at the commits below; isolated installed
    dogfood on x86_64 Linux of edge `c8c62cb63938b145b13cfa4fe161ea5689807652`,
    installed `eon` profile
    `/nix/store/m5mzxy8cikz3f6y7k0ldgf7qxmc64bji-eon-0.1.1` (EONW 19),
    private headless Sway 1.12 at 1280x720, with Claude Code 2.1.303,
    Codex 0.164.0 and Claude Code 2.1.293.
  - **Evidence:**
    - **Automated:** the Runtime tests that `e87b9f8`
      (`eon-agent-sidebar-yx8.6`) added in
      `runtime/crates/eon-runtime/src/attention.rs`, whose bead records their
      `nix develop` run, among them
      `identity_fixes_change_times_and_discards_older_replies_until_a_new_run`,
      `stale_results_keep_their_state_and_only_agent_popups_start_unavailable`,
      `seen_needs_an_attached_venus_and_the_exact_focused_terminal` and
      `the_queue_walks_its_total_order_and_its_cursor_resets` and
      `raises_follow_the_changes_eon_c31_names`, which alone covers Q6 (an
      `idle` from `blocked` does not raise).
      `attention_focus_hides_keep_alive_popups_and_never_stops_a_transient_one`
      in `runtime/crates/eon-runtime/src/workspace.rs`, added by `4b150d5`
      (`eon-agent-sidebar-yx8.7`, whose `nix develop -c eon-test` run its bead
      records), covers a transient popup over the target, which the installed
      run did not use.
      `[sidebar] enabled = false` was observed installed under
      `eon-agent-sidebar-yx8.8` (VEN-C30).
    - **Installed:** states come from `eon workspace` dumps, the stale row from
      captures, focus from Sway tree samples and liveness from process lists.
      - A real Claude Code and a real Codex each went from working to needs
        input to finished. Both finishes stayed unseen until visited.
      - After its first finish, Claude's own dialog read needs input.
      - A pane whose program never reports had no attention (`null`).
      - A burst of working, blocked, working and done sent back to back
        showed only finished.
      - A record cleared and reported again after its finish was seen showed
        finished again.
      - An Agent popup running Claude Code 2.1.293 read unavailable.
      - Stale. 5 s after its Orbit host was stopped:
        - the terminal's row was dimmed with "status stale" and kept its last
          state, needs input;
        - Alt+G still jumped to it;
        - it recovered when the host resumed.

        The 3 s threshold itself is covered only by the automated tests.
      - Alt+G from a quiet tab:
        - It visited needs input, then the three finishes oldest first across
          three tabs, then wrapped.
        - Each visited finish became idle.
        - On an empty queue, Alt+G and Alt+Shift+G changed nothing.
        - With a keep-alive Agent popup covering the target, the popup was
          hidden and kept running, and the target pane stayed the tab's
          selected pane.
      - Alt+Shift+H toggled `Open` and `Closed`.
      - No side effects. While a foot window held desktop focus on another
        Sway workspace, Claude, Codex and the fixtures changed attention.
        Every focus sample stayed on foot, and none showed an urgent window.
        Later, with the sidebar closed, a new needs input left the sidebar
        closed, the active tab unchanged and desktop focus on Eon.
      - At the last process check every terminal, agent and popup was still
        running, and the later tab-6 pane read `live=true`.
      - `eon workspace --json` carried only `state`, `app` and `name`.
  - **Amendment** (`eon-eon-attention-working-hides-finish-7df0`, newer work sees a finish):
    the commit that carries this record is its revision.
    - **Automated:** `newer_work_sees_an_older_finish_but_never_an_error` in
      `runtime/crates/eon-runtime/src/attention.rs`, on the record shapes Claude Code 2.1.303
      was observed to report, within `nix develop -c eon-test -p eon-runtime` (126 passed,
      1 ignored). It fails when an `idle` that keeps its `done`'s raise compares at its own
      change.
    - **Installed:** private headless Sway 1.12 (pixman, 1600x900) on x86_64 Linux. The
      candidate `/nix/store/wvsqcg8q7z2w3kn5xr57wspyzslrrpz9-eon-0.1.5`, built from this
      commit's tree before this record was added, was run against the earlier profile build
      `/nix/store/77w9f35fj1p8a9hz89j5wdcbii2b8bhc-eon-0.1.5`. A tab-1 pane reported a
      session record `working`, then `done`, then a subagent record `working` under its own
      `id`, then that id's clear, while tab 2 held focus. `eon workspace --json` read
      `finished`, `working`, then `idle` on the candidate, and `finished` throughout on the
      earlier build. The candidate's capture showed the spinner row with `+1` without a
      click.
  - **Amendment** (`eon-agent-background-work-yte0`, waiting on background work): the Runtime
    tests that `eon-agent-background-work-yte0.2` recorded, and the installed observation of
    `eon-agent-background-work-yte0.3`, whose commit carries this record.
    - **Installed:** private headless Sway 1.12 (pixman, 1600x900) on x86_64 Linux, the
      candidate `/nix/store/4pw8bqg1vrdafkr2mpa90mssmcdaqk9b-eon-0.1.6`, built from this
      commit's tree on edge `8c42c73` before this record was added. In tab 1, Claude Code
      2.1.303 ran a background `sleep 25` and reported `done` while it ran; tab 2 held focus.
      `eon workspace --json` read `background` for tab 1's pane in every sample for 14 s,
      then `finished` once the sleep ended, without a visit. A capture between the two
      samples showed the working badge while Claude Code reported the ended task, and the
      finish raised after it stayed unseen.
  - **Amendment** (`eon-agent-parity-a9g0.2`, the full record and quotas in CLI JSON): the
    commit that carries this record is its revision.
    - **Automated:**
      `cli_json_carries_the_full_attention_record_and_quotas_but_human_output_does_not` in
      `runtime/crates/eon-runtime/src/workspace.rs`, within `nix develop -c eon-test -p
      eon-runtime` (136 passed, 1 ignored) on x86_64 Linux. It fails on the earlier
      three-field projection.
  - **Amendment** (`eon-ngwf.1`, the queue in order over EONW v25 and in CLI JSON): the
    commit that carries this record is its revision.
    - **Automated:** `the_snapshot_carries_the_queue_in_its_total_order_across_workspaces` in
      `runtime/crates/eon-runtime/src/attention.rs` (needs input, error and finished terminals
      across two workspaces, the same queue from either workspace's snapshot, and Alt+G
      walking it), the extended
      `cli_json_carries_the_full_attention_record_and_quotas_but_human_output_does_not` in
      `runtime/crates/eon-runtime/src/workspace.rs`, and
      `attention_queue_round_trips_and_rejects_invalid_entries_on_both_sides` and
      `attention_queue_keeps_its_wire_bytes` in
      `runtime/crates/eon-workspace-protocol/src/v25.rs`, within `nix develop -c eon-test`
      (590 passed, 57 ignored) on x86_64 Linux. The first three fail without the queue, and
      the golden fails when the entry field order changes.
- **Open proof:** A reply with a lower update sequence for the same run
  cannot be produced by an emitter, so only the automated test covers its
  discard. The `eon-agent-parity-a9g0.2` amendment awaits one installed
  `eon workspace --json` observation after landing. So does the `eon-ngwf.1`
  amendment, with agents needing attention in two workspaces.

## EON-C32 — Eon Bar items and order

- **Status:** Proven on x86_64 Linux Wayland. Amended on 2026-10-31 (approved
  by Lucca, relayed by eon-overseer) by `eon-hjtns`: the CPU and RAM load
  levels. Amended on 2026-10-31 (approved by Lucca, relayed by eon-overseer) by
  `eon-d7itl`: the disk widget. Amended on 2026-10-31 (approved by Lucca,
  relayed by eon-overseer) by `eon-wc119`: a leftover `show_version` loads as
  an alias. Amended on 2026-10-31 (approved by Lucca, "ok" to options A and E,
  recorded on the bead) by `eon-0oh9v`: the CPU label colours by the use it
  shows, and its hover gives the average. Amended on 2026-11-03 (approved by
  Lucca, relayed by eon-overseer) by `eon-369s`: the Eon icon leads the Eon Bar,
  and the version label sits between Shortcuts and Close.
- **Consumer:** Full-Eon workspace users.
- **Trigger:** A workspace window opens or reopens.
- **Result:** `[bar] widgets` is an ordered list of unique names from
  `codex_quota_5h`, `codex_quota_7d`,
  `claude_quota_5h`, `claude_quota_7d`, `claude_quota_scoped`, `cpu`, `ram`,
  `disk`, `clock` and `version`. `_5h` and `_7d` select only unscoped five-hour and seven-day
  windows, and `claude_quota_scoped` selects every model-scoped Claude window.
  Unknown names warn and are ignored.
  The Eon Bar shows the listed widgets left to right in that order, between the
  drag region and Shortcuts, except `version`, which sits between Shortcuts and
  Close. The quota widgets select Codex and Claude windows (EON-C27), `cpu`
  and `ram` the whole-machine CPU and memory use labels, `disk` the used
  percentage of the filesystem holding the configured path, `clock` the local
  24-hour clock, and `version` the read-only `EON <version>` label of the
  supervisor serving the window. The decorative Eon icon at the bar's leading
  edge is not a widget: whatever the list, it shows when Eon has one and the
  bar has room for it (VEN-C21). A widget left out doesn't show, and
  `widgets = []` leaves only the icon, tabs and controls. Omitting the
  key shows the ten default widgets in the order documented in
  [Configuration](CONFIGURATION.md). A listed widget without data still shows
  nothing, as [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar) describes for
  each widget. Narrow windows keep the fixed hiding priority it gives, whatever
  the order. `ram_warning_percent` and `ram_critical_percent` set the RAM use,
  in whole percent, from which the RAM label turns orange and red (defaults 80
  and 90); `cpu_warning_percent` sets the CPU use the label shows, over the last
  2 seconds, from which the CPU label turns orange (default 90), and
  `cpu_average_seconds` the window of the average CPU use that its hover and
  accessibility text also give (default 30). `disk_path` (default `~`) names
  the path whose filesystem `disk` measures: `~` or a path starting with `~/`
  under HOME, or an absolute path.
  `disk_warning_percent` and `disk_critical_percent` set the disk use from which
  the disk label turns orange and red (defaults 85 and 95), with the same rule
  as RAM's levels. Percentages are 1 through 100, `ram_critical_percent` is not
  below `ram_warning_percent` nor `disk_critical_percent` below
  `disk_warning_percent`, and the window is 2 through 600 seconds. They apply
  when a window opens or reopens, like the list.
  A leftover `[bar] show_version` (deprecated in v0.1.2) still loads, with one
  warning naming `bar.widgets`: `false` reads as the default list without
  `version`, `true` as the default list, and a `widgets` key set beside it
  wins.
- **Important failures:** A repeated known name, a non-string entry, a
  value that is not a list, a load level that is not a whole number in its
  range, a critical level below its warning level, a `disk_path` other than
  `~`, a path starting with `~/` or an absolute path, or a `[bar] show_version`
  that is not true or false fails through the existing startup and replacement
  handling, preserving live terminals.
- **Owner:** Eon owns the settings, their names and defaults. Runtime parses
  and validates them and passes the recognized list to Venus as `--bar-items`,
  with `--eon-version` only when `version` is listed, the Eon icon's path
  (`--eon-icon`) whenever Eon has one, the RAM levels only when
  `ram` is listed, the CPU levels only when `cpu` is listed, and the disk
  levels and resolved path only when `disk` is listed. Venus owns placement,
  fitting, colour, hover and accessibility.
- **Boundary:** No configurable hiding priority, live reload, settings UI,
  EONW change or platform expansion. Runtime reads a provider while at least
  one of its quota widgets is listed (EON-C27). Selection affects only
  presentation and its logo-room reservation; `eon workspace --json` retains
  every reported window.
- **Proof:** `eon-igae.14.2`, for the load levels `eon-hjtns` and
  `eon-0oh9v`, for the disk widget `eon-d7itl`, and for the leftover
  `show_version` alias `eon-wc119`
  - **Environment:** Automated; one isolated installed observation.
  - **Evidence:** `version_badge_uses_supervising_product_and_visibility_at_each_launch`
    and `invalid_presentation_configuration_precedes_supervisor_generation_and_children`;
    the installed observation recorded in `eon-igae.14.2`: a reordered list
    without `cpu` on window open, a leftover `show_version` refused on reattach
    (before `eon-wc119`) with live terminals kept, and the corrected list on
    the next reattach. For the load levels, the same launch test extended to
    the levels and their failures, Venus's
    `load_levels_are_a_bounded_workspace_choice`, and the isolated installed
    observation recorded in `eon-hjtns`. For the disk widget,
    the same launch test extended to the disk levels, the disk path's expansion
    and its failures, `disk_path_expands_home_and_keeps_absolute_paths`, the
    default launch in `bare_eon_attaches_only_to_the_live_current_generation`,
    Venus's `load_levels_are_a_bounded_workspace_choice` and
    `bar_items_are_an_ordered_workspace_choice` extended to the disk inputs,
    `disk_chip_shows_filesystem_use_and_colours_only_when_filling_up`, the
    `disk_use_*` reading tests,
    `readings_yield_after_the_clock_and_have_no_actions` extended to the disk
    chip's hiding order and accessible node, and the isolated installed
    observation recorded in `eon-d7itl`. For the `show_version` alias,
    `deprecated_show_version_reads_as_bar_widgets`,
    `configuration_corpus_loads_from_the_migration_floor`, the extended
    `version_badge_uses_supervising_product_and_visibility_at_each_launch`, and
    the isolated installed observation recorded in `eon-wc119`. For the CPU
    colour by the shown use, `cpu_and_ram_chips_colour_under_load_and_say_so`,
    `a_failed_cpu_reading_restarts_the_average_window`, and the isolated
    installed observation recorded in `eon-0oh9v`. For the Eon icon, the
    extended `version_badge_uses_supervising_product_and_visibility_at_each_launch`
    and `bare_eon_attaches_only_to_the_live_current_generation`, Venus's
    `eon_icon_is_a_workspace_only_absolute_path`, and the isolated installed
    observation recorded in `eon-369s`.

## EON-C33 — Several workspaces in one window

- **Status:** Proven on x86_64 Linux Wayland. Approved by Lucca on 2026-10-22
  (`eon-workspace-switching-shn4.1`, shape D of `eon-n77e`). The children of
  `eon-workspace-switching-shn4` implement it. Each child folds its part into
  the contract it amends: EON-C8 the final-close and emptying rules, EON-C10
  the workspace requests, targeting and exit statuses, EON-C11 attach, EON-C23
  the workspaces a window holds, EON-C28 the `eon workspaces --json` fields,
  EON-C31 focus, the queue and the rollup, VEN-C8 the shown workspace, and
  VEN-C21 the label. `eon-workspace-switching-shn4.7` implements the Eon Bar
  label and the workspace-worded final-close prompt in Venus and the CLI.
  `eon-workspace-switching-shn4.8` implements attention across workspaces,
  the picker's order and needs-input marker, and the label's badge on EONW
  v22.
  Amended on 2026-10-29 (approved by Lucca, relayed by eon-overseer) by
  `eon-ngwf.2`: `FocusAttention` across workspaces.
  Amended on 2026-11-04 (approved by Lucca) by `eon-p7omw`: Ctrl+X in the
  picker closes the highlighted workspace.
  Amended on 2026-11-04 (approved by eon-overseer under Lucca's standing
  rule) by `eon-vhhon`: the picker's own key confirms the highlighted row.
  Amended on 2026-11-04 (approved by eon-overseer under Lucca's standing
  rule) by `eon-hbpkq`: Alt+Shift+1..9 and Alt+Shift+0 show the workspace at
  list position N.
- **Consumer:** One full-Eon user, or an agent working for them, who keeps
  several separate sets of work in one Eon window.
- **Trigger:** Alt+Shift+S or Commands > Switch workspace; Alt+Shift+1
  through Alt+Shift+9 and Alt+Shift+0; Commands > New
  workspace, Rename workspace or Close this workspace; `eon workspaces` or
  `eon workspace new|switch|rename|close`; the final close or last exit in one
  workspace; an attention jump to a terminal in a hidden workspace; detach,
  attach or recovery of the window.
- **Result:**
  - **Model.** A window holds an ordered list of 1 to 32 workspaces and shows
    exactly one. Each workspace has its own tabs, panes, popups, focus and
    terminals, as EON-C8 describes. Each workspace has a stable `wN` identity,
    which is unique for the life of the window's supervisor and never reused
    in it, and an optional name of 1 to 64 characters with no control
    characters. Names are display text, not identity, and need not be unique.
    Tab, pane and popup identities are unique across the window's workspaces,
    so a tab or pane action never reaches another workspace; only the Alt+G
    jump below and `FocusAttention` (EON-C31) cross workspaces. Where a name
    is shown, an unnamed workspace shows its `wN`.
  - **Showing.** Venus renders only the shown workspace's tabs and terminals.
    Terminals in hidden workspaces keep running; only Venus's view of them is
    detached, as for a background tab. Switching never starts, stops or
    restarts a terminal. A snapshot change confined to hidden workspaces, such
    as their terminals, attention or tab edits, keeps the shown workspace's
    rejected-action notice and disturbs none of its navigation, scroll or
    attachments. A new window holds one workspace, `w1`, and a window with one
    unnamed workspace looks and behaves as before, without a label.
  - **Listing.** `eon workspaces [--json]` lists this window's workspaces in
    order. Each entry gives `id`, `name` (null when unnamed), `shown`, `tabs`
    (the tab count), `attention` and `recency` (0 for the shown workspace,
    then 1, 2, ... from the most recently shown, the picker's order, so an
    agent reads what a person sees there). `attention` uses exactly the JSON spelling
    of EON-C31 states that `eon workspace --json` gains with
    `eon-agent-sidebar-yx8.7` (kebab-case values such as `needs-input`, null
    when no terminal reports one) and carries no title or message. This window
    means the caller's window
    inside Eon, and otherwise the window `eon workspace` targets.
    `eon workspace [--json]` keeps its meaning for the caller's workspace:
    inside an Eon terminal the terminal's workspace, outside Eon the shown
    one.
  - **New.** `eon workspace new [--name NAME] [--no-focus]` and Commands >
    New workspace add a workspace after the shown one. It opens with one
    pending tab and a fresh directory picker, as a new window does (EON-C23),
    and becomes the shown workspace unless `--no-focus` is given, as for
    `pane create`. The reply names the new `wN` in `created`, whether or not
    it is shown.
  - **Switch.** `eon workspace switch WS` shows WS. Switching to the shown
    workspace returns the unchanged snapshot (EON-C10). Each workspace keeps
    its active tab, selected pane and popups across switches; zoom ends, as on
    any tab change (EON-C8). Alt+Shift+1 through Alt+Shift+9 show the window's
    first through ninth workspace in `eon workspaces` order, and Alt+Shift+0
    the tenth, as `eon workspace switch` with that workspace's ID does; a
    position with no workspace, or the shown workspace's own, changes nothing.
    `[workspace] number_keys = false` leaves those keys to the terminal, and a
    popup configured on one of them takes it.
  - **Rename.** `eon workspace rename [WS] NAME` and Commands > Rename
    workspace set the name; an empty NAME clears it. Without WS it renames the
    caller's workspace.
  - **Close.** `eon workspace close WS` and Commands > Close this workspace
    ask for confirmation, as `eon stop` does, naming the workspace and saying
    that closing stops every terminal in it, and that it quits the window
    when it is the last one; `--json` asks nothing. Closing then stops every
    terminal of that workspace, as closing each of its tabs does (EON-C8),
    and removes it.
    Closing the last workspace is the window's final close (EON-C8).
  - **Emptying.** When a workspace's final tab closes or its last terminal ends
    while other workspaces remain, it is removed. If it was shown, the most
    recently shown remaining workspace is shown; otherwise the shown workspace
    stays. Only the last workspace's final close or last
    exit ends the window. While other workspaces remain, the final-close
    prompt reads "Close the last tab? This closes workspace NAME and stops N
    terminals."; with one workspace it keeps "This quits the window".
  - **Targeting.** Run from an Eon terminal, every workspace-scoped command
    acts in that terminal's workspace, found through `EON_TERMINAL`, and
    EON-C10's per-tab rules apply inside it. Run outside Eon, commands act on
    the shown workspace. WS resolves an exact `wN` first, then an exact name.
  - **Picker.** Alt+Shift+S and Commands > Switch workspace open an
    Eon-owned fzf popup over the shown tab (VEN-C18). Rows show the name, or
    `wN` when unnamed, the tab count, and a marker when the workspace needs
    input. Rows are ordered most recently shown first, with the shown
    workspace last and marked, so Alt+Shift+S then Enter returns to the
    previous workspace; the Alt+G jump, not the picker, leads to agents that
    need input. Enter shows the highlighted workspace, including on an empty
    query. Enter on a non-empty query that matches no row creates a workspace
    with that name and shows it. Esc or Ctrl+C runs nothing. While the picker
    is its tab's selected popup, its own popup key (`[popups.workspaces]
    keybinding`, Alt+Shift+S by default) presses Enter in it instead of
    hiding it, so it confirms the highlighted row as Enter does: Alt+Shift+S
    twice returns to the previous workspace, and twice again comes back. With
    keyboard focus elsewhere in the window the key first focuses the picker,
    as for any popup, and a picker shown before its tab's directory changed
    restarts in the new directory (EON-C10). Esc, Ctrl+C and `eon popup hide`
    still hide it. Ctrl+X on the highlighted row closes that workspace as
    `eon workspace close` does, after
    the same confirmation in the popup; then the picker lists the remaining
    workspaces again. Closing the picker's own workspace ends the picker with
    it, and closing the last workspace is the window's final close (EON-C8).
    The marker reads
    ` · needs input`. EONW v22 gives each workspace its `recency`: 0 for the
    shown one, then 1, 2, ... from the most recently shown, which orders the
    rows (`eon-workspace-switching-shn4.6` and
    `eon-workspace-switching-shn4.8`, as EON-C10 describes).
  - **Eon Bar.** While the window holds two or more workspaces, or its one
    workspace has a name, the Eon Bar shows a leading label before the tabs:
    the shown workspace's name or `wN`, followed by `N/M` when there are
    several. Its accessible name reads "Workspace NAME, N of M", clicking it
    opens the picker, and it shows a badge while a hidden workspace needs
    input. The badge is the EON-C31 needs-input tab marker at the label's
    corner; it moves neither the label nor the tabs, and the accessible name
    then ends with ", another workspace needs input"
    (`eon-workspace-switching-shn4.7` and `eon-workspace-switching-shn4.8`,
    as VEN-C21 describes).
  - **Attention.** Each workspace carries the most urgent normalized EON-C31
    state of its terminals. The Alt+G queue (EON-C31) spans every workspace of
    the window, and a jump to a terminal in a hidden workspace shows that
    workspace first. Focus sees a raise only while its terminal's workspace
    is shown, under EON-C31's rules. The sidebar lists the shown workspace's
    terminals only. Implemented on EONW v22, whose `WorkspaceInfo` carries
    each workspace's rollup as `attention` (`eon-workspace-switching-shn4.8`).
  - **Detach, attach and recovery.** Alt+Shift+D detaches every workspace
    together, and attach shows the last shown workspace; closing the window
    follows EON-C8's close rule. `eon stop` and Commands > Quit this window
    stop every workspace of the window. Same-boot recovery projects every
    recovered run into one workspace `w1`, as EON-C8 projects them into `t1`;
    names and membership are not recovered.
- **Important failures:** These leave every workspace unchanged and return
  EON-C10's typed failure: an unknown or stale `wN` or a name that matches
  nothing (exit 3); a name that matches more than one workspace
  (`ambiguous-workspace`, exit 1, naming the matching IDs); a 33rd workspace
  (`workspace-limit`, exit 1); a malformed name (exit 2); and a tab, pane or
  popup identity from another workspace (exit 3), except a `FocusAttention`
  target (EON-C31). A request ID repeated
  within the last 256 accepted mutations never repeats a switch, creation
  or close. A failed stop while closing a
  workspace keeps the workspace and its remaining terminals, as a partial tab
  close does (EON-C8). Switching never moves desktop focus.
- **Owner:** Eon Runtime owns the workspace list, identities, the shown
  workspace, names, removal and the per-workspace attention rollup. Eon owns
  the picker, the Commands entries and the CLI projection. Venus renders the
  shown workspace and the bar label (VEN-C8, VEN-C21). Orbit owns every
  terminal.
- **Consumes:** EON-C8, EON-C10, EON-C11, EON-C15, EON-C23, EON-C31, the next
  EONW version, VEN-C8, VEN-C18 and VEN-C21.
- **Boundary:** No showing another window's or generation's workspace in this
  window (Venus never changes supervisor), moving a workspace, tab or pane
  between workspaces or windows, a window with zero workspaces, names or
  membership kept past the supervisor's end or across a reboot
  (`eon-reboot-restore-im8y`), unique names or names as identity,
  next/previous/last-workspace shortcut beyond the picker's order,
  per-workspace detach or configuration, or a second workspace in eonbase,
  which keeps one workspace, no picker and no workspace commands.
- **Proof:** `eon-workspace-switching-shn4.9`
  - **Environment:** Automated tier and isolated installed dogfood at edge
    `57718aee6278aeb700256d891a7c5fb752cc1bea`; the dogfood ran on x86_64 Linux
    with the installed `eon` profile
    `/nix/store/d52p46myxlci6r61k60waw39xcvlpz0h-eon-0.1.5` (EONW v22),
    private headless Sway 1.12 (pixman) at 1600x900, keys and pointer from
    wdotool. An OSC 7501 fixture stood in for an agent, and the host's foot
    1.16.2 was the second client.
  - **Evidence:**
    - **Automated:** the full `nix develop -c eon-test` run on that revision,
      recorded in `eon-workspace-switching-shn4.8`. In
      `runtime/crates/eon-runtime/src/workspace.rs`:
      - `workspace_requests_keep_order_projection_names_and_bounds` covers the
        duplicate, ambiguous, unknown, malformed and limit refusals;
      - `workspace_replay_windows_are_bounded_and_inspections_do_not_evict_mutations`
        covers the 256-mutation replay window;
      - `workspace_set_refuses_foreign_targets` covers identities from another
        workspace.

      `workspace_refusals_use_the_existing_exit_status_classes` in
      `runtime/crates/eon-runtime/src/control.rs` covers the exit classes.
      `hidden_raises_stay_unseen_and_the_queue_crosses_into_their_workspace`
      in `runtime/crates/eon-runtime/src/attention.rs` and
      `only_a_queue_jump_crosses_workspaces_and_it_shows_its_target` in
      `workspace.rs` cover attention across workspaces, and
      `workspace_label_leads_the_tabs_and_opens_the_picker` in
      `venus/src/accessibility.rs` covers the label's accessible name. In
      `eon/crates/eon/tests/workspace_control.rs`:
      - `workspace_requests_project_callers_and_close_only_the_target_workspace`
        covers a failed stop while closing and per-workspace state across
        switches;
      - `workspace_cli_lists_creates_switches_renames_and_closes` covers the CLI;
      - `a_workspace_final_close_shows_the_recent_one_and_attach_keeps_it` covers
        emptying;
      - `workspace_picker_switches_creates_and_cancels` covers the picker.

      `workspace_picker_lists_by_recency_with_the_shown_last_and_maps_its_result`
      in `runtime/crates/eon-runtime/src/cli.rs` covers the picker's order.
    - **Installed:** results come from captures, `eon workspaces --json`
      dumps, Sway trees and process lists, over four runs.
      - Three workspaces in one window. `eon workspace new --name notes`
        created and showed one; Alt+Shift+S with the unmatched query `build`
        created and showed another. The label read `build 3/3`.
      - The picker listed notes, w1, then `build · shown`, most recently
        shown first. Enter showed notes. `eon workspace switch w1` and
        `switch build` (by name) switched from outside Eon.
      - From a terminal in w1, with the picker closed, the agent path worked:
        - `eon workspace rename w1 main`;
        - `eon workspace new --no-focus --name tmp`, after which w1 stayed
          shown;
        - `eon workspace close w4 --json`;
        - `eon workspace switch w3`.

        In the fourth run, a reply saved from a terminal's
        `eon workspaces --json` listed the window's workspaces with
        `attention` and `recency`.
      - Commands > New, Rename, Switch and Close this workspace each worked.
        Close asked "Close workspace w2 (docs)? This stops every terminal in
        it. [y/N]"; n kept the workspace and y removed it. On a TTY,
        `eon workspace close scratch` answered with n printed "cancelled;
        workspace w3 stays open", and y removed it. A gone `w2` exited 3.
      - Attention. The fixture in hidden notes went to `needs-input` in
        `eon workspaces`. The label showed its ◆ badge, and the picker row
        read `notes 1 tab · needs input`. Alt+G showed notes with the
        fixture's pane selected, and the badge cleared.
      - Alt+Shift+W on notes, the middle of three, asked "Close the last tab?
        This closes workspace notes and stops 1 terminal." Enter removed only
        notes, stopped its terminal and showed build, the most recently
        shown survivor. With one workspace left, the prompt read "This quits
        the window".
      - Closing the window from the compositor (superseded by `eon-7cf0y.16`:
        a close now follows EON-C8's close rule), and later Alt+Shift+D, ended
        Venus and kept the Orbit hosts and every workspace. `eon attach`
        showed build, the last shown workspace, and main's scrollback was
        intact.
      - Recovery. SIGKILL to the exact supervisor PID ended Venus, and the
        three Orbit hosts kept running. A fresh `eon` recovered one unnamed
        workspace `w1` whose tab `t1` held all three terminals.
      - Commands > Quit this window runs `eon stop` for the generation. With
        two workspaces open, it listed all five live terminals. After y, no
        Eon, Venus or Orbit process of the run remained.
      - Desktop focus. Sway used `focus_on_window_activation smart`, and a
        foot window beside Eon held focus. It kept focus through
        `eon workspace switch`, `eon workspace new --name gamma`, which shows
        the new workspace, and a needs-input raise in a hidden workspace. No
        window turned urgent.

## EON-C35 — Reading a terminal from the CLI

- **Status:** Proven on x86_64 Linux Wayland. Approved by Lucca on 2026-10-30
  in the merged text of `eon-agent-parity-a9g0.14`; its `terminal-unavailable`
  and exit 4 failures amended by Lucca on 2026-10-31 ("agreed", then "ok" for
  the narrowed exit 4, relayed by eon-overseer, `eon-agent-parity-a9g0.19`).
  `eon-agent-parity-a9g0.16` implements the Runtime's EONW v26 read and the
  setting, `eon-agent-parity-a9g0.17` the command.
- **Consumer:** An agent or script running as the person's user, in an Eon
  terminal or outside one.
- **Trigger:** `eon pane read TARGET [--json]`, with:
  - TARGET a pane or popup ID as `eon workspace --json` names them;
  - EON-C10's `--window` and `--generation` selection.
- **Result:**
  - The supervisor that EON-C10 selects (validating `EON_TERMINAL` when set)
    resolves TARGET in any workspace of its window, shown or hidden, and reads
    the target's terminal through ORB-C17.
  - JSON carries:
    - the target: `workspace`, `tab`, `pane` or `popup`, and `terminal`;
    - `caller`, as `eon workspace` reports it (null outside Eon);
    - `generation` and `window`;
    - `revision`, `screen` (`primary` or `alternate`), `rows` and `columns`;
    - `title`;
    - `directory`: a string when valid UTF-8, else a byte array, as EON-C10;
    - `text`, `omitted_rows`, and `lossy: true`.
  - Human output prints the text alone.
  - Reading changes no workspace, tab, pane or popup selection, shown
    workspace or popup, or desktop focus, and never shows a hidden popup. It
    leaves no visible trace.
  - `[agents] terminal_access` ([Configuration](CONFIGURATION.md#configuration-behavior))
    governs it: `"window"` allows it; `"none"` refuses every read with
    `terminal-access-off` and no terminal content.
- **Important failures:**
  - Exit 1:
    - `terminal-access-off`;
    - `terminal-ended`: the target's terminal has exited; its exit status
      belongs to eon-agent-parity-a9g0.4;
    - `terminal-unavailable`: Orbit refused, or timed out after 2 s.
  - Exit 3: a missing or stale target, generation or window.
  - Exit 4: a missing or unreachable supervisor, including one that speaks
    another EONW version (`supervisor-unavailable`).
  - Exit 2: invalid arguments.
  - No failure carries terminal content.
- **Owner:**
  - Eon Runtime: policy, target resolution, the EONW v26 request and the CLI
    projection;
  - Orbit: ORB-C17.
- **Boundary:** None of the following:
  - history, styles, streaming or waiting (eon-cli-action-discovery-ogzi);
  - per-agent tokens, grants, expiry or audit log;
  - remote or MCP exposure (eon-ciwn.3);
  - redaction;
  - protection against code running as the same user.
- **Proof:** the `eon-agent-parity-a9g0.19` commit, for the code at edge
  `acc29d2` (`eon-agent-parity-a9g0.16` landed as `ef82672`,
  `eon-agent-parity-a9g0.17` as `a2cb0a9`).
  - **Environment:** Automated tier: sandboxed `nix develop -c eon-test` on
    each of those beads' trees. Isolated installed tier: x86_64 Linux,
    `/nix/store/n9qkzdwngqvcbxfpla66k57vjsqdwgga-eon-0.1.9` built from
    `acc29d2` (Orbit
    `/nix/store/c6xzav8mbqbwp4dm4niyqy4n4s5774pr-yazelix-orbit-0.1.0`), in a
    private headless Sway 1.12 with its own HOME and runtime directory.
  - **Evidence:**
    - From `eon-agent-parity-a9g0.17`, the CLI test
      `pane_read_prints_the_text_or_json_and_exits_by_failure` in
      [`workspace_control.rs`](../eon/crates/eon/tests/workspace_control.rs),
      which checks human and JSON output, `caller`, each failure exit
      (Orbit's `terminal-ended` and `terminal-unavailable` from a fake Orbit)
      and that a read changes no workspace state.
    - From `eon-agent-parity-a9g0.16`, the EONW v26 codec tests in
      `runtime/crates/eon-workspace-protocol/src/v26.rs`, the
      lease tests in
      [`sessions.rs`](../runtime/crates/eon-runtime/src/sessions.rs) (one
      against a real Orbit), and the supervisor tests
      `terminal_reads_and_sends_reach_a_hidden_workspace_and_change_nothing`
      and `terminal_access_none_refuses_every_read_and_send` in
      [`workspace_control.rs`](../eon/crates/eon/tests/workspace_control.rs).
    - Installed: an agent script running in a shown pane read a hidden tab's
      pane, a hidden workspace's pane, a hidden keep-alive popup, and Claude
      Code's and Codex's panes in hidden tabs; its JSON reads named its pane as
      `caller`. The shown workspace (the other stayed hidden), the active tab,
      the selected pane and shown popup (none, then Files) of each tab in the
      shown workspace, and the desktop focus, held by another window, were
      unchanged, and no hidden popup was shown. Reads kept working while the
      window was detached. Under a `yes` flood in a shown pane, 20 reads of it
      took 73 to 109 ms and 20 reads of a quiet pane 76 ms at the median, as
      when idle. A closed pane, an unknown pane and an unknown generation or
      window exited 3. A terminal whose Orbit was stopped answered
      `terminal-unavailable` after 2 s, and at once while that request stayed
      unanswered; a pane whose stop had failed and was still outstanding
      answered `terminal-ended` (a terminal that exited on its own was checked
      only through the fake Orbit in the CLI tests). A second window started
      with `terminal_access = "none"` refused reads with `terminal-access-off`,
      exit 1, and no terminal content, while the first window allowed them.
    - Older generation: against an isolated eon 0.1.8 generation (EONW v25)
      running alone, `eon pane read` and `eon pane send` with its
      `--generation` each exited 4 with `supervisor-unavailable`, naming the
      version mismatch and pointing at the terminal's own `eon`. No EONW v26
      supervisor holds a terminal on an older Orbit: it recovers or adopts a
      terminal only when the record names its own component and management
      generation
      ([`sessions.rs`](../runtime/crates/eon-runtime/src/sessions.rs)), and
      the generation identity covers Orbit's source.

## EON-C36 — Sending input to a terminal from the CLI

- **Status:** Proven on x86_64 Linux Wayland. Approved by Lucca on 2026-10-30
  in the merged text of `eon-agent-parity-a9g0.14`; its `terminal-unavailable`
  and exit 4 failures amended by Lucca on 2026-10-31 ("agreed", then "ok" for
  the narrowed exit 4, relayed by eon-overseer, `eon-agent-parity-a9g0.19`).
  `eon-agent-parity-a9g0.16` implements the Runtime's EONW v26 send and the
  settings, and carries the Enter delay in its reply;
  `eon-agent-parity-a9g0.18` the command, which waits that delay before it
  sends Enter.
- **Consumer:** An agent or script running as the person's user, in an Eon
  terminal or outside one.
- **Trigger:** `eon pane send TARGET [--enter] [--stdin] [--json] [-- TEXT]`,
  where:
  - TARGET is a pane or popup ID as `eon workspace --json` names them;
  - TEXT is exactly one operand, or standard input with `--stdin`. TEXT is
    required unless `--enter` is given, and `--enter` alone sends only Enter;
    with neither `--stdin` nor TEXT, standard input that is a pipe, socket or
    file already holding data fails before anything is sent, naming `--stdin`
    and empty input as the remedies; a terminal, an empty or ended input, or a
    pipe or socket with no data within 100 ms is not read;
  - EON-C10's `--window` and `--generation` selection applies.
- **Result:**
  - The supervisor that EON-C10 selects (validating `EON_TERMINAL` when set)
    resolves TARGET in any workspace of its window, shown or hidden, and
    pastes TEXT into the target's terminal through ORB-C5's lease input.
    Orbit brackets the paste when the program enabled bracketed paste.
  - With `--enter`, Eon then sends one Enter key after
    `[agents] send_enter_delay_ms`
    ([Configuration](CONFIGURATION.md#configuration-behavior)). A line feed or
    carriage return in TEXT never submits.
  - Sending changes no workspace, tab, pane or popup selection, shown
    workspace or popup, scroll position or desktop focus. A hidden popup
    receives the input and stays hidden. The send leaves no trace beyond what
    the program shows.
  - On success the command exits 0. JSON carries:
    - the target: `workspace`, `tab`, `pane` or `popup`, and `terminal`;
    - `caller`, `generation` and `window`, as EON-C35;
    - `bytes`: TEXT's length, 0 for Enter only;
    - `bracketed`: a bool, null when only Enter was sent;
    - `enter`: true when the Enter was sent.
  - `[agents] terminal_access`
    ([Configuration](CONFIGURATION.md#configuration-behavior)) governs it:
    `"window"` allows it; `"none"` refuses every send with
    `terminal-access-off` and sends nothing.
  - Documentation advises keeping a send short and pointing at a file for long
    material. Some agents treat long pastes as outside material: Claude Code
    marks pastes over 800 characters or 3 lines that way.
- **Important failures:**
  - Exit 2, with nothing sent:
    - `invalid-text`: TEXT is not UTF-8, or contains ESC, another C0 control
      except tab, line feed and carriage return, DEL, or a C1 control. Eon
      refuses rather than rewrites it;
    - `input-too-large`: TEXT exceeds 64 KiB;
    - `unread-stdin`: neither `--stdin` nor TEXT, and standard input holds
      data, as the Trigger says;
    - no TEXT and no `--enter`, or other invalid arguments.
  - Exit 1:
    - `terminal-access-off`, with nothing sent;
    - `unbracketed-newline`: TEXT has a line feed or carriage return and the
      program has not enabled bracketed paste. Nothing is sent;
    - `terminal-ended`: the terminal has exited. The input is rejected, never
      queued;
    - `terminal-unavailable`: Orbit refused, or timed out after 2 s.
  - Exit 1 also when the paste succeeds and the Enter then fails. The reply
    names the failure with `enter: false`, and the text stays in the program.
  - Exit 3: a missing or stale target, generation or window.
  - Exit 4: a missing or unreachable supervisor, including one that speaks
    another EONW version (`supervisor-unavailable`).
  - No failure changes focus or workspace state.
- **Owner:**
  - Eon Runtime: policy, TEXT validation, target resolution, the Enter delay,
    the EONW v26 request and the CLI projection;
  - Orbit: ORB-C5 lease input.
- **Boundary:** None of the following:
  - other keys, chords or raw bytes, or a typed (keystroke) mode;
  - waiting for the program to consume or answer the input; reading back is
    EON-C35;
  - per-agent tokens, grants, expiry or audit log;
  - remote or MCP exposure (eon-ciwn.3);
  - protection against code running as the same user.
- **Proof:** the `eon-agent-parity-a9g0.19` commit, for the code at edge
  `acc29d2` (`eon-agent-parity-a9g0.16` landed as `ef82672`,
  `eon-agent-parity-a9g0.18` as `acc29d2`).
  - **Environment:** as EON-C35, except that the automated tier ran on the
    `eon-agent-parity-a9g0.16` and `eon-agent-parity-a9g0.18` trees. In the
    installed tier, Claude Code 2.1.303 and Codex 0.166.0 (`codex
    --no-daemon -c model_reasoning_effort=low -c features.hooks=false`,
    since a paste cannot answer its hook-review menu) ran in Eon panes.
  - **Evidence:**
    - From `eon-agent-parity-a9g0.18`, the CLI test
      `pane_send_pastes_then_waits_for_enter_and_exits_by_failure` in
      [`workspace_control.rs`](../eon/crates/eon/tests/workspace_control.rs),
      which checks the JSON reply, the Enter delay, `--stdin`, each failure
      exit (Orbit's refusals from a fake Orbit), an Enter that fails after
      its paste, and that refusals send nothing and a send changes no
      workspace state.
    - From `eon-agent-parity-a9g0.16`, the EONW v26 codec tests, which
      refuse invalid and oversized text on both sides, the lease tests,
      which map every Orbit answer to its failure code and show lease input
      in a real Orbit's text, and the two supervisor tests named under
      EON-C35.
    - Installed: the agent script of EON-C35's proof sent a command with
      `--enter` to a hidden tab's pane, a hidden workspace's pane and a hidden
      keep-alive popup; each ran it (`bracketed: true`), and the person's view
      and desktop focus were unchanged, with the hidden popup still hidden.
      Sends worked while the window was detached. A prompt sent with `--enter`
      to Claude Code and to Codex in hidden tabs was submitted and answered. To
      `cat`, which has no bracketed paste, a one-line send arrived with
      `bracketed: false`; a line break was refused with `unbracketed-newline`
      (exit 1), ESC with `invalid-text` and 70,000 bytes with `input-too-large`
      (exit 2), and neither TEXT nor `--enter` was a usage error (exit 2); none
      of them wrote anything. Sends to a closed pane exited 3; to a stopped
      Orbit's terminal, `terminal-unavailable`; to a pane whose stop had failed
      and was still outstanding, `terminal-ended`. A terminal that exited on
      its own was checked only through the fake Orbit in the CLI tests. The
      `terminal_access = "none"` window refused a send with
      `terminal-access-off`, and its picker stayed pending.
    - Older generation: `eon pane send` exited 4 with
      `supervisor-unavailable`, as recorded under EON-C35.

## EON-C37 — Upgrades within 0.x

- **Status:** Planned (`eon-c8ytm`, approved by Lucca 2026-10-31, relayed by
  eon-overseer). It binds from the announcement: the macOS launch or the first
  public announcement, whichever comes first.
- **Consumer:** People, scripts and agents that use Eon 0.x on x86_64 Linux on
  native Wayland and upgrade or roll back within 0.x.
- **Trigger:** Installing a later 0.x release over an earlier one at or above
  its declared migration floor, or rolling back to one.
- **Result:** A `config.toml` that loaded in the earlier release loads with
  the same meaning or, for a deprecated key, its documented replacement and a
  warning naming it. A 0.N.y patch release removes no key, command, flag or
  JSON field. A removed key warns and never stops startup, unless ignoring it
  would widen access, run something the user did not configure, or lose data.
  Eon never rewrites `config.toml` on its own. Durable state written at or
  above the floor is read or migrated, and the earlier file stays readable by
  the earlier release, so rollback within the floor loses no setting.
  Installing never stops live terminals, and the new `eon` lists and stops
  older generations directly or through their recorded `eon`.
- **Important failures:** A release that fails on, or silently changes the
  meaning of, a configuration that loaded at its floor; deletes migration code
  above its floor; or rewrites user configuration without an explicit command.
- **Owner:** Eon owns the promise, the release review against it and each
  release's declared migration floor. Eon Runtime owns the configuration
  parsing, durable-state reading and lifecycle commands that carry it.
- **Boundary:** Not covered: how appearance and layout render; behavior that
  no document states; the ORBS and ORBF protocols, EONW beyond listing and
  stopping older generations above, and the component manifest format, which
  carry their own versions; internal helpers that no document lists; the
  unprefixed tool names inside terminals; Anima's styles and options; eonbase;
  the behavior of pinned tools such as Helix, Yazi and Nushell when their pins
  update; reattaching across presentation-incompatible releases, which needs the
  earlier release's executable (EON-C11); and every platform other than x86_64
  Linux on native Wayland, including the macOS preview.
- **Proof:** None yet.
- **Open proof:** The config and state corpus test (`eon-embg6`) in
  `eon-test`, plus one installed skip-version observation (floor release to
  new release) and one rollback observation per minor release.

## EON-C38 — Persistent workspace identity and host registry

- **Status:** Planned (`eon-remote-workspaces-52i4.4`, text by Yu Sun in
  PR #3, approved by Lucca 2026-11-03). Slice 2a
  (`eon-remote-workspaces-52i4.5`) implements it; the plan and acceptance
  matrix are in [`REMOTE-WORKSPACES.md`](REMOTE-WORKSPACES.md).
- **Consumer:** Local and remote Eon workspace clients and the host's
  ID-targeted workspace operations.
- **Trigger:** Create or rename a workspace, list/read its registration, resolve
  a target, detach/reattach, or restart the workspace supervisor.
- **Result:**
  - The host owns a durable, opaque workspace ID and a versioned registry.
    An ID identifies the same registered workspace across client and supervisor
    lifetimes. A window's `wN` is a local handle, not durable identity. A remote
    target includes its host and workspace ID; one window still uses one host.
  - Names and directories are attributes. Renaming keeps the ID; names need
    not be unique. Separate workspaces may use the same directory, and one
    workspace may contain multiple directories. A stale/deleted ID is never
    rebound to a different workspace merely because its name/path matches.
  - Registry inspection can find registered identities independently of the
    current window's members. Registration is metadata, not proof that any
    terminal is running or that a recorded process may be signalled.
  - Concurrent registry mutations are serialized. A successful durable mutation
    is acknowledged only after atomic persistence; failure does not publish a
    successful new identity or partly committed registration. IDs are not reused.
    Version changes preserve the old durable file as the repository's durable
    format policy requires; no unknown version is silently overwritten.
- **Important failures:** Unknown/stale/ambiguous targets, corrupt or unknown
  registry versions and persistence failures give explicit diagnostics rather
  than guessed targets or a replacement with the same name. Malformed state
  remains available for diagnosis. Registry failure authorizes no terminal
  stop, destructive cleanup, duplicate process launch or fallback local creation.
- **Owner:** Eon Runtime owns persistent IDs, registry state and target
  resolution. Eon projects that authority through its existing command/picker
  boundaries. Orbit remains the authority for terminal/run state and lifecycle.
- **Boundary:** Identity and registry only. Slice 2a does not promise original
  membership/layout recovery, SSH transport, takeover, terminal stop/Forget
  behavior, reboot process restoration or moving live work across hosts. Any
  CLI/schema changes introduced with 2a amend EON-C10 in that slice; this text
  allocates no separate CLI contract. Existing window-local handles may remain
  adapters, with an explicit mapping to persistent IDs.
- **Proof:** None yet.
- **Open proof:** I1–I3 in the acceptance matrix of
  [`REMOTE-WORKSPACES.md`](REMOTE-WORKSPACES.md), run by slice 2a. Original
  groups/layout and verified run recovery belong to slice 2b and amendments
  to EON-C15/C33, not this proof.

## EON-C39 — Workspace events from the CLI

- **Status:** Planned (`eon-ciwn.1`, approved by Lucca 2026-11-03, relayed by
  eon-overseer). `eon-ciwn.5` builds the Runtime event projection,
  `eon-ciwn.6` the EONW v30 codec, `eon-ciwn.7` the supervisor
  subscriptions, `eon-ciwn.8` `eon events` and `eon-ciwn.9` `eon pane wait`.
  Amended on 2026-11-03 (approved by Lucca, relayed by eon-overseer) by
  `eon-ciwn.6`: a snapshot's tabs cross as `TABS` messages, and tab and pane
  events carry the order of tabs and columns. Amended on 2026-11-03
  (approved by Lucca, relayed by eon-overseer) from `eon-ciwn.5`: the
  workspace tab count is derived, the stream's busy list is uncapped and
  busy events carry `name`, and attention and busy events follow a terminal
  between popup and pane. Amended on 2026-11-03 (approved by Lucca, relayed
  by eon-overseer) by `eon-ciwn.5`: applying events rebuilds state only for
  streams without filters or filtered only by `--tab` or `--kind`, and the
  `queue` event is also sent when an entry's state changes. Amended on
  2026-11-03 (approved by Lucca, relayed by eon-overseer) by `eon-ciwn.5`: a
  `--tab` stream does not rebuild workspace-wide values, and the text says
  which kinds a stream admits for rebuilding. Amended on 2026-11-03
  (approved by Lucca, relayed by eon-overseer) by `eon-ciwn.7`: output has
  moved when the subscriber has read at least one write unit of up to 4 KiB
  since the last check. Amended on 2026-11-03 (approved by Lucca, relayed by
  eon-overseer) by `eon-ciwn.11`: the retained events are bounded by 8 MiB
  as well as by 1024 events. Amended on 2026-11-04 (approved by eon-overseer
  under Lucca's standing approval for this fix) by `eon-61efs.1`: Venus
  subscribes to learn of workspace changes promptly, and the events of a
  change caused by an EONW action are written before that action's reply.
- **Consumer:** An agent or script running as the person's user, in an Eon
  terminal or outside one, that must notice workspace and attention changes
  without polling: an overseer answering its tab's questions (`eon-ciwn.4`),
  `eon pane wait`, and a person's own notifier (`eon-sixj`). Venus also
  subscribes, once per window, so that it re-reads the workspace when it
  changes instead of at its next `Inspect` poll.
- **Trigger:**
  - `eon events [--json]`, with:
    - EON-C10's `--window` and `--generation` selection and `EON_TERMINAL`
      validation;
    - the filters `--workspace WS`, `--tab TAB`, `--target ID`,
      `--kind KIND` (`workspace`, `tab`, `pane`, `popup`, `attention`,
      `queue` or `busy`) and `--state STATE` (an EON-C31 state as the CLI
      spells it), each repeatable;
    - `--after STREAM:SEQ`;
  - `eon pane wait TARGET --until done|needs-input|attention|ended
    [--timeout SECONDS] [--json]`.
- **Result:**
  - **One subscription.** The CLI sends one EONW v31 `Subscribe` request.
    The supervisor either refuses it with an ordinary EONW `Failure` reply
    and closes the connection, or answers `SUBSCRIBED` and keeps the
    connection open for `TABS`, `EVENT`, `GAP` and `END` messages. A
    `SUBSCRIBED` or `GAP` message carries every field of its snapshot but
    `tabs`, and the number of `TABS` messages that follow it: one per
    workspace with admitted tabs, each holding that workspace's tabs. Every
    message thus fits EON-C10's 2 MiB frame ceiling, and the CLI joins a
    snapshot's messages into one snapshot line. Every message is one
    length-delimited EONW frame; EOF never frames a message.
  - **Snapshot, then events.** The first JSON line is
    `{"type":"snapshot","stream","seq","resumed","generation","window",
    "caller","workspaces","tabs","attention_queue","busy"}`. It covers every
    workspace of the window that the filters admit:
    - `workspaces` holds each workspace as `eon workspaces --json` lists it,
      plus its `active_tab`;
    - `tabs` holds each tab as `eon workspace --json` writes it, with its
      panes, popups and attention objects, plus its `workspace`;
    - `attention_queue` holds the window's queue in its EON-C31 order, each
      entry `{workspace, tab, target, state}`;
    - `busy` holds the EON-C11 busy terminals of the workspaces, tabs and
      targets the filters admit, as `eon workspace --json` writes them but
      without its 32-terminal bound (`omitted` is 0), and without the
      calling terminal.

    `active_tab`, the tab's `workspace` and the queue entry's `state` in
    place of its `attention` are the only differences from those commands'
    objects. Each later line is `{"type":"event","seq","kind","event",...}`
    with the event's payload. `seq` is the supervisor's event counter and
    rises by one per event before filtering, so a filtered stream skips
    numbers without losing events.
  - **Applying events.** For a stream without filters, or filtered only by
    `--tab` or `--kind`, applying its events in order to its snapshot gives
    the Runtime's state, after the last event, for the kinds the stream
    admits; a stream without `pane`, for example, never sees `pane-closed`.
    A pane's or popup's `attention` field belongs to the `attention` kind,
    and the window order of the busy list and of the tab groups stays current
    only when `workspace`, `tab`, `pane` and `popup` are all admitted. Under
    `--tab` or `--target`, `workspace` and `queue` count as admitted only
    when `--kind` names them. In a `--tab` stream the named tabs and their
    panes and popups stay current, but workspace-wide values are not
    rebuilt: a workspace's `active_tab`, the order of other tabs, the queue
    and the busy list's window order; a consumer reads `eon workspace --json`
    for them. Under `--workspace`, `--target` or `--state` the stream is a
    change feed for the objects it admits, and a consumer reads
    `eon workspace --json` for full state. The tab `attention_rollup` and the
    workspace `attention`, `recency` and `tabs` are derived values that no
    event names; a consumer recomputes them or reads `eon workspaces`. A
    workspace's `active_tab` follows `tab-activated`. Changes within one
    supervisor loop turn coalesce, and EON-C31 already coalesces attention
    between polls.
  - **Events.**
    - `workspace`: `workspace-created`, `workspace-renamed`,
      `workspace-closed` and `workspace-shown`, each with the workspace as
      the snapshot lists it.
    - `tab`: `tab-created`, `tab-moved`, `tab-directory`, `tab-closed` and
      `tab-activated`, each with its `workspace` and the tab.
      `tab-created`, `tab-moved` and `tab-closed` also carry `tabs`, the
      workspace's tab IDs in order.
    - `pane`: `pane-created`, `pane-moved`, `pane-renamed`, `pane-ended`,
      `pane-closed` and `pane-selected`, each with its `workspace`, `tab`
      and the pane. Each `pane` event, `pane-renamed` included, also carries
      `columns`, the tab's columns in order as `{panes, expanded}` with pane
      IDs.
    - `popup`: `popup-created`, `popup-shown`, `popup-hidden` and
      `popup-closed`, each with its `workspace`, `tab` and the popup.
    - `attention`: the terminal's `workspace`, `tab`, `pane` or `popup` (the
      other null), `terminal`, `previous` state (null when it had none) and
      its full EON-C31 `attention` object (null when it has none). It is
      sent at most once per terminal per supervisor loop turn, when any
      field of that object changes: a poll reply, a focus or attaching
      Venus that sees a raise, and the stale threshold passing each change
      it in their own loop turn. It is also sent, with `previous` equal to
      its state, when its terminal's pane or popup changes, as when an Agent
      popup is parked into a pane.
    - `queue`: the whole queue, as the snapshot carries it, when any entry
      changes: its order, its membership or an entry's state.
    - `busy`: a terminal's EON-C11 busy state, with its `workspace`, `tab`,
      `pane` or `popup`, `terminal`, `busy` and, while busy, its `name`. It
      is sent when the terminal becomes busy or idle, and while it stays busy
      when its `name`, pane or popup changes. The calling terminal is never
      reported, as EON-C11 excludes it.
  - **Latency.** An event is written within one supervisor loop turn of the
    Runtime state change it reports. The events of a change made by an EONW
    action are written before that action's reply to each subscriber with no
    earlier events still pending; a subscriber that is behind receives them
    after its pending output. An attention change reported by a program is a
    Runtime state change when the poller receives it (at most 2 Hz, EON-C31).
  - **Filters.** The Runtime applies them. Different flags combine with
    AND; repeats of one flag combine with OR. Without filters the stream
    covers the whole window.
    - `--workspace WS` resolves once, when subscribing, as EON-C33
      resolves WS: an exact `wN` first, then an exact name.
    - `--target ID` follows the terminal that ID names when subscribing,
      so an Agent popup parked into a pane (EON-C10) stays admitted as that
      pane. A followed target is gone when its terminal ends or its pane or
      popup closes, whichever comes first.
    - An `attention` event passes `--state` when its new or its previous
      state is listed.
    - `workspace` and `queue` events pass a `--tab` or `--target` filter
      only when `--kind` names them.
    - When every workspace, tab and target that the filters name is gone,
      the stream ends with `{"type":"end","reason":"targets-gone"}`.
  - **Gaps.** A subscriber may hold 1 MiB of pending event output; snapshot,
    `TABS` and gap frames do not count against it. When pending events pass
    that bound, the Runtime drops them. Once the subscriber reads again, the
    Runtime sends one `{"type":"gap","seq","reason":"overflow",
    "snapshot":{...}}`, with `seq` the current event counter and the
    snapshot holding the `workspaces`, `tabs`, `attention_queue` and `busy`
    that a non-resumed snapshot line carries, then live events. Events are never
    skipped silently.
  - **Resume.** `--after STREAM:SEQ` resumes when STREAM names this supervisor
    process, SEQ is not above its current event counter, and the events it
    retains still hold every event after SEQ. It retains its last 1024 events,
    or fewer while those take more than 8 MiB as EONW frames. The snapshot
    line then carries `"resumed":true`, `seq` equal to SEQ and no state, and
    the matching events after SEQ follow. Otherwise the first line is a full
    snapshot with `"resumed":false`. STREAM identifies one supervisor process,
    so a supervisor restarted or recovered under the same generation (EON-C15)
    is a new STREAM.
  - **End.** The stream ends with `{"type":"end","reason",...}` and EOF.
    Its reasons are:
    - `ended`, with `generation`: Stop, quit or the window's final close;
      the CLI exits 0;
    - `targets-gone`: under Filters; the CLI exits 0;
    - `too-slow`: under Important failures; the CLI exits 1.
  - **Human output.** One line per message:
    - an event prints `seq`, the event name and its identities, and for
      `attention` its `state`, `app` and `name` only, as EON-C31's human
      projection;
    - a snapshot or gap line prints `seq`, the stream and its counts of
      workspaces, tabs and terminals;
    - an end line prints its reason.
  - **`eon pane wait`.** It subscribes filtered to TARGET, a pane or popup
    ID that it follows as `--target` does, and tests its predicate on the
    snapshot, on each gap's snapshot and on each event:
    - `done`: the target's attention state is needs input, error, finished
      or idle, or its terminal ended while the window still runs; no
      attention, `working`, `background` and `unavailable` are not done, so
      a wait started before the agent's first report does not return at
      once, and the window ending is never done but `target-gone`;
    - `needs-input`: its attention state is needs input;
    - `attention`: its attention state is needs input, error or finished,
      the states the attention queue holds;
    - `ended`: its terminal ended, its pane or popup closed, or the window
      ended.

    It exits 0 with the target's last attention object (JSON) or state
    (human). Without `--timeout` it waits until the predicate holds or the
    target is gone.
  - **Side effects.** Subscribing and waiting change no workspace, tab, pane
    or popup selection, shown workspace or popup, or desktop focus, and
    never count as an attached Venus, so they never make a raise seen.
- **Important failures:**
  - Refusals come as an EONW `Failure` reply before any snapshot:
    - exit 1: `subscriber-limit` (a 33rd subscriber of one supervisor),
      and `ambiguous-workspace` (EON-C33);
    - exit 3: a missing or stale workspace, tab, target, generation or
      window named when subscribing, and `unknown-caller` (EON-C10).
  - Exit 1 after subscribing:
    - `too-slow`: when a subscriber's pending output has not moved for
      30 s, the Runtime drops its unsent whole frames, writes what it can
      of any partly written frame and of an `end` with reason `too-slow`
      without waiting further, and closes the connection. Output has moved
      when the subscriber has read at least one write unit of up to 4 KiB
      since the last check. The CLI exits 1 with `too-slow` when that `end`
      arrives complete, and otherwise exits 4 as a lost connection;
    - `wait-timeout`.
  - Exit 3 `target-gone`: `eon pane wait`'s target
    closes, its terminal ends, or the window ends before the predicate
    holds, except under `--until ended`, which then succeeds. It is distinct
    from a target missing when subscribing, which is EON-C10's missing or
    stale failure.
  - Exit 2: invalid arguments, including a malformed `--after`.
  - Exit 4: a missing or unreachable supervisor, one that speaks another
    EONW version, and a connection lost without a complete `end`.
  - Exit 130: interrupt. A closed standard output ends the CLI quietly with
    exit 0.
  - A slow or stalled subscriber never delays a request, another
    subscriber, the attention poller or Stop.
- **Owner:** Eon Runtime: the event projection, sequence, retained events,
  filters, subscriber bounds and CLI projection. Its canonical EONW package
  owns the v31 `Subscribe` action and the `SUBSCRIBED`, `TABS`, `EVENT`,
  `GAP` and `END` messages.
- **Consumes:** EON-C10 (EONW v30 from `eon-ciwn.6`, targeting, exit codes),
  EON-C31 (attention records, the queue and its CLI visibility), EON-C11
  (busy terminals, generations), EON-C23 (window and supervisor start time)
  and EON-C33 (workspaces).
- **Boundary:** None of the following:
  - terminal content beyond what `eon workspace --json` already carries
    (EON-C10's listed exceptions): no screen content (EON-C35 owns reads)
    and no input (EON-C36);
  - an agent's menu options: OSC 7501 (ORB-C15) does not carry them, and a
    consumer reads them with `eon pane read`;
  - quotas or sidebar visibility;
  - Venus presentation: Venus renders from `Inspect`, re-reading it when an
    event arrives and every 250 ms as a fallback, and uses an event itself
    only to stop presenting a popup that was hidden;
  - events across a supervisor restart (a new STREAM starts with a
    snapshot);
  - remote or MCP exposure (`eon-ciwn.3`);
  - per-subscriber authorization: the socket stays private to the user, and
    `[agents] terminal_access` does not apply because no screen content or
    input crosses;
  - action discovery (`eon-cli-action-discovery-ogzi`), whose later `wait`
    consumes this subscription.
- **Proof:** None yet.
- **Open proof:** `eon-ciwn.8` proves `eon events` on the installed product
  with a real Claude Code question in a filtered tab; `eon-ciwn.9` proves
  `eon pane wait` with a real agent pane.
