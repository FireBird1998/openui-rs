//! Growing the stack on demand (ADR 0002), so the core needs no minimum
//! thread stack: a stack overflow aborts the process, with nothing to catch.
//!
//! Every public entry point that walks a tree, and every recursive walk,
//! starts with [`grow_stack!`]. When less than [`RED_ZONE`] of the current
//! stack is left, the call starts again on a fresh [`SEGMENT`] allocated on
//! the heap, and the next segment chains on in the same way.

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

/// Whether the stack is too low to go deeper in place. On a platform where
/// `stacker` can't measure the stack it isn't, as in `stacker::maybe_grow`:
/// a new segment there would measure no better, and re-entering would loop.
#[inline]
pub(crate) fn low() -> bool {
    stacker::remaining_stack().is_some_and(|left| left < RED_ZONE)
}

/// Run `f` on a fresh stack segment.
#[cold]
#[inline(never)]
pub(crate) fn on_new_segment<R>(f: impl FnOnce() -> R) -> R {
    stacker::grow(SEGMENT, f)
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
