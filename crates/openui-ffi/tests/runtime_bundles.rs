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
    assert!(first.evaluation.is_query_loading && first.evaluation.blocked);
    assert!(session.take_query_update()?.calls.is_empty());
    assert!(!session.evaluate_with_response(bundle(false, "A"))?.blocked);
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
    assert!(!error.is_query_loading && error.blocked);
    assert_eq!(
        error.parser_error_json.as_deref(),
        Some(r#"{"source":"parser","code":"parse-failed","message":"Invalid scripts bundle"}"#)
    );
    assert!(error.json.contains(r#""value":3"#));
    assert!(session.run(None, "click".into(), None)?.events.is_empty());
    session.dispose();
    Ok(())
}

#[test]
fn native_set_program_preserves_the_click_and_retry_gate_until_the_next_draw()
-> Result<(), Box<dyn Error>> {
    let library = ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into())?;
    let session = RuntimeSession::new(library.clone(), None)?;
    let program = parse("q=Query(\"cloud\",{},0)\nroot=Box(q)".into(), library, None).program;
    session.attach_tool_provider()?;
    session.replace_program(program.clone(), None, None, true)?;
    let call = session
        .take_query_update()?
        .calls
        .pop()
        .ok_or("missing call")?;
    session.complete_tool_call(call, r#"{"failed":"offline"}"#.into())?;
    session.replace_program_with_response(
        program.clone(),
        None,
        None,
        true,
        ResponseState {
            is_bundle: true,
            streaming: true,
            complete: false,
            ..Default::default()
        },
    )?;
    session.set_program(program)?;
    session.retry_failed_queries()?;
    let default = session.run(None, "click".into(), None)?;
    let legacy = session.run_legacy(r#"{"type":"custom"}"#.into(), "click".into(), None)?;
    assert_eq!(
        (
            default.events.len(),
            legacy.events.len(),
            session.take_query_update()?.calls.len()
        ),
        (0, 0, 0)
    );
    assert!(!session.evaluate()?.blocked);
    session.retry_failed_queries()?;
    assert_eq!(session.take_query_update()?.calls.len(), 1);
    assert_eq!(session.run(None, "click".into(), None)?.events.len(), 1);
    session.dispose();
    Ok(())
}

/// Each script name costs four bytes beside its text (#143), through the
/// native binding as through the session: 1,024 empty names fit 4,096
/// bytes, 1,025 do not, and a refusal leaves the display as it was.
#[test]
fn native_response_state_charges_four_bytes_per_script_name() -> Result<(), Box<dyn Error>> {
    let library = ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into())?;
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_json_bytes: 4096,
            ..Default::default()
        },
    )?;
    session.set_program(parse("root=Box(1)".into(), library, None).program)?;
    let names = |count| ResponseState {
        scripts: vec![String::new(); count],
        ..Default::default()
    };
    let drawn = session.evaluate_with_response(names(1024))?;
    session.evaluate_with_response(ResponseState {
        scripts: vec!["a".repeat(4088), String::new()],
        ..Default::default()
    })?;
    assert!(matches!(
        session.evaluate_with_response(names(1025)),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(matches!(
        session.evaluate_with_response(ResponseState {
            scripts: vec!["a".repeat(4089), String::new()],
            ..Default::default()
        }),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(session.evaluate()?.json, drawn.json);
    session.dispose();
    Ok(())
}
