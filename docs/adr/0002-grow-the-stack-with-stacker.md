---
status: accepted
---

# Grow the stack on demand with `stacker`

The core's recursive walks need up to about 320 KB of stack at the depth limit. iOS and Swift concurrency threads have 512 KB, and a stack overflow kills the app outright, with no error to catch. So the core calls `stacker` at its recursive points. When the stack runs low, it continues on a new segment on the heap.

This removes the stack requirement for every caller on every thread, and lets depth be raised like any other limit (ADR 0001). It costs about a nanosecond per check, and adds one dependency (the Rust project's own, MIT or Apache-2.0) that contains unsafe code. The core itself keeps `#![forbid(unsafe_code)]`.

## Considered options

- **Document a minimum thread stack and test it.** It's cheap, but it leaves the risk with every app, and the headroom on Apple platforms is under 2×.
- **The binding runs each parse on a thread of its own with a large stack.** It's safe for binding callers, but not for Rust callers. It also costs a thread hop on every push and adds ordering and lifetime complexity to the binding.
