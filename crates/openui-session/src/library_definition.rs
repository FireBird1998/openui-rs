//! A component library from a host's text: the JSON Schema upstream's
//! `library.toJSONSchema()` writes, or a library definition (ADR 0017), the
//! plain-data form read from JSON text, checked as upstream's
//! `createLibrary` checks it, and compiled through the parser's one way in,
//! the JSON Schema. Both bindings call these, so they refuse and accept the
//! same text with the same errors (ADR 0007).

use std::sync::Arc;

use openui_lang::{
    DefinedLibrary, Library, LibraryDefinition, create_library, free_json, stringify_json,
};

use crate::{ComponentLibrary, OpenUiError, RuntimeLimits, admit_text};

/// Reads a component library's JSON Schema text and compiles it.
///
/// The text is host input, so it is held to `limits.max_json_bytes`,
/// counted in bytes, before anything reads it: a longer one is refused with
/// `LimitExceeded` ("host input byte limit"), as other host text is (#140).
/// Text that is not JSON is refused with `InvalidSchema`, with the JSON
/// reader's message. Of the limits, only `max_json_bytes` applies. The text
/// is read with serde_json on a stack segment of its own
/// ([`openui_lang::read_host_json`]), so no schema it accepts or refuses
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
    let json = openui_lang::read_host_json(schema).map_err(|e| OpenUiError::InvalidSchema {
        reason: e.to_string(),
    })?;
    Ok(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&json)),
        schema: Arc::new(json),
        defined: None,
    })
}

/// What a library made from a definition keeps beside its compiled form:
/// the definition itself, its JSON Schema and prompt spec as JSON text, byte
/// for byte as upstream's `toJSONSchema()` and `toSpec()` give them when
/// `JSON.stringify` writes them, and the limits it was made under, which its
/// prompts are generated under too.
pub struct Defined {
    /// The library as the core made it, for prompts, binding props and root.
    pub library: DefinedLibrary,
    /// The library's JSON Schema.
    pub json_schema: String,
    /// The library's prompt spec: the library object a prompt spec's
    /// `library` field holds.
    pub prompt_spec: String,
    limits: RuntimeLimits,
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
/// the schema the library holds keeps the core's own key order. The library
/// keeps the definition ([`ComponentLibrary::defined`]): a session over it
/// reads its binding props and root from it, and its prompts are generated
/// from it under `limits`.
///
/// Past the byte limit, a definition costs time and memory in step with its
/// size: the compiled library shares each library action's shape between
/// the params that refer to it, where upstream copies it into each (#140).
pub fn define_library(
    definition: &str,
    limits: &RuntimeLimits,
) -> Result<ComponentLibrary, OpenUiError> {
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
    Ok(ComponentLibrary {
        inner: Arc::new(library),
        schema: Arc::new(schema),
        defined: Some(Arc::new(Defined {
            library: defined,
            json_schema,
            prompt_spec,
            limits: limits.clone(),
        })),
    })
}

impl ComponentLibrary {
    /// A library made from a definition: its system prompt, upstream's
    /// `library.prompt(options)`, byte for byte. `options` is upstream's
    /// `PromptOptions` as JSON text (`{"bindings": true}`, `{"tools": [...]}`,
    /// `{"preamble": ...}` and the rest); none, or text that is not an
    /// object, adds nothing. None for a library made from a JSON Schema.
    ///
    /// Generated under the limits the library was made with: `options` is
    /// host input, held to `max_json_bytes` (`LimitExceeded`) and read under
    /// `evaluation` (`InvalidPromptSpec` when it is not JSON or nests too
    /// deep), and the prompt is refused as `Prompt`, with the core's message,
    /// where `generate_system_prompt_with_limits` refuses a spec.
    pub fn prompt(&self, options: Option<&str>) -> Result<Option<String>, OpenUiError> {
        let Some(defined) = &self.defined else {
            return Ok(None);
        };
        let limits = defined.limits.evaluation.to_core();
        let options = prompt_options(options, &defined.limits)?;
        let prompt = defined.library.prompt_with_limits(options, limits);
        prompt.map(Some).map_err(prompt_error)
    }

    /// A library made from a definition: its OpenUI Cloud config block, as
    /// upstream's `generateSystemPrompt({cloud: true, library:
    /// library.toSpec(), promptOptions, instructions})` writes it, byte for
    /// byte. `options` is upstream's `CloudPromptOptions` as JSON text (only
    /// `examples`, `preamble` and `additionalRules` are sent); non-empty
    /// `instructions` follow the block. None for a library made from a JSON
    /// Schema.
    ///
    /// Generated under the limits the library was made with, as
    /// [`ComponentLibrary::prompt`]; `instructions` is host input too, held to
    /// `max_json_bytes`. A library Cloud does not accept (no component, or a
    /// root its JSON Schema has no entry for) is refused as `Prompt`, with
    /// upstream's message.
    pub fn cloud_config(
        &self,
        options: Option<&str>,
        instructions: Option<&str>,
    ) -> Result<Option<String>, OpenUiError> {
        let Some(defined) = &self.defined else {
            return Ok(None);
        };
        if let Some(instructions) = instructions {
            admit_text(instructions, defined.limits.max_json_bytes)?;
        }
        let limits = defined.limits.evaluation.to_core();
        let options = prompt_options(options, &defined.limits)?;
        let config = defined
            .library
            .cloud_config_with_limits(options, instructions, limits);
        config.map(Some).map_err(prompt_error)
    }
}

/// A prompt's options from the host: held to `max_json_bytes`, then read
/// under the evaluation limits with the core's reader, which grows its
/// stack and refuses nesting past the depth limit.
fn prompt_options(
    options: Option<&str>,
    limits: &RuntimeLimits,
) -> Result<Option<serde_json::Value>, OpenUiError> {
    let Some(options) = options else {
        return Ok(None);
    };
    admit_text(options, limits.max_json_bytes)?;
    openui_lang::parse_json_with_limits(options, limits.evaluation.to_core())
        .map(Some)
        .map_err(|e| OpenUiError::InvalidPromptSpec {
            reason: e.to_string(),
        })
}

fn prompt_error(e: openui_lang::PromptError) -> OpenUiError {
    OpenUiError::Prompt {
        reason: e.to_string(),
    }
}
