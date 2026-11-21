use std::time::Instant;

use crate::{OpenUiError, ParseOutput, StreamState};
use openui_lang::{Limits, ResponseReaderError};

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

/// One read of a raw response ([`StreamSession::read_response`](crate::StreamSession::read_response)).
#[derive(uniffi::Record)]
pub struct ResponseRead {
    /// [`parse_response_bundle`]'s JSON for the text.
    pub bundle: String,
    /// `parse_message`'s `context` and `attributes` for the text, as a JSON
    /// object without its content, or `None` when the text has no `]]>`.
    pub message: Option<String>,
    /// The program's parse: the stream's latest result while `streaming`,
    /// its finished result otherwise.
    pub parse: ParseOutput,
}

pub(crate) fn read(
    state: &mut StreamState,
    text: &str,
    streaming: bool,
    started: Instant,
) -> Result<ResponseRead, OpenUiError> {
    let reader = &mut state.reader;
    reader.read(text, streaming).map_err(reader_error)?;
    let message = match reader.message_json() {
        Ok(message) => message,
        Err(error) => {
            // Refused after the read was staged: leave nothing staged.
            reader.discard();
            return Err(reader_error(error));
        }
    };
    let partial = reader
        .try_parse(true, false, Ok::<_, std::convert::Infallible>)
        .unwrap_or_else(|never| match never {});
    let result = if streaming {
        partial.unwrap_or_else(|| reader.result())
    } else {
        reader.finish()
    };
    let bundle = reader.bundle_json();
    let parse = state.output(result, started, !streaming);
    // What `set` would remember of the program, for its own shortcut.
    let program = state.reader.program();
    state.known_text = (program.len() <= state.program_limit).then(|| program.to_owned());
    Ok(ResponseRead {
        bundle,
        message,
        parse,
    })
}

fn reader_error(error: ResponseReaderError) -> OpenUiError {
    match error {
        ResponseReaderError::Message(error) => crate::message::message_error(error),
        _ => OpenUiError::LimitExceeded {
            reason: error.to_string(),
        },
    }
}
