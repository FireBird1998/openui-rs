//! Where a tool outcome's JSON ends and its result begins (#90). A result too
//! large or too deep for the session's limits, or past its cache cap, is the
//! call's `tool-error`. Malformed or wrongly shaped outcome JSON is the
//! embedder's, and is refused with nothing consumed.
use openui_lang::{JsObject, Library};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

const SIZE: u64 = 1024;
const DEPTH: u64 = 8;

fn library() -> Arc<ComponentLibrary> {
    let schema = json!({"$defs":{"Box":{"properties":{"value":{},"action":{}}}}});
    Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    })
}

fn refs() -> PreparedReferenceValues<'static> {
    PreparedReferenceValues::owned_with_limits(JsObject::new(), openui_lang::Limits::DEFAULT)
}

fn limits(cache: Option<u64>) -> RuntimeLimits {
    RuntimeLimits {
        evaluation: ParseLimits {
            result_size: Some(SIZE),
            depth: Some(DEPTH),
            ..Default::default()
        },
        max_cache_bytes: cache,
        ..Default::default()
    }
}

/// A settled session with a provider, its program drawn, and its first call.
fn session(source: &str, cache: Option<u64>) -> (SessionCore, Arc<WantedToolCall>) {
    let (s, mut calls) = session_calls(source, cache);
    (s, calls.remove(0))
}

fn session_calls(source: &str, cache: Option<u64>) -> (SessionCore, Vec<Arc<WantedToolCall>>) {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, limits(cache)).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let inner = Arc::new(openui_lang::parse(source, &lib.inner, None));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    s.replace_program(&r, program, None, None, true).unwrap();
    let calls = s.take_query_update().calls;
    (s, calls)
}

fn snapshot(s: &mut SessionCore) -> Json {
    let draw = s.evaluate(&refs()).unwrap();
    serde_json::from_str(&draw.query_snapshot_json.unwrap()).unwrap()
}

/// `{"value":"xx…"}` whose result's JSON is `bytes` long.
fn text_outcome(bytes: usize) -> String {
    json!({"value": "x".repeat(bytes - 2)}).to_string()
}

fn nested(levels: usize) -> String {
    format!("{}0{}", "[".repeat(levels), "]".repeat(levels))
}

fn tool_error(s: &mut SessionCore, message: &str) {
    let snapshot = snapshot(s);
    let error = &snapshot["__openui_errors"][0];
    assert_eq!(error["code"], "tool-error");
    assert_eq!(error["message"], format!("Query \"t\" failed: {message}"));
}

#[test]
fn through_json_a_result_fits_at_the_size_limit_and_one_byte_over_is_a_tool_error() {
    let size = format!("tool result exceeds the result size limit of {SIZE} bytes");
    // Compact JSON one byte over is longer than the outcome bound and is never
    // parsed. A result whose JSON grows past the limit when written (each
    // `1e300` is written `1e+300`) is measured after reading.
    let grows = format!("{{\"value\":[{}]}}", vec!["1e300"; 170].join(","));
    assert!(grows.len() <= SIZE as usize + 10);
    for (outcome, refused) in [
        (text_outcome(SIZE as usize), None),
        (text_outcome(SIZE as usize + 1), Some(&size)),
        (grows, Some(&size)),
    ] {
        let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
        s.complete_tool_call_json(&call.id, &outcome).unwrap();
        assert!(!call.id.is_pending(), "the call is consumed either way");
        match refused {
            Some(message) => tool_error(&mut s, message),
            None => assert_eq!(snapshot(&mut s)["__openui_errors"], json!([])),
        }
        assert!(matches!(
            s.complete_tool_call_json(&call.id, r#"{"value":1}"#),
            Err(OpenUiError::InvalidToolCall)
        ));
    }
}

#[test]
fn through_json_a_result_fits_at_the_depth_limit_and_one_level_over_is_a_tool_error() {
    let depth = format!("tool result nests deeper than the depth limit of {DEPTH} levels");
    for (levels, fits) in [(DEPTH as usize, true), (DEPTH as usize + 1, false)] {
        let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
        let outcome = format!("{{\"value\":{}}}", nested(levels));
        s.complete_tool_call_json(&call.id, &outcome).unwrap();
        if fits {
            assert_eq!(
                snapshot(&mut s)["q"],
                serde_json::from_str::<Json>(&nested(levels)).unwrap()
            );
        } else {
            tool_error(&mut s, &depth);
        }
    }
    // Too deep and too long at once: the scan of the long text finds the depth.
    let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
    let outcome = format!("{{\"value\":{}}}", nested(2000));
    s.complete_tool_call_json(&call.id, &outcome).unwrap();
    tool_error(&mut s, &depth);
}

#[test]
fn malformed_or_wrongly_shaped_outcomes_are_refused_and_the_call_stays_completable() {
    let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
    let before = snapshot(&mut s);
    let long = "x".repeat(4 * SIZE as usize);
    for (outcome, invalid_json) in [
        (r#"{"value":1"#.to_owned(), true),
        (r#"{"value":1,"failed":"x"}"#.to_owned(), true),
        // Validity first, whatever the length.
        (format!("{{\"value\":\"{long}\""), true),
        (format!("{{\"value\":\"{long}\",\"failed\":\"x\"}}"), false),
        (json!({"failed": long}).to_string(), false),
        (format!("{{\"failed\":{}}}", nested(40)), false),
        (format!("{{\"x\":1,\"value\":{}}}", nested(40)), false),
    ] {
        let refused = s.complete_tool_call_json(&call.id, &outcome);
        if invalid_json {
            assert!(
                matches!(refused, Err(OpenUiError::InvalidJson { .. })),
                "{outcome:.40}"
            );
        } else {
            assert!(
                matches!(refused, Err(OpenUiError::LimitExceeded { .. })),
                "{outcome:.40}"
            );
        }
        assert_eq!(snapshot(&mut s), before);
        assert!(call.id.is_pending());
    }
    s.complete_tool_call_json(&call.id, r#"{"value":"fits"}"#)
        .unwrap();
    assert_eq!(snapshot(&mut s)["q"], "fits");
}

#[test]
fn an_oversized_mcp_response_is_the_tools_result_too() {
    let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
    let response =
        json!({"structuredContent": {"rows": "x".repeat(2 * SIZE as usize)}}).to_string();
    let outcome = json!({ "mcpResponseJson": response }).to_string();
    s.complete_tool_call_json(&call.id, &outcome).unwrap();
    tool_error(
        &mut s,
        &format!("tool result exceeds the result size limit of {SIZE} bytes"),
    );
    // Malformed MCP JSON stays the embedder's error.
    let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
    assert!(matches!(
        s.complete_tool_call_json(&call.id, r#"{"mcpResponseJson":"{"}"#),
        Err(OpenUiError::InvalidJson { .. })
    ));
    assert!(call.id.is_pending());
}

#[test]
fn the_cache_cap_is_set_through_the_session_limits() {
    let source = "a = Query(\"a\")\nb = Query(\"t\")\nroot = Box([a, b])";
    let (mut s2, calls) = session_calls(source, Some(100));
    s2.complete_tool_call_json(&calls[0].id, &text_outcome(40))
        .unwrap();
    // 40 + 60 fits the cap of 100; 40 + 61 does not.
    s2.complete_tool_call_json(&calls[1].id, &text_outcome(61))
        .unwrap();
    tool_error(
        &mut s2,
        "tool result does not fit the cache limit of 100 bytes",
    );
    assert_eq!(
        snapshot(&mut s2)["a"],
        "x".repeat(38),
        "nothing held is evicted"
    );
    s2.retry_failed_queries().unwrap();
    let retry = s2.take_query_update().calls.remove(0);
    s2.complete_tool_call_json(&retry.id, &text_outcome(60))
        .unwrap();
    assert_eq!(snapshot(&mut s2)["__openui_errors"], json!([]));
}

#[test]
fn a_mutation_result_refused_under_a_limit_ends_its_plan() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, limits(None)).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let inner = Arc::new(openui_lang::parse(
        "m = Mutation(\"save\")\nroot = Box(m, Action([@Run(m), @Set($done, true)]))",
        &lib.inner,
        None,
    ));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    let draw = s
        .replace_program(&r, program, None, None, true)
        .unwrap()
        .evaluation;
    let action = s
        .action_at(&r, draw.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    s.run(&r, Some(action), String::new(), None).unwrap();
    let call = s.take_query_update().calls.remove(0);
    s.complete_tool_call_json(&call.id, &text_outcome(SIZE as usize + 1))
        .unwrap();
    let update = s.take_query_update();
    assert_eq!(update.actions.len(), 1);
    assert!(update.actions[0].events.is_empty(), "the tail did not run");
    assert_ne!(s.get(&r, "$done".into()).unwrap().as_deref(), Some("true"));
    let snapshot = snapshot(&mut s);
    assert_eq!(snapshot["m"]["status"], "error");
    assert_eq!(
        snapshot["__openui_errors"][0]["message"],
        format!(
            "Mutation \"save\" failed: tool result exceeds the result size limit of {SIZE} bytes"
        )
    );
}

#[test]
fn delivering_an_oversized_json_result_allocates_nothing_in_proportion_to_it() {
    let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
    let outcome = text_outcome(4 << 20);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    s.complete_tool_call_json(&call.id, &outcome).unwrap();
    let extra = test_alloc::peak().saturating_sub(before);
    assert!(extra < 64 * 1024, "{extra} bytes allocated to refuse");
    tool_error(
        &mut s,
        &format!("tool result exceeds the result size limit of {SIZE} bytes"),
    );
}

#[test]
fn validity_is_decided_before_size_for_long_mcp_responses_in_either_order() {
    let size = format!("tool result exceeds the result size limit of {SIZE} bytes");
    let padding = " ".repeat(SIZE as usize);
    // Malformed MCP text is refused at any length; valid text past the
    // bound is too large.
    for (response, malformed) in [
        ("{".to_owned(), true),
        (format!("{padding}{{"), true),
        (format!("{{{padding}"), true),
        (format!("{padding}{{}}"), false),
        (format!("{{}}{padding}"), false),
    ] {
        let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
        let outcome = json!({ "mcpResponseJson": response }).to_string();
        let result = s.complete_tool_call_json(&call.id, &outcome);
        if malformed {
            assert!(
                matches!(result, Err(OpenUiError::InvalidJson { .. })),
                "{response:.8}"
            );
            assert!(call.id.is_pending());
        } else if response.len() > SIZE as usize {
            result.unwrap();
            tool_error(&mut s, &size);
        }
    }
}

#[test]
fn a_mutation_result_refused_by_the_runtime_ends_its_plan() {
    // Within the outcome bound, so the runtime's own admission refuses it:
    // each `1e300` is written `1e+300`.
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, limits(None)).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let inner = Arc::new(openui_lang::parse(
        "m = Mutation(\"save\")\nroot = Box(m, Action([@Run(m), @Set($done, true)]))",
        &lib.inner,
        None,
    ));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    let draw = s
        .replace_program(&r, program, None, None, true)
        .unwrap()
        .evaluation;
    let action = s
        .action_at(&r, draw.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    s.run(&r, Some(action), String::new(), None).unwrap();
    let call = s.take_query_update().calls.remove(0);
    let grows = format!("{{\"value\":[{}]}}", vec!["1e300"; 170].join(","));
    assert!(grows.len() <= SIZE as usize + 10);
    s.complete_tool_call_json(&call.id, &grows).unwrap();
    assert_eq!(s.take_query_update().actions.len(), 1);
    assert_ne!(
        s.get(&r, "$done".into()).unwrap().as_deref(),
        Some("true"),
        "the tail did not run"
    );
    assert_eq!(snapshot(&mut s)["m"]["status"], "error");
}

#[test]
fn a_result_two_readers_cannot_both_show_is_their_tool_error() {
    let (mut s, call) = session(
        "a = Query(\"t\")\nb = Query(\"t\")\nroot = Box([a, b])",
        None,
    );
    s.complete_tool_call_json(&call.id, &text_outcome(600))
        .unwrap();
    assert!(!call.id.is_pending());
    let snapshot = snapshot(&mut s);
    assert_eq!(snapshot["__openui_loading"], json!([]));
    assert_eq!(snapshot["__openui_errors"].as_array().unwrap().len(), 2);
}

#[test]
fn an_over_limit_json_result_ends_the_call_for_twenty_four_readers() {
    let source = (0..24)
        .map(|i| format!("q{i} = Query(\"t\")"))
        .collect::<Vec<_>>()
        .join("\n");
    for outcome in [
        // Past the outcome bound: refused before it is read.
        text_outcome(SIZE as usize + 1),
        // Within it, each `1e300` is written `1e+300`: refused after reading.
        format!("{{\"value\":[{}]}}", vec!["1e300"; 170].join(",")),
    ] {
        let (mut s, call) = session(&format!("{source}\nroot = Box(q0)"), None);
        s.complete_tool_call_json(&call.id, &outcome).unwrap();
        assert!(!call.id.is_pending());
        let snapshot = snapshot(&mut s);
        assert_eq!(snapshot["__openui_loading"], json!([]));
        assert_eq!(
            snapshot["__openui_errors"][0]["message"],
            format!(
                "24 queries failed with errors that do not fit the result size limit of {SIZE} bytes"
            )
        );
    }
}

#[test]
fn a_null_mcp_response_is_refused_at_any_length() {
    for response in [
        "null".to_owned(),
        format!("{}null", " ".repeat(SIZE as usize)),
    ] {
        let (mut s, call) = session("q = Query(\"t\")\nroot = Box(q)", None);
        let outcome = json!({ "mcpResponseJson": response }).to_string();
        assert!(
            matches!(
                s.complete_tool_call_json(&call.id, &outcome),
                Err(OpenUiError::Evaluation { .. })
            ),
            "{} bytes",
            response.len()
        );
        assert!(call.id.is_pending());
    }
}

#[test]
fn a_refused_json_result_of_an_older_submission_is_that_calls_failure() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, limits(None)).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let parsed = |source: &str| {
        let inner = Arc::new(openui_lang::parse(source, &lib.inner, None));
        Arc::new(ParsedProgram {
            json_bytes: inner.to_json().len() as u64,
            inner,
        })
    };
    let action = |s: &mut SessionCore, source: &str| {
        let draw = s
            .replace_program(&r, parsed(source), None, None, true)
            .unwrap()
            .evaluation;
        let action = s
            .action_at(&r, draw.evaluation, vec!["props".into(), "action".into()])
            .unwrap();
        s.run(&r, Some(action), String::new(), None).unwrap();
        s.take_query_update().calls.remove(0)
    };
    let first = action(
        &mut s,
        "m = Mutation(\"old\")\nroot = Box(m, Action([@Run(m)]))",
    );
    let second = action(
        &mut s,
        "m = Mutation(\"new\")\nroot = Box(m, Action([@Run(m)]))",
    );
    s.complete_tool_call_json(&second.id, r#"{"value":"saved"}"#)
        .unwrap();
    s.complete_tool_call_json(&first.id, &text_outcome(SIZE as usize + 1))
        .unwrap();
    let snapshot = snapshot(&mut s);
    assert_eq!(snapshot["m"]["status"], "error");
    assert_eq!(
        snapshot["m"]["error"],
        format!("tool result exceeds the result size limit of {SIZE} bytes")
    );
}

#[test]
fn a_removed_query_discards_even_malformed_outcome_json_once() {
    let (mut s, call) = session("a=Query(\"a\")\nb=Query(\"b\",{x:a})\nroot=Box(b)", None);
    let lib = library();
    let inner = Arc::new(openui_lang::parse("root=Box(1)", &lib.inner, None));
    let replacement = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    s.replace_program(&refs(), replacement, None, None, true)
        .unwrap();
    s.take_query_update();
    s.complete_tool_call_json(&call.id, "{").unwrap();
    let update = s.take_query_update();
    assert!(!update.changed && update.calls.is_empty());
    assert!(matches!(
        s.complete_tool_call_json(&call.id, "{"),
        Err(OpenUiError::InvalidToolCall)
    ));
}
