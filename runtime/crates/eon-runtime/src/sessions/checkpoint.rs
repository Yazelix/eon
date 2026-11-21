//! A running terminal's part of an Update checkpoint: whether this Runtime
//! started it with the default shell and the Venus process it last named.
// Consumed by `checkpoint::update::model`, whose capture and restore halves
// the supervisor's export and import use once S4a (eon-t3fyd.7) and S4b-1
// (eon-t3fyd.8) call them; until then their tests consume it.
#![cfg_attr(not(test), allow(dead_code))]

use super::{AttachName, Naming, RunningSession};
use crate::checkpoint::{
    Error, Result,
    update::{Named, TerminalExtra},
};

impl RunningSession {
    /// The terminal's extra, refused unless the row is settled: no request in
    /// flight, no answer kept for the poller, no naming retry, no stop sent,
    /// not ended and no unreconciled end.
    pub(crate) fn checkpoint_extra(&self) -> Result<TerminalExtra> {
        let unsettled = if self.request.is_some() {
            Some("a terminal request is in flight")
        } else if self.answered_status.is_some() {
            Some("a terminal status answer is not consumed")
        } else if matches!(self.naming, Some(Naming::Failed { .. })) {
            Some("a terminal naming retry is pending")
        } else if self.stop_sent {
            Some("a terminal is stopping")
        } else if self.finished.is_some() {
            Some("a terminal has ended")
        } else if self.reap_failure_reported {
            Some("a terminal's end is not reconciled")
        } else {
            None
        };
        if let Some(what) = unsettled {
            return Err(Error::Unsettled(what));
        }
        Ok(TerminalExtra {
            terminal: self.id.clone(),
            default_shell: self.default_shell,
            named: match self.naming {
                Some(Naming::Named(name)) => Some(Named {
                    process_id: name.process_id,
                    process_start: name.process_start,
                }),
                _ => None,
            },
        })
    }

    /// Applies `extra` to this run, which S4 re-adopted for its terminal.
    pub(crate) fn restore_extra(&mut self, extra: &TerminalExtra) {
        self.default_shell = extra.default_shell;
        self.naming = extra.named.map(|named| {
            Naming::Named(AttachName {
                process_id: named.process_id,
                process_start: named.process_start,
            })
        });
    }
}

#[cfg(test)]
mod tests;
