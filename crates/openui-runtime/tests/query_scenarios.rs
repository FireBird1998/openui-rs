//! Public query commands replay the reference manager's scripted observations.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    QueryCommand, QueryManager, Store, ToolCallId, ToolOutcome, Value, value_to_json,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn program(source: &JsonValue, library: &Library, store: &mut Store) -> ParseResult {
    let program = parse(source.as_str().unwrap(), library, None);
    let initialized = store.initialize(
        program
            .state_declarations
            .iter()
            .map(|(name, value)| (name.as_str(), Some(value))),
        [],
    );
    assert!(initialized.refused.is_empty());
    program
}

fn outcome(value: &JsonValue) -> ToolOutcome<'_> {
    match value["kind"].as_str().unwrap() {
        "result" => ToolOutcome::Result(value.get("value")),
        "tool-not-found" => ToolOutcome::ToolNotFound(
            value["availableTools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect(),
        ),
        "mcp-error" => ToolOutcome::McpError(value["message"].as_str().unwrap()),
        "tool-error" => ToolOutcome::ToolError(value["message"].as_str().unwrap()),
        kind => panic!("Unknown outcome {kind}"),
    }
}

#[test]
fn query_scenarios_match_upstream_after_every_step() {
    let scenarios = parse_json(include_str!("../../../conformance/query-scenarios.json")).unwrap();
    let goldens = parse_json(include_str!(
        "../../../conformance/golden/query-scenarios.json"
    ))
    .unwrap();
    let scenarios = scenarios.as_array().unwrap();
    assert_eq!(scenarios.len(), goldens.as_object().unwrap().len());
    let filter = std::env::var("OPENUI_QUERY_SCENARIO").ok();
    let mut matched = 0;
    for scenario in scenarios {
        let name = scenario["name"].as_str().unwrap();
        if filter.as_deref().is_some_and(|selected| selected != name) {
            continue;
        }
        matched += 1;
        let empty_library = parse_json("{\"$defs\":{}}").unwrap();
        let library = Library::from_json_schema(scenario.get("library").unwrap_or(&empty_library));
        let mut store = Store::new();
        let mut current = program(&scenario["program"], &library, &mut store);
        let mut manager = QueryManager::default();
        let mut calls: Vec<ToolCallId> = Vec::new();
        let steps = scenario["steps"].as_array().unwrap();
        let expected = goldens[name].as_array().unwrap();
        assert_eq!(steps.len(), expected.len(), "{name}");
        for (index, (step, expected)) in steps.iter().zip(expected).enumerate() {
            let command = match step["kind"].as_str().unwrap() {
                "evaluate" => Some(QueryCommand::Evaluate {
                    program: &current,
                    store: &store,
                }),
                "complete" => {
                    let call = usize::try_from(step["call"].as_u64().unwrap()).unwrap();
                    Some(QueryCommand::Complete {
                        id: &calls[call],
                        outcome: outcome(&step["outcome"]),
                    })
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
                "replace" => {
                    current = program(&step["program"], &library, &mut store);
                    None
                }
                kind => panic!("Unknown step {kind}"),
            };
            let update = command.map(|command| {
                manager
                    .apply(command)
                    .unwrap_or_else(|error| panic!("{name}, step {index}: {error}"))
            });
            let changed = update.as_ref().is_some_and(|update| update.changed);
            let mut actual = format!("{{\"snapshot\":{},\"calls\":[", manager.snapshot_json());
            for (offset, call) in update
                .into_iter()
                .flat_map(|update| update.calls)
                .enumerate()
            {
                use std::fmt::Write as _;
                if offset != 0 {
                    actual.push(',');
                }
                write!(
                    actual,
                    "{{\"call\":{},\"toolName\":{}",
                    calls.len(),
                    value_to_json(&Value::Str(call.tool_name)).unwrap()
                )
                .unwrap();
                if let Some(args) = call.args {
                    write!(actual, ",\"args\":{}", value_to_json(&args).unwrap()).unwrap();
                }
                actual.push('}');
                calls.push(call.id);
            }
            actual.push_str(if changed {
                "],\"changed\":true}"
            } else {
                "],\"changed\":false}"
            });
            assert_eq!(
                actual,
                expected.as_str().unwrap(),
                "{name}, step {index}: {step}"
            );
        }
    }
    assert!(matched > 0, "Unknown scenario filter {filter:?}");
}
