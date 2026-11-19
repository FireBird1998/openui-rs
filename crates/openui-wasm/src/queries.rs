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
pub struct QueryEffects(pub(crate) Arc<core::QueryEffects>);
impl From<core::QueryEffects> for QueryEffects {
    fn from(v: core::QueryEffects) -> Self {
        Self(Arc::new(v))
    }
}
#[wasm_bindgen]
impl QueryEffects {
    #[wasm_bindgen(getter)]
    pub fn changed(&self) -> bool {
        self.0.changed
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
const RESPONSE_STATE: &'static str = "export interface ResponseState { streaming: boolean }";

// The package guard passes a plain copy of the response state. All host
// getters run before borrowing the session.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ResponseState")]
    pub type ResponseState;
    #[wasm_bindgen(method, getter)]
    fn streaming(this: &ResponseState) -> bool;
}

impl ResponseState {
    pub(crate) fn read(state: Option<Self>) -> core::ResponseState {
        core::ResponseState {
            streaming: state.is_some_and(|s| s.streaming()),
        }
    }
}
