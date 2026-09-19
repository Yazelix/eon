# Architecture

## Product boundary

The Eon orchestrator is the thin composition and distribution owner. It ships
Eon as the full managed product and EonTerm as the reusable terminal product
while preserving the boundaries of the projects it composes.

| Repository | Subsystem owner | Owns | Eon consumes |
|---|---|---|---|
| Eon Sessions | Orbit | Persistent sessions, PTYs, terminal state, attachment, transport | A versioned session and attachment contract |
| Eon Desktop | Venus | Native windows, surfaces, input, rendering, desktop integration | A versioned client artifact and launch contract |
| Nushell | Nushell | Shell language, execution, and native configuration | A pinned shell artifact and vendor-autoload input |
| Starship | Starship | Prompt rendering, native modules, and configuration discovery | A pinned prompt artifact and guarded native initialization |
| Zoxide and fzf | Zoxide and fzf | Directory ranking, storage, shell integration, and interactive fuzzy selection | Pinned artifacts, generated Zoxide initialization, and the picker command boundary |
| Helix | Helix | Editing, language integration, editor state | A relocatable editor artifact and explicit configuration inputs |
| Yazi | Yazi | File management and navigation | A relocatable file-manager artifact and launch contract |
| LazyGit | LazyGit | Git TUI behavior and configuration | A pinned executable and native configuration inputs |
| Ratconfig | Ratconfig | User-facing configuration editing | A schema-aware configuration artifact and output contract |
| Eon | Eon orchestrator | Eon and EonTerm product policy, launch mode, workspace topology and EONW, component selection, launch, updates, integration checks, distribution | Exact child revisions and their declared artifacts |

Nova stays independent. Eon may reuse proven ideas from Nova through explicit
contracts, but the repositories do not share release identity or require each
other at runtime.

## Naming boundary

Public documentation presents the full composition as **Eon** and its reusable
terminal product as **EonTerm**. Their only command spellings are `eon` and
`eonterm`. It calls durable terminal work **Sessions** and does not require
users to learn subsystem names.

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
tab-scoped popup Sessions, explicit retarget and chooser-completion actions, and
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
their numeric Session identities into one synthetic `t1` whose launch directory
is fresh replacement policy; it does not reconstruct prior topology or launch
directories. Whole-generation stop is EONW-owned product
policy routed through Orbit-owned Stop and terminal records. Venus is tied to
its Eon owner by a private stream and exits on owner EOF.

## Repository subsystem boundaries

Eon's internal boundaries follow owned invariants rather than delivery phases.
They route changes and audits without creating additional product scope.

### CLI and executable boundary

- **Owning surfaces:** `crates/eon/src/cli.rs` and `crates/eon/src/main.rs`
- **Owns:** `cli.rs` owns invocation selection, CLI argument and output projection, and
  managed-tool dispatch; `main.rs` owns executable composition and top-level error-to-exit
  mapping
- **Does not own:** Supervisor lifecycle, generation policy, EONW transport, workspace
  state, child mechanisms, or rendering

### Runtime and supervisor lifecycle

- **Owning surfaces:** `crates/eon/src/supervisor.rs`
- **Owns:** Launch mode, private configuration and product runtime roots, startup
  serialization, supervisor composition, presentation policy, component selection, and child
  lifecycle coordination
- **Does not own:** CLI parsing, generation discovery policy, EONW transport, Orbit Session
  mechanisms, PTYs, terminal state, native rendering, persistent topology, or managed-tool
  behavior

### EONW transport

- **Owning surfaces:** `crates/eon/src/control.rs`
- **Owns:** Owned mode-`0600` endpoint validation, bounded client connection and
  length-delimited request/response I/O, listener lifetime, socket identity, protocol
  failure projection, and concrete client probes
- **Does not own:** Workspace or supervisor action policy, Orbit lifecycle, EONW values and
  codec, authorization, or rendering

### Generation lifecycle

- **Owning surfaces:** `crates/eon/src/generation.rs`
- **Owns:** Runtime-source identity and generation-directory projection, bounded discovery
  and classification, list output, explicit attachment selection, and validated owner-routed
  stop initiation
- **Does not own:** EONW transport, Orbit Session lifecycle, component launch, topology, or
  CLI dispatch

### Orbit Session lifecycle adapter

- **Owning surfaces:** `crates/eon/src/sessions.rs`
- **Owns:** Ready-claim authority, management-record and peer validation, lease acquisition,
  same-boot recovery, Orbit launch and rollback, management Stop, terminal-record and
  endpoint reconciliation, durable and transient Session cleanup, and live Session
  bookkeeping
- **Does not own:** Orbit-owned shutdown mechanics and terminal state, workspace topology,
  EONW transport, Venus presentation, or generation discovery

### Workspace state

- **Owning surfaces:** `crates/eon/src/workspace.rs`
- **Owns:** Live ordered tabs and panes, stable identities, authoritative tab launch
  directories, active selection, Session-to-endpoint mapping, one captured-tab
  directory-picker state, deterministic recovered-Session projection, semantic action
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

- **Owning surfaces:** `crates/eon/src/managed_environment.rs` and its `flake.nix` wiring
- **Owns:** Stable managed command names, private configuration projection, exact tool
  selection, and default interactive policy
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
