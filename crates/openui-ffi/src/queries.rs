use crate::OpenUiError;
use std::sync::Arc;

#[derive(Debug, uniffi::Object)]
pub struct WantedToolCall {
    pub(crate) inner: Arc<openui_session::WantedToolCall>,
}
#[uniffi::export]
impl WantedToolCall {
    pub fn tool_name(&self) -> String {
        self.inner.tool_name.clone()
    }
    pub fn arguments_json(&self) -> Option<String> {
        self.inner.arguments_json()
    }
}

#[derive(Debug, uniffi::Record)]
pub struct QueryEffects {
    pub changed: bool,
    pub calls: Vec<Arc<WantedToolCall>>,
    pub evaluation_error: Option<OpenUiError>,
    #[uniffi(default = [])]
    pub actions: Vec<crate::ActionOutput>,
    #[uniffi(default = None)]
    pub next_refresh_due_ms: Option<u64>,
}
impl From<openui_session::QueryEffects> for QueryEffects {
    fn from(v: openui_session::QueryEffects) -> Self {
        Self {
            next_refresh_due_ms: v.next_refresh_due_ms,
            changed: v.changed,
            actions: v.actions.into_iter().map(Into::into).collect(),
            calls: v
                .calls
                .into_iter()
                .map(|inner| Arc::new(WantedToolCall { inner }))
                .collect(),
            evaluation_error: v.evaluation_error.map(Into::into),
        }
    }
}
/// The response gate for a draw. The default is settled.
#[derive(Debug, Default, uniffi::Record)]
pub struct ResponseState {
    #[uniffi(default = false)]
    pub streaming: bool,
}
impl From<ResponseState> for openui_session::ResponseState {
    fn from(value: ResponseState) -> Self {
        Self {
            streaming: value.streaming,
        }
    }
}
