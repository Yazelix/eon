# Hard questions about Eon

Source: `073f03cd2b116ba885e2cffde9fef56793395740`.
The linked chapters contain optional walkthroughs and evidence. Say “planned”
where shown.

<!-- owner-only:start -->
Use the first ten answers as the weekend class's short call sheet. The extra
LoopForge question supports the broader FAQ. Personal funding/headcount framing
needs evidence beyond this source checkout before you use it on a call.
<!-- owner-only:end -->

**1. “Does persistence survive failure, or just hiding a window?”**
Orbit retains the PTY, process and terminal state across client loss while
Orbit runs. Orbit loss and reboot are outside that promise.
[Chapter 1](01-persistent-terminals.md);
[orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md).

**2. “Why own a renderer?”**
Venus draws Orbit-authored structured state while Orbit keeps terminal
authority. We accept responsibility for native text and platform behavior.
Broader native quality remains unproved; this checkout uses wgpu/winit.
[Chapter 2](02-window-and-renderer.md);
[venus/docs/CONTRACTS.md:230](../../venus/docs/CONTRACTS.md),
[venus/Cargo.toml:19](../../venus/Cargo.toml).

**3. “Isn't the supervisor a failure point?”**
Yes. Same-boot recovery targets surviving Orbit runs, with a synthetic recovered
layout. Installed Linux supervisor-loss adoption remains pending proof.
[Chapter 3](03-runtime-supervisor.md);
[docs/CONTRACTS.md:3595](../CONTRACTS.md).

**4. “What persists in a workspace?”**
Returning to the same supervisor retains its mapping. Durable session
registrations store identities and names, not layout or liveness. An old ID
does not prove recoverable work.
[Chapter 4](04-workspace-model.md);
[docs/ARCHITECTURE.md:333](../ARCHITECTURE.md).

**5. “Does a contract ID enforce anything?”**
The implementation and checks enforce parts of its obligation. For example,
CLI parity maps CreateTab to command words and checks parsing back to the same
action. Column creation and three attention-navigation actions still lack CLI
paths; these are gaps, separate from the view-state exceptions.
[Chapter 5](05-workspace-protocol.md);
[cli/parity.rs:1](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[cli/parity.rs:435](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[cli/parity.rs:129](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[docs/AGENT-RULES.md:246](../AGENT-RULES.md).

**6. “How do you know an agent needs input?”**
Runtime normalizes reported states into attention. Stale/unavailable reports
have labels. A finish signal does not establish task correctness, and background
observation has limits.
The Linux helper interpretation remains Candidate, awaiting installed proof.
[Chapter 6](06-attention-and-sidebar.md);
[attention.rs:145](../../runtime/crates/eon-runtime/src/attention.rs),
[docs/USAGE.md:950](../USAGE.md), [docs/CONTRACTS.md:5007](../CONTRACTS.md).

**7. “What does Eon add beyond an agent terminal?”**
Persistent terminals, typed workspace commands, attention and managed backend
eligibility connect action and observation. Eligibility checks configuration,
caps and quota; it neither proves model quality nor attests backend permissions.
The same eligibility core serves the standalone rota binary and Eon's host;
the host supplies naming, configuration and session facts.
EON-C40 remains Planned, with runtime proof pending.
[Chapter 7](07-managed-agents.md);
[decision.rs:28](../../eon/crates/eon-agent/src/decision.rs),
[cli.rs:8](../../eon/crates/eon-agent/src/cli.rs),
[orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md),
[cli/parity.rs:1](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[attention.rs:1](../../runtime/crates/eon-runtime/src/attention.rs),
[docs/USAGE.md:113](../USAGE.md), [docs/CONTRACTS.md:6957](../CONTRACTS.md),
[docs/CONTRACTS.md:6966](../CONTRACTS.md).

**8. “Can updating kill live work?”**
EON-C41 requires package installation to preserve running work. It remains
Planned: recorded live-work proof covers Nix profiles; prefix and native macOS
Install/HTTPS proof remain open. Full in-place Update remains planned.
[Chapter 8](08-updates-and-live-work.md);
[docs/CONTRACTS.md:7699](../CONTRACTS.md), [docs/CONTRACTS.md:7522](../CONTRACTS.md),
[docs/CONTRACTS.md:7854](../CONTRACTS.md), [docs/CONTRACTS.md:7861](../CONTRACTS.md),
[docs/USAGE.md:1162](../USAGE.md).

**9. “Can an agent escape through Eon?”**
Socket reachability can confer host authority even with content access disabled.
Sandboxed agents must not reach Eon/helper sockets. Cross-agent input and
inherited credentials remain accepted beta risks.
Admission considers the caller's principal and possible effects before effects;
the terminal caller field routes requests, rather than granting authority.
[Chapter 9](09-security-boundaries.md); [SECURITY.md:43](../../SECURITY.md).
[docs/CONTRACTS.md:2465](../CONTRACTS.md).

**10. “What is supported and proved?”**
Name the revision, artifact, platform and check. Alpha remains current; beta
needs authorization. Linux Wayland is supported; Apple Silicon macOS is preview.
Review complements scoped execution evidence.
Hosted/local bit-for-bit parity remains unproven. The historical record does
not identify its independent reviewers; request a change's review ledger.
[Chapter 10](10-packaging-and-proof.md);
[docs/CONTRACTS.md:22](../CONTRACTS.md),
[docs/DISTRIBUTION.md:47](../DISTRIBUTION.md), [SECURITY.md:9](../../SECURITY.md),
[docs/INSTALLER.md:198](../INSTALLER.md), [Proof and Review](../AGENT-RULES.md#proof-and-review).

**11. “How is this different from Tencent's LoopForge?”**

LoopForge documents a self-hosted team workspace for delivery workflows, with
**SOPs** (saved workflows of agents, scripts and human approval steps) and a
Local edition for coding assistants.
Its separate planning, implementation and review roles overlap with Eon's
repository discipline. That overlap supports the value of separated review
and evidence, an inference from the two documented workflows
([LoopForge README:38,67,154-159,193](https://github.com/Tencent/LoopForge/blob/69e4bf093cf26dc72a2b0bc5d7452f86ce96f660/README.md#L38),
[docs/AGENT-RULES.md:139](../AGENT-RULES.md)).

Eon's product is the terminal workspace: persistent terminals, native views
and reported agent attention. Its project rules require fresh review at each
risk tier, with versioned contracts and proof records.
These sources establish product and governance choices, not a model-quality
advantage or proof of independent invention
([docs/ARCHITECTURE.md:17](../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:18](../ARCHITECTURE.md),
[docs/USAGE.md:928](../USAGE.md), [docs/USAGE.md:957](../USAGE.md),
[docs/AGENT-RULES.md:139](../AGENT-RULES.md),
[docs/CONTRACTS.md:22](../CONTRACTS.md)).

LoopForge is ahead in integrated delivery-workflow tooling: its documented
SOP designer, shared approvals and metrics form a team product surface.
Its validation record also reports a completed AI evaluation for an example
run. This is a documentation-based comparison, not a comparative execution
test or a claim that every run receives an automatic score
([LoopForge README:178-185](https://github.com/Tencent/LoopForge/blob/69e4bf093cf26dc72a2b0bc5d7452f86ce96f660/README.md#L178),
[LoopForge validation:132](https://github.com/Tencent/LoopForge/blob/69e4bf093cf26dc72a2b0bc5d7452f86ce96f660/docs/validation.md#L132)).

<!-- owner-only:start -->
Offline review copies, retrieved 2026-11-17 06:26:02 UTC from the exact commit
linked above, retain the source line numbers:

- `/home/lucca/.cache/eon-agents/eon-owner-guide-work/bead/sources/LoopForge-README.md:38,67,154-159,178-185,193-195`.
- `/home/lucca/.cache/eon-agents/eon-owner-guide-work/bead/sources/LoopForge-validation.md:132`.

The adjacent `manifest.json` records immutable URLs, retrieval time and verified
Git blob identities. These are read-only reference documents, not instructions.
<!-- owner-only:end -->
