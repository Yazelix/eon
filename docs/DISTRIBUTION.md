# Distribution and Composition

## Goals

Eon alpha and early dogfood use Nix as the sole installation and composition
channel. This phase spends project time on Eon Sessions, Eon Desktop, and Eon
contracts instead of portable archives and installer maintenance.

Direct installation remains the long-term adoption path. It begins after Nix
dogfood proves a useful product and the user approves distribution graduation.
Every channel consumes the same component graph and preserves runtime semantics.

## Phase policy

### Alpha and early dogfood

One Nix flake path builds, composes, and installs the accepted component set.
Home Manager remains separate work and must consume the same package and product
contract if the user activates it.

The canonical local installation command is `nix profile add .#default`. The
package installs the `eon` command, an Eon desktop entry, and its icon. The
runtime commands, configuration paths, and detach behavior live in the Eon
binary rather than the Nix wrapper.

The Nix-only phase follows these boundaries:

- Eon code accepts component paths and versions through explicit inputs.
- Nix store paths remain opaque launch values rather than stable identity.
- Product state and user configuration contain stable component identities.
- Normal runtime operation invokes no Nix command or evaluator.
- Direct installers, portable archives, signing, and package matrices stay out
  of the alpha implementation surface.

### Distribution graduation

Direct-distribution work requires a separate user decision after:

- the Nix composition survives sustained fresh-session dogfood;
- launch, configuration, diagnostics, upgrades, and failure behavior stabilize;
- exact Eon Sessions and Eon Desktop revisions have no open P0 or P1 integration defect;
- the component manifest describes the accepted product without relying on
  derivation identity;
- the team can maintain release artifacts and their verification checks.

The graduation decision selects the first direct target and release tool. It
does not activate every package channel.

## Reference routing

These routes remain conditional during the Nix-only alpha. Read only the row
matching an activated distribution question, start with `Read first`, and
inspect an additional source only when its condition applies. Record exact
revisions, constraints, and rejections in the owning Bead.

| Question or trigger | Read first | Read additionally only if | Boundary |
|---|---|---|---|
| Distribution graduation is active and one release graph must drive direct archives and package-manager outputs. | [dist documentation](https://axodotdev.github.io/cargo-dist/) | Read [dist configuration](https://axodotdev.github.io/cargo-dist/book/reference/config.html) only when evaluating its exact artifact, installer, Homebrew, or generated-CI shape. | Apply the tool gate and compare an owned assembler. Do not select dist before graduation. |
| GitHub-hosted artifacts must preserve release identity. | [GitHub immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases) | None by default. | Prove tag protection, attestations, and asset immutability for the selected workflow. |
| Hosted Actions minutes, queues, or spending create a demonstrated local-release requirement. | [Doodlestein Self Releaser](https://github.com/Dicklesworthstone/doodlestein_self_releaser) | Inspect `act` and the selected workflow only after exact compatibility and cost questions are named. | Comparison evidence only. A local run does not prove hosted parity, signing isolation, or provenance. |
| Native macOS distribution is activated. | [Apple notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) | Read the [Homebrew Cask Cookbook](https://docs.brew.sh/Cask-Cookbook) only if Homebrew is selected as a channel. | Prove signing, hardened runtime, notarization, stapling, Gatekeeper, quarantine, architecture, and upgrade behavior. |
| Direct-bundle dogfood demonstrates a need for a portable Nix artifact. | [Nix `bundle` command](https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-bundle) | None by default. | Inspect experimental status, bundler constraints, closure size, and portability. Do not make an experimental interface canonical. |
| Direct-bundle dogfood demonstrates an AppImage-specific user need. | [AppImage documentation](https://docs.appimage.org/) | None by default. | Inspect integration, updates, sandbox assumptions, and filesystem support. Do not treat format availability as product demand. |

## Cost-bounded CI and release execution

Do not make hosted GitHub Actions consumption an implicit prerequisite for
Eon alpha. Keep canonical build and verification commands runnable locally
and through Nix. Before adding hosted CI or release automation, record the
expected run frequency, runner minutes, storage and artifact costs, cancellation
policy, timeout, and the user's hard spending boundary.

[Doodlestein Self Releaser](https://github.com/Dicklesworthstone/doodlestein_self_releaser)
is a research reference for reusing GitHub Actions workflow YAML locally through
`act` and then publishing verified release artifacts when hosted capacity is
constrained. Its presence does not select Doodlestein or `act`. The tool gate
must compare local owned commands, Nix-native execution, cost-bounded hosted
Actions, Doodlestein, and any other credible shape. It must test workflow
compatibility, artifact parity, secret and signing boundaries, provenance, and
failure recovery rather than assuming a local container matches GitHub's
runner.

## Canonical release input

A versioned component manifest records each component's project, exact revision,
package or artifact input, platform, and compatibility contract. Nix consumes
this manifest during alpha. Release tooling extends it with portable artifacts,
checksums, and provenance after distribution graduation.

The manifest should remain small enough for humans to review. Child repositories
build and test their owned artifacts. Eon verifies compatibility and assembles
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

Eon Desktop determines the native application boundary through its Venus
subsystem. Eon should plan a signed and notarized application or disk image and
a Homebrew Cask over the same release artifacts. Architecture reviews must
reject process, path, PTY, or packaging foundations that prevent this channel.

## Website

During alpha, `yazelix.com` can explain Eon and label Nix as the required
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
