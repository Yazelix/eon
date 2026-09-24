# Configuration

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

[anima]
enabled = true
style = "random"
duration_seconds = 3
```

Omitting `cursor_trail_color` chooses a random color for each surface. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically. The cursor body
uses the chosen color with a shape-aware contrasting edge unless Orbit supplies
an explicit cursor color.

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. Eon installs no fonts and compositors may
ignore blur or initial window sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each Session. Invalid startup configuration creates no fresh Session;
a failed replacement leaves existing Sessions and commands alive.

On a fresh full-Eon workspace, Anima plays after the desktop is ready and before
the initial directory picker. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback;
`style` is passed to the pinned Anima executable, and `duration_seconds` accepts
1–30. Anima owns its style names; run `eon anima --help` to see them. A failed
animation continues to the picker. Attachment, recovery, later tabs, and
EonTerm do not play the startup animation.

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

A Session without an explicit command starts pinned Nushell. Select a different
managed shell with `eon-bash`, `eon-zsh`, or `eon-fish` in `[shell].command`.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` and default Eon Sessions also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-hx`, `eon-yazi`, `eon-ya`, `eon-lazygit`, and
`eon-lg`. Inside Sessions their unprefixed names resolve to the pinned tools.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.
