# 04 — Mutation testing: parsing and streaming

**What to build:** Find out whether the suite would notice real bugs in the parsing and streaming code: the lexer, statements and expressions, the parser, the preprocessor, materializing and validation, the streaming table and the parser, and results. Plant bugs automatically (mutants) and run the suite against each, with live diffs off. Run overnight on AC power, at about 29 seconds per mutant. Every mutant the suite misses is either:
- caught by a new case;
- or listed in a committed exclusion file as changing nothing, with a reason, so reruns skip it.

**Blocked by:** 01 — The suite runs anywhere (live diffs off); 03 — Coverage map (so unrun code isn't sorted twice)

**Status:** done (2026-11-09), finished under the hardening spec (GitHub #1).

- [x] The maintainer approves installing cargo-mutants before it's installed.
- [x] The run's command, duration and counts (mutants, caught, missed, timed out) are recorded in this ticket.
- [x] Every missed mutant is caught by a new case or explained in the exclusion file.
- [x] A rerun over these modules misses nothing unexplained.
- [x] TESTING.md has a section on mutation testing.
- [x] `scripts/check.sh` passes.

## Results

Checked 2026-11-09 against the code and the worker Mac's runs.
- **Tooling:** `scripts/mutants.sh parsing` (3019307), with `scripts/mutants-watchdog.sh` beside it. The exclusions are in `.cargo/mutants.toml`, and TESTING.md has a section on mutation testing.
- **The 1.1 long runs** left 334 missed mutants across both groups. GitHub #11–#16 caught each one or listed it with its reason.
- **Full run** on the worker Mac, 2026-11-08 at 185735a, `scripts/mutants.sh parsing --jobs 2`: 1,954 mutants in 12 h, with 1,799 caught, 147 unviable, 5 timeouts and 3 missed.
  - The 5 timeouts are mutants the suite notices as hangs.
  - The 3 misses were a block that did nothing, deleted in b0dad81 after an independent review.
- **Targeted rerun** at be4b5d3: 49 mutants, with 44 caught, 5 unviable and none missed.
- Watchdog kills: see GitHub #19 and #24.
- `scripts/check.sh` passes at 4cc48f8.
