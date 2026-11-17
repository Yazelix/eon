# How Eon works

Eon combines persistent terminals, a native window and a typed workspace
control path for people and agents. The chapters explain their owners and
limits ([docs/ARCHITECTURE.md:17](../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:18](../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:267](../ARCHITECTURE.md)).

<!-- owner-only:start -->
Read this as Eon's owner: learn enough of the code to explain a mechanism,
defend a choice and say where its evidence stops. You need no Rust or Bend
background. Each chapter takes about 20 minutes, including its reading exercise
and a short explanation you give aloud. The timing is a study budget, not a
measured completion time.
<!-- owner-only:end -->

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740` (edge,
2026-11-17). These chapters describe that checkout. They distinguish source
observations, contract promises and recorded runtime proof. The contract index
defines those evidence limits at [docs/CONTRACTS.md:7](../CONTRACTS.md).

## Chapters

| Chapter | What it explains |
| --- | --- |
| [1. Persistent terminals](01-persistent-terminals.md) | terminal survival and attachment |
| [2. Window and renderer](02-window-and-renderer.md) | state becoming pixels |
| [3. Runtime supervisor](03-runtime-supervisor.md) | process coordination and recovery |
| [4. Workspace model](04-workspace-model.md) | sessions, tabs, columns and panes |
| [5. Workspace protocol](05-workspace-protocol.md) | typed control and CLI parity |
| [6. Attention and sidebar](06-attention-and-sidebar.md) | reported state and its limits |
| [7. Managed agents](07-managed-agents.md) | backend eligibility and execution scope |
| [8. Updates and live work](08-updates-and-live-work.md) | installation versus live adoption |
| [9. Security boundaries](09-security-boundaries.md) | principals, effects and host authority |
| [10. Packaging and proof](10-packaging-and-proof.md) | build identity and evidence |

The [hard questions](answers.md) summarize these limits and compare Eon with
LoopForge. The [worked example](examples/README.md) follows one terminal across
its owners.

<!-- owner-only:start -->
## A weekend plan (owner only)

Saturday morning: chapters 1–3. Saturday afternoon: 4–6. Sunday morning: 7–8.
Sunday afternoon: 9–10, then the answers sheet. Each chapter budgets about
20 minutes including its optional code walkthrough and explain-it-back question.
The reading list below adds about an hour of code reading across the weekend;
it is part of those chapter budgets, not an extra class.
<!-- owner-only:end -->

## Setup

Run **every command from the root of this repository checkout, in Nushell**.
The two commands below identify your directory and source revision:

```nu
pwd
^git rev-parse HEAD
```

The second prints your checkout's revision. A later documentation-only commit
can have a different hash while retaining the source verified above. With a
different revision, check the cited files against that source before relying
on their numbering or status. You need Nushell and the external tools `git`,
`nl` and `sed` on PATH.
The `^` prefix tells Nushell to run an external program; it also avoids a local
command with the same name. See the command itself when interpreting output:
`nl -ba` numbers source lines and `sed -n 'A,Bp'` prints lines A through B.

The chapter commands read repository files. They start no Eon process and
inspect no live terminal or home configuration. They prove that you can locate
the mechanism; they do not reproduce runtime behavior. Product invocations
mentioned in prose are vocabulary, not additional exercises. Runtime proof has
its own routes in [docs/AGENT-RULES.md:75](../AGENT-RULES.md).

## Words used throughout

- **edge** is Eon's development/dogfood branch, where tracked changes land;
  releases take an exact revision from it
  ([docs/AGENT-RULES.md:224](../AGENT-RULES.md)).
- A **risk tier** is the review classifier's grouping of a change. High and
  Medium require more independent review than Low
  ([docs/AGENT-RULES.md:123](../AGENT-RULES.md),
  [docs/AGENT-RULES.md:139](../AGENT-RULES.md)).
- A **process** is a running program. A **PTY**, or pseudoterminal, is the
  operating-system connection through which a terminal talks to its programs.
  Orbit owns that connection, the child process and terminal state
  ([docs/ARCHITECTURE.md:42](../ARCHITECTURE.md)).
- A **terminal** holds a program's terminal state; a **pane** shows it. A
  **session** contains tabs, each tab contains columns of panes, and a window
  shows one of its sessions. These are Eon's product terms; Orbit's code uses
  “Session” for one terminal run
  ([docs/ARCHITECTURE.md:38](../ARCHITECTURE.md),
  [docs/ARCHITECTURE.md:53](../ARCHITECTURE.md)).
- **Runtime** means Eon Runtime, the library whose supervisor coordinates
  processes and whose workspace model owns tabs and panes. **Venus** is the
  native graphical client; **Orbit** owns persistent terminals
  ([docs/ARCHITECTURE.md:17](../ARCHITECTURE.md),
  [docs/ARCHITECTURE.md:18](../ARCHITECTURE.md),
  [docs/ARCHITECTURE.md:283](../ARCHITECTURE.md)).
- **Presentation** means the graphical client/window process, Venus, as
  distinct from the supervisor and Orbit terminals
  ([docs/ARCHITECTURE.md:18](../ARCHITECTURE.md),
  [supervisor.rs:1611](../../runtime/crates/eon-runtime/src/supervisor.rs)).
- A **snapshot** is a description of state at a point in time. **EONW** is
  Eon's versioned workspace protocol: the agreed messages clients exchange
  with Runtime. Orbit has a separate terminal protocol
  ([docs/ARCHITECTURE.md:359](../ARCHITECTURE.md),
  [venus/docs/CONTRACTS.md:6](../../venus/docs/CONTRACTS.md)).
- A **contract** records a consumer, trigger, result and important failures.
  EON-C, ORB-C and VEN-C identify contracts owned by Eon, Orbit and Venus.
  A recorded promise and proof of its implementation are separate things
  ([docs/CONTRACTS.md:47](../CONTRACTS.md),
  [docs/CONTRACTS.md:7](../CONTRACTS.md)).
- A Rust **crate** is a library or executable package. `fn` defines a function;
  `struct` groups named fields; `enum` lists alternatives; `Result` carries
  success or failure. `?` passes a failure to the caller. Read the walkthroughs
  before trying to understand every symbol.

Read contract status before quoting a behavior:

| Label | Meaning |
| --- | --- |
| Planned | accepted intent without sufficient implementation evidence for the whole contract |
| Candidate | implementation and mechanical checks exist; accepted final proof remains open |
| Partially proved | a useful slice is proved, with a required gap |
| Proven / Proved | named checks, revision, platform and artifact cover the stated surface |
| Unproved | the stated claim still lacks sufficient proof |

The index defines the formal statuses; individual clauses may have implemented
slices while the overall contract remains Planned. Read their amendments and
proof scope together ([docs/CONTRACTS.md:7](../CONTRACTS.md),
[orbit/docs/CONTRACTS.md:15](../../orbit/docs/CONTRACTS.md),
[docs/CONTRACTS.md:7522](../CONTRACTS.md)).

<!-- owner-only:start -->
## The code that matters

Allow about six minutes per row, roughly one hour in all. Each chapter explains
these lines in small groups. A pair of ranges is one reading step, not a request
to read the whole file. Paths refer to this repository root.

| Chapter | Source | What to follow |
| --- | --- | --- |
| 1 | [orbit/src/lifecycle.bend:1-43](../../orbit/src/lifecycle.bend) | attach, detach and Stop |
| 2 | [venus/src/render.rs:1838-1890](../../venus/src/render.rs) | content rebuild and moving cursors |
| 3 | [supervisor.rs:1593-1634](../../runtime/crates/eon-runtime/src/supervisor.rs) | the control loop |
| 4 | [workspace.rs:1537-1583](../../runtime/crates/eon-runtime/src/workspace.rs) | snapshot construction |
| 5 | [cli/parity.rs:59-72,433-454](../../runtime/crates/eon-runtime/src/cli/parity.rs) | a CLI mapping and its check |
| 6 | [attention.rs:137-163](../../runtime/crates/eon-runtime/src/attention.rs) | state normalization |
| 7 | [decision.rs:28-67](../../eon/crates/eon-agent/src/decision.rs) | backend eligibility |
| 8 | [update.rs:229-260](../../eon/crates/eon/src/update.rs) | installation-maintenance handoff |
| 9 | [admission.rs:14-23,106-128](../../runtime/crates/eon-runtime/src/admission.rs) | principal and admission |
| 10 | [flake.nix:1119-1149](../../flake.nix) | resolved component paths in the launcher |
<!-- owner-only:end -->

## Where authority lives

This guide explains existing work. Follow the owning contracts, architecture and
user documentation linked by each chapter when you need the full boundary.
Contract proof applies to its named revision, environment and surface, so do not
turn an old successful run into a claim about today's build
([docs/CONTRACTS.md:22](../CONTRACTS.md)).

<!-- owner-only:start -->
## Adapting this material (owner only)

Use two line-delimited markers consistently:

- `<!-- owner-only:start -->` through `<!-- owner-only:end -->` encloses class
  planning, timing, private/internal context and personal call framing. Remove
  these complete blocks for a public adaptation. Put any later internal bead
  IDs, TODOs, machine-specific setup or agent-cache paths inside these blocks.
- `<!-- under-the-hood:start -->` through `<!-- under-the-hood:end -->` encloses
  an optional code walkthrough. Its HTML `details` element renders a collapsible
  **Under the hood** section. A public page may keep or omit it.

The plain-English idea, portable source-reading commands and honest limits
remain outside both kinds of block. Generic repository-root/Nushell setup stays
with the commands so the public version remains runnable. No first-project
setup tutorial or website publication is part of this guide.
<!-- owner-only:end -->
