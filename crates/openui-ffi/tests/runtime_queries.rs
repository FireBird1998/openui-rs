//! Native dispatch and completion go through the session's worker.
use openui_ffi::*;
use std::sync::{Arc, Weak};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

struct Argument(Weak<RuntimeSession>);
impl LibraryFunction for Argument {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        let session = self.0.upgrade().unwrap();
        assert!(matches!(
            session.detach_tool_provider(),
            Err(OpenUiError::Worker { .. })
        ));
        Ok(Some("42".into()))
    }
}

#[test]
fn native_worker_dispatches_live_arguments_and_refuses_reentrant_or_invalid_completions() {
    let library = ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Arg":{"params":{"type":"object","properties":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session
        .register_function("Arg".into(), Box::new(Argument(Arc::downgrade(&session))))
        .unwrap();
    session.set_reference_values(r#"{"q":1}"#.into()).unwrap();
    assert!(matches!(
        session.attach_tool_provider(),
        Err(OpenUiError::ReferenceConflict)
    ));
    session.set_reference_values("{}".into()).unwrap();
    session.attach_tool_provider().unwrap();
    assert!(matches!(
        session.set_reference_values("{}".into()),
        Err(OpenUiError::ReferenceConflict)
    ));
    let program = parse(
        "$n=1\nq=Query(\"read\",{n:$n,x:@Arg()})\nroot=Box(q)".into(),
        library,
        None,
    )
    .program;
    let draw = session
        .replace_program_streaming(program, None, None, true, true)
        .unwrap()
        .evaluation;
    assert!(draw.queries.unwrap().calls.is_empty());
    let call = session
        .evaluate_streaming(false)
        .unwrap()
        .queries
        .unwrap()
        .calls
        .remove(0);
    assert_eq!(call.tool_name(), "read");
    assert_eq!(call.arguments_json().as_deref(), Some(r#"{"n":1,"x":42}"#));
    let update = session
        .complete_tool_call(call.clone(), r#"{"value":"live"}"#.into())
        .unwrap();
    assert!(update.changed && update.calls.is_empty());
    assert!(matches!(
        session.complete_tool_call(call, r#"{"value":0}"#.into()),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(
        session
            .evaluate_streaming(false)
            .unwrap()
            .json
            .contains(r#""value":"live""#)
    );
    let write = session
        .set_with_queries("$n".into(), Some("2".into()))
        .unwrap();
    assert!(write.changed && !write.save_requested);
    assert_eq!(
        write.queries.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":2,"x":42}"#)
    );
    let marker = r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#;
    let write = session
        .write_binding_with_queries(marker.into(), "3".into())
        .unwrap();
    assert_eq!(
        write.queries.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":3,"x":42}"#)
    );
    let field = session
        .set_field_with_queries(
            None,
            Some("Field".into()),
            "$n".into(),
            Some("4".into()),
            true,
        )
        .unwrap();
    assert!(field.save_requested);
    assert_eq!(
        field.queries.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":4,"x":42}"#)
    );
    assert!(session.take_query_update().unwrap().calls.is_empty());
    session.detach_tool_provider().unwrap();
    session
        .set_reference_values(r#"{"q":"supplied"}"#.into())
        .unwrap();
    assert!(session.evaluate().unwrap().json.contains("supplied"));
}
