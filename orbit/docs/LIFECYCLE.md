# Session lifecycle

## Run the server

`yazelix-orbit serve` runs one Session in the foreground. Inside `nix develop`,
from `orbit/`:

```sh
tic -x terminfo/eon.terminfo
cargo run --locked -- serve
```

The first command installs Orbit's `eon` terminal identity in the current
user's terminfo database. Eon distribution remains responsible for installing
the same entry in its runtime closure. Orbit has no client of its own;
[Venus](../../venus/README.md) attaches to its socket.

The server is two processes ([HOST.md](HOST.md)). The Rust host owns the PTY,
child process, terminal state, presentation extraction, input encoding,
resize, and terminal-generated replies in one event loop, and asks the Bend
worker, which it starts and contains, for every decision. Clients and the
server use only the bounded ORBS v14 codec. An exact-version
attachment receives typed outcomes, canonical presentation frames,
acknowledgements, bounded failures, and session exit. Client messages carry
semantic key, mouse, focus, arbitrary paste, full surface-resize, and
revision-bound selection plus revision-carrying vertical-preview and bounded
signed-scroll events, never raw PTY output.

One separately negotiated observer receives only an acknowledgement, bounded
coalescible title/CWD metadata at exact presentation revisions, failures, and
session exit.

### Local socket and management

The default socket is `$XDG_RUNTIME_DIR/yazelix-orbit/orbit.sock`, falling back
to `/tmp/yazelix-orbit-$UID/orbit.sock`. Its directory is private and user-owned;
the socket mode is `0600`. Orbit:

- removes a connection-refused stale socket but never replaces a non-socket path;
- serializes concurrent stale-socket claims and admits one input client and one
  metadata observer, rejecting excess roles with `Busy`;
- retains the last PTY size while detached, reaps an exited child, and removes
  the owned socket after normal or signal-driven shutdown.

On Linux and Apple Silicon macOS, the private same-boot management owner is
enabled with `serve SOCKET --management-v4 SESSION_ID RUN_ID COMPONENT_GENERATION -- COMMAND`.
It derives `SOCKET.management` and `SOCKET.record`, publishes the live record
only after the PTY, terminal owner, and both sockets are usable, and permits one
UID- and identity-validated management lease. A launcher may retain an empty
owned `SOCKET.record` inode. Orbit wins its lock and publishes Live, or fails
startup without publishing when the launcher holds it. Lease loss does not stop
the Session; only explicit Stop does. Natural exit or successful Stop replaces
the live record with a typed terminal tombstone and removes only the exact
sockets. Messages and records are limited
to 4 KiB, except a visible-text reply of up to 1 MiB of text and a lease paste
of up to 64 KiB, identities to 128 UTF-8 bytes, and negotiation to one second.
The interactive client of a managed run is only the process the lease holder
names. Eon and Eonova consume this owner for accepted same-boot recovery and
owner-routed Stop; this mode does not promise logout, reboot, machine-restart,
topology, or same-UID isolation.

### Shutdown

Orbit starts ordinary and managed PTY Sessions on x86_64 Linux and Apple
Silicon macOS with user process permissions and no service-manager requirement.
SIGINT, SIGTERM, and SIGHUP send SIGHUP to the initial PTY process group and the
terminal's current foreground group; authorized management Stop uses the same
path on both targets. Orbit gives the child up to 500 ms to exit. If it
remains unreaped, Orbit sends SIGKILL to its still-stable initial group and
re-reads the terminal's foreground group before escalating it. If the child
exits during the grace period, Orbit sends no later signal to the cached
foreground-group number. After a signal, Orbit requires the direct child to be
reaped within two seconds before reporting success. After a management Stop, a
child not reaped by then keeps the run and its live record until it is reaped,
and the Stop then succeeds with its outcome. A child that had already exited before
shutdown is reaped without signaling its recyclable numeric identity.

This is terminal cleanup, not whole-process-tree ownership. A deliberately
detached process, a non-foreground group outside the initial group, or a
SIGHUP-ignoring foreground job whose shell exits during the grace period may
survive. Orbit does not scan process names, ancestry, session IDs, or procfs to
find it. Client disconnection remains non-destructive and separate from
explicit Session stop. The special detached and foreground-group topology is
Linux-proved only; [ORB-C14](CONTRACTS.md) records the macOS proof status.

If the Bend worker dies or stops answering, the host stops the child the same
way and removes the socket pathname; [BEHAVIOR.md](BEHAVIOR.md#fatal-exit-boundary)
states the bounds and the checks that cover them.

### Platform seam and checks

The proof pins `libc` 0.2.189 for the small Unix `openpty`, controlling-terminal,
poll, resize, and signal boundary. This avoids `portable-pty` 0.9.0's general
cross-platform abstraction and dependency stack. `rustix` 1.1.4 exposes the
lower-level PTY calls, but a complete open-PTY path also needs
`rustix-openpty` 0.2.0. On Linux that alternative adds `rustix-openpty`,
`rustix`, and `linux-raw-sys` beyond packages already present in Orbit. Orbit
keeps Cargo unchanged for this proof. `pty-process` 0.5.3 is the narrower
credible alternative: a focused scratch build passed the canonical checks and
reduced owned Rust by 44 lines while adding `pty-process`, `rustix`, and
`linux-raw-sys` to the normal Linux graph. Adopting it remains a separate crate
decision.

One seam, [src/platform.rs](../src/platform.rs), holds the host's Unix calls:
pipes, polling, termination signals, the PTY and its window size, the worker's
and child's setup between fork and exec, process-group signals, the
zero-length send, peer credentials, process-start identities and the
background count's process-tree reads. The
mechanisms that use them keep their policy: [src/pty.rs](../src/pty.rs) the
PTY child, [src/socket.rs](../src/socket.rs) the socket path, permissions and
peers, and [src/management.rs](../src/management.rs) the managed records. Only
pipe creation, the death signal of the worker and child, a zero-length send's
flags, peer credentials, process-start identities and the background count's
process-tree reads diverge by OS, all inside the seam
([BEHAVIOR.md](BEHAVIOR.md#macos-evidence)). The Bend worker sees only facts
and never a `libc` value.

The subprocess checks in [`tests/lifecycle.rs`](../tests/lifecycle.rs),
which run unmodified against this server, synchronize on typed responses,
frames, and PTY state. With bounded timeouts they cover:

- negotiation, handshake order, input, paste, mouse, focus, and resize;
- selection over soft wraps and wide text, exact copy after later output, and
  selection cleanup on client loss;
- continued foreground output after detach, reconnection to the same shell PID,
  and second-client rejection;
- socket identity, permissions, stale claims, child exit, signal cleanup,
  process-group reaping, and intentional foreground-job survival;
- recovery when a live child closes every PTY descriptor and later reopens it.

The real-PTY checks in [BEHAVIOR.md](BEHAVIOR.md) cover the same ground in
finer steps, and the Bend laws prove the lifecycle decisions.

## Initial boundaries

- ordinary and managed Orbit PTY Sessions are proved on x86_64 Linux;
  [ORB-C14](CONTRACTS.md) records Apple Silicon macOS, and Venus/Eon
  composition, packaging, and distribution remain unsupported on macOS as
  separate downstream scope
- local Unix socket transport
- one terminal session, one input-capable client, and one read-only metadata
  observer
- one product binary with its Bend worker, one canonical protocol library, and
  one isolated repository-governance tool; only the host owns terminal and PTY
  authority, and only the worker decides
- [Venus](../../venus/README.md) owns graphical implementation; the
  [contract index](CONTRACTS.md) records the producer and consumer proofs
- no multiplayer, remote transport, Zellij compatibility, layouts, tabs,
  plugins, or configuration framework
- Mars and Mars Next may inform behavior, but their code and architecture are
  not foundations for Orbit
