# 1. Persistent terminals: who stays alive?

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

When you run a shell, editor or coding agent in Eon, Orbit owns its process,
PTY and terminal state. Venus supplies the window. That separation lets Orbit
keep the terminal running when its graphical client disconnects. ORB-C1 promises
reattachment to the same live terminal while Orbit remains running
([docs/ARCHITECTURE.md:42](../ARCHITECTURE.md),
[orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md)).

Three events need different names. **Detach** removes the window while keeping
work. **Quit** asks Runtime to Stop the window's terminals. In a healthy full
Eon workspace, closing the window follows that Quit path, including any required
confirmation. An unexpected exit of Venus, the window process, does not itself
send Stop. If workspace state is unavailable, Close falls back to detach
([application.rs:1015](../../venus/src/application.rs),
[application.rs:5985](../../venus/src/application.rs)).

Losing the graphical client unexpectedly is a client disconnect; losing Orbit
itself falls outside ORB-C1's survival boundary. A reboot ends live terminals
([docs/ARCHITECTURE.md:42](../ARCHITECTURE.md),
[orbit/docs/CONTRACTS.md:62](../../orbit/docs/CONTRACTS.md),
[README.md:185](../../README.md)).

Orbit expresses the attachment lifecycle in **Bend**, a language used here for
session policy. Its Rust host handles the PTY and other operating-system
mechanics. You can read the small policy without learning Bend: a state carries
a process ID, and an operation returns the next state
([orbit/docs/CONTRACTS.md:60](../../orbit/docs/CONTRACTS.md),
[orbit/src/lifecycle.bend:1](../../orbit/src/lifecycle.bend)).

## Run it

From the **repository root, in Nushell**, read the lifecycle and its contract:

```nu
^nl -ba orbit/src/lifecycle.bend | ^sed -n '1,43p'
^nl -ba orbit/docs/CONTRACTS.md | ^sed -n '49,64p'
```

## What to look at

The first output has 43 numbered lines. In `detach`, the `Attached{pid}` case
returns `Detached{pid}`. The identifier stays the same. `begin_stop` produces
`Stopping{pid}` instead. In the second output, read “while Orbit remains
running” and the boundary excluding Orbit or machine restart
([orbit/src/lifecycle.bend:23](../../orbit/src/lifecycle.bend),
[orbit/docs/CONTRACTS.md:55](../../orbit/docs/CONTRACTS.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [orbit/src/lifecycle.bend:1-43](../../orbit/src/lifecycle.bend):

- Lines 1–5 define four states. `pid` means process identifier, carried by
  each state. These alternatives describe a lifecycle; they do not create an
  operating-system process.
- Lines 7–10 name operations that may occur before the process gets reaped,
  meaning the host collects its exit status.
- Lines 12–21 implement attach. A detached process becomes attached; attach
  leaves stopping and stopped states alone.
- Lines 23–32 implement detach. Changing attachment preserves the ID, and
  detaching does not reverse a Stop already under way.
- Lines 34–43 begin Stop. Attached and detached states enter Stopping;
  Stopping and Stopped stay as they are. The later `finish_stop` operation
  changes Stopping to Stopped at
  [orbit/src/lifecycle.bend:68](../../orbit/src/lifecycle.bend).

The small state machine explains the policy. To check real process survival,
ORB-C1 names the `shell_survives_detach_and_one_client_reattaches` lifecycle
test and real-PTY checks, alongside policy laws. Reading this function is a
different kind of evidence from executing those checks
([orbit/docs/CONTRACTS.md:74](../../orbit/docs/CONTRACTS.md)).

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Does this survive a failure, or just hiding a window?”

Client disconnect and later reattachment are the promised boundary. Orbit
continues owning the terminal while the window is absent. That supports long
work through client loss, but it is no promise to preserve a program through
Orbit failure or reboot. Runtime also has a separate same-boot recovery path
for surviving Orbit runs after supervisor loss; chapter 3 describes its proof
limits ([orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md),
[docs/CONTRACTS.md:3581](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

An agent is halfway through a build. Explain why detach, window close and reboot
need three different answers. Point to the line where detach keeps `pid`.
<!-- owner-only:end -->

## Going deeper

- [ORB-C1 and its checks, orbit/docs/CONTRACTS.md:49](../../orbit/docs/CONTRACTS.md).
- [Naming and lifecycle, docs/ARCHITECTURE.md:34](../ARCHITECTURE.md).
- Next: [the window and renderer](02-window-and-renderer.md).
