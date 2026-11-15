//! Newly supplied program and reference input is admitted across a suspension.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{
    JsObject, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree, resume_action,
    run_action,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn fixture(source: &str) -> (ParseResult, Library, Store, Value) {
    let l = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse(source, &l, None);
    let mut s = Store::new();
    assert!(
        s.initialize(
            p.state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            []
        )
        .refused
        .is_empty()
    );
    let t = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    let Some(Value::Element(e)) = &t.root else {
        panic!()
    };
    let a = e.props.get("a").unwrap().clone();
    (p, l, s, a)
}
fn rows(n: usize) -> Value {
    Value::Arr(Arc::new(
        (0..n)
            .map(|i| {
                let mut o = JsObject::new();
                o.define("id", Value::Num(i as f64));
                Value::Obj(Arc::new(o))
            })
            .collect(),
    ))
}
fn refs(n: usize) -> JsObject<Value> {
    let mut q = JsObject::new();
    q.define("rows", rows(n));
    let mut r = JsObject::new();
    r.define("q", Value::Obj(Arc::new(q)));
    r
}
#[test]
fn refreshed_query_rows_are_available_after_a_mutation() {
    for initial in [0, 2000] {
        for prepared in [false, true] {
            let (p, _, mut s, a) = fixture(
                "$rows=[]\nq=Query(\"t\",{},{rows:[]})\nm=Mutation(\"t\",{})\nroot=Box(Action([@Run(m),@Set($rows,q.rows)]))",
            );
            let before = refs(initial);
            let after = refs(if initial == 0 { 200 } else { 2100 });
            let r = run_action(
                TreeInput::new(&p).with_reference_values(&before),
                &mut s,
                Some(&a),
                "",
                None,
            );
            assert!(r.is_suspended());
            let prepared_values = PreparedReferenceValues::new(&after);
            let input = if prepared {
                TreeInput::new(&p).with_prepared_reference_values(&prepared_values)
            } else {
                TreeInput::new(&p).with_reference_values(&after)
            };
            let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
            assert!(
                r.failure.is_none(),
                "initial={initial}, prepared={prepared}: {:?}",
                r.failure
            );
            assert!(
                matches!(s.get("$rows"),Some(Value::Arr(ref a))if a.len()==if initial==0{200}else{2100})
            );
        }
    }
}
#[test]
fn a_larger_program_can_supply_reset_defaults_after_suspension() {
    let (p, l, mut s, a) =
        fixture("$a=[]\nm=Mutation(\"t\",{})\nroot=Box(Action([@Run(m),@Reset($a)]))");
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    let source = format!(
        "$a=[{}]\nm=Mutation(\"t\",{{}})\nroot=Box(null)",
        (0..4000)
            .map(|i| format!("{{id:{i},n:{i}}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let next = parse(&source, &l, None);
    assert!(!next.size_limited);
    let r = resume_action(
        TreeInput::new(&next),
        &mut s,
        &r.continuation.unwrap(),
        true,
    )
    .unwrap();
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert!(matches!(s.get("$a"),Some(Value::Arr(ref a))if a.len()==4000));
    assert!(s.is_pristine("$a"));
}
#[test]
fn supplied_kept_literals_earn_credit_before_their_read_work() {
    let p = ParseResult::default();
    let mut s = Store::new();
    let mut step = JsObject::new();
    step.define("type", Value::Str("set".into()));
    step.define("target", Value::Str("$n".into()));
    step.define(
        "valueAST",
        Value::Ast(Arc::new(openui_lang::Ast::Str("x".repeat(1_100_000)))),
    );
    let mut plan = JsObject::new();
    plan.define(
        "steps",
        Value::Arr(Arc::new(vec![Value::Obj(Arc::new(step))])),
    );
    let plan = Value::Obj(Arc::new(plan));
    let r = run_action(TreeInput::new(&p), &mut s, Some(&plan), "", None);
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert!(matches!(s.get("$n"),Some(Value::Str(ref v))if v.len()==1_100_000));
}
#[test]
fn a_single_byte_set_does_not_reserve_unused_event_text() {
    use openui_runtime::{Limits, run_action_with_limits};
    let p = ParseResult::default();
    let mut s = Store::new();
    let a = Value::from_json(
        &parse_json(r#"{"steps":[{"type":"set","target":"$n","valueAST":{"k":"Num","v":1}}]}"#)
            .unwrap(),
    );
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_result_size(1),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert_eq!(s.get("$n"), Some(Value::Num(1.)));
}

#[test]
fn replacing_a_shared_program_uses_its_new_mutation_arguments() {
    let (p, l, mut s, a) =
        fixture("m=Mutation(\"t\",{version:1})\nroot=Box(Action([@Run(m),@Run(m)]))");
    let mut shared = Arc::new(p);
    let r = run_action(
        TreeInput::from_shared_result(&shared),
        &mut s,
        Some(&a),
        "",
        None,
    );
    *Arc::make_mut(&mut shared) = parse("m=Mutation(\"t\",{version:2})\nroot=Box(null)", &l, None);
    let r = resume_action(
        TreeInput::from_shared_result(&shared),
        &mut s,
        &r.continuation.unwrap(),
        true,
    )
    .unwrap();
    assert!(r.failure.is_none());
    assert!(r.events[0].to_json().contains(r#""args":{"version":2}"#));
}

#[test]
fn replacing_the_public_result_field_cannot_reuse_an_unrelated_program_index() {
    let (p, l, mut s, a) =
        fixture("m=Mutation(\"t\",{version:1})\nroot=Box(Action([@Run(m),@Run(m)]))");
    let shared = Arc::new(p);
    let r = run_action(
        TreeInput::from_shared_result(&shared),
        &mut s,
        Some(&a),
        "",
        None,
    );
    let next = parse(
        "other=Mutation(\"t\",{wrong:true})\nm=Mutation(\"t\",{version:3})\nroot=Box(null)",
        &l,
        None,
    );
    let mut input = TreeInput::from_shared_result(&shared);
    input.result = &next;
    let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
    assert!(r.failure.is_none());
    assert!(r.events[0].to_json().contains(r#""args":{"version":3}"#));
}
