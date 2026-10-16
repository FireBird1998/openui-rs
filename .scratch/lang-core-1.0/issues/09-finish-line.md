# 09 — Finish line

**What to build:** The core conforms to lang-core 1.0.1 with nothing pending, and everything around it says so:
- every pending list is empty, and CONFORMANCE.md names 1.0.1 with its counts;
- each divergence and quirk is reviewed against 1.0: removed if it's gone, rewritten if it changed, and new ones added with draft upstream reports for the maintainer;
- the fuzzer's generator writes calls to library functions and actions, and uses 1.0 libraries;
- the viewer uses 1.0's react-ui library spec, so its system prompts are 1.0's;
- a hunt reports no lang-core commits since the pin, and no findings.

**Blocked by:** 04 — The entry rule; 05 — Calls; 06 — Validation; 07 — Stream end; 08 — Converting back to source, the prompt and error hints.

**Status:** done (2026-10-16)

- [x] Every pending list is empty.
- [x] CONFORMANCE.md, QUIRKS.md, TESTING.md and the README describe 1.0.
- [x] A hunt of at least ten minutes finds nothing and reports the pin current.
- [x] `scripts/check.sh` and `scripts/check-viewer.sh` pass.

## Comments

**Candidate quirks found during the move, to review here:**
- **A name that's both a listed component and a library action** (found in ticket 03): a bare call to it materializes as the action, because 1.0 checks the call registry before components. `hand/102` covers it.
- **The stream end and a pending redefinition** (the change map): the spec says a final redefinition wins at stream end, but the reference skips a pending redefinition that needed closing even with `complete`. Ticket 07 checks it.

**2026-10-16, done.**
- **Pending lists:** all empty. 534 parse cases (every push, every chunking, every stream end), 532 staged, 61 prompts, 2,521 merges, 506 trees back to source and 339 enriched error lists match 1.0.1.
- **Fuzzing:** the generator writes calls to the library's functions and actions (with `@` mostly, sometimes bare, sometimes as one of several steps, with too few, enough or too many args). It also writes 1.0's call forms: a step on its own, a step's context, bad and missing args, `@` names nothing registers, and `Action` without its list. The libraries it draws from include the 1.0 ones (`libraries`, the spec fixtures', the reference tests', react-ui's).
  - 140,000 inputs (parsing, seeds 41–44 at 25,000; edits, seeds 41 and 42 at 20,000), 0 unexplained after one fix.
  - **The fix:** 1.0 assigns each prop twice, raw and then validated, so `null` for a param named `__proto__` drops the props' prototype, and the second assignment makes `__proto__` an own prop. The core now does the same (`JsObject` tracks a null prototype). `hand/105-proto-named-param` pins it. An object value still replaces the prototype silently: that's the documented divergence.
  - **Newly explained:** a root that's an object literal shaped like an AST node. The reference walks it as one, and throws a `TypeError` when a node lacks the children its kind has (`root = {k: "Comp"}`). The divergence row says so now, and the fuzzer recognizes it, through a chain of references too.
- **react-ui's 1.0 spec:** the import loads upstream's `packages/react-ui` at the pin with React and its other dependencies stubbed out (`reference/react-ui-stubs.ts`, `react-ui-spec.ts`), so nothing is installed. It writes the library's JSON Schema (`imported/schemas/react-ui.json`, 82 components) and two prompt specs, `react-ui-schema` (with the package's own examples and rules) and `react-ui-schema-tools`. The library's own `prompt()` equals `generateSystemPrompt` of the exported spec, and the core matches both prompts. The viewer asks Claude with react-ui by default, and a follow-up keeps the spec's examples and rules with edit mode on (before, edit mode replaced them).
- **Quirks:** every repro run again on 1.0.1; all 11 still hold, and each entry says so. New quirk 12: a library function or action called without `@` is still called, and wins over a component of the same name (the candidate from ticket 03). language.md, section 1.5 says custom functions need the `@` and only `Action` works without it. `hand/106-bare-library-calls` pins it.
- **Divergences:** each row checked on 1.0.1. The `__proto__` row now says the null-param case is reproduced, and the AST-shaped root row adds the reference's `TypeError`.
- **Docs:** CONFORMANCE.md, README, TESTING.md and QUIRKS.md describe 1.0.1, with nothing pending.
- **The hunt:** 12 minutes, seed 2124613667: 664,000 programs, 4,490,641 merges and 4,488,959 trees back to source, with no findings. The upstream check found no lang-core commits since the pin. The 589 known divergences are 454 roots shaped like an AST node (the reference throws on them) and 135 `__proto__` assignments. The slowest program took the core 195 ms.
- **Checks:** `scripts/check.sh`, `scripts/check-viewer.sh` (58 tests) and `scripts/coverage.sh` (99.26% of lines, every line no test runs explained) pass.

