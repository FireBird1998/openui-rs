# 01 — Walking skeleton: one program, end to end

**What to build:** The thinnest complete path through the whole system. The repo becomes a Cargo workspace with the `openui-lang` crate. The reference parser is vendored at upstream commit `bba1d251`. A reference runner, adapted from the salvaged one, writes golden files. A conformance test passes for the simplest program, `root = Text("hi")`, against a one-component library, both one-shot and streamed in chunks of 4. The command-line tool can `parse` a file. Just enough lexer, parser and materialize code to make that one program conform; everything else comes in later tickets.

**Blocked by:** None — can start immediately.

**Status:** done (2026-10-06)

- [x] `cargo test` passes on a machine without Bun, using only committed golden files.
- [x] One command regenerates the golden files from the vendored reference parser using Bun, with OpenUI telemetry disabled.
- [x] The vendored reference records its upstream commit, and a script refreshes it from a given commit.
- [x] Deliberately breaking the Rust output makes the test fail with the case name, step number, chunk sizes and a readable JSON diff.
- [x] `openui-lang parse <schema> <file>` prints the parse result JSON.
- [x] One check command runs formatting, lints and tests, and it's green.

## Comments

**2026-10-06, done.** What landed:
- A Cargo workspace (Rust 2024) with the `openui-lang` crate, plus `scripts/check.sh`.
- The reference vendored at `bba1d2515fa5` with `reference/UPSTREAM`. `scripts/update-reference.sh` works from a local checkout (any commit name) or GitHub (full hash only; it says so if given a short one). Both modes were verified to produce identical files.
- `reference/run.ts`, adapted from the salvage: `golden` writes `conformance/golden/<case>.oneshot.json` (exact bytes) and `<case>.steps.json` (fingerprint `fnv1a32:len` per push). `show <case> <chunking> <step>` prints one step, counting from 1. Telemetry is forced off.
- `conformance/manifest.json` lists the schemas, the cases (name, schema, file, source, optional root) and the chunk sizes.
- `tests/conformance.rs` collects every mismatch: case, chunk size, step, the input so far, and a unified diff. The diff uses Bun to fetch the reference step if it's installed; otherwise it prints the command. Verified both by breaking `partial` on purpose.
- Rust code covers only what `root = Text("hi")` touches. `auto_close` and `split` are full ports; lexer, expressions, materialize and preprocess are partial. Gaps are commented in the code.

Notes for later tickets are on 03, 05, 06 and 07.
