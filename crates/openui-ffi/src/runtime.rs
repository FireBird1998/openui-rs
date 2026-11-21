//! Runtime adaptation lives here so changes to the runtime's safety API have
//! one binding seam. No evaluator, action validation or authority rules live here.

use crate::{
    ComponentLibrary, OpenUiError, ParseLimits, ParsedProgram, QueryEffects, ResponseState,
    WantedToolCall,
};
use openui_lang::JsObject;
use openui_runtime::PreparedReferenceValues;
use openui_session::SessionCore as Worker;
use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// A peak_memory setting here refuses session construction with LimitExceeded.
    pub reference_preparation: ParseLimits,
    /// Byte bound for host text/JSON, program JSON, action paths and explicit
    /// binding annotations (default 8 MiB). Draws use only runtime limits.
    pub max_json_bytes: u64,
    /// Maximum simultaneous suspended runs (default 64, minimum 1). A new
    /// host handoff evicts the oldest pending run when full. A managed submission
    /// at capacity returns a size cut at its Run step.
    pub max_continuations: u64,
    /// Optional smaller retained action-path bound; absent preserves max_json_bytes.
    #[uniffi(default = None)]
    pub max_action_path_bytes: Option<u64>,
    /// Optional query cache byte cap over held tool result data, as JSON;
    /// absent uses the evaluation result-size ceiling. A result that would
    /// exceed it is delivered as a `tool-error`; nothing held is evicted.
    #[uniffi(default = None)]
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
    #[uniffi(default = None)]
    pub query_snapshot_json: Option<String>,
    #[uniffi(default = None)]
    pub query_evaluation_error: Option<OpenUiError>,
    #[uniffi(default = false)]
    pub is_query_loading: bool,
    #[uniffi(default = None)]
    pub parser_error_json: Option<String>,
    /// The renderer's `blocked`: the response this draw was given has not
    /// settled. Upstream's components draw disabled while it is true; the
    /// session's own click and Retry gate does not read it.
    #[uniffi(default = false)]
    pub blocked: bool,
}

/// Store initialization's batch result, including per-name refusals.
#[derive(Debug, uniffi::Record)]
pub struct InitializationOutput {
    /// At least one value changed.
    pub changed: bool,
    /// Refused entries, in the store's encounter order.
    pub refused: Vec<StateRefusal>,
}

/// A committed program replacement and its complete display data.
#[derive(Debug, uniffi::Record)]
pub struct ReplacementOutput {
    pub initialization: InitializationOutput,
    pub projection: InitializationOutput,
    pub evaluation: EvaluationOutput,
    pub snapshot: String,
    pub version: u64,
}
impl From<openui_session::ReplacementOutput> for ReplacementOutput {
    fn from(v: openui_session::ReplacementOutput) -> Self {
        Self {
            initialization: v.initialization.into(),
            projection: v.projection.into(),
            evaluation: v.evaluation.into(),
            snapshot: v.snapshot,
            version: v.version,
        }
    }
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
/// Called synchronously on the session callback thread, without a time bound.
/// Calls back into an active ancestor session refuse; other threads queue normally.
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
        HostScope::enter(vec![self.owner.clone()])
    }
}
fn host_scopes() -> Vec<Arc<()>> {
    HOST_SCOPES.with(|scopes| scopes.borrow().to_vec())
}
struct HostScope {
    previous: usize,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl HostScope {
    fn enter(owners: Vec<Arc<()>>) -> Self {
        let previous = HOST_SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let previous = scopes.len();
            scopes.extend(owners);
            previous
        });
        Self {
            previous,
            _thread: std::marker::PhantomData,
        }
    }
}
impl Drop for HostScope {
    fn drop(&mut self) {
        HOST_SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            scopes.truncate(self.previous);
            if scopes.is_empty() {
                // ManuallyDrop makes the TLS key immortal, not its heap.
                drop(std::mem::take(&mut **scopes));
            }
        });
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

// Only this thread owns accepted foreign implementations. Worker-side closures
// contain private keys and a sender, so even their final drop on a grown segment
// can only send a release request. No foreign value travels back to the worker.
struct CallbackMailbox {
    sender: mpsc::Sender<CallbackCommand>,
    failure: Arc<Mutex<Option<String>>>,
    #[cfg(test)]
    control: Arc<tests::CallbackControl>,
}
struct CallbackCommand {
    scopes: Vec<Arc<()>>,
    operation: CallbackOperation,
    #[cfg(test)]
    origin_on_stack: Option<bool>,
}
enum CallbackOperation {
    Register(
        Arc<()>,
        PendingCallback,
        mpsc::SyncSender<Result<(), OpenUiError>>,
    ),
    Invoke(
        Arc<()>,
        String,
        mpsc::SyncSender<Result<Option<String>, String>>,
    ),
    Release(Arc<()>, mpsc::SyncSender<()>),
    #[cfg(test)]
    Stop,
}
// The registering caller retains this slot until its synchronous call returns.
// Neither a failed send nor destruction of an unexecuted worker job can release
// foreign code: only the callback thread takes ownership after admission.
type PendingCallback = Arc<Mutex<Option<CallbackHolder>>>;
struct CallerCallback {
    pending: PendingCallback,
}
impl Drop for CallerCallback {
    fn drop(&mut self) {
        // This guard stays on the registering caller even if submission unwinds.
        // No foreign destructor runs while the handoff lock is held.
        let refused = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        drop(refused);
    }
}
impl CallbackMailbox {
    fn stopped(&self) -> OpenUiError {
        record_failure(&self.failure, "callback thread stopped unexpectedly")
    }
    fn send(&self, operation: CallbackOperation) -> Result<(), OpenUiError> {
        #[cfg(test)]
        if self.control.fail_send(&operation) {
            return Err(self.stopped());
        }
        self.sender
            .send(CallbackCommand {
                scopes: host_scopes(),
                operation,
                #[cfg(test)]
                origin_on_stack: tests::on_original_stack(),
            })
            .map_err(|_| self.stopped())
    }
}
struct CallbackThread {
    mailbox: Arc<CallbackMailbox>,
    thread: JoinHandle<()>,
    closing: Arc<AtomicBool>,
}
struct CallbackExit {
    closing: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}
impl Drop for CallbackExit {
    fn drop(&mut self) {
        if !self.closing.load(Ordering::Acquire) {
            let _ = record_failure(&self.failure, "callback thread stopped unexpectedly");
        }
    }
}
impl CallbackThread {
    fn start(
        mailbox: Arc<CallbackMailbox>,
        receiver: mpsc::Receiver<CallbackCommand>,
    ) -> Result<Self, OpenUiError> {
        #[cfg(test)]
        let fail_spawn = mailbox.control.take(tests::Fault::Spawn);
        let closing = Arc::new(AtomicBool::new(false));
        let callback_closing = closing.clone();
        let callback_failure = mailbox.failure.clone();
        #[cfg(test)]
        let control = mailbox.control.clone();
        let spawn = || {
            std::thread::Builder::new()
                .name("openui-callback".into())
                .spawn(move || {
                    let _exit = CallbackExit {
                        closing: callback_closing,
                        failure: callback_failure,
                    };
                    #[cfg(test)]
                    tests::observe_callback_thread(control.clone());
                    #[cfg(test)]
                    tests::record_stack();
                    // Declared before the registry so an unexpected unwind also releases
                    // implementations under the active command's ancestry.
                    let mut scope = None;
                    let mut functions: Vec<(Arc<()>, CallbackHolder)> = Vec::new();
                    while let Ok(command) = receiver.recv() {
                        drop(scope.take());
                        scope = Some(HostScope::enter(command.scopes));
                        #[cfg(test)]
                        tests::ORIGIN_ON_STACK.set(command.origin_on_stack);
                        match command.operation {
                            CallbackOperation::Register(key, pending, reply) => {
                                #[cfg(test)]
                                if control.take(tests::Fault::RegisterReply) {
                                    return;
                                }
                                let callback =
                                    pending.lock().unwrap_or_else(|p| p.into_inner()).take();
                                let Some(callback) = callback else {
                                    // A caller that unwound before handoff reclaimed it.
                                    // This job failed, but the callback thread is healthy.
                                    let _ = reply.send(Err(worker_error(
                                        "host function released before registration",
                                    )));
                                    continue;
                                };
                                functions.push((key, callback));
                                #[cfg(test)]
                                if control.take(tests::Fault::RegisterAccepted) {
                                    return;
                                }
                                let _ = reply.send(Ok(()));
                            }
                            CallbackOperation::Invoke(key, json, reply) => {
                                #[cfg(test)]
                                if control.take(tests::Fault::InvokeReply) {
                                    return;
                                }
                                let result = match functions
                                    .iter()
                                    .find(|(k, _)| Arc::ptr_eq(k, &key))
                                {
                                    Some((_, callback)) => {
                                        catch_unwind(AssertUnwindSafe(|| callback.call(json)))
                                            .map_err(|_| "host function panicked".to_owned())
                                            .and_then(|result| result.map_err(|e| e.to_string()))
                                    }
                                    None => Err("host function released".into()),
                                };
                                let _ = reply.send(result);
                            }
                            CallbackOperation::Release(key, reply) => {
                                if let Some(index) =
                                    functions.iter().position(|(k, _)| Arc::ptr_eq(k, &key))
                                {
                                    drop(functions.swap_remove(index));
                                }
                                #[cfg(test)]
                                if control.take(tests::Fault::ReleaseReply) {
                                    return;
                                }
                                let _ = reply.send(());
                            }
                            #[cfg(test)]
                            CallbackOperation::Stop => return,
                        }
                    }
                })
        };
        #[cfg(test)]
        let thread = if fail_spawn {
            Err(std::io::Error::other("injected callback spawn failure"))
        } else {
            spawn()
        };
        #[cfg(not(test))]
        let thread = spawn();
        // Fault injection and OS spawn failures use the same refusal path.
        let thread = thread.map_err(|e| worker_error(&e.to_string()))?;
        Ok(Self {
            mailbox,
            thread,
            closing,
        })
    }
    fn register(&self, key: Arc<()>, callback: PendingCallback) -> Result<(), OpenUiError> {
        let (reply, received) = mpsc::sync_channel(1);
        self.mailbox
            .send(CallbackOperation::Register(key, callback, reply))?;
        received
            .recv()
            .map_err(|_| self.mailbox.stopped())?
            // The live core already admitted this proxy. A failed handoff must
            // revoke it, even when the callback thread itself remains healthy.
            .map_err(|error| record_failure(&self.mailbox.failure, &error.to_string()))
    }
    fn join(self) {
        // The worker and all its proxies have gone before the final sender.
        self.closing.store(true, Ordering::Release);
        drop(self.mailbox);
        let _ = self.thread.join();
    }
}
struct CallbackProxy {
    key: Arc<()>,
    mailbox: Arc<CallbackMailbox>,
    installed: AtomicBool,
}
impl CallbackProxy {
    fn call(&self, json: String) -> Result<Option<String>, String> {
        let (reply, received) = mpsc::sync_channel(1);
        self.mailbox
            .send(CallbackOperation::Invoke(self.key.clone(), json, reply))
            .map_err(|e| e.to_string())?;
        received
            .recv()
            .map_err(|_| self.mailbox.stopped().to_string())?
    }
}
impl Drop for CallbackProxy {
    fn drop(&mut self) {
        if !self.installed.load(Ordering::Acquire) {
            return;
        }
        let (reply, received) = mpsc::sync_channel(1);
        if self
            .mailbox
            .send(CallbackOperation::Release(self.key.clone(), reply))
            .is_ok()
            && received.recv().is_err()
        {
            let _ = self.mailbox.stopped();
        }
    }
}

type Job =
    Box<dyn FnOnce(&mut Worker, &PreparedReferenceValues<'_>, &mut CallbackState) -> bool + Send>;
enum Command {
    Call(Job),
    References(String, mpsc::SyncSender<Result<(), OpenUiError>>),
    DrainReleases,
    Stop,
}
struct ScopedCommand {
    scopes: Vec<Arc<()>>,
    command: Command,
}
impl ScopedCommand {
    fn new(command: Command) -> Self {
        Self {
            scopes: host_scopes(),
            command,
        }
    }
}
#[derive(Debug)]
struct Mailbox {
    sender: mpsc::Sender<ScopedCommand>,
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

struct CallbackState {
    thread: Option<CallbackThread>,
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
/// The session suppresses clicks while a response bundle is blocked.
#[derive(uniffi::Object)]
pub struct RuntimeSession {
    transport: Mutex<TransportSlot>,
    closed: Condvar,
    failure: Arc<Mutex<Option<String>>>,
    fence: Arc<ReentryFence>,
    #[cfg(test)]
    callback_control: Arc<tests::CallbackControl>,
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
        let command = ScopedCommand::new(command(sender));
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
        #[cfg(test)]
        tests::notify_submission();
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
        self.call_with_callbacks(move |state, refs, _| operation(state, refs))
    }
    fn call_with_callbacks<T: Send + 'static>(
        &self,
        operation: impl FnOnce(
            &mut Worker,
            &PreparedReferenceValues<'_>,
            &mut CallbackState,
        ) -> Result<T, OpenUiError>
        + Send
        + 'static,
    ) -> Result<T, OpenUiError> {
        self.dispatch_call(true, operation)
    }
    fn dispatch_call<T: Send + 'static>(
        &self,
        drain_before: bool,
        operation: impl FnOnce(
            &mut Worker,
            &PreparedReferenceValues<'_>,
            &mut CallbackState,
        ) -> Result<T, OpenUiError>
        + Send
        + 'static,
    ) -> Result<T, OpenUiError> {
        let failure = self.failure.clone();
        self.dispatch(move |sender| {
            Command::Call(Box::new(move |state, refs, callbacks| {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    if drain_before {
                        state.drain_releases();
                    }
                    operation(state, refs, callbacks)
                }));
                match result {
                    Ok(result) => {
                        if let Some(reason) = failure_reason(&failure) {
                            let _ = sender.send(Err(worker_error(&reason)));
                            return false;
                        }
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
            let _ = transport
                .mailbox
                .sender
                .send(ScopedCommand::new(Command::Stop));
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
        #[cfg(test)]
        let callback_control = Arc::new(tests::CallbackControl::default());
        #[cfg(test)]
        let worker_control = callback_control.clone();
        let mailbox = Arc::new(Mailbox { sender });

        let worker_mailbox = Arc::downgrade(&mailbox);
        let worker_failure = failure.clone();
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("openui-runtime".into())
            .spawn(move || {
                #[cfg(test)]
                let _observed_exit = tests::observe_worker_thread(worker_control);
                // Construction, operations and destruction of every thread-affine
                // runtime value happen inside this closure.
                #[cfg(test)]
                tests::record_stack();
                let mut callbacks = None;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    // Construct and destroy session policy on its owning worker.
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
                            let callbacks = callbacks.insert(CallbackState { thread: None });
                            state.set_release_notifier(openui_session::ReleaseNotifier(Arc::new(
                                move || {
                                    if let Some(mailbox) = worker_mailbox.upgrade() {
                                        let _ = mailbox
                                            .sender
                                            .send(ScopedCommand::new(Command::DrainReleases));
                                    }
                                },
                            )));
                            let _ = ready.send(Ok(()));
                            worker_loop(state, receiver, &worker_failure, callbacks);
                        }
                        Err(error) => {
                            let _ = ready.send(Err(error));
                        }
                    }
                }));
                if result.is_err() {
                    let _ = record_failure(&worker_failure, "runtime worker panicked");
                }
                if let Some(CallbackState {
                    thread: Some(thread),
                    ..
                }) = callbacks
                {
                    thread.join();
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
                    #[cfg(test)]
                    callback_control,
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
    /// execute on the session callback thread without a time bound. A host callback must
    /// not wait for another thread to call this same session: that call queues
    /// behind the callback, so the host would deadlock itself.
    /// Name/limit refusals create no callback thread and release on this caller.
    /// A failed first thread start returns Worker without changing the session;
    /// a later registration may retry. Once admitted, callback handoff/transport
    /// failure revokes the session. Disposal and final drop join both threads.
    pub fn register_function(
        &self,
        name: String,
        callback: Box<dyn LibraryFunction>,
    ) -> Result<(), OpenUiError> {
        let callback = CallerCallback {
            pending: Arc::new(Mutex::new(Some(CallbackHolder {
                callback: Some(callback),
                fence: self.fence.clone(),
            }))),
        };
        let pending = callback.pending.clone();
        let failure = self.failure.clone();
        #[cfg(test)]
        let control = self.callback_control.clone();
        self.call_with_callbacks(move |s, _, callbacks| {
            let (mailbox, receiver) = match &callbacks.thread {
                Some(thread) => (thread.mailbox.clone(), None),
                None => {
                    let (sender, receiver) = mpsc::channel();
                    (
                        Arc::new(CallbackMailbox {
                            sender,
                            failure,
                            #[cfg(test)]
                            control,
                        }),
                        Some(receiver),
                    )
                }
            };
            let proxy = Arc::new(CallbackProxy {
                key: Arc::new(()),
                mailbox: mailbox.clone(),
                installed: AtomicBool::new(false),
            });
            let implementation = proxy.clone();
            if let Some(receiver) = receiver {
                // Ask the live core through the same path as real admission.
                // Starting first keeps spawn failure atomic and retryable.
                s.check_function_registration(&name)?;
                callbacks.thread = Some(CallbackThread::start(mailbox, receiver)?);
            }
            // An uninstalled proxy owns no foreign code and sends no releases.
            s.register_function(name, Arc::new(move |json| implementation.call(json)))?;
            if let Some(thread) = &callbacks.thread {
                thread.register(proxy.key.clone(), pending)?;
                proxy.installed.store(true, Ordering::Release);
            }
            Ok(())
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

    /// Replace, project, initialize and draw as one transaction. Refusal keeps
    /// the existing session and capabilities. native_view checks Swift's
    /// existing element-root/JSON-depth requirements before commit.
    pub fn replace_program(
        &self,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
    ) -> Result<ReplacementOutput, OpenUiError> {
        self.replace_program_with_response(
            program,
            persisted,
            projected,
            native_view,
            ResponseState::default(),
        )
    }

    pub fn replace_program_with_response(
        &self,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
        response_state: ResponseState,
    ) -> Result<ReplacementOutput, OpenUiError> {
        self.dispatch_call(false, move |s, refs, _| {
            s.replace_program_with_response(
                refs,
                Arc::new(openui_session::ParsedProgram {
                    inner: program.inner.clone(),
                    json_bytes: program.json_bytes,
                }),
                persisted,
                projected,
                native_view,
                response_state.into(),
            )
            .map(Into::into)
            .map_err(Into::into)
        })
    }

    pub fn set_query_extra_values(
        &self,
        tool_name: String,
        values_json: String,
    ) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            s.set_query_extra_values(&tool_name, &values_json)
                .map_err(Into::into)
        })
    }
    pub fn advance_query_clock(&self, now_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.advance_query_clock(now_ms).map_err(Into::into))
    }
    pub fn pause_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.pause_queries().map_err(Into::into))
    }
    pub fn resume_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.resume_queries().map_err(Into::into))
    }
    pub fn resume_queries_at(&self, now_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.resume_queries_at(now_ms).map_err(Into::into))
    }
    pub fn attach_tool_provider_at(&self, now_ms: u64, minimum_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, refs| {
            s.attach_tool_provider_at(refs, now_ms, minimum_ms)
                .map_err(Into::into)
        })
    }
    pub fn attach_tool_provider_with_minimum(&self, minimum_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, refs| {
            s.attach_tool_provider_with_minimum(refs, minimum_ms)
                .map_err(Into::into)
        })
    }
    pub fn attach_tool_provider(&self) -> Result<(), OpenUiError> {
        self.call(move |s, refs| s.attach_tool_provider(refs).map_err(Into::into))
    }
    pub fn detach_tool_provider(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            s.detach_tool_provider();
            Ok(())
        })
    }
    pub fn take_query_update(&self) -> Result<QueryEffects, OpenUiError> {
        self.call(move |s, _| Ok(s.take_query_update().into()))
    }
    /// Run again every query whose error is in the query snapshot, as
    /// upstream's renderer Retry does. Claim the calls with `takeQueryUpdate`.
    pub fn retry_failed_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.retry_failed_queries().map_err(Into::into))
    }
    pub fn evaluate_with_response(
        &self,
        response_state: ResponseState,
    ) -> Result<EvaluationOutput, OpenUiError> {
        self.call(move |s, refs| {
            s.evaluate_with_response(refs, response_state.into())
                .map(Into::into)
                .map_err(Into::into)
        })
    }
    pub fn complete_tool_call(
        &self,
        call: Arc<WantedToolCall>,
        outcome_json: String,
    ) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            s.complete_tool_call_json(&call.inner.id, &outcome_json)
                .map_err(Into::into)
        })
    }
    /// Report a driver's final outcome. An outcome refused under the limits
    /// or as invalid JSON settles the call as a tool failure, so the receipt
    /// never stays live waiting for a correction no driver will send.
    pub fn finish_tool_call(
        &self,
        call: Arc<WantedToolCall>,
        outcome_json: String,
    ) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            s.finish_tool_call_json(&call.inner.id, &outcome_json)
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
    /// A blocked response bundle ignores the whole click.
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
    state: Worker,
    receiver: mpsc::Receiver<ScopedCommand>,
    failure: &Mutex<Option<String>>,
    callbacks: &mut CallbackState,
) {
    // State drops before its last command's ancestry, including on unwind.
    let mut scope = None;
    let mut state = state;
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
            let command = match receiver.recv() {
                Ok(command) => command,
                Err(_) => return,
            };
            drop(scope.take());
            scope = Some(HostScope::enter(command.scopes));
            if failure_reason(failure).is_some() {
                return;
            }
            match command.command {
                Command::Call(job) => {
                    if !job(&mut state, &prepared, callbacks) {
                        return;
                    }
                }
                Command::References(json, reply) => {
                    match catch_unwind(AssertUnwindSafe(|| {
                        state.check_reference_values()?;
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
                Command::DrainReleases => {
                    state.drain_releases();
                }
                Command::Stop => return,
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
            query_evaluation_error: v.query_evaluation_error.map(Into::into),
            query_snapshot_json: v.query_snapshot_json,
            is_query_loading: v.is_query_loading,
            parser_error_json: v.parser_error_json,
            blocked: v.blocked,
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
            max_cache_bytes: v.max_cache_bytes,
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
            peak_memory: v.peak_memory,
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
            openui_session::OpenUiError::InvalidToolCall => Self::InvalidToolCall,
            openui_session::OpenUiError::ReferenceConflict => Self::ReferenceConflict,
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
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Copy)]
    pub(super) enum Fault {
        Spawn = 1,
        RegisterSend,
        RegisterReply,
        RegisterAccepted,
        InvokeSend,
        InvokeReply,
        ReleaseSend,
        ReleaseReply,
    }
    #[derive(Default)]
    pub(super) struct CallbackControl {
        fault: AtomicUsize,
        started: AtomicUsize,
        exited: AtomicUsize,
        worker_exited: AtomicUsize,
    }
    impl CallbackControl {
        pub(super) fn take(&self, fault: Fault) -> bool {
            self.fault
                .compare_exchange(fault as usize, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        }
        pub(super) fn fail_send(&self, operation: &CallbackOperation) -> bool {
            match operation {
                CallbackOperation::Register(..) => self.take(Fault::RegisterSend),
                CallbackOperation::Invoke(..) => self.take(Fault::InvokeSend),
                CallbackOperation::Release(..) => self.take(Fault::ReleaseSend),
                CallbackOperation::Stop => false,
            }
        }
    }
    pub(super) struct ObservedWorkerExit(Arc<CallbackControl>);
    impl Drop for ObservedWorkerExit {
        fn drop(&mut self) {
            self.0.worker_exited.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct ObservedCallbackExit(Arc<CallbackControl>);
    impl Drop for ObservedCallbackExit {
        fn drop(&mut self) {
            // Exercise late teardown scheduling before publishing the thread's
            // final witness. Joining must cover this too, without a time bound.
            for _ in 0..64 {
                std::thread::yield_now();
            }
            self.0.exited.fetch_add(1, Ordering::SeqCst);
        }
    }
    thread_local! {
        // Runs during actual thread teardown, after the callback loop's locals
        // (including the registry and unexpected-exit guard) have been dropped.
        static CALLBACK_EXIT: RefCell<Option<ObservedCallbackExit>> = const { RefCell::new(None) };
    }
    pub(super) fn observe_callback_thread(control: Arc<CallbackControl>) {
        control.started.fetch_add(1, Ordering::SeqCst);
        CALLBACK_EXIT.with(|exit| *exit.borrow_mut() = Some(ObservedCallbackExit(control)));
    }
    pub(super) fn observe_worker_thread(control: Arc<CallbackControl>) -> ObservedWorkerExit {
        ObservedWorkerExit(control)
    }

    // stacker measures the ordinary OS stack before any core walk starts.
    // The anchor gives its upper end with a conservative 128 KiB margin.
    thread_local! {
        static ORIGINAL_STACK: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
        pub(super) static ORIGIN_ON_STACK: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
        static SUBMISSION: RefCell<Option<mpsc::Sender<()>>> = const { RefCell::new(None) };
        static UNWIND_SUBMISSION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    pub(super) fn notify_submission() {
        assert!(!UNWIND_SUBMISSION.replace(false), "injected caller unwind");
        SUBMISSION.with(|reply| {
            if let Some(reply) = reply.borrow().as_ref() {
                let _ = reply.send(());
            }
        });
    }
    pub(super) fn record_stack() {
        let anchor = 0u8;
        let address = &anchor as *const u8 as usize;
        ORIGINAL_STACK.set(stacker::remaining_stack().map(|left| {
            (
                address.saturating_sub(left),
                address.saturating_add(128 * 1024),
            )
        }));
    }
    pub(super) fn on_original_stack() -> Option<bool> {
        let local = 0u8;
        let address = &local as *const u8 as usize;
        ORIGINAL_STACK
            .get()
            .map(|(low, high)| (low..high).contains(&address))
    }
    struct Function<F>(F);
    impl<F: Fn(String) -> Result<Option<String>, FunctionError> + Send + Sync> LibraryFunction
        for Function<F>
    {
        fn call(&self, json: String) -> Result<Option<String>, FunctionError> {
            (self.0)(json)
        }
    }
    const FUNCTION_SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"properties":{"value":{}}}}}}"#;

    struct StackDropProbe(Arc<std::sync::atomic::AtomicUsize>);
    impl LibraryFunction for StackDropProbe {
        fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
            Ok(None)
        }
    }
    impl Drop for StackDropProbe {
        fn drop(&mut self) {
            assert_eq!(std::thread::current().name(), Some("openui-callback"));
            if let Some(on_stack) = on_original_stack() {
                assert!(on_stack);
            }
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[test]
    fn accepted_foreign_releases_use_the_original_callback_stack_on_replacement_and_worker_unwind()
    {
        let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
        let session = RuntimeSession::new(library, None).unwrap();
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let make = || Box::new(StackDropProbe(drops.clone())) as Box<dyn LibraryFunction>;
        session.register_function("Echo".into(), make()).unwrap();
        session.register_function("Echo".into(), make()).unwrap();
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
        let result: Result<(), OpenUiError> =
            session.call(|_, _| panic!("worker unwind with a registration"));
        assert!(matches!(result, Err(OpenUiError::Worker { .. })));
        session.dispose();
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    // These hooks inject infrastructure failures only. Every assertion exercises
    // the exported session API, including release observations outside Drop so
    // panic containment cannot hide a broken thread or reentry guarantee.
    fn watched(name: &str, body: impl FnOnce()) {
        let full = format!("runtime::tests::{name}");
        if std::env::var("OPENUI_FFI_LIFECYCLE_CHILD").as_deref() == Ok(&full) {
            body();
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &full, "--nocapture"])
            .env("OPENUI_FFI_LIFECYCLE_CHILD", &full)
            .spawn()
            .unwrap();
        for _ in 0..6000 {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "lifecycle child failed: {status}");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("lifecycle child stalled (liveness watchdog)");
    }
    #[derive(Debug)]
    struct ReleaseObservation {
        thread: ThreadId,
        name: Option<String>,
        original_stack: Option<bool>,
        reentry: String,
    }
    type Releases = Arc<Mutex<Vec<ReleaseObservation>>>;
    struct ReleaseProbe {
        owner: std::sync::Weak<RuntimeSession>,
        releases: Releases,
        panic: bool,
    }
    impl LibraryFunction for ReleaseProbe {
        fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
            Ok(Some("7".into()))
        }
    }
    impl Drop for ReleaseProbe {
        fn drop(&mut self) {
            let session = self.owner.upgrade().unwrap();
            let reentry = session.version().unwrap_err().to_string();
            session.dispose();
            self.releases.lock().unwrap().push(ReleaseObservation {
                thread: std::thread::current().id(),
                name: std::thread::current().name().map(str::to_owned),
                original_stack: on_original_stack(),
                reentry,
            });
            assert!(!self.panic, "injected host destructor panic");
        }
    }
    fn probe(
        session: &Arc<RuntimeSession>,
        releases: &Releases,
        panic: bool,
    ) -> Box<dyn LibraryFunction> {
        Box::new(ReleaseProbe {
            owner: Arc::downgrade(session),
            releases: releases.clone(),
            panic,
        })
    }
    fn assert_releases(releases: &Releases, names: &[Option<ThreadId>]) {
        let observations = releases.lock().unwrap();
        assert_eq!(observations.len(), names.len(), "{observations:?}");
        for (seen, caller) in observations.iter().zip(names) {
            match caller {
                Some(id) => assert_eq!(seen.thread, *id, "{seen:?}"),
                None => assert_eq!(seen.name.as_deref(), Some("openui-callback"), "{seen:?}"),
            }
            assert_ne!(seen.original_stack, Some(false), "{seen:?}");
            assert_eq!(seen.reentry, "session already in use");
        }
    }
    fn assert_joined(session: &RuntimeSession, callbacks: usize) {
        session.dispose();
        session.dispose();
        let control = &session.callback_control;
        assert_eq!(control.started.load(Ordering::SeqCst), callbacks);
        assert_eq!(control.exited.load(Ordering::SeqCst), callbacks);
        assert_eq!(control.worker_exited.load(Ordering::SeqCst), 1);
    }
    fn assert_sticky_failure(session: &RuntimeSession, reason: &str) {
        for error in [
            session.version().unwrap_err(),
            session.evaluate().unwrap_err(),
            session.snapshot().unwrap_err(),
            session.set("$n".into(), Some("9".into())).unwrap_err(),
            session.set_reference_values("{}".into()).unwrap_err(),
        ] {
            assert!(
                matches!(error, OpenUiError::Worker { reason: ref later } if later == reason),
                "{error:?}"
            );
        }
    }
    #[test]
    fn a_caller_unwind_reclaims_its_queued_implementation_before_handoff() {
        watched(
            "a_caller_unwind_reclaims_its_queued_implementation_before_handoff",
            || {
                record_stack();
                let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                let session = RuntimeSession::new(library, None).unwrap();
                let (entered, active) = mpsc::channel();
                let (release, wait) = mpsc::channel();
                let blocked = session.clone();
                let blocker = std::thread::spawn(move || {
                    blocked.call(move |_, _| {
                        entered.send(()).unwrap();
                        wait.recv().unwrap();
                        Ok(())
                    })
                });
                active.recv().unwrap();
                let releases = Releases::default();
                UNWIND_SUBMISSION.set(true);
                assert!(
                    catch_unwind(AssertUnwindSafe(|| {
                        session.register_function("Echo".into(), probe(&session, &releases, true))
                    }))
                    .is_err()
                );
                assert_releases(&releases, &[Some(std::thread::current().id())]);
                release.send(()).unwrap();
                blocker.join().unwrap().unwrap();
                assert_sticky_failure(&session, "host function released before registration");
                assert_joined(&session, 1);
                assert_releases(&releases, &[Some(std::thread::current().id())]);
            },
        );
    }
    #[test]
    fn refused_registrations_release_on_the_caller_without_starting_a_callback_thread() {
        watched(
            "refused_registrations_release_on_the_caller_without_starting_a_callback_thread",
            || {
                for (name, limit, expected) in [
                    ("missing", 1024, "expected a declared library function name"),
                    ("Echo", 67, "function registration limit"),
                    ("Echo", 3, "host input byte limit"),
                ] {
                    record_stack();
                    let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                    let session = RuntimeSession::with_limits(
                        library,
                        None,
                        RuntimeLimits {
                            max_json_bytes: limit,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let releases = Releases::default();
                    let before = (session.snapshot().unwrap(), session.version().unwrap());
                    let error = session
                        .register_function(name.into(), probe(&session, &releases, true))
                        .unwrap_err();
                    assert!(matches!(
                        error,
                        OpenUiError::InvalidJson { .. } | OpenUiError::LimitExceeded { .. }
                    ));
                    assert_eq!(error.to_string(), expected);
                    assert_eq!(
                        (session.snapshot().unwrap(), session.version().unwrap()),
                        before
                    );
                    assert_eq!(session.callback_control.started.load(Ordering::SeqCst), 0);
                    assert_releases(&releases, &[Some(std::thread::current().id())]);
                    assert_joined(&session, 0);
                    assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
                }
                // A refused registration after acceptance preserves the implementation,
                // evaluation bytes/flags and the sole existing callback thread.
                let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                let session = RuntimeSession::new(library.clone(), None).unwrap();
                session
                    .set_program(crate::parse("root=Box(@Echo())".into(), library, None).program)
                    .unwrap();
                let releases = Releases::default();
                session
                    .register_function("Echo".into(), probe(&session, &releases, false))
                    .unwrap();
                let before = session.evaluate().unwrap();
                assert!(matches!(
                    session.register_function("missing".into(), probe(&session, &releases, false)),
                    Err(OpenUiError::InvalidJson { .. })
                ));
                let after = session.evaluate().unwrap();
                assert_eq!(
                    (before.json, before.size_limited, before.depth_limited),
                    (after.json, after.size_limited, after.depth_limited)
                );
                assert_releases(&releases, &[Some(std::thread::current().id())]);
                assert_joined(&session, 1);
                assert_releases(&releases, &[Some(std::thread::current().id()), None]);
            },
        );
    }
    #[test]
    fn a_failed_first_callback_thread_start_preserves_the_session_and_registration_room() {
        watched(
            "a_failed_first_callback_thread_start_preserves_the_session_and_registration_room",
            || {
                record_stack();
                let schema = r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{}},"Other":{"params":{}}}}"#;
                let library = ComponentLibrary::new(schema.into()).unwrap();
                let session = RuntimeSession::with_limits(
                    library.clone(),
                    None,
                    RuntimeLimits {
                        max_json_bytes: 2048,
                        ..Default::default()
                    },
                )
                .unwrap();
                session
                    .set_program(
                        crate::parse(
                            "$n=1\nroot=Box($n,Action([@Set($n,2)]))".into(),
                            library.clone(),
                            None,
                        )
                        .program,
                    )
                    .unwrap();
                session.initialize(None).unwrap();
                let before = session.evaluate().unwrap();
                let action = session
                    .action_at(
                        before.evaluation.clone(),
                        vec!["props".into(), "action".into()],
                    )
                    .unwrap();
                let snapshot = session.snapshot().unwrap();
                let version = session.version().unwrap();
                let pristine = session.is_pristine("$n".into()).unwrap();
                let releases = Releases::default();
                // Repeated failures must retain neither a registration nor byte room.
                for _ in 0..2 {
                    session
                        .callback_control
                        .fault
                        .store(Fault::Spawn as usize, Ordering::SeqCst);
                    let error = session
                        .register_function("Echo".into(), probe(&session, &releases, true))
                        .unwrap_err();
                    assert!(matches!(error, OpenUiError::Worker { ref reason }
                        if reason == "injected callback spawn failure"));
                    assert_eq!(session.snapshot().unwrap(), snapshot);
                    assert_eq!(session.version().unwrap(), version);
                    assert_eq!(session.is_pristine("$n".into()).unwrap(), pristine);
                    assert_eq!(session.pending_continuations().unwrap(), 0);
                    // Lookup through the original generation still works before another draw.
                    session
                        .action_at(
                            before.evaluation.clone(),
                            vec!["props".into(), "action".into()],
                        )
                        .unwrap();
                }
                assert_releases(&releases, &[Some(std::thread::current().id()); 2]);
                assert_eq!(session.callback_control.started.load(Ordering::SeqCst), 0);
                let after = session.evaluate().unwrap();
                assert_eq!(
                    (&before.json, before.size_limited, before.depth_limited),
                    (&after.json, after.size_limited, after.depth_limited)
                );
                session
                    .register_function("Echo".into(), probe(&session, &releases, false))
                    .unwrap();
                session
                    .set_program(crate::parse("root=Box(@Echo())".into(), library, None).program)
                    .unwrap();
                assert!(session.evaluate().unwrap().json.contains("\"value\":7"));
                // The pre-failure plan was retained through refusal (lookup above);
                // successful draws/program replacement stale it as usual.
                assert!(matches!(
                    session.run(Some(action), String::new(), None),
                    Err(OpenUiError::StaleHandle)
                ));
                assert_joined(&session, 1);
                assert_releases(
                    &releases,
                    &[
                        Some(std::thread::current().id()),
                        Some(std::thread::current().id()),
                        None,
                    ],
                );
                assert!(matches!(session.version(), Err(OpenUiError::Disposed)));

                // With room for one registration only, a different name must fit
                // after a failed start; replacing Echo alone would hide spent room.
                let library = ComponentLibrary::new(schema.into()).unwrap();
                let session = RuntimeSession::with_limits(
                    library,
                    None,
                    RuntimeLimits {
                        max_json_bytes: 72,
                        ..Default::default()
                    },
                )
                .unwrap();
                let releases = Releases::default();
                session
                    .callback_control
                    .fault
                    .store(Fault::Spawn as usize, Ordering::SeqCst);
                assert!(matches!(
                    session.register_function("Echo".into(), probe(&session, &releases, true)),
                    Err(OpenUiError::Worker { .. })
                ));
                session
                    .register_function("Other".into(), probe(&session, &releases, false))
                    .unwrap();
                assert!(matches!(
                    session.register_function("Echo".into(), probe(&session, &releases, true)),
                    Err(OpenUiError::LimitExceeded { .. })
                ));
                assert_joined(&session, 1);
                assert_releases(
                    &releases,
                    &[
                        Some(std::thread::current().id()),
                        Some(std::thread::current().id()),
                        None,
                    ],
                );
            },
        );
    }

    #[test]
    fn a_reclaimed_handoff_replies_with_an_error_and_keeps_serving_callbacks() {
        watched(
            "a_reclaimed_handoff_replies_with_an_error_and_keeps_serving_callbacks",
            || {
                let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                let session = RuntimeSession::new(library.clone(), None).unwrap();
                session
                    .set_program(crate::parse("root=Box(@Echo())".into(), library, None).program)
                    .unwrap();
                let releases = Releases::default();
                session
                    .register_function("Echo".into(), probe(&session, &releases, false))
                    .unwrap();
                let before = session.evaluate().unwrap();
                // Inject only the stale infrastructure job, as if its caller's
                // guard had already emptied the handoff. Public operations below
                // must continue using the same thread and implementation.
                let error = session
                    .call_with_callbacks(|_, _, callbacks| {
                        let thread = callbacks.thread.as_ref().unwrap();
                        let (reply, received) = mpsc::sync_channel(1);
                        thread.mailbox.send(CallbackOperation::Register(
                            Arc::new(()),
                            Arc::new(Mutex::new(None)),
                            reply,
                        ))?;
                        received.recv().unwrap()
                    })
                    .unwrap_err();
                assert!(matches!(error, OpenUiError::Worker { ref reason }
                    if reason == "host function released before registration"));
                assert!(session.version().is_ok());
                let after = session.evaluate().unwrap();
                assert_eq!(
                    (before.json, before.size_limited, before.depth_limited),
                    (after.json, after.size_limited, after.depth_limited)
                );
                session
                    .register_function("Echo".into(), probe(&session, &releases, false))
                    .unwrap();
                assert_releases(&releases, &[None]);
                assert_joined(&session, 1);
                assert_releases(&releases, &[None, None]);
            },
        );
    }

    #[test]
    fn disposal_and_final_drop_complete_callback_thread_joins() {
        watched(
            "disposal_and_final_drop_complete_callback_thread_joins",
            || {
                for dispose in [false, true].into_iter().cycle().take(64) {
                    let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                    let session = RuntimeSession::new(library, None).unwrap();
                    let drops = Arc::new(AtomicUsize::new(0));
                    session
                        .register_function("Echo".into(), Box::new(StackDropProbe(drops.clone())))
                        .unwrap();
                    let control = session.callback_control.clone();
                    if dispose {
                        session.dispose();
                        session.dispose();
                        assert_eq!(control.exited.load(Ordering::SeqCst), 1);
                    }
                    drop(session);
                    assert_eq!(drops.load(Ordering::SeqCst), 1);
                    assert_eq!(control.started.load(Ordering::SeqCst), 1);
                    assert_eq!(control.exited.load(Ordering::SeqCst), 1);
                    assert_eq!(control.worker_exited.load(Ordering::SeqCst), 1);
                }
            },
        );
    }

    #[test]
    fn callback_start_and_registration_failures_release_once_on_the_fenced_caller() {
        watched(
            "callback_start_and_registration_failures_release_once_on_the_fenced_caller",
            || {
                for fault in [Fault::RegisterSend, Fault::RegisterReply] {
                    record_stack();
                    let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                    let session = RuntimeSession::new(library, None).unwrap();
                    let releases = Releases::default();
                    session
                        .callback_control
                        .fault
                        .store(fault as usize, Ordering::SeqCst);
                    let error = session
                        .register_function("Echo".into(), probe(&session, &releases, true))
                        .unwrap_err();
                    let OpenUiError::Worker { reason } = error else {
                        panic!("untyped failure: {error:?}")
                    };
                    assert_eq!(reason, "callback thread stopped unexpectedly");
                    assert_releases(&releases, &[Some(std::thread::current().id())]);
                    assert_sticky_failure(&session, &reason);
                    // Later rejected registrations must release on their own caller too.
                    assert!(matches!(
                        session.register_function("Echo".into(), probe(&session, &releases, false)),
                        Err(OpenUiError::Worker { .. })
                    ));
                    assert_joined(&session, 1);
                    assert_sticky_failure(&session, &reason);
                    assert_releases(&releases, &[Some(std::thread::current().id()); 2]);
                }
            },
        );
    }
    #[test]
    fn lost_registration_acknowledgements_release_each_owner_once_and_final_drop_joins() {
        watched(
            "lost_registration_acknowledgements_release_each_owner_once_and_final_drop_joins",
            || {
                for replacing in [false, true] {
                    for fault in [
                        Fault::RegisterSend,
                        Fault::RegisterReply,
                        Fault::RegisterAccepted,
                    ] {
                        record_stack();
                        let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                        let session = RuntimeSession::new(library, None).unwrap();
                        let releases = Releases::default();
                        if replacing {
                            session
                                .register_function("Echo".into(), probe(&session, &releases, true))
                                .unwrap();
                        }
                        session
                            .callback_control
                            .fault
                            .store(fault as usize, Ordering::SeqCst);
                        let error = session
                            .register_function("Echo".into(), probe(&session, &releases, true))
                            .unwrap_err();
                        assert!(
                            matches!(error, OpenUiError::Worker { ref reason } if reason == "callback thread stopped unexpectedly")
                        );
                        assert_sticky_failure(&session, &error.to_string());
                        let control = session.callback_control.clone();
                        // Observe destruction while the session can still be upgraded.
                        session.dispose();
                        let mut owners = Vec::new();
                        if replacing {
                            owners.push(None);
                        }
                        owners.push(if matches!(fault, Fault::RegisterAccepted) {
                            None
                        } else {
                            Some(std::thread::current().id())
                        });
                        assert_releases(&releases, &owners);
                        drop(session);
                        assert_eq!(control.started.load(Ordering::SeqCst), 1);
                        assert_eq!(control.exited.load(Ordering::SeqCst), 1);
                        assert_eq!(control.worker_exited.load(Ordering::SeqCst), 1);
                    }
                }
                // Failed sessions also join on final drop without explicit disposal.
                let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                let session = RuntimeSession::new(library.clone(), None).unwrap();
                session
                    .set_program(crate::parse("root=Box(@Echo())".into(), library, None).program)
                    .unwrap();
                let drops = Arc::new(AtomicUsize::new(0));
                session
                    .register_function("Echo".into(), Box::new(StackDropProbe(drops.clone())))
                    .unwrap();
                session
                    .callback_control
                    .fault
                    .store(Fault::InvokeReply as usize, Ordering::SeqCst);
                assert!(matches!(
                    session.evaluate(),
                    Err(OpenUiError::Worker { .. })
                ));
                let control = session.callback_control.clone();
                drop(session);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                assert_eq!(control.started.load(Ordering::SeqCst), 1);
                assert_eq!(control.exited.load(Ordering::SeqCst), 1);
                assert_eq!(control.worker_exited.load(Ordering::SeqCst), 1);
            },
        );
    }

    #[test]
    fn callback_transport_death_is_a_sticky_worker_error_and_disposal_joins_both_threads() {
        watched(
            "callback_transport_death_is_a_sticky_worker_error_and_disposal_joins_both_threads",
            || {
                for fault in [
                    Fault::InvokeSend,
                    Fault::InvokeReply,
                    Fault::ReleaseSend,
                    Fault::ReleaseReply,
                ] {
                    let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
                    let session = RuntimeSession::new(library.clone(), None).unwrap();
                    session
                        .set_program(
                            crate::parse("root=Box(@Echo())".into(), library, None).program,
                        )
                        .unwrap();
                    let releases = Releases::default();
                    session
                        .register_function("Echo".into(), probe(&session, &releases, true))
                        .unwrap();
                    session
                        .callback_control
                        .fault
                        .store(fault as usize, Ordering::SeqCst);
                    let error = if matches!(fault, Fault::ReleaseSend | Fault::ReleaseReply) {
                        session
                            .register_function("Echo".into(), probe(&session, &releases, true))
                            .unwrap_err()
                    } else {
                        session.evaluate().unwrap_err()
                    };
                    assert!(
                        matches!(error, OpenUiError::Worker { ref reason } if reason == "callback thread stopped unexpectedly"),
                        "{error:?}"
                    );
                    assert_sticky_failure(&session, &error.to_string());
                    assert_joined(&session, 1);
                    let owners = match fault {
                        Fault::ReleaseReply => vec![None, Some(std::thread::current().id())],
                        Fault::ReleaseSend => vec![None, None],
                        _ => vec![None],
                    };
                    assert_releases(&releases, &owners);
                }
            },
        );
    }
    #[test]
    fn an_idle_callback_thread_death_revokes_calls_that_do_not_invoke_functions() {
        watched(
            "an_idle_callback_thread_death_revokes_calls_that_do_not_invoke_functions",
            || {
                let schema = FUNCTION_SCHEMA.replace(
                    "\"functions\":{",
                    "\"functions\":{\"Next\":{\"params\":{}},",
                );
                let library = ComponentLibrary::new(schema).unwrap();
                let limits = RuntimeLimits {
                    max_json_bytes: 68,
                    ..Default::default()
                };
                let session =
                    RuntimeSession::with_limits(library.clone(), None, limits.clone()).unwrap();
                let releases = Releases::default();
                session
                    .register_function("Echo".into(), probe(&session, &releases, true))
                    .unwrap();
                // Infrastructure-only injection: kill and join the callback thread,
                // without asking the mailbox to record a send/receive failure.
                let _: Result<(), OpenUiError> =
                    session.call_with_callbacks(move |state, _, callbacks| {
                        assert!(matches!(
                            state.check_function_registration("Next"),
                            Err(openui_session::OpenUiError::LimitExceeded { .. })
                        ));
                        // Reset the test core (and thus release all registry proxies)
                        // before removing their callback transport. No stale registry
                        // or charged registration room may outlive the removed thread.
                        *state = Worker::new(
                            Arc::new(openui_session::ComponentLibrary {
                                inner: library.inner.clone(),
                                schema: library.schema.clone(),
                            }),
                            None,
                            limits.into(),
                        )?;
                        state.check_function_registration("Next")?;
                        let thread = callbacks.thread.take().unwrap();
                        thread.mailbox.send(CallbackOperation::Stop).unwrap();
                        thread.thread.join().unwrap();
                        Ok(())
                    });
                // Only the exported calls below determine the observed failure.
                assert_sticky_failure(&session, "callback thread stopped unexpectedly");
                assert_joined(&session, 1);
                assert_releases(&releases, &[None]);
            },
        );
    }

    struct QueuedDropProbe {
        ancestor: std::sync::Weak<RuntimeSession>,
        own: std::sync::Weak<RuntimeSession>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl LibraryFunction for QueuedDropProbe {
        fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
            Ok(None)
        }
    }
    impl Drop for QueuedDropProbe {
        fn drop(&mut self) {
            // This unexecuted registration returns to A's callback thread,
            // under A's submitting scope and B's destructor fence.
            assert_eq!(std::thread::current().name(), Some("openui-callback"));
            if let Some(on_stack) = on_original_stack() {
                assert!(on_stack);
            }
            for session in [&self.ancestor, &self.own] {
                let session = session.upgrade().unwrap();
                assert_eq!(
                    session.version().unwrap_err().to_string(),
                    "session already in use"
                );
                session.dispose();
            }
            self.dropped
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[test]
    fn queued_registration_destruction_after_worker_failure_preserves_ancestry() {
        let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
        let a = RuntimeSession::new(library.clone(), None).unwrap();
        let b = RuntimeSession::new(library.clone(), None).unwrap();
        a.set_program(crate::parse("root=Box(@Echo())".into(), library, None).program)
            .unwrap();
        let (entered, active) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let blocked = b.clone();
        let failure = std::thread::spawn(move || {
            let result: Result<(), OpenUiError> = blocked.call(move |_, _| {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                panic!("injected worker failure with queued registration");
            });
            assert!(matches!(result, Err(OpenUiError::Worker { .. })));
        });
        active.recv().unwrap();
        let (submitted, queued) = mpsc::channel();
        let ancestor = Arc::downgrade(&a);
        let other = b.clone();
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = dropped.clone();
        a.register_function(
            "Echo".into(),
            Box::new(Function(move |_| {
                SUBMISSION.with(|reply| *reply.borrow_mut() = Some(submitted.clone()));
                let result = other.register_function(
                    "Echo".into(),
                    Box::new(QueuedDropProbe {
                        ancestor: ancestor.clone(),
                        own: Arc::downgrade(&other),
                        dropped: count.clone(),
                    }),
                );
                SUBMISSION.with(|reply| *reply.borrow_mut() = None);
                assert!(matches!(result, Err(OpenUiError::Worker { .. })));
                Ok(Some("7".into()))
            })),
        )
        .unwrap();
        let evaluating = a.clone();
        let evaluation = std::thread::spawn(move || evaluating.evaluate().unwrap().json);
        queued.recv().unwrap();
        release.send(()).unwrap();
        failure.join().unwrap();
        assert!(evaluation.join().unwrap().contains("\"value\":7"));
        b.dispose();
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(a.version().is_ok());
        a.dispose();
    }

    #[test]
    fn deep_callbacks_run_on_the_callback_threads_original_stack() {
        let library = ComponentLibrary::new(FUNCTION_SCHEMA.into()).unwrap();
        let limits = ParseLimits {
            depth: Some(20_000),
            result_size: Some(8 * 1024 * 1024),
            ..Default::default()
        };
        let session = RuntimeSession::with_limits(
            library.clone(),
            None,
            RuntimeLimits {
                evaluation: limits.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let ids = Arc::new(Mutex::new(Vec::new()));
        let seen = ids.clone();
        session
            .register_function(
                "Echo".into(),
                Box::new(Function(move |json: String| {
                    assert_eq!(std::thread::current().name(), Some("openui-callback"));
                    // A reroute onto a grown segment of the callback thread also fails.
                    if let Some(on_stack) = on_original_stack() {
                        assert!(on_stack);
                    }
                    seen.lock()
                        .unwrap()
                        .push((std::thread::current().id(), ORIGIN_ON_STACK.get()));
                    Ok(Some(json))
                })),
            )
            .unwrap();
        for depth in [0, 12_000, 0] {
            let source = format!(
                "root=Box({}@Echo(7){})",
                "[".repeat(depth),
                "]".repeat(depth)
            );
            let parsed = crate::parse_with_limits(source, library.clone(), None, limits.clone());
            assert!(!parsed.depth_limited && !parsed.size_limited);
            session.set_program(parsed.program).unwrap();
            let tree = session.evaluate().unwrap();
            assert!(!tree.depth_limited && !tree.size_limited, "{}", tree.json);
            assert!(tree.json.ends_with(r#""errors":[]}"#), "{}", tree.json);
        }
        session.dispose();
        let ids = ids.lock().unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().all(|(id, _)| *id == ids[0].0));
        if ids[0].1.is_some() {
            assert_eq!(ids[0].1, Some(true));
            assert_eq!(
                ids[1].1,
                Some(false),
                "deep evaluation must actually leave the worker's OS stack"
            );
            assert_eq!(ids[2].1, Some(true));
        }
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

#[cfg(test)]
#[path = "runtime_retained_root_test.rs"]
mod retained_root_test;
