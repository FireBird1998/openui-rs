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

**Status:** ready-for-agent

- [ ] Every one-shot corpus case conforms, and none remain on the pending list.
- [ ] Number formatting matches JavaScript for integers, decimals, exponents, `-0` and very large and very small values.
- [ ] All six validation error codes are produced with the same component, path, statement id and message as the reference parser.
- [ ] A custom root name works as it does in the reference parser.

## Comments

**From ticket 01:**
- Schema defaults aren't applied to missing or null required props yet. That needs `Value` to cover every JSON type.
- `compileSchema` walks `Object.entries`, which puts integer-like keys ("0", "12") first in ascending order, then the rest in insertion order. `serde_json`'s `preserve_order` keeps document order. Add a case with an integer-like property name and match JS order.
- Enum and const values in signatures currently use serde_json's number formatting. Switch to the JS number formatter.
- Query, mutation and state statements aren't classified yet, and `to_json` writes `[]` for them.
