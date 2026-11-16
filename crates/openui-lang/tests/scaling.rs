//! Allocation traffic stays within counted growth bounds over larger programs.
//! These witnesses do not measure allocation-free CPU scans.

use std::sync::Arc;

use openui_lang::{Library, StreamParser, parse};

const RUNS: usize = 5;
const BOUND: f64 = 8.0;

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

/// Allocation requests by this thread; scheduling does not affect the count.
fn count() -> f64 {
    test_alloc::allocation_calls() as f64
}

fn one_shot(text: &str) {
    std::hint::black_box(parse(text, &library(), None));
}

/// The whole text in one push, as `set` gives it after a reconnect.
fn one_push(text: &str) {
    let mut parser = StreamParser::new(library(), None);
    std::hint::black_box(parser.push(text));
    std::hint::black_box(parser.finish());
}

/// Asserts `run` on `shape(4 * n)` takes less than [`BOUND`] times what it
/// takes on `shape(n)`.
fn check_linear(n: usize, shape: impl Fn(usize) -> String, run: impl Fn(&str)) {
    let (small, large) = (shape(n), shape(4 * n));
    let time = |text: &str| {
        let started = count();
        run(text);
        count() - started
    };
    let (mut small_cost, mut large_cost) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..RUNS {
        small_cost = small_cost.min(time(&small));
        large_cost = large_cost.min(time(&large));
    }
    let ratio = large_cost / small_cost;
    println!(
        "{:.2} allocations at 4n, {:.2} allocations at n: {ratio:.2}×",
        large_cost, small_cost
    );
    assert!(
        ratio < BOUND,
        "four times the program costs {ratio:.2}× as much ({:.2} allocations against {:.2} allocations)",
        large_cost,
        small_cost
    );
}

/// A statement, `n` blank lines, then the `?` line that continues it: each
/// blank line once looked ahead past all the rest.
fn blank_lines_before_a_ternary(n: usize) -> String {
    format!("root = 1{}? 2 : 3\n", "\n".repeat(n))
}

#[test]
fn blank_lines_before_a_ternary_line_are_read_once() {
    let whole = parse(&blank_lines_before_a_ternary(3), &library(), None);
    assert_eq!(whole.meta.statement_count, 1);
    check_linear(20_000, blank_lines_before_a_ternary, one_shot);
}

#[test]
fn so_they_are_in_a_stream() {
    check_linear(20_000, blank_lines_before_a_ternary, one_push);
}

/// Lines of blank space between two statements: a stream once looked past
/// all the rest from each line break.
fn blank_space_between_statements(n: usize) -> String {
    format!("root = 1{}\nx = 2\n", "\n ".repeat(n))
}

#[test]
fn blank_space_between_statements_is_read_once_in_a_stream() {
    check_linear(20_000, blank_space_between_statements, one_push);
}

/// `n` statements, each referencing the next: completed in one push, each
/// once dropped the cached statements of the whole chain above it.
fn reference_chain(n: usize) -> String {
    let mut text = String::from("root = a0\n");
    for i in 0..n {
        text.push_str(&format!("a{i} = a{}\n", i + 1));
    }
    text
}

#[test]
fn a_reference_chain_completed_in_one_push_is_invalidated_once() {
    check_linear(5_000, reference_chain, one_push);
}

/// Dropping what's cached once for all a push changed drops what dropping
/// after each change did: every push gives what a fresh stream gives for the
/// text so far, whatever the push re-defines, and whichever references it
/// adds or takes away on the way.
#[test]
fn a_push_changing_several_statements_gives_what_a_fresh_stream_does() {
    let scripts: [&[&str]; 5] = [
        // `b` changes, then `a` stops referencing it.
        &[
            "root = Stack([a, c])\na = b\nb = Text(1)\nc = a\n",
            "b = Text(2)\na = Text(5)\n",
        ],
        // `a` starts referencing `c`, which starts referencing the changed `b`.
        &[
            "root = Stack([a])\na = b\nb = Text(1)\nc = Text(9)\n",
            "a = c\nb = Text(3)\nc = b\n",
        ],
        // A reference moves away and back within one push.
        &[
            "root = Stack([a])\na = b\nb = Text(1)\n",
            "a = Text(0)\nb = Text(2)\na = b\n",
        ],
        // A statement still arriving lies over one changed in the same push.
        &[
            "root = Stack([a, b])\na = b\nb = Text(1)\n",
            "b = Text(2)\nc = Stack([a",
            ", b])\nroot = c\n",
        ],
        // An unresolved name resolves, under a statement cached with it.
        &[
            "root = Stack([a, a])\na = [u, b]\nb = v\n",
            "v = Text(1)\nu = v\n",
            "b = u\n",
        ],
    ];
    for script in scripts {
        let mut parser = StreamParser::new(library(), None);
        let mut text = String::new();
        for push in script {
            text.push_str(push);
            let result = parser.push(push);
            let mut fresh = StreamParser::new(library(), None);
            assert_eq!(result, fresh.push(&text), "after {text:?}");
        }
        let mut fresh = StreamParser::new(library(), None);
        fresh.push(&text);
        assert_eq!(parser.finish(), fresh.finish(), "at the end of {text:?}");
    }
}

/// `n` queries, each reading the next one's result and one further on: the
/// search for loops once scanned its whole path for every read.
fn query_chain(n: usize) -> String {
    let mut text = String::from("root = Stack([])\n");
    for i in 0..n {
        text.push_str(&format!("q{i} = Query(q{}, q{})\n", i + 1, (i * 7) % n));
    }
    text
}

#[test]
fn a_chain_of_queries_is_searched_for_loops_in_one_pass() {
    // The loops found, and their order, are what the walk gave before it
    // was made linear.
    let small = parse(
        "root = Stack([])\na = Query(b, c)\nb = Query(c, a)\nc = Query(a)",
        &library(),
        None,
    );
    let cycles: Vec<(&str, Option<&str>)> = small
        .meta
        .errors
        .iter()
        .filter(|e| e.code.as_str() == "query-cycle")
        .map(|e| (e.message.as_str(), e.statement_id.as_deref()))
        .collect();
    let closing = "read each other's results, so none of them can run first. They do not run";
    assert_eq!(
        cycles,
        [
            (format!("Queries a, b, c {closing}").as_str(), Some("a")),
            (format!("Queries a, b {closing}").as_str(), Some("a")),
        ]
    );
    check_linear(4_000, query_chain, one_shot);
}

/// `names` unresolved names at the bottom of a chain of `levels` references.
fn unresolved_under_a_chain(names: usize, levels: usize) -> String {
    let mut text = String::from("root = a0\n");
    for i in 0..levels {
        text.push_str(&format!("a{i} = a{}\n", i + 1));
    }
    text.push_str(&format!("a{levels} = [{}]\n", vec!["u"; names].join(",")));
    text
}

/// Every statement up the chain once held its own copy of the names: 75 MB
/// for these 40 KB, and 5 GB at the program-size limit. Shared, they fit
/// under the binary's 64 MiB allocation cap many times over.
#[test]
fn unresolved_names_are_held_once_however_long_the_chain_above_them() {
    let program = unresolved_under_a_chain(20_000, 80);
    let result = parse(&program, &library(), None);
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(result.meta.unresolved.len(), 20_000);
    assert!(result.meta.unresolved.iter().all(|name| name == "u"));

    // A stream keeps the chain's statements cached, and gives the same.
    let mut parser = StreamParser::new(library(), None);
    parser.push(&program);
    assert_eq!(parser.finish(), result);
}

/// The names come in the order they're met, a reached statement's where it's
/// reached, each time it is.
#[test]
fn unresolved_names_keep_their_order_through_reached_statements() {
    let program = "root = [x, a, y, a, z]\na = [p, b, q]\nb = [r]";
    let result = parse(program, &library(), None);
    assert_eq!(
        result.meta.unresolved,
        ["x", "p", "r", "q", "y", "p", "r", "q", "z"]
    );
    let mut parser = StreamParser::new(library(), None);
    for line in program.split_inclusive('\n') {
        parser.push(line);
    }
    assert_eq!(parser.finish(), result);
}

/// `n` references to the top of a chain of `levels` aliases that ends in an
/// unresolved name.
fn replays_over_an_alias_chain(n: usize, levels: usize) -> String {
    let mut text = format!("root = [{}]\n", vec!["a0"; n].join(","));
    for i in 0..levels {
        text.push_str(&format!("a{i} = a{}\n", i + 1));
    }
    text
}

/// A statement whose only names are one reached statement's shares its list
/// as it is; the names read the same through any number of aliases.
#[test]
fn unresolved_names_read_the_same_through_a_chain_of_aliases() {
    let program = replays_over_an_alias_chain(3, 5);
    let result = parse(&program, &library(), None);
    assert_eq!(result.meta.unresolved, ["a5", "a5", "a5"]);
    let mut parser = StreamParser::new(library(), None);
    for line in program.split_inclusive('\n') {
        parser.push(line);
    }
    assert_eq!(parser.finish(), result);
}

/// A chain the depth limit cuts is never cached, so every reference to it
/// walks it again, and an alias writes nothing: 35 s and some 20 GB for a
/// program at the program-size limit. Each reference followed counts against
/// the result size, so the walks stop where that limit does: a sixth of a
/// second there, within the binary's allocation cap here.
#[test]
fn references_to_a_chain_the_depth_limit_cuts_stop_at_the_size_limit() {
    let program = replays_over_an_alias_chain(5_000, 260);
    let result = parse(&program, &library(), None);
    assert!(result.depth_limited);
    assert!(result.size_limited, "5,000 walks of 254 aliases, uncounted");

    // A chain short enough to finish is walked once and reused, and its
    // references fit.
    let short = replays_over_an_alias_chain(20, 20);
    let whole = parse(&short, &library(), None);
    assert!(!whole.depth_limited && !whole.size_limited);

    let mut parser = StreamParser::new(library(), None);
    for line in program.split_inclusive('\n') {
        parser.push(line);
    }
    assert_eq!(parser.finish(), result);
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

/// Each element already knows whether its subtree has dynamic props.
#[test]
fn nested_static_elements_check_dynamic_props_once() {
    let shape = |n| {
        format!(
            "root = {}Text(\"x\"){}",
            "Stack([".repeat(n),
            "])".repeat(n)
        )
    };
    check_linear(200, shape, |text| {
        let result = openui_lang::parse_with_limits(
            text,
            &library(),
            None,
            openui_lang::Limits::default().with_depth(4000),
        );
        assert!(!result.size_limited && !result.depth_limited);
        std::hint::black_box(result);
    });
}

/// Validation keeps an element unchanged; equality must compare its identity
/// rather than walking its entire subtree again at each parent.
#[test]
fn validating_nested_elements_compares_identity_once() {
    let lib = Library::from_json_schema(&serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}},
        "Stack": {"properties": {"children": {"type": "array", "items": {}}}}
    }}));
    let shape = |n| {
        format!(
            "root = {}Text(\"x\"){}",
            "Stack([".repeat(n),
            "])".repeat(n)
        )
    };
    check_linear(200, shape, |text| {
        let result = openui_lang::parse_with_limits(
            text,
            &lib,
            None,
            openui_lang::Limits::default().with_depth(4000),
        );
        assert!(!result.size_limited && !result.depth_limited);
        std::hint::black_box(result);
    });
}

mod scaling_cases;

/// Count allocations and peak requested heap, independently of elapsed time.
/// The unit companion counts traversal steps too (including the sharing mutant).
#[test]
fn alias_references_at_raised_limits_allocate_linearly() {
    for repeated in [false, true] {
        let mut measurements = Vec::new();
        for n in if repeated { [50, 200] } else { [2_000, 8_000] } {
            let text = scaling_cases::aliases(n, repeated);
            let limits = openui_lang::Limits::default()
                .with_depth(if repeated { 256 } else { 20_000 })
                .with_result_size(1024 * 1024 * 1024);
            let lib = library();
            test_alloc::reset_peak();
            let before = test_alloc::allocation_calls();
            let result = openui_lang::parse_with_limits(&text, &lib, None, limits);
            let allocations = test_alloc::allocation_calls() - before;
            let peak = test_alloc::peak();
            assert!(!result.depth_limited && !result.size_limited);
            assert_eq!(
                result.meta.unresolved.len(),
                if repeated { 100 * n } else { n }
            );
            assert!(result.meta.unresolved.iter().all(|s| s == "missing"));
            assert!(result.meta.orphaned.is_empty());
            assert!(
                peak < 16 * 1024 * 1024,
                "n={n}, repeated={repeated}: {peak}"
            );
            println!("aliases n={n} repeated={repeated}: allocations={allocations}, peak={peak}");
            measurements.push(allocations);
        }
        assert!(measurements[1] < 5 * measurements[0], "{measurements:?}");
    }
}

#[test]
fn cut_alias_references_keep_heap_proportional_to_result_room() {
    let lib = library();
    for room in [64 * 1024, 256 * 1024, 1024 * 1024] {
        let text = scaling_cases::aliases(300, true);
        test_alloc::reset_peak();
        let result = openui_lang::parse_with_limits(
            &text,
            &lib,
            None,
            openui_lang::Limits::default().with_result_size(room),
        );
        let peak = test_alloc::peak();
        assert!(result.size_limited && result.depth_limited);
        // Includes input/table/cache storage; the fixed allowance covers this
        // 120 KB program, and each followed reference still costs 32 bytes.
        assert!(
            peak < 2 * room + 9 * 1024 * 1024,
            "room={room}, peak={peak}"
        );
        println!("cut aliases room={room}: peak={peak}");
    }
}

/// These bytes came from archived main, including the unchanged cut paths.
#[test]
fn alias_limit_results_keep_their_main_bytes() {
    let lib = library();
    let pins = include_str!("support/alias-main-limits.tsv");
    let mut checks = 0;
    for line in pins.lines().filter(|line| !line.starts_with('#')) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let n = fields[0].parse().unwrap();
        let repeated = fields[1].parse().unwrap();
        let depth = fields[2].parse().unwrap();
        let room = fields[3].parse().unwrap();
        let text = scaling_cases::aliases(n, repeated);
        let result = openui_lang::parse_with_limits(
            &text,
            &lib,
            None,
            openui_lang::Limits::default()
                .with_depth(depth)
                .with_result_size(room),
        );
        let hash = result.to_json().bytes().fold(2166136261_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16777619)
        });
        assert_eq!(format!("{hash:08x}"), fields[4], "{line}");
        checks += 1;
    }
    assert_eq!(checks, 72);
}

#[test]
fn cached_depth_cuts_respect_open_paths_depth_and_stream_changes() {
    let pins: serde_json::Value =
        serde_json::from_str(include_str!("support/alias-context-main.json")).unwrap();
    let limits = openui_lang::Limits::default()
        .with_depth(32)
        .with_result_size(65536);
    for case in pins.as_array().unwrap() {
        let text = case["program"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        assert_eq!(
            openui_lang::parse_with_limits(text, &library(), None, limits).to_json(),
            expected,
            "{}",
            case["name"]
        );
        let mut stream = StreamParser::with_limits(library(), None, limits);
        stream.push(text);
        assert_eq!(stream.finish().to_json(), expected);
        for next in [
            text.replace("missing", "7"),
            text.replace("a3=b", "a3=null"),
            text.to_owned(),
        ] {
            stream.set(&next);
            assert_eq!(
                stream.finish(),
                openui_lang::parse_with_limits(&next, &library(), None, limits)
            );
        }
    }
}

#[test]
fn replayed_depth_cuts_keep_their_flags_on_later_stream_results() {
    let text = scaling_cases::aliases(50, true);
    let limits = openui_lang::Limits::default()
        .with_depth(32)
        .with_result_size(1024 * 1024 * 1024);
    let mut stream = StreamParser::with_limits(library(), None, limits);
    let first = stream.push(&text);
    assert!(first.depth_limited && !first.size_limited);
    let text = text + "unrelated=7\n";
    stream.push("unrelated=7\n");
    // The alias chain is now completed; the pending unrelated statement's
    // temporary overlay cannot invalidate it on the next observation.
    let later = stream.push("another=8\n");
    assert!(later.depth_limited && !later.size_limited);
    let mut fresh = StreamParser::with_limits(library(), None, limits);
    assert_eq!(later, fresh.push(&(text + "another=8\n")));
}

#[test]
fn depth_cut_containers_keep_fresh_identities_at_each_reach() {
    let mut text = String::from("root=Stack([a0,a0])\n");
    for i in 0..50 {
        text.push_str(&format!("a{i}=Stack(a{})\n", i + 1));
    }
    text.push_str("a50=missing\n");
    let result = openui_lang::parse_with_limits(
        &text,
        &library(),
        None,
        openui_lang::Limits::default()
            .with_depth(32)
            .with_result_size(65536),
    );
    assert!(result.depth_limited && !result.size_limited);
    let openui_lang::Value::Arr(children) =
        result.root.as_ref().unwrap().props.get("children").unwrap()
    else {
        panic!("children")
    };
    let [
        openui_lang::Value::Element(first),
        openui_lang::Value::Element(second),
    ] = children.as_slice()
    else {
        panic!("two elements")
    };
    assert!(!Arc::ptr_eq(first, second));
    assert_eq!(first, second);
}

#[test]
fn raised_alias_depth_cuts_keep_small_results_and_input_sized_heap() {
    let text = scaling_cases::aliases(32_000, false);
    test_alloc::reset_peak();
    let result = openui_lang::parse_with_limits(
        &text,
        &library(),
        None,
        openui_lang::Limits::default()
            .with_depth(20_000)
            .with_result_size(1024 * 1024 * 1024),
    );
    let peak = test_alloc::peak();
    assert!(result.depth_limited && result.size_limited);
    assert_eq!(
        result.to_json().trim(),
        include_str!("support/raised-alias-main.json").trim()
    );
    assert!(peak < 48 * 1024 * 1024, "peak={peak}");
    println!("32,000 raised aliases: {peak} peak heap bytes");
}

#[test]
fn raised_alias_cuts_preserve_reference_prices_at_each_boundary() {
    let lib = library();
    let mut checked = 0;
    for line in include_str!("support/raised-alias-prices-main.tsv")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let f = line.split_whitespace().collect::<Vec<_>>();
        let n = f[0].parse().unwrap();
        let repeated = f[1].parse().unwrap();
        let expression: bool = f[2].parse().unwrap();
        let limits = openui_lang::Limits::default()
            .with_depth(f[3].parse().unwrap())
            .with_result_size(f[4].parse().unwrap());
        let mut text = scaling_cases::aliases(n, repeated);
        if expression {
            text = text
                .replacen("root=Stack([", "root=Stack(@Count([", 1)
                .replacen("])\n", "]))\n", 1);
        }
        let result = openui_lang::parse_with_limits(&text, &lib, None, limits);
        let hash = result.to_json().bytes().fold(2166136261u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16777619)
        });
        assert_eq!(format!("{hash:08x}"), f[5], "{line}");
        assert_eq!(
            result.depth_limited,
            f[6].parse::<bool>().unwrap(),
            "{line}"
        );
        assert_eq!(result.size_limited, f[7].parse::<bool>().unwrap(), "{line}");
        let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
        stream.push(&text);
        assert_eq!(stream.finish(), result, "stream {line}");
        checked += 1;
    }
    assert_eq!(checked, 552);
}

#[test]
fn raised_alias_cuts_keep_failed_lookup_marks_flags_and_required_checks() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("support/raised-alias-edges-main.json")).unwrap();
    for row in rows.as_array().unwrap() {
        let limits = openui_lang::Limits::default()
            .with_depth(row["depth"].as_u64().unwrap() as usize)
            .with_result_size(row["room"].as_u64().unwrap() as usize);
        let text = row["program"].as_str().unwrap();
        let result = openui_lang::parse_with_limits(text, &library(), None, limits);
        assert_eq!(
            result.to_json(),
            row["expected"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        assert_eq!(
            result.depth_limited,
            row["depth_limited"].as_bool().unwrap()
        );
        assert_eq!(result.size_limited, row["size_limited"].as_bool().unwrap());
        let mut stream = StreamParser::with_limits(library(), None, limits);
        stream.push(text);
        assert_eq!(stream.finish(), result);
    }
}

#[test]
fn empty_memoized_error_records_preserve_later_reach_suppression() {
    let pins: serde_json::Value =
        serde_json::from_str(include_str!("support/empty-errors-main.json")).unwrap();
    let lib = Arc::new(Library::from_json_schema(&pins["schema"]));
    let cases = pins["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let text = case["program"].as_str().unwrap();
        let limits =
            openui_lang::Limits::default().with_depth(case["depth"].as_u64().unwrap() as usize);
        let result = openui_lang::parse_with_limits(text, &lib, None, limits);
        assert_eq!(result.to_json(), case["expected"].as_str().unwrap());
        assert_eq!(
            result.depth_limited,
            case["depth_limited"].as_bool().unwrap()
        );
        assert_eq!(result.size_limited, case["size_limited"].as_bool().unwrap());
        let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
        stream.push(text);
        assert_eq!(stream.finish(), result);
    }
}
