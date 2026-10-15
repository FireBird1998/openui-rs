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
| `conformance` | Every corpus case (300), one-shot and after every streaming push, under each of the corpus's chunkings, against the reference's goldens. Also that a stream ends where one-shot parsing ends, apart from three kinds of programs where the reference itself doesn't. | Any difference from the reference in parsing or streaming |
| `stages` | Tokens, statements and expressions, and fence and comment stripping, on every prefix of every program | Which stage of the pipeline a difference starts in |
| `edits` | Merging (over 1,400 cases), converting element trees back to source, and error hints, against the reference | Differences in the edit-mode and error layer |
| `prompt` | Every prompt spec in the corpus (44) against the reference's `generateSystemPrompt` | Differences in system prompts |
| `robustness` | Property tests: any text, cut anywhere, through every stage and every entry point, finishes without panicking | Crashes on arbitrary input |
| `depth` | Absurdly deep nesting stops at 256 levels with `depth_limited`, on a 512 KB thread stack | Stack overflows |
| `size` | Programs that double with every line stop at about 8 MiB with `size_limited` | Runaway memory |
| `streaming_cost` | A push costs about the same on a program four times the size (thread CPU time, best of 7) | Streaming that grows with the square of the program |
| `report` | The counts in CONFORMANCE.md are what the corpus holds | A stale write-up |
| `cli` | The command-line tool's commands | A broken CLI |
| doc tests | The crate docs' examples and the README's | Examples that no longer compile or run |
| unit tests in `js.rs`, `preprocess.rs` | JSON reading and writing; incremental preprocessing against the one-shot form | Internal disagreements |
| `openui-ffi`'s `binding` | The binding returns exactly the core's output: one-shot, every streaming step, `set`, prompts, merges | A binding that changes what it hands on |
| the viewer's Swift tests | Decoding, playback, goldens through the viewer, native drawing, live streaming against a stub server, recordings, edit mode | Viewer bugs (`scripts/check-viewer.sh`) |
| compile time | The streaming parser, parse results and libraries can be shared between threads (`src/lib.rs`) | A change that stops embedders parsing off the UI thread |

Not in the check, because they need Bun, time or a screen:

| Command | What it does |
|---|---|
| `scripts/fuzz.sh --seed N --count N` | Differential fuzzing: generated and mangled programs through the core and a live reference, comparing every step. It saves shrunk mismatches as corpus cases. |
| `scripts/fuzz.sh --edits …` | The same, for merging and converting back to source |
| `fuzz/` | Coverage-guided fuzz targets, which need nightly Rust and cargo-fuzz (testing ticket 06) |
| `scripts/bench.sh` | Streaming speed against the reference |
| `scripts/viewer-timing.sh` | The viewer's drawing speed |

## The corpus

Everything the tests compare is in `conformance/`:
- **Parse cases** are listed in `manifest.json` (hand-written), `imported.json` (upstream's docs, examples and tests), `fuzz.json` (shrunk fuzzer finds) and `synthetic.json`. Each case has a `name`, a `schema` (component library), a program `file`, and optionally a `root` name and a narrower set of `chunkings`.
- **Chunkings** are how a case is cut into pushes:
  - `1`, `2`, `3`, `4`, `7` and `16` code points;
  - `random-1` and `random-2`, with seeded sizes their golden records;
  - `resync-1` and `resync-2`, which mix in `set` calls.
- **Goldens** are in `golden/`. Each case has `<case>.oneshot.json`, the reference's JSON, and `<case>.steps.json`, a fingerprint of every step under every chunking, plus stage goldens. A fingerprint is FNV-1a over the JSON's UTF-8.
- **Other corpora:** `merge.json` (and its generated and imported siblings), `serialize.json`, and `prompts/`. Their goldens are in `golden/edits/` and `golden/prompts/`.
- **The reference's own oddities:**
  - `QUIRKS.md` lists behaviour the core reproduces on purpose;
  - `pending.txt` lists cases the core doesn't handle yet. It's empty today.

## Adding a case

1. Write the program as `conformance/cases/hand/NN-what-it-tests.oui`, using the next free number.
2. Add it to `conformance/manifest.json`: `{"name": "hand/NN-what-it-tests", "schema": "hand", "file": "cases/hand/NN-what-it-tests.oui", "source": "hand-written"}`, with a `root` if it needs one.
3. Compute its goldens from the reference with `scripts/regen-golden.sh`. This needs [Bun](https://bun.sh) 1.4.
4. Run `cargo test -p openui-lang --test conformance`. If the core doesn't match yet and the fix is for later, `UPDATE_PENDING=1 cargo test -p openui-lang --test conformance` lists the case as pending.
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

## Switches

| Variable | Effect |
|---|---|
| `OPENUI_CONFORMANCE_DIR=<path>` | Read the corpus and goldens from that folder instead of the repository's `conformance/`, for a copy or a phone. The `report` test also reads `CONFORMANCE.md` and `reference/UPSTREAM` from the source tree. |
| `OPENUI_NO_LIVE_DIFFS=1` | A failure shows the core's side and the command to see the reference's, and never starts Bun. Use it for mutation testing, where every caught mutant would start Bun, and on phones. |
| `UPDATE_PENDING=1` | Rewrite `conformance/pending.txt` to the cases failing now |
| `UPDATE_REPORT=1` | Rewrite the counts in CONFORMANCE.md |
| `DEPTH_STACK_KB=<n>` | The thread stack the depth tests run on, in KB (default 512) |

When you point `OPENUI_CONFORMANCE_DIR` at a copy, turn live diffs off too. A live diff runs the reference on the repository's corpus, not the copy's.
