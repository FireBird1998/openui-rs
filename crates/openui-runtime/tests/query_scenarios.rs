//! Public query commands replay the reference manager's scripted observations.
use std::collections::{BTreeMap, HashSet};

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

fn replay(scenario: &JsonValue, mut observe: impl FnMut(usize, String, usize)) {
    let name = scenario["name"].as_str().unwrap();
    let empty_library = parse_json("{\"$defs\":{}}").unwrap();
    let library = Library::from_json_schema(scenario.get("library").unwrap_or(&empty_library));
    let mut store = Store::new();
    let mut current = program(&scenario["program"], &library, &mut store);
    let mut manager = QueryManager::default();
    let mut calls: Vec<ToolCallId> = Vec::new();
    let mut dispatches = Vec::new();
    let steps = scenario["steps"].as_array().unwrap();
    let mut registered = HashSet::new();
    for (index, step) in steps.iter().enumerate() {
        if step["kind"] == "evaluate" {
            registered = current
                .query_statements
                .iter()
                .filter(|q| q.complete || registered.contains(&q.statement_id))
                .map(|q| q.statement_id.clone())
                .collect();
        }
        let command = match step["kind"].as_str().unwrap() {
            "evaluate" => Some(QueryCommand::Evaluate {
                program: &current,
                store: &store,
            }),
            "complete" => {
                let call = if let Some(dispatch) = step.get("dispatch") {
                    let mut matches = dispatches
                        .iter()
                        .enumerate()
                        .filter(|(_, address)| *address == dispatch);
                    let first = matches.next().map(|(number, _)| number);
                    assert!(
                        matches.next().is_none(),
                        "ambiguous dispatch address {dispatch}"
                    );
                    first
                } else {
                    Some(usize::try_from(step["call"].as_u64().unwrap()).unwrap())
                };
                call.map(|call| QueryCommand::Complete {
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
        assert!(
            manager.cache_entry_count() <= 2 * registered.len(),
            "{name}, step {index}: {} entries for {} queries",
            manager.cache_entry_count(),
            registered.len()
        );
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
            let mut address = JsonValue::Object(Default::default());
            address["step"] = index.into();
            address["toolName"] = call.tool_name.clone().into();
            if let Some(args) = &call.args {
                address["args"] = parse_json(&value_to_json(args).unwrap()).unwrap();
            }
            dispatches.push(address);
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
        observe(index, actual, manager.cache_entry_count());
    }
}

#[test]
fn query_scenarios_match_upstream_after_every_step() {
    let scenarios = parse_json(include_str!("../../../conformance/query-scenarios.json")).unwrap();
    let goldens = parse_json(include_str!(
        "../../../conformance/golden/query-scenarios.json"
    ))
    .unwrap();
    let pins = std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/divergences"
    ))
    .unwrap()
    .map(Result::unwrap)
    .filter(|entry| entry.file_name().to_string_lossy().starts_with("query-"))
    .map(|entry| {
        let pin = parse_json(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
        let name = pin["case"].as_str().unwrap().to_owned();
        assert_eq!(
            Some(&pin["reference"]),
            goldens.get(&name),
            "{name} upstream pin"
        );
        assert_ne!(pin["rust"], pin["reference"], "{name} has converged");
        (name, pin)
    })
    .collect::<std::collections::HashMap<_, _>>();
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
        let expected = pins
            .get(name)
            .map_or(&goldens[name], |pin| &pin["rust"])
            .as_array()
            .unwrap();
        let steps = scenario["steps"].as_array().unwrap();
        assert_eq!(steps.len(), expected.len(), "{name}");
        replay(scenario, |index, actual, _| {
            assert_eq!(
                actual,
                expected[index].as_str().unwrap(),
                "{name}, step {index}: {}",
                steps[index]
            );
        });
    }
    assert!(matched > 0, "Unknown scenario filter {filter:?}");
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn random_query_edits_match_only_documented_entry_lifetime_differences() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let mut reference = Command::new("bun")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .args(["--no-install", "query-scenarios.ts", "--random"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut scenarios = 0;
    let mut steps = 0;
    let mut counts = documented_situations();
    let mut affected = 0;
    for line in BufReader::new(reference.stdout.take().unwrap()).lines() {
        let row = parse_json(&line.unwrap()).unwrap();
        let scenario = &row["scenario"];
        let name = scenario["name"].as_str().unwrap();
        let upstream = row["reference"].as_array().unwrap();
        let overrides = row["overrides"].as_object().unwrap();
        assert_eq!(upstream.len(), scenario["steps"].as_array().unwrap().len());
        let reasons = row["reasons"].as_object().unwrap();
        assert_eq!(reasons.len(), overrides.len(), "{name} attribution");
        replay(scenario, |index, actual, entry_count| {
            assert_eq!(
                Some(entry_count as u64),
                row["entryCounts"][index].as_u64(),
                "{name}, step {index}: live keys"
            );
            let expected = overrides
                .get(&index.to_string())
                .unwrap_or(&upstream[index]);
            assert_eq!(
                actual,
                expected.as_str().unwrap(),
                "{name}, step {index}: {scenario}"
            );
            if actual != upstream[index].as_str().unwrap() {
                let reason = reasons[&index.to_string()].as_str().unwrap();
                let (_, observed) = counts
                    .get_mut(reason)
                    .unwrap_or_else(|| panic!("{name}: undocumented {reason}"));
                *observed += 1;
            } else {
                assert!(
                    !reasons.contains_key(&index.to_string()),
                    "{name}: identical observation attributed"
                );
            }
        });
        scenarios += 1;
        steps += upstream.len();
        affected += usize::from(!overrides.is_empty());
    }
    assert!(reference.wait().unwrap().success());
    assert_eq!(scenarios, 6000);
    assert!(affected > 0);
    let mut differences = 0;
    for (case, (documented, observed)) in counts {
        assert_eq!(observed, documented, "{case} random observations");
        differences += observed;
        eprintln!("{case}: {observed}");
    }
    eprintln!(
        "{scenarios} scripts, {steps} observations, {differences} differences in {affected} scripts"
    );
}

// Each table row names the scenario pin and its observation count. A new
// difference therefore needs a two-sided pin and an entry in CONFORMANCE.md.
fn documented_situations() -> BTreeMap<String, (usize, usize)> {
    let section = include_str!("../../../CONFORMANCE.md")
        .split_once("### Query cache lifetime differences")
        .unwrap()
        .1;
    let mut counts = BTreeMap::new();
    for row in section
        .lines()
        .skip_while(|line| !line.starts_with("| `"))
        .take_while(|line| line.starts_with("| `"))
    {
        let name = row.split('`').nth(1).unwrap();
        let count = row
            .trim_end_matches('|')
            .rsplit('|')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let path = format!(
            "{}/../../conformance/divergences/query-{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let pin = parse_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(pin["case"], name);
        assert_ne!(pin["reference"], pin["rust"], "{name} has converged");
        assert!(
            counts.insert(name.to_owned(), (count, 0)).is_none(),
            "duplicate {name}"
        );
    }
    assert!(!counts.is_empty());
    counts
}
