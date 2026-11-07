//! Dropping a result needs no stack, however deep it is: its trees are freed
//! with a loop, not recursion (ADR 0002). Each test parses, streams and drops
//! on a thread with a 64 KB stack, at the default depth limit and at a raised
//! one. Kept in their own test binary, because a stack overflow aborts the
//! process rather than failing one test. `DEPTH_STACK_KB=<n>` overrides the
//! stack size; run it unoptimized too
//! (`CARGO_PROFILE_TEST_OPT_LEVEL=0 cargo test --test drop`), where frames are
//! largest.

use std::sync::Arc;

use openui_lang::{Library, Limits, StreamParser, parse_with_limits};

fn stack() -> usize {
    std::env::var("DEPTH_STACK_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(64 * 1024, |kb| kb * 1024)
}

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Programs whose results nest `levels` deep in each kind of tree a result
/// holds: arrays, elements, a runtime expression, evaluated defaults, and a
/// chain of statements each referencing the next.
fn deep_programs(levels: usize) -> Vec<(&'static str, String)> {
    let wrap = |open: &str, inner: &str, close: &str| {
        format!("{}{inner}{}", open.repeat(levels), close.repeat(levels))
    };
    let mut defaults = String::from("root = Text($v3)\n$v0 = [1, \"x\"]\n");
    for i in 1..=3 {
        defaults.push_str(&format!(
            "$v{i} = {}\n",
            wrap("[", &format!("$v{}", i - 1), "]")
        ));
    }
    let mut chain = String::from("root = Stack([s0])\n");
    for i in 0..levels {
        chain.push_str(&format!("s{i} = Text(s{})\n", i + 1));
    }
    chain.push_str(&format!("s{levels} = \"end\"\n"));
    vec![
        ("arrays", format!("root = Stack({})", wrap("[", "1", "]"))),
        (
            "elements",
            format!("root = {}", wrap("Text(", "\"x\"", ")")),
        ),
        (
            "a runtime expression",
            format!("root = Text({}1)", "$x ? 1 : ".repeat(levels)),
        ),
        ("evaluated defaults", defaults),
        ("a reference chain", chain),
    ]
}

/// Parse and stream each program within `limits`, then drop every result and
/// the stream parser, all on a small stack.
fn parses_and_drops(levels: usize, limits: Limits) {
    std::thread::Builder::new()
        .stack_size(stack())
        .spawn(move || {
            let lib = library();
            for (shape, program) in deep_programs(levels) {
                let one = parse_with_limits(&program, &lib, None, limits);
                assert!(
                    one.to_json().len() > levels,
                    "{shape}: the result isn't deep"
                );
                drop(one);
                let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
                let chars: Vec<char> = program.chars().collect();
                for chunk in chars.chunks(chars.len().div_ceil(4)) {
                    drop(stream.push(&chunk.iter().collect::<String>()));
                }
                drop(stream.finish());
                drop(stream);
            }
        })
        .unwrap()
        .join()
        .expect("parsing or dropping panicked");
}

#[test]
fn results_at_the_default_depth_limit_drop_on_a_small_stack() {
    parses_and_drops(1_000, Limits::default());
}

#[test]
fn results_at_a_raised_depth_limit_drop_on_a_small_stack() {
    parses_and_drops(10_000, Limits::default().with_depth(20_000));
}
