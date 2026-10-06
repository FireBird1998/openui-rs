//! The component library, compiled from JSON Schema as the reference parser's
//! `compileSchema` does.

use indexmap::IndexMap;
use serde_json::Value as Json;

/// One component parameter, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub required: bool,
    /// The schema's `default`, used when a required prop is missing or null.
    pub default: Option<Json>,
    /// The raw JSON Schema fragment for this param.
    pub schema: Json,
}

/// A component's parameters, in positional order.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub params: Vec<Param>,
}

/// The components a program may use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    components: IndexMap<String, Component>,
}

impl Library {
    /// Compile the JSON Schema produced by `library.toJSONSchema()`: one
    /// component per `$defs` entry, params in `properties` order.
    pub fn from_json_schema(schema: &Json) -> Library {
        let mut components = IndexMap::new();
        if let Some(defs) = schema.get("$defs").and_then(Json::as_object) {
            for (name, def) in defs {
                let required: Vec<&str> = def
                    .get("required")
                    .and_then(Json::as_array)
                    .map(|r| r.iter().filter_map(Json::as_str).collect())
                    .unwrap_or_default();
                let params = def
                    .get("properties")
                    .and_then(Json::as_object)
                    .map(|props| {
                        props
                            .iter()
                            .map(|(key, prop)| Param {
                                name: key.clone(),
                                required: required.contains(&key.as_str()),
                                default: prop.as_object().and_then(|p| p.get("default")).cloned(),
                                schema: prop.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                components.insert(name.clone(), Component { params });
            }
        }
        Library { components }
    }

    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.get(name)
    }

    /// Component names in library order.
    pub fn component_names(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_str)
    }
}
