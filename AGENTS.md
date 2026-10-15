# Agent Guidelines

This file is self-contained. Canonical protocol text was rendered into it;
the source repository is needed only to update or verify the import.
Do not edit this generated file directly. Edit `.agent-protocols.local.md`
or `.agent-protocols.exceptions.json`, then render from the pinned source.

## Protocol import record

- Source: `https://github.com/Yazelix/starcompass`
- Source commit: `4af5a3265acd425d1359f56f805221fc264a9e1a`
- Profiles: `greenfield`, `orchestrator`, `terminal-engine`
- Manifest: `.agent-protocols.json` (schema 1)
- Directory rules: `orbit/`, `runtime/`, `venus/`; read a directory's `AGENTS.md` before changing files under it.

| Protocol | Version | SHA-256 |
| --- | ---: | --- |
| `AP-SCOPE-001` | 3 | `cae30f9031beea4f831a89445f35807f50618e0f2cb0432f3b94bec40da9aeef` |
| `AP-CONTRACT-001` | 4 | `cdf2e5d69cefd34ceeeaa6b7511b21f921e2be50c17d3c01b85120b20234cff7` |
| `AP-REFERENCE-001` | 3 | `186fa74aa530c9c76ae507685461dff2b1506ecb5087230ded51f02cecb9231a` |
| `AP-MINIMAL-001` | 4 | `612bc9c62a20adf7332724d4663af0a0e591d624de2a76b1c257a9ed428db1f6` |
| `AP-DEPENDENCY-001` | 2 | `ded49add538b82de0a9c522bc8a34720f4ebbb47f39fc2f7ddb25e7aad1700d3` |
| `AP-OWNERSHIP-001` | 2 | `d218a4e0625b659ec366284110bdfce02bd66cb229ab6ba07f2317092ea13053` |
| `AP-TEST-001` | 3 | `58b5837cb679e958192b366bb15d6e34649f5a91ff9f4accdc7edb4ef5cdb873` |
| `AP-PROOF-001` | 6 | `01fc3712fd369189b934b0503173e6a8963be64b7f18869abfaef3cca53866f7` |
| `AP-PLAN-001` | 11 | `fcb7c8d2b99c6641a7cd1244072eda6230f1ccfc3a4aa47c29cf2525fcb1376d` |
| `AP-SESSION-001` | 1 | `f8bdac637027895ec8d1ad4f192e13808d095a809d091b76eadadaf936b2a340` |
| `AP-CI-001` | 3 | `c7cb65a81ce8434d02f2d19306bf93358f3624d0b77886bc91d4de93f7a779ba` |
| `AP-EXCEPTION-001` | 2 | `2c4f00299922edac83286821af07ad485e5a630bca6acbbce918d87614abb395` |
| `AP-GIT-001` | 5 | `f2c3254311de57a23a13aa382fe3285e65bb34e535ecd813d1be783ca79d4bf9` |
| `AP-ORCHESTRATOR-001` | 2 | `f78ebc0db6db525aac1d3226703af7e84cba9953c579470763831241a7d0c2ad` |
| `AP-FRONTIER-001` | 2 | `0777c74645955a26dc3829812ff26fa1a00a017c3b4cc1ac6740284f2664566c` |
| `AP-PORTABILITY-001` | 2 | `a2f5025bc7a10436b5c2f8f42c6002ccf1e361f05abbff3dea6507a3426394c0` |
| `AP-TERMINAL-001` | 2 | `c1805e12ca5c41d3ed2a6b24739fcded2ad986758fda01fdfa3132c9f8351d9c` |
| `AP-FAILURE-001` | 4 | `450e48b4e1532ff50a9a1da7faa49e3539450e0676f3c530f888d33b4b752e80` |

### Local exceptions

- `AP-TEST-001` in Agent-instruction, prompt, and skill changes in the Eon repository: The user requires the active agent to reuse its existing context instead of invoking another agent for validation.
  Approved by user on 2026-08-09; review condition: Review only if the user explicitly reauthorizes isolated agent trials.

## Canonical protocols

### AP-SCOPE-001 — User-owned scope

The user owns scope. Inspection, audit, diagnosis, explanation, and
recommendation authorize no implementation, external-state, or durable-planning
write. Implement only a chosen outcome.

Do not silently add features, compatibility promises, public surfaces,
migrations, repositories, or planning items. Preserve user-owned inputs and
artifacts; replace or delete an exact target only when the outcome requires it.
Otherwise write a distinct result. State consequential assumptions and stop
when a missing scope choice would materially change the result. “Finish” adds
persistence, not authority. Report out-of-scope findings unless the user chose
a durable destination.

### AP-CONTRACT-001 — Contract-driven changes

Before code shape, state the smallest observable contract: consumer, trigger,
result, and important failures. Of the contracts consistent with the request
and evidence, choose the fewest unsupported guarantees or restrictions; leave
reasonable future behavior unspecified. Give it a stable ID only when later
consumers need one.

Keep docs, help, examples, and configuration aligned with current commands,
paths, flags, defaults, and availability; label planned, partial, or gated
behavior. Choose one source-of-truth owner, the cheapest falsifying check, and
the smallest complete vertical slice. Update the contract first for an
intentional behavior change. Implementation details are not contracts unless a
component must rely on them.

### AP-REFERENCE-001 — Evidence before code shape

Before a consequential code shape, read affected instructions, code, contracts,
tests, and required subsystem references. Record the adopted, rejected, or
unresolved mechanism; separate evidence from inference and revisit it after a
material shape change. Memory, summaries, and reputation are discovery only.

Apply source-license terms to their exact actors, uses, conditions,
beneficiaries, and direction. Choosing a provider does not make a user act for
it; an “including” example remains scoped by its condition. Inspection is
distinct from copying, adaptation, redistribution, dependency selection, and
incorporation, so a restriction on one does not spread to another. If an
interpretation would block required evidence, identify the exact clause and
roles and resolve material ambiguity with the user. Review does not require
reuse.

### AP-MINIMAL-001 — Minimum sufficient implementation

After identifying the contract, evidence, owner, flow, and boundaries, take the
first sufficient option: no change; existing owner, helper, or pattern;
standard library or native platform; accepted dependency that owns the
behavior; minimum correct local code.

Judge the whole lifecycle, including duplicate truth, coordination, coupling,
migration and removal, portability, operations, proof, and agent context. Scope
instructions narrowly; retain non-obvious constraints and reusable
behavior-changing workflows, not generic or duplicated policy. Load details
only when needed.

A patch is not minimal if it preserves a wrong or duplicate owner, treats a
symptom below its shared cause, bypasses a boundary, or raises downstream cost.
Prefer deletion, direct ownership, and fewer files; use patch size only between
equally correct system shapes. Never remove required behavior, trust-boundary
validation, data-loss protection, security, accessibility, or the cheapest
runnable check for non-trivial logic.

When available, use upstream
[Ponytail](https://github.com/DietrichGebert/ponytail/tree/16f29800fd2681bdf24f3eb4ccffe38be3baec6b)
as a fallible code-shape bias after these constraints. Its absence does not
suspend them, and its hooks do not prove compliance.

### AP-DEPENDENCY-001 — Dependency gate

Before adding a dependency, name its capability and contract. Compare owned
code, the standard library, and credible candidates for correctness,
maintenance, platform and license fit, transitive weight, API stability, and
net complexity. Record the choice, meaningful rejections, and replacement cost;
pin it and prove the relied-on behavior. Remove it when its ownership no longer
justifies its cost.

### AP-OWNERSHIP-001 — One owner per invariant

Give every invariant, state transition, and user-visible policy one owner;
consumers must not reconstruct or reinterpret it. Name the owner before adapters
or synchronization and delete duplicates. Put policy at the highest layer with
enough context and enforcement at the lowest correct layer. Make cross-boundary
data explicit and versioned for independently released consumers.

### AP-TEST-001 — Strong and few tests

Protect meaningful contracts, regressions, boundaries, and failures with a few
strong tests. Use TDD for deterministic helpers, parsers, protocol behavior,
and regressions whose expected behavior is known first; use contract-first
integration checks for layout, runtime, architecture, forks, and dogfood.

For consequential agent-instruction, prompt, or skill changes, run isolated
representative tasks without supplying the expected conclusion. Structural
checks prove structure, not agent behavior. Test observable effects; delete
duplicate proof, implementation trivia, and scaffolding. Guard absence only
when absence is a security, licensing, size, ownership, or known-regression
contract.

### AP-PROOF-001 — Explicit proof lifecycle

Proof records revision, command/observation, environment, result/surface,
and phase: proposed, implemented, mechanically verified, dogfooded, accepted,
or promoted. Never widen/reuse after material change.

Use the highest boundary exercising the changed contract. Add broader/lower
suites only for changed owner, failure, trust, distinct delivery, or
promotion—not availability, issue text, or prior phase. Exact downstream proof
subsumes lower checks only for the identity/behavior exercised.

Run a complete suite once per unchanged candidate/environment. Metadata/docs/
planning follow-ups reuse it; commands are evidence, not a checklist. Keep logs
only when reproduction is costly, disputed, performance-sensitive, or lossily
summarized; temporary paths are not proof.

Performance claims need a measured bottleneck, fixed workload/environment,
baseline, oracle, and distribution/bound; retain constraining negatives. Review
changes require another pass; convergence needs a full clean pass.

### AP-PLAN-001 — Durable planning state

Keep later-needed outcomes/constraints in planning or canonical docs. Issues
hold chosen goals/decisions, material defects, or schedulable follow-ups;
methods stay with their owner. Acceptance names outcomes/cheapest falsifiers,
not command catalogs unless the boundary/transition needs them.

A run includes continuations until control returns; reads are unrestricted. A
writing run does exactly one:

1. Owns at most one issue and may create/claim/update/implement/close it.
2. With explicit user authorization, changes a named/accepted planning-only
   batch; claims nothing and changes no implementation.

Bind implementation before writes; return at completion, block, or handoff.
Report unapproved findings; create issues only for material out-of-scope or
schedulable work, deferred outside authorized batches.

Keep fields current; append only needed contracts, decisions, dependencies,
acceptance, material failures/negatives, constraining rejections, and approvals.
Baseline/reference comments record only changed identities, dirty state,
mechanism/stop decisions, and first check. Keep status/prerequisites honest; use
the issue tool, never its storage.

### AP-SESSION-001 — Session continuity across compaction

Treat context compaction as lossy. Write each user decision to its durable
owner when it is made, and reread the active issue after any compaction. Once
a session has compacted, start the next issue in a fresh session; within an
issue, a second compaction or any sign of lost context triggers a handoff
comment (state, next step, open decisions) and a new session. Prefer a
clean-break restart to compaction, and start a large issue fresh when most of
the context is already used.

### AP-CI-001 — Bounded continuous integration

Before hosted automation, name its contract and why local proof is
insufficient. Bound triggers, jobs, timeouts, permissions, artifacts, cache,
concurrency; specify fork and secret behavior. Prefer one cheap deterministic
job. Record when to expand, reduce, or remove. Confidence must justify its
financial, latency, security, and maintenance costs, including private minutes
and storage.

### AP-EXCEPTION-001 — Explicit local exceptions

Only an exception approved by the user or named authority may narrow, replace,
or suspend an import. Record protocol ID, exact scope, reason, approver, date,
and any expiry or review condition. Unrecorded conflicts are drift; additive
detail is an overlay.

### AP-GIT-001 — Safe repository history

Repository history is shared user state. Inspect status and local policy;
preserve unrelated or concurrent work and use the least destructive sufficient
operation.

Amend the unpublished task commit for same-task corrections; refresh downstream
commits and generated artifacts. After external reliance (push, promotion,
release, or pin), commit corrections separately. Never reset, discard,
force-push, rewrite published history, or create a branch without authority.
Verify intended diff before commit and remote state after push. Roll back with a
reviewed revert unless local policy says otherwise.

### AP-ORCHESTRATOR-001 — Thin orchestrator ownership

An orchestrator owns composition, lifecycle policy, compatibility selection,
and user defaults, not child mechanisms or state. Consume pinned child artifacts
through explicit contracts; keep translation narrow; reject copies, hidden
forks, and duplicate schemas. Expose startup, shutdown, update, and partial
failure policy, and test with substitutes when a child is unavailable.

### AP-FRONTIER-001 — One active integration frontier

When greenfield components integrate, keep one boundary under active
architectural change when practical. Stabilize the others with released
artifacts, fixtures, adapters, or interchangeable projects. Dogfood the active
frontier through a real vertical slice and move it only after contract and
failure evidence. Parallel work may continue behind stable integration
contracts.

### AP-PORTABILITY-001 — Portable core with explicit platform seams

Keep multi-OS contracts and state neutral; isolate system APIs, processes,
paths, packaging, and events. Evaluate targets before foundational runtime,
graphics, filesystem, or transport choices. Prove behavior on each platform,
not by compilation; name unsupported targets.

### AP-TERMINAL-001 — Terminal state authority

One component owns terminal parsing, grid, modes, scrollback, selection,
graphics, title, and process attachment; others consume versioned projections,
never a second emulator. Define replication as raw bytes, checkpoints plus
tails, structured frames, or another contract. Cover reconnect, resize,
alternate screen, partial input, and slow consumers; test semantic attachment
equivalence, not visuals alone.

### AP-FAILURE-001 — Designed lifecycle and failure semantics

Long-lived sessions and services define start, attach/detach, crash/restart,
shutdown, version mismatch, abandonment; bound queues, memory, retries,
timeouts, and slow-consumer impact. Own cancellation and idempotent cleanup;
persist only required recovery state. Test deterministic failures when
practical; dogfood other process/platform cases.

## Repository-local rules

### Eon repository rules

This repository holds Eon and the subsystems it composes, built as one Cargo
workspace:

- `orbit/`: Orbit, which owns persistent terminals, attachment, and terminal
  state.
- `venus/`: Venus, the native graphical client that renders Orbit terminals.
- `runtime/`: Eon Runtime, which owns runtime mechanisms and canonical EONW.
- `eon/` and the repository root: Eon's product crates, Nix composition,
  product documentation, and distribution.

## Status

This repository ships the accepted Nix-only x86_64 Linux Wayland alpha. `eon-omq`
owns its canonical component graph, and `eon-ax4` owns the first composed
runtime and package. On 2026-09-11 the user activated a Nix-only Apple Silicon
macOS expansion under `EON-C7` and
`eon-prove-full-eon-apple-silicon-macos-t8o`. The required order is Orbit
`ORB-C14`, Venus `VEN-C16`, then full Eon. macOS remains unsupported until all
three native proofs are accepted.

## Implementation Languages

Rust owns durable Eon product behavior and repository tooling. Nix owns package
resolution and composition; shell is limited to irreducible process glue.
Python may support disposable investigation but must not become checked-in Eon
policy. Add durable Python, Go, TypeScript, or another implementation language
only after an explicit user decision that names the subsystem advantage and
accepts the added toolchain and ownership cost.

## Core Rule

The user decides scope. Do not create a feature, compatibility surface, module,
repository, or planning bead until the user chooses that direction.

## Desktop Focus

Preserve the user's active desktop focus. Prefer background, isolated, or
non-activating surfaces. Take focus only when no workable alternative exists
or alternatives give materially worse results; explain why before doing so.

## Product Boundary

Eon composes its subsystems and external tools and keeps their ownership intact:

- Orbit owns persistent terminals, attachment, and terminal state.
- Venus owns the native graphical client and renders Orbit terminals.
- Eon Runtime owns the runtime mechanisms and canonical EONW.
- Helix owns editing and language integration.
- Yazi owns file management.
- Ratconfig owns configuration editing.
- Eon owns launch policy, product configuration, component selection,
  integration contracts, updates, and distribution.

Among workspace crates, Venus and Eon Runtime depend only on the protocol crates
`orbit-protocol` and `eon-workspace-protocol`, Eon's crates depend on Eon
Runtime and the protocol crates, and only Orbit depends on `libghostty-vt`. The protocol crates stay standalone, with
no workspace-inherited settings, so projects outside this repository can use
them.

Yazelix Nova remains an independent product line. Do not make Eon a successor
repository, compatibility layer, or release channel for Nova.

Do not copy one owner's behavior into another. Fix the owning subsystem when its
contract is wrong or incomplete.

Use workspace, tab, pane and terminal in product text as defined in
`docs/ARCHITECTURE.md#naming-boundary`. Technical session identities, JSON fields
and protocol names retain their spellings.

## Contract-Driven Method

Index durable contracts with their owner: `EON-C<number>` in
`docs/CONTRACTS.md`, `ORB-C<number>` in `orbit/docs/CONTRACTS.md`, and
`VEN-C<number>` in `venus/docs/CONTRACTS.md`. Never renumber or reuse an ID.
Stop for user approval before adding, changing, replacing, or retiring a
contract.

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

Other changes, including presentation-only changes, need the cheapest check that
exercises them; for installed UI that is one dogfood observation. Do not use a
reference as authority outside the problem it demonstrates.

Do not mark a contract `Proven` from a document or mock. Its proof record cites
the check, the exact repository and external component revisions, the platform,
and the artifact.

## Proof and Review

- **Batch proof.** Iterate freely within a slice. Intermediate rounds produce
  candidate observations, not accepted proof. Run the installed proof once, on
  the final revision of the accepted batch.
- **One commit per change.** Run checks on the exact tree being committed and
  record the proof (check, environment, result) in the owning bead within that
  commit; the commit is the proof's revision. Do not add follow-up commits
  whose only purpose is naming the source commit. Edit a contract index only
  when a contract, its checks, status, or gap changes. Existing indexed proofs
  stay valid as written.
- **Review loop.** After each slice, run a fresh-eyes and simplification pass in
  a fresh context and fix what it finds. Repeat until a full pass on the final
  revision finds nothing actionable; any fix resets that. Simplification never
  loses demonstrated regression coverage. Record one ledger line per slice in
  its bead: pass count and findings.
- **Autonomous review.** Run the loop in read-only review subagents without
  waiting for the user. Each reviewer reads the owning bead itself, not a
  paraphrase, plus the diff, and reports concrete failures with `file:line` or
  says that nothing is actionable. Keep briefs neutral: never mention earlier
  passes or passing checks. Exclude only gaps the bead records as accepted with
  a reason, and let a reviewer flag one anyway if it judges it more severe. The
  implementing agent fixes or records each finding and asks the user only when
  a finding needs a scope, contract, or other user-owned decision. Keep design
  decisions in the main session; delegate reading-heavy review, search, and
  investigation.

## Composition and Distribution

Use Nix as the sole installation and composition channel during Eon alpha and
early dogfood. Keep the first path narrow enough to improve the product without
maintaining installers, portable archives, signing, or a package matrix.

Keep runtime ownership distribution-neutral. Nix may supply store paths as
opaque component launch inputs, but Eon code must not construct store paths,
persist them as component identity, query the Nix store, or invoke a Nix
evaluator during normal operation.

Use a versioned component manifest as the canonical version and compatibility
graph. Nix consumes that graph during alpha. Direct bundles and other channels
must consume the same graph after the user approves distribution graduation.
Do not implement a second composition graph for any channel.

Direct release bundles remain the long-term primary adoption path. Begin that
work only after sustained Nix dogfood proves the product contract and the user
activates the distribution-graduation bead.

`yazelix.com` may host documentation and installer entry points. Immutable
release assets, checksums, signatures, and attestations remain the artifact
authority after direct distribution begins.

## Platform Discipline

Treat x86_64 Linux on native Wayland as Eon's only proved product and
distribution platform. `aarch64-darwin` is the only approved expansion and
remains Nix-only during initial proof. Additional Linux architectures, X11,
Xwayland, Intel macOS, signing, notarization, direct bundles, and other release
channels require separate scope and proof. Keep platform mechanics at the
existing Orbit, Venus, Eon runtime, and Nix composition owners; do not add a
second component graph or generic compatibility framework.

Keep Eon's Linux and Wayland contracts independent of any specific init or
service manager. Proof on systemd does not authorize a systemd runtime or
distribution requirement. Orbit owns bounded PTY process-group shutdown and
direct-child reaping without cgroup delegation; deliberately detached processes
may survive terminal stop. Claim non-systemd support only after installed
dogfood in such an environment.

Keep wire contracts, component identity, and product state platform-neutral
when that follows their existing ownership. Neutral data is not a portability
promise. Eon consumes Venus's accepted native-host boundary and does not copy
Wayland, Metal, application-lifecycle, or presentation mechanisms into the
orchestrator.

Each Orbit or Venus implementation bead records one portability disposition:
`neutral`, an isolated Linux, Wayland, or Darwin dependency, or a `platform
blocker`. A blocker names the assumption, affected contracts, replacement
shape, and removal cost, then stops for user choice before it enters a protocol
or durable format.

## Testing Discipline

Keep tests strong and few. A check should prove a user contract, integration
boundary, failure mode, or release property.

Before adding a test, name the meaningful failure it detects beyond existing
coverage and the supported project harness that runs it. Follow upstream test
conventions; do not ship ignored tests or bespoke harness scaffolding merely
to accompany a patch. If no suitable harness exists, retain a runnable
reproducer and red/green evidence with the issue and report the gap in
maintained coverage.

Use TDD for parsers, deterministic CLI behavior, manifest handling, and
regression fixes. Use contract-first integration checks for process trees,
terminal attachment, desktop behavior, packaging, and dogfooding.

Do not invoke another coding agent merely to validate instructions, prompts, or
skills. The active agent validates them directly with structural checks and
repository evidence; use another agent only when the user explicitly asks.

Do not add mirror tests for literals, defaults, or configuration values. Test
the artifact or behavior that consumes the value.

Run the Rust test suites through `nix develop -c eon-test`, never directly
inside a live Eon session's environment: they start supervisors and helper
processes. `nix/test-sandbox.nix` owns the isolation. The command passes any
arguments to `cargo test` and otherwise runs the workspace and installer
suites. A test that needs a real display, such as Venus's ignored live
regression, runs against its own isolated display as
`venus/docs/DEVELOPMENT.md` describes.

## Local Runtime Synchronization

After any accepted runtime, package, or product-configuration change, refresh
the active `eon` Nix profile element from the current working tree before
handoff. Verify that the profile resolves to the just-built store artifact and
that the changed installed behavior or artifact passes its cheapest exact
check. A build or profile-update failure leaves the change incomplete.

Do not stop or restart a running Eon supervisor automatically because that
terminates its live terminals. Update the profile, preserve the running
processes, and report that restart is required unless the user explicitly
authorizes interruption.

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
contract, prerequisites, acceptance checks, and scope boundary, and its
reference gate when the change is consequential.

## Documentation

Update `CHANGELOG.md` only when user-visible runtime behavior, commands,
configuration, packaged tools, installation, or a proven runtime contract
changes. `orbit/CHANGELOG.md` and `venus/CHANGELOG.md` record their accepted
wire, protocol, and contract changes. Planning edits do not require changelog
entries.

## Verification

Use the narrowest route that proves every changed surface; combine routes only
when the change actually crosses them.

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

`nix develop -c eon-test` runs every workspace suite and the installer's
separate static workspace.
Changes to Orbit or Venus crates also keep `cargo fmt --check` and
`cargo clippy --locked --all-targets -- -D warnings` green for the changed
crate.
