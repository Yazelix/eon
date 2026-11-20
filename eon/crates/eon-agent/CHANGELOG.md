# Changelog

Changes to rota (`rota-cli`) that a user of its commands or configuration can
see.

## Unreleased

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
