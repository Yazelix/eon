# Rust audit over-report evidence

These exact exceptions belong to the existing archive notice inventory.
The initial records were checked against the candidate produced from tree
`9dbb50a4171e760efd954c0175c0894b2ffcee98` (Linux x86_64, Eon 0.1.24).
`audit-exceptions.json` names each producer, root name and exact package version.
The checker binds these records to the normalized Cargo.lock, resolved runtime
and native package/feature sets, audited package sets, build flags, target and
pinned toolchain. Local package versions and binary bytes are excluded: ordinary
code and release-version changes preserve the dependency evidence. Dependency,
feature or build-context changes require explicit inspection and reviewed renewal;
unused exceptions and missing evidence anchors fail. The full native-binding
graph remains intact. See the checked-in [inspection and renewal recipe](../../eon/tools/archive-notices/README.md).
The binary SHA-256 values below identify observations, not acceptance keys.
Records before `eon-igae.15.6.1` (2026-11-19) cover two archive variants: the
full archive and a fallback without Accelerando, which that bead retired. The
archive now has one variant, so later renewals record one observation per
producer.
The historical `eon-c828u.13` observations below record the combined
renewal over reserved edge `8a19251d`, including the accepted `eon-tulof.4`
Venus features. Observations use source tree
`05ed9701cebcb6fc01b53683b094131cefaa4d4f`, including the directory-EOF review
fix. Earlier accepted observations remain labelled historical. The owning bead
records the final staged-tree gates and artifact identities after this update.

Each producer section gives the exact binary, source, vendor and actual build
flags from `nix log <producer-output>`. Its runtime graph uses
`cargo tree --locked --offline --target TARGET --manifest-path MANIFEST FLAGS
--package ROOT --edges normal,no-proc-macro --prefix none --format "{p}"`.
The normal/build graph uses the same command with `--edges normal,build`.
For multiple roots, repeat `--package ROOT` in the same command, preserving
feature unification. These reproduce the original notice producer commands.

To use the exact vendor outside the Nix sandbox, copy its `.cargo/config.toml`
to scratch, replace `@vendor@` and the quoted relative `cargo-vendor-dir` with
its absolute vendor path below, then pass `cargo --config SCRATCH/vendor.toml`.
No source, lockfile or features are changed.

For the **host** exceptions, the identities appear in the normal/build graph
but not the runtime graph: every selected path crosses a proc-macro or build
dependency boundary. `cargo tree` with the same arguments and
`--edges normal,build --invert NAME@VERSION` displays those paths. For the
**unselected** exceptions, the identities appear in neither graph and the
exact producer build log has no `Compiling NAME vVERSION` in its build phase.
The audit records conservatively retain disabled or unrelated dependencies.
No missing target runtime dependency is waived.

The independent observation is the real producer command and compilation log,
corroborated by the exact feature-selected dependency paths. Symbol namespaces
alone cannot distinguish package versions or standard-library code. As an
additional Helix check, `nm -C BIN` has no `steel_` namespace; Steel packages
are also absent from the actual compile log and selected graphs.

Full proof records, including the baseline-accepts/fixed-rejects dependency
omission, are owned by eon-igae.15.8. This records no new license decision.

### Rust 1.100.0 upgrade (eon-tfrjz)

For `eon-tfrjz` over `3028262e` (Rust `1.96.1` → `1.100.0`), all eleven
exception producers (`eon`, `orbit`, `venus`, `installer`, `anima`, `nushell`,
`starship`, `zoxide`, `atuin`, `helix`, and `yazi`) change only through
`context.toolchain.rust` in `nix/notices/inputs.nix`, which moved from
`/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` to
`/nix/store/10l4vk1qkvhqzp6xwh6gjbh1b1320w7n-rust-default-1.100.0`. No
`Cargo.lock` stanza, registry/git package, version, checksum, runtime package,
native package/feature, or `cargo-auditable` dependency set changes. `eon`,
`orbit`, and `venus` were rebuilt with Rust 1.100.0 (with `candidate-rune`
disabled and zero unselected exception compile lines in their build logs); the
eight non-workspace producers keep byte-identical binaries. Replacing
`context.toolchain.rust` in each producer's canonical `graph.json` with the
1.96.1 toolchain store path reproduces every accepted `3028262e` `graph_sha256`
byte for byte: `3fbd74f7` → `719abf85` for Eon, `3865499b` → `5d34d02c` for
Orbit, `d2b3ae73` → `aea9642c` for Venus, `064b1b27` → `361bb0c5` for
Installer, `e4d8399c` → `9981c2ea` for Anima, `ec9549bf` → `8133b268` for
Nushell, `de4e1b4f` → `51b7eca0` for Starship, `6bd0bb81` → `6759fae7` for
Zoxide, `9e08c1fc` → `0a9bbc55` for Atuin, `6d9874a0` → `2232f301` for Helix,
and `eda2a809` → `cb7a1427` for Yazi. For all twelve Cargo reports (including
`launcher`), `runtime-crates.txt`, `native-packages.txt`, `Cargo.lock`, and the
1.96.1-restored `graph.json` match the `3028262e` base (`eon-yp6vg.65`)
observations byte for byte. Every host-only and unselected classification
therefore carries over unchanged (Eon 9/2, Orbit 6/1, Venus 15/4).

Independent `inspect-audit` replays against the actual binaries reproduce every
saved `graph.json` byte for byte (all twelve Cargo reports, including
`launcher`). `prove-audit-omission` on Zoxide's observation accepts the
retained root under the root-only baseline and rejects omitted `anyhow 1.0.98`
with both the landed `3028262e` and renewed exception data. The strict
`propose-audit-renewal` proposal changes only the twenty-four `graph_sha256`
fingerprints across the eleven producers, and parsed, it equals the checked-in
file. The new contexts:

- Eon: `3fbd74f79d03931a7624eb62383b3a2ac7ecc206d29adf4e5bfba9a4056f7dc9`.
- Orbit: `3865499b6087726fd01c4c1d4d397ec85ae751d03a1428d34786eb388fd8884b`.
- Venus: `d2b3ae735959fe5f29c94b9a8d5712cdf1b83d6b1c29589686d4672bb349c3d1`.
- Installer: `064b1b27e0f2fde37411911c6c63d940f77b2a21d145a99fb13601aa8de29c95`.
- Anima: `e4d8399c97de0767c6a479479e1511f5bc4fc0568312f166c588bb830ce36387`.
- Nushell: `ec9549bf5630de2d91f591752fe60d96b8c90cd09dc25d6d6fda6c0f4e489ff3`.
- Starship: `de4e1b4fd3b4ae7c57208138f769bc1a553f8df1f96ec0fd689b2d713506de93`.
- Zoxide: `6bd0bb81256705d6fcbee6af33d7daea9edf44de625c9aae3b94820ebb856ce3`.
- Atuin: `9e08c1fc8120a85b37396429ff3122b36177d52af9bfe3c8c090c3c04027406d`.
- Helix: `6d9874a0e927acd202e5a7aade68d70b5cd952f7ad4e3b94683782b275f3c474`.
- Yazi: `eda2a8090d45061da82295aacd41b0a72b344f70bf42bb19bb38de92c05ae4b4`.
- Vendor at `/nix/store/dm83ykc5wadridaajbfq5nfkqx6ak914-cargo-vendor-dir`.
- Base input (`eon-yp6vg.65`) at `/nix/store/bh87dwl4zkm34kkrnfbj56j5xr0cvgxl-eon-archive-notice-input.json`.
- Proposal input at `/nix/store/nh4mxdga98ja0b2bxfkldyj21pbkwiri-eon-archive-notice-input.json` (before renewal).

### Wayland precision scroll pin (eon-yp6vg.65)

From Eon `03e7f81a`, the `dpi` and `winit` source records move
from `7c209ec5` to `c1dacc9d`. Reversing the winit revision in the
canonical graphs reproduces the landed Eon, Orbit, and Venus graphs
byte for byte. This includes the normalized lock and the winit
source identities in Venus's linked dependency and binary audit.
The producer feature selections and audit exception identities match
the landed base.

The exact Cargo trees and producer logs confirm the same classifications.
Eon has 9 host-only and 2 unselected exceptions, Orbit has 6 and 1,
and Venus has 15 and 4. The `inspect-audit` replays reproduce the
graphs and binary observations on both sides. The strict
`propose-audit-renewal` output changes six fingerprints. Zoxide's
dependency-omission probe still rejects removal of `anyhow 1.0.98`.

- Vendor at `/nix/store/ckj07fdk9h5s2wc2sm1d34n7np4rgl7l-cargo-vendor-dir`.
- Landed input at `/nix/store/4kwkgmnic25iqv0kj544jj952i4adxs9-eon-archive-notice-input.json`.
- Proposal input at `/nix/store/bh87dwl4zkm34kkrnfbj56j5xr0cvgxl-eon-archive-notice-input.json` (before renewal).

### Rune typed skip pin (eon-yp6vg.64)

Compared with Eon `5269709b`, the Eon, Orbit and Venus contexts change
through the normalized lock. The six package source records for Rune move
from `77cc200d` to `c22cc2d`. Reversing that replacement reproduces the
three preceding canonical graphs byte for byte.

The exact vendor trees and producer logs confirm the same exception
classifications: Eon 9 host and 2 unselected, Orbit 6 and 1, and Venus
15 and 4. Those producers build with `candidate-rune` disabled. The
`inspect-audit` replays reproduce the graphs and binary observations.
The strict `propose-audit-renewal` result changes six fingerprints.
Zoxide's dependency-omission probe rejects removal of `anyhow 1.0.98`
with the preceding and the renewed exception data.

- Vendor at `/nix/store/qar39p9zbzipvqzg7y0g9ccw7y1jl7a9-cargo-vendor-dir`.
- Proposal input at `/nix/store/yfz4d9nyjdma6jdb2ap5lgvyi5cp81bz-eon-archive-notice-input.json` (before renewal).

### Rune X11 gate (eon-5a9kx)

Compared with `bdcc26d6`, the Eon, Orbit and Venus audit contexts change
through the normalized workspace lock. Six Rune Git package stanzas move
from `52f6f6c9` to `77cc200d`. The lock drops `as-raw-xcb-connection 1.0.1`
and `x11-dl 2.21.0`, and removes dependency edges from `arboard`, `rune`,
`winit` and `x11rb`. Every existing registry version and checksum carried
forward matches the base.

The three `inspect-audit` replays reproduced the canonical graphs and actual
binary observations. Restoring the base normalized lock in each candidate
graph reproduces the base bytes. All producers keep the same runtime and
native Cargo trees and audit exception sets. The exact vendors and build
logs confirm the same host and unselected classifications on both sides
(Eon 9/2, Orbit 6/1, Venus 15/4). The archive producers leave
`candidate-rune` disabled.

The strict `propose-audit-renewal` output changes six graph fingerprints.
Every exception identity, reason and evidence anchor matches the base.
Zoxide's `prove-audit-omission` accepts the retained root with the root-only
baseline and rejects omission of the actual audited runtime dependency
`anyhow 1.0.98`.

- Eon: `cc33db7bce3523dac61613a891715935b958e6869f6f76452d7415fb1566a525`.
- Orbit: `2052ecbfedc0033a0d8442b8334fde5de29bd1bfbb20a7b67757322b6ae217ad`.
- Venus: `59d6fb26d4ede50795fd68c6757170fe457be4f1daa1cfe118338a16b9c87023`.
- Vendor at `/nix/store/cmwhc8m51rvfdnnlrpqkz5n5z35lnzbz-cargo-vendor-dir`.
- Proposal input at `/nix/store/2yxznhjnfwjcr25xag909msy9p21y7an-eon-archive-notice-input.json` (before renewal).

### Attention-wait pin (eon-yp6vg.7.9)

Historical (`eon-yp6vg.7.9`):

For eon-yp6vg.7.9 over `ffc97873`, the Eon, Orbit and Venus contexts change
through the normalized workspace lock. The `eon-rune` revision moves from
`caeb0472` to `52f6f6c9` in the source fields for `parley_engine`, `rune`,
`rune-motion`, `rune-staging`, `rune-taffy` and `tiny-skia-path`. Reversing
that substitution in each candidate graph reproduces the accepted graph
byte for byte. The archive producers use default features and leave
`candidate-rune` disabled.

Three `inspect-audit` replays against the actual binaries reproduce the saved
graphs and observations. The candidate and base runtime and native feature
trees match byte for byte, and their audited package records and missing sets
match. The exact-vendor Cargo tree replays confirm that the host exceptions
appear only in the normal/build tree. The unselected exceptions are absent
from both trees and from the base and candidate producer build-phase compile
lines. The counts remain Eon 9 host / 2 unselected, Orbit 6 / 1 and Venus 15 / 4.

The strict `propose-audit-renewal` result changes six `graph_sha256` fields and
preserves every exception identity, reason and evidence anchor. Zoxide's
`prove-audit-omission` accepts the retained root in the root-only baseline and
rejects the omission of audited runtime dependency `anyhow 1.0.98` with both
the preceding and renewed exception data.

- Vendor at `/nix/store/jha1vj3q936amz2xz7svdhbm3grplg9h-cargo-vendor-dir`.
- Proposal input at `/nix/store/0n6nqy1j5j9qla957bznpak4v553gv1x-eon-archive-notice-input.json` (candidate tree `472b7388`, before this renewal).

Historical (`eon-eon-agent-lr8sx.40.5`):

For eon-eon-agent-lr8sx.40.5 over `027f86d8`, the Eon, Orbit and Venus
contexts change through one local package. The bead moves Boat out of
`rota-cli` into the new unpublished local crate `eon-boat`, whose lock stanza
lists dependencies that were already pinned: `fs2`, `libc`, `rota-cli`,
`rustix 1.1.4`, `serde`, `serde_json`, `sha2`, `tempfile` and
`toml 0.8.23`. `eon` depends on it on Linux, so in the Eon context
`eon-boat` also joins the runtime set, the native package list and the
producer's audit records as one local package. Orbit and Venus gain only the
lock stanza; their binaries are byte for byte the accepted ones. No registry
crate, version, checksum or feature changes. Removing `eon-boat` from each new
graph (its lock stanza and `eon`'s dependency line, and in Eon its runtime,
native and audit entries) and re-serializing reproduces the accepted hash byte
for byte: `91ec5905` → `4c690223` for Eon, `368da961` → `613ab539` for Orbit,
and `1e960751` → `2c6597da` for Venus. Registry identities, native features,
host-only audited packages and build contexts are therefore identical to the
accepted graphs, and the host-only and unselected classifications carry over.

Three `inspect-audit` replays against the actual binaries reproduce the saved
graphs. The strict proposal changes only the six fingerprints, and parsed, it
equals the checked-in file. The new contexts:

- Eon: `91ec5905fa2ac006539872d8a56afa5e2c6935d2665389b64cae3166197876e7`.
- Orbit: `368da961dae3792455876c44df2000c58baffb6f51d5b6babb8d674085a2ffe6`.
- Venus: `1e96075187bb8d307bae06d72614839399272f31a471c480f6514412a26bc249`.
- Vendor: `/nix/store/75aa166h0c5wsnxs0ivkdkpzrpk724ar-cargo-vendor-dir`.
- Proposal input: `/nix/store/9w0mk4rm254xb002p0baxnm33rc9kxgp-eon-archive-notice-input.json`.

Historical (`eon-yp6vg.7.6`):

For eon-yp6vg.7.6 over `f592d1ba`, the Eon, Orbit and Venus contexts change
through the lock alone. The bead adds Venus's optional `rune` dependency
behind the non-default `candidate-rune` feature, which no producer enables.
The normalized lock gains 83 stanzas (390 → 473). 14 existing stanzas gain
dependency-list entries and change nothing else: arboard, codespan-reporting,
cosmic-text, fontconfig-parser, gpu-allocator, objc2-core-foundation,
objc2-io-surface, objc2-metal, sha2, wgpu, wgpu-hal, winit, x11rb and
yazelix-venus. Every added entry names a locked package. No stanza is removed,
and no version or checksum changes.

Each producer's canonical `graph.json` differs from the accepted one only in
`lock`. Putting the parent's normalized lock back into the candidate graph and
re-serializing it reproduces the accepted hash byte for byte: `4c690223` →
`423b5016` for Eon, `613ab539` → `212a651f` for Orbit, and `2c6597da` →
`3f5a96c0` for Venus. Runtime identities, native features, audited packages
and build contexts are therefore identical to the accepted graphs, and the
host-only and unselected classifications carry over (Eon 9/2, Orbit 6/1,
Venus 15/4). The Orbit and Venus binaries are byte-identical to the parent's.

Three `inspect-audit` replays against the actual binaries reproduce the saved
graphs. The strict proposal changes only the six fingerprints, and parsed, it
equals the checked-in file. The new contexts:

- Eon: `4c6902230e5d19585cd20dd31d70d9c10e600fbb86d04dab358257a8da8608c2`.
- Orbit: `613ab53969dbd29e983713f8db80a4825bbbe2e5c98cafafa4e9ae7fec8a0e5a`.
- Venus: `2c6597da6a1f6a6efb0a5e3df75ea7d0a40b7d4d9d730e4d8e36d346fee1cea7`.
- Vendor: `/nix/store/5yq1h5qm76ya33z4b077l5vbgil2c96s-cargo-vendor-dir`.
- Proposal input: `/nix/store/gby2jn9qamfpq3f917qpy0z6zs2lwk9z-eon-archive-notice-input.json`.

Historical (`eon-2r02l.2`):

For eon-2r02l.2 over `2f6857b2`, the Eon, Orbit and Venus contexts change
through the lock alone. The bead adds the local repository tool `agent-jail`,
whose lock stanza lists five dependencies that were already pinned: `rustix 1.1.4`,
`serde`, `serde_json`, `sha2` and `toml 1.1.4+spec-1.1.0`. No producer uses
`agent-jail`, and no registry crate, version, checksum or feature changes; the
installer and launcher producers have their own locks and keep their graphs, as
do the other Rust producers. Removing that one stanza from each new graph's
`lock` and re-serializing reproduces the accepted hash byte for byte:
`423b5016` → `c8d2d618` for Eon, `212a651f` → `3241bb55` for Orbit, and
`3f5a96c0` → `fe464495` for Venus. Runtime identities, native features,
audited packages and build contexts are therefore identical to the accepted
graphs, and the host-only and unselected classifications carry over (Eon 9/2,
Orbit 6/1, Venus 15/4).

Three `inspect-audit` replays against the actual binaries reproduce the saved
graphs. The strict proposal changes only the six fingerprints, and parsed, it
equals the checked-in file. The new contexts:

- Eon: `423b5016bbcb628ec72491c3481d3e04d8eb293ad205eb0371edd72981221bd8`.
- Orbit: `212a651f920d55b4176c199f9dcf4c134d08a7b32b349d070bc92298d08c9884`.
- Venus: `3f5a96c03d5800a4cde1af9e40fe26ecbd3914ef9ec1e5f069041ee1d9c43b0a`.
- Vendor: `/nix/store/wkh4giji17bk477ia51axj9w0p395gg5-cargo-vendor-dir`.
- Proposal input: `/nix/store/2sapppwd801ij95n0qwybry3qdl0l0xa-eon-archive-notice-input.json`.

Historical (`eon-2r02l.1`):

For eon-2r02l.1 over `d8386d4f`, the Eon, Orbit and Venus contexts change
through the lock alone. The bead adds the local repository tool `agent-bench`,
whose lock stanza lists five dependencies that were already pinned: `rustix 1.1.4`,
`serde`, `serde_json`, `sha2` and `tempfile`. No producer uses `agent-bench`, and
no registry crate, version, checksum or feature changes; the installer and
launcher producers have their own locks and keep their graphs, as do the
other Rust producers. Removing that one stanza from each new graph's `lock`
and re-serializing reproduces the accepted hash byte for byte:
`c8d2d618` → `8edb0d3b` for Eon, `3241bb55` → `dd3ec666` for Orbit, and
`fe464495` → `9ae02fe9` for Venus. Runtime identities, native features,
audited packages and build contexts are therefore identical to the accepted
graphs, and the host-only and unselected classifications carry over (Eon 9/2,
Orbit 6/1, Venus 15/4).

Three `inspect-audit` replays against the actual binaries reproduce the saved
graphs. The strict proposal changes only the six fingerprints, and parsed, it
equals the checked-in file. The new contexts:

- Eon: `c8d2d61894eeab90ed50b748eb194ec89b8742665557580fb5002f9485a5be7a`.
- Orbit: `3241bb55785821a278d320643d9b6b00e0484d1e7378c635d8c75d7fe8d26def`.
- Venus: `fe464495133ef4c7bf903417c4efd8a5d0e30f8a32a8829811e59ec21ee1107e`.
- Vendor: `/nix/store/xm177f9k6wx7w14cvk3vp853z99pam8d-cargo-vendor-dir`.
- Proposal input: `/nix/store/78izn7vglpz5ydh72pcyjdb9nxgijz2l-eon-archive-notice-input.json`.

Historical (`eon-yp6vg.7.13`):

For eon-yp6vg.7.13 over `72982b81`, the shared workspace lock selects
smallvec 1.16.2 and objc2 0.6.5. In both Linux variants, Eon and Orbit differ
only in the normalized lock. Venus also selects the new smallvec version in
the runtime, native and audited graphs. All other Rust producer graphs
match the parent. The parent includes the eon-kx9uh renewal for the
earlier review-tier lock edit.

The six exact-vendor tree and actual-ELF replays match the saved producer
observations. The pinned build logs and the selected trees preserve every
existing exception's classification (Eon 9 host / 2 unselected, Orbit 6 / 1,
Venus 15 / 4). The strict proposal renews six fingerprints and preserves
every exception identity, reason and evidence anchor.

These pre-renewal observations use vendor `/nix/store/ny3wxal34pixr94p3y2bmk5iwy72rbyb-cargo-vendor-dir` and the following
notice inputs. The bead records the final package and archive gates.

- Full: `/nix/store/ppfl0c6wqls4faji73si42ch143mqhbd-eon-archive-notice-input.json`.
- Without Accelerando: `/nix/store/ny53i9nlhkz8s02q2yhjrq2vyxjqp61j-eon-archive-notice-input.json`.

Historical (`eon-kx9uh`):

For eon-kx9uh, the Eon, Orbit and Venus contexts changed through the lock
alone. eon-3n73k.1 (`89dc5264`) gave the local `review-tier` package two
dependencies that were already pinned, `serde_json` and `sha2`. The
canonical graph embeds the whole normalized workspace lock. So that edit
changes the context of every producer built from the workspace lock (Eon,
Orbit and Venus), even though none of them uses `review-tier`. The installer
and launcher producers have their own locks and are unchanged. Reports from
`4ee5ce34` match the accepted Orbit and Venus contexts, and reports from
`89dc5264` do not. Their parsed graphs differ only in `lock`. Eon was not
built at either revision; the reconstruction below alone covers it. Removing just those two dependency lines from each new graph
reproduces the accepted hash byte for byte:
`843c43c7` → `deffe99d` for Eon, `8af38403` → `f31033ee` for Orbit, and
`21358fd8` → `e80f9935` for Venus. Runtime identities, native features,
audited packages and build contexts are therefore identical to the accepted
graphs. Selection is unchanged, so the host-only and unselected
classifications carry over (Eon 9/2, Orbit 6/1, Venus 15/4).

Both variants resolve to equal contexts per producer. Six `inspect-audit`
replays reproduce the saved graphs. The strict proposal changes only the six
fingerprints, and parsed, it equals the checked-in file. The new contexts:

- Eon: `843c43c7a3b7e8a9923ec880b341586b00d15b8dcfc609d60fe3757183e088af`.
- Orbit: `8af38403080d1b929c3f8ce6f53b8ff1036935011ee13317e56ff992c3a5c443`.
- Venus: `21358fd89d42ea77db8fc7629f268adb47f2bdea330f17f24b0704cf0f6f07f1`.
- Vendor: `/nix/store/bp6nxdfdcsbdi696v9fcw3md20h8afq7-cargo-vendor-dir`.
- Proposal inputs, from edge `22967582` before this renewal:
  full `/nix/store/zh857967bg51w2r7082fkk08gjf75xsy-eon-archive-notice-input.json`,
  without Accelerando `/nix/store/f3lkhvs2agiss991ddbsn571cxf3wkjg-eon-archive-notice-input.json`.

Historical (`eon-yp6vg.7.12`):

For eon-yp6vg.7.12, both release variants carry forward rota-cli's
`time` edge and update wgpu-core and wgpu-hal from 30.0.0 to 30.0.1.
Compared with edge `ca0f5780`, the Eon and Orbit contexts change only
in the shared lock. The Linux Venus runtime, native feature and audited
normal/build graphs differ only by those version substitutions. No
exception names either changed crate. Selection is unchanged for every
exception, so the host-only and unselected classifications carry over
(Eon 9/2, Orbit 6/1 and Venus 15/4). Six `inspect-audit` replays reproduced
the saved graphs. The strict proposal changes six fingerprints and
preserves every exception identity, reason and anchor.

The three renewals below share the vendor directory
`/nix/store/m1dbijzwgxcgl927npimixbcfra0wzgk-cargo-vendor-dir`.
They use these notice inputs.

- Full: `/nix/store/awrqbx2q7a0qhbh7dizqgbkw1vih111k-eon-archive-notice-input.json`.
- Without Accelerando: `/nix/store/9xj2wyh1gk098j66fyfnaz0ryv1xvlfk-eon-archive-notice-input.json`.

The lead records the final notice and source checks in the bead at landing.

Historical (`eon-eon-agent-lr8sx.40.3`):

For `eon-eon-agent-lr8sx.40.3`, the normalized workspace lock adds the
existing `time` package to `rota-cli`. Before and after that change, the
locked x86_64-linux Cargo trees for Eon, Orbit and Venus select the same
runtime and normal/build package and feature sets. The candidate fingerprints
substitute that one lock edge into the accepted canonical graphs under
`eon`, `orbit` and `venus` below. Every exception identity, reason and anchor
is retained. The owning bead records the projection and the lead's
independent inspection of both release candidates at landing.

Historical (`eon-c828u.7`):

For `eon-c828u.7` dependency prep, observed on staged tree
`ed395078dd7e2cd8bee29aa91a5e0e276380fff0` over `636d8ea8`, the only
Cargo.lock changes add already pinned rustix 1.1.4 to Eon and eon-bench.
Eon production selects event/fs/process/std/termios, development adds net/pty;
eon-bench selects event/fs/net/param/process/pty/std/system/termios/thread/time.
No registry/git package, version or checksum changes. All six local-producer
canonical graphs differ from the preceding accepted observations only in
`lock`: runtime identities, selected native features, audited packages and
build contexts remain identical. The graph includes the workspace lock even
when the changed package is not part of that producer.

Both realized full and fallback notice inputs were inspected
independently. Six `inspect-audit` runs reproduce the saved canonical graphs
byte for byte. Each exact-vendor offline runtime and normal/build feature tree
matches the saved tree after translating only its source prefix to
`/build/source`. Every host-only exception remains selected in normal/build
and absent from runtime; every unselected exception is absent from both trees
and has no compile line in the actual producer build log. The same Eon 9/2,
Orbit 6/1 and Venus 15/4 host/unselected identities remain. Log absence alone
is insufficient when dependencies are cached; selected-tree membership also
confirms the classifications.

All 12 Rust producers have equal full/fallback context hashes and
missing sets. The strict `propose-audit-renewal` command changes only six
Eon/Orbit/Venus fingerprints; every exception identity, reason and evidence
anchor is unchanged. The parsed checked-in JSON equals that proposal. No
target-runtime omission or new license decision is waived. Final gates on
the committed candidate are recorded in the owning bead; the observations
below bind dependency evidence, not acceptance of a report's own verdict.

Historical (`eon-c828u.13`):

For `eon-c828u.13`, compared with accepted edge `8a19251d`, the only lock
change is rota-cli's dependency on the already pinned rustix 1.1.4. Eon's
canonical graph changes only in `lock` and `native`: rustix gains `param`,
`rand` and `use-libc-auxv`. Orbit and Venus change only in `lock`; Venus keeps
the accepted `eon-tulof.4` native `param` feature. Runtime identities, audited
package sets and build contexts are unchanged for all three. No registry/git
package or version changes, license decision or target-runtime waiver occurs.

Both realized notice inputs were inspected independently. `inspect-audit` on
all six local-producer observations reproduces each saved graph byte for byte.
Their exact-vendor offline runtime and normal/build feature trees match the
saved trees after translating only the source prefix to `/build/source`.
Every host exception remains selected in normal/build and absent from runtime.
Every unselected exception remains absent from both trees and has no compile
line in the actual producer log. Cached Cargo artifacts make log absence alone
insufficient; the exact selected trees establish non-selection.

All 12 Rust producers have equal full/fallback context hashes and
missing sets. The strict Rust `propose-audit-renewal` command accepts both
inputs and changes only the six Eon/Orbit/Venus fingerprint fields. Every
exception identity, reason and evidence anchor stays unchanged. The parsed
checked-in JSON exactly equals that proposal. The owning proof records the
final `archive-source-check` and fallback release results and final
graphs. No release signing or full-route Accelerando grant decision occurs.

Historical (2026-11-17, `eon-eon-agent-lr8sx.40.1`):

The three `eon-eon-agent-lr8sx.40.1` contexts differ from the accepted
2026-11-16 contexts only
by two local-package changes in normalized Cargo.lock, compared field by field
with the accepted observations named below. Commit
`c3d73feea3d0efa9d39a570756e2322d1541361f` adds the local `review-tier`
stanza; `review-tier` is repository tooling in no producer's runtime, native
or audited set. `eon-eon-agent-lr8sx.40.1` renames the local package
`eon-agent` to `rota-cli` with the same directory, version, licence
(MIT OR Apache-2.0, unchanged LICENSE-MIT and LICENSE-APACHE) and dependency
list. Orbit and Venus differ only in that lock text. Eon's runtime, native and
audited sets differ only by that package name: the same local crate appears as
`rota-cli` where it appeared as `eon-agent`. Context fields (target, roots,
features, flags, toolchain) are unchanged. No registry/git crate is added,
removed or version-changed, and no feature changes.

Both variants were built from that tree. All 12 Rust producers have equal
full/fallback contexts; only Eon, Orbit and Venus rejected their old keys.
`inspect-audit` on each actual binary reproduces every saved graph. The strict
proposal changes only the six fingerprint fields and exactly equals the edited
JSON after canonical comparison; no exception identity changes. Because the
native normal/build feature data is identical apart from the name, each host
and unselected classification carries over unchanged. The omission falsifier
on Eon's `fs2 0.4.3` and Venus's `libc 0.2.189`, independently in both
variants, retains each root and rejects the missing audited runtime dependency
under the renewed keys.

Historical (2026-11-16):

The current three contexts differ from the accepted 2026-11-13 contexts only
in normalized Cargo.lock. Commit `4200c940c29fade5d2be955b285045c1022006f5`
adds `wayland-client` and `wayland-protocols-wlr` edges to benchmark-only
`eon-bench`; it changes no shipped runtime/native package, feature or audit
identity. The compiler and nixpkgs identities are unchanged. The raw runtime
trees also show local Eon 0.1.25 -> 0.1.27; both manifests declare Apache-2.0
and their LICENSE bytes match, so copyleft/source obligations are unchanged.
No registry/git runtime crate is added, removed or version-changed.
All eight local runtime crates also retain their declared licence/licence-file
fields and existing Cargo-about clarifications between accepted landing
`2a63b11c` and base `661fe604`; all six LICENSE files are byte-identical.
`eon-agent` remains MIT OR Apache-2.0; the other local runtime crates retain
Apache-2.0 through their declarations or existing clarifications.

Independent inspection of both variants reproduces every saved actual-binary
graph. Exact-vendor offline runtime and normal/build feature trees reproduce
the producer trees after translating only the local `/build/source` prefix.
All 12 Rust producers have equal full/fallback contexts and missing sets;
the three renewed hashes are recorded in their sections below. The existing
strict proposal changes only these six fingerprint fields and exactly equals
the edited JSON after canonical comparison; no exception identity changes.

The producer logs retain the exact flags below and no unselected exception's
compile line. These builds reuse `/nix/store/7hfm2gci8h6nik3l9jd3y6xn65h36qyg-eon-workspace-deps-0`;
log absence alone cannot prove non-selection of cached code. The exact
normal/build feature trees establish that those packages are unselected.
The existing omission falsifier on Eon's `fs2 0.4.3` and Venus's
`libc 0.2.189`, independently in both variants, retains each root and rejects
the missing audited runtime dependency under the renewed context.

The report paths below identify the observations used to justify renewal;
their `report.json` may still record the preceding stale-key rejection.
Their audit graphs and actual binaries were inspected directly. The owning
bead records passing reports generated with the accepted keys on the final
tree; evidence does not depend on a report file claiming its own acceptance.

## eon

Renewed on 2026-11-23 for `eon-tfrjz` to context
`3fbd74f79d03931a7624eb62383b3a2ac7ecc206d29adf4e5bfba9a4056f7dc9`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`719abf85…` byte for byte.

- Source at `/nix/store/qw9wq0f9f4ilkfjfsp4xcvv144d61f9p-source` (candidate tree over edge `3028262e`, before this renewal's own edits).
- Observation at `/nix/store/qzb6md064kq4pyji5ddxppadvnmp04ww-eon-cargo-notices-0.1.28`.
- Binary at `/nix/store/ky11qw9313shcw6y6q3v0gd7w16rb04f-eon-rust-0.1.28/bin/eon`
  with SHA-256 `3f3ef683b853ea6b327d00df10872347073b355dcc9654b5a468c3c15f52667f`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-yp6vg.65`):

Renewed for `eon-yp6vg.65` to context
`719abf85530c7a58b71ffe3bc31e4e2ff9e38a672781d14f59925a1cbf9edc3f`.
Reversing the winit revision reproduces `fcc65995…`.

- Source at `/nix/store/rjgyhv701j20byai1yhq5040sj1vwm3p-source`.
- Observation at `/nix/store/y580vy3fjm015dkmiylmh8kgk15i21n7-eon-cargo-notices-0.1.28`.
- Binary at `/nix/store/71n7737p6dz4r638jqqa06n0dlnqi9jv-eon-rust-0.1.28/bin/eon`
  with SHA-256 `ccd6dbdbe99b21f0004c58cd8a87c6abb5668ae944a3c6c8cbf3a5970675626c`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-yp6vg.64`):

Renewed for `eon-yp6vg.64` to context
`fcc659959879bac1ea6617cd3b3e373882908d9219cf8609ce07611af1b65a08`.
Restoring the base normalized lock reproduces `cc33db7b…`.

- Source at `/nix/store/izivbjrb7da5mgywcrnjsmhnhvqsm55k-source`.
- Observation at `/nix/store/3gnai0kkyf8ff0ki289qmpbnndvh1c18-eon-cargo-notices-0.1.28`.
- Binary at `/nix/store/6m9g0lkvbf6zdpm61222vn7avf6p2s0p-eon-rust-0.1.28/bin/eon`
  with SHA-256 `46dd7c25bf5e53cc367d924548e6dbb8d2653bb4fdf4ebde6706d49bb9b1906d`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-5a9kx`):

Renewed for `eon-5a9kx` to context
`cc33db7bce3523dac61613a891715935b958e6869f6f76452d7415fb1566a525`.
Restoring the base normalized lock reproduces `e5bb007e…`.

- Source at `/nix/store/qxypiava4cfsyxanccys18ncy3vd76ww-source`.
- Observation at `/nix/store/52jigm5k44dyrmvq7zcsdjjw8q8zs2nv-eon-cargo-notices-0.1.28`.
- Binary at `/nix/store/rrjzp58hkchv9mlyvz2g6crrqpgfrly0-eon-rust-0.1.28/bin/eon`
  with SHA-256 `6f20bd9fbd4221d9518028092a475de798a7183cb6d4e1ee02508ea4d690e776`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-yp6vg.7.9`):

Renewed on 2026-11-22 for `eon-yp6vg.7.9` to context
`e5bb007e31c6737e0c60bd13b6273ed8bd053815091be70065dcd4f6413a8549`
in the single archive variant. Restoring the parent Rune revision in the
normalized lock reproduces `91ec5905…`.

- Source at `/nix/store/l33z3rvd7dcvif2qc99axlxajnzvjiw9-source`.
- Observation at `/nix/store/a8lm3c59bvzd1dyg7abwjaaisrvx1n10-eon-cargo-notices-0.1.28`.
- Binary at `/nix/store/7bvjg8nrgnzln835iaayg5zj5wfazh79-eon-rust-0.1.28/bin/eon`
  with SHA-256 `a35a25abe1681ecc38aa003fa4d13f5b86eb6822cfda62f99f37bace9898d317`.
- Observed build flags `-j 6 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-eon-agent-lr8sx.40.5`):

Renewed on 2026-11-20 for `eon-eon-agent-lr8sx.40.5` to context
`91ec5905fa2ac006539872d8a56afa5e2c6935d2665389b64cae3166197876e7`
in the single archive variant. The local crate `eon-boat` joins this context: its lock stanza, `eon`'s
dependency on it, and one local runtime, native and audit entry. Removing it reproduces
`4c690223…`.

- Source: `/nix/store/2h4hna9y3b5v6na3i6dw5qpdlnnm1jb2-source` (candidate tree over edge `027f86d8`, before this renewal's own edits).
- Observation: `/nix/store/kla6bcp40y7s5736air0hmr8p6pljgzi-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/82jawl6q96mrjcwgf7vwq46l2472navm-eon-rust-0.1.28/bin/eon`.
  SHA-256: `84e0a540f62e044d58075cbec2088b2a6722a7cd24383b3bfa5c10a582410b1e`.

Historical (`eon-yp6vg.7.6`):

Renewed on 2026-11-20 for `eon-yp6vg.7.6` to context
`4c6902230e5d19585cd20dd31d70d9c10e600fbb86d04dab358257a8da8608c2`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains Venus's optional `rune` graph (83 stanzas plus
dependency-list entries in 14). Restoring the parent lock reproduces
`423b5016…`.

- Source: `/nix/store/sf769wlshz54i0l6qmifxflplxyig2dy-source` (candidate tree over edge `f592d1ba`, before this renewal's own edits).
- Observation: `/nix/store/p2jr7wdpp1gcjwrlnf9d16fn4a5zilx4-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/qalff9h2wys2cxk195m8874i2n29g9ld-eon-rust-0.1.28/bin/eon`.
  SHA-256: `1b264cb891f9e8c61cf423b88fa35951f953efeb1b9b279dc5813dd5ead7cc25`.

Historical (`eon-2r02l.2`):

Renewed on 2026-11-20 for `eon-2r02l.2` to context
`423b5016bbcb628ec72491c3481d3e04d8eb293ad205eb0371edd72981221bd8`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-jail` stanza. Removing it reproduces
`c8d2d618…`.

- Source: `/nix/store/c31ymp7kwxqsh1qx030dx4jr4yr1nszv-source` (staged tree over edge `2f6857b2`).
- Observation: `/nix/store/r2al77lcwb031zgwhkqs9ch1vn3kqlfa-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/d78h0dnlcam4j1z7jvvbas42xkrmlnxg-eon-rust-0.1.28/bin/eon`.
  SHA-256: `1275f242278b11e573c218a5f249cf370601481851c99c09b32c543a66e66d11`.

Historical (`eon-2r02l.1`):

Renewed on 2026-11-19 for `eon-2r02l.1` to context
`c8d2d61894eeab90ed50b748eb194ec89b8742665557580fb5002f9485a5be7a`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-bench` stanza. Removing it reproduces
`8edb0d3b…`.

- Source: `/nix/store/i14lciqrdy4hll7r4fp7gsx20z1q19kv-source` (staged tree over edge `d8386d4f`).
- Observation: `/nix/store/l0ir2nzmbz0inhk5w4wxc48wnmy78m95-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/xdp6lz4q2nqjbnmkajh00fqba331c1il-eon-rust-0.1.28/bin/eon`.
  SHA-256: `f665507aa08f5ec9561f6cecb5a49ba4b85ce18f05d019b3f865cbaa333cb6ae`.

Historical (`eon-yp6vg.7.13`):

Renewed on 2026-11-19 for `eon-yp6vg.7.13` to context
`8edb0d3baf8191752824bd3ca58a29250017d18c8b94a146e0a6774f33c5f177`
in both variants. Only the normalized workspace lock changes in this context.

- Source: `/nix/store/7ij10akj8igf3738yj0hn0x3iyk40z0g-source`.
- Full observation: `/nix/store/b7as1kbf3n8x3qmk1d4l43v6prq1jlbg-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/gbdmpqcmsxhzz1ikw54q5r7qzddqfk2l-eon-rust-0.1.28/bin/eon`.
  SHA-256: `4972b5307f2a60a59a6f1537ae4e5f228cd17b1e2633051f364f0561e085f3f7`.
- Without Accelerando observation: `/nix/store/n7az9ygqyp6ldsap4lyj26jvcvns5g9h-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/bnvqnahz2bxqadj7l93yslqch2vx0pn7-eon-rust-0.1.28/bin/eon`.
  SHA-256: `f7dbbc424244bf007a514b207e3d18d6c56e5f0c1795c6f43d6189179a7cc699`.

Historical (`eon-kx9uh`):

Renewed on 2026-11-19 for `eon-kx9uh` to context
`843c43c7a3b7e8a9923ec880b341586b00d15b8dcfc609d60fe3757183e088af`
in both variants. Only the shared lock changes in this context: it gains
`review-tier`'s `serde_json` and `sha2` edges. Removing those two lines from
either observation's graph reproduces `deffe99d…`. Eon was not built at
`4ee5ce34` or `89dc5264`; that reconstruction alone covers it.

- Source: `/nix/store/zfz1zysaz9r1rz01awnkiasqhmnvi0yd-source` (edge `22967582`).
- Full observation: `/nix/store/q1xil6468cwaqqpvlzzyjk69hdhvpj96-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/35z1s0s6bl9mwb3dagmgc740ad2jflsz-eon-rust-0.1.28/bin/eon`.
  SHA-256: `c4941bb738de469caea8831aa04b2574f4b60a4b462587f25db3c88aef9206d4`.
- Fallback observation: `/nix/store/h2125vk48lmj99w45nqxa19szp5fssld-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/nbhmi57nf81x44wqc8sml8xd2sdn144g-eon-rust-0.1.28/bin/eon`.
  SHA-256: `6dd8b6ad6e7daf060cb1892ed7ca2b2fd881f91a5b5cedd07e2e46345dcb7bf2`.

Historical (`eon-yp6vg.7.12`):

Renewed on 2026-11-19 for `eon-yp6vg.7.12` to context
`deffe99de7f841cddb7b77a27d93a1265f09d005bfed5e3c56e07d136ffc8e8b`
in both variants. Only the shared lock changes in this context.

- Source: `/nix/store/9piq02nmsb8zv46z2g10v2ywx5qw9122-source`.
- Full observation: `/nix/store/bb0lxr7fi8qikm3ssn09hf7qr3lmzqkw-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/lki7n5917gwy60vxvnq3p8acjkx556fl-eon-rust-0.1.28/bin/eon`.
  SHA-256: `b27f3bb2b630ad867748f65dd5bf1b997ca243be3b11bcdab7180ba2f56938aa`.
- Fallback observation: `/nix/store/s9gcnixarvcxfzq3in4cikngip26ci9v-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/z3gzsxmqll5js55gc02rnifa813izjgs-eon-rust-0.1.28/bin/eon`.
  SHA-256: `464ee6fd35545647c40c9cb1bb0c4d25ee8f43bb257786846ef818e8e9481a9c`.

Historical (`eon-eon-agent-lr8sx.40.3`):

For `eon-eon-agent-lr8sx.40.3`, the projected context changes from
`dceca7e518774ddfcd72ebebc66f097e051ae7592277eb133623c2499971f1e0` to
`d7b0fde812702cc3b3acd7770a7c7af10266e18258e956e2c2088085d5407e62`.
The normalized lock adds rota-cli's `time` edge. Full and fallback
release artifact observations are due at landing.

Historical (`eon-c828u.7`):

Renewed on 2026-11-17 for `eon-c828u.7` dependency prep to context
`dceca7e518774ddfcd72ebebc66f097e051ae7592277eb133623c2499971f1e0`.
Full and fallback contexts are identical. Compared with the
preceding accepted context
`97795ee94ac8866b88673065a797788eecadeff175ff655fe922085712d227d2`,
only `lock` changes; the same 9 host-only and 2 unselected exceptions remain.

- Source: `/nix/store/8zg7fabiqq48pr6sryxixm7a6ylnj33n-source`.
- Vendor: `/nix/store/0p2h1fsr16lz6k70ha1q1c9grrman53d-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `eon/crates/eon/Cargo.toml`;
  root: `eon`; default features, no added root features.
- Full observation: `/nix/store/m1cvh8ciwl60vxgf0xwg6xg1adxchv9l-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/phfznwgh53iwiyp3m3f30xvvvf2hfkhp-eon-rust-0.1.28/bin/eon`.
  SHA-256: `49a1e482cd116e9b54bcd632d4ff2fabd42dc5fdc00cbc7c54b146b302683313`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Fallback observation: `/nix/store/zb28sn3wribpsvav4sakl46sxrkmvvsb-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/fqdlw17psys9g7kr7yxz9x9dvi380z2j-eon-rust-0.1.28/bin/eon`.
  SHA-256: `08d23ff77cd408c818c4fb32dffd579dd1a28003425c7e492f0e4cde9154d122`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (`eon-c828u.13`):

Renewed on 2026-11-17 for `eon-c828u.13` over `8a19251d` to context
`97795ee94ac8866b88673065a797788eecadeff175ff655fe922085712d227d2`.
Full and fallback contexts are identical. Compared with accepted
context `3c8036faf5a889ebb14b58572717640ad2d4ac35bcd47f1794290917ba93d36a`, only `lock` and `native` change.
The same 9 host-only and 2 unselected exceptions remain.

- Source: `/nix/store/a07vbl3xg464p71a5y55yw5cnm9lnhgh-source`.
- Vendor: `/nix/store/8xhpmv7shr362b3k6h23z7vds17x8fld-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `eon/crates/eon/Cargo.toml`;
  root: `eon`; default features, no added root features.
- Full observation: `/nix/store/vgjdhphwqsz6xp19dp06f5bc1aafm30g-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/i77081b2ihyli3xw5fa9fpmfrr00laj8-eon-rust-0.1.28/bin/eon`.
  SHA-256: `31c5bfc51a2591be67c6565eca0a18b1c3836eff25fdb17ac27034e6cbc4eb52`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Fallback observation: `/nix/store/y80629wnm2s02zds6kga1kh83wkjsd34-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/xaqadrqz841vw11qdigdkiac3ckf7xjj-eon-rust-0.1.28/bin/eon`.
  SHA-256: `5c73ff79a755b7816533c6c6299ab02edebd6e513bc3944f76cd5549652b4716`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (2026-11-17, `eon-eon-agent-lr8sx.40.1`):

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`3c8036faf5a889ebb14b58572717640ad2d4ac35bcd47f1794290917ba93d36a`.
The full and fallback values are identical. Compared with accepted
context `73ec261a2f561ac31bf0847bc31a743a1c88b4c725449eca555bbded7a86b454`,
only the `review-tier` lock stanza and the `eon-agent` -> `rota-cli` package
name differ. The same 9 host-only and 2 unselected exceptions remain; no
target-runtime dependency is waived.

- Source: `/nix/store/q7xf8r1ag5w8m4539w7jbsx3bp9m4r3b-source`.
- Vendor: `/nix/store/d5bys2iry7p84jnbaj30yhcl3xsj78y3-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `eon/crates/eon/Cargo.toml`;
  root: `eon`; default features, no added features.
- Full observation: `/nix/store/jlv208cw9v89wpwnxxwzyzxj6m2mp4mg-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/vmlrwbj18vbfzyxqmrk6yh8ihxqy2lss-eon-rust-0.1.28/bin/eon`.
  SHA-256: `1a3aa70d5fb0b42556fd317a19207051a6d804f604fc0774fe0eb20515cc869d`.
- Fallback observation: `/nix/store/djvw8z795za2bmrqf0jz770zlsf8pj99-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/p7ykvhjr06l4wpndnnahv0x6r0d124hp-eon-rust-0.1.28/bin/eon`.
  SHA-256: `724e7a4ee3d1689ef1ef64e26fc851f4525be477463590ad1de97d0f33ae8e21`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (2026-11-16):

Renewed on 2026-11-16 for `eon-igae.19` to context
`73ec261a2f561ac31bf0847bc31a743a1c88b4c725449eca555bbded7a86b454`.
The full and fallback values are identical. Compared with accepted
context `c70fb5048770f0f09e8ff5ed556388ba036120a596d2630ecc8614d2012726de`,
only the benchmark lock edges described above differ. The same 9 host-only
and 2 unselected exceptions remain; no target-runtime dependency is waived.

- Source: `/nix/store/f80fwj5b2ys0fpi2arsyxq9nx3m23z3c-source`.
- Vendor: `/nix/store/2315zg2zx3f5jd9n9886wa7pjp7fr3ir-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `eon/crates/eon/Cargo.toml`;
  root: `eon`; default features, no added features.
- Full observation: `/nix/store/hml9kk3mifhi71dnyrz20wm1dfxjjdnk-eon-cargo-notices-0.1.27`.
  Binary: `/nix/store/8miw0xrh7i78k1zihf93s6q7ds9gfg70-eon-rust-0.1.27/bin/eon`.
  SHA-256: `f3fec424938d96464fa233f1c1b1c5a2b2589d4bbcf4d91adf6918e6a18e264e`.
- Fallback observation: `/nix/store/nbybygfmcpw0a2m4gq5km08h1s0lgayz-eon-cargo-notices-0.1.27`.
  Binary: `/nix/store/7bqkl8q51ni21j915vryq8syxpdb4nlx-eon-rust-0.1.27/bin/eon`.
  SHA-256: `fa66151387d9938fabe8eb1b65b26f75d7ca217459f8ddc5c0aa28a418c782e0`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (2026-11-13):

Renewed on 2026-11-13 from staged tree
`85aa2da7f6c11a847fa6eb186d797c6a2c727088` (`eon-eon-agent-lr8sx.27`).
Full and fallback observations independently reproduce dependency
context `c70fb5048770f0f09e8ff5ed556388ba036120a596d2630ecc8614d2012726de`.
Each has 9 exact host exceptions and 2 exact unselected exceptions.
`equivalent 1.0.2`, `hashbrown 0.17.1` and `indexmap 2.14.0` are runtime-covered
through the moved agent crate, so their exceptions are removed. `libm 0.2.16`
is an unselected audit over-report: the audit records an edge from
`num-traits 0.2.19`, but the selected feature is `std`, not `libm`.
Neither `libm` nor the retained `foldhash 0.2.0` appears in either selected
graph or the actual producer compilation log. No target runtime dependency is
waived. The existing omission proof rejects removal of runtime `fs2 0.4.3`
while the root-only baseline accepts the retained root, in both variants.

- Source: `/nix/store/daqksjml0bxslvdzwk6d1am8lpiy57v8-source`.
- Vendor: `/nix/store/gm878n1hhyy3xzjjzijarrz1rah4sdcw-cargo-vendor-dir`.
- Manifest: `eon/crates/eon/Cargo.toml`; target `x86_64-unknown-linux-gnu`;
  root `eon`; actual flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Full report: `/nix/store/sixkp1x91qblz85cj03dj6wwdr3r3jgq-eon-cargo-notices-0.1.25`.
  Binary `/nix/store/gxgynrmkbhvrh6mdhz6phl67d1zy05rs-eon-rust-0.1.25/bin/eon`,
  SHA-256 `61e951ae006091f1fc4054e855006777a7408836166b41367519a23be1a0e0da`.
- Fallback report: `/nix/store/w0429hsbxkppys66vz0zkwfnsy57lxfm-eon-cargo-notices-0.1.25`.
  Binary `/nix/store/99prj4pd9gvlc5lf5k95yr2sgi3v4g2s-eon-rust-0.1.25/bin/eon`,
  SHA-256 `b34721930a531982ed44b3ec8b89bccae7db789dbb54a62f2f4592381b49058c`.

Actual ELF reinspection reproduces each saved canonical graph. Vendor-selected
runtime and normal/build feature trees reproduce each report after only local
source-path normalization. The Rust renewal proposal matches all 12 producer
sets, contexts and missing-identity sets across both variants. Full investigation
and red/green records are owned by `eon-eon-agent-lr8sx.27`.

Historical observations: both exact supported variants were checked from source tree
`279f0494fe622b1b0c82fa51baf6501840da5813`. Their complete audit records and
runtime/build identity sets matched the initial candidate. Each had 12 exact
host exceptions and one exact unselected exception, with no unselected
identity compiled during its build. Both variants shared dependency evidence and
therefore the same acceptance key. Each release route still checks its own actual
binaries, inventory and independent artwork obligations.

### full

- Source: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source/eon/crates/eon/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; root: `eon`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Binary: `/nix/store/23qvl1lxlhvsgfa4hyjb4xdh6ga8h3mz-eon-rust-0.1.24/bin/eon`.
- SHA-256: `f722aab1b5d14958a6d86e346176ac38f90250b92afeed87f70fcd5c6d5a3c6f`.

### fallback (retired)

- Source: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source/eon/crates/eon/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; root: `eon`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Binary: `/nix/store/zimld5w76sl24n1xvjkcslssq14hz41j-eon-rust-0.1.24/bin/eon`.
- SHA-256: `96ad86f70b561c7216da0bacefc0891e812b4c6103ec52a85f49f13a2726b990`.

## orbit

Renewed on 2026-11-23 for `eon-tfrjz` to context
`3865499b6087726fd01c4c1d4d397ec85ae751d03a1428d34786eb388fd8884b`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`5d34d02c…` byte for byte.

- Source at `/nix/store/mbmp96xvfh9m6g3zmd4g7zwv4nbxck83-source` (candidate tree over edge `3028262e`, before this renewal's own edits).
- Observation at `/nix/store/d6f8ik9i01hqghy35g726izyb0idjk0h-orbit-cargo-notices-0.1.0`.
- Binary at `/nix/store/xxbbl50xay8cknrqs31cn51m8x6xxm9x-yazelix-orbit-0.1.0/bin/yazelix-orbit`
  with SHA-256 `8f4872369f14873999719a76f77bc1653be2b44b3b195c0056a8014bc76ea05a`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-yp6vg.65`):

Renewed for `eon-yp6vg.65` to context
`5d34d02cd6996269ae52cffb3cf052f4beea24ea038a028b171c08bb6cdef2aa`.
Reversing the winit revision reproduces `85cfb652…`.

- Source at `/nix/store/lq4dhksi8rs6rmrw5wr3js4kb23k2fqs-source`.
- Observation at `/nix/store/s8mm0q7gz8skzra6hmjlrwf1z44ikhx2-orbit-cargo-notices-0.1.0`.
- Binary at `/nix/store/8si875gvh6hh2ad6m99y4i57100smmi6-yazelix-orbit-0.1.0/bin/yazelix-orbit`
  with SHA-256 `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-yp6vg.64`):

Renewed for `eon-yp6vg.64` to context
`85cfb652e1bdb17c8866849e0ea994f8f1582750422f5b20a9fb78943c7fe5c0`.
Restoring the base normalized lock reproduces `2052ecbf…`.

- Source at `/nix/store/8ai2rh1ysr15c4i92zsbnxlxiyh7z5kx-source`.
- Observation at `/nix/store/5ialgakris5wgdg5qjn3zpn61yzvrz5j-orbit-cargo-notices-0.1.0`.
- Binary at `/nix/store/g55gd6siss84nvir1zx3bwhdfj87bhvn-yazelix-orbit-0.1.0/bin/yazelix-orbit`
  with SHA-256 `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-5a9kx`):

Renewed for `eon-5a9kx` to context
`2052ecbfedc0033a0d8442b8334fde5de29bd1bfbb20a7b67757322b6ae217ad`.
Restoring the base normalized lock reproduces `c06ba4ab…`.

- Source at `/nix/store/bgcn84l6693glmvdap8iv5b7c3zmqis5-source`.
- Observation at `/nix/store/i5am9jkwdazjl9wznfazsviw5p7cjhav-orbit-cargo-notices-0.1.0`.
- Binary at `/nix/store/mz1j4bq931yz2xnhwc8jrk705g83h4kr-yazelix-orbit-0.1.0/bin/yazelix-orbit`
  with SHA-256 `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-yp6vg.7.9`):

Renewed on 2026-11-22 for `eon-yp6vg.7.9` to context
`c06ba4abdcb8266bd7877235e7b3323f100b0ec3f6df153dc324ad9f1d95880c`
in the single archive variant. Restoring the parent Rune revision in the
normalized lock reproduces `368da961…`.

- Source at `/nix/store/ijybhmvgpjjspxzxmlj1ynw65khmnypr-source`.
- Observation at `/nix/store/dfd3xka4vw08i640a234138awlia9lqm-orbit-cargo-notices-0.1.0`.
- Binary at `/nix/store/ag5xxhj2sh9avrdnq2mww6fjfqr1crfk-yazelix-orbit-0.1.0/bin/yazelix-orbit`
  with SHA-256 `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.
- Observed build flags `-j 6 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-eon-agent-lr8sx.40.5`):

Renewed on 2026-11-20 for `eon-eon-agent-lr8sx.40.5` to context
`368da961dae3792455876c44df2000c58baffb6f51d5b6babb8d674085a2ffe6`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `eon-boat` stanza; the binary is unchanged. Removing it reproduces
`613ab539…`.

- Source: `/nix/store/8670fyzkabhjs2c8m8inwi36m72kjr2m-source` (candidate tree over edge `027f86d8`, before this renewal's own edits).
- Observation: `/nix/store/3s89c3g0vy4ckxffblclyb304lxw6dxx-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/05ci5qv88gwj0mcrsfidkxxkfiamvd29-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-yp6vg.7.6`):

Renewed on 2026-11-20 for `eon-yp6vg.7.6` to context
`613ab53969dbd29e983713f8db80a4825bbbe2e5c98cafafa4e9ae7fec8a0e5a`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains Venus's optional `rune` graph (83 stanzas plus
dependency-list entries in 14). Restoring the parent lock reproduces
`212a651f…`.

- Source: `/nix/store/sf769wlshz54i0l6qmifxflplxyig2dy-source` (candidate tree over edge `f592d1ba`, before this renewal's own edits).
- Observation: `/nix/store/0mhn8jmy6r9vhdh5apmqi1sqyp9z34ip-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/iyn09gcm1mpqhxydmpdg7zgx0ay0z5ba-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-2r02l.2`):

Renewed on 2026-11-20 for `eon-2r02l.2` to context
`212a651f920d55b4176c199f9dcf4c134d08a7b32b349d070bc92298d08c9884`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-jail` stanza. Removing it reproduces
`3241bb55…`.

- Source: `/nix/store/hyq746d49yv1m9clcndmia9z31s10da5-source` (staged tree over edge `2f6857b2`).
- Observation: `/nix/store/6g4qz907fdzrvzfkpcmkmi839mwvv7g7-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/cm7zv85h5x0nd983pv00af17g8kqf8h6-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-2r02l.1`):

Renewed on 2026-11-19 for `eon-2r02l.1` to context
`3241bb55785821a278d320643d9b6b00e0484d1e7378c635d8c75d7fe8d26def`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-bench` stanza. Removing it reproduces
`dd3ec666…`.

- Source: `/nix/store/a7j35j4f1pm405gyjl4nl7w803y2r2cx-source` (staged tree over edge `d8386d4f`).
- Observation: `/nix/store/gwf6w8kgj33zd0zg0c2g2lagyh5z9k9s-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/v5yqxpsgq6rpyfyl5jzv5rx259jhsiym-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-yp6vg.7.13`):

Renewed on 2026-11-19 for `eon-yp6vg.7.13` to context
`dd3ec6664114f69250ad22d85b3459a2093a786efa762328705928a5d2af757b`
in both variants. Only the normalized workspace lock changes in this context.

- Source: `/nix/store/5731kkk7786cybwwx1118df6x6k3jw17-source`.
- Both observations: `/nix/store/lxs2r9q8abnw2sw7mb8xq4qnlaah4bnl-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/fllk3igz8md5p8kcibipbr1m7zv6nq3w-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-kx9uh`):

Renewed on 2026-11-19 for `eon-kx9uh` to context
`8af38403080d1b929c3f8ce6f53b8ff1036935011ee13317e56ff992c3a5c443`
in both variants. Only the shared lock changes in this context: it gains
`review-tier`'s `serde_json` and `sha2` edges. Removing those two lines
reproduces `f31033ee…`. The `4ee5ce34` report
`/nix/store/81n6f6xv21nfr3684xprc013i2qk7kr4-orbit-cargo-notices-0.1.0`
holds the accepted context. The `89dc5264` report
`/nix/store/4fr65rnd2m9bb0c18dr2l3mjpjir7hi3-orbit-cargo-notices-0.1.0`
holds this one. Their parsed graphs differ only in `lock`.

- Source: `/nix/store/sgqqw1hnrhr4f0gada6xhd0sb8i9v7m5-source` (edge `22967582`).
- Both observations: `/nix/store/n70skjk229r00bnx2g5cfi77dj2gw690-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/ryf7vakjfmjyym2xmn87ffh18nbk26jc-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `49c5e6b7f9bbde1dff5d3d566e108b1a6cc8e52067da10a80fd34dbd88d47e05`.

Historical (`eon-yp6vg.7.12`):

Renewed on 2026-11-19 for `eon-yp6vg.7.12` to context
`f31033eeca20227b7c820d08e80dfabc00eb18ba7433409e792ff9f6d5847c71`
in both variants. Only the shared lock changes in this context.

- Source: `/nix/store/7s46p30d2n8hlwkk8wf1nqn61mcj4wqi-source`.
- Both observations: `/nix/store/nsgp795d45c7cgqkllja8k3df5sjff0d-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/adhrw32j6sb6h32pz6nyknzhy7cbv3kr-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `ab8db071d0749510f27bff3475d852e08a71a298394dda4dc9d6b3de84905acb`.

Historical (`eon-eon-agent-lr8sx.40.3`):

For `eon-eon-agent-lr8sx.40.3`, the projected context changes from
`0cc52b66ea290e19fc237d74b4f83bbc26fa8b7669c4db2769ecdf15a75b8ba1` to
`08b1b002544da6eb222850e2c24b5d1aee51650c53383629dfdb9a8e2271cac7`.
The normalized lock adds rota-cli's `time` edge. Full and fallback
release artifact observations are due at landing.

Historical (`eon-c828u.10`):

Renewed on 2026-11-18 for `eon-c828u.10` to context
`0cc52b66ea290e19fc237d74b4f83bbc26fa8b7669c4db2769ecdf15a75b8ba1`.
The full and fallback inputs independently resolve to the same
Orbit observation and binary. Both `inspect-audit` runs reproduce its canonical
graph byte for byte. Compared with accepted context
`a5d21c843de07f8c04aeda59db92486270fdcef7af8ad1fdd56f1fba63b27ab3`,
the only difference is rustix 1.1.4's selected native `pty` feature: removing
that one feature from the new canonical graph reproduces the accepted hash.
The historical graph is reconstructed for this comparison, rather than read
from an unavailable historical store output. Lock text, runtime identities,
all other native features, audited identities and build context are unchanged.

The exact-vendor offline runtime and normal/build feature trees match the
saved observation after translating only the source prefix to `/build/source`.
The same six host-only identities remain absent from runtime; their inverted
normal/build paths cross proc-macro boundaries. The sole unselected identity,
`errno 0.3.14`, is absent from both selected trees and has no compile line in
the actual Orbit producer log. Cached dependencies make log absence alone
insufficient; the selected trees corroborate it. The strict renewal proposal
changes only Orbit's two fingerprints, preserving every exception identity,
reason and evidence anchor across all producers. No runtime omission is waived.

- Source: `/nix/store/b459vd1wfpcisgmh0rqzgn5ddh8zsv1s-source`.
- Vendor: `/nix/store/0p2h1fsr16lz6k70ha1q1c9grrman53d-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `orbit/Cargo.toml`;
  root: `yazelix-orbit`; default features, no added root features.
- Full input: `/nix/store/85q71cd8q1jdaxi6nhr6r6mcyglwzg77-eon-archive-notice-input.json`.
- Fallback input: `/nix/store/44m789r4if1mpm2c7v29yvrkkdw134pa-eon-archive-notice-input.json`.
- Both observations: `/nix/store/sdgf8fdv46yb683mf8m594rrfry29kfl-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/03sp0n2gv522i743k34snxphrdpccqbz-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `ab8db071d0749510f27bff3475d852e08a71a298394dda4dc9d6b3de84905acb`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-c828u.7`):

Renewed on 2026-11-17 for `eon-c828u.7` dependency prep to context
`a5d21c843de07f8c04aeda59db92486270fdcef7af8ad1fdd56f1fba63b27ab3`.
Full and fallback contexts are identical. Compared with the
preceding accepted context
`eef5792335ae9efa2b09be7acf49529b7ede0497d44a445393c158d2bf21b644`,
only `lock` changes; the same 6 host-only and 1 unselected exceptions remain.

- Source: `/nix/store/03y8jb2d9hnzvn114ydlfrj4wzs01zxs-source`.
- Vendor: `/nix/store/0p2h1fsr16lz6k70ha1q1c9grrman53d-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `orbit/Cargo.toml`;
  root: `yazelix-orbit`; default features, no added root features.
- Full observation: `/nix/store/ph8bs6s23vrygag5pndjzq0qip7pcy2l-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/0acls5qawf1i7z03kk01z5dcmp8lrlvf-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `480288c838389eec01d01330141399b06269211c49f65d0030e1ea849bcaafd7`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.
- Fallback observation: `/nix/store/ph8bs6s23vrygag5pndjzq0qip7pcy2l-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/0acls5qawf1i7z03kk01z5dcmp8lrlvf-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `480288c838389eec01d01330141399b06269211c49f65d0030e1ea849bcaafd7`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (`eon-c828u.13`):

Renewed on 2026-11-17 for `eon-c828u.13` over `8a19251d` to context
`eef5792335ae9efa2b09be7acf49529b7ede0497d44a445393c158d2bf21b644`.
Full and fallback contexts are identical. Compared with accepted
context `e33f056c79434e86397a56cc4e9af89dfa18ee131b9caad6d28acc854c8bce04`, only `lock` changes.
The same 6 host-only and 1 unselected exceptions remain.

- Source: `/nix/store/6wqfap5qaa8v26crl8iwcxwjz98ni3a3-source`.
- Vendor: `/nix/store/8xhpmv7shr362b3k6h23z7vds17x8fld-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `orbit/Cargo.toml`;
  root: `yazelix-orbit`; default features, no added root features.
- Full observation: `/nix/store/9ygifgz3fz4r8y0wwb7hikj8c57rfgk0-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/qbp9k7xnfiyiz2hpznrlgdxvsmrpb2lc-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `480288c838389eec01d01330141399b06269211c49f65d0030e1ea849bcaafd7`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.
- Fallback observation: `/nix/store/9ygifgz3fz4r8y0wwb7hikj8c57rfgk0-orbit-cargo-notices-0.1.0`.
  Binary: `/nix/store/qbp9k7xnfiyiz2hpznrlgdxvsmrpb2lc-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
  SHA-256: `480288c838389eec01d01330141399b06269211c49f65d0030e1ea849bcaafd7`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (2026-11-17, `eon-eon-agent-lr8sx.40.1`):

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`e33f056c79434e86397a56cc4e9af89dfa18ee131b9caad6d28acc854c8bce04`.
The full and fallback values are identical, sharing the exact
report and binary. Compared with accepted context
`097e2347ea1e59e721f3de3c7b22d180cafa7e556f2e059b6618536cd3055e68`,
only the lock text differs (the `review-tier` stanza and the `rota-cli`
rename); the binary's SHA-256 is unchanged. The same 6 host-only and 1
unselected exceptions remain; no target-runtime dependency is waived.

- Source: `/nix/store/yfi665mzm3r68hbbz3bz28cn0h0xv2np-source`.
- Vendor: `/nix/store/d5bys2iry7p84jnbaj30yhcl3xsj78y3-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `orbit/Cargo.toml`;
  root: `yazelix-orbit`; default features, no added features.
- Both observations: `/nix/store/v0ywfnwm6vzhhkysdy76hx4j1as8rvzn-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/n654vqgil1ify4z3zry70kgibicba66f-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `10b9d6f80e796529ef5b583b5e8f839a4687467bb99a713723944981760406ea`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (2026-11-16):

Renewed on 2026-11-16 for `eon-igae.19` to context
`097e2347ea1e59e721f3de3c7b22d180cafa7e556f2e059b6618536cd3055e68`.
The full and fallback values are identical, sharing the exact
report and binary. Compared with accepted context
`db98f03d0c971c16d1dfac652ec82a7c25d8803af58563d45cf4ec6502626f07`,
only the benchmark lock edges differ. The same 6 host-only and 1 unselected
exceptions remain; no target-runtime dependency is waived.

- Source: `/nix/store/ibhp5z8c43hm12kppwda6p1g5lj0jxgh-source`.
- Vendor: `/nix/store/2315zg2zx3f5jd9n9886wa7pjp7fr3ir-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `orbit/Cargo.toml`;
  root: `yazelix-orbit`; default features, no added features.
- Both observations: `/nix/store/30ch3vipjk29lmn4c21a7kdlfsai8c2i-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/p8zgxwis9n36fds7vxb6vz9spyl6567g-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `10b9d6f80e796529ef5b583b5e8f839a4687467bb99a713723944981760406ea`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical (2026-11-13):

Renewed on 2026-11-13 for `eon-eon-agent-lr8sx.27` to dependency context
`db98f03d0c971c16d1dfac652ec82a7c25d8803af58563d45cf4ec6502626f07`.
Only normalized lock data differs from the accepted prior context; runtime,
native features, audited identities and the 6 host / 1 unselected exceptions
are unchanged. Both variants use the same report and binary. Reinspection,
exact vendor-selected trees and the actual producer log confirm the renewal.

- Source: `/nix/store/1hcf0vzc01bmr3x7wzj5y5nx2nr7bj45-source`.
- Vendor: `/nix/store/gm878n1hhyy3xzjjzijarrz1rah4sdcw-cargo-vendor-dir`.
- Report: `/nix/store/gsjlwcp4y5qfb92dhsgh1rxp37basq61-orbit-cargo-notices-0.1.0`.
- Binary: `/nix/store/b7pxv7j1crcxaf5rn9w1qjjix3pqv5ai-yazelix-orbit-0.1.0/bin/yazelix-orbit`.
- SHA-256: `f54d312253f9082fb364d2d5922f7388f41dab8590a9595f295133254022da70`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Historical observation:

- Source: `/nix/store/qvjrmxlca9drmhfjlr33zgf2iw5fab47-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/qvjrmxlca9drmhfjlr33zgf2iw5fab47-source/orbit/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazelix-orbit`.
- Observed build flags: `-j 6 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Binary `/nix/store/pl9b7z1vyzcvb7yfydii994m9fh10ikj-yazelix-orbit-0.1.0/bin/yazelix-orbit`:

- SHA-256: `f54d312253f9082fb364d2d5922f7388f41dab8590a9595f295133254022da70`.
- Result: 6 exact host exceptions; 1 exact unselected exceptions; zero unselected identities compiled in the build phase.

## venus

Renewed on 2026-11-23 for `eon-tfrjz` to context
`d2b3ae735959fe5f29c94b9a8d5712cdf1b83d6b1c29589686d4672bb349c3d1`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`aea9642c…` byte for byte.

- Source at `/nix/store/syff6bhl7hvq1f1g5lg4gcy2vaf7gccg-source` (candidate tree over edge `3028262e`, before this renewal's own edits).
- Observation at `/nix/store/d3g2imqfdr3mh14fl49gr5v0vx7s1nym-venus-cargo-notices-0.1.0`.
- Binary at `/nix/store/cf7l30rc3awn88p2bznqw79ff8dnca8r-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`
  with SHA-256 `dbc42fd216f72eec61389f9e62ab493fbd61b6986cce3fee26922f36fe855535`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-yp6vg.65`):

Renewed for `eon-yp6vg.65` to context
`aea9642c863c30538d7d94edc5e3d789f176d31fdc6fa8e6f696a66fc596a04d`.
Reversing the winit revision reproduces `9a0a1140…`.

- Source at `/nix/store/fq7a4nkiph7mwm9zz7vs9pd8ir28i4dy-source`.
- Observation at `/nix/store/9ig4z2pcq9bidkvdynfyscjr7lfarcdx-venus-cargo-notices-0.1.0`.
- Binary at `/nix/store/lv6h2j48bws63ckyk6a196wyfqy2x4qm-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`
  with SHA-256 `19ab1bd1e45e3570d5b006ce46f01579369ab3c38c37341ead972ec900eae0b0`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-yp6vg.64`):

Renewed for `eon-yp6vg.64` to context
`9a0a1140afdadf90f5f0245f0ade55355716ed3d22f57bd74bcc6471f933b024`.
Restoring the base normalized lock reproduces `59d6fb26…`.

- Source at `/nix/store/vfd3bsjm86axvd7g6znlrj6x3r1c9fp7-source`.
- Observation at `/nix/store/zi1hcwfqn2x9hdmxgv3vivr5hwmcgygz-venus-cargo-notices-0.1.0`.
- Binary at `/nix/store/a0yk1im0xxh0vpar6w79zjqb0ra5r41y-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`
  with SHA-256 `83369a2ca3301fad164afad8027b146cff97894ff147f4aaaad2682b10aa22f0`.
- Observed build flags `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-5a9kx`):

Renewed for `eon-5a9kx` to context
`59d6fb26d4ede50795fd68c6757170fe457be4f1daa1cfe118338a16b9c87023`.
Restoring the base normalized lock reproduces `bfa4519b…`.

- Source at `/nix/store/pa1n580and17i6ylp15xqf6z83wk05hf-source`.
- Observation at `/nix/store/hk4y8igbcfqlk1nlbcsf7xwxfkgqax4n-venus-cargo-notices-0.1.0`.
- Binary at `/nix/store/m0spn6p8sknxcr8811xrxgcwqw4nq31s-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`
  with SHA-256 `7dc214b4c304e3e3397c02f2535c03a8a6571f756db39b04e350c7fa446ff547`.
- Observed build flags `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-yp6vg.7.9`):

Renewed on 2026-11-22 for `eon-yp6vg.7.9` to context
`bfa4519b8698ca3dbd7cce90191ea0b93a3236dfa3d7031ced53a2eeb8778a80`
in the single archive variant. Restoring the parent Rune revision in the
normalized lock reproduces `1e960751…`.

- Source at `/nix/store/ghx2qicfk6bq7amnkqv4xv0mc5cb3mqy-source`.
- Observation at `/nix/store/43zs456258s2jzj7nmblva2w5jcvvkys-venus-cargo-notices-0.1.0`.
- Binary at `/nix/store/3whqfr9i2i6mwglnbmqmd6kf02wj4zgn-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`
  with SHA-256 `7dc214b4c304e3e3397c02f2535c03a8a6571f756db39b04e350c7fa446ff547`.
- Observed build flags `-j 6 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-eon-agent-lr8sx.40.5`):

Renewed on 2026-11-20 for `eon-eon-agent-lr8sx.40.5` to context
`1e96075187bb8d307bae06d72614839399272f31a471c480f6514412a26bc249`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `eon-boat` stanza; the binary is unchanged. Removing it reproduces
`2c6597da…`.

- Source: `/nix/store/y9n30z6ma9pm4gj1r49708cfpjwbv7gd-source` (candidate tree over edge `027f86d8`, before this renewal's own edits).
- Observation: `/nix/store/mz5jma46rqp47g3b65wc9xyffy9qa434-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/098ndbqcw53jfvbm217dgm0nkh7vp74b-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `3e128751102bbc6d2b18055cd5a66433df61358686d7fcc18829fbadb0f3b67a`.

Historical (`eon-yp6vg.7.6`):

Renewed on 2026-11-20 for `eon-yp6vg.7.6` to context
`2c6597da6a1f6a6efb0a5e3df75ea7d0a40b7d4d9d730e4d8e36d346fee1cea7`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains Venus's optional `rune` graph (83 stanzas plus
dependency-list entries in 14). Restoring the parent lock reproduces
`3f5a96c0…`.

- Source: `/nix/store/sf769wlshz54i0l6qmifxflplxyig2dy-source` (candidate tree over edge `f592d1ba`, before this renewal's own edits).
- Observation: `/nix/store/0p5pbijcyaw5im56cp3lbq071zx3vc0w-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/xav2dvp2hvx691ry61qcdg7w0l4v5pqy-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `3e128751102bbc6d2b18055cd5a66433df61358686d7fcc18829fbadb0f3b67a`.

Historical (`eon-2r02l.2`):

Renewed on 2026-11-20 for `eon-2r02l.2` to context
`3f5a96c03d5800a4cde1af9e40fe26ecbd3914ef9ec1e5f069041ee1d9c43b0a`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-jail` stanza. Removing it reproduces
`fe464495…`.

- Source: `/nix/store/i5gd0bg7nz8jq6ndzc921lirv6skpj25-source` (staged tree over edge `2f6857b2`).
- Observation: `/nix/store/526aywdbzkr3gszvdd1i3h4lwhpnzdaj-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/qnlryzlgqxqfals6d69zlp4j5hyhqdh7-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `3e128751102bbc6d2b18055cd5a66433df61358686d7fcc18829fbadb0f3b67a`.

Historical (`eon-2r02l.1`):

Renewed on 2026-11-19 for `eon-2r02l.1` to context
`fe464495133ef4c7bf903417c4efd8a5d0e30f8a32a8829811e59ec21ee1107e`
in the single archive variant. Only the normalized workspace lock changes in
this context: it gains the local `agent-bench` stanza. Removing it reproduces
`9ae02fe9…`.

- Source: `/nix/store/0nzm6kxn6478pdqg00g7ybb5pzim5s16-source` (staged tree over edge `d8386d4f`).
- Observation: `/nix/store/lzi23ralqzv2dw86myy4mnj16cdc1dsg-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/nad6906lxhr3009qswimylv5qx5g3hjm-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `3e128751102bbc6d2b18055cd5a66433df61358686d7fcc18829fbadb0f3b67a`.

Historical (`eon-yp6vg.7.13`):

Renewed on 2026-11-19 for `eon-yp6vg.7.13` to context
`9ae02fe985342587291f656aaef9b14e2a76b0969d6117db1da24fd698b5c753`
in both variants. The lock and the selected smallvec version change in the runtime,
native and audited graphs.

- Source: `/nix/store/ha0w93jabhgn5c9vdpdzq2whdyazg6in-source`.
- Full observation: `/nix/store/x0q0w1q8w5yp3wqck0ivf14r8w4s9zrd-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/1jvg3q87s9ag6v3212k3h837k4hmv9m8-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Without Accelerando observation: `/nix/store/lh80kjxiwl97w07jzh2n92b3jv964ymz-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/72s5hgyvjp078gl4s3y0hjf587hfb1wx-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- SHA-256 for both binaries: `3e128751102bbc6d2b18055cd5a66433df61358686d7fcc18829fbadb0f3b67a`.

Historical (`eon-kx9uh`):

Renewed on 2026-11-19 for `eon-kx9uh` to context
`21358fd89d42ea77db8fc7629f268adb47f2bdea330f17f24b0704cf0f6f07f1`
in both variants. Only the shared lock changes in this context: it gains
`review-tier`'s `serde_json` and `sha2` edges. Removing those two lines from
either observation's graph reproduces `e80f9935…`. The `4ee5ce34` report
`/nix/store/38x39hdbqksyflv9sihhn55bwb2qnc61-venus-cargo-notices-0.1.0`
holds the accepted context. The `89dc5264` report
`/nix/store/c9jvlspzgf6c2hgv337bh2z2rdqpkq2i-venus-cargo-notices-0.1.0`
holds this one.

- Source: `/nix/store/qm63sxyss33ma6d1v2nxw5vbsfl8jcwd-source` (edge `22967582`).
- Full observation: `/nix/store/i2zrdzbjki0bmfvncli58wcbhw4s22b2-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/vpy1kcg70fcbpyfc43l043v0y60w0xrc-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback observation: `/nix/store/akqqcqwlpb1iigv2z50mkqahx970i93p-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/35qa09xlj48dgqjq9gf4gvsl0d0ajwdj-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- SHA-256 for both binaries: `0b322c36e269136c9383c980ba0aa863b9802cab1a2e0c63537844f597a54f04`.

Historical (`eon-yp6vg.7.12`):

Renewed on 2026-11-19 for `eon-yp6vg.7.12` to context
`e80f9935370e3e52cc1120d03584e0440c3a452ba13239d27fcd0a5db1539e34`
in both variants. The two wgpu version changes affect the shared lock,
runtime graph, native feature graph and audited normal/build graph.

- Source: `/nix/store/gw8daf7xp83qivpx2ra5jwj4jdfxzck9-source`.
- Full observation: `/nix/store/c1ackl4a8rca86f2hnzfjmcb6p078gz2-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/1kkkb3l7952bv4s9ymqydrzxsxfrasb8-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback observation: `/nix/store/wp6q8z3gv008ijs65n39m62m5wcx1mxj-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/py2kxdps6bp1224d49gm1c3dny02j2y9-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- SHA-256 for both binaries: `b1c2a14b9399b0fece31376f2b55773940aca2051b63be111769a61919e066f5`.

Historical (`eon-eon-agent-lr8sx.40.3`):

For `eon-eon-agent-lr8sx.40.3`, the projected context changes from
`9a2afd2f38afe0655273d5c7c94984aa0534690fd8c339c9af4225417649c151` to
`79a485081ca3d288ba3b89eab0793151d240d6431bdf0413b04bb13f879e698e`.
The normalized lock adds rota-cli's `time` edge. Full and fallback
release artifact observations are due at landing.

Historical (`eon-c828u.7`):

Renewed on 2026-11-17 for `eon-c828u.7` dependency prep to context
`9a2afd2f38afe0655273d5c7c94984aa0534690fd8c339c9af4225417649c151`.
Full and fallback contexts are identical. Compared with the
preceding accepted context
`00e8194e48dd891f6845f9934f3525603de09f2a518e1729e1e4c0153443f91f`,
only `lock` changes; the same 15 host-only and 4 unselected exceptions remain.

- Source: `/nix/store/pfzfn1ihxk5xzr9anlz4m8dd9yj9fq6q-source`.
- Vendor: `/nix/store/0p2h1fsr16lz6k70ha1q1c9grrman53d-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added root features.
- Full observation: `/nix/store/lw1m59z9hdi10z67az46pcklijzfllkg-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/plb55b5m77v7h8iyqchw56rznlzp0mbl-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `c519ac8268d5837b3deee41aa7d8aab9c6448bcf90e4b0513b850dcdfc16f74f`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.
- Fallback observation: `/nix/store/pr7qyxaasn89w66jkgfc3xbm1njid3wj-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/yynq0i8r8a7si0q2wjxbbf9b2wjif5rs-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `c519ac8268d5837b3deee41aa7d8aab9c6448bcf90e4b0513b850dcdfc16f74f`.
  Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (`eon-c828u.13`):

Renewed on 2026-11-17 for `eon-c828u.13` over `8a19251d` to context
`00e8194e48dd891f6845f9934f3525603de09f2a518e1729e1e4c0153443f91f`.
Full and fallback contexts are identical. Compared with accepted
context `f03069b4636ff3169fb1f2e9257a80eff45da65adf635eec5ba94ab3796a83b3`, only `lock` changes.
The same 15 host-only and 4 unselected exceptions remain.

- Source: `/nix/store/bd1xbggpbrmjxm3gvqmprrm21m8yznkm-source`.
- Vendor: `/nix/store/8xhpmv7shr362b3k6h23z7vds17x8fld-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added root features.
- Full observation: `/nix/store/07b1wmblz2c18kjlm96xrkd403594bpb-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/wxwgb48cv8z8k0g9svz6lnnndz5ybccp-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `c519ac8268d5837b3deee41aa7d8aab9c6448bcf90e4b0513b850dcdfc16f74f`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.
- Fallback observation: `/nix/store/xwm3nksl6i3fm75d0dsi4ck52r93lm3b-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/lww4ychr666dllif2pn9680brfc4mzjv-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
  SHA-256: `c519ac8268d5837b3deee41aa7d8aab9c6448bcf90e4b0513b850dcdfc16f74f`.
  Actual flags: `-j 1 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (2026-11-17, `eon-tulof.4`):

Renewed on 2026-11-17 for `eon-tulof.4` to context
`f03069b4636ff3169fb1f2e9257a80eff45da65adf635eec5ba94ab3796a83b3`.
Full and fallback observations independently have this same context.
Compared with accepted context
`79f8d05538d07a968347a93ce2f0ca35ebe149b6f36abc223db694cf3ea9335b`,
the canonical graphs differ only by `param` in rustix 1.1.4's selected native
features. Context fields, normalized Cargo.lock, runtime identities and audited
package sets are unchanged. The added `NSProcessInfo` feature is macOS-only
and is not selected in these Linux graphs. No registry/git package or version
changes, license decision or target-runtime dependency waiver is introduced.

Independent `inspect-audit` runs on each actual binary reproduce the saved
canonical graph byte-for-byte. Offline trees from the exact vendor/source and
pinned Rust 1.96.1 toolchain reproduce each candidate's runtime and normal/build
feature trees after translating only `/build/source`. All 15 host-only and 4
unselected exception identities retain their classifications. In particular,
`async-recursion 1.1.1` occurs in the normal/build graph and not the runtime
graph. The four unselected identities (`bitflags 1.3.2`, `defmt 1.2.0`,
`defmt-macros 1.2.0`, `defmt-parser 1.1.0`) are absent from both selected trees
and have no compile line in either actual producer log. These builds reuse
Cargo artifacts; tree membership, rather than log absence alone, establishes
non-selection. The strict renewal proposal across both variants changes only
the two Venus fingerprint fields, preserving every producer, exception identity,
reason and evidence anchor. The owning bead records the runtime-dependency
omission falsifier and final full/fallback build results.

- Source: `/nix/store/jrpra68sxw44yp17p75ggr5piagsinyc-source`.
- Vendor: `/nix/store/d5bys2iry7p84jnbaj30yhcl3xsj78y3-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added root features.
- Full observation: `/nix/store/h2xyb708lm56x7sapsirisrj2xfxcg9a-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/wdih4ixzz5nlzhnbg497pr34k513r38h-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback observation: `/nix/store/c2y91h95x4kl7kjiazac5qj4j4a4l1pm-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/rbl1ydpsxm44732rkc34n5ff3sycikd9-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Both binaries' SHA-256: `c519ac8268d5837b3deee41aa7d8aab9c6448bcf90e4b0513b850dcdfc16f74f`.
- Actual flags in both producer logs: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (2026-11-17, `eon-eon-agent-lr8sx.40.1`):

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`79f8d05538d07a968347a93ce2f0ca35ebe149b6f36abc223db694cf3ea9335b`.
The full and fallback values are identical. Compared with accepted
context `f5bd33e8e6af2cc1b3b604bc31789009ca4169bdf3ecd6f10ee7d7e8c53f4ffb`,
only the lock text differs (the `review-tier` stanza and the `rota-cli`
rename). The same 15 host-only and 4 unselected exceptions remain; no
target-runtime dependency is waived.

- Source: `/nix/store/224bywpl5sl3151ss0p9j6ing5mfg57r-source`.
- Vendor: `/nix/store/d5bys2iry7p84jnbaj30yhcl3xsj78y3-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added features.
- Full observation: `/nix/store/whw7wqvlpcpf8v0xs772lg5pprh04bb7-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/5ya0blq4fh7ji31vlhqnk5wqx0b0hgar-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback observation: `/nix/store/8qga6pwb7ihmcp2gpsvf8vci9zy4iyil-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/d5njc9f489dkrip8a1ag6mh2xyg2574j-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Both binaries' SHA-256: `7ce1195634bae437c5578578628a85366df44c9ee3cc669ed97bcdf5811012db`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (2026-11-16):

Renewed on 2026-11-16 for `eon-igae.19` to context
`f5bd33e8e6af2cc1b3b604bc31789009ca4169bdf3ecd6f10ee7d7e8c53f4ffb`.
The full and fallback values are identical. Compared with accepted
context `1acb176eec03eca4ae079793fed52246022c1c9b4b05146b422f8bc227944173`,
only the benchmark lock edges differ. The same 15 host-only and 4 unselected
exceptions remain; no target-runtime dependency is waived.

- Source: `/nix/store/awqn2h223kwqh5v05q14k8hvbp1kj3jw-source`.
- Vendor: `/nix/store/2315zg2zx3f5jd9n9886wa7pjp7fr3ir-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added features.
- Full observation: `/nix/store/bmkbbhdadfq9wm6sf2445hss1925c87x-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/6mjv8xhq357ggyvmlb8nhhqyw4acvfkd-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback observation: `/nix/store/wp1xp1p1r6kpwzzgaq4rhyrm4cffpbh6-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/ka25kx1rrznxq1wmw17v1b67fx7m9b92-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Both binaries' SHA-256: `bd3ba284b8d3da20da4f95eac6cc83d62470d5b5f6e94974a9901707efbd7f6c`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (2026-11-13):

Renewed on 2026-11-13 for `eon-eon-agent-lr8sx.27` to dependency context
`1acb176eec03eca4ae079793fed52246022c1c9b4b05146b422f8bc227944173`.
Only normalized lock data differs from the accepted prior context; runtime,
native features, audited identities and the 15 host / 4 unselected exceptions
are unchanged. Both variants were reinspected independently; their binaries
have identical SHA-256 values. Exact vendor-selected trees and each producer
log confirm that no unselected exception was compiled.

- Source: `/nix/store/ikln6ry3dn0hy1yc40wgdf8m4c3gzba7-source`.
- Vendor: `/nix/store/gm878n1hhyy3xzjjzijarrz1rah4sdcw-cargo-vendor-dir`.
- Full report: `/nix/store/risxjdf350hc7r7faq3khmp62nqlbx0g-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/hg799gllcixgakvwi3saq6s9bypw96y2-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Fallback report: `/nix/store/cksirsz8bhqdhh4pcbrc7iqv59njfkip-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/y46fx9mz6clamx9lac32krmrawx0rpvj-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Both binaries' SHA-256: `2c7b8f352c9d2973642faee2b93e2935eb28978c26fd673ba3ad045c475d2237`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical observation:

- Source: `/nix/store/bqif93jfykfcja6kvpdhjl2lhb246ik5-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/bqif93jfykfcja6kvpdhjl2lhb246ik5-source/venus/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazelix-venus`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Binary `/nix/store/7sz2z1rbi2sgyk5w21bq8v7difbgnwnk-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`:

- SHA-256: `b5d848ceefe988b7282614c8a71732277e184dfa9d3fc8cf6c2334b90220fb8a`.
- Result: 15 exact host exceptions; 4 exact unselected exceptions; zero unselected identities compiled in the build phase.

## installer

Renewed on 2026-11-23 for `eon-tfrjz` to context
`064b1b27e0f2fde37411911c6c63d940f77b2a21d145a99fb13601aa8de29c95`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`361bb0c5…` byte for byte. The binary SHA-256 is unchanged.

- Observation at `/nix/store/cq328i2dyd9cmhc4z5diipjlq9wryz02-installer-cargo-notices-0.1.0`.
- Binary at `/nix/store/fhdd1m2fs6jr31819dy2vz4hyjwhcy9a-eon-install-static-x86_64-unknown-linux-musl-0.1.0/bin/eon-install`
  with SHA-256 `83011d6eb4700295f0836aa9537995167eb2f79630886298a5d7ba2b0de151d1`.

Historical observation:

- Source: `/nix/store/jhbpz34d05k1irhqwr49vdbym93azamd-source`.
- Vendor: `/nix/store/65w6nyhlqmrcha74zn24pnpkf86cib6i-cargo-vendor-dir`.
- Manifest: `/nix/store/jhbpz34d05k1irhqwr49vdbym93azamd-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-musl`; roots: `eon-install`.
- Observed build flags: `-j 6 --target x86_64-unknown-linux-musl --offline --profile release`.

Binary `/nix/store/ynf2l9b17pk3m3hnf4nk0ygvw57n6nni-eon-install-static-x86_64-unknown-linux-musl-0.1.0/bin/eon-install`:

- SHA-256: `83011d6eb4700295f0836aa9537995167eb2f79630886298a5d7ba2b0de151d1`.
- Result: 0 exact host exceptions; 1 exact unselected exceptions; zero unselected identities compiled in the build phase.

## anima

Renewed on 2026-11-23 for `eon-tfrjz` to context
`e4d8399c97de0767c6a479479e1511f5bc4fc0568312f166c588bb830ce36387`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`9981c2ea…` byte for byte. The binary is unchanged.

- Observation at `/nix/store/804pm8r0s356ycmx8dv9dsflrzsfcrns-anima-cargo-notices-0.2.0`.
- Binary at `/nix/store/8zg64frgyrf01yc6basg06wx6fsx6nv2-anima-0.2.0/bin/anima`
  with SHA-256 `125b9a2c1554f35464ba194bb01e738c482b1568853185d34aac7cb9451b0a68`.

Historical observation:

- Source: `/nix/store/8d7c3wmsdsa7zq2y07iv42ig9lzmvbz5-source`.
- Vendor: `/nix/store/2dbpj9yzyn8678m8g6y56k5rq5i9cqsp-cargo-vendor-dir`.
- Manifest: `/nix/store/8d7c3wmsdsa7zq2y07iv42ig9lzmvbz5-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazelix_screen`.
- Observed build flags: `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --bin anima`.

Binary `/nix/store/8zg64frgyrf01yc6basg06wx6fsx6nv2-anima-0.2.0/bin/anima`:

- SHA-256: `125b9a2c1554f35464ba194bb01e738c482b1568853185d34aac7cb9451b0a68`.
- Result: 2 exact host exceptions; 0 exact unselected exceptions; zero unselected identities compiled in the build phase.

Fallback binary `/nix/store/0pskrfzyj08z2ki854q2cxh7pd2nr9xs-anima-0.2.0/bin/anima`:

- SHA-256: `6b4e5869f52aa0165c2e294a1c7997956d022582b7b0ac1a30af25c9dd6a524b`.
- Result: its entire rust-audit-info package record equals the full variant; the fallback changes artwork, not Cargo dependencies. Both variants share the same dependency context and two exact host exceptions.

## nushell

Renewed on 2026-11-23 for `eon-tfrjz` to context
`ec9549bf5630de2d91f591752fe60d96b8c90cd09dc25d6d6fda6c0f4e489ff3`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`8133b268…` byte for byte. The binary is unchanged.

- Observation at `/nix/store/f47aqi1xacnzigrpdv10zy3rab63dxgn-nushell-cargo-notices-0.113.1`.
- Binary at `/nix/store/6rf5daj415jnir6218pjk6b78phhx6pf-nushell-0.113.1/bin/nu`
  with SHA-256 `83b03ee3de996d7b85d44beb87fd2e6759781aa72dec64ced5b7e50aa7d4578f`.

Historical observation:

- Source: `/nix/store/3k3wi82n4jcwidzzywgfrvzqsxxaq6r7-source`.
- Vendor: `/nix/store/cnk4dp68vb4qwyhhh1lxby9mqxmzg303-nushell-0.113.1-vendor`.
- Manifest: `/nix/store/3k3wi82n4jcwidzzywgfrvzqsxxaq6r7-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `nu`.
- Observed build flags: `-j 2 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/6rf5daj415jnir6218pjk6b78phhx6pf-nushell-0.113.1/bin/nu`:

- SHA-256: `83b03ee3de996d7b85d44beb87fd2e6759781aa72dec64ced5b7e50aa7d4578f`.
- Result: 25 exact host exceptions; 39 exact unselected exceptions; zero unselected identities compiled in the build phase.

## starship

Renewed on 2026-11-23 for `eon-tfrjz` to context
`de4e1b4fd3b4ae7c57208138f769bc1a553f8df1f96ec0fd689b2d713506de93`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`51b7eca0…` byte for byte. The binary is unchanged.

- Observation at `/nix/store/wjxd9apnw6a8xjps72z850sa4pxqsfw7-starship-cargo-notices-1.25.1`.
- Binary at `/nix/store/2iyzhxjwbpir2n2xaqgq14v8455kbrri-starship-1.25.1/bin/starship`
  with SHA-256 `e187b6a7784b3520cbc97c8e836036217b25ef9ddc267545aee0837993aac9a6`.

Historical observation:

- Source: `/nix/store/d2aw7ksgaqr03j52jrr4jxan3kxj4fdh-source`.
- Vendor: `/nix/store/v4d5famarmylaaspras7rv5z1a3fkcg0-starship-1.25.1-vendor`.
- Manifest: `/nix/store/d2aw7ksgaqr03j52jrr4jxan3kxj4fdh-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `starship`.
- Observed build flags: `-j 2 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/2iyzhxjwbpir2n2xaqgq14v8455kbrri-starship-1.25.1/bin/starship`:

- SHA-256: `e187b6a7784b3520cbc97c8e836036217b25ef9ddc267545aee0837993aac9a6`.
- Result: 12 exact host exceptions; 8 exact unselected exceptions; zero unselected identities compiled in the build phase.

## zoxide

Renewed on 2026-11-23 for `eon-tfrjz` to context
`6bd0bb81256705d6fcbee6af33d7daea9edf44de625c9aae3b94820ebb856ce3`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`6759fae7…` byte for byte. The binary is unchanged.

- Observation at `/nix/store/sbsyg7v33ha1w9skjyzn96xnh1kqw61n-zoxide-cargo-notices-0.9.9`.
- Binary at `/nix/store/5jl0jd7ma5mjl4y2480428jfamy44j9q-zoxide-0.9.9/bin/zoxide`
  with SHA-256 `004288fb757dc8d54af2f8bf2ec05180ecd823e4ddd2dfbbf3c85cb680dea91b`.

Historical observation:

- Source: `/nix/store/6rv8qxfavygcmx5s9zfd36xvqc4dxs3i-source`.
- Vendor: `/nix/store/vp6vd00il0kdlv7n610r51zlsnl6mwgp-zoxide-0.9.9-vendor`.
- Manifest: `/nix/store/6rv8qxfavygcmx5s9zfd36xvqc4dxs3i-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `zoxide`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/5jl0jd7ma5mjl4y2480428jfamy44j9q-zoxide-0.9.9/bin/zoxide`:

- SHA-256: `004288fb757dc8d54af2f8bf2ec05180ecd823e4ddd2dfbbf3c85cb680dea91b`.
- Result: 1 exact host exceptions; 3 exact unselected exceptions; zero unselected identities compiled in the build phase.

## atuin

Renewed on 2026-11-23 for `eon-tfrjz` to context
`9e08c1fc8120a85b37396429ff3122b36177d52af9bfe3c8c090c3c04027406d`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`0a9bbc55…` byte for byte. All three binaries are unchanged.

- Observation at `/nix/store/mvbdh0n3055q3nily0p1f3fsgg1yni8a-atuin-cargo-notices-18.16.1`.
- Binaries at `/nix/store/sz7rawzyjpixy5yqgad9rlij3gp48bp3-atuin-18.16.1/bin/{atuin,atuin-nucleo-bench,atuin-server}`.

Historical observation:

- Source: `/nix/store/900asnj6vmgw61k3ih3qn0jpm7g30bfd-source`.
- Vendor: `/nix/store/pwzf101j3yfk8csi3lgn5zx21zxyinn1-atuin-18.16.1-vendor`.
- Manifest: `/nix/store/900asnj6vmgw61k3ih3qn0jpm7g30bfd-source/crates/atuin/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `atuin`, `atuin-server`, `atuin-nucleo-bench`.
- Observed build flags: `-j 2 --target x86_64-unknown-linux-gnu --offline --profile release --no-default-features --features=ai\,client\,clipboard\,daemon\,hex\,sync`.

Binary `/nix/store/sz7rawzyjpixy5yqgad9rlij3gp48bp3-atuin-18.16.1/bin/atuin`:

- SHA-256: `3a401d2a209c8dd1068231d753d6738c8d19920534a1b28d1209a59af2e031b3`.
- Result: 21 exact host exceptions; 52 exact unselected exceptions; zero unselected identities compiled in the build phase.

Binary `/nix/store/sz7rawzyjpixy5yqgad9rlij3gp48bp3-atuin-18.16.1/bin/atuin-nucleo-bench`:

- SHA-256: `365eca105d6872ea3f2e26b66fc341b4a63effb7405a85723fee8d83e278d50f`.
- Result: 0 exact host exceptions; 0 exact unselected exceptions; zero unselected identities compiled in the build phase.

Binary `/nix/store/sz7rawzyjpixy5yqgad9rlij3gp48bp3-atuin-18.16.1/bin/atuin-server`:

- SHA-256: `ad15671064b47e822650edcd0a5b9b330a2aafb20bdb226cab425dd824c355e8`.
- Result: 11 exact host exceptions; 16 exact unselected exceptions; zero unselected identities compiled in the build phase.

## helix

Renewed on 2026-11-23 for `eon-tfrjz` to context
`6d9874a0e927acd202e5a7aade68d70b5cd952f7ad4e3b94683782b275f3c474`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`2232f301…` byte for byte. The binary is unchanged.

- Observation at `/nix/store/8lf2rm764g8lg6iljn6xsjlcmdn8df33-helix-cargo-notices-0`.
- Binary at `/nix/store/z1ly1fb45yn3mgq6yihkl2ax3520mh3v-helix-term/bin/hx`
  with SHA-256 `509554c6bf13fac32e01a1d4143c1ff66b34a142bfea09a0392df9ca7e7aa0f9`.

Historical observation:

- Source: `/nix/store/chwv3cyaalw9d6md279pfqbqayylz2kn-source`.
- Vendor: `/nix/store/q3mbqyl3wxpf5ycaia135sv8gkz37yqa-cargo-vendor-dir`.
- Manifest: `/nix/store/chwv3cyaalw9d6md279pfqbqayylz2kn-source/helix-term/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `helix-term`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --no-default-features --features=git`.

Binary `/nix/store/z1ly1fb45yn3mgq6yihkl2ax3520mh3v-helix-term/bin/hx`:

- SHA-256: `509554c6bf13fac32e01a1d4143c1ff66b34a142bfea09a0392df9ca7e7aa0f9`.
- Result: 6 exact host exceptions; 117 exact unselected exceptions; zero unselected identities compiled in the build phase.

## yazi

Renewed on 2026-11-23 for `eon-tfrjz` to context
`eda2a8090d45061da82295aacd41b0a72b344f70bf42bb19bb38de92c05ae4b4`
in the single archive variant. Only `context.toolchain.rust` changes (`1.96.1` → `1.100.0`);
restoring `/nix/store/n6l7lf1j0p7wxr3jxdp1lhag5jnv1i5n-rust-default-1.96.1` reproduces
`cb7a1427…` byte for byte. Both binaries are unchanged.

- Observation at `/nix/store/k5yjdvac8yzxhyhln4inq544w8ici8p5-yazi-cargo-notices-26.5.6`.
- Binaries at `/nix/store/pp4219z22rpc2ah111iv6pkpf4sgxgzf-yazi-26.5.6/bin/{ya,yazi}`.

Historical observation:

- Source: `/nix/store/59clvnpw37rj47yw5a40hlhsgmlh5bh1-source`.
- Vendor: `/nix/store/y3x24npj865gmsbwmrv6ysr6h500gf9q-yazi-26.5.6-vendor`.
- Manifest: `/nix/store/59clvnpw37rj47yw5a40hlhsgmlh5bh1-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazi-fm`, `yazi-cli`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/pp4219z22rpc2ah111iv6pkpf4sgxgzf-yazi-26.5.6/bin/ya`:

- SHA-256: `b5ec513022d785f8d84b2a9eb6a764d28e02a551a5ebb9cd33da461a9038da7f`.
- Result: 23 exact host exceptions; 52 exact unselected exceptions; zero unselected identities compiled in the build phase.

Binary `/nix/store/pp4219z22rpc2ah111iv6pkpf4sgxgzf-yazi-26.5.6/bin/yazi`:

- SHA-256: `de61d5b03f7d0cad121852740f89488787c331487ac2eb3bbd3c57244983c94c`.
- Result: 23 exact host exceptions; 52 exact unselected exceptions; zero unselected identities compiled in the build phase.
