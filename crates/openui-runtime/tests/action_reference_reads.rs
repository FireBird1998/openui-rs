//! Actions and drawing prepare supplied references with one identical walk.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    JsObject, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree, resume_action,
    run_action,
};
use std::{sync::Arc, time::Instant};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap(),
    )
}
fn deep(n: usize) -> Value {
    let mut v = Value::Null;
    for _ in 0..n {
        v = Value::Arr(Arc::new(vec![v]));
    }
    v
}
fn prop(tree: &openui_runtime::TreeResult, key: &str) -> Value {
    let Some(Value::Element(e)) = &tree.root else {
        panic!("root")
    };
    e.props.get(key).unwrap().clone()
}
#[test]
fn supplied_reference_reads_are_admitted_exactly_as_drawing() {
    let l = library();
    let mut stricter = 0;
    let mut looser = 0;
    let mut cases = 0;
    // Includes both preparation ceilings and entries before/after a failed read.
    for (array, sizes) in [
        (
            false,
            vec![
                1024,
                5 * 1024 * 1024,
                8 * 1024 * 1024,
                17 * 1024 * 1024 / 2,
                9 * 1024 * 1024,
                20 * 1024 * 1024,
                70 * 1024 * 1024,
            ],
        ),
        (true, vec![1024, 4_500_000, 10_000_000]),
    ] {
        for size in sizes {
            for extra in 0..5 {
                let q = if array {
                    Value::Arr(Arc::new(vec![Value::Num(1.); size]))
                } else {
                    Value::Str("y".repeat(size))
                };
                let other = match extra {
                    1 | 2 => Some(Value::Str("z".repeat(5 * 1024 * 1024))),
                    3 | 4 => Some(deep(40000)),
                    _ => None,
                };
                let mut refs = JsObject::new();
                if extra == 2 || extra == 4 {
                    refs.define("other", other.clone().unwrap());
                }
                refs.define("q", q);
                if extra == 1 || extra == 3 {
                    refs.define("other", other.unwrap());
                }
                let prepared = PreparedReferenceValues::new(&refs);
                for expr in ["q.length", "@Count(q)", "q == null"] {
                    for resumed in [false, true] {
                        let prefix = if resumed { "@Run(m)," } else { "" };
                        let p = Arc::new(parse(
                            &format!(
                                "$s=0\nq=Query(\"t\",{{}},[])\nm=Mutation(\"t\")\nroot=Box({expr},Action([{prefix}@Set($s,{expr})]))"
                            ),
                            &l,
                            None,
                        ));
                        let a = prop(
                            &evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new()),
                            "b",
                        );
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
                                    input.with_reference_values(&refs)
                                };
                                let draw = evaluate_tree(input.clone(), &Store::new());
                                let draws = !draw.size_limited && !draw.depth_limited;
                                let mut s = Store::new();
                                s.set("$s", Some(Value::Num(0.))).unwrap();
                                let mut r = run_action(input.clone(), &mut s, Some(&a), "", None);
                                if resumed {
                                    assert!(r.failure.is_none());
                                    r = resume_action(
                                        input,
                                        &mut s,
                                        &r.continuation.take().unwrap(),
                                        true,
                                    )
                                    .unwrap();
                                }
                                let runs = r.failure.is_none();
                                cases += 1;
                                stricter += usize::from(draws && !runs);
                                looser += usize::from(!draws && runs);
                                assert_eq!(
                                    runs, draws,
                                    "read admission: array={array}, size={size}, extra={extra}, expr={expr}, resumed={resumed}, shared={shared}, cached={cached}: {:?}",
                                    r.failure
                                );
                                let observation =
                                    (runs, r.size_limited, r.depth_limited, s.get("$s"));
                                if let Some(expected) = &expected {
                                    assert_eq!(
                                        &observation, expected,
                                        "forms: array={array}, size={size}, extra={extra}, expr={expr}, resumed={resumed}"
                                    );
                                } else {
                                    expected = Some(observation);
                                }
                                if draws && runs {
                                    assert_eq!(s.get("$s"), Some(prop(&draw, "a")));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("read grid: {cases} configurations, {stricter} stricter, {looser} looser");
    assert_eq!((stricter, looser), (0, 0));
}
fn setup(n: usize, rows: usize, trigger: usize) -> (usize, f64) {
    let l = library();
    let queries = (0..n)
        .map(|i| format!("q{i}=Query(\"t\",{{}})"))
        .collect::<Vec<_>>()
        .join("\n");
    let steps = (0..n)
        .map(|i| format!("@Set($s,q{i}==null)"))
        .collect::<Vec<_>>()
        .join(",");
    let p = Arc::new(parse(
        &format!("$s=0\nm=Mutation(\"t\")\n{queries}\nroot=Box(Action([@Run(m),{steps}]))"),
        &l,
        None,
    ));
    let a = prop(
        &evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new()),
        "a",
    );
    let mut s = Store::new();
    let r = run_action(
        TreeInput::from_shared_result(&p),
        &mut s,
        Some(&a),
        "",
        None,
    );
    let c = r.continuation.unwrap();
    let shared = Arc::new(vec![Value::Num(1.); rows]);
    let mut refs = JsObject::new();
    match trigger {
        1 => {
            refs.define("unread", deep(1000));
        }
        2 => {
            refs.define("unread", Value::Str("x".repeat(9 * 1024 * 1024)));
        }
        _ => {}
    }
    for i in 0..n {
        refs.define(format!("q{i}"), Value::Arr(shared.clone()));
    }
    let before = test_alloc::allocation_calls();
    let start = Instant::now();
    let r = resume_action(
        TreeInput::from_shared_result(&p).with_reference_values(&refs),
        &mut s,
        &c,
        true,
    )
    .unwrap();
    let seconds = start.elapsed().as_secs_f64();
    let allocations = test_alloc::allocation_calls() - before;
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert_eq!(s.get("$s"), Some(Value::Bool(false)));
    (allocations, seconds)
}
#[test]
fn shared_reference_bodies_are_prepared_once_even_after_unmeasurable_entries() {
    for trigger in [1, 2] {
        for n in [100, 400, 1000] {
            let small = setup(n, 1, trigger);
            let large = setup(n, 1_000_000, trigger);
            let without = setup(n, 1_000_000, 0);
            println!(
                "trigger {trigger}, {n} aliases: small {small:?}, million {large:?}, no trigger {without:?}"
            );
            assert!(
                large.0 <= small.0 + 32,
                "{n} aliases re-walked one shared body: {} -> {}",
                small.0,
                large.0
            );
        }
    }
}

#[test]
fn an_overdeep_set_value_cuts_at_that_step_and_keeps_earlier_effects() {
    fn set(value: Value) -> Value {
        let mut o = JsObject::new();
        o.define("type", Value::Str("set".into()));
        o.define("target", Value::Str("$s".into()));
        o.define("valueAST", value);
        Value::Obj(Arc::new(o))
    }
    let mut expr = Arc::new(openui_lang::Ast::Num(1.));
    for _ in 0..1000 {
        expr = Arc::new(openui_lang::Ast::UnaryOp {
            op: "-",
            operand: expr,
        });
    }
    let mut plan = JsObject::new();
    plan.define(
        "steps",
        Value::Arr(Arc::new(vec![
            set(Value::Ast(Arc::new(openui_lang::Ast::Num(1.)))),
            set(Value::Ast(expr)),
            set(Value::Ast(Arc::new(openui_lang::Ast::Num(2.)))),
        ])),
    );
    let mut s = Store::new();
    let r = run_action(
        TreeInput::new(&openui_lang::ParseResult::default()),
        &mut s,
        Some(&Value::Obj(Arc::new(plan))),
        "",
        None,
    );
    assert_eq!(r.failure.as_ref().unwrap().step, Some(1));
    assert!(r.depth_limited);
    assert_eq!(s.get("$s"), Some(Value::Num(1.)));
}

#[test]
fn refused_reference_entries_earn_no_extra_write_room() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Large":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Large", |_| Ok(Some(Value::Str("x".repeat(100_000)))))
        .unwrap();
    let p = Arc::new(parse(
        "q=Query(\"t\",{})\nroot=Box(Action([@Set($s,@Large())]))",
        &l,
        None,
    ));
    let a = prop(
        &evaluate_tree(TreeInput::new(&p).with_library(&l), &Store::new()),
        "a",
    );
    let draw_program = parse("q=Query(\"t\",{})\nroot=Box(@Large())", &l, None);
    for value in [deep(40_000), Value::Str("x".repeat(70 * 1024 * 1024))] {
        let mut refs = JsObject::new();
        refs.define("q", value);
        let prepared = PreparedReferenceValues::new(&refs);
        let draw = evaluate_tree(
            TreeInput::new(&draw_program)
                .with_library(&l)
                .with_functions(&functions)
                .with_prepared_reference_values(&prepared),
            &Store::new(),
        );
        assert!(draw.size_limited);
        for cached in [false, true] {
            let input = TreeInput::from_shared_result(&p)
                .with_library(&l)
                .with_functions(&functions);
            let input = if cached {
                input.with_prepared_reference_values(&prepared)
            } else {
                input.with_reference_values(&refs)
            };
            let mut s = Store::new();
            let r = run_action(input, &mut s, Some(&a), "", None);
            assert!(r.size_limited);
            assert_eq!(r.failure.as_ref().unwrap().step, Some(0));
            assert_eq!(s.get("$s"), None);
        }
    }
}
