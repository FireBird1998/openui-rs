# Testing

The core is correct when it does exactly what the reference does: `@openuidev/lang-core` at the pinned commit (see [CONFORMANCE.md](CONFORMANCE.md)). Most tests compare the core's output with the reference's, saved as goldens, so they need only Rust: no Bun and no network. The rest check what conformance can't: no input crashes the core, deep or huge input is cut off safely, streaming stays linear, and the binding hands on exactly what the core produces.

This guide covers what each test catches, how to run it, how to add a case, and how to read a failure. The testing phase ([spec](.scratch/testing/spec.md)) adds sections as its tickets land: hunts, coverage, mutation testing, coverage-guided fuzzing, invariants, recorded streams and Android.

## Running the tests

```bash
scripts/check.sh
```

The check runs formatting, clippy, the docs and every Rust test, including the doc examples and this repository's README example. It then builds the workspace on Rust 1.88.0, the oldest version the crates claim. That build is skipped, with a note, when the toolchain isn't installed; the check never installs one. It takes a minute or two.

```bash
scripts/check-viewer.sh                                   # OpenUINative's and the viewer's Swift tests (needs Xcode)
cargo test -p openui-lang --test conformance              # one test file
cargo test -p openui-lang --test conformance streaming_   # tests whose names match
```

## Allocation caps

Every core, runtime and binding Rust test binary uses the counting global allocator in
`crates/test-alloc`, a private, unpublished dev dependency. The published libraries
keep their unsafe-code prohibition and have no allocator, public API or default
feature changes. Only the test helper implements the unsafe `GlobalAlloc` contract.
Doc examples install the same allocator through hidden setup lines.

Each binary installs the allocator once with `Allocator::with_cap(...)`. There
is no per-test setup to forget: a const-initialized thread-local creates an
account on the thread's first allocation. Setup uses `System` directly for one
small permanent owner record and guards against re-entrancy. Libtest gives each
test a separate named thread, so property-test generation, every generated case,
and test setup/cleanup are all counted automatically. The libtest main thread
has its own account too.

The **64 MiB allocation cap** applies to the core's `size`, `depth`, `work`,
`limits`, `drop`, `entry_points`, `streaming_cost`, `streaming_reuse` and `cli` binaries, and the
binding's `stack` and `binding` binaries, and the runtime's `expressions`,
`limits`, `allowances`, `nesting`, `lookup_scope`, `reference_allowance`,
`supplied_values` binaries. Runtime `session_reentry` and
`parse_policy` use 128 MiB; `supplied_memory` uses 256 MiB for 900,000-array
inputs, with a stricter shape assertion that peak heap stays below twice the input heap for pass-through, cuts,
`Count`, `First` and object coercion. All other core and binding test
binaries, including unit tests, example tests and doc examples, use **256 MiB**.
Runtime `tree_loops_and_prepared_references` uses 512 MiB to prepare four million unread numbers;
its repeated-evaluation allocation and timing checks exclude preparation, and
preparation separately forbids per-number allocation.
Runtime `tree_replacement_room_and_inherited_calls` uses 256 MiB. Its fixed-room replacement-buffer and
failed-builtin argument-buffer cases also pin peak heap below twice their
input heap; prepared reads have fixed-work and allocation checks.
The mixed CLI and binding suites fit comfortably under the stricter cap.
`prompt_depth` uses 256 MiB: its 900-level object schema must cross the default
JSON depth limit and render successfully with a raised limit, which retains
nested schema copies. Moving it preserves that shape with substantial headroom.
The other largest depth shapes use 50,000 unary/member/addition links, still far
beyond the depth limit. Measured successful thread peaks stay below 60% of their
binary's allocation cap (the deliberately failing subprocess is the exception).

Workers get independent accounts against the same binary-wide cap; simultaneous
threads' peaks are not summed. The small-stack workers are named after their
test with `::worker` appended, so their reports identify the test. An aligned
prefix stores each allocation's owning account, and freeing or reallocating on
another thread credits that original account. Owner records survive thread exit
until process exit, so results returned from an exited worker remain safe to
free. CLI subprocesses are outside the test allocator's address space. Every CLI
child instead goes through one test-only helper which drains both output pipes,
polls every 10 ms, and kills/reaps the child past **512 MiB physical footprint
or 60 s**. On macOS `proc_pid_rusage` reads the same footprint (compressed
memory included) as the watchdog's `footprint` tool, and its lifetime peak
catches spikes between polls. Linux counts `/proc/<pid>/status` VmRSS + VmSwap;
other platforms fall back to `ps` RSS. Failure to measure a running child fails
the test instead of silently leaving memory unbounded.

The measurement is peak live heap bytes requested from `System`, including the
aligned accounting prefix, padding and owner record, not RSS, stack segments allocated outside
`GlobalAlloc`, or cumulative allocation traffic. Reallocation charges the size change while retaining the original owner. Shape assertions leave room
for platform layout differences: the size suite pins five doubling shapes below
1 MiB with the result-size limit fixed at 8 MiB, without serializing the result;
the depth suite pins each shape to a measured ceiling (1–36 MiB) on the worker
that actually parses and drops it. Resetting a shape measurement never resets
the thread-wide peak used for enforcement or reporting.
This catches subtree copying independently of the general allocation cap.

A failure looks like:

```text
allocation cap exceeded: test nested_arrays::worker; cap 67108864 bytes; peak 67109000 bytes
```

The peak includes the allocation about to be requested. The allocator checks
before asking `System` for memory, prints directly to stderr, and exits with 101
immediately. It cannot safely unwind from `GlobalAlloc`, so the whole test binary
fails; rerun the named test to isolate it. The `allocation` suite starts a copy of
itself with ordinary libtest capture enabled. A test with no accounting setup
deliberately requests more than the binary's 256 MiB allocation cap; the parent
checks status 101 and a diagnostic naming the test, cap and an above-cap peak.
The rejected allocation is never requested from `System`.
It also checks alignment, reallocation and freeing on another thread.

To print successful peaks (bytes), including independent worker measurements (printed automatically at thread exit):

```bash
export CARGO_NET_OFFLINE=true
OPENUI_ALLOCATION_PEAKS=1 cargo test -q -j 4 -p openui-lang --test depth -- --nocapture
```

Grown stack segments are outside the allocator too. Input-building walks have
a depth-scaled segment ceiling (ADR 0002). Unit tests inject a zero ceiling at
each capped site to exercise its typed cut and named debug panic, and a real
low-stack observation on a deep program checks the same recovery without
forcing that observation. They also check saturation, unwind restoration and
a stream staying cut until `set`. Finished-tree walks remain uncapped.

## What each test catches

| Test | Checks | Catches |
|---|---|---|
| `conformance` | Every corpus case (972), one-shot, after every streaming push and at each stream's end, under each of the corpus's chunkings, against the reference's goldens. Also that a stream, once ended, gives what one-shot parsing gives, apart from programs where the reference itself doesn't. | Any difference from the reference in parsing or streaming |
| `stages` | Tokens, statements and expressions, and fence and comment stripping, on every prefix of every program | Which stage of the pipeline a difference starts in |
| `edits` | Merging (3,269 cases), converting element trees back to source, and error hints, against the reference | Differences in the edit-mode and error layer |
| `prompt` | Every prompt spec in the corpus (65) against the reference's `generateSystemPrompt`. Its error messages are compared exactly; for crashes inside the JavaScript engine, only that it fails. | Differences in system prompts |
| `robustness` | Property tests: any text, cut anywhere, through every stage and every entry point, finishes without panicking | Crashes on arbitrary input |
| `depth` | Absurdly deep nesting stops at 256 levels with `depth_limited`, on a 64 KB thread stack, because the core grows its stack on demand; so do state defaults nesting the ones above them, at 768 JSON levels | Stack overflows, a recursive walk the stack growth misses |
| `size` | Programs that double with every line stop at the result-size limit with `size_limited` (by default about 64 KiB for these short programs, up to 8 MiB with the limit fixed there), state defaults evaluated from the ones above them too | Runaway memory |
| `work` | The work limit on evaluating state defaults: its default scaling with the program; running out cuts the default (and every later expression default) and flags the result, and raising it lets it finish; the cost in steps, found by bisecting the work limit, of `@Each` reading an expression per item (items times template), of a component read many times (its size once), and of sorting (`n log n`) and filtering (`n`); the slow fuzz input, shrunk | Evaluating that takes seconds on a small program, or a step that isn't counted |
| `limits` | The `Limits` value: its documented defaults; raising and lowering depth and result size, one-shot and streaming; the result-size limit scaling with the program (floor, amplification, ceiling, or fixed), a small program amplified past it cut, a stream cut early and whole once its text has grown, and the `stream` fuzz target's slow input from the 1.1 long runs, shrunk to 86 bytes with a 98 KB result; past the program-size limit, a flagged result equal to parsing the text up to it (never splitting a character), and a stream that ignores pushes until `set`; no default cuts a corpus case | A limit that doesn't reach a stage, or a default that cuts real programs |
| `entry_points` | `Limits` on the other entry points: merging reads each input to the program-size limit and expressions to the depth limit, flagged; one expression stops at the depth limit; reading JSON, converting back to source nest three JSON levels per depth level and 32 more (800 by default), refused past it and let through when raised; a 30,000-level JSON value frees on a 64 KB stack; a prompt's time grows linearly with its component groups (best of 3) | An entry point a hostile input can run unbounded, or a limit that doesn't reach it |
| `prompt_depth` | A 900-level tool input schema is refused at the default JSON depth limit and renders when the depth limit is raised; a lowered limit refuses a shallower schema | A prompt entry point that ignores the depth limit |
| `drop` | Deep results, at the default depth limit and at 20,000 levels, parse, stream and drop on a 64 KB thread stack: arrays, elements, a runtime expression, evaluated defaults and a 10,000-statement reference chain. Run it unoptimized too (`CARGO_PROFILE_TEST_OPT_LEVEL=0`), where frames are largest | A tree that frees itself by recursion |
| `streaming_cost` | A push costs about the same on a program four times the size, bare and in a fence with prose around it (thread CPU time, best of 7) | Streaming that grows with the square of the program |
| `scaling` | Shapes built to cost the square of their size cost about four times as much at four times the size (thread CPU time, best of 5): blank lines before a `?` line, one-shot and streamed; blank space between statements in a stream; a chain of references completed in one push; a chain of queries searched for loops, its loops and their order pinned. Unresolved names under an 80-statement chain fit the 64 MiB allocation cap, in order through reached statements; a push changing several statements gives what a fresh stream does | A scan, a cache walk or a list that is redone or copied once per line, statement or level |
| `streaming_reuse` | A stream takes the state defaults it evaluated at earlier pushes as they were (#8): every push equals a fresh stream given the text so far in one push, for defaults reading defaults, components read twice (one object), a runtime expression's properties read across defaults (one object), a redefinition, a late reference, a default cut by work or by room early and whole once the program grows, and the shrunk slow fuzz input; and by cost, a push after an expensive default costs a fraction of evaluating it, and sixty defaults streamed a character at a time cost about as much a push late as early (thread CPU time) | Reuse that changes a result, or streaming defaults that grows with the square of the program |
| `report` | The counts in CONFORMANCE.md are what the corpus holds | A stale write-up |
| `cli` | The command-line tool's commands print what the library gives, including reading stdin, the default chunk size, and `--limit name=value` (and its error naming the limits) for every command: `tokens` reads to the program-size limit, `prompt` refuses a spec past the depth limit | A broken CLI |
| `api` | The public JSON reader and writer, `JsObject`, and walking an expression | Public helpers that parsing alone doesn't reach |
| doc tests | The crate docs' examples and the README's | Examples that no longer compile or run |
| unit tests in `js.rs`, `preprocess.rs`, `progress.rs`, `result.rs`, `evaluate.rs` | JSON reading and writing; the engine-dependent divergences pinned as the core gives them (a built-in constructor turned into text, a 65-value sort whose comparison loops) and collation outside ASCII by code point; an element or a runtime expression shaped into the object its JSON reads back as, without the text, equal to writing it and reading it back; incremental preprocessing against the one-shot form; the stall guard: each guarded loop panics naming itself on a forced stall, and a release stall gives a flagged cut result, a stream that ignores pushes until `set`, and a refused JSON read | Internal disagreements; a loop that stops advancing |
| unit tests in the fuzzer's `rules.rs` | The invariants' newer exceptions (quirk 4's empty redefinition, quirk 19 across a line, 21, 22 and 23) on the programs that found them, run on the core, and the differential fuzzer's explanations of the engine-dependent divergences (sorts whose comparison loops, constructors turned into text), with near misses each must not explain | An exception that stops recognizing its quirk, or explains too much |
| `openui-ffi`'s `binding` | The binding returns exactly the core's output: one-shot, every streaming step, `set`, prompts, merges, and with limits given, merges and prompts included | A binding that changes what it hands on, or drops a limit |
| `openui-ffi`'s `stack` | The deepest programs parse, stream and merge through the binding on a 64 KB thread stack, at the default depth limit and at 20,000 levels, and their results and sessions drop there too | A deep drop or walk on the caller's stack |
| OpenUINative's Swift tests | The package through its public interface: drawing with an app's own components, an evaluated element drawn at its own path, and live programs drawn in an off-screen window, clicked and typed in the way AppKit delivers a person's (state redraws in the mounted view, host events, bound inputs and their transforms, form fields written with no component type, an app's component given evaluated props, markers and plans; a click or write whose element or marker has changed, or comes from another program, doing nothing, while two clicks or keystrokes before a redraw on something unchanged both act; a button in a renamed form and a field drawn for another name refusing; forged plans running nothing, only the exact marker shape being a marker; mutations resumed, refused steps; events delivered when the redraw after an action fails, with the session disposed underneath; upstream's form validation, rule by rule; buttons ignored and inputs disabled while a response streams; table action cells and elements in cells; a select picked from the menu its pop-up opens); the generated binding directly | Package bugs (`scripts/check-viewer.sh`) |
| the viewer's Swift tests | Decoding (lossless for every corpus parse result and evaluated tree, and for odd objects), playback with every step evaluated or, without a runtime session, its parse result, the store panel (its draft text when the value changes underneath), the inspector's tree being the drawn one under `@Each` and a ternary, the program streaming until the shown step has every chunk, host events kept to the newest with their continuations let go, the step cache counting more than JSON, goldens through the viewer, native drawing of every corpus case evaluated, live streaming against a stub server, recordings, edit mode | Viewer bugs (`scripts/check-viewer.sh`) |
| compile time | The streaming parser, parse results and libraries can be shared between threads (`src/lib.rs`) | A change that stops embedders parsing off the UI thread |

Not in the check, because they need Bun, time or a screen:

| Command | What it does |
|---|---|
| `scripts/hunt.sh --minutes N` | A hunt: differential fuzzing with a fresh seed and the upstream check, for a set time, leaving a report (see [Hunts](#hunts)) |
| `scripts/upstream-check.sh <folder>` | The upstream check on its own |
| `scripts/fuzz.sh --seed N --count N` | Differential fuzzing: generated and mangled programs through the core and a live reference, comparing every step. It saves shrunk mismatches as corpus cases. A result a limit cut can't match the reference, which has none: where the answers differ and the core's was cut for size or work, it's parsed again with those limits raised, and counts as "cut by a limit" when it matches then, a mismatch when it doesn't. |
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

### Every night

```bash
scripts/nightly-hunt.sh --install      # a 120-minute hunt at 02:00 every night
scripts/nightly-hunt.sh --uninstall
scripts/nightly-hunt.sh --minutes 30   # one now, as the job runs it
```

`scripts/nightly-hunt.sh` is a hunt nobody has to remember: a line in the user's crontab on a Mac that can spare the time, so it runs whether or not anyone is logged in at the screen. It runs only on AC power, one at a time, and brings a clean checkout up to date first. Like every hunt it never commits. The latest run's folder and exit status are in `findings/nightly-latest.txt`, and what it printed in `findings/nightly.log`. Installing it is a standing configuration on that Mac, so it's the maintainer's decision.

### Reading the report

`report.md` starts with a summary: what each check ran and found. Then each finding gets its kind, its seed and input number, how far it shrank, the command that replays it, and where it's saved as a case. The kinds are as far as the hunt can tell:

| Kind | Means | Then |
|---|---|---|
| mismatch | The core and the reference disagree | Fixed in the core, with a case first |
| crash | The core panicked, a fuzzer died (its last batch is the replay), or the reference died on an input (it's restarted and the run goes on; the input isn't shrunk) | Fixed in the core. A reference that dies on an input is a divergence or a quirk. |
| blow-up | The core took over a second on one input, or a fuzzer hung | Fixed in the core. If the reference blows up too and the input is unlike real output, it becomes a documented divergence. |
| quirk? | The reference throws where the core doesn't | We decide: a documented divergence, or a quirk with a draft upstream report |

Results cut by a limit aren't findings either: the differential fuzzer compares them with the reference under raised limits, and counts them as cut when they match then (seed 82's programs 3241 and 4052, 41 and 48 characters with 49 KB and 78 KB results, are two). Known divergences, the ones in CONFORMANCE.md the fuzzer recognizes (`__proto__` assignments, and a root that's an object shaped like an element or an AST node), are counted, not reported, including the reference throwing on the last. We review findings together, one report at a time: small fixes go in one commit after the check passes, and anything that needs a design decision becomes a ticket.

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

A stream in the random chunks gives, once ended, what parsing the whole program at once gives. Same lengths. Exceptions: a redefinition counted twice (quirk 4, including a line `a=` with nothing after it, behind a stray opening bracket), a mismatched closing bracket (quirk 5), a final redefinition that needed closing (quirk 11), a program without statements (quirk 18), a stray character before a ternary's `?`, on its line or alone on the line before (quirk 19), a bracket opened before a statement and closed after its `?` (quirk 23).

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

The watchdog's kill line includes a UTC timestamp, the mutant build directory,
and the latest mutant description associated with that worker in cargo-mutants'
`mutants.out/debug.log` (or `description unavailable`). Output records are read
only. To exercise attribution and actual kills without cargo-mutants:

```bash
python3 scripts/test-mutants-watchdog.py
```

This uses a fake output/build layout, a 64 MiB owned hog, and a `ps` shim
restricted to that PID. On macOS a fixture `footprint` shim queries libproc,
so the test also works when a sandbox denies system-wide process discovery.
It checks parallel-worker attribution, a missing description, timestamps,
and that output records stay unchanged. It never targets another session.

## Coverage-guided fuzzing

```bash
scripts/fuzz-coverage.sh --seconds 28800              # every target, a full night each, in parallel (or on GitHub: see On GitHub)
scripts/fuzz-coverage.sh --seconds 600 merge json     # some of them
```

libFuzzer, through cargo-fuzz on the nightly toolchain, mutates inputs toward code they haven't reached yet. The targets, in `fuzz/fuzz_targets/`, check that nothing panics, and that nothing amplifies:

| Target | What it runs | Seeds |
|---|---|---|
| `parse` | One-shot parsing, with and without a root name, under the default limits and under small ones (a 1 KiB floor, 4 times the program, a little work, 16 levels) that it hits often; every result checked (below) | the corpus's programs, and the long runs' slow inputs (`crates/openui-lang/tests/slow/`) |
| `stream` | Streaming in chunks the input's first byte picks, at most 128 pushes (fewer for a long input, see the caps below), under the default or the small limits, and resyncing with `set`; every push's result checked, its limits scaling with the text so far | the same, with a plan byte |
| `merge` | Merging a program with a patch (a zero byte between them); the merge is never longer than its inputs, give or take 64 bytes of joining newlines | the merge cases |
| `serialize` | Converting JSON back to source: a parse result's tree with its state, or anything else | the parse goldens |
| `json` | Reading JSON, and that writing what it read settles after one round | the parse goldens |

They share a library (`fuzz/fuzz_targets/library.rs`) with every schema shape validation handles, plus 1.0's library functions and actions. The dictionary, `fuzz/openui.dict`, holds fragments of the language and of model output. Inputs stay under 8 KB, the size of a long response.

**The caps** (#10) make slowness and memory findings, not passes: an input over **2 s**, a single allocation over **512 MB**, or the process over **1 GB** stops the target and leaves the input. Every limit the core has (ADR 0001) bounds an input well under them, even in the fuzz build with its debug assertions. The long runs' slow inputs (18.5 s and 12.5 s against lang-core 1.1) replay in milliseconds now, and so do the 180,000 inputs the long runs' corpora had grown to. `scripts/fuzz-coverage.sh` sets the caps, so GitHub's runs and hunts have them too. The stream target caps its pushes because a statement that never closes is read again in full on every push, and a state default in it evaluated again up to the work limit (#22): the work limit for the whole text times the pushes stays within 16 million steps (about 0.2 s in the fuzz build, a tenth of the timeout), with at most 128 pushes. With the cap, a timeout still means a push costing more than it should. The hardening run (#19) found three looser caps too loose: 256 pushes (a 6 KB input whose stray quotes left a statement open took 0.8 s idle), then 128 (an 884-byte default left open and evaluated to the 1-million-step limit on each push took 0.9 s idle), then 64 million steps (a 1.2 KB default left open on each of 53 pushes took 0.85 s on the worker at load 6 and passed 2 s when it was busier).

**The checks** (`fuzz/fuzz_targets/checks.rs`), on every result of `parse` and `stream`:
- **Past the result-size limit, flagged.** An unflagged result's JSON stays within the limit plus half again plus 4 KiB. The limit is priced as the result is built, not by writing it, so it's approximate: over the 23,754 inputs of the parse corpus under six sets of limits, the most an unflagged result ran past was 16%.
- **A cut result stays bounded.** It keeps what fit, plus a placeholder for each node left (a dozen bytes for a node of a few bytes of program), so it may run past by 16 bytes a byte of program more, never with the amplification.
- **Past the depth limit, flagged.** An unflagged result nests no deeper than three JSON levels a level of the depth limit, plus 32.

The checks found two holes when they were written, both fixed with cases: validation errors and unresolved names counted against the result-size limit without ever cutting (a list of `""` where `Text` items are expected made 45 bytes of errors a byte of program, unflagged), and strings priced at their raw length, where JSON escapes a control character in six bytes.

The fuzz build has no AddressSanitizer: the crate forbids unsafe code, and over hours the sanitizer's own memory grows without bound (GitHub's 5-hour runs stopped at 4 GB that way). The working corpus (`fuzz/corpus/`) and the finds (`fuzz/artifacts/`) aren't tracked. Replay a find under the caps with `cargo +nightly fuzz run -s none <target> <file> -- -timeout=2 -malloc_limit_mb=512 -rss_limit_mb=1024`.

## Recorded streams

```bash
scripts/import-recordings.sh            # from the viewer's own folder
scripts/import-recordings.sh <folder>   # or any folder of *.recording.json
```

The viewer saves each live request it streams from Claude as a recording (the format is documented on `Recording` in `tools/viewer/Sources/ViewerKit/Recording.swift`): the request, the models, the effort, the library, and each text delta as it arrived. The command copies new recordings into `conformance/recordings/`, turns every recording there into cases (`reference/import-recordings.ts`), and regenerates the goldens:

- **A fresh recording** becomes a parse case, `recorded/<file>`, in its library, with a **recorded chunking**: the text cut where the deltas arrived, in code points. The reference computes that chunking's golden alongside the corpus's others, and the conformance test checks it at every step.
- **A follow-up** (an edit-mode request, which records the program it patched) becomes a merge case, that program with its text as the patch, and a parse case of the merged program, `recorded/<file>-merged`.

The request, the models, the effort and when it was recorded stay with each case, in `recorded.json` and `merge-recorded.json`. Recordings never hold a key. `tests/recordings.rs` checks that every recording is in the corpus as the command puts it there, and that a recording has only the fields the viewer writes. The two recordings there now are hand-made, to prove the path; real ones come from the viewer.

## OpenUINative's cost and the viewer's layout

### The runtime session's cost

`LiveProgram` calls its runtime session on the main actor and waits: every write or click evaluates the tree again and decodes it before it returns, so the drawing and a field being typed in never disagree. Measured on an M3 Pro (#59), the Swift package and the Rust library both built for release, 300 writes each, each forcing a full evaluation:

| Program | Program text | Tree JSON | Write and redraw, median / p95 | Worker's evaluate | Swift's JSON parse |
|---|---|---|---|---|---|
| `bench/dashboard` | 3.5 KB | 8 KB | 0.21 / 0.44 ms | 0.06 ms | 0.08 ms |
| `synthetic/dashboard-x2` | 7.4 KB | 17 KB | 0.39 / 0.77 ms | 0.10 ms | 0.15 ms |
| `synthetic/dashboard-x4` | 15 KB | 33 KB | 0.72 / 0.90 ms | 0.19 ms | 0.31 ms |
| `synthetic/dashboard-x8` | 30 KB | 66 KB | 1.43 / 1.65 ms | 0.39 ms | 0.63 ms |

A single round trip (`isPristine`, `getField`) costs about 5 µs. The cost grows linearly, about 22 µs per KB of tree JSON, and about three quarters of it is Swift's: parsing the JSON and decoding it into elements. Half a 60 Hz frame (8 ms, leaving the rest to SwiftUI) is reached at about 350 KB of tree, roughly a 160 KB program shaped like the dashboard; the core caps programs at 1 MiB, about 50 ms. Models write programs of a few KB, so evaluation stays synchronous.

Moving it off the main actor would need, to stay correct: each input keeping its own draft text, so keystrokes typed during an evaluation are never overwritten or lost; writes sent to the worker in order, each against the evaluation that was on screen, and a result that arrives after a newer write thrown away; a click resolved against the tree that was drawn when it happened, not the one that arrives later; and the decoding moved off the main actor too, since it is most of the cost. The session's own handles already go stale on every evaluation, so a stale click there runs nothing.

### The viewer's layout loop

On the commit before #43 (13e3ee0), the scripted run (`VIEWER_PLAY=bench/dashboard VIEWER_CHUNK=1`, release build) died with "more Update Constraints in Window passes than there are views in the window" for `bench/dashboard`, `synthetic/dashboard-x2` and `-x8` (not the gallery). The exception's stack runs through SwiftUI's split view child controller updating its minimum size: the drawing was the vertical split's pane itself, with a minimum height and no minimum width, so its minimum width followed what it drew, changed as text wrapped, and each change asked for another layout pass. #43 put the drawing in a horizontal split beside the store panel with `minWidth: 200`. Built from this branch with the drawing back as the vertical split's pane and no minimum width, the same runs died again; with either the minimum width or the horizontal split, they ran clean (three runs each). So #43 fixed it by giving the pane a minimum of its own, without meaning to; a comment at that frame in `PlaybackView` now says why it must stay. A test in the viewer's suite, hosting the same views in a window that size and playing one code point a chunk, did not reproduce the loop even with the old layout, so there's no regression test: the scripted run in `scripts/viewer-timing.sh` is what shows it.

## Android

```bash
scripts/android-test.sh                    # the running emulator, or the only device
scripts/android-test.sh --serial <serial>  # one of several
scripts/android-test.sh --abi x86_64       # an x86_64 emulator, as on GitHub
```

Cross-compiles the test binaries for 64-bit ARM (or `--abi`'s) with cargo-ndk, pushes them to the device with the conformance data (about 21 MB, to `/data/local/tmp/openui-rs/`), and runs each there with `OPENUI_CONFORMANCE_DIR` pointing at the copy, live diffs off, and the depth tests on a 512 KB thread stack (`DEPTH_STACK_KB`). It needs the Android SDK and NDK, cargo-ndk and the `aarch64-linux-android` Rust target, and a device already running. Two test binaries stay on the host: `report` checks CONFORMANCE.md in the source tree, and `cli` runs the host's command-line binary. Each binary's log is left in `target/android-<test>.log`. It isn't part of the check, and hunts don't run it.

## iOS

There's no script for iOS yet. The suite was run by hand in an iPhone 16 simulator (iOS 18.6, 2026-11-11): the test binaries built with `cargo test --no-run --target aarch64-apple-ios-sim -p openui-lang`, each started with `xcrun simctl spawn <device> <binary>` and `SIMCTL_CHILD_OPENUI_CONFORMANCE_DIR`, `SIMCTL_CHILD_OPENUI_NO_LIVE_DIFFS=1` and `SIMCTL_CHILD_DEPTH_STACK_KB=512`. All 22 passed; `report` and `cli` stay on the host, as on Android.

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

## Expression evaluation

`cargo test -p openui-runtime` checks the expression interface through the
runtime crate. `reference/expression-interface.ts` runs the pinned upstream
evaluator with supplied state, references and extra scope; its saved outputs
are in `crates/openui-runtime/tests/expressions.json`. Regenerate them with:

```bash
bun reference/expression-interface.ts > crates/openui-runtime/tests/expressions.json
```

The tests use public interfaces. `allowances.rs` checks shared work and room,
input credits, AST JSON byte edges and default scaling. `nesting.rs` covers
references, fresh nested evaluations, unwind restoration and stack ceilings.
`session_reentry.rs` resolves 4,000 statements through the session itself,
including raised depth, shared work, loop bindings and cached names.
`reference_allowance.rs` checks that wrapped computed values cannot buy credit,
and that external reference data needs its size prepaid. `lookup_scope.rs`
checks root evaluation from lookup callbacks and session re-entry. Pinned
upstream cases cover statement lookups inside and outside loops, lookup order,
and RuntimeRefs inlined by parsing. `parse_policy.rs` compares nested parsing,
JSON and expression reading, and streaming against top-level results during a
16,000-statement evaluation. Earlier segments must not spend these entries'
stack allowances. Mixed session, callback and fresh hops count the same depth.
Deferred-AST tests bound sizing by remaining work, including a shared DAG with
2^40 virtual leaves. Larger deferred ASTs can require an explicit work limit.
`supplied_values.rs` checks undefined, typed values and identity.

`supplied_memory.rs` runs under a 256 MiB allocation cap. Bounds include the
supplied input and the live result; they are for these shapes, not a universal
multiple for every expression. For 900,000 distinct empty arrays, direct
pass-through, cuts, Count, First and element-prop reads peak below 2× input;
`@Each($x,"i",i)` stays below 4×. Over 900,000 numbers, Sum stays below 1.1×,
and Round and a missing-field pluck (`$x.a`) stay below 2.5×. Object coercion
over 100,000 fields stays below 2×. Tests check the returned values too.

`store_reference.rs` compares the pinned store's snapshots, identities, reads,
notification/version mapping and pristine behavior after every call. The
reference probes pristine through initialization; Rust reads `is_pristine`
directly. The fixture has directed cases, four seeds generating varied names
and nested values, typed ASTs, and three real recordings under three persistence
assumptions. Rust also parses every recorded chunk before initializing. Three synthetic
ASCII programs stream one byte at a time under the same three persistence modes,
covering non-null scalar/container defaults, user edits, rewinds and expressions.
A bidirectional id map checks snapshot/read container identity across steps;
parsed inputs are checked structurally and for aliases within each result. One
separately named case pins the nested-undefined representation divergence.
Regenerate from `reference/`:

```bash
bun --no-install store-interface.ts > ../crates/openui-runtime/tests/store.json
```

The fixture stores one call and its observations per line, with shared value
and snapshot pools; its header names the observation positions. `store.rs`
tests the individual rules, snapshot COW and shared container bodies, including
50,000 nested arrays on a 64 KiB stack. `test_alloc::allocation_calls` counts
allocation/reallocation requests, including temporary allocations: unchanged
initialization of unique names and ignored scalar sets allocate zero times
with 1 and 4,096 names, even with a retained snapshot. A real scalar set without a retained
snapshot also allocates zero times. Both store binaries have 64 MiB caps.

Store admission uses the retained `Limits` result-size ceiling (ADR 0005).
Initialization returns accepted-change status plus per-name refusals, so one
rejected entry does not discard independent accepted values. The total-state
count includes pristine-only names even when they have no snapshot entry.

Tree conformance runs with `cargo test -p openui-runtime --test tree_conformance`.
It covers the entire parse corpus plus `conformance/runtime.json`, both state
scenarios, and every streaming step. `RUNTIME_CASE=<substring>` limits a local
investigation; the normal suite checks every case, pending reason, divergence,
chunk plan and the runtime counts in CONFORMANCE.md. `OPENUI_NO_LIVE_DIFFS=1`
keeps diagnosis offline without starting Bun. `UPDATE_REPORT=1 cargo test -p
openui-runtime --test tree_conformance` refreshes the separate runtime counts
block after a successful comparison. To show one reference result:

```bash
(cd reference && bun --no-install runtime.ts show runtime/objects defaults 1 20)
```

`tree_rules.rs` checks the public tree interface directly: per-value ordering,
static/container sharing, raw fallback and exact errors, structural AST data,
undefined, shared work and result room, and state credit only on read. Value
statements are already inlined; caller-supplied statement tables are absent.
A 200,000-element tree and expression cut on 64 KiB stacks; raised depth admits
a 1,000-expression chain. A combined tree/expression path cuts even when each
part fits alone. Widths 128/256/512 require allocation calls to grow by at most
2.2x on doubling. A 50,000-number state list stays shared and peaks below twice
the input heap. This binary has a 128 MiB cap. A 32 KB component name under a
40 KB result limit gets past the header and evaluates a preceding prop, then
cuts before allocating its large diagnostic; peak remains below 3x input.

`tree_supplied_memory.rs` has a 256 MiB cap: 900,000 distinct empty arrays pass
through by identity below 2x input heap, and a 64-level shared DAG compares by
identity without expanding its bodies. There is no runtime diagnostic scan.
The same binary distinguishes object index coercion from array indexing.
These bounds cover those shapes rather than every possible supplied value.

`tree_concatenation_and_cuts.rs` (256 MiB cap) compares the core-backed runtime against
`reference/tree-interface.ts`, using explicit persisted values for the reviewer's
five concatenations, coercion failures, and descriptor cases. A deterministic
work bound grows linearly for 32/64/128/200 concatenated terms. Work, room and
depth edge cases retain fitted props, raw remaining props and earlier errors.
The corpus binary has a 256 MiB cap. Its pending cases also compare independent
one-shot props, nested elements, root metadata and snapshots. Its only message
normalization is structurally restricted to the named map-target divergence.

The core's new `expression_failures.rs` uses only public APIs. It pins typed
failures, immediate stop at work edges, callback propagation, sequential reset,
uncached failed lookups, extra-scope failure before state fallback, descriptor
laziness and loop bindings, caller work/room/depth, fresh nested cuts and unwind.
The caller depth closure runs 200,000 requested hops on 64 KiB; logical depth
stops it first. Parser tests and goldens are unchanged.

Regenerate the focused golden from `reference/` with:

```bash
bun --no-install tree-interface.ts > ../conformance/golden/runtime-interface.json
```

`bindings.rs` checks marker JSON, plain and transformed writes against current
state, JSON round trips, null extra-scope fallback, inherited target names,
literal strings, typed/data Assign parity, and refusal preserving snapshot,
version and pristine status. It pins exact room for plain and loop-produced
markers, assignment markers, and shared supplied marker bodies in separate
output positions, cuts one byte below, and retains the interrupted raw prop. Deep
expressions and large writes exercise bounded failure without partial state.
Declaration-scope reads have separate caches, restore on unwind, and retain
the same allowance. Query defaults read state without the field-value overlay. Extra scope reads
use object key coercion and inherited members; state keeps exact Map keys.
Failed field writes compare TypeError messages and post-write trees with the
reference, and preserve snapshot identity, version and pristine status.
The function-valued-write divergence pins both outputs at every write step.
The non-string-target divergence pins entire write records, including markers,
refusal errors and both post-write observations. Refusals check every existing
name's pristine status without pretending a non-string target is a string.
Overlapping element/marker data cases pin expression-result priority separately
from raw-marker priority in state, member, query, ternary, list and inline paths.
Signature tests use structural names, live Zod marks and explicit ambiguous
annotation cases. `BindingProps::undecided` exposes every unresolved pair in
four live Zod specs; tests check all true marks are recovered or reported and
that explicit insertion clears its diagnostic.
`prop_projection.rs` exercises the general pre-projection scope hook, with
marker interpretation entirely in the runtime. It checks typed and data-shaped
results, state-only reads, callback opt-in, one substitution per result,
unchanged and identity hooks with identical room/work/depth, and view/lookup
ownership inside foreign sessions. Replacement output spends the normal
allowance, including borrowed values, nulls, large strings and deep containers.
Hooks skip failed/cut results and view operations stop after a cut.
The hook's views cannot escape their owner lifetime, enforced by a compile-fail
doctest. Markerless string fields pin main's exact work: 781431 for eleven
60000-byte strings and 1200241 for one 400000-byte string in the Box fixture;
one unit below cuts, and the eleven-field case fits one million work.
Public APIs exercise the state-only lookup and one-read rule. `reuses_data`
exposes read-only provenance, with a compile-fail
doctest rejecting mutation and existing result-equality tests unchanged.
Assignment stripping pins main's 292 room/338 work with bindings, and
165/179 after stripping without bindings. Eleven repeated data markers fit
main's 3030/5946 baseline
and now pin 214/655 exactly. Each boundary cuts one unit below.

The corpus also replays seeded `.writes.json` sequences through public marker
writes; missing sequences fail, and pending cases keep partial comparisons.
Action execution now has separate renderer interaction scenarios, described below.
The core's existing suite still runs by itself with `cargo test -p openui-lang`.
The unchanged runaway-stack unit test also runs under release: its assertion
is enabled for unit-test builds while production retains the existing
debug-only guard. Before this test-only fix, the same release test failed on
`main` because `debug_assert!` was compiled out.

`evaluator_ownership.rs` captures a session's root evaluator inside a foreign
session's state callback, with both work-limit directions and with/without an
enclosing caller level. AST evaluation, descriptor evaluation, result and value
reservations, flags and an intervening unwind must all use the owner's account.
Direct `reserve_value` tests check work, room, depth, cumulative reservations and
absence of state-input credit. Tests formerly using removed `charge_work` and
session-level descriptor evaluation now use caller levels and the callback
evaluator respectively, retaining their assertions.

Pending cases compare in-scope errors as well as independent props and snapshots;
`runtime/pending-error` supplies a failing prop beside a deferred action. All
runtime divergences pin complete reference and Rust one-shot observations
under both state scenarios, independently of the regenerated goldens.

The descriptor matrix starts its streaming root with a state prop, so the root
is already dynamic while structural element data is incomplete. Without that
prefix, the two parsers disagree on `hasDynamicProps` at an early partial
structural-element prefix (16-character chunk 6). Changing parsing is outside
this fix round; the direct one-shot name regressions are pinned separately.

`tree_reference_input_limits` (128 MiB): supplied reference bytes increase
scaled allowances before evaluation (including unread data) while fixed limits
still cut; large
reference lists retain identity below twice input heap; loop allocations grow
linearly; loop cuts retain earlier props/errors; cycles, nested loops and deep
supplied data cut on a 64 KiB worker stack. Tree conformance also pins caller
context through components and query defaults using explicit embedder state.

The per-item callback preserves the original JS value when it returns no
replacement. `tree-interface.ts` pins undefined array entries and own fields
through an unhandled Each item; `tree_concatenation_and_cuts` compares that reference result.
`tree_reference_input_limits` also checks that static Each elements avoid replacement room.

`action_allowances` (256 MiB) pins equal room thresholds for one through five
plan aggregation levels, one byte below each threshold, the ordinary descriptor
program's exact `main` work, and bounded buffering of 100,000 shared data steps.
It requires a described 30-field action and nested object/array/enum arguments
to fit default limits, checks linear work growth for a fixed schema, and cuts
wide enums, repeated required/default copies, deep metadata keys and expanded
composites. `action_definitions` compares complete trees and errors for all 268
reference observations of action metadata. `action_plans` checks separate-store
identity and revocation on disposal, including retained plans and forged raw
props retained by a cut.


`validation_sessions` pins default-limit evaluation of 100 row actions with
100 properties and 60- or 1,000-character descriptions. Marginal work is the
same with no descriptions, and scales with present fields and required names.
Prepared-check and positional-naming allocations are independent of absent
schema width. Computed keys are admitted before copying, malformed string
properties cut before numeric-index materialization, and cached required names
preserve explicit nulls even without a schema type. Foreign
sessions readmit metadata, and recursive required-name joins are bounded before
copying. The prepared-schema compile-fail example forbids mutation of the
borrowed schema. Existing hostile-schema cases remain cut and flagged.
`expression_transform_buffers` uses a 2 GiB allocation cap specifically to test
real 64 KiB and 1,000,000,000-byte limits. It measures live allocation peaks
within the vector limit plus 16 KiB of fixed evaluator/allocator overhead, and
requires interrupted transforms to return no exposed partial buffer. It does
not allocate the previous 1.69 GB excess: capacity admission cuts first. Nested
vectors share the ceiling, projected vectors are included, and discarded
vectors release credit. A prior transform must not change the work needed by
a later ordinary array evaluation in the same session.
`action_plans` names and checks invocation of a declared action from AST-shaped
props in element-shaped state and query data, followed by same-store plan reuse;
finished steps supplied as data are still rejected.

Library function tests use the public tree interface. `function_definitions`
compares 5,292 direct lang-core observations; `library_functions` covers
unregistered names, replacement registration, host unwind recovery, deferred
set expressions, returned data, and admission cuts. `function_allowances` uses
a 64 MiB allocator cap and pins exact work/room thresholds, shared schema work,
large host errors and shared DAG results. The other two use 256 MiB.
Regenerate their direct goldens from `reference/` with
`bun --no-install function-definitions.ts` and `bun --no-install function-results.ts`;
`bun --no-install runtime.ts golden` includes the 37 function corpus cases.

Nested integration binary layouts (`tests/<directory>/main.rs`) are forbidden
by scripts/check.sh before compilation; every supported integration binary
must be tests/<name>.rs and declare the counting allocator. Query registration
and large shared arguments are covered by query_default_calls, with a 256 MiB
allocation cap. Its definition-argument divergence is regenerated with
`bun --no-install function-undefined-arguments.ts` from reference/.

`action_execution` uses a 256 MiB allocator and the public runner/form APIs.
It pins click-time state reads and function calls, frozen query defaults within
a run, explicit redraws, exact result-size/work boundaries, shared work
for a long plan, stopping after failure/cut, reset/pristine streaming defaults,
event snapshots and legacy shadowing. It also checks retained and forged custom
authority, raw cut props, undefined field wrappers, save requests, streaming
defaults and inherited field names. Enumerated malformed inputs and deep state
run on a named 64 KiB worker stack.

`tree_conformance` replays the renderer interaction scenarios generated by
`reference/action-scenarios.ts` using the separately attributed
`reference/renderer-actions.ts` transcription. The compact field-value tables
retain full observations; every event, snapshot, pristine list, subsequent
tree/error and function-call count is compared as serialized bytes. Every
discovered plan must run; both outputs of the six small non-string-target divergences
are pinned. No pending case loses its existing partial tree comparisons.
Regenerate everything from `reference/` with `bun --no-install runtime.ts golden`;
`bun --no-install runtime.ts actions` refreshes only interactions and their
coverage. `bun --no-install renderer-action-lookups.ts` refreshes the 84 direct
legacy metadata/key lookup observations. None installs dependencies or runs tools.

`borrowed_output_checks` uses a 128 MiB allocator for the general core output
admission hook: exact room, one byte below, sticky cuts, repeated scalar work,
aliased-output JSON size, depth on a 64 KiB worker, and ownership across foreign
sessions. No existing core test or parser function changed. Reset targets use
this same hook so one step shares work while discarded write room is released.
Legacy string spread checks its output during construction, and large scalar
messages/fields are refused before copying; `action_execution` pins the
allocation bounds as well as the resulting cut flags.

The legacy lookup matrix includes UTF-16 string spreading with paired and lone
surrogates. `action_execution` also pins bounded UTF-16 index lookup in string
action definitions (without walking unrelated metadata), and exact text room
for supplementary Unicode scalars.

Retained loop expressions receive their measured input allowance in each fresh
run, with identical credit for typed and data ASTs. Mutation argument ASTs stay
borrowed until bounded evaluation; allocation tests refuse a large literal
without first copying it. Existing binding and draw sizing retain their prior
paths and thresholds.

`action_payload_admission` reproduces payloads past the 64 KiB floor, named-form
fallback, large legacy params/type coercion, all handoff kinds and the hard
ceiling. It pins that a 2 MiB snapshot emits before an oversized set fails,
that large property names earn credit before copying, and that undefined
snapshot properties consume no output room.
`action_run_budget` uses actual parsed 5,000/20,000-step set and reset plans.
Thread CPU ratios (below 6×), allocation counts (below 20 per step plus 2,048)
and release CPU time (below 2 seconds) reject repeated whole-program walks.
Its expensive-loop plan pins shared work and preservation of earlier writes.
A second CPU ratio crosses 5,000/20,000 program steps with 8/32 events over
80 KiB of state, catching program remeasurement or repeated admission per event.
`mutation_suspension` uses only public APIs for success/failure, live fields,
current-program resets/args, repeated and last mutations, shared work across
resumes, query invalidation and single-use/foreign/disposed refusals. It also
pins that index rebuilding and raw overlay preparation on resume spend work.
`action_query_defaults` checks call counts and that a costly unrelated default
cannot cut an event-only/reset/legacy/default click.
`sequential_evaluation_budget` pins exact work charges, released output room,
fresh lookup caches, sticky cuts and bounded supplied-input credit.
`javascript_string_units` pins the shared core iterator's paired/lone surrogate
and supplementary-scalar behavior. No parsing-path function changed.


Action cost regressions use real parsed plans, including 20,000 sets/resets and
5,000 distinct mutation handoffs. The 4,000/16,000 comparisons assert on counting
allocator calls, with minima of three CPU samples and a loose 10× CPU backstop.
They cover shared immutable program input across resumes and O(events × state)
payload copying; small CPU samples alone do not decide the linearity assertion.
`action_resume_inputs` and `supplied_input_accounting` enforce replacement input
credit, immutable identity checks and work that stays spent. Reset refusal tests
check values, pristine status and version, not just the final event list.


`action_supply_allowance` checks borrowed/reparsed/alternating shared programs,
raw/prepared references, fresh-supply reclamation, a single combined input floor,
and identical cut points across input forms. Input measurement is caller setup;
allocation comparisons permit O(resumes × program bytes) borrowed setup while
shared-program evaluation stays linear. Retained bytes are sampled with the
allocator's reset peak (current live bytes), after caller inputs/events are dropped.
`literal_result_limits` bounds refused final literal copies without changing the
price of fitting literals or the evaluation of intermediate expressions.


`unread_action_inputs` pins admission of unread large/deep references
and cuts only at the step that reads an unusable entry. `thread_safety` asserts
Send + Sync for the prepared input types and Store. `input_measurement_cost`
pins byte counts for parsed programs and shared AST subtrees and reports input
throughput separately from parsing; its allocation count catches per-node setup.
Default-limit action form comparisons cover both room and work cuts.

`action_reference_reads` uses a 1 GiB cap for ten-million-number input arrays.
It compares admission and scalar results against drawing
in a grid of large strings/lists, unread large/deep entries in both orders,
fresh/resumed runs and all four input forms. Refused entries cannot inflate
a later write's room. Its allocator bound requires a
shared million-number body to cost only a constant number of extra allocations
when aliased by 100/400/1,000 references, even after an unmeasurable entry.
`input_measurement_cost` also requires over-deep action parts to earn no input
credit; `action_reference_reads` keeps earlier writes when their evaluation cuts.

Runtime replacement accounting is pinned by `loop_output_accounting`: four
native/data exact room thresholds measured against 729567f, a deterministic
100,000-element work boundary, marker stripping and wrapped plain/plan output,
and 768 generated programs under 3,072 limit configurations. It uses a
512 MiB allocator cap. `replacement_reservations` uses 64 MiB and checks
nested loops, re-entrant callbacks, a caught host panic, a cut mid-item,
separately retained output, and memoized bodies without name caches.
`query_default_limits` uses 1 GiB for amplifying defaults near the shared 64M
work boundary; its room/depth cuts stay local until read. It pins four query
cycle divergences and their flags, plus lazy defaults during action runs.
`value_use_depth` uses 256 MiB for 748-deep values at different positions in
both syntax forms and both orders. `expression_transform_buffers` now runs
its 64 KiB / 1 GB peak bound for flat and three-level transformed arrays.
`query_default_calls` pins string allocation identity through ordinary,
member and extra-scope reads, undefined-key cleanup, returned argument aliases,
transformed text admission and call order before a malformed argument.

`cargo run --release -p openui-runtime --example accounting` bisects minimum
work and room per complete corpus case. Passing `persisted` uses the saved
input from the runtime golden. It reuses the corpus's function recipes and
binding declarations; measurement never times evaluation. Compare its TSV
output between revisions. Its allocator cap is 512 MiB.

## Runtime accounting measurements

The TSV files in `crates/openui-runtime/tests/support/` record deterministic
public-API observations, not elapsed time. `reuse-work.tsv` and `mixed-work.tsv`
have columns `seed`, `data`, `main work`, `current work`. Work is the smallest
work limit that finishes uncut; tests also require a cut one unit below it.
The generator, schema, callbacks, and supplied values are in `support/reuse.rs`
and `support/mixed.rs`. The depth fixtures have columns `kind`, `data`, `object`,
`loop levels`, `slot`, `first cut`; their baseline is the first isolated use,
without earlier reads priming a cache.

Produce a main baseline from `git archive db80c86`, extracted into a scratch
directory, and copy the current measurement example and its `tests/support/`
dependencies there. Use an independent `CARGO_TARGET_DIR` for that archive;
never share compiled artifacts across revisions. The following commands run
from each checkout, offline (redirect each revision to a separate TSV):

```sh
cargo run --offline --release -p openui-runtime --example reuse_measurements -- costs 9216
cargo run --offline --release -p openui-runtime --example mixed_prop_measurements -- generator 4096
cargo run --offline --release -p openui-runtime --example depth_boundaries
cargo run --offline --release -p openui-runtime --example depth_boundaries -- bare
```

For `reuse-work.tsv`, retain the fixture's existing seed/data pairs, join the two
cost outputs on those columns, and take the baseline and current work columns.
For `mixed-work.tsv`, retain seeds below 48 or divisible by 37, both data forms,
and take the same four columns. Do not update the baseline column from a branch
build. The depth command outputs are the complete respective fixtures
`depth-first-use.tsv` and `depth-first-use-bare.tsv` from the archived main build.
`position_depth_boundaries` checks these independent first-use boundaries at
same, shallower, and deeper positions; `shared_subtree_depth` additionally checks
that combined uses cut exactly when either isolated use cuts.

Other runtime measurement examples:

- `accounting [persisted]`: per-corpus-case minimum work and result room with
  default or corpus-supplied state/query data.
- `reuse_measurements`: the `costs` mode above records work, peak live bytes,
  cut status, output size/hash, and source; `loops` records loop allocation
  grids. Its other modes measure action depth and parameter/return behavior.
- `mixed_prop_measurements cases`: the large element/text witnesses, both forms;
  `generator` records work and output size/hash for mixed prop programs.
- `depth_boundaries [bare]`: isolated-use depth thresholds described above.
- `subtree_depth_measurements`: shared-parent/child uses alone and combined.
- `loop_peak_witness [unique] [room]`: peak live bytes, unique retained string
  payload, and output bytes for seven-level element loops.

`scripts/compare-runtime-reuse.py main.tsv current.tsv` checks the generated
output hashes and forbids increased work, and reports the allocation ratios
separately. Allocation ratios vary with leaf shape; the finite loop grid in
`evaluation_reuse` enforces 5 × room for 1,000-byte string leaves and 20 × room
for each of eight-empty-array and `{a:[]}` leaves, without a universal bound.

## Store admission sequences

Runtime `state_admission` pins exact JSON byte boundaries (escaped names and
aliased bodies included), pristine-only names, persisted duplicates, field and
binding writes, action sets, atomic resets and resumed writes. A 64 KiB worker
admits, snapshots, serializes, disposes and drops a 10,000-level value with a
raised depth limit, and refuses it under default limits. Allocation counters pin
admission cost independently of unrelated state names and its growth with the
written value. Both test binaries use a 128 MiB allocator cap.

`public_call_sequences` runs 3,000 seeded sequences of 32 public calls (96,000
operations) with small random programs, values and limits, including continuation
consumption and disposal. Refused single writes preserve snapshot identity,
version and pristine flags; snapshots remain within the configured state cap.
It is a deterministic sequence test, not a differential oracle for every possible
program or a general proof of panic freedom.

`limit_measurements` is a runtime example that prints 8,192 observations: 4,096 unique
programs in both native and AST-shaped data form with exact minimum work and result room. Its output can be
compared against a separate checkout to detect changed values, flags or prices.

`operation_edges` also uses a 128 MiB cap. It pins adjacent work/depth/room
boundaries for drawing, binding writes, named form writes, direct writes,
initialization and a write after mutation suspension, plus total-state edges.
Its 64 KiB worker drives a 10,000-level value through prepared references,
drawing/JSON, bindings, forms/defaults, action sets, mutation resume, event JSON,
snapshot, disposal and Drop. These are successful deep-walk tests; they do not
assert exact physical segment counts for every route.

## Native runtime binding (#41)

`openui-ffi/tests/runtime_session.rs` compares exported session calls with direct
Rust runtime calls: corpus binding-plain (defaults/persistence), binding-write-supplied
(query/mutation data), action-steps (every step, mutation true/false), action-loop-capture
(nested Each/arrays), run-forms (fields/defaults/save/events), streaming and program
replacement. Additional scenarios cover custom identity surviving a binding, JSON
copies losing it, legacy/default events, and current inputs on resume. Trees and
events compare exact strings, including errors and flags. Refusal edges cover JSON
bytes, program admission, action paths, pending outcomes, evaluator depth/work/room,
marker/form writes and typed capability errors. A 500-level supplied reference
with a cheap length read pins preparation independently of evaluation, including
custom preparation work edges and a refused replacement preserving old data.
The two formerly parked per-value/total Store-admission tests are active.
`runtime_state_cap` checks 1024/1023-byte edges for set, top-level/named forms,
bindings, action sets, atomic two-target resets, resumes against current state,
and persisted initialization. Refusals preserve snapshot/version/pristine;
partial batches report each refused entry and load independent entries. Another
case pins the fixed Store ceiling independently of scaled drawing room.

`runtime_threads_and_sequences.rs` exercises 16 threads × 200 state/action rounds,
32 disposal races, foreign-thread last-owner destruction and 16 deterministic
seeds × 500 exported operations under a 256-byte state cap. Every step asserts
snapshot size within the cap; refused calls and failed single action writes
preserve snapshot, version and all probed pristine flags. The run must exercise
Store refusals and check their typed cap reasons. Worker unit tests inject a panic (sticky typed Worker)
and synchronize an in-flight call with disposal; integration scenarios remain
exclusively on the exported API. `stack.rs` adds 10,000-level JSON and action
paths plus pending-run destruction on a 64 KiB caller stack. All binaries use
test_alloc. Run these in debug and release.

`cargo run --offline --release -p openui-ffi --example session_overhead` reports
20,000 direct/worker version reads after warm-up; it has no timing assertions.
Generated Swift/Kotlin signatures are checked by scripts/check-runtime-binding-api.py.
Native tests live in OpenUINativeTests/RuntimeSessionTests.swift and Android's
androidTest/RuntimeSessionTest.kt; compile/run them on the platform machines,
including native ill-formed UTF-16 conversion. See docs/runtime-binding.md.

### Ticket 41 independent-review regressions

`runtime_large_draws` compares 4097 plans (including execution of the last) and
2.2 million values / 4,400,132 JSON bytes against direct runtime JSON and flags.
`runtime_abandonment` isolates process-wide test_alloc::live_bytes measurement:
10000 abandoned mutations, 100 ~800 KB program replacements, cross-thread token
drops, cleanup barriers and zero pending slots, with flat tracked heap checkpoints.
Mapped stack segments and allocator account metadata are outside that counter.
`stream_output_memory` retains 51 unchanged 50 KB outputs and requires incremental
live bytes to equal their JSON plus at most 16 KB, with shared opaque program Arcs.
New session tests cover full-table completed/default/legacy runs, oldest-only
eviction, typed Evaluation failures and sticky Worker failures (the existing
unit fault injector triggers the otherwise unreachable panic). All integration
regressions use exported FFI APIs. Their red-before-fix logs live in target/.
Android tests exercise generated MalformedInputException rejection, a real
caller-side Utf16StreamBuffer and the JSON-escape route; they require Android.

Native session lifetime follows [ADR 0006](docs/adr/0006-native-runtime-sessions-own-a-worker-and-opaque-capabilities.md);
the Store admission rules remain [ADR 0005](docs/adr/0005-admit-total-state-incrementally.md).

## Shared sessions and WebAssembly (#42)

`openui-session` owns binding-neutral session policy. Existing ffi scenario assertions and expected JSON bytes
are unchanged; their direct runtime oracles unwrap the fallible exports. Its
program-sharing unit assertion lives with the owning fields in the session crate.
`openui-wasm/tests/runtime_session.rs` reuses all 28 native scenario/attack cases through a test adapter over the Rust-callable
`openui_wasm::session::Session`; all exported RuntimeSession operations forward
to that inner type one-to-one. `runtime_state_cap.rs` adds the same five boundary
cases; `runtime_abandonment.rs` checks 10,000 dropped outcomes. The test adapter
preserves the native test shapes, not another session implementation.

The inner type's unit tests exercise nested operation/disposal refusal and a
sticky caught-panic error on native Rust. A third unit test drops an actual
exported continuation token and checks release without allowing the following
operation to drain first; weak cleanup also remains safe after session drop. All public wasm calls are additionally
guarded in generated package glue (`js/package-guards.js`), including argument
conversion, free and `[Symbol.dispose]`; the guard is tested only on the built
package.

After `scripts/wasm.sh`, run `bun --no-install test crates/openui-wasm/js`.
`session.test.ts` drives nine scripts and attack/cap/reentry cases.
`lifetime.test.ts` checks that every free goes through the guard and that no
argument conversion runs application code (hostile `valueOf`/`length`/getters,
every member of every freed object). `errors.test.ts` patches `JSON`,
`Reflect`, `Error`, `TextDecoder`, `WebAssembly.Table` and an
`Object.prototype.kind` setter and checks results are unchanged and sessions
stay freeable. `package.test.ts` checks exports, the module's import allowlist,
pointer/private-helper hardening, the web build, and that mutated glue with an
uncovered member refuses to load.
`js/scenarios.json` carries full tree/event JSON strings, snapshots, versions and
form decisions generated from the public Rust runtime and checked against the
thin wasm adapter. It is a binding-seam oracle, not new upstream goldens:

```bash
UPDATE_WASM_EXPECTATIONS=1 cargo test --offline -p openui-wasm --test runtime_session javascript_expectations
```

Without that variable the Rust test verifies the checked-in expectation bytes.
No test uses a wall-clock assertion. The package suite requires the built node
package; it deliberately fails to import when the toolchain build was omitted.
The comparison page's README lists browser/typecheck/parser-comparison commands.

## Runtime adapter and nested-substitution pins

`nested_loop_memory` (512 MiB cap) measures additional Rust heap for a unique
1,000-byte leaf at depths 1–8 with one/three items, typed AST and shared descriptor
data. Each one-item case must stay below 128,000 bytes. The support files
`nested-loop-oracle.tsv` and `nested-loop-prices.tsv` were generated independently
against archived 8ffe3b0: 32 output-hash/cut observations and 16 exact work/room
threshold pairs. Changed results or prices fail, including cut cases. The test
does not assert a universal flat memory bound; active templates still cost space
with depth. Existing `evaluation_reuse` retains its 384-configuration leaf grid
and its 5/20-times-room shape bounds.

`temporary_array_semantics` (128 MiB cap) pins an otherwise uncut eager temporary:
32–32,768 opaque host strings, followed by a callback choosing an array index
after all producers ran. Only one leaf reaches the result. The test records heap
peaks without a minimum-peak assertion that would forbid future compaction. It is a compatibility counterexample
to a universal peak/room formula (ADR 0008), not an amplification safety test.
`peak_measurements` continues to expose the unresolved 2,652-row read/unread
`Each(L,i,L)` peaks.

`admission_helpers` (64 MiB) pins adjacent serialization bytes, bounded refusal
before copying caller-edited results, incremental/atomic configuration caps,
prepared-token cuts and finite reference-combination iteration. Bulk preparation
uses input-linear key-ordering scratch; a refused tail does not allocate an error
for every remaining key. `read_bounds` checks allocation-free impossible-name
lookups and input-linear shallow form reads. `owner_walks` (128 MiB) exercises
Debug/equality/serialization, snapshot, event, pending-plan, continuation and
prepared-token last-owner release on a 64 KiB stack. Existing `operation_edges`
covers the other evaluation/write/resume/default/preparation deep routes.

`deep_callback_walk` (128 MiB Rust-heap cap) runs a real 34,000-level reference
callback chain on a 64 KiB thread. Each callback reserves an uninitialized 64 KiB
frame so optimized builds also cross 1,024 stack segments without initializing
gigabytes of padding. At the bottom, it walks and serializes 100,000 AST nodes.
The archived implementation panics at 1,174 nested segments; the relative base
passes. Stacker's mapped segments are outside the Rust test allocator.

All allocator assertions use deterministic byte/allocation counters, never
wall-clock cutoffs. None establishes a global peak bound for arbitrary host
computation, retained provenance history or unprojected eager temporaries.

## Default export and configuration compatibility

`default_export_compatibility` (512 MiB allocator) pins the 8,388,616-byte
uncut draw envelope, 8,392,146-byte raw cut draw, a raised-depth draw, empty
snapshot at room 0/1, ordinary large prompt libraries and control-character
binding/function names. Explicit admission remains covered by `admission_helpers`.
`prepared_reference_compatibility` (512 MiB) pins unmeasured opaque preparation:
40,000-deep tokens have uncut shallow reads on 64 KiB; explicit preparation still
refuses depth. Nine-MiB strings and five-million-item arrays are not value-size
admission failures. Neither default nor explicit preparation promises an 8 MiB
per-token value cap. Default reference combination also retains all prepaid
contributions when evaluation limits are raised; explicit limited iteration
remains bounded.

Both bindings' `runtime_session` suites exercise these public-surface rules:
`tree_exports_over_the_host_input_cap_replace_draws_without_consuming_pending_runs`
returns a tree larger than the host-input ceiling, stales the previous draw, and
keeps pending mutations and state intact. `empty_snapshot_exports_at_zero_and_one_byte_store_ceilings`
pins `{}` without changing state or the current evaluation.
`binding_annotations_keep_the_raw_host_byte_budget` accepts JSON-expanding names
while retaining the original raw annotation budget.
`draw_exports_keep_the_runtime_ceiling_and_envelope` covers the exact default
ceiling, raw cut props and a 12,312,139-byte draw with raised result ceiling.
`prompt_configuration_accepts_ordinary_libraries_independently_of_evaluation_depth`
checks five ordinary large spec shapes and evaluation depths 0–8.

The session unit test `continuation_generation_refusals_precede_effects_and_consumption`
seeds the counter at exhaustion: run/resume refuse before writes or table removal,
and abandoning an existing outcome remains possible. Neither binding introduces
another policy or exported interface. Existing fitting JSON oracles, including
wasm's checked-in `js/scenarios.json`, remain unchanged.

Session callback and follow-up checks (#56–#60):

- `openui-ffi/tests/runtime_functions.rs` compares callback JSON to direct Rust,
  pins thrown-message text, same-thread callback reentry, panic containment,
  guarded foreign destruction, defaults cached across mutation resumes, and
  function failures producing null while the action continues like direct Rust.
- `runtime_callback_concurrency.rs` uses gated schedules for registration and
  disposal during callbacks, long callbacks with direct Rust output comparison,
  worker reuse and destructor reentry. Its seeded stress runs 8 seeds, 4 callers
  and 40 operations per caller, mixing slow callbacks/destructors, replacement,
  disposal and calls. A child-process watchdog detects stalls; tests assert
  results and bounded progress, never execution speed. Other-thread calls queue.
- Both bindings' `runtime_followups.rs` pin element-shaped state/query/tool data,
  custom identity refusal, expression-kind literals, opt-in path bounds and
  seven upstream data observations plus 82 streaming EditableTable prefixes.
- `openui-ffi/tests/retained_root.rs` is isolated for a representative heap
  measurement and release check; `openui-runtime/tests/owned_references.rs`
  compares owned/borrowed preparation bytes and flags under independent limits.
  The wasm follow-up test also bounds repeated-draw allocations with 2,000 unread
  references, detecting accidental remeasurement without wall-clock assertions.
- Generate native bindings into a temporary directory, then run
  `python3 scripts/check-runtime-binding-api.py SWIFT_FILE KOTLIN_FILE`.
  Swift `RuntimeFunctionTests.swift` and Android `RuntimeFunctionTest.kt` cover
  callbacks and data-backed actions; `RuntimeSessionTest.kt` covers UTF-16 recovery.
- With the wasm target and matching wasm-bindgen CLI installed, run
  `scripts/wasm.sh` (both package load checks), then
  `bun --no-install test crates/openui-wasm/js`. `functions.test.ts` pins JSON,
  throws, all-method reentry, query default call counts, data plans and path caps.
- Regenerate the added observations from `reference/` with
  `bun --no-install session-data.ts`, or use `scripts/regen-golden.sh`.

Synchronous callbacks require the host to return for progress. The native long-
callback test deliberately exceeds the removed timeout and asserts only output.
The JavaScript suite additionally pins throw-text coercion failures and action
continuation after function faults. A blocked JS thread cannot run a watchdog
in that same realm; no preemption or timeout is claimed.

`node --test examples/next-compare/scripts/copy-wasm.test.mjs` checks copying an
existing built package and the clear missing-build error without invoking Rust.

## Parser materialisation regressions

`parser_materialisation` uses public parsing calls to check nullish element
flags and JavaScript truthiness, shadowed `Each` in values and expressions,
all twelve inherited schema names, required-only entries, null-prototype
literals and call arguments, validation spread copies, and the inert prototype's null parent. The parser corpus adds `parser/dynamic-*`,
`parser/shadow-*`, `parser/inherited-*` and `parser/descriptor-extra-keys`.
The existing one-shot, streaming, stage and runtime conformance tests pick up
these cases without a separate pending list.

`scaling::alias_references_at_raised_limits_allocate_linearly` counts allocator
requests for references to distinct chain positions (2,000/8,000 aliases at
depth 20,000), and 100 × N reaches of the head (50/200 aliases at default depth).
Both use 1 GiB result room and must finish uncut with ordered unresolved names
and no orphans. Fourfold growth permits fewer than five times the allocations;
peak requested heap stays below 16 MiB. The unit companion
`materialize::cost_tests::alias_metadata_walks_scale_with_statements_and_references`
counts actual error/unresolved/reach traversal steps under those same public
parse calls, with a fivefold ceiling. Its instrumentation is compiled only for
unit tests and does not change evaluator work or parser limits. Deleting the
singleton unresolved-sharing arm in `fold` fails this counted regression.

`scaling::cut_alias_references_keep_heap_proportional_to_result_room` keeps a
300-alias/30,000-reach program fixed and varies result room across 64 KiB,
256 KiB and 1 MiB. Each parse must retain its size/depth flags and fit below
2 × room + 9 MiB of requested heap, including the program's parsed storage.
These are finite shape checks, not a bound for every program or a measurement
of stacker's mappings. None of the new tests asserts elapsed time.

Reproduce the focused upstream observations offline from `reference/`:

```sh
bun --no-install parser-materialisation-probe.ts
```

`alias_limit_results_keep_their_main_bytes` pins 72 combinations of both shapes,
depth 32/256/20,000 and room 64 KiB/1 MiB/1 GiB against archived main
`6367d530` JSON fingerprints. `cached_depth_cuts_respect_open_paths_depth_and_stream_changes`
pins three full main outputs, including an ancestor cycle, a changed starting
depth and required-prop error suppression; it also checks streaming cache
invalidation. The unit test `repeated_depth_cut_aliases_walk_metadata_once_per_reach`
uses 50/200 aliases at depth 32, with 100 × N reaches and 1 GiB room, and limits
traversal visits to 300 × N + 200. This catches disabling depth-cut reuse even
though the serialized reference charges remain unchanged.

`replayed_depth_cuts_keep_their_flags_on_later_stream_results` checks a completed
alias chain after two unrelated streaming statements, so a fresh walk cannot
mask a lost replay flag.

`depth_cut_containers_keep_fresh_identities_at_each_reach` pins main's container
identity on cut results. Only null cuts may be cached. For raised depth, a
separate iterative alias index certifies acyclic tails ending in a missing name;
per-frame successor marks skip slots already reached. The full original
32,000-alias witness, depth 20,000 and room 1 GiB, must stay below 48 MiB counted
heap and retain its 292-byte JSON cut result (293 bytes with the CLI newline).
The saved main result was captured with 1 MiB room; both rooms exhaust before
any missing name is reached, leaving the same empty unresolved/orphan lists.
`raised_depth_cuts_over_distinct_aliases_scale_with_the_program` counts visits
at 600/2,400 aliases with depth N/2 and permits less than fivefold growth.
`raised_alias_cuts_preserve_reference_prices_at_each_boundary` pins 552 archived
main byte/flag observations across value/expression mode, distinct/head reaches,
depths 257/280/300 and byte-price boundaries, then checks streaming finishes.
`raised_alias_cuts_keep_failed_lookup_marks_flags_and_required_checks` pins
three additional full main results: the lookup whose price fails still counts
as reached, a size failure stays flagged when no orphan can expose it, and
virtual depth cuts suppress false required-prop errors.

`empty_memoized_error_records_preserve_later_reach_suppression` pins twelve
archived-main results at depth 8/32/256, with the cut and uncut reaches in both
orders. An empty memoized error record still marks its statement as reported;
omitting it would expose a later validation error. One-shot and streaming finish
must retain the saved bytes and flags. Only empty non-memoized records can be
discarded without this side effect.

`streaming_aliases::replayed_depth_cuts_keep_reported_markers_after_a_late_alias`
and `replayed_default_depth_cuts_keep_reported_markers_after_a_late_alias` pin
zero errors through both pushes and finish in the review's depth-5/default-depth
examples. `late_alias_stream_results_match_fresh_prefixes_at_small_limits`
generates 13,500 programs over nine depths, five alias-chain lengths, ten depth
positions, three root shapes and ten late statements/rooms. Each complete-line
push and finish must equal a fresh parse, including both limit flags. These
programs also match archived main at every push, finish and set in the offline
differential; no elapsed-time assertion is added.

`parser_prototype_divergences` tests the separate twelve-case corpus in
`conformance/parser-prototype-assignments.json`. Complete results and all pushes
after `$v` is assigned must retain archived main's bytes. Every prefix and finish
is pinned, alongside upstream outputs generated by
`reference/parser-prototype-assignments.ts` during `scripts/regen-golden.sh`.
Earlier prefixes can still differ from main under the intended #52 ordinary
inherited-validation fix. These intentional divergences use explicit expected
results, not a pending list. `discarded_prototype_assignments_keep_value_equality_and_debug`
additionally protects public equality, Debug output and the non-dynamic flag.
