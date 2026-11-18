use openui_lang::{JsonValue, Limits, MessageError};
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
    let kind = match error {
        MessageError::ProgramSize | MessageError::ContextDepth => "LimitExceeded",
        MessageError::InvalidAttribute | MessageError::InvalidInput => "InvalidJson",
    };
    crate::runtime::thrown(kind, &error.to_string())
}
