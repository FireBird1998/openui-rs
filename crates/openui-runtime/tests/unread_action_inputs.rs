//! Unread supplied references affect allowance, never action admission.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    JsObject, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree, resume_action,
    run_action,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn values(kind: usize) -> JsObject<Value> {
    let mut o = JsObject::new();
    let value = if kind == 2 {
        let mut v = Value::Null;
        for _ in 0..40000 {
            v = Value::Arr(Arc::new(vec![v]));
        }
        v
    } else {
        Value::Str("y".repeat(if kind == 0 { 9 } else { 5 } * 1024 * 1024))
    };
    o.define("q", value);
    if kind == 1 {
        o.define("other", Value::Str("z".repeat(5 * 1024 * 1024)));
    }
    o
}
fn library() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}
#[test]
fn unread_large_and_deep_references_allow_events_sets_and_resumes() {
    let l = library();
    let p = Arc::new(parse(
        "$s=0\nq=Query(\"t\",{})\nm=Mutation(\"t\")\nroot=Box(Action([@ToAssistant(\"before\"),@Run(m),@Set($s,1),@Run(m),@Set($s,2)]))",
        &l,
        None,
    ));
    for kind in 0..3 {
        let v = values(kind);
        let prepared = PreparedReferenceValues::new(&v);
        let mut expected = None;
        for shared in [false, true] {
            for cached in [false, true] {
                let input = if shared {
                    TreeInput::from_shared_result(&p)
                } else {
                    TreeInput::new(&p)
                }
                .with_library(&l);
                let input = if cached {
                    input.with_prepared_reference_values(&prepared)
                } else {
                    input.with_reference_values(&v)
                };
                let tree = evaluate_tree(input.clone(), &Store::new());
                assert!(!tree.size_limited && !tree.depth_limited);
                let Some(Value::Element(ref e)) = tree.root else {
                    panic!("root")
                };
                let a = e.props.get("a").unwrap();
                let mut s = Store::new();
                s.set("$s", Some(Value::Num(0.))).unwrap();
                let mut r = run_action(input.clone(), &mut s, Some(a), "", None);
                let mut events = Vec::new();
                let mut resumes = 0;
                loop {
                    assert!(
                        r.failure.is_none(),
                        "kind={kind}, shared={shared}, cached={cached}: {:?}",
                        r.failure
                    );
                    events.extend(r.events.iter().map(|e| e.to_json()));
                    let Some(c) = r.continuation.take() else {
                        break;
                    };
                    resumes += 1;
                    r = resume_action(input.clone(), &mut s, &c, true).unwrap();
                }
                assert_eq!(resumes, 2);
                assert_eq!(s.get("$s"), Some(Value::Num(2.)));
                assert_eq!(events.len(), 3);
                if let Some(expected) = &expected {
                    assert_eq!(expected, &events)
                } else {
                    expected = Some(events)
                }
            }
        }
    }
}
#[test]
fn a_new_unread_supply_does_not_consume_the_remaining_action() {
    let l = library();
    let p = parse(
        "m=Mutation(\"t\")\nroot=Box(Action([@Run(m),@Set($s,1),@ToAssistant(\"after\")]))",
        &l,
        None,
    );
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new());
    let Some(Value::Element(ref e)) = tree.root else {
        panic!()
    };
    let a = e.props.get("a").unwrap();
    for kind in 0..3 {
        let mut s = Store::new();
        let r = run_action(TreeInput::new(&p), &mut s, Some(a), "", None);
        let v = values(kind);
        let r = resume_action(
            TreeInput::new(&p).with_reference_values(&v),
            &mut s,
            &r.continuation.unwrap(),
            true,
        )
        .unwrap();
        assert!(r.failure.is_none(), "{kind}: {:?}", r.failure);
        assert_eq!(r.events.len(), 1);
        assert_eq!(s.get("$s"), Some(Value::Num(1.)));
    }
}
#[test]
fn reading_an_unmeasurable_reference_cuts_only_that_step() {
    let l = library();
    let p = Arc::new(parse(
        "q=Query(\"t\",{})\nm=Mutation(\"t\")\nroot=Box(Action([@Set($s,1),@Run(m),@Set($s,q),@Set($s,2)]))",
        &l,
        None,
    ));
    for kind in [0, 2] {
        let v = values(kind);
        let prepared = PreparedReferenceValues::new(&v);
        for shared in [false, true] {
            for cached in [false, true] {
                let input = if shared {
                    TreeInput::from_shared_result(&p)
                } else {
                    TreeInput::new(&p)
                }
                .with_library(&l);
                let input = if cached {
                    input.with_prepared_reference_values(&prepared)
                } else {
                    input.with_reference_values(&v)
                };
                let tree = evaluate_tree(input.clone(), &Store::new());
                let Some(Value::Element(ref e)) = tree.root else {
                    panic!()
                };
                let mut s = Store::new();
                let r = run_action(input.clone(), &mut s, e.props.get("a"), "", None);
                assert!(r.failure.is_none());
                assert_eq!(s.get("$s"), Some(Value::Num(1.)));
                assert_eq!(r.events.len(), 1);
                let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
                assert_eq!(r.failure.as_ref().unwrap().step, Some(2));
                assert!(if kind == 0 {
                    r.size_limited
                } else {
                    r.depth_limited
                });
                assert!(r.events.is_empty());
                assert_eq!(s.get("$s"), Some(Value::Num(1.)));
            }
        }
    }
}
#[test]
fn raw_and_prepared_references_keep_conservative_string_and_ast_read_sizes() {
    let l = library();
    let p = parse(
        "q=Query(\"t\",{})\nm=Mutation(\"t\")\nroot=Box(Action([@Run(m),@Set($s,q)]))",
        &l,
        None,
    );
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new());
    let Some(Value::Element(ref e)) = tree.root else {
        panic!()
    };
    for ast in [false, true] {
        let text = "\u{f0000}".repeat(300000);
        let value = if ast {
            Value::Ast(Arc::new(openui_lang::Ast::Obj(Arc::new(vec![
                ("s".into(), openui_lang::Ast::Str(text)),
                (
                    "rows".into(),
                    openui_lang::Ast::Arr(Arc::new(
                        (0..50000)
                            .map(|i| openui_lang::Ast::Num(i as f64))
                            .collect(),
                    )),
                ),
            ]))))
        } else {
            Value::Str(text)
        };
        let mut values = JsObject::new();
        values.define("q", value);
        let prepared = PreparedReferenceValues::new(&values);
        let mut states = Vec::new();
        let mut failures = Vec::new();
        for cached in [false, true] {
            let input = TreeInput::new(&p);
            let input = if cached {
                input.with_prepared_reference_values(&prepared)
            } else {
                input.with_reference_values(&values)
            };
            let mut s = Store::new();
            let r = run_action(input.clone(), &mut s, e.props.get("a"), "", None);
            assert!(r.failure.is_none());
            let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
            failures.push(format!("{:?}", r.failure));
            states.push(s.get("$s"));
        }
        assert!(states[0] == states[1]);
        assert_eq!(failures[0], failures[1]);
        let draw_program = parse("q=Query(\"t\",{})\nroot=Box(q)", &l, None);
        let draw = evaluate_tree(
            TreeInput::new(&draw_program)
                .with_library(&l)
                .with_prepared_reference_values(&prepared),
            &Store::new(),
        );
        assert!(!draw.size_limited && !draw.depth_limited);
        let Some(Value::Element(ref e)) = draw.root else {
            panic!("root")
        };
        assert_eq!(states[0].as_ref(), e.props.get("a"));
    }
}
