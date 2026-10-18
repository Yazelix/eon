# Agent Guidelines for `orbit/`

These directory-local rules add to the repository root `AGENTS.md`, which
holds the protocol import and the repository-wide rules. Do not edit this
generated file directly. Edit `.agent-protocols.local.md` in this directory,
then render from the pinned source.

Orbit is Eon's terminal-session engine: the `yazelix-orbit` server, the
standalone `orbit-protocol` crate (ORBS and ORBF), and the `orbit-governance`
tool.

## Terminal authority

Orbit owns a real PTY process and the sole authoritative libghostty terminal
state. The process survives graphical-client exit or disconnection while Orbit
remains running. Exactly one local client may attach. Attachment supplies one
coherent complete host-authored structured presentation frame followed by
ordered frame revisions. The client sends semantic input; Orbit encodes it
against authoritative terminal state and alone answers terminal queries.

Clients never reconstruct terminal authority from Formatter output or raw PTY
bytes. The presentation boundary accounts explicitly for rich terminal state,
such as styled graphemes, cursor, title, hyperlinks, and graphics resources or
capabilities, rather than becoming a plain-text grid ceiling. Persistence
across Orbit or machine restarts is outside the contract.

## Fixed boundaries

Orbit serves one terminal per session over a local Unix socket to one active
client; multiplayer is a deliberate non-goal. Orbit has no remote transport,
Zellij compatibility, layouts, tabs, plugins, or configuration framework.
Broaden these only by explicit user decision.

Classify every `orbit-protocol` change as compatible or breaking and name each
known consumer: Venus, Eon Runtime, Eon, and bend-orbit. Breaking changes,
adapters, and compatibility windows need an explicit user choice. A consumer
reports missing state or capability to Orbit instead of compensating for it.

## Platform seam

x86_64 Linux is proved, and `aarch64-darwin` is proved natively under
`ORB-C14`; Venus and Eon have not adopted it. Keep PTY creation, process
groups, signals, polling, file-descriptor flags, socket paths and permissions,
and platform-specific EOF or error behavior behind one narrow seam. Keep Darwin
mechanics to what `ORB-C14` requires; add no generic portability framework,
speculative trait hierarchy, second owner loop, or other platform backend.
Compilation alone is not runtime proof.

## Clean-room rule

Mars and Mars Next are behavioral references only. Do not port, copy, or adapt
their code or architecture. To reuse a piece, first state the contract it
satisfies and why a smaller implementation is insufficient, then let the user
choose.

## Contracts, crates, and test doubles

Index only user-visible behavior, correctness boundaries, ownership invariants,
and interfaces as `ORB-C*`, never functions, modules, dependencies,
configuration literals, individual tests, or speculative features.
`docs/CRATES.md` records accepted direct and architecture-shaping crate
decisions; studying a crate does not approve it.

A diagnostic client, recorded-frame corpus, or fake child process proves only
the boundary it exercises and never becomes another backend or owner. Test
clients consume the canonical wire representation, with no second handwritten
schema and no raw PTY reparsing. Mark lossy projections: a text projection
cannot prove `ORB-C6` rich-presentation fidelity.

Name Superlogical's published first product `Superlogical terminal
multiplexer`; do not invent a shorter name or research codename.
