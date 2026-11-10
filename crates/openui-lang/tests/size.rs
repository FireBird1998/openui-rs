//! A parse result can't grow without bound. A statement inlined in two
//! places, then that one in two more, and so on, doubles the result with
//! every line (a "billion laughs" program); a component nested inside a
//! runtime expression does too, since it's written as `args` and again as
//! `mappedProps`. The core cuts the result at the result-size limit, by
//! default the larger of 64 KiB and 64 times the program's size, never above
//! `MAX_RESULT_SIZE` bytes of JSON, and sets `size_limited`. Inlined statements are shared rather than
//! copied, so memory stays proportional to the program, not the result. State
//! defaults, evaluated, can double too (a list of the one above twice, a
//! string added to itself): evaluating stops at what's left of the result-size limit.
//! (The reference shares only the `mappedProps` case; the others take it
//! exponential time and memory.)

mod common;

use std::sync::Arc;

use common::load_corpus;
use openui_lang::{
    Library, Limits, MAX_RESULT_SIZE, ParseResult, StreamParser, parse, parse_with_limits,
};

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}},
        "Stack": {"properties": {"children": {}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Programs whose result doubles at each of `n` levels.
fn doubling(n: usize) -> Vec<(&'static str, String)> {
    // Evaluated, each state default reads the one above it.
    let state_chain = |first: &str, each: &dyn Fn(&str) -> String| {
        let mut lines = vec!["root = Text(\"x\")".to_owned(), format!("$s0 = {first}")];
        lines.extend((1..=n).map(|i| format!("$s{i} = {}", each(&format!("$s{}", i - 1)))));
        lines.join("\n")
    };
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
            "builtin call nested in a runtime expression",
            format!(
                "root = Text($x ? {}\"a\"{} : null)",
                "@Count(".repeat(n),
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
            "state default listing the one above twice",
            state_chain("[1, \"x\"]", &|p| format!("[{p}, {p}]")),
        ),
        (
            "state default adding the one above to itself",
            state_chain("\"ab\"", &|p| format!("{p} + {p}")),
        ),
        (
            "state default joining a list of the one above",
            state_chain("\"ab\"", &|p| format!("\"\" + [{p}, {p}]")),
        ),
        (
            "state default copying the one above with @Each",
            state_chain("[1, \"x\"]", &|p| format!("@Each([{p}, {p}], \"x\", x)")),
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
        .name(format!(
            "{}::worker",
            std::thread::current().name().unwrap_or("test")
        ))
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
        let limit = Limits::default().result_size_for(program.len());
        let (one, streamed) = parse_both(program);
        assert!(one.size_limited, "{shape}: not flagged");
        let json = one.to_json();
        assert!(
            json.len() <= limit + limit / 4,
            "{shape}: {} bytes of JSON",
            json.len()
        );
        assert!(streamed.size_limited, "{shape}: not flagged when streamed");
        assert_eq!(streamed.to_json(), json, "{shape}: streamed differently");
    }
}

#[test]
fn doubling_a_few_times_is_complete() {
    for (shape, program) in doubling(5) {
        let (one, streamed) = parse_both(program);
        assert!(
            !one.size_limited && !streamed.size_limited,
            "{shape}: cut at depth 5"
        );
    }
}

#[test]
fn doubling_up_to_the_ceiling_is_complete_when_the_limit_is_fixed_there() {
    // With the result size fixed at the ceiling, as before it scaled.
    let fixed = Limits::default().with_result_size(MAX_RESULT_SIZE);
    for (shape, program) in doubling(8) {
        let result = parse_with_limits(&program, &library(), None, fixed);
        assert!(!result.size_limited, "{shape}: cut at depth 8");
    }
}

#[test]
fn no_corpus_program_comes_near_the_limit() {
    for case in load_corpus().cases {
        let result = parse(&case.input, &case.library, case.root.as_deref());
        assert!(!result.size_limited, "{}: cut", case.name);
    }
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn shared_subtrees_stay_small_at_the_result_ceiling() {
    let lib = library();
    for (shape, program) in doubling(18).into_iter().take(5) {
        test_alloc::reset_peak();
        let result = parse_with_limits(
            &program,
            &lib,
            None,
            Limits::default().with_result_size(MAX_RESULT_SIZE),
        );
        let peak = test_alloc::peak();
        eprintln!("shared shape {shape}: {peak} bytes");
        assert!(
            peak < 1024 * 1024,
            "{shape}: peak {peak} bytes; shared subtrees must stay below 1 MiB"
        );
        std::hint::black_box(result);
    }
}

/// A root cut at the result-size limit, then `rest`.
fn cut_root_then(rest: &str) -> String {
    let (_, doubling) = doubling(60).swap_remove(0);
    format!("{doubling}\n{rest}")
}

/// Programs with `n` state declarations, queries, mutations or references to
/// them: what those write besides their values counts against the limit too.
fn wrappers(n: usize) -> Vec<(&'static str, String)> {
    let lines = |line: &dyn Fn(usize) -> String| (0..n).map(line).collect::<String>();
    vec![
        (
            "queries",
            cut_root_then(&lines(&|i| format!("q{i}=Query()\n"))),
        ),
        (
            "mutations",
            cut_root_then(&lines(&|i| format!("m{i}=Mutation()\n"))),
        ),
        ("state", cut_root_then(&lines(&|i| format!("$s{i}=1\n")))),
        (
            "state never declared",
            cut_root_then(&lines(&|i| format!("q{i}=Query(\"t\",$u{i})\n"))),
        ),
        (
            "queries reading state",
            cut_root_then(&lines(&|i| {
                format!("q{i}=Query(\"t\",[$a,$b,$c,q{}])\n", i + 1)
            })),
        ),
        (
            "queries alone",
            lines(&|i| format!("q{i}=Query(q{})\n", i + 1)),
        ),
        (
            "references to a query, written twice at each level",
            format!(
                "q=Query()\nroot = Text($x ? {}[{}]{} : null)",
                "Text(".repeat(9),
                vec!["q"; n].join(","),
                ")".repeat(9)
            ),
        ),
        (
            "references to a mutation, in a list",
            format!(
                "m=Mutation()\nroot = Stack([{}])",
                vec!["m"; n * 5].join(",")
            ),
        ),
        ("query loops", lines(&|i| format!("q{i}=Query(q{i})\n"))),
        (
            "many query loops through each other",
            lines(&|i| format!("q{i}=Query(q{},q{})\n", i + 1, (i * 7) % n)),
        ),
    ]
}

#[test]
fn state_queries_and_mutations_count_against_the_size_limit() {
    for (shape, program) in wrappers(6000) {
        let limit = Limits::default().result_size_for(program.len());
        let (one, streamed) = parse_both(program);
        let json = one.to_json();
        assert!(
            json.len() <= limit + 1024,
            "{shape}: {} bytes of JSON, limit {limit}",
            json.len()
        );
        assert_eq!(streamed.to_json(), json, "{shape}: streamed differently");
    }
}

#[test]
fn a_query_that_does_not_fit_is_dropped_with_every_later_statement() {
    let program = "q1=Query()\nq2=Query()\nm1=Mutation()\n$s=1\nq3=Query()\nroot=Text(1)";
    let all = parse(program, &library(), None);
    assert!(!all.size_limited);
    assert_eq!(all.query_statements.len(), 3);
    // The smallest limit with room for the root and the first two queries.
    let whole = all.to_json().len();
    let mut cut = None;
    for limit in 0..whole {
        let limits = Limits::default().with_result_size(limit);
        let result = parse_with_limits(program, &library(), None, limits);
        if result.root.is_some() && result.query_statements.len() == 2 {
            cut = Some(result);
            break;
        }
    }
    let cut = cut.expect("no limit keeps the root and two queries");
    assert!(cut.size_limited);
    let ids: Vec<&str> = cut
        .query_statements
        .iter()
        .map(|q| q.statement_id.as_str())
        .collect();
    assert_eq!(ids, ["q1", "q2"]);
    assert!(
        cut.mutation_statements.is_empty(),
        "a later mutation was kept"
    );
    assert!(!cut.state_declarations.has("$s"), "later state was kept");
}

#[test]
fn references_to_a_mutation_are_counted_as_written() {
    let program = format!(
        "m=Mutation()\nroot = Stack([{}])",
        vec!["m"; 40_000].join(",")
    );
    let limit = 1_000_000;
    let limits = Limits::default().with_result_size(limit);
    let result = parse_with_limits(&program, &library(), None, limits);
    assert!(result.size_limited);
    let written = result.to_json().len();
    assert!(written <= limit + 1024, "{written} bytes of JSON");
}

#[test]
fn a_state_declaration_that_does_not_fit_is_not_declared_null() {
    let program = "root = Stack([])\nq0 = Query()\n$s = 5\nq1 = Query(\"t\", $s)";
    let whole = parse(program, &library(), None).to_json().len();
    for limit in 0..whole {
        let limits = Limits::default().with_result_size(limit);
        let result = parse_with_limits(program, &library(), None, limits);
        // Declared at all, it's after the query above it fit.
        assert!(
            !result.state_declarations.has("$s") || !result.query_statements.is_empty(),
            "limit {limit}: {}",
            result.to_json()
        );
    }
}
