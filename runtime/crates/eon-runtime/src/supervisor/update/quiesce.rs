//! The private Commit's between-turn barrier. Only the supervisor can submit
//! observer, scan, attention and notice work, and it holds this turn through
//! validation and exec. Quota workers also reserve their own reads.

use super::SupervisorState;
use crate::notification::NoticePark;
use std::{
    thread,
    time::{Duration, Instant},
};

const SETTLE_HELPERS: Duration = Duration::from_millis(1_250);
const QUIESCE: Duration = Duration::from_millis(1_500);

pub(super) struct Quiesce<'a> {
    pub(super) state: &'a mut SupervisorState,
    notices: Option<NoticePark>,
    deadline: Instant,
}

impl<'a> Quiesce<'a> {
    pub(super) fn start(state: &'a mut SupervisorState) -> Result<Self, &'static str> {
        let started = Instant::now();
        if let Some(what) = state.unsettled() {
            return Err(what);
        }
        let notices = state.notices.park();
        let mut guard = Self {
            state,
            notices,
            deadline: started + QUIESCE,
        };
        if let Some(quotas) = &guard.state.quotas {
            quotas.set_park(true);
        }
        guard.state.pane_labels.park();
        let helper_deadline = started + SETTLE_HELPERS;
        loop {
            let unsettled = guard.unsettled()?;
            let left = helper_deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(unsettled.unwrap_or("helper settlement exceeded its deadline"));
            }
            if unsettled.is_none() {
                return Ok(guard);
            }
            thread::sleep(left.min(Duration::from_millis(5)));
        }
    }

    fn unsettled(&mut self) -> Result<Option<&'static str>, &'static str> {
        let notices = self
            .notices
            .as_mut()
            .map_or(Ok(true), NoticePark::settled)?;
        if notices && self.state.notices.has_pending_event() {
            return Err("a detach notice action is pending");
        }
        self.state.reap_visits();
        let checks = [
            (
                self.state.visits.is_empty(),
                "a Zoxide visit is still running",
            ),
            (
                self.state
                    .quotas
                    .as_ref()
                    .is_none_or(crate::quota::Quotas::parked),
                "quota collection is still running",
            ),
            (
                self.state.pane_labels.parked(),
                "a pane observer is still running",
            ),
            (
                self.state
                    .running
                    .as_mut()
                    .is_none_or(crate::running::Running::is_idle),
                "a running window scan is still running",
            ),
            (
                self.state
                    .attention
                    .as_mut()
                    .is_none_or(crate::attention::Attention::park),
                "an attention helper is still running",
            ),
            (notices, "the notification worker is still running"),
        ];
        Ok(checks
            .into_iter()
            .find_map(|(settled, what)| (!settled).then_some(what)))
    }

    pub(super) fn within_deadline(&self) -> Result<(), &'static str> {
        if Instant::now() >= self.deadline {
            return Err("quiesce exceeded its deadline");
        }
        Ok(())
    }
}

impl Drop for Quiesce<'_> {
    fn drop(&mut self) {
        if let Some(quotas) = &self.state.quotas {
            quotas.set_park(false);
        }
        if let Some(attention) = &mut self.state.attention {
            attention.unpark();
        }
        // The notice token wakes its command worker on drop. Pane observers
        // restart from their cached metadata on the next normal reconciliation.
    }
}
