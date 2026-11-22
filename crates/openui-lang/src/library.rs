//! The component library, compiled from JSON Schema as the reference parser's
//! `compileSchema` does: its components, and the calls programs may make with
//! `@`, built-in and the library's own.
//!
//! # Action shapes are shared, not copied
//!
//! The reference copies each action shape into every param schema that
//! refers to it (`inlineActionRefs`), so a param's schema is that expanded
//! copy. A chain of actions each referring to the next makes those copies
//! quadratic, an action naming the next one twice doubles them with each
//! action, and many params referring to one large action multiply it.
//!
//! The compiled library keeps each param's schema as the library wrote it
//! and each action shape once, shared by every param that refers to it.
//! Validation reads a param's schema through [`SchemaNode`], which follows a
//! `$ref` to an action shape where the expanded copy would have the shape,
//! so it sees exactly the expanded copy, node for node, without anyone
//! making it. Compiling therefore costs time and memory in step with the
//! schema's size, and nothing in the library is deeper than the schema.
//!
//! The expanded copy stops at a cycle: inside a shape's copy, a `$ref` to a
//! shape already being copied on the way there is kept as the `$ref`. The
//! reader keeps that same path, the shapes entered from the param's root to
//! the node, and treats a `$ref` to one of them as the plain `$ref` too. A
//! path holds each shape at most once, so following references always ends,
//! and the path is a list from the node back to the root that nothing in the
//! library holds, so sharing makes no reference cycle.
//!
//! The library frees and clones its JSON with loops (ADR 0002), so dropping
//! or cloning it needs no stack however deep its schemas nest.

use std::borrow::Cow;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};

use crate::builtins::{ACTION_SHAPES, builtin_calls};
use crate::js::{json_entries, object_entries, truthy};

/// One parameter of a component, function or action, in positional order.
#[derive(Debug, PartialEq)]
pub struct Param {
    /// The prop name.
    pub name: String,
    /// Listed in the component's `required`.
    pub required: bool,
    /// The schema's `default`, used when a required prop is missing or null.
    pub default: Option<Json>,
    /// The raw JSON Schema fragment for this param, as the library wrote it;
    /// `null` for a built-in's. A `$ref` in it to one of the library's
    /// action shapes stands for that shape, which the library holds once:
    /// validation follows it where the reference parser would have copied
    /// the shape in (see the module docs).
    pub schema: Json,
}

impl Clone for Param {
    /// Copies the schema and default with a loop, whatever their depth.
    fn clone(&self) -> Param {
        Param {
            name: self.name.clone(),
            required: self.required,
            default: self.default.as_ref().map(clone_json),
            schema: clone_json(&self.schema),
        }
    }
}

impl Drop for Param {
    /// Frees the schema and default with a loop, whatever their depth.
    fn drop(&mut self) {
        free(std::mem::take(&mut self.schema));
        if let Some(default) = self.default.take() {
            free(default);
        }
    }
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
    /// The action shapes params refer to, each held once.
    shapes: Arc<ActionShapes>,
}

impl Default for Library {
    /// No components, and the built-in calls.
    fn default() -> Library {
        Library {
            components: IndexMap::new(),
            calls: builtin_calls(),
            shapes: Arc::default(),
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
    ///   `$ref` to one stands for the shape, so an action slot validates as
    ///   its union. Each shape is held once and shared by every param that
    ///   refers to it, where the reference copies it into each (see the
    ///   module docs): compiling costs time and memory in step with the
    ///   schema's size.
    /// - **Calls:** the built-ins, then `functions`, then `actions`.
    ///
    /// Everything is read with `Object.entries` and `Object.keys`, as the
    /// reference does: object keys in JavaScript's order (array-index keys
    /// like "0" and "12" first), and an array or string by index.
    ///
    /// Compiling copies the parts it keeps with a loop, so it needs no stack
    /// however deep the schema nests.
    pub fn from_json_schema(schema: &Json) -> Library {
        let keys =
            |value: &Json| -> Vec<String> { entries(value).into_iter().map(|(k, _)| k).collect() };
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
                // A name met twice names the same body.
                shapes
                    .entry(format!("#/$defs/{name}"))
                    .or_insert_with(|| clone_json(body));
            }
        }

        let mut library = Library::default();
        for (name, def) in defs.map(entries).unwrap_or_default() {
            let skip = match &components {
                Some(components) => !components.contains(&name),
                None => shapes.contains_key(&format!("#/$defs/{name}")),
            };
            if !skip {
                let params = compile_params(&def);
                library.components.insert(name, Component { params });
            }
        }
        for (key, kind) in [
            ("functions", CallKind::Function),
            ("actions", CallKind::Action),
        ] {
            for (name, def) in schema.get(key).map(entries).unwrap_or_default() {
                // A call without `params` makes the reference throw; it has none here.
                let params = compile_params(def.get("params").unwrap_or(&Json::Null));
                let call = Call {
                    kind,
                    params,
                    builtin: false,
                    lazy: false,
                };
                library.calls.insert(name, call);
            }
        }
        library.shapes = Arc::new(ActionShapes { bodies: shapes });
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

    /// A param's schema, `schema` (one of this library's [`Param::schema`]),
    /// as the reference's expanded copy has it: its root, with the action
    /// shapes it refers to followed (see the module docs).
    pub(crate) fn schema_root<'a>(&'a self, schema: &'a Json) -> SchemaNode<'a> {
        if self.shapes.bodies.is_empty() {
            return SchemaNode::verbatim(schema);
        }
        Scope::Inlining {
            shapes: &self.shapes,
            entered: None,
        }
        .at(schema)
    }
}

/// The action shapes a library's params refer to: each one's `$ref` target
/// (`#/$defs/<name>`) and body, in the order the reference collects them.
#[derive(Debug, Default, PartialEq)]
struct ActionShapes {
    bodies: IndexMap<String, Json>,
}

impl Drop for ActionShapes {
    /// Frees each body with a loop, whatever its depth.
    fn drop(&mut self) {
        for (_, body) in std::mem::take(&mut self.bodies) {
            free(body);
        }
    }
}

/// A node of a param's schema as the reference's expanded copy has it
/// (`inlineActionRefs`), read in place: `json` is the node, and where the
/// copy has an action shape in place of a `$ref`, `json` is the shape.
///
/// Reading a child with [`SchemaNode::get`] or [`SchemaNode::child`] follows
/// the child's `$ref` the way the copy does, so every read sees what the
/// copy holds there. [`SchemaNode::data`] gives a value read as data (a
/// default, an enum member) as the copy has it.
#[derive(Clone)]
pub(crate) struct SchemaNode<'a> {
    /// The node: the param's own JSON, or a shape's.
    pub(crate) json: &'a Json,
    scope: Scope<'a>,
}

/// How a node's children are read.
#[derive(Clone)]
enum Scope<'a> {
    /// As they are: the expanded copy has this node as written. A library
    /// with no action shapes, a `$ref` the copy keeps (to something that is
    /// not an action shape, or to a shape already entered on the way here),
    /// and everything below such a `$ref`.
    Verbatim,
    /// With a `$ref` to an action shape followed, unless the shape is one
    /// already entered on the way from the param's root.
    Inlining {
        shapes: &'a ActionShapes,
        entered: Option<Rc<Entered>>,
    },
}

/// The shapes entered on the way from a param's root to a node, innermost
/// first: the reference's `inlining` stack at that node.
struct Entered {
    /// The shape's index in [`ActionShapes::bodies`].
    shape: usize,
    /// One bit per shape index modulo 128, for this shape and every outer
    /// one: a clear bit means the shape is not on the path.
    mask: u128,
    outer: Option<Rc<Entered>>,
}

impl Drop for Entered {
    /// A path can be as long as the library has shapes: let go of it with a
    /// loop, stopping at the first part someone else still holds.
    fn drop(&mut self) {
        let mut outer = self.outer.take();
        while let Some(part) = outer {
            match Rc::try_unwrap(part) {
                Ok(mut part) => outer = part.outer.take(),
                Err(_) => break,
            }
        }
    }
}

fn mask_bit(shape: usize) -> u128 {
    1u128 << (shape % 128)
}

/// Whether `shape` was entered on the way to a node.
fn on_path(entered: &Option<Rc<Entered>>, shape: usize) -> bool {
    let bit = mask_bit(shape);
    let mut part = entered.as_deref();
    if part.is_none_or(|p| p.mask & bit == 0) {
        return false;
    }
    while let Some(p) = part {
        if p.shape == shape {
            return true;
        }
        part = p.outer.as_deref();
    }
    false
}

impl<'a> Scope<'a> {
    /// The node the expanded copy has for `json`, a child of a node read in
    /// this scope: `json` itself, or, for a `$ref` to an action shape not yet
    /// entered, the shape, its own `$ref` followed in turn.
    fn at(&self, json: &'a Json) -> SchemaNode<'a> {
        let Scope::Inlining { shapes, entered } = self else {
            return SchemaNode::verbatim(json);
        };
        let mut json = json;
        let mut entered = entered.clone();
        // Each turn enters a shape not on the path yet, so there are at most
        // as many turns as shapes, and the last one returns.
        for _ in 0..=shapes.bodies.len() {
            let Some(target) = json
                .as_object()
                .and_then(|map| map.get("$ref"))
                .and_then(Json::as_str)
            else {
                return SchemaNode {
                    json,
                    scope: Scope::Inlining { shapes, entered },
                };
            };
            match shapes.bodies.get_full(target) {
                Some((shape, _, body)) if !on_path(&entered, shape) => {
                    let mask = entered.as_ref().map_or(0, |e| e.mask) | mask_bit(shape);
                    entered = Some(Rc::new(Entered {
                        shape,
                        mask,
                        outer: entered,
                    }));
                    json = body;
                }
                // Kept as written: not an action shape, or one already
                // entered on the way here.
                _ => return SchemaNode::verbatim(json),
            }
        }
        SchemaNode::verbatim(json)
    }
}

impl<'a> SchemaNode<'a> {
    /// `json`, read as written all the way down.
    pub(crate) fn verbatim(json: &'a Json) -> SchemaNode<'a> {
        SchemaNode {
            json,
            scope: Scope::Verbatim,
        }
    }

    /// The node as an object, its keys as the expanded copy has them.
    pub(crate) fn object(&self) -> Option<&'a Map<String, Json>> {
        self.json.as_object()
    }

    /// The node's string, if it is one.
    pub(crate) fn as_str(&self) -> Option<&'a str> {
        self.json.as_str()
    }

    /// `node[key]` of an object node, as the expanded copy has it.
    pub(crate) fn get(&self, key: &str) -> Option<SchemaNode<'a>> {
        let child = self.json.as_object()?.get(key)?;
        Some(self.scope.at(child))
    }

    /// `child`, a value inside this node's own JSON reached through objects
    /// and arrays with no `$ref` on the way (a property of a `properties`
    /// node, a member of an `anyOf` array), as the expanded copy has it.
    pub(crate) fn child(&self, child: &'a Json) -> SchemaNode<'a> {
        self.scope.at(child)
    }

    /// The value at this node, as the expanded copy holds it: borrowed when
    /// it is the same as written, copied with its references followed
    /// otherwise.
    pub(crate) fn data(&self) -> Cow<'a, Json> {
        match (&self.scope, self.json) {
            (Scope::Verbatim, json)
            | (_, json @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::String(_))) => {
                Cow::Borrowed(json)
            }
            _ => Cow::Owned(copy_tree(self.clone())),
        }
    }
}

/// A copy of the tree at `root` as the expanded copy has it, made with a
/// loop: each container's children are read in its scope.
fn copy_tree(root: SchemaNode<'_>) -> Json {
    enum Open<'a> {
        Object(
            serde_json::map::Iter<'a>,
            Scope<'a>,
            Map<String, Json>,
            String,
        ),
        Array(std::slice::Iter<'a, Json>, Scope<'a>, Vec<Json>),
    }
    let mut open: Vec<Open<'_>> = Vec::new();
    let mut next = Some(root);
    // Each turn opens a container or finishes a scalar, then hands finished
    // values up until a container has another child to visit: every turn
    // consumes one child, so the walk ends after the copy's node count.
    loop {
        let mut done = None;
        if let Some(node) = next.take() {
            match node.json {
                Json::Object(map) => open.push(Open::Object(
                    map.iter(),
                    node.scope,
                    Map::with_capacity(map.len()),
                    String::new(),
                )),
                Json::Array(items) => open.push(Open::Array(
                    items.iter(),
                    node.scope,
                    Vec::with_capacity(items.len()),
                )),
                scalar => done = Some(scalar.clone()),
            }
        }
        loop {
            if let Some(value) = done.take() {
                match open.last_mut() {
                    None => return value,
                    Some(Open::Object(_, _, out, key)) => {
                        out.insert(std::mem::take(key), value);
                    }
                    Some(Open::Array(_, _, out)) => out.push(value),
                }
            }
            match open.last_mut() {
                None => return Json::Null,
                Some(Open::Object(entries, scope, _, key)) => {
                    if let Some((k, v)) = entries.next() {
                        k.clone_into(key);
                        next = Some(scope.at(v));
                        break;
                    }
                }
                Some(Open::Array(items, scope, _)) => {
                    if let Some(v) = items.next() {
                        next = Some(scope.at(v));
                        break;
                    }
                }
            }
            done = match open.pop() {
                Some(Open::Object(_, _, out, _)) => Some(Json::Object(out)),
                Some(Open::Array(_, _, out)) => Some(Json::Array(out)),
                None => return Json::Null,
            };
        }
    }
}

/// A deep copy of `json`, made with a loop: serde_json's own clone recurses.
pub(crate) fn clone_json(json: &Json) -> Json {
    match json {
        Json::Array(_) | Json::Object(_) => copy_tree(SchemaNode::verbatim(json)),
        scalar => scalar.clone(),
    }
}

/// Frees `json` with a loop when it has children: serde_json's own drop
/// recurses.
fn free(json: Json) {
    if matches!(json, Json::Array(_) | Json::Object(_)) {
        crate::free_json(json);
    }
}

/// `Object.entries(value)`, borrowing what it can: an object's or array's
/// values as they are (no deep copy), a string's UTF-16 units as new
/// one-unit strings.
fn entries(value: &Json) -> Vec<(String, Cow<'_, Json>)> {
    match value {
        Json::Object(map) => json_entries(map)
            .into_iter()
            .map(|(k, v)| (k.clone(), Cow::Borrowed(v)))
            .collect(),
        Json::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), Cow::Borrowed(v)))
            .collect(),
        Json::String(_) => object_entries(value)
            .into_iter()
            .map(|(k, v)| (k, Cow::Owned(v)))
            .collect(),
        _ => Vec::new(),
    }
}

/// `compileParams`: params in `properties` order, each with whether it's
/// required, its default, and its schema as written.
fn compile_params(def: &Json) -> Vec<Param> {
    let required = def.get("required");
    // `required.includes(key)`: array membership, or a substring of a string.
    // The array's strings are gathered once, so each prop's lookup is one
    // set lookup, not a scan of the list.
    let listed: HashSet<&str> = match required {
        Some(Json::Array(items)) => items.iter().filter_map(Json::as_str).collect(),
        _ => HashSet::new(),
    };
    let is_required = |key: &str| match required {
        Some(Json::Array(_)) => listed.contains(key),
        Some(Json::String(s)) => s.contains(key),
        _ => false,
    };
    def.get("properties")
        .map(entries)
        .unwrap_or_default()
        .into_iter()
        .map(|(key, prop)| Param {
            required: is_required(&key),
            default: prop
                .as_object()
                .and_then(|p| p.get("default"))
                .map(clone_json),
            schema: match prop {
                Cow::Borrowed(prop) => clone_json(prop),
                Cow::Owned(prop) => prop,
            },
            name: key,
        })
        .collect()
}

/// How deep serde_json reads before it refuses: its recursion limit, 128
/// levels counted from 128 down, so 127 nested arrays or objects are read
/// and the 128th is refused.
const SERDE_JSON_LEVELS: usize = 128;

/// The most stack serde_json's reader takes per level, rounded up: measured
/// (2026-11-22, serde_json 1.0.151, Apple silicon) at opt-level 0, 2.3 KiB
/// for nested arrays and 2.8 KiB for nested objects (369 KiB to read and
/// drop 127 nested objects, 17 KiB to read one), and less optimized.
const SERDE_JSON_STACK_PER_LEVEL: usize = 3 * 1024;

/// The segment [`read_host_json`] reads on: four times the reader's need at
/// its level bound, plus room for the reader's own entry frames, so text
/// nested to the limit, or one level past it, fits on any thread at any
/// optimization level. 1,664 KiB, reserved rather than touched.
const HOST_JSON_SEGMENT: usize = SERDE_JSON_LEVELS * SERDE_JSON_STACK_PER_LEVEL * 4 + 128 * 1024;

/// `serde_json::from_str` of a host's JSON text, run on a stack segment of
/// its own sized for serde_json's level bound, so that reading text nested
/// as deep as serde_json allows, or refusing text nested deeper, needs
/// nothing of the caller's stack (ADR 0002). It accepts and refuses exactly
/// what `serde_json::from_str` does, with the same errors. The value it
/// returns nests at most as deep; serde_json drops a value by recursing, so
/// free it with [`crate::free_json`] on a small stack.
///
/// The bindings read a library schema's text through this (#140), and the
/// native binding a prompt spec (#203).
pub fn read_host_json(text: &str) -> Result<Json, serde_json::Error> {
    stacker::grow(HOST_JSON_SEGMENT, || serde_json::from_str(text))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The reference's `inlineActionRefs` as this library ran it before
    /// shapes were shared: the expanded copy, made eagerly. Kept as the
    /// oracle the shared form is checked against.
    fn inline_action_shapes(
        node: &Json,
        shapes: &IndexMap<String, Json>,
        inlining: &mut Vec<String>,
    ) -> Json {
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

    /// Every param's schema as read through [`SchemaNode`] equals the
    /// expanded copy the reference makes.
    fn assert_reads_as_expanded(schema: &Json) {
        let library = Library::from_json_schema(schema);
        let params = library
            .components
            .values()
            .flat_map(|c| c.params.iter())
            .chain(library.calls.values().flat_map(|c| c.params.iter()));
        for param in params {
            let expected = if library.shapes.bodies.is_empty() {
                param.schema.clone()
            } else {
                inline_action_shapes(&param.schema, &library.shapes.bodies, &mut Vec::new())
            };
            let read = library.schema_root(&param.schema).data().into_owned();
            assert_eq!(read, expected, "{}: {schema}", param.name);
        }
    }

    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }
    }

    fn random_node(rng: &mut Rng, names: &[&str], depth: usize) -> Json {
        let target = |rng: &mut Rng| json!(format!("#/$defs/{}", names[rng.below(names.len())]));
        match rng.below(if depth == 0 { 4 } else { 9 }) {
            0 => json!({"type": "string"}),
            1 => json!({"$ref": target(rng)}),
            2 => json!(rng.below(3)),
            3 => json!("text"),
            4 => json!({"$ref": target(rng), "sibling": random_node(rng, names, depth - 1)}),
            5 => {
                json!({"$ref": random_node(rng, names, depth - 1), "x": random_node(rng, names, depth - 1)})
            }
            6 => json!([
                random_node(rng, names, depth - 1),
                random_node(rng, names, depth - 1)
            ]),
            7 => json!({"anyOf": [random_node(rng, names, depth - 1), {"$ref": target(rng)}]}),
            _ => json!({
                "type": "object",
                "properties": {"a": random_node(rng, names, depth - 1), "b": random_node(rng, names, depth - 1)},
                "default": random_node(rng, names, depth - 1),
            }),
        }
    }

    #[test]
    fn shared_shapes_read_as_the_reference_expands_them() {
        let names = [
            "A",
            "B",
            "C",
            "ActionExpression",
            "OpenUrl",
            "Box",
            "Missing",
        ];
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..3000 {
            let mut defs = Map::new();
            let mut actions = Map::new();
            for name in ["A", "B", "C", "ActionExpression", "OpenUrl", "Box"] {
                if rng.below(5) > 0 {
                    defs.insert(name.into(), random_node(&mut rng, &names, 3));
                }
                if name.len() == 1 {
                    actions.insert(
                        name.into(),
                        json!({"params": {"type": "object", "properties": {
                            "p": random_node(&mut rng, &names, 3),
                            "q": random_node(&mut rng, &names, 3),
                        }}}),
                    );
                }
            }
            let mut schema = json!({"$defs": defs, "actions": actions});
            if rng.below(2) == 0 {
                schema["properties"] = json!({"Box": {}});
            }
            assert_reads_as_expanded(&schema);
        }
    }

    #[test]
    fn cycles_and_self_references_stop_where_the_reference_stops() {
        let schema = json!({
            "$defs": {
                "A": {"type": "object", "properties": {"me": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}},
                "B": {"$ref": "#/$defs/C"},
                "C": {"anyOf": [{"$ref": "#/$defs/A"}, {"$ref": "#/$defs/B"}, {"$ref": "#/$defs/C"}]},
                "Box": {"properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}}}
            },
            "properties": {"Box": {}},
            "actions": {"A": {"params": {"properties": {"x": {"$ref": "#/$defs/A"}}}}, "B": {}, "C": {}}
        });
        assert_reads_as_expanded(&schema);
    }

    /// More shapes than the path's mask has bits: the mask only filters, the
    /// list decides.
    #[test]
    fn long_paths_past_the_mask_width_stop_where_the_reference_stops() {
        let mut defs = Map::new();
        let mut actions = Map::new();
        for i in 0..300 {
            defs.insert(
                format!("A{i}"),
                json!({"type": "object", "properties": {
                    "next": {"$ref": format!("#/$defs/A{}", (i + 1) % 300)},
                    "far": {"$ref": format!("#/$defs/A{}", (i + 128) % 300)},
                }}),
            );
            actions.insert(format!("A{i}"), json!({}));
        }
        defs.insert(
            "Box".into(),
            json!({"properties": {"a": {"$ref": "#/$defs/A0"}}}),
        );
        let schema = json!({"$defs": defs, "actions": actions, "properties": {"Box": {}}});
        let library = Library::from_json_schema(&schema);
        let param = &library.component("Box").unwrap().params[0];
        // Walk `next` 300 times: the 300th returns to A0, already entered.
        let mut node = library.schema_root(&param.schema);
        for i in 0..300 {
            let next = node.get("properties").unwrap().get("next").unwrap();
            if i < 299 {
                assert!(next.get("$ref").is_none(), "followed at {i}");
            } else {
                assert_eq!(next.get("$ref").unwrap().as_str(), Some("#/$defs/A0"));
            }
            node = next;
        }
    }

    #[test]
    fn deep_schemas_compile_clone_and_drop_with_loops() {
        // Built by hand: `json!` copies an interpolated value by recursing.
        let mut schema = json!({"type": "string"});
        for _ in 0..100_000 {
            let mut array = Map::new();
            array.insert("type".into(), "array".into());
            array.insert("items".into(), schema);
            schema = Json::Object(array);
        }
        let mut props = Map::new();
        props.insert("a".into(), schema);
        let mut component = Map::new();
        component.insert("properties".into(), Json::Object(props));
        let mut defs = Map::new();
        defs.insert("Box".into(), Json::Object(component));
        let mut root = Map::new();
        root.insert("$defs".into(), Json::Object(defs));
        let schema = Json::Object(root);
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                let library = Library::from_json_schema(&schema);
                let copy = library.clone();
                drop(library);
                drop(copy);
                crate::free_json(schema);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
