# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Keep other tabs usable while a directory picker is open: create and focus
  panes, reorder panes or tabs, update launch directories, and close unrelated
  tabs without disturbing the picker. Tab selection by identity remains
  available; starting another picker or new tab still requires finishing it.
- Keep scrolling, cell/word/line selection, and tab/pane focus usable during
  compatible live output by composing accepted Orbit `91999d7` and Venus
  `e13970e`. Reading retained history leaves terminal parsing and query replies
  running, with existing geometry, history, and synchronized-output boundaries.
- Preserve retained scrollback motion when DEC 2026 synchronized output overlaps
  a semantic preview or signed scroll. Eon composes Orbit
  `64b225eb249490c9075894814942652a8b9d6192` with unchanged ORBS v10 and Venus
  `7f325a31d0052d84a1a09ff065c0e9103563c0e8`; Orbit holds one unresolved
  request until normal or watchdog release instead of failing it.
- Preserve touchpad fling momentum when Venus transiently times out acquiring a
  presentation texture during live output; explicit surface occlusion still
  cancels the gesture.
- Add stable-target `eon tab close TAB` and Ctrl+Shift+W closing for non-final
  tabs. Pending tabs stop their picker before removal; durable tabs stop and
  prune each confirmed Orbit Session and retain every still-live mapping after
  a partial failure.
- Add semantic CLI actions that move the active tab left/right or selected pane
  up/down by one adjacent position without wrapping, changing identity, or
  stopping any Orbit Session. Ctrl+Alt+H/L and Ctrl+Alt+K/J invoke the same
  movement directly without a mode.
- Keep Alt+H/L and CLI left/right tab traversal available while a directory
  picker is open. The picker stays bound to its original tab across ordinary
  and wrapped traversal, while unrelated picker-blocked actions remain blocked.
- Wrap left/right tab traversal and up/down pane traversal at ordered workspace
  edges when at least two targets exist; singleton axes remain unavailable
  without changing topology or Sessions.
- Keep retained scrollback responsive under continuous terminal output by
  composing Orbit's corrected bounded scroll-outcome queue admission with
  Venus's compatible viewport retention, transient routing retirement, and
  exact in-flight outcome admission.
- Open the ranked directory picker before the first durable Session in a fresh
  full-Eon workspace and in every Ctrl+T tab. Accepting starts exactly one first
  pane in the chosen directory. Cancelling fresh `t1` falls back to Eon's
  validated launch directory; cancelling a later pending tab removes it and
  restores prior focus. EONW v4 represents the pane-free pending tab, and Venus
  starts full Eon from the workspace endpoint without an initial Orbit endpoint.
- Open one tab-bound ranked directory picker from anywhere in a full-Eon tab
  with Alt+Z. Eon runs the packaged Zoxide and fzf in a transient Orbit Session;
  Venus keeps the tab bar visible and presents the picker with a one-cell inset.
  The exact picker ignores ambient fzf default options and uses a private socket
  no longer than the primary Session socket.
  Accepting a valid path retargets only future Sessions in the captured tab,
  cancel changes nothing, and an actionable error remains visible for a bounded
  interval before cleanup. Tab or presentation loss retries transient Orbit Stop
  failure and leaves no picker state. EONW v4 carries the semantic action and
  modal endpoint.
- Validate full Eon's initial tab launch directory before recovering or starting
  Sessions, so a startup filesystem race cannot report failure after launching
  a persistent Session.
- Keep every valid EONW v4 workspace snapshot representable within its bounded
  2 MiB frame, so accepted high-bound workspace actions still return their
  complete typed result.
- Give every full-Eon tab a stable `tN` identity and one authoritative launch
  directory, expose explicit retargeting through `eon tab directory`, and start
  later panes there without changing existing Sessions. EONW v4 carries exact
  raw directory bytes into Venus labels and the runtime-generation identity.
- Compose accepted ORBS v10 native cell, word, and logical-line selection,
  terminal mouse capture with Shift override, and automatic ordinary-plus-primary
  Wayland clipboard copy for installed dogfood. Host selection keeps only the
  latest unpresented frame so touchpad drag tracks promptly and stops at release.
- Compose accepted ORBS v7 bounded preview windows and Venus multi-row
  compositor-paced kinetic scrollback for installed native-Wayland dogfood.
- Give full Eon, standalone EonTerm, and approved EonTerm compositions distinct
  native Wayland application identities without changing terminal titles or
  child process state.
- Let the next launch replace exact owned Session records and sockets after
  both their Eon supervisor and recorded Orbit process have died unexpectedly.
- Wait for Eon's current delegated cgroup before forking a new Orbit process,
  preventing asynchronous desktop-launch placement from leaving Orbit in the
  inherited login cgroup without requiring a compositor or service-manager API.
- Make an attach-capable launch that overlaps clean exit of the last Session
  wait for the retiring supervisor and start one fresh Session instead of
  reporting a successful presentation against the ended Session.
- Name the installed launcher action `Open Eon`, identify panes as `p1`, `p2`,
  and so on, and show that identity, two spaces, then a `~/`-anchored path below
  home, an absolute path elsewhere, or Nova's exact-home marker. Unset or empty
  `HOME` leaves paths absolute. Exhausted numeric pane identities fail before
  starting another Session. New full-Eon windows omit the redundant native
  title bar while the selected terminal keeps compositor title semantics.
  The action invokes its exact packaged executable without depending on the
  desktop session's `PATH`; EonTerm remains decorated by default.
- Encode Kitty Space releases in Orbit and remove Venus's duplicate-input
  workaround, so new Eon launches deliver one space per physical press.
- Serialize Session startup with Orbit's exact retained Ready claim. Only an
  empty locked claim permits local-child rollback; after Orbit marks it, Eon
  uses management Stop for a rejected spawned identity and never falls back to
  a Child or PID signal.

- Refresh new Eon and EonTerm launches to the accepted post-audit Orbit and
  Venus sources, including bounded renderer failures and retained events,
  coherent native input and accessibility, and corrected X11 lifecycle behavior.
- Recover exact current-generation Orbit runs after same-boot Eon or EonTerm
  supervisor loss, preserving their process, PTY child, and terminal state while
  projecting full Eon into one deterministic numeric workspace; the accepted
  installed boundary covers three full Eon Sessions, one EonTerm Session,
  competing replacement controls, and unrelated-Session isolation.
- Acquire Orbit's sole canonical management lease before publishing Sessions or
  Venus, and complete whole-generation stop only after every exact Orbit
  tombstone; uncertain recovery or stop fails without PID or signal fallback.
- Bound management connection and partial-response pressure inside the same
  five-second recovery deadline.
- Finish the supervisor after an exact Stop even when the EONW client disappears
  before receiving its result, without reviving stopped Sessions or stale state.
- Refresh the canonical composition to Orbit ORB-C13/ORBS v4 and the accepted
  stream-supervised Venus, whose replacement lifetime follows Eon's control EOF.
- Consume Venus's blue cursor-tail default for new and replacement Eon and
  EonTerm surfaces without adding Eon-owned cursor configuration.
- Request compositor-owned terminal background blur by default for Eon and
  EonTerm, allow strict `terminal.background_blur = false` opt-out, and keep
  opacity independent when Venus is created or reopened.
- Accept finite `terminal.background_opacity` values from `0.0` through `1.0`
  for Eon and EonTerm, default to `0.80`, and apply the current value whenever
  Venus is created or reopened without restarting its live Orbit Session.
- Preserve Sessions through client-pressure disconnects, transient PTY closure,
  and concurrent stale-socket claims while rejecting contradictory presentation
  capabilities.
- Return a committed workspace snapshot when Session startup finishes within
  Eon's accepted work bound instead of reporting the supervisor unavailable.
- Preserve live replacement supervisor sockets and generation directories when
  dead-generation inspection races with startup, without waiting on its lock.
- Report empty and overlong focus identities as local `malformed-action` errors
  without contacting or blaming the Eon supervisor.
- Preserve the existing Venus window and request native presentation when a live
  Eon or EonTerm generation is launched again.
- Present synchronized terminal updates atomically in new Eon and EonTerm
  Sessions, while bounding producers that omit the closing marker.
- Render adjacent Unicode full-block cells without visible seams in packaged
  native Eon and Eonova terminal surfaces.
- Initialize new Eon and EonTerm Sessions with Eon's vivid ANSI palette while
  preserving program overrides, direct RGB, extended colors, and live older
  runtime generations.
- Expose generation listing, exact attach, and owner-routed stop through
  `eonterm` so its separate runtime namespace remains manageable across updates.
  Reconnect guidance names the exact generation.
- Paste ordinary native clipboard text through Orbit's semantic normal or
  bracketed-paste path in packaged Eon and Eonova surfaces.
- Deliver bounded terminal-requested text to the ordinary or primary native
  clipboard in packaged Eon and Eonova surfaces.
- Render Unicode Braille progress cells with distinct dots in packaged native
  Eon and Eonova surfaces while preserving their cell-grid bounds.
- Preserve Shift-produced layout text such as `?`, `:`, and `+` in the packaged
  native terminal surface without changing modified shortcuts.
- Component graph schema 3 removes inert artifact paths and launch declarations;
  Nix owns physical paths and Rust owns launch policy.
- Preserve native Fish per-command completions when Carapace is enabled while
  retaining its fallback definitions for commands without native coverage.
- Reject an invalid embedded component graph before selecting a runtime
  generation, creating runtime state, or starting a child.
- Serialize supervisor control-socket acquisition so concurrent launches
  converge on one live owner instead of unlinking a newly bound peer.
- Refuse a whole-generation stop when its validated supervisor is replaced
  during confirmation, preserving the replacement and its live Sessions.
- Resolve the four stable managed-shell commands from the Session-private PATH
  so direct package launches do not depend on an ambient Eon profile.
- Include generated managed-shell and private Session command behavior in the
  runtime-generation identity so profile updates select a fresh supervisor
  without stopping older work.
- Preserve every opaque Orbit endpoint byte in workspace-action JSON by
  reporting `endpoint` as an ordered integer array instead of a lossy string.
- Publish `eonterm [--no-decorations] -- COMMAND...` as the slim reusable
  terminal product. It preserves Eon's proved exact-command lifecycle while
  excluding the managed shell and tool bundle. Remove the former `eon terminal`
  spelling without an alias.
- Supply Symbols Nerd Font Mono to packaged Venus so Yazi can render its
  supported private-use icons without an ambient font.
- Exit quietly when a downstream reader closes piped CLI output while keeping
  other detected stdout write failures explicit.
- Isolate supervisors by a deterministic runtime-generation identity. Bare
  `eon` attaches only to the exact current generation while older live and
  fixed-namespace legacy workspaces remain discoverable. Add deterministic
  human and JSON generation listing, explicit compatible attach, and confirmed
  owner-routed whole-generation stop without PID or process-tree inference.
- Extend EONW v1 with additive supervisor identity and stop actions whose result
  type remains separate from the pinned Venus workspace response.
- Supply pinned Nushell, Bash, Zsh, Fish, Starship, Zoxide, Atuin, Carapace,
  Helix, Yazi, and LazyGit through bounded `eon-*` commands and a
  Session-private PATH. Let users select one direct shell argv and disable each
  shell integration in `config.toml`; load native shell and tool configuration;
  preserve existing prompt, completer, and same-tool hooks; keep Nushell user
  autoload last; honor `ATUIN_NOBIND`; and suppress Nushell's stock banner.
- Keep the user's global shell environment unchanged, resolve a relative Eon
  configuration root once, and reject ambient Helix, Yazi, or LazyGit
  configuration-path overrides that bypass Eon's private root.
- Add Eon-owned live tabs and vertical pane stacks with stable-ID and direct
  directional CLI actions, deterministic JSON output, and one Orbit process per pane.
- Remove a pane when its Orbit Session exits, select the nearest survivor,
  remove an empty tab, and close Eon when the final pane exits.
- Expose those actions and complete accepted workspace snapshots through the
  bounded, versioned, dependency-free EONW v1 local protocol while preserving
  human and JSON CLI projections.
- Render every fitting pane header around one selected Session, bind Alt+H/L and
  Alt+K/J traversal plus Alt+M and Ctrl+T creation, refresh external actions
  within one second, and keep reattachment available while another pane remains
  live.
- Keep terminal text and Unicode table borders on the authoritative Venus cell
  grid, and anchor shaped input-method preedit text and underlines at the cursor.
- Recover the selected live Session automatically after retryable local socket
  loss while retaining its last coherent presentation until a fresh frame.
- Ship the x86_64 Linux alpha through one locked Nix flake with an Eon desktop
  entry that groups its X11/Xwayland window and uses a transparent violet
  three-fold loop icon rendered at native launcher sizes.
- Add `eon`, `eon run -- COMMAND...`, `eon attach`, `eon versions`, and
  `eon config-path`.
- Make bare `eon` and the desktop launcher reconnect to the live workspace;
  keep `eon run` as the explicit start command.
- Supply pinned Mesa Vulkan drivers to desktop-launched Venus instead of
  depending on shell-only graphics environment variables.
- Keep Orbit sessions alive across Venus exit and reconnect them through the
  same private local socket.
- Ignore relative XDG base paths, create missing configuration and runtime
  directories with mode `0700`, preserve existing configuration-directory
  permissions, and reject unsafe existing runtime directories without mutation.
