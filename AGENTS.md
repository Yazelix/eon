# Agent Guidelines

Yazelix Astra is the greenfield Yazelix product line built around Orbit and
Venus. Astra owns orchestration, composition, product policy, and distribution.

## Status

This repository holds plans and contracts. Orbit is the active implementation
frontier. Do not add runtime code, installers, package manifests, CI, or release
automation until the user activates an Astra implementation bead.

## Core Rule

The user decides scope. Do not create a feature, compatibility surface, module,
repository, or planning bead until the user chooses that direction.

## Product Boundary

Astra composes child projects and keeps their ownership intact:

- Orbit owns persistent terminal sessions, attachment, and terminal state.
- Venus owns the native graphical client and renders Orbit sessions.
- Helix owns editing and language integration.
- Yazi owns file management.
- Ratconfig owns configuration editing.
- Astra owns launch policy, product configuration, component selection,
  integration contracts, updates, and distribution.

Yazelix Nova remains an independent product line. Do not make Astra a successor
repository, compatibility layer, or release channel for Nova.

Do not copy child behavior into Astra. Fix a child project when its contract is
wrong or incomplete.

## One Active Frontier

Keep one active implementation frontier across Orbit, Venus, and Astra unless
the user authorizes parallel product work. Use exact accepted revisions from
upstream projects. Do not build Astra against speculative child APIs.

The expected sequence is:

```text
Orbit proves its session and attachment contracts
  -> Venus proves a useful native client against Orbit
  -> Astra proves the smallest composed Yazelix product
```

Research, contracts, and Beads may prepare a later frontier without adding
production code.

## Contract-Driven Method

Index durable product contracts as `AST-C<number>` in `docs/CONTRACTS.md`.
Before implementation:

1. State the irreducible user-visible behavior in one paragraph.
2. Name the source projects and reference implementations that inform it.
3. Review the routed references and record the evidence in the bead.
4. Choose one owner for each behavior and data structure.
5. Consider existing crates and tools before adding local code.
6. Choose the cheapest check that proves the contract.
7. Implement the smallest usable vertical slice.
8. Record rejected alternatives and their failure conditions in Beads.

Do not mark a contract `Proven` from a document or mock. Cite the check, exact
component revisions, platform, and artifact that prove it.

## Reference Gate

`docs/REFERENCES.md` routes design questions to source material. Before choosing
a code shape, read the relevant primary sources and at least one comparable
implementation when one exists. Record:

- the question;
- sources and inspected revisions;
- constraints worth preserving;
- rejected approaches;
- the decision and its check.

Do not use a reference as authority outside the problem it demonstrates.

## Crate and Tool Gate

Before adding a runtime crate, framework, bundler, installer, or release tool:

1. Define the capability and ownership boundary.
2. Compare plausible maintained options, including using no dependency.
3. Inspect platform support, release health, dependency weight, license,
   security posture, and API fit.
4. Estimate the local code and maintenance each option removes or creates.
5. Record the decision, version policy, and exit condition in the bead.
6. Add the dependency only in the slice that proves its value.

Prefer a dependency that removes a complete responsibility. Avoid crates that
save a few lines while adding a second owner for the same state.

## Composition and Distribution

Treat relocatable child artifacts plus a versioned component manifest as the
canonical composition interface. Astra must not require Nix store paths or a
Nix evaluator at runtime.

Plan direct release bundles as the primary installation path. Keep Nix and Home
Manager as optional first-class channels over the same artifacts and contracts.
Do not implement a second composition graph for Nix.

`yazelix.com` may host documentation and installer entry points. Immutable
release assets, checksums, signatures, and attestations remain the artifact
authority.

## Platform Discipline

Treat Linux and macOS as architectural targets. A Linux-only prototype may
prove a narrow contract, but it must not introduce a foundation that blocks a
native, signed, and notarized macOS distribution.

Keep Unix-specific process, path, PTY, and packaging assumptions behind small
owned boundaries. Record a macOS proof plan before accepting those boundaries.

## Testing Discipline

Keep tests strong and few. A check should prove a user contract, integration
boundary, failure mode, or release property.

Use TDD for parsers, deterministic CLI behavior, manifest handling, and
regression fixes. Use contract-first integration checks for process trees,
terminal attachment, desktop behavior, packaging, and dogfooding.

Do not add mirror tests for literals, defaults, or configuration values. Test
the artifact or behavior that consumes the value.

## Git Workflow

Keep one linear history with this ancestry invariant:

```text
stable ⊆ main ⊆ edge
```

All tracked changes originate on `edge`. Work on `edge` by default. The `main`
and `stable` branches are promotion-only channels. Do not create branches,
merge commits, rebase published channel history, force-push, or promote without
explicit user direction.

## Beads

Use `br` for all issue work. Do not edit `.beads/` files directly. Serialize
`br` write commands and run `br sync --flush-only` at the end of each session.

Use `bv --robot-triage` as the graph-aware entry point. Use only `--robot-*`
flags; bare `bv` launches an interactive interface.

Keep future decisions in Beads instead of chat history. A bead must name its
contract, prerequisites, reference gate, acceptance checks, and scope boundary.

## Documentation and LOC

Update the README LOC scorecard whenever tracked project files change. Count
handwritten project text and code; exclude `.git/`, Beads data, lock files, and
generated artifacts.

Update `CHANGELOG.md` only when user-visible runtime behavior, commands,
configuration, packaged tools, installation, or a proven runtime contract
changes. Planning edits do not require changelog entries.

## Verification

Run the cheapest exact checks for the changed surface. During the planning
phase, verify Markdown paths, contract identifiers, Beads state, the LOC
scorecard, and a clean Git diff. Do not claim runtime behavior from planning
documents.
