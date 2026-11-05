# Map: moving the core to lang-core 1.1

Upstream released OpenUI Lang 1.1 on 2026-11-04. This map covers:
- what changed upstream between the pin (`eb20708`, 1.0.1) and `47b9cae` (1.1.0);
- where each change lands in the core;
- what our corpus covers already;
- how the move splits into tickets.

The process is the 1.0 move's ([../lang-core-1.0/map.md](../lang-core-1.0/map.md)): vendor first with everything that differs pending, then one ticket per area until nothing is pending.

## Destination

The core conforms to `@openuidev/lang-core` 1.1.0 (`47b9cae`, 2026-11-04) the way it conforms to 1.0.1 today:
- every golden comes from the new reference;
- the corpus also holds upstream's new tests;
- the core matches all of it;
- CONFORMANCE.md names 1.1.

## Notes

- **Decided 2026-11-06:** the maintainer asked for the move: "OpenUI has updated … test cases also have been written and fixes have been merged, so integrate that as well." The target is the newest lang-core commit on main, `47b9cae` (1.1.0). It includes #1554, a fence fix released with 1.1.
- **Upstream commits since the pin:**
  - #1480, "OpenUI 1.1", the main one;
  - #1554, fence lines inside multi-line strings;
  - #1473 and #1475, telemetry;
  - two version bumps.
- **Size:** about 480 changed lines in lang-core's source, across 12 parser and runtime files, plus two new test files:
  - `toolCompatibility.test.ts`: tools whose names or arguments read other statements;
  - `toolReferences.test.ts`: queries reading queries.
- **Spec:** `spec/language.md` changes in four sections:
  - fence extraction (2.1);
  - built-ins taking lists (3.4);
  - expression state defaults (5);
  - queries reading queries and `query-cycle` (6.1).

  The conformance fixtures are unchanged.
- **Telemetry:** still off for us. Install telemetry returns early on `OPENUI_TELEMETRY_DISABLED` or `DO_NOT_TRACK`, which every reference script sets. Runtime telemetry still needs `OPENUI_RUNTIME_TELEMETRY_ENABLED`.
- **What 1.1.0 changes across our corpus** (`scripts/upstream-check.sh`, 2026-11-06):
  - tokens: 540 of 542 parse cases;
  - statements: 534;
  - one-shot results: 7;
  - streaming steps: 20;
  - preprocessing: 8;
  - merges: 35 of 2,525, all from programs with multi-line statements, redefinitions or fences;
  - prompts: 23 of 61;
  - conversions back to source and error hints: none.

## What changed, per area of the core

| Area | Upstream change | Core module | Our corpus covers it now |
|---|---|---|---|
| Tokens and statements | Identifiers, type names, `$state`s, newlines and EOF carry `start`, their source offset. Statements carry `start` (at the identifier) and `end` (at the newline or end of input closing them). The lexer skips `//` and `#` line comments itself; `#` used to be skipped as a stray character. | `lexer.rs`, `statements.rs` | Yes: every tokens and statements golden changes. |
| Fences | `stripFences` is rewritten to work line by line. A fence line opens a block and the next one closes it. A ```` ``` ```` pair on one line is inline code. Code may follow the tag on the opening line. `openui-js` blocks are skipped. When no block starts a program, a program outside the blocks wins (a stray closing fence, `js` helpers). A fence line inside a `"…"` string is part of it: in a block, strings may open on any line, outside one only on a statement line (#1554). | `preprocess.rs` | Partly: 8 cases. Nothing has `openui-js`, code on the opener line, or a fence inside a multi-line string. |
| Merging | `mergeStatements` splits with the parser's own `split()`, so comments, strings and multi-line statements behave as when parsing. Each statement's text is its source span, trimmed at the end. A statement defined twice in the existing program keeps its first position. | `merge.rs` | Yes: 35 merges, from programs with ternaries, redefinitions and fences. Quirks 6, 7, 8, 15, 16 and 20 need re-checking. |
| Materializing | References are memoized per pass, keyed by mode and name, unless they met an unresolved or cyclic reference on the way. A statement used many times materializes once and reports its errors once. An element reached through an alias (`b = a`) is copied before being tagged with the alias's statement. `hasDynamicProps` short-circuits the dynamic check. | `materialize.rs` | Partly: where shared statements report errors, or an alias is the root. |
| Queries and mutations | Their arguments are materialized like any expression, so references are inlined and library calls are named and checked as elsewhere (this replaces 1.0's `nameRawCallArgs`). `deps` come from the materialized arguments, so state read through an alias counts. A new `waitFor` lists the queries whose results the tool name or arguments read. Queries that read each other, around any loop, get the new `query-cycle` error, reported once per loop on its first query in statement order, with the message listing the loop. | `parser.rs`, `result.rs`, `validation.rs`, `enrich.rs` | Partly: 6 cases with queries over other statements. Nothing has a cycle. |
| State defaults | An expression default (`$total = 1 + 2`, also inside list and object defaults) is evaluated once at parse time by the runtime evaluator. It reads the defaults declared above it and the plain statements it references (inlined by materializing). Queries read as null. Any call that isn't a built-in function makes the whole default null. | `parser.rs` and a new evaluator for built-in functions over data | Partly: 2 one-shot cases (`hand/61-state-declarations`, `test/functions-11` and `-12`). |
| Built-in functions | `@Round`, `@Abs`, `@Floor` and `@Ceil` take a number or a list, applying element by element (nested lists too), and return the same shape. Their prompt signatures change, for example `@Round(number: number \| number[], decimals?: number) → number \| number[]`. | `builtins.rs`, `prompt.rs`, the new evaluator | Yes: 23 prompts. |
| Results | Statement offsets don't reach parse results. Query statements gain `waitFor`. `ValidationErrorCode` gains `query-cycle`. | `result.rs` | Through the above. |

**Not part of the core, as before:**
- the runtime query manager: waiting for queries, retries, `isLoading`, `evaluateStatements`;
- `evaluate-prop`'s reactive bindings in ternary roots;
- the message protocol (`parseMessage`'s new unmarked-text rule);
- telemetry.

The new evaluator covers only what parsing evaluates: state defaults over data, built-in functions and operators. It's not the renderer's runtime.

## Keeping the check green during the move

As in the 1.0 move:
1. Vendoring lists everything that differs in `conformance/pending/`.
2. Each later ticket shrinks those lists, and the last one empties them.
3. Every commit passes `scripts/check.sh`.

A local tag on the last 1.0.1-conformant commit keeps 1.0.1 reachable.

## Decisions

1. **Target:** `47b9cae`, lang-core 1.1.0, the newest on main as of 2026-11-06.
2. **State defaults:** the core gains a parse-time evaluator for state defaults, built-in functions and operators over data, as 1.1's parser has. Where it can't match the reference exactly, the difference becomes a divergence in CONFORMANCE.md. An example would be `@Sort`'s locale collation of strings.
3. **Out of scope, as in 1.0:** the message protocol, the runtime query manager and telemetry.
4. **1.0.1:** not kept. The tag `lang-core-1.0.1` marks the last conformant commit.
5. **Testing tickets:** the long runs on GitHub and the worker Mac (invariants, coverage-guided fuzzing, mutants) are repeated against 1.1 at the finish line, and their findings are sorted then. Findings from the 1.0.1 runs that 1.1 makes moot aren't worked on.

## Tickets

In [issues/](issues/):

1. **Vendor lang-core 1.1.0 and its new tests.** Blocked by: none.
2. **Tokens and statements:** offsets, and comments in the lexer. Blocked by: 1.
3. **Fences:** the line-based extraction. Blocked by: 1.
4. **Merging:** splitting like the parser, spans, first positions, and the merge quirks re-checked. Blocked by: 2.
5. **Materializing:** memoized references, errors once, alias copies. Blocked by: 1.
6. **Queries and mutations:** materialized arguments, `deps` through aliases, `waitFor` and `query-cycle`. Blocked by: 5.
7. **State defaults and built-in functions:**
   - the parse-time evaluator;
   - list arguments for `@Round`, `@Abs`, `@Floor` and `@Ceil`;
   - their prompt signatures.

   Blocked by: 5.
8. **Finish line:**
   - nothing pending, and CONFORMANCE.md names 1.1;
   - QUIRKS.md and the divergences reviewed against 1.1;
   - the fuzzer generates 1.1's new inputs (comments, `openui-js` blocks, expression defaults, queries over queries);
   - the viewer uses 1.1's react-ui spec;
   - the upstream check reports the pin current;
   - the long runs repeated on the worker Mac, and their findings sorted.

   Blocked by: 2–7.
