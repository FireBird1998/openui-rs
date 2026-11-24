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
        defined: None,
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
        Self::with_result_size(2048)
    }

    fn with_result_size(bytes: u64) -> Self {
        let lib = library();
        let limits = RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(bytes),
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
        self.grow_state_to(1500);
    }

    fn grow_state_to(&mut self, bytes: usize) {
        let big = json!("x".repeat(bytes)).to_string();
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
fn a_query_run_while_unregistered_is_skipped_and_the_plan_carries_on() {
    // One rule for a step that cannot run (ADR 0013, #117): a mutation step
    // ends its plan with a failure; a query step starts no call and the
    // plan goes on, as upstream's `invalidate` does when it cannot fetch.
    let mut f = Fixture::new();
    f.replace("$n=\"a\"\nq=Query(\"search\",{term:$n})\nroot=Box(q)");
    f.drain();
    f.finish("search", json!({"value":"old data"}));
    f.grow_state();
    let draw = f.replace(
        "q=Query(\"new\",{term:$n+$n},\"fresh\")\n$x=\"none\"\nroot=Box([q,$x],Action([@Run(q),@Set($x,\"after\"),@OpenUrl(\"next\")]))",
    );
    f.drain();
    let run = f.click(&draw);
    assert!(run.failure.is_none() && !run.size_limited);
    assert_eq!(run.events.len(), 1, "the step after the query run ran");
    assert!(f.drain().calls.is_empty(), "@Run started no call");
    assert_eq!(value(&f.evaluate()), json!(["fresh", "after"]));
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

#[test]
fn a_reader_whose_call_a_pause_dropped_calls_again_when_its_own_re_registration_is_refused() {
    // #125 item 2 (#142). `r`'s call is in flight when queries pause, which
    // releases it; resume re-registers the same program, the limit refuses
    // that, and nothing asked for `r`'s call again: `r` stayed idle with no
    // result, error or call. The registrations still belong to the drawn
    // program, so it keeps its own results and asks for the calls they owe.
    let mut f = Fixture::new();
    f.replace(
        "$n=\"a\"\nq=Query(\"search\",{term:$n})\nr=Query(\"follow\",{v:q})\nroot=Box([q,r])",
    );
    f.drain();
    f.grow_state();
    f.finish("search", json!({"value":"v1"}));
    assert_eq!(f.drain().calls[0].tool_name, "follow");
    f.s.pause_queries().unwrap();
    f.drain();
    f.s.resume_queries().unwrap();
    let resumed = f.drain();
    assert!(
        matches!(
            resumed.evaluation_error,
            Some(OpenUiError::LimitExceeded { .. })
        ),
        "the re-registration is still refused"
    );
    let calls: Vec<_> = resumed
        .calls
        .iter()
        .map(|c| (c.tool_name.as_str(), c.arguments_json()))
        .collect();
    assert_eq!(calls, [("follow", Some(r#"{"v":"v1"}"#.to_owned()))]);
    let draw = f.evaluate();
    assert_eq!(value(&draw), json!(["v1", null]));
    assert_eq!(snapshot(&draw)["__openui_loading"], json!(["r"]));
    assert!(f.drain().calls.is_empty(), "asked for once");
    f.finish("follow", json!({"value":"r1"}));
    f.drain();
    assert_eq!(value(&f.evaluate()), json!(["v1", "r1"]));
}

#[test]
fn a_refused_resume_restarts_refreshing_the_accepted_query() {
    replay_refused_refresh(
        include_str!("../../../conformance/refused-refresh-resume.json"),
        include_str!("../../../conformance/golden/refused-refresh-resume.jsonl"),
        6,
        ResumeRefusal::Limit,
    );
}

#[test]
fn a_deferred_refusal_sets_the_clock_for_a_reader_its_owed_result_wakes() {
    replay_refused_refresh(
        include_str!("../../../conformance/refused-refresh-deferred.json"),
        include_str!("../../../conformance/golden/refused-refresh-deferred.jsonl"),
        6,
        ResumeRefusal::Limit,
    );
}

#[test]
fn a_tool_name_coercion_error_keeps_the_earlier_refresh_timer_on_resume() {
    replay_refused_refresh(
        include_str!("../../../conformance/refused-refresh-tool-name.json"),
        include_str!("../../../conformance/golden/refused-refresh-tool-name.jsonl"),
        5,
        ResumeRefusal::ToolName,
    );
}

#[test]
fn a_tool_name_coercion_error_leaves_no_later_refresh_timer_on_resume() {
    replay_refused_refresh(
        include_str!("../../../conformance/refused-refresh-tool-name-first.json"),
        include_str!("../../../conformance/golden/refused-refresh-tool-name-first.jsonl"),
        5,
        ResumeRefusal::ToolName,
    );
}

enum ResumeRefusal {
    Limit,
    ToolName,
}

fn replay_refused_refresh(fixture: &str, reference: &str, refusal: usize, kind: ResumeRefusal) {
    let fixture: Json = serde_json::from_str(fixture).unwrap();
    let reference: Vec<Json> = reference
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut f = Fixture::new();
    let mut pending = std::collections::HashMap::new();
    let mut observed = Vec::new();
    let mut response = ResponseState::default();
    let mut streaming = false;
    let steps = fixture["steps"].as_array().unwrap();
    assert_eq!(steps.len(), reference.len());
    let diagnostic = match kind {
        ResumeRefusal::Limit => {
            assert!(reference[refusal]["registrationError"].is_null());
            "work or size limit exceeded"
        }
        ResumeRefusal::ToolName => {
            assert_eq!(
                reference[refusal]["registrationError"],
                "TypeError: No default value"
            );
            "query evaluation failed: CannotConvertToPrimitive"
        }
    };
    for (index, step) in steps.iter().enumerate() {
        let mut clock_rejected = None;
        let mut explicit_draw = None;
        let time = || step["now"].as_u64().unwrap();
        match step["kind"].as_str().unwrap() {
            "register" => {
                explicit_draw = Some(f.replace(fixture["program"].as_str().unwrap()));
            }
            "pause" => f.s.pause_queries().unwrap(),
            "set" => {
                let input = match step["bytes"].as_u64() {
                    Some(bytes) => json!("x".repeat(bytes as usize)),
                    None => step["value"].clone(),
                };
                let name = step["name"].as_str().unwrap_or("$detail");
                assert!(
                    f.s.set(&refs(), name.into(), Some(input.to_string()))
                        .unwrap()
                );
            }
            "resume" => {
                if step["streaming"] == true {
                    streaming = true;
                    response = ResponseState::from(true);
                    f.s.evaluate_with_response(&refs(), response.clone())
                        .unwrap();
                }
                f.s.resume_queries_at(time()).unwrap();
            }
            "settle" => {
                streaming = false;
                response = ResponseState::default();
                explicit_draw = Some(
                    f.s.draw_response_at(&refs(), None, response.clone(), Some(time()))
                        .unwrap(),
                );
            }
            "advance" => f.s.advance_query_clock(time()).unwrap(),
            "backwards" => {
                clock_rejected = Some(match f.s.advance_query_clock(time()) {
                    Ok(()) => false,
                    Err(OpenUiError::LimitExceeded { reason }) => {
                        assert_eq!(reason, "invalid refresh minimum or clock time");
                        true
                    }
                    Err(error) => panic!("unexpected clock error: {error}"),
                });
            }
            "finish" => {
                let tool = step["tool"].as_str().unwrap_or("poll");
                let dispatch = step["dispatch"].as_u64().unwrap() as usize;
                if let Some(call) = pending.remove(&(dispatch, tool.to_owned())) {
                    let outcome = match step.get("error") {
                        Some(error) => json!({"failed": error}),
                        None => json!({"value": step["value"]}),
                    };
                    f.s.complete_tool_call_json(&call, &outcome.to_string())
                        .unwrap();
                }
            }
            kind => panic!("unknown replay step {kind}"),
        }

        // Owed calls remain idle while a resumed response is still streaming.
        let drain = |f: &mut Fixture| {
            if streaming {
                f.s.take_query_update()
            } else {
                f.drain()
            }
        };
        let effects = drain(&mut f);
        let draw = explicit_draw.unwrap_or_else(|| {
            f.s.evaluate_with_response(&refs(), response.clone())
                .unwrap()
        });
        let after_draw = drain(&mut f);
        assert!(after_draw.calls.is_empty());
        assert!(!after_draw.changed);
        assert_eq!(after_draw.next_refresh_due_ms, effects.next_refresh_due_ms);
        assert!(effects.actions.is_empty());
        assert!(!effects.reader_rounds_limited);
        assert_eq!(draw.query_evaluation_error, effects.evaluation_error);
        if index >= refusal {
            assert!(matches!(
                (&kind, &effects.evaluation_error),
                (
                    ResumeRefusal::Limit,
                    Some(OpenUiError::LimitExceeded { .. })
                ) | (
                    ResumeRefusal::ToolName,
                    Some(OpenUiError::Evaluation { .. })
                )
            ));
        }
        let calls: Vec<_> = effects
            .calls
            .iter()
            .map(|call| {
                let args: Json = serde_json::from_str(&call.arguments_json().unwrap()).unwrap();
                pending.insert((index, call.tool_name.clone()), call.id.clone());
                json!({"toolName": call.tool_name, "args": args})
            })
            .collect();
        let tree: Json = serde_json::from_str(&draw.json).unwrap();
        let mut row = json!({
            "calls": calls,
            "nextDue": effects.next_refresh_due_ms,
            "snapshot": snapshot(&draw),
            "registrationError": effects.evaluation_error.map(|error| error.to_string()),
            "changed": effects.changed,
            "value": value(&draw),
            "errors": tree["errors"],
        });
        if let Some(rejected) = clock_rejected {
            row["clockRejected"] = json!(rejected);
        }
        observed.push(row);
    }

    // Print the entire run even when a missed deadline prevents later calls.
    for row in &observed {
        eprintln!("{row}");
    }
    for (index, (actual, mut expected)) in observed.into_iter().zip(reference).enumerate() {
        // The port retains the diagnostic in the draw. Upstream has no result
        // cap and exposes the cache-key exception only as a thrown error.
        if index >= refusal {
            expected["registrationError"] = json!(diagnostic);
        }
        if index == refusal {
            expected["changed"] = json!(true);
        }
        if steps[index]["kind"] == "pause" && expected["snapshot"]["__openui_loading"] != json!([])
        {
            // Rust clears loading during pause. Upstream publishes it when
            // the disposed source's old promise settles on the next row.
            // Recorded in CONFORMANCE.md, row "Dispose while a query call or
            // mutation is loading" (the pause case, #235). This adjusts only
            // the loading list and changed flag at a pause; don't widen it.
            expected["snapshot"]["__openui_loading"] = json!([]);
            expected["changed"] = json!(true);
        }
        if index > 0 && steps[index - 1]["kind"] == "pause" && steps[index]["kind"] == "finish" {
            expected["changed"] = json!(false);
        }
        assert_eq!(actual, expected, "step {index}: {}", steps[index]);
    }
}

#[test]
fn a_query_expression_error_before_application_recovers_no_refresh_timer() {
    for statements in [
        "bad=Query(\"\"+$tool,{},null)",
        "bad=Query($tool,{},null)\nargs=Query(\"args\",{v:\"\"+$tool},null)",
    ] {
        let mut f = Fixture::new();
        f.replace(&format!(
            "$tool=\"other\"\nq=Query(\"poll\",{{id:7}},null,2)\n{statements}\nroot=Box(q)"
        ));
        for call in f.drain().calls {
            f.s.complete_tool_call_json(&call.id, r#"{"value":"initial"}"#)
                .unwrap();
        }
        f.drain();
        f.s.pause_queries().unwrap();
        f.s.set(&refs(), "$tool".into(), Some(r#"{"toString":null}"#.into()))
            .unwrap();

        f.s.resume_queries_at(5000).unwrap();
        let resumed = f.drain();
        assert!(matches!(
            resumed.evaluation_error,
            Some(OpenUiError::Evaluation { .. })
        ));
        assert_eq!(resumed.next_refresh_due_ms, None, "{statements}");
        f.s.advance_query_clock(7000).unwrap();
        assert!(f.drain().calls.is_empty());
        assert_eq!(value(&f.evaluate()), "initial");
    }
}

#[test]
fn a_refused_settled_draw_restarts_only_the_accepted_refresh_intervals() {
    let mut f = Fixture::new();
    f.replace(concat!(
        "$detail=\"short\"\n$period=2\n",
        "q=Query(\"poll\",{},$detail+$detail,$period)\n",
        "source=Query(\"source\")\nreader=Query(\"reader\",{v:source},null,1)\n",
        "root=Box(q)"
    ));
    f.drain();
    f.finish("poll", json!({"value":"initial"}));
    assert_eq!(f.drain().next_refresh_due_ms, Some(2000));

    f.s.pause_queries().unwrap();
    assert_eq!(f.drain().next_refresh_due_ms, None);
    let streaming = ResponseState::from(true);
    f.s.evaluate_with_response(&refs(), streaming).unwrap();
    f.s.set(&refs(), "$period".into(), Some("1".into()))
        .unwrap();
    let detail = json!("x".repeat(1500)).to_string();
    f.s.set(&refs(), "$detail".into(), Some(detail)).unwrap();
    f.s.resume_queries_at(5000).unwrap();
    let active = f.s.take_query_update();
    assert!(active.calls.is_empty());
    assert_eq!(active.next_refresh_due_ms, None);

    let draw =
        f.s.draw_response_at(&refs(), None, ResponseState::default(), Some(6000))
            .unwrap();
    assert!(matches!(
        draw.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(value(&draw), "initial");
    let effects = f.drain();
    let [source] = effects.calls.as_slice() else {
        panic!("expected the call owed to the source");
    };
    assert_eq!(source.tool_name, "source");
    // No interval was installed for the waiting reader or for the refused period.
    assert_eq!(effects.next_refresh_due_ms, Some(8000));
    f.s.advance_query_clock(7999).unwrap();
    assert!(f.drain().calls.is_empty());
    f.s.advance_query_clock(8000).unwrap();
    let tick = f.drain();
    let [poll] = tick.calls.as_slice() else {
        panic!("expected the cached query's refresh call");
    };
    assert_eq!(poll.tool_name, "poll");
    assert_eq!(tick.next_refresh_due_ms, Some(10_000));
}

#[test]
fn a_refused_replacement_on_deferred_resume_recovers_no_old_timer() {
    let mut f = Fixture::new();
    f.replace(concat!(
        "$detail=\"short\"\nq=Query(\"poll\",{id:7},null,2)\n",
        "removed=Query(\"removed\",{},null,3)\nroot=Box([q,removed])"
    ));
    f.drain();
    f.finish("poll", json!({"value":"initial"}));
    f.finish("removed", json!({"value":"old value"}));
    f.drain();
    f.s.pause_queries().unwrap();
    f.s.evaluate_with_response(&refs(), ResponseState::from(true))
        .unwrap();
    f.s.set(
        &refs(),
        "$detail".into(),
        Some(json!("x".repeat(1500)).to_string()),
    )
    .unwrap();
    f.s.resume_queries_at(5000).unwrap();

    let replacement = program(
        &f.lib,
        "$detail=\"short\"\nq=Query(\"new\",{v:$detail+$detail},\"fresh\",1)\nroot=Box(q)",
    );
    let draw =
        f.s.draw_response_at(
            &refs(),
            Some(replacement),
            ResponseState::default(),
            Some(6000),
        )
        .unwrap();
    assert!(matches!(
        draw.query_evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(value(&draw), "fresh");
    assert_eq!(draw.query_snapshot_json.as_deref(), Some(UNREGISTERED));
    f.s.advance_query_clock(10_000).unwrap();
    let effects = f.drain();
    assert_eq!(effects.next_refresh_due_ms, None);
    assert!(effects.calls.is_empty());
}

#[test]
fn detaching_the_provider_during_deferred_resume_ends_refreshes() {
    let mut f = Fixture::new();
    f.replace("$detail=\"short\"\nq=Query(\"poll\",{id:7},$detail+$detail,2)\nroot=Box(q)");
    f.drain();
    f.finish("poll", json!({"value":"initial"}));
    assert_eq!(f.drain().next_refresh_due_ms, Some(2000));
    f.s.pause_queries().unwrap();
    f.s.evaluate_with_response(&refs(), ResponseState::from(true))
        .unwrap();
    f.s.set(
        &refs(),
        "$detail".into(),
        Some(json!("x".repeat(1500)).to_string()),
    )
    .unwrap();
    f.s.resume_queries_at(5000).unwrap();

    f.s.detach_tool_provider();
    let draw =
        f.s.draw_response_at(&refs(), None, ResponseState::default(), Some(6000))
            .unwrap();
    assert!(draw.query_snapshot_json.is_none());
    f.s.advance_query_clock(10_000).unwrap();
    let effects = f.drain();
    assert_eq!(effects.next_refresh_due_ms, None);
    assert!(effects.calls.is_empty());
}

#[test]
fn a_late_result_of_the_other_program_reports_no_redraw_or_round_cut() {
    // #125 item 7 (#142). A NaN result makes `r`'s input differ on every
    // round, so its cascade stops at the round bound and changes the other
    // program's snapshot. Neither is the drawn program's.
    let mut f = Fixture::new();
    f.replace(
        "$n=\"a\"\nq=Query(\"search\",{term:$n})\nr=Query(\"follow\",{v:q})\nroot=Box([q,r])",
    );
    f.drain();
    f.grow_state();
    f.replace("q=Query(\"new\",{term:$n+$n},\"fresh\")\nroot=Box([q,1])");
    f.drain();
    let search = f.outstanding[0].clone();
    assert_eq!(search.tool_name, "search");
    f.s.complete_tool_call(
        &search.id,
        openui_runtime::ToolCallOutcome::Result(openui_runtime::Value::Num(f64::NAN)),
    )
    .unwrap();
    let late = f.drain();
    assert!(late.calls.is_empty(), "no follow-up call");
    assert!(!late.changed, "nothing the host draws changed");
    assert!(
        !late.reader_rounds_limited,
        "the cut is the other program's"
    );
    let draw = f.evaluate();
    assert_eq!(value(&draw), json!(["fresh", 1]));
    assert_eq!(draw.query_snapshot_json.as_deref(), Some(UNREGISTERED));
}

/// Allocation requests of each command on the unregistered path (#125 item
/// 10, #142), for programs of `n` queries: the refused replacement, a late
/// result whose `n` readers' calls are held, a draw, a drain, and the
/// registration that leaves the state and ends the held calls.
fn unregistered_costs(n: usize) -> [usize; 5] {
    let mut f = Fixture::with_result_size(65_536);
    let readers: String = (0..n)
        .map(|i| format!("r{i}=Query(\"follow\",{{v:q,i:{i}}})\n"))
        .collect();
    f.replace(&format!(
        "$n=\"a\"\nq=Query(\"search\",{{term:$n}})\n{readers}root=Box(q)"
    ));
    let first = f.drain();
    assert!(
        first.evaluation_error.is_none(),
        "{:?}",
        first.evaluation_error
    );
    f.grow_state_to(48_000);
    let others: String = (0..n)
        .map(|i| format!("s{i}=Query(\"other\",{{i:{i}}})\n"))
        .collect();
    let replacement = format!("{others}q=Query(\"new\",{{term:$n+$n}},\"fresh\")\nroot=Box(q)");
    let p = program(&f.lib, &replacement);
    let mut costs = [0; 5];
    let before = test_alloc::allocation_calls();
    let draw = f.s.replace_program(&refs(), p, None, None, false).unwrap();
    costs[0] = test_alloc::allocation_calls() - before;
    assert_eq!(
        draw.evaluation.query_snapshot_json.as_deref(),
        Some(UNREGISTERED)
    );
    f.drain();
    let search = f.outstanding[0].clone();
    let before = test_alloc::allocation_calls();
    f.s.complete_tool_call_json(&search.id, r#"{"value":"late"}"#)
        .unwrap();
    costs[1] = test_alloc::allocation_calls() - before;
    let before = test_alloc::allocation_calls();
    let draw = f.s.evaluate(&refs()).unwrap();
    costs[2] = test_alloc::allocation_calls() - before;
    assert_eq!(value(&draw), json!("fresh"));
    let before = test_alloc::allocation_calls();
    let held = f.s.take_query_update();
    costs[3] = test_alloc::allocation_calls() - before;
    assert!(held.calls.is_empty() && !held.changed);
    let before = test_alloc::allocation_calls();
    f.s.set(&refs(), "$n".into(), Some(r#""b""#.into()))
        .unwrap();
    costs[4] = test_alloc::allocation_calls() - before;
    let registered = f.drain();
    assert!(registered.evaluation_error.is_none());
    assert_eq!(
        registered.calls.len(),
        n + 1,
        "the new program's calls only"
    );
    costs
}

#[test]
fn each_command_on_the_unregistered_path_costs_in_proportion_to_the_program() {
    let small = unregistered_costs(100);
    let large = unregistered_costs(200);
    eprintln!("allocation requests at 100 and 200 queries: {small:?} {large:?}");
    for (name, (small, large)) in [
        "refused replacement",
        "late result",
        "draw",
        "drain",
        "registration",
    ]
    .into_iter()
    .zip(small.into_iter().zip(large))
    {
        assert!(
            large <= small * 5 / 2 + 64,
            "{name}: {small} allocation requests at 100 queries, {large} at 200"
        );
    }
}

#[test]
fn reread_registrations_keep_data_but_a_query_or_mutation_edit_draws_defaults() {
    let mut f = Fixture::new();
    let source =
        "$tool=\"search\"\nq=Query($tool,{},\"default\")\nm=Mutation(\"save\")\nroot=Box(q)";
    f.replace(source);
    f.drain();
    f.finish("search", json!({"value": "held"}));
    f.s.set(&refs(), "$tool".into(), Some(r#"{"toString":null}"#.into()))
        .unwrap();

    for (candidate, expected) in [
        (source.to_string(), "held"),
        (source.replace("save", "delete"), "default"),
        (source.replace("default", "edited"), "edited"),
        (source.replace("root=Box(q)", "root=Box(q,1)"), "held"),
    ] {
        let draw = f.replace(&candidate);
        assert!(matches!(
            draw.query_evaluation_error,
            Some(OpenUiError::Evaluation { .. })
        ));
        assert_eq!(value(&draw), expected);
        assert_eq!(value(&f.evaluate()), expected);
    }
    f.replace(&source.replace("save", "delete"));
    f.s.set_program(&refs(), program(&f.lib, source)).unwrap();
    assert_eq!(value(&f.evaluate()), "held");
}

const MUTATION_FROM_DEFAULT: &str = "$t=\"save\"\n$q=\"search\"\nq=Query($q,{},\"default\")\nm=Mutation($t)\nroot=Box([$t,q],Action([@Run(m)]))";

fn registered_mutation_with_refused_query() -> Fixture {
    let mut f = Fixture::new();
    f.replace(MUTATION_FROM_DEFAULT);
    f.drain();
    f.finish("search", json!({"value":"held"}));
    f.s.set(&refs(), "$q".into(), Some(r#"{"toString":null}"#.into()))
        .unwrap();
    assert!(matches!(
        f.drain().evaluation_error,
        Some(OpenUiError::Evaluation { .. })
    ));
    f
}

#[test]
fn a_changed_state_default_draws_fresh_data_and_cannot_fire_the_previous_tool() {
    let mut f = registered_mutation_with_refused_query();
    let draw = f.replace(&MUTATION_FROM_DEFAULT.replace("save", "delete"));
    assert!(matches!(
        draw.query_evaluation_error,
        Some(OpenUiError::Evaluation { .. })
    ));
    assert_eq!(value(&draw), json!(["delete", "default"]));
    let failure = f.click(&draw).failure.expect("unregistered mutation");
    assert_eq!(failure.step, Some(0));
    assert_eq!(Some(failure.error), draw.query_evaluation_error);
    assert!(f.drain().calls.is_empty());
}

#[test]
fn identical_state_defaults_keep_held_data_after_a_refused_reread() {
    let mut f = registered_mutation_with_refused_query();
    let draw = f.replace(MUTATION_FROM_DEFAULT);
    assert!(matches!(
        draw.query_evaluation_error,
        Some(OpenUiError::Evaluation { .. })
    ));
    assert_eq!(value(&draw), json!(["save", "held"]));
    assert!(f.click(&draw).failure.is_none());
    let effects = f.drain();
    let [call] = effects.calls.as_slice() else {
        panic!("expected one registered mutation call")
    };
    assert_eq!(call.tool_name, "save");
}

#[test]
fn rereads_compare_nested_object_order_and_signed_zero_in_registration_inputs() {
    let mut f = Fixture::new();
    let source = "$tool=\"search\"\n$args=[{a:1,b:-0}]\nq=Query($tool,{},\"default\")\nm=Mutation(\"save\",[$args,-0])\nroot=Box(q)";
    f.replace(source);
    f.drain();
    f.finish("search", json!({"value":"held"}));
    f.s.set(&refs(), "$tool".into(), Some(r#"{"toString":null}"#.into()))
        .unwrap();

    for (candidate, expected) in [
        (source.replace("a:1,b:-0", "b:-0,a:1"), "default"),
        (source.replace("b:-0", "b:0"), "default"),
        (source.replace("$args,-0", "$args,0"), "default"),
        (source.to_owned(), "held"),
    ] {
        let draw = f.replace(&candidate);
        assert!(matches!(
            draw.query_evaluation_error,
            Some(OpenUiError::Evaluation { .. })
        ));
        assert_eq!(value(&draw), expected, "{candidate}");
    }
}

#[test]
fn a_held_mutation_from_a_click_goes_out_once_after_another_program_registers() {
    let mut f = Fixture::new();
    let draw = f.replace("m=Mutation(\"save\",{id:7})\nroot=Box(m.status,Action([@Run(m)]))");
    f.drain();
    f.click(&draw);
    let refused = f.replace("q=Query({toString:null})\nroot=Box(q)");
    assert!(refused.query_evaluation_error.is_some());
    assert!(f.drain().calls.is_empty());
    f.replace("m=Mutation(\"delete\")\nroot=Box(m.status)");
    let update = f.drain();
    let [call] = update.calls.as_slice() else {
        panic!("one held mutation")
    };
    assert_eq!(call.tool_name, "save");
    assert_eq!(call.arguments_json().as_deref(), Some(r#"{"id":7}"#));
    assert!(f.drain().calls.is_empty());
}

#[test]
fn an_unregistered_mutation_reports_the_conversion_error_without_a_size_cut() {
    let mut f = Fixture::new();
    let draw =
        f.replace("q=Query({toString:null})\nm=Mutation(\"save\")\nroot=Box(q,Action([@Run(m)]))");
    let run = f.click(&draw);
    let failure = run.failure.unwrap();
    assert_eq!(failure.step, Some(0));
    assert_eq!(Some(failure.error), draw.query_evaluation_error);
    assert!(!run.size_limited);
    assert!(f.drain().calls.is_empty());
}

#[test]
fn mutation_clicked_after_refused_state_write_registration_matches_the_renderer() {
    let golden: Json = serde_json::from_str(include_str!(
        "../../../conformance/golden/mutation-after-state-write.json"
    ))
    .unwrap();
    let mut f = Fixture::new();
    let source = include_str!("../../../conformance/mutation-after-state-write.oui");
    let p = program(&f.lib, source);
    let persisted = json!({"$detail": "x".repeat(1200)}).to_string();
    let before =
        f.s.replace_program(&refs(), p, Some(persisted), None, false)
            .unwrap()
            .evaluation;
    assert!(before.query_evaluation_error.is_none());
    let initial = f.drain();
    assert_eq!(initial.calls.len(), 1);
    assert_eq!(initial.calls[0].tool_name, "read");

    assert!(
        f.s.set(&refs(), "$t".into(), Some(r#""delete""#.into()))
            .unwrap()
    );
    let write = f.drain();
    assert!(matches!(
        write.evaluation_error,
        Some(OpenUiError::LimitExceeded { .. })
    ));
    assert!(write.calls.is_empty());
    let after = f.evaluate();
    assert_eq!(after.query_evaluation_error, write.evaluation_error);
    assert!(f.drain().calls.is_empty());

    assert!(f.click(&after).failure.is_none());
    let calls: Vec<_> = f
        .drain()
        .calls
        .iter()
        .map(|c| {
            json!({
                "toolName": c.tool_name,
                "args": serde_json::from_str::<Json>(&c.arguments_json().unwrap()).unwrap()
            })
        })
        .collect();
    assert_eq!(
        json!({"before": value(&before), "after": value(&after), "clickCalls": calls}),
        golden
    );
}

/// The calls a click handed over, in the golden's projection.
fn click_calls(f: &mut Fixture) -> Vec<Json> {
    f.drain()
        .calls
        .iter()
        .map(|c| {
            json!({
                "toolName": c.tool_name,
                "args": serde_json::from_str::<Json>(&c.arguments_json().unwrap()).unwrap()
            })
        })
        .collect()
}

// Route A of #205: nothing is refused. The write registers queries only, so
// the click still sends the tool registered with the program, as upstream's
// renderer does without a limit (the golden is that replay).
#[test]
fn mutation_clicked_after_an_accepted_state_write_keeps_the_registered_tool() {
    let golden: Json = serde_json::from_str(include_str!(
        "../../../conformance/golden/mutation-after-state-write.json"
    ))
    .unwrap();
    let mut f = Fixture::with_result_size(1 << 20);
    let p = program(
        &f.lib,
        include_str!("../../../conformance/mutation-after-state-write.oui"),
    );
    let persisted = json!({"$detail": "x".repeat(1200)}).to_string();
    let before =
        f.s.replace_program(&refs(), p, Some(persisted), None, false)
            .unwrap()
            .evaluation;
    assert_eq!(f.drain().calls.len(), 1);
    assert!(
        f.s.set(&refs(), "$t".into(), Some(r#""delete""#.into()))
            .unwrap()
    );
    let write = f.drain();
    assert!(write.evaluation_error.is_none());
    assert!(write.calls.is_empty());
    let after = f.evaluate();
    assert!(after.query_evaluation_error.is_none());
    assert!(f.drain().calls.is_empty());
    assert!(f.click(&after).failure.is_none());
    assert_eq!(
        json!({"before": value(&before), "after": value(&after), "clickCalls": click_calls(&mut f)}),
        golden
    );
}

// Route B of #205: the write is refused for a reason other than a limit (a
// query tool name that cannot become text, as in #187). Upstream's query
// effect throws on that write, so it has no comparable row; the pin is that a
// later write of `$t` still dispatches the registered tool with the current
// argument, the same call upstream sends after a write of `$t` alone.
#[test]
fn mutation_clicked_after_a_non_limit_refusal_keeps_the_registered_tool() {
    let mut f = Fixture::with_result_size(1 << 20);
    let before = f.replace(
        "$t = \"save\"\n$q = \"read\"\nm = Mutation($t, {shown: $t})\n\
         q = Query($q, {}, \"\")\nroot = Box($t, Action([@Run(m)]))",
    );
    assert_eq!(f.drain().calls.len(), 1);
    assert!(
        f.s.set(&refs(), "$q".into(), Some(r#"{"toString":null}"#.into()))
            .unwrap()
    );
    let refused = f.drain();
    assert!(matches!(
        refused.evaluation_error,
        Some(OpenUiError::Evaluation { .. })
    ));
    assert!(
        f.s.set(&refs(), "$t".into(), Some(r#""delete""#.into()))
            .unwrap()
    );
    let write = f.drain();
    assert_eq!(write.evaluation_error, refused.evaluation_error);
    assert!(write.calls.is_empty());
    let after = f.evaluate();
    assert_eq!(after.query_evaluation_error, refused.evaluation_error);
    assert!(f.drain().calls.is_empty());
    assert!(f.click(&after).failure.is_none());
    assert_eq!(
        json!({"before": value(&before), "after": value(&after), "clickCalls": click_calls(&mut f)}),
        json!({"before": "save", "after": "delete",
            "clickCalls": [{"toolName": "save", "args": {"shown": "delete"}}]})
    );
}
