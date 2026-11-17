//! Public native replacement seam, including the ordinary-stack callback thread.
use openui_ffi::*;
use std::sync::{Arc, Mutex};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
struct Callback(Arc<Mutex<Vec<std::thread::ThreadId>>>);
impl LibraryFunction for Callback {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        self.0.lock().unwrap().push(std::thread::current().id());
        Ok(Some("true".into()))
    }
}
#[test]
fn refused_native_replacement_preserves_callback_thread_and_old_program_on_app_writes() {
    let library = ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_json_bytes: 2048,
            ..Default::default()
        },
    )
    .unwrap();
    let threads = Arc::new(Mutex::new(Vec::new()));
    session
        .register_function("Echo".into(), Box::new(Callback(threads.clone())))
        .unwrap();
    session.mark_binding("Box".into(), "unused".into()).unwrap();
    let old = parse(
        "$n=1\n$keep=2\n$edited=0\nroot=Box([$n,@Echo()],@Go())".into(),
        library.clone(),
        None,
    )
    .program;
    let before = session
        .replace_program(old, Some("{\"$edited\":5}".into()), None, true)
        .unwrap();
    let action = session
        .action_at(
            before.evaluation.evaluation,
            vec!["props".into(), "action".into()],
        )
        .unwrap();
    let small = parse("$n=99\n$new=1\nroot=Box(0)".into(), library.clone(), None).program;
    let huge = parse(
        format!("root=Box(\"{}\")", "x".repeat(3000)),
        library.clone(),
        None,
    )
    .program;
    let malformed = parse(
        r#"$n=99
$new=1
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=@Echo() ? $shape : Box()"#
            .into(),
        library.clone(),
        None,
    )
    .program;
    for (candidate, persisted) in [(huge, None), (small, Some("[]".into())), (malformed, None)] {
        assert!(
            session
                .replace_program(candidate, persisted, Some("{\"$n\":88}".into()), true)
                .is_err()
        );
        assert_eq!(session.snapshot().unwrap(), before.snapshot);
        assert_eq!(session.version().unwrap(), before.version);
        for name in ["$n", "$keep", "$edited", "$new", "$extra"] {
            assert_eq!(
                session.is_pristine(name.into()).unwrap(),
                matches!(name, "$n" | "$keep")
            );
        }
        assert_eq!(session.pending_continuations().unwrap(), 0);
        assert!(
            session
                .run(Some(action.clone()), "go".into(), None)
                .unwrap()
                .events[0]
                .contains("Go")
        );
    }
    session.set("$n".into(), Some("2".into())).unwrap();
    session
        .set_field(Some("f".into()), None, "x".into(), Some("3".into()), true)
        .unwrap();
    let after = session.evaluate().unwrap();
    assert!(after.json.contains("\"value\":[2,true]"));
    assert!(!session.snapshot().unwrap().contains("$new"));
    let threads = threads.lock().unwrap();
    assert!(threads.len() >= 3);
    assert!(
        threads
            .iter()
            .all(|id| *id == threads[0] && *id != std::thread::current().id())
    );
    session.dispose();
}

#[test]
fn post_evaluation_refusal_preserves_pending_mutation_and_all_store_marks() {
    let library = ComponentLibrary::new(
        r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}}}"#.into(),
    )
    .unwrap();

    let mut control = RuntimeSession::new(library.clone(), None).unwrap();
    let mut subject = RuntimeSession::new(library.clone(), None).unwrap();
    let program = |source: &str| parse(source.into(), library.clone(), None).program;
    let old = program(
        "$n=1\n$pristine=2\nm=Mutation(\"m\",{})\nroot=Box($n,Action([@Run(m),@Set($n,$n+1),@OpenUrl(\"done\")]))",
    );
    let mut tokens = Vec::new();
    for session in [&mut control, &mut subject] {
        let shown = session
            .replace_program(old.clone(), None, None, true)
            .unwrap();
        session.set("$n".into(), Some("5".into())).unwrap();
        session
            .set_field(Some("f".into()), None, "x".into(), Some("7".into()), false)
            .unwrap();
        let action = session
            .action_at(
                shown.evaluation.evaluation,
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        let run = session.run(Some(action), "go".into(), None).unwrap();
        assert!(!run.size_limited && !run.depth_limited && run.failure.is_none());
        assert_eq!(run.events.len(), 1);
        tokens.push(run.continuation.unwrap());
        assert_eq!(session.pending_continuations().unwrap(), 1);
    }
    let candidate = program(
        r#"$n=99
$pristine=88
$new=3
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()"#,
    );
    assert!(
        subject
            .replace_program(
                candidate,
                Some("{\"$n\":99,\"$extra\":8}".into()),
                Some("{\"$pristine\":77}".into()),
                true
            )
            .is_err()
    );
    assert_eq!(subject.snapshot().unwrap(), control.snapshot().unwrap());
    assert_eq!(subject.version().unwrap(), control.version().unwrap());
    for name in ["$n", "$pristine", "$new", "$extra", "$shape", "f"] {
        assert_eq!(
            subject.is_pristine(name.into()).unwrap(),
            control.is_pristine(name.into()).unwrap(),
            "{name}"
        );
    }
    assert!(!subject.is_pristine("$n".into()).unwrap());
    assert!(subject.is_pristine("$pristine".into()).unwrap());
    assert_eq!(subject.pending_continuations().unwrap(), 1);
    assert_eq!(control.pending_continuations().unwrap(), 1);
    assert_eq!(
        subject.get_field(Some("f".into()), "x".into()).unwrap(),
        Some("7".into())
    );
    assert_eq!(
        subject.get_field(Some("f".into()), "x".into()).unwrap(),
        control.get_field(Some("f".into()), "x".into()).unwrap()
    );
    let a = subject.evaluate().unwrap();
    let b = control.evaluate().unwrap();
    assert_eq!(a.json, b.json);
    assert_eq!(
        (a.size_limited, a.depth_limited),
        (b.size_limited, b.depth_limited)
    );
    let mut resumed = Vec::new();
    for (session, token) in [&mut control, &mut subject].into_iter().zip(tokens) {
        let result = session.resume(token, true).unwrap();
        assert_eq!(session.pending_continuations().unwrap(), 0);
        resumed.push((
            result.events,
            result.size_limited,
            result.depth_limited,
            format!("{:?}", result.failure),
            result.continuation.is_some(),
        ));
    }
    assert_eq!(resumed[0], resumed[1]);
    assert_eq!(subject.snapshot().unwrap(), control.snapshot().unwrap());
    assert_eq!(subject.version().unwrap(), control.version().unwrap());
    for session in [&mut control, &mut subject] {
        session.set("$n".into(), Some("9".into())).unwrap();
    }
    assert_eq!(subject.snapshot().unwrap(), control.snapshot().unwrap());
    assert_eq!(subject.version().unwrap(), control.version().unwrap());
    assert_eq!(
        subject.evaluate().unwrap().json,
        control.evaluate().unwrap().json
    );
}
