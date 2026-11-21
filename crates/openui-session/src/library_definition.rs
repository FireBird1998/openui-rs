//! A component library from a host's library definition (ADR 0017): the
//! plain-data form read from JSON text, checked as upstream's
//! `createLibrary` checks it, and compiled through the parser's one way in,
//! the JSON Schema. Both bindings call this, so they refuse and accept the
//! same definitions with the same errors (ADR 0007).

use std::sync::Arc;

use openui_lang::{LibraryDefinition, create_library, free_json, stringify_json};

use crate::{ComponentLibrary, OpenUiError, RuntimeLimits, admit_text};

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
/// What a definition within the byte limit costs past that is the core's:
/// compiling library actions that refer to each other copies each action's
/// shape into every param that refers to it, as upstream does, so the cost
/// is not bounded by the definition's size (see
/// `docs/library-definition.md`).
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
