# 07 — State defaults and built-in functions

**What to build:** Expression state defaults are evaluated at parse time as 1.1 does:
- built-in functions, operators, member access and ternaries over data;
- the defaults declared above read as state;
- query results read as null;
- any call that isn't a built-in function makes the default null.

`@Round`, `@Abs`, `@Floor` and `@Ceil` apply to each element of a list, nested lists too, and their prompt signatures say so. The evaluator covers only what parsing evaluates. Where it can't match the reference exactly (`@Sort`'s locale collation, say), the difference is a documented divergence, kept off real programs.

**Blocked by:** 05 — Materializing

**Status:** ready-for-agent

- [ ] The parse cases pending for state defaults match, and no case is pending in `prompt.txt`.
- [ ] Cases cover each built-in function in a default, operators with JavaScript's coercions, list and object defaults, a default reading an earlier one and a later one, a statement reference, a query read, and a library call.
- [ ] Any divergence is in CONFORMANCE.md, and the fuzzer counts it rather than reporting it.
- [ ] `scripts/check.sh` passes.
