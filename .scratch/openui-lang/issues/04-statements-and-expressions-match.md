# 04 — Statements and expressions match the reference

**What to build:** Splitting tokens into statements, auto-closing an incomplete trailing statement, and parsing expressions into every node kind the reference parser has. Stage-level conformance against the reference package's exported `split`, `autoClose` and `parseExpression`, for every corpus program and its prefixes.

**Blocked by:** 03 — Tokens match the reference.

**Status:** needs-triage

- [ ] `split` and `autoClose` output matches the reference for every corpus program and prefix, including the "was incomplete" flag.
- [ ] Parsed expression trees match the reference for every statement in the corpus.
- [ ] Operator precedence, ternaries, member and index chains, and assignments have named cases.
