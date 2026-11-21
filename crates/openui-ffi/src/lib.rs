//! The core behind a UniFFI interface, for the viewer's Swift today and
//! Dynamic News's Kotlin later.
//!
//! Parse results cross as the JSON text [`ParseResult::to_json`] produces,
//! byte-identical with the reference, plus the two limit flags the reference
//! has no field for and how long the core took. Callers may call in from any
//! thread, whatever its stack: the core grows its stack on demand, and a
//! result frees its tree without recursion (ADR 0002).
//!
//! [`RuntimeSession`] shares [`ParsedProgram`] objects from parser outputs and
//! confines state, action plans and mutation continuations to one worker thread.
//! Evaluation and event JSON come directly from `openui-runtime`. See
//! `docs/runtime-binding.md` and ADR 0006 for capability lifetime and threading.
//! [`RuntimeSession::register_function`] supplies JSON-only [`LibraryFunction`]
//! callbacks on one ordinary-stack callback thread, started after a non-mutating
//! live-core admission query and before the first registration. A failed start returns
//! [`OpenUiError::Worker`] without changing the session, so registration may retry.
//! Refused/failed handoffs release on the fenced registering caller; an admitted
//! handoff or callback transport failure revokes the session with a sticky
//! Worker error.
//! Disposal/final drop join both threads. Same-session reentry refuses, other
//! threads queue. See the binding docs for synchronous host responsibilities
//! and action failure semantics.
//! A marker is a bounded expression-write capability, never a host action.
//! Sessions suppress clicks while a response bundle is blocked. The runtime
//! Store enforces the evaluation result-size ceiling on values and total state
//! (ADR 0005); the binding does not duplicate its admission rules.

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use openui_lang::{Library, Limits, ParseResult, StreamParser};

mod message;
mod response_bundle;
pub use message::{build_message, parse_message};
pub use response_bundle::{ResponseRead, parse_response_bundle};
mod queries;
pub use queries::*;
mod runtime;
pub use runtime::*;

uniffi::setup_scaffolding!();

/// Extract an MCP tool call outcome as JSON containing `value` or `error`.
/// Malformed input, JavaScript response errors and limit refusals throw the
/// binding's InvalidJson, Evaluation or LimitExceeded error respectively.
#[uniffi::export]
pub fn extract_mcp_tool_result(
    response_json: String,
    limits: ParseLimits,
) -> Result<String, OpenUiError> {
    openui_session::extract_mcp_tool_result(&response_json, limits.to_core()).map_err(Into::into)
}

/// Something the core couldn't accept. The field is `reason`, not `message`,
/// which Kotlin's exceptions already have.
#[derive(Debug, uniffi::Error)]
pub enum OpenUiError {
    /// The runtime session was normally disposed.
    Disposed,
    /// A capability belongs to a different session.
    ForeignHandle,
    /// An evaluation released the capability's generation.
    StaleHandle,
    /// A continuation was already resumed.
    ConsumedHandle,
    /// A newer mutation suspension evicted this pending continuation.
    EvictedHandle,
    /// No retained action exists at that path.
    InvalidHandle,
    /// The input is not a binding marker.
    InvalidMarker,
    /// Host JSON is malformed, too deep, or has the wrong top-level shape.
    InvalidJson { reason: String },
    /// A runtime write was refused before changing that value.
    RefusedWrite { reason: String },
    /// Expression evaluation failed before a write or while running an action.
    Evaluation { reason: String },
    /// A binding resource limit was exceeded.
    LimitExceeded { reason: String },
    /// Worker creation, transport or a caught Rust panic failed the session.
    Worker { reason: String },
    /// The component library's JSON Schema isn't valid JSON.
    InvalidSchema { reason: String },
    /// The prompt spec isn't valid JSON.
    InvalidPromptSpec { reason: String },
    /// The core can't make a system prompt from the spec, with the message
    /// the reference throws.
    Prompt { reason: String },
    /// Supplied reference values and a tool provider are mutually exclusive.
    ReferenceConflict,
    /// The tool call expired, belongs to another manager, or was already completed.
    InvalidToolCall,
}

impl fmt::Display for OpenUiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disposed => f.write_str("session disposed"),
            Self::ForeignHandle => f.write_str("foreign capability"),
            Self::StaleHandle => f.write_str("stale action capability"),
            Self::ConsumedHandle => f.write_str("continuation consumed"),
            Self::EvictedHandle => f.write_str("continuation evicted"),
            Self::InvalidToolCall => f.write_str("invalid tool call identity"),
            Self::InvalidHandle => f.write_str("invalid action path"),
            Self::ReferenceConflict => {
                f.write_str("tool provider conflicts with supplied reference values")
            }
            Self::InvalidMarker => f.write_str("invalid binding marker"),
            Self::InvalidJson { reason }
            | Self::RefusedWrite { reason }
            | Self::Evaluation { reason }
            | Self::LimitExceeded { reason }
            | Self::Worker { reason } => f.write_str(reason),
            OpenUiError::InvalidSchema { reason } => write!(f, "invalid JSON Schema: {reason}"),
            OpenUiError::InvalidPromptSpec { reason } => {
                write!(f, "invalid prompt spec: {reason}")
            }
            OpenUiError::Prompt { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for OpenUiError {}

/// A component library, loaded once and shared by every parse against it.
#[derive(uniffi::Object)]
pub struct ComponentLibrary {
    inner: Arc<Library>,
    schema: Arc<serde_json::Value>,
}

#[uniffi::export]
impl ComponentLibrary {
    /// Loads the library from the JSON Schema `library.toJSONSchema()` produces.
    #[uniffi::constructor]
    pub fn new(schema: String) -> Result<Arc<Self>, OpenUiError> {
        let json: serde_json::Value =
            serde_json::from_str(&schema).map_err(|e| OpenUiError::InvalidSchema {
                reason: e.to_string(),
            })?;
        Ok(Arc::new(ComponentLibrary {
            inner: Arc::new(Library::from_json_schema(&json)),
            schema: Arc::new(json),
        }))
    }

    /// The components' names, in the schema's order.
    pub fn component_names(&self) -> Vec<String> {
        self.inner.component_names().map(str::to_owned).collect()
    }
}

/// An immutable parsed program, created only by parsing or streaming.
/// Passing it to a runtime session clones one Arc, never the program or JSON.
#[derive(Debug, uniffi::Object)]
pub struct ParsedProgram {
    inner: Arc<ParseResult>,
    json_bytes: u64,
}

/// A parse result as it crosses the boundary.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ParseOutput {
    /// The actual shared program. Changing the JSON fields cannot change it.
    pub program: Arc<ParsedProgram>,
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
    pub(crate) fn timed(result: ParseResult, started: Instant) -> ParseOutput {
        let parsed = Instant::now();
        let json = result.to_json();
        let depth_limited = result.depth_limited;
        let size_limited = result.size_limited;
        ParseOutput {
            program: Arc::new(ParsedProgram {
                json_bytes: u64::try_from(json.len()).unwrap_or(u64::MAX),
                inner: Arc::new(result),
            }),
            json,
            depth_limited,
            size_limited,
            parse_nanos: nanos(started, parsed),
            json_nanos: nanos(parsed, Instant::now()),
        }
    }
}

/// The core's limits (its `Limits`), each one left out meaning its default:
/// 1 MiB of program; a result of the larger of 64 KiB and 64 times the
/// program's size in JSON, never above 8 MiB; 256 levels of nesting; and
/// evaluating state defaults in the larger of a million steps and 1,000 per
/// byte of program, at most 64 million. Hitting one cuts the result and sets
/// `size_limited` or `depth_limited`.
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
    /// How many steps evaluating state defaults may take, for every program;
    /// unset, it scales with the program.
    #[uniffi(default = None)]
    pub work: Option<u64>,
    /// Optional portable evaluation-memory allowance (ADR 0011); unset by default.
    #[uniffi(default = None)]
    pub peak_memory: Option<u64>,
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
        if let Some(n) = self.work {
            limits = limits.with_work(size(n));
        }
        if let Some(n) = self.peak_memory {
            limits = limits.with_peak_memory(size(n));
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
    generate_system_prompt_with_limits(spec, ParseLimits::default())
}

/// [`generate_system_prompt`] within `limits`: a spec nesting past the JSON
/// nesting limit described by [`openui_lang::Limits::depth`] is refused as a `Prompt` error.
/// The spec is read with serde_json, which refuses nesting past 128 levels on
/// its own, as `InvalidPromptSpec`.
#[uniffi::export]
pub fn generate_system_prompt_with_limits(
    spec: String,
    limits: ParseLimits,
) -> Result<String, OpenUiError> {
    let json: serde_json::Value =
        serde_json::from_str(&spec).map_err(|e| OpenUiError::InvalidPromptSpec {
            reason: e.to_string(),
        })?;
    openui_lang::generate_system_prompt_with_limits(&json, limits.to_core()).map_err(|e| {
        OpenUiError::Prompt {
            reason: e.to_string(),
        }
    })
}

/// Merges an edit-mode patch into a program by statement name, as the
/// reference's `mergeStatements` does; `root_id` defaults to `root`.
#[uniffi::export]
pub fn merge_statements(existing: String, patch: String, root_id: Option<String>) -> String {
    openui_lang::merge_statements(&existing, &patch, root_id.as_deref())
}

/// A merge as it crosses the boundary: the merged program, and whether a
/// limit cut it.
#[derive(Debug, uniffi::Record)]
pub struct MergeOutput {
    /// The merged program.
    pub text: String,
    /// An input was longer than the program-size limit and was read only up
    /// to it.
    pub size_limited: bool,
    /// An expression nested past the depth limit, so statements only its cut
    /// part reaches may have been dropped as unreachable.
    pub depth_limited: bool,
}

/// [`merge_statements`] within `limits`: the program-size limit applies to
/// each input, and the depth limit to each statement's expression.
#[uniffi::export]
pub fn merge_statements_with_limits(
    existing: String,
    patch: String,
    root_id: Option<String>,
    limits: ParseLimits,
) -> MergeOutput {
    let merged = openui_lang::merge_statements_with_limits(
        &existing,
        &patch,
        root_id.as_deref(),
        limits.to_core(),
    );
    MergeOutput {
        text: merged.text,
        size_limited: merged.size_limited,
        depth_limited: merged.depth_limited,
    }
}

/// A streaming parse of one program, fed chunk by chunk. Calls on one session
/// are serialized; a viewer drives it from a single thread anyway.
#[derive(uniffi::Object)]
pub struct StreamSession {
    parser: Mutex<StreamState>,
}

struct StreamState {
    /// The raw response reader and its parser, which `push` and `set`
    /// drive too: one parser per session, whichever way text arrives.
    reader: openui_lang::ResponseReader,
    output: Option<ParseOutput>,
    complete: bool,
    known_text: Option<String>,
    program_limit: usize,
}
impl StreamState {
    fn new(parser: StreamParser, program_limit: usize) -> Self {
        Self {
            // Two limits, on purpose. The reader refuses a whole response
            // past the core's default limits (program size, JSON depth), as
            // `parse_response_bundle` did for the Swift driver; the parser
            // keeps the session's own and cuts a longer program, flagged
            // `size_limited`, as `set` did with that program. Reading under
            // the session's limits would refuse responses that drew before.
            reader: openui_lang::ResponseReader::new(parser, Limits::DEFAULT),
            output: None,
            complete: false,
            known_text: Some(String::new()),
            program_limit,
        }
    }
    fn remember_chunk(&mut self, chunk: String) {
        if let Some(mut text) = self.known_text.take()
            && text.len().saturating_add(chunk.len()) <= self.program_limit
        {
            if text.is_empty() {
                self.known_text = Some(chunk);
            } else {
                text.push_str(&chunk);
                self.known_text = Some(text);
            }
        }
    }
    fn output(&mut self, result: ParseResult, started: Instant, complete: bool) -> ParseOutput {
        // Never deduplicate by JSON: equal JSON can hide fresh container
        // identities relevant to Store initialization. Cache only known no-ops.
        let output = ParseOutput::timed(result, started);
        self.complete = complete;
        self.output = Some(output.clone());
        output
    }
    fn current(&mut self, started: Instant, complete: bool) -> ParseOutput {
        if complete == self.complete
            && let Some(output) = &self.output
        {
            let parsed = Instant::now();
            let mut output = output.clone();
            output.parse_nanos = nanos(started, parsed);
            output.json_nanos = nanos(parsed, Instant::now());
            return output;
        }
        let result = if complete {
            self.reader.finish()
        } else {
            self.reader.result()
        };
        self.output(result, started, complete)
    }
}
impl StreamSession {
    fn parser(&self) -> MutexGuard<'_, StreamState> {
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
            parser: Mutex::new(StreamState::new(parser, Limits::DEFAULT.program_size())),
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
            parser: Mutex::new(StreamState::new(parser, limits.to_core().program_size())),
        })
    }

    /// Pushes the next chunk; the result after it.
    pub fn push(&self, chunk: String) -> ParseOutput {
        let started = Instant::now();
        let mut state = self.parser();
        if chunk.is_empty() || state.known_text.is_none() {
            return state.current(started, false);
        }
        let result = state.reader.parser_mut().push(&chunk);
        state.remember_chunk(chunk);
        state.output(result, started, false)
    }

    /// Pushes several chunks one by one and returns only the result after the
    /// last, for a viewer that skips steps. The core sees exactly the pushes it
    /// would have seen one call at a time, so the result is the same.
    pub fn push_all(&self, chunks: Vec<String>) -> ParseOutput {
        let started = Instant::now();
        let mut state = self.parser();
        if state.known_text.is_none() {
            return state.current(started, false);
        }
        let mut result = None;
        for chunk in chunks.into_iter().filter(|c| !c.is_empty()) {
            result = Some(state.reader.parser_mut().push(&chunk));
            state.remember_chunk(chunk);
        }
        match result {
            Some(result) => state.output(result, started, false),
            None => state.current(started, false),
        }
    }

    /// Gives the whole text so far, as after a reconnect.
    pub fn set(&self, text: String) -> ParseOutput {
        let started = Instant::now();
        let mut state = self.parser();
        if state.known_text.as_deref() == Some(text.as_str())
            && state.output.as_ref().is_some_and(|o| !o.size_limited)
        {
            return state.current(started, false);
        }
        let result = state.reader.parser_mut().set(&text);
        state.known_text = (text.len() <= state.program_limit).then_some(text);
        state.output(result, started, false)
    }

    /// The latest result, without new text.
    pub fn result(&self) -> ParseOutput {
        let started = Instant::now();
        self.parser().current(started, false)
    }

    /// The result once the stream has ended, after its last chunk. Until
    /// then, a `$name` at the very end may still be growing and isn't declared
    /// as state yet.
    pub fn finish(&self) -> ParseOutput {
        let started = Instant::now();
        self.parser().current(started, true)
    }

    /// Read the whole raw response so far and parse its program, as
    /// `parse_response_bundle`, `parse_message` and `set` with the program
    /// do together, but remembering how far it got: a response that only
    /// grows costs about what pushing it does, and any other edit is read
    /// afresh (ADR 0015). The parse is this session's, so `finish`, `push`
    /// and `set` go on from it. Refuses what `parse_response_bundle` or
    /// `parse_message` refuse, with the same errors, and then the session
    /// is as it was. `parse.parse_nanos` covers the reading and the parse.
    pub fn read_response(
        &self,
        text: String,
        streaming: bool,
    ) -> Result<response_bundle::ResponseRead, OpenUiError> {
        let started = Instant::now();
        let mut state = self.parser();
        response_bundle::read(&mut state, &text, streaming, started)
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
