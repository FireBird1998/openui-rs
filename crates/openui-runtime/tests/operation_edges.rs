//! Adjacent work, nesting, output and total-state boundaries for public writes.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{
    BindingWriteError, JsObject, Limits, Store, TreeInput, Value, evaluate_tree_with_limits,
    resume_action, run_action_with_limits, set_field_value_with_limits, write_binding_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
#[derive(Clone, Copy, Debug)]
enum Bound {
    Work,
    Depth,
    Room,
    State,
}
fn configured(bound: Bound, n: usize) -> Limits {
    let limits = Limits::DEFAULT
        .with_work(1_000_000)
        .with_depth(100)
        .with_result_size(100_000);
    match bound {
        Bound::Work => limits.with_work(n),
        Bound::Depth => limits.with_depth(n),
        Bound::Room | Bound::State => limits.with_result_size(n),
    }
}
fn nested(n: usize) -> Value {
    let mut v = Value::Str("x".repeat(128));
    for _ in 0..n {
        v = Value::Arr(Arc::new(vec![v]));
    }
    v
}
fn marker() -> Value {
    Value::from_json(
        &parse_json(r#"{"__reactive":"assign","target":"x","expr":{"k":"StateRef","n":"$value"}}"#)
            .unwrap(),
    )
}
fn first(mut fits: impl FnMut(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, 100_000);
    assert!(fits(hi));
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) { hi = mid } else { lo = mid + 1 }
    }
    lo
}
fn check_cut(result: Result<bool, BindingWriteError>, bound: Bound) -> bool {
    match result {
        Ok(_) => true,
        Err(BindingWriteError::Cut(f)) => {
            assert!(if matches!(bound, Bound::Depth) {
                f.depth_limited
            } else {
                f.size_limited
            });
            false
        }
        Err(BindingWriteError::Store(_)) if matches!(bound, Bound::State) => false,
        other => panic!("unexpected {bound:?} refusal: {other:?}"),
    }
}
#[test]
fn tree_binding_form_and_store_have_adjacent_limit_edges() {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse("root=Box($source)", &library, None);
    let value = nested(12);
    let marker = marker();
    let action = Value::from_json(
        &parse_json(
            r#"{"steps":[{"type":"set","target":"x","valueAST":{"k":"StateRef","n":"$source"}}]}"#,
        )
        .unwrap(),
    );
    for route in ["tree", "binding", "form", "store", "initialize", "action"] {
        for bound in [Bound::Work, Bound::Depth, Bound::Room, Bound::State] {
            if route == "tree" && matches!(bound, Bound::State) {
                continue;
            }
            let run = |n| {
                let operation = configured(bound, n);
                let state =
                    if matches!(bound, Bound::State) || route == "store" || route == "initialize" {
                        operation
                    } else {
                        configured(Bound::Room, 100_000)
                    };
                let mut store = Store::with_limits(state);
                match route {
                    "store" => store.set("x", Some(value.clone())).is_ok(),
                    "initialize" => store
                        .initialize([], [("x", Some(&value))])
                        .refused
                        .is_empty(),
                    "tree" => {
                        store.set("$source", Some(value.clone())).unwrap();
                        let r = evaluate_tree_with_limits(TreeInput::new(&p), &store, operation);
                        !(r.size_limited || r.depth_limited)
                    }
                    "binding" => check_cut(
                        write_binding_with_limits(
                            TreeInput::new(&ParseResult::default()),
                            &mut store,
                            &marker,
                            value.clone(),
                            if matches!(bound, Bound::State) {
                                Limits::DEFAULT
                            } else {
                                operation
                            },
                        ),
                        bound,
                    ),
                    "action" => {
                        if store.set("$source", Some(value.clone())).is_err() {
                            return false;
                        }
                        let before = store.snapshot();
                        let version = store.version();
                        let r = run_action_with_limits(
                            TreeInput::new(&p),
                            &mut store,
                            Some(&action),
                            "",
                            None,
                            if matches!(bound, Bound::State) {
                                Limits::DEFAULT
                            } else {
                                operation
                            },
                        );
                        if let Some(failure) = r.failure {
                            assert!(Arc::ptr_eq(&before, &store.snapshot()));
                            assert_eq!(store.version(), version);
                            assert!(r.size_limited || r.depth_limited);
                            check_cut(Err(failure.cause), bound)
                        } else {
                            true
                        }
                    }
                    _ => check_cut(
                        set_field_value_with_limits(
                            &mut store,
                            Some("f"),
                            None,
                            "x",
                            Some(value.clone()),
                            false,
                            if matches!(bound, Bound::State) {
                                Limits::DEFAULT
                            } else {
                                operation
                            },
                        )
                        .map(|w| w.changed),
                        bound,
                    ),
                }
            };
            let edge = first(run);
            assert!(edge > 0, "{route}/{bound:?}");
            assert!(run(edge), "{route}/{bound:?}/{edge}");
            assert!(!run(edge - 1), "{route}/{bound:?}/{edge}");
            println!("{route} {bound:?} {edge}");
        }
    }
}
#[test]
fn resumed_writes_have_adjacent_limits_without_refreshing_the_run() {
    let p = parse(
        "q=Query(\"q\",{},null)\nm=Mutation(\"m\",{})",
        &Default::default(),
        None,
    );
    let mut references = JsObject::new();
    references.define("q", nested(12));
    let mut set = JsObject::new();
    set.define("type", Value::Str("set".into()));
    set.define("target", Value::Str("x".into()));
    set.define(
        "valueAST",
        Value::Ast(Arc::new(openui_lang::Ast::Ref("q".into()))),
    );
    let mutation = Value::from_json(
        &parse_json(r#"{"type":"run","statementId":"m","refType":"mutation"}"#).unwrap(),
    );
    let mut plan = JsObject::new();
    plan.define(
        "steps",
        Value::Arr(Arc::new(vec![mutation, Value::Obj(Arc::new(set))])),
    );
    let plan = Value::Obj(Arc::new(plan));
    for bound in [Bound::Work, Bound::Depth, Bound::Room, Bound::State] {
        let run = |n, at_edge: bool| {
            let operation = configured(bound, n);
            let mut store = Store::with_limits(if matches!(bound, Bound::State) {
                operation
            } else {
                configured(Bound::Room, 100_000)
            });
            let first = run_action_with_limits(
                TreeInput::new(&p),
                &mut store,
                Some(&plan),
                "",
                None,
                if matches!(bound, Bound::State) {
                    Limits::DEFAULT
                } else {
                    operation
                },
            );
            let Some(c) = first.continuation else {
                assert!(!at_edge, "edge must suspend first: {bound:?}/{n}");
                return false;
            };
            let before = store.snapshot();
            let version = store.version();
            let resumed = resume_action(
                TreeInput::new(&p).with_reference_values(&references),
                &mut store,
                &c,
                true,
            )
            .unwrap();
            if resumed.failure.is_some() {
                assert!(Arc::ptr_eq(&before, &store.snapshot()));
                assert_eq!(store.version(), version);
                assert!(resumed.size_limited || resumed.depth_limited);
                false
            } else {
                true
            }
        };
        let edge = first(|n| run(n, false));
        assert!(edge > 0);
        assert!(run(edge, true));
        assert!(!run(edge - 1, true));
        println!("resume {bound:?} {edge}");
    }
}

#[test]
fn deep_values_cross_tree_binding_form_preparation_action_and_resume_on_a_small_stack() {
    std::thread::Builder::new().name("public_deep_values::worker".into()).stack_size(64*1024).spawn(|| {
        let limits=Limits::DEFAULT.with_depth(20_000).with_result_size(200_000).with_work(4_000_000);
        let library=Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap());
        let p=parse("q=Query(\"q\",{},null)\nm=Mutation(\"m\",{})\nroot=Box(q)",&library,None);
        let mut deep=Value::Null;for _ in 0..10_000 {deep=Value::Arr(Arc::new(vec![deep]));}
        let mut references=JsObject::new();references.define("q",deep.clone());
        let prepared=openui_runtime::PreparedReferenceValues::with_limits(&references,limits);
        let mut store=Store::with_limits(limits);
        store.set("$source",Some(deep.clone())).unwrap();
        let tree=evaluate_tree_with_limits(TreeInput::new(&p).with_prepared_reference_values(&prepared),&store,limits);
        assert!(!tree.size_limited&&!tree.depth_limited);drop(tree.to_json());drop(tree);
        write_binding_with_limits(TreeInput::new(&p),&mut store,&marker(),deep.clone(),limits).unwrap();
        set_field_value_with_limits(&mut store,Some("f"),None,"x",Some(deep.clone()),false,limits).unwrap();
        openui_runtime::initialize_field_default_with_limits(&mut store,None,None,"default",Some(deep.clone()),false,limits).unwrap();
        drop(openui_runtime::get_field_value(&store,Some("f"),"x"));
        let action=Value::from_json(&parse_json(r#"{"steps":[{"type":"set","target":"before","valueAST":{"k":"StateRef","n":"$source"}},{"type":"run","statementId":"m","refType":"mutation"},{"type":"set","target":"after","valueAST":{"k":"Ref","n":"q"}}]}"#).unwrap());
        let first=run_action_with_limits(TreeInput::new(&p),&mut store,Some(&action),"",None,limits);
        assert!(first.failure.is_none(),"{:?}",first.failure);let continuation=first.continuation.unwrap();
        let resumed=resume_action(TreeInput::new(&p).with_prepared_reference_values(&prepared),&mut store,&continuation,true).unwrap();
        assert!(resumed.failure.is_none(),"{:?}",resumed.failure);
        for event in first.events.iter().chain(&resumed.events){drop(event.to_json());}
        let snapshot=store.snapshot();drop(store.get("before"));store.dispose();drop(snapshot);drop(deep);drop(prepared);drop(references);
    }).unwrap().join().unwrap();
}
