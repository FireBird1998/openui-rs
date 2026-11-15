//! Event-only clicks do not register query defaults.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{LibraryFunctions, Store, TreeInput, Value, evaluate_tree, run_action};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn event_only_runs_do_not_call_unrelated_query_defaults() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Tick":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "q=Query(\"q\",{},@Tick())\nroot=Box(@OpenUrl(\"u\"))",
        &l,
        None,
    );
    let mut s = Store::new();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Tick", move |_| {
        c.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Value::Num(7.)))
    });
    let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
    let Value::Element(ref root) = evaluate_tree(input.clone(), &s).root.unwrap() else {
        panic!()
    };
    let before = count.load(Ordering::Relaxed);
    for action in [root.props.get("a"), None] {
        let r = run_action(input.clone(), &mut s, action, "", None);
        assert!(r.failure.is_none());
    }
    assert_eq!(
        count.load(Ordering::Relaxed),
        before,
        "event-only click evaluated query defaults"
    );
}
#[test]
fn unrelated_expensive_defaults_do_not_cut_event_reset_or_legacy_runs() {
    use openui_runtime::{Limits, run_action_with_limits};
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$n=0\nq=Query(\"q\",{},@Count(@Each($rows,r,r+1)))\nroot=Box(@OpenUrl(\"u\"),@Reset($n))",
        &l,
        None,
    );
    let mut s = Store::new();
    s.set(
        "$rows",
        Some(Value::Arr(Arc::new(vec![Value::Num(1.); 10000]))),
    )
    .unwrap();
    s.set(
        "contact",
        Some(Value::from_json(&parse_json("{}").unwrap())),
    )
    .unwrap();
    let tree = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    let Some(Value::Element(root)) = &tree.root else {
        panic!()
    };
    let legacy = Value::from_json(&parse_json(r#"{"type":"legacy"}"#).unwrap());
    for a in [
        root.props.get("a"),
        root.props.get("b"),
        None,
        Some(&legacy),
    ] {
        let r = run_action_with_limits(
            TreeInput::new(&p),
            &mut s,
            a,
            "",
            Some("contact"),
            Limits::DEFAULT.with_work(5000),
        );
        assert!(r.failure.is_none(), "{:?}", r.failure);
    }
}

#[test]
fn defaults_initialize_once_at_the_first_expression_across_resumes() {
    use openui_runtime::resume_action;
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Tick":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$n=1\nq=Query(\"q\",{},@Tick())\nm=Mutation(\"m\",{q:q})\nroot=Box(Action([@OpenUrl(\"before\"),@Set($n,q+1),@Run(m),@Set($n,q+1)]))",
        &l,
        None,
    );
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Tick", move |_| {
        c.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Value::Num(7.)))
    });
    let mut s = Store::new();
    let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
    let tree = evaluate_tree(input.clone(), &s);
    let Some(Value::Element(root)) = &tree.root else {
        panic!()
    };
    let before = count.load(Ordering::Relaxed);
    let r = run_action(input.clone(), &mut s, root.props.get("a"), "", None);
    assert!(r.is_suspended());
    assert_eq!(count.load(Ordering::Relaxed), before + 1);
    assert_eq!(s.get("$n"), Some(Value::Num(8.)));
    s.set("$n", Some(Value::Num(99.))).unwrap();
    let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
    assert!(r.failure.is_none());
    assert!(!r.is_suspended());
    assert_eq!(s.get("$n"), Some(Value::Num(8.)));
    assert_eq!(count.load(Ordering::Relaxed), before + 1);
}
