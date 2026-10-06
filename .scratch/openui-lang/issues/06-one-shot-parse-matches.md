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

**Status:** needs-triage

- [ ] Every one-shot corpus case conforms, and none remain on the pending list.
- [ ] Number formatting matches JavaScript for integers, decimals, exponents, `-0` and very large and very small values.
- [ ] All six validation error codes are produced with the same component, path, statement id and message as the reference parser.
- [ ] A custom root name works as it does in the reference parser.
