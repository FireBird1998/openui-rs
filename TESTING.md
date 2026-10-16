# Testing

The core is correct when it does exactly what the reference does: `@openuidev/lang-core` at the pinned commit (see [CONFORMANCE.md](CONFORMANCE.md)). Most tests compare the core's output with the reference's, saved as goldens, so they need only Rust: no Bun and no network. The rest check what conformance can't: no input crashes the core, deep or huge input is cut off safely, streaming stays linear, and the binding hands on exactly what the core produces.

This guide covers what each test catches, how to run it, how to add a case, and how to read a failure. The testing phase ([spec](.scratch/testing/spec.md)) adds sections as its tickets land: hunts, coverage, mutation testing, coverage-guided fuzzing, invariants, recorded streams and Android.

## Running the tests

```bash
scripts/check.sh
```

The check runs formatting, clippy, the docs and every Rust test, including the doc examples and this repository's README example. It then builds the workspace on Rust 1.88.0, the oldest version the crates claim. That build is skipped, with a note, when the toolchain isn't installed; the check never installs one. It takes a minute or two.

```bash
scripts/check-viewer.sh                                   # the viewer's Swift tests (needs Xcode)
cargo test -p openui-lang --test conformance              # one test file
cargo test -p openui-lang --test conformance streaming_   # tests whose names match
```

## What each test catches

| Test | Checks | Catches |
|---|---|---|
| `conformance` | Every corpus case (532), one-shot, after every streaming push and at each stream's end, under each of the corpus's chunkings, against the reference's goldens. Also that a stream, once ended, gives what one-shot parsing gives, apart from programs where the reference itself doesn't. | Any difference from the reference in parsing or streaming |
| `stages` | Tokens, statements and expressions, and fence and comment stripping, on every prefix of every program | Which stage of the pipeline a difference starts in |
| `edits` | Merging (over 1,400 cases), converting element trees back to source, and error hints, against the reference | Differences in the edit-mode and error layer |
| `prompt` | Every prompt spec in the corpus (59) against the reference's `generateSystemPrompt`. Its error messages are compared exactly; for crashes inside the JavaScript engine, only that it fails. | Differences in system prompts |
| `robustness` | Property tests: any text, cut anywhere, through every stage and every entry point, finishes without panicking | Crashes on arbitrary input |
| `depth` | Absurdly deep nesting stops at 256 levels with `depth_limited`, on a 512 KB thread stack | Stack overflows |
| `size` | Programs that double with every line stop at about 8 MiB with `size_limited` | Runaway memory |
| `streaming_cost` | A push costs about the same on a program four times the size (thread CPU time, best of 7) | Streaming that grows with the square of the program |
| `report` | The counts in CONFORMANCE.md are what the corpus holds | A stale write-up |
| `cli` | The command-line tool's commands print what the library gives, including reading stdin and the default chunk size | A broken CLI |
| `api` | The public JSON reader and writer, `JsObject`, and walking an expression | Public helpers that parsing alone doesn't reach |
| doc tests | The crate docs' examples and the README's | Examples that no longer compile or run |
| unit tests in `js.rs`, `preprocess.rs` | JSON reading and writing; incremental preprocessing against the one-shot form | Internal disagreements |
| `openui-ffi`'s `binding` | The binding returns exactly the core's output: one-shot, every streaming step, `set`, prompts, merges | A binding that changes what it hands on |
| the viewer's Swift tests | Decoding, playback, goldens through the viewer, native drawing, live streaming against a stub server, recordings, edit mode | Viewer bugs (`scripts/check-viewer.sh`) |
| compile time | The streaming parser, parse results and libraries can be shared between threads (`src/lib.rs`) | A change that stops embedders parsing off the UI thread |

Not in the check, because they need Bun, time or a screen:

| Command | What it does |
|---|---|
| `scripts/hunt.sh --minutes N` | A hunt: differential fuzzing with a fresh seed and the upstream check, for a set time, leaving a report (see [Hunts](#hunts)) |
| `scripts/upstream-check.sh <folder>` | The upstream check on its own |
| `scripts/fuzz.sh --seed N --count N` | Differential fuzzing: generated and mangled programs through the core and a live reference, comparing every step. It saves shrunk mismatches as corpus cases. |
| `scripts/fuzz.sh --edits …` | The same, for merging and converting back to source |
| `fuzz/` | Coverage-guided fuzz targets, which need nightly Rust and cargo-fuzz (testing ticket 06) |
| `scripts/bench.sh` | Streaming speed against the reference |
| `scripts/viewer-timing.sh` | The viewer's drawing speed |

## The corpus

Everything the tests compare is in `conformance/`:
- **Parse cases** are listed in `manifest.json` (hand-written), `imported.json` (upstream's docs, examples, tests and spec fixtures), `fuzz.json` (shrunk fuzzer finds) and `synthetic.json`. Each case has a `name`, a `schema` (component library), a program `file`, and optionally a `root` component name, a narrower set of `chunkings`, and `recorded` chunk sizes.
- **Chunkings** are how a case is cut into pushes:
  - `1`, `2`, `3`, `4`, `7` and `16` code points;
  - `random-1` and `random-2`, with seeded sizes their golden records;
  - `recorded`, for a case that came with its own chunks (the spec's streaming fixtures): the sizes in its `recorded`;
  - `resync-1` and `resync-2`, which mix in `set` calls.

  Every stream then ends: its last step is the result once the stream is complete, the reference's `getResult(true)` and the core's `StreamParser::finish()`.
- **Goldens** are in `golden/`. Each case has `<case>.oneshot.json`, the reference's JSON, and `<case>.steps.json`, a fingerprint of every step under every chunking, plus stage goldens. A fingerprint is FNV-1a over the JSON's UTF-8.
- **Other corpora:** `merge.json` (and its generated and imported siblings), `serialize.json`, and `prompts/`. Their goldens are in `golden/edits/` and `golden/prompts/`.
- **The reference's own oddities:**
  - `QUIRKS.md` lists behaviour the core reproduces on purpose;
  - `pending/` holds a list per golden test of the cases the core doesn't match yet: `parse`, the three stages (`tokens`, `statements`, `preprocess`), `merge`, `serialize`, `enrich` and `prompt`. A listed case may fail. One that matches again, or a listed name that isn't a case, fails its test, so the lists stay exact. Every list is empty today.

## Adding a case

1. Write the program as `conformance/cases/hand/NN-what-it-tests.oui`, using the next free number.
2. Add it to `conformance/manifest.json`: `{"name": "hand/NN-what-it-tests", "schema": "hand", "file": "cases/hand/NN-what-it-tests.oui", "source": "hand-written"}`, with a `root` if it needs one.
3. Compute its goldens from the reference with `scripts/regen-golden.sh`. This needs [Bun](https://bun.sh) 1.4. If the reference crashes on a case, the old goldens stay as they were.
4. Run `cargo test -p openui-lang --test conformance`. If the core doesn't match yet and the fix is for later, `UPDATE_PENDING=1 cargo test -p openui-lang --test conformance` lists the case in `conformance/pending/parse.txt`.
5. Refresh the counts in CONFORMANCE.md with `UPDATE_REPORT=1 cargo test -p openui-lang --test report`.

Merge, serialization and prompt cases work the same way: add an entry to `merge.json` or `serialize.json`, or a spec file to `prompts/`, then regenerate. If the reference does something that looks wrong, the core still matches it. Record the behaviour in `QUIRKS.md` with a draft upstream report; filing it is the maintainer's call. Fuzzing saves its own finds as cases.

## Reading a failure

A conformance failure names the case, the chunking and the step (the push number). Next to that it shows a diff of the reference's JSON against the core's.
- **The diff** comes from running the reference live, if Bun is installed. Otherwise you get the command that prints the reference's JSON for that step:

  ```bash
  (cd reference && bun run.ts show <case> <chunking> <step>)
  ```

- **Which test failed first** narrows things down. A failure in `stages` names the stage and the prefix length, which points closer to the cause than a whole-result diff. `streaming_ends_where_parsing_ends` lists the cases whose stream ends differently from their one-shot parse.
- **`edits` and `prompt` failures** show the expected text against the actual.

## Coverage

```bash
scripts/coverage.sh
```

The script measures line coverage of the core and the binding over the offline suite. It prints a summary per file, writes an HTML report to `target/llvm-cov/html/index.html`, and fails if a line no test runs isn't explained in [crates/openui-lang/UNREACHED.md](crates/openui-lang/UNREACHED.md). It needs cargo-llvm-cov and the `llvm-tools-preview` component, and takes about a minute.

When the script names a line:
1. **Reachable by real input:** add a case that runs it. If it's reachable through the reference too, add it to the corpus with a golden; otherwise add a test.
2. **Dead code:** delete it.
3. **Unreachable on purpose:** list its text under its file in UNREACHED.md, with the reason. That's a guard that mirrors the reference, or an invariant.

There's no percentage target; the bar is that nothing goes unexplained. On 2026-10-15 it stood at 99.12% of lines, with the remaining 30 lines all explained.

## Hunts

```bash
scripts/hunt.sh --minutes 60
```

A hunt searches for findings for a set time, then leaves a report and the shrunk cases in `findings/<date>-seed<seed>/`, a folder git ignores. It runs:
- **Differential fuzzing with a fresh seed:** parsing for the first half of the time, then edits (merging and converting back to source). An input the core takes over a second on counts as a blow-up.
- **The upstream check, alongside:** are there lang-core commits on upstream's main since the pin? If so, it computes our corpus's goldens with the newest in a scratch copy, and lists which cases' outputs change. Moving the pin is its own decision. `scripts/upstream-check.sh <folder>` runs the check alone.

A hunt never commits and never changes tracked files. It compares them before and after, and exits with status 1 if they changed. Fuzzing needs Bun; without it, the hunt runs only the upstream check, which then lists commits only. Without network, the upstream check uses the copy of upstream's history from the last time it reached GitHub (cached in `target/upstream.git`), and says so. `--seed S` repeats a hunt's fuzzing, and `--no-upstream` skips the upstream check.

### Reading the report

`report.md` starts with a summary: what each check ran and found. Then each finding gets its kind, its seed and input number, how far it shrank, the command that replays it, and where it's saved as a case. The kinds are as far as the hunt can tell:

| Kind | Means | Then |
|---|---|---|
| mismatch | The core and the reference disagree | Fixed in the core, with a case first |
| crash | The core panicked, or a fuzzer died (its last batch is the replay) | Fixed in the core |
| blow-up | The core took over a second on one input, or a fuzzer hung | Fixed in the core. If the reference blows up too and the input is unlike real output, it becomes a documented divergence. |
| quirk? | The reference throws where the core doesn't | We decide: a documented divergence, or a quirk with a draft upstream report |

Known divergences, the ones in CONFORMANCE.md the fuzzer recognizes (`__proto__` assignments, and an object shaped like an element as the root), are counted, not reported. We review findings together, one report at a time: small fixes go in one commit after the check passes, and anything that needs a design decision becomes a ticket.

### Turning a finding into a case

The hunt saves each finding in the corpus's own formats, so it moves over as it is:
- **Parsing:** copy `cases/fuzz/<name>.oui` into `conformance/cases/fuzz/`, and its entry from the folder's `fuzz.json` into `conformance/fuzz.json`. Or give it a name that says what it tests and add it as a hand-written case.
- **Merging or converting back to source:** copy its line from the folder's `merge-fuzz.json` or `serialize-fuzz.json` into the file of the same name in `conformance/`.

Then run `scripts/regen-golden.sh`. The tests now show the failure, with the step and a diff. A `quirk?` isn't saved as a case, because the reference has no output for it to compare with; it's written up in CONFORMANCE.md or QUIRKS.md instead. A replay command regenerates its input from the corpus, so it reproduces the finding only while the corpus is unchanged.

## Switches

| Variable | Effect |
|---|---|
| `OPENUI_CONFORMANCE_DIR=<path>` | Read the corpus and goldens from that folder instead of the repository's `conformance/`, for a copy or a phone. The `report` test also reads `CONFORMANCE.md` and `reference/UPSTREAM` from the source tree. |
| `OPENUI_NO_LIVE_DIFFS=1` | A failure shows the core's side and the command to see the reference's, and never starts Bun. Use it for mutation testing, where every caught mutant would start Bun, and on phones. |
| `UPDATE_PENDING=1` | Rewrite each test's list in `conformance/pending/` to the cases failing now. Run the whole suite with it (`cargo test -p openui-lang --no-fail-fast`) to rewrite them all. |
| `UPDATE_REPORT=1` | Rewrite the counts in CONFORMANCE.md |
| `DEPTH_STACK_KB=<n>` | The thread stack the depth tests run on, in KB (default 512) |

When you point `OPENUI_CONFORMANCE_DIR` at a copy, turn live diffs off too. A live diff runs the reference on the repository's corpus, not the copy's.
