# Eon

*Build for Eons.*

![Yazelisk, the Yazelix basilisk mascot, standing before the Eon portal](assets/yazelisk-eon.png)

Eon is a greenfield product built around Eon Sessions and Eon Desktop. Its
canonical component graph selects them with the editor, file manager, and
configuration tools without reimplementing its child projects.

## Project status

This repository contains the canonical alpha component manifest, its Rust
validator, planning contracts, and Beads. It contains no runtime, installer,
package, or release implementation. `astra-db8.2` owns the accepted component
graph; `astra-db8.3` owns the first Nix-only Linux composition.

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
that outweighs its toolchain and ownership cost; this validator has none.

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

The validator rejects malformed, incomplete, or incompatible graphs. The Nix
composition that resolves artifact paths remains `astra-db8.3` work.

The documents in [`docs/`](docs/) hold the current planning truth:

- [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) defines naming, ownership, and sequencing.
- [`CONTRACTS.md`](docs/CONTRACTS.md) indexes the planned product contracts.
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
| Agent policy | 195 |
| README | 125 |
| Architecture and contracts | 153 |
| Distribution and references | 197 |
| Changelog | 8 |
| Rust source and tests | 528 |
| Cargo manifests | 13 |
| Component manifest | 173 |
| **Total** | **1,392** |
