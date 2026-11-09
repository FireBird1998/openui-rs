# 05 — Mutation testing: the rest of the crate

**What to build:** The same as ticket 04, for the rest of the core: merging, conversion back to source, prompt generation, error hints, JSON reading and writing, the component library, builtins, and the crate's front page. Add the binding crate too, whose tests check it against the core.

**Blocked by:** 04 — Mutation testing: parsing and streaming

**Status:** done (2026-11-09), finished under the hardening spec (GitHub #1).

- [x] The run's command, duration and counts are recorded in this ticket.
- [x] Every missed mutant is caught by a new case or explained in the exclusion file.
- [x] A rerun misses nothing unexplained.
- [x] `scripts/check.sh` passes.

## Results

Checked 2026-11-09 against the code and the worker Mac's runs.
- **Tooling:** `scripts/mutants.sh rest` covers the rest of `openui-lang` and the binding crate `openui-ffi`.
- **Full run** on the worker Mac, 2026-11-08 at 185735a: 696 mutants in 2 h, with 634 caught, 60 unviable and 2 missed. The misses were `JsObject::get_mut` and an edge case of `json_string_len`, both caught by new cases in b0dad81.
- **Targeted rerun** at be4b5d3: 11 mutants, with 9 caught, 2 unviable and none missed.
- `scripts/check.sh` passes at 4cc48f8.
