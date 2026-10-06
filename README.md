# openui-rs

A Rust implementation of the OpenUI Lang language layer (the `openui-lang` crate), and Dynamic News, an Android app built on it.

**Status:** tickets 01–11 of 13 done. Prompt generation matches the reference on 44 specs (every mode, tools, groups, cloud config, and its error cases). Streaming cost now grows linearly with the program: a push costs the arriving statement plus the chain above it, not the whole tree, and doubling a program costs about 2.15× (3.7 KB to 30 KB); see [BENCHMARKS.md](BENCHMARKS.md). No input can panic it or overflow the stack: nesting stops at 256 levels (`depth_limited`). Differential fuzzing (180,000 generated and mangled programs) finds no unexplained mismatches. **All 299 corpus cases conform**: one-shot, at every step of eight push chunkings, and through two resync sequences (`set` with growing, rewound and edited text). The cases are hand-written edge cases, every program the reference test suites parse, the benchmark samples, programs from the upstream docs and examples, fuzz finds, and synthetic programs up to 30 KB. Stages (preprocessing, tokens, statements, expression trees) also match on every prefix.

## How correctness works

The TypeScript parser in `@openuidev/lang-core` is the reference. It's vendored under `reference/` at the commit recorded in [`reference/UPSTREAM`](reference/UPSTREAM). For every case in `conformance/`, the Rust parse result has to serialize to exactly the JSON the reference produced: one-shot, and after every push of a streaming parse, under eight chunkings (sizes 1, 2, 3, 4, 7, 16 and two seeded random ones).

```bash
scripts/check.sh                       # fmt, clippy, tests (Rust only, no Bun needed)
scripts/regen-golden.sh                # re-run the reference over the corpus (needs Bun)
scripts/import-corpus.sh [<checkout>]  # re-import upstream programs at the pinned commit, then regen
scripts/update-reference.sh <commit>   # vendor a different upstream commit (full hash)
UPDATE_PENDING=1 cargo test --test conformance   # rewrite pending.txt to the cases failing now
scripts/fuzz.sh --seed 1 --count 10000           # differential fuzzing; saves shrunk mismatches as cases
scripts/bench.sh                                 # streaming benchmarks, core against the reference (needs Bun)
cargo run -p openui-lang -- parse conformance/schemas/text.json conformance/cases/hello-text.oui
cargo run -p openui-lang -- stream conformance/schemas/hand.json conformance/cases/hand/71-expression-forms.oui 7
cargo run -p openui-lang -- tokens conformance/cases/hand/67-lexer-number-forms.oui
cargo run -p openui-lang -- prompt conformance/prompts/library-tools-specs.json
cargo run --release -p openui-lang -- bench conformance/imported/schemas/bench.json conformance/imported/cases/bench/*.oui
```

Reference behaviour that looks wrong, but that the core matches anyway, is recorded in [`conformance/QUIRKS.md`](conformance/QUIRKS.md). How fast it is, and how that was measured, is in [`BENCHMARKS.md`](BENCHMARKS.md).

## Map

- Glossary: [CONTEXT.md](CONTEXT.md)
- Parser spec and tickets: [.scratch/openui-lang/](.scratch/openui-lang/spec.md)
- Dynamic News map (parked until the parser tickets are done): [.scratch/dynamic-news/map.md](.scratch/dynamic-news/map.md)
