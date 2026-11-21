//! Renderer action handling with ordered host handoffs and bounded continuations.
use super::*;
use std::{cell::RefCell, collections::HashMap, fmt, rc::Rc, sync::Weak};

/// One handler argument. Undefined properties are absent, as in JSON.stringify.
#[derive(Clone, Debug)]
pub struct HostEvent(Value);
impl HostEvent {
    /// The handler argument, retaining shared plan and state values.
    pub fn value(&self) -> &Value {
        &self.0
    }
    /// JSON in the renderer's handler-argument property order, without a new
    /// admission limit. The run already checked each emitted event.
    pub fn to_json(&self) -> Result<String, crate::LimitError> {
        crate::value_to_json(&self.0)
    }
    /// Serialize an event under explicit work, depth and output ceilings.
    pub fn to_json_with_limits(&self, limits: Limits) -> Result<String, crate::LimitError> {
        crate::value_to_json_with_limits(&self.0, limits)
    }
}
/// A stopped action. Earlier writes and host events are retained.
#[derive(Debug)]
#[non_exhaustive]
pub struct ActionFailure {
    /// Index in the plan, or None for the legacy/default action.
    pub step: Option<usize>,
    /// A malformed plan, failed evaluation, refused write, or cut.
    pub cause: BindingWriteError,
}
/// A mutation step awaiting dispatch to an attached tool provider.
#[derive(Debug)]
pub struct MutationRequest {
    /// Zero-based plan step.
    pub step: usize,
    /// Mutation registration to fire. A non-string target matches no registration.
    pub statement: Option<String>,
    /// Arguments evaluated at the Run step. None denotes undefined.
    pub args: Option<Value>,
}
/// Ordered effects and diagnostics from an action run or resumed segment.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ActionRun {
    /// A provider-managed suspension: step index, statement name and evaluated arguments.
    pub mutation: Option<MutationRequest>,
    memory: Option<crate::EvaluationMemory>,
    /// Handler arguments, in step order. Nothing has been sent to the host.
    pub events: Vec<HostEvent>,
    /// The first failure stops the plan; completed effects remain.
    pub failure: Option<ActionFailure>,
    /// Work or result-size exhaustion in this run.
    pub size_limited: bool,
    /// Nesting exhaustion in this run.
    pub depth_limited: bool,
    /// A mutation is awaiting its host outcome. No later step has run.
    pub continuation: Option<ActionContinuation>,
    /// With [`TreeInput::with_query_runs`], the statement id of each `@Run`
    /// step naming a query, in step order, in place of its host event. Pass
    /// each to the query manager's invalidate command on its own: an empty
    /// list there means every query.
    pub query_runs: Vec<String>,
}
impl ActionRun {
    /// Portable memory accounting for this run, including previous segments.
    /// Absent when disabled or setup refused before executing the plan.
    pub fn memory(&self) -> Option<crate::EvaluationMemory> {
        self.memory
    }
    fn record_memory(&mut self, run: &RunState) {
        self.memory = run.limits.peak_memory().and_then(|_| run.budget.memory());
    }

    /// Whether a mutation outcome is required to continue this run.
    pub fn is_suspended(&self) -> bool {
        self.continuation.is_some()
    }
    fn fail(&mut self, step: Option<usize>, cause: BindingWriteError) {
        if let BindingWriteError::Cut(flags) = &cause {
            self.size_limited = flags.size_limited;
            self.depth_limited = flags.depth_limited;
        }
        if let BindingWriteError::Store(error) = &cause {
            match error {
                crate::StoreError::DepthLimited => self.depth_limited = true,
                crate::StoreError::WorkLimited
                | crate::StoreError::ValueTooLarge
                | crate::StoreError::StateTooLarge => self.size_limited = true,
                crate::StoreError::VersionExhausted => {}
            }
        }
        self.failure = Some(ActionFailure { step, cause });
    }
}
/// An opaque, single-use suspended run. Clones refer to the same pending run.
/// It belongs to one Store session, has no JSON form, and is revoked on dispose.
#[derive(Clone)]
pub struct ActionContinuation {
    owner: Weak<()>,
    pending: Rc<RefCell<Option<RunState>>>,
}
impl fmt::Debug for ActionContinuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActionContinuation { .. }")
    }
}
impl ActionContinuation {
    /// The index of the mutation step the run is suspended at, while it is
    /// still pending (not yet resumed or revoked).
    pub fn step(&self) -> Option<usize> {
        let pending = self.pending.try_borrow().ok()?;
        pending.as_ref().map(|run| run.next.saturating_sub(1))
    }
}
/// A continuation was refused before any effects occurred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionResumeError {
    /// The owning Store was disposed or dropped.
    Disposed,
    /// The continuation belongs to another Store.
    ForeignSession,
    /// This continuation (or a clone) was already consumed.
    Consumed,
}
impl fmt::Display for ActionResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ActionResumeError {}

struct RunState {
    plan: Value,
    next: usize,
    message: String,
    form: Option<String>,
    limits: Limits,
    budget: EvaluationSession<'static>,
    defaults: Option<HashMap<String, QueryDefault>>,
    program: Option<CachedProgram>,
    started_with: Option<openui_lang::PreparedStart>,
}
/// Run a retained plan, legacy object or no action. The embedder must ignore
/// clicks while its bundled response is blocked (`bundleBlocked` in React).
/// Authority is checked in this Store; drawing remains explicit. A mutation
/// emits its evaluated args and suspends until resume_action supplies an outcome.
pub fn run_action(
    input: TreeInput<'_>,
    store: &mut Store,
    action: Option<&Value>,
    user_message: &str,
    form_name: Option<&str>,
) -> ActionRun {
    run_action_with_limits(
        input,
        store,
        action,
        user_message,
        form_name,
        Limits::DEFAULT,
    )
}
/// Run with one shared evaluation-work allowance. Input measurement is bounded
/// caller setup. The supplied-input floor is the largest combined program,
/// references and retained starting input supplied to this run. State reads
/// earn normal credit; resuming never refreshes already-spent work.
/// Output room is released after each write/event. A cut stops later steps.
pub fn run_action_with_limits(
    input: TreeInput<'_>,
    store: &mut Store,
    action: Option<&Value>,
    user_message: &str,
    form_name: Option<&str>,
    limits: Limits,
) -> ActionRun {
    let measured = prepare_references(&input);
    let input = if let Some(values) = measured.as_ref() {
        input.with_prepared_reference_values(values)
    } else {
        input
    };
    let prepare = || -> Result<RunState, BindingWriteError> {
        let mut run = RunState {
            plan: Value::Null,
            next: 0,
            message: String::new(),
            form: None,
            limits,
            budget: EvaluationSession::with_limits(&(), 0, limits),
            defaults: None,
            program: None,
            started_with: None,
        };
        RunContext::new(
            input.clone(),
            &mut run,
            false,
            input.prepared_reference_values,
        )?;
        let receipt = run.budget.with_evaluator(|e| {
            e.supply_input(openui_lang::SuppliedInput {
                program: openui_lang::SuppliedProgram::Prepared(
                    &run.program
                        .as_ref()
                        .ok_or_else(|| invalid("missing program receipt"))?
                        .input,
                ),
                start: openui_lang::SuppliedStart::Raw {
                    action,
                    message: user_message,
                    form: form_name,
                },
                references: input.prepared_reference_values.map(|r| &r.input),
            })
            .map_err(|_| BindingWriteError::Cut(e.flags()))
        })?;
        run.started_with = Some(receipt.start);
        let (message, form) = run.scope(|e| {
            Ok((
                if user_message.len() > limits.result_size()
                    && matches!(action, Some(Value::Obj(o)) if matches!(o.get("steps"), Some(Value::Arr(_))))
                {
                    // A step plan never uses the legacy/default message. Avoid
                    // retaining oversized unused text across a suspension.
                    String::new()
                } else {
                    capture_text(e, user_message, limits)?
                },
                form_name.map(|s| capture_text(e, s, limits)).transpose()?,
            ))
        })?;
        run.plan = action.cloned().unwrap_or(Value::Null);
        run.message = message;
        run.form = form;
        Ok(run)
    };
    match prepare() {
        Ok(run) => execute(input, store, run, false),
        Err(cause) => {
            let mut out = ActionRun::default();
            out.fail(None, cause);
            out
        }
    }
}
/// Consume a suspended mutation outcome. True continues against the supplied
/// current program/state; false ends the run. No fresh work allowance is granted.
/// Stale, foreign, disposed and already-consumed tokens have no effects.
pub fn resume_action(
    input: TreeInput<'_>,
    store: &mut Store,
    continuation: &ActionContinuation,
    succeeded: bool,
) -> Result<ActionRun, ActionResumeError> {
    let owner = continuation
        .owner
        .upgrade()
        .ok_or(ActionResumeError::Disposed)?;
    if !Arc::ptr_eq(&owner, &store.action_session) {
        return Err(ActionResumeError::ForeignSession);
    }
    let run = continuation
        .pending
        .try_borrow_mut()
        .map_err(|_| ActionResumeError::Consumed)?
        .take()
        .ok_or(ActionResumeError::Consumed)?;
    if !succeeded {
        let mut out = ActionRun::default();
        out.record_memory(&run);
        return Ok(out);
    }
    Ok(execute(input, store, run, true))
}
fn checked_flags(session: &EvaluationSession<'_>) -> Result<(), BindingWriteError> {
    let flags = session.flags();
    if flags.size_limited || flags.depth_limited {
        Err(BindingWriteError::Cut(flags))
    } else {
        Ok(())
    }
}
struct CachedProgram {
    owner: Option<Weak<ParseResult>>,
    input: openui_lang::PreparedProgram,
    references: ReferenceLocations,
    mutations: Rc<HashMap<String, usize>>,
}
struct RunContext<'a> {
    input: TreeInput<'a>,
    references: ReferenceIndex<'a>,
    mutations: Rc<HashMap<String, usize>>,
}
impl<'a> RunContext<'a> {
    fn new(
        input: TreeInput<'a>,
        run: &mut RunState,
        initial: bool,
        supplied: Option<&PreparedReferenceValues<'_>>,
    ) -> Result<Self, BindingWriteError> {
        let shared = input
            .shared_result
            .filter(|p| std::ptr::eq(p.as_ref(), input.result));
        let reused = initial && run.program.is_some()
            || match (&shared, &run.program) {
                (Some(p), Some(cache)) => cache
                    .owner
                    .as_ref()
                    .is_some_and(|old| old.strong_count() != 0 && old.ptr_eq(&Arc::downgrade(p))),
                _ => false,
            };
        if !reused {
            let prepared = openui_lang::PreparedProgram::new(input.result, run.limits);
            let mut references = HashMap::new();
            let mut mutations = HashMap::new();
            for (i, q) in input.result.query_statements.iter().enumerate() {
                references
                    .entry(q.statement_id.clone())
                    .or_insert(ReferenceLocation::Query(i));
            }
            for (i, m) in input.result.mutation_statements.iter().enumerate() {
                references
                    .entry(m.statement_id.clone())
                    .or_insert(ReferenceLocation::Mutation);
                mutations.entry(m.statement_id.clone()).or_insert(i);
            }
            run.program = Some(CachedProgram {
                owner: shared.map(Arc::downgrade),
                input: prepared,
                references: Rc::new(references),
                mutations: Rc::new(mutations),
            });
        }
        let Some(cache) = &run.program else {
            return Err(invalid("program input was not admitted"));
        };
        if let Some(start) = &run.started_with {
            run.budget.with_evaluator(|e| {
                e.supply_input(openui_lang::SuppliedInput {
                    program: openui_lang::SuppliedProgram::Prepared(&cache.input),
                    start: openui_lang::SuppliedStart::Prepared(start),
                    references: supplied.map(|r| &r.input),
                })
                .map_err(|_| BindingWriteError::Cut(e.flags()))
            })?;
        }
        Ok(Self {
            references: ReferenceIndex {
                program: input.result,
                entries: cache.references.clone(),
            },
            mutations: cache.mutations.clone(),
            input,
        })
    }
    fn evaluate(
        &self,
        run: &mut RunState,
        store: &Store,
        expr: bindings::KeptExpression<'_>,
        supplied: Option<&PreparedReferenceValues<'_>>,
    ) -> Result<crate::StateValue, BindingWriteError> {
        if run.defaults.is_none() {
            let scope = TreeScope::with_references(
                &self.input,
                store,
                supplied,
                run.limits,
                self.references.clone(),
            );
            run.budget.with_scope(&scope, |s| {
                s.with_evaluator(|e| scope.initialize_queries(self.input.result, e))
            });
            checked_flags(&run.budget)?;
            run.defaults = Some(
                scope
                    .defaults
                    .into_inner()
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect(),
            );
        }
        let mut scope = TreeScope::with_references(
            &self.input,
            store,
            supplied,
            run.limits,
            self.references.clone(),
        );
        scope.run_defaults = run.defaults.as_ref();
        scope.registering.set(false);
        let result = run.budget.with_scope(&scope, |s| {
            s.with_evaluator(|e| {
                e.with_call_context(false, || match expr {
                    bindings::KeptExpression::Ast(a) => e.evaluate_result(a),
                    bindings::KeptExpression::Value(Value::Ast(a)) => e.evaluate_result(a),
                    bindings::KeptExpression::Value(v) => e.evaluate_value(v),
                })
            })
        });
        checked_flags(&run.budget)?;
        let result = result.ok_or_else(|| BindingWriteError::Cut(run.budget.flags()))?;
        if let Some(failure) = result.failure {
            return Err(BindingWriteError::Evaluation(failure));
        }
        Ok(result.value)
    }
}
fn prepare_references<'a>(input: &TreeInput<'a>) -> Option<PreparedReferenceValues<'a>> {
    if input.prepared_reference_values.is_none() {
        input.reference_values.map(PreparedReferenceValues::new)
    } else {
        None
    }
}
fn execute(
    input: TreeInput<'_>,
    store: &mut Store,
    mut run: RunState,
    resuming: bool,
) -> ActionRun {
    let mut out = ActionRun::default();
    let measured = prepare_references(&input);
    let supplied = input.prepared_reference_values.or(measured.as_ref());
    let context = match RunContext::new(input, &mut run, !resuming, supplied) {
        Ok(context) => context,
        Err(cause) => {
            out.fail(resuming.then_some(run.next), cause);
            out.record_memory(&run);
            return out;
        }
    };
    if !resuming && (!truthy(&run.plan) || !matches!(&run.plan,Value::Obj(o) if o.has("steps"))) {
        if let Err(cause) = run.legacy(&context.input, store, &mut out) {
            out.fail(None, cause);
        }
        out.record_memory(&run);
        return out;
    }
    let plan = run.plan.clone();
    let fields = if let Value::Obj(fields) = &plan {
        Some(fields.as_ref())
    } else {
        None
    };
    let Some(steps) = fields.and_then(|o| o.get("steps")).filter(|v| truthy(v)) else {
        match run.default_event(store) {
            Ok(e) => out.events.push(e),
            Err(cause) => out.fail(None, cause),
        };
        out.record_memory(&run);
        return out;
    };
    let steps = match steps {
        Value::Arr(a) => a,
        Value::Str(_) => {
            out.record_memory(&run);
            return out;
        }
        _ => {
            out.fail(None, invalid("actionPlan.steps is not iterable"));
            out.record_memory(&run);
            return out;
        }
    };
    out.record_memory(&run);
    for (index, step) in steps.iter().enumerate().skip(run.next) {
        let result = run
            .budget
            .with_scope(&(), |s| s.with_evaluator(|e| e.charge_work(1)));
        if !result {
            out.fail(Some(index), BindingWriteError::Cut(run.budget.flags()));
            break;
        }
        run.next = index;
        let result = run.step(&context, store, step, supplied, &mut out);
        out.record_memory(&run);
        match result {
            Ok(true) => {
                run.next = index.saturating_add(1);
                out.continuation = Some(ActionContinuation {
                    owner: Arc::downgrade(&store.action_session),
                    pending: Rc::new(RefCell::new(Some(run))),
                });
                break;
            }
            Ok(false) => {}
            Err(cause) => {
                out.fail(Some(index), cause);
                break;
            }
        }
    }
    out
}
impl RunState {
    fn scope<T>(
        &self,
        f: impl FnOnce(&ExpressionEvaluator<'_>) -> Result<T, BindingWriteError>,
    ) -> Result<T, BindingWriteError> {
        self.budget.with_scope(&(), |s| s.with_evaluator(f))
    }
    fn clone_field(
        &self,
        fields: &JsObject<Value>,
        key: &str,
    ) -> Result<crate::StateValue, BindingWriteError> {
        self.scope(|e| {
            fields
                .get(key)
                .map(|v| clone_in(e, v, self.limits))
                .transpose()
        })
    }
    fn event(
        &self,
        store: &Store,
        kind: Option<Value>,
        params: Option<Value>,
        message: Option<Value>,
    ) -> Result<HostEvent, BindingWriteError> {
        let value = self.scope(|eval| {
            if !eval.reserve_container_storage(5) {
                return Err(BindingWriteError::Cut(eval.flags()));
            }
            let mut out = JsObject::new();
            if let Some(v) = kind {
                out.define("type", v);
            }
            if let Some(v) = params {
                out.define("params", v);
            }
            if let Some(v) = message {
                out.define("humanFriendlyMessage", v);
            }
            out.define(
                "formState",
                form::form_payload(store, self.form.as_deref(), self.limits, eval)?,
            );
            if let Some(name) = &self.form {
                out.define("formName", Value::Str(copy_in(eval, name, self.limits)?));
            }
            Ok(Value::Obj(Arc::new(out)))
        })?;
        self.scope(|e| {
            if e.check_result_value(&value) {
                Ok(())
            } else {
                Err(BindingWriteError::Cut(e.flags()))
            }
        })?;
        Ok(HostEvent(value))
    }
    fn default_event(&self, store: &Store) -> Result<HostEvent, BindingWriteError> {
        let message = self.scope(|e| copy_in(e, &self.message, self.limits))?;
        self.event(
            store,
            Some(text("continue_conversation")),
            Some(object([])),
            Some(Value::Str(message)),
        )
    }
    fn legacy(
        &self,
        input: &TreeInput<'_>,
        store: &Store,
        out: &mut ActionRun,
    ) -> Result<(), BindingWriteError> {
        if !truthy(&self.plan) {
            out.events.push(self.default_event(store)?);
            return Ok(());
        }
        if !is_object(&self.plan) && !matches!(&self.plan, Value::Arr(_)) {
            return Err(invalid("action is not an object"));
        }
        let mut typed = JsObject::new();
        if matches!(&self.plan, Value::Element(_)) {
            typed.define("type", text("element"));
        }
        let fields = if let Value::Obj(o) = &self.plan {
            o.as_ref()
        } else {
            &typed
        };
        let kind = self
            .clone_field(fields, "type")?
            .filter(truthy)
            .unwrap_or_else(|| text("continue_conversation"));
        if let Some(actions) = input.actions.filter(|v| json_truthy(v)) {
            let shadowed = self.scope(|e| {
                let key = e.coerce_string(&kind).ok_or_else(|| {
                    let f = e.flags();
                    if f.size_limited || f.depth_limited {
                        BindingWriteError::Cut(f)
                    } else {
                        BindingWriteError::Evaluation(EvaluationFailure::CannotConvertToPrimitive)
                    }
                })?;
                action_has_own(actions, &key, e)
            })?;
            if shadowed {
                return Ok(());
            }
        }
        let mut params = self.scope(|e| spread(fields.get("params"), self.limits, e))?;
        for key in ["url", "context"] {
            if let Some(v) = fields.get(key).filter(|v| truthy(v)) {
                params.set(key, self.scope(|e| clone_in(e, v, self.limits))?);
            }
        }
        let message = self.scope(|e| copy_in(e, &self.message, self.limits))?;
        out.events.push(self.event(
            store,
            Some(kind),
            Some(Value::Obj(Arc::new(params))),
            Some(Value::Str(message)),
        )?);
        Ok(())
    }
    fn step(
        &mut self,
        context: &RunContext<'_>,
        store: &mut Store,
        step: &Value,
        supplied: Option<&PreparedReferenceValues<'_>>,
        out: &mut ActionRun,
    ) -> Result<bool, BindingWriteError> {
        if matches!(step, Value::Null) {
            return Err(invalid("null is not an object (evaluating 'step.type')"));
        }
        let Value::Obj(fields) = step else {
            return Ok(false);
        };
        let kind = if let Some(Value::Str(s)) = fields.get("type") {
            s.as_str()
        } else {
            return Ok(false);
        };
        match kind {
            "set" => self.set(context, store, fields, supplied)?,
            "reset" => self.reset(&context.input, store, fields)?,
            "run" => return self.run(context, store, fields, supplied, out),
            "continue_conversation" => {
                let mut params = JsObject::new();
                if let Some(v) = self.clone_field(fields, "context")? {
                    params.define("context", v);
                }
                out.events.push(self.event(
                    store,
                    Some(text(kind)),
                    Some(Value::Obj(Arc::new(params))),
                    self.clone_field(fields, "message")?,
                )?);
            }
            "open_url" => {
                let mut params = JsObject::new();
                if let Some(v) = self.clone_field(fields, "url")? {
                    params.define("url", v);
                }
                out.events.push(self.event(
                    store,
                    Some(text(kind)),
                    Some(Value::Obj(Arc::new(params))),
                    Some(text("")),
                )?);
            }
            "custom_action" if store.actions.owns(step) => out.events.push(self.event(
                store,
                self.clone_field(fields, "name")?,
                self.clone_field(fields, "params")?,
                Some(text("")),
            )?),
            _ => {}
        }
        Ok(false)
    }
    fn set(
        &mut self,
        context: &RunContext<'_>,
        store: &mut Store,
        fields: &JsObject<Value>,
        supplied: Option<&PreparedReferenceValues<'_>>,
    ) -> Result<(), BindingWriteError> {
        let Some(expr) = fields.get("valueAST").filter(|v| truthy(v)) else {
            return Ok(());
        };
        let value = context.evaluate(self, store, expr.into(), supplied)?;
        let Some(Value::Str(target)) = fields.get("target") else {
            return Err(BindingWriteError::InvalidMarker);
        };
        store.set(target, value).map_err(BindingWriteError::Store)?;
        Ok(())
    }
    fn reset(
        &self,
        input: &TreeInput<'_>,
        store: &mut Store,
        fields: &JsObject<Value>,
    ) -> Result<(), BindingWriteError> {
        let Some(targets) = fields.get("targets") else {
            return Err(invalid("step.targets is not iterable"));
        };
        let writes = self.scope(|e| {
            let mut writes = Vec::new();
            if let Value::Arr(targets) = targets {
                for target in targets.iter() {
                    if !e.charge_work(1) {
                        return Err(BindingWriteError::Cut(e.flags()));
                    }
                    if !matches!(target, Value::Str(_)) {
                        return Err(BindingWriteError::InvalidMarker);
                    }
                }
            }
            let mut reset = |target: &Value| -> Result<(), BindingWriteError> {
                let Value::Str(target) = target else {
                    return Err(BindingWriteError::InvalidMarker);
                };
                let target = e
                    .copy_text(target)
                    .ok_or_else(|| BindingWriteError::Cut(e.flags()))?;
                let fallback = if target == "__proto__" {
                    Some(object([]))
                } else if !input.result.state_declarations.has(&target) {
                    Some(Value::Null)
                } else {
                    None
                };
                let value = input
                    .result
                    .state_declarations
                    .get(&target)
                    .or(fallback.as_ref());
                if let Some(v) = value
                    && !e.check_result_value(v)
                {
                    return Err(BindingWriteError::Cut(e.flags()));
                }
                writes.push((target, value.cloned()));
                Ok(())
            };
            match targets {
                Value::Arr(a) => {
                    for t in a.iter() {
                        reset(t)?;
                    }
                }
                Value::Str(s) => {
                    for t in s.chars() {
                        reset(&text(&t.to_string()))?;
                    }
                }
                _ => return Err(invalid("step.targets is not iterable")),
            }
            Ok(writes)
        })?;
        store.reset_batch(writes).map_err(BindingWriteError::Store)
    }
    fn run(
        &mut self,
        context: &RunContext<'_>,
        store: &Store,
        fields: &JsObject<Value>,
        supplied: Option<&PreparedReferenceValues<'_>>,
        out: &mut ActionRun,
    ) -> Result<bool, BindingWriteError> {
        let mutation = matches!(fields.get("refType"),Some(Value::Str(t))if t=="mutation");
        let id = match fields.get("statementId") {
            Some(Value::Str(id)) => Some(id.as_str()),
            _ => None,
        };
        if mutation && context.input.managed_mutations {
            let args = self.mutation_args(context, store, id, supplied)?;
            let id = id
                .map(|name| self.scope(|e| copy_in(e, name, self.limits)))
                .transpose()?;
            out.mutation = Some(MutationRequest {
                step: self.next,
                statement: id,
                args,
            });
            return Ok(true);
        }
        if !mutation && context.input.query_runs {
            // Upstream's renderer calls invalidate([step.statementId]): a
            // missing or non-string id names no query and runs nothing.
            if let Some(id) = id {
                let id = self.scope(|e| {
                    e.copy_text(id)
                        .ok_or_else(|| BindingWriteError::Cut(e.flags()))
                })?;
                out.query_runs.push(id);
            }
            return Ok(false);
        }
        self.scope(|e| {
            if e.reserve_container_storage(3) {
                Ok(())
            } else {
                Err(BindingWriteError::Cut(e.flags()))
            }
        })?;
        let mut params = JsObject::new();
        if let Some(id) = self.clone_field(fields, "statementId")? {
            params.define("statementId", id);
        }
        params.define("refType", text(if mutation { "mutation" } else { "query" }));
        if mutation {
            let args = self.mutation_args(context, store, id, supplied)?;
            if let Some(args) = args {
                params.define("args", args);
            }
        }
        out.events.push(self.event(
            store,
            Some(text("run")),
            Some(Value::Obj(Arc::new(params))),
            Some(text("")),
        )?);
        Ok(mutation)
    }
    fn mutation_args(
        &mut self,
        context: &RunContext<'_>,
        store: &Store,
        id: Option<&str>,
        supplied: Option<&PreparedReferenceValues<'_>>,
    ) -> Result<Option<Value>, BindingWriteError> {
        if let Some(expr) = id
            .and_then(|id| context.mutations.get(id))
            .and_then(|index| context.input.result.mutation_statements.get(*index))
            .and_then(|m| m.args.as_ref())
        {
            context.evaluate(self, store, expr.into(), supplied)
        } else {
            Ok(Some(object([])))
        }
    }
}
fn text(value: &str) -> Value {
    Value::Str(value.into())
}
fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut out = JsObject::new();
    for (k, v) in fields {
        out.define(k, v);
    }
    Value::Obj(Arc::new(out))
}
fn is_object(v: &Value) -> bool {
    matches!(v, Value::Obj(_) | Value::Element(_) | Value::Ast(_))
}
pub(super) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Num(n) => *n != 0.0 && !n.is_nan(),
        Value::Str(s) => !s.is_empty(),
        _ => true,
    }
}
fn invalid(message: &str) -> BindingWriteError {
    BindingWriteError::Evaluation(EvaluationFailure::Callback(message.into()))
}
pub(super) fn sizing_cut(depth: bool) -> BindingWriteError {
    let mut flags = crate::EvaluationFlags::default();
    flags.size_limited = !depth;
    flags.depth_limited = depth;
    BindingWriteError::Cut(flags)
}
pub(super) fn reserve_text(eval: &ExpressionEvaluator<'_>, text: &str, limits: Limits) -> bool {
    match input::text_size(text, limits) {
        Ok(bytes) => eval.reserve_result(bytes),
        Err(depth) => {
            if let BindingWriteError::Cut(flags) = sizing_cut(depth) {
                eval.mark_cut(flags);
            }
            false
        }
    }
}
pub(super) fn copy_text(text: &str, limits: Limits) -> Result<String, BindingWriteError> {
    let bytes = input::text_size(text, limits).map_err(sizing_cut)?;
    let session = EvaluationSession::with_limits(&(), bytes, limits);
    session
        .with_evaluator(|eval| {
            if eval.reserve_result(bytes) {
                eval.copy_text(text)
            } else {
                None
            }
        })
        .ok_or_else(|| BindingWriteError::Cut(session.flags()))
}
pub(super) fn admit(v: &Value, limits: Limits) -> Result<(), BindingWriteError> {
    let bytes = input::value_size(
        v,
        limits,
        &mut Default::default(),
        &mut limits.work_for(usize::MAX),
    )
    .map_err(sizing_cut)?;
    let session = EvaluationSession::with_limits(&(), bytes, limits);
    if session.with_evaluator(|e| e.reserve_value(v)) {
        Ok(())
    } else {
        Err(BindingWriteError::Cut(session.flags()))
    }
}
// Object spread defines own __proto__; it does not invoke the prototype setter.
fn spread(
    v: Option<&Value>,
    limits: Limits,
    eval: &ExpressionEvaluator<'_>,
) -> Result<JsObject<Value>, BindingWriteError> {
    let mut out = JsObject::new();
    let admitted = (|| {
        if !eval.reserve_result(2) {
            return false;
        }
        if let Some(Value::Str(s)) = v {
            for (i, unit) in openui_lang::utf16_units(s).enumerate() {
                let value = Value::Str(openui_lang::utf16_unit_string(unit));
                if !eval.reserve_result(2) || !eval.reserve_value(&value) {
                    return false;
                }
                let key = i.to_string();
                if !reserve_text(eval, &key, limits) {
                    return false;
                }
                let Some(key) = eval.copy_text(&key) else {
                    return false;
                };
                out.define(key, value);
            }
        } else if let Some(v) = v {
            for (key, value) in own_fields(v).iter() {
                if !eval.reserve_result(2) || !eval.reserve_value(value) {
                    return false;
                }
                if !reserve_text(eval, key, limits) {
                    return false;
                }
                let Some(key) = eval.copy_text(key) else {
                    return false;
                };
                out.define(key, value.clone());
            }
        }
        true
    })();
    if admitted {
        Ok(out)
    } else {
        Err(BindingWriteError::Cut(eval.flags()))
    }
}
// Captured caller text is retained input, not a write's output reservation.
// Bound the allocation before copying, even when the eventual event cuts.
fn capture_text(
    eval: &ExpressionEvaluator<'_>,
    text: &str,
    limits: Limits,
) -> Result<String, BindingWriteError> {
    if text.len() > limits.result_size() {
        return Err(sizing_cut(false));
    }
    eval.copy_text(text)
        .ok_or_else(|| BindingWriteError::Cut(eval.flags()))
}
fn copy_in(
    eval: &ExpressionEvaluator<'_>,
    text: &str,
    limits: Limits,
) -> Result<String, BindingWriteError> {
    if !reserve_text(eval, text, limits) {
        return Err(BindingWriteError::Cut(eval.flags()));
    }
    eval.copy_text(text)
        .ok_or_else(|| BindingWriteError::Cut(eval.flags()))
}
fn clone_in(
    eval: &ExpressionEvaluator<'_>,
    value: &Value,
    limits: Limits,
) -> Result<Value, BindingWriteError> {
    if let Value::Str(s) = value {
        Ok(Value::Str(copy_in(eval, s, limits)?))
    } else {
        Ok(value.clone())
    }
}
fn json_truthy(value: &openui_lang::JsonValue) -> bool {
    use openui_lang::JsonValue as J;
    match value {
        J::Null => false,
        J::Bool(b) => *b,
        J::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        J::String(s) => !s.is_empty(),
        _ => true,
    }
}
fn action_has_own(
    actions: &openui_lang::JsonValue,
    key: &str,
    eval: &ExpressionEvaluator<'_>,
) -> Result<bool, BindingWriteError> {
    use openui_lang::JsonValue as J;
    Ok(match actions {
        J::Object(o) => o.contains_key(key),
        J::Array(a) => {
            key == "length"
                || key
                    .parse::<usize>()
                    .ok()
                    .is_some_and(|i| i < a.len() && i.to_string() == key)
        }
        J::String(s) => {
            if key == "length" {
                true
            } else if let Some(index) = key
                .parse::<usize>()
                .ok()
                .filter(|i| i.to_string() == key && *i < s.len())
            {
                let found = (|| {
                    let mut units = openui_lang::utf16_units(s);
                    for _ in 0..=index {
                        if !eval.charge_work(1) {
                            return None;
                        }
                        if units.next().is_none() {
                            return Some(false);
                        }
                    }
                    Some(true)
                })();
                found.ok_or_else(|| BindingWriteError::Cut(eval.flags()))?
            } else {
                false
            }
        }
        _ => false,
    })
}
