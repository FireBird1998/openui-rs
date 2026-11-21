//! wasm-bindgen surface; no mutable receiver is held across JavaScript conversion.
use crate::queries::{QueryEffects, ResponseState, WantedToolCall, error_json};
use crate::session::{ReleaseGuard, Session};
use crate::{ComponentLibrary, ParsedProgram};
use openui_session as core;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

pub(crate) fn kind(e: &core::OpenUiError) -> &'static str {
    match e {
        core::OpenUiError::Disposed => "Disposed",
        core::OpenUiError::ForeignHandle => "ForeignHandle",
        core::OpenUiError::StaleHandle => "StaleHandle",
        core::OpenUiError::ConsumedHandle => "ConsumedHandle",
        core::OpenUiError::EvictedHandle => "EvictedHandle",
        core::OpenUiError::InvalidHandle => "InvalidHandle",
        core::OpenUiError::InvalidToolCall => "InvalidToolCall",
        core::OpenUiError::ReferenceConflict => "ReferenceConflict",
        core::OpenUiError::InvalidMarker => "InvalidMarker",
        core::OpenUiError::InvalidJson { .. } => "InvalidJson",
        core::OpenUiError::RefusedWrite { .. } => "RefusedWrite",
        core::OpenUiError::Evaluation { .. } => "Evaluation",
        core::OpenUiError::LimitExceeded { .. } => "LimitExceeded",
        core::OpenUiError::Worker { .. } => "Worker",
        core::OpenUiError::InvalidSchema { .. } => "InvalidSchema",
        core::OpenUiError::InvalidPromptSpec { .. } => "InvalidPromptSpec",
        core::OpenUiError::Prompt { .. } => "Prompt",
        core::OpenUiError::QueriesPaused { .. } => "QueriesPaused",
        core::OpenUiError::InvalidAttribute { .. } => "InvalidAttribute",
    }
}
/// A thrown value the package guard turns into a typed `Error`. The core
/// calls no JavaScript except registered host functions while holding a
/// session: errors and records
/// leave Rust as text, and the guard builds the objects after the call returns,
/// with intrinsics captured when the package loads. A throwing or re-entering
/// `Object.prototype.kind` setter or a patched `JSON.parse` therefore cannot
/// unwind through a borrowed Rust frame and strand wasm-bindgen's borrow flag.
fn error(e: core::OpenUiError) -> JsValue {
    thrown(kind(&e), &e.to_string())
}
/// `openui:<Kind>\n<message>`, the shape the guard recognises.
pub(crate) fn thrown(kind: &str, message: &str) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        JsValue::from_str(&format!("openui:{kind}\n{message}"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (kind, message);
        JsValue::NULL
    }
}
/// JSON text for a record; the guard parses it after Rust has returned.
fn record(v: serde_json::Value) -> String {
    v.to_string()
}
fn failure(e: core::OpenUiError) -> serde_json::Value {
    // Same stable variant names for nested refusals as for thrown errors.
    serde_json::json!({"kind": kind(&e), "message": e.to_string()})
}
#[wasm_bindgen]
pub struct EvaluationHandle {
    inner: Arc<core::EvaluationHandle>,
}
#[wasm_bindgen]
pub struct ActionHandle {
    inner: Arc<core::ActionHandle>,
}
#[wasm_bindgen]
impl ActionHandle {
    /// Used by package glue to borrow an optional handle without consuming it.
    pub fn retain(&self) -> ActionHandle {
        Self {
            inner: self.inner.clone(),
        }
    }
}
#[wasm_bindgen]
pub struct ContinuationHandle {
    inner: Arc<core::ContinuationHandle>,
    // Fields drop in declaration order: release the token, then drain its queue.
    _release: ReleaseGuard,
}
impl ContinuationHandle {
    pub(crate) fn owned(inner: Arc<core::ContinuationHandle>, release: ReleaseGuard) -> Self {
        Self {
            inner,
            _release: release,
        }
    }
}
#[wasm_bindgen(getter_with_clone)]
pub struct EvaluationOutput {
    query_evaluation_error: Option<core::OpenUiError>,
    #[wasm_bindgen(readonly, js_name = querySnapshotJson)]
    pub query_snapshot_json: Option<String>,
    #[wasm_bindgen(readonly, js_name = isQueryLoading)]
    pub is_query_loading: bool,
    #[wasm_bindgen(readonly, js_name = parserErrorJson)]
    pub parser_error_json: Option<String>,
    /// The renderer's `blocked`: the response this draw was given has not
    /// settled. Upstream's components draw disabled while it is true; the
    /// session's own click and Retry gate does not read it.
    #[wasm_bindgen(readonly)]
    pub blocked: bool,
    #[wasm_bindgen(readonly)]
    pub json: String,
    #[wasm_bindgen(readonly, js_name = sizeLimited)]
    pub size_limited: bool,
    #[wasm_bindgen(readonly, js_name = depthLimited)]
    pub depth_limited: bool,
    evaluation: Arc<core::EvaluationHandle>,
}
#[wasm_bindgen]
impl EvaluationOutput {
    #[wasm_bindgen(getter, js_name = queryEvaluationError, unchecked_return_type = "{ kind: string; message: string } | null")]
    pub fn query_evaluation_error(&self) -> String {
        error_json(self.query_evaluation_error.as_ref())
    }
    #[wasm_bindgen(getter)]
    pub fn evaluation(&self) -> EvaluationHandle {
        EvaluationHandle {
            inner: self.evaluation.clone(),
        }
    }
}
impl From<core::EvaluationOutput> for EvaluationOutput {
    fn from(v: core::EvaluationOutput) -> Self {
        Self {
            query_evaluation_error: v.query_evaluation_error,
            is_query_loading: v.is_query_loading,
            parser_error_json: v.parser_error_json,
            blocked: v.blocked,
            query_snapshot_json: v.query_snapshot_json,
            json: v.json,
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            evaluation: v.evaluation,
        }
    }
}
#[wasm_bindgen]
pub struct ReplacementOutput {
    inner: core::ReplacementOutput,
}
#[wasm_bindgen]
impl ReplacementOutput {
    #[wasm_bindgen(getter, unchecked_return_type = "any")]
    pub fn initialization(&self) -> String {
        initialization_record(&self.inner.initialization)
    }
    #[wasm_bindgen(getter, unchecked_return_type = "any")]
    pub fn projection(&self) -> String {
        initialization_record(&self.inner.projection)
    }
    #[wasm_bindgen(getter)]
    pub fn evaluation(&self) -> EvaluationOutput {
        let v = &self.inner.evaluation;
        EvaluationOutput {
            query_evaluation_error: v.query_evaluation_error.clone(),
            is_query_loading: v.is_query_loading,
            parser_error_json: v.parser_error_json.clone(),
            blocked: v.blocked,
            query_snapshot_json: v.query_snapshot_json.clone(),
            json: v.json.clone(),
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            evaluation: v.evaluation.clone(),
        }
    }
    #[wasm_bindgen(getter)]
    pub fn snapshot(&self) -> String {
        self.inner.snapshot.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn version(&self) -> u64 {
        self.inner.version
    }
}
fn initialization_record(v: &core::InitializationOutput) -> String {
    record(
        serde_json::json!({"changed":v.changed,"refused":v.refused.iter().map(|v|
        serde_json::json!({"name":v.name,"error":{"kind":kind(&v.error),"message":v.error.to_string()}})).collect::<Vec<_>>()}),
    )
}
#[wasm_bindgen]
pub struct ActionOutput {
    /// JSON array of the runtime's event strings, unchanged inside.
    events: String,
    /// JSON record, `null` when the run did not fail.
    failure: String,
    size_limited: bool,
    depth_limited: bool,
    continuation: Option<Arc<core::ContinuationHandle>>,
    release: ReleaseGuard,
}
#[wasm_bindgen]
impl ActionOutput {
    /// Parsed into `string[]` by the package guard.
    #[wasm_bindgen(getter, unchecked_return_type = "string[]")]
    pub fn events(&self) -> String {
        self.events.clone()
    }
    /// Parsed into the failure record (or `null`) by the package guard.
    #[wasm_bindgen(getter, unchecked_return_type = "any")]
    pub fn failure(&self) -> String {
        self.failure.clone()
    }
    #[wasm_bindgen(getter, js_name = sizeLimited)]
    pub fn size_limited(&self) -> bool {
        self.size_limited
    }
    #[wasm_bindgen(getter, js_name = depthLimited)]
    pub fn depth_limited(&self) -> bool {
        self.depth_limited
    }
    #[wasm_bindgen(getter)]
    pub fn continuation(&self) -> Option<ContinuationHandle> {
        self.continuation
            .as_ref()
            .map(|v| ContinuationHandle::owned(v.clone(), self.release.clone()))
    }
}
impl ActionOutput {
    pub(crate) fn owned(v: core::ActionOutput, release: ReleaseGuard) -> Self {
        Self {
            events: record(serde_json::Value::Array(
                v.events
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            )),
            failure: record(
                v.failure
                    .map(|f| serde_json::json!({"step": f.step, "error": failure(f.error)}))
                    .unwrap_or(serde_json::Value::Null),
            ),
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            continuation: v.continuation,
            release,
        }
    }
}
#[wasm_bindgen]
pub struct RuntimeSession {
    inner: Session,
}
#[wasm_bindgen]
impl RuntimeSession {
    #[wasm_bindgen(constructor)]
    pub fn new(
        library: &ComponentLibrary,
        prompt_spec: Option<String>,
    ) -> Result<RuntimeSession, JsValue> {
        Ok(Self {
            inner: Session::new(library.0.clone(), prompt_spec).map_err(error)?,
        })
    }
    #[wasm_bindgen(js_name = withLimits)]
    pub fn with_limits(
        library: &ComponentLibrary,
        prompt_spec: Option<String>,
        limits: &RuntimeLimits,
    ) -> Result<RuntimeSession, JsValue> {
        Ok(Self {
            inner: Session::with_limits(library.0.clone(), prompt_spec, limits.inner.clone())
                .map_err(error)?,
        })
    }
    #[wasm_bindgen(js_name = registerFunction)]
    pub fn register_function(
        &self,
        name: String,
        callback: crate::callbacks::FunctionCallback,
    ) -> Result<(), JsValue> {
        let callback = crate::callbacks::bridge(callback);
        self.inner.register_function(name, callback).map_err(error)
    }
    #[wasm_bindgen(js_name = setProgram)]
    pub fn set_program(&self, program: &ParsedProgram) -> Result<(), JsValue> {
        self.inner.set_program(program.0.clone()).map_err(error)
    }
    #[wasm_bindgen(js_name = replaceProgram)]
    pub fn replace_program(
        &self,
        program: &ParsedProgram,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
        #[wasm_bindgen(unchecked_optional_param_type = "ResponseState")] response_state: Option<
            ResponseState,
        >,
    ) -> Result<ReplacementOutput, JsValue> {
        self.inner
            .replace_program_with_response(
                program.0.clone(),
                persisted,
                projected,
                native_view,
                ResponseState::read(response_state).map_err(error)?,
            )
            .map(|inner| ReplacementOutput { inner })
            .map_err(error)
    }
    // Internal stages of the package's guarded setResponse operation. The
    // package sends the whole text (`base` undefined) or, when the text
    // continues the last accepted one, only what follows its `base` units.
    #[wasm_bindgen(js_name = __readResponseBundle, skip_typescript)]
    pub fn read_response_bundle(
        &self,
        units: &[u16],
        base: Option<u32>,
        streaming: bool,
    ) -> Result<String, JsValue> {
        self.inner
            .read_response(units, base.map(|base| base as usize), streaming)
            .map_err(error)
    }

    #[wasm_bindgen(js_name = __drawResponse, skip_typescript)]
    pub fn draw_response(
        &self,
        parse: bool,
        response: ResponseState,
        now_ms: Option<u64>,
    ) -> Result<EvaluationOutput, JsValue> {
        self.inner
            .draw_response(
                parse,
                ResponseState::read(Some(response)).map_err(error)?,
                now_ms,
            )
            .map(Into::into)
            .map_err(error)
    }

    #[wasm_bindgen(js_name = sampleQueryClock)]
    pub fn sample_query_clock(&self, now_ms: u64) -> Result<(), JsValue> {
        self.inner.sample_query_clock(now_ms).map_err(error)
    }
    #[wasm_bindgen(js_name = setQueryExtraValues)]
    pub fn set_query_extra_values(
        &self,
        tool_name: &str,
        values_json: &str,
    ) -> Result<(), JsValue> {
        self.inner
            .set_query_extra_values(tool_name, values_json)
            .map_err(error)
    }
    #[wasm_bindgen(js_name = advanceQueryClock)]
    pub fn advance_query_clock(&self, now_ms: u64) -> Result<(), JsValue> {
        self.inner.advance_query_clock(now_ms).map_err(error)
    }
    #[wasm_bindgen(js_name = pauseQueries)]
    pub fn pause_queries(&self) -> Result<(), JsValue> {
        self.inner.pause_queries().map_err(error)
    }
    #[wasm_bindgen(js_name = resumeQueries)]
    pub fn resume_queries(&self) -> Result<(), JsValue> {
        self.inner.resume_queries().map_err(error)
    }
    #[wasm_bindgen(js_name = resumeQueriesAt)]
    pub fn resume_queries_at(&self, now_ms: u64) -> Result<(), JsValue> {
        self.inner.resume_queries_at(now_ms).map_err(error)
    }
    #[wasm_bindgen(js_name = attachToolProviderAt)]
    pub fn attach_tool_provider_at(&self, now_ms: u64, minimum_ms: u64) -> Result<(), JsValue> {
        self.inner
            .attach_tool_provider_at(now_ms, minimum_ms)
            .map_err(error)
    }
    #[wasm_bindgen(js_name = attachToolProviderWithMinimum)]
    pub fn attach_tool_provider_with_minimum(&self, minimum_ms: u64) -> Result<(), JsValue> {
        self.inner
            .attach_tool_provider_with_minimum(minimum_ms)
            .map_err(error)
    }
    #[wasm_bindgen(js_name = attachToolProvider)]
    pub fn attach_tool_provider(&self) -> Result<(), JsValue> {
        self.inner.attach_tool_provider().map_err(error)
    }
    #[wasm_bindgen(js_name = detachToolProvider)]
    pub fn detach_tool_provider(&self) -> Result<(), JsValue> {
        self.inner.detach_tool_provider().map_err(error)
    }
    #[wasm_bindgen(js_name = takeQueryUpdate)]
    pub fn take_query_update(&self) -> Result<QueryEffects, JsValue> {
        self.inner
            .take_query_update()
            .map(|v| QueryEffects(v, self.inner.release_guard()))
            .map_err(error)
    }
    /// Run again every query whose error is in the query snapshot, as
    /// upstream's renderer Retry does. Claim the calls with `takeQueryUpdate`.
    #[wasm_bindgen(js_name = retryFailedQueries)]
    pub fn retry_failed_queries(&self) -> Result<(), JsValue> {
        self.inner.retry_failed_queries().map_err(error)
    }
    #[wasm_bindgen(js_name = completeToolCall)]
    pub fn complete_tool_call(
        &self,
        call: &WantedToolCall,
        outcome_json: String,
        terminal: Option<bool>,
    ) -> Result<(), JsValue> {
        self.inner
            .deliver(&call.id, outcome_json, terminal == Some(true))
            .map_err(error)
    }
    #[wasm_bindgen(js_name = undecidedBindings, unchecked_return_type = "any")]
    pub fn undecided_bindings(&self) -> Result<String, JsValue> {
        let v = self.inner.undecided_bindings().map_err(error)?;
        Ok(record(serde_json::Value::Array(
            v.into_iter()
                .map(|v| serde_json::json!({"component":v.component,"prop":v.prop}))
                .collect(),
        )))
    }
    #[wasm_bindgen(js_name = markBinding)]
    pub fn mark_binding(&self, component: String, prop: String) -> Result<(), JsValue> {
        self.inner.mark_binding(component, prop).map_err(error)
    }
    #[wasm_bindgen(js_name = initialize, unchecked_return_type = "any")]
    pub fn initialize(&self, persisted: Option<String>) -> Result<String, JsValue> {
        let v = self.inner.initialize(persisted).map_err(error)?;
        Ok(record(
            serde_json::json!({"changed":v.changed,"refused":v.refused.into_iter().map(|v| serde_json::json!({"name":v.name,"error":failure(v.error)})).collect::<Vec<_>>()}),
        ))
    }
    #[wasm_bindgen(js_name = setReferenceValues)]
    pub fn set_reference_values(&self, json: String) -> Result<(), JsValue> {
        self.inner.set_reference_values(json).map_err(error)
    }
    #[wasm_bindgen(js_name = evaluate)]
    pub fn evaluate(
        &self,
        #[wasm_bindgen(unchecked_optional_param_type = "ResponseState")] response_state: Option<
            ResponseState,
        >,
    ) -> Result<EvaluationOutput, JsValue> {
        let v = self
            .inner
            .evaluate_with_response(ResponseState::read(response_state).map_err(error)?)
            .map_err(error)?;
        Ok(v.into())
    }
    #[wasm_bindgen(js_name = actionAt)]
    pub fn action_at(
        &self,
        evaluation: &EvaluationHandle,
        // The guard copies the caller's array and passes it as JSON text, so
        // Rust converts no JavaScript values while it holds this session.
        #[wasm_bindgen(unchecked_param_type = "string[]")] path: String,
    ) -> Result<ActionHandle, JsValue> {
        let path: Vec<String> =
            serde_json::from_str(&path).map_err(|_| error(core::OpenUiError::InvalidHandle))?;
        Ok(ActionHandle {
            inner: self
                .inner
                .action_at(evaluation.inner.clone(), path)
                .map_err(error)?,
        })
    }
    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, name: String) -> Result<Option<String>, JsValue> {
        self.inner.get(name).map_err(error)
    }
    #[wasm_bindgen(js_name = set)]
    pub fn set(&self, name: String, json: Option<String>) -> Result<bool, JsValue> {
        self.inner.set(name, json).map_err(error)
    }
    #[wasm_bindgen(js_name = snapshot)]
    pub fn snapshot(&self) -> Result<String, JsValue> {
        self.inner.snapshot().map_err(error)
    }
    #[wasm_bindgen(js_name = pendingContinuations)]
    pub fn pending_continuations(&self) -> Result<u64, JsValue> {
        self.inner.pending_continuations().map_err(error)
    }
    #[wasm_bindgen(js_name = version)]
    pub fn version(&self) -> Result<u64, JsValue> {
        self.inner.version().map_err(error)
    }
    #[wasm_bindgen(js_name = isPristine)]
    pub fn is_pristine(&self, name: String) -> Result<bool, JsValue> {
        self.inner.is_pristine(name).map_err(error)
    }
    #[wasm_bindgen(js_name = writeBinding)]
    pub fn write_binding(&self, marker: String, json: String) -> Result<bool, JsValue> {
        self.inner.write_binding(marker, json).map_err(error)
    }
    #[wasm_bindgen(js_name = getField)]
    pub fn get_field(&self, form: Option<String>, name: String) -> Result<Option<String>, JsValue> {
        self.inner.get_field(form, name).map_err(error)
    }
    #[wasm_bindgen(js_name = setField, unchecked_return_type = "any")]
    pub fn set_field(
        &self,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        save: bool,
    ) -> Result<String, JsValue> {
        let v = Some(
            self.inner
                .set_field(form, component_type, name, json, save)
                .map_err(error)?,
        );
        Ok(record(
            v.map(|v| serde_json::json!({"changed":v.changed,"saveRequested":v.save_requested}))
                .unwrap_or(serde_json::Value::Null),
        ))
    }
    #[wasm_bindgen(js_name = initializeField, unchecked_return_type = "any")]
    pub fn initialize_field(
        &self,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        streaming: bool,
    ) -> Result<String, JsValue> {
        let v = self
            .inner
            .initialize_field(form, component_type, name, json, streaming)
            .map_err(error)?;
        Ok(record(
            v.map(|v| serde_json::json!({"changed":v.changed,"saveRequested":v.save_requested}))
                .unwrap_or(serde_json::Value::Null),
        ))
    }
    #[wasm_bindgen(js_name = run)]
    pub fn run(
        &self,
        action: Option<ActionHandle>,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, JsValue> {
        Ok(ActionOutput::owned(
            self.inner
                .run(action.map(|v| v.inner.clone()), message, form)
                .map_err(error)?,
            self.inner.release_guard(),
        ))
    }
    #[wasm_bindgen(js_name = runLegacy)]
    pub fn run_legacy(
        &self,
        json: String,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, JsValue> {
        Ok(ActionOutput::owned(
            self.inner.run_legacy(json, message, form).map_err(error)?,
            self.inner.release_guard(),
        ))
    }
    #[wasm_bindgen(js_name = resume)]
    pub fn resume(
        &self,
        continuation: &ContinuationHandle,
        succeeded: bool,
    ) -> Result<ActionOutput, JsValue> {
        Ok(ActionOutput::owned(
            self.inner
                .resume(continuation.inner.clone(), succeeded)
                .map_err(error)?,
            self.inner.release_guard(),
        ))
    }
    #[wasm_bindgen(js_name = dispose)]
    pub fn dispose(&self) -> Result<(), JsValue> {
        self.inner.dispose().map_err(error)
    }
}
#[wasm_bindgen]
pub struct RuntimeLimits {
    inner: core::RuntimeLimits,
}
#[wasm_bindgen]
impl RuntimeLimits {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RuntimeLimits {
        Self {
            inner: core::RuntimeLimits::default(),
        }
    }
    #[wasm_bindgen(getter)]
    pub fn evaluation(&self) -> ParseLimits {
        ParseLimits {
            inner: self.inner.evaluation.clone(),
        }
    }
    #[wasm_bindgen(setter)]
    pub fn set_evaluation(&mut self, value: &ParseLimits) {
        self.inner.evaluation = value.inner.clone();
    }
    #[wasm_bindgen(getter, js_name = referencePreparation)]
    pub fn reference_preparation(&self) -> ParseLimits {
        ParseLimits {
            inner: self.inner.reference_preparation.clone(),
        }
    }
    #[wasm_bindgen(setter, js_name = referencePreparation)]
    pub fn set_reference_preparation(&mut self, value: &ParseLimits) {
        self.inner.reference_preparation = value.inner.clone();
    }
    #[wasm_bindgen(getter, js_name = maxJsonBytes)]
    pub fn max_json_bytes(&self) -> u64 {
        self.inner.max_json_bytes
    }
    #[wasm_bindgen(setter, js_name = maxJsonBytes)]
    pub fn set_max_json_bytes(&mut self, value: u64) {
        self.inner.max_json_bytes = value;
    }
    #[wasm_bindgen(getter, js_name = maxActionPathBytes)]
    pub fn max_action_path_bytes(&self) -> Option<u64> {
        self.inner.max_action_path_bytes
    }
    #[wasm_bindgen(setter, js_name = maxActionPathBytes)]
    pub fn set_max_action_path_bytes(&mut self, value: Option<u64>) {
        self.inner.max_action_path_bytes = value;
    }
    #[wasm_bindgen(getter, js_name = maxContinuations)]
    pub fn max_continuations(&self) -> u64 {
        self.inner.max_continuations
    }
    #[wasm_bindgen(setter, js_name = maxContinuations)]
    pub fn set_max_continuations(&mut self, value: u64) {
        self.inner.max_continuations = value;
    }
    #[wasm_bindgen(getter, js_name = maxCacheBytes)]
    pub fn max_cache_bytes(&self) -> Option<u64> {
        self.inner.max_cache_bytes
    }
    #[wasm_bindgen(setter, js_name = maxCacheBytes)]
    pub fn set_max_cache_bytes(&mut self, value: Option<u64>) {
        self.inner.max_cache_bytes = value;
    }
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::new()
    }
}
#[wasm_bindgen]
pub struct ParseLimits {
    inner: core::ParseLimits,
}

/// Extract an MCP tool call outcome as JSON containing `value` (a result) or
/// `error` (the text of an error the tool reported). Throws InvalidJson,
/// Evaluation or LimitExceeded on refusal. A free function, like the native
/// binding's and like the other stateless helpers: it needs no session.
#[wasm_bindgen(js_name = extractMcpToolResult)]
pub fn extract_mcp_tool_result(
    response_json: &str,
    limits: &ParseLimits,
) -> Result<String, JsValue> {
    core::extract_mcp_tool_result(response_json, limits.inner.to_core()).map_err(error)
}
#[wasm_bindgen]
impl ParseLimits {
    #[wasm_bindgen(constructor)]
    pub fn new() -> ParseLimits {
        Self {
            inner: core::ParseLimits::default(),
        }
    }
    #[wasm_bindgen(getter, js_name = programSize)]
    pub fn program_size(&self) -> Option<u64> {
        self.inner.program_size
    }
    #[wasm_bindgen(setter, js_name = programSize)]
    pub fn set_program_size(&mut self, value: Option<u64>) {
        self.inner.program_size = value;
    }
    #[wasm_bindgen(getter, js_name = resultSize)]
    pub fn result_size(&self) -> Option<u64> {
        self.inner.result_size
    }
    #[wasm_bindgen(setter, js_name = resultSize)]
    pub fn set_result_size(&mut self, value: Option<u64>) {
        self.inner.result_size = value;
    }
    #[wasm_bindgen(getter, js_name = resultFloor)]
    pub fn result_floor(&self) -> Option<u64> {
        self.inner.result_floor
    }
    #[wasm_bindgen(setter, js_name = resultFloor)]
    pub fn set_result_floor(&mut self, value: Option<u64>) {
        self.inner.result_floor = value;
    }
    #[wasm_bindgen(getter, js_name = amplification)]
    pub fn amplification(&self) -> Option<u64> {
        self.inner.amplification
    }
    #[wasm_bindgen(setter, js_name = amplification)]
    pub fn set_amplification(&mut self, value: Option<u64>) {
        self.inner.amplification = value;
    }
    #[wasm_bindgen(getter, js_name = resultCeiling)]
    pub fn result_ceiling(&self) -> Option<u64> {
        self.inner.result_ceiling
    }
    #[wasm_bindgen(setter, js_name = resultCeiling)]
    pub fn set_result_ceiling(&mut self, value: Option<u64>) {
        self.inner.result_ceiling = value;
    }
    #[wasm_bindgen(getter, js_name = depth)]
    pub fn depth(&self) -> Option<u64> {
        self.inner.depth
    }
    #[wasm_bindgen(setter, js_name = depth)]
    pub fn set_depth(&mut self, value: Option<u64>) {
        self.inner.depth = value;
    }
    #[wasm_bindgen(getter, js_name = work)]
    pub fn work(&self) -> Option<u64> {
        self.inner.work
    }
    #[wasm_bindgen(setter, js_name = work)]
    pub fn set_work(&mut self, value: Option<u64>) {
        self.inner.work = value;
    }
    #[wasm_bindgen(getter, js_name = peakMemory)]
    pub fn peak_memory(&self) -> Option<u64> {
        self.inner.peak_memory
    }
    #[wasm_bindgen(setter, js_name = peakMemory)]
    pub fn set_peak_memory(&mut self, value: Option<u64>) {
        self.inner.peak_memory = value;
    }
}
impl Default for ParseLimits {
    fn default() -> Self {
        Self::new()
    }
}
