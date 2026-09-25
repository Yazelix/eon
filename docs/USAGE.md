# Using Eon

## Supported environment

Eon is a Nix-only alpha. The supported product target is **x86_64 Linux on
native Wayland**.

Before installing, you need:

- Git and Nix with flakes enabled; and
- a native Wayland session.

The accepted runtime proof uses COSMIC with systemd, and installed checks also
exercise isolated Sway. Eon does not require systemd or cgroup delegation, but
non-systemd use remains unproved. The current graphics proof uses Mesa on Intel
hardware; proprietary NVIDIA remains unproved.

Apple Silicon macOS is the active expansion. Orbit has native proof, and Venus
has partial M1 proof, including physical Scaleway tests. Venus acceptance and
full Eon Nix composition remain open, so this release has no Mac install path.
X11, Xwayland, ARM Linux, Intel macOS, direct bundles, Home Manager installation,
background updates, and release automation are unsupported. Restarting the
machine ends live Session process state.

## Install and start

Install the public alpha:

```sh
nix profile add --refresh github:Yazelix/eon/linux-alpha-2026-09-24
```

Run `eon` to start. `eon versions` shows the installed component revisions.

Anima startup, its popup and CLI command, and the independent-window shortcut,
launcher action, and `eon window` commands below require the current `edge`
build; the tagged alpha does not include them.

Fresh workspaces first show a three-second random Anima animation after the
window is ready. Press any key other than the style-browsing keys to dismiss it;
set `[anima].enabled = false` in `config.toml` to skip it. The directory picker
then opens. Press Enter to use a Zoxide history
match, or Tab to browse folders with Yazi. In the browser, Enter chooses the
highlighted folder and F1 shows its keys. Cancelling a pending picker starts a
Session in that tab's directory if it is the only tab; otherwise it removes
just that pending tab.

A Session starts after the directory is chosen. Closing the Eon window detaches
the desktop without stopping its Sessions or PTY commands. Reconnect with:

```sh
eon attach
```

Press **Alt+/** inside Eon to open the native Shortcuts dialog. It includes the
fixed workspace bindings and every enabled popup binding. Escape or Alt+/ closes
it.

### Stop Sessions

To inspect or stop work explicitly:

```sh
eon generations
eon stop GENERATION
eon stop previous
eon stop all
```

`eon stop` shows the generation's live Session identities and asks for
confirmation. Closing a window is therefore detach; stopping a generation ends
its Sessions. Previous generations using EONW v2 through v7 can be stopped
through their own supervisor even when the current desktop cannot attach to them.
`previous` attempts every non-dead generation except the installed build's exact
current generation, including when current has not started.
`all` includes current and stops it last. Each generation gets its own
confirmation. Declining one skips that generation; earlier Stops remain and
later prompts continue. A failed Stop does not prevent later attempts.
When the command completes, it exits nonzero if any attempt fails. `--json`
skips confirmation and returns an array of attempted results, or `[]` if there
is nothing to stop.

Run a batch command outside the generations it will stop when you need every
attempt or a complete result: stopping the caller's own Session can terminate
the CLI mid-batch. Fixed-namespace legacy work has no authoritative Stop action
and is reported as unavailable.

## Workspace and picker

| Shortcut | Action |
|---|---|
| Alt+/ | Open or close shortcut help |
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+H / Alt+L | Select the previous or next tab |
| Alt+K / Alt+J | Select the previous or next pane |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the active tab |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane |
| Alt+Shift+T | Create a tab through the directory picker |
| Alt+M | Create a pane in the active tab |
| Alt+Shift+W | Close the active non-final tab |
| Alt+Shift+N | Open an independent Eon window |
| Alt+Z | Open the tab's Project directory picker |
| Alt+Shift+J | Open or hide the tab's Git popup |
| Alt+Shift+L | Open or hide the tab's Agent popup |
| Alt+Shift+A | Open or dismiss the tab's Anima popup |

### Eon Bar and tabs

The top row is the native Eon Bar. Its fixed `+`, `?`, and `×` controls create
a tab, open Shortcuts, and close the active non-final tab. The empty space
between the scrollable tabs and those controls moves the undecorated window;
tabs, controls, pane headers, and terminal content remain ordinary targets.
When a compatible authenticated `codex` executable is available on `PATH`, the
bar also shows the monochrome OpenAI Blossom and its read-only Codex quota:
elapsed/total positions and remaining percentages, visibly old last-good data,
or an explicit blocked or unknown permission state. The chip disappears before
tabs or controls under width pressure, and provider absence leaves the bar
unchanged.

Tabs own launch directories; changing a shell's directory does not rename or
retarget its tab. Project changes the directory used by future Sessions in the
captured tab without moving existing Sessions. Git and Agent are tab-scoped
Sessions whose terminal state survives hiding and reopening. Anima is transient:
closing its popup stops playback and returns to the tab.

### Scrolling and directory picker

When you scroll above live output, `↓ N rows` shows how many display rows back
you are. It appears in Eon's pane header or at the top right of EonTerm, updates
as output continues, and disappears at live output. Click the whole pill to
return there.

The directory picker starts with Zoxide and fzf. Tab switches to Yazi, Shift+Z
jumps through Zoxide history from the browser, `g h` goes home, `g /` goes to
root, `g Space` accepts a folder path, and F1 opens browser help. Files are shown
for orientation but are never opened or managed by the picker.

## CLI commands

| Command | Result |
|---|---|
| `eon` | Present the current workspace or start its first directory picker |
| `eon anima [STYLE] [CHILD OPTIONS...]` | Run pinned Anima in the caller's terminal without starting a workspace; `--help` lists styles and options |
| `eon run [-- COMMAND...]` | Start a fresh workspace picker, optionally for an exact first command |
| `eon window new` | Open an independent Eon window and print its window ID |
| `eon windows [--json]` | List independent windows and their generations |
| `eon window attach ID` | Present one window's current generation after detach |
| `eon window stop ID [--json]` | Stop only the selected window's Sessions |
| `eon window stop all` | Attempt Stop in every independent window |
| `eon attach [GENERATION]` | Present the current or one selected compatible generation |
| `eon generations [--json]` | List validated current and older generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor |
| `eon stop previous [--json]` | Attempt every other non-dead generation |
| `eon stop all [--json]` | Attempt every non-dead generation, including current |
| `eon workspace [--json]` | Inspect the live tab, pane, popup, and Session mapping |
| `eon tab create [--json]` | Create a distinct pending tab with its directory picker, even while another picker is open |
| `eon tab close TAB [--json]` | Close the expected active non-final tab |
| `eon tab directory TAB [--json] -- DIRECTORY` | Set a tab's directory for future Sessions |
| `eon tab move left\|right [--json]` | Move the active tab one position |
| `eon pane create [--json]` | Create a pane in the active tab |
| `eon pane move up\|down [--json]` | Move the selected pane one position |
| `eon focus ID\|left\|right\|up\|down [--json]` | Focus a stable identity or traverse the workspace |
| `eon versions` | Print Eon, protocol, and component identities |
| `eon config-path` | Create and print the configuration root |

Workspace commands target the current-generation supervisor. Human and `--json`
output describe the same result. A missing supervisor is not started implicitly;
run `eon` first.
Commands run inside an independent window target that window; commands outside
one target the default workspace. `eon stop all` covers generations in that
current namespace. `eon window stop all` covers independent windows and visits
the caller's window last. Run it outside those Sessions when you need its final
status; stopping its own Session can terminate the CLI.

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

- **Nix cannot fetch a child input:** check access to GitHub and retry. The
  locked Eon Sessions and Eon Desktop source revisions are public.
- **A workspace command reports a missing supervisor:** run `eon`, then retry.
- **The window closed but commands remain:** this is detach behavior. Use
  `eon attach` to return or `eon stop GENERATION` to end them.
- **A replacement window rejects configuration:** fix `config.toml` and run
  `eon attach`; the existing Sessions remain alive.
- **Graphics fail outside native Wayland or on unproved hardware:** that
  environment is outside the supported alpha boundary.

Report reproducible failures in [GitHub Issues](https://github.com/Yazelix/eon/issues).
Include `eon versions`, the relevant generation from `eon generations`, your
distribution, compositor, GPU, and the exact command and error. Remove paths,
environment values, terminal content, and credentials that should remain
private.
