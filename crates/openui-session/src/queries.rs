//! Session policy around the clock-free query manager.
use super::*;
use openui_runtime::{QueryCommandError, QueryManager, QueryUpdate, ToolCallId};

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

#[derive(Debug)]
pub struct QueryWriteOutput {
    pub changed: bool,
    pub save_requested: bool,
    pub queries: QueryEffects,
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
    read: impl FnOnce(TreeInput<'_>) -> Result<T, OpenUiError>,
) -> Result<(T, Option<QueryUpdate>), OpenUiError> {
    let draw_with = |supply: &PreparedReferenceValues<'_>| {
        read(input.clone().with_prepared_reference_values(supply))
    };
    match manager {
        None => Ok((draw_with(refs)?, None)),
        Some(manager) if !register => Ok((draw_with(&manager.prepared_results(limits))?, None)),
        Some(manager) => manager
            .evaluate_transaction(input, store, |candidate| {
                draw_with(&candidate.prepared_results(limits))
            })
            .map(|(output, update)| (output, Some(update))),
    }
}

impl SessionCore {
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
        }
        Ok(())
    }

    pub fn detach_tool_provider(&mut self) {
        self.manager = None;
        self.query_effects = QueryEffects::default();
    }

    pub fn check_reference_values(&self) -> Result<(), OpenUiError> {
        if self.manager.is_some() {
            Err(OpenUiError::ReferenceConflict)
        } else {
            Ok(())
        }
    }

    pub fn take_query_update(&mut self) -> QueryEffects {
        std::mem::take(&mut self.query_effects)
    }

    pub(super) fn finish_draw(
        &mut self,
        streaming: bool,
        update: Option<QueryUpdate>,
    ) -> (Option<String>, QueryEffects) {
        self.streaming = streaming;
        self.query_inputs_changed = streaming;
        if let Some(update) = update {
            self.query_effects.evaluation_error = None;
            self.query_effects.record(update);
        }
        let snapshot = self.manager.as_ref().map(|m| m.snapshot_json().to_owned());
        // A blocked draw starts no calls. Any unclaimed dispatch from an
        // earlier store operation remains available to its driver.
        let effects = if streaming {
            QueryEffects::default()
        } else {
            self.take_query_update()
        };
        (snapshot, effects)
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
            self.query_inputs_changed = result.is_err();
            match result {
                Ok(update) => {
                    self.query_effects.evaluation_error = None;
                    self.query_effects.record(update);
                }
                Err(error) => self.query_effects.evaluation_error = Some(error),
            }
        }
    }

    pub fn complete_tool_call(
        &mut self,
        call: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<QueryEffects, OpenUiError> {
        let manager = self.manager.as_mut().ok_or(OpenUiError::InvalidToolCall)?;
        let update = manager.complete_tool_call(call, outcome)?;
        self.query_effects.record(update);
        Ok(self.take_query_update())
    }

    /// A single tagged JSON outcome for both bindings. Parse the result with
    /// JavaScript number and string semantics. MCP responses enter the same
    /// completion path directly, without a stringify/parse of extracted data.
    pub fn complete_tool_call_json(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<QueryEffects, OpenUiError> {
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

    pub fn set_with_queries(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        name: String,
        json: Option<String>,
    ) -> Result<QueryWriteOutput, OpenUiError> {
        let changed = self.set(refs, name, json)?;
        Ok(QueryWriteOutput {
            changed,
            save_requested: false,
            queries: self.take_query_update(),
        })
    }
    pub fn write_binding_with_queries(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        marker: String,
        json: String,
    ) -> Result<QueryWriteOutput, OpenUiError> {
        let changed = self.write_binding(refs, marker, json)?;
        Ok(QueryWriteOutput {
            changed,
            save_requested: false,
            queries: self.take_query_update(),
        })
    }
    pub fn set_field_with_queries(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        save: bool,
    ) -> Result<QueryWriteOutput, OpenUiError> {
        let field = self.set_field(refs, form, component_type, name, json, save)?;
        Ok(QueryWriteOutput {
            changed: field.changed,
            save_requested: field.save_requested,
            queries: self.take_query_update(),
        })
    }
}
