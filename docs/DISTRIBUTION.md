# Distribution and Composition

## Goals

Astra alpha and early dogfood use Nix as the sole installation and composition
channel. This phase spends project time on Orbit, Venus, and Astra product
contracts instead of portable archives and installer maintenance.

Direct installation remains the long-term adoption path. It begins after Nix
dogfood proves a useful product and the user approves distribution graduation.
Every channel consumes the same component graph and preserves runtime semantics.

## Phase policy

### Alpha and early dogfood

One Nix flake path builds, composes, and installs the accepted component set.
Home Manager remains separate work and must consume the same package and product
contract if the user activates it.

The Nix-only phase follows these boundaries:

- Astra code accepts component paths and versions through explicit inputs.
- Nix store paths remain opaque launch values rather than stable identity.
- Product state and user configuration contain stable component identities.
- Normal runtime operation invokes no Nix command or evaluator.
- Direct installers, portable archives, signing, and package matrices stay out
  of the alpha implementation surface.

### Distribution graduation

Direct-distribution work requires a separate user decision after:

- the Nix composition survives sustained fresh-session dogfood;
- launch, configuration, diagnostics, upgrades, and failure behavior stabilize;
- exact Orbit and Venus revisions have no open P0 or P1 integration defect;
- the component manifest describes the accepted product without relying on
  derivation identity;
- the team can maintain release artifacts and their verification checks.

The graduation decision selects the first direct target and release tool. It
does not activate every package channel.

## Canonical release input

A versioned component manifest records each component's project, exact revision,
package or artifact input, platform, and compatibility contract. Nix consumes
this manifest during alpha. Release tooling extends it with portable artifacts,
checksums, and provenance after distribution graduation.

The manifest should remain small enough for humans to review. Child repositories
build and test their owned artifacts. Astra verifies compatibility and assembles
the product without rebuilding child behavior.

## Channel sequence

### Nix alpha

Nix supplies the first and sole alpha channel for reproducible composition,
source builds, development, and dogfood. The flake consumes the canonical
component manifest and owns no product behavior.

The initial slice should support one installation path. Home Manager can follow
through a separate decision without changing the package or runtime contract.

### Linux direct bundle

After distribution graduation, the first direct channel should publish a
compressed relocatable bundle, checksum, signature or attestation, and a small
installer. The installer should support a versioned user prefix plus an atomic
`current` link, a dry run, and removal. It must avoid shell-profile rewrites
unless the user requests them.

A manual download and extraction path remains available beside the installer.

Nix remains supported when the direct channel arrives. Direct-install users do
not need Nix, and Nix users receive the same accepted component set.

### macOS

Venus determines the native application boundary. Astra should plan a signed and
notarized application or disk image and a Homebrew Cask over the same release
artifacts. Architecture reviews must reject process, path, PTY, or packaging
foundations that prevent this channel.

## Website

During alpha, `yazelix.com` can explain Astra and label Nix as the required
dogfood path. After distribution graduation, the site can publish direct
installation instructions and select a platform-specific installer. GitHub
release assets or another immutable object store remain the authority for
binaries, checksums, signatures, attestations, and historical versions.

The site should keep copy-and-paste install commands inspectable. A convenience
script must resolve to a pinned release, use TLS, verify the downloaded artifact,
and support manual inspection before execution.

## Deferred channels

AppImage, Flatpak, Debian and RPM packages, container images, and system package
repositories need evidence from direct-bundle dogfooding. Each channel adds an
update model and platform policy. Add one only when users need it and the
canonical manifest can drive it without a parallel product definition.

## Release checks

An alpha candidate should prove:

- Nix consumes the reviewed component manifest;
- the installed composition reports every exact component revision;
- normal runtime use invokes no Nix evaluator;
- product state uses stable component identity instead of persisted store paths;
- Linux runs in a clean Nix-enabled user environment.

A direct release candidate should also prove:

- component revisions match the reviewed manifest;
- direct and Nix channels report the same component set;
- bundles contain no undeclared store paths or machine-local references;
- installation, upgrade, rollback, and removal preserve user data;
- signatures or attestations bind the published checksums;
- Linux runs in a clean user environment;
- macOS passes signing, notarization, quarantine, and first-launch checks once
  that channel exists.
