//! Session policy around the clock-free query manager.
use super::*;
use openui_runtime::{QueryCommand, QueryCommandError, QueryManager, QueryUpdate, ToolCallId};

/// Effects since the previous handoff. A store operation can succeed even if
/// evaluating its new query arguments fails. The previous registrations survive.
#[derive(Default, Debug, Clone)]
pub struct QueryEffects {
    /// Current absolute refresh deadline, repeated even on an empty drain.
    pub next_refresh_due_ms: Option<u64>,
    pub calls: Vec<Arc<WantedToolCall>>,
    /// Resumed action segments in completion order. Drained exactly once.
    pub actions: Vec<ActionOutput>,
    pub changed: bool,
    pub evaluation_error: Option<OpenUiError>,
}

impl QueryEffects {
    pub(super) fn record(&mut self, update: QueryUpdate) {
        self.calls.retain(|call| call.id.is_pending());
        self.changed |= update.needs_redraw();
        self.calls.extend(update.calls.into_iter().map(Arc::new));
    }
}

type Registration = Result<QueryUpdate, OpenUiError>;

enum DrawError {
    Registration(QueryCommandError),
    Screen(OpenUiError),
}

impl From<QueryCommandError> for DrawError {
    fn from(error: QueryCommandError) -> Self {
        Self::Registration(error)
    }
}

impl From<QueryCommandError> for OpenUiError {
    fn from(error: QueryCommandError) -> Self {
        let reason = error.to_string();
        match error {
            QueryCommandError::InvalidCall => Self::InvalidToolCall,
            QueryCommandError::InvalidClock | QueryCommandError::Limit(_) => {
                Self::LimitExceeded { reason }
            }
            _ => Self::Evaluation { reason },
        }
    }
}

// Do the query effect before drawing. A rejected drawing must not commit the
// proposed registrations, cache ownership changes or calls.
pub(super) fn draw<T>(
    manager: Option<&mut QueryManager>,
    effects: RegistrationEffects,
    input: &TreeInput<'_>,
    store: &Store,
    refs: &PreparedReferenceValues<'_>,
    limits: Limits,
    read: impl Fn(TreeInput<'_>) -> Result<T, OpenUiError>,
) -> Result<(T, Option<Registration>), OpenUiError> {
    let draw_with = |supply: &PreparedReferenceValues<'_>| {
        read(input.clone().with_prepared_reference_values(supply))
    };
    match manager {
        None => Ok((draw_with(refs)?, None)),
        Some(manager) if effects == RegistrationEffects::None || manager.is_disposed() => {
            Ok((draw_with(&manager.prepared_results(limits))?, None))
        }
        Some(manager) => {
            let read = |candidate: &QueryManager| {
                draw_with(&candidate.prepared_results(limits)).map_err(DrawError::Screen)
            };
            let result = if effects == RegistrationEffects::QueriesAndMutations {
                manager.settled_transaction(input, store, read)
            } else {
                manager.evaluate_transaction(input, store, read)
            };
            match result {
                Ok((output, update)) => Ok((output, Some(Ok(update)))),
                Err(DrawError::Registration(error @ QueryCommandError::Limit(_))) => Ok((
                    draw_with(&manager.prepared_results(limits))?,
                    Some(Err(error.into())),
                )),
                Err(DrawError::Registration(error)) => Err(error.into()),
                Err(DrawError::Screen(error)) => Err(error),
            }
        }
    }
}

// Both Run targets use the provider for initial and resumed action segments.
pub(super) fn with_provider_runs(input: TreeInput<'_>, managed: bool) -> TreeInput<'_> {
    if managed {
        input.with_query_runs().with_managed_mutations()
    } else {
        input
    }
}

impl SessionCore {
    /// Upstream's renderer Retry (`retryQueries`): run again every query
    /// represented by an error in the snapshot, including a compact error,
    /// in registration order.
    /// The wanted calls come from the next `take_query_update`. As upstream
    /// passes the list to `invalidate`, an empty list runs every query. With
    /// no tool provider or while a response bundle is blocked it does nothing.
    pub fn retry_failed_queries(&mut self) -> Result<(), OpenUiError> {
        if self.response.bundle_blocked {
            return Ok(());
        }
        let Some(manager) = self.manager.as_mut() else {
            return Ok(());
        };
        let failed = manager.failed_query_ids();
        let ids = failed.iter().map(String::as_str).collect::<Vec<_>>();
        let update = manager.apply(QueryCommand::Invalidate {
            statement_ids: &ids,
        })?;
        self.query_effects.record(update);
        Ok(())
    }

    // Each `@Run` of a query, in step order, as upstream's renderer calls
    // `invalidate([statementId])` once per step, on the queries registered
    // so far. The click gate applies once, before the first step.
    pub(super) fn run_queries(&mut self, ids: &[String]) -> Result<(), OpenUiError> {
        let Some(manager) = self.manager.as_mut() else {
            return Ok(());
        };
        for id in ids {
            let update = manager.apply(QueryCommand::Invalidate {
                statement_ids: &[id.as_str()],
            })?;
            self.query_effects.record(update);
        }
        Ok(())
    }

    pub fn attach_tool_provider(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
    ) -> Result<(), OpenUiError> {
        self.attach_provider(refs, None, None)
    }

    /// Choose the polling floor. Repeated attachments must agree with the existing floor.
    pub fn attach_tool_provider_with_minimum(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        minimum_ms: u64,
    ) -> Result<(), OpenUiError> {
        self.attach_provider(refs, Some(minimum_ms), None)
    }

    /// Attach on the host's clock, or advance the existing provider with the same floor.
    pub fn attach_tool_provider_at(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        now_ms: u64,
        minimum_ms: u64,
    ) -> Result<(), OpenUiError> {
        self.attach_provider(refs, Some(minimum_ms), Some(now_ms))
    }

    fn attach_provider(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        minimum_ms: Option<u64>,
        now_ms: Option<u64>,
    ) -> Result<(), OpenUiError> {
        if !refs.is_empty() {
            return Err(OpenUiError::ReferenceConflict);
        }
        if let Some(manager) = &self.manager {
            if let Some(requested) = minimum_ms
                && requested != manager.refresh_minimum_ms()
            {
                return Err(OpenUiError::RefusedWrite {
                    reason: format!(
                        "attached provider has a {} ms refresh minimum, requested {requested} ms",
                        manager.refresh_minimum_ms()
                    ),
                });
            }
            if let Some(now) = now_ms {
                self.advance_query_clock(now)?;
            }
            return Ok(());
        }
        let cache = self
            .limits
            .max_cache_bytes
            .map_or(self.core_limits.result_size(), |bytes| {
                usize::try_from(bytes).unwrap_or(usize::MAX)
            });
        let manager = match minimum_ms {
            Some(minimum) => QueryManager::with_refresh_minimum(self.core_limits, minimum)?,
            None => QueryManager::with_limits(self.core_limits),
        };
        let mut manager = manager.with_cache_limit(cache);
        if let Some(now_ms) = now_ms {
            manager.apply(QueryCommand::AdvanceClock { now_ms })?;
        }
        self.manager = Some(manager);
        self.query_inputs_changed = true;
        self.query_effects.changed = true;
        Ok(())
    }

    pub fn detach_tool_provider(&mut self) {
        if self.manager.take().is_some() {
            self.continuations.retain(|_, entry| entry.call.is_none());
            self.query_effects = QueryEffects {
                actions: std::mem::take(&mut self.query_effects.actions),
                changed: true,
                ..QueryEffects::default()
            };
        }
    }

    pub fn check_reference_values(&self) -> Result<(), OpenUiError> {
        if self.manager.is_some() {
            Err(OpenUiError::ReferenceConflict)
        } else {
            Ok(())
        }
    }

    pub fn take_query_update(&mut self) -> QueryEffects {
        QueryEffects {
            next_refresh_due_ms: self
                .manager
                .as_ref()
                .and_then(QueryManager::next_refresh_due_ms),
            actions: std::mem::take(&mut self.query_effects.actions),
            calls: std::mem::take(&mut self.query_effects.calls),
            changed: std::mem::take(&mut self.query_effects.changed),
            evaluation_error: self.query_effects.evaluation_error.clone(),
        }
    }

    fn record_registration(&mut self, result: Registration, register_statements: bool) {
        // Repeat the refused effect on the next eligible draw. A key-only
        // change must not register mutations.
        self.query_inputs_changed = result.is_err() && register_statements;
        self.extra_values_changed = result.is_err();
        let error = match result {
            Ok(update) => {
                self.query_effects.record(update);
                None
            }
            Err(error) => Some(error),
        };
        self.query_effects.changed |= self.query_effects.evaluation_error != error;
        self.query_effects.evaluation_error = error;
    }

    pub(super) fn finish_draw(
        &mut self,
        mut response: DrawResponse,
        registration: Option<Registration>,
        register_statements: bool,
    ) -> (Option<String>, Option<OpenUiError>) {
        if let Some(result) = registration {
            if result.is_err() {
                // The screen was drawn with the previous registrations.
                response.scripts = self.response.scripts.take();
            }
            self.record_registration(result, register_statements);
        }
        self.response = response;
        (
            self.manager.as_ref().map(|m| m.snapshot_json().to_owned()),
            self.query_effects.evaluation_error.clone(),
        )
    }

    pub(super) fn state_changed(&mut self) {
        if !self.response.blocked
            && let Some(manager) = self.manager.as_mut().filter(|m| !m.is_disposed())
        {
            let result = Self::functions(
                &self.library,
                &self.functions,
                self.core_limits,
                self.limits.max_json_bytes,
            )
            .and_then(|functions| {
                let input = self.response.input(program_input(
                    &self.program,
                    &self.library,
                    &self.bindings,
                    functions.as_ref(),
                ));
                let read = |_: &QueryManager| Ok::<_, OpenUiError>(());
                if self.query_inputs_changed {
                    manager.settled_transaction(&input, &self.store, read)
                } else {
                    manager.evaluate_transaction(&input, &self.store, read)
                }
                .map(|(_, update)| update)
            });
            self.record_registration(result, self.query_inputs_changed);
        }
    }

    pub fn complete_tool_call(
        &mut self,
        call: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<(), OpenUiError> {
        if let Some((&generation, _)) = self
            .continuations
            .iter()
            .find(|(_, entry)| entry.call.as_ref() == Some(call))
        {
            return self.complete_mutation(generation, call, outcome);
        }
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let update = manager.complete_tool_call(call, outcome)?;
        self.query_effects.record(update);
        Ok(())
    }

    /// Deliver a result refused under `limit` before it was read as the
    /// call's `tool-error`. It always ends the call (#90).
    pub(super) fn complete_refused(
        &mut self,
        call: &ToolCallId,
        limit: openui_runtime::ResultLimit,
    ) -> Result<(), OpenUiError> {
        if let Some((&generation, _)) = self
            .continuations
            .iter()
            .find(|(_, entry)| entry.call.as_ref() == Some(call))
        {
            return self.complete_mutation_delivery(
                generation,
                call,
                mutations::Delivery::Refused(limit),
            );
        }
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let update = manager.complete_tool_call_refused(call, limit)?;
        self.query_effects.record(update);
        Ok(())
    }

    /// A single tagged JSON outcome for both bindings. Parse the result with
    /// JavaScript number and string semantics. MCP responses enter the same
    /// completion path directly, without a stringify/parse of extracted data.
    ///
    /// A result (or MCP response) too large or too deep for the limits is
    /// delivered as the call's `tool-error`. Malformed or wrongly shaped
    /// outcome JSON is refused and consumes nothing (#90).
    pub fn complete_tool_call_json(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<(), OpenUiError> {
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        manager.check_call(call)?;
        if manager.is_disposed() {
            manager.apply(QueryCommand::Complete {
                id: call,
                outcome: openui_runtime::ToolOutcome::Result(None),
            })?;
            return Ok(());
        }
        let limits = self.core_limits;
        if json.len() > self.outcome_byte_bound() {
            return self.complete_long_outcome(call, json);
        }
        let invalid = || OpenUiError::InvalidJson {
            reason:
                "expected one of value, undefined, error, failed, availableTools, mcpResponseJson"
                    .into(),
        };
        // The envelope is one level, so a result may nest to the depth limit.
        let value = match openui_lang::parse_json_value(json, limits.depth().saturating_add(1)) {
            Ok(value) => value,
            Err(e) if e.depth_limited() => return self.complete_deep_outcome(call, json),
            Err(e) => {
                return Err(OpenUiError::InvalidJson {
                    reason: e.to_string(),
                });
            }
        };
        let Value::Obj(fields) = &value else {
            return Err(invalid());
        };
        if fields.len() != 1 {
            return Err(invalid());
        }
        let outcome = match fields.iter().next().map(|(k, v)| (k.as_str(), v)) {
            Some(("undefined", Value::Bool(true))) => ToolCallOutcome::MissingResult,
            Some(("value", value)) => ToolCallOutcome::Result(value.clone()),
            Some(("error", Value::Str(text))) => ToolCallOutcome::McpError(text.clone()),
            Some(("failed", Value::Str(text))) => ToolCallOutcome::CallFailed(text.clone()),
            Some(("availableTools", Value::Arr(names))) => ToolCallOutcome::ToolNotFound(
                names
                    .iter()
                    .map(|name| {
                        if let Value::Str(name) = name {
                            Ok(name.clone())
                        } else {
                            Err(invalid())
                        }
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Some(("mcpResponseJson", Value::Str(response))) => {
                match openui_runtime::extract_mcp_tool_result(response, limits) {
                    Ok(outcome) => outcome.into(),
                    Err(openui_runtime::McpResponseError::Limit(error)) => {
                        return self.complete_refused_mcp(call, &error);
                    }
                    Err(error) => return Err(mcp::error(error)),
                }
            }
            _ => return Err(invalid()),
        };
        self.complete_tool_call(call, outcome)
    }
}
