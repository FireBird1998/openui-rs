//! Tool result limits (#90) replayed beside lang-core's own query manager.
//! Upstream has no limits, so a scenario at a limit matches its golden byte
//! for byte and a scenario one byte or level over is a pinned divergence,
//! each listed in CONFORMANCE.md's table of result limit differences.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    Limits, QueryCommand, QueryManager, Store, ToolCallId, ToolOutcome, TreeInput, Value,
};
use std::collections::{BTreeMap, BTreeSet};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn program(source: &JsonValue, library: &Library, store: &mut Store) -> ParseResult {
    let program = parse(source.as_str().unwrap(), library, None);
    let declarations = program
        .state_declarations
        .iter()
        .map(|(name, value)| (name.as_str(), Some(value)));
    assert!(store.initialize(declarations, []).refused.is_empty());
    program
}

fn manager(limits: Option<&JsonValue>) -> QueryManager {
    let field = |name: &str| {
        limits
            .and_then(|limits| limits.get(name))
            .and_then(JsonValue::as_u64)
            .map(|n| usize::try_from(n).unwrap())
    };
    let mut core = Limits::DEFAULT;
    if let Some(bytes) = field("resultSize") {
        core = core.with_result_size(bytes);
    }
    if let Some(levels) = field("depth") {
        core = core.with_depth(levels);
    }
    let manager = QueryManager::with_limits(core);
    match field("cacheBytes") {
        Some(bytes) => manager.with_cache_limit(bytes),
        None => manager,
    }
}

fn replay(scenario: &JsonValue) -> Vec<String> {
    let library = Library::from_json_schema(&parse_json(r#"{"$defs":{}}"#).unwrap());
    let mut store = Store::new();
    let mut current = program(&scenario["program"], &library, &mut store);
    let mut manager = manager(scenario.get("limits"));
    let mut calls: Vec<ToolCallId> = Vec::new();
    let mut rows = Vec::new();
    let mut dispatches = Vec::new();
    for (index, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        let before = manager.snapshot_json().to_owned();
        let ids: Vec<&str> = step
            .get("ids")
            .and_then(JsonValue::as_array)
            .map(|ids| ids.iter().map(|id| id.as_str().unwrap()).collect())
            .unwrap_or_default();
        let completed = step
            .get("call")
            .map(|n| n.as_u64().unwrap() as usize)
            .or_else(|| {
                let target = step.get("dispatch")?;
                Some(
                    dispatches
                        .iter()
                        .position(|address| address == target)
                        .expect("expected dispatch"),
                )
            });
        let update = match step["kind"].as_str().unwrap() {
            "evaluate" => Some(manager.apply(QueryCommand::Evaluate {
                program: &current,
                store: &store,
            })),
            "register" => Some(manager.register_mutations(&TreeInput::new(&current), &store)),
            "fire" => Some(manager.fire_mutation(
                step["name"].as_str().unwrap(),
                step.get("args").map(Value::from_json).as_ref(),
            )),
            "complete" => {
                let id = &calls[completed.unwrap()];
                let outcome = &step["outcome"];
                let nan = Value::Num(f64::NAN);
                assert_eq!(outcome["kind"], "result");
                Some(manager.apply(QueryCommand::Complete {
                    id,
                    outcome: if outcome["hostNumber"] == "NaN" {
                        ToolOutcome::ResultValue(&nan)
                    } else {
                        ToolOutcome::Result(outcome.get("value"))
                    },
                }))
            }
            "set" => {
                let value = step.get("value").map(Value::from_json);
                store.set(step["name"].as_str().unwrap(), value).unwrap();
                None
            }
            "replace" => {
                current = program(&step["program"], &library, &mut store);
                None
            }
            "invalidate" => Some(manager.apply(QueryCommand::Invalidate {
                statement_ids: &ids,
            })),
            kind => panic!("unknown step {kind}"),
        };
        // Every limit here ends in a delivered tool-error, never a refusal.
        let update = update.transpose().unwrap();
        let cut = scenario["roundCuts"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|n| n.as_u64() == Some(index as u64)));
        assert_eq!(
            update.as_ref().is_some_and(|u| u.reader_rounds_limited),
            cut,
            "{}, step {index}",
            scenario["name"]
        );
        assert_eq!(
            matches!(
                manager.reader_diagnostic(),
                Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
            ),
            scenario["propagationCut"].as_u64() == Some(index as u64),
            "{}, step {index}: propagation diagnostic",
            scenario["name"]
        );
        if let Some(completed) = completed {
            assert!(
                manager.check_call(&calls[completed]).is_err(),
                "a delivered receipt must be consumed"
            );
        }
        let changed = update.as_ref().is_some_and(|update| update.changed);
        assert_eq!(changed, before != manager.snapshot_json());
        let mut dispatched = Vec::new();
        for call in update.into_iter().flat_map(|update| update.calls) {
            let args = call
                .arguments_json()
                .map(|args| format!(",\"args\":{args}"))
                .unwrap_or_default();
            dispatched.push(format!(
                "{{\"call\":{},\"toolName\":{}{args}}}",
                calls.len(),
                JsonValue::from(call.tool_name.as_str())
            ));
            dispatches.push(
                parse_json(&format!(
                    "{{\"step\":{index},\"toolName\":{}{args}}}",
                    JsonValue::from(call.tool_name)
                ))
                .unwrap(),
            );
            calls.push(call.id);
        }
        assert_eq!(manager.cache_bytes(), manager.measured_cache_bytes());
        assert!(manager.cache_bytes() <= manager.cache_limit());
        rows.push(format!(
            "{{\"snapshot\":{},\"calls\":[{}],\"changed\":{changed}}}",
            manager.snapshot_json(),
            dispatched.join(",")
        ));
    }
    rows
}

fn pins() -> BTreeMap<String, JsonValue> {
    std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/divergences"
    ))
    .unwrap()
    .map(Result::unwrap)
    .filter(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("result-limit-")
    })
    .map(|entry| {
        let pin = parse_json(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
        (pin["case"].as_str().unwrap().to_owned(), pin)
    })
    .collect()
}

#[test]
fn result_limits_match_upstream_at_each_limit_and_pin_each_refusal() {
    let scenarios = parse_json(include_str!("../../../conformance/result-limits.json")).unwrap();
    let goldens = parse_json(include_str!(
        "../../../conformance/golden/result-limits.json"
    ))
    .unwrap();
    let pins = pins();
    let print = std::env::var_os("OPENUI_PRINT_RESULT_LIMITS").is_some();
    let mut pinned = BTreeSet::new();
    for scenario in scenarios.as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let golden = &goldens[name];
        let actual = replay(scenario);
        if print {
            println!("{name}\t{}", JsonValue::from(actual.clone()));
            continue;
        }
        if scenario.get("failureControl").is_some() {
            assert_eq!(
                JsonValue::from(actual.clone()),
                goldens[format!("{name}-provider-failure")],
                "{name}: readers after an upstream provider failure"
            );
        }
        let expected = match pins.get(name) {
            Some(pin) => {
                assert_eq!(&pin["reference"], golden, "{name}: upstream pin");
                assert_ne!(pin["rust"], pin["reference"], "{name} has converged");
                pinned.insert(name.to_owned());
                &pin["rust"]
            }
            None => golden,
        };
        let expected = expected.as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (index, row) in actual.iter().enumerate() {
            assert_eq!(
                row,
                expected[index].as_str().unwrap(),
                "{name}, step {index}"
            );
        }
    }
    if print {
        return;
    }
    assert_eq!(pinned.len(), pins.len(), "a pin without a scenario");
    // Each pin is a row of the table, and each row a pin.
    let section = include_str!("../../../CONFORMANCE.md")
        .split_once("### Tool result limits")
        .unwrap()
        .1;
    let rows: BTreeSet<String> = section
        .lines()
        .skip_while(|line| !line.starts_with("| Situation and pin |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|row| {
            let name = row.split('`').nth(1).unwrap();
            name.to_owned()
        })
        .collect();
    assert_eq!(rows, pinned);
}
