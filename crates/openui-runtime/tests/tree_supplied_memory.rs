//! Tree pass-through must keep supplied container bodies shared and scans bounded.
use openui_lang::{Library, free_json, parse, parse_json};
use openui_runtime::{JsObject, Store, Value, evaluate_tree};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn library() -> Library {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
fn root(r: &openui_runtime::TreeResult) -> &openui_lang::Element {
    match r.root.as_ref().unwrap() {
        Value::Element(e) => e,
        _ => panic!("element"),
    }
}
#[test]
fn large_supplied_tree_prop_shares_bodies_below_twice_the_input_heap() {
    let l = library();
    let p = parse("root=Box($items)", &l, None);
    let original: Arc<Vec<Value>> = Arc::new(
        (0..900_000)
            .map(|_| Value::Arr(Arc::new(Vec::new())))
            .collect(),
    );
    let mut s = Store::new();
    s.set("$items", Some(Value::Arr(original.clone()))).unwrap();
    test_alloc::reset_peak();
    let input = test_alloc::peak();
    let r = evaluate_tree((&p).into(), &s);
    assert!(!r.size_limited && !r.depth_limited, "{r:?}");
    let Value::Arr(output) = root(&r).props.get("a").unwrap() else {
        panic!("array")
    };
    assert_eq!(output.len(), 900_000);
    assert!(Arc::ptr_eq(&original, output));
    assert!(
        test_alloc::peak() < input * 2,
        "input={input}, peak={}",
        test_alloc::peak()
    );
}
#[test]
fn shared_dag_identity_avoids_exponential_diagnostic_walks() {
    let l = library();
    let p = parse(
        "a=Query(\"a\",{})\nb=Query(\"b\",{})\nc=Query(\"c\",{})\nd=Query(\"d\",{})\nroot=Box(a==b,c==d)",
        &l,
        None,
    );
    let mut value = Value::Num(1.0);
    for _ in 0..64 {
        value = Value::Arr(Arc::new(vec![value.clone(), value]));
    }
    let mut s = Store::new();
    for name in ["$a", "$b", "$c", "$d"] {
        assert!(s.set(name, Some(value.clone())).is_err());
    }
    assert!(s.snapshot().is_empty());
    let mut references = JsObject::new();
    for name in ["a", "b", "c", "d"] {
        references.define(name, value.clone());
    }
    let r = evaluate_tree(
        openui_runtime::TreeInput::new(&p).with_reference_values(&references),
        &s,
    );
    assert!(!r.size_limited && !r.depth_limited, "{r:?}");
    for name in ["a", "b"] {
        assert_eq!(root(&r).props.get(name), Some(&Value::Bool(true)));
    }
}
#[test]
fn object_index_coercion_fails_but_array_indices_follow_numeric_conversion() {
    let l = library();
    let p = parse("root=Box($obj[$key],$arr[$key])", &l, None);
    let mut object = JsObject::new();
    object.define("a", Value::Num(1.0));
    let mut key = JsObject::new();
    key.define("toString", Value::Null);
    let mut s = Store::new();
    s.set("$obj", Some(Value::Obj(Arc::new(object)))).unwrap();
    s.set("$key", Some(Value::Obj(Arc::new(key)))).unwrap();
    s.set("$arr", Some(Value::Arr(Arc::new(vec![Value::Num(7.0)]))))
        .unwrap();
    let r = evaluate_tree((&p).into(), &s);
    assert!(!r.size_limited && !r.depth_limited);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(
        r.errors[0].message,
        "Evaluating prop \"a\" on Box failed: No default value"
    );
    assert_eq!(root(&r).props.get("b"), Some(&Value::Num(7.0)));
}
