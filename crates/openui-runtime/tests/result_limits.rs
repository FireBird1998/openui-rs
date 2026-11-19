//! Tool results are admitted under the result-size and depth limits and the
//! cache cap before they enter the cache (#90). A refused result is the
//! call's `tool-error` on every query of its key, or the mutation's.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    JsObject, Limits, QueryCommand, QueryCommandError, QueryManager, ResultLimit, Store, ToolCall,
    ToolCallId, ToolCallOutcome, ToolOutcome, TreeInput, Value,
};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

const SIZE: usize = 1024;

fn program(source: &str) -> ParseResult {
    parse(source, &Library::from_json_schema(&JsonValue::Null), None)
}

fn evaluate(manager: &mut QueryManager, program: &ParseResult, store: &Store) -> Vec<ToolCall> {
    manager
        .apply(QueryCommand::Evaluate { program, store })
        .unwrap()
        .calls
}

/// A string result whose JSON is exactly `bytes` long.
fn text(bytes: usize) -> JsonValue {
    JsonValue::String("x".repeat(bytes - 2))
}

fn nested(levels: usize) -> JsonValue {
    (0..levels).fold(JsonValue::Null, |value, _| JsonValue::Array(vec![value]))
}

fn complete(manager: &mut QueryManager, id: &ToolCallId, value: &JsonValue) {
    manager
        .apply(QueryCommand::Complete {
            id,
            outcome: ToolOutcome::Result(Some(value)),
        })
        .unwrap();
}

fn view(manager: &QueryManager) -> JsonValue {
    parse_json(manager.snapshot_json()).unwrap()
}

fn query_error(tool: &str, limit: ResultLimit) -> String {
    format!("Query \"{tool}\" failed: {limit}")
}

fn check_charge(manager: &QueryManager) {
    assert_eq!(manager.cache_bytes(), manager.measured_cache_bytes());
    assert!(manager.cache_bytes() <= manager.cache_limit());
}

#[test]
fn the_limits_name_themselves_in_the_tool_error() {
    assert_eq!(
        ResultLimit::Size(64).to_string(),
        "tool result exceeds the result size limit of 64 bytes"
    );
    assert_eq!(
        ResultLimit::Depth(3).to_string(),
        "tool result nests deeper than the depth limit of 3 levels"
    );
    assert_eq!(
        ResultLimit::Cache(9).to_string(),
        "tool result does not fit the cache limit of 9 bytes"
    );
    let manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(SIZE));
    assert_eq!(
        manager.cache_limit(),
        SIZE,
        "the cap defaults to the result size"
    );
}

#[test]
fn a_result_fits_at_the_size_limit_and_one_byte_over_is_a_tool_error_on_every_reader() {
    let store = Store::default();
    // One reader shows a result at the limit; the snapshot holds one copy.
    let one = program("a = Query(\"t\", {}, \"a\")");
    let two = program("a = Query(\"t\", {}, \"a\")\nb = Query(\"t\", {}, \"b\")");
    for (bytes, fits) in [(SIZE, true), (SIZE + 1, false)] {
        let limits = Limits::DEFAULT.with_result_size(SIZE);
        let mut manager = QueryManager::with_limits(limits).with_cache_limit(usize::MAX);
        let p = if fits { &one } else { &two };
        let call = evaluate(&mut manager, p, &store).remove(0);
        complete(&mut manager, &call.id, &text(bytes));
        assert!(!call.id.is_pending(), "the call ends either way");
        let snapshot = view(&manager);
        if fits {
            assert_eq!(snapshot["a"], text(bytes));
            assert_eq!(manager.cache_bytes(), SIZE);
        } else {
            let message = query_error("t", ResultLimit::Size(SIZE));
            for (index, id) in ["a", "b"].iter().enumerate() {
                let error = &snapshot["__openui_errors"][index];
                assert_eq!(error["statementId"], *id);
                assert_eq!(error["code"], "tool-error");
                assert_eq!(error["message"], message.as_str());
                assert_eq!(snapshot[*id], *id, "defaults stay on screen");
            }
            assert_eq!(manager.cache_bytes(), 0);
        }
        check_charge(&manager);
    }
}

#[test]
fn a_value_result_has_the_same_thresholds_through_the_session_completion() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    for (value, refused) in [
        (text(SIZE), None),
        (text(SIZE + 1), Some(ResultLimit::Size(SIZE))),
        (nested(6), None),
        (nested(7), Some(ResultLimit::Depth(6))),
    ] {
        let limits = Limits::DEFAULT.with_result_size(SIZE).with_depth(6);
        let mut manager = QueryManager::with_limits(limits);
        let call = evaluate(&mut manager, &p, &store).remove(0);
        let (_, delivered) = manager
            .complete_tool_call_admitted(
                &call.id,
                ToolCallOutcome::Result(Value::from_json(&value)),
            )
            .unwrap();
        assert_eq!(delivered, refused, "{value}");
        let errors = view(&manager)["__openui_errors"].clone();
        match refused {
            Some(limit) => assert_eq!(errors[0]["message"], query_error("t", limit).as_str()),
            None => assert_eq!(errors, JsonValue::Array(Vec::new())),
        }
        check_charge(&manager);
    }
}

#[test]
fn a_result_fits_at_the_depth_limit_and_one_level_over_is_a_tool_error() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    for (levels, fits) in [(6, true), (7, false)] {
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(6));
        let call = evaluate(&mut manager, &p, &store).remove(0);
        complete(&mut manager, &call.id, &nested(levels));
        let snapshot = view(&manager);
        if fits {
            assert_eq!(snapshot["q"], nested(levels));
        } else {
            assert_eq!(snapshot["q"], JsonValue::Null);
            assert_eq!(
                snapshot["__openui_errors"][0]["message"],
                query_error("t", ResultLimit::Depth(6)).as_str()
            );
        }
        check_charge(&manager);
    }
}

#[test]
fn a_result_fits_at_the_cache_cap_and_a_previous_keys_data_counts() {
    let mut store = Store::default();
    let p = program("$k = 0\na = Query(\"a\", {k: $k})\nb = Query(\"b\")");
    for (bytes, fits) in [(60, true), (61, false)] {
        let mut manager = QueryManager::with_limits(Limits::DEFAULT).with_cache_limit(100);
        store.set("$k", Some(Value::Num(0.0))).unwrap();
        let calls = evaluate(&mut manager, &p, &store);
        complete(&mut manager, &calls[0].id, &text(40));
        // `a` moves on: its previous key's 40 bytes stay on screen and count.
        store.set("$k", Some(Value::Num(1.0))).unwrap();
        let next = evaluate(&mut manager, &p, &store).remove(0);
        assert_eq!(manager.cache_entry_count(), 3);
        complete(&mut manager, &calls[1].id, &text(bytes));
        let snapshot = view(&manager);
        assert_eq!(snapshot["a"], text(40), "nothing held is evicted");
        if fits {
            assert_eq!(snapshot["b"], text(bytes));
            assert_eq!(manager.cache_bytes(), 100);
        } else {
            assert_eq!(snapshot["b"], JsonValue::Null);
            assert_eq!(
                snapshot["__openui_errors"][0]["message"],
                query_error("b", ResultLimit::Cache(100)).as_str()
            );
            assert_eq!(manager.cache_bytes(), 40);
        }
        check_charge(&manager);
        // `a`'s own result replaces its previous entry, which is released
        // with it: only what is held after the completion counts.
        let fits_after_release = 100 - manager.cache_bytes() + 40;
        complete(&mut manager, &next.id, &text(fits_after_release));
        assert_eq!(view(&manager)["a"], text(fits_after_release));
        assert_eq!(manager.cache_bytes(), 100);
        check_charge(&manager);
    }
}

fn mutation_manager(limits: Limits, cache: usize) -> (QueryManager, ToolCallId) {
    let p = program("q = Query(\"q\")\nm = Mutation(\"save\")");
    let store = Store::default();
    let mut manager = QueryManager::with_limits(limits).with_cache_limit(cache);
    let query = evaluate(&mut manager, &p, &store).remove(0);
    complete(&mut manager, &query.id, &text(30));
    manager
        .register_mutations(&TreeInput::new(&p), &store)
        .unwrap();
    let call = manager.fire_mutation("m", None).unwrap().calls.remove(0);
    (manager, call.id)
}

#[test]
fn mutation_results_have_the_same_thresholds_and_count_toward_the_cap() {
    let limits = Limits::DEFAULT.with_result_size(SIZE).with_depth(6);
    for (value, cache, refused) in [
        (text(SIZE), usize::MAX, None),
        (text(SIZE + 1), usize::MAX, Some(ResultLimit::Size(SIZE))),
        (nested(6), usize::MAX, None),
        (nested(7), usize::MAX, Some(ResultLimit::Depth(6))),
        // The query's 30 bytes count: 70 more fit a cap of 100, 71 do not.
        (text(70), 100, None),
        (text(71), 100, Some(ResultLimit::Cache(100))),
    ] {
        let (mut manager, id) = mutation_manager(limits, cache);
        let (_, delivered) = manager
            .complete_tool_call_admitted(&id, ToolCallOutcome::Result(Value::from_json(&value)))
            .unwrap();
        assert_eq!(delivered, refused);
        let snapshot = view(&manager);
        match refused {
            None => {
                assert_eq!(snapshot["m"]["status"], "success");
                assert_eq!(snapshot["m"]["data"], value);
            }
            Some(limit) => {
                assert_eq!(snapshot["m"]["status"], "error");
                assert_eq!(snapshot["m"]["error"], limit.to_string().as_str());
                let error = &snapshot["__openui_errors"][0];
                assert_eq!(error["code"], "tool-error");
                assert_eq!(
                    error["message"],
                    format!("Mutation \"save\" failed: {limit}").as_str()
                );
                assert_eq!(snapshot["q"], text(30), "nothing held is evicted");
            }
        }
        assert!(!id.is_pending());
        check_charge(&manager);
        // A second fire drops the previous data from the charge.
        manager.fire_mutation("m", None).unwrap();
        assert_eq!(manager.cache_bytes(), 30);
        check_charge(&manager);
    }
}

#[test]
fn a_value_that_is_not_json_is_refused_and_changes_nothing() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    let mut manager = QueryManager::default();
    let call = evaluate(&mut manager, &p, &store).remove(0);
    let before = manager.snapshot_json().to_owned();
    let syntax = Value::Ast(Arc::new(openui_runtime::Ast::Null));
    assert!(matches!(
        manager.complete_tool_call(&call.id, ToolCallOutcome::Result(syntax)),
        Err(QueryCommandError::Limit(_))
    ));
    assert_eq!(manager.snapshot_json(), before);
    assert!(call.id.is_pending());
    manager
        .complete_tool_call(&call.id, ToolCallOutcome::Result(Value::Null))
        .unwrap();
}

#[test]
fn refusing_an_oversized_result_allocates_nothing_in_proportion_to_it() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    let limits = Limits::DEFAULT.with_result_size(4096).with_depth(16);
    let wide = JsonValue::Array(vec![JsonValue::Null; 1 << 16]);
    let long = JsonValue::String("x".repeat(1 << 20));
    let deep = nested(1 << 12);
    let mut values = JsObject::new();
    values.define("x", Value::Str("x".repeat(1 << 20)));
    let as_value = Value::Obj(Arc::new(values));
    for (name, input) in [("wide", &wide), ("long", &long), ("deep", &deep)] {
        let mut manager = QueryManager::with_limits(limits);
        let call = evaluate(&mut manager, &p, &store).remove(0);
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        complete(&mut manager, &call.id, input);
        let extra = test_alloc::peak().saturating_sub(before);
        assert!(manager.snapshot_json().contains("tool-error"), "{name}");
        assert!(
            extra < 16 * 1024,
            "{name}: {extra} bytes allocated to refuse"
        );
    }
    let mut manager = QueryManager::with_limits(limits);
    let call = evaluate(&mut manager, &p, &store).remove(0);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let (_, refused) = manager
        .complete_tool_call_admitted(&call.id, ToolCallOutcome::Result(as_value))
        .unwrap();
    let extra = test_alloc::peak().saturating_sub(before);
    assert_eq!(refused, Some(ResultLimit::Size(4096)));
    assert!(
        extra < 16 * 1024,
        "value: {extra} bytes allocated to refuse"
    );
    openui_lang::free_json(deep);
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

#[test]
fn a_long_random_script_keeps_every_invariant_after_every_command() {
    const STEPS: usize = 20_000;
    let p = program(
        "$a = 0\n$b = 0\nx = Query(\"t\", {v: $a})\ny = Query(\"t\", {v: $b})\nz = Query(\"u\", {v: $a + $b})\nm = Mutation(\"save\" + $t)\nn = Mutation(\"post\")",
    );
    let limits = Limits::DEFAULT.with_result_size(256).with_depth(4);
    let mut manager = QueryManager::with_limits(limits).with_cache_limit(600);
    let mut store = Store::default();
    store.set("$a", Some(Value::Num(0.0))).unwrap();
    store.set("$b", Some(Value::Num(0.0))).unwrap();
    store.set("$t", Some(Value::Num(0.0))).unwrap();
    manager
        .register_mutations(&TreeInput::new(&p), &store)
        .unwrap();
    let mut rng = Lcg(90);
    let mut outstanding: Vec<ToolCallId> = Vec::new();
    let mut mutation_calls: Vec<ToolCallId> = Vec::new();
    let mut consumed: Vec<ToolCallId> = Vec::new();
    let (mut refusals, mut deliveries) = (0, 0);
    for step in 0..STEPS {
        let before = (manager.snapshot_json().to_owned(), manager.cache_bytes());
        let mut refused_command = false;
        match rng.below(7) {
            // Rename `m`: an idle registration whose earlier calls still run.
            6 => {
                store
                    .set("$t", Some(Value::Num(rng.below(3) as f64)))
                    .unwrap();
                manager
                    .register_mutations(&TreeInput::new(&p), &store)
                    .unwrap();
            }
            0 | 1 => {
                let name = if rng.below(2) == 0 { "$a" } else { "$b" };
                let value = rng.below(4) as f64;
                store.set(name, Some(Value::Num(value))).unwrap();
                let update = manager
                    .apply(QueryCommand::Evaluate {
                        program: &p,
                        store: &store,
                    })
                    .unwrap();
                outstanding.extend(update.calls.into_iter().map(|call| call.id));
            }
            2 | 3 if !outstanding.is_empty() => {
                let id = outstanding.swap_remove(rng.below(outstanding.len()));
                // Sizes around the 256-byte result limit and the 600-byte cap.
                let value = match rng.below(4) {
                    0 => nested(3 + rng.below(3)),
                    _ => text(2 + rng.below(320)),
                };
                let result = if rng.below(2) == 0 {
                    manager
                        .apply(QueryCommand::Complete {
                            id: &id,
                            outcome: ToolOutcome::Result(Some(&value)),
                        })
                        .map(|update| (update, None))
                } else {
                    manager.complete_tool_call_admitted(
                        &id,
                        ToolCallOutcome::Result(Value::from_json(&value)),
                    )
                };
                match result {
                    Ok((update, refused)) => {
                        deliveries += 1;
                        refusals += usize::from(refused.is_some());
                        outstanding.extend(update.calls.into_iter().map(|call| call.id));
                        consumed.push(id);
                    }
                    // The snapshot cannot hold the delivery: nothing changes.
                    Err(QueryCommandError::Limit(_)) => {
                        refused_command = true;
                        outstanding.push(id);
                    }
                    Err(error) => panic!("step {step}: {error}"),
                }
            }
            4 => {
                let name = if rng.below(2) == 0 { "m" } else { "n" };
                let update = manager.fire_mutation(name, None).unwrap();
                assert!(update.calls.len() <= 1);
                mutation_calls.extend(update.calls.iter().map(|call| call.id.clone()));
                outstanding.extend(update.calls.into_iter().map(|call| call.id));
            }
            _ => {
                let update = manager
                    .apply(QueryCommand::Invalidate { statement_ids: &[] })
                    .unwrap();
                outstanding.extend(update.calls.into_iter().map(|call| call.id));
            }
        }
        // A completed call is refused again, and that changes nothing.
        if let Some(id) = consumed.last() {
            let after = (manager.snapshot_json().to_owned(), manager.cache_bytes());
            assert!(matches!(
                manager.apply(QueryCommand::Complete {
                    id,
                    outcome: ToolOutcome::Result(None)
                }),
                Err(QueryCommandError::InvalidCall)
            ));
            assert_eq!(
                (manager.snapshot_json().to_owned(), manager.cache_bytes()),
                after
            );
        }
        if refused_command {
            assert_eq!(
                (manager.snapshot_json().to_owned(), manager.cache_bytes()),
                before,
                "step {step}: a refused command changed something"
            );
        }
        outstanding.retain(ToolCallId::is_pending);
        check_charge(&manager);
        assert!(manager.cache_entry_count() <= 2 * 3, "step {step}");
        // Each query has at most one call for its current key and one on the
        // key it shows while loading, and each mutation at most one.
        mutation_calls.retain(ToolCallId::is_pending);
        // A query has at most two live calls: one for its current key and
        // one finishing on the previous key it shows (ADR 0013 rule 3).
        assert!(
            outstanding.len() - mutation_calls.len() <= 2 * 3,
            "step {step}"
        );
        assert!(
            outstanding.len() - mutation_calls.len() <= manager.cache_entry_count(),
            "step {step}"
        );
        let snapshot = view(&manager);
        for id in ["x", "y", "z"] {
            let error = snapshot["__openui_errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["statementId"] == id);
            assert!(
                manager.current_entry_is_served(id, error, outstanding.iter()),
                "step {step}: {id} is starved"
            );
        }
    }
    assert!(
        refusals > 100 && deliveries > refusals,
        "{refusals} of {deliveries}"
    );
    // At the end the charge equals a fresh measurement of what is held.
    assert_eq!(manager.cache_bytes(), manager.measured_cache_bytes());
}

// ---- Review round: each test below failed before its fix. ----

fn mutation_program(tool: &str) -> ParseResult {
    program(&format!("q = Query(\"t\")\nm = Mutation(\"{tool}\")"))
}

#[test]
fn a_renamed_mutations_idle_null_is_not_charged_and_the_charge_stays_exact() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024);
    let mut manager = QueryManager::with_limits(limits).with_cache_limit(20);
    let old = mutation_program("old");
    let call = evaluate(&mut manager, &old, &store).remove(0);
    manager
        .register_mutations(&TreeInput::new(&old), &store)
        .unwrap();
    complete(&mut manager, &call.id, &text(20));
    check_charge(&manager);
    manager
        .register_mutations(&TreeInput::new(&mutation_program("new")), &store)
        .unwrap();
    assert_eq!(view(&manager)["m"]["data"], JsonValue::Null);
    assert_eq!(manager.cache_bytes(), 20);
    check_charge(&manager);
}

#[test]
fn a_refused_result_of_an_older_submission_behaves_as_that_calls_failure() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024);
    let mut manager = QueryManager::with_limits(limits).with_cache_limit(1024);
    let old = mutation_program("old");
    manager
        .register_mutations(&TreeInput::new(&old), &store)
        .unwrap();
    let first = manager.fire_mutation("m", None).unwrap().calls.remove(0);
    manager
        .register_mutations(&TreeInput::new(&mutation_program("new")), &store)
        .unwrap();
    let second = manager.fire_mutation("m", None).unwrap().calls.remove(0);
    complete(&mut manager, &second.id, &JsonValue::from("saved"));
    // As in lang-core 1.1.0 a stale outcome replaces the newer one, and a
    // refused result is that call's failure.
    let (_, refused) = manager
        .complete_tool_call_admitted(
            &first.id,
            ToolCallOutcome::Result(Value::from_json(&text(1025))),
        )
        .unwrap();
    assert_eq!(refused, Some(ResultLimit::Size(1024)));
    assert!(!first.id.is_pending(), "the old call is consumed");
    let snapshot = view(&manager);
    assert_eq!(snapshot["m"]["status"], "error");
    assert_eq!(
        snapshot["m"]["error"],
        ResultLimit::Size(1024).to_string().as_str()
    );
    check_charge(&manager);
    // A call whose registration was removed settles nothing, as on main.
    let third = manager.fire_mutation("m", None).unwrap().calls.remove(0);
    manager
        .register_mutations(&TreeInput::new(&program("q = Query(\"t\")")), &store)
        .unwrap();
    let before = manager.snapshot_json().to_owned();
    complete(&mut manager, &third.id, &JsonValue::from("late"));
    assert_eq!(manager.snapshot_json(), before);
}

#[test]
fn a_completion_for_a_removed_query_is_discarded_on_both_paths_whatever_it_holds() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024).with_depth(4);
    let long = "x".repeat(1024);
    let (large, deep) = (text(2048), nested(9));
    let syntax = Value::Ast(Arc::new(openui_runtime::Ast::Null));
    for kind in [
        "tool-not-found",
        "mcp-error",
        "failed",
        "large",
        "deep",
        "syntax",
    ] {
        for owned in [false, true] {
            let mut manager = QueryManager::with_limits(limits);
            let call = evaluate(&mut manager, &program("q = Query(\"t\")"), &store).remove(0);
            evaluate(&mut manager, &program("r = Query(\"u\")"), &store);
            let before = manager.snapshot_json().to_owned();
            let result = if owned {
                let outcome = match kind {
                    "tool-not-found" => ToolCallOutcome::ToolNotFound(vec![long.clone()]),
                    "mcp-error" => ToolCallOutcome::McpError(long.clone()),
                    "failed" => ToolCallOutcome::CallFailed(long.clone()),
                    "large" => ToolCallOutcome::Result(Value::from_json(&large)),
                    "deep" => ToolCallOutcome::Result(Value::from_json(&deep)),
                    _ => ToolCallOutcome::Result(syntax.clone()),
                };
                manager.complete_tool_call(&call.id, outcome).map(|_| ())
            } else {
                let outcome = match kind {
                    "tool-not-found" => ToolOutcome::ToolNotFound(vec![long.as_str()]),
                    "mcp-error" => ToolOutcome::McpError(&long),
                    "failed" => ToolOutcome::CallFailed(&long),
                    "large" => ToolOutcome::Result(Some(&large)),
                    "deep" => ToolOutcome::Result(Some(&deep)),
                    _ => ToolOutcome::ResultValue(&syntax),
                };
                manager
                    .apply(QueryCommand::Complete {
                        id: &call.id,
                        outcome,
                    })
                    .map(|_| ())
            };
            assert!(result.is_ok(), "{kind}, owned {owned}: {result:?}");
            assert!(!call.id.is_pending(), "{kind}, owned {owned}");
            assert_eq!(manager.snapshot_json(), before, "{kind}, owned {owned}");
        }
    }
}

#[test]
fn an_over_limit_result_ends_the_call_for_any_number_of_readers() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024);
    let readers = program(
        &(0..24)
            .map(|i| format!("q{i} = Query(\"t\")"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    for path in ["owned", "borrowed", "refused before reading"] {
        let mut manager = QueryManager::with_limits(limits);
        let call = evaluate(&mut manager, &readers, &store).remove(0);
        match path {
            "owned" => {
                let value = Value::from_json(&text(1025));
                manager
                    .complete_tool_call(&call.id, ToolCallOutcome::Result(value))
                    .unwrap();
            }
            "borrowed" => complete(&mut manager, &call.id, &text(1025)),
            _ => {
                manager
                    .complete_tool_call_refused(&call.id, ResultLimit::Size(1024))
                    .unwrap();
            }
        }
        assert!(!call.id.is_pending(), "{path}");
        let snapshot = view(&manager);
        assert_eq!(
            snapshot["__openui_loading"],
            JsonValue::Array(Vec::new()),
            "{path}"
        );
        assert_eq!(
            snapshot["__openui_errors"][0]["message"],
            "24 queries failed with errors that do not fit the result size limit of 1024 bytes",
            "{path}"
        );
    }
}

#[test]
fn an_objects_member_order_never_decides_invalid_versus_too_deep() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    let limits = Limits::DEFAULT.with_result_size(1024).with_depth(4);
    let deep = || (0..6).fold(Value::Null, |value, _| Value::Arr(Arc::new(vec![value])));
    let syntax = || Value::Ast(Arc::new(openui_runtime::Ast::Null));
    // Integer keys come first in JavaScript's order, last in insertion order.
    for (first, second) in [("b", "1"), ("1", "b")] {
        for syntax_first in [true, false] {
            let mut fields = JsObject::new();
            let (a, b) = if syntax_first {
                (syntax(), deep())
            } else {
                (deep(), syntax())
            };
            fields.define(first, a);
            fields.define(second, b);
            let mut manager = QueryManager::with_limits(limits);
            let call = evaluate(&mut manager, &p, &store).remove(0);
            let result = manager.complete_tool_call(
                &call.id,
                ToolCallOutcome::Result(Value::Obj(Arc::new(fields))),
            );
            assert!(
                matches!(result, Err(QueryCommandError::Limit(_))),
                "{first} then {second}, syntax first {syntax_first}"
            );
            assert!(call.id.is_pending());
        }
    }
    // Past the depth limit nothing is read: a syntax value there is too deep.
    let hidden = (0..6).fold(syntax(), |value, _| Value::Arr(Arc::new(vec![value])));
    let mut manager = QueryManager::with_limits(limits);
    let call = evaluate(&mut manager, &p, &store).remove(0);
    let (_, refused) = manager
        .complete_tool_call_admitted(&call.id, ToolCallOutcome::Result(hidden))
        .unwrap();
    assert_eq!(refused, Some(ResultLimit::Depth(4)));
}
#[test]
fn every_completion_path_admits_through_the_same_limits() {
    let store = Store::default();
    // A missing query result is `null`, four bytes: past a three-byte limit.
    let limits = Limits::DEFAULT.with_result_size(3);
    let mut manager = QueryManager::with_limits(limits).with_cache_limit(16);
    let call = evaluate(&mut manager, &program("q = Query(\"t\")"), &store).remove(0);
    manager
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(None),
        })
        .unwrap();
    assert_eq!(manager.cache_bytes(), 0);
    assert!(manager.snapshot_json().contains("of 3 bytes"));
    // The work limit bounds a result too, on every path.
    let limits = Limits::DEFAULT.with_work(256).with_result_size(8192);
    let size = limits.work_for(usize::MAX);
    assert!(size < 4096);
    let message = ResultLimit::Size(size).to_string();
    assert!(!message.is_empty());
    let big = text(4096);
    for path in [
        "borrowed query",
        "owned query",
        "borrowed mutation",
        "owned mutation",
    ] {
        let mut manager = QueryManager::with_limits(limits).with_cache_limit(8192);
        let p = program("q = Query(\"t\")\nm = Mutation(\"t\")");
        let query = evaluate(&mut manager, &p, &store).remove(0);
        manager
            .register_mutations(&TreeInput::new(&p), &store)
            .unwrap();
        let id = if path.ends_with("mutation") {
            manager.fire_mutation("m", None).unwrap().calls.remove(0).id
        } else {
            query.id
        };
        if path.starts_with("borrowed") {
            complete(&mut manager, &id, &big);
        } else {
            manager
                .complete_tool_call(&id, ToolCallOutcome::Result(Value::from_json(&big)))
                .unwrap();
        }
        // Named by the tool-error, or by the compact diagnostic when this
        // small a work limit cannot list the error either.
        let snapshot = manager.snapshot_json();
        assert!(
            snapshot.contains(&message) || snapshot.contains(&format!("limit of {size} bytes")),
            "{path}: {snapshot}"
        );
        assert_eq!(manager.cache_bytes(), 0, "{path}");
    }
}

#[test]
fn refusing_wide_or_long_results_allocates_nothing_in_proportion_to_them() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    let limits = Limits::DEFAULT.with_result_size(1024);
    let object = |keys: usize| {
        let mut fields = JsObject::new();
        for key in 0..keys {
            fields.define(format!("k{key}"), Value::Null);
        }
        Value::Obj(Arc::new(fields))
    };
    let mut peaks = Vec::new();
    for width in [1024, 2048, 4096, 1 << 16] {
        for value in [
            object(width),
            Value::Arr(Arc::new(vec![Value::Null; width])),
            Value::Str("x".repeat(width * 4)),
        ] {
            let mut manager = QueryManager::with_limits(limits);
            let call = evaluate(&mut manager, &p, &store).remove(0);
            // Keep an owner, so the call's drop of its outcome only releases
            // it: freeing a uniquely owned tree is the core's stack-safe free,
            // not admission.
            let kept = value.clone();
            test_alloc::reset_peak();
            let before = test_alloc::peak();
            let (_, refused) = manager
                .complete_tool_call_admitted(&call.id, ToolCallOutcome::Result(value))
                .unwrap();
            peaks.push(test_alloc::peak().saturating_sub(before));
            drop(kept);
            assert_eq!(refused, Some(ResultLimit::Size(1024)));
        }
    }
    assert!(peaks.iter().all(|&peak| peak < 8 * 1024), "{peaks:?}");
}

#[test]
fn validity_within_the_depth_limit_is_decided_before_depth_and_size_in_either_order() {
    let store = Store::default();
    let p = program("q = Query(\"t\")");
    let limits = Limits::DEFAULT.with_result_size(1024).with_depth(4);
    let syntax = || Value::Ast(Arc::new(openui_runtime::Ast::Null));
    let long = || Value::Str("x".repeat(1500));
    let deep = || (0..6).fold(Value::Null, |value, _| Value::Arr(Arc::new(vec![value])));
    for (name, items, expected) in [
        ("long then syntax", vec![long(), syntax()], None),
        ("syntax then long", vec![syntax(), long()], None),
        ("deep then syntax", vec![deep(), syntax()], None),
        ("syntax then deep", vec![syntax(), deep()], None),
    ] {
        let mut manager = QueryManager::with_limits(limits);
        let call = evaluate(&mut manager, &p, &store).remove(0);
        let result = manager.complete_tool_call_admitted(
            &call.id,
            ToolCallOutcome::Result(Value::Arr(Arc::new(items))),
        );
        match expected {
            None => {
                assert!(matches!(result, Err(QueryCommandError::Limit(_))), "{name}");
                assert!(call.id.is_pending(), "{name}: nothing consumed");
            }
            Some(limit) => {
                assert_eq!(result.unwrap().1, Some(limit), "{name}");
                assert!(!call.id.is_pending(), "{name}");
            }
        }
    }
}

#[test]
fn a_delivery_the_snapshot_cannot_hold_still_ends_the_call_on_both_paths() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024);
    let two = program("a = Query(\"t\")\nb = Query(\"t\")");
    let result = text(602);
    for owned in [false, true] {
        let mut manager = QueryManager::with_limits(limits);
        let call = evaluate(&mut manager, &two, &store).remove(0);
        if owned {
            let (_, refused) = manager
                .complete_tool_call_admitted(
                    &call.id,
                    ToolCallOutcome::Result(Value::from_json(&result)),
                )
                .unwrap();
            assert_eq!(refused, Some(ResultLimit::Snapshot(1024)));
        } else {
            complete(&mut manager, &call.id, &result);
        }
        assert!(!call.id.is_pending(), "owned {owned}");
        let snapshot = view(&manager);
        assert_eq!(snapshot["__openui_loading"], JsonValue::Array(Vec::new()));
        let errors = snapshot["__openui_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 2, "owned {owned}");
        assert_eq!(
            errors[0]["message"],
            query_error("t", ResultLimit::Snapshot(1024)).as_str()
        );
        assert_eq!(manager.cache_bytes(), 0);
    }
    // Six readers: even their errors do not fit, so one compact diagnostic
    // stands for all of them, the call ends and nothing is called again.
    let six = program(
        &(0..6)
            .map(|i| format!("q{i} = Query(\"t\")"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let mut manager = QueryManager::with_limits(limits);
    let call = evaluate(&mut manager, &six, &store).remove(0);
    complete(&mut manager, &call.id, &text(1025));
    assert!(!call.id.is_pending());
    let snapshot = view(&manager);
    let errors = snapshot["__openui_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0]["message"],
        "6 queries failed with errors that do not fit the result size limit of 1024 bytes"
    );
    assert!(manager.current_entry_is_served("q0", false, std::iter::empty()));
    assert!(evaluate(&mut manager, &six, &store).is_empty());
    // A run fetches again; a success clears the diagnostic.
    let again = manager
        .apply(QueryCommand::Invalidate { statement_ids: &[] })
        .unwrap()
        .calls;
    complete(&mut manager, &again[0].id, &JsonValue::from("ok"));
    assert_eq!(
        view(&manager)["__openui_errors"],
        JsonValue::Array(Vec::new())
    );
}

#[test]
fn a_refused_refresh_result_ends_its_call_and_the_refresh_schedule_continues() {
    let store = Store::default();
    let limits = Limits::DEFAULT.with_result_size(1024);
    let p = program("q = Query(\"t\", {}, null, 5)");
    for owned in [false, true] {
        let mut manager = QueryManager::with_limits(limits);
        let first = evaluate(&mut manager, &p, &store).remove(0);
        complete(&mut manager, &first.id, &JsonValue::from("fresh"));
        assert_eq!(manager.next_refresh_due_ms(), Some(5000));
        let tick = manager
            .apply(QueryCommand::AdvanceClock { now_ms: 5000 })
            .unwrap()
            .calls
            .remove(0);
        // The refreshed result is admitted as any result is.
        let update = if owned {
            manager
                .complete_tool_call(
                    &tick.id,
                    ToolCallOutcome::Result(Value::from_json(&text(1025))),
                )
                .unwrap()
        } else {
            manager
                .apply(QueryCommand::Complete {
                    id: &tick.id,
                    outcome: ToolOutcome::Result(Some(&text(1025))),
                })
                .unwrap()
        };
        assert!(!tick.id.is_pending(), "owned {owned}");
        let snapshot = view(&manager);
        assert_eq!(snapshot["q"], "fresh", "nothing held is changed");
        assert_eq!(
            snapshot["__openui_errors"][0]["message"],
            query_error("t", ResultLimit::Size(1024)).as_str()
        );
        // As after a failed call, the next tick refetches the failed entry.
        assert_eq!(update.next_refresh_due_ms, Some(10_000), "owned {owned}");
        let again = manager
            .apply(QueryCommand::AdvanceClock { now_ms: 10_000 })
            .unwrap()
            .calls;
        assert_eq!(again.len(), 1, "owned {owned}");
        complete(&mut manager, &again[0].id, &JsonValue::from("again"));
        let snapshot = view(&manager);
        assert_eq!(snapshot["q"], "again");
        assert_eq!(snapshot["__openui_errors"], JsonValue::Array(Vec::new()));
        check_charge(&manager);
    }
}
