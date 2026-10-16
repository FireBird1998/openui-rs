# 02 — Vendor lang-core 1.0.1, its tests and the spec's fixtures

**What to build:** The reference is lang-core 1.0.1 (`eb20708`), every golden comes from it, and the corpus holds what 1.0 adds to test against. The core's remaining differences from 1.0 are all pending, so the check passes and the gap is listed case by case.

- **The reference:** vendored at the new commit, with `zod`, which 1.0 imports at run time, in its packages. A local git tag marks the last commit that conforms to 0.3.0.
- **Stream end:** every stream in the goldens ends with a stream-end step (upstream's `getResult(true)`), and so does the fuzzer's. The core gets `StreamParser::finish()`, which for now gives the same result as `result()`. The `$name` rule comes in ticket 07.
- **Upstream's new tests:** imported at the new commit. The test capture rebuilds a library's `properties`, `functions` and `actions` from what the parser was given.
- **The spec's fixtures:** the 54 cases of `spec/fixtures/1.0/` become corpus cases, keeping where each came from:
  - each program is a parse case;
  - a chunked case keeps its chunks, as their own case;
  - each edit (program plus patch) is a merge case;
  - the fixture library is both a schema and a prompt spec.
- **Goldens:** regenerated. Whatever the core doesn't match yet is pending.

**Blocked by:** 01 — Pending lists for every golden test.

**Status:** ready-for-agent

- [ ] The reference runs lang-core 1.0.1, and `reference/UPSTREAM` names it.
- [ ] A local tag marks the last 0.3.0-conformant commit.
- [ ] Goldens end every stream with a stream-end step, and the core has `finish()`.
- [ ] Upstream's 1.0 tests and the 54 fixtures are corpus cases with their sources recorded.
- [ ] Every golden is regenerated, and the pending lists hold exactly what differs.
- [ ] CONFORMANCE.md says the move is in progress, with the pending counts.
- [ ] `scripts/check.sh` passes.
