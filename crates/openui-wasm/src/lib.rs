//! The core for JavaScript: compiled to WebAssembly, with results handed
//! over as JSON text for the caller to `JSON.parse`. Text is the cheapest
//! way across: one copy, and the engine's own JSON reader builds the objects.
//!
//! The JavaScript wrapper beside the built package (`openui.js`) gives this
//! lang-core's own interface, `createParser` and `createStreamingParser`.

use std::sync::Arc;

use openui_lang::{Library, StreamParser};
use wasm_bindgen::prelude::*;

/// A component library, from the JSON Schema lang-core's
/// `library.toJSONSchema()` gives.
#[wasm_bindgen]
pub struct ComponentLibrary(Arc<Library>);

#[wasm_bindgen]
impl ComponentLibrary {
    /// Throws when `schema` isn't JSON.
    #[wasm_bindgen(constructor)]
    pub fn new(schema: &str) -> Result<ComponentLibrary, JsError> {
        let schema: serde_json::Value = serde_json::from_str(schema)?;
        Ok(ComponentLibrary(Arc::new(Library::from_json_schema(
            &schema,
        ))))
    }
}

/// The whole of `program` parsed at once: the result as JSON.
#[wasm_bindgen]
pub fn parse(program: &str, library: &ComponentLibrary, root_name: Option<String>) -> String {
    openui_lang::parse(program, &library.0, root_name.as_deref()).to_json()
}

/// A program arriving in pieces. Every call gives the result so far as JSON.
#[wasm_bindgen]
pub struct StreamSession(StreamParser);

#[wasm_bindgen]
impl StreamSession {
    #[wasm_bindgen(constructor)]
    pub fn new(library: &ComponentLibrary, root_name: Option<String>) -> StreamSession {
        StreamSession(StreamParser::new(library.0.clone(), root_name.as_deref()))
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
