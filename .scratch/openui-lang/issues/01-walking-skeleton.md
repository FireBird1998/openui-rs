# 01 — Walking skeleton: one program, end to end

**What to build:** The thinnest complete path through the whole system. The repo becomes a Cargo workspace with the `openui-lang` crate. The reference parser is vendored at upstream commit `bba1d251`. A reference runner, adapted from the salvaged one, writes golden files. A conformance test passes for the simplest program, `root = Text("hi")`, against a one-component library, both one-shot and streamed in chunks of 4. The command-line tool can `parse` a file. Just enough lexer, parser and materialize code to make that one program conform; everything else comes in later tickets.

**Blocked by:** None — can start immediately.

**Status:** needs-triage

- [ ] `cargo test` passes on a machine without Bun, using only committed golden files.
- [ ] One command regenerates the golden files from the vendored reference parser using Bun, with OpenUI telemetry disabled.
- [ ] The vendored reference records its upstream commit, and a script refreshes it from a given commit.
- [ ] Deliberately breaking the Rust output makes the test fail with the case name, step number, chunk sizes and a readable JSON diff.
- [ ] `openui-lang parse <schema> <file>` prints the parse result JSON.
- [ ] One check command runs formatting, lints and tests, and it's green.
