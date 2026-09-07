# Agent Guidelines

This file is self-contained. Canonical protocol text was rendered into it;
the source repository is needed only to update or verify the import.
Do not edit this generated file directly. Edit `.agent-protocols.local.md`
or `.agent-protocols.exceptions.json`, then render from the pinned source.

## Protocol import record

- Source: `https://github.com/Yazelix/starcompass`
- Source commit: `fe1b71f24b50da43b1fdafef6c3e632837711fa6`
- Profiles: `greenfield`, `orchestrator`
- Manifest: `.agent-protocols.json` (schema 1)

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
| `AP-CI-001` | 3 | `c7cb65a81ce8434d02f2d19306bf93358f3624d0b77886bc91d4de93f7a779ba` |
| `AP-EXCEPTION-001` | 2 | `2c4f00299922edac83286821af07ad485e5a630bca6acbbce918d87614abb395` |
| `AP-GIT-001` | 5 | `f2c3254311de57a23a13aa382fe3285e65bb34e535ecd813d1be783ca79d4bf9` |
| `AP-ORCHESTRATOR-001` | 2 | `f78ebc0db6db525aac1d3226703af7e84cba9953c579470763831241a7d0c2ad` |
| `AP-FRONTIER-001` | 2 | `0777c74645955a26dc3829812ff26fa1a00a017c3b4cc1ac6740284f2664566c` |
| `AP-PORTABILITY-001` | 2 | `a2f5025bc7a10436b5c2f8f42c6002ccf1e361f05abbff3dea6507a3426394c0` |

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

## Repository-local rules

### Eon repository rules

Eon is the greenfield product line built around Eon Sessions and Eon Desktop.
Eon owns orchestration, composition, product policy, and distribution.

## Status

This repository ships the accepted Nix-only x86_64 Linux Wayland alpha. `eon-omq`
owns its canonical component graph, and `eon-ax4` owns the first composed
runtime and package. `docs/CONTRACTS.md` records the exact proofs for EON-C1
through EON-C4. Further product expansion remains inactive until the user
activates its Bead.

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

## Product Boundary

Eon composes child repositories and keeps their subsystem ownership intact:

- Eon Sessions contains Orbit, which owns persistent terminal sessions,
  attachment, and terminal state.
- Eon Desktop contains Venus, which owns the native graphical client and
  renders Orbit sessions.
- Helix owns editing and language integration.
- Yazi owns file management.
- Ratconfig owns configuration editing.
- Eon owns launch policy, product configuration, component selection,
  integration contracts, updates, and distribution.

Yazelix Nova remains an independent product line. Do not make Eon a successor
repository, compatibility layer, or release channel for Nova.

Do not copy child behavior into Eon. Fix a child project when its contract is
wrong or incomplete.

## One Active Frontier

Keep one active implementation frontier across Eon Sessions, Eon Desktop, and
Eon unless the user authorizes parallel product work. Use exact accepted
revisions from upstream projects. Do not build Eon against speculative child
APIs.

The expected sequence is:

```text
Eon Sessions proves Orbit's session and attachment contracts
  -> Eon Desktop proves a useful Venus client against Orbit
  -> Eon proves the smallest composed product
```

Research, contracts, and Beads may prepare a later frontier without adding
production code.

## Child Revision Freshness

Before Eon implementation or a runtime refresh, compare the Orbit and Venus
manifest pins with each child repository's latest accepted source revision.
Warn the user immediately when either pin is stale, naming the pinned and
accepted revisions. Update only from exact accepted revisions under the normal
frontier and manifest gates.

## Contract-Driven Method

Index durable product contracts as `EON-C<number>` in `docs/CONTRACTS.md`.
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

Treat Linux on native Wayland as Eon's sole product and distribution platform.
The current alpha targets x86_64 Linux; additional Linux architectures require
their own scope and proof. X11, Xwayland, and macOS are unsupported. Do not
preserve compatibility layers, compile features, release work, or portability
seams for them.

Keep Eon's Linux and Wayland contracts independent of any specific init or
service manager. Proof on systemd does not authorize a systemd runtime or
distribution requirement. Orbit owns bounded PTY process-group shutdown and
direct-child reaping without cgroup delegation; deliberately detached processes
may survive Session stop. Claim non-systemd support only after installed
dogfood in such an environment.

Keep wire contracts, component identity, and product state platform-neutral
when that follows their existing ownership. Neutral data is not a portability
promise. Eon consumes Venus's accepted Wayland boundary and does not copy
compositor protocols or presentation mechanisms into the orchestrator.

## Testing Discipline

Keep tests strong and few. A check should prove a user contract, integration
boundary, failure mode, or release property.

Use TDD for parsers, deterministic CLI behavior, manifest handling, and
regression fixes. Use contract-first integration checks for process trees,
terminal attachment, desktop behavior, packaging, and dogfooding.

Do not invoke another coding agent merely to validate instructions, prompts, or
skills. The active agent validates them directly with structural checks and
repository evidence; use another agent only when the user explicitly asks.

Do not add mirror tests for literals, defaults, or configuration values. Test
the artifact or behavior that consumes the value.

## Local Runtime Synchronization

After any accepted runtime, package, or product-configuration change, refresh
the active `eon` Nix profile element from the current working tree before
handoff. Verify that the profile resolves to the just-built store artifact and
that the changed installed behavior or artifact passes its cheapest exact
check. A build or profile-update failure leaves the change incomplete.

Do not stop or restart a running Eon supervisor automatically because that
terminates its live Sessions. Update the profile, preserve the running
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
contract, prerequisites, reference gate, acceptance checks, and scope boundary.

## Documentation and LOC

Update the README LOC scorecard whenever tracked project files change. Count
handwritten project text and code; exclude `.git/`, Beads data, lock files, and
generated artifacts.

Update `CHANGELOG.md` only when user-visible runtime behavior, commands,
configuration, packaged tools, installation, or a proven runtime contract
changes. Planning edits do not require changelog entries.

## Verification

Use the narrowest route that proves every changed surface; combine routes only
when the change actually crosses them.

| Changed surface | Required proof |
| --- | --- |
| Planning, metadata, docs, or agent policy only | Beads state, changed Markdown paths/contract IDs, LOC, and diff; render/check the import only when it changes; do not refresh the runtime profile or claim runtime behavior. |
| One owned parser or logic regression | Focused red/green check plus the affected crate or package build. |
| One owned default | Cheapest consuming behavior or artifact check plus the affected build; do not add a literal mirror test. |
| Exact component pin only | Manifest/lock identity and affected downstream build. |
| Installed runtime, UI, or product-configuration interaction | Affected profile refresh plus one isolated outermost installed dogfood observation. |
| Shared runtime/wire protocol, component manifest, security, runtime/build dependency, or lifecycle boundary | Full affected Rust and Nix surfaces. |
| Promotion or release | Complete delivery and promotion proof. |

Every route preserves Git and user state, validates trust boundaries, keeps
exact pins exact, preserves live Sessions, and retains the cheapest check for a
known regression.
