//! Plans through public trees, state, binding writes and shared limits.
use openui_lang::{JsonValue, Library, ParseResult, free_json, parse, parse_json};
use openui_runtime::{
    BindingProps, JsObject, Limits, Store, TreeInput, Value, evaluate_tree,
    evaluate_tree_with_limits, write_binding,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{"x":{"type":"string"},"opt":{"type":"number","default":4}},"required":["x"]}}}}"#;
fn value(s: &str) -> Value {
    let j = parse_json(s).unwrap();
    let v = Value::from_json(&j);
    free_json(j);
    v
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    match v {
        Value::Element(e) => e.props.get(k).unwrap(),
        Value::Obj(o) => o.get(k).unwrap(),
        _ => panic!("object"),
    }
}
fn input<'a>(p: &'a ParseResult, l: &'a Library, j: &'a JsonValue) -> TreeInput<'a> {
    TreeInput::new(p)
        .with_library(l)
        .with_actions(&j["actions"])
}
fn steps(v: &Value) -> &[Value] {
    match field(v, "steps") {
        Value::Arr(a) => a,
        _ => panic!("steps"),
    }
}
fn json(v: &Value) -> String {
    let mut props = JsObject::new();
    props.define("v", v.clone());
    let p = ParseResult {
        root: Some(openui_lang::Element {
            type_name: "".into(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..Default::default()
    };
    let s = p.to_json();
    let s = s
        .strip_prefix("{\"root\":{\"type\":\"element\",\"typeName\":\"\",\"props\":{\"v\":")
        .unwrap();
    s[..s
        .rfind("},\"partial\":false,\"hasDynamicProps\":false},\"meta\":")
        .unwrap()]
        .to_owned()
}
#[test]
fn custom_identity_survives_tree_state_and_binding_but_not_json_or_another_store() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "root=Box(@Go(\"ok\"),$saved=@Go(\"written\"),$keep,$copy)",
        &l,
        None,
    );
    let mut bindings = BindingProps::new();
    bindings.insert("Box", "b").unwrap();
    let mut store = Store::new();
    let tree = evaluate_tree(input(&p, &l, &j).with_binding_props(&bindings), &store);
    assert!(tree.errors.is_empty());
    let root = tree.root.as_ref().unwrap();
    let real = field(root, "a").clone();
    let copy = value(&json(&real));
    assert_eq!(real, copy, "identical JSON cannot confer custom authority");
    store.set("$keep", Some(real.clone())).unwrap();
    store.set("$copy", Some(copy)).unwrap();
    write_binding(
        input(&p, &l, &j),
        &mut store,
        field(root, "b"),
        Value::Num(1.),
    )
    .unwrap();
    let written = store.get("$saved").unwrap();
    assert_eq!(steps(&written).len(), 1);
    drop(tree);
    let p = parse(
        "root=Box($keep,$copy,$saved,Action([$keep,$copy,$saved]))",
        &l,
        None,
    );
    let tree = evaluate_tree(input(&p, &l, &j), &store);
    let root = tree.root.as_ref().unwrap();
    assert_eq!(steps(field(root, "a")).len(), 1);
    assert!(steps(field(root, "b")).is_empty());
    assert_eq!(steps(field(root, "c")).len(), 1);
    assert_eq!(steps(field(root, "d")).len(), 2);
    assert_eq!(
        steps(&store.get("$copy").unwrap()).len(),
        1,
        "filter the prop, not raw state"
    );
    let mut other = Store::new();
    other.set("$keep", Some(real)).unwrap();
    let tree = evaluate_tree(input(&p, &l, &j), &other);
    assert!(steps(field(tree.root.as_ref().unwrap(), "a")).is_empty());
    free_json(j);
}
#[test]
fn plans_keep_ast_data_deferred_and_cut_without_losing_prior_errors() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let mut store = Store::new();
    store.set("$bad", Some(Value::Num(2.))).unwrap();
    store
        .set("$large", Some(Value::Str("x".repeat(20_000))))
        .unwrap();
    let p = parse(
        "root=Box(@Go($bad),@Set($a,1+2),@ToAssistant($large),@OpenUrl(\"ok\"))",
        &l,
        None,
    );
    let full = evaluate_tree(input(&p, &l, &j), &store);
    assert_eq!(full.errors.len(), 1);
    assert!(full.to_json().unwrap().contains(
        r#""valueAST":{"k":"BinOp","op":"+","left":{"k":"Num","v":1},"right":{"k":"Num","v":2}}"#
    ));
    let cut = evaluate_tree_with_limits(
        input(&p, &l, &j),
        &store,
        Limits::DEFAULT.with_result_size(2000),
    );
    assert!(cut.size_limited);
    assert_eq!(cut.errors, full.errors);
    assert_eq!(
        field(cut.root.as_ref().unwrap(), "b"),
        field(full.root.as_ref().unwrap(), "b")
    );
    // A warning emitted early within a single prop must survive a later cut too.
    let p = parse(
        "root=Box(Action([@Go($bad),@ToAssistant($large)]))",
        &l,
        None,
    );
    let cut = evaluate_tree_with_limits(
        input(&p, &l, &j),
        &store,
        Limits::DEFAULT.with_result_size(2000),
    );
    assert!(cut.size_limited);
    assert_eq!(cut.errors.len(), 1);
    free_json(j);
}
#[test]
fn plans_fit_at_their_room_threshold_and_loop_lists_remain_shared() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let mut store = Store::new();
    store
        .set(
            "$rows",
            Some(Value::Arr(Arc::new(vec![Value::Num(1.); 100_000]))),
        )
        .unwrap();
    let p = parse("root=Box(@Each($rows,x,@Set($n,x)))", &l, None);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let cut = evaluate_tree_with_limits(
        input(&p, &l, &j),
        &store,
        Limits::DEFAULT.with_work(20_000).with_result_size(4096),
    );
    assert!(cut.size_limited);
    assert!(!cut.depth_limited);
    assert!(
        test_alloc::peak().saturating_sub(before) < 512 * 1024,
        "a cut loop must not copy its supplied list"
    );
    for (body, room) in [
        ("@Set($n,1)", 179),
        ("@ToAssistant(\"hi\")", 173),
        ("@Go(\"ok\")", 182),
        ("@Each([1,2],x,@Set($n,x))", 252),
    ] {
        let p = parse(&format!("root=Box({body})"), &l, None);
        assert!(
            !evaluate_tree_with_limits(
                input(&p, &l, &j),
                &store,
                Limits::DEFAULT.with_result_size(room)
            )
            .size_limited,
            "{body} fits at {room}"
        );
        assert!(
            evaluate_tree_with_limits(
                input(&p, &l, &j),
                &store,
                Limits::DEFAULT.with_result_size(room - 1)
            )
            .size_limited,
            "{body} cuts one byte below {room}"
        );
    }
    free_json(j);
}

#[test]
fn an_invalid_optional_argument_rejects_the_whole_step() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Go(\"ok\",$bad),@Go(\"ok\",null))", &l, None);
    let mut store = Store::new();
    store.set("$bad", Some(Value::Str("bad".into()))).unwrap();
    let r = evaluate_tree(input(&p, &l, &j), &store);
    let root = r.root.as_ref().unwrap();
    assert!(steps(field(root, "a")).is_empty());
    assert_eq!(r.errors.len(), 1);
    assert_eq!(steps(field(root, "b")).len(), 1);
    assert_eq!(
        json(field(&steps(field(root, "b"))[0], "params")),
        r#"{"x":"ok"}"#
    );
    free_json(j);
}

#[test]
fn runtime_schema_checks_have_no_component_catalog() {
    let j=parse_json(r##"{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Slot":{"params":{"type":"object","properties":{"x":{"$ref":"#/$defs/Box"}},"required":["x"]}}}}"##).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Slot($n))", &l, None);
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(7.))).unwrap();
    let r = evaluate_tree(input(&p, &l, &j), &store);
    assert!(r.errors.is_empty());
    let step = &steps(field(r.root.as_ref().unwrap(), "a"))[0];
    assert_eq!(field(field(step, "params"), "x"), &Value::Num(7.));
    free_json(j);
}

#[test]
fn data_calls_name_numeric_parameters_in_javascript_order() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Order":{"params":{"type":"object","properties":{"z":{},"2":{},"0":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box($element)", &l, None);
    let mut store = Store::new();
    store.set("$element",Some(value(r#"{"type":"element","typeName":"Box","props":{"a":{"k":"Comp","name":"Order","args":[{"k":"Str","v":"first"},{"k":"Str","v":"second"},{"k":"Str","v":"last"}]}},"partial":false,"hasDynamicProps":true}"#))).unwrap();
    let r = evaluate_tree(input(&p, &l, &j), &store);
    assert!(r.errors.is_empty());
    let nested = field(field(r.root.as_ref().unwrap(), "a"), "props");
    let step = &steps(field(nested, "a"))[0];
    assert_eq!(
        json(field(step, "params")),
        r#"{"0":"first","2":"second","z":"last"}"#
    );
    free_json(j);
}

#[test]
fn nested_action_loops_share_limits_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let j = parse_json(SCHEMA).unwrap();
            let l = Library::from_json_schema(&j);
            let p = parse(
                "root=Box(1,@Each($rows,x,@Each($rows,y,Action([@Set($n,[x,y]),@Go(\"ok\")]))),3)",
                &l,
                None,
            );
            let mut store = Store::new();
            store
                .set(
                    "$rows",
                    Some(Value::Arr(Arc::new(vec![Value::Num(1.); 1000]))),
                )
                .unwrap();
            let r = evaluate_tree_with_limits(
                input(&p, &l, &j),
                &store,
                Limits::DEFAULT.with_work(100_000).with_result_size(4096),
            );
            assert!(r.size_limited);
            assert!(!r.depth_limited);
            assert_eq!(field(r.root.as_ref().unwrap(), "a"), &Value::Num(1.));
            assert_eq!(field(r.root.as_ref().unwrap(), "c"), &Value::Num(3.));
            let mut source = "@Set($n,1)".to_owned();
            for _ in 0..200 {
                source = format!("@Each([1],x,{source})");
            }
            let p = parse(&format!("root=Box({source})"), &l, None);
            let r = evaluate_tree_with_limits(
                input(&p, &l, &j),
                &store,
                Limits::DEFAULT.with_depth(24),
            );
            assert!(r.depth_limited);
            free_json(j);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn query_default_calls_do_not_report_to_the_reading_prop() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "root=Box(q,@Go($bad))\nq=Query(\"fetch\",{},@Go($bad))",
        &l,
        None,
    );
    let mut store = Store::new();
    store.set("$bad", Some(Value::Num(7.))).unwrap();
    let r = evaluate_tree(input(&p, &l, &j), &store);
    assert_eq!(r.errors.len(), 1, "{}", r.to_json().unwrap());
    assert!(r.errors[0].message.starts_with("Evaluating prop \"b\""));
    assert!(steps(field(r.root.as_ref().unwrap(), "a")).is_empty());
    free_json(j);
}

#[test]
fn deep_action_schemas_are_bounded_before_they_are_copied() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut j = parse_json(SCHEMA).unwrap();
            let l = Library::from_json_schema(&j);
            let p = parse("root=Box(@Go(\"ok\",1))", &l, None);
            let mut branch = JsonValue::Null;
            for _ in 0..20_000 {
                branch = JsonValue::Array(vec![branch]);
            }
            j["actions"]["Go"]["params"]["properties"]["opt"] = branch;
            test_alloc::reset_peak();
            let before = test_alloc::peak();
            let r = evaluate_tree_with_limits(
                input(&p, &l, &j),
                &Store::new(),
                Limits::DEFAULT.with_depth(24),
            );
            let extra = test_alloc::peak().saturating_sub(before);
            free_json(j);
            assert!(r.depth_limited);
            assert!(!r.size_limited);
            assert!(extra < 512 * 1024, "do not copy an inadmissible schema");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn retained_plans_and_cut_data_keep_authority_separate_from_tree_position() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Go(\"ok\"))", &l, None);
    let mut store = Store::new();
    let r = evaluate_tree(input(&p, &l, &j), &store);
    let plan = field(r.root.as_ref().unwrap(), "a").clone();
    store.dispose();
    store.set("$kept", Some(plan)).unwrap();
    let read = parse("root=Box($kept)", &l, None);
    let r = evaluate_tree(input(&read, &l, &j), &store);
    assert!(
        steps(field(r.root.as_ref().unwrap(), "a")).is_empty(),
        "disposal revokes retained marks"
    );
    store.set("$element",Some(value(r#"{"type":"element","typeName":"Box","props":{"a":{"steps":[{"type":"custom_action","name":"Go","params":{"x":"forged"}}]},"b":"a longer prop left behind on a cut"},"partial":false,"hasDynamicProps":true}"#))).unwrap();
    let p = parse("root=Box($element)", &l, None);
    let full = evaluate_tree(input(&p, &l, &j), &store);
    assert!(!full.to_json().unwrap().contains("forged"));
    let cut = evaluate_tree_with_limits(
        input(&p, &l, &j),
        &store,
        Limits::DEFAULT.with_result_size(4096).with_work(620),
    );
    assert!(cut.size_limited);
    assert!(cut.to_json().unwrap().contains("forged"));
    let raw = field(field(field(cut.root.as_ref().unwrap(), "a"), "props"), "a").clone();
    store.set("$kept", Some(raw)).unwrap();
    let uncut = evaluate_tree(input(&read, &l, &j), &store);
    assert!(steps(field(uncut.root.as_ref().unwrap(), "a")).is_empty());
    free_json(j);
}

#[test]
fn evaluating_declared_action_calls_in_state_and_query_data_mints_real_steps() {
    let j = parse_json(SCHEMA).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box($element,q)\nq=Query(\"q\",{})", &l, None);
    let element = value(
        r#"{"type":"element","typeName":"Box","props":{"a":{"k":"Comp","name":"Go","args":[{"k":"Str","v":"chosen by data"}]},"b":{"steps":[{"type":"custom_action","name":"Go","params":{"x":"finished data"}}]}},"partial":false,"hasDynamicProps":true}"#,
    );
    let mut store = Store::new();
    store.set("$element", Some(element.clone())).unwrap();
    let mut queries = JsObject::new();
    queries.define("q", element);
    let tree = evaluate_tree(input(&p, &l, &j).with_reference_values(&queries), &store);
    assert!(tree.errors.is_empty(), "{}", tree.to_json().unwrap());
    let root = tree.root.as_ref().unwrap();
    for (key, state_key) in [("a", "$from_state"), ("b", "$from_query")] {
        let el = field(root, key);
        let from_call = field(field(el, "props"), "a");
        assert_eq!(steps(from_call).len(), 1);
        assert_eq!(
            field(field(steps(from_call).first().unwrap(), "params"), "x"),
            &Value::Str("chosen by data".into())
        );
        assert!(
            steps(field(field(el, "props"), "b")).is_empty(),
            "finished custom steps cannot be forged"
        );
        store.set(state_key, Some(from_call.clone())).unwrap();
    }
    let read = parse(
        "root=Box($from_state,$from_query,Action([$from_state,$from_query]))",
        &l,
        None,
    );
    let retained = evaluate_tree(input(&read, &l, &j), &store);
    let root = retained.root.as_ref().unwrap();
    assert_eq!(steps(field(root, "a")).len(), 1);
    assert_eq!(steps(field(root, "b")).len(), 1);
    assert_eq!(steps(field(root, "c")).len(), 2);
    free_json(j);
}
