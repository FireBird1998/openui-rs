// A library with every schema shape validation handles; shared by the targets.

use std::sync::{Arc, LazyLock};

use openui_lang::Library;

pub static LIBRARY: LazyLock<Arc<Library>> = LazyLock::new(|| {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"type": "string"}, "tone": {"enum": ["info", 2, null], "default": "info"}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array", "items": {"anyOf": [{"$ref": "#/$defs/Text"}]}}}},
        "Card": {"properties": {"meta": {"type": "object", "properties": {"id": {"type": "string"}, "0": {"const": 1}}, "required": ["id"]}, "rows": {"type": "array", "items": {"type": "number"}}}, "required": ["meta"]},
        "10": {"properties": {"b": {}, "0": {"type": "boolean", "default": true}}, "required": ["0"]},
        "Button": {"properties": {"label": {"type": "string"}, "action": {"$ref": "#/$defs/ActionExpression"}, "share": {"anyOf": [{"$ref": "#/$defs/OpenUrl"}, {"$ref": "#/$defs/Copy"}]}}},
        "ActionExpression": {},
        "OpenUrl": {"properties": {"type": {"const": "open_url"}, "url": {"type": "string"}}}
    },
    // 1.0's library calls: a function and actions, one named like a component.
    "functions": {
        "Twice": {"params": {"type": "object", "properties": {"n": {"type": "number"}, "by": {"type": "number", "default": 2}}, "required": ["n"]}, "returns": {"type": "number"}}
    },
    "actions": {
        "Copy": {"params": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
        "Card": {"params": {"type": "object", "properties": {"id": {"type": "string"}}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
});
