# Spec: openui-lang, the OpenUI Lang parser in Rust

**Status:** ready-for-agent (approved 2026-10-06)

## Problem Statement

OpenUI Lang can only be parsed in JavaScript today, through `@openuidev/lang-core`. Anything outside JavaScript, such as a native Android app, a Python backend or a command-line tool, has to embed a JavaScript engine or write its own parser, and a hand-written parser drifts from the real one. Drift already happens inside the TypeScript code: upstream issue thesysdev/openui#1127 was the streaming parser and the one-shot parser disagreeing about redefined statements.

I want a Rust implementation that behaves exactly like the reference parser, so it can be the shared engine for platforms outside JavaScript, starting with Dynamic News on Android.

The reference streaming parser also redoes work on every chunk. On the pricing-page benchmark sample (4.8 KB), streaming costs about 49 ms in total against 0.2 ms for one one-shot parse, and total time grows about 3–4× each time the program doubles in size (measured with Bun at commit bba1d251). That's fine on the web today. A new implementation should do better, provided the output stays identical.

## Solution

`openui-lang` is a Rust library crate (the **core**) that implements the language layer of OpenUI Lang:

- one-shot parse
- streaming parse
- loading the component library from JSON Schema
- prop validation
- system prompt generation

"Correct" means **conformance** with the reference parser at a pinned upstream commit. For the same program, component library and chunking, the core's parse result serializes to exactly the same JSON as the reference parser's, at every streaming step. A differential test harness checks this on a growing corpus of real and hand-written programs and on generated ones.

The work happens in two phases: make it right (a faithful port that conforms), then make it fast (a streaming parse whose total cost grows linearly, still conformant). A small command-line tool exposes parse, stream, prompt and bench for debugging.

## User Stories

1. As a Rust developer, I want to parse a complete program against a component library and get the element tree, so that I can draw it with native components.
2. As a Rust developer, I want the same parse result the web renderer gets, so that a story screen looks the same on every platform.
3. As a Rust developer, I want to feed a program chunk by chunk as the LLM streams it and get a usable parse result after every chunk, so that I can render progressively.
4. As a Rust developer, I want elements that are still arriving to be flagged as partial, so that I can show loading states for them.
5. As a Rust developer, I want to load the component library from the JSON Schema that `library.toJSONSchema()` produces, so that components are defined once for web and native.
6. As a Rust developer, I want positional arguments mapped to named props in the library's order, with schema defaults filled in, exactly as the reference parser does.
7. As a Rust developer, I want validation errors (missing required, null required, unknown component, inline reserved, excess arguments, type mismatch) with component, path, statement id and message, so that I can report or repair bad output the same way the web does.
8. As a Rust developer, I want unresolved references and orphaned statements listed in the parse result, so that I can detect broken model output.
9. As a Rust developer, I want state declarations, query statements and mutation statements extracted exactly as the reference parser does, so that a future runtime can use them even though evaluating them is out of scope.
10. As a Rust developer, I want a program wrapped in markdown fences or surrounded by prose to parse, as it does in the reference parser.
11. As a Rust developer, I want comments ignored the way the reference parser ignores them, including comment-like text inside strings.
12. As a Rust developer, I want redefined statement ids to behave as they do in the reference parser: the last definition wins, and while streaming the latest one renders as it arrives.
13. As a Rust developer, I want to resync a streaming parse with the full text so far, and have it start over when the new text doesn't continue the old one, so that reconnects work.
14. As a Rust developer, I want to choose the root statement name, so that programs whose entry point isn't `root` still work.
15. As a Rust developer, I want parsing never to panic on any input, including garbage, truncated text and odd chunk boundaries, so that a bad model response can't crash my app.
16. As a Rust developer, I want the total cost of a streaming parse to grow linearly with program size, so that long story screens stay cheap on low-end phones.
17. As a Rust developer, I want parse results serializable to the same JSON the reference parser produces, so that I can pass them across language boundaries and compare them with web output.
18. As a Rust developer, I want a system prompt generated from my component library, tools, examples and rules that is identical to the reference generator's output, so that the model gets the same instructions on every platform.
19. As a Rust developer, I want few dependencies, so that the core stays small enough to embed in a mobile app.
20. As a future Kotlin binding author, I want the public API made of plain owned types with no borrowed lifetimes crossing it, so that wrapping it for Android is straightforward.
21. As the maintainer, I want one command that runs the whole conformance suite without Bun or Node installed, so that tests are fast and self-contained.
22. As the maintainer, I want one command that regenerates the expected outputs from the reference parser, so that moving the pinned commit is cheap.
23. As the maintainer, I want the pinned upstream commit recorded next to the vendored reference, so that I always know what "correct" means.
24. As the maintainer, I want every mismatch to report the case, the streaming step, the chunk sizes and a readable diff between Rust and reference JSON, so that I can fix it quickly.
25. As the maintainer, I want a visible count of conformant cases out of total cases, so that I can see porting progress.
26. As the maintainer, I want the corpus to include the reference test suite's inputs, the benchmark's real LLM outputs and the documentation's examples, so that coverage reflects real use.
27. As the maintainer, I want every program run under many chunkings (one-shot, sizes 1, 2, 3, 4, 7 and 16, and seeded random sizes), so that chunk-boundary bugs surface.
28. As the maintainer, I want randomly generated and mutated programs compared against the reference parser, so that I find cases nobody wrote by hand.
29. As the maintainer, I want every mismatch found by fuzzing saved as a permanent corpus case, so that it never comes back.
30. As the maintainer, I want behaviour that looks wrong in the reference parser recorded as a quirk with an upstream report, while the core still matches it, so that conformance stays strict and OpenUI benefits from what I find.
31. As the maintainer, I want benchmarks that run the same workload through the core and the reference parser, so that performance claims are measured, not guessed.
32. As the maintainer, I want a command-line tool to parse a file, stream it with a chosen chunk size, print a generated prompt and run benchmarks, so that I can debug without writing code.
33. As an OpenUI maintainer, I want to see the pinned commit, the conformance report and the list of quirks, so that I can judge whether the core is trustworthy enough to adopt.
34. As an agent implementing a ticket, I want stage-by-stage comparisons (tokens, statements, expressions, preprocessing) as well as whole-result comparisons, so that I can tell where behaviour diverges.

## Implementation Decisions

### Reference and conformance
- **Reference parser:** the `@openuidev/lang-core` source at thesysdev/openui commit `bba1d251` (lang-core 0.3.0). Only that package's source is vendored into the repo, with the upstream commit recorded beside it and a script to refresh it. It is not a git submodule, which keeps it small, reproducible and easy to update.
- **Contract:** the core's parse result, serialized, is byte-identical to `JSON.stringify` of the reference parse result. That holds for one-shot parse and for every push of a streaming parse. Both sides chunk the same text at the same points, measured in Unicode code points.
- **Quirks:** where the reference parser seems wrong, the core matches it anyway and the case goes on the quirks list with an upstream report. Fixing a quirk happens upstream first; the core follows when the pinned commit moves.
- **JSON details that must match JavaScript:**
  - number formatting: shortest round-trip, integers without `.0`, JavaScript's exponent rules, `-0` written as `0`, non-finite numbers as `null`
  - object keys in insertion order
  - absent fields omitted rather than written as null
  - `JSON.stringify` string escaping
- **Input:** programs are valid Unicode, and a future binding must not split a character across chunks.
- **Lone surrogates** (decided in ticket 03): decoded strings can still hold one, for example when a stream stops between `\uD83D` and `\uDE00`. The reference parser keeps it as a lone UTF-16 surrogate and writes it as `\ud83d`. The core carries each lone surrogate as one of the last 2048 code points of plane 16 (U+10F800–U+10FFFF) and writes it back out the same way. The cost: a program that contains those private-use code points itself can't conform. A Kotlin binding, whose strings are UTF-16, should map them back to real lone surrogates.
- **JavaScript object semantics** (decided in ticket 06): where they reach the output, the core reproduces them. That covers array-index keys ordered first, `in` seeing `Object.prototype` names, `__proto__` assignment, and objects shaped like AST nodes or elements. Cases that would need a full JS object model (an element-shaped object literal as the root, prototype mutation, shared mutable schema defaults) are documented divergences, not emulated.
- **Telemetry:** the reference runner always runs with OpenUI telemetry disabled. The core has no telemetry.

### Modules
Each module mirrors one stage of the reference pipeline, with Rust idioms inside:
- **Lexer:** program text to tokens.
- **Statements:** splits tokens into statements, and auto-closes an incomplete trailing statement.
- **Expressions:** tokens to an expression tree covering every node kind the reference parser has (component call, string, number, boolean, null, array, object, reference, placeholder, state reference, runtime reference, binary, unary, ternary, member, index, assignment).
- **Preprocessing:** strips markdown fences and comments.
- **Component library:** compiled from JSON Schema into each component's ordered params, with required flags, defaults and schema fragments.
- **Builtin catalogue:** builtin and action names, signatures and descriptions, stored as data. The parser needs it to classify calls and the prompt generator needs it for docs. Evaluating builtins is out of scope.
- **Materialize and validate:** resolves references, maps positional arguments to named props, fills defaults, validates props, sets the partial and dynamic-props flags, tracks unresolved and orphaned statements, and extracts state, query and mutation statements.
- **Parser:** one-shot parse to a parse result.
- **Streaming parser:** push a chunk, resync with the full text, read the current result.
- **Prompt generation:** system prompt from a prompt spec in the same JSON shape the reference parser accepts.
- **Later:** statement merging for edit mode, JSON-to-OpenUI serialization, and error hints.

### Public API (Rust)
- Load a component library from a JSON Schema value.
- Parse a program against a library with an optional root name; returns a parse result. Parsing never returns an error type, because problems are data inside the parse result. A panic is a bug.
- A streaming parser created from a shared, reference-counted library and an optional root name, offering push, resync and current result. Each call returns a full parse result. Whether the core should hand callers only what changed per chunk is decided on the Dynamic News map, not here.
- Parse results and elements are plain owned types that serialize to the reference JSON shape, plus a convenience method that returns that JSON.
- The stage functions the reference package exports (tokenize, split, auto-close, parse expression, strip fences) are public too, for stage-level conformance and for consumers that walk partial programs.
- Prompt generation takes a prompt spec deserialized from the reference JSON shape and returns a string.

### Performance
- **Phase 1:** port the reference algorithms faithfully, including the streaming parser's rescanning, until the core conforms.
- **Phase 2:** make streaming incremental, so that total work grows linearly with program length and doubling a program costs no more than about 2.2× the streaming time. Output must stay byte-identical. Measured on the 7 benchmark samples plus synthetic large programs, against the reference parser on Bun.
- **Done (2026-10-06), see `BENCHMARKS.md`:** a push costs the arriving statement plus the chain of statements above it, not the program. The cleaned text is extended chunk by chunk; materialized statements are cached and invalidated through the reverse reference graph; results share unchanged subtrees, so `Value::Element` holds an `Arc<Element>`. Doubling costs 2.15× on average from 3.7 KB to 30 KB (2.0–2.4 per step, higher where the tree gains a level). Cycles and depth-limited branches are never cached, so pathological programs cost what they cost in the reference.

### Depth limit (decided 2026-10-06)
- Expressions and reference chains nest at most `MAX_DEPTH` = 256 levels. Deeper parts become null and `ParseResult::depth_limited` is set (not part of the JSON). The reference copes up to roughly 8,000 levels, then throws, so 257 to about 8,000 is a documented divergence that no real model output comes near.
- At full depth, optimized builds need at most 256 KB of stack and unoptimized ones at most 1 MB. Embedders must build the core optimized (opt-level 1 or more) and call it from threads with at least 512 KB of stack.

### Crate and tooling
- Rust 2024 edition on stable (1.97 on this machine).
- One library crate with a command-line binary, inside a Cargo workspace that leaves room for bindings and the Android app later.
- Runtime dependencies: an order-preserving JSON library, an insertion-ordered map, and a JavaScript-compatible float formatter. Dev dependencies: property testing, benchmarking and diff printing.
- One check command runs formatting, lints and tests.

## Testing Decisions

- **A good test** compares the output of a public function against the reference parser, never private internals. The reference parser is the oracle, so expected values aren't written by hand.
- **Seams:**
  1. **Primary seam:** the parse result JSON from one-shot parse and from every streaming push, compared with the reference. This is the contract.
  2. **Stage seams:** tokenize, split, auto-close, parse expression and strip fences, compared with the same functions exported by the reference package. These are public API on both sides, not implementation details. They exist to localize failures.
  3. **Prompt seam:** the generated prompt string for a prompt spec.
- **Golden conformance (self-contained):** the reference runner, using Bun, turns corpus cases into expected outputs that are committed to the repo: a hash for every streaming step plus the full JSON of each final step. `cargo test` compares against them without Bun. On a mismatch it prints the case, step, chunk sizes and a diff, plus the command that fetches the full reference JSON for that step. This design comes from the Oct 3 salvage.
- **Pending list:** cases the core doesn't support yet are listed as expected failures, so the suite stays green while showing progress. The suite fails if a pending case starts passing (remove it from the list) or a passing case regresses.
- **Chunking matrix:** every case runs one-shot, at sizes 1, 2, 3, 4, 7 and 16, and with seeded random sizes. Targeted cases put boundaries inside strings, escapes, numbers, fences and comments.
- **Corpus sources:**
  - the salvaged hand-written corpus (validation, Query/Mutation, Actions, `$state`, `@Each`)
  - inputs from the reference test suites (parser, materialize validation, serialize, prompt)
  - the 7 benchmark samples, which are real GPT output
  - examples from the OpenUI docs and examples folders
  - saved regressions
- **Differential fuzzing (opt-in, needs Bun):** generates random programs from the grammar and mutates them (truncate, delete or insert characters, swap tokens, wrap in fences or prose), then runs them through both parsers via one long-lived reference process. Mismatches are shrunk and saved as corpus cases.
- **Rust-only robustness:**
  - never panic on arbitrary strings or chunkings
  - the final streaming result equals the one-shot result for the same text, wherever the reference parser also holds that; any divergence goes on the quirks list
- **Benchmarks:** the core and the reference parser on the same workloads, with results recorded in a benchmarks document.
- **Prior art:** the reference package's own test suites, the upstream benchmarks folder, and the salvaged per-step-hash harness.

## Out of Scope

- The runtime: expression evaluation, state store, query manager, MCP and tool calls.
- Defining component libraries with Zod; the core accepts JSON Schema only.
- Telemetry and the reference package's cloud client.
- Bindings (Kotlin, Python, WebAssembly). The API is designed with them in mind, but they aren't built here.
- The Dynamic News app; it has its own map.
- Fixing quirks in the reference parser, and getting OpenUI to adopt the core.
- Publishing to crates.io or GitHub.
- Input that isn't valid Unicode.

## Further Notes

- **Salvage:** the folder next to this spec holds what survived of the Oct 3 attempt: the per-step-hash reference runner, the hand-written corpus and its validation schema, the TypeScript bench script, the command-line entry point, and a `lib.rs` sketch of the module list. The module source files were lost, so the code starts fresh; the harness design and corpus carry over.
- **Decisions made without discussion, open to review:**
  - pinning `bba1d251`
  - byte-identical JSON as the contract
  - vendoring rather than a submodule
  - matching quirks rather than fixing them
  - the Rust 2024 edition
  - a shared, reference-counted library in the streaming parser
  - two-phase performance
- **Prior art:** upstream's blog post "Rewriting our Rust WASM Parser in TypeScript" (docs/content/blog/rust-wasm-parser.mdx, 2026-03-13). Thesys first built this parser in Rust compiled to WebAssembly and replaced it with TypeScript, which ran about 3× faster in the browser. Copying strings and results across the WASM boundary dominated (about 20–61 µs per call), and serde-wasm-bindgen was 9–29% slower still. That's why this core targets platforms outside JavaScript, not the web. The post also describes the statement-level caching that the reference streaming parser uses today. The old Rust code isn't in the upstream history.
- **Glossary:** `CONTEXT.md` at the repo root.
