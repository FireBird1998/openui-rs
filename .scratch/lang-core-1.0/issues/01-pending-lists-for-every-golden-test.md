# 01 — Pending lists for every golden test

**What to build:** Every test that compares the core with the reference's goldens can list cases the core doesn't match yet, the way parse conformance already does: the stages, merging, converting back to source, error hints and system prompts. A listed case may fail; one that passes again fails the test until it's taken off the list; a listed name that isn't a case is reported. `UPDATE_PENDING=1` rewrites each list to the cases failing now. On 0.3.0 every list is empty, so nothing changes in what the check accepts.

This is the prefactor that lets the move to 1.0 land in steps: ticket 02 marks the whole gap as pending, and each later ticket shrinks it.

**Blocked by:** None — can start immediately.

**Status:** done (2026-10-16)

- [x] Each golden test reads its own pending list, and the lists can't overwrite each other when tests run in parallel.
- [x] A pending case may fail, a pending case that passes fails the test, and a pending name that isn't a case is reported.
- [x] `UPDATE_PENDING=1` rewrites every list to the cases failing now.
- [x] Checked for each list: a deliberately wrong golden fails, and listing it makes the test pass.
- [x] TESTING.md and CONFORMANCE.md describe the lists.
- [x] `scripts/check.sh` passes with every list empty.

## Comments

**2026-10-16, done.** One list per golden test in `conformance/pending/`: `parse` (moved from `pending.txt`; the one-shot test reads it too), `tokens`, `statements`, `preprocess`, `merge`, `serialize`, `enrich` and `prompt`. A shared `Pending` type in the tests reads a list, skips its cases' failures, and reports a listed case that matches again or a listed name that isn't a case. With `UPDATE_PENDING=1` it rewrites the list. CONFORMANCE.md's generated counts gain a line, "Not matched yet, listed in `conformance/pending/`", which reads "none" today.

**Checked once, on a copy of `conformance/` through `OPENUI_CONFORMANCE_DIR`** (a scratch script; one golden broken per list):
- broken and unlisted: all nine golden tests failed, the eight with lists and the one-shot test;
- broken and listed: all passed;
- fixed but still listed, plus a name that isn't a case: all eight failed, each with both messages.

The one-shot test doesn't settle the list; the main parse test does.
