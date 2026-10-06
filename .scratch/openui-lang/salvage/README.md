# Salvage from the Oct 3 attempt

Copied from the untracked `crates/openui-lang` folder in the thesysdev/openui checkout (branch `feat/rust-lang-core`) on 2026-10-06. The module source files had been lost, so only these survived:

- `conformance/ts-reference.ts`: runs the reference parser over a cases file and prints a hash for every streaming step, or the full JSON of one step. This design carries over.
- `conformance/corpus.oui` and `validation-schema.json`: hand-written cases for validation, Query/Mutation, Actions, `$state` and `@Each`.
- `bench/stream.ts`: TypeScript streaming benchmark (4-character chunks, 5 warmups, mean of 20 runs).
- `src/bin/openui-lang.rs`: command-line entry point for parse, stream and bench.
- `src/lib.rs` and `Cargo.toml`: a sketch of the module list and dependencies, for reference only.

Ticket 01 moves what's useful into the real crate. The original folder in the OpenUI checkout is untouched.
