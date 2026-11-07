//! The work limit (ADR 0001): evaluating state defaults counts everything it
//! does, not only what it builds, so a small program can't spend seconds or
//! gigabytes on it. Costs are stated in steps against the input's size,
//! observed only through the public API: the smallest work limit a program
//! parses under without being cut.

use std::sync::Arc;

use openui_lang::{Library, Limits, MAX_RESULT_SIZE, ParseResult, parse, parse_with_limits};

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

fn parse_with_work(program: &str, work: usize) -> ParseResult {
    parse_with_limits(program, &library(), None, Limits::default().with_work(work))
}

/// `[0, 1, …, n - 1]`.
fn list(n: usize) -> String {
    let items: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    format!("[{}]", items.join(", "))
}

/// An `@Each` over `n` items whose template assigns an `n`-item list: each
/// item reads the assignment's expression as a value, so it costs `n * n`.
fn each_reading_an_expression(n: usize) -> String {
    format!(
        "root = Stack([])\n$a = @Each({}, \"x\", @Count([$t = {}]))\n$b = 1 + 1",
        list(n),
        list(n)
    )
}

#[test]
fn running_out_of_work_cuts_the_default_and_flags_the_result() {
    let program = each_reading_an_expression(100);
    let whole = parse(&program, &library(), None);
    assert!(!whole.size_limited);
    assert!(whole.to_json().contains("\"$a\":[1,1,1"));

    let cut = parse_with_work(&program, 1_000);
    assert!(cut.size_limited);
    // The default is null, and so is every expression default after it.
    let json = cut.to_json();
    assert!(json.contains("\"$a\":null"), "{json}");
    assert!(json.contains("\"$b\":null"), "{json}");

    // Raised, it finishes as the default does.
    assert_eq!(parse_with_work(&program, 10_000_000), whole);
}

/// The fewest steps `program` parses in without being cut, found by
/// bisection: what its evaluating costs.
fn steps(program: &str) -> usize {
    let whole = |work| !parse_with_work(program, work).size_limited;
    assert!(whole(64_000_000), "cut even at 64 million steps");
    let (mut lo, mut hi) = (0, 64_000_000);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if whole(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

#[test]
fn each_reading_an_expression_costs_its_items_times_its_template() {
    // `n` items, each reading an `n`-item expression: a step per node of the
    // template per item (`@Count`'s list is there twice, as its argument and
    // by name), not a node's JSON written and read back.
    let small = steps(&each_reading_an_expression(100));
    let large = steps(&each_reading_an_expression(200));
    assert!(small <= 3 * 100 * 100, "{small}");
    assert!(large <= 3 * 200 * 200, "{large}");
    let ratio = large as f64 / small as f64;
    assert!((3.0..5.0).contains(&ratio), "{small} -> {large}");
}

/// A default holding a component of `children` texts, and one reading it
/// `reads` times.
fn reading_a_component(children: usize, reads: usize) -> String {
    let texts: Vec<String> = (0..children).map(|i| format!("Text(\"t{i}\")")).collect();
    let refs = vec!["$c"; reads].join(", ");
    format!(
        "root = Stack([])\n$c = Stack([{}])\n$a = @Count([{refs}])",
        texts.join(", ")
    )
}

#[test]
fn a_component_read_many_times_is_converted_once() {
    // Reading it costs its size once, then a few steps a read.
    let few = steps(&reading_a_component(300, 10));
    let many = steps(&reading_a_component(300, 1_000));
    assert!(few > 300, "{few}");
    assert!(many - few <= 10 * 990, "{few} -> {many}");
}

#[test]
fn the_first_component_read_costs_its_size() {
    // Subtract constructing the default itself to isolate its first read.
    let read_cost = |children| {
        let program = reading_a_component(children, 1);
        let construction = program.split("\n$a =").next().unwrap();
        steps(&program) - steps(construction)
    };
    let small = read_cost(100);
    let large = read_cost(200);
    assert!(small >= 100 * 40, "{small}");
    let ratio = large as f64 / small as f64;
    assert!((1.8..2.2).contains(&ratio), "{small} -> {large}");
}

/// A default sorting, or filtering, `n` numbers in a scrambled order.
fn sorting(n: usize, how: &str) -> String {
    let items: Vec<String> = (0..n).map(|i| ((i * 7919) % n).to_string()).collect();
    let call = match how {
        "sort" => format!("@Sort([{}], \"\", \"asc\")", items.join(", ")),
        _ => format!("@Filter([{}], \"\", \">\", {})", items.join(", "), n / 2),
    };
    format!("root = Stack([])\n$a = @Count({call})")
}

#[test]
fn sorting_costs_a_step_per_comparison_and_filtering_one_per_item() {
    let sort = (
        steps(&sorting(2_000, "sort")),
        steps(&sorting(4_000, "sort")),
    );
    // `n log n` comparisons: doubling `n` a little more than doubles it.
    assert!(sort.0 > 2_000 * 10, "{sort:?}");
    let ratio = sort.1 as f64 / sort.0 as f64;
    assert!((2.0..2.5).contains(&ratio), "{sort:?}");

    let filter = (
        steps(&sorting(2_000, "filter")),
        steps(&sorting(4_000, "filter")),
    );
    let ratio = filter.1 as f64 / filter.0 as f64;
    assert!((1.8..2.2).contains(&ratio), "{filter:?}");
}

/// The slow `parse` input from coverage-guided fuzzing in the 1.1 long runs
/// (`coverage-parse-timeout-94563fe…`), shrunk from 5.7 KB to 629 bytes:
/// `@Each`s nested in index expressions, each copying a template that holds
/// the next, and sorts. Before the work limit it took 0.8 s in release (the
/// whole input 3.6 s, and 18.5 s with a 678 MB peak in the fuzzing build).
#[test]
fn the_slow_fuzz_input_is_cut_by_default_and_whole_within_four_million_steps() {
    let schema = serde_json::from_str(include_str!("slow/fuzz-library.json")).unwrap();
    let library = Library::from_json_schema(&schema);
    let program = include_str!("slow/parse-94563fe.oui");
    let parse_under = |limits| parse_with_limits(program, &library, None, limits);
    // The defaults cut it: its 64 KiB of result room and its million steps
    // are both too few.
    assert!(parse_under(Limits::default()).size_limited);
    // With room to spare, the work is what cuts it: its 629 bytes get the
    // work limit's floor, a million steps, and it needs more.
    let roomy = Limits::default().with_result_size(MAX_RESULT_SIZE);
    let cut = parse_under(roomy);
    assert!(cut.size_limited);
    assert!(cut.to_json().contains("\"$s\":null"));
    // Four million are enough.
    let whole = parse_under(roomy.with_work(4_000_000));
    assert!(!whole.size_limited);
    assert!(whole.to_json().contains("\"$s\":[[null,null],[null,null]]"));
}

#[test]
fn the_default_work_limit_scales_with_the_program() {
    let limits = Limits::default();
    assert_eq!(limits.work(), None);
    // The larger of a million steps and 1,000 per byte, never above 64 million.
    assert_eq!(limits.work_for(0), 1_000_000);
    assert_eq!(limits.work_for(500), 1_000_000);
    assert_eq!(limits.work_for(5_000), 5_000_000);
    assert_eq!(limits.work_for(64_000), 64_000_000);
    assert_eq!(limits.work_for(1024 * 1024), 64_000_000);
    // Set, it's the same for every program.
    let set = limits.with_work(10);
    assert_eq!(set.work(), Some(10));
    assert_eq!(set.work_for(1024 * 1024), 10);
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
