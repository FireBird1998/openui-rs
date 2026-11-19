//! Managed mutations retain their action continuation in the session. The
//! embedder needs only the tool call identity to complete the segment.
use super::*;
use openui_runtime::ToolCallId;

impl SessionCore {
    pub(super) fn dispatch_mutation(
        &mut self,
        request: openui_runtime::MutationRequest,
        continuation: &ActionContinuation,
        output: &mut ActionOutput,
    ) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let Some(name) = request.statement else {
            return;
        };
        if !manager.can_fire_mutation(&name) {
            return;
        }
        // A managed run has promised a resume. Report a cut on this step
        // before dispatch if all the session's continuation slots are occupied.
        let started = if self.continuations.len() as u64 >= self.limits.max_continuations {
            output.size_limited = true;
            Err(limit("pending mutation continuation limit"))
        } else {
            manager
                .fire_mutation(&name, request.args.as_ref())
                .map_err(|error| {
                    if let openui_runtime::QueryCommandError::Limit(limit) = &error {
                        output.size_limited = limit.flags.size_limited;
                        output.depth_limited = limit.flags.depth_limited;
                    }
                    error.into()
                })
        };
        match started {
            Ok(update) => {
                if let Some(call) = update.calls.first() {
                    // Every entry point reserves a generation before running any steps.
                    self.next_continuation = self.next_continuation.saturating_add(1);
                    self.continuations.insert(
                        self.next_continuation,
                        PendingContinuation {
                            continuation: continuation.clone(),
                            handle: Weak::new(),
                            call: Some(call.id.clone()),
                        },
                    );
                }
                self.query_effects.record(update);
            }
            Err(error) => {
                output.failure = Some(ActionFailureOutput {
                    step: Some(request.step as u64),
                    error,
                })
            }
        }
    }

    pub(super) fn complete_mutation(
        &mut self,
        generation: u64,
        call: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<(), OpenUiError> {
        let succeeded = matches!(outcome, ToolCallOutcome::Result(_));
        if succeeded {
            self.continuation_room()?;
        }
        let functions = Self::functions(
            &self.library,
            &self.functions,
            self.core_limits,
            self.limits.max_json_bytes,
        )?;
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let update = manager.complete_tool_call(call, outcome)?;
        self.query_effects.record(update);
        let pending = self
            .continuations
            .remove(&generation)
            .ok_or(OpenUiError::ConsumedHandle)?;
        let refs = manager.prepared_results(self.reference_limits);
        let input = tree_input(
            &self.program,
            &self.library,
            &self.bindings,
            &refs,
            functions.as_ref(),
        )
        .with_managed_mutations();
        let version = self.store.version();
        let run =
            openui_runtime::resume_action(input, &mut self.store, &pending.continuation, succeeded)
                .map_err(|e| {
                    runtime_error(RuntimeFailure::Resume(e), self.core_limits.result_size())
                })?;
        drop(functions);
        let output = self.output(run)?;
        self.query_effects.actions.push(output);
        if self.store.version() != version {
            self.query_effects.changed = true;
            self.state_changed();
        }
        Ok(())
    }
}
