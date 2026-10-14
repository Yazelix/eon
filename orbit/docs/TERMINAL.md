# Terminal state and attachment

## Current implementation

Orbit owns one real PTY and the authoritative libghostty terminal. A client can
disconnect and later attach to the same live process. Orbit sends complete,
structured frames; it does not ask clients to parse raw terminal output.

The ordinary and managed server and diagnostic client are proved on x86_64 Linux
and Apple Silicon macOS. Native macOS proof covers PTY spawn, input, resize,
reconnect, socket permissions, EOF, bounded shutdown, and management Stop.
Venus, Eon, Nix packaging, signing, and distribution have not adopted macOS.

### Scrollback and wire versions

Each ORBF v2 frame reports two wrapped display-row counts:

- `rows_from_live` is the distance from live output, zero at the bottom.
- `history_rows` excludes the active viewport; distance cannot exceed it.

Both are zero on the alternate screen. Output, scrolling, reflow, pruning,
clear/reset, and reattachment publish the counts with the same revision as the
cells. Extraction failure does not publish an estimated position.

| Producer | Accepted source | Change |
|---|---|---|
| ORBS v11 / ORBF v2 | `ea9fd28ce0908f218cf65d4e6df368f0a4e565f5` | Complete frames carry scrollback position. |
| ORBS v12 | `f8ad14e5195109ba8cb421f30e5ae4a9619a1419` | Return to live output; pinned by Venus and Eon. |
| ORBS v13 | `b6cecf8f2ee35570b41cfdc578b095889d917fe2` | One upward host-selection tick with an applied distance and complete frame. |

Wire versions are exact and have no adapter. v13 has Linux headless producer
proof; its Venus pointer cadence and installed Eon acceptance are separate work.
The [contract index](CONTRACTS.md) records the exact proofs and gaps. Eonova
remains an independent product line outside Eon's component graph.

### Input and selection

The session boundary admits one input-capable client and one bounded read-only
title/CWD observer. It carries lifecycle, frames, semantic input, selection and
copy, terminal clipboard writes, row previews, wheel outcomes, and bounded
signed viewport commits. Selection completion names its authoritative frame
revision so the client can wait for that frame.

- At left press, Orbit chooses terminal input when mouse tracking is active and
  Shift is absent. Otherwise it chooses host selection. The choice lasts through
  release or cancel.
- Terminal phases use Orbit's terminal-aware mouse encoder. Host phases use
  libghostty's tracked selection: single-click drag selects cells, double-click
  selects words, triple-click selects space-delimited spans including
  punctuation, and fourth-or-later clicks select logical lines. Dragging extends
  at that granularity; spans and lines cross soft wraps. The span uses ASCII
  space/tab boundaries and retains native Unicode selection limits. A Begin at
  an already presented revision resolves against current state even if PTY
  output arrived.
- Frames show selected cells. Release freezes bounded plain text for the
  selection clipboard; explicit copy targets the ordinary clipboard. Output,
  resize, reflow, and screen changes do not alter the frozen value. A new
  selection, client loss, or exit clears it.
- Canonical keys reject C0, DEL, and macOS function-key PUA associated text.
  Positional modifier bits require their logical modifier. Mouse press/release
  requires a button; wheel events are momentary; buttonless motion means no
  button is held. Coordinates stay within the mapper's `u16` domain.
- Held-button motion can report outside the viewport. After a fatal protocol
  response, the client cannot send input or receive later frames while the
  bounded error drains; its unread input is released. Once PTY closure is
  observed, semantic requests fail instead of accumulating undeliverable input.

## Attachment-model proof

The proof pins `libghostty-vt` and `libghostty-vt-sys` 0.2.1 from crates.io.
The sys crate builds Ghostty commit
`a887df42c56f6de86c0fe6da9c4eeca37931e083`. The dependency builds on Linux
with Rust 1.96.0 and Zig 0.15.2. Cargo locks ten third-party Rust crates,
including the safe binding, sys crate, and proc-macro stack.

By default, the sys crate clones its pinned Ghostty source when
`GHOSTTY_SOURCE_DIR` is unset and may let Zig resolve packages from the network
unless `GHOSTTY_ZIG_SYSTEM_DIR` supplies a prefetched package set. The proof's
local Ghostty source checkout measured approximately 155 MiB; it is external
dependency material, not owned Rust LOC. Eon's Nix composition prefetches the
Ghostty source and Zig package inputs, so its Cargo build does not fetch them
over the network.

### Why Orbit sends structured frames

The public VT formatter cannot satisfy Orbit's exact attachment contract. The
accepted `orb-bi4.1` experiment feeds a deterministic prefix to an authoritative
terminal, reconstructs a fresh terminal from formatter output, and then gives
both the same distinguishing future tail.

| State | Proof result |
| --- | --- |
| Active text, DECCKM, Kitty keyboard flags, palette, working directory, and state-aware Up-key encoding | Preserved in the checked corpus |
| Custom tab stop | Future tab behavior is preserved after explicit repositioning, but tab restoration moves the reconstructed cursor |
| Character set and currently active hyperlink | Preserved for future text when tab-stop replay is omitted |
| Inactive primary screen while alternate is active | Lost; exiting alternate reveals an empty primary screen |
| Pending wrap | Lost; the next printable byte diverges |
| Parser between CSI, UTF-8, or APC bytes | Lost; the shared future suffix diverges |
| Scrollback | Visible history and the checked resize tail converge, but the immediate scrollback row count differs |
| Closed-cell hyperlink metadata, title, and Kitty image storage | Lost |
| PTY-directed effects | Emitted once by the authoritative terminal; the reconstructed terminal has no PTY callback |
| Current SGR style, scrolling region, and character protection | Not classified by this proof; the failure conclusion does not depend on them |

Git commit `ac0a291ebe7fcebe4d7cace914b89363856c4903` preserves the exact
counterexample corpus. The current suite exercises the accepted authoritative
runtime through real PTY lifecycle and structured-presentation checks. The
repository carries no permanent test target for the rejected reconstruction
path. Orbit uses no fork, binding extension, compatibility wrapper, second
terminal engine, or replay fallback.

The focused Orbit terminal conformance corpus reuses eight authoritative unit
and real-PTY tests under one filter:

```sh
cargo test --locked conformance_
```

It covers parser continuation and terminal replies, complete rich reattachment,
semantic input and resize, canonical rich extraction, viewport routing and
retained-history reflow, and frozen selection copy. The contract-oriented case
inventory and assertion policy live in the [contract index](CONTRACTS.md).
The check is offline, headless, and pinned to the same libghostty path as the
normal suite; it adds no second engine, fixture framework, or replay format.

### Frame publication

Orbit keeps the sole authoritative terminal state and supplies clients with
host-authored structured presentation frames. It does not export a checkpoint,
replicate raw PTY tails, or run another terminal emulator in the client. The
optional `serve --ansi-palette-v1 RGB,...` component input accepts exactly 16
six-digit sRGB entries before the command separator. It replaces only
libghostty palette indices 0 through 15 for that process; omission retains the
built-in defaults, and ordinary OSC overrides and resets remain terminal-owned.
The authoritative extractor uses `RenderState::update` and its public row and cell
iterators to encode a complete versioned frame containing geometry, styled
graphemes, colors and palette, cursor state, active screen, title, working
directory, hyperlinks and scroll position. Frames explicitly advertise hyperlink
support and declare Kitty graphics unsupported.

Publication stays bounded:

- A frame holds at most 4 MiB and 100,000 cells. Nonblocking output keeps its
  initial frame and at most one coalesced latest frame between other messages.
- DEC 2026 synchronized output holds complete frames while PTY parsing, replies,
  and ordered effects continue. Release publishes the latest frame; a one-second
  watchdog clears an abandoned hold. New attachments wait for release.
- One pending revision-bound preview or signed scroll resolves against the
  complete state after release. The owner handles one PTY read per readiness turn.
- A fixed byte bound covers terminal replies and semantic input. When it fills,
  Orbit rejects a whole new input and closes that client.

The real-PTY proof in [`src/presentation.rs`](../src/presentation.rs) covers
ordered revisions, rich alternate and restored primary state, split parser
input, slow clients, input fairness, and final convergence after reattach.

[`crates/protocol`](../crates/protocol) is the sole ORBF v2 format owner. Its
strict decoder validates magic, version, dimensions, tags, reserved flags,
UTF-8, cursor/scroll bounds, truncation, trailing bytes, and the 4 MiB/100,000-cell
limits before making payload-sized allocations. Canonical incremental size
accounting prevents Orbit from retaining rows or cells beyond the same frame
budget while it materializes authoritative state. Its rich owned-frame corpus
round-trips byte-for-byte, every truncated prefix is rejected, and its reducer
accepts only strictly newer complete revisions. The package is private,
`std`-only, platform-neutral, and has no direct or transitive dependency;
libghostty, PTYs, sockets, input, and rendering remain outside it.
