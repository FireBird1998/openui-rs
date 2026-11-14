//! Contracts from the second independent ticket 34 review.
use openui_lang::{Library, free_json, parse, parse_json};
use openui_runtime::{
    JsObject, Limits, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree_with_limits,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn library() -> Library {
    let j =
        parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
fn element(width: Option<usize>) -> Value {
    let a = width.map_or_else(|| "1".into(), |n| format!("\"{}\"", "y".repeat(n)));
    let j=parse_json(&format!(r#"{{"type":"element","typeName":"Box","props":{{"a":{a}}},"partial":false,"hasDynamicProps":true}}"#)).unwrap();
    let v = Value::from_json(&j);
    free_json(j);
    v
}
#[test]
fn dynamic_element_room_matches_main() {
    let l = library();
    for native in [false, true] {
        for (n, width, kind, room) in [
            (10, Some(1000), 0, 21941),
            (10, Some(1000), 1, 21942),
            (10, Some(1000), 2, 32872),
            (100, None, 1, 18212),
            (100, None, 2, 27412),
        ] {
            let raw = element(width);
            let e = if native {
                let Value::Obj(o) = &raw else {
                    panic!("element")
                };
                let Some(Value::Obj(props)) = o.get("props") else {
                    panic!("props")
                };
                Value::Element(Arc::new(openui_lang::Element {
                    type_name: "Box".into(),
                    props: props.as_ref().clone(),
                    partial: false,
                    has_dynamic_props: true,
                    statement_id: None,
                }))
            } else {
                raw
            };
            let mut store = Store::new();
            store.set("$de", Some(e.clone())).unwrap();
            store
                .set("$l", Some(Value::Arr(Arc::new(vec![e; n]))))
                .unwrap();
            let body = match kind {
                0 => format!("[{}]", vec!["$de"; n].join(",")),
                1 => format!(
                    "@Each([{}],x,$de)",
                    (0..n).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
                ),
                _ => "@Each($l,x,x)".into(),
            };
            let p = parse(&format!("root=Box({body})"), &l, None);
            for (size, cut) in [(room, false), (room - 1, true)] {
                let r = evaluate_tree_with_limits(
                    (&p).into(),
                    &store,
                    Limits::DEFAULT.with_result_size(size),
                );
                assert_eq!(r.size_limited, cut, "n={n} kind={kind} room={size}");
                assert!(!r.depth_limited);
            }
        }
    }
}
#[test]
fn prepared_index_reads_reuse_the_measurement() {
    let l = library();
    let p = parse("root=Box(q[0])\nq=Query(\"get\",{})", &l, None);
    let store = Store::new();
    let mut small = JsObject::new();
    small.define("q", Value::Arr(Arc::new(vec![Value::Num(7.0)])));
    let mut large = JsObject::new();
    large.define("q", Value::Arr(Arc::new(vec![Value::Num(7.0); 1_000_000])));
    let small = PreparedReferenceValues::new(&small);
    let large = PreparedReferenceValues::new(&large);
    let measure = |values: &PreparedReferenceValues<'_>| {
        let input = TreeInput::new(&p).with_prepared_reference_values(values);
        let before = test_alloc::allocation_calls();
        let start = Instant::now();
        for _ in 0..40 {
            let r = evaluate_tree_with_limits(
                input.clone(),
                &store,
                Limits::DEFAULT.with_work(100_000).with_result_size(4096),
            );
            assert!(!r.size_limited && !r.depth_limited);
            assert!(r.to_json().contains("\"a\":7"));
        }
        (start.elapsed(), test_alloc::allocation_calls() - before)
    };
    measure(&small);
    measure(&large);
    let (a, ac) = measure(&small);
    let (b, bc) = measure(&large);
    assert_eq!(ac, bc);
    assert!(
        b < a.saturating_mul(20) + Duration::from_millis(20),
        "small={a:?} large={b:?}"
    );
    // The prepared marker never discounts building the full output.
    let full = parse("root=Box(q)\nq=Query(\"get\",{})", &l, None);
    let r = evaluate_tree_with_limits(
        TreeInput::new(&full).with_prepared_reference_values(&large),
        &store,
        Limits::DEFAULT.with_work(100_000).with_result_size(4096),
    );
    assert!(r.size_limited);
}

#[test]
fn prepared_strings_are_converted_once_before_reads() {
    let p = parse("root=Box(q ? 7 : 0)\nq=Query(\"get\",{})", &library(), None);
    let mut values = JsObject::new();
    values.define("q", Value::Str("y".repeat(1_000_000)));
    let prepared = PreparedReferenceValues::new(&values);
    for _ in 0..40 {
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_prepared_reference_values(&prepared),
            &Store::new(),
            Limits::DEFAULT.with_work(10_000).with_result_size(4096),
        );
        assert!(!r.size_limited && !r.depth_limited);
        assert!(r.to_json().contains("\"a\":7"));
    }
}
#[test]
fn prepared_claims_never_add_credit_or_discount_output() {
    use openui_runtime::{Ast, EvaluationScope, EvaluationSession, PreparedReferenceValue};
    struct Scope(PreparedReferenceValue);
    impl EvaluationScope for Scope {
        fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
            Some(&self.0)
        }
    }
    let node = Ast::Ref("q".into());
    let value = Value::Arr(Arc::new(vec![Value::Num(7.0); 100_000]));
    let scope = Scope(PreparedReferenceValue::new(&value, 0));
    let session = EvaluationSession::with_limits(
        &scope,
        0,
        Limits::DEFAULT.with_work(10_000).with_result_size(64),
    );
    assert!(session.evaluate(&node).size_limited);
    let scope = Scope(PreparedReferenceValue::new(&Value::Num(1.0), 1000));
    let session = EvaluationSession::new(&scope, 0);
    assert!(session.evaluate(&node).size_limited);
    let scope = Scope(PreparedReferenceValue::new(
        &Value::Str("x".repeat(600)),
        300,
    ));
    let session = EvaluationSession::with_limits(
        &scope,
        400,
        Limits::DEFAULT
            .with_result_floor(32)
            .with_amplification(1)
            .with_result_ceiling(4096),
    );
    assert!(session.evaluate(&node).size_limited);
}

#[test]
fn item_projection_of_native_props_keeps_the_props_object() {
    let Value::Obj(inner) = &element(None) else {
        panic!("element")
    };
    let outer = openui_lang::Element {
        type_name: "Container".into(),
        props: inner.as_ref().clone(),
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    };
    let mut store = Store::new();
    store
        .set("$outer", Some(Value::Element(Arc::new(outer))))
        .unwrap();
    let p = parse("root=Box(@Each([1],x,$outer.props))", &library(), None);
    let r = evaluate_tree_with_limits((&p).into(), &store, Limits::DEFAULT);
    assert!(!r.size_limited && !r.depth_limited);
    assert!(r.errors.is_empty());
    let json = r.to_json();
    assert_eq!(json.matches("\"typeName\":\"Box\"").count(), 2, "{json}");
    assert!(!json.contains("Container"));
}

#[test]
fn typed_and_data_inherited_calls_fail_before_the_component_hook() {
    use openui_runtime::{
        Ast, EvaluationFailure, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    };
    use std::cell::Cell;
    struct Scope {
        reads: Cell<usize>,
        calls: Cell<usize>,
    }
    impl EvaluationScope for Scope {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.reads.set(self.reads.get() + 1);
            Some(Value::Num(3.0))
        }
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.calls.set(self.calls.get() + 1);
            Some(Value::Null)
        }
        fn call_value(&self, _: &Value, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.calls.set(self.calls.get() + 1);
            Some(Value::Null)
        }
    }
    for name in [
        "toString",
        "valueOf",
        "hasOwnProperty",
        "__proto__",
        "constructor",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toLocaleString",
        "__defineGetter__",
        "__defineSetter__",
        "__lookupGetter__",
        "__lookupSetter__",
    ] {
        for mapped in [false, true] {
            let scope = Scope {
                reads: Cell::new(0),
                calls: Cell::new(0),
            };
            let mut props = JsObject::new();
            props.define("a", Ast::StateRef("$z".into()));
            let ast = Ast::Comp {
                name: name.into(),
                args: Arc::new(vec![Ast::StateRef("$z".into())]),
                function: false,
                mapped_props: mapped.then(|| Arc::new(props)),
            };
            let extra = if mapped {
                r#", "mappedProps":{"a":{"k":"StateRef","n":"$z"}}"#
            } else {
                ""
            };
            let j = parse_json(&format!(
                r#"{{"k":"Comp","name":"{name}","args":[{{"k":"StateRef","n":"$z"}}]{extra}}}"#
            ))
            .unwrap();
            let raw = Value::from_json(&j);
            free_json(j);
            let session = EvaluationSession::new(&scope, 512);
            let a = session.evaluate(&ast);
            let b = session.with_evaluator(|eval| eval.evaluate_value(&raw));
            assert!(
                matches!(a.failure, Some(EvaluationFailure::BuiltinNotCallable)),
                "{name}"
            );
            assert!(
                matches!(b.failure, Some(EvaluationFailure::BuiltinNotCallable)),
                "{name}"
            );
            assert_eq!(scope.calls.get(), 0);
            assert_eq!(scope.reads.get(), usize::from(mapped));
        }
    }
}

#[test]
fn replacement_buffers_stay_funded_until_their_projection() {
    for nested in [false, true] {
        let mut values = JsObject::new();
        values.define("q", Value::Arr(Arc::new(vec![Value::Num(0.0); 50_000])));
        let prepared = PreparedReferenceValues::new(&values);
        let mut store = Store::new();
        let mut de = element(None);
        if nested {
            let Value::Obj(node) = &de else {
                panic!("element")
            };
            let mut body = node.as_ref().clone();
            let mut props = JsObject::new();
            props.define("a", Value::Ast(Arc::new(openui_lang::Ast::Num(1.0))));
            body.define("props", Value::Obj(Arc::new(props)));
            de = Value::Obj(Arc::new(body));
        }
        store.set("$de", Some(de)).unwrap();
        let p = parse(
            "root=Box(@Each(q,x,$de))\nq=Query(\"get\",{})",
            &library(),
            None,
        );
        // Inner prop projections must not release an outer loop's earlier items.
        let mut small = JsObject::new();
        small.define("q", Value::Arr(Arc::new(vec![Value::Num(0.0); 10])));
        let small = PreparedReferenceValues::new(&small);
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_prepared_reference_values(&small),
            &store,
            Limits::DEFAULT.with_result_size(1922),
        );
        assert!(!r.size_limited && !r.depth_limited);
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_prepared_reference_values(&prepared),
            &store,
            Limits::DEFAULT.with_result_size(4096),
        );
        assert!(r.size_limited && !r.depth_limited);
        assert!(
            test_alloc::peak() < before * 2,
            "nested={nested} input={before} peak={}",
            test_alloc::peak()
        );
    }
}

#[test]
fn failed_inherited_calls_do_not_buffer_discarded_named_args() {
    let mut call = JsObject::new();
    call.define("k", Value::Str("Comp".into()));
    call.define("name", Value::Str("toString".into()));
    call.define("args", Value::Arr(Arc::new(vec![])));
    call.define(
        "mappedProps",
        Value::Arr(Arc::new(vec![Value::Num(0.0); 200_000])),
    );
    let Value::Obj(base) = &element(None) else {
        panic!("element")
    };
    let mut node = base.as_ref().clone();
    let mut props = JsObject::new();
    props.define("a", Value::Obj(Arc::new(call)));
    node.define("props", Value::Obj(Arc::new(props)));
    let mut store = Store::new();
    store.set("$e", Some(Value::Obj(Arc::new(node)))).unwrap();
    let p = parse("root=Box($e)", &library(), None);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = evaluate_tree_with_limits((&p).into(), &store, Limits::DEFAULT);
    assert!(!r.size_limited && !r.depth_limited);
    assert_eq!(r.errors.len(), 1);
    assert!(
        r.errors
            .first()
            .unwrap()
            .message
            .contains("builtin.fn is not a function")
    );
    assert!(
        test_alloc::peak() < before * 2,
        "input={before} peak={}",
        test_alloc::peak()
    );
}
