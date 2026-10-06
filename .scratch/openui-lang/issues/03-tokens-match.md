# 03 — Tokens match the reference

**What to build:** The full lexer. For every corpus program and its prefixes, the core's tokens match the tokens from the reference package's exported `tokenize`. This adds a stage-level conformance check alongside the whole-result one.

**Blocked by:** 02 — Real-world corpus.

**Status:** ready-for-agent

- [ ] Tokens for every corpus program and every prefix of it match the reference parser exactly.
- [ ] Strings with escapes, numbers in every form, identifiers vs component names vs state refs (`$x`) vs builtins (`@X`), and operators are covered by named cases.
- [ ] Tokenizing arbitrary text never panics.

## Comments

**From ticket 01:** strings unescape through `serde_json`, standing in for `JSON.parse`. They differ on lone-surrogate escapes such as `"\ud800"`: JSON.parse accepts them and serde_json errors, which sends the core down the "strip the quotes" fallback. Decide what the core should return there (a Rust `String` can't hold a lone surrogate), and record it as a quirk if it can't match. Numbers go through JS `+str`, so `1e` is `NaN` (which JSON writes as `null`) and `-` handling depends on the previous token. Port that exactly.
