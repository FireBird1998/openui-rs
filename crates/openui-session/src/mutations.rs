//! Managed mutations retain their action continuation in the session. The
//! embedder needs only the tool call identity to complete the segment.
use super::*;
use openui_runtime::ToolCallId;

impl SessionCore {
    pub(super) fn dispatch_mutation(
        &mut self,
        request: openui_runtime::MutationRequest,
        continuation: ActionContinuation,
        generation: u64,
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
                    self.next_continuation = generation;
                    self.continuations.insert(
                        generation,
                        PendingContinuation {
                            continuation,
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
            self.next_continuation_generation()?;
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
        );
        let input = queries::with_provider_runs(input, true);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_mutation_generation_can_fail_after_a_successful_resume_is_refused() {
        let schema = serde_json::json!({"$defs":{"Box":{"properties":{"action":{}}}}});
        let library = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
        });
        let inner = Arc::new(openui_lang::parse(
            "m=Mutation(\"save\")\nroot=Box(Action([@Run(m),@Set($done,true),@Run(m)]))",
            &library.inner,
            None,
        ));
        let program = Arc::new(ParsedProgram {
            json_bytes: inner.to_json().len() as u64,
            inner,
        });
        let mut s = SessionCore::new(library, None, RuntimeLimits::default()).unwrap();
        let values = JsObject::new();
        let refs = PreparedReferenceValues::new(&values);
        s.attach_tool_provider(&refs).unwrap();
        let draw = s
            .replace_program(&refs, program, None, None, true)
            .unwrap()
            .evaluation;
        let action = s
            .action_at(
                &refs,
                draw.evaluation,
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        s.next_continuation = u64::MAX - 1;
        s.run(&refs, Some(action), String::new(), None).unwrap();
        let call = s.take_query_update().calls.pop().unwrap();
        assert!(s.continuations.contains_key(&u64::MAX));
        let version = s.store.version();
        assert!(matches!(
            s.complete_tool_call_json(&call.id, r#"{"value":1}"#),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert!(s.take_query_update().actions.is_empty());
        assert_eq!(s.store.version(), version);
        s.complete_tool_call_json(&call.id, r#"{"failed":"cancelled"}"#)
            .unwrap();
        assert!(s.continuations.is_empty());
        assert_eq!(s.take_query_update().actions.len(), 1);
    }
}
