//! A program whose registration is refused by a limit while the manager's
//! registrations belong to another program (#119). Nothing of the other
//! program is drawn, read by an action, called or refreshed in that state.
use openui_lang::{JsObject, Library};
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

fn refs() -> PreparedReferenceValues<'static> {
    PreparedReferenceValues::owned_with_limits(JsObject::new(), openui_lang::Limits::DEFAULT)
}

fn program(lib: &ComponentLibrary, source: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(openui_lang::parse(source, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}

struct Fixture {
    lib: Arc<ComponentLibrary>,
    s: SessionCore,
    // Receipts handed to the driver and not yet completed.
    outstanding: Vec<Arc<WantedToolCall>>,
}

impl Fixture {
    /// A session with a 2,048-byte result limit and a tool provider.
    fn new() -> Self {
        let lib = library();
        let limits = RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(2048),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
        s.attach_tool_provider_at(&refs(), 0, 1000).unwrap();
        Fixture {
            lib,
            s,
            outstanding: Vec::new(),
        }
    }

    fn replace(&mut self, source: &str) -> EvaluationOutput {
        let p = program(&self.lib, source);
        self.s
            .replace_program(&refs(), p, None, None, false)
            .unwrap()
            .evaluation
    }

    fn evaluate(&mut self) -> EvaluationOutput {
        self.s.evaluate(&refs()).unwrap()
    }

    /// Drain the effects, keep the new receipts and check the manager.
    fn drain(&mut self) -> QueryEffects {
        let effects = self.s.take_query_update();
        self.outstanding.retain(|c| c.id.is_pending());
        self.outstanding.extend(effects.calls.iter().cloned());
        self.s
            .audit_query_cache(self.outstanding.iter().map(|c| &c.id))
            .unwrap();
        effects
    }

    fn finish(&mut self, tool: &str, outcome: Json) {
        let call = self
            .outstanding
            .iter()
            .find(|c| c.tool_name == tool && c.id.is_pending())
            .unwrap_or_else(|| panic!("no outstanding call for {tool}"))
            .clone();
        self.s
            .complete_tool_call_json(&call.id, &outcome.to_string())
            .unwrap();
    }

    fn click(&mut self, draw: &EvaluationOutput) -> ActionOutput {
        let action = self
            .s
            .action_at(
                &refs(),
                draw.evaluation.clone(),
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        self.s
            .run(&refs(), Some(action), String::new(), None)
            .unwrap()
    }

    /// Make the registered program's own re-registration refused, so a
    /// replacement whose arguments read `$n` is refused too.
    fn grow_state(&mut self) {
        let big = json!("x".repeat(1500)).to_string();
        self.s.set(&refs(), "$n".into(), Some(big)).unwrap();
        assert!(self.drain().evaluation_error.is_some());
    }
}

fn value(draw: &EvaluationOutput) -> Json {
    let tree: Json = serde_json::from_str(&draw.json).unwrap();
    tree["root"]["props"]["value"].clone()
}

fn snapshot(draw: &EvaluationOutput) -> Json {
    serde_json::from_str(draw.query_snapshot_json.as_deref().unwrap()).unwrap()
}

const UNREGISTERED: &str =
    r#"{"__openui_loading":[],"__openui_refetching":[],"__openui_errors":[]}"#;

#[test]
fn an_action_reads_the_new_queries_defaults_while_unregistered() {
    // B1 of the #119 review.
    let mut f = Fixture::new();
    f.replace("$n=\"a\"\nq=Query(\"search\",{term:$n})\nroot=Box(q)");
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    f.grow_state();
    let draw = f.replace(
        "q=Query(\"new\",{term:$n+$n},\"fresh\")\n$x=\"none\"\nroot=Box([q,$x],Action([@Set($x,q)]))",
    );
    assert_eq!(value(&draw), json!(["fresh", "none"]));
    let run = f.click(&draw);
    assert!(run.failure.is_none());
    assert_eq!(value(&f.evaluate()), json!(["fresh", "fresh"]));
}

#[test]
fn a_mutation_resumed_while_unregistered_reads_the_new_queries_defaults() {
    // B1: a mutation in flight across the refused replacement resumes its
    // plan against the new program, which reads no other program's data.
    let mut f = Fixture::new();
    let action = "Action([@Run(m),@Set($x,q)])";
    f.replace(&format!(
        "$n=\"a\"\n$x=\"none\"\nm=Mutation(\"save\")\nq=Query(\"search\",{{term:$n}})\nroot=Box([q,$x],{action})"
    ));
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    let draw = f.evaluate();
    f.click(&draw);
    assert_eq!(f.drain().calls[0].tool_name, "save");
    f.grow_state();
    f.replace(&format!(
        "q=Query(\"new\",{{term:$n+$n}},\"fresh\")\n$x=\"none\"\nm=Mutation(\"save\")\nroot=Box([q,$x],{action})"
    ));
    f.drain();
    f.finish("save", json!({"value":"saved"}));
    let resumed = f.drain();
    assert!(resumed.calls.is_empty());
    assert_eq!(value(&f.evaluate()), json!(["fresh", "fresh"]));
}

#[test]
fn a_mutation_of_the_other_program_draws_idle_and_is_not_fired() {
    // B2: `m` is in both programs with another tool. The new program's
    // button must not call the old program's tool.
    let mut f = Fixture::new();
    f.replace(
        "$n=\"a\"\nm=Mutation(\"save\")\nq=Query(\"search\",{term:$n})\nroot=Box([m.status,q],Action([@Run(m),@Set($n,\"after\")]))",
    );
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    let draw = f.evaluate();
    f.click(&draw);
    f.drain();
    f.finish("save", json!({"value":"saved"}));
    f.drain();
    f.grow_state();
    let draw = f.replace(
        "m=Mutation(\"delete\")\nq=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box([m.status,q],Action([@Run(m),@Set($n,\"after\")]))",
    );
    assert_eq!(value(&draw), json!(["idle", "fresh"]));
    let before = f.s.get(&refs(), "$n".into()).unwrap();
    let run = f.click(&draw);
    let failure = run.failure.expect("the mutation step fails for the limit");
    assert_eq!(failure.step, Some(0));
    assert!(matches!(failure.error, OpenUiError::LimitExceeded { .. }));
    assert!(run.size_limited && run.continuation.is_none());
    assert!(f.drain().calls.is_empty(), "no call to any tool");
    assert_eq!(
        f.s.get(&refs(), "$n".into()).unwrap(),
        before,
        "the rest of the plan did not run"
    );
}

#[test]
fn a_query_run_and_retry_start_no_call_while_unregistered() {
    // B3: `@Run(q)` and Retry would call the other program's tool.
    let mut f = Fixture::new();
    f.replace("$n=\"a\"\nq=Query(\"search\",{term:$n})\nroot=Box(q,Action([@Run(q)]))");
    f.drain();
    f.finish("search", json!({"failed":"offline"}));
    f.grow_state();
    let draw = f.replace("q=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box(q,Action([@Run(q)]))");
    // The snapshot describes the drawn program: nothing loading or held.
    assert_eq!(draw.query_snapshot_json.as_deref(), Some(UNREGISTERED));
    assert!(!draw.is_query_loading);
    assert!(matches!(
        draw.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    f.drain();
    f.click(&draw);
    assert!(f.drain().calls.is_empty(), "@Run started no call");
    f.s.retry_failed_queries().unwrap();
    assert!(f.drain().calls.is_empty(), "Retry started no call");
    let again = f.evaluate();
    assert_eq!(value(&again), json!("fresh"));
    assert_eq!(
        snapshot(&again),
        serde_json::from_str::<Json>(UNREGISTERED).unwrap()
    );
}

#[test]
fn the_other_programs_refreshes_do_not_fire_while_unregistered() {
    // B3: timers of the registered program.
    let mut f = Fixture::new();
    f.replace("$n=\"a\"\nq=Query(\"search\",{term:$n},null,5)\nroot=Box(q)");
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    assert_eq!(f.drain().next_refresh_due_ms, Some(5000));
    f.grow_state();
    f.replace("q=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box(q)");
    let effects = f.drain();
    assert_eq!(
        effects.next_refresh_due_ms, None,
        "no wake for another program"
    );
    f.s.advance_query_clock(20_000).unwrap();
    assert!(f.drain().calls.is_empty(), "no refresh call");
}

#[test]
fn a_late_result_of_the_other_program_starts_no_call_and_is_not_drawn() {
    // A call in flight for the registered program when the replacement is
    // refused. Its reader would start a follow-up call for that program.
    let mut f = Fixture::new();
    f.replace(
        "$n=\"a\"\nq=Query(\"search\",{term:$n})\nr=Query(\"follow\",{v:q})\nroot=Box([q,r])",
    );
    assert_eq!(f.drain().calls.len(), 1);
    f.grow_state();
    let draw = f.replace("q=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box([q,1])");
    assert_eq!(value(&draw), json!(["fresh", 1]));
    f.drain();
    f.finish("search", json!({"value":"late"}));
    let late = f.drain();
    assert!(late.calls.is_empty(), "no follow-up call");
    let draw = f.evaluate();
    assert_eq!(value(&draw), json!(["fresh", 1]));
    assert_eq!(draw.query_snapshot_json.as_deref(), Some(UNREGISTERED));
    // Leaving the state registers the new program; only its call goes out.
    f.s.set(&refs(), "$n".into(), Some(r#""b""#.into()))
        .unwrap();
    let registered = f.drain();
    assert!(registered.evaluation_error.is_none());
    let tools: Vec<_> = registered
        .calls
        .iter()
        .map(|c| c.tool_name.as_str())
        .collect();
    assert_eq!(tools, ["new"]);
}

#[test]
fn returning_to_the_registered_program_draws_its_own_data() {
    // Follow-up 1: P registered, Q refused, then P again (the same parse
    // result) with its re-registration still refused.
    let mut f = Fixture::new();
    let p = program(
        &f.lib,
        "$n=\"a\"\nq=Query(\"search\",{term:$n})\nroot=Box(q)",
    );
    f.s.replace_program(&refs(), p.clone(), None, None, false)
        .unwrap();
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    f.grow_state();
    let refused = f.replace("q=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box(q)");
    assert_eq!(value(&refused), json!("fresh"));
    let back =
        f.s.replace_program(&refs(), p, None, None, false)
            .unwrap()
            .evaluation;
    assert!(back.query_evaluation_error.is_some(), "still refused");
    assert_eq!(value(&back), json!("old data"));
    assert_eq!(snapshot(&back)["q"], "old data");
}

/// Replay a `conformance/session-mutations.json` scenario under a 2,048-byte
/// result limit, in that corpus's observation format, adding a failed
/// segment's step and reason.
fn replay_mutation_scenario(scenario: &Json) -> Vec<String> {
    let mut f = Fixture::new();
    let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
    let mut drawing: Option<EvaluationOutput> = None;
    let mut lines = Vec::new();
    for step in scenario["steps"].as_array().unwrap() {
        let mut segments = Vec::new();
        let draw = if let Some(source) = step["replace"].as_str() {
            Some(f.replace(source))
        } else {
            if let Some(name) = step["set"].as_str() {
                f.s.set(&refs(), name.into(), Some(step["value"].to_string()))
                    .unwrap();
            } else if step["click"] == true {
                segments.push(f.click(drawing.as_ref().unwrap()));
            } else {
                let call = &calls[step["finish"].as_u64().unwrap() as usize];
                let outcome = json!({"value":step["value"]});
                f.s.complete_tool_call_json(&call.id, &outcome.to_string())
                    .unwrap();
            }
            None
        };
        let update = f.drain();
        segments.extend(update.actions);
        let mut events = Vec::new();
        let mut failure = String::new();
        for segment in segments {
            if let Some(stopped) = segment.failure {
                let OpenUiError::LimitExceeded { reason } = stopped.error else {
                    panic!("expected the registration's limit")
                };
                failure = format!(
                    ",\"failure\":{}",
                    json!({"step":stopped.step,"error":reason,"sizeLimited":segment.size_limited})
                );
            }
            events.extend(segment.events);
        }
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
        let draw = draw.unwrap_or_else(|| f.evaluate());
        lines.push(format!(
            "{{\"tree\":{},\"snapshot\":{},\"events\":[{}],\"calls\":[{dispatch}]{failure},\"changed\":{}}}",
            draw.json,
            draw.query_snapshot_json.as_deref().unwrap(),
            events.join(","),
            update.changed
        ));
        drawing = Some(draw);
    }
    lines
}

#[test]
fn a_refused_replacement_neither_reads_nor_calls_the_old_program_where_upstream_registers_it() {
    // Both sides of `a-refused-replacement-neither-reads-nor-calls-the-old-program`
    // (CONFORMANCE.md, #119). Upstream has no limits and registers the new
    // program: its click calls `delete`. Refused here, the click reads the
    // new query's default, starts no call and ends at the mutation step.
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-mutations.json")).unwrap();
    let pin: Json = serde_json::from_str(include_str!(
        "../../../conformance/divergences/session-a-refused-replacement-neither-reads-nor-calls-the-old-program.json"
    ))
    .unwrap();
    let scenarios = corpus["scenarios"].as_array().unwrap();
    let index = scenarios
        .iter()
        .position(|s| s["name"] == pin["case"])
        .unwrap();
    let offset: usize = scenarios[..index]
        .iter()
        .map(|s| s["steps"].as_array().unwrap().len())
        .sum();
    let steps = scenarios[index]["steps"].as_array().unwrap().len();
    let golden: Vec<_> = include_str!("../../../conformance/golden/session-mutations.jsonl")
        .lines()
        .skip(offset)
        .take(steps)
        .map(Json::from)
        .collect();
    assert_eq!(pin["reference"], Json::Array(golden), "upstream side");
    let situation = pin["situation"].as_str().unwrap();
    assert!(include_str!("../../../CONFORMANCE.md").contains(&format!("| `{situation}` |")));
    let rust: Vec<_> = replay_mutation_scenario(&scenarios[index])
        .into_iter()
        .map(Json::from)
        .collect();
    assert_eq!(pin["rust"], Json::Array(rust), "Rust side");
    assert_ne!(pin["rust"], pin["reference"]);
}

/// Replay a `conformance/session-runs.json` scenario under a 2,048-byte
/// result limit, in that corpus's observation format. A completion of a call
/// only upstream made is skipped.
fn replay_run_scenario(scenario: &Json) -> Vec<String> {
    let mut f = Fixture::new();
    let mut calls: Vec<Arc<WantedToolCall>> = Vec::new();
    let mut lines = Vec::new();
    for step in scenario["steps"].as_array().unwrap() {
        let mut output = None;
        if let Some(source) = step["replace"].as_str() {
            output = Some(f.replace(source));
        } else if let Some(index) = step["finish"].as_u64() {
            if let Some(call) = calls.get(index as usize) {
                let outcome = if step.get("error").is_some() {
                    json!({"failed":step["error"]})
                } else {
                    json!({"value":step["value"]})
                };
                f.s.complete_tool_call_json(&call.id, &outcome.to_string())
                    .unwrap();
            }
        } else if let Some(name) = step["set"].as_str() {
            f.s.set(&refs(), name.into(), Some(step["value"].to_string()))
                .unwrap();
        } else {
            assert_eq!(step["retry"], true);
            f.s.retry_failed_queries().unwrap();
        }
        let effects = f.drain();
        let draw = output.unwrap_or_else(|| f.evaluate());
        let dispatch = effects
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
        lines.push(format!(
            "{{\"tree\":{},\"snapshot\":{},\"calls\":[{dispatch}],\"events\":[],\"changed\":{}}}",
            draw.json,
            draw.query_snapshot_json.as_deref().unwrap(),
            effects.changed
        ));
        calls.extend(effects.calls);
    }
    lines
}

#[test]
fn a_refused_replacement_retries_none_of_the_old_programs_queries_where_upstream_registers_it() {
    // Both sides of `a-refused-replacement-retries-none-of-the-old-programs-queries`
    // (CONFORMANCE.md, #119). Upstream registers the new program and Retry
    // calls its failed query; refused here, Retry starts no call.
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/session-runs.json")).unwrap();
    let pin: Json = serde_json::from_str(include_str!(
        "../../../conformance/divergences/session-a-refused-replacement-retries-none-of-the-old-programs-queries.json"
    ))
    .unwrap();
    let scenarios = corpus["scenarios"].as_array().unwrap();
    let index = scenarios
        .iter()
        .position(|s| s["name"] == pin["case"])
        .unwrap();
    let offset: usize = scenarios[..index]
        .iter()
        .map(|s| s["steps"].as_array().unwrap().len())
        .sum();
    let steps = scenarios[index]["steps"].as_array().unwrap().len();
    let golden: Vec<_> = include_str!("../../../conformance/golden/session-runs.jsonl")
        .lines()
        .skip(offset)
        .take(steps)
        .map(Json::from)
        .collect();
    assert_eq!(pin["reference"], Json::Array(golden), "upstream side");
    let situation = pin["situation"].as_str().unwrap();
    assert!(include_str!("../../../CONFORMANCE.md").contains(&format!("| `{situation}` |")));
    let rust: Vec<_> = replay_run_scenario(&scenarios[index])
        .into_iter()
        .map(Json::from)
        .collect();
    assert_eq!(pin["rust"], Json::Array(rust), "Rust side");
    assert_ne!(pin["rust"], pin["reference"]);
}

/// The reviewer's setup (#119 round 2): `q` refreshes, its late result
/// wakes `r`, whose call for the old program is held, then `b` registers.
fn held_reader_call(f: &mut Fixture, replacement: &str) {
    f.replace(
        "$n=\"a\"\nq=Query(\"search\",{term:$n},null,5)\nr=Query(\"follow\",{v:q})\nroot=Box([q,r])",
    );
    f.drain();
    f.finish("search", json!({"value":"v1"}));
    assert_eq!(f.drain().calls[0].tool_name, "follow");
    f.finish("follow", json!({"value":"r1"}));
    f.drain();
    f.s.advance_query_clock(6000).unwrap();
    assert_eq!(f.drain().calls[0].tool_name, "search");
    f.grow_state();
    f.replace(replacement);
    f.drain();
    f.finish("search", json!({"value":"v2"}));
    assert!(f.drain().calls.is_empty(), "follow{{v:v2}} is held");
}

#[test]
fn a_held_call_only_a_previous_key_wants_is_abandoned_when_another_program_registers() {
    let mut f = Fixture::new();
    held_reader_call(
        &mut f,
        "q=Query(\"new\",{term:$n+$n},\"fresh\")\nr=Query(\"other\",{v:1})\nroot=Box([q,r])",
    );
    f.s.set(&refs(), "$n".into(), Some(r#""b""#.into()))
        .unwrap();
    let registered = f.drain();
    let mut tools: Vec<_> = registered
        .calls
        .iter()
        .map(|c| c.tool_name.as_str())
        .collect();
    tools.sort_unstable();
    assert_eq!(
        tools,
        ["new", "other"],
        "the old program's follow call is not handed over"
    );
    // `q` shows its previous key's data while its new key loads, as any key
    // change does; `r` shows its own default until its own result.
    let draw = f.evaluate();
    assert_eq!(value(&draw), json!(["v2", null]));
    assert_eq!(snapshot(&draw)["__openui_loading"], json!(["q", "r"]));
    f.finish("other", json!({"value":"mine"}));
    f.drain();
    assert_eq!(value(&f.evaluate()), json!(["v2", "mine"]));
}

#[test]
fn a_held_call_the_new_program_wants_for_a_current_key_goes_out_once() {
    // The mirror case: `b`'s `r` wants follow{v:"v2"} too, for its current key.
    let mut f = Fixture::new();
    held_reader_call(
        &mut f,
        "q=Query(\"new\",{term:$n+$n},\"fresh\")\nr=Query(\"follow\",{v:\"v2\"})\nroot=Box([q,r])",
    );
    f.s.set(&refs(), "$n".into(), Some(r#""b""#.into()))
        .unwrap();
    let registered = f.drain();
    let follow: Vec<_> = registered
        .calls
        .iter()
        .filter(|c| c.tool_name == "follow")
        .collect();
    assert_eq!(follow.len(), 1, "one call, not two");
    assert!(follow[0].id.is_pending(), "not a retired receipt");
    assert_eq!(follow[0].arguments_json().as_deref(), Some(r#"{"v":"v2"}"#));
    f.finish("follow", json!({"value":"r2"}));
    f.drain();
    assert_eq!(value(&f.evaluate())[1], "r2");
}
