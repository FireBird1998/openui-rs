# 07 — State defaults and built-in functions

**What to build:** Expression state defaults are evaluated at parse time as 1.1 does:
- built-in functions, operators, member access and ternaries over data;
- the defaults declared above read as state;
- query results read as null;
- any call that isn't a built-in function makes the default null.

`@Round`, `@Abs`, `@Floor` and `@Ceil` apply to each element of a list, nested lists too, and their prompt signatures say so. The evaluator covers only what parsing evaluates. Where it can't match the reference exactly (`@Sort`'s locale collation, say), the difference is a documented divergence, kept off real programs.

**Blocked by:** 05 — Materializing

**Status:** done (2026-11-06)

- [x] The parse cases pending for state defaults match, and no case is pending in `prompt.txt`.
- [x] Cases cover each built-in function in a default, operators with JavaScript's coercions, list and object defaults, a default reading an earlier one and a later one, a statement reference, a query read, and a library call.
- [x] Any divergence is in CONFORMANCE.md, and the fuzzer counts it rather than reporting it.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **Prompts:** the four rounding built-ins' signatures now read `number | number[]`. All 23 pending prompts match.
- **The evaluator** (`src/evaluate.rs`) is a port of the runtime evaluator, limited to what parsing reaches. Its values are JavaScript's:
  - `undefined` apart from `null`;
  - arrays and objects compared by identity;
  - loose `==`, `Number()` and `String()`;
  - member access plucking from lists;
  - UTF-16 string indexing;
  - built-in methods and `__proto__` read off values.

  It implements every built-in function, `@Each` with its literal substitution included. Defaults are evaluated after all the materializing, in statement order, each reading the defaults above it; one that comes out `undefined` is left out of the result but counts as declared. Programs without expression defaults skip it.
- **`@Sort`:**
  - **Strings:** `localeCompare` is ICU's root collation. The core has ASCII exactly: a weight table measured in Bun, with case as the tie-breaker. It matched Bun on 20,000 random ASCII pairs, controls and punctuation included.
  - **Sorting:** JavaScriptCore's `sort`, traced in Bun, is binary insertion under 8 items, and run detection then binary insertion from 8 to 65. The core does the same, matching Bun on 9,000 random lists whose comparisons loop. Past 65 it uses a stable merge sort. `slice::sort_by` was dropped, because it may panic when a comparison loops.
- **New cases:** `hand/135` to `hand/140`: operators and coercions, every built-in function, lists and objects, defaults that come out null, sorting text, and sorting mixed values that loop.
- **Divergences:** four rows in CONFORMANCE.md: text outside ASCII, more than 65 looping values, properties of built-in methods, and AST-shaped object literals in a default. The fuzzer counts the three it can recognize from the text.
- **Result:** every pending list is empty. The core conforms to lang-core 1.1.0 on the whole corpus. `scripts/check.sh` and `scripts/check-viewer.sh` pass.
