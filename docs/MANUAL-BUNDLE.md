# Manual Linux bundle

Orbit, Venus, Eon and Eon Runtime commit hashes from before 2026-10-14 in this
file refer to the private pre-consolidation archive. They record history, not
proof in this repository; the contract indexes own current proof status.
Upstream project revisions keep their usual meaning.

`nix build .#manual-bundle` produces the development archive
`eon-0.1.0-x86_64-linux.tar.zst` and `SHA256SUMS`. Nix builds the artifact;
the extracted entrypoint requires no installed Nix, store, daemon or evaluator.
This unpublished candidate targets x86_64 Linux native Wayland. Normal alpha
installation remains Nix-based; public-beta acceptance is separate.

## Execution and host boundary

Extract into a directory owned by your user, then invoke its `eon` executable
from any working directory. An external symlink to it also works. Move the
bundle while stopped; keep the selected payload while terminals use it.
Configuration, state and sockets follow the existing [rules](CONFIGURATION.md)
outside the payload.

The static Rust entrypoint finds its own directory and invokes pinned static
Bubblewrap 0.11.2. Bubblewrap projects the complete bundled closure read-only
at `/nix` in a private mount/user namespace. Eon, Orbit, Venus and managed tools
run from unchanged Nix artifacts. PID identities, networking, arguments and
working directory retain their existing meaning. No ELF/config path rewriting
or second component graph is maintained.

The host must permit unprivileged user and mount namespaces and provide `/proc`,
`/dev`, Unix sockets, writable user storage and native Wayland. Prohibited
namespace creation fails with Bubblewrap's diagnostic; the bundle changes no
host policy. This filesystem view provides relocation, not a security sandbox.
Its `/nix` hides any host `/nix` from Eon and children. Use the supported Nix
package when composing with tools from a host Nix store. Other external tools,
resolver, user, timezone and desktop services remain host inputs. Orbit's
existing CLI requires UTF-8 child arguments in both delivery modes.

The payload carries its loader, libc, libraries, fonts, terminfo and Mesa
software driver. Hardware GPU compatibility, a minimum kernel, other distros,
X11/Xwayland and non-systemd qualification are not established by this proof.

## Verification before extraction

An approved release must supply an immutable archive, signed checksums, source
receipt and independently trusted signer record. In a POSIX shell, verify the
signature and files before extraction or execution:

```sh
ssh-keygen -Y verify -f allowed_signers -I eon-candidate -n eon-bundle -s SHA256SUMS.sig < SHA256SUMS
sha256sum -c SHA256SUMS
tar --zstd -xf eon-0.1.0-x86_64-linux.tar.zst
./eon-0.1.0-x86_64-linux/eon versions
./eon-0.1.0-x86_64-linux/eon
```

The retained candidate uses a disposable proof key whose private part was
removed. Its accompanying public key alone supplies no release authority.
Lucca's approved release trust anchor remains a publication prerequisite.

Inside the archive, `component-manifest.json` and `flake.lock` are exact
producer inputs. `provenance.json` schema 1 binds their SHA-256 values, composed
package, target and helper source identity. The composed package path binds the
in-tree Orbit, Venus, runtime and Eon sources. `closure-paths.txt` lists the
complete private closure; absolute `/nix/store` symlinks resolve within that
view. These receipts consume the canonical graph rather than selecting
components independently.

## Retained development proof

This proof predates the single repository and awaits re-proof here. The
2026-10-10 candidate archive has SHA-256
`ad351bb2c914f7319ba0bd1006a8ab77041a05375687a6aac350cd7a6294f971`.
Its Eon artifact is `/nix/store/ayb2ilp1rq7c95q4r1b3qn8qzdx5pw02-eon-0.1.0`,
with Orbit `6bc269c40b18f08b95778939518f77556ba91c67` and Venus
`58f2eaa8057627bdbf73262fdb10a1dc8b681ffc` from the unchanged manifest/lock.
The archive measures 406,102,478 bytes; its 409-path closure measures
1,830,687,400 NAR bytes. Extraction allocated 1,877,639,168 bytes on the proof
filesystem and took 14.88 seconds. These are observations, not bounds; build
caches and extra live generations are excluded.

A fresh isolated user environment on Pop!_OS 24.04/Linux 7.1.5 hides host `/nix`,
the checkout and the user's home. Private Sway 1.12 and packaged Mesa 26.1.2
lavapipe exercise a moved read-only root with spaces/apostrophes and an external
entrypoint symlink. Exact Nix/direct diagnostics, twelve managed tools, Unicode
arguments/cwd, native Nushell input, same-process detach/reopen and scoped Stop
pass. A missing private closure fails despite a matching host store package.
Altered signed inputs and incorrect archive digests fail verification.

The owning bead retains exact source hashes, commands, receipts and the
assert-based native probe under
`~/.local/state/eon/proofs/eon-manual-bundle-2026-10-10-m7mqh_vg/`.
This scripted native interaction shares the host kernel; it is not independent
machine, hardware-GPU or human screen-reader acceptance. VEN-C1 records the
narrow Orca limitation.

## Installer and publication handoff

The [static installer](INSTALLER.md) consumes this verified artifact, preserves
its layout and selects its root entrypoint. Its accepted development proof
under `eon-prove-thin-linux-bundle-installer-8r7n` covers placement, upgrade,
rollback and owned removal while retaining every version at its original path.
That bead records its exact second archive and unchanged component graph.

Existing component notices remain in the closure; Eon's license and Bubblewrap's
upstream source/license are also carried at the archive root. Publication still
requires the exact closure's complete notice and source obligations, including
statically linked helper dependencies, to be resolved. An inventory and proof
key do not establish public redistribution clearance.

Draft manual-download copy for a separately approved release: Download the
immutable Linux archive, signed checksums and source receipt from the approved
release. Verify them with the trusted release signer before extracting. Run
the extracted `eon` on native Wayland; Nix is unnecessary. Linux user namespaces
must be permitted. Keep the selected payload while its terminals are running.
No website, release asset or installer is published by this development slice.
