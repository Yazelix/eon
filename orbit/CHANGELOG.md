# Changelog

Orbit records accepted user- and consumer-visible changes here. Contracts
remain canonical in `docs/CONTRACTS.md`, dependency decisions in
`docs/CRATES.md`, and implementation evidence in Beads.

## Unreleased

- Orbit's implementation is a Rust host plus a Bend session-policy worker,
  imported from bend-orbit `e03926274203d78b76f49337e9f6670d2dd6e97d`. The
  ORBS v13 and ORBF v2 wire, the `yazelix-orbit serve` command line, the
  sockets and the management records are unchanged, so the change is
  compatible for Venus, Eon Runtime and Eon. The package ships the worker as
  `libexec/yazelix-orbit/yazelix-orbit-worker` beside `bin/yazelix-orbit`, and
  Bend's Apache-2.0 license under `share/licenses/orbit/bend`. The old Orbit's
  `client` diagnostic subcommand is removed. The old single-process server is
  kept at the tag `orbit-rust-final`. Contract proof moves to the import:
  ORB-C10 becomes Proved; ORB-C2, ORB-C4 to ORB-C9, ORB-C11 and ORB-C13 become
  Partially proved, since some behavior the old unit tests checked has no
  check yet (`eon-awmq.7`); ORB-C14's native macOS proof covers only the old
  server, so macOS is unproved for this Orbit until it is re-proved.
- ORB-C2 and ORB-C5 return to Proved: a real-PTY check covers a whole-palette
  OSC 104 reset and the colors above 15, and a host unit test covers a Kitty
  release of an unidentified key (`eon-awmq.7.1`).
- ORB-C11 and ORB-C13 return to Proved: real-PTY checks cover clipboard writes
  that fill a client's queue, and untrusted records and runtime directories
  refused before publishing (`eon-awmq.7.4`). A wrong-mode record was already
  refused at publish time, not only at lease acquisition as the import noted.
- ORB-C4, ORB-C6 and ORB-C7 return to Proved: a real-PTY check covers
  attachment during a held update and exact reattachment to a rich alternate
  screen, and host unit tests cover direct rows against frame rows through
  reflow and the scroll-outcome room at the worker's output bound
  (`eon-awmq.7.2`).
- Orbit builds Ghostty `521cf2c5d30bd48028ca61546c64e7913764073d` with Zig
  0.17, through the upstream `Uzaaft/libghostty-rs` commit that
  `docs/CRATES.md` records (`eon-a9tr.2`, `eon-9rtt.4`). The ORBS v13 and
  ORBF v2 wire is unchanged, so the change is compatible for Venus, Eon
  Runtime and Eon. Terminal behavior follows upstream: cursor positioning in
  origin mode and DECRQM reports take Ghostty's fixes, and Orbit answers
  DECXCPR queries.
- ORB-C15 enters the index as Planned. Orbit answers the OSC 7501 program
  status probe, so programs such as Claude Code 2.1.295 or later start
  reporting, and it keeps each terminal's reports as records whose working
  and blocked ones a prompt mark or a return to the child's foreground group
  drops (`eon-agent-sidebar-yx8.4`). Nothing reads the records yet;
  management serves them from `eon-agent-sidebar-yx8.5`. The worker boundary
  gains fact fields and a decision, which Orbit's host and worker ship
  together, so the ORBS v13 and ORBF v2 wire and every consumer are
  unaffected.
- Management moves to generation 2, a breaking change with no compatibility
  window (`eon-agent-sidebar-yx8.5`). Under the lease, the `ProgramStatus`
  request (client tag 4) reads a terminal's OSC 7501 records: the reply
  (server tag 69), at most 4 KiB, carries the record count, the latest update
  sequence, the count omitted for size, and the records with their neutral
  facts, blocked, error and done first, each group most recent change first.
  Orbit answers while the child runs; its policy refuses with Busy once a
  stop has begun, which no request reaches today. `--management-v1` becomes
  `--management-v2`. The live record and tombstone are framed as generation
  2 too, and a generation-1 peer or record is refused by the exact-generation
  rule, so Eon Runtime and Eon move to generation 2 in the same change; Venus
  does not use management. A new Eon cannot stop or recover a run started by
  a generation-1 Orbit. ORB-C15 is Proved.
- ORB-C15's `ProgramStatus` reply moves from server tag 69 to 70, a breaking
  change for its one consumer, Eon Runtime, with no compatibility window
  (`eon-agent-sidebar-yx8.12`). Each record carries its creation sequence:
  the update sequence of the report that created it. After the omitted count
  the reply carries the creation sequence of each record omitted for size,
  inside the same 4 KiB, so the Runtime can tell an omitted record from a
  removed one. Management stays generation 2: a tag-69 reply from an older
  generation-2 Orbit is refused as an unknown tag, so that terminal's
  attention stays stale; if the Runtime never had a good reply, it has none,
  and an Agent popup shows unavailable. Stop and recovery are unaffected.
  Venus and Eon do not read the reply.
- ORB-C15's `ProgramStatus` reply carries the terminal title, the window
  title that ORBS metadata publishes, after the update sequence: without
  control or invisible format characters, cut to 64 bytes at a UTF-8
  boundary, and empty when the terminal has none or its title is not valid
  UTF-8. It counts toward the 4 KiB, so at their bounds 7 records fit while
  the terminal holds 14 or fewer. The reply moves from server tag 70 to 71,
  a breaking change for its one consumer, Eon Runtime, with no compatibility
  window (`eon-agent-sidebar-yx8.11`). Management stays generation 2; a
  tag-70 reply from an older Orbit is refused as an unknown tag, as yx8.12's
  tag-69 reply was. Stop and recovery are unaffected.
- ORB-C3 adds a failure: invalid UTF-8 terminal metadata, such as a title
  libghostty cuts inside a character or a non-UTF-8 OSC 7 directory, reaches
  the client and observer as empty and never ends the terminal. Before, either
  ended the terminal while a client or observer was attached
  (`eon-title-utf8-ends-terminal-atq2`).
- ORB-C6 adds a failure: a hyperlink URI that is not valid UTF-8 reaches the
  client as no link, the cell keeps its text in frames and previews, and the
  terminal keeps running. Before, such a link ended the terminal while a
  client was attached (`eon-osc8-invalid-utf8-ymts`).
- ORB-C6 adds a failure: when a frame's hyperlinks would take it past the
  frame budget, every cell keeps its text, links are kept in reading order
  while they fit, and the rest reach the client as no link. Before, such a
  frame ended the terminal while a client was attached
  (`eon-frame-budget-links-m62b`).
- ORB-C6 widens that failure to cell text: when a frame would pass the frame
  budget, every cell keeps its first codepoint; full cell text and then
  hyperlinks are kept in reading order while they fit, and the rest reach the
  client as their first codepoint and as no link. Before, a screen of cells
  heavy with combining marks ended the terminal while a client was attached
  (`eon-frame-budget-text-kzze`).
- ORB-C12 changes a failure: a management Stop whose child is not reaped
  within two seconds of SIGKILL keeps the run and its live record until the
  child is reaped, then succeeds with the child's outcome. Before, the Bend
  worker died, and the run exited with no tombstone and removed its record. A
  termination signal still requires the reap within two seconds. Compatible
  for Venus, Eon Runtime and Eon: Eon Runtime's Stop still waits five seconds
  for the result, so a child reaped after that still fails its Stop there,
  and the tombstone comes later (`eon-orbit-stop-not-reaped-7vb4`).
- ORB-C16 enters the index as Planned: static Kitty images and Unicode
  placeholder placements will reach the client as decoded RGB or RGBA resources
  and per-frame placement entries on ORBF v3 in ORBS v14, a breaking revision
  for Venus, Eon Runtime and Eon. ORB-C6 becomes Partially proved: a frame that
  carries ORB-C16's state states Kitty graphics supported, and one that does
  not still declares them unsupported. Behavior and wire are unchanged until
  `orb-j2i.4` to `orb-j2i.7` land (`orb-j2i.2`).
- ORBF v3 in ORBS v14, a breaking, exact-version revision for Venus, Eon
  Runtime and Eon with no adapter. A frame that states the Kitty graphics
  capability ends with ORB-C16's graphics block: the cell size its geometry
  uses, an omitted-placements flag and up to 4,096 placement entries
  referencing at most 256 resources, outside the 4 MiB frame budget for cells
  and links. Rows drop the `kitty_virtual_placeholder` flag. ORBS v14 adds
  resource announcements, chunks of 262,144 bytes and retirements. The largest
  frame grows to 4,448,267 bytes and the largest payload to 8,642,584, so the
  worker's per-client output bound grows to 13,094,971 bytes and ORB-C7's room
  for a scroll outcome behind a started frame holds. Orbit still states Kitty
  graphics unsupported and sends no resources (`orb-j2i.5`).

## Before this repository

Orbit was developed as Eon Sessions until 2026-10-14; that history is kept in a
private archive. It arrives here with ORBF v2 and exact-version ORBS v13 on
libghostty-vt 0.2.2 built for the portable baseline CPU: one real PTY with
authoritative terminal state, bounded attachment and metadata observation,
retained history and authoritative scrolling, the native selection ladder and
copy, terminal-authored clipboard writes, the Eon terminal identity, bounded
Session shutdown, same-boot management ownership, and the Apple Silicon
macOS platform seam.
