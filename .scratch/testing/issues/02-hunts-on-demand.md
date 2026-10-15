# 02 — Hunts on demand

**What to build:** A hunt: one command that searches for findings for a set time, then leaves a report and shrunk cases for review.

`scripts/hunt.sh --minutes N` runs, within its budget:
- differential fuzzing with a fresh seed, for parsing and for edits (merge and conversion back to source);
- an upstream check: are there lang-core commits newer than the pin? When there are, it vendors the new reference into a scratch copy, computes goldens for our corpus there, and reports which cases' outputs changed. Moving the pin stays a separate ticket.

Everything goes into a dated report in a findings folder git ignores. A hunt never commits and never changes tracked files. Today the fuzzer saves shrunk cases into the corpus, so a hunt needs it to save them into the findings folder instead. The report sorts each finding into a mismatch, a crash, a blow-up or a new quirk as far as the hunt can tell, with its seed and how to reproduce it. Later tickets add their checks to the hunt.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] One command with a time budget runs differential fuzzing for parsing and edits with a fresh seed, and stops within the budget.
- [ ] Shrunk findings and a dated report land in a folder git ignores. A test or check confirms that no tracked file changes during a hunt.
- [ ] Each finding in the report has its kind as far as the hunt can tell, its seed, and how to reproduce it.
- [ ] The upstream check reports new lang-core commits since the pin and, when there are any, which corpus cases' outputs they change. Without network or Bun, it says so and the hunt goes on.
- [ ] TESTING.md has a section on hunts: running one, reading its report, and turning a finding into a case.
- [ ] `scripts/check.sh` passes.
