# Yazelix Astra

Yazelix Astra is the greenfield Yazelix product line built around Orbit and
Venus. Astra will compose the terminal runtime, native client, editor, file
manager, and configuration tools into one small product. It owns Yazelix policy
and distribution instead of reimplementing its child projects.

## Project status

This repository contains planning contracts and Beads. It contains no runtime,
installer, package, or release implementation. Orbit remains the active
implementation frontier; Venus follows after Orbit proves the contracts needed
by a graphical client. Astra implementation starts after those boundaries have
working evidence and the user activates an implementation bead.

## Target composition

```text
yzx / Astra policy
        |
        v
      Venus  <---- native graphical interaction
        |
        v
      Orbit  <---- persistent sessions and terminal state
        |
        +---- Helix
        +---- Yazi
        +---- Ratconfig
```

Each child repository keeps one clear responsibility. Astra selects compatible
versions, supplies product configuration, launches the composition, and ships
the result.

Yazelix Nova remains a separate product. Nova provides a compact Yazelix runtime
on its own architecture and keeps value independent of Astra's progress.

## Design commitments

- Orbit, Venus, and Astra advance through one active implementation frontier.
- Indexed contracts define cross-repository behavior.
- Exact child revisions and explicit component inputs form the composition boundary.
- Nix provides the sole alpha and early-dogfood installation path.
- Astra runtime code remains independent of Nix concepts and evaluation.
- Direct bundles wait for sustained dogfood and an explicit graduation decision.
- Linux and macOS constrain architecture from the start.
- Astra owns product policy and avoids copying child behavior.

The documents in [`docs/`](docs/) hold the current planning truth:

- [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) defines ownership and sequencing.
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
| Agent policy | 182 |
| README | 78 |
| Architecture and contracts | 112 |
| Distribution and references | 191 |
| Changelog | 8 |
| **Total** | **571** |
