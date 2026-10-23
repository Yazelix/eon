# Configuration

`eon config-path` prints the configuration root, normally
`~/.config/eon`. Settings live in `config.toml`:

```toml
[bar]
widgets = ["codex_quota", "claude_quota", "cpu", "ram", "clock", "version"]

[terminal]
background_opacity = 0.80
background_blur = true
pane_frames = true
cursor_trail_color = "preset:ice"
cursor_trail_duration = 1.5
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
mise = true

[anima]
enabled = true
style = "random"
duration_seconds = 3

[workspace]
warning_close_last_tab = true

[sidebar]
enabled = true
```

`[bar] widgets` chooses which Eon Bar widgets show and their order, left to
right between the tabs and the Shortcuts and Close buttons: the Codex and
Claude quota chips, CPU and RAM use, the local clock and `EON <version>`. Each
name appears at most once. A widget left out doesn't show, `widgets = []`
leaves only the tabs and buttons, and omitting the key shows all six in the
order above. The list applies when a window next opens or reopens, and an
unknown or repeated name is a configuration error. Narrow windows hide the
clock first, then RAM, CPU and the version, then shorten the quota chips,
whatever the order; then quota chips disappear one by one, the one closest to
the tabs first. The version belongs to its running supervisor; updating
the installation does not change it.

Omitting `cursor_trail_color` chooses a random color that lasts as long as the
running workspace, so `eon attach` reopens with the same color; a new workspace
picks again. eonbase keeps one color across reopens, though its first window
may differ. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically. The cursor body
uses the chosen color with a shape-aware contrasting edge unless Orbit supplies
an explicit cursor color.

`cursor_trail_duration` is a finite multiplier from `0.25` through `4.0`,
defaulting to `1.5`. Larger values keep the tail visible longer. Eon owns this
setting for both products; no Venus or Orbit configuration is needed.

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. The default order is `Symbols Nerd Font Mono`,
then `Accelerando Terminal`, so alien glyphs avoid searching all installed fonts.
An explicit list replaces that order. The terminal package includes fonts for
ordinary text, Nerd Font icons and Accelerando; it does not install them
system-wide. Compositors may ignore blur or initial window sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each terminal. Invalid startup configuration creates no fresh terminal;
a failed replacement leaves existing terminals and commands alive.

On a fresh full-Eon workspace, Anima plays after the desktop is ready and before
the initial directory picker. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback;
`style` is passed to the pinned Anima executable, and `duration_seconds` accepts
1–30. Anima owns its style names; run `eon anima --help` to see them. A failed
animation continues to the picker. Attachment, recovery, later tabs, and
eonbase do not play the startup animation.

Closing the last tab quits its window and stops its terminals.
`warning_close_last_tab = true`, the default, makes `eon tab close` and
`eon pane close` ask first when run from a terminal, listing the terminals that
will stop; `--json` never asks. In the window, Alt+Shift+W, the bar's `×` and
Alt+W on the last pane show a prompt listing the same terminals: Enter quits,
Escape cancels, and Alt+Shift+D keeps them running instead. With `false`, both
close at once. The window reads the setting when it starts.

`[sidebar] enabled = true`, the default, offers the agent sidebar, which lists
the coding agents in the window's tabs that report their status. `false` turns
it off. The window reads the setting when it starts. The sidebar itself is not
drawn yet.

## Agent quota

Eon bundles pinned CodexBar CLI for Codex and Claude quota.
Codex uses your authenticated `codex` executable on `PATH`; Claude uses its local
OAuth credentials. Explicit sources avoid browser fallback and model prompts.
Eon does not change provider settings or manage accounts. CodexBar owns credential
selection and its local authentication/cache mechanisms.

Codex shows its available five-hour and seven-day windows in one chip; Claude
shows one chip per available window. Only reported windows appear; an idle
Claude window can lack a reset time. Hover and accessible labels report that
absence. Narrow windows may condense Codex's chip to its lower remaining
percentage, then hide quota chips one by one, the one closest to the tabs
first, before tabs and controls.

Eon reads a provider's quota only while `[bar] widgets` lists its chip, checked
when a window opens or reopens. A chip listed again shows its provider's
stored reading at once, if there is one, and otherwise appears after its
provider's next read.

Codex refreshes at least one minute apart and Claude at least five minutes
apart, also after a failed read or a chip shown again. A failed read keeps the last good values; they become `old` once they are more than an hour old and
disappear after 24 hours. Each supplied reset expires its own window. Missing
facts do not mean zero usage. Sign-out is not detected; the last reading ages
like any other and disappears after 24 hours.

Each provider's last good reading is also stored in `$XDG_STATE_HOME/eon/quota`
(`~/.local/state/eon/quota` by default), shared by all your Eon windows, so a
newly started window shows it until its own read succeeds. The stored reading
holds only windows, remaining percentages, resets and when they were read.

Claude needs no status-line setup.

Existing Eon windows retain their running build after installation updates;
start a fresh workspace with the updated command to use this backend.

## Popups

Popup geometry and entries use the same file:

```toml
[popup]
side_margin = 8
vertical_margin = 4

[popups.monitor]
command = ["btop"]
keybinding = "Alt+Shift+O"
label = "Monitor"
keep_alive = true
```

Git defaults to `eon-lazygit` on Alt+Shift+J. Agent defaults to the first
available command among Codex, Grok, OpenCode, Pi, and Claude on Alt+Shift+L.
Files opens Yazi with Eon's own popup keys on Alt+Shift+Y and reads no Yazi
configuration of yours. Setting `[popups.files].command` replaces it, for
example with `["eon-yazi"]`, which reads the `yazi` directory under Eon's
configuration root.
Anima defaults to Alt+Shift+A, opens a random animation, and stops when dismissed
or replaced. Set `[popups.anima].enabled = false` to hide that shortcut; its
command and transient lifetime are fixed.
Commands defaults to Alt+Shift+M and searches and runs Eon commands; a custom
popup on the same key collides with it until one of them moves.
Set `enabled = false` on Git, Agent, Files, Commands, or a custom popup to disable
it. Project is required on Alt+Z. A popup shortcut on the key of a built-in
Eon shortcut takes that key, and Alt+/ lists the built-in one as overridden.
Alt+/ and Alt+0 through Alt+9 are reserved and cannot be bound.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

## Managed tools

A terminal without an explicit command starts pinned Nushell. Select managed
Bash with `eon-bash` in `[shell].command`. Any other shell, such as a host
`zsh` or `fish`, runs as an unmanaged command without Eon's integrations.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations. Last, if your PATH has
[mise](https://mise.jdx.dev), managed Nushell and Bash activate it the way
`mise activate` does in your own shell, so a project's mise tools and
environment apply from the first prompt. Eon skips it when mise is missing,
fails, or your configuration already activates it, and `mise = false` turns
it off. Eon never installs mise or trusts its configuration for you.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` and default Eon terminals also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-hx`, `eon-yazi`, `eon-ya`, `eon-lazygit`, and
`eon-lg`. Inside terminals their unprefixed names resolve to the pinned tools.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.
