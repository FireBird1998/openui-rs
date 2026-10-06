//! Streaming work grows with the program, not with its square: a push costs
//! about the same early in a stream as late, so a program four times the
//! size takes about four times as long, not sixteen. The two synthetic
//! programs differ only in size and one level of nesting; a parser that
//! redid the whole tree on every push would cost about four times more per
//! push on the larger one. On an idle machine the ratio is about 1.3.
//!
//! Wall-clock time on a busy machine measures the machine, so this measures
//! the thread's CPU time (on Unix; wall time elsewhere). The two programs are
//! also measured in turn, seven times each, and the fastest run of each
//! counts. The bound, 2.5, leaves room for what noise remains while still
//! catching the 4× of a quadratic parser.

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

/// Mean cost of one push in one run of the whole stream.
fn cost_per_push(library: &Arc<Library>, chunks: &[&str]) -> f64 {
    let started = now();
    let mut parser = StreamParser::new(library.clone(), None);
    for chunk in chunks {
        std::hint::black_box(parser.push(chunk));
    }
    (now() - started) / chunks.len() as f64
}

#[test]
fn a_push_costs_about_the_same_on_a_program_four_times_the_size() {
    let conformance = conformance_dir();
    let schema =
        serde_json::from_str(&read(&conformance.join("imported/schemas/bench.json"))).unwrap();
    let library = Arc::new(Library::from_json_schema(&schema));
    let small_text = read(&conformance.join("cases/synthetic/dashboard-x2.oui"));
    let large_text = read(&conformance.join("cases/synthetic/dashboard-x8.oui"));
    let (small, large) = (chunks(&small_text), chunks(&large_text));
    let (mut small_cost, mut large_cost) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..RUNS {
        small_cost = small_cost.min(cost_per_push(&library, &small));
        large_cost = large_cost.min(cost_per_push(&library, &large));
    }
    let ratio = large_cost / small_cost;
    println!(
        "per push: {:.1} µs on the 4× program, {:.1} µs on the small one: {ratio:.2}×",
        large_cost * 1e6,
        small_cost * 1e6
    );
    assert!(
        ratio < BOUND,
        "a push on the 4× program costs {ratio:.2}× as much ({:.1} µs against {:.1} µs)",
        large_cost * 1e6,
        small_cost * 1e6
    );
}
