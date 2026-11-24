//! The private Commit's between-turn barrier. Only the supervisor can submit
//! observer, scan, attention, notice and lease work, and it holds this turn
//! through validation and exec, so it accepts no control client and sends
//! nothing on a lease meanwhile. Quota workers also reserve their own reads.

use super::SupervisorState;
use crate::{attention::Target, notification::NoticePark, sessions};
use std::{
    thread,
    time::{Duration, Instant},
};

const SETTLE_HELPERS: Duration = Duration::from_millis(1_250);
const DRAIN_LEASES: Duration = Duration::from_millis(1_000);
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
        #[cfg(test)]
        if crate::update::fault("held-status")
            && let Some(root) = std::env::var_os("EON_TEST_HANDOFF_ROOT")
        {
            let _ = std::fs::write(std::path::Path::new(&root).join("quiescing"), b"");
        }
        // The barrier changes no topology, so attention's targets hold for it.
        let targets = guard
            .state
            .workspaces
            .as_ref()
            .map(|workspaces| crate::attention::targets(&workspaces.snapshots()).0)
            .unwrap_or_default();
        let helper_deadline = started + SETTLE_HELPERS;
        let lease_deadline = started + DRAIN_LEASES;
        loop {
            let leases = guard.drain_leases(&targets)?;
            let unsettled = match leases {
                Some(what) => Some(what),
                None => guard.unsettled(&targets)?,
            };
            let now = Instant::now();
            let deadline = if leases.is_some() {
                lease_deadline
            } else {
                helper_deadline
            };
            let left = deadline.saturating_duration_since(now);
            if left.is_zero() {
                return Err(unsettled.unwrap_or("helper settlement exceeded its deadline"));
            }
            if unsettled.is_none() {
                return Ok(guard);
            }
            thread::sleep(left.min(Duration::from_millis(5)));
        }
    }

    /// Advances every lease's issued request at once and lets attention take
    /// the answers, sending nothing (C02). The first reason a lease is not yet
    /// idle, or a refusal that leaves its request for the next turn.
    fn drain_leases(&mut self, targets: &[Target]) -> Result<Option<&'static str>, &'static str> {
        let state = &mut *self.state;
        let mut pending = None;
        for session in &mut state.sessions {
            if !sessions::drain_for_update(session, &state.log)? {
                pending = Some("a terminal request is in flight");
            }
        }
        if let Some(attention) = &mut state.attention {
            attention.drain(targets, &mut state.sessions);
        }
        if pending.is_none() && state.sessions.iter().any(sessions::status_answered) {
            pending = Some("a terminal status answer is not consumed");
        }
        Ok(pending)
    }

    /// The first helper still at work, once every lease is idle.
    fn unsettled(&mut self, targets: &[Target]) -> Result<Option<&'static str>, &'static str> {
        let notices = self
            .notices
            .as_mut()
            .map_or(Ok(true), NoticePark::settled)?;
        if notices && self.state.notices.has_pending_event() {
            return Err("a detach notice action is pending");
        }
        self.state.reap_visits();
        // Parked only now, so a sample a drained answer needs could still start.
        let attention = match &mut self.state.attention {
            Some(attention) => {
                attention.park() && attention.drain(targets, &mut self.state.sessions)
            }
            None => true,
        };
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
            (attention, "an attention helper is still running"),
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
