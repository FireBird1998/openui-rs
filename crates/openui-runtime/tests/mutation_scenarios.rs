//! Independent mutation and query commands, including overlapping registrations.
use openui_lang::{JsonValue as Json, Library, Limits, parse, parse_json};
use openui_runtime::{
    QueryCommand, QueryCommandError, QueryManager, Store, ToolCallId, ToolOutcome, TreeInput, Value,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn replay(scenario: &Json) -> Vec<String> {
    let lib = Library::from_json_schema(&parse_json(r#"{"$defs":{}}"#).unwrap());
    let mut store = Store::new();
    let program = |source: &Json, store: &mut Store| {
        let p = parse(source.as_str().unwrap(), &lib, None);
        assert!(
            store
                .initialize(
                    p.state_declarations
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v))),
                    []
                )
                .refused
                .is_empty()
        );
        p
    };
    let mut parsed = program(&scenario["program"], &mut store);
    let mut manager = QueryManager::default();
    let mut foreign = QueryManager::default();
    let mut calls: Vec<ToolCallId> = Vec::new();
    let mut queries = Vec::new();
    let mut rows = Vec::new();
    for step in scenario["steps"].as_array().unwrap() {
        let before = manager.snapshot_json().to_owned();
        let update = match step["kind"].as_str().unwrap() {
            "evaluate" => {
                queries = parsed
                    .query_statements
                    .iter()
                    .map(|q| q.statement_id.clone())
                    .collect();
                Some(
                    manager
                        .apply(QueryCommand::Evaluate {
                            program: &parsed,
                            store: &store,
                        })
                        .unwrap(),
                )
            }
            "register" => Some(
                manager
                    .register_mutations(&TreeInput::new(&parsed), &store)
                    .unwrap(),
            ),
            "replace" => {
                parsed = program(&step["program"], &mut store);
                None
            }
            "set" => {
                store
                    .set(
                        step["name"].as_str().unwrap(),
                        step.get("value").map(Value::from_json),
                    )
                    .unwrap();
                None
            }
            "fire" => Some(
                manager
                    .fire_mutation(
                        step["name"].as_str().unwrap(),
                        step.get("args").map(Value::from_json).as_ref(),
                    )
                    .unwrap(),
            ),
            "complete" => {
                let id = &calls[step["call"].as_u64().unwrap() as usize];
                let o = &step["outcome"];
                let outcome = match o["kind"].as_str().unwrap() {
                    "result" => ToolOutcome::Result(o.get("value")),
                    "tool-not-found" => ToolOutcome::ToolNotFound(
                        o["availableTools"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap())
                            .collect(),
                    ),
                    "tool-error" => ToolOutcome::CallFailed(o["message"].as_str().unwrap()),
                    "mcp-error" => ToolOutcome::McpError(o["message"].as_str().unwrap()),
                    other => panic!("unknown outcome {other}"),
                };
                let other_before = foreign.snapshot_json().to_owned();
                assert!(matches!(
                    foreign.apply(QueryCommand::Complete {
                        id,
                        outcome: ToolOutcome::Result(None)
                    }),
                    Err(QueryCommandError::InvalidCall)
                ));
                assert_eq!(foreign.snapshot_json(), other_before);
                let applied = manager
                    .apply(QueryCommand::Complete { id, outcome })
                    .unwrap();
                let after = manager.snapshot_json().to_owned();
                assert!(matches!(
                    manager.apply(QueryCommand::Complete {
                        id,
                        outcome: ToolOutcome::Result(None)
                    }),
                    Err(QueryCommandError::InvalidCall)
                ));
                assert_eq!(manager.snapshot_json(), after);
                Some(applied)
            }
            other => panic!("unknown command {other}"),
        };
        let changed = update.as_ref().is_some_and(|u| u.changed);
        assert_eq!(changed, before != manager.snapshot_json());
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
            calls.push(call.id);
        }
        assert!(manager.cache_entry_count() <= 2 * queries.len());
        let snapshot = parse_json(manager.snapshot_json()).unwrap();
        for query in &queries {
            let error = snapshot["__openui_errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["source"] == "query" && e["statementId"] == query.as_str());
            assert!(manager.current_entry_is_served(
                query,
                error,
                calls.iter().filter(|id| id.is_pending())
            ));
        }
        rows.push(format!(
            "{{\"snapshot\":{},\"calls\":[{}],\"changed\":{changed}}}",
            manager.snapshot_json(),
            dispatch.join(",")
        ));
    }
    rows
}

#[test]
fn registration_submission_and_outcomes_match_the_upstream_manager() {
    let scenarios =
        parse_json(include_str!("../../../conformance/mutation-scenarios.json")).unwrap();
    let mut upstream = include_str!("../../../conformance/golden/mutation-scenarios.jsonl").lines();
    let pin = parse_json(include_str!(
        "../../../conformance/divergences/mutation-status-list-names.json"
    ))
    .unwrap();
    let mut used_pin = false;
    for scenario in scenarios.as_array().unwrap() {
        let reserved = scenario["name"] == pin["case"];
        let actual = replay(scenario);
        if reserved {
            assert!(!used_pin);
            used_pin = true;
            assert_eq!(pin["reference"].as_array().unwrap().len(), actual.len());
            assert_eq!(pin["rust"].as_array().unwrap().len(), actual.len());
            assert_ne!(
                pin["reference"], pin["rust"],
                "{} has converged",
                pin["case"]
            );
        }
        for (index, row) in actual.iter().enumerate() {
            let reference = upstream.next().unwrap();
            let expected = if reserved {
                assert_eq!(reference, pin["reference"][index].as_str().unwrap());
                pin["rust"][index].as_str().unwrap()
            } else {
                reference
            };
            assert_eq!(row, expected, "{}, step {index}", scenario["name"]);
        }
    }
    assert!(used_pin);
    assert!(upstream.next().is_none());
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn random_mutation_interleavings_match_upstream_after_every_command() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    let mut child = Command::new("bun")
        .args(["--no-install", "mutation-scenarios.ts", "--random"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut differences = 0usize;
    let mut count = 0;
    let mut steps = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let row = parse_json(&line.unwrap()).unwrap();
        let actual = replay(&row["scenario"]);
        let expected = row["reference"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        let mut query_pending = true;
        let mut previous = QueryManager::default().snapshot_json().to_owned();
        for (n, (actual, upstream)) in actual.iter().zip(expected).enumerate() {
            let step = &row["scenario"]["steps"][n];
            if step["kind"] == "complete" && step["call"] == 0 {
                query_pending = false;
            }
            let upstream = upstream.as_str().unwrap();
            let mut expected = parse_json(upstream).unwrap();
            // Rule 2: the loading list names current pending queries, including
            // when a mutation overwrites that property in the reference snapshot.
            if expected["snapshot"]["__openui_loading"].is_object() {
                expected["snapshot"]["__openui_loading"] = if query_pending {
                    parse_json(r#"["q"]"#).unwrap()
                } else {
                    Json::Array(vec![])
                };
            }
            let snapshot = expected["snapshot"].to_string();
            expected["changed"] = (snapshot != previous).into();
            previous = snapshot;
            assert_eq!(
                actual,
                &expected.to_string(),
                "seed {}, step {n}",
                row["scenario"]["name"]
            );
            if actual != upstream {
                assert!(
                    row["scenario"]["program"]
                        .as_str()
                        .unwrap()
                        .contains("__openui_loading=Mutation")
                );
                differences += 1;
            }
        }
        count += 1;
        steps += actual.len();
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 512);
    let report = format!("All {differences} differing observations follow the status-list row.");
    assert!(
        include_str!("../../../CONFORMANCE.md").contains(&report),
        "{report}"
    );
    eprintln!(
        "{count} scripts, {steps} observations, {differences} attributed to mutation-status-list-names"
    );
}

#[test]
fn refused_deep_result_preserves_loading_and_the_same_receipt_can_succeed() {
    let lib = Library::from_json_schema(&parse_json(r#"{"$defs":{}}"#).unwrap());
    let program = parse("m=Mutation(\"x\")", &lib, None);
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(12));
    manager
        .register_mutations(&TreeInput::new(&program), &Store::new())
        .unwrap();
    let mut deep = Json::Null;
    for _ in 0..128 {
        deep = Json::Array(vec![deep]);
    }
    let before = manager.snapshot_json().to_owned();
    assert!(matches!(
        manager.fire_mutation("m", Some(&Value::from_json(&deep))),
        Err(QueryCommandError::Limit(_))
    ));
    assert_eq!(manager.snapshot_json(), before);
    let id = manager.fire_mutation("m", None).unwrap().calls.remove(0).id;
    let loading = manager.snapshot_json().to_owned();
    assert!(matches!(
        manager.apply(QueryCommand::Complete {
            id: &id,
            outcome: ToolOutcome::Result(Some(&deep))
        }),
        Err(QueryCommandError::Limit(_))
    ));
    assert_eq!(manager.snapshot_json(), loading);
    assert!(id.is_pending());
    assert!(
        manager
            .apply(QueryCommand::Complete {
                id: &id,
                outcome: ToolOutcome::Result(None)
            })
            .unwrap()
            .changed
    );
    assert!(!id.is_pending());
}

#[test]
fn the_tree_reads_a_mutation_before_a_query_with_the_same_name() {
    let lib = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#).unwrap(),
    );
    let query = parse("m=Query(\"read\",{},42)\nroot=Box(m)", &lib, None);
    let mutation = parse("m=Mutation(\"save\")", &lib, None);
    let mut manager = QueryManager::default();
    let store = Store::new();
    manager
        .apply(QueryCommand::Evaluate {
            program: &query,
            store: &store,
        })
        .unwrap();
    manager
        .register_mutations(&TreeInput::new(&mutation), &store)
        .unwrap();
    let draw = |m: &QueryManager| {
        let refs = m.prepared_results(Limits::DEFAULT);
        let tree = openui_runtime::evaluate_tree(
            TreeInput::new(&query).with_prepared_reference_values(&refs),
            &store,
        );
        parse_json(&tree.to_json().unwrap()).unwrap()["root"]["props"]["value"].clone()
    };
    assert_eq!(draw(&manager), parse_json(r#"{"status":"idle"}"#).unwrap());
    manager
        .register_mutations(&TreeInput::new(&query), &store)
        .unwrap();
    assert_eq!(draw(&manager), 42);
}

#[test]
fn prototype_named_mutation_redraws_after_registration_and_completion() {
    let lib = Library::from_json_schema(&parse_json(r#"{"$defs":{}}"#).unwrap());
    let p = parse("__proto__=Mutation(\"save\")", &lib, None);
    let mut manager = QueryManager::default();
    let initial = manager.snapshot_json().to_owned();
    let u = manager
        .register_mutations(&TreeInput::new(&p), &Store::new())
        .unwrap();
    assert!(u.needs_redraw() && !u.changed);
    let mut u = manager.fire_mutation("__proto__", None).unwrap();
    assert!(u.needs_redraw() && !u.changed);
    let id = u.calls.remove(0).id;
    let u = manager
        .complete_tool_call(&id, openui_runtime::ToolCallOutcome::Result(Value::Null))
        .unwrap();
    assert!(u.needs_redraw() && !u.changed);
    assert_eq!(manager.snapshot_json(), initial);
}

#[test]
fn upstream_dispose_ends_both_late_outcomes_and_keeps_its_last_published_snapshot() {
    let rows = include_str!("../../../conformance/golden/mutation-dispose.jsonl")
        .lines()
        .map(|line| parse_json(line).unwrap())
        .collect::<Vec<_>>();
    let snapshot = parse_json(
        r#"{"__openui_loading":[],"__openui_refetching":[],"__openui_errors":[],"m":{"status":"loading"}}"#,
    ).unwrap();
    assert_eq!(rows.len(), 2);
    for (row, success) in rows.iter().zip([true, false]) {
        assert_eq!(row["success"], success);
        assert_eq!(row["before"], snapshot);
        assert_eq!(row["after"], snapshot);
        assert_eq!(row["resumed"], false);
        assert_eq!(row.get("result"), Some(&Json::Null));
        assert_eq!(row["fireAfterDispose"], false);
    }
}
