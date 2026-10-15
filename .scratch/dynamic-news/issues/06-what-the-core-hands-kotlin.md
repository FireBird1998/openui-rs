# 06 — What does the core hand Kotlin on each chunk?

Type: grilling
Status: open
Blocked by: 01 (and openui-lang ticket 07)

## Question

During a streaming parse, does the Kotlin side get the whole parse result after every chunk, or only what changed since the last one? Which shape lets Compose redraw only the changed parts of a story screen, and what does each cost to send across the boundary?

## Comments

**2026-10-06 (from openui-lang ticket 02):** upstream's blog post "Rewriting our Rust WASM Parser in TypeScript" measured the cost of crossing a language boundary with a whole parse result on every chunk. The boundary dominated, not the parsing: about 20–61 µs per call in the browser, and returning native objects (serde-wasm-bindgen) was slower than a JSON string. The same question applies to Kotlin over UniFFI/JNI. Measure the per-chunk crossing cost, not just parse speed.

**Note from openui-lang ticket 10 (2026-10-06):** elements in a parse result are reference-counted (`Arc<Element>`) and shared between consecutive streaming results. Handing Kotlin the whole tree after every push would undo that: the binding should hand over only what changed, or an opaque handle plus a way to walk it. Decide here.

**Note from openui-lang ticket 14 (2026-10-07):** arrays, objects and expressions are now shared too, so one subtree can appear in many places of a result. A walk that copies the tree into Kotlin objects re-expands every shared subtree; that's bounded (a result is capped at about 8 MiB of JSON), but a handle-based walk wouldn't need to. The binding should also surface `depth_limited` and `size_limited`, so the app can say "this screen was cut short".

**2026-10-15 (from viewer ticket 05; measured in the Mac viewer, M3 Pro, release build):** handing the whole parse result across as JSON after every chunk costs almost nothing next to drawing it. Per update, median:

| | `bench/dashboard` (3.4 KB program, 8 KB result) | largest corpus program (28.9 KB program, 65 KB result) |
|---|---:|---:|
| core parse (steady streaming, 1–2 chunks) | 0.04 ms | 0.07 ms |
| crossing: serialize to JSON and copy into Swift | 0.04 ms | 0.14–0.22 ms |
| decoding into the UI's model | 0.07 ms | 0.37–0.46 ms |
| redraw, including about 17 ms waiting for the next frame | 26–30 ms | 44 ms |

Even when one update carries hundreds of chunks at once, crossing stays under 1 ms.

**Recommendation for Android:** send the whole result as JSON after each chunk, and drop the handle-walking and only-what-changed designs unless Android's own measurement says otherwise. Things to measure there:
- **The JNA crossing and the JSON decoding on ART.** They'll cost more than on the Mac, but they would have to grow a hundredfold to matter.
- **Layout.** What did matter in SwiftUI was laying out the whole screen, off-screen parts included, on every update. Making column stacks lazy cut the largest program's worst redraw from 267 to 48 ms. In Compose, a story screen's top level should be a `LazyColumn` too.
- **Recomposition.** Decode into immutable data classes, so Compose can skip unchanged subtrees. In SwiftUI, skipping unchanged views didn't help because layout dominated; Compose may differ, so measure it.
- **The partial flag.** Keep it on elements, which the UI already uses for its "still arriving" marker.

