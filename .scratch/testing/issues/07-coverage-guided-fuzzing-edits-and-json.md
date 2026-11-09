# 07 — Coverage-guided fuzzing: edits and JSON

**What to build:** New coverage-guided targets for merging, converting element trees back to source, and reading JSON. Each runs a full night, the same way as ticket 06, and is added to hunts.

**Blocked by:** 06 — Coverage-guided fuzzing: parsing

**Status:** done (2026-11-09), finished under the hardening spec (GitHub #1).

- [x] Three new targets, each starting from the matching corpus: the merge cases, the serialization cases, and the JSON the corpus holds.
- [x] Each has run a full night, with its numbers recorded, and nothing new turned up by the end.
- [x] Every finding is a case, fixed or explained.
- [x] Hunts run these targets.
- [x] `scripts/check.sh` passes.

## Results

Checked 2026-11-09.
- **Targets:** `merge`, `serialize` and `json` (3019307), seeded from their corpora by `scripts/fuzz-coverage.sh`, and run by hunts. A `library` target was added later too.
- **6 h run** on the worker Mac, 2026-11-08 at 185735a, under the caps:
  - `merge`: 233,084,740 inputs, coverage 1,678, clean;
  - `serialize`: 595,575,034 inputs, coverage 2,174, clean;
  - `json`: 975,445,608 inputs, coverage 831, clean.
- `scripts/check.sh` passes.
