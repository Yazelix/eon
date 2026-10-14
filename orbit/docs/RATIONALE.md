# Orbit rationale

Orbit is an architecture experiment. This file records its origin, hypothesis,
and stop conditions. The README and `AGENTS.md` define the current boundaries,
the [contract index](CONTRACTS.md) records accepted behavior and proof status,
and Beads define authorized work.

## Where the idea came from

Yazelix Nova composes three independently evolved layers:

```text
yzx -> Mars -> Yazelix Zellij fork
```

Mars provides the graphical terminal, while Zellij owns durable PTYs, terminal
state, panes, and session behavior. The multiplexer and graphical terminal each
interpret terminal behavior and follow different ownership models.

We arrived at the Orbit hypothesis after examining three projects:

- [TUIOS](https://tuios.gaurav.zip/) treats the terminal as an entire workspace.
  Its feature set also shows how tiling, workspaces, themes, scripting, and
  window-management policy can overtake the core session problem.
- [Herdr](https://github.com/ogulcancelik/herdr) uses a Rust client/server
  multiplexer to keep real agent terminals alive across detach/reattach. It is
  close prior art for the use case. Herdr also covers agent state, layouts,
  remote access, and multiplayer concerns that Orbit excludes.
- Mitchell Hashimoto's public [Superlogical](https://www.superlogical.com/)
  discussion,
  [architecture explanation](https://x.com/mitchellh/status/2082936029426892960),
  and
  [libghostty roadmap](https://mitchellh.com/writing/libghostty-is-coming)
  led us to the ownership inversion: terminal multiplexers already need
  terminal-emulation state, and that state can come from the same reusable core
  as a graphical terminal.

We combine those observations for Yazelix. Superlogical has disclosed a
checkpoint-plus-tail design: its server keeps authoritative libghostty state,
sends connecting clients a binary reconstruction snapshot, then streams raw PTY
bytes to a terminal emulator in each client. It can declare a client ready
before transferring older scrollback. Orbit shares the durable PTY,
server-authority, and attachment-barrier principles while using a different
replication boundary. Venus receives complete host-authored
presentation frames and does not parse PTY output or reconstruct libghostty
state. This repository uses the published description **Superlogical terminal
multiplexer** rather than inventing a product codename.

## The hypothesis

A conventional graphical-terminal and multiplexer chain resembles:

```text
application -> PTY -> multiplexer terminal model -> composed VT stream
            -> graphical terminal model -> pixels
```

We are testing a narrower ownership model:

```text
application -> PTY -> Orbit authoritative libghostty state
                              |
                  structured presentation frames
                              v
                         Venus renderer -> pixels

semantic client input and resize --------------> Orbit -> PTY
```

Orbit owns process lifetime, the PTY, and authoritative terminal state. The
client owns presentation and user interaction. On attachment, Orbit establishes
one atomic boundary: the client receives a coherent complete structured frame
at one revision followed by later frame revisions in order. Hidden terminal
state stays in Orbit, and semantic client input is encoded against that state.
The client does not parse a replicated PTY stream or run another terminal
emulator.

We start with one local session and one active client, giving input and resize
one unambiguous owner and isolating presentation transfer from workspace policy.
The boundary must account explicitly for rich presentation state instead of
assuming that a plain-text cell grid is the final product contract.

## Replication boundary

ORBF carries complete presentation frames. This boundary proves attachment
convergence. Any future incremental protocol keeps a complete frame as its
resync fallback. Orbit has made no permanent decision to use full frames for
every steady-state update.

The preferred later shape, subject to a separate user-approved protocol slice,
divides the connection into six planes:

| Plane | Owner and semantics |
| --- | --- |
| Snapshot | Orbit sends one coherent current presentation at attach, resync, or a discontinuity |
| Patch | Orbit sends absolute row replacements and changed global fields with `epoch`, `base_revision`, and `revision` |
| History | Orbit serves bounded pages by stable anchors; the attach snapshot does not carry the full scrollback |
| Effects | Orbit preserves ordered bells, clipboard requests, and notifications outside coalescible screen state |
| Resources | Orbit transfers capability-negotiated, content-addressed images and other rich payloads independently of effect ordering |
| Actions and control | Venus sends semantic input, resize, lifecycle, and bounded history requests; Orbit applies them against authoritative state |

Absolute row replacements keep patch composition independent of terminal
operations. Orbit may merge changed rows into one queued successor when the
base revision remains known. A revision gap, resize discontinuity, capability
change, or ambiguous merge causes a complete snapshot instead. Presentation
state can be coalesced; Orbit preserves effect identity and delivery order.

`libghostty-vt` 0.2.1 exposes global and per-row dirty state through
`RenderState`. That API supplies the incremental extraction primitive without a
new dependency. Orbit should activate it when measurements show that
full-frame extraction or transfer limits the first Venus client. The decision
must measure extraction time, encoded bytes, queue replacement, and client apply
time under shells, Neovim, Yazi, resize storms, and high-volume output.

Venus decodes canonical ORBF frames into persistent presentation state before
drawing. A later patch decoder can update the same model. Orbit remains the
schema owner; Venus must not handwrite a second interpretation. The
[contract index](CONTRACTS.md) records accepted producer and consumer revisions.

The original `orbit-protocol` boundary provided ORBF v1 decoding and complete-frame
reduction plus ORBS v1 attachment, frame-envelope, lifecycle, semantic-input,
paste, and resize. ORBS v2 replaced that session revision, retained those message
families, and added revision-bound selection and bounded copy. The accepted ORBS
v3 producer retains those values and carries bounded normalized clipboard
writes as ordered effects outside replaceable presentation frames. Orbit adds
revision-bound adjacent-row previews and typed vertical-wheel outcomes in ORBS
v4. Venus source `a768e9a1bcb61eac5a21d25b7463c9dc44aa2df8` pins exact ORBS v4 proof
`7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`; Eon acceptance
`0bf0b165d06b4a8162be497011070f61f6c2000a` composes the exact accepted
Orbit and Venus sources. ORBS v5 replaces v4 at Orbit proof
`69c402737799f03e615473956954a043647a4713`: the existing presentation socket
negotiates either the sole input-capable attachment or one bounded read-only
title/CWD observer. This is the minimum live inactive-pane result; it adds no
second endpoint, terminal text observation, agent authorization, or process
authority. Venus and Eon adopt the exact owner proof in that order without a
dual-version window. Accepted ORBS v6 supersedes the unconsumed v5 producer and
adds one bounded signed whole-row viewport commit with one resulting frame and
adjacent preview; pixels, gesture timing, and momentum remain Venus-owned.
Accepted ORBS v7 replaces that adjacent row with up to one active viewport
of nearest-first canonical rows. Orbit still owns history and mutation; Venus
uses the bounded window only for compositor-paced fractional presentation.
Accepted ORBS v8 replaces client-authored selection cells with bounded
surface positions and monotonic press time. Orbit maps positions through the
authoritative surface geometry, and the pinned libghostty gesture state owns
cell, word, logical-line, and matching drag granularity. Accepted ORBS v9 adds
current modifiers to every positional phase and moves terminal-versus-host
routing into Orbit, where authoritative mouse tracking already lives. Shift
forces host selection, the Begin route remains fixed, release returns frozen
text for the semantic selection clipboard, and explicit Copy targets the
ordinary clipboard. Venus still owns native event delivery, focus and pointer
capture cancellation, and native clipboard effects.

Orbit owns history because the authoritative terminal supplies wrap metadata,
row contents, and stable anchors. Venus owns native pointer events, but sends
their revision, bounded surface position, and monotonic press time to Orbit;
Orbit owns position mapping, gesture semantics, selected presentation, copy
extraction, and content semantics. Once extraction succeeds, Orbit keeps that
bounded frozen text independent of later terminal output, resize or reflow, and
active-screen transitions until a new selection or client-lifecycle boundary
clears it. Agent observation or durable session history
needs a separate Eon or Orbit observer contract; the presentation stream is
not a durable event log.

## Conditional client-side terminal replicas

Superlogical's checkpoint-plus-tail design reduces steady-state protocol work
and lets each native client use libghostty features as they appear. Orbit cannot
build that design from `libghostty-vt` 0.2.1: its Formatter loses hidden
continuation state, and the release exposes no exact terminal checkpoint import
and export contract.

Orbit should reconsider a raw-byte client replica when public libghostty
supplies all of these capabilities and a user chooses the change:

- versioned exact export and import for parser partials, both screens, modes,
  scrollback, graphics, and other continuation state;
- client-side suppression of PTY replies and duplicate terminal effects;
- deterministic drift detection and complete resynchronization;
- a counterexample corpus that covers the formatter failures already recorded
  in Orbit;
- lower measured total owned code and maintenance cost than the structured
  protocol; and
- a supported public API with no libghostty fork or private binding extension.

Until those conditions hold, a client-side terminal creates a second
interpretation of state and side effects. Orbit keeps raw PTY bytes inside the
authoritative process and sends materialized presentation to Venus.

## Why it may be better

- A client can disappear without taking the shell or its foreground process
  with it.
- Venus gains persistence without Zellij's composited terminal stream or
  compatibility surface.
- Orbit is the only terminal-semantics owner; clients can use native, web, or
  mobile rendering stacks without reproducing hidden terminal state.
- Complete frames and monotonic revisions form a precise attachment contract
  that can be tested with real PTYs.
- A successful design gives Venus a small session owner instead of inheriting a
  general-purpose multiplexer architecture.
- Starting clean permits ownership and tests to be stricter than Mars Next
  before compatibility makes mistakes expensive.

## Why it may fail

The visible grid is not complete terminal state. Correct continuation can also
depend on cursor and keyboard modes, alternate-screen state, tab stops,
character sets, palette changes, hyperlinks, images, scrollback, and a parser
partway through an escape sequence. Orbit retains that hidden state, but a
presentation frame must still expose every user-visible consequence without
silently flattening styled text, metadata, or graphics.

Other material risks are:

- libghostty's embeddable APIs may not expose the read-only presentation state
  required by a headless owner;
- obtaining that presentation state may require a large fork or unstable
  private API;
- extracting and publishing frames atomically may introduce stale revisions,
  excessive copying, or unbounded slow-client buffering;
- a client-neutral rich presentation schema may become a maintenance and
  compatibility surface of its own;
- the Orbit process becomes a failure boundary for every PTY it owns;
- a one-session experiment is not yet a replacement for Zellij's workspace
  features;
- native-window and rendering integration may outweigh the simplicity gained
  in the session layer.

The feasibility and convergence work address these risks before UI polish.

## Why Orbit is a separate process

[Canario](https://rioterm.com/canario) keeps its application source private and
distributes signed builds through
[`canarioterm/releases`](https://github.com/canarioterm/releases/tree/fcadcfeea7e337114d28beec0e0cbe89493a654d).
Rio commit
[`ff8efcc4ad03`](https://github.com/raphamorim/rio/tree/ff8efcc4ad0359889de44b1baacea7014cdada30/frontends/canario),
the final public Canario tree before its extraction, is the strongest
inspectable compact counterexample to the Orbit boundary. It combines a native
SwiftUI/AppKit workspace with librio surfaces in one application. The frontend
can pull materialized terminal state and ship spaces, splits, a command palette,
CWD routing, a quick terminal, and on-demand previews without a separate
versioned session protocol. That shape fits a product contract where restarting
shells and restoring workspace presentation is sufficient.

It does not meet Orbit's contract. At the inspected revision, librio owns each
PTY inside the application process. Canario's persistence path starts fresh
shells: SessionStore saves layout, CWD, title, and plain-text scrollback, and
RioEngine injects that text into a new terminal display. Closing the application
therefore ends the live processes; visual restoration is not detach and
reattach.

Orbit exists only to make that lifetime boundary real. Graduation must compare
its extra process, repository, and protocol cost against Canario's simpler
integrated shape. If native dogfood does not make process survival, independent
failure, replaceable clients, and authoritative structured state materially
valuable, the separate boundary has not earned its cost and the experiment
should stop.

## How references are used

Orbit studies prior art just in time rather than choosing an entire stack in
advance. Each implementation bead names the projects and crates that can answer
its immediate questions. The
[reference map](REFERENCES.md) records what to inspect, what each source can
teach Orbit, and which surrounding features remain outside the experiment.

Reference study is part of the evidence for completing a bead. It records a
release or commit where behavior matters, extracts a relevant contract or
failure mode, and states what Orbit accepts or rejects. It does not authorize a
dependency, fork, compatibility layer, or scope expansion. Those choices still
require the user. When a dependency choice is in scope, the separate
[crate decision index](CRATES.md) and crate gate compare credible
implementation shapes, including using no new crate, before the manifest
changes.

## Independent validation without shared authority

The three greenfield components do not need to advance as one inseparable
implementation. Each owner can expose bounded evidence that lets the next
component validate its own boundary while only one architectural frontier is
active:

- Orbit can be exercised by diagnostic clients and comparison oracles that
  consume its proven structured frames without parsing PTY output or owning
  terminal state.
- Venus can validate decoding and frame-to-draw behavior from a small,
  versioned, Orbit-generated frame corpus or protocol-faithful replay source
  pinned to an exact Orbit proof revision before depending on a live PTY for
  every development cycle.
- Eon can validate Eon-owned package composition, launch arguments,
  environment, ordering, and failure reporting with explicit process probes;
  those probes do not claim to prove Orbit runtime or Venus rendering.

These validation surfaces stay subordinate to the product contracts. A lossy
terminal projection can prove attachment, ordering, reconnect, input, or
backpressure behavior, but not rich-presentation fidelity. A replay source must
reuse the canonical Orbit wire representation rather than grow a mirror schema
or alternate server contract. Recorded frames are bounded test evidence, not
restart persistence, unbounded history, or a public trace format. Any public
command, durable format, adapter, package, or additional binary remains a user
scope decision.

## Why this is uncommon

Mature multiplexers have kept PTYs alive for decades. Orbit adds the requirement
of reconnecting graphical clients to a host-owned rich presentation without
nesting another terminal emulator in the path.

Historically, terminal-emulation cores were embedded inside applications rather
than exposed as stable libraries. Existing multiplexers also gain portability
by presenting a conventional terminal to many unrelated clients, so their
architecture favors broad compatibility, remote attachment, and shared
sessions over a native presentation protocol. A structured boundary avoids
transferring parser state, but rich cell, metadata, graphics, ordering, and
backpressure semantics remain hard.

Libghostty gives us a plausible way to test this arrangement. Orbit determines
whether its APIs make the design small enough for the narrower Yazelix use
case.

## Why multiplayer is excluded

Multiple attached clients introduce shared input arbitration, focus, resize
authority, slow-client backpressure, clipboard and terminal-side effects,
authorization, and observable concurrency semantics. Those concerns would
shape the protocol before Orbit proves its basic value.

Yazelix serves one local user, and normal Git and agent workflows cover the
collaboration this project needs. We exclude multiplayer from the protocol and
from postponed scaffolding.

## Why Mars Next is not the base

Mars Next demonstrated that a Rust/libghostty direction could compile and
support small isolated checks. Its live-session work did not establish the
real-PTY, detach/reattach, socket, or terminal-convergence contracts on which
Orbit depends, and its default verification could omit Ghostty-backed paths.

Because the code is still small and unused as a compatibility surface, a clean
start is cheaper than trying to infer and repair its ownership model. Mars Next
remains a behavioral reference. Reuse requires a specific contract and an
explicit user decision.

## What Orbit would replace

Orbit would replace the process, PTY, and session-lifetime role that a
conventional multiplexer such as Zellij would otherwise provide in Eon. Venus
would provide graphical presentation through Eon Desktop, while Eon would own
only product orchestration, policy, and configuration above both subsystems.

Eon is not a migration of Nova and does not make Nova the compatibility path
for the experiment. Nova remains an independently valuable product on its
current Mars and Zellij architecture. The two lines grow separately unless the
user later chooses a specific shared child-repository contract.

Venus consumes Orbit's structured presentation boundary, while the current Mars
application owns its terminal state directly. The experiment includes the
smallest Venus client needed to prove the one-authority design. Orbit owns the
headless proof through `orb-bi4.3`. That convergence gate passed, and the
private [`Yazelix/eon-desktop`](https://github.com/Yazelix/eon-desktop)
repository now owns graphical implementation through Venus; `orb-bi4.4` tracks
Orbit-side acceptance. Graphical Venus code does not live in Orbit; the proven
presentation contract is the repository boundary.

## Naming and eventual ownership

The product is named **Eon**. The repositories are **Eon**, **Eon Desktop**, and
**Eon Sessions**; Venus and Orbit remain their underlying client and session
subsystems. `Saturn` remains reserved for possible future use; that reservation
creates no product, repository, feature, or planning scope.

Orbit retains the headless runtime and diagnostic client used by its proof. Eon
Desktop owns the graphical client through Venus. If Venus and Orbit graduate,
the intended product boundary becomes:

```text
Eon
|-- Eon-owned orchestration and product policy/configuration
|-- Eon Desktop / Venus: graphical terminal client
`-- Eon Sessions / Orbit: durable local session runtime
```

Orbit survives as the runtime subsystem name. Venus remains distinct from Mars.
A failed experiment leaves Nova and its Mars and Zellij path unchanged. A
successful experiment gives Eon its own architecture while Nova continues to
grow independently.

## Evidence sequence

Beads order the work:

1. Prove the libghostty attachment model with deterministic state.
2. Own one real PTY and survive diagnostic-client detachment.
3. Complete `orb-pmp` to audit and isolate the Linux platform seam without
   implementing macOS.
4. Prove race-free structured presentation convergence on reattach.
5. Complete `orb-9o6` to automate the machine-checkable governance invariants
   after the protocol has two implementation slices of evidence.
6. Activate Eon Desktop, choose the Venus protocol-code and rendering
   boundaries, and add one minimum native client.
7. Harden the one-client boundary.
8. Dogfood Eon Desktop and Eon Sessions through a minimum Eon-owned
   integration path.
9. Decide whether the Venus and Orbit subsystems earn graduation into Eon.
10. Choose later Eon, Venus, and Orbit ownership expansions from evidence.

A failed contract returns the project to planning before the next step.

## Graduation criteria

Graduate Eon Desktop and Eon Sessions into Eon only if all of these are true:

- reattaching clients receive coherent rich presentation while hidden terminal
  state remains authoritative in Orbit;
- the PTY survives client failure and has explicit exit semantics;
- rapid output, alternate screens, Unicode, partial escape sequences, and
  resize boundaries behave deterministically;
- a native Venus client can use the structured boundary without a second
  terminal emulator, broad adapter, or libghostty fork;
- fresh Eon dogfood is useful with shells, Neovim, Yazi, full-screen
  TUIs, and long-running processes;
- the resulting ownership and LOC are smaller than continuing the current
  Mars/Zellij chain or Mars Next;
- direct and architecture-shaping crate choices have evidence-backed reasons,
  recorded alternatives, and acceptable owned-LOC, build, and maintenance
  cost;
- Linux-specific runtime mechanics remain behind a narrow platform seam, while
  core ownership and cross-repository contracts have no unrecorded macOS
  blocker;
- no known P0 or P1 correctness risk remains.

## Stop criteria

Stop and return to planning if any of these become true:

- a sufficient read-only presentation surface requires maintaining a large
  libghostty fork;
- rich visible terminal state cannot cross the presentation boundary without
  silent loss or a text-grid ceiling;
- correct attachment requires retaining unbounded frame history;
- atomic frame extraction and publication cannot be made race-free without
  disproportionate protocol machinery;
- the client needs another terminal-semantics implementation or PTY parser;
- the smallest usable slice grows into layout, compatibility, multiplayer, or
  remote-session work before proving detach/reattach;
- macOS would require redesigning core ownership, presentation or input
  contracts, or durable state rather than adding a bounded platform backend;
- dogfood shows no meaningful advantage over the simpler existing options.

If we stop, we keep the evidence and avoid another untrusted multiplexer
implementation.
