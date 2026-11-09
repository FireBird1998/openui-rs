//! Growing the stack on demand (ADR 0002), so the core needs no minimum
//! thread stack: a stack overflow aborts the process, with nothing to catch.
//!
//! Every public entry point that walks a tree, and every recursive walk,
//! starts with [`grow_stack!`]. When less than [`RED_ZONE`] of the current
//! stack is left, the call starts again on a fresh [`SEGMENT`] allocated on
//! the heap, and the next segment chains on in the same way.

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
/// Large enough that a whole walk to the default depth limit fits in one
/// segment even unoptimized, so a call made on a small thread allocates once,
/// at its entry point, rather than at every level. The pages are reserved,
/// not touched, so the unused part costs address space only.
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
    /// Whether this thread is running on a segment [`on_new_segment`] grew.
    static ON_SEGMENT: Cell<bool> = const { Cell::new(false) };
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
        }
    }
    stacker::grow(SEGMENT, || {
        let _restore = Restore(ON_SEGMENT.replace(true));
        f()
    })
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
}
