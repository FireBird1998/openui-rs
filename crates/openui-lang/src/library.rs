//! The component library, compiled from JSON Schema as the reference parser's
//! `compileSchema` does.

use indexmap::IndexMap;
use serde_json::Value as Json;

use crate::js::object_entries;

/// One component parameter, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// The prop name.
    pub name: String,
    /// Listed in the component's `required`.
    pub required: bool,
    /// The schema's `default`, used when a required prop is missing or null.
    pub default: Option<Json>,
    /// The raw JSON Schema fragment for this param.
    pub schema: Json,
}

/// A component's parameters, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// Params in positional order: the order of the schema's `properties`.
    pub params: Vec<Param>,
}

/// The components a program may use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    components: IndexMap<String, Component>,
}

impl Library {
    /// Compile the JSON Schema produced by `library.toJSONSchema()`: one
    /// component per `$defs` entry, params in `properties` order. Both are read
    /// with `Object.entries`, as the reference does: object keys in
    /// JavaScript's order (array-index keys like "0" and "12" first), and an
    /// array or string by index.
    pub fn from_json_schema(schema: &Json) -> Library {
        let mut components = IndexMap::new();
        let defs = schema.get("$defs").map(object_entries).unwrap_or_default();
        for (name, def) in defs {
            let required = def.get("required");
            // `required.includes(key)`: array membership, or a substring of a string.
            let is_required = |key: &str| match required {
                Some(Json::Array(items)) => items.iter().any(|r| r.as_str() == Some(key)),
                Some(Json::String(s)) => s.contains(key),
                _ => false,
            };
            let params = def
                .get("properties")
                .map(object_entries)
                .unwrap_or_default()
                .into_iter()
                .map(|(key, prop)| Param {
                    required: is_required(&key),
                    default: prop.as_object().and_then(|p| p.get("default")).cloned(),
                    name: key,
                    schema: prop,
                })
                .collect();
            components.insert(name, Component { params });
        }
        Library { components }
    }

    /// A component by name.
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.get(name)
    }

    /// Component names in library order.
    pub fn component_names(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_str)
    }
}
