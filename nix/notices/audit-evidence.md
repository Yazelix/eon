# Rust audit over-report evidence

These exact exceptions belong to the existing archive notice inventory. They
were checked against the candidate produced from tree
`9dbb50a4171e760efd954c0175c0894b2ffcee98` (Linux x86_64, Eon 0.1.24).
`audit-exceptions.json` names each producer, root name and exact package version.
The checker binds these records to the normalized Cargo.lock, resolved runtime
and native package/feature sets, audited package sets, build flags, target and
pinned toolchain. Local package versions and binary bytes are excluded: ordinary
code and release-version changes preserve the dependency evidence. Dependency,
feature or build-context changes require explicit inspection and reviewed renewal;
unused exceptions and missing evidence anchors fail. The full native-binding
graph remains intact. See the checked-in [inspection and renewal recipe](../../eon/tools/archive-notices/README.md).
The SHA-256 values below identify the original observations, not acceptance keys.

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

## eon

Both exact supported variants were checked from source tree
`279f0494fe622b1b0c82fa51baf6501840da5813`. Their complete audit records and
runtime/build identity sets equal the initial candidate. Each has 12 exact
host exceptions and one exact unselected exception, with no unselected
identity compiled during its build. Both variants share dependency evidence and
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

- Source: `/nix/store/qvjrmxlca9drmhfjlr33zgf2iw5fab47-source`.
- Vendor: `/nix/store/yr6yn574307ph9hzp3saylp50gx7j10b-cargo-vendor-dir`.
- Manifest: `/nix/store/qvjrmxlca9drmhfjlr33zgf2iw5fab47-source/orbit/Cargo.toml`.
- Target: `x86_64-unknown-linux-gnu`; roots: `yazelix-orbit`.
- Observed build flags: `-j 6 --target x86_64-unknown-linux-gnu --offline --profile release --package yazelix-orbit`.

Binary `/nix/store/pl9b7z1vyzcvb7yfydii994m9fh10ikj-yazelix-orbit-0.1.0/bin/yazelix-orbit`:

- SHA-256: `f54d312253f9082fb364d2d5922f7388f41dab8590a9595f295133254022da70`.
- Result: 6 exact host exceptions; 1 exact unselected exceptions; zero unselected identities compiled in the build phase.

## venus

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
