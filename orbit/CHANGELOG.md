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
- Orbit builds Ghostty `8c425a7bd57de848aa83cb9b003b1ecd6037c56a` with Zig
  0.17, through the `Yazelix/libghostty-rs` fork that `docs/CRATES.md`
  records (`eon-a9tr.2`). The ORBS v13 and ORBF v2 wire is unchanged, so the
  change is compatible for Venus, Eon Runtime and Eon. Terminal behavior
  follows upstream: cursor positioning in origin mode and DECRQM reports
  take Ghostty's fixes, and Orbit answers DECXCPR queries.
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

## Before this repository

Orbit was developed as Eon Sessions until 2026-10-14; that history is kept in a
private archive. It arrives here with ORBF v2 and exact-version ORBS v13 on
libghostty-vt 0.2.2 built for the portable baseline CPU: one real PTY with
authoritative terminal state, bounded attachment and metadata observation,
retained history and authoritative scrolling, the native selection ladder and
copy, terminal-authored clipboard writes, the Eon terminal identity, bounded
Session shutdown, same-boot management ownership, and the Apple Silicon
macOS platform seam.
