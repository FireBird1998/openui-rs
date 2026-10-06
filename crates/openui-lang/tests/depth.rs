//! Absurdly deep input can't crash the process. The core stops at
//! `MAX_DEPTH` levels and says so in `depth_limited`. (The reference copes
//! up to roughly 8,000 levels and then throws a RangeError.) Each runs on a
//! thread with a 512 KB stack, because a stack overflow aborts the process
//! rather than panicking. Kept in their own test binary so an overflow can't
//! hide other results.
//!
//! Measured (2026-10-06) with `DEPTH_STACK_KB=<n>`, which overrides the stack
//! size: every shape fits in 384 KB in both test (opt-level 1) and release
//! builds. The 10,000-statement reference chain is the largest; the nested
//! shapes need 256 KB or less. Unoptimized builds need up to about 1 MB,
//! hence opt-level 1 for this crate even in dev and test builds.
//!
//! The same goes for the rest of the language layer on the deepest results:
//! reading the result's JSON back, serializing its tree to source, and merging
//! the program into itself.

use std::sync::Arc;

use openui_lang::{
    Library, MAX_DEPTH, StreamParser, json_to_openui, merge_statements, parse, parse_json,
};

// Ordinary programs must stay well inside the limit.
const _: () = assert!(MAX_DEPTH >= 256);

const STACK: usize = 512 * 1024;
fn stack() -> usize {
    std::env::var("DEPTH_STACK_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(STACK, |kb| kb * 1024)
}

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Parse one-shot and streamed (in 16 chunks) on a small stack. Streaming in
/// small chunks would only add time: re-parsing one huge pending line per
/// push is the quadratic cost ticket 10 is about, not a crash risk.
fn survives(program: String) -> (bool, bool) {
    std::thread::Builder::new()
        .stack_size(stack())
        .spawn(move || {
            let lib = library();
            let one = parse(&program, &lib, None);
            one.to_json();
            let mut stream = StreamParser::new(lib.clone(), None);
            let chars: Vec<char> = program.chars().collect();
            let mut last = None;
            for chunk in chars.chunks(chars.len().div_ceil(16)) {
                last = Some(stream.push(&chunk.iter().collect::<String>()));
            }
            let last = last.unwrap();
            last.to_json();
            // Back to source: every parse result's JSON reads back and serializes.
            let json = parse_json(&one.to_json()).expect("a parse result reads back");
            if !json["root"].is_null() {
                json_to_openui(&json["root"], &lib, Some(&json["stateDeclarations"]))
                    .expect("a parse result serializes");
            }
            merge_statements(&program, &program, None);
            merge_statements("", &program, None);
            (one.depth_limited, last.depth_limited)
        })
        .unwrap()
        .join()
        .expect("parsing panicked")
}

fn assert_limited(program: String) {
    assert_eq!(survives(program), (true, true));
}

#[test]
fn nested_parentheses() {
    assert_limited(format!(
        "root = {}1{}",
        "(".repeat(10_000),
        ")".repeat(10_000)
    ));
}

#[test]
fn nested_arrays() {
    assert_limited(format!(
        "root = Stack({}1{})",
        "[".repeat(10_000),
        "]".repeat(10_000)
    ));
}

#[test]
fn nested_objects() {
    assert_limited(format!(
        "root = Text({}1{})",
        "{a: ".repeat(10_000),
        "}".repeat(10_000)
    ));
}

#[test]
fn nested_component_calls() {
    assert_limited(format!(
        "root = {}\"x\"{}",
        "Text(".repeat(10_000),
        ")".repeat(10_000)
    ));
}

#[test]
fn long_unary_chain() {
    assert_limited(format!("root = Text({}$x)", "!".repeat(100_000)));
}

#[test]
fn long_member_chain() {
    assert_limited(format!("root = Text($x{})", ".b".repeat(100_000)));
}

#[test]
fn long_addition_chain() {
    assert_limited(format!("root = Text(1{})", " + 1".repeat(100_000)));
}

#[test]
fn long_ternary_chain() {
    assert_limited(format!("root = Text({}0)", "$a ? 1 : ".repeat(10_000)));
}

#[test]
fn long_reference_chain() {
    let mut program = String::from("root = Stack([s0])\n");
    for i in 0..10_000 {
        program.push_str(&format!("s{i} = s{}\n", i + 1));
    }
    program.push_str("s10000 = Text(\"end\")");
    assert_limited(program);
}

#[test]
fn ordinary_nesting_is_not_limited() {
    let program = format!(
        "root = Stack([{}Text(\"x\"){}])",
        "Stack([".repeat(40),
        "])".repeat(40)
    );
    assert_eq!(survives(program), (false, false));
}

/// JSON deeper than any parse result is refused, not recursed into.
#[test]
fn json_deeper_than_any_parse_result_is_refused() {
    std::thread::Builder::new()
        .stack_size(stack())
        .spawn(|| {
            let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
            assert!(parse_json(&deep).is_err());
            // Trees built in code: at the limit, serialized; just past it, refused.
            // (Built by hand: `json!` deep-copies its argument recursively.)
            let element = |levels: usize| {
                use serde_json::Value;
                let mut value = Value::String("leaf".into());
                for _ in 0..levels {
                    value = Value::Array(vec![value]);
                }
                let mut props = serde_json::Map::new();
                props.insert("children".into(), value);
                let mut element = serde_json::Map::new();
                element.insert("type".into(), Value::String("element".into()));
                element.insert("typeName".into(), Value::String("Stack".into()));
                element.insert("props".into(), Value::Object(props));
                element.insert("partial".into(), Value::Bool(false));
                Value::Object(element)
            };
            assert!(json_to_openui(&element(797), &library(), None).is_ok());
            assert!(json_to_openui(&element(900), &library(), None).is_err());
            // And JSON text at the limit reads back.
            let at_limit = format!("{}1{}", "[".repeat(800), "]".repeat(800));
            assert!(parse_json(&at_limit).is_ok());
        })
        .unwrap()
        .join()
        .expect("deep JSON panicked");
}
