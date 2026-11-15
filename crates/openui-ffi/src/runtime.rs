//! Runtime adaptation lives here so changes to the runtime's safety API have
//! one binding seam. No evaluator, action validation or authority rules live here.

use crate::{ComponentLibrary, OpenUiError, ParseLimits, ParsedProgram};
use openui_lang::{JsObject, Limits, ParseResult, Value};
use openui_runtime::{ActionContinuation, BindingProps, PreparedReferenceValues, Store, TreeInput};
use std::collections::{BTreeMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread::JoinHandle;

/// Limits retained by a session; every operation uses these same bounds.
/// Runtime evaluation uses the core's limits. JSON admission and retained
/// capability counts additionally bound binding-owned input and tables.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RuntimeLimits {
    /// Evaluation, binding, form and action limits, including action resumes.
    /// The result-size ceiling also caps each value and total retained Store
    /// state (8 MiB by default), independently of scaled drawing room.
    pub evaluation: ParseLimits,
    /// Independent supplied-reference preparation bounds. Default depth is
    /// 32,768 with the core's default work/size ceilings, exactly as Rust's
    /// PreparedReferenceValues::new; this never spends evaluation work.
    pub reference_preparation: ParseLimits,
    /// Byte bound for host text/JSON, program JSON, action paths and explicit
    /// binding annotations (default 8 MiB). Draws use only runtime limits.
    pub max_json_bytes: u64,
    /// Maximum simultaneous suspended runs (default 64, minimum 1). A new
    /// suspension evicts the oldest pending run when full.
    pub max_continuations: u64,
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
        }
    }
}

/// A component/prop annotation that the prompt-spec reader could not decide.
#[derive(Debug, uniffi::Record)]
pub struct BindingPair {
    /// Component name.
    pub component: String,
    /// Prop name.
    pub prop: String,
}

/// Identity of one evaluated tree. Only the latest successful evaluation lives.
/// There is no public constructor or numeric representation.
#[derive(Debug, uniffi::Object)]
pub struct EvaluationHandle {
    owner: Arc<()>,
    generation: u64,
}

/// A retained action at a path in an evaluation. Reusable until the next
/// successful evaluation or program replacement; JSON cannot construct one.
#[derive(Debug, uniffi::Object)]
pub struct ActionHandle {
    evaluation: Arc<EvaluationHandle>,
    path: Vec<String>,
}

/// A single-use mutation outcome capability. The run itself stays on the worker.
#[derive(Debug, uniffi::Object)]
pub struct ContinuationHandle {
    owner: Arc<()>,
    generation: u64,
    transport: Weak<Mailbox>,
    evicted: AtomicBool,
}

impl Drop for ContinuationHandle {
    fn drop(&mut self) {
        // An unbounded control channel makes destruction nonblocking, including
        // destruction on the worker or a Kotlin cleaner thread. No run crosses it.
        if let Some(transport) = self.transport.upgrade() {
            let _ = transport
                .sender
                .send(Command::ReleaseContinuation(self.generation));
        }
    }
}

/// The runtime's exact `{root,errors}` JSON and separate limit flags.
#[derive(Debug, uniffi::Record)]
pub struct EvaluationOutput {
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
#[derive(Debug, uniffi::Record)]
pub struct InitializationOutput {
    /// At least one value changed.
    pub changed: bool,
    /// Refused entries, in the store's encounter order.
    pub refused: Vec<StateRefusal>,
}

/// A refused entry in initialization; other entries can still apply.
#[derive(Debug, uniffi::Record)]
pub struct StateRefusal {
    /// State name.
    pub name: String,
    /// Typed binding error translated from the runtime's store error.
    pub error: OpenUiError,
}

/// A form write's change and renderer save decision.
#[derive(Debug, uniffi::Record)]
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
#[derive(Debug, uniffi::Record)]
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
#[derive(Debug, uniffi::Record)]
pub struct ActionFailureOutput {
    /// Zero-based step, absent for legacy/default actions.
    pub step: Option<u64>,
    /// Translated runtime failure.
    pub error: OpenUiError,
}

type Job = Box<dyn FnOnce(&mut Worker, &PreparedReferenceValues<'_>) -> bool + Send>;
enum Command {
    Call(Job),
    References(String, mpsc::SyncSender<Result<(), OpenUiError>>),
    ReleaseContinuation(u64),
    Stop,
}
#[derive(Debug)]
struct Mailbox {
    sender: mpsc::Sender<Command>,
    failure: Arc<Mutex<Option<String>>>,
}
struct Transport {
    mailbox: Arc<Mailbox>,
    worker: JoinHandle<()>,
}

/// A live runtime confined to its own worker thread. Calls from any thread
/// serialize and wait; disposal waits for an in-flight call, then drops all
/// plans, continuations and state on that worker. No host callbacks run here.
///
/// A marker is a capability to write an expression of state, statements and
/// query data into a state variable. It runs no host action; writes are bounded.
/// Marker expressions can mint declared custom steps in state. Accept marker
/// JSON only from the evaluated tree, never directly from model/tool data.
/// Suppressing clicks while
/// a bundled response is blocked is the embedder's responsibility.
#[derive(uniffi::Object)]
pub struct RuntimeSession {
    transport: Mutex<Option<Transport>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl RuntimeSession {
    fn dispatch<T>(
        &self,
        command: impl FnOnce(mpsc::SyncSender<Result<T, OpenUiError>>) -> Command,
    ) -> Result<T, OpenUiError> {
        let transport = self
            .transport
            .lock()
            .map_err(|_| record_failure(&self.failure, "session lock poisoned"))?;
        if let Some(reason) = failure_reason(&self.failure) {
            return Err(worker_error(&reason));
        }
        let transport = transport.as_ref().ok_or(OpenUiError::Disposed)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        transport
            .mailbox
            .sender
            .send(command(sender))
            .map_err(|_| record_failure(&self.failure, "runtime worker stopped unexpectedly"))?;
        receiver
            .recv()
            .map_err(|_| record_failure(&self.failure, "runtime worker stopped unexpectedly"))?
    }
    fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Worker, &PreparedReferenceValues<'_>) -> Result<T, OpenUiError>
        + Send
        + 'static,
    ) -> Result<T, OpenUiError> {
        let failure = self.failure.clone();
        self.dispatch(move |sender| {
            Command::Call(Box::new(move |state, refs| {
                let result = catch_unwind(AssertUnwindSafe(|| operation(state, refs)));
                match result {
                    Ok(result) => {
                        let _ = sender.send(result);
                        true
                    }
                    Err(_) => {
                        let error = record_failure(&failure, "runtime panic; session revoked");
                        let _ = sender.send(Err(error));
                        false
                    }
                }
            }))
        })
    }
    fn close(&self) {
        let mut guard = match self.transport.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(transport) = guard.take() {
            let _ = transport.mailbox.sender.send(Command::Stop);
            let _ = transport.worker.join();
        }
    }
}
impl Drop for RuntimeSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[uniffi::export]
impl RuntimeSession {
    /// Create an empty session. `prompt_spec` is either its library object
    /// (`components` signatures) or the full `{library,promptOptions}` spec.
    /// Actions always come from the ComponentLibrary's original schema.
    #[uniffi::constructor]
    pub fn new(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
    ) -> Result<Arc<Self>, OpenUiError> {
        Self::with_limits(library, prompt_spec, RuntimeLimits::default())
    }

    /// Create a session with fixed bounds for its entire lifetime.
    #[uniffi::constructor]
    pub fn with_limits(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
        limits: RuntimeLimits,
    ) -> Result<Arc<Self>, OpenUiError> {
        let (sender, receiver) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let mailbox = Arc::new(Mailbox {
            sender,
            failure: failure.clone(),
        });
        let worker_mailbox = Arc::downgrade(&mailbox);
        let worker_failure = failure.clone();
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("openui-runtime".into())
            .spawn(move || {
                // Construction, operations and destruction of every thread-affine
                // runtime value happen inside this closure.
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let state = Worker::new(library, prompt_spec, limits, worker_mailbox);
                    match state {
                        Ok(mut state) => {
                            let _ = ready.send(Ok(()));
                            worker_loop(&mut state, receiver);
                        }
                        Err(error) => {
                            let _ = ready.send(Err(error));
                        }
                    }
                }));
                if result.is_err() {
                    let _ = record_failure(&worker_failure, "runtime worker panicked");
                }
            })
            .map_err(|e| worker_error(&e.to_string()))?;
        match initialized.recv() {
            Ok(Ok(())) => Ok(Arc::new(Self {
                transport: Mutex::new(Some(Transport { mailbox, worker })),
                failure,
            })),
            result => {
                let _ = worker.join();
                Err(match result {
                    Ok(Err(e)) => e,
                    _ => worker_error("worker initialization panicked"),
                })
            }
        }
    }

    /// Replace the current program by sharing its Arc. No JSON is parsed or
    /// program cloned. Old action handles become stale; continuations survive
    /// and resume against the new program, as the runtime specifies.
    pub fn set_program(&self, program: Arc<ParsedProgram>) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            if program.json_bytes > s.limits.max_json_bytes {
                return Err(limit("program JSON byte limit"));
            }
            s.release_evaluation()?;
            s.program = program.inner.clone();
            Ok(())
        })
    }

    /// Report prompt annotations needing an explicit choice by the embedder.
    pub fn undecided_bindings(&self) -> Result<Vec<BindingPair>, OpenUiError> {
        self.call(|s, _| {
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
    pub fn mark_binding(&self, component: String, prop: String) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
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
                s.bindings.insert(key.0.clone(), key.1.clone());
                s.explicit_bindings.insert(key);
                s.binding_bytes = total;
            }
            Ok(())
        })
    }

    /// Apply current declarations and optional persisted object. Omitted JSON
    /// means no persistence. JSON null is a defined value; absent keys are not.
    pub fn initialize(
        &self,
        persisted: Option<String>,
    ) -> Result<InitializationOutput, OpenUiError> {
        self.call(move |s, _| {
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

    /// Replace query/mutation results with a JSON object. Parsing and bounded
    /// preparation happen once, reused by draws, binding writes, runs and resumes.
    /// Suspended runs do not retain this reference supply.
    pub fn set_reference_values(&self, json: String) -> Result<(), OpenUiError> {
        self.dispatch(move |reply| Command::References(json, reply))
    }

    /// Draw explicitly. Keep the runtime's evaluated root without indexing it;
    /// action lookup walks a root-relative JSON path lazily. Only runtime limits
    /// cut a draw. A successful draw releases the preceding generation.
    pub fn evaluate(&self) -> Result<EvaluationOutput, OpenUiError> {
        self.call(|s, refs| {
            let tree =
                openui_runtime::evaluate_tree_with_limits(s.input(refs), &s.store, s.core_limits);
            let json = tree.to_json();
            s.release_evaluation()?;
            s.root = tree.root;
            Ok(EvaluationOutput {
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
        &self,
        evaluation: Arc<EvaluationHandle>,
        path: Vec<String>,
    ) -> Result<Arc<ActionHandle>, OpenUiError> {
        self.call(move |s, _| {
            s.check_evaluation(&evaluation)?;
            let mut bytes = 0u64;
            for key in &path {
                bytes = bytes.saturating_add(key.len() as u64).saturating_add(1);
                if bytes > s.limits.max_json_bytes {
                    return Err(limit("action path byte limit"));
                }
            }
            s.action_value(&path)?;
            Ok(Arc::new(ActionHandle { evaluation, path }))
        })
    }

    /// Undefined is None; JSON null is Some("null"). State names include `$`.
    pub fn get(&self, name: String) -> Result<Option<String>, OpenUiError> {
        self.call(move |s, _| {
            s.text(&name)?;
            Ok(s.store.get(&name).as_ref().map(value_json))
        })
    }
    /// Host JSON never grants custom-step identity. None writes undefined.
    pub fn set(&self, name: String, json: Option<String>) -> Result<bool, OpenUiError> {
        self.call(move |s, _| {
            s.text(&name)?;
            let value = json.map(|j| s.json(&j)).transpose()?;
            s.store
                .set(&name, value)
                .map_err(|e| runtime_error(RuntimeFailure::Store(e), s.core_limits.result_size()))
        })
    }
    /// JSON.stringify-compatible snapshot: undefined entries are omitted.
    pub fn snapshot(&self) -> Result<String, OpenUiError> {
        self.call(|s, _| {
            let mut object = JsObject::new();
            for (name, value) in s.store.snapshot().iter() {
                if let Some(value) = value {
                    object.define(name, value.clone());
                }
            }
            Ok(value_json(&Value::Obj(Arc::new(object))))
        })
    }
    /// Number of mutation outcomes currently retained by this session.
    pub fn pending_continuations(&self) -> Result<u64, OpenUiError> {
        self.call(|s, _| Ok(s.continuations.len() as u64))
    }

    /// Store notification count.
    pub fn version(&self) -> Result<u64, OpenUiError> {
        self.call(|s, _| Ok(s.store.version()))
    }
    /// Whether a later declared default may still replace this name.
    pub fn is_pristine(&self, name: String) -> Result<bool, OpenUiError> {
        self.call(move |s, _| {
            s.text(&name)?;
            Ok(s.store.is_pristine(&name))
        })
    }
    /// A marker can write any state target and call declared custom actions,
    /// minting real steps for a later draw/click. Accept only marker JSON from
    /// the given tree, never model output or tool data. This call runs no host
    /// action; finished plan JSON alone cannot restore custom authority.
    pub fn write_binding(&self, marker: String, json: String) -> Result<bool, OpenUiError> {
        self.call(move |s, refs| {
            let marker = s.json(&marker)?;
            let value = s.json(&json)?;
            openui_runtime::write_binding_with_limits(
                tree_input(&s.program, &s.library, &s.bindings, refs),
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
        &self,
        form: Option<String>,
        name: String,
    ) -> Result<Option<String>, OpenUiError> {
        self.call(move |s, _| {
            s.field_text(form.as_deref(), None, &name)?;
            Ok(
                openui_runtime::get_field_value(&s.store, form.as_deref(), &name)
                    .as_ref()
                    .map(value_json),
            )
        })
    }
    /// Write a renderer field; report the runtime's save request separately.
    pub fn set_field(
        &self,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        save: bool,
    ) -> Result<FieldWriteOutput, OpenUiError> {
        self.call(move |s, _| {
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
        &self,
        form: Option<String>,
        component_type: Option<String>,
        name: String,
        json: Option<String>,
        streaming: bool,
    ) -> Result<Option<FieldWriteOutput>, OpenUiError> {
        self.call(move |s, _| {
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
        &self,
        action: Option<Arc<ActionHandle>>,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(move |s, refs| {
            let value = if let Some(action) = action {
                s.check_evaluation(&action.evaluation)?;
                Some(s.action_value(&action.path)?.clone())
            } else {
                None
            };
            s.run(refs, value.as_ref(), &message, form.as_deref())
        })
    }
    /// Supply a legacy object without `steps`. Plans must use action handles;
    /// legacy names still pass the runtime's custom-action shadowing checks.
    pub fn run_legacy(
        &self,
        json: String,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(move |s, refs| {
            let value = s.json(&json)?;
            if !matches!(&value, Value::Obj(o) if !o.has_own("steps")) {
                return Err(OpenUiError::InvalidJson {
                    reason:
                        "expected a legacy object without steps; plans require an action handle"
                            .into(),
                });
            }
            s.run(refs, Some(&value), &message, form.as_deref())
        })
    }
    /// Consume a mutation outcome once. False ends the run. True uses current
    /// program/state/reference inputs with the original action work allowance.
    pub fn resume(
        &self,
        continuation: Arc<ContinuationHandle>,
        succeeded: bool,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(move |s, refs| {
            if !Arc::ptr_eq(&s.owner, &continuation.owner) {
                return Err(OpenUiError::ForeignHandle);
            }
            if continuation.evicted.load(Ordering::Acquire) {
                return Err(OpenUiError::EvictedHandle);
            }
            let entry = s
                .continuations
                .remove(&continuation.generation)
                .ok_or(OpenUiError::ConsumedHandle)?;
            let run = openui_runtime::resume_action(
                tree_input(&s.program, &s.library, &s.bindings, refs),
                &mut s.store,
                &entry.continuation,
                succeeded,
            )
            .map_err(|e| runtime_error(RuntimeFailure::Resume(e), s.core_limits.result_size()))?;
            s.output(run)
        })
    }
    /// Idempotently revoke all handles and shut down. Normal closure gives
    /// Disposed on later calls; an existing Worker failure stays distinguishable.
    pub fn dispose(&self) {
        self.close();
    }
}

struct Worker {
    library: Arc<ComponentLibrary>,
    program: Arc<ParseResult>,
    bindings: BindingProps,
    explicit_bindings: HashSet<(String, String)>,
    binding_bytes: u64,
    store: Store,
    limits: RuntimeLimits,
    core_limits: Limits,
    reference_limits: Limits,
    owner: Arc<()>,
    generation: u64,
    root: Option<Value>,
    mailbox: Weak<Mailbox>,
    next_continuation: u64,
    continuations: BTreeMap<u64, PendingContinuation>,
}
impl Worker {
    fn new(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
        limits: RuntimeLimits,
        mailbox: Weak<Mailbox>,
    ) -> Result<Self, OpenUiError> {
        if limits.max_continuations == 0 {
            return Err(limit("maxContinuations must be at least one"));
        }
        let core_limits = limits.evaluation.to_core();
        let reference_limits = limits.reference_preparation.to_core();
        let mut s = Self {
            library,
            program: Arc::new(ParseResult::default()),
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
            mailbox,
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
            s.bindings = BindingProps::from_prompt_spec(library);
        }
        Ok(s)
    }
    // Deliberately borrow only the input fields: mutations borrow Store separately.
    fn input<'a>(&'a self, refs: &'a PreparedReferenceValues<'a>) -> TreeInput<'a> {
        tree_input(&self.program, &self.library, &self.bindings, refs)
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
    fn object(&self, text: &str) -> Result<JsObject<Value>, OpenUiError> {
        let value = self.json(text)?;
        match &value {
            Value::Obj(o) => Ok(o.as_ref().clone()),
            _ => Err(OpenUiError::InvalidJson {
                reason: "expected a JSON object".into(),
            }),
        }
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
    fn fail(&self, reason: &str) -> OpenUiError {
        match self.mailbox.upgrade() {
            Some(mailbox) => record_failure(&mailbox.failure, reason),
            None => worker_error(reason),
        }
    }
    fn run(
        &mut self,
        refs: &PreparedReferenceValues<'_>,
        action: Option<&Value>,
        message: &str,
        form: Option<&str>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.field_text(form, None, message)?;
        let input = tree_input(&self.program, &self.library, &self.bindings, refs);
        let run = openui_runtime::run_action_with_limits(
            input,
            &mut self.store,
            action,
            message,
            form,
            self.core_limits,
        );
        self.output(run)
    }
    fn output(&mut self, mut run: openui_runtime::ActionRun) -> Result<ActionOutput, OpenUiError> {
        let continuation = if let Some(continuation) = run.continuation.take() {
            self.next_continuation = self
                .next_continuation
                .checked_add(1)
                .ok_or_else(|| limit("continuation generations exhausted"))?;
            if self.continuations.len() as u64 >= self.limits.max_continuations
                && let Some((_, oldest)) = self.continuations.pop_first()
                && let Some(handle) = oldest.handle.upgrade()
            {
                handle.evicted.store(true, Ordering::Release);
            }
            let handle = Arc::new(ContinuationHandle {
                owner: self.owner.clone(),
                generation: self.next_continuation,
                transport: self.mailbox.clone(),
                evicted: AtomicBool::new(false),
            });
            self.continuations.insert(
                self.next_continuation,
                PendingContinuation {
                    continuation,
                    handle: Arc::downgrade(&handle),
                },
            );
            Some(handle)
        } else {
            None
        };
        Ok(ActionOutput {
            events: run.events.iter().map(|event| event.to_json()).collect(),
            failure: run.failure.map(|f| ActionFailureOutput {
                step: f.step.map(|v| v as u64),
                error: runtime_error(
                    RuntimeFailure::Binding(f.cause),
                    self.core_limits.result_size(),
                ),
            }),
            size_limited: run.size_limited,
            depth_limited: run.depth_limited,
            continuation,
        })
    }
}

fn tree_input<'a>(
    program: &'a Arc<ParseResult>,
    library: &'a ComponentLibrary,
    bindings: &'a BindingProps,
    refs: &'a PreparedReferenceValues<'a>,
) -> TreeInput<'a> {
    let input = TreeInput::from_shared_result(program)
        .with_library(&library.inner)
        .with_binding_props(bindings)
        .with_prepared_reference_values(refs);
    match library.schema.get("actions") {
        Some(actions) => input.with_actions(actions),
        None => input,
    }
}

fn worker_loop(state: &mut Worker, receiver: mpsc::Receiver<Command>) {
    let mut values = JsObject::new();
    let mut acknowledge: Option<mpsc::SyncSender<Result<(), OpenUiError>>> = None;
    loop {
        // Lexical ownership avoids a self-referential struct or unsafe lifetime
        // extension. Preparation is complete before replacement is acknowledged.
        let prepared = catch_unwind(AssertUnwindSafe(|| {
            PreparedReferenceValues::with_limits(&values, state.reference_limits)
        }));
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(_) => {
                let _ = state.fail("reference preparation panicked; session revoked");
                if let Some(reply) = acknowledge.take() {
                    let _ = reply.send(Err(
                        state.fail("reference preparation panicked; session revoked")
                    ));
                }
                return;
            }
        };
        if let Some(reply) = acknowledge.take() {
            let _ = reply.send(Ok(()));
        }
        let replacement = loop {
            match receiver.recv() {
                Ok(Command::Call(job)) => {
                    if !job(state, &prepared) {
                        return;
                    }
                }
                Ok(Command::References(json, reply)) => {
                    match catch_unwind(AssertUnwindSafe(|| state.object(&json))) {
                        Ok(Ok(values)) => {
                            break (values, reply);
                        }
                        Ok(Err(error)) => {
                            let _ = reply.send(Err(error));
                        }
                        Err(_) => {
                            let _ = reply
                                .send(Err(state.fail("reference JSON panicked; session revoked")));
                            return;
                        }
                    }
                }
                Ok(Command::ReleaseContinuation(generation)) => {
                    state.continuations.remove(&generation);
                }
                Ok(Command::Stop) | Err(_) => return,
            }
        };
        drop(prepared);
        values = replacement.0;
        acknowledge = Some(replacement.1);
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
}

fn failure_reason(failure: &Mutex<Option<String>>) -> Option<String> {
    failure.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
fn record_failure(failure: &Mutex<Option<String>>, reason: &str) -> OpenUiError {
    let mut failure = failure.lock().unwrap_or_else(|p| p.into_inner());
    let reason = failure.get_or_insert_with(|| reason.to_owned());
    worker_error(reason)
}

enum RuntimeFailure {
    Store(openui_runtime::StoreError),
    Binding(openui_runtime::BindingWriteError),
    Resume(openui_runtime::ActionResumeError),
}
// One place to translate the runtime's non-exhaustive refusal categories.
fn runtime_error(error: RuntimeFailure, state_cap: usize) -> OpenUiError {
    use openui_runtime::{ActionResumeError as R, BindingWriteError as B};
    match error {
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

// Keep all JSON escaping, number formatting and AST projection in the core.
// Use a fixed core-owned wrapper, as the runtime's own serializer does.
fn value_json(value: &Value) -> String {
    let mut props = JsObject::new();
    props.define("v", value.clone());
    let result = ParseResult {
        root: Some(openui_lang::Element {
            type_name: String::new(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..ParseResult::default()
    };
    let json = result.to_json();
    let prefix = "{\"root\":{\"type\":\"element\",\"typeName\":\"\",\"props\":{\"v\":";
    let suffix = "},\"partial\":false,\"hasDynamicProps\":false},\"meta\":";
    json.strip_prefix(prefix)
        .and_then(|s| s.rfind(suffix).map(|n| s.get(..n).unwrap_or("null")))
        .unwrap_or("null")
        .into()
}

struct Json(serde_json::Value);
impl Drop for Json {
    fn drop(&mut self) {
        openui_lang::free_json(std::mem::take(&mut self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_handoff_keeps_the_original_shared_parse_result() {
        let library = ComponentLibrary::new(r#"{"$defs":{}}"#.into()).unwrap();
        let session = RuntimeSession::new(library.clone(), None).unwrap();
        let parsed = crate::parse("$n=1".into(), library, None).program;
        session.set_program(parsed.clone()).unwrap();
        assert!(
            session
                .call(move |s, _| Ok(Arc::ptr_eq(&s.program, &parsed.inner)))
                .unwrap()
        );
    }

    #[test]
    fn a_worker_panic_returns_a_typed_error_and_revokes_the_session() {
        let library = ComponentLibrary::new(r#"{"$defs":{}}"#.into()).unwrap();
        let session = RuntimeSession::new(library, None).unwrap();
        let result: Result<(), OpenUiError> =
            session.call(|_, _| panic!("injected worker failure"));
        let Err(OpenUiError::Worker { reason }) = result else {
            panic!("worker failure was not typed")
        };
        for error in [
            session.version().unwrap_err(),
            session.evaluate().unwrap_err(),
            session.set_reference_values("{}".into()).unwrap_err(),
        ] {
            assert!(matches!(error, OpenUiError::Worker { reason: later } if later == reason));
        }
        session.dispose();
        assert!(matches!(session.version(), Err(OpenUiError::Worker { .. })));
    }

    #[test]
    fn disposal_joins_an_in_flight_call_and_all_calls_use_one_thread() {
        let library = ComponentLibrary::new(r#"{"$defs":{}}"#.into()).unwrap();
        let session = RuntimeSession::new(library, None).unwrap();
        let owner = session
            .call(|_, _| Ok(std::thread::current().id()))
            .unwrap();
        assert_ne!(owner, std::thread::current().id());
        let (started, observed) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let caller = session.clone();
        let thread = std::thread::spawn(move || {
            caller.call(move |_, _| {
                assert_eq!(owner, std::thread::current().id());
                started.send(()).unwrap();
                released.recv().unwrap();
                Ok(())
            })
        });
        observed.recv().unwrap();
        let disposer = session.clone();
        let (enter, entered) = mpsc::channel();
        let disposal = std::thread::spawn(move || {
            enter.send(()).unwrap();
            disposer.dispose();
        });
        entered.recv().unwrap();
        release.send(()).unwrap();
        thread.join().unwrap().unwrap();
        disposal.join().unwrap();
        assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
    }
}
