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
        "$n=1\nroot=Box([$n,@Echo()],@Go())".into(),
        library.clone(),
        None,
    )
    .program;
    let before = session.replace_program(old, None, None, true).unwrap();
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
        assert!(session.is_pristine("$n".into()).unwrap());
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
