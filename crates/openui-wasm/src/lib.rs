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

use openui_lang::StreamParser;
mod message;
mod queries;
mod response_bundle;
mod runtime;
pub mod session;
pub use runtime::*;
use wasm_bindgen::prelude::*;

/// A component library, from the JSON Schema lang-core's
/// `library.toJSONSchema()` gives, or from a library definition. The second
/// field holds a defined library's JSON Schema and prompt spec text.
#[wasm_bindgen]
pub struct ComponentLibrary(
    Arc<openui_session::ComponentLibrary>,
    Option<Arc<DefinedTexts>>,
);

struct DefinedTexts {
    json_schema: String,
    prompt_spec: String,
}

#[wasm_bindgen]
impl ComponentLibrary {
    /// The library a JSON Schema from `library.toJSONSchema()` makes, within
    /// the default RuntimeLimits; see `fromSchemaWithLimits`.
    #[wasm_bindgen(constructor)]
    pub fn new(schema: &str) -> Result<ComponentLibrary, JsValue> {
        Self::from_schema(schema, &openui_session::RuntimeLimits::default())
    }

    /// `new ComponentLibrary(schema)` within a session's limits. The schema
    /// is host input: one longer than `maxJsonBytes` (8 MiB by default),
    /// counted in UTF-8 bytes, throws LimitExceeded before it is read. Text
    /// that isn't JSON throws InvalidSchema.
    #[wasm_bindgen(js_name = fromSchemaWithLimits)]
    pub fn from_schema_with_limits(
        schema: &str,
        limits: &runtime::RuntimeLimits,
    ) -> Result<ComponentLibrary, JsValue> {
        Self::from_schema(schema, limits.core())
    }

    /// The library a library definition in plain-data form, as JSON text,
    /// makes (`docs/library-definition.md`), within the default
    /// RuntimeLimits; see `fromDefinitionWithLimits`.
    #[wasm_bindgen(js_name = fromDefinition)]
    pub fn from_definition(definition: &str) -> Result<ComponentLibrary, JsValue> {
        Self::defined(definition, &openui_session::RuntimeLimits::default())
    }

    /// `fromDefinition` within a session's limits. The text is host input:
    /// one longer than `maxJsonBytes` (8 MiB by default), counted in UTF-8
    /// bytes, throws LimitExceeded before it is read. Malformed JSON, a
    /// definition not in the plain-data form, an unsupported kind, a root
    /// that is not a component and a colliding function or action name throw
    /// InvalidJson with the core's message.
    #[wasm_bindgen(js_name = fromDefinitionWithLimits)]
    pub fn from_definition_with_limits(
        definition: &str,
        limits: &runtime::RuntimeLimits,
    ) -> Result<ComponentLibrary, JsValue> {
        Self::defined(definition, limits.core())
    }

    /// A defined library's JSON Schema as JSON text, byte for byte as
    /// upstream's `JSON.stringify(library.toJSONSchema())`; undefined for a
    /// library made from a JSON Schema.
    #[wasm_bindgen(js_name = jsonSchema)]
    pub fn json_schema(&self) -> Option<String> {
        self.1.as_ref().map(|d| d.json_schema.clone())
    }

    /// A defined library's prompt spec as JSON text, byte for byte as
    /// upstream's `JSON.stringify(library.toSpec())`; undefined for a
    /// library made from a JSON Schema.
    #[wasm_bindgen(js_name = promptSpec)]
    pub fn prompt_spec(&self) -> Option<String> {
        self.1.as_ref().map(|d| d.prompt_spec.clone())
    }
}

impl ComponentLibrary {
    fn from_schema(
        schema: &str,
        limits: &openui_session::RuntimeLimits,
    ) -> Result<ComponentLibrary, JsValue> {
        let library = openui_session::library_from_schema(schema, limits).map_err(|e| match e {
            // The JSON reader's message alone, as before the limit.
            openui_session::OpenUiError::InvalidSchema { reason } => {
                runtime::thrown("InvalidSchema", &reason)
            }
            other => runtime::error(other),
        })?;
        Ok(ComponentLibrary(Arc::new(library), None))
    }

    fn defined(
        definition: &str,
        limits: &openui_session::RuntimeLimits,
    ) -> Result<ComponentLibrary, JsValue> {
        let defined = openui_session::define_library(definition, limits).map_err(runtime::error)?;
        Ok(ComponentLibrary(
            Arc::new(defined.library),
            Some(Arc::new(DefinedTexts {
                json_schema: defined.json_schema,
                prompt_spec: defined.prompt_spec,
            })),
        ))
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
