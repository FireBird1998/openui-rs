//! Public helpers the conformance tests don't reach through parsing: the JSON
//! reader and writer, `JsObject`, and walking an expression.

use openui_lang::{Ast, JsObject, Library, Value, parse, parse_json, stringify_json};
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
