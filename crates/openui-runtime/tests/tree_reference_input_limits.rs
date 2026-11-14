//! Ticket 34 safety and allocation contracts through the tree interface.
use openui_lang::{Element, Library, ParseResult, free_json, parse, parse_json};
use openui_runtime::{
    JsObject, Limits, Store, TreeInput, Value, evaluate_tree, evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn library() -> Library {
    let j =
        parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
fn root(r: &openui_runtime::TreeResult) -> &Element {
    match r.root.as_ref().unwrap() {
        Value::Element(e) => e,
        _ => panic!("element"),
    }
}
#[test]
fn reference_input_is_prepaid_and_fixed_limits_stay_fixed() {
    let p = parse("root=Box(q)\nq=Query(\"fetch\",{})", &library(), None);
    let mut supplied = JsObject::new();
    supplied.define("q", Value::Str("x".repeat(8000)));
    let input = TreeInput::new(&p).with_reference_values(&supplied);
    let limits = Limits::DEFAULT
        .with_result_floor(32)
        .with_amplification(2)
        .with_result_ceiling(100_000);
    let r = evaluate_tree_with_limits(input.clone(), &Store::new(), limits);
    assert!(!r.size_limited && !r.depth_limited);
    assert_eq!(root(&r).props.get("a"), supplied.get("q"));
    let r = evaluate_tree_with_limits(input, &Store::new(), limits.with_result_size(1000));
    assert!(r.size_limited && !r.depth_limited);
    assert_eq!(
        root(&r).props.get("a"),
        p.root.as_ref().unwrap().props.get("a")
    );
}
#[test]
fn supplied_reference_lists_are_shared_and_each_cost_is_linear() {
    let l = library();
    let p = parse("root=Box(q)\nq=Query(\"fetch\",{})", &l, None);
    let original = Arc::new(vec![Value::Num(1.0); 200_000]);
    let mut supplied = JsObject::new();
    supplied.define("q", Value::Arr(original.clone()));
    test_alloc::reset_peak();
    let input_heap = test_alloc::peak();
    let r = evaluate_tree(
        TreeInput::new(&p).with_reference_values(&supplied),
        &Store::new(),
    );
    assert!(!r.size_limited && !r.depth_limited);
    let Some(Value::Arr(output)) = root(&r).props.get("a") else {
        panic!("list")
    };
    assert!(Arc::ptr_eq(&original, output));
    assert!(test_alloc::peak() < input_heap * 2);

    let p = parse(
        "root=Box(@Each(q,x,Box(x)))\nq=Query(\"fetch\",{})",
        &l,
        None,
    );
    let measure = |n| {
        let mut supplied = JsObject::new();
        supplied.define("q", Value::Arr(Arc::new(vec![Value::Num(1.0); n])));
        let before = test_alloc::allocation_calls();
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_reference_values(&supplied),
            &Store::new(),
            Limits::DEFAULT.with_result_size(8 * 1024 * 1024),
        );
        let calls = test_alloc::allocation_calls() - before;
        assert!(!r.size_limited && !r.depth_limited);
        let Some(Value::Arr(items)) = root(&r).props.get("a") else {
            panic!("list")
        };
        assert_eq!(items.len(), n);
        for item in items.iter() {
            let Value::Element(e) = item else {
                panic!("element")
            };
            assert_eq!(e.props.get("a"), Some(&Value::Num(1.0)));
        }
        calls
    };
    let (a, b, c) = (measure(128), measure(256), measure(512));
    assert!(b <= a * 23 / 10 && c <= b * 23 / 10, "{a} {b} {c}");
}
#[test]
fn loop_cuts_preserve_fitted_props_and_prior_errors() {
    let p = parse(
        "root=Box($n+1,\"\"+$bad,@Each(q,x,Box(x,x,x)),\"later\")\n$n=7\n$bad={toString:null}\nq=Query(\"fetch\",{})",
        &library(),
        None,
    );
    let mut store = Store::new();
    assert!(
        store
            .initialize(
                p.state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                []
            )
            .refused
            .is_empty()
    );
    let mut supplied = JsObject::new();
    supplied.define("q", Value::Arr(Arc::new(vec![Value::Num(1.0); 2000])));
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_reference_values(&supplied),
        &store,
        Limits::DEFAULT.with_result_size(2000),
    );
    assert!(r.size_limited && !r.depth_limited);
    assert_eq!(root(&r).props.get("a"), Some(&Value::Num(8.0)));
    assert_eq!(r.errors.len(), 1);
    for key in ["b", "c", "d"] {
        assert_eq!(
            root(&r).props.get(key),
            p.root.as_ref().unwrap().props.get(key)
        );
    }
}
#[test]
fn cycles_and_nested_loops_cut_but_unread_deep_data_does_not() {
    std::thread::Builder::new()
        .name("reference_input::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let l = library();
            let p = parse("root=Box(q)\nq=Query(\"fetch\",{},q)", &l, None);
            let r = evaluate_tree((&p).into(), &Store::new());
            assert!(r.depth_limited || r.size_limited);
            let p = parse(
                "root=Box(@Each(q,x,Box(@Each(q,y,Box(x,y)))))\nq=Query(\"fetch\",{})",
                &l,
                None,
            );
            let mut supplied = JsObject::new();
            supplied.define("q", Value::Arr(Arc::new(vec![Value::Num(1.0); 100])));
            let r = evaluate_tree_with_limits(
                TreeInput::new(&p).with_reference_values(&supplied),
                &Store::new(),
                Limits::DEFAULT.with_work(40_000),
            );
            assert!(r.size_limited && !r.depth_limited);
            let mut v = Value::Null;
            for _ in 0..20_000 {
                v = Value::Arr(Arc::new(vec![v]));
            }
            supplied.define("q", v);
            let p = ParseResult::default();
            let r = evaluate_tree(
                TreeInput::new(&p).with_reference_values(&supplied),
                &Store::new(),
            );
            assert!(!r.depth_limited && !r.size_limited);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn unread_reference_data_is_prepaid_before_the_first_prop() {
    let values = (0..100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(
        &format!("root=Box(@Each([{values}],x,Box(x)))\nunused=Query(\"get\",{{}})"),
        &library(),
        None,
    );
    let limits = Limits::DEFAULT
        .with_result_floor(32)
        .with_amplification(1)
        .with_result_ceiling(200_000);
    let r = evaluate_tree_with_limits((&p).into(), &Store::new(), limits);
    assert!(r.size_limited && !r.depth_limited);
    let mut supplied = JsObject::new();
    supplied.define("unused", Value::Str("x".repeat(64_000)));
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_reference_values(&supplied),
        &Store::new(),
        limits,
    );
    assert!(!r.size_limited && !r.depth_limited);
    let Some(Value::Arr(items)) = root(&r).props.get("a") else {
        panic!("list")
    };
    assert_eq!(items.len(), 100);
}

#[test]
fn shared_typed_reference_bodies_cannot_multiply_input_credit() {
    let values = (0..100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(
        &format!(
            "root=Box(@Each([{values}],x,Box(\"{}\")))\nunused=Query(\"get\",{{}})",
            "x".repeat(1024)
        ),
        &library(),
        None,
    );
    let mut props = JsObject::new();
    props.define("a", Value::Str("y".repeat(8000)));
    let shared = Value::Element(Arc::new(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    }));
    let mut supplied = JsObject::new();
    supplied.define("unused", Value::Arr(Arc::new(vec![shared; 100])));
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_reference_values(&supplied),
        &Store::new(),
        Limits::DEFAULT
            .with_result_floor(32)
            .with_amplification(1)
            .with_result_ceiling(2_000_000),
    );
    assert!(r.size_limited && !r.depth_limited);
}

#[test]
fn static_each_elements_skip_replacement_work() {
    let source = |dynamic| {
        format!(
            "root=Box(@Each([1],x,{{type:\"element\",typeName:\"Box\",props:{{a:\"{}\"}},partial:false,hasDynamicProps:{dynamic}}}))",
            "x".repeat(2000)
        )
    };
    let l = library();
    let static_input = parse(&source(false), &l, None);
    let dynamic_input = parse(&source(true), &l, None);
    let mut observed = false;
    for work in (512..100_000).step_by(128) {
        let limits = Limits::DEFAULT.with_result_size(100_000).with_work(work);
        let static_result =
            evaluate_tree_with_limits((&static_input).into(), &Store::new(), limits);
        let dynamic_result =
            evaluate_tree_with_limits((&dynamic_input).into(), &Store::new(), limits);
        if !static_result.size_limited
            && !static_result.depth_limited
            && dynamic_result.size_limited
        {
            observed = true;
            break;
        }
    }
    assert!(
        observed,
        "a static Each element must avoid the replacement work"
    );
}
