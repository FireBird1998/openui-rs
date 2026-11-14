//! Evaluating parsed prop trees with one expression allowance.
use crate::{
    Ast, EvaluationFailure, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    ExpressionOperation, ExpressionType, ExpressionValue, JsObject, Limits, Store, Value,
};
use openui_lang::{Element, Library, ParseResult};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    sync::Arc,
};
pub(crate) mod actions;
mod bindings;
mod fields;
mod functions;
pub use bindings::{BindingProps, BindingWriteError, write_binding, write_binding_with_limits};
pub use functions::{FunctionResult, LibraryFunctions};
mod input;
mod prepared;
use fields::{own_fields, shaped_element};
pub use prepared::PreparedReferenceValues;

/// Extensible input to tree evaluation. Parsing already inlines value statements.
/// Parsing has already named component arguments. Library definitions separate
/// components from library functions and declared actions.
/// Binding props are supplied independently of JSON Schema.
#[derive(Clone)]
#[non_exhaustive]
pub struct TreeInput<'a> {
    /// Original library action definitions (`schema.actions`). The compiled
    /// parsing library intentionally does not retain their whole schemas.
    /// Without these definitions custom calls evaluate to null.
    pub actions: Option<&'a openui_lang::JsonValue>,
    /// Synchronous function implementations and their original schemas.
    pub functions: Option<&'a LibraryFunctions<'a>>,
    /// Props whose schema objects upstream marks reactive.
    pub binding_props: Option<&'a BindingProps>,
    /// The parse result. Evaluation reads the store without initializing it.
    pub result: &'a ParseResult,
    /// Supplied query/mutation results, keyed by statement id. Explicit null wins.
    /// Borrowed for the evaluation; container bodies are never copied on read.
    pub reference_values: Option<&'a JsObject<Value>>,
    /// Library definitions distinguish components from functions and actions.
    /// Parsing has already named the component props. Without a library, mapped
    /// calls use upstream's component fallback.
    pub library: Option<&'a Library>,
    /// A set measured once by the embedder. Takes precedence over `reference_values`.
    pub prepared_reference_values: Option<&'a PreparedReferenceValues<'a>>,
}
impl<'a> TreeInput<'a> {
    /// Evaluate this parse result; future options have compatible defaults.
    pub fn new(result: &'a ParseResult) -> Self {
        Self {
            result,
            actions: None,
            functions: None,
            binding_props: None,
            reference_values: None,
            library: None,
            prepared_reference_values: None,
        }
    }
    /// Supply synchronous library implementations and original definitions.
    pub fn with_functions(mut self, functions: &'a LibraryFunctions<'a>) -> Self {
        self.functions = Some(functions);
        self
    }
    /// Supply original action definitions, including each `params` schema.
    pub fn with_actions(mut self, actions: &'a openui_lang::JsonValue) -> Self {
        self.actions = Some(actions);
        self
    }
    /// Supply binding prop names, independently of the parsing library.
    pub fn with_binding_props(mut self, props: &'a BindingProps) -> Self {
        self.binding_props = Some(props);
        self
    }
    /// Supply the library used to parse this result.
    pub fn with_library(mut self, library: &'a Library) -> Self {
        self.library = Some(library);
        self
    }
    /// Use reference values prepared once and reused across stream evaluations.
    pub fn with_prepared_reference_values(
        mut self,
        values: &'a PreparedReferenceValues<'a>,
    ) -> Self {
        self.prepared_reference_values = Some(values);
        self.reference_values = None;
        self
    }
    /// Supply reference values, measuring them for this call. Undeclared names
    /// do not resolve, but admitted supplied data contributes input allowance.
    pub fn with_reference_values(mut self, values: &'a JsObject<Value>) -> Self {
        self.reference_values = Some(values);
        self.prepared_reference_values = None;
        self
    }
}
impl<'a> From<&'a ParseResult> for TreeInput<'a> {
    fn from(result: &'a ParseResult) -> Self {
        Self::new(result)
    }
}

/// A runtime diagnostic, in upstream's encounter order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuntimeError {
    /// The upstream runtime error code.
    pub code: &'static str,
    /// The component whose prop failed.
    pub component: Option<String>,
    /// The statement containing that component, when present.
    pub statement_id: Option<String>,
    /// The reference's diagnostic, including the prop name.
    pub message: String,
    /// The reference's suggested check.
    pub hint: Option<String>,
}
impl RuntimeError {
    fn value(&self) -> Value {
        let mut o = JsObject::new();
        o.define("source", Value::Str("runtime".into()));
        o.define("code", Value::Str(self.code.into()));
        if let Some(component) = &self.component {
            o.define("component", Value::Str(component.clone()));
        }
        if let Some(id) = &self.statement_id {
            o.define("statementId", Value::Str(id.clone()));
        }
        o.define("message", Value::Str(self.message.clone()));
        if let Some(hint) = &self.hint {
            o.define("hint", Value::Str(hint.clone()));
        }
        Value::Obj(Arc::new(o))
    }
}

/// The evaluated root, errors, and sticky cuts affecting the whole tree.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TreeResult {
    /// The evaluated root. Value also preserves structurally shaped elements.
    /// None when neither the root nor its expression yields an element. Cuts
    /// retain an existing root and fitted props.
    pub root: Option<Value>,
    /// Failed props retain their original value.
    pub errors: Vec<RuntimeError>,
    /// Work or result room ran out.
    pub size_limited: bool,
    /// Tree nesting or expression/reference depth ran out.
    pub depth_limited: bool,
}
impl TreeResult {
    /// `{root,errors}` in upstream's JSON.stringify order. Flags are not JSON.
    pub fn to_json(&self) -> String {
        let mut o = JsObject::new();
        o.define("root", self.root.as_ref().cloned().unwrap_or(Value::Null));
        o.define(
            "errors",
            Value::Arr(Arc::new(
                self.errors.iter().map(RuntimeError::value).collect(),
            )),
        );
        value_json(&Value::Obj(Arc::new(o)))
    }
}

// Reuse the core's stack-safe, JS-compatible serializer without adding a
// second number/string/AST serializer or a dependency. The wrapper is fixed.
pub(crate) fn value_json(value: &Value) -> String {
    let mut props = JsObject::new();
    props.define("v", value.clone());
    let result = ParseResult {
        root: Some(Element {
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
        .and_then(|s| s.rfind(suffix).map(|n| s[..n].to_owned()))
        .unwrap_or_else(|| "null".into())
}

/// Evaluate with default limits, using one shared core session.
/// Static container bodies remain shared. Dynamic headers and keys are copied;
/// rebuilding a container copies scalar strings but shares unchanged bodies.
/// The owned parse root's header is copied once. State strings are copied by
/// the evaluator on read and its returned value is moved into the tree.
///
/// On a cut, keep evaluated props and collected errors. The prop interrupted
/// by that cut, and every prop still unevaluated, retain their raw input value.
/// A completed nested subtree keeps its fitted props by the same rule. Raw
/// input retained on a cut does not spend generated-result room. Retained props
/// still use JavaScript property-assignment semantics, like failed props.
/// Raw props retained on a cut may display forged custom-step objects. Their
/// position in the tree grants no authority: execution must check the store's
/// private registry, including for retained or cut values. Evaluating an AST
/// call to a declared action earns a real mark, whether the call came from
/// program text or from an evaluated prop in state/query data. Such data can
/// invoke declared actions with its chosen arguments; a finished custom step
/// supplied as data cannot manufacture the mark.
pub fn evaluate_tree(input: TreeInput<'_>, store: &Store) -> TreeResult {
    evaluate_tree_with_limits(input, store, Limits::DEFAULT)
}
/// Evaluate with supplied limits. Work, result room, nesting and state input
/// credit are shared by the entire tree. See evaluate_tree for cut recovery.
pub fn evaluate_tree_with_limits(
    input: TreeInput<'_>,
    store: &Store,
    limits: Limits,
) -> TreeResult {
    let bytes = match input::input_size(&input, limits) {
        Ok(bytes) => bytes,
        Err(depth) => {
            return TreeResult {
                root: input
                    .result
                    .root
                    .as_ref()
                    .map(|e| Value::Element(Arc::new(e.clone()))),
                errors: Vec::new(),
                size_limited: !depth,
                depth_limited: depth,
            };
        }
    };
    let measured = if input.prepared_reference_values.is_none() {
        input.reference_values.map(PreparedReferenceValues::new)
    } else {
        None
    };
    let supplied = input.prepared_reference_values.or(measured.as_ref());
    let bytes = bytes.saturating_add(supplied.map_or(0, |v| v.bytes));
    let scope = TreeScope::new(&input, store, supplied);
    let session = EvaluationSession::with_limits(&scope, bytes, limits);
    let root = session.with_evaluator(|eval| {
        scope.initialize_queries(input.result, eval);
        if let Some(e) = &input.result.root {
            return Some(scope.element(e, eval));
        }
        let expression = input.result.root_expression.as_ref()?;
        let value = eval.evaluate_result(expression);
        if cut(eval) {
            return None;
        }
        if let Some(failure) = value.failure {
            let message = failure_message(&failure);
            if eval.reserve_result(160usize.saturating_add(text_bytes(&message))) {
                scope.errors.borrow_mut().push(RuntimeError {
                    code: "runtime-error",
                    component: None,
                    statement_id: Some("root".into()),
                    message: format!("Evaluating root failed: {message}"),
                    hint: Some("Check the expression used for root".into()),
                });
            }
            return None;
        }
        if let Some(value) = value.value.filter(element_value) {
            return scope.expression_result(Some(value), eval);
        }
        if eval.reserve_result(120) {
            scope.errors.borrow_mut().push(RuntimeError {
                code: "no-root",
                component: None,
                statement_id: Some("root".into()),
                message: "root does not evaluate to a component".into(),
                hint: None,
            });
        }
        None
    });
    let flags = session.flags();
    TreeResult {
        root,
        errors: scope.errors.into_inner(),
        size_limited: flags.size_limited,
        depth_limited: flags.depth_limited,
    }
}
struct TreeScope<'a> {
    registering: Cell<bool>,
    defaults: RefCell<BTreeMap<&'a str, Option<Value>>>,
    store: &'a Store,
    errors: RefCell<Vec<RuntimeError>>,
    inline: Cell<bool>,
    report_bytes: Cell<Option<usize>>,
    // Only newly constructed markers can reuse a projection reservation.
    // Supplied data can alias unevaluated props elsewhere in the input, so
    // each of its output positions must reserve independently. Retaining the
    // Arc also prevents a discarded marker's address being reused here.
    paid_markers: RefCell<std::collections::HashMap<usize, Arc<JsObject<Value>>>>,
    references: BTreeMap<&'a str, Option<(bool, Option<&'a Ast>)>>,
    reference_values: Option<&'a PreparedReferenceValues<'a>>,
    actions: Option<&'a openui_lang::JsonValue>,
    functions: Option<&'a LibraryFunctions<'a>>,
    checked_schemas:
        RefCell<BTreeMap<usize, std::rc::Rc<openui_lang::PreparedValidationSchema<'a>>>>,
    library: Option<&'a Library>,
    binding_props: Option<&'a BindingProps>,
}
impl<'a> TreeScope<'a> {
    fn new(
        input: &TreeInput<'a>,
        store: &'a Store,
        supplied: Option<&'a PreparedReferenceValues<'a>>,
    ) -> Self {
        Self {
            registering: Cell::new(true),
            defaults: RefCell::new(BTreeMap::new()),
            store,
            errors: RefCell::new(Vec::new()),
            inline: Cell::new(false),
            report_bytes: Cell::new(None),
            paid_markers: RefCell::new(std::collections::HashMap::new()),
            references: {
                let mut references = BTreeMap::new();
                for q in &input.result.query_statements {
                    references
                        .entry(q.statement_id.as_str())
                        .or_insert(Some((q.complete, q.defaults.as_ref())));
                }
                for m in &input.result.mutation_statements {
                    references.entry(m.statement_id.as_str()).or_insert(None);
                }
                references
            },
            reference_values: supplied,
            actions: input.actions,
            functions: input.functions,
            checked_schemas: RefCell::new(BTreeMap::new()),
            library: input.library,
            binding_props: input.binding_props,
        }
    }
}
impl<'a> TreeScope<'a> {
    fn initialize_queries(&self, result: &'a ParseResult, eval: &ExpressionEvaluator<'_>) {
        for q in &result.query_statements {
            if cut(eval) {
                break;
            }
            if !q.complete {
                continue;
            }
            let value = q.defaults.as_ref().map_or(Some(Value::Null), |a| {
                eval.with_temporary_results(|| {
                    eval.with_call_context(false, || eval.evaluate(a)).flatten()
                })
                .flatten()
            });
            self.defaults
                .borrow_mut()
                .insert(q.statement_id.as_str(), value);
        }
        self.registering.set(false);
    }
}
impl EvaluationScope for TreeScope<'_> {
    fn cache_references(&self) -> bool {
        !self.registering.get()
    }
    fn wants_unmapped_call(&self, call: &Value, name: &str) -> bool {
        self.functions.is_some_and(|f| f.get(name).is_some())
            || matches!(call,Value::Obj(o) if matches!(o.get("name"),Some(Value::Str(n)) if matches!(n.as_str(),"Action"|"Run"|"Set"|"Reset"|"ToAssistant"|"OpenUrl")))
            || self
                .actions
                .and_then(|a| a.as_object())
                .is_some_and(|a| a.contains_key(name))
    }
    fn call_output(
        &self,
        call: &Ast,
        eval: &ExpressionEvaluator<'_>,
    ) -> openui_lang::ExpressionOutput {
        if let Ast::Comp { name, .. } = call
            && name == "Action"
        {
            let value = Value::Ast(Arc::new(call.clone()));
            if let Some(output) = self.aggregate(&own_fields(&value), eval) {
                return output;
            }
        }
        self.call(call, eval).into()
    }
    fn call_value_output(
        &self,
        call: &Value,
        name: &str,
        eval: &ExpressionEvaluator<'_>,
    ) -> openui_lang::ExpressionOutput {
        if let Value::Obj(o) = call
            && let Some(output) = self.aggregate(o, eval)
        {
            return output;
        }
        self.call_value_named(call, name, eval).into()
    }

    fn project_result<'v>(
        &self,
        value: ExpressionValue<'v>,
        state: &dyn Fn(&str) -> ExpressionValue<'v>,
    ) -> Option<ExpressionValue<'v>> {
        bindings::project_marker(value, state)
    }
    fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.store.get(name)
    }
    fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let Some(default) = self.references.get(name) else {
            return Some(Value::Null);
        };
        if let Some(values) = self.reference_values {
            match values.get(name) {
                Ok(Some((value, _))) => return Some(value.clone()),
                Err(flags) => {
                    eval.mark_cut(flags);
                    return Some(Value::Null);
                }
                Ok(None) => {}
            }
        }
        if !self.registering.get() && default.is_some() {
            return self
                .defaults
                .borrow()
                .get(name)
                .cloned()
                .unwrap_or(Some(Value::Null));
        }
        match default {
            None => {
                let mut idle = JsObject::new();
                idle.define("status", Value::Str("idle".into()));
                Some(Value::Obj(Arc::new(idle)))
            }
            Some((false, _)) => Some(Value::Null),
            Some((true, default)) => default.map_or(Some(Value::Null), |a| {
                // Query registration evaluates defaults with the root context,
                // before any prop installs its diagnostic reporter.
                let _report = ReportContext {
                    bytes: &self.report_bytes,
                    old: self.report_bytes.replace(None),
                };
                eval.with_call_context(false, || eval.evaluate(a)).flatten()
            }),
        }
    }
    fn prepared_reference(&self, name: &str) -> Option<&crate::PreparedReferenceValue> {
        if !self.references.contains_key(name) {
            return None;
        }
        self.reference_values?
            .get(name)
            .ok()
            .flatten()
            .map(|(_, prepared)| prepared)
    }
    fn wants_each_item(&self, value: &ExpressionValue<'_>, context: bool) -> bool {
        context
            && value.property("type").as_str() == Some("element")
            && value.property("typeName").as_str().is_some()
            && value.property("props").is_object()
            && value.property("partial").as_bool().is_some()
            && value.property("hasDynamicProps").as_bool() != Some(false)
    }
    fn each_item(&self, value: Option<&Value>, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let value = value.filter(|v| element_value(v))?;
        let dynamic = match value {
            Value::Element(e) => e.has_dynamic_props,
            Value::Obj(o) => !matches!(o.get("hasDynamicProps"), Some(Value::Bool(false))),
            _ => false,
        };
        if !dynamic || !eval.call_context() {
            return None;
        }
        let _inline = Inline::enter(&self.inline);
        self.expression_result(Some(value.clone()), eval)
    }
    fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        if let Ast::Comp { name, .. } = call
            && (matches!(
                name.as_str(),
                "Action" | "Run" | "Set" | "Reset" | "ToAssistant" | "OpenUrl"
            ) || self.functions.is_some_and(|f| f.get(name).is_some())
                || self
                    .actions
                    .and_then(|a| a.as_object())
                    .is_some_and(|a| a.contains_key(name)))
        {
            let value = Value::Ast(Arc::new(call.clone()));
            let node = own_fields(&value);
            if self.functions.is_some_and(|f| f.get(name).is_some())
                && !matches!(node.get("name"),Some(Value::Str(n)) if matches!(n.as_str(),"Action"|"Run"|"Set"|"Reset"|"ToAssistant"|"OpenUrl"))
            {
                return eval
                    .with_temporary_results(|| self.declared_call(&node, name, eval))
                    .flatten();
            }
            if let Some(plan) = self.declared_call(&node, name, eval) {
                return Some(plan);
            }
        }
        let Ast::Comp {
            name,
            mapped_props: Some(mapped),
            ..
        } = call
        else {
            return Some(Value::Null);
        };
        self.component(
            Some(Value::Str(name.clone())),
            name,
            mapped.iter().map(|(k, v)| (k.clone(), CallProp::Typed(v))),
            eval,
        )
    }
    fn call_value_named(
        &self,
        call: &Value,
        name: &str,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        let Value::Obj(o) = call else {
            return Some(Value::Null);
        };
        if self.functions.is_some_and(|f| f.get(name).is_some())
            && !matches!(o.get("name"),Some(Value::Str(n)) if matches!(n.as_str(),"Action"|"Run"|"Set"|"Reset"|"ToAssistant"|"OpenUrl"))
        {
            return eval
                .with_temporary_results(|| self.declared_call(o, name, eval))
                .flatten();
        }
        if let Some(plan) = self.declared_call(o, name, eval) {
            return Some(plan);
        }
        let Some(mapped) = o.get("mappedProps").filter(|v| match v {
            Value::Null | Value::Bool(false) => false,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }) else {
            return Some(Value::Null);
        };
        match mapped {
            Value::Obj(fields) => self.component(
                o.get("name").cloned(),
                name,
                fields.iter().map(|(k, v)| (k.clone(), CallProp::Data(v))),
                eval,
            ),
            Value::Arr(fields) => self.component(
                o.get("name").cloned(),
                name,
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), CallProp::Data(v))),
                eval,
            ),
            _ => {
                let fields = own_fields(mapped);
                self.component(
                    o.get("name").cloned(),
                    name,
                    fields.iter().map(|(k, v)| (k.clone(), CallProp::Data(v))),
                    eval,
                )
            }
        }
    }
}
enum CallProp<'a> {
    Typed(&'a Ast),
    Data(&'a Value),
}
impl TreeScope<'_> {
    fn component<'v>(
        &self,
        name: Option<Value>,
        binding_component: &str,
        mapped: impl IntoIterator<Item = (String, CallProp<'v>)>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        if let Some(Value::Str(name)) = &name
            && self.library.and_then(|l| l.call(name)).is_some()
        {
            return Some(Value::Null);
        }
        let contextual = eval.call_context();
        let bindings = if contextual {
            self.binding_props
                .and_then(|b| b.component(binding_component))
        } else {
            None
        };
        let mut props = JsObject::new();
        for (key, value) in mapped {
            if cut(eval) {
                break;
            }
            let state_ref = match &value {
                CallProp::Typed(a) => matches!(a, Ast::StateRef(_)),
                CallProp::Data(Value::Ast(a)) => matches!(a.as_ref(), Ast::StateRef(_)),
                CallProp::Data(Value::Obj(o)) => {
                    matches!(o.get("k"),Some(Value::Str(k)) if k=="StateRef")
                }
                _ => false,
            };
            if contextual && state_ref && bindings.is_some_and(|b| b.contains(&key)) {
                let marker = match &value {
                    CallProp::Typed(a) => bindings::state_marker_ast(a),
                    CallProp::Data(v) => bindings::state_marker(v),
                };
                if let Some(marker) = marker {
                    // The enclosing expression projection charges this once.
                    self.remember_marker(&marker);
                    props.set(key, marker);
                }
            } else if !contextual && state_ref {
                props.set(
                    key,
                    match value {
                        CallProp::Typed(a) => Value::Ast(Arc::new(a.clone())),
                        CallProp::Data(v) => v.clone(),
                    },
                );
            } else {
                let result = match value {
                    CallProp::Typed(a) => eval.evaluate_result(a),
                    CallProp::Data(v) => eval.evaluate_call_prop(v),
                };
                let fresh = !result.reuses_data();
                if let Some(value) = result.value {
                    if fresh && !element_value(&value) && bindings::is_marker(&value) {
                        self.remember_marker(&value);
                    }
                    props.set(key, value);
                }
            }
        }
        if contextual
            && props
                .values()
                .any(|v| element_value(v) || matches!(v, Value::Arr(_)))
        {
            let _inline = Inline::enter(&self.inline);
            for (key, value) in props.clone().iter() {
                if (element_value(value) || matches!(value, Value::Arr(_)))
                    && let Some(value) = self.expression_result(Some(value.clone()), eval)
                {
                    props.set(key.clone(), value);
                }
            }
        }
        if let Some(Value::Str(name)) = &name {
            return Some(Value::Element(Arc::new(Element {
                type_name: name.clone(),
                props,
                partial: false,
                has_dynamic_props: true,
                statement_id: None,
            })));
        }
        let mut out = JsObject::new();
        out.define("type", Value::Str("element".into()));
        if let Some(name) = name {
            out.define("typeName", name);
        }
        out.define("props", Value::Obj(Arc::new(props)));
        out.define("partial", Value::Bool(false));
        out.define("hasDynamicProps", Value::Bool(true));
        Some(Value::Obj(Arc::new(out)))
    }
}
struct Inline<'a> {
    flag: &'a Cell<bool>,
    old: bool,
}
impl<'a> Inline<'a> {
    fn enter(flag: &'a Cell<bool>) -> Self {
        Self {
            flag,
            old: flag.replace(true),
        }
    }
}
impl Drop for Inline<'_> {
    fn drop(&mut self) {
        self.flag.set(self.old);
    }
}
#[derive(Clone, Copy)]
struct PropOwner<'a> {
    component: &'a str,
    statement: Option<&'a str>,
    bindings: Option<&'a std::collections::BTreeSet<String>>,
}
struct ReportContext<'a> {
    bytes: &'a Cell<Option<usize>>,
    old: Option<usize>,
}
impl<'a> ReportContext<'a> {
    fn enter(bytes: &'a Cell<Option<usize>>, value: usize) -> Self {
        Self {
            bytes,
            old: bytes.replace(Some(value)),
        }
    }
}
impl Drop for ReportContext<'_> {
    fn drop(&mut self) {
        self.bytes.set(self.old);
    }
}
fn error_bytes(owner: PropOwner<'_>, key: &str) -> usize {
    160usize
        .saturating_add(text_bytes(owner.component).saturating_mul(2))
        .saturating_add(text_bytes(key).saturating_mul(2))
        .saturating_add(
            owner
                .statement
                .map_or(0, |s| text_bytes(s).saturating_add(15)),
        )
}
fn cut(eval: &ExpressionEvaluator<'_>) -> bool {
    let f = eval.flags();
    f.size_limited || f.depth_limited
}
fn text_bytes(s: &str) -> usize {
    s.chars().fold(2usize, |n, c| {
        n.saturating_add(match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if c < '\u{20}' => 6,
            c => c.len_utf8(),
        })
    })
}
fn expression(value: &Value) -> bool {
    match value {
        Value::Ast(_) => true,
        Value::Obj(o) => matches!(o.get("k"), Some(Value::Str(k)) if matches!(k.as_str(),
            "Comp"|"Str"|"Num"|"Bool"|"Null"|"Arr"|"Obj"|"Ref"|"Ph"|"StateRef"|"RuntimeRef"|"BinOp"|"UnaryOp"|"Ternary"|"Member"|"Index"|"Assign")),
        _ => false,
    }
}
fn element_value(v: &Value) -> bool {
    matches!(v, Value::Element(_)) || matches!(v,Value::Obj(o) if shaped_element(o))
}
fn object(v: &Value) -> bool {
    matches!(
        v,
        Value::Arr(_) | Value::Obj(_) | Value::Element(_) | Value::Ast(_)
    )
}
impl TreeScope<'_> {
    fn remember_marker(&self, value: &Value) {
        if let Value::Obj(o) = value {
            self.paid_markers
                .borrow_mut()
                .insert(Arc::as_ptr(o) as usize, o.clone());
        }
    }
    fn reserve_marker(&self, value: &Value, eval: &ExpressionEvaluator<'_>) {
        let paid = match value {
            Value::Obj(o) => self
                .paid_markers
                .borrow()
                .contains_key(&(Arc::as_ptr(o) as usize)),
            _ => false,
        };
        if !paid {
            eval.reserve_value(value);
        }
    }
    fn marker_value(
        &self,
        raw: &Value,
        binding: bool,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        if binding {
            self.reserve_marker(raw, eval);
            return Some(raw.clone());
        }
        if let Value::Obj(o) = raw
            && let Some(Value::Str(name)) = o.get("target")
            && let Some(value) = eval.evaluate(&Ast::StateRef(name.clone()))
        {
            return Some(value);
        }
        // Map keys remain literal; raw markers pay for their null result.
        eval.reserve_result(4);
        Some(Value::Null)
    }

    fn prop(
        &self,
        raw: &Value,
        binding: bool,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<Value>, EvaluationFailure> {
        // Cuts preserve raw input, including raw plans, rather than running
        // an unbounded sanitizing walk with no work left. Tree position never
        // confers authority: custom steps must pass the store registry check.
        self.prop_value(raw, binding, eval)
            .map(|v| v.map(|v| if cut(eval) { v } else { self.own_plan(v) }))
    }
    fn prop_value(
        &self,
        raw: &Value,
        binding: bool,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<Value>, EvaluationFailure> {
        if cut(eval) {
            return Ok(Some(raw.clone()));
        }
        eval.with_level(|| {
            if bindings::is_marker(raw) {
                let value = self.marker_value(raw, binding, eval);
                return Ok(if cut(eval) { Some(raw.clone()) } else { value });
            }
            if expression(raw) {
                if binding && let Some(marker) = bindings::state_marker(raw) {
                    self.reserve_marker(&marker, eval);
                    if cut(eval) {
                        return Ok(Some(raw.clone()));
                    }
                    self.remember_marker(&marker);
                    return Ok(Some(marker));
                }
                let Some((result, resolved)) = eval.with_call_context(true, || {
                    let evaluate = |eval: &ExpressionEvaluator<'_>| match raw {
                        Value::Ast(a) => eval.evaluate_result(a),
                        _ => eval.evaluate_value(raw),
                    };
                    if binding {
                        (evaluate(eval), false)
                    } else {
                        eval.with_projection(evaluate)
                    }
                }) else {
                    return Ok(Some(raw.clone()));
                };
                if result.size_limited || result.depth_limited {
                    return Ok(Some(raw.clone()));
                }
                if let Some(failure) = result.failure {
                    return Err(failure);
                }
                if resolved {
                    return Ok(result.value);
                }
                // Result order differs from raw values: elements and lists
                // recurse before a coincident reactive tag is considered.
                if result
                    .value
                    .as_ref()
                    .is_some_and(|v| element_value(v) || matches!(v, Value::Arr(_)))
                {
                    return Ok(self.expression_result(result.value, eval));
                }
                if let Some(marker) = result.value.as_ref().filter(|v| bindings::is_marker(v)) {
                    // The evaluator has already charged this marker's output.
                    if !result.reuses_data() {
                        self.remember_marker(marker);
                    }
                    return Ok(result.value);
                }
                return Ok(self.expression_result(result.value, eval));
            }
            match raw {
                Value::Element(e) if !e.has_dynamic_props => {
                    eval.reserve_value(raw);
                    Ok(Some(raw.clone()))
                }
                Value::Element(e) => Ok(Some(self.element(e, eval))),
                Value::Arr(items) => {
                    if !eval.reserve_result(2usize.saturating_add(items.len().saturating_sub(1))) {
                        return Ok(Some(raw.clone()));
                    }
                    let mut out = Vec::new();
                    for item in items.iter() {
                        out.push(self.prop(item, binding, eval)?.unwrap_or(Value::Null));
                    }
                    Ok(Some(Value::Arr(Arc::new(out))))
                }
                Value::Obj(o) if shaped_element(o) => Ok(Some(self.shaped(raw, o, eval))),
                Value::Obj(_) if actions::is_plan(raw) => {
                    eval.reserve_value(raw);
                    Ok(Some(raw.clone()))
                }
                Value::Obj(o) if o.values().any(object) => {
                    if !eval.reserve_result(2usize.saturating_add(o.len().saturating_sub(1))) {
                        return Ok(Some(raw.clone()));
                    }
                    let mut out = JsObject::new();
                    for (k, v) in o.iter() {
                        if cut(eval) || !eval.reserve_result(text_bytes(k).saturating_add(1)) {
                            out.set(k.clone(), v.clone());
                        } else if let Some(v) = self.prop(v, binding, eval)? {
                            out.set(k.clone(), v);
                        }
                    }
                    Ok(Some(Value::Obj(Arc::new(out))))
                }
                _ => {
                    eval.reserve_value(raw);
                    Ok(Some(raw.clone()))
                }
            }
        })
        .unwrap_or_else(|| Ok(Some(raw.clone())))
    }
    fn expression_result(
        &self,
        result: Option<Value>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        match result {
            Some(Value::Element(ref e)) if !e.has_dynamic_props => result,
            Some(Value::Element(ref e)) => Some(self.element(e, eval)),
            Some(Value::Obj(ref o)) if shaped_element(o) => {
                Some(self.shaped(result.as_ref()?, o, eval))
            }
            Some(Value::Arr(ref a)) if a.iter().any(element_value) => {
                let mut out = Vec::new();
                for item in a.iter() {
                    out.push(if element_value(item) {
                        self.expression_result(Some(item.clone()), eval)
                            .unwrap_or(Value::Null)
                    } else {
                        item.clone()
                    });
                }
                Some(Value::Arr(Arc::new(out)))
            }
            other => other,
        }
    }
    fn prop_on(
        &self,
        owner: PropOwner<'_>,
        key: &str,
        raw: &Value,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        if cut(eval) || !eval.reserve_result(text_bytes(key).saturating_add(1)) {
            return Some(raw.clone());
        }
        let _report = (!self.inline.get())
            .then(|| ReportContext::enter(&self.report_bytes, error_bytes(owner, key)));
        let before = self.errors.borrow().len();
        let result = self.prop(raw, owner.bindings.is_some_and(|b| b.contains(key)), eval);
        if !self.inline.get() {
            for error in self.errors.borrow_mut().iter_mut().skip(before) {
                if error.component.is_none() {
                    error.component = Some(owner.component.to_owned());
                    error.statement_id = owner.statement.map(str::to_owned);
                    error.message = format!(
                        "Evaluating prop \"{key}\" on {} failed: {}",
                        owner.component, error.message
                    );
                    error.hint = Some(format!("Check the expression used for prop \"{key}\""));
                }
            }
        }
        match result {
            Ok(value) => value,
            Err(_) if self.inline.get() => Some(raw.clone()),
            Err(failure) => {
                self.report(owner, key, failure_message(&failure), eval);
                Some(raw.clone())
            }
        }
    }
    fn report(
        &self,
        owner: PropOwner<'_>,
        key: &str,
        message: String,
        eval: &ExpressionEvaluator<'_>,
    ) {
        let bytes = error_bytes(owner, key).saturating_add(text_bytes(&message));
        if eval.reserve_result(bytes) {
            self.errors.borrow_mut().push(RuntimeError {
                code: "runtime-error",
                component: Some(owner.component.to_owned()),
                statement_id: owner.statement.map(str::to_owned),
                message: format!(
                    "Evaluating prop \"{key}\" on {} failed: {message}",
                    owner.component
                ),
                hint: Some(format!("Check the expression used for prop \"{key}\"")),
            });
        }
    }
    fn element(&self, e: &Element, eval: &ExpressionEvaluator<'_>) -> Value {
        if cut(eval) {
            return Value::Element(Arc::new(e.clone()));
        }
        eval.with_level(|| {
            if !e.has_dynamic_props {
                let v = Value::Element(Arc::new(e.clone()));
                eval.reserve_value(&v);
                return v;
            }
            let bytes = r#"{"type":"element","typeName":,"props":{},"partial":false,"hasDynamicProps":false}"#.len().saturating_sub(usize::from(e.partial)).saturating_sub(usize::from(e.has_dynamic_props))
                .saturating_add(text_bytes(&e.type_name))
                .saturating_add(e.props.len().saturating_sub(1))
                .saturating_add(
                    e.statement_id
                        .as_ref()
                        .map_or(0, |s| text_bytes(s).saturating_add(15)),
                );
            if !eval.reserve_result(bytes) {
                return Value::Element(Arc::new(e.clone()));
            }
            let owner = PropOwner {
                component: &e.type_name,
                bindings: self.binding_props.and_then(|b|b.component(&e.type_name)),
                statement: e.statement_id.as_deref(),
            };
            let mut props = JsObject::new();
            for (k, v) in e.props.iter() {
                if let Some(v) = self.prop_on(owner, k, v, eval) {
                    props.set(k.clone(), v);
                }
            }
            Value::Element(Arc::new(Element {
                type_name: e.type_name.clone(),
                props,
                partial: e.partial,
                has_dynamic_props: e.has_dynamic_props,
                statement_id: e.statement_id.clone(),
            }))
        })
        .unwrap_or_else(|| Value::Element(Arc::new(e.clone())))
    }
    fn shaped(&self, raw: &Value, o: &JsObject<Value>, eval: &ExpressionEvaluator<'_>) -> Value {
        if cut(eval) {
            return raw.clone();
        }
        eval.with_level(|| {
            if matches!(o.get("hasDynamicProps"), Some(Value::Bool(false))) {
                eval.reserve_value(raw);
                return raw.clone();
            }
            if !eval.reserve_result(2usize.saturating_add(o.len().saturating_sub(1))) {
                return raw.clone();
            }
            for (k, v) in o.iter() {
                if !eval.reserve_result(text_bytes(k).saturating_add(1)) {
                    return raw.clone();
                }
                if k != "props" && !eval.reserve_value(v) {
                    return raw.clone();
                }
            }
            let fields = o.get("props").map(own_fields).unwrap_or_default();
            if !eval.reserve_result(2usize.saturating_add(fields.len().saturating_sub(1))) {
                return raw.clone();
            }
            let component = match o.get("typeName") {
                Some(Value::Str(s)) => s.as_str(),
                _ => "",
            };
            let owner = PropOwner {
                component,
                bindings: self.binding_props.and_then(|b| b.component(component)),
                statement: match o.get("statementId") {
                    Some(Value::Str(s)) => Some(s),
                    _ => None,
                },
            };
            let mut props = JsObject::new();
            for (k, v) in fields.iter() {
                if let Some(v) = self.prop_on(owner, k, v, eval) {
                    props.set(k.clone(), v);
                }
            }
            let mut out = o.clone();
            out.define("props", Value::Obj(Arc::new(props)));
            Value::Obj(Arc::new(out))
        })
        .unwrap_or_else(|| raw.clone())
    }
}
fn failure_message(failure: &EvaluationFailure) -> String {
    match failure {
        EvaluationFailure::Callback(message) => message.clone(),
        EvaluationFailure::CannotConvertToPrimitive => "No default value".into(),
        EvaluationFailure::BuiltinNotCallable => "builtin.fn is not a function. (In 'builtin.fn(evaluateArgs(node, context))', 'builtin.fn' is undefined)".into(),
        EvaluationFailure::InvalidExpression {
            field,
            operation,
            actual,
            ..
        } => {
            let nullish = match actual {
                ExpressionType::Null => "null",
                _ => "undefined",
            };
            match operation {
                ExpressionOperation::ReadKind => {
                    let receiver = match *field {
                        "callProp" => "val",
                        "eachVar" => "args[1]",
                        _ => "node",
                    };
                    format!("{nullish} is not an object (evaluating '{receiver}.k')")
                }
                ExpressionOperation::Length => {
                    format!("{nullish} is not an object (evaluating 'args.length')")
                }
                ExpressionOperation::Map | ExpressionOperation::SubstituteMap
                    if !matches!(actual, ExpressionType::Undefined | ExpressionType::Null) =>
                {
                    let map = match (*operation,*field) {
                        (ExpressionOperation::SubstituteMap,"els") => "(e) => substituteRef(e, varName, value)",
                        (ExpressionOperation::SubstituteMap,"args") => "sub",
                        (ExpressionOperation::SubstituteMap,_) => "([k, v]) => [k, substituteRef(v, varName, value)]",
                        (_,"els") => "(el) => evaluate(el, context)",
                        _ => "([k, v]) => [k, evaluate(v, context)]",
                    };
                    format!(
                        "node.{field}.map is not a function. (In 'node.{field}.map({map})', 'node.{field}.map' is undefined)"
                    )
                }
                ExpressionOperation::Map | ExpressionOperation::SubstituteMap => {
                    format!("{nullish} is not an object (evaluating 'node.{field}.map')")
                }
                ExpressionOperation::Entry => match actual {
                    ExpressionType::Null => "null is not an object (evaluating '[k, v]')".into(),
                    ExpressionType::Number => "number is not iterable".into(),
                    ExpressionType::Boolean(b) => format!("{b} is not iterable"),
                    ExpressionType::Object => "{} is not iterable".into(),
                    _ => "Invalid AST entry".into(),
                },
                _ => "Invalid expression".into(),
            }
        }
        _ => "Expression evaluation failed".into(),
    }
}
