//! Streaming identities: Store comparisons stay Object.is-compatible; parser
//! default reuse is an explicitly pinned notification divergence.
use openui_lang::{Library, StreamParser, Value, parse_json};
use openui_runtime::Store;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Arr(a), Value::Arr(b)) => Arc::ptr_eq(a, b),
        (Value::Obj(a), Value::Obj(b)) => Arc::ptr_eq(a, b),
        (Value::Ast(a), Value::Ast(b)) => Arc::ptr_eq(a, b),
        (Value::Element(a), Value::Element(b)) => Arc::ptr_eq(a, b),
        _ => a == b,
    }
}
#[test]
fn completed_defaults_keep_values_but_reused_containers_suppress_notifications() {
    let fixture = parse_json(include_str!("streaming-defaults.json")).unwrap();
    let schema = parse_json(include_str!("../../../conformance/schemas/runtime.json")).unwrap();
    let program = fixture["program"].as_str().unwrap();
    let mut observed = Vec::new();
    for scenario in fixture["outputs"].as_array().unwrap() {
        let mode = scenario["edited"].as_str().unwrap();
        let mut stream = StreamParser::new(Arc::new(Library::from_json_schema(&schema)), None);
        let mut store = Store::new();
        let mut previous = None;
        let mut reused = Vec::new();
        let mut notifications = Vec::new();
        let mut differences = Vec::new();
        for (i, c) in program.chars().enumerate() {
            let parsed = stream.push(&c.to_string());
            let aliases = parsed
                .state_declarations
                .iter()
                .filter_map(|(k, v)| {
                    previous
                        .as_ref()
                        .and_then(|p: &openui_lang::ParseResult| p.state_declarations.get(k))
                        .filter(|old| same(old, v))
                        .map(|_| k.clone())
                })
                .collect::<Vec<_>>();
            reused.push(aliases);
            let before = store.version();
            let init = store.initialize(
                parsed
                    .state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                std::iter::empty(),
            );
            assert!(init.refused.is_empty());
            let notified = store.version() - before;
            notifications.push(notified);
            let expected = &scenario["steps"][i];
            assert_eq!(
                parse_json(&store.snapshot_json().unwrap()).unwrap(),
                expected["snapshot"],
                "{mode}, byte {i}"
            );
            if expected["notified"].as_u64().unwrap() != notified {
                differences.push(i);
            }
            if scenario["setAt"].as_u64() == Some(i as u64) {
                store
                    .set(
                        scenario["setKey"].as_str().unwrap(),
                        Some(Value::from_json(&scenario["setValue"])),
                    )
                    .unwrap();
            }
            previous = Some(parsed);
        }
        observed.push(openui_lang::JsonValue::from(mode));
        let reference_history = scenario["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| {
                openui_lang::JsonValue::from(vec![step["same"].clone(), step["notified"].clone()])
            })
            .collect::<Vec<_>>();
        observed.push(openui_lang::JsonValue::from(reference_history));
        observed.push(openui_lang::JsonValue::from(differences));
        observed.push(openui_lang::JsonValue::from(notifications));
        observed.push(openui_lang::JsonValue::from(reused));
    }
    let got = openui_lang::JsonValue::from(observed);
    let expected = parse_json(include_str!(
        "../../../conformance/divergences/store-stream-default-identities.json"
    ))
    .unwrap();
    assert_eq!(got, expected);
}
