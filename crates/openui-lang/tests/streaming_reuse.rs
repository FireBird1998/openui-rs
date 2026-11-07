//! Streaming reuses what evaluating state defaults found (#8): a default whose
//! value, and every default it reads, are unchanged since the last push keeps
//! what it was evaluated to, so a push costs about the same late in a stream
//! as early, defaults or not. What it may never change is a result: each push
//! gives what a stream given the same text in one push gives, with nothing to
//! reuse, under the same limits (even where the limits that scale with the
//! program cut an early push and not a later one), and its end what such a
//! stream's end gives.
//!
//! Costs are measured in the thread's CPU time, the fastest of a few runs, as
//! `streaming_cost.rs` does, against generous bounds.

use std::sync::Arc;

use openui_lang::{Library, Limits, ParseResult, StreamParser, parse_with_limits};

fn library() -> Arc<Library> {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }});
    Arc::new(Library::from_json_schema(&schema))
}

fn fuzz_library() -> Arc<Library> {
    let schema = serde_json::from_str(include_str!("slow/fuzz-library.json")).unwrap();
    Arc::new(Library::from_json_schema(&schema))
}

/// `text` in pushes of `n` code points.
fn chunks(text: &str, n: usize) -> Vec<&str> {
    let starts: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .step_by(n)
        .chain([text.len()])
        .collect();
    starts.windows(2).map(|w| &text[w[0]..w[1]]).collect()
}

/// Stream `program` in pushes of `n` code points, asserting each push's
/// result, and the end's, is that of a fresh stream given the text so far in
/// one push.
fn streams_as_without_reuse(program: &str, library: &Arc<Library>, limits: Limits, n: usize) {
    let fresh = |text: &str| {
        let mut stream = StreamParser::with_limits(library.clone(), None, limits);
        stream.push(text);
        stream
    };
    let mut stream = StreamParser::with_limits(library.clone(), None, limits);
    let mut so_far = 0;
    for chunk in chunks(program, n) {
        so_far += chunk.len();
        let pushed = stream.push(chunk);
        assert_same(
            &pushed,
            &fresh(&program[..so_far]).result(),
            &program[..so_far],
        );
    }
    assert_same(&stream.finish(), &fresh(program).finish(), program);
}

fn assert_same(pushed: &ParseResult, expected: &ParseResult, text: &str) {
    assert_eq!(
        pushed.to_json(),
        expected.to_json(),
        "after pushing {} bytes, ending {:?}",
        text.len(),
        text.chars()
            .rev()
            .take(40)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    assert_eq!(
        (pushed.size_limited, pushed.depth_limited),
        (expected.size_limited, expected.depth_limited),
        "flags after pushing {} bytes",
        text.len()
    );
}

/// `[0, 1, …, n - 1]`.
fn list(n: usize) -> String {
    let items: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    format!("[{}]", items.join(","))
}

/// Value statements nothing reaches, about `bytes` long in all, to grow a
/// program.
fn padding(bytes: usize) -> String {
    let line = |i: usize| format!("p{i} = Text(\"{}\")\n", "x".repeat(40));
    let mut out = String::new();
    while out.len() < bytes {
        out.push_str(&line(out.len()));
    }
    out
}

#[test]
fn defaults_reading_defaults_and_components_stream_as_without_reuse() {
    // Objects keep their identity across defaults: the same component read
    // twice is the same object, whichever default read it first.
    let program = "\
root = Stack([card, $x])
card = Text(\"hi\")
$a = [card, card]
$b = $a[0] == $a[1]
$c = card
$d = $c == $a[0]
$e = @Each([1, 2, 3], \"i\", $i + 1)
$x = @Count($e)
late = Text(\"later\")
$g = $b ? \"same\" : \"different\"
$h = [$a, $c, later]
later = Text(\"arrived\")
$e = @Each([4, 5], \"i\", $i * 2)
$k = $h[2] == later
";
    for n in [1, 3, 8] {
        streams_as_without_reuse(program, &library(), Limits::default(), n);
    }
}

#[test]
fn properties_of_runtime_expressions_keep_their_identity_across_defaults() {
    // A property of a runtime expression read twice is one object, whichever
    // default read it first; the defaults that read it first are taken as
    // they were at later pushes, the ones comparing it evaluated again.
    let across = "\
root = Text(\"x\")
$a = @Each([1], \"i\", $t = [i, {a: 1}])
$els = $a[0].expr.els
v1 = Text(\"pad\")
$same = $els == $a[0].expr.els
v2 = Text(\"more\")
$entries = $a[0].expr.els[1].entries == $els[1].entries
";
    let cases = [
        include_str!(
            "../../../conformance/cases/hand/146-state-defaults-reading-an-assignment.oui"
        ),
        include_str!("../../../conformance/cases/hand/147-runtime-property-identity.oui"),
        across,
    ];
    for program in cases {
        for n in [1, 4] {
            streams_as_without_reuse(program, &library(), Limits::default(), n);
        }
    }
}

#[test]
fn a_redefined_default_and_a_late_reference_stream_as_without_reuse() {
    let program = "\
root = Stack([])
$a = later + 1
$b = $a * 2
$s = @Sort([3, 1, 2], \"\")
later = 5
$a = 10
$c = $b + $a
$a = $c
";
    for n in [1, 4] {
        streams_as_without_reuse(program, &library(), Limits::default(), n);
    }
}

/// A default of about 1.6 million steps: more than the work limit's floor of
/// a million gives a short program, less than 1,000 steps a byte gives one of
/// 2 KB.
fn costly_default() -> String {
    format!(
        "$l = {}\n$a = @Count(@Each($l, \"x\", @Each($l, \"y\", @Each($l, \"z\", 1))))\n",
        list(80)
    )
}

#[test]
fn a_default_cut_by_work_early_is_whole_once_the_program_grows() {
    let program = format!(
        "root = Stack([])\n{}$b = $a + 1\n{}",
        costly_default(),
        padding(2_000)
    );
    let short = format!("root = Stack([])\n{}", costly_default());
    let limits = Limits::default();
    // Cut while the program is short, whole once it's long.
    assert!(parse_with_limits(&short, &library(), None, limits).size_limited);
    assert!(!parse_with_limits(&program, &library(), None, limits).size_limited);
    streams_as_without_reuse(&program, &library(), limits, 13);
}

#[test]
fn a_default_cut_by_room_early_is_whole_once_the_program_grows() {
    // 80 strings of 1,300 bytes: about 104 KB, which needs a program of
    // about 1.6 KB under the default 64 times.
    let program = format!(
        "root = Stack([])\n$r = @Each({}, \"x\", \"{}\")\n$n = @Count($r)\n{}",
        list(80),
        "y".repeat(1_300),
        padding(1_000)
    );
    let limits = Limits::default();
    let short = &program[..program.find("$n =").unwrap()];
    assert!(parse_with_limits(short, &library(), None, limits).size_limited);
    assert!(!parse_with_limits(&program, &library(), None, limits).size_limited);
    streams_as_without_reuse(&program, &library(), limits, 17);
}

#[test]
fn the_slow_fuzz_input_streams_as_without_reuse() {
    let program = include_str!("slow/parse-94563fe.oui");
    streams_as_without_reuse(program, &fuzz_library(), Limits::default(), 3);
}

// ─── Costs ──────────────────────────────────────────────────────────────────

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

/// The fastest of `runs` timings of `f`.
fn fastest(runs: usize, mut f: impl FnMut() -> f64) -> f64 {
    (0..runs).map(|_| f()).fold(f64::INFINITY, f64::min)
}

#[test]
fn an_unchanged_default_isnt_evaluated_again_on_later_pushes() {
    // A default of over a million steps, then many small statements arriving
    // a few characters at a time: each of those pushes leaves it unchanged.
    let head = format!("root = Stack([])\n{}", costly_default());
    let tail = padding(1_500);
    let limits = Limits::default().with_work(64_000_000);
    let library = library();

    let evaluating = fastest(3, || {
        let started = now();
        std::hint::black_box(parse_with_limits(&head, &library, None, limits));
        now() - started
    });
    let per_push = fastest(3, || {
        let mut stream = StreamParser::with_limits(library.clone(), None, limits);
        stream.push(&head);
        let pushes = chunks(&tail, 5);
        let started = now();
        for chunk in &pushes {
            std::hint::black_box(stream.push(chunk));
        }
        (now() - started) / pushes.len() as f64
    });
    // Evaluated again on every push, a push would cost about as much as
    // evaluating it; reused, a small fraction of that.
    assert!(
        per_push < evaluating / 10.0,
        "a push costs {:.3} ms, evaluating the default {:.3} ms",
        per_push * 1e3,
        evaluating * 1e3
    );
}

/// Sixty state defaults of some thousands of steps each, with value
/// statements between them, streamed a character at a time: a push late in
/// the stream costs about what one early in it does, the defaults that have
/// arrived taken as they were, not evaluated again. The last quarter's pushes
/// cost about 1.5 times the first quarter's on an idle machine, under 3 on a
/// busy one; with every default evaluated again on every push, 8 times.
#[test]
fn streaming_completed_defaults_costs_time_linear_in_its_length() {
    let block = |i: usize| {
        format!(
            "$a{i} = @Count(@Each($l, \"x\", @Each($l, \"y\", $x + $y)))\nv{i} = Text(\"value {i}\")\n"
        )
    };
    let program = format!(
        "root = Stack([])\n$l = {}\n{}",
        list(30),
        (0..60).map(block).collect::<String>()
    );
    let library = library();
    let pushes = chunks(&program, 1);
    // The pushes of the last quarter against those of the first, in one run.
    let quarter = pushes.len() / 4;
    let ratio = fastest(5, || {
        let mut stream = StreamParser::new(library.clone(), None);
        let (mut early, mut late) = (0.0, 0.0);
        for (i, chunk) in pushes.iter().enumerate() {
            let started = now();
            std::hint::black_box(stream.push(chunk));
            let spent = now() - started;
            if i < quarter {
                early += spent;
            } else if i >= pushes.len() - quarter {
                late += spent;
            }
        }
        late / early
    });
    assert!(
        ratio < 4.0,
        "the last quarter's pushes cost {ratio:.1} times the first's"
    );
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
