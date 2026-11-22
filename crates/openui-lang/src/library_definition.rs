//! A library definition in plain-data form: the declarations of a component
//! library's components, functions and actions, which
//! [`create_library`](crate::create_library)
//! turns into the library's JSON Schema and prompt spec, as upstream's
//! `createLibrary` does from zod declarations (ADR 0017).
//!
//! Upstream has no plain-data form: its authors write zod. This one is ours,
//! so a host without zod (Swift, Kotlin, Rust) can describe a library and get
//! the output upstream would give for the equivalent zod declaration, byte for
//! byte. It crosses a binding as JSON text, like every other value.
//!
//! # The JSON shape
//!
//! ```json
//! {
//!   "components": [
//!     {
//!       "name": "Metric",
//!       "description": "A labelled number.",
//!       "props": [
//!         {"name": "label", "type": {"kind": "string"}},
//!         {"name": "value", "type": {"kind": "number"}},
//!         {"name": "delta", "type": {"kind": "number"}, "optional": true},
//!         {"name": "data", "type": {"kind": "any"}, "optional": true}
//!       ]
//!     }
//!   ],
//!   "componentGroups": [
//!     {"name": "Content", "components": ["Metric"], "notes": ["Use for numbers."]}
//!   ],
//!   "functions": [
//!     {
//!       "name": "Percent",
//!       "description": "Percent of a total.",
//!       "params": [
//!         {"name": "part", "type": {"kind": "number"}},
//!         {"name": "total", "type": {"kind": "number"}}
//!       ],
//!       "returns": {"kind": "number"}
//!     }
//!   ],
//!   "actions": [
//!     {
//!       "name": "Copy",
//!       "description": "Copies text.",
//!       "params": [{"name": "text", "type": {"kind": "string"}}]
//!     }
//!   ],
//!   "root": "Metric",
//!   "id": "my-library"
//! }
//! ```
//!
//! - `components` (required): an array, in order. Each has a `name`, a
//!   `description` (may be empty) and `props`, an **ordered array**, never a
//!   map: the order is the positional order of the component's arguments. As
//!   in a JavaScript object, prop names that are array indices ("0", "12")
//!   come first, ascending, whatever their place in the list.
//! - A prop has a `name`, a `type`, `optional` (default `false`) and
//!   `binding` (default `false`). A prop's name may appear once per
//!   component.
//! - A type is an object with a `kind`, and the fields that kind has, no
//!   others. Any other kind is refused with
//!   [`LibraryDefinitionError::UnsupportedKind`], naming it. The kinds:
//!
//!   | Type | zod | JSON Schema zod 4.6.7 writes | Signature |
//!   |---|---|---|---|
//!   | `{"kind": "string"}` | `z.string()` | `{"type": "string"}` | `string` |
//!   | `{"kind": "number"}` | `z.number()` | `{"type": "number"}` | `number` |
//!   | `{"kind": "boolean"}` | `z.boolean()` | `{"type": "boolean"}` | `boolean` |
//!   | `{"kind": "any"}` | `z.any()` | `{}` | `any` |
//!   | `{"kind": "unknown"}` | `z.unknown()` | `{}` | `any` |
//!   | `{"kind": "enum", "values": ["a", "b"]}` | `z.enum(["a", "b"])` | `{"type": "string", "enum": ["a", "b"]}` | `"a" \| "b"` |
//!   | `{"kind": "array", "items": T}` | `z.array(T)` | `{"type": "array", "items": T}` | `T[]`, `(T)[]` for a union |
//!   | `{"kind": "array", "items": T, "minItems": n}` | `z.array(T).min(n)` | `{"minItems": n, "type": "array", "items": T}` | as an array |
//!   | `{"kind": "union", "options": [T, U]}` | `z.union([T, U])` | `{"anyOf": [T, U]}` | `T \| U` |
//!   | `{"kind": "object", "fields": [F]}` | `z.object({...})` | as a component's props | `{a: T, b?: U}` |
//!   | `{"kind": "record", "values": T}` | `z.record(z.string(), T)` | `{"type": "object", "propertyNames": {"type": "string"}, "additionalProperties": T}` | `Record<string, T>` |
//!   | `{"kind": "nullable", "type": T}` | `T.nullable()` | `{"anyOf": [T, {"type": "null"}]}` | `T`, and the prop or field gets `?` |
//!   | `{"kind": "component", "name": "Item"}` | `Item.ref` | `{"$ref": "#/$defs/Item"}` | `Item` |
//!   | `{"kind": "action"}` | `action()` | `{"$ref": "#/$defs/ActionExpression"}` | `ActionExpression`, `(ActionExpression)[]` in an array |
//!   | `{"kind": "action", "name": "OpenUrl"}` | `steps.OpenUrl.ref` | `{"$ref": "#/$defs/OpenUrl"}` | `@OpenUrl` |
//!   | `{"kind": "action", "name": "Copy"}` | `copy.ref` (a library action) | `{"$ref": "#/$defs/Copy"}` | `@Copy` |
//!   | `{"kind": "named", "name": "Color", "type": T}` | `tagSchemaId(T, "Color")` | `T` | `Color` |
//!
//!   As zod 4.6.7 does, an `anyOf` whose members are all bare types
//!   (`{"type": ...}` and nothing else) is written as one `type` list, each
//!   type once: a union of string and number is `{"type": ["string",
//!   "number"]}`, a nullable string `{"type": ["string", "null"]}`, a
//!   one-member union its member's type. A nullable prop stays in `required`
//!   but its signature has `?`, as upstream's signature builder treats
//!   nullable as optional; optional and nullable are separate facts, and a
//!   prop may be both (zod's `.nullable().optional()` and `.nullish()`).
//!   Upstream's other quirks are copied too: an enum's words come in
//!   JavaScript property order (array indices first) with each word once, and
//!   unescaped in the signature; an empty enum is `{"not": {}}` and has no
//!   signature type (the prop prints as its bare name, or `[name]` when it is
//!   an array of one); an empty union is `{"anyOf": []}` and prints `any`.
//! - A prop or field may carry `"default": value`, for any JSON value,
//!   including null. Zod writes `default` before the inner schema's keys.
//!   The field stays in `required` unless optional, and prints with `?`.
//! - A custom refinement on the props object uses the ordinary props list.
//!   Upstream emits the object without its validation predicate.
//! - The kinds that link a library together:
//!   - A **component reference** names a component of the library: the
//!     last one of that name, as upstream's record keeps the last, or the
//!     last one named `__proto__`, whose `$defs` entry upstream writes
//!     without a description. A name that is no component's is refused
//!     with [`LibraryDefinitionError::UnknownComponent`], and a reference
//!     to the component with the empty name as invalid (upstream writes
//!     that one's schema inline, or never finishes when it refers to
//!     itself).
//!   - An **action slot** takes any action; with a `name` it takes that one
//!     action, `OpenUrl`, `ToAssistant` or one of the library's own actions
//!     (another name is refused with
//!     [`LibraryDefinitionError::UnknownAction`], and a reference to the
//!     library action with the empty name as invalid: upstream writes that
//!     one inline), and a slot restricted to several is a union of them. An
//!     action's `$defs` entry is written only when a component refers to it,
//!     and a built-in one's not at all when a component has its name: the
//!     action is then written inline wherever it is used. A library action's
//!     entry is `{"type": "object", "properties": {"type": {"type": "string",
//!     "const": name}, "params": ...}, "required": ["type", "params"],
//!     "additionalProperties": false}`, as upstream's `defineAction` makes it.
//!   - A **named type** prints its name in the signature and is written as
//!     its `type` in the schema. Its `type` may not be a component
//!     reference, an action slot or another named type. As in upstream, the
//!     name belongs to the type it tags: a named nullable prints its inner
//!     type's annotation (upstream unwraps the nullable first), an empty name
//!     prints the type's own annotation, and a named union as an array's
//!     items is parenthesised.
//!   - A prop's **`binding`** marks its outermost type, the `optional`
//!     included, as upstream's `reactive()`: the signature prints
//!     `$binding<T>` (or the bare name when the type has no annotation) and
//!     the schema is the unmarked type's. A binding marks only a prop's
//!     outermost type, so a field or a type inside another has none
//!     (upstream prints `$binding<...>` there too, but its runtime binds only
//!     a prop's outermost schema).
//!
//!   `$defs` come in the order zod first meets each schema: walking the
//!   components in record order, and each one's props depth first, a
//!   component or action referred to gets its entry at its first reference,
//!   followed by the entries of what it refers to in turn.
//! - An object's `fields` are an ordered array like a component's props: a
//!   field has a `name`, a `type` and `optional` (default `false`), a name
//!   may appear once per object, and array-index names come first.
//! - `componentGroups` (optional): copied into the prompt spec in order. A
//!   group has a `name`, `components` (names, not checked) and optional
//!   `notes`, and no other field (another is refused as an unknown field).
//!   The spec always writes a group's fields in that order, whatever order
//!   the JSON gives them in; upstream writes a group's fields as its author
//!   gave them, so the reference harness's converter refuses a declaration
//!   whose groups have other fields or another order.
//! - `functions` (optional): upstream's `defineFunction` without its
//!   implementation, which never enters the core. A function has a `name`, a
//!   `description` (may be empty), `params`, an ordered array of fields like
//!   an inline object's (`name`, `type`, `optional`; no `binding`: nothing
//!   binds a call's argument), and an optional `returns`, a type. Its
//!   signature is `Name(params) → T`, without the arrow when there is no
//!   return type or it has no annotation (an empty enum). The JSON Schema
//!   gets `functions`, after `$defs`: each function's `description`, its
//!   `params` as an object schema and its `returns` schema. As upstream's
//!   `toCallJSONSchema` drops the `$defs` zod writes for a call, a `$ref` in
//!   a call refers to the library's `$defs`, which have an entry only when a
//!   component refers to it too.
//! - `actions` (optional): upstream's `defineAction`, a `name`, a
//!   `description` and `params` as a function's. The JSON Schema gets
//!   `actions` after `functions`, each with its `description` and `params`.
//!   Upstream registers an action named `ActionExpression` under the built-in
//!   action slot's id, and zod throws when a library refers to both, so that
//!   name is refused as invalid.
//!
//!   Functions and actions are keyed by name, in JavaScript property order
//!   (array indices first), in the JSON Schema and in the prompt spec, where
//!   they follow `componentGroups`. A function or action name that is a
//!   built-in, a reserved call (`Query`, `Mutation`, or a name every
//!   JavaScript object inherits, as upstream checks `name in
//!   RESERVED_CALLS`), a component's, or an earlier function's or action's
//!   is refused with [`LibraryDefinitionError::NameCollision`], whose message
//!   is upstream's; functions are checked before actions, each in order,
//!   after the root.
//! - `root` (optional): must name a component, as upstream checks it.
//! - `id` (optional): copied into the prompt spec.
//!
//! A field the form doesn't have is refused, so a misspelt `"optinal"` or a
//! field from a later version is never silently ignored.
//!
//! The form has no object identity: a type used in two places is two equal
//! types. So a zod declaration in which upstream writes one shared schema as
//! a `__schemaN` `$defs` entry (a schema met again inside itself through a
//! component or action reference) is not expressible here; the reference
//! harness's converter refuses it.
//!
//! # How later kinds extend it
//!
//! Each new kind is a new `kind` value with its own fields, read by one new
//! arm in [`TypeDescription`]'s reader, and a new variant of the
//! `#[non_exhaustive]` enum; facts about a prop rather than its type become
//! new optional prop fields, as `binding` is.
//!
//! # Limits
//!
//! Types nest, so reading counts how deep: a prop's type is one level (a
//! call's param and return type too), and
//! each type inside another one more (a named type's `type` too). A type
//! deeper than
//! [`Limits::depth`](crate::Limits::depth) (256 by default) is refused with
//! [`LibraryDefinitionError::DepthLimited`] before it is read, so reading
//! recurses at most that deep, growing the stack on demand as the parser's
//! walks do (ADR 0002). A definition at the default limit fits within the
//! JSON depth [`crate::parse_json`] reads, and its JSON Schema and prompt
//! spec within the depth the prompt entry points accept. Writing the schema
//! and the signatures walks the types read, growing the stack the same way,
//! and [`DefinedLibrary::library`](crate::DefinedLibrary::library) frees
//! and compiles the schema with loops. Otherwise a definition's time and
//! memory grow with its size, nothing more; it is already in the caller's
//! memory. References make the library a graph, which may have cycles;
//! nothing follows one: a reference is written as a `$ref` and printed as a
//! name, the `$defs` order is found visiting each component once, with a
//! loop, and the compiled library shares each action's shape between the
//! params that refer to it instead of copying it (#140). The binding entry
//! points cap a definition's size as they cap other host input (ADR 0001).
//!
//! What is handed back is freed by the caller. A definition, a defined
//! library and a compiled [`Library`](crate::Library) free their trees with
//! loops, so they drop on any stack at any depth (ADR 0002). The JSON Schema
//! and prompt spec are `serde_json` values, whose own drop recurses: at the
//! default limit it fits a 64 KiB stack in optimized builds, but not at
//! opt-level 0; on a small stack, or under a raised depth limit, free them
//! with [`crate::free_json`].
use std::fmt;

use serde_json::{Map, Value as Json};

use crate::js::JsObject;
use crate::library::clone_json;
use crate::{CallKind, Limits};

/// A component library's declarations, in plain-data form. Read one from its
/// JSON with [`LibraryDefinition::from_json`] (the module docs give the
/// shape), then build the library with [`crate::create_library`].
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryDefinition {
    /// The components, in order.
    pub components: Vec<ComponentDefinition>,
    /// The prompt's component groups, if any.
    pub component_groups: Option<Vec<ComponentGroup>>,
    /// The functions, in order.
    pub functions: Vec<FunctionDefinition>,
    /// The library's own actions, in order.
    pub actions: Vec<ActionDefinition>,
    /// The root component's name, if any.
    pub root: Option<String>,
    /// The library's public id, if any.
    pub id: Option<String>,
}

/// One component: upstream's `defineComponent` without its renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentDefinition {
    /// The name programs call it by.
    pub name: String,
    /// Its description for the prompt; may be empty.
    pub description: String,
    /// Its props, in positional order.
    pub props: Vec<PropDefinition>,
}

/// One function: upstream's `defineFunction` without its implementation.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDefinition {
    /// The name programs call it by, with `@`.
    pub name: String,
    /// Its description for the prompt; may be empty.
    pub description: String,
    /// Its params, in positional order.
    pub params: Vec<FieldDefinition>,
    /// What it returns, if declared.
    pub returns: Option<TypeDescription>,
}

/// One action of the library's own: upstream's `defineAction`.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionDefinition {
    /// The name action steps call it by, with `@`.
    pub name: String,
    /// Its description for the prompt; may be empty.
    pub description: String,
    /// Its params, in positional order.
    pub params: Vec<FieldDefinition>,
}

/// One prop of a component.
#[derive(Clone, Debug, PartialEq)]
pub struct PropDefinition {
    /// The prop's name.
    pub name: String,
    /// What the prop holds.
    pub ty: TypeDescription,
    /// Whether a program may leave it out (zod's `.optional()`).
    pub optional: bool,
    /// Whether it is a binding (react-lang's `reactive()` on the prop's
    /// outermost schema, the `.optional()` included).
    pub binding: bool,
}

/// One field of an inline object type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDefinition {
    /// The field's name.
    pub name: String,
    /// What the field holds.
    pub ty: TypeDescription,
    /// Whether it may be left out (zod's `.optional()`).
    pub optional: bool,
}

/// What a prop holds. Later versions add kinds; see the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeDescription {
    /// `z.string()`.
    String,
    /// `z.number()`.
    Number,
    /// `z.boolean()`.
    Boolean,
    /// `z.any()`.
    Any,
    /// `z.unknown()`.
    Unknown,
    /// `z.enum([...])`: one of these words. They are written in JavaScript
    /// property order, each once, as zod keeps them.
    Enum(Vec<String>),
    /// `z.array(T)`.
    Array(Box<TypeDescription>),
    /// `z.array(T).min(n)`, with the tightest declared minimum.
    ArrayWithMinimum(Box<TypeDescription>, serde_json::Number),
    /// `z.union([...])`, its options in order.
    Union(Vec<TypeDescription>),
    /// `z.object({...})`, written inline, its fields in positional order.
    Object(Vec<FieldDefinition>),
    /// `z.record(z.string(), T)`: string keys, values of this type.
    Record(Box<TypeDescription>),
    /// A prop or field with a JSON default. Only the outermost type of a field
    /// can carry this wrapper in the plain-data form.
    Default(Box<TypeDescription>, DefaultValue),
    /// `T.nullable()`: this type or null.
    Nullable(Box<TypeDescription>),
    /// `Item.ref`: the library's component of this name.
    Component(String),
    /// `action()`: any action, or with a name that one action: a built-in
    /// one (`steps.OpenUrl.ref`) or one of the library's (`copy.ref`).
    Action(Option<String>),
    /// `tagSchemaId(T, name)`: `T`, shown by this name in the signature.
    Named(String, Box<TypeDescription>),
}

/// A JSON default, copied and freed with the same iterative walks as a
/// compiled library's parameter defaults.
#[derive(Debug, PartialEq, Eq)]
pub struct DefaultValue(Json);

impl From<Json> for DefaultValue {
    fn from(value: Json) -> Self {
        Self(value)
    }
}

impl AsRef<Json> for DefaultValue {
    fn as_ref(&self) -> &Json {
        &self.0
    }
}

impl Clone for DefaultValue {
    fn clone(&self) -> Self {
        Self(clone_json(&self.0))
    }
}

impl Drop for DefaultValue {
    fn drop(&mut self) {
        if self.0.is_array() || self.0.is_object() {
            crate::free_json(std::mem::take(&mut self.0));
        }
    }
}

impl Drop for TypeDescription {
    /// Types nest as deep as the reader's depth limit allows, and deeper
    /// under a raised one: free the inner types with a loop, so dropping a
    /// definition needs no stack however deep its types nest (ADR 0002).
    fn drop(&mut self) {
        let mut rest = Vec::new();
        self.take_inner(&mut rest);
        while let Some(mut ty) = rest.pop() {
            ty.take_inner(&mut rest);
            // `ty` drops here with no inner types left.
        }
    }
}

impl TypeDescription {
    /// Move the types inside this one onto `rest`, leaving leaves behind.
    fn take_inner(&mut self, rest: &mut Vec<TypeDescription>) {
        match self {
            TypeDescription::Array(inner)
            | TypeDescription::ArrayWithMinimum(inner, _)
            | TypeDescription::Default(inner, _)
            | TypeDescription::Record(inner)
            | TypeDescription::Nullable(inner)
            | TypeDescription::Named(_, inner) => {
                if !inner.is_leaf() {
                    rest.push(std::mem::replace(&mut **inner, TypeDescription::Any));
                }
            }
            TypeDescription::Union(options) => rest.append(options),
            TypeDescription::Object(fields) => {
                rest.extend(
                    fields
                        .iter_mut()
                        .map(|f| std::mem::replace(&mut f.ty, TypeDescription::Any)),
                );
            }
            TypeDescription::String
            | TypeDescription::Number
            | TypeDescription::Boolean
            | TypeDescription::Any
            | TypeDescription::Unknown
            | TypeDescription::Enum(_)
            | TypeDescription::Component(_)
            | TypeDescription::Action(_) => {}
        }
    }

    fn is_leaf(&self) -> bool {
        !matches!(
            self,
            TypeDescription::Array(_)
                | TypeDescription::ArrayWithMinimum(..)
                | TypeDescription::Default(..)
                | TypeDescription::Record(_)
                | TypeDescription::Nullable(_)
                | TypeDescription::Named(..)
                | TypeDescription::Union(_)
                | TypeDescription::Object(_)
        )
    }
}

/// A group of components for the prompt, as upstream's `ComponentGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentGroup {
    /// The group's heading.
    pub name: String,
    /// The components it lists, by name.
    pub components: Vec<String>,
    /// Notes shown under the group, if any.
    pub notes: Option<Vec<String>>,
}

/// Why a library definition was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LibraryDefinitionError {
    /// The root isn't one of the components; the message is upstream's
    /// `createLibrary`'s.
    RootNotFound {
        /// The root asked for.
        root: String,
        /// The components there are, in library order.
        available: Vec<String>,
    },
    /// A type's `kind` is one this version doesn't support.
    UnsupportedKind {
        /// Where, as `components[0].props[1].type`.
        path: String,
        /// The kind given.
        kind: String,
    },
    /// A type nests deeper than [`Limits::depth`] allows.
    DepthLimited {
        /// Where, as `components[0].props[1].type.items`: the first type
        /// past the limit.
        path: String,
        /// The limit, in levels of type.
        limit: usize,
    },
    /// A component reference names no component of the library.
    UnknownComponent {
        /// Where, as `components[0].props[1].type`.
        path: String,
        /// The name given.
        name: String,
    },
    /// A function's or action's name is a built-in's, a reserved call's, a
    /// component's, or an earlier function's or action's; the message is
    /// upstream's `createLibrary`'s.
    NameCollision {
        /// Whether a function or an action has the name.
        kind: CallKind,
        /// The name.
        name: String,
    },
    /// An action slot names an action a slot can't name.
    UnknownAction {
        /// Where, as `components[0].props[1].type`.
        path: String,
        /// The name given.
        name: String,
    },
    /// The definition isn't in the plain-data form.
    Invalid {
        /// Where, as `components[0].props[1].name` (empty for the whole
        /// definition).
        path: String,
        /// What's wrong: "missing", "unknown field", "expected a string", ...
        problem: String,
    },
}

impl fmt::Display for LibraryDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryDefinitionError::RootNotFound { root, available } => write!(
                f,
                "[createLibrary] Root component \"{root}\" was not found in components. Available components: {}",
                available.join(", ")
            ),
            LibraryDefinitionError::UnsupportedKind { path, kind } => {
                let kind = crate::stringify_json(&Json::String(kind.clone()));
                write!(
                    f,
                    "library definition: {path}: type kind {kind} is not supported"
                )
            }
            LibraryDefinitionError::DepthLimited { path, limit } => write!(
                f,
                "library definition: {path}: types nest deeper than {limit} levels"
            ),
            LibraryDefinitionError::UnknownComponent { path, name } => {
                let name = crate::stringify_json(&Json::String(name.clone()));
                write!(
                    f,
                    "library definition: {path}: {name} is not a component of the library"
                )
            }
            LibraryDefinitionError::NameCollision { kind, name } => {
                let kind = match kind {
                    CallKind::Function => "Function",
                    CallKind::Action => "Action",
                };
                write!(
                    f,
                    "[createLibrary] {kind} \"{name}\" collides with a built-in, component, function, or action of the same name. Rename it."
                )
            }
            LibraryDefinitionError::UnknownAction { path, name } => {
                let name = crate::stringify_json(&Json::String(name.clone()));
                write!(
                    f,
                    "library definition: {path}: {name} is not an action a slot can name (the built-in ones are {}, and the library's own actions)",
                    SLOT_ACTIONS.join(" and ")
                )
            }
            LibraryDefinitionError::Invalid { path, problem } if path.is_empty() => {
                write!(f, "library definition: {problem}")
            }
            LibraryDefinitionError::Invalid { path, problem } => {
                write!(f, "library definition: {path}: {problem}")
            }
        }
    }
}

impl std::error::Error for LibraryDefinitionError {}

/// Where a value sits in the definition, built into text only for an error.
#[derive(Clone, Copy)]
enum Path<'a> {
    Root,
    Field(&'a Path<'a>, &'a str),
    Index(&'a Path<'a>, usize),
}

impl Path<'_> {
    /// The path as text, built with a loop: a path is as long as the
    /// definition is deep.
    fn text(&self) -> String {
        let mut steps = Vec::new();
        let mut at = self;
        loop {
            match at {
                Path::Root => break,
                Path::Field(parent, _) | Path::Index(parent, _) => {
                    steps.push(at);
                    at = parent;
                }
            }
        }
        let mut out = String::new();
        for step in steps.into_iter().rev() {
            match step {
                Path::Field(Path::Root, name) => out.push_str(name),
                Path::Field(_, name) => {
                    out.push('.');
                    out.push_str(name);
                }
                Path::Index(_, i) => out.push_str(&format!("[{i}]")),
                Path::Root => {}
            }
        }
        out
    }

    fn invalid(&self, problem: impl Into<String>) -> LibraryDefinitionError {
        LibraryDefinitionError::Invalid {
            path: self.text(),
            problem: problem.into(),
        }
    }
}

type Read<T> = Result<T, LibraryDefinitionError>;

/// An object with only the given fields.
fn object<'a>(value: &'a Json, at: Path<'_>, fields: &[&str]) -> Read<&'a Map<String, Json>> {
    let map = value
        .as_object()
        .ok_or_else(|| at.invalid("expected an object"))?;
    if let Some(unknown) = map.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(Path::Field(&at, unknown).invalid("unknown field"));
    }
    Ok(map)
}

fn required<'a>(map: &'a Map<String, Json>, at: Path<'_>, field: &str) -> Read<&'a Json> {
    map.get(field)
        .ok_or_else(|| Path::Field(&at, field).invalid("missing"))
}

fn string(value: &Json, at: Path<'_>) -> Read<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| at.invalid("expected a string"))
}

fn array<'a>(value: &'a Json, at: Path<'_>) -> Read<&'a [Json]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| at.invalid("expected an array"))
}

fn strings(value: &Json, at: Path<'_>) -> Read<Vec<String>> {
    array(value, at)?
        .iter()
        .enumerate()
        .map(|(i, item)| string(item, Path::Index(&at, i)))
        .collect()
}

/// An optional field read with `read`, absent when missing.
fn optional<T>(
    map: &Map<String, Json>,
    at: Path<'_>,
    field: &str,
    read: impl FnOnce(&Json, Path<'_>) -> Read<T>,
) -> Read<Option<T>> {
    map.get(field)
        .map(|value| read(value, Path::Field(&at, field)))
        .transpose()
}

/// How deep a type is, and how deep one may be.
#[derive(Clone, Copy)]
struct Depth {
    level: usize,
    limit: usize,
}

impl Depth {
    /// A prop's type: the first level.
    fn top(limits: Limits) -> Depth {
        Depth {
            level: 1,
            limit: limits.depth(),
        }
    }

    /// A type inside one at this depth.
    fn inner(self) -> Depth {
        Depth {
            level: self.level.saturating_add(1),
            ..self
        }
    }
}

/// A list of named entries (props, or an object's fields), each name once.
fn named_list<T>(
    value: &Json,
    at: Path<'_>,
    what: &str,
    read: impl Fn(&Json, Path<'_>) -> Read<T>,
    name: impl Fn(&T) -> &str,
) -> Read<Vec<T>> {
    let items = array(value, at)?;
    let mut list: Vec<T> = Vec::with_capacity(items.len());
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let item_at = Path::Index(&at, i);
        let entry = read(item, item_at)?;
        let entry_name = name(&entry);
        if !seen.insert(entry_name.to_owned()) {
            let problem = format!(
                "duplicate {what} {}",
                crate::stringify_json(&Json::String(entry_name.to_owned()))
            );
            return Err(Path::Field(&item_at, "name").invalid(problem));
        }
        list.push(entry);
    }
    Ok(list)
}

/// A boolean field, `false` when missing.
fn flag(map: &Map<String, Json>, at: Path<'_>, field: &str) -> Read<bool> {
    Ok(optional(map, at, field, |value, at| {
        value
            .as_bool()
            .ok_or_else(|| at.invalid("expected a boolean"))
    })?
    .unwrap_or(false))
}

/// A prop's or a field's `name`, `type` and `optional`, and a prop's
/// `binding` (`fields` names the fields it may have).
fn named_type(
    value: &Json,
    at: Path<'_>,
    depth: Depth,
    fields: &[&str],
) -> Read<(String, TypeDescription, bool, bool)> {
    let map = object(value, at, fields)?;
    let name = string(required(map, at, "name")?, Path::Field(&at, "name"))?;
    let ty =
        TypeDescription::from_json(required(map, at, "type")?, Path::Field(&at, "type"), depth)?;
    let ty = match map.get("default") {
        Some(value) => TypeDescription::Default(Box::new(ty), clone_json(value).into()),
        None => ty,
    };
    Ok((
        name,
        ty,
        flag(map, at, "optional")?,
        flag(map, at, "binding")?,
    ))
}

const PROP_FIELDS: [&str; 5] = ["name", "type", "optional", "binding", "default"];
const FIELD_FIELDS: [&str; 4] = ["name", "type", "optional", "default"];

/// The built-in actions a slot can name: upstream's `steps`, the ones with
/// an action shape of their own.
pub(crate) const SLOT_ACTIONS: [&str; 2] = ["OpenUrl", "ToAssistant"];

/// Entries in JavaScript property order, as a zod shape or an enum's entries
/// iterate them: names that are array indices first, ascending, and a name
/// given twice in its first place with its last entry.
pub(crate) fn in_property_order<'a, T>(
    items: &'a [T],
    name: impl Fn(&'a T) -> &'a str,
) -> Vec<&'a T> {
    let mut shape: JsObject<&T> = JsObject::with_capacity(items.len());
    for item in items {
        shape.define(name(item), item);
    }
    shape.values().copied().collect()
}

impl LibraryDefinition {
    /// Read a library definition from its plain-data JSON (see the module
    /// docs for the shape) under the default [`Limits`]. Refuses a kind this
    /// version doesn't support with [`LibraryDefinitionError::UnsupportedKind`],
    /// types nested too deep with [`LibraryDefinitionError::DepthLimited`],
    /// and anything else not in the form with
    /// [`LibraryDefinitionError::Invalid`], saying where.
    pub fn from_json(value: &Json) -> Result<LibraryDefinition, LibraryDefinitionError> {
        LibraryDefinition::from_json_with_limits(value, Limits::DEFAULT)
    }

    /// [`LibraryDefinition::from_json`] under `limits`: a type may nest
    /// [`Limits::depth`] levels, a prop's own type being the first.
    pub fn from_json_with_limits(
        value: &Json,
        limits: Limits,
    ) -> Result<LibraryDefinition, LibraryDefinitionError> {
        let at = Path::Root;
        let map = object(
            value,
            at,
            &[
                "components",
                "componentGroups",
                "functions",
                "actions",
                "root",
                "id",
            ],
        )?;
        let components_at = Path::Field(&at, "components");
        let components = array(required(map, at, "components")?, components_at)?
            .iter()
            .enumerate()
            .map(|(i, c)| ComponentDefinition::from_json(c, Path::Index(&components_at, i), limits))
            .collect::<Read<_>>()?;
        let component_groups = optional(map, at, "componentGroups", |value, at| {
            array(value, at)?
                .iter()
                .enumerate()
                .map(|(i, g)| ComponentGroup::from_json(g, Path::Index(&at, i)))
                .collect()
        })?;
        let functions = optional(map, at, "functions", |value, at| {
            array(value, at)?
                .iter()
                .enumerate()
                .map(|(i, f)| FunctionDefinition::from_json(f, Path::Index(&at, i), limits))
                .collect()
        })?
        .unwrap_or_default();
        let actions = optional(map, at, "actions", |value, at| {
            array(value, at)?
                .iter()
                .enumerate()
                .map(|(i, a)| ActionDefinition::from_json(a, Path::Index(&at, i), limits))
                .collect()
        })?
        .unwrap_or_default();
        Ok(LibraryDefinition {
            components,
            component_groups,
            functions,
            actions,
            root: optional(map, at, "root", string)?,
            id: optional(map, at, "id", string)?,
        })
    }
}

impl ComponentDefinition {
    fn from_json(value: &Json, at: Path<'_>, limits: Limits) -> Read<ComponentDefinition> {
        let map = object(value, at, &["name", "description", "props"])?;
        let name = string(required(map, at, "name")?, Path::Field(&at, "name"))?;
        let description = string(
            required(map, at, "description")?,
            Path::Field(&at, "description"),
        )?;
        let props = named_list(
            required(map, at, "props")?,
            Path::Field(&at, "props"),
            "prop",
            |value, at| {
                let (name, ty, optional, binding) =
                    named_type(value, at, Depth::top(limits), &PROP_FIELDS)?;
                Ok(PropDefinition {
                    name,
                    ty,
                    optional,
                    binding,
                })
            },
            |prop| &prop.name,
        )?;
        Ok(ComponentDefinition {
            name,
            description,
            props,
        })
    }
}

/// A call's params: an ordered list of fields, as an inline object's.
fn params(value: &Json, at: Path<'_>, limits: Limits) -> Read<Vec<FieldDefinition>> {
    named_list(
        value,
        at,
        "param",
        |value, at| {
            let (name, ty, optional, _) = named_type(value, at, Depth::top(limits), &FIELD_FIELDS)?;
            Ok(FieldDefinition { name, ty, optional })
        },
        |field| &field.name,
    )
}

/// A call's `name` and `description`.
fn call_header(map: &Map<String, Json>, at: Path<'_>) -> Read<(String, String)> {
    Ok((
        string(required(map, at, "name")?, Path::Field(&at, "name"))?,
        string(
            required(map, at, "description")?,
            Path::Field(&at, "description"),
        )?,
    ))
}

impl FunctionDefinition {
    fn from_json(value: &Json, at: Path<'_>, limits: Limits) -> Read<FunctionDefinition> {
        let map = object(value, at, &["name", "description", "params", "returns"])?;
        let (name, description) = call_header(map, at)?;
        let params = params(
            required(map, at, "params")?,
            Path::Field(&at, "params"),
            limits,
        )?;
        let returns = optional(map, at, "returns", |value, at| {
            TypeDescription::from_json(value, at, Depth::top(limits))
        })?;
        Ok(FunctionDefinition {
            name,
            description,
            params,
            returns,
        })
    }
}

impl ActionDefinition {
    fn from_json(value: &Json, at: Path<'_>, limits: Limits) -> Read<ActionDefinition> {
        let map = object(value, at, &["name", "description", "params"])?;
        let (name, description) = call_header(map, at)?;
        let params = params(
            required(map, at, "params")?,
            Path::Field(&at, "params"),
            limits,
        )?;
        Ok(ActionDefinition {
            name,
            description,
            params,
        })
    }
}

impl TypeDescription {
    fn from_json(value: &Json, at: Path<'_>, depth: Depth) -> Read<TypeDescription> {
        if depth.level > depth.limit {
            return Err(LibraryDefinitionError::DepthLimited {
                path: at.text(),
                limit: depth.limit,
            });
        }
        crate::stack::grow_stack!(TypeDescription::from_json(value, at, depth));
        let map = value
            .as_object()
            .ok_or_else(|| at.invalid("expected an object"))?;
        let kind = required(map, at, "kind")?;
        let kind = kind
            .as_str()
            .ok_or_else(|| Path::Field(&at, "kind").invalid("expected a string"))?;
        // Each kind names the fields it has; a new kind is a new arm.
        let fields: &[&str] = match kind {
            "string" | "number" | "boolean" | "any" | "unknown" => &["kind"],
            "enum" | "record" => &["kind", "values"],
            "array" => &["kind", "items", "minItems"],
            "union" => &["kind", "options"],
            "object" => &["kind", "fields"],
            "nullable" => &["kind", "type"],
            "component" | "action" => &["kind", "name"],
            "named" => &["kind", "name", "type"],
            _ => {
                return Err(LibraryDefinitionError::UnsupportedKind {
                    path: at.text(),
                    kind: kind.to_owned(),
                });
            }
        };
        object(value, at, fields)?;
        // The kind's other field, and where it is.
        let field = |name: &'static str| required(map, at, name).map(|v| (v, name));
        let inner = |name: &'static str| -> Read<Box<TypeDescription>> {
            let (value, name) = field(name)?;
            TypeDescription::from_json(value, Path::Field(&at, name), depth.inner()).map(Box::new)
        };
        Ok(match kind {
            "string" => TypeDescription::String,
            "number" => TypeDescription::Number,
            "boolean" => TypeDescription::Boolean,
            "unknown" => TypeDescription::Unknown,
            "enum" => {
                let (value, name) = field("values")?;
                TypeDescription::Enum(strings(value, Path::Field(&at, name))?)
            }
            "array" => {
                let items = inner("items")?;
                match map.get("minItems") {
                    None => TypeDescription::Array(items),
                    Some(Json::Number(min)) => {
                        TypeDescription::ArrayWithMinimum(items, min.clone())
                    }
                    Some(_) => {
                        return Err(Path::Field(&at, "minItems").invalid("expected a number"));
                    }
                }
            }
            "record" => TypeDescription::Record(inner("values")?),
            "nullable" => TypeDescription::Nullable(inner("type")?),
            "component" => {
                let (value, name) = field("name")?;
                TypeDescription::Component(string(value, Path::Field(&at, name))?)
            }
            "action" => TypeDescription::Action(optional(map, at, "name", string)?),
            "named" => {
                let (value, name) = field("name")?;
                let name = string(value, Path::Field(&at, name))?;
                TypeDescription::Named(name, inner("type")?)
            }
            "union" => {
                let (value, name) = field("options")?;
                let options_at = Path::Field(&at, name);
                let options = array(value, options_at)?
                    .iter()
                    .enumerate()
                    .map(|(i, option)| {
                        TypeDescription::from_json(
                            option,
                            Path::Index(&options_at, i),
                            depth.inner(),
                        )
                    })
                    .collect::<Read<_>>()?;
                TypeDescription::Union(options)
            }
            "object" => {
                let (value, name) = field("fields")?;
                let fields = named_list(
                    value,
                    Path::Field(&at, name),
                    "field",
                    |value, at| {
                        let (name, ty, optional, _) =
                            named_type(value, at, depth.inner(), &FIELD_FIELDS)?;
                        Ok(FieldDefinition { name, ty, optional })
                    },
                    |field| &field.name,
                )?;
                TypeDescription::Object(fields)
            }
            // "any": every other kind was refused above.
            _ => TypeDescription::Any,
        })
    }
}

/// The names a reference can name: the definition's components and its
/// own actions.
struct Names<'a> {
    components: std::collections::HashSet<&'a str>,
    actions: std::collections::HashSet<&'a str>,
}

/// The upstream action name the core refuses for a library action: upstream
/// registers it under the built-in action slot's id.
pub(crate) const ACTION_SLOT_ID: &str = "ActionExpression";

/// Checks the kinds that link a library together, as
/// [`create_library`](crate::create_library) does: each component reference
/// names a component of the definition (any of them; the last of a name is
/// the one meant) other than the empty name, each action slot's name is one
/// a slot can name (a built-in one, or one of the library's own other than
/// the empty name), and a named type names a data type; and no library
/// action is named `ActionExpression`. Walks every type of every component,
/// function and action once, as deep as it nests, growing the stack on
/// demand.
pub(crate) fn check_references(definition: &LibraryDefinition) -> Read<()> {
    let names = Names {
        components: definition
            .components
            .iter()
            .map(|c| c.name.as_str())
            .collect(),
        actions: definition.actions.iter().map(|a| a.name.as_str()).collect(),
    };
    let at = Path::Root;
    let components_at = Path::Field(&at, "components");
    for (i, component) in definition.components.iter().enumerate() {
        let component_at = Path::Index(&components_at, i);
        let props_at = Path::Field(&component_at, "props");
        for (j, prop) in component.props.iter().enumerate() {
            let prop_at = Path::Index(&props_at, j);
            check_type(&prop.ty, Path::Field(&prop_at, "type"), &names)?;
        }
    }
    let check_params = |params: &[FieldDefinition], params_at: Path<'_>| -> Read<()> {
        for (j, param) in params.iter().enumerate() {
            let param_at = Path::Index(&params_at, j);
            check_type(&param.ty, Path::Field(&param_at, "type"), &names)?;
        }
        Ok(())
    };
    let functions_at = Path::Field(&at, "functions");
    for (i, function) in definition.functions.iter().enumerate() {
        let function_at = Path::Index(&functions_at, i);
        check_params(&function.params, Path::Field(&function_at, "params"))?;
        if let Some(returns) = &function.returns {
            check_type(returns, Path::Field(&function_at, "returns"), &names)?;
        }
    }
    let actions_at = Path::Field(&at, "actions");
    for (i, action) in definition.actions.iter().enumerate() {
        let action_at = Path::Index(&actions_at, i);
        if action.name == ACTION_SLOT_ID {
            return Err(Path::Field(&action_at, "name").invalid(
                "an action named \"ActionExpression\" is not supported (upstream registers it under the action slot's id)",
            ));
        }
        check_params(&action.params, Path::Field(&action_at, "params"))?;
    }
    Ok(())
}

fn check_type(ty: &TypeDescription, at: Path<'_>, names: &Names<'_>) -> Read<()> {
    crate::stack::grow_stack!(check_type(ty, at, names));
    match ty {
        TypeDescription::Component(name) if name.is_empty() => {
            Err(at.invalid("a reference to the component with the empty name is not supported"))
        }
        TypeDescription::Component(name) if !names.components.contains(name.as_str()) => {
            Err(LibraryDefinitionError::UnknownComponent {
                path: at.text(),
                name: name.clone(),
            })
        }
        TypeDescription::Action(Some(name)) if SLOT_ACTIONS.contains(&name.as_str()) => Ok(()),
        TypeDescription::Action(Some(name)) if name.is_empty() && names.actions.contains("") => {
            Err(at.invalid("a reference to the action with the empty name is not supported"))
        }
        TypeDescription::Action(Some(name)) if !names.actions.contains(name.as_str()) => {
            Err(LibraryDefinitionError::UnknownAction {
                path: at.text(),
                name: name.clone(),
            })
        }
        TypeDescription::Component(_)
        | TypeDescription::Action(_)
        | TypeDescription::String
        | TypeDescription::Number
        | TypeDescription::Boolean
        | TypeDescription::Any
        | TypeDescription::Unknown
        | TypeDescription::Enum(_) => Ok(()),
        TypeDescription::Named(_, inner) => {
            let inner_at = Path::Field(&at, "type");
            if matches!(
                **inner,
                TypeDescription::Component(_)
                    | TypeDescription::Action(_)
                    | TypeDescription::Named(..)
            ) {
                return Err(inner_at.invalid(
                    "a named type names a data type, not a component reference, an action slot or another named type",
                ));
            }
            check_type(inner, inner_at, names)
        }
        TypeDescription::Array(inner) | TypeDescription::ArrayWithMinimum(inner, _) => {
            check_type(inner, Path::Field(&at, "items"), names)
        }
        TypeDescription::Record(inner) => check_type(inner, Path::Field(&at, "values"), names),
        TypeDescription::Default(inner, _) => check_type(inner, at, names),
        TypeDescription::Nullable(inner) => check_type(inner, Path::Field(&at, "type"), names),
        TypeDescription::Union(options) => {
            let options_at = Path::Field(&at, "options");
            options
                .iter()
                .enumerate()
                .try_for_each(|(i, option)| check_type(option, Path::Index(&options_at, i), names))
        }
        TypeDescription::Object(fields) => {
            let fields_at = Path::Field(&at, "fields");
            fields.iter().enumerate().try_for_each(|(i, field)| {
                let field_at = Path::Index(&fields_at, i);
                check_type(&field.ty, Path::Field(&field_at, "type"), names)
            })
        }
    }
}

impl ComponentGroup {
    fn from_json(value: &Json, at: Path<'_>) -> Read<ComponentGroup> {
        let map = object(value, at, &["name", "components", "notes"])?;
        Ok(ComponentGroup {
            name: string(required(map, at, "name")?, Path::Field(&at, "name"))?,
            components: strings(
                required(map, at, "components")?,
                Path::Field(&at, "components"),
            )?,
            notes: optional(map, at, "notes", strings)?,
        })
    }
}
