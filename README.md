# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

Eon is a greenfield product built around Eon Sessions and Eon Desktop. Its
canonical component graph also selects the shell, prompt, directory navigator,
editor, file manager, and Git TUI without reimplementing its child projects.

## Project status

This repository ships the first Nix-only Eon alpha for x86_64 Linux. One Rust
supervisor launches the accepted Eon Sessions and Eon Desktop revisions, owns
either a live workspace of independent Sessions or one terminal-host Session,
supplies one pinned interactive environment, exposes control through EONW v1,
keeps one configuration root, isolates live runtime generations, and reports
the canonical component identities. Direct bundles, Home Manager, background
updates, release automation, and macOS packaging remain outside this slice.

## Naming model

**Eon** is the public product name. **Sessions** is its user-facing name for
durable terminal work. Repository names describe what each repository ships;
Venus and Orbit identify the underlying engineering subsystems.

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
the result.

Yazelix Nova remains a separate product. Nova provides a compact Yazelix runtime
on its own architecture and keeps value independent of Eon's progress.

## Design commitments

- Eon Sessions, Eon Desktop, and Eon advance through one active implementation frontier.
- Indexed contracts define cross-repository behavior.
- Exact child revisions and explicit component inputs form the composition boundary.
- Nix provides the sole alpha and early-dogfood installation path.
- Eon runtime code remains independent of Nix concepts and evaluation.
- Direct bundles wait for sustained dogfood and an explicit graduation decision.
- Linux and macOS constrain architecture from the start.
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

The package installs one `Eon` desktop entry and a transparent violet three-fold
loop icon at native launcher sizes, with X11 and Xwayland window grouping.
Opening Eon reconnects only to the exact installed runtime generation or starts
that generation in its own private namespace. Older live generations and their
Sessions remain running. Concurrent starts converge on one supervisor and one
initial Session; every attach-capable peer presents that owner. Its Nix closure
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
explicit non-interactive form. Pressing `Ctrl-C` in the original foreground
`eon` process also stops that composed generation. Restarting the machine
preserves no process state beyond the accepted child contracts.

Use terminal-host mode when one command should receive the terminal key stream
without Eon's tab and pane shortcuts:

```sh
eon terminal -- COMMAND...
eon terminal --no-decorations -- COMMAND...
```

This mode hosts exactly one Orbit Session, gives Venus only the Orbit endpoint,
and keeps Eon's generation, presentation, stop, child-exit, and cleanup lifecycle.
A terminal host uses native window decorations unless `--no-decorations` is
selected. The supervisor preserves its original choice when it replaces a
detached surface.
A repeated invocation preserves the active surface; after that surface closes,
another invocation asks the supervisor to open one replacement against the same
Session. Workspace actions are unavailable, and a live workspace and terminal
host cannot share one generation namespace.
Terminal programs can request bounded plain-text clipboard writes through Orbit;
Venus delivers them to the requested ordinary or primary Linux clipboard.
`Ctrl+Shift+V` and the native Paste key read the ordinary clipboard and send one
semantic paste to Orbit, which alone applies terminal paste encoding.

While the foreground supervisor is running, Eon owns horizontal tab order and
one vertical pane selection per tab. Each pane starts and maps to a distinct
Orbit session; changing focus never stops a session. The CLI reaches that owner
through the private local Eon socket using EONW v1. Each accepted action returns
one complete ordered workspace snapshot; incompatible or malformed requests
receive a bounded structured failure. Additive EONW lifecycle actions report a
supervisor's generation, component graph, live Sessions, idempotent presentation,
and stop result through a separate result type that Eon Desktop never receives.
Eon Desktop consumes the workspace result, shows every fitting pane header around
one selected live Session, and binds Alt+H/L to tabs, Alt+K/J to panes, Alt+M to
pane creation, and Ctrl+T to tab creation. External workspace changes appear
within one second
because Eon Desktop re-inspects EONW v1 every 250 ms; the protocol adds no event
stream. When a shell exits, Eon removes its pane, selects the nearest surviving
pane, removes an empty tab, and closes when the final pane exits.

The command surface is small:

| Command | Result |
|---|---|
| `eon` | Present the exact current-generation workspace, or start it with the default shell |
| `eon run` | Explicitly start one Orbit session and one Venus window with the default shell |
| `eon run -- COMMAND...` | Run one explicit command as the Orbit-owned PTY child |
| `eon terminal [--no-decorations] -- COMMAND...` | Start or present one command in a native terminal surface without Eon workspace actions |
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

Workspace commands target the exact current-generation supervisor and return
`workspace-unavailable` in terminal-host mode. The workspace topology is bounded
to 64 tabs and 256 panes, is not restored after supervisor loss, and has no
per-pane or per-Session removal action. Whole-generation stop names and
terminates every Session through that supervisor. `--json` reports the same
accepted EONW result as the human view; neither output format is the protocol
schema. Its `endpoint` field is an ordered integer array that preserves every
opaque endpoint byte.

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
or `$HOME/.config/eon`. `EON_RUNTIME_DIR` selects the socket directory; Eon
otherwise uses `$XDG_RUNTIME_DIR/eon` or a private per-user temporary directory.
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

The validator rejects malformed, incomplete, or incompatible graphs. The flake
asserts each resolved source revision against this manifest before it builds a
package. The installed wrapper injects resolved paths as opaque runtime inputs;
`eon versions` prints stable identities and no Nix store path.

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
| Agent policy | 459 |
| README | 316 |
| Repository ignore rules | 3 |
| License | 201 |
| Architecture and contracts | 590 |
| Distribution and references | 231 |
| Changelog | 88 |
| Rust source and tests | 5,957 |
| Cargo manifests | 35 |
| Component manifest | 316 |
| Nix composition | 647 |
| Product defaults | 0 |
| **Total** | **8,843** |
