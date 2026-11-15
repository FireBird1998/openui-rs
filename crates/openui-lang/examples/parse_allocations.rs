//! Count parse and fixed-chunk streaming allocations over the conformance corpus.
use openui_lang::{Library, StreamParser, parse, parse_json};
use std::{collections::BTreeMap, path::PathBuf};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut schemas = BTreeMap::new();
    let mut cases = Vec::new();
    for name in [
        "manifest",
        "imported",
        "fuzz",
        "synthetic",
        "recorded",
        "runtime",
    ] {
        let m = parse_json(&std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap())
            .unwrap();
        schemas.extend(
            m["schemas"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())),
        );
        cases.extend(m["cases"].as_array().unwrap().iter().cloned());
    }
    for c in cases {
        let schema = parse_json(
            &std::fs::read_to_string(dir.join(&schemas[c["schema"].as_str().unwrap()])).unwrap(),
        )
        .unwrap();
        let l = Library::from_json_schema(&schema);
        let source = std::fs::read_to_string(dir.join(c["file"].as_str().unwrap())).unwrap();
        let root = c.get("root").and_then(|v| v.as_str());
        let chars = source.chars().collect::<Vec<_>>();
        let chunks = chars
            .chunks(16)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>();
        let start = test_alloc::allocation_calls();
        let p = parse(&source, &l, root);
        let direct = test_alloc::allocation_calls() - start;
        drop(p);
        let start = test_alloc::allocation_calls();
        let mut stream = StreamParser::new(std::sync::Arc::new(l), root);
        for chunk in chunks {
            stream.push(&chunk);
        }
        let streamed = test_alloc::allocation_calls() - start;
        println!("{}\t{direct}\t{streamed}", c["name"].as_str().unwrap());
    }
}
