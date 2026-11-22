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
    state.reader.read(text, streaming).map_err(reader_error)?;
    drawn(state, streaming, started)
}

/// [`read`] of the accepted text, which the reader still holds.
pub(crate) fn reread(
    state: &mut StreamState,
    streaming: bool,
    started: Instant,
) -> Result<ResponseRead, OpenUiError> {
    state
        .reader
        .read_append("", streaming)
        .map_err(reader_error)?;
    drawn(state, streaming, started)
}

/// What [`read`] of the accepted text and `appended` keeps, without its
/// outputs: the read and its refusals, but no parse result and no bundle,
/// message or parse JSON, each of which costs the whole response.
pub(crate) fn append(
    state: &mut StreamState,
    appended: &str,
    streaming: bool,
) -> Result<(), OpenUiError> {
    state
        .reader
        .read_append(appended, streaming)
        .map_err(reader_error)?;
    // `read` refuses what the message reader refuses, so this does too. With
    // the core's readers no text reaches it today: the message reader's only
    // refusal is a context nested past the depth limit, and the bundle reader
    // reads the same context, or a longer one, and refuses it first.
    message(state)?;
    // The parser takes the program at the next draw, or when `push`, `set`,
    // `result` or `finish` asks for it.
    state.reader.keep();
    // The next result or finish comes from the parser, not the last output.
    state.output = None;
    state.know_program();
    Ok(())
}

/// The staged read's message, refused as `parse_message` refuses it.
fn message(state: &mut StreamState) -> Result<Option<String>, OpenUiError> {
    let reader = &mut state.reader;
    reader.message_json().map_err(|error| {
        // Refused after the read was staged: leave nothing staged.
        reader.discard();
        reader_error(error)
    })
}

/// Parse a staged read and write what the driver draws from.
fn drawn(
    state: &mut StreamState,
    streaming: bool,
    started: Instant,
) -> Result<ResponseRead, OpenUiError> {
    let message = message(state)?;
    let reader = &mut state.reader;
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
    state.know_program();
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
