# 01 — The suite runs anywhere

**What to build:** The offline suite runs wherever its data is, without Bun, on the oldest Rust we claim, and the core's types are proven shareable across threads.
- **Two environment switches** for the Rust tests:
  - one names the conformance folder, defaulting to the repository's;
  - one turns off the live reference diffs a failing test shows, so a failure reports from the goldens alone and never starts Bun.

  Mutation testing (tickets 04 and 05) and Android (ticket 12) need both.
- **`scripts/check.sh` also builds on Rust 1.88.0.** Name that installed toolchain exactly, because a bare `1.88` made rustup install a second copy.
- **A compile-time check** that the streaming parser, parse results and component libraries can be sent between threads and shared.
- **TESTING.md is started.** It covers the tests that exist today: what each catches, how to run it, how to add a case, how to read a failure, and the two switches. Later tickets add their own sections.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] With the conformance folder copied elsewhere and the switch pointing there, the whole suite passes from the copy.
- [ ] With live diffs off, a deliberately failing comparison reports from the goldens and doesn't start Bun.
- [ ] `scripts/check.sh` builds the workspace on Rust 1.88.0, and no toolchain gets installed.
- [ ] The thread-sharing check fails to compile if one of those types stops being shareable.
- [ ] TESTING.md exists and covers everything above.
- [ ] `scripts/check.sh` passes.
