//! Runtime adaptation lives here so changes to the runtime's safety API have
//! one binding seam. No evaluator, action validation or authority rules live here.

use crate::{ComponentLibrary, OpenUiError, ParseLimits, ParsedProgram};
use openui_lang::JsObject;
use openui_runtime::PreparedReferenceValues;
use openui_session::SessionCore as Worker;
use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::{JoinHandle, ThreadId};

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
    /// Optional smaller retained action-path bound; absent preserves max_json_bytes.
    #[uniffi(default = None)]
    pub max_action_path_bytes: Option<u64>,
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
    inner: Arc<openui_session::EvaluationHandle>,
}
#[derive(Debug, uniffi::Object)]
pub struct ActionHandle {
    inner: Arc<openui_session::ActionHandle>,
}
#[derive(Debug, uniffi::Object)]
pub struct ContinuationHandle {
    inner: Arc<openui_session::ContinuationHandle>,
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

/// A thrown host-function message, without a binding-specific prefix.
#[derive(Debug, uniffi::Error)]
pub enum FunctionError {
    /// The runtime emits `@Name threw: reason` and null, just as in Rust.
    Thrown { reason: String },
}
impl std::fmt::Display for FunctionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Thrown { reason } => f.write_str(reason),
        }
    }
}
impl std::error::Error for FunctionError {}
impl From<uniffi::UnexpectedUniFFICallbackError> for FunctionError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Thrown {
            reason: error.reason,
        }
    }
}
/// Implement on the host. JSON null is Some("null"); None is undefined.
/// Called synchronously on the session worker, without a time bound. Calls back
/// into this session on that thread refuse; other threads queue normally.
#[uniffi::export(callback_interface)]
pub trait LibraryFunction: Send + Sync {
    fn call(&self, arguments_json: String) -> Result<Option<String>, FunctionError>;
}
// A worker must never queue a request to itself. Foreign destructors can also
// run on callers whose registrations are refused; fence only their own thread.
// No shared flag is saved/restored across independent callback lifetimes.
// A no-Drop TLS key stays usable in other TLS destructors as well. Its Vec is
// emptied and freed explicitly when the last scope leaves; nothing is leaked.
thread_local! {
    static HOST_SCOPES: RefCell<ManuallyDrop<Vec<Arc<()>>>> =
        const { RefCell::new(ManuallyDrop::new(Vec::new())) };
}
struct ReentryFence {
    owner: Arc<()>,
    worker: ThreadId,
}
impl ReentryFence {
    fn is_reentrant(&self) -> bool {
        std::thread::current().id() == self.worker
            || HOST_SCOPES.with(|scopes| {
                scopes
                    .borrow()
                    .iter()
                    .any(|owner| Arc::ptr_eq(owner, &self.owner))
            })
    }
    fn enter(&self) -> HostScope {
        let active = std::thread::current().id() != self.worker;
        if active {
            HOST_SCOPES.with(|scopes| scopes.borrow_mut().push(self.owner.clone()));
        }
        HostScope {
            active,
            _thread: std::marker::PhantomData,
        }
    }
}
struct HostScope {
    active: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for HostScope {
    fn drop(&mut self) {
        if self.active {
            HOST_SCOPES.with(|scopes| {
                let mut scopes = scopes.borrow_mut();
                scopes.pop();
                if scopes.is_empty() {
                    // ManuallyDrop makes the TLS key immortal, not its heap.
                    drop(std::mem::take(&mut **scopes));
                }
            });
        }
    }
}
// Destroy host implementations outside transport locks, even on refused calls.
struct CallbackHolder {
    callback: Option<Box<dyn LibraryFunction>>,
    fence: Arc<ReentryFence>,
}
impl CallbackHolder {
    fn call(&self, json: String) -> Result<Option<String>, FunctionError> {
        let _scope = self.fence.enter();
        match &self.callback {
            Some(callback) => callback.call(json),
            None => Err(FunctionError::Thrown {
                reason: "host function released".into(),
            }),
        }
    }
}
impl Drop for CallbackHolder {
    fn drop(&mut self) {
        let _scope = self.fence.enter();
        let _ = catch_unwind(AssertUnwindSafe(|| drop(self.callback.take())));
    }
}

type Job = Box<dyn FnOnce(&mut Worker, &PreparedReferenceValues<'_>) -> bool + Send>;
enum Command {
    Call(Job),
    References(String, mpsc::SyncSender<Result<(), OpenUiError>>),
    DrainReleases,
    Stop,
}
#[derive(Debug)]
struct Mailbox {
    sender: mpsc::Sender<Command>,
}
#[derive(Default)]
struct TransportSlot {
    running: Option<Transport>,
    closing: bool,
}
struct Transport {
    mailbox: Arc<Mailbox>,
    worker: JoinHandle<()>,
}

/// A live runtime confined to its own worker thread. Calls from any thread
/// serialize on the worker; disposal waits for an in-flight call, then drops all
/// plans, continuations and state on that worker. Calls during host callbacks
/// refuse before queueing on the worker or inside a host destructor. Other
/// threads queue normally, including during callbacks.
///
/// A marker is a capability to write an expression of state, statements and
/// query data into a state variable. It runs no host action; writes are bounded.
/// Marker expressions can mint declared custom steps in state. Accept marker
/// JSON only from the evaluated tree, never directly from model/tool data.
/// Suppressing clicks while
/// a bundled response is blocked is the embedder's responsibility.
#[derive(uniffi::Object)]
pub struct RuntimeSession {
    transport: Mutex<TransportSlot>,
    closed: Condvar,
    failure: Arc<Mutex<Option<String>>>,
    fence: Arc<ReentryFence>,
}

impl RuntimeSession {
    fn dispatch<T>(
        &self,
        command: impl FnOnce(mpsc::SyncSender<Result<T, OpenUiError>>) -> Command,
    ) -> Result<T, OpenUiError> {
        if self.fence.is_reentrant() {
            return Err(worker_error("session already in use"));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let command = command(sender);
        let transport = self
            .transport
            .lock()
            .map_err(|_| record_failure(&self.failure, "session lock poisoned"))?;
        if let Some(reason) = failure_reason(&self.failure) {
            return Err(worker_error(&reason));
        }
        let transport_ref = transport.running.as_ref().ok_or(OpenUiError::Disposed)?;
        let sent = transport_ref.mailbox.sender.send(command);
        drop(transport);
        // A rejected job can own a foreign callback; destroy it outside the lock.
        sent.map_err(|_| record_failure(&self.failure, "runtime worker stopped unexpectedly"))?;
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
                let result = catch_unwind(AssertUnwindSafe(|| {
                    state.drain_releases();
                    operation(state, refs)
                }));
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
        // Disposal on the worker/in a destructor cannot wait on itself. Other
        // threads enqueue Stop and wait for callbacks and destruction to finish.
        if self.fence.is_reentrant() {
            return;
        }
        let mut guard = match self.transport.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        while guard.closing {
            guard = self.closed.wait(guard).unwrap_or_else(|p| p.into_inner());
        }
        let transport = guard.running.take();
        guard.closing = transport.is_some();
        drop(guard);
        if let Some(transport) = transport {
            let _ = transport.mailbox.sender.send(Command::Stop);
            let _ = transport.worker.join();
            let mut guard = self.transport.lock().unwrap_or_else(|p| p.into_inner());
            guard.closing = false;
            self.closed.notify_all();
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
        let mailbox = Arc::new(Mailbox { sender });

        let worker_mailbox = Arc::downgrade(&mailbox);
        let worker_failure = failure.clone();
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("openui-runtime".into())
            .spawn(move || {
                // Construction, operations and destruction of every thread-affine
                // runtime value happen inside this closure.
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let state = Worker::new(
                        Arc::new(openui_session::ComponentLibrary {
                            inner: library.inner.clone(),
                            schema: library.schema.clone(),
                        }),
                        prompt_spec,
                        limits.into(),
                    );
                    match state {
                        Ok(mut state) => {
                            state.set_release_notifier(openui_session::ReleaseNotifier(Arc::new(
                                move || {
                                    if let Some(mailbox) = worker_mailbox.upgrade() {
                                        let _ = mailbox.sender.send(Command::DrainReleases);
                                    }
                                },
                            )));
                            let _ = ready.send(Ok(()));
                            worker_loop(&mut state, receiver, &worker_failure);
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
            Ok(Ok(())) => {
                let fence = Arc::new(ReentryFence {
                    owner: Arc::new(()),
                    worker: worker.thread().id(),
                });
                Ok(Arc::new(Self {
                    transport: Mutex::new(TransportSlot {
                        running: Some(Transport { mailbox, worker }),
                        closing: false,
                    }),
                    closed: Condvar::new(),
                    failure,
                    fence,
                }))
            }
            result => {
                let _ = worker.join();
                Err(match result {
                    Ok(Err(e)) => e.into(),
                    _ => worker_error("worker initialization panicked"),
                })
            }
        }
    }

    /// Register or replace a declared synchronous JSON implementation. Calls
    /// execute on the session worker without a time bound. A host callback must
    /// not wait for another thread to call this same session: that call queues
    /// behind the callback, so the host would deadlock itself.
    pub fn register_function(
        &self,
        name: String,
        callback: Box<dyn LibraryFunction>,
    ) -> Result<(), OpenUiError> {
        let callback = CallbackHolder {
            callback: Some(callback),
            fence: self.fence.clone(),
        };
        self.call(move |s, _| {
            s.register_function(
                name,
                Arc::new(move |json| {
                    catch_unwind(AssertUnwindSafe(|| callback.call(json)))
                        .map_err(|_| "host function panicked".to_owned())
                        .and_then(|result| result.map_err(|e| e.to_string()))
                }),
            )
            .map_err(Into::into)
        })
    }

    /// Replace the current program by sharing its Arc. No JSON is parsed or
    /// program cloned. Old action handles become stale; continuations survive
    /// and resume against the new program, as the runtime specifies.
    pub fn set_program(&self, program: Arc<ParsedProgram>) -> Result<(), OpenUiError> {
        self.call(move |s, refs| {
            s.set_program(
                refs,
                Arc::new(openui_session::ParsedProgram {
                    inner: program.inner.clone(),
                    json_bytes: program.json_bytes,
                }),
            )
            .map_err(Into::into)
        })
    }

    /// Report prompt annotations needing an explicit choice by the embedder.
    pub fn undecided_bindings(&self) -> Result<Vec<BindingPair>, OpenUiError> {
        self.call(move |s, refs| {
            s.undecided_bindings(refs)
                .map(|v| v.into_iter().map(Into::into).collect())
                .map_err(Into::into)
        })
    }

    /// Resolve a binding annotation explicitly; affects the next evaluation.
    pub fn mark_binding(&self, component: String, prop: String) -> Result<(), OpenUiError> {
        self.call(move |s, refs| s.mark_binding(refs, component, prop).map_err(Into::into))
    }

    /// Apply current declarations and optional persisted object. Omitted JSON
    /// means no persistence. JSON null is a defined value; absent keys are not.
    pub fn initialize(
        &self,
        persisted: Option<String>,
    ) -> Result<InitializationOutput, OpenUiError> {
        self.call(move |s, refs| {
            s.initialize(refs, persisted)
                .map(|v| v.into())
                .map_err(Into::into)
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
        self.call(move |s, refs| s.evaluate(refs).map(|v| v.into()).map_err(Into::into))
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
        self.call(move |s, refs| {
            s.action_at(refs, evaluation.inner.clone(), path)
                .map(|inner| Arc::new(ActionHandle { inner }))
                .map_err(Into::into)
        })
    }

    /// Undefined is None; JSON null is Some("null"). State names include `$`.
    pub fn get(&self, name: String) -> Result<Option<String>, OpenUiError> {
        self.call(move |s, refs| s.get(refs, name).map_err(Into::into))
    }
    /// Host JSON never grants custom-step identity. None writes undefined.
    pub fn set(&self, name: String, json: Option<String>) -> Result<bool, OpenUiError> {
        self.call(move |s, refs| s.set(refs, name, json).map_err(Into::into))
    }
    /// JSON.stringify-compatible snapshot: undefined entries are omitted.
    pub fn snapshot(&self) -> Result<String, OpenUiError> {
        self.call(move |s, refs| s.snapshot(refs).map_err(Into::into))
    }
    /// Number of mutation outcomes currently retained by this session.
    pub fn pending_continuations(&self) -> Result<u64, OpenUiError> {
        self.call(move |s, refs| s.pending_continuations(refs).map_err(Into::into))
    }

    /// Store notification count.
    pub fn version(&self) -> Result<u64, OpenUiError> {
        self.call(move |s, refs| s.version(refs).map_err(Into::into))
    }
    /// Whether a later declared default may still replace this name.
    pub fn is_pristine(&self, name: String) -> Result<bool, OpenUiError> {
        self.call(move |s, refs| s.is_pristine(refs, name).map_err(Into::into))
    }
    /// A marker can write any state target and call declared custom actions,
    /// minting real steps for a later draw/click. Accept only marker JSON from
    /// the given tree, never model output or tool data. This call runs no host
    /// action; finished plan JSON alone cannot restore custom authority.
    pub fn write_binding(&self, marker: String, json: String) -> Result<bool, OpenUiError> {
        self.call(move |s, refs| s.write_binding(refs, marker, json).map_err(Into::into))
    }
    /// Read an unbound renderer form field, including wrapped values.
    pub fn get_field(
        &self,
        form: Option<String>,
        name: String,
    ) -> Result<Option<String>, OpenUiError> {
        self.call(move |s, refs| s.get_field(refs, form, name).map_err(Into::into))
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
        self.call(move |s, refs| {
            s.set_field(refs, form, component_type, name, json, save)
                .map(|v| v.into())
                .map_err(Into::into)
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
        self.call(move |s, refs| {
            s.initialize_field(refs, form, component_type, name, json, streaming)
                .map(|v| v.map(Into::into))
                .map_err(Into::into)
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
            s.run(refs, action.map(|a| a.inner.clone()), message, form)
                .map(|v| v.into())
                .map_err(Into::into)
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
            s.run_legacy(refs, json, message, form)
                .map(|v| v.into())
                .map_err(Into::into)
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
            s.resume(refs, continuation.inner.clone(), succeeded)
                .map(|v| v.into())
                .map_err(Into::into)
        })
    }
    /// Idempotently revoke all handles and shut down. Normal closure gives
    /// Disposed on later calls; an existing Worker failure stays distinguishable.
    pub fn dispose(&self) {
        self.close();
    }
}

fn worker_loop(
    state: &mut Worker,
    receiver: mpsc::Receiver<Command>,
    failure: &Mutex<Option<String>>,
) {
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
                let _ = record_failure(failure, "reference preparation panicked; session revoked");
                if let Some(reply) = acknowledge.take() {
                    let _ = reply.send(Err(record_failure(
                        failure,
                        "reference preparation panicked; session revoked",
                    )));
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
                    match catch_unwind(AssertUnwindSafe(|| {
                        state.object(&json).map_err(OpenUiError::from)
                    })) {
                        Ok(Ok(values)) => {
                            break (values, reply);
                        }
                        Ok(Err(error)) => {
                            let _ = reply.send(Err(error));
                        }
                        Err(_) => {
                            let _ = reply.send(Err(record_failure(
                                failure,
                                "reference JSON panicked; session revoked",
                            )));
                            return;
                        }
                    }
                }
                Ok(Command::DrainReleases) => {
                    state.drain_releases();
                }
                Ok(Command::Stop) | Err(_) => return,
            }
        };
        drop(prepared);
        values = replacement.0;
        acknowledge = Some(replacement.1);
    }
}

fn failure_reason(failure: &Mutex<Option<String>>) -> Option<String> {
    failure.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
fn record_failure(failure: &Mutex<Option<String>>, reason: &str) -> OpenUiError {
    let mut failure = failure.lock().unwrap_or_else(|p| p.into_inner());
    let reason = failure.get_or_insert_with(|| reason.to_owned());
    worker_error(reason)
}

fn worker_error(reason: &str) -> OpenUiError {
    OpenUiError::Worker {
        reason: reason.into(),
    }
}

// Keep all JSON escaping, number formatting and AST projection in the core.
// Use a fixed core-owned wrapper, as the runtime's own serializer does.
impl From<openui_session::BindingPair> for BindingPair {
    fn from(v: openui_session::BindingPair) -> Self {
        Self {
            component: v.component,
            prop: v.prop,
        }
    }
}
impl From<openui_session::InitializationOutput> for InitializationOutput {
    fn from(v: openui_session::InitializationOutput) -> Self {
        Self {
            changed: v.changed,
            refused: v.refused.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<openui_session::StateRefusal> for StateRefusal {
    fn from(v: openui_session::StateRefusal) -> Self {
        Self {
            name: v.name,
            error: v.error.into(),
        }
    }
}
impl From<openui_session::FieldWriteOutput> for FieldWriteOutput {
    fn from(v: openui_session::FieldWriteOutput) -> Self {
        Self {
            changed: v.changed,
            save_requested: v.save_requested,
        }
    }
}
impl From<openui_session::EvaluationOutput> for EvaluationOutput {
    fn from(v: openui_session::EvaluationOutput) -> Self {
        Self {
            json: v.json,
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            evaluation: Arc::new(EvaluationHandle {
                inner: v.evaluation,
            }),
        }
    }
}
impl From<openui_session::ActionOutput> for ActionOutput {
    fn from(v: openui_session::ActionOutput) -> Self {
        Self {
            events: v.events,
            failure: v.failure.map(Into::into),
            size_limited: v.size_limited,
            depth_limited: v.depth_limited,
            continuation: v
                .continuation
                .map(|inner| Arc::new(ContinuationHandle { inner })),
        }
    }
}
impl From<openui_session::ActionFailureOutput> for ActionFailureOutput {
    fn from(v: openui_session::ActionFailureOutput) -> Self {
        Self {
            step: v.step,
            error: v.error.into(),
        }
    }
}
impl From<RuntimeLimits> for openui_session::RuntimeLimits {
    fn from(v: RuntimeLimits) -> Self {
        Self {
            evaluation: v.evaluation.into(),
            reference_preparation: v.reference_preparation.into(),
            max_json_bytes: v.max_json_bytes,
            max_action_path_bytes: v.max_action_path_bytes,
            max_continuations: v.max_continuations,
        }
    }
}
impl From<crate::ParseLimits> for openui_session::ParseLimits {
    fn from(v: crate::ParseLimits) -> Self {
        Self {
            program_size: v.program_size,
            result_size: v.result_size,
            result_floor: v.result_floor,
            amplification: v.amplification,
            result_ceiling: v.result_ceiling,
            depth: v.depth,
            work: v.work,
        }
    }
}
impl From<openui_session::OpenUiError> for OpenUiError {
    fn from(v: openui_session::OpenUiError) -> Self {
        match v {
            openui_session::OpenUiError::Disposed => Self::Disposed,
            openui_session::OpenUiError::ForeignHandle => Self::ForeignHandle,
            openui_session::OpenUiError::StaleHandle => Self::StaleHandle,
            openui_session::OpenUiError::ConsumedHandle => Self::ConsumedHandle,
            openui_session::OpenUiError::EvictedHandle => Self::EvictedHandle,
            openui_session::OpenUiError::InvalidHandle => Self::InvalidHandle,
            openui_session::OpenUiError::InvalidMarker => Self::InvalidMarker,
            openui_session::OpenUiError::InvalidJson { reason } => Self::InvalidJson { reason },
            openui_session::OpenUiError::RefusedWrite { reason } => Self::RefusedWrite { reason },
            openui_session::OpenUiError::Evaluation { reason } => Self::Evaluation { reason },
            openui_session::OpenUiError::LimitExceeded { reason } => Self::LimitExceeded { reason },
            openui_session::OpenUiError::Worker { reason } => Self::Worker { reason },
            openui_session::OpenUiError::InvalidSchema { reason } => Self::InvalidSchema { reason },
            openui_session::OpenUiError::InvalidPromptSpec { reason } => {
                Self::InvalidPromptSpec { reason }
            }
            openui_session::OpenUiError::Prompt { reason } => Self::Prompt { reason },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

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
