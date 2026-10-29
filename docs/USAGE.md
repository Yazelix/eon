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
COSMIC desktop proof awaits re-proof here. Eon does not require systemd or cgroup delegation, but non-systemd use and
proprietary NVIDIA remain unproved.

Apple Silicon macOS is the active expansion. Orbit's native proof passes in this
repository. Venus's foundation and interaction are partially re-proved
natively here (`VEN-C16`). Venus acceptance and full
Eon Nix composition remain open, so this release has no Mac install path.
X11, Xwayland, ARM Linux, Intel macOS, direct bundles, Home Manager installation,
background updates, and release automation are unsupported. Restarting the
machine ends live terminal process state.

## Install and start

Install the latest release, 0.1.2:

```sh
nix profile add --refresh github:Yazelix/eon/v0.1.2
```

Run `eon` to start. `eon versions` shows the generation of the in-tree build and
each exact external component revision; Orbit, Venus and Eon Runtime report
`workspace`.

Fresh workspaces first show a three-second random Anima animation after the
window is ready. Press any key other than the style-browsing keys to dismiss it;
set `[anima].enabled = false` in `config.toml` to skip it. The directory picker
then opens. Press Enter to use a Zoxide history
match, or Tab to browse folders with Yazi. In the browser, Enter chooses the
highlighted folder and F1 shows its keys. New tabs start in your home directory. Escape dismisses the picker and opens
a terminal there, keeping the tab. Close the tab with Alt+Shift+W if you no
longer want it. The initial workspace uses the directory where Eon was launched.

A terminal starts after choosing a directory or dismissing the picker. Closing the last
tab quits the window and stops its terminals; in a window with several
workspaces, it closes only that workspace. By default Eon asks first, listing the
terminals that stop, and the question names the workspace while others remain.
Closing the Eon window instead detaches the desktop without stopping its
terminals or PTY commands. Alt+Shift+D does the same from the keyboard. The
first time, Eon shows a desktop notification saying it is still running in the
background. Reconnect with:

```sh
eon attach
```

Press **Alt+/** inside Eon to open the native Shortcuts dialog. It includes the
fixed workspace bindings and every enabled popup binding. Escape or Alt+/ closes
it.

### Stop terminals

To inspect or stop work explicitly:

```sh
eon generations
eon stop GENERATION
eon stop previous
eon stop all
```

`eon stop` shows the generation's live terminal identities and asks for
confirmation. Closing a window is therefore detach; stopping a generation ends
its terminals. To end every terminal in the window you are in, open Commands
(Alt+Shift+M) and choose Quit this window: it runs `eon stop` for that window's
generation, with the same confirmation, and stops only that window.
At a terminal, any answer but `y`, `Y`, `yes` or `YES` cancels
and exits 0. Without a terminal, a piped yes still confirms; any other answer,
including end of input, exits 1 with `not-confirmed` and sends no Stop.
Previous generations that use the current EONW can be stopped
through their own supervisor even when the current desktop cannot attach to them.
A generation from a build with another EONW version is listed as incompatible;
stop it with the build that started it. Each generation records its own
build's `eon`, and `eon generations` lists it as the executable of a generation
this `eon` cannot present. `eon attach GENERATION` and `eon window attach ID`
present such a generation by running that build's `eon`. Generations started
before Eon recorded it need that build's `eon` by hand.
The runtime extraction changes the component graph and selects a new generation.
Reattach earlier work with its retained exact Eon or eonbase executable.
`previous` attempts every non-dead generation except the exact current
generation of the `eon` that runs it, including when current has not started.
Inside an Eon terminal that `eon` is the terminal's own build, so `previous`
run from an older window can stop a newer generation in the same runtime
directory; each confirmation names the generation.
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
| Alt+Shift+H | Show or hide the agent sidebar |
| Alt+G / Alt+Shift+G | Jump to the next or previous agent needing attention |
| F6 | Move keyboard focus: terminal, agent sidebar, tabs, panes |
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+H / Alt+L | Select the column to the left or right, continuing into the previous or next tab |
| Alt+K / Alt+J | Select the previous or next pane in the column |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the selected pane into the column to the left or right; at the tab's edge a stacked pane opens a new column |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane up or down in its column |
| Ctrl+Alt+Shift+H / Ctrl+Alt+Shift+L | Move the active tab |
| Alt+Shift+T | Create a tab through the directory picker |
| Alt+M | Create a pane in the selected column |
| Ctrl+Alt+M | Create a column right of the selected one, up to four per tab |
| Alt+Shift+Z | Show the selected pane alone across the tab, or restore the columns; changing the selected pane, tab or layout also restores them |
| Alt+W | Close the selected pane, stopping its terminal; the final tab's last pane quits the window, or closes the workspace while others remain |
| Alt+Shift+W | Close the active tab; closing the final tab quits the window, or closes the workspace while others remain |
| Alt+Shift+N | Open an independent Eon window (also the desktop launcher's **New Eon Window** action) |
| Alt+Shift+D | Detach: close the window, keep terminals running |
| Alt+Z | Open the tab's Project directory picker |
| Alt+Shift+J | Open or hide the tab's Git popup |
| Alt+Shift+L | Open or hide the tab's Agent popup |
| Alt+Shift+Y | Open or hide the tab's Files popup |
| Alt+Shift+A | Open or dismiss the tab's Anima popup |
| Alt+Shift+M | Open or dismiss the tab's Commands popup |
| Alt+Shift+S | Open or dismiss the Workspaces picker: Enter shows the highlighted workspace, Enter on an unmatched name creates one |

### Eon Bar and tabs

The top row is the native Eon Bar. Its `+`, `?`, and `×` controls create
a tab, open Shortcuts, and close the active tab. The empty space
between the scrollable tabs and those controls moves the undecorated window;
tabs, controls, pane headers, and terminal content remain ordinary targets.
A zoomed tab shows a zoom-in magnifier after its name, or ` · zoom` if the
bundled Symbols Nerd Font face is unavailable. Its accessible name says `, zoomed`.
While the window holds two or more workspaces, or its one workspace has a name,
an outlined label before the tabs names the shown workspace, or gives its ID
such as `w2`, followed, when there are several, by its position and the
count, as in `notes 2/3`.
Clicking it, or Enter after Tab focuses it, opens the Workspaces picker as
Alt+Shift+S does, and its tooltip names that key. With the Workspaces popup
turned off the label is plain text. While another workspace has an agent that
needs input, a ◆ badge sits on the label's corner, also while the sidebar is
shown, and its accessible name adds `, another workspace needs input`. A long
name is shortened while the position stays whole; quota chips hide before the
label, and the label hides before the tabs would lose their room.
Pinned CodexBar CLI supplies read-only Codex and Claude quota from their local
authentication. Codex also needs a compatible `codex` executable on `PATH`.
Codex chips show its white mark and Claude chips its orange splat.
Codex's chip shows its available windows; Claude gets one chip per available
window, five-hour then seven-day. Each window shows its remaining percentage,
with reset and freshness details on hover. Missing resets stay unknown.
Readings say `old` once they are more than an hour old and disappear after 24
hours. Chips yield before tabs and controls under width pressure, the one
closest to the tabs first, and unavailable quota leaves the bar unchanged.
See [quota behavior](CONFIGURATION.md#agent-quota).
After the quota chips, `CPU 12%` shows whole-machine CPU use over the last 2
seconds and `RAM 43%` shows whole-machine memory in use; hover them for
details. A 24-hour clock follows and shows local time; hover it for the date
and UTC offset, and it stays hidden if the local time zone can't be
determined. The `EON <version>` label sits at the edge, just before `?` and
`×`. When the bar runs short of space the clock hides first, then RAM, then
CPU.

The bottom hints bar starts visible. Alt+Shift+B hides or restores it for this
window, returning or reserving its space without restarting terminals. Hints
follow the current bindings, enabled popup catalog and native focus; narrow
windows omit whole hints. Compact groups spread across the bar with
equal gaps and the outer groups near its edges; a lone group stays centered. Hint spacing stays compact.
Only modifier headers have padded rounded backgrounds. Each pane is its own
rounded card, stacked without gaps in its column's allocated card area. Even
outer gaps surround and separate the allocated column areas. Pane titles sit
in rounded edges, with neutral outlines sized by `border_width`. Collapsed rows'
corners turn toward the expanded pane, and their sides join the next card in
that direction.
Pane labels come from Runtime: the pane ID, then a pinned name, otherwise its
sanitized terminal title, otherwise its compact working directory. A differing
terminal directory adds ` · DIRECTORY`, using a relative or shorter compact path.
Labels keep explicit connecting, unavailable and offline states. The ID is dimmer
when the resolved background, including an extended header band, and opacity
permit readable contrast; narrow labels
keep the ID when it fits, while accessibility and close listings retain the full
bounded label. Metadata and names are display text, never command input.

The selected pane has a brighter label and a whole outline in
its focus colour, at the configured focus border width; hover adds a label pill and
F6 keyboard focus keeps a mint pill outline. Turning pane frames off leaves labels and
their interaction feedback. The hints sit bare in the margin below the cards. Hiding the footer returns its row to terminal content.

With pane frames on, each column has one continuous terminal-background fill,
from its first pane's rounded top edge down to the column's rounded bottom.
Stacked title rows and the space between cards share the expanded terminal's
background colour and opacity, except that extended top padding fills an
unzoomed expanded pane's band below its title (`padding_color`). Above that edge
and outside the outer corners, the gaps and footer stay the opaque bar colour.
The first title sits on that boundary. Popup and Agents fills follow their
rounded outer edges in the same way, with their titles on the boundary. Header
scrolling keeps the fill's top fixed. Frameless mode keeps terminal-background
strips across the window below the bar, including the sidebar and footer.

Steel-blue modifier badges use plain aliases, such as A-S, before mint keys and gray
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
terminals whose terminal state survives hiding and reopening. When the tab's
directory changes, a running Agent moves out of its popup into a pane of the
tab, with its conversation and old directory intact, and the next Alt+Shift+L
starts a fresh agent in the new directory. Git restarts in the new directory
the next time you open it. Files is Yazi at the tab's directory: Tab or Z
searches recent folders and Enter moves Yazi there, Shift+Tab shows the
hovered file's details, Enter on a folder moves into it, and every other Yazi
key, including file operations, works as usual. It keeps its place when the tab's directory changes; quitting Yazi with
`q` closes it, and the next Alt+Shift+Y opens a fresh one. Alt+Enter sets the
tab's directory to the hovered folder, or Yazi's current one, without moving
Yazi; in search it uses the selected result. Enter on a text file opens it in
a new Helix pane beside your work and hides Files, one pane per selected file;
other files keep Yazi's default openers. Anima is transient:
closing its popup stops playback and returns to the tab.

Commands lists Eon commands you can search and run: new tab, pane or
window, close or move the tab, quit this window, switch, create, rename or
close a workspace, show the workspace, windows, generations, versions or config
path, stop older generations, and command help. Each line shows the `eon`
command it runs. Enter runs it from the popup
for that tab, with the command's own checks and confirmations. New pane
closes the popup first, as Alt+M does, and the pane joins the tab's selected
column. Other actions close the popup when they succeed; listings, stopping
older generations and failures stay until you press Enter. Esc closes it
without running anything.

Workspaces (Alt+Shift+S, or Commands > Switch workspace) lists this window's
workspaces most recently shown first, each by name or ID with its tab count and
` · needs input` when one of its agents needs input, the shown one last and
marked, so Alt+Shift+S then Enter returns to the previous workspace. Type to
filter by name. Enter shows the highlighted workspace, and Enter on a name that
matches no workspace creates a workspace with that name and shows it. A name
that is part of an existing one matches it, so create that workspace with
Commands > New workspace and then rename it. Esc or Ctrl+C closes the picker
without changing anything. Rename
workspace names the popup's workspace (an empty name clears it), and Close this
workspace asks before it stops every terminal in it.

### Agent sidebar

In full Eon windows the agent sidebar lists, in tab order, the tabs whose
coding agents report their status. Its card looks like a pane card, with the
same outline, background and background opacity. Each tab gets a heading such
as `3  site`, then one row per reporting terminal: a popup's row first, then
the panes' rows in pane order. A row shows:

- the state: ◆ needs input, ✕ error, ✓ finished, stepping braille dots while
  working (still without GPU rendering or with
  `[workspace] animate_working_badge = false`), three still dots in a row while
  the agent waits on background work (it reported done or idle while processes
  it started still run), or ○ idle;
- the Claude or Codex icon;
- a Claude Code session's name, such as `eon-9a` for a session started
  with `claude --name eon-9a`; otherwise the title the agent reports, and
  without that, the app's name, such as `Codex`;
- how long ago the state changed (`now`, `4m`, `2h`, `3d`);
- the pane (`p3`) or popup it runs in.

A needs-input row adds the agent's question. `+2` counts the terminal's other
status records. A dimmed row marked `status stale` has not reported for more
than 3 seconds. An Agent popup whose agent does not report shows `status
unavailable`. A small window mark follows the popup's name on the row of an
agent running in a popup, whether or not the popup is shown. The same mark
after a tab's heading means that tab shows a popup now, so switching to it
brings the popup up. While no agent reports, the sidebar says so.

Click a row, or press Enter or Space on it, to jump to that agent: Eon selects
its tab and shows its pane or popup, and keyboard focus moves to that terminal.
If a transient popup or a pending directory picker covers the agent, the jump
only selects the tab, and focus moves to the tabs. F6 moves keyboard focus from
the terminal into the sidebar, back to the row it last focused while that row
remains, then to the tabs and panes. Up and Down move between rows, and Escape
returns to the terminal. Rows that don't fit are cut off at the bottom, and
keyboard focus scrolls them into view.

Alt+G jumps to the next agent that needs attention, in any workspace of the
window, whether the sidebar is shown, hidden or turned off; jumping to an agent
in another workspace shows that workspace. A finish or error in a workspace you
are not showing stays new until you show it with that agent's terminal
selected; a finish also stops being new once that terminal reports work again.
Agents that need input come first, then errors, then finished agents,
each oldest first; an error or a finish leaves the walk once you have looked at
that agent. Each press moves one agent further, wrapping around; Alt+Shift+G
steps back. The walk starts again from the front
once you select another terminal some other way, such as with a click, a
sidebar row or Alt+H, or when an agent needs something more urgent than the one
you last jumped to, for example input while you were walking finished agents.
A jump to an agent under a
transient popup or a pending directory picker only selects its tab. With no
agent waiting, the keys show "No agent needs attention" and move nothing.

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
| `eon` | Present the current workspace or start its first directory picker |
| `eon anima [STYLE] [CHILD OPTIONS...]` | Run pinned Anima in the caller's terminal without starting a workspace; `--help` lists styles and options |
| `eon run [-- COMMAND...]` | Start a fresh workspace picker, optionally for an exact first command |
| `eon window new` | Open an independent Eon window and print its window ID |
| `eon windows [--json]` | List independent windows and their running generations, or that a window has ended |
| `eon window attach ID` | Present one window's live generation after detach, through the Eon build that started it when needed |
| `eon window stop ID [--json]` | Stop only the selected window's terminals |
| `eon window stop all [--json]` | Attempt Stop in every independent window |
| `eon attach [GENERATION]` | Present the current or one selected compatible generation |
| `eon generations [--json]` | List validated current and older generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor |
| `eon stop previous [--json]` | Attempt every other non-dead generation |
| `eon stop all [--json]` | Attempt every non-dead generation, including current |
| `eon workspaces [--json]` | List the window's workspaces in order, with each one's ID, name, whether it is shown, its tab count, its most urgent agent attention state, and its recency (0 for the shown one, then 1 for the one shown before it, and so on, the Workspaces picker's order) |
| `eon workspace [--json]` | Inspect the live tab, pane, popup, and terminal mapping, with each terminal's agent attention state, app and name; JSON includes each terminal's full agent attention record and the Codex and Claude quota readings, and the calling terminal in `caller` (null without `EON_TERMINAL`; a `caller` line in human output only when present) |
| `eon workspace new [--name NAME] [--no-focus] [--json]` | Add a workspace after the shown one, with one tab and its directory picker, and show it; with `--no-focus` the shown workspace stays. The reply names the new workspace's ID in `created` (`created wN` in human output) |
| `eon workspace switch WS [--json]` | Show workspace WS, with its tabs, panes and popups as it left them |
| `eon workspace rename [WS] NAME [--json]` | Name workspace WS, or the caller's workspace; an empty NAME clears the name |
| `eon workspace close WS [--json]` | Stop every terminal of WS and remove it, asking first without `--json`; closing the last workspace quits the window |
| `eon tab create [--json]` | Create a distinct pending tab with its directory picker, even while another picker is open |
| `eon tab create [--json] [--no-focus] --directory DIRECTORY [-- COMMAND [ARGS...]]` | Create a tab at DIRECTORY without a picker, optionally running an exact command from a calling terminal; `--no-focus` preserves the active tab, selection and popup |
| `eon tab close TAB [--json]` | Close the expected active tab; the final tab quits the window, or closes the workspace while others remain, asking first from a terminal |
| `eon tab directory TAB [--json] -- DIRECTORY` | Set a tab's directory for future terminals |
| `eon tab move left\|right [--json]` | Move the active tab, or the calling terminal's tab, one position |
| `eon pane create [--json] [--no-focus]` | Create a pane in the active tab's selected column, or in the calling pane's column; with `--no-focus`, the tab's selected pane and any shown popup stay as they are |
| `eon pane create [--json] [--no-focus] -- COMMAND...` | From an Eon terminal, run COMMAND exactly as given (no shell) in a new pane of the caller's column, at the tab's directory, with `--no-focus` as above; refused with exit 1 outside an Eon terminal |
| `eon edit [--json] -- FILE` | Open FILE in Helix in a new pane, placed as `pane create` places it, at the tab's directory |
| `eon pane close PANE [--json]` | Close a pane of the active tab, or of the calling terminal's tab, stopping its terminal; the final tab's last pane quits the window, or closes the workspace while others remain, asking first from a terminal |
| `eon pane move up\|down [--json]` | Move the selected pane, or the calling pane, one position within its column |
| `eon pane move left\|right [--json]` | Move the selected pane, or the calling pane, into the neighbouring column, as Ctrl+Alt+H/L does |
| `eon focus ID\|left\|right\|up\|down [--json]` | Focus a stable identity or traverse the workspace |
| `eon versions` | Print Eon, protocol, and component identities |
| `eon config-path` | Create and print the configuration root |
| `eon config defaults [--json]` | Print built-in defaults as commented TOML or a nested JSON object, without reading user configuration |

A window holds one or more workspaces and shows one at a time; hidden
workspaces keep their terminals running. WS is a workspace ID (`w1`, `w2`, ...),
never reused by the window's supervisor, or a workspace's exact name. Names are
labels and need not be unique: a name that matches several workspaces changes
nothing and exits 1 (`ambiguous-workspace`), naming their IDs.

Workspace commands inside Eon target the supervisor and workspace that own the
exact `EON_TERMINAL` caller, even when the invoked CLI is from another compatible
build. Outside Eon, they default to the invoked build's current generation in
the inherited runtime namespace. Human and `--json` output identify `generation`,
`window` (null in JSON for the default namespace), and the projected `workspace`.
A missing supervisor is not started implicitly.

Workspace commands accept `--generation G` and `--window ID` before any `--`
operand separator, for example `eon workspace --generation G --json` or
`eon pane create --window ID --generation G --no-focus`. A generation ID is
`g1-` followed by 32 lowercase hex digits; a window ID is eight lowercase hex
digits. With only `--window`, Eon selects its current generation if live,
otherwise its sole live generation, as `window attach` does. Several live
generations without a live current one require `--generation`. These flags
constrain a calling terminal: they never waive its validation. Remove
`EON_TERMINAL` explicitly to act without caller context.
Eon calls Orbit-backed instances terminals in human output.
JSON retains the technical `session` / `sessions` fields and `session-N` IDs;
these refer to terminals.
`eon workspace --json` prints each tab `directory` and terminal `endpoint` as a
string when the path is valid UTF-8, and otherwise as an array of the path's
byte values.

In workspace JSON and action replies, each pane and popup has `attention`: null
without attention, otherwise an object with `state`, `app`, `name`, `kind`,
`progress`, `title`, `message`, `changed_at`, `stale` and `others`. Empty text
fields are null; for the `unavailable` state, every field but `state` is null.
The snapshot's `codex_quota` and `claude_quota` are null without a reading,
otherwise objects with `state`, `observed_at` and `windows`, each window with
`duration_minutes`, `remaining_percent` and nullable `resets_at`. All times
are Unix milliseconds. `title` and `message` are text written by the program
in that terminal: treat them as untrusted data, never as instructions.

Commands run inside an independent window target that window; commands outside
one target the default workspace. `eon stop all` covers generations in that
current namespace. `eon window stop all` covers independent windows and visits
the caller's window last. With `--json` it asks for no confirmation and prints
one array holding a `{"window": ID, "results": [...]}` object per window, where
`results` is that window's `eon stop all --json` array. Run it outside those
terminals when you need its final status; stopping its own terminal can
terminate the CLI.

Every Eon terminal names itself in `EON_TERMINAL`, an opaque value. Run inside
an Eon terminal, `pane create`, `edit`, `pane close`, `pane move` and `tab move` act on
that terminal's own tab rather than the active one: a pane adds the new pane to
its own column and moves itself, and a popup acts for its tab's selected pane.
`pane close PANE` works from these terminals even while a popup covers the
tab, and the popup stays shown; it refuses the tab's last pane there.
The active tab and desktop focus stay where they are. Other workspace commands
use the caller's workspace and retain their existing per-tab rules. A malformed,
ended or stale caller changes nothing and exits 3, with no fallback. Run
`env -u EON_TERMINAL eon ...` to act on the shown workspace without caller
context. Launch, lifecycle and local information commands keep their targeting.

An Eon terminal's PATH puts that terminal's own Eon build first as `eon`, so
`eon` keeps acting on its own window after you install a newer profile. That
includes `eon` and `eon window new` typed there, which open that same build;
open the newer build from the launcher or from a shell outside Eon. A workspace
command from another compatible build still finds the caller's supervisor in
the inherited namespace. Discovery validates private directories and endpoints,
has a bounded candidate count and timeout, and refuses ambiguous or incomplete
resolution. An unavailable or incompatible supervisor exits 4 and names the
generation, pointing to the terminal's own `eon`; it never launches, presents or
restarts a supervisor. Unknown explicit targets exit 3, malformed flags or IDs
exit 2, and ambiguous window selection exits 1.

### Exit status

`eon` and `eonbase` commands exit with the same status with or without
`--json`:

| Status | Meaning |
|---:|---|
| 0 | Success, including a Stop or close confirmation declined at a terminal |
| 1 | The command was understood but refused or failed, including a confirmation that got no yes when stdin is not a terminal |
| 2 | Invalid command or arguments, such as a malformed tab or generation ID |
| 3 | The named tab, pane, terminal, popup, generation or window was not found |
| 4 | The supervisor is not running or did not answer; outside Eon, run `eon` first |
| 130 | Interrupted with Ctrl+C |

The `code` in an `error` result, such as `unknown-tab`, names the failure more
precisely. `eon stop previous|all` and `eon window stop all` exit with the
largest status among their attempts. `eon window attach|stop ID` exits with the
selected window's status, and `eon window attach ID` exits 4 once that window
has ended. An `eon attach` that runs the `eon` of another build exits with that
`eon`'s status. Commands that run another program instead exit with its status:
`eon anima` with Anima's, and `eon run -- COMMAND` or `eonbase -- COMMAND` with
the initial command's once it has ended.

### Agents in Eon

Eon packages an `eon` skill that teaches agents the CLI, how to find their own
pane or popup through `eon workspace --json`'s `caller`, and how to leave your
focus alone. Install it for the agents you use. For a `nix profile` install in
`~/.nix-profile`, run the relevant commands below; substitute your profile path
if you installed with `--profile`. The links follow profile upgrades.

Claude Code:

```sh
mkdir -p ~/.claude/skills
ln -s ~/.nix-profile/share/eon/skills/eon ~/.claude/skills/eon
```

Codex:

```sh
mkdir -p ~/.agents/skills
ln -s ~/.nix-profile/share/eon/skills/eon ~/.agents/skills/eon
```

Remove a skill by deleting its link: `rm ~/.claude/skills/eon` or
`rm ~/.agents/skills/eon`.

Optionally add a session-start hook. Merge the following `SessionStart` entry
into `~/.claude/settings.json`, preserving your other settings and hooks:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "sh -c '[ -n \"$EON_TERMINAL\" ] || exit 0; printf \"This session runs in an Eon terminal; the eon skill explains the eon CLI; leave focus where the person put it\\n\"; eon workspace 2>/dev/null || true'",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

For Codex, merge the same entry into `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "sh -c '[ -n \"$EON_TERMINAL\" ] || exit 0; printf \"This session runs in an Eon terminal; the eon skill explains the eon CLI; leave focus where the person put it\\n\"; eon workspace 2>/dev/null || true'",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

If you already keep hooks in the `[hooks]` table of `~/.codex/config.toml`, add
the entry there instead; Codex warns when both that table and
`~/.codex/hooks.json` contain hooks.

Codex runs a user hook only after you trust it in `/hooks`. Both examples run
the same command through `sh -c`, including when Codex invokes hooks through
`$SHELL -lc` with nushell as your login shell. The hook stays silent outside
Eon. Inside Eon it prints one sentence and the human `eon workspace` output,
including the caller line; a failed workspace query does not fail the session.
Remove the entry to remove the hook.

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
- **A window closed on its own:** each Eon or eonbase supervisor keeps a log
  in `$XDG_STATE_HOME/eon/logs` (`~/.local/state/eon/logs` by default), named
  `GENERATION-SECONDS-PID.log`. It records when the window started and how it
  exited, with its error output and any crash backtrace, when terminals exit on
  their own, and how long each tab close took. Eon keeps the newest 20 logs
  plus those of running supervisors, and stops a log at 1 MiB. Orbit's own
  diagnostics go to `$XDG_STATE_HOME/eon/logs/orbit`
  (`~/.local/state/eon/logs/orbit` by default), named
  `GENERATION-SESSION-RUN.log`; Eon keeps the newest 100 plus those of running
  Orbit hosts.
- **A command works in Yazelix Nova but not in Eon:** Eon's managed Nushell
  and Bash load your own shell configuration (`~/.config/nushell` and
  `~/.bashrc`), while Nova's Nushell reads `~/.config/yazelix/nu` in place of
  `~/.config/nushell/config.nu` and `env.nu`. Environment variables, secrets
  and hooks set in your own configuration apply in Eon terminals, so look
  there first.
- **A replacement window rejects configuration:** fix `config.toml` and run
  `eon attach`; the existing terminals remain alive.
- **Graphics fail outside native Wayland or on unproved hardware:** that
  environment is outside the supported boundary.

Report reproducible failures in [GitHub Issues](https://github.com/Yazelix/eon/issues/new?template=bug_report.yml).
Include `eon versions`, the relevant generation from `eon generations`, your
distribution, compositor, GPU, and the exact command and error. Remove paths,
environment values, terminal content, and credentials that should remain
private.

### Naming a pane

`eon pane rename p1 'Review pane labels'` pins a name to that pane. Use
`eon pane rename p1 --clear` or an empty name to restore automatic naming.
Names are trimmed, limited to 80 Unicode characters, and reject control and
invisible format characters. `eon pane rename p1 -- '--clear'` pins an
option-looking name; put `--json` before `--` to request JSON with a literal name.
Rename targets the explicit pane in the addressed workspace, including background
tabs, and preserves selections and focus. Commands offers **Rename pane** and
**Clear pane name** for the popup tab's selected pane; Enter on an empty name
clears it, and Escape cancels. Pins follow pane moves and survive presentation
detach/reattach, but end with the pane or supervisor and are not recovered.
`eon workspace --json` and action replies expose each pane's full `label` and
nullable `pinned_name`, including panes without agent-attention records.
