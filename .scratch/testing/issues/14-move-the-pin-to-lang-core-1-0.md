# 14 — Move the pin to lang-core 1.0

**What to build:** The core conforms to `@openuidev/lang-core` 1.0 instead of 0.3.0. The vendored reference, the imported corpus and every golden come from the new upstream commit, and the core matches all of them, so the crate's conformance claim names 1.0.

The first hunts (testing ticket 02) found upstream had moved. There are 11 lang-core commits on main since the pin (`bba1d25`, 0.3.0), up to `eb20708` (1.0.1, 2026-10-15), which include "OpenUI 1.0" (#1348) and "report plain objects in component slots as type-mismatch" (#1287). With 1.0.1, our corpus's outputs change for:
- 305 of 309 parse cases (one-shot results for 79 of them, streaming steps for all 305);
- 25 of 335 conversions back to source;
- 77 of 191 error hints;
- 27 of 47 system prompts.

Changes seen so far:
- A streaming result whose root doesn't evaluate to a component yet carries a `no-root` error. This is why nearly every case's early steps change.
- Built-in action calls carry `mappedProps`.
- An object literal shaped like an element is no longer returned as the root. That removes one of our documented divergences.
- The parser layer's source changes by about 1,300 lines, across `parser`, `materialize`, `validation`, `builtins`, `prompt`, `serialize`, `ast` and `types`. `library.ts` is reworked, and there are new `extend.ts`, `signature.ts`, `message.ts` and `functions.ts`.
- It imports `zod/v4` at run time, a peer dependency (`^3.25.0 || ^4.0.0` in upstream's pnpm catalog). The reference's packages need it.
- Its runtime telemetry is opt-in (`OPENUI_RUNTIME_TELEMETRY_ENABLED`) and honours `OPENUI_TELEMETRY_DISABLED` and `DO_NOT_TRACK`, which every reference script sets.

`scripts/upstream-check.sh <folder>` reproduces all of this: the new lang-core, the packages it imports, the new goldens for the files that change, and the full list of changed cases.

This is likely more than one ticket. Its first step is a map of the changes per area of the core, which then splits it into tickets, the way the Dynamic News map did.

**Blocked by:** None — can start immediately, once the maintainer decides to move.

**Status:** in progress (the change map is done; its tickets wait for approval)

- [x] A change map: each upstream change in the parser layer, the core module it touches, and whether our corpus covers it. Upstream's new tests are imported as cases.
- [ ] The reference is vendored at the new commit with `scripts/update-reference.sh`, with `zod` added to its packages. The corpus is re-imported, and every golden is regenerated.
- [ ] The core matches every golden, one-shot and streaming. `pending.txt` is empty again.
- [ ] CONFORMANCE.md names the new version, and its divergences and QUIRKS.md are reviewed against 1.0: some go away, and new ones may appear.
- [ ] The binding and the viewer still pass their tests. The viewer's system prompts come from the new core.
- [ ] A hunt afterwards reports no lang-core commits since the pin, and no findings.
- [ ] `scripts/check.sh` passes.

## Comments

**Recommendation (2026-10-16):** move, and do it before testing tickets 10 and 11. The recorded streams' system prompts come from the core, and 27 of 47 prompts change with 1.0. Recordings made against 0.3.0 prompts would test the old language. Tickets 04 to 09 and 12 work either way. Running them first means their results are on 0.3.0, so mutation testing in particular would want a rerun on the parser layer after the move.

**2026-10-16:** the maintainer decided to move. The change map is [../../lang-core-1.0/map.md](../../lang-core-1.0/map.md): each upstream change per area of the core, what our corpus covers, five decisions with recommendations, and nine proposed tickets. Upstream's 1.0 conformance fixtures (54 cases) join the corpus as inputs, since the core has no evaluator to check them as upstream does.
