# Architecture

## Product boundary

The Eon orchestrator is the thin composition and distribution owner. It gives
users one product while preserving the boundaries of the projects it
composes.

| Repository | Subsystem owner | Owns | Eon consumes |
|---|---|---|---|
| Eon Sessions | Orbit | Persistent sessions, PTYs, terminal state, attachment, transport | A versioned session and attachment contract |
| Eon Desktop | Venus | Native windows, surfaces, input, rendering, desktop integration | A versioned client artifact and launch contract |
| Helix | Helix | Editing, language integration, editor state | A relocatable editor artifact and explicit configuration inputs |
| Yazi | Yazi | File management and navigation | A relocatable file-manager artifact and launch contract |
| Ratconfig | Ratconfig | User-facing configuration editing | A schema-aware configuration artifact and output contract |
| Eon | Eon orchestrator | Product policy, component selection, launch, updates, integration checks, distribution | Exact child revisions and their declared artifacts |

Nova stays independent. Eon may reuse proven ideas from Nova through explicit
contracts, but the repositories do not share release identity or require each
other at runtime.

## Naming boundary

Public documentation presents the composition as **Eon**. It calls durable
terminal work **Sessions** and does not require users to learn subsystem names.

Repository documentation uses **Eon Desktop** and **Eon Sessions**. Engineering
documentation uses **Eon orchestrator**, **Venus client**, and **Orbit session
runtime** when subsystem ownership matters. Venus and Orbit are not additional
public products.

Platform qualifiers describe Eon rather than Venus. Use **Eon for desktop** for
the implemented desktop surface. If later surfaces are implemented, use **Eon
for mobile** and **Eon for the web**, with `eon-mobile` and `eon-web` as their
repository names.

## Sequencing

Three coupled greenfield projects make simultaneous dogfooding expensive. Eon
therefore uses one active implementation frontier:

1. Eon Sessions proves Orbit's persistent local sessions, attachment, and
   client-facing protocol against a simple reference client.
2. Eon Desktop proves Venus's native graphical interaction against an accepted
   Orbit revision.
3. Eon composes accepted Eon Sessions and Eon Desktop revisions with Helix,
   Yazi, and Ratconfig into the smallest usable product slice through Nix.

This order lets each project use a substitute peer. Orbit can use a small CLI or
test client before Venus exists. Venus can use recorded protocol fixtures and a
reference Orbit server before Eon exists. Eon can use pinned component
artifacts and command-line checks before it gains a polished installer.

Eon alpha and early dogfood use Nix as the sole composition and installation
channel. Direct distribution starts after sustained dogfood and a separate user
decision. This sequence keeps installer, archive, signing, and package-matrix
work outside the product-discovery loop.

## Ownership rules

Eon may adapt paths, environment, configuration, and process launch at its
boundary. It must not implement terminal parsing, multiplexer state, rendering,
editor behavior, file management, or a second configuration schema.

A cross-project defect belongs to the project that owns the violated contract.
Eon may pin a known-good revision while the owner fixes the defect. Compatibility
shims require an explicit user decision, a removal condition, and a bead.

## Composition unit

Each composed build consumes:

- a versioned component manifest;
- exact source or release revisions;
- declared component artifacts or package outputs;
- available checksums and provenance;
- a small set of declared launch and configuration inputs.

Nix provides those inputs during alpha. Eon code receives component paths as
opaque launch inputs and does not invoke a Nix evaluator during normal use. It
does not construct store paths or persist them as stable component identity.

The component manifest provides the canonical graph for the Nix alpha, direct
bundles, and later package-manager channels. Package definitions translate that
graph instead of creating a second set of versions or policies. Relocatable
artifact requirements activate with direct-distribution work.

## Activation boundary

Planning can refine contracts, references, and Beads. Runtime work begins after
the user activates an implementation bead and its upstream proof revisions exist.
The first Eon slice launches one accepted Eon Desktop and Eon Sessions pair,
exposes Helix and Yazi, preserves one configuration path, and reports component
identity through one Nix-managed path. Later slices earn their scope through
dogfooding. Direct distribution has its own activation gate.
