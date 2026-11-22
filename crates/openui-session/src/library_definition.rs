//! A component library from a host's text: the JSON Schema upstream's
//! `library.toJSONSchema()` writes, or a library definition (ADR 0017), the
//! plain-data form read from JSON text, checked as upstream's
//! `createLibrary` checks it, and compiled through the parser's one way in,
//! the JSON Schema. Both bindings call these, so they refuse and accept the
//! same text with the same errors (ADR 0007).

use std::sync::Arc;

use openui_lang::{Library, LibraryDefinition, create_library, free_json, stringify_json};

use crate::{ComponentLibrary, OpenUiError, RuntimeLimits, admit_text};

/// Reads a component library's JSON Schema text and compiles it.
///
/// The text is host input, so it is held to `limits.max_json_bytes`,
/// counted in bytes, before anything reads it: a longer one is refused with
/// `LimitExceeded` ("host input byte limit"), as other host text is (#140).
/// Text that is not JSON is refused with `InvalidSchema`, with the JSON
/// reader's message. Of the limits, only `max_json_bytes` applies. The text
/// is read with serde_json on a stack segment of its own
/// ([`openui_lang::read_schema_json`]), so no schema it accepts or refuses
/// can overflow the caller's thread.
///
/// Compiling costs time and memory in step with the schema's size: the
/// compiled library shares each action shape between the params that refer
/// to it. The schema is held as read, for a session's action definitions.
pub fn library_from_schema(
    schema: &str,
    limits: &RuntimeLimits,
) -> Result<ComponentLibrary, OpenUiError> {
    admit_text(schema, limits.max_json_bytes)?;
    // serde_json's reader, on a segment sized for its level bound: a schema
    // nested to the limit reads on any thread.
    let json = openui_lang::read_schema_json(schema).map_err(|e| OpenUiError::InvalidSchema {
        reason: e.to_string(),
    })?;
    Ok(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&json)),
        schema: Arc::new(json),
    })
}

/// A library made from a definition, with its JSON Schema and prompt spec as
/// JSON text, byte for byte as upstream's `toJSONSchema()` and `toSpec()`
/// give them when `JSON.stringify` writes them.
pub struct DefinedLibraryOutput {
    /// The compiled library and the schema it was compiled from, as a library
    /// supplied as JSON Schema has them.
    pub library: ComponentLibrary,
    /// The library's JSON Schema.
    pub json_schema: String,
    /// The library's prompt spec: the library object a prompt spec's
    /// `library` field holds.
    pub prompt_spec: String,
}

/// Reads a library definition's JSON text and makes its library.
///
/// The text is host input, so it is held to `limits.max_json_bytes`,
/// counted in bytes, before anything reads it: a longer one is refused with
/// `LimitExceeded` ("host input byte limit"), as other host text is. Then it
/// is read as JSON under `limits.evaluation`, whose depth limit also bounds
/// how deep types nest. Malformed JSON, a definition not in the plain-data
/// form (an unsupported kind, types nested too deep, a missing or unknown
/// field), a root that is not a component and a function or action name
/// that collides are refused with `InvalidJson`, with the core's message
/// (upstream's own for the root and the collisions).
///
/// The JSON Schema and prompt spec are written with the core's
/// `stringify_json`, which puts integer-like keys first as JavaScript does;
/// the schema the library holds keeps the core's own key order.
///
/// Past the byte limit, a definition costs time and memory in step with its
/// size: the compiled library shares each library action's shape between
/// the params that refer to it, where upstream copies it into each (#140).
pub fn define_library(
    definition: &str,
    limits: &RuntimeLimits,
) -> Result<DefinedLibraryOutput, OpenUiError> {
    admit_text(definition, limits.max_json_bytes)?;
    let core_limits = limits.evaluation.to_core();
    let invalid = |reason: String| OpenUiError::InvalidJson { reason };
    let json = openui_lang::parse_json_with_limits(definition, core_limits)
        .map_err(|e| invalid(e.to_string()))?;
    let read = LibraryDefinition::from_json_with_limits(&json, core_limits);
    // The JSON may nest to the reader's limit: free it without recursing.
    free_json(json);
    let defined = create_library(read.map_err(|e| invalid(e.to_string()))?)
        .map_err(|e| invalid(e.to_string()))?;
    let schema = defined.to_json_schema();
    let spec = defined.to_spec();
    let json_schema = stringify_json(&schema);
    let prompt_spec = stringify_json(&spec);
    free_json(spec);
    let library = defined.library();
    Ok(DefinedLibraryOutput {
        library: ComponentLibrary {
            inner: Arc::new(library),
            schema: Arc::new(schema),
        },
        json_schema,
        prompt_spec,
    })
}
