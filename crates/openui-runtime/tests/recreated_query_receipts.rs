//! A receipt whose call the manager no longer owns is discarded, even while a
//! staged delivery keeps the old call alive and the same key has been
//! recreated with a call of its own (ADR 0013 rule 3, #204).
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    Limits, QueryCommand, QueryCommandError, QueryManager, ResultLimit, Store, ToolCall,
    ToolCallId, ToolCallOutcome, ToolOutcome, Value,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

const SIZE: usize = 1024;

fn program(source: &str) -> ParseResult {
    parse(source, &Library::from_json_schema(&JsonValue::Null), None)
}

fn evaluate(manager: &mut QueryManager, source: &str, store: &Store) -> Vec<ToolCall> {
    manager
        .apply(QueryCommand::Evaluate {
            program: &program(source),
            store,
        })
        .unwrap()
        .calls
}

fn view(manager: &QueryManager) -> JsonValue {
    parse_json(manager.snapshot_json()).unwrap()
}

/// Codex's reproduction from #204. Register `a` and `b`, and stage (but
/// neither finish nor drop) a delivery for `b`: its candidate manager keeps
/// `a`'s original call alive. Remove both queries, then register `recreated`.
/// `act` gets the manager, `a`'s original receipt, and the new calls. With
/// `stage` false the delivery is never staged, so the original receipt is
/// released as usual: the control.
fn with_old_receipt(
    stage: bool,
    recreated: &str,
    act: impl FnOnce(&mut QueryManager, &ToolCallId, Vec<ToolCall>),
) {
    let store = Store::default();
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_result_size(SIZE));
    let calls = evaluate(&mut m, "a=Query(\"a\")\nb=Query(\"b\")", &store);
    m.audit_output(&calls).unwrap();
    let old = calls
        .iter()
        .find(|c| c.tool_name == "a")
        .unwrap()
        .id
        .clone();
    let b = calls
        .iter()
        .find(|c| c.tool_name == "b")
        .unwrap()
        .id
        .clone();
    m.audit_cache([&old, &b].into_iter(), true).unwrap();
    // Unused after this line, the delivery stays in scope to the end.
    let _probe = stage.then(|| {
        m.stage_tool_call(&b, ToolCallOutcome::MissingResult)
            .unwrap()
    });
    assert!(evaluate(&mut m, "", &store).is_empty());
    let new = evaluate(&mut m, recreated, &store);
    m.audit_output(&new).unwrap();
    m.audit_cache(new.iter().map(|c| &c.id), true).unwrap();
    assert_eq!(
        old.is_pending(),
        stage,
        "only the staged delivery keeps the old call alive"
    );
    act(&mut m, &old, new);
}

/// The discarded receipt changed nothing: the snapshot is the same, the
/// replacement's call is still pending and the receipt is consumed.
fn assert_discarded(m: &QueryManager, old: &ToolCallId, before: &str, new: &[ToolCall]) {
    assert_eq!(
        m.snapshot_json(),
        before,
        "a retired call changed the manager"
    );
    assert!(new.iter().all(|call| call.id.is_pending()));
    assert!(matches!(
        m.check_call(old),
        Err(QueryCommandError::InvalidCall)
    ));
    m.audit_cache(new.iter().map(|c| &c.id), true).unwrap();
}

#[test]
fn a_late_result_does_not_settle_a_recreated_query_while_a_staged_delivery_is_alive() {
    for (stage, command) in [(true, false), (true, true), (false, false), (false, true)] {
        with_old_receipt(stage, "a=Query(\"a\")", |m, old, new| {
            let [replacement] = new.as_slice() else {
                panic!("expected one call for the recreated query")
            };
            assert_eq!(replacement.tool_name, "a");
            let before = m.snapshot_json().to_owned();
            let json = JsonValue::from("stale");
            let late = if command {
                m.apply(QueryCommand::Complete {
                    id: old,
                    outcome: ToolOutcome::Result(Some(&json)),
                })
            } else {
                m.complete_tool_call(old, ToolCallOutcome::Result(Value::Str("stale".into())))
            }
            .expect("a retired receipt is discarded, not refused");
            assert!(!late.needs_redraw() && late.calls.is_empty());
            assert_discarded(m, old, &before, &new);
            assert_eq!(view(m)["a"], JsonValue::Null);
            assert_eq!(view(m)["__openui_loading"], parse_json(r#"["a"]"#).unwrap());

            // The replacement's own call still delivers.
            m.complete_tool_call(
                &replacement.id,
                ToolCallOutcome::Result(Value::Str("fresh".into())),
            )
            .unwrap();
            assert_eq!(view(m)["a"], JsonValue::from("fresh"));
            assert_eq!(view(m)["__openui_loading"], parse_json("[]").unwrap());
            m.audit_cache(std::iter::empty(), true).unwrap();
        });
    }
}

#[test]
fn a_late_failure_does_not_reach_a_recreated_query_while_a_staged_delivery_is_alive() {
    let long = "x".repeat(SIZE + 1);
    for stage in [true, false] {
        for outcome in [
            || ToolCallOutcome::CallFailed("offline".into()),
            || ToolCallOutcome::McpError("denied".into()),
            || ToolCallOutcome::ToolNotFound(vec!["other".into()]),
        ]
        .map(|outcome| Box::new(outcome) as Box<dyn Fn() -> ToolCallOutcome>)
        .into_iter()
        // Failure text past the limits would refuse the command if it were
        // charged to the recreated query: a discarded payload is never read.
        .chain([
            Box::new(|| ToolCallOutcome::CallFailed(long.clone())) as Box<dyn Fn() -> _>,
            Box::new(|| ToolCallOutcome::ToolNotFound(vec![String::new(); SIZE / 2 + 1])),
        ]) {
            with_old_receipt(stage, "a=Query(\"a\")", |m, old, new| {
                // As for any released receipt, the payload is discarded before it is read.
                assert!(m.discards_tool_outcome(old).unwrap());
                let before = m.snapshot_json().to_owned();
                let late = m
                    .complete_tool_call(old, outcome())
                    .expect("a retired receipt's failure is discarded before admission");
                assert!(!late.needs_redraw() && late.calls.is_empty());
                assert_discarded(m, old, &before, &new);
                assert_eq!(view(m)["__openui_errors"], parse_json("[]").unwrap());
            });
        }
        // A result refused before it was read ends no call but its own.
        with_old_receipt(stage, "a=Query(\"a\")", |m, old, new| {
            let before = m.snapshot_json().to_owned();
            m.complete_tool_call_refused(old, ResultLimit::Size(SIZE))
                .unwrap();
            assert_discarded(m, old, &before, &new);
        });
    }
}

#[test]
fn a_late_receipt_keeps_the_run_remembered_for_a_recreated_query() {
    for stage in [true, false] {
        with_old_receipt(stage, "a=Query(\"a\")", |m, old, new| {
            let [replacement] = new.as_slice() else {
                panic!("expected one call for the recreated query")
            };
            // A run while the replacement is in flight is remembered for the key.
            let run = m
                .apply(QueryCommand::Invalidate {
                    statement_ids: &["a"],
                })
                .unwrap();
            assert!(run.calls.is_empty());
            let before = m.snapshot_json().to_owned();
            m.complete_tool_call(old, ToolCallOutcome::Result(Value::Num(1.0)))
                .unwrap();
            assert_discarded(m, old, &before, &new);
            let delivered = m
                .complete_tool_call(&replacement.id, ToolCallOutcome::Result(Value::Num(2.0)))
                .unwrap();
            let [again] = delivered.calls.as_slice() else {
                panic!("the remembered run starts when the replacement is delivered")
            };
            assert_eq!(again.tool_name, "a");
            assert_eq!(view(m)["a"], JsonValue::from(2));
            m.audit_cache(std::iter::once(&again.id), true).unwrap();
        });
    }
}

#[test]
fn abandoning_a_late_receipt_is_refused_as_for_a_released_one() {
    for stage in [true, false] {
        // The key is current again: the replacement keeps its call.
        with_old_receipt(stage, "a=Query(\"a\")", |m, old, new| {
            let before = m.snapshot_json().to_owned();
            assert!(!m.abandon_unperformed_call(old));
            assert!(m.check_call(old).is_ok());
            assert_eq!(m.snapshot_json(), before);
            assert!(new.iter().all(|call| call.id.is_pending()));
            m.complete_tool_call(old, ToolCallOutcome::MissingResult)
                .unwrap();
            assert_discarded(m, old, &before, &new);
        });
        // No query holds the key: the manager owns no call to end.
        with_old_receipt(stage, "c=Query(\"c\")", |m, old, new| {
            let before = m.snapshot_json().to_owned();
            assert!(!m.abandon_unperformed_call(old));
            assert!(m.check_call(old).is_ok());
            m.complete_tool_call(old, ToolCallOutcome::MissingResult)
                .unwrap();
            assert_discarded(m, old, &before, &new);
        });
    }
}

/// A small deterministic generator, so a failure replays.
struct Lcg(u64);
impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

fn late_outcome(rng: &mut Lcg) -> ToolCallOutcome {
    match rng.below(6) {
        0 => ToolCallOutcome::MissingResult,
        1 => ToolCallOutcome::CallFailed("offline".into()),
        2 => ToolCallOutcome::McpError("denied".into()),
        3 => ToolCallOutcome::ToolNotFound(vec!["other".into()]),
        // Past the result-size limit: refused if read, so discarding shows.
        4 => ToolCallOutcome::CallFailed("x".repeat(SIZE + 1)),
        _ => ToolCallOutcome::Result(Value::Num(rng.below(3) as f64)),
    }
}

/// A differential over receipt sequences (#204). Two managers get the same
/// seeded commands: edits that remove, recreate, share and move keys,
/// completions of any receipt ever issued (current, previous-only, released,
/// consumed), runs, abandonment and dispose. Before each round one of them
/// stages a delivery for a random receipt and keeps it, unfinished, until
/// the round ends. Every observation must be the same: a staged delivery
/// that was neither finished nor dropped changes nothing.
#[test]
fn a_live_staged_delivery_changes_no_observation_across_receipt_sequences() {
    const ROUNDS: usize = 20_000;
    const PROGRAMS: [&str; 6] = [
        "a=Query(\"a\")\nb=Query(\"b\")",
        "",
        "a=Query(\"a\")",
        "b=Query(\"b\")\nc=Query(\"a\")",
        "a=Query(\"a\",{v:$v})\nb=Query(\"b\")",
        "a=Query(\"a\")\nr=Query(\"r\",{x:a})",
    ];
    let programs = PROGRAMS.map(program);
    let mut rng = Lcg(204);
    let new = || QueryManager::with_limits(Limits::DEFAULT.with_result_size(SIZE));
    let (mut staged, mut plain) = (new(), new());
    let mut store = Store::default();
    // Receipts in issue order: the managers issue them in the same order.
    let mut receipts: Vec<(ToolCallId, ToolCallId)> = Vec::new();
    let (mut stages, mut late) = (0usize, 0usize);
    // As the replay drivers track it: an accepted evaluate while active
    // registers, and dispose releases every call until the next one.
    let mut registered = false;
    for round in 0..ROUNDS {
        // Any receipt not yet consumed can be staged. Its candidate keeps
        // every call open at this point alive until the round ends.
        let open: Vec<usize> = (0..receipts.len())
            .filter(|&i| plain.check_call(&receipts[i].1).is_ok())
            .collect();
        let held = (!open.is_empty()).then(|| receipts[open[rng.below(open.len())]].0.clone());
        let probe = held.as_ref().and_then(|id| {
            staged
                .stage_tool_call(id, late_outcome(&mut Lcg(round as u64)))
                .ok()
        });
        stages += usize::from(probe.is_some());
        for _ in 0..1 + rng.below(8) {
            let step = rng.below(10);
            let (ours, theirs) = match step {
                0..=2 => {
                    let p = &programs[rng.below(programs.len())];
                    let command = || QueryCommand::Evaluate {
                        program: p,
                        store: &store,
                    };
                    let outcome = (staged.apply(command()), plain.apply(command()));
                    registered |= outcome.1.is_ok() && !plain.is_disposed();
                    outcome
                }
                3..=6 if !receipts.is_empty() => {
                    // Half the time, a receipt that was open when the round began.
                    let (s, p) = &receipts[match rng.below(2) {
                        0 if !open.is_empty() => open[rng.below(open.len())],
                        _ => rng.below(receipts.len()),
                    }];
                    let seed = rng.below(usize::MAX);
                    let outcome = || late_outcome(&mut Lcg(seed as u64));
                    late += usize::from(s.is_pending() != p.is_pending());
                    assert_eq!(
                        staged.discards_tool_outcome(s).ok(),
                        plain.discards_tool_outcome(p).ok(),
                        "round {round}"
                    );
                    (
                        staged.complete_tool_call(s, outcome()),
                        plain.complete_tool_call(p, outcome()),
                    )
                }
                7 if !receipts.is_empty() => {
                    let (s, p) = &receipts[rng.below(receipts.len())];
                    assert_eq!(
                        staged.abandon_unperformed_call(s),
                        plain.abandon_unperformed_call(p),
                        "round {round}"
                    );
                    continue;
                }
                8 => {
                    let ids: &[&str] = [&["a"][..], &["b", "a"], &[]][rng.below(3)];
                    let command = || QueryCommand::Invalidate { statement_ids: ids };
                    (staged.apply(command()), plain.apply(command()))
                }
                _ => match rng.below(4) {
                    0 => {
                        registered = false;
                        (
                            staged.apply(QueryCommand::Dispose),
                            plain.apply(QueryCommand::Dispose),
                        )
                    }
                    1 => (
                        staged.apply(QueryCommand::Activate),
                        plain.apply(QueryCommand::Activate),
                    ),
                    _ => {
                        store
                            .set("$v", Some(Value::Num(rng.below(3) as f64)))
                            .unwrap();
                        continue;
                    }
                },
            };
            match (ours, theirs) {
                (Ok(ours), Ok(theirs)) => {
                    assert_eq!(ours.changed, theirs.changed, "round {round}");
                    assert_eq!(ours.calls.len(), theirs.calls.len(), "round {round}");
                    for (s, p) in ours.calls.into_iter().zip(theirs.calls) {
                        assert_eq!(
                            (&s.tool_name, s.arguments_json()),
                            (&p.tool_name, p.arguments_json()),
                            "round {round}"
                        );
                        receipts.push((s.id, p.id));
                    }
                }
                (ours, theirs) => assert_eq!(
                    format!("{:?}", ours.map(|_| ())),
                    format!("{:?}", theirs.map(|_| ())),
                    "round {round}"
                ),
            }
            assert_eq!(
                staged.snapshot_json(),
                plain.snapshot_json(),
                "round {round}"
            );
            assert_eq!(staged.cache_entry_count(), plain.cache_entry_count());
            assert_eq!(staged.cache_bytes(), plain.cache_bytes());
            for (s, p) in &receipts {
                assert_eq!(
                    staged.check_call(s).is_ok(),
                    plain.check_call(p).is_ok(),
                    "round {round}"
                );
            }
            // The plain manager says which receipts are truly outstanding.
            let live: Vec<_> = receipts
                .iter()
                .filter(|(_, p)| p.is_pending())
                .map(|(s, _)| s)
                .collect();
            plain
                .audit_cache(receipts.iter().map(|(_, p)| p), registered)
                .unwrap_or_else(|e| panic!("round {round}, plain: {e}"));
            staged
                .audit_cache(live.into_iter(), registered)
                .unwrap_or_else(|e| panic!("round {round}: {e}"));
        }
        // Bound the ledger on a long run.
        if receipts.len() > 256 {
            receipts.drain(..128);
        }
    }
    // The run reached the case: staged deliveries kept retired calls alive
    // while their receipts came back.
    assert!(stages > ROUNDS / 2, "{stages} stages");
    assert!(late > 200, "{late} late receipts held alive by a stage");
}
