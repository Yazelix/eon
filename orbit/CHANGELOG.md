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

## Before this repository

Orbit was developed as Eon Sessions until 2026-10-14; that history is kept in a
private archive. It arrives here with ORBF v2 and exact-version ORBS v13 on
libghostty-vt 0.2.2 built for the portable baseline CPU: one real PTY with
authoritative terminal state, bounded attachment and metadata observation,
retained history and authoritative scrolling, the native selection ladder and
copy, terminal-authored clipboard writes, the Eon terminal identity, bounded
Session shutdown, same-boot management ownership, and the Apple Silicon
macOS platform seam.
