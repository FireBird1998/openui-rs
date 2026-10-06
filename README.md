# openui-rs

A Rust implementation of the OpenUI Lang language layer (the `openui-lang` crate), and Dynamic News, an Android app built on it.

**Status:** tickets 01–05 of 13 done. Preprocessing (fences, comments, prose), tokens, statements and expression trees match the reference on every prefix of every program. The corpus has 278 cases (hand-written edge cases, every program the reference test suites parse, the benchmark samples, and programs from the upstream docs and examples). 37 conform end to end so far; the other 241 are listed in [`conformance/pending.txt`](conformance/pending.txt) and drop off as later tickets land.

## How correctness works

The TypeScript parser in `@openuidev/lang-core` is the reference. It's vendored under `reference/` at the commit recorded in [`reference/UPSTREAM`](reference/UPSTREAM). For every case in `conformance/`, the Rust parse result has to serialize to exactly the JSON the reference produced: one-shot, and after every push of a streaming parse, under eight chunkings (sizes 1, 2, 3, 4, 7, 16 and two seeded random ones).

```bash
scripts/check.sh                       # fmt, clippy, tests (Rust only, no Bun needed)
scripts/regen-golden.sh                # re-run the reference over the corpus (needs Bun)
scripts/import-corpus.sh [<checkout>]  # re-import upstream programs at the pinned commit, then regen
scripts/update-reference.sh <commit>   # vendor a different upstream commit (full hash)
UPDATE_PENDING=1 cargo test --test conformance   # rewrite pending.txt to the cases failing now
cargo run -p openui-lang -- parse conformance/schemas/text.json conformance/cases/hello-text.oui
cargo run -p openui-lang -- tokens conformance/cases/hand/67-lexer-number-forms.oui
```

Reference behaviour that looks wrong, but that the core matches anyway, is recorded in [`conformance/QUIRKS.md`](conformance/QUIRKS.md).

## Map

- Glossary: [CONTEXT.md](CONTEXT.md)
- Parser spec and tickets: [.scratch/openui-lang/](.scratch/openui-lang/spec.md)
- Dynamic News map (parked until the core can stream): [.scratch/dynamic-news/map.md](.scratch/dynamic-news/map.md)
