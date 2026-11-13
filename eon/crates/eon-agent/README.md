# rota

Picks which coding-agent subscription runs your next agent session.

You may pay for several coding-agent subscriptions: Claude Code, Codex,
Antigravity, OpenCode and others. rota looks at each one's remaining quota and
how many sessions it already runs, then starts the next agent session on a
backend that has room, with the model and effort level the task asks for.

## Usage

Build with `cargo build --release` (Linux, Rust 1.85 or later). Copy
`config.example.toml` to `~/.config/rota/config.toml` and replace its placeholder
paths, commands, models and example percentages.

```text
rota launch [--role implement|review] [--level medium|high] [--only BACKEND] [--dry-run] <prompt-file>
rota run --brief FILE --name NAME --timeout DURATION [--role implement|review] [--only BACKEND] [--cwd DIR] [--effort LEVEL] [--add-dir DIR]...
rota ps [--json] [--all]
rota slot run [--class NAME] [--quiet-exempt] [--front] -- COMMAND...
rota slot ls [--json]
rota retire NAME [--project P] [--bead ID] [--target-only] [--dry-run] [--json]
rota status [--json]
```

Launch defaults to `implement` and `high`, runs from the current folder and
replaces itself with the selected CLI. It checks backend guards, account checks,
quota limits and running processes before rotating to the next available backend.
Only implementers without `--only` can use the configured fallback, reported as
`FALLBACK`. Exit codes: 2 for invalid input/config, 3 for refusal, 1 for I/O or
launch failure. Status reports availability for both roles; a role a backend
runs only through `rota run` shows as `available for rota run`.

`--dry-run` prints the pick and shell command, showing the prompt as `$(cat file)`.
It does not write the state, log, lock or trusted settings, run preparation hooks,
or refresh an account. Quota and guard commands must themselves be read-only.
Status also skips refresh and writes no rota files.

`rota run` runs one headless agent session (a plan, review or investigation)
and waits for it. It checks guards, account checks, quota limits and caps like
`launch`, with no fallback, then runs the backend's `run` command for the role
in `--cwd` (default: the current folder). It does not apply `trusted_settings`
or `--level`. Its stdin is the brief file or
`/dev/null`, never rota's own. Output goes to a log under a run directory named
by the session, `NAME-N` with the first free `N`; rota prints the pick, session,
PID, log and report paths before the run starts. `--timeout` takes seconds or
`Ns`, `Nm`, `Nh`, at most 30 days, and counts from the moment the run starts.

`rota run` needs Linux 5.14 or newer with cgroup v2 and a systemd user manager,
systemd 254 or newer (tested on 255; systemd-run gets `--expand-environment=no`,
so arguments stay literal on 258 and later too). It starts the command with
`systemd-run --user --scope` in a transient unit `rota-ID-HEX.scope`, where HEX
is 128 random bits, so the unit's cgroup holds every process the run starts,
whatever their process group, session, parent or threads. Before it claims a
session, rota starts a throwaway scope that checks its own `cgroup.events` and
`cgroup.kill`; if that fails or takes more than 10 seconds, rota exits 1. A run
always gets rota's own `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS`, the
variables that reach the manager, and `[run]` may not unset or set them.

rota acts on the unit only once it has seen the run's main process inside it,
so a name that was already taken is never signalled. When the run's main
process exits, at the timeout, or when rota gets SIGINT, SIGTERM or SIGHUP, rota
sends SIGTERM and SIGCONT to every process in the unit's cgroup, waits up to 5
seconds for it to empty, then sends SIGKILL (and writes `cgroup.kill`) for up to
10 seconds. rota signals the processes directly through pidfds, never through
the systemd manager, so a stalled manager cannot delay cleanup, and it skips any
listed PID that has since left the unit. The run ends only when
`cgroup.events` confirms the cgroup is empty. If rota cannot confirm that by
then (processes stuck in uninterruptible I/O, or a cgroup or process membership
it cannot read) the run is abandoned: rota prints the PIDs it can see, records
them, and exits 125, never 0. If systemd-run fails before the command starts
(rota read the main process's cgroup and it is not the unit's), the outcome is
`failed` and rota exits 1; the log says why.
rota exits with the run's status: 128+N when a signal N ended it, 124 after a
timeout, 130 after an interruption and 125 when it abandoned processes. If the
final record write fails, rota exits 1. A refusal exits 3 and an unusable
option, backend or brief exits 2, as for `launch`, before anything starts. A run's own status can also be 1, 2 or 3: a run started
when the `session:` line was printed, and its record's `outcome` says how it
ended.

Each run is recorded in the session registry: `sessions/ID.json` beside
`state_file`, naming the run's `unit`, rewritten when the run ends with `finished` (Unix seconds),
`outcome` (`exited`, `signaled`, `timeout`, `interrupted`, `failed` or `abandoned`) and
`exit_code`; an abandoned run also lists `abandoned_pids`.
Run directories are `runs/ID/` beside `state_file`, holding `log` and the
`report.md` path given to the command. Readers ignore fields they do not know.
Writers hold `sessions/.lock` and merge their fields into the record as it is
on disk, so fields another command adds during a run are kept. A record that
exists but cannot be read or parsed is never replaced. `rota run` takes the lock
before the run starts; the final write waits for it at most 10 seconds.

`rota ps` lists the registry's sessions that have not finished, oldest
first; `--all` adds finished ones. Each has a state: `running` while its
process is alive, `dead` when the process is gone but the record never got
`finished` (rota itself was stopped), or `finished`. A process is alive while
`/proc/PID` exists and, for a record that names a `unit`, the process is in
that unit, so a reused PID never passes for the session. The text form prints
one line per session: ID, state, backend, role, PID, age, folder, log, and the
pane, outcome and exit code when the record has them. `--json` prints the
records as stored, each with `state`, `alive` and `age_seconds` (seconds since
`started`) added. ps reads the registry without its lock and writes nothing.
A record it cannot read or parse is reported on stderr, the others are still
printed, and rota exits 1.

`rota slot run` waits for a machine slot and runs the command with its original
arguments and inherited stdin, stdout and stderr. Use the same state directory
and slot configuration from each checkout to share slots. The oldest eligible
waiter gets each free slot. By default Nix builds and profile commands use slot
2 when two or more slots are configured. Without an explicit `--class`, rota
skips the queue for `nix eval`, `nix path-info` and `nix build --dry-run`,
including a single Nix command inside `sh -c`, `bash -c` or `timeout`. A
nested invocation runs directly when its inherited slot and grant token match
a locked holder record, including across PID namespaces. The record stores
the SHA-256 digest of the token. A command returns
its exit status, or 128 plus the signal that ended it. If rota cannot register
a ticket, it exits 3 and prints
the queue directory that needs write access.

`rota slot ls` lists holders by slot and waiters by priority and arrival time.
The JSON form returns `holders` and `waiters` arrays. Each waiter has a
`slots` array with its current eligibility. An empty array means quiet policy
is holding the waiter. The queue, holder records and `log.jsonl` live under
`slots/` beside `state_file`. The log records queue, grant and exit times.
`slot ls` reads `/proc/locks` and matches locks to files by device and inode.
This works on ext4. Filesystems that report different device numbers through
the two interfaces can omit locks from the listing. Listing works with
read-only state and exits 1 if the lock table cannot be read. Run it from the
host PID namespace to see locks across sandboxed agents. Slot locks stay with
the rota wrapper and close on exec. A descendant that
outlives the command cannot retain a slot lock.

## Configuration

`ROTA_CONFIG` selects a config file; otherwise rota uses
`$XDG_CONFIG_HOME/rota/config.toml`, defaulting to `~/.config/rota/config.toml`.
Paths in TOML are literal: use absolute paths; shell variables and `~` are not
expanded. Keep personal state and logs outside the repository.

- `backends` array order is round-robin order. `roles` controls eligibility;
  the fallback backend has an empty role list. Disabled backends never run checks.
- `commands.medium` and `commands.high` are argv arrays, with one whole argument
  `{prompt}` and optional `{cwd}` arguments. They run directly, without a shell.
  OpenCode uses `opencode mini -m MODEL#VARIANT --prompt {prompt}`.
- `env` adds environment variables to commands and probes, including proxy base
  URLs. `unset_env` removes inherited variables, for example on the main fallback.
- `guard` must exit successfully. Configure it to verify the command guard is
  installed **and hooked into that backend**. `required_files`, `account_check`
  and `excluded_roots` support credential/account checks.
- `quota.command` returns data for `quota.parser`: `agy` (tab-separated usage),
  `claude_text` (five-hour/seven-day used percentages, with `None` for missing),
  `claude_json` (usage endpoint JSON), `eon_codex` / `eon_claude` (workspace JSON),
  or `remaining_json` (object mapping window names to remaining percentages).
  `agy` reads the rows of one model group: `agy_group` names it and defaults to
  `Claude and GPT models`. The group needs a weekly row; its five-hour row is
  optional. A limit that names `five_hour` still refuses a group without that
  row unless `allow_missing` is set. A `disabled` value reads as 0% remaining,
  and each row's RFC 3339 reset time is kept when present.
  Reader commands have a configurable timeout covering output collection, and a
  1 MiB output limit. Optional `refresh` runs only after a failed real-launch probe,
  below the backend's cap, then retries once.
- `limits.implement` / `limits.review` compare `min_remaining` inclusively and
  `max_used` exclusively. `strictly_above_min` makes the minimum exclusive;
  `allow_missing` permits absent windows in an otherwise valid reading. `"*"`
  checks every returned window. Eon window keys are `DURATIONm/INDEX` to preserve
  scoped windows with the same duration. Eon snapshots must report `state = "fresh"`;
  missing, stale, error and unknown snapshots refuse. A refused role lists every
  window under its limit as `NAME_left=N%`, adding `(resets TIME)` when the reader
  reports a reset time, for example
  `quota limit: weekly_left=0% (resets 2030-01-08T14:00:00Z)`.
- A backend without a quota reader requires explicit `allow_unknown_quota = true`.
  An unsuccessful configured reader always refuses. The example disables OpenCode
  until a model and guard are chosen.
- `cap` scans Linux `/proc` for an executable basename and optional environment
  values; account-directory aliases are canonicalized. `exclude_args` omits usage
  probes. Counts include only the matching processes of the user running rota,
  regardless of role; a process that exits or changes owner mid-scan is
  skipped, and a live matching process whose environment or working directory
  cannot be read fails the check. A lock beside
  `state_file` serializes rota launches through exec; independently started CLIs
  are observed by the process scan rather than controlled by this lock. Caps are
  rechecked before refresh and exec, including resolved executable identities.
  Set `runtime_executables` to the actual binary paths behind wrappers that change
  argv[0]. For an interpreted CLI, set the executable to its interpreter basename
  and `script` to the absolute CLI script path to distinguish other interpreter jobs.
- `state_file` is written atomically. `log_file` receives JSON lines with time,
  folder, role, level, decision and readings, without prompt contents. Fallback and
  refusal are explicit log decisions. `trusted_settings` atomically appends the
  folder to `trustedWorkspaces`, preserving other JSON keys and the file mode.
  `prepare` is an optional list of checked commands run just before exec.
- `[backends.run]` makes a backend usable by `rota run`. `commands.implement`
  and `commands.review` are argv arrays; a role with a command can run headless
  even when `roles` leaves it out, and its quota limits apply. `{brief}`,
  `{cwd}`, `{report}` and `{effort}` are replaced inside any argument;
  `{add_dirs}` is a whole argument, never the first, that repeats `add_dir` (an
  argv fragment with one `{dir}` argument) once per `--add-dir`. `stdin` is `"brief"` or `"null"`;
  with `"null"` every command must name `{brief}`. `--effort` must be one of
  `efforts` and defaults to `default_effort`.
- `[run]` changes the environment of headless runs only, never of checks.
  `unset_env` removes inherited variables, where a trailing `*` matches a name
  prefix, then `[run.env]` sets variables, then the backend's `unset_env` and
  `env` apply. Use it to keep a run away from a desktop or terminal session.
- `[slots]` sets the slot `count` (default 2), the child nice increment
  (default 19), and child environment variables in `[slots.env]`.
  `[[slots.classes]]` lists classes in match order, each with `name`,
  `slots` and optional `match_argv`. A pattern matches consecutive arguments
  or words in the joined command, including a shell command argument. The
  final class omits `match_argv` and is the default. `--class` selects a class
  explicitly. `--front` jumps ordinary waiters and can use any active slot.
- During a cutover, `slots.lock_files` lists the existing absolute flock paths
  in slot order. Setting `slots.legacy_dir` requires this list, with the extra
  lock path last.
  Swap the old wrapper to a shim before starting native runs. The native
  scheduler honors outstanding legacy tickets, and both versions share the
  locks while the old wrappers drain. The shim translates the old overrides
  to `--front` and `--quiet-exempt`.
- Rota checks the optional `slots.legacy_dir` policies every five seconds.
  `extra-slot-until` enables slot `count + 1`. `one-slot-until` or a
  `slot2-stalled` marker narrows all classes to slot 1. `quiet-until` holds
  new grants except for `--quiet-exempt`, which also jumps ordinary waiters.
  The timed files contain Unix epoch seconds. A waiter under a directory in
  `yield-roots` yields on each slot to eligible higher-priority waiters for
  up to `yield-max-minutes` (default 30 minutes), then resumes FIFO by its
  original arrival time.

For a sandboxed Codex agent, add the directory containing rota's `state_file`
as a writable root in the active permission profile. These settings extend
an existing profile named `rota`.

```toml
[permissions.rota.workspace_roots]
"/PATH/TO/state" = true

[permissions.rota.filesystem.":workspace_roots"]
"." = "write"
```

Select the profile with `codex --permission-profile rota` and verify it before
cutover by running `rota slot run -- true` in the sandbox. See
[Codex permission profiles](https://learn.chatgpt.com/docs/permissions#define-and-select-a-profile).

Configuration and reader fixtures are tested with `cargo test`. Before committing,
run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`.
The slot namespace test requires Bubblewrap with working PID namespaces and fails
if its probe cannot run. On hosts that cannot run this probe,
`ROTA_TEST_ALLOW_NO_BWRAP=1 cargo test` permits an explicit skip with a diagnostic.

## Retiring build output

`rota retire NAME --project P --bead ID` frees finished agents' Cargo caches.
Configure `[projects.P]` with absolute `repo` and `agents_root`. Names identify
`NAME-target` and `NAME-work/target` directly under that root. Target contents
are removed only from plain directories with their own regular, non-symlink
Cargo `CACHEDIR.TAG`. An empty directory at either exact target path can be
finished without a tag using `rmdir`; non-empty untagged targets stay.
The worktree and work directory stay. Every regular work file survives except
contents of that direct tagged Cargo target. Full retire also removes store
symlinks beneath the work directory and the exact root files `NAME-prompt.md`
and `NAME-launch.sh`. Symlinks to other locations stay.

Full retire requires `[projects.P.done]`: an argv `command` (with `{bead}` and
optional `{repo}`/`{name}`), JSON `pointers`, and string `equals`. The command
runs in `repo`, with a 30-second timeout and 1 MiB output limit. The first
resolving pointer determines done status. See `config.example.toml` for a Beads
check. A sole configured project needs no `--project`.
When supplying `--bead`, the caller vouches that NAME and the bead identify the
same finished task.

`[cleanup] min_idle` defaults to `"2h"`; supported units are seconds, `s`, `m`
and `h`, up to 30 days. Targets with any recent or future modification time
stay unless the done check passes. Empty target directories can be finished
without waiting for `min_idle`; live-session and full-retire done guards still
apply. `store_prefixes` defaults to empty, keeping
all work symlinks; configured prefixes match absolute link text by whole path
components. Parent traversal in a link is refused.

`--target-only` leaves prompts, launchers and store links alone and permits
cleanup without a done bead. It retains the full live-session guard: any owned
process using the worktree, work directory or targets (cwd, executable,
`CARGO_TARGET_DIR`, open file descriptors or mapped files), or a matching
unfinished registry record whose PID exists, keeps the session. An inspection
failure, including unreadable process details, keeps files and exits 1. A failed
explicitly requested done check also exits 1.
The guard checks resolved paths plus cwd ancestor device/inode identities.
Bind-mount aliases of agent directories are unsupported and may escape
detection; do not rely on the guard to protect such a setup. `rota retire --help`
also describes this limit. Unreadable fd or maps details for an owned process
refuse cleanup with its PID named; other users' processes are skipped.
Run retire after stopping work for the session; a scan cannot prevent a later
build from starting.

`--dry-run` performs the guards without changing files or permissions. `--json`
reports each path's kind, disk bytes, outcome and reason, plus `freed_bytes`
(zero in dry-run). Exit codes are 0 for completed cleanup or nothing found,
3 for a policy keep, 2 for input/config errors and 1 for inspection/removal
errors. Partial removals stop at the first failure.

Readable read-only targets, including the top-level directory, are made writable
during cleanup. Interrupted cleanup can be retried after min_idle or with a
passing done check, under the usual guards, including after tag deletion when
only the empty directory remains.
Retire requires Linux 5.6 or newer.

## Design principles

- **Choose across backends, not across accounts.** rota chooses between
  providers. Several accounts of one provider are only a config option, used
  at your own responsibility under each provider's terms.
- **Concurrency caps per backend.** A backend can allow a set number of
  sessions at a time, for example one on a small plan.
- **Refuse rather than fall back silently.** When no backend fits, rota says
  so and exits. A fallback happens only when the config names one, and rota
  reports it.
- **Require the command guard.** rota refuses a backend whose configured
  command guard (for example [dcg](https://github.com/Dicklesworthstone/destructive_command_guard))
  is missing.
- **Proxy-backed backends.** A backend can point its CLI's base URL at a local
  proxy such as CLIProxyAPI, so adopting a proxy is a config entry.

Personal details (paths, accounts, thresholds) live in your config file. The
repository will ship only an example config.

## Related tools and how rota differs

These tools solve nearby problems well, and rota borrows ideas from several of
them. [docs/references.md](docs/references.md) has the details.

- [caam](https://github.com/Dicklesworthstone/coding_agent_account_manager)
  switches between accounts of each coding CLI, ranks them by live usage, and
  can switch account after a rate limit. rota chooses between backends instead,
  and adds concurrency caps and a model or effort level per task.
- [ntm](https://github.com/Dicklesworthstone/ntm) runs many agents in tmux
  panes and can ask caam to choose an account before it starts a pane. rota is
  a single launcher that any orchestrator can call.
- [T3 Code](https://github.com/pingdotgg/t3code) is a GUI for coding agents
  that shows usage limits across providers, but by design does not route on
  them. rota routes, and borrows its quota-reading approach.
- [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) exposes
  subscription CLIs as an API and pools requests across accounts. rota works
  per session rather than per request, and can use the proxy as a backend.
- [claude-swap](https://github.com/realiti4/claude-swap) and
  [clauth](https://github.com/uwuclxdy/clauth) manage several Claude Code
  accounts. rota covers other providers too.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
