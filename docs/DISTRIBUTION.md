# Distribution and Composition

## Goals

Eon alpha and early dogfood use Nix as the sole installation and composition
channel. This phase spends project time on Orbit, Venus, and Eon contracts
instead of portable archives and installer maintenance.

The current accepted channel targets x86_64 Linux on native Wayland. Apple
Silicon macOS is the only approved expansion and remains within the same
Nix-only alpha channel until native proof succeeds. Another Linux architecture,
X11, Xwayland, Intel macOS, signing, notarization, direct bundles, and public
macOS distribution need separate scope and proof.

Eon's Nix Orbit package explicitly selects `LIBGHOSTTY_VT_SYS_CPU=baseline`.
The selected libghostty binding owns the Zig compiler invocation; Eon owns this
distribution policy. The native library must not inherit the cache builder's
CPU features. Older-hardware qualification applies only to the exact tested
artifact; a successful local build does not prove cache substitution.

No channel requires systemd or another specific init or service manager. The
accepted runtime proof used COSMIC with systemd before consolidation and awaits
re-proof here, so non-systemd native Wayland remains unproved. Eon and Orbit require no cgroup delegation. Orbit's accepted
Linux lifecycle uses bounded PTY process-group shutdown and direct-child
reaping; deliberately detached processes may survive terminal stop.

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

The Nix alpha follows these boundaries:

- Eon code accepts component paths and versions through explicit inputs.
- Nix store paths remain opaque launch values rather than stable identity.
- Product state and user configuration contain stable component identities.
- Normal runtime operation invokes no Nix command or evaluator.
- Direct distribution follows the separately approved graduation gates;
  package matrices remain outside the alpha surface.

### Distribution graduation

Direct-distribution work requires a separate user decision after:

- the Nix composition survives sustained dogfood with fresh terminals;
- launch, configuration, diagnostics, upgrades, and failure behavior stabilize;
- the exact Eon revision has no unresolved P0 or P1 Orbit or Venus
  integration defect outside an explicitly accepted limitation;
- the component manifest describes the accepted product without relying on
  derivation identity;
- the team can maintain release artifacts and their verification checks.

`eon-p93` completed the graduation decision on 2026-10-10: Go to develop one
manual Nix-free x86_64 Linux native-Wayland bundle, then its thin user-local
installer, then qualify the public beta. The accepted decision candidate
predates this repository; `eon-p93` records its exact revisions and artifact,
the exact graph, and scoped proof reuse including reader and lifecycle evidence.
Normal alpha installation remains Nix-only. The unpublished
[manual bundle](MANUAL-BUNDLE.md) and [installer](INSTALLER.md) proved the
unpublished direct-delivery lifecycle under EON-C5/C6 before consolidation;
that proof awaits re-proof in this repository. Public-beta
qualification, release trust and redistribution clearance remain separate gates.

Alpha development remains the active focus by user decision on 2026-10-11.
`eon-qualify-nix-free-linux-public-beta-lm9p` is deferred until the user explicitly
resumes beta qualification. Accepted bundle and installer proof does not
change the alpha channel or authorize publication.

Lucca Huguet owns manual releases when he chooses, using existing local locked
Nix builds and Rust manifest validation. No additional release framework,
composition graph or hosted job is selected. Additional hosted spending is $0,
with no promised support response time. Failed staging leaves the prior
installation usable. Upgrades retain executables and payloads needed by live
work or defer replacement; no Orbit/machine restart persistence is promised.

The accepted cost envelope uses provisional estimates of 0.5–1.0 GB per archive,
1.8–3.0 GB per extracted payload and 6.4–11.0 GB for two payloads, staging and
two archives, excluding build caches and extra live generations. Its historical
Nix-closure/compression measurements are comparison evidence, not portable
bundle measurements. The manual bundle must measure actual costs and prove
artifact verification and clean non-Nix execution. Material cost/capacity
excess, paid services, added ownership, verification failure or portability
failure reopens the decision; staying Nix-only is the fallback. Publication
requires its separate approval and delivery/public-beta gates.

On 2026-10-10 the user accepted the demonstrated
[Orca #757](https://gitlab.gnome.org/GNOME/orca/-/work_items/757) Say All
reading-position failure as an upstream limitation for distribution work.
`eon-p93` records its scope and review conditions. On 2026-10-11, before
consolidation, EON-C8 verified the upstream correction at exact Orca `d13f83dc`
with GNOME Terminal/Vim and bounded current-development Eon workflows; that
qualification awaits re-proof in this repository. The historical reader failures
remain recorded; older readers and broader support remain unqualified. This
review changes no distribution approval, accepts no pending child revision
and waives no other defect.

## Reference routing

These routes follow the distribution gates. Read only the record
matching an activated distribution question, start with `Read first`, and
inspect an additional source only when its condition applies. Record exact
revisions, constraints, and rejections in the owning Bead.

### Distribution graduation is active and one release graph must drive direct archives and package-manager outputs.

- **Read first:** [dist documentation](https://axodotdev.github.io/cargo-dist/)
- **Read additionally only if:** Read [dist
  configuration](https://axodotdev.github.io/cargo-dist/book/reference/config.html) only
  when evaluating its exact artifact, installer, Homebrew, or generated-CI shape.
- **Boundary:** Apply the tool gate and compare an owned assembler. Do not select dist
  before graduation.

### GitHub-hosted artifacts must preserve release identity.

- **Read first:** [GitHub immutable
  releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
- **Read additionally only if:** None by default.
- **Boundary:** Prove tag protection, attestations, and asset immutability for the selected
  workflow.

### Hosted Actions minutes, queues, or spending create a demonstrated local-release requirement.

- **Read first:** [Doodlestein Self
  Releaser](https://github.com/Dicklesworthstone/doodlestein_self_releaser)
- **Read additionally only if:** Inspect `act` and the selected workflow only after exact
  compatibility and cost questions are named.
- **Boundary:** Comparison evidence only. A local run does not prove hosted parity, signing
  isolation, or provenance.

### Linux product or desktop work could widen the Wayland ownership boundary.

- **Read first:** [foot at
  `85655c74a4de`](https://codeberg.org/dnkl/foot/src/commit/85655c74a4ded119392ea8b632626c3920042807)
- **Read additionally only if:** Follow Eon's [central Monstar
  route](REFERENCES.md#terminal-product-comparisons) only when direct Wayland ownership is
  in question.
- **Boundary:** Preserve the accepted Linux Wayland proof and keep presentation
  in Venus; reject X11, Xwayland, and an Eon-owned raw-Wayland rewrite. The
  separately approved Apple Silicon proof uses Venus's native-host boundary and
  does not derive its shape from this Linux reference.

### Distribution graduation needs a compact Linux archive and package-publishing comparison.

- **Read first:** Eon's [central Monstar route](REFERENCES.md#terminal-product-comparisons)
- **Read additionally only if:** Follow its exact release workflow, binary assembler, and
  AUR templates only after Eon activates distribution graduation.
- **Boundary:** Study release-built archives, debug stripping, and package publication.
  Preserve Eon's canonical component graph, exact child artifacts, verification, and
  multi-process ownership; reject single-binary size parity as an Eon release goal.

### Direct-bundle dogfood demonstrates a need for a portable Nix artifact.

- **Read first:** [Nix `bundle`
  command](https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-bundle)
- **Read additionally only if:** None by default.
- **Boundary:** Inspect experimental status, bundler constraints, closure size, and
  portability. Do not make an experimental interface canonical.

### Direct-bundle dogfood demonstrates an AppImage-specific user need.

- **Read first:** [AppImage documentation](https://docs.appimage.org/)
- **Read additionally only if:** None by default.
- **Boundary:** Inspect integration, updates, sandbox assumptions, and filesystem support.
  Do not treat format availability as product demand.

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

## Optional binary cache

`eon-yazelix` is a public Cachix cache donated and managed by
[bioinformatist](https://github.com/bioinformatist), separate from Lucca's cache
storage. It accelerates the existing Nix path; it is not a release channel or a
requirement for source builds. Cache misses still build from the pinned sources.
Consumers choose whether to trust this third-party binary provider.

The flake advertises these additive settings, leaving existing caches intact:

```nix
extra-substituters = [ "https://eon-yazelix.cachix.org" ];
extra-trusted-public-keys = [
  "eon-yazelix.cachix.org-1:kPIp7u0Gjoz0GNzmRqeaqs798vU2raSOUpK+3xi7oK0="
];
```

Accept the flake settings when Nix prompts, or use the Cachix CLI to configure
them explicitly with `cachix use eon-yazelix`. Setup depends on your installation:

- **NixOS:** integrate the generated Cachix configuration into your system
  configuration, then activate it with your normal NixOS rebuild/switch workflow
  before installing Eon. Writing the configuration alone does not activate the
  cache; see the [Cachix FAQ](https://docs.cachix.org/faq).
- **Multi-user Nix:** if your user is not trusted by the Nix daemon, ask an
  administrator to configure the cache URL and signing key for the daemon.
  Accepting flake settings does not bypass these
  [daemon permissions](https://nix.dev/manual/nix/2.35/installation/multi-user.html).
- **Single-user Nix:** the CLI updates your writable `nix.conf`; no NixOS
  activation is needed.

Flake input consumers must configure their consuming installation; an input's
`nixConfig` does not configure the parent flake. Historical tags do not acquire
the new hints or guaranteed cache coverage.

### Publishing

Before activating `.github/workflows/cachix.yml`, a repository maintainer must
add the cache owner's per-cache write token as the `CACHIX_AUTH_TOKEN` Actions
repository secret. Never put the token in Git, an issue, or a pull request. Cache
reads are public; only the credential check and upload step receive the token.
A missing secret fails before installing tools or building the package.

A push to `edge` or a manual dispatch first runs the pinned Helix grammar check,
which rejects Codeberg and SourceHut evaluation fetches and compiles their bundled
sources. It then builds `packages.x86_64-linux.default`
and `packages.x86_64-linux.eonterm` with their required package checks. An explicit
`cachix push` publishes both outputs and their runtime dependencies. Paths
already available from `cache.nixos.org` remain there rather than being copied
into Cachix; keep that standard cache and its signing key configured alongside
Eon's cache. See the [Cachix FAQ](https://docs.cachix.org/faq).
It does not upload every compiler, build intermediate, or unrelated store path.
No pull-request event runs the workflow, and forks cannot publish through it.
Maintain repository write access carefully: trusted workflow editors can expose
Actions secrets.

Each run uses one standard `ubuntu-24.04` GitHub-hosted runner, with a 90-minute
job timeout and cancellation of superseded runs on the same ref. Frequency is
one run per `edge` push or explicit dispatch. It runs only in the public upstream
repository: standard public-runner minutes are free and do not draw from the
organization's private-repository allowance. No larger runner,
Actions artifact, or Actions cache is used. GitHub paid runner spend is zero;
Cachix storage and retention remain with bioinformatist. Remove or revisit this
job if builds repeatedly exceed the timeout, the cache donation ends, or free
public-runner eligibility changes. Local `nix build .#default .#eonterm` remains
canonical.

To verify publication, check both exact outputs' `.narinfo` records at the cache
URL and realise them in a fresh Nix store with both caches and signing keys
configured, signatures required, and local builds disabled. A local build alone
does not prove upload or fresh-store substitution.

## Canonical release input

A versioned component manifest records each component's project, source,
abstract artifacts, platform, and compatibility contract. External components
carry an exact revision; the repository revision identifies in-tree ones. Nix consumes this
manifest during alpha and owns physical package paths. Release tooling extends
the graph with consumed portable artifact locators, checksums, and provenance
after distribution graduation.

The manifest should remain small enough for humans to review. Each in-tree
subsystem builds and tests its owned crates; Eon verifies compatibility and
assembles the product without copying subsystem behavior.

## Channel sequence

### Nix alpha

Nix supplies the first and sole alpha channel for reproducible composition,
source builds, development, and dogfood. The flake consumes the canonical
component manifest and owns no product behavior.

The initial slice should support one installation path. Home Manager can follow
through a separate decision without changing the package or runtime contract.

### Linux direct bundle

`eon-prove-manual-nix-free-linux-bundle-216b` proves the compressed relocatable
bundle and verification on an isolated non-Nix native-Wayland environment.
`eon-prove-thin-linux-bundle-installer-8r7n` proves the static Rust installer
over that artifact, with a versioned user prefix, atomic `current` link,
verified dry-run, active-work-safe upgrade/rollback and owned removal. It
retains every payload and edits no shell profile. Wider public-beta
qualification and publication follow their separate gates.

A manual download and extraction path remains available beside the installer.

Nix remains supported when the direct channel arrives. Direct-install users do
not need Nix, and Nix users receive the same accepted component set.

## Website

During alpha, `yazelix.com` can explain Eon and label Nix as the required
dogfood path. After accepted delivery proof and publication approval, the site
can publish direct Linux installation instructions. GitHub
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
- the installed composition reports every exact external component revision and
  the generation of its in-tree build;
- normal runtime use invokes no Nix evaluator;
- product state uses stable component identity instead of persisted store paths;
- Eon runs in a clean Nix-enabled native Wayland environment.

EON-C7 has an installed x86_64 Linux Wayland slice on private Sway. It remains
partially proved because Apple Silicon macOS is still unproved. A non-systemd
support claim separately requires installed dogfood on a non-systemd native
Wayland host with the documented kernel capabilities.

A direct release candidate should also prove:

- external component revisions match the reviewed manifest;
- direct and Nix channels report the same component set;
- bundles contain no undeclared store paths or machine-local references;
- installation, upgrade, rollback, and removal preserve user data;
- signatures or attestations bind the published checksums;
- Eon runs in a clean native Wayland environment.
