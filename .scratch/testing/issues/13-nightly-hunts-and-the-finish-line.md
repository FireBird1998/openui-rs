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

**Status:** blocked by 10, 11 and 12 (each waiting on the maintainer); 04–07 and 09 are done.

- [ ] The maintainer approves installing the scheduled job, a standing configuration on their Mac, before it's installed.
- [ ] It runs only on AC power, never commits, and its first nightly report has been reviewed with the maintainer.
- [ ] Every finish-line item is checked, with evidence linked in this ticket.
- [ ] CONFORMANCE.md has a section with the numbers: coverage, mutation results, fuzzing hours, invariants and recorded streams. The report test keeps the counts honest.
- [ ] TESTING.md covers every kind of test and the hunt.
- [ ] The spec's status is done.
- [ ] `scripts/check.sh` passes.

## Results

Checked 2026-11-09.
- **Blockers:** 04, 05, 06, 07 and 09 are done. 10 and 11 wait for the maintainer's recordings, and 12 for the real-phone run or a note that there's no phone.
- **Scheduled job:** no nightly hunt is installed yet. It needs the maintainer's OK. Since the main Mac shouldn't run long jobs, the worker Mac is the natural place for it.
- **Done since this ticket was written:**
  - GitHub workflows run the hunts' parts by hand (TESTING.md, "On GitHub");
  - TESTING.md covers every kind of test and the hunt;
  - the hardening phase (GitHub #1) measured the fuzzing and mutation numbers this ticket asks for.

**2026-11-11, the nightly job is written, for the worker Mac.** The maintainer chose the worker Mac for it (2026-11-11). `scripts/nightly-hunt.sh` installs and removes the launchd agent and is what it runs (TESTING.md, "Every night"). A 3-minute trial on the worker in a bare environment, as launchd gives, found nothing and left its report. It gets installed there once the script is pushed. Blocker 12 is done; 10 and 11 wait for the maintainer's batch recording.

