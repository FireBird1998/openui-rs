# 12 — Edit-mode merge, JSON-to-OpenUI, and error hints match

**What to build:** The rest of the reference package's language layer:
- merging a patch program into an existing one (edit mode, including garbage-collecting unreachable statements)
- turning a JSON element tree back into OpenUI Lang
- enriching validation errors with signature hints

Each is conformant against the reference parser, using inputs lifted from its merge and serialize tests.

**Blocked by:** 06 — One-shot parse matches.

**Status:** done (2026-10-06)

- [x] Merge output matches the reference for every case from its tests plus generated patch cases.
- [x] JSON-to-OpenUI output matches the reference for every case from its serialize tests.
- [x] Enriched errors match the reference for every validation case in the corpus.

## Comments

**2026-10-06, done.**
- **Built:** `src/merge.rs` (`merge_statements`), `src/serialize.rs` (`json_to_openui`), `src/enrich.rs` (`enrich_errors`, `EnrichedError`), and `parse_json` / `stringify_json` in `src/js.rs`.
  - Merge is a text-level port: the reference's own statement splitter, fences stripped from the patch only, `name = null` deletes, garbage collection from the root (or `root_id`), `$` statements always kept.
  - Serialize walks the element tree's JSON form with the JavaScript rules the reference relies on: `isElementNode`/`isASTNode`, `Object.entries` order, `String()` and template stringification (`undefined` empty in a join, `"undefined"` in text), inherited names (`__proto__`, `length`, methods), statement ids of any JSON type keyed as a `Set` keys them, operator precedence including inherited names, and every place it throws, returned as `Err`. The root's props may be a string, number or boolean, read by index as JavaScript does.
- **Corpus** (goldens in `conformance/golden/edits/`, written by `reference/edits.ts`):

  | | Reference tests | Hand-written | Generated | From the parse corpus | Fuzz |
  |---|---:|---:|---:|---:|---:|
  | merge | 1 | 46 | 1,370 | | 0 |
  | serialize | 30 | 57 | | 238 roots | 0 |
  | enrich | | | | 187 error lists | |

  - The reference tests' calls are captured live, as parse inputs already were: `capture-tests.ts` now wraps `mergeStatements` and `jsonToOpenUI`, and the importer writes `imported/merge.json` and `imported/serialize.json`. Re-importing left the parse corpus byte-identical.
  - Generated merges (`reference/merge-cases.ts`): for each of the 235 corpus programs with a root, edit a string, add a child, delete a statement, drop the root's first child, a fenced patch with prose, a non-default root, and the program replayed onto itself and onto nothing.
  - Enrich covers all six error codes, with and without hints, and without component names.
- **Found and fixed in the parser:** the library loader read `$defs` and `properties` only as objects. The reference reads them with `Object.entries`, so an array or string is read by index. New parse case `hand/86-odd-schema-shapes` (schema `odd`), which also covers the signature hint's odd branches.
- **Fuzzing:** `scripts/fuzz.sh --edits` compares full outputs with the reference: merges of corpus and generated programs with patches made from them, and serialization of real parse results with fields deleted, retyped or renamed. 100,000 inputs over seeds 1–5: 0 mismatches, 0 panics. (The first run found the primitive-root-props divergence I had chosen; now emulated.) Planted faults in merge and serialize are both caught.
- **Robustness:** never-panic property tests for merge, the JSON reader, serialize and enrich, with planted panics confirming each is reached; deep-input tests now also read back, serialize and merge the deepest results on a 512 KB stack.
- **Stack budget corrected:** ticket 09's "256 KB optimized" was wrong; the 10,000-statement reference chain needed 384 KB in release builds even then. Now measured per shape: at most 384 KB in test and release builds. To keep it there, the serializer's recursion uses small frames, and the JSON reader is iterative (serde_json's recursive reader needed 512 KB at 800 levels in release).
- **Quirks 6–10** recorded with draft upstream reports: merge drops ternary continuation lines; prose with an apostrophe before the existing program loses it; an id defined twice comes out twice; serializing drops parentheses around an equal-precedence right operand (`a - (b - c)` becomes `a - b - c`); object keys aren't quoted.
- **Found, not fixed (out of scope):** components nested in a runtime expression cost exponential memory in the core. Proposed as ticket 14.
