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
        let mut calls = Vec::new();
        for offset in 0..3 {
            store
                .set(
                    "$x",
                    Some(openui_runtime::Value::Num(f64::from(cycle * 3 + offset))),
                )
                .unwrap();
            let update = manager.apply(evaluate(&program, &store)).unwrap();
            counts.push(manager.cache_entry_count());
            assert_eq!(update.calls.len(), 1);
            calls.extend(update.calls.into_iter().map(|call| call.id));
            if offset == 0 {
                manager
                    .apply(QueryCommand::Complete {
                        id: &calls[0],
                        outcome: ToolOutcome::Result(Some(&result)),
                    })
                    .unwrap();
                counts.push(manager.cache_entry_count());
            }
        }
        for id in &calls[1..] {
            manager
                .apply(QueryCommand::Complete {
                    id,
                    outcome: ToolOutcome::Result(Some(&result)),
                })
                .unwrap();
            counts.push(manager.cache_entry_count());
        }
        assert_eq!(
            counts,
            [if cycle == 0 { 1 } else { 2 }, 1, 2, 2, 2, 1],
            "cycle {cycle}"
        );
    }
}
