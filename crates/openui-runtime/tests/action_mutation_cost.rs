//! A retained program can run distinct mutations without rescanning its statements.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Store, TreeInput, Value, evaluate_tree, resume_action, run_action};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn cost(n: usize, repeats: usize) -> usize {
    let l = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let source = format!(
        "{}\nroot=Box(Action([{}]))",
        (0..n)
            .map(|i| format!("m{i}=Mutation(\"t\")"))
            .collect::<Vec<_>>()
            .join("\n"),
        (0..n)
            .map(|i| format!("@Run(m{i})"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let p = Arc::new(parse(&source, &l, None));
    assert!(!p.size_limited);
    let t = evaluate_tree(
        TreeInput::from_shared_result(&p).with_library(&l),
        &Store::new(),
    );
    assert!(!t.size_limited);
    let Some(Value::Element(e)) = &t.root else {
        panic!()
    };
    let a = e.props.get("a").unwrap();
    let mut allocations = usize::MAX;
    for _ in 0..repeats {
        let mut s = Store::new();
        let before = test_alloc::allocation_calls();
        let mut r = run_action(TreeInput::from_shared_result(&p), &mut s, Some(a), "", None);
        let mut resumes = 0;
        while let Some(c) = r.continuation.take() {
            resumes += 1;
            r = resume_action(TreeInput::from_shared_result(&p), &mut s, &c, true).unwrap();
        }
        let calls = test_alloc::allocation_calls() - before;
        assert!(
            r.failure.is_none(),
            "{resumes}/{n} mutations: {:?}",
            r.failure
        );
        assert_eq!(resumes, n);
        allocations = allocations.min(calls);
    }
    println!("{n} distinct mutations: {allocations} allocations");
    allocations
}
#[test]
fn every_distinct_mutation_completes_without_a_spurious_cut() {
    cost(5000, 1);
}
#[test]
fn many_mutations_scale_with_the_program_and_steps() {
    let small = cost(4000, 3);
    let large = cost(16000, 3);
    assert!(
        large < small * 5,
        "nonlinear allocations: {} / {}",
        large,
        small
    );
    assert!(
        large < 16000 * 130 + 8192,
        "excess allocation work: {}",
        large
    );
}
