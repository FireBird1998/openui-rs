//! Absurdly deep input can't crash the process. The core stops at
//! `MAX_DEPTH` levels and says so in `depth_limited`. (The reference copes
//! up to roughly 8,000 levels and then throws a RangeError.) Each runs on a
//! thread with a 512 KB stack, because a stack overflow aborts the process
//! rather than panicking. Kept in their own test binary so an overflow can't
//! hide other results.
//!
//! Measured at depth 256: optimized builds (opt-level 1 or 3) need at most
//! 256 KB of stack, unoptimized ones up to about 1 MB. Hence opt-level 1 for
//! this crate even in dev and test builds. `DEPTH_STACK_KB=<n>` overrides the
//! stack size to measure.

use std::sync::Arc;

use openui_lang::{Library, MAX_DEPTH, StreamParser, parse};

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
            let mut stream = StreamParser::new(lib, None);
            let chars: Vec<char> = program.chars().collect();
            let mut last = None;
            for chunk in chars.chunks(chars.len().div_ceil(16)) {
                last = Some(stream.push(&chunk.iter().collect::<String>()));
            }
            let last = last.unwrap();
            last.to_json();
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
