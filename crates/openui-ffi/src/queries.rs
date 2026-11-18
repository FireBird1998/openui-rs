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
}
impl From<openui_session::QueryEffects> for QueryEffects {
    fn from(v: openui_session::QueryEffects) -> Self {
        Self {
            changed: v.changed,
            calls: v
                .calls
                .into_iter()
                .map(|inner| Arc::new(WantedToolCall { inner }))
                .collect(),
            evaluation_error: v.evaluation_error.map(Into::into),
        }
    }
}
#[derive(Debug, uniffi::Record)]
pub struct QueryWriteOutput {
    pub changed: bool,
    pub save_requested: bool,
    pub queries: QueryEffects,
}
impl From<openui_session::QueryWriteOutput> for QueryWriteOutput {
    fn from(v: openui_session::QueryWriteOutput) -> Self {
        Self {
            changed: v.changed,
            save_requested: v.save_requested,
            queries: v.queries.into(),
        }
    }
}
