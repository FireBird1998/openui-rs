//! An action the session drops because queries are paused (#117). A mutation
//! step clicked while paused, or in flight when the pause came, ends its plan
//! with exactly one `QueriesPaused` failure; nothing of the plan runs later.
//! A query step that cannot fetch is skipped and the plan carries on, as
//! upstream's `invalidate` does while its manager is disposed.
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

fn refs() -> PreparedReferenceValues<'static> {
    PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT)
}

fn program(lib: &ComponentLibrary, source: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(parse(source, &lib.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}

/// `$n` records how far the plan got: 1 before the first mutation, 2 after
/// it, and the second mutation is followed by an event.
const TWO_MUTATIONS: &str = "$n=0\nm=Mutation(\"save\")\nn=Mutation(\"other\")\nroot=Box([$n],Action([@Set($n,1),@Run(m),@Set($n,2),@Run(n),@Set($n,3),@OpenUrl(\"after\")]))";

struct Fixture {
    lib: Arc<ComponentLibrary>,
    s: SessionCore,
    // Receipts handed to the driver and not yet completed.
    outstanding: Vec<Arc<WantedToolCall>>,
    draw: Option<EvaluationOutput>,
}

impl Fixture {
    fn new(source: &str) -> Self {
        Self::with_limits(source, RuntimeLimits::default())
    }

    fn with_limits(source: &str, limits: RuntimeLimits) -> Self {
        let lib = library();
        let mut s = SessionCore::new(lib.clone(), None, limits).unwrap();
        s.attach_tool_provider(&refs()).unwrap();
        let mut f = Fixture {
            lib,
            s,
            outstanding: Vec::new(),
            draw: None,
        };
        f.replace(source);
        f
    }

    fn replace(&mut self, source: &str) {
        let p = program(&self.lib, source);
        let out = self
            .s
            .replace_program(&refs(), p, None, None, true)
            .unwrap();
        self.draw = Some(out.evaluation);
    }

    /// Drain the effects, keep the new receipts and check the manager.
    fn drain(&mut self) -> QueryEffects {
        let effects = self.s.take_query_update();
        self.outstanding.retain(|c| c.id.is_pending());
        self.outstanding.extend(effects.calls.iter().cloned());
        self.audit();
        effects
    }

    fn audit(&self) {
        self.s
            .audit_query_cache(self.outstanding.iter().map(|c| &c.id))
            .unwrap();
    }

    fn click(&mut self) -> ActionOutput {
        let draw = self.s.evaluate(&refs()).unwrap();
        let action = self
            .s
            .action_at(
                &refs(),
                draw.evaluation.clone(),
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        self.draw = Some(draw);
        let out = self
            .s
            .run(&refs(), Some(action), "click".into(), None)
            .unwrap();
        self.audit();
        out
    }

    fn call(&self, tool: &str) -> Arc<WantedToolCall> {
        self.outstanding
            .iter()
            .find(|c| c.tool_name == tool)
            .unwrap_or_else(|| panic!("no call for {tool}"))
            .clone()
    }

    fn finish(&mut self, call: &WantedToolCall, outcome: &Json) {
        self.s
            .complete_tool_call_json(&call.id, &outcome.to_string())
            .unwrap();
        self.audit();
    }

    fn pause(&mut self) {
        self.s.pause_queries().unwrap();
        self.audit();
    }

    fn resume(&mut self) {
        self.s.resume_queries().unwrap();
        self.audit();
    }

    fn n(&mut self) -> Json {
        let text = self.s.get(&refs(), "$n".into()).unwrap().unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

/// The one failure a dropped mutation step reports.
fn paused_failure(segment: &ActionOutput, step: u64) -> String {
    let failure = segment.failure.as_ref().expect("a failure");
    assert_eq!(failure.step, Some(step));
    let OpenUiError::QueriesPaused { reason } = &failure.error else {
        panic!("expected QueriesPaused, got {:?}", failure.error)
    };
    assert!(!segment.size_limited && !segment.depth_limited);
    assert!(segment.events.is_empty() && segment.continuation.is_none());
    assert!(reason.contains("paused") && reason.len() < 256, "{reason}");
    assert_eq!(failure.error.to_string(), *reason);
    reason.clone()
}

/// Nothing is pending and nothing comes out of a drain.
fn quiet(effects: &QueryEffects) {
    assert!(effects.calls.is_empty(), "no call");
    assert!(effects.actions.is_empty(), "no action segment");
}

#[test]
fn a_mutation_clicked_while_paused_fails_once_and_nothing_runs_after_resume() {
    let mut f = Fixture::new(TWO_MUTATIONS);
    quiet(&f.drain());
    f.pause();
    f.drain();
    let run = f.click();
    let reason = paused_failure(&run, 1);
    assert!(reason.contains("not sent"), "{reason}");
    assert_eq!(
        f.n(),
        json!(1),
        "the step before the mutation keeps its effect"
    );
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 0);
    quiet(&f.drain());
    f.resume();
    quiet(&f.drain());
    assert_eq!(f.n(), json!(1), "nothing of the plan runs after resume");
}

#[test]
fn a_mutation_in_flight_at_pause_fails_once_and_its_late_outcome_is_ignored() {
    for outcome in [json!({"value": 7}), json!({"failed": "late"})] {
        let mut f = Fixture::new(TWO_MUTATIONS);
        f.drain();
        let run = f.click();
        assert!(run.failure.is_none() && run.events.is_empty());
        f.drain();
        let save = f.call("save");
        f.pause();
        assert!(!save.id.is_pending());
        let paused = f.drain();
        assert!(paused.calls.is_empty());
        let [segment] = paused.actions.as_slice() else {
            panic!("expected one dropped action, got {:?}", paused.actions)
        };
        let reason = paused_failure(segment, 1);
        assert!(reason.contains("outcome"), "{reason}");
        // Pausing again reports nothing more.
        f.pause();
        quiet(&f.drain());
        f.resume();
        quiet(&f.drain());
        let version = f.s.version(&refs()).unwrap();
        f.finish(&save, &outcome);
        let late = f.drain();
        quiet(&late);
        assert!(!late.changed);
        assert_eq!(f.s.version(&refs()).unwrap(), version);
        assert!(matches!(
            f.s.complete_tool_call_json(&save.id, &outcome.to_string()),
            Err(OpenUiError::InvalidToolCall)
        ));
        assert_eq!(f.n(), json!(1));
    }
}

#[test]
fn a_provider_that_never_settles_still_gives_one_failure() {
    let mut f = Fixture::new(TWO_MUTATIONS);
    f.drain();
    f.click();
    f.drain();
    f.pause();
    let paused = f.drain();
    assert_eq!(paused.actions.len(), 1);
    paused_failure(&paused.actions[0], 1);
    f.resume();
    quiet(&f.drain());
    // The plan can be clicked again; it runs as a fresh click.
    let again = f.click();
    assert!(again.failure.is_none());
    assert_eq!(f.drain().calls[0].tool_name, "save");
}

#[test]
fn the_second_mutation_of_a_plan_in_flight_at_pause_names_its_own_step() {
    let mut f = Fixture::new(TWO_MUTATIONS);
    f.drain();
    f.click();
    f.drain();
    let save = f.call("save");
    f.finish(&save, &json!({"value": 1}));
    let resumed = f.drain();
    let [segment] = resumed.actions.as_slice() else {
        panic!("expected the resumed segment")
    };
    assert!(segment.failure.is_none());
    assert_eq!(f.n(), json!(2));
    let other = f.call("other");
    f.pause();
    let paused = f.drain();
    assert_eq!(paused.actions.len(), 1);
    paused_failure(&paused.actions[0], 3);
    f.finish(&other, &json!({"value": 2}));
    quiet(&f.drain());
    f.resume();
    quiet(&f.drain());
    assert_eq!(f.n(), json!(2), "steps after the dropped one never run");
}

#[test]
fn a_failure_waiting_in_the_session_survives_a_replacement_and_a_second_pause() {
    let mut f = Fixture::new(TWO_MUTATIONS);
    f.drain();
    f.click();
    f.drain();
    f.pause();
    f.pause();
    f.replace("$n=9\nroot=Box($n)");
    f.resume();
    let effects = f.drain();
    assert_eq!(effects.actions.len(), 1);
    paused_failure(&effects.actions[0], 1);
    quiet(&f.drain());
}

#[test]
fn a_host_continuation_resumed_while_paused_fails_at_its_managed_mutation() {
    // A plan suspended for the host before the provider was attached
    // reaches a managed mutation when it resumes, while queries are paused.
    let lib = library();
    let mut s = SessionCore::new(lib.clone(), None, RuntimeLimits::default()).unwrap();
    let out = s
        .replace_program(&refs(), program(&lib, TWO_MUTATIONS), None, None, true)
        .unwrap();
    let action = s
        .action_at(
            &refs(),
            out.evaluation.evaluation,
            vec!["props".into(), "action".into()],
        )
        .unwrap();
    let run = s.run(&refs(), Some(action), "click".into(), None).unwrap();
    let continuation = run.continuation.expect("a host handoff");
    s.attach_tool_provider(&refs()).unwrap();
    s.evaluate(&refs()).unwrap();
    s.take_query_update();
    s.pause_queries().unwrap();
    let resumed = s.resume(&refs(), continuation, true).unwrap();
    paused_failure(&resumed, 3);
    let effects = s.take_query_update();
    quiet(&effects);
    s.resume_queries().unwrap();
    quiet(&s.take_query_update());
    let n = s.get(&refs(), "$n".into()).unwrap();
    assert_eq!(n.as_deref(), Some("2"));
}

#[test]
fn an_undrained_failure_keeps_its_continuation_slot() {
    // Failures waiting for a driver are bounded by maxContinuations: until
    // one is taken, it holds the slot of the plan it ended.
    let limits = RuntimeLimits {
        max_continuations: 1,
        ..RuntimeLimits::default()
    };
    let mut f = Fixture::with_limits(TWO_MUTATIONS, limits);
    f.drain();
    for round in 0..3 {
        assert_eq!(f.click().failure.is_none(), round == 0);
        f.pause();
        f.resume();
    }
    // Only the first click was admitted; the others were refused at the
    // limit while its failure waited.
    let effects = f.drain();
    assert_eq!(effects.actions.len(), 1);
    paused_failure(&effects.actions[0], 1);
    assert!(f.click().failure.is_none(), "the slot is free once taken");
}

#[test]
fn a_click_at_the_limit_while_a_failure_waits_is_refused_at_its_step() {
    let limits = RuntimeLimits {
        max_continuations: 1,
        ..RuntimeLimits::default()
    };
    let mut f = Fixture::with_limits(TWO_MUTATIONS, limits);
    f.drain();
    f.click();
    f.pause();
    f.resume();
    let run = f.click();
    let failure = run.failure.expect("refused at the limit");
    assert_eq!(failure.step, Some(1));
    assert!(matches!(failure.error, OpenUiError::LimitExceeded { .. }));
    assert!(run.size_limited);
    let effects = f.drain();
    assert!(effects.calls.is_empty());
    assert_eq!(effects.actions.len(), 1);
}

#[test]
fn a_query_run_while_paused_starts_no_call_and_the_plan_carries_on() {
    // Upstream's `invalidate` returns at once while disposed and the
    // renderer's loop goes on to the next step (renderer-actions.ts).
    let mut f = Fixture::new(
        "$n=0\nq=Query(\"read\")\nroot=Box([q,$n],Action([@Run(q),@Set($n,5),@OpenUrl(\"after\")]))",
    );
    let first = f.drain();
    let read = first.calls[0].clone();
    f.finish(&read, &json!({"value": "data"}));
    f.drain();
    f.pause();
    f.drain();
    let run = f.click();
    assert!(run.failure.is_none() && !run.size_limited);
    assert_eq!(run.events.len(), 1);
    assert_eq!(f.n(), json!(5));
    quiet(&f.drain());
    f.resume();
    quiet(&f.drain());
}

/// Replay `conformance/paused-actions.json`. Each row is upstream's row with
/// the failures the session reported at that step.
fn replay_paused_scenarios() -> (String, String) {
    let corpus: Json =
        serde_json::from_str(include_str!("../../../conformance/paused-actions.json")).unwrap();
    assert_eq!(*library().schema, corpus["schema"]);
    let (mut plain, mut reported) = (String::new(), String::new());
    for scenario in corpus["scenarios"].as_array().unwrap() {
        let mut f = Fixture::new(scenario["program"].as_str().unwrap());
        f.drain();
        // Calls in start order, as the reference numbers its promises.
        let mut calls: Vec<Arc<WantedToolCall>> = f.outstanding.clone();
        for step in scenario["steps"].as_array().unwrap() {
            let mut segments = Vec::new();
            if step["pause"] == true {
                f.pause();
            } else if step["resume"] == true {
                f.resume();
            } else if step["click"] == true {
                segments.push(f.click());
            } else {
                let call = calls[step["finish"].as_u64().unwrap() as usize].clone();
                let outcome = if step.get("failed").is_some() {
                    json!({"failed": step["failed"]})
                } else {
                    json!({"value": step["value"]})
                };
                f.finish(&call, &outcome);
            }
            let effects = f.drain();
            segments.extend(effects.actions);
            let names: Vec<_> = effects.calls.iter().map(|c| c.tool_name.clone()).collect();
            calls.extend(effects.calls);
            let mut events = Vec::new();
            let mut failures = Vec::new();
            for segment in segments {
                events.extend(segment.events.iter().cloned());
                if let Some(failure) = segment.failure {
                    let OpenUiError::QueriesPaused { .. } = failure.error else {
                        panic!("unexpected failure {:?}", failure.error)
                    };
                    failures.push(json!({"step": failure.step, "kind": "QueriesPaused"}));
                }
            }
            let row = format!(
                "\"calls\":{},\"events\":[{}],\"n\":{}",
                json!(names),
                events.join(","),
                f.n()
            );
            plain.push_str(&format!("{{{row}}}\n"));
            reported.push_str(&format!("{{{row},\"failures\":{}}}\n", json!(failures)));
        }
    }
    (plain, reported)
}

#[test]
fn upstream_runs_nothing_of_a_paused_plan_and_the_session_adds_one_failure() {
    let upstream = include_str!("../../../conformance/golden/paused-actions.jsonl");
    let pin: Json = serde_json::from_str(include_str!(
        "../../../conformance/divergences/paused-actions.json"
    ))
    .unwrap();
    assert_eq!(upstream, pin["reference"].as_str().unwrap());
    let (plain, reported) = replay_paused_scenarios();
    // Calls, host events and state match upstream at every step.
    assert_eq!(plain, upstream);
    // The only addition is the failure, once per dropped action.
    assert_eq!(reported, pin["rust"].as_str().unwrap());
}

#[test]
fn a_call_never_handed_to_a_driver_is_reported_as_not_sent() {
    // The click's call was still in the session's undrained calls when the
    // pause cleared them: no driver ever held it.
    let mut f = Fixture::new(TWO_MUTATIONS);
    f.drain();
    assert!(f.click().failure.is_none());
    f.pause();
    let effects = f.drain();
    assert!(effects.calls.is_empty());
    let [segment] = effects.actions.as_slice() else {
        panic!("expected one dropped action")
    };
    let reason = paused_failure(segment, 1);
    assert!(reason.contains("not sent"), "{reason}");
    assert!(!reason.contains("may have run"), "{reason}");
}

#[test]
fn a_call_handed_to_a_driver_says_the_tool_may_have_run() {
    let mut f = Fixture::new(TWO_MUTATIONS);
    f.drain();
    f.click();
    f.drain();
    f.pause();
    let effects = f.drain();
    let reason = paused_failure(&effects.actions[0], 1);
    assert!(reason.contains("may have run"), "{reason}");
    assert!(!reason.contains("not sent"), "{reason}");
}

#[test]
fn a_host_handoff_counts_waiting_failures_against_the_slots() {
    // With room for two, one failure waiting and one host run pending, a
    // second host run evicts the first: the slots hold at most two. With
    // only failures waiting there is nothing to evict, and the handoff is
    // refused (the next test, docs/runtime-binding.md).
    let limits = RuntimeLimits {
        max_continuations: 2,
        ..RuntimeLimits::default()
    };
    let mut f = Fixture::with_limits(TWO_MUTATIONS, limits);
    f.drain();
    f.click();
    f.drain();
    f.pause();
    f.s.detach_tool_provider();
    let host = |f: &mut Fixture| f.click().continuation.expect("a host handoff");
    let first = host(&mut f);
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 1);
    let _second = host(&mut f);
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 1);
    assert!(matches!(
        f.s.resume(&refs(), first, true),
        Err(OpenUiError::EvictedHandle)
    ));
    let effects = f.s.take_query_update();
    assert_eq!(effects.actions.len(), 1, "the waiting failure is kept");
    paused_failure(&effects.actions[0], 1);
}

#[test]
fn a_host_handoff_with_only_failures_in_the_slots_is_refused_at_its_step() {
    // #127 item 8 (#142). Two plans in flight at the pause (the first at
    // its second mutation, the second at its first) leave two failures
    // waiting, filling both slots. With nothing to evict, a host
    // handoff would make three: it fails at its mutation step with the
    // pending-plan limit instead, before its mutation is handed to the host,
    // as a managed submission at the limit does. One slot short of full, a
    // handoff is still accepted, so runs plus failures reach the limit.
    let limits = RuntimeLimits {
        max_continuations: 2,
        ..RuntimeLimits::default()
    };
    let mut f = Fixture::with_limits(TWO_MUTATIONS, limits.clone());
    f.drain();
    f.click();
    f.drain();
    let save = f.call("save");
    f.finish(&save, &json!({"value": 1}));
    f.drain();
    assert!(f.click().failure.is_none());
    f.drain();
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 2);
    f.pause();
    f.s.detach_tool_provider();
    let run = f.click();
    let failure = run.failure.expect("refused at the limit");
    assert_eq!(failure.step, Some(1));
    assert!(matches!(failure.error, OpenUiError::LimitExceeded { .. }));
    assert!(run.size_limited && run.continuation.is_none());
    assert!(
        run.events.is_empty(),
        "the mutation is not handed to the host"
    );
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 0);
    let effects = f.s.take_query_update();
    assert_eq!(effects.actions.len(), 2, "both waiting failures are kept");
    paused_failure(&effects.actions[0], 3);
    paused_failure(&effects.actions[1], 1);
    // Taken, they free their slots.
    let _host = f.click().continuation.expect("a host handoff");
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 1);

    // At the boundary: one failure waiting and one slot free.
    let mut f = Fixture::with_limits(TWO_MUTATIONS, limits);
    f.drain();
    f.click();
    f.drain();
    f.pause();
    f.s.detach_tool_provider();
    let run = f.click();
    assert!(run.failure.is_none() && run.continuation.is_some());
    assert_eq!(f.s.pending_continuations(&refs()).unwrap(), 1);
    assert_eq!(f.s.take_query_update().actions.len(), 1);
}
