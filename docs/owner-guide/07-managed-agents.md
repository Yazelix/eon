# 7. Managed agents: eligibility before execution

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

Eon uses **rota**, its managed-agent core, to decide which configured backend
may run. Here a **backend** means a configured coding-agent provider/command;
**quota** means its reported usage allowance. rota's eligibility code considers
whether the backend is enabled, whether its readings have a problem, its
running-session cap and its quota limits
([decision.rs:28](../../eon/crates/eon-agent/src/decision.rs)).

Eon hosts rota's standalone core as `eon agent`. The host supplies naming,
configuration and a session-facts reader; the core owns the behavior behind
that interface. Its crate builds a library named `rota` and a standalone
binary of that name without Eon-crate dependencies
([cli.rs:8](../../eon/crates/eon-agent/src/cli.rs),
[eon-agent/Cargo.toml:25](../../eon/crates/eon-agent/Cargo.toml),
[docs/AGENT-RULES.md:286](../AGENT-RULES.md)).

Keep its delivered scope precise. EON-C40's amendments say the first delivery
keeps rota's moved command behavior while later clean-port targets remain
deferred. Linux Agent popups with an agent configuration use managed launch;
without that configuration, and on macOS, the built-in path remains. This is
not evidence that the entire later contract target has shipped. EON-C40's
overall status is Planned, and its runtime proof remains pending
([docs/CONTRACTS.md:6968](../CONTRACTS.md),
[docs/USAGE.md:128](../USAGE.md),
[docs/CONTRACTS.md:6957](../CONTRACTS.md),
[docs/CONTRACTS.md:6966](../CONTRACTS.md)).

## Run it

From the repository root, in Nushell. Read the decision and the host interface;
do not launch a backend for this exercise:

```nu
^nl -ba eon/crates/eon-agent/src/decision.rs | ^sed -n '28,67p'
^nl -ba eon/crates/eon-agent/src/cli.rs | ^sed -n '8,25p'
```

## What to look at

Find the explicit refusal `quota unknown (not explicitly allowed)`. The unknown
case can pass only through a configured allowance. Find `session` in `Host`:
the embedded core receives facts through an interface instead of depending
on an Eon crate for them
([decision.rs:42](../../eon/crates/eon-agent/src/decision.rs),
[cli.rs:23](../../eon/crates/eon-agent/src/cli.rs)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [decision.rs:28-67](../../eon/crates/eon-agent/src/decision.rs):

- Line 28 takes a backend, a reading and a role, such as implement or review.
  Success is `Ok(())`, meaning “eligible at this check.”
- Lines 29–34 refuse disabled backends or propagate a reading problem.
- Lines 35–41 refuse when a configured cap is reached. The comparison is
  greater-than-or-equal, so the cap itself is already busy.
- Lines 42–48 handle unknown quota. Configuration can permit it; omission
  refuses rather than inventing a percentage.
- Lines 49–54 distinguish an empty quota map and the allowances for unknown
  or missing values.
- Lines 55–59 apply role-specific limits when configured and return success.
  This function checks availability; it does not itself select or launch the
  winning backend.
- Lines 61–67 compare percentages with a tiny floating-point tolerance.
  Complementary percentages can differ at a decimal cutoff because computers
  cannot represent all decimals exactly. Outside that tolerance, use the
  number's normal total ordering.

Chapter 5 explained workspace action parity. That lets an agent address the
workspace through typed commands; this eligibility function answers the separate
question of whether a configured coding backend may run
([cli/parity.rs:1](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[decision.rs:28](../../eon/crates/eon-agent/src/decision.rs)).

</details>
<!-- under-the-hood:end -->

## The hardest question

> “What does Eon add beyond opening an agent in another terminal?”

Point to the pieces you can defend: persistent Orbit terminals, a typed
workspace CLI, reported attention and shared managed-backend eligibility. They
connect work and observation in one workspace. This source does not establish
better model answers or a measured advantage over a competing product
([orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md),
[cli/parity.rs:1](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[attention.rs:1](../../runtime/crates/eon-runtime/src/attention.rs),
[decision.rs:28](../../eon/crates/eon-agent/src/decision.rs)).

Those mechanisms do not upgrade the managed-agents contract's Planned status
or resolve its pending runtime proof
([docs/CONTRACTS.md:6957](../CONTRACTS.md),
[docs/CONTRACTS.md:6966](../CONTRACTS.md)).

Eligibility is also not a security attestation. The documented backend name
does not attest its permissions, and the headless run path has Linux/systemd
requirements. Chapter 9 explains the host-authority risk
([docs/USAGE.md:113](../USAGE.md), [docs/USAGE.md:120](../USAGE.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

Explain the difference between “backend unavailable,” “quota unknown,” and
“backend eligible.” Then say which other code would still need to choose and
launch it; eligibility alone has not started anything.
<!-- owner-only:end -->

## Going deeper

- [Moved behavior and target scope, EON-C40, docs/CONTRACTS.md:6955](../CONTRACTS.md).
- [Agent command scope, docs/USAGE.md:88](../USAGE.md).
- Next: [updates and live work](08-updates-and-live-work.md).
