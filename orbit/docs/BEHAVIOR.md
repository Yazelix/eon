# Orbit behavior

Orbit is two processes built and shipped together: a Rust host runs every
mechanism and asks a Bend worker, which uses only Bend's Base effects, for
every decision. Orbit has no C of its own. This document states what Orbit
does, claim by claim, and names the real-PTY check, unit test or law that
covers each claim; [Check](#check) says how they run. The
[contracts](CONTRACTS.md) remain the canonical statement of what consumers may
rely on. [BEND.md](BEND.md) collects the practices for writing Bend 2 that
Orbit's development and Bend's own sources established.

Orbit's Bend implementation was developed in the separate bend-orbit
repository and imported at its revision
`e03926274203d78b76f49337e9f6670d2dd6e97d`; that repository keeps its history.
The throughput measurements name the old Orbit: the single-process Rust
server Orbit replaced, kept at the tag `orbit-rust-final`. References of the
form lj8.N name the issues `borb-orbit-in-bend-lj8.N` in bend-orbit's
tracker. The imported sensitivity checks, each of which broke the code or a law
in a scratch copy to show that a check or proof can fail, ran in bend-orbit
before the import.

## Ownership

- The [Bend worker](../src/main.bend) decides every answer: attachment, detach,
  publication and holding, releases, shutdown sequencing, metadata, and every
  peer's admission. Its state is explicit and its step
  ([src/session.bend](../src/session.bend)) is a pure function of that state and
  one fact, using the proved policies in
  [src/lifecycle.bend](../src/lifecycle.bend),
  [src/admission.bend](../src/admission.bend) and
  [src/vertical.bend](../src/vertical.bend).
- The [Rust host](../src/host.rs) is the process callers start. It directly
  parents the PTY child, owns the private socket pathname, the listener, the
  client sockets and libghostty, runs the event loop, reports one fact per event
  to the worker, and carries out the worker's decisions. It contains the child
  and removes the socket pathname if the worker dies or stops answering.
- Rust mechanisms implement the PTY child and terminal
  ([src/pty.rs](../src/pty.rs)), the listener, peer negotiation and output queues
  ([src/socket.rs](../src/socket.rs)), libghostty frames and history
  ([src/frame.rs](../src/frame.rs)), and input encoding
  ([src/input.rs](../src/input.rs)), with canonical protocol encoding and
  decoding throughout.

[HOST.md](HOST.md) states the fact and decision protocol between the
two processes. Their boundary code is generated from one schema, whose digest
the worker names first: a host paired with a worker from another schema exits
with status 1 and a message naming both digests, before it binds the socket or
starts the child. The real-PTY schema-mismatch check pairs the host with a
worker generated from the schema plus one flag; it fails with the host's
comparison removed, with the host reading no schema line, or with a digest
blind to the schema.

## Command line

`yazelix-orbit` takes this command line:

```sh
yazelix-orbit serve [SOCKET] [--management-v1 SESSION_ID RUN_ID COMPONENT_GENERATION]
                  [--ansi-palette-v1 RGB,...] [-- COMMAND ...]
```

It runs COMMAND, or `$SHELL`, or `/bin/sh`, as its PTY child, and peers may
connect as soon as the child starts. Without SOCKET it listens on
`orbit.sock` in a private `yazelix-orbit` directory under
`$XDG_RUNTIME_DIR` (or `/tmp/yazelix-orbit-UID`). The palette replaces the
terminal's 16 default ANSI colors, which a palette reset restores. After the
child exits, the server exits with the child's code, 1 for a child killed by a
signal, the code `Exited` carries; it prints nothing on success. A bad command
line or a command that cannot start fails with status 1 and a message
prefixed `orbit: `. The real-PTY command-line and lifecycle
checks cover the palette in frames, the child's arguments, exit codes, and
both failures. The real-PTY palette-reset check has the child override colors
0, 1 and 200, then send an OSC 104 that names no index: every color returns
to the supplied 16 and xterm's defaults above them. Installing the palette as
current colors instead of defaults made it fail.

A start replaces a stale socket pathname, a private socket nobody listens on,
under a lock on its private directory, so one of two racing starts wins
(ORB-C1). The other fails with status 1 and the message "socket
claim already in progress at PATH" while the lock is held, or "server already
listening at PATH". The real-PTY stale-socket check replaces a stale pathname,
gets each message from a start beside the live server, and finds that server
keeps its pathname and exits with its child's code.

## Terminal identity

The PTY child gets `TERM=eon` and `COLORTERM=truecolor`
(ORB-C10). The package carries the compiled `eon` entry, which inherits
`xterm-256color`, in `share/terminfo`: the flake compiles
[orbit/terminfo/eon.terminfo](../terminfo/eon.terminfo) with `tic -x`. Nothing
falls back to another identity, so the launch environment must put that
directory on `TERMINFO_DIRS`, as Eon's wrappers do; without it, lookup and
`clear` fail visibly.

The terminfo check resolves `eon` with only the package's directory on
`TERMINFO_DIRS`, gets `xterm-256color`'s clear sequence from
`TERM=eon tput clear`, and finds no `eon` without that directory. The
real-PTY terminal-identity check gives the child only the package's
directory: the child reports its `TERM`, `COLORTERM` and `TERMINFO_DIRS`,
`clear` exits 0, and the next frame is blank but for what the child prints
after it. With the entry removed, the check failed.

## Management record

`--management-v1` makes the run managed (ORB-C13).
Each identity has 1 to 128 UTF-8 bytes. Before the child starts, Rust opens a
private management socket at `SOCKET.management`; at Ready it publishes an
exact user-owned 0600 live record at `SOCKET.record` naming the three
identities, both protocol generations, the host's PID and kernel start time,
the UID, and the path and inode of both sockets, written through a private
temporary file and one rename. When a launcher retained an empty 0600 record
there, Rust first claims it with an exclusive lock and marks it, so a run
whose claim is held stops before publishing anything. When the child exits,
and when a termination signal stops the run, a tombstone replaces the live
record, carrying the natural or explicit reason and the child's exact exit
code or signal; then both sockets are removed and the record stays. A record
that is no longer this run's is left alone, and so is a socket pathname that
another endpoint has replaced: the run still exits
with the child's code.

The real-PTY management-record check decodes the live record with the
canonical management codec, checks every identity and mode and that the
child inherits neither socket, reads the natural and explicit tombstones,
replaces the socket pathname with another listener before the child exits 5
and finds the server exits 5, the replacement intact, the management socket
gone and the natural tombstone,
refuses a start whose retained record is locked and claims it once it is
free, and refuses identities outside the bounds.

Rust checks the runtime directory and any object at `SOCKET.record` before it
publishes: the directory must be an exact user-owned 0700 directory, reached
without a symlink, and an existing record an exact user-owned 0600 regular
file. Otherwise the start is refused with nothing published, and the object
stays as it was. The management-record check refuses a symlinked record, a
FIFO, an empty 0700 file, a 0640 file, a directory in the record's place, a
1700 runtime directory, and a symlink to a private runtime directory holding
an empty retained record. Ignoring the record's mode made it fail, and so did
ignoring the directory's mode both when binding the socket and when
publishing; either directory check alone suffices.

## Management lease

The management socket takes one connection at a time. Bend decides every
answer with the pure policy in [src/management.bend](../src/management.bend): a
connection is admitted only while no other holds the slot and gets Busy
otherwise; it must acquire the lease within one second; Acquire is granted only
when Rust found the expected identity, the record's exact inode and contents,
the peer's UID and both sockets all equal to this run's, and fails with
`InvalidIdentity` and closes otherwise; Status and Stop are answered only
under the lease, and a request out of place, over 4 KiB, malformed or
pipelined fails with `InvalidRequest` and closes. A
refused connection keeps the slot until its Failure is written and it closes,
so a connection meanwhile is Busy. Bend's session holds the slot and whether
a stop came through the lease, which makes the finish explicit; Rust keeps
the connection, its one-second bound and the identity checks. Losing the lease
does not stop the run. An accepted Stop begins the reap-gated stop: the host hangs up
and reaps the child with the bounded shutdown of ORB-C12, the tombstone
records the explicit stop and the exact signal, the lease hears `Stopped` with
it, the observer hears the exit, the interactive client closes without
`Exited`, and the server exits 0. Files and process
metadata never authorize anything.

[LAWS.bend](../LAWS.bend) states, and [PROOF.bend](../PROOF.bend) proves, that over
any list of connections, requests and departures, from any slot, the answers
satisfy an audit kept apart from the policy: a connection is admitted only
into a free slot and is Busy otherwise, a lease is granted only for a valid
identity during negotiation, Status and Stop are carried out only under the
lease, anything out of place is refused and keeps the slot until it closes,
and a refused connection is answered nothing more; and that a Stop is carried
out exactly when the lease is held. Four sensitivity checks, one that stopped
the run during negotiation, one that admitted a second connection under the
lease, one that freed the slot at a refusal and one that admitted a connection
while a refused one closed, made the proof fail.

The real-PTY management-lease check refuses a wrong identity, holds the slot
with a silent connection so a contender is Busy until the one-second bound
closes it, refuses Status before the lease, an oversized request and an
Acquire after the record's mode changed, grants a lease that reports Status,
keeps the run after the lease goes, lets one of two racing replacements win
while the other is Busy, and stops the run through the lease with an explicit
tombstone and exit 0.

## Host and worker split

Bend owns session policy and state transitions; Rust owns the OS, libghostty,
the canonical protocol and resource ownership. Decisions belong to Bend even
when Rust carries out their effects.

Bend's native effects are C only: the compiler splices an effect's C file into
the program it generates. The worker therefore runs in its own process and
talks to the host over two inherited pipes with Base's `File` effects, so
Orbit has no C of its own, and a Bend update cannot break a native adapter.
Each decision costs a pipe round trip; lj8.40 measured that cost against the
old Orbit, and [Throughput](#throughput) records the result. Bend alone still
lacks the PTY, Unix socket and terminal interfaces the Orbit contracts
require. Revisit the split if the round trip shows in the measurements, if
fatal-exit containment or macOS behavior changes it, or if Bend gains native
interfaces that change the tradeoff.

## Reap-gated Stop

[LAWS.bend](../LAWS.bend) states that any sequence of `Attach`, `Detach`, and
`BeginStop` events preserves whether the session is `Stopped`.
[PROOF.bend](../PROOF.bend) proves this by checking each transition and inducting
over the event list. Thus Bend's pre-reap policy cannot turn a live session
into `Stopped`, even after repeated or reordered events.

The running paths use that policy. The run ends when the direct child exits,
not when the PTY closes (ORB-C12): the host checks the
child with `Child::try_wait` every turn and at least every 100 ms, so a
descendant that holds the PTY open, idle or writing, cannot hide the exit.
Once the child is reaped, the host feeds the PTY's remaining output, at most
four reads, to the terminal; Bend clears a presented selection (Selection and
copy), publishes the result, begins the stop and asks the host to reap the
child, whose status the host reports from that wait. Only then does Bend call
`finish_stop` and decide the exit, and the server exits
with the child's code after the host reports that the clients heard it and the
socket pathname is gone. Nothing signals the descendants after a natural exit;
a writing one fails once the PTY master closes.

The Bend proof covers its transitions, not the truth of the host's facts or
the full ORB-C12 shutdown contract. The real-PTY checks cover the
connection to Rust and the OS: they receive Exited and confirm that the child
PID is absent after the server finishes, but they do not observe the child at
the instant Exited is sent. A one-time sensitivity check changed `BeginStop`
on an attached session to return `Stopped`; the Bend proof failed, and the
change was restored.

Linux reports EIO on the PTY master while no process holds the terminal open,
which a live child may do briefly before reopening `/dev/tty`.
Rust reports the PTY closed only on end of file or once the
direct child has exited; until then EIO pauses reading for 10 ms (ORB-C1). A
PTY that reaches end of file while the child lives, as a portable PTY may, is
read and written no more: queued input is discarded, a pointer sequence ends,
every later request fails with `Failure(Terminal)` "PTY is closed" and keeps
the client, and the run waits for the child's exit. The real-PTY
descendant-exit check runs a writing descendant, a
HUP-ignoring job that keeps writing to the PTY after the child exits 3: the
server exits 3 within three seconds with its socket gone, and the job is gone
soon after. A managed run whose idle HUP-ignoring job holds the PTY ends
within a second of its child's exit with the natural tombstone. A sensitivity
check that waited for the PTY to close instead made it hang. The
real-PTY pty-reopen check has an attached child close every terminal
descriptor for 2.5 seconds, longer than the reap bound, and then reopen its
terminal; its later output reaches the client and it exits with its own status.

## Observer pressure

An observer send never waits. Rust queues the
observer's output under the same bound as the interactive client's and writes
what its socket takes. Bend publishes metadata only when its title or working
directory differs from the metadata last queued, which Rust reports; the first
metadata stays, and newer metadata replaces queued metadata that has not
started, as Bend decides, so an observer that falls behind converges on the
current title and working directory. Only metadata the queue
cannot take is pressure, which Rust reports; an I/O error is reported
separately. Bend drops a pressured observer, as ORB-C7 requires, so it never
parks the session loop. The real-PTY slow-observer check stops reading while
the child changes its title a thousand times; the observer stays attached,
receives fewer changes in order and then the latest, hears nothing for later
output under the same title, and hears the exit; a sensitivity check that
published metadata without a change made it fail. The
lifecycle check runs one interactive client beside a nonreading metadata
observer to completion. This establishes behavior for these checks, not
general multi-client fairness.

Every stop tells a live observer how the run ended
(ORB-C3): at child exit, at a management Stop, and once a termination signal's
stop has reaped the child, before the explicit tombstone, the observer gets
Exited with the child's code, 1 for a signal, behind its queued metadata, and
one second, the finish bound, to take them before its socket closes. The
real-PTY observer-finish check sends SIGTERM to a managed run with an
interactive client and an observer attached: the observer reads `Exited` with
code 1 and then the end of the stream, the client closes without `Exited`, the
server exits 0 and the tombstone records the explicit stop. It also fills an
observer's socket with a thousand title changes and reads it again only after
the child is reaped; the observer still receives the latest title and
`Exited`. Sensitivity checks that closed the observer without the finish bound,
or sent it nothing after the signal, made it fail.

## Interactive pressure

Rust queues every message to the interactive client and writes only what the
socket takes, so a slow or stopped client never parks Bend's loop.
A newer frame replaces queued frames and previews that have
not started to send, and a newer preview replaces one such preview. Bend sets
the queue's bound at attachment to 12,587,045 bytes: one maximum
frame, one maximum payload, two headers, and 4 KiB. A message that would pass
the bound is not queued. The session's next wait reports the pressure, and
Bend detaches the client (ORB-C7) while the child, listener, and observer keep
running. The client keeps the whole messages already written, with at most one
cut message at the end. If its requests are still unread when the socket
closes, the client may see a reset instead.

The real-PTY interactive-pressure check attaches a client that stops reading
and sends upward selection ticks with no gesture, which Bend answers with Failure, while
the child keeps printing and changing its title. The observer receives the
changed titles while that output builds. After 82,000 to 120,000 requests Bend
detaches the client; a new client then attaches to the same child and receives
a current frame.

At child exit, Bend gives the interactive client one second, the
finish timeout, to take its remaining output and Exited, and then closes it
without the rest. The real-PTY stopped-client check floods a client that has
stopped reading; the server finishes about one second after the child exits,
with the child reaped and the socket pathname removed.

The client's output bound, which Bend sends at start, is one maximum frame,
one maximum payload, two headers and 4 KiB, so a maximum frame the client has
begun to read still leaves room for one maximum scroll outcome (ORB-C7). The
host unit test `worker_bound_admits_a_maximum_scroll_outcome_behind_a_started_maximum_frame`
reads the bound from the built worker, writes one byte of a maximum frame,
and queues a maximum scroll outcome behind it; a worker bound without the
frame's share made it fail.

## PTY input

Client input and the terminal's replies to the child's queries reach the PTY
through one queue bounded at one maximum paste plus 16
bytes. libghostty generates each reply, such as the cursor position or device
attributes, and Rust queues it whether or not a client is attached, so Orbit
alone answers terminal queries (ORB-C2). Rust accepts input once it is queued
and writes it as the PTY drains, while the loop keeps reading PTY output, so a
child that prints what it reads cannot stall the session on a large paste.
Input that does not fit is refused whole: Bend answers `Failure(Terminal)` and
ends that attachment, and the child keeps running. A
client Bend refuses this way, or for a repeated role or a request that fails
to decode, is closed only once its queued output, the Failure included, is
written, within the one-second finish bound; its slot is free at once. A
client under output pressure is dropped at once (ORB-C7). The real-PTY
refused-flush check floods a client that stops reading, overflows its input,
and reads only whole messages ending in the Failure; on macOS, whose socket
buffers are small, a sensitivity check that dropped the client at once cut a
frame and made it fail.

The real-PTY paste-flood check sends two 600,000-byte pastes to a child that
reads nothing: the first is accepted, the second fails and ends the attachment.
A new client then attaches, the child starts copying its input to its output,
and the queued paste and a third one reach it in order, line for line, while
frames keep arriving. The real-PTY terminal-replies check has the child ask for
its cursor position before any client attaches, and for its cursor position and
device attributes while one is, and receive each answer.

## PTY output wake

The attached Venus loop waits on the client socket, PTY master, and listener.
Rust reports requests, PTY readiness, disconnects, and negotiated peers. Bend
chooses when to read the PTY and publish a canonical frame. The real-PTY
delayed-output check receives a frame while the client is idle, then sends an
enabled Focus event and three inputs on the same connection. The Focus event
produces no output. The first input produces output, the second produces none,
and the third is accepted before final output and Exited reach the half-closed
client. The request half-close check holds child output after enabled Focus and
Resize until the client closes its write side. An accepted PTY write or Resize
keeps that client attached for later child output; a full close detaches.
The reconnect check proves the server survives a partial Hello, a close before
Attached, and a partial request. Three interactive clients then attach in
sequence; the check preserves the child PID and detached title, and confirms a
later full close frees a client that had half-closed after input.

While an interactive client is attached, Bend also accepts a later metadata
observer and chooses whether to admit that role or return canonical Busy to an
overlapping interactive client. Rust keeps one pending role negotiation
nonblocking and expires a silent or partial peer after Bend's one-second bound;
a peer still negotiating when the client detaches keeps negotiating.
The real-PTY late-observer check proves initial and changed title metadata,
live interactive input, Busy, and timeout without losing the child. It also
has an observer send Focus: the observer reads `Failure(Protocol)` and the end
of the stream, and a new observer is then admitted. Without the refusal, the
check failed.

While detached and awaiting a connection, Bend waits on the listener, the PTY
and the synchronized-output timer. It keeps reading PTY output, sends a
metadata observer each change, and, when the direct child exits, reaps it before
closing the listener and removing its socket pathname. The real-PTY check
covers exit before the first attachment, and after a client disconnects. During the first Hello, Bend also waits on the peer, the
PTY and the synchronized-output timer, and keeps the observer current. A peer
that sends no role within Bend's one-second bound is closed unanswered, as
while attached, and the next connection then negotiates. A connection that
arrives meanwhile hears Busy at once and is closed. If
the child exits while the peer is silent, Bend closes that
unnegotiated socket, reaps the child, and removes the listener pathname. The
same check holds a silent accepted peer connected through child exit and
verifies it receives no exit event. The real-PTY silent-peer check connects a
peer that never sends a role, before the first attachment and again while a
client is attached: a connection behind it reads Busy and the end of the
stream, the silent peer is closed once the bound passes, and a client then
attaches to the same child. Without the Busy answer, the check failed.

A peer's first role message decides what it is, so a metadata observer may also
connect while no interactive client is attached. Bend admits it to a free
observer slot or returns Busy, and keeps waiting detached. The real-PTY
detached-observer check admits an observer before any attachment and turns away
a second. The observer receives changed titles before the first client
attaches and after it leaves, and receives a title held by a synchronized
update once the watchdog ends it. A second client then attaches to the same
child.

## Synchronized output

While libghostty reports DEC 2026 synchronized output, Bend holds attached
frames, the initial attach frame, and observer metadata. Rust reports the mode
and arms a one-shot timer with Bend's one-second bound the first time Bend sees
the mode set. When the update ends, Bend publishes one complete frame. If the
timer expires first, Bend clears the mode through Rust and publishes the
current frame; child exit does the same before `Exited`. While detached, the
same timer ends an update that holds observer metadata. The real-PTY
synchronized-output check splits one update across reads, accepts input while
it is held, and covers the watchdog and exit release with one interactive
client and one metadata observer.

A client that attaches while an update is held receives its first frame only
when the update ends (ORB-C4). The real-PTY held-attach check attaches during
a held update and receives nothing until the child releases it, then a first
frame with the title the update set; it attaches during one that is never
released, checks that the attachment stays quiet for 0.3 seconds, then
receives the first frame from the watchdog within two seconds of observing the
child's mark. It then detaches from a rich alternate screen (styles, a
hyperlink, wide and combining graphemes, a moved cursor), and a new client's
first frame equals the last one exactly. Not reporting the held update to Bend
made it fail, and so did a three-second watchdog.

## Revisions

Bend keeps the presentation revision, a 64-bit counter held as two U32
halves in [src/revision.bend](../src/revision.bend), since Bend has no U64. It
advances the revision once for each change of presented state
(ORB-C4): a PTY turn that feeds output to the terminal, a resize, a
viewport move, an applied host selection phase or tick, a Cancel, and a
cleared selection. Each decision that publishes names its revision, which
Rust stamps on the frame, outcome or metadata, and Bend compares the revision
a request names with its own. A
synchronized update advances it once, at its first turn, so the frame that
ends the update carries that revision. Publishing advances nothing: an
attachment's first frame, a frame the watchdog releases and any other frame
of unchanged state carry the current revision. Output while no client is
attached still advances it, and a metadata observer hears it. The real-PTY
three-attach check reattaches after the last frame with no output between,
receives that frame again, revision included, and then a later revision for
the child's next output.

[LAWS.bend](../LAWS.bend) states, and [PROOF.bend](../PROOF.bend) proves, that the
next revision is always ahead of the current one, except after the last,
where the counter wraps to 0 after 2^64 changes, which no session reaches. Two
sensitivity checks, one that dropped the carry into the high half and one that
compared the low halves before the high ones, made the proof fail. The
scrolling and selection law below covers where `step` advances it, except
which clears do, and the real-PTY checks named in each section cover the
frames that carry it.

## History

libghostty keeps primary history under a 16 MiB byte budget, and each
frame's scroll position comes from libghostty's scrollbar. The
real-PTY history check prints 20,000 lines and reads the same history rows,
at the live position, from a reconnected client. The host unit test
`scroll_position_follows_reflow_eviction_and_clears` follows the frame's
position through output below a held viewport, which keeps its rows, the
alternate screen and back, a reflow to fewer columns, eviction under a small
byte budget with the viewport held at the oldest row, `CSI 3 J` and reset
(ORB-C8); counting `rows_from_live` from the top instead made it fail.

Rows read directly from libghostty's screen, as previews and history reads
do, equal the canonical frame's rows (ORB-C6). The host unit test
`direct_rows_match_rich_canonical_frame_rows` writes protected, styled,
hyperlinked text with wide, combining and spacer-head cells that wraps, and
compares every frame row with its direct read, before and after a resize to
fewer columns reflows it; reading the direct row one line off made it fail.

## Clipboard writes

While an interactive client is attached, Bend has Rust capture the
terminal's clipboard writes from each PTY turn
(ORB-C11): one
nonempty UTF-8 `text/plain` text without NUL, at most 1 MiB, for the
standard, selection or primary clipboard. Clears, other representations and
writes while detached are refused, and nothing is retained for a later turn.
Bend sends the turn's writes in order before its frame, even while a
synchronized update holds the frame, and no later frame replaces them in the
client's queue. A turn with more than 64 writes or 1 MiB of text ends the
attachment without sending any; a write the client's queue cannot take ends
it as other output pressure does (ORB-C7).

The real-PTY clipboard check drops a write made before the first attachment
and a clear, receives a standard and a primary write before the frame of
their read, receives a selection write while a synchronized update holds its
frame, and ends the attachment on a read with 300 writes; a new client then
attaches to the same child and receives none of them. A sensitivity check
that sent the writes after the frame made it fail, and so did one in which
Bend never turned capture on at attachment.

The real-PTY clipboard-pressure check attaches a client that stops reading
after its first frame, while the child sends 40 writes of 512 KiB, one at a
time, paced by a metadata observer. The writes fill the client's queue, which
ends the attachment after fewer than 40 whole writes; a new client then hears
none of them. Ignoring the queue's bound made it fail, and so did keeping
each turn's writes for later turns. Letting only ordered messages past the
bound did not: the frame queued behind them still overflowed it.

## Vertical preview

An attached client can ask for a read-only `PreviewVertical` from a frame
revision it has presented. Rust reports facts: the revision the request
names, whether mouse tracking or alternate-screen mode 1007 routes the wheel
to the terminal, and whether a synchronized update is open. Bend compares the
revision with authority and passes the facts, with its own slot, to the pure
policy in
[src/vertical.bend](../src/vertical.bend), in this order: a
future revision fails, a terminal-routed request is answered at once, and
otherwise one preview may wait while an update is held; a second fails. Bend
asks the same policy after every published frame, which resolves the waiting
preview with routing evaluated again. Bend's session holds the waiting
preview and its direction, and Rust carries out each answer from the decision
that names it. The answer names the current revision and carries up
to one viewport of canonical history rows, nearest first, read by the frame
extractor from libghostty. A preview sends no PTY input or frame and does not
move the viewport.

## Authoritative scroll

An attached client can commit a signed `ScrollVertical` of -1,024 to 1,024 rows
from a presented revision; the decoder refuses any other distance. The commit
takes the preview's path: Rust reports the same facts and the same proved
policy answers it, so a future revision fails, a terminal-owned wheel gets
`TerminalOwned` at once without moving anything, and one request, preview or
scroll, may wait while an update is held. A lagging revision resolves against
current authority. Before the viewport moves, Rust checks that the client's
queue can take an outcome of any size; if not, Bend answers `Failure(Terminal)`
and the client stays attached. Otherwise libghostty
moves the viewport, clamped at the history edges, authority advances once, and
one `ScrollOutcome` carries the requested and applied rows, the frame, and the
next window in the scroll's direction. No PTY input is sent, and a host
selection is cleared, as for every viewport move. In the client's queue a scroll outcome replaces
frames and previews that have not started, and nothing replaces it.

The real-PTY scroll check covers an upward commit and a lagging downward one,
both history edges, a future revision, deferral during synchronized output with
a refused second request, the terminal-owned answer, and a commit that fails
under output pressure without moving the viewport. The child confirms that the
first input it reads is the final paste.

## Return to live

An attached client's `ReturnToLive` takes the scroll's path to the same proved
policy, with no revision to check. The alternate screen, and mouse tracking or
mode 1007 routing the wheel, own scrolling, so Bend answers
`Failure(InvalidInput)` at once and nothing moves; otherwise one return may
wait while an update is held, and a second request fails. Before the viewport
moves, Rust checks that the client's queue can take a frame of any size; if
not, Bend answers `Failure(Terminal)` and the client stays attached. Otherwise
libghostty moves the viewport to the bottom from any depth, beyond the 1,024
rows one scroll may move, authority advances once, and one canonical frame
with `rows_from_live` zero is published. No PTY input is sent.

The real-PTY return-to-live check reaches more than 1,024 rows from live with
two scroll commits and returns in one frame. It covers deferral during
synchronized output with a refused second request, refusal under mouse tracking
and on the alternate screen without moving the viewport, and a client that
stops reading until its queue lacks room for a frame: that return fails with
`Failure(Terminal)`, as does a host selection press behind it; a later update
finds no sequence, a later scroll shows the viewport did not move, and the
client then returns to live. Admitting the press without room made it fail.
The child confirms that the first input it reads is a paste sent after every
return.

A key that writes PTY bytes also returns a scrolled-back primary viewport to
live output: Rust reports the key and whether the
viewport is above live, and Bend answers Accepted, moves the viewport, and
publishes the frame, which waits like any other while an update is held. Paste
and other input, and a key that writes nothing, leave the viewport where it is.
The real-PTY key-live check scrolls up, sends a paste and a key release that
leave the viewport in history, then a key press that brings it back.

A key release keeps its text only when the encoder knows the key's unshifted
codepoint, so a Kitty-mode release of an unidentified key writes nothing
rather than falling back to its text (ORB-C5). The host unit test
`kitty_release_never_falls_back_to_text` encodes a space release and an
unidentified release carrying `!`; keeping the text on every release made it
fail.

## Mouse and wheel

Rust encodes mouse input with libghostty's mouse encoder from the terminal's
modes and the client's surface size, and queues it
for the PTY like other input (ORB-C5); while the child tracks no mouse the
encoder writes nothing and Bend answers Accepted. A physical vertical wheel is
Bend's to decide (ORB-C8). While mouse tracking, or mode 1007 on the alternate
screen, owns it, the wheel reaches the child as its mouse event or as an arrow
key and Bend answers `WheelOutcome::TerminalRouted`. Otherwise Bend moves the
viewport one row, clamped at the history edges, and authority advances once
with the frame in `WheelOutcome::Viewport`, which the client's queue treats as
a frame. Unlike a vertical request, a wheel is never deferred: during
synchronized output, or when the client's queue has no room for a frame, it
fails with `Failure(Terminal)` and nothing moves. Rust reports whether the
routed input encodes to nothing, which fails with `Failure(InvalidInput)`, and
whether the client's queue or the PTY input queue could take it; a full PTY
queue ends the attachment, as for other input.

The real-PTY mouse check sends an untracked click, which the child never reads,
moves the viewport up, down and against live output one row at a time, and
fails a wheel during synchronized output without moving the viewport. Under SGR
mouse tracking the child reads a press, a release and the wheel, while motion
with no button writes nothing; on the alternate screen with mode 1007 it reads
the wheel as an arrow key.

## Vertical request laws

[LAWS.bend](../LAWS.bend) states five laws over that policy, and
[PROOF.bend](../PROOF.bend) proves them:

- Over any list of requests, published frames and detaches, from either slot
  state, the answers satisfy an audit automaton: at most one request waits, only
  a waiting request is resolved or discarded, Busy is answered only while one
  waits, and the final occupancy matches the replayed slot.
- A future revision is always rejected.
- A terminal-routed request is always answered at once.
- No published frame resolves a request while the update is held.
- A published frame after release always resolves a waiting request.

One sensitivity check per law changed the policy in a scratch copy (defer a
second preview, answer a future revision, quiet a routed request, resolve while
held, stay quiet after release); each made the matching proof fail. The
sequence proof splits on every case, so it also fails for any policy change,
not only for ones that break the audit. The answer type has no PTY-input
constructor, so no answer sends input; only a committed scroll or a return to
live, which Rust carries out, moves the viewport, and Bend advances the
revision once for it. The proofs cover
Bend's decisions, not the truth of Rust's facts or delivery. Losing the client
clears the waiting request in Bend's session, as the policy's `Discard` answer
models; the running loop does not call the policy for detach, and the
scrolling and selection law covers that `step` clears it.

## Role admission

Rust reads each peer's first message and reports it as `Hello`,
`ObserveMetadata`, a refused message, or an unsupported version; it no longer
compares a role against an expected one. Every admission decision goes to the
pure policy in [src/admission.bend](../src/admission.bend), for ORB-C3's one
interactive client and one metadata observer: a role is admitted while its slot
is free and gets Busy otherwise, a role repeated by the attached client ends
that attachment with `Failure(Protocol)`, a malformed first message gets
`Failure(Protocol)`, an unsupported version gets nothing, and a connection
that arrives while another peer negotiates is turned away with Busy, which
Rust writes without waiting before it closes the connection. An observer may
send nothing: once a whole header arrives from it
Rust reports a message, and Bend answers `Failure(Protocol)` "metadata
observers cannot send messages", or the decoder's detail for a malformed
header, and frees the observer slot; Rust closes the observer once the failure
is written, within the finish bound. A header of another ORBS version drops
the observer silently. The Venus loop
asks the policy at each decision point, where its position gives the
interactive slot and Bend's observer handle the observer slot, and carries out
the answer. Rust still records the role Bend admitted, because frames and
metadata use different output mechanisms.

[LAWS.bend](../LAWS.bend) states, and [PROOF.bend](../PROOF.bend) proves, that over
any list of first messages, repeated roles and departures, from any slots, the
answers satisfy an audit kept apart from the policy: a role is admitted only
into a free slot, Busy is answered only when the asked slot is taken, a valid
request on a free slot is never turned away, a reject only ends an attached
client, a bad first message is refused or dropped, and a connection that
arrives during another's negotiation is turned away without changing either
slot, and only an observer is expelled for a message, which frees its slot.
Three sensitivity checks, one that admitted a second interactive client, one
that answered a crowded connection as the negotiating peer's Busy, and one
that kept an observer that sent a message, made the proof fail. As with the vertical
laws, the proof covers Bend's decisions, not Rust's decoding or delivery, and
the running loop does not call the policy for departures; its position already
reflects them.

## Lifecycle law

The laws above prove the pure policies; this one proves the whole `step` of
[src/session.bend](../src/session.bend), which composes them with the session's
state. [LAWS.bend](../LAWS.bend) states an automaton for the lifecycle
obligations of ORB-C1, C3 and C12 that Bend decides, written from the
contracts and kept apart from the policy. It reads only the lifecycle
decisions among `step`'s answers: admissions, Busy and refusals, observer
drops, the end of an attachment (`Detach` or `Dismiss`), `Reap`, `ManageStop`
and `Finish`. The law says that over any list of facts with their flags, from
any session state, `step`'s decisions never break the automaton:

- Peers are admitted as the admission audit of
  [src/admission.bend](../src/admission.bend) requires: one interactive client
  and one observer, Busy only for a taken slot, a repeated role ends the
  attachment, and an observer that speaks is expelled.
- A client that closes is detached; any other fact from the client, and
  output, may end the attachment and do nothing else. Nothing a client does
  stops the session, and a closed PTY alone does not either (ORB-C1).
- A stop begins only from the child's exit, which asks the host to reap, or
  from a management Stop. Once it begins, the only lifecycle decision is one
  `Finish`, answering the reap with the reaped status (ORB-C12).

The automaton assumes the host's fact order, which
[HOST.md](HOST.md) states; after a fact out of that order it
checks nothing more. [PROOF.bend](../PROOF.bend) proves the law by induction
over the facts, and [proof/step.bend](../proof/step.bend) shows for each fact
that the automaton's next phase is the phase of the state `step` reaches.
`step-proof lifecycle` ([tools/step-proof.rs](../tools/step-proof.rs), whose
header says how to build and run it) generates that file. Bend reduces a call
only once the values it matches on are constructors, so each lemma splits
cases until every leaf computes; the tool finds those splits by asking Bend
which variable a failing leaf waits on. Rerun it after changing `step` or the
automaton.

Two sensitivity checks broke `step` in a scratch copy and made the proof
fail: one admitted a second client while one was attached, and one began the
stop at the child's exit without asking the host to reap. As with the other
laws, the proof covers Bend's decisions, not the truth of Rust's facts or
their delivery.

## Selection and copy

An attached client's left-pointer sequence (`Selection` Begin, Update and
Finish) takes one route from its press to its finish or cancel, which Bend
chooses with the pure policy in [src/selection.bend](../src/selection.bend)
(ORB-C9): host selection when the terminal does not track
the mouse or Shift is held, and the terminal otherwise. A press from a future
revision, a press during a sequence, and an update or finish without one fail
with `Failure(InvalidInput)`. On the host route Rust resolves each position
through libghostty's tracked gesture APIs, after Bend checks that the position
lies in the viewport and that the client's queue has room, drops unstarted
frames from earlier phases, and publishes the selected presentation; the finish
names the revision of its frame in `SelectionFinished`, freezes the selected
plain text, at most 1 MiB, and returns it as `CopiedText` for the selection
clipboard. `Copy` returns the same text for the standard clipboard, and fails
when nothing is frozen. On the terminal route each phase reaches the child as a
left-button mouse event, and a finish names the current revision. `Cancel`
ends either route and clears the presentation but keeps the copy; a resize or
viewport move clears the selection, and losing the client also forgets the
copy. The child's exit ends either route and clears a presented selection;
when the host reaps the child while the PTY is open, the
last frame before `Exited` shows it cleared. Bend's session holds the route
and whether a selection is presented or a copy exists, and Bend decides each
of these clears; Rust keeps libghostty's gesture and the frozen text.

Presses repeat within 500 ms and one cell width: one press
and drag selects cells, two select words with libghostty's default
boundaries, three select a span delimited only by ASCII space, tab, unwritten
cells and hard line breaks, and four or more select the logical line; spans
and lines cross soft wraps, and drags extend at the same granularity.
libghostty counts at most three clicks, so Rust gives the third press word
events with space and tab as the only boundaries. Compatible PTY output keeps
the gesture, its click count and the tracked selection as content moves,
including into history; output that newly enables mouse tracking, or leaves
no valid anchor, clears them, which Rust reports and Bend decides, and a later
phase of that sequence fails.

While a host sequence is held on the primary screen, `Selection(AutoscrollUp)`
from a top-row position extends the same tracked selection one row into
history: a drag to the top edge enters libghostty's
upward autoscroll, one autoscroll tick moves the viewport and the selection,
authority advances once, and one `ScrollOutcome` carries the applied rows, 0
at the oldest row, the frame and the next window up. A tick without such a
gesture or away from the top row fails with `Failure(InvalidInput)`, and one
whose anchor is gone also ends the sequence. Otherwise Bend treats it as a
vertical request of the proved policy: it fails with `Failure(Terminal)` when
the client's queue cannot take a whole scroll outcome, and waits while a
synchronized update is open, where a second request fails; a waiting tick is
decided again once the update ends, since its gesture may have ended.

[LAWS.bend](../LAWS.bend) states, and [PROOF.bend](../PROOF.bend) proves, that over
any list of pointer requests and resets, from any route, the answers satisfy
an audit kept apart from the policy: a phase runs only on the route its
sequence began with, a sequence begins only from idle and from a presented
revision, on the host whenever Shift is held or the terminal does not track the
mouse, and only an existing copy is returned. Two sensitivity checks, one that
sent Shift presses to the terminal and one that ran a phase without a sequence,
made the proof fail. As with the other laws, the proof covers Bend's decisions,
not libghostty's selection; the scrolling and selection law covers how `step`
in [src/session.bend](../src/session.bend) keeps the route in the session.

The real-PTY selection check refuses a copy, an update and a future press
before any sequence, selects and copies `HELLO` on the host, refuses a second
press during it, cancels while keeping the copy, climbs the click ladder from
`beta/gamma` to `alpha:beta/gamma` to the whole line, ends a host sequence
when the child enables mouse tracking, sends a sequence to a child that tracks
the mouse as SGR press and release reports, selects `WORLD` on the host with
Shift while the child tracks the mouse, and finds the copy gone for a new
client, which selects `WORLD` again before the child exits: the last frame
before `Exited` presents no selection, and a `step` that kept the selection at
the exit made it fail. The real-PTY selection-tick check refuses a tick
without a gesture and one away from the top row, defers one during a
synchronized update and refuses a second, then ticks through all 37 history
rows and four more at the oldest row, and copies exactly `line 01` to
`line 42`. It then commits 35 rows toward live and finds the presented selection
cleared though rows of it stay in view, selects again two rows above live and
returns to live with that selection cleared (ORB-C8); dropping the clear from
either move made it fail. A unit test replays a click-ladder stimulus on a
wrapped line with output between each press and release, and checks that
exactly the copied text's cells are highlighted; giving the third press
libghostty's native events instead of span events made it fail, and so did
presenting no cell as selected.

Unit tests in [src/pointer.rs](../src/pointer.rs) cover the rest of the
gesture mechanism (ORB-C9). `spans_end_only_at_ascii_space_tab_and_unwritten_cells`
triple-clicks punctuation, a space run, which selects and copies as empty,
the text after a tab and the unwritten cells the tab skips, which select
nothing, a non-breaking space and a combining mark; making the non-breaking
space a boundary made it fail.
`a_drag_back_to_the_press_cell_clears_the_selection` drags away and back and
finds nothing presented or highlighted; keeping the last selection when a drag
selects nothing made it fail.
`a_held_gesture_survives_output_that_scrolls_it_into_history` scrolls a held
selection's row into history, checks that its selected text survives the output,
and copies from there on release; unanchoring on output that grows history
made it fail.
`upward_ticks_extend_cells_spans_and_lines_with_unicode_copy` ticks a held
cell, span and line gesture from mid-row through more than one viewport to the
oldest row and copies the exact text with its wide characters (ORB-C8); not
recognizing the span made it fail. Giving the ticks themselves native events
instead of span events did not, since libghostty appears to keep the span its
press chose.

## Scrolling and selection law

The lifecycle law covers who attaches and when the run stops; this one proves
the whole `step` for ORB-C8 and C9, where it composes the vertical and
selection policies with the waiting request, the pointer route, the copy and
the revision that Bend's session holds. [LAWS.bend](../LAWS.bend) states a view
automaton, written from the contracts and kept apart from `step`, of whether a
client is attached, which kind of vertical request waits, the route, whether a
copy exists, authority's revision, and whether a synchronized update was held. It
reads frames, releases, clears, viewport moves, vertical and pointer answers,
copies, attachment and the start of a stop among `step`'s decisions. The law
says that over any list of facts with their flags, from any session state,
`step`'s decisions never break the automaton:

- A preview, a scroll, a return to live, or an upward tick that fits gets the
  proved vertical policy's answer, which the vertical audit checks against the
  slot; a deferred request is answered nothing and takes the slot.
- No `Frame` is published while a synchronized update is held, until
  `Release`, and a frame published while a request waits is followed at once
  by that request's answer, which empties the slot.
- A preview and a tick never move. A scroll moves only while the terminal does
  not own scrolling and the client has room for its outcome, and a return to
  live only while neither terminal routing nor the alternate screen owns
  scrolling and the client has room for a frame; a clear followed by a move
  counts as moving. This holds for a request answered at once and for one that
  a frame resolves after it waited. Once a request is answered, nothing else
  in that fact clears the selection or moves the viewport.
- A wheel moves the viewport only outside terminal routing and a held update,
  and with room for a frame.
- A pointer phase gets the proved selection policy's answer on the route the
  selection audit keeps. A phase refused for its position or the client's room
  keeps its route, a terminal press clears the host selection and forgets the
  copy, and a Cancel clears the selection.
- A copy is returned only for `Copy` while one exists, to the standard
  clipboard, or once at a host finish that presented a selection, to the
  selection clipboard; a host press, a terminal press and losing the client
  forget it.
- A clear ends a host sequence and keeps a terminal one. An unanchored note
  clears before the fact is decided; a closed PTY and a resize end the
  sequence on either route, and a failed host phase and a lost anchor end the
  host's; a tick needs a held host gesture on the primary screen.
- Authority advances once for each viewport move and not for a
  terminal-owned wheel; once for output fed outside a held update, a resize,
  and an applied host phase or tick (ORB-C4); and once for each clear that
  `step` marks as advancing. A request is from the future when it names a
  revision ahead of the automaton's own, so a `step` whose revision departs
  from the automaton's answers some later request wrongly.

The law takes each clear's advance from `step`'s decision rather than
restating which clears advance. It says when an answer must not move, not
when it must, so a refusal where the contracts allow a move passes. A tick's
outcome and its refusal for room both move nothing, so the law does not
check a tick's room. Once a stop begins it checks nothing more, as the
lifecycle law covers the rest. [PROOF.bend](../PROOF.bend) proves it by the same
induction, and [proof/view.bend](../proof/view.bend), which
`step-proof view` generates, holds each fact's lemma; rerun it, too,
after changing `step` or this automaton.

Four sensitivity checks broke `step` in a scratch copy and made the proof
fail: a committed scroll that did not advance authority, a frame that left a
waiting request unresolved, a detach that kept the copy, and a terminal-routed
scroll that moved the viewport. For the first, a scroll and then a preview
from the revision the scroll should have reached break the law itself: that
`step` refuses the preview as from the future. Seven more each break the law
on a one-fact trace from an attached session: a terminal-routed scroll
answered and then followed by a clear and a move, a preview answered with a
move, a scroll without room that moves, a wheel that moves during a held
update, an explicit Copy to the selection clipboard, a finish's copy to the
standard clipboard, and a waiting return to live that a frame on the
alternate screen resolves with a move. As with the other laws, the
proof covers Bend's decisions, not the truth of Rust's facts or their
delivery.

## Management law

The [management lease](#management-lease) laws prove the pure policy; this one
proves that `step`, which keeps the management slot in Bend's session, carries
it out for ORB-C13. [LAWS.bend](../LAWS.bend) states a management automaton,
written from the contract and kept apart from `step`, of the slot as the audit
of [src/management.bend](../src/management.bend) keeps it. It reads only the
management answers among `step`'s decisions: `ManageAdmit`, `ManageBusy`,
`ManageLease`, `ManageRejectIdentity`, `ManageReject`, `ManageStatus` and
`ManageStop`. The law says that over any list of facts with their flags, from
any session state, `step`'s decisions never break the automaton:

- Each management event gets at most one answer, the one the management audit
  requires from the slot the automaton keeps, and the slot follows that audit:
  a connection is admitted only into a free slot and is Busy otherwise, a
  lease is granted only for a valid identity during negotiation, Status and
  Stop are carried out only under the lease, anything out of place is refused
  and keeps the slot until it closes, and a refused connection is answered
  nothing more.
- `ManageStop`, which begins the stop, answers a Stop exactly when the lease
  is held.
- No other fact answers anything on the management socket.

The automaton assumes the host's fact order, which
[HOST.md](HOST.md) states, and once a stop begins, from a Stop or
the child's exit, it checks nothing more, as the lifecycle law covers the
rest. [PROOF.bend](../PROOF.bend) proves it by the same induction, and
[proof/lease.bend](../proof/lease.bend), which `step-proof management`
generates, holds each fact's lemma; rerun it, too, after changing `step` or
this automaton. The whole proof, with the three laws over `step`, checks in
about eleven seconds.

Two sensitivity checks broke `step` in a scratch copy and made the proof
fail: one kept the slot from before each event, so an Acquire after a
connection was answered nothing, and one answered a Stop under the lease with
nothing, which the lifecycle law allows. Both break the law itself on a
connection, an Acquire and a Stop. As with the other laws, the proof covers
Bend's decisions, not the truth of Rust's facts or their delivery.

## Refused requests

Orbit answers every canonical request. A repeated `Hello` or
`ObserveMetadata` gets `Failure(Protocol)`,
and Bend then ends that attachment while the child keeps
running. A request that fails to decode, such as a scroll distance outside
-1,024 to 1,024, does the same with the decoder's detail, and a message with
another ORBS version ends the attachment silently.

A bad first message ends only its peer, whether or not a client is attached,
and Bend answers it: an unsupported ORBS version is
dropped silently, and a malformed header or a message other than `Hello` or
`ObserveMetadata` gets `Failure(Protocol)`. Rust discards a refused message's
payload that has already arrived, so the peer reads the failure rather than a
reset. The real-PTY unsupported-request check covers all three refusals before
the first attachment and again while a client is attached, a failing request
that keeps the client, the repeated, malformed and other-version requests, and
then reattaches to the same child.

## Fatal-exit boundary

A fatal Bend exit cannot leak the child or the socket: the worker holds
neither. The host owns both, watches the worker's decision pipe in every wait,
and when the worker dies or the host is terminated it stops the child
(ORB-C12): a child that has already exited is reaped
without a signal; otherwise its process group and the terminal's foreground
group, when distinct, get SIGHUP, then after 500 ms SIGKILL, with the
foreground group read again, and the direct child must be reaped within two
seconds. It checks the socket's recorded identity before unlinking and leaves
an observed replacement. A termination signal is an orderly stop, so the host
then exits 0 once the child is reaped, or 128 plus the signal if it is not; a
dead worker's status is the host's. The worker exits when its fact pipe
closes, and on Linux it also dies with the host (`PR_SET_PDEATHSIG`), so it
never outlives its pipes.

A worker that stops answering, such as one stopped by SIGSTOP or wedged by a
runtime defect, is treated as dead. The host waits at most two seconds for the
worker's whole answer to one fact, and for its schema line and bounds at
startup. The deadline is a host constant, not a Bend decision, since a stalled
worker cannot decide; answers took at most 7 ms in a vtebench run and in the
paste-flood and pressure checks at a load average near 10 (lj8.80). The host
takes whatever the worker has written before it judges the deadline, so a
complete answer is accepted even when the host itself was paused past it, and
it judges again after each read, so a worker that keeps writing without
finishing its answer is cut off. A worker whose answer is still incomplete
past the deadline has stalled: the host contains the child as for a dead worker, then kills the
worker without waiting for it to exit, and exits 1 with `orbit: worker
stalled: no answer within 2 s`. At startup it gives up before it binds the
socket or starts the child. A termination signal ends the wait as it ends
every other, so a stalled worker cannot delay the orderly stop; before the
child starts, the host kills the worker and exits 0.

The real-PTY containment check exercises the worker's `IO.die`, SIGKILL of
the worker with an interactive client and a metadata observer attached,
SIGTERM of the host with a
HUP-ignoring shell, and SIGTERM with a HUP-ignoring foreground job that the
shell runs in its own process group. Each removes the direct child, the job and
the socket within the check's deadline. The `IO.die` case sets
`ORBIT_GUARDIAN_FAULT=1`, an environment variable outside Orbit's
command line, because no client message can make the proved policy fail. Two
cases stop the worker with SIGSTOP and then send a peer's Hello: in one, the
deadline passes, and the host exits 1 naming the stall with the child, the
socket and the worker gone; in the other, SIGTERM 0.3 s after the Hello reaps
the child before the deadline and the host exits 0. A unit check shows that an
answer already written or read is accepted after the deadline, and that a
worker writing decisions without its end line is cut off at it; it fails when
the deadline is judged before what is ready, and hangs when a pipe that stays
ready is never judged (sensitivity checks). With a
script worker that never answers, the host exits 1 after the deadline without
binding the socket or starting the child, and exits 0 at once when the script
sends it SIGTERM. Without the deadline the first SIGSTOP case hangs, and with a
wait that ignores signals the second reaps the child only after the deadline
(sensitivity checks). The observer-finish check covers what the peers hear
after a termination signal.

When the worker dies, each attached peer's connection ends with nothing to
read; Orbit closes them once the child is contained, about 0.5 s after the
worker's SIGKILL with the HUP-ignoring shell. With the decision pipe left out
of the wait the peers stay connected, and with the observer told Exited, as
after a termination signal, the case fails (sensitivity checks).

The host's own SIGKILL runs none of its cleanup. The
containment check SIGKILLs the host with both peers attached and a shell child
that exits on SIGHUP: each peer's connection ends with nothing to read, the
closed PTY hangs up the child, and on Linux its `PR_SET_PDEATHSIG` sends it
SIGHUP too, so it is gone within two seconds; the worker dies within two
seconds, and the socket pathname stays until the next start reclaims it as a
stale socket and a client reaches the new session. A child that ignores SIGHUP
survives the host's SIGKILL, since no process is left to escalate. The case
fails when the next start leaves a stale pathname in place, when the child
loses its death signal and holds the PTY master open, and when the worker loses
its death signal and holds its own fact pipe open; with only the death signal
removed it passes, since the closed PTY or fact pipe still ends the child or
the worker (sensitivity checks).

Machine failure and detached descendants remain outside this proof. The
identity check is not atomic against a malicious same-UID pathname swap.

## macOS evidence

The host runs the same loop on Apple Silicon macOS. Its deadlines, the
synchronized-output watchdog and both negotiation bounds, are instants the
wait polls until, not timer descriptors; pipes are made close-on-exec with
`fcntl` where `pipe2` is missing; the management lease reads the peer's UID
with `getpeereid` and the host's start identity from `proc_pidinfo`; and
only Linux gets `PR_SET_PDEATHSIG`, elsewhere the worker exits on its closed
fact pipe and the child on its closed PTY. macOS reports POLLHUP
for a client's half-close as well as for its full close, so the host tells
them apart with a zero-length send, which fails only once the client can no
longer read, and checks a half-closed client every 100 ms.
[build.rs](../build.rs) passes `-Wl,-z,noexecstack` to clang only on Linux.

No macOS proof covers this tree. The last native run was in the bend-orbit
repository at revision b017e4f0, on a Scaleway Mac mini M2 (16 GiB) with macOS
26.7.1 (25G241), built natively with Rust 1.95.0, zig 0.15.2 and bun 1.3.13.
There every real-PTY check passed through `cargo test`, 36 of 36, the Rust
unit tests passed, 24 of 24, `PROOF.bend` printed `ALL PROOFS CHECK`, and
`tests/lifecycle.rs`, with its binary lookup redirected, passed 15
of 15; its other three tests are `cfg(target_os = "linux")`. The imported code
differs from b017e4f0 in its names, its schema digest, its worker lookup and
its `orbit-protocol` path dependency, so that run is evidence about the
design, not a proof of this revision. zig 0.15 cannot link against the macOS
26 SDK, whose `libSystem` stubs list `arm64e` but not `arm64`; that run
answered `xcrun --show-sdk-path` with the Command Line Tools' 15.4 SDK.

The real-PTY checks find processes with `pgrep` and `ps` and read `/proc` only
on Linux, using `lsof` elsewhere. macOS applies no read timeout to a peek and
refuses socket options on a connection its peer has closed, so the harness
polls before it peeks and ignores a refused timeout.

## Throughput

[tools/perf/orbit-venus-baseline.sh](../tools/perf/orbit-venus-baseline.sh)
runs any server that takes Orbit's command line, the old Orbit included.
[tools/perf/headless-throughput.rs](../tools/perf/headless-throughput.rs) runs
that harness's vtebench command without a desktop. Each server runs it
detached, with no client attached once the benchmark starts, and then again
with a client that reads and discards every message as fast as it can. Once a
second the harness also counts the runnable threads outside the benchmark, and
it prints them after the table with the load.

These measurements predate the import and come from the bend-orbit
repository. In both tables, Bend is bend-orbit at the named revision and Old is
the old Orbit at Eon 1d287440. The first table measures bend-orbit at b017e4f0
against the old Orbit, both built by bend-orbit's flake, with
vtebench ead80032 at a 58x93 grid on CPUs 6-11 of an i5-13450HX. It comes from
three runs, each of three interleaved repetitions of five samples, and each
cell is the median of the three runs' medians. Each run started below a 1-min
load of 2, but other work kept running: in the third run, the threads outside
the benchmark averaged 1.28 runnable, and the benchmark's own 1.65. From run
to run, the old Orbit's own median for one benchmark moves by up to 40%
(`scrolling_bottom_region` detached, 170 to 238 ms), and a single run can put
either server more than 10% ahead:

| Benchmark | Old detached | Bend detached | Old attached | Bend attached |
| --- | ---: | ---: | ---: | ---: |
| cursor_motion | 6 ms | 6 ms | 219 ms | 196 ms |
| dense_cells | 28 ms | 27 ms | 721 ms | 698 ms |
| light_cells | 3 ms | 3 ms | 185 ms | 175 ms |
| medium_cells | 8 ms | 7 ms | 169 ms | 165 ms |
| scrolling | 196 ms | 193 ms | 303 ms | 282 ms |
| scrolling_bottom_region | 205 ms | 191 ms | 298 ms | 305 ms |
| scrolling_bottom_small_region | 203 ms | 192 ms | 308 ms | 293 ms |
| scrolling_fullscreen | 8 ms | 7 ms | 184 ms | 169 ms |
| scrolling_top_region | 216 ms | 187 ms | 309 ms | 294 ms |
| scrolling_top_small_region | 203 ms | 186 ms | 294 ms | 301 ms |
| sync_medium_cells | 8 ms | 7 ms | 11 ms | 15 ms |
| unicode | 3 ms | 3 ms | 159 ms | 158 ms |

By the median of the three runs' paired gaps, Bend stays within 10% or 5 ms of
the old Orbit on every benchmark, detached and attached. Detached, it matches
the old Orbit within 1 ms on the light benchmarks and is 2 to 13% faster on the
scrolling ones. Each fact costs a round trip through the worker, so while no
client is attached the host drains the PTY, up to 512 KiB, before asking Bend
([HOST.md](HOST.md)); when the host asked once per 64 KiB at most, the scrolling
benchmarks were up to 27% slower than the old Orbit. With a client attached, both
servers publish a frame for every PTY read, so frame production dominates:
Bend is at most 3% slower on every benchmark but one, and up to 11% faster.
That one is `sync_medium_cells`, 4 ms slower in the median and 3 to 5 ms slower
in every run, where both stay close to their detached times. Its client reads
about 3.6 GB per run against the old Orbit's 3.7 GB. The harness also counts the
frames the client reads and keeps the last frame's revision; in these 18
attached runs every revision reached the client (the frame count is the last
revision plus one) and frames averaged 133 KB from both servers, but
bend-orbit took 26,752 PTY turns on average against the old Orbit's 27,668. Both
read at most 8 KiB per turn and publish one frame for each, and vtebench
writes the same output to both, so bend-orbit's reads are fuller on average,
likely because each of its turns also waits for a round trip through the
worker. ORB-C4 orders complete frames without fixing their number, so this is
no contract difference. The last in-process build (`9ef0e61f`), where Bend
called the Rust mechanisms through a C adapter, was 15 to 28% faster than the
old Orbit on the detached scrolling benchmarks in the same conditions. Neither the move to
libghostty-vt 0.2.2 nor the host's later restructuring explains that
difference, and bend-orbit did not pursue it. The client does not
render, so this does not replace the harness run with Venus.

The Venus harness ran unchanged with native Venus (Eon a14a90cc) on the same
CPUs, at Venus's 54x90 grid, for the old Orbit and for bend-orbit at
f096691c, each starting below the harness's load limit (1.77 and 1.27) and
under its compositor CPU limit. Medians of 15 samples:

| Benchmark | Old detached | Bend detached | Old attached | Bend attached |
| --- | ---: | ---: | ---: | ---: |
| cursor_motion | 5 ms | 5 ms | 168 ms | 158 ms |
| dense_cells | 25 ms | 23 ms | 582 ms | 598 ms |
| light_cells | 3 ms | 3 ms | 165 ms | 169 ms |
| medium_cells | 7 ms | 7 ms | 150 ms | 150 ms |
| scrolling | 219 ms | 195 ms | 260 ms | 258 ms |
| scrolling_bottom_region | 207 ms | 192 ms | 259 ms | 253 ms |
| scrolling_bottom_small_region | 216 ms | 189 ms | 255 ms | 251 ms |
| scrolling_fullscreen | 8 ms | 8 ms | 162 ms | 161 ms |
| scrolling_top_region | 202 ms | 207 ms | 273 ms | 235 ms |
| scrolling_top_small_region | 208 ms | 189 ms | 267 ms | 241 ms |
| sync_medium_cells | 7 ms | 7 ms | 11 ms | 15 ms |
| unicode | 3 ms | 3 ms | 133 ms | 147 ms |

With Venus rendering, the two stay within 3% on most attached benchmarks.
Bend is 10% slower on `unicode` and 4 ms slower on `sync_medium_cells`, the latter as
in the headless runs, and 4 to 14% faster on the top-region scrolling ones;
detached, it matches the old Orbit and is up to 13% faster on scrolling. Each
server ran once, not interleaved, so drift between the two runs stays in
these numbers. To repeat it on an idle desktop, inside `nix develop` from the
repository root, where a cargo-built host runs the worker its build made:

```sh
cargo build --release --locked -p yazelix-orbit
ORB_BASELINE_CPUS=6-11 ORB_BASELINE_GRID="54 90" \
  orbit/tools/perf/orbit-venus-baseline.sh throughput \
  "$PWD/target/release/yazelix-orbit" VENUS_BIN ../vtebench EVIDENCE_DIR
```

## Check

`nix flake check -L` from the repository root runs Orbit's checks, from
[nix/orbit-checks.nix](../../nix/orbit-checks.nix), with the rest of Eon's:

- `orbit-laws` runs [PROOF.bend](../PROOF.bend) with the pinned Bend compiler
  and requires `ALL PROOFS CHECK`.
- `orbit-boundary` regenerates the boundary from
  [src/boundary.schema](../src/boundary.schema) and fails while the committed
  [src/boundary.rs](../src/boundary.rs) or
  [src/boundary.bend](../src/boundary.bend) is stale.
- `orbit-terminfo` checks the package's `eon` entry
  ([Terminal identity](#terminal-identity)).
- `orbit-NAME` runs the real-PTY check NAME against the package, for each
  real-PTY check this document names: lifecycle, guardian-containment,
  delayed-output, request-half-close, detached-exit,
  three-attach, late-observer, detached-observer, synchronized-output,
  held-attach, history, preview, scroll, return-to-live, schema-mismatch,
  paste-flood, terminal-replies, silent-peer, pty-reopen, descendant-exit,
  unsupported, interactive-pressure, key-live, mouse, slow-observer,
  observer-finish, stale-socket, clipboard, clipboard-pressure, command-line,
  palette-reset, selection, selection-tick, refused-flush, management-record,
  management-lease, terminal-identity and stopped-client-exit.

Building the package runs the Rust unit tests. The real-PTY checks are Rust
integration tests in [tests/real_pty](../tests/real_pty). They encode every
request and decode every answer, frame and management record with the
canonical `orbit-protocol` codec, as clients do. One harness,
[tests/real_pty/support.rs](../tests/real_pty/support.rs), starts the server on
Orbit's command line, finds its PTY child with `pgrep` and `ps`, opens the FIFO
gates the child scripts (tests/NAME-shell.sh) wait on, and checks how the
session ends. `cargo test` builds them and leaves them ignored. The package
keeps their test binary in its `tests` output, and each flake check runs one
test with the child script and timeout its table entry names. A check that
runs for three minutes terminates the servers it started and ends the run.

Inside `nix develop`, from the repository root, cargo builds the host and
[build.rs](../build.rs) builds the worker with the pinned Bend compiler and
clang. The real-PTY checks then run one at a time in `eon-test`'s sandbox,
which passes `ORBIT_*` and `MISMATCHED_WORKER` through,
against a cargo-built host, which runs its build's worker, and take each child
script from the source tree:

```sh
cargo build --release --locked -p yazelix-orbit
ORBIT_BIN=$PWD/target/release/yazelix-orbit \
  eon-test --release -p yazelix-orbit --test real_pty -- --ignored
```

The terminal-identity check also needs the `eon` entry in `share/terminfo`
beside the directory that holds `ORBIT_BIN`, as in the package, and a plain
`bash` in `ORBIT_BASH`, since a wrapper that changes `TERMINFO_DIRS`, as Eon's
does, fails it. The schema-mismatch check needs `MISMATCHED_WORKER`, a worker
generated from the schema plus one flag:

```sh
rustc --edition 2024 orbit/tools/boundary.rs -o target/boundary
cp -R orbit/src/. target/mismatched
echo 'flag 31 spare' >> target/mismatched/boundary.schema
target/boundary target/mismatched/boundary.schema target/mismatched
(cd target/mismatched && bun "$BEND_SOURCE_DIR/bend2/main.ts" main.bend -o ../mismatched.c)
"$BEND_CLANG" -std=c11 -O2 target/mismatched.c -lpthread -lm -o target/mismatched-worker
export MISMATCHED_WORKER=$PWD/target/mismatched-worker
```

The black-box suite, [tests/lifecycle.rs](../tests/lifecycle.rs),
runs unmodified against Orbit with Eon's workspace suites, or alone:

```sh
nix develop -c eon-test -p yazelix-orbit --test lifecycle
```
