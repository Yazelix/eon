# Architecture

## Product boundary

Astra is a thin orchestrator and distribution owner. It gives users one Yazelix
product while preserving the boundaries of the projects it composes.

| Project | Owns | Astra consumes |
|---|---|---|
| Orbit | Persistent sessions, PTYs, terminal state, attachment, transport | A versioned session and attachment contract |
| Venus | Native windows, surfaces, input, rendering, desktop integration | A versioned client artifact and launch contract |
| Helix | Editing, language integration, editor state | A relocatable editor artifact and explicit configuration inputs |
| Yazi | File management and navigation | A relocatable file-manager artifact and launch contract |
| Ratconfig | User-facing configuration editing | A schema-aware configuration artifact and output contract |
| Astra | Product policy, component selection, launch, updates, integration checks, distribution | Exact child revisions and their declared artifacts |

Nova stays independent. Astra may reuse proven ideas from Nova through explicit
contracts, but the repositories do not share release identity or require each
other at runtime.

## Sequencing

Three coupled greenfield projects make simultaneous dogfooding expensive. Astra
therefore uses one active implementation frontier:

1. Orbit proves persistent local sessions, attachment, and the client-facing
   protocol against a simple reference client.
2. Venus proves native graphical interaction against an accepted Orbit revision.
3. Astra composes accepted Orbit and Venus revisions with Helix, Yazi, and
   Ratconfig into the smallest usable Yazelix slice.

This order lets each project use a substitute peer. Orbit can use a small CLI or
test client before Venus exists. Venus can use recorded protocol fixtures and a
reference Orbit server before Astra exists. Astra can use pinned component
artifacts and command-line checks before it gains a polished installer.

## Ownership rules

Astra may adapt paths, environment, configuration, and process launch at its
boundary. It must not implement terminal parsing, multiplexer state, rendering,
editor behavior, file management, or a second configuration schema.

A cross-project defect belongs to the project that owns the violated contract.
Astra may pin a known-good revision while the owner fixes the defect. Compatibility
shims require an explicit user decision, a removal condition, and a bead.

## Composition unit

Each release consumes:

- a versioned component manifest;
- exact source or release revisions;
- relocatable artifacts for the target platform;
- checksums and provenance;
- a small set of declared launch and configuration inputs.

The component manifest provides the canonical graph for direct bundles, Nix,
Home Manager, and later package-manager channels. Package definitions translate
that graph instead of creating a second set of versions or policies.

## Activation boundary

Planning can refine contracts, references, and Beads. Runtime work begins after
the user activates an implementation bead and its upstream proof revisions exist.
The first Astra slice should launch one accepted Venus and Orbit pair, open the
core tools, preserve one configuration path, and expose a diagnostic version
report. Later slices earn their scope through dogfooding.
