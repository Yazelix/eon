# Development checks

## Architecture and evidence

- [`docs/CONTRACTS.md`](CONTRACTS.md) indexes Venus subsystem behavior and proof.
- [`docs/REFERENCES.md`](REFERENCES.md) records the exact architectural
  evidence used by the implementation gate.
- [`docs/CRATES.md`](CRATES.md) records the measured dependency selection,
  owner seams, and rejected alternatives.
- The [memory comparison](benchmarks/venus-memory-2026-09-08.md) records
  the cell-buffer allocation savings and their environment limits.

```sh
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The application regression also needs an isolated native Wayland display and
Vulkan renderer. Point `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` at that display,
and `TMPDIR` at disposable proof storage, then run:

```sh
timeout 30s cargo test --locked --bin yazelix-venus live_output_keeps_application_input_admitted_before_repaint -- --ignored
```
