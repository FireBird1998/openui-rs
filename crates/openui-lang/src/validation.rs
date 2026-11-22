//! Prop validation against the component library's JSON Schema, as the
//! reference parser's `validateSchemaValue` and `resolveInvalidValue`, with
//! every message worded exactly as the reference words it.

use std::borrow::Cow;

use serde_json::Value as Json;

use crate::js::{
    JsObject, OBJECT_PROTOTYPE_KEYS, js_string, json_entries, json_string, json_value_string,
    write_json_number,
};
use crate::library::{Library, Param, SchemaNode};
use crate::materialize::{Ctx, js_strict_eq};
use crate::result::{Severity, ValidationErrorCode, Value};

mod prepared;
pub use prepared::PreparedValidationSchema;

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

/// The `query-cycle` message around its queries' names.
const CYCLE_OPENING: &str = "Queries ";
const CYCLE_SEPARATOR: &str = ", ";
const CYCLE_CLOSING: &str =
    " read each other's results, so none of them can run first. They do not run";

/// How long the `query-cycle` message is for `count` queries whose names
/// come to `bytes` in all, without building it: whether it fits in the
/// result is known before the names are gathered.
pub(crate) fn query_cycle_message_len(count: usize, bytes: usize) -> usize {
    CYCLE_OPENING.len()
        + bytes
        + CYCLE_SEPARATOR.len() * count.saturating_sub(1)
        + CYCLE_CLOSING.len()
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
            Some(s) => format!(" — signature: {s}"),
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
                "{CYCLE_OPENING}{}{CYCLE_CLOSING}",
                cycle.join(CYCLE_SEPARATOR)
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
/// marks required params, which are `library`'s.
pub(crate) fn params_signature(library: &Library, component: &str, params: &[Param]) -> String {
    let rendered: Vec<String> = params
        .iter()
        .map(|p| {
            let marked = if p.required {
                format!("{}*", p.name)
            } else {
                p.name.clone()
            };
            match type_from_schema(&library.schema_root(&p.schema)) {
                Some(ty) if !ty.is_empty() => format!("{marked}: {ty}"),
                _ => marked,
            }
        })
        .collect();
    format!("{component}({})", rendered.join(", "))
}

/// A schema leaf's allowed literal values, or its scalar type.
enum Leaf<'a> {
    Enum(Vec<Cow<'a, Json>>),
    Type(&'static str),
    Unchecked,
}

/// The reference's `getScalarTypeInfo`: `enum`, then `const`, then `type`.
fn scalar_leaf<'a>(s: &SchemaNode<'a>) -> Leaf<'a> {
    if let Some(values) = s.get("enum")
        && let Json::Array(items) = values.json
    {
        return Leaf::Enum(items.iter().map(|v| values.child(v).data()).collect());
    }
    if let Some(value) = s.get("const") {
        return Leaf::Enum(vec![value.data()]);
    }
    match s.get("type").and_then(|t| t.as_str()) {
        Some("string") => Leaf::Type("string"),
        Some("number" | "integer") => Leaf::Type("number"),
        Some("boolean") => Leaf::Type("boolean"),
        _ => Leaf::Unchecked,
    }
}

/// The type hint for a param's schema, as the reference `getTypeFromSchema`.
fn type_from_schema(schema: &SchemaNode<'_>) -> Option<String> {
    schema.object()?;
    if let Some(r) = schema.get("$ref").and_then(|r| r.as_str())
        && let Some(name) = r.rsplit('/').next().filter(|name| !name.is_empty())
    {
        return Some(name.to_owned());
    }
    match scalar_leaf(schema) {
        Leaf::Enum(values) => Some(
            values
                .iter()
                .map(|v| json_value_string(v))
                .collect::<Vec<_>>()
                .join("|"),
        ),
        Leaf::Type(t) => Some(t.to_owned()),
        Leaf::Unchecked => schema
            .get("type")
            .and_then(|t| t.as_str())
            .map(str::to_owned),
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

/// `enumValues.includes(value)` uses SameValueZero, not `===`. Schema JSON
/// numbers are finite, so numeric equality suffices: a parsed NaN never
/// matches, while +0 and -0 do.
fn enum_includes(values: &[Cow<'_, Json>], value: &Value) -> bool {
    values.iter().any(|v| match (v.as_ref(), value) {
        (Json::String(a), Value::Str(b)) => a == b,
        (Json::Number(a), Value::Num(b)) => crate::js::json_number(a) == *b,
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

/// What an invalid value becomes: `resolveInvalidValue`.
pub(crate) enum Fallback<'a> {
    /// The schema's default.
    Default(&'a Json),
    /// None, and the key is required: its container is invalid.
    Invalid,
    /// None, and the key is optional: it's deleted.
    Remove,
}

/// The reference's edge rule for an invalid value: use the schema default if
/// there is one; otherwise a required key makes its container invalid, and an
/// optional one is deleted.
pub(crate) fn invalid_fallback(required: bool, default: Option<&Json>) -> Fallback<'_> {
    match default {
        Some(default) => Fallback::Default(default),
        None if required => Fallback::Invalid,
        None => Fallback::Remove,
    }
}

/// [`invalid_fallback`] applied to `container[key]`: true when the container
/// is invalid.
pub(crate) fn resolve_invalid_value(
    container: &mut JsObject<Value>,
    key: &str,
    required: bool,
    default: Option<&Json>,
    ctx: &mut Ctx,
) -> bool {
    match invalid_fallback(required, default) {
        Fallback::Default(default) => {
            ctx.default_used(default, 1);
            container.set(key, Value::from_json(default));
            false
        }
        Fallback::Invalid => true,
        Fallback::Remove => {
            container.remove(key);
            false
        }
    }
}

/// The object's copy, made on its first change (the reference's `out ??=
/// { ...obj }`), so an object nothing changes keeps its identity.
fn copied<'a>(
    out: &'a mut Option<JsObject<Value>>,
    obj: &JsObject<Value>,
) -> &'a mut JsObject<Value> {
    out.get_or_insert_with(|| {
        // Object.prototype's own methods/accessor are non-enumerable.
        if is_inert_prototype(obj) {
            JsObject::new()
        } else {
            obj.spread()
        }
    })
}

static INERT_PROTOTYPE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
fn inert_prototype() -> &'static Value {
    INERT_PROTOTYPE.get_or_init(|| Value::Obj(std::sync::Arc::new(JsObject::new())))
}
fn is_inert_prototype(obj: &JsObject<Value>) -> bool {
    matches!(INERT_PROTOTYPE.get(), Some(Value::Obj(prototype)) if std::ptr::eq(obj, prototype.as_ref()))
}
fn object_property<'a, const BOUNDED: bool>(
    obj: &'a JsObject<Value>,
    key: &str,
) -> Option<&'a Value> {
    obj.get(key).or_else(|| {
        if key != "__proto__" || !obj.has(key) || (!BOUNDED && obj.has_unrepresented_prototype()) {
            return None;
        }
        // Object.prototype itself has a null prototype. These are inert Rust
        // values, not host objects, and never invoke an accessor or setter.
        static NULL: Value = Value::Null;
        Some(if is_inert_prototype(obj) {
            &NULL
        } else {
            inert_prototype()
        })
    })
}

/// A schema's `default`, as the expanded copy has it.
fn schema_default<'a>(schema: Option<&SchemaNode<'a>>) -> Option<Cow<'a, Json>> {
    let schema = schema?;
    schema.object()?;
    Some(schema.get("default")?.data())
}

/// Validate `value` against a JSON Schema fragment, reporting every problem
/// and pruning what can't be salvaged. True when the value itself is invalid
/// and its parent must resolve it.
///
/// `schema` is a param's schema of `ctx.library`, read as the reference's
/// expanded copy, with the library's action shapes followed where it refers
/// to them.
#[inline(always)]
pub(crate) fn validate_schema_value(
    value: &mut Value,
    schema: &Json,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let library = ctx.library;
    let root = library.schema_root(schema);
    validate_schema_value_in::<false>(value, &root, component, path, ctx, None)
}
pub(crate) fn validate_bounded_value(
    value: &mut Value,
    prepared: &PreparedValidationSchema<'_>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let root = SchemaNode::verbatim(prepared.root);
    validate_schema_value_in::<true>(value, &root, component, path, ctx, Some(prepared))
}
fn validate_schema_value_in<'s, const BOUNDED: bool>(
    value: &mut Value,
    schema: &SchemaNode<'s>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
    prepared: Option<&PreparedValidationSchema<'s>>,
) -> bool {
    let validate_schema_value = validate_schema_value_in::<BOUNDED>;
    crate::stack::grow_stack!(validate_schema_value(value, schema, component, path, ctx, prepared), budget else { *value = Value::Null; ctx.stack_stalled(); false });
    if BOUNDED && !crate::evaluate::validation_node(value, schema.json, path) {
        ctx.stack_cut = true;
        return false;
    }
    let Some(s) = schema.object() else {
        return false;
    };
    if matches!(value, Value::Null) || is_ast_node(value) {
        return false; // absence is the parent's concern; runtime values resolve later
    }
    if let Some((type_name, _)) = element_parts(value) {
        let type_name = type_name.to_owned();
        return validate_element_position(&type_name, schema, component, path, ctx);
    }
    if is_composite(s) {
        // Composite shapes can't be matched to one plain value, but data in a
        // slot that takes only components is a mismatch.
        let Some(expected) = component_slot(schema, ctx) else {
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
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => validate_object::<BOUNDED>(value, schema, component, path, ctx, prepared),
        Some("array") => validate_array::<BOUNDED>(value, schema, component, path, ctx, prepared),
        _ => validate_leaf::<BOUNDED>(value, schema, component, path, ctx),
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
fn component_slot(s: &SchemaNode<'_>, ctx: &Ctx) -> Option<String> {
    // `s.anyOf ?? s.oneOf ?? [s]`
    let union = s
        .get("anyOf")
        .filter(|u| !u.json.is_null())
        .or_else(|| s.get("oneOf").filter(|u| !u.json.is_null()));
    let options: Vec<SchemaNode<'_>> = match union {
        None => vec![s.clone()],
        Some(union) => match union.json {
            Json::Array(options) => options
                .iter()
                .map(|o| {
                    let option = union.child(o);
                    option.object().map(|_| option)
                })
                .collect::<Option<_>>()?,
            _ => return None,
        },
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
    schema: &SchemaNode<'_>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let Some(s) = schema.object() else {
        return false;
    };
    if is_composite(s) {
        return false;
    }
    let ty = schema.get("type").and_then(|t| t.as_str());
    if ty.is_some()
        || matches!(schema.get("enum").map(|e| e.json), Some(Json::Array(_)))
        || s.contains_key("const")
    {
        let expected = ty.unwrap_or("a literal value").to_owned();
        let actual = format!("component \"{type_name}\"");
        ctx.push_issue(component, path, Issue::TypeMismatch { expected, actual });
        return true;
    }
    false
}

fn validate_object<'s, const BOUNDED: bool>(
    value: &mut Value,
    schema: &SchemaNode<'s>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
    prepared: Option<&PreparedValidationSchema<'s>>,
) -> bool {
    let Some(s) = schema.object() else {
        return false;
    };
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
    // Match the reference's lazy copy: unchanged containers keep identity.
    let mut out: Option<JsObject<Value>> = None;
    let cached = if BOUNDED {
        prepared.and_then(|p| p.object(s))
    } else {
        None
    };
    // The `properties` and `required` nodes, as the expanded copy has them;
    // a property's schema is read through `properties` when it's used. The
    // cached indexes (bounded validation only) read the schema as written.
    let properties = schema.get("properties").filter(|_| cached.is_none());
    let property = |raw: &'s Json| match &properties {
        Some(properties) => properties.child(raw),
        None => SchemaNode::verbatim(raw),
    };
    let owned_props: Vec<(String, &Json)> = match properties.as_ref().map(|p| p.json) {
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
    let props = cached.map_or(owned_props.as_slice(), |c| c.props.as_slice());
    let prop = |key: &str| {
        if let Some(c) = cached {
            c.prop(key).map(SchemaNode::verbatim)
        } else {
            props
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| property(v))
        }
    };
    let required_node = schema.get("required").filter(|_| cached.is_none());
    let required_items: &[Json] = match required_node.as_ref().map(|r| r.json) {
        Some(Json::Array(items)) => items,
        _ => &[],
    };
    let required_item = |raw: &'s Json| match &required_node {
        Some(required) => required.child(raw),
        None => SchemaNode::verbatim(raw),
    };
    let owned_required: Vec<String> = required_items
        .iter()
        .map(|r| {
            let item = required_item(r);
            match item.json {
                // `String(object)`, whatever the object holds.
                Json::Object(_) => "[object Object]".to_owned(),
                _ => js_string(&item.data()),
            }
        })
        .collect();
    let required = cached.map_or(owned_required.as_slice(), |c| c.required.as_slice());
    let is_required = |key: &str| {
        if let Some(c) = cached {
            c.is_required(key)
        } else {
            required_items
                .iter()
                .any(|r| required_item(r).as_str() == Some(key))
        }
    };

    let mut invalid = false;
    // Keys filled from their default here, which aren't validated after: the
    // reference fills them into a copy, and validates the object as it was.
    let mut filled: Vec<String> = Vec::new();
    // Structural checks are streaming-sensitive: only on complete input.
    if !ctx.partial {
        for key in required {
            if BOUNDED && !crate::evaluate::validation_work(key.len().saturating_add(1)) {
                ctx.stack_cut = true;
                break;
            }
            if ctx.stack_cut {
                break;
            }
            let present = obj.has(key);
            if !present || matches!(object_property::<BOUNDED>(obj, key), Some(Value::Null)) {
                if let Some(fallback) = schema_default(prop(key).as_ref()) {
                    if BOUNDED && !crate::evaluate::validation_default(&fallback) {
                        ctx.stack_cut = true;
                        break;
                    }
                    copied(&mut out, obj).set(key.clone(), Value::from_json(&fallback));
                    filled.push(key.clone());
                    continue;
                }
                // Not the program's mistake where a limit cut the value.
                if !ctx.cut_inside {
                    let issue = if present {
                        Issue::NullRequired { signature: None }
                    } else {
                        Issue::MissingRequired { signature: None }
                    };
                    ctx.push_issue(component, &format!("{path}/{key}"), issue);
                }
                invalid = true;
            }
        }
    }
    let order = if let Some(c) = cached {
        match c.order(obj) {
            Some(o) => o,
            None => {
                ctx.stack_cut = true;
                return invalid;
            }
        }
    } else {
        Vec::new()
    };
    let count = if cached.is_some() {
        order.len()
    } else {
        props.len()
    };
    let mut filled_index = std::collections::BTreeSet::new();
    if BOUNDED {
        for key in &filled {
            filled_index.insert(key.as_str());
        }
    }
    for offset in 0..count {
        let index = if cached.is_some() {
            order.get(offset).copied()
        } else {
            Some(offset)
        };
        let Some((key, sub)) = index.and_then(|i| props.get(i)) else {
            break;
        };
        let sub = &property(sub);
        if ctx.stack_cut {
            break;
        }
        if if BOUNDED {
            filled_index.contains(key.as_str())
        } else {
            filled.contains(key)
        } {
            continue;
        }
        let child_path = format!("{path}/{key}");
        let Some(before) = object_property::<BOUNDED>(obj, key) else {
            // Inherited methods are functions, not representable as Value.
            if obj.has(key)
                && key != "__proto__"
                && OBJECT_PROTOTYPE_KEYS.contains(&key.as_str())
                && validate_inherited_function::<BOUNDED>(sub, component, &child_path, ctx)
                && (!BOUNDED
                    || schema_default(Some(sub)).is_none_or(|d| {
                        let ok = crate::evaluate::validation_default(&d);
                        if !ok {
                            ctx.stack_cut = true;
                        }
                        ok
                    }))
                && resolve_invalid_value(
                    copied(&mut out, obj),
                    key,
                    is_required(key),
                    schema_default(Some(sub)).as_deref(),
                    ctx,
                )
            {
                invalid = true;
            }
            continue;
        };
        let mut next = before.clone();
        if validate_schema_value_in::<BOUNDED>(
            &mut next,
            sub,
            component,
            &child_path,
            ctx,
            prepared,
        ) {
            let default = schema_default(Some(sub));
            match invalid_fallback(is_required(key), default.as_deref()) {
                Fallback::Default(default) => {
                    if BOUNDED && !crate::evaluate::validation_default(default) {
                        ctx.stack_cut = true;
                        break;
                    }
                    ctx.default_used(default, 1);
                    next = Value::from_json(default);
                }
                Fallback::Invalid => {
                    invalid = true;
                    continue;
                }
                Fallback::Remove => {
                    copied(&mut out, obj).remove(key);
                    continue;
                }
            }
        }
        // Reference: `next === obj[key]`. In particular NaN triggers a copy.
        if !js_strict_eq(&next, before) {
            copied(&mut out, obj).set(key.clone(), next);
        }
    }
    if let Some(out) = out {
        *obj = std::sync::Arc::new(out);
    }
    invalid
}

/// What validating an inherited method (a function) against `schema` reports.
fn validate_inherited_function<const BOUNDED: bool>(
    schema: &SchemaNode<'_>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    if BOUNDED && !crate::evaluate::validation_node(&Value::Bool(false), schema.json, path) {
        ctx.stack_cut = true;
        return false;
    }
    let Some(s) = schema.object() else {
        return false;
    };
    if is_composite(s) {
        let Some(expected) = component_slot(schema, ctx) else {
            return false;
        };
        let issue = Issue::TypeMismatch {
            expected,
            actual: "function".to_owned(),
        };
        ctx.push_issue(component, path, issue);
        return true;
    }
    let expected = match schema.get("type").and_then(|t| t.as_str()) {
        Some(t @ ("object" | "array")) => t,
        _ => match scalar_leaf(schema) {
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
fn validate_array<'s, const BOUNDED: bool>(
    value: &mut Value,
    schema: &SchemaNode<'s>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
    prepared: Option<&PreparedValidationSchema<'s>>,
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
    let Some(item_schema) = schema.get("items").filter(|i| i.json.is_object()) else {
        return false;
    };
    let mut out = None;
    for (i, before) in items.iter().enumerate() {
        if ctx.stack_cut {
            break;
        }
        let mut next = before.clone();
        let invalid = validate_schema_value_in::<BOUNDED>(
            &mut next,
            &item_schema,
            component,
            &format!("{path}/{i}"),
            ctx,
            prepared,
        );
        // Reference: `next !== value[i]`; NaN counts as changed even if kept.
        if invalid || !js_strict_eq(&next, before) {
            out.get_or_insert_with(|| items[..i].to_vec());
        }
        if let Some(out) = &mut out
            && !invalid
        {
            out.push(next);
        }
    }
    if let Some(out) = out {
        *items = std::sync::Arc::new(out);
    }
    false
}

/// A scalar or enum leaf. Enum membership waits while streaming (a partial
/// literal may still become valid); scalar type checks don't.
fn validate_leaf<const BOUNDED: bool>(
    value: &Value,
    schema: &SchemaNode<'_>,
    component: &str,
    path: &str,
    ctx: &mut Ctx,
) -> bool {
    let issue = match scalar_leaf(schema) {
        Leaf::Unchecked => return false,
        Leaf::Enum(_) if ctx.partial => return false,
        Leaf::Enum(values) => {
            let scalar = matches!(value, Value::Str(_) | Value::Num(_) | Value::Bool(_));
            if !scalar || enum_includes(&values, value) {
                return false;
            }
            if BOUNDED {
                for value in &values {
                    if !crate::evaluate::validation_default(value) {
                        ctx.stack_cut = true;
                        return false;
                    }
                }
            }
            let listed: Vec<String> = values.iter().map(|v| json_value_string(v)).collect();
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
