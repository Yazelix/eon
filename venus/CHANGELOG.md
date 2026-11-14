# Changelog

## Unreleased

- VEN-C14: report native window focus and the keyboard-focused terminal using
  canonical EONW v45 frames; chrome, dialogs and unavailable attachments report
  no terminal. Private supervision modes use v2 (`eon-81emk`).

- VEN-C21/VEN-C30: Background attention uses a calm round-dot pulse on the shared
  tick; Idle rings and Unavailable dots use anti-aliased round coverage. The
  existing animation-off/CPU guards keep both animated states still
  (`eon-sidebar-app-icons-y99b7.10`, delegated approval 2026-11-14).

- VEN-C30: centre Agents sidebar and attention inbox status glyphs and app marks
  across the full row, enlarge their boxes, and use crisp direct-size Clawd/Codex rasters
  (`eon-sidebar-app-icons-y99b7.9`).

- VEN-C30: Codex sidebar and inbox rows use a colour vector of Codex's symbol,
  preserving its colours and transparency on stale rows. Quota chips keep
  provider marks (`eon-sidebar-app-icons-y99b7.8`, delegated approval 2026-11-13).

- VEN-C30: Claude Code sidebar and inbox rows use Anthropic's official standalone
  Clawd symbol in its supplied colour, including on stale rows. Quota chips retain
  their provider marks
  (`eon-sidebar-app-icons-y99b7.7`,
  delegated approval 2026-11-13).

- Consume canonical EONW v44 lifetime/build metadata and the `updating` End
  reason; a closed subscription reconnects without retaining partial frames.
  Reserved Stop and workspace presentation are unchanged (`eon-t3fyd.2`).

- Consume the canonical workspace wire with separate Google quota fact and
  failure slots (`eon-o9f3h.1`). Selected Google windows show text chips
  with exact groups, window position, remaining share and stale state, plus
  hover and accessibility read/reset facts (`eon-o9f3h.5`).

- VEN-C30: attention badges have no backing at any opacity, with two-surface
  colour adaptation and a glyph-only outline when needed
  (`eon-0ntg1`, approved 2026-11-12).

- VEN-C30: attention badges adapt every glyph colour for 3:1 before stale
  dimming, with no backing on opaque surfaces and a quiet opaque rounded
  square on translucent rows (`eon-wuau8.7`, delegated approval 2026-11-12).
- Consume canonical EONW v42 pane tags. Tag-only refreshes preserve workspace
  navigation and attachments; tags remain data for the later Agents row
  presentation change (`eon-xvz5c`). Reserved Stop is unchanged.

- Consume canonical EONW v41, which distinguishes Editor-popup edits from
  earlier builds that assume pane creation (`eon-xodrj.5`). The message layout
  and Reserved Stop are unchanged.

- VEN-C30: the Agents sidebar shares inbox wheel/touchpad scrolling, retains
  its position across attention refreshes and shows a neutral overflow thumb
  beside its rows (`eon-r1scq`, delegated approval 2026-11-12).

- VEN-C8/VEN-C18: panel headings default to `connected`; `notch` remains a
  supported setting. Both drawings and pane header bands are unchanged
  (`eon-j7ii2`, Lucca's 2026-11-11 decision).

- VEN-C30: the current Agents row uses a one-physical-pixel focus-colour outline
  without a selection fill. Hover and keyboard focus keep their dark fill on
  every row; mint keyboard focus sits one pixel inside the current outline.
  Tabs retain their selected fill (`eon-wuau8.4`, Lucca's 2026-11-11 approval).

- Selected tabs use a brighter fill; keyboard focus stays distinct and adapts
  for contrast. Resting tab and sidebar heading numbers are dimmer than their
  names, hint and dialog text use grey roles, and zoomed tabs share the pane
  Restore mark `▣`
  (`eon-wuau8.3`, Lucca's 2026-11-10 approval and delegated contrast decisions).

- VEN-C8/VEN-C18: `--panel-heading connected` uses a straight panel top at notch
  height, retaining title/action placement and the existing compact fallback.
  Pane header bands are unchanged (`eon-6ui6p`).

- VEN-C21: bar widgets and the session label use the idle-tab capsule fill,
  CPU/RAM/disk percentages use the interface font in fixed slots, and Shortcuts
  draws a plain `?`. Session hover and press step up the tab fills; mint focus
  remains. The shared critical/pace red meets 4.5:1 on the capsule fill
  (`eon-wuau8.2`, Lucca's 2026-11-10 approval and delegated contrast amendment).

- VEN-C30: popup-source agent rows draw two overlapping rounded-square outlines
  at the existing right edge. The front square hides the rear stroke; both
  use the neutral card stroke (`border_width`), reduced only when it would
  fill the smaller squares (`eon-wpbjz`, `eon-5bft1`).
- VEN-C8: pane header bands retain the shared column background and opacity;
  terminal edge backgrounds extend only into terminal padding, including popup
  margins and frameless gutters, below the terminal's top edge. No wire or
  geometry change (`eon-jg86j`).

- The first expanded Stack pane's band and focus frame sit one title row
  below the Stack outline, matching the gap below collapsed titles. Existing
  multi-pane clearance is redistributed without losing terminal rows. A lone
  or zoomed pane reserves half a title row and loses about one terminal row
  (`eon-yjrru`, VEN-C8).

- VEN-C8/C18: each Stack tab carries the shared right action notch holding
  one Close button. It sends EONW v39's column close with no consent, and
  Runtime's `Confirm` opens a prompt worded from that answer alone. Enter
  counts only 400 ms and one full event-loop turn after the list's first
  presented frame; an accepted Enter sends the consent that repeats the
  answer, and a second `Confirm` asks again ("This Stack changed."). Tab,
  Enter or Space and AccessKit (`Close Stack`, `Close Stack N`) share the
  scene's targets. No wire change (`eon-t5boa`).

- VEN-C8: pane Zoom/Restore and Close draw and accept clicks only on the tab's
  selected pane, except for one-pane Stacks. Hidden controls keep their title
  space reserved; keyboard and AccessKit actions retain their availability.
  No wire change (`eon-vzf2k`).

- Consume EONW v39; the Agents Hide button sets the sidebar closed through
  `SetSidebar { open: false }` (`eon-cq38r`).

- VEN-C18/C30: popup and Agents headings carry a right action notch mirroring
  the title notch, holding one Hide button. The popup's sends `DismissPopup`
  for the shown instance; the sidebar's sends `ToggleSidebar` while the
  sidebar is open. The notch drawing is shared by every framed title tab;
  pane bands, Stack tabs and dialogs are unchanged. Pointer release, Tab and
  Enter or Space in the sidebar, and AccessKit buttons (`Hide popup <label>`,
  `Hide agent sidebar`) share the scene's targets. No wire change
  (`eon-5g0e5`).

- VEN-C8/C18: the expanded, lone or zoomed pane's header forms one continuous
  raised band across the fitted title and right Zoom/Restore and Close group.
  Collapsed titles sit in compact gaps in their row's line. When room permits,
  the band starts one row's height below the preceding collapsed title line.
  Tight-clearance titles retain compact gaps. Title focus colours
  its edge span; title hover and button focus use rounded pills. Pointer release,
  keyboard traversal and AccessKit share the existing exact-pane targets, local
  zoom and final-close path. Stack, popup, Agents and dialog tabs stay unchanged.
  No wire change (`eon-7xryp`, `eon-ipc6f`, `eon-2ibdd`, `eon-u5uv2`, `eon-xaq6c`).

- Codex and Claude quota chips show pace colour and hover/accessibility text
  from 1% used, with no elapsed-time gate. Windows with no quota used are
  neutral at any age and show no pace line (`eon-7aw48`).

- VEN-C5: plain-text loopback hosts with a port (`localhost`, `127.0.0.1`,
  `[::1]`) are detected as links targeting `http://` and the text, unless
  `--detect-local-hosts false` (`eon-onvk0`). New launch flag, no wire change.
- VEN-C8/C18/C21/C30: each column card's top line carries
  the shared tab, labelled `Stack` or `Stack N`, one stack band above the
  first pane; headers scroll below the band. Title tab tops sit one card gap below the Eon Bar's
  pills (`eon-whjhd`). No wire change.

- VEN-C18/C30: popup, Agents and native titled dialog headings share one fitted
  tab with a tangent S return spanning 1.5 times its rise. Native dialog titles
  use the chrome title style. Title clearance, terminal rows and accessibility
  bounds remain unchanged (`eon-27deg`).

- EONW v38 carries nullable recovered session IDs and registration diagnostics.
  A registry failure shows a notice while the recovered terminal remains usable.

- VEN-C18: a popup's catalog label sits in a rounded tab continuous with its
  left and top frame edges, on the terminal background and opacity. The original
  border-gap title remains when the tab's clearance cannot fit
  (`eon-popup-name-tab-89wv5`). No wire or terminal geometry change.

- CPU, RAM and disk chips use common yellow/orange/red percentage grades,
  with descriptive hover and accessibility text; CPU use can also turn red
  and keeps the stronger utilization or temperature colour
  (`eon-chip-grade-60-21mnn`). New launch flags, no wire change.

- VEN-C30: popup-source sidebar and attention inbox rows in the shown session
  share pane-row fills, with a larger window mark at the padded right edge and
  text clipped before it. Resting row text and marks adapt to light terminal
  backgrounds; focus outlines and accessible names stay intact (`eon-x8d5x`).
  No wire change.
- VEN-C21: the leading Eon icon opens the accessible Window menu and stays
  reachable while tabs overflow. The session pill keeps its Sessions picker
  (`eon-7cf0y.6`).
- VEN-C30: a pane's pinned name leads its agent sidebar row and its attention
  inbox row in the shown session, ahead of reported names, with the app icon
  retained (`eon-sidebar-pinned-name-asubh`). No wire change.
- Consume EONW v38 session identities and typed session targets. Compact
  labels and pickers keep their local handles; Reserved Stop is unchanged
  (`eon-remote-workspaces-52i4.5`, EON-C38).

- VEN-C8: Quit sends EON-C11's Reserved Stop and reads its reply, in place of
  the EONW Stop that v37 removes (`eon-7cf0y.17`). Venus consumes EONW v37.
- VEN-C8: on macOS, Command+Q in an Eon workspace window acts as a close
  request; Venus takes it off winit's default app menu, and the Shortcuts row
  reads "Option+Shift+Q or Command+Q". A standalone Orbit session keeps
  AppKit's quit (`eon-7cf0y.15`). No wire change.
- VEN-C21: the clock in the bottom shortcut hints is plain text in the version
  label's colour, with no outline, its inset mirroring the version's; the
  keycap hover popup has no title row
  (`eon-e3a1z.3`). No wire change.
- VEN-C21: the clock moves from the header to the left end of the bottom
  shortcut hints, which hide it with them (`eon-e3a1z.2`). No wire change.
- VEN-C21: the version label moves from the header to the right end of the
  bottom shortcut hints, which show one centred keycap per set of modifiers
  with a hover popup (`eon-e3a1z.1`). No wire change.
- VEN-C21 and VEN-C32: the header Shortcuts button always shows, and the
  footer has no Alt+Q inbox hint (`eon-yvhke`). No wire change.
- VEN-C14: a supervised workspace Venus reports its native window focus to
  Eon on its private presentation stream, after startup readiness when
  admission is requested, without waiting on Eon (`eon-2kvr`). The private
  modes become `stdin-ready-focus-v1` and `stdin-focus-v1`; the old mode names
  are no longer accepted. No EONW change.
- Confirming a final-close prompt sends its named tab or pane close directly.
  If another tab became active while the prompt was open, it stays active
  (VEN-C8, `eon-agent-parity-a9g0.3`).
- Consume EONW v35, which changes no message: only Eon's CLI forms change
  (`eon-7cf0y.22`, EON-C11). No Venus behavior change.
- Consume EONW v34, which adds `EditTerminalHistory` and its `Editing` reply
  (`eon-u5bvd.5`, EON-C35). No Venus behavior change.
- Consume EONW v33, which adds `ReadTerminalHistory` and its `History` reply
  (`eon-u5bvd.3`, EON-C35). Venus still reads every message under the 2 MiB
  bound. No Venus behavior change.
- VEN-C20: a pointer press outside the shortcut viewer's dialog closes it, as
  Escape does; that press and its matching release, from any button, reach no
  other target (`eon-gl1xo`). No wire change.
- Consume EONW v32, which adds `SendTerminal`'s force flag (`eon-buys1.2`,
  EON-C36). No Venus behavior change.
- VEN-C8: physical Alt+Shift+1 through Alt+Shift+9 and Alt+Shift+0 show
  workspace positions 1 to 10 through `SwitchWorkspace`, unless
  `--number-keys false` leaves them to the terminal or a configured popup takes
  one (`eon-hbpkq`). No wire change.
- VEN-C8: when Eon has not answered a workspace request within two seconds,
  Venus shows "Eon is slow to answer." and changes nothing else; the next
  snapshot clears it (`eon-8wh7h`). A request that fails or reaches its
  timeout still makes the workspace unavailable, now with "Cannot reach Eon."
  when Venus cannot connect and "Eon is not answering." otherwise; the
  operating-system error text goes to Venus's diagnostics instead of the
  notice. No wire change.

- VEN-C21: the Eon icon's room reserves a chip for each window a provider's
  latest read reported, each at that window's own widest, or Codex's one
  blocked or unknown chip, instead of Codex's two chips and Claude's two
  unscoped chips plus its scoped ones at the provider's widest; a read gaining
  or losing any window may now show or hide it once (`eon-nfrbn`). No wire
  change.

- Subscribe to EON-C39 workspace events, once per window (`eon-61efs.1`).
  Venus re-reads `Inspect` as soon as an event arrives, keeping the 250 ms
  refresh as a fallback. It stops showing a popup as soon as its
  `popup-hidden` event arrives, before it applies a frame drawn after it. No
  wire change.

- Consume EONW v31, in which a pending tab holds its Files popup instead of a
  Project popup (`eon-quick-search-parity-23y9.2`). Venus renders it as any
  selected popup and presents nothing new; its Project shortcut now names the
  tab's Files popup as the expected instance.

- Consume EONW v30, which adds the EON-C39 event subscription messages
  (`eon-ciwn.6`). Venus keeps polling `Inspect` and presents nothing new.

- VEN-C8, VEN-C14 and VEN-C20: a compositor close request quits as Alt+Shift+Q
  does, prompt included, including one a desktop sends to each window at
  logout, and a close request while the quit prompt is open leaves it open. It
  still detaches without a running-windows summary, while the workspace is
  unavailable, and while it refuses refreshes after a failed Stop.
  Alt+Shift+D alone detaches, listed as "Detach: hide the window, keep
  terminals running" (`eon-7cf0y.16`). No wire change.

- VEN-C8, VEN-C14 and VEN-C20: Alt+Shift+Q quits an Eon window with the EONW
  Stop that `eon quit` sends, behind the final-close prompt's owner while the
  snapshot lists a busy terminal; Venus reads Stop's lifecycle reply, so
  `Stopped` exits as `Ended` does and `Failure` shows as a refused action's
  notice (`eon-7cf0y.3`). No wire change.

- VEN-C21: the Eon icon may lead the Eon Bar before the workspace label and
  tabs, only while everything else would show with it at its widest and the
  tabs fit unscrolled, however wide the window (so only a provider's quota
  first appearing or going away, or a read finding a scoped window gained or
  lost, may show or hide it, once; no window's reset does);
  Keyboard shortcuts is a round button with its glyph centred; the version
  label sits between Keyboard shortcuts and Close; the icon's centre and
  Close's glyph ink sit as far from the window's sides as from its top (Close's
  centred instead where that would leave its target), with as much space after
  the icon as before it. Eon passes the icon as
  `--eon-icon PATH`, a square premultiplied RGBA image Venus reads once at
  startup; a missing or unusable file shows no icon and never fails launch
  (`eon-369s`). No wire change.

- VEN-C7: a held selection keeps scrolling into history while its pointer is
  above the pane's top row, including over the Eon Bar, a pane above, or the
  header's scrollback pill, with the tick position clamped to the top row and
  the pane's columns (`eon-5uw0`). Outside the window this is proved on
  Wayland only. No wire change.

- Consume EONW v29, which adds the window's busy terminals (`eon-7cf0y.12`).
  This change supplies data only; Venus presents nothing new.

- VEN-C1: accept a zero cursor-tail duration to disable motion while keeping
  the cursor's color and contrasting edge (`eon-configurability-qkq8.6`).

- Consume EONW v28, which carries the window's running-windows summary and
  supervisor name and start time (`eon-7cf0y.4`). This change supplies data only;
  running-window presentation remains separate.

- Consume EONW v27, which adds a scope to quota windows and each provider's
  last failed quota read. Claude's scoped windows show as their own chips
  after its unscoped chips (`eon-wqkjc`); Venus shows no failure.

- VEN-C20: Alt+H, Alt+J, Alt+K and Alt+L join Alt+Slash and Alt+0 through
  Alt+9 as keys no popup takes, in dispatch, hints and the viewer; Eon loads a
  popup configured on one of the four without a key (`eon-0gllz`). No wire
  change.

- Consume EONW v26, which adds EON-C35's and EON-C36's terminal reads and
  sends and their replies. Venus sends neither (`eon-agent-parity-a9g0.16`).

- VEN-C9, VEN-C11, VEN-C15, VEN-C22 and VEN-C24 to VEN-C29 are retired,
  demoted to features (`eon-vn0nk`). Their behavior is in
  `docs/PRESENTATION.md`, and the bottom shortcut hints in the root
  `docs/USAGE.md`. VEN-C6 and VEN-C12, IDs the index never recorded, get
  Retired headings. No behavior or wire change.

- VEN-C22: Codex quota shows one chip per window, like Claude, hiding chips
  one by one under width pressure (`eon-7iml`). No wire change.

- VEN-C19: two-cell emoji clusters without text presentation prefer the
  platform colour emoji family at regular upright style, ahead of primary and
  named fallback fonts (`eon-wy1r`).

- VEN-C32 (Partially proved): Alt+Q shows EONW v25's attention queue in one
  native inbox across workspaces, with four counted filters, keyboard and
  pointer jumps, and AccessKit rows. Rows share the agent sidebar's
  presentation and retain their highlight across updates. Popup overrides
  take Alt+Q. No wire change.

- Consume EONW v24, which adds EON-C10's directory-based tab launch request.
  Venus's new-tab action keeps the directory picker (`eon-tab-create-launch-8tl4`).

- VEN-C28: on Apple Silicon macOS the CPU and RAM chips read the Mach host
  statistics instead of staying hidden. RAM is Activity Monitor's Memory
  Used, app memory plus wired and compressed pages, over `hw.memsize`
  (`eon-mac-launch-kz4p.3`).

- VEN-C21/VEN-C30: consume EONW v23, whose attention state adds background,
  waiting on background work, ranked between working and idle
  (`eon-agent-background-work-yte0.2`). VEN-C21/VEN-C30: its tab badge and
  sidebar row glyph are three still dots in the working blue that never animate, and
  their accessible state is `waiting on background work`
  (`eon-agent-background-work-yte0.3`).

- VEN-C22/VEN-C26: each fresh quota window with a reset, past its first tenth,
  colors its text on a green, yellow, orange, red gradient by how early its
  linear pace runs out before the reset; a used-up window is always colored.
  Hover and the accessible description say at this pace it lasts until reset,
  runs out about a stated time before reset, or is used up. Stale, blocked,
  unknown and reset-less windows keep the neutral color (`eon-l8lt`).

- VEN-C21: the working badge draws braille dots stepping through
  ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ at no more than 10 Hz instead of an open arc turning at 5 Hz.
  The new `--animate-working-badge true|false` launch argument, true by
  default, keeps it at the still frame ⠋ when false, as on a software adapter
  (`eon-spinner-braille-dots-hmg0`).

- VEN-C31 becomes Proven: on the installed Eon and a hardware Vulkan device,
  Yazi's RGB and RGBA image previews drew in a pane and in the Files popup
  and, in the pane, across resizing, deletion, the alternate screen, detach
  and reattach; a directly placed image scrolled with its rows and back from
  history.
  Behavior is unchanged (`orb-j2i.12`).

- VEN-C31: Venus uploads Orbit's image resources into one window-wide GPU
  cache of at most 256 MiB and draws each frame's placement entries between
  cell backgrounds and text, in panes, other columns and popups. An entry
  whose image Venus does not hold shows a marker in the terminal's foreground
  colour, at once or after 250 ms pending; a frame with omitted placements
  raises a notice. Venus reattaches a terminal whose frame needs an image it
  freed, or refused for the budget once there is room. The resource
  retirement message now frees the texture once no frame references it.
  Orbit turns ORB-C16 on in the same change (`orb-j2i.11`).

- VEN-C21: consume EONW v22, whose workspace summaries carry each workspace's
  attention rollup and recency. While another workspace needs input, the Eon
  Bar's workspace label shows the needs-input badge at its corner and its
  accessible name ends with ", another workspace needs input"
  (`eon-workspace-switching-shn4.8`).

- VEN-C20: consume EONW v21, whose popup entries may have no shortcut. Only
  entries with one dispatch, take a fixed key, or show in hints and header
  tooltips. Alt+/ lists an entry overridden by another under its default
  shortcut as "<label> (overridden by <popup label>)" and omits an entry the
  configuration left without a key (`eon-configurability-qkq8.1`).

- Consume ORBF v3 in ORBS v14. Frames may carry ORB-C16 placement entries,
  which Venus does not draw yet, and a resource announcement, chunk or
  retirement fails the attachment as an unexpected message until VEN-C31's
  store lands; Orbit sends none while it states Kitty graphics unsupported.
  Draw rows drop the placeholder flag (`orb-j2i.5`).

- VEN-C8: closing the last tab or its last pane shows the final-close prompt
  whatever the workspace count. While the snapshot lists other workspaces it
  reads "This closes workspace NAME and stops N terminals." with "Enter Close ·
  Esc Cancel", and an open prompt re-words from each snapshot (EON-C33,
  `eon-workspace-switching-shn4.7`).

- VEN-C21: while the snapshot lists two or more workspaces, or its one
  workspace has a name, the Eon Bar leads with an outlined workspace label,
  `NAME N/M`, read from EONW v20's `workspace` and `workspaces`. Clicking it,
  or Enter or Space on it, invokes the `workspaces` popup entry; without that
  entry it is inert text. AccessKit orders it first, as `Workspace NAME, N of
  M`. Quota chips yield before it, and it yields before the tabs' minimum
  (`eon-workspace-switching-shn4.7`).

- VEN-C8/VEN-C11: edge backgrounds extend through the expanded pane's header band
  below its title rule, with the same top-padding exceptions and rounded
  clipping. Title text, cutouts and outlines stay on top; background mode,
  collapsed rows, popups, zoom and frameless mode retain their header paint.
  The expanded label's pane ID dims only when it also keeps 4.5:1 against the
  band's extended colours (`eon-q1u6`).

- VEN-C28: the CPU tooltip and AccessKit label name the logical CPUs the
  percentage covers, as `CPU use 12% across 16 logical CPUs, over the last 2
  seconds`; a `/proc/stat` sample without per-CPU lines shows no label
  (`eon-icoh`).

- VEN-C24: hint modifier headers are steel blue, not coral red, so the footer
  carries no error color (`eon-ffjr`).

- VEN-C7/VEN-C8: consume Runtime's bounded pane labels and pins from EONW v20;
  retire separate workspace label observers. IDs dim when contrast permits,
  fitting retains their identity, and accessibility/close lists keep full
  labels. Label-only updates preserve zoom, scroll and attachments (`eon-a7xd`).

- Decode canonical EONW v20 workspace snapshots; keep the shown workspace
  presentation and preserve view state on summary-only updates. No workspace
  controls or keys are added.

- VEN-C11: terminal padding extends edge cell backgrounds inside rounded cards, with primary-screen default-background and powerline exceptions; `--padding-color` selects `extend` (default), `extend-always`, or `background` (`eon-pane-padding-extend-d9t8`).

- VEN-C8: the zoomed tab marker is a baseline-aligned Nerd Font U+F00E magnifier;
  width fitting includes it, `, zoomed` stays accessible, and a missing face uses
  ` · zoom` (`eon-x7l8`).

- VEN-C21: card areas start lower while bar pills, widgets and controls stay
  in place. The gap to the tops of the card titles' capital letters matches
  the space above the pills; the added space yields first on short windows
  (`eon-hg2v`).

- VEN-C8: zoom shows only the selected pane as one card, with a `Zoomed pane <label>` panel; selection or structural changes end it, and a one-column stack can zoom (`eon-tfsg`).

- VEN-C8: the expanded bottom edge keeps the collapsed-row pitch below it, bounded by padding and the requested/displayed grids, including retained frames during resize (`eon-6ros`).

- VEN-C8: a collapsed last pane's label closes the stack in its rounded bottom
  edge, with no second line below it. Column and Agents cards share that snapped
  painted bottom line, bounded by terminal padding to keep strokes clear of grid
  cells after fractional-scale resize rounding; a last expanded frame ends there too
  when space permits. Short, scrolled viewports clip bottom strokes below the grid.
  Terminal layout, grids, hit targets
  and AccessKit bounds stay unchanged, as do frameless and popup paint (`eon-glp0`).

- VEN-C8, VEN-C21, VEN-C27 and VEN-C28: tab pills and Eon Bar widgets share
  one centred height. Tabs keep their fills; quota, CPU, RAM and clock widgets
  use unfilled rounded outlines in the pane cards' neutral colour and configured
  border width. Version and header controls share their centre. On short windows,
  the top gap yields before the pills shrink (`eon-u2id`).

- VEN-C8: the selected pane's outline defaults to 1 logical pixel (`eon-q1of`).

- VEN-C8: each pane has its own rounded frame without added stack spacing;
  collapsed title edges turn toward the expanded pane and their sides join the
  next card, continuing around the column's bottom. The expanded frame stays
  unchanged (`eon-m5bv`).
  Configured neutral/focus widths paint each frame once. VEN-C18 and VEN-C30
  popup/sidebar titles share the pane-title treatment. Each column has one
  continuous terminal-background fill from its first rounded top edge to its
  rounded bottom, including stacked title rows and the space between cards.
  Popup and sidebar fills follow their outer curves too; the space above and
  outside stays opaque frame (`eon-54jv`). Grids, full-row targets and AccessKit
  bounds stay unchanged (`eon-2x6h`).

- VEN-C8: `--border-width` and `--focus-border-width` accept 1–3 logical pixels
  (defaults 1 and 1) for neutral card outlines/title rules and the selected pane
  outline. Display scaling rounds each to whole physical pixels (defaults at
  scale 1.3: 1 and 1; at scale 2: 2 and 2). The agent sidebar and popup cards
  share the neutral width; the current agent-row outline stays one physical
  pixel (`eon-f5pg`).

- VEN-C8: the selected pane's title rule, sides and bottom form an outline in
  the focus colour. `--focus-color`
  accepts cursor presets or custom hex colours and defaults to `custom:#b207fb`;
  F6 focus pills stay mint, and frameless mode draws no outline (`eon-e8hh`).

- VEN-C8: compact pane headers share the VEN-C30 title metric. Mid-row rules
  break around each label; the first forms the card top. Selection brightens
  the label and accents its rule, hover adds a label pill, and keyboard focus
  outlines that pill. Frameless headers retain the same full-row hit targets
  and AccessKit bounds without rules or card borders. VEN-C18 popup cards and
  the VEN-C30 sidebar share the column cards' painted top; popup labels keep
  their terminal gutter, and on a window too short for one padded terminal
  cell below that top a popup's top edge rises to keep it (`eon-mur2`).

- VEN-C30: the agent sidebar row of the terminal that has keyboard focus in
  the active tab, a pane or the shown popup, takes a one-pixel outline in the
  pane cards' outline color, over any hover or focus highlight and under a
  focused row's accent outline, and its accessible name adds "current". A
  4 px gap separates each tab heading from its first row. Tab headings no
  longer end with the popup marker, and their accessible names no longer add
  "popup shown"; popup rows keep the marker (`eon-sidebar-you-are-here-3743`).

- VEN-C5 and VEN-C8: a link success notice ("Link handed to the desktop
  handler." or "Link copied.") clears on its own after about 2 seconds;
  failure notices still wait for input. No notice disables link hover, open or
  copy. The status line shows the newer of a workspace notice and a link
  result, and a hovered link's hint shows over a workspace notice, which
  returns when the pointer leaves (`eon-link-notice-lifetime-454q`).

- VEN-C21: the working badge, in Eon Bar tabs and VEN-C30 sidebar rows, is
  an antialiased open arc of 270° that turns clockwise in 45° steps. As
  before, it repaints at most 5 times a second, and only the badge. It
  replaces the segmented ring. On a software (CPU) adapter it is the same
  arc, drawn without turning (`eon-arc-spinner-t6li`).

- VEN-C30: Venus shows Eon's `no-attention` rejection of Alt+G or Alt+Shift+G
  as the bare VEN-C8 notice "No agent needs attention", without taking focus.
  Unlike other rejected-action notices, it also clears when a terminal's
  attention state changes and on the next input Venus sends to the terminal,
  as the clipboard notice does (`eon-empty-attention-notice-opy3`).

- VEN-C30: within a tab, a sidebar row whose source is a popup comes first,
  above the pane rows, which keep pane order. It carries the popup marker, a
  small window glyph drawn from quads, after its catalog label, whether or not
  the popup is shown, and its accessible name adds "popup". When a listed tab
  shows a popup (its `selected_popup` is set), its heading ends with the same
  glyph and its accessible name adds "popup shown" (`eon-agent-sidebar-yx8.16`,
  `eon-popup-marker-always-52rk`).

- VEN-C21: on a software (CPU) wgpu adapter the working badge, in Eon Bar
  tabs and VEN-C30 sidebar rows, is a still ring with every segment lit, and
  no spinner tick or redraw is scheduled. Other adapters keep the 5 Hz
  spinner (`eon-agent-sidebar-yx8.15`).

- VEN-C21 and EON-C21: while the agent sidebar is not visible, each Eon Bar
  tab whose EON-C31 rollup is needs input, error, finished or working shows a
  corner badge (◆, ✕, ✓ or a spinner). The badge is dimmed when every terminal
  in that state is stale, and it changes neither tab width nor label. The
  spinner turns at most 5 times a second by rewriting only its own vertices.
  Each tab-overflow edge shows the most urgent badge hidden past it, and the
  tab's accessible name appends its state (`eon-agent-sidebar-yx8.2`).

- VEN-C30 (Partially proved): the agent sidebar. While the snapshot's
  `sidebar` is `Open` and the window keeps at least 80 cells right of it, a
  28-cell sidebar sits below the Eon Bar and the cards share the content area
  to its right (VEN-C8 side gaps and CreateColumn check, VEN-C18 popup card
  area). Alt+Shift+H sends `ToggleSidebar`; it is unbound, and missing from
  the viewer and hints, under `Off` (VEN-C20, VEN-C24). The sidebar is an
  AccessKit region between the header and the panes, with an empty state when
  no tab has attention (`eon-agent-sidebar-yx8.3`). Each tab with attention
  gets a heading and one row per terminal with attention: state glyph, the
  VEN-C26 splat or VEN-C22 mark or the `app` verbatim, title or directory
  leaf, age from `changed_at`, source `pN` or popup label, a needs-input `msg`
  line, "+N" and a dimmed "status stale". A click, or Enter on the row F6 and
  Up/Down focus, sends `FocusAttention`; focus follows the result, and returns
  to the terminal when the focused row or the sidebar goes away. Rows are
  AccessKit buttons under tab headings. The card takes the column cards'
  outline, and the leftmost column's terminal background at the
  terminal-background opacity, which VEN-C11 now covers
  (`eon-agent-sidebar-yx8.13`). Alt+G and Alt+Shift+G send `NextAttention`
  and `PreviousAttention` under every `sidebar`, `Off` included, and move no
  keyboard focus themselves; a configured popup on either key takes it. The
  viewer lists them as "Jump to the next agent needing attention" and "Jump to
  the previous agent needing attention", and the hints as "next agent" and
  "previous agent" (VEN-C20, VEN-C24, `eon-agent-sidebar-yx8.8`).

- Venus consumes EONW v23, whose attention carries `name`, the terminal
  title. A sidebar row's first line shows one label: for `app=claude-code`
  the name, which is the session's name, else the record title, else the
  app's name (`Claude`, `Codex`, or the `app` verbatim). Other apps'
  terminal titles, such as Codex's spinner title, never lead a row, and the
  row no longer falls back to the tab's directory leaf, which the tab's
  heading shows. The row's accessible name includes Claude Code's session
  name (VEN-C30, `eon-agent-sidebar-yx8.11`).

- EONW v18 (now v23, above) adds EON-C31 agent attention to panes,
  popups and tabs, the window's sidebar visibility, and the `ToggleSidebar`,
  `FocusAttention`, `NextAttention` and `PreviousAttention` actions; the
  VEN-C21 and VEN-C30 entries above render and send them. VEN-C8 and VEN-C22: a
  snapshot that differs only in attention or sidebar visibility keeps a
  rejected-action notice and disturbs no navigation, scroll or attachment
  (`eon-agent-sidebar-yx8.7`).
- VEN-C8: a final close from Alt+Shift+W, the Eon Bar's Close tab or Alt+W
  first shows a native "Close the last tab?" alert dialog listing the tab's
  panes and popups; Enter sends the close and Escape dismisses it. The new
  `--warning-close-last-tab true|false` launch argument, true by default,
  turns it off. VEN-C14: Alt+Shift+D detaches like a compositor close request.
  VEN-C20/VEN-C24: Shortcuts and the footer list Alt+Shift+D
  (`eon-final-close-prompt-and-detach-key-m798`).
- VEN-C3, VEN-C4: when Venus's bounded Orbit event queue is full, Venus stops
  reading from Orbit until its event loop drains the queue, in place of
  dropping the attachment with `Orbit event queue exceeded its bounded
  capacity`; Orbit's slow-reader handling keeps the newest frame
  (`eon-10qt`).
- VEN-C5: the link highlight and hint are derived from the frame being
  presented, so output no longer blinks them while the pointer rests on a
  link, and a `Ctrl+click` press survives output revisions that keep the same
  URI over the same cells, in place of every revision retiring both
  (`eon-1u9e`).
- Venus consumes EONW v17, whose `Ended` reply answers a close that stopped
  the window's last terminal; Venus exits on it as on control loss
  (`eon-final-tab-close-ends-window-fwnp`).
- VEN-C8: Alt+Shift+Z toggles window-local zoom of the selected column across
  the tab's cards; hidden columns stay attached at their column size, and a
  change of the active tab, column count or selected column ends the zoom
  (`eon-zoom-selected-pane-e2pv`).
- VEN-C8: Ctrl+Alt+M sends `CreateColumn`, with the same 40-terminal-column
  refusal, in place of Alt+Shift+M (`eon-0ka6`).
- VEN-C26: Claude gets one read-only chip per reported window, five-hour then
  seven-day, each with its own label, hover and accessible node; under width
  pressure, after compaction, quota chips disappear starting with the one
  closest to the tabs, in place of the trailing one
  (`eon-claude-chip-split-u3ik`).
- VEN-C2: when Shift is the only held modifier and Space carries other text,
  such as `"` composed from a dead key under `us+intl`, Venus reports Shift as
  consumed, so a program using the kitty keyboard protocol without report-all
  receives the text rather than Shift+Space; Linux dead keys and compose
  sequences from the locale's compose table are accepted (`eon-e2vl`).
- VEN-C28: the CPU and RAM labels put one space between name and value
  (`CPU 12%`) and center it in a chip that keeps the width of `CPU 100%`, in
  place of padding the value to four cells (`eon-igae.14.4`).
- VEN-C29: an optional workspace-only `--bar-items` launch input sets the
  order of the Eon Bar's quota chips, CPU and RAM labels, clock and version
  label, and leaves out the items it does not name; Venus does not read or
  wake for a left-out reading. Without it the bar is unchanged
  (`eon-igae.14.1`).
- VEN-C5: Venus detects `http://` and `https://` URLs printed as plain text
  in visible rows, joined across soft wraps and trimmed of trailing
  punctuation and unmatched closing brackets, and gives the ones its opener
  accepts the same hover, preview, Open, Copy and accessibility actions as
  OSC 8 links; an OSC 8 link on any of their cells wins (`eon-nlcy`).
- VEN-C2: a named key the active keymap produces (Escape, Return, Tab, arrows,
  function and modifier keys) outside the numpad replaces the physical key for
  terminal input and Venus's own key handling; letter, digit, punctuation and
  numpad keys stay physical (`eon-venus-keysym-named-keys-chnh`).
- Venus consumes EONW v16, which adds a focus flag to both pane-create
  requests; Venus always sends a focused pane (`eon-zhjj`).
- Venus consumes EONW v15, which adds a command-pane request that Venus never
  sends (`eon-n6ga`).
- Venus consumes EONW v14, which adds a file-edit request that Venus never
  sends (`eon-ecne.2`).
- Venus consumes EONW v13, whose requests may name a calling terminal; Venus
  sends none, so its actions keep targeting the active tab (`eon-8mnr.2`).
- VEN-C28, VEN-C27, VEN-C25: a read-only `RAM 43%` label, from
  `/proc/meminfo` on the CPU label's 2-second wake, joins the Eon Bar after
  the CPU label and yields right after the clock. The CPU and RAM labels and
  the clock use the quota chips' unfilled rounded outlines, and the version label
  moves to the edge before Shortcuts and Close (`eon-igae.3`).
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
  corner radius) from the window's sides and between cards;
  popups draw as one card, and the footer's hints sit bare in the
  margin below the cards. Everything outside the cards takes the header's
  opaque color, and each card fills with its terminal's background
  (`eon-tubj`). Tab pills keep the card gap from the window's left edge
  (`eon-gv1r`) and share the Eon Bar widgets' centred height (VEN-C21).
  Column width and the 40-column admission check use
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
