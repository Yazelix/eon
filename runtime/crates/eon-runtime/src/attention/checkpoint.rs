//! Attention's part of an Update checkpoint: each terminal's last good reply,
//! its records and raises, recovered baselines, the poll schedule, the sidebar
//! and the epoch. Instants travel as distances from the capture anchor.
// Consumed by `checkpoint::update::model` and its tests until eon-t3fyd.6 (S3b).
#![cfg_attr(not(test), allow(dead_code))]

use super::{Attention, NoticeDispatch, PendingNotices, Raise, Record, Terminal};
use crate::checkpoint::{
    Error, Result,
    update::{self, AttentionRecord, AttentionTerminal, MAX_STAMP_NS, Recovered, Slot},
};
use std::{
    sync::atomic::Ordering::Relaxed,
    time::{Duration, Instant},
};

impl Attention {
    /// Attention at `anchor` over the window's `slots`. Refuses helper work in
    /// flight, a notice still queued, an entry for a run no slot holds, and a
    /// past fact after `anchor`. Focus, viewing and the jump cursor are not
    /// kept: the new Venus observes again.
    pub(crate) fn checkpoint(
        &self,
        anchor: Instant,
        slots: &[Slot<'_>],
    ) -> Result<update::Attention> {
        if !self.helper_pending.is_empty() {
            return Err(Error::Unsettled("an attention helper sample is in flight"));
        }
        let age = |instant: Instant| -> Result<u64> {
            anchor
                .checked_duration_since(instant)
                .and_then(nanos)
                .ok_or(Error::Unsettled("an attention time lies after the capture"))
        };
        let mut terminals = Vec::with_capacity(self.terminals.len());
        for terminal in &self.terminals {
            if !slots.iter().any(|slot| {
                !slot.pending
                    && slot.run.terminal_id == terminal.session
                    && slot.run.run_id == terminal.run
            }) {
                return Err(Error::Unsettled("attention holds a run no terminal has"));
            }
            if terminal.notices.0.iter().any(|notice| {
                notice
                    .dispatch
                    .upgrade()
                    .is_some_and(|dispatch| dispatch.0.load(Relaxed) == NoticeDispatch::QUEUED)
            }) {
                return Err(Error::Unsettled("a desktop notice is still queued"));
            }
            let records = |records: &[Record]| -> Result<Vec<AttentionRecord>> {
                records
                    .iter()
                    .map(|record| {
                        Ok(AttentionRecord {
                            record: record.record.clone(),
                            changed_age_ns: age(record.changed_at)?,
                            raise: record
                                .raise
                                .map(|raise| {
                                    Ok::<_, Error>(update::Raise {
                                        age_ns: age(raise.at)?,
                                        changed: raise.changed,
                                        seen: raise.seen,
                                        notified: raise.notified,
                                    })
                                })
                                .transpose()?,
                            blocked_notified: record.blocked_notified,
                        })
                    })
                    .collect()
            };
            terminals.push(AttentionTerminal {
                terminal: terminal.session.clone(),
                run_id: terminal.run.clone(),
                update: terminal.update,
                count: terminal.count,
                background: terminal.background,
                foreground: terminal.foreground,
                name: terminal.name.clone(),
                records: records(&terminal.records)?,
                omitted: records(&terminal.omitted)?,
                replied_age_ns: age(terminal.replied)?,
            });
        }
        let mut recovered = Vec::with_capacity(self.recovered.len());
        for (run, baseline) in &self.recovered {
            if !slots.iter().any(|slot| &slot.run.run_id == run) {
                return Err(Error::Unsettled("attention holds a run no terminal has"));
            }
            recovered.push(Recovered {
                run_id: run.clone(),
                baseline: *baseline,
            });
        }
        Ok(update::Attention {
            terminals,
            recovered,
            // A poll already due is due now: the overdue amount changes nothing.
            next_poll_due_ns: self
                .next_poll
                .checked_duration_since(anchor)
                .map_or(Some(0), nanos)
                .ok_or(Error::Unsettled("the next attention poll is out of range"))?,
            sidebar: self.sidebar,
            epoch_age_ns: age(self.epoch.0)?,
            epoch_wall_ms: self.epoch.1,
        })
    }

    /// Attention `elapsed` after its capture, rebased on `now`: each past fact
    /// is `age + elapsed` before `now`, and the next poll `due - elapsed` after
    /// it, or now when that has passed. No focus, viewing, cursor, helper
    /// worker or notice. The caller runs the new Venus's first `observe` before
    /// the first `take_notifications`, else a raise on the terminal the user
    /// is looking at can send one desktop notice.
    pub(crate) fn restore(
        saved: &update::Attention,
        now: Instant,
        elapsed: Duration,
    ) -> Result<Self> {
        let out_of_range = Error::Unrestorable("an attention time is out of range");
        let at = |age_ns: u64| -> Result<Instant> {
            Duration::from_nanos(age_ns)
                .checked_add(elapsed)
                .and_then(|age| now.checked_sub(age))
                .ok_or(out_of_range.clone())
        };
        let records = |records: &[AttentionRecord]| -> Result<Vec<Record>> {
            records
                .iter()
                .map(|record| {
                    Ok(Record {
                        record: record.record.clone(),
                        changed_at: at(record.changed_age_ns)?,
                        raise: record
                            .raise
                            .map(|raise| {
                                Ok::<_, Error>(Raise {
                                    at: at(raise.age_ns)?,
                                    changed: raise.changed,
                                    seen: raise.seen,
                                    notified: raise.notified,
                                })
                            })
                            .transpose()?,
                        blocked_notified: record.blocked_notified,
                    })
                })
                .collect()
        };
        let terminals = saved
            .terminals
            .iter()
            .map(|terminal| {
                Ok(Terminal {
                    session: terminal.terminal.clone(),
                    run: terminal.run_id.clone(),
                    update: terminal.update,
                    count: terminal.count,
                    background: terminal.background,
                    foreground: terminal.foreground,
                    name: terminal.name.clone(),
                    records: records(&terminal.records)?,
                    omitted: records(&terminal.omitted)?,
                    notices: PendingNotices::default(),
                    replied: at(terminal.replied_age_ns)?,
                })
            })
            .collect::<Result<_>>()?;
        let due = Duration::from_nanos(saved.next_poll_due_ns).saturating_sub(elapsed);
        Ok(Attention {
            terminals,
            recovered: saved
                .recovered
                .iter()
                .map(|run| (run.run_id.clone(), run.baseline))
                .collect(),
            focus: None,
            viewing: false,
            cursor: None,
            next_poll: now.checked_add(due).ok_or(out_of_range.clone())?,
            sidebar: saved.sidebar,
            epoch: (at(saved.epoch_age_ns)?, saved.epoch_wall_ms),
            helper_worker: None,
            helper_pending: Vec::new(),
            helper_sample: 0,
        })
    }
}

/// A distance in the checkpoint's bounded nanoseconds.
fn nanos(duration: Duration) -> Option<u64> {
    u64::try_from(duration.as_nanos())
        .ok()
        .filter(|nanos| *nanos <= MAX_STAMP_NS)
}

#[cfg(test)]
pub(crate) mod tests;
