//! Host payloads receive the input credit of supplied actions and state reads.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Store, TreeInput, Value, evaluate_tree, run_action};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn value(s: &str) -> Value {
    Value::from_json(&parse_json(s).unwrap())
}
#[test]
fn held_state_above_the_floor_still_reaches_every_host_event() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let parsed = parse(
        "$rows=[]\n$n=0\nroot=Box(Action([@ToAssistant(\"send\"),@Set($n,$n+1),@OpenUrl(\"u\")]))",
        &library,
        None,
    );
    for size in [65_520, 65_530, 112_000, 2 * 1024 * 1024] {
        let mut store = Store::new();
        store.set("$s", Some(Value::Str("s".repeat(size)))).unwrap();
        store.set("$n", Some(Value::Num(0.))).unwrap();
        let tree = evaluate_tree(TreeInput::new(&parsed).with_library(&library), &store);
        let Value::Element(ref root) = tree.root.unwrap() else {
            panic!("element")
        };
        for form in [None, Some("contact")] {
            let r = run_action(
                TreeInput::new(&parsed).with_library(&library),
                &mut store,
                root.props.get("a"),
                "",
                form,
            );
            assert!(r.failure.is_none(), "{size} bytes: {:?}", r.failure);
            assert_eq!(r.events.len(), 2);
            let r = run_action(TreeInput::new(&parsed), &mut store, None, "default", form);
            assert!(r.failure.is_none(), "{size} default: {:?}", r.failure);
            assert_eq!(r.events.len(), 1);
        }
    }
}
#[test]
fn supplied_large_legacy_params_and_coercible_types_earn_input_credit() {
    let parsed = Default::default();
    let mut store = Store::new();
    let s = "x".repeat(70_000);
    let actions = [
        value(&format!(r#"{{"type":"x","params":{{"data":"{s}"}}}}"#)),
        value(&format!(r#"{{"type":["{s}"]}}"#)),
    ];
    let defs = parse_json("{}").unwrap();
    for action in actions {
        let r = run_action(
            TreeInput::new(&parsed).with_actions(&defs),
            &mut store,
            Some(&action),
            "",
            None,
        );
        assert!(r.failure.is_none(), "{:?}", r.failure);
        assert_eq!(r.events.len(), 1);
    }
}
#[test]
fn a_large_snapshot_emits_before_an_oversized_set_is_refused() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let p = parse(
        "$n=0\nroot=Box(Action([@ToAssistant(\"before\"),@Set($n,$s+$s+$s+$s+$s),@Set($n,99),@ToAssistant(\"never\")]))",
        &l,
        None,
    );
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("s".repeat(2 * 1024 * 1024))))
        .unwrap();
    s.set("$n", Some(Value::Num(0.))).unwrap();
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    let Value::Element(ref root) = tree.root.unwrap() else {
        panic!()
    };
    let r = run_action(TreeInput::new(&p), &mut s, root.props.get("a"), "", None);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.failure.as_ref().unwrap().step, Some(1));
    assert!(r.size_limited);
    assert_eq!(s.get("$n"), Some(Value::Num(0.)));
    let snapshot = s.snapshot();
    s.set("$huge", Some(Value::Str("x".repeat(8 * 1024 * 1024))))
        .unwrap();
    let r = run_action(TreeInput::new(&p), &mut s, None, "", None);
    assert!(r.size_limited);
    assert!(r.events.is_empty());
    assert_eq!(s.get("$n"), snapshot.get("$n").cloned().flatten());
}
#[test]
fn row_snapshots_reach_custom_query_mutation_and_legacy_handoffs() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "q=Query(\"q\",{})\nm=Mutation(\"m\",{})\nroot=Box(Action([@Go(),@Run(q),@Run(m)]))",
        &l,
        None,
    );
    let mut s = Store::new();
    let row = value(r#"{"id":1,"title":"a reasonably sized row of persisted data"}"#);
    s.set(
        "$rows",
        Some(Value::Arr(std::sync::Arc::new(vec![row; 2000]))),
    )
    .unwrap();
    let tree = evaluate_tree(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &s,
    );
    let Value::Element(ref root) = tree.root.unwrap() else {
        panic!()
    };
    let r = run_action(
        TreeInput::new(&p).with_actions(&j["actions"]),
        &mut s,
        root.props.get("a"),
        "",
        Some("contact"),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert_eq!(r.events.len(), 3);
    assert!(r.is_suspended());
    let a = value(r#"{"type":"legacy"}"#);
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert!(r.failure.is_none());
    assert_eq!(r.events.len(), 1);
}

#[test]
fn held_state_property_names_earn_credit_before_copying() {
    let p = Default::default();
    for bytes in [70_000, 2 * 1024 * 1024] {
        let mut s = Store::new();
        s.set(&"k".repeat(bytes), Some(Value::Num(1.))).unwrap();
        let r = run_action(TreeInput::new(&p), &mut s, None, "", None);
        assert!(r.failure.is_none(), "{bytes}-byte name: {:?}", r.failure);
        assert_eq!(r.events.len(), 1);
    }
}

#[test]
fn undefined_snapshot_entries_do_not_consume_payload_room() {
    use openui_runtime::{Limits, run_action_with_limits};
    let p = Default::default();
    let mut s = Store::new();
    let name = "k".repeat(70_000);
    assert!(s.initialize([], [(name.as_str(), None)]).refused.is_empty());
    assert!(s.snapshot().has_own(&name));
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        None,
        "",
        None,
        Limits::DEFAULT.with_result_size(256).with_work(5000),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert_eq!(r.events.len(), 1);
    assert!(r.events[0].to_json().contains(r#""formState":{}"#));
}
