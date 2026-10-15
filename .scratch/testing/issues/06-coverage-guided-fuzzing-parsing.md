# 06 — Coverage-guided fuzzing: parsing

**What to build:** Run the existing coverage-guided targets for one-shot parsing and streaming for real. They start from the corpus's programs and the existing dictionary. A per-input time limit turns slow inputs into blow-up findings. Each target runs a full night, about 8 hours, and the targets can run in parallel across cores. Every crash or blow-up is shrunk, becomes a case, and is fixed or explained. Hunts then run these targets for a share of their budget.

**Blocked by:** 02 — Hunts on demand

**Status:** ready-for-agent

- [ ] The maintainer approves installing cargo-fuzz before it's installed. It uses the nightly toolchain that's already there.
- [ ] The targets start from the corpus's programs, and the time limit is set with a reason.
- [ ] Each target has run a full night. Executions, coverage and findings are recorded in this ticket, and nothing new turned up by the end.
- [ ] Every finding is a case, fixed or explained.
- [ ] Hunts run these targets.
- [ ] TESTING.md has a section on coverage-guided fuzzing.
- [ ] `scripts/check.sh` passes.
