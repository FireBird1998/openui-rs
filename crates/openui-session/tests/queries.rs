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
        assert!(s.take_query_update().changed); // The provider makes the query snapshot visible.
        let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
        let mut blocked = true;
        for step in scenario["steps"].as_array().unwrap() {
            let mut output = None;
            if step.get("evaluate").is_some() || step.get("replace").is_some() {
                blocked = step["streaming"] == true
                    || step["complete"] == false
                    || step["error"].is_string();
            }
            let response = ResponseState { streaming: blocked };
            if let Some(source) = step["replace"].as_str() {
                output = Some(
                    s.replace_program_with_response(
                        &refs,
                        program(&lib, source),
                        None,
                        None,
                        false,
                        response,
                    )
                    .unwrap()
                    .evaluation,
                );
            } else if step["evaluate"] == true {
                output = Some(s.evaluate_with_response(&refs, response).unwrap());
            } else if let Some(index) = step["finish"].as_u64() {
                let outcome = if step.get("error").is_some() {
                    json!({"failed":step["error"]})
                } else {
                    json!({"value":step["value"]})
                };
                s.complete_tool_call_json(&calls[index as usize].id, &outcome.to_string())
                    .unwrap();
            } else if let Some(value) = step.get("field") {
                let out = s
                    .set_field(
                        &refs,
                        None,
                        Some("Input".into()),
                        "$n".into(),
                        Some(value.to_string()),
                        true,
                    )
                    .unwrap();
                assert!(out.changed && out.save_requested);
            } else if let Some(value) = step.get("binding") {
                assert!(
                    s.write_binding(&refs, MARKER.into(), value.to_string())
                        .unwrap()
                );
            } else {
                assert!(
                    s.set(&refs, "$n".into(), Some(step["set"].to_string()))
                        .unwrap()
                );
            }
            let effects = s.take_query_update();
            assert!(effects.evaluation_error.is_none());
            let draw = output.unwrap_or_else(|| {
                let draw = s.evaluate_with_response(&refs, response).unwrap();
                let update = s.take_query_update();
                assert!(update.calls.is_empty() && !update.changed);
                draw
            });
            let dispatch = effects
                .calls
                .iter()
                .map(|call| {
                    let mut fields = format!(r#"{{"toolName":{}"#, json!(call.tool_name));
                    if let Some(args) = call.arguments_json() {
                        fields.push_str(r#","args":"#);
                        fields.push_str(&args);
                    }
                    fields.push('}');
                    fields
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
    s.replace_program(&refs, p, None, None, true).unwrap();
    let old = s.take_query_update().calls.remove(0);
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
    s.complete_tool_call_json(&old.id, r#"{"value":3}"#)
        .unwrap();
    assert!(s.take_query_update().changed);
    assert!(matches!(
        s.complete_tool_call_json(&old.id, r#"{"value":2}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(s.evaluate(&refs).unwrap().json.contains("\"value\":3"));
    s.detach_tool_provider();
    s.detach_tool_provider();
    assert!(s.take_query_update().changed);
    s.detach_tool_provider();
    assert!(!s.take_query_update().changed);
    assert!(s.check_reference_values().is_ok());
    assert!(s.evaluate(&refs).unwrap().query_snapshot_json.is_none());
    s.attach_tool_provider(&refs).unwrap();
    s.evaluate(&refs).unwrap();
    assert_eq!(s.take_query_update().calls.len(), 1);
}

#[test]
fn refused_native_view_preserves_the_pending_call_state_and_streaming_gate() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(&lib, "$n=1\nq=Query(\"old\",{n:$n})\nroot=Box(q)");
    s.replace_program(&refs, p, None, None, true).unwrap();
    let old = s.take_query_update().calls.remove(0);
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
    s.complete_tool_call(&old.id, ToolCallOutcome::Result(Value::Num(7.)))
        .unwrap();
    assert!(s.take_query_update().changed);
    let output = s.evaluate(&refs).unwrap();
    assert!(output.json.contains("\"value\":7") && s.take_query_update().calls.is_empty());
    assert!(
        s.replace_program_with_response(
            &refs,
            bad,
            None,
            None,
            true,
            ResponseState { streaming: true }
        )
        .is_err()
    );
    // A refused blocked edit must not close the old program's settled gate.
    assert!(s.set(&refs, "$n".into(), Some("2".into())).unwrap());
    assert_eq!(s.take_query_update().calls[0].tool_name, "old");
}

#[test]
fn mcp_numbers_surrogates_and_reported_errors_reach_the_live_program() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(
        &lib,
        "a=Query(\"a\")\nb=Query(\"b\")\nc=Query(\"c\")\nd=Query(\"d\")\nroot=Box([\"\"+a,b,c])",
    );
    s.replace_program(&refs, p, None, None, true).unwrap();
    let calls = s.take_query_update().calls;
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
fn refusing_long_failure_text_leaves_the_same_receipt_retryable() {
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
    let id = s.take_query_update().calls[0].id.clone();
    // A deep, oversized or unshowable result is the tool's and is delivered
    // as a tool-error (tests/result_limits.rs). Failure text past the limits
    // is the embedder's to shorten: it refuses the command.
    for json in [
        json!({"failed":"X".repeat(600)}).to_string(),
        json!({"availableTools":["X".repeat(600)]}).to_string(),
    ] {
        assert!(matches!(
            s.complete_tool_call_json(&id, &json),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        let again = s.evaluate(&refs).unwrap();
        assert_eq!(again.query_snapshot_json, draw.query_snapshot_json);
        assert!(s.take_query_update().calls.is_empty());
    }
    s.complete_tool_call_json(&id, r#"{"availableTools":["read","write"]}"#)
        .unwrap();
    assert!(s.take_query_update().changed);
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
    assert_eq!(s.take_query_update().calls.len(), 1);
    let action = s
        .action_at(&refs, draw.evaluation, vec!["props".into(), "value".into()])
        .unwrap();
    s.run(&refs, Some(action), String::new(), None).unwrap();
    assert!(matches!(
        s.set(&refs, "$n".into(), Some("broken".into())),
        Err(OpenUiError::InvalidJson { .. })
    ));
    s.evaluate_with_response(&refs, ResponseState { streaming: true })
        .unwrap();
    let calls = s.take_query_update().calls;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments_json().as_deref(), Some(r#"{"n":2}"#));
    s.set_program(&refs, p.clone()).unwrap();
    s.set(&refs, "$n".into(), Some("3".into())).unwrap();
    assert!(s.take_query_update().calls.is_empty());
    s.evaluate(&refs).unwrap();
    let parked = s.take_query_update();
    assert_eq!(
        parked.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":3}"#)
    );
    s.detach_tool_provider();
    s.attach_tool_provider(&refs).unwrap();
    s.evaluate(&refs).unwrap();
    let fresh = s.take_query_update().calls.remove(0);
    assert!(matches!(
        s.complete_tool_call_json(&parked.calls[0].id, r#"{"value":3}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    s.attach_tool_provider(&refs).unwrap();
    s.complete_tool_call_json(&fresh.id, r#"{"value":3}"#)
        .unwrap();
    assert!(s.take_query_update().changed);
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
    let id = s.take_query_update().calls[0].id.clone();
    let value = json!("x".repeat(1500)).to_string();
    let changed = s.set(&refs, "$n".into(), Some(value.clone())).unwrap();
    let write = s.take_query_update();
    assert!(changed && write.changed && write.calls.is_empty());
    assert!(matches!(
        write.evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(
        s.get(&refs, "$n".into()).unwrap().as_deref(),
        Some(value.as_str())
    );
    let fallback = s.evaluate(&refs).unwrap();
    assert_eq!(fallback.json, draw.json);
    assert_eq!(fallback.query_snapshot_json, draw.query_snapshot_json);
    assert!(matches!(
        fallback.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert!(!s.take_query_update().changed);
    s.complete_tool_call_json(&id, r#"{"value":1}"#).unwrap();
    assert!(s.take_query_update().changed);
    s.set(&refs, "$n".into(), Some(r#""b""#.into())).unwrap();
    let recovery = s.take_query_update();
    assert!(recovery.changed && recovery.evaluation_error.is_none());
    assert!(s.evaluate(&refs).unwrap().query_evaluation_error.is_none());
    assert_eq!(
        recovery.calls[0].arguments_json().as_deref(),
        Some(r#"{"term":"bb"}"#)
    );
    // Block a second oversized write, then hit the limit on the settling draw itself.
    s.evaluate_with_response(&refs, ResponseState { streaming: true })
        .unwrap();
    s.set(&refs, "$n".into(), Some(value)).unwrap();
    let settled = s.evaluate(&refs).unwrap();
    assert!(settled.json.contains(r#""value":1"#));
    assert!(matches!(
        settled.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    let cut = s.take_query_update();
    assert!(cut.changed && cut.calls.is_empty());
    assert_eq!(cut.evaluation_error, settled.query_evaluation_error);
    let proposed = program(&lib, "q=Query(\"new\",{term:$n+$n})\nroot=Box([q,42])");
    let replaced = s
        .replace_program(&refs, proposed, None, None, true)
        .unwrap()
        .evaluation;
    assert!(replaced.json.contains("[1,42]"));
    assert_eq!(
        replaced.query_evaluation_error,
        settled.query_evaluation_error
    );
    assert!(s.take_query_update().calls.is_empty());
}

#[test]
fn an_undrained_driver_keeps_only_live_calls_across_writes_draws_and_completions() {
    let (lib, mut s, refs) = base();
    s.attach_tool_provider(&refs).unwrap();
    let p = program(
        &lib,
        "$n=0\na=Query(\"keep\")\nb=Query(\"vary\",{n:$n})\nroot=Box([a,b])",
    );
    s.replace_program(&refs, p, None, None, false).unwrap();
    for n in 1..100 {
        s.set(&refs, "$n".into(), Some(n.to_string())).unwrap();
        s.evaluate(&refs).unwrap();
    }
    let update = s.take_query_update();
    assert!(update.changed);
    assert_eq!(update.calls.len(), 3);
    // The varying query retains both its current and previous entries (ADR 0013).
    assert_eq!(
        update.calls[1].arguments_json().as_deref(),
        Some(r#"{"n":98}"#)
    );
    assert_eq!(update.calls[0].tool_name, "keep");
    assert_eq!(
        update.calls[2].arguments_json().as_deref(),
        Some(r#"{"n":99}"#)
    );
    s.set(&refs, "$n".into(), Some("100".into())).unwrap();
    s.complete_tool_call_json(&update.calls[0].id, r#"{"value":"finished"}"#)
        .unwrap();
    let next = s.take_query_update();
    assert!(next.changed);
    assert_eq!(next.calls.len(), 1);
    assert_eq!(
        next.calls[0].arguments_json().as_deref(),
        Some(r#"{"n":100}"#)
    );
    let empty = s.take_query_update();
    assert!(empty.calls.is_empty() && !empty.changed);
}

#[test]
fn a_blocked_first_draw_uses_declared_defaults_only_without_a_provider() {
    let (lib, mut s, refs) = base();
    let p = program(&lib, "q=Query(\"read\",{},\"waiting\")\nroot=Box(q)");
    let response = ResponseState { streaming: true };
    let plain = s
        .replace_program_with_response(&refs, p, None, None, false, response)
        .unwrap();
    assert!(plain.evaluation.json.contains(r#""value":"waiting""#));
    s.attach_tool_provider(&refs).unwrap();
    assert!(
        s.evaluate_with_response(&refs, response)
            .unwrap()
            .json
            .contains(r#""value":null"#)
    );
    assert!(s.take_query_update().calls.is_empty());
    assert!(
        s.evaluate(&refs)
            .unwrap()
            .json
            .contains(r#""value":"waiting""#)
    );
    assert_eq!(s.take_query_update().calls.len(), 1);
}
