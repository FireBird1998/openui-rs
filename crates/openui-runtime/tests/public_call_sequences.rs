//! Seeded public-call sequences preserve admission and continuation invariants.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{
    ActionContinuation, BindingWriteError, JsObject, Limits, Store, StoreError, TreeInput, Value,
    evaluate_tree_with_limits, get_field_value, initialize_field_default_with_limits,
    resume_action, run_action_with_limits, set_field_value_with_limits, write_binding_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn next(seed: &mut u64) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed as usize
}
fn value(text: &str) -> Value {
    Value::from_json(&parse_json(text).unwrap())
}
fn snapshot_size(store: &Store) -> usize {
    let mut fields = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        fields.define(k, v.clone().unwrap_or(Value::Null));
    }
    let mut tree = evaluate_tree_with_limits(
        TreeInput::new(&ParseResult::default()),
        &Store::new(),
        Limits::DEFAULT,
    );
    tree.root = Some(Value::Obj(Arc::new(fields)));
    tree.to_json().len() - r#"{"root":,"errors":[]}"#.len()
}
#[test]
fn thousands_of_small_random_call_sequences_never_panic_or_exceed_state_room() {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let marker =
        value(r#"{"__reactive":"assign","target":"$x","expr":{"k":"StateRef","n":"$value"}}"#);
    let set =
        value(r#"{"steps":[{"type":"set","target":"$x","valueAST":{"k":"StateRef","n":"$y"}}]}"#);
    let mut seed = 0x4f50454e5549u64;
    for _ in 0..3000 {
        let cap = 12 + next(&mut seed) % 117;
        let store_limits = Limits::DEFAULT
            .with_result_size(cap)
            .with_depth(1 + next(&mut seed) % 5);
        let limits = Limits::DEFAULT
            .with_result_size(next(&mut seed) % 129)
            .with_depth(next(&mut seed) % 6)
            .with_work(next(&mut seed) % 1000);
        let mut store = Store::with_limits(store_limits);
        let program = parse(
            &format!(
                "$x=0\nm=Mutation(\"m\",{{}})\nroot=Box({})",
                ["$x", "@Each([1,2,3],i,i+1)", "$x+1", "@Set($x,1)"][next(&mut seed) % 4]
            ),
            &library,
            None,
        );
        let mutation = value(
            r#"{"steps":[{"type":"run","statementId":"m","refType":"mutation"},{"type":"set","target":"$x","valueAST":{"k":"Num","v":9}}]}"#,
        );
        let mut pending: Vec<ActionContinuation> = Vec::new();
        for _ in 0..32 {
            let name = ["$x", "$y", "f", "__proto__"][next(&mut seed) % 4];
            let mut input = match next(&mut seed) % 4 {
                0 => Value::Null,
                1 => Value::Num((next(&mut seed) % 100) as f64),
                2 => Value::Str("x".repeat(next(&mut seed) % 140)),
                _ => value("[1,2,3]"),
            };
            for _ in 0..next(&mut seed) % 5 {
                input = Value::Arr(Arc::new(vec![input]));
            }
            let before = store.snapshot();
            let version = store.version();
            let pristine = ["$x", "$y", "f", "__proto__"].map(|n| store.is_pristine(n));
            let refused = match next(&mut seed) % 14 {
                0 => store.set(name, Some(input)).is_err(),
                1 => store.set_pristine(name, Some(input)).is_err(),
                2 => {
                    let initialized = store.initialize([], [(name, Some(&input))]);
                    !initialized.refused.is_empty()
                }
                3 => write_binding_with_limits(
                    TreeInput::new(&program),
                    &mut store,
                    &marker,
                    input,
                    limits,
                )
                .is_err(),
                4 => set_field_value_with_limits(
                    &mut store,
                    Some("f"),
                    None,
                    name,
                    Some(input),
                    false,
                    limits,
                )
                .is_err(),
                5 => initialize_field_default_with_limits(
                    &mut store,
                    None,
                    None,
                    name,
                    Some(input),
                    false,
                    limits,
                )
                .is_err(),
                6 => {
                    let r = evaluate_tree_with_limits(TreeInput::new(&program), &store, limits);
                    assert!(!r.depth_limited || limits.depth() < 6);
                    false
                }
                7 => {
                    let r = run_action_with_limits(
                        TreeInput::new(&program),
                        &mut store,
                        Some(&set),
                        "",
                        None,
                        limits,
                    );
                    if let Some(f) = r.failure {
                        match f.cause {
                            BindingWriteError::Cut(flags) => {
                                assert!(flags.size_limited || flags.depth_limited)
                            }
                            BindingWriteError::Store(StoreError::DepthLimited) => {
                                assert!(r.depth_limited)
                            }
                            BindingWriteError::Store(_) => assert!(r.size_limited),
                            _ => {}
                        }
                        true
                    } else {
                        false
                    }
                }
                8 => {
                    let r = run_action_with_limits(
                        TreeInput::new(&program),
                        &mut store,
                        Some(&mutation),
                        "",
                        None,
                        limits,
                    );
                    if let Some(c) = r.continuation {
                        pending.push(c);
                    }
                    false
                }
                9 => {
                    if let Some(c) = pending.last() {
                        resume_action(
                            TreeInput::new(&program),
                            &mut store,
                            c,
                            next(&mut seed).is_multiple_of(2),
                        )
                        .is_err()
                    } else {
                        false
                    }
                }
                10 => {
                    store.dispose();
                    false
                }
                11 => {
                    let _ = get_field_value(&store, Some("f"), name);
                    false
                }
                12 => store.set_pristine(name, None).is_err(),
                _ => {
                    let snapshot = store.snapshot();
                    drop(snapshot);
                    false
                }
            };
            if refused {
                assert!(Arc::ptr_eq(&before, &store.snapshot()));
                assert_eq!(store.version(), version);
                assert_eq!(
                    ["$x", "$y", "f", "__proto__"].map(|n| store.is_pristine(n)),
                    pristine
                );
            }
            if !store.snapshot().is_empty() {
                assert!(snapshot_size(&store) <= cap);
            }
        }
    }
}
