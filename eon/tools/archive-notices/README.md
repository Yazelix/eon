# Archive Rust audit checks

`cargo-texts` checks each producer's actual ELF audit data against the notice
inventory's runtime Cargo tree. Build-kind records remain available to native
binding checks. Runtime over-reports require an exact producer/root/package/version
record in `nix/notices/audit-exceptions.json` and an existing evidence heading.

Each record carries a dependency-context SHA-256: producer, target, build flags,
default/selected features, pinned toolchain, Cargo.lock, runtime packages, native
packages/features and audited package sets. Only local package versions, local
source directory paths, binary bytes and evidence locations are excluded. This
lets code edits and release-version bumps retain the same dependency evidence.
Registry/git versions, checksums, dependency changes and feature changes remain
significant. The graph is deliberately conservative: even an unrelated change in
the producer's lockfile requires inspection. It does not decide license terms.

## Inspect and reproduce

Use the existing `archive-notice-inputs`, `archive-notices`, `archive-notice-check`,
`release` and `release-without-accelerando` outputs. Obtain candidates through the
overseer when working as a pane agent; follow the repository's heavy-run rules.
No regeneration step runs automatically during an archive build.

Every Cargo notice report has an adjacent `audit/` directory, including when an
old exception key rejects the candidate. It contains `context.json`, `Cargo.lock`,
`runtime-crates.txt`, `native-packages.txt`, canonical `graph.json` and the actual
binary audit `observation.json`. The producer's `error.txt` preserves failures.
If Cargo fails before inspection, fix that failure first.

Run the existing `archive-notices` executable from the candidate's native build
inputs, or compile this single tool with `nix develop -c cargo build --locked -p
archive-notices`. Its PATH needs `rust-audit-info` and `sha256sum` from the pinned
Nix inputs. Set `TOOL` to that executable, `AUDIT` to the report's audit directory,
`PRODUCER_BIN` to the original producer's bin directory named in its derivation,
and `SCRATCH` to an isolated writable directory. Output paths must be absent.

```sh
"$TOOL" inspect-audit "$AUDIT/context.json" "$AUDIT/Cargo.lock" \
  "$PRODUCER_BIN" "$AUDIT/runtime-crates.txt" "$AUDIT/native-packages.txt" \
  "$SCRATCH/inspection"
cmp "$AUDIT/graph.json" "$SCRATCH/inspection/graph.json"
```

To repeat the real dependency-omission proof, select Zoxide's observation and
retain its root while removing covered `anyhow 1.0.98` from the inventory in memory:

```sh
"$TOOL" prove-audit-omission "$ZOXIDE_AUDIT/observation.json" \
  nix/notices/audit-exceptions.json anyhow 1.0.98
```

This first validates the actual audit and complete inventory, checks the saved
audit against the current binary, then requires rejection of the omitted audited
runtime dependency. The old root-only condition still accepts the retained root.
No inventory or binary is modified.

## Renew dependency evidence

Inspect both full and without-Accelerando candidates independently. Compare their
canonical graphs with the preceding accepted candidates. For each changed context,
repeat the vendor/feature-selected Cargo tree and actual build-log checks described
in `nix/notices/audit-evidence.md`; confirm every excepted identity remains a host
or unselected over-report. Update those evidence sections when necessary.

Set `FULL_INPUT` and `FALLBACK_INPUT` to the realized notice-input JSON files used
by those two candidates (the `generate` commands in their notice derivations name
them). Then propose keys with the same Rust tool:

```sh
"$TOOL" propose-audit-renewal nix/notices/audit-exceptions.json \
  "$FULL_INPUT" "$FALLBACK_INPUT" "$SCRATCH/audit-exceptions.proposed.json"
diff -u nix/notices/audit-exceptions.json "$SCRATCH/audit-exceptions.proposed.json"
```

The proposal command requires unchanged exact exception identities, matching
contexts in both variants and existing evidence headings. It refuses additions,
removals and mismatched variants. It writes a fresh file; it cannot approve the
evidence or change the checked-in inventory. Review the proposal and evidence,
then replace the data file explicitly. New or removed gaps require investigation
and a reviewed identity/evidence edit instead of renewal. Rebuild and check both
release routes; the full route independently retains its Accelerando grant gate.
