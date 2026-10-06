# 06 — One-shot parse matches on the whole corpus

**What to build:** Everything from text to a complete parse result:
- the component library compiled from JSON Schema
- materializing elements: positional args to named props, schema defaults, partial and dynamic-props flags
- validation errors
- unresolved and orphaned statements
- state, query and mutation extraction
- choosing the entry statement and the root name
- JavaScript-identical JSON output

Every one-shot case in the corpus conforms.

**Blocked by:** 04 — Statements and expressions match; 05 — Fences, comments and prose.

**Status:** done (2026-10-06)

- [x] Every one-shot corpus case conforms, and none remain on the pending list.
- [x] Number formatting matches JavaScript for integers, decimals, exponents, `-0` and very large and very small values.
- [x] All six validation error codes are produced with the same component, path, statement id and message as the reference parser.
- [x] A custom root name works as it does in the reference parser.

## Comments

**From ticket 01:**
- Schema defaults aren't applied to missing or null required props yet. That needs `Value` to cover every JSON type.
- `compileSchema` walks `Object.entries`, which puts integer-like keys ("0", "12") first in ascending order, then the rest in insertion order. `serde_json`'s `preserve_order` keeps document order. Add a case with an integer-like property name and match JS order.
- Enum and const values in signatures currently use serde_json's number formatting. Switch to the JS number formatter.
- Query, mutation and state statements aren't classified yet, and `to_json` writes `[]` for them.

**From ticket 04:** `materialize_value` maps every node kind except Str, Null, Ph, Ref and Comp to null for now. Ticket 06 replaces that with the real lowering: Num, Bool, Arr and Obj to plain values (arrays drop Ph, and drop null Comp or Ref results), and runtime expressions kept as AST via `materializeExpr` (which adds `mappedProps` to catalog Comp nodes and turns Query/Mutation refs into `RuntimeRef`). `Value` needs a variant for an AST node, and `Ast::Comp` needs optional `mapped_props` (key order `k, name, args, mappedProps`).

**2026-10-06, done: all 284 cases conform, one-shot and at every streaming step; `pending.txt` is empty.**
- Full ports of materialize/materializeExpr (lazy `@Each` scoping, `mappedProps`, `RuntimeRef`), recursive validation (all six error codes, the edge rule, enum deferral while streaming), statement classification (value/state/query/mutation) and extraction (state declarations including auto-declared `$vars`, query deps).
- JavaScript object semantics reproduced, because they leak into the output:
  - `js::JsObject` iterates array-index keys first. That changes props order, positional param order and the `$defs` order in "Available components" (quirk 3).
  - `in` sees the 12 `Object.prototype` names: in `isReservedCall` (quirk 2), in missing-required checks, and in object validation, where an inherited method validates as a function.
  - Assigning `__proto__` is dropped, while `JSON.parse` (schema defaults) keeps it as an own key.
  - Objects shaped like AST nodes or elements count as such (`isASTNode`/`isElementNode`).
  - `required` given as a string matches substrings.
  - serde_json `float_roundtrip` makes schema numbers parse exactly like `JSON.parse`.
- New cases: hand 80–83 (root names: statement, preferred component, missing, `root` wins) and hand 84 with a `keys` schema (index-like param names, a `__proto__` param, a default filling a missing required prop).
- The one-shot test (`every_one_shot_parse_matches_the_reference_parser`, no pending list) passed on the first run, so I ran eight deliberate breaks. Six were caught by one-shot cases. Break 5 (enum checks while streaming) is caught by streaming conformance. Break 3 (accepting an empty `@Each` variable) can't change any output, because both paths give the same tree.
- Known divergences, documented rather than emulated:
  - An object literal shaped like an element used as the root: the reference returns that object as `root`, the core returns `None`.
  - The reference shares and can mutate schema default objects across parses; the core copies them.
  - Assigning an object to `__proto__` changes later `in` checks in the reference.
  None of these are in the corpus.
- `conformance/QUIRKS.md`: quirk 2 (`@toString()` and friends treated as Query/Mutation) and quirk 3 (numeric-looking prop names reorder positional args).
