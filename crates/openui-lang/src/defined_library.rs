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
    ComponentDefinition, LibraryDefinition, LibraryDefinitionError, PropDefinition, TypeDescription,
};
use crate::signature::signature;

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
            let mut body = component_schema(component);
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
                        signature(&component.name, ordered_props(&component.props)).into(),
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
    pub fn library(&self) -> Library {
        Library::from_json_schema(&self.to_json_schema())
    }
}

/// A component's props as zod writes their object schema.
fn component_schema(component: &ComponentDefinition) -> Map<String, Json> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for prop in ordered_props(&component.props) {
        properties.insert(prop.name.clone(), type_schema(&prop.ty));
        if !prop.optional {
            required.push(Json::String(prop.name.clone()));
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

/// What zod writes for a prop's type.
fn type_schema(ty: &TypeDescription) -> Json {
    match ty {
        TypeDescription::String => object([("type", "string".into())]),
        TypeDescription::Number => object([("type", "number".into())]),
        TypeDescription::Boolean => object([("type", "boolean".into())]),
        TypeDescription::Any => Json::Object(Map::new()),
    }
}

/// The props in JavaScript property order, as a zod shape iterates them.
fn ordered_props(props: &[PropDefinition]) -> impl Iterator<Item = &PropDefinition> {
    let mut shape: JsObject<&PropDefinition> = JsObject::with_capacity(props.len());
    for prop in props {
        shape.define(prop.name.as_str(), prop);
    }
    shape.values().copied().collect::<Vec<_>>().into_iter()
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
