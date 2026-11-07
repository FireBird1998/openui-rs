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
| `conformance` | Every corpus case (534), one-shot, after every streaming push and at each stream's end, under each of the corpus's chunkings, against the reference's goldens. Also that a stream, once ended, gives what one-shot parsing gives, apart from programs where the reference itself doesn't. | Any difference from the reference in parsing or streaming |
| `stages` | Tokens, statements and expressions, and fence and comment stripping, on every prefix of every program | Which stage of the pipeline a difference starts in |
| `edits` | Merging (over 1,400 cases), converting element trees back to source, and error hints, against the reference | Differences in the edit-mode and error layer |
| `prompt` | Every prompt spec in the corpus (61) against the reference's `generateSystemPrompt`. Its error messages are compared exactly; for crashes inside the JavaScript engine, only that it fails. | Differences in system prompts |
| `robustness` | Property tests: any text, cut anywhere, through every stage and every entry point, finishes without panicking | Crashes on arbitrary input |
| `depth` | Absurdly deep nesting stops at 256 levels with `depth_limited`, on a 64 KB thread stack, because the core grows its stack on demand; so do state defaults nesting the ones above them, at 768 JSON levels | Stack overflows, a recursive walk the stack growth misses |
| `size` | Programs that double with every line stop at the result-size limit with `size_limited` (by default about 64 KiB for these short programs, up to 8 MiB with the limit fixed there), state defaults evaluated from the ones above them too | Runaway memory |
| `limits` | The `Limits` value: its documented defaults; raising and lowering depth and result size, one-shot and streaming; the result-size limit scaling with the program (floor, amplification, ceiling, or fixed), a small program amplified past it cut, a stream cut early and whole once its text has grown, and the `stream` fuzz target's slow input from the 1.1 long runs, shrunk to 86 bytes with a 98 KB result; past the program-size limit, a flagged result equal to parsing the text up to it (never splitting a character), and a stream that ignores pushes until `set`; no default cuts a corpus case | A limit that doesn't reach a stage, or a default that cuts real programs |
| `entry_points` | `Limits` on the other entry points: merging reads each input to the program-size limit and expressions to the depth limit, flagged; one expression stops at the depth limit; reading JSON, converting back to source and prompt specs nest three JSON levels per depth level and 32 more (800 by default), refused past it and let through when raised; a 30,000-level JSON value frees on a 64 KB stack; a prompt's time grows linearly with its component groups (best of 3) | An entry point a hostile input can run unbounded, or a limit that doesn't reach it |
| `drop` | Deep results, at the default depth limit and at 20,000 levels, parse, stream and drop on a 64 KB thread stack: arrays, elements, a runtime expression, evaluated defaults and a 10,000-statement reference chain. Run it unoptimized too (`CARGO_PROFILE_TEST_OPT_LEVEL=0`), where frames are largest | A tree that frees itself by recursion |
| `streaming_cost` | A push costs about the same on a program four times the size, bare and in a fence with prose around it (thread CPU time, best of 7) | Streaming that grows with the square of the program |
| `report` | The counts in CONFORMANCE.md are what the corpus holds | A stale write-up |
| `cli` | The command-line tool's commands print what the library gives, including reading stdin, the default chunk size, and `--limit name=value` (and its error naming the limits) for every command: `tokens` reads to the program-size limit, `prompt` refuses a spec past the depth limit | A broken CLI |
| `api` | The public JSON reader and writer, `JsObject`, and walking an expression | Public helpers that parsing alone doesn't reach |
| doc tests | The crate docs' examples and the README's | Examples that no longer compile or run |
| unit tests in `js.rs`, `preprocess.rs`, `progress.rs` | JSON reading and writing; incremental preprocessing against the one-shot form; the stall guard: each guarded loop panics naming itself on a forced stall, and a release stall gives a flagged cut result, a stream that ignores pushes until `set`, and a refused JSON read | Internal disagreements; a loop that stops advancing |
| unit tests in the fuzzer's `rules.rs` | The invariants' newer exceptions (quirks 4's empty redefinition, 21 and 22) on the programs that found them, run on the core, and near misses each must not explain | An exception that stops recognizing its quirk, or explains too much |
| `openui-ffi`'s `binding` | The binding returns exactly the core's output: one-shot, every streaming step, `set`, prompts, merges, and with limits given, merges and prompts included | A binding that changes what it hands on, or drops a limit |
| `openui-ffi`'s `stack` | The deepest programs parse, stream and merge through the binding on a 64 KB thread stack, at the default depth limit and at 20,000 levels, and their results and sessions drop there too | A deep drop or walk on the caller's stack |
| the viewer's Swift tests | Decoding, playback, goldens through the viewer, native drawing, live streaming against a stub server, recordings, edit mode | Viewer bugs (`scripts/check-viewer.sh`) |
| compile time | The streaming parser, parse results and libraries can be shared between threads (`src/lib.rs`) | A change that stops embedders parsing off the UI thread |

Not in the check, because they need Bun, time or a screen:

| Command | What it does |
|---|---|
| `scripts/hunt.sh --minutes N` | A hunt: differential fuzzing with a fresh seed and the upstream check, for a set time, leaving a report (see [Hunts](#hunts)) |
| `scripts/upstream-check.sh <folder>` | The upstream check on its own |
| `scripts/fuzz.sh --seed N --count N` | Differential fuzzing: generated and mangled programs through the core and a live reference, comparing every step. It saves shrunk mismatches as corpus cases. |
| `scripts/fuzz.sh --edits …` | The same, for merging and converting back to source |
| `scripts/fuzz.sh --invariants …` | The invariants, on the core and the reference alike (see [Invariants](#invariants)) |
| `scripts/fuzz-coverage.sh …` | Coverage-guided fuzzing of the targets in `fuzz/` (see [Coverage-guided fuzzing](#coverage-guided-fuzzing)) |
| `scripts/mutants.sh parsing\|rest` | Mutation testing (see [Mutation testing](#mutation-testing)) |
| `scripts/import-recordings.sh` | Recorded streams into the corpus (see [Recorded streams](#recorded-streams)) |
| `scripts/android-test.sh` | The suite on a running Android emulator or phone (see [Android](#android)) |
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
- **Other corpora:** `merge.json` (and its generated and imported siblings), `serialize.json`, and `prompts/`. Their goldens are in `golden/edits/` and `golden/prompts/`. Two prompt specs per imported library are written by the import: the spec fixtures' library, and upstream's react-ui library (`react-ui-schema`), which the viewer asks Claude with. The import loads react-ui with React and its other dependencies stubbed out (`reference/react-ui-stubs.ts`), so nothing gets installed.
- **The reference's own oddities:**
  - `QUIRKS.md` lists behaviour the core reproduces on purpose;
  - `pending/` holds a list per golden test of the cases the core doesn't match yet: `parse`, the three stages (`tokens`, `statements`, `preprocess`), `merge`, `serialize`, `enrich` and `prompt`. A listed case may fail. One that matches again, or a listed name that isn't a case, fails its test, so the lists stay exact. Every list is empty today.

## Adding a case

1. Write the program as `conformance/cases/hand/NN-what-it-tests.oui`, using the next free number.
2. Add it to `conformance/manifest.json`: `{"name": "hand/NN-what-it-tests", "schema": "hand", "file": "cases/hand/NN-what-it-tests.oui", "source": "hand-written"}`, with a `root` if it needs one.
3. Compute its goldens from the reference with `scripts/regen-golden.sh`. This needs [Bun](https://bun.sh) 1.4. If the reference crashes on a case, the old goldens stay as they were.
4. Run `cargo test -p openui-lang --test conformance`. If the core doesn't match yet and the fix is for later, `UPDATE_PENDING=1 cargo test -p openui-lang --test conformance` lists the case in `conformance/pending/parse.txt`.
5. Refresh the counts in CONFORMANCE.md with `UPDATE_REPORT=1 cargo test -p openui-lang --test report`.

Merge, serialization and prompt cases work the same way: add an entry to `merge.json` or `serialize.json`, or a spec file to `prompts/`, then regenerate. If the reference does something that looks wrong, the core still matches it. Record the behaviour in `QUIRKS.md` with a draft upstream report (unless it only follows from an upstream fix); filing it is the maintainer's call. Fuzzing saves its own finds as cases.

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
- **Differential fuzzing with a fresh seed:** parsing for the first 40% of the time, then edits (merging and converting back to source) for the next 35%. An input the core takes over a second on counts as a blow-up.
- **The invariants**, for the rest of the time, with the same seed (see [Invariants](#invariants)).
- **Coverage-guided fuzzing, alongside** for the whole hunt, when cargo-fuzz is installed: each target on a core of its own (see [Coverage-guided fuzzing](#coverage-guided-fuzzing)). Its crashes, timeouts and memory blow-ups are findings.
- **The upstream check, alongside:** are there lang-core commits on upstream's main since the pin? If so, it computes our corpus's goldens with the newest in a scratch copy, and lists which cases' outputs change. Moving the pin is its own decision. `scripts/upstream-check.sh <folder>` runs the check alone.

A hunt never commits and never changes tracked files. It compares them before and after, and exits with status 1 if they changed. Fuzzing needs Bun; without it, the hunt runs only the upstream check, which then lists commits only. Without network, the upstream check uses the copy of upstream's history from the last time it reached GitHub (cached in `target/upstream.git`), and says so. `--seed S` repeats a hunt's fuzzing, and `--no-upstream` skips the upstream check.

### Reading the report

`report.md` starts with a summary: what each check ran and found. Then each finding gets its kind, its seed and input number, how far it shrank, the command that replays it, and where it's saved as a case. The kinds are as far as the hunt can tell:

| Kind | Means | Then |
|---|---|---|
| mismatch | The core and the reference disagree | Fixed in the core, with a case first |
| crash | The core panicked, a fuzzer died (its last batch is the replay), or the reference died on an input (it's restarted and the run goes on; the input isn't shrunk) | Fixed in the core. A reference that dies on an input is a divergence or a quirk. |
| blow-up | The core took over a second on one input, or a fuzzer hung | Fixed in the core. If the reference blows up too and the input is unlike real output, it becomes a documented divergence. |
| quirk? | The reference throws where the core doesn't | We decide: a documented divergence, or a quirk with a draft upstream report |

Known divergences, the ones in CONFORMANCE.md the fuzzer recognizes (`__proto__` assignments, and a root that's an object shaped like an element or an AST node), are counted, not reported, including the reference throwing on the last. We review findings together, one report at a time: small fixes go in one commit after the check passes, and anything that needs a design decision becomes a ticket.

### Turning a finding into a case

The hunt saves each finding in the corpus's own formats, so it moves over as it is:
- **Parsing:** copy `cases/fuzz/<name>.oui` into `conformance/cases/fuzz/`, and its entry from the folder's `fuzz.json` into `conformance/fuzz.json`. Or give it a name that says what it tests and add it as a hand-written case.
- **Merging or converting back to source:** copy its line from the folder's `merge-fuzz.json` or `serialize-fuzz.json` into the file of the same name in `conformance/`.

Then run `scripts/regen-golden.sh`. The tests now show the failure, with the step and a diff. A `quirk?` isn't saved as a case, because the reference has no output for it to compare with; it's written up in CONFORMANCE.md or QUIRKS.md instead. A replay command regenerates its input from the corpus, so it reproduces the finding only while the corpus is unchanged.

## Invariants

```bash
scripts/fuzz.sh --invariants --seed 1 --count 1000000
```

An invariant is a rule a result should keep whatever the input. Each is written once, in `examples/fuzz/rules.rs` (the runner is `invariants.rs`), against an engine interface (`engine.rs`) with two implementations: the core in-process, and the reference through the fuzzer's long-lived Bun process. The interface covers a full one-shot result, a stream over given chunks, merging, and converting back to source. Results and trees cross it as JSON, as each parser wrote them. The programs are the differential fuzzer's: generated from a library's grammar, or corpus programs mangled. The merging rules also take a patch for each, from the edit fuzzer's generator. `--rule NAME` runs one rule.

Programs whose result passes 1 MiB of JSON, as the core measures it, are kept from the reference, which runs out of memory on them (a divergence). That goes for what a rule makes too, such as a merge's result: the rule doesn't apply to it, on either parser.

Each program's verdict on each parser is that the rule holds, doesn't apply, or is broken. Sorting:

| Core | Reference | Meaning |
|---|---|---|
| broken | broken | A candidate quirk. The maintainer judges whether the reference is wrong (a quirk, with a draft upstream report, and an exception to the rule) or the rule is (it's narrowed). |
| broken | holds | A mismatch, fixed in the core |
| holds | broken | A mismatch too: the parsers disagree |

A break a known quirk causes is that rule's **exception**: the rule recognizes it from the tree and counts it, without reporting it. A disagreement or a break that one of CONFORMANCE.md's divergences explains (`__proto__`, an object literal shaped like an element or AST node as the root) is counted the same way.

### Round trip (`round-trip`)

Parsing, converting the tree back to source and parsing that again gives the same tree. Trees are compared as upstream's own round-trip test compares them, without the `partial` and `hasDynamicProps` flags (at every depth here), and with a null prop the same as a missing one, since converting back to source leaves trailing nulls out on purpose (language.md, section 9.2).

It doesn't apply to:
- a program without a tree, or one still arriving (`meta.incomplete`), whose values may be half written;
- a tree that refers to a Query or Mutation statement: `jsonToOpenUI` takes the tree and the state, not those statements, and the spec says they don't round-trip;
- a tree with a placeholder (`Ph`), or with an element inside an element of its own statement: a statement that refers to itself, which the tree cuts;
- a program with a number JSON can't hold (`1e` is `NaN`, `1e400` is `Infinity`), since JSON writes both as null.

Its exceptions, each beside its quirk in QUIRKS.md:

| Exception | Quirk |
|---|---|
| A unary operand, a binary operator's ternary operand, or a member access's object that needs parentheses | 13 |
| A right operand of the same precedence | 9 |
| An object key that isn't a bare name | 10 (by design since 1.0) |
| A bare call to a library function or action | 12 |
| A call parsing made null (`{k: "Null"}`) | 14 (a candidate) |
| A call named after an `Object.prototype` member | 2 |
| Statements that refer to each other in a cycle, with one statement as two different elements in the tree | 21 |
| An object literal shaped like an element or an AST node | the divergences' family |

On a million programs (seed 61, about a minute): 169,773 hold, 824,291 don't apply, 5,936 are exceptions, and none is a mismatch or unexplained. Quirk 14 accounts for 2,054 exceptions, 12 and the node-shaped literals for 1,132 each, 13 for 862 and 10 for 707. Against lang-core 1.1 (seed 82, lang-core 1.1 ticket 08), the 91 programs that broke the round trip and merging its own source on both parsers were quirk 21's.

### Chunking doesn't matter (`chunking`)

A stream's result once it has ended is the same however the program was cut: in one chunk, in the fuzzer's random chunks of up to 16 code points, and in chunks of three. It applies to non-empty programs up to 4,000 code points.

### Streaming ends where one-shot parsing ends (`stream-end`)

A stream in the random chunks gives, once ended, what parsing the whole program at once gives. Same lengths. Exceptions: a redefinition counted twice (quirk 4, including a line `a=` with nothing after it, behind a stray opening bracket), a mismatched closing bracket (quirk 5), a final redefinition that needed closing (quirk 11), a program without statements (quirk 18), a stray character before a ternary's `?` (quirk 19).

### Fences don't matter (`fences`)

A program inside a markdown fence, with prose before and after (no apostrophe, which is quirk 1's), parses the same as the bare program. It applies to finished programs without a fence of their own, since an unclosed string or bracket would take in the closing fence and the prose. Exception: a program that's empty once preprocessed (quirk 18).

### Merging behaves (`merge-twice`, `merge-empty`, `merge-own-source`)

- Applying a patch twice parses the same as applying it once. (The text can differ in order: a patch that deletes a name and defines it again moves it the second time.)
- An empty patch changes nothing: the merged program parses as the program did.
- The program's tree converted back to source works as a patch: merging it in gives the same tree. This applies where the round trip does, with its exceptions.

Exceptions: an unfinished statement, in the program or the patch, which takes in what's appended after it (quirk 15); a program without statements, which the merge returns the patch for as it is (quirk 16); an existing program with fences, whose prose the merge reads as code (quirk 7); a statement whose ternary is still open (more `?` than `:` outside brackets), in the program, the patch or the first merge's result, which takes in what the merge writes after it (quirk 22). Quirks 8 and 20, a name defined twice and comments read as code, were exceptions until 1.1 fixed them.

### Finished statements stay put (`finished-stay-put`)

While streaming, a statement that is complete and fully resolved shows the same element in every later result, until it or a statement it reaches is defined again.

- **Complete**, as language.md, section 4, rule 1 defines it: its ending newline has arrived at bracket depth zero, outside strings, with no ternary open, and the next line's first non-whitespace character has arrived and isn't `?`.
- **Fully resolved:** it and every statement it reaches, through references, are complete; no reference closes a cycle; and none of them is being defined again in the text still arriving (a pending redefinition takes over once it no longer needs closing, section 4, rule 3).

The tree names only statements that are components, so those are the ones the rule sees. It applies to programs up to 2,000 code points that preprocessing leaves alone (no fences or comments). Exception: while a later statement is open, finished elements are marked partial and their props checked leniently (quirk 17), recognized as a change between two results whose `meta.incomplete` differs.

### The exceptions, by quirk

| Quirk | Rules it's an exception to |
|---|---|
| 2: a call named after an `Object.prototype` member | round trip, merging its own source |
| 4, 5, 11, 19: streaming's redefinitions, brackets and statement boundaries | chunking, stream end |
| 9, 13: dropped parentheses | round trip, merging its own source |
| 10: unquoted object keys (by design since 1.0) | round trip, merging its own source |
| 12: bare library calls | round trip, merging its own source |
| 14: a call made null when parsing | round trip, merging its own source |
| 21: a statement reached through a reference cycle, written back once | round trip, merging its own source |
| 8, 15, 16, 20, 22: merging duplicate names, unfinished or empty programs, comments, open ternaries | merging twice, merging an empty patch |
| 17: finished statements treated as partial | finished statements stay put |
| 18: an empty program reads as incomplete | fences, chunking, stream end |
| the divergences' node-shaped literals and `__proto__` | any |

## Mutation testing

```bash
scripts/mutants.sh parsing --jobs 3   # testing ticket 04: about 1,800 mutants, the 1.1 evaluator's included
scripts/mutants.sh rest --jobs 3      # testing ticket 05: about 650, the binding's included
```

[cargo-mutants](https://mutants.rs) plants one bug at a time in the source (a `>` turned into `>=`, a function that returns a default, a deleted `!`) and runs the offline suite against each, with live diffs off. A mutant the suite doesn't catch is code a real bug could break unnoticed. Each one is either caught by a new case, or listed in `.cargo/mutants.toml` with the reason it changes nothing a caller can observe, so reruns skip it. A mutant costs a rebuild plus the suite, so a whole group takes hours: run it on AC power, or on GitHub (see [On GitHub](#on-github)). Results go to `target/mutants/<group>/mutants.out/` (`missed.txt`, `caught.txt`, `timeout.txt`).

A mutant that stops the parser advancing makes a test, or the CLI a test starts, allocate without bound, faster than cargo-mutants' timeout ends it, and macOS has no per-process memory limit: such processes once took a Mac down with a kernel panic, and later froze it. So the script runs `scripts/mutants-watchdog.sh` beside cargo-mutants. It kills any process run from a mutant's build, test or CLI, whose memory passes `MUTANTS_MEMORY_LIMIT_MB` (default 2048; the suite peaks near 600 MB), and on macOS, when free memory falls under 20%, the largest one over 1 GB. On macOS memory is the physical footprint, compressed memory included, since under pressure the compressor keeps a runaway's RSS small. The test fails, the mutant counts as caught, and each kill is logged (`mutants-watchdog: killed a process holding …`).

## Coverage-guided fuzzing

```bash
scripts/fuzz-coverage.sh --seconds 28800              # every target, a full night each, in parallel (or on GitHub: see On GitHub)
scripts/fuzz-coverage.sh --seconds 600 merge json     # some of them
```

libFuzzer, through cargo-fuzz on the nightly toolchain, mutates inputs toward code they haven't reached yet. The targets, in `fuzz/fuzz_targets/`, check that nothing panics:

| Target | What it runs | Seeds |
|---|---|---|
| `parse` | One-shot parsing, with and without a root name | the corpus's programs |
| `stream` | Streaming in chunks the input's first byte picks, and resyncing with `set` | the same, with a plan byte |
| `merge` | Merging a program with a patch (a zero byte between them) | the merge cases |
| `serialize` | Converting JSON back to source: a parse result's tree with its state, or anything else | the parse goldens |
| `json` | Reading JSON, and that writing what it read settles after one round | the parse goldens |

They share a library (`fuzz/fuzz_targets/library.rs`) with every schema shape validation handles, plus 1.0's library functions and actions. The dictionary, `fuzz/openui.dict`, holds fragments of the language and of model output. Inputs stay under 8 KB, the size of a long response. An input over 10 s is a blow-up: the core answers any input well inside a second in release, and the fuzz build's debug assertions make it slower. The fuzz build has no AddressSanitizer: the crate forbids unsafe code, and over hours the sanitizer's own memory grows without bound (GitHub's 5-hour runs stopped at 4 GB that way). The working corpus (`fuzz/corpus/`) and the finds (`fuzz/artifacts/`) aren't tracked. Replay a find with `cargo +nightly fuzz run <target> <file>`.

## Recorded streams

```bash
scripts/import-recordings.sh            # from the viewer's own folder
scripts/import-recordings.sh <folder>   # or any folder of *.recording.json
```

The viewer saves each live request it streams from Claude as a recording (the format is documented on `Recording` in `viewer/Sources/ViewerKit/Recording.swift`): the request, the models, the effort, the library, and each text delta as it arrived. The command copies new recordings into `conformance/recordings/`, turns every recording there into cases (`reference/import-recordings.ts`), and regenerates the goldens:

- **A fresh recording** becomes a parse case, `recorded/<file>`, in its library, with a **recorded chunking**: the text cut where the deltas arrived, in code points. The reference computes that chunking's golden alongside the corpus's others, and the conformance test checks it at every step.
- **A follow-up** (an edit-mode request, which records the program it patched) becomes a merge case, that program with its text as the patch, and a parse case of the merged program, `recorded/<file>-merged`.

The request, the models, the effort and when it was recorded stay with each case, in `recorded.json` and `merge-recorded.json`. Recordings never hold a key. `tests/recordings.rs` checks that every recording is in the corpus as the command puts it there, and that a recording has only the fields the viewer writes. The two recordings there now are hand-made, to prove the path; real ones come from the viewer.

## Android

```bash
scripts/android-test.sh                    # the running emulator, or the only device
scripts/android-test.sh --serial <serial>  # one of several
scripts/android-test.sh --abi x86_64       # an x86_64 emulator, as on GitHub
```

Cross-compiles the test binaries for 64-bit ARM (or `--abi`'s) with cargo-ndk, pushes them to the device with the conformance data (about 21 MB, to `/data/local/tmp/openui-rs/`), and runs each there with `OPENUI_CONFORMANCE_DIR` pointing at the copy, live diffs off, and the depth tests on a 512 KB thread stack (`DEPTH_STACK_KB`). It needs the Android SDK and NDK, cargo-ndk and the `aarch64-linux-android` Rust target, and a device already running. Two test binaries stay on the host: `report` checks CONFORMANCE.md in the source tree, and `cli` runs the host's command-line binary. Each binary's log is left in `target/android-<test>.log`. It isn't part of the check, and hunts don't run it.

## On GitHub

Every push to `main`, and every pull request, runs the check on Linux with Bun installed, so the conformance tests diff against the reference too, and the viewer's check on macOS 27 (`.github/workflows/ci.yml`). Both use the maintainer's stable Rust, 1.97.1, so clippy's lints there are the ones the check passed locally. The long runs don't run on their own: each is a workflow started by hand, from the repository's Actions tab or with `gh`:

```bash
gh workflow run mutants.yml -f group=parsing -f shards=16       # or group=rest, or both
gh workflow run fuzz-coverage.yml -f seconds=20400              # every target; -f targets="merge json" for some
gh workflow run invariants.yml -f seed=82 -f count=1000000      # all eight rules; -f rule=chunking for one
gh workflow run android.yml
gh run download <run id>                                        # what a run kept
```

A job on GitHub's machines may run 6 hours, so each workflow splits its run:

| Workflow | Split | What it keeps |
|---|---|---|
| `mutants.yml` | Each group's mutants across `shards` jobs (`--shard`), two at a time each | Each shard's `mutants.out/`, and the lists put together as `mutants`; the counts and the missed mutants are in the run's summary |
| `fuzz-coverage.yml` | One job a target, for `seconds` (5 h 40 min at most). A full night is two runs: each target's corpus is kept in the Actions cache, and the next run starts from it. | Each target's log and finds, as `fuzz-<target>`. A find fails the target's job. |
| `invariants.yml` | The programs into `shards` ranges of one seed (`--from`, `--count`), so it's the same run as one long one | Each range's findings folder; the totals by rule and every finding, with its replay command, are in the run's summary |
| `android.yml` | One job: an x86_64 emulator (GitHub's Linux machines have KVM, not ARM), `scripts/android-test.sh --abi x86_64` | The test logs |

The 64-bit ARM run stays on the Mac's emulator or a phone.

## Switches

| Variable | Effect |
|---|---|
| `OPENUI_CONFORMANCE_DIR=<path>` | Read the corpus and goldens from that folder instead of the repository's `conformance/`, for a copy or a phone. The `report` test also reads `CONFORMANCE.md` and `reference/UPSTREAM` from the source tree. |
| `OPENUI_NO_LIVE_DIFFS=1` | A failure shows the core's side and the command to see the reference's, and never starts Bun. Use it for mutation testing, where every caught mutant would start Bun, and on phones. |
| `UPDATE_PENDING=1` | Rewrite each test's list in `conformance/pending/` to the cases failing now. Run the whole suite with it (`cargo test -p openui-lang --no-fail-fast`) to rewrite them all. |
| `UPDATE_REPORT=1` | Rewrite the counts in CONFORMANCE.md |
| `DEPTH_STACK_KB=<n>` | The thread stack the depth tests run on, in KB (default 64) |

When you point `OPENUI_CONFORMANCE_DIR` at a copy, turn live diffs off too. A live diff runs the reference on the repository's corpus, not the copy's.
