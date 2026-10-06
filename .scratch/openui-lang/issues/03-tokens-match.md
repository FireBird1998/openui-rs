# 03 — Tokens match the reference

**What to build:** The full lexer. For every corpus program and its prefixes, the core's tokens match the tokens from the reference package's exported `tokenize`. This adds a stage-level conformance check alongside the whole-result one.

**Blocked by:** 02 — Real-world corpus.

**Status:** needs-triage

- [ ] Tokens for every corpus program and every prefix of it match the reference parser exactly.
- [ ] Strings with escapes, numbers in every form, identifiers vs component names vs state refs (`$x`) vs builtins (`@X`), and operators are covered by named cases.
- [ ] Tokenizing arbitrary text never panics.
