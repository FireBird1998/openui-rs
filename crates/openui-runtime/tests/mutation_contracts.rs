//! Public contracts exposed by the bounded #40 mutation sample.
use openui_lang::{Library, Limits, parse, parse_json};
use openui_runtime::{
    BindingProps, LibraryFunctions, Store, TreeInput, Value, evaluate_tree, value_to_json,
};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

#[test]
fn value_export_preserves_non_null_values_and_json_escaping() {
    let value = Value::Arr(Arc::new(vec![
        Value::Num(7.0),
        Value::Str("a\"b\nλ".into()),
        Value::Bool(true),
    ]));
    assert_eq!(value_to_json(&value).unwrap(), "[7,\"a\\\"b\\nλ\",true]");
}

#[test]
fn limited_binding_registration_refuses_an_extra_name_without_changing_marks() {
    let mut props = BindingProps::with_limits(Limits::DEFAULT.with_result_size(10));
    props.insert("Box", "a").unwrap();
    props.insert("Box", "a").unwrap();
    let refusal = props.insert("Box", "b").unwrap_err();
    assert!(refusal.flags.size_limited);
    assert!(props.contains("Box", "a"));
    assert!(!props.contains("Box", "b"));
    assert!(props.remove("Box", "a"));
    assert!(!props.contains("Box", "a"));
}

#[test]
fn limited_function_registration_keeps_the_existing_implementation_after_refusal() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"One":{"params":{"type":"object","properties":{}}},"Two":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let definitions = schema.get("functions").unwrap();
    let mut functions =
        LibraryFunctions::with_limits(definitions, Limits::DEFAULT.with_result_size(6));
    functions
        .register("One", |_| Ok(Some(Value::Num(7.0))))
        .unwrap();
    assert!(
        functions
            .register("Two", |_| Ok(Some(Value::Num(9.0))))
            .unwrap_err()
            .flags
            .size_limited
    );
    let result = parse("root=Box(One())", &library, None);
    let evaluated = evaluate_tree(
        TreeInput::new(&result)
            .with_library(&library)
            .with_functions(&functions),
        &Store::new(),
    );
    assert!(!evaluated.size_limited && !evaluated.depth_limited);
    assert!(evaluated.errors.is_empty());
    assert_eq!(
        evaluated.to_json().unwrap(),
        r#"{"root":{"type":"element","typeName":"Box","props":{"a":7},"partial":false,"hasDynamicProps":true,"statementId":"root"},"errors":[]}"#
    );
}

#[test]
fn store_version_advances_only_when_values_change() {
    let mut store = Store::new();
    assert_eq!(store.version(), 0);
    assert!(store.set("$n", Some(Value::Num(7.0))).unwrap());
    assert_eq!(store.version(), 1);
    assert!(!store.set("$n", Some(Value::Num(7.0))).unwrap());
    assert_eq!(store.version(), 1);
    assert!(store.set("$n", Some(Value::Num(8.0))).unwrap());
    assert_eq!(store.version(), 2);
}
