# Linux bundle installer

`nix build .#installer` builds the static `eon-install` development bootstrap.
The installer consumes the [manual Linux bundle](MANUAL-BUNDLE.md); it neither
composes packages nor runs Nix. No public installer or release signer is published.
Normal installation remains Nix-based.

The target is x86_64 Linux on native Wayland, with the bundle's documented
[host prerequisites](MANUAL-BUNDLE.md#execution-and-host-boundary).
Installation also requires host `curl` with HTTPS and OpenSSH 8.9 or newer `ssh-keygen`
supporting `-Y match-principals` and `-Y verify`.
Missing or older `ssh-keygen` gets its own prerequisite error before downloads.
Install and rollback use host GNU coreutils `timeout` to bound the packaged
check and kill its entire process group if it hangs. Rust, Nix, tar and zstd
commands are unnecessary on the target machine.

## Inspect and install

Inspect the source in `installer/src/`, then build the bootstrap and read its help:

```sh
nix run .#installer -- --help
```

An approved release must supply an exact immutable archive URL, `SHA256SUMS`
and its OpenSSH `SHA256SUMS.sig` in the same release directory. Obtain the
approved `allowed_signers` record independently of those downloaded files.
It must authorize identity `eon-release` in signature namespace `eon-bundle`.
An accompanying downloaded public key alone is not a trust anchor.

Create the signer record only after authenticating the release public key through
an independent trusted channel. For example, with that authenticated key saved
as `TRUSTED_RELEASE_KEY.pub`:

```sh
mkdir -p "$HOME/.config/eon"
{ printf 'eon-release namespaces="eon-bundle" '; cat TRUSTED_RELEASE_KEY.pub; } \
  > "$HOME/.config/eon/allowed_signers"
chmod 600 "$HOME/.config/eon/allowed_signers"
```

Pass its absolute path as `--signers "$HOME/.config/eon/allowed_signers"`.
The installer requires an absolute signer path and refuses a missing, unreadable,
empty or invalid signer file before downloading anything or creating a prefix.
OpenSSH parses the record and checks for `eon-release`; signature verification
still authorizes the downloaded checksums. This setup never downloads or trusts a key
alongside the archive; Lucca holds the release signing key.

For a future approved release, replace `HTTPS_ARCHIVE` and `ALLOWED_SIGNERS`
below with that exact HTTPS URL and your trusted local signer record:

```sh
eon-install install HTTPS_ARCHIVE --signers ALLOWED_SIGNERS --dry-run
eon-install install HTTPS_ARCHIVE --signers ALLOWED_SIGNERS
```

Dry-run downloads and verifies the signed checksum and archive bytes without
changing the installation prefix. Actual installation additionally validates
extraction and runs the packaged `eon versions` before selecting the version.
TLS verification remains enabled for downloads and redirects. The installer
does not load curlrc or accept an HTTP fallback.

The default prefix is `$XDG_DATA_HOME/eon/install`, or
`$HOME/.local/share/eon/install` when the data directory is unset, empty or
relative, following the
[XDG Base Directory specification](https://specifications.freedesktop.org/basedir/latest/).
An explicit `--prefix` accepts an absolute path without `..`, including spaces/apostrophes.
The installer creates a private user-owned prefix. It refuses an existing
directory without its ownership marker and preserves conflicting commands.

Run `PREFIX/bin/eon`, or add `PREFIX/bin` to PATH yourself. The installer writes
no shell startup file or desktop entry and does not replace a Nix profile.

## Installer inside a payload

Direct payloads carry the same static installer bytes as the release's separate
bootstrap. Eon's bundle launcher sets `EON_INSTALLER` to that payload's exact
`nix/store/…-eon-install-…/bin/eon-install`, recalculated from the launcher's
own location. It replaces any inherited value; no PATH search, Nix invocation
or checkout lookup locates the installer after relocation. The bundled
installer still uses the host prerequisites above.

The payload also carries `share/eon-source-receipt.json` next to the packaged
Eon's `bin` directory. Its EON-C41 schema-1 fields bind repository, source
revision (null for dirty/unknown source), build and platform; `attribute` is
null for this direct channel. `provenance.json` records the installer producer
output alongside the existing composition inputs. Installation adds the
versioned `.eon-source` record with `eon-install-v1`, the archive SHA-256 and
exact URL. Neither receipt establishes trust or prefix ownership on its own.

Automatic direct updates remain planned in `eon-update-button-f10d1.6` and
`.7`. Their `updates.allowed_signers` setting takes the same absolute signer
file path shown above; Runtime will resolve it and the product helper will
pass it to this installer as `--signers`, with the exact URL and validated
prefix. That setting and `eon update --prefix` are not supported yet. An unset
signer must refuse automatic installation with setup instructions. The helper
must validate the owned prefix/selector/receipt chain before using its bundled
installer; the environment locator alone grants no ownership, even when inherited
from a bundled supervisor by a Nix executable. Payloads without
the bundled helper remain manual, with no PATH fallback or bootstrap download.

## Release assets

A release carries `eon-VERSION-x86_64-linux.tar.zst`, the static installer as
`eon-install-x86_64-linux`, `SHA256SUMS` over both and the maintainer's
`SHA256SUMS.sig`. `nix build .#release` builds the first three from one
revision. Verify a downloaded installer like the archive: check the signature
over `SHA256SUMS`, then run `sha256sum -c --ignore-missing SHA256SUMS`
([manual verification](MANUAL-BUNDLE.md#verification-before-extraction)).

The [release workflow](../.github/workflows/release.yml) builds them on GitHub
from the commit it is dispatched on, creates a GitHub build-provenance
attestation for each, and attaches them to a draft release. The tag must be
`vVERSION`, or `vVERSION-SUFFIX` for a pre-release, where VERSION is the Eon
package version; an existing tag must name the dispatched commit. A release
tags its exact proven `edge` commit ([Git Workflow](AGENT-RULES.md#git-workflow)),
so push the tag first and dispatch the workflow on it. The draft is unsigned.
Before signing, the maintainer who holds the release key compares its
`SHA256SUMS` with one from a local `nix build .#release` of the same commit. If
they differ, do not sign: stop and file a bead, since bit-for-bit parity of the
hosted and local builds is unproven until a rehearsal shows it. Otherwise sign
offline and publish:

```sh
git push origin vVERSION
gh workflow run release.yml -R Yazelix/eon --ref vVERSION -f tag=vVERSION
# Once that run has succeeded; each step runs only if the previous one passed.
gh release download vVERSION -R Yazelix/eon -p SHA256SUMS &&
  git checkout vVERSION && nix build .#release &&
  diff SHA256SUMS result/SHA256SUMS &&
  ssh-keygen -Y sign -f RELEASE_KEY -n eon-bundle SHA256SUMS &&
  gh release upload vVERSION -R Yazelix/eon SHA256SUMS.sig &&
  gh release edit vVERSION -R Yazelix/eon --draft=false
```

Turn on immutable releases for the repository before publishing, so
publication locks the tag and assets. The signature stays the installer's
trust anchor. `gh attestation verify FILE -R Yazelix/eon` additionally shows
that this repository's workflow built a downloaded file; it needs the GitHub
CLI and network access, and the installer does not run it.

## Upgrade, rollback and removal

Installing another exact archive performs an upgrade. Payloads live under
`PREFIX/versions/SHA256`; one atomic `PREFIX/current` symlink selects the
default. The install output names the selected digest. Keep an earlier digest
for rollback; installed version directories also list the retained digests.

```sh
eon-install rollback SHA256
eon-install remove
```

Supply the same `--prefix` on each command when using a custom prefix. Both
commands support `--dry-run`. Rollback uses an already installed version and
does not download it again. Removal deletes only installer-owned command and
current links; a replaced user command is preserved.

Every payload remains at its original path through upgrade, rollback and
removal. Live supervisors, Orbit terminals and their child processes are never
stopped by the installer. Retained work can use its exact version's `eon`
entrypoint; a new default does not transfer an old generation into a new one.
The existing [generation rules](USAGE.md) still apply. Reboot or Orbit restart
persistence is not promised.

Removal deliberately retains disk usage: approximately one extracted bundle
per version. Automatic pruning is outside this installer. Stop work explicitly
and review retained contents before manually reclaiming the prefix.

## Failure and recovery

Signature/hash, download, archive-boundary and preselection checks fail without
changing the prior `current` selector. Extraction uses a new private staging
directory, validates entry paths and actual link targets, and rejects absolute
symlinks, duplicate entries and special files. The tar extractor prevents writes
outside that staging directory.
Selection requires the payload's `eon` to be a symlink to an executable file.
The trusted publisher still controls executable contents; this is not a sandbox for
running hostile code or protection against another hostile process under your UID.

One exclusive prefix lock serializes mutations. An interrupted install may
leave `.eon-stage-*` or `.eon-select-*` residue, or a complete unselected version.
Rerun the verified install: it does not reuse abandoned staging and only selects
a complete checked version. Inspect abandoned staging for cleanup after the
installer has exited. No power-loss durability claim follows from atomic selection.
An interrupted installer may leave its in-flight preflight running until its
30-second deadline; wait for that check to end before cleaning its staging.
If interruption leaves a newly created prefix without its ownership marker,
inspect that incomplete directory and remove it manually before retrying;
the installer refuses to adopt an unmarked directory.

## Retained development proof

This proof used archives of the bundle's earlier Bubblewrap mechanism, which the
installer no longer accepts; it awaits re-proof with the relinked payload.
`eon-prove-thin-linux-bundle-installer-8r7n` records seven focused Rust checks,
the static Nix build, signed TLS failures/interruption and native installation,
upgrade, rollback, retained reattachment and removal, and binds exact archives
and the unchanged component graph. Evidence lives under
`~/.local/state/eon/proofs/eon-installer-review-2026-10-11-08l4lm2k/`.
The isolated same-host Wayland rerun preserves ordinary work and bundled Helix.
The earlier `eon-installer-2026-10-10-2uahgczq/` proof retains exact idle
authenticated Codex process evidence for the unchanged runtime and archives;
the rerun's Codex bootstrap hit an account-routing timeout. No model turn,
semantic resume, independent-machine, hardware-GPU, human-reader or Nix-free
install qualification follows.

The parser correction for unused non-link header fields is checked with the
static bootstrap and signed installation under
`~/.local/state/eon/proofs/eon-installer-review2-2026-10-11-2u3zxw74/`.
It retains the earlier delivery/lifecycle proof for unchanged runtime behavior.

## Website copy for an approved release

Draft for `eonterm.com`, pending publication and approval of the release trust anchor:

Download the reviewed Linux bootstrap `eon-install-x86_64-linux` and inspect
its source. Use the independently approved Eon signer record to install an
exact release from `https://github.com/Yazelix/eon/releases/download/APPROVED_TAG/`.
That immutable release must provide `eon-VERSION-x86_64-linux.tar.zst`,
`eon-install-x86_64-linux`, `SHA256SUMS` and `SHA256SUMS.sig`. Run the install
command with `--dry-run` first. Run the installed `PREFIX/bin/eon` on native
Wayland; Nix is unnecessary.
Upgrades and removal keep old payloads and live terminals. Manual download,
verification and extraction remain available.

`APPROVED_TAG` and `VERSION` are placeholders, not published releases.
Public release signing, redistribution obligations and Nix-free install
qualification retain their separate gates. The development proof uses a
disposable signer.
