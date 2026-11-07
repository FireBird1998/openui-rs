//! Streaming reuses what evaluating state defaults found (#8): a default whose
//! value, and every default it reads, are unchanged since the last push keeps
//! what it was evaluated to, so a push costs about the same late in a stream
//! as early, defaults or not. What it may never change is a result: each push
//! gives what a stream given the same text in one push gives, with nothing to
//! reuse, under the same limits (even where the limits that scale with the
//! program cut an early push and not a later one), and its end what such a
//! stream's end gives.
//!
//! A push mid-stream is compared with a fresh stream, not a one-shot parse,
//! because the two legitimately differ there (a redefinition still arriving
//! keeps the earlier one, quirk 4; a `$name` still being typed isn't declared
//! yet). The fresh stream evaluates everything, so it can't share a mistake
//! the reuse makes; but it can share one in the evaluator itself, so the end
//! is also compared with a one-shot parse, which streams' ends match apart
//! from quirk 4's count of a redefinition.
//!
//! Cost checks use thread CPU time, fastest of several runs, against generous
//! ratios. Work pricing is checked through public work limits.

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
/// one push; and the end's that of a one-shot parse, but for its statement
/// count (quirk 4).
fn streams_as_without_reuse(program: &str, library: &Arc<Library>, limits: Limits, n: usize) {
    streams_as_without_reuse_root(program, library, limits, n, None);
}

fn streams_as_without_reuse_root(
    program: &str,
    library: &Arc<Library>,
    limits: Limits,
    n: usize,
    root: Option<&str>,
) {
    let fresh = |text: &str| {
        let mut stream = StreamParser::with_limits(library.clone(), root, limits);
        stream.push(text);
        stream
    };
    let mut stream = StreamParser::with_limits(library.clone(), root, limits);
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
    let end = stream.finish();
    assert_same(&end, &fresh(program).finish(), program);
    let one_shot = parse_with_limits(program, library, root, limits);
    let uncounted = |result: &ParseResult| {
        let mut json: serde_json::Value = serde_json::from_str(&result.to_json()).unwrap();
        json["meta"]["statementCount"] = serde_json::Value::Null;
        json
    };
    assert_eq!(
        uncounted(&end),
        uncounted(&one_shot),
        "the end, against a one-shot parse"
    );
    assert_eq!(
        (end.size_limited, end.depth_limited),
        (one_shot.size_limited, one_shot.depth_limited),
        "the end's flags, against a one-shot parse"
    );
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

/// Under `limits`, `$n0` to `$nK`, each a list of the one before, where
/// `$nK` is the deepest that isn't too deep (the next is null), with `K`.
fn deepest_chain(limits: Limits) -> (usize, String) {
    let chain = |k: usize| {
        (0..=k)
            .map(|i| match i {
                0 => "$n0 = [0]\n".to_owned(),
                _ => format!("$n{i} = [$n{}]\n", i - 1),
            })
            .collect::<String>()
    };
    let program = format!("root = Stack([])\n{}", chain(30));
    let result = parse_with_limits(&program, &library(), None, limits);
    let too_deep = (1..=30)
        .find(|k| {
            matches!(
                result.state_declarations.get(&format!("$n{k}")),
                Some(openui_lang::Value::Null)
            )
        })
        .expect("a default past the depth limit");
    (too_deep - 1, chain(too_deep - 1))
}

/// `$t` is too deep (null), but it read `first` before it went too deep;
/// `$b` and `$c` read it after, so as the same object. `$c`, while it's
/// arriving, is evaluated at every push and `$t` and `$b` taken as they were:
/// what `$t` read first must be read again for `$c`, though `$t` came out
/// null.
fn too_deep_first(first: &str, then: &str) -> (String, Limits) {
    let limits = Limits::default().with_depth(6);
    let (k, chain) = deepest_chain(limits);
    let program = format!(
        "\
root = Stack([card])
card = Text(\"hi\")
$p = {{a: 1}}
$e = [card]
{chain}$t = [{first}, [$n{k}]]
$b = [{first}, 1 + 1]
v0 = Text(\"pad\")
$c = $b[0] == {then}
"
    );
    let whole = parse_with_limits(&program, &library(), None, limits);
    assert!(whole.depth_limited);
    let json = whole.to_json();
    assert!(json.contains("\"$t\":null"), "{json}");
    assert!(json.contains("\"$c\":true"), "{json}");
    (program, limits)
}

#[test]
fn a_too_deep_default_reads_an_element_first_for_the_defaults_after_it() {
    let (program, limits) = too_deep_first("card", "$e[0]");
    for n in [1, 3] {
        streams_as_without_reuse(&program, &library(), limits, n);
    }
}

#[test]
fn a_too_deep_default_reads_a_plain_default_first_for_the_defaults_after_it() {
    let (program, limits) = too_deep_first("$p", "$p");
    for n in [1, 3] {
        streams_as_without_reuse(&program, &library(), limits, n);
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
fn after_room_runs_out_every_expression_default_is_null() {
    // `$big` runs out of room. Every expression default after it is null,
    // even `$u`, whose value would be `undefined` and need none; a one-shot
    // parse and a stream agree.
    let program = format!(
        "root = Stack([])\n$big = @Each({}, \"x\", \"{}\")\n$obj = {{a: 1}}\n$u = $obj.missing\n$n = 1 + 1\n",
        list(80),
        "y".repeat(1_300)
    );
    let limits = Limits::default();
    let one_shot = parse_with_limits(&program, &library(), None, limits);
    assert!(one_shot.size_limited);
    let json = one_shot.to_json();
    for null in ["$big", "$u", "$n"] {
        assert!(json.contains(&format!("\"{null}\":null")), "{null}: {json}");
    }
    assert!(json.contains("\"$obj\":{\"a\":1}"), "{json}");
    streams_as_without_reuse(&program, &library(), limits, 9);
}

#[test]
fn the_slow_fuzz_input_streams_as_without_reuse() {
    let program = include_str!("slow/parse-94563fe.oui");
    streams_as_without_reuse(program, &fuzz_library(), Limits::default(), 3);
}

#[test]
fn completed_depth_cuts_survive_later_shallow_statements() {
    let program = "root = Stack([])\ndeep = [[[[[[1]]]]]]\nplain = 1\n";
    let limits = Limits::default().with_depth(3);
    assert!(parse_with_limits(program, &library(), None, limits).depth_limited);
    for n in [1, 7, program.len()] {
        streams_as_without_reuse(program, &library(), limits, n);
    }
}

#[test]
fn changing_reference_graphs_stream_as_fresh_tables() {
    // Definitions, temporary overlays and their undo change both the reverse
    // reference graph and the reached tree. Keep an orphan throughout so the
    // memoized orphan list must change even when its length stays nonzero.
    let programs = [
        "spare = Text(\"spare\")\na = Text(\"a\")\nb = Text(\"b\")\nc = Text(\"c\")\nx = Stack([a, b])\ny = Stack([a, c])\nroot = Stack([x, y])\nx = Stack([])\nroot = Stack([y])\ny = Stack([])\nroot = Stack([b])\n",
        "spare = 0\na = Text(\"a\")\nb = Text(\"b\")\nx = Stack([a])\nroot = Stack([x])\nx = Stack([b])\na = Text(\"changed\")\nb = Text(\"changed too\")\nx = Stack([a, b])\na = Text(\"again\")\n",
        "spare = 0\na = Text(\"a\")\nroot = Stack([])\nlate = 1\n$late = 2\nlate = $late\nlate = Text(\"value\")\nroot = Stack([late, a])\n",
    ];
    for program in programs {
        for n in [1, 2, 7, 31] {
            streams_as_without_reuse(program, &library(), Limits::default(), n);
        }
    }
}

#[test]
fn cache_variants_at_depth_boundaries_keep_their_own_reach() {
    let program = "spare = Text(\"spare\")\na = Text(\"a\")\nb = Stack([a])\nc = Stack([b])\nroot = Stack([a, b, c])\npending = Stack([\n";
    for depth in 2..8 {
        for n in [1, 7, 23] {
            streams_as_without_reuse(program, &library(), Limits::default().with_depth(depth), n);
        }
    }
}

/// The smallest whole-run work limit, observed through the public parser.
fn default_steps(program: &str) -> usize {
    let library = library();
    let whole = |work| {
        !parse_with_limits(program, &library, None, Limits::default().with_work(work)).size_limited
    };
    let (mut lo, mut hi) = (0, 64_000_000);
    assert!(whole(hi));
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
fn reused_defaults_charge_work_and_room_at_every_boundary() {
    let programs = [
        "root = Stack([])\n$p = {a: [1, 2]}\n$a = @Each([1, 2, 3], \"x\", $p)\n$b = $a[0] == $a[1]\n$c = @Count($a) + 1\n",
        "root = Stack([])\n$e = Text(\"hi\")\n$a = [$e, $e]\n$b = $a[0] == $e\n$c = @Each([1, 2], \"i\", $a)\n",
        "root = Stack([])\n$a = @Each([1, 2], \"i\", $t = [i, {a: 1}])\n$b = $a[0].expr.els\n$c = $b == $a[0].expr.els\n",
    ];
    for program in programs {
        let steps = default_steps(program);
        for work in 0..=steps + 2 {
            streams_as_without_reuse(program, &library(), Limits::default().with_work(work), 7);
        }
        for room in (0..600).step_by(7) {
            streams_as_without_reuse(
                program,
                &library(),
                Limits::default().with_result_size(room),
                11,
            );
        }
    }
}

#[test]
fn changing_default_shapes_and_aliases_never_reuses_old_values() {
    let program = "root = Stack([])\n$seed = {a: true, b: null}\n$read = [$seed, $seed]\n$same = $read[0] == $read[1]\n$seed = {b: null, a: false}\n$seed = [true, null]\n$seed = \"changed\"\n$seed = false\n$seed = null\n$seed = Text(\"first\")\n$seed = Stack([])\n$seed = {a: Text(\"nested\")}\n$seed = [$read = {a: 1}]\n$seed = {a: $read = [1]}\n";
    for n in [1, 3, 11] {
        streams_as_without_reuse(program, &library(), Limits::default(), n);
    }
}

#[test]
fn shared_elements_and_expression_properties_survive_reparsed_defaults() {
    // Aliases occur both within one default and across defaults. A default
    // that returns an assignment retains runtime expressions nested in arrays
    // and objects, then later defaults read their shared properties.
    let program = "root = Stack([])\ncard = Text(\"hi\")\n$a = @Each([1, 2], \"i\", [$t = {a: card, b: [card]}])\n$b = $a[0][0]\n$c = $b.expr.entries\n$d = $a[0][0].expr.entries == $c\n$e = [$b, {nested: $b}]\n$f = $e[0].expr.entries == $e[1].nested.expr.entries\ncard = Text(\"changed\")\n$g = $a[0][0].expr.entries == $c\n";
    for n in [1, 4, 13] {
        streams_as_without_reuse(program, &library(), Limits::default(), n);
    }
}

#[test]
fn doubled_room_runs_replay_first_reads_at_exact_work_boundaries() {
    let heads = [
        "root = Stack([])\ncard = Text(\"shared element\")\n$p = {a: 1}\n$a = @Each([1, 2, 3, 4], \"i\", [card, $p, \"a long payload that needs more room than its source\"])\n",
        "root = Stack([])\n$a = @Each([1, 2, 3, 4], \"i\", $t = [i, {a: 1}])\n$b = @Each([1, 2, 3, 4], \"i\", [$a[0].expr.els, \"a long payload that needs more room than its source\"])\n",
    ];
    for head in heads {
        let steps = default_steps(head);
        let program = format!("{head}$same = $a[0] == $a[0]\n{}", " ".repeat(1_200));
        for work in [steps - 1, steps, steps + 1, steps + 2, steps * 2] {
            let limits = Limits::default()
                .with_work(work)
                .with_result_floor(0)
                .with_amplification(1);
            for n in [7, 31] {
                streams_as_without_reuse(&program, &library(), limits, n);
            }
        }
    }
}

#[test]
fn reparsed_expression_owners_keep_shared_reads_across_many_pushes() {
    for (template, access) in [
        ("$t = [i, {a: 1}]", "[0]"),
        ("[$t = [i, {a: 1}]]", "[0][0]"),
        ("{nested: $t = [i, {a: 1}]}", "[0].nested"),
    ] {
        let program = format!(
            "root = Stack([])\n$seed = source\n$first = $seed{access}.expr.els\n$again = $seed\n$same = $first == $again{access}.expr.els\nsource = @Each([1], \"i\", {template}){}",
            " ".repeat(60)
        );
        for n in [1, 7, 19] {
            streams_as_without_reuse(&program, &library(), Limits::default(), n);
        }
        let json = parse_with_limits(&program, &library(), None, Limits::default()).to_json();
        assert!(json.contains("\"$same\":true"), "{json}");
    }
}

#[test]
fn equal_looking_elements_with_different_owners_do_not_reuse() {
    let program = "root = Stack([])\none = Text(\"same\")\ntwo = Text(\"same\")\n$seed = source\n$read = [$seed, $seed]\nsource = one\nsource = two\nsource = Text($live)\nsource = Text(\"same\")\n";
    for n in [1, 3, 9] {
        streams_as_without_reuse(program, &library(), Limits::default(), n);
    }
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

#[test]
fn an_empty_push_at_a_completed_boundary_reuses_the_table() {
    let program = format!("root = Stack([])\n{}", padding(40_000));
    let library = library();
    let parse_cost = fastest(5, || {
        let start = now();
        std::hint::black_box(parse_with_limits(
            &program,
            &library,
            None,
            Limits::default(),
        ));
        now() - start
    });
    let push_cost = fastest(5, || {
        let mut stream = StreamParser::new(library.clone(), None);
        stream.push(&program);
        let start = now();
        for _ in 0..20 {
            std::hint::black_box(stream.push(""));
        }
        (now() - start) / 20.0
    });
    assert!(
        push_cost < parse_cost / 5.0,
        "empty push {push_cost:.6}s, parsing {parse_cost:.6}s"
    );
}

#[test]
fn changing_an_obsolete_reference_keeps_the_new_tree_cached() {
    let children = (0..600)
        .map(|i| format!("Text(\"value {i}\")"))
        .collect::<Vec<_>>()
        .join(",");
    let head = format!("a = 0\nx = a\nroot = Stack([x])\nx = Stack([{children}])\n");
    let library = library();
    let cost = |name: &str| {
        fastest(5, || {
            let mut stream = StreamParser::new(library.clone(), None);
            stream.push(&head);
            let start = now();
            for i in 0..30 {
                std::hint::black_box(stream.push(&format!("{name} = {i}\n")));
            }
            (now() - start) / 30.0
        })
    };
    let unrelated = cost("unused");
    let obsolete = cost("a");
    assert!(
        obsolete < unrelated * 3.0,
        "obsolete reference {obsolete:.6}s, unrelated {unrelated:.6}s"
    );
}

#[test]
fn changing_partial_flags_shares_unchanged_reach_trees() {
    // One cached value with many children, used by a state default (AST
    // mode) as well as the root. The trailing statement alternates the
    // partial flag without changing any reference in that cached value.
    let children = (0..4_000).map(|_| "leaf").collect::<Vec<_>>().join(",");
    let head = format!(
        "leaf = Text(\"value\")\nwide = Stack([{children}])\nroot = Stack([wide])\n$copy = wide\n"
    );
    let library = library();
    let cost = |toggle: bool| {
        fastest(5, || {
            let mut stream = StreamParser::new(library.clone(), None);
            stream.push(&head);
            // Warm both partial variants before timing.
            stream.push("pending = Text(");
            stream.push("\"done\")\n");
            let start = now();
            for _ in 0..40 {
                if toggle {
                    std::hint::black_box(stream.push("pending = Text("));
                    std::hint::black_box(stream.push("\"done\")\n"));
                } else {
                    std::hint::black_box(stream.push(""));
                    std::hint::black_box(stream.push(""));
                }
            }
            (now() - start) / 80.0
        })
    };
    let unchanged = cost(false);
    let toggled = cost(true);
    assert!(
        toggled < unchanged * 4.0,
        "partial flag changes {toggled:.6}s, unchanged {unchanged:.6}s"
    );
}

#[test]
fn structurally_unchanged_reparsed_defaults_keep_expensive_followers() {
    // The final source definition is complete but has no newline. Every push
    // reparses it and invalidates its readers, giving same() fresh values to
    // compare, including objects and runtime expressions with new addresses.
    // Two readers of source also exercise pairing the same identity twice.
    let seeds = [
        "null",
        "true",
        "\"same\"",
        "[1, {a: null}]",
        "{a: true}",
        "Text(\"hi\")",
        "[$t = 1]",
        "{a: $t = 1}",
        "$t = {a: [1]}",
        "$t = true",
        "$t = @Sum([1, 2])",
    ];
    let library = library();
    let limits = Limits::default().with_work(64_000_000);
    for seed in seeds {
        let program = format!(
            "root = Stack([])\n$seed = source\n$again = source\n{}source = {seed}",
            costly_default()
        );
        let whole = parse_with_limits(&program, &library, None, limits);
        assert!(!whole.size_limited, "{seed}");
        streams_as_without_reuse(&format!("{program}    "), &library, limits, 29);
        let evaluating = fastest(3, || {
            let start = now();
            std::hint::black_box(parse_with_limits(&program, &library, None, limits));
            now() - start
        });
        let pushing = fastest(3, || {
            let mut stream = StreamParser::with_limits(library.clone(), None, limits);
            stream.push(&program);
            let start = now();
            for _ in 0..20 {
                std::hint::black_box(stream.push(" "));
            }
            (now() - start) / 20.0
        });
        assert!(
            pushing < evaluating / 8.0,
            "seed {seed}: push {pushing:.6}s, evaluating {evaluating:.6}s"
        );
    }
}

#[test]
fn scalar_runs_move_to_the_now_cached_expression() {
    // Completing source changes the materialized AST's address once. A run
    // returning a scalar and retaining no shared reads can then adopt that
    // new owner. Rechecking a long, old AST on every empty push costs linear
    // time; adopting the cached owner makes the equality check constant time.
    let head = format!(
        "root = Stack([])\n$a = @Count(source)\nsource = {}",
        list(8_000)
    );
    let library = library();
    let cost = |complete_later: bool| {
        fastest(5, || {
            let mut stream = StreamParser::new(library.clone(), None);
            if complete_later {
                stream.push(&head);
                stream.push("\n");
            } else {
                stream.push(&format!("{head}\n"));
            }
            stream.push("");
            let start = now();
            for _ in 0..40 {
                std::hint::black_box(stream.push(""));
            }
            (now() - start) / 40.0
        })
    };
    let already_cached = cost(false);
    let adopted = cost(true);
    assert!(
        adopted < already_cached * 4.0,
        "adopted owner {adopted:.6}s, originally cached {already_cached:.6}s"
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

/// A default that never finishes within the work limit (eight million
/// iterations), then 3 KB of statements arriving ten characters at a time:
/// every push gives it more work than the last (1,000 steps a byte), and it
/// runs out every time. Given twice what it had whenever it's given more, it
/// costs about as much in all as evaluating it once at the end; evaluated
/// again whenever it's given more, a few hundred times that.
#[test]
fn a_default_that_keeps_running_out_costs_about_one_evaluation_in_all() {
    let program = format!(
        "root = Stack([])\n$l = {}\n$a = @Count(@Each($l, \"x\", @Each($l, \"y\", @Each($l, \"z\", 1))))\n{}",
        list(200),
        padding(3_000)
    );
    let library = library();
    let limits = Limits::default();
    assert!(parse_with_limits(&program, &library, None, limits).size_limited);
    let once = fastest(3, || {
        let started = now();
        std::hint::black_box(parse_with_limits(&program, &library, None, limits));
        now() - started
    });
    let pushes = chunks(&program, 10);
    let streamed = fastest(3, || {
        let mut stream = StreamParser::with_limits(library.clone(), None, limits);
        let started = now();
        for chunk in &pushes {
            std::hint::black_box(stream.push(chunk));
        }
        now() - started
    });
    assert!(
        streamed < once * 10.0,
        "streamed in {} pushes: {:.0} ms; parsed once: {:.1} ms",
        pushes.len(),
        streamed * 1e3,
        once * 1e3
    );
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn recovered_defaults_invalidate_followers() {
    let head = format!(
        "root = Stack([])\n$a = @Each([1], \"i\", \"old\")\n$b = @Count($a)\n$c = $b + 1\n$a = @Each({}, \"i\", \"{}\")\n$b = @Count($a)\n$c = $b + 1\n",
        list(30),
        "payload".repeat(20)
    );
    let program = format!("{head}{}", padding(8_000));
    let limits = Limits::default().with_result_floor(0).with_amplification(8);
    assert!(parse_with_limits(&head, &library(), None, limits).size_limited);
    assert!(!parse_with_limits(&program, &library(), None, limits).size_limited);
    streams_as_without_reuse(&program, &library(), limits, 37);
}

#[test]
fn a_changed_default_that_becomes_too_deep_invalidates_its_followers() {
    let limits = Limits::default().with_depth(6);
    let (k, chain) = deepest_chain(limits);
    let program =
        format!("root = Stack([])\n{chain}$t = 5 + 0\n$b = $t + 1\n$t = [[$n{k}], 1 + 1]\n");
    for n in [1, 7] {
        streams_as_without_reuse(&program, &library(), limits, n);
    }
}

#[test]
fn a_speculative_depth_cut_still_needs_this_pushs_work() {
    let limits = Limits::default().with_depth(6);
    let (k, chain) = deepest_chain(limits);
    let program = format!(
        "root = Stack([])\n{chain}$l = {}\n$a = [@Count(@Each($l, \"x\", @Each($l, \"y\", @Each($l, \"z\", 1)))), [$n{k}]]\n{}",
        list(80),
        padding(2_000)
    );
    assert!(parse_with_limits(&program, &library(), None, limits).depth_limited);
    streams_as_without_reuse(&program, &library(), limits, 31);
}

#[test]
fn unsubstituted_assignment_templates_keep_their_original_owners() {
    for template in [
        "$t = [1, {a: 1}]",
        "[$t = [1, {a: 1}]]",
        "{nested: $t = [1, {a: 1}]}",
    ] {
        let access = if template.starts_with('[') {
            "[0][0]"
        } else if template.starts_with('{') {
            "[0].nested"
        } else {
            "[0]"
        };
        let program = format!(
            "root = Stack([])\n$seed = source\n$first = $seed{access}.expr.els\n$again = source\n$same = $first == $again{access}.expr.els\nsource = @Each([1], \"i\", {template}){}\n$new = source\n$late = $first == $new{access}.expr.els\n",
            " ".repeat(40)
        );
        for n in [1, 3, 7] {
            streams_as_without_reuse(&program, &library(), Limits::default(), n);
        }
    }
}

#[test]
fn empty_expression_properties_keep_identity_after_owner_changes() {
    let head = "root = Stack([])\n$a = source\nsource = @Each([1], \"i\", $t = [])[0].expr.els";
    let program = format!("{head}{}\n$b = source\n$same = $a == $b\n", " ".repeat(40));
    for n in [1, 3, 7] {
        streams_as_without_reuse(&program, &library(), Limits::default(), n);
    }
}

#[test]
fn state_defaults_share_materialized_owners_between_streaming_modes() {
    // No references or library checks: both modes can use the SAME AST owner.
    // Merely warming both caches is insufficient if each holds a different
    // owner: every mode switch then walks this list again to compare them.
    let head = format!("root = Stack([])\n$a = {} == null\npad = 0\n", list(8_000));
    let library = library();
    let cost = |toggle: bool| {
        fastest(5, || {
            let mut stream = StreamParser::new(library.clone(), None);
            stream.push(&head);
            stream.push("pending = Text(");
            stream.push("\"done\")\n");
            let start = now();
            for _ in 0..40 {
                if toggle {
                    std::hint::black_box(stream.push("pending = Text("));
                    std::hint::black_box(stream.push("\"done\")\n"));
                } else {
                    std::hint::black_box(stream.push(""));
                    std::hint::black_box(stream.push(""));
                }
            }
            (now() - start) / 80.0
        })
    };
    let unchanged = cost(false);
    let toggled = cost(true);
    assert!(
        toggled < unchanged * 10.0,
        "mode switches {toggled:.6}s, unchanged {unchanged:.6}s"
    );
    streams_as_without_reuse(
        &format!("{head}pending = Text(\"done\")\n"),
        &library,
        Limits::default(),
        71,
    );
}

#[test]
fn revised_defaults_and_reference_graphs_match_fresh_streams_at_limit_edges() {
    let programs = [
        "root = Stack([$a, $b, $c])\n$a = [1, 2] + \"old\"\n$b = [$a, $a]\n$c = $b + \"!\"\n$a = [3, 4] + \"new\"\n",
        "root = Stack([$a, $b, $c])\n$a = 1 + 2\n$b = $a + 10\n$c = $b + 100\n$a = @Each([1,2,3,4,5], \"x\", [1,2,3,4,5])\n",
        "root = Stack([a, b])\na = Stack([b])\nb = Stack([a, missing])\nb = Text(\"done\")\na = Stack([])\na = Stack([b])\n",
        "root = Stack([])\na = Text(\"a\")\nb = Text(\"b\")\nroot = Stack([a,b])\na = Query(\"x\")\na = Text(\"a\")\nroot = Stack([])\nroot = Stack([a,b])\n",
        "root = Stack([$a])\n$a = source\n$b = $a.expr\nsource = $t = true\nsource = $t = false\nsource = $t = @Sum([1,2])\nsource = $t = @Sum([3,4])\n",
    ];
    for program in programs {
        for work in [0, 1, 5, 10, 20, 40, 60, 80, 100, 150, 200, 400, 1000] {
            for n in [1, 11] {
                streams_as_without_reuse(program, &library(), Limits::default().with_work(work), n);
            }
        }
        for room in (80..1000).step_by(17) {
            streams_as_without_reuse(
                program,
                &library(),
                Limits::default().with_result_size(room),
                7,
            );
        }
    }
}

#[test]
fn partial_validation_changes_mapped_props_even_when_written_args_are_equal() {
    // A library may override a builtin's signature. Enum validation waits
    // until the pending statement closes, changing only mappedProps.
    for default in [serde_json::json!("good"), serde_json::Value::Null] {
        let mut param = serde_json::json!({"enum": ["good"]});
        if !default.is_null() {
            param["default"] = default;
        }
        let schema = serde_json::json!({
            "$defs": {"Text": {"properties": {"value": {}}}},
            "functions": {"Sum": {"params": {"properties": {"items": param}}}}
        });
        let lib = Arc::new(Library::from_json_schema(&schema));
        let program = "root = Text(\"x\")\n$a = $t = @Sum(\"bad\")\n$b = $a.expr.mappedProps.items.v\npending = Text(\"done\")\npending = Text(\"again\")\n";
        for n in [1, 7, 23] {
            streams_as_without_reuse(program, &lib, Limits::default(), n);
        }
    }
}

#[test]
fn speculative_recovery_after_redefinition_replaces_old_followers() {
    let lib = library();
    let old = "root = Stack([])\n$a = 1 + 0\n$b = [$a, $a]\n$c = $b + \"!\"\n";
    for count in [40, 80, 160] {
        let changed = format!("$a = @Each({}, \"i\", {})\n", list(count), list(count));
        for amplification in [1, 2, 4, 8, 16] {
            let limits = Limits::default()
                .with_result_floor(0)
                .with_amplification(amplification);
            let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
            let mut text = String::new();
            for push in [old, &changed].into_iter().chain(std::iter::repeat_n("                                                                                                 ", 200)) {
                text.push_str(push);
                let result = stream.push(push);
                let mut fresh = StreamParser::with_limits(lib.clone(), None, limits);
                assert_same(&result, &fresh.push(&text), &text);
            }
            let end = stream.finish();
            let whole = parse_with_limits(&text, &lib, None, limits);
            assert_eq!(end.state_declarations, whole.state_declarations);
            assert_eq!(
                (end.size_limited, end.depth_limited),
                (whole.size_limited, whole.depth_limited)
            );
        }
    }
}

#[test]
fn changing_the_fallback_entry_invalidates_the_orphan_list() {
    let program = "a = 0\nb = Text(\"b\")\nc = 0\na = Text(\"a\")\na = 0\nb = 0\nc = Text(\"c\")\n";
    for n in [1, 7, 17] {
        streams_as_without_reuse_root(program, &library(), Limits::default(), n, Some("Text"));
    }
}

#[test]
fn expression_and_value_variants_reach_different_excess_arguments() {
    for extra in ["b", "[b, c]", "Stack([b, c])"] {
        for roots in ["[card, $x ? card : null]", "[$x ? card : null, card]"] {
            let program = format!(
                "root = Stack({roots})\nb = Text(\"b\")\nc = Text(\"c\")\ncard = Text(\"hi\", {extra})\npending = Text(\"done\")\ncard = Text(\"hi\")\ncard = Text(\"hi\", {extra})\nb = Text(\"new\")\n"
            );
            for n in [1, 7, 19] {
                streams_as_without_reuse(&program, &library(), Limits::default(), n);
            }
        }
    }
}

#[test]
fn closing_one_fence_and_opening_another_keeps_completed_statements() {
    // The unfinished fence was the pending line. Replacing it with the next
    // block's code preserves exactly the completed prefix, including its last
    // newline. A large orphan string makes reparsing that prefix expensive,
    // while its parse result stays small.
    let head = format!(
        "```openui\nroot = Stack([])\npad = \"{}\"\n``",
        "x".repeat(150_000)
    );
    let lib = library();
    let cost = |advance: bool| {
        fastest(5, || {
            let mut stream = StreamParser::new(lib.clone(), None);
            stream.push(&head);
            let start = now();
            for _ in 0..60 {
                std::hint::black_box(stream.push(if advance {
                    "`\n```openui\nnext = 0\n``"
                } else {
                    ""
                }));
            }
            now() - start
        })
    };
    let unchanged = cost(false);
    let advanced = cost(true);
    assert!(
        advanced < unchanged * 10.0,
        "new blocks {advanced:.6}s, unchanged {unchanged:.6}s"
    );
    let program = "```openui\nroot = Stack([])\npad = 0\n```\n```openui\nnext = 0\n```\n";
    streams_as_without_reuse(program, &lib, Limits::default(), 1);
}

#[test]
fn speculative_work_recovery_after_redefinition_replaces_old_followers() {
    let lib = library();
    let limits = Limits::default().with_result_size(8 * 1024 * 1024);
    let items = list(80);
    let changed = format!(
        "$a = @Count(@Each({items}, \"x\", @Count(@Each({items}, \"y\", @Count(@Each({items}, \"z\", 1))))))\n"
    );
    let mut stream = StreamParser::with_limits(lib.clone(), None, limits);
    let mut text = String::new();
    for push in ["root = Stack([])\n$a = 1 + 0\n$b = $a + 10\n", &changed].into_iter().chain(std::iter::repeat_n("                                                                                                    ", 60)) {
        text.push_str(push);
        let result = stream.push(push);
        let mut fresh = StreamParser::with_limits(lib.clone(), None, limits);
        assert_same(&result, &fresh.push(&text), &text);
    }
    assert!(!stream.finish().size_limited);
}

#[test]
fn mode_changes_share_a_deep_reach_tree_used_many_times() {
    // Errors are memoized by statement, so a repeated alias chain is walked
    // once when reporting. Reach sharing must also avoid walking that same
    // chain once PER occurrence every time the partial flag changes.
    let mut head = "leaf = Text(\"value\")\na0 = leaf\n".to_owned();
    for i in 1..100 {
        head.push_str(&format!("a{i} = a{}\n", i - 1));
    }
    head.push_str(&format!(
        "wide = Stack([{}])\nroot = Stack([wide])\n",
        vec!["a99"; 800].join(",")
    ));
    let lib = library();
    let cost = |toggle: bool| {
        fastest(5, || {
            let mut stream = StreamParser::with_limits(
                lib.clone(),
                None,
                Limits::default()
                    .with_depth(1024)
                    .with_result_size(8 * 1024 * 1024),
            );
            let result = stream.push(&head);
            assert!(!result.depth_limited && !result.size_limited);
            stream.push("pending = Text(");
            stream.push("\"done\")\n");
            let start = now();
            for _ in 0..40 {
                if toggle {
                    std::hint::black_box(stream.push("pending = Text("));
                    std::hint::black_box(stream.push("\"done\")\n"));
                } else {
                    std::hint::black_box(stream.push(""));
                    std::hint::black_box(stream.push(""));
                }
            }
            now() - start
        })
    };
    let unchanged = cost(false);
    let toggled = cost(true);
    assert!(
        toggled < unchanged * 8.0,
        "shared aliases: toggled {toggled:.6}s, unchanged {unchanged:.6}s"
    );
}

#[test]
fn an_unresolved_depth_cut_reports_each_occurrences_validation_errors() {
    // A depth cut prevents caching. Each occurrence therefore materializes
    // and reports its own error; an unresolved reference also forbids error
    // memoization, even though there is no cycle.
    let lib = Arc::new(Library::from_json_schema(&serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"type": "string"}}},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }})));
    let program = format!(
        "root = Stack([a, a])\na = Stack([missing, Text(1), deep])\ndeep = {}0{}\n",
        "[".repeat(20),
        "]".repeat(20)
    );
    let limits = Limits::default().with_depth(8);
    let result = parse_with_limits(&program, &lib, None, limits);
    assert!(result.depth_limited);
    assert_eq!(
        result
            .meta
            .errors
            .iter()
            .filter(|e| e.component == "Text" && e.path == "/value")
            .count(),
        2
    );
    streams_as_without_reuse(&program, &lib, limits, 7);
}
