//! A component library built from a [`LibraryDefinition`]: upstream's
//! `createLibrary` (its `library.ts`), with its `toJSONSchema()` and
//! `toSpec()`. Upstream asks zod 4 for the JSON Schema; this writes what zod
//! 4.6.7 (the version in the reference harness) writes for the equivalent
//! declaration, byte for byte, and builds the signatures as upstream's
//! `signature.ts` does ([`crate::signature`]).
//!
//! The parser never sees the definition: [`DefinedLibrary::library`] compiles
//! the JSON Schema with [`Library::from_json_schema`], the parser's one way
//! in, so a defined library parses exactly as its JSON would.
//!
//! What upstream does with JavaScript objects is copied too. The components
//! are a record keyed by name, so two of one name keep the first one's place
//! with the later one's declaration, and a component named `__proto__` is lost
//! (assigning it sets the record's prototype). The root check,
//! `!record[root]`, is a truthiness test: it accepts a name every object
//! inherits (`toString`), and, when there is a `__proto__` component, a field
//! of the last one, as `defineComponent` returns it: `name`, `props` and
//! `ref` always, `description` only when it isn't empty, and `component`, its
//! renderer. The plain-data form has no renderer; every real library has a
//! function there, so `component` is taken as present (the reference harness
//! gives its samples a function renderer so they compare like with like).
//! Names that are array indices come first, ascending, for components and
//! props alike.

use serde_json::{Map, Value as Json};

use crate::Library;
use crate::js::{JsObject, OBJECT_PROTOTYPE_KEYS};
use crate::library_definition::{
    ComponentDefinition, LibraryDefinition, LibraryDefinitionError, PropDefinition,
    TypeDescription, in_property_order,
};
use crate::signature::{enum_words, signature};

/// The `$schema` zod 4 writes.
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// The fields of the object upstream's `defineComponent` returns that are
/// always truthy, which a component named `__proto__` puts on the component
/// record's prototype. Its `description` is truthy only when it isn't empty;
/// `component`, the renderer, is taken as present (see the module docs).
const TRUTHY_DEFINED_COMPONENT_FIELDS: [&str; 4] = ["name", "props", "component", "ref"];

/// A component library made from a [`LibraryDefinition`] by
/// [`create_library`]: its JSON Schema, its prompt spec and the [`Library`]
/// the parser compiles from that JSON Schema.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinedLibrary {
    definition: LibraryDefinition,
    /// Indices into `definition.components` of the components the record
    /// keeps, in JavaScript property order.
    components: Vec<usize>,
}

/// Upstream's `createLibrary`: checks the root and makes the library.
///
/// Refuses a root that names no component with
/// [`LibraryDefinitionError::RootNotFound`], whose message is upstream's.
/// An empty root isn't checked, as upstream doesn't check it.
pub fn create_library(
    definition: LibraryDefinition,
) -> Result<DefinedLibrary, LibraryDefinitionError> {
    // `componentsRecord[comp.name] = comp`
    let mut record: JsObject<usize> = JsObject::with_capacity(definition.components.len());
    for (i, component) in definition.components.iter().enumerate() {
        record.set(component.name.as_str(), i);
    }
    // The record's prototype: the last component named `__proto__`.
    let prototype = definition
        .components
        .iter()
        .rev()
        .find(|c| c.name == "__proto__");
    if let Some(root) = definition.root.as_deref().filter(|root| !root.is_empty()) {
        // `!componentsRecord[input.root]`
        let found = record.has_own(root)
            || OBJECT_PROTOTYPE_KEYS.contains(&root)
            || prototype.is_some_and(|proto| {
                TRUTHY_DEFINED_COMPONENT_FIELDS.contains(&root)
                    || (root == "description" && !proto.description.is_empty())
            });
        if !found {
            return Err(LibraryDefinitionError::RootNotFound {
                root: root.to_owned(),
                available: record.iter().map(|(name, _)| name.clone()).collect(),
            });
        }
    }
    let components = record.values().copied().collect();
    Ok(DefinedLibrary {
        definition,
        components,
    })
}

impl DefinedLibrary {
    /// The definition it was made from.
    pub fn definition(&self) -> &LibraryDefinition {
        &self.definition
    }

    fn components(&self) -> impl Iterator<Item = &ComponentDefinition> {
        self.components
            .iter()
            .filter_map(|&i| self.definition.components.get(i))
    }

    /// Upstream's `toJSONSchema()`: the library's JSON Schema, keys in the
    /// order zod writes them.
    pub fn to_json_schema(&self) -> Json {
        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut defs = Map::new();
        for component in self.components() {
            let mut body = object_schema(
                ordered_props(&component.props)
                    .into_iter()
                    .map(|p| (p.name.as_str(), &p.ty, p.optional)),
            );
            required.push(Json::String(component.name.clone()));
            if component.name.is_empty() {
                // zod skips an empty id: the schema is written in place with
                // its `id`, it gets no `$defs` entry, and so upstream finds no
                // entry to merge the description into.
                body.insert("id".to_owned(), "".into());
                properties.insert(String::new(), Json::Object(body));
                continue;
            }
            // Upstream merges a non-empty description in after zod's keys.
            if !component.description.is_empty() {
                body.insert(
                    "description".to_owned(),
                    component.description.as_str().into(),
                );
            }
            let reference = format!("#/$defs/{}", json_pointer_token(&component.name));
            properties.insert(component.name.clone(), object([("$ref", reference.into())]));
            defs.insert(component.name.clone(), Json::Object(body));
        }
        let mut schema = Map::new();
        schema.insert("$schema".to_owned(), DRAFT_2020_12.into());
        schema.insert("type".to_owned(), "object".into());
        schema.insert("properties".to_owned(), Json::Object(properties));
        if !required.is_empty() {
            schema.insert("required".to_owned(), Json::Array(required));
        }
        schema.insert("additionalProperties".to_owned(), false.into());
        if !defs.is_empty() {
            schema.insert("$defs".to_owned(), Json::Object(defs));
        }
        Json::Object(schema)
    }

    /// Upstream's `toSpec()`: the prompt spec, with each component's
    /// signature built here, and the JSON Schema as its `schema`.
    pub fn to_spec(&self) -> Json {
        let mut spec = Map::new();
        if let Some(root) = &self.definition.root {
            spec.insert("root".to_owned(), root.as_str().into());
        }
        let components = self
            .components()
            .map(|component| {
                let entry = object([
                    (
                        "signature",
                        signature(&component.name, ordered_props(&component.props).into_iter())
                            .into(),
                    ),
                    ("description", component.description.as_str().into()),
                ]);
                (component.name.clone(), entry)
            })
            .collect();
        spec.insert("components".to_owned(), Json::Object(components));
        if let Some(groups) = &self.definition.component_groups {
            let groups = groups
                .iter()
                .map(|group| {
                    let mut entry = Map::new();
                    entry.insert("name".to_owned(), group.name.as_str().into());
                    entry.insert("components".to_owned(), strings(&group.components));
                    if let Some(notes) = &group.notes {
                        entry.insert("notes".to_owned(), strings(notes));
                    }
                    Json::Object(entry)
                })
                .collect();
            spec.insert("componentGroups".to_owned(), Json::Array(groups));
        }
        spec.insert("schema".to_owned(), self.to_json_schema());
        if let Some(id) = &self.definition.id {
            spec.insert("id".to_owned(), id.as_str().into());
        }
        Json::Object(spec)
    }

    /// The library the parser uses: [`Library::from_json_schema`] of
    /// [`DefinedLibrary::to_json_schema`].
    ///
    /// Compiling a schema clones and drops parts of it by recursing into
    /// them, without growing the stack, so the build, and the drop of the
    /// schema it builds, run on one stack segment sized for this
    /// definition's depth: enough for a definition at any depth its reader
    /// allowed, on any thread.
    pub fn library(&self) -> Library {
        let size = SEGMENT_BASE
            .saturating_add(schema_depth(self.type_depth()).saturating_mul(SEGMENT_PER_JSON_LEVEL));
        stacker::grow(size, || Library::from_json_schema(&self.to_json_schema()))
    }

    /// How deep the deepest prop's type nests, a prop's own type being one
    /// level, measured with a loop.
    fn type_depth(&self) -> usize {
        let mut open: Vec<(&TypeDescription, usize)> = self
            .components()
            .flat_map(|c| c.props.iter().map(|p| (&p.ty, 1)))
            .collect();
        let mut deepest = 0;
        while let Some((ty, depth)) = open.pop() {
            deepest = deepest.max(depth);
            let inner = depth.saturating_add(1);
            match ty {
                TypeDescription::Array(t)
                | TypeDescription::Record(t)
                | TypeDescription::Nullable(t) => open.push((t, inner)),
                TypeDescription::Union(options) => open.extend(options.iter().map(|t| (t, inner))),
                TypeDescription::Object(fields) => {
                    open.extend(fields.iter().map(|f| (&f.ty, inner)));
                }
                TypeDescription::String
                | TypeDescription::Number
                | TypeDescription::Boolean
                | TypeDescription::Any
                | TypeDescription::Enum(_) => {}
            }
        }
        deepest
    }
}

/// The stack [`DefinedLibrary::library`] reserves whatever the depth, and
/// for each level of JSON its schema nests. Measured (2026-11-21) on the
/// test suite's definitions at the default depth limit, compiled at
/// opt-level 0: 2 KiB a level with no base overflowed, 4 KiB a level was
/// enough, so this leaves at least four times the need. The pages are
/// reserved, not touched, so unused space costs address space only.
const SEGMENT_BASE: usize = 1024 * 1024;
const SEGMENT_PER_JSON_LEVEL: usize = 16 * 1024;

/// How many JSON levels the schema of types `type_depth` deep nests: each
/// type takes at most two (an object's `properties` and the field; a union's
/// `anyOf` and the option), under the schema's own few.
fn schema_depth(type_depth: usize) -> usize {
    type_depth.saturating_mul(2).saturating_add(8)
}

/// An object's entries (a component's props, or an inline object's fields,
/// already in property order) as zod writes its object schema: a nullable
/// entry stays in `required`, an optional one is left out.
fn object_schema<'a>(
    entries: impl Iterator<Item = (&'a str, &'a TypeDescription, bool)>,
) -> Map<String, Json> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, ty, optional) in entries {
        properties.insert(name.to_owned(), type_schema(ty));
        if !optional {
            required.push(Json::String(name.to_owned()));
        }
    }
    let mut schema = Map::new();
    schema.insert("type".to_owned(), "object".into());
    schema.insert("properties".to_owned(), Json::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_owned(), Json::Array(required));
    }
    schema.insert("additionalProperties".to_owned(), false.into());
    schema
}

/// What zod writes for a type. The walk goes as deep as the type, which
/// reading bounded (see the `library_definition` module docs), growing the
/// stack on demand.
fn type_schema(ty: &TypeDescription) -> Json {
    crate::stack::grow_stack!(type_schema(ty));
    match ty {
        TypeDescription::String => object([("type", "string".into())]),
        TypeDescription::Number => object([("type", "number".into())]),
        TypeDescription::Boolean => object([("type", "boolean".into())]),
        TypeDescription::Any => Json::Object(Map::new()),
        TypeDescription::Enum(values) => {
            let words = enum_words(values);
            if words.is_empty() {
                // An empty enum accepts nothing, as z.never().
                return object([("not", Json::Object(Map::new()))]);
            }
            let words = words.into_iter().map(|w| Json::String(w.to_owned()));
            object([
                ("type", "string".into()),
                ("enum", Json::Array(words.collect())),
            ])
        }
        TypeDescription::Array(items) => {
            object([("type", "array".into()), ("items", type_schema(items))])
        }
        TypeDescription::Union(options) => any_of(options.iter().map(type_schema).collect()),
        TypeDescription::Object(fields) => Json::Object(object_schema(
            in_property_order(fields, |f| &f.name)
                .into_iter()
                .map(|f| (f.name.as_str(), &f.ty, f.optional)),
        )),
        TypeDescription::Record(values) => object([
            ("type", "object".into()),
            ("propertyNames", object([("type", "string".into())])),
            ("additionalProperties", type_schema(values)),
        ]),
        TypeDescription::Nullable(inner) => {
            any_of(vec![type_schema(inner), object([("type", "null".into())])])
        }
    }
}

/// `{"anyOf": options}`, compacted as zod 4.6.7's `compactTypeUnion` does:
/// when every option is a bare type assertion (an object whose only key is
/// `type`, a string or a list of strings), the options become one `type`,
/// each type once in first-seen order, a single one as a string. The options
/// were compacted when they were written, as zod compacts each one first.
fn any_of(options: Vec<Json>) -> Json {
    let mut types: Vec<&str> = Vec::new();
    let bare = !options.is_empty()
        && options.iter().all(|option| {
            bare_types(option).is_some_and(|members| {
                for member in members {
                    if !types.contains(&member) {
                        types.push(member);
                    }
                }
                true
            })
        });
    if !bare {
        return object([("anyOf", Json::Array(options))]);
    }
    let ty = match types.as_slice() {
        [single] => Json::String((*single).to_owned()),
        _ => Json::Array(
            types
                .iter()
                .map(|t| Json::String((*t).to_owned()))
                .collect(),
        ),
    };
    object([("type", ty)])
}

/// The types a bare type assertion (`{"type": "string"}` or `{"type":
/// ["string", "null"]}`, nothing else) allows; `None` for anything else.
fn bare_types(option: &Json) -> Option<Vec<&str>> {
    let map = option.as_object().filter(|m| m.len() == 1)?;
    match map.get("type")? {
        Json::String(member) => Some(vec![member.as_str()]),
        Json::Array(members) => members.iter().map(Json::as_str).collect(),
        _ => None,
    }
}

/// The props in JavaScript property order, as a zod shape iterates them.
fn ordered_props(props: &[PropDefinition]) -> Vec<&PropDefinition> {
    in_property_order(props, |p| &p.name)
}

/// A name as one JSON Pointer reference token: `~` as `~0`, `/` as `~1`.
fn json_pointer_token(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains(['~', '/']) {
        name.replace('~', "~0").replace('/', "~1").into()
    } else {
        name.into()
    }
}

fn object<const N: usize>(entries: [(&str, Json); N]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::String(s.clone())).collect())
}
