---
status: accepted
---

# Grow the stack on demand with `stacker`

The core's recursive walks need up to about 320 KB of stack at the depth limit. iOS and Swift concurrency threads have 512 KB, and a stack overflow kills the app outright, with no error to catch. So the core calls `stacker` at its recursive points. When the stack runs low, it continues on a new segment on the heap.

This removes the stack requirement for every caller on every thread, and lets depth be raised like any other limit (ADR 0001). It costs about a nanosecond per check, and adds one dependency (the Rust project's own, MIT or Apache-2.0) that contains unsafe code. The core itself keeps `#![forbid(unsafe_code)]`.

`stacker` can't read the thread's stack limit everywhere: as of 0.1.25, Android and iOS get its fallback, which can't measure the thread's stack at all. There a walk starts on a segment of its own, which `stacker` does measure, and goes on as it does elsewhere (#25, found running the suite on Android). That costs a 2 MB reservation for each walk the core starts outside a segment, about 4–5 µs per push on the arm64 emulator: a 3.5 KB program streamed in 870 pushes took 11.4 ms instead of 7.4 ms. If `stacker` learns to read the limit there, the reservation goes away by itself.

Growing the stack covers the core's own work, but a result is dropped by whoever holds it, and Rust drops a tree by recursing into it. So results free their trees with a loop instead: `Value` and `Ast` (and the core's internal trees) implement `Drop`, taking their uniquely owned children out onto a list on the heap. A deep result then drops on any stack, at any depth limit, optimized or not, and nothing is left for callers to remember. The cost is that a pattern can no longer move a field out of a `Value` or an `Ast`, a breaking change: the maintainer chose it on 2026-11-07 over a `drop_deep` helper callers would have to remember, and over a ceiling on the depth limit.

`stacker` brings `psm`, whose build script (from 0.1.28) uses `ar_archive_writer`, licensed Apache-2.0 WITH LLVM-exception. It runs only while `psm` compiles and none of it is in the core, the binding or an app, so the crate stays MIT and free for any use. The maintainer approved that licence for it on 2026-11-07, so `psm` isn't pinned to an older version.

## A ceiling for input-building walks (#24)

Grown segments are mapped directly, outside the test allocator. A core bug
that recurses forever could therefore grow gigabytes despite the allocation
cap. Each input-building entry scopes a thread-local ceiling, restored even
on unwind: **8 segments plus 64 KiB per depth level**, rounded up to 2 MiB
segments, with saturating arithmetic, above the segment count at that entry.
Segments already grown by an enclosing public call do not spend the new
entry's allowance. Expression evaluations sharing an active depth chain keep
that chain's original base, including across fresh nested evaluations.
Raising `Limits::depth` raises the ceiling; there is no new public limit.

Capped recursive walks name themselves in a panic in debug/test builds. In
release builds they report through the existing stall channel and return
a minimal cut value (null, no value, a short-circuit predicate, or nothing
written). Further
capped calls and sibling loops stop. The result is flagged `size_limited`, a
stream returns the empty cut result and stays so until `set`, and merging
stops. Statements not reached may be listed as orphaned. A ceiling stall
alone never sets `depth_limited`. This is a core bug, never
bad input. Finished-tree walks (serialization, equality, schema/prompt
rendering and freeing) remain uncapped, even inside a scoped entry.

A debug build also guards repeated segment growth without usable-stack
progress: 1,024 segments beyond the active ceiling or the last healthy
uncapped observation (64 in the unit suite). A bug that makes every fresh
segment look low still stops; a legitimate finished walk can exceed 1,024
segments. Healthy observations are recorded only for uncapped debug/test
walks, leaving release parsing's path unchanged. The prior absolute guard
panicked on legitimate multi-million-level finished trees. The public
`runaway_finished_walk` test walks, serializes and drops 4,100,000 levels on a
64 KiB thread, including at opt-level 0; restoring the absolute guard fails
that unoptimized test. Callback suspension still establishes its own base.

Measured at opt-level 0 on 512 KiB threads, `tests/depth.rs` needed at most
2 nested segments; `tests/drop.rs`'s raised-depth case needed 30. The
respective ceilings are 16 (depth 256) and 633 (depth 20,000): **8× and
21.1× headroom**, both above the required 4×. Measurement also exposed
recursive JSON cloning in serializing state defaults on a 512 KiB caller;
that path now borrows defaults, and the depth test frees JSON iteratively.

## Considered options

- **Document a minimum thread stack and test it.** It's cheap, but it leaves the risk with every app, and the headroom on Apple platforms is under 2×.
- **The binding runs each parse on a thread of its own with a large stack.** It's safe for binding callers, but not for Rust callers. It also costs a thread hop on every push and adds ordering and lifetime complexity to the binding.
