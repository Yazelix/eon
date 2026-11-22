//! Stream parsers for Claude Code, Codex CLI, and `agy` into one run-event model.
//!
//! Supported NDJSON stdout streams: `claude --output-format stream-json --verbose`
//! ([`Harness::ClaudeCode`]), `codex exec --json` / `codex exec resume <id> --json`
//! ([`Harness::Codex`]), and `agy --output-format stream-json` ([`Harness::Agy`]).
//! Unknown event kinds, subtypes, and item/step types are skipped and counted in `unknown_events`;
//! malformed JSON or corrupt typed fields return an error.
//!
//! ## Accounting and capability boundaries
//!
//! - **Claude Code:** Assistant frames may repeat across content blocks without `requestId` and
//!   are deduplicated by `message.id`. Subagent frames (`parent_tool_use_id != null`) are excluded
//!   from parent `context_tokens`. Because assistant `output_tokens` are placeholders and
//!   `result.usage` omits subagents/auxiliary calls, authoritative whole-tree totals come from
//!   non-empty `result.modelUsage` summed across all models (subtracting `thinkingTokens` from
//!   `outputTokens` so buckets stay disjoint). Claude Code v2.1.277+ restores prior session spend
//!   into `modelUsage` on `--resume`, so [`RunUsage`] tracks authoritative Claude totals as
//!   cumulative session snapshots keyed by `session_id`. Any fallback (`result.usage` + subagent
//!   usage, or observed assistant steps on EOF or a crash emitting an empty `modelUsage` and
//!   zeroed `usage`) is per-invocation [`UsageAccounting::Partial`] and stays distinct from
//!   session snapshots in [`RunUsage`].
//! - **Codex CLI:** Upstream `codex exec --json` (`exec_events.rs`,
//!   `event_processor_with_jsonl_output.rs`) emits accumulated thread usage on `turn.completed`,
//!   not single-request context occupancy, and defines no compaction item/event. Both
//!   `cached_input_tokens` and `cache_write_input_tokens` are subtracted from `input_tokens`, and
//!   `reasoning_output_tokens` is subtracted from `output_tokens`. [`StreamEvent::Step`] reports
//!   `context_tokens: None` and keeps `turn.completed.usage` as a snapshot;
//!   [`StreamEvent::Compaction`] is never synthesized (runner policy: Codex lanes set
//!   `context_handoff_tokens: null` and rely on Codex's own compaction, wall cap, stall timeout,
//!   and early-stop resume). [`FinalStatus::Success`] requires `turn.completed` with no later
//!   known in-flight turn or error; unknown `item.completed` frames are counted and skipped
//!   without altering turn state, and EOF mid-turn emits no [`StreamEvent::Final`]. Across
//!   `codex exec resume <id>`, [`RunUsage`] supersedes prior snapshots on the same session ID.
//! - **`agy`:** Completed `agent_response` steps report single-request `context_tokens` and
//!   subtract `thinking_tokens` from `output_tokens` so output and reasoning buckets stay
//!   non-overlapping. Completed `checkpoint` steps emit [`StreamEvent::Compaction`] and
//!   accumulate any nonzero checkpoint usage once into `cumulative_usage` without emitting a
//!   parent `Step`, so subsequent steps and EOF-before-`result` partial totals include compaction
//!   cost. Resumed `--conversation` invocations report per-invocation usage, so [`RunUsage`] adds.

use crate::{Error, Result, fail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Cursor};

type JsonObj = Map<String, Value>;

/// Harness NDJSON wire protocol decoded by [`StreamParser`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Agy,
}

impl Harness {
    /// Whether this harness's stream reports per-step parent context occupancy.
    pub const fn reports_context_tokens(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Agy)
    }

    /// Whether this harness's stream emits an observable compaction signal.
    pub const fn reports_compaction(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Agy)
    }
}

/// Non-overlapping token counts matching [`crate::rows::StageTokens`]'s five token fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl TokenUsage {
    /// Single-request context occupancy (`input + cache_read + cache_write`).
    /// Only meaningful for a single model call, not cumulative multi-request totals.
    pub const fn context_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    pub fn saturating_add(self, other: Self) -> Self {
        self.zip(other, u64::saturating_add)
    }

    pub fn saturating_sub(self, other: Self) -> Self {
        self.zip(other, u64::saturating_sub)
    }

    fn max_fields(self, other: Self) -> Self {
        self.zip(other, u64::max)
    }

    fn zip(self, other: Self, op: fn(u64, u64) -> u64) -> Self {
        Self {
            input: op(self.input, other.input),
            output: op(self.output, other.output),
            cache_read: op(self.cache_read, other.cache_read),
            cache_write: op(self.cache_write, other.cache_write),
            reasoning: op(self.reasoning, other.reasoning),
        }
    }

    const fn ge_all(self, other: Self) -> bool {
        self.input >= other.input
            && self.output >= other.output
            && self.cache_read >= other.cache_read
            && self.cache_write >= other.cache_write
            && self.reasoning >= other.reasoning
    }

    const fn is_zero(self) -> bool {
        self.input == 0
            && self.output == 0
            && self.cache_read == 0
            && self.cache_write == 0
            && self.reasoning == 0
    }
}

/// Whether a [`StreamEvent::UsageTotal`] or [`RunUsage`] is authoritative or partial/fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageAccounting {
    #[default]
    Authoritative,
    Partial,
}

/// Terminal status of one harness invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalStatus {
    Success,
    MaxTurns,
    Canceled,
    Error,
}

/// Normalized run event yielded by [`StreamParser`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    Session {
        id: String,
    },
    Step {
        context_tokens: Option<u64>,
        cumulative_usage: TokenUsage,
    },
    Compaction,
    Denied {
        action_kind: String,
    },
    Final {
        text: String,
        status: FinalStatus,
    },
    UsageTotal {
        usage: TokenUsage,
        accounting: UsageAccounting,
    },
}

/// Batch result of [`parse_reader`] and [`parse_str`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedStream {
    pub events: Vec<StreamEvent>,
    pub unknown_events: u64,
}

/// Accumulates run-level token cost across initial, resumed, and handed-off invocations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunUsage {
    total: TokenUsage,
    accounting: UsageAccounting,
    sessions: HashMap<(Harness, String), TokenUsage>,
}

impl RunUsage {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn total(&self) -> TokenUsage {
        self.total
    }

    pub const fn accounting(&self) -> UsageAccounting {
        self.accounting
    }

    /// Incorporates one invocation's total usage according to `harness`'s resume semantics.
    ///
    /// - [`Harness::Codex`] — every invocation reports a cumulative thread snapshot keyed by
    ///   `session_id`.
    /// - [`Harness::ClaudeCode`] — [`UsageAccounting::Authoritative`] (`result.modelUsage`)
    ///   reports a cumulative session snapshot keyed by `session_id`, whereas
    ///   [`UsageAccounting::Partial`] fallbacks (`result.usage` or assistant steps) are
    ///   per-invocation totals added distinctly while advancing the session's accounted floor.
    /// - [`Harness::Agy`] — resumed invocations report per-invocation usage and are added.
    pub fn record_invocation(
        &mut self,
        harness: Harness,
        session_id: Option<&str>,
        usage: TokenUsage,
        accounting: UsageAccounting,
    ) {
        if accounting == UsageAccounting::Partial {
            self.accounting = UsageAccounting::Partial;
        }
        match (harness, accounting) {
            (Harness::Agy, _) => self.total = self.total.saturating_add(usage),
            (Harness::ClaudeCode, UsageAccounting::Partial) => {
                self.total = self.total.saturating_add(usage);
                if let Some(id) = session_id.filter(|id| !id.is_empty()) {
                    let entry = self.sessions.entry((harness, id.to_owned())).or_default();
                    *entry = entry.saturating_add(usage);
                }
            }
            (Harness::ClaudeCode, UsageAccounting::Authoritative) | (Harness::Codex, _) => {
                self.record_snapshot(harness, session_id, usage);
            }
        }
    }

    fn record_snapshot(&mut self, harness: Harness, session_id: Option<&str>, usage: TokenUsage) {
        let Some(id) = session_id.filter(|id| !id.is_empty()) else {
            self.total = self.total.saturating_add(usage);
            self.accounting = UsageAccounting::Partial;
            return;
        };
        let key = (harness, id.to_owned());
        if let Some(prev) = self.sessions.get_mut(&key) {
            let next = if usage.ge_all(*prev) {
                usage
            } else {
                self.accounting = UsageAccounting::Partial;
                prev.max_fields(usage)
            };
            self.total = self.total.saturating_add(next.saturating_sub(*prev));
            *prev = next;
        } else {
            self.sessions.insert(key, usage);
            self.total = self.total.saturating_add(usage);
        }
    }

    /// Incorporates a parsed invocation stream. Streams without [`StreamEvent::UsageTotal`]
    /// downgrade accounting to [`UsageAccounting::Partial`].
    pub fn record_stream(&mut self, harness: Harness, stream: &ParsedStream) {
        let (mut session_id, mut usage_total) = (None, None);
        for event in &stream.events {
            match event {
                StreamEvent::Session { id } => session_id = Some(id.as_str()),
                StreamEvent::UsageTotal { usage, accounting } => {
                    usage_total = Some((*usage, *accounting));
                }
                _ => {}
            }
        }
        if let Some((usage, accounting)) = usage_total {
            self.record_invocation(harness, session_id, usage, accounting);
        } else {
            self.accounting = UsageAccounting::Partial;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CodexTurn {
    #[default]
    Idle,
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Default)]
struct ParserState {
    line_number: u64,
    unknown_events: u64,
    last_session_id: Option<String>,
    cumulative_usage: TokenUsage,
    saw_usage: bool,
    emitted_usage_total: bool,
    finished: bool,
    claude_seen_messages: HashSet<String>,
    claude_sub_usage: TokenUsage,
    agy_seen_usage: HashSet<u64>,
    agy_seen_checkpoints: HashSet<u64>,
    codex_turn: CodexTurn,
    codex_last_text: Option<String>,
    codex_saw_error: bool,
    codex_usage_degraded: bool,
    codex_last_turn_missing_usage: bool,
}

/// Stateful incremental NDJSON parser for one harness invocation stream.
#[derive(Clone, Debug)]
pub struct StreamParser {
    harness: Harness,
    s: ParserState,
}

impl StreamParser {
    pub fn new(harness: Harness) -> Self {
        Self {
            harness,
            s: ParserState::default(),
        }
    }

    pub const fn harness(&self) -> Harness {
        self.harness
    }

    pub const fn unknown_events(&self) -> u64 {
        self.s.unknown_events
    }

    /// Running token usage observed so far, updated to the final total once
    /// [`StreamEvent::UsageTotal`] is emitted.
    pub const fn cumulative_usage(&self) -> TokenUsage {
        self.s.cumulative_usage
    }

    /// Parses one NDJSON line, updating state and returning any emitted events.
    pub fn feed_line(&mut self, line: &str) -> Result<Vec<StreamEvent>> {
        if self.s.finished {
            fail!("cannot feed lines after finish()");
        }
        self.s.line_number += 1;
        let line_no = self.s.line_number;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|error| Error(format!("line {line_no}: {error}")))?;
        let Some(obj) = value.as_object() else {
            fail!("line {line_no}: expected a JSON object");
        };
        let mut out = Vec::new();
        match self.harness {
            Harness::ClaudeCode => self.parse_claude(obj, line_no, &mut out)?,
            Harness::Codex => self.parse_codex(obj, line_no, &mut out)?,
            Harness::Agy => self.parse_agy(obj, line_no, &mut out)?,
        }
        Ok(out)
    }

    /// Signals EOF and flushes any EOF-gated events. Idempotent on repeat calls.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        if self.s.finished {
            return Vec::new();
        }
        self.s.finished = true;
        let mut out = Vec::new();
        match self.harness {
            Harness::ClaudeCode | Harness::Agy => {
                if !self.s.emitted_usage_total && self.s.saw_usage {
                    self.emit_usage(self.s.cumulative_usage, UsageAccounting::Partial, &mut out);
                }
            }
            Harness::Codex => {
                let (status, acc) = match (self.s.codex_turn, self.s.codex_saw_error) {
                    (CodexTurn::Completed, false) => (
                        Some(FinalStatus::Success),
                        if self.s.codex_usage_degraded || self.s.codex_last_turn_missing_usage {
                            UsageAccounting::Partial
                        } else {
                            UsageAccounting::Authoritative
                        },
                    ),
                    (CodexTurn::Failed, _) | (_, true) => {
                        (Some(FinalStatus::Error), UsageAccounting::Partial)
                    }
                    _ => (None, UsageAccounting::Partial),
                };
                if let Some(status) = status {
                    out.push(StreamEvent::Final {
                        text: self.s.codex_last_text.take().unwrap_or_default(),
                        status,
                    });
                }
                if self.s.saw_usage {
                    self.emit_usage(self.s.cumulative_usage, acc, &mut out);
                }
            }
        }
        out
    }

    fn emit_session(&mut self, id: Option<&str>, out: &mut Vec<StreamEvent>) {
        if let Some(id) = id.filter(|id| !id.is_empty())
            && self.s.last_session_id.as_deref() != Some(id)
        {
            self.s.last_session_id = Some(id.to_owned());
            out.push(StreamEvent::Session { id: id.to_owned() });
        }
    }

    fn emit_usage(&mut self, usage: TokenUsage, acc: UsageAccounting, out: &mut Vec<StreamEvent>) {
        self.s.cumulative_usage = usage;
        self.s.emitted_usage_total = true;
        out.push(StreamEvent::UsageTotal {
            usage,
            accounting: acc,
        });
    }

    fn add_usage(&mut self, delta: TokenUsage) {
        self.s.cumulative_usage = self.s.cumulative_usage.saturating_add(delta);
        self.s.saw_usage = true;
    }

    fn parse_claude(&mut self, obj: &JsonObj, line: u64, out: &mut Vec<StreamEvent>) -> Result<()> {
        let Some(kind) = opt_str(obj, "type", line)? else {
            self.s.unknown_events += 1;
            return Ok(());
        };
        match kind {
            "system" => {
                let session_id = opt_str(obj, "session_id", line)?;
                let subtype = opt_str(obj, "subtype", line)?;
                self.emit_session(session_id, out);
                match subtype {
                    Some("compact_boundary") => out.push(StreamEvent::Compaction),
                    Some(
                        "init" | "status" | "hook_started" | "hook_progress" | "hook_response"
                        | "task_started" | "task_progress" | "task_notification"
                        | "files_persisted",
                    ) => {}
                    _ => self.s.unknown_events += 1,
                }
            }
            "assistant" => {
                let session_id = opt_str(obj, "session_id", line)?;
                let is_sub =
                    opt_str(obj, "parent_tool_use_id", line)?.is_some_and(|id| !id.is_empty());
                let parsed = if let Some(msg) = opt_obj(obj, "message", line)? {
                    let msg_id = opt_str(msg, "id", line)?;
                    opt_obj(msg, "usage", line)?
                        .map(|usage_obj| parse_claude_step_usage(usage_obj, line))
                        .transpose()?
                        .map(|step| (msg_id, step))
                } else {
                    None
                };
                self.emit_session(session_id, out);
                let Some((msg_id, step)) = parsed else {
                    return Ok(());
                };
                if let Some(id) = msg_id.filter(|id| !id.is_empty())
                    && !self.s.claude_seen_messages.insert(id.to_owned())
                {
                    return Ok(());
                }
                self.add_usage(step);
                if is_sub {
                    self.s.claude_sub_usage = self.s.claude_sub_usage.saturating_add(step);
                } else {
                    out.push(StreamEvent::Step {
                        context_tokens: Some(step.context_tokens()),
                        cumulative_usage: self.s.cumulative_usage,
                    });
                }
            }
            "result" => {
                let session_id = opt_str(obj, "session_id", line)?;
                let denied =
                    parse_denied(obj, "permission_denials", &["tool_name", "action"], line)?;
                let text = opt_str(obj, "result", line)?.unwrap_or_default().to_owned();
                let is_err = opt_bool(obj, "is_error", line)?.unwrap_or(false);
                let status = match opt_str(obj, "subtype", line)? {
                    Some("error_max_turns") => FinalStatus::MaxTurns,
                    _ if is_err => FinalStatus::Error,
                    None | Some("success") => FinalStatus::Success,
                    Some(_) => FinalStatus::Error,
                };
                let mut usage_total = None;
                if let Some(models) = opt_obj(obj, "modelUsage", line)? {
                    let mut total = TokenUsage::default();
                    for (model, entry) in models {
                        let Some(entry_obj) = entry.as_object() else {
                            fail!("line {line}: modelUsage.{model} must be a JSON object");
                        };
                        let u = parse_usage(entry_obj, CLAUDE_MODEL, false, line)?;
                        total = total.saturating_add(u);
                    }
                    if !models.is_empty() && (!total.is_zero() || self.s.cumulative_usage.is_zero())
                    {
                        usage_total = Some((total, UsageAccounting::Authoritative));
                    }
                }
                if usage_total.is_none()
                    && let Some(usage_obj) = opt_obj(obj, "usage", line)?
                {
                    let res_usage = parse_claude_step_usage(usage_obj, line)?;
                    let zeroed_after_steps = !self.s.cumulative_usage.is_zero()
                        && (res_usage.is_zero()
                            || (res_usage.context_tokens() == 0
                                && self.s.cumulative_usage.context_tokens() > 0));
                    if !zeroed_after_steps {
                        let usage = res_usage.saturating_add(self.s.claude_sub_usage);
                        usage_total = Some((usage, UsageAccounting::Partial));
                    }
                }
                self.emit_session(session_id, out);
                out.extend(denied);
                out.push(StreamEvent::Final { text, status });
                if let Some((usage, acc)) = usage_total {
                    self.emit_usage(usage, acc, out);
                } else if self.s.saw_usage {
                    self.emit_usage(self.s.cumulative_usage, UsageAccounting::Partial, out);
                }
            }
            "user" | "stream_event" => {}
            _ => self.s.unknown_events += 1,
        }
        Ok(())
    }

    fn parse_codex(&mut self, obj: &JsonObj, line: u64, out: &mut Vec<StreamEvent>) -> Result<()> {
        let Some(kind) = opt_str(obj, "type", line)? else {
            self.s.unknown_events += 1;
            return Ok(());
        };
        match kind {
            "thread.started" => {
                let id = opt_str(obj, "thread_id", line)?;
                self.emit_session(id, out);
            }
            "turn.started" => {
                self.s.codex_turn = CodexTurn::InProgress;
                self.s.codex_last_text = None;
            }
            "item.started" | "item.updated" => self.s.codex_turn = CodexTurn::InProgress,
            "item.completed" => {
                let item = req_obj(obj, "item", line)?;
                let (agent_text, denied_kind) = match opt_str(item, "type", line)? {
                    Some("agent_message") => (
                        Some(opt_str(item, "text", line)?.unwrap_or_default().to_owned()),
                        None,
                    ),
                    Some(
                        item_type @ ("command_execution" | "file_change" | "mcp_tool_call"
                        | "web_search" | "reasoning" | "todo_list"
                        | "collab_tool_call" | "error"),
                    ) => {
                        let denied = (opt_str(item, "status", line)? == Some("declined"))
                            .then(|| item_type.to_owned());
                        (None, denied)
                    }
                    _ => {
                        self.s.unknown_events += 1;
                        return Ok(());
                    }
                };
                self.s.codex_turn = CodexTurn::InProgress;
                if let Some(text) = agent_text {
                    self.s.codex_last_text = Some(text);
                } else if let Some(action_kind) = denied_kind {
                    out.push(StreamEvent::Denied { action_kind });
                }
            }
            "turn.completed" => {
                let snap = opt_obj(obj, "usage", line)?
                    .map(|usage_obj| parse_usage(usage_obj, CODEX_KEYS, true, line))
                    .transpose()?;
                self.s.codex_turn = CodexTurn::Completed;
                self.s.codex_saw_error = false;
                if let Some(snap) = snap {
                    if self.s.saw_usage && !snap.ge_all(self.s.cumulative_usage) {
                        self.s.cumulative_usage = self.s.cumulative_usage.max_fields(snap);
                        self.s.codex_usage_degraded = true;
                    } else {
                        self.s.cumulative_usage = snap;
                    }
                    self.s.saw_usage = true;
                    self.s.codex_last_turn_missing_usage = false;
                    out.push(StreamEvent::Step {
                        context_tokens: None,
                        cumulative_usage: self.s.cumulative_usage,
                    });
                } else {
                    self.s.codex_last_turn_missing_usage = true;
                }
            }
            "turn.failed" => self.s.codex_turn = CodexTurn::Failed,
            "error" => self.s.codex_saw_error = true,
            _ => self.s.unknown_events += 1,
        }
        Ok(())
    }

    fn parse_agy(&mut self, obj: &JsonObj, line: u64, out: &mut Vec<StreamEvent>) -> Result<()> {
        let Some(event) = opt_str(obj, "event", line)? else {
            self.s.unknown_events += 1;
            return Ok(());
        };
        let top_id = opt_str(obj, "conversation_id", line)?;
        match event {
            "init" => {
                let nested = opt_obj(obj, "init", line)?
                    .map(|init| opt_str(init, "conversation_id", line))
                    .transpose()?
                    .flatten();
                self.emit_session(first_non_empty(top_id, nested), out);
            }
            "step_update" => {
                let su = req_obj(obj, "step_update", line)?;
                let id = first_non_empty(opt_str(su, "conversation_id", line)?, top_id);
                let idx = opt_u64(su, "step_index", line)?;
                let is_done = matches!(opt_str(su, "state", line)?, None | Some("DONE"));
                let step_type = opt_str(su, "step_type", line)?;
                let parsed_usage = match step_type {
                    Some("agent_response" | "checkpoint") => {
                        let usage_obj = opt_obj(su, "usage", line)?;
                        if is_done {
                            usage_obj
                                .map(|u| parse_usage(u, AGY_KEYS, false, line))
                                .transpose()?
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                self.emit_session(id, out);
                match step_type {
                    Some("agent_response") => {
                        if let Some(step) = parsed_usage
                            && idx.is_none_or(|i| self.s.agy_seen_usage.insert(i))
                        {
                            self.add_usage(step);
                            out.push(StreamEvent::Step {
                                context_tokens: Some(step.context_tokens()),
                                cumulative_usage: self.s.cumulative_usage,
                            });
                        }
                    }
                    Some("checkpoint") => {
                        if is_done {
                            if let Some(cp) = parsed_usage.filter(|cp| !cp.is_zero())
                                && idx.is_none_or(|i| self.s.agy_seen_usage.insert(i))
                            {
                                self.add_usage(cp);
                            }
                            if idx.is_none_or(|i| self.s.agy_seen_checkpoints.insert(i)) {
                                out.push(StreamEvent::Compaction);
                            }
                        }
                    }
                    Some("user_input" | "tool" | "subagent" | "system_message" | "finish") => {}
                    _ => self.s.unknown_events += 1,
                }
            }
            "result" => {
                let res = req_obj(obj, "result", line)?;
                let id = first_non_empty(opt_str(res, "conversation_id", line)?, top_id);
                let denied_keys = &["action", "tool_name", "display_name"];
                let denied = parse_denied(res, "denied_actions", denied_keys, line)?;
                let text = opt_str(res, "response", line)?
                    .unwrap_or_default()
                    .to_owned();
                let status = match opt_str(res, "status", line)? {
                    None => FinalStatus::Success,
                    Some(raw) => match raw.to_ascii_uppercase().as_str() {
                        "SUCCESS" => FinalStatus::Success,
                        "CANCELED" | "CANCELLED" => FinalStatus::Canceled,
                        "MAX_TURNS" => FinalStatus::MaxTurns,
                        _ => FinalStatus::Error,
                    },
                };
                let usage = opt_obj(res, "usage", line)?
                    .map(|usage_obj| parse_usage(usage_obj, AGY_KEYS, false, line))
                    .transpose()?;
                self.emit_session(id, out);
                out.extend(denied);
                out.push(StreamEvent::Final { text, status });
                if let Some(usage) = usage {
                    self.emit_usage(usage, UsageAccounting::Authoritative, out);
                } else if self.s.saw_usage {
                    self.emit_usage(self.s.cumulative_usage, UsageAccounting::Partial, out);
                }
            }
            _ => self.s.unknown_events += 1,
        }
        Ok(())
    }
}

/// Streams a complete NDJSON transcript from `reader` using one reused line buffer.
pub fn parse_reader<R: BufRead>(harness: Harness, mut reader: R) -> Result<ParsedStream> {
    let mut parser = StreamParser::new(harness);
    let mut events = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        events.extend(parser.feed_line(&line)?);
    }
    events.extend(parser.finish());
    Ok(ParsedStream {
        events,
        unknown_events: parser.unknown_events(),
    })
}

/// Parses an in-memory NDJSON transcript for `harness`.
pub fn parse_str(harness: Harness, input: &str) -> Result<ParsedStream> {
    parse_reader(harness, Cursor::new(input.as_bytes()))
}

// `[input, output, cache_read, cache_write, reasoning]` wire field names.
const CLAUDE_STEP: [&str; 5] = [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "thinking_tokens",
];
const CLAUDE_MODEL: [&str; 5] = [
    "inputTokens",
    "outputTokens",
    "cacheReadInputTokens",
    "cacheCreationInputTokens",
    "thinkingTokens",
];
const CODEX_KEYS: [&str; 5] = [
    "input_tokens",
    "output_tokens",
    "cached_input_tokens",
    "cache_write_input_tokens",
    "reasoning_output_tokens",
];
const AGY_KEYS: [&str; 5] = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "thinking_tokens",
];

/// Parses a 5-field usage object into non-overlapping [`TokenUsage`] buckets.
///
/// Every supported harness includes thinking/reasoning tokens inside its raw output token count,
/// so `reasoning` is always subtracted from raw output. When `sub_cached_input` is true
/// (Codex / OpenAI Responses API), both `cache_read` and `cache_write` are also subtracted from
/// raw input.
fn parse_usage(
    obj: &JsonObj,
    k: [&str; 5],
    sub_cached_input: bool,
    line: u64,
) -> Result<TokenUsage> {
    let raw_in = opt_u64(obj, k[0], line)?.unwrap_or(0);
    let raw_out = opt_u64(obj, k[1], line)?.unwrap_or(0);
    let cache_read = opt_u64(obj, k[2], line)?.unwrap_or(0);
    let cache_write = opt_u64(obj, k[3], line)?.unwrap_or(0);
    let reasoning = opt_u64(obj, k[4], line)?.unwrap_or(0);
    Ok(TokenUsage {
        input: if sub_cached_input {
            raw_in
                .saturating_sub(cache_read)
                .saturating_sub(cache_write)
        } else {
            raw_in
        },
        output: raw_out.saturating_sub(reasoning),
        cache_read,
        cache_write,
        reasoning,
    })
}

/// Parses Claude Code `message.usage` / `result.usage`, supporting both top-level
/// `thinking_tokens` and nested `output_tokens_details.thinking_tokens` as a subset of
/// `output_tokens`.
fn parse_claude_step_usage(obj: &JsonObj, line: u64) -> Result<TokenUsage> {
    let details_thinking = opt_obj(obj, "output_tokens_details", line)?
        .map(|details| opt_u64(details, "thinking_tokens", line))
        .transpose()?
        .flatten();
    let mut usage = parse_usage(obj, CLAUDE_STEP, false, line)?;
    if usage.reasoning == 0
        && let Some(thinking) = details_thinking
    {
        usage.output = usage.output.saturating_sub(thinking);
        usage.reasoning = thinking;
    }
    Ok(usage)
}

fn first_non_empty<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    a.filter(|s| !s.is_empty())
        .or_else(|| b.filter(|s| !s.is_empty()))
}

fn opt_field<'a, T>(
    obj: &'a JsonObj,
    key: &str,
    line: u64,
    expected: &str,
    extract: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<Option<T>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => extract(v)
            .map(Some)
            .ok_or_else(|| Error(format!("line {line}: field `{key}` must be {expected}"))),
    }
}

fn opt_u64(obj: &JsonObj, key: &str, line: u64) -> Result<Option<u64>> {
    opt_field(obj, key, line, "a non-negative integer", Value::as_u64)
}

fn opt_str<'a>(obj: &'a JsonObj, key: &str, line: u64) -> Result<Option<&'a str>> {
    opt_field(obj, key, line, "a string", Value::as_str)
}

fn opt_bool(obj: &JsonObj, key: &str, line: u64) -> Result<Option<bool>> {
    opt_field(obj, key, line, "a boolean", Value::as_bool)
}

fn opt_obj<'a>(obj: &'a JsonObj, key: &str, line: u64) -> Result<Option<&'a JsonObj>> {
    opt_field(obj, key, line, "a JSON object", Value::as_object)
}

fn req_obj<'a>(obj: &'a JsonObj, key: &str, line: u64) -> Result<&'a JsonObj> {
    opt_obj(obj, key, line)?
        .ok_or_else(|| Error(format!("line {line}: field `{key}` must be a JSON object")))
}

fn parse_denied(obj: &JsonObj, field: &str, keys: &[&str], line: u64) -> Result<Vec<StreamEvent>> {
    let Some(entries) = opt_field(obj, field, line, "an array", Value::as_array)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let action_kind = match entry {
            Value::String(s) if !s.is_empty() => s.clone(),
            Value::String(_) => "unknown".to_owned(),
            Value::Object(map) => {
                let mut found = None;
                for key in keys {
                    if let Some(val) = opt_str(map, key, line)?.filter(|s| !s.is_empty()) {
                        found = Some(val.to_owned());
                        break;
                    }
                }
                found.unwrap_or_else(|| "unknown".to_owned())
            }
            _ => fail!("line {line}: `{field}` entry must be a string or object"),
        };
        out.push(StreamEvent::Denied { action_kind });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        FinalStatus::{Canceled, MaxTurns, Success},
        Harness::*,
        StreamEvent::*,
        UsageAccounting::*,
        *,
    };

    const CLAUDE_FIX: &str = include_str!("fixtures/stream_claude.jsonl");
    const CLAUDE_RESUMED_FIX: &str = include_str!("fixtures/stream_claude_resumed.jsonl");
    const CLAUDE_CRASH_FIX: &str = include_str!("fixtures/stream_claude_crash.jsonl");
    const CODEX_FIX: &str = include_str!("fixtures/stream_codex.jsonl");
    const CODEX_RESUMED_FIX: &str = include_str!("fixtures/stream_codex_resumed.jsonl");
    const AGY_COMP_FIX: &str = include_str!("fixtures/stream_agy_compaction.jsonl");
    const AGY_CP_EOF_FIX: &str = include_str!("fixtures/stream_agy_checkpoint_eof.jsonl");
    const AGY_EARLY_FIX: &str = include_str!("fixtures/stream_agy_early_stop.jsonl");

    fn u(input: u64, output: u64, cache_read: u64, cache_write: u64, reasoning: u64) -> TokenUsage {
        TokenUsage {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
        }
    }
    fn sess(id: &str) -> StreamEvent {
        Session { id: id.into() }
    }
    fn step(context_tokens: Option<u64>, cumulative_usage: TokenUsage) -> StreamEvent {
        Step {
            context_tokens,
            cumulative_usage,
        }
    }
    fn denied(kind: &str) -> StreamEvent {
        Denied {
            action_kind: kind.into(),
        }
    }
    fn fin(text: &str, status: FinalStatus) -> StreamEvent {
        Final {
            text: text.into(),
            status,
        }
    }
    fn tot(usage: TokenUsage, accounting: UsageAccounting) -> StreamEvent {
        UsageTotal { usage, accounting }
    }
    fn check(h: Harness, input: &str, unk: u64, expected: &[StreamEvent]) -> ParsedStream {
        let parsed = parse_str(h, input).unwrap();
        assert_eq!(parsed.unknown_events, unk);
        assert_eq!(parsed.events, expected);
        parsed
    }

    #[test]
    fn claude_fixture_dedups_without_request_id_and_uses_model_usage() {
        check(
            ClaudeCode,
            CLAUDE_FIX,
            2,
            &[
                sess("sess-claude-0001"),
                step(Some(30_000), u(1_200, 4, 28_000, 800, 0)),
                Compaction,
                step(Some(10_000), u(2_600, 15, 41_100, 1_000, 0)),
                fin("Fix verified.", Success),
                // modelUsage: outputTokens (460+140=600) includes thinkingTokens (160+40=200).
                tot(u(2_600, 400, 41_100, 1_000, 200), Authoritative),
            ],
        );
    }

    #[test]
    fn codex_fixtures_keep_snapshots_omit_context_and_account_across_resume() {
        assert!(!Codex.reports_context_tokens() && !Codex.reports_compaction());
        let initial = check(
            Codex,
            CODEX_FIX,
            1,
            &[
                sess("019a0000-0000-7000-8000-000000000001"),
                step(None, u(3_000, 180, 10_000, 2_000, 120)),
                step(None, u(6_000, 430, 28_000, 3_000, 320)),
                fin("Patch applied.", Success),
                tot(u(6_000, 430, 28_000, 3_000, 320), Authoritative),
            ],
        );
        let resumed = check(
            Codex,
            CODEX_RESUMED_FIX,
            0,
            &[
                sess("019a0000-0000-7000-8000-000000000001"),
                step(None, u(7_500, 650, 44_000, 3_500, 450)),
                fin("Ready.", Success),
                tot(u(7_500, 650, 44_000, 3_500, 450), Authoritative),
            ],
        );

        // Upstream Codex cache-write example: 100 input = 0 uncached + 40 cache_read + 60 cache_write.
        let cache_write_only = r#"{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"ok"}}
{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":60,"output_tokens":20,"reasoning_output_tokens":5}}"#;
        check(
            Codex,
            cache_write_only,
            0,
            &[
                step(None, u(0, 15, 40, 60, 5)),
                fin("ok", Success),
                tot(u(0, 15, 40, 60, 5), Authoritative),
            ],
        );

        let mut run = RunUsage::new();
        run.record_stream(Codex, &initial);
        run.record_stream(Codex, &resumed);
        assert_eq!(run.total(), u(7_500, 650, 44_000, 3_500, 450));
        assert_eq!(run.accounting(), Authoritative);

        // Fresh handoff thread adds cleanly; backward resumed snapshot clamps and marks Partial.
        let t1 = Some("019a0000-0000-7000-8000-000000000001");
        let t2 = Some("019a0000-0000-7000-8000-000000000002");
        run.record_invocation(Codex, t2, u(2_000, 200, 5_000, 500, 50), Authoritative);
        assert_eq!(run.total(), u(9_500, 850, 49_000, 4_000, 500));
        assert_eq!(run.accounting(), Authoritative);
        run.record_invocation(
            Codex,
            t1,
            u(4_000, 1_400, 10_000, 1_000, 100),
            Authoritative,
        );
        assert_eq!(run.total(), u(9_500, 1_600, 49_000, 4_000, 500));
        assert_eq!(run.accounting(), Partial);
    }

    #[test]
    fn agy_compaction_checkpoint_eof_and_early_stop_fixtures() {
        assert!(Agy.reports_context_tokens() && Agy.reports_compaction());
        check(
            Agy,
            AGY_COMP_FIX,
            2,
            &[
                sess("00000000-0000-4000-8000-000000000001"),
                step(Some(420_000), u(40_000, 500, 380_000, 0, 300)),
                Compaction,
                step(Some(102_000), u(60_000, 1_100, 500_000, 0, 600)),
                fin("All checks pass.", Success),
                tot(u(60_000, 1_100, 500_000, 0, 600), Authoritative),
            ],
        );
        check(
            Agy,
            AGY_CP_EOF_FIX,
            0,
            &[
                sess("00000000-0000-4000-8000-000000000003"),
                step(Some(400_000), u(30_000, 400, 370_000, 0, 200)),
                Compaction,
                tot(u(36_000, 675, 390_000, 0, 275), Partial),
            ],
        );
        check(
            Agy,
            AGY_EARLY_FIX,
            0,
            &[
                sess("00000000-0000-4000-8000-000000000002"),
                step(Some(20_000), u(5_000, 40, 15_000, 0, 80)),
                denied("command"),
                fin("", Success),
                tot(u(5_000, 40, 15_000, 0, 80), Authoritative),
            ],
        );

        // Google's documented streaming-json example: 10,302 input + 582 output = 10,884 total
        // with 551 thinking tokens inside output -> normalized output 31, reasoning 551.
        let doc_example = r#"{"event":"result","conversation_id":"conv-doc","result":{"status":"SUCCESS","response":"ok","usage":{"input_tokens":10302,"output_tokens":582,"thinking_tokens":551,"cache_read_tokens":0,"total_tokens":10884}}}"#;
        check(
            Agy,
            doc_example,
            0,
            &[
                sess("conv-doc"),
                fin("ok", Success),
                tot(u(10_302, 31, 0, 0, 551), Authoritative),
            ],
        );
    }

    #[test]
    fn claude_denials_max_turns_crash_partial_accounting_and_resume() {
        assert!(ClaudeCode.reports_context_tokens() && ClaudeCode.reports_compaction());
        let max_turns = r#"{"type":"assistant","session_id":"s1","parent_tool_use_id":null,"message":{"id":"m1","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":900,"output_tokens":2}}}
{"type":"assistant","session_id":"s1","parent_tool_use_id":"toolu_1","message":{"id":"m2","usage":{"input_tokens":50,"cache_creation_input_tokens":0,"cache_read_input_tokens":450,"output_tokens":1}}}
{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"s1","result":"Stopped.","permission_denials":["Bash",{"tool_name":"Write"},{"other":"x"}],"usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":900,"output_tokens":80,"output_tokens_details":{"thinking_tokens":30}}}"#;
        check(
            ClaudeCode,
            max_turns,
            0,
            &[
                sess("s1"),
                step(Some(1_000), u(100, 2, 900, 0, 0)),
                denied("Bash"),
                denied("Write"),
                denied("unknown"),
                fin("Stopped.", MaxTurns),
                // output_tokens (80) includes thinking_tokens (30) -> 50 + 1 subagent output = 51.
                tot(u(150, 51, 1_350, 0, 30), Partial),
            ],
        );

        // Crash fixture: empty modelUsage and zeroed usage after valid parent + subagent steps
        // must retain observed usage as Partial rather than overwriting with zeros.
        let crash = check(
            ClaudeCode,
            CLAUDE_CRASH_FIX,
            0,
            &[
                sess("sess-claude-crash-0001"),
                step(Some(30_000), u(1_200, 4, 28_000, 800, 0)),
                fin("Session interrupted.", FinalStatus::Error),
                tot(u(1_500, 6, 30_000, 900, 0), Partial),
            ],
        );

        // Zeroed modelUsage entry after valid steps also falls back to observed usage as Partial.
        let zero_model_usage = r#"{"type":"assistant","session_id":"s-zm","parent_tool_use_id":null,"message":{"id":"m1","usage":{"input_tokens":120,"cache_creation_input_tokens":0,"cache_read_input_tokens":880,"output_tokens":3}}}
{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s-zm","result":"Oops.","usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0},"modelUsage":{"claude-opus-4-6":{"inputTokens":0,"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}"#;
        check(
            ClaudeCode,
            zero_model_usage,
            0,
            &[
                sess("s-zm"),
                step(Some(1_000), u(120, 3, 880, 0, 0)),
                fin("Oops.", FinalStatus::Error),
                tot(u(120, 3, 880, 0, 0), Partial),
            ],
        );

        let eof_in = r#"{"type":"assistant","session_id":"sess-claude-0001","parent_tool_use_id":null,"message":{"id":"m_eof","usage":{"input_tokens":200,"cache_creation_input_tokens":50,"cache_read_input_tokens":750,"output_tokens":3}}}"#;
        let eof_same_session = check(
            ClaudeCode,
            eof_in,
            0,
            &[
                sess("sess-claude-0001"),
                step(Some(1_000), u(200, 3, 750, 50, 0)),
                tot(u(200, 3, 750, 50, 0), Partial),
            ],
        );

        // Cumulative `--resume` modelUsage supersedes prior session snapshot without double-counting.
        let inv1 = parse_str(ClaudeCode, CLAUDE_FIX).unwrap();
        let inv2 = check(
            ClaudeCode,
            CLAUDE_RESUMED_FIX,
            0,
            &[
                sess("sess-claude-0001"),
                step(Some(5_400), u(400, 5, 5_000, 0, 0)),
                fin("Done.", Success),
                tot(u(3_000, 470, 46_100, 1_000, 230), Authoritative),
            ],
        );
        let mut run = RunUsage::new();
        run.record_stream(ClaudeCode, &inv1);
        run.record_stream(ClaudeCode, &inv2);
        assert_eq!(run.total(), u(3_000, 470, 46_100, 1_000, 230));
        assert_eq!(run.accounting(), Authoritative);

        // Fresh handoff session adds cleanly and keeps Authoritative.
        run.record_invocation(
            ClaudeCode,
            Some("sess-claude-0002"),
            u(500, 90, 2_000, 100, 20),
            Authoritative,
        );
        assert_eq!(run.total(), u(3_500, 560, 48_100, 1_100, 250));
        assert_eq!(run.accounting(), Authoritative);

        // Per-invocation Partial fallback on the SAME session ID adds distinctly (not dropped
        // against the larger prior session snapshot) and downgrades accounting to Partial.
        run.record_stream(ClaudeCode, &eof_same_session);
        assert_eq!(run.total(), u(3_700, 563, 48_850, 1_150, 250));
        assert_eq!(run.accounting(), Partial);

        // Later cumulative Authoritative snapshot on that session supersedes without double-counting
        // the prior snapshot or the per-invocation fallback.
        run.record_invocation(
            ClaudeCode,
            Some("sess-claude-0001"),
            u(3_500, 550, 50_000, 1_050, 260),
            Authoritative,
        );
        assert_eq!(run.total(), u(4_000, 640, 52_000, 1_150, 280));
        assert_eq!(run.accounting(), Partial);

        // Backward Authoritative snapshot on a session clamps via max_fields and marks Partial.
        let mut backward = RunUsage::new();
        backward.record_stream(ClaudeCode, &inv2);
        backward.record_stream(ClaudeCode, &inv1);
        assert_eq!(backward.total(), u(3_000, 470, 46_100, 1_000, 230));
        assert_eq!(backward.accounting(), Partial);

        // Consecutive per-invocation Partial fallbacks on the same session ID both add, and a
        // later cumulative Authoritative snapshot supersedes cleanly without double-counting.
        let mut crash_run = RunUsage::new();
        crash_run.record_stream(ClaudeCode, &crash);
        crash_run.record_invocation(
            ClaudeCode,
            Some("sess-claude-crash-0001"),
            u(300, 4, 4_000, 100, 0),
            Partial,
        );
        assert_eq!(crash_run.total(), u(1_800, 10, 34_000, 1_000, 0));
        assert_eq!(crash_run.accounting(), Partial);
        crash_run.record_invocation(
            ClaudeCode,
            Some("sess-claude-crash-0001"),
            u(2_000, 120, 36_000, 1_000, 40),
            Authoritative,
        );
        assert_eq!(crash_run.total(), u(2_000, 120, 36_000, 1_000, 40));
        assert_eq!(crash_run.accounting(), Partial);

        // Missing session_id on a Claude authoritative snapshot adds and marks Partial.
        let mut no_sid = RunUsage::new();
        no_sid.record_invocation(ClaudeCode, None, u(100, 20, 300, 0, 5), Authoritative);
        assert_eq!(no_sid.total(), u(100, 20, 300, 0, 5));
        assert_eq!(no_sid.accounting(), Partial);
    }

    #[test]
    fn codex_truncation_failure_declined_and_no_synthetic_compaction() {
        // Truncation after progress commentary without `turn.completed`: no Final event.
        let after_commentary = r#"{"type":"thread.started","thread_id":"t-trunc-1"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Running checks..."}}"#;
        let parsed = check(Codex, after_commentary, 0, &[sess("t-trunc-1")]);
        let mut run = RunUsage::new();
        run.record_stream(Codex, &parsed);
        assert_eq!(run.accounting(), Partial);

        // Truncation during tool execution after Turn 1 completed: no Final, Partial usage.
        let during_tool = r#"{"type":"thread.started","thread_id":"t-trunc-2"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Step 1"}}
{"type":"turn.completed","usage":{"input_tokens":5000,"cached_input_tokens":3000,"output_tokens":100}}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"i2","type":"command_execution","command":"cargo test","status":"in_progress"}}"#;
        check(
            Codex,
            during_tool,
            0,
            &[
                sess("t-trunc-2"),
                step(None, u(2_000, 100, 3_000, 0, 0)),
                tot(u(2_000, 100, 3_000, 0, 0), Partial),
            ],
        );

        // Unknown item types (including synthetic "compaction") count as unknown; declined
        // items emit Denied; turn.failed yields Error.
        let failed = r#"{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"command_execution","status":"declined"}}
{"type":"item.completed","item":{"id":"i2","type":"compaction"}}
{"type":"turn.failed","error":{"message":"boom"}}"#;
        check(
            Codex,
            failed,
            1,
            &[denied("command_execution"), fin("", FinalStatus::Error)],
        );

        // Unknown `item.completed` after a completed turn must count and skip without resetting
        // turn state back to InProgress, preserving Final and Authoritative usage at EOF.
        let unknown_after_completed = r#"{"type":"thread.started","thread_id":"t-post-unk"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done."}}
{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":400,"cache_write_input_tokens":100,"output_tokens":50,"reasoning_output_tokens":10}}
{"type":"item.completed","item":{"id":"i2","type":"future_post_turn_item"}}"#;
        check(
            Codex,
            unknown_after_completed,
            1,
            &[
                sess("t-post-unk"),
                step(None, u(500, 40, 400, 100, 10)),
                fin("Done.", Success),
                tot(u(500, 40, 400, 100, 10), Authoritative),
            ],
        );

        // Transient error before turn.completed still succeeds; finish() is silent and idempotent.
        let transient = r#"{"type":"turn.started"}
{"type":"error","message":"Reconnecting..."}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Recovered."}}
{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":50}}"#;
        assert!(
            parse_str(Codex, transient)
                .unwrap()
                .events
                .contains(&fin("Recovered.", Success))
        );

        let mut empty = StreamParser::new(Codex);
        assert!(empty.finish().is_empty() && empty.finish().is_empty());
        assert!(empty.feed_line("{}").is_err());
    }

    #[test]
    fn codex_missing_usage_on_final_turn_marks_partial_and_recovers_on_later_turn() {
        // Turn 1 reports usage, Turn 2 completes without usage -> stale Turn 1 snapshot is Partial.
        let missing_final = r#"{"type":"thread.started","thread_id":"t-miss-1"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Turn 1"}}
{"type":"turn.completed","usage":{"input_tokens":5000,"cached_input_tokens":3000,"output_tokens":200,"reasoning_output_tokens":80}}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"Turn 2"}}
{"type":"turn.completed","usage":null}"#;
        check(
            Codex,
            missing_final,
            0,
            &[
                sess("t-miss-1"),
                step(None, u(2_000, 120, 3_000, 0, 80)),
                fin("Turn 2", Success),
                tot(u(2_000, 120, 3_000, 0, 80), Partial),
            ],
        );

        // Turn 1 lacks usage, Turn 2 provides cumulative thread snapshot -> Authoritative again.
        let later_usage = r#"{"type":"thread.started","thread_id":"t-miss-2"}
{"type":"turn.started"}
{"type":"turn.completed"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done"}}
{"type":"turn.completed","usage":{"input_tokens":6000,"cached_input_tokens":4000,"output_tokens":150,"reasoning_output_tokens":50}}"#;
        check(
            Codex,
            later_usage,
            0,
            &[
                sess("t-miss-2"),
                step(None, u(2_000, 100, 4_000, 0, 50)),
                fin("Done", Success),
                tot(u(2_000, 100, 4_000, 0, 50), Authoritative),
            ],
        );
    }

    #[test]
    fn agy_checkpoint_dedups_compaction_and_nonzero_usage_separately() {
        // DONE checkpoint with no usage, then all-zero usage, then populated usage on the same
        // step_index: emits one Compaction and accumulates the nonzero usage exactly once.
        let delayed_usage = r#"{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"checkpoint"}}
{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0}}}
{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":400,"output_tokens":30,"thinking_tokens":10,"cache_read_tokens":100}}}
{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":400,"output_tokens":30,"thinking_tokens":10,"cache_read_tokens":100}}}"#;
        check(
            Agy,
            delayed_usage,
            0,
            &[Compaction, tot(u(400, 20, 100, 0, 10), Partial)],
        );

        // DONE checkpoint with only all-zero placeholder usage does not set saw_usage or emit UsageTotal.
        let zero_only = r#"{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0}}}"#;
        check(Agy, zero_only, 0, &[Compaction]);
    }

    #[test]
    fn corrupt_lines_do_not_mutate_parser_state() {
        // Claude: corrupt system, assistant, and result lines leave session_id unrecorded.
        let mut claude = StreamParser::new(ClaudeCode);
        assert!(
            claude
                .feed_line(r#"{"type":"system","session_id":"s-cl","subtype":123}"#)
                .is_err()
        );
        assert!(
            claude
                .feed_line(r#"{"type":"assistant","session_id":"s-cl","message":{"id":"m1","usage":{"input_tokens":-1}}}"#)
                .is_err()
        );
        assert!(
            claude
                .feed_line(r#"{"type":"result","session_id":"s-cl","modelUsage":{"m":"bad"}}"#)
                .is_err()
        );
        let evs = claude
            .feed_line(r#"{"type":"system","subtype":"init","session_id":"s-cl"}"#)
            .unwrap();
        assert_eq!(evs, vec![sess("s-cl")]);

        // Codex: corrupt turn.completed must not mark Completed or clear error; corrupt
        // item.completed after a valid turn.completed must not clobber Completed back to InProgress.
        let mut codex = StreamParser::new(Codex);
        assert!(
            codex
                .feed_line(r#"{"type":"turn.started"}"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            codex
                .feed_line(r#"{"type":"turn.completed","usage":{"input_tokens":"bad"}}"#)
                .is_err()
        );
        assert!(codex.finish().is_empty());

        let mut codex_err = StreamParser::new(Codex);
        assert!(
            codex_err
                .feed_line(r#"{"type":"error","message":"boom"}"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            codex_err
                .feed_line(r#"{"type":"turn.completed","usage":{"input_tokens":"bad"}}"#)
                .is_err()
        );
        assert_eq!(codex_err.finish(), vec![fin("", FinalStatus::Error)]);

        let mut codex_ok = StreamParser::new(Codex);
        assert!(
            codex_ok
                .feed_line(r#"{"type":"turn.started"}"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            codex_ok
                .feed_line(r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"ok"}}"#)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            codex_ok
                .feed_line(r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10}}"#)
                .unwrap(),
            vec![step(None, u(100, 10, 0, 0, 0))]
        );
        assert!(
            codex_ok
                .feed_line(r#"{"type":"item.completed","item":null}"#)
                .is_err()
        );
        assert_eq!(
            codex_ok.finish(),
            vec![fin("ok", Success), tot(u(100, 10, 0, 0, 0), Authoritative)]
        );

        // agy: corrupt step_update and result lines leave conversation_id unrecorded.
        let mut agy = StreamParser::new(Agy);
        assert!(
            agy.feed_line(r#"{"event":"step_update","step_update":{"conversation_id":"c-agy","step_index":-1,"step_type":"tool"}}"#)
                .is_err()
        );
        assert!(
            agy.feed_line(r#"{"event":"result","result":{"conversation_id":"c-agy","usage":{"input_tokens":"bad"}}}"#)
                .is_err()
        );
        let evs = agy
            .feed_line(r#"{"event":"init","conversation_id":"c-agy"}"#)
            .unwrap();
        assert_eq!(evs, vec![sess("c-agy")]);
    }

    #[test]
    fn agy_result_only_session_non_done_checkpoint_and_resume() {
        let input = r#"{"event":"step_update","step_update":{"step_index":1,"state":"ERROR","step_type":"checkpoint","usage":{"input_tokens":999,"output_tokens":99,"cache_read_tokens":0}}}
{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":500,"output_tokens":40,"cache_read_tokens":200,"thinking_tokens":10}}}
{"event":"result","result":{"conversation_id":"conv-res-only","status":"CANCELED","response":"stopped"}}"#;
        check(
            Agy,
            input,
            0,
            &[
                Compaction,
                sess("conv-res-only"),
                fin("stopped", Canceled),
                tot(u(500, 30, 200, 0, 10), Partial),
            ],
        );

        let inv1 = parse_str(Agy, AGY_EARLY_FIX).unwrap();
        let inv2_in = r#"{"event":"result","conversation_id":"00000000-0000-4000-8000-000000000002","result":{"status":"SUCCESS","response":"Done.","usage":{"input_tokens":1000,"output_tokens":80,"thinking_tokens":20,"cache_read_tokens":4000}}}"#;
        let inv2 = parse_str(Agy, inv2_in).unwrap();
        let mut run = RunUsage::new();
        run.record_stream(Agy, &inv1);
        run.record_stream(Agy, &inv2);
        assert_eq!(run.total(), u(6_000, 100, 19_000, 0, 100));
        assert_eq!(run.accounting(), Authoritative);
    }

    #[test]
    fn malformed_json_and_corrupt_fields_error_while_unknown_objects_count() {
        for (harness, bad) in [
            (ClaudeCode, "not json"),
            (Codex, "[1, 2, 3]"),
            (Agy, "42"),
            (
                ClaudeCode,
                r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":-1}}}"#,
            ),
            (
                ClaudeCode,
                r#"{"type":"assistant","message":{"id":"m1","usage":{"output_tokens":10,"output_tokens_details":{"thinking_tokens":-1}}}}"#,
            ),
            (
                ClaudeCode,
                r#"{"type":"result","modelUsage":{"m":"not-an-object"}}"#,
            ),
            (
                ClaudeCode,
                r#"{"type":"result","permission_denials":"Bash"}"#,
            ),
            (
                Codex,
                r#"{"type":"turn.completed","usage":{"input_tokens":"bad"}}"#,
            ),
            (Codex, r#"{"type":"item.completed","item":null}"#),
            (
                Agy,
                r#"{"event":"step_update","step_update":{"step_index":-1,"step_type":"tool"}}"#,
            ),
            (
                Agy,
                r#"{"event":"result","result":{"denied_actions":[123]}}"#,
            ),
        ] {
            let err = parse_str(harness, &format!("\n  \n{bad}\n")).unwrap_err();
            assert!(
                err.to_string().starts_with("line 3:"),
                "{harness:?} {bad}: {err}"
            );
        }
        for harness in [ClaudeCode, Codex, Agy] {
            check(harness, "\n   \n{}\n", 1, &[]);
        }
    }

    #[test]
    fn incremental_feed_line_matches_parse_str() {
        for (harness, fixture) in [
            (ClaudeCode, CLAUDE_FIX),
            (ClaudeCode, CLAUDE_RESUMED_FIX),
            (ClaudeCode, CLAUDE_CRASH_FIX),
            (Codex, CODEX_FIX),
            (Codex, CODEX_RESUMED_FIX),
            (Agy, AGY_COMP_FIX),
            (Agy, AGY_CP_EOF_FIX),
            (Agy, AGY_EARLY_FIX),
        ] {
            let expected = parse_str(harness, fixture).unwrap();
            let crlf = fixture.trim_end_matches('\n').replace('\n', "\r\n");
            let mut parser = StreamParser::new(harness);
            let mut events = Vec::new();
            for line in crlf.split('\n') {
                events.extend(parser.feed_line(line).unwrap());
            }
            events.extend(parser.finish());
            assert_eq!(parser.harness(), harness);
            assert_eq!(events, expected.events);
            assert_eq!(parser.unknown_events(), expected.unknown_events);
            if let Some(UsageTotal { usage, .. }) = events.last() {
                assert_eq!(parser.cumulative_usage(), *usage);
            }
        }
    }

    #[test]
    fn fixtures_contain_no_secrets_or_host_paths() {
        let forbidden = "/home/|/Users/|/mnt/|/var/lib/|/tmp/|sk-|ghp_|gho_|AIza|ya29.|Bearer |xoxb-|eyJ|PRIVATE KEY|accessToken|refresh_token|@";
        for (name, content) in [
            ("stream_claude.jsonl", CLAUDE_FIX),
            ("stream_claude_resumed.jsonl", CLAUDE_RESUMED_FIX),
            ("stream_claude_crash.jsonl", CLAUDE_CRASH_FIX),
            ("stream_codex.jsonl", CODEX_FIX),
            ("stream_codex_resumed.jsonl", CODEX_RESUMED_FIX),
            ("stream_agy_compaction.jsonl", AGY_COMP_FIX),
            ("stream_agy_checkpoint_eof.jsonl", AGY_CP_EOF_FIX),
            ("stream_agy_early_stop.jsonl", AGY_EARLY_FIX),
        ] {
            for needle in forbidden.split('|') {
                assert!(
                    !content.contains(needle),
                    "fixture {name} contains {needle:?}"
                );
            }
        }
    }
}
