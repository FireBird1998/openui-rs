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
//! - A type is an object with a `kind`. This version supports `"string"`,
//!   `"number"`, `"boolean"` and `"any"`, with no other field. Any other kind
//!   is refused with [`LibraryDefinitionError::UnsupportedKind`], naming it.
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
//! {"kind": "enum", "values": ["info", "warn"]}
//! {"kind": "array", "items": T}
//! {"kind": "union", "options": [T, T]}
//! {"kind": "object", "fields": [{"name": "x", "type": T, "optional": true}]}
//! {"kind": "record", "values": T}
//! {"kind": "nullable", "type": T}
//! {"kind": "component", "name": "ListItem"}
//! {"kind": "action"}
//! {"kind": "named", "name": "Color", "type": T}
//! ```
//!
//! and on a prop, `"binding": true` (a binding marks the outermost type only,
//! so it belongs to the prop). Nullable is a kind, separate from `optional`:
//! upstream keeps a nullable prop in `required`. The recursive kinds bring
//! nesting; their reader will count depth against [`crate::Limits`] then.
//!
//! # Limits
//!
//! Reading a definition never recurses into the input: unknown fields are
//! refused unread, and every supported value is a string, a boolean or an
//! array of them. Its time and memory grow with the definition, nothing more;
//! the definition is already in the caller's memory. The binding entry
//! points, which arrive with #137, are to cap its size as they cap other host
//! input (ADR 0001).

use std::fmt;

use serde_json::{Map, Value as Json};

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
    fn text(&self) -> String {
        match self {
            Path::Root => String::new(),
            Path::Field(Path::Root, name) => (*name).to_owned(),
            Path::Field(parent, name) => format!("{}.{name}", parent.text()),
            Path::Index(parent, i) => format!("{}[{i}]", parent.text()),
        }
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

impl LibraryDefinition {
    /// Read a library definition from its plain-data JSON (see the module
    /// docs for the shape). Refuses a kind this version doesn't support with
    /// [`LibraryDefinitionError::UnsupportedKind`], and anything else not in
    /// the form with [`LibraryDefinitionError::Invalid`], saying where.
    pub fn from_json(value: &Json) -> Result<LibraryDefinition, LibraryDefinitionError> {
        let at = Path::Root;
        let map = object(value, at, &["components", "componentGroups", "root", "id"])?;
        let components_at = Path::Field(&at, "components");
        let components = array(required(map, at, "components")?, components_at)?
            .iter()
            .enumerate()
            .map(|(i, c)| ComponentDefinition::from_json(c, Path::Index(&components_at, i)))
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
    fn from_json(value: &Json, at: Path<'_>) -> Read<ComponentDefinition> {
        let map = object(value, at, &["name", "description", "props"])?;
        let name = string(required(map, at, "name")?, Path::Field(&at, "name"))?;
        let description = string(
            required(map, at, "description")?,
            Path::Field(&at, "description"),
        )?;
        let props_at = Path::Field(&at, "props");
        let items = array(required(map, at, "props")?, props_at)?;
        let mut props: Vec<PropDefinition> = Vec::with_capacity(items.len());
        let mut seen = std::collections::HashSet::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let prop_at = Path::Index(&props_at, i);
            let prop = PropDefinition::from_json(item, prop_at)?;
            if !seen.insert(prop.name.clone()) {
                let problem = format!(
                    "duplicate prop {}",
                    crate::stringify_json(&Json::String(prop.name))
                );
                return Err(Path::Field(&prop_at, "name").invalid(problem));
            }
            props.push(prop);
        }
        Ok(ComponentDefinition {
            name,
            description,
            props,
        })
    }
}

impl PropDefinition {
    fn from_json(value: &Json, at: Path<'_>) -> Read<PropDefinition> {
        let map = object(value, at, &["name", "type", "optional"])?;
        Ok(PropDefinition {
            name: string(required(map, at, "name")?, Path::Field(&at, "name"))?,
            ty: TypeDescription::from_json(required(map, at, "type")?, Path::Field(&at, "type"))?,
            optional: optional(map, at, "optional", |value, at| {
                value
                    .as_bool()
                    .ok_or_else(|| at.invalid("expected a boolean"))
            })?
            .unwrap_or(false),
        })
    }
}

impl TypeDescription {
    fn from_json(value: &Json, at: Path<'_>) -> Read<TypeDescription> {
        let map = value
            .as_object()
            .ok_or_else(|| at.invalid("expected an object"))?;
        let kind = required(map, at, "kind")?;
        let kind = kind
            .as_str()
            .ok_or_else(|| Path::Field(&at, "kind").invalid("expected a string"))?;
        // Each kind names the fields it has; a new kind is a new arm.
        let (ty, fields): (TypeDescription, &[&str]) = match kind {
            "string" => (TypeDescription::String, &["kind"]),
            "number" => (TypeDescription::Number, &["kind"]),
            "boolean" => (TypeDescription::Boolean, &["kind"]),
            "any" => (TypeDescription::Any, &["kind"]),
            _ => {
                return Err(LibraryDefinitionError::UnsupportedKind {
                    path: at.text(),
                    kind: kind.to_owned(),
                });
            }
        };
        object(value, at, fields)?;
        Ok(ty)
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
