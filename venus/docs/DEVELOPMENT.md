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
nix develop -c eon-test -p yazelix-venus
cargo clippy --locked --all-targets -- -D warnings
```

The application regression also needs an isolated native Wayland display and
Vulkan renderer, so it runs outside `eon-test`, which hides every display.
Point `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` at that display, and `TMPDIR` at
disposable proof storage, then run:

```sh
timeout 30s cargo test --locked --bin yazelix-venus live_output_keeps_application_input_admitted_before_repaint -- --ignored
```

The image regression runs the same way. It checks that oversize image uploads are
refused without failing the renderer, then shows a test image beside a refused image's
marker in a pane, in a popup and under a notice for 2.5 seconds each, so a capture of
that display can confirm the band:

```sh
timeout 60s cargo test --locked --lib images_draw_in_panes_and_popups_and_refusals_keep_the_renderer -- --ignored
```
