//! Late statements must not change diagnostic suppression when a cut is replayed.
use std::sync::Arc;

use openui_lang::{Library, Limits, StreamParser, parse_with_limits};
use serde_json::json;

#[path = "support/alias_programs.rs"]
mod programs;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn library() -> Arc<Library> {
    Arc::new(Library::from_json_schema(&json!({"$defs":{
        "Stack":{"properties":{"children":{}}},
        "Box":{"properties":{"a":{},"b":{}}},
        "Req":{"properties":{"x":{"type":"string"},"y":{}},"required":["x"]}
    }})))
}

fn replay_repro(wraps: usize, depth: usize, chunk: Option<usize>) {
    let c = format!("{}c{}", "Box(".repeat(wraps), ")".repeat(wraps));
    let text = format!("root = Stack([{c}, L, {c}])\nc = Req()\nL = c\n");
    let chunk = chunk.unwrap_or(text.len() - 6);
    let lib = library();
    let limits = Limits::default().with_depth(depth);
    let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
    let mut count = 0;
    for bytes in text.as_bytes().chunks(chunk) {
        let result = stream.push(std::str::from_utf8(bytes).unwrap());
        assert!(result.meta.errors.is_empty(), "{:?}", result.meta.errors);
        count += 1;
    }
    let result = stream.finish();
    assert!(result.meta.errors.is_empty(), "{:?}", result.meta.errors);
    assert_eq!(result, parse_with_limits(&text, &lib, None, limits));
    assert_eq!(count, 2);
}

#[test]
fn replayed_depth_cuts_keep_reported_markers_after_a_late_alias() {
    replay_repro(2, 5, Some(54));
}

#[test]
fn replayed_default_depth_cuts_keep_reported_markers_after_a_late_alias() {
    replay_repro(253, openui_lang::MAX_DEPTH, None);
}

#[test]
fn late_alias_stream_results_match_fresh_prefixes_at_small_limits() {
    let lib = library();
    let mut cases = 0;
    for depth in [4, 5, 6, 8, 12, 16, 24, 32, 48] {
        for chain in 0..5 {
            for offset in 0..10 {
                for shape in 0..3 {
                    for late in 0..10 {
                        let lines = programs::late_aliases(depth, chain, offset, shape, late);
                        let room = [256, 1024, 4096, 1024 * 1024 * 1024][late % 4];
                        let limits = Limits::default().with_depth(depth).with_result_size(room);
                        let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
                        let mut prefix = String::new();
                        for line in &lines {
                            prefix.push_str(line);
                            let actual = stream.push(line);
                            let expected = parse_with_limits(&prefix, &lib, None, limits);
                            assert_eq!(actual, expected, "depth={depth} room={room}\n{prefix}");
                        }
                        assert_eq!(
                            stream.finish(),
                            parse_with_limits(&prefix, &lib, None, limits)
                        );
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 13_500);
}
