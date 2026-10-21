# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- `eon window stop all --json` stops every independent window without asking
  and prints one JSON array with each window's ID and its `eon stop all --json`
  results. It used to fail with `invalid Eon window ID "all"`.

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
- The Eon Bar shows whole-machine CPU use, such as `CPU 12%`, before the
  clock, updated every 2 seconds. Hover it for details. It hides right after
  the clock when the window is narrow.
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
- `eon` speaks only EONW 13. A generation started by an older build that uses
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
  workspace action, matching Eon's own commands. Opening a popup or pane whose
  terminal starts slowly on a busy machine, within Eon's five-second start
  limit, no longer flashes Eon as unavailable. An action that waits behind
  other slow work, or that stops a terminal before starting one, can still
  exceed the six seconds.
- A tab can show up to four columns side by side, each a pane stack with its
  own live terminal. Alt+Shift+M opens a column right of the selected one with
  a new pane in the tab's directory; Alt+M adds a pane to the selected column;
  Alt+H/L move between columns and on into the previous or next tab; clicking
  another column's terminal selects it; closing a column's last pane removes
  the column. Eon refuses another column when one would be narrower than 40
  terminal columns. Alt+Shift+M no longer reaches terminal programs in a full
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
