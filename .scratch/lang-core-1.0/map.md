# Map: moving the core to lang-core 1.0

Testing ticket 14 ([../testing/issues/14-move-the-pin-to-lang-core-1-0.md](../testing/issues/14-move-the-pin-to-lang-core-1-0.md)), first step: what changed upstream between the pin and 1.0.1, where each change lands in the core, what our corpus covers already, and how the move splits into tickets.

## Destination

The core conforms to `@openuidev/lang-core` 1.0.1 (`eb20708`, 2026-10-15) the way it conforms to 0.3.0 today. Every golden comes from the new reference, the corpus also holds upstream's new tests and its 1.0 conformance fixtures, the core matches all of it, and CONFORMANCE.md names 1.0.

## Notes

- **Decided 2026-10-16:** the maintainer approved moving. The target is the newest lang-core commit on main, `eb20708` (1.0.1). No later commit touches lang-core as of 2026-10-16.
- **Sources:** the source diff between the pin (`bba1d25`) and `eb20708`, lang-core's CHANGELOG, and upstream's new written specification (`spec/overview.md`, `spec/language.md`, `spec/prompt.md`). The core conforms to the reference implementation, not to the spec text. Where they disagree, the reference wins, and the difference becomes a quirk.
- **Size:** about 1,300 changed lines in the parser layer, over nine files. The lexer, statement splitting, preprocessing, merging and tokens are unchanged.
- **What 1.0.1 changes across our corpus** (`scripts/upstream-check.sh`):
  - parse cases: 305 of 309, 81 of them one-shot and 225 only while streaming;
  - conversions back to source: 25 of 335;
  - error hints: 77 of 191;
  - system prompts: 27 of 47.
- **Upstream's conformance fixtures** (`spec/fixtures/1.0/`) hold 54 cases over a test library (`library.json`): programs, streamed chunks, edits and validation rules. Their expected results describe an evaluated tree, and the core has no evaluator, so they can't be checked as upstream checks them. Their inputs still become corpus cases, compared with the reference's parse results like every other case.
- **Telemetry:** 1.0.1's runtime telemetry is opt-in (`OPENUI_RUNTIME_TELEMETRY_ENABLED`) and honours the disable flags every reference script sets.

## What changed, per area of the core

| Area | Upstream change | Core module | Our corpus covers it now |
|---|---|---|---|
| Library | A schema's `properties` names its components, and other `$defs` are data. Action `$defs` (`ActionExpression`, `OpenUrl`, `ToAssistant` and custom actions) are never components, and `$ref`s to them are inlined into slot schemas. `functions` and `actions` join a call registry with the built-ins, their params compiled like props. | `library.rs` | Partly. Imported schemas have `properties`. None has `functions` or `actions`. |
| Entry | The entry is `root`, else the first statement calling the library's root component (`no-root` warning), else none (`no-root` error). The root name is no longer a statement id. Results are no longer empty without an entry; there are just no orphans. `root = $t ? A : B` is exposed as `rootExpression`. `no-root` is reported while streaming too. | `parser.rs`, `result.rs` | Mostly: 61 one-shot results gain a `no-root` error, 34 change their root, 13 their orphans, and every early streaming step changes. One case has the fallback warning. Nothing has `rootExpression`. |
| Calls | `@Name(...)` that isn't a built-in gets `fn: true`, and if it isn't in the registry either, it's `unknown-function` and null. Built-ins and steps that aren't lazy gain `mappedProps` by param name. Library functions' and actions' arguments are validated like props: an invalid function call is null, and an invalid action is the empty plan `Action([])`. Query and Mutation arguments name their calls' arguments. A `{steps: [...]}` literal counts as dynamic. | `expr.rs`, `ast.rs`, `materialize.rs`, `builtins.rs` | Partly: 11 cases gain `mappedProps`. Nothing has `fn`, `unknown-function`, library functions or actions. |
| Validation | Data in a slot that takes only components is a `type-mismatch` and pruned (0.3.2). Validation no longer mutates; it returns cleaned copies. The core already copies before changing, so this should match. | `validation.rs` | Partly: 10 cases. |
| Streaming | `push`, `set` and `getResult` take a `complete` flag. While not complete, a `$name` at the very end of the text isn't auto-declared as state. Spec rule: streaming equals parsing at stream end. But the reference still skips a pending redefinition that needed closing, with or without `complete`, so a stream cut inside a redefinition may end unlike its batch parse: a quirk candidate for ticket 7. | `stream.rs`, binding, viewer | No: the goldens never end a stream. |
| Results and errors | `rootExpression` in results. Errors gain `severity: "warning"` (only the fallback `no-root` today), carried through error hints. New codes `no-root` and `unknown-function`, with new messages. | `result.rs`, `enrich.rs` | Through the entry changes. |
| Converting back to source | Every call in the library's registry is written with `@`, not just built-ins. | `serialize.rs` | No library calls yet. |
| Prompt | Built-in function and step lines are generated from their schemas, for example `@Count(array: any[]) → number`; the core copies the generated text. A spec's `functions` and `actions` get listed, and there's a new `builtinFunctions` option. One step is taught bare and several in `Action([...])`, and restricted action slots are recognized. The streaming rules number their steps by what's enabled. | `prompt.rs` | Yes: 27 of 47 prompts change. |

**Unchanged:** the lexer, statement splitting and auto-closing, preprocessing (fences and comments), merging, and the token stage. The `statements` stage golden changes for 2 cases, through the new `fn` flag in expression trees.

**Not part of the core, as today:**
- the runtime evaluator, including `evaluateRoot`, actions as plans, the state store and `Action` flattening;
- `library.extend()`, and the zod-side builders `defineFunction`, `defineAction` and `createLibrary`, whose output (the JSON schema and the LibrarySpec) is the core's input;
- the message protocol (`parseMessage` and `buildMessage`, the `]]>openui:` lines). It's a framing layer around a response, not parsing. Dynamic News may want it if it stores messages; it's a separate ticket if so.

## Keeping the check green during the move

Today only parse conformance has a pending list. The first ticket extends `pending.txt` to every test that compares with goldens: stages, merges, conversions back to source, error hints, prompts, and the stream-end check. Vendoring 1.0 then marks everything that differs as pending. Each later ticket shrinks the list, and the last one empties it. Every commit passes `scripts/check.sh`.

## Decisions, with recommendations

1. **Stream end in the Rust API:** add `StreamParser::finish() -> ParseResult`, upstream's `getResult(true)`. `push`, `set` and `result` keep their meaning, "not ended yet". In the reference, `push(chunk, true)` gives the same result as `push(chunk)` followed by `getResult(true)`, because `complete` only changes how the result is computed, not the parser's state. So one new method covers it. The binding gets `StreamSession.finish()`, and the viewer calls it when a stream ends.
2. **`root` argument:** keep the parameter. It now names only the library's root component, as upstream's `rootName` does. Document the change.
3. **Fixtures:** import the 54 fixture inputs as corpus cases: programs as parse cases, chunked ones with their chunks recorded, edits as merge cases, and `library.json` as both a schema and a prompt spec. The evaluated level stays out of scope.
4. **0.3.0:** don't keep supporting it. vue-lang, svelte-lang and angular-lang stay on 0.x upstream, but the core follows 1.0. A local git tag on the last 0.3.0-conformant commit keeps it reachable.
5. **The message protocol:** not in this move.

## Proposed tickets

1. **Pending lists for every golden test** (prefactor). Blocked by: none. On 0.3.0 nothing is pending, and the check stays green.
2. **Vendor lang-core 1.0.1, its tests and the spec's fixtures.**
   - The reference gets `zod`, and every stream in the goldens ends with a stream-end step. The core gets `finish()`, at first without the `$name` rule.
   - The test capture rebuilds `functions`, `actions` and `properties`, and a fixture importer brings in the 54 cases.
   - Goldens are regenerated, and everything that differs becomes pending.

   Blocked by: 1.
3. **Libraries: components from `properties`, action `$defs`, `$ref` inlining and the call registry.** Blocked by: 2.
4. **The entry rule:** `root`, the fallback and no entry; `no-root` with its severity; `rootExpression`; orphans; results without an entry. Blocked by: 2.
5. **Calls:** the `fn` flag, `unknown-function`, `mappedProps` for built-ins and steps, argument checks for library functions and actions, invalid steps as empty plans, Query and Mutation arguments, `{steps}` literals. Blocked by: 3.
6. **Validation:** data in component-only slots, and confirming copy-on-write matches 1.0's non-mutating rules, including the shared-default divergence. Blocked by: 3.
7. **Stream end:** the `$name` rule, the stream-end exceptions revisited, and `finish()` in the binding and the viewer. Blocked by: 2.
8. **Converting back to source, the prompt and error hints:** `@` for library calls, the 1.0 prompt, and `severity` in hints. Blocked by: 3.
9. **Finish line:**
   - nothing pending, and CONFORMANCE.md names 1.0;
   - divergences and QUIRKS.md reviewed against 1.0;
   - the fuzzer generates calls to library functions and actions;
   - the viewer uses 1.0's react-ui library spec;
   - a hunt reports no lang-core commits since the pin and no findings.

   Blocked by: 4, 5, 6, 7, 8.
