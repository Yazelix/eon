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

### Release signing custody and recovery

Custody assurance from the release owner, 2026-11-16: the OpenSSH Ed25519
private key for identity `eon-release`, namespace `eon-bundle`, lives in
owner-controlled cloud storage. The key stays outside development and agent
environments, including during signing. Only the release owner signs, by hand.
A backup exists; this record establishes neither its location nor a recovery
rehearsal. Authenticated cloud-session reachability from the development
environment remains **not independently verified**.

Under this reported custody, a compromised worker cannot sign with a local
private-key file. The owner must approve the exact checksums through the
[release-assets procedure](INSTALLER.md#release-assets). A signature
authenticates those checksums under the trusted release key; it does not prove
safe source or an uncompromised build. A compromised worker could alter source,
the local build or the comparison and induce the owner to sign malicious
content. Separate key custody does not remove that content-integrity risk.

Agents never create, move, delete or push `v*` tags or run any signing step.
An agent asked to sign refuses and reports the request. These are standing
agent rules, not technical access restrictions.

Release owner policy, 2026-11-16: on key loss or suspected compromise:

1. Generate a replacement Ed25519 signer outside development and agent
   environments.
2. Publish the replacement allowed-signers line in the README and release
   notes, and update the [installer signer setup](INSTALLER.md#install-a-release).
   Identify the revoked old public key in [SECURITY.md](../SECURITY.md#release-signing-and-revocation).
3. Publish a new patch release signed with the replacement key. Preserve
   existing immutable tags and assets. The release notice names the old key
   and which releases users must stop trusting.
4. Tell users to remove or replace the old entry in their local allowed-signers
   files using an independently authenticated replacement. This includes the
   file selected by `updates.allowed_signers` for direct updates. Publishing a
   revocation notice does not change local trust files.

A repository-hosted notice alone cannot authenticate a replacement key if
repository authority is compromised. A separate replacement-key authentication
route remains **unverified**.

This records the recovery procedure; an end-to-end recovery and replacement
publication rehearsal remains **unverified**.

The read-only repository ruleset API observation on 2026-11-16 found active
**Protect release tags** rules for `refs/tags/v*`, with no exclusions. The
rules restrict tag creation, updates and deletion; repository administrators
can always bypass them. Maintainer automation runs with administrator access,
so the ruleset does not stop a compromised agent from creating a release tag.
A rogue tag supplies no release signature. The signature remains the signed
bundle installer's authorization boundary; Nix cache publication has separate
authority.

The [Linux cache workflow](../.github/workflows/cachix.yml) runs on `edge`
pushes, `v*` tag pushes or manual dispatch, and publishes cache roots on each
run. The [Darwin cache workflow](../.github/workflows/cachix-darwin.yml)
runs on `v*` tag pushes or manual dispatch and publishes cache roots when its
publishing switch is enabled and credentials are available. On both platforms a
run for the newest `vX.Y.Z` tag also moves that platform's
`latest-release-<platform>` pin ([Release retention](#release-retention)). A rogue
tag can therefore publish to the cache without a bundle signature, and a rogue
`vX.Y.Z` tag above every release takes over those pins until a higher release
replaces it. A maintainer recovers by deleting the rogue tag and rerunning
both cache workflows on the newest genuine tag. Actual workflow activation and cache access policy
remain **unverified**.

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

The release workflow and the local parity build both build `.#release`
([release assets](INSTALLER.md#release-assets)).

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

### Pull request checks

`.github/workflows/pr-checks.yml` gives every pull request into `edge`, from a
fork or not, one quick check, "Pull request checks / quick". It runs once per
push to the pull request; a newer push cancels the superseded run. One standard
`ubuntu-24.04` runner, free for public repositories, with a 15-minute timeout
and no artifacts, caches or storage; additional paid spend is $0, and a private
copy of the repository skips the job. It checks whitespace in the pull request's
diff, builds the quick flake checks (workflows and shell scripts, the release
pin script, the unsafe policy, and the other checks that build no Cargo crate
graph, Orbit, Venus, Eon or Ghostty), and evaluates every Linux and Darwin
check, package and development shell without building them. Rust suites,
package builds and real-PTY checks stay on `edge` pushes and the author's and
maintainer's proof.

The pull request's code is untrusted, so the workflow keeps it from every
secret:

- It triggers on `pull_request` only, which gives fork code no secrets and a
  read-only token. `pull_request_target` and `workflow_run` would run with the
  base repository's secrets and a write token; the `lint` check fails any
  workflow that names either.
- `permissions: contents: read`, `persist-credentials: false`, no `secrets`, no
  `environment`, no Cachix action and no cache or artifact upload. Nix reads
  only `cache.nixos.org` with `accept-flake-config = false`, so a pull request's
  `flake.nix` cannot add a substituter or key.
- Actions are pinned by commit, as in the other workflows.
- Whether a first-time contributor's run waits for a maintainer's approval is a
  repository setting (Settings, Actions, General), not the workflow's.

Run the same checks locally before opening a pull request: the `nix build` and
`nix eval` commands in the workflow, and `git diff --check edge`.

## Optional binary cache

`eon-yazelix` is a public Cachix cache donated and managed by
[bioinformatist](https://github.com/bioinformatist), separate from Lucca's cache
storage. It speeds up the Nix path and is neither a release channel nor a
requirement for source builds: a cache miss builds from the pinned sources.
Consumers choose whether to trust this third-party binary provider.

### What the cache holds

Two GitHub Actions workflows in the public `Yazelix/eon` repository push to the
cache, with the cache owner's write token from the `cachix` environment:
[`cachix.yml`](../.github/workflows/cachix.yml) for Linux and
[`cachix-darwin.yml`](../.github/workflows/cachix-darwin.yml) for Apple Silicon.
Nothing else in the repository writes to it, and pull requests and forks cannot
publish.

The Linux workflow runs on every push to `edge`, every `v*` tag push and every
manual dispatch. It pushes the `x86_64-linux` packages `default`, `eonbase` and
`no-helix`, their shared `eon-rust` output, the `eon-tests` check output and
`helix-grammar-sources`, each with its runtime closure. The Darwin workflow runs
on `v*` tag pushes and manual dispatches, and uploads only while the repository
variable `EON_DARWIN_CACHE_PUSH` is `true`. It pushes the `aarch64-darwin`
packages `default` and `helix-grammar-sources` with their closures. Manual
dispatch works only from `edge` or a `v*` tag. The flake defines packages only
for the product targets in `components/eon-v5.json`, `x86_64-linux` and
`aarch64-darwin`; on `aarch64-linux` or an Intel Mac (`x86_64-darwin`) it has
no Eon package to build or fetch. Cachix also skips paths that `cache.nixos.org`
already serves (see [Publishing](#publishing)), so keep that cache configured
alongside this one.

Two workflow pins keep paths in the cache:

- `helix-grammar-sources-<helix rev>` keeps each Helix revision's grammar
  sources forever. Every Linux run writes it.
- `latest-release-<platform>` holds the default runtime of the newest
  `vX.Y.Z` release, one revision per pin. When a newer release takes the pin,
  the older release loses it. `latest-release-x86_64-linux` moves on every
  newest-release Linux run; `latest-release-aarch64-darwin` moves only while
  Darwin publishing is on.

The workflows leave any older per-tag Darwin pins,
`aarch64-darwin-<tag>-<root>`, to the cache owner. Paths outside the pins'
closures, including `edge` builds, releases older than the newest and whatever
the `eonbase` and `no-helix` packages and the test output do not share with the
pinned runtime, stay only until Cachix evicts them.
[Publishing](#publishing), [Darwin publishing](#darwin-publishing) and
[Release retention](#release-retention) give the details.

### Using the cache

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
`nixConfig` does not configure the parent flake.

### User-level `substituters`

A plain `substituters =` line replaces the list Nix has read so far, while
`extra-substituters` appends to it. Nix reads the system configuration
(`/etc/nix/nix.conf` and its includes) before the user's
`~/.config/nix/nix.conf`, so a plain `substituters =` in the user file drops
every cache the system configuration lists, including `eon-yazelix` when
`cachix use` or an administrator added it there. Nix reports nothing about the
dropped caches. The flake's `extra-substituters` still append on top, but only
when Nix accepts the flake's settings, through `accept-flake-config = true` or
the prompt. Otherwise Nix prints
`warning: ignoring untrusted flake configuration setting 'extra-substituters'`
and builds Eon from source.

How much of that list takes effect depends on trust:

- Single-user Nix, and a multi-user daemon that trusts the user
  (`nix store info` prints `Trusted: 1`), use the user's list and keys as they
  are.
- For an untrusted user (`Trusted: 0`), the daemon keeps only the entries of the
  user's list that its own `substituters` or `trusted-substituters` also name,
  and uses that shorter list for the user's commands. A plain `substituters =`
  without `eon-yazelix` therefore drops the cache even when the administrator
  configured it. A cache the daemon does not already name is dropped with
  `warning: ignoring untrusted substituter 'https://eon-yazelix.cachix.org', you are not a trusted user.`
  For these users the daemon also ignores user-level keys and checks
  signatures against its own `trusted-public-keys`.

Add the cache in the user file with the additive settings instead:

```ini
extra-substituters = https://eon-yazelix.cachix.org
extra-trusted-public-keys = eon-yazelix.cachix.org-1:kPIp7u0Gjoz0GNzmRqeaqs798vU2raSOUpK+3xi7oK0=
```

If you keep a plain `substituters =` on purpose, for example to drop another
cache, add `https://eon-yazelix.cachix.org` to that line and keep the
`extra-trusted-public-keys` line. As an untrusted user, these lines take effect
only when the daemon's configuration already names the cache and its key; if it
does not, ask an administrator (see the multi-user item above).

Run the checks as the user who installs Eon. Root and `sudo` read root's
configuration, which can list the cache while yours lacks it:

```sh
nix config show substituters
nix config show trusted-public-keys
nix store info
```

`nix config show` prints the client's settings from the configuration files,
without any flake's settings: the first command must list
`https://eon-yazelix.cachix.org` and the second its `eon-yazelix.cachix.org-1:`
key. It cannot show what the daemon keeps. On single-user Nix, or with
`Trusted: 1`, Nix uses that list. With `Trusted: 0`, the cache applies only if
the daemon's own configuration names it, which an administrator can confirm.
For a direct check, run `nix build --dry-run github:Yazelix/eon/main` (or the
reference you install) as the installing user before installing. Eon's own
paths under "will be fetched" mean the cache serves them. The same paths under
"will be built", or the `ignoring untrusted substituter` warning, mean the
daemon does not use the cache or the cache lacks that revision.

### Publishing

Publishing uses the cache owner's per-cache write token, which a repository
maintainer stores as the `CACHIX_AUTH_TOKEN` secret of the `cachix` Actions
environment rather than as a repository secret. The environment's
deployment rules allow only the `edge` branch and `v*` tags, and both Cachix
workflows run their jobs in it, so a dispatch from any other branch fails before
a step runs and a branch-modified workflow never receives the token. While a
repository-level `CACHIX_AUTH_TOKEN` secret also exists, a modified workflow can
skip the environment and read it, so delete the repository secret once the
environment secret is set. Never put
the token in Git, an issue, or a pull request. Cache reads are public; the
credential check, upload and release-pin steps receive the token. A missing secret
fails before installing tools or building the package. To build the Apple Silicon
package from another branch without credentials, dispatch
`.github/workflows/darwin-build.yml`.

A push to `edge`, a `v*` tag push or a manual dispatch first runs the pinned
Helix grammar check.
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
job timeout. Runs on `edge` cancel superseded runs on the same ref; release-tag
runs share one concurrency group and queue without cancelling an active run.
GitHub may replace an older pending run in a group, so a missing release result
needs an explicit rerun. Frequency is
one run per `edge` push, `v*` tag push or explicit dispatch. It runs only in the
public upstream
repository: standard public-runner minutes are free and do not draw from the
organization's private-repository allowance. No larger runner,
Actions artifact, or Actions cache is used. GitHub paid runner spend is zero;
Cachix storage and retention remain with bioinformatist. Remove or revisit this
job if builds repeatedly exceed the timeout, the cache donation ends, or free
public-runner eligibility changes. Local `nix build .#default .#eonbase` remains
canonical.

To verify publication, check both exact product outputs' `.narinfo` records at
the cache URL. Check that `helix-grammar-sources-<helix rev>` points to the source
aggregate from that revision, with permanent retention. On a run for the newest
`vX.Y.Z` tag, check that `latest-release-x86_64-linux` points to that revision's
default runtime with one retained revision. Inspect a catalog source output's
`.narinfo` at the cache serving it. For a source Cachix skipped, check `cache.nixos.org` instead. Realise the products and
source aggregate in a fresh Nix store with both caches and signing keys
configured, signatures required and local builds disabled. A local build alone does not prove upload, retention
or fresh-store source substitution.

### Darwin publishing

`.github/workflows/cachix-darwin.yml` builds on `v*` tag pushes and manual
dispatches only. It builds `packages.aarch64-darwin.default` and
`packages.aarch64-darwin.helix-grammar-sources`, including disabled grammar
sources. Logs and the job summary report the exact revision, roots, individual
closure sizes and deduplicated combined closure NAR bytes. These uncompressed
sizes do not measure compressed incremental Cachix storage.

Publishing is off unless the repository variable `EON_DARWIN_CACHE_PUSH` is
`true`, which enables uploads to `eon-yazelix`. A maintainer sets it only with
the cache owner's consent on storage and capacity
([issue #8](https://github.com/Yazelix/eon/issues/8)). The
workflow runs in the `cachix` environment and uses its `CACHIX_AUTH_TOKEN`
secret only in gated credential, upload and pin steps; enabled publishing fails
early if that secret is absent. With the variable absent or set to `false`, a
dispatch from `edge` or a `v*` tag still builds both outputs and reports closure
sizes without uploading or pinning anything. A dispatch from any other branch
fails at the environment check; build-only runs on other branches use
`.github/workflows/darwin-build.yml`.

Enabled runs push both closures. Only runs selecting the newest `vX.Y.Z` tag,
including manual dispatches on it, update `latest-release-aarch64-darwin` to the
default runtime with `cachix pin --keep-revisions 1`. The only branch whose
dispatches can upload is `edge`, and its uploads are left to eviction.
Cachix's action has automatic pushing disabled in every mode.

Each run uses one standard `macos-15` Apple Silicon runner, restricted to the
public upstream repository, with a 120-minute timeout. Release-tag runs share
one concurrency group, other runs share one per ref, and no run cancels an
active one. GitHub may replace an older pending run in a group, so a
missing release result needs an explicit rerun. There are no edge-push or
pull-request triggers, paid runners, Actions artifacts or Actions caches.
Additional paid spend is $0. Local Nix builds remain available independently.

### Release retention

After a successful upload, a run for the newest `vX.Y.Z` tag updates one stable pin
per platform, `latest-release-x86_64-linux` or `latest-release-aarch64-darwin`,
to its default runtime closure. Each pin keeps one revision, without a timed
expiry. Both workflows use
[one shared pin script](../.github/scripts/pin-latest-release.sh). Grammar
sources keep their own `helix-grammar-sources-<helix rev>` pins, which every
Linux run writes (see [Publishing](#publishing)).
See [Cachix pins](https://docs.cachix.org/pins).

"Newest" means the greatest top-level release tag, `v` and three dot-separated
numbers, on the repository when the pin step checks, by Git's version sort.
Other `v*` tags, such as `v0.3.0-rc1` or `vendor-x`, never count and never pin.
The step checks once and then writes its one pin. A run for any other tag,
including a rerun of an older release after a newer one exists, uploads
normally, leaves the pin alone and logs one notice saying why.
Release-tag runs of one platform share a concurrency group and queue instead of
overlapping, so no two release runs pin at once. Other variants, including
`no-helix`, intermediate outputs and edge runtime builds are uploaded normally
but have no retention guarantee from these pins: `no-helix` shares most of its
closure, including `eon-rust`, with the pinned default, and a fresh install
builds any evicted remainder locally. Darwin publication still requires its
opt-in.

A failed build or upload never reaches the pin step. When Cachix rejects a pin
update, for example because a path is missing from the cache, the pin keeps its
previous revision. A pin step that fails after Cachix may have accepted the
update, such as on a lost response, leaves the outcome unknown. After any failed
pin step, check the platform's pin in the cache's pin list on Cachix: if it
names this run's runtime store path, this release is retained, whichever run
wrote it; otherwise rerun the job for the newest tag. There is no explicit
unpin, cache clearing, cross-platform wait or additional download-verification
job.

The workflows leave the per-tag Darwin pins, `aarch64-darwin-<tag>-<root>`,
alone. They hold storage until the cache owner checks their consumers and
decides to retire them. This policy does not assert current billed usage or
reclaim space immediately.

Historical cold-store mac-proof runs `42898289714` and `42342668048` took
49–53 minutes for the default package, building 620–624 derivations and
substituting about 630 paths (0.6–0.9 GiB downloaded, 6 GiB unpacked).
Budget about 55 minutes cold plus unmeasured grammar-source and upload time;
warm-cache savings are not guaranteed. GitHub documents `macos-15` as 3 M1
CPUs, 7 GB RAM and 14 GB SSD, and limits hosted jobs to six hours; standard
public-runner minutes are free. See the [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [Actions limits](https://docs.github.com/en/actions/reference/limits).
Build temporaries add disk pressure beyond closure sizes; the job reports disk
before and after building, including after failure.

Cachix advertises a free 5 GB open-source allowance with least-recently-used
eviction at capacity; the donated cache's actual plan, usage and room for
permanent release pins remain its owner's decision. See
[Cachix pricing](https://www.cachix.org/pricing). Revisit the job if disk or
timeout failures recur, public runners stop being free, the donation ends, or
the owner's capacity is insufficient. No paid expansion is authorized.

A build-only dispatch proves neither publication nor a compile-free install.
After authorized publication of the newest `vX.Y.Z` tag, verify
`latest-release-aarch64-darwin` and the exact output
`.narinfo` records, then realise the default package in a fresh Apple Silicon
Nix store with both caches and keys configured, signatures required and local
builds disabled. Record the release revision, run, paths and installing-user
result. Until that succeeds, Darwin substitution remains unproved.

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
