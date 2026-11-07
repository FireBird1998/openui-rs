//! Prop validation against the component library's JSON Schema, as the
//! reference parser's `validateSchemaValue` and `resolveInvalidValue`, with
//! every message worded exactly as the reference words it.

use serde_json::Value as Json;

use crate::js::{
    JsObject, OBJECT_PROTOTYPE_KEYS, js_string, json_entries, json_string, json_value_string,
    write_json_number,
};
use crate::library::Param;
use crate::materialize::Ctx;
use crate::result::{Severity, ValidationErrorCode, Value};

/// The varying parts of a validation error; the wording lives in [`Issue::message`].
pub(crate) enum Issue {
    TypeMismatch { expected: String, actual: String },
    MissingRequired { signature: Option<String> },
    NullRequired { signature: Option<String> },
    UnknownComponent { available: Vec<String> },
    InlineReserved,
    ExcessArgs { declared: usize, got: usize },
    NoRoot { reason: NoRootReason },
    UnknownFunction,
    QueryCycle { cycle: Vec<String> },
}

/// Why a program has no usable entry, which the `no-root` message words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoRootReason {
    /// No `root`, and no statement calls the root component.
    Missing,
    /// `root` is there but isn't a component.
    NotComponent,
    /// No `root`; a statement calling the root component stands in.
    Fallback,
}

impl Issue {
    pub(crate) fn code(&self) -> ValidationErrorCode {
        match self {
            Issue::TypeMismatch { .. } => ValidationErrorCode::TypeMismatch,
            Issue::MissingRequired { .. } => ValidationErrorCode::MissingRequired,
            Issue::NullRequired { .. } => ValidationErrorCode::NullRequired,
            Issue::UnknownComponent { .. } => ValidationErrorCode::UnknownComponent,
            Issue::InlineReserved => ValidationErrorCode::InlineReserved,
            Issue::ExcessArgs { .. } => ValidationErrorCode::ExcessArgs,
            Issue::NoRoot { .. } => ValidationErrorCode::NoRoot,
            Issue::UnknownFunction => ValidationErrorCode::UnknownFunction,
            Issue::QueryCycle { .. } => ValidationErrorCode::QueryCycle,
        }
    }

    /// The fallback `no-root` is a warning: the program still renders.
    pub(crate) fn severity(&self) -> Option<Severity> {
        match self {
            Issue::NoRoot {
                reason: NoRootReason::Fallback,
            } => Some(Severity::Warning),
            _ => None,
        }
    }

    pub(crate) fn message(&self, component: &str, path: &str) -> String {
        let hint = |signature: &Option<String>| match signature {
            Some(s) if !s.is_empty() => format!(" — signature: {s}"),
            _ => String::new(),
        };
        match self {
            Issue::TypeMismatch { expected, actual } => {
                format!("field \"{path}\" expects {expected} but got {actual}")
            }
            Issue::MissingRequired { signature } => {
                format!("missing required field \"{path}\"{}", hint(signature))
            }
            Issue::NullRequired { signature } => {
                format!(
                    "required field \"{path}\" cannot be null{}",
                    hint(signature)
                )
            }
            Issue::UnknownComponent { available } => {
                let list = if available.is_empty() {
                    String::new()
                } else {
                    format!(". Available components: {}", available.join(", "))
                };
                format!(
                    "Unknown component \"{component}\" — not found in catalog or builtins{list}"
                )
            }
            Issue::InlineReserved => {
                format!(
                    "{component}() must be declared as a top-level statement, not used inline as a value"
                )
            }
            Issue::ExcessArgs { declared, got } => format!(
                "{component} takes {declared} arg(s), got {got} ({} excess dropped)",
                got - declared
            ),
            Issue::UnknownFunction => format!(
                "Unknown function \"@{component}\": not a built-in or action step. The call evaluates to null"
            ),
            Issue::QueryCycle { cycle } => format!(
                "Queries {} read each other's results, so none of them can run first. They do not run",
                cycle.join(", ")
            ),
            Issue::NoRoot { reason } => match reason {
                NoRootReason::NotComponent => "root does not evaluate to a component".to_owned(),
                NoRootReason::Fallback => format!(
                    "No root statement. Rendering the first statement that calls the root component {component}"
                ),
                NoRootReason::Missing if component.is_empty() => "No root statement".to_owned(),
                NoRootReason::Missing => format!(
                    "No root statement, and no statement calls the root component {component}"
                ),
            },
        }
    }
}

/// A typed signature such as `Header(title*: string, variant: "a"|"b")`; `*`
/// marks required params.
pub(crate) fn params_signature(component: &str, params: &[Param]) -> String {
    let rendered: Vec<String> = params
        .iter()
        .map(|p| {
            let marked = if p.required {
                format!("{}*", p.name)
            } else {
                p.name.clone()
            };
            match type_from_schema(&p.schema) {
                Some(ty) if !ty.is_empty() => format!("{marked}: {ty}"),
                _ => marked,
            }
        })
        .collect();
    format!("{component}({})", rendered.join(", "))
}

/// A schema leaf's allowed literal values, or its scalar type.
enum Leaf<'a> {
    Enum(Vec<&'a Json>),
    Type(&'static str),
    Unchecked,
}

/// The reference's `getScalarTypeInfo`: `enum`, then `const`, then `type`.
fn scalar_leaf(s: &serde_json::Map<String, Json>) -> Leaf<'_> {
    if let Some(Json::Array(values)) = s.get("enum") {
        return Leaf::Enum(values.iter().collect());
    }
    if let Some(value) = s.get("const") {
        return Leaf::Enum(vec![value]);
    }
    match s.get("type").and_then(Json::as_str) {
        Some("string") => Leaf::Type("string"),
        Some("number" | "integer") => Leaf::Type("number"),
        Some("boolean") => Leaf::Type("boolean"),
        _ => Leaf::Unchecked,
    }
}

/// The type hint for a param's schema, as the reference `getTypeFromSchema`.
fn type_from_schema(schema: &Json) -> Option<String> {
    let p = schema.as_object()?;
    if let Some(r) = p.get("$ref").and_then(Json::as_str) {
        return r.rsplit('/').next().map(str::to_owned);
    }
    match scalar_leaf(p) {
        Leaf::Enum(values) => Some(
            values
                .into_iter()
                .map(json_value_string)
                .collect::<Vec<_>>()
                .join("|"),
        ),
        Leaf::Type(t) => Some(t.to_owned()),
        Leaf::Unchecked => p.get("type").and_then(Json::as_str).map(str::to_owned),
    }
}

/// `typeof value` in JavaScript.
fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Str(_) => "string",
        Value::Num(_) => "number",
        Value::Bool(_) => "boolean",
        _ => "object",
    }
}

/// `typeof value`, but `array` for arrays (the reference's `jsType`).
fn js_type(value: &Value) -> &'static str {
    match value {
        Value::Arr(_) => "array",
        other => type_of(other),
    }
}

/// `JSON.stringify(value)` for a scalar.
fn scalar_json(value: &Value) -> String {
    match value {
        Value::Str(s) => json_string(s),
        Value::Num(n) => {
            let mut out = String::new();
            write_json_number(&mut out, *n);
            out
        }
        Value::Bool(b) => b.to_string(),
        _ => "null".to_owned(),
    }
}

/// `enumValues.includes(value)` (SameValueZero: NaN equals NaN).
fn enum_includes(values: &[&Json], value: &Value) -> bool {
    values.iter().any(|v| match (v, value) {
        (Json::String(a), Value::Str(b)) => a == b,
        (Json::Number(a), Value::Num(b)) => {
            let a = crate::js::json_number(a);
            a == *b || (a.is_nan() && b.is_nan())
        }
        (Json::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    })
}

const AST_KINDS: [&str; 17] = [
    "Comp",
    "Ref",
    "StateRef",
    "RuntimeRef",
    "BinOp",
    "UnaryOp",
    "Ternary",
    "Member",
    "Index",
    "Assign",
    "Str",
    "Num",
    "Bool",
    "Null",
    "Arr",
    "Obj",
    "Ph",
];

/// The reference's `isASTNode`: an AST node, or any object whose `k` is an AST kind.
pub(crate) fn is_ast_node(value: &Value) -> bool {
    match value {
        Value::Ast(_) => true,
        Value::Obj(o) => {
            matches!(o.get("k"), Some(Value::Str(k)) if AST_KINDS.contains(&k.as_str()))
        }
        _ => false,
    }
}

/// The reference's `isElementNode`: an element, or an object shaped like one.
/// Returns the type name and the props, when it is one.
pub(crate) fn element_parts(value: &Value) -> Option<(&str, Option<&JsObject<Value>>)> {
    match value {
        Value::Element(e) => Some((&e.type_name, Some(&e.props))),
        Value::Obj(o) => {
            let is_element = matches!(o.get("type"), Some(Value::Str(t)) if t == "element")
                && matches!(o.get("partial"), Some(Value::Bool(_)))
                && matches!(
                    o.get("props"),
                    Some(Value::Obj(_) | Value::Arr(_) | Value::Element(_) | Value::Ast(_))
                );
            match (is_element, o.get("typeName")) {
                (true, Some(Value::Str(name))) => {
                    let props = match o.get("props") {
                        Some(Value::Obj(p)) => Some(&**p),
                        _ => None,
                    };
                    Some((name, props))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The reference's edge rule for an invalid value: use the schema default if
/// there is one; otherwise a required key makes its container invalid (true),
/// and an optional one is deleted.
pub(crate) fn resolve_invalid_value(
    container: &mut JsObject<Value>,
    key: &str,
    required: bool,
    default: Option<&Json>,
) -> bool {
    if let Some(default) = default {
        container.set(key, Value::from_json(default));
        return false;
    }
    if required {
        return true;
    }
    container.remove(key);
    false
}

fn schema_default(schema: Option<&Json>) -> Option<&Json> {
    schema?.as_object()?.get("default")
}

/// Validate `value` against a JSON Schema fragment, reporting every problem
/// and pruning what can't be salvaged. True when the value itself is invalid
/// and its parent must resolve it.
pub(crate) fn validate_schema_value(
    value: &mut Value,
    schema: &Json,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    crate::stack::grow_stack!(validate_schema_value(value, schema, component, path, ctx));
    let Some(s) = schema.as_object() else {
        return false;
    };
    if matches!(value, Value::Null) || is_ast_node(value) {
        return false; // absence is the parent's concern; runtime values resolve later
    }
    if let Some((type_name, _)) = element_parts(value) {
        let type_name = type_name.to_owned();
        return validate_element_position(&type_name, s, component, path, ctx);
    }
    if is_composite(s) {
        // Composite shapes can't be matched to one plain value, but data in a
        // slot that takes only components is a mismatch.
        let Some(expected) = component_slot(s, ctx) else {
            return false;
        };
        let actual = match value {
            Value::Obj(_) => "plain object",
            other => js_type(other),
        };
        let issue = Issue::TypeMismatch {
            expected,
            actual: actual.to_owned(),
        };
        ctx.push_issue(component, path, issue);
        return true;
    }
    match s.get("type").and_then(Json::as_str) {
        Some("object") => validate_object(value, s, component, path, ctx),
        Some("array") => validate_array(value, s, component, path, ctx),
        _ => validate_leaf(value, s, component, path, ctx),
    }
}

/// `isCompositeSchema`: a `$ref` or a union, which no one plain value matches.
fn is_composite(s: &serde_json::Map<String, Json>) -> bool {
    ["$ref", "anyOf", "oneOf", "allOf"]
        .iter()
        .any(|k| s.contains_key(*k))
}

/// For a slot that takes only components (`isOnlyComponentSlot`: every option
/// of its `$ref`, `anyOf` or `oneOf` is a `$ref` to a component), the
/// components it takes, as the mismatch message lists them: `Card | Text`.
/// `None` for any other slot. Options that aren't an array, or an option
/// that's `null`, make the reference throw; such a slot takes data here.
fn component_slot(s: &serde_json::Map<String, Json>, ctx: &Ctx) -> Option<String> {
    // `s.anyOf ?? s.oneOf ?? [s]`
    let union = s
        .get("anyOf")
        .filter(|u| !u.is_null())
        .or_else(|| s.get("oneOf").filter(|u| !u.is_null()));
    let options: Vec<&serde_json::Map<String, Json>> = match union {
        None => vec![s],
        Some(Json::Array(options)) => options.iter().map(Json::as_object).collect::<Option<_>>()?,
        Some(_) => return None,
    };
    // `refName`: the last segment of a string `$ref`.
    let names: Vec<&str> = options
        .iter()
        .map(|o| o.get("$ref")?.as_str()?.rsplit('/').next())
        .collect::<Option<_>>()?;
    let library = ctx.library;
    let only_components = !names.is_empty() && names.iter().all(|n| library.component(n).is_some());
    only_components.then(|| names.join(" | "))
}

/// A component in a data slot: fine where components may go, a mismatch where
/// the slot wants plain data.
fn validate_element_position(
    type_name: &str,
    s: &serde_json::Map<String, Json>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    if is_composite(s) {
        return false;
    }
    let ty = s.get("type").and_then(Json::as_str);
    if ty.is_some() || matches!(s.get("enum"), Some(Json::Array(_))) || s.contains_key("const") {
        let expected = ty.unwrap_or("a literal value").to_owned();
        let actual = format!("component \"{type_name}\"");
        ctx.push_issue(component, path, Issue::TypeMismatch { expected, actual });
        return true;
    }
    false
}

fn validate_object(
    value: &mut Value,
    s: &serde_json::Map<String, Json>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let Value::Obj(obj) = value else {
        let actual = js_type(value).to_owned();
        ctx.push_issue(
            component,
            path,
            Issue::TypeMismatch {
                expected: "object".to_owned(),
                actual,
            },
        );
        return true;
    };
    // Shared with every other place the value is inlined: copy before changing.
    let obj = std::sync::Arc::make_mut(obj);
    let props: Vec<(String, &Json)> = match s.get("properties") {
        Some(Json::Object(m)) => json_entries(m)
            .into_iter()
            .map(|(k, v)| (k.clone(), v))
            .collect(),
        Some(Json::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    };
    let prop = |key: &str| props.iter().find(|(k, _)| k == key).map(|(_, v)| *v);
    let required: Vec<String> = match s.get("required") {
        Some(Json::Array(items)) => items.iter().map(js_string).collect(),
        _ => Vec::new(),
    };
    let is_required = |key: &str| matches!(s.get("required"), Some(Json::Array(items)) if items.iter().any(|r| r.as_str() == Some(key)));

    let mut invalid = false;
    // Keys filled from their default here, which aren't validated after: the
    // reference fills them into a copy, and validates the object as it was.
    let mut filled: Vec<String> = Vec::new();
    // Structural checks are streaming-sensitive: only on complete input.
    if !ctx.partial {
        for key in &required {
            let present = obj.has(key);
            if !present || matches!(obj.get(key), Some(Value::Null)) {
                if let Some(fallback) = schema_default(prop(key)) {
                    obj.set(key.clone(), Value::from_json(fallback));
                    filled.push(key.clone());
                    continue;
                }
                let issue = if present {
                    Issue::NullRequired { signature: None }
                } else {
                    Issue::MissingRequired { signature: None }
                };
                ctx.push_issue(component, &format!("{path}/{key}"), issue);
                invalid = true;
            }
        }
    }
    for (key, sub) in &props {
        if filled.contains(key) {
            continue;
        }
        let child_path = format!("{path}/{key}");
        let child_invalid = match obj.get_mut(key) {
            Some(child) => validate_schema_value(child, sub, component, &child_path, ctx),
            // An inherited name (`toString`, ...) is "in" every object, as a function.
            None if key != "__proto__" && OBJECT_PROTOTYPE_KEYS.contains(&key.as_str()) => {
                validate_inherited_function(sub, component, &child_path, ctx)
            }
            None => continue,
        };
        if child_invalid
            && resolve_invalid_value(obj, key, is_required(key), schema_default(Some(sub)))
        {
            invalid = true;
        }
    }
    invalid
}

/// What validating an inherited method (a function) against `schema` reports.
fn validate_inherited_function(schema: &Json, component: &str, path: &str, ctx: &mut Ctx) -> bool {
    let Some(s) = schema.as_object() else {
        return false;
    };
    if is_composite(s) {
        let Some(expected) = component_slot(s, ctx) else {
            return false;
        };
        let issue = Issue::TypeMismatch {
            expected,
            actual: "function".to_owned(),
        };
        ctx.push_issue(component, path, issue);
        return true;
    }
    let expected = match s.get("type").and_then(Json::as_str) {
        Some(t @ ("object" | "array")) => t,
        _ => match scalar_leaf(s) {
            Leaf::Type(t) => t,
            _ => return false,
        },
    };
    let issue = Issue::TypeMismatch {
        expected: expected.to_owned(),
        actual: "function".to_owned(),
    };
    ctx.push_issue(component, path, issue);
    true
}

/// An array slot: invalid items are pruned (after validating all of them, so
/// error paths keep the original indices). Never invalid itself.
fn validate_array(
    value: &mut Value,
    s: &serde_json::Map<String, Json>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let Value::Arr(items) = value else {
        let actual = js_type(value).to_owned();
        ctx.push_issue(
            component,
            path,
            Issue::TypeMismatch {
                expected: "array".to_owned(),
                actual,
            },
        );
        return true;
    };
    let Some(item_schema) = s.get("items").filter(|i| i.is_object()) else {
        return false;
    };
    // Shared with every other place the value is inlined: copy before changing.
    let items = std::sync::Arc::make_mut(items);
    let mut invalid = Vec::new();
    for (i, item) in items.iter_mut().enumerate() {
        if validate_schema_value(item, item_schema, component, &format!("{path}/{i}"), ctx) {
            invalid.push(i);
        }
    }
    for i in invalid.into_iter().rev() {
        items.remove(i);
    }
    false
}

/// A scalar or enum leaf. Enum membership waits while streaming (a partial
/// literal may still become valid); scalar type checks don't.
fn validate_leaf(
    value: &Value,
    s: &serde_json::Map<String, Json>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let issue = match scalar_leaf(s) {
        Leaf::Unchecked => return false,
        Leaf::Enum(_) if ctx.partial => return false,
        Leaf::Enum(values) => {
            let scalar = matches!(value, Value::Str(_) | Value::Num(_) | Value::Bool(_));
            if !scalar || enum_includes(&values, value) {
                return false;
            }
            let listed: Vec<String> = values.into_iter().map(json_value_string).collect();
            Issue::TypeMismatch {
                expected: format!("one of [{}]", listed.join(", ")),
                actual: scalar_json(value),
            }
        }
        Leaf::Type(expected) => {
            if expected == type_of(value) {
                return false;
            }
            Issue::TypeMismatch {
                expected: expected.to_owned(),
                actual: js_type(value).to_owned(),
            }
        }
    };
    ctx.push_issue(component, path, issue);
    true
}
