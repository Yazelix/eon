# 3. Runtime supervisor: who coordinates the work?

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

A **supervisor** coordinates the lifetime of other processes. Eon Runtime's
supervisor manages startup, presentation and child-lifecycle coordination.
Orbit owns terminal mechanics; Venus owns rendering. Separating these owners
lets Runtime treat a graphical-client exit as an event to handle rather than
an instruction to discard terminal state
([docs/ARCHITECTURE.md:283](../ARCHITECTURE.md),
[supervisor.rs:1611](../../runtime/crates/eon-runtime/src/supervisor.rs)).

A healthy window's Close action requests Stop; an unexpected window-process
exit does not itself make that request. The loop's “terminals remain active”
message covers presentation exit while Runtime is not stopping
([application.rs:1015](../../venus/src/application.rs),
[supervisor.rs:1611](../../runtime/crates/eon-runtime/src/supervisor.rs)).

A **generation** identifies a runtime lifetime. For full Eon, that lifetime ID
is independent of the **build identity**, which identifies the assembled
software. Runtime reports creation and active builds separately; a lifetime
is not just a version number
([docs/ARCHITECTURE.md:302](../ARCHITECTURE.md),
[docs/USAGE.md:1151](../USAGE.md)).

The supervisor repeatedly checks state, handles a presentation exit and accepts
control requests. Its explicit Stop state matters: an incomplete Stop can
retain the owner so someone can retry cleanup. “The window vanished” and
“the owner completed Stop” therefore lead to different paths
([supervisor.rs:1595](../../runtime/crates/eon-runtime/src/supervisor.rs)).

## Run it

From the repository root, in Nushell:

```nu
^nl -ba runtime/crates/eon-runtime/src/supervisor.rs | ^sed -n '1593,1634p'
^nl -ba docs/CONTRACTS.md | ^sed -n '3595,3620p'
```

## What to look at

In the loop, find the guard `state.stopping.is_none()`, the separate `may_exit`
decision and the message “terminals remain active.” In EON-C15's output,
find “Partially proved” and “installed Linux supervisor-loss adoption remains
pending.” Read that qualification before quoting the recovery behavior
([supervisor.rs:1599](../../runtime/crates/eon-runtime/src/supervisor.rs),
[docs/CONTRACTS.md:3595](../CONTRACTS.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [supervisor.rs:1593-1634](../../runtime/crates/eon-runtime/src/supervisor.rs):

- Line 1593 names the presentation after startup reached this point.
- Lines 1595–1598 enter the loop and service registry progress. The registry
  holds durable session IDs and names, described in chapter 4.
- Lines 1599–1602 reconcile terminals while the owner is not stopping.
  `?` can propagate a failure. This excerpt does not promise that every error
  leaves the supervisor alive.
- Lines 1604–1609 explain and check `may_exit`. An incomplete Stop can keep
  its owner even when the tracked-terminal list is empty.
- Lines 1611–1621 reap the presentation, meaning collect its process exit.
  When not stopping, the log gives an attach command. Workspace and standalone
  terminal modes get different messages.
- Line 1623 services timed work. Lines 1624–1628 let the detach notice's Quit
  action retry Stop and, when appropriate, finish this loop.
- Lines 1630–1634 accept control traffic and pass the request plus peer
  information to dispatch. Chapter 9 explains admission before effects.

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Isn't the supervisor another single point of failure?”

It is an owner whose loss matters. EON-C15 defines recovery of exact surviving
Orbit runs on the same boot and login. It does not promise to resurrect a dead
program or restore the old layout. Full Eon projects recovered runs into a new
synthetic session and tab. The contract remains partially proved, with installed
Linux supervisor-loss adoption pending
([docs/CONTRACTS.md:3595](../CONTRACTS.md),
[docs/CONTRACTS.md:3612](../CONTRACTS.md)).

Explain two boundaries: client loss can leave the existing supervisor and
workspace intact; supervisor loss needs recovery and may change topology.
Neither implies survival through reboot
([supervisor.rs:1614](../../runtime/crates/eon-runtime/src/supervisor.rs),
[docs/ARCHITECTURE.md:114](../ARCHITECTURE.md),
[docs/CONTRACTS.md:3612](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

Someone reports “Eon crashed.” Ask which process failed, then name the evidence
you would need before promising their terminals and layout are recoverable.
<!-- owner-only:end -->

## Going deeper

- [Runtime boundaries, docs/ARCHITECTURE.md:283](../ARCHITECTURE.md).
- [Same-boot recovery, EON-C15, docs/CONTRACTS.md:3581](../CONTRACTS.md).
- Next: [the workspace model](04-workspace-model.md).
