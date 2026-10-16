//! The built-in functions and action steps, and the reserved calls: names
//! the reference parser treats as calls rather than components.

use indexmap::IndexMap;
use serde_json::Value as Json;

use crate::library::{Call, CallKind, Param};

/// The built-ins, in the order of the reference's call registry
/// (`BUILTIN_CALLS`): name, kind, param names, and whether they take their
/// args unevaluated. Built-ins aren't validated, so their params are names.
const BUILTINS: [(&str, CallKind, &[&str], bool); 20] = [
    ("Count", CallKind::Function, &["array"], false),
    ("First", CallKind::Function, &["array"], false),
    ("Last", CallKind::Function, &["array"], false),
    ("Sum", CallKind::Function, &["numbers"], false),
    ("Avg", CallKind::Function, &["numbers"], false),
    ("Min", CallKind::Function, &["numbers"], false),
    ("Max", CallKind::Function, &["numbers"], false),
    (
        "Sort",
        CallKind::Function,
        &["array", "field", "direction"],
        false,
    ),
    (
        "Filter",
        CallKind::Function,
        &["array", "field", "operator", "value"],
        false,
    ),
    ("Round", CallKind::Function, &["number", "decimals"], false),
    ("Abs", CallKind::Function, &["number"], false),
    ("Floor", CallKind::Function, &["number"], false),
    ("Ceil", CallKind::Function, &["number"], false),
    (
        "Each",
        CallKind::Function,
        &["array", "varName", "template"],
        true,
    ),
    ("Run", CallKind::Action, &["ref"], true),
    (
        "ToAssistant",
        CallKind::Action,
        &["message", "context"],
        false,
    ),
    ("OpenUrl", CallKind::Action, &["url"], false),
    ("Set", CallKind::Action, &["variable", "value"], true),
    ("Reset", CallKind::Action, &["variable"], true),
    ("Action", CallKind::Action, &["steps"], true),
];

/// The action shapes every library's JSON Schema may hold in `$defs`: data,
/// never components (`ACTION_DEFS`).
pub(crate) const ACTION_SHAPES: [&str; 3] = ["ActionExpression", "OpenUrl", "ToAssistant"];

/// The built-ins as calls, the base of every library's call registry.
pub(crate) fn builtin_calls() -> IndexMap<String, Call> {
    BUILTINS
        .iter()
        .map(|&(name, kind, params, lazy)| {
            let params = params
                .iter()
                .map(|&param| Param {
                    name: param.to_owned(),
                    // Only `Action`'s one param is required.
                    required: name == "Action",
                    default: None,
                    schema: Json::Null,
                })
                .collect();
            let call = Call {
                kind,
                params,
                builtin: true,
                lazy,
            };
            (name.to_owned(), call)
        })
        .collect()
}

/// Whether `name` is a builtin function or action, not a component.
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|&(builtin, ..)| builtin == name)
}

/// Statement-level calls that are neither builtins nor components. The
/// reference checks `name in { Query, Mutation }`, so names every object
/// inherits (`toString`, `constructor`, ...) count too.
pub fn is_reserved_call(name: &str) -> bool {
    matches!(name, "Query" | "Mutation") || crate::js::OBJECT_PROTOTYPE_KEYS.contains(&name)
}

/// Builtins whose arguments are kept as expressions with a scoped iterator
/// variable (`@Each(items, "item", template)`).
pub(crate) fn is_lazy_builtin(name: &str) -> bool {
    name == "Each"
}
