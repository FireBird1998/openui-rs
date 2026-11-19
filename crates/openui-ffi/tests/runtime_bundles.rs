//! Response state crosses the native worker with both draw entry points.
use openui_ffi::*;
use std::error::Error;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn native_draws_publish_bundle_loading_errors_and_calls_for_the_settled_revision()
-> Result<(), Box<dyn Error>> {
    let library = ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into())?;
    let session = RuntimeSession::new(library.clone(), None)?;
    let program = parse("q=Query(\"cloud\",{},0)\nroot=Box(q)".into(), library, None).program;
    let bundle = |streaming, revision: &str| ResponseState {
        streaming,
        is_bundle: true,
        scripts: vec!["cloud".into()],
        script_revision: Some(revision.into()),
        ..Default::default()
    };
    session.attach_tool_provider()?;
    let first =
        session.replace_program_with_response(program, None, None, true, bundle(true, "A"))?;
    assert!(first.evaluation.is_query_loading);
    assert!(session.take_query_update()?.calls.is_empty());
    session.evaluate_with_response(bundle(false, "A"))?;
    let a = session
        .take_query_update()?
        .calls
        .pop()
        .ok_or("missing first call")?;
    session.complete_tool_call(a, r#"{"value":1}"#.into())?;
    session.set_query_extra_values("cloud".into(), r#"{"tenant":1}"#.into())?;
    session.evaluate_with_response(bundle(false, "A"))?;
    let configured = session
        .take_query_update()?
        .calls
        .pop()
        .ok_or("missing app key call")?;
    session.complete_tool_call(configured, r#"{"value":2}"#.into())?;
    session.evaluate_with_response(bundle(false, "B"))?;
    let b = session
        .take_query_update()?
        .calls
        .pop()
        .ok_or("missing revised script call")?;
    session.complete_tool_call(b, r#"{"value":3}"#.into())?;
    let error = session.evaluate_with_response(ResponseState {
        is_bundle: true,
        error: Some("Invalid scripts bundle".into()),
        ..Default::default()
    })?;
    assert!(!error.is_query_loading);
    assert_eq!(
        error.parser_error_json.as_deref(),
        Some(r#"{"source":"parser","code":"parse-failed","message":"Invalid scripts bundle"}"#)
    );
    assert!(error.json.contains(r#""value":3"#));
    assert!(session.run(None, "click".into(), None)?.events.is_empty());
    session.dispose();
    Ok(())
}
