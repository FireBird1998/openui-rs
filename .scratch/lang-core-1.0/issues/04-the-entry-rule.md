# 04 — The entry rule

**What to build:** Parsing picks its entry as 1.0 does, and reports what 1.0 reports:
- **The entry:** the `root` statement; else the first statement that calls the library's root component (the `root` argument, which no longer names a statement); else none.
- **`no-root`:** with no entry, an error; with the fallback, a warning (`severity: "warning"`); with a `root` that isn't a component, an error. The parser reports it while streaming too.
- **No entry:** the result is no longer empty. State, queries and mutations are still extracted, and there are no orphans.
- **`rootExpression`:** a root whose value is an expression that can produce a component (`root = $t ? A : B`) is exposed there.

This is the bulk of the move: it changes 61 one-shot results and the early streaming steps of nearly every case.

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** ready-for-agent

- [ ] `ParseResult` has `root_expression`, and validation errors have `severity`, in the reference's JSON places.
- [ ] Every pending case whose difference is the entry rule conforms.
- [ ] The `root` argument's new meaning is documented.
- [ ] `scripts/check.sh` passes, with the pending lists shorter.
