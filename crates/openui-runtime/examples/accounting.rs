//! Reproduce minimum runtime work/room for the one-shot corpus (no timings).
use openui_lang::{JsonValue, Library, parse, parse_json};
use openui_runtime::{
    BindingProps, JsObject, Limits, Store, TreeInput, Value, evaluate_tree_with_limits,
};
use std::{collections::BTreeMap, path::PathBuf};
#[path = "../tests/support/functions.rs"]
mod functions;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn load(path: PathBuf) -> JsonValue {
    parse_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let persisted = std::env::args()
        .nth(1)
        .is_some_and(|arg| arg == "persisted");
    let mut schemas = BTreeMap::new();
    let mut cases = Vec::new();
    for manifest in [
        "manifest",
        "imported",
        "fuzz",
        "synthetic",
        "recorded",
        "runtime",
    ] {
        let m = load(dir.join(format!("{manifest}.json")));
        schemas.extend(
            m["schemas"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned())),
        );
        cases.extend(m["cases"].as_array().unwrap().iter().cloned());
    }
    let binding_sources = load(dir.join("bindings.json"));
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let key = case["schema"].as_str().unwrap();
        let schema = load(dir.join(&schemas[key]));
        let library = Library::from_json_schema(&schema);
        let source = std::fs::read_to_string(dir.join(case["file"].as_str().unwrap())).unwrap();
        let mut p = parse(
            &source,
            &library,
            case.get("root").and_then(JsonValue::as_str),
        );
        if case.get("queryComplete").and_then(JsonValue::as_bool) == Some(false) {
            for query in &mut p.query_statements {
                query.complete = false;
            }
        }
        let mut store = Store::new();
        let _ = store.initialize(
            p.state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            None,
        );
        if persisted {
            let golden = load(dir.join(format!("golden/runtime/{name}.oneshot.json")));
            if let Some(saved) = golden["saved"].as_object() {
                for (key, value) in saved {
                    store.set(key, Some(Value::from_json(value))).unwrap();
                }
            }
        }
        if let Some(values) = case.get("stateValues").and_then(JsonValue::as_object) {
            for (k, v) in values {
                store.set(k, Some(Value::from_json(v))).unwrap();
            }
        }
        let mut references = JsObject::new();
        if let Some(values) = case.get("referenceValues").and_then(JsonValue::as_object) {
            for (k, v) in values {
                references.define(k, Value::from_json(v));
            }
        }
        let bindings = binding_sources
            .get(key)
            .and_then(JsonValue::as_str)
            .map_or_else(BindingProps::new, |path| {
                BindingProps::from_prompt_spec(&Value::from_json(&load(dir.join(path))["library"]))
            });
        let f = functions::registry(schema.get("functions").unwrap_or(&JsonValue::Null));
        let input = TreeInput::new(&p)
            .with_library(&library)
            .with_functions(&f)
            .with_actions(schema.get("actions").unwrap_or(&JsonValue::Null))
            .with_binding_props(&bindings)
            .with_reference_values(&references);
        let run = |work, room| {
            evaluate_tree_with_limits(
                input.clone(),
                &store,
                Limits::DEFAULT.with_work(work).with_result_size(room),
            )
        };
        let maximum = 128 * 1024 * 1024;
        let whole = run(1_000_000_000, maximum);
        if whole.size_limited || whole.depth_limited {
            println!("{name}\tcut\tcut");
            continue;
        }
        let minimum = |is_work| {
            let (mut lo, mut hi) = (0, if is_work { 1_000_000_000 } else { maximum });
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let r = if is_work {
                    run(mid, maximum)
                } else {
                    run(1_000_000_000, mid)
                };
                if r.size_limited || r.depth_limited {
                    lo = mid + 1
                } else {
                    hi = mid
                }
            }
            lo
        };
        println!("{name}\t{}\t{}", minimum(true), minimum(false));
    }
}
