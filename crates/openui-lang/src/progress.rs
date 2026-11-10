//! The stall guard: every loop that consumes input either advances or stops.
//!
//! Such a loop checks its position at the top of every pass. A position that
//! isn't past the one at the last check is a stall: a bug in the core, never
//! the input's fault, and one that would otherwise allocate without bound.
//! In debug and test builds a stall panics, so the bug fails a test at once.
//! In release builds the loop stops there instead, and the parse around it
//! gives a cut result flagged `size_limited`: a panic must never cross the
//! binding.
//!
//! Loops that only skip over bytes are written so they can't stall at all
//! (an iterator's count, or a `for` over a range); the guard is for loops
//! that move a cursor by hand.

use std::cell::Cell;

thread_local! {
    /// Whether a loop stalled since the innermost [`watch`] began.
    static STALLED: Cell<bool> = const { Cell::new(false) };
}

/// One loop's guard.
pub(crate) struct Progress {
    /// The loop, as a stall's panic names it.
    what: &'static str,
    last: Option<usize>,
}

impl Progress {
    pub(crate) const fn new(what: &'static str) -> Progress {
        Progress { what, last: None }
    }

    /// Whether the loop has stalled at `pos`: not past where it was at the
    /// last check. Panics in debug builds; in release builds the caller
    /// stops the loop.
    #[must_use]
    pub(crate) fn stalled(&mut self, pos: usize) -> bool {
        let stalled = self.last.is_some_and(|last| pos <= last) || testing::injected(self.what);
        self.last = Some(pos);
        if stalled {
            if panics() {
                panic!("stall: {} stopped advancing at {pos}", self.what);
            }
            STALLED.with(|s| s.set(true));
        }
        stalled
    }
}

/// A recursive input walk requesting too many stack segments is a core bug,
/// with the same release recovery as a loop that stopped advancing.
#[cold]
#[inline(never)]
pub(crate) fn stack_stalled(walk: &str) {
    if panics() {
        panic!("stall: grown stack ceiling reached in {walk}");
    }
    STALLED.with(|s| s.set(true));
    crate::evaluate::stack_stalled();
}

/// Run `f`, and say whether a loop stalled in it.
pub(crate) fn watch<T>(f: impl FnOnce() -> T) -> (T, bool) {
    let outer = STALLED.with(|s| s.replace(false));
    let value = f();
    let stalled = STALLED.with(|s| s.replace(outer || s.get()));
    (value, stalled)
}

/// Whether a loop has stalled since the innermost [`watch`] began, so the
/// parse can stop there.
#[inline(always)]
pub(crate) fn stalled_so_far() -> bool {
    STALLED.with(Cell::get)
}

#[cfg(not(test))]
fn panics() -> bool {
    cfg!(debug_assertions)
}

#[cfg(test)]
fn panics() -> bool {
    !testing::RELEASE.with(Cell::get)
}

/// Hooks for the unit tests: force a stall in a named loop, and take the
/// release path in a test build. Absent from the published crate.
#[cfg(test)]
pub(crate) mod testing {
    use std::cell::Cell;

    thread_local! {
        /// A loop's name, and how many more of its checks pass before one stalls.
        static INJECT: Cell<Option<(&'static str, usize)>> = const { Cell::new(None) };
        pub(super) static RELEASE: Cell<bool> = const { Cell::new(false) };
    }

    /// Make the loop named `what` stall at its `after`-th check from now
    /// (0 for the next one), once.
    pub(crate) fn stall(what: &'static str, after: usize) {
        INJECT.with(|i| i.set(Some((what, after))));
    }

    /// Run `f` as a release build would: a stall cuts instead of panicking.
    pub(crate) fn as_release<T>(f: impl FnOnce() -> T) -> T {
        RELEASE.with(|r| r.set(true));
        let value = f();
        RELEASE.with(|r| r.set(false));
        INJECT.with(|i| i.set(None));
        value
    }

    pub(super) fn injected(what: &str) -> bool {
        INJECT.with(|i| match i.get() {
            Some((name, 0)) if name == what => {
                i.set(None);
                true
            }
            Some((name, n)) if name == what => {
                i.set(Some((name, n - 1)));
                false
            }
            _ => false,
        })
    }
}

#[cfg(not(test))]
mod testing {
    pub(super) fn injected(_: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::testing::as_release;
    use super::*;

    #[test]
    fn advancing_is_not_a_stall() {
        let mut progress = Progress::new("a loop");
        for pos in [0, 1, 5, 6] {
            assert!(!progress.stalled(pos));
        }
    }

    #[test]
    #[should_panic(expected = "stall: a loop stopped advancing at 3")]
    fn standing_still_panics_in_a_test_build() {
        let mut progress = Progress::new("a loop");
        let _ = progress.stalled(3);
        let _ = progress.stalled(3);
    }

    #[test]
    #[should_panic(expected = "stall: a loop stopped advancing at 2")]
    fn moving_back_panics_in_a_test_build() {
        let mut progress = Progress::new("a loop");
        let _ = progress.stalled(3);
        let _ = progress.stalled(2);
    }

    #[test]
    fn in_a_release_build_a_stall_is_reported_to_the_watch() {
        let ((), stalled) = as_release(|| {
            watch(|| {
                let mut progress = Progress::new("a loop");
                assert!(!progress.stalled(3));
                assert!(progress.stalled(3));
            })
        });
        assert!(stalled);
        let ((), stalled) = watch(|| {});
        assert!(!stalled, "a watch starts clean");
    }

    #[test]
    fn a_stall_inside_a_nested_watch_reaches_the_outer_one() {
        let (inner, outer) = as_release(|| {
            watch(|| {
                let ((), inner) = watch(|| {
                    let mut progress = Progress::new("a loop");
                    let _ = progress.stalled(1);
                    let _ = progress.stalled(0);
                });
                inner
            })
        });
        assert!(inner && outer);
    }

    /// The guarded loops, each with the entry point that runs it.
    const LOOPS: &[(&str, fn())] = &[
        ("the lexer", parse_a_program),
        ("statement splitting", parse_a_program),
        ("the expression parser", parse_a_program),
        ("the preprocessor", parse_a_program),
        ("the preprocessor", stream_a_program),
        ("the JSON reader", read_json),
    ];

    fn parse_a_program() {
        crate::parse(
            "a = [1, {b: 2}]\nb = a + 1",
            &crate::Library::default(),
            None,
        );
    }

    fn stream_a_program() {
        let mut stream = crate::StreamParser::new(Default::default(), None);
        stream.push("a = [1, {b: 2}]\nb = a");
    }

    fn read_json() {
        let _ = crate::parse_json(r#"[1, {"a": "b"}]"#);
    }

    const PROGRAM: &str = "root = Stack([a, b])\na = [1, {b: 2}]\nb = \"two\"\nc = 3";

    fn parse(text: &str) -> crate::ParseResult {
        crate::parse(text, &crate::Library::default(), None)
    }

    #[test]
    fn in_a_release_build_a_stall_cuts_the_parse_and_flags_it() {
        assert!(!parse(PROGRAM).size_limited);
        for what in [
            "the lexer",
            "statement splitting",
            "the expression parser",
            "the preprocessor",
        ] {
            // Partway in, so some of the program is read before the stall.
            let result = as_release(|| {
                testing::stall(what, 3);
                parse(PROGRAM)
            });
            assert!(result.size_limited, "{what}");
            assert!(result.meta.statement_count < 4, "{what}: not cut");
        }
    }

    #[test]
    fn in_a_release_build_the_json_reader_refuses_on_a_stall() {
        let read = as_release(|| {
            testing::stall("the JSON reader", 2);
            crate::parse_json(r#"[1, {"a": "b"}]"#)
        });
        assert!(read.unwrap_err().to_string().contains("stalled"));
    }

    #[test]
    fn in_a_release_build_a_stalled_stream_ignores_pushes_until_set() {
        let mut stream = crate::StreamParser::new(Default::default(), None);
        let (first, second) = as_release(|| {
            stream.push("a = 1\n");
            testing::stall("the lexer", 0);
            let first = stream.push("b = 2\n");
            let second = stream.push("c = 3\n");
            (first, second)
        });
        assert!(first.size_limited);
        assert_eq!(second.to_json(), first.to_json(), "the push was taken in");
        assert!(second.size_limited);
        assert_eq!(stream.result().to_json(), first.to_json());

        let fresh = stream.set("a = 1\nb = 2\nc = 3\n");
        assert!(!fresh.size_limited);
        assert_eq!(fresh.meta.statement_count, 3);
        assert_eq!(stream.push("d = 4\n").meta.statement_count, 4);
    }

    #[test]
    fn every_guarded_loop_panics_on_a_stall_in_a_test_build() {
        for &(what, run) in LOOPS {
            testing::stall(what, 0);
            let outcome = std::panic::catch_unwind(run);
            testing::stall("nothing", 0);
            let message = outcome
                .err()
                .and_then(|e| e.downcast::<String>().ok())
                .unwrap_or_else(|| panic!("{what}: no stall panic"));
            assert!(
                message.starts_with(&format!("stall: {what} stopped advancing")),
                "{what}: {message}"
            );
        }
    }
}
