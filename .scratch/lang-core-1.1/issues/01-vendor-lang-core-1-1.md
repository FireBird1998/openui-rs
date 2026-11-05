# 01 — Vendor lang-core 1.1.0 and its new tests

**What to build:** The reference is lang-core 1.1.0 (`47b9cae`), every golden comes from it, and the corpus holds upstream's new tests. Whatever the core doesn't match yet is pending, so the check passes and the gap is listed case by case.

- **The reference:** vendored at the new commit; `scripts/update-reference.sh` reports any package it newly imports.
- **Upstream's new tests:** `toolCompatibility.test.ts` and `toolReferences.test.ts` captured with the others by the corpus import.
- **Goldens:** regenerated; the generated merges regenerated over the new corpus.
- **A local tag** `lang-core-1.0.1` on the last 1.0.1-conformant commit.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [ ] The reference runs lang-core 1.1.0, and `reference/UPSTREAM` names it.
- [ ] The tag `lang-core-1.0.1` marks the last 1.0.1-conformant commit.
- [ ] Upstream's two new test files are captured as corpus cases.
- [ ] Every golden is regenerated, and the pending lists hold exactly what differs.
- [ ] CONFORMANCE.md says the move is in progress, with the pending counts.
- [ ] `scripts/check.sh` and `scripts/check-viewer.sh` pass.
