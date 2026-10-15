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

**Status:** done (2026-10-15)

- [x] With the conformance folder copied elsewhere and the switch pointing there, the whole suite passes from the copy.
- [x] With live diffs off, a deliberately failing comparison reports from the goldens and doesn't start Bun.
- [x] `scripts/check.sh` builds the workspace on Rust 1.88.0, and no toolchain gets installed.
- [x] The thread-sharing check fails to compile if one of those types stops being shareable.
- [x] TESTING.md exists and covers everything above.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-15, done.**
- **`OPENUI_CONFORMANCE_DIR`** names the corpus folder. It's honored by every test that reads the corpus: the shared helpers, the CLI tests, and through the shared helpers, the binding's tests. The `report` test also reads `CONFORMANCE.md` and `reference/UPSTREAM` from the source tree, which ticket 12 will need to account for on a phone.
- **`OPENUI_NO_LIVE_DIFFS`** makes the one helper that starts Bun (`reference_output`) return the command to run instead.
- **Checked by hand:**
  - The corpus copied to the scratchpad, with both switches set, passes the whole suite.
  - One golden step broken in the copy fails `hand/04-nested-card-mixed-args`, so the copy is what's read. The failure says live diffs are off, and a fake `bun` on the PATH was called 0 times. With live diffs on, the same failure called it once.
- **The compile-time check** is in `src/lib.rs`. Adding `Rc<()>` to it failed to compile, with "cannot be sent between threads" and "cannot be shared between threads". That experiment was then removed.
- **The 1.88.0 build** in `scripts/check.sh` uses its own target directory (`target/msrv`) and `RUSTUP_AUTO_INSTALL=0`. It's skipped with a note when the toolchain is missing, so the check still runs elsewhere. The toolchain list was the same before and after.
- **TESTING.md** covers every existing test with what it catches, the corpus, adding a case, reading a failure, and the switches. The README links it.

