use openui_lang::{JsonValue, Limits, MessageError};
use openui_session::OpenUiError;
use wasm_bindgen::prelude::*;

/// Read message text with the core's default limits and return its parts as
/// JSON. Set `streaming` to hold back a potential last marker line.
#[wasm_bindgen(js_name = parseMessage)]
pub fn parse_message(text: &str, streaming: bool) -> Result<String, JsValue> {
    openui_lang::parse_message(text, streaming, Limits::DEFAULT)
        .map(|message| message.to_json())
        .map_err(message_error)
}

/// Write a message from a JSON object with `content`, optional `context` and
/// string `attributes`, and an optional boolean `end`. Uses the core's
/// default limits. The package guard restores the message's UTF-16 units.
#[wasm_bindgen(js_name = buildMessage)]
pub fn build_message(parts: &str) -> Result<String, JsValue> {
    openui_lang::build_message_json(parts, Limits::DEFAULT)
        .map(|text| openui_lang::stringify_json(&JsonValue::String(text)))
        .map_err(message_error)
}

fn message_error(error: MessageError) -> JsValue {
    let reason = error.to_string();
    let error = match error {
        MessageError::ProgramSize | MessageError::ContextDepth => {
            OpenUiError::LimitExceeded { reason }
        }
        MessageError::InvalidAttribute => OpenUiError::InvalidAttribute { reason },
        MessageError::InvalidInput => OpenUiError::InvalidJson { reason },
    };
    crate::runtime::thrown(crate::runtime::kind(&error), &error.to_string())
}
