//! Growing the stack on demand (ADR 0002), so the core needs no minimum
//! thread stack: a stack overflow aborts the process, with nothing to catch.
//!
//! Recursive walks, and the helpers that start them, check [`grow_stack!`].
//! When less than [`RED_ZONE`] of the current
//! stack is left, the call starts again on a fresh [`SEGMENT`] allocated on
//! the heap, and the next segment chains on in the same way.
//!
//! Input-building walks use the two-argument form, with a minimal cut
//! value. A Limits entry scopes a ceiling of eight segments plus 64 KiB per
//! depth level, rounded up to segments, using saturating arithmetic. Nested
//! segments on this thread count even when an uncapped walk grew them. A
//! guard restores the enclosing ceiling and segment count on unwind too.
//! Reaching the ceiling is a core bug: debug/test builds panic naming the
//! walk; release builds use the stall channel to flag a cut result. Capped
//! walks then stop, including their sibling loops. Finished-tree walks use
//! the one-argument form and may grow past the ceiling, even inside a parse;
//! none calls back into an input-building walk to bypass its checks. A debug
//! build panics 1,024 segments past the ceiling, whichever walk grew them.

use std::cell::Cell;

/// What must be left on the stack before a recursive step goes on in place.
///
/// It has to cover the most stack used between two checks: one level of a
/// walk, plus whatever non-recursive work hangs off it (formatting numbers,
/// building strings, sorting). One level takes a few KB even unoptimized,
/// and a whole walk to the depth limit took under 1 MB at opt-level 0 when
/// measured (2026-10-07), so 128 KiB leaves a wide margin. On an ordinary
/// thread (512 KB and up) only its last 128 KiB is never used in place.
const RED_ZONE: usize = 128 * 1024;

/// The size of each segment allocated when the stack runs low.
///
/// Large enough that ordinary walks to the default depth limit usually
/// fit in one segment even unoptimized; evaluated defaults needed two in
/// the depth suite. A small thread grows per segment, rather than per level.
/// The pages are reserved, not touched, so unused space costs address space
/// only.
const SEGMENT: usize = 2 * 1024 * 1024;

/// Whether the stack is too low to go deeper in place.
///
/// Where `stacker` can't read the operating system's stack limit (Android and
/// iOS, as of 0.1.25), it can't measure the thread's own stack, so this says
/// low there and the walk starts on a segment of its own. `stacker` measures
/// the segments it grows wherever it can switch stacks, so the walk then goes
/// on as it does elsewhere. Where even a segment can't be measured, there is
/// no point growing another: once on a segment the core grew, it goes on in
/// place.
#[inline]
pub(crate) fn low() -> bool {
    low_given(stacker::remaining_stack(), ON_SEGMENT.get())
}

/// [`low`], given what `stacker` measures and whether the thread is on a
/// segment the core grew.
#[inline]
fn low_given(remaining: Option<usize>, on_segment: bool) -> bool {
    match remaining {
        Some(left) => left < RED_ZONE,
        None => !on_segment,
    }
}

thread_local! {
    /// Whether we are running on a segment we grew.
    static ON_SEGMENT: Cell<bool> = const { Cell::new(false) };
    static SEGMENTS: Cell<usize> = const { Cell::new(0) };
    static CEILING: Cell<usize> = const { Cell::new(usize::MAX) };
}

/// Run `f` on a fresh stack segment.
#[cold]
#[inline(never)]
pub(crate) fn on_new_segment<R>(f: impl FnOnce() -> R) -> R {
    /// Puts back whether the thread was on a segment, unwinding or not.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ON_SEGMENT.set(self.0);
            SEGMENTS.set(SEGMENTS.get() - 1);
        }
    }
    stacker::grow(SEGMENT, || {
        let _restore = Restore(ON_SEGMENT.replace(true));
        SEGMENTS.set(SEGMENTS.get() + 1);
        debug_assert!(
            SEGMENTS.get() <= runaway_bound(),
            "runaway: {} nested stack segments",
            SEGMENTS.get()
        );
        f()
    })
}

/// Nested segments a debug build grows past the ceiling before it panics.
///
/// A finished-tree walk has no ceiling, and a bug that makes every fresh
/// segment look low nests segments without end: gigabytes a second, which
/// froze a Mac before any outside watchdog could act (2026-11-11). The whole
/// suite nests 30 at most, and at the stack a level was measured to take,
/// 1,024 segments carry a walk some 260,000 levels deep. The unit tests nest
/// two, and the ones that say the stack is low hold more on each segment, so
/// they stop sooner.
const RUNAWAY: usize = if cfg!(test) { 64 } else { 1024 };

/// Where a debug build stops nesting segments: [`RUNAWAY`] past the ceiling
/// in force, or [`RUNAWAY`] alone outside any entry.
fn runaway_bound() -> usize {
    match CEILING.get() {
        usize::MAX => RUNAWAY,
        ceiling => RUNAWAY.saturating_add(ceiling),
    }
}

/// Number of nested segments allowed for an input-building walk.
/// The per-level allowance is rounded only after saturating multiplication.
fn ceiling(depth: usize) -> usize {
    8usize.saturating_add(depth.saturating_mul(64 * 1024).div_ceil(SEGMENT))
}

/// Restore the enclosing entry's ceiling, including on unwind.
pub(crate) struct Ceiling(usize);
impl Drop for Ceiling {
    fn drop(&mut self) {
        CEILING.set(self.0);
    }
}

#[inline]
pub(crate) fn limit(depth: usize) -> Ceiling {
    Ceiling(CEILING.replace(testing::ceiling_override().unwrap_or_else(|| ceiling(depth))))
}

/// Only called when a capped walk needs a fresh segment.
#[cold]
#[inline(never)]
pub(crate) fn exhausted(walk: &str) -> bool {
    if SEGMENTS.get() >= CEILING.get() || testing::injected(walk) {
        crate::progress::stack_stalled(walk);
    }
    crate::progress::stalled_so_far()
}

#[inline]
pub(crate) fn capped_low(walk: &str) -> bool {
    low() || crate::progress::stalled_so_far() || testing::injected(walk)
}

#[inline(always)]
pub(crate) fn budget_low(walk: &str) -> bool {
    low() || testing::injected(walk)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::cell::Cell;
    thread_local! {
        static WALK: Cell<Option<&'static str>> = const { Cell::new(None) };
        static ZERO: Cell<bool> = const { Cell::new(false) };
        pub(super) static HITS: Cell<usize> = const { Cell::new(0) };
    }
    // Inject a zero ceiling and a low-stack observation at one named site,
    // so even short corpus cases exercise every typed recovery branch.
    pub(crate) fn cutting<T>(walk: &'static str, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<&'static str>);
        impl Drop for Restore {
            fn drop(&mut self) {
                WALK.set(self.0);
            }
        }
        let _restore = Restore(WALK.replace(Some(walk)));
        HITS.set(0);
        f()
    }
    // Inspect the exact node returned by the cut, before an enclosing walk
    // can discard it. The expectations are literals, independent of the cut.
    pub(crate) fn assert_cut(walk: &str, value: &impl std::fmt::Debug) {
        if !WALK.get().is_some_and(|name| walk.starts_with(name)) {
            return;
        }
        let expected = match walk.split('(').next().unwrap() {
            "parse_text" => return,          // Pinned as a complete result below.
            "self.current_result" => return, // Pinned as a complete result below.
            "join" => "()",
            "walk" | "replay" => "false",
            "holds_expression" | "has_runtime" | "contains_dynamic_value" => "true",
            "loose_eq" | "validate_schema_value" => "false",
            "literal_value" | "substitute" => "None",
            "to_value" => "None",
            "filter" => "Null",
            "to_string" | "merge" => "\"\"",
            "self.expr" | "materialize_value" | "to_literal_ast" | "materialize_expr"
            | "state_default" | "from_value" | "evaluate" | "literal" | "elementwise" => "Null",
            _ => panic!("missing cut expectation: {walk}"),
        };
        assert_eq!(format!("{value:?}"), expected, "exact cut node in {walk}");
    }
    pub(super) fn ceiling_override() -> Option<usize> {
        ZERO.get().then_some(0)
    }
    pub(crate) fn zero<T>(f: impl FnOnce() -> T) -> T {
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                ZERO.set(self.0);
            }
        }
        let _restore = Restore(ZERO.replace(true));
        f()
    }
    pub(super) fn injected(walk: &str) -> bool {
        let hit = WALK.get().is_some_and(|name| walk.starts_with(name));
        if hit {
            HITS.set(HITS.get() + 1);
        }
        hit
    }
}
#[cfg(not(test))]
mod testing {
    pub(super) fn ceiling_override() -> Option<usize> {
        None
    }
    pub(super) fn injected(_: &str) -> bool {
        false
    }
}

/// At the top of a function that recurses (or starts a walk), with the call
/// that re-enters it: when the stack runs low, the function returns that call
/// made again on a fresh segment, which then has room and goes on in place.
///
/// ```ignore
/// fn walk(node: &Node) -> usize {
///     grow_stack!(walk(node));
///     node.children.iter().map(walk).sum::<usize>() + 1
/// }
/// ```
macro_rules! grow_stack {
    ($call:expr, else $cut:expr) => {
        $crate::stack::grow_stack!(@check $call, $crate::stack::capped_low(stringify!($call)), $cut);
    };
    // The walk's existing cursor/work/size state stops sibling descent.
    ($call:expr, budget else $cut:expr) => {
        $crate::stack::grow_stack!(@check $call, $crate::stack::budget_low(stringify!($call)), $cut);
    };
    (@check $call:expr, $low:expr, $cut:expr) => {
        if $low {
            if $crate::stack::exhausted(stringify!($call)) {
                let cut = $cut;
            #[cfg(test)]
            $crate::stack::testing::assert_cut(stringify!($call), &cut);
                return cut;
            }
            return $crate::stack::on_new_segment(|| $call);
        }
    };
    ($call:expr) => {
        if $crate::stack::low() {
            return $crate::stack::on_new_segment(|| $call);
        }
    };
}
pub(crate) use grow_stack;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measured_stack_is_low_inside_the_red_zone_only() {
        for on_segment in [false, true] {
            assert!(low_given(Some(RED_ZONE - 1), on_segment));
            assert!(!low_given(Some(RED_ZONE), on_segment));
        }
    }

    #[test]
    fn an_unmeasured_stack_is_low_until_on_a_segment_the_core_grew() {
        assert!(low_given(None, false));
        assert!(!low_given(None, true));
    }

    #[test]
    fn a_debug_build_stops_a_runaway_of_nested_segments() {
        fn nest(levels: usize) {
            if levels > 0 {
                on_new_segment(|| nest(levels - 1));
            }
        }
        let runs_away = |levels| std::panic::catch_unwind(|| nest(levels)).is_err();
        // Outside any entry, and past an entry's ceiling (8 at depth 0).
        assert!(!runs_away(RUNAWAY) && runs_away(RUNAWAY + 1));
        let _ceiling = limit(0);
        assert!(!runs_away(RUNAWAY + 8) && runs_away(RUNAWAY + 9));
        assert_eq!(SEGMENTS.get(), 0);
    }

    /// A ceiling stall in the evaluator spends its work, so the default being
    /// evaluated is declared null: never left out, and never the part of its
    /// value that was built before the cut.
    #[test]
    fn an_evaluator_stack_cut_declares_the_default_null() {
        let lib = crate::Library::from_json_schema(&serde_json::json!({"$defs": {
            "Text": {"properties": {"value": {}}}
        }}));
        for (walk, program, declared) in [
            (
                "to_value(",
                "root = Text($a)\n$a = 1 + 2",
                serde_json::json!({"$a": null}),
            ),
            (
                "to_string(",
                "root = Text($b)\n$a = @Sum([1, 2])\n$b = [5] + \"!\"",
                serde_json::json!({"$a": 3, "$b": null}),
            ),
            (
                "literal(",
                "root = Text($a)\n$a = @Each([1, 2], \"x\", x + 1)",
                serde_json::json!({"$a": null}),
            ),
        ] {
            let cut = crate::progress::testing::as_release(|| {
                testing::cutting(walk, || crate::parse(program, &lib, None))
            });
            assert!(testing::HITS.get() > 0 && cut.size_limited, "{walk}");
            let json: serde_json::Value = serde_json::from_str(&cut.to_json()).unwrap();
            assert_eq!(json["stateDeclarations"], declared, "{walk}");
        }
    }

    #[test]
    fn a_segment_says_so_while_it_runs_and_not_after() {
        assert!(!ON_SEGMENT.get());
        assert!(on_new_segment(|| ON_SEGMENT.get()));
        assert!(!ON_SEGMENT.get());
        // A segment grown on a segment leaves the thread on the outer one.
        assert!(on_new_segment(|| {
            on_new_segment(|| ());
            ON_SEGMENT.get()
        }));
        assert!(!ON_SEGMENT.get());
        let unwound = std::panic::catch_unwind(|| on_new_segment(|| panic!("in a segment")));
        assert!(unwound.is_err());
        assert!(!ON_SEGMENT.get());
    }
    #[test]
    fn ceiling_scales_and_saturates() {
        assert_eq!(ceiling(0), 8);
        assert_eq!(ceiling(1), 9);
        assert_eq!(ceiling(32), 9);
        assert_eq!(ceiling(33), 10);
        assert_eq!(ceiling(256), 16);
        assert_eq!(ceiling(20_000), 633);
        assert_eq!(ceiling(usize::MAX), 8 + usize::MAX.div_ceil(SEGMENT));
    }

    #[test]
    fn ceiling_and_segments_restore_on_unwind() {
        let previous = CEILING.get();
        let result = std::panic::catch_unwind(|| {
            let _limit = limit(1);
            assert_eq!(CEILING.get(), 9);
            on_new_segment(|| {
                assert_eq!(SEGMENTS.get(), 1);
                let _nested = limit(256);
                panic!("unwind the ceiling");
            });
        });
        assert!(result.is_err());
        assert_eq!(CEILING.get(), previous);
        assert_eq!(SEGMENTS.get(), 0);
    }

    // These need the injection hook, which is deliberately absent from the
    // published crate. Public API behavior remains in integration tests.
    #[test]
    fn every_cut_value_panics_or_reports_a_release_stall() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(check_every_cut_value)
            .unwrap()
            .join()
            .unwrap();
    }

    fn check_every_cut_value() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
        let mut cases = Vec::new();
        for schema in ["text", "hand", "libraries"] {
            let schema = serde_json::from_str(
                &std::fs::read_to_string(root.join(format!("schemas/{schema}.json"))).unwrap(),
            )
            .unwrap();
            let library = crate::Library::from_json_schema(&schema);
            for program in [
                "root = Text(\"hi\")",
                "root = Root([item, item])\nitem = Text(\"hi\")",
                "root = Page([], \"oops\")",
                "root = Widget(@Format(\"a\", {upper: true}))",
                "root = Text($a)\n$a = [1, $b]",
                "root = Text($a)\n$a = 1 + 2",
                "root = Text($a)\n$a = @Each([1], \"x\", x + 1)",
                "root = Text($a)\n$a = @Round([1.2])",
                "root = Text($a)\n$a = @Filter([1], \"\", \"==\", 1)",
                "root = Text($a)\n$a = [1] + \"!\"",
            ] {
                cases.push((library.clone(), program));
            }
        }
        for walk in [
            "parse_text(",
            "self.expr(",
            "walk(parts, seen",
            "walk(parts, out",
            "materialize_value(",
            "replay(",
            "literal_value(",
            "to_literal_ast(",
            "materialize_expr(",
            "contains_dynamic_value(",
            "validate_schema_value(",
            "holds_expression(",
            "has_runtime(",
            "state_default(",
            "from_value(",
            "to_value(",
            "evaluate(",
            "substitute(",
            "literal(",
            "elementwise(",
            "filter(",
            "to_string(",
            "join(",
            "loose_eq(",
        ] {
            let mut exercised = false;
            for (lib, program) in &cases {
                let result = crate::progress::testing::as_release(|| {
                    testing::cutting(walk, || crate::parse(program, lib, None))
                });
                if testing::HITS.get() == 0 {
                    continue;
                }
                assert!(result.size_limited, "{walk}: unflagged cut");
                assert!(!result.depth_limited, "{walk}: stall is not a depth cut");
                if walk == "validate_schema_value(" {
                    let root = result.root.as_ref().unwrap();
                    assert!(matches!(root.props.get("value"), Some(crate::Value::Null)));
                }

                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    testing::cutting(walk, || crate::parse(program, lib, None));
                }))
                .unwrap_err();
                let message = panic.downcast::<String>().unwrap();
                assert!(message.contains("grown stack ceiling"), "{message}");
                assert!(message.contains(walk), "{walk}: {message}");
                exercised = true;
                break;
            }
            assert!(exercised, "no case exercised {walk}");
        }
    }

    fn assert_empty_cut(result: &crate::ParseResult) {
        assert!(result.size_limited);
        assert!(!result.depth_limited);
        assert_eq!(
            result.to_json(),
            r#"{"root":null,"meta":{"incomplete":true,"unresolved":[],"orphaned":[],"statementCount":0,"errors":[]},"stateDeclarations":{},"queryStatements":[],"mutationStatements":[]}"#
        );
    }

    #[test]
    fn diagnostic_flattening_cuts_write_nothing() {
        use crate::materialize::{ErrorPart, UnresolvedPart, flatten_errors, flatten_unresolved};
        let error = crate::parse("root = Missing()", &Default::default(), None)
            .meta
            .errors
            .remove(0);
        let errors = crate::progress::testing::as_release(|| {
            crate::progress::watch(|| {
                testing::cutting("walk(parts, seen", || {
                    flatten_errors(&[ErrorPart::Own(error)])
                })
            })
            .0
        });
        assert!(testing::HITS.get() > 0);
        let unresolved = crate::progress::testing::as_release(|| {
            crate::progress::watch(|| {
                testing::cutting("walk(parts, out", || {
                    flatten_unresolved(&[UnresolvedPart::Own("missing".into())])
                })
            })
            .0
        });
        assert!(testing::HITS.get() > 0);
        assert!(errors.is_empty());
        assert_eq!(unresolved, Vec::<String>::new());
    }

    #[test]
    fn the_parser_entry_cut_is_exactly_empty() {
        let cut = crate::progress::testing::as_release(|| {
            testing::cutting("parse_text(", || {
                crate::parse("root = 1", &Default::default(), None)
            })
        });
        assert_empty_cut(&cut);
    }

    #[test]
    fn a_stack_cut_stream_stays_cut_until_set() {
        let mut stream = crate::StreamParser::new(Default::default(), None);
        let current_cut = crate::progress::testing::as_release(|| {
            testing::cutting("self.current_result(", || stream.push("root = 1"))
        });
        assert_empty_cut(&current_cut);
        stream.set("");
        let first = crate::progress::testing::as_release(|| {
            testing::cutting("self.expr(", || stream.push("root = [1, 2]\n"))
        });
        assert_empty_cut(&first);
        assert_eq!(first.to_json(), stream.push("other = 3\n").to_json());
        assert_eq!(first.to_json(), stream.finish().to_json());
        assert!(!stream.set("root = 1").size_limited);
    }
    #[test]
    fn zero_ceiling_on_a_small_stack_panics_or_cuts_a_legitimate_deep_program() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let program = format!("root = {}1{}", "[".repeat(3000), "]".repeat(3000));
                let limits = crate::Limits::DEFAULT
                    .with_depth(4096)
                    .with_result_size(8 * 1024 * 1024);
                let library = crate::Library::default();
                let run = || crate::parse_with_limits(&program, &library, None, limits);
                assert!(!run().size_limited);
                let panic = std::panic::catch_unwind(|| testing::zero(run)).unwrap_err();
                assert!(
                    panic
                        .downcast::<String>()
                        .unwrap()
                        .contains("grown stack ceiling")
                );
                let cut = crate::progress::testing::as_release(|| testing::zero(run));
                assert!(cut.size_limited);
                assert!(prefix(
                    &crate::parse_json_with_limits(&cut.to_json(), limits).unwrap(),
                    &crate::parse_json_with_limits(&run().to_json(), limits).unwrap()
                ));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    // Only data is compared: flags and diagnostics describe the cut itself.
    fn prefix(cut: &serde_json::Value, whole: &serde_json::Value) -> bool {
        use serde_json::Value as Json;
        match (cut, whole) {
            (Json::Null, _) => true,
            (Json::Object(a), _) if a.get("k").and_then(Json::as_str) == Some("Null") => true,
            (Json::Array(a), Json::Array(b)) => {
                a.len() <= b.len() && a.iter().zip(b).all(|(a, b)| prefix(a, b))
            }
            (Json::Object(a), Json::Object(b)) => a
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "meta" | "sizeLimited" | "depthLimited" | "hasDynamicProps" | "partial"
                    )
                })
                .all(|(k, a)| b.get(k).is_some_and(|b| prefix(a, b))),
            _ => cut == whole,
        }
    }
    #[test]
    fn a_merge_stack_cut_is_flagged_and_stops() {
        for walk in ["merge(", "self.expr("] {
            let cut = crate::progress::testing::as_release(|| {
                testing::cutting(walk, || {
                    crate::merge_statements_with_limits(
                        "root = 1",
                        "root = 2",
                        None,
                        crate::Limits::DEFAULT,
                    )
                })
            });
            assert!(cut.size_limited);
            assert!(cut.text.is_empty());
            assert!(!cut.depth_limited);
        }
    }

    #[test]
    fn deep_programs_cut_consistently_in_each_building_family() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let lib = crate::Library::from_json_schema(&serde_json::json!({"$defs": {
                    "Text": {"properties": {"value": {}}},
                    "Stack": {"properties": {"children": {"type": "array"}}}
                }}));
                let nested = format!("{}1{}", "[".repeat(400), "]".repeat(400));
                let limits = crate::Limits::DEFAULT
                    .with_depth(2048)
                    .with_result_size(8 * 1024 * 1024);
                for (walk, program) in [
                    ("self.expr(", format!("root = Stack({nested})")),
                    ("materialize_value(", format!("root = Stack({nested})")),
                    ("materialize_expr(", format!("root = Text($x + {nested})")),
                    ("validate_schema_value(", format!("root = Stack({nested})")),
                    ("evaluate(", format!("root = Text($v)\n$v = @Sum({nested})")),
                ] {
                    let run = || crate::parse_with_limits(&program, &lib, None, limits);
                    let whole = run();
                    assert!(
                        !whole.size_limited && !whole.depth_limited,
                        "{walk}: legitimate input was cut"
                    );
                    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        testing::cutting(walk, run)
                    }))
                    .unwrap_err();
                    assert!(panic.downcast::<String>().unwrap().contains(walk));
                    let cut = crate::progress::testing::as_release(|| testing::cutting(walk, run));
                    assert!(testing::HITS.get() > 0 && cut.size_limited, "{walk}");
                    let cut: serde_json::Value =
                        crate::parse_json_with_limits(&cut.to_json(), limits).unwrap();
                    let whole: serde_json::Value =
                        crate::parse_json_with_limits(&whole.to_json(), limits).unwrap();
                    assert!(prefix(&cut, &whole), "{walk}: inconsistent cut");
                    crate::free_json(cut);
                    crate::free_json(whole);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a_ceiling_cut_does_not_redescend_both_branches() {
        fn twice(calls: &mut usize, first: &mut usize) {
            *calls += 1;
            grow_stack!(twice(calls, first), else { if *first == 0 { *first = *calls; } });
            twice(calls, first);
            twice(calls, first);
        }
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut calls = 0;
                let mut first = 0;
                let (_, stalled) = crate::progress::testing::as_release(|| {
                    crate::progress::watch(|| {
                        testing::zero(|| {
                            let _ceiling = limit(256);
                            twice(&mut calls, &mut first);
                        })
                    })
                });
                assert!(stalled);
                assert!(calls <= 2 * first + 1, "walk revisited siblings: {calls}");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn finished_walks_ignore_an_installed_ceiling() {
        fn finished() -> usize {
            grow_stack!(finished());
            SEGMENTS.get()
        }
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let _restore = Ceiling(CEILING.replace(0));
                assert!(finished() >= 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn the_public_expression_entry_starts_a_clean_watch() {
        let tokens = crate::tokenize("[1, 2]");
        let (ast, cut) = crate::progress::testing::as_release(|| {
            testing::cutting("self.expr(", || {
                crate::parse_expression_with_limits(&tokens, crate::Limits::DEFAULT)
            })
        });
        assert!(matches!(ast, crate::Ast::Null) && !cut);
        let (_, cut) = crate::parse_expression_with_limits(&tokens, crate::Limits::DEFAULT);
        assert!(!cut);
    }
}
