//! Compare the session's real action segments and single drain with the renderer.
use openui_lang::{JsObject, Library, Limits, parse};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn library() -> Arc<ComponentLibrary> {
    let schema = json!({"$defs":{"Box":{"properties":{"value":{},"action":{}}}}});
    Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
    })
}
fn program(lib: &ComponentLibrary, text: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(parse(text, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}
fn refs() -> PreparedReferenceValues<'static> {
    PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT)
}
fn click(
    s: &mut SessionCore,
    r: &PreparedReferenceValues<'_>,
    e: &EvaluationOutput,
) -> ActionOutput {
    let action = s
        .action_at(
            r,
            e.evaluation.clone(),
            vec!["props".into(), "action".into()],
        )
        .unwrap();
    s.run(r, Some(action), "click".into(), None).unwrap()
}

#[test]
fn settled_registration_clicks_and_resumed_segments_match_the_renderer() {
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-mutations.json")).unwrap();
    let mut reference = include_str!("../../../conformance/golden/session-mutations.jsonl").lines();
    let mut pins = [
        include_str!("../../../conformance/divergences/bounded-pending-mutation-plans.json"),
        include_str!("../../../conformance/divergences/numeric-mutation-tool-name.json"),
    ]
    .map(|text| serde_json::from_str::<Json>(text).unwrap())
    .to_vec();
    let lib = library();
    assert_eq!(*lib.schema, corpus["schema"]);
    let r = refs();
    for scenario in corpus["scenarios"].as_array().unwrap() {
        let mut limits = RuntimeLimits::default();
        let capacity_case = scenario["maxContinuations"].as_u64();
        if let Some(max) = capacity_case {
            limits.max_continuations = max;
        }
        let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
        s.attach_tool_provider(&r).unwrap();
        s.take_query_update();
        let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
        let mut drawing = None;
        let mut streaming = true;
        for (index, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
            let mut segments = Vec::new();
            if step.get("replace").is_some() || step.get("settle").is_some() {
                streaming = step["streaming"] == true;
            }
            let response = ResponseState { streaming };
            let draw = if let Some(source) = step["replace"].as_str() {
                Some(
                    s.replace_program_with_response(
                        &r,
                        program(&lib, source),
                        None,
                        None,
                        true,
                        response,
                    )
                    .unwrap()
                    .evaluation,
                )
            } else if step["settle"] == true {
                Some(s.evaluate_with_response(&r, response).unwrap())
            } else {
                if let Some(n) = step["set"].as_str() {
                    s.set(&r, n.into(), Some(step["value"].to_string()))
                        .unwrap();
                } else if step["click"] == true {
                    segments.push(click(&mut s, &r, drawing.as_ref().unwrap()));
                } else {
                    let call = &calls[step["finish"].as_u64().unwrap() as usize];
                    let o = if step.get("failed").is_some() {
                        json!({"failed":step["failed"]})
                    } else {
                        json!({"value":step["value"]})
                    };
                    s.complete_tool_call_json(&call.id, &o.to_string()).unwrap();
                }
                None
            };
            let update = s.take_query_update();
            assert!(update.evaluation_error.is_none());
            let redraw = s.take_query_update();
            assert!(redraw.calls.is_empty() && redraw.actions.is_empty() && !redraw.changed);
            if let Some(finish) = step["finish"].as_u64() {
                assert_eq!(
                    update.actions.len(),
                    usize::from(calls[finish as usize].tool_name != "read")
                );
            } else {
                assert!(update.actions.is_empty());
            }
            segments.extend(update.actions);
            let mut failure = Json::Null;
            let mut events = Vec::new();
            for segment in segments {
                assert!(segment.continuation.is_none());
                if let Some(stopped) = segment.failure {
                    assert!(capacity_case.is_some());
                    let OpenUiError::LimitExceeded { reason } = stopped.error else {
                        panic!("expected a capacity cut")
                    };
                    failure = json!({"step":stopped.step,"error":reason,"sizeLimited":segment.size_limited,"depthLimited":segment.depth_limited});
                } else {
                    assert!(!segment.depth_limited && !segment.size_limited);
                }
                events.extend(segment.events);
            }
            let events = events.join(",");
            let failure = capacity_case
                .map(|_| format!(",\"failure\":{failure}"))
                .unwrap_or_default();
            let dispatch = update
                .calls
                .iter()
                .map(|c| {
                    let args = c
                        .arguments_json()
                        .map(|v| format!(",\"args\":{v}"))
                        .unwrap_or_default();
                    format!("{{\"toolName\":{}{args}}}", json!(c.tool_name))
                })
                .collect::<Vec<_>>()
                .join(",");
            calls.extend(update.calls);
            let draw = draw.unwrap_or_else(|| s.evaluate_with_response(&r, response).unwrap());
            let actual = format!(
                "{{\"tree\":{},\"snapshot\":{},\"events\":[{events}],\"calls\":[{dispatch}]{failure},\"changed\":{}}}",
                draw.json,
                draw.query_snapshot_json.as_deref().unwrap(),
                update.changed
            );
            let upstream = reference.next().unwrap();
            if let Some(position) = pins.iter().position(|pin| {
                pin["case"] == scenario["name"] && pin["step"].as_u64() == Some(index as u64)
            }) {
                let pin = pins.remove(position);
                assert_eq!(pin["reference"], json!([upstream]));
                assert_eq!(pin["rust"], json!([actual]));
                assert_ne!(actual, upstream, "{} has converged", scenario["name"]);
            } else {
                assert_eq!(actual, upstream, "{}, step {index}", scenario["name"]);
            }
            drawing = Some(draw);
        }
    }
    assert!(pins.is_empty(), "unused mutation pins: {pins:?}");
    assert!(reference.next().is_none());
}

#[test]
fn a_refused_draw_or_outcome_keeps_the_call_and_detaching_releases_its_plan() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let p = program(
        &lib,
        "m=Mutation(\"old\")\nroot=Box(m,Action([@Run(m),@Set($done,true)]))",
    );
    let out = s.replace_program(&r, p, None, None, true).unwrap();
    click(&mut s, &r, &out.evaluation);
    let call = s.take_query_update().calls.remove(0);
    let bad = program(
        &lib,
        "m=Mutation(\"new\")\n$shape={type:\"element\",typeName:\"Box\",props:{},partial:false,hasDynamicProps:false,extra:1}\nroot=true ? $shape : Box()",
    );
    assert!(s.replace_program(&r, bad, None, None, true).is_err());
    assert!(s.complete_tool_call_json(&call.id, "{}").is_err());
    assert_eq!(s.pending_continuations(&r).unwrap(), 1);
    let still = s.evaluate(&r).unwrap();
    assert!(still.query_snapshot_json.unwrap().contains("\"loading\""));
    assert!(!s.take_query_update().changed);
    s.detach_tool_provider();
    assert_eq!(s.pending_continuations(&r).unwrap(), 0);
    s.attach_tool_provider(&r).unwrap();
    assert!(matches!(
        s.complete_tool_call_json(&call.id, "{\"value\":1}"),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(s.take_query_update().actions.is_empty());
    assert_eq!(s.get(&r, "$done".into()).unwrap().as_deref(), Some("null"));
}

#[test]
fn pending_plan_limit_stops_only_the_new_submission_and_resumed_failures_keep_earlier_events() {
    let lib = library();
    let r = refs();
    let mut limits = RuntimeLimits {
        max_continuations: 1,
        ..Default::default()
    };
    limits.evaluation.result_size = Some(2048);
    let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let plan =
        "\nroot=Box(m,Action([@Run(m),@OpenUrl(\"one\"),@Set($x,$big+$big),@OpenUrl(\"two\")]))";
    let first = s
        .replace_program(
            &r,
            program(&lib, &("m=Mutation(\"a\")".to_owned() + plan)),
            None,
            None,
            true,
        )
        .unwrap();
    click(&mut s, &r, &first.evaluation);
    let old = s.take_query_update().calls.remove(0);
    let second = s
        .replace_program(
            &r,
            program(&lib, &("m=Mutation(\"b\")".to_owned() + plan)),
            None,
            None,
            true,
        )
        .unwrap();
    let failed = click(&mut s, &r, &second.evaluation);
    assert_eq!(failed.failure.unwrap().step, Some(0));
    assert!(failed.size_limited);
    assert!(s.take_query_update().calls.is_empty());
    // The first resumed event fits. Its following write exceeds the action's result room.
    s.set(&r, "$big".into(), Some(json!("x".repeat(1100)).to_string()))
        .unwrap();
    s.complete_tool_call_json(&old.id, r#"{"value":3}"#)
        .unwrap();
    let effects = s.take_query_update();
    let action = effects.actions.first().unwrap();
    assert_eq!(action.events.len(), 1);
    assert_eq!(action.failure.as_ref().unwrap().step, Some(2));
    assert!(action.size_limited);
    assert_eq!(s.get(&r, "$x".into()).unwrap().as_deref(), Some("null"));
    assert!(s.take_query_update().actions.is_empty());
}

#[test]
fn reserved_mutation_redraws_and_two_resumes_drain_in_completion_order() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    s.attach_tool_provider(&r).unwrap();
    s.take_query_update();
    let text = "__openui_loading=Mutation(\"old\")\nm=Mutation(\"other\")\nroot=Box(__openui_loading,Action([@Run(__openui_loading),@OpenUrl(\"first\")]))";
    let first = s
        .replace_program(&r, program(&lib, text), None, None, true)
        .unwrap()
        .evaluation;
    let snapshot = first.query_snapshot_json.clone();
    s.take_query_update();
    click(&mut s, &r, &first);
    let mut dispatch = s.take_query_update();
    assert!(dispatch.changed);
    let old = dispatch.calls.remove(0);
    let next = text
        .replace("@Run(__openui_loading)", "@Run(m)")
        .replace("first", "second");
    let second = s
        .replace_program(&r, program(&lib, &next), None, None, true)
        .unwrap()
        .evaluation;
    assert_eq!(second.query_snapshot_json, snapshot);
    assert!(second.json.contains(r#""status":"loading""#));
    click(&mut s, &r, &second);
    let other = s.take_query_update().calls.remove(0);
    s.complete_tool_call_json(&other.id, r#"{"value":2}"#)
        .unwrap();
    s.complete_tool_call_json(&old.id, r#"{"value":1}"#)
        .unwrap();
    s.detach_tool_provider();
    let batch = s.take_query_update();
    assert_eq!(batch.actions.len(), 2);
    assert!(batch.actions[0].events[0].contains("second"));
    assert!(batch.actions[1].events[0].contains("first"));
    assert!(s.take_query_update().actions.is_empty());
}

#[test]
fn a_non_string_run_target_ends_the_plan_like_an_unknown_mutation() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let source = r#"m=Mutation("save")
$plan={steps:[{type:"run",refType:"mutation",statementId:123},{type:"open_url",url:"later"}]}
root=Box(m,$plan)"#;
    let draw = s
        .replace_program(&r, program(&lib, source), None, None, true)
        .unwrap()
        .evaluation;
    s.take_query_update();
    let run = click(&mut s, &r, &draw);
    assert!(run.events.is_empty() && run.continuation.is_none() && run.failure.is_none());
    assert!(s.take_query_update().calls.is_empty());
}

#[test]
fn reporting_one_cancelled_call_as_failed_frees_one_of_64_pending_slots() {
    let lib = library();
    let r = refs();
    let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    s.attach_tool_provider(&r).unwrap();
    let mut source = String::new();
    let mut buttons = Vec::new();
    for i in 0..=64 {
        source.push_str(&format!("m{i}=Mutation(\"save\")\n"));
        buttons.push(format!("Action([@Run(m{i}),@OpenUrl(\"done\")])"));
    }
    source.push_str(&format!("root=Box(null,[{}])", buttons.join(",")));
    let tree = s
        .replace_program(&r, program(&lib, &source), None, None, true)
        .unwrap()
        .evaluation;
    let submit = |s: &mut SessionCore, i: usize| {
        let action = s
            .action_at(
                &r,
                tree.evaluation.clone(),
                vec!["props".into(), "action".into(), i.to_string()],
            )
            .unwrap();
        s.run(&r, Some(action), "click".into(), None).unwrap()
    };
    for i in 0..64 {
        assert!(submit(&mut s, i).failure.is_none());
    }
    let mut wanted = s.take_query_update();
    assert_eq!(wanted.calls.len(), 64);
    let cancelled = wanted.calls.pop().unwrap();
    drop(wanted);
    let refused = submit(&mut s, 64);
    assert!(refused.size_limited);
    assert!(matches!(
        refused.failure.unwrap().error,
        OpenUiError::LimitExceeded { .. }
    ));
    assert!(s.take_query_update().calls.is_empty());

    s.complete_tool_call_json(&cancelled.id, r#"{"failed":"cancelled by provider"}"#)
        .unwrap();
    let update = s.take_query_update();
    assert!(update.changed && update.calls.is_empty());
    assert_eq!(update.actions.len(), 1);
    assert!(update.actions[0].events.is_empty() && update.actions[0].failure.is_none());
    assert_eq!(s.pending_continuations(&r).unwrap(), 63);
    assert!(matches!(
        s.complete_tool_call_json(&cancelled.id, r#"{"value":1}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    assert!(submit(&mut s, 64).failure.is_none());
    assert_eq!(s.take_query_update().calls.len(), 1);
    let snapshot: Json =
        serde_json::from_str(&s.evaluate(&r).unwrap().query_snapshot_json.unwrap()).unwrap();
    assert_eq!(
        snapshot["m63"],
        json!({"status":"error","error":"cancelled by provider"})
    );
    assert_eq!(snapshot["__openui_errors"][0]["code"], "tool-error");
    assert_eq!(snapshot["m64"]["status"], "loading");
    assert_eq!(s.pending_continuations(&r).unwrap(), 64);
    s.detach_tool_provider();
    assert_eq!(s.pending_continuations(&r).unwrap(), 0);
}
