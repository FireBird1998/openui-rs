---
status: accepted
---

# Grow the stack on demand with `stacker`

The core's recursive walks need up to about 320 KB of stack at the depth limit. iOS and Swift concurrency threads have 512 KB, and a stack overflow kills the app outright, with no error to catch. So the core calls `stacker` at its recursive points. When the stack runs low, it continues on a new segment on the heap.

This removes the stack requirement for every caller on every thread, and lets depth be raised like any other limit (ADR 0001). It costs about a nanosecond per check, and adds one dependency (the Rust project's own, MIT or Apache-2.0) that contains unsafe code. The core itself keeps `#![forbid(unsafe_code)]`.

Growing the stack covers the core's own work, but a result is dropped by whoever holds it, and Rust drops a tree by recursing into it. So results free their trees with a loop instead: `Value` and `Ast` (and the core's internal trees) implement `Drop`, taking their uniquely owned children out onto a list on the heap. A deep result then drops on any stack, at any depth limit, optimized or not, and nothing is left for callers to remember. The cost is that a pattern can no longer move a field out of a `Value` or an `Ast`, a breaking change: the maintainer chose it on 2026-11-07 over a `drop_deep` helper callers would have to remember, and over a ceiling on the depth limit.

`stacker` brings `psm`, whose build script (from 0.1.28) uses `ar_archive_writer`, licensed Apache-2.0 WITH LLVM-exception. It runs only while `psm` compiles and none of it is in the core, the binding or an app, so the crate stays MIT and free for any use. The maintainer approved that licence for it on 2026-11-07, so `psm` isn't pinned to an older version.

## Considered options

- **Document a minimum thread stack and test it.** It's cheap, but it leaves the risk with every app, and the headroom on Apple platforms is under 2×.
- **The binding runs each parse on a thread of its own with a large stack.** It's safe for binding callers, but not for Rust callers. It also costs a thread hop on every push and adds ordering and lifetime complexity to the binding.
