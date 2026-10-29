# Eon Runtime

The runtime library and canonical workspace protocol used by Eon and Venus.
Both crates build and test without the Eon crates.

| Package | Owns | Version |
|---|---|---|
| `eon-runtime` | Invocation, terminal lifecycle, workspace state, configuration parsing and generation identity | `0.1.0` |
| `eon-workspace-protocol` | EONW v25 types and bounded codec | `0.1.0` |

## Consumer boundary

Eon's actual executable calls `eon_runtime::run(inputs)`. The concrete root
exports construct `Inputs`, defaults and managed-program inputs; the ten
mechanism modules remain private. `run` returns the product label and exit
result. Eon retains process exit/error handling.
The private CLI prints grouped Eon help for standalone `-h` and `--help`,
with terminal-only color and concise usage errors; child options remain forwarded.

Popup configuration rejects Alt+Slash, reserved for the shortcut viewer, and
Alt+0 through Alt+9, reserved for tab positions, before creating workspace
state. A popup on the key of any other native binding takes that key.

Runtime parses `[bar] widgets` against the assembly-supplied widget names when
opening or reopening a workspace presentation and passes the list to Venus
unchanged; the version launch input, which carries the supervising product
version, goes only when the list names `version`.

Later tabs begin in the current user's native home directory, validated before
creation. Dismissing a pending Project picker keeps the tab and starts its first
pane at that directory; an explicit tab close removes it without a replacement.
Initial workspace launch and explicit directory selection retain their own paths.
A failed first-pane fallback reports the error and leaves other tabs running.

Eon supplies its own product version, chosen defaults, validated component
report, opaque executable paths, and immutable assembly contribution. The
runtime has no dependency on Eon, its component graph, validator, product
defaults, assets or Nix expressions.

Runtime hashes the supplied assembly contribution into one `g1-` generation,
which Orbit terminal records carry as their component generation. The
contribution must cover the runtime and EONW source, as Eon's source-tree
digest does. Mutable configuration, live state and store/profile paths do not
become generation identity. Orbit remains the terminal owner; Venus remains the
native presentation owner.

When an acquired Orbit dies without a final result, Runtime validates the dead
process and original Live record before retiring its exact residue. Healthy
terminals remain usable; owner loss is a local failure, not a successful Stop
or a fabricated terminal outcome.

Eon's `eon/crates/eon/src/product.rs` supplies the concrete inputs; its
`build.rs` digests the source tree into the assembly contribution.
Consumers in this workspace use path dependencies; crates are not published to
a registry. Eon owns product composition and distribution.

CodexBar CLI owns Codex and Claude quota collection and credential access.
Eon supplies its opaque executable path. Independent workers use explicit Codex
CLI and Claude OAuth sources, with no browser fallback or inference. Each read
has a ten-second deadline, 64 KiB output cap and process-group cleanup; Codex
reads start at least one minute apart and Claude reads at least five minutes
apart. Raw provider JSON and identities are discarded.

Runtime validates timestamps, actual periods and percentages, rounding remaining
percentages down once. Failed reads keep last-good facts unchanged;
observations become stale after one hour and disappear after 24 hours.
Windows disappear at a supplied reset; missing reset times remain unknown.
Successful missing quota clears prior facts.
Eon's [EON-C27](../docs/CONTRACTS.md#eon-c27--codexbar-quota-collection)
owns product policy and installed acceptance.

EONW v25 adds the window-wide EON-C31 `attention_queue` in Runtime's total
order; consumers never sort it. v24 adds a directory-based tab launch request,
optionally carrying an exact argv from a calling terminal. v23 adds the background attention state, ranked
between working and idle.
v22 gives each workspace summary its attention rollup and recency rank. v21
gives each popup entry a shortcut, or none because the configuration
unbound it or because another entry holds its default shortcut. v20 adds the
ordered workspace summaries, shown and projected workspace
identities, create/switch/rename/close workspace requests, and pane labels,
nullable pins and `RenamePane`. Runtime observes existing ORBS metadata for all
workspace panes and owns label composition; clients render that bounded text. v19 adds the
attention `name`, the terminal title Orbit reports beside
the status records; v18 adds EON-C31 agent attention on panes, popups and
tabs, the window's sidebar visibility, and the sidebar and attention-focus
actions; v17 adds an
`Ended` reply to tab and pane close, sent when the close ended the window;
v16 lets both pane-create requests add a pane without taking focus; v15 adds
a request that runs an exact command in a new pane for a calling terminal; v14
adds a request that opens a file in a new editor pane; v13 lets a request name
its calling terminal; v12 adds a request that
moves the selected pane into the neighbouring column; v11 adds a request that
closes one pane by its stable identity; v10 arranges each tab's panes into up
to four columns; v9 permits genuinely absent provider resets, including stale
observations. Only v25 remains: the CLI
inspects and stops supervisors that use it, lists any other as incompatible,
and presentation remains bound to the original exact executable.

## Packages

Runtime and codec have separate package manifests and version owners. Runtime
depends on the in-tree Orbit protocol crate.

EONW's complete package is its Cargo manifest, `src/lib.rs` and
`src/v25.rs`. It has no external dependencies, feature declarations,
build script or workspace-inherited settings.

Runtime evolution has one owner here; Eon owns product composition and
installed-product proof.

## Development

Use Rust 1.95 or later. From the repository root:

```sh
cargo fmt --check -p eon-runtime -p eon-workspace-protocol
cargo check --locked -p eon-runtime -p eon-workspace-protocol
nix develop -c eon-test -p eon-runtime -p eon-workspace-protocol
cargo clippy --locked -p eon-runtime -p eon-workspace-protocol --all-targets -- -D warnings
```

`eon-test` runs the tests in the isolated sandbox that `nix/test-sandbox.nix`
defines.
Tests use independent supplied inputs. Eon's actual executable integration
tests stay with Eon in `eon/crates/eon/tests`.
The composed product is proved on x86_64 Linux native Wayland; Apple Silicon
macOS remains unproved for Eon.

## License

[Apache-2.0](LICENSE), with the attribution in [NOTICE](NOTICE).
Eon assets and their unrelated third-party notices are not part of this source
library.

## LOC scorecard

Tracked handwritten text and code; excludes Git data, lock files and generated
artifacts.

| Surface | Lines |
|---|---:|
| Rust source and tests | 13,601 |
| Cargo manifests | 24 |
| Agent guidelines | 37 |
| README | 127 |
| License and notice | 209 |
| **Total** | **13,998** |
