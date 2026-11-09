# 06 — Coverage-guided fuzzing: parsing

**What to build:** Run the existing coverage-guided targets for one-shot parsing and streaming for real. They start from the corpus's programs and the existing dictionary. A per-input time limit turns slow inputs into blow-up findings. Each target runs a full night, about 8 hours, and the targets can run in parallel across cores. Every crash or blow-up is shrunk, becomes a case, and is fixed or explained. Hunts then run these targets for a share of their budget.

**Blocked by:** 02 — Hunts on demand

**Status:** done (2026-11-09), finished under the hardening spec (GitHub #1, #19).

- [x] The maintainer approves installing cargo-fuzz before it's installed. It uses the nightly toolchain that's already there.
- [x] The targets start from the corpus's programs, and the time limit is set with a reason.
- [x] Each target has run a full night. Executions, coverage and findings are recorded in this ticket, and nothing new turned up by the end.
- [x] Every finding is a case, fixed or explained.
- [x] Hunts run these targets.
- [x] TESTING.md has a section on coverage-guided fuzzing.
- [x] `scripts/check.sh` passes.

## Results

Checked 2026-11-09.
- **Setup:** cargo-fuzz is installed on the worker Mac (approved 2026-11-06). `scripts/fuzz-coverage.sh` seeds the targets from the corpus with `fuzz/openui.dict`.
- **Caps:** a 2 s timeout, 512 MB per allocation and 1 GB RSS, with the reasons in TESTING.md under "Coverage-guided fuzzing" (GitHub #10).
- **Hunts:** `scripts/hunt.sh` runs these targets alongside the other checks.
- **Runs:** a "full night" became a 6 h run per target, the hardening spec's finish line and GitHub's job limit. The runs on the worker Mac, 2026-11-08 to 2026-11-09, found five real bugs, fixed with cases in b0dad81, 4295fc4, 1c122ba and 4cc48f8. They also found harness caps that were too loose for GitHub #22, tightened in 6b6e7bf. Details are in GitHub #19.
- **Final run** at 4cc48f8, 2026-11-09 12:39–18:39:
  - `parse`: 16,175,248 inputs, coverage 6,742, **clean**;
  - `stream`: 9,180,761 inputs, coverage 8,156, **clean**.
- `scripts/check.sh` passes.
