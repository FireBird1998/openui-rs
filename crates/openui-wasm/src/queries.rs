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
    #[wasm_bindgen(getter, js_name = evaluationError, unchecked_return_type = "any")]
    pub fn evaluation_error(&self) -> String {
        self.0
            .evaluation_error
            .as_ref()
            .map(|e| {
                serde_json::json!({
                    "kind": crate::runtime::kind(e), "message": e.to_string()
                })
                .to_string()
            })
            .unwrap_or_else(|| "null".into())
    }
}
#[wasm_bindgen]
pub struct QueryWriteOutput {
    changed: bool,
    save_requested: bool,
    queries: Arc<core::QueryEffects>,
}
impl From<core::QueryWriteOutput> for QueryWriteOutput {
    fn from(v: core::QueryWriteOutput) -> Self {
        Self {
            changed: v.changed,
            save_requested: v.save_requested,
            queries: Arc::new(v.queries),
        }
    }
}
#[wasm_bindgen]
impl QueryWriteOutput {
    #[wasm_bindgen(getter)]
    pub fn changed(&self) -> bool {
        self.changed
    }
    #[wasm_bindgen(getter, js_name = saveRequested)]
    pub fn save_requested(&self) -> bool {
        self.save_requested
    }
    #[wasm_bindgen(getter)]
    pub fn queries(&self) -> QueryEffects {
        QueryEffects(self.queries.clone())
    }
}
