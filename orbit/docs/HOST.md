# Host and worker

Bend's native effects are C only: the compiler splices an effect's `.c` file
into the program it generates. So that Orbit has no C of its own, the Rust
mechanisms run in their own process. The host ([src/host.rs](../src/host.rs))
owns the PTY child, the listener, the client sockets, libghostty and the event
loop. The Bend worker ([src/main.bend](../src/main.bend)) uses only Base's
`File` effects and decides every answer with the pure `step` of
[src/session.bend](../src/session.bend). [build.rs](../build.rs) compiles the
worker with the pinned Bend compiler and clang, and the package ships the host
as `bin/yazelix-orbit` and the worker as
`libexec/yazelix-orbit/yazelix-orbit-worker`. The host runs the worker it finds
there beside its own `bin`, or else the one its own build made, since a cargo
build has no package ([src/main.rs](../src/main.rs)).

## Processes and pipes

The host starts the worker with two inherited pipes: the worker reads facts on
its descriptor 3 (`/dev/fd/3`) and writes decisions on its descriptor 4
(`/dev/fd/4`). It exits when the fact pipe closes, and on Linux it also dies
with the host (`PR_SET_PDEATHSIG`). The host passes it `--threads 1`, since the policy
is sequential. The host watches the decision pipe in every
wait; if the worker dies or stalls, or the host receives SIGINT, SIGTERM or
SIGHUP, the host hangs up the child's group and the terminal's foreground group,
escalates to SIGKILL, reaps the child, and removes the socket pathname. Once a
signal's stop reaps the child, it gives a live observer the finish bound to
take Exited with the child's code, writes a managed run's explicit tombstone
and exits 0; the interactive client closes without
Exited. A signal that leaves the child unreaped exits 128 plus the signal, a
dead worker exits with the worker's status after naming it on stderr, and
after a stalled worker is killed the host exits 1. After a natural exit it exits
with the child's code, which Bend decides with `Finish` ([Command line](BEHAVIOR.md#command-line)).

The host runs one event loop. Each wait watches the attached client, the PTY
(and its room for input while some waits), the synchronized-output timer, the
metadata observer (for its close or a message, and for room while its queue
holds output), the listener, a peer
negotiating its role and its one-second deadline, and the management socket and
connection of a managed run. A connection accepted while a peer negotiates is
reported as fact Crowded for Bend to turn away. Timers are deadlines, not descriptors: the wait polls until the earliest
one, and each that has passed counts as a ready source. No wait lasts more
than 100 ms, so the host checks the direct child at least that often. Each turn serves one
ready source, starting after the one served last, so a source that stays
ready, such as a busy PTY, cannot starve the others.

The exchange is synchronous. The host sends one fact as one line in one write
under `PIPE_BUF`, carrying every fact the decision needs, then reads decision
lines until `end`. It waits at most two seconds for them, a host constant: it
takes what the worker has written first, and a worker whose answer still
lacks `end` past that deadline has stalled; a termination signal ends the
wait, as it ends every wait ([Fatal-exit boundary](BEHAVIOR.md#fatal-exit-boundary)). A decision that needs more facts makes the host send a
follow-up fact before it waits for the next event. Before the first fact, the
worker writes the line `schema DIGEST`, which names the schema it was
generated from ([The boundary](#the-boundary)), then answers with its bounds alone (decision
`Bounds`), within the same deadline. The host compares that line with its own before it decodes any
decision. A worker whose first line differs is refused before the host binds
the socket or starts the child: the host kills it and exits 1 with a message
quoting both lines.

Client input and libghostty's replies to the child's terminal queries (cursor
position, device attributes, mode reports) wait for the PTY in one queue,
bounded at one maximum paste plus 16 bytes. The host
writes it whenever its loop turns and watches the PTY for room while bytes
remain, so a child that stops reading its input never stops the host from
reading its output. Input the queue cannot hold is refused whole (`Applied Full`);
a reply that does not fit fails the session rather than
send part of an answer.

## The boundary

[src/boundary.schema](../src/boundary.schema) is the only definition of the
facts and the decisions: each one's code, its fields and their meanings, and
the flags word's bits. `tools/boundary.rs` generates the host's `Fact` and
`Flags` types with their line encoder and its `Decision` type with a line
decoder ([src/boundary.rs](../src/boundary.rs)), and the worker's matching
types, fact decoder and decision encoder
([src/boundary.bend](../src/boundary.bend)). Both carry the schema's
identity, the 64-bit FNV-1a digest of the schema file in hex, so a host and a
worker from different schemas, after a partial update or a mixed closure,
refuse to pair instead of misreading each other's lines; the digest tells
builds apart and is no defense against an attacker. The flake's `orbit-boundary`
check fails while the committed files are stale. This document names facts and
decisions by their schema names, such as `Applied Full` for `Applied` with
the effect `Full`.

## Facts

A fact crosses the pipe as one line of numbers, `CODE FLAGS ARG...`, with one
word per field, so the stream stays readable while debugging. The
flags word carries every fact a decision may need; every flag but `held`,
`unanchored` and the observer's `metadata_changed` and `metadata_fresh` is set
only while a client is attached. The worker dies
on a line that does not decode to a fact, as the host's encoder never writes
one.

The host reports facts in an order that the laws over the whole step in
[LAWS.bend](../LAWS.bend) assume. `Started` comes first and only once. A fact
from the client (its input, close, pressure, requests, pointer phases and
their outcomes) comes only while one is attached. `Reaped`, `ExitedBeforeStop`
and `NotReaped` come only after Bend decides `Reap` or `ManageStop`; before
them come at most the observer losses, ticks and host-phase outcomes that the
same decisions queued. After `Finish` comes only `Finished`. The laws check
nothing after a fact out of this order.

The observer's output waits in a queue under the client's bound. Rust
reports whether the title or working directory differs from the metadata last
queued (flag `metadata_changed`) and whether none was queued yet (flag
`metadata_fresh`); without an observer it reports both, for the one Bend may
admit. Bend publishes only a change, and queues an
observer's first metadata `ordered`, so it stays, while newer metadata
replaces queued metadata that has not started, so a slow observer converges
on the current title and directory. No answer queues metadata twice, and no
decision feeds the terminal, so the flags of a fact still hold when its
metadata is queued. The host never drops the observer on its own: metadata
the queue cannot take becomes `ObserverGone Pressured`, and Bend's `DropObserver` carries
out ORB-C7.

## Decisions

A decision crosses the pipe as one line of numbers, `CODE ARG...`, and the
worker ends each answer with a line `end`. The host fails the session, as a
protocol error, on a line that does not decode to a decision. A rejection
carries a typed reason (`Reject PtyFull`); the host owns each reason's
Failure code and exact text for the request it answers
(`Reason::failure` in [src/host.rs](../src/host.rs)), so Bend never writes
one.

Bend keeps the presentation revision as two U32 halves
([src/revision.bend](../src/revision.bend)), since it has no U64, and
advances it once for each change of presented state
(ORB-C4). One pass after each decision, `stamp` in
[src/session.bend](../src/session.bend), owns that rule. A fact the host applied
before Bend answers advances it first: a PTY turn that fed output to the
terminal, unless a synchronized update was held at the previous fact, a
resize, and a host phase or tick that applied. Then each decision that changes
presented state advances it as it is carried out: a viewport move, and an
`Unselect` of a presented selection, unless the clear counts with
the change that caused or accompanied it: a resize, a viewport move, output
that lost the anchor, or the child's exit with drained output. Each decision
that publishes names the revision it presents, which Rust stamps without
computing; a tick's outcome
names the revision its move reaches. No decision advances it by publishing,
so a frame of unchanged state repeats the revision of the last one. A request
carries the revision of its frame, and Bend compares it with authority. Rust
carries out a decided viewport move even when it finds the client lost
meanwhile, so the revision never advances without a change.

While an interactive client is attached, the host captures the terminal's
clipboard writes during each PTY turn (ORB-C11).
Bend turns capture on with `Capture` when it attaches a client and off when
the client goes; libghostty asks synchronously, so Rust holds the bit. Each
write must carry exactly one nonempty UTF-8 `text/plain` text without NUL and
at most 1 MiB, so a clear or any other representation is refused. Writes
while capture is off are refused, and a turn's writes that are not sent are
discarded when the next turn starts, so none is retained or replayed. Bend
sends them before the frame of the same turn, even while a synchronized update
holds that frame, and answers a turn that passed the bound with `Detach`,
sending none of them.

The host keeps the program status records of ORB-C15
([src/status.rs](../src/status.rs)). libghostty parses each OSC 7501 report,
enforces the specification's limits and answers the support probe; Rust
collects the turn's accepted reports, clears and primary prompt starts in
stream order. Before each read of a PTY turn the host samples the PTY's
foreground process group, and a sample that differs from the turn's first
ends the turn, so fact `Output` carries the sample that held for all of the
turn's output and whether the turn carried a prompt mark. Bend leads its
answer with `StatusLifetime`, which the host carries out: the foreground
return's drop first, then the turn's events in order, with a drop at each
prompt mark. Whether the terminal has ever sent a prompt mark, and the last
sample, live in Bend ([src/status.bend](../src/status.bend)). A management
`ProgramStatus` request arrives as fact `ProgramStatus` with a foreground
sample taken when the host reads it; Bend leads its answer with that sample's
`StatusLifetime`, then answers `ManageProgramStatus`, for which the host
builds the reply from the records and the terminal title, or `ManageRefuse`,
for which it sends Busy and keeps the lease.

Bend's session holds the pointer sequence's route, whether a host selection
is presented, and whether a finished host sequence froze a copy. Rust keeps
libghostty's gesture and the frozen text, reports whether each host phase or
tick leaves a selection presented (fact `Selected`), and notes PTY output that
left the gesture without an anchor or newly enabled mouse tracking (flag
`unanchored`). Bend asks the proved policy in
[src/selection.bend](../src/selection.bend) which route a press takes and
whether a phase may run, and checks each route's room before anything moves.
Bend decides every clear with `Unselect`, which resets the gesture and clears
the selection: a Cancel, a
terminal press, a lost anchor, a closed PTY, a resize, a viewport move,
losing the client and the child's exit clear it; a
terminal press, losing the client and the child's exit also forget the copy.

Bend's state is explicit: the lifecycle of [src/lifecycle.bend](../src/lifecycle.bend),
whether a half-closed client stays attached, the observer slot, whether the
stop came through the management lease, which makes its `Finish` explicit, the vertical request waiting for a synchronized update to end, the
pointer state above, the management slot, and the presentation revision with
whether a synchronized update was held at the last fact. A
waiting request keeps what the host needs to carry it out: a preview's
direction, a scroll's rows, or a tick's position, which Bend never reads, so
Rust holds no vertical request between facts. `step` is a pure function from that state and one fact to the next
state and its decisions; admission answers come from the proved policy in
[src/admission.bend](../src/admission.bend), and vertical answers from the
proved policy in [src/vertical.bend](../src/vertical.bend). The management
slot is free, negotiating, leased, or closing, and the proved policy in
[src/management.bend](../src/management.bend) answers each management event
from it and gives the slot after Rust carries out the answer. Rust keeps the
connection, its one-second bound, which `ManageLease` ends, and every identity
check; it reads the bound only to time it and, after a termination signal,
which it handles without the worker, to answer the lease Stopped. Frames and metadata
wait while a synchronized update is open, every publication asks the vertical
policy, and a scroll without room for its outcome fails with Failure(Terminal)
before the viewport moves. A return to live output takes the same policy, with
the alternate screen counted as routed, and fails with Failure(InvalidInput)
while the terminal owns scrolling, or with Failure(Terminal) when the client's
queue has no room for a frame. A physical wheel is never deferred: routed, it
reaches the terminal; otherwise it fails with Failure(Terminal) while a
synchronized update is open or the client's queue has no room for a frame.

## What Rust holds

Every field the host and its mechanisms keep falls in one of four classes;
none holds session state that Bend decides from.

- Resource handles: the worker and its pipes, the listener and the
  management listener, record and connection, the attached client, the
  observer, a negotiating peer, a crowded connection, refused clients still
  writing their last output, the PTY master and the direct child,
  libghostty's terminal, render state and gesture, whether its clipboard
  writes are captured, which Bend's `Capture` sets, the PTY input queue, each
  peer's socket, partial write and bounded output queue, and each management
  connection's buffers.
- Timers and loop scheduling: the peer's negotiation deadline, the half-close
  probe, the synchronized-output watchdog and whether it is armed, the
  management connection's one-second bound, the source the next wait starts
  from, and each client's output bound. Bend chooses every bound.
- Raw observations: whether the PTY is still open, whether the host has
  reported the child's exit, the child's pid and cached exit status, the
  surface size, a terminal reply that overflowed, the turn's clipboard writes
  and their overflow, a lost gesture anchor, the frozen copy's text (Bend holds
  only its presence), the title and working directory last queued for the
  observer, the turn's program status events and the program status records
  with their neutral facts, a client's half-close, its unread bytes and
  negotiation phase, the identity a managed run publishes, a management peer's
  UID, and the containment fixture's fault.
- Carry-out scratch, held only while Bend decides the current fact: the
  facts the host owes Bend before the next wait, a client's decoded or
  refused request, the wheel or pointer phase Bend is deciding on, a
  management connection awaiting admission or its refusal, and the exit code
  Bend's `Finish` chose, which the host returns once the worker exits.
