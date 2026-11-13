//! Evaluating parsed prop trees with one expression allowance.
use crate::{
    Ast, EvaluationFailure, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    ExpressionOperation, ExpressionType, JsObject, Limits, Store, Value,
};
use openui_lang::{Element, ParseResult};
use std::{cell::RefCell, sync::Arc};
mod fields;
mod input;
use fields::{own_fields, shaped_element};

/// Extensible input to tree evaluation. Parsing already inlines value statements.
/// Binding-prop configuration will be added here in ticket 35. Component
/// arguments are already mapped by parsing; custom-action validation in ticket
/// 36 is the first planned use of library definitions during tree evaluation.
#[derive(Clone)]
#[non_exhaustive]
pub struct TreeInput<'a> {
    /// The parse result. Evaluation reads the store without initializing it.
    pub result: &'a ParseResult,
}
impl<'a> TreeInput<'a> {
    /// Evaluate this parse result; future options have compatible defaults.
    pub fn new(result: &'a ParseResult) -> Self {
        Self { result }
    }
}
impl<'a> From<&'a ParseResult> for TreeInput<'a> {
    fn from(result: &'a ParseResult) -> Self {
        Self::new(result)
    }
}

/// A prop failure, in upstream's encounter order.
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
    /// None when the parse has no root; cuts retain the root and fitted props.
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
    let bytes = match input::input_size(input.result, limits) {
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
    let scope = TreeScope {
        store,
        errors: RefCell::new(Vec::new()),
    };
    let session = EvaluationSession::with_limits(&scope, bytes, limits);
    let root = input
        .result
        .root
        .as_ref()
        .map(|e| session.with_evaluator(|eval| scope.element(e, eval)));
    let flags = session.flags();
    TreeResult {
        root,
        errors: scope.errors.into_inner(),
        size_limited: flags.size_limited,
        depth_limited: flags.depth_limited,
    }
}
struct TreeScope<'a> {
    store: &'a Store,
    errors: RefCell<Vec<RuntimeError>>,
}
impl EvaluationScope for TreeScope<'_> {
    fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.store.get(name)
    }
    // Parsed value statements are inlined. External query/mutation values are #34.
    fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
}
#[derive(Clone, Copy)]
struct PropOwner<'a> {
    component: &'a str,
    statement: Option<&'a str>,
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
fn assign_descriptor(value: &Value) -> bool {
    let Value::Obj(o) = value else {
        return false;
    };
    matches!(o.get("k"), Some(Value::Str(k)) if k == "Assign")
}
impl TreeScope<'_> {
    fn prop(
        &self,
        raw: &Value,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<Value>, EvaluationFailure> {
        if cut(eval) {
            return Ok(Some(raw.clone()));
        }
        eval.with_level(|| {
            if expression(raw) {
                // Assignment descriptors/binding markers belong to ticket 35.
                if assign_descriptor(raw) {
                    return Ok(Some(Value::Null));
                }
                let result = match raw {
                    Value::Ast(a) => eval.evaluate_result(a),
                    _ => eval.evaluate_value(raw),
                };
                if result.size_limited || result.depth_limited {
                    return Ok(Some(raw.clone()));
                }
                if let Some(failure) = result.failure {
                    return Err(failure);
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
                        out.push(self.prop(item, eval)?.unwrap_or(Value::Null));
                    }
                    Ok(Some(Value::Arr(Arc::new(out))))
                }
                Value::Obj(o) if shaped_element(o) => Ok(Some(self.shaped(raw, o, eval))),
                Value::Obj(o) if o.values().any(object) => {
                    if !eval.reserve_result(2usize.saturating_add(o.len().saturating_sub(1))) {
                        return Ok(Some(raw.clone()));
                    }
                    let mut out = JsObject::new();
                    for (k, v) in o.iter() {
                        if cut(eval) || !eval.reserve_result(text_bytes(k).saturating_add(1)) {
                            out.set(k.clone(), v.clone());
                        } else if let Some(v) = self.prop(v, eval)? {
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
        match self.prop(raw, eval) {
            Ok(value) => value,
            Err(failure) => {
                let message = failure_message(&failure);
                // Reserve repeated owner/key text before formatting the diagnostic.
                let bytes = 160usize
                    .saturating_add(text_bytes(owner.component).saturating_mul(2))
                    .saturating_add(text_bytes(key).saturating_mul(2))
                    .saturating_add(text_bytes(&message))
                    .saturating_add(
                        owner
                            .statement
                            .map_or(0, |s| text_bytes(s).saturating_add(15)),
                    );
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
                Some(raw.clone())
            }
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
            let owner = PropOwner {
                component: match o.get("typeName") {
                    Some(Value::Str(s)) => s,
                    _ => "",
                },
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
        EvaluationFailure::CannotConvertToPrimitive => "No default value".into(),
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
                    format!("{nullish} is not an object (evaluating 'node.k')")
                }
                ExpressionOperation::Length => {
                    format!("{nullish} is not an object (evaluating 'args.length')")
                }
                ExpressionOperation::Map
                    if !matches!(actual, ExpressionType::Undefined | ExpressionType::Null) =>
                {
                    let map = if *field == "els" {
                        "(el) => evaluate(el, context)"
                    } else {
                        "([k, v]) => [k, evaluate(v, context)]"
                    };
                    format!(
                        "node.{field}.map is not a function. (In 'node.{field}.map({map})', 'node.{field}.map' is undefined)"
                    )
                }
                ExpressionOperation::Map => {
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
