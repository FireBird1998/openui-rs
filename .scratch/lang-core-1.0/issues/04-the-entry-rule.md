# 04 — The entry rule

**What to build:** Parsing picks its entry as 1.0 does, and reports what 1.0 reports:
- **The entry:** the `root` statement; else the first statement that calls the library's root component (the `root` argument, which no longer names a statement); else none.
- **`no-root`:** with no entry, an error; with the fallback, a warning (`severity: "warning"`); with a `root` that isn't a component, an error. The parser reports it while streaming too.
- **No entry:** the result is no longer empty. State, queries and mutations are still extracted, and there are no orphans.
- **`rootExpression`:** a root whose value is an expression that can produce a component (`root = $t ? A : B`) is exposed there.

This is the bulk of the move: it changes 61 one-shot results and the early streaming steps of nearly every case.

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** done (2026-10-16)

- [x] `ParseResult` has `root_expression`, and validation errors have `severity`, in the reference's JSON places.
- [x] Every pending case whose difference is the entry rule conforms.
- [x] The `root` argument's new meaning is documented.
- [x] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, done.** The core picks its entry as 1.0 does: `root`; else the first statement whose value calls the library's root component (the root argument, which no longer names a statement), as a fallback; else none.
- **`no-root`:** reported after state and queries are extracted, with 1.0's three wordings: missing, not a component, or the fallback, which carries `severity: "warning"`. A `root` that resolves to a component call through references, even one validation dropped, doesn't report.
- **Without an entry:** the result isn't empty any more, and it has no orphans.
- **`root_expression`:** holds an expression root that can produce a component (`$t ? A : B`, `@Each(..., Card(...))`).
- **API:** `ValidationError` and the enriched errors have `severity`, written where the reference writes it; the error-hint part was ticket 08's, and came along. A component call now excludes reserved names as `name in {Query, Mutation}` reads them (so `toString(...)` isn't one), as 1.0 does.

**The gap:** 340 of the 520 pending parse cases conform now (349 of 529 in all, with five new hand cases: an `@Each` root, a root through references, a dropped fallback, a prototype-named root, and a root in a reference cycle). Enriched errors went from 229 pending to 132. None of the 180 left differs in its entry, orphans or `rootExpression`:
- 64 differ only while streaming: the half-typed `$name` (ticket 07), and the component list and action-slot signatures (ticket 03);
- the rest are calls (`mappedProps`, `fn`, `unknown-function`, `{steps}` literals: ticket 05), data in component slots (ticket 06), and the same ticket 03 differences.

**A correction:** lang-core 1.0 still returns an element-shaped object literal as the root. An earlier check gave the object no entry, and concluded wrongly; CONFORMANCE.md and ticket 14 said so, and commit `0610c25`'s message does too. The divergence stands, and the core now counts such a root as one, so it reports no `no-root` and only `root` differs. A new row documents its sibling: an object literal shaped like an AST node as the root, which 1.0 returns as `rootExpression`.
