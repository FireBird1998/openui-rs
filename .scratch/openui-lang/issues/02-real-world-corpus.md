# 02 — Real-world corpus and the chunking matrix

**What to build:** Grow the conformance corpus from one program to everything we can get:
- the salvaged hand-written corpus with its validation schema
- the 7 benchmark samples and their schema
- inputs lifted from the reference test suites (parser, materialize validation)
- OpenUI Lang examples from the upstream docs and examples folders

Every case runs one-shot, at chunk sizes 1, 2, 3, 4, 7 and 16, and with seeded random sizes. Cases the core can't handle yet go on the pending list, so the suite stays green and prints "N of M cases conformant".

**Blocked by:** 01 — Walking skeleton.

**Status:** done (2026-10-06)

- [x] Every corpus case has committed golden output from the reference parser for every chunking.
- [x] The test run prints the count of conformant cases.
- [x] A pending case that starts passing fails the suite until it's removed from the pending list.
- [x] A case that used to pass and now fails fails the suite.
- [x] Each case records where it came from (hand-written, benchmark, reference test, docs).

## Comments

**2026-10-06, done.** The corpus is 265 cases:
- 66 hand-written in `conformance/manifest.json`: hello-text plus the 65 salvaged edge cases, now named (such as `hand/21-number-literal-forms`). Case 65 now really contains a BOM, a non-breaking space and a trailing U+0085.
- 199 imported in `conformance/imported.json`, each with `source` and `origin`:
  - 74 reference-test: every program the parser, materialize-validation and serialize suites parse. They're recorded live by running those suites under `bun test --preload reference/capture-tests.ts`, which wraps the parser module. Gotcha: Bun's `mock.module` rewrites the live namespace too, so the original exports must be copied first. ParamMaps are converted to the JSON Schema `compileSchema` reads back.
  - 7 benchmark: real GPT output, with the 53-component react-ui schema.
  - 72 docs and 46 examples: code fences in MD/MDX, plus template and double-quoted string literals in TS/JS. They're kept when they look like a program (a statement line and a component call), deduplicated, and parsed against the benchmark schema.
- `scripts/import-corpus.sh [checkout]` rebuilds all of it at the pinned commit, then regenerates goldens. Verified deterministic and identical between the local-checkout and GitHub modes.
- Chunkings are 1, 2, 3, 4, 7, 16, `random-1` and `random-2` (sizes 1–16, Mulberry32 seeded by FNV-1a of `name:chunking`; the sizes are stored in the golden). Fingerprints are now 8 hex digits, without the length. Goldens total about 3 MB.
- Pending: the test isolates each case with `catch_unwind`. It fails on a pending case that now passes, a stale pending entry, or a regression. Bun lookups for diffs are capped at 5 per run. `UPDATE_PENDING=1` rewrites the list. Starting point: **26 of 265 conform, 239 pending.** The suite runs in about 1 s.

Notes for later tickets are on 07; the Rust/WASM blog post is noted on the spec and the Dynamic News map.
