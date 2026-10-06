# 09 — Never panic, and streaming ends where parsing ends

**What to build:** Rust-only robustness checks that don't need the reference parser. Parsing and streaming arbitrary text under arbitrary chunkings never panic. For any text, the final streaming result equals the one-shot result, except where the reference parser itself diverges, which goes on the quirks list.

**Blocked by:** 07 — Streaming parse matches.

**Status:** ready-for-agent

- [ ] Property tests over arbitrary strings and chunkings run in the normal test suite with fixed seeds.
- [ ] A longer coverage-guided fuzz target exists for parse and streaming parse.
- [ ] Any panic found is fixed and saved as a regression case.
- [ ] The stream-equals-parse invariant holds on the whole corpus.

## Comments

**From ticket 03:** proptest is already a dev-dependency, and `tests/robustness.rs` covers tokenize. Extend that file rather than starting a new one.

**From ticket 04:** the expression parser recurses, so very deep nesting (`((((...` thousands deep) would overflow the Rust stack. That's an abort, not a catchable panic, and in an app it would crash the process. The reference throws `RangeError` at some depth, so there's no result to conform to. Decide here: a nesting cap that degrades gracefully, or an explicit stack, and add a case for it.

**From ticket 08:** 130,000 fuzz programs, including truncated and garbled ones, gave 0 core panics. The generator caps nesting at depth 3, so the deep-nesting question here still stands. The fuzz driver is a good harness for a deep-nesting run once that's decided.
