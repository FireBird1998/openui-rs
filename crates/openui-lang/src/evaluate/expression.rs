//! Public expression evaluation, separate from parsing's entry and caches.

use super::*;
use crate::Limits;
use std::rc::Rc;
use std::sync::Weak;

/// Names and calls supplied by the caller. State names include `$`.
///
/// `None` is JavaScript undefined; `Some(Value::Null)` is explicit null.
/// Extra scope falls back to state for both undefined and null. References
/// and state otherwise preserve undefined, as upstream's evaluator does.
/// Lookups are lazy and cached for an allowance, so they must stay stable.
///
/// Every callback receives an evaluator sharing the allowance. Lookup callbacks
/// (`state`, `reference`, `extra`) evaluate at the root, without enclosing
/// `@Each` bindings. Call callbacks retain those bindings for their arguments.
/// Session re-entry follows the same rule.
///
/// Fresh public evaluations have separate work and room. Collect their result
/// flags explicitly: returning only `.value` loses work and result-size cuts.
/// Depth restrictions and their cut flag are shared across the active chain.
/// Prefer the callback evaluator or session to share every limit and flag.
/// Supplied state and extra values raise scaled limits as described in
/// [`EvaluationSession`]. Function results and substituted arguments are
/// output, not new input; reading function results costs shared work and depth.
/// Callback work and allocations outside the evaluator remain the caller's.
/// Evaluator, stall and stack context restore on return and unwind.
pub trait EvaluationScope {
    /// Read supplied state; `None` means undefined.
    /// Return supplied data, never evaluator output: these values earn input
    /// credit. Use `reference` or `call` for computed results.
    fn state(&self, _name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        None
    }
    /// Resolve another statement, using the shared evaluator when needed.
    /// Results never add input credit. Include external reference data, such
    /// as query results, in the session's initial `input_bytes`.
    fn reference(&self, _name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        None
    }
    /// Supplied extra scope such as `$value`; null and undefined fall back to
    /// state. Return supplied data, never evaluator output: it earns input credit.
    fn extra(&self, _name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        None
    }
    /// Handle a non-builtin call with unevaluated, already substituted args.
    /// An unimplemented call defaults to null; a handler can return undefined.
    fn call(&self, _call: &Ast, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
}
impl EvaluationScope for () {}

/// Evaluation within a callback, with the same scope, depth and allowance.
pub struct ExpressionEvaluator<'a> {
    evaluate: &'a dyn Fn(&Ast) -> EvaluationResult,
}
impl ExpressionEvaluator<'_> {
    /// Evaluate a statement or argument; `None` denotes undefined.
    pub fn evaluate(&self, expression: &Ast) -> Option<Value> {
        self.evaluate_result(expression).value
    }
    /// Evaluate and inspect the limits affecting this evaluation.
    pub fn evaluate_result(&self, expression: &Ast) -> EvaluationResult {
        (self.evaluate)(expression)
    }
}

/// Limits reached by any evaluation sharing an allowance. Flags are sticky.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EvaluationFlags {
    /// Work or result room was exhausted, or a stall stopped evaluation.
    pub size_limited: bool,
    /// Logical depth or the grown-stack ceiling was reached.
    pub depth_limited: bool,
}

/// One expression's result, with flags for cuts affecting that evaluation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EvaluationResult {
    /// Undefined is `None`; null is `Some(Value::Null)`. Deferred ASTs and
    /// caller-owned containers retain their types and ownership.
    pub value: Option<Value>,
    /// Work or result room was exhausted, or a stall stopped evaluation.
    pub size_limited: bool,
    /// Logical depth or the grown-stack ceiling was reached.
    pub depth_limited: bool,
}

/// Several expression evaluations sharing work, result room and input credits.
///
/// A session is bound to one scope and caches lookups by name. Keep that scope
/// stable. Writing through a binding with `$value`, or running an action plan,
/// uses a new session over a scope that wraps the original scope.
///
/// `input_bytes` supplies the program or parse result's size once, including
/// any external data that references will return, such as query results.
/// Zero means zero prepaid input; expressions are never measured by a session.
/// State and extra values actually read add their JSON bytes, counting each
/// shared container body once per allowance. Keys, punctuation and scalar
/// occurrences count within it. Unread values add none. Reference results,
/// function results and substituted arguments never add input credit.
///
/// Work and result limits scale with that total under ADR 0001, retaining the
/// configured floors and ceilings. Fixed limits stay fixed. Thus output is
/// bounded by the amplification of the initial input plus state and extra
/// values read, not by the number of evaluations. Measuring input costs work
/// and depth. A deferred AST must fit its sizing walk within the work remaining
/// before it earns credit; raise `Limits::with_work` for larger deferred ASTs.
/// Already parsed inputs are not subject to the program-text size limit.
///
/// Each evaluation reports its cuts; [`Self::flags`] reports the whole session.
/// After a cut, later evaluations return a flagged null. Sequential evaluations
/// close their depth; references and callbacks share the depth while nested.
/// Fresh nested sessions inherit depth already used on this thread. Every
/// expression hop counts, including a resumed allowance inside a fresh one;
/// the tightest active depth limit applies to the entire nested chain. Context,
/// including stall flags and stack ceilings, restores on unwind (ADR 0002).
pub struct EvaluationSession<'a> {
    names: Supplied<'a>,
    saved: RefCell<Saved>,
    flags: Rc<Cell<EvaluationFlags>>,
}

impl<'a> EvaluationSession<'a> {
    /// Start an allowance with default limits and the initial input's byte size.
    /// Include the program and external reference data in `input_bytes`.
    /// Reference and call results never increase the allowance. Zero is valid
    /// and uses only the configured floors until state or extra scope is read.
    pub fn new(scope: &'a dyn EvaluationScope, input_bytes: usize) -> Self {
        Self::with_limits(scope, input_bytes, Limits::DEFAULT)
    }
    /// Start an allowance with supplied limits and the parsed input's byte size.
    pub fn with_limits(scope: &'a dyn EvaluationScope, input_bytes: usize, limits: Limits) -> Self {
        let mut saved = Saved::enter();
        LIMITS.with(|l| {
            l.room.set(limits.result_size_for(input_bytes));
            l.work.set(limits.work_for(input_bytes));
            l.depth.set(0);
            l.max_depth
                .set(limits.depth().saturating_mul(VALUE_LEVELS_PER_LEVEL));
            l.size_limited.set(false);
            l.depth_limited.set(false);
            l.cut.set(None);
        });
        ACCOUNT.with(|a| *a.borrow_mut() = Account::new(limits, input_bytes));
        let flags = ACCOUNT.with(|a| a.borrow().flags.clone());
        saved.swap();
        saved.restore = false;
        Self {
            names: Supplied {
                scope,
                read: RefCell::new(HashMap::new()),
                callbacks: RefCell::new(Vec::new()),
            },
            saved: RefCell::new(saved),
            flags,
        }
    }
    /// Evaluate another expression under the same allowance, including when
    /// re-entered from this session's own callback. Re-entry uses the callback's
    /// caches, remaining work and room, and current depth. Only call callbacks
    /// retain enclosing loop bindings; lookup callbacks use the root.
    pub fn evaluate(&self, expression: &Ast) -> EvaluationResult {
        let frame = self.names.callbacks.borrow().last().cloned();
        if let Some(frame) = frame {
            return self.names.resume(&frame, expression);
        }
        let outer = DepthContext::current();
        let _resume = Resume::enter(&self.saved);
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        evaluate_result(expression, &Scope::Root(&self.names))
    }
    /// The aggregate cut flags, including cuts preceding the latest evaluation.
    pub fn flags(&self) -> EvaluationFlags {
        self.flags.get()
    }
}

/// Evaluate one expression under default limits and a fresh allowance.
/// The initial input size is the root expression's AST JSON byte length.
pub fn evaluate_expression(expression: &Ast, scope: &dyn EvaluationScope) -> EvaluationResult {
    evaluate_expression_with_limits(expression, scope, Limits::DEFAULT)
}
/// Evaluate once with supplied limits. See [`EvaluationSession`] for the input
/// scaling rule and for evaluating many props under one allowance.
pub fn evaluate_expression_with_limits(
    expression: &Ast,
    scope: &dyn EvaluationScope,
    limits: Limits,
) -> EvaluationResult {
    let session = EvaluationSession::with_limits(scope, 0, limits);
    {
        let outer = DepthContext::current();
        let _resume = Resume::enter(&session.saved);
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        let upper = limits.work_for(usize::MAX);
        LIMITS.with(|l| l.work.set(upper));
        let bytes = expression_bytes(expression);
        let spent = LIMITS.with(|l| upper.saturating_sub(l.work.get()));
        ACCOUNT.with(|a| a.borrow_mut().bytes = bytes);
        LIMITS.with(|l| {
            l.room.set(limits.result_size_for(bytes));
            let allowed = limits.work_for(bytes);
            l.work.set(allowed.saturating_sub(spent));
            if spent > allowed {
                l.cut_by(Cut::Work);
            }
        });
    }
    session.evaluate(expression)
}

fn evaluate_result(expression: &Ast, scope: &Scope<Supplied<'_>>) -> EvaluationResult {
    let js = evaluate(expression, scope);
    let value = runtime_value(&js);
    if let Some(v) = &value {
        remember_result(v, &js);
    }
    result(value)
}
fn result(value: Option<Value>) -> EvaluationResult {
    LIMITS.with(|l| {
        if l.depth_limited.get() {
            DEPTH.with(|d| d.borrow().cut.set(true));
        }
        if DEPTH.with(|d| d.borrow().cut.get()) {
            l.cut_by(Cut::Depth);
        }
        EvaluationResult {
            value,
            size_limited: l.size_limited.get() || crate::progress::stalled_so_far(),
            depth_limited: l.depth_limited.get(),
        }
    })
}

struct Supplied<'a> {
    scope: &'a dyn EvaluationScope,
    read: RefCell<HashMap<(bool, String), Js>>,
    callbacks: RefCell<Vec<Rc<Suspension>>>,
}
struct Suspension {
    saved: RefCell<Saved>,
    bindings: Vec<(String, Js)>,
    flags: Rc<Cell<EvaluationFlags>>,
}
struct CallbackGuard<'a>(&'a RefCell<Vec<Rc<Suspension>>>);
impl Drop for CallbackGuard<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().pop();
    }
}
fn in_scope(
    node: &Ast,
    bindings: &[(String, Js)],
    scope: &Scope<Supplied<'_>>,
) -> EvaluationResult {
    crate::stack::grow_stack!(in_scope(node, bindings, scope), budget else result(Some(Value::Null)));
    if let Some(((name, item), rest)) = bindings.split_first() {
        if !step(1) {
            return evaluate_result(&Ast::Null, scope);
        }
        in_scope(
            node,
            rest,
            &Scope::Each {
                name,
                item,
                parent: scope,
            },
        )
    } else {
        evaluate_result(node, scope)
    }
}
impl Supplied<'_> {
    fn callback(
        &self,
        scope: &Scope<Self>,
        callback: impl FnOnce(&ExpressionEvaluator<'_>) -> Option<Value>,
    ) -> Option<Value> {
        let mut bindings = Vec::new();
        let mut parent = scope;
        while let Scope::Each {
            name,
            item,
            parent: next,
        } = parent
        {
            if !step(name.len().saturating_add(1)) {
                return Some(Value::Null);
            }
            bindings.push(((*name).to_owned(), (*item).clone()));
            parent = next;
        }
        bindings.reverse();
        let flags = ACCOUNT.with(|a| a.borrow().flags.clone());
        let frame = Rc::new(Suspension {
            saved: RefCell::new(Saved::enter()),
            bindings,
            flags,
        });
        self.callbacks.borrow_mut().push(frame.clone());
        let guard = CallbackGuard(&self.callbacks);
        DepthContext::suspend(&frame.saved.borrow());
        let eval = |node: &Ast| self.resume(&frame, node);
        let value = callback(&ExpressionEvaluator { evaluate: &eval });
        if DEPTH.with(|d| d.borrow().cut.get()) {
            frame.saved.borrow().limits.cut_by(Cut::Depth);
        }
        drop(guard);
        drop(frame);
        ACCOUNT.with(|a| a.borrow_mut().collect_bridges());
        value
    }
    fn resume(&self, frame: &Suspension, node: &Ast) -> EvaluationResult {
        let outer = DepthContext::current();
        let active = ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &frame.flags));
        // The saved frame holds this allowance while suspended, and the
        // callback's context while resumed. The session's name cache is shared.
        let _resume = if active {
            None
        } else {
            Some(Resume::enter(&frame.saved))
        };
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        in_scope(node, &frame.bindings, &Scope::Root(self))
    }
    fn get(&self, name: &str, reference: bool) -> Js {
        let key = (reference, name.to_owned());
        if let Some(js) = self.read.borrow().get(&key) {
            return js.clone();
        }
        let value = self.callback(&Scope::Root(self), |eval| {
            if reference {
                self.scope.reference(name, eval)
            } else {
                self.scope
                    .extra(name, eval)
                    .filter(|v| !matches!(v, Value::Null))
                    .or_else(|| self.scope.state(name, eval))
            }
        });
        if let Some(value) = &value {
            if reference {
                measure_data(value, false);
            } else {
                register_input(value);
            }
        }
        let js = value.as_ref().map(data_value).unwrap_or(Js::Undefined);
        self.read.borrow_mut().insert(key, js.clone());
        js
    }
}
impl Names for Supplied<'_> {
    const RUNTIME: bool = true;
    fn state(&self, name: &str, _scope: &Scope<Self>) -> Option<Js> {
        Some(self.get(name, false))
    }
    fn undefined_state(&self) -> Js {
        Js::Undefined
    }
    fn reference(&self, name: &str, _scope: &Scope<Self>) -> Js {
        self.get(name, true)
    }
    fn call(&self, node: &Ast, scope: &Scope<Self>) -> Js {
        let value = self.callback(scope, |eval| self.scope.call(node, eval));
        if let Some(value) = &value {
            measure_data(value, false);
        }
        value.as_ref().map(data_value).unwrap_or(Js::Undefined)
    }
}
fn runtime_value(js: &Js) -> Option<Value> {
    LIMITS.with(|l| to_value_in::<true>(js, l))
}

fn record_flags() {
    if crate::progress::stalled_so_far() {
        super::stack_stalled();
    }
    let now = LIMITS.with(|l| EvaluationFlags {
        size_limited: l.size_limited.get() || crate::progress::stalled_so_far(),
        depth_limited: l.depth_limited.get(),
    });
    ACCOUNT.with(|a| {
        let a = a.borrow();
        let old = a.flags.get();
        a.flags.set(EvaluationFlags {
            size_limited: old.size_limited || now.size_limited,
            depth_limited: old.depth_limited || now.depth_limited,
        });
    });
}

struct Account {
    limits: Limits,
    flags: Rc<Cell<EvaluationFlags>>,
    bytes: usize,
    inputs: HashSet<(u8, usize)>,
    bridges: HashMap<(u8, usize), Bridge>,
    collect_at: usize,
    owners: Vec<Value>,
}
impl Account {
    fn new(limits: Limits, bytes: usize) -> Self {
        Self {
            limits,
            flags: Rc::new(Cell::new(EvaluationFlags::default())),
            bytes,
            inputs: HashSet::new(),
            bridges: HashMap::new(),
            collect_at: 128,
            owners: Vec::new(),
        }
    }
    fn collect_bridges(&mut self) {
        if self.bridges.len() >= self.collect_at {
            self.bridges.retain(|_, v| v.value.alive());
            self.collect_at = self.bridges.len().saturating_mul(2).max(128);
        }
    }
    fn credit(&mut self, bytes: usize) {
        let before = self.bytes;
        self.bytes = self.bytes.saturating_add(bytes);
        LIMITS.with(|l| {
            l.room.set(
                l.room.get().saturating_add(
                    self.limits
                        .result_size_for(self.bytes)
                        .saturating_sub(self.limits.result_size_for(before)),
                ),
            );
            l.work.set(
                l.work.get().saturating_add(
                    self.limits
                        .work_for(self.bytes)
                        .saturating_sub(self.limits.work_for(before)),
                ),
            );
        });
    }
}
thread_local! {
    static ACCOUNT: RefCell<Account> = RefCell::new(Account::new(Limits::DEFAULT, 0));
    static DEPTH: RefCell<DepthContext> = RefCell::new(DepthContext::default());
}

fn register_input(value: &Value) {
    if LIMITS.with(|l| l.cut.get().is_some()) {
        return;
    }
    if let Some(key) = data_identity(value) {
        let skip = ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            if a.inputs.contains(&key) {
                true
            } else {
                a.owners.push(value.clone());
                false
            }
        });
        if skip {
            return;
        }
    }
    measure_data(value, true);
}
// Only values crossing the public callback boundary need an identity bridge.
// Weak entries neither keep discarded results alive nor index every input node.
enum WeakValue {
    Arr(Weak<Vec<Value>>),
    Obj(Weak<JsObject<Value>>),
}
impl WeakValue {
    fn alive(&self) -> bool {
        match self {
            Self::Arr(v) => v.strong_count() != 0,
            Self::Obj(v) => v.strong_count() != 0,
        }
    }
}
enum Original {
    Arr(Weak<Vec<Js>>),
    Obj(Weak<JsObject<Js>>),
    Props(Weak<Element>),
}
impl Original {
    fn read(&self) -> Option<Js> {
        match self {
            Self::Arr(v) => v.upgrade().map(Js::Arr),
            Self::Obj(v) => v.upgrade().map(Js::Obj),
            Self::Props(v) => v.upgrade().map(Js::Props),
        }
    }
}
struct Bridge {
    value: WeakValue,
    original: Original,
}
fn remember_result(value: &Value, js: &Js) {
    if let Some(key) = data_identity(value) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            if a.inputs.contains(&key) || a.bridges.contains_key(&key) {
                return;
            }
            let value = match value {
                Value::Arr(v) => WeakValue::Arr(Arc::downgrade(v)),
                Value::Obj(v) => WeakValue::Obj(Arc::downgrade(v)),
                _ => return,
            };
            let original = match js {
                Js::Arr(v) if Arc::strong_count(v) > 1 => Original::Arr(Arc::downgrade(v)),
                Js::Obj(v) if Arc::strong_count(v) > 1 => Original::Obj(Arc::downgrade(v)),
                Js::Props(v) => Original::Props(Arc::downgrade(v)),
                _ => return,
            };
            a.bridges.insert(key, Bridge { value, original });
            a.collect_bridges();
        });
    }
}
pub(super) fn data_identity(value: &Value) -> Option<(u8, usize)> {
    match value {
        Value::Arr(v) => Some((0, Arc::as_ptr(v) as usize)),
        Value::Obj(v) => Some((1, Arc::as_ptr(v) as usize)),
        Value::Element(v) => Some((2, Arc::as_ptr(v) as usize)),
        Value::Ast(v) => Some((3, Arc::as_ptr(v) as usize)),
        _ => None,
    }
}

// Input bytes are credited before charging the work to read them, so a large
// value can raise a scaled allowance without first running into its floor.
fn data_bytes(bytes: usize, input: bool) {
    if input {
        ACCOUNT.with(|a| a.borrow_mut().credit(bytes));
    }
}
fn data_text(text: &str, input: bool) -> usize {
    data_bytes(text.len(), input);
    if !step(text.len().saturating_add(1)) {
        return 0;
    }
    let bytes = js::json_string_len(text).saturating_add(2);
    data_bytes(bytes.saturating_sub(text.len()), input);
    bytes
}

// Input counts each container body once; output counts every occurrence.
fn measure_data(value: &Value, input: bool) -> usize {
    crate::stack::grow_stack!(measure_data(value, input), budget else 0);
    let Some(_level) = Level::enter() else {
        return 0;
    };
    if input
        && let Some(key) = data_identity(value)
        && !ACCOUNT.with(|a| a.borrow_mut().inputs.insert(key))
    {
        return 0;
    }
    if let Value::Str(text) = value {
        return data_text(text, input);
    }
    let own = match value {
        Value::Null => 4,
        Value::Bool(v) => {
            if *v {
                4
            } else {
                5
            }
        }
        Value::Num(n) => {
            if n.is_finite() {
                number_to_string(*n).len()
            } else {
                4
            }
        }
        Value::Arr(items) => 2usize.saturating_add(items.len().saturating_sub(1)),
        Value::Obj(o) => 2usize.saturating_add(o.len().saturating_sub(1)),
        Value::Element(el) => {
            // Braces, field names, colons, commas, booleans, props braces.
            let keys = ["type", "typeName", "props", "partial", "hasDynamicProps"];
            let mut n = 2
                + 4
                + 2
                + el.props.len().saturating_sub(1)
                + keys.iter().map(|k| k.len() + 3).sum::<usize>()
                + if el.partial { 4 } else { 5 }
                + if el.has_dynamic_props { 4 } else { 5 };
            if el.statement_id.is_some() {
                n += "statementId".len() + 4;
            }
            n
        }
        Value::Ast(node) => {
            // Unlike ordinary data, deferred ASTs must be walked before their
            // byte size is known. Never lend this walk the configured ceiling.
            let bytes = expression_bytes(node);
            if input {
                data_bytes(bytes, true);
            }
            return bytes;
        }
        Value::Str(_) => 0,
    };
    data_bytes(own, input);
    if !step(1) {
        return 0;
    }
    let mut bytes = own;
    let mut entries = |key: &str, value: &Value| {
        let n = data_text(key, input).saturating_add(1);
        data_bytes(1, input);
        bytes = bytes
            .saturating_add(n)
            .saturating_add(measure_data(value, input));
    };
    match value {
        Value::Arr(items) => {
            for item in items.iter() {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                bytes = bytes.saturating_add(measure_data(item, input));
            }
        }
        Value::Obj(o) => {
            for (key, value) in o.iter() {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                entries(key, value);
            }
        }
        Value::Element(el) => {
            for (key, value) in el.props.iter() {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                entries(key, value);
            }
            bytes = bytes
                .saturating_add(data_text("element", input))
                .saturating_add(data_text(&el.type_name, input));
            if let Some(id) = &el.statement_id {
                bytes = bytes.saturating_add(data_text(id, input));
            }
        }
        _ => {}
    }
    bytes
}

pub(super) fn data_value(value: &Value) -> Js {
    if !step(1) {
        return Js::Null;
    }
    if let Some(key) = data_identity(value)
        && let Some(original) =
            ACCOUNT.with(|a| a.borrow().bridges.get(&key).and_then(|v| v.original.read()))
    {
        return original;
    }
    match value {
        Value::Null => Js::Null,
        Value::Bool(v) => Js::Bool(*v),
        Value::Num(v) => Js::Num(*v),
        Value::Str(v) => {
            if step(v.len()) {
                Js::Str(v.as_str().into())
            } else {
                Js::Null
            }
        }
        Value::Ast(node) => Js::Ast(node.clone()),
        _ => Js::Data(Arc::new(value.clone())),
    }
}
pub(super) fn output_data(value: &Value) -> Value {
    let bytes = measure_data(value, false);
    if spend(bytes) {
        value.clone()
    } else {
        Value::Null
    }
}
pub(super) fn output_props(el: &Element) -> Value {
    let Some(_level) = Level::enter() else {
        return Value::Null;
    };
    let mut bytes = 2usize.saturating_add(el.props.len().saturating_sub(1));
    for (key, value) in el.props.iter() {
        if !step(1) {
            return Value::Null;
        }
        bytes = bytes
            .saturating_add(data_text(key, false))
            .saturating_add(1)
            .saturating_add(measure_data(value, false));
    }
    if spend(bytes) {
        Value::Obj(Arc::new(el.props.clone()))
    } else {
        Value::Null
    }
}
fn expand_props(props: &JsObject<Value>) -> Js {
    let mut out = JsObject::new();
    for (key, value) in props.iter() {
        if !step(key.len().saturating_add(1)) {
            break;
        }
        out.define(key.clone(), data_value(value));
    }
    Js::Obj(Arc::new(out))
}
pub(super) fn expand(value: &Js) -> Js {
    if let Js::Props(el) = value {
        return expand_props(&el.props);
    }
    let Js::Data(value) = value else {
        return value.clone();
    };
    match value.as_ref() {
        Value::Arr(items) => {
            if !step(items.len()) {
                return Js::Null;
            }
            Js::Arr(Arc::new(
                items
                    .iter()
                    .take_while(|_| LIMITS.with(|l| l.cut.get().is_none()))
                    .map(data_value)
                    .collect(),
            ))
        }
        Value::Obj(o) => expand_props(o),
        Value::Element(el) => {
            let mut out = JsObject::new();
            for key in [
                "type",
                "typeName",
                "props",
                "partial",
                "hasDynamicProps",
                "statementId",
            ] {
                let field = data_property(value, key);
                if key != "statementId" || el.statement_id.is_some() {
                    out.define(key, field);
                }
            }
            Js::Obj(Arc::new(out))
        }
        _ => data_value(value),
    }
}
pub(super) fn data_property(value: &Value, key: &str) -> Js {
    if !step(1) {
        return Js::Null;
    }
    match value {
        Value::Arr(_) | Value::Obj(_) => property(&Js::Data(Arc::new(value.clone())), key),
        Value::Element(el) => match key {
            "type" => Js::Str("element".into()),
            "typeName" => Js::Str(el.type_name.as_str().into()),
            "props" => Js::Props(el.clone()),
            "partial" => Js::Bool(el.partial),
            "hasDynamicProps" => Js::Bool(el.has_dynamic_props),
            "statementId" => el
                .statement_id
                .as_ref()
                .map(|s| Js::Str(s.as_str().into()))
                .unwrap_or(Js::Undefined),
            _ => property(&Js::Proto(Kind::Object), key),
        },
        _ => property(&data_value(value), key),
    }
}
pub(super) fn aggregate(value: &Js, name: &str) -> Js {
    let Js::Data(value) = value else {
        return Js::Num(0.0);
    };
    let Value::Arr(items) = value.as_ref() else {
        return Js::Num(0.0);
    };
    if !step(items.len()) {
        return Js::Null;
    }
    let mut total = 0.0;
    for (i, item) in items.iter().enumerate() {
        if LIMITS.with(|l| l.cut.get().is_some()) {
            return Js::Null;
        }
        let number = to_number(&data_value(item));
        total = match name {
            "Min" if i != 0 => js_min(total, number),
            "Max" if i != 0 => js_max(total, number),
            "Min" | "Max" => number,
            _ => total + number,
        };
    }
    Js::Num(if name == "Avg" && !items.is_empty() {
        total / items.len() as f64
    } else {
        total
    })
}
pub(super) fn elementwise_data(value: &Js, f: &dyn Fn(f64) -> f64) -> Js {
    let Js::Data(value) = value else {
        return Js::Null;
    };
    Js::Data(Arc::new(map_data(value, f)))
}
fn map_data(value: &Value, f: &dyn Fn(f64) -> f64) -> Value {
    crate::stack::grow_stack!(map_data(value, f), budget else Value::Null);
    let Some(_level) = Level::enter() else {
        return Value::Null;
    };
    if !step(1) {
        return Value::Null;
    }
    match value {
        Value::Arr(items) => Value::Arr(Arc::new(
            items
                .iter()
                .take_while(|_| LIMITS.with(|l| l.cut.get().is_none()))
                .map(|item| map_data(item, f))
                .collect(),
        )),
        _ => Value::Num(f(to_number(&data_value(value)))),
    }
}
pub(super) fn data_member(value: &Value, field: &str) -> Js {
    match value {
        Value::Arr(items) if field != "length" => {
            if !step(items.len()) {
                return Js::Null;
            }
            // Own object fields and missing scalar fields can be kept as
            // public values, without an intermediate JavaScript result list.
            if field != "__proto__"
                && items.iter().all(|item| {
                    matches!(
                        item,
                        Value::Obj(_) | Value::Null | Value::Num(_) | Value::Bool(_)
                    )
                })
            {
                let values = items
                    .iter()
                    .take_while(|_| step(2))
                    .map(|item| match item {
                        Value::Obj(o) => {
                            let value = o.get(field);
                            if let Some(Value::Str(text)) = value
                                && !step(text.len())
                            {
                                Value::Null
                            } else {
                                value.cloned().unwrap_or(Value::Null)
                            }
                        }
                        _ => Value::Null,
                    })
                    .collect();
                return Js::Data(Arc::new(Value::Arr(Arc::new(values))));
            }
            Js::Arr(Arc::new(
                items
                    .iter()
                    .take_while(|_| LIMITS.with(|l| l.cut.get().is_none()))
                    .map(|item| {
                        let js = data_property(item, field);
                        if is_nullish(&js) { Js::Null } else { js }
                    })
                    .collect(),
            ))
        }
        _ => data_property(value, field),
    }
}
pub(super) fn data_index(value: &Value, index: &Js) -> Js {
    if let Value::Arr(items) = value {
        let n = to_number(index);
        if n.fract() == 0.0 && n >= 0.0 && n < items.len() as f64 {
            return data_value(&items[n as usize]);
        }
        Js::Undefined
    } else {
        data_property(value, &to_string(index))
    }
}

// Price deferred expressions without allocating their JSON or cloning their
// tree. Early exits also bound DAGs whose serialized size grows exponentially.
pub(super) fn expression_bytes(node: &Ast) -> usize {
    measure_ast(node)
}
fn measure_ast(node: &Ast) -> usize {
    crate::stack::grow_stack!(measure_ast(node), budget else 0);
    let Some(_level) = Level::enter() else {
        return 0;
    };
    if !step(1) {
        return 0;
    }
    let mut bytes = 2usize;
    for (i, (key, field)) in fields(node).into_iter().enumerate() {
        if !step(1) {
            break;
        }
        bytes = bytes.saturating_add(key.len() + 3 + usize::from(i != 0));
        let size = match field {
            Field::Text(s) => text_bytes(s),
            Field::Num(n) => {
                if n.is_finite() {
                    number_to_string(n).len()
                } else {
                    4
                }
            }
            Field::Bool(b) => {
                if b {
                    4
                } else {
                    5
                }
            }
            Field::Node(n) => measure_ast(n),
            Field::Nodes(nodes) => {
                let mut size = 2usize;
                for (i, n) in nodes.iter().enumerate() {
                    if !step(1) {
                        break;
                    }
                    size = size
                        .saturating_add(measure_ast(n))
                        .saturating_add(usize::from(i != 0));
                }
                size
            }
            Field::Entries(entries) => {
                let mut size = 2usize;
                for (i, (key, n)) in entries.iter().enumerate() {
                    if !step(1) {
                        break;
                    }
                    size = size
                        .saturating_add(text_bytes(key))
                        .saturating_add(measure_ast(n))
                        .saturating_add(3 + usize::from(i != 0));
                }
                size
            }
            Field::Props(props) => {
                let mut size = 2usize;
                for (i, (key, n)) in props.iter().enumerate() {
                    if !step(1) {
                        break;
                    }
                    size = size
                        .saturating_add(text_bytes(key))
                        .saturating_add(measure_ast(n))
                        .saturating_add(1 + usize::from(i != 0));
                }
                size
            }
        };
        bytes = bytes.saturating_add(size);
    }
    bytes
}

fn text_bytes(s: &str) -> usize {
    if !step(s.len()) {
        return 0;
    }
    js::json_string_len(s).saturating_add(2)
}

#[derive(Clone)]
struct DepthContext {
    used: usize,
    limit: usize,
    cut: Rc<Cell<bool>>,
    stack_base: usize,
}
impl Default for DepthContext {
    fn default() -> Self {
        Self {
            used: 0,
            limit: usize::MAX,
            cut: Rc::new(Cell::new(false)),
            stack_base: 0,
        }
    }
}
impl DepthContext {
    fn current() -> Self {
        let mut context = DEPTH.with(|d| d.borrow().clone());
        context.used = context.used.saturating_add(LIMITS.with(|l| l.depth.get()));
        context
    }
    fn total_limit() -> usize {
        DEPTH.with(|d| d.borrow().limit)
    }
    fn suspend(saved: &Saved) {
        let mut context = saved.depth.clone();
        context.used = context.used.saturating_add(saved.limits.depth.get());
        DEPTH.with(|d| *d.borrow_mut() = context);
        LIMITS.with(|l| l.depth.set(0));
    }
}
struct DepthEntry {
    old: DepthContext,
    max_depth: usize,
}
impl DepthEntry {
    fn enter(mut outer: DepthContext) -> Self {
        let old = DEPTH.with(|d| d.borrow().clone());
        let max_depth = LIMITS.with(|l| l.max_depth.get());
        if outer.limit == usize::MAX && outer.used == 0 {
            outer.cut = Rc::new(Cell::new(false));
            outer.stack_base = crate::stack::segments();
        }
        // A resumed allowance's open levels already contribute to the incoming
        // total. Keep other allowances' levels in the offset, counted once.
        outer.used = outer.used.saturating_sub(LIMITS.with(|l| l.depth.get()));
        let configured = ACCOUNT
            .with(|a| a.borrow().limits.depth())
            .saturating_mul(VALUE_LEVELS_PER_LEVEL);
        outer.limit = outer.limit.min(configured);
        LIMITS.with(|l| l.max_depth.set(outer.limit.saturating_sub(outer.used)));
        DEPTH.with(|d| *d.borrow_mut() = outer);
        Self { old, max_depth }
    }
}
impl Drop for DepthEntry {
    fn drop(&mut self) {
        LIMITS.with(|l| l.max_depth.set(self.max_depth));
        DEPTH.with(|d| *d.borrow_mut() = self.old.clone());
    }
}
pub(crate) fn ceiling_cut() {
    LIMITS.with(|l| l.cut_by(Cut::Depth));
    DEPTH.with(|d| d.borrow().cut.set(true));
}

// Save every part of evaluation context. Parsing does not pay for these swaps.
struct Saved {
    restore: bool,
    limits: EvalLimits,
    account: Account,
    depth: DepthContext,
    stalled: bool,
    stack: crate::stack::Context,
    properties: HashMap<(usize, String), PropertyRead>,
    properties_log: Vec<(usize, String)>,
    read: HashMap<usize, (Arc<Element>, Js)>,
    read_log: Vec<usize>,
    same_as: Aliases,
}
impl Saved {
    fn enter() -> Self {
        record_flags();
        Self {
            restore: true,
            limits: LIMITS.with(Clone::clone),
            account: ACCOUNT.with(|a| {
                std::mem::replace(&mut *a.borrow_mut(), Account::new(Limits::DEFAULT, 0))
            }),
            depth: DEPTH.with(|d| std::mem::take(&mut *d.borrow_mut())),
            stalled: crate::progress::replace_stalled(false),
            stack: crate::stack::suspend_context(),
            properties: PROPERTIES.with(|s| std::mem::take(&mut *s.borrow_mut())),
            properties_log: PROPERTIES_LOG.with(|s| std::mem::take(&mut *s.borrow_mut())),
            read: READ.with(|s| std::mem::take(&mut *s.borrow_mut())),
            read_log: READ_LOG.with(|s| std::mem::take(&mut *s.borrow_mut())),
            same_as: SAME_AS.with(|s| std::mem::take(&mut *s.borrow_mut())),
        }
    }
    fn swap(&mut self) {
        record_flags();
        LIMITS.with(|l| {
            l.room.swap(&self.limits.room);
            l.work.swap(&self.limits.work);
            l.depth.swap(&self.limits.depth);
            l.max_depth.swap(&self.limits.max_depth);
            l.size_limited.swap(&self.limits.size_limited);
            l.depth_limited.swap(&self.limits.depth_limited);
            l.cut.swap(&self.limits.cut);
        });
        ACCOUNT.with(|a| std::mem::swap(&mut *a.borrow_mut(), &mut self.account));
        DEPTH.with(|d| std::mem::swap(&mut *d.borrow_mut(), &mut self.depth));
        self.stalled = crate::progress::replace_stalled(self.stalled);
        crate::stack::swap_context(&mut self.stack);
        PROPERTIES.with(|s| std::mem::swap(&mut *s.borrow_mut(), &mut self.properties));
        PROPERTIES_LOG.with(|s| std::mem::swap(&mut *s.borrow_mut(), &mut self.properties_log));
        READ.with(|s| std::mem::swap(&mut *s.borrow_mut(), &mut self.read));
        READ_LOG.with(|s| std::mem::swap(&mut *s.borrow_mut(), &mut self.read_log));
        SAME_AS.with(|s| std::mem::swap(&mut *s.borrow_mut(), &mut self.same_as));
    }
}
impl Drop for Saved {
    fn drop(&mut self) {
        if self.restore {
            self.swap();
        }
    }
}
struct Resume<'a>(&'a RefCell<Saved>);
impl<'a> Resume<'a> {
    fn enter(saved: &'a RefCell<Saved>) -> Self {
        saved.borrow_mut().swap();
        Self(saved)
    }
}
impl Drop for Resume<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().swap();
    }
}
