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

**2026-10-06 (from openui-lang ticket 09):** the core needs up to 384 KB of stack at its nesting limit when optimized (corrected in ticket 12 from 256 KB), but up to 1 MB unoptimized. The Android build must compile it with opt-level 1 or more even for debug APKs (set in the binding crate's profile), and call it from threads with at least 512 KB of stack. Check what thread UniFFI calls land on, and its stack size.

**2026-10-15 (from viewer ticket 05; the Mac viewer is the rehearsal):** what calling the core through UniFFI took on macOS, and what carries over.

**Build:**
- **The binding crate.** `crates/openui-ffi` wraps the core with UniFFI 0.32.2 (MPL-2.0), using proc-macros with no UDL file. It builds as `lib`, `staticlib` and `cdylib`. UniFFI's default features are off for the library the app links.
- **The bindings generator** is a binary in the same crate behind a `bindgen` feature. It's built in its own target directory, so its features never leak into the linked library, and it reads the interface from the built `cdylib`. Nothing is installed globally.
- **For Android,** the same crate should generate Kotlin with `--language kotlin` and build one `cdylib` per ABI. UniFFI's Kotlin bindings call through JNA: an extra dependency, and a per-call cost worth measuring.
- **Optimization.** Both crates build at opt-level 1 even in debug, as the core's stack notes require. That's set per package in the workspace profile. Release uses LTO.
- **A macOS-only gotcha:** an Xcode update can reset its license, and then nothing links until it's accepted again.

**Threads and stack:**
- **No threads of its own.** UniFFI calls are synchronous and run on the calling thread, and the core spawns no threads.
- **What the viewer does:** it makes every call, and decodes the JSON, on one dedicated thread with a 1 MB stack. A debug check stops any call from another thread. The core needs at most 384 KB optimized, and nothing came close in any run.
- **For Kotlin:** use a dedicated thread made with an explicit stack size, `Thread(null, runnable, "openui-core", 1 shl 20)`, rather than a coroutine dispatcher's threads, whose stack size isn't ours to choose.
- **Sharing.** `ComponentLibrary` and `StreamSession` are both safe to share. The session serializes its own calls with a mutex.

**What crossing costs:** see ticket 06. In short, it's negligible next to drawing.

