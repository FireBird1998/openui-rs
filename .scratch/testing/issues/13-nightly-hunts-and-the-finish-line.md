# 13 — Nightly hunts and the finish line

**What to build:** Two things: hunting that doesn't depend on anyone remembering, and a check that the phase reached its finish line.
- **The scheduled job.** It runs a hunt nightly, only on AC power, and leaves its report in the findings folder. Like every hunt, it never commits.
- **The finish line**, from the spec, checked item by item with evidence:
  - coverage and mutation testing leave nothing unexplained;
  - every coverage-guided target has run a full night;
  - every invariant holds on a million programs, apart from listed exceptions;
  - about 50 recorded streams conform;
  - the suite passes on Android;
  - hunts and TESTING.md exist.
- **The write-up.** CONFORMANCE.md gains a section with the numbers. TESTING.md is complete. The spec is marked done.

**Blocked by:** 05 — Mutation testing: the rest; 07 — Coverage-guided fuzzing: edits and JSON; 09 — The other five invariants; 11 — Fifty recorded streams; 12 — The suite on Android

**Status:** ready-for-agent

- [ ] The maintainer approves installing the scheduled job, a standing configuration on their Mac, before it's installed.
- [ ] It runs only on AC power, never commits, and its first nightly report has been reviewed with the maintainer.
- [ ] Every finish-line item is checked, with evidence linked in this ticket.
- [ ] CONFORMANCE.md has a section with the numbers: coverage, mutation results, fuzzing hours, invariants and recorded streams. The report test keeps the counts honest.
- [ ] TESTING.md covers every kind of test and the hunt.
- [ ] The spec's status is done.
- [ ] `scripts/check.sh` passes.
