//! Exact total-state admission is shared by every write route.
use openui_runtime::{
    Ast, BindingWriteError, JsObject, Limits, Store, StoreError, TreeInput, Value,
    initialize_field_default, run_action, set_field_value, write_binding,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn limits(room: usize) -> Limits {
    Limits::DEFAULT.with_result_size(room)
}
fn value(json: &str) -> Value {
    Value::from_json(&openui_lang::parse_json(json).unwrap())
}
fn encode(value: &Value) -> String {
    openui_runtime::value_to_json_with_limits(value, limits(100_000).with_depth(20_000)).unwrap()
}

fn json(store: &Store) -> String {
    let snapshot = store.snapshot();
    let mut fields = JsObject::new();
    for (key, v) in snapshot.iter() {
        fields.define(key, v.clone().unwrap_or(Value::Null));
    }
    encode(&Value::Obj(Arc::new(fields)))
}
fn unchanged(
    store: &Store,
    snapshot: &openui_runtime::StoreSnapshot,
    version: u64,
    pristine: bool,
    name: &str,
) {
    assert!(Arc::ptr_eq(snapshot, &store.snapshot()));
    assert_eq!(store.version(), version);
    assert_eq!(store.is_pristine(name), pristine);
}
#[test]
fn escaped_names_and_aliases_count_exactly_at_each_position() {
    let item = value(r#"[[],{"a":"\n😀"}]"#);
    for name in ["x", "\"\n😀", "__proto__", "0"] {
        let mut object = JsObject::new();
        object.define(name, item.clone());
        let bytes = encode(&Value::Obj(Arc::new(object))).len();
        let mut store = Store::with_limits(limits(bytes));
        assert!(store.set(name, Some(item.clone())).unwrap());
        assert_eq!(json(&store).len(), bytes);
        let before = store.snapshot();
        let version = store.version();
        assert!(matches!(
            store.set("other", Some(item.clone())),
            Err(StoreError::StateTooLarge)
        ));
        unchanged(&store, &before, version, false, name);
        let mut below = Store::with_limits(limits(bytes - 1));
        assert!(matches!(
            below.set(name, Some(item.clone())),
            Err(StoreError::StateTooLarge)
        ));
        assert!(below.snapshot().is_empty());
    }
    let bytes = format!("{{\"a\":{},\"b\":{}}}", encode(&item), encode(&item)).len();
    let mut store = Store::with_limits(limits(bytes));
    store.set("a", Some(item.clone())).unwrap();
    store.set("b", Some(item.clone())).unwrap();
    assert_eq!(json(&store).len(), bytes);
    assert!(store.set("c", Some(Value::Num(0.))).is_err());
    store.set("b", Some(Value::Null)).unwrap();
    store.set("c", Some(Value::Num(0.))).unwrap();
}
#[test]
fn pristine_only_names_and_refused_replacements_are_atomic() {
    let mut store = Store::with_limits(limits(10));
    assert!(!store.set_pristine("a", None).unwrap()); // {"a":null}, ten bytes
    assert!(store.is_pristine("a"));
    let before = store.snapshot();
    let version = store.version();
    assert!(store.set_pristine("b", None).is_err());
    assert!(!store.is_pristine("b"));
    assert!(store.set("a", Some(Value::Str("longer".into()))).is_err());
    unchanged(&store, &before, version, true, "a");
    assert!(!store.set("a", None).unwrap()); // removes pristine-only retention
    assert!(!store.is_pristine("a"));
    assert!(store.set_pristine("b", None).is_ok());
    store.dispose();
    assert!(store.set_pristine("c", None).is_ok());
}
#[test]
fn persisted_initialization_loads_what_fits_and_reports_each_refusal() {
    let a = Value::Num(1.);
    let b = Value::Num(22.);
    let huge = Value::Str("x".repeat(100));
    let mut store = Store::with_limits(limits(13)); // {"a":1,"b":22} is fourteen
    let result = store.initialize([("d", Some(&huge))], [("a", Some(&a)), ("b", Some(&b))]);
    assert!(result.changed);
    assert_eq!(result.refused.len(), 2);
    assert_eq!(result.refused[0].name, "b");
    assert_eq!(result.refused[1].name, "d");
    assert_eq!(json(&store), r#"{"a":1}"#);
    assert_eq!(store.version(), 1);
    let before = store.snapshot();
    let version = store.version();
    let refused = store.initialize([], [("a", Some(&huge))]);
    assert_eq!(refused.refused.len(), 1);
    unchanged(&store, &before, version, false, "a");
}
#[test]
fn bindings_forms_action_sets_and_resets_use_the_store_cap() {
    let program = openui_lang::parse("$a=123456\n$b=123456", &Default::default(), None);
    let marker =
        value(r#"{"__reactive":"assign","target":"b","expr":{"k":"StateRef","n":"$value"}}"#);
    let mut store = Store::with_limits(limits(14));
    store.set("a", Some(Value::Num(1.))).unwrap();
    let before = store.snapshot();
    let version = store.version();
    let refused = write_binding(
        TreeInput::new(&program),
        &mut store,
        &marker,
        Value::Str("long".into()),
    );
    assert!(matches!(refused, Err(BindingWriteError::Store(_))));
    unchanged(&store, &before, version, false, "a");
    for form in [None, Some("f")] {
        assert!(matches!(
            set_field_value(&mut store, form, None, "b", Some(Value::Num(1.)), false),
            Err(BindingWriteError::Store(_))
        ));
        assert!(
            initialize_field_default(&mut store, form, None, "b", Some(Value::Num(1.)), false)
                .is_err()
        );
        unchanged(&store, &before, version, false, "a");
    }
    let plan = value(
        r#"{"steps":[{"type":"set","target":"b","valueAST":{"k":"Str","v":"long"}},{"type":"set","target":"a","valueAST":{"k":"Num","v":9}}]}"#,
    );
    let run = run_action(TreeInput::new(&program), &mut store, Some(&plan), "", None);
    assert!(matches!(
        run.failure.unwrap().cause,
        BindingWriteError::Store(_)
    ));
    unchanged(&store, &before, version, false, "a");
    let mut store = Store::with_limits(limits(14));
    store.set("$a", Some(Value::Num(1.))).unwrap();
    let before = store.snapshot();
    let version = store.version();
    let reset = value(r#"{"steps":[{"type":"reset","targets":["$a","$b"]}]}"#);
    let run = run_action(TreeInput::new(&program), &mut store, Some(&reset), "", None);
    assert!(run.failure.is_some());
    unchanged(&store, &before, version, false, "$a");
}
#[test]
fn deep_admission_snapshot_serialization_and_disposal_use_a_small_stack() {
    std::thread::Builder::new()
        .name("deep_state::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut deep = Value::Null;
            for _ in 0..10_000 {
                deep = Value::Arr(Arc::new(vec![deep]));
            }
            let mut store = Store::new();
            assert!(matches!(
                store.set("d", Some(deep.clone())),
                Err(StoreError::DepthLimited)
            ));
            assert_eq!(store.initialize([], [("d", Some(&deep))]).refused.len(), 1);
            let mut store = Store::with_limits(limits(100_000).with_depth(20_000));
            store.set("d", Some(deep.clone())).unwrap();
            let snapshot = store.snapshot();
            assert_eq!(
                encode(snapshot.get("d").unwrap().as_ref().unwrap()).len(),
                20_004
            );
            drop(store.get("d"));
            store.dispose();
            drop(snapshot);
            drop(deep);
            let ast = Value::Ast(Arc::new(Ast::Null));
            drop(ast);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn refused_last_persisted_duplicate_preserves_pristine() {
    let mut store = Store::with_limits(limits(16));
    let small = Value::Num(1.);
    store.set_pristine("a", Some(small.clone())).unwrap();
    let before = store.snapshot();
    let version = store.version();
    let huge = Value::Str("x".repeat(100));
    let result = store.initialize([], [("a", Some(&small)), ("a", Some(&huge))]);
    assert_eq!(result.refused.len(), 1);
    unchanged(&store, &before, version, true, "a");
}

#[test]
fn resumed_sets_use_current_total_state_and_consume_the_continuation_on_a_cut() {
    let program = openui_lang::parse("m=Mutation(\"m\",{})", &Default::default(), None);
    let action = value(
        r#"{"steps":[{"type":"open_url","url":"before"},{"type":"run","refType":"mutation","statementId":"m"},{"type":"set","target":"b","valueAST":{"k":"Num","v":2}},{"type":"open_url","url":"after"}]}"#,
    );
    for (cap, cut) in [(13, false), (12, true)] {
        let mut store = Store::with_limits(limits(cap));
        store.set("a", Some(Value::Num(1.))).unwrap();
        let first = run_action(
            TreeInput::new(&program),
            &mut store,
            Some(&action),
            "",
            None,
        );
        assert_eq!(first.events.len(), 2);
        let continuation = first.continuation.unwrap();
        let before = store.snapshot();
        let version = store.version();
        let resumed = openui_runtime::resume_action(
            TreeInput::new(&program),
            &mut store,
            &continuation,
            true,
        )
        .unwrap();
        assert_eq!(resumed.size_limited, cut);
        if cut {
            assert!(resumed.failure.is_some());
            assert!(resumed.events.is_empty());
            unchanged(&store, &before, version, false, "a");
        } else {
            assert!(resumed.failure.is_none());
            assert_eq!(resumed.events.len(), 1);
            assert_eq!(json(&store).len(), cap);
        }
        assert!(resumed.continuation.is_none());
        assert!(matches!(
            openui_runtime::resume_action(
                TreeInput::new(&program),
                &mut store,
                &continuation,
                true
            ),
            Err(openui_runtime::ActionResumeError::Consumed)
        ));
    }
}
#[test]
fn state_accounting_does_not_rescan_unrelated_names() {
    let mut observations = Vec::new();
    for count in [8, 128, 2048] {
        let mut store = Store::new();
        for i in 0..count {
            store.set(&format!("k{i}"), Some(Value::Num(0.))).unwrap();
        }
        let input = value(r#"["abc",[1,2],{"x":true}]"#);
        store.set("written", Some(Value::Null)).unwrap();
        let before = test_alloc::allocation_calls();
        store.set("written", Some(input)).unwrap();
        observations.push(test_alloc::allocation_calls() - before);
    }
    assert!(observations.iter().all(|n| *n <= 12), "{observations:?}");
    assert!(observations[2] <= observations[0], "{observations:?}");
    let mut calls = Vec::new();
    for n in [64, 128, 256] {
        let input = Value::Arr(Arc::new((0..n).map(|_| value("{\"x\":[1,2,3]}")).collect()));
        let mut store = Store::new();
        let before = test_alloc::allocation_calls();
        store.set("written", Some(input)).unwrap();
        calls.push(test_alloc::allocation_calls() - before);
    }
    assert!(calls.windows(2).all(|v| v[1] <= 2 * v[0] + 16), "{calls:?}");
    println!(
        "unrelated names [8,128,2048]: {observations:?}; written bodies [64,128,256]: {calls:?}"
    );
}

#[test]
fn a_reset_uses_its_final_size_when_one_target_shrinks() {
    let p = openui_lang::parse("$a=123456\n$b=0", &Default::default(), None);
    let mut store = Store::with_limits(limits(20));
    store.set("$a", Some(Value::Num(0.))).unwrap();
    store.set("$b", Some(Value::Num(123456.))).unwrap();
    let action = value(r#"{"steps":[{"type":"reset","targets":["$a","$b"]}]}"#);
    let run = run_action(TreeInput::new(&p), &mut store, Some(&action), "", None);
    assert!(run.failure.is_none(), "{:?}", run.failure);
    assert_eq!(store.get("$a"), Some(Value::Num(123456.)));
    assert_eq!(store.get("$b"), Some(Value::Num(0.)));
    assert!(store.is_pristine("$a") && store.is_pristine("$b"));
    assert_eq!(json(&store).len(), 20);
}

#[test]
fn state_name_and_value_measurement_share_one_work_budget() {
    for (work, fits) in [(4, true), (3, false)] {
        let mut store = Store::with_limits(limits(32).with_work(work));
        let before = store.snapshot();
        // Two raw key bytes plus the core's two units for a one-byte number.
        let result = store.set("xy", Some(Value::Num(1.)));
        assert_eq!(result.is_ok(), fits);
        if !fits {
            assert!(matches!(result, Err(StoreError::WorkLimited)));
            unchanged(&store, &before, 0, false, "xy");
        }
    }
}

#[test]
fn saturating_totals_refuse_aliases_even_at_the_largest_configured_cap() {
    let limits = limits(usize::MAX).with_work(usize::MAX).with_depth(128);
    let mut huge = Value::Null;
    for _ in 0..usize::BITS.saturating_sub(3) {
        huge = Value::Arr(Arc::new(vec![huge.clone(), huge]));
    }
    let medium = if let Value::Arr(items) = &huge {
        items[0].clone()
    } else {
        unreachable!()
    };
    let mut store = Store::with_limits(limits);
    store.set("a", Some(huge.clone())).unwrap();
    let before = store.snapshot();
    let version = store.version();
    assert!(matches!(
        store.set("b", Some(huge.clone())),
        Err(StoreError::StateTooLarge)
    ));
    unchanged(&store, &before, version, false, "a");
    let mut p = openui_lang::ParseResult::default();
    p.state_declarations.define("$a", huge.clone());
    p.state_declarations.define("$b", Value::Num(0.));
    let mut store = Store::with_limits(limits);
    store.set("$a", Some(Value::Num(0.))).unwrap();
    store.set("$b", Some(huge)).unwrap();
    let reset = value(r#"{"steps":[{"type":"reset","targets":["$a","$b"]}]}"#);
    let run = openui_runtime::run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&reset),
        "",
        None,
        limits,
    );
    assert!(run.failure.is_none(), "{:?}", run.failure);
    assert_eq!(store.get("$b"), Some(Value::Num(0.)));
    // The final reset counter must recover its exact value even if the
    // intermediate replacement order saturated usize.
    let before = store.snapshot();
    let version = store.version();
    assert!(matches!(
        store.set("$c", Some(medium)),
        Err(StoreError::StateTooLarge)
    ));
    unchanged(&store, &before, version, true, "$a");
}
