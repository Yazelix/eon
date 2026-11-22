# 8. Updates: installation and live work have different owners

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

An **installation** selects the executable for fresh launches. A running window
already has a supervisor and terminal processes. EON-C41 separates package
maintenance from live adoption: installation selection does not replace the
controller of existing work. Its Running work clause says Check, refusal,
failure and successful selection do not stop/re-exec a supervisor, replace
Venus or move terminals ([docs/CONTRACTS.md:7699](../CONTRACTS.md)).

Package maintenance has implemented slices for Nix profiles and installer-owned
Linux prefixes. Nix is the package build/composition system; a profile selects
installed packages, and a prefix is an installation directory. The contract's
overall status remains Planned, with those
slices listed. Full in-place Update remains planned in the current usage
documentation. Do not describe a completed package update as proof of live
controller adoption ([docs/CONTRACTS.md:7520](../CONTRACTS.md),
[docs/USAGE.md:1162](../USAGE.md)).

A subtle case arises inside an older window. Its internal command directory
can still name the window's build, while the person's external PATH selects a
newer installation. The **handoff** function sends untargeted package
maintenance to an external launcher, with a guard against handing off twice.
The process it replaces is this maintenance CLI, not the workspace supervisor
([update.rs:209](../../eon/crates/eon/src/update.rs),
[docs/CONTRACTS.md:7561](../CONTRACTS.md)).

Workspace commands do not hand off: a pane in an Eon window has that window's
own `eon` first on PATH, so after a profile refresh `eon window reload` run from
a pane runs the old binary; reloading presentation must call the profile's
`eon`: `~/.nix-profile/bin/eon window reload default` (the plain Nix profile
path is `~/.nix-profile` on macOS too)
([sessions.rs:2205](../../runtime/crates/eon-runtime/src/sessions.rs),
[managed_environment.rs:2993](../../runtime/crates/eon-runtime/src/managed_environment.rs),
[managed_environment.rs:3577](../../runtime/crates/eon-runtime/src/managed_environment.rs),
[docs/USAGE.md:279](../USAGE.md),
[docs/DISTRIBUTION.md:159](../DISTRIBUTION.md)).

## Run it

From the repository root, in Nushell. Read the mechanism and its live-work
boundary; no installation changes in this exercise:

```nu
^nl -ba eon/crates/eon/src/update.rs | ^sed -n '229,260p'
^nl -ba docs/CONTRACTS.md | ^sed -n '7699,7707p'
```

## What to look at

Find the configured-target early return, the `HANDOFF_MARKER` check and
`command.exec()`. Then read “installed
for fresh launches” in the contract. The caller's `exec` is a process
replacement, but it is not replacement of the window's Runtime
([update.rs:229](../../eon/crates/eon/src/update.rs),
[docs/CONTRACTS.md:7702](../CONTRACTS.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Earlier guards restrict handoff to an untargeted internal launcher
([update.rs:217](../../eon/crates/eon/src/update.rs)). Walk the remaining
[update.rs:229-260](../../eon/crates/eon/src/update.rs):

- Lines 229–233 retain normal processing when update configuration already
  supplies a target. A configuration error propagates instead of guessing.
- Lines 234–240 prepare guidance and refuse a second handoff whenever the
  marker exists. This bounds a possible launcher loop.
- Lines 241–248 select an external launcher from PATH or refuse with a
  diagnostic. The selector helper defines the detailed path checks at
  [update.rs:261](../../eon/crates/eon/src/update.rs).
- Lines 249–253 forward the arguments, set the marker and remove the named
  build-locator environment variables. The receiving package should supply
  its own component paths.
- Lines 254–260 call `exec`. On success, the CLI process continues as the
  chosen launcher. On failure, return a selection error naming it.

This function explains maintenance routing. It does not alone establish every
installer cancellation, trust or rollback property; EON-C41 names those other
obligations and its proof records ([docs/CONTRACTS.md:7520](../CONTRACTS.md)).

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Can you update Eon without killing or silently replacing the work?”

EON-C41 requires preserving running work during installation selection, but
its overall status remains Planned
([docs/CONTRACTS.md:7699](../CONTRACTS.md), [docs/CONTRACTS.md:7522](../CONTRACTS.md)).

Recorded live-work proof covers Nix-profile installs; prefix and native macOS
Install/HTTPS proof remain open
([docs/CONTRACTS.md:7854](../CONTRACTS.md), [docs/CONTRACTS.md:7861](../CONTRACTS.md)).

Full in-place Update remains planned. Presentation reload replaces only Venus,
not the whole Runtime/terminal set
([docs/USAGE.md:1162](../USAGE.md), [docs/CONTRACTS.md:99](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

An old window reports a successful package update. Explain why its running
version badge (the version label in the bottom hints bar) can remain old,
which process the handoff replaced, and what you still cannot claim about
live adoption ([docs/USAGE.md:620](../USAGE.md)).
<!-- owner-only:end -->

## Going deeper

- [Installation update ownership, EON-C41, docs/CONTRACTS.md:7520](../CONTRACTS.md).
- [User-facing update behavior, docs/USAGE.md:223](../USAGE.md).
- Next: [security boundaries](09-security-boundaries.md).
