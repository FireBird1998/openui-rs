# 05 — Materializing: memoized references

**What to build:** Materializing matches 1.1:
- a statement reached many times materializes once per pass, unless an unresolved or cyclic reference was met on the way;
- its errors are reported once;
- an element reached through an alias is copied before it's tagged with the alias's statement;
- the dynamic check stops at `hasDynamicProps`.

The core's own guards against exponential results (`MAX_RESULT_SIZE`, `size_limited`) are checked against the reference's new behaviour. The divergence in CONFORMANCE.md is updated to say what still differs.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** done (2026-11-06)

- [x] The parse cases pending for shared statements, aliases or repeated errors match.
- [x] Cases cover a statement used twice with an error, an alias of the root, and a reference reached once through a cycle and once not.
- [x] The size divergence is re-measured and CONFORMANCE.md updated.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **Errors once:** the core's cache already shares a statement's materialization across a pass and across pushes. On a hit it replayed the statement's errors, as 1.0 did, re-materializing every time. A pass now notes each statement it has materialized, as a value and as an expression, and a repeat replays its errors only if it met an unresolved reference. Those are the ones 1.1 doesn't memoize, a cycle included.
- **Aliases and `hasDynamicProps`:** nothing to do. The core's elements are shared copy-on-write, so tagging an alias copies already. An element answers the dynamic check from its own flag already.
- **New cases:**
  - `hand/125`: an error used four times reports once;
  - `hand/126`: aliases of the root and of a child;
  - `hand/127`: an error inside a cycle, reached twice, reports twice;
  - `hand/128`: an unresolved reference inside a statement used twice reports twice;
  - `hand/129`: one statement as a value and in expressions.

  Without the change, `hand/125` has 3 extra errors.
- **The size divergence, re-measured on the worker Mac:** 1.1's parse of a doubling program takes under 3 ms at every size. The JSON doubles per level, though: 248 MB at 20 levels, 994 MB at 22, and `JSON.stringify` runs out of memory at 24, with 4.4 GB resident. CONFORMANCE.md says so; the core still cuts at 8 MiB.
- No pending parse case was about repeated errors: the 77 left are queries and state defaults (tickets 06 and 07).
