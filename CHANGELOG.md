# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

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

- The agent sidebar marks a tab that shows a popup: its heading ends with a
  small window mark, and the row of an agent running in that popup carries
  the same mark after the popup's name. From another tab you can see which
  tabs will open on a popup. A hidden popup gets no mark.

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
  CPU. The readings and the clock sit on rounded chips like the quota chips,
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
  gap around and between the cards; popups and the Project picker draw as one
  card. The shortcut hints sit bare in the margin below the cards. Everything
  outside the cards takes the top bar's color, so the bar, the gaps and the
  hints read as one frame; with `background_opacity` below 1, only the cards
  stay see-through. The tabs sit the same 8 pixels from the window's top and
  left edges, so the first tab lines up with the first column. Terminals lose
  the few pixels the gaps take.
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
