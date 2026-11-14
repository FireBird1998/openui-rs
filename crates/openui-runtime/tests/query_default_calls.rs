//! Query registration calls, representation boundaries and supplied arguments.
use openui_lang::{JsonValue, Library, parse, parse_json};
use openui_runtime::{
    LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree, evaluate_tree_with_limits,
};
use std::sync::{Arc, Mutex};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn schema() -> JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}},"Len":{"params":{"properties":{"value":{}}}}}}"#).unwrap()
}
#[test]
fn defaults_run_before_the_tree_even_when_unused_or_supplied() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let seen = Mutex::new(Vec::new());
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        seen.lock().unwrap().push(v.clone());
        Ok(Some(v))
    });
    for supplied in [false, true] {
        seen.lock().unwrap().clear();
        let p = parse("q=Query(\"t\",{},@Id(5))\nroot=Box(1)", &l, None);
        let mut values = openui_runtime::JsObject::new();
        values.define("q", Value::Num(8.));
        let mut input = TreeInput::new(&p).with_library(&l).with_functions(&f);
        if supplied {
            input = input.with_reference_values(&values);
        }
        let r = evaluate_tree(input, &Store::new());
        assert!(r.errors.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
#[test]
fn a_million_item_argument_is_shared_through_a_thousand_calls() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let big = Arc::new((0..1_000_000).map(|n| Value::Num(n as f64)).collect());
    let calls = Mutex::new(0);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Len", |v| {
        let Value::Obj(o) = &v else { panic!("args") };
        let Some(Value::Arr(a)) = o.get("value") else {
            panic!("list")
        };
        assert!(Arc::ptr_eq(a, &big));
        *calls.lock().unwrap() += 1;
        Ok(Some(Value::Num(a.len() as f64)))
    });
    let mut store = Store::new();
    store.set("$big", Some(Value::Arr(big.clone()))).unwrap();
    store
        .set(
            "$rows",
            Some(Value::Arr(Arc::new(vec![Value::Num(1.); 1000]))),
        )
        .unwrap();
    let p = parse("root=Box(@Each($rows,r,@Len($big)))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&f),
        &store,
    );
    assert!(!r.size_limited && !r.depth_limited);
    assert_eq!(*calls.lock().unwrap(), 1000);
}
#[test]
fn undefined_argument_keys_are_a_pinned_representation_divergence() {
    let cases = parse_json(include_str!(
        "../../../conformance/divergences/function-argument-undefined.json"
    ))
    .unwrap();
    for c in cases.as_array().unwrap() {
        let mut j = schema();
        j["functions"] = parse_json("{}").unwrap();
        j["functions"]["K2"] = c["definition"].clone();
        let l = Library::from_json_schema(&j);
        let received = Mutex::new(None);
        let mut f = LibraryFunctions::new(&j["functions"]);
        f.register("K2", |v| {
            *received.lock().unwrap() = Some(v);
            Ok(None)
        });
        let p = parse(c["source"].as_str().unwrap(), &l, None);
        let mut store = Store::new();
        store
            .set("$o", Some(Value::from_json(&parse_json("{}").unwrap())))
            .unwrap();
        evaluate_tree(
            TreeInput::new(&p).with_library(&l).with_functions(&f),
            &store,
        );
        let got = received.lock().unwrap();
        let Some(Value::Obj(o)) = &*got else {
            panic!("args")
        };
        let mut rows = Vec::new();
        for (k, v) in o.iter() {
            let mut item = openui_runtime::JsObject::new();
            item.define("key", Value::Str(k.clone()));
            item.define("value", v.clone());
            rows.push(Value::Obj(Arc::new(item)));
        }
        assert_eq!(Value::Arr(Arc::new(rows)), Value::from_json(&c["rust"]));
        assert_ne!(c["reference"], c["rust"]);
    }
}
#[test]
fn eager_defaults_work_thresholds() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    for (source, expected) in [
        ("root=Box(1)", 142),
        ("q=Query(\"t\",{},@Sum([1,2,3]))\nroot=Box(1)", 151),
        ("q=Query(\"t\",{},@Sum([1,2,3]))\nroot=Box(q)", 129),
    ] {
        let p = parse(source, &l, None);
        let run = |work| {
            evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l),
                &Store::new(),
                Limits::DEFAULT.with_work(work),
            )
        };
        let (mut lo, mut hi) = (0, 10000);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if run(mid).size_limited {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        assert_eq!(lo, expected, "{source}");
        assert!(!run(lo).size_limited);
        assert!(run(lo - 1).size_limited);
    }
}

#[test]
fn owned_string_arguments_still_copy_and_spend_work() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let calls = Mutex::new(0);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Len", |v| {
        let Value::Obj(o) = &v else { panic!("args") };
        let Some(Value::Str(s)) = o.get("value") else {
            panic!("string")
        };
        *calls.lock().unwrap() += 1;
        Ok(Some(Value::Num(s.len() as f64)))
    });
    let mut store = Store::new();
    store
        .set("$big", Some(Value::Str("x".repeat(500_000))))
        .unwrap();
    store
        .set(
            "$rows",
            Some(Value::Arr(Arc::new(vec![Value::Num(1.); 1000]))),
        )
        .unwrap();
    let p = parse("root=Box(@Each($rows,r,@Len($big)))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&f),
        &store,
    );
    assert!(r.size_limited);
    let n = *calls.lock().unwrap();
    println!("500000-byte string: {n} calls before cut");
    assert_eq!(n, 41);
}

#[test]
fn registration_reference_reads_repeat_in_declaration_order() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let calls = Mutex::new(Vec::new());
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        let Value::Obj(o) = &v else { panic!("args") };
        let n = o.get("value").cloned();
        calls.lock().unwrap().push(n.clone());
        Ok(n)
    });
    let p = parse(
        "q=Query(\"t\",{},@Id(r))\nr=Query(\"t\",{},@Id(5))\nroot=Box([q,r,@Id(9)])",
        &l,
        None,
    );
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&f),
        &Store::new(),
    );
    assert!(r.errors.is_empty());
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Some(Value::Num(5.)),
            Some(Value::Num(5.)),
            Some(Value::Num(5.)),
            Some(Value::Num(9.))
        ]
    );
}

#[test]
fn incomplete_duplicates_do_not_replace_registered_defaults_and_undefined_stays_undefined() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut p = parse("q=Query(\"t\",{},5)\nroot=Box(q)", &l, None);
    let mut duplicate = p.query_statements[0].clone();
    duplicate.complete = false;
    p.query_statements.push(duplicate);
    let r = evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new());
    assert!(r.to_json().contains("\"value\":5"));
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |_| Ok(None));
    let p = parse("q=Query(\"t\",{},@Id(1))\nroot=Box(q)", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&f),
        &Store::new(),
    );
    assert!(r.to_json().contains("\"props\":{}"), "{}", r.to_json());
}

#[test]
fn a_primed_argument_body_does_not_bypass_deeper_call_limits() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Len", |_| Ok(Some(Value::Num(1.))));
    let mut body = Value::Null;
    for _ in 0..40 {
        body = Value::Arr(Arc::new(vec![body]));
    }
    let mut template = "@Len($big)".to_owned();
    for level in 0..25 {
        let p = parse(
            &format!("root=Box([$prime?@Len($big):null,{template}])"),
            &l,
            None,
        );
        let mut cuts = Vec::new();
        for prime in [false, true] {
            let mut store = Store::new();
            store.set("$big", Some(body.clone())).unwrap();
            store.set("$prime", Some(Value::Bool(prime))).unwrap();
            let r = evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l).with_functions(&f),
                &store,
                Limits::DEFAULT.with_depth(32),
            );
            cuts.push(r.depth_limited);
        }
        assert_eq!(
            cuts[0], cuts[1],
            "level {level}: admission cannot hide depth"
        );
        template = format!("@Each([1],r{level},{template})");
    }
}

#[test]
fn argument_admission_rechecks_a_narrower_reentered_session() {
    use openui_runtime::{Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator};
    use std::cell::Cell;
    struct Outer {
        body: Value,
        observed: Cell<bool>,
    }
    struct Forward<'e, 's> {
        eval: &'e ExpressionEvaluator<'s>,
        observed: &'e Cell<bool>,
    }
    impl EvaluationScope for Forward<'_, '_> {
        fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let r = self.eval.evaluate_result(&Ast::StateRef("$big".into()));
            self.observed.set(r.depth_limited);
            Some(Value::Null)
        }
    }
    impl EvaluationScope for Outer {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(self.body.clone())
        }
        fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            eval.with_temporary_results(|| {
                let first = eval.evaluate_result(&Ast::StateRef("$big".into()));
                assert!(!first.depth_limited);
                let forward = Forward {
                    eval,
                    observed: &self.observed,
                };
                let inner =
                    EvaluationSession::with_limits(&forward, 0, Limits::DEFAULT.with_depth(12));
                inner.evaluate(&Ast::Ref("again".into()));
            });
            Some(Value::Null)
        }
    }
    let mut body = Value::Null;
    for _ in 0..40 {
        body = Value::Arr(Arc::new(vec![body]));
    }
    let outer = Outer {
        body,
        observed: Cell::new(false),
    };
    let session = EvaluationSession::with_limits(&outer, 0, Limits::DEFAULT.with_depth(64));
    session.evaluate(&Ast::Comp {
        name: "Host".into(),
        args: Arc::new(vec![]),
        function: true,
        mapped_props: None,
    });
    assert!(outer.observed.get());
}
