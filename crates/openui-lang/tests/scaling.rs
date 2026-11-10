//! Work and memory grow with the program, not with its square, on shapes
//! built to make them: a program four times the size takes about four times
//! as long, not sixteen. Each shape took seconds, or gigabytes, at the
//! program-size limit before it was fixed.
//!
//! Time is the thread's CPU time (on Unix; wall time elsewhere), the two
//! sizes measured in turn and the fastest run of each counted, as in
//! `streaming_cost`. The bound, 8, is halfway between linear and quadratic.
//! Memory is held by the binary's 64 MiB allocation cap.

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

/// Seconds of CPU time this thread has used.
#[cfg(unix)]
fn now() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec for the call to fill in.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(ok, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// Seconds since the first call: wall time, where thread CPU time isn't at hand.
#[cfg(not(unix))]
fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
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
        let started = now();
        run(text);
        now() - started
    };
    let (mut small_cost, mut large_cost) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..RUNS {
        small_cost = small_cost.min(time(&small));
        large_cost = large_cost.min(time(&large));
    }
    let ratio = large_cost / small_cost;
    println!(
        "{:.2} ms at 4n, {:.2} ms at n: {ratio:.2}×",
        large_cost * 1e3,
        small_cost * 1e3
    );
    assert!(
        ratio < BOUND,
        "four times the program costs {ratio:.2}× as much ({:.2} ms against {:.2} ms)",
        large_cost * 1e3,
        small_cost * 1e3
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
