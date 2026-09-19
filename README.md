# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

Eon is a durable terminal workspace for the native Linux desktop. Tabs and
panes run in persistent Sessions: closing the window detaches the desktop while
the commands keep running, and opening Eon again reconnects to them.

The package also includes a pinned interactive environment with Nushell, Bash,
Zsh, Fish, Helix, Yazi, LazyGit, Starship, Zoxide, fzf, Atuin, and Carapace.
`eonterm` provides the same terminal and Session lifecycle for one exact command
without Eon's workspace or managed environment.

## Project status

Eon is a Nix-only alpha. The supported product target is **x86_64 Linux on
native Wayland**.

Before installing, you need:

- Nix with flakes enabled;
- a native Wayland session; and
- GitHub credentials that can read this repository's private Eon Sessions and
  Eon Desktop inputs.

The accepted runtime proof uses COSMIC with systemd, and installed checks also
exercise isolated Sway. Eon does not require systemd or cgroup delegation, but
non-systemd use remains unproved. The current graphics proof uses Mesa on Intel
hardware; proprietary NVIDIA remains unproved.

Apple Silicon macOS is the active expansion but remains unsupported until its
native proofs pass. X11, Xwayland, ARM Linux, Intel macOS, direct bundles, Home
Manager installation, background updates, and release automation are also
unsupported. Restarting the machine ends live Session process state.

## Install and start

From a clone whose credentials can fetch the private child inputs:

```sh
nix profile add .#default
eon versions
eon
```

The first launch opens a directory picker. Press Enter to use a Zoxide history
match, or Tab to browse folders with Yazi. In the browser, Enter chooses the
highlighted folder and F1 shows its keys. Cancelling the first picker starts in
the directory where Eon was launched.

A Session starts after the directory is chosen. Closing the Eon window detaches
the desktop without stopping its Sessions or PTY commands. Reconnect with:

```sh
eon attach
```

Press **Alt+/** inside Eon to open the native Shortcuts dialog. It includes the
fixed workspace bindings and every enabled popup binding. Escape or Alt+/ closes
it.

To inspect or stop work explicitly:

```sh
eon generations
eon stop GENERATION
```

`eon stop` shows the generation's live Session identities and asks for
confirmation. Closing a window is therefore detach; stopping a generation ends
its Sessions.

## Everyday use

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
| Alt+Z | Open the tab's Project directory picker |
| Alt+Shift+J | Open or hide the tab's Git popup |
| Alt+Shift+L | Open or hide the tab's Agent popup |

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
Sessions whose terminal state survives hiding and reopening.

When you scroll above live output, `↓ N rows` shows how many display rows back
you are. It appears in Eon's pane header or at the top right of EonTerm, updates
as output continues, and disappears at live output. Click the whole pill to
return there.

The directory picker starts with Zoxide and fzf. Tab switches to Yazi, Shift+Z
jumps through Zoxide history from the browser, `g h` goes home, `g /` goes to
root, `g Space` accepts a folder path, and F1 opens browser help. Files are shown
for orientation but are never opened or managed by the picker.

## Commands

| Command | Result |
|---|---|
| `eon` | Present the current workspace or start its first directory picker |
| `eon run [-- COMMAND...]` | Start a fresh workspace picker, optionally for an exact first command |
| `eon attach [GENERATION]` | Present the current or one selected compatible generation |
| `eon generations [--json]` | List validated current and older generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor |
| `eon workspace [--json]` | Inspect the live tab, pane, popup, and Session mapping |
| `eon tab create [--json]` | Create a pending tab with its directory picker |
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

## Configuration

`eon config-path` prints the configuration root, normally
`~/.config/eon`. Settings live in `config.toml`:

```toml
[terminal]
background_opacity = 0.80
background_blur = true
pane_frames = true
cursor_trail_color = "preset:ice"
font_family = "Iosevka"
font_size = 16
line_height = 1.125
columns = 100
rows = 30

[shell]
command = ["eon-nu"]
starship = true
zoxide = true
atuin = true
carapace = true
```

Omitting `cursor_trail_color` chooses a random color for each surface. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically.

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. Eon installs no fonts and compositors may
ignore blur or initial window sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each Session. Invalid startup configuration creates no fresh Session;
a failed replacement leaves existing Sessions and commands alive.

Popup geometry and entries use the same file:

```toml
[popup]
side_margin = 8
vertical_margin = 4

[popups.files]
command = ["eon-yazi"]
keybinding = "Alt+Shift+F"
label = "Files"
keep_alive = true
```

Git defaults to `eon-lazygit` on Alt+Shift+J. Agent defaults to the first
available command among Codex, Grok, OpenCode, Pi, and Claude on Alt+Shift+L.
Set `enabled = false` on Git, Agent, or a custom popup to disable it. Project is
required on Alt+Z. Popup shortcuts cannot collide with workspace shortcuts.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

## Managed tools

A Session without an explicit command starts pinned Nushell. Select a different
managed shell with `eon-bash`, `eon-zsh`, or `eon-fish` in `[shell].command`.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations.

The profile also exposes `eon-hx`, `eon-yazi`, `eon-ya`, `eon-lazygit`, and
`eon-lg`. Inside Sessions their unprefixed names resolve to the pinned tools.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.

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

- **Nix cannot fetch a child input:** the alpha still requires GitHub access to
  the private Eon Sessions and Eon Desktop repositories. Credential-free
  installation is separate work.
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

## Maintainers

Eon owns orchestration, product configuration, component selection, updates,
and distribution. Eon Sessions owns persistent terminal state and attachment;
Eon Desktop owns native presentation and input. Yazelix Nova remains a separate
product line.

The maintainer sources of truth are:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — naming, ownership, and sequencing;
- [`docs/CONTRACTS.md`](docs/CONTRACTS.md) — indexed product contracts and proofs;
- [`docs/DISTRIBUTION.md`](docs/DISTRIBUTION.md) — composition and release policy;
- [`docs/REFERENCES.md`](docs/REFERENCES.md) — routed primary and comparable sources;
- [`docs/VISUAL-REFERENCES.md`](docs/VISUAL-REFERENCES.md) — discovery-only visual references;
- [terminal memory benchmark](docs/benchmarks/eon-zlf-2026-09-08.md) — the measured
  Eon, EonTerm, Foot, and Ghostty comparison; and
- [`CHANGELOG.md`](CHANGELOG.md) — accepted user-visible chronology.

[`components/eon-alpha-v3.json`](components/eon-alpha-v3.json) is the canonical
distribution-neutral component graph. Validate it with:

```sh
cargo run --locked -p eon-manifest -- components/eon-alpha-v3.json
```

Use Beads for implementation plans and deferred decisions:

```sh
bv --robot-triage
br ready
br show <id>
```

## LOC scorecard

The scorecard counts tracked handwritten text and code. It excludes `.git/`,
Beads data, lock files, and generated artifacts.

| Surface | Lines |
|---|---:|
| Agent policy inputs | 269 |
| README | 313 |
| Repository ignore rules | 3 |
| License | 201 |
| Architecture and contracts | 1,973 |
| Distribution and references | 720 |
| Benchmark report | 317 |
| Changelog | 326 |
| Rust source and tests | 15,996 |
| Cargo manifests | 38 |
| Component manifest | 355 |
| Nix composition | 790 |
| Product defaults | 0 |
| **Total** | **21,301** |
