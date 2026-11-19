//! Timed commands compared with the upstream manager and its platform timer queue.
use openui_lang::{JsonValue as Json, Library, Limits, parse, parse_json};
use openui_runtime::{
    QueryCommand, QueryCommandError, QueryManager, Store, ToolCallId, ToolOutcome,
};
use std::process::Command;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn receipt_is_refused(manager: &mut QueryManager, id: &ToolCallId) {
    let before = (
        manager.snapshot_json().to_owned(),
        manager.next_refresh_due_ms(),
        manager.cache_entry_count(),
    );
    assert!(matches!(
        manager.apply(QueryCommand::Complete {
            id,
            outcome: ToolOutcome::Result(None)
        }),
        Err(QueryCommandError::InvalidCall)
    ));
    assert_eq!(
        before,
        (
            manager.snapshot_json().to_owned(),
            manager.next_refresh_due_ms(),
            manager.cache_entry_count()
        )
    );
}

fn replay(scenario: &Json, expected: &Json) -> Vec<String> {
    let library = Library::from_json_schema(&Json::Null);
    let program = |text: &Json, store: &mut Store| {
        let parsed = parse(text.as_str().unwrap(), &library, None);
        assert!(
            store
                .initialize(
                    parsed
                        .state_declarations
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v))),
                    []
                )
                .refused
                .is_empty()
        );
        parsed
    };
    let mut store = Store::new();
    let mut parsed = program(&scenario["program"], &mut store);
    let mut manager = scenario["minimumMs"]
        .as_u64()
        .map_or_else(QueryManager::default, |minimum| {
            QueryManager::with_refresh_minimum(Limits::DEFAULT, minimum).unwrap()
        });
    let mut foreign = QueryManager::default();
    let mut calls: Vec<ToolCallId> = Vec::new();
    let mut registered = Vec::new();
    let mut evaluated = false;
    let mut rows = Vec::new();
    for (index, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        let before = manager.snapshot_json().to_owned();
        let ids: Vec<_> = step["ids"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| s.as_str().unwrap())
            .collect();
        let command = match step["kind"].as_str().unwrap() {
            "set" => {
                store
                    .set(
                        step["name"].as_str().unwrap(),
                        step.get("value").map(openui_runtime::Value::from_json),
                    )
                    .unwrap();
                None
            }
            "register" | "fire" => None,
            "replace" => {
                parsed = program(&step["program"], &mut store);
                None
            }
            "evaluate" if step["blocked"] == true => None,
            "evaluate" => {
                if !manager.is_disposed() {
                    registered = parsed
                        .query_statements
                        .iter()
                        .filter(|q| q.complete)
                        .map(|q| q.statement_id.clone())
                        .collect();
                    evaluated = true;
                }
                Some(QueryCommand::Evaluate {
                    program: &parsed,
                    store: &store,
                })
            }
            "advance" => Some(QueryCommand::AdvanceClock {
                now_ms: step["now"].as_u64().unwrap(),
            }),
            "dispose" => {
                evaluated = false;
                Some(QueryCommand::Dispose)
            }
            "activate" => Some(QueryCommand::Activate),
            "invalidate" => Some(QueryCommand::Invalidate {
                statement_ids: &ids,
            }),
            "complete" => {
                let id = &calls[step["call"].as_u64().unwrap() as usize];
                receipt_is_refused(&mut foreign, id);
                let outcome = &step["outcome"];
                Some(QueryCommand::Complete {
                    id,
                    outcome: match outcome["kind"].as_str().unwrap() {
                        "result" => ToolOutcome::Result(outcome.get("value")),
                        "tool-error" => {
                            ToolOutcome::CallFailed(outcome["message"].as_str().unwrap())
                        }
                        other => panic!("unknown outcome {other}"),
                    },
                })
            }
            other => panic!("unknown command {other}"),
        };
        let available = (step["kind"] == "fire")
            .then(|| manager.can_fire_mutation(step["name"].as_str().unwrap()));
        let update = match step["kind"].as_str() {
            Some("register") => Some(
                manager
                    .register_mutations(&openui_runtime::TreeInput::new(&parsed), &store)
                    .unwrap(),
            ),
            Some("fire") => Some(
                manager
                    .fire_mutation(
                        step["name"].as_str().unwrap(),
                        Some(&openui_runtime::Value::from_json(&step["args"])),
                    )
                    .unwrap(),
            ),
            _ => command.map(|c| manager.apply(c).unwrap()),
        };
        if let Some(can_fire) = available {
            assert_eq!(
                can_fire,
                update.as_ref().is_some_and(|u| !u.calls.is_empty())
            );
        }
        let changed = before != manager.snapshot_json();
        assert_eq!(changed, update.as_ref().is_some_and(|u| u.changed));
        if let Some(update) = &update {
            let row = parse_json(expected[index].as_str().unwrap()).unwrap();
            assert_eq!(
                Json::from(update.next_refresh_due_ms),
                row["nextDue"],
                "{} step {index}",
                scenario["name"]
            );
        }
        if step["kind"] == "complete" {
            receipt_is_refused(
                &mut manager,
                &calls[step["call"].as_u64().unwrap() as usize],
            );
        }
        let mut dispatch = Vec::new();
        for call in update.into_iter().flat_map(|u| u.calls) {
            let args = call
                .arguments_json()
                .map(|v| format!(",\"args\":{v}"))
                .unwrap_or_default();
            dispatch.push(format!(
                "{{\"call\":{},\"toolName\":{}{args}}}",
                calls.len(),
                Json::String(call.tool_name)
            ));
            assert!(call.id.is_pending());
            calls.push(call.id);
        }
        assert!(manager.cache_entry_count() <= 2 * registered.len());
        if !evaluated {
            assert!(manager.next_refresh_due_ms().is_none());
            if manager.is_disposed() {
                assert!(calls.iter().all(|id| !id.is_pending()));
            }
        } else {
            let snapshot = parse_json(manager.snapshot_json()).unwrap();
            for id in &registered {
                let error = snapshot["__openui_errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["statementId"].as_str() == Some(id));
                assert!(
                    manager.current_entry_is_served(
                        id,
                        error,
                        calls.iter().filter(|c| c.is_pending())
                    ),
                    "{id} is starved after {step}"
                );
            }
        }
        let due = Json::from(manager.next_refresh_due_ms());
        rows.push(format!(
            "{{\"snapshot\":{},\"calls\":[{}],\"changed\":{changed},\"nextDue\":{due}}}",
            manager.snapshot_json(),
            dispatch.join(",")
        ));
    }
    rows
}

#[test]
fn refresh_deadlines_dispatch_and_lifecycle_match_the_scripted_manager() {
    let scenarios = parse_json(include_str!("../../../conformance/query-refresh.json")).unwrap();
    let gold = parse_json(include_str!(
        "../../../conformance/golden/query-refresh.json"
    ))
    .unwrap();
    let pins = parse_json(include_str!(
        "../../../conformance/divergences/refresh-lifecycle.json"
    ))
    .unwrap();
    for scenario in scenarios.as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let expected = pins.get(name).map_or(&gold[name], |pin| &pin["rust"]);
        let actual = Json::from(replay(scenario, expected));
        if let Some(pin) = pins.get(name) {
            assert_eq!(pin["reference"], gold[name], "{name} reference");
            assert_ne!(pin["reference"], pin["rust"], "{name} has converged");
            assert_eq!(actual, pin["rust"], "{name}");
        } else {
            assert_eq!(actual, gold[name], "{name}");
        }
    }
}

#[test]
#[ignore = "seeded timed scripts against Bun"]
fn seeded_clock_edits_skipped_ticks_and_reattach_match_upstream() {
    let output = Command::new("bun")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .args(["--no-install", "query-refresh.ts", "--random"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut count = 0;
    let mut steps = 0;
    let mut differences = std::collections::BTreeMap::<String, usize>::new();
    for line in std::str::from_utf8(&output.stdout).unwrap().lines() {
        let row = parse_json(line).unwrap();
        let mut expected = row.get("policy").unwrap_or(&row["reference"]).clone();
        if let Some(overrides) = row["overrides"].as_object() {
            for (index, bytes) in overrides {
                expected[index.parse::<usize>().unwrap()] = bytes.clone();
            }
        }
        let actual = replay(&row["scenario"], &expected);
        steps += actual.len();
        assert_eq!(actual.len(), expected.as_array().unwrap().len());
        for (i, bytes) in actual.iter().enumerate() {
            assert_eq!(
                Some(bytes.as_str()),
                expected[i].as_str(),
                "{} step {i}",
                row["scenario"]["name"]
            );
            if row["reference"][i].as_str() != Some(bytes) {
                let reason = row["reason"].as_str().unwrap();
                assert!(matches!(
                    reason,
                    "infinite-refresh-is-disabled"
                        | "configured-refresh-floor"
                        | "dispose-publishes-loading"
                ));
                *differences.entry(reason.to_owned()).or_default() += 1;
            }
        }
        count += 1;
    }
    assert_eq!(count, 640);
    assert_eq!(steps, 38_941);
    assert_eq!(differences.get("configured-refresh-floor"), Some(&1214));
    assert_eq!(differences.get("dispose-publishes-loading"), Some(&6894));
    assert_eq!(differences.get("infinite-refresh-is-disabled"), Some(&1216));
    eprintln!("{count} scripts, {steps} observations, {differences:?}");
}

#[test]
fn invalid_clock_commands_preserve_phase_and_large_advances_skip_whole_periods() {
    for minimum in [0, i32::MAX as u64 + 1] {
        assert!(matches!(
            QueryManager::with_refresh_minimum(Limits::DEFAULT, minimum),
            Err(QueryCommandError::InvalidClock)
        ));
    }
    let mut m = QueryManager::with_refresh_minimum(Limits::DEFAULT, 1).unwrap();
    let p = parse(
        "q=Query(\"read\",{},null,0.001)",
        &Library::from_json_schema(&Json::Null),
        None,
    );
    let first = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &Store::new(),
        })
        .unwrap()
        .calls
        .remove(0);
    let last = u64::MAX - i32::MAX as u64;
    let update = m
        .apply(QueryCommand::AdvanceClock { now_ms: last })
        .unwrap();
    assert!(update.calls.is_empty());
    assert_eq!(update.next_refresh_due_ms, Some(last + 1));
    let before = m.snapshot_json().to_owned();
    for invalid in [0, u64::MAX] {
        assert!(matches!(
            m.apply(QueryCommand::AdvanceClock { now_ms: invalid }),
            Err(QueryCommandError::InvalidClock)
        ));
        assert_eq!(before, m.snapshot_json());
        assert_eq!(m.next_refresh_due_ms(), Some(last + 1));
        assert!(first.id.is_pending());
    }
}
