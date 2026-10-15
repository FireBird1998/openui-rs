# 05 — Boundary experiment

**What to build:** Find out what each chunk costs as it crosses from Rust to Swift, using real native views.

First, measure the baseline, where the whole parse result crosses as JSON text after every chunk. Run it on the largest corpus programs, split into parsing, crossing, decoding and redrawing. Then pick one alternative, whichever the numbers point to:
- a handle to the Rust-side tree that Swift walks;
- or only what changed since the last chunk.

Build it behind a switch in the viewer and measure it the same way.

Write the results into Dynamic News tickets 01 and 06:
- the build setup;
- which threads calls land on, and the stack each needs;
- both sets of numbers;
- a recommendation for Android.

This finishes the viewer's first milestone.

**Blocked by:** 02 — Streaming playback; 04 — Native components

**Status:** ready-for-agent

- [ ] One command reproduces the timing run: a fixed set of the largest corpus programs, fixed chunkings, an optimized build. It reports the median and the worst time per chunk for parsing, crossing, decoding and redrawing.
- [ ] The choice of alternative is written down with its reason, drawn from the baseline numbers.
- [ ] The alternative is built in the binding and the viewer behind a switch.
- [ ] For every corpus case at every step, the alternative produces the same Swift model as the JSON path. This is tested.
- [ ] Both sets of numbers are recorded.
- [ ] Dynamic News ticket 01 gets the build setup, threads and stack findings as a comment.
- [ ] Dynamic News ticket 06 gets the comparison and a recommendation as a comment.
- [ ] The README's viewer section is current.

## Comments

**2026-10-15 (from ticket 02):** the scripted run (`VIEWER_PLAY=<case> VIEWER_CHUNK=<n>`) already plays a case in the real window and prints the median parse, crossing, decode and redraw per update, plus the longest main-thread stall. It's a starting point for this ticket's timing command. Two baselines from ticket 02, release build, on the largest case at 1 code point per chunk:
- **Parse and crossing:** about 11.5 ms of core time per 1,600-chunk update, and 0.2 ms to cross.
- **Redraw:** with the box drawing, the redraw (median 53 ms) dwarfs both.

**2026-10-15 (from ticket 04):** baselines with native drawing, release build, at 1 code point per chunk:

| | gallery | `bench/dashboard` | `synthetic/dashboard-x8` |
|---|---:|---:|---:|
| redraw, median | 96 ms | 85 ms | 93 ms |
| longest main-thread stall | 157 ms | 136 ms | 165 ms |
| core parse per update | about 5 ms | 8 ms | 13 ms |
| crossing per update | under 0.2 ms | under 0.2 ms | under 0.2 ms |

The redraw isn't charts: drawing them as boxes left `bench/dashboard` at 106 ms. It isn't rebuilding views either: SwiftUI skipping unchanged subtrees through `Equatable` didn't move it. Start by profiling layout and drawing in Instruments before blaming the boundary. The scripted run also plays program files now (an absolute path in `VIEWER_PLAY`), which is useful for a fixed timing set.
