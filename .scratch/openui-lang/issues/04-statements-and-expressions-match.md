# 04 — Statements and expressions match the reference

**What to build:** Splitting tokens into statements, auto-closing an incomplete trailing statement, and parsing expressions into every node kind the reference parser has. Stage-level conformance against the reference package's exported `split`, `autoClose` and `parseExpression`, for every corpus program and its prefixes.

**Blocked by:** 03 — Tokens match the reference.

**Status:** done (2026-10-06)

- [x] `split` and `autoClose` output matches the reference for every corpus program and prefix, including the "was incomplete" flag.
- [x] Parsed expression trees match the reference for every statement in the corpus.
- [x] Operator precedence, ternaries, member and index chains, and assignments have named cases.

## Comments

**2026-10-06, done.**
- `auto_close` and `split` were already full ports from ticket 01. The expression parser is now the full Pratt parser: every node kind the parser produces (Comp, Str, Num, Bool, Null, Arr, Obj, Ref, StateRef, BinOp, UnaryOp, Ternary, Member, Index, Assign; Ph comes from materialize). `Ast::to_json` writes the reference key order. Member names and object keys from numbers use `js::number_to_string` (JS `String(n)`).
- Stage golden: `<case>.statements.json` fingerprints one document per prefix, `{autoClose, statements: [{id, idTokenType, tokens, expr}]}`, built exactly as `bun run.ts show-statements <case> <length>` prints it. `tests/stages.rs` now shares one every-prefix loop between the token and statement stages. All 271 cases match on all 64,683 prefixes.
- The test passed on the first run, so I checked it can fail. Swapping `*` to `+` precedence, making ternaries left-associative, keeping `$` in member keys, and binding assignment tighter were each caught by named cases (hand 20, 22, 32, 39, 48, 60, 66 and the new 71-expression-forms).
- `[profile.test] opt-level = 1`: the stage tests went from about 9 s to 2.5 s.
- Whole-result conformance is unchanged at 29/271 (the new case 71 was added to pending). Materialize still turns the new node kinds into null until ticket 06.
