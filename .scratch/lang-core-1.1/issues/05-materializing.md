# 05 — Materializing: memoized references

**What to build:** Materializing matches 1.1:
- a statement reached many times materializes once per pass, unless an unresolved or cyclic reference was met on the way;
- its errors are reported once;
- an element reached through an alias is copied before it's tagged with the alias's statement;
- the dynamic check stops at `hasDynamicProps`.

The core's own guards against exponential results (`MAX_RESULT_SIZE`, `size_limited`) are checked against the reference's new behaviour. The divergence in CONFORMANCE.md is updated to say what still differs.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** ready-for-agent

- [ ] The parse cases pending for shared statements, aliases or repeated errors match.
- [ ] Cases cover a statement used twice with an error, an alias of the root, and a reference reached once through a cycle and once not.
- [ ] The size divergence is re-measured and CONFORMANCE.md updated.
- [ ] `scripts/check.sh` passes.
