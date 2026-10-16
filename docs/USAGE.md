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

Apple Silicon macOS is the active expansion. Orbit's native proof and Venus's
partial M1 proof, including physical Scaleway tests, predate this repository and
await re-proof here. Venus acceptance and full Eon Nix composition remain open,
so this release has no Mac install path.
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

Fresh workspaces first show a three-second random Anima animation after the
window is ready. Press any key other than the style-browsing keys to dismiss it;
set `[anima].enabled = false` in `config.toml` to skip it. The directory picker
then opens. Press Enter to use a Zoxide history
match, or Tab to browse folders with Yazi. In the browser, Enter chooses the
highlighted folder and F1 shows its keys. New tabs start in your home directory. Escape dismisses the picker and opens
a terminal there, keeping the tab. Close the tab with Alt+Shift+W if you no
longer want it. The initial workspace uses the directory where Eon was launched.

A terminal starts after choosing a directory or dismissing the picker. Closing the Eon window detaches
the desktop without stopping its terminals or PTY commands. Reconnect with:

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
its terminals. Previous generations using EONW v2 through v7 can be stopped
through their own supervisor even when the current desktop cannot attach to them.
The runtime extraction changes the component graph and selects a new generation.
Reattach earlier work with its retained exact Eon or EonTerm executable; the
new installed client can inspect and stop that work through its owner.
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
the CLI mid-batch. Fixed-namespace legacy work has no authoritative Stop action
and is reported as unavailable.

## Workspace and picker

A **workspace** is the whole collection of tabs and panes. A **tab** groups
panes around its launch directory. A **pane** is a view of a persistent
**terminal**, whose process, PTY and terminal state outlive the window.

| Shortcut | Action |
|---|---|
| Alt+/ | Open or close shortcut help |
| Alt+Shift+B | Show or hide the bottom shortcut hints |
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+H / Alt+L | Select the previous or next tab |
| Alt+K / Alt+J | Select the previous or next pane |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the active tab |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane |
| Alt+Shift+T | Create a tab through the directory picker |
| Alt+M | Create a pane in the active tab |
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
windows omit whole hints. Compact groups spread across the outlined bar with
equal gaps and the outer groups near its edges; a lone group stays centered. Hint spacing stays compact.
Only modifier headers have padded rounded backgrounds. The main region and
footer share side margins and one outer frame with a straight separator,
matching lower corner radii and the main frame's bottom inset. Hiding the footer
returns the row and its bottom gap to terminal content.
Coral red modifier badges use plain aliases, such as A-S, before mint keys and gray
labels. The compact tab range is 1..9; Alt+0 still selects the tenth tab.
Copy/Paste and Open link stay in full help, without footer hints.
Each group uses C for Ctrl, A for Alt, and S for Shift, joined by hyphens.
H/L moves tabs and K/J moves panes in the combined move hint. Full help and
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
you are. It appears in Eon's pane header or at the top right of EonTerm, updates
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
| `eon windows [--json]` | List independent windows and their generations |
| `eon window attach ID` | Present one window's current generation after detach |
| `eon window stop ID [--json]` | Stop only the selected window's terminals |
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
| `eon pane create [--json]` | Create a pane in the active tab |
| `eon pane move up\|down [--json]` | Move the selected pane one position |
| `eon focus ID\|left\|right\|up\|down [--json]` | Focus a stable identity or traverse the workspace |
| `eon versions` | Print Eon, protocol, and component identities |
| `eon config-path` | Create and print the configuration root |

Workspace commands target the current-generation supervisor. Human and `--json`
output describe the same result. A missing supervisor is not started implicitly;
run `eon` first.
Eon calls Orbit-backed instances terminals in human output.
JSON retains the technical `session` / `sessions` fields and `session-N` IDs;
these refer to terminals.
Commands run inside an independent window target that window; commands outside
one target the default workspace. `eon stop all` covers generations in that
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

## EonTerm

Install EonTerm when one exact command needs a native terminal surface without
Eon's workspace or managed environment:

```sh
nix profile add .#eonterm
eonterm -- COMMAND...
eonterm --no-decorations -- COMMAND...
```

Its lifecycle commands mirror Eon's:

```sh
eonterm generations
eonterm attach [GENERATION]
eonterm stop GENERATION
```

Eon and EonTerm use separate runtime namespaces. EonTerm keeps native window
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
