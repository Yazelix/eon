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
control through EONW v1, isolate live runtime generations, and consume the same
component identities. Direct bundles, Home Manager, background updates, and
release automation remain outside this slice. X11, Xwayland, and macOS are
unsupported. The accepted runtime proof uses COSMIC with systemd. Eon targets
native Wayland without requiring a specific init or service manager, but
non-systemd use remains unproved. Orbit's accepted Linux lifecycle still
requires a user-owned writable cgroup-v2 parent with `cgroup.kill`.

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
        |       +---- Starship + Zoxide + Atuin + Carapace
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
in product policy. A durable second language needs a concrete subsystem benefit
that outweighs its toolchain and ownership cost; this composition has none.

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
Session. Before starting a new Orbit process, Eon waits within the existing
startup deadline until its current cgroup is safe for Orbit to inherit. This
uses no compositor, init, or service-manager API; Orbit still validates and
owns Session containment. Its Nix closure
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
one vertical pane selection per tab. Each pane starts and maps to a distinct
Orbit session; changing focus never stops a session. The CLI reaches that owner
through the private local Eon socket using EONW v1. Each accepted action returns
one complete ordered workspace snapshot; incompatible or malformed requests
receive a bounded structured failure. Additive EONW lifecycle actions report a
supervisor's generation, component graph, live Sessions, idempotent presentation,
and stop result through a separate result type that Eon Desktop never receives.
Eon Desktop consumes the workspace result and shows every fitting pane header
around one selected live Session. New generations identify panes as `p1`, `p2`,
and so on. Each visible live header shows that exact identity, two spaces, then
a `~/`-anchored path below home, an absolute path elsewhere, or Nova's marker
for the exact home directory. Unset or empty `HOME` leaves paths absolute.
Overlong labels preserve their rightmost components.
New current-generation full Eon windows omit the redundant native title bar;
the selected terminal retains compositor title semantics, while EonTerm and
legacy Eon attachment keep native decorations. Alt+H/L traverses tabs,
Alt+K/J traverses panes, Alt+M creates a
pane, and Ctrl+T creates a tab. External workspace changes appear within one
second because Eon Desktop re-inspects EONW v1 every 250 ms; the protocol adds
no event stream. When a shell exits, Eon removes its pane, selects the nearest
surviving pane, removes an empty tab, and closes when the final pane exits.

Workspace topology is live-only. After same-boot supervisor loss, full Eon
projects surviving canonical `session-N` runs into one `tab-1` as `pN` in
numeric order and selects the lowest number; it does not reconstruct prior tabs,
focus, commands, or history. The exact Orbit processes, PTY children, and
terminal state remain Orbit-owned and unchanged.

The command surface is small:

| Command | Result |
|---|---|
| `eon` | Present the exact current-generation workspace, or start it with the default shell |
| `eon run` | Explicitly start one Orbit session and one Venus window with the default shell |
| `eon run -- COMMAND...` | Run one explicit command as the Orbit-owned PTY child |
| `eonterm [--no-decorations] [--application-id ID] -- COMMAND...` | Start or present one exact command in a native terminal surface without Eon workspace or managed-environment policy |
| `eonterm attach [GENERATION]` | Present the current or one selected compatible EonTerm generation |
| `eonterm generations [--json]` | List validated current and older EonTerm generations |
| `eonterm stop GENERATION [--json]` | Stop one EonTerm generation through its supervisor; human mode confirms first |
| `eon attach` | Present Eon Desktop against the exact current-generation launch mode |
| `eon attach GENERATION` | Present one explicitly selected compatible generation, including `legacy` |
| `eon generations [--json]` | List validated current, previous, legacy, dead, incompatible, unreachable, and corrupt generations |
| `eon stop GENERATION [--json]` | Stop one generation through its supervisor; human mode confirms first |
| `eon workspace [--json]` | Inspect the live Eon-owned tab, pane, and Session mapping |
| `eon tab create [--json]` | Create and focus a tab containing a default-shell Session |
| `eon pane create [--json]` | Create and select a default-shell Session in the active tab |
| `eon focus ID [--json]` | Focus a stable tab or pane identity |
| `eon focus left\|right\|up\|down [--json]` | Traverse tabs or panes directly without wrapping |
| `eon versions` | Print the runtime generation, EONW version, and stable component identities |
| `eon config-path` | Create and print the Eon configuration root |

Workspace commands target the exact current-generation Eon supervisor. An
EonTerm supervisor returns `workspace-unavailable` to topology actions at the
EONW boundary. The workspace topology is bounded
to 64 tabs and 256 panes, is not persisted, and has no per-pane or per-Session
removal action. Whole-generation stop sends canonical management Stop to every
validated Session lease and succeeds only after every exact terminal record is
complete. `--json` reports the same
accepted EONW result as the human view; neither output format is the protocol
schema. Its `endpoint` field is an ordered integer array that preserves every
opaque endpoint byte.

## Terminal presentation

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
request. Eon applies both values from one snapshot when creating a Venus
surface. Editing the file does not change a live surface; after Venus exits,
`eon attach` or `eonterm attach` reads the current values for its replacement
without restarting the live Orbit Session or PTY child.

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
that resolves the four shells, four integrations, `hx`, `yazi`, `ya`, and
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
- [`REFERENCES.md`](docs/REFERENCES.md) routes design work to primary sources and
  comparable projects.

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
| Agent policy | 483 |
| README | 413 |
| Repository ignore rules | 3 |
| License | 201 |
| Architecture and contracts | 700 |
| Distribution and references | 243 |
| Changelog | 163 |
| Rust source and tests | 8,782 |
| Cargo manifests | 37 |
| Component manifest | 327 |
| Nix composition | 728 |
| Product defaults | 0 |
| **Total** | **12,080** |
