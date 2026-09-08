# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

This repository ships **Eon**, the full managed product, and **EonTerm**, its
reusable terminal product. Both use Eon Sessions and Eon Desktop through one
canonical component graph without reimplementing either child project.

## Project status

This repository ships the first Nix-only Eon alpha for x86_64 Linux on native
Wayland. One Rust supervisor launches the accepted Eon Sessions and Eon Desktop
revisions. The `eon` package owns a live workspace and supplies one pinned
interactive environment. The slimmer `eonterm` package owns one exact-command
Session and contains no managed shell or tool bundle. Both expose lifecycle
control through EONW v4, isolate live runtime generations, and consume the same
component identities. Direct bundles, Home Manager, background updates, and
release automation remain outside this slice. X11, Xwayland, and macOS are
unsupported. The accepted runtime proof uses COSMIC with systemd. Eon targets
native Wayland without requiring a specific init or service manager, but
non-systemd use remains unproved. Launch requires no cgroup delegation. Orbit
owns bounded PTY process-group shutdown and direct-child reaping; deliberately
detached processes may survive an explicit Session stop.

## Naming model

**Eon** is the full public product and `eon` is its command. **EonTerm** is the
reusable terminal product and `eonterm` is its command. **Sessions** is the
user-facing name for durable terminal work. Venus and Orbit identify the
underlying engineering subsystems, not additional products.

| Repository | Product-facing role | Underlying owner |
|---|---|---|
| [`Yazelix/eon`](https://github.com/Yazelix/eon) | Eon orchestration, composition, updates, and distribution | Eon orchestrator |
| [`Yazelix/eon-desktop`](https://github.com/Yazelix/eon-desktop) | Eon for desktop | Venus client subsystem |
| [`Yazelix/eon-sessions`](https://github.com/Yazelix/eon-sessions) | Sessions | Orbit session subsystem |

If more clients are implemented, their repositories are `eon-mobile` and
`eon-web`, while public documentation calls them **Eon for mobile** and **Eon
for the web**. Those names create no implementation scope by themselves.

## Target composition

```text
Eon / product orchestrator
        +---- Eon Desktop / Venus client subsystem
        +---- Eon Sessions / Orbit session subsystem
        +---- Nushell + Bash + Zsh + Fish
        |       +---- Starship + Zoxide + fzf + Atuin + Carapace
        +---- Helix
        +---- Yazi
        +---- LazyGit
        +---- Ratconfig
```

Each child repository keeps one clear responsibility. Eon selects compatible
versions, supplies product configuration, launches the composition, and ships
the Eon and EonTerm packages.

Yazelix Nova remains a separate product. Nova provides a compact Yazelix runtime
on its own architecture and keeps value independent of Eon's progress.

## Design commitments

- Eon Sessions, Eon Desktop, and Eon advance through one active implementation frontier.
- Indexed contracts define cross-repository behavior.
- Exact child revisions and explicit component inputs form the composition boundary.
- Nix provides the sole alpha and early-dogfood installation path.
- Eon runtime code remains independent of Nix concepts and evaluation.
- Direct bundles wait for sustained dogfood and an explicit graduation decision.
- Linux on native Wayland is the sole platform target, without a required init
  or service manager.
- Eon owns product policy and avoids copying child behavior.

## Implementation language

Rust owns durable Eon behavior and repository tooling so schema, validation,
tests, and later runtime code share one compiler and maintenance path. Nix owns
package resolution and composition, and shell is reserved for irreducible
process glue. Python remains suitable for disposable investigation, not checked-
in product policy. The directory picker uses a small Lua status hook inside
Yazi's existing runtime for its shortcut footer; it adds no separate toolchain.
Other durable language additions require an explicit subsystem decision.

## Install and run

The alpha builds from the locked flake and its exact private Eon child inputs.
Clone the repository with GitHub credentials that can read those inputs, then
install the package into your Nix profile:

```sh
nix profile add .#default
eon versions
eon
```

Install only the reusable terminal product when an exact command needs one
native terminal surface without Eon's managed environment:

```sh
nix profile add .#eonterm
eonterm -- COMMAND...
```

EonTerm keeps its own runtime generations. List them with
`eonterm generations`, reopen one with `eonterm attach [GENERATION]`, and stop
one through its supervisor with `eonterm stop GENERATION`.

The full Eon package installs one `Open Eon` desktop action, bound to its exact
packaged executable, and a transparent violet three-fold loop icon at native
launcher sizes. Running surfaces keep their terminal-authored titles,
distinguishing the launcher action from live Eon state.
Opening Eon reconnects only to the exact installed runtime generation or starts
that generation in its own private namespace. If that generation's supervisor
was lost on the same boot, Eon validates and adopts its exact surviving Orbit
runs before publishing a replacement workspace or Venus surface. Older live
generations and their Sessions remain running. An exact retained record claim
separates pre-Ready local-child rollback from post-Ready Orbit management;
rejected Ready identities are stopped only through that management owner.
Concurrent starts converge on one supervisor and one complete Session set;
every attach-capable peer presents that owner. If a launch overlaps clean exit
of the last Session, it waits for the retiring owner and starts one fresh
Session. Eon validates Orbit's Ready identity and acquires its management lease
within the existing startup deadline. Orbit owns PTY startup and bounded
shutdown without a cgroup or service-manager requirement. Its Nix closure
supplies Mesa's open-source Vulkan drivers; the current graphics proof uses
Intel hardware, while proprietary NVIDIA remains unproved. Run Eon from a
terminal when you need foreground lifecycle control.
Closing the Eon Desktop window detaches the client while Sessions and its PTY
keep running. Ask the same supervisor to present them again with:

```sh
eon attach
```

`eon generations` lists the current, previous, and fixed-namespace legacy
workspaces after validating their live supervisors. `eon attach GENERATION`
selects one compatible generation without fallback. `eon stop GENERATION`
shows its live Session identities and asks for confirmation; `--json` is the
explicit non-interactive form. Restarting the machine preserves no process
state beyond the accepted child contracts.

EonTerm gives one command the terminal key stream without Eon's tab and pane
shortcuts:

```sh
eonterm -- COMMAND...
eonterm --no-decorations -- COMMAND...
eonterm --application-id eonova -- COMMAND...
```

This mode hosts exactly one Orbit Session, gives Venus only the Orbit endpoint,
and keeps Eon's generation, presentation, stop, child-exit, and cleanup lifecycle.
After same-boot supervisor loss, a replacement EonTerm adopts that exact live
Session rather than launching another one. If both the supervisor and recorded
Orbit process die unexpectedly, the next launch removes only their exact owned
runtime residue and starts a fresh Session.
EonTerm uses native window decorations unless `--no-decorations` is selected.
It supplies the distinct native application identity `eonterm`; an approved
composition may select one bounded desktop identity with `--application-id`.
Full Eon remains `eon`, and terminal-authored titles remain independent.
The supervisor preserves its original choice when it replaces a detached
surface. A repeated invocation preserves the active surface and asks Venus to
request native presentation. After that surface closes, another invocation asks
the supervisor to open one replacement against the same Session. Workspace
actions are unavailable. Eon and EonTerm use separate default runtime namespaces.
Terminal programs can request bounded plain-text clipboard writes through Orbit;
Venus delivers them to the requested ordinary or primary Linux clipboard.
`Ctrl+Shift+V` and the native Paste key read the ordinary clipboard and send one
semantic paste to Orbit, which alone applies terminal paste encoding.
New Eon and EonTerm Sessions use Eon's vivid 16-color ANSI palette. Programs
retain normal OSC override and reset behavior; direct RGB and palette indices
16 through 255 remain unchanged. Existing live Sessions keep their current
palette until they are started in the refreshed runtime generation.

While the foreground supervisor is running, Eon owns horizontal tab order and
one vertical pane selection per durable tab; the active picker-bound pending tab
has no pane selection. Each pane starts and maps to a distinct Orbit session;
changing focus never stops a session. The CLI reaches that owner
through the private local Eon socket using EONW v4. Each accepted action returns
one complete ordered workspace snapshot; incompatible or malformed requests
receive a bounded structured failure. Additive EONW lifecycle actions report a
supervisor's generation, component graph, live Sessions, idempotent presentation,
and stop result through a separate result type that Eon Desktop never receives.
Eon Desktop consumes the workspace result and shows every fitting pane header
around one selected live Session. New generations identify tabs as `t1`, `t2`,
and so on. A tab header shows the numeric part, two spaces, then the leaf of
Eon's authoritative launch directory, `~` for the exact home directory, or `/`
for root; hit testing and actions retain the complete `tN` identity. Panes remain
`p1`, `p2`, and so on. Each visible live pane header shows that exact identity,
two spaces, then a `~/`-anchored path below home, an absolute path elsewhere, or
Nova's marker for the exact home directory. Unset or empty `HOME` leaves paths
absolute. Overlong labels preserve their rightmost components.
New current-generation full Eon windows omit the redundant native title bar;
the selected terminal retains compositor title semantics, while EonTerm and
legacy Eon attachment keep native decorations. Alt+H/L traverses tabs,
Alt+K/J traverses panes, Ctrl+Alt+H/L moves the active tab, Ctrl+Alt+K/J moves
the selected pane, Alt+Shift+W closes the active non-final tab, Alt+M creates a
pane, Alt+Shift+T opens a new tab's directory picker, and Alt+Z opens the active
tab's directory picker. External workspace changes appear
within one second because Eon Desktop re-inspects EONW v4 every 250 ms; the
protocol adds no event stream. When a shell exits, Eon removes its pane, selects
the nearest surviving pane, removes an empty tab, and closes when the final pane
exits.
Movement stops at ordered edges without changing stable identities or Session
mappings. `eon tab close tN` and Alt+Shift+W name the expected active tab; they
never close the final tab or silently advance a stale request to another tab.

Every live tab owns one absolute launch directory. A fresh workspace validates
Eon's launch directory, then opens `t1` with the ranked directory picker before
starting a durable Session. A new tab inherits the active tab's directory as its
picker starting point. Accepting starts that tab's first `pN` Session in the
chosen directory. Cancelling fresh `t1` starts `p1` in the validated launch
directory; cancelling a later pending tab removes it and restores the previous
focus. Existing-generation recovery creates no automatic picker.
`eon tab directory tN -- DIRECTORY` explicitly retargets only future Sessions
in that tab. Existing Sessions and shell working directories do not change.
Invalid targets or directories change no state; if an accepted path later
disappears, Session creation fails without adding a pane and the tab retains its
accepted value.

Alt+Z works from anywhere in a full-Eon tab. It keeps the tab bar visible and
replaces the tab body with a one-cell-inset ranked-directory picker backed by
the packaged Zoxide and fzf, independent of ambient fzf default options.
Enter opens a history match; Esc switches to Yazi to browse folders, including
those absent from history. Ctrl+C cancels. Empty history still offers Esc to
browse. Arrows and Tab/Shift+Tab move through quick-search results.

In the folder browser, arrows or hjkl navigate; Enter uses the **current folder**.
Shift+Z searches Zoxide history and moves the browser without committing the
tab, so you can jump to a known parent and walk the remaining directories.
`g h` goes home, `g /` goes to root, `g Space` accepts a folder path, `.` toggles
hidden entries, and F1 lists the keys. q, Q, or Ctrl+C cancel; Esc clears a
filter or returns from nested search. Files are visible for orientation; the
picker does not open, edit, or manage them. Browsing does not add Zoxide entries.
Yazi uses a packaged picker keymap independent of your regular Yazi configuration.
Each screen keeps its own actions visible at the bottom: quick search shows
Use directory, Browse folders, and Cancel; Yazi shows Use current folder,
Shift+Z Jump, Cancel, and F1 Help. Nested jump search shows Jump and Back to folders.
Browser hints hide while a Yazi prompt or overlay has focus.

Accepting a valid path retargets that captured tab for future Sessions; cancel
or failure changes nothing, and an actionable error stays visible briefly before cleanup.
The picker is one transient Orbit Session rather than a normal pane, and it
closes with its tab, Eon Desktop surface, process, or supervisor. Existing
Sessions and working directories remain untouched. Alt+H/L continues to
traverse and wrap live tabs while the picker stays bound to its original tab;
returning shows the same picker for normal acceptance or cancellation.
Alt+Shift+W or the CLI can discard the active non-final pending tab after its
picker stops. Other tabs allow pane creation, pane focus, pane/tab movement,
directory updates, and closing, including tab and pane header clicks. The
picker-bound tab stays modal, and opening another picker or new tab requires
finishing the current picker.

Workspace topology is live-only. After same-boot supervisor loss, full Eon
projects surviving canonical `session-N` runs into one synthetic `t1` as `pN`
in numeric order and selects the lowest number. That tab receives the replacement
supervisor's launch directory as fresh future-launch policy; Eon does not
reconstruct prior tabs, focus, launch directories, commands, or history. The
exact Orbit processes, PTY children, and terminal state remain Orbit-owned and
unchanged.

The command surface is small:

| Command | Result |
|---|---|
| `eon` | Present the exact current-generation workspace, or open its first directory picker |
| `eon run` | Explicitly open a fresh workspace picker; acceptance starts the default shell |
| `eon run -- COMMAND...` | Open a fresh workspace picker; acceptance starts the command as the first Orbit-owned PTY child |
| `eonterm [--no-decorations] [--application-id ID] -- COMMAND...` | Start or present one exact command in a native terminal surface without Eon workspace or managed-environment policy |
| `eonterm attach [GENERATION]` | Present the current or one selected compatible EonTerm generation |
| `eonterm generations [--json]` | List validated current and older EonTerm generations |
| `eonterm stop GENERATION [--json]` | Stop one EonTerm generation through its supervisor; human mode confirms first |
| `eon attach` | Present Eon Desktop against the exact current-generation launch mode |
| `eon attach GENERATION` | Present one explicitly selected compatible generation, including `legacy` |
| `eon generations [--json]` | List validated current, previous, legacy, dead, incompatible, unreachable, and corrupt generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor; human mode confirms first |
| `eon workspace [--json]` | Inspect the live Eon-owned tab, pane, and Session mapping |
| `eon tab create [--json]` | Create and focus a pending tab with its directory picker |
| `eon tab close TAB [--json]` | Close the expected active non-final `tN` and its Sessions |
| `eon tab directory TAB [--json] -- DIRECTORY` | Set one live `tN` tab's absolute launch directory for future Sessions |
| `eon tab move left\|right [--json]` | Move the active tab by one position without wrapping |
| `eon pane create [--json]` | Create and select a default-shell Session in the active tab |
| `eon pane move up\|down [--json]` | Move the selected pane by one position without wrapping |
| `eon focus ID [--json]` | Focus a stable tab or pane identity |
| `eon focus left\|right\|up\|down [--json]` | Traverse tabs or panes directly, wrapping at multi-target edges |
| `eon versions` | Print the runtime generation, EONW version, and stable component identities |
| `eon config-path` | Create and print the Eon configuration root |

Workspace commands target the exact current-generation Eon supervisor. An
EonTerm supervisor returns `workspace-unavailable` to topology actions at the
EONW boundary. EONW v4 frames are bounded to 2 MiB. The workspace topology is
bounded to 64 tabs and 256 panes, is not persisted, and has no per-pane or
per-Session removal action. Whole-generation stop sends canonical management
Stop to every validated Session lease and succeeds only after every exact
terminal record is complete. `--json` reports the same
accepted EONW result as the human view; neither output format is the protocol
schema. Its tab `directory`, pane `endpoint`, and optional picker `endpoint`
fields are ordered integer arrays that preserve every opaque Unix path or
endpoint byte.

## Terminal presentation

Scrolling, selection, and tab or pane focus remain usable while terminal output
continues between repaints. Orbit keeps parsing output and answering terminal
queries while the user reads anchored scrollback. Selection supports cell,
word, and logical-line gestures; completed copied text stays frozen. Reading
does not pause the program or preserve history beyond its existing budget.

Eon and EonTerm use the same terminal presentation settings in
`$EON_CONFIG_HOME/config.toml`, normally `~/.config/eon/config.toml`:

```toml
[terminal]
background_opacity = 0.80
background_blur = true
```

`background_opacity` accepts a finite number from `0.0` through `1.0` and
defaults to `0.80`. `background_blur` accepts a boolean, defaults to `true`,
and requests full-surface compositor blur; set it to `false` to omit that
request. Eon applies terminal settings from one snapshot when creating a Venus
surface. Editing the file does not change a live surface; after Venus exits,
`eon attach` or `eonterm attach` reads the current values for its replacement
without restarting the live Orbit Session or PTY child.

Optional typography and initial geometry fields belong in the same section:

| Field | Accepted value | When omitted |
|---|---|---|
| `font_family` | Installed monospace family name | Venus font selection |
| `font_fallbacks` | Ordered array of at most eight installed family names | No named fallback override |
| `font_size` | Finite number, 6–96 logical pixels | 16 |
| `line_height` | Finite multiplier, 1–3 | 1.125 |
| `columns` | Integer, 1–65,535 | Initial window width remains 960 logical pixels |
| `rows` | Integer, 1–65,535 | Initial window height remains 600 logical pixels |

Family names must be nonempty, trimmed, at most 128 UTF-8 bytes, and contain no
control characters. A supplied columns/rows pair must fit 100,000 cells. For
example, `font_size = 20`, `line_height = 1.5`, `columns = 100`, and `rows = 30`
request a 100 by 30 terminal with workspace chrome included. The compositor
may override initial sizing; later resizing remains unrestricted by these fields.

With a typography or geometry override, Venus admits fonts and the actual
window's initial native geometry before Eon starts a new Session or command.
Missing families and impossible geometry fail startup; Venus reports the
specific cause in the supervisor output. Invalid configuration or a failed
replacement leaves existing Sessions and their commands alive. A live Venus
keeps its settings until reopened. Eon installs no fonts and does not promise
that a selected family covers every glyph.

Cursor presentation remains Venus-owned. Eon passes no cursor-effect profile,
so every new or replacement surface uses Venus's blue cursor tail with its
default timing.

Eonova provides no opacity override, so an Eonova release that pins this
EON-C13 revision consumes the same `0.80` default from EonTerm.

Opacity applies only to the terminal default background and padding. Explicit
cell backgrounds, text, cursor, selection, Eon workspace chrome, native
decorations, input, hit testing, and accessibility keep their existing
semantics. Opacity and blur are independent: Eon never changes one because of
the other, and an opaque background can visually hide compositor blur.
Unsupported or policy-disabled Wayland compositors may ignore the best-effort
request. Transparency does not enable click-through.

## Managed environment

A Session without an explicit command starts Eon's pinned Nushell. Configure
the command and Eon-provided shell integrations in
`$EON_CONFIG_HOME/config.toml`, normally `~/.config/eon/config.toml`:

```toml
[shell]
command = ["eon-nu"]
starship = true
zoxide = true
atuin = true
carapace = true
```

Eon reads these settings for each new Session. `command` is a direct argv array;
`["eon-nu"]`, `["eon-bash"]`, `["eon-zsh"]`, and `["eon-fish"]` select the
managed shells. Their unprefixed aliases do the same from Eon's private PATH;
any other command remains unmanaged. Each boolean defaults to `true`; `false`
disables Eon's activation without disabling a user-owned setup.

Managed shells load native user configuration before Eon's integrations:

| Shell | Native configuration |
|---|---|
| Nushell | `$XDG_CONFIG_HOME/nushell/env.nu`, `config.nu`, then Eon's vendor file, then native `autoload/*.nu` |
| Bash | `~/.bashrc` |
| Zsh | `${ZDOTDIR:-$HOME}/.zshenv` and `.zshrc` |
| Fish | `$XDG_CONFIG_HOME/fish/config.fish` |

Eon preserves an existing prompt, external completer, or same-tool hook. It
supplies Starship, Zoxide, Atuin, and Carapace only where the native config has
left room for them. Managed Nushell suppresses its stock startup banner while
keeping native first-run config creation. `ATUIN_NOBIND` disables Atuin's key
bindings without disabling history integration. Starship reads normal
`~/.config/starship.toml`; Eon leaves `STARSHIP_CONFIG` unset. The other tools
keep their native configuration, data, and cache paths.

The package exposes managed tools outside Eon through these names:

| Command | Managed tool |
|---|---|
| `eon-nu` | Nushell |
| `eon-bash` | Bash |
| `eon-zsh` | Zsh |
| `eon-fish` | Fish |
| `eon-hx` | Helix |
| `eon-yazi` | Yazi |
| `eon-ya` | Yazi companion CLI |
| `eon-lazygit`, `eon-lg` | LazyGit |

Outside a Session, prefixed non-shell commands inherit the ambient PATH.
Managed shell launchers and Sessions prepend one process-local private PATH
that resolves the four shells, four integrations, `fzf`, `hx`, `yazi`, `ya`, and
`lazygit` to pinned artifacts. Eon does not alter the parent process's PATH,
aliases, or shell startup files. At launch, Eon ignores
ambient Helix runtime and Steel configuration paths and Yazi or LazyGit
configuration paths that would bypass its private root; explicit Helix and
LazyGit configuration arguments remain available. LazyGit uses the Git
executable available from the user's environment. The package supplies Symbols
Nerd Font Mono for Yazi's supported private-use icons without relying on an
ambient font installation. Other unsupported symbols may render as fallback
boxes.

`EON_CONFIG_HOME` selects the configuration root; Eon resolves a relative value
once against the launch directory. Without it, Eon uses `$XDG_CONFIG_HOME/eon`
or `$HOME/.config/eon`. `EON_RUNTIME_DIR` selects the socket directory. Eon
otherwise uses `$XDG_RUNTIME_DIR/eon`, while EonTerm uses
`$XDG_RUNTIME_DIR/eonterm`; each falls back to a separate private per-user
temporary directory.
Eon passes the absolute root as `XDG_CONFIG_HOME` to Venus and managed tools
other than shells. Sessions and managed shells inherit ambient XDG
configuration; Eon preserves `EON_CONFIG_HOME` through Sessions and managed
dispatch. Eon ignores relative XDG base paths. Each opaque `g1-…` identity is a
deterministic digest of Eon's runtime source, dependency lock, EONW source, and
canonical component manifest. Runtime endpoints live below
`$EON_RUNTIME_DIR/generations/<GENERATION>/`; a pre-generation supervisor remains
discoverable as `legacy` at the root. Eon creates missing configuration and
runtime directories with mode `0700`, leaves existing configuration-directory
permissions unchanged, and rejects unsafe directories or endpoints without
changing them.

## Hyperlinks

Explicit OSC 8 hyperlinks are available in Eon and EonTerm. Hover previews the
actual target; Ctrl+Shift+left click opens it. Ctrl+Shift+O enters inspection:
Tab/Shift+Tab chooses a link, Left/Right pages its target, Enter opens,
Ctrl+Shift+C copies, and Escape returns to typing. Ordinary URL-looking text
and terminal mouse/selection behavior retain their existing meaning.

Opening accepts ASCII HTTP/HTTPS targets up to 4096 bytes, without credentials,
and requires the desktop host's `gio` command and registered handler. Copy also
supports other schemes, within the same size limit and without control
characters. Venus inherits host XDG configuration for desktop preferences;
`EON_CONFIG_HOME` remains the product configuration root.

## Component manifest

[`components/eon-alpha-v3.json`](components/eon-alpha-v3.json) is the one
distribution-neutral source of component identity, compatibility, and abstract
artifacts. Its activation list distinguishes required alpha contracts from
additional recorded proof such as `ORB-C10`. Nix owns physical package paths,
Rust owns launch policy, and the graph contains neither resolved store paths nor
duplicated launch policy.

Validate it with the pinned Rust dependency graph:

```sh
cargo run --locked -p eon-manifest -- components/eon-alpha-v3.json
```

The validator rejects malformed, incomplete, or incompatible graphs. Before
building Eon or EonTerm, the flake checks the selected Orbit, Venus, and Helix
revisions and Cargo-owned versions against the manifest. Eon consumes the
complete composition; EonTerm selects the manifest's service and client roles
without another graph. Installed wrappers inject resolved paths as opaque
runtime inputs; `eon versions` prints stable identities and no Nix store path.

The documents in [`docs/`](docs/) hold the current planning truth:

- [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) defines naming, ownership, and sequencing.
- [`CONTRACTS.md`](docs/CONTRACTS.md) indexes the product contracts and proofs.
- [`DISTRIBUTION.md`](docs/DISTRIBUTION.md) defines composition and release policy.
- [Terminal memory on COSMIC](docs/benchmarks/eon-zlf-2026-09-08.md) reports
  the measured 219-run Eon/EonTerm, Foot, and Ghostty comparison and its limits.
- [`REFERENCES.md`](docs/REFERENCES.md) routes design work to primary sources and
  comparable projects.
- [`VISUAL-REFERENCES.md`](docs/VISUAL-REFERENCES.md) keeps discovery-only visual
  inspiration outside implementation reference gates.

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
| Agent policy inputs | 263 |
| README | 527 |
| Repository ignore rules | 3 |
| License | 201 |
| Architecture and contracts | 1,392 |
| Distribution and references | 666 |
| Benchmark report | 317 |
| Changelog | 268 |
| Rust source and tests | 13,326 |
| Cargo manifests | 37 |
| Component manifest | 349 |
| Nix composition | 783 |
| Product defaults | 0 |
| **Total** | **18,132** |
