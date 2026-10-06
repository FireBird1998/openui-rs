# openui-rs

A Rust implementation of the OpenUI Lang language layer (the `openui-lang` crate), and Dynamic News, an Android app built on it.

**Status:** ticket 01 of 13 done. One program parses end to end and conforms with the reference parser, both one-shot and while streaming.

## How correctness works

The TypeScript parser in `@openuidev/lang-core` is the reference. It's vendored under `reference/` at the commit recorded in [`reference/UPSTREAM`](reference/UPSTREAM). For every case in `conformance/`, the Rust parse result has to serialize to exactly the JSON the reference produced: one-shot, and after every push of a streaming parse.

```bash
scripts/check.sh                       # fmt, clippy, tests (Rust only, no Bun needed)
scripts/regen-golden.sh                # re-run the reference over the corpus (needs Bun)
scripts/update-reference.sh <commit>   # vendor a different upstream commit (full hash)
cargo run -p openui-lang -- parse conformance/schemas/text.json conformance/cases/hello-text.oui
```

## Map

- Glossary: [CONTEXT.md](CONTEXT.md)
- Parser spec and tickets: [.scratch/openui-lang/](.scratch/openui-lang/spec.md)
- Dynamic News map (parked until the core can stream): [.scratch/dynamic-news/map.md](.scratch/dynamic-news/map.md)
