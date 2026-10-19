# Canonical Agent Protocols

The `AGENTS.md` that names this file imports these protocols and records
their source, versions, digests, and local exceptions. Do not edit this
generated file directly; render it from the pinned source.

## AP-SCOPE-001 — User-owned scope

The user owns scope. Inspection, audit, diagnosis, explanation, and advice
authorize no implementation, external-state, or durable-planning write.
Implement only a chosen outcome.

Never silently add features, migrations, repositories, or planning items.
Preserve user inputs and artifacts; replace or delete an exact target only when
required, else write a distinct result. State consequential assumptions; stop
when a missing scope choice would materially change the result. Recommend and
justify an answer to every choice. “Finish” adds persistence, not authority.
Report out-of-scope findings unless the user chose a durable home.

## AP-CONTRACT-001 — Contract-driven changes

Before code shape, state the smallest observable contract: consumer, trigger,
result, and important failures. Of the contracts consistent with the request
and evidence, choose the fewest unsupported guarantees or restrictions; leave
reasonable future behavior unspecified. Give it a stable ID only when later
consumers need one. Choose the cheapest check that would falsify it.

Keep docs, help, examples, and configuration aligned with current commands,
paths, flags, defaults, and availability; label planned, partial, or gated
behavior. Update the contract first for an intentional behavior change.
Implementation details are not contracts unless a component must rely on them.

## AP-REFERENCE-001 — Evidence before code shape

Before a consequential code shape, read affected instructions, code, contracts,
tests, and required subsystem references. Record the adopted, rejected, or
unresolved mechanism; separate evidence from inference and revisit it after a
material shape change. Memory, summaries, and reputation are discovery only.

## AP-MINIMAL-001 — Minimum sufficient implementation

After identifying the contract, evidence, owner, flow, and boundaries, take the
first sufficient option: no change; existing owner, helper, or pattern;
standard library or native platform; accepted dependency that owns the
behavior; minimum correct local code. Build only for current callers, not
forecasts; abstract only for a second caller: speculation costs every reader.

Judge the whole lifecycle, including coordination, coupling, migration and
removal, portability, operations, proof, and agent context. Scope instructions
narrowly; retain non-obvious constraints and reusable behavior-changing
workflows, not generic or duplicated policy. Load details only when needed.

A patch is not minimal if it treats a symptom below its shared cause, bypasses
a boundary, or raises downstream cost. Leftovers are a change's commonest
defect: update every caller, test, fixture, config, and export it breaks and
delete what it orphans. Prefer fewer files; report unrelated dead code as out of
scope; use patch size only between equally correct system shapes. Handle empty,
malformed, and boundary input; real callers send it. Never remove required
behavior, trust-boundary validation, data-loss protection, security,
accessibility, or the cheapest runnable check for non-trivial logic.

## AP-DEPENDENCY-001 — Dependency gate

Before adding a dependency, name its capability and contract. Compare owned
code, the standard library, and credible candidates for correctness,
maintenance, platform and license fit, transitive weight, API stability, and
net complexity. Record the choice, meaningful rejections, and replacement cost;
pin it and prove the relied-on behavior. Remove it when its ownership no longer
justifies its cost.

## AP-OWNERSHIP-001 — One owner per invariant

Give every invariant, state transition, and user-visible policy one owner;
consumers must not reconstruct or reinterpret it. Name the owner before adapters
or synchronization and delete duplicates. Put policy at the highest layer with
enough context and enforcement at the lowest correct layer. Make cross-boundary
data explicit and versioned for independently released consumers.

## AP-TEST-001 — Strong and few tests

Protect meaningful contracts, regressions, boundaries, and failures with a few
strong tests. Use TDD for deterministic helpers, parsers, protocol behavior,
and regressions whose expected behavior is known first; use contract-first
integration checks for layout, runtime, architecture, forks, and dogfood.

For consequential agent-instruction, prompt, or skill changes, run isolated
representative tasks without supplying the expected conclusion when the user
authorizes a separate agent; otherwise validate in the active context and
report agent behavior as unproven. Structural checks prove structure, not
agent behavior. Test observable effects; delete duplicate proof,
implementation trivia, and scaffolding. Guard absence only when absence is a
security, licensing, size, ownership, or known-regression contract.

## AP-PROOF-001 — Explicit proof lifecycle

Proof records the revision, command or observation, environment, and result
with its surface. Never widen or reuse proof after a material change.

Use the highest boundary that exercises the changed contract. Add broader or
lower suites only for a changed owner, failure, trust boundary, distinct
delivery, or promotion, not because a suite exists, an issue lists it, or it
ran before. Exact downstream proof covers lower checks only for the identity
and behavior it exercised.

Run a complete suite once per unchanged candidate and environment; metadata,
docs, and planning follow-ups reuse it. Commands are evidence, not a checklist.
Keep logs only when reproduction is costly, disputed, or performance-sensitive,
or a summary loses detail; a temporary path is not proof. Unattended runs and
waits close unused stdin, time out, check progress early, and never match
themselves as the target; stalls fail.

Finish every change to code, tests, or scripts with review by a fresh agent
that did not write it, since authors miss their own leftovers; in Claude Code,
use the converge skill. Docs-only and planning-only work skips it. A pass that
changes anything requires another; convergence needs a full clean pass. Nits
never block it, and text-only fixes restart a review at most twice per slice.

## AP-PLAN-001 — Durable planning state

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
the issue tool (beads_rust `br` when available), never its storage.

## AP-SESSION-001 — Session continuity across compaction

Compaction is lossy; avoid it. Write each user decision to its durable owner
when it is made. Delegate work whose raw output matters only as a conclusion,
such as broad searches, long logs and checks, or research, to a fresh
subagent; subagents are authorized unless the user or local rules say
otherwise.

Budget absolute context tokens, not window share; if the host reports none,
estimate high:

- From 400k, add no scope or new steps: close the issue if it can close by
  600k, else bring work in progress to a verified state and hand off.
- Past 600k, continue only to reach an imminent verified state or keep live
  state that cannot be written down; record why on the issue.
- Hand off before the host would compact. After any compaction, reread the
  issue and hand off.

A handoff is an issue comment: state, next step, and open decisions with
reasons. The next session primes, e.g. "prime, then continue <issue> from its
handoff".

## AP-EXCEPTION-001 — Explicit local exceptions

Only an exception approved by the user or named authority may narrow, replace,
or suspend an import. Record protocol ID, exact scope, reason, approver, date,
and any expiry or review condition. Unrecorded conflicts are drift; additive
detail is an overlay.

## AP-GIT-001 — Safe repository history

Repository history is shared user state. Inspect status and local policy;
preserve unrelated or concurrent work and use the least destructive sufficient
operation.

Amend the unpublished task commit for same-task corrections; refresh downstream
commits and generated artifacts. After external reliance (push, promotion,
release, or pin), commit corrections separately. Never reset, discard,
force-push, rewrite published history, or create a branch without authority.
Verify intended diff before commit and remote state after push. Roll back with a
reviewed revert unless local policy says otherwise.

## AP-CI-001 — Bounded continuous integration

Before hosted automation, name its contract and why local proof is
insufficient. Bound triggers, jobs, timeouts, permissions, artifacts, cache,
concurrency; specify fork and secret behavior. Prefer one cheap deterministic
job. Record when to expand, reduce, or remove. Confidence must justify its
financial, latency, security, and maintenance costs, including private minutes
and storage.

## AP-ORCHESTRATOR-001 — Thin orchestrator ownership

An orchestrator owns composition, lifecycle policy, compatibility selection,
and user defaults, not child mechanisms or state. Consume pinned child artifacts
through explicit contracts; keep translation narrow; reject copies, hidden
forks, and duplicate schemas. Expose startup, shutdown, update, and partial
failure policy, and test with substitutes when a child is unavailable.

## AP-FRONTIER-001 — One active integration frontier

When greenfield components integrate, keep one boundary under active
architectural change when practical. Stabilize the others with released
artifacts, fixtures, adapters, or interchangeable projects. Dogfood the active
frontier through a real vertical slice and move it only after contract and
failure evidence. Parallel work may continue behind stable integration
contracts.

## AP-PORTABILITY-001 — Portable core with explicit platform seams

Keep multi-OS contracts and state neutral; isolate system APIs, processes,
paths, packaging, and events. Evaluate targets before foundational runtime,
graphics, filesystem, or transport choices. Prove behavior on each platform,
not by compilation; name unsupported targets.

## AP-PERF-001 — Measured performance claims

Performance claims need a measured bottleneck, a fixed workload and
environment, a baseline, a correctness oracle, and a distribution or bound.
Keep negative results that constrain them.

## AP-TERMINAL-001 — Terminal state authority

One component owns terminal parsing, grid, modes, scrollback, selection,
graphics, title, and process attachment; others consume versioned projections,
never a second emulator. Define replication as raw bytes, checkpoints plus
tails, structured frames, or another contract. Cover reconnect, resize,
alternate screen, partial input, and slow consumers; test semantic attachment
equivalence, not visuals alone.

## AP-FAILURE-001 — Designed lifecycle and failure semantics

Long-lived sessions and services define start, attach/detach, crash/restart,
shutdown, version mismatch, abandonment; bound queues, memory, retries,
timeouts, and slow-consumer impact. Own cancellation and idempotent cleanup;
persist only required recovery state. Test deterministic failures when
practical; dogfood other process/platform cases.
