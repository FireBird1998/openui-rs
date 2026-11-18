use crate::OpenUiError;
use openui_lang::Limits;

/// Read a response bundle with the core's default limits and return its JSON.
/// Text follows the core's [UTF-16 string convention](openui_lang::utf16_units).
#[uniffi::export]
pub fn parse_response_bundle(text: Option<String>, streaming: bool) -> Result<String, OpenUiError> {
    openui_lang::parse_response_bundle(text.as_deref(), streaming, Limits::DEFAULT)
        .map(|bundle| bundle.to_json())
        .map_err(|error| OpenUiError::LimitExceeded {
            reason: error.to_string(),
        })
}
