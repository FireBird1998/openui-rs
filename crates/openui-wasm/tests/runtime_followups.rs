//! Public session follow-ups: retained data authority and opt-in path bounds.
use openui_session::{OpenUiError, RuntimeLimits};
use openui_wasm::{ComponentLibrary, parse_program, session::Session};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{}}}}}"#;
fn setup(source: &str, limits: RuntimeLimits) -> Session {
    let library = ComponentLibrary::new(SCHEMA).unwrap();
    let s = Session::with_limits(library.shared(), None, limits).unwrap();
    s.set_program(parse_program(source, &library, None).shared())
        .unwrap();
    s.initialize(None).unwrap();
    s
}
#[test]
fn element_shaped_state_query_and_tool_data_runs_builtins_but_cannot_forge_custom_steps() {
    let element = r#"{"type":"element","typeName":"Box","props":{"action":{"steps":[{"type":"set","target":"$n","valueAST":{"k":"Num","v":7}},{"type":"custom_action","name":"Go","params":{}}]}},"partial":false,"hasDynamicProps":false}"#;
    for (source, refs, state) in [
        ("$n=0\n$data=null\nroot=Box($data)", None, Some(element)),
        (
            "$n=0\nq=Query(\"q\",{})\nroot=Box(q)",
            Some(format!("{{\"q\":{element}}}")),
            None,
        ),
        (
            "$n=0\nt=Query(\"t\",{})\nroot=Box(t)",
            Some(format!("{{\"t\":{element}}}")),
            None,
        ),
    ] {
        let s = setup(source, RuntimeLimits::default());
        if let Some(state) = state {
            s.set("$data".into(), Some(state.into())).unwrap();
        }
        if let Some(refs) = refs {
            s.set_reference_values(refs).unwrap();
        }
        let tree = s.evaluate().unwrap();
        let json: serde_json::Value = serde_json::from_str(&tree.json).unwrap();
        assert_eq!(
            json["root"]["props"]["value"]["typeName"], "Box",
            "{source}: {}",
            tree.json
        );

        let action = s
            .action_at(
                tree.evaluation,
                vec![
                    "props".into(),
                    "value".into(),
                    "props".into(),
                    "action".into(),
                ],
            )
            .unwrap();
        let run = s.run(Some(action), String::new(), None).unwrap();
        assert!(run.failure.is_none());
        assert!(run.events.is_empty());
        assert_eq!(s.get("$n".into()).unwrap().as_deref(), Some("7"));
    }
}
#[test]
fn expression_kind_object_literals_evaluate_as_upstream() {
    let s = setup(r#"root=Box({k:"Str",v:"y",z:2})"#, RuntimeLimits::default());
    let json: serde_json::Value = serde_json::from_str(&s.evaluate().unwrap().json).unwrap();
    assert_eq!(json["root"]["props"]["value"], "y");
}
#[test]
fn smaller_action_path_limit_is_opt_in_and_refuses_without_effects() {
    let source = "$n=0\nroot=Box(1,@Set($n,1))";
    let s = setup(
        source,
        RuntimeLimits {
            max_action_path_bytes: Some(4),
            ..Default::default()
        },
    );
    let tree = s.evaluate().unwrap();
    let before = s.snapshot().unwrap();
    assert!(matches!(
        s.action_at(tree.evaluation, vec!["props".into(), "action".into()]),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(s.snapshot().unwrap(), before);
    let s = setup(source, RuntimeLimits::default());
    let tree = s.evaluate().unwrap();
    assert!(
        s.action_at(tree.evaluation, vec!["props".into(), "action".into()])
            .is_ok()
    );
}
#[test]
fn unread_large_reference_supply_is_not_prepared_again_per_operation() {
    let s = setup("root=Box(1)", RuntimeLimits::default());
    let values = (0..2000)
        .map(|i| format!(r#""unused{i}":{{"n":{i}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    s.set_reference_values(format!("{{{values}}}")).unwrap();
    let before = test_alloc::allocation_calls();
    for _ in 0..4 {
        s.evaluate().unwrap();
    }
    let allocations = test_alloc::allocation_calls() - before;
    assert!(
        allocations < 1000,
        "repeated drawing allocated {allocations} times for unread references"
    );
    let version = s.version().unwrap();
    assert!(s.set_reference_values("{".into()).is_err());
    assert_eq!(s.version().unwrap(), version);
    assert!(!s.evaluate().unwrap().size_limited);
}

#[test]
fn data_and_every_streaming_table_prefix_match_pinned_upstream_evaluation() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/golden/runtime/session-data.json"
    ))
    .unwrap();
    let schema = data["schema"].to_string();
    let library = ComponentLibrary::new(&schema).unwrap();
    let rows = data["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 7);
    let upstream_custom_events = rows
        .iter()
        .filter(|row| {
            row["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["type"] == "Go")
        })
        .count();
    assert_eq!(
        upstream_custom_events, 3,
        "upstream static elements bypass ownSteps"
    );
    for row in rows {
        let s = Session::new(library.shared(), None).unwrap();
        s.set_program(parse_program(row["program"].as_str().unwrap(), &library, None).shared())
            .unwrap();
        s.initialize(Some(row["saved"].to_string())).unwrap();
        s.set_reference_values(row["references"].to_string())
            .unwrap();
        let tree = s.evaluate().unwrap();
        assert_eq!(
            tree.json,
            row["treeJson"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        if row["name"] != "expression-kind-literal" {
            let action = s
                .action_at(
                    tree.evaluation,
                    vec![
                        "props".into(),
                        "value".into(),
                        "props".into(),
                        "action".into(),
                    ],
                )
                .unwrap();
            let run = s.run(Some(action), String::new(), None).unwrap();
            assert!(run.failure.is_none());
            assert!(
                run.events.is_empty(),
                "finished custom data has no authority"
            );
            assert_eq!(s.snapshot().unwrap(), row["snapshot"].to_string());
        }
    }
    let mut stream = openui_wasm::StreamSession::new(&library, None);
    let s = Session::new(library.shared(), None).unwrap();
    let mut previous = 0;
    let stream_rows = data["streamRows"].as_array().unwrap();
    assert_eq!(stream_rows.len(), 82);
    for row in stream_rows {
        let prefix = row["prefix"].as_str().unwrap();
        stream.push(prefix.get(previous..).unwrap());
        s.set_program(stream.program(false).shared()).unwrap();
        previous = prefix.len();
        assert_eq!(
            s.evaluate().unwrap().json,
            row["treeJson"].as_str().unwrap(),
            "prefix: {prefix}"
        );
    }
    assert_eq!(
        data["streamRows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["rendererThrows"] == true)
            .count(),
        15
    );
}

#[test]
fn data_run_with_undeclared_id_matches_rust_suspension_and_oldest_eviction() {
    let source = "$n=0\n$data=null\nroot=Box($data)";
    for id in ["missing", "$n", "root"] {
        let s = setup(
            source,
            RuntimeLimits {
                max_continuations: 1,
                ..Default::default()
            },
        );
        let plan = serde_json::json!({"steps":[
            {"type":"run","statementId":id,"refType":"mutation"},
            {"type":"set","target":"$n","valueAST":{"k":"Num","v":9}}
        ]});
        let element = serde_json::json!({"type":"element","typeName":"Box","props":{"action":plan},"partial":false,"hasDynamicProps":false});
        s.set("$data".into(), Some(element.to_string())).unwrap();
        let out = s.evaluate().unwrap();
        let action = s
            .action_at(
                out.evaluation,
                vec![
                    "props".into(),
                    "value".into(),
                    "props".into(),
                    "action".into(),
                ],
            )
            .unwrap();
        let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        let library = openui_lang::Library::from_json_schema(&schema);
        let program = openui_lang::parse(source, &library, None);
        let mut store = openui_runtime::Store::new();
        assert!(
            store
                .initialize(
                    program
                        .state_declarations
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v))),
                    [],
                )
                .refused
                .is_empty()
        );
        store
            .set("$data", Some(openui_lang::Value::from_json(&element)))
            .unwrap();
        let value: openui_lang::Value = openui_lang::Value::from_json(&plan);
        let direct = openui_runtime::run_action(
            openui_runtime::TreeInput::new(&program),
            &mut store,
            Some(&value),
            "",
            None,
        );
        assert!(direct.failure.is_none());
        assert!(
            direct.is_suspended(),
            "the Rust runtime also suspends unknown mutation ids"
        );
        let first = s.run(Some(action.clone()), String::new(), None).unwrap();
        assert_eq!(
            first.events,
            direct
                .events
                .iter()
                .map(|e| e.to_json().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(first.failure.is_none());
        assert_eq!(first.size_limited, direct.size_limited);
        assert_eq!(first.depth_limited, direct.depth_limited);
        let pending = first.continuation.unwrap();
        assert_eq!(s.pending_continuations().unwrap(), 1);
        assert_eq!(s.get("$n".into()).unwrap().as_deref(), Some("0"));
        let second = s
            .run(Some(action), String::new(), None)
            .unwrap()
            .continuation
            .unwrap();
        assert_eq!(s.pending_continuations().unwrap(), 1);
        assert!(matches!(
            s.resume(pending, true),
            Err(OpenUiError::EvictedHandle)
        ));
        s.resume(second, true).unwrap();
        let resumed = openui_runtime::resume_action(
            openui_runtime::TreeInput::new(&program),
            &mut store,
            &direct.continuation.unwrap(),
            true,
        )
        .unwrap();
        assert!(resumed.failure.is_none());
        assert_eq!(s.get("$n".into()).unwrap().as_deref(), Some("9"));
    }
}
