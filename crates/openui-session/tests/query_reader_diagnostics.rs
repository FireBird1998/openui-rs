//! A truthful completion can be displayed while downstream scheduling reports a limit.
use openui_lang::{JsObject, Library, Limits};
use openui_runtime::{PreparedReferenceValues, ToolCallOutcome, Value};
use openui_session::{
    ComponentLibrary, OpenUiError, ParseLimits, ParsedProgram, RuntimeLimits, SessionCore,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn program(lib: &ComponentLibrary, s: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(openui_lang::parse(s, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}

#[test]
fn downstream_limits_settle_the_source_hide_stale_results_and_recover_on_edit() {
    let schema = serde_json::json!({"$defs":{"Box":{"properties":{"v":{}}}}});
    let lib = Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    });
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut s = SessionCore::new(
        lib.clone(),
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(2048),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    s.attach_tool_provider(&refs).unwrap();
    s.replace_program(
        &refs,
        program(
            &lib,
            "a=Query(\"a\")\nb=Query(\"b\",{v:a+a})\nc=Query(\"c\",{v:b})\nroot=Box(c)",
        ),
        None,
        None,
        true,
    )
    .unwrap();
    let id = s.take_query_update().calls[0].id.clone();
    s.complete_tool_call(&id, ToolCallOutcome::Result(Value::Str("x".repeat(1200))))
        .unwrap();
    let effects = s.take_query_update();
    assert!(
        effects.calls.is_empty()
            && matches!(
                effects.evaluation_error,
                Some(OpenUiError::LimitExceeded { .. })
            )
    );
    assert!(matches!(
        s.complete_tool_call(&id, ToolCallOutcome::Result(Value::Null)),
        Err(OpenUiError::InvalidToolCall)
    ));
    let draw = s.evaluate(&refs).unwrap();
    assert_eq!(draw.query_evaluation_error, effects.evaluation_error);
    let snapshot: serde_json::Value =
        serde_json::from_str(draw.query_snapshot_json.as_deref().unwrap()).unwrap();
    assert_eq!(snapshot["a"], "x".repeat(1200));
    assert!(snapshot["b"].is_null() && snapshot["c"].is_null());
    assert_eq!(snapshot["__openui_errors"].as_array().unwrap().len(), 2);
    let draw = s
        .replace_program(
            &refs,
            program(
                &lib,
                "a=Query(\"a\")\nb=Query(\"b\",{v:@Count([a])})\nroot=Box(b)",
            ),
            None,
            None,
            true,
        )
        .unwrap();
    assert!(draw.evaluation.query_evaluation_error.is_none());
    assert_eq!(s.take_query_update().calls.len(), 1);
}

#[test]
fn a_changed_self_input_displays_a_diagnostic_until_registration_reconciles_the_arguments() {
    let schema = serde_json::json!({"$defs":{"Box":{"properties":{"v":{}}}}});
    let lib = Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    });
    let mut s = SessionCore::new(lib.clone(), None, Default::default()).unwrap();
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    s.attach_tool_provider(&refs).unwrap();
    s.replace_program(
        &refs,
        program(&lib, "a=Query(\"a\")\nroot=Box(a)"),
        None,
        None,
        true,
    )
    .unwrap();
    let id = s.take_query_update().calls[0].id.clone();
    s.complete_tool_call(&id, ToolCallOutcome::Result(Value::Num(1.)))
        .unwrap();
    let p = program(&lib, "a=Query(\"a\",{v:a})\nroot=Box(a)");
    s.replace_program(&refs, p.clone(), None, None, true)
        .unwrap();
    let call = s.take_query_update().calls[0].clone();
    assert_eq!(call.arguments_json().as_deref(), Some(r#"{"v":1}"#));
    s.complete_tool_call(&call.id, ToolCallOutcome::Result(Value::Num(2.)))
        .unwrap();
    let update = s.take_query_update();
    assert!(
        update.calls.is_empty()
            && matches!(&update.evaluation_error,Some(OpenUiError::Evaluation{reason}) if reason.contains("previous self-input"))
    );
    let draw = s.evaluate(&refs).unwrap();
    assert_eq!(draw.query_evaluation_error, update.evaluation_error);
    s.replace_program(&refs, p, None, None, true).unwrap();
    let next = s.take_query_update();
    assert!(next.evaluation_error.is_none());
    assert_eq!(
        next.calls[0].arguments_json().as_deref(),
        Some(r#"{"v":2}"#)
    );
    s.complete_tool_call(&next.calls[0].id, ToolCallOutcome::Result(Value::Num(3.)))
        .unwrap();
    assert!(s.take_query_update().evaluation_error.is_some());
    let before = s.evaluate(&refs).unwrap().query_snapshot_json;
    let literal = program(&lib, "a=Query(\"a\",{v:2})\nroot=Box(a)");
    let draw = s.replace_program(&refs, literal, None, None, true).unwrap();
    assert_eq!(draw.evaluation.query_snapshot_json, before);
    let reconciled = s.take_query_update();
    assert!(
        reconciled.changed && reconciled.calls.is_empty() && reconciled.evaluation_error.is_none()
    );
    assert!(!s.take_query_update().changed);
}
