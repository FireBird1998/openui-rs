//! A parse result can't grow without bound. A statement inlined in two
//! places, then that one in two more, and so on, doubles the result with
//! every line (a "billion laughs" program); a component nested inside a
//! runtime expression does too, since it's written as `args` and again as
//! `mappedProps`. The core cuts the result at about `MAX_RESULT_SIZE` bytes
//! of JSON and sets `size_limited`. Inlined statements are shared rather than
//! copied, so memory stays proportional to the program, not the result.
//! (The reference shares only the `mappedProps` case; the others take it
//! exponential time and memory.)

mod common;

use std::sync::Arc;

use common::load_corpus;
use openui_lang::{Library, MAX_RESULT_SIZE, ParseResult, StreamParser, parse};

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}},
        "Stack": {"properties": {"children": {}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Programs whose result doubles at each of `n` levels.
fn doubling(n: usize) -> Vec<(&'static str, String)> {
    let chain = |first: &str, each: &dyn Fn(usize) -> String, root: String| {
        let mut lines = vec![root, first.to_owned()];
        lines.extend((1..=n).map(each));
        lines.join("\n")
    };
    vec![
        (
            "component nested in a runtime expression",
            format!(
                "root = Text($x ? {}\"a\"{} : null)",
                "Text(".repeat(n),
                ")".repeat(n)
            ),
        ),
        (
            "data referenced twice per level",
            chain(
                "s0 = {a: 1, b: \"x\"}",
                &|i| format!("s{i} = [s{}, s{}]", i - 1, i - 1),
                format!("root = Stack(s{n})"),
            ),
        ),
        (
            "expression referenced twice per level",
            chain(
                "s0 = $x + 1",
                &|i| format!("s{i} = s{} + s{}", i - 1, i - 1),
                format!("root = Text(s{n})"),
            ),
        ),
        (
            "element referenced twice per level",
            chain(
                "s0 = Text(\"x\")",
                &|i| format!("s{i} = Stack([s{}, s{}])", i - 1, i - 1),
                format!("root = Stack([s{n}])"),
            ),
        ),
        (
            "invalid value referenced twice per level",
            chain(
                "s0 = Text([1])",
                &|i| format!("s{i} = Stack([s{}, s{}, missing])", i - 1, i - 1),
                format!("root = Stack([s{n}])"),
            ),
        ),
    ]
}

/// One-shot, and streamed in eight pieces, on a 512 KB stack.
fn parse_both(program: String) -> (ParseResult, ParseResult) {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let lib = library();
            let one = parse(&program, &lib, None);
            let mut stream = StreamParser::new(lib, None);
            let chars: Vec<char> = program.chars().collect();
            let mut last = None;
            for piece in chars.chunks(chars.len().div_ceil(8)) {
                last = Some(stream.push(&piece.iter().collect::<String>()));
            }
            (one, last.unwrap())
        })
        .unwrap()
        .join()
        .expect("parsing panicked")
}

#[test]
fn doubling_programs_are_cut_at_the_size_limit() {
    for (shape, program) in doubling(60) {
        let (one, streamed) = parse_both(program);
        assert!(one.size_limited, "{shape}: not flagged");
        let json = one.to_json();
        assert!(
            json.len() <= MAX_RESULT_SIZE + MAX_RESULT_SIZE / 4,
            "{shape}: {} bytes of JSON",
            json.len()
        );
        assert!(streamed.size_limited, "{shape}: not flagged when streamed");
        assert_eq!(streamed.to_json(), json, "{shape}: streamed differently");
    }
}

#[test]
fn doubling_a_few_times_is_complete() {
    for (shape, program) in doubling(8) {
        let (one, streamed) = parse_both(program);
        assert!(
            !one.size_limited && !streamed.size_limited,
            "{shape}: cut at depth 8"
        );
    }
}

#[test]
fn no_corpus_program_comes_near_the_limit() {
    for case in load_corpus().cases {
        let result = parse(&case.input, &case.library, case.root.as_deref());
        assert!(!result.size_limited, "{}: cut", case.name);
    }
}
