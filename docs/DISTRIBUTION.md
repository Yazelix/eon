# Distribution and Composition

## Goals

Astra should install from a direct download without requiring Nix. Users who
choose Nix or Home Manager should receive the same component set and runtime
semantics.

## Canonical release input

A versioned component manifest records each component's project, exact revision,
artifact, checksum, platform, and compatibility contract. Release tooling consumes
this manifest to assemble every channel.

The manifest should remain small enough for humans to review. Child repositories
build and test their owned artifacts. Astra verifies compatibility and assembles
the product without rebuilding child behavior.

## Planned channels

### Linux direct bundle

The first direct channel should publish a compressed relocatable bundle, checksum,
signature or attestation, and a small installer. The installer should support a
versioned user prefix plus an atomic `current` link, a dry run, and removal. It
must avoid shell-profile rewrites unless the user requests them.

A manual download and extraction path remains available beside the installer.

### Nix and Home Manager

Nix stays a first-class optional channel for reproducible composition, source
builds, development, and users who already manage systems with Nix. The Nix
expressions consume the canonical component manifest. They must not define a
different product graph or require direct-install users to adopt Nix.

### macOS

Venus determines the native application boundary. Astra should plan a signed and
notarized application or disk image and a Homebrew Cask over the same release
artifacts. Architecture reviews must reject process, path, PTY, or packaging
foundations that prevent this channel.

## Website

`yazelix.com` can explain Astra, publish installation instructions, and select a
platform-specific installer. GitHub release assets or another immutable object
store remain the authority for binaries, checksums, signatures, attestations,
and historical versions.

The site should keep copy-and-paste install commands inspectable. A convenience
script must resolve to a pinned release, use TLS, verify the downloaded artifact,
and support manual inspection before execution.

## Deferred channels

AppImage, Flatpak, Debian and RPM packages, container images, and system package
repositories need evidence from direct-bundle dogfooding. Each channel adds an
update model and platform policy. Add one only when users need it and the
canonical manifest can drive it without a parallel product definition.

## Release checks

A release candidate should prove:

- component revisions match the reviewed manifest;
- direct and optional Nix channels report the same component set;
- bundles contain no undeclared store paths or machine-local references;
- installation, upgrade, rollback, and removal preserve user data;
- signatures or attestations bind the published checksums;
- Linux runs in a clean user environment;
- macOS passes signing, notarization, quarantine, and first-launch checks once
  that channel exists.
