# 09 — Never panic, and streaming ends where parsing ends

**What to build:** Rust-only robustness checks that don't need the reference parser. Parsing and streaming arbitrary text under arbitrary chunkings never panic. For any text, the final streaming result equals the one-shot result, except where the reference parser itself diverges, which goes on the quirks list.

**Blocked by:** 07 — Streaming parse matches.

**Status:** done (2026-10-06)

- [x] Property tests over arbitrary strings and chunkings run in the normal test suite with fixed seeds.
- [x] A longer coverage-guided fuzz target exists for parse and streaming parse.
- [x] Any panic found is fixed and saved as a regression case.
- [x] The stream-equals-parse invariant holds on the whole corpus.

## Comments

**From ticket 03:** proptest is already a dev-dependency, and `tests/robustness.rs` covers tokenize. Extend that file rather than starting a new one.

**From ticket 04:** the expression parser recurses, so very deep nesting (`((((...` thousands deep) would overflow the Rust stack. That's an abort, not a catchable panic, and in an app it would crash the process. The reference throws `RangeError` at some depth, so there's no result to conform to. Decide here: a nesting cap that degrades gracefully, or an explicit stack, and add a case for it.

**From ticket 08:** 130,000 fuzz programs, including truncated and garbled ones, gave 0 core panics. The generator caps nesting at depth 3, so the deep-nesting question here still stands. The fuzz driver is a good harness for a deep-nesting run once that's decided.

**2026-10-06, done.**
- **Depth limit (you chose 256 on 2026-10-06):** `pub const MAX_DEPTH = 256`.
  - The expression parser stops descending past it. A too-deep branch becomes `null` and consumes one token so every loop progresses. It also caps left-nested chains (`a.b.c...`, `1 + 1 + ...`), which are parsed iteratively but build trees as deep as the chain is long.
  - Materialization has its own depth counter, which also counts reference chains (`a = b`, `b = c`, ...), and returns null past it.
  - `ParseResult::depth_limited` reports it; it isn't part of the JSON.
  - The reference copes with about 8,000 levels on Bun and throws a RangeError by 16,000. So programs nesting 257 to about 8,000 deep are a documented divergence (noted in the spec).
- `tests/depth.rs`: 10 shapes (10k nested parens, arrays, objects and component calls; 100k unary, member and addition chains; a 10k ternary chain; a 10k-statement reference chain), parsed one-shot and streamed on a 512 KB thread stack. Red first: SIGABRT stack overflow. All pass now, and the flag is set; a 40-deep ordinary program isn't flagged.
- **Stack budget**, measured with `DEPTH_STACK_KB`: optimized builds (opt-level 1 or 3) need at most 256 KB at full depth (**corrected in ticket 12:** 384 KB; the reference chain needed that in release builds already, and the measurement here only covered the test build); unoptimized needed up to 2 MB for nested components. After refactoring materialize (small recursive frames, heavy work in `#[inline(never)]` helpers) it's at most 1 MB. So the crate builds at opt-level 1 in dev and test too, and the Android binding must compile it optimized and call it on threads with at least 512 KB of stack (noted on the Dynamic News map).
- `tests/robustness.rs`: fixed-seed proptest (`RngSeed::Fixed`, no persistence files). Tokenize and preprocess take 2,000 cases each. Parse takes 1,500 cases each of any text and program-like text: one-shot (with and without a root name), pushes at random boundaries, `set` rewind, edit and reset, then `result`. All results are serialized. A planted failure confirmed the generators reach structure, after a reformat had silently dropped the new pieces.
- **stream end = one-shot:** `streaming_ends_where_parsing_ends` holds on all 286 cases, except the 12 where the reference diverges too: quirk 4 (redefinitions counted twice), new quirk 5 (a mismatched closing bracket leaves one-shot `incomplete`/`partial`), and the by-design #1127 behavior for an unclosed trailing redefinition.
- `fuzz/`: cargo-fuzz targets `parse` and `stream` (the first byte picks the chunk size and whether to resync), plus `openui.dict`. Excluded from the workspace; type-checks on stable. **Not run yet:** it needs nightly Rust and `cargo install cargo-fuzz`, which aren't installed. Ask before installing.
- No panic has been found anywhere: proptest, depth tests, and 130k differential fuzz programs.
