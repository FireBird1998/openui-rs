//! Single-threaded session policy shared by native and WebAssembly bindings.
#![forbid(unsafe_code)]
use openui_lang::{JsObject, Limits, ParseResult, Value};
use openui_runtime::{
    ActionContinuation, BindingProps, LibraryFunctions, PreparedReferenceValues, Store, TreeInput,
};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak, mpsc};

mod mcp;
mod mutations;
mod queries;
mod refresh;
mod tool_results;
pub use mcp::extract_mcp_tool_result;
pub use openui_runtime::{ToolCall as WantedToolCall, ToolCallOutcome};
pub use queries::{QueryEffects, ResponseState};

#[derive(Clone, Debug, Eq, PartialEq)]
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

/// The core's limits (its `Limits`), each one left out meaning its default:
/// 1 MiB of program; a result of the larger of 64 KiB and 64 times the
/// program's size in JSON, never above 8 MiB; 256 levels of nesting; and
/// evaluating state defaults in the larger of a million steps and 1,000 per
/// byte of program, at most 64 million. Hitting one cuts the result and sets
/// `size_limited` or `depth_limited`.
/// New limits are added as fields that default to unset, so code that builds
/// one keeps compiling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParseLimits {
    /// How many bytes of a program are read; the rest is ignored as if the
    /// stream had stopped there.
    pub program_size: Option<u64>,
    /// A fixed result-size limit, in bytes of JSON, whatever the program's
    /// size: it sets both the floor and the ceiling. Applied before the three
    /// below, which can then change either.
    pub result_size: Option<u64>,
    /// The fewest bytes of JSON the result-size limit scales down to.
    pub result_floor: Option<u64>,
    /// How many times its program's size a result may grow to, between the
    /// floor and the ceiling.
    pub amplification: Option<u64>,
    /// The most bytes of JSON a result may grow to, however large the
    /// program.
    pub result_ceiling: Option<u64>,
    /// How deep expressions and references may nest.
    pub depth: Option<u64>,
    /// How many steps evaluating state defaults may take, for every program;
    /// unset, it scales with the program.
    pub work: Option<u64>,
    /// Optional portable evaluation-memory allowance (ADR 0011); unset by default.
    pub peak_memory: Option<u64>,
}

impl ParseLimits {
    /// The core's limits: these, the defaults where unset. A value past what
    /// the platform can count is as good as no limit.
    pub fn to_core(&self) -> Limits {
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

/// Shared compiled library and original action schema.
pub struct ComponentLibrary {
    pub inner: Arc<openui_lang::Library>,
    pub schema: Arc<serde_json::Value>,
}
/// Shared parser output; JSON carries no authority.
#[derive(Debug)]
pub struct ParsedProgram {
    pub inner: Arc<ParseResult>,
    pub json_bytes: u64,
}
/// Limits retained by a session; every operation uses these same bounds.
/// Runtime evaluation uses the core's limits. JSON admission and retained
/// capability counts additionally bound binding-owned input and tables.
#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    /// Evaluation, binding, form and action limits, including action resumes.
    /// The result-size ceiling also caps each value and total retained Store
    /// state (8 MiB by default), independently of scaled drawing room.
    pub evaluation: ParseLimits,
    /// Independent supplied-reference preparation bounds. Default depth is
    /// 32,768 with the core's default work/size ceilings, exactly as Rust's
    /// PreparedReferenceValues::new; this never spends evaluation work.
    /// Setting peak_memory here refuses session construction with LimitExceeded.
    pub reference_preparation: ParseLimits,
    /// Byte bound for host text/JSON, program JSON, action paths and explicit
    /// binding annotations (default 8 MiB). Draws use only runtime limits.
    pub max_json_bytes: u64,
    /// Maximum simultaneous suspended runs (default 64, minimum 1). A new
    /// host handoff evicts the oldest pending run when full. A provider-managed
    /// submission at capacity returns a size cut at its Run step.
    pub max_continuations: u64,
    /// Optional smaller retained action-path bound. Absent preserves max_json_bytes.
    pub max_action_path_bytes: Option<u64>,
    /// The query cache's byte cap: the most bytes of tool result data, as
    /// JSON, the session holds at once, across live cache entries (a
    /// previous key's included) and mutation results. Absent uses the
    /// evaluation result-size ceiling. A result that would exceed it is
    /// delivered as a `tool-error`; nothing held is evicted.
    pub max_cache_bytes: Option<u64>,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            evaluation: ParseLimits::default(),
            reference_preparation: ParseLimits {
                depth: Some(32_768),
                ..ParseLimits::default()
            },
            max_json_bytes: 8 * 1024 * 1024,
            max_continuations: 64,
            max_action_path_bytes: None,
            max_cache_bytes: None,
        }
    }
}

/// A component/prop annotation that the prompt-spec reader could not decide.
#[derive(Debug)]
pub struct BindingPair {
    /// Component name.
    pub component: String,
    /// Prop name.
    pub prop: String,
}

/// Identity of one evaluated tree. Only the latest successful evaluation lives.
/// There is no public constructor or numeric representation.
#[derive(Debug)]
pub struct EvaluationHandle {
    owner: Arc<()>,
    generation: u64,
}

/// A retained action at a path in an evaluation. Reusable until the next
/// successful evaluation or program replacement; JSON cannot construct one.
#[derive(Debug)]
pub struct ActionHandle {
    evaluation: Arc<EvaluationHandle>,
    path: Vec<String>,
}

/// A single-use mutation outcome capability. The run stays in SessionCore.
#[derive(Debug)]
pub struct ContinuationHandle {
    owner: Arc<()>,
    generation: u64,
    release: mpsc::Sender<u64>,
    notify: Option<ReleaseNotifier>,
    evicted: AtomicBool,
}

impl Drop for ContinuationHandle {
    fn drop(&mut self) {
        let _ = self.release.send(self.generation);
        if let Some(notify) = &self.notify {
            (notify.0)();
        }
    }
}

/// The runtime's exact `{root,errors}` JSON and separate limit flags.
#[derive(Debug)]
pub struct EvaluationOutput {
    pub query_evaluation_error: Option<OpenUiError>,
    pub query_snapshot_json: Option<String>,
    /// Byte-identical to `openui_runtime::TreeResult::to_json`.
    pub json: String,
    /// Work or result size cut the evaluation.
    pub size_limited: bool,
    /// Nesting cut the evaluation.
    pub depth_limited: bool,
    /// Use with `action_at`; never embedded in the tree JSON.
    pub evaluation: Arc<EvaluationHandle>,
}

/// Store initialization's batch result, including per-name refusals.
#[derive(Debug)]
pub struct InitializationOutput {
    /// At least one value changed.
    pub changed: bool,
    /// Refused entries, in the store's encounter order.
    pub refused: Vec<StateRefusal>,
}

/// One committed replacement, including all data needed to publish a drawing.
#[derive(Debug)]
pub struct ReplacementOutput {
    pub initialization: InitializationOutput,
    pub projection: InitializationOutput,
    pub evaluation: EvaluationOutput,
    pub snapshot: String,
    pub version: u64,
}

/// A refused entry in initialization; other entries can still apply.
#[derive(Debug)]
pub struct StateRefusal {
    /// State name.
    pub name: String,
    /// Typed binding error translated from the runtime's store error.
    pub error: OpenUiError,
}

/// A form write's change and renderer save decision.
#[derive(Debug)]
pub struct FieldWriteOutput {
    /// Store value changed.
    pub changed: bool,
    /// Embedder should request persistence.
    pub save_requested: bool,
}
impl From<openui_runtime::FieldWrite> for FieldWriteOutput {
    fn from(value: openui_runtime::FieldWrite) -> Self {
        Self {
            changed: value.changed,
            save_requested: value.save_requested,
        }
    }
}

/// One segment of an action run, before or after a mutation handoff.
#[derive(Clone, Debug)]
pub struct ActionOutput {
    /// Ordered `HostEvent::to_json` strings. Nothing has been sent to the host.
    pub events: Vec<String>,
    /// First failure, if any; earlier effects remain applied.
    pub failure: Option<ActionFailureOutput>,
    /// Work or result size cut the run.
    pub size_limited: bool,
    /// Nesting cut the run.
    pub depth_limited: bool,
    /// Present when a mutation awaits its host outcome.
    pub continuation: Option<Arc<ContinuationHandle>>,
}

/// First stopped action step, with a typed reason.
#[derive(Clone, Debug)]
pub struct ActionFailureOutput {
    /// Zero-based step, absent for legacy/default actions.
    pub step: Option<u64>,
    /// Translated runtime failure.
    pub error: OpenUiError,
}

/// A synchronous JSON-only implementation. None represents undefined; an error
/// is the thrown message, formatted by the runtime as `@Name threw: ...`.
/// Host work is outside evaluator accounting. Bindings contain foreign throws
/// and refuse reentry; synchronous host callbacks must return for progress.
pub type HostFunction = dyn Fn(String) -> Result<Option<String>, String> + Send + Sync;

pub struct SessionCore {
    manager: Option<openui_runtime::QueryManager>,
    query_effects: QueryEffects,
    streaming: bool,
    query_inputs_changed: bool,
    library: Arc<ComponentLibrary>,
    program: Arc<ParseResult>,
    functions: BTreeMap<String, Arc<HostFunction>>,
    function_bytes: u64,
    bindings: BindingProps,
    explicit_bindings: HashSet<(String, String)>,
    binding_bytes: u64,
    store: Store,
    limits: RuntimeLimits,
    core_limits: Limits,
    pub reference_limits: Limits,
    owner: Arc<()>,
    generation: u64,
    root: Option<Value>,
    release: mpsc::Sender<u64>,
    notify: Option<ReleaseNotifier>,
    released: mpsc::Receiver<u64>,
    next_continuation: u64,
    continuations: BTreeMap<u64, PendingContinuation>,
}
impl SessionCore {
    pub fn new(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
        limits: RuntimeLimits,
    ) -> Result<Self, OpenUiError> {
        if limits.reference_preparation.peak_memory.is_some() {
            return Err(limit(
                "referencePreparation.peakMemory is unsupported; set evaluation.peakMemory",
            ));
        }
        if limits.max_continuations == 0 {
            return Err(limit("maxContinuations must be at least one"));
        }
        let (release, released) = mpsc::channel();
        let core_limits = limits.evaluation.to_core();
        let reference_limits = limits.reference_preparation.to_core();
        let mut s = Self {
            manager: None,
            query_effects: QueryEffects::default(),
            streaming: true,
            query_inputs_changed: true,
            library,
            program: Arc::new(ParseResult::default()),
            functions: BTreeMap::new(),
            function_bytes: 0,
            bindings: BindingProps::new(),
            explicit_bindings: HashSet::new(),
            binding_bytes: 0,
            store: Store::with_limits(core_limits),
            limits,
            core_limits,
            reference_limits,
            owner: Arc::new(()),
            generation: 0,
            root: None,
            release,
            notify: None,
            released,
            next_continuation: 0,
            continuations: BTreeMap::new(),
        };
        if let Some(spec) = prompt_spec {
            let spec = s.json(&spec)?;
            let library = match &spec {
                Value::Obj(o) => o.get("library").unwrap_or(&spec),
                _ => {
                    return Err(OpenUiError::InvalidJson {
                        reason: "expected a prompt spec object".into(),
                    });
                }
            };
            if !matches!(library, Value::Obj(_)) {
                return Err(OpenUiError::InvalidJson {
                    reason: "expected a prompt library object".into(),
                });
            }
            s.bindings = BindingProps::from_prompt_spec(library).map_err(|e| {
                runtime_error(RuntimeFailure::Limit(e), s.core_limits.result_size())
            })?;
        }
        Ok(s)
    }
    /// Register or replace a declared function. At most 256 names, additionally
    /// bounded by max_json_bytes (name bytes plus 64 bytes per entry). Refusal
    /// preserves the previous callback. Registrations survive program changes.
    pub fn register_function(
        &mut self,
        name: String,
        callback: Arc<HostFunction>,
    ) -> Result<(), OpenUiError> {
        let total = self.function_registration_bytes(&name)?;
        self.functions.insert(name, callback);
        self.query_inputs_changed = true;
        self.function_bytes = total;
        Ok(())
    }
    /// Check registration against the live registry without changing callbacks
    /// or retained room. Uses the same admission path as `register_function`.
    pub fn check_function_registration(&self, name: &str) -> Result<(), OpenUiError> {
        self.function_registration_bytes(name).map(|_| ())
    }
    fn function_registration_bytes(&self, name: &str) -> Result<u64, OpenUiError> {
        self.text(name)?;
        if self
            .library
            .schema
            .get("functions")
            .and_then(|f| f.as_object())
            .and_then(|f| f.get(name))
            .is_none()
        {
            return Err(OpenUiError::InvalidJson {
                reason: "expected a declared library function name".into(),
            });
        }
        let replacing = self.functions.contains_key(name);
        let bytes = (name.len() as u64).saturating_add(64);
        let total = self
            .function_bytes
            .saturating_add(if replacing { 0 } else { bytes });
        if total > self.limits.max_json_bytes || (!replacing && self.functions.len() >= 256) {
            return Err(limit("function registration limit"));
        }
        Ok(total)
    }
    // Borrow only definitions. Each operation uses the runtime's own function
    // validation and argument cache, without replaying or pre-evaluating calls.
    fn functions<'a>(
        library: &'a ComponentLibrary,
        registered: &BTreeMap<String, Arc<HostFunction>>,
        limits: Limits,
        max_bytes: u64,
    ) -> Result<Option<LibraryFunctions<'a>>, OpenUiError> {
        if registered.is_empty() {
            return Ok(None);
        }
        let definitions = library
            .schema
            .get("functions")
            .unwrap_or(&serde_json::Value::Null);
        let mut functions = LibraryFunctions::new(definitions);
        for (name, callback) in registered {
            let callback = callback.clone();
            functions
                .register(name, move |arguments| {
                    let json = openui_runtime::value_to_json_with_limits(&arguments, limits)
                        .map_err(|e| e.to_string())?;
                    if json.len() as u64 > max_bytes {
                        return Err("host function argument byte limit".into());
                    }
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(json)))
                            .map_err(|_| "host function panicked".to_owned())??;
                    result
                        .map(|json| {
                            if json.len() as u64 > max_bytes {
                                return Err("host function result byte limit".into());
                            }
                            let json = Json(
                                openui_lang::parse_json_with_limits(&json, limits)
                                    .map_err(|e| e.to_string())?,
                            );
                            Ok(Value::from_json(&json.0))
                        })
                        .transpose()
                })
                .map_err(|e| limit(&e.to_string()))?;
        }
        Ok(Some(functions))
    }
    fn text(&self, text: &str) -> Result<(), OpenUiError> {
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > self.limits.max_json_bytes {
            Err(limit("host input byte limit"))
        } else {
            Ok(())
        }
    }
    fn json(&self, text: &str) -> Result<Value, OpenUiError> {
        self.text(text)?;
        let json = Json(
            openui_lang::parse_json_with_limits(text, self.core_limits).map_err(|e| {
                OpenUiError::InvalidJson {
                    reason: e.to_string(),
                }
            })?,
        );
        Ok(Value::from_json(&json.0))
    }
    pub fn object(&self, text: &str) -> Result<JsObject<Value>, OpenUiError> {
        let value = self.json(text)?;
        match &value {
            Value::Obj(o) => Ok(o.as_ref().clone()),
            _ => Err(OpenUiError::InvalidJson {
                reason: "expected a JSON object".into(),
            }),
        }
    }
    // Retained values were already admitted. Export does not spend a new
    // evaluation allowance or charge empty-object/envelope bytes to Store room.
    fn retained_json(&self, value: &Value) -> Result<String, OpenUiError> {
        openui_runtime::value_to_json(value)
            .map_err(|e| runtime_error(RuntimeFailure::Limit(e), self.core_limits.result_size()))
    }
    fn next_continuation_generation(&self) -> Result<u64, OpenUiError> {
        self.next_continuation
            .checked_add(1)
            .ok_or_else(|| limit("continuation generations exhausted"))
    }
    fn field_text(
        &self,
        form: Option<&str>,
        component: Option<&str>,
        name: &str,
    ) -> Result<(), OpenUiError> {
        self.text(name)?;
        for text in [form, component].into_iter().flatten() {
            self.text(text)?;
        }
        Ok(())
    }
    fn release_evaluation(&mut self) -> Result<(), OpenUiError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| limit("evaluation generations exhausted"))?;
        self.root = None;
        Ok(())
    }
    fn check_evaluation(&self, evaluation: &EvaluationHandle) -> Result<(), OpenUiError> {
        if !Arc::ptr_eq(&self.owner, &evaluation.owner) {
            return Err(OpenUiError::ForeignHandle);
        }
        if self.generation != evaluation.generation {
            return Err(OpenUiError::StaleHandle);
        }
        Ok(())
    }
    fn action_value(&self, path: &[String]) -> Result<&Value, OpenUiError> {
        action_value(self.root.as_ref(), path)
    }
    fn run_value(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        action: Option<&Value>,
        message: &str,
        form: Option<&str>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.field_text(form, None, message)?;
        // Check generation space before any effects. Only a handoff claims it.
        self.next_continuation_generation()?;
        let functions = Self::functions(
            &self.library,
            &self.functions,
            self.core_limits,
            self.limits.max_json_bytes,
        )?;
        let managed = self
            .manager
            .as_ref()
            .map(|m| m.prepared_results(self.reference_limits));
        let refs = managed.as_ref().unwrap_or(refs);
        let input = tree_input(
            &self.program,
            &self.library,
            &self.bindings,
            refs,
            functions.as_ref(),
        );
        let input = queries::with_provider_runs(input, managed.is_some());
        let run = openui_runtime::run_action_with_limits(
            input,
            &mut self.store,
            action,
            message,
            form,
            self.core_limits,
        );
        drop(functions);
        self.output(run)
    }
    fn output(&mut self, mut run: openui_runtime::ActionRun) -> Result<ActionOutput, OpenUiError> {
        // Queries run as their steps did, before any later operation; like
        // the run's writes, they stand even if serializing an event fails.
        self.run_queries(&std::mem::take(&mut run.query_runs))?;
        // Events were checked by the runtime against the same result ceiling.
        // No smaller host-input, work or depth allowance may newly refuse them
        // after earlier action effects. Serialize before publishing/evicting a
        // continuation; all runtime admission errors use the same mapper.
        let events = run
            .events
            .iter()
            .map(|event| event.to_json())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| runtime_error(RuntimeFailure::Limit(e), self.core_limits.result_size()))?;
        let mut output = ActionOutput {
            events,
            failure: run.failure.take().map(|f| ActionFailureOutput {
                step: f.step.map(|v| v as u64),
                error: runtime_error(
                    RuntimeFailure::Binding(f.cause),
                    self.core_limits.result_size(),
                ),
            }),
            size_limited: run.size_limited,
            depth_limited: run.depth_limited,
            continuation: None,
        };
        output.continuation = if let Some(continuation) = run.continuation.take() {
            let generation = self.next_continuation_generation()?;
            if let Some(request) = run.mutation.take() {
                self.dispatch_mutation(request, continuation, generation, &mut output);
                return Ok(output);
            }
            self.next_continuation = generation;
            if self.continuations.len() as u64 >= self.limits.max_continuations
                && let Some((_, oldest)) = self.continuations.pop_first()
                && let Some(handle) = oldest.handle.upgrade()
            {
                handle.evicted.store(true, Ordering::Release);
            }
            let handle = Arc::new(ContinuationHandle {
                owner: self.owner.clone(),
                generation: self.next_continuation,
                release: self.release.clone(),
                notify: self.notify.clone(),
                evicted: AtomicBool::new(false),
            });
            self.continuations.insert(
                self.next_continuation,
                PendingContinuation {
                    continuation,
                    handle: Arc::downgrade(&handle),
                    call: None,
                },
            );
            Some(handle)
        } else {
            None
        };
        Ok(output)
    }
}

fn initialization_output(
    result: openui_runtime::InitializationResult,
    limits: Limits,
) -> InitializationOutput {
    InitializationOutput {
        changed: result.changed,
        refused: result
            .refused
            .into_iter()
            .map(|r| StateRefusal {
                name: r.name,
                error: runtime_error(RuntimeFailure::Store(r.error), limits.result_size()),
            })
            .collect(),
    }
}
fn snapshot_json(store: &Store) -> Result<String, openui_runtime::LimitError> {
    let mut object = JsObject::new();
    for (name, value) in store.snapshot().iter() {
        if let Some(value) = value {
            object.define(name, value.clone());
        }
    }
    openui_runtime::value_to_json(&Value::Obj(Arc::new(object)))
}
fn check_native_view(root: Option<&Value>, json: &str, snapshot: &str) -> Result<(), OpenUiError> {
    let element = match root {
        None | Some(Value::Null) | Some(Value::Element(_)) => true,
        Some(Value::Obj(o)) => {
            let keys: Vec<_> = o.iter().map(|(k, _)| k).collect();
            (keys == ["type", "typeName", "props", "partial", "hasDynamicProps"]
                || keys
                    == [
                        "type",
                        "typeName",
                        "props",
                        "partial",
                        "hasDynamicProps",
                        "statementId",
                    ])
                && matches!(o.get("type"), Some(Value::Str(s)) if s == "element")
                && matches!(o.get("typeName"), Some(Value::Str(_)))
                && matches!(o.get("props"), Some(Value::Obj(_)))
                && matches!(o.get("partial"), Some(Value::Bool(_)))
                && matches!(o.get("hasDynamicProps"), Some(Value::Bool(_)))
                && o.get("statementId")
                    .is_none_or(|v| matches!(v, Value::Str(_)))
        }
        _ => false,
    };
    if !element {
        return Err(OpenUiError::Evaluation {
            reason: "native view root is not an element".into(),
        });
    }
    // Both strings are produced by runtime serializers. Scan their nesting
    // without reparsing, charging work again, or imposing a new byte limit.
    for text in [json, snapshot] {
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        for byte in text.bytes() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'}' | b']' => depth = depth.saturating_sub(1),
                b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' => {}
                _ => {
                    if depth >= 512 {
                        return Err(limit("native view JSON nested too deeply"));
                    }
                    match byte {
                        b'{' | b'[' => depth += 1,
                        b'"' => quoted = true,
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn tree_input<'a>(
    program: &'a Arc<ParseResult>,
    library: &'a ComponentLibrary,
    bindings: &'a BindingProps,
    refs: &'a PreparedReferenceValues<'a>,
    functions: Option<&'a LibraryFunctions<'a>>,
) -> TreeInput<'a> {
    program_input(program, library, bindings, functions).with_prepared_reference_values(refs)
}

fn program_input<'a>(
    program: &'a Arc<ParseResult>,
    library: &'a ComponentLibrary,
    bindings: &'a BindingProps,
    functions: Option<&'a LibraryFunctions<'a>>,
) -> TreeInput<'a> {
    let input = TreeInput::from_shared_result(program)
        .with_library(&library.inner)
        .with_binding_props(bindings);
    let input = match functions {
        Some(functions) => input.with_functions(functions),
        None => input,
    };
    match library.schema.get("actions") {
        Some(actions) => input.with_actions(actions),
        None => input,
    }
}

impl SessionCore {
    fn call<T>(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        operation: impl FnOnce(&mut Self, &PreparedReferenceValues<'_>) -> Result<T, OpenUiError>,
    ) -> Result<T, OpenUiError> {
        if self.manager.is_some() && !refs.is_empty() {
            return Err(OpenUiError::ReferenceConflict);
        }
        self.drain_releases();
        let version = self.store.version();
        let result = operation(self, refs)?;
        if self.store.version() != version {
            self.state_changed();
        }
        Ok(result)
    }
    /// Drop queued pending runs and return the number actually released.
    pub fn drain_releases(&mut self) -> usize {
        let mut released = 0;
        while let Ok(generation) = self.released.try_recv() {
            if self.continuations.remove(&generation).is_some() {
                released += 1;
            }
        }
        released
    }
    /// Replace the current program by sharing its Arc. No JSON is parsed or
    /// program cloned. Old action handles become stale; continuations survive
    /// and resume against the new program, as the runtime specifies.
    pub fn set_program(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        program: Arc<ParsedProgram>,
    ) -> Result<(), OpenUiError> {
        self.call(refs, move |s, _| {
            if program.json_bytes > s.limits.max_json_bytes {
                return Err(limit("program JSON byte limit"));
            }
            s.release_evaluation()?;
            s.program = program.inner.clone();
            s.streaming = true;
            Ok(())
        })
    }

    /// Atomically replace, project edited state, initialize and evaluate.
    /// Per-entry initialization refusals remain successful batch results.
    /// `native_view` requires an element/null root and JSON readable by the
    /// existing Swift decoder (depth below 512), before committing anything.
    /// Diagnostics and cut flags are successful evaluations, as with evaluate.
    /// External effects of synchronous host functions cannot be rolled back.
    pub fn replace_program(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
    ) -> Result<ReplacementOutput, OpenUiError> {
        self.replace_program_with_response(
            refs,
            program,
            persisted,
            projected,
            native_view,
            ResponseState::default(),
        )
    }

    pub fn replace_program_with_response(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
        response: ResponseState,
    ) -> Result<ReplacementOutput, OpenUiError> {
        if self.manager.is_some() && !refs.is_empty() {
            return Err(OpenUiError::ReferenceConflict);
        }
        // Match Swift show's persisted preflight error precedence, using the
        // actual session's reader. Deliberately do not drain releases on refusal.
        let persisted = persisted
            .map(|json| self.object(&json))
            .transpose()?
            .unwrap_or_default();
        if program.json_bytes > self.limits.max_json_bytes {
            return Err(limit("program JSON byte limit"));
        }
        let generation = self
            .generation
            .checked_add(2)
            .ok_or_else(|| limit("evaluation generations exhausted"))?;
        let projected = projected.map(|json| self.object(&json)).transpose()?;
        let functions = Self::functions(
            &self.library,
            &self.functions,
            self.core_limits,
            self.limits.max_json_bytes,
        )?;
        let (projection, initialization, ((tree, json, snapshot, version), update)) =
            self.store.initialize_transaction(
                &program.inner.state_declarations,
                &persisted,
                projected.as_ref(),
                |store| {
                    queries::draw(
                        self.manager.as_mut(),
                        !response.streaming,
                        &program_input(
                            &program.inner,
                            &self.library,
                            &self.bindings,
                            functions.as_ref(),
                        ),
                        store,
                        refs,
                        self.reference_limits,
                        |input| {
                            let tree = openui_runtime::evaluate_tree_with_limits(
                                input,
                                store,
                                self.core_limits,
                            );
                            let json = tree.to_json().map_err(|e| {
                                runtime_error(
                                    RuntimeFailure::Limit(e),
                                    self.core_limits.result_size(),
                                )
                            })?;
                            let snapshot = snapshot_json(store).map_err(|e| {
                                runtime_error(
                                    RuntimeFailure::Limit(e),
                                    self.core_limits.result_size(),
                                )
                            })?;
                            if native_view {
                                check_native_view(tree.root.as_ref(), &json, &snapshot)?;
                            }
                            let version = store.version();
                            Ok((tree, json, snapshot, version))
                        },
                    )
                },
            )?;
        drop(functions);
        self.program = program.inner.clone();
        self.generation = generation;
        self.root = tree.root;
        self.drain_releases();
        let (query_snapshot_json, query_evaluation_error) = self.finish_draw(response, update);
        Ok(ReplacementOutput {
            projection: initialization_output(projection, self.core_limits),
            initialization: initialization_output(initialization, self.core_limits),
            evaluation: EvaluationOutput {
                query_snapshot_json,
                query_evaluation_error,
                json,
                size_limited: tree.size_limited,
                depth_limited: tree.depth_limited,
                evaluation: Arc::new(EvaluationHandle {
                    owner: self.owner.clone(),
                    generation,
                }),
            },
            snapshot,
            version,
        })
    }

    /// Report prompt annotations needing an explicit choice by the embedder.
    pub fn undecided_bindings(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
    ) -> Result<Vec<BindingPair>, OpenUiError> {
        self.call(refs, |s, _| {
            Ok(s.bindings
                .undecided()
                .map(|(c, p)| BindingPair {
                    component: c.into(),
                    prop: p.into(),
                })
                .collect())
        })
    }

    /// Resolve a binding annotation explicitly; affects the next evaluation.
    pub fn mark_binding(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        component: String,
        prop: String,
    ) -> Result<(), OpenUiError> {
        self.call(refs, move |s, _| {
            s.text(&component)?;
            s.text(&prop)?;
            let key = (component, prop);
            if !s.explicit_bindings.contains(&key) {
                let bytes = (key.0.len() as u64)
                    .saturating_add(key.1.len() as u64)
                    .saturating_add(64);
                let total = s.binding_bytes.saturating_add(bytes);
                if total > s.limits.max_json_bytes {
                    return Err(limit("binding annotation byte limit"));
                }
                s.bindings.insert(&key.0, &key.1).map_err(|e| {
                    runtime_error(RuntimeFailure::Limit(e), s.core_limits.result_size())
                })?;
                s.explicit_bindings.insert(key);
                s.binding_bytes = total;
            }
            Ok(())
        })
    }

    /// Apply current declarations and optional persisted object. Omitted JSON
    /// means no persistence. JSON null is a defined value; absent keys are not.
    pub fn initialize(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        persisted: Option<String>,
    ) -> Result<InitializationOutput, OpenUiError> {
        self.call(refs, move |s, _| {
            let persisted = persisted
                .map(|j| s.object(&j))
                .transpose()?
                .unwrap_or_default();
            let result = s.store.initialize(
                s.program
                    .state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                persisted.iter().map(|(k, v)| (k.as_str(), Some(v))),
            );
            Ok(InitializationOutput {
                changed: result.changed,
                refused: result
                    .refused
                    .into_iter()
                    .map(|r| StateRefusal {
                        name: r.name,
                        error: runtime_error(
                            RuntimeFailure::Store(r.error),
                            s.core_limits.result_size(),
                        ),
                    })
                    .collect(),
            })
        })
    }

    /// Draw explicitly. Keep the runtime's evaluated root without indexing it;
    /// action lookup walks a root-relative JSON path lazily. Only runtime limits
    /// cut a draw. A successful draw releases the preceding generation.
    pub fn evaluate(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
    ) -> Result<EvaluationOutput, OpenUiError> {
        self.evaluate_with_response(refs, ResponseState::default())
    }

    pub fn evaluate_with_response(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        response: ResponseState,
    ) -> Result<EvaluationOutput, OpenUiError> {
        self.call(refs, |s, refs| {
            let functions = Self::functions(
                &s.library,
                &s.functions,
                s.core_limits,
                s.limits.max_json_bytes,
            )?;
            let generation = s
                .generation
                .checked_add(1)
                .ok_or_else(|| limit("evaluation generations exhausted"))?;
            let ((tree, json), update) = queries::draw(
                s.manager.as_mut(),
                !response.streaming && (s.streaming || s.query_inputs_changed),
                &program_input(&s.program, &s.library, &s.bindings, functions.as_ref()),
                &s.store,
                refs,
                s.reference_limits,
                |input| {
                    let tree =
                        openui_runtime::evaluate_tree_with_limits(input, &s.store, s.core_limits);
                    let json = tree.to_json().map_err(|e| {
                        runtime_error(RuntimeFailure::Limit(e), s.core_limits.result_size())
                    })?;
                    Ok((tree, json))
                },
            )?;
            drop(functions);
            let (query_snapshot_json, query_evaluation_error) = s.finish_draw(response, update);
            s.generation = generation;
            s.root = tree.root;
            Ok(EvaluationOutput {
                query_snapshot_json,
                query_evaluation_error,
                json,
                size_limited: tree.size_limited,
                depth_limited: tree.depth_limited,
                evaluation: Arc::new(EvaluationHandle {
                    owner: s.owner.clone(),
                    generation: s.generation,
                }),
            })
        })
    }

    /// Path follows JSON from `root`, e.g. `["props","children","0","props","onClick"]`.
    /// Array indices are decimal strings. Plans in nested elements, arrays and
    /// Each output use exactly those displayed paths. Lookup costs path length;
    /// action handles retain no tree, plan table or previous evaluation.
    pub fn action_at(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        evaluation: Arc<EvaluationHandle>,
        path: Vec<String>,
    ) -> Result<Arc<ActionHandle>, OpenUiError> {
        self.call(refs, move |s, _| {
            s.check_evaluation(&evaluation)?;
            let mut bytes = 0u64;
            for key in &path {
                bytes = bytes.saturating_add(key.len() as u64).saturating_add(1);
                if bytes
                    > s.limits
                        .max_json_bytes
                        .min(s.limits.max_action_path_bytes.unwrap_or(u64::MAX))
                {
                    return Err(limit("action path byte limit"));
                }
            }
            s.action_value(&path)?;
            Ok(Arc::new(ActionHandle { evaluation, path }))
        })
    }

    /// Undefined is None; JSON null is Some("null"). State names include `$`.
    pub fn get(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        name: String,
    ) -> Result<Option<String>, OpenUiError> {
        self.call(refs, move |s, _| {
            s.text(&name)?;
            s.store
                .get(&name)
                .as_ref()
                .map(|v| s.retained_json(v))
                .transpose()
        })
    }
    /// Host JSON never grants custom-step identity. None writes undefined.
    pub fn set(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        name: String,
        json: Option<String>,
    ) -> Result<bool, OpenUiError> {
        self.call(refs, move |s, _| {
            s.text(&name)?;
            let value = json.map(|j| s.json(&j)).transpose()?;
            s.store
                .set(&name, value)
                .map_err(|e| runtime_error(RuntimeFailure::Store(e), s.core_limits.result_size()))
        })
    }
    /// JSON.stringify-compatible snapshot: undefined entries are omitted.
    pub fn snapshot(&mut self, refs: &PreparedReferenceValues<'_>) -> Result<String, OpenUiError> {
        self.call(refs, |s, _| {
            let mut object = JsObject::new();
            for (name, value) in s.store.snapshot().iter() {
                if let Some(value) = value {
                    object.define(name, value.clone());
                }
            }
            s.retained_json(&Value::Obj(Arc::new(object)))
        })
    }
    /// Number of mutation outcomes currently retained by this session.
    pub fn pending_continuations(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
    ) -> Result<u64, OpenUiError> {
        self.call(refs, |s, _| Ok(s.continuations.len() as u64))
    }

    /// Store notification count.
    pub fn version(&mut self, refs: &PreparedReferenceValues<'_>) -> Result<u64, OpenUiError> {
        self.call(refs, |s, _| Ok(s.store.version()))
    }
    /// Whether a later declared default may still replace this name.
    pub fn is_pristine(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        name: String,
    ) -> Result<bool, OpenUiError> {
        self.call(refs, move |s, _| {
            s.text(&name)?;
            Ok(s.store.is_pristine(&name))
        })
    }
    /// A marker can write any state target and call declared custom actions,
    /// minting real steps for a later draw/click. Accept only marker JSON from
    /// the given tree, never model output or tool data. This call runs no host
    /// action; finished plan JSON alone cannot restore custom authority.
    pub fn write_binding(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        marker: String,
        json: String,
    ) -> Result<bool, OpenUiError> {
        self.call(refs, move |s, refs| {
            let marker = s.json(&marker)?;
            let value = s.json(&json)?;
            let functions = Self::functions(
                &s.library,
                &s.functions,
                s.core_limits,
                s.limits.max_json_bytes,
            )?;
            let managed = s
                .manager
                .as_ref()
                .map(|m| m.prepared_results(s.reference_limits));
            let refs = managed.as_ref().unwrap_or(refs);
            openui_runtime::write_binding_with_limits(
                tree_input(
                    &s.program,
                    &s.library,
                    &s.bindings,
                    refs,
                    functions.as_ref(),
                ),
                &mut s.store,
                &marker,
                value,
                s.core_limits,
            )
            .map_err(|e| runtime_error(RuntimeFailure::Binding(e), s.core_limits.result_size()))
        })
    }
    /// Read an unbound renderer form field, including wrapped values.
    pub fn get_field(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        form: Option<String>,
        name: String,
    ) -> Result<Option<String>, OpenUiError> {
        self.call(refs, move |s, _| {
            s.field_text(form.as_deref(), None, &name)?;
            openui_runtime::get_field_value(&s.store, form.as_deref(), &name)
                .as_ref()
                .map(|v| s.retained_json(v))
                .transpose()
        })
    }
    /// Write a renderer field; report the runtime's save request separately.
    pub fn set_field(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        save: bool,
    ) -> Result<FieldWriteOutput, OpenUiError> {
        self.call(refs, move |s, _| {
            s.field_text(form.as_deref(), component_type.as_deref(), &name)?;
            let value = json.map(|j| s.json(&j)).transpose()?;
            openui_runtime::set_field_value_with_limits(
                &mut s.store,
                form.as_deref(),
                component_type.as_deref(),
                &name,
                value,
                save,
                s.core_limits,
            )
            .map(Into::into)
            .map_err(|e| runtime_error(RuntimeFailure::Binding(e), s.core_limits.result_size()))
        })
    }
    /// Persist a settled default only when the field is undefined.
    pub fn initialize_field(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        streaming: bool,
    ) -> Result<Option<FieldWriteOutput>, OpenUiError> {
        self.call(refs, move |s, _| {
            s.field_text(form.as_deref(), component_type.as_deref(), &name)?;
            let value = json.map(|j| s.json(&j)).transpose()?;
            openui_runtime::initialize_field_default_with_limits(
                &mut s.store,
                form.as_deref(),
                component_type.as_deref(),
                &name,
                value,
                streaming,
                s.core_limits,
            )
            .map(|v| v.map(Into::into))
            .map_err(|e| runtime_error(RuntimeFailure::Binding(e), s.core_limits.result_size()))
        })
    }
    /// Run a retained plan. None invokes the runtime's no-action message event.
    /// Ignore clicks while a bundled response is blocked in the embedder.
    pub fn run(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        action: Option<Arc<ActionHandle>>,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(refs, move |s, refs| {
            let value = if let Some(action) = action {
                s.check_evaluation(&action.evaluation)?;
                Some(s.action_value(&action.path)?.clone())
            } else {
                None
            };
            s.run_value(refs, value.as_ref(), &message, form.as_deref())
        })
    }
    /// Supply a legacy object without `steps`. Plans must use action handles;
    /// legacy names still pass the runtime's custom-action shadowing checks.
    pub fn run_legacy(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        json: String,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(refs, move |s, refs| {
            let value = s.json(&json)?;
            if !matches!(&value, Value::Obj(o) if !o.has_own("steps")) {
                return Err(OpenUiError::InvalidJson {
                    reason:
                        "expected a legacy object without steps; plans require an action handle"
                            .into(),
                });
            }
            s.run_value(refs, Some(&value), &message, form.as_deref())
        })
    }
    /// Consume a mutation outcome once. False ends the run. True uses current
    /// program/state/reference inputs with the original action work allowance.
    pub fn resume(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        continuation: Arc<ContinuationHandle>,
        succeeded: bool,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(refs, move |s, refs| {
            if !Arc::ptr_eq(&s.owner, &continuation.owner) {
                return Err(OpenUiError::ForeignHandle);
            }
            if continuation.evicted.load(Ordering::Acquire) {
                return Err(OpenUiError::EvictedHandle);
            }
            if !s.continuations.contains_key(&continuation.generation) {
                return Err(OpenUiError::ConsumedHandle);
            }
            if succeeded {
                s.next_continuation_generation()?;
            }
            let entry = s
                .continuations
                .remove(&continuation.generation)
                .ok_or(OpenUiError::ConsumedHandle)?;
            let functions = Self::functions(
                &s.library,
                &s.functions,
                s.core_limits,
                s.limits.max_json_bytes,
            )?;
            let managed = s
                .manager
                .as_ref()
                .map(|m| m.prepared_results(s.reference_limits));
            let refs = managed.as_ref().unwrap_or(refs);
            let run = openui_runtime::resume_action(
                queries::with_provider_runs(
                    tree_input(
                        &s.program,
                        &s.library,
                        &s.bindings,
                        refs,
                        functions.as_ref(),
                    ),
                    managed.is_some(),
                ),
                &mut s.store,
                &entry.continuation,
                succeeded,
            )
            .map_err(|e| runtime_error(RuntimeFailure::Resume(e), s.core_limits.result_size()))?;
            drop(functions);
            s.output(run)
        })
    }
}
// Resolve exactly the JSON path, borrowing element props instead of copying a
// header at each level. No tree walk or index is needed when drawing.
fn action_value<'a>(root: Option<&'a Value>, path: &[String]) -> Result<&'a Value, OpenUiError> {
    enum Cursor<'a> {
        Value(&'a Value),
        Props(&'a JsObject<Value>),
    }
    let mut cursor = Cursor::Value(root.ok_or(OpenUiError::InvalidHandle)?);
    for key in path {
        cursor = match cursor {
            Cursor::Value(Value::Element(e)) if key == "props" => Cursor::Props(&e.props),
            Cursor::Value(Value::Obj(o)) => {
                Cursor::Value(o.get(key).ok_or(OpenUiError::InvalidHandle)?)
            }
            Cursor::Props(o) => Cursor::Value(o.get(key).ok_or(OpenUiError::InvalidHandle)?),
            Cursor::Value(Value::Arr(a)) => {
                let index = key
                    .parse::<usize>()
                    .map_err(|_| OpenUiError::InvalidHandle)?;
                if index.to_string() != *key {
                    return Err(OpenUiError::InvalidHandle);
                }
                Cursor::Value(a.get(index).ok_or(OpenUiError::InvalidHandle)?)
            }
            _ => return Err(OpenUiError::InvalidHandle),
        };
    }
    match cursor {
        Cursor::Value(v @ Value::Obj(o)) if matches!(o.get("steps"), Some(Value::Arr(_))) => Ok(v),
        _ => Err(OpenUiError::InvalidHandle),
    }
}

struct PendingContinuation {
    continuation: ActionContinuation,
    handle: Weak<ContinuationHandle>,
    call: Option<openui_runtime::ToolCallId>,
}

enum RuntimeFailure {
    Limit(openui_runtime::LimitError),
    Store(openui_runtime::StoreError),
    Binding(openui_runtime::BindingWriteError),
    Resume(openui_runtime::ActionResumeError),
}
// One place to translate the runtime's non-exhaustive refusal categories.
fn runtime_error(error: RuntimeFailure, state_cap: usize) -> OpenUiError {
    use openui_runtime::{ActionResumeError as R, BindingWriteError as B};
    match error {
        RuntimeFailure::Limit(e) => limit(&e.to_string()),
        RuntimeFailure::Store(e) => OpenUiError::RefusedWrite {
            reason: format!("{e} (state cap: {state_cap} JSON bytes)"),
        },
        RuntimeFailure::Binding(B::InvalidMarker) => OpenUiError::InvalidMarker,
        RuntimeFailure::Binding(B::Store(e)) => runtime_error(RuntimeFailure::Store(e), state_cap),
        RuntimeFailure::Binding(B::Cut(_)) => limit("runtime evaluation cut"),
        RuntimeFailure::Binding(e) => OpenUiError::Evaluation {
            reason: e.to_string(),
        },
        RuntimeFailure::Resume(R::Disposed) => OpenUiError::Disposed,
        RuntimeFailure::Resume(R::ForeignSession) => OpenUiError::ForeignHandle,
        RuntimeFailure::Resume(R::Consumed) => OpenUiError::ConsumedHandle,
        RuntimeFailure::Resume(e) => worker_error(&e.to_string()),
    }
}
fn limit(reason: &str) -> OpenUiError {
    OpenUiError::LimitExceeded {
        reason: reason.into(),
    }
}
fn worker_error(reason: &str) -> OpenUiError {
    OpenUiError::Worker {
        reason: reason.into(),
    }
}

struct Json(serde_json::Value);
impl Drop for Json {
    fn drop(&mut self) {
        openui_lang::free_json(std::mem::take(&mut self.0));
    }
}

/// A binding transport wakeup. It never receives a runtime object or capability ID.
#[derive(Clone)]
pub struct ReleaseNotifier(pub Arc<dyn Fn() + Send + Sync>);
impl std::fmt::Debug for ReleaseNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReleaseNotifier")
    }
}
impl SessionCore {
    /// Wake the binding's event loop after a token is dropped on any thread.
    pub fn set_release_notifier(&mut self, notify: ReleaseNotifier) {
        self.notify = Some(notify);
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuation_generation_refusals_precede_effects_and_consumption() {
        let schema = serde_json::json!({"$defs":{"Box":{"properties":{"a":{}}}}});
        let library = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
        });
        let parsed = Arc::new(openui_lang::parse(
            "$n=0\nm=Mutation(\"m\",{})\nroot=Box(Action([@Set($n,$n+1),@Run(m),@Set($n,$n+1)]))",
            &library.inner,
            None,
        ));
        let program = Arc::new(ParsedProgram {
            json_bytes: parsed.to_json().len() as u64,
            inner: parsed,
        });
        let mut session = SessionCore::new(library, None, RuntimeLimits::default()).unwrap();
        let values = JsObject::new();
        let refs = PreparedReferenceValues::new(&values);
        session.set_program(&refs, program).unwrap();
        session.initialize(&refs, None).unwrap();
        let tree = session.evaluate(&refs).unwrap();
        let action = session
            .action_at(&refs, tree.evaluation, vec!["props".into(), "a".into()])
            .unwrap();
        session.next_continuation = u64::MAX;
        let before = session.snapshot(&refs).unwrap();
        let version = session.store.version();
        assert!(matches!(
            session.run(&refs, Some(action.clone()), String::new(), None),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert_eq!(session.snapshot(&refs).unwrap(), before);
        assert_eq!(session.store.version(), version);
        assert!(session.continuations.is_empty());
        session.next_continuation = 0;
        let pending = session
            .run(&refs, Some(action), String::new(), None)
            .unwrap()
            .continuation
            .unwrap();
        session.next_continuation = u64::MAX;
        let before = session.snapshot(&refs).unwrap();
        let version = session.store.version();
        assert!(matches!(
            session.resume(&refs, pending.clone(), true),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert_eq!(session.snapshot(&refs).unwrap(), before);
        assert_eq!(session.store.version(), version);
        assert_eq!(session.continuations.len(), 1);
        // Abandoning remains possible at exhaustion and consumes exactly once.
        session.resume(&refs, pending.clone(), false).unwrap();
        assert!(session.continuations.is_empty());
        assert!(matches!(
            session.resume(&refs, pending, true),
            Err(OpenUiError::ConsumedHandle)
        ));
    }

    #[test]
    fn an_exhausted_provider_free_draw_keeps_the_last_root_and_handle() {
        let schema = serde_json::json!({"$defs":{"Box":{"properties":{"action":{}}}}});
        let library = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
        });
        let program = Arc::new(ParsedProgram {
            inner: Arc::new(openui_lang::parse(
                "root=Box(@Set($n,1))",
                &library.inner,
                None,
            )),
            json_bytes: 0,
        });
        let mut session = SessionCore::new(library, None, RuntimeLimits::default()).unwrap();
        let values = JsObject::new();
        let refs = PreparedReferenceValues::new(&values);
        session.set_program(&refs, program).unwrap();
        session.generation = u64::MAX - 1;
        let last = session.evaluate(&refs).unwrap().evaluation;
        assert!(matches!(
            session.evaluate(&refs),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        let action = session
            .action_at(&refs, last, vec!["props".into(), "action".into()])
            .unwrap();
        session
            .run(&refs, Some(action), String::new(), None)
            .unwrap();
        assert_eq!(
            session.get(&refs, "$n".into()).unwrap().as_deref(),
            Some("1")
        );
    }

    #[test]
    fn program_handoff_keeps_the_original_shared_parse_result() {
        let schema = serde_json::json!({"$defs":{}});
        let library = Arc::new(ComponentLibrary {
            inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
            schema: Arc::new(schema),
        });
        let parsed = Arc::new(ParsedProgram {
            inner: Arc::new(openui_lang::parse("$n=1", &library.inner, None)),
            json_bytes: 0,
        });
        let mut session = SessionCore::new(library, None, RuntimeLimits::default()).unwrap();
        let values = JsObject::new();
        let refs = PreparedReferenceValues::new(&values);
        session.set_program(&refs, parsed.clone()).unwrap();
        assert!(Arc::ptr_eq(&session.program, &parsed.inner));
    }
}
