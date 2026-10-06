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
