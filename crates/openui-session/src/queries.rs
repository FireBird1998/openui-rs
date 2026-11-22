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
    /// A command reached the reader round bound with inputs still changing.
    pub reader_rounds_limited: bool,
    pub evaluation_error: Option<OpenUiError>,
}

impl QueryEffects {
    pub(super) fn record(&mut self, update: QueryUpdate) {
        self.record_for(update, true);
    }

    /// Record an update of the registrations. Unless they speak for the
    /// drawn program, keep only its calls: they stay with the session (see
    /// `take_query_update`), and its redraw and round cut describe another
    /// program's snapshot, which nothing draws (#142).
    pub(super) fn record_for(&mut self, update: QueryUpdate, drawn: bool) {
        self.calls.retain(|call| call.id.is_pending());
        if drawn {
            self.changed |= update.needs_redraw();
            self.reader_rounds_limited |= update.reader_rounds_limited;
        }
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
            QueryCommandError::InvalidClock
            | QueryCommandError::Limit(_)
            | QueryCommandError::SourceCut { .. } => Self::LimitExceeded { reason },
            _ => Self::Evaluation { reason },
        }
    }
}

/// Which program the manager's registrations come from (#119).
#[derive(Default)]
pub(super) struct Registered {
    /// Share at most one accepted parse result across refused replacements.
    program: Option<Arc<ParseResult>>,
    pub(super) drawn: Drawn,
    /// Queries resumed and no registration has been accepted since. The
    /// pause released every call, and only a registration asks for them
    /// again (#142).
    pub(super) resumed: bool,
}

impl Registered {
    /// Compare only on installation, before a replacement draws. Its caller
    /// commits the answer together with the program after the draw succeeds.
    pub(super) fn for_install(&self, current: &Arc<ParseResult>, next: &Arc<ParseResult>) -> Drawn {
        if Arc::ptr_eq(current, next) {
            return self.drawn;
        }
        let belongs = self.program.as_ref().is_some_and(|registered| {
            #[cfg(test)]
            STATEMENT_COMPARISONS.set(STATEMENT_COMPARISONS.get() + 1);
            Arc::ptr_eq(registered, next) || registered.same_registration_inputs(next)
        });
        Drawn {
            belongs,
            ..self.drawn
        }
    }
}

/// The registrations as a draw of one program sees them.
#[derive(Clone, Copy, Default)]
pub(super) struct Drawn {
    pub(super) belongs: bool,
    pub(super) refused: bool,
}

/// The one rule (#119): a program is unregistered while the registrations
/// belong to another program and registering it was refused. Nothing of the
/// other program is then drawn, read by an action, called or refreshed.
pub(super) fn unregistered(belongs: bool, refused: bool) -> bool {
    refused && !belongs
}

/// The query and mutation results a draw or an action reads. Unregistered,
/// none: each query reads its own default and each mutation is idle.
pub(super) fn supply(
    manager: &QueryManager,
    unregistered: bool,
    limits: Limits,
) -> PreparedReferenceValues<'static> {
    if unregistered {
        PreparedReferenceValues::owned_with_limits(JsObject::new(), limits)
    } else {
        manager.prepared_results(limits)
    }
}

/// The query snapshot of an unregistered program: nothing loading, fetched
/// or failed. The refusal is the registration's error beside it.
const UNREGISTERED_SNAPSHOT: &str =
    r#"{"__openui_loading":[],"__openui_refetching":[],"__openui_errors":[]}"#;

// Do the query effect before drawing. A rejected drawing must not commit the
// proposed registrations, cache ownership changes or calls. A program whose
// registration is refused draws its queries' defaults, never results held for
// another program's statements (#119).
pub(super) fn draw<T>(
    manager: Option<&mut QueryManager>,
    registration: (RegistrationEffects, Option<u64>, Drawn),
    input: &TreeInput<'_>,
    store: &Store,
    refs: &PreparedReferenceValues<'_>,
    limits: Limits,
    read: impl Fn(TreeInput<'_>) -> Result<T, OpenUiError>,
) -> Result<(T, Option<Registration>), OpenUiError> {
    let (effects, now_ms, registered) = registration;
    let draw_with = |supply: &PreparedReferenceValues<'_>| {
        read(input.clone().with_prepared_reference_values(supply))
    };
    let held = |manager: &QueryManager, refused: bool| {
        supply(manager, unregistered(registered.belongs, refused), limits)
    };
    match manager {
        None => Ok((draw_with(refs)?, None)),
        Some(manager) if effects == RegistrationEffects::None || manager.is_disposed() => {
            Ok((draw_with(&held(manager, registered.refused))?, None))
        }
        Some(manager) => {
            let read = |candidate: &QueryManager| {
                draw_with(&candidate.prepared_results(limits)).map_err(DrawError::Screen)
            };
            let result = manager.registration_transaction(
                input,
                store,
                effects == RegistrationEffects::QueriesAndMutations,
                now_ms,
                read,
            );
            match result {
                Ok((output, update)) => Ok((output, Some(Ok(update)))),
                Err(DrawError::Registration(
                    error @ (QueryCommandError::Limit(_)
                    | QueryCommandError::SourceCut { .. }
                    | QueryCommandError::Evaluation(_)),
                )) => Ok((draw_with(&held(manager, true))?, Some(Err(error.into())))),
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
        // Unregistered, the failed queries are another program's (#119).
        if self.response.bundle_blocked || self.is_unregistered() {
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
        // Unregistered, the registered queries are another program's (#119).
        if self.is_unregistered() {
            return Ok(());
        }
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
        self.registered = Registered::default();
        self.query_effects.changed = true;
        Ok(())
    }

    pub fn detach_tool_provider(&mut self) {
        if self.manager.take().is_some() {
            self.registered = Registered::default();
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

    fn query_error(&self) -> Option<OpenUiError> {
        self.query_effects.evaluation_error.clone().or_else(|| {
            match self.manager.as_ref()?.reader_diagnostic()? {
                openui_runtime::ReaderDiagnostic::SelfInputChanged(id) => {
                    Some(OpenUiError::Evaluation {
                        reason: format!(
                            "query {id} now displays a result fetched using its previous self-input"
                        ),
                    })
                }
                openui_runtime::ReaderDiagnostic::PropagationLimit(e) => {
                    Some(OpenUiError::LimitExceeded {
                        reason: e.to_string(),
                    })
                }
                _ => None,
            }
        })
    }

    /// Conformance-test hook: check the manager's cache rules against the
    /// receipts the driver holds and those the session has not handed out.
    /// Outside the supported interface.
    #[doc(hidden)]
    pub fn audit_query_cache<'a>(
        &'a self,
        driver: impl Iterator<Item = &'a ToolCallId>,
    ) -> Result<(), String> {
        let Some(manager) = self.manager.as_ref() else {
            return Ok(());
        };
        let held = self.query_effects.calls.iter().map(|call| &call.id);
        manager.audit_cache(driver.chain(held), true)
    }

    /// Whether the drawn program is unregistered (see [`unregistered`]).
    pub(super) fn is_unregistered(&self) -> bool {
        self.manager.is_some()
            && unregistered(self.registered.drawn.belongs, self.registered.drawn.refused)
    }

    /// The results an action or a resumed plan reads for the drawn program.
    pub(super) fn query_supply(&self) -> Option<PreparedReferenceValues<'static>> {
        let unregistered = self.is_unregistered();
        self.manager
            .as_ref()
            .map(|manager| supply(manager, unregistered, self.reference_limits))
    }

    /// The query snapshot beside a draw. Unregistered, it describes the
    /// drawn program, which has nothing loading, fetched or failed.
    pub(super) fn query_snapshot_json(&self) -> Option<String> {
        let manager = self.manager.as_ref()?;
        Some(if self.is_unregistered() {
            UNREGISTERED_SNAPSHOT.to_owned()
        } else {
            manager.snapshot_json().to_owned()
        })
    }

    /// A program the registrations did not belong to has registered. Query
    /// calls still undrained were wanted by the other program's
    /// registrations. The new registration requested none of them (an entry
    /// with a call in flight requests no other), so one whose key is current
    /// for a new query is that query's call and goes out once; one whose key
    /// only a previous slot holds is ended by the manager without delivery
    /// (#119). Mutation calls go out as they are.
    fn abandon_held_query_calls(&mut self) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        self.query_effects
            .calls
            .retain(|call| !manager.abandon_unperformed_call(&call.id));
    }

    /// The manager whose snapshot speaks for the drawn program, if any.
    pub(super) fn drawn_manager(&self) -> Option<&QueryManager> {
        self.manager.as_ref().filter(|_| !self.is_unregistered())
    }

    pub fn take_query_update(&mut self) -> QueryEffects {
        // Unregistered, calls the registrations want stay with the session:
        // none is for the drawn program. A registration hands over those
        // still pending; a new program's releases the rest (#119).
        self.query_effects.calls.retain(|call| call.id.is_pending());
        let unregistered = self.is_unregistered();
        // The failures a pause left are handed over with the actions.
        self.paused_failures = 0;
        QueryEffects {
            next_refresh_due_ms: self
                .manager
                .as_ref()
                .filter(|_| !unregistered)
                .and_then(QueryManager::next_refresh_due_ms),
            actions: std::mem::take(&mut self.query_effects.actions),
            calls: if unregistered {
                Vec::new()
            } else {
                std::mem::take(&mut self.query_effects.calls)
            },
            changed: std::mem::take(&mut self.query_effects.changed),
            reader_rounds_limited: std::mem::take(&mut self.query_effects.reader_rounds_limited),
            evaluation_error: self.query_error(),
        }
    }

    fn record_registration(&mut self, result: Registration, register_statements: bool) {
        // Repeat the refused effect on the next eligible draw. A key-only
        // change must not register mutations.
        self.query_inputs_changed = result.is_err() && register_statements;
        self.extra_values_changed = result.is_err();
        // Every registration evaluates the current program (a replacement
        // records its own after installing the program). Only a refused
        // program the registrations do not belong to is unregistered; a
        // refused re-registration keeps its own program's results.
        let belongs = self.registered.drawn.belongs;
        self.registered.drawn.refused = result.is_err() && !belongs;
        // The first registration after a resume: if it re-registers the
        // program the registrations belong to and is refused, that program
        // keeps its results but nothing asked again for the calls the pause
        // released (#142).
        let owed = belongs && std::mem::take(&mut self.registered.resumed);
        let error = match result {
            Ok(update) => {
                self.registered.program = Some(self.program.clone());
                if !belongs {
                    self.registered.drawn.belongs = true;
                    self.abandon_held_query_calls();
                }
                self.registered.resumed = false;
                self.query_effects.record(update);
                None
            }
            Err(error) => {
                if owed && let Some(manager) = self.manager.as_mut() {
                    self.query_effects.record(manager.request_owed_calls());
                }
                Some(error)
            }
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
        (self.query_snapshot_json(), self.query_error())
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
        self.complete_delivery(call, mutations::Delivery::Outcome(outcome))
    }

    fn complete_delivery(
        &mut self,
        call: &ToolCallId,
        outcome: mutations::Delivery,
    ) -> Result<(), OpenUiError> {
        if let Some((&generation, _)) = self
            .continuations
            .iter()
            .find(|(_, entry)| entry.call.as_ref() == Some(call))
        {
            return self.complete_mutation_delivery(generation, call, outcome);
        }
        // Unregistered, the delivery is another program's (#142).
        let drawn = !self.is_unregistered();
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let delivery = outcome.stage(manager, call)?;
        let update = if delivery.needs_reader_context() {
            let functions = match Self::functions(
                &self.library,
                &self.functions,
                self.core_limits,
                self.limits.max_json_bytes,
            ) {
                Ok(functions) => functions,
                Err(_) if delivery.result_refusal().is_some() => {
                    self.query_effects.record_for(delivery.finish(None)?, drawn);
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let input = program_input(
                &self.program,
                &self.library,
                &self.bindings,
                functions.as_ref(),
            );
            delivery.finish(Some((&self.response.input(input), &self.store)))?
        } else {
            delivery.finish(None)?
        };
        self.query_effects.record_for(update, drawn);
        Ok(())
    }

    /// Deliver a result refused under `limit` before it was read as the
    /// call's `tool-error`. It always ends the call (#90).
    pub(super) fn complete_refused(
        &mut self,
        call: &ToolCallId,
        limit: openui_runtime::ResultLimit,
    ) -> Result<(), OpenUiError> {
        self.complete_delivery(call, mutations::Delivery::Refused(limit))
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
        if manager.discards_tool_outcome(call)? {
            return self.complete_tool_call(call, ToolCallOutcome::Result(Value::Null));
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

#[cfg(test)]
thread_local! {
    pub(super) static TABLE_BUILDS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
    static STATEMENT_COMPARISONS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawing_and_draining_a_refused_reread_do_not_repeat_the_statement_comparison() {
        let schema = serde_json::json!({"$defs":{}});
        let lib = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
            defined: None,
        });
        let parse = || {
            let inner = Arc::new(openui_lang::parse(
                "$tool=\"ok\"\nq=Query($tool)",
                &lib.inner,
                None,
            ));
            Arc::new(ParsedProgram {
                json_bytes: inner.to_json().len() as u64,
                inner,
            })
        };
        let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
        let mut session = SessionCore::new(lib.clone(), None, Default::default()).unwrap();
        session.attach_tool_provider(&refs).unwrap();
        session
            .replace_program(&refs, parse(), None, None, false)
            .unwrap();
        session
            .set(&refs, "$tool".into(), Some(r#"{"toString":null}"#.into()))
            .unwrap();
        STATEMENT_COMPARISONS.set(0);
        session
            .replace_program(&refs, parse(), None, None, false)
            .unwrap();
        for _ in 0..3 {
            assert!(
                session
                    .evaluate(&refs)
                    .unwrap()
                    .query_evaluation_error
                    .is_some()
            );
            assert!(session.take_query_update().evaluation_error.is_some());
        }
        assert_eq!(STATEMENT_COMPARISONS.get(), 1);
    }

    #[test]
    fn an_unrelated_completion_skips_the_function_table_even_with_a_registered_reader() {
        let schema = serde_json::json!({"$defs":{}});
        let lib = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
            defined: None,
        });
        let mut s = SessionCore::new(lib.clone(), None, Default::default()).unwrap();
        let inner = Arc::new(openui_lang::parse(
            "a=Query(\"a\")\nb=Query(\"b\",{v:a})\nc=Query(\"c\")",
            &lib.inner,
            None,
        ));
        let p = Arc::new(ParsedProgram {
            json_bytes: inner.to_json().len() as u64,
            inner,
        });
        let refs = PreparedReferenceValues::owned_with_limits(
            openui_lang::JsObject::new(),
            Limits::DEFAULT,
        );
        s.attach_tool_provider(&refs).unwrap();
        s.replace_program(&refs, p, None, None, true).unwrap();
        let calls = s.take_query_update().calls;
        TABLE_BUILDS.set(0);
        s.complete_tool_call(&calls[1].id, ToolCallOutcome::Result(Value::Null))
            .unwrap();
        assert_eq!(TABLE_BUILDS.get(), 0);
        s.complete_tool_call(&calls[0].id, ToolCallOutcome::Result(Value::Null))
            .unwrap();
        assert_eq!(TABLE_BUILDS.get(), 1);
    }
}
