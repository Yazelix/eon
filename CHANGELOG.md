# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Eon has one window family in the CLI. `eon attach`, `eon stop` and
  `--generation` are removed: use `eon window attach [ID]` and
  `eon window stop ID|previous|all`, which take a window ID as
  `eon window ls` shows it or any unique prefix of at least 4 characters, and
  `--window ID` now takes the same IDs and prefixes, `default` included, on
  every workspace command. A window that another build started is stopped or
  presented by running that build's `eon` in the form it takes:
  `stop GENERATION --json` or `attach GENERATION` for EONW 34 or older,
  `window stop ID --json` or `window attach ID` for later builds, with
  `EON_STOP_DELEGATED` or `EON_ATTACH_DELEGATED` naming the generation, and
  the advice for a missing recorded `eon` prints that command. EONW v35 marks
  the change (no message changes), and Eon is 0.1.19. Windows started before
  this release keep running and this `eon` stops and attaches them through
  their recorded `eon`, but their own `eon` cannot stop or attach a window of
  this release: restart old windows to use the new commands everywhere
  (`eon-7cf0y.22`, EON-C10, EON-C11, EON-C23).
- Tab, workspace and quota chips in the Eon Bar, and the bare key hints in the
  bottom bar (such as Left/Right), no longer cut the tails of g, p, y, j and q:
  their text clips at the chip or bar instead of at the terminal's line
  height, which is shorter than the ascent and descent of some header fonts,
  such as Open Sans at the default `line_height = 1.125`. The text keeps its
  place, except that at a large `font_size`, where the font is taller than
  the chip, a label moves to centre its letters in the chip, keeping its
  descenders inside first (`eon-4x5tb`).
- The bottom hints bar's clock drops its outline and shows as plain text in the
  version label's softened cyan, as far from the left edge as the version is
  from the right, so the two match at the bar's ends. A keycap's
  hover popup no longer repeats the keycap's name above its keys
  (`eon-e3a1z.3`, VEN-C21).
- The clock moves from the Eon Bar to the left end of the bottom hints bar,
  keeping its format and hover, which now opens above it. It hides
  with the bar (Alt+Shift+B), and in narrow windows after the version label,
  before any hint drops. The Eon Bar's quota chips and CPU, RAM and disk readings get
  its room; narrow windows now hide the Eon logo, then disk, RAM and CPU, then
  quota chips. `[bar] widgets` still lists `clock`, but its place in the list
  no longer moves it (`eon-e3a1z.2`, EON-C32, VEN-C21).
- The JSON replies of `eon pane create` and `eon tab create`, in every form,
  and of `eon edit` name what they started in a top-level `created` object,
  `{"tab": "tN", "pane": "pN"}`, so an agent no longer has to compare
  pane lists before and after; `pane` is null for a bare `tab create`, whose
  tab waits on quick search. A pick (`eon tab directory` on a pending tab)
  and `eon popup hide` of its quick search start that tab's first pane
  without naming it. Human output and every other reply are unchanged
  (`eon-1jeai`, EON-C10). EONW is unchanged.
- `eon window stop previous [--json]` stops every running window that another
  Eon build started, in the default namespace and in independent windows, and
  `eon window stop all [--json]` now stops every running window, the default
  one included; `eon quit --all` covers the same windows. Each window is
  stopped by its own validated Stop, the caller's window last (inside Eon, a
  caller whose window cannot be identified is refused), and a damaged
  generation directory or a window that cannot be inspected is reported,
  never stopped, and the other windows still stop. With `--json` each prints
  one `{"window": ID, "results": [...]}` object per window, `window` being its
  full ID, so `eon quit --all --json` names the default window `default`
  instead of null. Stop and Quit questions, results and `eon quit`'s listing
  name windows by the ID `eon window ls` shows instead of a generation, as in
  `Stop window 0a1b and 2 live terminals [session-1, session-2]? [y/N]`. The
  detach notice and the log line after Eon Desktop exits name a
  default-namespace window by its generation's first 8 hex digits, as in
  `eon window stop 1a2b3c4d`, which `default` now also answers to, so the
  command still reaches that window after an upgrade; an ID that several
  windows start, even one window's whole ID, is refused as ambiguous.
  Commands has "Stop old-version windows" (`eon window stop previous`) in
  place of "Stop older generations" (`eon-7cf0y.22`, EON-C23, EON-C11,
  EON-C8, EON-C10). EONW is unchanged.
- Working and background attention badges, on tabs and in the agent sidebar,
  draw their dots in the window's focus colour (`[terminal] focus_color`, or
  Venus's built-in focus colour) instead of a fixed blue. Where that colour
  would read below 3:1 contrast on what lies under the dots, they blend it
  toward black or white just enough to reach 3:1. With a translucent terminal
  background, the sidebar's working and background badges sit on the bar's
  dark disc, as on a tab, since the desktop behind the window sets the card's
  colour. A stale agent's badge stays dimmed below 3:1 on purpose. Needs
  input, error and finished badges keep their own colours (`eon-i3th2`).
- The bottom hints bar shows one keycap per set of modifiers, named in full
  (`Alt`, `Ctrl+Alt`, `Alt+Shift`, `Ctrl+Alt+Shift`), centred in the window,
  with keys without modifiers such as `Escape terminal` written out after them.
  Pointing at a keycap opens a popup above it listing its keys and what each
  does, as Alt+/ words them; it never takes focus, and Escape or a click
  closes it until the pointer leaves the keycap. The Ctrl+Alt+Shift+H/L tab
  moves are back in the bar, under `Ctrl+Alt+Shift`. `EON <version>` moves
  from the header to the hints bar's right end, hides with it (Alt+Shift+B),
  and its hover opens above it; the header's `?` sits beside `×`. Narrow
  windows shift the keycaps clear of the version, then hide the version, then
  drop the written-out keys, and finally shorten the keycaps to `A`, `CA`,
  `AS` and `CAS`. Screen readers read each keycap as, for example, `Alt+Shift
  shortcuts` with its chords (`eon-e3a1z.1`, VEN-C21, EON-C32).
- The bottom hints bar fits a 1280-pixel-wide window with Eon's packaged font
  (DejaVu Sans) at the default size:
  directions read `HJKL`, create hints drop their `+` (`pane`, `col`, `tab`,
  `window`), the sidebar hint reads `side`, the Scrollback popup's `Scroll`,
  and modifier badges drop their hyphens (`CA`, `AS`). The Alt+/, Alt+Q inbox,
  Alt+Shift+Q quit and Ctrl+Alt+Shift tab-move hints leave the bar; those keys
  still work and stay listed in Alt+/. Popup hints other than Git and Agent
  show as width allows. The header `?` Shortcuts button always shows
  (`eon-yvhke`, VEN-C32, EON-C21, VEN-C21).
- Eon Bar hover tooltips show a short list, one fact per line, instead of
  one long sentence, and leave out what the chip already shows: a quota
  tooltip names its window, then when it resets in local time and the pace,
  such as `Codex 7d window`, `Resets Sun 21:00`, `Runs out about 1d18h before
  reset`; CPU lists its logical CPU count and average, RAM its size, and disk
  its mount point and free space, each with a load line only under load; the
  clock lists the date and UTC offset. Stale data and blocked or unknown
  permission get their own line. Screen readers still read the full sentence
  (`eon-y22fl`).
- `eon window ls [--json]` lists every running Eon window, the default
  window included, with its ID, name, version, start time, terminals and
  whether this `eon` controls it directly. It replaces `eon windows` and
  `eon generations`, which are removed (`eon-7cf0y.22`, EON-C23, EON-C11).
  `eon window attach` and `eon window stop` take `default` for the window
  bare `eon` opens, an older build's window by the ID `eon window ls` shows,
  and any unique prefix of at least 4 characters. An ambiguous prefix exits
  3 and lists the matches. `eon window attach` without an ID reopens
  `default`, and `eon window stop ID --json` prints
  `[{"window": ID, "results": [...]}]`. The other-windows hint in a new
  window's quick search reads "See them: eon window ls · Quit one: eon
  window stop ID", and Commands drops "List generations" while "List
  windows" runs `eon window ls`. EONW is unchanged.
- An agent's finish or error in the selected terminal stays unread, in the
  sidebar, the attention inbox and Alt+G's queue, while another app has
  focus; it becomes read once Eon's window has focus with that terminal
  selected (`eon-2kvr`, EON-C31). Windows started before this change keep the
  old behavior until restarted.
- `eon tab close TAB` and `eon pane close PANE` close targets in any tab of
  the addressed workspace without changing the person's active tab or popup.
  CLI closes ask before stopping busy work; JSON callers pass `--yes` to
  confirm a busy close. In an eonbase window every tab or pane close counts
  as "busy state unknown", so `--json` needs `--yes` there and a terminal
  asks. IDs in another workspace remain refused. A native final-close prompt
  no longer selects its tab before closing it if another tab became active
  (`eon-agent-parity-a9g0.3`, EON-C8, EON-C10).
- `eon pane history TARGET --edit` opens TARGET's scrollback in the configured
  editor from the CLI, in a Scrollback popup over its tab, as Alt+Shift+E does
  for what the tab shows (`eon-u5bvd.5`, EON-C35, EON-C10). It works only in
  the shown workspace's active tab, exits 1 with `not-active` elsewhere and
  with `unavailable` while that tab shows a transient popup, which it never
  closes, and `[agents] terminal_access` governs it. `docs/CONFIGURATION.md` explains a
  GUI editor's wait flag, saving edits under another name before the popup
  ends, and the editor state files Eon leaves. EONW moves to v34: windows
  started before this change keep running, but an `eon` from this version
  cannot reach them; restart them to use it.
- Alt+Shift+E opens the scrollback of what the tab shows, its shown popup or
  else its selected pane, in the configured `[editor] command`: the retained
  history and then the screen, as plain text, from a private snapshot file
  that Eon removes when the popup ends (`eon-u5bvd.4`, EON-C10). A first line
  says when the 8 MiB bound left lines out, or when a full-screen app shows
  only its screen. `[popups.scrollback]` sets its `keybinding`, `label` and
  `enabled`; a popup of yours already on Alt+Shift+E keeps the key.
- `eon pane history TARGET [--json]` prints a pane's or popup's retained
  history followed by its screen, oldest first, as plain text with wrapped
  lines joined (`eon-u5bvd.3`, EON-C35). Past 8 MiB it leaves out the earliest
  lines, or the start of the newest line when that line alone is longer, and
  says so on standard error; `--json` reports them as `omitted_lines` and
  `cut_bytes`. `[agents] terminal_access` governs it as it
  governs `eon pane read`. EONW moves to v33: windows started before this
  change keep running, but an `eon` from this version cannot reach them;
  restart them to use it.
- Orbit can export a terminal's retained history and its screen as plain text,
  oldest first, up to 8 MiB, with the oldest lines dropped and counted past
  that (ORB-C17, `eon-u5bvd.2`). This is the first step of opening a pane's
  scrollback in the configured editor; `eon pane history` and the window key
  follow. Orbit's management protocol moves to generation 7; windows started
  before this change keep their own supervisor and Orbit runs, as with every
  management change, and EONW is unchanged.

## v0.1.16 (2026-11-05)

- Install from `github:Yazelix/eon/main`, which always holds the latest
  release, or from `github:Yazelix/eon/edge` for fixes before they are
  released (`eon-8lycf`). Both update in place with
  `nix profile upgrade --refresh eon`; open a new window afterwards, since
  windows already running keep their version until closed. Tags such as
  `v0.1.12` name one exact release and never update: move a tag install once
  with `nix profile remove eon`, then
  `nix profile add --refresh github:Yazelix/eon/main`.
- Fixed: Tab completion after `cd` in managed Nushell listed the startup directory's entries
  (issue #4, `eon-gkeae`).
- The Eon Bar's quota pace colours are less forgiving and end in a true red
  (`eon-w1h7c`). A window that would run out a twentieth of a window before
  its reset is yellow, a tenth orange, and a quarter or more full red, where
  full colour used to wait for half a window. The red is a saturated
  rgb(255, 59, 59) instead of a light coral, at 5.5:1 on the bar; RAM and disk
  chips at their red level use it too.
- A click outside the shortcut viewer (Alt+/) closes it, as Escape does
  (`eon-gl1xo`, VEN-C20). The click does nothing else: it does not focus,
  select, paste into or open a link in what lies under the viewer. A click
  inside the viewer keeps it open.
- `eon pane send` refuses with `person-typing` (exit 5, nothing sent) while
  a person is typing in the target. `[agents] send_typing_quiet_ms` defaults
  to 2000; 0 disables it and `--force` bypasses only this check. If the paste
  succeeded but Enter is refused, exit 1 with `enter: false`; do not repaste
  (`eon-buys1.2`, EON-C36). EONW v32 and Eon 0.1.16 require old windows to
  restart before this build's CLI can control them; live terminals keep running.
- Choose quota chips individually in `[bar] widgets` with `codex_quota_5h`,
  `codex_quota_7d`, `claude_quota_5h`, `claude_quota_7d` and
  `claude_quota_scoped`. All five are listed by default. Leave out
  `claude_quota_scoped` to hide chips such as Fable; workspace JSON keeps all
  readings. Unknown widget names warn and are ignored (`eon-8r9a4`).
- Alt+Shift+1 through Alt+Shift+9 show the first through ninth workspace, and
  Alt+Shift+0 the tenth, in `eon workspaces` order (`eon-hbpkq`, EON-C33, VEN-C8). A
  position with no workspace does nothing. `[workspace] number_keys = false`
  leaves those keys to the terminal, and a popup configured on one takes it.
- When Eon is slow to answer, the window says so instead of freezing
  silently (`eon-8wh7h`, VEN-C8). After two seconds without an answer the
  status line shows "Eon is slow to answer.", and the next answer clears it;
  nothing else in the window changes. When Eon cannot be reached the window
  shows "Cannot reach Eon.", and when it stops answering, "Eon is not
  answering.", instead of raw system text such as "Resource temporarily
  unavailable (os error 11)", which now goes to Eon's log.
- Alt+Shift+S twice switches to the previous workspace (`eon-vhhon`,
  EON-C33, EON-C10): in the Workspaces picker its own key works as Enter,
  showing the highlighted workspace, instead of closing the picker. Esc and
  Ctrl+C still close it.
- Closing Commands, Workspaces or Anima, by key or with `eon popup hide`,
  returns to the popup it covered (`eon-knowp`, EON-C33, EON-C10). Switching
  workspaces from the picker over the Agent popup and back, or Esc in the
  picker, showed the tab's pane instead of Agent.
- Closing a tab, pane or workspace stops its terminals together, under one
  shared five-second bound, instead of one at a time (`eon-8wh7h`, EON-C8).
  The window and `eon` commands waited while the stops ran; a tab of
  programs slow to exit could hold them past six seconds, and the window then
  showed "Cannot read Eon workspace response header". If a stop fails, Eon
  still stops the close's other terminals; only the terminals whose end it
  could not confirm stay in the tab.
- `eon edit` and Enter on a text file in Files open your configured editor
  (`eon-ecupy`): `[editor] command`, a direct argv such as `["nvim"]`, runs
  with the file's path appended. The default, `["hx"]`, is Eon's packaged
  Helix. Each edit reads the setting again; an editor missing from the
  terminal PATH fails the edit with `edit-command` instead of falling back to
  Helix.
- Picking a folder in quick search, or pressing Esc there, no longer shows a
  frame of Files' Yazi before the tab's terminal (`eon-61efs.1`). Eon
  Desktop now learns of workspace changes from Eon's event stream instead of
  waiting for its next 250 ms check, and Eon writes a change's events before
  answering the command that made it.
- Ctrl+X in the Workspaces picker (Alt+Shift+S, Commands > Switch workspace)
  closes the highlighted workspace after the same confirmation as
  `eon workspace close`, then lists the remaining workspaces again
  (`eon-p7omw`).
- The Project directory picker and the Files popup are one Yazi per tab
  (`eon-quick-search-parity-23y9.2`). A fresh window, a new tab (Alt+Shift+T,
  `eon tab create`, `eon workspace new`) and Alt+Z show the tab's Files on
  quick search; Alt+Shift+Y shows Files; each key hides it again when its own
  screen is already shown. Using a folder, with Enter in quick search or
  Alt+Enter in Files, starts a new tab's first terminal there, and closing
  the popup starts it at the tab's starting directory; either way the same
  Yazi stays as the tab's Files. `eon tab directory` on a new tab still
  waiting for its directory uses it the same way. The "other windows running"
  notice moves to quick search's header, and Alt+Z's default label is now
  "Quick search". The separate picker and its own Yazi keys are gone.
  Files is always Eon's packaged Yazi: `command`, `enabled = false` and
  `keep_alive = false` under `[popups.files]` are now ignored with a warning;
  move a custom Yazi to its own popup, such as `[popups.yazi] command =
  ["eon-yazi"]`.
- EONW moves to v31 and Eon to 0.1.15 (`eon-quick-search-parity-23y9.2`).
  v31 gives a new tab the tab's Files popup instead of a Project popup and
  drops the picker's own request. As with every EONW change, this `eon` lists
  a window started by an earlier Eon as incompatible and refuses its
  workspace commands with the usual other-version failure; older windows
  remain attachable and stoppable through the recorded `eon` that started
  them.
- Files has new keys (`eon-quick-search-parity-23y9.3`). In quick search (Tab),
  Enter uses the highlighted folder for the tab and Tab moves Yazi there; in
  Yazi, Alt+Enter uses a folder, Enter opens, and Esc closes Files after
  leaving any visual mode, selection, filter, find or search; Ctrl+C does the
  same instead of closing a Yazi tab. Z, q and Q no longer search or quit.
  Browsing and searching no longer write Zoxide history: a folder used for a
  tab, from Files or `eon tab directory`, adds exactly one visit as Zoxide
  records it (by default not for the home directory), unless
  `[shell] zoxide = false`. New `eon popup hide POPUP` hides a popup as its
  window's dismissal would: a keep-alive popup keeps running, a transient one
  stops.
- EONW moves to v30 and Eon to 0.1.14 (`eon-ciwn.6`). v30 carries the
  workspace event subscription that `eon events` will use (EON-C39); no
  command uses it yet, though supervisors serve it (`eon-ciwn.7`). As
  with every EONW change, this `eon` lists a window started by an earlier Eon
  as incompatible and refuses its workspace commands with the usual
  other-version failure; older windows remain attachable and stoppable
  through the recorded `eon` that started them.
- `eon --skill` prints the `eon` agent skill that this build ships, so an
  agent in an Eon terminal can learn the CLI with no setup, and `eon --help`
  lists it and ends with `Agent? Run eon --skill to learn how to work in Eon.`
  (`eon-ivge0`).
- Closing the Eon window, with the desktop's close-window key or the window's
  close button, quits it as Alt+Shift+Q does instead of detaching: an idle
  window quits at once, and while a terminal runs something the quit prompt
  asks first; closing again leaves the prompt open. Alt+Shift+D is the way to
  keep terminals running, and the Shortcuts viewer now lists it as "Detach:
  hide the window, keep terminals running". eonbase windows still detach on
  close, and so does a window whose supervisor does not answer or that failed
  to stop. A Venus crash, a lost connection or the compositor exiting still
  keeps every terminal, and so does logging out of a desktop that ends the
  session without closing windows first, such as GNOME or COSMIC. A desktop
  that closes each window at logout, such as KDE Plasma, quits an idle Eon
  window and shows the prompt on a busy one; there, press Alt+Shift+D before
  logging out to keep its terminals (`eon-7cf0y.16`).
- A newer `eon` stops and quits windows that an Eon build with another EONW
  started, through the `eon` that build recorded; `eon quit` lists only stop
  commands that work. Human-mode errors, `eon generations` and `eon windows`
  print quotes, backslashes and line breaks as written, so a command they name
  can be pasted (`eon-7cf0y.9`).
- Security: Helix that Eon starts no longer runs a repository's own Steel
  configuration. When a repository held both `.helix/helix.scm` and
  `.helix/init.scm`, opening Helix there from Eon (`hx`, `eon edit`, Files)
  ran them as you, so an untrusted clone could run any command. Eon's Helix
  now loads its Steel configuration only from `$(eon config-dir)/helix/`,
  where your own `helix.scm` and `init.scm` still load, and does not start
  when that root's path is not valid UTF-8 (`eon-2yj90`).
- `echo hi | eon pane send p1 --enter`, without `--stdin`, fails with
  `unread-stdin` (exit 2) and sends nothing, instead of pressing Enter alone
  and dropping the piped text. Text from a file or socket on standard input
  fails the same way. A terminal, `/dev/null`, an ended input, or a pipe or
  socket still empty after 100 ms is not read, so `--enter` alone still
  presses Enter. A loop that presses Enter in several panes, such as a
  Nushell `each` or a bash `while read`, can hand `eon` its own input; give
  that command empty input (`null | eon ...` in Nushell, `</dev/null` in
  bash), as the refusal says (`eon-zuidu`).
- `eon pane --help`, `eon pane send -h` and the same after every other command
  family print only that family's or command's lines of `eon --help`, instead
  of failing as an invalid command; `eon focus --help` looked for a target
  named `--help` before. An unknown family or command still fails with exit 2,
  and `eon anima --help` still shows Anima's own help. A name or text that is
  itself `--help` goes after `--`, as in `eon workspace rename -- --help`
  (`eon-xubt`).
- Alt+Shift+Q quits the window, asking first while a terminal runs something:
  the prompt names the busy terminals, Enter quits, Esc cancels and
  Alt+Shift+D detaches instead. The Shortcuts viewer lists it as "Quit: close
  the window, stop its terminals" and the hints bar shows `quit`
  (`eon-7cf0y.3`).
- The Eon Bar starts with the Eon logo, in its own colours and just shorter
  than the tab pills, before the tabs. The `?` button is a circle as tall as
  the pills, the `EON <version>` label sits between `?` and `×` whatever its
  place in `[bar] widgets`, and the logo and `×` sit as far from the window's
  sides as the bar's centre is from its top, with the same space between the
  logo and the first tab. The logo shows whatever `[bar] widgets` lists. On a
  narrow window it hides first, leaving room for a chip for each window each
  provider's latest read reported and for each reading, all at their widest,
  so changing values and window resets never show or hide it (a provider's
  quota first appearing or going away, or a read finding a window gained or
  lost, may, once), and it hides whenever the tabs would not all fit beside
  it, however wide the window.
  Nix derives the icon from `assets/eon.png` (EON-C1, EON-C3, EON-C32;
  `eon-369s`, `eon-nfrbn`).
- The bottom hints bar leaves more room on narrow windows: the `next agent`
  and `previous agent` hints are gone (Alt+G and Alt+Shift+G still work and
  stay in the Shortcuts viewer), the move hints read `mv tab` and `mv pane`,
  and the column hint reads `+col` (`eon-x0edg`).
- `eon config-path` prints the full path of the settings file, `config.toml`,
  whether or not the file exists yet, so `hx $(eon config-path)` opens it. The
  new `eon config-dir` prints the configuration root, which `config-path`
  printed before; scripts that read the root from `config-path` move to
  `config-dir`. Both create the root when it is missing, and `eon --help`
  names `config.toml` (`eon-wt5v8`, `eon-hb4zp`).
- A new window's first directory picker says when other Eon windows still run,
  naming them (`eon-7cf0y.7`).
- Holding a drag selection above a pane's top row, over the Eon Bar or a pane
  above, keeps extending the selection into history, as it already did with
  the pointer in the top row. On Wayland this also holds with the pointer
  outside the window; macOS is not yet checked (`eon-5uw0`).
- `eon quit` asks first only while a terminal of the window, other than the
  one running it, runs something other than its idle shell, and names those
  terminals in window language: `Quit this window? It stops 3 terminals; 1
  still running: p2  cargo build. [y/N]`, or `Quit window NAME?` outside Eon.
  While nothing runs it quits without asking. The Files and Git popups never
  count; the Agent popup does. Eon does not see a shell builtin or a background
  job of an idle shell. `eon workspace --json`, `eon generations --json` and
  the generation inventory in `eon windows --json` carry the busy terminals as
  `busy`, and human output prints one `busy` line per terminal.
  EONW moves to v29 and Eon to 0.1.13 (`eon-7cf0y.12`); older windows remain
  attachable and stoppable through the recorded `eon` that started them.
- Fixed: an Eon started from a Claude Code or Codex shell no longer gives its
  terminals that agent's command settings. A bare `git commit` there got
  Claude Code's `GIT_EDITOR=true` and an empty message, and Codex's
  `NO_COLOR`, `PAGER=cat` and the like turned off colour and paging. Eon
  removes each only with the agent's exact value and marker, so a different
  value you set yourself passes through; the same value set on purpose is
  removed too, so set it in your shell configuration or the command. Codex's
  forced `C.UTF-8` locale gives way to your desktop session's `LANG`,
  `LC_CTYPE` and `LC_ALL` from the systemd user manager, or, without one,
  keeps `LANG=C.UTF-8`. Eon also removes Claude Code's
  `CLAUDE_CODE_HOST_WORKTREE` pair, as it removes the other session names.
  EON-C9 lists them. Only supervisors started after the update remove them
  (`eon-yxo2y`).
- Fresh Nix installs no longer fail when an upstream Helix grammar repository
  disappears or goes offline. Pinned grammar downloads try hash-verified
  Yazelix-owned archives first, with upstream as a fallback (`eon-5y25o`).

## v0.1.12 (2026-11-02)

- Set `[terminal] cursor_trail_duration = 0` to turn the cursor trail off in
  new or reopened Eon and eonbase windows. Cursor colors and outlines stay the
  same; the default remains `1.5` (`eon-configurability-qkq8.6`).

## v0.1.11 (2026-11-02)

- Bundle Helix's pinned cylc grammar source so its deleted upstream repository
  cannot break fresh Nix installs (`eon-5y25o`).
- On Linux, every time an Eon window detaches, by closing it or with
  Alt+Shift+D, a desktop notification names the window, says how many
  terminals still run and offers Quit, which asks once more and then stops
  that window. Without notification buttons, it names the command that stops
  that window from a shell outside Eon. It replaces the once-per-user notice
  and never takes focus (`eon-7cf0y.2`).
- Fixed: when an agent's shell starts Eon, such as `eon` run by Claude Code,
  Eon removes a fixed list of agent session, nesting-marker and credential
  names from its terminals, popups, Venus and quota reader. Claude Code started
  in such a terminal read the inherited `CLAUDE_CODE_CHILD_SESSION` and saved
  no transcript, so `--continue` and `/resume` found nothing, and the launching
  session's messaging token reached every process. EON-C9 lists exactly which
  names Eon removes; agent settings such as `CLAUDE_CONFIG_DIR` and API keys
  pass through. Only supervisors started after the update remove them; closing
  and reopening a window, reattaching it, or recovering it keeps the old
  environment (`eon-geigu`).
- `eon quit` ends the window you are in without a window ID: run in any Eon
  terminal it stops that terminal's window, and outside Eon it stops the only
  running window, or lists them when several run. `eon quit --all` stops every
  window, the default one included. Commands > Quit this window runs `eon quit`
  (`eon-7cf0y.1`).
- Each window carries a running-windows summary, available as `running` in
  `eon workspace --json` and as `running` lines in human output. It names this
  window and the other running windows, their generations, supervisor start
  times and live terminal counts. `eon generations --json` and the generation
  inventory in `eon windows --json` include `name` and `started_at`.
  EONW moves to v28 and Eon to 0.1.11 (`eon-7cf0y.4`); older windows remain
  attachable and stoppable through the recorded `eon` that started them.

- Fixed: Eon windows and generations from this release on share one Claude
  quota read every five minutes when the shared schedule is usable; otherwise
  each keeps its own schedule. After Claude's usage endpoint refuses a read (as
  when Anthropic rate-limits it) Eon waits 30 minutes before trying again,
  then one hour after a second refusal and two hours after each later one,
  until a read succeeds; `claude_quota_failure` names such a refusal
  `provider_error` instead of `exit_status` (`eon-agsni`).
- Claude's model-scoped weekly window, such as CodexBar's `Fable only`, shows
  as its own Claude chip after the unscoped ones, labelled with its scope
  (`Fable 3d/7d 60%`); `[bar] widgets`' `claude_quota` covers it. `eon
  workspace --json` carries each quota window's `scope` (null when unscoped)
  and, as `codex_quota_failure` and `claude_quota_failure`, when and why the
  provider's last quota read failed, until a read succeeds. EONW moves to v27
  and Eon to 0.1.10, so after a profile refresh, windows started on v26 need a
  restart before the CLI controls them. An older Eon ignores a stored Claude
  reading that holds a scoped window until its own read succeeds.
- Alt+H, Alt+J, Alt+K and Alt+L are reserved for moving focus. A popup
  configured on one of them still loads but gets no key, and Eon logs a
  configuration warning naming the popup and the key, so an older
  `config.toml` keeps starting Eon (`eon-0gllz`).
- The bottom hints bar is shorter: one `focus` hint joins tab and pane focus
  and tab-by-position keys (`H/L · K/J · 1..9` under Alt), and create hints
  read `+pane`, `+column`, `+tab` and `+window`. Shortcuts and screen readers
  keep the full action names.
- Fixed: the Claude quota chip stopped updating and showed its last reading as
  `old` once Claude reported only a weekly limit, which CodexBar returns twice.
  Eon now reads an exact repeat as one weekly window and still rejects two
  differing windows of the same length (`eon-8jdmw`).

- The Eon Bar shows `DISK 80%`, the used share of the filesystem holding your
  home directory, after RAM by default, refreshed every 30 seconds. Hover and
  accessibility give its size, mount point and free space. It turns orange from
  85% use and red from 95%, saying `filling up` or `nearly full` in words.
  `[bar] widgets` accepts `disk`, and `[bar] disk_path`, `disk_warning_percent`
  and `disk_critical_percent` set the measured path and the levels (EON-C32
  amended).
- Deprecated: `[bar] show_version`, replaced by `[bar] widgets`. A leftover
  v0.1.1 `show_version` opens its window again instead of stopping the load:
  Eon reads it as `[bar] widgets` and logs one warning naming the replacement,
  without rewriting `config.toml`. `show_version = false` shows the built-in
  widgets without the version and `true` shows them all; when `widgets` is set
  too, it wins and `show_version` is ignored with a warning. Eon removes the
  alias only once the migration floor passes v0.1.2.

- Migration floor: v0.1.1, the oldest release whose configuration Eon keeps
  loading.

- The Eon Bar's CPU and RAM chips colour when the machine is under load: RAM
  turns orange from 80% use and red from 90%, and CPU turns orange while the
  2-second use it shows is 90% or more. Hover and accessibility say why in
  words, and for CPU also give the average over the last 30 seconds, such as
  `45% on average over the last 30 seconds`. `[bar] ram_warning_percent`,
  `ram_critical_percent`, `cpu_warning_percent` and `cpu_average_seconds` set
  the levels and the averaging window (EON-C32 amended).

- EON-C35 (`eon pane read`) and EON-C36 (`eon pane send`) become Proven on
  x86_64 Linux Wayland, and EON-C11's amendment (only the window's own Venus
  attaches to its terminals) is proven: on an isolated installed Eon, an agent
  script in one pane read and sent to panes in hidden tabs and workspaces and
  to a hidden popup without changing what the window showed or the desktop
  focus, submitted prompts to Claude Code and Codex, and another process could
  not attach to a hidden terminal. Behavior is unchanged
  (`eon-agent-parity-a9g0.19`). Both contracts and `docs/USAGE.md` no longer
  say that a terminal on an older Orbit answers `terminal-unavailable`: a
  supervisor that speaks another EONW version is answered with
  `supervisor-unavailable`, exit 4, as before.

- `eon pane send TARGET [--enter] [--stdin] [--json] [-- TEXT]` pastes up to
  64 KiB of text into a pane or popup in any workspace of the window, bracketed
  when the program asked for bracketed paste, and with `--enter` presses Enter
  after `[agents] send_enter_delay_ms`. It refuses control characters, and a
  line break when the program has not enabled bracketed paste, sending
  nothing. Selection, the shown workspace and popup, and focus stay as they
  are. `[agents] terminal_access = "none"` refuses it.

- `eon pane read TARGET [--json]` prints the visible text of a pane or popup in
  any workspace of the window, shown or hidden, and changes nothing the window
  shows. `--json` adds the target, the calling terminal, the screen, its size,
  title, directory and revision. `[agents] terminal_access = "none"` refuses
  it.

- New `[agents]` settings: `terminal_access = "window"` (or `"none"`) decides
  whether `eon pane read` and `eon pane send` may reach the window's terminals,
  and `send_enter_delay_ms = 300` is the wait before `eon pane send --enter`
  sends Enter. EONW moves to v26 and Eon to 0.1.9, so after a profile refresh
  a window started from an older build needs one restart before the new `eon`
  can control it.

- Ctrl+Tab and Ctrl+Shift+Tab select the next and previous tab, wrapping
  around. They no longer reach the terminal; a configured popup on either key
  takes it.

- Only the Venus that Eon starts for a window can attach to that window's
  terminals. Another program that connects to a terminal's socket is refused as
  busy and cannot see its screen or type into it.

- Codex quota shows one chip per window, like Claude.

- Two-cell emoji, including flags, joined families and VS16 hearts, prefer the
  platform colour emoji font even with configured terminal fonts or bold text.
  Eon includes Noto Color Emoji on Linux; macOS uses Apple Color Emoji.

- Alt+Q opens an attention inbox across every workspace, with All, Needs input,
  Error and Finished filters and keyboard, pointer and accessibility navigation.
  Selecting a row jumps to its agent and shows its workspace. `FocusAttention`
  accepts a tab of any workspace of the window; stale targets leave focus and
  the shown workspace unchanged.

- Emoji flags, skin tones and ZWJ sequences such as 👍🏽, 🇯🇵 and 👨‍👩‍👧 show
  as one emoji two cells wide, also after `reset`. Terminals already running
  when Eon is updated keep the old widths until those terminals are restarted.

- `eon workspace --json` and workspace action replies include the window's
  `attention_queue` in Alt+G order, with workspace, tab and target IDs and each
  terminal's full attention record. EONW moves to v25 and Eon to 0.1.8, so
  after a profile refresh a window started from an older build needs one
  restart before the new `eon` can control it.

- `eon workspace` names its calling terminal, tab and pane or popup in JSON
  `caller` and a human caller line. Eon packages an agent skill with documented
  manual install and optional session-start hooks for Claude Code and Codex.

- `eon workspace --json` and workspace action replies include each terminal's
  full agent attention record and the Codex and Claude quota readings. All
  JSON times are Unix milliseconds. Human output is unchanged.

- `eon tab create [--json] [--no-focus] --directory DIRECTORY [-- COMMAND [ARGS...]]`
  starts a tab at DIRECTORY without the picker, with the default pane or an
  exact command requested from an Eon terminal. `--no-focus` preserves the
  active tab, selected panes, shown popup and desktop focus. EONW moves to v24
  and Eon to 0.1.7, so after a profile refresh a window started from an older
  build needs one restart before the new `eon` can control it.

- CLI confirmations without a terminal now exit 1 with `not-confirmed` when
  no yes is read, including end of input. A piped yes still confirms; declining
  at a terminal still prints the cancel note and exits 0.

- Agent attention: an agent that reports done, or idle, while processes it
  started in its terminal still run shows as waiting on background work
  instead of finished or idle, until that work ends. Its tab badge and
  sidebar row show three still dots in the working blue, and screen readers
  name it "waiting on background work". `eon workspace [--json]` and
  `eon workspaces [--json]` report the state as `background`. EONW moves to
  v23 and Eon to 0.1.6, so after a profile refresh the new `eon` cannot
  control a window started from an older build until that window is
  restarted.

- The Eon Bar's quota windows are colored by how sustainable their pace is:
  green while usage is comfortably under pace, a light green right on pace,
  then yellow, orange and red the earlier the window would run out before it
  resets at the current rate. Hovering a chip says the same in words, such as
  "At this pace it runs out about 1d18h before reset". Stale readings, windows
  without a reset time and windows in their first tenth, unless already used
  up, keep the plain color.

- Agent attention: an agent that is working again shows as working in the
  sidebar, even when an older finish on the same terminal was never looked at.
  Before, that finish's green check covered the spinner until you focused the
  terminal. Work the program reports after a finish now counts that finish as
  seen, so its check does not come back when the work stops. A finish reported
  after the work still shows. Errors and needs input are unchanged.

- The working attention badge, on tabs and in the agent sidebar, is now
  braille dots stepping through ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ten times a second, one cycle a
  second, instead of an open arc turning five times a second. Set
  `[workspace] animate_working_badge = false` to keep it still at ⠋; without
  GPU rendering it is always still. The window reads the setting when it
  starts.

- Terminal images: programs that show pictures through the Kitty graphics
  protocol, such as Yazi's image previews, now draw them in Eon's panes and
  popups, between the cell backgrounds and the text. Until an image arrives,
  or when Eon cannot show it, its area shows a faint outlined marker in the
  terminal's text colour instead of staying blank. Images in one window use at
  most 256 MiB of GPU memory; past that, Eon frees images no terminal shows
  and marks the rest. Images that a program sends from a file or shared
  memory and PNG data are not shown, and animations do not play. Yazi's
  previews are proven on the installed product, x86_64 Linux Wayland, in a
  pane and in the Files popup (ORB-C16, VEN-C31).

- Several workspaces in one window (EON-C33) are proven on the installed
  product, x86_64 Linux Wayland. One window held three workspaces.
  Alt+Shift+S switched between them and created one by name; Commands and
  the CLI, including from a terminal inside Eon, created, switched, renamed
  and closed them. Switching never took desktop focus from another window.
  Closing a middle workspace's last tab kept the window. Detach and attach
  came back on the last shown workspace. An agent needing input in a hidden
  workspace badged the workspace label, and Alt+G showed that workspace.

- Agent attention now spans every workspace of a window. Alt+G and Alt+Shift+G
  walk agents in hidden workspaces too, and jumping to one shows its
  workspace; a finish or error in a hidden workspace stays unseen until you
  show that workspace with that terminal focused. The Workspaces picker
  (Alt+Shift+S) lists workspaces most recently shown first, so Alt+Shift+S
  then Enter returns to the previous one, and marks one that needs input
  ` · needs input`. The Eon Bar's workspace label shows a needs-input badge
  while another workspace needs input. `eon workspaces [--json]` adds each
  workspace's `attention` and `recency`. EONW moves to v22 and Eon to 0.1.5,
  so after a profile refresh the new `eon` cannot control a window started
  from an older build until that window is restarted.

- A small touchpad scroll of less than one row at live output no longer leaves
  an old history row drawn at the terminal's top edge while a program keeps
  printing; the edge follows the output, refreshed about every 50 ms.

- A terminal scrolled into history with less than one row of scroll left over
  no longer keeps drawing a stale row at its edge when a long burst of output
  drops the oldest history and moves the view; the edge is refreshed about
  every 50 ms.

- A popup you configure on a built-in popup's default key now takes that key
  instead of failing to load; the built-in popup stays in Alt+/ under its
  default key as "overridden by" yours. `keybinding = ""` leaves any popup,
  Project included, without a key. `eon workspace --json` entries add
  `overridden_by` and `default_keybinding`, and an entry's `keybinding` can be
  `null`; the text form shows `key=none`. EONW moves to v21 and Eon to 0.1.4,
  so after a profile refresh the new `eon` cannot control a window started
  from an older build until that window is restarted.

- When a terminal's Orbit worker dies, its Orbit log names how the worker
  ended, such as `signal: 9 (SIGKILL)`, with the terminal's session and run,
  instead of nothing.

- A program in an Eon terminal reads the pane's text area, without its
  padding, from the terminal's pixel size, and gets answers to the size
  queries CSI 14t, 16t, 18t and 19t and a size report when it turns on mode
  2048, so programs that size images from cell pixels, such as Yazi, match
  the terminal's cells.

- With a screen reader running, Venus no longer exits when the active tab has
  no panes and shows nothing, such as a tab left with only a dismissed popup.

- Extended terminal backgrounds reach the pane's title line, filling the band
  below it under the existing title text, cutout and outline. Primary-screen
  exceptions still apply; background mode, popups, zoom and frameless mode keep
  their header backgrounds. The pane ID there stays as bright as the name
  whenever a dimmer ID would be hard to read on that band.

- One window can hold several workspaces, managed from the CLI:
  `eon workspaces [--json]` lists them, and `eon workspace new [--name NAME]
  [--no-focus]`, `eon workspace switch WS`, `eon workspace rename [WS] NAME`
  and `eon workspace close WS` add, show, name and close one, where WS is a
  workspace ID such as `w2` or its exact name. Closing a workspace's last tab
  while others remain closes only that workspace.

- While a window holds several workspaces, or its one workspace has a name,
  the Eon Bar starts with a label naming the shown workspace and its position,
  as `notes 2/3`; clicking it opens the Workspaces picker. Closing the last tab
  while other workspaces remain asks first too, in the window and from
  `eon tab close` or `eon pane close`, with "This closes workspace NAME and
  stops N terminals" instead of "This quits the window".

- Alt+Shift+S opens the Workspaces picker, which lists this window's
  workspaces in window order with their tab counts and the shown one last.
  Enter shows the highlighted workspace, Enter on a name that matches none
  creates it, and Esc changes nothing. `[popups.workspaces]` sets its key,
  label and enablement. Commands gains Switch workspace, New workspace, Rename
  workspace and Close this workspace.

- The CPU chip's tooltip names how many logical CPUs its percentage covers,
  as `CPU use 12% across 16 logical CPUs, over the last 2 seconds`.

- Venus chrome layers separate more clearly: pane, popup, sidebar and chip
  outlines, hover and highlight fills, tooltips, the Shortcuts viewer and
  scrollback pills are brighter, and dimmed sidebar details are easier to
  read. Finished agent badges are leaf green instead of the mint focus color,
  and hint modifiers (A, A-S) are steel blue instead of the error red.

- An unknown configuration key is reported once per supervisor start in its
  log, and no longer printed into each new terminal; `eon-nu` and `eon-bash`
  started other than as a terminal's default shell still print it.

- Tab pills stand out more from the Eon Bar: inactive, hovered and selected
  tabs have brighter fills that step up in that order, and inactive tab
  labels are brighter.

- Popup margins are fixed at 8 logical pixels at the sides and 4 above and
  below. A `[popup]` table in `config.toml` is ignored with the usual
  unknown-setting warning, and `eon config defaults` no longer lists it.

- A popup's outline ends on the same bottom edge as the pane cards and the
  agent sidebar, including while a resize is in flight and with pane frames off.

- Workspace CLI commands follow the calling terminal's own supervisor and
  workspace across compatible builds. `--generation` and `--window` select
  explicit targets, and workspace results identify their generation, window
  and projected workspace.

- Pane labels prefer a pinned name, then the sanitized terminal title, then
  the working directory, with differing-directory context. `eon pane rename`
  and Commands set or clear pins; workspace JSON includes `label` and
  `pinned_name`. Runtime supplies labels over unreleased EONW v20. Pane IDs
  dim when contrast permits; label updates preserve zoom and view state.

- EONW moves to v20 for workspace summaries and workspace requests. Windows
  started with older protocol versions need a restart before a newer CLI can
  control them; workspace UI and CLI subcommands follow in later slices.
  Eon's version moves to 0.1.3 with it, as every EONW change bumps it.

- `eon config defaults` prints the build's configuration defaults as commented
  TOML, or as a nested object with `--json`, even when user configuration is broken.

## v0.1.2

The first release since v0.1.1. EONW moves from version 9 to 19, so a
window started from 0.1.1 needs one restart before the 0.1.2 `eon` command
can control it; `eon --help`, `eon versions` and the window label report
0.1.2.

- Terminal padding extends the nearest edge cell backgrounds by default, so full-screen TUIs fill their panes. `[terminal] padding_color` selects `extend`, `extend-always`, or the previous `background` behavior.

- Zoomed tabs show a zoom-in magnifier after their name; the accessible name
  keeps `, zoomed`, and a missing symbol font falls back to ` · zoom`.

- Card titles have more space below the Eon Bar: the gap to their capital
  letters matches the space above the pills. Pills and controls stay in place;
  the added space yields first on short windows.

- Alt+Shift+Z shows the selected pane alone across the tab, including in a one-column stack. Changing the selected pane, tab or layout ends zoom; a one-pane tab refuses it.

- Unknown `config.toml` keys are ignored with a warning naming the key and file,
  so a newer setting keeps older running builds working. Known settings still
  reject wrong types and values.

- The expanded pane's bottom edge keeps the stack's row spacing to collapsed rows below, within the padding and grid bounds; resizing keeps visible rows clear of strokes.

- A collapsed last pane's label sits in the stack's rounded bottom edge, closing
  it with one line. Columns and the Agents card share that bottom line; a last
  expanded frame ends there inside terminal padding. Fractional-scale resize
  rounding keeps strokes clear of every visible grid. A short, scrolled viewport
  clips a bottom stroke that cannot fit below the grid. Terminal layout and grids
  stay unchanged, as do frameless and popup paint.

- Tab pills and quota, CPU, RAM and clock widgets share one centred height,
  with equal space above and below to the card titles' capital letters. Tabs keep
  their fills; widgets use neutral rounded outlines sized by `[terminal] border_width`.
  The version label and header controls share their centre line. On short windows,
  the top gap yields before the pills shrink.

- The selected pane's outline defaults to 1 logical pixel instead of 2.

- Each pane has its own rounded frame, with collapsed panes showing rounded
  title edges whose corners turn toward the expanded pane and whose sides join
  the next card. The expanded pane stays fully outlined. Popup and Agents titles sit
  in matching top borders. Each column shares one terminal background and opacity
  from its first rounded top edge down to its rounded bottom, including stacked
  title rows and the space between cards. Popup and sidebar fills follow their
  rounded outer edges too; the space above and outside stays opaque frame.
  Terminal grids and pane hit targets stay unchanged.

- `[terminal] border_width` and `focus_border_width` size neutral card outlines
  and title rules, and the selected pane's outline, respectively. Both accept
  1–3 logical pixels, defaulting to 1 and 1. Widths round to whole physical
  pixels with display scale (defaults at scale 1.3: 1 and 1; at scale 2: 2 and 2).

- The selected pane's whole outline uses `[terminal] focus_color`, defaulting to
  violet `custom:#b207fb`. Preset and
  custom colours are supported; F6 keyboard-focus pills stay mint. Frameless
  mode keeps labels without outlines.

- Pane titles sit in thin rules instead of filled header bars, sharing the agent
  sidebar title size. Compact rows leave more space for terminal content; hover
  and keyboard focus highlight the label. Turning pane frames off keeps labels
  and their interaction feedback. Column, popup and agent-sidebar cards share
  the same top edge, except that on a very short window a popup's top edge
  rises to keep one terminal row.

- Orbit host stderr now goes to a private per-terminal log, and Eon's terminal
  failure diagnostics name that file.

- Closing the window while a Project picker can't be stopped no longer ends
  Eon in the background. That picker stays in its tab, Eon logs why once and
  tries again at the next detach, the other pickers are still cleaned, and
  the other tabs keep running.

- The agent sidebar shows where you are: the row of the agent whose terminal
  has keyboard focus, in a pane or the shown popup, takes a thin slate outline.
  Tab headings no longer carry the popup mark; popup rows keep it.

- Closing a tab whose terminal's program is slow to die after it is killed,
  for example on a heavily loaded machine, now waits for the program instead
  of failing like a crash. A program gone within the close's 5-second limit
  closes with the tab as usual; a slower one still makes the close report a
  failure, and its terminal records the stop once the program is gone.

- A terminal whose end can't be accounted for, after it exits on its own or a
  close of it fails, no longer ends the whole window. It stays in its tab, Eon
  logs why once and keeps checking, and the window and other tabs keep running.

- The "Link handed to the desktop handler." and "Link copied." notices go
  away on their own after about 2 seconds, so returning from the browser no
  longer finds them waiting. A notice in the status line no longer stops you
  from hovering or Ctrl+clicking a link.

- Closing a tab no longer ends the whole window when one of its terminals
  fails to stop. A terminal whose Orbit failed and exited without a result is
  closed as an ended terminal. One that still can't be accounted for stays in
  its tab, the close reports the failure, and the window and other tabs keep
  running.

- The working badge on tabs and in the agent sidebar is now a smooth open
  arc that turns, in place of the small segmented ring that was hard to read.
  On a software graphics adapter it shows the same arc, still.

- `eon window attach ID` reopens a detached window after you install a newer
  Eon profile. It presents the window's running generation instead of only the
  new build's, and when the new `eon` cannot present it, it runs the `eon` of
  the build that started the window. `eon attach GENERATION` does the same.
  A window started before this change by a build this `eon` cannot present
  has no such record: the command exits 1 and says that the build that started
  the window can attach or stop it. `eon windows` lists a window whose
  supervisor has ended as ended instead of as a current generation that has
  not started, and workspace commands no longer leave an empty generation
  folder behind for a build that isn't running.

- Alt+G and Alt+Shift+G no longer do nothing when no agent needs attention:
  the window shows "No agent needs attention" and focus stays where it was.
  The notice goes away when you type or click in the terminal, select another
  terminal, or an agent's attention state changes.

- The agent sidebar marks popups: the row of an agent running in a popup
  comes first in its tab and carries a small window mark after the popup's
  name, shown or hidden, and a tab that shows a popup ends its heading with
  the same mark. From another tab you can see which tabs will open on a
  popup.

- A program that prints long links, or text heavy with combining marks such
  as zalgo text, across much of the screen no longer ends its terminal while a
  window shows it. When one screen is too large to send whole, every cell
  still shows its first character; full text comes first and links last, and
  past that limit cells show only their first character, without a link.

- `eon` commands run inside a window, including `pane create`, `edit` and
  agent scripts, keep working after you install a newer Eon profile. Each
  terminal's PATH now puts its own window's build first as `eon`. An `eon`
  from another build, run by path there, that finds no supervisor of its own
  no longer says to run `eon` first: it says to use the terminal's own `eon`,
  and `pane create`, `edit`, `pane close`, `pane move` and `tab move` exit 3.
  `eon`, `eon window new` and `eon versions` typed in a window use that
  window's build too; open the newer build from the launcher or a shell
  outside Eon. Windows started before this change still need a restart.

- Agent attention (EON-C31) is proven on the installed product, x86_64 Linux
  Wayland. In it, Eon follows real Claude Code and Codex sessions through
  working, needs input and finished, and the agent sidebar lists them. Stale
  status dims, Alt+G walks agents across tabs, and an Agent popup running an
  older Claude Code (2.1.293) reads unavailable. Status changes never take
  focus or stop a terminal. The agent sidebar (VEN-C30) is proven there too,
  except compositor blur behind it.

- A program that prints a link whose address is not valid UTF-8 no longer
  ends its terminal while a window shows it. The text shows without the link,
  and the terminal and its program keep running.

- A program that sets an invalid title or working directory no longer ends
  its terminal while a window shows it or Eon watches its title. A title that
  Orbit's terminal cuts at 1024 bytes inside a character, or an OSC 7
  directory that is not UTF-8, now reads as empty, and the terminal and its
  program keep running.

- When Venus renders in software, as in a VM without GPU acceleration, the
  working badge on tabs and in the agent sidebar is a still ring instead of a
  spinner, so an agent at work no longer costs Venus a quarter to a third of a
  CPU core.

- Claude Code rows in the agent sidebar lead with the session's name, so
  agents in one project are easy to tell apart. A session started with
  `claude --name eon-9a`, or renamed with `/rename`, shows `eon-9a`;
  otherwise it shows Claude's session title, or `Claude Code` before it has
  one. Other agents' rows show the title the agent reports, else its app,
  such as `Codex`; rows no longer repeat the tab's folder, which the tab's
  heading already shows. `eon workspace` adds `name=` and `--json` adds
  `"name"` to each terminal's attention: the title its terminal sets. This
  changes the workspace protocol to EONW 19. The updated `eon` cannot manage
  a window started before the update: it reports that the supervisor uses
  EONW 18 and that the Eon build that started it can attach or stop it.
  Restart such a window to use the updated `eon` there.

- Alt+G jumps to the next coding agent that needs attention, and Alt+Shift+G
  to the previous one, whether the agent sidebar is shown, hidden or turned
  off. Agents that need input come first, then errors, then finished agents,
  each oldest first. The shortcut viewer and hints list both keys, and a
  popup configured on either key takes it.

- While the agent sidebar is hidden, Eon Bar tabs show what their agents are
  doing. A corner badge marks a tab whose agent needs input (◆), hit an error
  (✕), finished (✓) or is working (a spinner), including the active tab. A
  badge is dimmed while its status is stale. When tabs overflow, each scroll
  edge shows the most urgent badge hidden past it. Screen readers hear the
  state after the tab's name, such as "needs input". Badges never change a
  tab's width or name, and clicking a badged tab selects it as before.

- `eon pane close PANE`, run from a popup such as Agent or from a pane of the
  same tab, now closes that pane while the popup stays shown. Alt+W under a
  popup, and closing the tab's last pane from there, are still refused.

- Full Eon windows show the agent sidebar on the left, below the Eon Bar, 28
  cells wide; the terminal columns move right to make room. Alt+Shift+H shows
  or hides it without taking focus from the terminal, and the shortcut viewer
  and hints list the key. The sidebar steps aside while the window leaves
  fewer than 80 cells for the terminals, and returns when it is wide enough.
  It lists, in tab order, each tab whose agents report their status, with one
  row per agent: its state, the Claude or Codex icon, its name, title or
  app, how long ago it changed, and the pane or popup it runs in, plus the
  question of an agent that needs input. Click a row, or reach it with F6 and
  the arrow keys and press Enter, to jump to that agent. With no reporting
  agent it explains what will show there. Its card has the pane cards' outline,
  background and background opacity, so desktop blur shows through it.
  `[sidebar] enabled = false` removes it and frees Alt+Shift+H.

- The first time an Eon window detaches, by closing it or with Alt+Shift+D,
  a desktop notification says Eon is still running in the background and how
  to return or stop it. It appears once per user and never takes focus.

- A terminal whose Orbit receives a termination signal now ends on its own; it
  no longer brings down the supervisor and window.

- `eon workspace` shows each terminal's agent attention, such as
  `attention=needs-input app=codex`, each tab's most urgent one, and whether
  the agent sidebar is open (`sidebar open`). With `--json` that is
  `"attention":{"state":"needs-input","app":"codex"}` on panes and popups,
  `"attention_rollup"` on tabs and `"sidebar"` on the workspace, or `null`
  without attention. The states are `needs-input`, `error`, `finished`,
  `working`, `idle` and `unavailable`. An agent's own status text never
  appears there. `[sidebar] enabled = false` turns the coming agent sidebar
  off. This changed the workspace protocol to EONW 18, since followed by
  EONW 19: the updated `eon` cannot manage windows started by an earlier
  build, so stop the ones you no longer need with the old `eon` first.

- Closing the last tab from the window now asks first: Alt+Shift+W, the bar's
  `×` and Alt+W on the last pane show "Close the last tab?" with the
  terminals that will stop. Enter quits, Escape cancels. The new Alt+Shift+D
  closes the window and keeps everything running, as closing the window does;
  `eon attach` brings it back. It works from any state, the prompt included,
  and appears in Shortcuts and the footer hints. `[workspace]
  warning_close_last_tab = false` skips the prompt.

- `eon workspace --json` prints tab directories and terminal endpoints as
  strings, such as `"directory":"/home/you/project"`, instead of arrays of
  byte values. A path that is not valid UTF-8 still prints as a byte array.
  Human `eon windows` prints each window's generations as `eon generations`
  does, instead of their JSON. A terminal whose command fails as it starts
  reports `terminal command exited with status N during startup`, and the
  window's notice wraps onto up to three lines instead of being cut off at the
  window's edge.

- A pane no longer shows `Orbit connection lost: Orbit event queue exceeded
  its bounded capacity` when a program prints a large burst while you scroll.
  Venus waits for its screen to catch up and then shows the latest output.

- A link under a still pointer stays highlighted, with its Open and Copy
  hint, while the terminal prints output, for example while an agent streams.
  `Ctrl+click` on it also opens it during such output, as long as the link
  stays in place.

- Closing the last tab now quits its window, like closing any other tab
  stops that tab's terminals: Alt+Shift+W, the bar's `×`, Alt+W on the last
  pane, `eon tab close` and `eon pane close` all work on it, where they used
  to refuse. Closing the window itself still keeps everything running. Run
  from a terminal, `eon tab close` and `eon pane close` ask first, listing the
  terminals that will stop, unless `[workspace] warning_close_last_tab = false`
  or `--json`. This changes the workspace protocol to EONW 17: the updated
  `eon` cannot manage windows started by an earlier build, so stop the ones
  you no longer need with the old `eon` first.

- `eon pane create --no-focus`, with or without `-- COMMAND`, adds the pane
  without taking focus: the tab's selected pane stays selected and a popup
  you run it from, such as Agent, stays open. An agent can start other agents
  in panes while you keep working with it. Without `--no-focus`, the new pane
  is selected as before. This changes the workspace protocol to EONW 16:
  the updated `eon` cannot manage windows started by an earlier build, so
  stop the ones you no longer need with the old `eon` first.

- Commands (Alt+Shift+M) has a Quit this window entry. It runs `eon stop` for
  the window you are in, lists that window's live terminals and asks first;
  answering yes ends them all and closes the window, and anything else changes
  nothing. In an independent window it stops only that window.

- A new Eon window that starts slowly on a busy machine now gets 30 seconds
  to become ready instead of 5, so Eon no longer closes it with
  `cannot admit Eon Desktop presentation: EAGAIN`. If it still is not ready,
  the error says so: `Venus was not ready within 30s`. While it starts,
  `eon` commands wait for it rather than timing out.

- `eon pane create -- COMMAND [ARGS...]`, run from an Eon terminal, opens a
  new pane in its column running COMMAND, for example
  `eon pane create -- claude "prime, then continue eon-123"`. The command runs
  exactly as given, with no shell. This changes the workspace protocol to
  EONW 15: the updated `eon` cannot manage windows started by an earlier
  build, so stop the ones you no longer need with the old `eon` first.

- A newly started Eon window shows the last Codex and Claude quota readings
  until its own read succeeds, instead of no quota chips. Eon keeps each
  provider's last good reading in `$XDG_STATE_HOME/eon/quota`, shared by all
  your Eon windows; it turns `old` and disappears on the usual schedule.

- Alt+Shift+M opens a Commands popup: search a list of Eon commands, such
  as new tab, move tab, list generations or stop older generations, and press
  Enter to run one for that tab. Each line shows the `eon` command it runs.
  A custom popup already on Alt+Shift+M now collides with it: move one of
  them with `keybinding`, or set `[popups.commands].enabled = false`.

- Alt+Shift+Z zooms the selected pane: its column fills the tab while the
  other columns' terminals keep running out of sight, and the tab's pill gains
  `· zoom`. Pressing it again restores the columns as they were; selecting
  another column or tab, or adding or removing a column, also ends the zoom.
  Shortcut help lists it, as do the bottom hints when there is room. Alt+Shift+Z no longer reaches
  terminal programs in a full Eon window; a popup configured on that key takes
  it in place of zoom.

- The Eon Bar shows Claude's quota as two chips, one for the five-hour window
  and one for the seven-day window, in place of one chip showing both. In a
  narrow window, quota chips now disappear starting with the one closest to
  the tabs.

- Each Eon or eonbase supervisor writes a diagnostic log to
  `$XDG_STATE_HOME/eon/logs` (`~/.local/state/eon/logs` by default): when the
  window starts and exits, its exit code or signal, its error output with a
  crash backtrace, terminals that exit on their own, tab-close timing, and the
  supervisor's own errors. A window launched from the desktop used to leave no
  record of why it closed. Eon keeps the newest 20 logs plus those of running
  supervisors, at most 1 MiB each.

- A quota chip left out of `[bar] widgets` also stops Eon reading that
  provider's quota through CodexBar, from the next window open or reopen.

- Under dead-key layouts such as `us+intl`, `"`, `~` or `^` composed on Space
  with Shift still held now reaches programs that use the kitty keyboard
  protocol without its report-all mode, such as Claude Code. It used to arrive
  as Shift+Space and type nothing.

- `[bar] widgets` chooses which Eon Bar widgets show and in what order, from
  `codex_quota`, `claude_quota`, `cpu`, `ram`, `clock` and `version`, when a
  window opens or reopens. Omitting it keeps today's bar. It replaces
  `[bar] show_version`, which now fails with an error that names
  `bar.widgets`: in place of `show_version = false`, list the widgets you
  want without `version`.

- Terminals run on Ghostty `8c425a7bd`: cursor positioning in origin mode
  and DECRQM reports follow upstream fixes, and terminals answer DECXCPR
  queries.

- `eon window stop all --json` stops every independent window without asking
  and prints one JSON array with each window's ID and its `eon stop all --json`
  results. It used to fail with `invalid Eon window ID "all"`.

- Enter on a text file in the Files popup opens it in a new Helix pane at the
  tab's directory and hides Files; Enter on a folder moves into it. `eon edit
  -- FILE` does the same from any Eon terminal.

- Alt+Enter in the Files popup sets the tab's directory to the hovered folder,
  or Yazi's current one, without moving Yazi; in its search it uses the
  selected result. Popup terminals carry their tab's identity in `EON_TAB`.

- Alt+Shift+Y opens a Files popup: Yazi at the tab's directory that keeps its
  place while hidden and when the tab's directory changes. Tab or Z searches
  recent folders and Enter moves Yazi there, Shift+Tab shows the hovered
  file's details, and file operations such as delete and rename work. A
  configured `[popups.files]` now customizes this popup, and a custom popup
  already on Alt+Shift+Y now conflicts with it, so move one of the two.

- Changing a tab's directory with Alt+Z or `eon tab directory` no longer
  restarts a running Agent on the next Alt+Shift+L. The agent moves out of its
  popup into a pane of the tab, with its conversation and old directory intact,
  and the next Alt+Shift+L starts a fresh agent in the new directory. The pane
  is selected if the Agent popup was showing or the tab had no panes.

- URLs printed as plain text, such as `http://localhost:5173/` from a dev
  server or a link in an agent's reply, highlight on hover and open with
  Ctrl+click, like links programs mark explicitly. Only `http://` and
  `https://` URLs are detected.
- Keyboard remaps such as CapsLock as Escape (`caps:escape`,
  `caps:swapescape`) work in Eon: Escape, Return, Tab, arrows and other named
  keys follow your keymap, where the remapped key used to send nothing or the
  wrong key. Letter shortcuts still follow key position, whatever the layout.
- Managed Nushell and Bash activate the host's mise when it is on PATH, so a
  project's mise-managed tools and environment work in Eon terminals as in
  your own shell. Set `[shell] mise = false` to turn it off.
- Run inside an Eon terminal, `eon pane create`, `pane close`, `pane move` and
  `tab move` act on that terminal's own tab instead of the active one, so an
  agent in a background tab no longer changes the tab you are looking at. Each
  terminal names itself in `EON_TERMINAL`; a command from a terminal that has
  ended or belongs to another generation changes nothing and exits 3. Use
  `env -u EON_TERMINAL eon ...` for the previous behavior.
- Ctrl+Alt+K/J and `eon pane move up|down` no longer close a popup that
  covers the tab to reorder the panes behind it. Like sideways moves, they
  refuse until you close the popup. Alt+K/J still closes it and selects the
  pane above or below.
- The Eon Bar shows whole-machine CPU and memory use, such as `CPU 12%` and
  `RAM 43%`, before the clock, updated every 2 seconds. Hover them for
  details. When the window is narrow, RAM hides right after the clock, then
  CPU. The readings and the clock use unfilled rounded outlines like the quota chips,
  and the `EON <version>` label moves to the edge, just before `?` and `×`.
- A popup keybinding may now use the key of a built-in Eon shortcut, such as
  Alt+Shift+N or Alt+W, instead of failing to load: the popup takes the key,
  and Alt+/ lists the built-in shortcut as overridden. Only Alt+/ and Alt+0
  through Alt+9 stay reserved.
- The component graph's product ID is `eon` instead of `eon-alpha`, so
  `eon versions` reports `eon x86_64-linux`. The canonical manifest is
  `components/eon-v4.json`. The new build still lists and stops a generation
  started before this change, but it reports that generation's component graph
  as different; reattach it with the build that started it.
- The Eon Bar shows the local time as a 24-hour `HH:MM` clock beside the
  Shortcuts and Close buttons. Hover it for the date and UTC offset. It hides
  first when the window is narrow, and it doesn't appear if Eon can't
  determine your time zone.
- `eon` and `eonbase` commands exit with a documented status per failure kind:
  1 refused or failed, 2 invalid command or arguments, 3 target not found
  (including an unknown `eon window` ID), 4 supervisor unreachable,
  130 interrupted. Structured failures exited 2 and
  every other failure 1; scripts that tested for those values need updating.
  `eon --help` and [Using Eon](docs/USAGE.md#exit-status) list the statuses.
- `eon` speaks only EONW 19. A generation started by an older build that uses
  an earlier EONW is listed as incompatible and can no longer be inspected or
  stopped by this `eon`; stop it with the build that started it.
- `eon claude-statusline` is gone; Claude quota has come from CodexBar since
  EON-C27, so the command only drained old status-line wrappers. The shared
  executable no longer refuses the retired `eonterm` name; any name other than
  `eonbase` runs Eon.
- `eon generations` no longer looks for work started before generations
  existed, and `legacy` is no longer a generation target. A supervisor that
  cannot answer a presentation request is now reported as incompatible
  instead of listed without attach.
- Ctrl+Alt+H/L now moves the selected pane into the column to its left or
  right, landing just below that column's open pane, so Ctrl+Alt+H/J/K/L moves
  a pane in every direction. A column left empty goes away, and at the tab's
  edge a pane from a stack opens a new column, up to four. Moving the active
  tab moves to Ctrl+Alt+Shift+H/L.
  `eon pane move left|right` does the same from the command line.
- Orbit, which keeps Eon's terminals alive, decides its session policy in a
  Bend worker whose laws are machine-checked, beside the Rust host that owns
  the terminal. Terminals, the protocol Venus uses, and how Eon starts Orbit
  are unchanged. Orbit's package adds the worker at
  `libexec/yazelix-orbit/yazelix-orbit-worker` and Bend's Apache-2.0 license
  under `share/licenses/orbit/bend`, and drops the `yazelix-orbit client`
  diagnostic subcommand.
- A terminal whose command exits right away, such as a failing
  `[shell].command`, no longer blocks later terminals in that workspace. New
  panes and tabs used to fail with "cannot create terminal Ready claim ...:
  File exists" until the generation was stopped; Eon now removes the ended
  terminal's leftover record when the start fails.
- The unpublished manual Linux bundle no longer uses Bubblewrap or a user
  namespace, so stock Ubuntu 26.04 can run it without an AppArmor change. It
  ships Eon's closure relinked for the host glibc and now needs glibc 2.39 or
  newer and a host Vulkan driver. Its terminals no longer inherit the
  namespace and no-new-privileges flag that blocked `sudo` and setuid
  programs. `eon-install` accepts only this layout.
- Zoxide's `zi` runs the `fzf` found on the terminal's PATH, which is Eon's
  managed fzf unless the shell puts another first, instead of a second fzf
  build compiled into Zoxide.
- Eon no longer ships Zsh or Fish. The `eon-zsh` and `eon-fish` commands and
  the managed `zsh` and `fish` names inside terminals are gone, and
  `eon versions` lists neither. Nushell and Bash remain the managed shells. A
  configuration with `command = ["eon-zsh"]` or `["eon-fish"]` now fails to
  start the terminal; use `["zsh"]` or `["fish"]` to run the host shell
  unmanaged, without Eon's Starship, Zoxide, Atuin, or Carapace integration.
- Alt+W closes the selected pane and stops its terminal, with no prompt, the
  same way Alt+Shift+W closes a tab. A column whose last pane closes goes away,
  and so does a tab left with no panes or popups. The final tab's last pane
  stays, and closing is refused while a popup or picker covers the tab.
  `eon pane close PANE` does the same from the command line, and
  `eon versions` reports EONW 11. Alt+W no longer reaches terminal programs in
  a full Eon workspace.
- EonTerm is now eonbase: the command is `eonbase`, with the same arguments,
  and the flake package and app are `.#eonbase`. There is no `eonterm` alias.
  To switch, remove the `eonterm` profile element and run
  `nix profile add github:Yazelix/eon/<tag>#eonbase`. eonbase keeps its
  generations under a new runtime directory, so terminals started by `eonterm`
  stay reachable only through that `eonterm` executable until stopped. Its
  default window application ID is `eonbase`.
- The window waits up to six seconds, instead of two, for Eon to answer a
  workspace action. Opening a popup or pane whose terminal starts slowly on a
  busy machine, within Eon's five-second start limit, no longer flashes Eon as
  unavailable. An action that waits behind other slow work, or that stops a
  terminal before starting one, can still exceed the six seconds.
- A tab can show up to four columns side by side, each a pane stack with its
  own live terminal. Ctrl+Alt+M opens a column right of the selected one with
  a new pane in the tab's directory; Alt+M adds a pane to the selected column;
  Alt+H/L move between columns and on into the previous or next tab; clicking
  another column's terminal selects it; closing a column's last pane removes
  the column. Eon refuses another column when one would be narrower than 40
  terminal columns. Ctrl+Alt+M no longer reaches terminal programs in a full
  Eon workspace.
- Pane frames draw each column as its own rounded card, with an even 8-pixel
  gap from the window's sides and between column card areas; popups and the
  Project picker draw as one card. The shortcut hints sit bare in the margin
  below the cards. Everything
  outside the cards takes the top bar's color, so the bar, the gaps and the
  hints read as one frame; with `background_opacity` below 1, only the cards
  stay see-through. The tabs keep the 8-pixel left gap, so the first tab lines
  up with the first column, and share the Eon Bar widgets' centred height.
  Terminals lose the few pixels the gaps take.
- `eon workspace` lists each pane's column and whether it is that column's
  open pane (`column=` and `expanded=`, or `"column"` and `"expanded"` with
  `--json`).
- The Claude quota chip shows how far into each window you are, like the Codex
  chip: for example `4h10m/5h 98% · 5d20h/7d 100%`.
- `eon attach` reopens with the same random cursor color the window had before
  it closed, instead of picking a new one each time. This applies when
  `cursor_trail_color` is omitted or `random`; a new workspace still picks
  again, and `preset:` and `custom:` colors are unchanged.
- Quota chips say `old` only when their last good reading is more than an hour
  old; a failed or rate-limited read no longer marks them old. Old readings
  stay visible for 24 hours instead of one.
- Eon reads Claude quota every five minutes instead of every minute, also
  after a failed read, to reduce rate-limit pressure. Codex quota is still read
  every minute.

## v0.1.1

The first numbered release. Eon behaves as in `linux-alpha-2026-10-14`;
`eon --help`, `eon versions` and the window label now report 0.1.1.

- Eon is now in beta, versioned by number. Every 0.x release is beta, and
  1.0.0 marks the end of beta. Releases bump Eon's version and carry `vX.Y.Z`
  tags instead of date tags; `linux-alpha-2026-10-14` was version 0.1.0.
  Installing without Nix is a later 0.x milestone.

## linux-alpha-2026-10-14

The first tag of this repository. It ships everything listed below and in the
summary that follows.

- Build Orbit, Venus and Eon Runtime from this repository. `eon versions`
  reports `workspace` as their revision; the generation ID on its first line
  identifies the build. Orbit's and Venus's licenses install under
  `share/licenses/orbit/` and `share/licenses/venus/`.

- Retire EON-C22 and EON-C26: the `codex app-server` collector and the Claude
  status-line collector no longer exist, and EON-C27 owns quota collection
  through CodexBar.

## Before this repository

Eon was developed across four repositories until 2026-10-14; their history is
kept in a private archive. In summary:

- The `linux-alpha-2026-09-24` tag shipped the Nix-only x86_64 Linux Wayland
  alpha: terminals that outlive the window, `eon attach`, generations and
  `eon stop`, tabs with launch directories and pane stacks, the Zoxide and Yazi
  directory picker, Git and Agent popups, EonTerm, pinned managed shells
  and tools, configurable typography, opacity and compositor blur, and the
  native Eon Bar, shortcut viewer and Codex quota chip.
- Later `edge` builds added the Anima welcome, `eon anima` and its popup,
  independent windows (Alt+Shift+N and `eon window`),
  bottom shortcut hints, the `EON <version>` label, later tabs starting in the
  home directory, Codex and Claude quota through pinned CodexBar CLI over EONW v9,
  grouped CLI help, the four-click selection ladder, Accelerando glyphs for
  `eon anima accelerando`, screen-reader caret and text geometry, the optional
  `eon-yazelix` binary cache, a pinned `nix develop` environment, and an
  unpublished manual Linux bundle and installer.
