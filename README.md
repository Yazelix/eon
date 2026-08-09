# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

Eon is a greenfield product built around Eon Sessions and Eon Desktop. Its
canonical component graph also selects the shell, prompt, directory navigator,
editor, file manager, and Git TUI without reimplementing its child projects.

## Project status

This repository ships the first Nix-only Eon alpha for x86_64 Linux. One Rust
supervisor launches the accepted Eon Sessions and Eon Desktop revisions, owns a
live workspace of independent Sessions, supplies one pinned interactive
environment, exposes that workspace through EONW v1, keeps one configuration
root, and reports the canonical component identities. Direct bundles, Home
Manager, updates, release automation, and macOS packaging remain outside this
slice.

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
        +---- Nushell + Starship + Zoxide
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
loop icon at native launcher sizes, with X11 and Xwayland window grouping. Opening
Eon starts a session or reconnects to its live workspace. Its Nix closure supplies
Mesa's open-source Vulkan drivers; the current graphics proof uses Intel hardware,
while proprietary NVIDIA remains unproved. Run Eon from a terminal when you need
foreground lifecycle control.
Closing the Eon Desktop window detaches the client while Sessions and its PTY
keep running. Reconnect explicitly with:

```sh
eon attach
```

Press `Ctrl-C` in the original foreground `eon` process to stop that composed
session. Orbit removes its socket during shutdown. Restarting Orbit or the
machine preserves no process state beyond the accepted child contracts.

While the foreground supervisor is running, Eon owns horizontal tab order and
one vertical pane selection per tab. Each pane starts and maps to a distinct
Orbit session; changing focus never stops a session. The CLI reaches that owner
through the private local Eon socket using EONW v1. Each accepted action returns
one complete ordered workspace snapshot; incompatible or malformed requests
receive a bounded structured failure. Eon Desktop consumes that same protocol,
renders the tabs and accordion panes, and follows the selected live Session.
After creating a tab or pane from another CLI, reopen Eon Desktop to load the
new snapshot; EONW v1 has no subscription or polling stream.

The command surface is small:

| Command | Result |
|---|---|
| `eon` | Attach to the live workspace, or start one with the default shell |
| `eon run` | Explicitly start one Orbit session and one Venus window with the default shell |
| `eon run -- COMMAND...` | Run one explicit command as the Orbit-owned PTY child |
| `eon attach` | Open Eon Desktop against the live Eon workspace |
| `eon workspace [--json]` | Inspect the live Eon-owned tab, pane, and Session mapping |
| `eon tab create [--json]` | Create and focus a tab containing a default-shell Session |
| `eon pane create [--json]` | Create and select a default-shell Session in the active tab |
| `eon focus ID [--json]` | Focus a stable tab or pane identity |
| `eon focus left\|right\|up\|down [--json]` | Traverse tabs or panes directly without wrapping |
| `eon versions` | Print stable component versions and Git revisions from the canonical manifest |
| `eon config-path` | Create and print the Eon configuration root |

Workspace commands require the live foreground supervisor. The topology is
bounded to 64 tabs and 256 panes, is not restored after supervisor loss, and
has no removal or session-stop action in this slice. `--json` reports the same
accepted EONW result as the human view; neither output format is the protocol
schema.

## Managed environment

A Session without an explicit command starts Eon's pinned Nushell. Starship
supplies its native modules with a violet `∴` prompt marker and no leading blank
line, while Zoxide supplies its native `z` integration. The package exposes the
managed tools outside Eon only through these names:

| Command | Managed tool |
|---|---|
| `eon-nu` | Nushell |
| `eon-hx` | Helix |
| `eon-yazi` | Yazi |
| `eon-ya` | Yazi companion CLI |
| `eon-lazygit`, `eon-lg` | LazyGit |

Outside a Session, the prefixed commands inherit the ambient PATH. Inside a
Session, one private PATH resolves `nu`, `hx`, `yazi`, `ya`, and `lazygit` to
those same artifacts; managed Nushell also defines `lg`. Eon does not alter the
parent process's PATH, aliases, or shell startup files. Each tool keeps its
native configuration schema, data, and cache behavior. At launch, Eon ignores
ambient Helix runtime and Steel configuration paths and Yazi or LazyGit
configuration paths that would bypass its private root; explicit Helix and
LazyGit configuration arguments remain available. LazyGit uses the Git
executable already available from the user's environment. The current packaged
font set does not guarantee every emoji, Powerline, or Yazi icon glyph, so
unsupported symbols may render as fallback boxes.

`EON_CONFIG_HOME` selects the configuration root; Eon resolves a relative value
once against the launch directory. Without it, Eon uses `$XDG_CONFIG_HOME/eon`
or `$HOME/.config/eon`. `EON_RUNTIME_DIR` selects the socket directory; Eon
otherwise uses `$XDG_RUNTIME_DIR/eon` or a private per-user temporary directory.
Eon passes the absolute root as `XDG_CONFIG_HOME` to child components and
managed tools; it additionally preserves `EON_CONFIG_HOME` through Sessions and
managed dispatch so nested commands keep the same root. Eon ignores relative
XDG base paths. It creates missing configuration and runtime directories with
mode `0700`. It leaves existing configuration-directory permissions unchanged
and rejects unsafe existing runtime directories without changing their
permissions.

## Component manifest

[`components/eon-alpha-v1.json`](components/eon-alpha-v1.json) is the one
distribution-neutral source of component identity, compatibility, artifacts,
and semantic launch inputs. Its activation list distinguishes required alpha
contracts from additional recorded proof such as `ORB-C10`. It contains no
resolved Nix store paths.

Validate it with the pinned Rust dependency graph:

```sh
cargo run --locked -p eon-manifest -- components/eon-alpha-v1.json
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
| README | 231 |
| Repository ignore rules | 3 |
| License | 201 |
| Architecture and contracts | 572 |
| Distribution and references | 202 |
| Changelog | 33 |
| Rust source and tests | 3,169 |
| Cargo manifests | 33 |
| Component manifest | 263 |
| Nix composition | 334 |
| Product defaults | 10 |
| **Total** | **5,510** |
