# Architecture

## Product boundary

Eon owns product composition and distribution. It ships
Eon as the full managed product and eonbase as the reusable terminal product
while preserving the boundaries of the projects it composes.

This repository holds Orbit, Venus, Eon Runtime with EONW, and Eon as one Cargo
workspace. The [component manifest](../components/eon-v5.json) names each
in-tree component's workspace path and each external source's exact revision;
the [runtime library boundary](#runtime-library-boundary) records the runtime
seam.

| Repository | Subsystem owner | Owns | Eon consumes |
|---|---|---|---|
| Eon (`orbit/`) | Orbit | Persistent terminals, PTYs, terminal state, attachment, transport | A versioned session and attachment contract |
| Eon (`venus/`) | Venus | Native windows, surfaces, input, rendering, desktop integration | A versioned client artifact and launch contract |
| Anima | Anima | Terminal animations, style selection, input dismissal, and playback timing | A pinned executable for startup, CLI, and transient popup |
| Nushell | Nushell | Shell language, execution, and native configuration | A pinned shell artifact and vendor-autoload input |
| Starship | Starship | Prompt rendering, native modules, and configuration discovery | A pinned prompt artifact and guarded native initialization |
| Zoxide and fzf | Zoxide and fzf | Directory ranking, storage, shell integration, and interactive fuzzy selection | Pinned artifacts, generated Zoxide initialization, and the picker command boundary |
| Helix | Helix | Editing, language integration, editor state | A relocatable editor artifact and explicit configuration inputs |
| Yazi | Yazi | File management and navigation | A relocatable file-manager artifact and launch contract |
| LazyGit | LazyGit | Git TUI behavior and configuration | A pinned executable and native configuration inputs |
| Ratconfig | Ratconfig | User-facing configuration editing | A schema-aware configuration artifact and output contract |
| Eon | Eon orchestrator | Eon and eonbase assembly, version, defaults, component selection, updates, integration checks, distribution | In-tree subsystems, exact external revisions, and their declared artifacts |
| Eon (`runtime/`) | Runtime library and EONW | Invocation, terminal lifecycle, workspace topology, strict configuration parsing, generation identity and canonical EONW codecs | In-tree runtime and canonical codec packages |

Nova stays independent. Eon may reuse proven ideas from Nova through explicit
contracts, but the repositories do not share release identity or require each
other at runtime.

## Naming boundary

Public documentation presents the full composition as **Eon** and its reusable
terminal product as **eonbase**. Their only command spellings are `eon` and
`eonbase`. A **workspace** contains **tabs** and **panes**. Each tab groups
panes around a launch directory and arranges them in up to four side-by-side
**columns**, each a vertical stack with one expanded pane; a pane is a view of
a persistent **terminal**.
Orbit owns that terminal's process, PTY and terminal state. Closing its window
detaches the view; explicitly stopping it ends the terminal.
Under planned [EON-C33](CONTRACTS.md#eon-c33--several-workspaces-in-one-window),
an Eon **window** holds one or more workspaces and shows one at a time;
switching the shown workspace keeps every terminal running. A workspace may
have a name, but its `wN` identity identifies it.

Product text uses these terms without requiring subsystem names. Technical
`session-N` identities and JSON `session` / `sessions` fields refer to terminals.
Their spellings, socket paths and protocol names remain unchanged.

Documentation names the subsystems **Orbit**, **Venus** and **Eon Runtime**
when ownership matters; engineering documentation may say **Eon orchestrator**,
**Venus client**, and **Orbit terminal runtime**. Venus and Orbit are not
additional public products.

Platform qualifiers describe Eon rather than Venus. Use **Eon for desktop** for
the implemented desktop surface. If later surfaces are implemented, use **Eon
for mobile** and **Eon for the web**, with `eon-mobile` and `eon-web` as their
repository names.

## Sequencing

Coupled greenfield projects make simultaneous dogfooding expensive. Eon
therefore uses one active implementation frontier:

1. Orbit proves its persistent local terminals, attachment, and client-facing
   protocol against a simple reference client.
2. Venus proves its native graphical interaction against an accepted Orbit.
3. Eon composes accepted Orbit and Venus with its managed interactive
   environment and Ratconfig through Nix.

This order lets each project use a substitute peer. Orbit can use a small CLI or
test client before Venus exists. Venus can use recorded protocol fixtures and a
reference Orbit server before Eon exists. Eon can use pinned component
artifacts and command-line checks before it gains a polished installer.

Eon currently uses Nix as its sole published composition and installation
channel. Publishing a direct channel needs accepted delivery proof and a
separate user decision, which keeps installer and archive maintenance off the
published path.

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
framing. EONW v25 carries Eon-owned raw tab launch directories, each tab's
zero to four pane columns, each pane's semantic label and optional pin, the popup catalog,
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
reports unchanged. In eonbase mode the same private EONW endpoint retains
lifecycle authority but returns `workspace-unavailable` to topology actions,
while Venus receives only Orbit's endpoint. EONW carries opaque Orbit endpoint
bytes but no terminal content, except a program's own bounded status report
text and its terminal's title (EON-C31).

Since v20, `workspace` names the workspace described by `tabs` and `active_tab`,
and `active_workspace` names the one shown in the window. A request carrying a
validated caller projects that terminal's workspace; a request without a caller
projects the shown workspace. The four workspace requests operate on Runtime's
workspace set. The `eon workspaces` and `eon workspace new|switch|rename|close`
CLI is their front end, and the Workspaces picker (`eon-workspaces`,
Alt+Shift+S) and the Commands workspace entries run that CLI as the popup's
caller.
`RenamePane` targets an explicit pane in that addressed workspace.
Runtime's independent read-only ORBS observers cover every pane, including
background tabs and hidden workspaces. They supply title/directory display data;
Runtime combines it with its pane pin and launch-directory context into `label`.
The pin is separate from terminal title and attention, follows the pane object,
and is absent after supervisor recovery. Venus consumes the label without a
second metadata observer or naming policy.

EONW v25 is the current Eon source boundary. It adds the window-wide EON-C31
`attention_queue` in Runtime's total order. v24 adds `LaunchTab`, which starts a
non-pending tab at an explicit directory with the default pane or an exact argv
from a calling terminal, optionally preserving focus (EON-C10). v23 adds the
background attention state, for an agent that reported done or idle while
processes it started still run, ranked between working and idle (EON-C31).
v22 gives each workspace
summary its attention rollup and its recency rank, which the Workspaces picker
orders by and the Eon Bar label's badge reads (EON-C33). v21 gives each popup
entry a shortcut, or none because the configuration unbound it or because another
entry holds its default shortcut (EON-C10). v20 adds the window's ordered
workspace summaries, shown identity and caller projection, and requests to
create, switch, rename and close a workspace (EON-C33). v19 adds the attention
`name`, the terminal title that tells agents apart in the sidebar (EON-C31). v18 adds
EON-C31 agent attention: each pane and popup's normalized attention with its
most urgent record, each tab's rollup, the window's sidebar visibility, and the
`ToggleSidebar`, `FocusAttention`, `NextAttention` and `PreviousAttention`
actions. v17 adds one
reply, `Ended`, which answers a tab or pane close that stopped the window's last
terminal, since a snapshot cannot carry an empty workspace. v16 adds a focus flag to both
pane-create requests, so a pane can be added without selecting it or
dismissing the tab's popup. v15 adds one request that runs an
exact argv, with no shell, in a new pane, and the runtime accepts it only from a
calling terminal; it is the one request that carries a command. v14 adds one
request that opens an absolute file path in a new pane running the editor, which
the runtime chooses. Since v13 each request may name its
calling terminal, whose tab and pane replace the active selection for pane
create, edit, move and close actions and tab moves; the CLI sends the terminal's
`EON_TERMINAL`, Venus sends none, and the supervisor resolves it against its
live Orbit runs. v12 adds one request that moves the selected pane into the
neighbouring column; v11 adds one request that closes a pane by its stable
identity. v10 arranges each tab's panes into up to four columns, each with
one expanded pane, ahead of the unchanged v9 snapshot of the flattened panes. The current
snapshot retains v6's optional, bounded Codex quota field with fresh, stale,
blocked, or unknown state and up to two
normalized windows. Pinned CodexBar CLI owns Codex and Claude collection; Runtime validates and ages
its readings, while Venus alone presents the optional chips. v9 permits absent
provider resets.
Credentials, account identity, provider payloads, and provider lifecycle stay
out of EONW and Venus. v7 also permits distinct pending Project pickers in
separate tabs. Eon provides no version negotiation layer. Existing live
supervisors retain their generation until explicitly stopped; the updated
executable starts a separate generation.
Composed activation requires an exact v25 Venus consumer.

The optional Claude observation adds only bounded presentation facts. Under
EON-C27, pinned CodexBar collects it; Runtime owns bounded subprocesses,
validation, observation age and expiry, and Venus owns its native chips.
Credential contents, account identity and raw provider output stay
outside EONW and Venus.

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
Paths below are relative to the repository root.

### Product assembly

- **Owning surfaces:** `eon/crates/eon/src/product.rs` and
  `eon/crates/eon/build.rs`, which digests the source tree
- **Owns:** Product version, validated component facts, chosen shell/terminal/Anima
  and popup defaults, palette, Agent preference order, program fallbacks, opaque
  launch inputs, and the immutable assembly contribution to generation identity
- **Invocation:** `main.rs` passes concrete inputs to `eon_runtime::run`;
  graph failures are consumed only at required preflight in private `cli.rs`

### CLI and executable boundary

- **Owning surfaces:** `runtime/crates/eon-runtime/src/cli.rs` and `eon/crates/eon/src/main.rs`
- **Owns:** `cli.rs` owns invocation selection, CLI argument and output projection, and
  managed-tool dispatch; `main.rs` owns executable composition and top-level error-to-exit
  mapping
- **Does not own:** Supervisor lifecycle, generation policy, EONW transport, workspace
  state, child mechanisms, or rendering
- **Agent parity:** The CLI is the agent's projection of every Runtime workspace action.
  Each EONW `Action` a client can send has a CLI command or a named exception;
  `runtime/crates/eon-runtime/src/cli/parity.rs` maps them, and its test checks
  that each workspace command parses to its action and each lifecycle command is
  in `eon --help`. Venus-local view state is listed as an exception and has no
  EONW action. See [Humans and Agents](../AGENTS.md#humans-and-agents). Open
  gaps: `eon-agent-parity-a9g0`.

### Runtime and supervisor lifecycle

- **Owning surfaces:** `runtime/crates/eon-runtime/src/supervisor.rs`
- **Owns:** Launch mode, private configuration and product runtime roots, startup
  serialization, supervisor composition, presentation policy, and child
  lifecycle coordination
- **Does not own:** CLI parsing, generation discovery policy, EONW transport, Orbit terminal
  mechanisms, PTYs, terminal state, native rendering, persistent topology, or managed-tool
  behavior

### EONW transport

- **Owning surfaces:** `runtime/crates/eon-runtime/src/control.rs`
- **Owns:** Owned mode-`0600` endpoint validation, bounded client connection and
  length-delimited request/response I/O, listener lifetime, socket identity, protocol
  failure projection, and concrete client probes
- **Does not own:** Workspace or supervisor action policy, Orbit lifecycle, EONW values and
  codec, authorization, or rendering

### Generation lifecycle

- **Owning surfaces:** `runtime/crates/eon-runtime/src/generation.rs`
- **Owns:** One generation identity hashing the supplied assembly contribution,
  generation-directory projection, bounded discovery
  and classification, list output, explicit attachment selection, and validated owner-routed
  stop initiation
- **Does not own:** EONW transport, Orbit terminal lifecycle, component launch, topology, or
  CLI dispatch

### Orbit terminal lifecycle adapter

- **Owning surfaces:** `runtime/crates/eon-runtime/src/sessions.rs`
- **Owns:** Ready-claim authority, management-record and peer validation, lease acquisition,
  same-boot recovery, Orbit launch and rollback, management Stop, terminal-record and
  endpoint reconciliation, durable and transient terminal cleanup, and live terminal
  bookkeeping
- **Does not own:** Orbit-owned shutdown mechanics and terminal state, workspace topology,
  EONW transport, Venus presentation, or generation discovery

### Workspace state

- **Owning surfaces:** `runtime/crates/eon-runtime/src/workspace.rs`
- **Owns:** Live ordered tabs and panes, stable identities, authoritative tab launch
  directories, active selection, terminal-to-endpoint mapping, one captured-tab
  directory-picker state, deterministic recovered-terminal projection, semantic action
  results, and complete snapshots
- **Does not own:** EONW encoding, Orbit state, prior-topology persistence, shell-CWD
  inference, picker rendering, or Venus geometry

### EONW boundary

- **Owning surfaces:** `runtime/crates/eon-workspace-protocol`
- **Owns:** Versioned values, bounded message codec, validation, complete workspace
  snapshots, supervisor identity and capabilities, presentation and stop results, and
  structured failures
- **Does not own:** Live topology, lifecycle policy, transport ownership, authorization, or
  rendering

### Component graph

- **Owning surfaces:** `components/eon-v5.json` and `eon/crates/eon-manifest`
- **Owns:** Stable component identity, the targets each component supports,
  compatibility requirements, abstract artifact declarations, graph validation, and
  version reporting
- **Does not own:** Resolved package paths, launch policy, or package construction

### Managed environment

- **Owning surfaces:** `runtime/crates/eon-runtime/src/managed_environment.rs` and `nix/shell-environment.nix`
- **Owns:** Stable managed command names, private configuration projection, strict
  configuration parsing, validation and overrides of assembly-supplied defaults;
  Nix generates the selected tools' startup files
- **Does not own:** Shell, prompt, editor, file-manager, or Git-TUI native behavior
- **Configurability:** Runtime is the only configuration parser. Venus and
  Orbit receive resolved values as launch inputs or over EONW and parse none.
  Each user-visible value must be a documented setting or recorded as fixed
  with a reason. See
  [Configurability](../AGENTS.md#configurability). Open gaps:
  `eon-configurability-qkq8`.

### Nix composition

- **Owning surfaces:** `flake.nix`
- **Owns:** In-tree workspace builds, exact external source resolution, opaque launch-path injection, full Eon
  desktop packaging, and the slim eonbase package
- **Does not own:** Runtime product semantics or a second component graph

The supervisor is the composition root for runtime policy. An opaque source-
and-graph digest selects its private generation namespace. Candidate directories
locate control endpoints, but only a bounded EONW response establishes live
identity and capabilities. A bare launch never adopts or stops a different
generation. Workspace and lifecycle state cross process boundaries only through
EONW for Eon clients; canonical Orbit management records and leases remain the
private child-lifecycle boundary. Nix resolves the canonical component graph and
injects paths without becoming a runtime owner. A subsystem review includes its
direct callers and consumers; a repository-wide integration review reconciles
invariants that cross these boundaries.

## Runtime library boundary

**Status: in-tree composition on x86_64 Linux native Wayland.** Eon and
eonbase build Eon Runtime, EONW, Orbit and Venus from this workspace.

Eon's actual executable and Nix packager consume the in-tree runtime library and
codec as an in-process Rust library, with no daemon or IPC boundary between
them. Invalid graph or package inputs fail at their required boundary.

### Ownership and invocation

| Owner | Responsibility |
|---|---|
| Eon | Product contracts/version/default values, actual executable and exit/error mapping, canonical component graph and `eon-manifest`, assets, component selection, Nix translation and distribution |
| `eon-runtime` | The `attention`, `cli`, `quota`, `control`, `generation`, `managed_environment`, `sessions`, `supervisor`, `windows` and `workspace` mechanisms, private state/invariants and one strict config parser |
| `eon-workspace-protocol` in `runtime/crates/` | Canonical EONW package/API/codec v25 (package `0.1.0`, Rust `1.95`, Apache-2.0) |
| Orbit and Venus | Their terminal and native-client authority |

Eon calls one concrete library entry point. Runtime handles invocation selection,
CLI projection and managed dispatch; Eon's real `main` retains process exit
mapping and the private picker's failure delay. Public inputs carry product
data, not access to private runtime operations. Rust type and function spellings
remain implementation choices; the repository revision defines the accepted API
pair.

| Eon-supplied input | Required meaning |
|---|---|
| Product version | Version from `eon/crates/eon/Cargo.toml`, used by CLI and EONW runtime diagnostics; library and codec versions retain their own Cargo owners |
| Fallible validated component facts | The sole graph validator's report |
| Concrete defaults | Chosen shell/terminal/Anima settings, popup definitions/margins, palette, program fallbacks and Agent preference order |
| Immutable assembly contribution | Eon's source-tree digest; see [In-tree package selection](#in-tree-package-selection) |
| Opaque launch inputs | Component/program paths and environment/configuration projection, in their resolution and override order |

Eon may precompute graph validation without effects and pass its result.
Runtime consumes a failure only where the current command needs the graph,
before runtime ownership directories, sockets or children. It preserves
help/usage, managed-tool, `config-path`, direct Anima and private-picker ordering;
the entry point does not impose blanket graph validation on them.

Source dependencies run from Eon to the library and selected Orbit/Venus
artifacts, from runtime to Orbit's canonical protocol, and from Venus to Orbit's
protocol and EONW. The library has no Eon dependency, graph, validator or
reverse source include. Its unit tests use independent, test-only input data;
the real Eon executable keeps its process integration tests.
`eon-runtime` builds and tests without the Eon crates.

### Defaults, overrides and generation

Eon supplies chosen default values; runtime parses and validates user overrides.
An absent config file, section or field inherits those supplied defaults.
Explicit values keep their meaning, including false values and empty
collections where accepted. Runtime preserves unknown-field rejection, popup
constraints and validation bounds. It reads shell config for each new implicit
shell terminal and terminal config when opening or reopening a surface; it
does not cache mutable config in the immutable assembly inputs.

Deadlines, framing limits, grammar, permissions, identity checks and native
admission bounds remain runtime/child constraints. Supplied defaults do not
turn those limits into product tuning options or introduce another parser.

Runtime owns the generation algorithm and its `g1-` format, and hashes only
the assembly contribution Eon supplies: a digest of the source tree that covers
runtime and EONW source, defaults, generated shell adapters and private command
projection. Identical inputs yield stable `g1-` identity; a change to any of
them selects a different generation.
Source relocation may change identity. Store/profile paths, mutable config,
PIDs and live state do not enter it; normal runtime use evaluates no Nix.

### In-tree package selection

Orbit, Venus, `eon-runtime` and `eon-workspace-protocol` are in-tree graph
components with `workspace` sources: the repository revision identifies them, so
they carry no revision of their own. A requirement names its target and
interfaces, never a revision; the target alone owns its identity. Nix builds
Orbit and Venus from the in-tree crates and Eon from those crates plus the graph
and its Nix recipes, all with the root `Cargo.lock`. It checks that each
workspace component's path holds the expected Cargo package at the graph's
version.

Eon's build script digests the path and bytes of each file in the source tree
that [EON-C3](CONTRACTS.md#eon-c3--distribution-neutral-runtime-inputs) names.
That digest is Eon's assembly contribution, so a change in any in-tree component
selects a new generation.
Orbit terminal records carry the runtime's generation ID as their component
generation. Mutable configuration, store and profile paths never enter the
digest.

## Composition unit

Each composed build consumes:

- a versioned component manifest;
- exact source or release revisions;
- declared abstract component artifacts;
- available checksums and provenance.

Nix resolves physical package paths in the Nix channel. Eon code receives those
paths as opaque launch inputs, owns launch and configuration policy, and does
not invoke a Nix evaluator during normal use. It does not construct store paths
or persist them as stable component identity.

The component manifest provides the canonical graph for the Nix channel, direct
bundles, and later package-manager channels. Package definitions translate that
graph instead of creating a second set of versions or policies. Relocatable
artifact requirements activate with direct-distribution work.

## Activation boundary

Planning can refine contracts, references, and Beads. Runtime work begins after
the user activates an implementation bead and its prerequisite proofs exist. The
active Eon slice launches one accepted Venus and Orbit pair, supplies the
accepted managed environment, preserves Eon component and native shell
configuration boundaries, reports component identity through one Nix-managed
path, and exposes its live workspace through EONW v25. Venus consumes a new EONW
version only after Eon Runtime's producer proof; the graph's interface versions
change only after Venus's consumer proof exists. Later slices earn their scope
through dogfooding. Direct distribution has its own activation gate.

## Remote access

**Status: planned and unproved; no contract.** On 2026-10-15 the user chose
SSH as Eon's remote path: a user keeps Eon's window on a local Wayland desktop
while terminals, editor, Git and agents run on a headless Linux server. Eon
opens no network listener and owns no login, authentication or encryption.
`eon-remote-ssh-x7dr` owns the candidate shapes (a Waypipe-forwarded window
first and, only if that falls short, a local Venus with a remote supervisor
over SSH-forwarded sockets), their measurements, and keeping remote work alive
after the SSH session ends. EON-C15 currently excludes both logout survival and
remote authority.

| Owner | Responsibility |
|---|---|
| OpenSSH | Reachability, authentication, encryption, host configuration, keys and jump hosts |
| Waypipe | Candidate external tool for the first shape: Wayland forwarding, buffer transfer and compression |
| Eon | Remote launch policy and disconnect-versus-Stop policy |
| Eon Runtime | Supervisor start on the server (headless start open in `eon-remote-ssh-x7dr.3`) and any Eon-owned survival mechanism (open in `eon-remote-ssh-x7dr.2`) |
| Orbit | Terminal authority over local Unix sockets; any row-patch protocol for slow links is a separate decision |
| Venus | Presentation of the workspace, including the attached machine if Venus stays local |
