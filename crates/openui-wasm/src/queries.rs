//! Opaque results returned by a session. Lists lend capabilities by index.
use openui_session as core;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WantedToolCall(pub(crate) Arc<core::WantedToolCall>);
#[wasm_bindgen]
impl WantedToolCall {
    #[wasm_bindgen(getter, js_name = toolName)]
    pub fn tool_name(&self) -> String {
        self.0.tool_name.clone()
    }
    #[wasm_bindgen(getter, js_name = argumentsJson)]
    pub fn arguments_json(&self) -> Option<String> {
        self.0.arguments_json()
    }
}
#[wasm_bindgen]
pub struct QueryEffects(
    pub(crate) core::QueryEffects,
    pub(crate) crate::session::ReleaseGuard,
);
#[wasm_bindgen]
impl QueryEffects {
    #[wasm_bindgen(getter, js_name = nextRefreshDueMs)]
    pub fn next_refresh_due_ms(&self) -> Option<u64> {
        self.0.next_refresh_due_ms
    }
    #[wasm_bindgen(getter)]
    pub fn changed(&self) -> bool {
        self.0.changed
    }
    #[wasm_bindgen(getter, js_name = actionCount)]
    pub fn action_count(&self) -> u64 {
        self.0.actions.len() as u64
    }
    #[wasm_bindgen(js_name = actionAt)]
    pub fn action_at(&self, index: u64) -> Option<crate::runtime::ActionOutput> {
        self.0
            .actions
            .get(usize::try_from(index).ok()?)
            .cloned()
            .map(|v| crate::runtime::ActionOutput::owned(v, self.1.clone()))
    }
    #[wasm_bindgen(getter, js_name = callCount)]
    pub fn call_count(&self) -> u64 {
        self.0.calls.len() as u64
    }
    #[wasm_bindgen(js_name = callAt)]
    pub fn call_at(&self, index: u64) -> Option<WantedToolCall> {
        self.0
            .calls
            .get(usize::try_from(index).ok()?)
            .cloned()
            .map(WantedToolCall)
    }
    #[wasm_bindgen(getter, js_name = evaluationError, unchecked_return_type = "{ kind: string; message: string } | null")]
    pub fn evaluation_error(&self) -> String {
        error_json(self.0.evaluation_error.as_ref())
    }
}

pub(crate) fn error_json(error: Option<&core::OpenUiError>) -> String {
    error
        .map(|e| {
            serde_json::json!({ "kind": crate::runtime::kind(e), "message": e.to_string() })
                .to_string()
        })
        .unwrap_or_else(|| "null".into())
}

#[wasm_bindgen(typescript_custom_section)]
const RESPONSE_STATE: &'static str = "export interface ResponseState { streaming: boolean; isBundle?: boolean; complete?: boolean; error?: string | null; scripts?: string[]; scriptRevision?: string | null }";

// The package guard passes a plain copy of the response state. All host
// getters run before borrowing the session.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ResponseState")]
    pub type ResponseState;
    #[wasm_bindgen(method, getter)]
    fn streaming(this: &ResponseState) -> bool;
    #[wasm_bindgen(method, getter, js_name = isBundle)]
    fn is_bundle(this: &ResponseState) -> bool;
    #[wasm_bindgen(method, getter)]
    fn complete(this: &ResponseState) -> bool;
    #[wasm_bindgen(method, getter)]
    fn error(this: &ResponseState) -> Option<String>;
    #[wasm_bindgen(method, getter, js_name = scriptsJson)]
    fn scripts_json(this: &ResponseState) -> String;
    #[wasm_bindgen(method, getter, js_name = scriptRevision)]
    fn script_revision(this: &ResponseState) -> Option<String>;
}

impl ResponseState {
    pub(crate) fn read(state: Option<Self>) -> Result<core::ResponseState, core::OpenUiError> {
        let Some(state) = state else {
            return Ok(core::ResponseState::default());
        };
        Ok(core::ResponseState {
            streaming: state.streaming(),
            is_bundle: state.is_bundle(),
            complete: state.complete(),
            error: state.error(),
            scripts: serde_json::from_str(&state.scripts_json()).map_err(|e| {
                core::OpenUiError::InvalidJson {
                    reason: e.to_string(),
                }
            })?,
            script_revision: state.script_revision(),
        })
    }
}
