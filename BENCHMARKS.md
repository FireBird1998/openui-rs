# Benchmarks

How fast the core streams, measured against the reference parser on the same
workload, and why the numbers look the way they do. The table was measured on
2026-10-06 on an idle Apple Silicon laptop (Rust 1.97, release profile; Bun
1.4), reference at commit `bba1d251`. The reference hasn't changed since. The
core's synthetic-program times were measured again on 2026-10-07, after
tickets 12 and 14 changed its internals, and matched within noise (below).

Benchmarks need an idle machine: a run on 2026-10-07 while a virtual machine
was using most of a core came out 4–7× slower on both sides, unevenly, and was
discarded.

## Workload

`scripts/bench.sh` runs both sides:

- `openui-lang bench <schema> <program>...` (the core) and `bun reference/bench.ts`
  (the reference) feed each program to a fresh streaming parser in chunks of 4
  code points, take 5 warm-up runs, and report the mean of `BENCH_RUNS` runs
  (20 by default; 5 for the table below, since the reference takes half a
  minute per run on the largest program).
- The programs are the 7 benchmark samples (real GPT output, from upstream
  `benchmarks/samples`) and synthetic programs built by `reference/synth.ts`:
  1 to 32 renamed copies of the `dashboard` sample, nested under one root in
  groups of four, so the tree gets deeper as the program grows rather than the
  root wider. One copy is 3.7 KB; 32 copies are 120 KB.

The time is the whole stream: every push, and the result it returns. Nothing
is serialized; `to_json` is the caller's choice.

## Results

| Program | Code points | Pushes | Reference (ms) | Core (ms) | Speedup |
|---|---:|---:|---:|---:|---:|
| simple-table | 433 | 109 | 1.29 | 0.60 | 2.2× |
| chart-with-data | 795 | 199 | 1.92 | 1.25 | 1.5× |
| contact-form | 1117 | 280 | 3.30 | 1.75 | 1.9× |
| settings-panel | 2367 | 592 | 14.93 | 3.99 | 3.7× |
| dashboard | 3478 | 870 | 33.75 | 5.95 | 5.7× |
| e-commerce-product | 4165 | 1042 | 52.25 | 12.00 | 4.4× |
| pricing-page | 4761 | 1191 | 51.12 | 9.42 | 5.4× |
| dashboard-x1 | 3708 | 927 | 41.42 | 7.21 | 5.7× |
| dashboard-x2 | 7386 | 1847 | 148.15 | 14.94 | 9.9× |
| dashboard-x4 | 14742 | 3686 | 553.82 | 31.81 | 17.4× |
| dashboard-x8 | 29538 | 7385 | 2424.66 | 72.22 | 33.6× |
| dashboard-x16 | 59718 | 14930 | 9273.54 | 153.92 | 60.2× |
| dashboard-x32 | 120356 | 30089 | 38388.04 | 349.76 | 109.8× |

Per push, the core costs 7.5 µs on the 1-copy program and 11.3 µs on the
32-copy one; within one stream the cost is the same at the start and at the
end. Per doubling of the synthetic program: 2.01, 2.07, 2.38, 2.10, 2.35. The
steps above 2.2 are the sizes where the synthetic tree gains a nesting level
(4 to 8 copies, 16 to 32), which adds one statement to what every push has to
redo. Over the range the ticket asked for, 3.7 KB to 30 KB, the geometric mean
is 2.15 per doubling.

The reference costs about 4× per doubling (the whole text is preprocessed and
the whole tree rebuilt on every push), and so did the core before this work:
on the 8-copy program it took 2.8 s, slower than the reference's 2.0 s,
because cloning every completed statement's tree per push cost more than
Bun's allocator.

## What a push costs now

- The arriving statement is re-parsed and re-materialized whole, as in the
  reference: a single enormous statement is still quadratic in its own length.
- The chain of statements from the entry down to it is re-materialized: one
  statement per nesting level, each a handful of lookups and props.
- The cached statements the chain references are taken as they were, and so
  is everything below them. Results share those subtrees: elements are
  reference-counted.
- Invalidating the chain, extending the cleaned text, and updating the reach
  counts cost the chain and the chunk, not the program.
- About 7 µs of fixed cost per push on this machine, mostly allocation and
  string formatting in validation. A constant-factor pass is separate work.

What is still proportional to the program: listing orphaned statements, only
when there are some and the set may have changed; and a state initial value
is re-materialized on every push (they're small).

What is never cached: a statement whose materialization met a reference cycle
or the 256-level depth limit. Such programs cost what they cost in the
reference.

## Memory

The cache holds the materialized tree, twice when both `partial` variants
have been needed (the flag is stamped on every element), plus the reached
tree and the reverse reference graph: a few times the program's size. A
result keeps alive the subtrees it shares with the cache for as long as the
caller holds it.

Since ticket 14 every inlined statement is shared, not copied: elements,
arrays, objects and expressions in a result are reference-counted. A
program built to double its result at every line (each statement
referencing the one before twice) used 1.8 GB at 20 lines; it now stays
under 40 MB at any length, and the result is cut at `MAX_RESULT_SIZE`
(8 MiB of JSON). The streaming numbers above didn't change: re-measured after
the change, a push still costs 8 to 12 µs and doubling a program 2.1×.

## Checking it stays that way

- `tests/streaming_cost.rs` fails if a push on the 8-copy program costs 2× or
  more a push on the 2-copy program (a parser that redid the tree is about 4×).
- `synthetic/dashboard-x2`, `-x4` and `-x8` are conformance cases, so the
  incremental machinery is checked byte for byte against the reference at
  every step under several chunkings and resyncs.
