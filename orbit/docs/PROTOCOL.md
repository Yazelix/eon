# Session protocol

ORBS carries exact-version session messages. ORBF carries complete terminal
frames inside those messages; [`orbit-protocol`](../crates/protocol) owns both
codecs.

## Framing and validation

ORBS v13 starts with a 12-byte little-endian header: `ORBS` magic, exact
revision, typed message kind, zero reserved flags, and bounded payload length.
The incremental decoder rejects unsupported versions, wrong-role or unknown
kinds, and impossible message declarations before payload-sized buffering. It
then checks typed values, truncation, and trailing bytes.

## Semantic messages

- Paste bytes stay opaque. Keys retain physical identity, action, modifiers,
  composition, text, and an optional unshifted codepoint.
- Left-pointer phases carry bounded surface positions and modifiers. Begin adds
  the presented frame revision and monotonic press time. Orbit chooses terminal
  or host routing; libghostty handles cell, word, and logical-line host selection.
- A vertical preview names an already-presented frame. Orbit returns up to one
  viewport of nearest-first rows from current state without moving the viewport
  or writing to the PTY. Finish reports the authoritative presentation revision.
- A wheel result is either terminal-routed or an applied row count plus a newer
  complete frame. A signed viewport commit moves at most 1,024 rows and returns
  requested/applied distance, a newer frame, and a bounded row window or edge.
  Requests claiming a future revision fail before mutation.
- `ReturnToLive` jumps an eligible primary viewport to live output in one frame.
  Terminal-owned and alternate-screen routing reject it; synchronized output
  defers it; output pressure fails before mutation.
- `AutoscrollUp` extends an active host selection after moving at most one
  retained row upward. Zero applied rows marks the oldest edge. Ordinary
  signed scrolling clears selection.

The session layer embeds canonical ORBF frames without interpreting them again.
Copy and gesture cancellation remain explicit; terminal authority stays in Orbit.

## Contract and version history

Orbit keeps a real terminal process and the sole authoritative libghostty state
alive independently of its graphical client. One local input-capable client
may attach at a time, while one read-only metadata observer may coexist. On
attachment, the input-capable client receives a coherent complete structured
presentation frame at one revision followed by later frame revisions in order.
The observer receives only live title, working directory, revision, and
lifecycle metadata. The client sends semantic input for Orbit to encode against
authoritative terminal state. The initial contract covers surviving client and
observer exits and disconnections while Orbit continues running; it does not
cover Orbit or machine restarts.

The first accepted cross-repository integration used Venus source
`e5e37a3df119ee2bcfa2493a2ce5a46307493732` with canonical ORBF v1 over
ORBS v6 at exact Orbit proof `780f5d746175b4a9b71df57c51ed4bfcc4c4c375`. Eon source
`10c402b3754d600a6bb0a0da0de2ff6d1d4feca3` composes that accepted pair;
Eonova source `92acf64e8c43531bd4c5d639bdfa99b717dfe5b3` consumes the resulting Eon
runtime. Accepted x86_64 Linux evidence
covers same-boot recovery, owner-routed Stop, and native Wayland delivery to the
primary selection. Ordinary clipboard delivery, Wayland without data-control,
broader compositors, and macOS Venus/Eon composition remain unproved. No
adapter, feature probe, dual-version support, or compatibility window exists.

Later exact revisions changed the same boundary without a compatibility
window:

| Version | Accepted source | Change |
|---|---|---|
| ORBS v7 | `baf8aa28dcaa50484cd221aa7730defedc2356bb` | Expanded one-row preview to a bounded row window. |
| ORBS v8 | `d9b22eb294f8f42b4f49324fd5467eab239c2917` | Replaced client-authored viewport cells with bounded pointer positions and press time. |
| ORBS v9 | `aed0bcb7e9ad08c8e3e086c7dad0a0eb3ef16672` | Moved terminal/host left-pointer routing into Orbit; Shift bypasses mouse capture. Release and explicit copy target distinct clipboards. |
| ORBS v10 | `59975e9176f5caf8b78dc3273e88d9ecbb75dc3f` | Finish reports its exact presentation revision for rapid pointer sequences. |

### Replication boundary

Complete frames prove convergence and define the attach boundary. Any later patch protocol keeps a complete frame as its resync
fallback. The preferred later replication shape uses revisioned row patches
during steady operation, bounded history pages, and a separate ordered stream
for terminal effects. That direction changes no current contract and requires a user-approved protocol
decision plus measured evidence before implementation. The
[design rationale](RATIONALE.md#replication-boundary) records the model and
its recovery rules.

The [contract index](CONTRACTS.md) assigns stable `ORB-C*` identities to
accepted behavior and records its owner, proof status, checks, and remaining
gap. Candidate implementation evidence does not become proved until its owning
Bead is accepted and the exact proof-bearing commit is recorded.
