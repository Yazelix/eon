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
The Eon, Orbit and Venus sections record the 2026-11-17 renewal for
`eon-eon-agent-lr8sx.40.1`, observed on that bead's staged tree over edge
`c901a60f`. Their earlier observations remain labelled historical. The owning
bead records the final staged-tree gates and artifact identities after the
evidence update.

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

The current three contexts differ from the accepted 2026-11-16 contexts only
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

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`3c8036faf5a889ebb14b58572717640ad2d4ac35bcd47f1794290917ba93d36a`.
The full and without-Accelerando values are identical. Compared with accepted
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
- Without-Accelerando observation: `/nix/store/djvw8z795za2bmrqf0jz770zlsf8pj99-eon-cargo-notices-0.1.28`.
  Binary: `/nix/store/p7ykvhjr06l4wpndnnahv0x6r0d124hp-eon-rust-0.1.28/bin/eon`.
  SHA-256: `724e7a4ee3d1689ef1ef64e26fc851f4525be477463590ad1de97d0f33ae8e21`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (2026-11-16):

Renewed on 2026-11-16 for `eon-igae.19` to context
`73ec261a2f561ac31bf0847bc31a743a1c88b4c725449eca555bbded7a86b454`.
The full and without-Accelerando values are identical. Compared with accepted
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
- Without-Accelerando observation: `/nix/store/nbybygfmcpw0a2m4gq5km08h1s0lgayz-eon-cargo-notices-0.1.27`.
  Binary: `/nix/store/7bqkl8q51ni21j915vryq8syxpdb4nlx-eon-rust-0.1.27/bin/eon`.
  SHA-256: `fa66151387d9938fabe8eb1b65b26f75d7ca217459f8ddc5c0aa28a418c782e0`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.

Historical (2026-11-13):

Renewed on 2026-11-13 from staged tree
`85aa2da7f6c11a847fa6eb186d797c6a2c727088` (`eon-eon-agent-lr8sx.27`).
Full and without-Accelerando observations independently reproduce dependency
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
- Without-Accelerando report: `/nix/store/w0429hsbxkppys66vz0zkwfnsy57lxfm-eon-cargo-notices-0.1.25`.
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

### without-accelerando

- Source: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/ms5cx9w32fav2swkgd0365a3nqa01mwc-source/eon/crates/eon/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; root: `eon`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package eon`.
- Binary: `/nix/store/zimld5w76sl24n1xvjkcslssq14hz41j-eon-rust-0.1.24/bin/eon`.
- SHA-256: `96ad86f70b561c7216da0bacefc0891e812b4c6103ec52a85f49f13a2726b990`.

## orbit

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`e33f056c79434e86397a56cc4e9af89dfa18ee131b9caad6d28acc854c8bce04`.
The full and without-Accelerando values are identical, sharing the exact
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
The full and without-Accelerando values are identical, sharing the exact
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

Renewed on 2026-11-17 for `eon-eon-agent-lr8sx.40.1` to context
`79f8d05538d07a968347a93ce2f0ca35ebe149b6f36abc223db694cf3ea9335b`.
The full and without-Accelerando values are identical. Compared with accepted
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
- Without-Accelerando observation: `/nix/store/8qga6pwb7ihmcp2gpsvf8vci9zy4iyil-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/d5njc9f489dkrip8a1ag6mh2xyg2574j-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Both binaries' SHA-256: `7ce1195634bae437c5578578628a85366df44c9ee3cc669ed97bcdf5811012db`.
- Actual flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-venus`.

Historical (2026-11-16):

Renewed on 2026-11-16 for `eon-igae.19` to context
`f5bd33e8e6af2cc1b3b604bc31789009ca4169bdf3ecd6f10ee7d7e8c53f4ffb`.
The full and without-Accelerando values are identical. Compared with accepted
context `1acb176eec03eca4ae079793fed52246022c1c9b4b05146b422f8bc227944173`,
only the benchmark lock edges differ. The same 15 host-only and 4 unselected
exceptions remain; no target-runtime dependency is waived.

- Source: `/nix/store/awqn2h223kwqh5v05q14k8hvbp1kj3jw-source`.
- Vendor: `/nix/store/2315zg2zx3f5jd9n9886wa7pjp7fr3ir-cargo-vendor-dir`.
- Target: `x86_64-unknown-linux-gnu`; manifest: `venus/Cargo.toml`;
  root: `yazelix-venus`; default features, no added features.
- Full observation: `/nix/store/bmkbbhdadfq9wm6sf2445hss1925c87x-venus-cargo-notices-0.1.0`.
  Binary: `/nix/store/6mjv8xhq357ggyvmlb8nhhqyw4acvfkd-yazelix-venus-0.1.0/bin/.yazelix-venus-wrapped`.
- Without-Accelerando observation: `/nix/store/wp1xp1p1r6kpwzzgaq4rhyrm4cffpbh6-venus-cargo-notices-0.1.0`.
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
- Without-Accelerando report: `/nix/store/cksirsz8bhqdhh4pcbrc7iqv59njfkip-venus-cargo-notices-0.1.0`.
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

- Source: `/nix/store/jhbpz34d05k1irhqwr49vdbym93azamd-source`.
- Vendor: `/nix/store/65w6nyhlqmrcha74zn24pnpkf86cib6i-cargo-vendor-dir`.
- Manifest: `/nix/store/jhbpz34d05k1irhqwr49vdbym93azamd-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-musl`; roots: `eon-install`.
- Observed build flags: `-j 6 --target x86_64-unknown-linux-musl --offline --profile release`.

Binary `/nix/store/ynf2l9b17pk3m3hnf4nk0ygvw57n6nni-eon-install-static-x86_64-unknown-linux-musl-0.1.0/bin/eon-install`:

- SHA-256: `83011d6eb4700295f0836aa9537995167eb2f79630886298a5d7ba2b0de151d1`.
- Result: 0 exact host exceptions; 1 exact unselected exceptions; zero unselected identities compiled in the build phase.

## anima

- Source: `/nix/store/8d7c3wmsdsa7zq2y07iv42ig9lzmvbz5-source`.
- Vendor: `/nix/store/2dbpj9yzyn8678m8g6y56k5rq5i9cqsp-cargo-vendor-dir`.
- Manifest: `/nix/store/8d7c3wmsdsa7zq2y07iv42ig9lzmvbz5-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazelix_screen`.
- Observed build flags: `-j 4 --target x86_64-unknown-linux-gnu --offline --profile release --bin anima`.

Binary `/nix/store/8zg64frgyrf01yc6basg06wx6fsx6nv2-anima-0.2.0/bin/anima`:

- SHA-256: `125b9a2c1554f35464ba194bb01e738c482b1568853185d34aac7cb9451b0a68`.
- Result: 2 exact host exceptions; 0 exact unselected exceptions; zero unselected identities compiled in the build phase.

Without-Accelerando binary `/nix/store/0pskrfzyj08z2ki854q2cxh7pd2nr9xs-anima-0.2.0/bin/anima`:

- SHA-256: `6b4e5869f52aa0165c2e294a1c7997956d022582b7b0ac1a30af25c9dd6a524b`.
- Result: its entire rust-audit-info package record equals the full variant; the fallback changes artwork, not Cargo dependencies. Both variants share the same dependency context and two exact host exceptions.

## nushell

- Source: `/nix/store/3k3wi82n4jcwidzzywgfrvzqsxxaq6r7-source`.
- Vendor: `/nix/store/cnk4dp68vb4qwyhhh1lxby9mqxmzg303-nushell-0.113.1-vendor`.
- Manifest: `/nix/store/3k3wi82n4jcwidzzywgfrvzqsxxaq6r7-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `nu`.
- Observed build flags: `-j 2 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/6rf5daj415jnir6218pjk6b78phhx6pf-nushell-0.113.1/bin/nu`:

- SHA-256: `83b03ee3de996d7b85d44beb87fd2e6759781aa72dec64ced5b7e50aa7d4578f`.
- Result: 25 exact host exceptions; 39 exact unselected exceptions; zero unselected identities compiled in the build phase.

## starship

- Source: `/nix/store/d2aw7ksgaqr03j52jrr4jxan3kxj4fdh-source`.
- Vendor: `/nix/store/v4d5famarmylaaspras7rv5z1a3fkcg0-starship-1.25.1-vendor`.
- Manifest: `/nix/store/d2aw7ksgaqr03j52jrr4jxan3kxj4fdh-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `starship`.
- Observed build flags: `-j 2 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/2iyzhxjwbpir2n2xaqgq14v8455kbrri-starship-1.25.1/bin/starship`:

- SHA-256: `e187b6a7784b3520cbc97c8e836036217b25ef9ddc267545aee0837993aac9a6`.
- Result: 12 exact host exceptions; 8 exact unselected exceptions; zero unselected identities compiled in the build phase.

## zoxide

- Source: `/nix/store/6rv8qxfavygcmx5s9zfd36xvqc4dxs3i-source`.
- Vendor: `/nix/store/vp6vd00il0kdlv7n610r51zlsnl6mwgp-zoxide-0.9.9-vendor`.
- Manifest: `/nix/store/6rv8qxfavygcmx5s9zfd36xvqc4dxs3i-source/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `zoxide`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release`.

Binary `/nix/store/5jl0jd7ma5mjl4y2480428jfamy44j9q-zoxide-0.9.9/bin/zoxide`:

- SHA-256: `004288fb757dc8d54af2f8bf2ec05180ecd823e4ddd2dfbbf3c85cb680dea91b`.
- Result: 1 exact host exceptions; 3 exact unselected exceptions; zero unselected identities compiled in the build phase.

## atuin

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

- Source: `/nix/store/chwv3cyaalw9d6md279pfqbqayylz2kn-source`.
- Vendor: `/nix/store/q3mbqyl3wxpf5ycaia135sv8gkz37yqa-cargo-vendor-dir`.
- Manifest: `/nix/store/chwv3cyaalw9d6md279pfqbqayylz2kn-source/helix-term/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `helix-term`.
- Observed build flags: `-j 8 --target x86_64-unknown-linux-gnu --offline --profile release --no-default-features --features=git`.

Binary `/nix/store/z1ly1fb45yn3mgq6yihkl2ax3520mh3v-helix-term/bin/hx`:

- SHA-256: `509554c6bf13fac32e01a1d4143c1ff66b34a142bfea09a0392df9ca7e7aa0f9`.
- Result: 6 exact host exceptions; 117 exact unselected exceptions; zero unselected identities compiled in the build phase.

## yazi

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
