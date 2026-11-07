//! The one `Limits` value (ADR 0001): its defaults, and raising or lowering
//! each limit through one-shot parsing and the stream parser. Hitting a limit
//! gives a cut result, flagged, never an error.

mod common;

use std::sync::Arc;

use openui_lang::{
    Library, Limits, MAX_DEPTH, MAX_PROGRAM_SIZE, MAX_RESULT_SIZE, StreamParser, parse,
    parse_with_limits,
};

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

#[test]
fn no_default_cuts_a_corpus_case() {
    let corpus = common::load_corpus();
    assert!(!corpus.cases.is_empty());
    let cut: Vec<&str> = corpus
        .cases
        .iter()
        .filter(|case| {
            let result = parse(&case.input, &case.library, case.root.as_deref());
            result.size_limited || result.depth_limited
        })
        .map(|case| case.name.as_str())
        .collect();
    assert!(cut.is_empty(), "cut by a default limit: {cut:?}");
}

#[test]
fn the_defaults_are_the_documented_ones() {
    let limits = Limits::default();
    assert_eq!(limits.program_size(), 1024 * 1024);
    assert_eq!(limits.result_size(), 8 * 1024 * 1024);
    assert_eq!(limits.depth(), 256);
    assert_eq!(
        (limits.program_size(), limits.result_size(), limits.depth()),
        (MAX_PROGRAM_SIZE, MAX_RESULT_SIZE, MAX_DEPTH)
    );
}

/// `root = Stack([[...[1]...]])`, nested `levels` deep.
fn nested(levels: usize) -> String {
    format!(
        "root = Stack({}1{})",
        "[".repeat(levels),
        "]".repeat(levels)
    )
}

#[test]
fn lowering_depth_cuts_shallower_nesting() {
    let lib = library();
    let program = nested(20);
    assert!(!parse(&program, &lib, None).depth_limited);
    let lowered = Limits::default().with_depth(10);
    assert!(parse_with_limits(&program, &lib, None, lowered).depth_limited);
}

#[test]
fn raising_depth_lets_deeper_nesting_through() {
    let lib = library();
    let program = nested(1_000);
    assert!(parse(&program, &lib, None).depth_limited);
    let raised = Limits::default().with_depth(2_000);
    let deep = parse_with_limits(&program, &lib, None, raised);
    assert!(!deep.depth_limited);
    // Every level made it into the result: 1,000 arrays around the 1.
    let json = deep.to_json();
    assert!(json.contains(&format!("{}1{}", "[".repeat(1_000), "]".repeat(1_000))));
}

/// A program whose result is about 30 KB of JSON: 300 texts of 100 bytes.
fn thirty_kb() -> String {
    let texts: Vec<String> = (0..300).map(|i| format!("Text(\"{i:0>100}\")")).collect();
    format!("root = Stack([{}])", texts.join(", "))
}

#[test]
fn the_result_size_limit_can_be_lowered_and_raised() {
    let lib = library();
    let program = thirty_kb();
    let whole = parse(&program, &lib, None);
    assert!(!whole.size_limited);
    let json = whole.to_json();
    assert!(json.len() > 30_000);

    let lowered = Limits::default().with_result_size(8 * 1024);
    let cut = parse_with_limits(&program, &lib, None, lowered);
    assert!(cut.size_limited);
    assert!(cut.to_json().len() < 9 * 1024);

    let raised = lowered.with_result_size(64 * 1024);
    assert_eq!(parse_with_limits(&program, &lib, None, raised), whole);
}

/// `program` pushed a few characters at a time, and its result at the end.
fn streamed(program: &str, limits: Limits) -> openui_lang::ParseResult {
    let mut stream = StreamParser::with_limits(library(), None, limits);
    let chars: Vec<char> = program.chars().collect();
    for chunk in chars.chunks(7) {
        stream.push(&chunk.iter().collect::<String>());
    }
    stream.finish()
}

#[test]
fn a_stream_parser_keeps_its_limits_for_every_push() {
    let lib = library();
    for (program, limits) in [
        (nested(20), Limits::default().with_depth(10)),
        (nested(1_000), Limits::default().with_depth(2_000)),
        (thirty_kb(), Limits::default().with_result_size(8 * 1024)),
    ] {
        let one = parse_with_limits(&program, &lib, None, limits);
        assert_eq!(streamed(&program, limits), one);
    }
    assert!(streamed(&nested(20), Limits::default().with_depth(10)).depth_limited);
    assert!(!streamed(&nested(1_000), Limits::default().with_depth(2_000)).depth_limited);
}

const PROGRAM: &str = "root = Stack([a, b])\na = Text(\"first\")\nb = Text(\"second\")";

#[test]
fn past_the_program_size_limit_the_rest_is_ignored_as_if_the_stream_stopped() {
    let lib = library();
    let whole = parse(PROGRAM, &lib, None);
    assert!(!whole.size_limited);
    // Cut 6 bytes into `b = Text("second")`.
    let at = PROGRAM.find("b = ").unwrap() + 6;
    let limits = Limits::default().with_program_size(at);
    let cut = parse_with_limits(PROGRAM, &lib, None, limits);
    assert!(cut.size_limited);
    let mut prefix = parse(&PROGRAM[..at], &lib, None);
    assert!(!prefix.size_limited);
    prefix.size_limited = true;
    assert_eq!(cut, prefix);
}

#[test]
fn the_program_size_limit_never_splits_a_character() {
    let lib = library();
    let program = "root = Text(\"héllo\")";
    let inside_e = program.find('é').unwrap() + 1;
    let limits = Limits::default().with_program_size(inside_e);
    let mut before = parse(&program[..inside_e - 1], &lib, None);
    before.size_limited = true;
    assert_eq!(parse_with_limits(program, &lib, None, limits), before);
    let mut stream = StreamParser::with_limits(library(), None, limits);
    assert_eq!(stream.push(program), before);
}

#[test]
fn a_stream_past_the_program_size_limit_stays_cut_until_set() {
    let at = PROGRAM.find("b = ").unwrap() + 6;
    let limits = Limits::default().with_program_size(at);
    let lib = library();
    let expected = parse_with_limits(PROGRAM, &lib, None, limits);

    let mut stream = StreamParser::with_limits(library(), None, limits);
    let chars: Vec<char> = PROGRAM.chars().collect();
    let mut last = None;
    for chunk in chars.chunks(5) {
        last = Some(stream.push(&chunk.iter().collect::<String>()));
    }
    assert_eq!(stream.finish(), expected);
    let pushed = last.unwrap();
    assert!(pushed.size_limited);

    // Further pushes are ignored: the stream stopped where it was cut.
    assert_eq!(stream.push("\nc = Text(\"third\")"), pushed);
    assert_eq!(stream.finish(), expected);

    // `set` starts over, under the same limits.
    let short = "root = Text(\"x\")";
    let again = stream.set(short);
    assert!(!again.size_limited);
    assert_eq!(again, parse(short, &lib, None));
}

#[test]
fn a_program_within_the_limit_is_not_cut() {
    let lib = library();
    let limits = Limits::default().with_program_size(PROGRAM.len());
    assert_eq!(
        parse_with_limits(PROGRAM, &lib, None, limits),
        parse(PROGRAM, &lib, None)
    );
    let mut stream = StreamParser::with_limits(library(), None, limits);
    stream.push(PROGRAM);
    assert_eq!(stream.finish(), parse(PROGRAM, &lib, None));
}

#[test]
fn the_default_program_size_limit_cuts_plain_parsing_and_streaming() {
    // Just over 1 MiB of statements.
    let line = "a = \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"\n";
    let program = format!(
        "root = Text(a)\n{}",
        line.repeat(MAX_PROGRAM_SIZE / line.len() + 1)
    );
    assert!(program.len() > MAX_PROGRAM_SIZE);
    assert!(parse(&program, &library(), None).size_limited);
    let mut stream = StreamParser::new(library(), None);
    assert!(stream.push(&program).size_limited);
}

#[test]
fn at_a_raised_depth_a_default_holding_a_deep_component_is_read_whole() {
    // The default's component nests 400 levels: written as JSON, about 1,200,
    // past the 800 that `parse_json` takes from outside.
    let levels = 400;
    let program = format!(
        "root = Stack([])\n$a = {}Text(\"x\"){}\n$b = $a",
        "Stack([".repeat(levels),
        "])".repeat(levels)
    );
    let limits = Limits::default().with_depth(2_000);
    let result = parse_with_limits(&program, &library(), None, limits);
    assert!(!result.size_limited && !result.depth_limited);
    let json = result.to_json();
    assert!(!json.contains("\"$a\":null") && !json.contains("\"$b\":null"));
}
