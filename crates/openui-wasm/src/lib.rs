//! The core for JavaScript: compiled to WebAssembly, with results handed
//! over as JSON text for the caller to `JSON.parse`. Text is the cheapest
//! way across: one copy, and the engine's own JSON reader builds the objects.
//!
//! The JavaScript wrapper beside the built package (`openui.js`) gives this
//! lang-core's own interface, `createParser` and `createStreamingParser`.
//! Runtime sessions reuse owned reference preparation and accept JSON-only host
//! function callbacks under the package reentry guard. Synchronous JavaScript
//! callbacks must return for progress; they cannot be preempted if they block.

use std::sync::Arc;

use openui_lang::{Library, StreamParser};
mod message;
mod response_bundle;
mod runtime;
pub mod session;
pub use runtime::*;
use wasm_bindgen::prelude::*;

/// A component library, from the JSON Schema lang-core's
/// `library.toJSONSchema()` gives.
#[wasm_bindgen]
pub struct ComponentLibrary(Arc<openui_session::ComponentLibrary>);

#[wasm_bindgen]
impl ComponentLibrary {
    /// Throws InvalidSchema when `schema` isn't JSON.
    #[wasm_bindgen(constructor)]
    pub fn new(schema: &str) -> Result<ComponentLibrary, JsValue> {
        let schema: serde_json::Value = serde_json::from_str(schema)
            .map_err(|e| runtime::thrown("InvalidSchema", &e.to_string()))?;
        Ok(ComponentLibrary(Arc::new(
            openui_session::ComponentLibrary {
                inner: Arc::new(Library::from_json_schema(&schema)),
                schema: Arc::new(schema),
            },
        )))
    }
}

/// The whole of `program` parsed at once: the result as JSON.
#[wasm_bindgen]
pub fn parse(program: &str, library: &ComponentLibrary, root_name: Option<String>) -> String {
    openui_lang::parse(program, &library.0.inner, root_name.as_deref()).to_json()
}

/// A program arriving in pieces. Every call gives the result so far as JSON.
#[wasm_bindgen]
pub struct StreamSession(StreamParser);

#[wasm_bindgen]
impl StreamSession {
    #[wasm_bindgen(constructor)]
    pub fn new(library: &ComponentLibrary, root_name: Option<String>) -> StreamSession {
        StreamSession(StreamParser::new(
            library.0.inner.clone(),
            root_name.as_deref(),
        ))
    }

    /// The next piece of text.
    pub fn push(&mut self, chunk: &str) -> String {
        self.0.push(chunk).to_json()
    }

    /// The next piece of text, parsed but not written out: for a caller
    /// that draws less often than text arrives. [`StreamSession::result`]
    /// gives the result when it's wanted.
    pub fn feed(&mut self, chunk: &str) {
        self.0.push(chunk);
    }

    /// The whole text so far, replacing what was pushed.
    pub fn set(&mut self, text: &str) -> String {
        self.0.set(text).to_json()
    }

    /// The result so far.
    pub fn result(&mut self) -> String {
        self.0.result().to_json()
    }

    /// The stream has ended: the final result.
    pub fn finish(&mut self) -> String {
        self.0.finish().to_json()
    }
}

/// Actual parser output shared with a runtime, without JSON round-tripping.
#[wasm_bindgen]
pub struct ParsedProgram(Arc<openui_session::ParsedProgram>);
fn program(result: openui_lang::ParseResult) -> ParsedProgram {
    let json_bytes = result.to_json().len() as u64;
    ParsedProgram(Arc::new(openui_session::ParsedProgram {
        inner: Arc::new(result),
        json_bytes,
    }))
}
#[wasm_bindgen(js_name = parseProgram)]
pub fn parse_program(
    text: &str,
    library: &ComponentLibrary,
    root_name: Option<String>,
) -> ParsedProgram {
    program(openui_lang::parse(
        text,
        &library.0.inner,
        root_name.as_deref(),
    ))
}
#[wasm_bindgen]
impl StreamSession {
    #[wasm_bindgen(js_name = program)]
    pub fn program(&mut self, complete: bool) -> ParsedProgram {
        program(if complete {
            self.0.finish()
        } else {
            self.0.result()
        })
    }
}
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

impl ParsedProgram {
    /// Rust adapter access to the actual parser output.
    pub fn shared(&self) -> Arc<openui_session::ParsedProgram> {
        self.0.clone()
    }
}
impl ComponentLibrary {
    /// Rust adapter access to the compiled schema.
    pub fn shared(&self) -> Arc<openui_session::ComponentLibrary> {
        self.0.clone()
    }
}

mod callbacks;
