# 03 — Coverage map

**What to build:** A line-coverage report for the core over the offline suite, the same tests the check runs, from one command. Use it to leave no unrun line unexplained. Each line no test runs is either:
- covered by a new case;
- deleted as dead code;
- or listed with the reason it can't be reached, in a committed file.

There's no percentage target. The bar is nothing left unexplained.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] The maintainer approves installing cargo-llvm-cov, and the Rust component it needs, before either is installed.
- [ ] One command produces the report: a summary in the terminal and per-file detail outside git.
- [ ] Every unrun line in the core is covered, deleted, or listed with its reason in a committed file.
- [ ] New conformance cases come with the reference's goldens.
- [ ] TESTING.md has a section on coverage: running it and reading it.
- [ ] `scripts/check.sh` passes.
