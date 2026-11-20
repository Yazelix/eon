# Unsafe code

Most of Eon's Rust code forbids `unsafe`. The rest keeps it in a few private modules. They call
the operating system where neither the standard library nor
[rustix](https://github.com/bytecodealliance/rustix) has a safe call. They also call macOS
frameworks and the terminal engine's formatter.

## Rules

- **Manifests.** Every package's `Cargo.toml` sets `unsafe_code` to `forbid`. A package with one
  of the files below sets it to `deny` instead, and also denies `unsafe_op_in_unsafe_fn`. Every
  package denies Clippy's `undocumented_unsafe_blocks`.
- **Standalone files.** A Rust file that is compiled on its own, such as a Nix check or a tool,
  forbids `unsafe_code` at its root.
- **Allowances.** Only the files below may allow `unsafe_code`. Each allowance covers exactly
  what its row names: a module, a function, an item, or one statement with a single unsafe
  block.
- **Safety notes.** Every unsafe block has a `// SAFETY:` comment saying why it is sound. Every
  unsafe function and extern block has a safety note.
- **Enforcement.** `nix build .#checks.x86_64-linux.unsafe-policy` checks this page against the
  source, including code for every platform and test configuration. The check fails on a new
  unsafe block, a new or wider allowance, a changed count, or a lint lowered in build
  configuration. CI runs it on every push to `edge`.
- **Editing this page.** When you change a row, review its reason too.

## Where unsafe code remains

Sites count unsafe blocks, unsafe functions and extern blocks. "Test" marks code that is built
only for tests.

### Shipped code

| File | Sites | Allowances | Why |
| --- | --- | --- | --- |
| `orbit/src/lib.rs` | none | mod platform | Declares Orbit's platform module. |
| `orbit/src/platform.rs` | 12 blocks, 1 test block | none | Setup in the forked child before `exec`, which needs `pre_exec`. The termination signal handler: installing it, saving errno and waking the main loop. macOS process and peer queries (libproc, `getpeereid`). Linux peer credentials and the macOS foreground group, which can be PID 0; rustix's safe types cannot hold PID 0. |
| `orbit/src/visible.rs` | 1 block | fn row_text | libghostty-vt formats a chosen range only through an unsafe call. The safe route would change the terminal's shared selection. |
| `runtime/crates/eon-runtime/src/lib.rs` | none | mod platform | Declares the Runtime's platform module. |
| `runtime/crates/eon-runtime/src/platform.rs` | 19 blocks, 2 test blocks | none | Signal handlers and dispositions, `_exit` from a handler, and a new session before `exec`. Peer credentials. A macOS monotonic clock and libproc queries. `localtime_r`. The unsent-bytes `ioctl`. The home-directory lookup (`getpwuid_r`). |
| `eon/crates/eon/src/main.rs` | 1 block | mod platform, in fn main: 1 statement | Declares the platform module, and clears launch variables once at single-threaded startup. |
| `eon/crates/eon/src/launch.rs` | 1 block, 1 unsafe fn | fn clear_capture | Removes those variables. `std::env::remove_var` is unsafe because another thread could read the environment at the same time. |
| `eon/crates/eon/src/platform.rs` | 5 blocks | none | Signal handlers, and a parent-death signal set up before `exec`. |
| `eon/crates/eon-agent/src/lib.rs` | none | mod platform | Declares rota's platform module. |
| `eon/crates/eon-agent/src/platform.rs` | 10 blocks | none | Signal handlers, and terminal foreground setup and a nice level before `exec`. A `close` that reports its error. An effective-ID access check on Linux older than 5.8. `pidfd_send_signal` for signal 0 and realtime signals. |
| `venus/src/system.rs` | 8 blocks | fn cpu_load, fn cpu_name, fn load_average, fn memory, fn per_cpu_load, fn port | macOS Mach and sysctl calls for the bar's CPU and memory readings. |
| `launcher/src/bin/eon-macos-launcher.rs` | 1 extern block | extern block | Declares the C library's `getuid` and `kill` as safe functions: they take and return only integers. |

### Tests

| File | Sites | Allowances | Why |
| --- | --- | --- | --- |
| `orbit/tests/lifecycle.rs` | none | mod native_process | Declares the shared process helpers. |
| `orbit/tests/real_pty/main.rs` | none | mod native_process | Declares the shared process helpers. |
| `orbit/tests/support/process.rs` | 4 test blocks | none | macOS libproc and `mkfifo`, `getrusage`, and Linux `PTRACE_SEIZE`. |
| `eon/crates/eon/tests/process/mod.rs` | 5 test blocks | module | Session and terminal setup before `exec`, plus macOS libproc and `sysctl` queries. |
| `eon/crates/eon-agent/test-host/tests/slot.rs` | 1 test block | mod platform | Terminal setup before `exec`. |

### Tools (not shipped)

| File | Sites | Allowances | Why |
| --- | --- | --- | --- |
| `eon/tools/eon-bench/src/lib.rs` | none | mod sys | Declares the benchmark's system module. |
| `eon/tools/eon-bench/src/sys.rs` | 7 blocks, 2 test blocks | none | Child setup before `exec`, signal handlers, `getrusage`, and the macOS load average. |
| `eon/tools/mac-proof/src/mac.rs` | 33 blocks, 1 extern block | extern block, in fn backdrop: 2 statements, in fn configure: 4 statements, in fn configure_mirror: 6 statements, in fn displays: 1 statement, in fn modes: 3 statements, in fn virtual_display: 15 statements, in fn windows: 2 statements | The macOS proof harness: Objective-C messages to the private virtual-display classes, CoreGraphics display configuration, and libdispatch. |

## Kept by choice

Some of these sites could use a safe API from a crate that Eon does not depend on directly:

- **Orbit's termination handler**: its errno save and its wake write could use `errno`, or the
  handler could use `signal-hook`.
- **Runtime's `localtime_r`**: `jiff` could replace it.
- **Runtime's home-directory lookup and Orbit's test helpers**: `nix` could replace them.
- **rota's signal handlers**: `signal-hook` could replace them.
- **The macOS launcher**: rustix could replace its two declarations, but the launcher has no
  dependencies.
- **mac-proof**: its array construction and its libdispatch queue could use `objc2-foundation`
  and `dispatch2`.

## How it is tested

The Linux code in these files runs under its crate's tests before every change lands. The macOS
code builds in the macOS workflow on every push to `edge` that touches it, and its workspace
tests run there when a maintainer starts them. Orbit's real-PTY tests run the forked child's
setup on real terminals. Clippy runs with the lints above before every change lands.
