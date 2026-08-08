# Eon

*Build for Eons.*

![Glowing violet three-bend Eon portal](assets/eon.png)

Eon is a greenfield product built around Eon Sessions and Eon Desktop. Its
canonical component graph selects them with the editor, file manager, and
configuration tools without reimplementing its child projects.

## Project status

This repository ships the first Nix-only Eon alpha for x86_64 Linux. One Rust
supervisor launches the accepted Eon Sessions and Eon Desktop revisions,
exposes the pinned Helix and Yazi tools, keeps one configuration root, and
reports the canonical component identities. Direct bundles, Home Manager,
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
        |
        v
Eon Desktop / Venus client subsystem
        |
        v
Eon Sessions / Orbit session subsystem
        |
        +---- Helix
        +---- Yazi
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

The package installs an `Eon` desktop entry and violet three-bend portal icon, with X11
and Xwayland window grouping. Opening Eon starts a session or reconnects to the
active session. Its Nix closure supplies Mesa's open-source Vulkan drivers; the
current graphics proof uses Intel hardware, while proprietary NVIDIA remains
unproved. Run Eon from a terminal when you need foreground lifecycle control.
Closing the Eon Desktop window detaches the client while Sessions and its PTY
keep running. Reconnect explicitly with:

```sh
eon attach
```

Press `Ctrl-C` in the original foreground `eon` process to stop that composed
session. Orbit removes its socket during shutdown. Restarting Orbit or the
machine preserves no process state beyond the accepted child contracts.

The command surface is small:

| Command | Result |
|---|---|
| `eon` | Attach to the active Orbit session, or start one with the default shell |
| `eon run` | Explicitly start one Orbit session and one Venus window with the default shell |
| `eon run -- COMMAND...` | Run one explicit command as the Orbit-owned PTY child |
| `eon attach` | Open Venus against the active local Orbit socket |
| `eon versions` | Print stable component versions and Git revisions from the canonical manifest |
| `eon config-path` | Create and print the Eon configuration root |

`EON_CONFIG_HOME` selects the configuration root. Without it, Eon uses
`$XDG_CONFIG_HOME/eon` or `$HOME/.config/eon`. `EON_RUNTIME_DIR` selects the
socket directory; Eon otherwise uses `$XDG_RUNTIME_DIR/eon` or a private
per-user temporary directory. Eon passes the configuration root to its child
components as `XDG_CONFIG_HOME`. Eon ignores relative XDG base paths. It creates
missing configuration and runtime directories with mode `0700`. It leaves
existing configuration-directory permissions unchanged and rejects unsafe
existing runtime directories without changing their permissions.

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
| Agent policy | 200 |
| README | 178 |
| Repository ignore rules | 3 |
| Architecture and contracts | 207 |
| Distribution and references | 202 |
| Changelog | 20 |
| Rust source and tests | 1,026 |
| Cargo manifests | 23 |
| Component manifest | 173 |
| Nix composition | 254 |
| **Total** | **2,286** |
