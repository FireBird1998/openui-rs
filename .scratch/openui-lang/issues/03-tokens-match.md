# 03 — Tokens match the reference

**What to build:** The full lexer. For every corpus program and its prefixes, the core's tokens match the tokens from the reference package's exported `tokenize`. This adds a stage-level conformance check alongside the whole-result one.

**Blocked by:** 02 — Real-world corpus.

**Status:** done (2026-10-06)

- [x] Tokens for every corpus program and every prefix of it match the reference parser exactly.
- [x] Strings with escapes, numbers in every form, identifiers vs component names vs state refs (`$x`) vs builtins (`@X`), and operators are covered by named cases.
- [x] Tokenizing arbitrary text never panics.

## Comments

**From ticket 01:** strings unescape through `serde_json`, standing in for `JSON.parse`. They differ on lone-surrogate escapes such as `"\ud800"`: JSON.parse accepts them and serde_json errors, which sends the core down the "strip the quotes" fallback. Decide what the core should return there (a Rust `String` can't hold a lone surrogate), and record it as a quirk if it can't match. Numbers go through JS `+str`, so `1e` is `NaN` (which JSON writes as `null`) and `-` handling depends on the previous token. Port that exactly.

**2026-10-06, done.**
- Full lexer port, working on bytes. `tokens_to_json` writes what `JSON.stringify(tokenize(...))` writes (numeric `t` kinds, `v` only where the reference has it). Numbers use `ryu-js`, so `1e+21`, `1e-7` and `-0`→`0` match, and NaN or Infinity is `null`. `+text` semantics: a dangling `1e` is NaN.
- Strings: `serde_json` was replaced by `js::json_parse_string`, which accepts and rejects exactly what `JSON.parse` does. Lone surrogates are carried in U+10F800–U+10FFFF; see the spec's input decisions. The case that forced this is `hand/69-lexer-string-forms` at 37 chars: a stream stopped between `\uD83D` and `\uDE00`.
- Stage golden: `<case>.tokens.json` holds a fingerprint of the tokens for every code-point prefix of the raw program. `bun run.ts show-tokens <case> <length>` prints one of them. `tests/stages.rs` checks all of them (64,495 prefixes over 270 cases) and passes. Shared test helpers moved to `tests/common/`.
- 5 new hand cases (66–70) cover operators, number forms, identifier forms, string forms and stray characters.
- `tests/robustness.rs`: proptest, 2 × 2000 cases (any string, and lexer-heavy text) never panic and always end in Eof.
- `openui-lang tokens <program>`.
- Whole-result conformance went from 26/265 to 29/270 (3 cases newly conform). `scripts/check.sh` takes about 13 s; the token stage test is about 4 s in debug.
