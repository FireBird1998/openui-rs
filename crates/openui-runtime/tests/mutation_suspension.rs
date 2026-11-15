//! Mutation handoffs suspend later effects until a host outcome arrives.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Store, TreeInput, Value, evaluate_tree, run_action};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn a_mutation_handoff_keeps_the_draft_until_success() {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$draft=\"initial\"\ncreate=Mutation(\"create\",{text:$draft})\nroot=Box(Action([@Run(create),@Reset($draft),@OpenUrl(\"after\")]))",
        &l,
        None,
    );
    let mut s = Store::new();
    s.set("$draft", Some(Value::Str("edited".into()))).unwrap();
    let Value::Element(ref root) = evaluate_tree(TreeInput::new(&p).with_library(&l), &s)
        .root
        .unwrap()
    else {
        panic!()
    };
    let r = run_action(
        TreeInput::new(&p).with_library(&l),
        &mut s,
        root.props.get("a"),
        "",
        None,
    );
    assert!(r.failure.is_none());
    assert_eq!(
        s.get("$draft"),
        Some(Value::Str("edited".into())),
        "reset ran before host outcome"
    );
    assert_eq!(r.events.len(), 1);
}

fn fixture(source: &str) -> (openui_lang::ParseResult, Library, Store, Value) {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(source, &l, None);
    let mut s = Store::new();
    let _ = s.initialize(
        p.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        [],
    );
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    assert!(!tree.size_limited);
    let action = if let Some(Value::Element(root)) = &tree.root {
        root.props.get("a").unwrap().clone()
    } else {
        panic!()
    };
    (p, l, s, action)
}
#[test]
fn outcomes_see_intervening_fields_and_tokens_are_single_use_and_session_owned() {
    use openui_runtime::{ActionResumeError, resume_action, set_field_value};
    let (p, l, mut s, a) = fixture(
        "$draft=\"initial\"\n$seen=null\nm=Mutation(\"m\",{text:$draft})\nroot=Box(Action([@Run(m),@Set($seen,$draft),@Reset($draft),@OpenUrl(\"after\")]))",
    );
    let r = run_action(
        TreeInput::new(&p).with_library(&l),
        &mut s,
        Some(&a),
        "",
        None,
    );
    assert!(r.is_suspended());
    assert!(
        r.events[0]
            .to_json()
            .unwrap()
            .contains(r#""args":{"text":"initial"}"#)
    );
    let c = r.continuation.unwrap();
    let clone = c.clone();
    let mut other = Store::new();
    assert_eq!(
        resume_action(TreeInput::new(&p), &mut other, &c, true).unwrap_err(),
        ActionResumeError::ForeignSession
    );
    set_field_value(
        &mut s,
        None,
        Some("Input"),
        "$draft",
        Some(Value::Str("while waiting".into())),
        false,
    )
    .unwrap();
    let r = resume_action(TreeInput::new(&p).with_library(&l), &mut s, &c, true).unwrap();
    assert!(!r.is_suspended());
    assert!(r.failure.is_none());
    assert_eq!(r.events.len(), 1);
    assert_eq!(s.get("$seen"), Some(Value::Str("while waiting".into())));
    assert_eq!(s.get("$draft"), Some(Value::Str("initial".into())));
    assert_eq!(
        resume_action(TreeInput::new(&p), &mut s, &clone, true).unwrap_err(),
        ActionResumeError::Consumed
    );
    let c = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None)
        .continuation
        .unwrap();
    s.set("$draft", Some(Value::Str("keep".into()))).unwrap();
    let r = resume_action(TreeInput::new(&p), &mut s, &c, false).unwrap();
    assert!(r.events.is_empty());
    assert_eq!(s.get("$draft"), Some(Value::Str("keep".into())));
    let c = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None)
        .continuation
        .unwrap();
    s.dispose();
    assert_eq!(
        resume_action(TreeInput::new(&p), &mut s, &c, true).unwrap_err(),
        ActionResumeError::Disposed
    );
}
#[test]
fn two_mutations_and_a_last_mutation_each_require_an_outcome() {
    use openui_runtime::resume_action;
    let (p, l, mut s, a) = fixture(
        "$n=0\nm=Mutation(\"m\",{n:$n})\nroot=Box(Action([@Run(m),@Set($n,$n+1),@Run(m)]))",
    );
    let r = run_action(
        TreeInput::new(&p).with_library(&l),
        &mut s,
        Some(&a),
        "",
        None,
    );
    let c = r.continuation.unwrap();
    assert_eq!(s.get("$n"), Some(Value::Num(0.)));
    let r = resume_action(TreeInput::new(&p), &mut s, &c, true).unwrap();
    assert!(r.is_suspended());
    assert_eq!(r.events.len(), 1);
    assert!(r.events[0].to_json().unwrap().contains(r#""args":{"n":1}"#));
    let r = resume_action(TreeInput::new(&p), &mut s, &r.continuation.unwrap(), false).unwrap();
    assert!(!r.is_suspended());
    assert!(r.events.is_empty());
    assert_eq!(s.get("$n"), Some(Value::Num(1.)));
}
#[test]
fn resumed_steps_use_the_current_program_and_keep_the_original_work_allowance() {
    use openui_runtime::{Limits, resume_action, run_action_with_limits};
    let (p, l, mut s, a) =
        fixture("$n=0\nm=Mutation(\"m\",{})\nroot=Box(Action([@Run(m),@Reset($n),@Run(m)]))");
    let c = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None)
        .continuation
        .unwrap();
    let next = parse(
        "$n=42\nm=Mutation(\"m\",{version:2})\nroot=Box(null)",
        &l,
        None,
    );
    let r = resume_action(TreeInput::new(&next), &mut s, &c, true).unwrap();
    assert_eq!(s.get("$n"), Some(Value::Num(42.)));
    assert!(
        r.events[0]
            .to_json()
            .unwrap()
            .contains(r#""args":{"version":2}"#)
    );
    let source = format!(
        "$n=0\nm=Mutation(\"m\",@Count(@Each($rows,r,r+1)))\nroot=Box(Action([{}]))",
        vec!["@Run(m)"; 100].join(",")
    );
    let (p, _, mut s, a) = fixture(&source);
    s.set(
        "$rows",
        Some(Value::Arr(std::sync::Arc::new(vec![Value::Num(1.); 1000]))),
    )
    .unwrap();
    let mut r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_work(100_000),
    );
    let mut resumes = 0;
    while let Some(c) = r.continuation.take() {
        resumes += 1;
        r = resume_action(TreeInput::new(&p), &mut s, &c, true).unwrap();
    }
    assert!(
        resumes > 0 && resumes < 100,
        "fresh budget on each resume: {resumes}"
    );
    assert!(r.size_limited);
    assert!(r.failure.is_some());
}
#[test]
fn query_invalidations_do_not_suspend() {
    let (p, _, mut s, a) =
        fixture("q=Query(\"q\",{})\nroot=Box(Action([@Run(q),@OpenUrl(\"after\")]))");
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert!(!r.is_suspended());
    assert_eq!(r.events.len(), 2);
}

#[test]
fn repeated_reference_input_does_not_replenish_spent_work() {
    use openui_runtime::{JsObject, PreparedReferenceValues, resume_action};
    for prepared in [false, true] {
        let source = format!(
            "m=Mutation(\"m\",@Count(@Each($rows,r,@Count(@Each($rows,x,r+x)))))\nroot=Box(Action([{}]))",
            vec!["@Run(m)"; 30].join(",")
        );
        let (p, _, mut s, a) = fixture(&source);
        let p = std::sync::Arc::new(p);
        s.set(
            "$rows",
            Some(Value::Arr(std::sync::Arc::new(vec![Value::Num(1.); 250]))),
        )
        .unwrap();
        let mut values = JsObject::new();
        values.define("unused", Value::Str("x".repeat(10_000)));
        let measured = PreparedReferenceValues::new(&values);
        let input = if prepared {
            TreeInput::from_shared_result(&p).with_prepared_reference_values(&measured)
        } else {
            TreeInput::from_shared_result(&p).with_reference_values(&values)
        };
        let mut r = run_action(input.clone(), &mut s, Some(&a), "", None);
        let mut resumes = 0;
        while let Some(c) = r.continuation.take() {
            resumes += 1;
            r = resume_action(input.clone(), &mut s, &c, true).unwrap();
        }
        assert!(
            resumes > 0 && resumes < 30,
            "same references minted work: {resumes}, prepared={prepared}"
        );
        assert!(r.size_limited);
    }
}
