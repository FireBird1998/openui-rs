use openui_lang::Limits;
use wasm_bindgen::prelude::*;

/// Read a response bundle and return its JSON. The package guard carries the
/// input as UTF-16 to preserve lone units even on the last streaming cut.
#[wasm_bindgen(js_name = parseResponseBundle)]
pub fn parse_response_bundle(
    #[wasm_bindgen(unchecked_param_type = "string | null")] text: &[u16],
    streaming: bool,
) -> Result<String, JsValue> {
    openui_lang::parse_response_bundle_utf16(text, streaming, Limits::DEFAULT)
        .map(|bundle| bundle.to_json())
        .map_err(|error| crate::runtime::thrown("LimitExceeded", &error.to_string()))
}
