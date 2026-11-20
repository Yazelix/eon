# Changelog

Changes to rota (`rota-cli`) that a user of its commands or configuration can
see.

## Unreleased

- `slot`: `[slots.boat]` and `slots.local_runner` need `eon agent`. When one
  would act (a permitted Boat shape, a nonempty runner), `rota slot` warns
  that it needs `eon agent` and runs every command on a local slot. `slot ls
  --json` has no `boat_lease_held`, `boat_lease_pid`, `boat` or `boat_error`.
  rota keeps `[slots.boat]` as an opaque table for `eon agent`: it no longer
  checks the table's values or warns about its removed `snapshot` key.
- The local runner handoff variables are `ROTA_SLOT_QUEUED_NS`,
  `ROTA_SLOT_CLASS`, `ROTA_SLOT_RUN_KEY` and `ROTA_SLOT_DELEGATE`; the names
  Eon used before have no alias.

- Project profiles: `[projects.NAME]` caps live agents in total (`max`), per
  role (`roles.ROLE.max`) and per backend (`backends.BACKEND.max`). It runs a
  role's `guard` in the working directory, and it inserts a role's `overlay`
  (keyed by command executable) at a command's `{overlay}`.
  - `launch` and `run` select a project with `--project NAME` or by the working
    directory's repository (including linked worktrees). The popup never
    selects one.
  - Admissions record their process (PID and start time) under `agents/`
    beside `state_file`, and caps count records whose process still runs.
  - Holds name the live PIDs or the guard's first stderr line, and decision
    records gain `project` and `hold`.
  - Leads are never capped.
- `launch --name NAME` fills `{name}`. `launch --add-dir DIR` repeats the
  backend's new `add_dir` template at `{add_dirs}`. `run` fills `{name}` with
  its `--name`.
  - A backend whose command needs an option that was not given is skipped with
    its reason, and the command is refused (exit 2) when none remains.
  - Launch commands fill `{cwd}` inside any argument, not only as a whole
    argument.
  - Configuration refuses `{prompt}` inside another argument, `{dir}` outside
    an `add_dir` template, and any placeholder in the program.
  - `--popup` refuses `--project`, `--name` and `--add-dir`.
- Upgrade every binary that reads a shared configuration before adding these
  keys: older builds refuse them.

- `parser = "codexbar"` reads Codex and Claude five-hour and weekly limits
  and Claude's named model weeklies through CodexBar CLI. `status --json`
  includes the provider identity when available. Upgrade every binary using
  a shared config before enabling it. When switching from native quota,
  replace duration-based keys in limits and `absent_windows` with the
  CodexBar keys documented in [Configuration](README.md#configuration).

- Native quota sources: a host reading may report where each provider's facts
  came from (`quota_source`) and why none has a provider (`quota_unreadable`).
  The source shows as `reading.source.via` in `status --json` and as
  `via SOURCE` at the end of the launch candidate line on stderr, also when
  the source's facts cannot be used; a reading without one keeps the previous
  shape. A native provider failure without usable facts names its reason.
- Ordered admission: `launch` and `run` take the first available backend in
  configured order and read candidates one at a time, stopping at the first
  that admits; unread ones show `not checked`. Nothing rotates and nothing is
  written at `state_file`.
- One admission per quota observation: a backend that sets limits needs a
  reading with `observed_at` (`claude_text` `observed_at=MS`, top-level
  `"observed_at"` in `claude_json` and `remaining_json`, the native snapshot's
  time) that is not in the future, no older than `[backends.quota]
  max_age_seconds` (default 600; 0 is a configuration error), and not yet
  spent on its account.
  Admissions record it in `admissions.json` beside `state_file`; a repeat or a
  reading too old runs `refresh` once and reads again, then holds. A native provider failure beside
  retained facts holds. A failed ledger write refuses the launch (exit 1).
- Windows: `[backends.quota] absent_windows` declares missing windows; a
  limit naming another missing window holds. `DURATIONm` keys match every
  native `DURATIONm/INDEX` window.
- Configuration errors: `allow_unknown_quota` with limits, limits without
  `account`, `limits.lead`, `run.commands.lead`, invalid `absent_windows`.
- Leads: `launch --role lead` for backends listing the `lead` role reads no
  quota, consumes nothing and checks no limits; `run --role lead` exits 2.
- Refusals: every quota hold names the account; a limit hold names each
  window under its limit, its value, the rule and its threshold
  (`quota limit on account A: five_hour used 55% (max_used < 50%)`).
- Output: readings carry `quota_problem` and `source` (account and
  `observed_at`); `status` adds a `lead` column.
