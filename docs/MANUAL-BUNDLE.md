# Manual Linux bundle

Orbit, Venus, Eon and Eon Runtime commit hashes from before 2026-10-14 in this
file refer to the private pre-consolidation archive. They record history, not
proof in this repository; the contract indexes own current proof status.
Upstream project revisions keep their usual meaning.

`nix build .#manual-bundle` produces the development archive
`eon-VERSION-x86_64-linux.tar.zst`, where VERSION is the Eon package version,
and `SHA256SUMS`. Nix builds the artifact;
the extracted entrypoint requires no installed Nix, store, daemon or evaluator.
This unpublished candidate targets x86_64 Linux native Wayland. Normal
installation remains Nix-based; Nix-free install qualification is separate.

## Execution and host boundary

Extract into a directory owned by your user, then invoke its `eon` executable
from any working directory. An external symlink to it also works. Move the
bundle while stopped; keep the selected payload while terminals use it.
Configuration, state and sockets follow the existing [rules](CONFIGURATION.md)
outside the payload.

The payload is Eon's Nix closure relinked at build time to run on the host's
glibc. Every dynamically linked program's interpreter is the x86-64 psABI
loader `/lib64/ld-linux-x86-64.so.2`, library search paths are relative to each
file, and store symlinks are relative. The archive's `eon` links to a static
launcher inside the payload. Copies of that launcher replace the Nix wrapper
scripts and set their environment relative to wherever the payload sits, minus
entries in the excluded store paths and plus data roots only the payload needs.
Programs that need data from a compiled-in store path get it from that
environment or from a link the payload adds where they look next. Eon, Orbit,
Venus and managed tools run from their payload paths and need no host `/nix`.
Some still probe their compiled-in store paths first, so on a host whose `/nix`
holds those exact paths, Helix, for example, reads its runtime from there. PID
identities, networking, arguments and working directory retain their existing
meaning. No second component graph is maintained.

The host supplies glibc 2.39 or newer, with its loader at the psABI path, its
NSS modules and locales; `libgcc_s`; `libwayland-client` for Venus; and a
Vulkan driver, which Venus loads through its bundled Vulkan loader. It must also
provide `/proc`, `/dev`, Unix sockets, writable user storage and native Wayland.
The build fails if a shipped program needs a newer glibc symbol version or a
library found neither in the payload nor among these host libraries. Nothing
checks the floor at install or launch: on an older glibc, `eon versions` and the
installer's check can pass while a program that needs a newer symbol fails when
it starts. The bundle needs no root, user namespace or security-policy change,
and changes no host policy. Orbit terminals are ordinary host processes,
without a namespace or the no-new-privileges flag. Other external tools, the
resolver, users, timezone, certificate store and desktop services remain host
inputs. CodexBar's `SSL_CERT_FILE` and `CURL_CA_BUNDLE` each default to the
bundled CA certificates, as in the Nix channel.
Orbit's existing CLI requires UTF-8 child arguments in both delivery modes.

The payload carries every other library its components link, plus their fonts,
terminfo and data. It leaves out glibc, Mesa and Wayland's libraries, and what
only they reach, such as LLVM. Hardware GPU compatibility, a minimum kernel,
specific distros, X11/Xwayland and non-systemd qualification are not
established.

## Verification before extraction

An approved release must supply an immutable archive, signed checksums, source
receipt and independently trusted signer record. In a POSIX shell, verify the
signature and files before extraction or execution:

```sh
ssh-keygen -Y verify -f allowed_signers -I eon-candidate -n eon-bundle -s SHA256SUMS.sig < SHA256SUMS
sha256sum -c SHA256SUMS
tar --zstd -xf eon-VERSION-x86_64-linux.tar.zst
./eon-VERSION-x86_64-linux/eon versions
./eon-VERSION-x86_64-linux/eon
```

The retained candidate uses a disposable proof key whose private part was
removed. Its accompanying public key alone supplies no release authority.
Lucca's approved release trust anchor remains a publication prerequisite.

Inside the archive, `component-manifest.json` and `flake.lock` are exact
producer inputs. `provenance.json` schema 2 binds their SHA-256 values, the
composed package and the target. Its `relink` entry records the glibc floor,
the excluded glibc, Mesa and Wayland store paths, and each substituted path with
the rebuild whose files ship under its name. ncurses is the one substitute: the
same source rebuilt with a header that replaces glibc 2.42's `cfgetospeed`, so
it runs on older glibc. The composed package path binds the in-tree Orbit,
Venus, runtime and Eon sources. `closure-paths.txt` lists the store paths the
payload ships; the rest of the package's closure is what the exclusions
removed. Relinking patches the shipped files, so these paths name each file's
origin, not its bytes. The payload holds no absolute symlink. These receipts
consume the canonical graph rather than selecting components independently.

## Retained development proof

This proof covers the earlier Bubblewrap mechanism, which ran the unrelinked
closure at a private `/nix` inside a user namespace. `eon-zti4` replaced it:
stock Ubuntu 26.04 forbids that namespace, and inside it no terminal could use
`sudo`. The proof predates the single repository and awaits re-proof of the
relinked payload. The 2026-10-10 candidate archive has SHA-256
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
under `eon-prove-thin-linux-bundle-installer-8r7n` covers placement,
upgrade, rollback and owned removal while retaining every version at its
original path. That bead records its exact second archive and unchanged
component graph. Its archives used the earlier Bubblewrap mechanism, which the
installer no longer accepts.

Existing component notices remain in the payload, and Eon's license is carried
at the archive root. The payload's programs and libraries are modified builds:
relinking changes their interpreter and library search paths. Publication still
requires the exact payload's complete notice and source obligations, including
for these modified files and the launcher's statically linked dependencies, to
be resolved. An inventory and proof key do not establish public redistribution
clearance.

Draft manual-download copy for a separately approved release: Download the
immutable Linux archive, signed checksums and source receipt from the approved
release. Verify them with the trusted release signer before extracting. Run
the extracted `eon` on native Wayland with glibc 2.39 or newer and a Vulkan
driver; Nix, root and user namespaces are unnecessary. Keep the selected payload
while its terminals are running.
No website, release asset or installer is published by this development slice.
