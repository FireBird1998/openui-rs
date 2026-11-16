//! The unsupported prototype assignment keeps main's results, including its
//! documented difference from the reference's inherited AST fields.
mod common;

use openui_lang::{Library, StreamParser, parse};
use serde_json::Value;
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn object_prototype_assignments_keep_main_bytes_and_pin_the_reference_difference() {
    let load = |path: &str| -> Value {
        serde_json::from_str(&common::read(&common::conformance_dir().join(path))).unwrap()
    };
    let corpus = load("parser-prototype-assignments.json");
    let main = load("divergences/parser-object-prototypes.json");
    let reference = load("golden/parser-prototype-assignments.json");
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let text = case["program"].as_str().unwrap();
        let lib = Arc::new(Library::from_json_schema(&case["schema"]));
        let result = parse(text, &lib, None);
        assert_eq!(
            result.to_json(),
            main[name]["oneshot"].as_str().unwrap(),
            "{name}"
        );
        assert!(!result.depth_limited && !result.size_limited);
        assert_ne!(
            result.to_json(),
            serde_json::to_string(&reference[name]["oneshot"]).unwrap(),
            "{name}: divergence disappeared"
        );
        assert_eq!(reference[name]["oneshot"]["root"]["hasDynamicProps"], true);
        assert_eq!(
            reference[name]["oneshot"]["meta"]["errors"],
            serde_json::json!([])
        );
        for n in corpus["chunkings"].as_array().unwrap() {
            let n = n.as_u64().unwrap() as usize;
            let mut stream = StreamParser::new(lib.clone(), None);
            let mut steps = Vec::new();
            let mut prefix = String::new();
            for chunk in text.as_bytes().chunks(n) {
                let chunk = std::str::from_utf8(chunk).unwrap();
                prefix.push_str(chunk);
                let result = stream.push(chunk);
                assert!(!result.depth_limited && !result.size_limited);
                steps.push(common::fingerprint(&result.to_json()));
                if prefix.contains("$v") {
                    assert_eq!(
                        steps.last().unwrap(),
                        main[name]["streams"][n.to_string()]["main"][steps.len() - 1]
                            .as_str()
                            .unwrap(),
                        "{name}: an assigned state expression must keep main's output"
                    );
                }
            }
            steps.push(common::fingerprint(&stream.finish().to_json()));
            assert_eq!(
                serde_json::json!(steps),
                main[name]["streams"][n.to_string()]["current"],
                "{name}, chunk={n}"
            );
            assert_ne!(
                main[name]["streams"][n.to_string()]["current"],
                reference[name]["streams"][n.to_string()]
            );
        }
    }
}
