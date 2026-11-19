//! Limits and opaque call identities through the query manager's commands.
use openui_lang::{JsonValue, Library, ParseResult, free_json, parse};
use openui_runtime::{Limits, QueryCommand, QueryCommandError, QueryManager, Store, ToolOutcome};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn program(source: &str) -> ParseResult {
    parse(source, &Library::from_json_schema(&JsonValue::Null), None)
}

fn evaluate<'a>(program: &'a ParseResult, store: &'a Store) -> QueryCommand<'a> {
    QueryCommand::Evaluate { program, store }
}

#[test]
fn one_work_allowance_covers_all_query_statements() {
    let store = Store::default();
    let payload = "x".repeat(64);
    let first = format!("a = Query(\"read\", {{x: \"{payload}\"}})");
    let second = format!("b = Query(\"find\", {{x: \"{payload}\"}})");
    let one = program(&first);
    let two = program(&format!("{first}\n{second}"));
    // Start above the setup walk's work for two declarations, so the
    // refusal witnesses expression work even with evaluation made free.
    let minimum = (64..1024)
        .find(|&work| {
            QueryManager::with_limits(Limits::DEFAULT.with_work(work))
                .apply(evaluate(&one, &store))
                .is_ok()
        })
        .expect("a single query fits");
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(minimum));
    let empty = manager.snapshot_json().to_owned();
    assert!(matches!(
        manager.apply(evaluate(&two, &store)),
        Err(QueryCommandError::Limit(error)) if error.flags.size_limited
    ));
    assert_eq!(manager.snapshot_json(), empty);
    assert_eq!(
        manager.apply(evaluate(&one, &store)).unwrap().calls.len(),
        1
    );
}

#[test]
fn deep_query_expression_preserves_the_current_snapshot_and_pending_call() {
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(16));
    let before = program("q = Query(\"read\")");
    let start = manager.apply(evaluate(&before, &store)).unwrap();
    let pending = &start.calls.first().unwrap().id;
    let snapshot = manager.snapshot_json().to_owned();
    let deep = program(&format!(
        "q = Query(\"read\", {}1{})",
        "[".repeat(80),
        "]".repeat(80)
    ));
    assert!(matches!(
        manager.apply(evaluate(&deep, &store)),
        Err(QueryCommandError::Limit(error)) if error.flags.depth_limited
    ));
    assert_eq!(manager.snapshot_json(), snapshot);
    assert!(!manager.apply(evaluate(&before, &store)).unwrap().changed);
    assert!(
        manager
            .apply(QueryCommand::Complete {
                id: pending,
                outcome: ToolOutcome::Result(None),
            })
            .unwrap()
            .changed
    );
}

#[test]
fn deep_host_json_is_refused_before_consuming_the_call() {
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(16));
    let started = manager
        .apply(evaluate(&program("q = Query(\"read\")"), &store))
        .unwrap();
    let id = &started.calls.first().unwrap().id;
    let snapshot = manager.snapshot_json().to_owned();
    let mut result = JsonValue::Bool(true);
    for _ in 0..2048 {
        result = JsonValue::Array(vec![result]);
    }
    let refused = manager.apply(QueryCommand::Complete {
        id,
        outcome: ToolOutcome::Result(Some(&result)),
    });
    free_json(result);
    assert!(matches!(refused, Err(QueryCommandError::Limit(error)) if error.flags.depth_limited));
    assert_eq!(manager.snapshot_json(), snapshot);
    assert!(
        manager
            .apply(QueryCommand::Complete {
                id,
                outcome: ToolOutcome::Result(Some(&JsonValue::Bool(true))),
            })
            .unwrap()
            .changed
    );
}

#[test]
fn call_identities_are_owned_by_one_manager_and_complete_once() {
    let p = program("q = Query(\"read\")");
    let store = Store::default();
    let mut owner = QueryManager::default();
    let mut other = QueryManager::default();
    let started = owner.apply(evaluate(&p, &store)).unwrap();
    other.apply(evaluate(&p, &store)).unwrap();
    let id = &started.calls.first().unwrap().id;
    let complete = || QueryCommand::Complete {
        id,
        outcome: ToolOutcome::Result(None),
    };
    let before = other.snapshot_json().to_owned();
    assert!(matches!(
        other.apply(complete()),
        Err(QueryCommandError::InvalidCall)
    ));
    assert_eq!(other.snapshot_json(), before);
    assert!(owner.apply(complete()).unwrap().changed);
    let after = owner.snapshot_json().to_owned();
    assert!(matches!(
        owner.apply(complete()),
        Err(QueryCommandError::InvalidCall)
    ));
    assert_eq!(owner.snapshot_json(), after);
}

#[test]
fn consecutive_key_changes_release_the_displaced_result() {
    let program = program("$x = 0\nq = Query(\"t\", {v:$x}, \"d\")");
    let mut store = Store::default();
    let mut manager = QueryManager::default();
    let result = JsonValue::String("q".repeat(10_000));

    for cycle in 0..2_000 {
        let mut counts = Vec::new();
        let mut calls = std::collections::VecDeque::new();
        let mut observe = |manager: &QueryManager, calls: &std::collections::VecDeque<_>| {
            counts.push(manager.cache_entry_count());
            assert!(
                manager.current_entry_is_served("q", false, calls.iter()),
                "cycle {cycle}"
            );
        };
        for offset in 0..3 {
            store
                .set(
                    "$x",
                    Some(openui_runtime::Value::Num(f64::from(cycle * 3 + offset))),
                )
                .unwrap();
            let update = manager.apply(evaluate(&program, &store)).unwrap();
            assert_eq!(update.calls.len(), 1);
            calls.extend(update.calls.into_iter().map(|call| call.id));
            observe(&manager, &calls);
            if offset == 0 {
                manager
                    .apply(QueryCommand::Complete {
                        id: &calls.pop_front().unwrap(),
                        outcome: ToolOutcome::Result(Some(&result)),
                    })
                    .unwrap();
                observe(&manager, &calls);
            }
        }
        while let Some(id) = calls.pop_front() {
            manager
                .apply(QueryCommand::Complete {
                    id: &id,
                    outcome: ToolOutcome::Result(Some(&result)),
                })
                .unwrap();
            observe(&manager, &calls);
        }
        assert_eq!(
            counts,
            [if cycle == 0 { 1 } else { 2 }, 1, 2, 2, 2, 1],
            "cycle {cycle}"
        );
    }
}

#[test]
fn superseded_result_is_discarded_before_json_admission() {
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(16));
    let start = manager
        .apply(evaluate(&program("q = Query(\"first\")"), &store))
        .unwrap();
    let next = manager
        .apply(evaluate(&program("q = Query(\"next\")"), &store))
        .unwrap();
    let before = manager.snapshot_json().to_owned();
    let mut deep = JsonValue::Null;
    for _ in 0..64 {
        deep = JsonValue::Array(vec![deep]);
    }
    let id = &start.calls[0].id;
    let update = manager.apply(QueryCommand::Complete {
        id,
        outcome: ToolOutcome::Result(Some(&deep)),
    });
    free_json(deep);
    let update = update.unwrap();
    assert!(!update.changed && update.calls.is_empty());
    assert_eq!(manager.snapshot_json(), before);
    assert!(matches!(
        manager.apply(QueryCommand::Complete {
            id,
            outcome: ToolOutcome::Result(None)
        }),
        Err(QueryCommandError::InvalidCall)
    ));
    assert!(
        manager
            .apply(QueryCommand::Complete {
                id: &next.calls[0].id,
                outcome: ToolOutcome::Result(None)
            })
            .unwrap()
            .changed
    );
}

fn extra(values: &JsonValue) -> impl Iterator<Item = (&str, &JsonValue)> {
    values
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.as_str(), value))
}

#[test]
fn refused_extra_key_values_keep_the_previous_values() {
    let store = Store::default();
    let p = program("q = Query(\"read\")");
    let limits = Limits::DEFAULT.with_depth(16).with_result_size(4096);
    let mut manager = QueryManager::with_limits(limits);
    let first = openui_lang::parse_json("{\"account\": \"A\"}").unwrap();
    manager.set_extra_key_values("read", extra(&first)).unwrap();
    let started = manager.apply(evaluate(&p, &store)).unwrap();
    manager
        .apply(QueryCommand::Complete {
            id: &started.calls[0].id,
            outcome: ToolOutcome::Result(None),
        })
        .unwrap();

    let mut deep = JsonValue::Bool(true);
    for _ in 0..64 {
        deep = JsonValue::Array(vec![deep]);
    }
    let mut deep_values = JsonValue::Object(Default::default());
    deep_values["account"] = deep;
    let refused = manager.set_extra_key_values("read", extra(&deep_values));
    free_json(deep_values);
    assert!(matches!(refused, Err(QueryCommandError::Limit(error)) if error.flags.depth_limited));

    let mut large = JsonValue::Object(Default::default());
    large["account"] = JsonValue::String("x".repeat(4096));
    assert!(matches!(
        manager.set_extra_key_values("read", extra(&large)),
        Err(QueryCommandError::Limit(error)) if error.flags.size_limited
    ));
    // The cap covers every tool name's values together.
    let mut half = JsonValue::Object(Default::default());
    half["account"] = JsonValue::String("x".repeat(2500));
    manager.set_extra_key_values("other", extra(&half)).unwrap();
    assert!(matches!(
        manager.set_extra_key_values("read", extra(&half)),
        Err(QueryCommandError::Limit(error)) if error.flags.size_limited
    ));
    // Replacing a tool name's values does not count the old ones.
    manager.set_extra_key_values("other", extra(&half)).unwrap();

    let update = manager.apply(evaluate(&p, &store)).unwrap();
    assert!(update.calls.is_empty() && !update.changed);
    manager
        .set_extra_key_values("other", std::iter::empty())
        .unwrap();
    manager.set_extra_key_values("read", extra(&half)).unwrap();
    let second = openui_lang::parse_json("{\"account\": \"B\"}").unwrap();
    manager
        .set_extra_key_values("read", extra(&second))
        .unwrap();
    assert_eq!(manager.apply(evaluate(&p, &store)).unwrap().calls.len(), 1);
}

#[test]
fn extra_key_values_are_charged_to_the_evaluation_work() {
    let store = Store::default();
    let p = program("q = Query(\"read\")");
    let minimum = (0..4096)
        .find(|&work| {
            QueryManager::with_limits(Limits::DEFAULT.with_work(work))
                .apply(evaluate(&p, &store))
                .is_ok()
        })
        .expect("the query fits");
    // Room to admit the values, not to evaluate the query with them.
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(minimum + 1000));
    let mut values = JsonValue::Object(Default::default());
    values["account"] = JsonValue::String("x".repeat(400));
    manager
        .set_extra_key_values("read", extra(&values))
        .unwrap();
    assert!(matches!(
        manager.apply(evaluate(&p, &store)),
        Err(QueryCommandError::Limit(error)) if error.flags.size_limited
    ));
    manager
        .set_extra_key_values("read", std::iter::empty())
        .unwrap();
    assert_eq!(manager.apply(evaluate(&p, &store)).unwrap().calls.len(), 1);
}

#[test]
fn a_repeated_extra_key_name_keeps_its_last_value() {
    let store = Store::default();
    let p = program("q = Query(\"read\")");
    let (first, last) = (JsonValue::from("A"), JsonValue::from("B"));
    let mut manager = QueryManager::default();
    manager
        .set_extra_key_values("read", [("account", &last)])
        .unwrap();
    let started = manager.apply(evaluate(&p, &store)).unwrap();
    manager
        .apply(QueryCommand::Complete {
            id: &started.calls[0].id,
            outcome: ToolOutcome::Result(None),
        })
        .unwrap();
    manager
        .set_extra_key_values("read", [("account", &first), ("account", &last)])
        .unwrap();
    assert!(
        manager
            .apply(evaluate(&p, &store))
            .unwrap()
            .calls
            .is_empty()
    );
    manager
        .set_extra_key_values("read", [("account", &last), ("account", &first)])
        .unwrap();
    assert_eq!(manager.apply(evaluate(&p, &store)).unwrap().calls.len(), 1);
}

fn nested(levels: usize) -> JsonValue {
    let mut value = JsonValue::Null;
    for _ in 0..levels {
        value = JsonValue::Array(vec![value]);
    }
    value
}

#[test]
fn extra_key_values_are_refused_at_the_depth_a_tool_result_is() {
    let store = Store::default();
    let p = program("q = Query(\"read\")");
    let limits = Limits::DEFAULT.with_depth(16);
    for (levels, admitted) in [(16, true), (17, false)] {
        let value = nested(levels);
        let mut manager = QueryManager::with_limits(limits);
        let extra = manager.set_extra_key_values("read", [("x", &value)]);
        let started = manager.apply(evaluate(&p, &store)).unwrap();
        let result = manager.apply(QueryCommand::Complete {
            id: &started.calls[0].id,
            outcome: ToolOutcome::Result(Some(&value)),
        });
        assert_eq!(extra.is_ok(), admitted, "{levels} levels as an extra value");
        assert_eq!(result.is_ok(), admitted, "{levels} levels as a tool result");
        if let Err(error) = extra {
            assert!(matches!(error, QueryCommandError::Limit(e) if e.flags.depth_limited));
        }
    }
}

#[test]
fn extra_key_values_are_charged_as_one_json_object_of_records() {
    // {"t":{"a":"…"}} is the text plus 14 bytes.
    let limits = Limits::DEFAULT.with_result_size(128);
    let fits = JsonValue::String("x".repeat(114));
    let over = JsonValue::String("x".repeat(115));
    let mut manager = QueryManager::with_limits(limits);
    manager.set_extra_key_values("t", [("a", &fits)]).unwrap();
    assert!(matches!(
        manager.set_extra_key_values("t", [("a", &over)]),
        Err(QueryCommandError::Limit(error)) if error.flags.size_limited
    ));
    // Escapes count as written: each "\0" is six bytes, so 60 of them do not fit.
    let one = JsonValue::from(1);
    let escaped = "\0".repeat(60);
    let mut manager = QueryManager::with_limits(limits);
    assert!(
        manager
            .set_extra_key_values("a", [(escaped.as_str(), &one)])
            .is_err()
    );
    // Another tool's member and the comma between them count too:
    // {"a":{"k":"…"},"b":{"k":1}} is the text plus 26 bytes.
    let mut manager = QueryManager::with_limits(limits);
    manager.set_extra_key_values("b", [("k", &one)]).unwrap();
    let shared = JsonValue::String("x".repeat(102));
    manager.set_extra_key_values("a", [("k", &shared)]).unwrap();
    let shared = JsonValue::String("x".repeat(103));
    assert!(manager.set_extra_key_values("a", [("k", &shared)]).is_err());
}

#[test]
fn refusing_large_extra_key_values_copies_nothing() {
    let limits = Limits::DEFAULT.with_result_size(4096);
    let nulls = JsonValue::Array(vec![JsonValue::Null; 8192]);
    let mut manager = QueryManager::with_limits(limits);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let refused = manager.set_extra_key_values("t", [("k", &nulls)]);
    let extra = test_alloc::peak().saturating_sub(before);
    assert!(matches!(refused, Err(QueryCommandError::Limit(error)) if error.flags.size_limited));
    assert!(extra < 1024, "{extra} bytes allocated to refuse");
}
