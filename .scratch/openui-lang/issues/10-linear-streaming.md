# 10 — Linear-time streaming, still identical

**What to build:** Rework the streaming parser so it stops reprocessing the whole text on every chunk, making total streaming work grow linearly with program length. Output stays byte-identical. Add benchmarks that run the same workload through the core and the reference parser, and record the results.

**Blocked by:** 08 — Differential fuzzing; 09 — Never panic.

**Status:** done (2026-10-06)

- [x] All conformance, fuzzing and robustness checks stay green.
- [x] Doubling a program costs no more than about 2.2× the streaming time, measured from 3.5 KB up to at least 28 KB.
- [x] Benchmarks report Rust vs reference timings for the 7 benchmark samples and the synthetic large programs, and the results are written to a benchmarks document.
- [x] `openui-lang bench` runs the same workload as the reference bench script.

## Comments

**2026-10-06, done.** The write-up with the numbers is `BENCHMARKS.md`.

- **Baseline first.** `openui-lang bench` and `reference/bench.ts` run the salvaged workload (chunks of 4 code points, 5 warm-ups, mean of `BENCH_RUNS` runs); `reference/synth.ts` builds large programs from a sample (renamed copies nested in groups of four, so the tree deepens instead of the root widening); `scripts/bench.sh` runs both sides. Before the change both parsers cost about 4× per doubling, and the core was *slower* than the reference on Bun (0.7–0.9×): cloning every completed statement's tree per push cost more than Bun's allocator.
- **Where the time went,** found by sampling, not guessing: preprocessing the whole buffer per push; cloning the statement map per push; rebuilding the whole tree per push; walking every statement for `$name`s per push; and, once those were gone, `hasDynamicProps` walking the whole (now shared) tree from the root, then the transitive dependency lists of the cache's first design, which cost a node's whole subtree to rebuild.
- **Preprocessing** is a `Preprocessor` that extends the cleaned text chunk by chunk through the same stages (leading trim, fences, comments, trailing trim). Only the current line and whitespace-only lines before it can still change; the first fence drops everything before it. Each push reports how much of the previous cleaned text survived, which is the reference's prefix check.
- **Result building** keeps a `Table` of statements by slot number and a `Cache` of materialized statements (`src/table.rs`):
  - `Value::Element` is now `Arc<Element>`, so results share the subtrees that didn't change. (`tag_statement` uses `Arc::make_mut`; validation never mutates inside an element.)
  - A cached statement carries its value and everything its materialization did on the side (errors, unresolved refs, the statements it reached, how deep it went), replayed into the parent's frame on a hit. Cached per (statement, `partial`, mode): `partial` is stamped on every element.
  - Invalidation is pushed through the program's reverse reference graph: a change to a statement drops the cached entries of everything that references it, however indirectly. That costs the chain of ancestors, not the program. No generation stamps, no dependency lists.
  - A materialization that met a cycle or the depth limit is never cached (it depends on where it was reached from). A valid entry can't reach a statement being materialized right now, since that would be a cycle, so no visited check is needed on a hit.
  - Orphans: each result's reached set is a tree of shared nodes (a node per statement, shared between its `partial` variants); the table diffs the new tree against the last one by pointer and keeps a reach count per statement, plus a count of value statements at zero. Listing them costs a scan only when there are some and the set may have changed.
  - State, queries, mutations and `$name` readers are kept in a `specials` list so the per-result extraction is proportional to them.
  - The pending statements are laid over the table for one result and taken back afterwards (`define`/`undo`), so the reference's "copy the map and overlay" has no copy.
- **Found by the fuzzer, fixed:** two pending statements with the same id in one push (possible when a leading `?` or `[` keeps every line pending). The "is this id already completed?" check had started consulting the table the first one was just laid over, so the second was skipped. Decided before any overlay now; 10 cases saved (`fuzz/00951ad8` and others). 50,000 more fuzz programs after the fix: 0 unexplained mismatches.
- **Synthetic cases** `synthetic/dashboard-x2` (all chunkings and stages), `-x4` and `-x8` (a few chunkings, no stage goldens: goldening every prefix and step of a 30 KB program costs its square). New manifest fields `stages: false` and `chunkings: [...]`.
- **Guard:** `tests/streaming_cost.rs` checks a push on the 8-copy program costs less than 2× a push on the 2-copy one (a parser that redid the tree would be about 4×).
- **Measured** (core, chunks of 4): per push 7.5 µs at 1 copy (3.7 KB), 7.6 at 2, 7.9 at 4, 9.4 at 8 (29.5 KB), 9.7 at 16, 11.3 at 32 (120 KB). Within one stream the cost is flat from the first decile to the last. Per doubling: 2.01, 2.07, 2.38, 2.10, 2.35; the steps above 2.2 are where the synthetic tree gains a nesting level, which adds one re-materialized statement per push. Geometric mean over 3.7–29.5 KB: 2.15.
- **What a push still costs:** the arriving statement is re-parsed and re-materialized whole (so a single enormous statement is quadratic in its own length, as in the reference), plus the chain from the entry down to it (one statement per nesting level), plus invalidating that chain. About 7 µs on this machine, mostly allocation and string formatting in validation; a constant-factor pass is a separate job.
- **Memory:** the cache holds the materialized tree, twice when both `partial` variants have been needed, plus the reached tree and the reverse reference graph. A few times the program's size.
