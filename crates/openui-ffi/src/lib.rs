//! The core behind a UniFFI interface, for the viewer's Swift today and
//! Dynamic News's Kotlin later.
//!
//! Parse results cross as the JSON text [`ParseResult::to_json`] produces,
//! byte-identical with the reference, plus the two limit flags the reference
//! has no field for and how long the core took. Callers may call in from any
//! thread, whatever its stack: the core grows its stack on demand, and a
//! result frees its tree without recursion (ADR 0002).

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use openui_lang::{Library, Limits, ParseResult, StreamParser};

uniffi::setup_scaffolding!();

/// Something the core couldn't accept.
#[derive(Debug, uniffi::Error)]
pub enum OpenUiError {
    /// The component library's JSON Schema isn't valid JSON.
    InvalidSchema { message: String },
    /// The prompt spec isn't valid JSON.
    InvalidPromptSpec { message: String },
    /// The core can't make a system prompt from the spec, with the message
    /// the reference throws.
    Prompt { message: String },
}

impl fmt::Display for OpenUiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenUiError::InvalidSchema { message } => write!(f, "invalid JSON Schema: {message}"),
            OpenUiError::InvalidPromptSpec { message } => {
                write!(f, "invalid prompt spec: {message}")
            }
            OpenUiError::Prompt { message } => f.write_str(message),
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
#[derive(Debug, uniffi::Record)]
pub struct ParseOutput {
    /// The parse result, serialized exactly as the reference serializes it.
    pub json: String,
    /// Nesting went past the core's depth limit and was cut.
    pub depth_limited: bool,
    /// The result grew past the core's size limit and was cut.
    pub size_limited: bool,
    /// Time the core spent parsing, in nanoseconds.
    pub parse_nanos: u64,
    /// Time spent serializing the result to `json`, in nanoseconds.
    pub json_nanos: u64,
}

impl ParseOutput {
    /// The result and how long it took to produce, from when parsing started.
    fn timed(result: ParseResult, started: Instant) -> ParseOutput {
        let parsed = Instant::now();
        let json = result.to_json();
        ParseOutput {
            json,
            depth_limited: result.depth_limited,
            size_limited: result.size_limited,
            parse_nanos: nanos(started, parsed),
            json_nanos: nanos(parsed, Instant::now()),
        }
    }
}

/// The core's limits (its `Limits`), each one left out meaning its default:
/// 1 MiB of program; a result of the larger of 64 KiB and 64 times the
/// program's size in JSON, never above 8 MiB; 256 levels of nesting.
/// Hitting one cuts the result and sets `size_limited` or `depth_limited`.
/// New limits are added as fields that default to unset, so code that builds
/// one keeps compiling.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct ParseLimits {
    /// How many bytes of a program are read; the rest is ignored as if the
    /// stream had stopped there.
    #[uniffi(default = None)]
    pub program_size: Option<u64>,
    /// A fixed result-size limit, in bytes of JSON, whatever the program's
    /// size: it sets both the floor and the ceiling. Applied before the three
    /// below, which can then change either.
    #[uniffi(default = None)]
    pub result_size: Option<u64>,
    /// The fewest bytes of JSON the result-size limit scales down to.
    #[uniffi(default = None)]
    pub result_floor: Option<u64>,
    /// How many times its program's size a result may grow to, between the
    /// floor and the ceiling.
    #[uniffi(default = None)]
    pub amplification: Option<u64>,
    /// The most bytes of JSON a result may grow to, however large the
    /// program.
    #[uniffi(default = None)]
    pub result_ceiling: Option<u64>,
    /// How deep expressions and references may nest.
    #[uniffi(default = None)]
    pub depth: Option<u64>,
}

impl ParseLimits {
    /// The core's limits: these, the defaults where unset. A value past what
    /// the platform can count is as good as no limit.
    fn to_core(&self) -> Limits {
        let size = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
        let mut limits = Limits::default();
        if let Some(n) = self.program_size {
            limits = limits.with_program_size(size(n));
        }
        if let Some(n) = self.result_size {
            limits = limits.with_result_size(size(n));
        }
        if let Some(n) = self.result_floor {
            limits = limits.with_result_floor(size(n));
        }
        if let Some(n) = self.amplification {
            limits = limits.with_amplification(size(n));
        }
        if let Some(n) = self.result_ceiling {
            limits = limits.with_result_ceiling(size(n));
        }
        if let Some(n) = self.depth {
            limits = limits.with_depth(size(n));
        }
        limits
    }
}

fn nanos(from: Instant, to: Instant) -> u64 {
    u64::try_from((to - from).as_nanos()).unwrap_or(u64::MAX)
}

/// One-shot parse of a whole program. `root_name` is the library's root
/// component: with no `root` statement, the first statement calling it is the
/// entry.
#[uniffi::export]
pub fn parse(
    program: String,
    library: Arc<ComponentLibrary>,
    root_name: Option<String>,
) -> ParseOutput {
    let started = Instant::now();
    let result = openui_lang::parse(&program, &library.inner, root_name.as_deref());
    ParseOutput::timed(result, started)
}

/// [`parse`] within `limits` instead of the core's defaults.
#[uniffi::export]
pub fn parse_with_limits(
    program: String,
    library: Arc<ComponentLibrary>,
    root_name: Option<String>,
    limits: ParseLimits,
) -> ParseOutput {
    let started = Instant::now();
    let result = openui_lang::parse_with_limits(
        &program,
        &library.inner,
        root_name.as_deref(),
        limits.to_core(),
    );
    ParseOutput::timed(result, started)
}

/// The system prompt for a prompt spec: `{library, promptOptions}` as the
/// reference's `generateSystemPrompt` takes it, as JSON text.
#[uniffi::export]
pub fn generate_system_prompt(spec: String) -> Result<String, OpenUiError> {
    let json: serde_json::Value =
        serde_json::from_str(&spec).map_err(|e| OpenUiError::InvalidPromptSpec {
            message: e.to_string(),
        })?;
    openui_lang::generate_system_prompt(&json).map_err(|e| OpenUiError::Prompt {
        message: e.to_string(),
    })
}

/// Merges an edit-mode patch into a program by statement name, as the
/// reference's `mergeStatements` does; `root_id` defaults to `root`.
#[uniffi::export]
pub fn merge_statements(existing: String, patch: String, root_id: Option<String>) -> String {
    openui_lang::merge_statements(&existing, &patch, root_id.as_deref())
}

/// A streaming parse of one program, fed chunk by chunk. Calls on one session
/// are serialized; a viewer drives it from a single thread anyway.
#[derive(uniffi::Object)]
pub struct StreamSession {
    parser: Mutex<StreamParser>,
}

impl StreamSession {
    fn parser(&self) -> MutexGuard<'_, StreamParser> {
        // The core doesn't panic, so the lock is never poisoned in practice.
        self.parser
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[uniffi::export]
impl StreamSession {
    /// A session over `library`; `root_name` is the library's root component,
    /// as for `parse`.
    #[uniffi::constructor]
    pub fn new(library: Arc<ComponentLibrary>, root_name: Option<String>) -> Arc<Self> {
        let parser = StreamParser::new(library.inner.clone(), root_name.as_deref());
        Arc::new(StreamSession {
            parser: Mutex::new(parser),
        })
    }

    /// A session within `limits` instead of the core's defaults, for every
    /// push. Past the program-size limit the stream is cut: later pushes are
    /// ignored until `set`.
    #[uniffi::constructor]
    pub fn with_limits(
        library: Arc<ComponentLibrary>,
        root_name: Option<String>,
        limits: ParseLimits,
    ) -> Arc<Self> {
        let parser = StreamParser::with_limits(
            library.inner.clone(),
            root_name.as_deref(),
            limits.to_core(),
        );
        Arc::new(StreamSession {
            parser: Mutex::new(parser),
        })
    }

    /// Pushes the next chunk; the result after it.
    pub fn push(&self, chunk: String) -> ParseOutput {
        let started = Instant::now();
        let result = self.parser().push(&chunk);
        ParseOutput::timed(result, started)
    }

    /// Pushes several chunks one by one and returns only the result after the
    /// last, for a viewer that skips steps. The core sees exactly the pushes it
    /// would have seen one call at a time, so the result is the same.
    pub fn push_all(&self, chunks: Vec<String>) -> ParseOutput {
        let started = Instant::now();
        let mut parser = self.parser();
        let mut result = None;
        for chunk in &chunks {
            result = Some(parser.push(chunk));
        }
        let result = result.unwrap_or_else(|| parser.result());
        ParseOutput::timed(result, started)
    }

    /// Gives the whole text so far, as after a reconnect.
    pub fn set(&self, text: String) -> ParseOutput {
        let started = Instant::now();
        let result = self.parser().set(&text);
        ParseOutput::timed(result, started)
    }

    /// The latest result, without new text.
    pub fn result(&self) -> ParseOutput {
        let started = Instant::now();
        let result = self.parser().result();
        ParseOutput::timed(result, started)
    }

    /// The result once the stream has ended, after its last chunk. Until
    /// then, a `$name` at the very end may still be growing and isn't declared
    /// as state yet.
    pub fn finish(&self) -> ParseOutput {
        let started = Instant::now();
        let result = self.parser().finish();
        ParseOutput::timed(result, started)
    }
}
