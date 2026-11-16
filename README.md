# openui-rs

`openui-lang` is the OpenUI Lang language layer in Rust: parsing (one-shot and streaming), prop validation, system prompt generation, edit-mode merging, and turning element trees back into source. It's a port of `@openuidev/lang-core`, the TypeScript reference, and its output is the same byte for byte, after every streamed chunk, so a screen looks the same on every platform. It's written to run on phones: no input can panic it, overflow its stack or exhaust its memory.

The repository also holds OpenUINative, a Swift package that draws programs as native SwiftUI views; the viewer, a Mac app built on it that is a development tool for the crate (both below); and will hold Dynamic News, an Android app whose story screens are written by an LLM and drawn natively with this crate. That part is still being designed ([map](.scratch/dynamic-news/map.md)).

**Status:** conforms to lang-core 1.1.0 (OpenUI Lang 1.1), at commit [`47b9cae0d3b26a8c9d7514d5887a6d6608801ac9`](https://github.com/thesysdev/openui/tree/47b9cae0d3b26a8c9d7514d5887a6d6608801ac9/packages/lang-core): 972 parse cases at every streaming step and at each stream's end, 65 prompts, 3,269 merges, 939 serializations and 510 enriched error lists, plus 100,000 fuzzed programs against 1.1.0 with every difference a documented divergence. Nothing is pending. The local tags `lang-core-1.0.1` and `lang-core-0.3.0` are the last states that conformed to those versions, which aren't kept up.

- [CHANGELOG.md](CHANGELOG.md): Rust API and behavior changes before release
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

Parser conformance includes element-shaped dynamic flags, library names that shadow built-in calls, and inherited schema property names. Late-arriving aliases preserve diagnostic suppression when depth cuts are replayed. Object-valued prototype assignments remain a pinned divergence with main-compatible completed results. Alias-chain cost regressions count traversal steps and allocations at raised limits; see [TESTING.md](TESTING.md#parser-materialisation-regressions).

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

Runtime differential fuzzing (`scripts/fuzz.sh --runtime`) generates programs,
state and interactions, independently saves/reloads Rust state, and checks
small-limit cuts with upstream controls. Children run under a process-tree
memory/deadline supervisor with parent-death cleanup. See
[TESTING.md](TESTING.md#runtime-differential-fuzzing) for replay and flag rules.

The Rust runtime uses one run-wide work allowance across action resumptions.
The core measures supplied program/action/reference input; resupply can raise
only the largest combined input floor. Prepared inputs cache setup work; `PreparedReferenceInput` and `SuppliedInput`
replace the removed `PreparedInput` API (see [CHANGELOG.md](CHANGELOG.md)).
Shared programs retain only a weak cache identity. Snapshot JSON omits undefined
state entries like the bindings. Serialized room is not a general peak-memory
bound; see [CONFORMANCE.md](CONFORMANCE.md) and [ADR 0008](docs/adr/0008-share-runtime-substitution-bodies.md).

## OpenUINative, the Swift package

`swift/OpenUINative/` is for Mac apps that want to draw OpenUI Lang: the crate's Swift binding (through `crates/openui-ffi`, a UniFFI binding that Kotlin will use too), the parse result as Swift values, and `ElementView`, which draws an element tree as SwiftUI. It draws the most used components of upstream's example library itself. An app adds its own components, wraps what's drawn and chooses what an undrawable element looks like through `ElementDrawing`, set in the environment.

A `LiveProgram` keeps a program live in a runtime session. Show it each parse result (each step of a stream) and draw it with `ElementView(live:)`: expressions draw as their values, state the app sets with `set(_:to:)` redraws what reads it, built-in buttons run their actions, and built-in inputs and selects write to state through their binding markers (or to form fields when unbound). Every host event (a message for the assistant, a URL to open, a custom action, a query or mutation to run) goes to the one `onHostEvent` handler. The app's own components get the evaluated props, with binding markers and action plans as `PropValue.binding` and `.plan`; they write through `write(_:_:)`, run a plan with `run(_:of:message:form:)`, and draw their children with `ElementView(element:)`, since an evaluated element knows its own path. A click or write acts only while what it was drawn from is still what the tree holds: an element (or marker) of the current evaluation, or, for a second click or keystroke that arrived before the redraw, one exactly like the element now at its place, in a form of the same name. So it runs exactly what was on screen or nothing, and says so in `failure`. A table's action cell runs like a button with no action. A primary button in a form first checks the rules of the form's built-in inputs and selects, with upstream's rules (`required`, `email`, `url`, `numeric`, `min`, `max`, `minLength`, `maxLength`, `pattern`), runs nothing while one fails, and the field shows why; rules on other components are not checked yet. While the app sets `isStreaming` (a response is still arriving), buttons and inputs are disabled and `run` refuses. An element, binding marker or plan is recognised only in exactly the shape the runtime writes it; other data, such as an object with a `steps` key, stays data. Only a prop whose evaluation failed (listed in `errors`) still draws as greyed source. Which props are bindings comes from the library's prompt spec or a list. Calls go to the session's worker thread and wait for it on the main actor: a write or click re-evaluates and decodes the tree before it returns. Built for release, that's about 0.2 ms for upstream's dashboard example and 1.5 ms for the largest corpus program (8 times that example), about 22 µs per KB of the evaluated tree, so typing stays well within a frame up to a few hundred KB (see [TESTING.md](TESTING.md#the-runtime-sessions-cost)). Host library functions can be registered through `RuntimeSession.registerFunction` (see the [callback contract](docs/runtime-binding.md#host-library-functions)).

The same UniFFI binding exposes [runtime sessions](docs/runtime-binding.md) for Swift and Kotlin: state, evaluated trees, binding markers, forms, action handles and mutation continuations. The session owns a worker thread; tree and event JSON come directly from the Rust runtime. The runtime Store enforces the session's result-size ceiling on individual values and total state.


```bash
scripts/swift-binding.sh   # build the binding into the package (generated, not committed)
```

Then depend on the package by path.

## The viewer, a development tool

A Mac app that draws programs natively, beside their store and parse result: a playground for the crate, and a rehearsal for Dynamic News's binding ([spec](.scratch/viewer/spec.md)). It is built on OpenUINative and lives in `tools/viewer/`; nobody using the crate or the package needs it. So far it opens any corpus case (searchable, grouped by source) or a program you paste or open, with a component library, and plays it as a stream: chunk size, speed, stepping, a slider to any step. It draws the most used components of upstream's example library natively (text, stacks, cards, buttons, tables, forms, line, bar and pie charts, tabs, tags and callouts; File > Open Component Gallery shows them all) and anything else as a labeled box, at the window's width or a phone's or tablet's, and shows the program text with what's been received. Every step is evaluated in a runtime session, as an app would: the panel beside the drawing shows the store (each value editable as JSON, and the drawing redraws; a value that changes while you edit it is shown beside your edit), the runtime errors and the newest 200 host events the program has sent, and buttons and inputs in the drawing act on the store once the shown step has every chunk (before that the response counts as still streaming). It has an inspector with the element tree the drawing shows (evaluated, so `@Each` rows and a ternary's branch appear with the drawing's paths), what's unresolved, orphaned or invalid, the limit flags, and, for corpus cases, whether each step matches the reference's golden. Its settings can point it at another conformance folder. The toolbar's sparkles button asks Claude for a screen and draws the answer as it streams. That needs your Anthropic API key, saved through Settings into the macOS Keychain, and a budget cap, also set in Settings. A finished stream can be saved as a recording, JSON with every delta and its arrival time. Recordings show up at the top of the sidebar, open cut where the deltas arrived, and replay with their original timing. "Follow Up" asks Claude to change the screen: it answers in edit mode with a patch, which the viewer merges into the program as it streams, showing the patch beside the merged program. It needs macOS 27 and Xcode.

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

Runtime sessions support declared host library functions through
`registerFunction`: a UniFFI `LibraryFunction` callback in Swift/Kotlin, or a
JavaScript function in the guarded wasm package. Arguments/results cross as
bounded JSON text; the Rust runtime owns validation, defaults and diagnostics.
Same-session callback reentry refuses without effects. Native callbacks run on
the session worker without a timeout; other threads queue. Hosts must return
without waiting for a thread that needs the same session. JavaScript callbacks
are synchronous too. See the [callback contract](docs/runtime-binding.md#host-library-functions)
for reentry, host responsibilities and action failure semantics.

Reference data is prepared once and reused in both bindings. An optional
`maxActionPathBytes` limit permits smaller retained handle paths without changing
default behavior. Element-shaped data can run built-in state writes; finished
custom-step JSON never grants custom authority (ADR 0004).

Runtime state and raw/prepared query receivers check depth only for the parts
consumed; shallow member and length reads do not walk a deep sibling. Whole-value
output still checks depth. Changed state values earn their own action-run input
credit across mutation resumes; unchanged consecutive scalar reads earn it once.
`Store::snapshot_json` omits undefined entries, matching the bindings. On persisted
re-initialization an omitted declared name takes its default (previous Rust
exports wrote null), and an undeclared undefined name disappears. See
[CONFORMANCE.md](CONFORMANCE.md) for the pinned compatibility changes.

Typed and data `@Each` templates share one substitution traversal and retain
existing work and room prices. Object keys matching the loop variable stay
literal in both forms. Explicit `BindingProps` limits recover the full escaped
name charge when a pair is removed; repeated insert/remove cycles retain no
charge for removed names. Same-session core `with_scope` re-entry shares
the suspended owner's work and cut flags; `reentry_accounting` pins its charges.

Runtime differential fuzzing is part of the existing fuzzer:
`scripts/fuzz.sh --runtime --seconds 60 --seed 40 --findings target/runtime-hunt --no-save`.
It compares generated state and interaction sequences with the pinned reference
runtime, in children bounded for time, memory and output. Cached Bun dependencies
are required; nothing is installed. See [TESTING.md](TESTING.md#runtime-differential-fuzzing)
for replay, triage and coverage, and the maintainer-started
`scripts/mutants.sh runtime --jobs 2` mutation run.
