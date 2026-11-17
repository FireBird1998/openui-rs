//! One-thread wasm adapter uses the same atomic replacement.
mod support;
use support::*;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
#[test]
fn refused_wasm_replacement_keeps_old_handles_store_and_program() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    let old = parse(
        "$n=1\nroot=Box(@Set($n,$n+1))".into(),
        library.clone(),
        None,
    )
    .program;
    let before = session.replace_program(old, None, None, true).unwrap();
    let action = session
        .action_at(
            before.evaluation.evaluation,
            vec!["props".into(), "a".into()],
        )
        .unwrap();
    let bad = parse(
        r#"$n=99
$new=1
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()"#
            .into(),
        library,
        None,
    )
    .program;
    assert!(
        session
            .replace_program(bad.clone(), Some("[]".into()), None, true)
            .is_err()
    );
    assert!(session.replace_program(bad, None, None, true).is_err());
    assert_eq!(session.snapshot().unwrap(), before.snapshot);
    assert_eq!(session.version().unwrap(), before.version);
    assert!(
        session
            .run(Some(action), "go".into(), None)
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(session.get("$n".into()).unwrap().as_deref(), Some("2"));
    session.set("$n".into(), Some("3".into())).unwrap();
    assert!(
        session
            .evaluate()
            .unwrap()
            .json
            .contains("\"typeName\":\"Box\"")
    );
    assert!(!session.snapshot().unwrap().contains("$new"));
}
