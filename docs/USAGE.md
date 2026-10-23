# Using Eon

## Supported environment

Eon is in beta (0.x) and installs only through Nix. The supported product
target is **x86_64 Linux on native Wayland**.

Before installing, you need:

- Git and Nix with flakes enabled; and
- a native Wayland session.

Venus draws with `wgpu` on Vulkan: a hardware adapter uses the GPU, and Mesa
lavapipe can render in software.

Before Eon moved into one repository, the accepted runtime proof used COSMIC
with systemd and Mesa on Intel hardware. This repository's installed checks so
far run on a private headless Sway, plus a brief COSMIC launch check; the full
COSMIC desktop proof awaits re-proof here. Local Eon does not require systemd or cgroup delegation, but non-systemd use
and proprietary NVIDIA remain unproved. The SSH hosting mode below
requires a systemd user manager and linger on the server.

Apple Silicon macOS is the active expansion. Orbit's native proof passes in this
repository. Venus's foundation and interaction are partially re-proved
natively here (`VEN-C16`). Venus acceptance and full
Eon Nix composition remain open, so this release has no Mac install path.
X11, Xwayland, ARM Linux, Intel macOS, direct bundles, Home Manager installation,
background updates, and release automation are unsupported. Restarting the
machine ends live terminal process state.

## Install and start

Install the latest release, 0.1.1:

```sh
nix profile add --refresh github:Yazelix/eon/v0.1.1
```

Run `eon` to start. `eon versions` shows the generation of the in-tree build and
each exact external component revision; Orbit, Venus and Eon Runtime report
`workspace`.

Fresh local windows show a three-second random Anima animation after the
window is ready. Press any key other than the style-browsing keys to dismiss it;
set `[anima].enabled = false` in `config.toml` to skip it. The directory picker
then opens. Headless workspace creation and reconnect skip the welcome. Press Enter to use a Zoxide history
match, or Tab to browse folders with Yazi. In the browser, Enter chooses the
highlighted folder and F1 shows its keys. New tabs start in your home directory. Escape dismisses the picker and opens
a terminal there, keeping the tab. Close the tab with Alt+Shift+W if you no
longer want it. The initial workspace uses the directory where Eon was launched.

A terminal starts after choosing a directory or dismissing the picker. Closing the Eon window detaches
the desktop without stopping its terminals or PTY commands. Reconnect with:

```sh
eon workspaces
eon connect '' WORKSPACE_ID
```

Press **Alt+/** inside Eon to open the native Shortcuts dialog. It includes the
fixed workspace bindings and every enabled popup binding. Escape or Alt+/ closes
it.

### Named workspaces and SSH

The following commands are unreleased. Installed two-host acceptance is
recorded in [EON-C29](CONTRACTS.md#eon-c29--recoverable-ssh-named-workspaces)
and `eon-remote-ssh-x7dr.3`.

Bare `eon` and `eon window new` create a named local workspace and a new
window. Names are labels: two workspaces with the same name or directory have
different IDs. List, create and select them with:

```sh
eon workspaces
eon workspace create --name research
eon connect '' WORKSPACE_ID
```

The empty host selects Local. `connect` uses the sole open window, or opens
one when none exists. With several windows, add `--window WINDOW_ID` from
`eon windows`. The header's workspace selector groups Local and configured SSH
hosts; selecting a row switches the current window and leaves the previous
workspace running. Use its Refresh, Reconnect and Disconnect actions without
closing the window.

The server package comes from the same component manifest as the desktop
package and contains no Venus, Wayland renderer or bundled presentation fonts.
On the server, install this checkout's package into a retained Nix profile:

```sh
nix profile add .#eon-server
eon-server service install
eon-server doctor --json
```

Hosting requires an ordinary account, a reachable systemd user manager, and
linger enabled explicitly by the administrator, for example
`sudo loginctl enable-linger USER`. Linger keeps the user manager alive after
the last login ends. Each workspace supervisor has its own user service;
each Orbit terminal runs in an independent scope, so replacing a supervisor
leaves its terminals running. Local workspaces use direct launches.
`service install` records the selected server executable and any explicit
configuration, state and runtime directory overrides. Re-run it to select a
new compatible build; it does not restart existing supervisors. Keep the
chosen build in a profile or GC root while its workspaces run.

Configure authentication, host keys, `IdentityFile` and `ProxyJump` in OpenSSH,
then register the host on the client:

```sh
eon remote add compute --server-program /absolute/profile/bin/eon-server
eon workspace create --name research --host compute
eon workspaces --host compute
eon connect compute WORKSPACE_ID
```

Omit `--server-program` when `eon-server` is on the remote login PATH.
Connections use noninteractive OpenSSH; complete authentication setup first.
Connecting does not install software, enable linger or change host-key policy.
Project, panes, Git, Agent and quota collection run as the selected host's
account. Each window owns and cleans up its private SSH master and forwards.
Cached remote names remain navigation hints when a host is offline.

Closing a window or `eon disconnect [--window WINDOW_ID]` detaches and leaves
work running. `eon workspace stop WORKSPACE_ID [--host HOST]` explicitly stops
that workspace and retains its layout. `eon workspace forget WORKSPACE_ID
[--host HOST]` deletes that saved workspace only after a confirmed Stop with no
live or unresolved terminal runs. It never stops tasks implicitly. Use it to
release a slot when the 64-workspace registry is full.
`eon window stop WINDOW_ID` stops the current target and closes
the window only after Stop succeeds. A failed connection keeps the last
consistent picture and disables terminal input; use Reconnect to try again.
No queued input or uncertain mutation is replayed. A terminal attached in a
different window is reported as Busy. The last window connection detaching
closes transient Project pickers and starts a pending tab's first pane at its
starting directory; existing panes and persistent popups keep running.

The workspace owner saves tabs, columns, panes, popup slots, directories and
selection atomically. A compatible supervisor recovers the existing Orbit
runs after replacement; it never recreates them by rerunning commands.
Missing runs remain stopped. Transient pickers are not replayed. A corrupt or
unknown layout is refused and existing tasks are preserved. Orbit or machine
restart recovery is outside this feature. The current compatibility boundary
is EONW 13, ORBS 14, ORBF 2 and management/record version 2; differing build
identities alone do not reject a named workspace.

An exit observed by the running supervisor removes that pane or popup through
the usual focus rules. Stopped slots are retained when recovery discovers a
missing run. Automatic local supervisors retain startup and recovery diagnostics
in `<EON_RUNTIME_DIR>/n/WORKSPACE_ID/supervisor.log` (under Eon's default runtime
directory when no override is set). Failed startup returns the diagnostic and
log path; warnings about preserved, unassigned runs remain in that private log.
Remote supervisor diagnostics are in its systemd user service journal.

### Legacy generations

`eon run [-- COMMAND...]`, `eon attach`, generation commands and eonbase retain
their generation-based lifecycle. Use the executable that started an older,
incompatible generation to manage it.

To inspect or stop work explicitly:

```sh
eon generations
eon stop GENERATION
eon stop previous
eon stop all
```

`eon stop` shows the generation's live terminal identities and asks for
confirmation. Closing a window is therefore detach; stopping a generation ends
its terminals. Previous generations that use the current EONW can be stopped
through their own supervisor even when the current desktop cannot attach to them.
A generation from a build with another EONW version is listed as incompatible;
stop it with the build that started it.
The runtime extraction changes the component graph and selects a new generation.
Reattach earlier work with its retained exact Eon or eonbase executable.
`previous` attempts every non-dead generation except the installed build's exact
current generation, including when current has not started.
`all` includes current and stops it last. Each generation gets its own
confirmation. Declining one skips that generation; earlier Stops remain and
later prompts continue. A failed Stop does not prevent later attempts.
When the command completes, it exits nonzero if any attempt fails. `--json`
skips confirmation and returns an array of attempted results, or `[]` if there
is nothing to stop.

Run a batch command outside the generations it will stop when you need every
attempt or a complete result: stopping the caller's own terminal can terminate
the CLI mid-batch.

## Workspace and picker

A **workspace** is the whole collection of tabs and panes. A **tab** groups
panes around its launch directory and shows them in one to four side-by-side
**columns**. Each column stacks its panes and expands one. A **pane** is a view
of a persistent **terminal**, whose process, PTY and terminal state outlive the
window. Every column's expanded terminal stays live; keys go to the selected
one, and clicking another column's terminal selects it.

| Shortcut | Action |
|---|---|
| Alt+/ | Open or close shortcut help |
| Alt+Shift+B | Show or hide the bottom shortcut hints |
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+H / Alt+L | Select the column to the left or right, continuing into the previous or next tab |
| Alt+K / Alt+J | Select the previous or next pane in the column |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the selected pane into the column to the left or right; at the tab's edge a stacked pane opens a new column |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane up or down in its column |
| Ctrl+Alt+Shift+H / Ctrl+Alt+Shift+L | Move the active tab |
| Alt+Shift+T | Create a tab through the directory picker |
| Alt+M | Create a pane in the selected column |
| Alt+Shift+M | Create a column right of the selected one, up to four per tab |
| Alt+W | Close the selected pane, stopping its terminal; not the final tab's last pane |
| Alt+Shift+W | Close the active non-final tab |
| Alt+Shift+N | Open an independent Eon window (also the desktop launcher's **New Eon Window** action) |
| Alt+Z | Open the tab's Project directory picker |
| Alt+Shift+J | Open or hide the tab's Git popup |
| Alt+Shift+L | Open or hide the tab's Agent popup |
| Alt+Shift+A | Open or dismiss the tab's Anima popup |

### Eon Bar and tabs

The top row is the native Eon Bar. Its `+`, `?`, and `×` controls create
a tab, open Shortcuts, and close the active non-final tab. The empty space
between the scrollable tabs and those controls moves the undecorated window;
tabs, controls, pane headers, and terminal content remain ordinary targets.
Pinned CodexBar CLI supplies read-only Codex and Claude quota from their local
authentication. Codex also needs a compatible `codex` executable on `PATH`.
Codex chips show its white mark and Claude chips its orange splat.
Both available five-hour/seven-day windows show remaining percentages, with reset
and freshness details on hover. Missing resets stay unknown. Readings say `old`
once they are more than an hour old and disappear after 24 hours. Chips yield
before tabs and controls under width pressure, and unavailable quota leaves the
bar unchanged.
See [quota behavior](CONFIGURATION.md#agent-quota).

The bottom hints bar starts visible. Alt+Shift+B hides or restores it for this
window, returning or reserving its space without restarting terminals. Hints
follow the current bindings, enabled popup catalog and native focus; narrow
windows omit whole hints. Compact groups spread across the bar with
equal gaps and the outer groups near its edges; a lone group stays centered. Hint spacing stays compact.
Only modifier headers have padded rounded backgrounds. Each column is its own
rounded card with an even gap around it, and the hints sit bare in the margin
below the cards. Hiding the footer returns its row to terminal content.
Coral red modifier badges use plain aliases, such as A-S, before mint keys and gray
labels. The compact tab range is 1..9; Alt+0 still selects the tenth tab.
Copy/Paste and Open link stay in full help, without footer hints.
Each group uses C for Ctrl, A for Alt, and S for Shift, joined by hyphens.
The move pane hint groups H/L and K/J under C-A; the move tab hint shows H/L
under C-A-S. Full help and
screen-reader labels keep complete chords and action names. The header `?` disappears when all applicable hints
fit in the visible bar, unless that button has keyboard focus. Alt+/ always
opens the complete Shortcuts viewer, including the hints toggle. Eon has no
minor modes yet.

Tabs own launch directories; changing a shell's directory does not rename or
retarget its tab. Project changes the directory used by future terminals in the
captured tab without moving existing terminals. Git and Agent are tab-scoped
terminals whose terminal state survives hiding and reopening. Anima is transient:
closing its popup stops playback and returns to the tab.

### Scrolling and directory picker

When you scroll above live output, `↓ N rows` shows how many display rows back
you are. It appears in Eon's pane header or at the top right of eonbase, updates
as output continues, and disappears at live output. Click the whole pill to
return there.

Two clicks select a word, three select a
space/tab-delimited span, and four or more select the logical line across soft
wraps. Dragging extends that granularity; a delayed single click returns to
cell selection. Release copies to both Wayland clipboard destinations, and
Ctrl+Shift+C copies the frozen selection even after output changes.

The directory picker starts with Zoxide and fzf. Tab switches to Yazi, Shift+Z
jumps through Zoxide history from the browser, `g h` goes home, `g /` goes to
root, `g Space` accepts a folder path, and F1 opens browser help. `a` uses
Yazi's native **Create:** prompt: a plain name creates a
file; a name ending in `/` creates a folder. Creation highlights the item;
Enter selects a folder. Esc returns input to normal mode or closes it;
in help, it clears a filter or closes help. Existing targets require
Yazi's native confirmation. Cancelling leaves created files and folders on disk.
If creation fails, the pinned Yazi returns to browsing without an error dialog.
Files cannot be selected as a tab directory or opened by the picker.
Browsing, find, sorting and previews use Yazi defaults: `gg`/`G` reach the first
or last entry, `H`/`L` go back/forward, and `/`/`?` with `n`/`N` find entries.

## CLI commands

`eon -h` and `eon --help` show grouped commands,
short descriptions, and examples without starting a workspace. Help uses color
on terminals; redirected output is plain text. A nonempty `NO_COLOR` or
`TERM=dumb` disables color. Invalid syntax points to `eon --help`.

| Command | Result |
|---|---|
| `eon` | Create a named local workspace and window |
| `eon anima [STYLE] [CHILD OPTIONS...]` | Run pinned Anima in the caller's terminal without starting a workspace; `--help` lists styles and options |
| `eon run [-- COMMAND...]` | Use the legacy generation entry, optionally with an exact first command |
| `eon window new` | Open an independent Eon window and print its window ID |
| `eon windows [--json]` | List windows, accepted targets and connection states |
| `eon window attach ID` | Present an existing window |
| `eon window stop ID [--json]` | Stop the window's selected workspace and close it |
| `eon window stop all` | Attempt Stop in every independent window |
| `eon attach [GENERATION]` | Present the current or one selected compatible generation |
| `eon generations [--json]` | List validated current and older generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor |
| `eon stop previous [--json]` | Attempt every other non-dead generation |
| `eon stop all [--json]` | Attempt every non-dead generation, including current |
| `eon workspace [--json]` | Inspect the live tab, pane, popup, and terminal mapping |
| `eon tab create [--json]` | Create a distinct pending tab with its directory picker, even while another picker is open |
| `eon tab close TAB [--json]` | Close the expected active non-final tab |
| `eon tab directory TAB [--json] -- DIRECTORY` | Set a tab's directory for future terminals |
| `eon tab move left\|right [--json]` | Move the active tab one position |
| `eon pane create [--json]` | Create a pane in the active tab's selected column |
| `eon pane close PANE [--json]` | Close a pane of the active tab, stopping its terminal |
| `eon pane move up\|down [--json]` | Move the selected pane one position within its column |
| `eon pane move left\|right [--json]` | Move the selected pane into the neighbouring column, as Ctrl+Alt+H/L does |
| `eon focus ID\|left\|right\|up\|down [--json]` | Focus a stable identity or traverse the workspace |
| `eon versions` | Print Eon, protocol, and component identities |
| `eon config-path` | Create and print the configuration root |

Inside a named workspace, tab, pane and popup commands target its supervisor
even after the window switches away. Outside a named workspace they retain the
legacy current-generation target. Human and `--json`
output describe the same result. A missing supervisor is not started implicitly;
run `eon` first.
Eon calls Orbit-backed instances terminals in human output.
JSON retains the technical `session` / `sessions` fields and `session-N` IDs;
these refer to terminals.
Named workspace commands carry their workspace endpoint. The legacy entry
keeps its original window or default namespace. `eon stop all` covers generations in that
current namespace. `eon window stop all` covers independent windows and visits
the caller's window last. Run it outside those terminals when you need its final
status; stopping its own terminal can terminate the CLI.

### Accelerando glyphs

In Eon's native terminal, `eon anima accelerando` shows the 8,192-glyph alien
alphabet in long green trails; add `--symbols` to mix in letters, digits and
punctuation. Add `--big` for large, solid block glyphs; combine `--big --symbols`
for matching large letters, digits and punctuation. Add `--bold` for heavier
native glyphs; it combines with either option. Large solid blocks already fill
their cells. Random playback includes Accelerando. Eon bundles Regular and Bold
Accelerando Terminal faces; other terminals need these fonts for native-size
playback. There is no substitute alphabet.

## eonbase

Install eonbase when one exact command needs a native terminal surface without
Eon's workspace or managed environment:

```sh
nix profile add .#eonbase
eonbase -- COMMAND...
eonbase --no-decorations -- COMMAND...
```

Its lifecycle commands mirror Eon's:

```sh
eonbase generations
eonbase attach [GENERATION]
eonbase stop GENERATION
```

Eon and eonbase use separate runtime namespaces. eonbase keeps native window
decorations by default and accepts `--application-id ID` for an approved
composition.

## Troubleshooting and reporting

- **Nix cannot fetch an input:** check access to GitHub and retry. Eon
  fetches only external inputs; Orbit, Venus and Eon Runtime build
  from this repository.
- **A workspace command reports a missing supervisor:** run `eon`, then retry.
- **The window closed but commands remain:** this is detach behavior. Use
  `eon attach` to return or `eon stop GENERATION` to end them.
- **A replacement window rejects configuration:** fix `config.toml` and run
  `eon attach`; the existing terminals remain alive.
- **Graphics fail outside native Wayland or on unproved hardware:** that
  environment is outside the supported boundary.

Report reproducible failures in [GitHub Issues](https://github.com/Yazelix/eon/issues).
Include `eon versions`, the relevant generation from `eon generations`, your
distribution, compositor, GPU, and the exact command and error. Remove paths,
environment values, terminal content, and credentials that should remain
private.
