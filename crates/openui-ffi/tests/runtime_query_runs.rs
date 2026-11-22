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

#[test]
fn refused_tool_name_draws_with_the_state_writes_error() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session.attach_tool_provider().unwrap();
    let source = "$tool=\"search\"\nq=Query($tool,{},\"default\")\nroot=Box(q)";
    let parse_source = |s: &str| parse(s.into(), library.clone(), None).program;
    session
        .replace_program(parse_source(source), None, None, true)
        .unwrap();
    let call = session.take_query_update().unwrap().calls.remove(0);
    session
        .complete_tool_call(call, r#"{"value":"held"}"#.into())
        .unwrap();
    session
        .set("$tool".into(), Some(r#"{"toString":null}"#.into()))
        .unwrap();
    let write = session.take_query_update().unwrap();
    let Some(OpenUiError::Evaluation { reason }) = write.evaluation_error else {
        panic!("expected a tool name conversion error");
    };
    let draw = session.evaluate().unwrap();
    assert!(draw.json.contains(r#""value":"held""#));
    assert!(
        matches!(draw.query_evaluation_error, Some(OpenUiError::Evaluation { reason: error }) if error == reason)
    );
    let other = session
        .replace_program(
            parse_source("q=Query($tool,{},\"fresh\")\nroot=Box(q)"),
            None,
            None,
            true,
        )
        .unwrap()
        .evaluation;
    assert!(other.json.contains(r#""value":"fresh""#));
    assert!(
        matches!(other.query_evaluation_error, Some(OpenUiError::Evaluation { reason: error }) if error == reason)
    );
    assert!(session.take_query_update().unwrap().calls.is_empty());
}
