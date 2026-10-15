# 05 — Boundary experiment, rescoped: where the redraw time goes

**Rescoped 2026-10-15** (decided with the maintainer after ticket 04). Tickets 02 and 04 measured the crossing at under 0.2 ms per update against a redraw of 85–96 ms, so an alternative to sending JSON would barely matter. This ticket now profiles the redraw instead and fixes what it finds. It still records the boundary numbers for Dynamic News. Building a handle or diff alternative is dropped unless the profile points at the boundary.

**What to build:**
- **Measure.** Make the timing run one command: a fixed set of the largest programs, fixed chunkings, an optimized build, reporting the median and worst per update for parsing, crossing, decoding and redrawing.
- **Profile.** Profile the redraw with Instruments (`xctrace`, which comes with Xcode).
- **Fix.** Fix the biggest costs it shows, and measure again.
- **Hand over.** Write into Dynamic News tickets 01 and 06 what the viewer learned: the build setup, which threads calls land on and the stack each needs, the boundary numbers, and a recommendation for Android.

This finishes the viewer's first milestone.

**Blocked by:** 02 — Streaming playback; 04 — Native components

**Status:** done (2026-10-15)

- [x] One command reproduces the timing run: a fixed set of the largest programs, fixed chunkings, an optimized build. It reports the median and the worst per update for parsing, crossing, decoding and redrawing.
- [x] A time profile of the redraw names where the main thread's time goes.
- [x] The biggest costs it shows are fixed or explained, with numbers before and after.
- [x] Dynamic News ticket 01 gets the build setup, threads and stack findings as a comment.
- [x] Dynamic News ticket 06 gets the boundary numbers and a recommendation as a comment.
- [x] The README's viewer section is current.

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

**2026-10-15, done.** `scripts/viewer-timing.sh` runs the fixed set (the gallery, `bench/dashboard`, `synthetic/dashboard-x2` and `-x8`) in two modes:
- **Steady:** 30 chunks a second, so updates carry 1–2 chunks, like streaming from a model.
- **Stress:** 1 code point per chunk as fast as possible, so updates carry hundreds of chunks.

`VIEWER_SPEED` was added to the scripted run for this, and it now reports the median and worst for every part.

**What the profile showed.** A Time Profiler recording (`xcrun xctrace record --template 'Time Profiler' --launch …`, exported to XML and summed by function) of the largest program under stress:
- **4.7 s of main-thread time in all.**
- **About 2.1 s in `NSHostingView.layout`:** SwiftUI's layout of the whole window, mostly stacks and flexible frames measuring and aligning their children (`StackLayout`, `_FlexFrameLayout`, `explicitAlignment`).
- **About 1.1 s updating and rendering the view graph.**
- **Small parts:** the viewer's own view bodies about 8 ms, the inspector's list about 100 ms, the program text about 100 ms, charts under 20 ms.

**What changed:**
- **Column stacks are now lazy** (`LazyVStack`), so only what's on screen is laid out. Steady, largest program: redraw median 54 → 37 ms, worst 267 → 48 ms, longest stall 128 → 64 ms. On the gallery and dashboard, which are mostly on screen, it changed little: 25 → 23 ms and 24 → 22 ms medians.
- **Reveal scrolls through ancestors.** Because lazy stacks build only what's been on screen, picking an element in the tree now scrolls through its ancestors outermost first, so a deep element is reached.

**What didn't help, and was taken out again:**
- explicit alignment guides on each element, so stacks wouldn't walk subtrees: no change;
- `Equatable` element views, so SwiftUI skips unchanged subtrees: no change (tried in ticket 04);
- drawing charts as boxes: no change (ticket 04).

**After** (median / worst redraw, longest stall), from the final timing run:

| | steady | stress |
|---|---|---|
| gallery | 25 / 36 ms, 86 ms | 95 / 130 ms, 194 ms |
| `bench/dashboard` | 26–30 / 93–105 ms, 79–85 ms | 85 / 125 ms, 192 ms |
| `synthetic/dashboard-x2` | 38 / 109 ms, 117 ms | 72 / 102 ms, 140 ms |
| `synthetic/dashboard-x8` | 44 / 60 ms, 91 ms | 72 / 102 ms, 141 ms |

About 17 ms of every redraw is waiting for the next frame, so the real work under steady streaming is about 10–25 ms. The stress worsts are when an update adds hundreds of chunks of content at once, as when jumping. Parse, crossing and decoding stay under 1 ms in steady mode for every program. One steady run of the dashboard showed a 484 ms stall that three reruns didn't repeat (79–85 ms). The machine's load average was about 4 at the time, so treat single outliers as noise.

**Where it goes next:** the remaining cost is SwiftUI laying out and drawing the visible screen. More could come from fewer flexible frames, or from caching sizes, but streaming already keeps up. The boundary findings are in Dynamic News tickets 01 (build, threads, stack) and 06 (numbers and the recommendation: send JSON).

