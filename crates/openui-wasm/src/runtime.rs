//! wasm-bindgen surface; no mutable receiver is held across JavaScript conversion.
use crate::session::{ReleaseGuard, Session};
use crate::{ComponentLibrary, ParsedProgram};
use openui_session as core;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

fn kind(e: &core::OpenUiError) -> &'static str {
    match e {
        core::OpenUiError::Disposed => "Disposed",
        core::OpenUiError::ForeignHandle => "ForeignHandle",
        core::OpenUiError::StaleHandle => "StaleHandle",
        core::OpenUiError::ConsumedHandle => "ConsumedHandle",
        core::OpenUiError::EvictedHandle => "EvictedHandle",
        core::OpenUiError::InvalidHandle => "InvalidHandle",
        core::OpenUiError::InvalidMarker => "InvalidMarker",
        core::OpenUiError::InvalidJson { .. } => "InvalidJson",
        core::OpenUiError::RefusedWrite { .. } => "RefusedWrite",
        core::OpenUiError::Evaluation { .. } => "Evaluation",
        core::OpenUiError::LimitExceeded { .. } => "LimitExceeded",
        core::OpenUiError::Worker { .. } => "Worker",
        core::OpenUiError::InvalidSchema { .. } => "InvalidSchema",
        core::OpenUiError::InvalidPromptSpec { .. } => "InvalidPromptSpec",
        core::OpenUiError::Prompt { .. } => "Prompt",
    }
}
/// A thrown value the package guard turns into a typed `Error`. The core
/// calls no JavaScript while it holds an exported object: errors and records
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
    #[wasm_bindgen(getter)]
    pub fn evaluation(&self) -> EvaluationHandle {
        EvaluationHandle {
            inner: self.evaluation.clone(),
        }
    }
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
    fn owned(v: core::ActionOutput, release: ReleaseGuard) -> Self {
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
    #[wasm_bindgen(js_name = setProgram)]
    pub fn set_program(&self, program: &ParsedProgram) -> Result<(), JsValue> {
        self.inner.set_program(program.0.clone()).map_err(error)
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
    pub fn evaluate(&self) -> Result<EvaluationOutput, JsValue> {
        let v = self.inner.evaluate().map_err(error)?;
        Ok(EvaluationOutput {
            json: v.json,
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            evaluation: v.evaluation,
        })
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
    #[wasm_bindgen(getter, js_name = maxContinuations)]
    pub fn max_continuations(&self) -> u64 {
        self.inner.max_continuations
    }
    #[wasm_bindgen(setter, js_name = maxContinuations)]
    pub fn set_max_continuations(&mut self, value: u64) {
        self.inner.max_continuations = value;
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
}
impl Default for ParseLimits {
    fn default() -> Self {
        Self::new()
    }
}
