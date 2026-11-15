//! Public expression evaluation, separate from parsing's entry and caches.

mod input_identity;
mod input_size;
use input_identity::InputIndex;
mod inputs;
mod reference_input;
pub use inputs::PreparedInput;

use super::*;
use crate::{Limits, PreparedValidationSchema};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Weak;

/// Names and calls supplied by the caller. State names include `$`.
///
/// For name and call callbacks, `None` is JavaScript undefined;
/// `Some(Value::Null)` is explicit null. The per-item hook instead uses `None`
/// for no replacement.
/// Extra scope falls back to state for both undefined and null. References
/// and state otherwise preserve undefined, as upstream's evaluator does.
/// Lookups are lazy and cached for an allowance, so they must stay stable.
/// A registration phase can opt reference reads out through `cache_references`;
/// state and extra lookups remain cached.
///
/// Every callback receives an evaluator sharing the allowance. Lookup callbacks
/// (`state`, `reference`, `extra`) evaluate at the root, without enclosing
/// `@Each` bindings. Call and per-item callbacks retain those bindings.
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
    /// Whether reference lookups are stable enough to cache in this phase.
    fn cache_references(&self) -> bool {
        true
    }
    /// Prepared external data already included in the session's initial input.
    /// Reading its cached representation takes constant work and adds no input
    /// credit. Computed references retain the normal bounded measurement path.
    fn prepared_reference(&self, _name: &str) -> Option<&PreparedReferenceValue> {
        None
    }
    /// Supplied extra scope such as `$value`; null and undefined fall back to
    /// state. Return supplied data, never evaluator output: it earns input credit.
    fn extra(&self, _name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        None
    }
    /// Whether `extra` supplies own properties of a JavaScript plain object.
    /// Enables property-key coercion and inherited object members for data-shaped
    /// StateRef names. `None` from `extra` means an absent own property.
    /// Sampled once when the session starts. Defaults to the existing
    /// string-key callback behavior.
    fn extra_is_object(&self) -> bool {
        false
    }
    /// Inspect an opted-in expression result before its public projection.
    /// Return None to keep it, or a replacement view. `state` reads at the root,
    /// bypassing extra scope and retaining normal state-input credit. Replacement
    /// output always spends this evaluation's work, room and depth; it is not
    /// reinvoked on the replacement. Failed or cut evaluations skip the hook;
    /// view operations and lookups stop after a cut. Nested callbacks opt in separately.
    /// Inspection and other host work remain the caller's responsibility.
    /// Views cannot escape the callback's owner lifetime:
    /// ```compile_fail
    /// # use openui_lang::{EvaluationScope, ExpressionValue};
    /// # use std::cell::RefCell;
    /// struct Keep(RefCell<Option<ExpressionValue<'static>>>);
    /// impl EvaluationScope for Keep {
    ///     fn project_result<'v>(&self, value: ExpressionValue<'v>,
    ///         _: &dyn Fn(&str) -> ExpressionValue<'v>) -> Option<ExpressionValue<'v>> {
    ///         *self.0.borrow_mut() = Some(value.property("payload"));
    ///         None
    ///     }
    /// }
    /// ```
    fn project_result<'v>(
        &self,
        _value: ExpressionValue<'v>,
        _state: &dyn Fn(&str) -> ExpressionValue<'v>,
    ) -> Option<ExpressionValue<'v>> {
        None
    }
    /// Inspect an item without allocating its public projection or spending
    /// result room. Return true only when `each_item` will use the projection.
    fn wants_each_item(&self, _value: &ExpressionValue<'_>, _call_context: bool) -> bool {
        false
    }
    /// Opt in to unmapped data calls before any callback frame or result
    /// measurement is made. Mapped calls retain their existing dispatch.
    /// The name has object-property coercion; inspecting this predicate must
    /// not evaluate children. False keeps the ordinary null fallback free.
    fn wants_unmapped_call(&self, _call: &Value, _name: &str) -> bool {
        false
    }
    /// Return an opaque evaluated output, optionally transforming a child
    /// expression before public projection. Defaults to the ordinary call hook.
    fn call_output(&self, call: &Ast, evaluator: &ExpressionEvaluator<'_>) -> ExpressionOutput {
        self.call(call, evaluator).into()
    }
    /// Data counterpart of `call_output`. Also receives unmapped calls when
    /// `wants_unmapped_call` opts in; children remain unevaluated.
    fn call_value_output(
        &self,
        call: &Value,
        name: &str,
        evaluator: &ExpressionEvaluator<'_>,
    ) -> ExpressionOutput {
        self.call_value_named(call, name, evaluator).into()
    }
    /// Handle a mapped call descriptor supplied through data. Opted-in unmapped
    /// descriptors also arrive here through the default output hook. Children remain lazy.
    fn call_value(&self, _call: &Value, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
    /// Handle a call descriptor with its already-coerced object-property name.
    /// The core computes this key for builtin dispatch on the runtime path;
    /// reusing it avoids another coercion or result reservation in library lookup.
    /// The default preserves existing `call_value` implementations.
    fn call_value_named(
        &self,
        call: &Value,
        _name: &str,
        evaluator: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        self.call_value(call, evaluator)
    }
    /// Optionally replace an evaluated `@Each` item, with its iteration's
    /// bindings and shared allowance. `None` preserves the original JS value,
    /// including undefined fields and array items that `Value` cannot represent.
    /// The borrowed input is that value's public projection; None is undefined.
    /// Tree evaluation uses a replacement only when revisiting an element.
    fn each_item(
        &self,
        _value: Option<&Value>,
        _evaluator: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        None
    }
    /// Opt in to a literal-equivalent lazy view for a direct iterator read.
    /// The evaluator restricts this to canonical elements with bounded simple
    /// props, in per-item context and away from the depth boundary. The default
    /// retains the fully literalized public projection for custom scopes.
    #[doc(hidden)]
    fn allow_direct_each_reference(&self) -> bool {
        false
    }
    /// Owning counterpart of `each_item`. The default preserves that hook.
    /// An implementation may take the projection to rebuild it without keeping
    /// its discarded contents alive. If it takes or changes it, it must return
    /// a replacement; returning None must leave the projection unchanged.
    /// Every replacement is still measured and charged by the evaluator.
    #[doc(hidden)]
    fn each_item_owned(
        &self,
        value: &mut Option<Value>,
        evaluator: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        self.each_item(value.as_ref(), evaluator)
    }
    /// Handle a non-builtin call with unevaluated, already substituted args.
    /// An unimplemented call defaults to null; a handler can return undefined.
    fn call(&self, _call: &Ast, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
}
impl EvaluationScope for () {}

/// An external reference's cached representation and prepaid input size.
///
/// The size must come from a prior bounded measurement and be included in
/// [`EvaluationSession::new`]'s initial input bytes. This grants no credit on
/// read and never discounts output construction. Containers remain shared;
/// scalar strings are copied into shared storage once, when prepared.
pub struct PreparedReferenceValue {
    measured: ReferenceMeasurement,
}
enum ReferenceMeasurement {
    Small { value: Js, bytes: u32, depth: u32 },
    Large(Box<LargeReference>),
}
struct LargeReference {
    value: Js,
    bytes: usize,
    depth: usize,
}
impl PreparedReferenceValue {
    /// Cache a measured input value without evaluating it. The caller supplies
    /// its already-paid byte contribution; this constructor adds no height claim.
    pub fn new(value: &Value, input_bytes: usize) -> Self {
        Self::measured(value, input_bytes, 0)
    }
    fn measured(value: &Value, input_bytes: usize, depth: usize) -> Self {
        let value = match value {
            Value::Null => Js::Null,
            Value::Bool(v) => Js::Bool(*v),
            Value::Num(v) => Js::Num(*v),
            Value::Str(v) => Js::Str(v.as_str().into()),
            Value::Ast(v) => Js::Ast(v.clone()),
            _ => Js::Data(Arc::new(value.clone())),
        };
        let measured = match (u32::try_from(input_bytes), u32::try_from(depth)) {
            (Ok(bytes), Ok(depth)) => ReferenceMeasurement::Small {
                value,
                bytes,
                depth,
            },
            _ => ReferenceMeasurement::Large(Box::new(LargeReference {
                value,
                bytes: input_bytes,
                depth,
            })),
        };
        Self { measured }
    }
    fn value(&self) -> &Js {
        match &self.measured {
            ReferenceMeasurement::Small { value, .. } => value,
            ReferenceMeasurement::Large(v) => &v.value,
        }
    }
    fn bytes(&self) -> usize {
        match &self.measured {
            ReferenceMeasurement::Small { bytes, .. } => *bytes as usize,
            ReferenceMeasurement::Large(v) => v.bytes,
        }
    }
    fn depth(&self) -> usize {
        match &self.measured {
            ReferenceMeasurement::Small { depth, .. } => *depth as usize,
            ReferenceMeasurement::Large(v) => v.depth,
        }
    }
    /// Prepare named external values with one shared bounded measurement.
    /// Successful entries carry core-verified byte and height measurements;
    /// failed entries earn no input and retain their individual cut flags.
    /// Returns the entries and their combined supplied-object byte contribution.
    /// Preparation does not spend an active evaluation's work or earn credit.
    #[doc(hidden)]
    pub fn measure_all(
        values: &JsObject<Value>,
        limits: Limits,
    ) -> (BTreeMap<&str, Result<Self, EvaluationFlags>>, usize) {
        reference_input::prepare(values, limits)
    }
}

type ScopedOperation<'a> = dyn Fn(&mut dyn FnMut()) + 'a;

/// A scoped read-only view of an internal JS value. Inspection does not
/// normalize undefined entries or allocate a public container projection.
/// Views stay on the owning thread; hook views also retain their owner's allowance.
pub struct ExpressionValue<'a> {
    value: std::borrow::Cow<'a, Js>,
    operate: Option<&'a ScopedOperation<'a>>,
}
impl<'a> ExpressionValue<'a> {
    /// Read an object property using the evaluator's JS property semantics.
    pub fn property(&self, name: &str) -> ExpressionValue<'a> {
        let mut value = Js::Undefined;
        let mut read = || value = property(&self.value, name);
        if let Some(operate) = self.operate {
            operate(&mut read);
        } else {
            read();
        }
        ExpressionValue {
            value: std::borrow::Cow::Owned(value),
            operate: self.operate,
        }
    }
    /// Whether the value is an array, without property reads or coercion.
    pub fn is_array(&self) -> bool {
        matches!(self.value.as_ref(), Js::Arr(_))
            || matches!(self.value.as_ref(),Js::Data(v) if matches!(v.as_ref(),Value::Arr(_)))
    }
    /// Iterate array entries using bounded property reads, without projecting
    /// any public containers. Non-arrays yield no entries.
    pub fn items(&self) -> impl Iterator<Item = ExpressionValue<'a>> + use<'a> {
        let owner = ExpressionValue {
            value: std::borrow::Cow::Owned(self.value.clone().into_owned()),
            operate: self.operate,
        };
        let len = match self.value.as_ref() {
            Js::Arr(a) => a.len(),
            Js::Data(v) => match v.as_ref() {
                Value::Arr(a) => a.len(),
                _ => 0,
            },
            _ => 0,
        };
        let operate = owner.operate;
        (0..len)
            .take_while(move |_| {
                let mut live = false;
                let mut check = || live = LIMITS.with(|l| l.cut.get().is_none());
                if let Some(op) = operate {
                    op(&mut check)
                } else {
                    check()
                };
                live
            })
            .map(move |i| owner.property(&i.to_string()))
    }
    /// An existing caller-owned data value, borrowed without projection.
    pub fn data(&self) -> Option<&Value> {
        match self.value.as_ref() {
            Js::Data(v) => Some(v.as_ref()),
            _ => None,
        }
    }
    /// JavaScript property presence, including an own undefined property.
    pub fn has_property(&self, name: &str) -> bool {
        let mut present = false;
        let mut read = || {
            present = match self.value.as_ref() {
                Js::Obj(o) => o.has(name),
                Js::Data(v) => match v.as_ref() {
                    Value::Obj(o) => o.has(name),
                    _ => !matches!(property(&self.value, name), Js::Undefined),
                },
                _ => !matches!(property(&self.value, name), Js::Undefined),
            }
        };
        if let Some(op) = self.operate {
            op(&mut read)
        } else {
            read()
        };
        present
    }
    /// Build an internal array of views. Each retained entry spends work;
    /// public output room is charged by the enclosing expression projection.
    /// Live internal and public array capacities share a byte ceiling equal
    /// to the session's result-size limit, with a fixed 64-byte ledger header.
    /// This includes nested retained arrays;
    /// discarded arrays release memory credit. Final JSON projection is priced
    /// separately, so flattened steps do not spend serialized room twice.
    /// A cut discards the interrupted transform instead of exposing its buffer.
    pub fn array(&self, values: impl IntoIterator<Item = ExpressionValue<'a>>) -> Self {
        let mut values = Some(values);
        let mut result = Js::Undefined;
        let mut build = || {
            let Some(mut reservation) = BufferReservation::new(BUFFER_OVERHEAD) else {
                return;
            };
            let mut out = Vec::new();
            if let Some(values) = values.take() {
                for value in values {
                    if !step(1) || !builder_fits(out.len().saturating_add(3)) {
                        break;
                    }
                    if out.len() == out.capacity() {
                        let maximum = buffer_limit()
                            .saturating_sub(BUFFER_OVERHEAD.saturating_mul(2))
                            / std::mem::size_of::<Js>()
                                .saturating_add(std::mem::size_of::<Value>())
                                .max(1);
                        let next = out.capacity().max(2).saturating_mul(2).min(maximum);
                        let bytes = next
                            .saturating_mul(std::mem::size_of::<Js>())
                            .saturating_add(BUFFER_OVERHEAD);
                        if next <= out.len()
                            || !reservation.grow(bytes)
                            || out
                                .try_reserve_exact(next.saturating_sub(out.len()))
                                .is_err()
                        {
                            LIMITS.with(|l| l.cut_by(Cut::Room));
                            break;
                        }
                        let actual = out
                            .capacity()
                            .saturating_mul(std::mem::size_of::<Js>())
                            .saturating_add(BUFFER_OVERHEAD);
                        if !reservation.grow(actual) {
                            break;
                        }
                    }
                    out.push(value.value.into_owned());
                }
            }
            if LIMITS.with(|l| l.cut.get().is_some()) {
                return;
            }
            let out = Arc::new(out);
            reservation.finish(BufferOwner::Js(Arc::downgrade(&out)));
            result = Js::Arr(out);
        };
        if let Some(op) = self.operate {
            op(&mut build)
        } else {
            build()
        };
        Self {
            value: std::borrow::Cow::Owned(result),
            operate: self.operate,
        }
    }
    /// Build an internal object of views. Keys spend work; its public
    /// projection measures and charges the complete resulting object.
    pub fn object(
        &self,
        fields: impl IntoIterator<Item = (&'static str, ExpressionValue<'a>)>,
    ) -> Self {
        let mut fields = Some(fields);
        let mut out = JsObject::new();
        let mut minimum = 2usize;
        let mut build = || {
            if let Some(fields) = fields.take() {
                for (key, value) in fields {
                    if !step(key.len().saturating_add(1)) {
                        break;
                    }
                    if !out.has_own(key) {
                        minimum =
                            minimum.saturating_add(js::json_string_len(key).saturating_add(4));
                        if !builder_fits(minimum) {
                            break;
                        }
                    }
                    out.define(key, value.value.into_owned());
                }
            }
        };
        if let Some(op) = self.operate {
            op(&mut build)
        } else {
            build()
        };
        Self {
            value: std::borrow::Cow::Owned(Js::Obj(Arc::new(out))),
            operate: self.operate,
        }
    }
    /// A null replacement; projection still charges its output.
    pub fn null() -> Self {
        Self {
            value: std::borrow::Cow::Owned(Js::Null),
            operate: None,
        }
    }
    /// Whether the value is null or undefined, without coercion.
    pub fn is_nullish(&self) -> bool {
        is_nullish(&self.value)
    }
    /// A string value, without coercion.
    pub fn as_str(&self) -> Option<&str> {
        match self.value.as_ref() {
            Js::Str(s) => Some(s),
            _ => None,
        }
    }
    /// A boolean value, without coercion.
    pub fn as_bool(&self) -> Option<bool> {
        match self.value.as_ref() {
            Js::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// A non-null JS object, including arrays, excluding functions.
    pub fn is_object(&self) -> bool {
        matches!(
            self.value.as_ref(),
            Js::Arr(_) | Js::Obj(_) | Js::Ast(_) | Js::Props(_) | Js::Proto(_)
        ) || matches!(self.value.as_ref(), Js::Data(v) if matches!(v.as_ref(), Value::Arr(_) | Value::Obj(_) | Value::Ast(_) | Value::Element(_)))
    }
}

/// An opaque callback output. Values constructed by the caller are measured
/// on admission. Evaluated/transformed values retain their internal objects
/// until the enclosing expression projects them, spending its shared limits.
/// This never grants input credit and exposes no serialized provenance.
#[derive(Debug)]
pub struct ExpressionOutput {
    value: Output,
}
#[derive(Debug)]
enum Output {
    Data(Option<Value>),
    Evaluated(Js),
}
impl From<Option<Value>> for ExpressionOutput {
    fn from(value: Option<Value>) -> Self {
        Self {
            value: Output::Data(value),
        }
    }
}
impl ExpressionOutput {
    fn data(self) -> Option<Value> {
        match self.value {
            Output::Data(v) => v,
            Output::Evaluated(_) => None,
        }
    }
}
fn admit_output(output: ExpressionOutput) -> Js {
    if failed() {
        return Js::Undefined;
    }
    match output.value {
        Output::Data(value) => {
            if let Some(value) = &value {
                measure_boundary(value);
            }
            value.as_ref().map(data_value).unwrap_or(Js::Undefined)
        }
        Output::Evaluated(js) => {
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                a.transformed = a.transformed.saturating_add(1);
            });
            js
        }
    }
}
type ValueTransform<'a> = dyn for<'v> FnMut(ExpressionValue<'v>) -> ExpressionValue<'v> + 'a;

/// Evaluation within a callback, with the same scope, depth and allowance.
pub struct ExpressionEvaluator<'a> {
    evaluate: &'a dyn Fn(&Ast, bool) -> (EvaluationResult, bool),
    evaluate_value: &'a dyn Fn(&Value, bool, bool) -> (EvaluationResult, bool),
    operate: &'a dyn Fn(&mut dyn FnMut()),
    transform: &'a dyn Fn(&Value, &mut ValueTransform<'_>) -> ExpressionOutput,
    fields: &'a dyn Fn(&JsObject<Value>) -> EvaluationResult,
    projection: Option<&'a Cell<bool>>,
}
impl ExpressionEvaluator<'_> {
    /// Evaluate a statement or argument; `None` denotes undefined.
    pub fn evaluate(&self, expression: &Ast) -> Option<Value> {
        self.evaluate_result(expression).value
    }
    /// Evaluate and inspect the limits affecting this evaluation.
    pub fn evaluate_result(&self, expression: &Ast) -> EvaluationResult {
        self.projected((self.evaluate)(expression, self.projection.is_some()))
    }
    /// Evaluate an AST-shaped data value. Literal descriptors return their raw
    /// value; child fields are read lazily by the same operator implementation.
    pub fn evaluate_value(&self, expression: &Value) -> EvaluationResult {
        self.projected((self.evaluate_value)(
            expression,
            false,
            self.projection.is_some(),
        ))
    }
    /// Evaluate and transform an expression in internal form. The transform's
    /// views cannot escape; its output is projected only by the owning call.
    /// Operations and builders share work/depth, and never earn input credit.
    /// A cut while transforming or projecting discards the transformed value;
    /// its unpriced buffer never escapes in the public evaluation result.
    pub fn transform_value(
        &self,
        expression: &Value,
        mut transform: impl for<'v> FnMut(ExpressionValue<'v>) -> ExpressionValue<'v>,
    ) -> ExpressionOutput {
        (self.transform)(expression, &mut transform)
    }
    /// Evaluate a named call prop, retaining the call site's `val.k` failure
    /// when a raw null is supplied instead of an AST descriptor.
    pub fn evaluate_call_prop(&self, expression: &Value) -> EvaluationResult {
        self.projected((self.evaluate_value)(
            expression,
            true,
            self.projection.is_some(),
        ))
    }
    /// Opt evaluations in this closure into `EvaluationScope::project_result`.
    /// The boolean records replacements through the supplied evaluator. It keeps
    /// its owner, allowance and loop bindings. Nested callbacks receive their
    /// ordinary evaluator and opt in independently. No result is pre-projected.
    pub fn with_projection<T>(&self, f: impl FnOnce(&ExpressionEvaluator<'_>) -> T) -> (T, bool) {
        let replaced = Cell::new(false);
        let evaluator = ExpressionEvaluator {
            evaluate: self.evaluate,
            evaluate_value: self.evaluate_value,
            operate: self.operate,
            transform: self.transform,
            fields: self.fields,
            projection: Some(&replaced),
        };
        let result = f(&evaluator);
        (result, replaced.get())
    }
    fn projected(&self, (result, replaced): (EvaluationResult, bool)) -> EvaluationResult {
        if let Some(flag) = self.projection {
            flag.set(flag.get() || replaced);
        }
        result
    }
    /// Run a recursive caller walk within one shared logical depth level.
    /// Returns None on a cut. The level and stack context leave on return or
    /// unwind; a closure prevents guards escaping across session boundaries.
    pub fn with_level<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            if let Some(f) = f.take() {
                out = caller_level(f);
            }
        });
        out
    }
    /// Reserve caller-produced JSON bytes, charging work and result room.
    /// Retaining unevaluated supplied input after a cut needs no reservation.
    pub fn reserve_result(&self, bytes: usize) -> bool {
        let mut ok = false;
        (self.operate)(&mut || ok = spend(bytes));
        ok
    }
    /// Reserve caller output retained separately from the expression value,
    /// for example a diagnostic. Replacing/discarding an intermediate value
    /// cannot release this room. This grants no input or work credit.
    pub fn reserve_retained_result(&self, bytes: usize) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            ok = spend(bytes);
            if ok {
                ACCOUNT.with(|a| {
                    let mut a = a.borrow_mut();
                    a.retained_room = a.retained_room.saturating_add(bytes);
                });
            }
        });
        ok
    }
    /// Retain an array's unchanged punctuation while its items are revisited.
    /// The evaluator derives the room from the actual container. Reading and
    /// producing item bodies must be charged separately. This does not produce
    /// punctuation again, so it consumes no additional serialization work.
    #[doc(hidden)]
    pub fn reserve_array_header(&self, value: &Value) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            if let Value::Arr(items) = value {
                ok = LIMITS.with(|l| {
                    draw(
                        l,
                        Cut::Room,
                        2usize.saturating_add(items.len().saturating_sub(1)),
                    )
                });
            }
        });
        ok
    }
    /// Restore the unchanged part of a projected array after discarding its
    /// intermediate reservation. The selector names items rebuilt separately.
    /// Sizes come from this evaluator's projection/admission records, not the
    /// caller. The projection receipt is consumed once; missing proof returns
    /// None so callers can use ordinary admission. No spent work is refunded.
    #[doc(hidden)]
    pub fn reserve_array_remainder(
        &self,
        value: &Value,
        replaced: impl Fn(&Value) -> bool,
    ) -> Option<bool> {
        let mut out = None;
        (self.operate)(&mut || {
            let Value::Arr(items) = value else { return };
            let key = data_identity(value);
            let bytes = key.and_then(|key| {
                ACCOUNT.with(|a| {
                    let mut a = a.borrow_mut();
                    let entry = a.argument_bodies.get_mut(&key)?;
                    if !entry.projected || !entry.owner.alive() {
                        return None;
                    }
                    entry.projected = false;
                    Some(entry.bytes)
                })
            });
            let Some(mut remaining) = bytes else { return };
            // This structural pass is covered by the original projection's
            // item/punctuation work. Text and unchanged bodies are not read.
            for item in items.iter().filter(|v| replaced(v)) {
                let cached = data_identity(item).and_then(|key| {
                    ACCOUNT.with(|a| {
                        let a = a.borrow();
                        a.inputs
                            .measurement(&key)
                            .and_then(|(bytes, _)| bytes)
                            .or_else(|| {
                                a.argument_bodies
                                    .get(&key)
                                    .filter(|m| m.owner.alive() && m.bytes != 0)
                                    .map(|m| m.bytes)
                            })
                    })
                });
                let bytes = cached.unwrap_or_else(|| measure_data(item, false));
                let Some(rest) = remaining.checked_sub(bytes) else {
                    return;
                };
                remaining = rest;
            }
            out = Some(LIMITS.with(|l| draw(l, Cut::Room, remaining)));
        });
        out
    }
    /// Charge caller work in this allowance. Cuts remain sticky; this does not
    /// reserve output room or add input credit.
    pub fn charge_work(&self, units: usize) -> bool {
        let mut ok = false;
        (self.operate)(&mut || ok = step(units));
        ok
    }
    /// Remaining logical value levels at this position, including enclosing
    /// callbacks and foreign sessions. A successful walk can only be reused
    /// where at least as many levels remain.
    pub fn remaining_depth(&self) -> usize {
        let mut depth = 0;
        (self.operate)(&mut || {
            depth = LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()))
        });
        depth
    }
    /// Read supplied input using the same bounded measurement and credit rules
    /// as state reads. Shared container bodies earn credit once per allowance.
    /// Never call this for computed values from inside scope callbacks.
    /// Computed intermediate results must not be registered as supplied input.
    pub fn register_input(&self, value: &Value) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            inputs::register_value(value);
            ok = LIMITS.with(|l| l.cut.get().is_none());
        });
        ok
    }
    /// Raise the session's supplied-input floor to a core-measured byte count.
    /// Measurement is caller setup, not evaluation work. Only an increase raises
    /// the scaled allowance; already-spent work stays spent. This retains no
    /// input, owner or token. Computed values from scope callbacks must never be
    /// admitted as supplied input.
    pub fn register_input_floor(&self, input: &PreparedInput) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            if LIMITS.with(|l| l.cut.get().is_some()) {
                return;
            }
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                let extra = input.bytes.saturating_sub(a.supplied_floor);
                a.supplied_floor = a.supplied_floor.max(input.bytes);
                a.credit(extra);
            });
            ok = true;
        });
        ok
    }
    /// Read borrowed supplied text with the same work and input credit as a
    /// supplied string value, before allocating a copy (for example a key).
    pub fn register_input_text(&self, text: &str) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            if LIMITS.with(|l| l.cut.get().is_none()) {
                data_text(text, true);
            }
            ok = LIMITS.with(|l| l.cut.get().is_none());
        });
        ok
    }
    /// Reserve an unchanged supplied value's JSON, without earning input credit
    /// or copying its body. Its bounded measurement also charges work/depth.
    pub fn reserve_value(&self, value: &Value) -> bool {
        let mut ok = false;
        (self.operate)(&mut || ok = spend(measure_data(value, false)));
        ok
    }

    /// Reserve a caller-produced value and return its exact serialized byte
    /// size. Measurement uses this allowance's work, depth and room; a cut
    /// returns `None`. Shared bodies count at every output position.
    pub fn reserve_value_size(&self, value: &Value) -> Option<usize> {
        let mut out = None;
        (self.operate)(&mut || {
            let bytes = measure_data(value, false);
            if spend(bytes) && LIMITS.with(|l| l.cut.get().is_none()) {
                out = Some(bytes);
            }
        });
        out
    }
    /// Recheck an already reserved value's depth without another output
    /// reservation or input credit. Successful identity measurements are
    /// reusable only where their measured height fits.
    pub fn check_depth(&self, value: &Value) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            let _mode = DepthOnly::enter();
            measure_data(value, false);
            ok = LIMITS.with(|l| l.cut.get().is_none());
        });
        ok
    }
    /// Previously measured value-walk height, including the value's root. This
    /// reads only live, exact evaluator measurements; it performs no walk,
    /// changes no allowance and makes no claim about a new position's depth.
    pub fn measured_depth(&self, value: &Value) -> Option<usize> {
        let mut out = None;
        (self.operate)(&mut || {
            if let Some(key) = data_identity(value) {
                out = ACCOUNT.with(|a| {
                    let a = a.borrow();
                    a.inputs
                        .measurement(&key)
                        .map(|(_, h)| h.saturating_add(1))
                        .or_else(|| {
                            a.argument_bodies
                                .get(&key)
                                .filter(|v| v.owner.alive() && v.exact)
                                .map(|v| (v.depth as usize).saturating_add(1))
                        })
                });
            }
        });
        out
    }
    /// Check unchanged supplied output without retaining its result room.
    /// Work and depth remain charged and cuts stay sticky. Every serialized
    /// occurrence counts, including aliases discounted in argument buffers.
    /// The evaluator measures the value itself; a caller cannot claim its size or gain input credit.
    /// Use this for sequential writes whose output is retained by a store,
    /// rather than accumulating in the expression's own returned result.
    pub fn check_result_value(&self, value: &Value) -> bool {
        let mut ok = false;
        (self.operate)(&mut || {
            ACCOUNT.with(|a| a.borrow_mut().temporary_room.push((0, true)));
            let _room = TemporaryResults { release: true };
            let before = LIMITS.with(|l| l.room.get());
            ok = spend(measure_data(value, false));
            if ok {
                let consumed = LIMITS.with(|l| before.saturating_sub(l.room.get()));
                ACCOUNT.with(|a| {
                    if let Some(room) = a.borrow_mut().temporary_room.last_mut() {
                        room.0 = room.0.saturating_add(consumed);
                    }
                });
            }
        });
        ok
    }
    /// Scope projected intermediate results to a callback. Their room remains
    /// charged while the closure runs, then is released before the callback's
    /// final output is admitted. Work, diagnostics and cuts are retained.
    /// Sizes are measured by the evaluator; callers cannot claim credit.
    /// The scope restores on unwind too. Values escaping the closure still
    /// need normal final-output admission and projection.
    pub fn with_temporary_results<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            if let Some(f) = f.take() {
                ACCOUNT.with(|a| a.borrow_mut().temporary_room.push((0, true)));
                let _room = TemporaryResults { release: true };
                out = Some(f());
            }
        });
        out
    }
    /// Scope projections that may be replaced by a caller walk. The closure's
    /// boolean discards their room before rebuilding the final value; false
    /// retains it. Persistent caller reservations and spent work are unchanged.
    /// Escaping discarded values require normal output reservation.
    pub fn with_intermediate_results<T>(&self, f: impl FnOnce() -> (T, bool)) -> Option<T> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            if let Some(f) = f.take() {
                ACCOUNT.with(|a| a.borrow_mut().temporary_room.push((0, false)));
                let mut room = TemporaryResults { release: true };
                let (value, discard) = f();
                room.release = discard;
                out = Some(value);
            }
        });
        out
    }
    /// Evaluate named descriptors as one object. Supplied identities may reuse
    /// their public argument object; undefined fields are omitted. Reuse still
    /// checks depth at this position and never grants input credit.
    #[doc(hidden)]
    pub fn evaluate_fields(&self, fields: &JsObject<Value>) -> EvaluationResult {
        (self.fields)(fields)
    }
    /// Evaluate an intermediate that will be discarded on a cut. Room and
    /// depth cuts are returned locally; spent work is never restored, and a
    /// work cut remains sticky. The closure must not retain partial output.
    /// Context and reservations restore on unwind.
    pub fn with_discarded_result<T>(&self, f: impl FnOnce() -> T) -> Option<(T, EvaluationFlags)> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            if let Some(f) = f.take() {
                let guard = DiscardedResult::enter();
                let value = f();
                record_flags();
                let flags = ACCOUNT.with(|a| a.borrow().flags.get());
                drop(guard);
                out = Some((value, flags));
            }
        });
        out
    }
    /// Report a caller callback failure through the shared failure channel.
    pub fn fail(&self, message: impl Into<String>) {
        let mut message = Some(message.into());
        (self.operate)(&mut || {
            if let Some(message) = message.take() {
                fail(EvaluationFailure::Callback(message));
            }
        });
    }
    /// JavaScript `String(value)`, using this allowance and failure channel.
    /// No result room is reserved until the caller projects its final value.
    pub fn coerce_string(&self, value: &Value) -> Option<String> {
        let mut out = None;
        (self.operate)(&mut || {
            let text = to_string_in::<Supplied<'_>>(&data_value(value));
            if !failed() && LIMITS.with(|l| l.cut.get().is_none()) {
                out = Some(text);
            }
        });
        out
    }
    /// Copy caller-produced text after charging its bytes to shared work.
    /// This grants no input credit and reserves no final result room. It lets
    /// a host build a computed record without allocating a key before admission.
    pub fn copy_text(&self, text: &str) -> Option<String> {
        let mut out = None;
        (self.operate)(&mut || {
            if step(text.len().saturating_add(1)) {
                out = Some(text.to_owned());
            }
        });
        out
    }
    /// Coerce borrowed JSON to text after a bounded preflight, so a deeply
    /// nested metadata key is refused before any temporary value is copied.
    pub fn coerce_json_string(&self, value: &serde_json::Value) -> Option<String> {
        let mut out = None;
        (self.operate)(&mut || {
            measure_schema(value);
            if LIMITS.with(|l| l.cut.get().is_some()) {
                return;
            }
            let value = Value::from_json(value);
            let text = to_string_in::<Supplied<'_>>(&data_value(&value));
            if !failed() && LIMITS.with(|l| l.cut.get().is_none()) {
                out = Some(text);
            }
        });
        out
    }
    /// JavaScript numeric coercion using this allowance and failure channel.
    /// Like `coerce_string`, this reserves no final result room.
    pub fn coerce_number(&self, value: &Value) -> Option<f64> {
        let mut out = None;
        (self.operate)(&mut || {
            let value = data_value(value);
            let primitive = to_primitive_in::<Supplied<'_>>(&value);
            let number = to_number(&primitive);
            if !failed() && LIMITS.with(|l| l.cut.get().is_none()) {
                out = Some(number);
            }
        });
        out
    }
    /// Prepare immutable schema indexes under shared depth/work limits.
    /// The borrow prevents schema mutation while this handle exists. Reuse it
    /// for repeated checks; schema admission/indexing is paid once per session.
    pub fn prepare_validation_schema<'s>(
        &self,
        schema: &'s serde_json::Value,
    ) -> Option<PreparedValidationSchema<'s>> {
        let mut out = None;
        (self.operate)(&mut || {
            measure_schema(schema);
            if LIMITS.with(|l| l.cut.get().is_some()) {
                return;
            }
            let before = LIMITS.with(|l| l.work.get());
            if let Some(mut prepared) = PreparedValidationSchema::build(schema) {
                prepared.index_work = before.saturating_sub(LIMITS.with(|l| l.work.get()));
                let key = Arc::as_ptr(&prepared.token) as usize;
                ACCOUNT.with(|a| {
                    a.borrow_mut().schemas.insert(key, prepared.token.clone());
                });
                out = Some(prepared);
            }
        });
        out
    }
    /// Strictly validate a value using prepared schema indexes. Any diagnostic
    /// rejects the complete value, even when a default could salvage it.
    /// Component references stay unchecked without a component catalog.
    /// Final caller projection charges result room. Foreign sessions pay the
    /// schema's bounded admission/indexing work before using it; callers cannot
    /// supply or forge prepaid sizes. Actual value walks still pay per check.
    pub fn check_value(
        &self,
        value: &Value,
        schema: &PreparedValidationSchema<'_>,
    ) -> (Option<Value>, Vec<String>) {
        self.validate_prepared(value, schema, "", true)
    }
    /// Validate and salvage a value, retaining schema defaults and pruned
    /// containers even when diagnostics were reported. `path` prefixes field
    /// diagnostics. Like `check_value`, this shares limits and grants no input
    /// credit; an invalid whole value returns None.
    pub fn validate_value(
        &self,
        value: &Value,
        schema: &PreparedValidationSchema<'_>,
        path: &str,
    ) -> (Option<Value>, Vec<String>) {
        self.validate_prepared(value, schema, path, false)
    }
    fn validate_prepared(
        &self,
        value: &Value,
        schema: &PreparedValidationSchema<'_>,
        path: &str,
        strict: bool,
    ) -> (Option<Value>, Vec<String>) {
        let mut out = None;
        let mut messages = Vec::new();
        (self.operate)(&mut || {
            let key = Arc::as_ptr(&schema.token) as usize;
            let admitted = ACCOUNT.with(|a| a.borrow().schemas.contains_key(&key));
            if !admitted {
                measure_schema(schema.root);
                if !step(schema.index_work) {
                    return;
                }
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    return;
                }
                ACCOUNT.with(|a| {
                    a.borrow_mut().schemas.insert(key, schema.token.clone());
                });
            }
            measure_data(value, false);
            if LIMITS.with(|l| l.cut.get().is_some()) {
                return;
            }
            let remaining = LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()));
            let validation_key =
                data_identity(value).map(|identity| (key, identity, path.to_owned(), strict));
            if validation_key.as_ref().is_some_and(|key| {
                ACCOUNT.with(|a| {
                    a.borrow()
                        .validated
                        .get(key)
                        .is_some_and(|(owner, depth)| owner.alive() && remaining >= *depth)
                })
            }) {
                step(1);
                out = Some(value.clone());
                return;
            }
            let table = crate::table::Table::default();
            let mut cache = crate::table::Cache::default();
            let limits = LIMITS.with(|l| {
                Limits::DEFAULT
                    .with_depth(l.max_depth.get())
                    .with_result_size(l.room.get())
            });
            static CATALOG: std::sync::OnceLock<crate::Library> = std::sync::OnceLock::new();
            let mut ctx = crate::materialize::Ctx {
                table: &table,
                cache: &mut cache,
                library: CATALOG.get_or_init(crate::Library::default),
                partial: false,
                visited: Vec::new(),
                current_statement_id: None,
                depth: 0,
                stack_cut: false,
                size: 0,
                limits,
                cuts: 0,
                cut_inside: false,
                frame: crate::materialize::Frame::default(),
            };
            let mut checked = value.clone();
            let invalid =
                crate::validation::validate_bounded_value(&mut checked, schema, "", path, &mut ctx);
            if ctx.frame.size_limited {
                LIMITS.with(|l| l.cut_by(Cut::Room));
            }
            let valid = ctx.frame.errors.is_empty();
            for error in &ctx.frame.errors {
                if let crate::materialize::ErrorPart::Own(error) = error {
                    messages.push(error.message.clone());
                }
            }
            if !invalid
                && (!strict || valid)
                && !ctx.stack_cut
                && LIMITS.with(|l| l.cut.get().is_none())
            {
                if valid
                    && data_identity(value).is_some()
                    && data_identity(&checked) == data_identity(value)
                {
                    let weak = match value {
                        Value::Obj(v) if Arc::strong_count(v) > 2 => {
                            Some(WeakValue::Obj(Arc::downgrade(v)))
                        }
                        Value::Arr(v) if Arc::strong_count(v) > 2 => {
                            Some(WeakValue::Arr(Arc::downgrade(v)))
                        }
                        _ => None,
                    };
                    if let (Some(key), Some(weak)) = (validation_key, weak) {
                        ACCOUNT.with(|a| {
                            let mut a = a.borrow_mut();
                            if a.validated.len() >= 4 {
                                a.validated.retain(|_, (w, _)| w.alive());
                            }
                            if a.validated.len() >= 8 {
                                a.validated.clear();
                            }
                            a.validated.insert(key, (weak, remaining));
                        });
                    }
                }
                out = Some(checked);
            }
        });
        (out, messages)
    }
    /// Whether this call inherits the caller's context. Public evaluation starts
    /// enabled. `@Each` passes it to its template; operators, collections and
    /// eager builtin-function arguments disable it. The core does not interpret that context:
    /// tree evaluation uses it for upstream's schema-aware call behavior.
    pub fn call_context(&self) -> bool {
        let mut enabled = false;
        (self.operate)(&mut || enabled = ACCOUNT.with(|a| a.borrow().call_context));
        enabled
    }
    /// Run callback evaluation with an explicit call context, restoring it on
    /// return or unwind. Returns the closure's result when the evaluator runs it.
    /// Tree props start a new context; statement defaults evaluate without one.
    pub fn with_call_context<T>(&self, enabled: bool, f: impl FnOnce() -> T) -> Option<T> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            let _context = CallContext::set(enabled);
            if let Some(f) = f.take() {
                out = Some(f());
            }
        });
        out
    }
    /// Resolve declarations against state without the caller's extra overlay.
    /// Uses this session's allowance and a separate state-name cache, restoring
    /// the overlay on return or unwind. Reference results keep their shared cache.
    /// This is for query defaults resolved outside a field write's `$value` scope.
    pub fn without_extra<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let mut f = Some(f);
        let mut out = None;
        (self.operate)(&mut || {
            let _extra = ExtraContext::disable();
            if let Some(f) = f.take() {
                out = Some(f());
            }
        });
        out
    }
    /// Mark a refused external value as cut when it is actually read.
    /// Flags remain sticky in this evaluator's session.
    pub fn mark_cut(&self, flags: EvaluationFlags) {
        (self.operate)(&mut || {
            LIMITS.with(|l| {
                if flags.size_limited {
                    l.cut_by(Cut::Room);
                }
                if flags.depth_limited {
                    l.cut_by(Cut::Depth);
                }
            })
        });
    }
    /// Sticky cuts affecting the shared allowance.
    pub fn flags(&self) -> EvaluationFlags {
        let mut flags = EvaluationFlags::default();
        (self.operate)(&mut || {
            record_flags();
            flags = ACCOUNT.with(|a| a.borrow().flags.get());
        });
        flags
    }
}

fn caller_level<T>(f: impl FnOnce() -> T) -> Option<T> {
    // T need not implement Debug (the test-only macro assertion requires it).
    if crate::stack::budget_low("caller_level") {
        if crate::stack::exhausted("caller_level") {
            return None;
        }
        return crate::stack::on_new_segment(|| caller_level(f));
    }
    let _levels = [Level::enter()?, Level::enter()?, Level::enter()?];
    if !step(1) {
        return None;
    }
    Some(f())
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

/// A failure of expression evaluation, independent of a component or prop.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EvaluationFailure {
    /// A caller callback's failure, using its host-facing wording.
    Callback(String),
    /// An object could not produce a primitive for JavaScript coercion.
    CannotConvertToPrimitive,
    /// An inherited entry in the builtins object has no callable `fn` member.
    BuiltinNotCallable,
    /// An untyped AST descriptor supplied an invalid field.
    #[non_exhaustive]
    InvalidExpression {
        /// The descriptor field, or `node` for the descriptor itself.
        field: &'static str,
        /// The operation that required that field.
        operation: ExpressionOperation,
        /// The supplied field's relevant type.
        actual: ExpressionType,
    },
}
/// The operation that failed on an untyped expression field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpressionOperation {
    /// Read an expression's kind.
    ReadKind,
    /// Map a collection of child expressions.
    Map,
    /// Map child expressions while substituting a loop variable.
    SubstituteMap,
    /// Read a call's positional argument count.
    Length,
    /// Destructure one object entry.
    Entry,
}
/// The distinction needed to identify an invalid expression field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpressionType {
    /// The field was absent or undefined.
    Undefined,
    /// The field was null.
    Null,
    /// A numeric field.
    Number,
    /// A boolean field, retaining its primitive identity.
    Boolean(bool),
    /// An object field.
    Object,
    /// The field had another incompatible type.
    Other,
}

/// One expression's result, with flags for cuts affecting that evaluation.
/// Equality compares the value, failure and cut flags, independently of the
/// representation provenance used for output accounting.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EvaluationResult {
    /// Undefined is `None`; null is `Some(Value::Null)`. Deferred ASTs and
    /// caller-owned containers retain their types and ownership.
    pub value: Option<Value>,
    /// The result reuses a public data container or deferred AST rather than
    /// constructing a new JavaScript object. Reused identity does not imply
    /// that distinct output positions share one result reservation.
    reused_data: bool,
    /// The first failure in this evaluation, including nested callback work.
    /// A failure stops expression evaluation and makes `value` undefined.
    /// Sequential evaluations start with no failure; cut flags remain sticky.
    pub failure: Option<EvaluationFailure>,
    /// Work or result room was exhausted, or a stall stopped evaluation.
    pub size_limited: bool,
    /// Logical depth or the grown-stack ceiling was reached.
    pub depth_limited: bool,
}
impl EvaluationResult {
    /// Whether the result reuses supplied data or a deferred AST. This is
    /// representation provenance for accounting, not part of result equality.
    /// Accounting provenance is read-only.
    /// ```compile_fail
    /// # use openui_lang::EvaluationResult;
    /// fn forge(mut result: EvaluationResult) {
    ///     result.reused_data = false;
    /// }
    /// ```
    pub fn reuses_data(&self) -> bool {
        self.reused_data
    }
}
impl PartialEq for EvaluationResult {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
            && self.failure == other.failure
            && self.size_limited == other.size_limited
            && self.depth_limited == other.depth_limited
    }
}

/// Several expression evaluations sharing work, result room and input credits.
///
/// A session is bound to one scope and caches lookups by name. Keep that scope
/// stable. Binding writes use a new session. Sequential operations such as an
/// action run use `with_scope` to keep work while refreshing scope lookups and
/// releasing each operation's output room.
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
    saved: Rc<RefCell<Saved>>,
    flags: Rc<Cell<EvaluationFlags>>,
}

impl<'a> EvaluationSession<'a> {
    /// Run a sequential operation against a fresh scope and fresh name caches,
    /// sharing this session's work, input credits and sticky cut flags. Output
    /// room spent by the operation is released on return; work is not. The
    /// scope and its borrowed values do not escape through the session.
    pub fn with_scope<T>(
        &self,
        scope: &dyn EvaluationScope,
        f: impl FnOnce(&EvaluationSession<'_>) -> T,
    ) -> T {
        self.root_context(|| {
            let _room = SequentialRoom::enter(scope.extra_is_object());
            let session = EvaluationSession {
                names: Supplied {
                    scope,
                    read: RefCell::new(HashMap::new()),
                    callbacks: RefCell::new(Vec::new()),
                },
                saved: self.saved.clone(),
                flags: self.flags.clone(),
            };
            f(&session)
        })
    }
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
        let extra_object = scope.extra_is_object();
        ACCOUNT.with(|a| {
            let mut account = Account::new(limits, input_bytes);
            account.extra_object = extra_object;
            *a.borrow_mut() = account;
        });
        let flags = ACCOUNT.with(|a| a.borrow().flags.clone());
        saved.swap();
        saved.restore = false;
        Self {
            names: Supplied {
                scope,
                read: RefCell::new(HashMap::new()),
                callbacks: RefCell::new(Vec::new()),
            },
            saved: Rc::new(RefCell::new(saved)),
            flags,
        }
    }
    /// Evaluate another expression under the same allowance, including when
    /// re-entered from this session's own callback. Re-entry uses the callback's
    /// caches, remaining work and room, and current depth. Call and per-item
    /// callbacks retain enclosing loop bindings; lookup callbacks use the root.
    pub fn evaluate(&self, expression: &Ast) -> EvaluationResult {
        let frame = self.names.callbacks.borrow().last().cloned();
        if let Some(frame) = frame {
            return self.names.resume(&frame, expression, false).0;
        }
        let outer = DepthContext::current();
        let active = ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &self.flags));
        let _resume = if active {
            None
        } else {
            Some(Resume::for_account(&self.saved, &self.flags))
        };
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        let operate = |f: &mut dyn FnMut()| self.root_context(f);
        evaluate_result(expression, &Scope::Root(&self.names), false, &operate).0
    }
    /// Run a caller's walk under this allowance. Use `with_level` at each
    /// recursive hop and the reservation methods for caller-produced output.
    /// State reads still go through evaluated expressions and EvaluationScope.
    pub fn with_evaluator<T>(&self, f: impl FnOnce(&ExpressionEvaluator<'_>) -> T) -> T {
        let frame = self.names.callbacks.borrow().last().cloned();
        if let Some(frame) = frame {
            return self
                .names
                .resume_context(&frame, || self.names.evaluator(&frame, f));
        }
        self.root_context(|| {
            let operate = |f: &mut dyn FnMut()| self.root_context(f);
            let eval = |node: &Ast, project: bool| {
                self.root_context(|| {
                    evaluate_result(node, &Scope::Root(&self.names), project, &operate)
                })
            };
            let value = |node: &Value, call_prop: bool, project: bool| {
                self.root_context(|| {
                    let _entry = FailureEntry::enter();
                    if call_prop && matches!(node, Value::Null) {
                        fail(EvaluationFailure::InvalidExpression {
                            field: "callProp",
                            operation: ExpressionOperation::ReadKind,
                            actual: ExpressionType::Null,
                        });
                    }
                    evaluate_value_result(node, &Scope::Root(&self.names), project, &operate)
                })
            };
            let transform = |node: &Value, map: &mut ValueTransform<'_>| {
                self.root_context(|| {
                    in_scope_transform(node, &[], &Scope::Root(&self.names), map, &operate)
                })
            };
            let fields = |fields: &JsObject<Value>| {
                self.root_context(|| evaluate_fields(fields, &[], &Scope::Root(&self.names)))
            };
            f(&ExpressionEvaluator {
                fields: &fields,
                evaluate: &eval,
                evaluate_value: &value,
                operate: &operate,
                transform: &transform,
                projection: None,
            })
        })
    }
    fn root_context<T>(&self, f: impl FnOnce() -> T) -> T {
        let outer = DepthContext::current();
        let active = ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &self.flags));
        let _resume = if active {
            None
        } else {
            Some(Resume::for_account(&self.saved, &self.flags))
        };
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        let out = f();
        record_flags();
        out
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

// Final literal strings need no intermediate allocation when even their
// unescaped bytes cannot fit. Leave transforms and nested literals alone.
// This is a lower-bound check, not an extra work or output reservation.
fn literal_copy_fits(text: &str) -> bool {
    LIMITS.with(|l| {
        // Let normal evaluator/projection entry select depth or entry-work cuts.
        if l.cut.get().is_some()
            || l.depth.get().saturating_add(3) >= l.max_depth.get()
            || l.work.get() < 3
        {
            return true;
        }
        if text.len().saturating_add(2) > builder_room() {
            l.cut_by(Cut::Room);
            false
        } else if text.len() > l.work.get() {
            l.cut_by(Cut::Work);
            false
        } else {
            true
        }
    })
}
fn literal_text(value: &Value) -> Option<&str> {
    match value {
        Value::Ast(a) => {
            if let Ast::Str(s) = a.as_ref() {
                Some(s)
            } else {
                None
            }
        }
        Value::Obj(o) => match (o.get("k"), o.get("v")) {
            (Some(Value::Str(k)), Some(Value::Str(s)))
                if matches!(k.as_str(), "Str" | "Num" | "Bool") =>
            {
                Some(s)
            }
            _ => None,
        },
        _ => None,
    }
}
fn evaluate_result(
    expression: &Ast,
    scope: &Scope<Supplied<'_>>,
    project: bool,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> (EvaluationResult, bool) {
    let transform_before = ACCOUNT.with(|a| a.borrow().transformed);
    let _evaluation = FailureEntry::enter();
    let _projection = ProjectionEntry::enter();
    if !project
        && let Ast::Str(text) = expression
        && !literal_copy_fits(text)
    {
        return (result(Some(Value::Null)), false);
    }
    let js = evaluate(expression, scope);
    let (js, resolved) = if project && !failed() && LIMITS.with(|limits| limits.cut.get().is_none())
    {
        project_result(js, scope, operate)
    } else {
        (js, false)
    };
    let transformed = ACCOUNT.with(|a| {
        let n = a.borrow().transformed;
        n != transform_before || n == usize::MAX
    });
    let _buffers = BufferProjection::enter(transformed || resolved);
    let value = if failed() { None } else { runtime_value(&js) };
    let value = if transformed && LIMITS.with(|l| l.cut.get().is_some()) {
        None
    } else {
        value
    };
    if let Some(v) = &value {
        remember_result(v, &js);
    }
    let mut result = result(value);
    result.reused_data = matches!(js, Js::Data(_) | Js::Props(_) | Js::Ast(_));
    (result, resolved)
}
fn evaluate_value_result(
    expression: &Value,
    scope: &Scope<Supplied<'_>>,
    project: bool,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> (EvaluationResult, bool) {
    let transform_before = ACCOUNT.with(|a| a.borrow().transformed);
    let _evaluation = FailureEntry::enter();
    let _projection = ProjectionEntry::enter();
    if !project
        && let Some(text) = literal_text(expression)
        && !literal_copy_fits(text)
    {
        return (result(Some(Value::Null)), false);
    }
    let js = super::descriptor::evaluate(Some(expression), scope);
    let (js, resolved) = if project && !failed() && LIMITS.with(|limits| limits.cut.get().is_none())
    {
        project_result(js, scope, operate)
    } else {
        (js, false)
    };
    let transformed = ACCOUNT.with(|a| {
        let n = a.borrow().transformed;
        n != transform_before || n == usize::MAX
    });
    let _buffers = BufferProjection::enter(transformed || resolved);
    let value = if failed() { None } else { runtime_value(&js) };
    let value = if transformed && LIMITS.with(|l| l.cut.get().is_some()) {
        None
    } else {
        value
    };
    if let Some(v) = &value {
        remember_result(v, &js);
    }
    let mut result = result(value);
    result.reused_data = matches!(js, Js::Data(_) | Js::Props(_) | Js::Ast(_));
    (result, resolved)
}
fn project_result(
    js: Js,
    scope: &Scope<Supplied<'_>>,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> (Js, bool) {
    let mut root = scope;
    let names = loop {
        match root {
            Scope::Root(names) => break names,
            Scope::Each { parent, .. } => root = parent,
        }
    };
    let flags = ACCOUNT.with(|a| a.borrow().flags.clone());
    let owned = |f: &mut dyn FnMut()| {
        let mut checked = || {
            if !failed() && LIMITS.with(|limits| limits.cut.get().is_none()) {
                f();
            }
        };
        if ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &flags)) {
            checked();
        } else {
            operate(&mut checked);
        }
    };
    let state = |name: &str| {
        let mut js = Js::Undefined;
        owned(&mut || {
            let _extra = ExtraContext::disable();
            js = names.get(name, false);
        });
        ExpressionValue {
            value: std::borrow::Cow::Owned(js),
            operate: Some(&owned),
        }
    };
    let replacement = names.scope.project_result(
        ExpressionValue {
            value: std::borrow::Cow::Borrowed(&js),
            operate: Some(&owned),
        },
        &state,
    );
    match replacement {
        Some(value) => (value.value.into_owned(), true),
        None => (js, false),
    }
}
/// Report only the first failure of the active expression chain.
pub(super) fn fail(failure: EvaluationFailure) {
    ACCOUNT.with(|a| {
        let mut a = a.borrow_mut();
        if a.failure.is_none() {
            a.failure = Some(failure);
        }
    });
}
pub(super) fn failed() -> bool {
    ACCOUNT.with(|a| a.borrow().failure.is_some())
}
struct FailureEntry;
impl FailureEntry {
    fn enter() -> Self {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            if a.evaluations == 0 {
                a.failure = None;
            }
            a.evaluations = a.evaluations.saturating_add(1);
        });
        Self
    }
}
impl Drop for FailureEntry {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.evaluations = a.evaluations.saturating_sub(1);
        });
    }
}
fn in_scope_value(
    node: &Value,
    bindings: &[(String, Js)],
    scope: &Scope<Supplied<'_>>,
    project: bool,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> (EvaluationResult, bool) {
    crate::stack::grow_stack!(in_scope_value(node, bindings, scope, project, operate), budget else (result(Some(Value::Null)), false));
    if let Some(((name, item), rest)) = bindings.split_first() {
        if !step(1) {
            return (result(Some(Value::Null)), false);
        }
        in_scope_value(
            node,
            rest,
            &Scope::Each {
                name,
                item,
                parent: scope,
            },
            project,
            operate,
        )
    } else {
        evaluate_value_result(node, scope, project, operate)
    }
}
#[derive(PartialEq, Eq)]
enum FieldIdentity {
    Undefined,
    Null,
    Bool(bool),
    Num(u64),
    Shared(u8, usize),
}
struct FieldCache {
    key: Vec<(String, FieldIdentity)>,
    value: Value,
}
fn evaluate_fields(
    fields: &JsObject<Value>,
    bindings: &[(String, Js)],
    scope: &Scope<Supplied<'_>>,
) -> EvaluationResult {
    crate::stack::grow_stack!(evaluate_fields(fields, bindings, scope), budget else result(None));
    if let Some(((name, item), rest)) = bindings.split_first() {
        if !step(1) {
            return result(None);
        }
        return evaluate_fields(
            fields,
            rest,
            &Scope::Each {
                name,
                item,
                parent: scope,
            },
        );
    }
    let _texts = ArgumentTexts::enter();
    let _failure = FailureEntry::enter();
    let _projection = ProjectionEntry::enter();
    let mut out = Vec::with_capacity(fields.len().min(4));
    let mut key = Some(Vec::with_capacity(fields.len().min(4)));
    let mut supplied = false;
    for (name, node) in fields.iter() {
        if failed() || !step(1) {
            break;
        }
        let value = super::descriptor::evaluate(Some(node), scope);
        let identity = match &value {
            Js::Undefined => Some(FieldIdentity::Undefined),
            Js::Null => Some(FieldIdentity::Null),
            Js::Bool(b) => Some(FieldIdentity::Bool(*b)),
            Js::Num(n) => Some(FieldIdentity::Num(n.to_bits())),
            Js::Str(s)
                if ACCOUNT
                    .with(|a| a.borrow().supplied_strings.contains(&(s.as_ptr() as usize))) =>
            {
                supplied = true;
                Some(FieldIdentity::Shared(4, s.as_ptr() as usize))
            }
            Js::Data(v) => data_identity(v)
                .filter(|k| ACCOUNT.with(|a| a.borrow().inputs.contains(k)))
                .map(|(tag, ptr)| {
                    supplied = true;
                    FieldIdentity::Shared(tag, ptr)
                }),
            _ => None,
        };
        if let (Some(keys), Some(identity)) = (&mut key, identity) {
            keys.push((name.clone(), identity));
        } else {
            key = None;
        }
        out.push((name.clone(), value));
    }
    let key = key.filter(|_| supplied && !failed() && LIMITS.with(|l| l.cut.get().is_none()));
    if let Some(key) = &key {
        let hit = ACCOUNT.with(|a| {
            a.borrow()
                .field_cache
                .as_ref()
                .filter(|v| &v.key == key)
                .map(|v| v.value.clone())
        });
        if let Some(value) = hit {
            // A successful cached admission can be reused only at a position
            // with sufficient remaining depth. measure_data enforces this.
            let bytes = measure_data(&value, false);
            if !ACCOUNT.with(|a| a.borrow().temporary_room.last().is_some_and(|v| v.1)) {
                spend(bytes);
            }
            return result(Some(value));
        }
    }
    // Discard the old projection before allocating a replacement. Keep only
    // the current combination: a cache miss must not retain copies of text.
    ACCOUNT.with(|a| a.borrow_mut().field_cache = None);
    let value = if failed() {
        None
    } else if !ACCOUNT.with(|a| a.borrow().temporary_room.last().is_some_and(|v| v.1)) {
        let mut object = JsObject::new();
        for (name, value) in out {
            object.define(name, value);
        }
        runtime_value(&Js::Obj(Arc::new(object)))
    } else {
        let mut fields = JsObject::new();
        for (name, child) in out {
            if let Some(value) = runtime_value(&child) {
                fields.define(name, value);
            }
        }
        Some(Value::Obj(Arc::new(fields)))
    };
    if let (Some(key), Some(value)) = (key, &value)
        && LIMITS.with(|l| l.cut.get().is_none())
    {
        // Bound retained projection metadata independently of call count.
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.field_cache = Some(FieldCache {
                key,
                value: value.clone(),
            });
        });
    }
    result(value)
}
fn in_scope_transform(
    node: &Value,
    bindings: &[(String, Js)],
    scope: &Scope<Supplied<'_>>,
    map: &mut ValueTransform<'_>,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> ExpressionOutput {
    crate::stack::grow_stack!(in_scope_transform(node,bindings,scope,map,operate), budget else Some(Value::Null).into());
    if let Some(((name, item), rest)) = bindings.split_first() {
        if !step(1) {
            return Some(Value::Null).into();
        }
        return in_scope_transform(
            node,
            rest,
            &Scope::Each {
                name,
                item,
                parent: scope,
            },
            map,
            operate,
        );
    }
    let _failure = FailureEntry::enter();
    let _projection = ProjectionEntry::enter();
    let js = super::descriptor::evaluate(Some(node), scope);
    let js = if !failed() && LIMITS.with(|l| l.cut.get().is_none()) {
        map(ExpressionValue {
            value: std::borrow::Cow::Owned(js),
            operate: Some(operate),
        })
        .value
        .into_owned()
    } else {
        Js::Undefined
    };
    let js = if LIMITS.with(|l| l.cut.get().is_some()) {
        Js::Undefined
    } else {
        js
    };
    // Forward reservations of Each replacements until the owning projection.
    let credit = ACCOUNT.with(|a| {
        a.borrow_mut()
            .projection_room
            .last_mut()
            .map(std::mem::take)
            .unwrap_or(0)
    });
    drop(_projection);
    ACCOUNT.with(|a| {
        if let Some(parent) = a.borrow_mut().projection_room.last_mut() {
            *parent = parent.saturating_add(credit)
        }
    });
    ExpressionOutput {
        value: Output::Evaluated(js),
    }
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
            reused_data: false,
            failure: ACCOUNT.with(|a| a.borrow().failure.clone()),
            size_limited: l.size_limited.get() || crate::progress::stalled_so_far(),
            depth_limited: l.depth_limited.get(),
        }
    })
}

struct CachedRead {
    value: Js,
    depth: u32,
    height: u32,
}
fn recheck_read(value: &Js) {
    crate::stack::grow_stack!(recheck_read(value), budget else ());
    let _mode = DepthOnly::enter();
    match value {
        Js::Data(v) => {
            measure_data(v, false);
        }
        Js::Ast(a) => {
            expression_bytes(a);
        }
        Js::Props(e) => {
            measure_data(&Value::Element(e.clone()), false);
        }
        Js::Arr(a) => {
            let Some(_level) = Level::enter() else {
                return;
            };
            for value in a.iter() {
                if !step(1) {
                    break;
                }
                recheck_read(value);
            }
        }
        Js::Obj(o) => {
            let Some(_level) = Level::enter() else {
                return;
            };
            for value in o.values() {
                if !step(1) {
                    break;
                }
                recheck_read(value);
            }
        }
        _ => {}
    }
}
type ReadKey = (bool, Box<str>, bool, bool);
pub(super) struct Supplied<'a> {
    scope: &'a dyn EvaluationScope,
    read: RefCell<HashMap<ReadKey, CachedRead>>,
    callbacks: RefCell<Vec<Rc<Suspension>>>,
}
struct Suspension {
    saved: Rc<RefCell<Saved>>,
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
    project: bool,
    operate: &dyn Fn(&mut dyn FnMut()),
) -> (EvaluationResult, bool) {
    crate::stack::grow_stack!(in_scope(node, bindings, scope, project, operate), budget else (result(Some(Value::Null)), false));
    if let Some(((name, item), rest)) = bindings.split_first() {
        if !step(1) {
            return evaluate_result(&Ast::Null, scope, project, operate);
        }
        in_scope(
            node,
            rest,
            &Scope::Each {
                name,
                item,
                parent: scope,
            },
            project,
            operate,
        )
    } else {
        evaluate_result(node, scope, project, operate)
    }
}
impl Supplied<'_> {
    fn callback(
        &self,
        scope: &Scope<Self>,
        callback: impl FnOnce(&ExpressionEvaluator<'_>) -> Option<Value>,
    ) -> Option<Value> {
        self.callback_output(scope, |eval| callback(eval).into())
            .data()
    }
    fn callback_output(
        &self,
        scope: &Scope<Self>,
        callback: impl FnOnce(&ExpressionEvaluator<'_>) -> ExpressionOutput,
    ) -> ExpressionOutput {
        let mut bindings = Vec::new();
        let mut parent = scope;
        while let Scope::Each {
            name,
            item,
            parent: next,
        } = parent
        {
            if !step(name.len().saturating_add(1)) {
                return Some(Value::Null).into();
            }
            bindings.push(((*name).to_owned(), (*item).clone()));
            parent = next;
        }
        bindings.reverse();
        let flags = ACCOUNT.with(|a| a.borrow().flags.clone());
        let frame = Rc::new(Suspension {
            saved: Rc::new(RefCell::new(Saved::enter())),
            bindings,
            flags,
        });
        self.callbacks.borrow_mut().push(frame.clone());
        let guard = CallbackGuard(&self.callbacks);
        DepthContext::suspend(&frame.saved.borrow());
        let value = self.evaluator(&frame, callback);
        if DEPTH.with(|d| d.borrow().cut.get()) {
            frame.saved.borrow().limits.cut_by(Cut::Depth);
        }
        drop(guard);
        drop(frame);
        ACCOUNT.with(|a| a.borrow_mut().collect_bridges());
        value
    }
    fn evaluator<T>(&self, frame: &Suspension, f: impl FnOnce(&ExpressionEvaluator<'_>) -> T) -> T {
        let operate = |f: &mut dyn FnMut()| self.resume_context(frame, f);
        let eval = |node: &Ast, project: bool| self.resume(frame, node, project);
        let value = |node: &Value, call_prop: bool, project: bool| {
            self.resume_context(frame, || {
                let _entry = FailureEntry::enter();
                if call_prop && matches!(node, Value::Null) {
                    fail(EvaluationFailure::InvalidExpression {
                        field: "callProp",
                        operation: ExpressionOperation::ReadKind,
                        actual: ExpressionType::Null,
                    });
                }
                in_scope_value(node, &frame.bindings, &Scope::Root(self), project, &operate)
            })
        };
        let transform = |node: &Value, map: &mut ValueTransform<'_>| {
            self.resume_context(frame, || {
                in_scope_transform(node, &frame.bindings, &Scope::Root(self), map, &operate)
            })
        };
        let fields = |fields: &JsObject<Value>| {
            self.resume_context(frame, || {
                evaluate_fields(fields, &frame.bindings, &Scope::Root(self))
            })
        };
        f(&ExpressionEvaluator {
            fields: &fields,
            evaluate: &eval,
            evaluate_value: &value,
            operate: &operate,
            transform: &transform,
            projection: None,
        })
    }
    fn resume_context<T>(&self, frame: &Suspension, f: impl FnOnce() -> T) -> T {
        let outer = DepthContext::current();
        let active = ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &frame.flags));
        let _resume = if active {
            None
        } else {
            Some(Resume::for_account(&frame.saved, &frame.flags))
        };
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        let out = f();
        record_flags();
        out
    }
    fn resume(&self, frame: &Suspension, node: &Ast, project: bool) -> (EvaluationResult, bool) {
        let outer = DepthContext::current();
        let active = ACCOUNT.with(|a| Rc::ptr_eq(&a.borrow().flags, &frame.flags));
        // The saved frame holds this allowance while suspended, and the
        // callback's context while resumed. The session's name cache is shared.
        let _resume = if active {
            None
        } else {
            Some(Resume::for_account(&frame.saved, &frame.flags))
        };
        let _depth = DepthEntry::enter(outer);
        let _stack = crate::stack::expression_limit(
            DepthContext::total_limit().div_ceil(VALUE_LEVELS_PER_LEVEL),
            DEPTH.with(|d| d.borrow().stack_base),
        );
        let operate = |f: &mut dyn FnMut()| self.resume_context(frame, f);
        in_scope(node, &frame.bindings, &Scope::Root(self), project, &operate)
    }
    fn remember_read(&self, key: ReadKey, value: Js) {
        self.remember_read_height(key, value, 0);
    }
    fn remember_read_height(&self, key: ReadKey, value: Js, height: usize) {
        if let Js::Str(text) = &value
            && ACCOUNT.with(|a| a.borrow().argument_texts)
        {
            ACCOUNT.with(|a| {
                a.borrow_mut()
                    .supplied_strings
                    .insert(text.as_ptr() as usize)
            });
        }
        let depth = LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()));
        self.read.borrow_mut().insert(
            key,
            CachedRead {
                value,
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
                height: u32::try_from(height).unwrap_or(0),
            },
        );
    }
    fn cached_read(&self, key: &ReadKey) -> Option<Js> {
        let (value, checked, prepared_height) = self
            .read
            .borrow()
            .get(key)
            .map(|v| (v.value.clone(), v.depth, v.height))?;
        let depth = LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()));
        let required = if prepared_height != 0 {
            // A prepared read admits no body here. Its first read is lazy too;
            // member/index receivers may consume only a shallow child. Full
            // projections and argument admission check their actual values.
            None
        } else if let Js::Data(v) = &value {
            data_identity(v).and_then(|key| {
                ACCOUNT.with(|a| {
                    let a = a.borrow();
                    if let Some((_, height)) = a.inputs.measurement(&key) {
                        return Some(height.saturating_add(1));
                    }
                    a.argument_bodies
                        .get(&key)
                        .filter(|v| v.owner.alive() && v.exact)
                        .map(|v| (v.depth as usize).saturating_add(1))
                })
            })
        } else {
            None
        };
        if let Some(required) = required {
            if (checked == u32::MAX || depth < checked as usize) && depth < required {
                LIMITS.with(|l| l.cut_by(Cut::Depth));
            }
        } else if prepared_height == 0 && (checked == u32::MAX || depth < checked as usize) {
            recheck_read(&value);
            if LIMITS.with(|l| l.cut.get().is_none())
                && let Some(cached) = self.read.borrow_mut().get_mut(key)
            {
                cached.depth = u32::try_from(depth).unwrap_or(u32::MAX);
            }
        }
        if let Js::Str(text) = &value
            && ACCOUNT.with(|a| a.borrow().argument_texts)
        {
            ACCOUNT.with(|a| {
                a.borrow_mut()
                    .supplied_strings
                    .insert(text.as_ptr() as usize);
            });
        }
        Some(value)
    }
    fn object_extra(&self, name: &str, receiver: bool) -> Js {
        let key = (false, name.into(), true, true);
        if let Some(value) = {
            let _position = ReadPosition::enter(receiver);
            self.cached_read(&key)
        } {
            return value;
        }
        let value = self.callback(&Scope::Root(self), |eval| self.scope.extra(name, eval));
        if failed() {
            return Js::Undefined;
        }
        let value = match value {
            Some(Value::Null) => Js::Null,
            Some(value) => {
                {
                    let _position = ReadPosition::enter(receiver);
                    register_input(&value);
                }
                data_value(&value)
            }
            None if OBJECT_PROTOTYPE_KEYS.contains(&name) => {
                property(&Js::Obj(Arc::new(JsObject::new())), name)
            }
            None => Js::Undefined,
        };
        // Missing own fields need no second cache entry; the combined state
        // lookup caches their fallback. Actual extra values are credited once.
        if !matches!(value, Js::Undefined) {
            let _position = ReadPosition::enter(receiver);
            self.remember_read(key, value.clone());
        }
        value
    }
    fn get(&self, name: &str, reference: bool) -> Js {
        let receiver =
            ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().receiver_read, false));
        self.get_at(name, reference, receiver)
    }
    fn get_at(&self, name: &str, reference: bool, receiver: bool) -> Js {
        let (extra_enabled, extra_object) = ACCOUNT.with(|a| {
            let a = a.borrow();
            (reference || a.extra_enabled, a.extra_object)
        });
        let key = (reference, name.into(), extra_enabled, false);
        let cached = !reference || self.scope.cache_references();
        if cached
            && let Some(value) = {
                let _position = ReadPosition::enter(receiver);
                self.cached_read(&key)
            }
        {
            return value;
        }
        if !reference && extra_enabled && extra_object {
            let extra = self.object_extra(name, receiver);
            if failed() {
                return Js::Undefined;
            }
            let value = if is_nullish(&extra) {
                let _extra = ExtraContext::disable();
                self.get_at(name, false, receiver)
            } else {
                extra
            };
            {
                let _position = ReadPosition::enter(receiver);
                self.remember_read(key, value.clone());
            }
            return value;
        }
        if reference && let Some(prepared) = self.scope.prepared_reference(name) {
            // The prepared set was paid before this session. A claim larger
            // than the admitted input cannot use the fast path.
            if ACCOUNT.with(|a| prepared.bytes() > a.borrow().bytes) {
                LIMITS.with(|l| l.cut_by(Cut::Work));
                return Js::Null;
            }
            if !step(1) {
                return Js::Null;
            }
            let js = prepared.value().clone();
            if let Js::Str(text) = &js
                && ACCOUNT.with(|a| a.borrow().argument_texts)
            {
                ACCOUNT.with(|a| {
                    a.borrow_mut()
                        .supplied_strings
                        .insert(text.as_ptr() as usize)
                });
            }
            if cached {
                self.remember_read_height(key, js.clone(), prepared.depth().max(1));
            }
            return js;
        }
        let value = self.callback(&Scope::Root(self), |eval| {
            if reference {
                self.scope.reference(name, eval)
            } else {
                let extra = if extra_enabled {
                    self.scope.extra(name, eval)
                } else {
                    None
                };
                // Callback evaluation is suspended here: its first failure is
                // in this frame's saved allowance, not the caller's context.
                let failed = self
                    .callbacks
                    .borrow()
                    .last()
                    .is_some_and(|frame| frame.saved.borrow().account.failure.is_some());
                if failed {
                    None
                } else {
                    extra
                        .filter(|v| !matches!(v, Value::Null))
                        .or_else(|| self.scope.state(name, eval))
                }
            }
        });
        if failed() {
            return Js::Undefined;
        }
        if let Some(value) = &value {
            let _position = ReadPosition::enter(receiver);
            if reference {
                measure_boundary(value);
            } else {
                register_input(value);
            }
        }
        let js = value.as_ref().map(data_value).unwrap_or(Js::Undefined);
        if cached {
            let _position = ReadPosition::enter(receiver);
            self.remember_read(key, js.clone());
        }
        js
    }
}
// Extra scope is an object lookup, while state is Map.get. A non-string
// descriptor name can match the extra object after coercion, but cannot match
// the string-key store. With no extra object it must not coerce at all.
pub(super) fn extra_state(name: Option<&Value>, scope: &Scope<Supplied<'_>>) -> Js {
    if !ACCOUNT.with(|a| {
        let a = a.borrow();
        a.extra_enabled && a.extra_object
    }) {
        return Js::Undefined;
    }
    let mut root = scope;
    let names = loop {
        match root {
            Scope::Root(names) => break names,
            Scope::Each { parent, .. } => root = parent,
        }
    };
    let value = name.map(data_value).unwrap_or(Js::Undefined);
    let key = to_string_in::<Supplied<'_>>(&value);
    if failed() {
        return Js::Undefined;
    }
    let value = names.object_extra(&key, false);
    if is_nullish(&value) {
        Js::Undefined
    } else {
        value
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
    fn each_item(&self, mut js: Js, scope: &Scope<Self>) -> Js {
        if failed() {
            return Js::Undefined;
        }
        let context = ACCOUNT.with(|a| a.borrow().call_context);
        let view = ExpressionValue {
            value: std::borrow::Cow::Borrowed(&js),
            operate: None,
        };
        if !self.scope.wants_each_item(&view, context) {
            return js;
        }
        // A data projection is already public data. Move a unique owner out
        // instead of retaining a second copy across the per-item callback.
        let moved = if let Js::Data(data) = &mut js {
            Arc::get_mut(data).map(|v| std::mem::replace(v, Value::Null))
        } else {
            None
        };
        let was_moved = moved.is_some();
        let mut value =
            moved.or_else(|| LIMITS.with(|limits| to_value_mode::<true, false>(&js, limits)));
        let before = room_used();
        let retained_before = ACCOUNT.with(|a| a.borrow().retained_room);
        // Callback projections belong to this item, never an enclosing
        // temporary argument buffer. On unwind/cut keep all reservations.
        let _temporary = ItemRoom::enter();
        let replacement = self.callback(scope, |eval| self.scope.each_item_owned(&mut value, eval));
        if failed() {
            return Js::Undefined;
        }
        if let Some(value) = replacement {
            let bytes = measure_boundary(&value);
            // Keep this reservation while the loop buffers replacements. The
            // owning expression releases their final footprints only when its
            // projection is ready to charge them; nested argument projections
            // cannot release earlier items still retained by the outer loop.
            let retained =
                ACCOUNT.with(|a| a.borrow().retained_room.saturating_sub(retained_before));
            let spent = room_used().saturating_sub(before).saturating_sub(retained);
            if LIMITS.with(|l| l.cut.get().is_none()) {
                release_substitution();
                if bytes > spent {
                    spend(bytes.saturating_sub(spent));
                } else {
                    LIMITS.with(|l| l.room.set(l.room.get().saturating_add(spent - bytes)));
                }
                ACCOUNT.with(|a| {
                    if let Some(room) = a.borrow_mut().projection_room.last_mut() {
                        *room = room.saturating_add(bytes);
                    }
                });
            }
            data_value(&value)
        } else {
            if was_moved
                && let Js::Data(data) = &mut js
                && let Some(data) = Arc::get_mut(data)
            {
                *data = value.take().unwrap_or(Value::Null);
            }
            js
        }
    }
    fn direct_each_reference(&self, value: &Js) -> Option<Js> {
        if !self.scope.allow_direct_each_reference()
            || !ACCOUNT.with(|a| a.borrow().call_context)
            || LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()) < 64)
            || !literal_identity_element(value)
        {
            return None;
        }
        Some(expand(value))
    }
    fn wants_unmapped_call(&self, node: &Value, name: &str) -> bool {
        self.scope.wants_unmapped_call(node, name)
    }
    fn call_value(&self, node: &Value, name: &str, scope: &Scope<Self>) -> Js {
        admit_output(
            self.callback_output(scope, |eval| self.scope.call_value_output(node, name, eval)),
        )
    }
    fn call(&self, node: &Ast, scope: &Scope<Self>) -> Js {
        if let Ast::Comp {
            name, mapped_props, ..
        } = node
            && OBJECT_PROTOTYPE_KEYS.contains(&name.as_str())
        {
            let _context = CallContext::set(false);
            if let Some(props) = mapped_props {
                for arg in props.values() {
                    if failed() || LIMITS.with(|l| l.cut.get().is_some()) {
                        break;
                    }
                    super::evaluate(arg, scope);
                }
            }
            if failed() || LIMITS.with(|l| l.cut.get().is_some()) {
                return Js::Null;
            }
            fail(EvaluationFailure::BuiltinNotCallable);
            return Js::Undefined;
        }
        admit_output(self.callback_output(scope, |eval| self.scope.call_output(node, eval)))
    }
}
// This is a representation shortcut, not evaluation: the existing per-item
// hook still evaluates each prop once. Reject unknown fields, prototypes and
// expressions whose literal expansion could change dispatch or exceed 64 levels.
fn literal_identity_element(value: &Js) -> bool {
    fn ast(a: &Ast, n: usize) -> bool {
        if n == 0 || !step(1) {
            return false;
        }
        match a {
            Ast::Str(_) | Ast::Num(_) | Ast::Bool(_) | Ast::Null | Ast::StateRef(_) => true,
            Ast::BinOp { left, right, .. } => ast(left, n - 1) && ast(right, n - 1),
            Ast::UnaryOp { operand, .. } => ast(operand, n - 1),
            _ => false,
        }
    }
    fn node(v: &Value, n: usize) -> bool {
        if n == 0 || !step(1) {
            return false;
        }
        match v {
            Value::Ast(a) => ast(a, n - 1),
            Value::Obj(o) => {
                let Some(Value::Str(k)) = o.get("k") else {
                    return false;
                };
                match k.as_str() {
                    "Num" => o.len() == 2 && matches!(o.get("v"), Some(Value::Num(_))),
                    "Str" => o.len() == 2 && matches!(o.get("v"), Some(Value::Str(_))),
                    "Bool" => o.len() == 2 && matches!(o.get("v"), Some(Value::Bool(_))),
                    "Null" => o.len() == 1,
                    "StateRef" => o.len() == 2 && matches!(o.get("n"), Some(Value::Str(_))),
                    "BinOp" => {
                        o.len() == 4
                            && matches!(o.get("op"), Some(Value::Str(_)))
                            && o.get("left").is_some_and(|v| node(v, n - 1))
                            && o.get("right").is_some_and(|v| node(v, n - 1))
                    }
                    "UnaryOp" => {
                        o.len() == 3
                            && matches!(o.get("op"), Some(Value::Str(_)))
                            && o.get("operand").is_some_and(|v| node(v, n - 1))
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
    fn props(p: &JsObject<Value>) -> bool {
        p.len() <= 8
            && p.iter().all(|(key, value)| {
                key != "__proto__"
                    && match value {
                        Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_) => step(1),
                        _ => node(value, 6),
                    }
            })
    }
    let Js::Data(value) = value else { return false };
    match value.as_ref() {
        Value::Element(e) => e.has_dynamic_props && props(&e.props),
        Value::Obj(o) => {
            o.len() <= 6
                && o.iter().all(|(key, _)| {
                    matches!(
                        key.as_str(),
                        "type"
                            | "typeName"
                            | "props"
                            | "partial"
                            | "hasDynamicProps"
                            | "statementId"
                    )
                })
                && matches!(o.get("type"),Some(Value::Str(s)) if s=="element")
                && matches!(o.get("typeName"), Some(Value::Str(_)))
                && matches!(o.get("partial"), Some(Value::Bool(_)))
                && matches!(o.get("hasDynamicProps"), Some(Value::Bool(true)))
                && o.get("statementId")
                    .is_none_or(|v| matches!(v, Value::Str(_)))
                && matches!(o.get("props"),Some(Value::Obj(p)) if props(p))
        }
        _ => false,
    }
}
// Account for input credit earned inside a callback as well as room draws.
fn room_used() -> usize {
    ACCOUNT.with(|a| {
        let a = a.borrow();
        a.limits
            .result_size_for(a.bytes)
            .saturating_sub(LIMITS.with(|l| l.room.get()))
    })
}
// Substitution constructs literal descriptors, not additional output positions.
// Keep their room while evaluating the item, then let the owning projection
// replace exactly those reservations. No caller supplies a size or credit.
pub(super) struct SubstitutionRoom(usize);
impl SubstitutionRoom {
    pub(super) fn enter() -> Self {
        Self(room_used())
    }
    pub(super) fn finish(self) {
        if LIMITS.with(|l| l.cut.get().is_none()) {
            let bytes = room_used().saturating_sub(self.0);
            if bytes == 0 {
                return;
            }
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                let frame = a.projection_room.len().saturating_sub(1);
                let depth = a.loop_item_depth;
                if depth != 0 {
                    a.receipts.push((depth, frame, bytes));
                }
                if let Some(parent) = a.projection_room.last_mut() {
                    *parent = parent.saturating_add(bytes);
                }
            });
        }
    }
}
pub(super) struct LoopItem;
impl LoopItem {
    pub(super) fn enter() -> Self {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.loop_item_depth = a.loop_item_depth.saturating_add(1);
        });
        Self
    }
}
impl Drop for LoopItem {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            let depth = a.loop_item_depth;
            if a.metadata.is_some() {
                a.receipts.retain(|(d, _, _)| *d < depth);
            }
            a.loop_item_depth = a.loop_item_depth.saturating_sub(1);
        });
    }
}
fn release_substitution() {
    let released = ACCOUNT.with(|a| {
        let mut a = a.borrow_mut();
        let depth = a.loop_item_depth;
        let mut receipts = Vec::new();
        if a.metadata.is_some() {
            while a.receipts.last().is_some_and(|(d, _, _)| *d == depth) {
                if let Some((_, frame, bytes)) = a.receipts.pop() {
                    receipts.push((frame, bytes));
                }
            }
        }
        let mut released = 0usize;
        for (frame, bytes) in receipts {
            if let Some(room) = a.projection_room.get_mut(frame) {
                let paid = bytes.min(*room);
                *room -= paid;
                released = released.saturating_add(paid);
            }
        }
        released
    });
    LIMITS.with(|l| l.room.set(l.room.get().saturating_add(released)));
}

struct ProjectionEntry;
struct SequentialRoom {
    used: usize,
    extra_object: bool,
}
impl SequentialRoom {
    fn enter(extra_object: bool) -> Self {
        Self {
            used: room_used(),
            extra_object: ACCOUNT
                .with(|a| std::mem::replace(&mut a.borrow_mut().extra_object, extra_object)),
        }
    }
}
impl Drop for SequentialRoom {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            let mut account = a.borrow_mut();
            account.extra_object = self.extra_object;
            // Argument projections belong to this operation's borrowed scope.
            // In particular, do not retain reference supplies in a suspended run.
            if let Some(cache) = account.metadata.as_deref_mut() {
                cache.field_cache = None;
                cache.supplied_strings.clear();
                cache.argument_bodies.clear();
                cache.validated.clear();
            }
            if let Some(text) = account.supplied.text.as_deref_mut() {
                text.shared.clear();
            }
            LIMITS.with(|l| {
                if l.cut.get().is_none() {
                    l.room.set(
                        account
                            .limits
                            .result_size_for(account.bytes)
                            .saturating_sub(self.used),
                    );
                }
            });
        });
    }
}
struct TemporaryResults {
    release: bool,
}
struct DiscardedResult {
    used: usize,
    flags: EvaluationFlags,
    size: bool,
    depth: bool,
    cut: Option<Cut>,
    depth_cut: Rc<Cell<bool>>,
    retained: usize,
}
impl DiscardedResult {
    fn enter() -> Self {
        Self {
            retained: ACCOUNT.with(|a| a.borrow().retained_room),
            used: room_used(),
            flags: ACCOUNT.with(|a| a.borrow().flags.get()),
            size: LIMITS.with(|l| l.size_limited.get()),
            depth: LIMITS.with(|l| l.depth_limited.get()),
            cut: LIMITS.with(|l| l.cut.get()),
            depth_cut: DEPTH
                .with(|d| std::mem::replace(&mut d.borrow_mut().cut, Rc::new(Cell::new(false)))),
        }
    }
}
impl Drop for DiscardedResult {
    fn drop(&mut self) {
        DEPTH.with(|d| d.borrow_mut().cut = self.depth_cut.clone());
        // Exhausted work affects the rest of the evaluation. Room consumed
        // solely by the discarded intermediate does not.
        if LIMITS.with(|l| l.cut.get() == Some(Cut::Work)) || crate::progress::stalled_so_far() {
            return;
        }
        let ceiling = ACCOUNT.with(|a| {
            let a = a.borrow();
            a.flags.set(self.flags);
            a.limits
                .result_size_for(a.bytes)
                .saturating_sub(a.retained_room.saturating_sub(self.retained))
        });
        LIMITS.with(|l| {
            l.room.set(ceiling.saturating_sub(self.used));
            l.size_limited.set(self.size);
            l.depth_limited.set(self.depth);
            l.cut.set(self.cut);
        });
    }
}
struct ItemRoom;
impl ItemRoom {
    fn enter() -> Self {
        ACCOUNT.with(|a| a.borrow_mut().temporary_room.push((0, false)));
        Self
    }
}
impl Drop for ItemRoom {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            a.borrow_mut().temporary_room.pop();
        });
    }
}
impl Drop for TemporaryResults {
    fn drop(&mut self) {
        let released = ACCOUNT.with(|a| a.borrow_mut().temporary_room.pop().map_or(0, |v| v.0));
        if !self.release {
            ACCOUNT.with(|a| {
                if let Some(parent) = a.borrow_mut().temporary_room.last_mut() {
                    parent.0 = parent.0.saturating_add(released);
                }
            });
            return;
        }
        LIMITS.with(|l| {
            if l.cut.get().is_none() {
                l.room.set(l.room.get().saturating_add(released));
            }
        });
    }
}
impl ProjectionEntry {
    fn enter() -> Self {
        ACCOUNT.with(|a| a.borrow_mut().projection_room.push(0));
        Self
    }
}
impl Drop for ProjectionEntry {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            a.borrow_mut().projection_room.pop();
        });
    }
}
fn runtime_value(js: &Js) -> Option<Value> {
    let released = ACCOUNT.with(|a| {
        a.borrow_mut()
            .projection_room
            .last_mut()
            .map(std::mem::take)
            .unwrap_or(0)
    });
    LIMITS.with(|l| {
        if l.cut.get().is_none() {
            l.room.set(l.room.get().saturating_add(released));
        }
        let before = l.room.get();
        let value = to_value_in::<true>(js, l);
        if l.cut.get().is_none() {
            if let Some(value) = &value {
                remember_projected(value, before.saturating_sub(l.room.get()), true);
            }
            ACCOUNT.with(|a| {
                if let Some(room) = a.borrow_mut().temporary_room.last_mut() {
                    room.0 = room.0.saturating_add(before.saturating_sub(l.room.get()));
                }
            });
        }
        value
    })
}

fn record_flags() {
    if crate::progress::stalled_so_far() {
        super::stack_stalled();
    }
    let now = LIMITS.with(|l| EvaluationFlags {
        size_limited: l.size_limited.get() || crate::progress::stalled_so_far(),
        depth_limited: l.depth_limited.get(),
    });
    if now.depth_limited {
        DEPTH.with(|d| d.borrow().cut.set(true));
    }
    ACCOUNT.with(|a| {
        let a = a.borrow();
        let old = a.flags.get();
        a.flags.set(EvaluationFlags {
            size_limited: old.size_limited || now.size_limited,
            depth_limited: old.depth_limited || now.depth_limited,
        });
    });
}

// Array storage has a separate live-byte ceiling equal to the session's
// result ceiling plus a fixed 64-byte ledger header. It is not subtracted
// from serialized room: that would price
// final steps twice. Weak entries track aliases without retaining outputs.
const BUFFER_OVERHEAD: usize = std::mem::size_of::<Vec<Js>>()
    // Arc counters and one word per allocation for allocation bookkeeping.
    // The latter matters for thousands of small nested vectors too.
    + 4 * std::mem::size_of::<usize>()
    + 2 * std::mem::size_of::<(BufferOwner, usize)>();
enum BufferOwner {
    Js(Weak<Vec<Js>>),
    Value(Weak<Vec<Value>>),
}
impl BufferOwner {
    fn alive(&self) -> bool {
        match self {
            Self::Js(v) => v.strong_count() > 0,
            Self::Value(v) => v.strong_count() > 0,
        }
    }
}
#[derive(Default)]
struct Buffers {
    owners: Vec<(BufferOwner, usize)>,
    live: usize,
    reserved: usize,
    collect_at: usize,
}
impl Buffers {
    fn collect(&mut self) {
        self.owners.retain(|(owner, bytes)| {
            if owner.alive() {
                true
            } else {
                self.live = self.live.saturating_sub(*bytes);
                false
            }
        });
        if self.owners.capacity() > self.owners.len().saturating_mul(2) {
            self.owners.shrink_to_fit();
        }
        self.collect_at = self.owners.len().saturating_mul(2).max(32);
    }
}
fn buffer_limit() -> usize {
    ACCOUNT.with(|a| {
        let a = a.borrow();
        a.limits.result_size_for(a.bytes).saturating_add(64)
    })
}
struct BufferReservation {
    bytes: usize,
}
impl BufferReservation {
    fn new(bytes: usize) -> Option<Self> {
        if !step(1) {
            return None;
        }
        let mut r = Self { bytes: 0 };
        r.grow(bytes).then_some(r)
    }
    fn grow(&mut self, bytes: usize) -> bool {
        if bytes <= self.bytes {
            return LIMITS.with(|l| l.cut.get().is_none());
        }
        let extra = bytes.saturating_sub(self.bytes);
        let limit = buffer_limit();
        let fits = ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            let b = &mut a.buffers;
            if b.reserved.saturating_add(b.live).saturating_add(extra) > limit {
                if !step(b.owners.len().saturating_add(1)) {
                    return false;
                }
                b.collect();
            }
            if b.reserved.saturating_add(b.live).saturating_add(extra) > limit {
                return false;
            }
            b.reserved = b.reserved.saturating_add(extra);
            true
        });
        if fits {
            self.bytes = bytes;
        } else {
            LIMITS.with(|l| {
                if l.cut.get().is_none() {
                    l.cut_by(Cut::Room)
                }
            });
        }
        fits
    }
    fn finish(mut self, owner: BufferOwner) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            let b = &mut a.buffers;
            b.reserved = b.reserved.saturating_sub(self.bytes);
            if b.owners.len() >= b.collect_at {
                step(b.owners.len().saturating_add(1));
                b.collect();
            }
            b.live = b.live.saturating_add(self.bytes);
            b.owners.push((owner, self.bytes));
        });
        self.bytes = 0;
    }
}
impl Drop for BufferReservation {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.buffers.reserved = a.buffers.reserved.saturating_sub(self.bytes);
        });
    }
}
// Runtime projection of transformed arrays uses exact capacity, and includes
// both the retained internal vector and the public vector in the live ceiling.
// Parsing and ordinary runtime projections keep their existing collection path.
struct BufferProjection(bool);
impl BufferProjection {
    fn enter(enabled: bool) -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().buffer_projection, enabled)))
    }
}
impl Drop for BufferProjection {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().buffer_projection = self.0);
    }
}
pub(super) fn buffered_projection() -> bool {
    ACCOUNT.with(|a| a.borrow().buffer_projection)
}
pub(super) fn project_buffered_array(items: &[Js], limits: &EvalLimits) -> Option<Value> {
    let bytes = items
        .len()
        .saturating_mul(std::mem::size_of::<Value>())
        .saturating_add(BUFFER_OVERHEAD);
    let _reservation = BufferReservation::new(bytes)?;
    let mut out = Vec::new();
    if out.try_reserve_exact(items.len()).is_err() {
        limits.cut_by(Cut::Room);
        return None;
    }
    for item in items {
        if limits.cut.get().is_some() {
            return None;
        }
        out.push(to_value_mode::<true, true>(item, limits).unwrap_or(Value::Null));
    }
    if limits.cut.get().is_some() {
        return None;
    }
    let out = Arc::new(out);
    _reservation.finish(BufferOwner::Value(Arc::downgrade(&out)));
    Some(Value::Arr(out))
}
// A cheap lower bound for JSON construction; final projection prices bytes.
fn builder_room() -> usize {
    let prepaid = ACCOUNT.with(|a| a.borrow().projection_room.last().copied().unwrap_or(0));
    LIMITS.with(|l| l.room.get().saturating_add(prepaid))
}
fn builder_fits(minimum: usize) -> bool {
    let available = builder_room();
    LIMITS.with(|l| {
        if l.cut.get().is_some() {
            return false;
        }
        if minimum > available {
            l.cut_by(Cut::Room);
            false
        } else {
            true
        }
    })
}

type ValidationIdentity = (usize, (u8, usize), String, bool);
// Keep empty callback suspension frames small. Metadata belongs to the one
// active allowance; suspending it moves one pointer, not dozens of empty maps.
// Raw string addresses are valid only while these source containers live.
// Keep the owners, the address index, and their shared conversions together;
// no operation can drop an owner while leaving an eligible address behind.
#[derive(Default)]
struct SuppliedOwners {
    owners: Vec<Value>,
    text: Option<Box<SuppliedText>>,
}
#[derive(Default)]
struct SuppliedText {
    texts: HashSet<usize>,
    shared: HashMap<usize, Arc<str>>,
}
static EMPTY_TEXT: std::sync::LazyLock<SuppliedText> =
    std::sync::LazyLock::new(SuppliedText::default);
impl std::ops::Deref for SuppliedOwners {
    type Target = SuppliedText;
    fn deref(&self) -> &SuppliedText {
        self.text.as_deref().unwrap_or(&EMPTY_TEXT)
    }
}
impl std::ops::DerefMut for SuppliedOwners {
    fn deref_mut(&mut self) -> &mut SuppliedText {
        self.text
            .get_or_insert_with(|| Box::new(SuppliedText::default()))
    }
}
struct Metadata {
    receipts: Vec<(usize, usize, usize)>,
    field_cache: Option<FieldCache>,
    supplied_strings: HashSet<usize>,
    validated: HashMap<ValidationIdentity, (WeakValue, usize)>,
    argument_collect_at: usize,
    argument_bodies: HashMap<(u8, usize), BodyMeasurement>,
    bridges: HashMap<(u8, usize), Bridge>,
    collect_at: usize,
    schemas: std::collections::BTreeMap<usize, Arc<u8>>,
    buffers: Buffers,
}
impl Metadata {
    fn new() -> Self {
        Self {
            receipts: Vec::new(),
            field_cache: None,
            supplied_strings: HashSet::new(),
            validated: HashMap::new(),
            argument_collect_at: 8,
            argument_bodies: HashMap::new(),
            bridges: HashMap::new(),
            collect_at: 128,
            schemas: std::collections::BTreeMap::new(),
            buffers: Buffers::default(),
        }
    }
}
static EMPTY_METADATA: std::sync::LazyLock<Metadata> = std::sync::LazyLock::new(Metadata::new);
struct Account {
    projection_room: Vec<usize>,
    temporary_room: Vec<(usize, bool)>,
    inputs: InputIndex,
    supplied: SuppliedOwners,
    loop_item_depth: usize,
    retained_room: usize,
    limits: Limits,
    supplied_floor: usize,
    failure: Option<EvaluationFailure>,
    evaluations: usize,
    call_context: bool,
    extra_enabled: bool,
    extra_object: bool,
    flags: Rc<Cell<EvaluationFlags>>,
    bytes: usize,
    measure_peak: usize,
    measure_exact: bool,
    depth_only: bool,
    argument_texts: bool,
    receiver_read: bool,
    input_owner: bool,
    transformed: usize,
    buffer_projection: bool,
    metadata: Option<Box<Metadata>>,
}
impl std::ops::Deref for Account {
    type Target = Metadata;
    fn deref(&self) -> &Metadata {
        self.metadata.as_deref().unwrap_or(&EMPTY_METADATA)
    }
}
impl std::ops::DerefMut for Account {
    fn deref_mut(&mut self) -> &mut Metadata {
        self.metadata
            .get_or_insert_with(|| Box::new(Metadata::new()))
    }
}
impl Account {
    fn new(limits: Limits, bytes: usize) -> Self {
        Self {
            projection_room: Vec::new(),
            temporary_room: Vec::new(),
            inputs: InputIndex::default(),
            supplied: SuppliedOwners::default(),
            loop_item_depth: 0,
            retained_room: 0,
            limits,
            supplied_floor: bytes,
            failure: None,
            evaluations: 0,
            call_context: true,
            extra_enabled: true,
            extra_object: false,
            flags: Rc::new(Cell::new(EvaluationFlags::default())),
            bytes,
            measure_peak: 0,
            measure_exact: true,
            depth_only: false,
            argument_texts: false,
            receiver_read: false,
            input_owner: false,
            transformed: 0,
            buffer_projection: false,
            metadata: None,
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
struct ExtraContext(bool);
impl ExtraContext {
    fn disable() -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().extra_enabled, false)))
    }
}
impl Drop for ExtraContext {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().extra_enabled = self.0);
    }
}
pub(super) struct CallContext(bool);
impl CallContext {
    pub(super) fn set(enabled: bool) -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().call_context, enabled)))
    }
}
impl Drop for CallContext {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().call_context = self.0);
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
                a.supplied.owners.push(value.clone());
                false
            }
        });
        if skip {
            check_input_depth(value);
            return;
        }
    }
    let old = ACCOUNT.with(|a| {
        std::mem::replace(
            &mut a.borrow_mut().input_owner,
            data_identity(value).is_some(),
        )
    });
    let _owner = InputOwner(old);
    measure_data(value, true);
}
struct InputOwner(bool);
impl Drop for InputOwner {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().input_owner = self.0);
    }
}
pub(super) fn check_input_depth(value: &Value) {
    let required = data_identity(value).and_then(|key| {
        ACCOUNT.with(|a| {
            let a = a.borrow();
            if let Some((_, height)) = a.inputs.measurement(&key) {
                return Some(height.saturating_add(1));
            }
            a.argument_bodies
                .get(&key)
                .filter(|v| v.owner.alive() && v.exact)
                .map(|v| (v.depth as usize).saturating_add(1))
        })
    });
    if let Some(required) = required {
        if LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()) < required) {
            LIMITS.with(|l| l.cut_by(Cut::Depth));
        }
    } else {
        let _mode = DepthOnly::enter();
        measure_data(value, false);
    }
}
// Only values crossing the public callback boundary need an identity bridge.
// Weak entries neither keep discarded results alive nor index every input node.
enum WeakValue {
    Ast(Weak<Ast>),
    Element(Weak<Element>),
    Arr(Weak<Vec<Value>>),
    Obj(Weak<JsObject<Value>>),
}
impl WeakValue {
    fn alive(&self) -> bool {
        match self {
            Self::Ast(v) => v.strong_count() != 0,
            Self::Element(v) => v.strong_count() != 0,
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
    if ACCOUNT.with(|a| a.borrow().depth_only) {
        return 0;
    }
    data_bytes(text.len(), input);
    if !step(text.len().saturating_add(1)) {
        return 0;
    }
    let bytes = js::json_string_len(text).saturating_add(2);
    data_bytes(bytes.saturating_sub(text.len()), input);
    bytes
}

// Schema admission, ordering, required-name coercion and indexes are setup.
// Per check we price value bytes, required-name bytes and applied keywords.
// Ordered indexes have at most usize::BITS+1 levels and at most 11 key
// comparisons per B-tree node. Candidate order uses at most 32 insertion
// comparisons per index, or eight radix passes (<160 visits per index even
// including the fixed buckets at the smallest radix input). Allowing four
// indexed lookups and byte copies gives a conservative 4096 byte/key/bucket
// visits per charged unit on 64-bit targets (less on 32-bit). This bound is
// independent of schema width and description length, unlike the old product.
pub(crate) fn validation_work(cost: usize) -> bool {
    step(cost)
}
// Price js_string's recursive array joins before copying prepared required
// names. Each byte copied at every enclosing join is charged; ignored object
// bodies are not copied by js_string. This is setup, never per-call work.
pub(crate) fn validation_name(value: &serde_json::Value) -> Option<String> {
    validation_name_bytes(value);
    if LIMITS.with(|l| l.cut.get().is_some()) {
        None
    } else {
        Some(crate::js::js_string(value))
    }
}
fn validation_name_bytes(value: &serde_json::Value) -> usize {
    crate::stack::grow_stack!(validation_name_bytes(value), budget else 0);
    let Some(_level) = Level::enter() else {
        return 0;
    };
    use serde_json::Value as J;
    let bytes = match value {
        J::String(s) => s.len(),
        J::Array(a) => {
            let mut n = a.len().saturating_sub(1);
            for v in a {
                if !step(1) {
                    return 0;
                }
                if !v.is_null() {
                    n = n.saturating_add(validation_name_bytes(v));
                }
            }
            n
        }
        J::Object(_) => 15,
        J::Number(_) => 32,
        J::Bool(_) | J::Null => 5,
    };
    if step(bytes.saturating_add(1)) {
        bytes
    } else {
        0
    }
}
pub(crate) fn validation_default(schema: &serde_json::Value) -> bool {
    let bytes = measure_schema(schema);
    step(bytes) && LIMITS.with(|l| l.cut.get().is_none())
}
pub(crate) fn validation_node(value: &Value, schema: &serde_json::Value, path: &str) -> bool {
    let scalar_bytes = |v: &Value| match v {
        Value::Str(s) => s.len(),
        _ => 1,
    };
    let mut cost = path.len().saturating_add(1);
    match value {
        Value::Obj(o) => {
            for (k, v) in o.iter() {
                cost = cost.saturating_add(k.len()).saturating_add(scalar_bytes(v));
            }
        }
        Value::Arr(a) => {
            for v in a.iter() {
                cost = cost.saturating_add(scalar_bytes(v));
            }
        }
        _ => cost = cost.saturating_add(scalar_bytes(value)),
    }
    if !step(cost) {
        return false;
    }
    if let Some(s) = schema.as_object() {
        if matches!(value, Value::Null) || crate::validation::is_ast_node(value) {
            return LIMITS.with(|l| l.cut.get().is_none());
        }
        if crate::validation::element_parts(value).is_some() {
            return true;
        }
        // Upstream treats composites as opaque without a component catalog.
        // It inspects the immediate option refs, never expands nested branches.
        if ["$ref", "anyOf", "oneOf", "allOf"]
            .iter()
            .any(|k| s.contains_key(*k))
        {
            let options = s
                .get("anyOf")
                .filter(|v| !v.is_null())
                .or_else(|| s.get("oneOf").filter(|v| !v.is_null()));
            if let Some(serde_json::Value::Array(options)) = options {
                if !step(options.len()) {
                    return false;
                }
                for option in options {
                    if let Some(r) = option.get("$ref").and_then(|r| r.as_str())
                        && !step(r.len().saturating_add(1))
                    {
                        return false;
                    }
                }
            } else if let Some(r) = s.get("$ref").and_then(|r| r.as_str()) {
                step(r.len().saturating_add(1));
            }
            return LIMITS.with(|l| l.cut.get().is_none());
        }
        if matches!(
            s.get("type").and_then(serde_json::Value::as_str),
            Some("object" | "array")
        ) {
            return true;
        }
        if let Some(values) = s.get("enum").and_then(|e| e.as_array())
            && !step(
                values
                    .len()
                    .saturating_mul(scalar_bytes(value).saturating_add(1)),
            )
        {
            return false;
        }
        if s.contains_key("const") {
            step(scalar_bytes(value).saturating_add(1));
        }
    }
    LIMITS.with(|l| l.cut.get().is_none())
}
// Measure borrowed schema JSON before converting or cloning any subtree. A
// library can contain a very deep ignored schema branch; admission must cut
// that branch before validation or a temporary Value walks the whole thing.
fn measure_schema(schema: &serde_json::Value) -> usize {
    crate::stack::grow_stack!(measure_schema(schema), budget else 0);
    let Some(_level) = Level::enter() else {
        return 0;
    };
    use serde_json::Value as Json;
    if let Json::String(text) = schema {
        return data_text(text, false);
    }
    let own = match schema {
        Json::Null => 4,
        Json::Bool(true) => 4,
        Json::Bool(false) => 5,
        Json::Number(n) => crate::js::json_number_len(crate::js::json_number(n)),
        Json::Array(a) => 2usize.saturating_add(a.len().saturating_sub(1)),
        Json::Object(o) => 2usize.saturating_add(o.len().saturating_sub(1)),
        Json::String(_) => 0,
    };
    data_bytes(own, false);
    if !step(1) {
        return 0;
    }
    let mut bytes = own;
    match schema {
        Json::Array(a) => {
            for value in a {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                bytes = bytes.saturating_add(measure_schema(value));
            }
        }
        Json::Object(o) => {
            for (key, value) in o {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                bytes = bytes.saturating_add(data_text(key, false).saturating_add(1));
                data_bytes(1, false);
                bytes = bytes.saturating_add(measure_schema(value));
            }
        }
        _ => {}
    }
    bytes
}

// A member/index receiver is evaluated one expression frame below the use.
// Consume the marker at the name lookup so nested callbacks do not inherit it.
pub(super) struct ReceiverRead(bool);
impl ReceiverRead {
    pub(super) fn enter() -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().receiver_read, true)))
    }
}
impl Drop for ReceiverRead {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().receiver_read = self.0);
    }
}
struct ReadPosition(Option<usize>);
impl ReadPosition {
    fn enter(receiver: bool) -> Self {
        Self(receiver.then(|| {
            LIMITS.with(|l| {
                let old = l.depth.get();
                l.depth.set(old.saturating_sub(1));
                old
            })
        }))
    }
}
impl Drop for ReadPosition {
    fn drop(&mut self) {
        if let Some(depth) = self.0 {
            LIMITS.with(|l| l.depth.set(depth));
        }
    }
}
struct ArgumentTexts(bool);
impl ArgumentTexts {
    fn enter() -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().argument_texts, true)))
    }
}
impl Drop for ArgumentTexts {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().argument_texts = self.0);
    }
}
// A position check traverses shape only. String contents and JSON sizes are
// already admitted; checking their depth must never price them a second time.
struct DepthOnly(bool);
impl DepthOnly {
    fn enter() -> Self {
        Self(ACCOUNT.with(|a| std::mem::replace(&mut a.borrow_mut().depth_only, true)))
    }
}
impl Drop for DepthOnly {
    fn drop(&mut self) {
        ACCOUNT.with(|a| a.borrow_mut().depth_only = self.0);
    }
}
fn argument_measurement() -> bool {
    ACCOUNT.with(|a| a.borrow().temporary_room.last().is_some_and(|v| v.1))
}
fn measure_data(value: &Value, input: bool) -> usize {
    measure_data_with_memo::<true>(value, input)
}
fn measure_boundary(value: &Value) -> usize {
    // This root is about to be shared with the internal representation. Keep
    // its identity eligible without indexing every uniquely owned descendant.
    let _owner = if data_identity(value).is_some() {
        Some(value.clone())
    } else {
        None
    };
    measure_data(value, false)
}
struct BodyMeasurement {
    owner: WeakValue,
    // A successful root projection permits one room-only reservation.
    projected: bool,
    bytes: usize,
    depth: u32,
    exact: bool,
}
struct MeasuredDepth {
    start: usize,
    previous: usize,
    previous_exact: bool,
}
impl MeasuredDepth {
    fn enter() -> Self {
        let start = LIMITS.with(|l| l.depth.get());
        let (previous, previous_exact) = ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            (
                std::mem::replace(&mut a.measure_peak, start),
                std::mem::replace(&mut a.measure_exact, true),
            )
        });
        Self {
            start,
            previous,
            previous_exact,
        }
    }
    fn include(&self, height: usize, exact: bool) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.measure_peak = a.measure_peak.max(self.start.saturating_add(height));
            a.measure_exact &= exact;
        });
    }
    fn height(&self) -> usize {
        ACCOUNT.with(|a| a.borrow().measure_peak.saturating_sub(self.start))
    }
    fn exact(&self) -> bool {
        ACCOUNT.with(|a| a.borrow().measure_exact)
    }
}
impl Drop for MeasuredDepth {
    fn drop(&mut self) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            a.measure_peak = a.measure_peak.max(self.previous);
            a.measure_exact &= self.previous_exact;
        });
    }
}
fn measure_data_with_memo<const MEMO: bool>(value: &Value, input: bool) -> usize {
    crate::stack::grow_stack!(measure_data_with_memo::<MEMO>(value, input), budget else 0);
    let Some(_level) = Level::enter() else {
        return 0;
    };
    let measured_depth = MeasuredDepth::enter();
    if input
        && let Some(key) = data_identity(value)
        && !ACCOUNT.with(|a| a.borrow_mut().inputs.insert(key))
    {
        let checked = ACCOUNT.with(|a| a.borrow().inputs.measurement(&key));
        if let Some((bytes, height)) = checked {
            measured_depth.include(height, true);
            if LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()) < height) {
                LIMITS.with(|l| l.cut_by(Cut::Depth));
            }
            return bytes.unwrap_or(0);
        }
        measured_depth.include(LIMITS.with(|l| l.max_depth.get()), false);
        return 0;
    }
    let memo = MEMO && !input;
    // A foreign re-entry can lower max_depth without changing local depth.
    // Reuse only with at least the remaining depth of a successful walk.
    let depth = LIMITS.with(|l| l.max_depth.get().saturating_sub(l.depth.get()));
    if memo && let Some(key) = data_identity(value) {
        let prepaid = ACCOUNT.with(|a| a.borrow().inputs.measurement(&key));
        if let Some((Some(bytes), height)) = prepaid {
            if depth >= height {
                measured_depth.include(height, true);
                return bytes;
            }
            LIMITS.with(|l| l.cut_by(Cut::Depth));
            return 0;
        }
        let hit = ACCOUNT.with(|a| {
            a.borrow()
                .argument_bodies
                .get(&key)
                .filter(|v| v.owner.alive() && depth >= v.depth as usize)
                .and_then(|v| (v.bytes != 0).then_some((v.bytes, v.depth as usize, v.exact)))
        });
        if let Some((bytes, height, exact)) = hit {
            measured_depth.include(height, exact);
            return bytes;
        }
    }
    if let Value::Str(text) = value {
        if input {
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                if a.input_owner && a.argument_texts {
                    a.supplied.texts.insert(text.as_ptr() as usize);
                }
            });
        }
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
        Value::Num(n) => crate::js::json_number_len(*n),
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
            if MEMO {
                remember_measurement(value, bytes, &measured_depth);
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
            .saturating_add(measure_data_with_memo::<MEMO>(value, input));
    };
    match value {
        Value::Arr(items) => {
            for item in items.iter() {
                if LIMITS.with(|l| l.cut.get().is_some()) {
                    break;
                }
                bytes = bytes.saturating_add(measure_data_with_memo::<MEMO>(item, input));
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
    if MEMO {
        remember_measurement(value, bytes, &measured_depth);
    }
    bytes
}

fn remember_measurement(value: &Value, bytes: usize, measured_depth: &MeasuredDepth) {
    if LIMITS.with(|l| l.cut.get().is_none()) {
        let key = data_identity(value);
        if let Some(key) = key
            && ACCOUNT.with(|a| a.borrow().inputs.contains(&key))
        {
            let height = measured_depth.height();
            let exact = measured_depth.exact();
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                let bytes = (!a.depth_only).then_some(bytes);
                a.inputs.record(key, bytes, height, exact);
            });
            return;
        }
        let weak = match value {
            Value::Ast(v) if argument_measurement() || Arc::strong_count(v) > 1 => {
                Some(WeakValue::Ast(Arc::downgrade(v)))
            }
            Value::Element(v) => Some(WeakValue::Element(Arc::downgrade(v))),
            Value::Arr(v) if argument_measurement() || Arc::strong_count(v) > 1 => {
                Some(WeakValue::Arr(Arc::downgrade(v)))
            }
            Value::Obj(v)
                if argument_measurement() || Arc::strong_count(v) > 1 || projected_element(v) =>
            {
                Some(WeakValue::Obj(Arc::downgrade(v)))
            }
            _ => None,
        };
        if let (Some(key), Some(weak)) = (data_identity(value), weak) {
            let height = measured_depth.height();
            let exact = measured_depth.exact();
            ACCOUNT.with(|a| {
                let mut a = a.borrow_mut();
                if a.argument_bodies.len() >= a.argument_collect_at {
                    a.argument_bodies.retain(|_, v| v.owner.alive());
                    a.argument_collect_at = a.argument_bodies.len().saturating_mul(2).max(8);
                }
                let bytes = if a.depth_only {
                    a.argument_bodies.get(&key).map_or(0, |v| v.bytes)
                } else {
                    bytes
                };
                let Ok(height) = u32::try_from(height) else {
                    return;
                };
                a.argument_bodies.insert(
                    key,
                    BodyMeasurement {
                        owner: weak,
                        projected: false,
                        depth: height,
                        exact,
                        bytes,
                    },
                );
            });
        }
    }
}

fn projected_element(o: &JsObject<Value>) -> bool {
    matches!(o.get("type"),Some(Value::Str(s)) if s=="element")
}
// Cache only array roots and element-shaped objects, whose footprints can be
// needed when a caller rebuilds part of an already charged projection. A weak
// owner rejects mutation/reallocation; unknown height is never a depth proof.
pub(super) fn remember_projected(value: &Value, bytes: usize, root: bool) {
    if argument_measurement() {
        return;
    }
    let owner = match value {
        Value::Arr(v) if root => WeakValue::Arr(Arc::downgrade(v)),
        Value::Obj(v) if projected_element(v) => WeakValue::Obj(Arc::downgrade(v)),
        _ => return,
    };
    if let Some(key) = data_identity(value) {
        ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            if a.argument_bodies.len() >= a.argument_collect_at {
                a.argument_bodies.retain(|_, m| m.owner.alive());
                a.argument_collect_at = a.argument_bodies.len().saturating_mul(2).max(8);
            }
            let entry = a.argument_bodies.entry(key).or_insert(BodyMeasurement {
                owner,
                bytes,
                depth: u32::MAX,
                exact: false,
                projected: false,
            });
            entry.bytes = bytes;
            entry.projected = root;
        });
    }
}

pub(super) fn data_child(owner: &Value, child: &Value) -> Js {
    if let Value::Str(text) = child
        && ACCOUNT.with(|a| {
            let a = a.borrow();
            a.argument_texts && data_identity(owner).is_some_and(|k| a.inputs.contains(&k))
        })
    {
        ACCOUNT.with(|a| {
            a.borrow_mut().supplied.texts.insert(text.as_ptr() as usize);
        });
    }
    data_value(child)
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
            let key = v.as_ptr() as usize;
            let supplied = ACCOUNT.with(|a| a.borrow().supplied.texts.contains(&key));
            if supplied
                && let Some(text) = ACCOUNT.with(|a| a.borrow().supplied.shared.get(&key).cloned())
            {
                return Js::Str(text);
            }
            if !step(v.len()) {
                return Js::Null;
            }
            let text: Arc<str> = v.as_str().into();
            if supplied {
                ACCOUNT.with(|a| {
                    let mut a = a.borrow_mut();
                    a.supplied_strings.insert(text.as_ptr() as usize);
                    a.supplied.shared.insert(key, text.clone());
                });
            }
            Js::Str(text)
        }
        Value::Ast(node) => Js::Ast(node.clone()),
        _ => Js::Data(Arc::new(value.clone())),
    }
}
pub(super) fn output_data(value: &Value) -> Value {
    let shared = ACCOUNT.with(|a| {
        let a = a.borrow();
        a.temporary_room.last().is_some_and(|v| v.1)
            && data_identity(value).is_some_and(|k| {
                a.inputs.contains(&k)
                    || a.argument_bodies
                        .get(&k)
                        .is_some_and(|v| v.owner.alive() && v.bytes != 0)
            })
    });
    if shared {
        measure_data(value, false);
        return value.clone();
    }
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
        let key = to_string_in::<Supplied<'_>>(index);
        if failed() {
            return Js::Null;
        }
        data_property(value, &key)
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
    let _measured_depth = MeasuredDepth::enter();
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
    if ACCOUNT.with(|a| a.borrow().depth_only) {
        return 0;
    }
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
// A nested session can temporarily hold another session's account. Retain
// those saved locations while resumed, so captured evaluators find their own
// account even when their usual saved slot currently holds an outer context.
thread_local! {
    static SUSPENDED: RefCell<Vec<std::rc::Weak<RefCell<Saved>>>> = const { RefCell::new(Vec::new()) };
}
struct Resume(Rc<RefCell<Saved>>);
impl Resume {
    fn for_account(saved: &Rc<RefCell<Saved>>, flags: &Rc<Cell<EvaluationFlags>>) -> Self {
        let location = if Rc::ptr_eq(&saved.borrow().account.flags, flags) {
            saved.clone()
        } else {
            SUSPENDED
                .with(|slots| {
                    slots
                        .borrow()
                        .iter()
                        .rev()
                        .filter_map(std::rc::Weak::upgrade)
                        .find(|slot| Rc::ptr_eq(&slot.borrow().account.flags, flags))
                })
                .unwrap_or_else(|| saved.clone())
        };
        Self::enter(&location)
    }
    fn enter(saved: &Rc<RefCell<Saved>>) -> Self {
        saved.borrow_mut().swap();
        SUSPENDED.with(|slots| slots.borrow_mut().push(Rc::downgrade(saved)));
        Self(saved.clone())
    }
}
impl Drop for Resume {
    fn drop(&mut self) {
        SUSPENDED.with(|slots| {
            slots.borrow_mut().pop();
        });
        self.0.borrow_mut().swap();
    }
}
