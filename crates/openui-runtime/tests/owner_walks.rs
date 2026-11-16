//! Finished value walks and all runtime owners release deep bodies on 64 KiB.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    Ast, JsObject, Limits, PreparedReferenceValue, Store, TreeInput, Value, run_action_with_limits,
    value_to_json_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn deep(n: usize) -> Value {
    (0..n).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v])))
}
#[test]
fn formatting_equality_and_serialization_of_deep_values_grow_the_small_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let a = deep(10_000);
            let b = deep(10_000);
            assert!(a == b);
            assert!(format!("{a:?}").starts_with("Arr("));
            assert_eq!(
                value_to_json_with_limits(&a, Limits::DEFAULT.with_depth(20_000))
                    .unwrap()
                    .len(),
                20_004
            );
            let ast = |n| {
                (0..n).fold(Ast::Null, |a, _| Ast::BinOp {
                    op: "+",
                    left: Arc::new(a),
                    right: Arc::new(Ast::Null),
                })
            };
            let a = ast(10_000);
            let b = ast(10_000);
            assert!(a == b);
            assert!(format!("{a:?}").starts_with("BinOp"));
            drop((a, b));
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn snapshot_event_pending_plan_and_prepared_token_each_drop_the_last_deep_owner() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let limits = Limits::DEFAULT
                .with_depth(50_000)
                .with_result_size(1_000_000)
                .with_work(20_000_000);
            let library = Library::from_json_schema(
                &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
            );
            let p = parse("m=Mutation(\"m\")\nroot=Box(1)", &library, None);
            let mut store = Store::with_limits(limits);
            let persisted = deep(10_000);
            assert!(
                store
                    .initialize(None, [("saved", Some(&persisted))])
                    .refused
                    .is_empty()
            );
            drop(persisted);
            assert_eq!(store.snapshot().len(), 1);
            let snapshot = store.snapshot();
            assert!(store.snapshot_json_with_limits(limits).is_ok());
            store.dispose();
            drop(snapshot);
            store.set("$data", Some(deep(10_000))).unwrap();
            let event =
                run_action_with_limits(TreeInput::new(&p), &mut store, None, "", None, limits);
            assert!(event.failure.is_none());
            assert!(!event.events.is_empty());
            for event in &event.events {
                event.to_json_with_limits(limits).unwrap();
            }
            store.dispose();
            drop(event);
            let mut run = JsObject::new();
            run.define("type", Value::Str("run".into()));
            run.define("statementId", Value::Str("m".into()));
            run.define("refType", Value::Str("mutation".into()));
            let mut plan = JsObject::new();
            plan.define(
                "steps",
                Value::Arr(Arc::new(vec![Value::Obj(Arc::new(run))])),
            );
            plan.define("retained", deep(10_000));
            let plan = Value::Obj(Arc::new(plan));
            let suspended = run_action_with_limits(
                TreeInput::new(&p),
                &mut store,
                Some(&plan),
                "",
                None,
                limits,
            );
            assert!(suspended.failure.is_none());
            assert!(suspended.is_suspended());
            drop(plan);
            store.dispose();
            drop(suspended);
            let value = deep(10_000);
            let prepared = PreparedReferenceValue::with_limits(&value, limits);
            drop(value);
            drop(prepared);
            drop(store);
        })
        .unwrap()
        .join()
        .unwrap();
}
