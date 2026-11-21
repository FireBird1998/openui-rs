//! JSON transport for the native and WebAssembly MCP extraction exports.

use crate::OpenUiError;
use openui_runtime::{Limits, McpResponseError, McpToolOutcome, Value};

/// Extract a tool call outcome as JSON for either binding.
///
/// A successful extraction contains either a `value` or an `error` field.
/// Invalid response JSON, JavaScript response errors and configured limit
/// refusals use the binding's existing typed error kinds.
pub fn extract_mcp_tool_result(response_json: &str, limits: Limits) -> Result<String, OpenUiError> {
    let outcome = openui_runtime::extract_mcp_tool_result(response_json, limits).map_err(error)?;
    let (prefix, value) = match outcome {
        McpToolOutcome::Result(value) => (r#"{"value":"#, value),
        McpToolOutcome::ToolReported { message } => (r#"{"error":"#, Value::Str(message)),
    };
    let mut json = prefix.to_owned();
    openui_lang::write_host_json(&value, limits.result_size(), Some(&mut json)).ok_or_else(
        || OpenUiError::LimitExceeded {
            reason: "tool outcome JSON size limit exceeded".into(),
        },
    )?;
    json.push('}');
    Ok(json)
}

pub(super) fn error(error: McpResponseError) -> OpenUiError {
    let reason = error.to_string();
    match error {
        McpResponseError::InvalidJson(_) => OpenUiError::InvalidJson { reason },
        McpResponseError::InvalidResponse { .. } => OpenUiError::Evaluation { reason },
        McpResponseError::Limit(_) => OpenUiError::LimitExceeded { reason },
    }
}
