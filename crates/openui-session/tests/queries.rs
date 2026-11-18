//! Session observations, including the dispatch from the operation that caused it.
use openui_lang::{JsObject, Library};
use openui_runtime::{PreparedReferenceValues, ToolCallOutcome, Value};
use openui_session::*;
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn library(schema: Json) -> Arc<ComponentLibrary> {
    Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    })
}
fn program(lib: &ComponentLibrary, source: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(openui_lang::parse(source, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}
fn base() -> (
    Arc<ComponentLibrary>,
    SessionCore,
    PreparedReferenceValues<'static>,
) {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    (
        lib,
        s,
        PreparedReferenceValues::owned_with_limits(JsObject::new(), openui_lang::Limits::DEFAULT),
    )
}
const MARKER: &str =
    r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#;

#[test]
fn settled_draws_writes_and_blocked_edits_match_the_renderer_bytes() {
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-queries.json")).unwrap();
    let lib = library(corpus["schema"].clone());
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut actual = String::new();
    for scenario in corpus["scenarios"].as_array().unwrap() {
        let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
        s.attach_tool_provider(&refs).unwrap();
        let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
        let mut blocked = true;
        for step in scenario["steps"].as_array().unwrap() {
            let mut output = None;
            if step.get("evaluate").is_some() || step.get("replace").is_some() {
                blocked = step["streaming"] == true
                    || step["complete"] == false
                    || step["error"].is_string();
            }
            let effects = if let Some(source) = step["replace"].as_str() {
                let mut draw = s
                    .replace_program_streaming(
                        &refs,
                        program(&lib, source),
                        None,
                        None,
                        false,
                        blocked,
                    )
                    .unwrap()
                    .evaluation;
                let effects = std::mem::take(&mut draw.queries);
                output = Some(draw);
                effects
            } else if step["evaluate"] == true {
                let mut draw = s.evaluate_streaming(&refs, blocked).unwrap();
                let effects = std::mem::take(&mut draw.queries);
                output = Some(draw);
                effects
            } else if let Some(index) = step["finish"].as_u64() {
                let outcome = if step.get("error").is_some() {
                    json!({"failed":step["error"]})
                } else {
                    json!({"value":step["value"]})
                };
                s.complete_tool_call_json(&calls[index as usize].id, &outcome.to_string())
                    .unwrap()
            } else if let Some(value) = step.get("field") {
                let out = s
                    .set_field_with_queries(
                        &refs,
                        None,
                        Some("Input".into()),
                        "$n".into(),
                        Some(value.to_string()),
                        true,
                    )
                    .unwrap();
                assert!(out.changed && out.save_requested);
                out.queries
            } else if let Some(value) = step.get("binding") {
                let out = s
                    .write_binding_with_queries(&refs, MARKER.into(), value.to_string())
                    .unwrap();
                assert!(out.changed);
                out.queries
            } else {
                let out = s
                    .set_with_queries(&refs, "$n".into(), Some(step["set"].to_string()))
                    .unwrap();
                assert!(out.changed);
                out.queries
            };
            assert!(effects.evaluation_error.is_none());
            let draw = output.unwrap_or_else(|| {
                let draw = s.evaluate_streaming(&refs, blocked).unwrap();
                assert!(draw.queries.calls.is_empty() && !draw.queries.changed);
                draw
            });
            let dispatch = effects
                .calls
                .iter()
                .map(|call| {
                    format!(
                        r#"{{"toolName":{},"args":{}}}"#,
                        json!(call.tool_name),
                        call.arguments_json().as_ref().unwrap()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            actual.push_str(&format!(
                "{{\"tree\":{},\"snapshot\":{},\"calls\":[{}],\"changed\":{}}}\n",
                draw.json,
                draw.query_snapshot_json.unwrap(),
                dispatch,
                effects.changed
            ));
            calls.extend(effects.calls);
        }
    }
    assert_eq!(
        actual,
        include_str!("../../../conformance/golden/session-queries.jsonl")
    );
}

#[test]
fn provider_exclusion_detach_and_single_use_are_enforced_without_effects() {
    let (lib, mut s, refs) = base();
    let mut values = JsObject::new();
    values.define("q", Value::Null);
    let supplied = PreparedReferenceValues::new(&values);
    assert!(matches!(
        s.attach_tool_provider(&supplied),
        Err(OpenUiError::ReferenceConflict)
    ));
    assert!(s.check_reference_values().is_ok());
    s.attach_tool_provider(&refs).unwrap();
    assert!(matches!(
        s.evaluate(&supplied),
        Err(OpenUiError::ReferenceConflict)
    ));
    let p = program(&lib, "q=Query(\"old\")\nroot=Box(q)");
    let old = s
        .replace_program(&refs, p, None, None, true)
        .unwrap()
        .evaluation
        .queries
        .calls
        .remove(0);
    let (_, mut other, _) = base();
    other.attach_tool_provider(&refs).unwrap();
    assert!(matches!(
        other.complete_tool_call_json(&old.id, r#"{"value":2}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(matches!(
        s.complete_tool_call_json(&old.id, "{}"),
        Err(OpenUiError::InvalidJson { .. })
    ));
    assert!(
        s.complete_tool_call_json(&old.id, r#"{"value":3}"#)
            .unwrap()
            .changed
    );
    assert!(matches!(
        s.complete_tool_call_json(&old.id, r#"{"value":2}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(s.evaluate(&refs).unwrap().json.contains("\"value\":3"));
    s.detach_tool_provider();
    assert!(s.check_reference_values().is_ok());
    assert!(s.evaluate(&refs).unwrap().query_snapshot_json.is_none());
    s.attach_tool_provider(&refs).unwrap();
    assert_eq!(s.evaluate(&refs).unwrap().queries.calls.len(), 1);
}

#[test]
fn refused_native_view_preserves_the_pending_call_state_and_streaming_gate() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(&lib, "$n=1\nq=Query(\"old\",{n:$n})\nroot=Box(q)");
    let old = s
        .replace_program(&refs, p, None, None, true)
        .unwrap()
        .evaluation
        .queries
        .calls
        .remove(0);
    let bad = program(
        &lib,
        "$n=99\nq=Query(\"bad\")\n$shape={type:\"element\",typeName:\"Box\",props:{},partial:false,hasDynamicProps:false,extra:1}\nroot=true ? $shape : Box()",
    );
    let before = s.version(&refs).unwrap();
    assert!(
        s.replace_program(&refs, bad.clone(), None, None, true)
            .is_err()
    );
    assert_eq!(s.version(&refs).unwrap(), before);
    assert!(s.take_query_update().calls.is_empty());
    assert!(
        s.complete_tool_call(&old.id, ToolCallOutcome::Result(Value::Num(7.)))
            .unwrap()
            .changed
    );
    let output = s.evaluate(&refs).unwrap();
    assert!(output.json.contains("\"value\":7") && output.queries.calls.is_empty());
    assert!(
        s.replace_program_streaming(&refs, bad, None, None, true, true)
            .is_err()
    );
    // A refused blocked edit must not close the old program's settled gate.
    assert_eq!(
        s.set_with_queries(&refs, "$n".into(), Some("2".into()))
            .unwrap()
            .queries
            .calls[0]
            .tool_name,
        "old"
    );
}

#[test]
fn mcp_numbers_surrogates_and_reported_errors_reach_the_live_program() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(
        &lib,
        "a=Query(\"a\")\nb=Query(\"b\")\nc=Query(\"c\")\nd=Query(\"d\")\nroot=Box([\"\"+a,b,c])",
    );
    let calls = s
        .replace_program(&refs, p, None, None, true)
        .unwrap()
        .evaluation
        .queries
        .calls;
    for (call, response) in calls.iter().zip([
        r#"{"structuredContent":1e400}"#,
        r#"{"structuredContent":-0}"#,
        r#"{"structuredContent":"\ud800"}"#,
        r#"{"isError":true,"content":[{"type":"text","text":"denied"}]}"#,
    ]) {
        s.complete_tool_call_json(&call.id, &json!({"mcpResponseJson":response}).to_string())
            .unwrap();
    }
    let draw = s.evaluate(&refs).unwrap();
    assert!(
        draw.json.contains(r#"["Infinity",0,"\ud800"]"#),
        "{}",
        draw.json
    );
    // The store compares numbers with Object.is, including the sign of zero.
    let copy_b = r#"{"__reactive":"assign","target":"$signed","expr":{"k":"RuntimeRef","n":"b","refType":"query"}}"#;
    assert!(
        s.write_binding(&refs, copy_b.into(), "null".into())
            .unwrap()
    );
    assert_eq!(
        s.get(&refs, "$signed".into()).unwrap().as_deref(),
        Some("0")
    );
    assert!(s.set(&refs, "$signed".into(), Some("0".into())).unwrap());
    assert!(
        draw.query_snapshot_json
            .unwrap()
            .contains(r#""code":"mcp-error","message":"Query \"d\" returned an error: denied""#)
    );
}

#[test]
fn refusing_deep_large_or_shared_outcomes_leaves_the_same_receipt_retryable() {
    let (lib, _, refs) = base();
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            result_size: Some(1024),
            depth: Some(32),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
    s.attach_tool_provider(&refs).unwrap();
    let draw = s
        .replace_program(
            &refs,
            program(&lib, "a=Query(\"x\")\nb=Query(\"x\")\nroot=Box(a)"),
            None,
            None,
            true,
        )
        .unwrap()
        .evaluation;
    let id = draw.queries.calls[0].id.clone();
    for json in [
        format!(r#"{{"value":{}0{}}}"#, "[".repeat(40), "]".repeat(40)),
        json!({"value":"X".repeat(2000)}).to_string(),
        json!({"value":"X".repeat(600)}).to_string(),
        json!({"failed":"X".repeat(600)}).to_string(),
        json!({"availableTools":["X".repeat(600)]}).to_string(),
    ] {
        assert!(matches!(
            s.complete_tool_call_json(&id, &json),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        let again = s.evaluate(&refs).unwrap();
        assert_eq!(again.query_snapshot_json, draw.query_snapshot_json);
        assert!(again.queries.calls.is_empty());
    }
    let effect = s
        .complete_tool_call_json(&id, r#"{"availableTools":["read","write"]}"#)
        .unwrap();
    assert!(effect.changed);
    let draw = s.evaluate(&refs).unwrap();
    let snapshot: Json = serde_json::from_str(&draw.query_snapshot_json.unwrap()).unwrap();
    assert_eq!(
        snapshot["__openui_errors"][0]["hint"],
        "Available tools: read, write"
    );
    assert_eq!(snapshot["__openui_errors"][1]["code"], "tool-not-found");
}

#[test]
fn legacy_writes_dispatch_before_returning_and_a_refusal_preserves_unclaimed_calls() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(&lib, "$n=1\nq=Query(\"x\",{n:$n})\nroot=Box(@Set($n,2))");
    let draw = s
        .replace_program(&refs, p.clone(), None, None, true)
        .unwrap()
        .evaluation;
    let action = s
        .action_at(&refs, draw.evaluation, vec!["props".into(), "value".into()])
        .unwrap();
    s.run(&refs, Some(action), String::new(), None).unwrap();
    assert!(matches!(
        s.set(&refs, "$n".into(), Some("broken".into())),
        Err(OpenUiError::InvalidJson { .. })
    ));
    let blocked = s.evaluate_streaming(&refs, true).unwrap();
    assert!(blocked.queries.calls.is_empty());
    let calls = s.take_query_update().calls;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments_json().as_deref(), Some(r#"{"n":2}"#));
    s.set_program(&refs, p.clone()).unwrap();
    s.set(&refs, "$n".into(), Some("3".into())).unwrap();
    assert!(s.take_query_update().calls.is_empty());
    let draw = s.evaluate_streaming(&refs, false).unwrap();
    assert_eq!(
        draw.queries.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":3}"#)
    );
    s.detach_tool_provider();
    s.attach_tool_provider(&refs).unwrap();
    let fresh = s
        .evaluate_streaming(&refs, false)
        .unwrap()
        .queries
        .calls
        .remove(0);
    assert!(matches!(
        s.complete_tool_call_json(&draw.queries.calls[0].id, r#"{"value":3}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    s.attach_tool_provider(&refs).unwrap();
    assert!(
        s.complete_tool_call_json(&fresh.id, r#"{"value":3}"#)
            .unwrap()
            .changed
    );
}

#[test]
fn a_query_cut_after_an_accepted_write_preserves_registration_and_reports_the_limit() {
    let (lib, _, refs) = base();
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            result_size: Some(2048),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
    s.attach_tool_provider(&refs).unwrap();
    let draw = s
        .replace_program(
            &refs,
            program(
                &lib,
                "$n=\"a\"\nq=Query(\"search\",{term:$n+$n})\nroot=Box(q)",
            ),
            None,
            None,
            true,
        )
        .unwrap()
        .evaluation;
    let id = draw.queries.calls[0].id.clone();
    let value = json!("x".repeat(1500)).to_string();
    let write = s
        .set_with_queries(&refs, "$n".into(), Some(value.clone()))
        .unwrap();
    assert!(write.changed && write.queries.calls.is_empty());
    assert!(matches!(
        write.queries.evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(
        s.get(&refs, "$n".into()).unwrap().as_deref(),
        Some(value.as_str())
    );
    assert!(matches!(
        s.evaluate(&refs),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(
        s.complete_tool_call_json(&id, r#"{"value":1}"#)
            .unwrap()
            .changed
    );
    let recovery = s
        .set_with_queries(&refs, "$n".into(), Some(r#""b""#.into()))
        .unwrap();
    assert!(recovery.queries.evaluation_error.is_none());
    assert_eq!(
        recovery.queries.calls[0].arguments_json().as_deref(),
        Some(r#"{"term":"bb"}"#)
    );
}
