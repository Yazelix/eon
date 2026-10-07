# Architecture

## Product boundary

Eon owns product composition and distribution. It ships
Eon as the full managed product and EonTerm as the reusable terminal product
while preserving the boundaries of the projects it composes.

Eon consumes independently pinned runtime and EONW packages from
[Eon Runtime](https://github.com/Yazelix/eon-runtime). The
[component manifest](../components/eon-alpha-v3.json) selects each source; the
[runtime library boundary](#runtime-library-boundary) records package selection
and installed consumer acceptance.

| Repository | Subsystem owner | Owns | Eon consumes |
|---|---|---|---|
| Eon Sessions | Orbit | Persistent terminals, PTYs, terminal state, attachment, transport | A versioned session and attachment contract |
| Eon Desktop | Venus | Native windows, surfaces, input, rendering, desktop integration | A versioned client artifact and launch contract |
| Anima | Anima | Terminal animations, style selection, input dismissal, and playback timing | A pinned executable for startup, CLI, and transient popup |
| Nushell | Nushell | Shell language, execution, and native configuration | A pinned shell artifact and vendor-autoload input |
| Starship | Starship | Prompt rendering, native modules, and configuration discovery | A pinned prompt artifact and guarded native initialization |
| Zoxide and fzf | Zoxide and fzf | Directory ranking, storage, shell integration, and interactive fuzzy selection | Pinned artifacts, generated Zoxide initialization, and the picker command boundary |
| Helix | Helix | Editing, language integration, editor state | A relocatable editor artifact and explicit configuration inputs |
| Yazi | Yazi | File management and navigation | A relocatable file-manager artifact and launch contract |
| LazyGit | LazyGit | Git TUI behavior and configuration | A pinned executable and native configuration inputs |
| Ratconfig | Ratconfig | User-facing configuration editing | A schema-aware configuration artifact and output contract |
| Eon | Eon orchestrator | Eon and EonTerm assembly, version, defaults, component selection, updates, integration checks, distribution | Exact child revisions and their declared artifacts |
| Eon Runtime | Runtime library and EONW | Invocation, terminal lifecycle, workspace topology, strict configuration parsing, generation identity and canonical EONW codecs | Independently pinned runtime and canonical codec packages |

Nova stays independent. Eon may reuse proven ideas from Nova through explicit
contracts, but the repositories do not share release identity or require each
other at runtime.

## Naming boundary

Public documentation presents the full composition as **Eon** and its reusable
terminal product as **EonTerm**. Their only command spellings are `eon` and
`eonterm`. A **workspace** contains **tabs** and **panes**. Each tab groups
panes around a launch directory; a pane is a view of a persistent **terminal**.
Orbit owns that terminal's process, PTY and terminal state. Closing its window
detaches the view; explicitly stopping it ends the terminal.

Product text uses these terms without requiring subsystem names. Technical
`session-N` identities and JSON `session` / `sessions` fields refer to terminals.
Their spellings, socket paths and protocol names remain unchanged.

Repository documentation uses **Eon Desktop** and **Eon Sessions**. Engineering
documentation uses **Eon orchestrator**, **Venus client**, and **Orbit terminal
runtime** when subsystem ownership matters. Venus and Orbit are not additional
public products.

Platform qualifiers describe Eon rather than Venus. Use **Eon for desktop** for
the implemented desktop surface. If later surfaces are implemented, use **Eon
for mobile** and **Eon for the web**, with `eon-mobile` and `eon-web` as their
repository names.

## Sequencing

Coupled greenfield projects make simultaneous dogfooding expensive. Eon
therefore uses one active implementation frontier:

1. Eon Sessions proves Orbit's persistent local terminals, attachment, and
   client-facing protocol against a simple reference client.
2. Eon Desktop proves Venus's native graphical interaction against an accepted
   Orbit revision.
3. Eon composes accepted Eon Sessions and Eon Desktop revisions with its
   managed interactive environment and Ratconfig through Nix.

This order lets each project use a substitute peer. Orbit can use a small CLI or
test client before Venus exists. Venus can use recorded protocol fixtures and a
reference Orbit server before Eon exists. Eon can use pinned component
artifacts and command-line checks before it gains a polished installer.

Eon alpha and early dogfood use Nix as the sole composition and installation
channel. Direct distribution starts after sustained dogfood and a separate user
decision. This sequence keeps installer, archive, signing, and package-matrix
work outside the product-discovery loop.

The Apple Silicon macOS expansion reuses the same frontier: Orbit proves
`ORB-C14`, Venus extends `VEN-C16` against that exact revision, and Eon then
proves `EON-C7` through the canonical component graph. Later frontiers remain
planning-only while the preceding native proof is open.

## Ownership rules

Eon may adapt paths, environment, configuration, and process launch at its
boundary. It must not implement shell behavior, prompt rendering, directory
ranking, terminal parsing, multiplexer state, native rendering, editor behavior,
file management, Git TUI behavior, or a second configuration schema.

A cross-project defect belongs to the project that owns the violated contract.
Eon may pin a known-good revision while the owner fixes the defect. Compatibility
shims require an explicit user decision, a removal condition, and a bead.

EONW is the one versioned workspace boundary for independently released Eon
clients. Its dependency-free owner crate defines semantic actions, complete
snapshots, supervisor lifecycle results, structured failures, and bounded
framing. EONW v7 carries Eon-owned raw tab launch directories, the popup catalog,
tab-scoped popup terminals, explicit retarget and chooser-completion actions, and
an optional pending tab's absent pane selection without interpreting Orbit
terminal metadata. Workspace and
lifecycle results are separate types, so the pinned
Venus consumer remains source- and wire-compatible with additive lifecycle
tags it never requests. The running Eon supervisor remains the only live
topology, action, launch-mode, presentation-process, and generation-lifecycle
owner; the CLI and Venus never infer state from each other, terminal output, or
the wire format. An idempotent presentation action sends one bounded private
presentation signal to the supervisor's live Venus child, or starts one replacement
after detachment with the supervisor's original native-decoration choice. Venus
alone translates that signal into a native presentation request.
A side-effect-free presentation-capability inspection leaves existing runtime
reports unchanged. Supervisors with runtime inspection but no presentation
actions remain discoverable but not presentable. In EonTerm mode the same
private EONW endpoint retains lifecycle authority but returns
`workspace-unavailable` to topology actions, while Venus receives only Orbit's
endpoint. EONW carries opaque Orbit endpoint bytes but no terminal content.

EONW v7 is the current Eon source boundary. It retains v6's optional, bounded Codex
quota field with fresh, stale, blocked, or unknown state and up to two
normalized windows. Eon's supervisor reads those facts from a dedicated
user-installed Codex app-server child; Venus alone presents the optional chip.
Credentials, account identity, provider payloads, and provider lifecycle stay
out of EONW and Venus. v7 also permits distinct pending Project pickers in
separate tabs while preserving the v6 single-pending validator. Eon provides no
version negotiation layer. Existing live supervisors retain their generation
until a normal restart. Composed activation requires an exact v7 Venus consumer.

Within one exact runtime generation, Eon consumes Orbit's canonical private
management records and lease stream without copying their schema. Eon validates
the complete live identity and acquires every lease before publishing EONW or
Venus. Same-boot supervisor replacement retains the exact Orbit runs and maps
their numeric terminal identities into one synthetic `t1` whose launch directory
is fresh replacement policy; it does not reconstruct prior topology or launch
directories. Whole-generation stop is EONW-owned product
policy routed through Orbit-owned Stop and terminal records. Venus is tied to
its Eon owner by a private stream and exits on owner EOF.

## Repository subsystem boundaries

Eon's internal boundaries follow owned invariants rather than delivery phases.
They route changes and audits without creating additional product scope.
Runtime and codec paths below belong to [Eon Runtime](https://github.com/Yazelix/eon-runtime);
assembly, manifest and Nix paths belong to this repository.

### Product assembly

- **Owning surface:** `crates/eon/src/product.rs`
- **Owns:** Product version, validated component facts, chosen shell/terminal/Anima
  and popup defaults, palette, Agent preference order, program fallbacks, opaque
  launch inputs, and the immutable assembly contribution to generation identity
- **Invocation:** `main.rs` passes concrete inputs to `eon_runtime::run`;
  graph failures are consumed only at required preflight in private `cli.rs`

### CLI and executable boundary

- **Owning surfaces:** `crates/eon-runtime/src/cli.rs` and `crates/eon/src/main.rs`
- **Owns:** `cli.rs` owns invocation selection, CLI argument and output projection, and
  managed-tool dispatch; `main.rs` owns executable composition and top-level error-to-exit
  mapping
- **Does not own:** Supervisor lifecycle, generation policy, EONW transport, workspace
  state, child mechanisms, or rendering

### Runtime and supervisor lifecycle

- **Owning surfaces:** `crates/eon-runtime/src/supervisor.rs`
- **Owns:** Launch mode, private configuration and product runtime roots, startup
  serialization, supervisor composition, presentation policy, and child
  lifecycle coordination
- **Does not own:** CLI parsing, generation discovery policy, EONW transport, Orbit terminal
  mechanisms, PTYs, terminal state, native rendering, persistent topology, or managed-tool
  behavior

### EONW transport

- **Owning surfaces:** `crates/eon-runtime/src/control.rs`
- **Owns:** Owned mode-`0600` endpoint validation, bounded client connection and
  length-delimited request/response I/O, listener lifetime, socket identity, protocol
  failure projection, and concrete client probes
- **Does not own:** Workspace or supervisor action policy, Orbit lifecycle, EONW values and
  codec, authorization, or rendering

### Generation lifecycle

- **Owning surfaces:** `crates/eon-runtime/src/generation.rs`
- **Owns:** One generation identity combining runtime/EONW bytes with the supplied
  assembly contribution, generation-directory projection, bounded discovery
  and classification, list output, explicit attachment selection, and validated owner-routed
  stop initiation
- **Does not own:** EONW transport, Orbit terminal lifecycle, component launch, topology, or
  CLI dispatch

### Orbit terminal lifecycle adapter

- **Owning surfaces:** `crates/eon-runtime/src/sessions.rs`
- **Owns:** Ready-claim authority, management-record and peer validation, lease acquisition,
  same-boot recovery, Orbit launch and rollback, management Stop, terminal-record and
  endpoint reconciliation, durable and transient terminal cleanup, and live terminal
  bookkeeping
- **Does not own:** Orbit-owned shutdown mechanics and terminal state, workspace topology,
  EONW transport, Venus presentation, or generation discovery

### Workspace state

- **Owning surfaces:** `crates/eon-runtime/src/workspace.rs`
- **Owns:** Live ordered tabs and panes, stable identities, authoritative tab launch
  directories, active selection, terminal-to-endpoint mapping, one captured-tab
  directory-picker state, deterministic recovered-terminal projection, semantic action
  results, and complete snapshots
- **Does not own:** EONW encoding, Orbit state, prior-topology persistence, shell-CWD
  inference, picker rendering, or Venus geometry

### EONW boundary

- **Owning surfaces:** `crates/eon-workspace-protocol`
- **Owns:** Versioned values, bounded message codec, validation, complete workspace
  snapshots, supervisor identity and capabilities, presentation and stop results, and
  structured failures
- **Does not own:** Live topology, lifecycle policy, transport ownership, authorization, or
  rendering

### Component graph

- **Owning surfaces:** `components/eon-alpha-v3.json` and `crates/eon-manifest`
- **Owns:** Stable component identity, compatibility requirements, abstract artifact
  declarations, graph validation, and version reporting
- **Does not own:** Resolved package paths, launch policy, or package construction

### Managed environment

- **Owning surfaces:** `crates/eon-runtime/src/managed_environment.rs` and Eon's `flake.nix` wiring
- **Owns:** Stable managed command names, private configuration projection, strict
  configuration parsing, validation and overrides of assembly-supplied defaults;
  Nix generates the selected tools' startup files
- **Does not own:** Shell, prompt, editor, file-manager, or Git-TUI native behavior

### Nix alpha composition

- **Owning surfaces:** `flake.nix`
- **Owns:** Exact source resolution, child builds, opaque launch-path injection, full Eon
  desktop packaging, and the slim EonTerm package
- **Does not own:** Runtime product semantics or a second component graph

The supervisor is the composition root for runtime policy. An opaque source-
and-graph digest selects its private generation namespace. Candidate directories
locate control endpoints, but only a bounded EONW response establishes live
identity and capabilities. A bare launch never adopts or stops a different
generation. Workspace and lifecycle state cross process boundaries only through
EONW for Eon clients; canonical Orbit management records and leases remain the
private child-lifecycle boundary. Nix resolves the canonical component graph and
injects paths without becoming a runtime owner. A subsystem review includes its
direct callers and consumers; a separate repository integration review
reconciles invariants that cross these boundaries.

## Runtime library boundary

**Status: accepted external composition on x86_64 Linux native Wayland.**
Eon and EonTerm select independent runtime and EONW package records from
Eon Runtime `b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa`, with rebound Venus
`bbf4cf41b289f441683eb7a3f225a26a4d3edacc` and accepted Orbit
`b6cecf8f2ee35570b41cfdc578b095889d917fe2`. The runtime and codec retain their
accepted transferred trees and EONW v2–v7 bytes. This repository contains no
runtime or codec production copies. `eon-runtime-cutover-y1wy` records composed
build, generation and installed migration evidence; the extraction sequence
and earlier acceptance remain in `eon-runtime-extraction-l4wh`.

Eon's actual executable and Nix packager consume an exact runtime library and
codec selection. Invocation must preserve existing Eon/EonTerm commands,
errors, configuration, terminal authority and generation isolation. Invalid
graph or package inputs fail at their existing required boundary. The extraction
adds an in-process Rust library, with no additional daemon or IPC boundary.

### Ownership and invocation

| Owner | Responsibility after independent transfer |
|---|---|
| Eon | Product contracts/version/default values, actual executable and exit/error mapping, canonical component graph and `eon-manifest`, assets, component selection, Nix translation and distribution |
| `eon-runtime` | Existing `cli`, `codex_quota`, `control`, `generation`, `managed_environment`, `sessions`, `supervisor`, `windows` and `workspace` mechanisms, private state/invariants and one strict config parser |
| `eon-workspace-protocol` in `eon-runtime` | Canonical EONW package/API/codecs, retaining package `0.1.0`, Rust `1.95`, Apache-2.0 and v2–v7 exports/bytes at transfer |
| Orbit and Venus | Their existing terminal and native-client authority |

Eon calls one concrete library entry point. Runtime handles invocation selection,
CLI projection and managed dispatch; Eon's real `main` retains process exit
mapping and the private picker's failure delay. Public inputs carry product
data, not access to private runtime operations. Rust type and function spellings
remain implementation choices; the selected package version and exact source
pin define the accepted API pair.

| Eon-supplied input | Required meaning |
|---|---|
| Product version | Version from `crates/eon/Cargo.toml`, used by CLI and EONW runtime diagnostics; library and codec versions retain their own Cargo owners |
| Fallible validated component facts | The sole graph validator's report and exact Orbit revision, including independently selected runtime/codec identities |
| Concrete defaults | Chosen shell/terminal/Anima settings, popup definitions/margins, palette, program fallbacks and Agent preference order |
| Immutable assembly contribution | Executable/assembly/default/validator/graph, product Cargo/lock and Nix recipe/lock/generated-command inputs needed by generation identity |
| Opaque launch inputs | Existing component/program paths and environment/configuration projection, preserving current resolution and override order |

Eon may precompute graph validation without effects and pass its result.
Runtime consumes a failure only where the current command needs the graph,
before runtime ownership directories, sockets or children. It preserves
help/usage, managed-tool, `config-path`, direct Anima and private-picker ordering;
the entry point does not impose blanket graph validation on them.

Source dependencies run from Eon to the library and selected Orbit/Venus
artifacts, from runtime to Orbit's canonical protocol, and from Venus to Orbit's
protocol and EONW. The library has no Eon dependency, graph, validator or
reverse source include. Its unit tests use independent, test-only input data;
the real Eon executable keeps its unchanged process integration tests.
The independent producer passes locked build/test/lint checks with ambient Eon
checkouts unavailable. A disposable real Eon assembly passes the process suite
against its exact Git source; this does not activate the production consumer.

### Defaults, overrides and generation

Eon supplies chosen default values; runtime parses and validates user overrides.
An absent config file, section or field inherits those supplied defaults.
Explicit values retain their current meaning, including false values and empty
collections where accepted. Runtime preserves unknown-field rejection, popup
constraints and validation bounds. It reads shell config for each new implicit
shell terminal and terminal config when opening or reopening a surface; it
does not cache mutable config in the immutable assembly inputs.

Deadlines, framing limits, grammar, permissions, identity checks and native
admission bounds remain runtime/child constraints. Moving defaults does not
turn those limits into product tuning options or introduce another parser.

One runtime generation owner combines its immutable runtime/EONW source and
relevant build inputs with Eon's supplied assembly contribution. Both owners'
production changes remain covered, including generated shell adapters and
private command projection. Identical inputs yield stable `g1-` identity;
runtime, defaults or generated-command changes select a different generation.
Source relocation may change identity. Store/profile paths, mutable config,
PIDs and live state do not enter it; normal runtime use evaluates no Nix.

### Independent package selection

Runtime and EONW are separate packages in the same fourth repository. Use two
logical `library` records with `cargo-package` artifacts in Eon's existing
schema-3 graph. Each record names its own exact source, Cargo version and
accepted proof. Their source URL may match while their revisions differ.
Venus's EONW requirement targets the codec record; exact revision and interface
proof equality still apply to the named component. Other requirements retain
the same checks.

For runtime revision R1 and codec revision P0, unchanged-package acceptance
covers the complete codec crate, its Cargo manifest and relevant effective
dependency/build inputs, including inherited settings if present. The current
codec has explicit package metadata and no dependencies. Unrelated runtime
files or lock entries do not change codec identity. Package/wire version labels
alone cannot prove equivalence.

The private [`nix/workspace-package.nix`](../nix/workspace-package.nix) gate
returns the selected codec path only after checking exact fetched revisions,
complete package equality with the accepted proof and runtime's local codec,
and the consumers' sole normal codec dependencies and lock declarations.
`eon-manifest` owns graph and requirement validation. The gate's caller supplies
immutable sources fetched from the validated graph and accepted proof; arbitrary
paths carrying a claimed revision are not production provenance.

The gate bounds the current explicit, dependency-free codec and resolver. It rejects
inherited codec settings, producer profiles, codec overrides and Cargo configuration,
and ignores unrelated runtime files, lock entries and unused workspace package metadata.
A broader build shape needs renewed producer or affected-consumer evidence. The check
uses accepted sources and synthetic drift fixtures. Both production consumers
use this returned path before their Git-to-path substitutions. For different
Git revisions, source preparation collapses the exact proved-identical codec
lock entries and references to one path package before Cargo compilation.
A newer runtime checkout is not an implicit codec
selection.

A runtime-only R0 → R1 change with unchanged accepted codec P0 keeps Venus's
source, exact EONW Cargo/lock pin and codec proof. It needs composed runtime and
package checks and changes generation, but no codec version bump or Venus
source/native proof solely for that change. Initial provider relocation still
requires the one-time Venus rebind. Changed codec content or relevant build
inputs need new producer evidence and explicit affected-consumer acceptance.

Cargo's [workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html)
and [Git dependency](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-dependencies-from-git-repositories)
mechanisms support separate packages and exact source selection. They do not
establish Eon's package-equivalence or runtime acceptance.

### Acceptance boundaries

| Boundary | Cheapest falsifier and implementation owner |
|---|---|
| Defaults/config/preflight | Actual parser/launch effects with supplied non-default values and absent/partial overrides; invalid graph creates no runtime owner/socket/child. `eon-runtime-product-inputs-1dbd` |
| Library/version/generation | Real Eon process suite, unequal library/product versions, stable-input identity and runtime-only A/B with fixed codec/Venus. `eon-runtime-seam-2tes` |
| Independent producer | Locked build/tests without Eon plus exact runtime/codec source comparison. `eon-runtime-producer-68hq` |
| Venus provider | Exact Cargo/lock provider move with unchanged codec/API/bytes and other dependencies. `eon-runtime-venus-rebind-tbmn` |
| Package selection | Same codec inputs at distinct runtime/codec revisions pass; codec/build drift under unchanged labels fails before substitution; unrelated exact component checks still reject. `eon-runtime-package-identity-dmbj` |
| Composed activation | Locked Rust and both Nix products accept runtime-only A/B with fixed Venus/codec; installed old/new generations retain owner authority. `eon-runtime-cutover-y1wy` |

Cutover refreshes the named profile elements without restarting live work.
Old generations remain discoverable and owner-stoppable through retained
v2–v7 lifecycle codecs. New-client Present still requires exact current EONW
and component-report equality; added package records can make old presentation
incompatible. Retain the exact prior executable/artifact to reattach old work.
A new bare launch selects the new generation; extraction promises no automatic
adoption or same-generation migration.

Installed acceptance, source removal and profile refresh stay together.
Record exact sources/artifacts/environment/result/phase before
claiming runtime proof. Nix-only Linux Wayland remains the accepted platform;
the approved unproved Darwin and distribution-graduation gates are unchanged.

## Composition unit

Each composed build consumes:

- a versioned component manifest;
- exact source or release revisions;
- declared abstract component artifacts;
- available checksums and provenance.

Nix resolves physical package paths during alpha. Eon code receives those paths
as opaque launch inputs, owns launch and configuration policy, and does not
invoke a Nix evaluator during normal use. It does not construct store paths or
persist them as stable component identity.

The component manifest provides the canonical graph for the Nix alpha, direct
bundles, and later package-manager channels. Package definitions translate that
graph instead of creating a second set of versions or policies. Relocatable
artifact requirements activate with direct-distribution work.

## Activation boundary

Planning can refine contracts, references, and Beads. Runtime work begins after
the user activates an implementation bead and its upstream proof revisions exist.
The active Eon slice launches one accepted Eon Desktop and Eon Sessions pair,
supplies the accepted managed environment, preserves Eon component and native
shell configuration boundaries, reports component identity through one
Nix-managed path, and exposes its live workspace through EONW v7. Venus
consumption follows an exact Eon producer proof; the canonical composition graph
changes only after an exact consumer proof exists. Later slices earn their scope
through dogfooding. Direct distribution has its own activation gate.
