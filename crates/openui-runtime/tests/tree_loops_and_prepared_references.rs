//! Loop reservations and prepared reference reads.
use openui_lang::{Library, free_json, parse, parse_json};
use openui_runtime::{
    JsObject, Limits, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree,
    evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn library() -> Library {
    let j =
        parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
#[test]
fn data_items_release_substitution_reservations() {
    let xs = (0..100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let y = "y".repeat(200);
    for (template, room) in [
        (format!("{{a:x,b:\"{y}\"}}"), 21802),
        (format!("[x,\"{y}\"]"), 21002),
        (format!("x+\"{y}\""), 40792),
    ] {
        let p = parse(
            &format!("root=Box(@Each([{xs}],x,{template}))"),
            &library(),
            None,
        );
        for (size, cut) in [(room, false), (room - 1, true)] {
            let r = evaluate_tree_with_limits(
                (&p).into(),
                &Store::new(),
                Limits::DEFAULT.with_result_size(size),
            );
            assert_eq!(r.size_limited, cut, "room={size} {template}");
            assert!(!r.depth_limited);
        }
    }
}
#[test]
fn unavailable_references_cut_only_when_read() {
    let l = library();
    let mut deep = Value::Null;
    for _ in 0..300 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    let mut values = JsObject::new();
    values.define("q", deep.clone());
    values.define("undeclared", deep);
    let prepared = PreparedReferenceValues::with_limits(&values, Limits::DEFAULT.with_depth(64));
    let p = parse("root=Box(1+1)\nq=Query(\"fetch\",{})", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_prepared_reference_values(&prepared),
        &Store::new(),
    );
    assert!(!r.depth_limited && !r.size_limited);
    assert!(r.to_json().unwrap().contains("\"a\":2"));
    // The convenience form admits a 300-deep value with its independent bound.
    let r = evaluate_tree(
        TreeInput::new(&p).with_reference_values(&values),
        &Store::new(),
    );
    assert!(!r.depth_limited && !r.size_limited);
    let p = parse("root=Box(1+1,q,3)\nq=Query(\"fetch\",{})", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_prepared_reference_values(&prepared),
        &Store::new(),
    );
    assert!(r.depth_limited && !r.size_limited);
    let text = r.to_json().unwrap();
    assert!(text.contains("\"a\":2"));
    assert!(
        text.contains("\"b\":{\"k\":\"RuntimeRef\",\"n\":\"q\",\"refType\":\"query\"}"),
        "{text}"
    );
}
#[test]
fn oversized_preparation_is_unavailable_only_on_read() {
    let l = library();
    let mut values = JsObject::new();
    values.define("q", Value::Str("y".repeat(1000)));
    let prepared = PreparedReferenceValues::with_limits(
        &values,
        Limits::DEFAULT.with_work(32).with_result_size(32),
    );
    for (source, cut) in [
        ("root=Box(1+1)\nq=Query(\"get\",{})", false),
        ("root=Box(1+1,q)\nq=Query(\"get\",{})", true),
    ] {
        let p = parse(source, &l, None);
        let r = evaluate_tree(
            TreeInput::new(&p).with_prepared_reference_values(&prepared),
            &Store::new(),
        );
        assert_eq!(r.size_limited, cut);
        assert!(!r.depth_limited);
        assert!(r.to_json().unwrap().contains("\"a\":2"));
    }
}

#[test]
fn prepared_large_values_are_reused_without_scanning() {
    let l = library();
    let p = parse("root=Box(1+1)", &l, None);
    let store = Store::new();
    let mut large = JsObject::new();
    large.define(
        "unused",
        Value::Arr(Arc::new(vec![Value::Num(12345.0); 4_000_000])),
    );
    let before = test_alloc::allocation_calls();
    let large = PreparedReferenceValues::new(&large);
    // A number is measured without a per-number serialization allocation.
    assert!(test_alloc::allocation_calls() - before < 200);
    let small = JsObject::new();
    let small = PreparedReferenceValues::new(&small);
    let measure = |values: &PreparedReferenceValues<'_>| {
        let input = TreeInput::new(&p).with_prepared_reference_values(values);
        let before = test_alloc::allocation_calls();
        for _ in 0..40 {
            let r = evaluate_tree(input.clone(), &store);
            assert!(!r.size_limited && !r.depth_limited);
            std::hint::black_box(r);
        }
        test_alloc::allocation_calls() - before
    };
    measure(&small);
    measure(&large);
    let ac = measure(&small);
    let bc = measure(&large);
    assert_eq!(ac, bc, "prepared input should add no evaluation allocation");
}

#[test]
fn streamed_query_metadata_cannot_identify_an_unfinished_query() {
    let mut stream = openui_lang::StreamParser::new(Arc::new(library()), None);
    let unfinished = stream.push("root=Box(q)\nq=Query(\"get\",{},{a:1}");
    assert!(unfinished.meta.incomplete);
    assert_eq!(unfinished.query_statements.len(), 1);
    assert!(unfinished.query_statements.first().unwrap().complete);
    let finished = stream.push(")");
    assert!(!finished.meta.incomplete);
    assert!(finished.query_statements.first().unwrap().complete);
}
#[test]
fn string_each_arguments_evaluate_to_an_empty_list() {
    let p = parse(
        "root=Box({k:\"Comp\",name:\"Each\",args:\"abc\"})",
        &library(),
        None,
    );
    let r = evaluate_tree((&p).into(), &Store::new());
    assert!(r.errors.is_empty());
    assert!(r.to_json().unwrap().contains("\"a\":[]"));
}

#[test]
fn descriptor_loop_substitutes_array_mapped_props() {
    let p = parse(
        r#"root=Box({k:"Comp",name:"Each",args:[{k:"Arr",els:[{k:"Num",v:7}]},{k:"Ref",n:"x"},{k:"Comp",name:"Box",args:[],mappedProps:[{k:"Ref",n:"x"}]}]})"#,
        &library(),
        None,
    );
    let r = evaluate_tree((&p).into(), &Store::new());
    assert!(r.errors.is_empty());
    assert!(
        r.to_json().unwrap().contains("\"props\":{\"0\":7}"),
        "{}",
        r.to_json().unwrap()
    );
}

// A defer-capable call observes the substituted descriptor, rather than reading
// x immediately through the loop binding (which could hide missing capture).
#[test]
fn descriptor_calls_capture_array_props_before_the_hook() {
    struct Capture;
    impl openui_runtime::EvaluationScope for Capture {
        fn call_value(
            &self,
            call: &Value,
            _: &openui_runtime::ExpressionEvaluator<'_>,
        ) -> Option<Value> {
            Some(call.clone())
        }
    }
    let j=parse_json(r#"{"k":"Comp","name":"Each","args":[{"k":"Arr","els":[{"k":"Num","v":7}]},{"k":"Ref","n":"x"},{"k":"Comp","name":"Box","args":[],"mappedProps":[{"k":"Ref","n":"x"}]}]}"#).unwrap();
    let descriptor = Value::from_json(&j);
    free_json(j);
    let session = openui_runtime::EvaluationSession::new(&Capture, 256);
    let result = session.with_evaluator(|eval| eval.evaluate_value(&descriptor));
    assert!(result.failure.is_none());
    let Some(Value::Arr(items)) = result.value.as_ref() else {
        panic!("list")
    };
    let Some(Value::Obj(call)) = items.first() else {
        panic!("captured call")
    };
    let Some(Value::Obj(props)) = call.get("mappedProps") else {
        panic!("captured mapped props")
    };
    assert!(
        matches!(props.get("0"),Some(Value::Ast(a)) if matches!(a.as_ref(),openui_runtime::Ast::Num(n) if *n==7.0))
    );
}
