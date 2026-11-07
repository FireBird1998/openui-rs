# openui-rs

`openui-lang` is the OpenUI Lang language layer in Rust: parsing (one-shot and streaming), prop validation, system prompt generation, edit-mode merging, and turning element trees back into source. It's a port of `@openuidev/lang-core`, the TypeScript reference, and its output is the same byte for byte, after every streamed chunk, so a screen looks the same on every platform. It's written to run on phones: no input can panic it, overflow its stack or exhaust its memory.

The repository also holds the viewer, a Mac app that draws programs natively through the crate (see below), and will hold Dynamic News, an Android app whose story screens are written by an LLM and drawn natively with this crate. That part is still being designed ([map](.scratch/dynamic-news/map.md)).

**Status:** conforms to lang-core 1.1.0 (OpenUI Lang 1.1), at commit [`47b9cae0d3b26a8c9d7514d5887a6d6608801ac9`](https://github.com/thesysdev/openui/tree/47b9cae0d3b26a8c9d7514d5887a6d6608801ac9/packages/lang-core): 637 parse cases at every streaming step and at each stream's end, 61 prompts, 3,235 merges, 605 serializations and 378 enriched error lists, plus 100,000 fuzzed programs against 1.1.0 with every difference a documented divergence. Nothing is pending. The local tags `lang-core-1.0.1` and `lang-core-0.3.0` are the last states that conformed to those versions, which aren't kept up.

- [CONFORMANCE.md](CONFORMANCE.md): what's checked and how, fuzzing, what it found, and the few deliberate divergences
- [conformance/QUIRKS.md](conformance/QUIRKS.md): reference behaviour that looks wrong but is reproduced, with draft upstream reports
- [BENCHMARKS.md](BENCHMARKS.md): streaming speed against the reference, and why it grows linearly

## Using it

```rust
use std::sync::Arc;
use openui_lang::{Library, StreamParser, parse};

// The JSON Schema that `library.toJSONSchema()` produces on the web side.
let schema = serde_json::json!({"$defs": {
    "Stack": {"properties": {"children": {"type": "array"}}},
    "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
}});
let library = Arc::new(Library::from_json_schema(&schema));

// One-shot.
let result = parse("root = Stack([title])\ntitle = Text(\"Hello\")", &library, None);
assert_eq!(result.root.unwrap().type_name, "Stack");

// Streaming: a full result after every chunk, the unfinished parts marked partial.
let mut stream = StreamParser::new(library, None);
for chunk in ["root = Stack([ti", "tle])\ntitle = Te", "xt(\"Hello\")"] {
    let result = stream.push(chunk);
    println!("{}", result.to_json()); // the reference's JSON, byte for byte
}
```

`cargo doc -p openui-lang --open` has the rest, with examples for prompts, merging and serializing. The core grows its stack on demand and its results free their trees without recursion, so it runs, and its results drop, on any thread. Its limits on program size, result size, depth and the work evaluating state defaults may take are one `Limits` value with documented defaults, which an embedder may raise or lower.

The command-line tool runs each part on files:

```bash
cargo run -p openui-lang -- parse conformance/schemas/text.json conformance/cases/hello-text.oui
cargo run -p openui-lang -- stream conformance/schemas/hand.json conformance/cases/hand/71-expression-forms.oui 7
cargo run -p openui-lang -- tokens conformance/cases/hand/67-lexer-number-forms.oui
cargo run -p openui-lang -- prompt conformance/prompts/library-tools-specs.json
cargo run --release -p openui-lang -- bench conformance/imported/schemas/bench.json conformance/imported/cases/bench/*.oui
```

## Checking it

```bash
scripts/check.sh
```

That's formatting, clippy, the docs, and every test, doc examples included (this README's example too) and the binding's, then a build on Rust 1.88.0 when that toolchain is installed. It needs only Rust 1.88 or later: no Bun, no network. The reference's outputs are committed as goldens, so the tests compare against them directly. [TESTING.md](TESTING.md) explains what each test catches, how to add a case and how to read a failure.

## Working on it

These need [Bun](https://bun.sh) 1.4, which runs the vendored reference in `reference/`:

```bash
scripts/regen-golden.sh                  # recompute every golden from the reference
scripts/import-corpus.sh [<checkout>]    # re-import upstream programs and test calls, then regenerate
scripts/update-reference.sh <commit>     # vendor another upstream commit (full hash), then import
scripts/fuzz.sh --seed 1 --count 10000   # differential fuzzing; saves shrunk mismatches as cases
scripts/fuzz.sh --edits --seed 1 --count 10000   # the same for merging and serializing
scripts/bench.sh                         # streaming benchmarks, core against reference
scripts/hunt.sh --minutes 60             # fuzzing with a fresh seed and an upstream check; report in findings/
```

When a test fails, it prints the case, the streaming step and a diff against the reference (fetched live when Bun is there). Two switches rewrite generated files: `UPDATE_PENDING=1 cargo test -p openui-lang` rewrites the pending lists in `conformance/pending/` to the cases that fail now, and `UPDATE_REPORT=1 cargo test --test report` refreshes the counts in CONFORMANCE.md. Coverage-guided fuzz targets live in `fuzz/` and need nightly Rust and `cargo-fuzz`. `scripts/coverage.sh` measures line coverage over the offline suite and fails if a line no test runs isn't explained in [UNREACHED.md](crates/openui-lang/UNREACHED.md); it needs `cargo-llvm-cov`.

## The viewer

A Mac app that draws programs natively, beside their parse result: a playground for the crate, and a rehearsal for Dynamic News's binding ([spec](.scratch/viewer/spec.md)). It calls the crate through `crates/openui-ffi`, a UniFFI binding that Kotlin will use too. So far it opens any corpus case (searchable, grouped by source) or a program you paste or open, with a component library, and plays it as a stream: chunk size, speed, stepping, a slider to any step. It draws the most used components of upstream's example library natively (text, stacks, cards, buttons, tables, forms, line, bar and pie charts, tabs, tags and callouts; File > Open Component Gallery shows them all) and anything else as a labeled box, at the window's width or a phone's or tablet's, and shows the program text with what's been received, and has an inspector with the element tree, what's unresolved, orphaned or invalid, the limit flags, and, for corpus cases, whether each step matches the reference's golden. Its settings can point it at another conformance folder. The toolbar's sparkles button asks Claude for a screen and draws the answer as it streams. That needs your Anthropic API key, saved through Settings into the macOS Keychain, and a budget cap, also set in Settings. A finished stream can be saved as a recording, JSON with every delta and its arrival time. Recordings show up at the top of the sidebar, open cut where the deltas arrived, and replay with their original timing. "Follow Up" asks Claude to change the screen: it answers in edit mode with a patch, which the viewer merges into the program as it streams, showing the patch beside the merged program. It needs macOS 27 and Xcode.

```bash
scripts/viewer.sh run      # build the binding and the app, then run it
scripts/check-viewer.sh    # build with warnings as errors, then the Swift tests
```

Or run `scripts/viewer.sh` once, then open `viewer/Package.swift` in Xcode and press ⌘R; the shared scheme rebuilds the binding first (its log is `target/viewer-xcode-prebuild.log`). The app bundle lands in `target/viewer/`.

A scripted run plays a case (or `gallery`, or a program file's absolute path) in the real window with nobody at the screen, then prints how the UI kept up and quits:

```bash
VIEWER_PLAY=synthetic/dashboard-x8 VIEWER_CHUNK=1 target/viewer/release/Viewer.app/Contents/MacOS/Viewer
```

`scripts/viewer-timing.sh` runs a fixed set of them, streaming steadily and under stress, and reports where each update's time goes. Sending the whole result as JSON after every chunk costs well under a millisecond even for the largest program; drawing is what costs, about 25–45 ms per update while streaming (see `.scratch/viewer/issues/05-where-the-redraw-time-goes.md`).

## Layout

| Path | What's there |
|---|---|
| `crates/openui-lang/` | the crate: `src/` (one module per stage), `tests/`, the CLI in `src/bin/`, the differential fuzzer in `examples/fuzz/` |
| `crates/openui-ffi/` | the UniFFI binding the viewer (and later Dynamic News) calls, with the bindings generator |
| `viewer/` | the viewer, a Swift package; `scripts/viewer.sh` generates its `Frameworks/` and `Sources/OpenUIFFI/` |
| `conformance/` | the corpus (`manifest.json` and friends list every case), the goldens, quirks, and the hand-written and generated edit and prompt cases |
| `reference/` | the vendored reference (`lang-core/`, untouched) and the Bun scripts that run it: goldens, fuzz server, benchmarks, corpus import |
| `scripts/` | the commands above |
| `fuzz/` | cargo-fuzz targets |
| `.scratch/` | the earlier specs, their tickets with notes on every decision, and the Dynamic News map; new work is tracked in [GitHub Issues](https://github.com/FireBird1998/openui-rs/issues). The glossary is [GLOSSARY.md](GLOSSARY.md) |
| `docs/` | decision records (`adr/`) and how agents use this repository (`agents/`) |

## License

MIT, see [LICENSE](LICENSE). The binding depends on UniFFI, which is under MPL-2.0. The vendored reference in `reference/lang-core/` and the programs imported from upstream in `conformance/imported/` are Thesys Inc.'s, also under MIT; their notice is kept beside them.
