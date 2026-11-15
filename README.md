# openui-rs

`openui-lang` is the OpenUI Lang language layer in Rust: parsing (one-shot and streaming), prop validation, system prompt generation, edit-mode merging, and turning element trees back into source. It's a port of `@openuidev/lang-core`, the TypeScript reference, and its output is the same byte for byte, after every streamed chunk, so a screen looks the same on every platform. It's written to run on phones: no input can panic it, overflow its stack or exhaust its memory.

The repository also holds OpenUINative, a Swift package that draws programs as native SwiftUI views; the viewer, a Mac app built on it that is a development tool for the crate (both below); and will hold Dynamic News, an Android app whose story screens are written by an LLM and drawn natively with this crate. That part is still being designed ([map](.scratch/dynamic-news/map.md)).

**Status:** conforms to lang-core 1.1.0 (OpenUI Lang 1.1), at commit [`47b9cae0d3b26a8c9d7514d5887a6d6608801ac9`](https://github.com/thesysdev/openui/tree/47b9cae0d3b26a8c9d7514d5887a6d6608801ac9/packages/lang-core): 637 parse cases at every streaming step and at each stream's end, 61 prompts, 3,235 merges, 605 serializations and 378 enriched error lists, plus 100,000 fuzzed programs against 1.1.0 with every difference a documented divergence. Nothing is pending. The local tags `lang-core-1.0.1` and `lang-core-0.3.0` are the last states that conformed to those versions, which aren't kept up.

- [CONFORMANCE.md](CONFORMANCE.md): what's checked and how, fuzzing, what it found, and the few deliberate divergences
- [conformance/QUIRKS.md](conformance/QUIRKS.md): reference behaviour that looks wrong but is reproduced, with draft upstream reports
- [BENCHMARKS.md](BENCHMARKS.md): streaming speed against the reference, and why it grows linearly

## Using it

```rust
# #[global_allocator]
# static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
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
// A push re-reads the statement still arriving, so push once a frame, not once a token.
let mut stream = StreamParser::new(library, None);
for chunk in ["root = Stack([ti", "tle])\ntitle = Te", "xt(\"Hello\")"] {
    let result = stream.push(chunk);
    println!("{}", result.to_json()); // the reference's JSON, byte for byte
}
```

`cargo doc -p openui-lang --open` has the rest, with examples for prompts, merging and serializing. The core grows its stack on demand and its results free their trees without recursion, so it runs, and its results drop, on any thread. Its limits on program size, result size, depth and the work evaluating state defaults may take are one `Limits` value with documented defaults, which an embedder may raise or lower. The result-size limit scales with the program (by default the larger of 64 KiB and 64 times its size, never above 8 MiB), so a result's amplification is bounded and serializing every push of a stream has a known cost. So does the work limit (by default the larger of a million steps and 1,000 per byte, never above 64 million), so a small program can't keep it busy. Every entry point takes it: parsing and streaming, merging, converting back to source, reading JSON and generating prompts.

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

## OpenUINative, the Swift package

`swift/OpenUINative/` is for Mac apps that want to draw OpenUI Lang: the crate's Swift binding (through `crates/openui-ffi`, a UniFFI binding that Kotlin will use too), the parse result as Swift values, and `ElementView`, which draws an element tree as SwiftUI. It draws the most used components of upstream's example library itself. An app adds its own components, wraps what's drawn and chooses what an undrawable element looks like through `ElementDrawing`, set in the environment.

A `LiveProgram` keeps a program live in a runtime session. Show it each parse result (each step of a stream) and draw it with `ElementView(live:)`: expressions draw as their values, state the app sets with `set(_:to:)` redraws what reads it, built-in buttons run their actions, and built-in inputs and selects write to state through their binding markers (or to form fields when unbound). Every host event (a message for the assistant, a URL to open, a custom action, a query or mutation to run) goes to the one `onHostEvent` handler. The app's own components get the evaluated props, with binding markers and action plans as `PropValue.binding` and `.plan`; they write through `write(_:_:)`, run a plan with `run(_:of:message:form:)`, and draw their children with `ElementView(element:)`, since an evaluated element knows its own path. A click or write acts only if the element (or marker) it came from is from the current evaluation, so it runs exactly what was on screen or nothing, and says so in `failure`. An element, binding marker or plan is recognised only in exactly the shape the runtime writes it; other data, such as an object with a `steps` key, stays data. Only a prop whose evaluation failed (listed in `errors`) still draws as greyed source. Which props are bindings comes from the library's prompt spec or a list. Calls go to the session's worker thread and wait for it on the main actor: a write or click re-evaluates and decodes the tree before it returns. With everything built for debugging that took about 2 ms for upstream's dashboard example and 15 ms for the largest corpus program (8 times that example), so typing stays within a frame; with the Swift built for release, 0.6 and 4 ms. Library functions evaluate to null through the binding for now ([#56](https://github.com/FireBird1998/openui-rs/issues/56)).

The same UniFFI binding exposes [runtime sessions](docs/runtime-binding.md) for Swift and Kotlin: state, evaluated trees, binding markers, forms, action handles and mutation continuations. The session owns a worker thread; tree and event JSON come directly from the Rust runtime. The runtime Store enforces the session's result-size ceiling on individual values and total state.


```bash
scripts/swift-binding.sh   # build the binding into the package (generated, not committed)
```

Then depend on the package by path.

## The viewer, a development tool

A Mac app that draws programs natively, beside their store and parse result: a playground for the crate, and a rehearsal for Dynamic News's binding ([spec](.scratch/viewer/spec.md)). It is built on OpenUINative and lives in `tools/viewer/`; nobody using the crate or the package needs it. So far it opens any corpus case (searchable, grouped by source) or a program you paste or open, with a component library, and plays it as a stream: chunk size, speed, stepping, a slider to any step. It draws the most used components of upstream's example library natively (text, stacks, cards, buttons, tables, forms, line, bar and pie charts, tabs, tags and callouts; File > Open Component Gallery shows them all) and anything else as a labeled box, at the window's width or a phone's or tablet's, and shows the program text with what's been received. Every step is evaluated in a runtime session, as an app would: the panel beside the drawing shows the store (each value editable as JSON, and the drawing redraws), the runtime errors and the host events the program has sent, and buttons and inputs in the drawing act on the store. It has an inspector with the element tree, what's unresolved, orphaned or invalid, the limit flags, and, for corpus cases, whether each step matches the reference's golden. Its settings can point it at another conformance folder. The toolbar's sparkles button asks Claude for a screen and draws the answer as it streams. That needs your Anthropic API key, saved through Settings into the macOS Keychain, and a budget cap, also set in Settings. A finished stream can be saved as a recording, JSON with every delta and its arrival time. Recordings show up at the top of the sidebar, open cut where the deltas arrived, and replay with their original timing. "Follow Up" asks Claude to change the screen: it answers in edit mode with a patch, which the viewer merges into the program as it streams, showing the patch beside the merged program. It needs macOS 27 and Xcode.

```bash
scripts/viewer.sh run      # build the binding and the app, then run it
scripts/check-viewer.sh    # the package and the viewer: warnings as errors, then their Swift tests
```

Or run `scripts/viewer.sh` once, then open `tools/viewer/Package.swift` in Xcode and press ⌘R; the shared scheme rebuilds the binding first (its log is `target/viewer-xcode-prebuild.log`). The app bundle lands in `target/viewer/`.

A scripted run plays a case (or `gallery`, or a program file's absolute path) in the real window with nobody at the screen, then prints how the UI kept up and quits:

```bash
VIEWER_PLAY=synthetic/dashboard-x8 VIEWER_CHUNK=1 target/viewer/release/Viewer.app/Contents/MacOS/Viewer
```

For ticket 11's 150 real streams, the maintainer can launch a sequential batch after saving the API key in the viewer's Settings (Keychain) and checking the budget cap and recordings folder:

```sh
scripts/viewer.sh build
VIEWER_RECORD="$PWD/viewer/Requests/recorded-streams.json" target/viewer/debug/Viewer.app/Contents/MacOS/Viewer
# Optional inclusive range; keep the full JSON file:
VIEWER_RECORD="$PWD/viewer/Requests/recorded-streams.json" VIEWER_RECORD_FROM=51 VIEWER_RECORD_TO=150 target/viewer/debug/Viewer.app/Contents/MacOS/Viewer
scripts/import-recordings.sh # pass the folder as an argument if Settings uses a custom one
```

Run the same batch command again to resume. Matching completed batch recordings in the viewer's recordings folder are skipped; manual recordings have no batch identity. Follow-ups use their named fresh request's program from this run or its earlier batch recording, including parents outside the range, and are skipped if unavailable. The batch uses the sheet's react-ui prompt, low effort and each row's Sonnet/Haiku model, prints results with tokens and estimated spend, then quits. It stops before the next conservative cost reservation would exceed the viewer's budget cap, or after three consecutive failures. Failed/partial streams are kept for inspection and retried; output-token-limit recordings count as completed. Reservations for interrupted requests with unknown usage remain in the budget. The JSON is generated from [the request tables](.scratch/testing/recorded-streams.md) with `python3 tools/viewer/Requests/sync-recorded-streams.py`; `scripts/check-viewer.sh` checks agreement and tests batch behavior with fake streams, without Keychain or API access. Do not combine `VIEWER_RECORD` and `VIEWER_PLAY`; recording takes precedence.

`scripts/viewer-timing.sh` runs a fixed set of them, streaming steadily and under stress, and reports where each update's time goes. Sending the whole result as JSON after every chunk costs well under a millisecond even for the largest program; drawing is what costs, about 25–45 ms per update while streaming (see `.scratch/viewer/issues/05-where-the-redraw-time-goes.md`).

## In the browser

`examples/next-compare/` is a Next.js page that streams one program into two copies of upstream's React renderer and react-ui library: one parses with upstream's lang-core, the other with this core compiled to WebAssembly (`crates/openui-wasm`). Only the parser differs. Under each pane it shows parser time, React commit time and long tasks. `npm run compare` there checks without a browser that both give the same result at every update. See [its README](examples/next-compare/README.md).

```sh
scripts/wasm.sh
cd examples/next-compare && npm ci && npm run compare && npm run build && npm start
```

## Layout

| Path | What's there |
|---|---|
| `crates/openui-lang/` | the crate: `src/` (one module per stage), `tests/`, the CLI in `src/bin/`, the differential fuzzer in `examples/fuzz/` |
| `crates/openui-ffi/` | the UniFFI binding that OpenUINative (and later Dynamic News) calls, with the bindings generator |
| `swift/OpenUINative/` | the Swift package: binding, parse result, live programs and native drawing; `scripts/swift-binding.sh` generates its `Frameworks/` and `Sources/OpenUIFFI/` |
| `tools/viewer/` | the viewer, a development tool built on that package |
| `conformance/` | the corpus (`manifest.json` and friends list every case), the goldens, quirks, and the hand-written and generated edit and prompt cases |
| `reference/` | the vendored reference (`lang-core/`, untouched) and the Bun scripts that run it: goldens, fuzz server, benchmarks, corpus import |
| `crates/openui-wasm/` | the WebAssembly binding (`scripts/wasm.sh` builds it into `target/wasm-pkg/`) |
| `examples/next-compare/` | the side-by-side page above |
| `scripts/` | the commands above |
| `fuzz/` | cargo-fuzz targets |
| `.scratch/` | the earlier specs, their tickets with notes on every decision, and the Dynamic News map; new work is tracked in [GitHub Issues](https://github.com/FireBird1998/openui-rs/issues). The glossary is [GLOSSARY.md](GLOSSARY.md) |
| `docs/` | decision records (`adr/`) and how agents use this repository (`agents/`) |

## License

MIT, see [LICENSE](LICENSE). The binding depends on UniFFI, which is under MPL-2.0. The vendored reference in `reference/lang-core/` and the programs imported from upstream in `conformance/imported/` are Thesys Inc.'s, also under MIT; their notice is kept beside them.
