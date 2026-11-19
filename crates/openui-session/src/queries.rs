//! Session policy around the clock-free query manager.
use super::*;
use openui_runtime::{QueryCommand, QueryCommandError, QueryManager, QueryUpdate, ToolCallId};

/// Effects since the previous handoff. A store operation can succeed even if
/// evaluating its new query arguments fails. The previous registrations survive.
#[derive(Default, Debug, Clone)]
pub struct QueryEffects {
    pub calls: Vec<Arc<WantedToolCall>>,
    pub changed: bool,
    pub evaluation_error: Option<OpenUiError>,
}

impl QueryEffects {
    fn record(&mut self, update: QueryUpdate) {
        self.calls.retain(|call| call.id.is_pending());
        self.changed |= update.changed;
        self.calls.extend(update.calls.into_iter().map(Arc::new));
    }
}

/// The host's current response gate. The default is a settled response.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResponseState {
    /// For a bundled response, pass the renderer's combined blocked condition.
    pub streaming: bool,
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
            QueryCommandError::Limit(_) => Self::LimitExceeded { reason },
            _ => Self::Evaluation { reason },
        }
    }
}

// Do the query effect before drawing. A rejected drawing must not commit the
// proposed registrations, cache ownership changes or calls.
pub(super) fn draw<T>(
    manager: Option<&mut QueryManager>,
    register: bool,
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
        Some(manager) if !register => Ok((draw_with(&manager.prepared_results(limits))?, None)),
        Some(manager) => match manager.evaluate_transaction(input, store, |candidate| {
            draw_with(&candidate.prepared_results(limits)).map_err(DrawError::Screen)
        }) {
            Ok((output, update)) => Ok((output, Some(Ok(update)))),
            Err(DrawError::Registration(error @ QueryCommandError::Limit(_))) => Ok((
                draw_with(&manager.prepared_results(limits))?,
                Some(Err(error.into())),
            )),
            Err(DrawError::Registration(error)) => Err(error.into()),
            Err(DrawError::Screen(error)) => Err(error),
        },
    }
}

// With a tool provider, an action's `@Run` of a query runs it through the
// manager instead of handing the host a `run` event.
pub(super) fn with_query_runs(input: TreeInput<'_>, managed: bool) -> TreeInput<'_> {
    if managed {
        input.with_query_runs()
    } else {
        input
    }
}

impl SessionCore {
    /// Upstream's renderer Retry (`retryQueries`): run again every query
    /// whose error is in the query snapshot, in the snapshot's error order.
    /// The wanted calls come from the next `take_query_update`. As upstream
    /// passes the list to `invalidate`, an empty list runs every query. With
    /// no tool provider, or while the response streams, it does nothing, as
    /// upstream's renderer has no provider to call or ignores Retry while
    /// its bundle is blocked.
    pub fn retry_failed_queries(&mut self) -> Result<(), OpenUiError> {
        let Some(manager) = self.manager.as_mut().filter(|_| !self.streaming) else {
            return Ok(());
        };
        let failed = match manager.snapshot().get("__openui_errors") {
            Some(Value::Arr(errors)) => errors
                .iter()
                .filter_map(|error| match error {
                    Value::Obj(error)
                        if matches!(error.get("source"), Some(Value::Str(s)) if s == "query") =>
                    {
                        match error.get("statementId") {
                            Some(Value::Str(id)) => Some(id.clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let ids = failed.iter().map(String::as_str).collect::<Vec<_>>();
        let update = manager.apply(QueryCommand::Invalidate {
            statement_ids: &ids,
        })?;
        self.query_effects.record(update);
        Ok(())
    }

    // Each `@Run` of a query, in step order, as upstream's renderer calls
    // `invalidate([statementId])` once per step. Nothing runs while the
    // response streams: upstream's renderer ignores the click then.
    pub(super) fn run_queries(&mut self, ids: &[String]) -> Result<(), OpenUiError> {
        let Some(manager) = self.manager.as_mut().filter(|_| !self.streaming) else {
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
        if !refs.is_empty() {
            return Err(OpenUiError::ReferenceConflict);
        }
        if self.manager.is_none() {
            self.manager = Some(QueryManager::with_limits(self.core_limits));
            self.query_inputs_changed = true;
            self.query_effects.changed = true;
        }
        Ok(())
    }

    pub fn detach_tool_provider(&mut self) {
        if self.manager.take().is_some() {
            self.query_effects = QueryEffects {
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
            calls: std::mem::take(&mut self.query_effects.calls),
            changed: std::mem::take(&mut self.query_effects.changed),
            evaluation_error: self.query_effects.evaluation_error.clone(),
        }
    }

    fn record_registration(&mut self, result: Registration) {
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
        response: ResponseState,
        registration: Option<Registration>,
    ) -> (Option<String>, Option<OpenUiError>) {
        self.streaming = response.streaming;
        self.query_inputs_changed = false;
        if let Some(result) = registration {
            self.record_registration(result);
        }
        (
            self.manager.as_ref().map(|m| m.snapshot_json().to_owned()),
            self.query_effects.evaluation_error.clone(),
        )
    }

    pub(super) fn state_changed(&mut self) {
        if !self.streaming
            && let Some(manager) = &mut self.manager
        {
            let result = Self::functions(
                &self.library,
                &self.functions,
                self.core_limits,
                self.limits.max_json_bytes,
            )
            .and_then(|functions| {
                let input = program_input(
                    &self.program,
                    &self.library,
                    &self.bindings,
                    functions.as_ref(),
                );
                manager
                    .evaluate_transaction(&input, &self.store, |_| Ok::<_, OpenUiError>(()))
                    .map(|(_, update)| update)
            });
            self.query_inputs_changed = false;
            self.record_registration(result);
        }
    }

    pub fn complete_tool_call(
        &mut self,
        call: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<(), OpenUiError> {
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let update = manager.complete_tool_call(call, outcome)?;
        self.query_effects.record(update);
        Ok(())
    }

    /// A single tagged JSON outcome for both bindings. Parse the result with
    /// JavaScript number and string semantics. MCP responses enter the same
    /// completion path directly, without a stringify/parse of extracted data.
    pub fn complete_tool_call_json(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<(), OpenUiError> {
        self.manager
            .as_ref()
            .ok_or(OpenUiError::InvalidToolCall)?
            .check_call(call)?;
        self.text(json)?;
        let limits = self.core_limits;
        let bound = limits.result_size().min(limits.work_for(usize::MAX));
        if json.len() > bound {
            return Err(limit("tool outcome byte limit"));
        }
        let invalid = || OpenUiError::InvalidJson {
            reason: "expected one of value, error, failed, availableTools, mcpResponseJson".into(),
        };
        let value = openui_lang::parse_json_value(json, limits.depth()).map_err(|e| {
            if e.depth_limited() {
                limit("tool outcome depth limit")
            } else {
                OpenUiError::InvalidJson {
                    reason: e.to_string(),
                }
            }
        })?;
        let Value::Obj(fields) = &value else {
            return Err(invalid());
        };
        if fields.len() != 1 {
            return Err(invalid());
        }
        let outcome = match fields.iter().next().map(|(k, v)| (k.as_str(), v)) {
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
                openui_runtime::extract_mcp_tool_result(response, limits)
                    .map_err(mcp::error)?
                    .into()
            }
            _ => return Err(invalid()),
        };
        self.complete_tool_call(call, outcome)
    }
}
