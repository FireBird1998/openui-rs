//! A `@Run` of a query and Retry through the native binding's worker.
use openui_ffi::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn native_refresh_and_retry_hand_calls_to_the_driver() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session.attach_tool_provider().unwrap();
    let program = parse(
        "q = Query(\"read\", {}, \"loading\")\nroot = Box(Action([@Run(q)]))".into(),
        library,
        None,
    )
    .program;
    let draw = session
        .replace_program(program, None, None, false)
        .unwrap()
        .evaluation;
    let first = session.take_query_update().unwrap().calls.remove(0);
    session
        .complete_tool_call(first, r#"{"failed":"offline"}"#.into())
        .unwrap();
    session.take_query_update().unwrap();
    session.retry_failed_queries().unwrap();
    let retried = session.take_query_update().unwrap();
    assert!(retried.changed);
    assert_eq!(retried.calls.len(), 1);
    assert_eq!(retried.calls[0].tool_name(), "read");
    let action = session
        .action_at(draw.evaluation, vec!["props".into(), "value".into()])
        .unwrap();
    let run = session.run(Some(action), String::new(), None).unwrap();
    assert!(run.events.is_empty() && run.failure.is_none());
    // In flight: the run is remembered and follows the outcome.
    assert!(session.take_query_update().unwrap().calls.is_empty());
    session
        .complete_tool_call(retried.calls[0].clone(), r#"{"value":"rows"}"#.into())
        .unwrap();
    assert_eq!(session.take_query_update().unwrap().calls.len(), 1);
}
