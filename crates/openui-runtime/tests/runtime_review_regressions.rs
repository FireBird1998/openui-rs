//! Public-interface regressions from the independent rest-rt review.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Library, Limits, parse,
    parse_json,
};
use openui_runtime::{
    JsObject, Store, TreeInput, Value, evaluate_tree_with_limits, resume_action,
    run_action_with_limits,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}
fn chain(n: usize) -> Value {
    (0..n).fold(Value::Bool(false), |v, _| Value::Arr(Arc::new(vec![v])))
}
#[test]
fn changed_scalar_state_earns_credit_after_each_mutation_resume() {
    let l = library();
    let p = parse("$a=null\nm=Mutation(\"m\",{})\nroot=Box(1)", &l, None);
    let step = r#"{"type":"run","statementId":"m","refType":"mutation"},{"type":"continue_conversation","message":"x"}"#;
    let plan =
        Value::from_json(&parse_json(&format!("{{\"steps\":[{}]}}", [step; 3].join(","))).unwrap());
    for sizes in [
        [70_000, 130_000, 70_000],
        [70_000; 3],
        [70_000, 70_001, 70_002],
    ] {
        for names in [1, 24] {
            let mut store = Store::new();
            let _ = store.initialize(
                p.state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                None,
            );
            for i in 1..names {
                store
                    .set(&format!("$a{i}"), Some(Value::Str("s".repeat(1000))))
                    .unwrap();
            }
            let mut run = run_action_with_limits(
                TreeInput::new(&p),
                &mut store,
                Some(&plan),
                "",
                None,
                Limits::DEFAULT,
            );
            assert!(run.is_suspended());
            for size in sizes {
                store.set("$a", Some(Value::Str("z".repeat(size)))).unwrap();
                let token = run.continuation.take().unwrap();
                run = resume_action(TreeInput::new(&p), &mut store, &token, true).unwrap();
                assert!(run.failure.is_none(), "{size}: {:?}", run.failure);
                assert!(!run.size_limited && !run.depth_limited);
                assert!(
                    run.events
                        .iter()
                        .any(|e| e.to_json().unwrap().contains(&"z".repeat(size)))
                );
                if names == 1 {
                    assert_eq!(run.events[0].to_json().unwrap().len(), size + 93);
                }
            }
            assert!(!run.is_suspended());
        }
    }
}
#[test]
fn scalar_credit_tracks_consecutive_values_and_keys_without_recrediting_unchanged_reads() {
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT
            .with_result_floor(0)
            .with_amplification(1)
            .with_work(1_000_000),
    );
    session.with_evaluator(|e| {
        let mut bytes = 0;
        for i in 0..32 {
            let key = format!("$name{i}");
            assert!(e.read_state_key(&key));
            bytes += openui_lang::json_string_len(&key) + 2;
            for value in [
                Value::Null,
                Value::Num(123.),
                Value::Str("abc".into()),
                Value::Str("xyz".into()),
                Value::Arr(Arc::new(vec![Value::Num(1.)])),
                Value::Str("xyz".into()),
                Value::Str("abc".into()),
            ] {
                assert!(e.read_state_value(&key, &value));
                bytes += openui_runtime::value_to_json(&value).unwrap().len();
                let before = test_alloc::allocation_calls();
                for _ in 0..20 {
                    assert!(e.read_state_value(&key, &value));
                    assert!(e.read_state_key(&key));
                }
                assert_eq!(
                    test_alloc::allocation_calls(),
                    before,
                    "unchanged value/key copied"
                );
            }
        }
        assert!(e.reserve_result(bytes));
        assert!(!e.reserve_result(1));
    });
}
#[test]
fn shallow_reads_ignore_unvisited_deep_siblings_in_state_and_raw_or_prepared_queries() {
    for depth in [4, 7, 14] {
        for expr in ["X.n", "X.length", "@Count(X)", "X?1:2", "X==1"] {
            let l = library();
            let mut o = JsObject::new();
            o.define("n", Value::Num(39.));
            o.define("list", chain(60));
            let value = Value::Obj(Arc::new(o));
            let mut results = Vec::new();
            for mode in 0..3 {
                let source = if mode == 0 {
                    format!("$s=0\nroot=Box({})", expr.replace('X', "$s"))
                } else {
                    format!(
                        "q=Query(\"q\",{{}},null)\nroot=Box({})",
                        expr.replace('X', "q")
                    )
                };
                let p = parse(&source, &l, None);
                let mut store = Store::new();
                let mut refs = JsObject::new();
                if mode == 0 {
                    store.set("$s", Some(value.clone())).unwrap();
                } else {
                    refs.define("q", value.clone());
                }
                let prepared = openui_runtime::PreparedReferenceValues::new(&refs);
                let input = TreeInput::new(&p).with_library(&l);
                let input = if mode == 2 {
                    input.with_prepared_reference_values(&prepared)
                } else {
                    input.with_reference_values(&refs)
                };
                let out =
                    evaluate_tree_with_limits(input, &store, Limits::DEFAULT.with_depth(depth));
                assert!(
                    !out.depth_limited && !out.size_limited,
                    "{mode} {depth} {expr}"
                );
                results.push(out.to_json().unwrap());
            }
            assert_eq!(results[0], results[1]);
            assert_eq!(results[1], results[2]);
        }
    }
}
#[test]
fn default_limit_query_receivers_remain_lazy_past_the_old_regression_boundary() {
    let l = library();
    for n in [760, 761, 762, 800, 1100] {
        for expr in ["q.n", "q.length", "@Count(q)", "q?1:2"] {
            let p = parse(
                &format!("q=Query(\"q\",{{}},null)\nroot=Box({expr})"),
                &l,
                None,
            );
            let mut refs = JsObject::new();
            refs.define("q", chain(n));
            let prepared = openui_runtime::PreparedReferenceValues::new(&refs);
            for input in [
                TreeInput::new(&p).with_reference_values(&refs),
                TreeInput::new(&p).with_prepared_reference_values(&prepared),
            ] {
                let out = evaluate_tree_with_limits(input, &Store::new(), Limits::DEFAULT);
                assert!(!out.depth_limited && !out.size_limited, "{n} {expr}");
            }
        }
    }
    let p = parse("q=Query(\"q\",{},null)\nroot=Box(\"\"+q)", &l, None);
    let mut refs = JsObject::new();
    refs.define("q", chain(1100));
    assert!(
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_reference_values(&refs),
            &Store::new(),
            Limits::DEFAULT
        )
        .depth_limited
    );
}
thread_local! {
    static SESSION: RefCell<Option<Rc<EvaluationSession<'static>>>> = const { RefCell::new(None) };
    static MODE: Cell<u8> = const { Cell::new(0) };
}
struct Outer;
struct Inner;
impl EvaluationScope for Inner {
    fn state(&self, _: &str, e: &ExpressionEvaluator<'_>) -> Option<Value> {
        match MODE.with(Cell::get) {
            2 => {
                e.charge_work(1000);
            }
            3 => {
                e.reserve_result(1000);
            }
            _ => {}
        }
        Some(Value::Num(2.))
    }
}
impl EvaluationScope for Outer {
    fn state(&self, name: &str, e: &ExpressionEvaluator<'_>) -> Option<Value> {
        if MODE.with(Cell::get) == 1 && name == "$x" {
            return e.evaluate(&Ast::StateRef("$y".into()));
        }
        let s = SESSION.with(|s| s.borrow().as_ref().unwrap().clone());
        s.with_scope(&Inner, |sub| {
            sub.evaluate(&Ast::StateRef("$z".into())).value
        })
    }
}
#[test]
fn own_session_with_scope_reentry_uses_its_suspended_account_and_limits() {
    for mode in 0..4 {
        MODE.with(|m| m.set(mode));
        let s = Rc::new(EvaluationSession::with_limits(
            &Outer,
            0,
            Limits::DEFAULT.with_work(100).with_result_size(100),
        ));
        SESSION.with(|slot| *slot.borrow_mut() = Some(s.clone()));
        let out = s.evaluate(&Ast::StateRef("$x".into()));
        if mode < 2 {
            assert_eq!(out.value, Some(Value::Num(2.)));
            assert!(!out.size_limited && !out.depth_limited);
        } else {
            assert!(out.size_limited);
            assert!(s.flags().size_limited);
        }
        SESSION.with(|slot| slot.borrow_mut().take());
    }
}
#[test]
fn undefined_snapshot_round_trip_restores_defaults_and_omits_undeclared_names() {
    let l = library();
    let p = parse("$x=5\n$y=1\nroot=Box($x)", &l, None);
    let mut store = Store::new();
    let _ = store.initialize(
        p.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        None,
    );
    store.set("$x", None).unwrap();
    store.set("$z", None).unwrap();
    let json = store.snapshot_json().unwrap();
    assert_eq!(json, r#"{"$y":1}"#);
    let saved = Value::from_json(&parse_json(&json).unwrap());
    let Value::Obj(saved) = &saved else {
        panic!("snapshot")
    };
    let mut restored = Store::new();
    let _ = restored.initialize(
        p.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        saved.iter().map(|(k, v)| (k.as_str(), Some(v))),
    );
    assert_eq!(restored.get("$x"), Some(Value::Num(5.)));
    assert_eq!(restored.get("$z"), None);
    assert!(!restored.snapshot().has("$z"));
}

#[test]
fn a_scalar_read_after_undefined_earns_its_own_credit() {
    struct State(Option<Value>);
    impl EvaluationScope for State {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.0.clone()
        }
    }
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT
            .with_result_floor(0)
            .with_amplification(1)
            .with_work(1_000_000),
    );
    for value in [
        Some(Value::Str("abc".into())),
        None,
        Some(Value::Str("abc".into())),
    ] {
        let scope = State(value.clone());
        session.with_scope(&scope, |sub| {
            let out = sub.evaluate(&Ast::StateRef("$a".into()));
            assert_eq!(out.value, value);
            assert!(!out.size_limited && !out.depth_limited);
        });
    }
    session.with_evaluator(|e| {
        assert!(e.reserve_result(10));
        assert!(!e.reserve_result(1));
    });
}
