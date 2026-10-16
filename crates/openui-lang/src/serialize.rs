//! Element trees back to OpenUI Lang source, as the reference parser's
//! `jsonToOpenUI`.
//!
//! The input is an element tree in its JSON form: what a parse result's
//! `root` serializes to (read it back with [`crate::parse_json`]), or one built
//! by hand. The reference walks the live JavaScript objects; JSON is how they
//! cross any boundary, so this walks a `serde_json::Value` and applies the
//! JavaScript rules the reference relies on: which objects count as elements
//! and AST nodes, property order, how values stringify into the text, and
//! where it throws.

use std::collections::HashSet;
use std::fmt;

use serde_json::Value as Json;

use crate::js::{
    OBJECT_PROTOTYPE_KEYS, json_entries, json_number, json_string, json_value_string,
    lone_surrogate, number_to_string, object_entries, truthy,
};
use crate::library::Library;

/// How deeply the input may nest, in JSON levels. A parse result nests at most
/// three JSON levels per [`crate::MAX_DEPTH`] level (a runtime expression of
/// nested objects; measured at 764), so every parse result fits. A tree built
/// by hand at this depth needs 448 KB of stack to serialize in a test build
/// (opt-level 1) and 320 KB in release, inside the 512 KB embedders give the
/// core. Reading JSON back doesn't recurse at all.
pub(crate) const JSON_DEPTH_LIMIT: usize = 800;

/// Why an element tree couldn't be serialized: the reference throws on the
/// same inputs (a missing node, a field of the wrong type, an object that
/// can't be turned into text), or it nests deeper than 800 levels, which no
/// parse result does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializeError(String);

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SerializeError {}

fn thrown(message: &str) -> SerializeError {
    SerializeError(format!("[jsonToOpenUI] {message}"))
}

/// Turn an element tree back into OpenUI Lang, as the reference's `jsonToOpenUI`.
///
/// `element` is the root element in its JSON form; `state_declarations` is a
/// parse result's `stateDeclarations` (its non-null values become `$name = ...`
/// lines). Elements with a `statementId` become statements of their own, child
/// statements first; others are written inline. Props go back to positional
/// arguments in the library's order, with trailing nulls for optional params
/// left off. Calls in runtime expressions are written with `@` when they're
/// built-ins or the library's functions and actions, `Action` aside.
///
/// To serialize a [`crate::ParseResult`], read its JSON back:
/// `let json = parse_json(&result.to_json())?;` then pass `&json["root"]` and
/// `Some(&json["stateDeclarations"])`.
pub fn json_to_openui(
    element: &Json,
    library: &Library,
    state_declarations: Option<&Json>,
) -> Result<String, SerializeError> {
    let Json::Object(root) = element else {
        return Err(thrown("the root must be an element object"));
    };
    let mut s = Serializer {
        library,
        statements: Vec::new(),
        registered: HashSet::new(),
        unique: 0,
        depth: 0,
    };
    let root_expr = s.element_expr(element)?;
    let (root_key, root_id) = match root.get("statementId").filter(|id| truthy(Some(id))) {
        Some(id) => (s.key(id), to_string(id)?),
        None => (Key::Str("root".to_owned()), "root".to_owned()),
    };
    let mut lines = vec![format!("{root_id} = {root_expr}")];
    for (key, text) in std::mem::take(&mut s.statements) {
        if key != root_key {
            lines.push(text);
        }
    }
    if let Some(declarations) = state_declarations.filter(|d| truthy(Some(d))) {
        for (name, value) in object_entries(declarations) {
            if value.is_null() {
                continue; // auto-declared nulls are left out
            }
            let text = s.value(Some(&value))?.template()?;
            lines.push(format!("{name} = {text}"));
        }
    }
    Ok(lines.join("\n"))
}

/// A statement id as the reference's `Set` tells them apart: strings and
/// numbers by value, `true`, and each object or array as itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Str(String),
    Num(u64),
    True,
    Unique(usize),
}

struct Serializer<'a> {
    library: &'a Library,
    /// Statements in the order they were registered: children before parents.
    statements: Vec<(Key, String)>,
    registered: HashSet<Key>,
    unique: usize,
    depth: usize,
}

/// What a step of the reference serializer returns. Mostly a string, but a few
/// places hand back a raw value (a reference's `n`, a `statementId`) or
/// `undefined` (`JSON.stringify` of a missing `v`), and those turn into text
/// differently depending on where they land.
enum Out<'a> {
    Text(String),
    Raw(&'a Json),
    Undefined,
}

impl Out<'_> {
    /// `${x}` in a template literal.
    fn template(&self) -> Result<String, SerializeError> {
        match self {
            Out::Text(s) => Ok(s.clone()),
            Out::Raw(v) => to_string(v),
            Out::Undefined => Ok("undefined".to_owned()),
        }
    }

    /// An item of `Array.prototype.join`: null and undefined are empty.
    fn join_item(&self) -> Result<String, SerializeError> {
        match self {
            Out::Raw(Json::Null) | Out::Undefined => Ok(String::new()),
            _ => self.template(),
        }
    }

    /// `x === "null"`.
    fn is_null_text(&self) -> bool {
        match self {
            Out::Text(s) => s == "null",
            Out::Raw(Json::String(s)) => s == "null",
            _ => false,
        }
    }
}

/// `String(value)` for a JSON value. An object with its own `toString` key
/// can't be converted (the key shadows the method, and `valueOf` gives back
/// the object), so that throws, as in JavaScript.
fn to_string(value: &Json) -> Result<String, SerializeError> {
    match value {
        Json::Null => Ok("null".to_owned()),
        Json::Bool(b) => Ok(b.to_string()),
        Json::Number(n) => Ok(number_to_string(json_number(n))),
        Json::String(s) => Ok(s.clone()),
        Json::Array(items) => {
            let parts: Result<Vec<String>, _> = items
                .iter()
                .map(|v| {
                    if v.is_null() {
                        Ok(String::new())
                    } else {
                        to_string(v)
                    }
                })
                .collect();
            Ok(parts?.join(","))
        }
        Json::Object(map) if map.contains_key("toString") => {
            Err(thrown("cannot convert object to primitive value"))
        }
        Json::Object(_) => Ok("[object Object]".to_owned()),
    }
}

/// `x` in a template literal, where `x` may be missing (`undefined`).
fn field_text(value: Option<&Json>) -> Result<String, SerializeError> {
    value.map_or_else(|| Ok("undefined".to_owned()), to_string)
}

/// A string, number or boolean as `serializeValue` writes it.
fn scalar(value: &Json) -> Out<'static> {
    Out::Text(match value {
        Json::String(s) => json_string(s),
        Json::Number(n) => number_to_string(json_number(n)),
        Json::Bool(b) => b.to_string(),
        _ => unreachable!("only scalars"),
    })
}

/// The AST kinds that don't recurse.
#[inline(never)]
fn ast_leaf<'v>(
    kind: &str,
    o: &'v serde_json::Map<String, Json>,
) -> Result<Out<'v>, SerializeError> {
    Ok(match kind {
        "Str" => o
            .get("v")
            .map_or(Out::Undefined, |v| Out::Text(json_value_string(v))),
        "Num" => Out::Text(field_text(o.get("v"))?),
        "Bool" => Out::Text(if truthy(o.get("v")) { "true" } else { "false" }.to_owned()),
        "Null" => Out::Text("null".to_owned()),
        // Ph, StateRef, RuntimeRef, Ref: the name, as it is.
        _ => o.get("n").map_or(Out::Undefined, Out::Raw),
    })
}

/// The reference's `isElementNode`.
fn is_element(value: &Json) -> bool {
    let Json::Object(o) = value else { return false };
    matches!(o.get("type"), Some(Json::String(t)) if t == "element")
        && matches!(o.get("typeName"), Some(Json::String(_)))
        && matches!(o.get("props"), Some(Json::Object(_) | Json::Array(_)))
        && matches!(o.get("partial"), Some(Json::Bool(_)))
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

/// The reference's `isASTNode`.
fn is_ast(value: &Json) -> bool {
    matches!(value, Json::Object(o) if matches!(o.get("k"), Some(Json::String(k)) if AST_KINDS.contains(&k.as_str())))
}

/// `props[name]`, as JavaScript reads it from parsed JSON. Only the root's
/// props can be a string, number or boolean (a nested element's must be an
/// object to count as one); those read through their prototype.
enum Prop<'a> {
    Own(&'a Json),
    /// Missing, or an inherited method: both serialize as `null`.
    Missing,
    /// A value that isn't in the JSON, already serialized.
    Text(String),
}

/// `name` as an array index, if it's one in canonical form.
fn index(name: &str) -> Option<usize> {
    let i: usize = name.parse().ok()?;
    (i.to_string() == name).then_some(i)
}

/// A string's UTF-16 units, each as the one-unit string JavaScript indexes out.
fn units(s: &str) -> Vec<String> {
    s.encode_utf16()
        .map(|unit| match unit {
            0xD800..=0xDFFF => lone_surrogate(unit).to_string(),
            _ => char::from_u32(u32::from(unit))
                .expect("not a surrogate")
                .to_string(),
        })
        .collect()
}

fn prop<'a>(props: &'a Json, name: &str) -> Prop<'a> {
    match props {
        Json::Object(map) => match map.get(name) {
            Some(v) => Prop::Own(v),
            // `Object.prototype` serializes as an empty plain object.
            None if name == "__proto__" => Prop::Text("{}".to_owned()),
            None => Prop::Missing,
        },
        Json::Array(items) => match (index(name).and_then(|i| items.get(i)), name) {
            (Some(v), _) => Prop::Own(v),
            (None, "length") => Prop::Text(items.len().to_string()),
            // `Array.prototype` is itself an empty array.
            (None, "__proto__") => Prop::Text("[]".to_owned()),
            _ => Prop::Missing,
        },
        Json::String(s) => {
            let units = units(s);
            match (index(name).and_then(|i| units.get(i)), name) {
                (Some(unit), _) => Prop::Text(json_string(unit)),
                (None, "length") => Prop::Text(units.len().to_string()),
                // `String.prototype` has no enumerable keys of its own.
                (None, "__proto__") => Prop::Text("{}".to_owned()),
                _ => Prop::Missing,
            }
        }
        // `Number.prototype`, `Boolean.prototype`: no enumerable keys either.
        _ if name == "__proto__" => Prop::Text("{}".to_owned()),
        _ => Prop::Missing,
    }
}

/// An operator's precedence as `PRECEDENCE[op] ?? 0` gives it: a number, or,
/// for a name every object inherits, a value that compares as a string.
enum Precedence {
    Num(f64),
    Str(String),
}

fn precedence(op: &str) -> Precedence {
    Precedence::Num(match op {
        "||" => 2.0,
        "&&" => 3.0,
        "==" | "!=" => 4.0,
        ">" | "<" | ">=" | "<=" => 5.0,
        "+" | "-" => 6.0,
        "*" | "/" | "%" => 7.0,
        "__proto__" => return Precedence::Str("[object Object]".to_owned()),
        // A method: compares as its source text, `function name() { ... }`.
        "constructor" => return Precedence::Str("function Object() {".to_owned()),
        _ if OBJECT_PROTOTYPE_KEYS.contains(&op) => {
            return Precedence::Str(format!("function {op}() {{"));
        }
        _ => 0.0,
    })
}

/// `a < b` for two precedences, as JavaScript compares them.
fn less(a: &Precedence, b: &Precedence) -> bool {
    match (a, b) {
        (Precedence::Num(a), Precedence::Num(b)) => a < b,
        (Precedence::Str(a), Precedence::Str(b)) => a.encode_utf16().lt(b.encode_utf16()),
        // A string here never reads as a number, and NaN compares false.
        _ => false,
    }
}

impl<'a> Serializer<'a> {
    fn key(&mut self, id: &Json) -> Key {
        match id {
            Json::String(s) => Key::Str(s.clone()),
            Json::Number(n) => {
                let n = json_number(n);
                Key::Num(if n == 0.0 { 0.0f64 } else { n }.to_bits())
            }
            Json::Bool(true) => Key::True,
            _ => {
                self.unique += 1;
                Key::Unique(self.unique)
            }
        }
    }

    fn enter(&mut self) -> Result<(), SerializeError> {
        self.depth += 1;
        if self.depth > JSON_DEPTH_LIMIT {
            return Err(thrown("the element tree nests too deeply"));
        }
        Ok(())
    }

    /// The reference's `serializeValue`; `None` is `undefined`.
    ///
    /// The recursive path stays small, and each heavier case is an
    /// `#[inline(never)]` helper: a frame then holds only the case in hand,
    /// which keeps `JSON_DEPTH_LIMIT` levels inside a small thread stack.
    fn value<'v>(&mut self, value: Option<&'v Json>) -> Result<Out<'v>, SerializeError> {
        self.enter()?;
        let out = match value {
            None | Some(Json::Null) => Ok(Out::Text("null".to_owned())),
            Some(v @ (Json::String(_) | Json::Number(_) | Json::Bool(_))) => Ok(scalar(v)),
            Some(v) if is_element(v) => self.element_value(v),
            Some(v) if is_ast(v) => self.ast(Some(v)),
            Some(Json::Array(items)) => self.array_value(items),
            Some(Json::Object(map)) => self.object_value(map),
        };
        self.depth -= 1;
        out
    }

    #[inline(never)]
    fn array_value<'v>(&mut self, items: &'v [Json]) -> Result<Out<'v>, SerializeError> {
        let mut parts = Vec::with_capacity(items.len());
        for item in items {
            parts.push(self.value(Some(item))?.join_item()?);
        }
        Ok(Out::Text(format!("[{}]", parts.join(", "))))
    }

    #[inline(never)]
    fn object_value<'v>(
        &mut self,
        map: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let mut parts = Vec::with_capacity(map.len());
        for (k, v) in json_entries(map) {
            let text = self.value(Some(v))?.template()?;
            parts.push(format!("{k}: {text}"));
        }
        Ok(Out::Text(format!("{{{}}}", parts.join(", "))))
    }

    /// An element in a value position: a reference to its own statement when
    /// it has a `statementId` (registered the first time), otherwise inline.
    #[inline(never)]
    fn element_value<'v>(&mut self, node: &'v Json) -> Result<Out<'v>, SerializeError> {
        match node.get("statementId").filter(|id| truthy(Some(id))) {
            Some(id) => {
                let key = self.key(id);
                if !self.registered.contains(&key) {
                    let expr = self.element_expr(node)?;
                    if self.registered.insert(key.clone()) {
                        let text = format!("{} = {expr}", to_string(id)?);
                        self.statements.push((key, text));
                    }
                }
                Ok(Out::Raw(id))
            }
            None => Ok(Out::Text(self.element_expr(node)?)),
        }
    }

    /// `TypeName(args...)`: props in the library's param order, trailing nulls
    /// for optional params dropped; for a component the library doesn't know,
    /// the props' own order.
    fn element_expr(&mut self, node: &Json) -> Result<String, SerializeError> {
        let type_name = node.get("typeName");
        // Only the root can come without props: nested elements are told
        // apart by having them. The reference reads them only for a param, or
        // for an unknown component's `Object.values`, and throws there.
        let props = node.get("props").filter(|p| !p.is_null());
        let no_props = || thrown("the root element has no props");
        let library = self.library;
        let def = match type_name {
            Some(Json::String(name)) => library.component(name),
            _ => None,
        };
        let mut args: Vec<Out> = Vec::new();
        match def {
            Some(def) => {
                for param in &def.params {
                    args.push(match prop(props.ok_or_else(no_props)?, &param.name) {
                        Prop::Own(v) => self.value(Some(v))?,
                        Prop::Missing => Out::Text("null".to_owned()),
                        Prop::Text(t) => Out::Text(t),
                    });
                }
                while let Some(last) = args.last() {
                    if !last.is_null_text() || def.params[args.len() - 1].required {
                        break;
                    }
                    args.pop();
                }
            }
            None => {
                // `Object.values(props)`.
                let props = props.ok_or_else(no_props)?;
                let values: Vec<&Json> = match props {
                    Json::Object(map) => json_entries(map).into_iter().map(|(_, v)| v).collect(),
                    Json::Array(items) => items.iter().collect(),
                    _ => Vec::new(),
                };
                for v in values {
                    args.push(self.value(Some(v))?);
                }
                if let Json::String(s) = props {
                    args.extend(units(s).iter().map(|unit| Out::Text(json_string(unit))));
                }
            }
        }
        let parts: Result<Vec<String>, _> = args.iter().map(Out::join_item).collect();
        Ok(format!("{}({})", field_text(type_name)?, parts?.join(", ")))
    }

    /// The reference's `serializeASTNode`; `None` is `undefined`, and reading
    /// a field of it (or of null) throws. Kept small, as `value` is.
    fn ast<'v>(&mut self, node: Option<&'v Json>) -> Result<Out<'v>, SerializeError> {
        self.enter()?;
        let out = match node {
            None | Some(Json::Null) => Err(thrown("cannot read properties of a missing node")),
            Some(Json::Object(o)) => match o.get("k") {
                Some(Json::String(k)) => self.ast_kind(k, o),
                _ => Ok(Out::Text("null".to_owned())),
            },
            // No `k`: the default case.
            Some(_) => Ok(Out::Text("null".to_owned())),
        };
        self.depth -= 1;
        out
    }

    fn ast_kind<'v>(
        &mut self,
        kind: &str,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        match kind {
            "Str" | "Num" | "Bool" | "Null" | "Ph" | "StateRef" | "RuntimeRef" | "Ref" => {
                ast_leaf(kind, o)
            }
            "Arr" => self.ast_arr(o),
            "Obj" => self.ast_obj(o),
            "BinOp" => self.ast_binop(o),
            "UnaryOp" | "Ternary" | "Member" | "Index" | "Assign" => self.ast_compound(kind, o),
            "Comp" => self.ast_comp(o),
            _ => Ok(Out::Text("null".to_owned())),
        }
    }

    #[inline(never)]
    fn ast_arr<'v>(
        &mut self,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let Some(Json::Array(els)) = o.get("els") else {
            return Err(thrown("els is not an array"));
        };
        let mut parts = Vec::with_capacity(els.len());
        for el in els {
            parts.push(self.ast(Some(el))?.join_item()?);
        }
        Ok(Out::Text(format!("[{}]", parts.join(", "))))
    }

    #[inline(never)]
    fn ast_obj<'v>(
        &mut self,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let Some(Json::Array(entries)) = o.get("entries") else {
            return Err(thrown("entries is not an array"));
        };
        let mut parts = Vec::with_capacity(entries.len());
        for entry in entries {
            parts.push(self.obj_entry(entry)?);
        }
        Ok(Out::Text(format!("{{{}}}", parts.join(", "))))
    }

    #[inline(never)]
    fn ast_binop<'v>(
        &mut self,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let op = o.get("op");
        let left = self.binop_child(o.get("left"), op)?.template()?;
        let right = self.binop_child(o.get("right"), op)?.template()?;
        Ok(Out::Text(format!("{left} {} {right}", field_text(op)?)))
    }

    #[inline(never)]
    fn ast_compound<'v>(
        &mut self,
        kind: &str,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let text = match kind {
            "UnaryOp" => {
                let op = field_text(o.get("op"))?;
                format!("{op}{}", self.ast(o.get("operand"))?.template()?)
            }
            "Ternary" => {
                let cond = self.ast(o.get("cond"))?.template()?;
                let then = self.ast(o.get("then"))?.template()?;
                let otherwise = self.ast(o.get("else"))?.template()?;
                format!("{cond} ? {then} : {otherwise}")
            }
            "Member" => {
                let obj = self.ast(o.get("obj"))?.template()?;
                format!("{obj}.{}", field_text(o.get("field"))?)
            }
            "Index" => {
                let obj = self.ast(o.get("obj"))?.template()?;
                let index = self.ast(o.get("index"))?.template()?;
                format!("{obj}[{index}]")
            }
            _ => {
                let target = field_text(o.get("target"))?;
                format!("{target} = {}", self.ast(o.get("value"))?.template()?)
            }
        };
        Ok(Out::Text(text))
    }

    #[inline(never)]
    fn ast_comp<'v>(
        &mut self,
        o: &'v serde_json::Map<String, Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let Some(Json::Array(args)) = o.get("args") else {
            return Err(thrown("args is not an array"));
        };
        let mut parts = Vec::with_capacity(args.len());
        for arg in args {
            parts.push(self.ast(Some(arg))?.join_item()?);
        }
        let name = o.get("name");
        // Built-ins and the library's functions and actions, but not Action.
        let call = matches!(name, Some(Json::String(n)) if self.library.call(n).is_some() && n != "Action");
        let prefix = if call { "@" } else { "" };
        Ok(Out::Text(format!(
            "{prefix}{}({})",
            field_text(name)?,
            parts.join(", ")
        )))
    }

    /// `${k}: ${value}` for one `[k, v]` entry of an `Obj` node. Destructuring
    /// takes an array's first two items or a string's first two code points;
    /// anything else isn't iterable and throws.
    #[inline(never)]
    fn obj_entry(&mut self, entry: &Json) -> Result<String, SerializeError> {
        match entry {
            Json::Array(items) => {
                let key = field_text(items.first())?;
                Ok(format!("{key}: {}", self.ast(items.get(1))?.template()?))
            }
            Json::String(s) => {
                let mut chars = s.chars();
                let key = chars
                    .next()
                    .map_or_else(|| "undefined".to_owned(), String::from);
                match chars.next() {
                    // The value is a one-character string: a node with no `k`.
                    Some(_) => Ok(format!("{key}: null")),
                    None => Err(thrown("cannot read properties of a missing node")),
                }
            }
            _ => Err(thrown("an Obj entry is not iterable")),
        }
    }

    /// A `BinOp` operand, parenthesized when it's a `BinOp` that binds looser.
    #[inline(never)]
    fn binop_child<'v>(
        &mut self,
        child: Option<&'v Json>,
        parent_op: Option<&Json>,
    ) -> Result<Out<'v>, SerializeError> {
        let inner = self.ast(child)?;
        let Some(Json::Object(c)) = child else {
            return Ok(inner);
        };
        if !matches!(c.get("k"), Some(Json::String(k)) if k == "BinOp") {
            return Ok(inner);
        }
        let parent = precedence(&field_text(parent_op)?);
        let own = precedence(&field_text(c.get("op"))?);
        if less(&own, &parent) {
            return Ok(Out::Text(format!("({})", inner.template()?)));
        }
        Ok(inner)
    }
}
