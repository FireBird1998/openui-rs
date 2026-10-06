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
