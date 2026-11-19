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
    })
}

fn replay(name: &str) -> Result<()> {
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-bundles.json"))?;
    let lib = library(corpus.get("schema").ok_or("missing schema")?.clone());
    let scenarios = corpus
        .get("scenarios")
        .and_then(Json::as_array)
        .ok_or("missing scenarios")?;
    let mut golden =
        include_str!("../../../conformance/golden/session-bundles.jsonl").split_inclusive('\n');
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
        let mut actual = format!(
            r#"{{"name":{},"tree":{},"snapshot":{},"isQueryLoading":{},"parserError":{},"calls":[{}],"events":[{}]}}"#,
            json!(name),
            draw.json,
            draw.query_snapshot_json
                .as_deref()
                .ok_or("missing snapshot")?,
            draw.is_query_loading,
            draw.parser_error_json.as_deref().unwrap_or("null"),
            dispatch.join(","),
            events.join(",")
        );
        actual.push('\n');
        assert_eq!(actual, expected, "{name} step {index}");
        previous = Some(draw);
    }
    Ok(())
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
