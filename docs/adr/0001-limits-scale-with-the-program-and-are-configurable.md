---
status: accepted
---

# Limits scale with the program, are configurable, and cut the result instead of failing

A model's reply is untrusted input that the core parses on phones. The long runs against lang-core 1.1 showed small programs costing seconds or 127 times their size, all within the fixed 8 MiB result cap.

So every limit (program size, result size, depth, evaluation work) is one `Limits` value. It has documented defaults, and an embedder may raise or lower each one:
- **Program size:** 1 MiB by default.
- **Result size:** the larger of 64 KiB and 64 times the program's size, never above 8 MiB.
- **Evaluation work:** the larger of a million steps and 1,000 steps per byte of program, never above 64 million.
- **Depth:** 256 by default.

Work is counted in steps and bytes, never in time, so results stay the same on every machine. Hitting a limit gives a cut result flagged `size_limited` or `depth_limited`, never an error or a panic. A stall in a release build (a bug in the core, not a limit) ends the parse the same way, flagged `size_limited`, as decided in openui-rs #1. No default may cut a program in the conformance corpus: if one would, the default is raised.

## Considered options

- **A fixed cap only (8 MiB, as before).** It bounds memory but not amplification. A streaming caller serializing per push could still write gigabytes for one reply.
- **Refusing oversized input with an error.** It's simpler, but a stream already being drawn would lose everything at once. A cut result keeps what fits, and says so.
- **Hard-coded limits.** They're safe, but an embedder with a different budget (a desktop app or a watch) couldn't adapt them without forking.
