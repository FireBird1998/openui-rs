//! A library definition in plain-data form: the declarations of a component
//! library's components, which [`create_library`](crate::create_library)
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
//! - A prop has a `name`, a `type` and `optional` (default `false`). A
//!   prop's name may appear once per component.
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
//!   | `{"kind": "enum", "values": ["a", "b"]}` | `z.enum(["a", "b"])` | `{"type": "string", "enum": ["a", "b"]}` | `"a" \| "b"` |
//!   | `{"kind": "array", "items": T}` | `z.array(T)` | `{"type": "array", "items": T}` | `T[]`, `(T)[]` for a union |
//!   | `{"kind": "union", "options": [T, U]}` | `z.union([T, U])` | `{"anyOf": [T, U]}` | `T \| U` |
//!   | `{"kind": "object", "fields": [F]}` | `z.object({...})` | as a component's props | `{a: T, b?: U}` |
//!   | `{"kind": "record", "values": T}` | `z.record(z.string(), T)` | `{"type": "object", "propertyNames": {"type": "string"}, "additionalProperties": T}` | `Record<string, T>` |
//!   | `{"kind": "nullable", "type": T}` | `T.nullable()` | `{"anyOf": [T, {"type": "null"}]}` | `T`, and the prop or field gets `?` |
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
//! - `root` (optional): must name a component, as upstream checks it.
//! - `id` (optional): copied into the prompt spec.
//!
//! A field the form doesn't have is refused, so a misspelt `"optinal"` or a
//! field from a later version is never silently ignored.
//!
//! # How later kinds extend it
//!
//! Each new kind is a new `kind` value with its own fields, read by one new
//! arm in [`TypeDescription`]'s reader, and a new variant of the
//! `#[non_exhaustive]` enum; facts about a prop rather than its type become
//! new optional prop fields. Planned (spec #123), each an addition:
//!
//! ```json
//! {"kind": "component", "name": "ListItem"}
//! {"kind": "action"}
//! {"kind": "named", "name": "Color", "type": T}
//! ```
//!
//! and on a prop, `"binding": true` (a binding marks the outermost type only,
//! so it belongs to the prop).
//!
//! # Limits
//!
//! Types nest, so reading counts how deep: a prop's type is one level, and
//! each type inside another one more. A type deeper than
//! [`Limits::depth`](crate::Limits::depth) (256 by default) is refused with
//! [`LibraryDefinitionError::DepthLimited`] before it is read, so reading
//! recurses at most that deep, growing the stack on demand as the parser's
//! walks do (ADR 0002). A definition at the default limit fits within the
//! JSON depth [`crate::parse_json`] reads, and its JSON Schema and prompt
//! spec within the depth the prompt entry points accept. Writing the schema
//! and the signatures walks the types read, growing the stack the same way,
//! and [`DefinedLibrary::library`](crate::DefinedLibrary::library) compiles
//! the schema on a stack segment sized for the definition's depth. Otherwise
//! a definition's time and memory grow with its size, nothing more; it is
//! already in the caller's memory. The binding entry points, which arrive
//! with #137, are to cap its size as they cap other host input (ADR 0001).
//!
//! What is handed back is freed by the caller, and some of it recursively:
//! the JSON Schema and prompt spec are `serde_json` values, whose drop
//! recurses, and so is the [`Library`](crate::Library) from `library()`,
//! which holds each prop's schema. At the default limit these drops fit a
//! 64 KiB stack in optimized builds (opt-level 1, the test profile, and
//! release), but not at opt-level 0, where the schema of a 256-deep array
//! overflows 64 KiB and the `Library` of a 256-deep object needs up to
//! 256 KiB. On a small stack free the JSON with [`crate::free_json`]; the
//! `Library` has no such way out. A definition itself, a tree of boxed
//! types, is also freed by Rust's recursive drop. So reading under a raised
//! depth limit is unsupported for definitions nested that deep unless the
//! caller's thread has the stack for those drops: free the JSON with
//! `free_json`, and drop the definition and the `Library` on a large stack.
use std::fmt;

use serde_json::{Map, Value as Json};

use crate::Limits;
use crate::js::JsObject;

/// A component library's declarations, in plain-data form. Read one from its
/// JSON with [`LibraryDefinition::from_json`] (the module docs give the
/// shape), then build the library with [`crate::create_library`].
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryDefinition {
    /// The components, in order.
    pub components: Vec<ComponentDefinition>,
    /// The prompt's component groups, if any.
    pub component_groups: Option<Vec<ComponentGroup>>,
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

/// One prop of a component.
#[derive(Clone, Debug, PartialEq)]
pub struct PropDefinition {
    /// The prop's name.
    pub name: String,
    /// What the prop holds.
    pub ty: TypeDescription,
    /// Whether a program may leave it out (zod's `.optional()`).
    pub optional: bool,
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
    /// `z.enum([...])`: one of these words. They are written in JavaScript
    /// property order, each once, as zod keeps them.
    Enum(Vec<String>),
    /// `z.array(T)`.
    Array(Box<TypeDescription>),
    /// `z.union([...])`, its options in order.
    Union(Vec<TypeDescription>),
    /// `z.object({...})`, written inline, its fields in positional order.
    Object(Vec<FieldDefinition>),
    /// `z.record(z.string(), T)`: string keys, values of this type.
    Record(Box<TypeDescription>),
    /// `T.nullable()`: this type or null.
    Nullable(Box<TypeDescription>),
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

/// A prop's or a field's `name`, `type` and `optional`.
fn named_type(value: &Json, at: Path<'_>, depth: Depth) -> Read<(String, TypeDescription, bool)> {
    let map = object(value, at, &["name", "type", "optional"])?;
    let name = string(required(map, at, "name")?, Path::Field(&at, "name"))?;
    let ty =
        TypeDescription::from_json(required(map, at, "type")?, Path::Field(&at, "type"), depth)?;
    let optional = optional(map, at, "optional", |value, at| {
        value
            .as_bool()
            .ok_or_else(|| at.invalid("expected a boolean"))
    })?
    .unwrap_or(false);
    Ok((name, ty, optional))
}

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
        let map = object(value, at, &["components", "componentGroups", "root", "id"])?;
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
        Ok(LibraryDefinition {
            components,
            component_groups,
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
                let (name, ty, optional) = named_type(value, at, Depth::top(limits))?;
                Ok(PropDefinition { name, ty, optional })
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
            "string" | "number" | "boolean" | "any" => &["kind"],
            "enum" | "record" => &["kind", "values"],
            "array" => &["kind", "items"],
            "union" => &["kind", "options"],
            "object" => &["kind", "fields"],
            "nullable" => &["kind", "type"],
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
            "enum" => {
                let (value, name) = field("values")?;
                TypeDescription::Enum(strings(value, Path::Field(&at, name))?)
            }
            "array" => TypeDescription::Array(inner("items")?),
            "record" => TypeDescription::Record(inner("values")?),
            "nullable" => TypeDescription::Nullable(inner("type")?),
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
                        let (name, ty, optional) = named_type(value, at, depth.inner())?;
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
