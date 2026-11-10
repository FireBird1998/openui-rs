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

#[test]
fn placeholders_for_what_was_cut_are_counted() {
    // References to `big` fill the limit; then each of a long list of them
    // is cut, inside calls that write the list again at every level.
    let program = format!(
        "big = \"{}\"\nroot = Stack([{}Text($x ? {}[{}]{} : null)])",
        "x".repeat(6000),
        "big,".repeat(33),
        "Text(".repeat(4),
        vec!["big"; 3000].join(","),
        ")".repeat(4)
    );
    let limit = 200_000;
    let limits = Limits::default().with_result_size(limit);
    let result = parse_with_limits(&program, &library(), None, limits);
    assert!(result.size_limited);
    let written = result.to_json().len();
    assert!(written <= limit + 1024, "{written} bytes of JSON");
}

#[test]
fn orphaned_names_are_counted() {
    let program = format!(
        "root = Stack([])\n{}",
        (0..5000).map(|i| format!("a{i}=1\n")).collect::<String>()
    );
    let whole = parse(&program, &library(), None);
    assert_eq!(whole.meta.orphaned.len(), 5000);
    let limit = 20_000;
    let limits = Limits::default().with_result_size(limit);
    let result = parse_with_limits(&program, &library(), None, limits);
    assert!(result.size_limited);
    let written = result.to_json().len();
    assert!(written <= limit + 1024, "{written} bytes of JSON");
    // The first of them, in order.
    let kept = result.meta.orphaned.len();
    assert!(kept > 1000 && kept < 5000, "{kept} kept");
    assert_eq!(result.meta.orphaned[..], whole.meta.orphaned[..kept]);
}

/// A small deterministic generator (xorshift).
struct Random(u64);

impl Random {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len())]
    }

    /// `n` expressions, separated by commas.
    fn list(&mut self, n: usize, depth: usize, out: &mut String) {
        for i in 0..n {
            if i > 0 {
                out.push(',');
            }
            self.expression(depth, out);
        }
    }

    /// Every kind of expression: literals with characters JSON escapes,
    /// references that resolve, don't, or name a query or mutation, lists,
    /// objects, components, builtins and operators.
    fn expression(&mut self, depth: usize, out: &mut String) {
        const NAMES: [&str; 8] = ["a", "b", "c", "d", "q", "m", "big", "missing"];
        const STATE: [&str; 3] = ["$s", "$t", "$longername"];
        let kinds = if depth == 0 { 8 } else { 18 };
        let depth = depth.saturating_sub(1);
        match self.below(kinds) {
            0 | 1 => out.push_str(self.pick(&NAMES)),
            2 => out.push_str(self.pick(&STATE)),
            3 => out.push_str(self.pick(&["1", "-2.5", "1000000000000000", "true", "null"])),
            4 => {
                out.push('"');
                for _ in 0..self.below(6) {
                    out.push_str(self.pick(&["x", "\\n", "\\u0001", "é", "\\\""]));
                }
                out.push('"');
            }
            5 => out.push_str("[]"),
            6 => out.push_str("{}"),
            7 => out.push_str("Text()"),
            8 | 9 => {
                let n = if self.below(8) == 0 { 60 } else { 5 };
                let n = self.below(n);
                out.push('[');
                self.list(n, depth, out);
                out.push(']');
            }
            10 => {
                out.push('{');
                for i in 0..self.below(4) {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(self.pick(&["k:", "longkey:", "\"a\\u0001\":"]));
                    self.expression(depth, out);
                }
                out.push('}');
            }
            11 | 12 => {
                out.push_str(self.pick(&["Text(", "Stack(", "Nope(", "@Sum(", "@Each(", "@Nope("]));
                let n = self.below(4);
                self.list(n, depth, out);
                out.push(')');
            }
            13 => {
                self.expression(depth, out);
                out.push_str(self.pick(&[" + ", " == ", " && "]));
                self.expression(depth, out);
            }
            14 => {
                self.expression(depth, out);
                out.push_str(" ? ");
                self.expression(depth, out);
                out.push_str(" : ");
                self.expression(depth, out);
            }
            15 => {
                out.push_str(self.pick(&["!", "-"]));
                self.expression(depth, out);
            }
            16 => {
                self.expression(depth, out);
                out.push_str(self.pick(&[".f", ".\"a\\u0001\"", "[0]"]));
            }
            _ => {
                out.push_str(self.pick(&STATE));
                out.push_str(" = ");
                self.expression(depth, out);
            }
        }
    }

    /// Up to ten statements of every kind, some names defined twice.
    fn program(&mut self) -> String {
        let mut out = String::new();
        for _ in 0..1 + self.below(10) {
            let depth = self.below(5);
            let name = match self.below(9) {
                0 | 1 => "root = ",
                2 => self.pick(&["$s = ", "$t = ", "$longername = "]),
                3 => "q = Query(",
                4 => "m = Mutation(",
                5 => "big = \"",
                6 => self.pick(&["o1 = ", "o2 = ", "o3 = "]),
                _ => self.pick(&["a = ", "b = ", "c = ", "d = "]),
            };
            out.push_str(name);
            if name.ends_with('(') {
                let n = self.below(5);
                self.list(n, depth, &mut out);
                out.push(')');
            } else if name.ends_with('"') {
                out.push_str(&"x".repeat(self.below(400)));
                out.push('"');
            } else {
                self.expression(depth, &mut out);
            }
            out.push('\n');
        }
        out
    }
}

#[test]
fn no_generated_program_writes_far_past_its_limit() {
    // What a result writes whatever its limit: its own keys, and a few bytes
    // for each list open when the limit is passed.
    const ENVELOPE: usize = 512;
    let lib = library();
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    let mut cut = 0;
    for _ in 0..3000 {
        let program = random.program();
        let whole = parse(&program, &lib, None).to_json().len();
        for _ in 0..3 {
            let limit = random.below(whole + 50);
            let limits = Limits::default().with_result_size(limit);
            let result = parse_with_limits(&program, &lib, None, limits);
            let json = result.to_json();
            assert!(
                json.len() <= limit + ENVELOPE,
                "{} bytes of JSON, limit {limit}:\n{program}",
                json.len()
            );
            cut += usize::from(result.size_limited);
            // Arriving in pieces, each statement cached and replayed, the
            // result is the one the whole text gives a fresh stream.
            let mut whole_text = StreamParser::with_limits(lib.clone(), None, limits);
            let expected = whole_text.push(&program);
            let mut pieces = StreamParser::with_limits(lib.clone(), None, limits);
            let chars: Vec<char> = program.chars().collect();
            let mut last = None;
            for piece in chars.chunks(1 + random.below(9)) {
                last = Some(pieces.push(&piece.iter().collect::<String>()));
            }
            assert_eq!(
                last.unwrap().to_json(),
                expected.to_json(),
                "limit {limit}:\n{program}"
            );
        }
    }
    assert!(cut > 3000, "only {cut} results were cut");
}

/// How far past `limit` the JSON of `program` goes, and whether it's flagged.
fn past(program: &str, lib: &Library, limit: usize) -> (isize, bool) {
    let limits = Limits::default().with_result_size(limit);
    let result = parse_with_limits(program, lib, None, limits);
    let written = result.to_json().len() as isize;
    (written - limit as isize, result.size_limited)
}

#[test]
fn every_part_of_a_result_stays_near_its_limit() {
    let lib = library();
    let with_default = Library::from_json_schema(&serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"default": "d".repeat(2000)}}, "required": ["value"]},
        "Stack": {"properties": {"children": {}}}
    }}));
    let cases: [(&str, String, &Library, usize, isize); 4] = [
        (
            "a lazy builtin's extra arguments",
            format!(
                "root = Stack([@Each([], \"v\", 1, {})])",
                vec!["1"; 20_000].join(",")
            ),
            &lib,
            1000,
            512,
        ),
        (
            "orphans after evaluated state defaults",
            format!(
                "$a = \"{}\"\n$b = [$a,$a,$a,$a]\nroot = Stack([])\n{}",
                "x".repeat(3000),
                (0..2000).map(|i| format!("o{i} = 1\n")).collect::<String>()
            ),
            &lib,
            20_000,
            512,
        ),
        // An element is written around children cut to fit: its own JSON, 66
        // bytes besides its name, for each level open, the depth limit's 256
        // at most.
        (
            "elements nested to the depth limit",
            "root = ".to_owned() + &"Stack([".repeat(400),
            &lib,
            2000,
            256 * 66 + 512,
        ),
        // One default past the limit at most: the library's, not the program's.
        (
            "a library default for each component",
            format!("root = Stack([{}])", vec!["Text()"; 100].join(",")),
            &with_default,
            20_000,
            2000 + 512,
        ),
    ];
    for (shape, program, lib, limit, allowed) in cases {
        let (over, flagged) = past(&program, lib, limit);
        assert!(over <= allowed, "{shape}: {over} bytes past the limit");
        assert!(flagged, "{shape}: not flagged");
    }
}

/// Every size limit that cuts `program`, and every depth limit that does:
/// no result says a required value is null or missing, and a stream given
/// the text in pieces says what a one-shot parse does.
fn no_limit_blames_the_program(program: &str, lib: &Arc<Library>) {
    let whole = parse(program, lib, None);
    assert!(whole.meta.errors.is_empty(), "{:?}", whole.meta.errors);
    let sizes = (0..whole.to_json().len() + 200).map(|n| Limits::default().with_result_size(n));
    let depths = (0..12).map(|n| Limits::default().with_depth(n));
    let mut cut = 0;
    for limits in sizes.chain(depths) {
        let result = parse_with_limits(program, lib, None, limits);
        if !result.size_limited && !result.depth_limited {
            continue;
        }
        cut += 1;
        let json = result.to_json();
        assert!(!json.contains("cannot be null"), "{program}: {json}");
        assert!(!json.contains("missing required"), "{program}: {json}");
        let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
        let chars: Vec<char> = program.chars().collect();
        let mut last = None;
        for piece in chars.chunks(7) {
            last = Some(stream.push(&piece.iter().collect::<String>()));
        }
        assert_eq!(last.unwrap().to_json(), json, "{program}: streamed");
    }
    assert!(cut > 100, "{program}: {cut} limits cut it");
}

#[test]
fn a_required_value_a_limit_cut_is_not_reported_as_null() {
    let lib = Arc::new(Library::from_json_schema(&serde_json::json!({
        "$defs": {
            "Btn": {"properties": {"n": {"type": "number"}, "label": {"type": "string"}},
                    "required": ["n", "label"]},
            "Card": {"properties": {"head": {"type": "object",
                        "properties": {"title": {"type": "string"}}, "required": ["title"]}},
                     "required": ["head"]},
            "Stack": {"properties": {"children": {}}}
        },
        "functions": {"Label": {"params": {"properties": {"text": {"type": "string"}},
                                           "required": ["text"]}}}
    })));
    let long = "x".repeat(300);
    // The model wrote each of them; it's a limit that cut it.
    for program in [
        format!("root = Stack([Btn(5, \"{long}\")])"),
        format!("root = Stack([Card({{title: \"{long}\"}})])"),
        format!("root = Stack([Btn(5, @Label(\"{long}\"))])"),
        format!("t = \"{long}\"\nroot = Stack([Btn(5, t), Card({{title: t}})])"),
    ] {
        no_limit_blames_the_program(&program, &lib);
    }
    // One the model did write as null is still reported, cut or not.
    for program in [
        "root = Stack([Btn(5, null)])",
        "root = Stack([Card({title: null})])",
    ] {
        let null = parse(program, &lib, None);
        assert_eq!(null.meta.errors.len(), 1, "{program}");
        assert!(null.meta.errors[0].message.contains("cannot be null"));
    }
}
