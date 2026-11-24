//! Rust-callable, one-thread adapter. Exported methods forward here one-to-one.
use openui_lang::{JsObject, Limits, ResponseReader, ResponseReaderError};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::Arc,
};

struct State {
    core: SessionCore,
    response_limits: Limits,
    /// The raw response reader for `setResponse`, with its parser (ADR 0015).
    reader: Option<ResponseReader>,
    references: PreparedReferenceValues<'static>,
}
/// A response state as the package passes it, every host getter already
/// run. The script names are still the JSON text the package built; the
/// session reads it (`SessionCore::script_names`) under its byte limit once
/// borrowed, so a raw call cannot make it parse text the limit refuses.
pub struct RawResponse {
    pub streaming: bool,
    pub is_bundle: bool,
    pub complete: bool,
    pub error: Option<String>,
    pub scripts_json: Option<String>,
    pub script_revision: Option<String>,
}
impl Default for RawResponse {
    fn default() -> Self {
        let settled = ResponseState::default();
        Self {
            streaming: settled.streaming,
            is_bundle: settled.is_bundle,
            complete: settled.complete,
            error: None,
            scripts_json: None,
            script_revision: None,
        }
    }
}
impl RawResponse {
    fn read(self, core: &SessionCore) -> Result<ResponseState, OpenUiError> {
        Ok(ResponseState {
            streaming: self.streaming,
            is_bundle: self.is_bundle,
            complete: self.complete,
            error: self.error,
            scripts: match self.scripts_json {
                Some(json) => core.script_names(&json)?,
                None => Vec::new(),
            },
            script_revision: self.script_revision,
        })
    }
}
pub struct Session {
    state: Rc<RefCell<Option<State>>>,
    failed: Cell<bool>,
}
/// Dropped after an exported object's core token. Weak ownership permits late
/// JavaScript finalizers without retaining the session or borrowing recursively.
#[derive(Clone)]
pub(crate) struct ReleaseGuard(Weak<RefCell<Option<State>>>);
impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        if let Some(state) = self.0.upgrade()
            && let Ok(mut state) = state.try_borrow_mut()
            && let Some(state) = state.as_mut()
        {
            state.core.drain_releases();
        }
    }
}
impl Session {
    pub(crate) fn release_guard(&self) -> ReleaseGuard {
        ReleaseGuard(Rc::downgrade(&self.state))
    }
    pub fn new(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
    ) -> Result<Self, OpenUiError> {
        Self::with_limits(library, prompt_spec, RuntimeLimits::default())
    }
    pub fn with_limits(
        library: Arc<ComponentLibrary>,
        prompt_spec: Option<String>,
        limits: RuntimeLimits,
    ) -> Result<Self, OpenUiError> {
        let evaluation = limits.evaluation.to_core();
        let response_limits = evaluation
            .with_program_size(
                evaluation
                    .program_size()
                    .min(Limits::DEFAULT.program_size()),
            )
            .with_depth(evaluation.depth().min(Limits::DEFAULT.depth()));
        Ok(Self {
            failed: Cell::new(false),
            state: Rc::new(RefCell::new(Some(State {
                response_limits,
                reader: None,
                core: SessionCore::new(library, prompt_spec, limits.clone())?,
                references: PreparedReferenceValues::owned_with_limits(
                    JsObject::new(),
                    limits.reference_preparation.to_core(),
                ),
            }))),
        })
    }
    fn access<T>(
        &self,
        operation: impl FnOnce(&mut State) -> Result<T, OpenUiError>,
    ) -> Result<T, OpenUiError> {
        if self.failed.get() {
            return Err(OpenUiError::Worker {
                reason: "runtime panic; session revoked".into(),
            });
        }
        let mut slot = self.state.try_borrow_mut().map_err(|_| busy())?;
        let state = slot.as_mut().ok_or(OpenUiError::Disposed)?;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(state))) {
            Ok(result) => result,
            Err(_) => {
                slot.take();
                self.failed.set(true);
                Err(OpenUiError::Worker {
                    reason: "runtime panic; session revoked".into(),
                })
            }
        }
    }

    /// Reuse the opaque preparation, including its cut flags and input credit.
    fn call<T>(
        &self,
        operation: impl FnOnce(&mut SessionCore, &PreparedReferenceValues<'_>) -> Result<T, OpenUiError>,
    ) -> Result<T, OpenUiError> {
        self.access(|s| operation(&mut s.core, &s.references))
    }
    pub fn register_function(
        &self,
        name: String,
        callback: Arc<HostFunction>,
    ) -> Result<(), OpenUiError> {
        self.access(|s| s.core.register_function(name, callback))
    }
    pub fn set_reference_values(&self, json: String) -> Result<(), OpenUiError> {
        self.access(|s| {
            s.core.check_reference_values()?;
            let values = s.core.object(&json)?;
            s.references =
                PreparedReferenceValues::owned_with_limits(values, s.core.reference_limits);
            Ok(())
        })
    }
    pub fn dispose(&self) -> Result<(), OpenUiError> {
        self.state.try_borrow_mut().map_err(|_| busy())?.take();
        Ok(())
    }
    /// Replace the current program by sharing its Arc. No JSON is parsed or
    /// program cloned. Old action handles become stale; continuations survive
    /// and resume against the new program, as the runtime specifies.
    pub fn set_program(&self, program: Arc<ParsedProgram>) -> Result<(), OpenUiError> {
        self.call(|s, refs| s.set_program(refs, program))
    }

    /// Replace the program, initialize and evaluate as one call: either all
    /// of it is committed or the session is left exactly as it was.
    pub fn replace_program(
        &self,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
    ) -> Result<ReplacementOutput, OpenUiError> {
        self.call(|s, refs| s.replace_program(refs, program, persisted, projected, native_view))
    }
    pub fn replace_program_with_response(
        &self,
        program: Arc<ParsedProgram>,
        persisted: Option<String>,
        projected: Option<String>,
        native_view: bool,
        response: RawResponse,
    ) -> Result<ReplacementOutput, OpenUiError> {
        self.call(|s, refs| {
            let response = response.read(s)?;
            s.replace_program_with_response(
                refs,
                program,
                persisted,
                projected,
                native_view,
                response,
            )
        })
    }
    /// Stage a read of the raw response: `units` is the whole text, or with
    /// `base` what follows the last accepted text, `base` UTF-16 units long.
    /// Returns the reader's draw JSON, or `{"resend":true}` when `base` is
    /// not the length of the text the reader accepted.
    pub(crate) fn read_response(
        &self,
        units: &[u16],
        base: Option<usize>,
        streaming: bool,
    ) -> Result<String, OpenUiError> {
        self.access(|s| {
            let reader = s.reader.get_or_insert_with(|| {
                ResponseReader::new_utf16(
                    s.core.stream_parser(s.response_limits),
                    s.response_limits,
                )
            });
            let read = match base {
                Some(base) => reader.read_utf16_append(base, units, streaming),
                None => reader.read_utf16(units, streaming),
            };
            match read {
                Ok(()) => {}
                // The package's last text is not the reader's: it sends the
                // whole text instead. Nothing was staged.
                Err(ResponseReaderError::NotAnAppend) => return Ok("{\"resend\":true}".into()),
                Err(error) => {
                    return Err(OpenUiError::LimitExceeded {
                        reason: error.to_string(),
                    });
                }
            }
            Ok(reader.draw_json())
        })
    }

    /// Draw the staged read, parsing its program when `parse` asks or it
    /// changed. The read is kept only if the draw is: a refused draw leaves
    /// the reader, its parser and the session as they were.
    pub(crate) fn draw_response(
        &self,
        parse: bool,
        response: RawResponse,
        now_ms: Option<u64>,
    ) -> Result<EvaluationOutput, OpenUiError> {
        self.access(|s| {
            let response = response.read(&s.core)?;
            let Some(reader) = s.reader.as_mut() else {
                return s
                    .core
                    .draw_response_at(&s.references, None, response, now_ms);
            };
            let complete = !response.streaming;
            let core = &mut s.core;
            let references = &s.references;
            reader.try_parse(parse, complete, |parsed| {
                core.draw_response_at(
                    references,
                    parsed.map(|parsed| crate::program(parsed).0),
                    response,
                    now_ms,
                )
            })
        })
    }

    pub fn sample_query_clock(&self, now_ms: u64) -> Result<(), OpenUiError> {
        self.call(|s, _| s.sample_query_clock(now_ms))
    }
    pub fn set_query_extra_values(
        &self,
        tool_name: &str,
        values_json: &str,
    ) -> Result<(), OpenUiError> {
        self.call(|s, _| s.set_query_extra_values(tool_name, values_json))
    }
    pub fn advance_query_clock(&self, now_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.advance_query_clock(now_ms))
    }
    pub fn pause_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.pause_queries())
    }
    pub fn resume_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.resume_queries())
    }
    pub fn resume_queries_at(&self, now_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.resume_queries_at(now_ms))
    }
    pub fn attach_tool_provider_at(&self, now_ms: u64, minimum_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, refs| s.attach_tool_provider_at(refs, now_ms, minimum_ms))
    }
    pub fn attach_tool_provider_with_minimum(&self, minimum_ms: u64) -> Result<(), OpenUiError> {
        self.call(move |s, refs| s.attach_tool_provider_with_minimum(refs, minimum_ms))
    }
    pub fn attach_tool_provider(&self) -> Result<(), OpenUiError> {
        self.call(move |s, refs| s.attach_tool_provider(refs))
    }
    pub fn detach_tool_provider(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| {
            s.detach_tool_provider();
            Ok(())
        })
    }
    pub fn take_query_update(&self) -> Result<QueryEffects, OpenUiError> {
        self.call(move |s, _| Ok(s.take_query_update()))
    }
    pub fn retry_failed_queries(&self) -> Result<(), OpenUiError> {
        self.call(move |s, _| s.retry_failed_queries())
    }
    pub fn evaluate_with_response(
        &self,
        response: RawResponse,
    ) -> Result<EvaluationOutput, OpenUiError> {
        self.call(move |s, refs| {
            let response = response.read(s)?;
            s.evaluate_with_response(refs, response)
        })
    }
    pub fn complete_tool_call(
        &self,
        call: Arc<WantedToolCall>,
        outcome_json: String,
    ) -> Result<(), OpenUiError> {
        self.deliver(&call.id, outcome_json, false)
    }
    pub fn finish_tool_call(
        &self,
        call: Arc<WantedToolCall>,
        outcome_json: String,
    ) -> Result<(), OpenUiError> {
        self.deliver(&call.id, outcome_json, true)
    }
    pub(crate) fn deliver(
        &self,
        id: &openui_runtime::ToolCallId,
        json: String,
        terminal: bool,
    ) -> Result<(), OpenUiError> {
        self.call(|s, _| {
            if terminal {
                s.finish_tool_call_json(id, &json)
            } else {
                s.complete_tool_call_json(id, &json)
            }
        })
    }
    /// Report prompt annotations needing an explicit choice by the embedder.
    pub fn undecided_bindings(&self) -> Result<Vec<BindingPair>, OpenUiError> {
        self.call(|s, refs| s.undecided_bindings(refs))
    }

    /// Resolve a binding annotation explicitly; affects the next evaluation.
    pub fn mark_binding(&self, component: String, prop: String) -> Result<(), OpenUiError> {
        self.call(|s, refs| s.mark_binding(refs, component, prop))
    }

    /// Apply current declarations and optional persisted object. Omitted JSON
    /// means no persistence. JSON null is a defined value; absent keys are not.
    pub fn initialize(
        &self,
        persisted: Option<String>,
    ) -> Result<InitializationOutput, OpenUiError> {
        self.call(|s, refs| s.initialize(refs, persisted))
    }

    /// Draw explicitly. Keep the runtime's evaluated root without indexing it;
    /// action lookup walks a root-relative JSON path lazily. Only runtime limits
    /// cut a draw. A successful draw releases the preceding generation.
    pub fn evaluate(&self) -> Result<EvaluationOutput, OpenUiError> {
        self.call(|s, refs| s.evaluate(refs))
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
        self.call(|s, refs| s.action_at(refs, evaluation, path))
    }

    pub(crate) fn action_at_from_binding(
        &self,
        evaluation: Arc<EvaluationHandle>,
        path: Option<Vec<String>>,
    ) -> Result<Arc<ActionHandle>, OpenUiError> {
        self.call(|s, refs| s.action_at_from_binding(refs, evaluation, path))
    }

    /// Undefined is None; JSON null is Some("null"). State names include `$`.
    pub fn get(&self, name: String) -> Result<Option<String>, OpenUiError> {
        self.call(|s, refs| s.get(refs, name))
    }
    /// Host JSON never grants custom-step identity. None writes undefined.
    pub fn set(&self, name: String, json: Option<String>) -> Result<bool, OpenUiError> {
        self.call(|s, refs| s.set(refs, name, json))
    }
    /// JSON.stringify-compatible snapshot: undefined entries are omitted.
    pub fn snapshot(&self) -> Result<String, OpenUiError> {
        self.call(|s, refs| s.snapshot(refs))
    }
    /// Number of mutation outcomes currently retained by this session.
    pub fn pending_continuations(&self) -> Result<u64, OpenUiError> {
        self.call(|s, refs| s.pending_continuations(refs))
    }

    /// Store notification count.
    pub fn version(&self) -> Result<u64, OpenUiError> {
        self.call(|s, refs| s.version(refs))
    }
    /// Whether a later declared default may still replace this name.
    pub fn is_pristine(&self, name: String) -> Result<bool, OpenUiError> {
        self.call(|s, refs| s.is_pristine(refs, name))
    }
    /// A marker can write any state target and call declared custom actions,
    /// minting real steps for a later draw/click. Accept only marker JSON from
    /// the given tree, never model output or tool data. This call runs no host
    /// action; finished plan JSON alone cannot restore custom authority.
    pub fn write_binding(&self, marker: String, json: String) -> Result<bool, OpenUiError> {
        self.call(|s, refs| s.write_binding(refs, marker, json))
    }
    /// Read an unbound renderer form field, including wrapped values.
    pub fn get_field(
        &self,
        form: Option<String>,
        name: String,
    ) -> Result<Option<String>, OpenUiError> {
        self.call(|s, refs| s.get_field(refs, form, name))
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
        self.call(|s, refs| s.set_field(refs, form, component_type, name, json, save))
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
        self.call(|s, refs| s.initialize_field(refs, form, component_type, name, json, streaming))
    }
    /// Run a retained plan. None invokes the runtime's no-action message event.
    /// A blocked response bundle ignores the whole click.
    pub fn run(
        &self,
        action: Option<Arc<ActionHandle>>,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(|s, refs| s.run(refs, action, message, form))
    }
    /// Supply a legacy object without `steps`. Plans must use action handles;
    /// legacy names still pass the runtime's custom-action shadowing checks.
    pub fn run_legacy(
        &self,
        json: String,
        message: String,
        form: Option<String>,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(|s, refs| s.run_legacy(refs, json, message, form))
    }
    /// Consume a mutation outcome once. False ends the run. True uses current
    /// program/state/reference inputs with the original action work allowance.
    pub fn resume(
        &self,
        continuation: Arc<ContinuationHandle>,
        succeeded: bool,
    ) -> Result<ActionOutput, OpenUiError> {
        self.call(|s, refs| s.resume(refs, continuation, succeeded))
    }
}
fn busy() -> OpenUiError {
    OpenUiError::Worker {
        reason: "session already in use".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        let library = crate::ComponentLibrary::new(r#"{"$defs":{}}"#).unwrap();
        Session::new(library.shared(), None).unwrap()
    }
    #[test]
    fn reentry_and_disposal_while_borrowed_are_typed_and_leave_session_usable() {
        let session = session();
        session
            .access(|_| {
                assert!(matches!(session.version(), Err(OpenUiError::Worker { .. })));
                assert!(matches!(session.dispose(), Err(OpenUiError::Worker { .. })));
                Ok(())
            })
            .unwrap();
        assert_eq!(session.snapshot().unwrap(), "{}");
        session.dispose().unwrap();
        assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
    }
    #[test]
    fn a_caught_panic_revokes_the_session_with_a_sticky_typed_error() {
        let session = session();
        let result: Result<(), _> = session.access(|_| panic!("injected"));
        assert!(matches!(result, Err(OpenUiError::Worker { .. })));
        session.dispose().unwrap();
        assert!(matches!(session.version(), Err(OpenUiError::Worker { .. })));
    }
    #[test]
    fn exported_token_drop_releases_the_run_without_a_following_operation() {
        let library =
            crate::ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
        let session = Session::new(library.shared(), None).unwrap();
        let parsed =
            crate::parse_program("m=Mutation(\"m\",{})\nroot=Box(@Run(m))", &library, None);
        session.set_program(parsed.shared()).unwrap();
        let tree = session.evaluate().unwrap();
        let action = session
            .action_at(tree.evaluation, vec!["props".into(), "a".into()])
            .unwrap();
        let mut output = session.run(Some(action), String::new(), None).unwrap();
        let handle = crate::runtime::ContinuationHandle::owned(
            output.continuation.take().unwrap(),
            session.release_guard(),
        );
        drop(output);
        assert_eq!(session.pending_continuations().unwrap(), 1);
        drop(handle);
        // access() itself does not drain; this observes the exported destructor,
        // rather than letting pending_continuations() hide a delayed release.
        session
            .access(|s| {
                assert_eq!(s.core.drain_releases(), 0);
                Ok(())
            })
            .unwrap();
        assert_eq!(session.pending_continuations().unwrap(), 0);
        let late = session.release_guard();
        drop(session);
        drop(late);
    }
}
