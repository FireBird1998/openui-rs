# Testing

The core is correct when it does exactly what the reference does: `@openuidev/lang-core` at the pinned commit (see [CONFORMANCE.md](CONFORMANCE.md)). Most tests compare the core's output with the reference's, saved as goldens, so they need only Rust: no Bun and no network. The rest check what conformance can't: no input crashes the core, deep or huge input is cut off safely, streaming stays linear, and the binding hands on exactly what the core produces.

This guide covers what each test catches, how to run it, how to add a case, and how to read a failure. The testing phase ([spec](.scratch/testing/spec.md)) adds sections as its tickets land: hunts, coverage, mutation testing, coverage-guided fuzzing, invariants, recorded streams and Android.

## Running the tests

```bash
scripts/check.sh
```

The check runs formatting, the Python worker-supervisor tests, clippy, the docs and every Rust test, including the doc examples and this repository's README example. It then builds both WebAssembly packages and loads them through their guard (`scripts/wasm.sh`), and runs the package tests (`bun --no-install test crates/openui-wasm/js`). Last, it builds the workspace on Rust 1.88.0, the oldest version the crates claim. The WebAssembly step needs the `wasm32-unknown-unknown` target, the wasm-bindgen CLI at the version `Cargo.lock` names (`scripts/wasm-bindgen-version.sh` prints it) and Bun; the old-Rust build needs that toolchain. Each is skipped, with one line saying what is missing, when its tools aren't installed; the check never installs one. With `OPENUI_REQUIRE_WASM=1`, as CI sets it, a missing WebAssembly tool fails the check instead. It takes a few minutes.

```bash
scripts/check-viewer.sh                                   # OpenUINative's, the viewer's and the chat app's Swift tests (needs Xcode)
cargo test -p openui-lang --test conformance              # one test file
cargo test -p openui-lang --test conformance streaming_   # tests whose names match
```

For **any change to the native binding**, run the Kotlin instrumented tests on
an Android device or emulator (API 34+, including arm64). This exercises JNA and
ART's thread attachment, which host Rust tests cannot cover. With the Android
SDK/NDK, Rust Android target, cargo-ndk, Java and Gradle already installed:

```sh
scripts/android-demo.sh
cd android-demo
"$GRADLE" --no-daemon connectedDebugAndroidTest
```

Set `GRADLE` to the installed Gradle 9 executable, as used by the build script.
Run both `RuntimeFunctionTest` and `RuntimeSessionTest` (the command runs all
instrumented tests). For Swift, with Xcode installed:

```sh
scripts/swift-binding.sh
(cd swift/OpenUINative && swift test)
```

The #65 native regressions use public session operations. The unit test
`deep_callbacks_run_on_the_callback_threads_original_stack` measures ordinary
thread stacks at startup with the core's existing `stacker` dependency (test-only
in the binding), then draws shallow, 12,000-level and shallow programs. Where
stack measurement is available it requires the deep invocation to originate
outside the worker's ordinary stack, while every host invocation stays on the
callback thread's original stack. `accepted_foreign_releases_use_the_original_callback_stack_on_replacement_and_worker_unwind`
checks accepted foreign release on that stack, including after a worker panic.
`queued_registration_destruction_after_worker_failure_preserves_ancestry` gates a
worker panic after a nested public registration is queued, then checks destruction
on the submitting callback thread's ordinary stack and refusal of both the owning
and submitting sessions.
`cross_session_cycles_refuse_ancestors_and_restore_scopes` and
`nested_replacement_and_disposal_destructors_refuse_ancestors_even_when_they_panic`
check nested calls and destruction. `many_sessions_join_callback_threads_on_disposal_and_final_drop`
uses callback-thread TLS exit counters for 48 simultaneously registered sessions,
including sessions never evaluated; disposal/final drop must have joined each
thread before returning. The #73 unit test
`disposal_and_final_drop_complete_callback_thread_joins` observes a callback-thread
TLS destructor for 64 sessions split between disposal and final drop. The exit
witness is written by the terminating callback thread, after its registry and
exit guard are destroyed. The test-only TLS destructor yields before its final
witness to exercise late teardown scheduling; no assertion uses elapsed time.
Existing all-operation reentry and exact Rust result,
call-count, throw and continuing-action comparisons remain in force.
The Kotlin `deepFunctionsReuseOneThreadAndReplacementReleasesTheOldImplementation`
test counts generated registry entries by identity for only its own implementations,
while an unrelated registered session stays alive. It checks release without GC
or elapsed-time assertions; a process-wide size count would fail its initial zero.
The #67/#72 unit regressions use exported registration/evaluation/disposal operations
with session-local infrastructure fault injection, release observations outside
caught destructors, ordinary-stack measurements and thread-exit counters:

- `refused_registrations_release_on_the_caller_without_starting_a_callback_thread`
  checks unknown names, name-byte and registration-byte bounds, unchanged state,
  existing implementation output/flags, caller release and zero spawned threads.
- `callback_start_and_registration_failures_release_once_on_the_fenced_caller`
  injects registration-send and pre-transfer receive failures; release
  must complete once on the caller, and sticky Worker errors survive disposal.
- `a_failed_first_callback_thread_start_preserves_the_session_and_registration_room`
  injects repeated spawn failures and checks typed Worker errors, exactly-once
  fenced caller release, unchanged snapshot/version/pristine flags, evaluated
  handles, output bytes/flags, registration room and successful later registration.
- `a_reclaimed_handoff_replies_with_an_error_and_keeps_serving_callbacks`
  injects an empty handoff, checks the explicit Worker reply, then evaluates and
  replaces the original implementation on the same callback thread.
- `lost_registration_acknowledgements_release_each_owner_once_and_final_drop_joins`
  covers first/replacement handoffs, failure before/after ownership transfer,
  exact release counts and final-drop joining after callback death.
- `a_caller_unwind_reclaims_its_queued_implementation_before_handoff` injects
  a caller unwind after submission, while the worker is gated. Its caller guard
  must release immediately under the fence; the already admitted missing handoff
  then revokes the session with `host function released before registration`
  and disposal joins both threads.
- `callback_transport_death_is_a_sticky_worker_error_and_disposal_joins_both_threads`
  injects invocation/release send/receive failures, checks the active call and
  later non-function operations return Worker, and joins both threads.
- `an_idle_callback_thread_death_revokes_calls_that_do_not_invoke_functions`
  stops the callback thread independently of mailbox error handling; its exit
  guard must revoke even version/snapshot/write/reference operations.

The #73 table-driven test
`registration_preserves_main_errors_precedence_implementations_and_room` compares
22 scenarios with `crates/openui-ffi/tests/support/registration-main.jsonl`, captured
independently on main f641ded. It pins typed errors and precedence, tree bytes and
flags, callable replacements and remaining room through subsequent public
registrations and a replacement probe after reaching capacity. Tight byte limits that cannot admit a program pin that program
refusal too; larger byte/count-capacity cases check callable implementations.
With unchanged limits a previously admitted name cannot later exceed its own
name-byte bound; the smallest admissible registration and its replacement are
both pinned at the exact byte boundary.
`live_admission_queries_preserve_room_and_match_real_registration` exercises the
shared query repeatedly on empty and populated cores, including byte/count
refusals and replacements at capacity.
The idle-death injection resets its core before taking the callback thread and
checks that a different name fits again, so retained proxies and charged room
cannot outlive the removed transport.

Scratch negative controls for #73 replace the live query with an empty-core
query, increment a synthetic join counter while dropping the handle, omit the
idle-death core reset, validate declared names before their byte bound, and
charge room for a refused registration. They fail the live-query, thread-exit,
idle-death and main-comparison tests, respectively (both final mutations fail
the main comparison). The scratch changes are restored after each run.

The lifecycle children have a deadlock watchdog only; no result depends on
elapsed time. Fault hooks and thread counters are compiled only in unit tests.
Scratch mutations for #72 were caught as follows: worker-side release and
thread creation before admission by
`refused_registrations_release_on_the_caller_without_starting_a_callback_thread`;
non-sticky actual callback death by
`an_idle_callback_thread_death_revokes_calls_that_do_not_invoke_functions`;
dropping the thread without joining by
`disposal_and_final_drop_complete_callback_thread_joins`. Reintroducing sticky
spawn failure and exiting on an empty handoff are caught by the two new tests
above. No parser path changes in #67/#72. Regenerate reference goldens into
separate scratch
destinations with `scripts/regen-golden.sh --into target/<fresh-name>` to keep
`conformance/` unchanged, then compare with the checked-in goldens.
 Swift callback tests make no thread-identity
assertion and need no source change.

## Allocation caps

Never pin measured heap bytes exactly across platforms. Prefer deterministic
allocation-call counts, same-process equality/ordering or ratios against an input
or control. Keep values, flags, errors, work and serialized room exact. An
absolute memory regression guard needs documented layout headroom: at least 5%
or a few KiB, whichever is larger, and a statement of the regression it catches.
Existing coarse MiB bounds and input-relative bounds are shape guards, not byte
snapshots. Stack tests assert completion or stack ancestry, not exact frame sizes;
timing output is diagnostic; positivity/conversion checks test timer reporting,
and deadlines are watchdogs, not speed contracts.

Linux x86_64 / Rust 1.97.1 observations in `.agent/ci/` (allocator headers included):

| Witness | macOS baseline | Linux observed | Portable guard |
| --- | ---: | ---: | ---: |
| Repeated reference, read | 198,899,344 | 198,899,368 | 208,844,311 |
| Repeated reference, unread | 198,899,280 | 198,899,304 | 208,844,244 |
| Three rows, eight levels | 3,714,336 | 3,783,728 | 3,900,052 |
| One row, scalar credit | 9,536 | 9,656 | 13,632 |
| Retained root (03c2c4c, process-wide) | — | 14,713,736 / 1,797,916 JSON | Worker-only; 8x JSON + 5% |

The review guards allow `max(baseline / 20, 4096)` extra bytes. They still
catch the old untrimmed runtime arrays, geometric prop capacities, and doubled
unique leaves; the small scalar cap catches growth above 4 KiB. A same-process
32 KiB scalar/literal control additionally limits scalar-credit overhead to one
payload copy plus 4 KiB, detecting an extra comparison copy that the small cap's
layout tolerance could hide. Both controls must produce exactly the same JSON.
The retained
root also must release to within 64 KiB of its pre-draw worker account. Its unit
test samples inside worker commands, after earlier command cleanup, and warms
first-use evaluator state before measuring. Other test threads and caller-side
allocations cannot affect that account. The earlier isolated integration test's
process-wide samples had no such guarantee.

The retained-root Linux failure was at `03c2c4c`; later passes are different
revisions, including changed runtime capacities. Replaying that revision's source
with the current macOS toolchain/lockfile gives 13,993,712 retained bytes versus
13,433,600 now (560,112 fewer). The logs do not demonstrate nondeterminism at one
revision. Worker-local sampling removes the latent cross-thread race regardless.

The argument-order fixture previously differed in 140 of its 162 observation
rows on Linux (163 lines including the final empty split line): e.g. 1,640 vs
1,616 and 2,000 vs 1,976 allocated bytes. Its byte column is removed. All public
results and flags remain exact, and same-process allocation-call plateaus at
work 13/14 (typed) and 22/23 (data) catch props-first traversal even at cuts where
the public results coincide. No target-specific fixture is required.

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
| `conformance` | Every corpus case (975), one-shot, after every streaming push and at each stream's end, under each of the corpus's chunkings, against the reference's goldens. Also that a stream, once ended, gives what one-shot parsing gives, apart from programs where the reference itself doesn't. | Any difference from the reference in parsing or streaming |
| `stages` | Tokens, statements and expressions, and fence and comment stripping, on every prefix of every program | Which stage of the pipeline a difference starts in |
| `edits` | Merging (3,269 cases), converting element trees back to source, and error hints, against the reference | Differences in the edit-mode and error layer |
| `prompt` | Every prompt spec in the corpus (66) against the reference's `generateSystemPrompt`. Its error messages are compared exactly; for crashes inside the JavaScript engine, only that it fails. | Differences in system prompts |
| `library_definition` | Library definitions (#131, #132, #133, #134): every sample in `conformance/golden/library-definitions.json` (zod declarations run through upstream's `createLibrary` and converted to plain data by `reference/library-definitions.ts`) gives upstream's `toJSONSchema()` and `toSpec()` byte for byte, or `createLibrary`'s error message: every scalar kind, optional and required, descriptions (empty ones left out of `$defs`), groups, root and id; JavaScript key order for array-index names, listed in any order; a duplicate component name; `$ref` escaping and the empty name; inherited and empty roots, and a `__proto__` component whose own fields answer the root check (`description` only when not empty, the last such component counting); a strict props object; a root that is not a component. One sample per data kind, so a failure names it: allowed words (with array-index words first and a repeated word once, quotes unescaped in the signature, and the empty enum, which has no signature type), arrays, arrays of unions (parenthesised), unions of scalars (folded into a `type` list as zod 4.6.7 folds them, repeats once, one-member and nested unions), unions with an object (left as `anyOf`), the empty union, inline objects with optional and nullable fields, the empty object, inline-object key order, records, nullable (kept in `required`, `?` in the signature), optional and nullable together (including `.nullish()`), and kinds nested in a two-component library; and the kinds that link a library together (#133): component references inside every kind, `$defs` in upstream's depth-first order for references forwards, backwards, in a cycle and to the component itself, a reference into a component named `__proto__` (an entry without a description) and through an escaped `$ref`, a reference to the later of two components of one name, a component outside the library (a named type), the action slot (parenthesised as an array's items), slots restricted to named actions, action entries placed where first met, a component of an action's name (the action then inline), bindings on every kind (the schema unchanged, `$binding<...>` in the signature), named types (a named nullable printing its inner type, an empty name printing none) alone, in containers and colliding with each other and with a component's name; and functions and actions (#134): a function with params and a return type (` → T`), returning an array of objects, with no return type, with a nullable return type (unwrapped in the signature), every kind of param and of return type (an empty enum giving no arrow), function names in JavaScript key order with the empty name and `ActionExpression`, a call's `$ref`s to components and actions without `$defs` of their own, the action slot inline in a call when a component has its name, functions without components, a custom action referred to by a component's slot (its `$defs` entry `{type: const, params}`, printed `@Name`) and unreferenced (no entry), custom actions in arrays, unions, nullable and through an escaped `$ref`, an action's params referring to components and other actions (`$defs` where first met), the spec's key order with groups, root and id, and 16 name collisions (with a built-in function, a built-in step, `Action`, `Query`, `Mutation`, an inherited name, `__proto__`, a component, another function or action), each upstream's message; enum words and object fields listed in any order give the same output, and every sample compiles to the `Library` of upstream's own schema. The generating script also stops if the converter accepts any of the declarations the plain data can't carry (zod formats, checks, coercion, unsupported metadata, catch-alls on props or inline objects, a shared props object, a missing renderer, extra fields, group fields in another order; and for the data kinds: literals, tuples, intersections, maps, lazy, readonly, null, discriminated and exclusive unions, enums whose keys aren't their values, record keys other than a bare string or with checks, partial and loose records, array length and maximum checks, `.optional()` inside a type or inside `.nullable()`, `.default()` in a type, unsupported nested metadata; and for the linking kinds: a binding inside an array, a union, an object field or a record, under `.optional()` or `.nullable()`, or on a component's props object, a prompt name on `.optional()`, a reference to the empty-named component, to a shadowed duplicate (where upstream throws) or to the first of two `__proto__` components, and a custom action outside the library; and for functions and actions: an action named `ActionExpression` (where upstream throws when a library refers to it and to the action slot), a reference to the empty-named action, a binding on a param or a return type, an optional or checked return type, a params object shared with a component, extra fields on a function or an action, and an action ref shared by two actions; a schema zod writes as a `__schemaN` `$defs` entry (met again inside itself through a reference: an object, array, union, record, nullable or optional, two actions sharing params, an action's params as a prop type, a call's or an action's param type), where it also checks that upstream's output still has the entry, while sharing that zod writes inline still converts (two actions sharing params, a call's params or return type being an action's params, an object shared after its cycle); symbol keys in a props object or an inline object, where it also checks that upstream still throws, a recursive getter schema and a type past the 256-level limit, while 256 levels convert). Group fields come out as name, components, notes. A kind the core doesn't support is refused naming it, and malformed plain data (including a duplicate field, and a binding on an object field) with where and why. A reference to a name that is no component's is refused with `UnknownComponent` naming where, at any depth; an action name a slot can't name with `UnknownAction`; a reference to the empty-named component and a named type over a reference, a slot or another named type as invalid. A binding leaves the schema and the parser's `Library` as they are. Each name collision has its own test against upstream's message (built-ins, reserved calls including inherited names, components, other functions and actions, functions checked first and the root before them), an action named `ActionExpression` and a reference to the empty-named action are refused as invalid, references in a call's params and return type and in an action's params are checked with where, malformed calls are refused with where and why, a call's types nest to the limit and no further (and at the default limit build on a 64 KiB thread and pass the prompt's depth), every program calling each sample's functions and actions right, with an argument missing or extra, of the wrong type, or calling an unknown function parses exactly as with upstream's JSON, and a library's calls follow the 20 built-ins. A 2,000-component reference chain closed into a cycle is written in one pass on a 64 KiB thread, its `$defs` in depth-first order. Depth: a type nests to `Limits::depth` (256) and no further, for arrays, objects, unions, records, nullables and named types (and an action slot written inline at the deepest level); one level more is refused with `LibraryDefinitionError::DepthLimited` naming the first type past the limit; a definition at the limit reads back from its text with `parse_json`, writes its schema and spec, and its spec passes `generate_prompt_with_limits`, on a 512 KiB thread; a lowered or raised limit moves the boundary; 200,000 levels are refused at the limit on a 128 KiB thread; on a 64 KiB thread a definition at the limit in each recursive kind reads, builds its `Library` (compiled on a stack segment sized for the depth), schema and spec, and drops them, freeing the JSON with `free_json` (at opt-level 0 too, in `scripts/check.sh`'s unoptimised step). The parser guarantee: the corpus library for `conformance/cases/library-definition/` is the sample's upstream JSON Schema, the defined library compiles to the same `Library` and parses those cases identically one-shot and streamed, and the built-in table keeps 20 names. Prompts (#135): every entry of `conformance/golden/library-definition-prompts.json` (upstream's `library.prompt(options)` and `generateSystemPrompt({cloud: true, library: library.toSpec(), ...})`, written by the same script) is the defined library's `prompt` and `cloud_config` byte for byte, or upstream's error message, and the prompt equals `generate_system_prompt` with the spec; options and the cloud request keep the prompt's depth limit, options nulling `components` are refused, options that are not an object add nothing; `binding_props` names the kept components' binding props in property order; a definition at the type depth limit in each recursive kind makes its prompt and cloud config on a 64 KiB thread. The deep-stack prompt tests here and in openui-session's `library_definition` (options nested 9,000 levels as arrays and as objects, definitions 3,000 types deep) must also pass unoptimised, where serde_json's copies cost most: `cargo test --profile dev -p openui-session --test library_definition` and `cargo test --profile dev -p openui-lang --test library_definition prompts_` | Output that drifts from upstream's, a definition that changes how programs parse, a deep definition that crashes instead of being refused or built |
| `definition_traits` | `Clone`, `Debug` (`{:?}` and `{:#?}`) and `PartialEq` on `TypeDescription`, `LibraryDefinition` and `DefinedLibrary` (#231): each of the twelve on a 64 KiB thread, at depth 256 (the default limit) and 1,024 (a raised one), on every shape the reader makes (object, array, union, nullable, record, named and array-with-minimum chains, a deep JSON default, and deep function and action types), plus a type built directly 100,000 levels deep; and no change in answers: a clone equals its source, equality agrees with a recursive comparison over about 600 values (pairs that differ only deep inside included), and `Debug` equals the text the derived impls printed (`tests/definition_traits_golden/golden.rs`, captured before they were replaced). Run it unoptimised too: see [Host JSON on a small stack](#host-json-on-a-small-stack-203) | A clone, copy, print or comparison that recurses on the caller's stack, or an impl that drifts from the derived one |
| `robustness` | Property tests: any text, cut anywhere, through every stage and every entry point, finishes without panicking | Crashes on arbitrary input |
| `depth` | Absurdly deep nesting stops at 256 levels with `depth_limited`, on a 64 KB thread stack, because the core grows its stack on demand; so do state defaults nesting the ones above them, at 768 JSON levels | Stack overflows, a recursive walk the stack growth misses |
| `size` | Programs that double with every line stop at the result-size limit with `size_limited` (by default about 64 KiB for these short programs, up to 8 MiB with the limit fixed there), state defaults evaluated from the ones above them too | Runaway memory |
| `work` | The work limit on evaluating state defaults: its default scaling with the program; running out cuts the default (and every later expression default) and flags the result, and raising it lets it finish; the cost in steps, found by bisecting the work limit, of `@Each` reading an expression per item (items times template), of a component read many times (its size once), and of sorting (`n log n`) and filtering (`n`); the slow fuzz input, shrunk | Evaluating that takes seconds on a small program, or a step that isn't counted |
| `limits` | The `Limits` value: its documented defaults; raising and lowering depth and result size, one-shot and streaming; the result-size limit scaling with the program (floor, amplification, ceiling, or fixed), a small program amplified past it cut, a stream cut early and whole once its text has grown, and the `stream` fuzz target's slow input from the 1.1 long runs, shrunk to 86 bytes with a 98 KB result; past the program-size limit, a flagged result equal to parsing the text up to it (never splitting a character), and a stream that ignores pushes until `set`; no default cuts a corpus case | A limit that doesn't reach a stage, or a default that cuts real programs |
| `entry_points` | `Limits` on the other entry points: merging reads each input to the program-size limit and expressions to the depth limit, flagged; one expression stops at the depth limit; reading JSON, converting back to source nest three JSON levels per depth level and 32 more (800 by default), refused past it and let through when raised; a 30,000-level JSON value frees on a 64 KB stack; a prompt's allocation calls grow linearly with its component groups | An entry point a hostile input can run unbounded, or a limit that doesn't reach it |
| `prompt_depth` | A 900-level tool input schema is refused at the default JSON depth limit and renders when the depth limit is raised; a lowered limit refuses a shallower schema | A prompt entry point that ignores the depth limit |
| `drop` | Deep results, at the default depth limit and at 20,000 levels, parse, stream and drop on a 64 KB thread stack: arrays, elements, a runtime expression, evaluated defaults and a 10,000-statement reference chain. Run it unoptimized too (`CARGO_PROFILE_TEST_OPT_LEVEL=0`), where frames are largest | A tree that frees itself by recursion |
| `weak_drop` | Freeing a tree whose `Arc`s have weak references (#242), every test on a 64 KiB thread, also unoptimised: the review's repro (a JSON array and a JSON object chain 4,096 deep that `reserve_value` refuses, leaving a weak reference per level), directly built chains of 100,000 levels (arrays, objects, elements, a runtime expression in a value) and of every expression edge (unary, both binary operands, each ternary operand, member, both index operands, assign, array, object, call args, call mapped props) at 4,096 and 100,000 levels; a chain with a second strong owner at one level (the levels above are freed, the shared levels stay whole and equal to what they were, then free with their last owner); a weak reference upgraded before and after the chain's drop; every weak reference fails to upgrade after the last owner and live bytes return to the baseline; a 65,536-wide collection of watched children (and a watched collection) with a 50,000-level watched chain as its last child frees within #234's 16 KiB bound. The evaluator's own `Js` values are covered by two unit tests in `free.rs` | A freeing loop that leaves a child behind for Rust's recursive drop, or takes apart a subtree another owner still holds |
| `streaming_cost` | Allocation requests per push have a ratio below 2.5 on a program four times the size, bare and in a fence with prose around it | Streaming that grows with the square of the program |
| `scaling` | Allocation requests grow by less than 8x at four times the input size over these shapes: blank lines before a `?` line, one-shot and streamed; blank space between statements in a stream; a chain of references completed in one push; a chain of queries searched for loops, its loops and their order pinned. Unresolved names under an 80-statement chain fit the 64 MiB allocation cap, in order through reached statements; a push changing several statements gives what a fresh stream does | A scan, a cache walk or a list that is redone or copied once per line, statement or level |
| `streaming_reuse` | A stream takes the state defaults it evaluated at earlier pushes as they were (#8): every push equals a fresh stream given the text so far in one push, for defaults reading defaults, components read twice (one object), a runtime expression's properties read across defaults (one object), a redefinition, a late reference, a default cut by work or by room early and whole once the program grows, and the shrunk slow fuzz input; and by cost, a push after an expensive default costs a fraction of evaluating it, and sixty defaults streamed a character at a time cost about as much a push late as early (allocation requests) | Reuse that changes a result, or streaming defaults that grows with the square of the program |
| `report` | The counts in CONFORMANCE.md are what the corpus holds | A stale write-up |
| `cli` | The command-line tool's commands print what the library gives, including reading stdin, the default chunk size, and `--limit name=value` (and its error naming the limits) for every command: `tokens` reads to the program-size limit, `prompt` refuses a spec past the depth limit | A broken CLI |
| `api` | The public JSON reader and writer, `JsObject`, and walking an expression | Public helpers that parsing alone doesn't reach |
| doc tests | The crate docs' examples and the README's | Examples that no longer compile or run |
| unit tests in `js.rs`, `preprocess.rs`, `progress.rs`, `result.rs`, `evaluate.rs` | JSON reading and writing; the engine-dependent divergences pinned as the core gives them (a built-in constructor turned into text, a 65-value sort whose comparison loops) and collation outside ASCII by code point; an element or a runtime expression shaped into the object its JSON reads back as, without the text, equal to writing it and reading it back; incremental preprocessing against the one-shot form; the stall guard: each guarded loop panics naming itself on a forced stall, and a release stall gives a flagged cut result, a stream that ignores pushes until `set`, and a refused JSON read | Internal disagreements; a loop that stops advancing |
| unit tests in the fuzzer's `rules.rs` | The invariants' newer exceptions (quirk 4's empty redefinition, quirk 19 across a line, 21, 22 and 23) on the programs that found them, run on the core, and the differential fuzzer's explanations of the engine-dependent divergences (sorts whose comparison loops, constructors turned into text), with near misses each must not explain | An exception that stops recognizing its quirk, or explains too much |
| `openui-ffi`'s `binding` | The binding returns exactly the core's output: one-shot, every streaming step, `set`, prompts, merges, and with limits given, merges and prompts included | A binding that changes what it hands on, or drops a limit |
| `openui-ffi`'s `stack` | The deepest programs parse, stream and merge through the binding on a 64 KB thread stack, at the default depth limit and at 20,000 levels, and their results and sessions drop there too | A deep drop or walk on the caller's stack |
| `openui-ffi`'s `prompt_specs` | A prompt spec nested to serde_json's bound (127 levels, of objects and of arrays, as a spec, `{library, promptOptions}` and a cloud request) gives the core's prompt on a 64 KiB thread, and the 128th level, a spec a million levels deep and malformed specs are refused with the messages they always had; a library's prompt options at the evaluation depth limit make a prompt and a cloud config there, and options deep in each field the generator reads give what they give with room to spare; text the core's reader refuses, or whose repeated key replaces a value, after a value at the limit frees it with a loop (#203). Run it unoptimised too: see [Host JSON on a small stack](#host-json-on-a-small-stack-203) | A host JSON read, copy or drop on the caller's stack |
| OpenUINative's Swift tests | The package through its public interface: drawing with an app's own components, an evaluated element drawn at its own path, and live programs drawn in an off-screen window, clicked and typed in the way AppKit delivers a person's (state redraws in the mounted view, host events, bound inputs and their transforms, form fields written with no component type, an app's component given evaluated props, markers and plans; a click or write whose element or marker has changed, or comes from another program, doing nothing, while two clicks or keystrokes before a redraw on something unchanged both act; a button in a renamed form and a field drawn for another name refusing; forged plans running nothing, only the exact marker shape being a marker; mutations resumed, refused steps; events delivered when the redraw after an action fails, with the session disposed underneath; upstream's form validation, rule by rule; buttons ignored and inputs disabled while a response streams; table action cells and elements in cells; a select picked from the menu its pop-up opens); the typed prompt calls against every recorded prompt case their options can say, in full and cloud mode; each refusal the binding can produce, and each of the package's own, reaching Swift as an `OpenUIError` of its own kind; the generated binding directly | Package bugs (`scripts/check-viewer.sh`) |
| the viewer's Swift tests | Decoding (lossless for every corpus parse result and evaluated tree, and for odd objects), playback with every step evaluated or, without a runtime session, its parse result, the store panel (its draft text when the value changes underneath), the inspector's tree being the drawn one under `@Each` and a ternary, the program streaming until the shown step has every chunk, host events kept to the newest with their continuations let go, the step cache counting more than JSON, goldens through the viewer, native drawing of every corpus case evaluated, live streaming against a stub server, recordings, edit mode | Viewer bugs (`scripts/check-viewer.sh`) |
| the chat app's Swift tests | The Gateway client against recorded exchanges (a streamed answer split mid-line, an HTTP error, an error mid-stream, a lost connection, upstream's other error shapes), checking the exact request sent: endpoint, bearer key and body; no key means nothing is sent, and a shown message never holds the key; an overlong line and an error body past its limit are refused or cut, and an answer is read only as fast as the app takes it, with the request cancelled once it is let go. An endless answer ends when the session's response reader refuses it at the core's program-size limit: the app keeps only the text the session accepted, shows the session's refusal under the answer, and cancels the request. One thread end to end through that client: the answer drawn natively as it streams, the whole thread sent again with each answer's raw text, stop ending the request and keeping the partial answer drawn, an error shown in the callout with the person's message kept. The system message is the template's cloud config block, the bundled react-ui files are the corpus's, and Settings saves and removes the key through a store in memory. The recordings are shaped in OpenAI's format until a real run supplies real ones; no test touches the Keychain or the network | Chat app bugs (`scripts/check-viewer.sh`) |
| the WebAssembly packages | `scripts/wasm.sh` builds the web and Node packages and loads each through the package guard ([ADR 0007](docs/adr/0007-share-session-policy-between-native-and-wasm.md)); `bun --no-install test crates/openui-wasm/js` drives the built package. Skipped, with a note, without the wasm tools or Bun. | A generated member the guard doesn't cover (the package then fails to load), a declaration that drifts from the API, a JavaScript driver bug |
| compile time | The streaming parser, parse results and libraries can be shared between threads (`src/lib.rs`) | A change that stops embedders parsing off the UI thread |

Not in the check, because they need Bun, time or a screen:

| Command | What it does |
|---|---|
| `scripts/hunt.sh --minutes N` | A hunt: differential fuzzing with a fresh seed and the upstream check, for a set time, leaving a report (see [Hunts](#hunts)) |
| `scripts/upstream-check.sh <folder>` | The upstream check on its own |
| `scripts/fuzz.sh --seed N --count N` | Differential fuzzing: generated and mangled programs through the core and a live reference, comparing every step. It saves shrunk mismatches as corpus cases. A result a limit cut can't match the reference, which has none: where the answers differ and the core's was cut for size or work, it's parsed again with those limits raised, and counts as "cut by a limit" when it matches then, a mismatch when it doesn't. |
| `scripts/fuzz.sh --runtime --seconds 60 --seed 40 --findings target/runtime-hunt` | Generated state and interaction sequences through the public runtime and the existing reference runtime golden adapter, with supervised children (see [Runtime differential fuzzing](#runtime-differential-fuzzing)). |
| `scripts/fuzz.sh --edits …` | The same, for merging and converting back to source |
| `scripts/fuzz.sh --messages --seed 97 --count 4000` | Differential message reads at every UTF-16 cut and writes, seeded from `conformance/messages.json` and the ASCII message divergence inputs. Use `--only` to replay an input and `--findings` to retain the request and outputs. |
| `cd reference && bun --no-install test messages.test.ts` | Checks the message fuzzer's reserved-scalar projection against both sides of the pinned divergences and checks the neighbouring scalar. |
| `scripts/fuzz.sh --invariants …` | The invariants, on the core and the reference alike (see [Invariants](#invariants)) |
| `scripts/fuzz-coverage.sh …` | Coverage-guided fuzzing of the targets in `fuzz/` (see [Coverage-guided fuzzing](#coverage-guided-fuzzing)) |
| `scripts/mutants.sh parsing\|rest\|runtime\|query` | Mutation testing (see [Mutation testing](#mutation-testing)) |
| `scripts/import-recordings.sh` | Recorded streams into the corpus (see [Recorded streams](#recorded-streams)) |
| `scripts/android-test.sh` | The suite on a running Android emulator or phone (see [Android](#android)) |
| `scripts/bench.sh` | Streaming speed against the reference |
| `scripts/viewer-timing.sh` | The viewer's drawing speed |

### Query manager scenarios

`query_scenarios` replays `conformance/query-scenarios.json` and compares
each observation byte for byte with `golden/query-scenarios.json`, and
`conformance/query-runs.json` with `golden/query-runs.json` the same way.
An `invalidate` step runs the queries named by `ids`, or every query when
`ids` is absent or empty.
For a named divergence it checks the regenerated upstream bytes and the Rust
bytes against the separate streams in `divergences/query-*.json`. A pin
whose two streams converge fails the test.
From `reference/`, run `bun --no-install query-scenarios.ts` to regenerate
that file alone. The full regeneration script also runs this adapter.
Set `OPENUI_QUERY_SCENARIO` to a scenario name to filter the Rust test.
A refused command is observed as `{"refused": <message>}` and changes nothing;
only a pinned divergence may expect one.
Call numbers are zero-based dispatch positions. A completion may instead name
the dispatch step, tool and arguments. Only the side that issued that unique
call processes its outcome. This keeps an extra dispatch from shifting later
completions onto unrelated calls. Evaluate registers the current program.
Readers also read the live Store and per-tool extra key values when an input
result changes. The reference clock stays fixed during these scenarios.

With the cached Bun reference, run
`cargo test -p openui-runtime --test query_scenarios -- --ignored --nocapture`
for 6,000 seeded edit scripts and 512 reader graphs. In the edit scripts, statements can share a key, exchange keys, edit
both statements or remove one holder and transfer its key in a single evaluate.
A script can also run every query, one or both in either order, one twice or
an unknown name, including while calls are in flight.
The generator checks that it exercised both declaration orders for a batch
handoff. It also uses each reserved list name as a statement id with an array
default. Addressed completions settle calls issued by only one implementation.

`reference/query-rule-model.ts` computes the final registrations, projects
their current and previous keys, and retains exactly those entries across the
command. It supplies every expected Rust byte and entry count. Separately,
`reference/query-lifetimes.ts` models lang-core and enables one documented
correction at a time to attribute differences. Both ends of that comparison
are checked. Each differing observation must have one named, pinned row in
CONFORMANCE.md, and the test requires each row's count to be positive and exact.

The reader graphs vary declaration order, fan-in, completion order, failures,
refetches and Store edits between evaluate and complete. Their unmodified
reference trace is compared with counterfactual traces that change the
origin of the loading flag in upstream’s pending predicate, then defer dispatch
until the current holders are known. Completions address the original request
by step, tool and arguments on both sides. Fixed cases in
`conformance/query-readers.json` use the same adapter with `--readers`.
The Rust replay also checks that a boolean changing from true to false refetches its reader with the new argument, while a second false leaves an intervening Store edit unread.
The renderer replay drains reader calls before the next draw. The same ignored
integration suite streams 5,000 further two- to six-query graphs, including
cycles, alias and default reads, per-tool extra keys, removals, renames, and
missing or non-reflexive results. These scripts also run registered queries, remember runs, and read mutation status across submissions. The loop snapshots the pending call list at
each checkpoint. Every trace has a step and dispatch ceiling. Duplicate JSON
dispatch addresses carry an occurrence within the dispatch step.

Run `cargo test -p openui-runtime --lib each_completion_order -- --ignored`
for the ordered round traces. Two initial batches are each settled in every
permutation for n=2, 3, 4. Rust records each expression actually rebuilt,
in order, and the remaining stale set at a cut. Upstream's trace and bytes
are hashed in `query-reader-rounds.json`. The replay also compares both
phase witnesses and a four-query cycle with three readers made stale by
the same completion directly with `query-reader-fixes.json`.

After every command the query, clock and mutation replays compare the exact
cache keys and holder counts with the current and previous registrations.
They first check that each newly returned receipt is pending, belongs to
the manager and is unique in its batch. Each query receipt must match the
pending identity of an entry with a current holder.
They derive loading and refetching from receipts in the driver's ledger and
follow the waiting edges from those receipts. A wait needs a pending or failed
source, a diagnosed cut, or a chain that reaches another such source or a
cycle. The checker recognizes the self-delivery diagnostic and unfinished
rounds at the round bound. On completion it also checks the staged delivery,
before readers can change keys, against all current holders of the original
receipt. Unrelated holders keep their data, errors and fetch history.
The 6,000-script replay independently compares the exact entry count with the
reference rule model. In `query_commands`,
2,000 cycles of three edits and three successful 10 KB completions check the
exact cache size after each command. That suite also checks the shared work
allowance, expression and host JSON depth refusals, foreign and repeated
call identities, and extra cache-key values: refused ones keep the previous
values, they are refused at exactly the depth a tool result is and one byte
past the result-size limit as escaped JSON, a refusal copies nothing (its
peak allocation is checked), they count against the evaluation's work, and
a repeated name keeps its last value.

### Tool result limits

`result_limit_scenarios` replays `conformance/result-limits.json`, whose
scenarios carry their own `limits` (`resultSize`, `depth`, `cacheBytes`) that
the reference ignores. `reference/result-limits.ts` writes
`golden/result-limits.json` through lang-core's own manager. A scenario at a
limit must match its golden byte for byte. One a byte or level over must
match its pin in `divergences/result-limit-*.json`, and each pin must be a
row of CONFORMANCE.md's tool result limits table. After every step the
manager's charged cache bytes equal a fresh measurement of what it holds.

`result_limits` (runtime) pins the size, depth and cache-cap thresholds for
queries and mutations through both completion paths, the `tool-error` on
every query of the key, the peak allocation of a refusal, and a seeded
20,000-command script in which, after every command, the cache holds at most
two entries per query, no query is starved, the charge equals a fresh
measurement and stays within the cap, a repeated completion is refused and
changes nothing, and outstanding query calls never exceed two per query (one
on the current key, one finishing on the previous key). Failures on a previous
key or a mutation fired before disposal are discarded before admission. A
failure on a current key is charged only to its readers (#192). A mutation can have
several outstanding calls after renames, as upstream allows; a session caps
pending managed calls globally at `maxContinuations` (default 64).
`a_removed_mutation_discards_an_over_limit_failure_on_both_completion_paths`
retries a refused failure after removing the registration, both with the name
absent and with a new call under the same name (#214).
`a_removed_mutations_action_tail_runs_only_on_provider_success` checks the
session action after a late success, a failure and an over-limit failure.
The runtime tests also pin the depth error for nested host JSON objects, the exact failure-text charge across holders, and the empty-name count accepted for a missing tool on both completion paths (#193).
The runtime tests cover withheld delivery errors, receipt recovery after reader evaluation errors or missing context, and query registration at the exact statement-id work limit (#194).
`recreated_query_receipts` (runtime) replays #204's reproduction: a staged
delivery that is neither finished nor dropped keeps a removed query's call
alive while the query is registered again. The old receipt's success, each
failure (including text past the limits, never read), a refused result and
an abandonment change nothing; the recreated query keeps its call and its
remembered run. A seeded differential feeds two managers the same 20,000
rounds of edits, completions of any receipt ever issued, runs, abandonment
and dispose, one of them keeping a staged delivery alive through each round,
and requires every observation to match (about 400 completions arrive while
a stage holds their call alive). The session, native and WebAssembly suites
each run a similar sequence through their public API (`b` shares its key
with a third query, so its failure text is refused after staging), and show
that nothing they hold keeps the old calls alive (session and WebAssembly
check the receipts) and the old result is discarded there too.
`result_limits` (session) pins
the line between a delivered `tool-error` and a refused outcome for outcome
JSON, including text past the outcome bound, MCP responses and the peak
allocation of refusing a 4 MiB result. The native and WebAssembly bindings
each have a `result_limits` test for `maxCacheBytes`, and
`crates/openui-wasm/js/queries.test.ts` checks that the exported
`RuntimeSession.withLimits` carries it. `scripts/check.sh` builds both
WebAssembly packages, loads them through their guards and runs these package
tests when the wasm tools and Bun are installed (see
[Running the tests](#running-the-tests)); if it says it skipped them, the
binding change is unchecked there.

The review round added tests, each failing before its fix: a rename's idle
`null` is not charged; a refused result of an older submission is that
call's failure; a completion for a removed query is discarded on both paths
whatever it holds; an over-limit result ends the call for 24 readers through
the runtime and through session JSON; an object's member order never decides
invalid versus too deep; a padded or bare `null` MCP response is refused; a refused refresh result
ends its call while the refresh schedule continues; every completion path admits under the same limits, the work
limit included; refusing wide objects, wide arrays and long strings takes
the same peak allocation at every width; validity is decided before size
(both orders pinned, and depth only when reached first); a delivery the
snapshot cannot hold still ends the call on both paths; long MCP responses
are checked for validity before size; and a mutation result refused inside
the runtime ends its plan. The random script renames a mutation and asserts
after every command that the charge equals a fresh measurement and that
each query has at most two outstanding calls.

### Query manager suites

The other suites of the query manager and the session that drives it. Each
replay compares its observations byte for byte with a golden written by a
reference adapter in `reference/`, except where a two-sided pin in
`conformance/divergences/` names the difference (see
[CONFORMANCE.md](CONFORMANCE.md#divergences)).

The hidden cache audit keeps registration history for the current program
(#210). A missing incomplete source can explain a wait if it never
registered. A source that registered while complete keeps its result when
it becomes incomplete, so losing that registration is a rule 4 fault.
A committed program change starts fresh history with the registrations the
new program inherits. Changes to completeness alone preserve that history.
The same record keeps the exact reads of a skipped, already-waiting reader
so delivery can be audited after its reader plan is gone. The test and fuzz
drivers enable the `query-audit` feature. Default builds omit the record
and its observation during evaluation.

The #196 tests use complete, parser-produced statements with unique ids.
The cache audit tests change a settled manager's recorded reads or
diagnostics to check which waits and idle failures rule 4 accepts. They
cover self-delivery, a missing input that becomes ready at the round bound,
the propagation limit on a suspended reader, the source of a visible error,
and the compact diagnostic for withheld errors. The command tests also
check stale mutation receipts and the failed-entry test hook. #225 tracks
host-built programs, including the reactivation case from #226.

Two rule 4 mutants are equivalent (#196, ticket positions 235:34 and
240:55). A missing read has no available data. A suspended source already
passed the check requiring a reader limit. A live receipt or another wait
already explains the missing read. Every remaining registered source had
to pass the idle-query check with both a failed entry and a visible failure.
Widening either terminal therefore accepts exactly the same waits. The
two exclusions are in `.cargo/mutants.toml`.

The #223 tests in `query_reader_commands` suspend a reader with a call in
flight, then change only the public completeness flags. They audit every
command and check the snapshot, diagnostic, receipts, and recovery when
the reader is complete again. Further propagation cuts check the separate listing for newly suspended readers
and a different limit while an incomplete reader keeps its original refusal.

| Test | Checks |
|---|---|
| `query_refresh` (runtime) | `conformance/query-refresh.json` against `golden/query-refresh.json` from `reference/query-refresh.ts`, which runs lang-core's manager on a fake clock; each observation also has the next refresh deadline, and the differences are pinned in `divergences/refresh-lifecycle.json`. Also refused clock commands, large advances, the refresh floor a manager reports, a repeated clock sample, and that mutation registration and submission keep the active deadline and ask for a redraw when a mutation takes a reserved snapshot list's name. With `--ignored`, seeded timed scripts against Bun. |
| `mutation_scenarios` (runtime) | `conformance/mutation-scenarios.json` against `golden/mutation-scenarios.jsonl` from `reference/mutation-scenarios.ts`; deep arguments and results, a mutation read before a query of the same name, a `__proto__` mutation, late outcomes after dispose (`golden/mutation-dispose.jsonl`), and that registering again keeps a numeric tool name's result and asks for a redraw for a reserved-name mutation only when its result's JSON changes. |
| `query_reader_commands`, `query_reader_recovery` (runtime) | Readers: a reader cut by a limit, delivery to a healthy holder of its shared key, retirement and reissue of a call held only by a suspended reader, Store snapshots taken at registration, long waiting chains, a shared reader allowance (including probes of unchanged readers, comparisons after a string source changes version, and string defaults reapplied during a cascade), the two dispatch guards (a remembered run and a reader's request for one failed key start one call; a cascade cut by the snapshot limit starts none of its requests), the first request kept for a colliding key, a parse cut that names the reader and its dropped source, a reader whose wait list names a statement that is not a query (the fuzz input and the `@toString` trigger, #188), a reader waiting on an explicitly incomplete query through Evaluate and Invalidate, a registered reader becoming incomplete before delivery and complete again afterward (#210), and `conformance/query-reader-fixes.json` against its golden. |
| `delivery_work` (runtime) | One tool delivery spends one work allowance (#222, answering #200): readers that each read a cached version hidden when the delivery began, at a limit of 10,000 and of a million with the default and a raised cache cap, are cut within the delivery work limit plus the measured cost of one error row per suspended reader; default limits do not cut the same program. At W = 65 and W = 50,000 the error rows account for the entire charged excess (182 and 11,756 units). The registration bounds the reverse-dependency walk. One or 128 readers refetching an unchanged 16 KiB string pay for one comparison. A one-unit shortfall suspends the readers after the source has been admitted. A prior provider error of up to 256 KiB leaves a reader its full program-scaled allowance, including when the reader uses a 2,048-byte store preference. A 2,000-reader refetch pins the default cutoff after raw validation, and mutation tests count node visits and UTF-8 bytes on both result paths. The hidden `QueryManager::last_delivery_work` record (`query-audit` only) sums what each phase measured on its own counter. |
| `queries` (session) | `conformance/session-queries.json` against `golden/session-queries.jsonl` from `reference/session-queries.ts`; provider attach and detach, a provider refused beside supplied reference values, single-use call identities, a refused draw or outcome that keeps its call, and a replacement refused by a limit that draws its queries' defaults (both sides pinned in `conformance/divergences/session-a-refused-replacement-draws-the-new-queries-defaults.json`). |
| `refused_registration` (session) | A program whose registration is refused while the registrations belong to another (#119): actions and resumed plans read its defaults, its mutations draw idle and a mutation step fails without a call, `@Run` and Retry start no call, the other program's timers do not fire, a late result starts no call and is not drawn, a held reader call of the other program is abandoned when a new program registers unless that program wants it for a current key (then it goes out once), the snapshot lists nothing, and going back to the registered program draws its data; the cache audit holds after every drain. It also covers re-reading matching registration inputs, changing the state default used as a mutation tool name, a non-limit refusal and a held mutation across a refused replacement (#187). Two of its replays pin both sides against `session-mutations.json` and `session-runs.json`. |
| `query_runs` (runtime and session) | Runs of a query and Retry, described under [Expression evaluation](#expression-evaluation). |
| `mutations` (session) | `conformance/session-mutations.json` against `golden/session-mutations.jsonl` from `reference/session-mutations.ts`: action plans that suspend at a mutation and resume, the pending-plan limit, and detaching with a plan pending. |
| `query_refresh` (session) | Pause and resume, refresh during a streaming edit, a mutation outcome while paused, and a refused replacement that keeps its timer. |
| `paused_actions` (session) | An action dropped because queries are paused (#117): a mutation clicked while paused, one in flight at pause (late success, late failure, never settling), the second mutation of a plan, a host continuation resumed while paused, pausing twice and a replacement before the drain each give exactly one `QueriesPaused` failure, and nothing of the plan runs after resume; an undrained failure keeps its continuation slot; a query `@Run` while paused is skipped and the plan goes on. The cache audit holds after every command. One replay runs `conformance/paused-actions.json` against `golden/paused-actions.jsonl` from `reference/paused-actions.ts` (the renderer's loop on a disposed manager): calls, events and state match upstream at every step, and the failures are pinned in `divergences/paused-actions.json`. `runtime_mutations` (native) and `crates/openui-wasm/js/paused-actions.test.ts` (both sessions and the JavaScript driver, `free` and `dispose` right after a pause included) check the same through the bindings; the reason says "not sent" exactly when no driver took the call, and the provider is never invoked then. `paused-actions-free.test.ts` frees or disposes the session inside each `onError` and `onAction` of a drain and of the free's own delivery, with one, two and three waiting failures and callbacks that throw: each failure reaches both callbacks exactly once, in order, and nothing runs after the free returns (ADR 0014). It also covers a segment a callback produces during a free, dispose or detach, pins how a callback's returned thenable is followed (as `Promise.resolve` follows it), documents the accepted limit for a provider thenable freed before its job runs, and checks across seven busy sequences that no callback runs and no provider is invoked after the outermost free. `paused-actions-round3.test.ts` covers drivers detached earlier, a driver attached inside a closing drain, the configured and default continuation limits on delivery before a free (including nested frees), and the evaluation error a detach's delivery takes. A replacement from `onError` keeps the same segment's `onAction` with its original callbacks, including across a nested free. |
| `query_reader_diagnostics` (session) | A reader's limit or changed self-input reported beside the drawn tree, and its recovery. |
| `response_bundles` (session) | `conformance/session-bundles.json` against `golden/session-bundles.jsonl` from `reference/session-bundles.ts`: the bundle gate, Cloud script revisions in cache keys, and app values. The same adapter replays `conformance/session-blocked.json` into `golden/session-blocked.jsonl` with the renderer's `blocked` on every row; the walk compares each draw's `blocked` with it, then goes on with commands of the session alone (refused updates, pause and resume, program replacement, `setProgram`) and checks the click gate is unchanged (#118). `crates/openui-wasm/js/session-bundles.test.ts` replays the same rows through `setResponse` and measures the scripts JSON in UTF-8 bytes. |
| `runtime_queries`, `runtime_mutations`, `runtime_refresh_clock`, `runtime_query_runs`, `query_readers` (`openui-ffi`) | The same operations through the native binding's worker thread. |
| `crates/openui-wasm/js/{queries,mutations,query-clock,query-runs,query-readers,tool-provider,paused-actions,paused-actions-free,paused-actions-round3}.test.ts` | The JavaScript driver through the built package. The tool provider tests count Date, property and Proxy hooks, inject a stale busy count and run a fake host task inside a provider. The function tests exercise refused stops with a driver attached (`bun --no-install test crates/openui-wasm/js` after `scripts/wasm.sh`; both run by the check when their tools are installed). |

The coverage-guided `query` target replays the same scenario format (see
[Coverage-guided fuzzing](#coverage-guided-fuzzing)), and `scripts/mutants.sh
query` runs the mutation group for the query manager (see
[Mutation testing](#mutation-testing)).

## Host JSON on a small stack (#203)

No host input may overflow the caller's thread. The test profile builds at
opt-level 1, which shrinks frames enough to hide an overflow (#135's review
showed this), so the deep-stack tests for host JSON must also pass
unoptimised, on 64 KiB threads. `scripts/check.sh` (and so CI) runs the
stack-depth test files at opt-level 0 after the whole suite, and only those:

```bash
cargo test -q --profile dev --no-fail-fast \
  -p openui-lang --test depth --test drop --test prompt_depth --test entry_points \
  --test runaway_finished_walk --test library_definition --test library_sharing \
  --test definition_traits --test weak_drop \
  -p openui-session --test library_definition \
  -p openui-ffi --test stack --test prompt_specs --test library_schemas --test library_definitions
```

That step builds the dev profile once and runs in under a minute on an
Apple silicon Mac (measured for #203). Making it pass needed three test
fixes only: `depth`'s and `library_definition`'s own deep inputs were
dropped or cloned with serde_json's recursion, and are now freed with
`free_json` or built twice.

`library_sharing` is in the step for #215: `a_deep_prop_default_is_applied_on_a_small_stack`
applies a 2,000-level prop default built in Rust on a 64 KiB thread, which
the opt-level 1 test profile hides.

`definition_traits` is in the step for #231: each of `Clone`, both forms of `Debug`
and `PartialEq` on the three definition types runs on a 64 KiB thread at depths
256 and 1,024, which the opt-level 1 test profile hides.

`weak_drop` is in the step for #242: deep arrays, objects, elements and expressions
whose every level has a weak reference drop on a 64 KiB thread. `Arc::get_mut`
declines while a weak reference exists, and the freeing loop then has to take the
payload with `Arc::into_inner`.

`DEPTH_STACK_KB=<n>` sets the thread stack in `prompt_specs`. At opt-level 0
every `prompt_specs` test passes on a 16 KiB thread, the smallest macOS
gives; before #203 the spec at the bound needed between 256 and 512 KiB, and
the reader's refusals after a value at the limit as much. The WebAssembly
tests (`library-definitions.test.ts`, `session.test.ts`) also pass against a
package built unoptimised (`scripts/wasm.sh` with `--release` removed), where
WebAssembly's 1 MiB stack already held what overflowed natively. The WebAssembly binding has no `generateSystemPrompt`: its prompt input is a library's `prompt` and `cloudConfig` options, read by the core's reader, so its tests hold them at that reader's bound (800 levels read, the 801st refused) rather than serde_json's; its serde_json read, a library schema, is tested at 127 and 128 levels since #140.

Every place a binding, openui-session or the core reads host JSON, or copies,
compares or drops what it read, and why it fits any stack:

| Where | Reads | Why it fits |
|---|---|---|
| `openui_ffi::generate_system_prompt(_with_limits)` | a prompt spec, serde_json (127 levels) | Read on its own segment (`openui_lang::read_host_json`, 1,664 KiB), generated on one sized for its depth, freed with `free_json` (#203) |
| `openui_lang::generate_prompt` (Rust only), `generate_prompt_with_limits` and `generate_system_prompt_with_limits` | copy parts of a spec (its library, components, functions, actions) with serde_json | Run on a segment of 512 KiB plus 6 KiB a level, after the depth check (#203): reserved address space, not touched memory, about 5.2 MiB at the default limit and uncapped for `generate_prompt`, which takes no limit; before, options deep in `components` overflowed a 64 KiB thread at opt-level 0 |
| `openui_session::library_from_schema` (both bindings' schema constructors) | a JSON Schema, serde_json (127 levels) | `read_host_json`, the same segment (#140); compiled and freed with loops |
| the core's reader, `parse_json_with_limits` (definitions, prompt options, the session constructor's prompt spec, session state, references, field writes, query extra values, host function results) | iterative, to `Limits::json_depth` (800) | Never recurses; a refused text's values and a repeated key's earlier value are freed with `free_json` (#203); callers free with `free_json` or the session's `Json` wrapper |
| `parse_json_value`, `read_message_json`, `read_bundle_json` (messages, bundles, queries, MCP responses, tool results) | iterative, into `Value` | `Value` frees itself with a loop (ADR 0002); tool results read 3 levels |
| `openui_session::define_library` | a definition, the core's reader | Read under the evaluation limits, freed with `free_json`; `LibraryDefinition::from_json_with_limits`, `to_json_schema`, `to_spec` and `stringify_json` grow the stack (`grow_stack!`); the schema it holds freed by `free_shared_json` |
| `DefinedLibrary::prompt_with_limits` | prompt options merged into the spec | Moved in, never copied, generated through `generate_prompt_with_limits` and freed with `free_json` |
| `DefinedLibrary::cloud_config_with_limits` | the cloud request | Built from the library and the moved-in options, generated through `generate_system_prompt_with_limits`'s segment (which replaced #135's own) and freed with `free_json` |
| `Value::from_json` | a value the reader made | Grows the stack (`grow_stack!`) |
| openui-runtime's `set_extra_key_values` | query extra values the session read | `host_json_bytes` and `json_ast` are iterative |
| WebAssembly `RuntimeSession.actionAt` | a path, serde_json as `Vec<String>` | Two levels at most: a nested array is refused as the wrong type before serde_json recurses |
| openui-session's response script names | a JSON string token, serde_json | A string: no nesting |
| `openui-lang`'s command-line tool | a file, serde_json | Not a binding: runs on the process's main thread (8 MiB) |

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
- **Other corpora:** `merge.json` (and its generated and imported siblings), `serialize.json`, and `prompts/`. Their goldens are in `golden/edits/` and `golden/prompts/`. Prompt specs for two libraries are written by the import: the spec fixtures' library, and upstream's react-ui library (`react-ui-schema`), which the viewer asks Claude with, plus react-ui's library and options in cloud mode (`react-ui-cloud`), as upstream's openui-cloud template asks for OpenUI Cloud's config block. The import loads react-ui with React and its other dependencies stubbed out (`reference/react-ui-stubs.ts`), so nothing gets installed.
- **The reference's own oddities:**
  - `QUIRKS.md` lists behaviour the core reproduces on purpose;
  - `pending/` holds a list per golden test of the cases the core doesn't match yet: `parse`, the three stages (`tokens`, `statements`, `preprocess`), `merge`, `serialize`, `enrich` and `prompt`. A listed case may fail. One that matches again, or a listed name that isn't a case, fails its test, so the lists stay exact. Every list is empty today.

## Reproducing the real library definitions

The library-definition tests read `conformance/real-libraries/intelligent-ui.json`.
It contains the converter's plain data and the library's own `toJSONSchema()`
and `toSpec()` output, the `promptOptions` the package exports and the prompt
its library writes with them (`library.prompt(promptOptions)`). Like the imported react-ui schema and prompt spec, it
can be tested and the parser goldens regenerated without an upstream checkout.
To reproduce the fixture from the library commit in `reference/UPSTREAM`, run
from `reference/` with the locked dependencies available:

```sh
BUN_RUNTIME_TRANSPILER_CACHE_PATH=0 bun --no-install --preload ./react-ui-stubs.ts \
  real-library-spec.ts intelligent-ui "$checkout" > "$scratch/intelligent-ui.json"
cmp "$scratch/intelligent-ui.json" ../conformance/real-libraries/intelligent-ui.json
```

At the lang-core pin `47b9cae`, the react-ui fixture holds the converter's
plain data and upstream's default prompt and cloud config. Its schema, spec
and prompt and cloud oracles with the package's options are in
`conformance/imported/schemas`, `conformance/prompts` and
`conformance/golden/prompts`. From `reference/`, run

```sh
BUN_RUNTIME_TRANSPILER_CACHE_PATH=0 bun --no-install --preload ./react-ui-stubs.ts \
  real-library-spec.ts react-ui "$checkout" > "$scratch/react-ui.json"
cmp "$scratch/react-ui.json" ../conformance/real-libraries/react-ui.json
```

The importer checks two fresh loads, one with the package's declarations and
one using the original string and optional string before `Icon.name` and
`Icon.category` were described. Both match the pinned schema, every prompt
signature, the complete generated prompt through both upstream entry points,
and the cloud config, both with default options and with the package's options.

`react_ui_matches_upstream_byte_for_byte` checks those outputs for the
defined library, including all 82 components. A schema or spec difference
names its first differing entry. `react_ui_parses_the_whole_corpus_as_json`
compares all 976 programs one-shot, after each five-character push and at
finish. Both tests run from the checked-in data with
`cargo test -p openui-lang --test library_definition react_ui_`.

## Intelligent UI function results

`FunctionReplayTests` (in `swift/OpenUINative`, `OpenUIIntelligentUITests`)
proves each of the Intelligent UI library's Swift functions against
upstream's own. `reference/intelligent-ui-functions.ts`, which
`scripts/regen-golden.sh` runs, loads upstream's function source (vendored
verbatim from the library commit in `reference/intelligent-ui-functions/`)
and runs every call in `reference/intelligent-ui-function-cases.ts` through
the vendored parser and evaluator. It writes the pairs to
`conformance/golden/intelligent-ui-functions.json`, with the Bun and ICU
versions that made them in its header.

- **A pair** is a program and its result. Most programs hold their arguments
  in state variables so validation happens at runtime, as in
  `root = Probe(@MixColor($a0, $a1, $a2))`. The optional-literal cases write
  their arguments inline so the parser checks them. The result is the probe's
  evaluated value, the evaluation's errors and any parse errors (a missing
  required argument, an extra one).
- **Time zones, clock and draws.** Every call runs in four zones (`UTC`,
  `America/New_York`, `Asia/Kolkata`, `Australia/Lord_Howe`), each in its own
  Bun process, with the clock fixed and `Math.random` drawing a sequence
  seeded afresh for each call unless the case specifies a draw. A call with
  the same result in every zone is written once unless the case requests
  a separate replay in each zone. Zone-specific records include their `zone`. A call that drew
  numbers lists them in `draws`.
- **Values JSON can't carry** are tagged: `{"$num": "-0"}`, `"Infinity"`,
  `"-Infinity"` or `"NaN"`; `{"$utf16": [code units]}` for a string with a
  lone surrogate; `{"$undefined": true}`.
- **Replay.** The test shows each program in a `LiveProgram` whose library
  has the probe and `IntelligentUI.functions`, and compares the value, the
  errors and the parse errors byte for byte. It compares at the binding,
  where a recorded -0 is 0, non-finite results and container entries are null and
  unpaired UTF-16 surrogates become U+FFFD, including in diagnostics.
  `Includes`, `Join`, `Split`, `Replace` and `GroupBy` use Bun controls recorded with
  the arguments the callback actually receives when input repair can change
  a computed result.
  A top-level non-finite number that reaches the call at runtime gives null
  with a runtime error (#160). MixColor's body also replays its three non-finite `t` inputs
  directly. The test pins the counts for both boundary rules per function.
  [Model examples and outcomes](CONFORMANCE.md#intelligent-ui-function-values-at-the-json-boundary)
- **Argument rules.** A pair recorded for a rule names it in `rule`. The test
  holds exact counts of pending pairs per rule and separately reports
  the rules still waiting for their first successful replay.
- **Pending.** Every recorded function must be implemented, or listed in
  the test's `pending` set until its ticket lands; that set must be exactly
  the declared functions with no implementation. A pending function's pairs
  check only that its call gives null with no error.

**Adding a group.** Add its calls to `intelligent-ui-function-cases.ts`
under the function's name, run `scripts/regen-golden.sh` (or
`bun --no-install intelligent-ui-functions.ts` from `reference/`), give the
declarations implementations, and take the names out of `pending`. A group
that draws random numbers uses the source the replay sets from `draws`.
Groups using the clock or a time zone need to supply those from the recording.

### List functions (#161)

`FunctionReplayTests.everyRecordedPairReplaysThroughTheRuntime` reads
`conformance/golden/intelligent-ui-functions.json`. For each implemented
function it compares the evaluated value, parse errors and runtime errors
and checks that the callback consumed the recorded draws in order. A runtime
refusal must consume zero draws. The inputs include the 10,000-item boundaries, deep number/string
equality, object key order, literal Unicode comparisons, negative indices
and fractional ranges. `hostDrawsSelectBothEndsAndShuffleInDrawOrder` also
exercises the host's draw source at zero and the largest Double below one.

`rangeUsesTheSameStepForExplicitNullAndOmission` proves the optional
argument rule. The replay counts pending examples separately from rules
with no successful replay. The list parameters accept arrays of any item,
so `Reverse` retains all four mixed items. `Allocate` (#162) replays the
pruned-array rule on its typed weights. `Stats` replays the other example on its numeric array (#166).

`upstreamItemCapAccepts10000AndRejects10001` checks output sizes of 10,000
and 10,001 for Append, Prepend, Concat, Flatten and Range. The golden also
exercises Toggle growing to 10,001, Range refusing a zero step and Chunk
refusing a truncated size below one. Other loops are bounded by admitted
inputs and outputs under the existing host-function contract.

The comparison sorts in `ListFunctions.key` (object fields), `Top` (rows), `Stats` (median) and `Pivot` (column names) take O(n log n) comparisons over data bounded by host input admission.

### Data functions (#166)

`everyRecordedPairReplaysThroughTheRuntime` checks 287 recorded pairs for all ten data functions,
including numeric and string group keys, property order, missing fields,
mixed rows, stable ties, exact bin labels and local days in the recording’s
four time zones. The replay counts each projected or refused pair by function.
`statsPrunesInvalidItemsAndReportsTheirPath` pins the result and diagnostic
of `@Stats([1,"a",null,2])`. No argument-rule pairs await implementation.
`histogramAndBucketPinUpstreamCaps` covers 1–500 bins and the limit of
10,000 returned periods, including a sparse aggregation beyond 10,000 days.
The recording keeps the legacy date programs in each of UTC, America/New_York,
Asia/Kolkata and Australia/Lord_Howe even when their calendar days agree.
`betweenPreparesEachBoundFormOnce` counts numeric, ISO-prefix, day and UTF-16
conversions, including failed parses and short-circuited bounds.
`movingAverageSlidesWithLinearWork` counts additions for growing windows.
`movingAverageAdmitsOrderedReductionsBeforeAdding` checks the adjacent widths
at both the row-scaled and per-call ceilings and counts zero additions on refusal.
`movingAverageWorkLimitThroughLiveProgram` checks the model call on both sides
of the row-scaled ceiling, including its single diagnostic.
`recordedMovingAveragePairsAreFarBelowTheWorkLimit` checks every recorded
MovingAvg pair against one percent of the allowance even if the ordered fold runs.

`callbacksWriteNullForOverflowedSummaryAndAggregateFields` checks what the
Swift callback writes before the runtime validates the result.
`statsProjectsOnlyOverflowedFields` and the replay verify the summary and aggregate
rows retain null numeric fields and produce no diagnostic. All data
function tests run headless.

### Date functions (#165)

The replay builds one library per recorded time zone and injects the clock from
the golden header. A zone-specific record runs in that zone. For each date
record Bun stored once, all four native sessions must produce identical values
and diagnostics. Counts are per stored record. The nested non-finite input to
`FormatDate` uses the recorded `[null]` control for its error text.

`hostClockAndZoneDetermineNowAndToday` advances the clock on an existing library
across leap day and New Year. `formatDateUsesFixedEnglishNamesAndPunctuation`
pins all eight styles. `dateArithmeticClipsAtOneHundredMillionDays` checks both
ends of TimeClip and the neighboring refusals.
`localParsingUsesTheEarlierOffsetAtGapsAndOverlaps` checks the actual instant
for one-hour and half-hour transitions where the displayed time is ambiguous.
`localParsingAtTimeClipUsesTheOffsetForTheWallDate` checks the millisecond
before, at and after each wall-time limit, including explicit-offset controls.
`formatDateAtTimeClipMatchesBunInNewYorkAndApia` checks the displayed value
and both diagnostic lists for local inputs just outside those limits.

### Number formatting (#164)

`everyRecordedPairReplaysThroughTheRuntime` checks 467 Format and 40 Plural
pairs, including decimal ties, promotion at compact thresholds, grouping
above a trillion, currency prefixes and plural selection before rounding.
When the JSON argument loses the sign of zero, it replays against a recorded
positive-zero control and counts these pairs separately (4 Format, 1 Plural).
`formatAndPluralKeepTheSignWhenTheCallbackReceivesNegativeZero` checks the
callbacks with a signed input. `formatTruncatesAndClampsDecimalPlacesToZeroThroughTen`
pins both ends of upstream’s decimal-place clamp through the runtime.
`formatAndPluralUseUSSeparatorsUnderAFrenchNumericLocale` calls the registered
functions on a thread using the French numeric locale and verifies its decimal
separator is a comma. These tests run without a display.

### Text functions (#163)

`FunctionReplayTests.everyRecordedPairReplaysThroughTheRuntime` replays
305 text pairs. They cover dotted field reads and JSON spelling in Join,
literal separators and replacements, case expansions and contextual final
sigma, JavaScript whitespace, non-BMP letters, lone surrogates, duration
rounding and Default's argument rules. Of 46 runtime refusals for top-level
non-finite numbers, 15 have an optional argument that Bun prunes at runtime
and 18 also have a required null. The replay pins both groups by function
and projects the result of each repaired-input control. Another 15 pairs
pin upstream's values and diagnostics when the parser prunes an inline
literal for Pad's char, Split's sep, Join's sep or field, or Duration's style.

`literalSearchHasALinearComparisonBound` counts UTF-16 comparisons while
building the separator table and scanning near matches at three sizes.
`inheritedMethodResolutionIsIndependentOfRowCount` counts the object and
array method resolutions for a dotted field across 1, 2 and 1,024 rows.

`padCapsTheRequestedLengthAt1000CodeUnits` checks lengths 999, 1,000 and
1,001 on an emoji. `splitCapsTheNumberOfRetainedPartsAt10000` checks
10,000 and 10,001 parts after whitespace is discarded.
`expandedTextUsesTheExistingHostResultLimit` exercises oversized Join and
Replace results. All text tests run without a display.

### Number functions (#162)

The same runtime replay compares the recorded Bun Double's bits with Swift's result
for `Pow` and `Payment`, including square roots, reciprocal roots and the
integer-power cutoff at 1,000. There is no ULP tolerance. It also covers
every unit in `Convert`, all nine temperature directions, allocation ties,
overflow and diagnostics, and the exact draw count for `Random`. Four
`Random` pairs replay against null with empty diagnostics for draws of 0 and 0.5
with overflowing finite bounds in both modes. The replay checks all three
non-finite spellings in the optional integer slot and beside required nulls,
alongside finite controls. A refused call consumes zero native draws, including
when Bun prunes the integer flag and draws once.
`allocateClampsRoundedDecimalPlacesToZeroThroughSix` pins both precision
limits. `allocateAwardsBonusesInLinearVisits` counts remainder visits at four
sizes through 10,001. The runtime replay pins input order for a tie at the
cutoff and compares every part of an unsorted 10,001-weight allocation with Bun.

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

A hunt never commits and never changes tracked files. It compares them before and after, and exits with status 1 if they changed. Differential fuzzing needs Bun and cached `reference/node_modules`; without either it is skipped, and no packages are installed. The other available hunt checks still run. Without network, the upstream check uses the copy of upstream's history from the last time it reached GitHub (cached in `target/upstream.git`), and says so. `--seed S` repeats a hunt's fuzzing, and `--no-upstream` skips the upstream check.

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

## Runtime differential fuzzing

One short run, offline, with Bun and cached `reference/node_modules` already present:

```bash
scripts/fuzz.sh --runtime --seconds 60 --seed 40 --findings target/runtime-hunt --no-save
```

This is a mode of the existing differential fuzzer in `examples/fuzz/`, using
its SplitMix64 per-case seed/index, batch budget, progress files, findings JSONL,
and maximum of 25 saved findings. `--count`, `--from`, and `--only` have the
same meaning as in parsing mode. The first release build precedes the search
budget; the current comparison and final shrink verification can add at most
twelve seconds after it. No dependency installation occurs, in any fuzzer mode.
The reference command is `bun --no-install runtime.ts fuzz`, run from
`reference/`. It uses the same `observe` and `actionScenarios` functions as
runtime goldens, including the attributed renderer transcription, upstream
store, field resolver, query manager, evaluator and private action identities.

Version 2 of the runtime grammar generates programs as well as JSON state
(all six JSON kinds, nested arrays and objects, escaped and Unicode strings).
Six shapes cover forms and form fields, binding markers, tabs, tables, nested
`@Each`, and ordinary component trees, with generated nesting, table rows and
multi-step action bodies. Components use a bounded case-local `fuzzSchema`;
these are runtime component/prop evaluations, not mounted renderer UI tests.
Deferred arithmetic/conditional/container expressions and two retained action
plans remain in every ordinary case. Random interactions precede a
sequence that always exercises settled field defaults, form and binding writes,
an action with two mutation suspensions, host-written state between steps,
true and generated true/false resumes, a repeated-token refusal, replacement
program defaults, query events, reset/open-URL/custom events, a plan retained in
state, and disposal/revoked-plan use. Both default and generated persistence
scenarios run. Rust independently derives the deterministic saved-state recipe
from its own parsed declarations, writes it through `Store::snapshot_json`,
reloads those bytes with `parse_json`, and initializes the persisted scenario
from that result. Version 2 compares the saved map and its raw JSON bytes with
the upstream Store's snapshot too; Rust never receives upstream's saved map.
Initial and subsequent observations compare complete serialized
object order, tree JSON, errors, snapshots, pristine names, ordered events,
save/failure/suspension flags, and parser/tree/action limit flags.
Tree, snapshot and event JSON are also retained as raw strings before parsing;
an independent ordered comparer checks those strings byte for byte, including
number spelling and escape spelling. Only controller numbers use JS double
semantics. Unexpected cuts in ordinary observations are findings. Reference
flags, if supplied, are retained; missing resource flags on default-limit
observations are expected false under the documented no-cut rule.

Each version-2 case also generates nine bounded probes: parse, tree and action
operations under size, depth and work limits of one or two. Their short programs
have 8–16 nested arrays and real computed defaults or deferred state reads.
These admission-cut probes must change the result, set the designated cut flag, and agree with
upstream again at default limits. The controls compare raw parse/tree JSON or
action events, snapshot bytes and failure. No upstream resource limit exists,
so limited output bytes have no differential oracle; cut flags instead follow
the documented rules, independently of the actual flags:

| Rust observable | Oracle |
|---|---|
| `parseSize` | Result room or default-evaluation work exhausted → true (ADR 0001); default-limit control → false |
| `parseDepth` | Expression nesting exhausted → true (ADR 0001); control → false |
| `treeSize` | Shared result room or evaluation work exhausted → true (ADR 0003); control → false |
| `treeDepth` | Tree/expression nesting exhausted → true (ADR 0003); control → false |
| `actionSize` | Result room or run-wide work exhausted → true (ADR 0003); control → false |
| `actionDepth` | Action/input/expression nesting exhausted → true (ADR 0003); control → false |
| Action `failed` | Direct comparison with the renderer runner's rejected-operation observable on ordinary steps and controls; a limited action must stop with a failure under the Rust rule |
| Tree errors | Direct comparison with upstream's structured prop errors; upstream has no separate tree-wide failed bit |

An optional version-2 `evaluationProbes` field adds 24 evaluator boundaries,
without changing the original six pins. Depth limits 1–4 and serialized result
room 3, 4, 8 and 16 each have an under, exact-boundary and over case. Parsing
uses default limits, then `EvaluationSession` starts a public allowance directly:
no limited input-admission walk can mask evaluation. Depth uses unary chains
ending in a live `$probe` read; the existing evaluator permits three internal
walk levels per configured level, including the root visit. Size uses string
literals whose JSON is exactly one byte below, at, or above the room. These are
serialized bytes, independent of allocator or stack layout. Under/at cases must
have no flags and exactly the upstream evaluator's bytes; over cases must change
the value and set only the named flag. Default-limit controls also match upstream.
`runtime_evaluator_boundaries_admit_then_cut_or_match_exact_reference_bytes`
checks `examples/fuzz/runtime/boundary-pins.json`. Scratch controls dropping the
evaluator result's depth flag or cutting one internal walk level early must fail
this test. The grammar explores these specific boundaries, not arbitrary limits.

Size and work share Rust's size flag; upstream has no equivalent to any of the
six resource flags. Probe expectations are explicitly documented-rule checks,
not claimed upstream flags. Generated values and reference data are JSON;
library-function callbacks, asynchronous tools, React effect timing, streaming
prefixes, hostile schema generation, arbitrary descriptor syntax and general
limit-boundary exploration beyond these scalar/unary probes remain outside this
mode. Dedicated conformance, allowance and streaming tests cover those paths. The existing parser mode
continues to cover generated/mangled syntax and random streaming chunkings.

Every 32nd case replays one of the seven already-recorded non-string-action-target
cases. An explanation requires the *entire pair* of action observations to
match the committed reference and Rust divergence pins, and identical initial
observations. A new step or changed observation is reported. Other documented
divergence shapes are outside the generated grammar; there is no blanket skip
based on text containing a suspicious name.

Each generated input value has at most 40 nodes, depth three, container width three, and
strings of at most 24 UTF-8 bytes. A case has at most 64 explicit interactions,
4 KiB per program (including replacements), and 64 KiB of request JSON.
`scripts/fuzz-child.py` supervises each Rust and Bun child separately: a two-second
kill deadline, 256 MiB summed process-tree physical footprint on macOS
(RSS + swap on Linux), 4 MiB stdout, and 64 KiB stderr. The owned group includes
the guard, worker and descendants, even after the worker exits; recursive child
enumeration also includes descendants that start a new session while their
ancestry is visible. After the guard exits, buffered output and EOF are drained;
a still-open quiet worker pipe is a failure (`worker exited with inherited pipes
open`). This detects a new-session double-fork even after reparenting hides its
ancestry, but does not promise to kill that detached process. Descendants that
both detach and close every worker pipe are outside this portable supervisor's
coverage. Input is nonblocking too. A limit kills the owned group
and discovered descendants and reaps the guard; unavailable measurement refuses
the run. Sampling is every 1 ms and triggers at seven eighths of the requested
memory budget (224 MiB for 256 MiB), reserving room for allocation and shutdown.
This reduces sampled overshoot; it is not an OS-enforced allocation ceiling.
A separate group-leader guard watches a pipe held only by the supervisor:
SIGKILL of the supervisor closes it and kills the group. The guard stays after
worker exit while descendants remain. The supervisor also checks loss of its
own parent, so killing the runtime hunt triggers cleanup.
Only the reference command retries a deadline once, in a fresh child under the
same two-second and memory bounds. Other errors are never retried; two deadlines
remain a finding. A retried finding records `referenceSupervision` (original
comparison) and `replayReferenceSupervision` (final verification), including the
policy, deadline and both attempt outcomes, even when retry succeeds and the
finding is a later mismatch. The current comparison and final verification can
therefore use at most twelve seconds beyond the search budget. Retry logic is
checked with deterministic outcomes and a real self-limited hang.
These are harness limits, with no change to library defaults or parser costs.
Per-case children also discard all reference caches and suspended runs. Only one
case is resident at a time; bounded findings are written to disk.

```bash
scripts/fuzz.sh --runtime --seed 40 --only 17 --no-save
cargo run --offline -p openui-lang --example fuzz -- --runtime-case --seed 69 --only 3
scripts/fuzz.sh --runtime --replay target/runtime-hunt/runtime-HASH.json --only 0 --no-save
cargo test -p openui-lang --example fuzz runtime::tests
python3 scripts/pin-runtime-fuzz.py --check  # six v2 pins + evaluator boundaries
python3 scripts/test-fuzz-child.py
```

Mismatches shrink by deleting interactions, source lines and state entries,
under the same deadline and at most 80 attempts. Missing retained handles refuse
replay, so deleting a plan cannot become a false default-action difference.
Crashes, timeouts and reference
errors keep the original request. `runtime-HASH.json` is a directly replayable
case; its `.finding.json` holds the original request, both complete observations, the seed/index,
replay commands and whether shrinking finished. `progress-runtime` identifies
the active seed/index even if the parent is interrupted; `summary-runtime.json`
contains the tallies. Runtime mode always saves to `--findings` or
`target/runtime-findings`, including with `--no-save`; it never automatically
promotes an unreviewed difference into conformance.

Triage every finding: make a small safe runtime fix and add a case to
`conformance/runtime.json` (program plus `stateValues` and `actionInteractions`,
with `actionSequenceOnly`/`manualMutationOutcomes` for explicit resumes), or
record an intentional unsafe-to-match divergence with both pinned outputs and
its reason in CONFORMANCE.md. A reference quirk needing a decision stays in the
maintainer report; do not edit QUIRKS.md or file upstream. The adapters now
support `{kind:"set", name, value}` for direct host writes. Regenerate twice
with `scripts/regen-golden.sh` and run the public conformance tests. Three
additional offline replay controls live in `conformance/runtime-fuzz.json` and
`golden/runtime-fuzz.json`: two generated sequences and an exact known-divergence
pair. `generated_runtime_interactions_match_pinned_reference_bytes` checks them;
`runtime_replay_refuses_missing_handles_instead_of_running_a_default_action`,
`runtime_generation_replays_and_stays_within_bounds`,
`generated_state_covers_json_kinds_with_bounded_shapes`,
`runtime_case_bounds_refuse_oversized_source_and_sequences` and
`runtime_observation_comparison_preserves_object_order_and_flags` and
`runtime_serialized_observations_preserve_number_and_escape_bytes` enforce
replay, bounds, ordered comparison and flag sensitivity.

Six additional version-2 cases and upstream observations are pinned in
`examples/fuzz/runtime/pins.json`. The tests
`runtime_generated_program_shapes_and_independent_persistence_match_reference_bytes`,
`runtime_real_cuts_match_documented_flags_and_reference_controls`,
`runtime_saves_its_own_state_and_reloads_the_saved_bytes`,
`runtime_normalization_keeps_observable_reference_flags_and_failures`, and
`runtime_limit_probe_bounds_and_versions_refuse_invalid_replays` enforce the
new grammar, 54 real cut probes, persistence, flag/failure sensitivity and bounds.
The old three goldens remain unchanged and replay without a version field;
new generated cases declare `fuzzVersion:2`. Seed/index generation now means
version 2; a saved case's serialized program/operations remain authoritative.
`--runtime-case` prints a bounded generated replay without executing it.
`python3 scripts/pin-runtime-fuzz.py` regenerates the six pins and the separate
boundary pin, building offline and supervising each Rust/Bun child; `--check`
refuses any change.
`runtime_recorded_v1_replays_and_one_byte_changes_to_v1_and_v2_pins_fail`
replays an original unversioned (v1) recording and a v2 pin, then changes one byte
of each pin's original tree JSON and requires comparison failure. New evaluator
probe bounds have their own refusal test. The pin script preserves the six old
pins and regenerates the separate boundary pin from the vendored evaluator.
The supervisor's 25 tests use bounded children and deterministic retry outcomes,
without elapsed-time assertions. Every allocator fixture has at most 384 MiB of
deliberate payload across its descendants, leaving interpreter overhead within
the 512 MiB fixture ceiling. Each hog exits independently within 15 seconds
(forks rearm the alarm). The SIGKILL fixtures report a sentinel
if they reach their own exit; EOF passes only when cleanup killed them.
The detached double-fork fixture has only a 32 MiB payload and reports a failure
through inherited pipes; it independently exits even if enumeration misses it.
The reserve check requires the measured footprint to cross its trigger, not to
stay below a hard ceiling that sampling cannot enforce.

`stack::tests::finished_walks_ignore_an_installed_ceiling` installs zero and
recurses with a live frame until the finished walk itself needs growth. It
requires growth and restoration without depending on the OS accepting a tiny
thread stack. Other workspace `stack_size` tests check deep completion, explicit
limit cuts or injected stalls; none requires the first call to grow solely
because of a requested thread stack size.

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
scripts/mutants.sh runtime --list --json > target/runtime-mutants.json  # no mutations
scripts/mutants.sh runtime --jobs 2   # #40: maintainer starts this long run
scripts/mutants.sh query --list       # #95: the query manager alone, 569 with cargo-mutants 27.1.0
scripts/mutants.sh query --jobs 2     # maintainer starts this long run too
```

[cargo-mutants](https://mutants.rs) plants one bug at a time in the source (a `>` turned into `>=`, a function that returns a default, a deleted `!`) and runs the offline suite against each, with live diffs off. A mutant the suite doesn't catch is code a real bug could break unnoticed. Each one is either caught by a new case, or listed in `.cargo/mutants.toml` with the reason it changes nothing a caller can observe, so reruns skip it. A mutant costs a rebuild plus the suite, so a whole group takes hours: run it on AC power, or on GitHub (see [On GitHub](#on-github)). Results go to `target/mutants/<group>/mutants.out/` (`missed.txt`, `caught.txt`, `timeout.txt`).

The runtime group includes every module under `crates/openui-runtime/src/`,
including nested tree modules, and runs the runtime crate's offline public
suite. At #40 list mode enumerated 1,086 candidates in 12 files; that count
changes with source and cargo-mutants versions. There are no runtime exclusions
without evidence: every survivor must receive a public-interface test that
fails with that mutant, or a narrow entry in `.cargo/mutants.toml` explaining
why no caller can observe the change. Limits, accounting, errors and refusal
atomicity are observable; a slow test is no reason to exclude one. Never use
`--in-place` for this workflow. `CARGO_NET_OFFLINE=true` and
`OPENUI_NO_LIVE_DIFFS=1` are set by the script.

Budget many hours to multiple days for the full runtime group, depending on
parallelism and survivor rate. Its unmutated suite alone exceeded a three-minute
sample timeout on the worker, so do not apply that sample timeout to the full
run. Start the command above on the worker with working process discovery and
memory monitoring; inspect `target/mutants/runtime/mutants.out/{missed,caught,timeout,unviable}.txt`.
The manually dispatched GitHub workflow accepts `group=runtime` and
`group=all` (parsing, rest and runtime), and aggregates all three reports.
`group=both` preserves its existing parsing/rest selection.
A deliberately narrow proof run (not a replacement for the full suite) is:

```bash
scripts/mutants.sh runtime --jobs 1 --re 'Store::version -> u64 with 0$' \
  --exclude progress.rs --exclude stack.rs \
  --cargo-test-arg=--test=mutation_contracts --timeout 120 --build-timeout 120
```

cargo-mutants 27.1.0 also enumerates three struct-field deletions despite its
name filters. The sample therefore tests four mutants: the version getter and
fields retaining the exported value and binding/function configuration limits.
`mutation_contracts` checks each through public interfaces, including refusal
flags and the surviving function implementation. The file filters are sample
controls, not permanent exclusions. The maintainer must triage the full run's
survivors before claiming complete mutation coverage.

The query group (#95) mutates `crates/openui-runtime/src/query.rs` and
`crates/openui-runtime/src/query/`, which the runtime group also covers. It
runs the runtime's suite and the session crate's (`--test-package`), since
the session's replays of the renderer's query and mutation effects test the
manager through its only production caller. Results go to
`target/mutants/query/`. The runtime suite takes about 12 minutes per mutant
on the maintainer's Mac in a debug build, the session's a few seconds, so at
`--jobs 2` the whole group is days of work: start it on the worker. Five of
its mutants are equivalent and cannot be excluded: cargo-mutants 27.1.0 lists
"delete field" mutants whatever `exclude_re` says. Each deletes a
`QueryUpdate` field written with the value `..QueryUpdate::default()` already
supplies, so they are expected in `missed.txt`; `.cargo/mutants.toml` lists
them. Removing the explicit fields from the source removes the mutants.

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
| `query` | A query manager scenario in the conformance format (a JSON program, optional library and steps), replayed through the public query commands under the default and the small limits, at most 64 steps: an accepted command's `changed` is whether the snapshot's bytes changed and its deadline is the manager's; a refused one changes no snapshot, deadline, cache entry or call; a completed call is refused the second time; after every step the cache charge equals a fresh measurement within its cap, and the cache and delivery audits the scenario tests use hold. Each parse result gets the checks below. | every committed scenario in `conformance/query-scenarios.json`, `query-runs.json`, `query-refresh.json`, `query-readers.json`, `query-reader-fixes.json` and `mutation-scenarios.json`, one file each, copied in again on every run |

The language targets share a library (`fuzz/fuzz_targets/library.rs`) with every schema shape validation handles, plus 1.0's library functions and actions. The dictionary, `fuzz/openui.dict`, holds fragments of the language and of model output. Inputs stay under 8 KB, the size of a long response.

**The caps** (#10) make slowness and memory findings, not passes: an input over **2 s**, a single allocation over **512 MB**, or the process over **1 GB** stops the target and leaves the input. Every limit the core has (ADR 0001) bounds an input well under them, even in the fuzz build with its debug assertions. The long runs' slow inputs (18.5 s and 12.5 s against lang-core 1.1) replay in milliseconds now, and so do the 180,000 inputs the long runs' corpora had grown to. `scripts/fuzz-coverage.sh` sets the caps, so GitHub's runs and hunts have them too. The stream target caps its pushes because a statement that never closes is read again in full on every push, and a state default in it evaluated again up to the work limit (#22): the work limit for the whole text times the pushes stays within 16 million steps (about 0.2 s in the fuzz build, a tenth of the timeout), with at most 128 pushes. With the cap, a timeout still means a push costing more than it should. The hardening run (#19) found three looser caps too loose: 256 pushes (a 6 KB input whose stray quotes left a statement open took 0.8 s idle), then 128 (an 884-byte default left open and evaluated to the 1-million-step limit on each push took 0.9 s idle), then 64 million steps (a 1.2 KB default left open on each of 53 pushes took 0.85 s on the worker at load 6 and passed 2 s when it was busier).

**The checks** (`fuzz/fuzz_targets/checks.rs`), on every result of `parse` and `stream`, and on each program `query` parses:
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

Every push to `main`, and every pull request, runs the check on Linux with Bun installed, so the conformance tests diff against the reference too, and the viewer's check on macOS 27 (`.github/workflows/ci.yml`). The Linux job also installs the `wasm32-unknown-unknown` target and the wasm-bindgen CLI at the version `Cargo.lock` names (built once per version and kept in the Actions cache), and sets `OPENUI_REQUIRE_WASM=1`, so the WebAssembly packages are always built, loaded and tested there and a missing tool fails the job. Both use the maintainer's stable Rust, 1.97.1, so clippy's lints there are the ones the check passed locally. The long runs don't run on their own: each is a workflow started by hand, from the repository's Actions tab or with `gh`:

```bash
gh workflow run mutants.yml -f group=parsing -f shards=16       # or rest, runtime, query, both, all
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
| `OPENUI_REQUIRE_WASM=1` | `scripts/check.sh` fails, instead of skipping with a note, when it can't build and test the WebAssembly packages (wasm-bindgen at the locked version, the `wasm32-unknown-unknown` target or Bun missing). CI sets it. |
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
Deferred-AST tests credit supplied nodes before their measurement work,
including a shared DAG with 2^40 virtual leaves and more than 4 GiB of virtual
JSON. A 200,000-node AST and its plain-JSON form fit default limits. Fixed work
still cuts and output occurrences still pay their serialized room.
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
165/178 after stripping without bindings. Eleven repeated data markers fit
main's 3030/5946 baseline
and now pin 214/578 exactly. Each boundary cuts one unit below.

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
set expressions, returned data, admission cuts, and the functions of a library
definition (#134), which evaluate as the same library's upstream JSON does.
`function_allowances` uses
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
`query_runs` (runtime) asks for 30,000 runs of a query while its call is in flight
and checks that exactly one more call follows and the cache keeps one entry;
checks that a refused completion keeps the call and its remembered run, that
a remembered run calls with the arguments of the query that asked, that a
run with no query left on its key is forgotten, that a run whose delivery
moves the key's only holder away marks the entry stale so a return to the
key refetches (#119), and that the action runner
hands a query's `@Run` over in step order instead of a host event only when
asked to. Returning to a cached key after a late delivery shows the original
data without a call, including when only a suspended reader held the key at delivery (#198).
`query_runs` (session) compares `conformance/session-runs.json`
with `golden/session-runs.jsonl` from `reference/session-runs.ts`, and checks
that a session without a provider still emits the `run` event, that while
streaming a run and Retry act on the queries registered so far, and that
detaching forgets a remembered run. The native and WebAssembly bindings each drive a refresh and a Retry.

`action_run_budget` uses actual parsed 4,000/16,000-step set/reset comparisons and
a separate 20,000-step default-budget case.
Allocation counts stay below 20 per step plus 2,048, with a linear scaling bound.
Its expensive-loop plan pins shared work and preservation of earlier writes.
A second allocation ratio crosses 5,000/20,000 program steps with 8/32 events over
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
allocator calls, independent of scheduling. They cover shared immutable program
input across resumes and O(events × state) payload copying. Allocation-free CPU
scans are outside this measure.
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

## MCP tool call outcomes

`openui-runtime/tests/mcp_results.rs` compares every row byte for byte with
the output of `reference/mcp-results.ts`. The adapter calls the vendored
extractor and records stable reasons for JavaScript TypeErrors. Separate
tests exercise typed work, byte and depth refusals, including ignored input,
expanded text, projected JSON and valid and malformed 12,000-level input on
a 64 KiB caller stack. The native export and built WebAssembly package replay
the same table and check error kinds and supplied limits. The package test
also covers guard conversions and deep projection under a raised limit.

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

`scripts/check.sh` runs `scripts/wasm.sh`, then `bun --no-install test crates/openui-wasm/js`.
`session.test.ts` drives nine scripts and attack/cap/reentry cases.
`lifetime.test.ts` checks that every free goes through the guard and that no
argument conversion runs application code (hostile `valueOf`/`length`/getters,
every member of every freed object). `errors.test.ts` patches `JSON`,
`Reflect`, `Error`, `TextDecoder`, `WebAssembly.Table` and an
`Object.prototype.kind` setter and checks results are unchanged and sessions
stay freeable. `package.test.ts` checks exports, the module's import allowlist,
pointer/private-helper hardening, the web build, and that mutated glue with an
uncovered member refuses to load.
`pointer-redirection.test.ts` (#211) runs the free-redirection reproduction
for all fourteen classes (own property, subclass field and constructor,
setPrototypeOf with a subclass or Proxy prototype, a Proxy receiver; through
free and `[Symbol.dispose]`), checking the victim keeps its pointer and state,
and scans both generated packages: a generated read through an instance or
argument other than the pointer field (or `__destroy_into_raw` in free and
run's retained token) fails the test.
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
threshold pairs. Main outputs, cuts and exact work/room remain pinned. The
proposed #55 room refund is deferred because of the review peak conflict. The test
does not assert a universal flat memory bound; active templates still cost space
with depth. Existing `evaluation_reuse` retains its 384-configuration leaf grid
and its 5/20-times-room shape bounds.

`temporary_array_semantics` (128 MiB cap) pins an otherwise uncut eager temporary:
32–32,768 opaque host strings, followed by a callback choosing an array index
after all producers ran. Only one leaf reaches the result. The test records heap
peaks without a minimum-peak assertion that would forbid future compaction. It is a compatibility counterexample
to a universal peak/room formula (ADR 0008), not an amplification safety test.
`peak_measurements` continues to expose the default-limit 2,652-row read/unread
`Each(L,i,L)` peaks.

`admission_helpers` (64 MiB) pins adjacent serialization bytes, bounded refusal
before copying caller-edited results, incremental/atomic configuration caps,
prepared-token cuts and compact refused-reference tails. Bulk preparation
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
`prepared_reference_compatibility` (512 MiB) compares raw and prepared whole
reads at lowered and default depth limits, including deferred AST values. It
pins complete scalar AST JSON sizes and refusal of unpaid core-measured input.
Preparation remains separate from value admission; the public constructors no
longer accept byte claims or combine caller-sized reference iterators.

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
  callback-thread reuse and destructor reentry. Its seeded stress runs 8 seeds, 4 callers
  and 40 operations per caller, mixing slow callbacks/destructors, replacement,
  disposal and calls. A child-process watchdog detects stalls; tests assert
  results and bounded progress, never execution speed. Other-thread calls queue.
- Both bindings' `runtime_followups.rs` pin element-shaped state/query/tool data,
  custom identity refusal, expression-kind literals, opt-in path bounds and
  seven upstream data observations plus 82 streaming EditableTable prefixes.
- `openui-ffi/src/runtime_retained_root_test.rs` measures representative retained
  heap and release on the owning worker, as a unit test; `openui-runtime/tests/owned_references.rs`
  compares owned/borrowed preparation bytes and flags under independent limits.
  The wasm follow-up test also bounds repeated-draw allocations with 2,000 unread
  references, detecting accidental remeasurement without wall-clock assertions.
- Generate native bindings into a temporary directory, then run
  `python3 scripts/check-runtime-binding-api.py SWIFT_FILE KOTLIN_FILE`.
  Swift `RuntimeFunctionTests.swift` and Android `RuntimeFunctionTest.kt` cover
  callbacks and data-backed actions; `RuntimeSessionTest.kt` covers UTF-16 recovery.
- With the wasm target and matching wasm-bindgen CLI installed, `scripts/check.sh`
  runs `scripts/wasm.sh` (both package load checks), then
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

`parser_materialisation` compares all four empty-name `$ref` signatures and
the 287-byte nested-action program from #202 with the complete parse bytes
in `conformance/golden/parser-call-results.jsonl`. Regenerate them with
`bun --no-install parser-call-results.ts` from `reference/`. The action
case also checks streaming and a smaller fixed limit. Separate cases cover
an unused action default and replay with less room for temporary arguments.

`parser_prototype_divergences` tests the separate twelve-case corpus in
`conformance/parser-prototype-assignments.json`. Complete results and all pushes
after `$v` is assigned must retain archived main's bytes. Every prefix and finish
is pinned, alongside upstream outputs generated by
`reference/parser-prototype-assignments.ts` during `scripts/regen-golden.sh`.
Earlier prefixes can still differ from main under the intended #52 ordinary
inherited-validation fix. These intentional divergences use explicit expected
results, not a pending list. `discarded_prototype_assignments_keep_value_equality_and_debug`
additionally protects public equality, Debug output and the non-dynamic flag.

## Documented divergence pins

`core_divergences` reads 33 cases from `conformance/core-divergences.json`.
`reference/core-divergences.ts` writes the independent reference golden during
`scripts/regen-golden.sh`; the test compares it exactly with the reference
expectations in `divergences/core-divergences.json`, then checks each complete
public Rust result and cut flags (or JSON-reader error). No Bun is needed by
Rust tests. The pin records whether outputs agree, including the 65-value
inconsistent-comparison sort and the controls for corrected table examples.
Bun/JavaScriptCore error messages and native function text are engine-specific.
Unsafe OOM and recursion-limit probes are excluded explicitly in CONFORMANCE.md.

The runtime tree harness checks both sides of every divergence stream, including
step counts and chunkings. The existing function-prototype pin retains full
observations; other pins store reference fingerprints and either `"reference"`
for matching Rust steps or a distinct Rust fingerprint. Action-target pins check
both run and disposal. The cycle, native reset and undefined argument drivers
write independent goldens; their tests compare every reference field against
the divergence pins. Parser prototype tests compare complete reference streams.
No test offers an automatic divergence-pin update mode: capture changes in a
scratch edit, inspect them, and restore the assertions before running checks.

## Counted runtime and streaming follow-ups

The two #61 reuse tests count thread-local allocation requests. Empty pushes
exercise a wide completed tree and an evaluated default, with an absolute
per-push bound. Obsolete-reference updates compare narrow and wide replacement
trees after warming the replacement. In a scratch copy the first test fails
when either evaluator reuse or the completed cache is removed; the second fails
when the completed cache or all streaming reuse is removed. The state-key credit
pin also requires zero allocation requests on unchanged repeated reads.
All performance assertions count allocations or work, including `rebuild_cost`,
`scaling`, `streaming_cost`, `action_run_budget` and `action_mutation_cost`.
There are no elapsed or CPU-time assertions. Counts constrain allocation traffic;
they do not measure allocation-free scans. Timing remains only for benchmark
reporting and child-process watchdogs.

`runtime_review_regressions` (512 MiB) pins changed scalar credit after each
mutation resume (growth, unchanged values, changing back and many names), exact
scalar/key credit across container transitions, credit after an undefined scope
read, and zero repeat-copy allocations, shallow state/raw/prepared
reads past deep unvisited siblings, 760/761/762/800/1,100-level query receivers,
whole-value coercion cuts, and same-session re-entry through both callback paths.
Its re-entry case exhausts work or room inside the nested call and checks the
owning session's flags. The undefined snapshot round trip restores declared
defaults and drops undeclared undefined names. `review_peaks` (512 MiB) uses the
reviewer's repeated-reference sources with independent portable bounds of
208,844,311 (read) and 208,844,244 (unread) bytes, plus the unique-leaf cut bound
of 3,900,052 bytes.
The bound covers these shapes only.

`accounting_followups` (256 MiB) pins five-byte false projections, scalar
payload credit once per consecutive scalar value/name, retained-result-independent work over 128 generated
forms, unchanged argument-text projection traffic across 32 cache misses, weak
shared-program ownership and mutation argument invalidation, omitted undefined
snapshot entries, oversized unused messages, order-independent partial input
credit, and the typed/data concatenation credit and room prices documented in
CONFORMANCE.md. Its 200,000-node AST/plain-JSON test owns more than 64 MiB of
input and therefore lives outside the 64 MiB `allowances` binary. Argument
projection reuse does not remove validation's own copies or change work charges.

`streaming_default_notifications` replays a live-reference-generated byte stream
under seven scenarios: no edit, plus edits before and after completion for
arrays, objects and expression defaults individually. Both
parsers' default identities and every Store notification position are pinned;
values and exported snapshots match at every step. Regenerate from `reference/`
with `bun --no-install stream-default-identities.ts >
../crates/openui-runtime/tests/streaming-defaults.json`.

`runaway_finished_walk` (1 GiB) walks, serializes and drops 4,100,000 AST levels
on a 64 KiB thread. Run additionally with
`CARGO_PROFILE_TEST_OPT_LEVEL=0 cargo test -p openui-lang --test runaway_finished_walk`:
that build reproduces the old debug runaway panic; the normal optimized test
profile has smaller frames. Healthy stack observations reset only the runaway
progress guard; configured evaluator depth/stack cuts remain unchanged.

`nested-loop-allocation-calls.tsv` bounds the native/data substitution grid's
allocation calls with 16 calls of platform headroom. Restoring the replaced-key
String allocation fails the multi-row cases. Freeing values the evaluator holds weak references to takes
them apart on the heap instead of by recursion (#242), which costs a few hundred
more calls in two rows (native, 3 rows, depths 5 and 6: 37,700 to 37,821 and
123,653 to 124,017), whose pins moved to the new counts. Original output hashes and one-row prices remain independent checks, including
main's cut results and room boundaries. The proposed nested-rebuild refund is
deferred because it fails the review's peak requirement; its extra fixtures and
public scope method are removed. `rebuild_cost` checks linear allocation-request
traffic for retained row results and wide argument-cache misses over 1,000/6,000
fields; unchanged string-copy traffic is separately pinned in
`accounting_followups`. The borrowed name index avoids repeated field scans,
but allocation counts do not establish a CPU bound for allocation-free scans.
State, raw query and prepared query lookups use main's permissive receiver rule:
only consumed parts impose expression depth. Whole-value output and coercion
still enforce depth at their actual use. The depth fixtures pin those use
boundaries, both orders and representations, plus the generated union of
isolated uses. This is not a one-level adjustment.
The changed fourth columns of mixed/reuse work fixtures record approved false
projection and ownership-cache prices; archived main columns are retained.

The default path retains ADR 0008's compatibility obstruction to a universal
heap bound. #55 adds only the opt-in portable evaluation allowance in ADR 0011;
its accounting scope and exclusions are explicit. Finite shape checks and
caller-input measurement are not a universal peak bound.

Drawing retains its historical setup saturation guard separately from input
credit. `drawing_keeps_the_existing_saturated_setup_bound_before_unread_deep_declarations`
pins a caller-built program with a 44 MB U+100000 string and an unread
2,000-level array. In main's visitation order the six-unit high-codepoint
estimate saturates the guard before the deep array, so the program completes;
reversing declarations reaches the depth cut. Using actual JSON string bytes
for this guard would cut the formerly completed ordering. That estimate remains
only in `program_setup`'s refusal check; it never grants input credit. Program
credit itself uses one core measure shared by drawing and action runs. Removing
or modernizing the historical guard is deferred because it conflicts with
main's completed results.

## Shared substitution and accounting review (#49 item 2, #64)

`evaluate/substitution.rs::walk` is the one recursive substitution traversal for
native syntax, descriptor data and mixed templates. Representation adapters
handle child lookup and construction. Arrays and positional arguments share
`rewrite_list`: native lists copy only after a change; descriptor lists still
allocate their eager scratch. Descriptor object entries keep their permissive
string/pair destructuring, missing-field errors and assignment semantics.
A descriptor wrapping a native AST retains both entry charges. The parser uses
static dispatch and its zero-sized policy, without converting its template.
Native assignment construction deliberately clones the target before visiting
its value, including when that visit finds no change, preserving main's parser
allocation order.

`substitution_compatibility` uses a 128 MiB allocator. Its
`substitution_preserves_main_values_failures_flags_work_and_room` compares 60
independently captured `388ede7` observations in `support/substitution-main.tsv`:
16 templates in native/data/mixed forms and 12 permissive or malformed data
shapes. Each record pins the full result, remaining work/room after the ordinary
observation, and a fingerprint of all work limits 0–256 and all room limits
0–256 (30,840 boundary observations). The templates exercise every traversed
expression kind, nested/shadowed loops, unchanged members, assignments,
missing fields, scalar children, string entries and array-valued mapped props.
`unchanged_assignment_templates_preserve_parser_allocation_counts` separately
pins 290 allocation requests for a warmed public parse with four unchanged
assignment templates. Moving the assignment target copy after the child visit
fails that test, even though the output and work prices are unchanged.

`reentry_accounting` uses a 64 MiB allocator. Its
`reentry_charges_nested_reads_and_projection_to_the_owning_session` re-enters
the same session from `$z`'s state callback with a fresh scope, evaluates
`[$x,$y]` where `$x=100` and `$y=20`, and returns that value. It pins every work
limit 0–40: exactly 29 units complete; lower limits propagate sticky cuts.
The 29 units are four expression visits, four scalar measurement visits,
nine inner projection units (including the evaluator's historical array
punctuation price), four returned-array measurement visits, and eight outer
projection units. `reentry_releases_inner_projection_room_without_refunding_its_work`
pins room limits 0–20: nine units are needed during the nested call; its
sequential scope releases that room, leaving eight units spent by the outer
result. No evaluation work is refunded. The suspended callback's account must
be registered before host code can re-enter it. Removing that registration
fails both tests; spending the temporary account and losing its work/flags is
not an intended discount. These are current main's charges, with no new limit.

`review_peaks::runtime_container_capacity_keeps_both_review_peaks_below_main`
caps read and unread repeated-reference witnesses independently at
208,844,311 and 208,844,244 bytes (5% above macOS peaks of 198,899,344 and
198,899,280 bytes). These are portable test shape caps, not new runtime limits.
The one-row, one-level witness has a 13,632-byte cap (9,536 measured + 4 KiB)
and its complete 1,225-byte result. The three-row/eight-level cut has a
3,900,052-byte cap (3,714,336 measured + 5%) and its exact 163,573-byte result.

Profiling that small witness on the 64-bit host reproduced the prior
8,016-byte peak and current 9,536-byte peak. The extra 1,520 bytes comprise:

- 1,240 bytes retained by scalar-credit comparison: 1,008 for the 1,000-byte
  string with its allocator header, 16 for its copied name, and 216 for the
  initial scalar index table. Omitting only the comparison copy gives 8,528;
  omitting the scalar index gives 8,296. Neither is an acceptable optimization
  of the required consecutive-value credit contract.
- 144 bytes in the three added, initially empty metadata map headers (metadata
  is 496 bytes versus 352), including argument-string recycling and state/key
  credit indexes. The state-key table itself is unused in this drawing witness.
- 16 bytes from the optional large input-measurement index in the live saved
  accounts (`InputIndex` grows 48→56 bytes, `Saved` 648→656). Replaying with the
  previous compact index gives 9,520 bytes; removing callback-slot registration
  alone leaves 9,536, so that registration does not explain this peak.
- 120 bytes of outer prop capacity allocated before its child is evaluated.
  With the previous core, using the current tree code gives 8,136 bytes;
  restoring lazy `JsObject::new()` for those output props gives 8,016 again.
  This changes overlapping lifetimes, rather than the leaf's payload size.
  Keep that eager capacity: it avoids oversized retained prop buffers in the
  large repeated-reference witnesses, while the small witness is bounded
  separately. Removing it to save 120 bytes would give up that optimization.

These measurements explain this finite witness; the test enforces its portable
total upper bound, not a universal decomposition or peak bound. All figures include
`test_alloc` headers. The unrelated scalar-credit and argument-reuse tests
continue to enforce their accounting and allocation contracts.

The nested-rebuild room refund stays deferred under ADR 0008. The existing
`nested_loop_memory::one_item_loops_keep_unique_leaf_memory_instead_of_doubling_it`
checks independently archived work/room thresholds and output/cut hashes; the
unique-leaf cut above protects the peak that the attempted refund doubled.
Changing serialized room without proving the temporary's lifetime ended would
admit more simultaneously live results. No refund or new limit is introduced.

Eight isolated scratch negative controls were caught: replacing a binary right
child (`substitution_preserves_main_values_failures_flags_work_and_room`),
removing unchanged assignment target copies (the 290-allocation parser test),
using define instead of assignment for descriptor mapped props
(`an_omitted_prototype_prop_is_not_restored_by_sharing_substituted_bodies`), losing
the suspended owner (both re-entry tests), refunding loop room (the nested-loop
price test), retaining a second scalar copy (the small peak bound), padding
only the read witness above its own old cap (the repeated-reference read
bound), and restoring the removed export (the crate's `PreparedInput`
compile-fail import). The scratch tree was restored and its public pins passed.
No mutation or fuzz configuration was changed.

`CHANGELOG.md` records the removed `PreparedInput` export and the distinct
meanings of `PreparedReferenceInput` and `SuppliedInput`. The crate's
compile-fail import enforces that the old export remains absent.


## Substitution and binding refund regressions (#66, #68)

`substitution_cost` uses a 64 MiB allocator cap and public expression sessions.
`substitution_leaves_object_keys_named_like_the_loop_variable_untouched` checks
`@Each([2,4],i,{i:i,other:i+1})` in typed and descriptor forms against the pinned
upstream evaluator's `[{"i":2,"other":3},{"i":4,"other":5}]`.
`unchanged_typed_lists_allocate_independently_of_their_width` compares allocation
traffic for unchanged width-4 and width-2,048 lists under a substituted call.
`unchanged_descriptor_lists_keep_their_eager_scratch_allocation` separately
requires width-scaled scratch traffic in descriptor lists, preserving their
existing eager `.map` cost rather than applying typed-list laziness to data.
`assignment_bodies_escape_mapped_sharing_without_changing_allocation_or_peak`
compares allocation traffic and additional peak heap with mapped sharing opted
in/out, for both representations under an assignment.
`positional_arguments_precede_mapped_props_at_every_work_cut` pins 162 main
observations: typed/data forms at work limits 0–80, with complete public
results in `support/substitution-argument-order.tsv`. The results were captured
with main 767d4c4's unchanged core, outside parser setup. Allocation bytes are
excluded; same-process call-count plateaus separately pin traversal order.

Scratch mutants that make descriptor lists lazy, eagerly copy typed lists, retain the sharing policy through
assignments, or visit mapped props first fail those respective counted tests.
Disabling typed or descriptor mapped-body sharing independently fails the existing
`nested_loop_memory::one_item_loops_keep_unique_leaf_memory_instead_of_doubling_it`
128,000-byte peak assertion. Substituting object keys fails the new key test in
each representation. These are finite shape/count checks, without timing claims.

The `bindings` tests
`removing_bindings_restores_exact_escaped_name_room_across_repeated_cycles` and
`removing_undecided_and_resolved_bindings_refunds_each_retained_component_once`
exercise exact escaped-name refunds, duplicate insertions, absent removals,
shared component names, undecided annotations, and refused insert atomicity.
The first runs 1,024 cycles of the original ten-byte example and 128 cycles
for each of 32 name/component-sharing combinations;
the second fills all 512 bytes again after clearing an extracted configuration.
Restoring the old refund, omitting component JSON escapes, or mutating before
an insertion refusal fails these tests. No default quota is introduced.

## Chat app preferences (#153)

`ChatViewTests` mounts the welcome and composer, checks their accessible copy,
and activates each starter. It also mounts the theme toggle in both system
schemes, checks both flips and relaunches with the same preference store,
and verifies the effective appearance in both the chat and Settings windows.
`ChatPreferencesTests` pins the 17 grouped models and checks request model
IDs across turns, threads and recreated preferences. It replays both HTTP and
server-sent default-model refusals alongside unrelated and overlapping
failures. It covers structured codes, code-less messages, mismatched model
IDs, a redacted callout, and an HTTP error body that stays open at the existing
64 KiB cap. It compares the decoded cloud and full system messages with the
two react-ui goldens and checks that new preferences start in cloud mode
while the original instance is still in full mode. `ChatThreadTests` checks
that a failed composer send restores seeded history and leaves the next
request free of the failed turn. The terminal-marker test holds the response
open and counts the bytes pulled through `[DONE]`. Gateway calls use recorded
exchanges and an in-memory key store. Preference tests use an in-memory store.

## Swift issue #63 regressions

From the repository root, run:

```sh
swift build --package-path swift/OpenUINative
python3 scripts/check-swift-tests.py swift/OpenUINative
python3 scripts/check-swift-tests.py tools/viewer
scripts/check-viewer.sh
```

The Python command is the guarded `swift test` command for each package: it
runs `swift test list` to discover the expected tests, then requires a completion
or a named headless skip in Swift Testing's event stream for every declared test.
SwiftPM runs `OpenUINativeTests` and `OpenUIIntelligentUITests` one after the
other. Each target must print one passing summary with its declared count,
and the runner must exit zero. A bare `swift test` exit status alone is insufficient: an early runner exit can be zero. Counts
are discovered each run, including top-level and parameterized tests (cases
are not separate declared tests). The whole-package guard rejects filters.
`scripts/check-viewer.sh` applies it to OpenUINative, the viewer and the chat app,
and checks the other packages even if the native tests fail.
`python3 scripts/test-check-swift-tests.py` exercises the guard through its
CLI with incomplete, failing and passing runners.

`SwiftTestSupport` is shared by the two native test targets. Attach
`NativeTestWatchdog()` to a test or suite to give each case a fixed 60-second
wall-clock deadline. Call `NativeTestWatchdog.waitingFor("…")` immediately
before a potentially blocking operation. Helpers can pair `beginWaiting`
and `finishedWaiting` to restore the caller's label on return. Labels do
not affect the deadline. Cases with longer existing allowances run
outside the trait.

A fired watchdog names the stalled case and operation in the log, then
spends up to ten seconds capturing in-process backtraces and attempting
`/usr/bin/sample` before exiting with failure. The reports contain test
names and stacks. The viewer job sets `OPENUI_SWIFT_TEST_STACKS` on its
40-minute test step and uploads the reports as `swift-test-stacks`.
Locally the watchdog uses `swift-test-stacks` in the system temporary
directory by default.

The Swift gate has a single ten-minute deadline, including discovery,
audit and cleanup. It reserves its final second for cleanup and exits
with failure if cleanup itself stalls.

An outer sandbox may require `--disable-sandbox` and writable
`CLANG_MODULE_CACHE_PATH` / `SWIFTPM_MODULECACHE_OVERRIDE` directories. Append
`--disable-sandbox` to the build, guarded-test and check-script commands when
needed; the check script still enables warnings as errors.
No dependencies need to be installed. The offline validation observations run from
`reference/` as `bun --no-install ../tools/viewer/Checks/validation-reference.ts`.

The public-interface regressions include:

- `aSecondClickChecksTheCurrentFormBeforeAnyRedraw`: require a field, then send
  without a UI redraw; no event is delivered.
- `everyUpstreamFieldParticipatesInValidation` (seven component types),
  `selectionDefaultsAndStoredShapesFollowUpstream`,
  `aggregateValuePropsAreValidatedWithoutDiscardingTheirData`, and
  `aCustomComponentsRegisteredValueIsValidated`: required fields, structured
  values, selection defaults and an app's registered component.
- `onlyTheVisibleTabIsValidatedIncludingAfterSwitching` and
  `anAppsTabsValidateEveryContentWhenNoNativeSelectionIsRecorded`: native initial
  selection and switching, and app-drawn Tabs with both contents visible.
- `emptyNativeFieldsCannotUseInvisibleCompositeDefaults`: list and record defaults
  in built-in Input and Select block Send without changing the store; a subsequent
  scalar field value permits Send (four parameter cases).
- `eachRule`: ignored list-form rules, ASCII digits, absolute end anchors, email
  newlines, empty URL hosts, large ports and no-break-space prefixes.
- `tableActionCellsValidateTheirCurrentForm`: the repository's action cells
  refuse invalid forms without changing the store and send once valid.
- `snapshotFieldReadsKeepObjectsWithoutAValueKey`,
  `undefinedFieldWritesAndLookalikeDataMatchTheRuntimeGetter`, and
  `identicalButtonsAndMarkersFromAnEarlierShowAreRefused`: data objects and
  cross-show authority, including unchanged store version on refusal.
- `parsingAndReleasingDeepJSONOnA512KBStackIsBounded`: arrays and objects at
  depths 0, 250, 386 and 511 parse and release on a 512 KB thread; depths 512,
  4,000 and 100,000 throw JSONError. This is a debug-build stack check, not a
  wall-clock assertion. The old 4,000-level recursive allowance is replaced
  by iterative parsing and a 512-level refusal boundary.
- `enteringASelectsCurrentValueDoesNotApplyItsTransform` and
  `selectingTheAlreadySelectedValueDoesNotApplyItsTransform`: public entry and
  actual menu selection leave state/version/pristine status unchanged, while
  a different selection applies the transform.
- The old viewer `aFailedFinalParseStillClearsTheStreamingGate` checked flag
  clearing only; #71 replaces it with complete-program restoration tests below.
  The mounted
  `aMountedStoreDraftKeepsEditsAndCommitsTheCurrentValue` edits a real text field,
  changes its store underneath, presses Return and checks a subsequent update.

Native UI suites share a serialized parent to keep their windows and focus from
overlapping. Their redraw helpers, and the viewer's two mounted-view tests,
suspend asynchronously instead of calling `RunLoop.main.run(until:)` from a test
already running on the main executor.

The package's runner used to exit with status 0 and no summary after the select
tests on a Mac with a display. Swift's async `main` drains the main actor with
a single `CFRunLoopRun()` and calls `exit(0)` when it returns. Traced in lldb,
the old helper's menu (a Timer choosing the item and cancelling tracking with
animation, inside a nested `RunLoop.main.run(until:)`) left HIToolbox's event
thread queuing a `CFRunLoopStop` block on the main run loop for later
window-server events (`PushToCGEventQueue`); one such block ran in that outer
loop during a later test, and `swift_task_asyncMainDrainQueue` exited. The
helper now chooses the item and cancels tracking without animation as soon as
the menu begins tracking, and picks again in a fresh `Window`; traced again,
every queued stop block is consumed inside the menu's own event wait. The
completeness guard catches a recurrence.

Five tests open a real AppKit Select menu through `Window.pick`. AppKit needs
an `NSScreen` to position the popup. When `NSScreen.screens` is empty (as on
the SSH worker), Swift Testing skips each of these with the reason “No macOS
display is available”.

- `pickingFromItsMenuWritesItsBinding()`
- `pickingInAFormWritesItsField()`
- `aSelectsRulesAreChecked()`
- `selectingTheAlreadySelectedValueDoesNotApplyItsTransform()`
- `everyOptionOfALongSelectIsInItsMenuAndTheLastCanBePicked()`

Two Intelligent UI tests use the same display condition. AppKit needs a
numbered window for the trackpad event sequence and a screen to present
the anchored popover.

- `horizontalSwipeMovesTheCarouselToAnotherItem()`
- `popoverOpensAtItsTriggerAndDismissesOnKeysOutsideClickAndScroll()`

The gate probes AppKit independently and requires exactly one skip for each
name on a headless Mac, including the two Intelligent UI tests. All seven
run on a Mac with a display and in GitHub
Actions' `viewer` job (`scripts/check-viewer.sh`). Both require zero skips.

The 14 mounted [HtmlView tests](#htmlview-drawing-183) use the same display
condition and are also counted by the gate.

On a Mac with an available display, run the commands above, then repeat both
complete suites ten times (from the repository root):

```sh
for run in {1..10}; do
  python3 scripts/check-swift-tests.py swift/OpenUINative || exit 1
  python3 scripts/check-swift-tests.py tools/viewer || exit 1
done
```

The remaining #71 work and its tests are described below.

## Swift issue #71 compatibility, controls and failure recovery

The commands above still apply; rebuild `scripts/swift-binding.sh` first, then
build both Swift packages. The two offline probes are documented in
[tools/viewer/Checks/README.md](tools/viewer/Checks/README.md). Their committed JSON
observations are read by the Swift public-interface tests. The field probe runs
the pinned react-ui genui callbacks with inert React hooks/visuals and the
vendored state-field resolver, rather than installing React or starting a browser.

`upstreamValidationObservationsMatchNativeSendChecks` mounts forms and checks
Send against every upstream observation: all six URLs, digit/word/space classes,
word boundaries, flagless property escapes, the two named character classes,
valid scoped modifiers and ignored invalid bare inline modifiers. `eachRule` also changes its old Unicode
word expectation to JavaScript's ASCII answer. Pattern flags cannot be expressed
by upstream's string-only rule. Remaining regex and URL differences are listed
with the probes; this is not full ECMAScript or WHATWG conformance.

Native `Issue71` uses the imported react-ui schema and prompt spec:

- `everyNewControlWritesItsFormShapeAndUnblocksSend` covers all seven new types,
  refuses a required empty Send without a store change, writes through the
  public entry method, checks the event's exact value and component metadata,
  and checks streaming refusal. Its expected values are the callback fixture.
- `newStateFieldsWriteTheirBindingAndRefuseStaleElements` covers all five
  upstream `value` bindings and rejects stale entry without a state/version change.
- `aNewControlsDrawingChangesWithItsCurrentValue` compares a mounted control's
  pixels before/after a stored-value update. It uses no machine-specific golden.
  `anAppCanReplaceEveryNewControl` checks the public replacement hook for all seven.
- `clickingAChoiceWritesItsNativeControl`, `typingInTheTextEditorWritesItsBinding`,
  `movingTheSystemSliderWritesANumberArray`, `editingTheSystemDatePickerWritesAnISODate`,
  and `dateRangesKeepFromAndToAsISOStrings` exercise the actual mounted system
  controls, using mouse/text events or their standard AppKit target/action.
- `selectionDefaultsHydrateMetadataAndSingleChoicesCanBeCleared`,
  `aRequiredRadioDefaultCanBeChosenOnScreen`, and
  `disabledSelectionItemsRefuseClicksWithoutChangingState` cover defaults,
  undefined single selections, an already-selected sole radio option and
  disabled options.
  `visualDefaultsMatchTheirExplicitStoredValues` compares visual defaults with
  explicit stored values for RadioGroup and both react-ui array / bench scalar
  Slider defaults, then verifies a different value changes the pixels.
  `invalidSliderBoundsRefuseBeforeConstructingASystemControl` covers reversed/equal
  bounds, discrete zero/negative step, nonfinite
  bounds/step/span. Large finite step-position counts now draw tickless rather
  than being refused; Round 2 hosting tests cover these paths.
  `duplicateCheckboxNamesShareTheLastDefaultAndOneStoredBoolean` checks the
  source callback fixture for duplicate names; range editing checks key order
  after changing an already-populated start date.

`Session.push` and `Session.set` return preview steps; `finish` returns a complete
step. Merge playback explicitly marks its intermediate steps as previews too.
A `LiveProgram` keeps one extra preview session while streaming and retains its
actual complete session. Only the disabled preview drawing uses projected JSON;
that projection never replaces the complete store or its custom-action identity.
Whole-program parser adapters are marked with `asStreamingPreview()` before
presentation; `wholeProgramParserAdaptersCannotMakeIntermediateStepsClickable`
checks their failure recovery. A complete result discards the preview; a parser
failure calls `discardPreview`.
The public version describes the displayed session and restores the complete
version when a preview is discarded.

- `documentedValidationRemaindersStayExplicit` pins four concrete remaining
  engine/IDNA differences separately from the named compatibility cases.
- `tabsCannotChangeTheCompleteFormsValidationWhileAPreviewIsDrawn` ensures a
  preview's tab click cannot change which complete form fields are validated
  after recovery.
- `aStreamPushNeverBecomesClickableWhenTheAppClearsItsFlag` refuses clicks,
  entry and marker writes from a preview, then accepts the finished result.
- `discardingPreviewsRestoresCompleteFieldsPristineStateAndPendingMutations`
  clicks before/during/after, checks exact state/version/pristine/form restoration,
  refuses the retained preview and resumes an existing mutation afterward.
- `discardingAPreviewPreservesStateHeldCustomActionAuthority` stores and sends
  an actual custom action before and after discarding a preview.
- `appEditsDuringPreviewSurviveCompletionEvenWhenEqualToTheOldDefault` checks
  explicit app edits across subsequent chunks and completion.
- `aPreviewShowThatThrowsRestoresTheCompleteProgram` throws from initialization
  after setting the preview program, preserves complete state/version, and clicks
  the original element afterward.
- `refusedPersistedValuesLeaveThePreviousProgramClickableAndStateUnchanged`
  preflights non-object and oversized persisted values without replacing the
  complete runtime program. The old button continues to work after each refusal.

Viewer `ViewerIssue71` uses an actor-controlled final parse suspension, not a
wall-clock delay or nested run loop. `aFailedFinalParseRestoresTheCompleteProgramAndRefusesRetainedPreviews`
clicks before, during and after failure; `aFailedReloadRestoresThePreviousSourceRuntimeAndClickableButton`
checks source, position, runtime identity and store restoration across reloads.
`aRejectedReplacementRestoresAnotherPlaybacksCompleteRuntime` exercises the public
recovery method used when the viewer replaces the entire playback/library; the
viewer restores the old program/inspector context and keeps a failed live
request's status/draft available for recording. `automaticPlaybackStopsAfterAFailedFinalStep` awaits the parser's suspension and
an observation of playback stopping. `aSourceFailureRemainsReportedAfterSuccessfulFinalEvaluation`
preserves main's source-error reporting. The bench gallery now includes its five
additional field types; Chips/OptionCards are exercised with the react-ui library.

Issue #75 completes this transaction (below): the shared session core stages
program/store/evaluation and checks native decoding admission before commit.
The historical #71 tests above remain in force.

The new tests run headlessly using the existing async helpers.

## Swift issue #71, Round 2 allocation and request ownership

`BoundedNativeControls` is a separate suite because removing a bound can abort
inside SwiftUI before an assertion; the discovery/completion gate must report
an incomplete run. `enormousSliderGridsDrawWithoutChangingStateAndStillWriteArrays`
hosts both variants with range 1e15, step 1e-9 and fractional grids, asserts no
drawing write, then clicks and checks the array/grid shape.
`aThousandSliderValuesStayReadOnlyUntilReplacedWithABoundedValue` checks both
large defaults and large stored arrays (including malformed entries) without truncation, then repairs the
value through the app and clicks the resulting bounded control.
`sixteenSliderValuesDrawAndAnEditKeepsAllThumbs` checks the interactive boundary;
`continuousSlidersWithoutAProgramStepKeepFractionalPicks` checks absent-step input.
`longOptionListsDrawLazilyAndFirstChoiceWritesItsShape` supplies 100,000 and 100,001
distinct keys through the public native-view hook (independent of Rust parser
byte limits), hosts all four option families, checks unchanged drawing state,
and clicks the first option. Checkbox output contains one boolean per supplied key.
`invalidSliderArithmeticDrawsFallbackWithoutChangingStoredValues` hosts reversed
bounds, unsafe tiny spans, NaN/infinite/negative/zero steps and nonfinite defaults, then checks interaction after repair.
`extremeTextRowsKeepTheirValueAndCanBeEdited` covers huge/negative/nonfinite rows.
`LargeDataDrawing` checks that lists above the former Round 2 thresholds remain
native and reachable (details below). `LongViewerDrawing` hosts 1,001/5,000
fallback props/lists/records and scrolls to their final entry. The playback scrubber also has no SwiftUI step;
its existing setter rounds to integer chunk positions.

The range-driven bounds are at most 1,000 discrete SwiftUI positions and
16 drawn slider thumbs. Fixed scalar controls and date ranges create one/two
controls. TextArea rows affect only a 1...20-row viewport, never a loop.
List-driven controls are bounded by the runtime’s limits on the data, not by
a view-level cap. All supplied children, fields, buttons, tabs, options, rows,
columns, cells and chart points remain admitted. Option lists use lazy viewports
(240 points vertically or a 44-point horizontal row). Long viewer fallback
collections are lazy; ordinary boxes retain their existing layout.

`aRefusedEntryPreservesEveryObservableAndItsClickGate` snapshots every readable
public LiveProgram observable, field/value/pristine readers and retained-root
authority before refusing show (preview and complete), set, setField, write,
enter, run and resume. It checks callback and custom-validation registrations
through a mounted click, both with/without an app streaming request, and exposes
any hidden store change on the subsequent redraw.
`admittedAppWritesAreNotRefusedByAnOversizedPreview` fills a preview with 7 MiB,
then writes 2 MiB to the complete store through both state/form routes. The call
succeeds, reports a preview projection warning, and the admitted value and
clickable complete program survive discarding the preview. A disposable mirror
must not throw after its authoritative write already committed.
`refusedShowsPreserveAnExistingPreviewAndTheAppsOwnRequest` starts from a preview,
checks both refusals, then discards it and checks the app's request survives.
`temporarySuspensionsReleaseOnlyTheirOwnGate` checks independent release scopes.
Viewer `finalParsingRestoresOnlyTheViewersGate` covers success/failure with both
app requests; `changingTheAppsRequestWhileFinalParsingIsSuspendedSurvivesItsFailure`
changes that request during an actor-controlled suspension. No nested run loop
or machine-speed assertion is used.

`pickingADayStoresLocalMidnightInANonUTCTimeZone` edits a real NSDatePicker with a
hidden afternoon time under Asia/Kolkata, then compares its stored ISO UTC value
with the saved upstream local-midnight observation. Earlier UTC date/range tests
now explicitly set a UTC environment rather than assuming the machine's zone.
The validation fixture adds upstream answers for `http://a.+4` and
`http://xn--a.com`; the existing public Send observation test consumes both.

Issue #75 implements the deferred complete-show Rust transaction (below).
Accepted actions whose redraw fails retain main's existing event-delivery/error
semantics (`stillDeliversTheActionsHostEvents`); they are not pre-admission
refusals. Round 2's report describes the observable failed-replacement boundary.


## Swift issue #71, Round 3 list compatibility

`LargeDataDrawing.everyChildOfALargeContainerRemainsNativeAndReachable` hosts
Stack and Card with 1,001 and 5,000 distinct children, scrolls to every unrealized
child, and requires every path to be realized and the last child to be visible.
`tablesKeepEveryCellNativeAndTheirLastRowReachable` checks every cell path in
200x6 and 2,000x3 tables and scrolls to the last cell. Both check unchanged state
and version. There is no assertion about machine speed or a nested run loop.
`allTwelveHundredChartPointsReachTheChartDomain` asks the public Charts proxy
for every supplied label's position; `aFifteenHundredOptionSelectStillDrawsNatively`
hosts the large picker without changing state. The real menu test
`everyOptionOfALongSelectIsInItsMenuAndTheLastCanBePicked` requires all 1,500
choices in order and checks the last choice writes its binding. Like the other
Select menu tests, it requires an available macOS display.

`LongViewerDrawing.everyFallbackCollectionKeepsItsLastEntryReachable` checks
both 1,001 and 5,000 props, list elements and record fields by scrolling to the
last entry. The four lazy field-option families are also hosted with 100,000
and 100,001 distinct choices; a click writes the full upstream value shape.
The 256-entry switch in fallback stacks chooses an eager or lazy layout, never
refuses, omits or truncates entries. No list-derived allocation limits remain,
except the explicitly retained 16-thumb read-only slider presentation.

Run the offline main/current text and native-acceptance differential with
`python3 tools/viewer/Checks/compare-native-rendering.py`. It enumerates every
program in the viewer's five corpus manifests and the timing samples, plus both
galleries, in parsed and evaluated form. Its method, acceptance exceptions and
saved outputs are documented in [Checks/README.md](tools/viewer/Checks/README.md).
`RenderingCompatibility.everyCorpusAndBenchmarkProgramPreservesMainDrawing`
also runs it in the viewer suite (about 45 seconds here, without a speed
assertion). Process completion uses an async termination handler, no nested
run loop. The suite needs the local `main` ref and the already-built binding.
The standalone command accepts `--baseline` with a local ref or commit.
Neither downloads anything.


## Opt-in evaluation-memory allowance (#55, #74)

`crates/openui-runtime/tests/peak_memory.rs` declares a 512 MiB `test_alloc`
allocator. Its public-interface tests are:

| Test | Enforced behavior |
| --- | --- |
| `memory_cut_recovery_never_exports_refused_syntax` | Array, object, nested-element and Each-owned props: both ends of every reservation-equivalent allowance interval produce flagged cuts bounded by complete JSON plus 32 bytes; explicit 1257 setting and exact completion edges |
| `each_reserves_its_result_vector_before_building_items` | Edge derived from a one-item control and public storage reservations; just-under-edge cut heap below completed same-process heap / 32 + 16 KiB; fails when the check moves after construction |
| `repeated_reference_profile_covers_measured_heap` | 1,000-row Each profiling; representative real heap stays within the conservative account plus a 10% allocator/layout margin |
| `memory_settings_do_not_invalidate_setup_only_receipts` | Memory-only settings can change without invalidating core setup receipts; setup spends no memory allowance |
| `memory_is_opt_in_and_has_an_exact_portable_edge` | Default unset; exact and one-byte-adjacent accounted thresholds; sticky memory cuts; monotonic sweeps over scalar, string, array, object, Each, sort and filter expressions |
| `tree_rebuilds_share_the_account_and_keep_completed_bytes` | Shared tree/rebuild reservations, exact completed bytes at/above the observed peak, flagged cuts below it, monotonic sweeps |
| `repeated_references_cut_before_the_quadratic_result_including_unread_defaults` | 2,652-row read/unread Each witnesses cut under 2 MiB of accounted memory, with additional measured heap below one third of the same-process unset control (32 KiB tolerance) |
| `parse_time_defaults_cut_and_streams_reaccount_each_prefix` | Parse-time evaluated defaults cut; opt-in streaming equals fresh parsing at every tested prefix; a large allowance keeps default bytes |
| `action_resumes_keep_one_memory_allowance_and_cut_writes_change_no_state` | Adjacent whole-run thresholds, no refresh after mutation suspension, monotonic resumed runs, and unchanged Store snapshot/version/pristine status on a cut write |
| `shaped_element_props_are_admitted_before_the_fields_helper_copies_them` | Array-, element- and AST-valued shaped props preflight immediate owned copies before helper conversion; same-process/payload-relative peak comparisons |
| `static_element_payloads_are_admitted_before_the_root_clone` | A static element’s large directly owned string is refused before cloning the root; same-process peak comparison with 16 KiB tolerance |
| `a_one_byte_short_allowance_refuses_the_large_public_copy` | A large public string copy is refused before allocation at exactly one byte below the accounted peak; same-process measured heap comparison with 16 KiB tolerance |
| `descriptor_substitution_and_reentrant_scopes_share_memory_cuts` | Descriptor evaluation/substitution and evaluator callback reentry keep the same account and flags |
| `representative_shapes_account_for_measured_heap` | Representative empty-array, nested-array, object and string leaves; diagnostic heap/account ratios and `heap <= account + account/10` for these finite shapes |

The separate session/native/wasm `peak_memory` test binaries declare capped allocators
and pin `reference_preparation_refuses_memory_configuration_before_session_creation`: zero, one and maximum reference-preparation memory settings refuse with `LimitExceeded`, leaving existing state unchanged. Native/wasm also exercise the optional session configuration with unchanged completed JSON,
cut flags and unchanged state. Native also checks parse-time defaults; wasm
checks the generated-facing optional accessor. Existing pinned allocation/work/
room tests are unchanged. No measured bytes or elapsed times are exact pins.

On the development machine the 2,652-row witnesses measured about 199 MB unset
and 230 KiB with the 2 MiB accounted allowance, for both read and unread defaults.
The 1,000-row repeated-reference profile measured 900,663,496 accounted bytes
and 48,787,296 measured heap bytes (18.46 times); the Each vector reservation
is only 128,064 accounted bytes, under 0.02% of the total, so its weight remains
unchanged. These are observations, not byte pins. The four representative leaf shapes measured heap/account ratios approximately
0.403, 0.223, 0.606 and 0.333. These observations include allocator instrumentation,
are not Linux baselines, and are not universal multipliers. Reservations are
retained until the allowance ends, so account/heap can grow without a finite
upper ratio on long sequential runs. The uncounted paths in ADR 0011 also rule
out a universal heap/account multiplier.

The implementation review compared against main `92c9b23cf6ebd4215ded34faa8df617782f56f1f`:
972 corpus cases through both CLIs (`parse` and `stream`, chunk 97), and a single
release binary linking an archived/renamed main core/runtime and the current
core/runtime. That binary compared full bytes/flags and deterministic parse/tree/
stream allocation-call counts over all cases and 17,359 chunk-31 pushes, plus
10 expression work/room thresholds. The controls asserted equality; balanced
same-binary timing batches were diagnostic only. Recreate the baseline using
`git archive main` into a scratch directory, never a checkout that can disturb
another agent's files. The historical report under `target/peak-limit-report.txt` records
the exact commands, logs and final counts. Goldens are regenerated twice into
fresh `target/` destinations and compared without changing `conformance/`.

Scratch mutations are kept outside the worktree sources: drop the memory check;
delay it until after Each has constructed its output; reset the retained allowance
on mutation resume. The first is caught by the exact-edge test, the second by the
new `each_reserves_its_result_vector_before_building_items` heap comparison (#74), and the third by the action-resume test.
The report records actual mutation runs. Allocation and work assertions never
use a wall-clock threshold, so none of these checks depend on machine speed.

For #74, `target/peak-74-report.txt` records the fresh archived-main CLI and
same-binary controls, debug/release workspace counts, and scratch mutation
runs. Restoring main's element recovery fails the recovery sweep; moving Each
admission after construction fails its heap assertion; removing the constructor
refusal fails each binding's reference-preparation configuration test. The
state-defaults terminal memory clear was dead: subsequent state-defaults runs
initialize the account; expression sessions initialize and swap their own
account before use. Removing the terminal clear preserves the public suites
and the same-binary default parse/allocation/benchmark controls. No test asserts
timing or a platform's exact measured heap size.

The #74 weight experiments are recorded in `weights-64-suite.log` (13 passed,
1 failed: the existing portable overflow-edge test) and `weights-32-shapes.log`
(object heap 9,016 > account 7,496, rejected by the representative-shape guard).
At 64, the 1,000-item account fell to about 452.4 MB and the object account to
9,960 bytes; production retains 128. These figures are diagnostic observations,
not fixed byte assertions, and the scratch changes never entered production.

The built wasm package also has `reference preparation refuses evaluation
memory configuration without effects` in `crates/openui-wasm/js/session.test.ts`:
exported bigint settings 0/1/max refuse with `LimitExceeded`, existing snapshot/
version stay unchanged, and removing the setting permits construction.
`scripts/check.sh` runs it (`scripts/wasm.sh` followed by
`bun --no-install test crates/openui-wasm/js`) where the wasm tools are installed.

## Swift issue #75: atomic replacement and local controls

`openui-session/tests/replacement.rs` exercises program/JSON admission,
post-initialization evaluation refusal, native depth admission, old action
handles, prompt/explicit bindings and callback registrations. It compares one
replacement with the existing separate calls across work limits, including
per-entry initialization refusals and cuts. Suspended runs are compared with an
untouched control after refusal, including events, flags, failures and state.
`runtime_replacement` tests in ffi/wasm exercise their public adapters; native
also checks callback thread identity and subsequent app state/form writes.

Swift `refusedShowsKeepBothSessionsAndLaterAppWritesOnTheOldProgram` fails
initialization and post-evaluation display admission in complete and existing
preview sessions; subsequent set/setField must still draw OLD. Existing
`aRefusedEntryPreservesEveryObservableAndItsClickGate` and
`refusedShowsPreserveAnExistingPreviewAndTheAppsOwnRequest` cover observable
snapshots and app request ownership. `unsetDatesDrawAnEmptySystemFieldAndWriteThroughTheBinding`
checks the empty native date field and real editing for bound/unbound values.
`tabSwitchesDuringStreamingAreLocalAndNeverWriteOrRun` switches actual segmented
controls during app streaming and previews, checks unchanged store/version and
no host event, and checks complete validation after preview recovery. The older
`tabsCannotChangeTheCompleteFormsValidationWhileAPreviewIsDrawn` remains in force.
`invalidSliderArithmeticDrawsFallbackWithoutChangingStoredValues` now includes
Double.leastNonzeroMagnitude for both slider variants. Existing step-rounding,
local-midnight and native rendering differentials still apply.

Regex validation has no execution-time bound. No worker, timer or length cap
was added; a pathological ICU pattern/subject can block main-actor validation.
Existing upstream-observation tests pin compatibility, without time assertions.
The persisted-value throwaway session is gone: replacement admits it in the
real session under its actual prompt bindings and limits.

`scripts/check.sh` runs `scripts/wasm.sh`, then
`bun --no-install test crates/openui-wasm/js`, where the wasm tools are
installed, and CI always does. Native wasm adapter tests and the source API
checker run without them; generated JS guard/declaration behavior needs the
WebAssembly step. Android tooling is not needed for these checks.

The Store initializer's compile-fail rustdoc example prevents moving the staged
Store out of its shared borrow; no independently owned store alias can escape
with the same session authority. `a_refused_candidate_cannot_mint_custom_authority_in_the_live_store`
checks refused versus committed custom marks. `a_refused_replacement_does_not_drain_abandoned_continuations`
checks release bookkeeping, and `persisted_admission_keeps_swift_shows_error_precedence`
pins malformed persisted data before program-size refusal, matching Swift's
former pre-check. `persistedValuesUseTheRealPromptAnnotatedSessionsAdmission`
checks persisted state and a retained prompt-derived marker through refusal.


## Runtime missed mutants, first batch (#40), and review follow-ups (#76/#77)

`missed_admission.rs` pins refusal text, exact name/value admission edges,
allocation-free scalar replacement and typed Store comparisons. Its constructor
child tests a zero-size Store allowance before retaining any state. The literal
`with_limits -> Default` mutation recurses through Default; the optimized test
profile can turn it into an endless loop. The Unix child has a POSIX CPU-resource
ceiling and disabled core dumps, and the parent asserts successful completion.
The existing library lookup test performs this probe before its first Store
construction too: library tests run before integration binaries, so the full
mutation suite fails at that first entry rather than hanging before the new
integration test. There is no elapsed-time assertion. This catches both recursive construction and
an implementation that returns a Store with default limits instead of the given
limits, without hanging the suite on Linux or macOS.

`missed_tree.rs` tests descriptor binding authority, undeclared prepared supplies,
truthiness, array-like Action arguments, independent deferred positional/mapped
syntax, diagnostic context and export admission. Refused diagnostic export must
avoid copying a large message: its measured heap is compared with a completed
same-process control using a relative bound and 16 KiB tolerance. Array mapped
props use a same-process object-prop allocation-call control, with eight calls of
tolerance; no measured byte count, stack size or timing is pinned exactly.

The `support/missed-tree.tsv` observations come from unchanged main
`03c4ebbb8d44898409d0457ac091dc5609faabcd`. Named behavior tests check both ends
of each recorded constant-output interval under work, room, depth and opt-in
memory limits. Each row fingerprints complete tree/diagnostic JSON with FNV-1a64
and records both cut flags. Directed tests separately assert readable values,
ordered steps, text, authority and identities. Adding cases/rows extends this
batch without changing the parser or materialisation. Scratch mutation patches
and observed failures are recorded in `target/mut-40-proofs/`; the per-mutant
ledger is `target/mut-40-table.txt`. This batch is not a claim about the later
survivor batch or the unrelated full run.

The two finite-shape heap/account guards in `peak_memory.rs` allow 10% above the
portable account for allocator and layout drift (#76). This is a regression
margin for those shapes, not a universal heap multiplier. Reference-preparation
memory settings now produce the typed `LimitExceeded` refusal; README and
CHANGELOG record that older bindings silently ignored them.

The session, ffi and wasm replacement test binaries each include
`post_evaluation_refusal_preserves_pending_mutation_and_all_store_marks`. A
native-view refusal after staging/evaluation is compared with an untouched
control while a Mutation is pending: snapshot, version, every tested pristine
mark, continuation count, form field, next evaluation, resume result and a later
set remain equal. Existing refusal/equivalence tests also compare multiple marks
and continuation counts. The existing suspended-work test also refuses after
evaluation at the 2,000/4,000/100,000 work settings and compares the control
resumes, retaining their cut/completion edges. Staging remains outside evaluation memory as specified
in ADR 0011 and clarified in ADR 0012; no new limit is introduced.

Swift `Issue75.endingAPreviewRestoresTheCompleteTabsSelectionAndValidation`
mounts the real segmented control, selects the invalid preview tab, and ends
that preview both by discarding and by showing the complete program with
identical props. It requires the original complete selection to reappear,
unchanged state/version and no events, then checks validation follows complete
tab switches. Both parameter cases run on the headless worker.


## Runtime missed mutants, final batch (#40)

The full runtime run at `c3b6b8b` contained **1,106 mutants**. Its two survivor
batches are now accounted for individually:

| Survivor batch | Covered by tests | Reasoned exclusions | Removed code | Bugs |
| --- | ---: | ---: | ---: | ---: |
| First batch | 57 | 14 | 8 | 0 |
| Final batch | 83 | 9 | 6 | 0 |
| Total | 140 | 23 | 14 | 0 |

These are mutant counts, not numbers of test functions. Every one of the 177
listed survivors has a regression, a reasoned exclusion, or a removed redundant
path. This is **not a new full mutation run**. The final batch used individually
applied scratch mutations and focused integration binaries; every TEST entry
was observed failing, then its source was restored. The 98-entry ledger is
`target/mut-40b-table.txt`; patches, failure logs and exclusion-set comparisons
are in `target/mut-40b/`. The full crate mutation command was not run.

`missed_runtime.rs` adds 27 fast public-interface tests. The grouped matrices
pin complete output/error observations or complete-JSON fingerprints, refusal
variants, flags and work/room/depth edges. They cover supplied typed syntax,
ignored marker metadata, escaped text, malformed and coerced action arguments,
query registration, prepared references, function return truthiness, argument
cache identity, binding annotation ambiguity and retained-name refunds.
Combined work/depth limits distinguish guards that otherwise appear equivalent:
a premature size refusal cannot replace an observable depth refusal. Formatting
checks pin every ActionResumeError message and opaque continuation Debug text.
The existing array-like Action test also pins both emitted event payloads.

The mapped-argument allocation test compares the borrowed path with an otherwise
identical same-process path requiring a temporary name map, at widths 16 and 64.
Both paths are warmed before measurement to exclude one-time evaluator setup.
It compares allocation calls, with width-relative slack, not measured byte counts
or elapsed time. No new test has a timing assertion. No parser or materialisation
code changed, no new limit was introduced, and public results, errors, flags,
reported work/room and allocations remain unchanged against main
`c37b46c1dc9e4839dff25b01a0c88443245fa149`.

The production deletion is confined to redundant binding hooks and the duplicate
marker predicate. The wrapper's output hooks dispatch directly on TreeScope, so
its ordinary call hooks were unreachable. Each runs outside prop call context
for query defaults and binding transforms; its no-substitution defaults suffice.
The removed each_item forwarding hook itself called the trait's default None.
The marker's remaining object/string predicates already inspect caller-owned
records without projection, allocation or charged work. Four scratch public-path
allocation probes matched before/after, alongside unchanged observation fixtures.

Nine exclusions retain protective fast paths whose mutations change only unpriced
bookkeeping or checks dominated by later admission. Their reasons are beside
the patterns in `.cargo/mutants.toml`. Per-file `cargo mutants --list --json`
set comparisons removed exactly four action mutants, one form mutant and four
text-sizing mutants. Every pattern is anchored to its column. The advancing
string-cursor comparison is covered by the one-character argument work test,
not an exclusion: skipping an indexed character changes charged argument work.
Failures, cut-flag precedence and Display text are not excluded.

The session, native and wasm `post_evaluation_refusal_*` tests now require
`OpenUiError::Evaluation` with `native view root is not an element`, in addition
to the existing pending-continuation and state comparisons. A pre-evaluation
JSON or limit refusal no longer satisfies these tests.

### Deliberate regeneration of missed-mutant observations

The final-batch fixtures were recorded from unchanged main before any production
deletion. To approve an intentional cost change, inspect its effects and regenerate:

```sh
CARGO_NET_OFFLINE=true UPDATE_MISSED_RUNTIME=1 cargo test -p openui-runtime --test missed_runtime
CARGO_NET_OFFLINE=true cargo test -p openui-runtime --test missed_tree regenerate_missed_tree_observations -- --ignored
```

The first command writes `support/missed-*.txt` from the complete small probe
grids. The ignored writer updates `support/missed-tree.tsv` at its existing
endpoint coordinates. Equal endpoint observations remain paired; endpoints that
now differ become separate singleton rows. It preserves all existing probes and
does not claim to discover new transition coordinates. Run either writer twice
and require identical output, review the fixture diff, then run both binaries
without regeneration enabled. Neither writer is part of the normal mutation
suite; neither invokes Bun or updates conformance goldens.

## Scripted tool outcomes in the Viewer

`ScriptedToolOutcomes` in `tools/viewer/Tests/ViewerKitTests/ToolScriptTests.swift` runs with the same controllable clock for the Swift driver and the tool provider. Its tests check script validation, overlapping calls, the repeated final outcome, ordered missing-tool hints, first registration after a preview, and fresh outcomes on reload. A failed mutation leaves its action tail unrun. A mutation that finishes during a reload keeps its host event with the original runtime, including after rollback. The bundled News Desk files run through the Viewer's normal open and playback path. The tests inspect the query snapshot, store rows and evaluated buttons, run Retry, advance the refresh interval, and use SwiftUI's image renderer to observe the resulting native text drawing without opening a window.

## Chat actions and multi-turn history (#151)

ChatKit's action tests drive the binding's real `LiveProgram.run` and
`LiveProgram.enter` operations on responses streamed through the recorded
HTTP transport. The offline exchange covers a button and an earlier
answer's form. It compares all the message roles and the UTF-8 bytes of all
user and assistant turns on the third request. The expected action turns
come from the pinned react-ui helper and the vendored lang-core writer.
The browser test observes the address at the app's opener seam. It covers
both web schemes and rejects relative, scheme-relative, hostless and non-web
destinations. Bubble tests check field filtering and open the rendered
disclosure through its AppKit accessibility action to check the label,
title, and visibility of field labels and values.

HTTP, mid-stream, size and drawing failures preserve the pre-action turn
IDs and raw history. Retrying the same action sends the same HTTP body.
A parked transport releases its body at `[DONE]` and another action can
send. Explicit Stop keeps the partial reply. The lone-surrogate test
pins the replacement character's bytes on the wire. These tests use
recorded or synthetic transports and an in-memory key store.

## Raw responses in Swift

`RawResponses` compares a bare response with stream set and finish, reads a regenerated message, and follows a Cloud script through a partial section, settlement and an edit. It checks the drawn text, provider calls, interaction and Retry gates, and the parser diagnostic after a store redraw. A size refusal and a discarded in-flight read preserve the accepted drawing and metadata. A provider-free session accepts clicks again after clearing the app’s streaming request or discarding a bundle preview. `ResponsePlayback` opens both bundled samples through the Viewer model and checks their raw source, drawn editions and cached final steps.

For #124, the session and WebAssembly tests replay `streaming-over-a-registered-program`
and `replacing-a-program-while-queries-are-paused` from `session-blocked.json`.
Each step compares the drawn tree, snapshot, flags, tool calls and host events
with the pinned renderer. In OpenUINative,
`blockedReplacementDisablesTheMutationButtonAndInputUntilSettlement`
checks the built-in mutation button and input as B streams over A and while
B has an invalid scripts section. Both controls work after B settles.

## Swift component library definitions (#138)

`ComponentDefinitionTests` compares every Swift prop kind with its plain
data and the core's signatures, exercises the default root through parsing
and streaming, and checks refusal kinds and messages. SwiftUI's offscreen
`ImageRenderer` exercises the documented example, evaluated props on a
store redraw, and the fallback for a viewless declaration shadowing a
built-in name. The defined input test drives its SwiftUI binding setter.

## Swift function and action declarations (#139)

`swift test --filter FunctionDefinitionTests` in `swift/OpenUINative` runs
without a display. The suite checks the generated signatures, evaluated
values and diagnostics, exact collision refusals, and custom action events.
It replaces a registered callback to detect re-registration across shows,
exercises separate preview and complete sessions, and checks both sides of
the 256-implementation limit.

## The Intelligent UI library in Swift (#158)

`OpenUIIntelligentUITests` (in `swift/OpenUINative`, run by the same `swift
test`) holds the library to upstream's recording in
`conformance/real-libraries/intelligent-ui.json`. `WhatTheModelSeesTests`
first ties the recording to its bytes (the core's JSON Schema and prompt spec
for upstream's definition, and its prompt with upstream's prompt options,
equal the recorded text), then compares the Swift library's JSON Schema,
prompt spec and prompt with upstream's, byte for byte, after putting
upstream's text in place of each entry of `IntelligentUIAdditions.changes`
(each found exactly where it belongs) and leaving upstream's functions out
until #160 declares them; any other difference fails. It
also checks the prompt options field by field, the 52 components in
upstream's order with a view each, and the documented example.
`PlaceholderTests` parses one program using all 52 components (and both added
chart types) with no error, shows it in a live program and draws it with
SwiftUI’s offscreen `ImageRenderer`. Every component draws its children
through its library view and nothing reaches the fallback. In Rust,
`intelligent_ui_matches_upstream_byte_for_byte` also checks the core's prompt
for the definition and upstream's prompt options against the recorded prompt.

### Basic layout drawing (#167)

`LayoutDrawingTests` draws a `LiveProgram` through `ElementView`. Its
`LayoutPicture` helper uses `ElementDrawing.chrome` to observe child frames
by their evaluated paths without affecting measurement. It samples rendered
colours against a swatch rendered on the same host. A test can supply a leaf
through `ElementDrawing.component` to measure the layout around an Image or
text component independently of that component’s implementation.

The mixed program exercises all six native views together. Further cases
cover flow spacing, alignment, wrapping, flex, lengths, media rows, Card
typography, padding, backgrounds and clipping. One mounts a resizable scroll
view in `NSHostingView` to exercise viewport units and the Card breakpoint.
The URL-background test registers a URL protocol for its sentinel host and
checks that a file URL backed by a preloaded pipe retains all its bytes. All of these
tests run without a display.

Use `clipContainer(to:)` on a drawing container after its inner frame has
been sized. It clips both paint and pointer hits to the same shape. Apply it
before drawing the border. For a scrollable container, clip at the viewport
and place paging controls against the outer frame. `ContainerClippingTests`
sends native clicks just outside each boundary and checks the control inside
still responds.

### Grid, Carousel and Popover drawing (#168)

`GridDrawingTests` measures equal tracks, the last partial row, responsive
breakpoints, row heights and typography through the same `LayoutPicture`.
`CollectionDrawingTests` serializes the mounted windows used by Carousel and
Popover. The arrow tests click at the ends of the track and read both the
item frames and the buttons AppKit exposes to accessibility. The swipe test
delivers a horizontal sequence of pixel-scroll events and checks the next
item's leading edge. The Popover tests inspect the rewritten trigger, the
panel's measured content and the presented window as it opens and dismisses.
The headless regressions cover Grid's intrinsic column count and stretched
rows, six arrow clicks at a fractional Carousel stride, the centered strip,
the Popover hit area and the gaps between inline Markdown items. A single
Carousel item contains a responsive Grid whose tiles wrap at the track's
85% cap. Carousel extent tests read the native scroll document, including
overflow nested inside a Col and Grid, and check that a nested scrollport
or an explicitly clipped Box contains its own overflow. Popover panel measurements cover relative and HTTPS links, emphasis
inside a link label, ordinary source whitespace and nonbreaking spaces.

### Text drawing (#169)

`TextDrawingTests` renders programs through the same layout probe. It checks
type sizes at the 640-point breakpoint, line clamping, badge colours in both
modes, inline component wrapping, marker lanes, nested element paths and the
timeline connector. The Card opening-title test uses the real text components.

`TextInteractionTests.swift` mounts programs in a window. A double-click exposes
the selected word through AppKit accessibility. The accessible attributed text
supplies font sizes, variable weights, decorations and colours. Link clicks
record the URL passed to SwiftUI’s browser action. Overflow tests click the
visible part of a wide child and then its clipped part in the same window.
The Quote probe gives a linked glyph its intrinsic width inside an eight-point
text region. A linked ListItem title overflows into an empty Grid cell, and an
uppercased inline link overflows its assigned run inside a wider paragraph.
The Badge probe double-clicks its selectable label on either side of the
capsule’s curved edge. Other cases check which Hebrew word a double-click selects beside
a Badge, URLs delivered for escaped Unicode paths, rich ListItem inheritance,
inline child wrapping, and links below a mixed paragraph’s visual-line limit.
All text drawing and interaction tests run headlessly.

### Markdown drawing (#170)

`MarkdownReplayTests` compares the block sequence and inline token nesting,
text and accepted link targets against
`conformance/golden/intelligent-ui-markdown.json`. The same recording covers
the inline-parsed slots of the #169 text components, with a separate span for
each string in mixed text. To regenerate from the byte-for-byte upstream copy,
run `bun --no-install intelligent-ui-markdown.ts` from `reference/`.

`MarkdownDrawingTests` checks the flat block sequence and mounts real
`LiveProgram` views. The mixed document exercises every block, measured
type and inline formatting. The renderer retains malformed syntax and
HTML as text. Deep quote and list inputs stay flat and later blocks
still draw. The rule is sampled inside its measured frame.

AppKit events check that absolute web links open, and that overflowing
links and selectable code only respond inside the document, quote,
list and scroll boundaries. The mounted table tests check shared
leading-aligned tracks, body wrapping and access to links after
horizontal scrolling. These tests run without a display.

### Icon drawing (#172)

`IconDrawingTests` renders the full 1,912-name Lucide catalog through
`LiveProgram` and `ElementView`. The pinned name-list checksum covers
aliases. Each name must paint its own glyph, and the name-matching table
checks normalization, exact-name priority, word order, prefixes, fallback
and blank names, including long suffixes and hyphens followed by combining
marks. Measured frames and paint samples check the four sizes,
stroke width, fixed row slots and explicit or inherited colours.
`theLucideArtworkLicenseShipsInTheSwiftPackage` checks the notice bundled
alongside the artwork against the pinned release.

`clickingAnIconOnlyTriggerOpensItsOwnPanel` mounts two Popovers and clicks
the measured centre of the second trigger's glyph. All #172 tests run
headlessly, including that window test.

### Markdown table layout (#252)

`TableGeometryTests` exercises shared column constraints and the visible
slice of a variable-height row index. `MarkdownDrawingTests` mounts the
table, scrolls through its outer viewport, and checks that row links enter
and leave the view tree. It also checks that long body words can wrap
before horizontal overflow and that visible table text can be selected.
`MarkdownTableParityTests` mounts the original SwiftUI Grid alongside the
table and compares tracks, wrapped text, row offsets, host text sizing,
and selection through a scroll round trip.
`markdownTableSelectedTextCanBeCopied` needs a macOS display session for the
AppKit pasteboard. It uses a private pasteboard and joins the display skips
audited by the Swift test gate.

### HtmlView drawing (#183)

`HtmlViewDocumentTests` checks the policy and base styles against the
pinned upstream document and checks that model HTML stays inside the
iframe's escaped `srcdoc` attribute. A windowless drawing test measures
the full width and clamped fixed height in a start-aligned column.

`HtmlViewTests` uses the same live-program and native-window seams as
`CollectionDrawingTests`. The 14 mounted cases are named in the Swift test
gate. They require a display and run without skips in the macOS viewer CI
job.

- `inlineScriptRunsAndRequestsOutsidePolicyAreBlocked` uses a loopback server
  and a local file stub to check that allowed inline code runs and disallowed
  requests, file scripts, relative host access and `eval` fail.
- `navigationAndNewWindowsStayInTheAdmittedPage` exercises scripted and clicked
  navigation, forms, nested frames, popups, and a native main-frame request.
  The loopback server must receive zero requests.
- `onlyFiniteHeightFromThePageReachesSwift` checks the page world for a callable
  handler, exercises rejected height values and message sources, and confirms
  that an admitted finite height is used.
- `storageIsUnavailableInTwoViewsAndAFreshSession` probes cookies, local and
  session storage, IndexedDB and CacheStorage using separate ephemeral stores.
- `httpCacheIsSeparateAcrossViewsAndAppProcesses` checks reuse of an HTTPS
  image within one page and a new request per view, including a fresh process
  launched by Swift Testing. The test delegate trusts only the loopback
  fixture's leaf certificate.
- `allocatingPageLeavesHostResponsiveAndExitsWhenDismissed` checks the page
  process for resident memory growth, clicks a native host control, checks a
  second page, and waits for the dismissed process to exit. A test-only
  watchdog bounds the fixture independently of the main actor.
- `spinningPageKeepsHostResponsiveReleasesViewAndExitsWhenHostExits` checks that
  a page looping forever leaves the host and a second page responsive, that
  dismissal releases its web view, and that its WebContent process exits when
  its host process exits. WebKit has no public means to end a looping page's
  process sooner; the maintainer accepted and documented this (#183). The host
  exit was measured at 0.003 to 0.111 s; the test allows 10 s.
- `terminatedPageStaysStoppedUntilExplicitReload` terminates the test page's
  WebContent process and exercises the host's stopped state and Reload.
- `reportedHeightFollowsContentAndClampsBothEnds` clicks the page's resize
  button and measures the frame as the content grows and shrinks.
- `fixedHeightClampsAndIgnoresPageReports` exercises both clamps, then clears
  fixed height and waits for the page height without a reload.
- `fixedPageThenFixedEmptyPageReturnsToInitialAutoHeight` checks
  150 → 150 → 360 after replacing a 212-point fixed document with an empty
  one and switching it to auto.
- `streamingShowsPlaceholderAtInitialHeightUntilCompletion` checks the
  360-point initial height and placeholder, then finds the completed page.
- `anOverflowingPageControlOnlyReceivesClicksInsideTheViewport` clicks both
  outside and inside the clipped viewport.
- `largeDocumentPreparationKeepsHostControlResponsive` draws a 900 KiB
  model document and verifies that native input and page rendering complete.

### Cartesian chart drawing (#179)

`ChartDataTests` checks ordered row fields, explicit and inferred series, numeric
conversion, both scatter axis fallbacks, missing values, signed stacks, and
monotone-X control points captured from the pinned upstream builders. It counts
scatter candidates for long absent keys and checks peak memory growth while
drawing all four chart kinds with 65,536 rows and 65,536 distinct series.
`ChartDrawingTests` runs live programs through `LayoutPicture`. It measures the
drawing, legend and following element for auto and fixed heights. It checks the
four mark shapes, axes and grid, binds series colours to the recorded values,
checks the six-colour repeat and compares the effect of `format` and `stacked`.

`ChartInteractionTests` mounts AppKit windows. It exercises clipped legend and
plot clicks, checks hidden keys after live series updates, and observes the
SwiftUI transaction reaching the chart layout during an animated update. These
tests and the chart image tests run headlessly. `ChartScaleTests` draws a
100,000-point line and a sparse scatter through a tool result. Set
`OPENUI_CHART_ABSOLUTE_MAX=1` and filter to
`drawsSparseScatterFromAdmittedToolResult` to rerun the 8 MiB input and report
wall time and peak RSS.

### Pie and donut drawing (#180)

`PolarChartDataTests` checks category and first-series selection, descending
order, theme palette order, numeric strings, missing and signed values,
percentage rounding and the stacked legend's shared-category toggles.
`PolarChartGeometryTests` checks clockwise sectors, hit-testing the band and
track, and excluding the centre hole and gaps. `PolarChartDrawingTests` draws
live programs through `LayoutPicture` and samples the marks and legend on both
sides of the 400-point breakpoint. It also covers empty and single-slice
charts and checks that fixed height and stacking leave these charts unchanged.
`PolarChartInteractionTests` uses AppKit events to click beside, below and
inside the legend, including a partially visible row in the side scroll area.
The #180 tests run headlessly.

`PolarChartScaleTests` draws 100,000 pie slices and runs a live donut from a
tool result. Run the scale tests individually for independent peak RSS
measurements. Set `OPENUI_POLAR_CHART_ABSOLUTE_MAX=1` when filtering to
`drawsChartFromAdmittedToolResult` for the 8,388,606-byte pie with
65,536 slices.

### Theme and value rules (#159)

`ThemeTests` holds the light and dark themes to
`conformance/real-libraries/intelligent-ui-theme.json`, which
`reference/intelligent-ui-theme.ts` writes from upstream's own source at the
library pin: every token of `shared/tokens.ts`, resolved in each mode as a
page with no overrides resolves it (the library's `defaultThemes.ts` and
react-components' `defaultTheme.ts` written as CSS variables, every `var()`
replaced by its value or fallback). Each of the 66 tokens must equal the
Swift theme's: colours read with the library's own CSS colour reader
(`color-mix()` included) and compared in sRGB to 1e-9, shadows layer by
layer, spacing and radii in points. The font is the one recorded
difference: upstream's Inter (OFL, not approved) is the system font here,
and the test pins upstream's text so a change shows. `ThemeModeTests`
renders a probe with `ImageRenderer`: without a host mode the theme follows
the colour scheme the system sets, and `intelligentUIMode(.light)` or
`(.dark)` wins over either system setting. `ValueRulesTests` runs upstream's
`fg`, `bg`, `size`, `radius` and `space` results from the same fixture (about
250 inputs: every token, each word and character the CSS guard refuses,
JavaScript's whitespace, each unit and its near misses, object-prototype
names) through `ColorValue`, `Length`, `Radius` and `Spacing.points`, which must
agree on every one. `CSSColorTests` checks the colour reader against
conversions known apart from this code (sRGB red in every space, CSS's
keyword table) and that no text, however deep, fails it.

To reproduce the theme fixture, export the two upstream folders at the
library commit and run from `reference/`:

```sh
git -C "$upstream" archive 7b7a51985814d37201e16ece44d28a996bdfa528 \
  packages/libraries/intelligent-ui packages/react-components/src/components/ThemeProvider |
  tar -x -C "$scratch/checkout"
BUN_RUNTIME_TRANSPILER_CACHE_PATH=0 bun --no-install intelligent-ui-theme.ts "$scratch/checkout" \
  > "$scratch/intelligent-ui-theme.json"
cmp "$scratch/intelligent-ui-theme.json" ../conformance/real-libraries/intelligent-ui-theme.json
```

## React-ui table and pie chart drawing (#149)

`ReactUIDrawingTests` parses both the react-ui and bench shapes against their
imported schemas. SwiftUI's offscreen renderer checks the grid, nested cell
paths, chart dispatch and each documented placeholder label. The pie test
also checks the ordered categories and values supplied to Swift Charts.
`lastPageKeepsItsNestedButtonsAction()` advances a mounted table and clicks
the final row's button through its original path.

The replay reads every react-ui case from `conformance/recorded.json` and
shows its final parse result in a live program. It checks native dispatch
throughout the evaluated tree, including offscreen pages and inactive tabs.
A parent that draws a part directly (such as a table column) accounts for
that part. The replay also renders each qualifying program and checks the
fallback hook. The test prints the count and pins it at 18 of 152.

## Mac chat thread storage (#152)

`ConversationsTests` drives the production HTTP client through recorded
token minting and refresh, list and failed list, create, paged item reads,
rename, delete and append. It checks authorization, scope and wire bodies,
and checks the typed host-input error on an oversized response.

`ChatThreadsTests` restores a recorded conversation in a fresh workspace,
runs an actual restored button through `LiveProgram.run`, and compares the
append body byte for byte with the recorded request and pinned action
golden. The tests exercise refused reads and deletes during generation,
successful and refused deletes with a pending save, and deletion while an
append is already in flight. They check replay and storage after a refused
create, Gateway error, refused append, cleanup failure or ambiguous append.
The oversized stored-answer fixture checks that failed admission preserves
the active thread. A switching in-memory key store verifies that generation
and append use the same validated credential. These tests use offline
exchanges and made-up credentials.
Default-model refusals in HTTP and SSE are also replayed against a reopened
stored thread. The tests check that the failed turn stays out of storage and
the next request uses the saved fallback and the full prompt.
