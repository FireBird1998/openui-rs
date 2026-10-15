//! The core behind a UniFFI interface, for the viewer's Swift today and
//! Dynamic News's Kotlin later.
//!
//! Parse results cross as the JSON text [`ParseResult::to_json`] produces,
//! byte-identical with the reference, plus the two limit flags the reference
//! has no field for. Callers must call in from threads with at least 512 KB of
//! stack; the crate is built at opt-level 1 or more in every profile.

use std::fmt;
use std::sync::Arc;

use openui_lang::{Library, ParseResult};

uniffi::setup_scaffolding!();

/// Something the core couldn't accept.
#[derive(Debug, uniffi::Error)]
pub enum OpenUiError {
    /// The component library's JSON Schema isn't valid JSON.
    InvalidSchema { message: String },
}

impl fmt::Display for OpenUiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenUiError::InvalidSchema { message } => write!(f, "invalid JSON Schema: {message}"),
        }
    }
}

impl std::error::Error for OpenUiError {}

/// A component library, loaded once and shared by every parse against it.
#[derive(uniffi::Object)]
pub struct ComponentLibrary {
    inner: Arc<Library>,
}

#[uniffi::export]
impl ComponentLibrary {
    /// Loads the library from the JSON Schema `library.toJSONSchema()` produces.
    #[uniffi::constructor]
    pub fn new(schema: String) -> Result<Arc<Self>, OpenUiError> {
        let json: serde_json::Value =
            serde_json::from_str(&schema).map_err(|e| OpenUiError::InvalidSchema {
                message: e.to_string(),
            })?;
        Ok(Arc::new(ComponentLibrary {
            inner: Arc::new(Library::from_json_schema(&json)),
        }))
    }

    /// The components' names, in the schema's order.
    pub fn component_names(&self) -> Vec<String> {
        self.inner.component_names().map(str::to_owned).collect()
    }
}

/// A parse result as it crosses the boundary.
#[derive(Debug, PartialEq, Eq, uniffi::Record)]
pub struct ParseOutput {
    /// The parse result, serialized exactly as the reference serializes it.
    pub json: String,
    /// Nesting went past the core's depth limit and was cut.
    pub depth_limited: bool,
    /// The result grew past the core's size limit and was cut.
    pub size_limited: bool,
}

impl From<ParseResult> for ParseOutput {
    fn from(result: ParseResult) -> Self {
        ParseOutput {
            json: result.to_json(),
            depth_limited: result.depth_limited,
            size_limited: result.size_limited,
        }
    }
}

/// One-shot parse of a whole program; `root_name` picks the entry statement.
#[uniffi::export]
pub fn parse(
    program: String,
    library: Arc<ComponentLibrary>,
    root_name: Option<String>,
) -> ParseOutput {
    openui_lang::parse(&program, &library.inner, root_name.as_deref()).into()
}
