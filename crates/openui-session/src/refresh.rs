//! Clock and lifecycle commands use the same drain as registration and completion.
use super::*;
use openui_runtime::QueryCommand;

impl SessionCore {
    /// Sample registration time without delivering a refresh callback.
    pub fn sample_query_clock(&mut self, now_ms: u64) -> Result<(), OpenUiError> {
        if let Some(manager) = self.manager.as_mut() {
            manager.sample_clock(now_ms)?;
        }
        Ok(())
    }

    /// Advance the attached provider's monotonic clock, including while streaming.
    pub fn advance_query_clock(&mut self, now_ms: u64) -> Result<(), OpenUiError> {
        // Unregistered, the timers are another program's: keep the clock and
        // fire none (#119). Overdue timers fire on the first advance after a
        // registration keeps them.
        if self.is_unregistered() {
            return self.sample_query_clock(now_ms);
        }
        if let Some(manager) = self.manager.as_mut() {
            self.query_effects
                .record(manager.apply(QueryCommand::AdvanceClock { now_ms })?);
        }
        Ok(())
    }

    /// Retain query data, discard outstanding dispatches and release suspended mutations.
    /// Each released plan, and each mutation step run while paused, ends
    /// with a `QueriesPaused` failure (#117): a released plan's comes from
    /// the next `take_query_update` as a resumed segment, a click's in its
    /// own output.
    pub fn pause_queries(&mut self) -> Result<(), OpenUiError> {
        // Unregistered, the snapshot it changes is another program's (#142).
        let drawn = !self.is_unregistered();
        if let Some(manager) = self.manager.as_mut() {
            let update = manager.apply(QueryCommand::Dispose)?;
            // Also clears the undrained calls, which no driver ever held.
            self.drop_paused_plans();
            self.query_effects.record_for(update, drawn);
        }
        Ok(())
    }

    /// Advance before restarting registrations. An active provider simply processes the wake.
    pub fn resume_queries_at(&mut self, now_ms: u64) -> Result<(), OpenUiError> {
        self.advance_query_clock(now_ms)?;
        self.resume_queries()
    }

    /// Re-register a settled program. A streaming program waits for its settled draw.
    pub fn resume_queries(&mut self) -> Result<(), OpenUiError> {
        if let Some(manager) = self.manager.as_mut().filter(|m| m.is_disposed()) {
            manager.apply(QueryCommand::Activate)?;
            self.registered.resumed = true;
            self.query_inputs_changed = true;
            self.state_changed();
        }
        Ok(())
    }
}
