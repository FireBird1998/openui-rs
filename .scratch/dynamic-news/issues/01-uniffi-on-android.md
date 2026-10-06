# 01 — How well does UniFFI work on Android today?

Type: research
Status: open
Blocked by: none

## Question

Can Kotlin on Android call the core through UniFFI comfortably today? Find out:
- the build setup (cross-compiling for Android ABIs, Gradle integration)
- how much a Rust library adds to the APK
- how a streaming parse result, a tree rebuilt after every chunk, gets to Kotlin
- the cost of crossing that boundary per chunk
- any alternatives worth comparing

## Comments

**2026-10-06 (from openui-lang ticket 02):** upstream's blog post "Rewriting our Rust WASM Parser in TypeScript" measured the cost of crossing a language boundary with a whole parse result on every chunk. The boundary dominated, not the parsing: about 20–61 µs per call in the browser, and returning native objects (serde-wasm-bindgen) was slower than a JSON string. The same question applies to Kotlin over UniFFI/JNI. Measure the per-chunk crossing cost, not just parse speed.

**2026-10-06 (from openui-lang ticket 09):** the core needs about 256 KB of stack at its nesting limit when optimized, but up to 1 MB unoptimized. The Android build must compile it with opt-level 1 or more even for debug APKs (set in the binding crate's profile), and call it from threads with at least 512 KB of stack. Check what thread UniFFI calls land on, and its stack size.
