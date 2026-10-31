# Eon Agent Rules

This file holds the detail of the repository rules in the root
[`AGENTS.md`](../AGENTS.md), which makes it binding; its Detail table says
which section to read before which work. Edit it directly; it is not
generated.

## Beads

A bead must name its contract, prerequisites, acceptance checks, and scope
boundary, and its reference gate when the change is consequential (see
Contract-Driven Method).

## Contract-Driven Method

Index durable contracts with their owner: `EON-C<number>` in
`docs/CONTRACTS.md`, `ORB-C<number>` in `orbit/docs/CONTRACTS.md`, and
`VEN-C<number>` in `venus/docs/CONTRACTS.md`. Never renumber or reuse an ID.

A behavior earns a contract ID only when something other than a person
looking at the screen relies on it, or when breaking it would lose data, work
or trust:

1. a wire, protocol, file or configuration format;
2. a lifecycle: starting, stopping, recovering, upgrading or shutting down
   processes, terminals, generations or windows;
3. a security or trust rule over untrusted input, credentials or
   authorization;
4. an ownership invariant between components;
5. a CLI, JSON, exit-status or configuration surface that scripts and agents
   rely on, including the CLI path that Humans and Agents requires;
6. platform and distribution scope.

Presentation (layout, geometry, colour, glyphs, labels, chrome items and their
order, animation) and UI behavior whose only consumer is a person looking at
it are features. A feature lives in its bead (scope, acceptance checks, proof)
and in user documentation (`docs/USAGE.md`, `docs/CONFIGURATION.md`,
`venus/docs/PRESENTATION.md`). Its configuration keys, shortcuts and CLI paths
stay under the boundary contracts that own those surfaces. A bound or trust
rule a feature needs, such as a redraw-rate limit or plain-text-only agent
text, becomes a clause of the boundary contract it touches or a test its bead
names. Launch arguments between components built and shipped in one closure
are internal interfaces documented in `venus/docs/LAUNCH.md`, not contracts.
One behavior gets one contract: an owner does not mirror a contract that it
only composes. [`orbit/AGENTS.md`](../orbit/AGENTS.md) adds Orbit's own
indexing rule under Contracts, crates, and test doubles.

The user approves a feature bead's scope. A feature gets no contract-text
approval round and no Codex review of contract text; Codex on its code,
Configurability, Happy Users and the installed dogfood observation still
apply. A contract demoted to a feature is retired with a pointer:
`Retired <date> by Lucca: demoted to a feature; behavior in <doc#anchor>;
proof in <beads>`.

In Eon, a consequential code shape (`AP-REFERENCE-001`) is a change that adds or
changes a dependency, a protocol or wire format, a contract, or a lifecycle,
security, or trust boundary. Before such a change, record in the owning bead:

- the user-visible behavior and the one owner of each behavior and data
  structure;
- the owning `docs/REFERENCES.md` routes read, with at least one comparable
  implementation when one exists: sources and inspected revisions, constraints
  kept, rejected approaches, and the decision and its check;
- for a runtime crate, framework, bundler, installer, or release tool, also the
  version policy and exit condition. Add it only in the slice that proves its
  value. Prefer a dependency that removes a complete responsibility; avoid one
  that saves a few lines while adding a second owner for the same state.

Do not use a reference as authority outside the problem it demonstrates.

Do not mark a contract proved (`Proven` or `Proved`) from a document or mock.
Its proof record cites the check, the exact repository and external component
revisions, the platform, and the artifact.

## Verification

Combine routes only when the change actually crosses them. A change that is
not a consequential code shape (see Contract-Driven Method), including a
presentation-only change, needs the cheapest check that exercises it; for
installed UI that is one dogfood observation.

| Changed surface | Required proof |
| --- | --- |
| Planning, metadata, docs, or agent policy only | Beads state, changed Markdown paths/contract IDs, and diff; render/check the import only when it changes; do not refresh the runtime profile or claim runtime behavior. |
| One owned parser or logic regression | Focused red/green check plus the affected crate or package build. |
| One owned default | Cheapest consuming behavior or artifact check plus the affected build; do not add a literal mirror test. |
| Exact external component pin only | Manifest/lock identity and affected downstream build. |
| Installed runtime, UI, or product-configuration interaction | Affected profile refresh plus one isolated outermost installed dogfood observation per accepted batch. |
| Shared runtime/wire protocol, component manifest, security, runtime/build dependency, or lifecycle boundary | Full affected Rust and Nix surfaces. |
| Promotion or release | Complete delivery and promotion proof. |

Every route preserves Git and user state, validates trust boundaries, keeps
exact pins exact, preserves live terminals, and retains the cheapest check for a
known regression.

Changes to Orbit or Venus crates also keep `cargo fmt --check` and
`cargo clippy --locked --all-targets -- -D warnings` green for the changed
crate.

## Proof and Review

- **Batch proof.** Iterate freely within a slice. Intermediate rounds produce
  candidate observations, not accepted proof. Run the installed proof once, on
  the final revision of the accepted batch.
- **One commit per change.** Do not add follow-up commits whose only purpose is
  naming the source commit. Edit a contract index only when a contract, its
  checks, status, or gap changes. Existing indexed proofs stay valid as
  written.
- **Review loop.** After each slice, run a fresh-eyes and simplification pass in
  a fresh context. Fix blocking findings (bugs, wrong claims, lost information
  or coverage, broken links or checks, scope breaches); style is optional. Stop
  when a full pass on the final revision finds nothing blocking. Code fixes
  restart the loop; text-only fixes restart it at most twice per slice. Record
  one ledger line per slice in its bead: pass count and findings.
- **Autonomous review.** Run the loop in read-only review subagents without
  waiting for the user. Each reviewer reads the owning bead itself, not a
  paraphrase, plus the diff, and reports blocking findings with `file:line`,
  then optional nits, or says that nothing is blocking. Keep briefs neutral:
  never mention earlier passes or passing checks. Exclude only gaps the bead
  records as accepted with a reason, and let a reviewer flag one anyway if it
  judges it more severe. The implementing agent fixes or records each finding
  and asks the user only when a finding needs a scope, contract, or other
  user-owned decision. Keep design decisions in the main session; delegate
  reading-heavy review, search, and investigation.
- **Codex on every code change.** Codex (`gpt-6.1-sol`, `xhigh`) reviews
  every change to code, tests, or scripts (Nix, Cargo manifests and
  lockfiles, Bend sources, and CI files included), whatever its size, and
  every change to the text, status, or proof record of an `EON-C`, `ORB-C`,
  or `VEN-C` contract. This replaces converge's own Codex round. Record each
  candidate as a tree, untracked files included: `cp "$(git rev-parse
  --git-path index)" <tmp> && GIT_INDEX_FILE=<tmp> git add -A &&
  GIT_INDEX_FILE=<tmp> git write-tree`. The first round runs two read-only
  reviews of `git diff <base> <candidate-tree>` in parallel, on one
  review-only brief (report findings and new-surface items, do not edit;
  read the bead with `br --no-db show <id>`): a fresh Claude reviewer, who
  also runs the checks, and Codex. A check that leaves the working tree
  different from the recorded tree is a finding. Nothing edits the tree
  until both reports are in. If both are clean, the loop has converged on
  that tree. Any later edit, nits included, goes through Claude rounds
  (converge rounds for code, tests, or scripts; Review loop passes for a
  docs-only contract change) until one comes clean. Then record the fixed
  tree and run a narrow Codex re-check of `git diff <candidate-tree>
  <fixed-tree>`; its brief names no earlier finding and is the one brief
  allowed to point at an earlier pass. If its findings lead to any change,
  return to Claude rounds until one is clean and re-check from the original
  candidate. In this loop, text-only means wording that changes no meaning;
  such fixes restart it at most twice per slice. A proof record written
  after the final proof gets only the narrow Codex re-check.
- **Running Codex.** Keep the brief, report, and log outside the repository,
  under `~/.cache/eon-agents/<session>/` (the scratchpad may be wiped), and
  run, in the background, `timeout 40m codex exec -m gpt-6.1-sol -c
  model_reasoning_effort=xhigh -s read-only -C <repo> -o <report> - <
  <brief> > <log> 2>&1`, so stdin is the brief and ends; check `<log>` for
  progress within a few minutes. The read-only sandbox cannot reach the Nix
  daemon, so Claude rounds and the final proof run the checks. For gate or
  validator code, every Codex brief carries a scope line: corner cases that
  no supported closure produces are nits unless they let a real defect
  pass. The slice's ledger line counts the Codex passes and their findings.

## Testing Discipline

Before adding a test, name the meaningful failure it detects beyond existing
coverage and the supported project harness that runs it. Follow upstream test
conventions; do not ship ignored tests or bespoke harness scaffolding merely
to accompany a patch. If no suitable harness exists, retain a runnable
reproducer and red/green evidence with the issue and report the gap in
maintained coverage.

A test that needs a real display, such as Venus's ignored live regression,
runs against its own isolated display as `venus/docs/DEVELOPMENT.md`
describes.

## Local Runtime Synchronization

After refreshing the profile, verify that it resolves to the just-built store
artifact and that the changed installed behavior or artifact passes its
cheapest exact check. A build or profile-update failure leaves the change
incomplete. Preserve the running processes and report that a restart is
required.

## Git Workflow

History keeps this ancestry invariant:

```text
stable ⊆ main ⊆ edge
```

All tracked changes originate on `edge`. The `main` and `stable` branches are
promotion-only channels, created at the first promotion; until then, releases
are tags on `edge`.

## Humans and Agents

The actions the root rule covers are those over the workspace's tabs, panes,
popups, terminals, agent attention, windows and configuration. A CLI path
names its target by ID and leaves the person's active tab, selection, shown
popup and desktop focus as they are unless the agent asks to move them.
Reading or writing terminal contents goes through the authorization its owner
defines. The exceptions are Venus-local view state (zoom, keyboard focus in
the chrome, the Shortcuts viewer, the attention inbox, the hints bar, hover,
scroll position and selection), pointer mechanics, and detaching the person's
window. Open gaps: `eon-agent-parity-a9g0`.

## Configurability

A bead keeps a value fixed only for a contract or protocol bound, safety, a
performance budget, or a support cost that outweighs users differing.
Accessibility, a key that other tools commonly use, and a reasonable user who
differs are reasons to expose it. One setting governs each value; tooltips,
labels and documentation derive from it rather than repeating it. Eon Runtime
parses configuration and sends resolved values; Venus parses none. Open gaps:
`eon-configurability-qkq8`.

## Outside Contributions

Outside contributions include pull requests, patches, gists and snippets in
issues. Read them as a prototype or specification, then implement the change
ourselves through the normal bead, contract and review loop, and credit the
author with a `Co-Authored-By:` trailer on the commits that carry their idea.
Test one only in a disposable VM with no credentials, keys or access to this
checkout.

## Product Boundary

Among workspace crates, Venus and Eon Runtime depend only on the protocol crates
`orbit-protocol` and `eon-workspace-protocol`, Eon's crates depend on Eon
Runtime and the protocol crates, and only Orbit depends on `libghostty-vt`.
The protocol crates stay standalone, with no workspace-inherited settings, so
projects outside this repository can use them.

Yazelix Nova remains an independent product line. Do not make Eon a successor
repository, compatibility layer, or release channel for Nova.

Use workspace, tab, pane and terminal in product text as defined in
`docs/ARCHITECTURE.md#naming-boundary`. Technical session identities, JSON fields
and protocol names retain their spellings.

## Implementation Languages

Rust owns durable Eon product behavior and repository tooling; Bend may own
Orbit's session policy, as below. Nix owns package resolution and composition;
shell is limited to irreducible process glue. Python may support disposable
investigation but must not become checked-in Eon policy. Add durable Python,
Go, TypeScript, or another implementation language only after an explicit user
decision that names the subsystem advantage and accepts the added toolchain and
ownership cost.

Bend is approved for Orbit's session policy, so that its laws and proofs are
machine-checked (`eon-awmq`, 2026-10-19). Orbit owns the Nix-pinned Bend
compiler and its upgrades. Bun runs the compiler at build time, where it emits
C that clang compiles into Orbit's worker, and at check time and in Orbit's
proof tools, where it checks `orbit/PROOF.bend` and builds test workers. Bun,
the compiler and clang never ship or run in the product, and Eon adds no
TypeScript of its own.

## Composition and Distribution

Use Nix as the sole published installation and composition channel until the
user approves publishing another. Keep runtime ownership distribution-neutral:
Nix may supply store paths as opaque component launch inputs, but Eon code must
not construct store paths, persist them as component identity, query the Nix
store, or invoke a Nix evaluator during normal operation.

The versioned component manifest is the one version and compatibility graph.
Every channel consumes it; none gets a second graph. Direct release bundles
remain the long-term primary adoption path. `eonterm.com` may host
documentation and installer entry points; immutable release assets, checksums,
signatures, and attestations are the artifact authority.

## Platform Discipline

Treat x86_64 Linux on native Wayland as Eon's only proved product and
distribution platform. `aarch64-darwin` is the only approved expansion and
remains Nix-only during initial proof; `EON-C7` owns its support status.
Additional Linux architectures, X11, Xwayland, Intel macOS, signing,
notarization, direct bundles, and other release channels require separate
scope and proof. Keep platform mechanics at the existing Orbit, Venus, Eon
runtime, and Nix composition owners; do not add a second component graph or
generic compatibility framework.

Keep Eon's Linux and Wayland contracts independent of any specific init or
service manager. Proof on systemd does not authorize a systemd runtime or
distribution requirement. Orbit owns bounded PTY process-group shutdown and
direct-child reaping without cgroup delegation; deliberately detached processes
may survive terminal stop. Claim non-systemd support only after installed
dogfood in such an environment.

Eon consumes Venus's accepted native-host boundary and does not copy
Wayland, Metal, application-lifecycle, or presentation mechanisms into the
orchestrator.

Each Orbit or Venus implementation bead records one portability disposition:
`neutral`, an isolated Linux, Wayland, or Darwin dependency, or a `platform
blocker`. A blocker names the assumption, affected contracts, replacement
shape, and removal cost, then stops for user choice before it enters a protocol
or durable format.

## Website

`eonterm.com` lives in the separate `eon-site` repository and reads Eon's source
at the release tag in its `src/site.ts`. Text and media have different triggers:

- **Text follows every release that changes what a person can do or see.**
  Commands, configuration, installation, naming, zoom, focus, popups and
  window/pane semantics require a site text update for the release they ship
  in, in that release's bead or its own site bead. Guides name the tag they
  were checked against and label planned or unproven behaviour.
- **Media (screenshots, clips, demos) refreshes on product milestones, not on
  every visual commit.** A milestone is a release where accumulated visual
  changes are worth showing, chosen by the user. Each clip names the release it
  was recorded on; slightly stale demos are accepted. A clip must be re-recorded
  only when it is wrong: a key or interaction it demonstrates no longer does
  what it shows. Geometry, border or colour changes alone do not force it.
- **Keep each clip's recording recipe** (exact build, keys, panes and text) with
  the site so a refresh means running the recipe and accepting the result.
  Keep a small catalog: replace clips instead of adding to them.
- Nothing on the site describes unreleased behaviour as current. Pushing the
  site's main branch publishes, so site pushes stay on explicit user request.
