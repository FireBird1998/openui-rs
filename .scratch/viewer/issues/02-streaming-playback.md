# 02 — Streaming playback

**What to build:** The viewer plays the open case as a stream. The binding gains a stream session that can push a chunk, set the whole text so far, and return the current result. Parsing runs on a dedicated thread with a 1 MB stack, and the UI never calls the core itself.

The playback controls are:
- chunk size and speed;
- play, pause, and step forward or back one chunk;
- a slider to jump to any chunk.

Elements still arriving carry a subtle "still arriving" marker. Each chunk's time is shown, split into parsing, crossing to Swift, decoding and redrawing.

Playback is driven by the sources-of-chunks interface. A replay of a corpus case or file, at a chosen chunk size, is its first implementation.

**Blocked by:** 01 — One corpus case, drawn as boxes

**Status:** done (2026-10-15)

- [x] For every corpus case, the binding's stream result equals the core's at every step, under several chunkings including one character at a time. This test runs in `scripts/check.sh`.
- [x] Play, pause, step forward, step back, the slider, chunk size and speed all work.
- [x] Stepping back shows exactly the result the earlier step had.
- [x] Parsing happens only on the dedicated thread, which has a 1 MB stack. In debug builds, any call to the core from the main thread fails loudly.
- [x] Partial elements are visibly marked.
- [x] Each chunk's time is shown, split into parse, crossing, decoding and redrawing.
- [x] The UI stays responsive while the largest corpus program plays one character per chunk: the slider and pause keep working.
- [x] A Swift test checks that the replay source yields a known program's chunks in order, with the right boundaries, for several chunk sizes.

## Comments

**2026-10-15, done.** All checks pass: `scripts/check.sh`, with the binding's 7 tests, and `scripts/check-viewer.sh`, with 14 Swift tests.
- **Binding against the core:** the binding's stream equals the core's at every step of every corpus case, at 1, 3 and 16 code points per chunk. Large cases skip 1, as the corpus does.
- **Batched pushes:** `push_all` ends where pushing one by one does.
- **`set`:** it matches the core's.
- **Stepping back:** a Swift test steps through `hand/04` one character at a time, then goes back to earlier steps. With a big cache, and with one that keeps almost nothing so every step is rebuilt, each step shows exactly the JSON it had going forward.
- **The core thread:** a test checks that it has a 1 MB stack.

**How it works:**
- **One core thread.** A `Core` owns one thread named `openui-core` with a 1 MB stack and runs jobs on it in order. Every call to the binding happens in those jobs, and each wrapper asserts that it's on that thread, so debug builds stop at a call from anywhere else. Decoding happens there too, so the main thread only receives finished results.
- **What the binding gained.** `StreamSession` has `push`, `push_all`, `set` and `result`. `push_all` pushes chunks one by one but serializes only the last result, so a viewer can skip steps cheaply and still get exactly the result pushing them one at a time gives. Every result also carries the core's parse time and JSON time, measured in Rust.
- **Stepping back.** Recent steps are kept, up to 32 MB of JSON, dropping the steps farthest from the current one first. A step that isn't kept is rebuilt by pushing the same chunks into a new session. The core is deterministic, so the result is the same. The core's stream parser can't be cloned, so there's no cheaper checkpoint.
- **Playing.** About every 15 ms, playback pushes the chunks the speed says are due in one update, waiting for the previous one to be drawn. A slow drawing therefore slows playback instead of freezing the UI. Seeking or stepping pauses playback, and requests are handled one at a time, newest wins, so the session never sees two at once.
- **Timings.** Parse is measured in Rust; crossing is the call's time in Swift minus parse; decode is measured in Swift; redraw runs from handing over the result until an invisible view behind the drawing is drawn with the new step.
- **Chunk sizes.** The menu offers 1, 2, 3, 4, 7 and 16 code points, the corpus's fixed chunkings, so ticket 03 can compare against goldens, plus 64 and 256. Changing the size resumes at the step that has seen at least as much text.

**Responsiveness**, measured with the scripted run (`VIEWER_PLAY=<case>`, added for this; see the README). It plays the case as fast as possible in the real window, jumps back mid-play, plays on, pauses, and prints a report. On the largest case, `synthetic/dashboard-x8` (29.5 KB) at 1 code point per chunk, release build:

| | |
|---|---|
| per update (about 1,600 chunks) | parse 11.5 ms, crossing 0.2 ms, decode 0.4 ms |
| redraw | median 53 ms, worst 103 ms |
| longest main-thread stall | 125 ms |
| jumping back half way mid-play (rebuilding 13,000 steps) | 286 ms |
| updates after pausing | 0 |

The smallest program redraws in about 18 ms, one frame plus SwiftUI's update, and upstream's 3.5 KB dashboard in about 50 ms. Drawing the box tree is what costs, not the core. Making prop values plain text instead of selectable text halved the largest case's redraw, from a 94 ms median and a 222 ms worst stall. Debug builds parse about 5× slower and redraw the same.

Screen capture failed for a while in this session, and the screen was locked part of the time, so the UI was checked through scripted runs and background screenshots. Mid-play on the largest case, the controls, the timings line and the dashed "still arriving" borders all looked right.
