//! Refreshes and managed mutation continuations across the session's pause boundary.
use openui_lang::{JsObject, Library, Limits, parse};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn setup() -> (
    SessionCore,
    Arc<ComponentLibrary>,
    PreparedReferenceValues<'static>,
) {
    let schema = json!({"$defs":{"Box":{"properties":{"value":{},"action":{}}}}});
    let lib = Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    });
    let s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    (
        s,
        lib,
        PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT),
    )
}

fn program(lib: &ComponentLibrary, text: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(parse(text, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}

#[test]
fn a_streaming_edit_keeps_ticking_the_last_registration_and_resume_waits_for_settle() {
    let (mut s, lib, r) = setup();
    s.attach_tool_provider(&r).unwrap();
    s.replace_program(
        &r,
        program(&lib, "q=Query(\"old\",{},null,2)\nroot=Box(q)"),
        None,
        None,
        true,
    )
    .unwrap();
    let call = s.take_query_update().calls.remove(0);
    s.complete_tool_call_json(&call.id, r#"{"value":"cached"}"#)
        .unwrap();
    s.replace_program_with_response(
        &r,
        program(&lib, "q=Query(\"new\",{},null,1)\nroot=Box(q)"),
        None,
        None,
        true,
        ResponseState::from(true),
    )
    .unwrap();
    s.advance_query_clock(2000).unwrap();
    let tick = s.take_query_update();
    assert_eq!(tick.next_refresh_due_ms, Some(4000));
    assert_eq!(tick.calls[0].tool_name, "old");
    s.pause_queries().unwrap();
    assert!(s.take_query_update().next_refresh_due_ms.is_none());
    s.advance_query_clock(6000).unwrap();
    s.resume_queries().unwrap();
    assert!(s.take_query_update().calls.is_empty());
    let draw = s.evaluate(&r).unwrap();
    assert!(draw.json.contains("cached"));
    let update = s.take_query_update();
    assert_eq!(update.calls[0].tool_name, "new");
    assert_eq!(update.next_refresh_due_ms, Some(7000));
    s.complete_tool_call_json(&tick.calls[0].id, r#"{"failed":"late"}"#)
        .unwrap();
    let after = s.take_query_update();
    assert!(!after.changed && after.calls.is_empty());
    assert_eq!(after.next_refresh_due_ms, Some(7000));
}

#[test]
fn paused_mutation_outcomes_leave_the_plan_and_store_untouched() {
    let upstream = include_str!("../../../conformance/golden/mutation-dispose.jsonl");
    let pin: Json = serde_json::from_str(include_str!(
        "../../../conformance/divergences/refresh-mutation-dispose.json"
    ))
    .unwrap();
    assert_eq!(upstream, pin["reference"].as_str().unwrap());
    let mut rows = String::new();
    for success in [true, false] {
        let (mut s, lib, r) = setup();
        s.attach_tool_provider(&r).unwrap();
        let out = s
            .replace_program(
                &r,
                program(
                    &lib,
                    "m=Mutation(\"save\")\nroot=Box(m,Action([@Run(m),@Set($done,true)]))",
                ),
                None,
                None,
                true,
            )
            .unwrap();
        let action = s
            .action_at(
                &r,
                out.evaluation.evaluation,
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        s.run(&r, Some(action.clone()), "click".into(), None)
            .unwrap();
        let call = s.take_query_update().calls.remove(0);
        assert_eq!(s.pending_continuations(&r).unwrap(), 1);
        s.pause_queries().unwrap();
        assert!(!call.id.is_pending());
        assert_eq!(s.pending_continuations(&r).unwrap(), 0);
        let before = s.evaluate(&r).unwrap().query_snapshot_json.unwrap();
        s.take_query_update();
        let outcome = if success {
            r#"{"value":3}"#
        } else {
            r#"{"failed":"late"}"#
        };
        s.complete_tool_call_json(&call.id, outcome).unwrap();
        let effects = s.take_query_update();
        assert!(!effects.changed && effects.calls.is_empty());
        assert!(matches!(
            s.complete_tool_call_json(&call.id, outcome),
            Err(OpenUiError::InvalidToolCall)
        ));
        assert_eq!(s.get(&r, "$done".into()).unwrap().as_deref(), Some("null"));
        let draw = s.evaluate(&r).unwrap();
        let action = s
            .action_at(&r, draw.evaluation, vec!["props".into(), "action".into()])
            .unwrap();
        s.run(&r, Some(action), "click".into(), None).unwrap();
        let fired = !s.take_query_update().calls.is_empty();
        let after = draw.query_snapshot_json.unwrap();
        let result: Json = serde_json::from_str::<Json>(&after).unwrap()["m"].clone();
        rows.push_str(&format!("{{\"success\":{success},\"before\":{before},\"resumed\":{},\"after\":{after},\"result\":{result},\"fireAfterDispose\":{fired}}}\n", !effects.actions.is_empty()));
    }
    assert_eq!(rows, pin["rust"].as_str().unwrap());
}

#[test]
fn refusing_a_replacement_preserves_the_original_timer_and_dispatch() {
    let (mut s, lib, r) = setup();
    s.attach_tool_provider_with_minimum(&r, 50).unwrap();
    s.replace_program(
        &r,
        program(&lib, "q=Query(\"old\",{},null,0.1)\nroot=Box(q)"),
        None,
        None,
        true,
    )
    .unwrap();
    s.advance_query_clock(40).unwrap();
    let bad = program(
        &lib,
        "q=Query(\"new\",{},null,7)\n$shape={type:\"element\",typeName:\"Box\",props:{},partial:false,hasDynamicProps:false,extra:1}\nroot=true ? $shape : Box()",
    );
    assert!(s.replace_program(&r, bad, None, None, true).is_err());
    let first = s.take_query_update();
    assert_eq!(first.next_refresh_due_ms, Some(100));
    assert_eq!(first.calls.len(), 1);
    s.complete_tool_call_json(&first.calls[0].id, r#"{"value":3}"#)
        .unwrap();
    s.advance_query_clock(100).unwrap();
    assert_eq!(s.take_query_update().calls[0].tool_name, "old");
}

#[test]
fn pause_discards_undrained_calls_and_a_second_resume_preserves_the_deadline() {
    let (mut s, lib, r) = setup();
    s.attach_tool_provider(&r).unwrap();
    s.replace_program(
        &r,
        program(&lib, "q=Query(\"read\",{},null,0.01)\nroot=Box(q)"),
        None,
        None,
        true,
    )
    .unwrap();
    s.pause_queries().unwrap();
    let paused = s.take_query_update();
    assert!(paused.calls.is_empty() && paused.next_refresh_due_ms.is_none());
    s.pause_queries().unwrap();
    assert!(!s.take_query_update().changed);
    s.advance_query_clock(50).unwrap();
    s.resume_queries().unwrap();
    let fresh = s.take_query_update();
    assert_eq!(fresh.calls.len(), 1);
    assert_eq!(fresh.next_refresh_due_ms, Some(1050));
    s.advance_query_clock(75).unwrap();
    s.resume_queries().unwrap();
    let again = s.take_query_update();
    assert!(again.calls.is_empty() && !again.changed);
    assert_eq!(again.next_refresh_due_ms, Some(1050));
    drop(s);
    assert!(!fresh.calls[0].id.is_pending());
}

#[test]
fn repeated_attachment_checks_the_requested_floor_before_touching_calls_or_time() {
    let (mut s, lib, r) = setup();
    assert!(s.attach_tool_provider_at(&r, u64::MAX, 250).is_err());
    assert!(s.check_reference_values().is_ok());
    s.attach_tool_provider_at(&r, 8000, 250).unwrap();
    s.replace_program(
        &r,
        program(&lib, "q=Query(\"q\",{},null,0.01)"),
        None,
        None,
        false,
    )
    .unwrap();
    for invalid in [0, 1000, u64::MAX] {
        assert!(matches!(
            s.attach_tool_provider_with_minimum(&r, invalid),
            Err(OpenUiError::RefusedWrite { .. })
        ));
        assert!(matches!(
            s.attach_tool_provider_at(&r, 16000, invalid),
            Err(OpenUiError::RefusedWrite { .. })
        ));
    }
    s.attach_tool_provider(&r).unwrap();
    s.attach_tool_provider_with_minimum(&r, 250).unwrap();
    let first = s.take_query_update();
    assert_eq!(first.next_refresh_due_ms, Some(8250));
    assert_eq!(first.calls.len(), 1);
    s.complete_tool_call_json(&first.calls[0].id, r#"{"value":42}"#)
        .unwrap();
    s.attach_tool_provider_at(&r, 9999, 250).unwrap();
    let tick = s.take_query_update();
    assert_eq!(tick.next_refresh_due_ms, Some(10249));
    assert_eq!(tick.calls.len(), 1);
}

#[test]
fn resume_and_fresh_attachment_install_timers_on_the_supplied_clock() {
    let (mut s, lib, r) = setup();
    s.attach_tool_provider_at(&r, 8000, 1000).unwrap();
    s.replace_program(
        &r,
        program(&lib, "q=Query(\"q\",{},null,2)"),
        None,
        None,
        false,
    )
    .unwrap();
    s.pause_queries().unwrap();
    assert!(matches!(
        s.resume_queries_at(7999),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(s.take_query_update().next_refresh_due_ms, None);
    s.resume_queries_at(18000).unwrap();
    assert_eq!(s.take_query_update().next_refresh_due_ms, Some(20000));
    s.detach_tool_provider();
    s.attach_tool_provider_at(&r, 55000, 1000).unwrap();
    s.evaluate(&r).unwrap();
    assert_eq!(s.take_query_update().next_refresh_due_ms, Some(57000));
    s.resume_queries_at(58000).unwrap();
    let active = s.take_query_update();
    assert!(active.calls.is_empty());
    assert_eq!(active.next_refresh_due_ms, Some(60000));
}

#[test]
fn pausing_a_waiting_reader_and_resuming_on_the_host_clock_rearms_its_timer_on_delivery() {
    let (mut session, lib, refs) = setup();
    session.attach_tool_provider_at(&refs, 100, 1000).unwrap();
    session
        .replace_program(
            &refs,
            program(
                &lib,
                "a=Query(\"source\",{},null,2)\nb=Query(\"reader\",{x:a},null,1)\nroot=Box(b)",
            ),
            None,
            None,
            true,
        )
        .unwrap();
    let first = session.take_query_update();
    assert_eq!(first.next_refresh_due_ms, Some(2100));
    let [old] = first.calls.as_slice() else {
        panic!("expected just the source")
    };
    session.pause_queries().unwrap();
    assert_eq!(session.take_query_update().next_refresh_due_ms, None);
    session.advance_query_clock(5000).unwrap();
    session
        .complete_tool_call_json(&old.id, r#"{"value":"retired"}"#)
        .unwrap();
    assert!(session.take_query_update().calls.is_empty());
    session.resume_queries_at(5000).unwrap();
    let resumed = session.take_query_update();
    assert_eq!(resumed.next_refresh_due_ms, Some(7000));
    let [source] = resumed.calls.as_slice() else {
        panic!("expected a fresh source call")
    };
    session
        .complete_tool_call(
            &source.id,
            openui_runtime::ToolCallOutcome::Result(openui_runtime::Value::Num(f64::NAN)),
        )
        .unwrap();
    session.advance_query_clock(6000).unwrap();
    let update = session.take_query_update();
    assert!(update.reader_rounds_limited);
    assert_eq!(update.next_refresh_due_ms, Some(7000));
    let [reader] = update.calls.as_slice() else {
        panic!("expected one reader call")
    };
    assert_eq!(reader.tool_name, "reader");
    let drained = session.take_query_update();
    assert!(!drained.reader_rounds_limited && drained.calls.is_empty());
    assert_eq!(drained.next_refresh_due_ms, Some(7000));
}
