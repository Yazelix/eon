# Distribution and Composition

## Goals

Eon publishes through two channels. Nix is the first, and it also composes
and builds every artifact. From v0.2.0 the second is the signed Nix-free Linux
archive and [installer](INSTALLER.md) on GitHub releases, which Lucca approved
on 2026-11-06. Any other channel needs the user's approval.

Both channels target x86_64 Linux on native Wayland. Apple
Silicon macOS is the only approved expansion: from v0.2.0 it ships as a
labelled preview through the Nix channel only, outside any stability
promise, as
[EON-C7](CONTRACTS.md#eon-c7--native-linux-wayland-and-apple-silicon-macos)
states. Another Linux architecture, X11, Xwayland, Intel macOS, macOS signing
and notarization, and macOS distribution outside Nix need
separate scope and proof.

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

Direct installation remains the long-term adoption path. It began after Nix
dogfood proved a useful product and the user approved distribution graduation.
Every channel consumes the same component graph and preserves runtime semantics.

## Phase policy

### Versions and stages

Eon's product version in `eon/crates/eon/Cargo.toml` is its one user-facing
number; `eon --help`, `eon versions` and the window's `EON` label (when
shown) report it. The compiled `RELEASE_STAGE` constant beside the version
input in `eon/crates/eon/src/product.rs` supplies the stage independently of
that number. A stage change updates that constant together with the Cargo
version only at the explicitly authorized release cut:
`ReleaseStage::Alpha`, `ReleaseStage::Beta`, or `ReleaseStage::Stable`. The bar
shows `EON α0.1.24`, `EON β0.2.0`, or `EON 1.0.0`, respectively; CLI version
output keeps its numeric form. Eon remains in alpha. v0.2.0 is the planned first
beta; its version/stage switch requires the release owner's explicit go-ahead.
The signing key, release workflow and download instructions are prepared for
that cut; download commands require publication. In alpha and beta, commands,
configuration and behavior may still change, and the changelog records what
did. `eon-igae` holds the scope for entering beta.
1.0.0 has no scheduled date: user reports, not a plan, decide when it comes.
[EON-C28](CONTRACTS.md#eon-c28--1x-stability) states what 1.x would keep stable.
Orbit, Venus and Eon Runtime keep their own package versions, and EONW its
protocol version, as compatibility identities.

A release bumps the product version, tags that commit `vX.Y.Z`
(`eon-versioning-beta-bfyf`, decided 2026-10-16) and advances `main` to it
([Git Workflow](AGENT-RULES.md#git-workflow)). The earlier
`linux-alpha-2026-10-14` tag is version 0.1.0, and the first numbered
release is `v0.1.1`. Installing without Nix is a delivery milestone, not a
stage; publishing it gates beta.
Eon's product version is bumped at least whenever EONW's protocol version
changes, so one version never names builds that cannot control each other
(`eon-7sjm`, decided 2026-10-24).

### Nix channel

One Nix flake path builds, composes, and installs the accepted component set.
Home Manager remains separate work and must consume the same package and product
contract if the user activates it.

The canonical local installation command is `nix profile add .#default`. The
package installs the `eon` command, an Eon desktop entry, and its icon. The
runtime commands, configuration paths, and detach behavior live in the Eon
binary rather than the Nix wrapper.

Install Eon without Helix from the same flake with `nix profile add .#no-helix`.
Use `github:Yazelix/eon/main#no-helix` from v0.2.0 or
`github:Yazelix/eon/edge#no-helix` before that release. This variant is
available through Nix on x86_64 Linux. Its Nix output evaluates on Apple
Silicon macOS. Build and runtime proof there remain pending. Set
[`[editor] command`](CONFIGURATION.md#editor) in `config.toml`, for example
`command = ["nvim"]` under `[editor]`. With the setting unset, Eon uses the
`VISUAL` or `EDITOR` value inherited when the window starts. Desktop launches
may have neither variable. Use Nix to upgrade the profile entry. Support for
this variant in `eon update` is tracked in `eon-igae.17.4`.

On Apple Silicon macOS the same package contains
`Applications/Eon.app`, with its version, Eon icon and a native launcher for
the package's exact `eon` command. With a plain Nix profile install, open
`~/.nix-profile/Applications` through Finder's **Go > Go to Folder**, then
open `Eon.app` or drag it to the Dock. Keep the Dock target through the
profile path so upgrades follow the profile. Nix does not copy profile apps
into the user's Applications folder automatically.

Existing nix-darwin `environment.systemPackages` integration copies app
bundles into `/Applications/Nix Apps`. Home Manager `home.packages`
integration uses `~/Applications/Home Manager Apps`; its
`targets.darwin.copyApps` copies bundles (default with stateVersion 25.11 or
later), while older `targets.darwin.linkApps` links them. Eon supplies the
conventional package layout and requires no Eon-specific module. These
integration tools own their activation and updates; this package changes
neither the user's Dock nor their application directories. Spotlight indexing
is not promised by a plain profile install.

The app launcher looks up the account's login shell in macOS's account
database and asks it for PATH in login/interactive mode. Only PATH is imported;
shell startup output and other environment changes are discarded. Eon's
packaged commands are prepended afterward. A five-second safety limit prevents
a broken startup file from blocking launch: lookup, shell failure or timeout
appends a diagnostic to `$XDG_STATE_HOME/eon/logs/app-launch.log` (default
`~/.local/state/eon/logs/app-launch.log`) and retains the launching
environment's PATH. The supported login-shell modes are Bash, zsh and
Nushell; another account
shell uses the same logged fallback. No shell-startup marker or hardcoded
host-tool directory list supplies the environment. Bundle launch remains
unproved until the exact candidate's mac-proof and Yu's Dock check pass.
The current harness supplies launch arguments: it does not prove the
no-argument Finder/Dock launch or a click while Eon runs. Those are follow-ups
`eon-mac-launch-kz4p.59` (native reopen/Quit) and `eon-mac-launch-kz4p.60`
(dispatcher and Venus in the bundle).

The Nix channel follows these boundaries:

- Eon code accepts component paths and versions through explicit inputs.
- Nix store paths remain opaque launch values rather than stable identity.
- Product state and user configuration contain stable component identities.
- Normal runtime operation invokes no Nix command or evaluator.
- Direct distribution follows the separately approved graduation gates;
  package matrices remain out of scope.

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
installer, then qualify that Nix-free install. The accepted decision candidate
predates this repository; `eon-p93` records its exact revisions and artifact,
the exact graph, and scoped proof reuse including reader and lifecycle evidence.
The [manual bundle](MANUAL-BUNDLE.md) and [installer](INSTALLER.md) proved the
direct-delivery lifecycle under EON-C5/C6 before consolidation and before
publication; that proof awaits re-proof in this repository (`eon-igae.7.3`).

`eon-qualify-nix-free-linux-public-beta-lm9p` qualified the Nix-free Linux
install and recorded a Go on 2026-10-21, with the
[limits](INSTALLER.md#limits-of-the-published-install) the installer guide
lists. On 2026-11-06 Lucca approved publishing it with the first beta release
(`eon-igae.15`): GitHub release assets on Yazelix/eon, SHA-256 checksums signed
with the release key, and GitHub artifact attestations. Redistribution
clearance for the published payload remains a separate gate.

Lucca Huguet owns manual releases when he chooses, using existing local locked
Nix builds and Rust manifest validation. No additional release framework or
composition graph is selected; the one hosted job is the manually dispatched
[release workflow](INSTALLER.md#release-assets). Additional hosted spending is $0,
with no promised support response time. Failed staging leaves the prior
installation usable. Upgrades retain executables and payloads needed by live
work or defer replacement; no Orbit/machine restart persistence is promised.

The release workflow and local parity build select the same
[archive route](INSTALLER.md#release-assets): the full route with a recorded
Accelerando grant, or the route without Accelerando when no grant exists.

The accepted cost envelope uses provisional estimates of 0.5–1.0 GB per archive,
1.8–3.0 GB per extracted payload and 6.4–11.0 GB for two payloads, staging and
two archives, excluding build caches and extra live generations. Its historical
Nix-closure/compression measurements are comparison evidence, not portable
bundle measurements. The manual bundle must measure actual costs and prove
artifact verification and clean non-Nix execution. Material cost/capacity
excess, paid services, added ownership, verification failure or portability
failure reopens the decision; staying Nix-only is the fallback. Publication
was approved separately on 2026-11-06, after the qualification Go.

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
Eon releases. Keep canonical build and verification commands runnable locally
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
reads are public; the credential check and the two upload steps receive the token.
A missing secret fails before installing tools or building the package.

A push to `edge` or a manual dispatch first runs the pinned Helix grammar check.
The check rejects all five evaluator source-fetch builtins (`fetchTree`,
`fetchGit`, `fetchMercurial`, `fetchTarball` and `fetchurl`) and compiles every
selected grammar through Helix's builder. The job then builds
`packages.x86_64-linux.helix-grammar-sources`, including disabled grammars, and
pushes that source closure with `cachix push`. It pins the aggregate as
`helix-grammar-sources-<helix rev>`, using the exact Helix revision in
`flake.lock` and `cachix pin --keep-forever` to retain it.

The job then builds `packages.x86_64-linux.default`, `eonbase`, and `no-helix`
together. The packages share an `eon-rust` executable and require a build-only
check that runs Eon and Eon Runtime's tests through cached dev-profile Nextest,
including serial ignored lease tests against release Orbit and separate
doctests. After the no-Helix closure check passes, a second `cachix push`
publishes the three packages, the shared `eon-rust` output, the `eon-tests`
output containing `test-list.json`, and the runtime dependencies of those
outputs.

Cachix skips paths that `cache.nixos.org` already serves, including grammar
sources with the same store identity. Eon's Cachix pin does not retain those
copies: they remain available from the owned source archives and
`cache.nixos.org`. Keep that standard cache and its signing key configured
alongside Eon's cache. See the [Cachix FAQ](https://docs.cachix.org/faq).
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
public-runner eligibility changes. Local `nix build .#default .#eonbase` remains
canonical.

To verify publication, check both exact product outputs' `.narinfo` records at
the cache URL. Check that `helix-grammar-sources-<helix rev>` points to the source
aggregate from that revision, with permanent retention, and inspect a catalog
source output's `.narinfo` at the cache serving it. For a source Cachix skipped,
check `cache.nixos.org` instead. Realise the products and source aggregate in a
fresh Nix store with both caches and signing keys configured, signatures required
and local builds disabled. A local build alone does not prove upload, retention
or fresh-store source substitution.

## Canonical release input

A versioned component manifest records each component's project, source,
abstract artifacts, platform, and compatibility contract. External components
carry an exact revision; the repository revision identifies in-tree ones. Nix consumes this
manifest today and owns physical package paths. Release tooling extends
the graph with consumed portable artifact locators, checksums, and provenance
after distribution graduation.

The manifest should remain small enough for humans to review. Each in-tree
subsystem builds and tests its owned crates; Eon verifies compatibility and
assembles the product without copying subsystem behavior.

## Channel sequence

### Nix

Nix supplies the first published channel and the one channel for reproducible
composition, source builds, development, and dogfood. The flake consumes the canonical
component manifest and owns no product behavior.

The initial slice should support one installation path. Home Manager can follow
through a separate decision without changing the package or runtime contract.

### Linux direct bundle

`eon-prove-manual-nix-free-linux-bundle-216b` proved the compressed relocatable
bundle of the earlier Bubblewrap mechanism and its verification on an isolated
non-Nix native-Wayland environment.
`eon-prove-thin-linux-bundle-installer-8r7n` proved the static Rust installer
over that artifact, with a versioned user prefix, atomic `current` link,
verified dry-run, active-work-safe upgrade/rollback and owned removal. It
retains every payload and edits no shell profile. Both await re-proof with the
relinked payload. The Nix-free install was qualified separately and is
published from v0.2.0 as GitHub release assets
([installer](INSTALLER.md#install-a-release)).

A manual download and extraction path remains available beside the installer.

Nix remains supported beside the direct channel. Direct-install users do
not need Nix, and Nix users receive the same accepted component set.

## Website

From v0.2.0 the Nix-free install is published, so `eonterm.com` can carry
direct Linux installation instructions beside the Nix path. GitHub release
assets remain the authority for
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

A Nix release candidate should prove:

- Nix consumes the reviewed component manifest;
- the installed composition reports every exact external component revision and
  the generation of its in-tree build;
- normal runtime use invokes no Nix evaluator;
- product state uses stable component identity instead of persisted store paths;
- Eon runs in a clean Nix-enabled native Wayland environment.

EON-C7 has an installed x86_64 Linux Wayland slice on private Sway and a
partial Apple Silicon slice. It remains partially proved because full Eon on
Apple Silicon macOS is not fully proved. A non-systemd
support claim separately requires installed dogfood on a non-systemd native
Wayland host with the documented kernel capabilities.

A direct release candidate should also prove:

- external component revisions match the reviewed manifest;
- direct and Nix channels report the same component set;
- bundles contain no undeclared store paths or machine-local references;
- installation, upgrade, rollback, and removal preserve user data;
- signatures or attestations bind the published checksums;
- Eon runs in a clean native Wayland environment.
