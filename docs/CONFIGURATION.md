# Configuration

`eon config-path` prints the configuration root, normally
`~/.config/eon`. Settings live in `config.toml`:

```toml
[bar]
show_version = true

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

[anima]
enabled = true
style = "random"
duration_seconds = 3
```

The full-Eon top bar shows `EON <version>` by default. Set
`[bar] show_version = false` to hide the complete label when a window next
opens or reopens. The version belongs to its running supervisor; updating the
installation does not change it. Narrow windows hide the label automatically.

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

## Agent quota

Eon bundles pinned CodexBar CLI for Codex and Claude quota.
Codex uses your authenticated `codex` executable on `PATH`; Claude uses its local
OAuth credentials. Explicit sources avoid browser fallback and model prompts.
Eon does not change provider settings or manage accounts. CodexBar owns credential
selection and its local authentication/cache mechanisms.

Both available five-hour and seven-day windows appear by default. Only reported
windows appear; an idle Claude window can lack a reset time. Hover and accessible
labels report that absence. Narrow windows may condense to the lower remaining
percentage or hide optional chips before tabs and controls.

Codex refreshes at least one minute apart and Claude at least five minutes
apart, also after a failed read. A failed read keeps the last good values; they become `old` once they are more than an hour old and
disappear after 24 hours. Each supplied reset expires its own window. Missing
facts do not mean zero usage. Sign-out is not detected; the last reading ages
like any other and disappears after 24 hours.

Claude needs no status-line setup. Existing `eon claude-statusline` calls remain
silent and bounded for wrappers that also run another renderer, and can be
removed while preserving that renderer's exact command. Eon does not edit those
settings or read the previous quota cache.

Existing Eon windows retain their running build after installation updates;
start a fresh workspace with the updated command to use this backend.

## Popups

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
Anima defaults to Alt+Shift+A, opens a random animation, and stops when dismissed
or replaced. Set `[popups.anima].enabled = false` to hide that shortcut; its
command and transient lifetime are fixed.
Set `enabled = false` on Git, Agent, or a custom popup to disable it. Project is
required on Alt+Z. Popup shortcuts cannot collide with workspace shortcuts.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

## Managed tools

A terminal without an explicit command starts pinned Nushell. Select managed
Bash with `eon-bash` in `[shell].command`. Any other shell, such as a host
`zsh` or `fish`, runs as an unmanaged command without Eon's integrations.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` and default Eon terminals also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-hx`, `eon-yazi`, `eon-ya`, `eon-lazygit`, and
`eon-lg`. Inside terminals their unprefixed names resolve to the pinned tools.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.
