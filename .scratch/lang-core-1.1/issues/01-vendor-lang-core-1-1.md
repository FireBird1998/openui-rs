# 01 — Vendor lang-core 1.1.0 and its new tests

**What to build:** The reference is lang-core 1.1.0 (`47b9cae`), every golden comes from it, and the corpus holds upstream's new tests. Whatever the core doesn't match yet is pending, so the check passes and the gap is listed case by case.

- **The reference:** vendored at the new commit; `scripts/update-reference.sh` reports any package it newly imports.
- **Upstream's new tests:** `toolCompatibility.test.ts` and `toolReferences.test.ts` captured with the others by the corpus import.
- **Goldens:** regenerated; the generated merges regenerated over the new corpus.
- **A local tag** `lang-core-1.0.1` on the last 1.0.1-conformant commit.

**Blocked by:** None — can start immediately

**Status:** done (2026-11-06)

- [x] The reference runs lang-core 1.1.0, and `reference/UPSTREAM` names it.
- [x] The tag `lang-core-1.0.1` marks the last 1.0.1-conformant commit.
- [x] Upstream's two new test files are captured as corpus cases.
- [x] Every golden is regenerated, and the pending lists hold exactly what differs.
- [x] CONFORMANCE.md says the move is in progress, with the pending counts.
- [x] `scripts/check.sh` and `scripts/check-viewer.sh` pass.

## Comments

**2026-11-06, done.** The reference is lang-core 1.1.0 (`47b9cae`). It needs no packages beyond 1.0.1's. The local tag `lang-core-1.0.1` marks the last 1.0.1-conformant commit (`5e188e2`).

- **The corpus at 1.1.0:**

  | | Before | Now |
  |---|---:|---:|
  | Parse cases | 542 | 607 |
  | from upstream's tests | 196 | 259 (52 `toolCompatibility`, 11 `toolReferences`) |
  | from the docs | 105 | 107 |
  | Merges | 2,525 | 3,067 (3,011 generated) |
  | Trees back to source | 512 | 577 |
  | Enriched errors | 342 | 366 |
  | Prompts | 61 | 61 |

- **Pending:**

  | List | Pending |
  |---|---:|
  | tokens | 605 of 605 |
  | statements and expression trees | 599 |
  | parse results | 84 |
  | merges | 66 |
  | prompts | 23 |
  | preprocessing | 8 |
  | enriched errors | 2 |
  | trees back to source | none |

- **Checked:** `scripts/check.sh` and `scripts/check-viewer.sh` pass.
