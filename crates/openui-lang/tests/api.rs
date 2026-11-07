//! Public helpers the conformance tests don't reach through parsing: the JSON
//! reader and writer, `JsObject`, walking an expression, and a library's
//! components and calls.

use openui_lang::{Ast, CallKind, JsObject, Library, Value, parse, parse_json, stringify_json};
use serde_json::json;

/// What `parse_json` reads, `stringify_json` writes back as JavaScript's
/// `JSON.stringify` would: keys in order, numbers in JavaScript's format, and
/// a lone surrogate as its escape.
#[test]
fn json_round_trips_in_javascripts_format() {
    let text = r#"{"b":1,"a":[1e21,0.5,-0,"\ud800",null,true],"c":{}}"#;
    let value = parse_json(text).unwrap();
    assert_eq!(
        stringify_json(&value),
        r#"{"b":1,"a":[1e+21,0.5,0,"\ud800",null,true],"c":{}}"#
    );
}

#[test]
fn json_that_is_cut_off_or_garbled_is_an_error_that_says_where() {
    for text in [r#"{"a":1 x"#, r#"[1, 2"#, r#"{"a" 1}"#, ""] {
        let error = parse_json(text).unwrap_err();
        assert!(!error.to_string().is_empty(), "{text}");
    }
    assert!(
        parse_json(r#"{"a":1 x"#)
            .unwrap_err()
            .to_string()
            .contains("expected , or }")
    );
}

#[test]
fn a_js_object_keeps_own_properties_including_proto() {
    let mut object: JsObject<i32> = JsObject::new();
    assert!(object.is_empty());
    object.define("__proto__", 1);
    object.define("b", 2);
    assert_eq!(object.len(), 2);
    assert!(!object.is_empty());
    assert_eq!(object.get("__proto__"), Some(&1));
}

/// A component called inside an expression keeps its args and also maps them
/// to props (`mappedProps`); walking the expression visits both.
#[test]
fn walking_an_expression_visits_mapped_props_too() {
    let schema = json!({"$defs": {"Text": {"properties": {"value": {}}}}});
    let library = Library::from_json_schema(&schema);
    let result = parse(
        "root = Text(@Each($rows, \"r\", Text(r.title)))",
        &library,
        None,
    );
    let root = result.root.expect("a root element");
    let Some(Value::Ast(expression)) = root.props.get("value") else {
        panic!("value isn't an expression: {:?}", root.props.get("value"));
    };
    let mut members = 0;
    expression.walk(&mut |node| {
        if matches!(node, Ast::Member { .. }) {
            members += 1;
        }
    });
    assert_eq!(
        members, 2,
        "r.title once in the call's args, once in its mapped props"
    );
}

/// A 1.0 library: components named by `properties`, action shapes kept out
/// of them and inlined into the slots that refer to them, and a call registry
/// of the built-ins plus the schema's own functions and actions.
#[test]
fn a_library_has_components_and_calls() {
    let schema = json!({
        "properties": {"Page": {}, "Widget": {}},
        "$defs": {
            "Page": {"properties": {"action": {"$ref": "#/$defs/ActionExpression"}, "ping": {"$ref": "#/$defs/Ping"}}},
            "Widget": {"properties": {"label": {"type": "string"}}},
            "Shape": {"properties": {"x": {}}},
            "ActionExpression": {"anyOf": [{"type": "object"}, {"$ref": "#/$defs/Ping"}]},
            "Ping": {"type": "object", "properties": {"target": {"type": "string"}}}
        },
        "functions": {"Twice": {"params": {"properties": {"n": {"type": "number", "default": 2}}, "required": ["n"]}}},
        "actions": {"Ping": {"params": {"properties": {"target": {"type": "string"}}}}}
    });
    let library = Library::from_json_schema(&schema);
    assert_eq!(
        library.component_names().collect::<Vec<_>>(),
        ["Page", "Widget"]
    );
    let page = library.component("Page").unwrap();
    assert_eq!(
        page.params[0].schema,
        json!({"anyOf": [{"type": "object"}, {"type": "object", "properties": {"target": {"type": "string"}}}]}),
        "an action slot holds its shape, with the shapes inside it inlined too"
    );
    assert_eq!(page.params[1].schema["type"], "object");

    let names: Vec<&str> = library.calls().map(|(name, _)| name).collect();
    assert_eq!(names.first(), Some(&"Count"));
    assert_eq!(&names[names.len() - 3..], ["Action", "Twice", "Ping"]);
    let each = library.call("Each").unwrap();
    assert!(each.builtin && each.lazy && each.kind == CallKind::Function);
    assert_eq!(library.call("Action").unwrap().params[0].name, "steps");
    let twice = library.call("Twice").unwrap();
    assert!(!twice.builtin && !twice.lazy && twice.kind == CallKind::Function);
    assert!(twice.params[0].required);
    assert_eq!(twice.params[0].default, Some(json!(2)));
    assert_eq!(library.call("Ping").unwrap().kind, CallKind::Action);
}

/// Where the reference throws building the library (documented in
/// CONFORMANCE.md), the core keeps going: a call without `params` has none,
/// and an action shape that refers to itself is left as the `$ref`.
#[test]
fn a_library_survives_schemas_the_reference_throws_on() {
    let schema = json!({
        "$defs": {
            "Page": {"properties": {"action": {"$ref": "#/$defs/ActionExpression"}}},
            "ActionExpression": {"anyOf": [{"$ref": "#/$defs/ActionExpression"}]}
        },
        "functions": {"Bare": {}, "Odd": "not an object"}
    });
    let library = Library::from_json_schema(&schema);
    assert_eq!(
        library.component("Page").unwrap().params[0].schema,
        json!({"anyOf": [{"$ref": "#/$defs/ActionExpression"}]})
    );
    assert!(library.call("Bare").unwrap().params.is_empty());
    assert!(library.call("Odd").unwrap().params.is_empty());
}

/// A slot whose `anyOf` or `oneOf` isn't an array of objects makes the
/// reference throw while validating (documented in CONFORMANCE.md); here it's
/// a slot that takes data.
#[test]
fn a_malformed_union_takes_data() {
    for slot in [json!({"anyOf": [null]}), json!({"oneOf": "x"})] {
        let schema = json!({"$defs": {"Box": {"properties": {"slot": slot}}}});
        let result = parse(
            "root = Box(\"data\")",
            &Library::from_json_schema(&schema),
            None,
        );
        assert!(
            result.meta.errors.is_empty(),
            "{slot}: {:?}",
            result.meta.errors
        );
        assert_eq!(
            result.root.unwrap().props.get("slot"),
            Some(&Value::Str("data".to_owned()))
        );
    }
}

/// `$defs` read the way JavaScript reads parsed JSON: an array's entries are
/// its indexes, so an action named "0" makes entry 0 an action shape, and only
/// entry 1 is a component (checked against the reference).
#[test]
fn an_array_of_defs_is_read_by_index() {
    let schema = json!({
        "$defs": [{"properties": {}}, {"properties": {"a": {}}}],
        "actions": {"0": {"params": {}}}
    });
    let library = Library::from_json_schema(&schema);
    assert_eq!(library.component_names().collect::<Vec<_>>(), ["1"]);
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn js_object_members_include_its_prototype_but_own_checks_do_not() {
    let mut object: JsObject<i32> = JsObject::new();
    assert!(object.has("toString"));
    assert!(!object.has_own("toString"));
    assert!(!object.has("missing"));
    object.define("missing", 1);
    assert!(object.has("missing"));
}
