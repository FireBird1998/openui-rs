//! Streaming work grows with the program, not with its square: a push costs
//! about the same early in a stream as late, so a program four times the
//! size takes about four times as long, not sixteen. The two synthetic
//! programs differ only in size and one level of nesting; a parser that
//! redid the whole tree on every push would cost about four times more per
//! push on the larger one. The bound leaves room for the extra level and for
//! timing noise while still catching that.

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{conformance_dir, read};
use openui_lang::{Library, StreamParser};

/// Mean cost of one push (chunks of 4 code points), best of three runs.
fn cost_per_push(library: &Arc<Library>, text: &str) -> f64 {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let chunks: Vec<&str> = chars
        .chunks(4)
        .map(|g| {
            let start = g[0].0;
            let end = g.last().map_or(start, |&(i, c)| i + c.len_utf8());
            &text[start..end]
        })
        .collect();
    (0..3)
        .map(|_| {
            let started = Instant::now();
            let mut parser = StreamParser::new(library.clone(), None);
            for chunk in &chunks {
                std::hint::black_box(parser.push(chunk));
            }
            started.elapsed().as_secs_f64() / chunks.len() as f64
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_push_costs_about_the_same_on_a_program_four_times_the_size() {
    let conformance = conformance_dir();
    let schema =
        serde_json::from_str(&read(&conformance.join("imported/schemas/bench.json"))).unwrap();
    let library = Arc::new(Library::from_json_schema(&schema));
    let small = read(&conformance.join("cases/synthetic/dashboard-x2.oui"));
    let large = read(&conformance.join("cases/synthetic/dashboard-x8.oui"));
    let (small_cost, large_cost) = (
        cost_per_push(&library, &small),
        cost_per_push(&library, &large),
    );
    let ratio = large_cost / small_cost;
    assert!(
        ratio < 2.0,
        "a push on the 4× program costs {ratio:.2}× as much ({:.1} µs against {:.1} µs)",
        large_cost * 1e6,
        small_cost * 1e6
    );
}
