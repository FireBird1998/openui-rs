//! The component library, compiled from JSON Schema as the reference parser's
//! `compileSchema` does: its components, and the calls programs may make with
//! `@`, built-in and the library's own.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::Value as Json;

use crate::builtins::{ACTION_SHAPES, builtin_calls};
use crate::js::{object_entries, truthy};

/// One parameter of a component, function or action, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// The prop name.
    pub name: String,
    /// Listed in the component's `required`.
    pub required: bool,
    /// The schema's `default`, used when a required prop is missing or null.
    pub default: Option<Json>,
    /// The raw JSON Schema fragment for this param, with `$ref`s to action
    /// shapes replaced by the shapes; `null` for a built-in's.
    pub schema: Json,
}

/// A component's parameters, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// Params in positional order: the order of the schema's `properties`.
    pub params: Vec<Param>,
}

/// Whether a call computes a value or is an action step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// `@Count(...)`, or a library function from the schema's `functions`.
    Function,
    /// `@ToAssistant(...)`, `Action([...])`, or a library action from the
    /// schema's `actions`.
    Action,
}

/// A function or action step, called with `@` (except `Action`).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// A function or an action step.
    pub kind: CallKind,
    /// Params in positional order: for a library call, the order of its
    /// `params` schema's `properties`.
    pub params: Vec<Param>,
    /// One of the reference's own; their args aren't validated.
    pub builtin: bool,
    /// Takes its args unevaluated (`@Each`, `@Run`, `@Set`, `@Reset`, `Action`).
    pub lazy: bool,
}

/// The components a program may use, and the calls it may make.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    components: IndexMap<String, Component>,
    calls: IndexMap<String, Call>,
}

impl Default for Library {
    /// No components, and the built-in calls.
    fn default() -> Library {
        Library {
            components: IndexMap::new(),
            calls: builtin_calls(),
        }
    }
}

impl Library {
    /// Compile the JSON Schema produced by `library.toJSONSchema()`.
    ///
    /// - **Components:** the `$defs` entries that `properties` names, or
    ///   every `$defs` entry when there's no `properties`, except the action
    ///   shapes. Params come in `properties` order.
    /// - **Action shapes:** `ActionExpression`, `OpenUrl`, `ToAssistant` and
    ///   the library's own actions, when `$defs` has them. A param schema's
    ///   `$ref` to one is replaced by the shape, so an action slot validates
    ///   as its union.
    /// - **Calls:** the built-ins, then `functions`, then `actions`.
    ///
    /// Everything is read with `Object.entries` and `Object.keys`, as the
    /// reference does: object keys in JavaScript's order (array-index keys
    /// like "0" and "12" first), and an array or string by index.
    pub fn from_json_schema(schema: &Json) -> Library {
        let keys = |value: &Json| -> Vec<String> {
            object_entries(value).into_iter().map(|(k, _)| k).collect()
        };
        // `schema.properties && new Set(Object.keys(schema.properties))`
        let components: Option<HashSet<String>> = schema
            .get("properties")
            .filter(|p| truthy(Some(p)))
            .map(|p| keys(p).into_iter().collect());
        let is_component = |name: &str| components.as_ref().is_some_and(|c| c.contains(name));
        let defs = schema.get("$defs");

        let action_names = ACTION_SHAPES
            .iter()
            .map(|&name| name.to_owned())
            .chain(schema.get("actions").map(keys).unwrap_or_default());
        let mut shapes: IndexMap<String, Json> = IndexMap::new();
        for name in action_names {
            if let Some(body) = member(defs, &name).filter(|body| truthy(Some(body)))
                && !is_component(&name)
            {
                shapes.insert(format!("#/$defs/{name}"), body.clone());
            }
        }

        let mut library = Library::default();
        for (name, def) in defs.map(object_entries).unwrap_or_default() {
            let skip = match &components {
                Some(components) => !components.contains(&name),
                None => shapes.contains_key(&format!("#/$defs/{name}")),
            };
            if !skip {
                let params = compile_params(&def, &shapes);
                library.components.insert(name, Component { params });
            }
        }
        for (key, kind) in [
            ("functions", CallKind::Function),
            ("actions", CallKind::Action),
        ] {
            for (name, def) in schema.get(key).map(object_entries).unwrap_or_default() {
                // A call without `params` makes the reference throw; it has none here.
                let params = compile_params(def.get("params").unwrap_or(&Json::Null), &shapes);
                let call = Call {
                    kind,
                    params,
                    builtin: false,
                    lazy: false,
                };
                library.calls.insert(name, call);
            }
        }
        library
    }

    /// A component by name.
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.get(name)
    }

    /// Component names in library order.
    pub fn component_names(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_str)
    }

    /// A built-in or library function or action by name.
    pub fn call(&self, name: &str) -> Option<&Call> {
        self.calls.get(name)
    }

    /// Every call, built-ins first, then the library's functions and actions.
    pub fn calls(&self) -> impl Iterator<Item = (&str, &Call)> {
        self.calls.iter().map(|(name, call)| (name.as_str(), call))
    }
}

/// `compileParams`: params in `properties` order, each with whether it's
/// required, its default, and its schema with action shapes inlined.
fn compile_params(def: &Json, shapes: &IndexMap<String, Json>) -> Vec<Param> {
    let required = def.get("required");
    // `required.includes(key)`: array membership, or a substring of a string.
    let is_required = |key: &str| match required {
        Some(Json::Array(items)) => items.iter().any(|r| r.as_str() == Some(key)),
        Some(Json::String(s)) => s.contains(key),
        _ => false,
    };
    def.get("properties")
        .map(object_entries)
        .unwrap_or_default()
        .into_iter()
        .map(|(key, prop)| Param {
            required: is_required(&key),
            default: prop.as_object().and_then(|p| p.get("default")).cloned(),
            schema: if shapes.is_empty() {
                prop
            } else {
                inline_action_shapes(&prop, shapes, &mut Vec::new())
            },
            name: key,
        })
        .collect()
}

/// `inlineActionRefs`: a `$ref` to an action shape becomes the shape, itself
/// inlined; any other value is copied with its parts inlined. A shape that
/// refers back to itself is left as the `$ref` (the reference recurses until
/// its stack overflows).
fn inline_action_shapes(
    node: &Json,
    shapes: &IndexMap<String, Json>,
    inlining: &mut Vec<String>,
) -> Json {
    crate::stack::grow_stack!(inline_action_shapes(node, shapes, inlining));
    match node {
        Json::Object(map) => {
            if let Some(Json::String(target)) = map.get("$ref") {
                let Some(shape) = shapes.get(target) else {
                    return node.clone();
                };
                if inlining.contains(target) {
                    return node.clone();
                }
                inlining.push(target.clone());
                let inlined = inline_action_shapes(shape, shapes, inlining);
                inlining.pop();
                return inlined;
            }
            Json::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), inline_action_shapes(v, shapes, inlining)))
                    .collect(),
            )
        }
        Json::Array(items) => Json::Array(
            items
                .iter()
                .map(|v| inline_action_shapes(v, shapes, inlining))
                .collect(),
        ),
        _ => node.clone(),
    }
}

/// `$defs[name]` the way JavaScript reads it from parsed JSON: an own key of
/// an object, or an index of an array.
fn member<'a>(value: Option<&'a Json>, name: &str) -> Option<&'a Json> {
    match value? {
        Json::Object(map) => map.get(name),
        Json::Array(items) => {
            let i: usize = name.parse().ok()?;
            (i.to_string() == name).then(|| items.get(i)).flatten()
        }
        _ => None,
    }
}
