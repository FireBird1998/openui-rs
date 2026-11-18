//! Small documented divergences pin complete outputs independently on both sides.
mod common;

use openui_lang::{Library, Limits, parse, parse_json, parse_message, stringify_json};
use serde_json::{Value, json};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn load(path: &str) -> Value {
    serde_json::from_str(&common::read(&common::conformance_dir().join(path))).unwrap()
}

#[test]
fn documented_core_divergences_pin_both_complete_outputs() {
    let corpus = load("core-divergences.json");
    let reference = load("golden/core-divergences.json");
    let pins = load("divergences/core-divergences.json");
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(
        cases.len(),
        37,
        "documented divergences and agreement controls"
    );
    assert_eq!(reference.as_object().unwrap().len(), cases.len());
    assert_eq!(pins.as_object().unwrap().len(), cases.len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let actual = match case["operation"].as_str().unwrap() {
            "parse" => {
                let library = Library::from_json_schema(&case["schema"]);
                if name == "required-number" {
                    assert!(!library.component("Box").unwrap().params[0].required);
                }
                if name.starts_with("functions-") || name.starts_with("actions-") {
                    assert!(library.call("Bare").unwrap().params.is_empty());
                }
                let result = parse(case["program"].as_str().unwrap(), &library, None);
                json!({"output": result.to_json(), "depth_limited": result.depth_limited,
                    "size_limited": result.size_limited})
            }
            "json" => match parse_json(case["input"].as_str().unwrap()) {
                Ok(value) => json!({"output": stringify_json(&value)}),
                Err(error) => json!({"error": error.to_string()}),
            },
            "message" => {
                let message =
                    parse_message(case["input"].as_str().unwrap(), false, Limits::DEFAULT).unwrap();
                json!({"output": message.to_json()})
            }
            operation => panic!("unknown operation {operation}"),
        };
        assert_eq!(
            reference[name], pins[name]["reference"],
            "{name}: reference output"
        );
        assert_eq!(actual, pins[name]["rust"], "{name}: complete core output");
        assert_eq!(
            reference[name]["output"] == actual["output"],
            pins[name]["same_output"].as_bool().unwrap(),
            "{name}: agreement"
        );
    }
}
