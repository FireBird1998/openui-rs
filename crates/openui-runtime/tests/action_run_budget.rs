// A run measures its real parsed input once and shares work between writes.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    Limits, Store, TreeInput, Value, evaluate_tree, run_action, run_action_with_limits,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn measure(n: usize, step: &str) -> usize {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let lib = Library::from_json_schema(&schema);
    let source = format!("$n=0\nroot=Box(Action([{}]))", vec![step; n].join(","));
    let parsed = parse(&source, &lib, None);
    assert!(!parsed.size_limited);
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(0.))).unwrap();
    let tree = evaluate_tree(TreeInput::new(&parsed).with_library(&lib), &store);
    assert!(!tree.size_limited);
    let Value::Element(ref root) = tree.root.unwrap() else {
        panic!("element")
    };
    let mut minimum_allocations = usize::MAX;
    for _ in 0..3 {
        let mut store = Store::new();
        store.set("$n", Some(Value::Num(0.))).unwrap();
        let before = test_alloc::allocation_calls();
        let run = run_action(
            TreeInput::new(&parsed).with_library(&lib),
            &mut store,
            root.props.get("a"),
            "",
            None,
        );
        let allocations = test_alloc::allocation_calls() - before;
        assert!(run.failure.is_none(), "{:?}", run.failure);
        assert!(
            allocations < 20 * n + 2048,
            "repeated input measurement: {allocations}"
        );
        minimum_allocations = minimum_allocations.min(allocations);
    }
    println!("{step} {n}: {minimum_allocations} allocations");
    minimum_allocations
}
#[test]
fn parsed_plans_scale_linearly_for_sets_and_resets() {
    for step in ["@Set($n,$n+1)", "@Reset($n)"] {
        let small = measure(4000, step);
        let large = measure(16000, step);
        assert!(
            (large as f64) < 5. * small as f64,
            "quadratic allocation growth: {} / {}",
            large,
            small
        );
    }
}
#[test]
fn work_exhaustion_is_shared_by_all_steps() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let lib = Library::from_json_schema(&schema);
    let source = format!(
        "$n=0\nroot=Box(Action([{}]))",
        vec!["@Set($n,$n+@Count(@Each($rows,r,r+1)))"; 100].join(",")
    );
    let p = parse(&source, &lib, None);
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(0.))).unwrap();
    store
        .set(
            "$rows",
            Some(Value::Arr(std::sync::Arc::new(vec![Value::Num(1.); 1000]))),
        )
        .unwrap();
    let Value::Element(ref root) = evaluate_tree(TreeInput::new(&p).with_library(&lib), &store)
        .root
        .unwrap()
    else {
        panic!()
    };
    let run = run_action_with_limits(
        TreeInput::new(&p).with_library(&lib),
        &mut store,
        root.props.get("a"),
        "",
        None,
        Limits::DEFAULT.with_work(100_000),
    );
    assert!(run.size_limited, "each step received a fresh allowance");
    assert!(run.failure.as_ref().is_some_and(|f| f.step.is_some()));
    assert!(matches!(store.get("$n"),Some(Value::Num(n))if n>0.));
}

#[test]
fn string_action_definitions_can_shadow_an_index_named_action() {
    let p = Default::default();
    let mut s = Store::new();
    let actions = openui_lang::JsonValue::String("z".repeat(1_000_000));
    let action = Value::from_json(&parse_json(r#"{"type":"5000"}"#).unwrap());
    let r = run_action_with_limits(
        TreeInput::new(&p).with_actions(&actions),
        &mut s,
        Some(&action),
        "",
        None,
        Limits::DEFAULT.with_work(6000),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert!(r.events.is_empty());
}

fn payload_cost(events: usize, program_steps: usize) -> usize {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let source = format!(
        "root=Box(Action([{}]),Action([{}]))",
        vec!["@ToAssistant(\"send\")"; events].join(","),
        vec!["@Set($n,$n+1)"; program_steps].join(",")
    );
    let p = parse(&source, &l, None);
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("s".repeat(80_000)))).unwrap();
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    let Some(Value::Element(root)) = &tree.root else {
        panic!()
    };
    let mut minimum_allocations = usize::MAX;
    for _ in 0..3 {
        let before = test_alloc::allocation_calls();
        let r = run_action(TreeInput::new(&p), &mut s, root.props.get("a"), "", None);
        assert!(r.failure.is_none(), "{:?}", r.failure);
        assert_eq!(r.events.len(), events);
        minimum_allocations = minimum_allocations.min(test_alloc::allocation_calls() - before);
    }
    minimum_allocations
}
#[test]
fn payload_admission_scales_with_input_plus_events_times_state() {
    let small = payload_cost(8, 5000);
    let large = payload_cost(32, 20000);
    println!("payload allocations: {:?} -> {:?}", small, large);
    assert!(
        large < small * 5,
        "nonlinear allocation work: {:?} / {:?}",
        large,
        small
    );
}

#[test]
fn twenty_thousand_parsed_steps_fit_the_default_run_budget() {
    for step in ["@Set($n,$n+1)", "@Reset($n)"] {
        measure(20000, step);
    }
}
