// A library with every schema shape validation handles; shared by the targets.

use std::sync::{Arc, LazyLock};

use openui_lang::Library;

pub static LIBRARY: LazyLock<Arc<Library>> = LazyLock::new(|| {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"type": "string"}, "tone": {"enum": ["info", 2, null], "default": "info"}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array", "items": {"anyOf": [{"$ref": "#/$defs/Text"}]}}}},
        "Card": {"properties": {"meta": {"type": "object", "properties": {"id": {"type": "string"}, "0": {"const": 1}}, "required": ["id"]}, "rows": {"type": "array", "items": {"type": "number"}}}, "required": ["meta"]},
        "10": {"properties": {"b": {}, "0": {"type": "boolean", "default": true}}, "required": ["0"]}
    }});
    Arc::new(Library::from_json_schema(&schema))
});
