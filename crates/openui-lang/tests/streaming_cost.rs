//! Counted allocation requests per push stay bounded as the stream grows.
//! Scheduling cannot change this measure. It does not time allocation-free scans.

mod common;

use std::sync::Arc;

use common::{conformance_dir, read};
use openui_lang::{Library, StreamParser};

const RUNS: usize = 7;
const BOUND: f64 = 2.5;

/// `text` in pushes of 4 code points.
fn chunks(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    chars
        .chunks(4)
        .map(|g| {
            let start = g[0].0;
            let end = g.last().map_or(start, |&(i, c)| i + c.len_utf8());
            &text[start..end]
        })
        .collect()
}

/// Allocation requests by this thread; scheduling does not affect the count.
fn count() -> f64 {
    test_alloc::allocation_calls() as f64
}

/// Mean cost of one push in one run of the whole stream.
fn cost_per_push(library: &Arc<Library>, chunks: &[&str]) -> f64 {
    let started = count();
    let mut parser = StreamParser::new(library.clone(), None);
    for chunk in chunks {
        std::hint::black_box(parser.push(chunk));
    }
    (count() - started) / chunks.len() as f64
}

/// Asserts a push on the larger text costs less than [`BOUND`] times one on
/// the smaller: `wrap` turns each program into the text streamed.
fn check_ratio(wrap: impl Fn(&str) -> String) {
    let conformance = conformance_dir();
    let schema =
        serde_json::from_str(&read(&conformance.join("imported/schemas/bench.json"))).unwrap();
    let library = Arc::new(Library::from_json_schema(&schema));
    let small_text = wrap(&read(&conformance.join("cases/synthetic/dashboard-x2.oui")));
    let large_text = wrap(&read(&conformance.join("cases/synthetic/dashboard-x8.oui")));
    let (small, large) = (chunks(&small_text), chunks(&large_text));
    let (mut small_cost, mut large_cost) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..RUNS {
        small_cost = small_cost.min(cost_per_push(&library, &small));
        large_cost = large_cost.min(cost_per_push(&library, &large));
    }
    let ratio = large_cost / small_cost;
    println!(
        "per push: {:.1} allocations on the 4× program, {:.1} allocations on the small one: {ratio:.2}×",
        large_cost, small_cost
    );
    assert!(
        ratio < BOUND,
        "a push on the 4× program costs {ratio:.2}× as much ({:.1} allocations against {:.1} allocations)",
        large_cost,
        small_cost
    );
}

#[test]
fn a_push_costs_about_the_same_on_a_program_four_times_the_size() {
    check_ratio(str::to_owned);
}

/// As models usually send a program: in a fence, with prose around it. Fence
/// extraction is line by line, so it stays linear too.
#[test]
fn so_it_does_in_a_fence_with_prose_around_it() {
    check_ratio(|program| {
        format!("Here is the dashboard:\n```openui\n{program}\n```\nIt updates live.")
    });
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
