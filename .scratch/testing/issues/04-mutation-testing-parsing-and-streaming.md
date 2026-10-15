# 04 — Mutation testing: parsing and streaming

**What to build:** Find out whether the suite would notice real bugs in the parsing and streaming code: the lexer, statements and expressions, the parser, the preprocessor, materializing and validation, the streaming table and the parser, and results. Plant bugs automatically (mutants) and run the suite against each, with live diffs off. Run overnight on AC power, at about 29 seconds per mutant. Every mutant the suite misses is either:
- caught by a new case;
- or listed in a committed exclusion file as changing nothing, with a reason, so reruns skip it.

**Blocked by:** 01 — The suite runs anywhere (live diffs off); 03 — Coverage map (so unrun code isn't sorted twice)

**Status:** ready-for-agent

- [ ] The maintainer approves installing cargo-mutants before it's installed.
- [ ] The run's command, duration and counts (mutants, caught, missed, timed out) are recorded in this ticket.
- [ ] Every missed mutant is caught by a new case or explained in the exclusion file.
- [ ] A rerun over these modules misses nothing unexplained.
- [ ] TESTING.md has a section on mutation testing.
- [ ] `scripts/check.sh` passes.
