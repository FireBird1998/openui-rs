//! Clock and lifecycle commands use the same drain as registration and completion.
use super::*;
use openui_runtime::QueryCommand;

impl SessionCore {
    /// Advance the attached provider's monotonic clock, including while streaming.
    pub fn advance_query_clock(&mut self, now_ms: u64) -> Result<(), OpenUiError> {
        if let Some(manager) = self.manager.as_mut() {
            self.query_effects
                .record(manager.apply(QueryCommand::AdvanceClock { now_ms })?);
        }
        Ok(())
    }

    /// Retain query data, discard outstanding dispatches and release suspended mutations.
    pub fn pause_queries(&mut self) -> Result<(), OpenUiError> {
        if let Some(manager) = self.manager.as_mut() {
            let update = manager.apply(QueryCommand::Dispose)?;
            self.query_effects.calls.clear();
            self.continuations.retain(|_, entry| entry.call.is_none());
            self.query_effects.record(update);
        }
        Ok(())
    }

    /// Re-register a settled program. A streaming program waits for its settled draw.
    pub fn resume_queries(&mut self) -> Result<(), OpenUiError> {
        if let Some(manager) = self.manager.as_mut().filter(|m| m.is_disposed()) {
            manager.apply(QueryCommand::Activate)?;
            self.query_inputs_changed = true;
            self.state_changed();
        }
        Ok(())
    }
}
