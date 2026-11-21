use crate::OpenUiError;
use openui_lang::{Limits, MessageError};

/// Read message text using the core's default limits and return its parts as
/// JSON. Set `streaming` to hold back a potential last marker line. Message
/// text uses the core's [UTF-16 representation](openui_lang::utf16_units).
#[uniffi::export]
pub fn parse_message(text: String, streaming: bool) -> Result<String, OpenUiError> {
    openui_lang::parse_message(&text, streaming, Limits::DEFAULT)
        .map(|message| message.to_json())
        .map_err(message_error)
}

/// Write a message from a JSON object with `content`, optional `context` and
/// string `attributes`, and an optional boolean `end`. Uses the core's default
/// limits and [UTF-16 representation](openui_lang::utf16_units) for message text.
#[uniffi::export]
pub fn build_message(parts: String) -> Result<String, OpenUiError> {
    openui_lang::build_message_json(&parts, Limits::DEFAULT).map_err(message_error)
}

pub(crate) fn message_error(error: MessageError) -> OpenUiError {
    let reason = error.to_string();
    match error {
        MessageError::ProgramSize | MessageError::ContextDepth => {
            OpenUiError::LimitExceeded { reason }
        }
        MessageError::InvalidAttribute => OpenUiError::InvalidAttribute { reason },
        MessageError::InvalidInput => OpenUiError::InvalidJson { reason },
    }
}
