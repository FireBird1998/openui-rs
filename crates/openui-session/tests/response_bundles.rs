//! Draw the bundle reader's program and compare each public observation with React's effects.
use openui_lang::{JsObject, Library, Limits, StreamParser, parse_response_bundle};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use serde_json::{Value as Json, json};
use std::{error::Error, sync::Arc};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn library(schema: Json) -> Arc<ComponentLibrary> {
    Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
        defined: None,
    })
}

fn replay(name: &str) -> Result<()> {
    replay_corpus(
        include_str!("../../../conformance/session-bundles.json"),
        include_str!("../../../conformance/golden/session-bundles.jsonl"),
        name,
    )
    .map(drop)
}

/// The session after a replay, with the last response it was given and the
/// reference's `blocked` after the last step.
struct Replayed {
    session: SessionCore,
    lib: Arc<ComponentLibrary>,
    response: ResponseState,
    blocked: bool,
}

/// Replay one scenario. A golden row with `blocked` (the renderer's
/// `isStreaming` for its components) is compared with the draw's flag too.
fn replay_corpus(corpus: &str, golden: &str, name: &str) -> Result<Replayed> {
    let corpus: Json = serde_json::from_str(corpus)?;
    let lib = library(corpus.get("schema").ok_or("missing schema")?.clone());
    let scenarios = corpus
        .get("scenarios")
        .and_then(Json::as_array)
        .ok_or("missing scenarios")?;
    let mut golden = golden.split_inclusive('\n');
    let mut selected = None;
    for scenario in scenarios {
        let steps = scenario
            .get("steps")
            .and_then(Json::as_array)
            .ok_or("missing steps")?;
        let rows: Vec<_> = golden.by_ref().take(steps.len()).collect();
        assert_eq!(rows.len(), steps.len(), "missing golden observations");
        if scenario.get("name").and_then(Json::as_str) == Some(name) {
            selected = Some((steps, rows));
        }
    }
    assert!(golden.next().is_none(), "extra golden observations");
    let (steps, expected) = selected.ok_or("missing scenario")?;
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut s = SessionCore::new(lib.clone(), None, Default::default())?;
    let mut parser = StreamParser::new(lib.inner.clone(), None);
    s.attach_tool_provider(&refs)?;
    let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
    let mut response = ResponseState::default();
    let mut previous: Option<EvaluationOutput> = None;
    let mut blocked = false;
    for (index, (step, expected)) in steps.iter().zip(expected).enumerate() {
        let mut draw = None;
        let mut events = Vec::new();
        if let Some(raw) = step.get("response").and_then(Json::as_str) {
            let streaming = step.get("streaming") == Some(&Json::Bool(true));
            let bundle = parse_response_bundle(Some(raw), streaming, Limits::DEFAULT)?;
            response = ResponseState::from_bundle(streaming, &bundle);
            if step.get("evaluate") == Some(&Json::Bool(true)) {
                draw = Some(s.evaluate_with_response(&refs, response.clone())?);
            } else {
                let partial = parser.set(&bundle.program);
                let inner = Arc::new(if streaming { partial } else { parser.finish() });
                let program = Arc::new(ParsedProgram {
                    json_bytes: inner.to_json().len() as u64,
                    inner,
                });
                draw = Some(
                    s.replace_program_with_response(
                        &refs,
                        program,
                        None,
                        None,
                        false,
                        response.clone(),
                    )?
                    .evaluation,
                );
            }
        } else if let Some(tool) = step.get("extra").and_then(Json::as_str) {
            s.set_query_extra_values(
                tool,
                &step
                    .get("values")
                    .ok_or("missing extra values")?
                    .to_string(),
            )?;
        } else if let Some(id) = step.get("finish").and_then(Json::as_u64) {
            let call = calls.get(usize::try_from(id)?).ok_or("missing call")?;
            let outcome = if let Some(error) = step.get("error") {
                json!({"failed":error})
            } else {
                json!({"value":step.get("value")})
            };
            s.complete_tool_call_json(&call.id, &outcome.to_string())?;
        } else if let Some(prop) = step.get("click").and_then(Json::as_str) {
            let run = match prop {
                "default" => s.run(&refs, None, "click".into(), None)?,
                "legacy" => {
                    s.run_legacy(&refs, r#"{"type":"custom"}"#.into(), "click".into(), None)?
                }
                _ => {
                    let handle = &previous.as_ref().ok_or("missing draw")?.evaluation;
                    let action =
                        s.action_at(&refs, handle.clone(), vec!["props".into(), prop.into()])?;
                    s.run(&refs, Some(action), "click".into(), None)?
                }
            };
            assert!(run.failure.is_none());
            events.extend(run.events);
        } else if let Some(key) = step.get("set").and_then(Json::as_str) {
            s.set(&refs, key.into(), step.get("value").map(Json::to_string))?;
        } else {
            assert_eq!(step.get("retry"), Some(&Json::Bool(true)));
            s.retry_failed_queries()?;
        }
        let draw = match draw {
            Some(draw) => draw,
            None => s.evaluate_with_response(&refs, response.clone())?,
        };
        let effects = s.take_query_update();
        assert!(effects.evaluation_error.is_none());
        for run in effects.actions {
            assert!(run.failure.is_none());
            events.extend(run.events);
        }
        let mut dispatch = Vec::new();
        for call in &effects.calls {
            let mut record = format!(r#"{{"toolName":{}"#, json!(call.tool_name));
            if let Some(args) = call.arguments_json() {
                record.push_str(&format!(r#","args":{args}"#));
            }
            record.push('}');
            dispatch.push(record);
        }
        calls.extend(effects.calls);
        let expected_blocked = serde_json::from_str::<Json>(expected)?
            .get("blocked")
            .and_then(Json::as_bool);
        let mut actual = format!(
            r#"{{"name":{},"tree":{},"snapshot":{},{}"isQueryLoading":{},"parserError":{},"calls":[{}],"events":[{}]}}"#,
            json!(name),
            draw.json,
            draw.query_snapshot_json
                .as_deref()
                .ok_or("missing snapshot")?,
            expected_blocked.map_or(String::new(), |_| format!(r#""blocked":{},"#, draw.blocked)),
            draw.is_query_loading,
            draw.parser_error_json.as_deref().unwrap_or("null"),
            dispatch.join(","),
            events.join(",")
        );
        actual.push('\n');
        assert_eq!(actual, expected, "{name} step {index}");
        blocked = draw.blocked;
        previous = Some(draw);
    }
    Ok(Replayed {
        session: s,
        lib,
        response,
        blocked,
    })
}

#[test]
fn blocked_bundles_draw_and_report_loading_or_a_parse_error_then_settle() -> Result<()> {
    replay("bundle-lifecycle")
}

#[test]
fn only_queries_calling_settled_scripts_refetch_and_app_values_join_the_key() -> Result<()> {
    replay("script-cache-keys")
}

#[test]
fn a_streaming_bare_edit_runs_queries_retry_and_the_last_registered_mutation() -> Result<()> {
    replay("bare-streaming-edit")
}

#[test]
fn refused_response_metadata_preserves_the_display_and_click_gate() -> Result<()> {
    let lib = library(json!({"$defs":{"Box":{"properties":{"action":{}}}}}));
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut session = SessionCore::new(
        lib.clone(),
        None,
        RuntimeLimits {
            max_json_bytes: 2048,
            ..Default::default()
        },
    )?;
    let inner = Arc::new(openui_lang::parse(
        "root=Box(Action([@ToAssistant(\"visible\")]))",
        &lib.inner,
        None,
    ));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    let draw = session
        .replace_program(&refs, program.clone(), None, None, true)?
        .evaluation;
    let handle = session.action_at(
        &refs,
        draw.evaluation,
        vec!["props".into(), "action".into()],
    )?;
    for state in [
        ResponseState {
            scripts: vec![String::new(); 2049],
            ..Default::default()
        },
        ResponseState {
            scripts: vec!["a".repeat(1024)],
            script_revision: Some("b".repeat(1024)),
            ..Default::default()
        },
        ResponseState {
            error: Some("x".repeat(2049)),
            ..Default::default()
        },
        // Bytes, not characters: 1,024 of them under the limit, 2,049 bytes over it.
        ResponseState {
            scripts: vec!["é".repeat(1024)],
            ..Default::default()
        },
    ] {
        let blocked = ResponseState {
            is_bundle: true,
            streaming: true,
            ..state
        };
        assert!(matches!(
            session.evaluate_with_response(&refs, blocked.clone()),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert!(matches!(
            session.replace_program_with_response(
                &refs,
                program.clone(),
                None,
                None,
                true,
                blocked
            ),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert_eq!(
            session
                .run(&refs, Some(handle.clone()), String::new(), None)?
                .events
                .len(),
            1
        );
    }
    Ok(())
}

#[test]
fn refused_diagnostics_and_script_keys_leave_the_previous_calls_usable() -> Result<()> {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut limits = RuntimeLimits::default();
    limits.evaluation.result_size = Some(2048);
    let mut session = SessionCore::new(lib.clone(), None, limits)?;
    let inner = Arc::new(openui_lang::parse(
        "q=Query(\"cloud\",{},0)\nroot=Box(q)",
        &lib.inner,
        None,
    ));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    session.attach_tool_provider(&refs)?;
    session.replace_program(&refs, program, None, None, true)?;
    let pending = session
        .take_query_update()
        .calls
        .pop()
        .ok_or("missing first call")?;
    let invalid = ResponseState {
        is_bundle: true,
        error: Some("\n".repeat(2048)),
        ..Default::default()
    };
    assert!(matches!(
        session.evaluate_with_response(&refs, invalid),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    let long_revision = ResponseState {
        is_bundle: true,
        scripts: vec!["cloud".into()],
        script_revision: Some("X".repeat(32_768)),
        ..Default::default()
    };
    let rejected = session.evaluate_with_response(&refs, long_revision)?;
    assert!(matches!(
        rejected.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert!(session.take_query_update().calls.is_empty());
    session.complete_tool_call_json(&pending.id, r#"{"value":42}"#)?;
    let recovered = session.evaluate(&refs)?;
    assert!(recovered.query_evaluation_error.is_none());
    assert!(recovered.json.contains(r#""value":42"#));
    Ok(())
}

#[test]
fn changing_only_query_keys_preserves_the_registered_mutation_tool() -> Result<()> {
    replay("query-key-only-edit")
}

fn parsed(lib: &ComponentLibrary, source: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(openui_lang::parse(source, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}

fn revision(name: &str) -> ResponseState {
    ResponseState {
        is_bundle: true,
        scripts: vec!["cloud".into()],
        script_revision: Some(name.into()),
        ..Default::default()
    }
}

#[test]
fn a_refused_script_revision_is_retried_and_store_writes_use_the_last_admitted_revision()
-> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}},
        "functions":{"Tool":{"params":{"type":"object","properties":{}}}}}));
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut limits = RuntimeLimits::default();
    limits.evaluation.result_size = Some(2048);
    let mut session = SessionCore::new(lib.clone(), None, limits)?;
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    session.register_function(
        "Tool".into(),
        Arc::new(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
            Ok(Some(r#""cloud""#.into()))
        }),
    )?;
    session.attach_tool_provider(&refs)?;
    session.replace_program_with_response(
        &refs,
        parsed(&lib, "q=Query(@Tool(),{},0)\nroot=Box(q)"),
        None,
        None,
        true,
        revision("A"),
    )?;
    let original = session
        .take_query_update()
        .calls
        .pop()
        .ok_or("missing call")?;
    let oversized = revision(&"B".repeat(32_768));
    for _ in 0..2 {
        let before = attempts.load(Ordering::Relaxed);
        let draw = session.evaluate_with_response(&refs, oversized.clone())?;
        assert!(matches!(
            draw.query_evaluation_error,
            Some(OpenUiError::LimitExceeded { .. })
        ));
        assert!(
            attempts.load(Ordering::Relaxed) > before,
            "same revision must be evaluated again"
        );
        assert!(session.take_query_update().calls.is_empty());
    }
    session.set(&refs, "$n".into(), Some("1".into()))?;
    let write = session.take_query_update();
    assert!(
        write.evaluation_error.is_none(),
        "store writes must use the admitted key"
    );
    assert!(write.calls.is_empty());
    session.complete_tool_call_json(&original.id, r#"{"value":42}"#)?;
    let draw = session.evaluate_with_response(&refs, revision("A"))?;
    assert!(draw.json.contains(r#""value":42"#));
    Ok(())
}

#[test]
fn a_result_refused_while_the_bundle_is_blocked_ends_its_call_and_waits_for_retry() -> Result<()> {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut limits = RuntimeLimits::default();
    limits.evaluation.result_size = Some(1024);
    let mut session = SessionCore::new(lib.clone(), None, limits)?;
    session.attach_tool_provider(&refs)?;
    session.replace_program_with_response(
        &refs,
        parsed(&lib, "q=Query(\"cloud\",{},0)\nroot=Box(q)"),
        None,
        None,
        true,
        revision("A"),
    )?;
    let call = session
        .take_query_update()
        .calls
        .pop()
        .ok_or("missing call")?;
    let blocked = ResponseState {
        is_bundle: true,
        complete: false,
        ..Default::default()
    };
    session.evaluate_with_response(&refs, blocked.clone())?;
    session.complete_tool_call_json(&call.id, &json!({"value":"x".repeat(1024)}).to_string())?;
    assert!(!call.id.is_pending());
    let draw = session.evaluate_with_response(&refs, blocked)?;
    let snap: Json = serde_json::from_str(&draw.query_snapshot_json.ok_or("missing snapshot")?)?;
    assert_eq!(
        snap.pointer("/__openui_errors/0/code"),
        Some(&json!("tool-error"))
    );
    assert_eq!(
        snap.pointer("/__openui_errors/0/message"),
        Some(&json!(
            "Query \"cloud\" failed: tool result exceeds the result size limit of 1024 bytes"
        ))
    );
    assert!(!draw.is_query_loading);
    session.retry_failed_queries()?;
    assert!(session.take_query_update().calls.is_empty());
    session.evaluate_with_response(&refs, revision("A"))?;
    session.retry_failed_queries()?;
    assert_eq!(session.take_query_update().calls.len(), 1);
    Ok(())
}

#[test]
fn changing_a_script_key_releases_its_previous_cache_charge_on_success() -> Result<()> {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut session = SessionCore::new(
        lib.clone(),
        None,
        RuntimeLimits {
            max_cache_bytes: Some(10),
            ..Default::default()
        },
    )?;
    session.attach_tool_provider(&refs)?;
    session.replace_program_with_response(
        &refs,
        parsed(
            &lib,
            "q=Query(\"cloud\",{},0)\nr=Query(\"ordinary\",{},0)\nroot=Box([q,r])",
        ),
        None,
        None,
        true,
        revision("A"),
    )?;
    let calls = session.take_query_update().calls;
    let cloud = calls
        .iter()
        .find(|c| c.tool_name == "cloud")
        .ok_or("missing Cloud script call")?;
    let ordinary = calls
        .iter()
        .find(|c| c.tool_name == "ordinary")
        .ok_or("missing ordinary call")?;
    session.complete_tool_call_json(&cloud.id, r#"{"value":"123456"}"#)?;
    session.evaluate_with_response(&refs, revision("B"))?;
    let b = session
        .take_query_update()
        .calls
        .pop()
        .ok_or("missing revised script call")?;
    session.complete_tool_call_json(&ordinary.id, r#"{"value":100}"#)?;
    let draw = session.evaluate_with_response(&refs, revision("B"))?;
    let snap: Json = serde_json::from_str(&draw.query_snapshot_json.ok_or("missing snapshot")?)?;
    assert_eq!(snap.get("q"), Some(&json!("123456")));
    assert_eq!(
        snap.pointer("/__openui_errors/0/message"),
        Some(&json!(
            "Query \"ordinary\" failed: tool result does not fit the cache limit of 10 bytes"
        ))
    );
    session.complete_tool_call_json(&b.id, r#"{"value":"abcdefg"}"#)?;
    session.retry_failed_queries()?;
    let retry = session
        .take_query_update()
        .calls
        .pop()
        .ok_or("missing ordinary retry")?;
    session.complete_tool_call_json(&retry.id, r#"{"value":1}"#)?;
    let draw = session.evaluate_with_response(&refs, revision("B"))?;
    let snap: Json = serde_json::from_str(&draw.query_snapshot_json.ok_or("missing snapshot")?)?;
    assert_eq!(snap.get("q"), Some(&json!("abcdefg")));
    assert_eq!(snap.get("r"), Some(&json!(1)));
    assert_eq!(snap.get("__openui_errors"), Some(&json!([])));
    Ok(())
}

#[test]
fn source_revision_with_a_reader() -> Result<()> {
    replay("source-revision-with-a-reader")
}

#[test]
fn settled_reader_completes_during_a_blocked_edit() -> Result<()> {
    replay("settled-reader-completes-during-a-blocked-edit")
}

/// The draw's `blocked` after each command the session takes, compared with
/// the renderer's `blocked` where the reference has the command (the replay)
/// and, for commands of the session alone, with what the last response says.
#[test]
fn every_draw_reports_the_renderers_blocked_flag_and_the_gate_is_unchanged() -> Result<()> {
    let Replayed {
        session: mut s,
        lib,
        response: settled,
        blocked,
    } = replay_corpus(
        include_str!("../../../conformance/session-blocked.json"),
        include_str!("../../../conformance/golden/session-blocked.jsonl"),
        "blocked-flag-walk",
    )?;
    assert!(!blocked && settled.is_bundle && !settled.streaming);
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let program = parsed(&lib, "root=Box(\"x\",Action([@ToAssistant(\"x\")]))");
    let streaming = ResponseState {
        streaming: true,
        ..settled.clone()
    };
    let invalid = ResponseState {
        error: Some("Invalid scripts bundle".into()),
        ..settled.clone()
    };
    let refused = ResponseState {
        error: Some("x".repeat(9 << 20)),
        ..streaming.clone()
    };
    let clicks = |s: &mut SessionCore| -> Result<usize> {
        Ok(s.run(&refs, None, "click".into(), None)?.events.len())
    };
    // A bundle's draw, then a refused update: the gate is as the last draw left it.
    for (state, flag) in [(&settled, false), (&streaming, true), (&invalid, true)] {
        assert_eq!(
            s.evaluate_with_response(&refs, state.clone())?.blocked,
            flag
        );
        assert!(matches!(
            s.evaluate_with_response(&refs, refused.clone()),
            Err(OpenUiError::LimitExceeded { .. })
        ));
        assert_eq!(clicks(&mut s)?, usize::from(!flag));
        assert_eq!(
            s.evaluate_with_response(&refs, state.clone())?.blocked,
            flag
        );
    }
    // Pausing and resuming queries changes nothing the draw reports.
    s.pause_queries()?;
    assert!(s.evaluate_with_response(&refs, invalid.clone())?.blocked);
    s.resume_queries()?;
    assert!(s.evaluate_with_response(&refs, invalid.clone())?.blocked);
    // Program replacement takes the response it is given.
    let replaced = s.replace_program(&refs, program.clone(), None, None, true)?;
    assert!(!replaced.evaluation.blocked);
    let replaced = s.replace_program_with_response(
        &refs,
        program.clone(),
        None,
        None,
        true,
        streaming.clone(),
    )?;
    assert!(replaced.evaluation.blocked);
    assert_eq!(clicks(&mut s)?, 0);
    // setProgram keeps the gate until the next draw, which reports its own state.
    s.set_program(&refs, program.clone())?;
    assert_eq!(clicks(&mut s)?, 0);
    assert!(!s.evaluate(&refs)?.blocked);
    assert_eq!(clicks(&mut s)?, 1);
    // A bare program streaming is blocked, as upstream's components see it,
    // yet the session still takes its click: the flag only reports.
    let bare = ResponseState::from(true);
    assert!(s.evaluate_with_response(&refs, bare)?.blocked);
    assert_eq!(clicks(&mut s)?, 1);
    assert!(
        !s.evaluate_with_response(&refs, ResponseState::default())?
            .blocked
    );
    Ok(())
}
