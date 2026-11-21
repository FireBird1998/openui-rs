//! A `@Run` of a query and Retry on a session with a tool provider, compared
//! with the renderer's own action handling and retryQueries.
use openui_lang::{JsObject, Library};
use openui_runtime::PreparedReferenceValues;
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
fn no_refs() -> PreparedReferenceValues<'static> {
    PreparedReferenceValues::owned_with_limits(JsObject::new(), openui_lang::Limits::DEFAULT)
}
fn session(lib: &Arc<ComponentLibrary>) -> SessionCore {
    SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap()
}
fn dispatch(effects: &QueryEffects) -> String {
    effects
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
        .join(",")
}
fn click(s: &mut SessionCore, draw: &EvaluationOutput, prop: &str) -> ActionOutput {
    let refs = no_refs();
    let action = s
        .action_at(
            &refs,
            draw.evaluation.clone(),
            vec!["props".into(), prop.into()],
        )
        .unwrap();
    s.run(&refs, Some(action), "clicked".into(), None).unwrap()
}

#[test]
fn query_runs_and_retry_match_the_renderer_bytes() {
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-runs.json")).unwrap();
    let lib = library(corpus["schema"].clone());
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut actual = String::new();
    for scenario in corpus["scenarios"].as_array().unwrap() {
        let mut s = session(&lib);
        s.attach_tool_provider(&refs).unwrap();
        s.take_query_update();
        let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
        let mut response = ResponseState::default();
        let mut last: Option<EvaluationOutput> = None;
        for step in scenario["steps"].as_array().unwrap() {
            let mut output = None;
            let mut events = Vec::new();
            if step.get("evaluate").is_some() || step.get("replace").is_some() {
                response = ResponseState {
                    streaming: step["streaming"] == true,
                    is_bundle: scenario["bundle"] == true,
                    complete: step["complete"] != false,
                    error: step["error"].as_str().map(str::to_owned),
                    ..Default::default()
                };
            }
            if let Some(source) = step["replace"].as_str() {
                let replaced = s
                    .replace_program_with_response(
                        &refs,
                        program(&lib, source),
                        None,
                        None,
                        false,
                        response.clone(),
                    )
                    .unwrap();
                output = Some(replaced.evaluation);
            } else if step["evaluate"] == true {
                output = Some(s.evaluate_with_response(&refs, response.clone()).unwrap());
            } else if let Some(index) = step["finish"].as_u64() {
                let outcome = if step.get("error").is_some() {
                    json!({"failed":step["error"]})
                } else {
                    json!({"value":step["value"]})
                };
                s.complete_tool_call_json(&calls[index as usize].id, &outcome.to_string())
                    .unwrap();
            } else if let Some(prop) = step["click"].as_str() {
                let run = click(&mut s, last.as_ref().unwrap(), prop);
                assert!(run.failure.is_none() && run.continuation.is_none());
                events = run.events;
            } else if let Some(name) = step["set"].as_str() {
                s.set(&refs, name.into(), Some(step["value"].to_string()))
                    .unwrap();
            } else {
                assert_eq!(step["retry"], true);
                s.retry_failed_queries().unwrap();
            }
            let effects = s.take_query_update();
            assert!(effects.evaluation_error.is_none());
            let draw = output.unwrap_or_else(|| {
                let draw = s.evaluate_with_response(&refs, response.clone()).unwrap();
                let update = s.take_query_update();
                assert!(update.calls.is_empty() && !update.changed);
                draw
            });
            actual.push_str(&format!(
                "{{\"tree\":{},\"snapshot\":{},\"calls\":[{}],\"events\":[{}],\"changed\":{}}}\n",
                draw.json,
                draw.query_snapshot_json.clone().unwrap(),
                dispatch(&effects),
                events.join(","),
                effects.changed
            ));
            calls.extend(effects.calls);
            last = Some(draw);
        }
    }
    let expected = include_str!("../../../conformance/golden/session-runs.jsonl");
    for (index, (actual, expected)) in actual.lines().zip(expected.lines()).enumerate() {
        assert_eq!(actual, expected, "observation {index}");
    }
    assert_eq!(actual.lines().count(), expected.lines().count());
}

const REFRESH: &str = "q = Query(\"search\", {}, \"loading\")\nroot = Box(Action([@Run(q), @ToAssistant(\"after\")]))";

#[test]
fn without_a_provider_a_query_run_is_still_the_hosts_event() {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = no_refs();
    let mut s = session(&lib);
    let draw = s
        .replace_program(&refs, program(&lib, REFRESH), None, None, false)
        .unwrap()
        .evaluation;
    let run = click(&mut s, &draw, "value");
    assert_eq!(run.events.len(), 2);
    let event: Json = serde_json::from_str(&run.events[0]).unwrap();
    assert_eq!(
        event,
        json!({"type":"run","params":{"statementId":"q","refType":"query"},"humanFriendlyMessage":"","formState":{}})
    );
    assert!(s.take_query_update().calls.is_empty());
    // Retry has no provider to call.
    s.retry_failed_queries().unwrap();
    let update = s.take_query_update();
    assert!(update.calls.is_empty() && !update.changed);
}

#[test]
fn while_streaming_a_run_acts_on_the_queries_registered_so_far() {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = no_refs();
    let mut s = session(&lib);
    s.attach_tool_provider(&refs).unwrap();
    s.replace_program(&refs, program(&lib, REFRESH), None, None, false)
        .unwrap();
    let first = s.take_query_update().calls.remove(0);
    s.complete_tool_call_json(&first.id, r#"{"failed":"offline"}"#)
        .unwrap();
    s.take_query_update();
    // A streamed edit declares p and runs it, but p is not registered yet.
    let streamed = "q = Query(\"search\", {}, \"loading\")\np = Query(\"other\", {}, \"p\")\nroot = Box(Action([@Run(p), @Run(q), @ToAssistant(\"after\")]))";
    let draw = s
        .replace_program_with_response(
            &refs,
            program(&lib, streamed),
            None,
            None,
            false,
            ResponseState::from(true),
        )
        .unwrap()
        .evaluation;
    assert!(s.take_query_update().calls.is_empty());
    let run = click(&mut s, &draw, "value");
    assert_eq!(run.events.len(), 1, "only the message step emits");
    let update = s.take_query_update();
    assert_eq!(update.calls.len(), 1);
    assert_eq!(update.calls[0].tool_name, "search");
    s.complete_tool_call_json(&update.calls[0].id, r#"{"failed":"again"}"#)
        .unwrap();
    s.take_query_update();
    s.retry_failed_queries().unwrap();
    let retried = s.take_query_update();
    assert_eq!(retried.calls.len(), 1);
    assert_eq!(retried.calls[0].tool_name, "search");
}

#[test]
fn detaching_forgets_a_remembered_run() {
    let lib = library(json!({"$defs":{"Box":{"properties":{"value":{}}}}}));
    let refs = no_refs();
    let mut s = session(&lib);
    s.attach_tool_provider(&refs).unwrap();
    let draw = s
        .replace_program(&refs, program(&lib, REFRESH), None, None, false)
        .unwrap()
        .evaluation;
    let first = s.take_query_update().calls.remove(0);
    click(&mut s, &draw, "value");
    assert!(s.take_query_update().calls.is_empty());
    s.detach_tool_provider();
    s.attach_tool_provider(&refs).unwrap();
    assert!(matches!(
        s.complete_tool_call_json(&first.id, r#"{"value":1}"#),
        Err(OpenUiError::InvalidToolCall)
    ));
    s.evaluate(&refs).unwrap();
    let fresh = s.take_query_update().calls.remove(0);
    s.complete_tool_call_json(&fresh.id, r#"{"value":1}"#)
        .unwrap();
    assert!(s.take_query_update().calls.is_empty());
}
