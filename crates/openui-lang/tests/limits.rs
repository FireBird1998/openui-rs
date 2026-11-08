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
    // The work limit scales with the program unless set (see tests/work.rs):
    // a million steps for a short one, 1,000 a byte, at most 64 million.
    assert_eq!(limits.work(), None);
    assert_eq!(limits.work_for(10), 1_000_000);
    assert_eq!(limits.work_for(10_000), 10_000_000);
    assert_eq!(limits.work_for(MAX_PROGRAM_SIZE), 64_000_000);
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

/// A small program with a large result: `a`, a Text of 100 bytes, inlined
/// 1,000 times through three levels of ten references each. About 250 bytes
/// of program and about 230 KB of result, an amplification near 950.
fn amplifier() -> String {
    let ten = |name: &str| [name; 10].join(", ");
    format!(
        "root = Stack([{}])\nc = Stack([{}])\nb = Stack([{}])\na = Text(\"{}\")\n",
        ten("c"),
        ten("b"),
        ten("a"),
        "x".repeat(100)
    )
}

#[test]
fn the_default_result_size_limit_scales_with_the_program() {
    let limits = Limits::default();
    assert_eq!(limits.result_size_for(0), 64 * 1024);
    assert_eq!(limits.result_size_for(1_000), 64 * 1024);
    assert_eq!(limits.result_size_for(10_000), 640_000);
    assert_eq!(limits.result_size_for(1024 * 1024), 8 * 1024 * 1024);
    assert_eq!(limits.amplification(), 64);
    assert_eq!(limits.result_floor(), 64 * 1024);
    assert_eq!(limits.result_size(), 8 * 1024 * 1024);
}

#[test]
fn a_small_program_amplified_past_the_default_is_cut() {
    let lib = library();
    let program = amplifier();
    let fixed = Limits::default().with_result_size(MAX_RESULT_SIZE);
    let whole = parse_with_limits(&program, &lib, None, fixed);
    assert!(!whole.size_limited);
    assert!(whole.to_json().len() > 150_000);

    let cut = parse(&program, &lib, None);
    assert!(cut.size_limited);
    assert!(cut.to_json().len() < 70 * 1024);

    let raised = Limits::default().with_amplification(2_000);
    assert_eq!(parse_with_limits(&program, &lib, None, raised), whole);
}

#[test]
fn a_fixed_result_size_ignores_the_program_size() {
    let limits = Limits::default().with_result_size(10_000);
    assert_eq!(limits.result_size_for(0), 10_000);
    assert_eq!(limits.result_size_for(1024 * 1024), 10_000);
    let raised = Limits::default()
        .with_result_ceiling(64 * 1024 * 1024)
        .with_amplification(128);
    assert_eq!(raised.result_size_for(1024 * 1024), 64 * 1024 * 1024);
    assert_eq!(raised.with_result_floor(1).result_size_for(10), 1_280);
}

/// The `stream` fuzz target's slow input from the lang-core 1.1 long runs
/// (`coverage-stream-timeout-b1187e1…`, 4.8 KB with a 612 KB result), shrunk
/// to 86 bytes whose root expression, written out as an AST, is 98 KB of
/// JSON: an amplification over 1,100. The library is the part of the fuzz
/// target's that it needs. The coverage-guided targets' corpus is seeded
/// with it too (`scripts/fuzz-coverage.sh`).
const SLOW_STREAM_INPUT: &str = include_str!("slow/stream-b1187e1.oui");

fn slow_stream_library() -> Arc<Library> {
    let schema = serde_json::json!({
        "$defs": {"Text": {"properties": {
            "value": {"type": "string"},
            "tone": {"enum": ["info", 2, null], "default": "info"}
        }}},
        "actions": {"Card": {"params": {
            "type": "object", "properties": {"id": {"type": "string"}}
        }}}
    });
    Arc::new(Library::from_json_schema(&schema))
}

#[test]
fn the_slow_stream_fuzz_input_is_cut_under_the_defaults() {
    let lib = slow_stream_library();
    let program = SLOW_STREAM_INPUT;
    let fixed = Limits::default().with_result_size(MAX_RESULT_SIZE);
    let whole = parse_with_limits(program, &lib, None, fixed);
    assert!(!whole.size_limited);
    assert!(whole.to_json().len() > 90_000);

    let cut = parse(program, &lib, None);
    assert!(cut.size_limited);
    assert!(cut.to_json().len() < 64 * 1024);
    assert_eq!(streamed_with(lib.clone(), program, Limits::default()), cut);

    let raised = Limits::default().with_amplification(10_000);
    assert_eq!(parse_with_limits(program, &lib, None, raised), whole);
    assert_eq!(streamed_with(lib, program, raised), whole);
}

/// [`streamed`] with another library.
fn streamed_with(lib: Arc<Library>, program: &str, limits: Limits) -> openui_lang::ParseResult {
    let mut stream = StreamParser::with_limits(lib, None, limits);
    let chars: Vec<char> = program.chars().collect();
    for chunk in chars.chunks(3) {
        stream.push(&chunk.iter().collect::<String>());
    }
    stream.finish()
}

#[test]
fn a_stream_cut_early_is_whole_once_the_program_has_grown_enough() {
    let lib = library();
    let head = amplifier();
    // Statements nothing reads, enough to raise the limit past the result.
    let padding: String = (0..1_000).map(|i| format!("p{i:03} = 1\n")).collect();
    let mut stream = StreamParser::new(lib.clone(), None);
    let early = stream.push(&head);
    assert!(early.size_limited);
    assert_eq!(early, parse(&head, &lib, None));
    let whole = format!("{head}{padding}");
    let late = stream.push(&padding);
    assert!(!late.size_limited);
    assert_eq!(late, parse(&whole, &lib, None));
}

/// A library whose `Stack` takes only `Text` children, so each item that
/// isn't one is a validation error, written out with its message.
fn strict_library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array", "items": {"anyOf": [{"$ref": "#/$defs/Text"}]}}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Validation errors count against the result-size limit like the rest of
/// the result: each `""` item (3 bytes) costs an error of about 100 bytes, so
/// a program made of them amplifies past any limit unless its errors are cut.
/// Found by the coverage-guided targets' flag check (#10): a 294-byte input's
/// result was 13 KB, mostly errors, past a 1 KiB limit and unflagged.
#[test]
fn validation_errors_count_against_the_result_size_limit() {
    let lib = strict_library();
    let program = format!("root = Stack([{}])", "\"\", ".repeat(2_000));
    let limit = 16 * 1024;
    let limits = Limits::default().with_result_size(limit);
    let cut = parse_with_limits(&program, &lib, None, limits);
    assert!(cut.size_limited, "an over-limit result is flagged");
    assert!(
        cut.to_json().len() < limit + 1024,
        "{}",
        cut.to_json().len()
    );
    let whole = parse_with_limits(
        &program,
        &lib,
        None,
        Limits::default().with_result_size(MAX_RESULT_SIZE),
    );
    assert!(!whole.size_limited);
    assert_eq!(whole.meta.errors.len(), 2_000);

    let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
    let streamed = stream.push(&program);
    assert!(streamed.size_limited);
    assert_eq!(streamed, cut);
}

/// What a depth cut writes in a node's place counts against the result-size
/// limit too: each reference to `row` below writes its 300 numbers back as
/// `null`s, cut for depth, so the result amplifies past any limit unless
/// those count. Found by the parse target's flag check in the hardening run
/// (#19): a 4,565-byte input's result was 41,631 bytes, mostly `{"k":"Null"}`
/// placeholders, past an 18,260-byte limit and flagged only `depth_limited`.
#[test]
fn placeholders_for_a_depth_cut_count_against_the_result_size_limit() {
    let lib = library();
    let row = format!("row = [{}]", "0, ".repeat(300));
    let program = format!("{row}\nroot = Stack([[{}]])", "row, ".repeat(300));
    let limits = Limits::default().with_depth(5).with_result_size(16 * 1024);
    let cut = parse_with_limits(&program, &lib, None, limits);
    assert!(cut.depth_limited);
    assert!(cut.size_limited, "an over-limit result is flagged");
    assert!(
        cut.to_json().len() < 16 * 1024 + 1024,
        "{}",
        cut.to_json().len()
    );
    let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
    stream.push(&program);
    assert_eq!(stream.finish(), cut);
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn materialization_prices_exact_result_size_edges() {
    let lib = library();
    let cases = [
        ("root = Stack([1, 2])", 92),
        ("root = Stack([$state + \"abc\", 2])", 162),
        ("root = Stack([$state ? ?unknown : null, 2])", 171),
        ("root = Stack([$state ? true : false, 2])", 179),
        ("root = Stack([Text(null), 2])", 296),
        ("root = Stack([Text(), 2])", 274),
        ("root = Stack([Text(\"abc\"), 2])", 186),
        (
            "root = Stack([$state ? Text(\"abc\") : Text(\"def\"), 2])",
            357,
        ),
        ("root = Stack([true, false, null, ?missing, 2])", 101),
        ("root = Stack([\"abc\", 2])", 73),
        // A space is the first character written as itself, not escaped.
        ("root = Stack([\"     \", 2])", 75),
        ("root = Stack([{longkey: 1}, 2])", 106),
        ("root = Stack([$state, 2])", 97),
        ("root = Stack([@Sum([1, 2]), 2])", 339),
        ("root = Stack([$state.longfield, 2])", 142),
        ("root = Stack([$state = 1, 2])", 152),
        ("root = Stack([$state ? Text(\"abc\") : null, 2])", 258),
        ("root = Stack([$state + {longkey: 1}, 2])", 221),
        ("root = Stack([missing, 2])", 77),
        ("leaf = Text(\"abc\")\nroot = Stack([leaf, leaf, 2])", 349),
        ("root = Stack([Text(1), 2])", 205),
        ("q = Query(\"x\")\nroot = Stack([q, 2])", 166),
    ];
    // These constants price materialization, not the serialized result's length:
    // scalar/AST overhead, keys, duplicated mapped args, errors and inlining.
    // The last array item forces a fit check after earlier unchecked charges.
    for (program, edge) in cases {
        let whole = parse(program, &lib, None);
        for (limit, cut) in [(edge, false), (edge - 1, true)] {
            let limits = Limits::default().with_result_size(limit);
            let result = parse_with_limits(program, &lib, None, limits);
            assert_eq!(result.size_limited, cut, "{program}, limit {limit}");
            if !cut {
                assert_eq!(result.to_json(), whole.to_json(), "{program}");
            } else {
                assert_ne!(result.to_json(), whole.to_json(), "{program}");
            }
            let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
            stream.push(program);
            assert_eq!(stream.finish(), result, "{program}, limit {limit}");
        }
    }
}

/// Regression found while testing cache-sensitive materialization limits (#14).
#[test]
fn repeated_result_preserves_the_cut_tree() {
    let program = "leaf = Text(\"abc\")\nroot = Stack([leaf, [[[leaf]]], leaf])";
    let limits = Limits::default().with_depth(5).with_result_size(120);
    let lib = library();
    let expected = parse_with_limits(program, &lib, None, limits);
    let mut stream = StreamParser::with_limits(lib, None, limits);
    assert_eq!(stream.push(program), expected);
    assert_eq!(stream.result(), expected);
}

/// Seeded corpus coverage stays small enough for this binary's 64 MiB cap.
#[test]
fn lowered_limits_are_independent_of_stream_history() {
    let corpus = common::load_corpus();
    let mut cases: Vec<_> = corpus
        .cases
        .iter()
        .filter(|case| case.input.len() <= 512)
        .collect();
    cases.sort_by_key(|case| common::fingerprint(&format!("issue-14-seed-1:{}", case.name)));
    cases.truncate(96);
    assert_eq!(cases.len(), 96);
    for case in cases {
        for depth in [3, 5, 8] {
            for size in [120, 400, 2000] {
                let limits = Limits::default()
                    .with_depth(depth)
                    .with_result_size(size)
                    .with_work(200);
                let expected =
                    parse_with_limits(&case.input, &case.library, case.root.as_deref(), limits);
                let mut stream =
                    StreamParser::with_limits(case.library.clone(), case.root.as_deref(), limits);
                let first = stream.push(&case.input);
                let same = |actual: &openui_lang::ParseResult,
                            expected: &openui_lang::ParseResult,
                            stage: &str| {
                    let mut actual = actual.clone();
                    // Quirk 4: only the count of a redefinition may differ.
                    if matches!(stage, "whole push" | "whole finish" | "chunked finish") {
                        actual.meta.statement_count = expected.meta.statement_count;
                    }
                    assert_eq!(
                        actual.to_json(),
                        expected.to_json(),
                        "{}, depth {depth}, size {size}, {stage}",
                        case.name
                    );
                    assert_eq!(
                        (actual.size_limited, actual.depth_limited),
                        (expected.size_limited, expected.depth_limited),
                        "{}, {stage} flags",
                        case.name
                    );
                };
                same(&first, &expected, "whole push");
                same(&stream.result(), &first, "repeated result");
                same(&stream.finish(), &expected, "whole finish");
                let mut stream =
                    StreamParser::with_limits(case.library.clone(), case.root.as_deref(), limits);
                let mut start = 0;
                for (i, _) in case.input.char_indices().skip(1).step_by(7) {
                    stream.push(&case.input[start..i]);
                    start = i;
                }
                stream.push(&case.input[start..]);
                let end = stream.finish();
                same(&end, &expected, "chunked finish");
                same(&stream.finish(), &end, "repeated finish");
            }
        }
    }
}

#[test]
fn a_cached_reference_at_the_depth_edge_is_walked_again() {
    let program = "leaf = Text(\"abc\")\nroot = Stack([leaf, [[leaf]], leaf])";
    let lib = library();
    let limits = Limits::default().with_depth(6);
    let result = parse_with_limits(program, &lib, None, limits);
    assert!(result.depth_limited);
    let json: serde_json::Value = serde_json::from_str(&result.to_json()).unwrap();
    assert_eq!(json["root"]["props"]["children"][0]["typeName"], "Text");
    assert_eq!(
        json["root"]["props"]["children"][1],
        serde_json::json!([[]])
    );
    assert_eq!(json["root"]["props"]["children"][2]["typeName"], "Text");
    let whole = parse_with_limits(program, &lib, None, limits.with_depth(7));
    assert!(!whole.depth_limited);
    let json: serde_json::Value = serde_json::from_str(&whole.to_json()).unwrap();
    assert_eq!(
        json["root"]["props"]["children"][1][0][0]["typeName"],
        "Text"
    );
}
