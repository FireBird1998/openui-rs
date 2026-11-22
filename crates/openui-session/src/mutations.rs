//! Managed mutations retain their action continuation in the session. The
//! embedder needs only the tool call identity to complete the segment.
use super::*;
use openui_runtime::{QueryManager, ResultLimit, ToolCallId};

/// What completes a call: the provider's outcome, or a result the session
/// refused under a limit before reading it (#90).
pub(super) enum Delivery {
    Outcome(ToolCallOutcome),
    Refused(ResultLimit),
}

impl Delivery {
    pub(super) fn stage<'a>(
        self,
        manager: &'a mut QueryManager,
        call: &'a ToolCallId,
    ) -> Result<openui_runtime::ToolDelivery<'a>, openui_runtime::QueryCommandError> {
        match self {
            Self::Outcome(outcome) => manager.stage_tool_call(call, outcome),
            Self::Refused(limit) => manager.stage_tool_call_refused(call, limit),
        }
    }
}

/// A mutation step reached while queries are paused, or one whose call the
/// session still held, never handed to a driver, when they paused. No tool
/// provider can have been asked to run it.
const PAUSED_BEFORE_SEND: &str = "queries are paused, so the mutation was not sent to the tool provider and the rest of the action did not run";
/// A mutation whose call had been handed to a driver when queries paused.
/// The session cannot know whether the driver invoked the provider.
const PAUSED_IN_FLIGHT: &str = "queries were paused after the mutation's call was handed to the tool provider, so the tool may have run; its outcome is ignored and the rest of the action did not run";

fn paused(reason: &str) -> OpenUiError {
    OpenUiError::QueriesPaused {
        reason: reason.into(),
    }
}

impl SessionCore {
    /// Pausing releases every managed plan whose call is in flight. Each
    /// ends with one `QueriesPaused` failure at its mutation step, delivered
    /// as a resumed segment by the next drain, in the order they were
    /// clicked (#117). Its late outcome finds no plan and is ignored. A call
    /// still among the undrained calls was never handed to a driver, so its
    /// reason says it was not sent. Call before the undrained calls are
    /// cleared.
    pub(super) fn drop_paused_plans(&mut self) {
        let undrained = std::mem::take(&mut self.query_effects.calls);
        let continuations = std::mem::take(&mut self.continuations);
        for (generation, entry) in continuations {
            if entry.call.is_none() {
                self.continuations.insert(generation, entry);
                continue;
            }
            self.paused_failures = self.paused_failures.saturating_add(1);
            let held = undrained
                .iter()
                .any(|call| entry.call.as_ref() == Some(&call.id));
            let reason = if held {
                PAUSED_BEFORE_SEND
            } else {
                PAUSED_IN_FLIGHT
            };
            self.query_effects.actions.push(ActionOutput {
                failure: Some(ActionFailureOutput {
                    step: entry.step,
                    error: paused(reason),
                }),
                ..ActionOutput::default()
            });
        }
    }

    pub(super) fn dispatch_mutation(
        &mut self,
        request: openui_runtime::MutationRequest,
        continuation: ActionContinuation,
        generation: u64,
        output: &mut ActionOutput,
    ) {
        let unregistered = self.is_unregistered();
        let refusal = self.query_effects.evaluation_error.clone();
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let Some(name) = request.statement else {
            return;
        };
        // Unregistered, the registered mutation is another program's and may
        // call another tool (#119). The step fails for the registration's
        // error before sending a call, and the plan ends.
        if unregistered {
            let error = refusal.unwrap_or_else(|| OpenUiError::Evaluation {
                reason: "query registration refused".into(),
            });
            output.size_limited = matches!(error, OpenUiError::LimitExceeded { .. });
            output.failure = Some(ActionFailureOutput {
                step: Some(request.step as u64),
                error,
            });
            return;
        }
        // Paused, the manager sends nothing until resume and the plan would
        // vanish (#117). The step fails before any call, like the limit
        // failures here: the steps before it keep their effects and the rest
        // of the plan does not run.
        if manager.is_disposed() {
            output.failure = Some(ActionFailureOutput {
                step: Some(request.step as u64),
                error: paused(PAUSED_BEFORE_SEND),
            });
            return;
        }
        if !manager.can_fire_mutation(&name) {
            return;
        }
        // A managed run has promised a resume. Report a cut on this step
        // before dispatch if all the session's continuation slots are
        // occupied, a failure a pause left for the next drain included.
        let occupied = (self.continuations.len() as u64).saturating_add(self.paused_failures);
        let started = if occupied >= self.limits.max_continuations {
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
                            step: Some(request.step as u64),
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

    pub(super) fn complete_mutation_delivery(
        &mut self,
        generation: u64,
        call: &ToolCallId,
        delivery: Delivery,
    ) -> Result<(), OpenUiError> {
        let delivers_result = matches!(
            delivery,
            Delivery::Outcome(ToolCallOutcome::Result(_) | ToolCallOutcome::MissingResult)
        );
        let generation_room = self.next_continuation_generation();
        // Unregistered, the mutation result is another program's (#142).
        let drawn = !self.is_unregistered();
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let staged = delivery.stage(manager, call)?;
        let succeeded = delivers_result && staged.result_refusal().is_none();
        let functions = if succeeded {
            generation_room?;
            Self::functions(
                &self.library,
                &self.functions,
                self.core_limits,
                self.limits.max_json_bytes,
            )?
        } else {
            // Failure ends the plan without evaluating its tail or any readers.
            None
        };
        let update = staged.finish(None)?;
        self.query_effects.record_for(update, drawn);
        let pending = self
            .continuations
            .remove(&generation)
            .ok_or(OpenUiError::ConsumedHandle)?;
        let refs = self.query_supply().ok_or(OpenUiError::InvalidToolCall)?;
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

    fn submission() -> (
        SessionCore,
        PreparedReferenceValues<'static>,
        Arc<ActionHandle>,
    ) {
        let schema = serde_json::json!({"$defs":{"Box":{"properties":{"action":{}}}}});
        let library = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
            defined: None,
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
        let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
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
        (s, refs, action)
    }

    #[test]
    fn evicting_a_provider_run_reports_its_step_once_on_the_next_drain() {
        let (mut s, refs, action) = submission();
        s.limits.max_continuations = 1;
        s.run(&refs, Some(action.clone()), String::new(), None)
            .unwrap();
        let call = s.take_query_update().calls.pop().unwrap();

        // Exercise the session's host-handoff boundary with an actual suspended
        // host run. Public runs while attached always use the managed path.
        let plan = s.action_value(&action.path).unwrap().clone();
        let input = TreeInput::new(&s.program);
        let host_run = openui_runtime::run_action_with_limits(
            input,
            &mut s.store,
            Some(&plan),
            "",
            None,
            s.core_limits,
        );
        let host = s.output(host_run).unwrap();
        assert!(host.continuation.is_some());
        let update = s.take_query_update();
        let [ended] = update.actions.as_slice() else {
            panic!("one evicted plan")
        };
        let failure = ended.failure.as_ref().unwrap();
        assert_eq!(failure.step, Some(0));
        assert_eq!(failure.error, OpenUiError::EvictedHandle);
        assert_eq!(failure.error.to_string(), "continuation evicted");
        assert!(ended.events.is_empty() && ended.continuation.is_none());
        s.complete_tool_call_json(&call.id, r#"{"value":1}"#)
            .unwrap();
        assert!(s.take_query_update().actions.is_empty());
        assert_ne!(
            s.get(&refs, "$done".into()).unwrap().as_deref(),
            Some("true")
        );
    }

    #[test]
    fn a_failed_submission_ends_the_plan_without_building_the_function_table() {
        let (mut s, refs, action) = submission();
        s.run(&refs, Some(action), String::new(), None).unwrap();
        let call = s.take_query_update().calls.pop().unwrap();
        queries::TABLE_BUILDS.set(0);
        s.complete_tool_call_json(&call.id, r#"{"failed":"cancelled"}"#)
            .unwrap();
        assert_eq!(queries::TABLE_BUILDS.get(), 0);
        let update = s.take_query_update();
        assert_eq!(update.actions.len(), 1);
        assert!(update.calls.is_empty());
    }

    #[test]
    fn a_cache_refusal_ends_the_last_mutation_generation_without_evaluating_its_tail() {
        let (mut s, refs, action) = submission();
        s.next_continuation = u64::MAX - 1;
        s.manager = s.manager.take().map(|manager| manager.with_cache_limit(0));
        s.run(&refs, Some(action), String::new(), None).unwrap();
        let call = s.take_query_update().calls.pop().unwrap();
        queries::TABLE_BUILDS.set(0);
        s.complete_tool_call_json(&call.id, r#"{"value":1}"#)
            .unwrap();
        assert_eq!(queries::TABLE_BUILDS.get(), 0);
        assert!(s.continuations.is_empty());
        let update = s.take_query_update();
        assert!(update.calls.is_empty());
        assert_eq!(update.actions.len(), 1);
        assert!(!call.id.is_pending());
    }

    #[test]
    fn the_last_mutation_generation_can_fail_after_a_successful_resume_is_refused() {
        let (mut s, refs, action) = submission();
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
